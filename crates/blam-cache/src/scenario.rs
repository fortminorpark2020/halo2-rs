//! Objects a scenario places in its level: scenery, and the weapons and
//! equipment multiplayer games spawn ("netgame equipment").

use crate::mapset::MapSet;
use crate::{f32_at, i16_at, u32_at, DatumIndex, Error, Result};

const SCNR_SCENERY: usize = 0x50;
const SCNR_SCENERY_PALETTE: usize = 0x58;
const SCNR_NETGAME_EQUIPMENT: usize = 0x120;
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
