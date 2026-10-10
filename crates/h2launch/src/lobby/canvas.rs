//! The lobby's drawing: a pixel buffer (`0x00RRGGBB`, as `softbuffer`
//! shows it) with blended rectangles, circles and text. Text comes from a
//! font already on the PC, rasterised by `ab_glyph` and kept per glyph and
//! size.

use ab_glyph::{point, Font, FontArc, GlyphId, PxScale, ScaleFont};
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

    /// Draws `s` with its baseline at `y`, and returns its width.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        font: &mut Text,
        x: f32,
        y: f32,
        size: f32,
        c: Color,
        align: Align,
        s: &str,
    ) -> f32 {
        let width = font.measure(size, s);
        let mut pen = match align {
            Align::Left => x,
            Align::Center => x - width / 2.0,
            Align::Right => x - width,
        };
        let mut prev: Option<GlyphId> = None;
        for ch in s.chars() {
            let id = font.font.glyph_id(ch);
            if let Some(p) = prev {
                pen += font.font.as_scaled(PxScale::from(size)).kern(p, id);
            }
            let g = font.glyph(id, size);
            let gx = (pen + g.left).round() as i64;
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
            pen += g.advance;
            prev = Some(id);
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
    advance: f32,
    cover: Vec<u8>,
}

/// A font and the glyphs drawn with it so far.
pub struct Text {
    font: FontArc,
    cache: HashMap<(GlyphId, u32), GlyphBitmap>,
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

impl Text {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Text, String> {
        let font = FontArc::try_from_vec(bytes).map_err(|e| e.to_string())?;
        Ok(Text {
            font,
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

    fn glyph(&mut self, id: GlyphId, size: f32) -> &GlyphBitmap {
        let key = (id, (size * 4.0).round() as u32);
        let font = &self.font;
        self.cache.entry(key).or_insert_with(|| {
            let scale = PxScale::from(size);
            let advance = font.as_scaled(scale).h_advance(id);
            let glyph = id.with_scale_and_position(scale, point(0.0, 0.0));
            let Some(outline) = font.outline_glyph(glyph) else {
                return GlyphBitmap {
                    w: 0,
                    h: 0,
                    left: 0.0,
                    top: 0.0,
                    advance,
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
                advance,
                cover,
            }
        })
    }

    /// How wide `s` is at `size`.
    pub fn measure(&mut self, size: f32, s: &str) -> f32 {
        let scaled = self.font.as_scaled(PxScale::from(size));
        let mut w = 0.0;
        let mut prev: Option<GlyphId> = None;
        for ch in s.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(p) = prev {
                w += scaled.kern(p, id);
            }
            w += scaled.h_advance(id);
            prev = Some(id);
        }
        w
    }

    /// `s` cut to fit `width` at `size`, with "..." when it was cut.
    pub fn fit(&mut self, size: f32, width: f32, s: &str) -> String {
        if self.measure(size, s) <= width {
            return s.to_string();
        }
        let mut out: String = s.to_string();
        while !out.is_empty() && self.measure(size, &format!("{out}...")) > width {
            out.pop();
        }
        format!("{}...", out.trim_end())
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

    #[test]
    fn a_circle_is_round() {
        let mut c = Canvas::new(21, 21);
        c.circle(10.5, 10.5, 8.0, Color::rgb(0x00FF00));
        assert_eq!(c.px[10 * 21 + 10], 0x00FF00);
        assert_eq!(c.px[0], 0);
        assert_eq!(c.px[10 * 21], 0);
    }
}
