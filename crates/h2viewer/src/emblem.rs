//! Halo 2's emblems: a foreground picture in two colours over a background
//! pattern in a third, drawn in the menus and scoreboards.
//!
//! The pictures come from `ui\global_bitmaps\emblems` in the maps folder.
//! In them yellow (red channel) marks the first colour, blue the second and
//! black is see-through (their alpha only repeats the blue). Each colour
//! becomes its own white mask in an atlas, so an emblem draws as four
//! tinted quads. Armour shows just the picture, from one more atlas that
//! holds both colours' masks (red and green).

use crate::gpu::{hud_mode, EMBLEM_TEXTURES};
use crate::hud::HudBuilder;
use blam_cache::bitmap::{self, Image};
use blam_cache::{GroupTag, MapSet};
use h2sim::game::{Emblem, EMBLEM_BACKGROUNDS, EMBLEM_FOREGROUNDS};
use std::path::Path;

const FOREGROUND: &str = "ui\\global_bitmaps\\emblems\\foreground";
const BACKGROUND: &str = "ui\\global_bitmaps\\emblems\\background";
/// Each picture's size in the atlases.
const CELL: usize = 64;
/// Pictures across an atlas.
const ACROSS: usize = 8;

/// The armour atlas's place among the atlases `load` returns.
pub const ARMOUR_ATLAS: usize = 4;

/// The atlases in texture order: foreground first colour, foreground second
/// colour, background first colour, background second colour, then the
/// armour atlas.
pub fn load(dir: &Path) -> Option<Vec<Image>> {
    let mut set = ["mainmenu.map", "shared.map"]
        .iter()
        .find_map(|m| MapSet::open(dir.join(m)).ok())?;
    let [fg, bg] = [
        (FOREGROUND, EMBLEM_FOREGROUNDS),
        (BACKGROUND, EMBLEM_BACKGROUNDS),
    ]
    .map(|(name, count)| pictures(&mut set, name, count as usize));
    let (fg, bg) = (fg?, bg?);
    let [fg1, fg2] = [0, 2].map(|c| atlas(&fg, c));
    let [bg1, bg2] = [0, 2].map(|c| atlas(&bg, c));
    let armour = both(&fg1, &fg2);
    Some(vec![fg1, fg2, bg1, bg2, armour])
}

/// Two masks as one picture: the first in red, the second in green.
fn both(first: &Image, second: &Image) -> Image {
    let mut rgba = vec![0u8; first.rgba.len()];
    for (k, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        px[0] = first.rgba[k * 4 + 3];
        px[1] = second.rgba[k * 4 + 3];
        px[3] = 255;
    }
    Image { rgba, ..*first }
}

fn pictures(set: &mut MapSet, name: &str, count: usize) -> Option<Vec<Image>> {
    let group = GroupTag::parse("bitm")?;
    let tag = set.map.find_tag(group, name)?.datum;
    let images: Vec<Image> = (0..count)
        .map_while(|i| bitmap::read_bitmap_at(set, tag, i).ok())
        .collect();
    if images.len() < count {
        println!(
            "warning: {name}: {} of {count} emblem pictures",
            images.len()
        );
    }
    (!images.is_empty()).then_some(images)
}

/// White masks of one colour channel (0: yellow's red, 2: blue), shrunk
/// into a grid.
fn atlas(pictures: &[Image], channel: usize) -> Image {
    let rows = pictures.len().div_ceil(ACROSS);
    let (w, h) = (ACROSS * CELL, rows * CELL);
    let mut rgba = vec![255u8; w * h * 4];
    for px in rgba.as_chunks_mut::<4>().0 {
        px[3] = 0;
    }
    for (k, img) in pictures.iter().enumerate() {
        let (ox, oy) = ((k % ACROSS) * CELL, (k / ACROSS) * CELL);
        let (iw, ih) = (img.width as usize, img.height as usize);
        for y in 0..CELL {
            for x in 0..CELL {
                // Average the source pixels this cell pixel covers.
                let (x0, x1) = (x * iw / CELL, ((x + 1) * iw / CELL).max(x * iw / CELL + 1));
                let (y0, y1) = (y * ih / CELL, ((y + 1) * ih / CELL).max(y * ih / CELL + 1));
                let (mut sum, mut n) = (0u32, 0u32);
                for sy in y0..y1.min(ih) {
                    for sx in x0..x1.min(iw) {
                        sum += img.rgba[(sy * iw + sx) * 4 + channel] as u32;
                        n += 1;
                    }
                }
                let i = ((oy + y) * w + ox + x) * 4;
                rgba[i + 3] = (sum / n.max(1)) as u8;
            }
        }
    }
    Image {
        width: w as u32,
        height: h as u32,
        rgba,
    }
}

/// Where picture `k` sits in an atlas of `count` pictures, as texture
/// coordinates.
fn cell(k: u8, count: u8) -> [f32; 4] {
    let rows = (count as usize).div_ceil(ACROSS) as f32;
    let k = k as usize % count as usize;
    let (x, y) = ((k % ACROSS) as f32, (k / ACROSS) as f32);
    let across = ACROSS as f32;
    [x / across, y / rows, (x + 1.0) / across, (y + 1.0) / rows]
}

/// Where foreground picture `k` sits in the armour atlas, pulled in half
/// a texel so filtering stays inside it.
pub fn armour_cell(k: u8) -> [f32; 4] {
    let [x0, y0, x1, y1] = cell(k, EMBLEM_FOREGROUNDS);
    let rows = (EMBLEM_FOREGROUNDS as usize).div_ceil(ACROSS);
    let (dx, dy) = (0.5 / (ACROSS * CELL) as f32, 0.5 / (rows * CELL) as f32);
    [x0 + dx, y0 + dy, x1 - dx, y1 - dy]
}

/// A profile colour as a HUD colour (they're in gamma space, the HUD's
/// colours linear), darkened by `shade`.
fn hud_color(color: u8, shade: f32) -> [f32; 4] {
    let c = crate::profile::color(color);
    let lin = |v: f32| (v * shade).powf(2.2);
    [lin(c[0]), lin(c[1]), lin(c[2]), 1.0]
}

/// Draw `emblem` filling `rect` (window pixels).
pub fn draw(hb: &mut HudBuilder, rect: [f32; 4], emblem: Emblem) {
    let [primary, secondary, back] = emblem.colors;
    let bg = cell(emblem.background, EMBLEM_BACKGROUNDS);
    let fg = cell(emblem.foreground, EMBLEM_FOREGROUNDS);
    // The background's second colour is a darker shade of its first.
    let layers = [
        (2, bg, hud_color(back, 1.0)),
        (3, bg, hud_color(back, 0.55)),
        (0, fg, hud_color(primary, 1.0)),
        (1, fg, hud_color(secondary, 1.0)),
    ];
    for (texture, uv, color) in layers {
        hb.quad(
            EMBLEM_TEXTURES + texture,
            rect,
            uv,
            color,
            hud_mode::PLAIN,
            0.0,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_cells_tile_without_overlap() {
        assert_eq!(cell(0, 64), [0.0, 0.0, 0.125, 0.125]);
        assert_eq!(cell(63, 64), [0.875, 0.875, 1.0, 1.0]);
        assert_eq!(cell(9, 32), [0.125, 0.25, 0.25, 0.5]);
        // Out of range wraps rather than reading past the atlas.
        assert_eq!(cell(64, 64), cell(0, 64));
    }

    #[test]
    fn masks_keep_each_colour_and_the_cut_out() {
        // One 2x2 picture: yellow, blue, black, and dark yellow (an
        // edge). Alpha repeats the blue, as in Halo 2's pictures.
        let img = Image {
            width: 2,
            height: 2,
            rgba: vec![255, 255, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0, 128, 128, 0, 0],
        };
        let yellow = atlas(std::slice::from_ref(&img), 0);
        let at = |a: &Image, x: usize, y: usize| a.rgba[(y * a.width as usize + x) * 4 + 3];
        // The 64-pixel cell's four quarters come from the four pixels.
        assert_eq!(at(&yellow, 0, 0), 255);
        assert_eq!(at(&yellow, 40, 0), 0);
        assert_eq!(at(&yellow, 0, 40), 0);
        assert_eq!(at(&yellow, 40, 40), 128);
        let blue = atlas(std::slice::from_ref(&img), 2);
        assert_eq!(at(&blue, 40, 0), 255);
        assert_eq!(at(&blue, 0, 0), 0);
    }

    #[test]
    fn armour_atlas_holds_both_colours() {
        let img = Image {
            width: 2,
            height: 2,
            rgba: vec![255, 255, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0, 128, 128, 0, 0],
        };
        let one = std::slice::from_ref(&img);
        let armour = both(&atlas(one, 0), &atlas(one, 2));
        let px = |x: usize, y: usize| {
            let i = (y * armour.width as usize + x) * 4;
            [armour.rgba[i], armour.rgba[i + 1]]
        };
        assert_eq!(px(0, 0), [255, 0]);
        assert_eq!(px(40, 0), [0, 255]);
        assert_eq!(px(0, 40), [0, 0]);
        // Armour cells stay inside their picture.
        let [x0, y0, x1, y1] = armour_cell(9);
        let [a0, b0, a1, b1] = cell(9, EMBLEM_FOREGROUNDS);
        assert!(x0 > a0 && y0 > b0 && x1 < a1 && y1 < b1);
    }
}
