//! Campaign actors from the map's tags: each character's body, how tough
//! it is and how it fights (`char`), the squads the mission places
//! (`scnr`), and the weapons they carry. And the mission's scripts, with
//! the trigger volumes and named objects they work with.

use super::{Arms2, Body, Loader, MeshData, WeaponAssets};
use crate::rig::{Skeleton, SkinnedMesh};
use crate::scene::placement_matrix;
use blam_cache::ai::{self, Character, CharacterKind, Squad};
use blam_cache::animation::{self, AnimationGraph};
use blam_cache::orders;
use blam_cache::physics::{self, BipedPhysics};
use blam_cache::scenario::{self, PlacedKind};
use blam_cache::script::{self, Scripts};
use blam_cache::{model, text, vehicle, DatumIndex};
use glam::{Mat4, Vec3};
use h2sim::game::{CharacterDef, GrenadeKind, Mind, Side, Vitality};
use h2sim::KillZone;
use std::collections::HashMap;

/// Mesh copies of each actor body: how many of one kind show at once.
pub const ACTOR_BODIES: usize = 16;

/// A mission's actors: its characters (by the scenario's character
/// palette), their bodies, its squads and the weapons squads carry.
#[derive(Default)]
pub struct CampaignAi {
    pub characters: Vec<CharacterDef>,
    /// Each character's body, by index into `bodies`.
    pub body_of: Vec<Option<usize>>,
    pub bodies: Vec<Body>,
    /// Each character's armour colours.
    pub colors: Vec<[[f32; 3]; 2]>,
    /// Each character's accuracy on legendary (the defs have normal's).
    pub legendary_accuracy: Vec<(f32, f32)>,
    pub squads: Vec<Squad>,
    /// The scenario's weapon palette, as weapon indices.
    pub weapons: Vec<Option<usize>>,
    /// Each squad group's parent group, and the order its squads start
    /// with.
    pub group_parents: Vec<Option<u16>>,
    pub group_orders: Vec<Option<u16>>,
    /// Where squads go (firing positions in zones' areas), the orders
    /// sending them there, and the triggers that end orders.
    pub zones: Vec<orders::Zone>,
    pub orders: Vec<orders::Order>,
    pub triggers: Vec<orders::AiTrigger>,
    /// Points scripts send actors to.
    pub point_sets: Vec<orders::PointSet>,
    /// The seats scripts name ("warthog_d"): per entry, the names of the
    /// seats it picks out.
    pub seat_mappings: Vec<Vec<String>>,
    /// The names behind the string ids scripts use.
    pub string_ids: HashMap<u32, String>,
    pub scripts: Scripts,
    /// Trigger volumes, by index.
    pub volumes: Vec<KillZone>,
    /// Walking into a volume moves the game from one structure BSP to
    /// another: (volume, from, to).
    pub bsp_switches: Vec<(u16, u16, u16)>,
    /// The names scripts give placed objects, and where each is placed.
    pub object_names: Vec<String>,
    pub name_positions: Vec<Option<Vec3>>,
    /// The cutscene flags scripts teleport things to: where each is and
    /// which way it faces (yaw).
    pub flags: Vec<(Vec3, f32)>,
    /// Each named machine (door, lift): how it moves.
    pub machines: Vec<Option<scenario::Machine>>,
    /// The device groups switches and scripts set.
    pub device_groups: Vec<scenario::DeviceGroup>,
    /// The titles scripts show (chapter names), and the mission's
    /// objectives.
    pub titles: Vec<Title>,
    pub objectives: Vec<String>,
    /// The sounds scripts play (dialogue), by tag: in `Scene::sounds`.
    pub sounds: HashMap<u32, usize>,
    /// The music and loops scripts start, by tag.
    pub loops: HashMap<u32, ScriptLoop>,
    /// Effects (and damage) scripts set off, by tag.
    pub effects: HashMap<u32, ScriptEffect>,
    /// The mission dialogue lines scripts have actors say, by the string
    /// id scripts name them with: each voice's designator and its sound
    /// (in `Scene::sounds`).
    pub lines: HashMap<u32, Vec<(String, usize)>>,
    /// Each character's voices (dialogue designators).
    pub voices: Vec<Vec<String>>,
    /// The little scenes scripts stage with actors who fit their roles.
    pub scenes: Vec<orders::MissionScene>,
    /// The AI triggers scripts test by name, by the value scripts name
    /// each with.
    pub trigger_names: HashMap<u32, u16>,
    /// What the cutscenes play.
    pub cinema: Cinema,
}

/// What an effect a script sets off looks like, roughly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EffectLook {
    /// A fireball (`scale` times a grenade's), blue for plasma.
    Explosion {
        plasma: bool,
        scale: f32,
    },
    Smoke,
    /// A small glow of light (Cortana appearing, a beam charging).
    Glow([f32; 4]),
}

/// An effect a script sets off: its sounds and how it looks.
#[derive(Debug, Clone, Default)]
pub struct ScriptEffect {
    pub sounds: Vec<usize>,
    pub look: Option<EffectLook>,
}

/// How an effect looks, going by its tag's name.
fn effect_look(name: &str) -> Option<EffectLook> {
    let has = |k: &str| name.contains(k);
    let blue = has("plasma") || has("covenant") || has("cortana") || has("teleport");
    if [
        "explosion",
        "air_hit",
        "detonation",
        "blast",
        "bomb",
        "attack",
    ]
    .iter()
    .any(|k| has(k))
    {
        let scale = if has("large") || has("vehicle") || has("ships") {
            2.0
        } else if has("small") {
            0.7
        } else {
            1.0
        };
        return Some(EffectLook::Explosion {
            plasma: blue,
            scale,
        });
    }
    if has("smoke") || has("burn") || has("charred") {
        return Some(EffectLook::Smoke);
    }
    if [
        "spark",
        "charging",
        "beam",
        "teleport",
        "on_off",
        "data_transfer",
        "glow",
    ]
    .iter()
    .any(|k| has(k))
    {
        return Some(EffectLook::Glow(if blue || has("data") {
            [0.45, 0.7, 1.0, 0.9]
        } else {
            [1.0, 0.8, 0.45, 0.9]
        }));
    }
    None
}

/// What the mission's cutscenes play: the animation graphs scripts name,
/// the named bipeds they animate, where named objects are placed (the
/// anchors cutscenes play relative to) and the subtitles.
#[derive(Default)]
pub struct Cinema {
    /// By tag (the value scripts name each with).
    pub graphs: HashMap<u32, AnimationGraph>,
    /// The named bipeds and vehicles cutscenes animate, by object name.
    pub bodies: HashMap<u16, CinemaBody>,
    /// Their tags, while loading.
    cast: Vec<(u16, DatumIndex, String)>,
    /// Each named object's placement.
    pub placed: Vec<Option<Mat4>>,
    /// Subtitle text by string id.
    pub subtitles: HashMap<u32, String>,
}

/// A biped or vehicle a cutscene animates (Master Chief, Johnson, a
/// Pelican...): its own copy of its model's mesh, posed on the CPU.
pub struct CinemaBody {
    pub mesh: usize,
    /// It's Master Chief: he wears the player's colours.
    pub chief: bool,
    /// The colours its armour or skin takes, where the model leaves them
    /// to the game (Brutes' fur).
    pub colors: Option<[[f32; 3]; 2]>,
    pub skeleton: Skeleton,
    pub parents: Vec<i16>,
    pub skin: SkinnedMesh,
    /// Its other looks, a script having swapped one region's permutation
    /// (a Marine's face): region and permutation (string ids), mesh, skin.
    pub looks: Vec<(u32, u32, usize, SkinnedMesh)>,
}

/// A title scripts put on screen: its text, where (top, left, bottom,
/// right on Halo 2's 640 x 480 screen), and seconds to fade in, stay up
/// and fade out.
#[derive(Debug, Clone, Default)]
pub struct Title {
    pub text: String,
    pub bounds: [f32; 4],
    pub fade_in: f32,
    pub up: f32,
    pub fade_out: f32,
}

/// Music (or another looping sound) a script starts: its opening, the
/// part that loops, and its ending, in `Scene::sounds`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScriptLoop {
    pub start: Option<usize>,
    pub repeat: Option<usize>,
    pub end: Option<usize>,
    /// Linear gain.
    pub gain: f32,
    /// Plays once rather than looping.
    pub once: bool,
}

/// Which side a kind of character fights on.
fn side(kind: CharacterKind) -> Side {
    use CharacterKind::*;
    match kind {
        Marine | Player | Crew => Side::Human,
        CombatForm | InfectionForm | CarrierForm | Juggernaut => Side::Flood,
        Monitor | Sentinel => Side::Sentinel,
        _ => Side::Covenant,
    }
}

/// Armour colours by rank, from the character's name (Elite majors are
/// red, ultras white...).
fn rank_colors(name: &str, kind: CharacterKind) -> [[f32; 3]; 2] {
    let rank = name.rsplit('\\').next().unwrap_or(name);
    let has = |s: &str| rank.contains(s);
    let c = match kind {
        CharacterKind::Elite if has("zealot") => [0.85, 0.65, 0.2],
        CharacterKind::Elite if has("ultra") => [0.85, 0.85, 0.88],
        CharacterKind::Elite if has("major") => [0.75, 0.15, 0.12],
        CharacterKind::Elite if has("specops") || has("stealth") => [0.2, 0.2, 0.24],
        CharacterKind::Elite => [0.2, 0.3, 0.8],
        CharacterKind::Grunt if has("ultra") => [0.75, 0.75, 0.8],
        CharacterKind::Grunt if has("major") || has("heavy") => [0.75, 0.15, 0.1],
        CharacterKind::Grunt if has("specops") => [0.2, 0.2, 0.24],
        CharacterKind::Grunt => [0.9, 0.5, 0.15],
        CharacterKind::Jackal if has("major") => [0.75, 0.15, 0.1],
        CharacterKind::Jackal => [0.25, 0.35, 0.75],
        CharacterKind::Marine => [0.36, 0.42, 0.26],
        _ => [0.5, 0.5, 0.5],
    };
    [c, c.map(|v| v * 0.6)]
}

fn vitality(body: f32, shield: f32, recharge: f32) -> Vitality {
    Vitality {
        shield: shield.max(0.0),
        // Some characters leave it to their biped: about a Spartan's.
        health: if body > 0.0 { body } else { 45.0 },
        recharge: if recharge > 0.0 { recharge } else { 5.0 },
    }
}

/// A character's def for the game: its side, vitality and mind.
fn character_def(
    c: &Character,
    biped: BipedPhysics,
    weapon: Option<usize>,
) -> (CharacterDef, (f32, f32)) {
    let v = &c.vitality;
    let p = &c.perception;
    let w = c.weapon.unwrap_or_default();
    let positive = |x: f32, or: f32| if x > 0.0 { x } else { or };
    let defaults = Mind::default();
    let g = c.grenades.unwrap_or_default();
    let mind = Mind {
        sight: positive(p.vision_distance, defaults.sight),
        fov: positive(p.vision_angle, defaults.fov),
        peripheral: positive(p.peripheral_distance, defaults.peripheral),
        fire_range: positive(w.max_range, defaults.fire_range),
        combat_range: if w.combat_range.1 > 0.0 {
            w.combat_range
        } else {
            defaults.combat_range
        },
        accuracy: if w.accuracy.1 > 0.0 {
            w.accuracy
        } else {
            defaults.accuracy
        },
        melee_range: positive(c.melee_range, defaults.melee_range),
        melee_chance: c.melee_chance.max(0.0),
        grenade_chance: c.grenades.map_or(0.0, |g| g.chance.max(0.0)),
        grenade_delay: positive(g.delay, defaults.grenade_delay),
        grenade_range: if g.range.1 > 0.0 {
            g.range
        } else {
            defaults.grenade_range
        },
        skittish: c.kind == CharacterKind::Grunt,
    };
    let legendary = if w.legendary_accuracy.1 > 0.0 {
        w.legendary_accuracy
    } else {
        mind.accuracy
    };
    let def = CharacterDef {
        name: c.name.rsplit('\\').next().unwrap_or(&c.name).to_uppercase(),
        side: side(c.kind),
        biped,
        vitality: vitality(v.body, v.shield, v.shield_recharge_time),
        legendary: vitality(
            positive(v.legendary_body, v.body),
            positive(v.legendary_shield, v.shield),
            v.shield_recharge_time,
        ),
        mind,
        weapon,
        grenade: if g.kind == 1 {
            GrenadeKind::Plasma
        } else {
            GrenadeKind::Frag
        },
        grenades: (g.count.0 as u32 + g.count.1 as u32).div_ceil(2).min(4) as u8,
    };
    (def, legendary)
}

impl Loader {
    /// The mission's scripts and what they name.
    fn mission_scripts(&mut self, out: &mut CampaignAi) {
        let set = &mut self.set;
        out.scripts = script::scripts(set).unwrap_or_else(|e| {
            println!("warning: scripts: {e}");
            Scripts::default()
        });
        let groups = ai::squad_groups(set).unwrap_or_default();
        out.group_parents = groups.iter().map(|g| g.parent).collect();
        out.group_orders = groups.iter().map(|g| g.order).collect();
        out.zones = orders::zones(set).unwrap_or_else(|e| {
            println!("warning: zones: {e}");
            Vec::new()
        });
        out.orders = orders::orders(set).unwrap_or_else(|e| {
            println!("warning: orders: {e}");
            Vec::new()
        });
        out.triggers = orders::ai_triggers(set).unwrap_or_else(|e| {
            println!("warning: AI triggers: {e}");
            Vec::new()
        });
        out.scenes = orders::mission_scenes(set).unwrap_or_else(|e| {
            println!("warning: mission scenes: {e}");
            Vec::new()
        });
        out.point_sets = orders::point_sets(set).unwrap_or_else(|e| {
            println!("warning: point sets: {e}");
            Vec::new()
        });
        for (unit, seats) in ai::unit_seat_mappings(set).unwrap_or_default() {
            let names = vehicle::read_vehicle(set, unit)
                .map(|v| {
                    (0..v.seats.len().min(32))
                        .filter(|k| seats & (1 << k) != 0)
                        .map(|k| v.seats[k].animation.clone())
                        .collect()
                })
                .unwrap_or_default();
            out.seat_mappings.push(names);
        }
        out.string_ids = out
            .scripts
            .expressions
            .iter()
            .filter(|e| {
                e.kind == script::NodeKind::Value && e.value_type == script::value_type::STRING_ID
            })
            .map(|e| (e.value, out.scripts.text(e.text).to_string()))
            .collect();
        let triggers: HashMap<&str, u16> = out
            .triggers
            .iter()
            .enumerate()
            .map(|(k, t)| (t.name.as_str(), k as u16))
            .collect();
        out.trigger_names = out
            .scripts
            .expressions
            .iter()
            .filter(|e| e.kind == script::NodeKind::Value)
            .filter_map(|e| Some((e.value, *triggers.get(out.scripts.text(e.text))?)))
            .collect();
        out.volumes = scenario::trigger_volumes(set)
            .unwrap_or_default()
            .iter()
            .map(|t| {
                let v = &t.volume;
                KillZone::new(
                    v.position.into(),
                    v.forward.into(),
                    v.up.into(),
                    v.extents.into(),
                )
            })
            .collect();
        out.bsp_switches = scenario::bsp_switches(set).unwrap_or_default();
        out.flags = scenario::cutscene_flags(set)
            .unwrap_or_default()
            .iter()
            .map(|f| (Vec3::from(f.position), f.facing[0]))
            .collect();
        out.device_groups = scenario::device_groups(set).unwrap_or_default();
        let table = text::language_table(set).unwrap_or_default();
        let (titles, objectives) =
            scenario::text_lists(set).unwrap_or((DatumIndex::NONE, DatumIndex::NONE));
        let titles = text::unicode_strings(set, &table, titles).unwrap_or_default();
        out.titles = scenario::cutscene_titles(set)
            .unwrap_or_default()
            .iter()
            .map(|t| Title {
                text: titles
                    .iter()
                    .find(|(id, _)| *id == t.name)
                    .map_or_else(String::new, |(_, s)| s.clone()),
                bounds: t.bounds.map(f32::from),
                fade_in: t.fade_in,
                up: t.up,
                fade_out: t.fade_out,
            })
            .collect();
        out.objectives = text::unicode_strings(set, &table, objectives)
            .unwrap_or_default()
            .into_iter()
            .map(|(_, s)| s)
            .collect();
        if let Ok(subtitles) = scenario::subtitles(set) {
            out.cinema.subtitles = text::unicode_strings(set, &table, subtitles)
                .unwrap_or_default()
                .into_iter()
                .collect();
        }
        let placed: Vec<(PlacedKind, Vec<scenario::Placement>)> = PlacedKind::ALL
            .iter()
            .map(|&k| (k, scenario::placements(set, k).unwrap_or_default()))
            .collect();
        self.script_sounds(out);
        self.dialogue_lines(out);
        let set = &mut self.set;
        let names = scenario::object_names(set).unwrap_or_default();
        out.object_names = names.iter().map(|n| n.name.clone()).collect();
        out.name_positions = names
            .iter()
            .map(|n| {
                let (_, list) = placed.iter().find(|(k, _)| Some(*k) == n.kind)?;
                let p = list.get(n.index? as usize)?;
                Some(Vec3::from(p.position))
            })
            .collect();
        out.cinema.placed = names
            .iter()
            .map(|n| {
                let (_, list) = placed.iter().find(|(k, _)| Some(*k) == n.kind)?;
                let p = list.get(n.index? as usize)?;
                Some(placement_matrix(p.position, p.rotation, p.scale))
            })
            .collect();
        // Named bipeds, and vehicles only scripts create (the game has
        // the rest): the cast cutscenes animate.
        out.cinema.cast = names
            .iter()
            .enumerate()
            .filter_map(|(k, n)| {
                let kind = n
                    .kind
                    .filter(|&k| k == PlacedKind::Biped || k == PlacedKind::Vehicle)?;
                let (_, list) = placed.iter().find(|(k, _)| *k == kind)?;
                let p = list.get(n.index? as usize)?;
                (kind == PlacedKind::Biped || !p.automatic)
                    .then(|| (k as u16, p.object, p.variant.clone()))
            })
            .collect();
        out.machines = names
            .iter()
            .map(|n| {
                if n.kind != Some(PlacedKind::Machine) {
                    return None;
                }
                let (_, list) = placed.iter().find(|(k, _)| *k == PlacedKind::Machine)?;
                let p = list.get(n.index? as usize)?;
                scenario::machine(set, p.object).ok()
            })
            .collect();
    }

    /// The mission dialogue lines the scripts name, with every voice's
    /// recording.
    fn dialogue_lines(&mut self, out: &mut CampaignAi) {
        let lines = scenario::mission_dialogue(&mut self.set).unwrap_or_else(|e| {
            println!("warning: mission dialogue: {e}");
            Vec::new()
        });
        let named: HashMap<&str, u32> = out
            .string_ids
            .iter()
            .map(|(&id, name)| (name.as_str(), id))
            .collect();
        for line in lines {
            let Some(&id) = named.get(line.name.as_str()) else {
                continue;
            };
            let voices = line
                .variants
                .iter()
                .filter_map(|v| Some((v.designation.clone(), self.sound(v.sound)?)))
                .collect();
            out.lines.insert(id, voices);
        }
    }

    /// The cutscenes' animation graphs and the bipeds they animate.
    fn cinema(&mut self, out: &mut CampaignAi, meshes: &mut Vec<MeshData>) {
        let mut tags: Vec<u32> = out
            .scripts
            .expressions
            .iter()
            .filter(|e| {
                e.kind == script::NodeKind::Value
                    && e.value_type == script::value_type::ANIMATION_GRAPH
            })
            .map(|e| e.value)
            .collect();
        tags.sort_unstable();
        tags.dedup();
        for tag in tags {
            let datum = DatumIndex(tag);
            if datum == DatumIndex::NONE {
                continue;
            }
            match animation::read_animation_graph(&mut self.set, datum) {
                Ok(g) => {
                    out.cinema.graphs.insert(tag, g);
                }
                Err(e) => println!("warning: cutscene animations {tag:08x}: {e}"),
            }
        }
        // The permutations scripts swap in: object name, region and
        // permutation (string ids).
        let scripts = &out.scripts;
        let swaps: Vec<(u16, u32, u32)> = (0..scripts.expressions.len() as u16)
            .filter(|&i| {
                scripts.expressions[i as usize].kind == script::NodeKind::Call
                    && scripts.function_name(i) == "object_set_permutation"
            })
            .filter_map(|i| {
                let args = scripts.arguments(i);
                let arg = |k: usize| scripts.expression(*args.get(k)?);
                let object = arg(0).filter(|e| e.kind == script::NodeKind::Value)?;
                Some((object.value as u16, arg(1)?.value, arg(2)?.value))
            })
            .collect();
        for (name, object, variant) in std::mem::take(&mut out.cinema.cast) {
            let tag_name = self
                .set
                .locate(object)
                .map(|(_, t)| t.name)
                .unwrap_or_default();
            let chief = tag_name.ends_with("masterchief");
            let read = |set: &mut blam_cache::mapset::MapSet| {
                let v = model::named_variant(set, object, &variant)?;
                // Colours go by the name the scenario gives, even when the
                // model has no variant called that.
                let called = match variant.as_str() {
                    "" => v.as_ref().map_or("", |v| v.name.as_str()),
                    name => name,
                };
                let colors = model::object_change_colors(set, object, called)?;
                let mode = model::object_render_model(set, object)?;
                Ok::<_, blam_cache::Error>((
                    model::read_render_model_variant(set, mode, v.as_ref())?,
                    colors,
                    v,
                    mode,
                ))
            };
            let (m, colors, v, mode) = match read(&mut self.set) {
                Ok(m) => m,
                Err(e) => {
                    println!("warning: cutscene object {:08x}: {e}", object.0);
                    continue;
                }
            };
            let colors = match colors.as_slice() {
                [] => None,
                [p] => Some([*p, [1.0; 3]]),
                [p, s, ..] => Some([*p, *s]),
            };
            let mesh = self.model_mesh(&m);
            let skin = SkinnedMesh::new(&mesh);
            meshes.push(mesh);
            let mut looks = Vec::new();
            for &(_, region, perm) in swaps.iter().filter(|s| s.0 == name) {
                let sid = |id: u32| out.string_ids.get(&id).cloned().unwrap_or_default();
                let (region_name, perm_name) = (sid(region), sid(perm));
                let mut swapped = v.clone().unwrap_or_default();
                swapped.regions.retain(|r| r.0 != region_name);
                swapped
                    .regions
                    .push((region_name, Some(perm_name).filter(|p| !p.is_empty())));
                let Ok(m) = model::read_render_model_variant(&mut self.set, mode, Some(&swapped))
                else {
                    continue;
                };
                let mesh = self.model_mesh(&m);
                let skin = SkinnedMesh::new(&mesh);
                meshes.push(mesh);
                looks.push((region, perm, meshes.len() - 1, skin));
            }
            out.cinema.bodies.insert(
                name,
                CinemaBody {
                    mesh: meshes.len() - 1 - looks.len(),
                    chief,
                    colors,
                    skeleton: Skeleton::new(&m.nodes),
                    parents: m.nodes.iter().map(|n| n.parent).collect(),
                    skin,
                    looks,
                },
            );
        }
    }

    /// The dialogue and music the scripts play.
    fn script_sounds(&mut self, out: &mut CampaignAi) {
        let tags: Vec<(u16, u32)> = out
            .scripts
            .expressions
            .iter()
            .filter(|e| {
                e.kind == script::NodeKind::Value
                    && matches!(
                        e.value_type,
                        script::value_type::SOUND
                            | script::value_type::LOOPING_SOUND
                            | script::value_type::EFFECT
                            | script::value_type::DAMAGE
                    )
            })
            .map(|e| (e.value_type, e.value))
            .collect();
        for (kind, tag) in tags {
            let datum = DatumIndex(tag);
            if datum == DatumIndex::NONE {
                continue;
            }
            if matches!(
                kind,
                script::value_type::EFFECT | script::value_type::DAMAGE
            ) {
                if out.effects.contains_key(&tag) {
                    continue;
                }
                let name = self.set.locate(datum).map(|(_, t)| t.name);
                let sounds = blam_cache::sound::effect_sounds(&mut self.set, datum)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|s| self.sound(s))
                    .collect();
                let look = name.as_deref().and_then(effect_look);
                out.effects.insert(tag, ScriptEffect { sounds, look });
                continue;
            }
            if kind == script::value_type::SOUND {
                if out.sounds.contains_key(&tag) {
                    continue;
                }
                if let Some(s) = self.sound(datum) {
                    out.sounds.insert(tag, s);
                }
                continue;
            }
            if out.loops.contains_key(&tag) {
                continue;
            }
            let Ok(l) = blam_cache::sound::looping_sound(&mut self.set, datum) else {
                continue;
            };
            let Some(t) = l.tracks.first().copied() else {
                continue;
            };
            let looped = ScriptLoop {
                start: t.start.and_then(|s| self.sound(s)),
                repeat: t.repeat.and_then(|s| self.sound(s)),
                end: t.end.and_then(|s| self.sound(s)),
                gain: 10f32.powf(t.gain / 20.0),
                once: l.once,
            };
            out.loops.insert(tag, looped);
        }
    }

    /// A weapon by tag: one already loaded, else loaded now.
    fn weapon_by_tag(
        &mut self,
        tag: DatumIndex,
        weapons: &mut Vec<WeaponAssets>,
        arms: Arms2<'_>,
        meshes: &mut Vec<MeshData>,
    ) -> Option<usize> {
        if let Some(i) = weapons.iter().position(|w| w.tag == tag) {
            return Some(i);
        }
        let name = self.set.locate(tag).map(|(_, t)| t.name)?;
        let w = self.weapon(&name, arms, meshes)?;
        weapons.push(w);
        Some(weapons.len() - 1)
    }

    /// The mission's characters, squads and the weapons they carry.
    pub(super) fn campaign_ai(
        &mut self,
        fallback: BipedPhysics,
        weapons: &mut Vec<WeaponAssets>,
        arms: Arms2<'_>,
        meshes: &mut Vec<MeshData>,
    ) -> CampaignAi {
        let mut out = CampaignAi {
            squads: ai::squads(&mut self.set).unwrap_or_else(|e| {
                println!("warning: squads: {e}");
                Vec::new()
            }),
            ..CampaignAi::default()
        };
        self.mission_scripts(&mut out);
        self.cinema(&mut out, meshes);
        let palette = ai::weapon_palette(&mut self.set).unwrap_or_default();
        out.weapons = palette
            .iter()
            .map(|&w| self.weapon_by_tag(w, weapons, arms, meshes))
            .collect();
        let mut units: Vec<DatumIndex> = Vec::new();
        for tag in ai::character_palette(&mut self.set).unwrap_or_default() {
            let c = match ai::read_character(&mut self.set, tag) {
                Ok(c) => c,
                Err(e) => {
                    println!("warning: character {:08x}: {e}", tag.0);
                    Character::default()
                }
            };
            let biped = c
                .unit
                .and_then(|u| physics::biped_physics_of(&mut self.set, u).ok().flatten())
                .unwrap_or(fallback);
            // Its own weapon, if it's one carried by hand.
            let weapon = c
                .weapon
                .and_then(|w| w.weapon)
                .filter(|&w| {
                    self.set
                        .locate(w)
                        .is_some_and(|(_, t)| t.name.starts_with("objects\\weapons\\"))
                })
                .and_then(|w| self.weapon_by_tag(w, weapons, arms, meshes));
            let (def, legendary) = character_def(&c, biped, weapon);
            // Its biped's colours for its variant, else its rank's.
            let colors = c.unit.and_then(|u| {
                let variant = c.model_variants.first().map_or("", String::as_str);
                match model::object_change_colors(&mut self.set, u, variant).ok()?[..] {
                    [p] => Some([p, [1.0; 3]]),
                    [p, s, ..] => Some([p, s]),
                    _ => None,
                }
            });
            out.colors
                .push(colors.unwrap_or_else(|| rank_colors(&c.name, c.kind)));
            out.legendary_accuracy.push(legendary);
            out.characters.push(def);
            out.voices.push(c.voices.clone());
            let body = c.unit.and_then(|u| {
                if let Some(k) = units.iter().position(|&x| x == u) {
                    return Some(k);
                }
                let body = self.body_of(u, ["right_hand", "left_hand"], ACTOR_BODIES, meshes)?;
                units.push(u);
                out.bodies.push(body);
                Some(out.bodies.len() - 1)
            });
            out.body_of.push(body);
        }
        out
    }
}
