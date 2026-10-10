//! The lobby's drawing: a pixel buffer (`0x00RRGGBB`, as `softbuffer`
//! shows it) with blended rectangles, circles and text. Text is in Halo 2's
//! own fonts, read from the owner's MCC at run time (never shipped), with a
//! font already on the PC, rasterised by `ab_glyph`, for what they lack;
//! glyphs are kept per size once drawn.

use ab_glyph::{point, Font, FontArc, GlyphId, PxScale, ScaleFont};
use blam_cache::font::{Font as H2Font, FontFile, Glyph};
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(hex: u32) -> Color {
        Color {
            r: (hex >> 16) as u8,
            g: (hex >> 8) as u8,
            b: hex as u8,
            a: 255,
        }
    }

    pub const fn alpha(self, a: u8) -> Color {
        Color { a, ..self }
    }
}

/// Where a line of text sits against its x.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

fn blend(dst: u32, c: Color, cover: u32) -> u32 {
    let a = u32::from(c.a) * cover / 255;
    if a == 0 {
        return dst;
    }
    let mix = |d: u32, s: u8| (d * (255 - a) + u32::from(s) * a) / 255;
    let r = mix((dst >> 16) & 0xFF, c.r);
    let g = mix((dst >> 8) & 0xFF, c.g);
    let b = mix(dst & 0xFF, c.b);
    (r << 16) | (g << 8) | b
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        Canvas {
            w,
            h,
            px: vec![0; w * h],
        }
    }

    pub fn resize(&mut self, w: usize, h: usize) {
        if (w, h) != (self.w, self.h) {
            *self = Canvas::new(w, h);
        }
    }

    /// The pixel span `[a, b)` a coordinate range covers, clipped to `0..max`.
    fn span(a: f32, b: f32, max: usize) -> (usize, usize) {
        let lo = a.round().clamp(0.0, max as f32) as usize;
        let hi = b.round().clamp(0.0, max as f32) as usize;
        (lo, hi.max(lo))
    }

    pub fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, c: Color) {
        let (x0, x1) = Self::span(x, x + w, self.w);
        let (y0, y1) = Self::span(y, y + h, self.h);
        for row in y0..y1 {
            let line = &mut self.px[row * self.w..(row + 1) * self.w];
            for p in &mut line[x0..x1] {
                *p = blend(*p, c, 255);
            }
        }
    }

    /// A rectangle shading from `top` to `bottom`.
    pub fn gradient(&mut self, x: f32, y: f32, w: f32, h: f32, top: Color, bottom: Color) {
        let (y0, y1) = Self::span(y, y + h, self.h);
        let span = (y1 - y0).max(1) as f32;
        for row in y0..y1 {
            let t = (row - y0) as f32 / span;
            let lerp = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t) as u8;
            let c = Color {
                r: lerp(top.r, bottom.r),
                g: lerp(top.g, bottom.g),
                b: lerp(top.b, bottom.b),
                a: lerp(top.a, bottom.a),
            };
            self.fill(x, row as f32, w, 1.0, c);
        }
    }

    /// A frame `t` thick inside the rectangle.
    pub fn outline(&mut self, x: f32, y: f32, w: f32, h: f32, t: f32, c: Color) {
        self.fill(x, y, w, t, c);
        self.fill(x, y + h - t, w, t, c);
        self.fill(x, y + t, t, h - 2.0 * t, c);
        self.fill(x + w - t, y + t, t, h - 2.0 * t, c);
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, c: Color) {
        let (x0, x1) = Self::span(cx - r - 1.0, cx + r + 1.0, self.w);
        let (y0, y1) = Self::span(cy - r - 1.0, cy + r + 1.0, self.h);
        for row in y0..y1 {
            for col in x0..x1 {
                let d = ((col as f32 + 0.5 - cx).powi(2) + (row as f32 + 0.5 - cy).powi(2)).sqrt();
                let cover = (r + 0.5 - d).clamp(0.0, 1.0);
                if cover > 0.0 {
                    let p = &mut self.px[row * self.w + col];
                    *p = blend(*p, c, (cover * 255.0) as u32);
                }
            }
        }
    }

    /// Draws `s` in `style` with its baseline at `y`, and returns its
    /// width.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        font: &mut Text,
        style: Style,
        x: f32,
        y: f32,
        size: f32,
        c: Color,
        align: Align,
        s: &str,
    ) -> f32 {
        let (placed, width) = font.shape(style, size, s);
        let left = match align {
            Align::Left => x,
            Align::Center => x - width / 2.0,
            Align::Right => x - width,
        };
        for (pen, key) in placed {
            let Some(g) = font.cache.get(&key) else {
                continue;
            };
            let gx = (left + pen + g.left).round() as i64;
            let gy = (y + g.top).round() as i64;
            for row in 0..g.h {
                let py = gy + row as i64;
                if py < 0 || py >= self.h as i64 {
                    continue;
                }
                for col in 0..g.w {
                    let px = gx + col as i64;
                    if px < 0 || px >= self.w as i64 {
                        continue;
                    }
                    let cover = u32::from(g.cover[row * g.w + col]);
                    if cover > 0 {
                        let p = &mut self.px[py as usize * self.w + px as usize];
                        *p = blend(*p, c, cover);
                    }
                }
            }
        }
        width
    }

    /// Writes the picture as a PNG.
    pub fn save_png(&self, path: &Path) -> Result<(), String> {
        let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut enc =
            png::Encoder::new(std::io::BufWriter::new(file), self.w as u32, self.h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        let mut rgb = Vec::with_capacity(self.w * self.h * 3);
        for p in &self.px {
            rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
        }
        writer.write_image_data(&rgb).map_err(|e| e.to_string())
    }
}

/// One glyph at one size: its coverage and where it sits against the pen
/// and the baseline.
struct GlyphBitmap {
    w: usize,
    h: usize,
    left: f32,
    top: f32,
    cover: Vec<u8>,
}

/// Which of Halo 2's fonts a line uses. The screens pick it by the text's
/// size (`Style::for_size`), so the same line keeps its font at any window
/// size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Style {
    /// Small print: conduit, the text chat's font.
    Small,
    /// Column headings: Handel Gothic, the title slot's font.
    Heading,
    /// Ordinary text: conduit, the large body slot's.
    Body,
    /// Names and menu rows: Handel Gothic, the main menu's font.
    Menu,
    /// Screen titles and big numbers: the super large Handel Gothic.
    Large,
}

impl Style {
    /// The style for text `size` layout units high (the lobby's layout is
    /// 720 units high).
    pub fn for_size(size: f32) -> Style {
        match size {
            s if s < 15.0 => Style::Small,
            s if s < 17.0 => Style::Heading,
            s if s < 22.0 => Style::Body,
            s if s < 30.0 => Style::Menu,
            _ => Style::Large,
        }
    }

    fn slot(self) -> H2Font {
        match self {
            Style::Small => H2Font::TextChat,
            Style::Heading => H2Font::Title,
            Style::Body => H2Font::LargeBody,
            Style::Menu => H2Font::MainMenu,
            Style::Large => H2Font::SuperLarge,
        }
    }
}

/// A glyph in the cache: from the system font (its id and size in
/// quarter pixels) or from a Halo 2 font slot (the character and size).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    Vector(GlyphId, u32),
    Halo(usize, char, u32),
}

/// The lobby's fonts: Halo 2's own from MCC's `halo2\h2_fonts` when they
/// are there, and a font already on the PC for anything they lack (or for
/// everything, without them). Glyphs are kept per size once drawn.
pub struct Text {
    vector: Option<FontArc>,
    /// Each Halo 2 font slot's font, by `blam_cache::font::Font::index`.
    halo: Vec<Option<FontFile>>,
    cache: HashMap<Key, GlyphBitmap>,
}

/// Fonts tried in order: `H2LOBBY_FONT`, then fonts Windows and common
/// Linux installs have. None is shipped with the launcher.
const FONTS: [&str; 7] = [
    r"C:\Windows\Fonts\bahnschrift.ttf",
    r"C:\Windows\Fonts\segoeui.ttf",
    r"C:\Windows\Fonts\arial.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
];

/// The characters taken from Halo 2's fonts: Latin, punctuation and the
/// like (the rest of their code points are icons and other scripts).
fn wanted(c: char) -> bool {
    (c as u32) < 0x2200
}

/// `src` (`n` samples) resampled to `m`: averaged over each new sample's
/// span when shrinking, interpolated when growing.
fn resample(src: &[f32], n: usize, m: usize) -> Vec<f32> {
    let k = m as f32 / n as f32;
    (0..m)
        .map(|i| {
            if k >= 1.0 {
                let at = ((i as f32 + 0.5) / k - 0.5).clamp(0.0, (n - 1) as f32);
                let (a, t) = (at.floor() as usize, at.fract());
                let b = (a + 1).min(n - 1);
                src[a] * (1.0 - t) + src[b] * t
            } else {
                let (lo, hi) = (i as f32 / k, (i + 1) as f32 / k);
                let mut sum = 0.0;
                for (j, v) in src
                    .iter()
                    .enumerate()
                    .take(hi.ceil() as usize)
                    .skip(lo as usize)
                {
                    let overlap = (hi.min(j as f32 + 1.0) - lo.max(j as f32)).max(0.0);
                    sum += v * overlap;
                }
                sum * k
            }
        })
        .collect()
}

/// A Halo 2 glyph's alpha scaled by `k`.
fn scale_glyph(g: &Glyph, k: f32) -> GlyphBitmap {
    let (w, h) = (g.width as usize, g.height as usize);
    let (nw, nh) = (
        ((w as f32 * k).round() as usize).max(1),
        ((h as f32 * k).round() as usize).max(1),
    );
    let mut cover = Vec::new();
    if w > 0 && h > 0 && g.rgba.len() >= w * h * 4 {
        let alpha: Vec<f32> = g.rgba.chunks(4).map(|p| f32::from(p[3])).collect();
        let rows: Vec<Vec<f32>> = alpha.chunks(w).map(|r| resample(r, w, nw)).collect();
        let mut out = vec![0u8; nw * nh];
        for x in 0..nw {
            let column: Vec<f32> = rows.iter().map(|r| r[x]).collect();
            for (y, v) in resample(&column, h, nh).into_iter().enumerate() {
                out[y * nw + x] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
        cover = out;
    }
    GlyphBitmap {
        w: if cover.is_empty() { 0 } else { nw },
        h: if cover.is_empty() { 0 } else { nh },
        left: f32::from(g.origin[0]) * k,
        top: -f32::from(g.origin[1]) * k,
        cover,
    }
}

impl Text {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Text, String> {
        let font = FontArc::try_from_vec(bytes).map_err(|e| e.to_string())?;
        Ok(Text {
            vector: Some(font),
            halo: Vec::new(),
            cache: HashMap::new(),
        })
    }

    /// The first font found on this PC.
    pub fn system() -> Result<Text, String> {
        let wanted = std::env::var("H2LOBBY_FONT").ok();
        let candidates = wanted.iter().map(String::as_str).chain(FONTS);
        for path in candidates {
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(t) = Text::from_bytes(bytes) {
                    return Ok(t);
                }
            }
        }
        Err(format!(
            "no font found (looked for {}); set H2LOBBY_FONT to a .ttf file",
            FONTS.join(", ")
        ))
    }

    /// Halo 2's fonts from `dir` (MCC's `halo2\h2_fonts`, with its
    /// `font_table.txt`), over the system font. Either alone will do; it
    /// fails only with neither.
    pub fn halo2(dir: Option<&Path>) -> Result<Text, String> {
        let system = Text::system();
        let halo = dir
            .map(|d| blam_cache::font::read_table(d, wanted))
            .unwrap_or_default();
        if !halo.iter().any(Option::is_some) {
            return system;
        }
        Ok(Text {
            vector: system.ok().and_then(|t| t.vector),
            halo,
            cache: HashMap::new(),
        })
    }

    /// Which of Halo 2's font slots have a font, by name.
    pub fn halo2_slots(&self) -> Vec<&'static str> {
        H2Font::ALL
            .iter()
            .filter(|f| self.halo.get(f.index()).is_some_and(Option::is_some))
            .map(|f| f.name())
            .collect()
    }

    /// Where each glyph of `s` goes from the line's start (its bitmap now
    /// in the cache), and the line's width.
    fn shape(&mut self, style: Style, size: f32, s: &str) -> (Vec<(f32, Key)>, f32) {
        let mut out = Vec::new();
        let mut pen = 0.0;
        let mut prev: Option<Key> = None;
        let quarter = (size * 4.0).round() as u32;
        let cache = &mut self.cache;
        for ch in s.chars() {
            let halo = halo_font(&self.halo, style).and_then(|(slot, f)| {
                let g = f.glyph(ch)?;
                let k = size / f32::from((f.ascent + f.descent).max(1));
                let kern = match prev {
                    Some(Key::Halo(p, c, _)) if p == slot => f32::from(f.kern(c, ch)) * k,
                    _ => 0.0,
                };
                Some((slot, g, k, kern))
            });
            if let Some((slot, g, k, kern)) = halo {
                let key = Key::Halo(slot, ch, quarter);
                let advance = f32::from(g.advance) * k;
                cache.entry(key).or_insert_with(|| scale_glyph(g, k));
                pen += kern;
                out.push((pen, key));
                pen += advance;
                prev = Some(key);
                continue;
            }
            let Some(font) = &self.vector else {
                prev = None;
                continue;
            };
            let scaled = font.as_scaled(PxScale::from(size));
            let id = scaled.glyph_id(ch);
            if let Some(Key::Vector(p, _)) = prev {
                pen += scaled.kern(p, id);
            }
            let key = Key::Vector(id, quarter);
            let advance = scaled.h_advance(id);
            cache
                .entry(key)
                .or_insert_with(|| vector_glyph(font, id, size));
            out.push((pen, key));
            pen += advance;
            prev = Some(key);
        }
        (out, pen)
    }

    /// How wide `s` is at `size` in `style`.
    pub fn measure(&mut self, style: Style, size: f32, s: &str) -> f32 {
        self.shape(style, size, s).1
    }

    /// `s` cut to fit `width` at `size`, with "..." when it was cut.
    pub fn fit(&mut self, style: Style, size: f32, width: f32, s: &str) -> String {
        if self.measure(style, size, s) <= width {
            return s.to_string();
        }
        let mut out: String = s.to_string();
        while !out.is_empty() && self.measure(style, size, &format!("{out}...")) > width {
            out.pop();
        }
        format!("{}...", out.trim_end())
    }
}

/// The Halo 2 font for `style`, if there is one: the slot's own, or the
/// main menu's.
fn halo_font(halo: &[Option<FontFile>], style: Style) -> Option<(usize, &FontFile)> {
    [style.slot(), H2Font::MainMenu]
        .into_iter()
        .find_map(|f| Some((f.index(), halo.get(f.index())?.as_ref()?)))
}

/// A glyph of the system font at `size`.
fn vector_glyph(font: &FontArc, id: GlyphId, size: f32) -> GlyphBitmap {
    let glyph = id.with_scale_and_position(PxScale::from(size), point(0.0, 0.0));
    let Some(outline) = font.outline_glyph(glyph) else {
        return GlyphBitmap {
            w: 0,
            h: 0,
            left: 0.0,
            top: 0.0,
            cover: Vec::new(),
        };
    };
    let b = outline.px_bounds();
    let (w, h) = (b.width() as usize, b.height() as usize);
    let mut cover = vec![0u8; w * h];
    outline.draw(|x, y, c| {
        if let Some(v) = cover.get_mut(y as usize * w + x as usize) {
            *v = (c.clamp(0.0, 1.0) * 255.0) as u8;
        }
    });
    GlyphBitmap {
        w,
        h,
        left: b.min.x,
        top: b.min.y,
        cover,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangles_blend_and_clip() {
        let mut c = Canvas::new(4, 3);
        c.fill(-5.0, -5.0, 100.0, 100.0, Color::rgb(0x102030));
        assert!(c.px.iter().all(|&p| p == 0x102030));
        c.fill(1.0, 1.0, 2.0, 1.0, Color::rgb(0xFFFFFF).alpha(255));
        assert_eq!(c.px[4 + 1], 0xFFFFFF);
        assert_eq!(c.px[4], 0x102030);
        // Half of white over black is grey.
        let mut c = Canvas::new(1, 1);
        c.fill(0.0, 0.0, 1.0, 1.0, Color::rgb(0xFFFFFF).alpha(128));
        assert_eq!(c.px[0], 0x808080);
        // A frame leaves the middle alone.
        let mut c = Canvas::new(5, 5);
        c.outline(0.0, 0.0, 5.0, 5.0, 1.0, Color::rgb(0xFF0000));
        assert_eq!(c.px[0], 0xFF0000);
        assert_eq!(c.px[2 * 5 + 2], 0);
        assert_eq!(c.px[2 * 5 + 4], 0xFF0000);
    }

    /// A Halo 2 font file (the layout `blam_cache::font` reads) with one
    /// glyph: an 'I' two pixels wide and ten high, all opaque, advancing
    /// four, drawn a pixel right of the pen; ascent 8 and descent 2.
    fn one_glyph_font() -> Vec<u8> {
        let put16 =
            |b: &mut Vec<u8>, o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_le_bytes());
        let put32 =
            |b: &mut Vec<u8>, o: usize, v: u32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        let mut b = vec![0u8; 0x40400 + 2 * 16];
        put32(&mut b, 0x200, 0xF000_0001);
        put16(&mut b, 0x204, 8);
        put16(&mut b, 0x206, 2);
        put32(&mut b, 0x20C, 2);
        put32(&mut b, 0x400 + 4 * 'I' as usize, 1);
        let r = 0x40400 + 16;
        put16(&mut b, r, 4);
        put16(&mut b, r + 2, 1);
        put16(&mut b, r + 4, 2);
        put16(&mut b, r + 6, 10);
        put16(&mut b, r + 8, 1);
        put16(&mut b, r + 0xA, 8);
        let at = b.len() as u32;
        put32(&mut b, r + 0xC, at);
        // Twenty opaque pixels of the current colour (white).
        b.push(0x40 | 20);
        b
    }

    fn halo_text(name: &str) -> (Text, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("h2lobby-fonts-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("conduit-13"), one_glyph_font()).unwrap();
        std::fs::write(dir.join("font_table.txt"), "conduit-13\r\n".repeat(12)).unwrap();
        let text = Text::halo2(Some(&dir)).unwrap();
        (text, dir)
    }

    #[test]
    fn halo2_fonts_give_their_glyphs_at_any_size() {
        let (mut text, dir) = halo_text("draw");
        assert_eq!(text.halo2_slots().len(), 12);
        // Ascent and descent make 10 pixels: at size 10 the glyph is as
        // it is in the file, at 20 twice as big.
        assert_eq!(text.measure(Style::Menu, 10.0, "II"), 8.0);
        assert_eq!(text.measure(Style::Body, 20.0, "I"), 8.0);
        let mut c = Canvas::new(40, 40);
        let w = c.text(
            &mut text,
            Style::Menu,
            10.0,
            20.0,
            10.0,
            Color::rgb(0xFFFFFF),
            Align::Left,
            "I",
        );
        assert_eq!(w, 4.0);
        // Its two columns start a pixel right of the pen, from 8 above the
        // baseline down to 2 below it.
        assert_eq!(c.px[12 * 40 + 11], 0xFFFFFF);
        assert_eq!(c.px[21 * 40 + 12], 0xFFFFFF);
        assert_eq!(c.px[12 * 40 + 10], 0);
        assert_eq!(c.px[11 * 40 + 11], 0);
        assert_eq!(c.px[22 * 40 + 11], 0);
        let mut c = Canvas::new(40, 40);
        c.text(
            &mut text,
            Style::Menu,
            10.0,
            30.0,
            20.0,
            Color::rgb(0xFF0000),
            Align::Left,
            "I",
        );
        assert_eq!(c.px[14 * 40 + 12], 0xFF0000);
        assert_eq!(c.px[33 * 40 + 15], 0xFF0000);
        assert_eq!(c.px[14 * 40 + 16], 0);
        // A character the font lacks comes from the system font, if any.
        if Text::system().is_ok() {
            assert!(text.measure(Style::Menu, 10.0, "IA") > 4.0);
        } else {
            assert_eq!(text.measure(Style::Menu, 10.0, "IA"), 4.0);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_halo2_fonts_the_system_font_is_used() {
        let missing = std::path::Path::new("/nonexistent/h2_fonts");
        match (Text::halo2(Some(missing)), Text::system()) {
            (Ok(t), Ok(_)) => assert!(t.halo2_slots().is_empty()),
            (Err(_), Err(_)) => {}
            _ => panic!("halo2() should fall back to exactly what system() gives"),
        }
    }

    #[test]
    fn styles_follow_the_text_size() {
        assert_eq!(Style::for_size(13.0), Style::Small);
        assert_eq!(Style::for_size(16.0), Style::Heading);
        assert_eq!(Style::for_size(20.0), Style::Body);
        assert_eq!(Style::for_size(24.0), Style::Menu);
        assert_eq!(Style::for_size(48.0), Style::Large);
    }

    #[test]
    fn resampling_averages_down_and_blends_up() {
        assert_eq!(resample(&[0.0, 255.0, 255.0, 0.0], 4, 2), [127.5, 127.5]);
        assert_eq!(resample(&[10.0, 20.0, 30.0], 3, 3), [10.0, 20.0, 30.0]);
        let up = resample(&[0.0, 255.0], 2, 4);
        assert_eq!(up.len(), 4);
        assert!(up.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!((up[0], up[3]), (0.0, 255.0));
    }

    #[test]
    fn a_circle_is_round() {
        let mut c = Canvas::new(21, 21);
        c.circle(10.5, 10.5, 8.0, Color::rgb(0x00FF00));
        assert_eq!(c.px[10 * 21 + 10], 0x00FF00);
        assert_eq!(c.px[0], 0);
        assert_eq!(c.px[10 * 21], 0);
    }
}
