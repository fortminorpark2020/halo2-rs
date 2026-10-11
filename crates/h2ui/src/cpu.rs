//! Draws a `DrawList` into a pixel buffer on the CPU: `0x00RRGGBB` words
//! (as `softbuffer` and the lobby's canvas hold them) or RGBA bytes.
//!
//! - All blending is in gamma space, on the 0 to 255 values themselves, as
//!   the lobby's canvas does (menu-preview's linear-colour conversion
//!   isn't wanted here).
//! - Textures are sampled bilinearly, at each pixel's centre, with colours
//!   weighted by their alpha so a clear texel's colour doesn't bleed into
//!   its neighbours. On each axis, a clamped quad stops at the edges of
//!   its own part of the texture (a glyph in an atlas never picks up its
//!   neighbour); a wrapped one repeats the whole texture.
//! - A pixel is drawn when its centre is inside the quad; edges aren't
//!   anti-aliased.
//! - A texture row is sampled across once (into a scratch row) and shared
//!   by every target row between it and the next; integer arithmetic,
//!   two channels to a word, with texels premultiplied to 8 bits a channel
//!   as they're read; no allocation per pixel (the scratch rows are kept
//!   for the whole call); everything clipped to the target first.

use crate::paint::{Blend, DrawList, Quad, Texture, Textures};
use crate::Rgba;

/// A pixel of a target buffer.
pub trait Pixel: Copy {
    /// Its red, green and blue (0 to 255).
    fn rgb(self) -> [u32; 3];
    fn from_rgb(rgb: [u32; 3]) -> Self;
}

/// `0x00RRGGBB`.
impl Pixel for u32 {
    #[inline]
    fn rgb(self) -> [u32; 3] {
        [(self >> 16) & 0xFF, (self >> 8) & 0xFF, self & 0xFF]
    }

    #[inline]
    fn from_rgb([r, g, b]: [u32; 3]) -> u32 {
        (r << 16) | (g << 8) | b
    }
}

/// Red, green, blue, alpha (always left opaque: the target is a window).
impl Pixel for [u8; 4] {
    #[inline]
    fn rgb(self) -> [u32; 3] {
        [u32::from(self[0]), u32::from(self[1]), u32::from(self[2])]
    }

    #[inline]
    fn from_rgb([r, g, b]: [u32; 3]) -> [u8; 4] {
        [r as u8, g as u8, b as u8, 255]
    }
}

/// Draws `list` over the `w` by `h` pixels of `px` (rows top first), with
/// the pixels of its textures from `textures`. Quads whose texture can't
/// be found are skipped; so is a buffer too small for `w` by `h`.
pub fn draw<P: Pixel>(list: &DrawList, px: &mut [P], w: usize, h: usize, textures: &dyn Textures) {
    if w == 0 || h == 0 || px.len() < w * h {
        return;
    }
    let mut scratch = Scratch::default();
    for q in &list.quads {
        quad(q, &mut px[..w * h], w, h, textures, &mut scratch);
    }
}

/// `draw` into `0x00RRGGBB` words.
pub fn draw_u32(list: &DrawList, px: &mut [u32], w: usize, h: usize, textures: &dyn Textures) {
    draw(list, px, w, h, textures);
}

/// `draw` into RGBA bytes (4 to a pixel).
pub fn draw_rgba(list: &DrawList, rgba: &mut [u8], w: usize, h: usize, textures: &dyn Textures) {
    let (px, _) = rgba.as_chunks_mut::<4>();
    draw(list, px, w, h, textures);
}

/// A list drawn once and copied in at the start of each frame: the still
/// background (`screens::background`), which takes longer to draw than
/// the rest of a frame. It is drawn again only when the list or the size
/// changes, or after `forget` (other pictures behind the same textures).
pub struct Kept<P> {
    list: Option<DrawList>,
    size: (usize, usize),
    px: Vec<P>,
}

impl<P> Default for Kept<P> {
    fn default() -> Self {
        Kept {
            list: None,
            size: (0, 0),
            px: Vec::new(),
        }
    }
}

impl<P: Pixel> Kept<P> {
    /// Draws it again next time.
    pub fn forget(&mut self) {
        self.list = None;
    }

    /// Puts `list`, drawn over black, in the `w` by `h` pixels of `px`
    /// (what was there is replaced), drawing it only if it isn't kept.
    pub fn draw(
        &mut self,
        list: &DrawList,
        px: &mut [P],
        w: usize,
        h: usize,
        textures: &dyn Textures,
    ) {
        if w == 0 || h == 0 || px.len() < w * h {
            return;
        }
        if self.list.as_ref() != Some(list) || self.size != (w, h) {
            self.px.clear();
            self.px.resize(w * h, P::from_rgb([0; 3]));
            draw(list, &mut self.px, w, h, textures);
            self.list = Some(list.clone());
            self.size = (w, h);
        }
        px[..w * h].copy_from_slice(&self.px);
    }
}

/// The pixels `[lo, hi)` whose centres are in `[a, b)`, clipped to
/// `0..max`.
fn span(a: f32, b: f32, max: usize) -> (usize, usize) {
    let lo = (a - 0.5).ceil().clamp(0.0, max as f32) as usize;
    let hi = (b - 0.5).ceil().clamp(0.0, max as f32) as usize;
    (lo, hi.max(lo))
}

/// 0 to 1 as 0 to 256.
fn fixed(v: f32) -> u32 {
    (v * 256.0).round().clamp(0.0, 256.0) as u32
}

/// 0 to 1 as 0 to 255.
fn byte(v: f32) -> u32 {
    (v * 255.0).round().clamp(0.0, 255.0) as u32
}

/// A quad's tint on row `row` (its centre's place between the quad's top
/// and bottom).
fn row_tint(q: &Quad, row: usize) -> Rgba {
    let [top, bottom] = q.color;
    if top == bottom {
        return top;
    }
    let [_, y0, _, y1] = q.rect;
    let t = ((row as f32 + 0.5 - y0) / (y1 - y0)).clamp(0.0, 1.0);
    [0, 1, 2, 3].map(|k| top[k] + (bottom[k] - top[k]) * t)
}

const PLAIN: u8 = 0;
const MULTIPLY: u8 = 1;
const ADDITIVE: u8 = 2;

fn quad<P: Pixel>(
    q: &Quad,
    px: &mut [P],
    w: usize,
    h: usize,
    textures: &dyn Textures,
    scratch: &mut Scratch,
) {
    let [x0, y0, x1, y1] = q.rect;
    // (Also false for a NaN.)
    if !(x1 > x0 && y1 > y0) {
        return;
    }
    let (cols, rows) = (span(x0, x1, w), span(y0, y1, h));
    if cols.0 >= cols.1 || rows.0 >= rows.1 {
        return;
    }
    if q.texture == Texture::White {
        solid(q, px, w, cols, rows);
        return;
    }
    let Some(img) = textures.image(q.texture).filter(|i| i.is_whole()) else {
        return;
    };
    let tex = Tex {
        w: img.width,
        h: img.height,
        rgba: img.rgba,
    };
    let at = Span { w, cols, rows };
    match q.blend {
        Blend::Plain => textured::<P, PLAIN>(q, &tex, px, at, scratch),
        Blend::Multiply => textured::<P, MULTIPLY>(q, &tex, px, at, scratch),
        Blend::Additive => textured::<P, ADDITIVE>(q, &tex, px, at, scratch),
    }
}

/// A solid fill of the tint.
fn solid<P: Pixel>(
    q: &Quad,
    px: &mut [P],
    w: usize,
    (c0, c1): (usize, usize),
    rows: (usize, usize),
) {
    for row in rows.0..rows.1 {
        let tint = row_tint(q, row);
        let a = fixed(tint[3]);
        if a == 0 {
            continue;
        }
        let c = [byte(tint[0]), byte(tint[1]), byte(tint[2])];
        let line = &mut px[row * w + c0..row * w + c1];
        match q.blend {
            Blend::Plain => {
                let keep = 256 - a;
                let add = c.map(|v| v * a + 128);
                for p in line {
                    let d = p.rgb();
                    *p = P::from_rgb([0, 1, 2].map(|k| (d[k] * keep + add[k]) >> 8));
                }
            }
            Blend::Multiply => {
                let m = c.map(|v| 256 - a + (v * a + 127) / 255);
                for p in line {
                    let d = p.rgb();
                    *p = P::from_rgb([0, 1, 2].map(|k| (d[k] * m[k] + 128) >> 8));
                }
            }
            Blend::Additive => {
                let add = c.map(|v| (v * a + 128) >> 8);
                for p in line {
                    let d = p.rgb();
                    *p = P::from_rgb([0, 1, 2].map(|k| (d[k] + add[k]).min(255)));
                }
            }
        }
    }
}

/// A texture's pixels.
struct Tex<'a> {
    w: usize,
    h: usize,
    rgba: &'a [u8],
}

/// How far a coordinate times its texture's size may miss a whole texel
/// and still be taken as on it: a part's edge stored as k / n in an f32
/// comes back a hair either side of k.
const EDGE_SLACK: f32 = 1e-3;

/// The texels a clamped coordinate range may use: from the one under
/// `a`'s edge to the one under `b`'s, within the texture.
fn texel_range(a: f32, b: f32, n: usize) -> (i64, i64) {
    let last = n as i64 - 1;
    let lo = ((a.min(b) * n as f32 + EDGE_SLACK).floor() as i64).clamp(0, last);
    let hi = ((a.max(b) * n as f32 - EDGE_SLACK).ceil() as i64 - 1).clamp(lo, last);
    (lo, hi)
}

/// A texel coordinate (`i`, in whole texels) on the texture: repeated, or
/// held to `range`.
#[inline]
fn texel(i: i64, n: usize, wrap: bool, range: (i64, i64)) -> usize {
    if wrap {
        i.rem_euclid(n as i64) as usize
    } else {
        i.clamp(range.0, range.1) as usize
    }
}

/// Where a quad's pixels are in the target: the target's width, and the
/// columns and rows it covers.
#[derive(Clone, Copy)]
struct Span {
    w: usize,
    cols: (usize, usize),
    rows: (usize, usize),
}

/// A target column's two texels across (byte offsets in a texture row)
/// and how far it is from the first to the second (of 256).
#[derive(Clone, Copy)]
struct Column {
    a: usize,
    b: usize,
    f: u32,
}

/// What a call keeps from quad to quad: a quad's columns, and two texture
/// rows sampled across at them (by texture row; none is -1).
#[derive(Default)]
struct Scratch {
    columns: Vec<Column>,
    rows: [(i64, Vec<u32>); 2],
}

impl Scratch {
    /// Texture row `r` sampled across into scratch row `k`: premultiplied
    /// or straight.
    fn fill(&mut self, k: usize, r: usize, tex: &Tex, premultiplied: bool) {
        let stride = tex.w * 4;
        let row = &tex.rgba[r * stride..(r + 1) * stride];
        let (columns, out) = (&self.columns, &mut self.rows[k]);
        out.0 = r as i64;
        out.1.clear();
        out.1.extend(columns.iter().map(|c| {
            let (a, b) = (load(row, c.a), load(row, c.b));
            if premultiplied {
                lerp4(premultiply(a), premultiply(b), c.f)
            } else {
                lerp4(a, b, c.f)
            }
        }));
    }

    /// Texture rows `a` and `b` in scratch rows 0 and 1, sampling only
    /// what isn't there already.
    fn pair(&mut self, a: usize, b: usize, tex: &Tex, premultiplied: bool) {
        if self.rows[0].0 != a as i64 {
            if self.rows[1].0 == a as i64 {
                self.rows.swap(0, 1);
            } else {
                self.fill(0, a, tex, premultiplied);
            }
        }
        if self.rows[1].0 != b as i64 {
            self.fill(1, b, tex, premultiplied);
        }
    }
}

/// A texel as a word: red in the low byte, alpha in the high one.
#[inline]
fn load(row: &[u8], x: usize) -> u32 {
    u32::from_le_bytes([row[x], row[x + 1], row[x + 2], row[x + 3]])
}

/// A texel's colour premultiplied by its alpha, rounded to 8 bits.
#[inline]
fn premultiply(t: u32) -> u32 {
    let a = t >> 24;
    let rb = (t & 0x00FF_00FF) * a + 0x0080_0080;
    let rb = ((rb + ((rb >> 8) & 0x00FF_00FF)) >> 8) & 0x00FF_00FF;
    let g = ((t >> 8) & 0xFF) * a + 0x80;
    let g = ((g + (g >> 8)) >> 8) & 0xFF;
    rb | (g << 8) | (a << 24)
}

/// Between two texel words, `f` of the way (of 256), two channels at a
/// time.
#[inline]
fn lerp4(p: u32, q: u32, f: u32) -> u32 {
    let g = 256 - f;
    let rb = (((p & 0x00FF_00FF) * g + (q & 0x00FF_00FF) * f) >> 8) & 0x00FF_00FF;
    let ag = ((((p >> 8) & 0x00FF_00FF) * g + ((q >> 8) & 0x00FF_00FF) * f) >> 8) & 0x00FF_00FF;
    rb | (ag << 8)
}

/// The most texels a coordinate or a step may be (a quad far outside its
/// texture, or squeezed under a pixel, still can't overflow).
const LIMIT: f64 = 1e9;

/// A textured quad, blended by `MODE`.
fn textured<P: Pixel, const MODE: u8>(
    q: &Quad,
    tex: &Tex,
    px: &mut [P],
    Span {
        w,
        cols: (c0, c1),
        rows,
    }: Span,
    scratch: &mut Scratch,
) {
    let [x0, y0, x1, y1] = q.rect;
    let [u0, v0, u1, v1] = q.uv;
    let (tw, th) = (tex.w as f64, tex.h as f64);
    // Texels (16.16 fixed point) under a pixel's centre across, and the
    // step from one pixel to the next; held to sizes that can't overflow.
    let du = (f64::from(u1 - u0) * tw / f64::from(x1 - x0)).clamp(-LIMIT, LIMIT);
    let start = f64::from(u0) * tw - 0.5 + (c0 as f64 + 0.5 - f64::from(x0)) * du;
    let mut fx = (start.clamp(-LIMIT, LIMIT) * 65536.0) as i64;
    let step = (du * 65536.0) as i64;
    let dv = (f64::from(v1 - v0) * th / f64::from(y1 - y0)).clamp(-LIMIT, LIMIT);
    let urange = texel_range(u0, u1, tex.w);
    let vrange = texel_range(v0, v1, tex.h);
    // The same texels across for every row.
    scratch.columns.clear();
    scratch.columns.extend((c0..c1).map(|_| {
        let ix = fx >> 16;
        let f = ((fx >> 8) & 0xFF) as u32;
        fx += step;
        Column {
            a: texel(ix, tex.w, q.wrap[0], urange) * 4,
            b: texel(ix + 1, tex.w, q.wrap[0], urange) * 4,
            f,
        }
    }));
    scratch.rows[0].0 = -1;
    scratch.rows[1].0 = -1;
    let premultiplied = MODE != MULTIPLY;
    for row in rows.0..rows.1 {
        let tint = row_tint(q, row);
        let ta = fixed(tint[3]);
        if ta == 0 {
            continue;
        }
        let t = [fixed(tint[0]), fixed(tint[1]), fixed(tint[2])];
        let untinted = t == [256; 3] && ta == 256;
        let ty = f64::from(v0) * th - 0.5 + (row as f64 + 0.5 - f64::from(y0)) * dv;
        let ty = ty.clamp(-LIMIT, LIMIT);
        let iy = ty.floor();
        let fy = ((ty - iy) * 256.0) as u32;
        let iy = iy as i64;
        let a = texel(iy, tex.h, q.wrap[1], vrange);
        let b = texel(iy + 1, tex.h, q.wrap[1], vrange);
        scratch.pair(a, b, tex, premultiplied);
        let (top, bottom) = (&scratch.rows[0].1, &scratch.rows[1].1);
        let line = &mut px[row * w + c0..row * w + c1];
        for ((p, &hi), &lo) in line.iter_mut().zip(top).zip(bottom) {
            let s = lerp4(hi, lo, fy);
            if MODE != MULTIPLY && s < 0x0100_0000 {
                // Clear (premultiplied, so no colour either).
                continue;
            }
            let d = p.rgb();
            let out = if MODE == MULTIPLY {
                // Straight colour (the texture's alpha isn't used).
                let s = [s & 0xFF, (s >> 8) & 0xFF, (s >> 16) & 0xFF];
                [0, 1, 2].map(|k| {
                    let m = 256 - ta + (((s[k] * t[k]) >> 8) * ta + 127) / 255;
                    (d[k] * m + 128) >> 8
                })
            } else {
                let mut s = [s & 0xFF, (s >> 8) & 0xFF, (s >> 16) & 0xFF, s >> 24];
                if !untinted {
                    for k in 0..3 {
                        s[k] = (s[k] * t[k] * ta + 0x8000) >> 16;
                    }
                    s[3] = (s[3] * ta + 128) >> 8;
                }
                if MODE == ADDITIVE {
                    [0, 1, 2].map(|k| (d[k] + s[k]).min(255))
                } else {
                    let keep = 256 - (s[3] + (s[3] >> 7));
                    [0, 1, 2].map(|k| (((d[k] * keep + 128) >> 8) + s[k]).min(255))
                }
            };
            *p = P::from_rgb(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::ImageRef;

    /// One made-up texture for every textured quad.
    struct One(Vec<u8>, usize, usize);

    impl Textures for One {
        fn image(&self, texture: Texture) -> Option<ImageRef<'_>> {
            (texture == Texture::Art(0)).then_some(ImageRef {
                width: self.1,
                height: self.2,
                rgba: &self.0,
            })
        }
    }

    /// A 2 by 1 texture: opaque red, then half clear green.
    fn texture() -> One {
        One(vec![255, 0, 0, 255, 0, 255, 0, 128], 2, 1)
    }

    fn quad(rect: [f32; 4], blend: Blend, color: Rgba) -> Quad {
        Quad {
            texture: Texture::Art(0),
            rect,
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [color; 2],
            blend,
            wrap: [false; 2],
        }
    }

    fn list(quads: Vec<Quad>, w: u32, h: u32) -> DrawList {
        DrawList {
            size: [w, h],
            quads,
        }
    }

    /// Each channel within one of what it should be.
    fn close(got: u32, want: [f32; 3]) {
        let [r, g, b] = got.rgb();
        for (k, (g, w)) in [r, g, b].into_iter().zip(want).enumerate() {
            assert!(
                (g as f32 - w).abs() <= 1.0,
                "channel {k}: {g} for {w} in {got:06x}"
            );
        }
    }

    const BG: u32 = 0x40_6080;

    #[test]
    fn plain_blends_by_the_textures_alpha_and_the_tint() {
        // Drawn texel for pixel: each pixel's centre on a texel's centre.
        let mut px = vec![BG; 4];
        let l = list(
            vec![quad([1.0, 0.0, 3.0, 1.0], Blend::Plain, [1.0; 4])],
            4,
            1,
        );
        draw_u32(&l, &mut px, 4, 1, &texture());
        assert_eq!(px[0], BG);
        assert_eq!(px[1], 0xFF_0000);
        // Half green over the background.
        let a = 128.0 / 255.0;
        let over = |d: f32, s: f32| d * (1.0 - a) + s * a;
        close(
            px[2],
            [over(64.0, 0.0), over(96.0, 255.0), over(128.0, 0.0)],
        );
        assert_eq!(px[3], BG);
        // Tinted half blue, at half alpha.
        let mut px = vec![BG; 4];
        let l = list(
            vec![quad(
                [1.0, 0.0, 3.0, 1.0],
                Blend::Plain,
                [0.5, 1.0, 1.0, 0.5],
            )],
            4,
            1,
        );
        draw_u32(&l, &mut px, 4, 1, &texture());
        let half = |d: f32, s: f32| d * 0.5 + s * 0.5;
        close(
            px[1],
            [half(64.0, 127.5), half(96.0, 0.0), half(128.0, 0.0)],
        );
        let a = 64.0 / 255.0;
        close(
            px[2],
            [
                64.0 * (1.0 - a),
                96.0 * (1.0 - a) + 255.0 * a,
                128.0 * (1.0 - a),
            ],
        );
    }

    #[test]
    fn multiply_darkens_by_the_textures_colour_ignoring_its_alpha() {
        // Half red, full green, no blue, and clear.
        let colours = One(vec![128, 255, 0, 0], 1, 1);
        let mut px = vec![0xC8_C8C8u32; 2];
        let l = list(
            vec![quad([0.0, 0.0, 1.0, 1.0], Blend::Multiply, [1.0; 4])],
            2,
            1,
        );
        draw_u32(&l, &mut px, 2, 1, &colours);
        close(px[0], [200.0 * 128.0 / 255.0, 200.0, 0.0]);
        assert_eq!(px[1], 0xC8_C8C8);
        // At half alpha, halfway to no change.
        let mut px = vec![0xC8_C8C8u32; 1];
        let l = list(
            vec![quad(
                [0.0, 0.0, 1.0, 1.0],
                Blend::Multiply,
                [1.0, 1.0, 1.0, 0.5],
            )],
            1,
            1,
        );
        draw_u32(&l, &mut px, 1, 1, &One(vec![0, 0, 0, 255], 1, 1));
        close(px[0], [100.0, 100.0, 100.0]);
    }

    #[test]
    fn additive_adds_and_stops_at_white() {
        let mut px = vec![0x80_8080u32; 2];
        let l = list(
            vec![quad([0.0, 0.0, 2.0, 1.0], Blend::Additive, [1.0; 4])],
            2,
            1,
        );
        draw_u32(&l, &mut px, 2, 1, &texture());
        close(px[0], [255.0, 128.0, 128.0]);
        // Half clear green adds half of it.
        close(px[1], [128.0, 128.0 + 128.0, 128.0]);
    }

    #[test]
    fn solid_fills_and_gradients() {
        let fill = |blend, color: Rgba| Quad {
            texture: Texture::White,
            ..quad([0.0, 0.0, 1.0, 1.0], blend, color)
        };
        for (blend, color, want) in [
            (Blend::Plain, [1.0, 0.0, 0.0, 1.0], [255.0, 0.0, 0.0]),
            (Blend::Plain, [1.0, 1.0, 1.0, 0.5], [227.5, 227.5, 227.5]),
            (
                Blend::Multiply,
                [0.5, 1.0, 0.0, 1.0],
                [200.0 * 128.0 / 255.0, 200.0, 0.0],
            ),
            (
                Blend::Multiply,
                [0.0, 0.0, 0.0, 0.25],
                [150.0, 150.0, 150.0],
            ),
            (Blend::Additive, [0.5, 0.0, 1.0, 1.0], [255.0, 200.0, 255.0]),
            (Blend::Additive, [0.2, 0.2, 0.2, 0.5], [225.5, 225.5, 225.5]),
        ] {
            let mut px = vec![0xC8_C8C8u32];
            draw_u32(
                &list(vec![fill(blend, color)], 1, 1),
                &mut px,
                1,
                1,
                &texture(),
            );
            close(px[0], want);
        }
        // Top white to bottom black, four rows.
        let mut q = fill(Blend::Plain, [1.0; 4]);
        q.rect = [0.0, 0.0, 1.0, 4.0];
        q.color = [[1.0; 4], [0.0, 0.0, 0.0, 1.0]];
        let mut px = vec![0u32; 4];
        draw_u32(&list(vec![q], 1, 4), &mut px, 1, 4, &texture());
        for (k, want) in [0.875, 0.625, 0.375, 0.125].into_iter().enumerate() {
            close(px[k], [255.0 * want; 3]);
        }
    }

    #[test]
    fn quads_are_clipped_to_the_target_and_bad_ones_skipped() {
        let mut px = vec![0u32; 9];
        let quads = vec![
            Quad {
                texture: Texture::White,
                ..quad([-100.0, -100.0, 1.5, 100.0], Blend::Plain, [1.0; 4])
            },
            // No pixels for this one, nor for a texture that isn't there,
            // nor for one with no area or a NaN.
            Quad {
                texture: Texture::Art(7),
                ..quad([0.0, 0.0, 3.0, 3.0], Blend::Plain, [1.0; 4])
            },
            quad([2.0, 0.0, 2.0, 3.0], Blend::Plain, [1.0; 4]),
            quad([f32::NAN, 0.0, 3.0, 3.0], Blend::Plain, [1.0; 4]),
            quad([1e30, 1e30, 3e30, 3e30], Blend::Plain, [1.0; 4]),
        ];
        draw_u32(&list(quads.clone(), 3, 3), &mut px, 3, 3, &texture());
        for row in 0..3 {
            assert_eq!(&px[row * 3..row * 3 + 3], [0xFF_FFFF, 0, 0]);
        }
        // A buffer smaller than it says is left alone.
        let mut small = vec![0u32; 4];
        draw_u32(&list(quads, 3, 3), &mut small, 3, 3, &texture());
        assert!(small.iter().all(|&p| p == 0));
    }

    #[test]
    fn wrapped_textures_repeat_and_clamped_ones_stop() {
        // Two repeats of the 2 texel texture across 4 pixels.
        let mut px = vec![0u32; 4];
        let mut q = quad([0.0, 0.0, 4.0, 1.0], Blend::Plain, [1.0; 4]);
        q.uv = [0.0, 0.0, 2.0, 1.0];
        q.wrap = [true, false];
        draw_u32(&list(vec![q], 4, 1), &mut px, 4, 1, &texture());
        assert_eq!(px[0], 0xFF_0000);
        assert_eq!(px[2], 0xFF_0000);
        assert_eq!(px[1], px[3]);
        // Clamped to its first texel's part of the texture, it is all red.
        let mut px = vec![0u32; 4];
        let mut q = quad([0.0, 0.0, 4.0, 1.0], Blend::Plain, [1.0; 4]);
        q.uv = [0.0, 0.0, 0.5, 1.0];
        draw_u32(&list(vec![q], 4, 1), &mut px, 4, 1, &texture());
        assert!(px.iter().all(|&p| p == 0xFF_0000));
        // Scaled up, the middle blends the two texels (red fading into
        // green by green's alpha).
        let mut px = vec![0u32; 4];
        let q = quad([0.0, 0.0, 4.0, 1.0], Blend::Plain, [1.0; 4]);
        draw_u32(&list(vec![q], 4, 1), &mut px, 4, 1, &texture());
        assert_eq!(px[0], 0xFF_0000);
        let [r1, g1, _] = px[1].rgb();
        let [r2, g2, _] = px[2].rgb();
        assert!(r1 > r2 && g1 < g2, "{:06x} {:06x}", px[1], px[2]);
        // Scrolling across only: an edge row between pixels keeps to its
        // own row of the texture (red over blue), not the opposite one.
        let red_over_blue = One(vec![255, 0, 0, 255, 0, 0, 255, 255], 1, 2);
        let mut px = vec![0u32; 2];
        let mut q = quad([0.0, 0.4, 1.0, 2.4], Blend::Plain, [1.0; 4]);
        q.wrap = [true, false];
        draw_u32(&list(vec![q], 1, 2), &mut px, 1, 2, &red_over_blue);
        assert_eq!(px[0], 0xFF_0000);
        // Wrapped down too, it picks up the bottom row.
        q.wrap = [true, true];
        draw_u32(&list(vec![q], 1, 2), &mut px, 1, 2, &red_over_blue);
        assert_ne!(px[0], 0xFF_0000);
    }

    #[test]
    fn a_parts_edges_stay_on_their_texels() {
        // Edges at k / n, as a part's coordinates are stored, give exactly
        // the texels k to m - 1 for every part of textures up to 2048.
        for n in [3usize, 7, 100, 255, 1000, 1303, 2048] {
            for k in 0..n {
                for m in [k + 1, (k + 5).min(n), n] {
                    let (a, b) = (k as f32 / n as f32, m as f32 / n as f32);
                    assert_eq!(
                        texel_range(a, b, n),
                        (k as i64, m as i64 - 1),
                        "{k}..{m} of {n}"
                    );
                }
            }
        }
    }

    /// Counts the texture lookups, so a test sees when a list is drawn.
    struct Counted(One, std::cell::Cell<usize>);

    impl Textures for Counted {
        fn image(&self, texture: Texture) -> Option<ImageRef<'_>> {
            self.1.set(self.1.get() + 1);
            self.0.image(texture)
        }
    }

    #[test]
    fn a_kept_list_is_drawn_once_and_copied() {
        let textures = Counted(texture(), Default::default());
        // The texture's red texel on pixel `x`.
        let red_at = |x: f32| Quad {
            uv: [0.0, 0.0, 0.5, 1.0],
            ..quad([x, 0.0, x + 1.0, 1.0], Blend::Plain, [1.0; 4])
        };
        let red = list(vec![red_at(0.0)], 2, 1);
        let mut kept = Kept::default();
        let mut px = vec![BG; 2];
        kept.draw(&red, &mut px, 2, 1, &textures);
        assert_eq!(px, [0xFF_0000, 0]);
        assert_eq!(textures.1.get(), 1);
        // The same list again: copied, not drawn.
        px.fill(BG);
        kept.draw(&red, &mut px, 2, 1, &textures);
        assert_eq!(px, [0xFF_0000, 0]);
        assert_eq!(textures.1.get(), 1);
        // Another list, another size, or forgotten: drawn again.
        let moved = list(vec![red_at(1.0)], 2, 1);
        kept.draw(&moved, &mut px, 2, 1, &textures);
        assert_eq!(px, [0, 0xFF_0000]);
        let mut wide = vec![BG; 3];
        kept.draw(&moved, &mut wide, 3, 1, &textures);
        assert_eq!(wide, [0, 0xFF_0000, 0]);
        kept.forget();
        kept.draw(&moved, &mut wide, 3, 1, &textures);
        assert_eq!(textures.1.get(), 4);
        // A buffer too small is left alone.
        let mut small = vec![BG; 1];
        kept.draw(&moved, &mut small, 3, 1, &textures);
        assert_eq!(small, [BG]);
    }

    #[test]
    fn words_and_bytes_get_the_same_picture() {
        let quads = vec![
            Quad {
                texture: Texture::White,
                ..quad([0.0, 0.0, 3.0, 2.0], Blend::Plain, [0.1, 0.2, 0.3, 1.0])
            },
            quad([0.5, 0.0, 2.5, 2.0], Blend::Plain, [1.0, 1.0, 1.0, 0.7]),
            quad([0.0, 1.0, 3.0, 2.0], Blend::Additive, [0.5; 4]),
        ];
        let l = list(quads, 3, 2);
        let mut words = vec![0u32; 6];
        let mut bytes = vec![0u8; 24];
        draw_u32(&l, &mut words, 3, 2, &texture());
        draw_rgba(&l, &mut bytes, 3, 2, &texture());
        for (w, b) in words.iter().zip(bytes.chunks(4)) {
            assert_eq!(w.rgb(), [b[0], b[1], b[2]].map(u32::from));
            assert_eq!(b[3], 255);
        }
    }
}
