//! CPU-side scene: meshes (level, weapons), their textures, HUD bitmaps and
//! the gameplay data read from the map's tags.

use crate::audio::Clip;
use crate::body::BodyRig;
use crate::probe::LevelLight;
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
use blam_cache::{
    render, scenario, sound, weapon, DatumIndex, GroupTag, MapSet, PlayerSpawn, StructureBsp,
};
use glam::{Mat4, Vec3};
use h2sim::game::FallingDamage;
use h2sim::weapon::Rounds;
use h2sim::{ItemKind, WeaponDef};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

mod vehicles;
pub use vehicles::Vehicles;

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
    /// Where the player's armour colours go (red: primary, green: secondary).
    ChangeColor,
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

#[derive(Clone, Copy)]
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

#[derive(Clone, Default)]
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

    /// A copy with every triangle wound the other way, to draw mirrored
    /// (the left hand's gun and arm when dual wielding).
    pub fn mirrored(&self) -> MeshData {
        let mut m = self.clone();
        for t in m.indices.as_chunks_mut::<3>().0 {
            t.swap(1, 2);
        }
        m
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
    /// The `weap` tag, and which of its triggers this is.
    pub tag: DatumIndex,
    pub trigger: usize,
    pub def: WeaponDef,
    /// Third person model (lying on the map), in `Scene::meshes`.
    pub world_mesh: Option<usize>,
    /// First person model, in `Scene::meshes`.
    pub view_mesh: Option<usize>,
    /// The same, mirrored for the left hand (one-handed weapons).
    pub mirror_mesh: Option<usize>,
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
    pub sounds: WeaponSounds,
    /// How its rounds look and sound, when they fly.
    pub round: RoundAssets,
    /// Enemies this close under the crosshair turn it red.
    pub autoaim_range: f32,
}

/// A weapon's rounds in flight (rockets, plasma bolts, needles...).
#[derive(Default, Clone, Copy, Debug)]
pub struct RoundAssets {
    /// The round's own model (a rocket, a shell), in `Scene::meshes`.
    pub mesh: Option<usize>,
    /// The colour it glows, and how big.
    pub glow: [f32; 4],
    pub size: f32,
    /// Burns (a smoke trail and a fireball) rather than plasma.
    pub fiery: bool,
    /// Going off, or hitting something (in `Scene::sounds`), and going
    /// off all together (needles).
    pub impact: Option<usize>,
    pub supercombine: Option<usize>,
    /// The sound it makes in flight, looped.
    pub flight: Option<usize>,
}

/// How a weapon's rounds look: (glow, size, fiery), by the weapon's name.
fn round_look(weapon: &str) -> ([f32; 4], f32, bool) {
    let has = |s: &str| weapon.contains(s);
    if has("rocket") || has("cannon_turret") || has("brute_shot") {
        ([1.0, 0.65, 0.3, 1.0], 0.07, true)
    } else if has("gauss") {
        ([0.6, 0.8, 1.0, 1.0], 0.06, false)
    } else if has("needle") {
        ([1.0, 0.45, 0.85, 1.0], 0.035, false)
    } else if has("brute_plasma") {
        ([1.0, 0.35, 0.25, 1.0], 0.05, false)
    } else if has("plasma_pistol") {
        ([0.45, 1.0, 0.35, 1.0], 0.05, false)
    } else if has("mortar") {
        ([0.45, 0.6, 1.0, 1.0], 0.25, false)
    } else if has("banshee") || has("flak") {
        ([0.5, 1.0, 0.4, 1.0], 0.08, false)
    } else {
        ([0.35, 0.6, 1.0, 1.0], 0.05, false)
    }
}

/// Sounds a weapon's rounds make going off or hitting, best first.
fn round_impact_sounds(weapon: &str, exploding: bool) -> &'static [&'static str] {
    let has = |s: &str| weapon.contains(s);
    if !exploding {
        if has("needle") {
            &["sound\\weapons\\needler\\expl"]
        } else if has("plasma_pistol") {
            &[
                "sound\\weapons\\plasma_pistol\\plasma_charge_hit",
                "sound\\weapons\\plasma_rifle\\plasma_hit",
            ]
        } else {
            &["sound\\weapons\\plasma_rifle\\plasma_hit"]
        }
    } else if has("brute_shot") {
        &["sound\\weapons\\brute_shot\\brute_round_explode"]
    } else if has("flak") {
        &["sound\\weapons\\flak_cannon\\flak_expl"]
    } else if has("banshee") {
        &[
            "sound\\vehicles\\banshee\\banshee_cannon_expl",
            "sound\\weapons\\flak_cannon\\flak_expl",
        ]
    } else if has("mortar") {
        &["sound\\weapons\\plasma_grenade\\plasma_expl"]
    } else if has("gauss") {
        &["sound\\weapons\\plasma_rifle\\plasma_hit"]
    } else {
        &[
            "sound\\vehicles\\scorpion\\scorpion_explosion",
            "sound\\weapons\\frag_grenade\\frag_expl",
        ]
    }
}

/// A sound ready to play: its variations and how far it carries.
pub struct SoundAsset {
    pub clips: Vec<Arc<Clip>>,
    /// Full volume within the first distance, silent beyond the second
    /// (world units).
    pub distance: (f32, f32),
    /// Linear gain.
    pub gain: f32,
}

/// A weapon's sounds, in `Scene::sounds`.
#[derive(Default, Clone, Copy, Debug)]
pub struct WeaponSounds {
    pub fire: Option<usize>,
    /// Pulling the trigger with nothing loaded.
    pub empty: Option<usize>,
    pub ready: Option<usize>,
    pub reload: Option<usize>,
    pub melee: Option<usize>,
    pub pickup: Option<usize>,
    pub zoom_in: Option<usize>,
    pub zoom_out: Option<usize>,
}

/// Sounds of the game itself, in `Scene::sounds`.
#[derive(Default, Clone, Copy, Debug)]
pub struct GameSounds {
    /// Frag, plasma.
    pub explosion: [Option<usize>; 2],
    pub throw: Option<usize>,
    pub grenade_pickup: [Option<usize>; 2],
    pub footstep: Option<usize>,
    pub jump: Option<usize>,
    pub land: Option<usize>,
    /// A bullet hitting the level.
    pub impact: Option<usize>,
    /// A bullet hitting someone without shields.
    pub hit_body: Option<usize>,
    /// Your shields taking a hit, recharging, and down.
    pub shield_hit: Option<usize>,
    pub shield_charge: Option<usize>,
    pub shield_low: Option<usize>,
    /// Ticks counting down to respawning, and respawning.
    pub respawn_tick: Option<usize>,
    pub respawn: Option<usize>,
    pub announcer: Announcer,
    pub ui: UiSounds,
}

/// The menus' sounds, in `Scene::sounds`.
#[derive(Default, Clone, Copy, Debug)]
pub struct UiSounds {
    /// Moving between items.
    pub cursor: Option<usize>,
    /// Choosing an item.
    pub forward: Option<usize>,
    pub back: Option<usize>,
    /// Starting a game.
    pub advance: Option<usize>,
}

/// Halo 2's main menu music: an opening, then loops picked at random.
pub struct Music {
    pub intro: Arc<Clip>,
    pub loops: Vec<Arc<Clip>>,
}

/// Read the menu music through a map (it lives in shared.map).
pub fn load_music(path: &Path) -> Option<Music> {
    let mut set = MapSet::open(path).ok()?;
    let mut reader = sound::SoundReader::new();
    let mut clips = |name: &str| -> Option<Vec<Arc<Clip>>> {
        let group = GroupTag::parse("snd!")?;
        let datum = set
            .map
            .tags
            .iter()
            .find(|t| t.group == group && t.name == name)?
            .datum;
        let s = reader.read(&mut set, datum).ok()?;
        let (channels, rate) = (s.channels, s.sample_rate);
        Some(
            s.permutations
                .into_iter()
                .map(|samples| {
                    Arc::new(Clip {
                        samples,
                        channels,
                        rate,
                    })
                })
                .collect(),
        )
    };
    let intro = clips("sound\\ui\\main_menu_music\\menu_music\\in")?
        .into_iter()
        .next()?;
    let loops = clips("sound\\ui\\main_menu_music\\menu_music\\loop").unwrap_or_default();
    Some(Music { intro, loops })
}

/// The announcer's lines, in `Scene::sounds`.
#[derive(Default, Clone, Copy, Debug)]
pub struct Announcer {
    /// Each game type's name, in `GameType::ALL` order.
    pub game_names: [Option<usize>; 10],
    pub hill_moved: Option<usize>,
    pub hill_controlled: Option<usize>,
    pub hill_contested: Option<usize>,
    pub ball_taken: Option<usize>,
    pub play_ball: Option<usize>,
    pub new_juggernaut: Option<usize>,
    pub territory_taken: Option<usize>,
    pub territory_lost: Option<usize>,
    pub land_grab: Option<usize>,
    pub bomb_taken: Option<usize>,
    pub bomb_dropped: Option<usize>,
    pub bomb_returned: Option<usize>,
    pub bomb_armed: Option<usize>,
    pub bomb_defused: Option<usize>,
    pub flag_taken: Option<usize>,
    pub flag_returned: Option<usize>,
    pub flag_captured: Option<usize>,
    /// Sounds for the player who took the flag, and who can't score yet.
    pub flag_grabbed: Option<usize>,
    pub flag_failure: Option<usize>,
    pub game_over: Option<usize>,
    /// Double kill (2 kills) to Killimanjaro (7).
    pub multi_kill: [Option<usize>; 6],
    /// Kill sprees of 5, 10, 15, 20 and 25.
    pub spree: [Option<usize>; 5],
    pub suicide: Option<usize>,
    /// Killing a teammate, and being killed by one.
    pub betrayal: Option<usize>,
    pub betrayed: Option<usize>,
    pub gained_lead: Option<usize>,
    pub lost_lead: Option<usize>,
    pub tied_lead: Option<usize>,
}

/// Where the announcer's lines are, under `sound\dialog\multiplayer\`.
const MULTI_KILLS: [&str; 6] = [
    "double_kill",
    "triple_kill",
    "killtacular",
    "kill_frenzy",
    "killtrocity",
    "killimanjaro",
];
/// As the multiplayer globals pair them with 5..25 kills.
const SPREES: [&str; 5] = [
    "killing_spree",
    "running_riot",
    "in_the_zone",
    "untouchable",
    "un_frikin_believable",
];

/// An object placed in the level, drawn with the level's light where it stands.
pub struct SceneObject {
    pub mesh: usize,
    pub transform: Mat4,
    /// Baked light under the object (see `probe::LevelLight::at`).
    pub light: Option<[f32; 3]>,
}

/// A multiplayer item spawn: a weapon or grenades lying on the map.
/// `ItemKind::Weapon` indexes `Scene::weapons`.
pub struct MapItem {
    pub kind: ItemKind,
    pub mesh: Option<usize>,
    pub transform: Mat4,
    pub position: Vec3,
    pub respawn_seconds: f32,
    pub light: Option<[f32; 3]>,
}

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

#[derive(Default, Clone, Copy)]
pub struct GrenadeAssets {
    pub mesh: Option<usize>,
    pub speed: Option<f32>,
}

/// How many Spartans can be on screen at once (each gets its own copy of the
/// body mesh to pose).
pub const MAX_BODIES: usize = 16;

const SPARTAN: &str = "objects\\characters\\masterchief\\masterchief_mp";
const ELITE: &str = "objects\\characters\\elite\\elite_mp";
const SPARTAN_ARMS: &str = "objects\\characters\\masterchief\\fp\\fp";
const ELITE_ARMS: &str = "objects\\characters\\elite\\fp_arms\\fp_arms";

/// A multiplayer Spartan or Elite seen in third person.
pub struct Body {
    pub rig: BodyRig,
    /// One copy of the mesh per Spartan on screen.
    pub meshes: Vec<usize>,
    /// Another, for the profile menu's model.
    pub preview: usize,
}

/// First person arms: Master Chief's or an Elite's.
pub struct Arms {
    pub mesh: usize,
    /// The arms mirrored, for the left hand's gun.
    pub mirror: usize,
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
    /// Multiplayer respawn points.
    pub spawns: Vec<PlayerSpawn>,
    /// Collision geometry, used for walking, shooting and framing.
    pub collision: Mesh,
    pub movement: PlayerMovement,
    pub biped: BipedPhysics,
    /// Weapons the player can switch between; the Battle Rifle first.
    pub weapons: Vec<WeaponAssets>,
    pub arms: Option<Arms>,
    pub elite_arms: Option<Arms>,
    pub body: Option<Body>,
    pub elite: Option<Body>,
    /// The sky's model, drawn around the camera behind everything.
    pub sky: Option<usize>,
    /// Frag and plasma grenades: their models, and throw speed from the
    /// projectile tags.
    pub grenades: [GrenadeAssets; 2],
    pub objects: Vec<SceneObject>,
    pub items: Vec<MapItem>,
    /// The level's baked light, for lighting objects.
    pub level_light: LevelLight,
    /// The player's own HUD (shields, motion tracker, grenades).
    pub player_hud: Vec<HudWidget>,
    /// HUD textures for text and solid fills.
    pub hud_font: usize,
    pub hud_white: usize,
    pub sounds: Vec<SoundAsset>,
    pub game_sounds: GameSounds,
    /// Capture the Flag's flag (also in `weapons`).
    pub flag: Option<FlagAssets>,
    /// Oddball's ball and Assault's bomb, in `weapons`.
    pub ball: Option<usize>,
    pub bomb: Option<usize>,
    /// Where the multiplayer game types' flags, bases and hills are.
    pub netgame_flags: Vec<scenario::NetgameFlag>,
    /// HUD: the arrow over objectives, and the flag icon.
    pub waypoint: Option<usize>,
    pub flag_icon: Option<usize>,
    pub ball_icon: Option<usize>,
    pub bomb_icon: Option<usize>,
    /// Pits and drops that kill.
    pub kill_volumes: Vec<scenario::KillVolume>,
    pub falling: Option<FallingDamage>,
    /// The map's vehicles, and their guns (in `weapons`).
    pub vehicles: Vehicles,
}

/// Capture the Flag's flag: carried in hand like a weapon, with a cloth
/// waving from the top of its pole.
pub struct FlagAssets {
    /// Index into `Scene::weapons`.
    pub weapon: usize,
    /// The cloth's mesh (front and back faces), re-posed as it waves.
    pub cloth: usize,
    /// Cloth points at rest: x along the flag, z down from the pole.
    pub rest: Vec<Vec3>,
    pub uvs: Vec<[f32; 2]>,
    /// Where the cloth hangs from on the pole's world model.
    pub attach: Vec3,
    /// And on the first person pole: the node, and the offset in it.
    pub view_attach: (usize, Vec3),
    /// The stand a flag sits in at its base.
    pub stand: Option<usize>,
}

const FLAG: &str = "objects\\weapons\\multiplayer\\flag\\flag";
const BALL: &str = "objects\\weapons\\multiplayer\\ball\\ball";
const BOMB: &str = "objects\\weapons\\multiplayer\\assault_bomb\\assault_bomb";
const FLAG_STAND: &str = "objects\\multi\\flag_base\\flag_base";

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
    mesh_of_object: HashMap<DatumIndex, Option<usize>>,
    hud_textures: Vec<Image>,
    hud_of_bitmap: HashMap<(DatumIndex, i8), usize>,
    failures: usize,
    sound_reader: sound::SoundReader,
    sounds: Vec<SoundAsset>,
    sound_of_tag: HashMap<DatumIndex, Option<usize>>,
}

/// How far sounds carry when their tag leaves it to the sound class.
fn class_distance(class: u8) -> (f32, f32) {
    match class {
        // projectile impact, detonation
        0 => (0.5, 20.0),
        1 => (8.0, 80.0),
        // weapon fire
        4 => (3.0, 70.0),
        // weapon ready, reload, empty, charge, overheat, idle, melee, animation
        5..=0xC => (0.5, 12.0),
        // footsteps
        0x12 => (0.4, 10.0),
        _ => (1.0, 30.0),
    }
}

impl Loader {
    /// A sound tag, decoded once. `None` for sounds that can't be played.
    fn sound(&mut self, snd: DatumIndex) -> Option<usize> {
        if let Some(&s) = self.sound_of_tag.get(&snd) {
            return s;
        }
        let loaded = match self.sound_reader.read(&mut self.set, snd) {
            Ok(s) if !s.permutations.is_empty() => {
                let distance = if s.distance.1 > 0.0 {
                    s.distance
                } else {
                    class_distance(s.class)
                };
                self.sounds.push(SoundAsset {
                    clips: s
                        .permutations
                        .into_iter()
                        .map(|samples| {
                            Arc::new(Clip {
                                samples,
                                channels: s.channels,
                                rate: s.sample_rate,
                            })
                        })
                        .collect(),
                    distance,
                    gain: 10f32.powf(s.gain_db / 20.0),
                });
                Some(self.sounds.len() - 1)
            }
            Ok(_) => None,
            Err(e) => {
                println!("warning: sound {:08x}: {e}", snd.0);
                None
            }
        };
        self.sound_of_tag.insert(snd, loaded);
        loaded
    }

    fn sound_named(&mut self, name: &str) -> Option<usize> {
        let snd = self.find("snd!", name)?;
        self.sound(snd)
    }

    /// The first sound an effect plays.
    fn effect_sound(&mut self, effect: Option<DatumIndex>) -> Option<usize> {
        let sounds = sound::effect_sounds(&mut self.set, effect?).ok()?;
        sounds.into_iter().find_map(|d| self.sound(d))
    }

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
                } else if let Some(b) = info.change_color {
                    m.aux = self.bitmap_texture(b);
                    m.aux_kind = AuxKind::ChangeColor;
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
        self.weapon_trigger(name, 0, arms, meshes)
    }

    /// A weapon as fired by its trigger `trigger` (vehicle guns have a
    /// second: the Scorpion's machine gun, the Banshee's bomb).
    pub(super) fn weapon_trigger(
        &mut self,
        name: &str,
        trigger: usize,
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
        let barrel = WeaponDef::barrel_of(&w, trigger);
        let barrel_index = w
            .triggers
            .get(trigger)
            .and_then(|t| usize::try_from(t.barrel).ok())
            .unwrap_or(0);
        let projectile = weapon::read_projectile(&mut self.set, barrel.projectile).ok();
        let damage = projectile
            .as_ref()
            .and_then(|p| weapon::read_damage(&mut self.set, p.impact_damage).ok());
        let detonation = projectile
            .as_ref()
            .and_then(|p| weapon::read_damage(&mut self.set, p.detonation_damage).ok());
        let [super_detonation, attached_super] = projectile
            .as_ref()
            .map_or([DatumIndex::NONE; 2], |p| {
                [p.super_detonation_damage, p.attached_super_damage]
            })
            .map(|d| weapon::read_damage(&mut self.set, d).ok());
        let rounds = Rounds {
            projectile: projectile.as_ref(),
            impact: damage.as_ref(),
            detonation: detonation.as_ref(),
            super_detonation: super_detonation.as_ref(),
            attached_super: attached_super.as_ref(),
        };
        let mut def = WeaponDef::from_tags(&w, trigger, rounds);
        def.melee_damage = w
            .melee_damage
            .and_then(|d| weapon::read_damage(&mut self.set, d).ok())
            .map(|d| d.upper_bound.0.max(d.upper_bound.1))
            .filter(|d| *d > 0.0);

        let mut view_mesh = None;
        let mut mirror_mesh = None;
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
                    if def.dual.is_some() {
                        meshes.push(mesh.mirrored());
                        mirror_mesh = Some(meshes.len() - 1);
                    }
                    meshes.push(mesh);
                    view_mesh = Some(meshes.len() - 1);
                }
                Err(e) => println!("warning: first person model for {name}: {e}"),
            }
        }
        let mut anim_sounds = (None, None, None);
        let rig = match (w.first_person_animations, arms) {
            (Some(jmad), Some(arms)) if view_mesh.is_some() => {
                match animation::read_animation_graph(&mut self.set, jmad) {
                    Ok(g) => {
                        // Sounds the first person animations start.
                        for (what, slot) in [
                            ("first_person:ready", &mut anim_sounds.0),
                            ("first_person:reload_full", &mut anim_sounds.1),
                            ("first_person:melee_strike_1", &mut anim_sounds.2),
                        ] {
                            *slot = g
                                .find(what)
                                .and_then(|a| a.sound_events.first())
                                .and_then(|&(_, i)| g.sounds.get(i).copied().flatten());
                        }
                        Some(FirstPersonRig::new(g, arms, &skeleton))
                    }
                    Err(e) => {
                        println!("warning: first person animations for {name}: {e}");
                        None
                    }
                }
            }
            _ => None,
        };
        let hud = w.hud.map(|h| self.hud_widgets(h)).unwrap_or_default();
        let world_mesh = self.object_mesh(datum, meshes);
        let fx =
            weapon::read_weapon_effects(&mut self.set, datum, barrel_index).unwrap_or_default();
        let sounds = WeaponSounds {
            fire: self.effect_sound(fx.fire),
            empty: self.effect_sound(fx.empty),
            ready: anim_sounds
                .0
                .and_then(|d| self.sound(d))
                .or_else(|| self.effect_sound(fx.ready)),
            reload: anim_sounds
                .1
                .and_then(|d| self.sound(d))
                .or_else(|| self.effect_sound(fx.reload)),
            melee: anim_sounds.2.and_then(|d| self.sound(d)),
            pickup: self.effect_sound(fx.pickup),
            zoom_in: self.effect_sound(fx.zoom_in),
            zoom_out: self.effect_sound(fx.zoom_out),
        };
        let round = match def.flight {
            Some(f) => {
                let (glow, size, fiery) = round_look(name);
                let exploding = f.blast.is_some();
                RoundAssets {
                    mesh: self.object_mesh(barrel.projectile, meshes),
                    glow,
                    size,
                    fiery,
                    impact: round_impact_sounds(name, exploding)
                        .iter()
                        .find_map(|n| self.sound_named(n)),
                    supercombine: f.sticky.and_then(|_| {
                        self.sound_named("sound\\weapons\\needler\\needler_super_expl")
                    }),
                    flight: self.attached_loop(barrel.projectile),
                }
            }
            None => RoundAssets::default(),
        };
        Some(WeaponAssets {
            tag: datum,
            trigger,
            world_mesh,
            round,
            def,
            view_mesh,
            mirror_mesh,
            skeleton,
            skin,
            muzzle_node,
            muzzle,
            grip,
            rig,
            hud,
            sounds,
            autoaim_range: w.autoaim_range,
        })
    }

    /// A HUD bitmap's first image, by tag name.
    /// How hard landings hurt, from the globals.
    fn falling(&mut self) -> Option<FallingDamage> {
        let f = physics::falling_damage(&mut self.set).ok().flatten()?;
        let hurt = weapon::read_damage(&mut self.set, f.falling).ok()?;
        Some(FallingDamage {
            harmful: f.harmful_distance,
            damage: hurt.upper_bound.0.max(hurt.upper_bound.1),
            deadly: f.maximum_distance,
        })
    }

    fn hud_bitmap(&mut self, name: &str) -> Option<usize> {
        let datum = self.find("bitm", name)?;
        let img = bitmap::read_bitmap_at(&mut self.set, datum, 0).ok()?;
        self.hud_textures.push(img);
        Some(self.hud_textures.len() - 1)
    }

    /// The flag: its pole as a weapon (added to `weapons`), its cloth and
    /// its stand.
    fn flag(
        &mut self,
        arms: Option<&Arms>,
        weapons: &mut Vec<WeaponAssets>,
        meshes: &mut Vec<MeshData>,
    ) -> Option<FlagAssets> {
        let assets = self.weapon(FLAG, arms.map(|a| &a.skeleton), meshes)?;
        let datum = assets.tag;
        let cloth = self
            .find("clwd", FLAG)
            .and_then(|c| model::read_cloth(&mut self.set, c).ok())?;
        // Where the cloth hangs on the world and first person poles.
        let marker_at = |set: &mut MapSet, mode: DatumIndex| -> Option<(usize, Vec3, Vec3)> {
            let m = model::read_render_model(set, mode).ok()?;
            let mk = m.marker("flag_attach")?;
            let node = mk.node as usize;
            let bind = Skeleton::new(&m.nodes)
                .inverse_bind
                .get(node)
                .map_or(Mat4::IDENTITY, Mat4::inverse);
            let local = Vec3::from(mk.translation);
            Some((node, local, bind.transform_point3(local)))
        };
        let world_model = model::object_render_model(&mut self.set, datum).ok();
        let attach = world_model
            .and_then(|m| marker_at(&mut self.set, m))
            .map_or(Vec3::Z * 0.8, |(_, _, at)| at);
        let fp = weapon::read_weapon(&mut self.set, datum)
            .ok()
            .and_then(|w| w.first_person_model);
        let view_attach = fp
            .and_then(|m| marker_at(&mut self.set, m))
            .map_or((0, Vec3::ZERO), |(node, local, _)| (node, local));
        // A plain cloth that takes the team's colour.
        self.textures.push(Image {
            width: 1,
            height: 1,
            rgba: vec![255; 4],
        });
        let white = self.textures.len() - 1;
        self.materials.push(Material {
            texture: white,
            aux: white,
            aux_kind: AuxKind::ChangeColor,
            tint: [1.0; 3],
            opacity: 1.0,
            ..Material::default()
        });
        let material = self.materials.len() - 1;
        let (cols, rows) = cloth.grid;
        let n = cols * rows;
        let rest: Vec<Vec3> = cloth.vertices.iter().map(|v| Vec3::from(v.0)).collect();
        let uvs: Vec<[f32; 2]> = cloth.vertices.iter().map(|v| v.1).collect();
        let mut mesh = MeshData::default();
        for side in 0..2 {
            for k in 0..n {
                let normal = if side == 0 {
                    [0.0, -1.0, 0.0]
                } else {
                    [0.0, 1.0, 0.0]
                };
                mesh.vertices
                    .push(Vertex::new(rest[k].into(), normal, uvs[k]));
            }
        }
        for r in 0..rows - 1 {
            for c in 0..cols - 1 {
                let a = (r * cols + c) as u32;
                let (b, d, e) = (a + 1, a + cols as u32, a + cols as u32 + 1);
                mesh.indices.extend([a, d, b, b, d, e]);
                let o = n as u32;
                mesh.indices
                    .extend([a + o, b + o, d + o, b + o, e + o, d + o]);
            }
        }
        mesh.batches = vec![Batch {
            material,
            lightmap: 0,
            first_index: 0,
            index_count: mesh.indices.len() as u32,
        }];
        meshes.push(mesh);
        let cloth_mesh = meshes.len() - 1;
        let stand = self
            .find("scen", FLAG_STAND)
            .and_then(|s| self.object_mesh(s, meshes));
        weapons.push(assets);
        Some(FlagAssets {
            weapon: weapons.len() - 1,
            cloth: cloth_mesh,
            rest,
            uvs,
            attach,
            view_attach,
            stand,
        })
    }

    /// The mesh of an object tag's render model, loaded once.
    fn object_mesh(&mut self, object: DatumIndex, meshes: &mut Vec<MeshData>) -> Option<usize> {
        if let Some(&m) = self.mesh_of_object.get(&object) {
            return m;
        }
        let mesh = match model::object_render_model(&mut self.set, object)
            .and_then(|mode| model::read_render_model(&mut self.set, mode))
        {
            Ok(m) => {
                meshes.push(self.model_mesh(&m));
                Some(meshes.len() - 1)
            }
            Err(_) => None,
        };
        self.mesh_of_object.insert(object, mesh);
        mesh
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

    /// A multiplayer biped's body: the Spartan or the Elite, with the
    /// markers its hands hold weapons by.
    fn body(&mut self, bipd: &str, hands: [&str; 2], meshes: &mut Vec<MeshData>) -> Option<Body> {
        let tag = self.find("bipd", bipd)?;
        let loaded = model::object_render_model(&mut self.set, tag)
            .and_then(|mode| model::read_render_model(&mut self.set, mode))
            .and_then(|m| {
                let jmad = model::object_animations(&mut self.set, tag)?;
                Ok((m, animation::read_animation_graph(&mut self.set, jmad)?))
            });
        let (m, graph) = match loaded {
            Ok(x) => x,
            Err(e) => {
                println!("warning: {bipd}: {e}");
                return None;
            }
        };
        let mesh = self.model_mesh(&m);
        let skin = SkinnedMesh::new(&mesh);
        let first = meshes.len();
        for _ in 0..=MAX_BODIES {
            meshes.push(mesh.clone());
        }
        Some(Body {
            rig: BodyRig::new(
                graph,
                Skeleton::new(&m.nodes),
                skin,
                hands.map(|h| m.marker(h).copied()),
            ),
            meshes: (first..first + MAX_BODIES).collect(),
            preview: first + MAX_BODIES,
        })
    }

    fn arms(&mut self, name: &str, meshes: &mut Vec<MeshData>) -> Option<Arms> {
        let mode = self.find("mode", name)?;
        let m = match model::read_render_model(&mut self.set, mode) {
            Ok(m) => m,
            Err(e) => {
                println!("warning: {name}: {e}");
                return None;
            }
        };
        let mesh = self.model_mesh(&m);
        let skin = SkinnedMesh::new(&mesh);
        meshes.push(mesh.mirrored());
        meshes.push(mesh);
        Some(Arms {
            mesh: meshes.len() - 1,
            mirror: meshes.len() - 2,
            skeleton: Skeleton::new(&m.nodes),
            skin,
        })
    }
}

impl Scene {
    /// The third person body for a Spartan or an Elite (the Spartan's if
    /// the map has no Elite).
    pub fn body_for(&self, elite: bool) -> Option<&Body> {
        if elite {
            self.elite.as_ref().or(self.body.as_ref())
        } else {
            self.body.as_ref()
        }
    }

    /// First person arms for a Spartan or an Elite.
    pub fn arms_for(&self, elite: bool) -> Option<&Arms> {
        if elite {
            self.elite_arms.as_ref().or(self.arms.as_ref())
        } else {
            self.arms.as_ref()
        }
    }

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
        let spawns = set.map.player_spawns().unwrap_or_default();
        let movement = physics::player_movement(&mut set).unwrap_or_default();
        let biped = physics::player_biped(&mut set).unwrap_or_default();

        let mut loader = Loader {
            set,
            textures: vec![fallback_texture()],
            texture_of_bitmap: HashMap::new(),
            materials: vec![Material::plain(0)],
            material_of_shader: HashMap::new(),
            lightmap_pages: HashMap::new(),
            mesh_of_object: HashMap::new(),
            hud_textures: Vec::new(),
            hud_of_bitmap: HashMap::new(),
            failures: 0,
            sound_reader: sound::SoundReader::new(),
            sounds: Vec::new(),
            sound_of_tag: HashMap::new(),
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
        let arms = loader.arms(SPARTAN_ARMS, &mut meshes);
        let elite_arms = loader.arms(ELITE_ARMS, &mut meshes);
        let body = loader.body(SPARTAN, ["right_hand", "left_hand"], &mut meshes);
        let elite = loader.body(ELITE, ["right_hand_elite", "left_hand_elite"], &mut meshes);
        let weapons = WEAPONS
            .iter()
            .filter_map(|name| loader.weapon(name, arms.as_ref().map(|a| &a.skeleton), &mut meshes))
            .collect();
        let mut weapons: Vec<WeaponAssets> = weapons;
        let flag = loader.flag(arms.as_ref(), &mut weapons, &mut meshes);
        let mut carried = |name: &str| {
            let w = loader.weapon(name, arms.as_ref().map(|a| &a.skeleton), &mut meshes)?;
            weapons.push(w);
            Some(weapons.len() - 1)
        };
        let (ball, bomb) = (carried(BALL), carried(BOMB));
        let netgame_flags = scenario::netgame_flags(&mut loader.set).unwrap_or_default();
        let kill_volumes = scenario::kill_volumes(&mut loader.set).unwrap_or_default();
        let falling = loader.falling();
        let level_light = LevelLight::new(&meshes[0]);
        let mut objects = Vec::new();
        for p in scenario::scenery(&mut loader.set).unwrap_or_default() {
            if let Some(mesh) = loader.object_mesh(p.object, &mut meshes) {
                let light =
                    level_light.at(&loader.textures, Vec3::from(p.position) + Vec3::Z * 0.2);
                objects.push(SceneObject {
                    mesh,
                    transform: placement_matrix(p.position, p.rotation, p.scale),
                    light,
                });
            }
        }
        let mut items = Vec::new();
        for spawn in scenario::netgame_equipment(&mut loader.set).unwrap_or_default() {
            let collection = scenario::item_collection(&mut loader.set, spawn.collection);
            let Some(&(_, item)) = collection.ok().and_then(|c| c.first().copied()).as_ref() else {
                continue;
            };
            let name = loader
                .set
                .locate(item)
                .map(|(_, t)| t.name)
                .unwrap_or_default();
            let kind = if let Some(w) = weapons.iter().position(|w| w.tag == item) {
                ItemKind::Weapon(w)
            } else if name.ends_with("frag_grenade") {
                ItemKind::FragGrenades
            } else if name.ends_with("plasma_grenade") {
                ItemKind::PlasmaGrenades
            } else {
                continue;
            };
            let mesh = match kind {
                ItemKind::Weapon(w) => weapons[w].world_mesh,
                _ => loader.object_mesh(item, &mut meshes),
            };
            let light =
                level_light.at(&loader.textures, Vec3::from(spawn.position) + Vec3::Z * 0.2);
            items.push(MapItem {
                kind,
                mesh,
                // Items settle on the ground in the game; lay them on their
                // side rather than as placed in the editor.
                transform: placement_matrix(
                    spawn.position,
                    [spawn.rotation[0], 0.0, std::f32::consts::FRAC_PI_2],
                    1.0,
                ),
                position: Vec3::from(spawn.position),
                respawn_seconds: match spawn.respawn_seconds {
                    0 => 30.0,
                    s => s as f32,
                },
                light,
            });
        }
        let vehicles = loader.vehicles(&mut weapons, &mut meshes);
        let grenades = ["frag_grenade", "plasma_grenade"].map(|g| {
            let name = format!("objects\\weapons\\grenade\\{g}\\{g}");
            GrenadeAssets {
                mesh: loader
                    .find("eqip", &name)
                    .and_then(|e| loader.object_mesh(e, &mut meshes)),
                speed: loader
                    .find("proj", &name)
                    .and_then(|p| weapon::read_projectile(&mut loader.set, p).ok())
                    .map(|p| p.initial_velocity)
                    .filter(|v| *v > 0.0),
            }
        });
        let named = |loader: &mut Loader, name: &str| loader.sound_named(name);
        let game_sounds = GameSounds {
            explosion: [
                named(&mut loader, "sound\\weapons\\frag_grenade\\frag_expl"),
                named(&mut loader, "sound\\weapons\\plasma_grenade\\plasma_expl"),
            ],
            throw: named(&mut loader, "sound\\weapons\\frag_grenade\\frag_throw"),
            grenade_pickup: [
                named(&mut loader, "sound\\weapons\\frag_grenade\\frag_ammo"),
                named(
                    &mut loader,
                    "sound\\weapons\\plasma_grenade\\grenade_plas_ammo",
                ),
            ],
            footstep: named(
                &mut loader,
                "sound\\characters\\footsteps\\chief\\grtcement\\run",
            ),
            jump: named(
                &mut loader,
                "sound\\characters\\footsteps\\chief\\grtcement\\jump",
            ),
            land: named(
                &mut loader,
                "sound\\characters\\footsteps\\chief\\grtcement\\land",
            ),
            impact: named(
                &mut loader,
                "sound\\weapons\\impacts_riccs\\bullet_impact_stone",
            ),
            hit_body: named(
                &mut loader,
                "sound\\weapons\\impacts_riccs\\bullet_impact_flesh_dry",
            ),
            shield_hit: named(&mut loader, "sound\\ui\\shield_hit"),
            shield_charge: named(&mut loader, "sound\\ui\\shield_charge\\charge\\loop"),
            shield_low: named(&mut loader, "sound\\ui\\shield_low\\low\\loop"),
            respawn_tick: named(
                &mut loader,
                "sound\\game_sfx\\multiplayer\\countdown_for_respawn",
            ),
            respawn: named(&mut loader, "sound\\game_sfx\\multiplayer\\player_respawn"),
            ui: UiSounds {
                cursor: named(&mut loader, "sound\\ui\\cursor1"),
                forward: named(&mut loader, "sound\\ui\\forward1"),
                back: named(&mut loader, "sound\\ui\\back1"),
                advance: named(&mut loader, "sound\\ui\\advance"),
            },
            announcer: {
                let flag_grabbed =
                    loader.sound_named("sound\\game_sfx\\multiplayer\\target_point_collected");
                let flag_failure = loader.sound_named("sound\\game_sfx\\multiplayer\\flag_failure");
                let mut line =
                    |name: &str| loader.sound_named(&format!("sound\\dialog\\multiplayer\\{name}"));
                Announcer {
                    game_names: [
                        "slayer",
                        "team_slayer",
                        "capture_the_flag",
                        "king_of_the_hill",
                        "team_king",
                        "oddball",
                        "team_oddball",
                        "juggernaut",
                        "territories",
                        "assault",
                    ]
                    .map(|n| {
                        // Team variants fall back to the plain name.
                        let plain = n.strip_prefix("team_").unwrap_or(n);
                        let plain = if plain == "king" {
                            "king_of_the_hill"
                        } else {
                            plain
                        };
                        line(&format!("games\\names\\{n}"))
                            .or_else(|| line(&format!("games\\names\\{plain}")))
                    }),
                    hill_moved: line("games\\king\\hill_moved"),
                    hill_controlled: line("games\\king\\hill_controlled"),
                    hill_contested: line("games\\king\\hill_contested"),
                    ball_taken: line("games\\oddball\\ball_taken"),
                    play_ball: line("games\\oddball\\play_ball"),
                    new_juggernaut: line("games\\juggernaut\\new_juggernaut"),
                    territory_taken: line("games\\territories\\territory_controlled"),
                    territory_lost: line("games\\territories\\territory_lost"),
                    land_grab: line("games\\territories\\land_grab"),
                    bomb_taken: line("games\\invasion\\bomb_taken"),
                    bomb_dropped: line("games\\invasion\\bomb_dropped"),
                    bomb_returned: line("games\\invasion\\bomb_returned"),
                    bomb_armed: line("games\\invasion\\bomb_armed"),
                    bomb_defused: line("games\\invasion\\bomb_defused"),
                    flag_taken: line("games\\ctf\\flag_taken"),
                    flag_returned: line("games\\ctf\\flag_returned"),
                    flag_captured: line("games\\ctf\\flag_captured"),
                    flag_grabbed,
                    flag_failure,
                    game_over: line("general\\misc\\game_over"),
                    multi_kill: MULTI_KILLS.map(|n| line(&format!("flavor\\{n}"))),
                    spree: SPREES.map(|n| line(&format!("flavor\\{n}"))),
                    suicide: line("general\\misc\\suicide"),
                    betrayal: line("general\\misc\\betrayal"),
                    betrayed: line("general\\misc\\betrayed"),
                    gained_lead: line("general\\misc\\gained_the_lead"),
                    lost_lead: line("general\\misc\\lost_the_lead"),
                    tied_lead: line("general\\misc\\tied_the_leader"),
                }
            },
        };
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
        let waypoint = loader.hud_bitmap("ui\\hud\\bitmaps\\new_hud\\hud_waypoints");
        let flag_icon = loader.hud_bitmap("ui\\hud\\bitmaps\\new_hud\\reset_flag");
        let ball_icon = loader.hud_bitmap("ui\\hud\\bitmaps\\new_hud\\reset_ball");
        let bomb_icon = loader.hud_bitmap("ui\\hud\\bitmaps\\new_hud\\reset_bomb");
        if loader.failures > 0 {
            println!("warning: {} textures couldn't be decoded", loader.failures);
        }
        Ok(Scene {
            textures: loader.textures,
            materials: loader.materials,
            meshes,
            hud_textures: loader.hud_textures,
            spawns,
            collision,
            movement,
            biped,
            weapons,
            arms,
            elite_arms,
            body,
            elite,
            sky,
            grenades,
            objects,
            items,
            level_light,
            player_hud,
            hud_font,
            hud_white,
            sounds: loader.sounds,
            game_sounds,
            flag,
            ball,
            bomb,
            netgame_flags,
            waypoint,
            flag_icon,
            ball_icon,
            bomb_icon,
            kill_volumes,
            falling,
            vehicles,
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
