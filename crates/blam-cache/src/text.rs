//! Text shown to the player: unicode string lists (`unic`: chapter titles,
//! objectives, subtitles), whose strings live in the map's language tables
//! (English here).

use crate::mapset::{Map, MapSet};
use crate::{u32_at, DatumIndex, Error, GroupTag, Result};

/// The English language table in `matg`: string count, data size, index
/// offset and data offset.
const MATG_ENGLISH: usize = 0x190;
/// Each language's range of the table in a `unic` tag (English first).
const UNIC_RANGES: usize = 0x10;
/// Language table offsets with this bit are in the shared map.
const SHARED: u32 = 0x8000_0000;

/// Every English string in a map's language table, with its string id.
pub fn language_table(set: &mut MapSet) -> Result<Vec<(u32, String)>> {
    let matg = GroupTag::parse("matg").unwrap();
    let tag = set
        .map
        .tags
        .iter()
        .find(|t| t.group == matg)
        .cloned()
        .ok_or_else(|| Error::Corrupt("no globals tag".into()))?;
    let data = set.map.read_tag_data(&tag)?;
    if data.len() < MATG_ENGLISH + 0x10 {
        return Err(Error::Corrupt("globals tag too small".into()));
    }
    let count = u32_at(&data, MATG_ENGLISH) as usize;
    let size = u32_at(&data, MATG_ENGLISH + 4) as usize;
    let index = u32_at(&data, MATG_ENGLISH + 8);
    let strings = u32_at(&data, MATG_ENGLISH + 0xC);
    if count == 0 || count > 1 << 20 || size > 1 << 26 {
        return Ok(Vec::new());
    }
    let file: &mut Map = if index & SHARED != 0 {
        set.shared
            .as_mut()
            .ok_or_else(|| Error::Corrupt("language table in a missing shared map".into()))?
    } else {
        &mut set.map
    };
    let index = file.read_raw((index & !SHARED) as u64, count * 8)?;
    let strings = file.read_raw((strings & !SHARED) as u64, size)?;
    Ok(index
        .as_chunks::<8>()
        .0
        .iter()
        .map(|e| {
            let at = (u32_at(e, 4) as usize).min(strings.len());
            let rest = &strings[at..];
            let end = rest.iter().position(|&c| c == 0).unwrap_or(rest.len());
            (
                u32_at(e, 0),
                String::from_utf8_lossy(&rest[..end]).into_owned(),
            )
        })
        .collect())
}

/// A unicode string list's English strings, with their string ids, from
/// the map's language table.
pub fn unicode_strings(
    set: &mut MapSet,
    table: &[(u32, String)],
    unic: DatumIndex,
) -> Result<Vec<(u32, String)>> {
    let (_, _, data) = set.tag_data(unic)?;
    if data.len() < UNIC_RANGES + 4 {
        return Err(Error::Corrupt("string list too small".into()));
    }
    let start = u16::from_le_bytes([data[UNIC_RANGES], data[UNIC_RANGES + 1]]) as usize;
    let count = u16::from_le_bytes([data[UNIC_RANGES + 2], data[UNIC_RANGES + 3]]) as usize;
    Ok(table
        .get(start..(start + count).min(table.len()))
        .unwrap_or_default()
        .to_vec())
}
