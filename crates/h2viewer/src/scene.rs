//! CPU-side scene: meshes (level, weapons), their textures, HUD bitmaps and
//! the gameplay data read from the map's tags.

use crate::rig::{FirstPersonRig, Skeleton, SkinnedMesh};
use blam_cache::animation;
use blam_cache::bitmap::{self, Image};
use blam_cache::geometry::Mesh;
use blam_cache::hud::{self, Anchor};
use blam_cache::lightmap::{self, InstanceLighting};
use blam_cache::model::{self, RenderModel};
use blam_cache::physics::{self, BipedPhysics, PlayerMovement};
use blam_cache::render::{LevelGeometry, Section, SectionOwner};
use blam_cache::shader::{self, Blend};
use blam_cache::{render, weapon, DatumIndex, GroupTag, MapSet, PlayerSpawn, StructureBsp};
use h2sim::WeaponDef;
use std::collections::HashMap;
use std::path::Path;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Level geometry: where the vertex samples its lightmap page.
    pub lightmap_uv: [f32; 2],
    /// Level geometry: baked light colour; alpha 1 multiplies it by the
    /// lightmap page.
    pub light: [f32; 4],
}

impl Vertex {
    pub fn new(position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> Vertex {
        Vertex {
            position,
            normal,
            uv,
            lightmap_uv: [0.0; 2],
            light: [1.0, 1.0, 1.0, 0.0],
        }
    }
}

/// What a glow-or-mask texture slot holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AuxKind {
    #[default]
    None,
    /// Self-illumination, scaled by `Material::illum_color`.
    Illum,
    /// Opacity.
    Mask,
}

/// How a surface is drawn: its textures and how it blends.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Material {
    pub texture: usize,
    /// Glow or opacity map (texture 0 when unused).
    pub aux: usize,
    pub aux_kind: AuxKind,
    pub blend: Blend,
    pub illum_color: [f32; 3],
    pub tint: [f32; 3],
    pub opacity: f32,
}

impl Material {
    fn plain(texture: usize) -> Material {
        Material {
            texture,
            tint: [1.0; 3],
            opacity: 1.0,
            ..Material::default()
        }
    }
}

pub struct Batch {
    pub material: usize,
    /// Lightmap page texture (texture 0 when unused).
    pub lightmap: usize,
    pub first_index: u32,
    pub index_count: u32,
}

/// How one section of level geometry is lit.
#[derive(Clone, Debug, PartialEq)]
pub enum SectionLight {
    /// Texture index of its lightmap page.
    Page(usize),
    /// One colour per vertex.
    Colors(Vec<[f32; 3]>),
    Unlit,
}

/// Light given to level geometry that has no baked lighting.
const UNLIT: [f32; 4] = [0.5, 0.5, 0.5, 0.0];

#[derive(Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<Batch>,
    /// Model meshes only: per vertex, the nodes it follows and their weights.
    pub bones: Vec<[u8; 4]>,
    pub weights: Vec<[f32; 4]>,
    /// Lit by its vertices' baked light rather than the object lighting.
    pub baked_lighting: bool,
}

impl MeshData {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Build from sections whose parts index `materials` (the model's
    /// shaders, mapped to scene materials). Level geometry passes each
    /// section's baked lighting in `lights`.
    fn from_sections<'a>(
        sections: impl IntoIterator<Item = &'a Section>,
        materials: &[usize],
        lights: &[SectionLight],
    ) -> MeshData {
        let mut mesh = MeshData {
            baked_lighting: !lights.is_empty(),
            ..MeshData::default()
        };
        let mut by_material: HashMap<(usize, usize), Vec<u32>> = HashMap::new();
        for (s, section) in sections.into_iter().enumerate() {
            let base = mesh.vertices.len() as u32;
            let count = section.positions.len();
            let mut light = lights.get(s).cloned();
            if matches!(light, Some(SectionLight::Page(_))) && section.lightmap_uvs.len() < count {
                light = Some(SectionLight::Unlit);
            }
            let page = match light {
                Some(SectionLight::Page(t)) => t,
                _ => 0,
            };
            for i in 0..count {
                let mut v = Vertex::new(section.positions[i], section.normals[i], section.uvs[i]);
                match &light {
                    Some(SectionLight::Page(_)) => {
                        v.lightmap_uv = section.lightmap_uvs[i];
                        v.light = [1.0; 4];
                    }
                    Some(SectionLight::Colors(c)) => {
                        v.light = c.get(i).map_or(UNLIT, |c| [c[0], c[1], c[2], 0.0]);
                    }
                    Some(SectionLight::Unlit) => v.light = UNLIT,
                    None => {}
                }
                mesh.vertices.push(v);
                mesh.bones
                    .push(section.bones.get(i).copied().unwrap_or_default());
                mesh.weights.push(
                    section
                        .weights
                        .get(i)
                        .copied()
                        .unwrap_or([1.0, 0.0, 0.0, 0.0]),
                );
            }
            for part in &section.parts {
                let material = usize::try_from(part.material)
                    .ok()
                    .and_then(|m| materials.get(m))
                    .copied()
                    .unwrap_or(0);
                by_material
                    .entry((material, page))
                    .or_default()
                    .extend(part.indices.iter().map(|i| i + base));
            }
        }
        // In material order: a model's materials follow its shader list,
        // which is the order skies layer their transparent parts in.
        let mut keys: Vec<(usize, usize)> = by_material.keys().copied().collect();
        keys.sort_unstable();
        for k in keys {
            let idx = &by_material[&k];
            mesh.batches.push(Batch {
                material: k.0,
                lightmap: k.1,
                first_index: mesh.indices.len() as u32,
                index_count: idx.len() as u32,
            });
            mesh.indices.extend_from_slice(idx);
        }
        mesh
    }
}

/// A HUD bitmap placed on screen, in Halo 2's 640x480 HUD layout.
#[derive(Clone)]
pub struct HudWidget {
    pub name: String,
    pub texture: usize,
    pub anchor: Anchor,
    pub flags: u16,
    pub offset: [f32; 2],
    pub registration: [f32; 2],
    pub size: [f32; 2],
}

/// Everything needed to hold and fire one weapon.
pub struct WeaponAssets {
    pub def: WeaponDef,
    /// First person model, in `Scene::meshes`.
    pub view_mesh: Option<usize>,
    pub skeleton: Skeleton,
    pub skin: SkinnedMesh,
    /// The muzzle: a gun node and the offset from it.
    pub muzzle_node: usize,
    pub muzzle: [f32; 3],
    /// Where the gun's root node sits in its first person model.
    pub grip: [f32; 3],
    /// First person animations (arms and gun), when the map has them.
    pub rig: Option<FirstPersonRig>,
    pub hud: Vec<HudWidget>,
}

/// Master Chief's first person arms.
pub struct Arms {
    pub mesh: usize,
    pub skeleton: Skeleton,
    pub skin: SkinnedMesh,
}

pub struct Scene {
    /// Texture 0 is always a plain light-grey fallback.
    pub textures: Vec<Image>,
    /// Material 0 is the fallback texture, opaque.
    pub materials: Vec<Material>,
    /// Mesh 0 is the level.
    pub meshes: Vec<MeshData>,
    /// HUD bitmaps; their channels are data (meters, masks), not colours.
    pub hud_textures: Vec<Image>,
    pub spawn: Option<PlayerSpawn>,
    /// Collision geometry, used for walking, shooting and framing.
    pub collision: Mesh,
    pub movement: PlayerMovement,
    pub biped: BipedPhysics,
    /// Weapons the player can switch between; the Battle Rifle first.
    pub weapons: Vec<WeaponAssets>,
    pub arms: Option<Arms>,
    /// The sky's model, drawn around the camera behind everything.
    pub sky: Option<usize>,
    /// The player's own HUD (shields, motion tracker, grenades).
    pub player_hud: Vec<HudWidget>,
    /// HUD textures for text and solid fills.
    pub hud_font: usize,
    pub hud_white: usize,
}

/// Multiplayer weapons, in switching order.
const WEAPONS: &[&str] = &[
    "objects\\weapons\\rifle\\battle_rifle\\battle_rifle",
    "objects\\weapons\\rifle\\smg\\smg",
    "objects\\weapons\\pistol\\magnum\\magnum",
    "objects\\weapons\\rifle\\shotgun\\shotgun",
    "objects\\weapons\\rifle\\sniper_rifle\\sniper_rifle",
    "objects\\weapons\\support_high\\rocket_launcher\\rocket_launcher",
    "objects\\weapons\\rifle\\covenant_carbine\\covenant_carbine",
    "objects\\weapons\\rifle\\beam_rifle\\beam_rifle",
    "objects\\weapons\\rifle\\plasma_rifle\\plasma_rifle",
    "objects\\weapons\\rifle\\brute_plasma_rifle\\brute_plasma_rifle",
    "objects\\weapons\\pistol\\plasma_pistol\\plasma_pistol",
    "objects\\weapons\\pistol\\needler\\needler",
    "objects\\weapons\\support_low\\brute_shot\\brute_shot",
    "objects\\weapons\\support_high\\flak_cannon\\flak_cannon",
    "objects\\weapons\\melee\\energy_blade\\energy_blade",
];

fn fallback_texture() -> Image {
    Image {
        width: 1,
        height: 1,
        rgba: vec![200, 200, 200, 255],
    }
}

/// Loads tags into the scene, sharing textures between everything that uses them.
struct Loader {
    set: MapSet,
    textures: Vec<Image>,
    texture_of_bitmap: HashMap<DatumIndex, usize>,
    materials: Vec<Material>,
    material_of_shader: HashMap<DatumIndex, usize>,
    lightmap_pages: HashMap<(DatumIndex, usize), usize>,
    hud_textures: Vec<Image>,
    hud_of_bitmap: HashMap<(DatumIndex, i8), usize>,
    failures: usize,
}

impl Loader {
    /// The texture of a bitmap tag's first image (0 if it can't be read).
    fn bitmap_texture(&mut self, b: DatumIndex) -> usize {
        if let Some(&t) = self.texture_of_bitmap.get(&b) {
            return t;
        }
        let t = match bitmap::read_bitmap(&mut self.set, b) {
            Ok(img) => {
                self.textures.push(img);
                self.textures.len() - 1
            }
            Err(_) => {
                self.failures += 1;
                0
            }
        };
        self.texture_of_bitmap.insert(b, t);
        t
    }

    fn shader_material(&mut self, shader: DatumIndex) -> usize {
        if let Some(&m) = self.material_of_shader.get(&shader) {
            return m;
        }
        let material = match shader::read_shader(&mut self.set, shader) {
            Ok(info) => {
                let mut m = Material {
                    texture: info.diffuse.map_or(0, |b| self.bitmap_texture(b)),
                    blend: info.blend,
                    tint: info.tint,
                    opacity: info.opacity,
                    ..Material::default()
                };
                if let Some((b, color)) = info.illum {
                    m.aux = self.bitmap_texture(b);
                    m.aux_kind = AuxKind::Illum;
                    m.illum_color = color;
                } else if let Some(b) = info.mask {
                    m.aux = self.bitmap_texture(b);
                    m.aux_kind = AuxKind::Mask;
                }
                m
            }
            Err(_) => Material::plain(0),
        };
        self.materials.push(material);
        let m = self.materials.len() - 1;
        self.material_of_shader.insert(shader, m);
        m
    }

    /// Lighting for each section of a BSP's render geometry, loading the
    /// lightmap pages it uses.
    fn level_lights(&mut self, bsp: &StructureBsp, geo: &LevelGeometry) -> Vec<SectionLight> {
        let lighting =
            match lightmap::read_level_lighting(&mut self.set, bsp, &geo.instance_vertices) {
                Ok(l) => l,
                Err(e) => {
                    println!("warning: lightmap unavailable ({e}); the level will look flat");
                    return vec![SectionLight::Unlit; geo.sections.len()];
                }
            };
        geo.owners
            .iter()
            .map(|owner| {
                let page = match owner {
                    SectionOwner::Cluster(c) => lighting.clusters.get(*c).copied().flatten(),
                    SectionOwner::Instance(i) => match lighting.instances.get(*i) {
                        Some(InstanceLighting::Lightmap(p)) => Some(*p),
                        Some(InstanceLighting::VertexColors(c)) => {
                            return SectionLight::Colors(c.clone())
                        }
                        _ => None,
                    },
                };
                match (page, lighting.bitmap) {
                    (Some(p), Some(b)) => match self.lightmap_page(b, p) {
                        Some(t) => SectionLight::Page(t),
                        None => SectionLight::Unlit,
                    },
                    _ => SectionLight::Unlit,
                }
            })
            .collect()
    }

    fn lightmap_page(&mut self, bitmap: DatumIndex, page: usize) -> Option<usize> {
        if let Some(&t) = self.lightmap_pages.get(&(bitmap, page)) {
            return Some(t);
        }
        let t = match bitmap::read_bitmap_at(&mut self.set, bitmap, page) {
            Ok(img) => {
                self.textures.push(img);
                Some(self.textures.len() - 1)
            }
            Err(_) => {
                self.failures += 1;
                None
            }
        };
        if let Some(t) = t {
            self.lightmap_pages.insert((bitmap, page), t);
        }
        t
    }

    fn model_mesh(&mut self, model: &RenderModel) -> MeshData {
        let materials: Vec<usize> = model
            .shaders
            .iter()
            .map(|&s| self.shader_material(s))
            .collect();
        MeshData::from_sections(&model.sections, &materials, &[])
    }

    fn hud_widgets(&mut self, nhdt: DatumIndex) -> Vec<HudWidget> {
        let Ok(widgets) = hud::read_bitmap_widgets(&mut self.set, nhdt) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for w in widgets {
            if w.bitmap == DatumIndex::NONE {
                continue;
            }
            let key = (w.bitmap, w.sequence);
            let texture = if let Some(&t) = self.hud_of_bitmap.get(&key) {
                t
            } else {
                let seqs = bitmap::read_sequences(&mut self.set, w.bitmap).unwrap_or_default();
                let index = usize::try_from(w.sequence)
                    .ok()
                    .and_then(|s| seqs.get(s))
                    .map(|s| s.first_bitmap.max(0) as usize)
                    .unwrap_or(0);
                match bitmap::read_bitmap_at(&mut self.set, w.bitmap, index) {
                    Ok(img) => {
                        self.hud_textures.push(img);
                        let t = self.hud_textures.len() - 1;
                        self.hud_of_bitmap.insert(key, t);
                        t
                    }
                    Err(_) => continue,
                }
            };
            let img = &self.hud_textures[texture];
            out.push(HudWidget {
                name: w.name,
                texture,
                anchor: w.anchor,
                flags: w.flags,
                offset: [w.offset[0] as f32, w.offset[1] as f32],
                registration: w.registration,
                size: [img.width as f32, img.height as f32],
            });
        }
        out
    }

    fn find(&self, group: &str, name: &str) -> Option<DatumIndex> {
        let group = GroupTag::parse(group)?;
        self.set
            .map
            .tags
            .iter()
            .find(|t| t.group == group && t.name == name)
            .map(|t| t.datum)
    }

    fn weapon(
        &mut self,
        name: &str,
        arms: Option<&Skeleton>,
        meshes: &mut Vec<MeshData>,
    ) -> Option<WeaponAssets> {
        let datum = self.find("weap", name)?;
        let w = match weapon::read_weapon(&mut self.set, datum) {
            Ok(w) => w,
            Err(e) => {
                println!("warning: couldn't read {name}: {e}");
                return None;
            }
        };
        let barrel = w.barrels.first().copied().unwrap_or_default();
        let projectile = weapon::read_projectile(&mut self.set, barrel.projectile).ok();
        let damage = projectile
            .as_ref()
            .and_then(|p| weapon::read_damage(&mut self.set, p.impact_damage).ok());
        let def = WeaponDef::from_tags(&w, projectile.as_ref(), damage.as_ref());

        let mut view_mesh = None;
        let mut muzzle = [0.2, 0.0, 0.04];
        let mut muzzle_node = 0;
        let mut grip = [0.0; 3];
        let mut skeleton = Skeleton::default();
        let mut skin = SkinnedMesh::default();
        let fp = w
            .first_person_model
            .or_else(|| model::object_render_model(&mut self.set, datum).ok());
        if let Some(fp) = fp {
            match model::read_render_model(&mut self.set, fp) {
                Ok(m) => {
                    if let Some(mk) = m.marker("muzzle_flash") {
                        muzzle = mk.translation;
                        muzzle_node = mk.node as usize;
                    }
                    if let Some(root) = m.nodes.first() {
                        grip = root.translation;
                    }
                    skeleton = Skeleton::new(&m.nodes);
                    let mesh = self.model_mesh(&m);
                    skin = SkinnedMesh::new(&mesh);
                    meshes.push(mesh);
                    view_mesh = Some(meshes.len() - 1);
                }
                Err(e) => println!("warning: first person model for {name}: {e}"),
            }
        }
        let rig = match (w.first_person_animations, arms) {
            (Some(jmad), Some(arms)) if view_mesh.is_some() => {
                match animation::read_animation_graph(&mut self.set, jmad) {
                    Ok(g) => Some(FirstPersonRig::new(g, arms, &skeleton)),
                    Err(e) => {
                        println!("warning: first person animations for {name}: {e}");
                        None
                    }
                }
            }
            _ => None,
        };
        let hud = w.hud.map(|h| self.hud_widgets(h)).unwrap_or_default();
        Some(WeaponAssets {
            def,
            view_mesh,
            skeleton,
            skin,
            muzzle_node,
            muzzle,
            grip,
            rig,
            hud,
        })
    }

    fn sky(&mut self, meshes: &mut Vec<MeshData>) -> Option<usize> {
        let sky = *self.set.map.skies().ok()?.first()?;
        let model = model::sky_render_model(&mut self.set, sky)
            .and_then(|mode| model::read_render_model(&mut self.set, mode));
        match model {
            Ok(m) => {
                meshes.push(self.model_mesh(&m));
                Some(meshes.len() - 1)
            }
            Err(e) => {
                println!("warning: sky: {e}");
                None
            }
        }
    }

    fn arms(&mut self, meshes: &mut Vec<MeshData>) -> Option<Arms> {
        let mode = self.find("mode", "objects\\characters\\masterchief\\fp\\fp")?;
        let m = match model::read_render_model(&mut self.set, mode) {
            Ok(m) => m,
            Err(e) => {
                println!("warning: first person arms: {e}");
                return None;
            }
        };
        let mesh = self.model_mesh(&m);
        let skin = SkinnedMesh::new(&mesh);
        meshes.push(mesh);
        Some(Arms {
            mesh: meshes.len() - 1,
            skeleton: Skeleton::new(&m.nodes),
            skin,
        })
    }
}

impl Scene {
    pub fn load(path: &Path) -> Result<Scene, Box<dyn std::error::Error>> {
        let mut set = MapSet::open(path)?;
        if set.shared.is_none() {
            println!(
                "warning: shared.map not found next to the map; textures from it will be missing"
            );
        }
        let bsps = set.map.structure_bsps()?;
        let mut collision = Mesh::default();
        for bsp in &bsps {
            collision.append(&set.map.bsp_collision_mesh(bsp)?);
        }
        let spawn = set
            .map
            .player_spawns()
            .ok()
            .and_then(|s| s.first().copied());
        let movement = physics::player_movement(&mut set).unwrap_or_default();
        let biped = physics::player_biped(&mut set).unwrap_or_default();

        let mut loader = Loader {
            set,
            textures: vec![fallback_texture()],
            texture_of_bitmap: HashMap::new(),
            materials: vec![Material::plain(0)],
            material_of_shader: HashMap::new(),
            lightmap_pages: HashMap::new(),
            hud_textures: Vec::new(),
            hud_of_bitmap: HashMap::new(),
            failures: 0,
        };

        // The level: render geometry grouped by texture and lightmap page.
        let mut level = MeshData {
            baked_lighting: true,
            ..MeshData::default()
        };
        for bsp in &bsps {
            let geo = match render::bsp_render_geometry(&mut loader.set, bsp) {
                Ok(geo) => geo,
                Err(e) => {
                    println!(
                        "warning: render geometry unavailable ({e}); showing collision geometry"
                    );
                    continue;
                }
            };
            let mats: Vec<usize> = geo
                .shaders
                .iter()
                .map(|&s| loader.shader_material(s))
                .collect();
            let lights = loader.level_lights(bsp, &geo);
            let part = MeshData::from_sections(&geo.sections, &mats, &lights);
            let base = level.vertices.len() as u32;
            let first = level.indices.len() as u32;
            level.vertices.extend_from_slice(&part.vertices);
            level.indices.extend(part.indices.iter().map(|i| i + base));
            level
                .batches
                .extend(part.batches.into_iter().map(|b| Batch {
                    first_index: b.first_index + first,
                    ..b
                }));
        }
        if level.batches.is_empty() {
            level = collision_mesh(&collision);
        }

        let mut meshes = vec![level];
        let sky = loader.sky(&mut meshes);
        let arms = loader.arms(&mut meshes);
        let weapons = WEAPONS
            .iter()
            .filter_map(|name| loader.weapon(name, arms.as_ref().map(|a| &a.skeleton), &mut meshes))
            .collect();
        let player_hud = match loader.find("nhdt", "ui\\hud\\masterchief") {
            Some(h) => loader.hud_widgets(h),
            None => Vec::new(),
        };
        loader.hud_textures.push(crate::font::atlas());
        let hud_font = loader.hud_textures.len() - 1;
        loader.hud_textures.push(Image {
            width: 1,
            height: 1,
            rgba: vec![255; 4],
        });
        let hud_white = loader.hud_textures.len() - 1;
        if loader.failures > 0 {
            println!("warning: {} textures couldn't be decoded", loader.failures);
        }
        Ok(Scene {
            textures: loader.textures,
            materials: loader.materials,
            meshes,
            hud_textures: loader.hud_textures,
            spawn,
            collision,
            movement,
            biped,
            weapons,
            arms,
            sky,
            player_hud,
            hud_font,
            hud_white,
        })
    }

    pub fn triangle_count(&self) -> usize {
        self.meshes[0].triangle_count()
    }
}

/// Flat-shaded collision geometry with the fallback texture.
fn collision_mesh(collision: &Mesh) -> MeshData {
    let mut mesh = MeshData::default();
    for t in collision.indices.as_chunks::<3>().0 {
        let p = t.map(|i| glam::Vec3::from(collision.positions[i as usize]));
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or(glam::Vec3::Z);
        for v in p {
            mesh.indices.push(mesh.vertices.len() as u32);
            mesh.vertices
                .push(Vertex::new(v.into(), n.into(), [0.0; 2]));
        }
    }
    mesh.batches = vec![Batch {
        material: 0,
        lightmap: 0,
        first_index: 0,
        index_count: mesh.indices.len() as u32,
    }];
    mesh
}

/// Full mip chain for an RGBA8 image by 2x2 box filtering.
pub fn mip_chain(img: &Image) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(img.width, img.height, img.rgba.clone())];
    while let Some((w, h, prev)) = levels.last() {
        if *w == 1 && *h == 1 {
            break;
        }
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        sum += prev[((sy * w + sx) * 4 + c) as usize] as u32;
                    }
                    next[((y * nw + x) * 4 + c) as usize] = (sum / 4) as u8;
                }
            }
        }
        levels.push((nw, nh, next));
    }
    levels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_reaches_1x1() {
        let img = Image {
            width: 4,
            height: 2,
            rgba: vec![255; 4 * 2 * 4],
        };
        let mips = mip_chain(&img);
        assert_eq!(
            mips.iter().map(|m| (m.0, m.1)).collect::<Vec<_>>(),
            vec![(4, 2), (2, 1), (1, 1)]
        );
        assert!(mips[2].2.iter().all(|&b| b == 255));
    }

    #[test]
    fn sections_batch_by_texture() {
        let section = Section {
            positions: vec![[0.0; 3]; 4],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0; 2]; 4],
            parts: vec![
                render::Part {
                    material: 1,
                    indices: vec![0, 1, 2],
                },
                render::Part {
                    material: 0,
                    indices: vec![1, 2, 3],
                },
            ],
            bones: Vec::new(),
            weights: Vec::new(),
            lightmap_uvs: Vec::new(),
        };
        let mesh = MeshData::from_sections([&section], &[5, 7], &[]);
        assert_eq!(mesh.batches.len(), 2);
        assert_eq!(mesh.batches[0].material, 5);
        assert_eq!(&mesh.indices[..3], &[1, 2, 3]);
    }

    #[test]
    fn level_sections_carry_their_lighting() {
        let section = |lightmap_uvs: Vec<[f32; 2]>| Section {
            positions: vec![[0.0; 3]; 3],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uvs: vec![[0.0; 2]; 3],
            parts: vec![render::Part {
                material: 0,
                indices: vec![0, 1, 2],
            }],
            bones: Vec::new(),
            weights: Vec::new(),
            lightmap_uvs,
        };
        let paged = section(vec![[0.25, 0.5]; 3]);
        let coloured = section(Vec::new());
        let missing_uvs = section(Vec::new());
        let mesh = MeshData::from_sections(
            [&paged, &coloured, &missing_uvs],
            &[4],
            &[
                SectionLight::Page(9),
                SectionLight::Colors(vec![[0.1, 0.2, 0.3]; 3]),
                SectionLight::Page(9),
            ],
        );
        assert!(mesh.baked_lighting);
        assert_eq!(mesh.vertices[0].light, [1.0; 4]);
        assert_eq!(mesh.vertices[0].lightmap_uv, [0.25, 0.5]);
        assert_eq!(mesh.vertices[3].light, [0.1, 0.2, 0.3, 0.0]);
        assert_eq!(mesh.vertices[6].light, UNLIT);
        let pages: Vec<usize> = mesh.batches.iter().map(|b| b.lightmap).collect();
        assert_eq!(pages, vec![0, 9]);
    }
}
