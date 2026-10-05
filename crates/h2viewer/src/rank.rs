//! Halo 2's rank icons: one for each level, 1 to 50, shown beside
//! gamertags when a player's level is known (playing online).
//!
//! The icons come from `ui\global_bitmaps\rank_icons_sm` (17x17) and
//! `rank_icons` (28x26) in the player's own mainmenu.map, icon k for level
//! k + 1. Each set is packed into an atlas and drawn in its own colours.
//! Three of Xbox Live's icons from `live_icons_sm` go in a third atlas: the
//! party leader's and a party's, shown in the online lists, and the
//! download arrow that marks playlists the party lacks maps for.

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
/// Xbox Live's small icons, and the ones `LiveIcon` names, in its order.
const LIVE_ICONS: &str = "ui\\global_bitmaps\\live_icons_sm";
const LIVE_PICKS: [usize; 3] = [29, 19, 8];
/// Their squares in the atlas, and the middle part of each that is drawn
/// (each about 42 by 42).
const LIVE_CELL: usize = 64;
const LIVE_SIZE: usize = 44;

/// One of Xbox Live's icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveIcon {
    /// A person with a crown: the party leader.
    Leader,
    /// Two people: someone in a party.
    Party,
    /// A down arrow: maps to get.
    Download,
}

/// The atlases: small rank icons, large ones, then the live icons.
pub fn load(dir: &Path) -> Option<Vec<Image>> {
    let mut set = MapSet::open(dir.join("mainmenu.map")).ok()?;
    let group = GroupTag::parse("bitm")?;
    let mut atlases: Vec<Image> = SETS
        .iter()
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
        .collect::<Option<_>>()?;
    // Without the live icons the rank icons still show (and the live ones
    // draw nothing).
    match live_icons(&mut set, group) {
        Some(icons) => atlases.push(live_atlas(&icons)),
        None => println!("warning: no {LIVE_ICONS} {LIVE_PICKS:?}"),
    }
    Some(atlases)
}

/// The live icons `LiveIcon` names, if mainmenu.map has them all.
fn live_icons(set: &mut MapSet, group: GroupTag) -> Option<Vec<Image>> {
    let tag = set.map.find_tag(group, LIVE_ICONS)?.datum;
    LIVE_PICKS
        .iter()
        .map(|&i| bitmap::read_bitmap_at(set, tag, i).ok())
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

/// The live icons side by side, each in the middle of its square.
fn live_atlas(icons: &[Image]) -> Image {
    let (w, h) = (icons.len() * LIVE_CELL, LIVE_CELL);
    let mut rgba = vec![0u8; w * h * 4];
    for (k, icon) in icons.iter().enumerate() {
        let (iw, ih) = (icon.width as usize, icon.height as usize);
        let (cw, ch) = (iw.min(LIVE_SIZE), ih.min(LIVE_SIZE));
        let ox = k * LIVE_CELL + (LIVE_CELL - cw) / 2;
        let oy = (LIVE_CELL - ch) / 2;
        for y in 0..ch {
            let to = ((oy + y) * w + ox) * 4;
            rgba[to..to + cw * 4].copy_from_slice(&icon.rgba[y * iw * 4..][..cw * 4]);
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

/// Where a live icon's drawn part sits in its atlas.
fn live_cell(icon: LiveIcon) -> [f32; 4] {
    let n = LIVE_PICKS.len();
    let x = (icon as usize * LIVE_CELL + (LIVE_CELL - LIVE_SIZE) / 2) as f32;
    let y = ((LIVE_CELL - LIVE_SIZE) / 2) as f32;
    let (w, h) = ((n * LIVE_CELL) as f32, LIVE_CELL as f32);
    let size = LIVE_SIZE as f32;
    [x / w, y / h, (x + size) / w, (y + size) / h]
}

/// The largest `w` by `h` rectangle that fits in the middle of `rect`.
fn fit([x0, y0, x1, y1]: [f32; 4], w: f32, h: f32) -> [f32; 4] {
    let scale = ((x1 - x0) / w).min((y1 - y0) / h);
    let (dx, dy) = (w * scale * 0.5, h * scale * 0.5);
    let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    [cx - dx, cy - dy, cx + dx, cy + dy]
}

/// Draw level `level`'s icon as large as fits in the middle of `rect`
/// (window pixels), from the set nearer that size.
pub fn draw(hb: &mut HudBuilder, rect: [f32; 4], level: u8) {
    let set = (rect[3] - rect[1] >= LARGE_FROM) as usize;
    let [w, h] = SETS[set].1.map(|v| v as f32);
    let uv = cell(set, level);
    hb.quad(
        RANK_TEXTURES + set,
        fit(rect, w, h),
        uv,
        [1.0; 4],
        hud_mode::PLAIN,
        0.0,
    );
}

/// Draw a live icon as large as fits in the middle of `rect`.
pub fn draw_live(hb: &mut HudBuilder, rect: [f32; 4], icon: LiveIcon) {
    let uv = live_cell(icon);
    let texture = RANK_TEXTURES + SETS.len();
    hb.quad(
        texture,
        fit(rect, 1.0, 1.0),
        uv,
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

    #[test]
    fn live_icons_sit_in_the_middle_of_their_squares() {
        // A 46x42 leader, then 42x42 party and download icons.
        let icon = |w: u32, px: [u8; 4]| Image {
            width: w,
            height: 42,
            rgba: px.repeat(w as usize * 42),
        };
        let a = live_atlas(&[icon(46, [255; 4]), icon(42, [9; 4]), icon(42, [5; 4])]);
        assert_eq!((a.width, a.height), (192, 64));
        let at = |x: usize, y: usize| a.rgba[(y * 192 + x) * 4];
        assert_eq!(
            (at(9, 11), at(10, 11), at(53, 52), at(54, 52)),
            (0, 255, 255, 0)
        );
        assert_eq!(
            (at(74, 32), at(75, 32), at(116, 32), at(117, 32)),
            (0, 9, 9, 0)
        );
        assert_eq!((at(138, 32), at(139, 32), at(180, 32)), (0, 5, 5));
        // Each draws its square's middle 44x44.
        assert_eq!(
            live_cell(LiveIcon::Leader),
            [10.0 / 192.0, 10.0 / 64.0, 54.0 / 192.0, 54.0 / 64.0]
        );
        assert_eq!(
            live_cell(LiveIcon::Party),
            [74.0 / 192.0, 10.0 / 64.0, 118.0 / 192.0, 54.0 / 64.0]
        );
        assert_eq!(
            live_cell(LiveIcon::Download),
            [138.0 / 192.0, 10.0 / 64.0, 182.0 / 192.0, 54.0 / 64.0]
        );
    }
}
