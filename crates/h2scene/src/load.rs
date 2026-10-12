//! Reading mainmenu.map's scene into `SceneData`: the level, its sky and
//! fog, the scenery placed from the start, and the flythrough. A port of
//! h2viewer's `Backdrop::load` and the parts of its `Loader` it uses
//! (menu-preview), with shaders read the menu's way
//! (`shader::read_menu_shader`). It runs on any thread and needs no GPU.

use crate::data::{AuxKind, Material, MeshData, Object, SceneData, SectionLight, TextureData};
use crate::flythrough::Flythrough;
use crate::light::LevelLight;
use blam_cache::bitmap::{self, Image};
use blam_cache::lightmap::{self, InstanceLighting};
use blam_cache::model::{self, Atmosphere, RenderModel};
use blam_cache::render::{self, LevelGeometry, SectionOwner};
use blam_cache::{scenario, script, shader, DatumIndex, MapSet, StructureBsp};
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

/// MCC's cache format, whose geometry and bitmaps this crate can't read.
const MCC_FORMAT: i32 = 13;

/// World transform of a scenario placement: yaw about +z, then pitch
/// (raising +x), then roll about +x.
pub fn placement_matrix(position: [f32; 3], rotation: [f32; 3], scale: f32) -> Mat4 {
    let [yaw, pitch, roll] = rotation;
    Mat4::from_translation(Vec3::from(position))
        * Mat4::from_rotation_z(yaw)
        * Mat4::from_rotation_y(-pitch)
        * Mat4::from_rotation_x(roll)
        * Mat4::from_scale(Vec3::splat(scale))
}

/// Read the menu scene from `mainmenu` (Halo 2 Vista's mainmenu.map, as
/// Project Cartographer installs it). Warnings go to `log`; anything that
/// leaves nothing to draw (no map, not a Vista map, no level geometry, no
/// flythrough) is an error. A damaged map never panics the caller.
pub fn load(mainmenu: &Path, log: &dyn Fn(&str)) -> Result<SceneData, String> {
    let name = mainmenu.display();
    let mut head = [0u8; 8];
    std::fs::File::open(mainmenu)
        .and_then(|mut f| f.read_exact(&mut head))
        .map_err(|e| format!("{name}: {e}"))?;
    match blam_cache::cache_version(&head) {
        Some(MCC_FORMAT) => return Err("MCC maps have no geometry reader yet".into()),
        Some(8) => {}
        Some(v) => return Err(format!("{name}: cache format {v}, not Halo 2 Vista's 8")),
        None => return Err(format!("{name}: not a Halo 2 map")),
    }
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(mainmenu, log)));
    match read {
        Ok(scene) => scene.map_err(|e| format!("{name}: {e}")),
        Err(_) => Err(format!("{name}: the scene reader failed (damaged map?)")),
    }
}

fn read(path: &Path, log: &dyn Fn(&str)) -> Result<SceneData, String> {
    let mut set = MapSet::open(path).map_err(|e| e.to_string())?;
    let bsps = set.map.structure_bsps().map_err(|e| e.to_string())?;
    let flythrough = match (script::scripts(&mut set), scenario::camera_points(&mut set)) {
        (Ok(scripts), Ok(points)) => Flythrough::new(&scripts, &points),
        (Err(e), _) | (_, Err(e)) => {
            log(&format!("warning: flythrough: {e}"));
            None
        }
    }
    .ok_or("no flythrough (its script or camera points are missing)")?;
    let mut loader = Loader::new(set, log);
    let level = loader.level(&bsps);
    if level.indices.is_empty() {
        return Err("no level geometry".into());
    }
    let mut meshes = Vec::new();
    let sky = loader.sky(&mut meshes);
    let atmosphere = loader.atmosphere();
    let level_light = LevelLight::new(&level);
    let scenery = scenario::scenery(&mut loader.set).unwrap_or_else(|e| {
        log(&format!("warning: scenery: {e}"));
        Vec::new()
    });
    let mut objects = Vec::new();
    for p in scenery.into_iter().filter(|p| p.automatic) {
        let Some(mesh) = loader.object_mesh(p.object, &mut meshes) else {
            continue;
        };
        let light = level_light.at(&loader.textures, Vec3::from(p.position) + Vec3::Z * 0.2);
        let name = loader
            .set
            .map
            .tag(p.object)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        objects.push(Object {
            mesh,
            transform: placement_matrix(p.position, p.rotation, p.scale),
            light,
            name,
        });
    }
    if loader.failures > 0 {
        log(&format!(
            "warning: {} textures couldn't be decoded",
            loader.failures
        ));
    }
    Ok(SceneData {
        level,
        bsps: bsps.len(),
        meshes,
        objects,
        sky,
        atmosphere,
        textures: loader.textures,
        materials: loader.materials,
        flythrough,
        failed_textures: loader.failures,
    })
}

/// Loads tags into the scene, sharing textures between everything that
/// uses them.
struct Loader<'a> {
    set: MapSet,
    log: &'a dyn Fn(&str),
    textures: Vec<TextureData>,
    texture_of_bitmap: HashMap<DatumIndex, usize>,
    materials: Vec<Material>,
    material_of_shader: HashMap<DatumIndex, usize>,
    lightmap_pages: HashMap<(DatumIndex, usize), usize>,
    mesh_of_object: HashMap<DatumIndex, Option<usize>>,
    failures: usize,
}

impl<'a> Loader<'a> {
    fn new(set: MapSet, log: &'a dyn Fn(&str)) -> Loader<'a> {
        Loader {
            set,
            log,
            textures: vec![TextureData::fallback()],
            texture_of_bitmap: HashMap::new(),
            materials: vec![Material::plain(0)],
            material_of_shader: HashMap::new(),
            lightmap_pages: HashMap::new(),
            mesh_of_object: HashMap::new(),
            failures: 0,
        }
    }

    /// A decoded image as a texture with its mips, if it decoded whole.
    fn add_texture(&mut self, read: blam_cache::Result<Image>) -> Option<usize> {
        let texture = read
            .ok()
            .and_then(|img| TextureData::new(img.width, img.height, img.rgba));
        match texture {
            Some(t) => {
                self.textures.push(t);
                Some(self.textures.len() - 1)
            }
            None => {
                self.failures += 1;
                None
            }
        }
    }

    /// The texture of a bitmap tag's first image (0 if it can't be read).
    fn bitmap_texture(&mut self, b: DatumIndex) -> usize {
        if let Some(&t) = self.texture_of_bitmap.get(&b) {
            return t;
        }
        let read = bitmap::read_bitmap(&mut self.set, b);
        let t = self.add_texture(read).unwrap_or(0);
        self.texture_of_bitmap.insert(b, t);
        t
    }

    fn shader_material(&mut self, shader: DatumIndex) -> usize {
        if let Some(&m) = self.material_of_shader.get(&shader) {
            return m;
        }
        let material = match shader::read_menu_shader(&mut self.set, shader) {
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
                } else if let Some(b) = info.change_color {
                    m.aux = self.bitmap_texture(b);
                    m.aux_kind = AuxKind::ChangeColor;
                } else if let Some((b, scale)) = info.multiply {
                    m.aux = self.bitmap_texture(b);
                    m.aux_kind = match scale > 1.0 {
                        true => AuxKind::Multiply2x,
                        false => AuxKind::Multiply,
                    };
                } else if info.template.contains("emblem_overlay") {
                    m.aux_kind = AuxKind::Emblem;
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
                    (self.log)(&format!(
                        "warning: lightmap unavailable ({e}); the level will look flat"
                    ));
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
        let read = bitmap::read_bitmap_at(&mut self.set, bitmap, page);
        let t = self.add_texture(read);
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

    fn object_mesh(&mut self, object: DatumIndex, meshes: &mut Vec<MeshData>) -> Option<usize> {
        if let Some(&m) = self.mesh_of_object.get(&object) {
            return m;
        }
        let mesh = match model::read_object_render_model(&mut self.set, object) {
            Ok(m) => {
                meshes.push(self.model_mesh(&m));
                Some(meshes.len() - 1)
            }
            Err(_) => None,
        };
        self.mesh_of_object.insert(object, mesh);
        mesh
    }

    /// The level: render geometry grouped by texture and lightmap page.
    fn level(&mut self, bsps: &[StructureBsp]) -> MeshData {
        let mut level = MeshData {
            baked_lighting: true,
            ..MeshData::default()
        };
        for bsp in bsps {
            let geo = match render::bsp_render_geometry(&mut self.set, bsp) {
                Ok(geo) => geo,
                Err(e) => {
                    (self.log)(&format!("warning: render geometry unavailable ({e})"));
                    continue;
                }
            };
            let mats: Vec<usize> = geo
                .shaders
                .iter()
                .map(|&s| self.shader_material(s))
                .collect();
            let lights = self.level_lights(bsp, &geo);
            level.append(MeshData::from_sections(&geo.sections, &mats, &lights));
        }
        level
    }

    /// The first sky's fog and clear colour.
    fn atmosphere(&mut self) -> Atmosphere {
        let Some(sky) = self.set.map.skies().ok().and_then(|s| s.first().copied()) else {
            return Atmosphere::default();
        };
        model::sky_atmosphere(&mut self.set, sky).unwrap_or_else(|e| {
            (self.log)(&format!("warning: sky: {e}"));
            Atmosphere::default()
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
                (self.log)(&format!("warning: sky: {e}"));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::time::Instant;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("h2scene-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_map_is_an_error() {
        let err = load(Path::new("/nowhere/mainmenu.map"), &|_| {}).unwrap_err();
        assert!(err.contains("mainmenu.map"), "{err}");
    }

    #[test]
    fn junk_is_an_error_not_a_panic() {
        let dir = scratch("junk");
        let cases: [(&str, Vec<u8>); 4] = [
            ("empty.map", Vec::new()),
            ("text.map", b"not a map at all, just words".to_vec()),
            ("zeros.map", vec![0; 0x3000]),
            // A Vista header and nothing sensible behind it.
            ("vista.map", {
                let mut b = vec![0xA5; 0x3000];
                b[..4].copy_from_slice(b"daeh");
                b[4..8].copy_from_slice(&8i32.to_le_bytes());
                b
            }),
        ];
        for (name, bytes) in cases {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            assert!(load(&path, &|_| {}).is_err(), "{name}");
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_mcc_map_is_refused_by_its_format() {
        let dir = scratch("mcc");
        let path = dir.join("mainmenu.map");
        let mut b = vec![0u8; 0x1000];
        b[..4].copy_from_slice(b"daeh");
        b[4..8].copy_from_slice(&13i32.to_le_bytes());
        std::fs::write(&path, b).unwrap();
        assert_eq!(
            load(&path, &|_| {}).unwrap_err(),
            "MCC maps have no geometry reader yet"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn placements_turn_like_halo() {
        // Yawed a quarter turn: +x goes to +y. Pitched up: +x rises.
        let yawed = placement_matrix(
            [1.0, 2.0, 3.0],
            [std::f32::consts::FRAC_PI_2, 0.0, 0.0],
            2.0,
        );
        let p = yawed.transform_point3(Vec3::X);
        assert!(p.distance(Vec3::new(1.0, 4.0, 3.0)) < 1e-5, "{p}");
        let pitched = placement_matrix([0.0; 3], [0.0, 0.5, 0.0], 1.0);
        assert!(pitched.transform_vector3(Vec3::X).z > 0.4);
    }

    #[test]
    fn the_scene_crosses_threads() {
        fn send<T: Send + 'static>() {}
        send::<SceneData>();
    }

    /// The owner's mainmenu.map from `H2_MAPS` (Halo 2 Vista, as Project
    /// Cartographer installs it): counts near those of
    /// docs/notes/mainmenu-plan.md.
    #[test]
    #[ignore = "needs H2_MAPS with Halo 2 Vista's mainmenu.map"]
    fn reads_the_real_main_menu() {
        let dir = std::env::var("H2_MAPS").expect("H2_MAPS");
        let path = Path::new(&dir).join("mainmenu.map");
        let logged = RefCell::new(Vec::new());
        let started = Instant::now();
        let scene = load(&path, &|l| logged.borrow_mut().push(l.to_string())).unwrap();
        let ms = started.elapsed().as_millis();
        let (level, objects, sky) = scene.triangles();
        println!(
            "mainmenu.map read in {ms} ms: {} BSPs, triangles {level} level + {objects} \
             scenery + {sky} sky, {} textures ({} MB with mips), {} materials {:?}, \
             {} objects, {} camera points ({} used), {} moves, loop {} ticks ({:.1} s)",
            scene.bsps,
            scene.textures.len(),
            scene.texture_bytes() / 1_000_000,
            scene.materials.len(),
            scene.materials_by_pass(),
            scene.objects.len(),
            scene.flythrough.points(),
            scene.flythrough.points_used(),
            scene.flythrough.moves(),
            scene.flythrough.ticks(),
            scene.flythrough.seconds(),
        );
        for l in logged.borrow().iter() {
            println!("  {l}");
        }
        assert_eq!(scene.bsps, 1);
        // mainmenu-plan.md: 38,738 level triangles (h2viewer's count), and
        // 12 camera points flown through in a loop of 2,990 ticks; within a
        // tenth. Its 48 textures were h2viewer's whole-level loader's, with
        // the players' bodies and weapons; the scene alone takes fewer.
        assert!((34_800..=42_600).contains(&level), "{level} triangles");
        assert!(objects > 0 && sky > 0, "{objects} {sky}");
        assert!(
            (30..=48).contains(&scene.textures.len()),
            "{}",
            scene.textures.len()
        );
        assert_eq!(scene.flythrough.points_used(), 12);
        assert!((scene.flythrough.ticks() - 2990.0).abs() < 1.0);
        assert!(scene.sky.is_some());
        assert!(!scene.objects.is_empty());
        assert_eq!(scene.failed_textures, 0);
        // Every batch's material and lightmap page exist.
        for mesh in std::iter::once(&scene.level).chain(&scene.meshes) {
            for b in &mesh.batches {
                assert!(b.material < scene.materials.len());
                assert!(b.lightmap < scene.textures.len());
                assert!((b.first_index + b.index_count) as usize <= mesh.indices.len());
            }
            assert!(mesh
                .indices
                .iter()
                .all(|&i| (i as usize) < mesh.vertices.len()));
        }
    }
}
