//! Turning the menus into a `DrawList`: textured quads in window pixels,
//! each with a blend mode (plain, multiply or additive), wrapped or
//! clamped texture coordinates and a tint, plus solid fills. Text arrives
//! as glyph quads, so a backend only ever draws quads (`cpu` here; a
//! Direct3D 11 one later, menu.md section 2).
//!
//! The `Painter` places pictures, flat shapes and text in UI units (as
//! menu-preview's did, now writing a list rather than a HUD batch).

use crate::anim::{self, Animation};
use crate::art::{ArtSource, ImageRef, Picture};
use crate::layout::{BitmapWidget, Flat, Space, TextWidget};
use crate::text::{Fonts, Justify, Line};
use crate::{Font, Rgba};

/// How a quad goes over what is already drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Blend {
    /// Over it, by the quad's alpha (the texture's times the tint's).
    #[default]
    Plain,
    /// What's there times the quad's colour, faded towards no change by
    /// the tint's alpha. The texture's own alpha isn't used (menu-preview
    /// found the menu art's multiply pictures meant to be drawn so).
    Multiply,
    /// Added to it, by the quad's alpha.
    Additive,
}

/// Which pixels a quad shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Texture {
    /// None: a solid fill of the tint.
    White,
    /// A picture of the art source (`ArtSource::image`).
    Art(u32),
    /// One of Halo 2's fonts' atlases (`Fonts::image`).
    Font(u16),
    /// The built-in font's atlas.
    Fallback,
}

/// One quad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub texture: Texture,
    /// Where, in window pixels: left, top, right, bottom (+y down).
    pub rect: [f32; 4],
    /// The texture coordinates at its left top and right bottom (0 to 1
    /// across the texture; beyond that repeats on an axis that `wrap`s).
    pub uv: [f32; 4],
    /// The tint at its top and at its bottom (the same for a flat tint):
    /// the texture's colour is multiplied by it.
    pub color: [Rgba; 2],
    pub blend: Blend,
    /// Across and down: repeat the texture past its edges (art scrolling
    /// that way); otherwise stop at the edges of `uv`'s part of it.
    pub wrap: [bool; 2],
}

/// Quads in the order they're drawn, for a window `size` pixels big.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawList {
    pub size: [u32; 2],
    pub quads: Vec<Quad>,
}

/// Where a backend finds a quad's pixels.
pub trait Textures {
    /// The pixels of `texture` (none for `Texture::White`, or for what
    /// can't be found: a backend skips those quads).
    fn image(&self, texture: Texture) -> Option<ImageRef<'_>>;
}

/// The art source's pictures and the fonts' atlases together.
#[derive(Clone, Copy)]
pub struct Resources<'a> {
    pub art: &'a dyn ArtSource,
    pub fonts: &'a Fonts,
}

impl Textures for Resources<'_> {
    fn image(&self, texture: Texture) -> Option<ImageRef<'_>> {
        match texture {
            Texture::White => None,
            Texture::Art(id) => self.art.image(id),
            Texture::Font(_) | Texture::Fallback => self.fonts.image(texture),
        }
    }
}

/// How a line of text looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub font: Font,
    pub color: Rgba,
    pub justify: Justify,
    /// Times its font's size.
    pub scale: f32,
    /// Shaded to this colour at the baseline, from `color` at the top of
    /// its capitals.
    pub bottom: Option<Rgba>,
}

impl TextStyle {
    pub fn new(font: Font, color: Rgba, justify: Justify) -> TextStyle {
        TextStyle {
            font,
            color,
            justify,
            scale: 1.0,
            bottom: None,
        }
    }

    /// A text widget's look, at `alpha`.
    pub fn of(t: &TextWidget, alpha: f32) -> TextStyle {
        TextStyle::new(t.font, crate::rgba(t.color, alpha), t.justify)
    }

    /// Faded by `alpha`.
    pub fn alpha(self, alpha: f32) -> TextStyle {
        let fade = |mut c: Rgba| {
            c[3] *= alpha;
            c
        };
        TextStyle {
            color: fade(self.color),
            bottom: self.bottom.map(fade),
            ..self
        }
    }
}

/// Which corner of a bitmap its corner is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// A screen's bitmaps.
    TopLeft,
    /// A list skin's bitmaps.
    BottomLeft,
}

/// Dashes of a flat track line, as shares of its width.
const DASHES: [[f32; 2]; 5] = [
    [0.0, 0.22],
    [0.3, 0.38],
    [0.45, 0.7],
    [0.78, 0.83],
    [0.88, 0.97],
];
/// A flat sheen strip's dark band, as a share of its width.
const SHEEN_BAND: f32 = 0.3;
/// A flat glow bar's layers, and each one's share of its colour's alpha
/// (eight at 0.16 build up to about its alpha in the middle).
const GLOW_LAYERS: usize = 8;
const GLOW_LAYER: f32 = 0.16;

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    [0, 1, 2, 3].map(|k| a[k] + (b[k] - a[k]) * t)
}

/// Builds a `DrawList` in UI units.
pub struct Painter<'a> {
    list: DrawList,
    pub space: Space,
    pub art: &'a dyn ArtSource,
    pub fonts: &'a Fonts,
    /// Seconds on the menus' clock: art scrolls by it.
    pub clock: f64,
}

impl<'a> Painter<'a> {
    pub fn new(space: Space, art: &'a dyn ArtSource, fonts: &'a Fonts, clock: f64) -> Painter<'a> {
        Painter {
            list: DrawList {
                size: [space.size[0] as u32, space.size[1] as u32],
                quads: Vec::new(),
            },
            space,
            art,
            fonts,
            clock,
        }
    }

    pub fn finish(self) -> DrawList {
        self.list
    }

    pub fn list(&self) -> &DrawList {
        &self.list
    }

    /// Adds a quad, unless it can't show: no area, clear, or off the
    /// window.
    pub fn push(&mut self, q: Quad) {
        let [x0, y0, x1, y1] = q.rect;
        let [w, h] = self.space.size;
        let shows = x1 > x0 && y1 > y0 && x1 > 0.0 && y1 > 0.0 && x0 < w && y0 < h;
        if shows && (q.color[0][3] > 0.0 || q.color[1][3] > 0.0) {
            self.list.quads.push(q);
        }
    }

    /// A solid box of window pixels.
    pub fn fill_px(&mut self, rect: [f32; 4], color: Rgba) {
        self.gradient_px(rect, color, color);
    }

    /// A box of window pixels shading from `top` to `bottom`.
    pub fn gradient_px(&mut self, rect: [f32; 4], top: Rgba, bottom: Rgba) {
        self.push(Quad {
            texture: Texture::White,
            rect,
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [top, bottom],
            blend: Blend::Plain,
            wrap: [false; 2],
        });
    }

    /// A solid box (UI units).
    pub fn fill(&mut self, rect: [f32; 4], color: Rgba) {
        self.fill_blended(rect, color, Blend::Plain);
    }

    /// A solid box (UI units) drawn with `blend`.
    pub fn fill_blended(&mut self, rect: [f32; 4], color: Rgba, blend: Blend) {
        self.push(Quad {
            texture: Texture::White,
            rect: self.space.rect(rect),
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [color; 2],
            blend,
            wrap: [false; 2],
        });
    }

    /// A box (UI units) shading from `top` to `bottom`.
    pub fn gradient(&mut self, rect: [f32; 4], top: Rgba, bottom: Rgba) {
        self.gradient_px(self.space.rect(rect), top, bottom);
    }

    /// The art source's picture of a bitmap tag.
    pub fn picture(&self, name: &str, frame: usize) -> Option<Picture> {
        self.art.picture(name, frame)
    }

    /// How big a bitmap is (UI units): its picture's pixels times its
    /// scale, or its flat shape's size without the picture.
    pub fn size(&self, w: &BitmapWidget) -> Option<[f32; 2]> {
        match self.picture(&w.name, w.frame) {
            Some(p) => {
                let [sx, sy] = w.scale();
                Some([p.size[0] * sx, p.size[1] * sy])
            }
            None => w.flat_size(),
        }
    }

    /// A bitmap with its corner (by `anchor`) at `at` (UI units), faded to
    /// `alpha`: its picture, scrolling as it says, or its flat shape.
    /// Returns whether the picture was there.
    pub fn bitmap(&mut self, w: &BitmapWidget, at: [f32; 2], anchor: Anchor, alpha: f32) -> bool {
        let Some([bw, bh]) = self.size(w) else {
            return false;
        };
        let [x, y] = match anchor {
            Anchor::TopLeft => at,
            Anchor::BottomLeft => [at[0], at[1] + bh],
        };
        let Some(p) = self.picture(&w.name, w.frame) else {
            self.flat(w, [x, y, x + bw, y - bh], alpha);
            return false;
        };
        let [su, sv] = [0, 1].map(|k| anim::scroll(self.clock, w.wraps_per_second[k]));
        let [u0, v0, u1, v1] = p.uv;
        let (du, dv) = (su * (u1 - u0), sv * (v1 - v0));
        self.push(Quad {
            texture: p.texture,
            rect: self.space.rect([x, y, x + bw, y - bh]),
            uv: [u0 + du, v0 + dv, u1 + du, v1 + dv],
            color: [[1.0, 1.0, 1.0, alpha]; 2],
            blend: w.blend,
            wrap: w.wraps_per_second.map(|v| v != 0.0),
        });
        true
    }

    /// A bitmap's flat shape in the box `[l, t, r, b]` (UI units).
    fn flat(&mut self, w: &BitmapWidget, [l, t, r, b]: [f32; 4], alpha: f32) {
        let fade = |c: Rgba| [c[0], c[1], c[2], c[3] * alpha];
        let width = r - l;
        match w.flat {
            Flat::None => {}
            Flat::Glow { color, .. } => {
                // Soft layers, each a little narrower and shorter, so the
                // bar is brightest in its middle and fades out towards its
                // ends, top and bottom (clear in its top and bottom 12%,
                // so the rows' bars stay apart).
                let height = t - b;
                let mid = (t + b) * 0.5;
                let layer = fade([color[0], color[1], color[2], color[3] * GLOW_LAYER]);
                let clear = [layer[0], layer[1], layer[2], 0.0];
                for i in 0..GLOW_LAYERS {
                    let (x, y) = (width * 0.03 * i as f32, height * (0.12 + 0.02 * i as f32));
                    let (l, r) = (l + x, r - x);
                    self.gradient([l, t - y, r, mid], clear, layer);
                    self.gradient([l, mid, r, b + y], layer, clear);
                }
            }
            Flat::Track { color, .. } => {
                let s = anim::scroll(self.clock, w.wraps_per_second[0]);
                for [a, z] in DASHES {
                    self.band(l, width, [t, b], [a - s, z - s], fade(color), Blend::Plain);
                }
            }
            Flat::Sheen { color, .. } => {
                // Nested bands, each darkening a little more towards the
                // middle, so the band's sides are soft.
                let s = anim::scroll(self.clock, w.wraps_per_second[0]);
                let c = fade([color[0], color[1], color[2], color[3] / 3.0]);
                for k in 0..3 {
                    let edge = SHEEN_BAND * k as f32 / 6.0;
                    let band = [edge - s, SHEEN_BAND - edge - s];
                    self.band(l, width, [t, b], band, c, Blend::Multiply);
                }
            }
            Flat::Brace { color, .. } => {
                let c = fade(color);
                let e = 2.0;
                self.fill([l, t, r, t - e], c);
                self.fill([l, t, l + e, b], c);
                self.fill([r - e, t, r, b], c);
                let m = (l + r) * 0.5;
                self.fill([m - e * 0.5, t, m + e * 0.5, (t + b) * 0.5], c);
            }
            Flat::Lettering {
                text,
                font,
                scale,
                color,
                bottom,
                ..
            } => {
                let style = TextStyle {
                    scale,
                    bottom: Some(fade(bottom)),
                    ..TextStyle::new(font, fade(color), Justify::Center)
                };
                // A soft shadow down and right of it (how far and how
                // dark are our own choices).
                let drop = 6.0;
                let shadow = TextStyle {
                    bottom: None,
                    ..TextStyle::new(font, fade([0.0, 0.0, 0.0, 0.45]), Justify::Center)
                };
                let shadow = TextStyle { scale, ..shadow };
                self.text([l + drop, t - drop, r + drop, b - drop], shadow, text);
                self.text([l, t, r, b], style, text);
            }
        }
    }

    /// A share `[a, z]` of the width `width` from `l` (wrapping round at
    /// its ends), filled between `t` and `b`.
    fn band(
        &mut self,
        l: f32,
        width: f32,
        [t, b]: [f32; 2],
        [a, z]: [f32; 2],
        c: Rgba,
        blend: Blend,
    ) {
        let a0 = a.rem_euclid(1.0);
        let z0 = a0 + (z - a);
        for [p, q] in [[a0, z0.min(1.0)], [0.0, (z0 - 1.0).max(0.0)]] {
            if q > p {
                self.fill_blended([l + width * p, t, l + width * q, b], c, blend);
            }
        }
    }

    /// Bitmaps in their depth order (lower first), each at its corner from
    /// `origin`, `age` seconds after their screen opened, as their intros
    /// bring them in, faded by `alpha`.
    pub fn bitmaps(
        &mut self,
        widgets: &[BitmapWidget],
        origin: [f32; 2],
        anchor: Anchor,
        age: f32,
        alpha: f32,
    ) {
        let mut sorted: Vec<&BitmapWidget> = widgets.iter().collect();
        sorted.sort_by_key(|w| w.depth);
        for w in sorted {
            let (a, [dx, dy]) = anim::intro(w.intro.as_ref(), age, w.delay_ms);
            let at = [origin[0] + w.corner[0] + dx, origin[1] + w.corner[1] + dy];
            self.bitmap(w, at, anchor, a * alpha);
        }
    }

    /// A line of text in a box (UI units): across as its style says, its
    /// line (ascent and descent) centred down the box. Returns its width.
    pub fn text(&mut self, [l, t, r, b]: [f32; 4], style: TextStyle, text: &str) -> f32 {
        let line = self.fonts.line(style.font, style.scale, text);
        let x = match style.justify {
            Justify::Left => l,
            Justify::Right => r - line.width,
            Justify::Center => (l + r - line.width) * 0.5,
        };
        let top = (t + b + line.ascent + line.descent) * 0.5;
        self.line(&line, [x, top - line.ascent], style);
        line.width
    }

    /// A text widget's line, its box moved by `offset`, faded by `alpha`.
    pub fn text_widget(&mut self, w: &TextWidget, offset: [f32; 2], alpha: f32, text: &str) -> f32 {
        let bounds = crate::layout::offset(w.bounds, offset);
        self.text(bounds, TextStyle::of(w, alpha), text)
    }

    /// A line laid out, starting at `at` on its baseline (UI units). The
    /// start is put on a whole pixel, so glyphs keep their edges.
    pub fn line(&mut self, line: &Line, at: [f32; 2], style: TextStyle) {
        let [px, py] = self.space.at(at).map(f32::round);
        let k = self.space.k;
        let caps = line.caps.max(f32::MIN_POSITIVE);
        for g in &line.glyphs {
            let rect = [
                px + g.rect[0] * k,
                py + g.rect[1] * k,
                px + g.rect[2] * k,
                py + g.rect[3] * k,
            ];
            let color = match (g.color, style.bottom) {
                (Some([r, gr, b]), _) => [[r, gr, b, style.color[3]]; 2],
                (None, Some(bottom)) => {
                    let at = |y: f32| ((y + caps) / caps).clamp(0.0, 1.0);
                    [
                        mix(style.color, bottom, at(g.rect[1])),
                        mix(style.color, bottom, at(g.rect[3])),
                    ]
                }
                (None, None) => [style.color; 2],
            };
            self.push(Quad {
                texture: g.texture,
                rect,
                uv: g.uv,
                color,
                blend: Blend::Plain,
                wrap: [false; 2],
            });
        }
    }
}

/// An animation's alpha, made relative to its peak (a list's intro that
/// only reaches half still brings it in whole).
pub fn relative(anim: Option<&Animation>, t: f32, delay_ms: f32) -> (f32, [f32; 2]) {
    let (alpha, offset) = anim::intro(anim, t, delay_ms);
    (alpha / anim.map_or(1.0, Animation::peak), offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::{ArtImage, FlatArt, MemoryArt};

    fn art() -> MemoryArt {
        let mut art = MemoryArt::new();
        let img = |w: usize, h: usize| ArtImage {
            width: w,
            height: h,
            rgba: vec![255; w * h * 4],
        };
        art.add_image("ui\\a", img(4, 2));
        art.add_image("ui\\b", img(2, 2));
        art
    }

    /// A unit to a pixel, the origin at (800, 600).
    fn space() -> Space {
        Space::new(1600, 1200)
    }

    #[test]
    fn a_screens_bitmaps_become_quads_in_depth_order() {
        let (art, fonts) = (art(), Fonts::fallback());
        let mut p = Painter::new(space(), &art, &fonts, 1.0);
        let widgets = [
            BitmapWidget {
                scale: [2.0, 2.0],
                blend: Blend::Multiply,
                wraps_per_second: [0.25, 0.0],
                depth: 2,
                ..BitmapWidget::new("b", [100.0, 50.0])
            },
            BitmapWidget {
                depth: 1,
                ..BitmapWidget::new("ui\\a", [-100.0, 0.0])
            },
            // Neither a picture nor a flat shape: nothing.
            BitmapWidget::new("missing", [0.0, 0.0]),
            BitmapWidget {
                depth: 3,
                intro: Some(Animation::screen_fade()),
                delay_ms: 100.0,
                ..BitmapWidget::new("ui\\a", [0.0, 0.0])
            },
        ];
        p.bitmaps(&widgets, [0.0, 0.0], Anchor::TopLeft, 0.225, 1.0);
        let list = p.finish();
        assert_eq!(list.size, [1600, 1200]);
        assert_eq!(list.quads.len(), 3);
        let [a, b, faded] = [list.quads[0], list.quads[1], list.quads[2]];
        assert_eq!(a.texture, Texture::Art(0));
        assert_eq!(a.rect, [700.0, 600.0, 704.0, 602.0]);
        assert_eq!(a.uv, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!((a.blend, a.wrap), (Blend::Plain, [false; 2]));
        assert_eq!(a.color, [[1.0; 4]; 2]);
        // Scaled twice, scrolled a quarter of its width, multiplied and
        // repeating.
        assert_eq!(b.texture, Texture::Art(1));
        assert_eq!(b.rect, [900.0, 550.0, 904.0, 554.0]);
        assert_eq!(b.uv, [0.25, 0.0, 1.25, 1.0]);
        assert_eq!((b.blend, b.wrap), (Blend::Multiply, [true, false]));
        // Halfway through its fade, after its delay.
        assert!((faded.color[0][3] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn skin_bitmaps_hang_from_their_bottom_left() {
        let (art, fonts) = (art(), Fonts::fallback());
        let mut p = Painter::new(space(), &art, &fonts, 0.0);
        let w = BitmapWidget::new("a", [-60.0, -40.0]);
        p.bitmaps(&[w], [10.0, 0.0], Anchor::BottomLeft, 0.0, 0.5);
        let q = p.finish().quads[0];
        // Its bottom at -40, its top 2 above.
        assert_eq!(q.rect, [750.0, 638.0, 754.0, 640.0]);
        assert_eq!(q.color[0][3], 0.5);
    }

    #[test]
    fn without_the_art_bitmaps_draw_their_flat_shapes() {
        let fonts = Fonts::fallback();
        let mut p = Painter::new(space(), &FlatArt, &fonts, 0.0);
        let track = BitmapWidget {
            flat: Flat::Track {
                size: [1000.0, 4.0],
                color: [1.0, 1.0, 1.0, 0.5],
            },
            wraps_per_second: [0.1, 0.0],
            ..BitmapWidget::new("track", [-500.0, 10.0])
        };
        assert!(!p.bitmap(&track, track.corner, Anchor::TopLeft, 1.0));
        let dashes = p.list().quads.len();
        assert_eq!(dashes, DASHES.len());
        assert!(p
            .list()
            .quads
            .iter()
            .all(|q| q.rect[1] == 590.0 && q.rect[3] == 594.0));
        // Scrolled half way along, the first dash starts in the middle.
        p.clock = 5.0;
        p.bitmap(&track, track.corner, Anchor::TopLeft, 1.0);
        assert_eq!(p.list().quads[dashes].rect[0], 800.0);
        // A sheen strip is multiplied: three nested bands, the outer one
        // across its first 30%, each darkening by a third.
        let sheen = BitmapWidget {
            flat: Flat::Sheen {
                size: [100.0, 10.0],
                color: [0.5, 0.5, 0.5, 1.0],
            },
            ..BitmapWidget::new("sheen", [0.0, 0.0])
        };
        let before = p.list().quads.len();
        p.bitmap(&sheen, sheen.corner, Anchor::TopLeft, 1.0);
        let bands = &p.list().quads[before..];
        assert_eq!(bands.len(), 3);
        assert!(bands.iter().all(|q| q.blend == Blend::Multiply));
        assert_eq!(bands[0].rect, [800.0, 600.0, 830.0, 610.0]);
        assert_eq!(bands[2].rect, [810.0, 600.0, 820.0, 610.0]);
        assert!((bands[0].color[0][3] - 1.0 / 3.0).abs() < 1e-6);
        // Clear shapes and shapes off the window add nothing.
        let before = p.list().quads.len();
        p.bitmap(&track, track.corner, Anchor::TopLeft, 0.0);
        p.bitmap(&track, [5000.0, 0.0], Anchor::TopLeft, 1.0);
        assert_eq!(p.list().quads.len(), before);
    }

    #[test]
    fn text_becomes_glyph_quads_in_its_box() {
        let fonts = crate::text::tests::fonts();
        let mut p = Painter::new(space(), &FlatArt, &fonts, 0.0);
        let style = TextStyle::new(Font::MainMenu, [0.62, 0.74, 0.84, 0.5], Justify::Center);
        // "LI" is 13 wide: centred in a box 100 wide from 0.
        let w = p.text([0.0, 20.0, 100.0, -10.0], style, "LI");
        assert_eq!(w, 13.0);
        let list = p.finish();
        assert_eq!(list.quads.len(), 2);
        let l = list.quads[0];
        assert_eq!(l.texture, Texture::Font(0));
        assert_eq!(l.color, [[0.62, 0.74, 0.84, 0.5]; 2]);
        // The line (ascent 12, descent 3) centred down the box: its
        // baseline at 20 - (30 - 15) / 2 - 12 = 0.5, put on a whole pixel.
        let x = (800.0 + 43.5f32).round();
        assert_eq!(l.rect, [x + 2.0, 590.0, x + 8.0, 600.0]);
        assert_eq!((l.blend, l.wrap), (Blend::Plain, [false; 2]));
        // Right justified, it ends at the box's right.
        let mut p = Painter::new(space(), &FlatArt, &fonts, 0.0);
        let right = TextStyle {
            justify: Justify::Right,
            ..style
        };
        p.text([0.0, 20.0, 100.0, -10.0], right, "LI");
        assert_eq!(p.list().quads[1].rect[2], 800.0 + 99.0);
    }

    #[test]
    fn shaded_text_runs_from_its_colour_at_the_top_to_the_bottom() {
        let fonts = crate::text::tests::fonts();
        let mut p = Painter::new(space(), &FlatArt, &fonts, 0.0);
        let style = TextStyle {
            bottom: Some([0.0, 0.0, 0.0, 1.0]),
            ..TextStyle::new(Font::MainMenu, [1.0; 4], Justify::Left)
        };
        p.text([0.0, 10.0, 100.0, -10.0], style, "H");
        let q = p.finish().quads[0];
        assert_eq!(q.color, [[1.0; 4], [0.0, 0.0, 0.0, 1.0]]);
        // Fading fades both ends.
        let faded = style.alpha(0.5);
        assert_eq!((faded.color[3], faded.bottom.unwrap()[3]), (0.5, 0.5));
    }

    #[test]
    fn resources_hand_over_art_and_font_pixels() {
        let (art, fonts) = (art(), Fonts::fallback());
        let r = Resources {
            art: &art,
            fonts: &fonts,
        };
        assert_eq!(r.image(Texture::Art(1)).unwrap().width, 2);
        assert!(r.image(Texture::Fallback).is_some());
        assert!(r.image(Texture::White).is_none());
        assert!(r.image(Texture::Font(0)).is_none());
        let a = Animation::fade(100.0, 0.0, 0.5);
        assert_eq!(relative(Some(&a), 1.0, 0.0).0, 1.0);
        assert_eq!(relative(None, 0.0, 0.0).0, 1.0);
    }
}
