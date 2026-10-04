//! Where the campaign's squads go and when: zones (the firing positions
//! actors take up, grouped into areas), orders (which areas a squad holds,
//! and which order follows once its endings' triggers hold) and the AI
//! triggers those endings test.

use crate::ai::{ascii, index, scenario_data};
use crate::mapset::MapSet;
use crate::{f32_at, i16_at, u32_at, Result};

const SCNR_ZONES: usize = 0x168;
const ZONE_SIZE: usize = 0x38;
const FIRING_POSITION_SIZE: usize = 0x20;
const AREA_SIZE: usize = 0x88;
const SCNR_ORDERS: usize = 0x240;
const ORDER_SIZE: usize = 0x7C;
const AREA_REF_SIZE: usize = 0x8;
const SET_TRIGGER_SIZE: usize = 0xC;
const ENDING_SIZE: usize = 0x14;
const TRIGGER_REF_SIZE: usize = 0x8;
const SCNR_AI_TRIGGERS: usize = 0x248;
const AI_TRIGGER_SIZE: usize = 0x30;
const CONDITION_SIZE: usize = 0x38;
const SCNR_SCRIPTING_DATA: usize = 0x1D8;
const SCRIPTING_DATA_SIZE: usize = 0x80;
const POINT_SET_SIZE: usize = 0x30;
const POINT_SIZE: usize = 0x3C;
const SCNR_MISSION_SCENES: usize = 0x170;
const SCENE_SIZE: usize = 0x18;
const SCENE_ROLE_SIZE: usize = 0x10;
const ROLE_VARIANT_SIZE: usize = 0x4;

/// A named point scripts send actors to (`cs_go_to`) or look at.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub name: String,
    pub position: [f32; 3],
    /// Relative to a moving object, not the level.
    pub moving: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PointSet {
    pub name: String,
    pub points: Vec<Point>,
}

/// The scenario's point sets (scripts name a point by set and index).
pub fn point_sets(set: &mut MapSet) -> Result<Vec<PointSet>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let scripting = map.read_block(meta, &data, SCNR_SCRIPTING_DATA, SCRIPTING_DATA_SIZE)?;
    let Some(scripting) = scripting.get(..SCRIPTING_DATA_SIZE) else {
        return Ok(Vec::new());
    };
    let sets = map.read_block(meta, scripting, 0, POINT_SET_SIZE)?;
    let mut out = Vec::new();
    for s in sets.as_chunks::<POINT_SET_SIZE>().0 {
        let points = map.read_block(meta, s, 0x20, POINT_SIZE)?;
        out.push(PointSet {
            name: ascii(&s[..0x20]),
            points: points
                .as_chunks::<POINT_SIZE>()
                .0
                .iter()
                .map(|p| Point {
                    name: ascii(&p[..0x20]),
                    position: [f32_at(p, 0x20), f32_at(p, 0x24), f32_at(p, 0x28)],
                    moving: i16_at(p, 0x2C) >= 0,
                })
                .collect(),
        });
    }
    Ok(out)
}

/// A place an actor stands to fight from.
#[derive(Debug, Clone, PartialEq)]
pub struct FiringPosition {
    pub position: [f32; 3],
    /// Relative to a moving object (the tram), not the level.
    pub moving: bool,
    pub area: Option<u16>,
}

/// A part of a zone: some of its firing positions.
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub name: String,
    pub position: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Zone {
    pub name: String,
    pub firing_positions: Vec<FiringPosition>,
    pub areas: Vec<Area>,
}

impl Zone {
    /// The firing positions in an area.
    pub fn in_area(&self, area: u16) -> impl Iterator<Item = &FiringPosition> {
        self.firing_positions
            .iter()
            .filter(move |f| f.area == Some(area))
    }
}

/// How a set of tests combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Combine {
    #[default]
    Or,
    And,
}

impl Combine {
    fn from_number(n: i16) -> Combine {
        if n == 1 {
            Combine::And
        } else {
            Combine::Or
        }
    }

    pub fn holds(self, mut tests: impl Iterator<Item = bool>) -> bool {
        match self {
            Combine::Or => tests.any(|t| t),
            Combine::And => tests.all(|t| t),
        }
    }
}

/// An AI trigger an order ending tests, and whether it's negated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerRef {
    pub trigger: u16,
    pub not: bool,
}

/// When a squad moves on from an order, and to which.
#[derive(Debug, Clone, PartialEq)]
pub struct Ending {
    pub next: Option<u16>,
    pub combine: Combine,
    /// Seconds after the triggers hold.
    pub delay: f32,
    pub triggers: Vec<TriggerRef>,
}

/// Where a squad goes: the areas it holds (`primary`), and others it
/// adds once `secondary_trigger` holds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Order {
    pub name: String,
    pub flags: u32,
    /// Zone and area of each area it holds.
    pub primary: Vec<(u16, u16)>,
    pub secondary: Vec<(u16, u16)>,
    pub secondary_trigger: Option<(Combine, Vec<TriggerRef>)>,
    pub endings: Vec<Ending>,
}

impl Order {
    /// Allies under it go along with the player nearest them.
    pub fn follows_player(&self) -> bool {
        self.flags & ORDER_FOLLOW_PLAYER != 0
    }
}

const ORDER_FOLLOW_PLAYER: u32 = 1 << 4;

/// What one of a trigger's conditions tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    AliveAtLeast,
    AliveAtMost,
    StrengthAtLeast,
    StrengthAtMost,
    EnemySighted,
    AfterTicks,
    AlertedBySquad,
    ScriptTrue,
    ScriptFalse,
    PlayerInVolume,
    AllPlayersInVolume,
    CombatStatusAtLeast,
    CombatStatusAtMost,
    Arrived,
    InVehicle,
    SightedPlayer,
    FightingAtLeast,
    FightingAtMost,
    PlayerWithin,
    PlayerShotLongAgo,
    SafeToSave,
    Other(u16),
}

impl Rule {
    fn from_number(n: u16) -> Rule {
        use Rule::*;
        match n {
            0 => AliveAtLeast,
            1 => AliveAtMost,
            2 => StrengthAtLeast,
            3 => StrengthAtMost,
            4 => EnemySighted,
            5 => AfterTicks,
            6 => AlertedBySquad,
            7 => ScriptTrue,
            8 => ScriptFalse,
            9 => PlayerInVolume,
            10 => AllPlayersInVolume,
            11 => CombatStatusAtLeast,
            12 => CombatStatusAtMost,
            13 => Arrived,
            14 => InVehicle,
            15 => SightedPlayer,
            16 => FightingAtLeast,
            17 => FightingAtMost,
            18 => PlayerWithin,
            19 => PlayerShotLongAgo,
            20 => SafeToSave,
            n => Other(n),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub rule: Rule,
    /// The squad or squad group it's about (otherwise the squad testing).
    pub squad: Option<u16>,
    pub group: Option<u16>,
    pub a: i16,
    pub x: f32,
    pub volume: Option<u16>,
    /// The script it runs (a static script, by name).
    pub script: String,
    pub not: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiTrigger {
    pub name: String,
    /// Stays on once it has held.
    pub latch: bool,
    pub combine: Combine,
    pub conditions: Vec<Condition>,
}

/// The scenario's zones, in block order.
pub fn zones(set: &mut MapSet) -> Result<Vec<Zone>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let zones = map.read_block(meta, &data, SCNR_ZONES, ZONE_SIZE)?;
    let mut out = Vec::new();
    for z in zones.as_chunks::<ZONE_SIZE>().0 {
        let positions = map.read_block(meta, z, 0x28, FIRING_POSITION_SIZE)?;
        let areas = map.read_block(meta, z, 0x30, AREA_SIZE)?;
        out.push(Zone {
            name: ascii(&z[..0x20]),
            firing_positions: positions
                .as_chunks::<FIRING_POSITION_SIZE>()
                .0
                .iter()
                .map(|f| FiringPosition {
                    position: [f32_at(f, 0), f32_at(f, 4), f32_at(f, 8)],
                    moving: i16_at(f, 0xC) >= 0,
                    area: index(f, 0x10),
                })
                .collect(),
            areas: areas
                .as_chunks::<AREA_SIZE>()
                .0
                .iter()
                .map(|a| Area {
                    name: ascii(&a[..0x20]),
                    position: [f32_at(a, 0x24), f32_at(a, 0x28), f32_at(a, 0x2C)],
                })
                .collect(),
        });
    }
    Ok(out)
}

fn trigger_refs(b: &[u8]) -> Vec<TriggerRef> {
    b.as_chunks::<TRIGGER_REF_SIZE>()
        .0
        .iter()
        .filter_map(|t| {
            Some(TriggerRef {
                trigger: index(t, 4)?,
                not: u32_at(t, 0) & 1 != 0,
            })
        })
        .collect()
}

fn area_refs(b: &[u8]) -> Vec<(u16, u16)> {
    b.as_chunks::<AREA_REF_SIZE>()
        .0
        .iter()
        .filter_map(|a| Some((index(a, 4)?, index(a, 6)?)))
        .collect()
}

/// A little scene scripts stage (`ai_scene`): actors who fit its roles
/// act it out with a command script (Marines chatting, Johnson greeting
/// the player), once its triggers hold.
#[derive(Debug, Clone, PartialEq)]
pub struct MissionScene {
    /// Its name, a string id scripts name it by.
    pub name: u32,
    /// It can play more than once.
    pub repeats: bool,
    /// The trigger sets that must all hold for it to start.
    pub conditions: Vec<(Combine, Vec<TriggerRef>)>,
    pub roles: Vec<SceneRole>,
}

/// A part in a scene: its name (a string id the command script switches
/// to it by), which of the scene's groups of actors plays it (0-2), and
/// the voices (dialogue designators) that can, if only some can.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneRole {
    pub name: u32,
    pub group: u16,
    pub voices: Vec<String>,
}

pub fn mission_scenes(set: &mut MapSet) -> Result<Vec<MissionScene>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let scenes = map.read_block(meta, &data, SCNR_MISSION_SCENES, SCENE_SIZE)?;
    let mut out = Vec::new();
    for sc in scenes.as_chunks::<SCENE_SIZE>().0 {
        let mut conditions = Vec::new();
        let sets = map.read_block(meta, sc, 0x8, SET_TRIGGER_SIZE)?;
        for t in sets.as_chunks::<SET_TRIGGER_SIZE>().0 {
            conditions.push((
                Combine::from_number(i16_at(t, 0)),
                trigger_refs(&map.read_block(meta, t, 4, TRIGGER_REF_SIZE)?),
            ));
        }
        let mut roles = Vec::new();
        let block = map.read_block(meta, sc, 0x10, SCENE_ROLE_SIZE)?;
        for r in block.as_chunks::<SCENE_ROLE_SIZE>().0 {
            let voices = map.read_block(meta, r, 0x8, ROLE_VARIANT_SIZE)?;
            roles.push(SceneRole {
                name: u32_at(r, 0),
                group: i16_at(r, 4).max(0) as u16,
                voices: voices
                    .as_chunks::<ROLE_VARIANT_SIZE>()
                    .0
                    .iter()
                    .filter_map(|v| map.string_id(u32_at(v, 0)))
                    .map(str::to_string)
                    .collect(),
            });
        }
        out.push(MissionScene {
            name: u32_at(sc, 0),
            repeats: u32_at(sc, 4) & 1 != 0,
            conditions,
            roles,
        });
    }
    Ok(out)
}

/// The scenario's orders, in block order (squads and scripts name them by
/// index).
pub fn orders(set: &mut MapSet) -> Result<Vec<Order>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let orders = map.read_block(meta, &data, SCNR_ORDERS, ORDER_SIZE)?;
    let mut out = Vec::new();
    for o in orders.as_chunks::<ORDER_SIZE>().0 {
        let primary = area_refs(&map.read_block(meta, o, 0x54, AREA_REF_SIZE)?);
        let secondary = area_refs(&map.read_block(meta, o, 0x5C, AREA_REF_SIZE)?);
        let set_trigger = map.read_block(meta, o, 0x64, SET_TRIGGER_SIZE)?;
        let secondary_trigger = match set_trigger.get(..SET_TRIGGER_SIZE) {
            Some(t) => Some((
                Combine::from_number(i16_at(t, 0)),
                trigger_refs(&map.read_block(meta, t, 4, TRIGGER_REF_SIZE)?),
            )),
            None => None,
        };
        let endings = map.read_block(meta, o, 0x74, ENDING_SIZE)?;
        let mut ends = Vec::new();
        for e in endings.as_chunks::<ENDING_SIZE>().0 {
            ends.push(Ending {
                next: index(e, 0),
                combine: Combine::from_number(i16_at(e, 2)),
                delay: f32_at(e, 4),
                triggers: trigger_refs(&map.read_block(meta, e, 0xC, TRIGGER_REF_SIZE)?),
            });
        }
        out.push(Order {
            name: ascii(&o[..0x20]),
            flags: u32_at(o, 0x24),
            primary,
            secondary,
            secondary_trigger,
            endings: ends,
        });
    }
    Ok(out)
}

/// The scenario's AI triggers, in block order.
pub fn ai_triggers(set: &mut MapSet) -> Result<Vec<AiTrigger>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let triggers = map.read_block(meta, &data, SCNR_AI_TRIGGERS, AI_TRIGGER_SIZE)?;
    let mut out = Vec::new();
    for t in triggers.as_chunks::<AI_TRIGGER_SIZE>().0 {
        let conditions = map.read_block(meta, t, 0x28, CONDITION_SIZE)?;
        out.push(AiTrigger {
            name: ascii(&t[..0x20]),
            latch: u32_at(t, 0x20) & 1 != 0,
            combine: Combine::from_number(i16_at(t, 0x24)),
            conditions: conditions
                .as_chunks::<CONDITION_SIZE>()
                .0
                .iter()
                .map(|c| Condition {
                    rule: Rule::from_number(i16_at(c, 0) as u16),
                    squad: index(c, 2),
                    group: index(c, 4),
                    a: i16_at(c, 6),
                    x: f32_at(c, 8),
                    volume: index(c, 0xC),
                    script: ascii(&c[0x10..0x30]),
                    not: u32_at(c, 0x34) & 1 != 0,
                })
                .collect(),
        });
    }
    Ok(out)
}
