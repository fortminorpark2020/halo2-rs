//! Halo 2 Vista's fonts. They aren't in the maps: `maps\fonts\` holds
//! `font_table.txt`, naming a file for each of the engine's twelve font
//! slots in order, and those extensionless font files.
//!
//! A font file, after a 0x200 byte header the game doesn't read:
//! - its version (0xF0000001), then i16 ascent, descent, lead height and
//!   lead width, the character count and four sizes (i32), and kerning
//!   pairs (a count, then each pair's two characters as bytes and an i16
//!   adjustment), then eight i32s;
//! - at 0x400, each of the 65,536 code points' character index (a code
//!   point with code point 0's index, other than 0 itself, has no glyph);
//! - at 0x40400, each character's record of 16 bytes: its advance, its
//!   data's length, its width and height, its origin (the pen's place in
//!   its image, x and y) and its data's offset in the file;
//! - each glyph's pixels, ARGB4444, in a small run-length code (`decode`).
//!
//! The layout follows the notes of the community's font tools
//! (FontPackager) on Halo 2's loose fonts.

use crate::{i16_at, u32_at, Error, Result};
use std::path::Path;

/// The engine's font slots, in `font_table.txt`'s order (the menus name
/// them by index).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Font {
    Terminal,
    #[default]
    Body,
    Title,
    SuperLarge,
    LargeBody,
    SplitHudMessage,
    FullHudMessage,
    EnglishBody,
    HudNumber,
    Subtitle,
    MainMenu,
    TextChat,
}

impl Font {
    pub const ALL: [Font; 12] = [
        Font::Terminal,
        Font::Body,
        Font::Title,
        Font::SuperLarge,
        Font::LargeBody,
        Font::SplitHudMessage,
        Font::FullHudMessage,
        Font::EnglishBody,
        Font::HudNumber,
        Font::Subtitle,
        Font::MainMenu,
        Font::TextChat,
    ];

    pub fn from_index(index: u16) -> Option<Font> {
        Font::ALL.get(index as usize).copied()
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// The slot's name, as the menu tags' font enum names it.
    pub fn name(self) -> &'static str {
        match self {
            Font::Terminal => "terminal",
            Font::Body => "body",
            Font::Title => "title",
            Font::SuperLarge => "super_large",
            Font::LargeBody => "large_body",
            Font::SplitHudMessage => "split_hud_msg",
            Font::FullHudMessage => "full_hud_msg",
            Font::EnglishBody => "english_body",
            Font::HudNumber => "hud_number",
            Font::Subtitle => "subtitle",
            Font::MainMenu => "main_menu",
            Font::TextChat => "text_chat",
        }
    }
}

const VERSION: u32 = 0xF000_0001;
const HEADER: usize = 0x200;
const KERNING: usize = 0x220;
const CODE_POINTS: usize = 0x400;
const CHARACTERS: usize = 0x40400;
const CHARACTER_SIZE: usize = 0x10;
/// More characters than any font has: a count past this is corrupt.
const MAX_CHARACTERS: usize = 0x10000;

/// A character's picture, RGBA8.
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    pub code: char,
    /// How far the pen moves past it.
    pub advance: u16,
    pub width: u16,
    pub height: u16,
    /// Where its picture goes from the pen: x, how far right of the pen
    /// its left edge is (less than 0 for a glyph reaching back under the
    /// one before, like j); y, how far down from its top the baseline is.
    pub origin: [i16; 2],
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontFile {
    /// Pixels above and below the baseline.
    pub ascent: i16,
    pub descent: i16,
    /// The glyphs of the code points asked for, by code point.
    pub glyphs: Vec<Glyph>,
    /// Pairs of characters (first, then second) drawn closer or further
    /// apart, by this many pixels.
    pub kerning: Vec<(u8, u8, i16)>,
}

impl FontFile {
    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs
            .binary_search_by_key(&c, |g| g.code)
            .ok()
            .map(|k| &self.glyphs[k])
    }

    /// The kerning between two characters drawn one after the other.
    pub fn kern(&self, first: char, second: char) -> i16 {
        let (Ok(a), Ok(b)) = (u8::try_from(first), u8::try_from(second)) else {
            return 0;
        };
        self.kerning
            .iter()
            .find(|k| k.0 == a && k.1 == b)
            .map_or(0, |k| k.2)
    }
}

/// Parse a font file, keeping the glyphs of the code points `wanted` says
/// to (decoding all of a big font's would be slow and useless).
pub fn parse(b: &[u8], wanted: impl Fn(char) -> bool) -> Result<FontFile> {
    let short = || Error::Corrupt("font file too short".into());
    if b.len() < CHARACTERS || u32_at(b, HEADER) != VERSION {
        return Err(Error::Corrupt("not a Halo 2 font file".into()));
    }
    let count = u32_at(b, 0x20C) as usize;
    if count == 0 || count > MAX_CHARACTERS {
        return Err(Error::Corrupt(format!("font has {count} characters")));
    }
    let pairs = (u32_at(b, KERNING) as usize).min((CODE_POINTS - KERNING - 4 - 32) / 4);
    let kerning = (0..pairs)
        .map(|k| {
            let at = KERNING + 4 + 4 * k;
            (b[at], b[at + 1], i16_at(b, at + 2))
        })
        .collect();
    let index = |code: usize| u32_at(b, CODE_POINTS + 4 * code) as usize;
    let none = index(0);
    let mut glyphs = Vec::new();
    for code in 0..0x10000usize {
        let k = index(code);
        if code != 0 && k == none || k >= count {
            continue;
        }
        let Some(c) = char::from_u32(code as u32).filter(|&c| wanted(c)) else {
            continue;
        };
        let r = CHARACTERS + CHARACTER_SIZE * k;
        let record = b.get(r..r + CHARACTER_SIZE).ok_or_else(short)?;
        let len = u16::from_le_bytes([record[2], record[3]]) as usize;
        let at = u32_at(record, 0xC) as usize;
        let data = b.get(at..at + len).ok_or_else(short)?;
        let width = u16::from_le_bytes([record[4], record[5]]);
        let height = u16::from_le_bytes([record[6], record[7]]);
        glyphs.push(Glyph {
            code: c,
            advance: u16::from_le_bytes([record[0], record[1]]),
            width,
            height,
            origin: [i16_at(record, 8), i16_at(record, 0xA)],
            rgba: decode(data, width as usize * height as usize),
        });
    }
    Ok(FontFile {
        ascent: i16_at(b, 0x204),
        descent: i16_at(b, 0x206),
        glyphs,
        kerning,
    })
}

/// A 4 bit value as 8.
fn nibble(v: u16) -> u8 {
    (v & 0xF) as u8 * 17
}

/// Decode `pixels` pixels of a glyph to RGBA8. Pixels are ARGB4444; the
/// code keeps a current colour (RGB, white at first) and each byte is:
/// - 0: a whole pixel follows (two bytes, big-endian), which also becomes
///   the current colour;
/// - 0b00nnnnnn: n transparent pixels of the current colour;
/// - 0b01nnnnnn: n opaque ones;
/// - 0b11aaabbb: two pixels of the current colour, with 3 bit alphas a and
///   b (each widened to 4 bits by repeating its low bit);
/// - 0b10aaa000: one such pixel; 0b10aaabbb with b > 0: one, then a run of
///   5 - (b & 3) more, transparent if b & 4 else opaque.
pub fn decode(data: &[u8], pixels: usize) -> Vec<u8> {
    let mut out: Vec<u16> = Vec::with_capacity(pixels);
    let mut base: u16 = 0xFFF;
    let alpha3 = |v: u8| -> u16 {
        let v = (v & 7) as u16;
        ((v << 1) | (v & 1)) << 12
    };
    let mut i = 0;
    while i < data.len() && out.len() < pixels {
        let code = data[i];
        i += 1;
        let (kind, n) = (code >> 6, (code & 0x3F) as usize);
        match kind {
            0 if code == 0 => {
                let Some(px) = data.get(i..i + 2) else { break };
                let px = u16::from_be_bytes([px[0], px[1]]);
                i += 2;
                out.push(px);
                base = px & 0xFFF;
            }
            0 => out.extend(std::iter::repeat_n(base, n)),
            1 => out.extend(std::iter::repeat_n(base | 0xF000, n)),
            _ => {
                out.push(base | alpha3(code >> 3));
                let low = code & 7;
                if kind == 3 {
                    out.push(base | alpha3(low));
                } else if low != 0 {
                    let alpha = if low & 4 == 0 { 0xF000 } else { 0 };
                    let run = 5 - (low & 3) as usize;
                    out.extend(std::iter::repeat_n(base | alpha, run));
                }
            }
        }
    }
    out.resize(pixels, 0);
    out.iter()
        .flat_map(|&p| [nibble(p >> 8), nibble(p >> 4), nibble(p), nibble(p >> 12)])
        .collect()
}

/// The fonts in a `fonts` folder (beside the maps): each slot's font, by
/// `Font::index`, when its file is listed and reads. A missing or unreadable
/// folder gives none.
pub fn read_table(dir: &Path, wanted: impl Fn(char) -> bool + Copy) -> Vec<Option<FontFile>> {
    let mut fonts = vec![None; Font::ALL.len()];
    let Ok(table) = std::fs::read_to_string(dir.join("font_table.txt")) else {
        return fonts;
    };
    let mut read: Vec<(String, Option<FontFile>)> = Vec::new();
    for (slot, line) in table.lines().take(fonts.len()).enumerate() {
        let name = line.trim();
        // A file name, not a path elsewhere.
        if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
            continue;
        }
        if let Some((_, font)) = read.iter().find(|(n, _)| n == name) {
            fonts[slot] = font.clone();
            continue;
        }
        let font = std::fs::read(dir.join(name))
            .ok()
            .and_then(|b| parse(&b, wanted).ok());
        fonts[slot] = font.clone();
        read.push((name.to_string(), font));
    }
    fonts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put16(b: &mut [u8], o: usize, v: u16) {
        b[o..o + 2].copy_from_slice(&v.to_le_bytes());
    }

    fn put32(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// A font of two characters: an empty one (index 0, for code points
    /// without glyphs) and a 3x2 'A'.
    pub(crate) fn two_glyph_font() -> Vec<u8> {
        // 'A': a whole pixel (red, half transparent), two opaque red ones,
        // then a 7/15 alpha and an opaque pixel (0b11_011_111), then one
        // transparent one.
        let a = [0x00, 0x8F, 0x00, 0x42, 0b1101_1111, 0x01];
        let mut b = vec![0u8; CHARACTERS + 2 * CHARACTER_SIZE];
        put32(&mut b, HEADER, VERSION);
        put16(&mut b, 0x204, 12);
        put16(&mut b, 0x206, 3);
        put32(&mut b, 0x20C, 2);
        // One kerning pair: 'A' then 'V' a pixel closer.
        put32(&mut b, KERNING, 1);
        b[KERNING + 4] = b'A';
        b[KERNING + 5] = b'V';
        put16(&mut b, KERNING + 6, (-1i16) as u16);
        put32(&mut b, CODE_POINTS + 4 * 'A' as usize, 1);
        let r = CHARACTERS + CHARACTER_SIZE;
        put16(&mut b, r, 4);
        put16(&mut b, r + 2, a.len() as u16);
        put16(&mut b, r + 4, 3);
        put16(&mut b, r + 6, 2);
        put16(&mut b, r + 8, 0);
        put16(&mut b, r + 0xA, 12);
        let at = b.len() as u32;
        put32(&mut b, r + 0xC, at);
        b.extend_from_slice(&a);
        b
    }

    #[test]
    fn a_font_file_gives_its_glyphs_metrics_and_kerning() {
        let f = parse(&two_glyph_font(), |_| true).unwrap();
        assert_eq!((f.ascent, f.descent), (12, 3));
        // Code point 0 has the empty character; nothing else maps to it.
        assert_eq!(f.glyphs.len(), 2);
        assert!(f.glyph('B').is_none());
        let a = f.glyph('A').unwrap();
        assert_eq!((a.advance, a.width, a.height, a.origin), (4, 3, 2, [0, 12]));
        let px: Vec<&[u8]> = a.rgba.chunks(4).collect();
        assert_eq!(px[0], [255, 0, 0, 136]);
        assert_eq!(px[1], [255, 0, 0, 255]);
        assert_eq!(px[2], [255, 0, 0, 255]);
        assert_eq!(px[3], [255, 0, 0, 119]);
        assert_eq!(px[4], [255, 0, 0, 255]);
        assert_eq!(px[5], [255, 0, 0, 0]);
        assert_eq!(f.kern('A', 'V'), -1);
        assert_eq!(f.kern('V', 'A'), 0);
        // Only the glyphs asked for are decoded.
        let only = parse(&two_glyph_font(), |c| c == 'A').unwrap();
        assert_eq!(only.glyphs.len(), 1);
    }

    #[test]
    fn short_runs_follow_a_pixel_of_their_own_alpha() {
        // 0b10_111_001: one opaque pixel, then 4 opaque; 0b10_000_110: one
        // clear pixel, then 3 clear.
        let px = decode(&[0b1011_1001, 0b1000_0110], 9);
        let alpha: Vec<u8> = px.chunks(4).map(|p| p[3]).collect();
        assert_eq!(alpha, [255, 255, 255, 255, 255, 0, 0, 0, 0]);
        // White until a colour is given.
        assert_eq!(&px[..3], [255, 255, 255]);
    }

    #[test]
    fn a_bad_file_or_a_missing_folder_gives_no_fonts() {
        assert!(parse(&[0u8; 0x100], |_| true).is_err());
        let mut b = two_glyph_font();
        b.truncate(b.len() - 2);
        assert!(parse(&b, |_| true).is_err());
        let fonts = read_table(Path::new("/nonexistent/fonts"), |_| true);
        assert_eq!(fonts.len(), 12);
        assert!(fonts.iter().all(Option::is_none));
    }

    #[test]
    fn the_table_names_each_slots_file() {
        let dir = std::env::temp_dir().join(format!("h2fonts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("conduit"), two_glyph_font()).unwrap();
        // Terminal is missing, body and title share a file, and a path
        // elsewhere isn't followed.
        std::fs::write(
            dir.join("font_table.txt"),
            "missing\r\nconduit\r\nconduit\r\n../conduit\r\n",
        )
        .unwrap();
        let fonts = read_table(&dir, |_| true);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(fonts[Font::Terminal.index()].is_none());
        assert!(fonts[Font::Body.index()].is_some());
        assert_eq!(fonts[Font::Title.index()], fonts[Font::Body.index()]);
        assert!(fonts[Font::SuperLarge.index()].is_none());
    }
}
