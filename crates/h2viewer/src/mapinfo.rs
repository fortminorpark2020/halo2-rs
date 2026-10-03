//! Halo 2's own descriptions and pictures of its multiplayer maps, from the
//! multiplayer level list in mainmenu.map's globals.

use crate::menu::MapChoice;
use blam_cache::bitmap::{self, Image};
use blam_cache::{DatumIndex, GroupTag, MapSet};
use std::path::Path;

/// matg: the user interface level data block (one element).
const UI_LEVELS: usize = 0x178;
const UI_LEVELS_SIZE: usize = 0x40;
/// In it: the multiplayer levels.
const MULTIPLAYER_LEVELS: usize = 0x10;
const LEVEL_SIZE: usize = 0xC64;
/// In a level: the picture (a tag reference's datum), the English name and
/// description (UTF-16) and the scenario's path.
const LEVEL_PICTURE: usize = 0x8;
const LEVEL_NAME: usize = 0xC;
const LEVEL_DESCRIPTION: usize = 0x24C;
const LEVEL_PATH: usize = 0xB4C;

/// One multiplayer level as the main menu describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    /// The scenario's file name (as the `.map` file is named).
    pub name: String,
    pub title: String,
    pub description: String,
    pub picture: DatumIndex,
}

fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&b| u16::from_le_bytes(b))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// Parse the multiplayer levels block.
pub fn parse_levels(block: &[u8]) -> Vec<Level> {
    block
        .as_chunks::<LEVEL_SIZE>()
        .0
        .iter()
        .filter_map(|e| {
            let path = &e[LEVEL_PATH..LEVEL_PATH + 0x100];
            let path = String::from_utf8_lossy(&path[..path.iter().position(|&b| b == 0)?]);
            let name = path.rsplit('\\').next()?.to_lowercase();
            if name.is_empty() {
                return None;
            }
            let datum = u32::from_le_bytes(e[LEVEL_PICTURE..LEVEL_PICTURE + 4].try_into().ok()?);
            Some(Level {
                name,
                title: utf16(&e[LEVEL_NAME..LEVEL_NAME + 0x40]),
                description: utf16(&e[LEVEL_DESCRIPTION..LEVEL_DESCRIPTION + 0x100]),
                picture: DatumIndex(datum),
            })
        })
        .collect()
}

fn read_levels(set: &mut MapSet) -> blam_cache::Result<Vec<Level>> {
    let matg = GroupTag::parse("matg").expect("a group tag");
    let datum = set
        .map
        .tags
        .iter()
        .find(|t| t.group == matg)
        .map(|t| t.datum)
        .ok_or_else(|| blam_cache::Error::Corrupt("no globals".into()))?;
    let (src, _, data) = set.tag_data(datum)?;
    let file = set.get(src);
    let region = file.meta_region();
    let ui = file.read_block(region, &data, UI_LEVELS, UI_LEVELS_SIZE)?;
    if ui.len() < UI_LEVELS_SIZE {
        return Ok(Vec::new());
    }
    let block = file.read_block(region, &ui, MULTIPLAYER_LEVELS, LEVEL_SIZE)?;
    Ok(parse_levels(&block))
}

/// Give `maps` Halo 2's descriptions and pictures from the mainmenu.map in
/// `dir`. Returns the pictures, which `MapChoice::picture` indexes.
pub fn describe_maps(dir: &Path, maps: &mut [MapChoice]) -> Vec<Image> {
    let mut pictures = Vec::new();
    let Ok(mut set) = MapSet::open(dir.join("mainmenu.map")) else {
        return pictures;
    };
    let Ok(levels) = read_levels(&mut set) else {
        return pictures;
    };
    for map in maps {
        let Some(level) = levels.iter().find(|l| l.name == map.name) else {
            continue;
        };
        map.description = level.description.clone();
        if let Ok(mut img) = bitmap::read_bitmap_at(&mut set, level.picture, 0) {
            // Their alpha is a little under opaque, with the bottom right
            // corner cut off: keep only the cut.
            for px in img.rgba.as_chunks_mut::<4>().0 {
                px[3] = if px[3] > 0 { 255 } else { 0 };
            }
            pictures.push(img);
            map.picture = Some(pictures.len() - 1);
        }
    }
    pictures
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(name: &str, description: &str, path: &str, picture: u32) -> Vec<u8> {
        let mut e = vec![0u8; LEVEL_SIZE];
        e[4..8].copy_from_slice(b"mtib");
        e[LEVEL_PICTURE..LEVEL_PICTURE + 4].copy_from_slice(&picture.to_le_bytes());
        let put = |e: &mut Vec<u8>, at: usize, s: &str| {
            for (k, u) in s.encode_utf16().enumerate() {
                e[at + 2 * k..at + 2 * k + 2].copy_from_slice(&u.to_le_bytes());
            }
        };
        put(&mut e, LEVEL_NAME, name);
        put(&mut e, LEVEL_DESCRIPTION, description);
        e[LEVEL_PATH..LEVEL_PATH + path.len()].copy_from_slice(path.as_bytes());
        e
    }

    #[test]
    fn levels_are_named_by_their_scenario_file() {
        let mut block = level(
            "Beaver Creek",
            "Water. Grass. Rocks.",
            "scenarios\\multi\\halo\\beavercreek\\beavercreek",
            0xE77D_0559,
        );
        block.extend(level("", "", "", 0xFFFF_FFFF));
        let levels = parse_levels(&block);
        assert_eq!(
            levels,
            [Level {
                name: "beavercreek".into(),
                title: "Beaver Creek".into(),
                description: "Water. Grass. Rocks.".into(),
                picture: DatumIndex(0xE77D_0559),
            }]
        );
    }
}
