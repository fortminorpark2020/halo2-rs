//! CPU-side scene: meshes (level, weapons), their textures, HUD bitmaps and
//! the gameplay data read from the map's tags.

use blam_cache::bitmap::{self, Image};
use blam_cache::geometry::Mesh;
use blam_cache::hud::{self, Anchor};
use blam_cache::model::{self, RenderModel};
use blam_cache::physics::{self, BipedPhysics, PlayerMovement};
use blam_cache::render::Section;
use blam_cache::{render, shader, weapon, DatumIndex, GroupTag, MapSet, PlayerSpawn};
use h2sim::WeaponDef;
use std::collections::HashMap;
use std::path::Path;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

pub struct Batch {
    pub texture: usize,
    pub first_index: u32,
    pub index_count: u32,
}

#[derive(Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<Batch>,
}

impl MeshData {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Build from sections whose parts index `material_texture`.
    fn from_sections<'a>(
        sections: impl IntoIterator<Item = &'a Section>,
        material_texture: &[usize],
    ) -> MeshData {
        let mut mesh = MeshData::default();
        let mut by_texture: HashMap<usize, Vec<u32>> = HashMap::new();
        for section in sections {
            let base = mesh.vertices.len() as u32;
            for i in 0..section.positions.len() {
                mesh.vertices.push(Vertex {
                    position: section.positions[i],
                    normal: section.normals[i],
                    uv: section.uvs[i],
                });
            }
            for part in &section.parts {
                let tex = usize::try_from(part.material)
                    .ok()
                    .and_then(|m| material_texture.get(m))
                    .copied()
                    .unwrap_or(0);
                by_texture
                    .entry(tex)
                    .or_default()
                    .extend(part.indices.iter().map(|i| i + base));
            }
        }
        let mut keys: Vec<usize> = by_texture.keys().copied().collect();
        keys.sort_unstable();
        for k in keys {
            let idx = &by_texture[&k];
            mesh.batches.push(Batch {
                texture: k,
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
    /// Muzzle position in the first person model's space.
    pub muzzle: [f32; 3],
    /// Bounding box of the first person model.
    /// Where the gun's root node sits in its first person model.
    pub grip: [f32; 3],
    pub hud: Vec<HudWidget>,
}

pub struct Scene {
    /// Texture 0 is always a plain light-grey fallback.
    pub textures: Vec<Image>,
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
    texture_of_shader: HashMap<DatumIndex, usize>,
    hud_textures: Vec<Image>,
    hud_of_bitmap: HashMap<(DatumIndex, i8), usize>,
    failures: usize,
}

impl Loader {
    fn shader_texture(&mut self, shader: DatumIndex) -> usize {
        if let Some(&t) = self.texture_of_shader.get(&shader) {
            return t;
        }
        let t = match shader::read_shader(&mut self.set, shader) {
            Ok(shader::ShaderInfo { diffuse: Some(b) }) => {
                if let Some(&t) = self.texture_of_bitmap.get(&b) {
                    t
                } else {
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
            }
            _ => 0,
        };
        self.texture_of_shader.insert(shader, t);
        t
    }

    fn model_mesh(&mut self, model: &RenderModel) -> MeshData {
        let material_texture: Vec<usize> = model
            .shaders
            .iter()
            .map(|&s| self.shader_texture(s))
            .collect();
        MeshData::from_sections(&model.sections, &material_texture)
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

    fn weapon(&mut self, name: &str, meshes: &mut Vec<MeshData>) -> Option<WeaponAssets> {
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
        let mut grip = [0.0; 3];
        let fp = w
            .first_person_model
            .or_else(|| model::object_render_model(&mut self.set, datum).ok());
        if let Some(fp) = fp {
            match model::read_render_model(&mut self.set, fp) {
                Ok(m) => {
                    if let Some(mk) = m.marker("muzzle_flash") {
                        muzzle = mk.translation;
                    }
                    if let Some(root) = m.nodes.first() {
                        grip = root.translation;
                    }
                    let mesh = self.model_mesh(&m);
                    meshes.push(mesh);
                    view_mesh = Some(meshes.len() - 1);
                }
                Err(e) => println!("warning: first person model for {name}: {e}"),
            }
        }
        let hud = w.hud.map(|h| self.hud_widgets(h)).unwrap_or_default();
        Some(WeaponAssets {
            def,
            view_mesh,
            muzzle,
            grip,
            hud,
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
            texture_of_shader: HashMap::new(),
            hud_textures: Vec::new(),
            hud_of_bitmap: HashMap::new(),
            failures: 0,
        };

        // The level: render geometry grouped by texture.
        let mut level_sections = Vec::new();
        let mut level_textures = Vec::new();
        for bsp in &bsps {
            match render::bsp_render_geometry(&mut loader.set, bsp) {
                Ok(geo) => {
                    let mats: Vec<usize> = geo
                        .shaders
                        .iter()
                        .map(|&s| loader.shader_texture(s))
                        .collect();
                    level_sections.push(geo.sections);
                    level_textures.push(mats);
                }
                Err(e) => {
                    println!(
                        "warning: render geometry unavailable ({e}); showing collision geometry"
                    );
                }
            }
        }
        let mut level = MeshData::default();
        for (sections, mats) in level_sections.iter().zip(&level_textures) {
            let part = MeshData::from_sections(sections, mats);
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
        let weapons = WEAPONS
            .iter()
            .filter_map(|name| loader.weapon(name, &mut meshes))
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
            meshes,
            hud_textures: loader.hud_textures,
            spawn,
            collision,
            movement,
            biped,
            weapons,
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
            mesh.vertices.push(Vertex {
                position: v.into(),
                normal: n.into(),
                uv: [0.0; 2],
            });
        }
    }
    mesh.batches = vec![Batch {
        texture: 0,
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
            nodes: Vec::new(),
        };
        let mesh = MeshData::from_sections([&section], &[5, 7]);
        assert_eq!(mesh.batches.len(), 2);
        assert_eq!(mesh.batches[0].texture, 5);
        assert_eq!(&mesh.indices[..3], &[1, 2, 3]);
    }
}
