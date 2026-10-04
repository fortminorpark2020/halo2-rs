//! Objects a scenario places in its level: scenery, the campaign's
//! weapons, vehicles, doors and other objects, and the weapons and
//! equipment multiplayer games spawn ("netgame equipment").

use crate::mapset::MapSet;
use crate::{f32_at, i16_at, u32_at, DatumIndex, Error, Result};

const SCNR_OBJECT_NAMES: usize = 0x48;
const SCNR_STARTING_PROFILES: usize = 0xF8;
const SCNR_TRIGGER_VOLUMES: usize = 0x108;
const SCNR_NETGAME_FLAGS: usize = 0x118;
const SCNR_BSP_SWITCHES: usize = 0x130;
const BSP_SWITCH_SIZE: usize = 0xE;
const SCNR_NETGAME_EQUIPMENT: usize = 0x120;
const NETGAME_FLAG_SIZE: usize = 0x20;
const TRIGGER_VOLUME_SIZE: usize = 0x44;
const PALETTE_SIZE: usize = 0x28;
const OBJECT_NAME_SIZE: usize = 0x24;
const STARTING_PROFILE_SIZE: usize = 0x44;
const NETGAME_EQUIPMENT_SIZE: usize = 0x90;
const ITEM_COLLECTION_ENTRY_SIZE: usize = 0x10;

/// The kinds of object a scenario places, each in a block of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacedKind {
    Scenery,
    Biped,
    Vehicle,
    Equipment,
    Weapon,
    /// Doors, lifts and other moving machinery.
    Machine,
    /// Switches and panels.
    Control,
    Crate,
}

impl PlacedKind {
    /// The kinds in the order object names number them.
    pub const ALL: [PlacedKind; 8] = [
        PlacedKind::Biped,
        PlacedKind::Vehicle,
        PlacedKind::Weapon,
        PlacedKind::Equipment,
        PlacedKind::Scenery,
        PlacedKind::Machine,
        PlacedKind::Control,
        PlacedKind::Crate,
    ];

    /// Whether its instances name a model variant (after the object data).
    fn has_variant(self) -> bool {
        matches!(
            self,
            PlacedKind::Scenery | PlacedKind::Biped | PlacedKind::Vehicle | PlacedKind::Weapon
        )
    }

    /// Offsets of the instance and palette blocks, and an instance's size.
    fn layout(self) -> (usize, usize, usize) {
        match self {
            PlacedKind::Scenery => (0x50, 0x58, 0x5C),
            PlacedKind::Biped => (0x60, 0x68, 0x54),
            PlacedKind::Vehicle => (0x70, 0x78, 0x54),
            PlacedKind::Equipment => (0x80, 0x88, 0x38),
            PlacedKind::Weapon => (0x90, 0x98, 0x54),
            PlacedKind::Machine => (0xA8, 0xB0, 0x48),
            PlacedKind::Control => (0xB8, 0xC0, 0x44),
            PlacedKind::Crate => (0x328, 0x330, 0x4C),
        }
    }
}

/// An object instance placed in the level.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// The object's tag (`scen`, ...), from the block's palette.
    pub object: DatumIndex,
    pub position: [f32; 3],
    /// Yaw, pitch and roll in radians.
    pub rotation: [f32; 3],
    pub scale: f32,
    /// Its entry in the object names, which scripts know it by.
    pub name: Option<u16>,
    /// Placed when the level starts (otherwise a script creates it).
    pub automatic: bool,
    /// The model variant to show (vehicles, bipeds, weapons and scenery).
    pub variant: String,
    /// Machines: their device flags (bit 0 initially open, 1 initially
    /// off) and machine flags (bit 0 doesn't operate automatically, 5
    /// doesn't close automatically).
    pub device_flags: u32,
    pub machine_flags: u32,
}

/// An object name scripts use, and the instance it names.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectName {
    pub name: String,
    pub kind: Option<PlacedKind>,
    /// The instance within its kind's block.
    pub index: Option<u16>,
}

/// What a campaign player starts a level with.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StartingProfile {
    pub name: String,
    /// Weapons, with the rounds loaded and in total (`None`: the weapon's
    /// own).
    pub primary: Option<StartingWeapon>,
    pub secondary: Option<StartingWeapon>,
    pub frags: u8,
    pub plasmas: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartingWeapon {
    pub weapon: DatumIndex,
    pub loaded: Option<u16>,
    pub total: Option<u16>,
}

/// A multiplayer item spawn: a weapon (or vehicle, grenade...) collection
/// placed in the level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetgameItem {
    /// An item collection (`itmc`) or vehicle collection (`vehc`).
    pub collection: DatumIndex,
    pub position: [f32; 3],
    /// Yaw, pitch and roll in radians.
    pub rotation: [f32; 3],
    /// Seconds before the item comes back once taken (0: the game default).
    pub respawn_seconds: u16,
}

/// What a netgame flag marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetgameFlagKind {
    CtfFlagSpawn,
    /// Where a team brings the enemy flag to score.
    CtfFlagReturn,
    AssaultBombSpawn,
    AssaultBombReturn,
    OddballSpawn,
    RaceCheckpoint,
    TeleporterSource,
    TeleporterDestination,
    HeadhunterBin,
    TerritoriesFlag,
    /// King of the Hill: the hill's index (0-7).
    KingHill(u8),
    Other(u16),
}

/// A point the multiplayer game types use: flags, bomb sites, hills.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetgameFlag {
    pub kind: NetgameFlagKind,
    pub position: [f32; 3],
    /// Radians around +z.
    pub facing: f32,
    /// 0 red, 1 blue ... 8 neutral.
    pub team: u16,
    pub identifier: i16,
}

/// A box in the level that kills whoever enters it: the pits and drops a
/// map's designers made deadly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KillVolume {
    /// The box's corner; it reaches `extents` along `forward`, along the
    /// left of `forward` (`up` x `forward`), and along `up`.
    pub position: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
    pub extents: [f32; 3],
}

fn scenario_data(set: &mut MapSet) -> Result<Vec<u8>> {
    let map = &mut set.map;
    let scnr = map
        .tag(map.scenario)
        .cloned()
        .ok_or_else(|| Error::Corrupt("scenario tag missing".into()))?;
    map.read_tag_data(&scnr)
}

/// A fixed-length name in a tag (NUL padded).
fn ascii(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// The scenario's scenery instances.
pub fn scenery(set: &mut MapSet) -> Result<Vec<Placement>> {
    placements(set, PlacedKind::Scenery)
}

/// The instances of one kind of object the scenario places, in block
/// order (scripts and object names refer to them by index).
pub fn placements(set: &mut MapSet, kind: PlacedKind) -> Result<Vec<Placement>> {
    let (block, palette_block, size) = kind.layout();
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let palette = map.read_block(meta, &data, palette_block, PALETTE_SIZE)?;
    let palette: Vec<DatumIndex> = palette
        .as_chunks::<PALETTE_SIZE>()
        .0
        .iter()
        .map(|p| DatumIndex(u32_at(p, 4)))
        .collect();
    let instances = map.read_block(meta, &data, block, size)?;
    Ok(instances
        .chunks_exact(size)
        .map(|e| {
            let object = usize::try_from(i16_at(e, 0))
                .ok()
                .and_then(|i| palette.get(i).copied())
                .unwrap_or(DatumIndex::NONE);
            Placement {
                object,
                position: [f32_at(e, 0x8), f32_at(e, 0xC), f32_at(e, 0x10)],
                rotation: [f32_at(e, 0x14), f32_at(e, 0x18), f32_at(e, 0x1C)],
                scale: match f32_at(e, 0x20) {
                    s if s > 0.0 => s,
                    _ => 1.0,
                },
                name: u16::try_from(i16_at(e, 2)).ok(),
                automatic: u32_at(e, 4) & 1 == 0,
                variant: if kind.has_variant() {
                    map.string_id(u32_at(e, 0x34))
                        .unwrap_or_default()
                        .to_string()
                } else {
                    String::new()
                },
                device_flags: if kind == PlacedKind::Machine {
                    u32_at(e, 0x38)
                } else {
                    0
                },
                machine_flags: if kind == PlacedKind::Machine {
                    u32_at(e, 0x3C)
                } else {
                    0
                },
            }
        })
        .collect())
}

/// The names scripts give placed objects.
pub fn object_names(set: &mut MapSet) -> Result<Vec<ObjectName>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let names = map.read_block(meta, &data, SCNR_OBJECT_NAMES, OBJECT_NAME_SIZE)?;
    Ok(names
        .as_chunks::<OBJECT_NAME_SIZE>()
        .0
        .iter()
        .map(|e| ObjectName {
            name: ascii(&e[..0x20]),
            kind: usize::try_from(i16_at(e, 0x20)).ok().and_then(object_kind),
            index: u16::try_from(i16_at(e, 0x22)).ok(),
        })
        .collect())
}

/// An object name's type number as a placed kind (objects of other kinds,
/// like projectiles, aren't placed).
fn object_kind(k: usize) -> Option<PlacedKind> {
    Some(match k {
        0 => PlacedKind::Biped,
        1 => PlacedKind::Vehicle,
        2 => PlacedKind::Weapon,
        3 => PlacedKind::Equipment,
        6 => PlacedKind::Scenery,
        7 => PlacedKind::Machine,
        8 => PlacedKind::Control,
        11 => PlacedKind::Crate,
        _ => return None,
    })
}

/// What campaign players start the level with.
pub fn starting_profiles(set: &mut MapSet) -> Result<Vec<StartingProfile>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let profiles = map.read_block(meta, &data, SCNR_STARTING_PROFILES, STARTING_PROFILE_SIZE)?;
    let weapon = |e: &[u8], at: usize| {
        let weapon = DatumIndex(u32_at(e, at + 4));
        let rounds = |o: usize| u16::try_from(i16_at(e, o)).ok();
        (weapon != DatumIndex::NONE).then(|| StartingWeapon {
            weapon,
            loaded: rounds(at + 8),
            total: rounds(at + 0xA),
        })
    };
    Ok(profiles
        .as_chunks::<STARTING_PROFILE_SIZE>()
        .0
        .iter()
        .map(|e| StartingProfile {
            name: ascii(&e[..0x20]),
            primary: weapon(e, 0x28),
            secondary: weapon(e, 0x34),
            frags: e[0x40],
            plasmas: e[0x41],
        })
        .collect())
}

/// The scenario's multiplayer game type points.
pub fn netgame_flags(set: &mut MapSet) -> Result<Vec<NetgameFlag>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let entries = map.read_block(meta, &data, SCNR_NETGAME_FLAGS, NETGAME_FLAG_SIZE)?;
    Ok(entries
        .as_chunks::<NETGAME_FLAG_SIZE>()
        .0
        .iter()
        .map(|e| {
            let kind = match i16_at(e, 0x10) as u16 {
                0 => NetgameFlagKind::CtfFlagSpawn,
                1 => NetgameFlagKind::CtfFlagReturn,
                2 => NetgameFlagKind::AssaultBombSpawn,
                3 => NetgameFlagKind::AssaultBombReturn,
                4 => NetgameFlagKind::OddballSpawn,
                6 => NetgameFlagKind::RaceCheckpoint,
                7 => NetgameFlagKind::TeleporterSource,
                8 => NetgameFlagKind::TeleporterDestination,
                9 => NetgameFlagKind::HeadhunterBin,
                10 => NetgameFlagKind::TerritoriesFlag,
                k @ 11..=18 => NetgameFlagKind::KingHill((k - 11) as u8),
                k => NetgameFlagKind::Other(k),
            };
            NetgameFlag {
                kind,
                position: [f32_at(e, 0), f32_at(e, 4), f32_at(e, 8)],
                facing: f32_at(e, 0xC),
                team: i16_at(e, 0x12) as u16,
                identifier: i16_at(e, 0x14),
            }
        })
        .collect())
}

/// The scenario's trigger volumes that kill (Lockout's pit, Zanzibar's sea).
pub fn kill_volumes(set: &mut MapSet) -> Result<Vec<KillVolume>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let entries = map.read_block(meta, &data, SCNR_TRIGGER_VOLUMES, TRIGGER_VOLUME_SIZE)?;
    let v3 = |e: &[u8], at: usize| [f32_at(e, at), f32_at(e, at + 4), f32_at(e, at + 8)];
    Ok(entries
        .as_chunks::<TRIGGER_VOLUME_SIZE>()
        .0
        .iter()
        .filter(|e| i16_at(*e, 0x40) >= 0)
        .map(|e| KillVolume {
            forward: v3(e, 0xC),
            up: v3(e, 0x18),
            position: v3(e, 0x24),
            extents: v3(e, 0x30),
        })
        .collect())
}

/// A named box scripts test for who's inside (and bring the mission on
/// when the player walks in).
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerVolume {
    pub name: String,
    pub volume: KillVolume,
}

/// Every trigger volume, in block order (scripts know them by index).
pub fn trigger_volumes(set: &mut MapSet) -> Result<Vec<TriggerVolume>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let entries = map.read_block(meta, &data, SCNR_TRIGGER_VOLUMES, TRIGGER_VOLUME_SIZE)?;
    let v3 = |e: &[u8], at: usize| [f32_at(e, at), f32_at(e, at + 4), f32_at(e, at + 8)];
    Ok(entries
        .as_chunks::<TRIGGER_VOLUME_SIZE>()
        .0
        .iter()
        .map(|e| TriggerVolume {
            name: map.string_id(u32_at(e, 0)).unwrap_or_default().to_string(),
            volume: KillVolume {
                forward: v3(e, 0xC),
                up: v3(e, 0x18),
                position: v3(e, 0x24),
                extents: v3(e, 0x30),
            },
        })
        .collect())
}

/// Where walking into a trigger volume moves the game from one structure
/// BSP to another: (volume, from, to).
pub fn bsp_switches(set: &mut MapSet) -> Result<Vec<(u16, u16, u16)>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let entries = map.read_block(meta, &data, SCNR_BSP_SWITCHES, BSP_SWITCH_SIZE)?;
    Ok(entries
        .as_chunks::<BSP_SWITCH_SIZE>()
        .0
        .iter()
        .filter_map(|e| {
            let n = |o: usize| u16::try_from(i16_at(e, o)).ok();
            Some((n(0)?, n(2)?, n(4)?))
        })
        .collect())
}

/// The scenario's multiplayer item spawns.
pub fn netgame_equipment(set: &mut MapSet) -> Result<Vec<NetgameItem>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let entries = map.read_block(meta, &data, SCNR_NETGAME_EQUIPMENT, NETGAME_EQUIPMENT_SIZE)?;
    Ok(entries
        .as_chunks::<NETGAME_EQUIPMENT_SIZE>()
        .0
        .iter()
        .map(|e| NetgameItem {
            collection: DatumIndex(u32_at(e, 0x5C)),
            position: [f32_at(e, 0x40), f32_at(e, 0x44), f32_at(e, 0x48)],
            rotation: [f32_at(e, 0x4C), f32_at(e, 0x50), f32_at(e, 0x54)],
            respawn_seconds: i16_at(e, 0xE).max(0) as u16,
        })
        .filter(|i| i.collection != DatumIndex::NONE)
        .collect())
}

/// An item collection's items with their weights.
pub fn item_collection(set: &mut MapSet, itmc: DatumIndex) -> Result<Vec<(f32, DatumIndex)>> {
    let (src, _, data) = set.tag_data(itmc)?;
    let file = set.get(src);
    let region = file.meta_region();
    let entries = file.read_block(region, &data, 0, ITEM_COLLECTION_ENTRY_SIZE)?;
    Ok(entries
        .as_chunks::<ITEM_COLLECTION_ENTRY_SIZE>()
        .0
        .iter()
        .map(|e| (f32_at(e, 0), DatumIndex(u32_at(e, 8))))
        .filter(|(_, d)| *d != DatumIndex::NONE)
        .collect())
}

/// How a machine (a door, a lift) moves, from its `mach` tag.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Machine {
    /// Seconds to go from closed to open.
    pub position_time: f32,
    /// 0 door, 1 platform, 2 gear.
    pub kind: u16,
    /// Opens by itself for someone this close (doors that open
    /// automatically).
    pub activation_radius: f32,
    /// Seconds a door stays open.
    pub door_open_time: f32,
}

pub fn machine(set: &mut MapSet, tag: DatumIndex) -> Result<Machine> {
    let (_, _, data) = set.tag_data(tag)?;
    if data.len() < 0x124 {
        return Err(Error::Corrupt("machine tag too small".into()));
    }
    Ok(Machine {
        position_time: f32_at(&data, 0xC8),
        kind: i16_at(&data, 0x11C) as u16,
        activation_radius: f32_at(&data, 0x118),
        door_open_time: f32_at(&data, 0x120),
    })
}
