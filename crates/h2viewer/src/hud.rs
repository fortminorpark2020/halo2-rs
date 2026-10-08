//! Lays out Halo 2's HUD widgets on the window. Their bitmaps are drawn for
//! a 1280x960 screen: half size on the Xbox's 640x480.

use crate::font;
use crate::gpu::{hud_mode, HudBatch, HudVertex};
use crate::scene::HudWidget;
use blam_cache::hud::{Anchor, ScreenSplit, FLIP_HORIZONTALLY, FLIP_VERTICALLY};

/// Halo 2's HUD blue.
pub const BLUE: [f32; 4] = [0.30, 0.62, 1.0, 0.9];
pub const RED: [f32; 4] = [1.0, 0.25, 0.2, 0.95];
/// HUD blue, dimmed (the grenade type not selected).
pub const DIM_BLUE: [f32; 4] = [0.30, 0.62, 1.0, 0.4];

/// Distance of the corner anchors from the screen edge, in pixels of a
/// 640x480 screen.
const MARGIN: [f32; 2] = [24.0, 20.0];
/// The remake's own HUD text is never smaller than this, in window pixels
/// per pixel of a 1280x960 screen: a window pixel to each pixel of its 5x7
/// font in the smallest lines (the kill feed's, 8 pixels tall), which a
/// 720p window or a splitscreen view would otherwise draw too small to read.
const MIN_TEXT_SCALE: f32 = 1.0;

pub struct HudBuilder {
    w: f32,
    h: f32,
    /// Window pixels per pixel of a 640x480 screen: the size of the
    /// corner margins.
    s: f32,
    /// Window pixels per pixel of a 1280x960 screen for the remake's own
    /// text (and how far from where it's placed it sits).
    text_scale: f32,
    /// Which of the HUD tags' layouts this view draws.
    split: ScreenSplit,
    /// Window pixels per pixel of a HUD widget's bitmap.
    bitmap_scale: f32,
    /// Window pixels per unit of a HUD widget's offset.
    offset_scale: f32,
    /// Text gets a dark shadow, so small HUD lines read over bright snow,
    /// sky or walls (menus have their own dark backing).
    shadow: bool,
    batches: Vec<HudBatch>,
}

impl HudBuilder {
    /// A HUD (or menu) filling a `w` x `h` window.
    pub fn new(w: f32, h: f32) -> Self {
        HudBuilder {
            shadow: false,
            ..Self::for_view(w, h, ScreenSplit::Full)
        }
    }

    /// The HUD of a `w` x `h` view, which draws the widgets' `split` layout.
    pub fn for_view(w: f32, h: f32, split: ScreenSplit) -> Self {
        // Halo 2 Vista multiplies a widget's bitmap size and its offset by
        // one HUD scale (Project Cartographer's rebuild of its HUD code),
        // window height / 960 for the crosshair ("resolution_height *
        // 0.0010416667", in Cartographer's notes on the game). That is
        // half size on the Xbox's 640x480, which is what the tags fit: the
        // beam rifle's zoomed meters sit 453 pixels left of the crosshair.
        // A splitscreen view is half the window's height. Its bitmaps are
        // drawn at the same window height / 960, but its offsets count
        // double: half screen widgets have half the full screen numbers
        // (-103 for a 206 pixel wide weapon box that ends at its anchor),
        // and only then do a scope's left and right ticks mirror around
        // the crosshair.
        let (bitmap_scale, offset_scale) = match split {
            ScreenSplit::Full => (h / 960.0, h / 960.0),
            ScreenSplit::Half | ScreenSplit::Quarter => (h / 480.0, h / 240.0),
        };
        HudBuilder {
            w,
            h,
            s: h / 480.0,
            // Halo 2's tags don't say how big its HUD text is (each text
            // widget names a font, which isn't in the maps), so the remake's
            // own text goes at the art's scale.
            text_scale: bitmap_scale.max(MIN_TEXT_SCALE),
            split,
            bitmap_scale,
            offset_scale,
            shadow: true,
            batches: Vec::new(),
        }
    }

    pub fn scale(&self) -> f32 {
        self.s
    }

    /// Window pixels per pixel of a 1280x960 screen for the remake's own
    /// HUD text: the HUD art's scale, but never less than `MIN_TEXT_SCALE`.
    pub fn text_scale(&self) -> f32 {
        self.text_scale
    }

    /// Which of the HUD tags' layouts this view draws.
    pub fn split(&self) -> ScreenSplit {
        self.split
    }

    pub fn anchor(&self, a: Anchor) -> [f32; 2] {
        let (mx, my) = (MARGIN[0] * self.s, MARGIN[1] * self.s);
        match a {
            // Halo 2 puts the weapon display top right, grenades top left and
            // the shield meter over the motion tracker in the bottom left.
            Anchor::HealthAndShield => [self.w - mx, my],
            Anchor::WeaponHud => [mx, my],
            Anchor::MotionSensor => [mx, self.h - my],
            Anchor::Scoreboard => [self.w - mx, self.h - my],
            Anchor::Crosshair | Anchor::LockOnTarget | Anchor::Other(_) => {
                [self.w * 0.5, self.h * 0.5]
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn quad(
        &mut self,
        texture: usize,
        rect: [f32; 4],
        uv: [f32; 4],
        color: [f32; 4],
        mode: f32,
        value: f32,
    ) {
        let [x0, y0, x1, y1] = rect;
        let [u0, v0, u1, v1] = uv;
        let v = |x, y, u, t| HudVertex {
            position: [x, y],
            uv: [u, t],
            color,
            mode: [mode, value],
        };
        let corners = [
            v(x0, y0, u0, v0),
            v(x0, y1, u0, v1),
            v(x1, y1, u1, v1),
            v(x0, y0, u0, v0),
            v(x1, y1, u1, v1),
            v(x1, y0, u1, v0),
        ];
        match self.batches.last_mut() {
            Some(b) if b.texture == texture => b.vertices.extend_from_slice(&corners),
            _ => self.batches.push(HudBatch {
                texture,
                vertices: corners.to_vec(),
            }),
        }
    }

    /// Screen rectangle of a widget.
    pub fn widget_rect(&self, w: &HudWidget) -> [f32; 4] {
        let a = self.anchor(w.anchor);
        let size = w.size.map(|p| p * self.bitmap_scale);
        let x0 = a[0] + w.offset[0] * self.offset_scale - w.registration[0] * size[0];
        let y0 = a[1] + w.offset[1] * self.offset_scale - w.registration[1] * size[1];
        [x0, y0, x0 + size[0], y0 + size[1]]
    }

    pub fn widget(&mut self, w: &HudWidget, color: [f32; 4], mode: f32, value: f32) {
        let mut uv = [0.0, 0.0, 1.0, 1.0];
        if w.flags & FLIP_HORIZONTALLY != 0 {
            uv.swap(0, 2);
        }
        if w.flags & FLIP_VERTICALLY != 0 {
            uv.swap(1, 3);
        }
        let rect = self.widget_rect(w);
        self.quad(w.texture, rect, uv, color, mode, value);
    }

    /// A scope mask: one quadrant mirrored four ways around the centre, with
    /// the rest of the screen filled in the mask's outer colour.
    pub fn scope(&mut self, w: &HudWidget, white: usize, fill: [f32; 4]) {
        let [cx, cy] = self.anchor(w.anchor);
        let [qw, qh] = w.size.map(|p| p * self.bitmap_scale);
        let black = [0.0, 0.0, 0.0, 1.0];
        for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let (x0, x1) = if sx < 0.0 {
                (cx - qw, cx)
            } else {
                (cx, cx + qw)
            };
            let (y0, y1) = if sy < 0.0 {
                (cy - qh, cy)
            } else {
                (cy, cy + qh)
            };
            let uv = [
                if sx < 0.0 { 0.0 } else { 1.0 },
                if sy < 0.0 { 0.0 } else { 1.0 },
                if sx < 0.0 { 1.0 } else { 0.0 },
                if sy < 0.0 { 1.0 } else { 0.0 },
            ];
            self.quad(w.texture, [x0, y0, x1, y1], uv, black, hud_mode::PLAIN, 0.0);
        }
        let full = [0.0, 0.0, 1.0, 1.0];
        let (l, r, t, b) = (cx - qw, cx + qw, cy - qh, cy + qh);
        for rect in [
            [0.0, 0.0, self.w, t],
            [0.0, b, self.w, self.h],
            [0.0, t, l, b],
            [r, t, self.w, b],
        ] {
            if rect[2] > rect[0] && rect[3] > rect[1] {
                self.quad(white, rect, full, fill, hud_mode::PLAIN, 0.0);
            }
        }
    }

    /// Text centred on `x` with its top at `y`, `height` window pixels tall.
    pub fn text(
        &mut self,
        font_texture: usize,
        [x, y]: [f32; 2],
        height: f32,
        text: &str,
        color: [f32; 4],
    ) {
        if self.shadow {
            // One font pixel down and right, at least one window pixel.
            let d = (height / 8.0).max(1.0);
            let dark = [0.0, 0.0, 0.0, color[3] * 0.6];
            self.glyphs(font_texture, [x + d, y + d], height, text, dark);
        }
        self.glyphs(font_texture, [x, y], height, text, color);
    }

    fn glyphs(
        &mut self,
        font_texture: usize,
        [x, y]: [f32; 2],
        height: f32,
        text: &str,
        color: [f32; 4],
    ) {
        let cw = height * font::ASPECT;
        let width = cw * text.chars().count() as f32;
        let mut x = x - width * 0.5;
        for c in text.chars() {
            if let Some(uv) = font::glyph_uv(c) {
                self.quad(
                    font_texture,
                    [x, y, x + cw, y + height],
                    uv,
                    color,
                    hud_mode::PLAIN,
                    0.0,
                );
            }
            x += cw;
        }
    }

    /// Text starting at `x` with its top at `y`.
    pub fn text_left(
        &mut self,
        font_texture: usize,
        [x, y]: [f32; 2],
        height: f32,
        text: &str,
        color: [f32; 4],
    ) {
        let width = height * font::ASPECT * text.chars().count() as f32;
        self.text(font_texture, [x + width * 0.5, y], height, text, color);
    }

    pub fn finish(self) -> Vec<HudBatch> {
        self.batches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget(anchor: Anchor, offset: [f32; 2], registration: [f32; 2]) -> HudWidget {
        sized(anchor, offset, registration, [70.0, 70.0])
    }

    fn sized(
        anchor: Anchor,
        offset: [f32; 2],
        registration: [f32; 2],
        size: [f32; 2],
    ) -> HudWidget {
        HudWidget {
            name: "w".into(),
            texture: 0,
            anchor,
            flags: 0,
            offset,
            registration,
            size,
        }
    }

    #[test]
    fn crosshair_is_centred() {
        // The battle rifle's 70 pixel reticle, one to one at 1280x960.
        let h = HudBuilder::new(1280.0, 960.0);
        let r = h.widget_rect(&widget(Anchor::Crosshair, [0.0, 0.0], [0.5, 0.5]));
        assert_eq!(r, [605.0, 445.0, 675.0, 515.0]);
    }

    #[test]
    fn weapon_display_hugs_top_right() {
        // Half size on a 640x480 screen.
        let h = HudBuilder::new(640.0, 480.0);
        let r = h.widget_rect(&widget(Anchor::HealthAndShield, [-206.0, 0.0], [0.0, 0.0]));
        assert_eq!(r[0], 640.0 - MARGIN[0] - 103.0);
        assert_eq!(r[1], MARGIN[1]);
    }

    /// The battle rifle's zoom ticks either side of the crosshair, from
    /// lockout.map: the right one is the left one flipped.
    fn zoom_ticks(h: &HudBuilder, left: f32, right: f32, size: [f32; 2]) -> ([f32; 4], [f32; 4]) {
        let tick = |x| sized(Anchor::Crosshair, [x, 0.0], [0.0, 0.0], size);
        (h.widget_rect(&tick(left)), h.widget_rect(&tick(right)))
    }

    #[test]
    fn zoom_ticks_mirror_in_every_layout() {
        let full = HudBuilder::new(1920.0, 1080.0);
        let (l, r) = zoom_ticks(&full, -218.0, 40.0, [178.0, 44.0]);
        assert_eq!((960.0 - l[0], 960.0 - l[2]), (r[2] - 960.0, r[0] - 960.0));
        // Two players at 1920x1080: the half screen art, the same size on
        // screen as full screen art, but with offsets that count double.
        let half = HudBuilder::for_view(1920.0, 540.0, ScreenSplit::Half);
        let (l, r) = zoom_ticks(&half, -58.0, 11.0, [94.0, 28.0]);
        assert_eq!((960.0 - l[0], 960.0 - l[2]), (r[2] - 960.0, r[0] - 960.0));
        assert_eq!(l[2] - l[0], 94.0 * 1080.0 / 960.0);
    }

    #[test]
    fn split_weapon_display_ends_at_its_anchor() {
        // The half and quarter screen weapon box: 206 pixels wide, 103 to
        // the left of the anchor.
        let h = HudBuilder::for_view(960.0, 540.0, ScreenSplit::Quarter);
        let r = h.widget_rect(&sized(
            Anchor::HealthAndShield,
            [-103.0, 0.0],
            [0.0, 0.0],
            [206.0, 38.0],
        ));
        assert_eq!(r[2], h.anchor(Anchor::HealthAndShield)[0]);
        assert_eq!(r[2] - r[0], 206.0 * 1080.0 / 960.0);
    }

    #[test]
    fn quads_with_one_texture_share_a_batch() {
        let mut h = HudBuilder::new(640.0, 480.0);
        h.text(3, [0.0, 0.0], 10.0, "12", BLUE);
        h.quad(4, [0.0; 4], [0.0; 4], BLUE, 0.0, 0.0);
        let b = h.finish();
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].vertices.len(), 12);
    }
}
