//! The Halo 2 shell every screen after the main menu sits in (menu.md 1.3):
//! `game_shell_background` behind it (the navy, the framing and the track
//! strips), Halo 2's header at its top left and its button legend at its
//! bottom right, from a `ChromeLayout` (`tags::chrome`, or the built-in
//! numbers).
//!
//! What is behind comes as two lists: `still`, the same at every clock
//! (a caller draws it once for a window size, `cpu::Kept`, and copies it
//! in under each frame), and `moving`, the strips that scroll, drawn every
//! frame. Both are drawn settled: how the shell comes in when it first
//! shows (each piece's intro) is left for the screen transitions.
//!
//! The legend names the controller's buttons by their glyphs after the
//! pad was used, and the keys that do the same (`layout::key_name`) after
//! the keyboard or the mouse, and says where each button's part of it is,
//! so a click on it can press it.

use crate::art::ArtSource;
use crate::layout::{
    colour, key_name, BitmapWidget, ChromeLayout, DialogSize, Face, LegendItem, Space,
};
use crate::paint::{Anchor, DrawList, Painter, TextStyle};
use crate::text::{Fonts, Justify};

/// After the pad was used: a button's glyph, this, then what it does; and
/// this between two buttons (as Halo 2's own legends space them).
pub const PAD_GAP: &str = " ";
pub const PAD_ITEM_GAP: &str = " ";
/// After the keyboard or the mouse: a key's name, this, then what it
/// does; and this between two keys, wider, so each name stays with its
/// own words. Our own spacing: Halo 2 had no keyboard legends.
pub const KEY_GAP: &str = "   ";
pub const KEY_ITEM_GAP: &str = "      ";
/// How much of the window's width (UI units) a legend may take at most,
/// shrunk to fit beyond it: our own margin.
const LEGEND_ROOM: f32 = 0.96;

/// The navy, then the still pictures (lower depths first), settled. The
/// same list at any `clock`.
pub fn still(
    clock: f64,
    c: &ChromeLayout,
    art: &dyn ArtSource,
    fonts: &Fonts,
    space: Space,
) -> DrawList {
    let mut p = Painter::new(space, art, fonts, clock);
    p.gradient_px(p.space.window(), colour::NAVY_TOP, colour::NAVY_BOTTOM);
    settled(&mut p, &c.still);
    p.finish()
}

/// The pictures that scroll (the track strips), as they are at `clock`.
pub fn moving(
    clock: f64,
    c: &ChromeLayout,
    art: &dyn ArtSource,
    fonts: &Fonts,
    space: Space,
) -> DrawList {
    let mut p = Painter::new(space, art, fonts, clock);
    settled(&mut p, &c.moving);
    p.finish()
}

/// Bitmaps by depth at their corners, whole (their intros over).
fn settled(p: &mut Painter, widgets: &[BitmapWidget]) {
    let mut sorted: Vec<&BitmapWidget> = widgets.iter().collect();
    sorted.sort_by_key(|w| w.depth);
    for w in sorted {
        p.bitmap(w, w.corner, Anchor::TopLeft, 1.0);
    }
}

/// The header of a screen of `size`: one line in its box, cut to fit it.
pub fn header(p: &mut Painter, c: &ChromeLayout, size: DialogSize, text: &str) {
    let w = c.header(size);
    let [l, _, r, _] = w.bounds;
    let line = p.fonts.fit(w.font, text, r - l);
    p.text(w.bounds, TextStyle::of(w, 1.0), &line);
}

/// A legend item's two parts: the glyph or key, and the label.
fn key(item: &LegendItem, keyboard: bool) -> String {
    if keyboard {
        key_name(item.face).to_string()
    } else {
        item.face.glyph()
    }
}

/// The button legend of a screen of `size`: `items` in order on one
/// line, ending at its box's right edge (a line wider than the box goes
/// on to its left, and one wider than the window is shrunk to fit it).
/// Each button is its glyph (`keyboard` false) or its key's name, then
/// what it does. Returns each item's box in window pixels (`[x0, y0, x1,
/// y1]`, the legend box's height) and its button, for the clicks.
pub fn legend(
    p: &mut Painter,
    c: &ChromeLayout,
    size: DialogSize,
    items: &[LegendItem],
    keyboard: bool,
) -> Vec<([f32; 4], Face)> {
    if items.is_empty() {
        return Vec::new();
    }
    let w = c.legend(size);
    let font = w.font;
    let (gap, item_gap) = match keyboard {
        true => (KEY_GAP, KEY_ITEM_GAP),
        false => (PAD_GAP, PAD_ITEM_GAP),
    };
    let fonts = p.fonts;
    let keys: Vec<String> = items.iter().map(|i| key(i, keyboard)).collect();
    let (gap, item_gap) = (fonts.width(font, gap), fonts.width(font, item_gap));
    // Each item's key and label widths, and the space between them (none
    // without a key).
    let widths: Vec<(f32, f32, f32)> = items
        .iter()
        .zip(&keys)
        .map(|(i, k)| {
            let kw = fonts.width(font, k);
            let between = if k.is_empty() { 0.0 } else { gap };
            (kw, between, fonts.width(font, &i.label))
        })
        .collect();
    let total: f32 =
        widths.iter().map(|(k, g, l)| k + g + l).sum::<f32>() + item_gap * (items.len() - 1) as f32;
    let [_, top, right, bottom] = w.bounds;
    let [shown_l, _, shown_r, _] = p.space.shown();
    let room = (shown_r - shown_l) * LEGEND_ROOM;
    let scale = if total > room && total > 0.0 {
        room / total
    } else {
        1.0
    };
    let margin = (shown_r - shown_l - room) * 0.5;
    let right = right.min(shown_r - margin);
    let mut x = (right - total * scale).max(shown_l + margin);
    let style = TextStyle {
        scale,
        justify: Justify::Left,
        ..TextStyle::of(w, 1.0)
    };
    let mut boxes = Vec::with_capacity(items.len());
    for ((item, k), (kw, between, lw)) in items.iter().zip(&keys).zip(widths) {
        let start = x;
        if !k.is_empty() {
            p.text([x, top, x + kw * scale, bottom], style, k);
        }
        x += (kw + between) * scale;
        p.text([x, top, x + lw * scale, bottom], style, &item.label);
        x += lw * scale;
        boxes.push((p.space.rect([start, top, x, bottom]), item.face));
        x += item_gap * scale;
    }
    boxes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::{ArtImage, FlatArt, MemoryArt};
    use crate::layout::{self, header_box, legend_box};
    use crate::paint::{Resources, Texture};

    fn items() -> Vec<LegendItem> {
        vec![
            LegendItem::new(Face::A, "SELECT"),
            LegendItem::new(Face::B, "BACK"),
            LegendItem::new(Face::Y, "PARTY"),
        ]
    }

    #[test]
    fn the_header_sits_in_its_box_for_every_size() {
        let fonts = Fonts::fallback();
        let c = ChromeLayout::default();
        let space = Space::new(1920, 1080);
        for size in [
            DialogSize::Full,
            DialogSize::Large,
            DialogSize::Half,
            DialogSize::Quarter,
        ] {
            let mut p = Painter::new(space, &FlatArt, &fonts, 0.0);
            header(&mut p, &c, size, "PREGAME LOBBY");
            let list = p.finish();
            assert_eq!(list.texts, ["PREGAME LOBBY"]);
            let [x0, y0, x1, y1] = space.rect(header_box(size));
            assert!(!list.quads.is_empty());
            for q in &list.quads {
                let [l, t, r, b] = q.rect;
                assert!(l >= x0 - 1.0 && r <= x1 + 1.0, "{size:?} {q:?}");
                assert!(t >= y0 - 1.0 && b <= y1 + 1.0, "{size:?} {q:?}");
                assert_eq!(q.color[0][..3], colour::HEADER);
            }
            // Left justified: it starts at the box's left.
            let first = list
                .quads
                .iter()
                .map(|q| q.rect[0])
                .fold(f32::MAX, f32::min);
            assert!((first - x0).abs() <= 1.0, "{size:?}: {first} {x0}");
        }
        // Too long for the quarter box: cut with "...".
        let mut p = Painter::new(space, &FlatArt, &fonts, 0.0);
        let long = "A HEADER FAR TOO LONG FOR THE QUARTER DIALOG'S BOX";
        header(&mut p, &c, DialogSize::Quarter, long);
        let list = p.finish();
        assert!(list.texts[0].ends_with("..."), "{:?}", list.texts);
        let [_, _, x1, _] = space.rect(header_box(DialogSize::Quarter));
        assert!(list.quads.iter().all(|q| q.rect[2] <= x1 + 1.0));
    }

    #[test]
    fn legend_boxes_are_in_order_and_end_at_the_boxs_right() {
        let fonts = Fonts::fallback();
        let c = ChromeLayout::default();
        let space = Space::new(1280, 720);
        for keyboard in [false, true] {
            let mut p = Painter::new(space, &FlatArt, &fonts, 0.0);
            let boxes = legend(&mut p, &c, DialogSize::Full, &items(), keyboard);
            let faces: Vec<Face> = boxes.iter().map(|b| b.1).collect();
            assert_eq!(faces, [Face::A, Face::B, Face::Y]);
            let [x0, y0, x1, y1] = space.rect(legend_box(DialogSize::Full));
            // Ending at the box's right edge within a pixel.
            assert!((boxes[2].0[2] - x1).abs() <= 1.0, "{boxes:?}");
            for (k, (b, _)) in boxes.iter().enumerate() {
                assert!(b[0] < b[2] && b[0] >= x0 - 1.0, "{boxes:?}");
                assert_eq!([b[1], b[3]], [y0, y1]);
                if k > 0 {
                    assert!(boxes[k - 1].0[2] < b[0], "overlapping: {boxes:?}");
                }
            }
            let list = p.finish();
            // Every glyph quad is inside the boxes' span.
            for q in &list.quads {
                assert!(q.rect[0] >= boxes[0].0[0] - 1.0 && q.rect[2] <= x1 + 1.0);
            }
            if keyboard {
                assert_eq!(list.texts, ["ENTER", "SELECT", "ESC", "BACK", "Y", "PARTY"]);
            } else {
                assert_eq!(
                    list.texts,
                    ["\u{e100}", "SELECT", "\u{e101}", "BACK", "\u{e103}", "PARTY"]
                );
                // Without Halo 2's font, the glyphs are its coloured discs.
                assert!(
                    list.quads
                        .iter()
                        .any(|q| q.texture == Texture::Fallback
                            && q.color[0][..3] == [0.25, 0.7, 0.2])
                );
            }
        }
        // A key with no name (White) shows its label alone.
        let mut p = Painter::new(space, &FlatArt, &fonts, 0.0);
        let white = [LegendItem::new(Face::White, "MUTE")];
        legend(&mut p, &c, DialogSize::Full, &white, true);
        assert_eq!(p.finish().texts, ["MUTE"]);
        // None: nothing.
        let mut p = Painter::new(space, &FlatArt, &fonts, 0.0);
        assert!(legend(&mut p, &c, DialogSize::Full, &[], false).is_empty());
        assert!(p.finish().quads.is_empty());
    }

    #[test]
    fn an_over_wide_legend_stays_in_the_window() {
        let fonts = Fonts::fallback();
        let c = ChromeLayout::default();
        let many: Vec<LegendItem> = (0..12)
            .map(|k| LegendItem::new(Face::X, &format!("A LONG LABEL NUMBER {k}")))
            .collect();
        for (w, h) in [(1280, 720), (640, 480), (2560, 1080)] {
            let space = Space::new(w, h);
            let mut p = Painter::new(space, &FlatArt, &fonts, 0.0);
            let boxes = legend(&mut p, &c, DialogSize::Full, &many, true);
            assert_eq!(boxes.len(), 12);
            let list = p.finish();
            for q in &list.quads {
                assert!(q.rect[0] >= 0.0 && q.rect[2] <= w as f32, "{w}x{h}: {q:?}");
            }
            assert!(boxes[0].0[0] >= 0.0 && boxes[11].0[2] <= w as f32);
        }
        // Two items a little wider than the box: on past its left, unshrunk.
        let space = Space::new(1280, 720);
        let two = [
            LegendItem::new(Face::A, "A LABEL RATHER LONG FOR IT"),
            LegendItem::new(Face::B, "AND ANOTHER AS LONG"),
        ];
        let mut p = Painter::new(space, &FlatArt, &fonts, 0.0);
        let boxes = legend(&mut p, &c, DialogSize::Full, &two, true);
        let [x0, _, x1, _] = space.rect(legend_box(DialogSize::Full));
        assert!(boxes[0].0[0] < x0, "{boxes:?}");
        assert!((boxes[1].0[2] - x1).abs() <= 1.0);
    }

    /// A chrome of a framing, a still strip and two scrolling ones, with
    /// stand-in pictures.
    fn pictured() -> (ChromeLayout, MemoryArt) {
        let mut art = MemoryArt::new();
        let img = |w: usize, h: usize, v: u8| ArtImage {
            width: w,
            height: h,
            rgba: [v, v, 255, 200].repeat(w * h),
        };
        art.add_image(layout::FRAMING, img(64, 40, 30));
        art.add_image("track7a", img(32, 2, 90));
        art.add_image("track5", img(32, 2, 120));
        art.add_image("track7c", img(32, 2, 150));
        let strip = |name: &str, y: f32, wraps: f32| BitmapWidget {
            scale: [1.1, 1.1],
            wraps_per_second: [wraps, 0.0],
            depth: 11,
            ..BitmapWidget::new(name, [-1100.0, y])
        };
        let c = ChromeLayout {
            still: vec![
                strip("track7a", -284.0, 0.0),
                BitmapWidget {
                    scale: [32.0, 32.0],
                    depth: 10,
                    ..BitmapWidget::new(layout::FRAMING, [-1070.0, 654.0])
                },
            ],
            moving: vec![strip("track5", 136.0, 0.08), strip("track7c", 475.0, 0.15)],
            ..ChromeLayout::default()
        };
        (c, art)
    }

    #[test]
    fn the_strips_move_and_the_rest_stays() {
        let fonts = Fonts::fallback();
        let (c, art) = pictured();
        let space = Space::new(640, 360);
        let a = still(1.0, &c, &art, &fonts, space);
        let b = still(7.5, &c, &art, &fonts, space);
        assert_eq!(a, b, "the kept list is the same at any clock");
        // The navy, then the framing (depth 10) under the still strip.
        assert_eq!(a.quads.len(), 3);
        assert_eq!(a.quads[0].color, [colour::NAVY_TOP, colour::NAVY_BOTTOM]);
        assert_eq!(a.quads[1].texture, Texture::Art(0));
        let m1 = moving(1.0, &c, &art, &fonts, space);
        let m2 = moving(2.0, &c, &art, &fonts, space);
        assert_eq!(m1.quads.len(), 2);
        assert_ne!(m1, m2, "the strips scroll");
        assert!(m1.quads.iter().all(|q| q.wrap == [true, false]));
        // A second on, the first strip has moved 0.08 of its width.
        let du = m2.quads[0].uv[0] - m1.quads[0].uv[0];
        assert!((du - 0.08).abs() < 1e-4, "{du}");
        // Drawn on the CPU: the window is covered.
        let mut px = vec![0u32; 640 * 360];
        let res = Resources {
            art: &art,
            fonts: &fonts,
        };
        crate::cpu::draw_u32(&a, &mut px, 640, 360, &res);
        crate::cpu::draw_u32(&m1, &mut px, 640, 360, &res);
        assert!(px.iter().all(|&p| p != 0));
        // Without the art, the built-in chrome is the navy alone.
        let flat = ChromeLayout::default();
        let s = still(0.0, &flat, &FlatArt, &fonts, space);
        assert_eq!(s.quads.len(), 1);
        assert!(moving(0.0, &flat, &FlatArt, &fonts, space).quads.is_empty());
    }
}
