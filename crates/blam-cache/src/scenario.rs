//! Objects a scenario places in its level: scenery, and the weapons and
//! equipment multiplayer games spawn ("netgame equipment").

use crate::mapset::MapSet;
use crate::{f32_at, i16_at, u32_at, DatumIndex, Error, Result};

const SCNR_SCENERY: usize = 0x50;
const SCNR_SCENERY_PALETTE: usize = 0x58;
const SCNR_TRIGGER_VOLUMES: usize = 0x108;
const SCNR_NETGAME_FLAGS: usize = 0x118;
const SCNR_NETGAME_EQUIPMENT: usize = 0x120;
const NETGAME_FLAG_SIZE: usize = 0x20;
const TRIGGER_VOLUME_SIZE: usize = 0x44;
const PLACEMENT_SIZE: usize = 0x5C;
const PALETTE_SIZE: usize = 0x28;
const NETGAME_EQUIPMENT_SIZE: usize = 0x90;
const ITEM_COLLECTION_ENTRY_SIZE: usize = 0x10;

/// An object instance placed in the level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The object's tag (`scen`, ...), from the block's palette.
    pub object: DatumIndex,
    pub position: [f32; 3],
    /// Yaw, pitch and roll in radians.
    pub rotation: [f32; 3],
    pub scale: f32,
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

/// The scenario's scenery instances.
pub fn scenery(set: &mut MapSet) -> Result<Vec<Placement>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let palette = map.read_block(meta, &data, SCNR_SCENERY_PALETTE, PALETTE_SIZE)?;
    let palette: Vec<DatumIndex> = palette
        .as_chunks::<PALETTE_SIZE>()
        .0
        .iter()
        .map(|p| DatumIndex(u32_at(p, 4)))
        .collect();
    let instances = map.read_block(meta, &data, SCNR_SCENERY, PLACEMENT_SIZE)?;
    Ok(instances
        .as_chunks::<PLACEMENT_SIZE>()
        .0
        .iter()
        .filter_map(|e| {
            let object = *palette.get(usize::try_from(i16_at(e, 0)).ok()?)?;
            (object != DatumIndex::NONE).then(|| Placement {
                object,
                position: [f32_at(e, 0x8), f32_at(e, 0xC), f32_at(e, 0x10)],
                rotation: [f32_at(e, 0x14), f32_at(e, 0x18), f32_at(e, 0x1C)],
                scale: match f32_at(e, 0x20) {
                    s if s > 0.0 => s,
                    _ => 1.0,
                },
            })
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
