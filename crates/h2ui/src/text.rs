//! Lines of text in Halo 2's fonts, laid out in UI units, ready to become
//! glyph quads.
//!
//! The fonts are the player's own (MCC's `halo2\h2_fonts`, or Vista's
//! `maps\fonts`), read with `blam_cache::font` and packed here into one
//! atlas per font. As the lobby found on the real fonts, a glyph's picture
//! goes its origin x right of the pen and the next glyph its advance past
//! the picture's left edge (counting the advance from the pen runs an L's
//! foot into a following I). Kerning pairs are in the font's pixels.
//!
//! How big each font slot's text is in UI units isn't read from anything:
//! each slot has a size (an estimate, about the line height of Vista's
//! font in it, as menu-preview kept), and a font is scaled so its capitals
//! are `CAP_SHARE` of that, as the lobby scales them. A slot whose font is
//! missing borrows another slot's, and a character no font has comes from
//! a small built-in font (5 by 7 pixels, capitals only), as does all text
//! when no Halo 2 font is loaded. The controller's buttons (U+E100 to
//! U+E105) fall back to a letter on a coloured disc.

use crate::art::ImageRef;
use crate::paint::Texture;
use crate::Font;
use blam_cache::font::FontFile;
use std::path::Path;

/// Where a line sits across its box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Justify {
    Left,
    #[default]
    Center,
    Right,
}

/// A font's capitals are this share of its text size.
pub const CAP_SHARE: f32 = 0.65;

/// A slot's text size in UI units. Estimates (menu-preview's, about the
/// line height of Vista's font in each slot at a unit to its pixel).
pub fn size(font: Font) -> f32 {
    match font {
        Font::SuperLarge => 48.0,
        Font::Title => 36.0,
        Font::MainMenu => 30.0,
        Font::LargeBody => 26.0,
        Font::FullHudMessage => 24.0,
        Font::Body | Font::EnglishBody | Font::HudNumber => 22.0,
        Font::SplitHudMessage | Font::Terminal | Font::TextChat => 20.0,
        Font::Subtitle => 18.0,
    }
}

/// The characters taken from Halo 2's fonts: Latin, punctuation and the
/// like, and the controller's buttons (the rest of their code points are
/// icons and other scripts).
pub fn wanted(c: char) -> bool {
    (c as u32) < 0x2200 || button(c).is_some()
}

/// A controller button in the fonts' private code points, and how the
/// fallback draws it: a disc in its colour with its letter in `ink`.
struct Button {
    code: char,
    disc: [f32; 3],
    letter: Option<char>,
    ink: [f32; 3],
}

const fn b(code: char, disc: [f32; 3], letter: Option<char>, ink: [f32; 3]) -> Button {
    Button {
        code,
        disc,
        letter,
        ink,
    }
}

/// A, B, X, Y, Black and White.
const BUTTONS: [Button; 6] = [
    b('\u{e100}', [0.25, 0.7, 0.2], Some('A'), [1.0, 1.0, 1.0]),
    b('\u{e101}', [0.8, 0.15, 0.1], Some('B'), [1.0, 1.0, 1.0]),
    b('\u{e102}', [0.15, 0.35, 0.85], Some('X'), [1.0, 1.0, 1.0]),
    b('\u{e103}', [0.9, 0.75, 0.1], Some('Y'), [0.1, 0.1, 0.1]),
    b('\u{e104}', [0.1, 0.1, 0.1], None, [1.0; 3]),
    b('\u{e105}', [0.92, 0.92, 0.92], None, [0.1; 3]),
];

fn button(c: char) -> Option<&'static Button> {
    BUTTONS.iter().find(|b| b.code == c)
}

/// A glyph placed in a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedGlyph {
    pub texture: Texture,
    /// Its part of the texture (left, top, right, bottom, 0 to 1).
    pub uv: [f32; 4],
    /// Where it goes from the line's start on its baseline (left, top,
    /// right, bottom; UI units, +y down).
    pub rect: [f32; 4],
    /// Its own colour instead of the text's (a button's disc and letter).
    pub color: Option<[f32; 3]>,
}

/// A line laid out: its glyphs, how wide it is, and its font's ascent,
/// descent and capitals' height (UI units).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub glyphs: Vec<PlacedGlyph>,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub caps: f32,
}

/// A glyph in an atlas.
#[derive(Clone, Copy, Debug, PartialEq)]
struct AtlasGlyph {
    code: char,
    uv: [f32; 4],
    /// Its picture's size, and where the pen is in it (font pixels).
    size: [f32; 2],
    origin: [f32; 2],
    advance: f32,
}

/// A font's glyphs packed into one RGBA image.
#[derive(Clone, Debug, PartialEq)]
struct Atlas {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
    glyphs: Vec<AtlasGlyph>,
    kerning: Vec<(u8, u8, i16)>,
    ascent: f32,
    descent: f32,
    /// Its capitals' height, in its pixels.
    caps: f32,
}

/// Glyphs go this far apart in an atlas, and an atlas is this wide.
const ATLAS_PAD: usize = 2;
const ATLAS_W: usize = 1024;

/// A glyph's picture size in an atlas: none for one that doesn't fit
/// across it, or is as tall (only a broken font file has one), so the
/// glyph keeps its advance but draws nothing.
fn packed_size(g: &blam_cache::font::Glyph) -> (usize, usize) {
    let (w, h) = (usize::from(g.width), usize::from(g.height));
    if w > ATLAS_W - 2 * ATLAS_PAD || h > ATLAS_W {
        return (0, 0);
    }
    (w, h)
}

impl Atlas {
    fn pack(font: &FontFile) -> Atlas {
        let (mut x, mut y, mut row) = (ATLAS_PAD, ATLAS_PAD, 0);
        let mut places = Vec::with_capacity(font.glyphs.len());
        for g in &font.glyphs {
            let (w, h) = packed_size(g);
            if x + w + ATLAS_PAD > ATLAS_W {
                (x, y, row) = (ATLAS_PAD, y + row + ATLAS_PAD, 0);
            }
            places.push((x, y));
            x += w + ATLAS_PAD;
            row = row.max(h);
        }
        let height = y + row + ATLAS_PAD;
        let mut rgba = vec![0u8; ATLAS_W * height * 4];
        let mut glyphs = Vec::with_capacity(font.glyphs.len());
        for (g, &(x, y)) in font.glyphs.iter().zip(&places) {
            let (w, h) = packed_size(g);
            if w > 0 && h > 0 && g.rgba.len() >= w * h * 4 {
                for (row, line) in g.rgba.chunks_exact(w * 4).take(h).enumerate() {
                    let at = ((y + row) * ATLAS_W + x) * 4;
                    rgba[at..at + line.len()].copy_from_slice(line);
                }
            }
            let (aw, ah) = (ATLAS_W as f32, height as f32);
            glyphs.push(AtlasGlyph {
                code: g.code,
                uv: [
                    x as f32 / aw,
                    y as f32 / ah,
                    (x + w) as f32 / aw,
                    (y + h) as f32 / ah,
                ],
                size: [w as f32, h as f32],
                origin: [f32::from(g.origin[0]), f32::from(g.origin[1])],
                advance: f32::from(g.advance),
            });
        }
        Atlas {
            width: ATLAS_W,
            height,
            rgba,
            glyphs,
            kerning: font.kerning.clone(),
            ascent: f32::from(font.ascent.max(1)),
            descent: f32::from(font.descent.max(0)),
            caps: cap_height(font),
        }
    }

    fn glyph(&self, c: char) -> Option<&AtlasGlyph> {
        self.glyphs
            .binary_search_by_key(&c, |g| g.code)
            .ok()
            .map(|k| &self.glyphs[k])
    }

    fn kern(&self, first: char, second: char) -> f32 {
        let (Ok(a), Ok(b)) = (u8::try_from(first), u8::try_from(second)) else {
            return 0.0;
        };
        self.kerning
            .iter()
            .find(|k| k.0 == a && k.1 == b)
            .map_or(0.0, |k| f32::from(k.2))
    }
}

/// How tall a font's capitals are, in its pixels: from the top of an H
/// (or another capital) to the baseline; most of its ascent without one.
fn cap_height(f: &FontFile) -> f32 {
    let fallback = f32::from(f.ascent.max(1)) * 0.7;
    let Some(g) = ['H', 'E', 'I', 'T'].iter().find_map(|&c| f.glyph(c)) else {
        return fallback;
    };
    let w = usize::from(g.width).max(1);
    let top = g
        .rgba
        .chunks(4 * w)
        .position(|row| row.chunks(4).any(|p| p[3] >= 128));
    match top {
        Some(top) if (top as f32) < f32::from(g.origin[1]) => f32::from(g.origin[1]) - top as f32,
        _ => fallback,
    }
}

/// The built-in font: 5 by 7 pixel capitals, digits and punctuation (ours,
/// as h2viewer's HUD font), drawn at `FALLBACK_TEXELS` texels a pixel so
/// it stays sharp scaled up.
const FALLBACK_CHARS: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ/:.-x <>'!?,()+_#%&\";=*[]@";
#[rustfmt::skip]
const FALLBACK_GLYPHS: [[u8; 7]; 62] = [
    [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E], // 0
    [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E], // 1
    [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F], // 2
    [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E], // 3
    [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02], // 4
    [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E], // 5
    [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E], // 6
    [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08], // 7
    [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E], // 8
    [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C], // 9
    [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11], // A
    [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E], // B
    [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E], // C
    [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C], // D
    [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F], // E
    [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10], // F
    [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F], // G
    [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11], // H
    [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E], // I
    [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C], // J
    [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11], // K
    [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F], // L
    [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11], // M
    [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11], // N
    [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E], // O
    [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10], // P
    [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D], // Q
    [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11], // R
    [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E], // S
    [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04], // T
    [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E], // U
    [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04], // V
    [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A], // W
    [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11], // X
    [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04], // Y
    [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F], // Z
    [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10], // /
    [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00], // :
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C], // .
    [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00], // -
    [0x00, 0x00, 0x11, 0x0A, 0x04, 0x0A, 0x11], // x (times)
    [0x00; 7],                                  // space
    [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02], // <
    [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08], // >
    [0x04, 0x04, 0x08, 0x00, 0x00, 0x00, 0x00], // '
    [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04], // !
    [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04], // ?
    [0x00, 0x00, 0x00, 0x00, 0x0C, 0x04, 0x08], // ,
    [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02], // (
    [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08], // )
    [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00], // +
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F], // _
    [0x0A, 0x0A, 0x1F, 0x0A, 0x1F, 0x0A, 0x0A], // #
    [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03], // %
    [0x0C, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0D], // &
    [0x0A, 0x0A, 0x0A, 0x00, 0x00, 0x00, 0x00], // "
    [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x04, 0x08], // ;
    [0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00], // =
    [0x00, 0x04, 0x15, 0x0E, 0x15, 0x04, 0x00], // *
    [0x0E, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0E], // [
    [0x0E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0E], // ]
    [0x0E, 0x11, 0x17, 0x15, 0x17, 0x10, 0x0E], // @
];
const FALLBACK_TEXELS: usize = 4;
/// A glyph's cell in the atlas: the glyph with a pixel clear around it.
const FALLBACK_CELL: [usize; 2] = [7 * FALLBACK_TEXELS, 9 * FALLBACK_TEXELS];
/// The disc behind a button's letter, after the glyphs.
const DISC: usize = 9 * FALLBACK_TEXELS;
/// The built-in font's metrics in its pixels: a glyph's advance (its 5
/// columns and a gap), its capitals, and the ascent and descent a line
/// keeps.
const FALLBACK_ADVANCE: f32 = 6.0;
const FALLBACK_CAPS: f32 = 7.0;
const FALLBACK_ASCENT: f32 = 8.0;
const FALLBACK_DESCENT: f32 = 2.0;

/// The built-in font's atlas.
#[derive(Clone, Debug, PartialEq)]
struct Fallback {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Fallback {
    fn new() -> Fallback {
        let n = FALLBACK_CHARS.chars().count();
        let (cw, ch) = (FALLBACK_CELL[0], FALLBACK_CELL[1]);
        let width = n * cw + DISC;
        let height = ch.max(DISC);
        let mut rgba = vec![0u8; width * height * 4];
        let mut set = |x: usize, y: usize, a: u8| {
            rgba[(y * width + x) * 4..][..4].copy_from_slice(&[255, 255, 255, a]);
        };
        for (g, rows) in FALLBACK_GLYPHS.iter().enumerate() {
            for (y, row) in rows.iter().enumerate() {
                for x in (0..5).filter(|x| row & (0x10 >> x) != 0) {
                    for dy in 0..FALLBACK_TEXELS {
                        for dx in 0..FALLBACK_TEXELS {
                            let px = g * cw + (x + 1) * FALLBACK_TEXELS + dx;
                            set(px, (y + 1) * FALLBACK_TEXELS + dy, 255);
                        }
                    }
                }
            }
        }
        // The disc, with a soft edge.
        let r = DISC as f32 * 0.5;
        for y in 0..DISC {
            for x in 0..DISC {
                let d = ((x as f32 + 0.5 - r).powi(2) + (y as f32 + 0.5 - r).powi(2)).sqrt();
                let cover = (r - 0.5 - d + 0.5).clamp(0.0, 1.0);
                if cover > 0.0 {
                    set(n * cw + x, y, (cover * 255.0).round() as u8);
                }
            }
        }
        Fallback {
            width,
            height,
            rgba,
        }
    }

    /// A character's glyph's part of the atlas (capitals for small
    /// letters, a question mark for what it lacks).
    fn uv(&self, c: char) -> [f32; 4] {
        let c = if c.is_ascii_lowercase() && c != 'x' {
            c.to_ascii_uppercase()
        } else {
            c
        };
        let k = FALLBACK_CHARS
            .chars()
            .position(|f| f == c)
            .or_else(|| FALLBACK_CHARS.chars().position(|f| f == '?'))
            .unwrap_or(0);
        let x = (k * FALLBACK_CELL[0] + FALLBACK_TEXELS) as f32;
        let y = FALLBACK_TEXELS as f32;
        let (w, h) = (self.width as f32, self.height as f32);
        let t = FALLBACK_TEXELS as f32;
        [x / w, y / h, (x + 5.0 * t) / w, (y + 7.0 * t) / h]
    }

    fn disc_uv(&self) -> [f32; 4] {
        let x = (self.width - DISC) as f32 / self.width as f32;
        [x, 0.0, 1.0, DISC as f32 / self.height as f32]
    }
}

/// The menus' fonts.
#[derive(Clone, Debug)]
pub struct Fonts {
    atlases: Vec<Atlas>,
    /// Each slot's atlas and UI units to its pixels, by `Font::index`.
    slots: Vec<Option<(usize, f32)>>,
    fallback: Fallback,
}

impl Default for Fonts {
    fn default() -> Fonts {
        Fonts::fallback()
    }
}

/// Slots tried, in order, for a slot without its own font.
const BORROW: [Font; 4] = [Font::MainMenu, Font::Body, Font::LargeBody, Font::Title];

impl Fonts {
    /// The built-in font alone.
    pub fn fallback() -> Fonts {
        Fonts {
            atlases: Vec::new(),
            slots: vec![None; Font::ALL.len()],
            fallback: Fallback::new(),
        }
    }

    /// Halo 2's fonts, by slot (`Font::index`; as `blam_cache::font::
    /// read_table` gives them). Slots sharing a font share its atlas.
    pub fn new(files: &[Option<FontFile>]) -> Fonts {
        let mut fonts = Fonts::fallback();
        let mut seen: Vec<&FontFile> = Vec::new();
        for (slot, file) in files.iter().enumerate().take(Font::ALL.len()) {
            let Some(file) = file.as_ref().filter(|f| !f.glyphs.is_empty()) else {
                continue;
            };
            let k = match seen.iter().position(|&f| f == file) {
                Some(k) => k,
                None => {
                    seen.push(file);
                    fonts.atlases.push(Atlas::pack(file));
                    fonts.atlases.len() - 1
                }
            };
            let caps = fonts.atlases[k].caps.max(1.0);
            fonts.slots[slot] = Some((k, size(Font::ALL[slot]) * CAP_SHARE / caps));
        }
        fonts
    }

    /// Halo 2's fonts from a fonts folder (with its `font_table.txt`);
    /// the built-in font alone when none read.
    pub fn read(dir: &Path) -> Fonts {
        Fonts::new(&blam_cache::font::read_table(dir, wanted))
    }

    /// Whether `font`'s slot has a Halo 2 font of its own.
    pub fn has(&self, font: Font) -> bool {
        self.slots.get(font.index()).is_some_and(Option::is_some)
    }

    /// Draws `font`'s slot at `k` UI units to its font's pixels, instead
    /// of scaled to its size (once the real scale is known).
    pub fn set_units_per_pixel(&mut self, font: Font, k: f32) {
        if let Some(Some(slot)) = self.slots.get_mut(font.index()) {
            slot.1 = k;
        }
    }

    /// The Halo 2 font `font` is drawn in, and its UI units to a pixel,
    /// at `scale` times its size: its own, or a borrowed one scaled to its
    /// size.
    fn halo(&self, font: Font, scale: f32) -> Option<(usize, &Atlas, f32)> {
        if let Some(&Some((k, units))) = self.slots.get(font.index()) {
            return Some((k, &self.atlases[k], units * scale));
        }
        let (k, _) = BORROW
            .iter()
            .find_map(|f| self.slots.get(f.index()).copied().flatten())?;
        let atlas = &self.atlases[k];
        Some((
            k,
            atlas,
            size(font) * scale * CAP_SHARE / atlas.caps.max(1.0),
        ))
    }

    /// `text` in `font` at `scale` times its size, from its start on its
    /// baseline.
    pub fn line(&self, font: Font, scale: f32, text: &str) -> Line {
        let halo = self.halo(font, scale);
        // The capitals' height, and the built-in font's UI units to its
        // pixel, its capitals as tall.
        let caps = match halo {
            Some((_, a, k)) => a.caps * k,
            None => size(font) * scale * CAP_SHARE,
        };
        let fk = caps / FALLBACK_CAPS;
        let (ascent, descent) = match halo {
            Some((_, a, k)) => (a.ascent * k, a.descent * k),
            None => (FALLBACK_ASCENT * fk, FALLBACK_DESCENT * fk),
        };
        let mut line = Line {
            glyphs: Vec::with_capacity(text.len()),
            width: 0.0,
            ascent,
            descent,
            caps,
        };
        let mut pen = 0.0;
        let mut last: Option<char> = None;
        for c in text.chars() {
            if let Some((k, atlas, units)) = halo {
                if let Some(g) = atlas.glyph(c) {
                    pen += last.map_or(0.0, |l| atlas.kern(l, c)) * units;
                    let left = pen + g.origin[0] * units;
                    let top = -g.origin[1] * units;
                    if g.size[0] > 0.0 && g.size[1] > 0.0 {
                        line.glyphs.push(PlacedGlyph {
                            texture: Texture::Font(k as u16),
                            uv: g.uv,
                            rect: [left, top, left + g.size[0] * units, top + g.size[1] * units],
                            color: None,
                        });
                    }
                    pen = left + g.advance * units;
                    last = Some(c);
                    continue;
                }
            }
            last = None;
            if let Some(&Button {
                disc, letter, ink, ..
            }) = button(c)
            {
                pen = self.button_glyph(&mut line.glyphs, pen, caps, disc, letter, ink);
                continue;
            }
            if c != ' ' {
                let uv = self.fallback.uv(c);
                let left = pen;
                line.glyphs.push(PlacedGlyph {
                    texture: Texture::Fallback,
                    uv,
                    rect: [left, -FALLBACK_CAPS * fk, left + 5.0 * fk, 0.0],
                    color: None,
                });
            }
            pen += FALLBACK_ADVANCE * fk;
        }
        line.width = pen;
        line
    }

    /// A button as a disc with its letter, as tall as the capitals and a
    /// little more, from `pen`; where the pen goes next.
    fn button_glyph(
        &self,
        out: &mut Vec<PlacedGlyph>,
        pen: f32,
        caps: f32,
        disc: [f32; 3],
        letter: Option<char>,
        ink: [f32; 3],
    ) -> f32 {
        let d = caps * 1.3;
        let top = -caps * 0.5 - d * 0.5;
        out.push(PlacedGlyph {
            texture: Texture::Fallback,
            uv: self.fallback.disc_uv(),
            rect: [pen, top, pen + d, top + d],
            color: Some(disc),
        });
        if let Some(l) = letter {
            // The letter's capitals 0.55 of the disc, centred in it.
            let k = d * 0.55 / FALLBACK_CAPS;
            let (w, h) = (5.0 * k, FALLBACK_CAPS * k);
            let (x, y) = (pen + (d - w) * 0.5, top + (d - h) * 0.5);
            out.push(PlacedGlyph {
                texture: Texture::Fallback,
                uv: self.fallback.uv(l),
                rect: [x, y, x + w, y + h],
                color: Some(ink),
            });
        }
        pen + d + caps * 0.25
    }

    /// How wide `text` is in `font` at its size (UI units).
    pub fn width(&self, font: Font, text: &str) -> f32 {
        self.line(font, 1.0, text).width
    }

    /// `text` cut to fit `room` UI units, with "..." where it was cut.
    pub fn fit(&self, font: Font, text: &str, room: f32) -> String {
        if self.width(font, text) <= room {
            return text.to_string();
        }
        let mut kept: Vec<char> = text.chars().collect();
        while !kept.is_empty() {
            kept.pop();
            let cut = format!("{}...", kept.iter().collect::<String>().trim_end());
            if self.width(font, &cut) <= room {
                return cut;
            }
        }
        String::new()
    }

    /// The pixels behind a font texture.
    pub fn image(&self, texture: Texture) -> Option<ImageRef<'_>> {
        match texture {
            Texture::Font(k) => self.atlases.get(k as usize).map(|a| ImageRef {
                width: a.width,
                height: a.height,
                rgba: &a.rgba,
            }),
            Texture::Fallback => Some(ImageRef {
                width: self.fallback.width,
                height: self.fallback.height,
                rgba: &self.fallback.rgba,
            }),
            _ => None,
        }
    }

    /// Which slots have a Halo 2 font of their own, by name.
    pub fn loaded(&self) -> Vec<&'static str> {
        Font::ALL
            .iter()
            .filter(|f| self.has(**f))
            .map(|f| f.name())
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use blam_cache::font::Glyph;

    /// A glyph whose picture is a solid block `w` by `h`, its left edge
    /// `left` right of the pen, its baseline `base` down from its top, and
    /// the next glyph `advance` past its left edge.
    pub(crate) fn glyph(code: char, w: u16, h: u16, left: i16, base: i16, advance: u16) -> Glyph {
        Glyph {
            code,
            advance,
            width: w,
            height: h,
            origin: [left, base],
            rgba: vec![255; w as usize * h as usize * 4],
        }
    }

    /// A made-up font: an L whose foot reaches its whole width, an I, an H
    /// and a space, capitals 10 pixels high, with A and V kerned closer.
    pub(crate) fn font() -> FontFile {
        FontFile {
            ascent: 12,
            descent: 3,
            glyphs: vec![
                glyph(' ', 0, 0, 0, 0, 5),
                glyph('A', 6, 10, 1, 10, 6),
                glyph('H', 6, 10, 1, 10, 7),
                glyph('I', 2, 10, 2, 10, 3),
                glyph('L', 6, 10, 2, 10, 6),
                glyph('V', 6, 10, 1, 10, 6),
            ],
            kerning: vec![(b'A', b'V', -2)],
        }
    }

    /// Fonts with `font()` in the main menu slot, a pixel to a UI unit.
    pub(crate) fn fonts() -> Fonts {
        let mut files = vec![None; 12];
        files[Font::MainMenu.index()] = Some(font());
        let mut f = Fonts::new(&files);
        f.set_units_per_pixel(Font::MainMenu, 1.0);
        f
    }

    #[test]
    fn an_l_then_an_i_dont_touch() {
        let f = fonts();
        let line = f.line(Font::MainMenu, 1.0, "LI");
        let [l, i] = [line.glyphs[0].rect, line.glyphs[1].rect];
        // L from 2 to 8; the pen moves 6 past its left edge (to 8), and
        // the I starts 2 right of that.
        assert_eq!(l, [2.0, -10.0, 8.0, 0.0]);
        assert_eq!(i, [10.0, -10.0, 12.0, 0.0]);
        assert!(i[0] > l[2], "the I's left edge must clear the L's foot");
        assert_eq!(line.width, 13.0);
        assert_eq!((line.ascent, line.descent, line.caps), (12.0, 3.0, 10.0));
        // Scaled twice as big, everything doubles.
        let big = f.line(Font::MainMenu, 2.0, "LI");
        assert_eq!(big.glyphs[1].rect, [20.0, -20.0, 24.0, 0.0]);
        assert_eq!(big.width, 26.0);
    }

    #[test]
    fn a_glyph_too_big_for_the_atlas_keeps_its_advance_only() {
        let mut file = font();
        // After the space, in code point order.
        file.glyphs.insert(1, glyph('$', 4000, 1, 0, 1, 9));
        file.glyphs.insert(2, glyph('%', 1021, 2, 0, 2, 4));
        file.glyphs.insert(3, glyph('*', 3, 1100, 0, 2, 5));
        let atlas = Atlas::pack(&file);
        assert!(atlas.height < 64, "{}", atlas.height);
        let mut files = vec![None; 12];
        files[Font::MainMenu.index()] = Some(file);
        let mut f = Fonts::new(&files);
        f.set_units_per_pixel(Font::MainMenu, 1.0);
        let line = f.line(Font::MainMenu, 1.0, "$%*I");
        assert_eq!(line.glyphs.len(), 1);
        assert_eq!(line.glyphs[0].rect[0], 9.0 + 4.0 + 5.0 + 2.0);
    }

    #[test]
    fn kerning_and_spaces() {
        let f = fonts();
        let av = f.line(Font::MainMenu, 1.0, "AV");
        // A at 1..7, pen at 7, kerned back 2 to 5, V's left edge at 6.
        assert_eq!(av.glyphs[1].rect[0], 6.0);
        assert_eq!(av.width, 12.0);
        // A space has no picture, only its advance.
        let spaced = f.line(Font::MainMenu, 1.0, "I I");
        assert_eq!(spaced.glyphs.len(), 2);
        assert_eq!(spaced.glyphs[1].rect[0], 2.0 + 3.0 + 5.0 + 2.0);
    }

    #[test]
    fn fonts_scale_their_capitals_to_the_slots_size() {
        let mut files = vec![None; 12];
        files[Font::MainMenu.index()] = Some(font());
        files[Font::Title.index()] = Some(font());
        let f = Fonts::new(&files);
        // The same font in two slots is packed once.
        assert_eq!(f.atlases.len(), 1);
        assert!(f.has(Font::Title) && !f.has(Font::Body));
        assert_eq!(f.loaded(), ["title", "main_menu"]);
        // An H's capitals (10 pixels) become 0.65 of the slot's size.
        let h = f.line(Font::MainMenu, 1.0, "H").glyphs[0].rect;
        assert!((h[3] - h[1] - 30.0 * 0.65).abs() < 1e-4);
        let h = f.line(Font::Title, 1.0, "H").glyphs[0].rect;
        assert!((h[3] - h[1] - 36.0 * 0.65).abs() < 1e-4);
        // A slot without a font borrows the main menu's, at its own size.
        let h = f.line(Font::Body, 1.0, "H").glyphs[0];
        assert_eq!(h.texture, Texture::Font(0));
        assert!((h.rect[3] - h.rect[1] - 22.0 * 0.65).abs() < 1e-4);
        // The atlas holds the glyphs' pixels where their uvs say.
        let img = f.image(Texture::Font(0)).unwrap();
        let g = f.atlases[0].glyph('H').unwrap();
        let (x, y) = (
            (g.uv[0] * img.width as f32).round() as usize,
            (g.uv[1] * img.height as f32).round() as usize,
        );
        assert_eq!(img.rgba[(y * img.width + x) * 4 + 3], 255);
        assert_eq!(img.rgba[((y - 1) * img.width + x) * 4 + 3], 0);
    }

    #[test]
    fn without_halo_2s_fonts_the_built_in_font_is_used() {
        let f = Fonts::fallback();
        assert!(f.loaded().is_empty());
        let line = f.line(Font::MainMenu, 1.0, "Ab ?");
        let k = 30.0 * 0.65 / 7.0;
        assert_eq!(line.glyphs.len(), 3);
        assert!(line.glyphs.iter().all(|g| g.texture == Texture::Fallback));
        assert!((line.width - 4.0 * 6.0 * k).abs() < 1e-4);
        // Small letters are capitals, and capitals are the slot's height.
        assert_eq!(line.glyphs[1].uv, f.fallback.uv('B'));
        assert!((line.glyphs[0].rect[1] + 30.0 * 0.65).abs() < 1e-4);
        // A character it lacks is a question mark.
        assert_eq!(f.fallback.uv('~'), f.fallback.uv('?'));
        assert_eq!(FALLBACK_CHARS.chars().count(), FALLBACK_GLYPHS.len());
        // Its atlas has ink, and the uvs point at it.
        let img = f.image(Texture::Fallback).unwrap();
        assert!(img.is_whole());
        let uv = f.fallback.uv('I');
        let x = ((uv[0] + uv[2]) * 0.5 * img.width as f32) as usize;
        let y = (uv[1] * img.height as f32) as usize + 1;
        assert_eq!(img.rgba[(y * img.width + x) * 4 + 3], 255);
        assert!(f.image(Texture::White).is_none());
    }

    #[test]
    fn a_missing_glyph_comes_from_the_built_in_font_at_the_same_size() {
        let f = fonts();
        let line = f.line(Font::MainMenu, 1.0, "IZ");
        assert_eq!(line.glyphs[1].texture, Texture::Fallback);
        // As tall as the font's capitals (10 pixels, a unit each here).
        let z = line.glyphs[1].rect;
        assert!((z[3] - z[1] - 10.0).abs() < 1e-4);
        // It starts where the I's advance left the pen.
        assert_eq!(z[0], 5.0);
    }

    #[test]
    fn buttons_without_a_glyph_are_letters_on_discs() {
        let f = Fonts::fallback();
        let line = f.line(Font::Body, 1.0, "\u{e100} SELECT");
        let (disc, letter) = (line.glyphs[0], line.glyphs[1]);
        assert_eq!(disc.uv, f.fallback.disc_uv());
        assert_eq!(disc.color, Some([0.25, 0.7, 0.2]));
        assert_eq!(letter.uv, f.fallback.uv('A'));
        // The letter sits inside the disc.
        assert!(letter.rect[0] > disc.rect[0] && letter.rect[2] < disc.rect[2]);
        assert!(letter.rect[1] > disc.rect[1] && letter.rect[3] < disc.rect[3]);
        // The text after it follows the disc.
        assert!(line.glyphs[2].rect[0] > disc.rect[2]);
        assert!(wanted('\u{e103}') && wanted('e') && !wanted('\u{e200}'));
    }

    #[test]
    fn long_text_is_cut_to_fit() {
        let f = Fonts::fallback();
        let w = f.width(Font::Body, "ABCDEFGHIJ");
        assert_eq!(f.fit(Font::Body, "ABCDEFGHIJ", w), "ABCDEFGHIJ");
        let cut = f.fit(Font::Body, "ABCDEFGHIJ", w * 0.6);
        assert!(cut.ends_with("...") && f.width(Font::Body, &cut) <= w * 0.6);
        assert_eq!(f.fit(Font::Body, "ABC", 0.0), "");
    }
}
