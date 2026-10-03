//! Lays out Halo 2's HUD widgets (designed for a 640x480 screen) on the window.

use crate::font;
use crate::gpu::{hud_mode, HudBatch, HudVertex};
use crate::scene::HudWidget;
use blam_cache::hud::{Anchor, FLIP_HORIZONTALLY, FLIP_VERTICALLY};

/// Halo 2's HUD blue.
pub const BLUE: [f32; 4] = [0.30, 0.62, 1.0, 0.9];
pub const RED: [f32; 4] = [1.0, 0.25, 0.2, 0.95];
/// HUD blue, dimmed (the grenade type not selected).
pub const DIM_BLUE: [f32; 4] = [0.30, 0.62, 1.0, 0.4];

/// Distance of the corner anchors from the screen edge, in HUD pixels.
const MARGIN: [f32; 2] = [24.0, 20.0];

pub struct HudBuilder {
    w: f32,
    h: f32,
    /// Window pixels per HUD pixel.
    s: f32,
    batches: Vec<HudBatch>,
}

impl HudBuilder {
    pub fn new(w: f32, h: f32) -> Self {
        HudBuilder {
            w,
            h,
            s: h / 480.0,
            batches: Vec::new(),
        }
    }

    pub fn scale(&self) -> f32 {
        self.s
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
        let size = [w.size[0] * self.s, w.size[1] * self.s];
        let x0 = a[0] + w.offset[0] * self.s - w.registration[0] * size[0];
        let y0 = a[1] + w.offset[1] * self.s - w.registration[1] * size[1];
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
        let (qw, qh) = (w.size[0] * self.s, w.size[1] * self.s);
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
        HudWidget {
            name: "w".into(),
            texture: 0,
            anchor,
            flags: 0,
            offset,
            registration,
            size: [70.0, 70.0],
        }
    }

    #[test]
    fn crosshair_is_centred() {
        let h = HudBuilder::new(1280.0, 960.0);
        let r = h.widget_rect(&widget(Anchor::Crosshair, [0.0, 0.0], [0.5, 0.5]));
        assert_eq!(r, [570.0, 410.0, 710.0, 550.0]);
    }

    #[test]
    fn weapon_display_hugs_top_right() {
        let h = HudBuilder::new(640.0, 480.0);
        let r = h.widget_rect(&widget(Anchor::HealthAndShield, [-206.0, 0.0], [0.0, 0.0]));
        assert_eq!(r[0], 640.0 - MARGIN[0] - 206.0);
        assert_eq!(r[1], MARGIN[1]);
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
