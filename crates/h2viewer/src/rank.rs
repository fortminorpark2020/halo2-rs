//! Halo 2's rank icons: one for each level, 1 to 50, shown beside
//! gamertags when a player's level is known (playing online).
//!
//! The icons come from `ui\global_bitmaps\rank_icons_sm` (17x17) and
//! `rank_icons` (28x26) in the player's own mainmenu.map, icon k for level
//! k + 1. Each set is packed into an atlas and drawn in its own colours.

use crate::gpu::{hud_mode, RANK_TEXTURES};
use crate::hud::HudBuilder;
use blam_cache::bitmap::{self, Image};
use blam_cache::{GroupTag, MapSet};
use std::path::Path;

/// The highest level.
pub const MAX_LEVEL: u8 = 50;

/// The small and the large icons, in texture order, and Halo 2's size of
/// them in pixels.
const SETS: [(&str, [usize; 2]); 2] = [
    ("ui\\global_bitmaps\\rank_icons_sm", [17, 17]),
    ("ui\\global_bitmaps\\rank_icons", [28, 26]),
];
/// Icons drawn at least this many pixels tall come from the large set.
const LARGE_FROM: f32 = 22.0;
/// Icons across and down an atlas.
const ACROSS: usize = 10;
const ROWS: usize = (MAX_LEVEL as usize).div_ceil(ACROSS);
/// Each icon's square in an atlas, with clear pixels around the icon so
/// filtering doesn't reach its neighbours (a power of two, so smaller
/// mipmaps keep them apart too).
const CELL: usize = 32;
/// Where an icon starts in its square.
const PAD: usize = 1;

/// The atlases, small then large.
pub fn load(dir: &Path) -> Option<Vec<Image>> {
    let mut set = MapSet::open(dir.join("mainmenu.map")).ok()?;
    let group = GroupTag::parse("bitm")?;
    SETS.iter()
        .map(|&(name, size)| {
            let tag = set.map.find_tag(group, name)?.datum;
            let icons: Vec<Image> = (0..MAX_LEVEL as usize)
                .map_while(|i| bitmap::read_bitmap_at(&mut set, tag, i).ok())
                .collect();
            if icons.len() < MAX_LEVEL as usize {
                println!("warning: {name}: {} of {MAX_LEVEL} icons", icons.len());
            }
            (!icons.is_empty()).then(|| atlas(&icons, size))
        })
        .collect()
}

/// The icons copied into a grid, by level, at most `size` of each.
fn atlas(icons: &[Image], size: [usize; 2]) -> Image {
    let (w, h) = (ACROSS * CELL, ROWS * CELL);
    let mut rgba = vec![0u8; w * h * 4];
    for (k, icon) in icons.iter().enumerate().take(MAX_LEVEL as usize) {
        let (ox, oy) = ((k % ACROSS) * CELL + PAD, (k / ACROSS) * CELL + PAD);
        let iw = icon.width as usize;
        let n = iw.min(size[0]) * 4;
        for y in 0..(icon.height as usize).min(size[1]) {
            let to = ((oy + y) * w + ox) * 4;
            rgba[to..to + n].copy_from_slice(&icon.rgba[y * iw * 4..][..n]);
        }
    }
    Image {
        width: w as u32,
        height: h as u32,
        rgba,
    }
}

/// Where level `level`'s icon sits in atlas `set`, as texture coordinates.
fn cell(set: usize, level: u8) -> [f32; 4] {
    let size = SETS[set].1;
    let k = (level.clamp(1, MAX_LEVEL) - 1) as usize;
    let (x, y) = ((k % ACROSS) * CELL + PAD, (k / ACROSS) * CELL + PAD);
    let (x, y) = (x as f32, y as f32);
    let (w, h) = ((ACROSS * CELL) as f32, (ROWS * CELL) as f32);
    [
        x / w,
        y / h,
        (x + size[0] as f32) / w,
        (y + size[1] as f32) / h,
    ]
}

/// Draw level `level`'s icon as large as fits in the middle of `rect`
/// (window pixels), from the set nearer that size.
pub fn draw(hb: &mut HudBuilder, rect: [f32; 4], level: u8) {
    let [x0, y0, x1, y1] = rect;
    let set = (y1 - y0 >= LARGE_FROM) as usize;
    let [w, h] = SETS[set].1.map(|v| v as f32);
    let scale = ((x1 - x0) / w).min((y1 - y0) / h);
    let (dx, dy) = (w * scale * 0.5, h * scale * 0.5);
    let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    hb.quad(
        RANK_TEXTURES + set,
        [cx - dx, cy - dy, cx + dx, cy + dy],
        cell(set, level),
        [1.0; 4],
        hud_mode::PLAIN,
        0.0,
    );
}

/// H2_TEST_LEVELS=1 gives everyone a level from their place in a list (for
/// testing; levels otherwise come only from playing online).
pub fn test_level(slot: usize) -> Option<u8> {
    let spread = slot * 7 % MAX_LEVEL as usize;
    std::env::var_os("H2_TEST_LEVELS").map(|_| MAX_LEVEL - spread as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_find_their_icons() {
        // Level 1 top left, level 50 bottom right, each inside its square.
        let (w, h) = (320.0, 160.0);
        assert_eq!(cell(0, 1), [1.0 / w, 1.0 / h, 18.0 / w, 18.0 / h]);
        assert_eq!(cell(0, 50), [289.0 / w, 129.0 / h, 306.0 / w, 146.0 / h]);
        // Large icons are 28 by 26; level 12 is the second row's second.
        assert_eq!(cell(1, 12), [33.0 / w, 33.0 / h, 61.0 / w, 59.0 / h]);
        // Out of range clamps rather than reading past the atlas.
        assert_eq!(cell(0, 0), cell(0, 1));
        assert_eq!(cell(0, 99), cell(0, 50));
    }

    #[test]
    fn icons_are_copied_into_their_cells() {
        // Two 17x17 icons, one all red and one all green.
        let icon = |px: [u8; 4]| Image {
            width: 17,
            height: 17,
            rgba: px.repeat(17 * 17),
        };
        let a = atlas(&[icon([255, 0, 0, 255]), icon([0, 255, 0, 255])], [17, 17]);
        assert_eq!((a.width, a.height), (320, 160));
        let at = |x: usize, y: usize| &a.rgba[(y * 320 + x) * 4..][..4];
        assert_eq!(at(0, 0), [0, 0, 0, 0], "the edge stays clear");
        assert_eq!(at(1, 1), [255, 0, 0, 255]);
        assert_eq!(at(17, 17), [255, 0, 0, 255]);
        assert_eq!(at(18, 18), [0, 0, 0, 0]);
        assert_eq!(at(33, 1), [0, 255, 0, 255]);
        assert_eq!(at(65, 1), [0, 0, 0, 0], "no third icon");
    }
}
