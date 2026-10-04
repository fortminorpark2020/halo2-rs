//! Multiplayer colours from the game's globals: the 18 armour colours a
//! player picks from in their profile, and the team colours.

use crate::mapset::MapSet;
use crate::{f32_at, GroupTag, Result};

/// `matg` "Profile Colors": RGB, in the order the profile lists them
/// (White, Steel, Red ... Tan).
const MATG_PROFILE_COLORS: usize = 0x160;
/// `mulg` "Universal" (one element) and its "Team Colors" block: red, blue,
/// yellow, green, purple, orange, brown, pink, then neutral.
const MULG_UNIVERSAL: usize = 0x0;
const UNIVERSAL_TEAM_COLORS: usize = 0x10;
const COLOR_SIZE: usize = 0xC;

fn colors(block: &[u8]) -> Vec<[f32; 3]> {
    block
        .as_chunks::<COLOR_SIZE>()
        .0
        .iter()
        .map(|c| [f32_at(c, 0), f32_at(c, 4), f32_at(c, 8)])
        .collect()
}

/// The armour colours a player can choose.
pub fn profile_colors(set: &mut MapSet) -> Result<Vec<[f32; 3]>> {
    let matg = set.map.globals;
    let (src, _, data) = set.tag_data(matg)?;
    let file = set.get(src);
    let region = file.meta_region();
    let block = file.read_block(region, &data, MATG_PROFILE_COLORS, COLOR_SIZE)?;
    Ok(colors(&block))
}

/// The team colours (red first).
pub fn team_colors(set: &mut MapSet) -> Result<Vec<[f32; 3]>> {
    let mulg = GroupTag::parse("mulg").expect("valid group");
    let Some(tag) = set
        .shared
        .as_ref()
        .and_then(|s| s.find_tag(mulg, "multiplayer\\multiplayer_globals"))
        .or_else(|| set.map.find_tag(mulg, "multiplayer\\multiplayer_globals"))
        .map(|t| t.datum)
    else {
        return Ok(Vec::new());
    };
    let (src, _, data) = set.tag_data(tag)?;
    let file = set.get(src);
    let region = file.meta_region();
    let universal = file.read_block(region, &data, MULG_UNIVERSAL, UNIVERSAL_TEAM_COLORS + 8)?;
    if universal.len() < UNIVERSAL_TEAM_COLORS + 8 {
        return Ok(Vec::new());
    }
    let block = file.read_block(region, &universal, UNIVERSAL_TEAM_COLORS, COLOR_SIZE)?;
    Ok(colors(&block))
}
