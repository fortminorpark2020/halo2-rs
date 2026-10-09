//! Free-fly camera in Halo's coordinate system (z up, 1 unit = 10 feet),
//! the player's view of the world through it, and looking around with
//! the mouse and a controller as Halo 2 does.

use crate::input::{ButtonLayout, StickLayout};
use blam_cache::physics::PlayerControl;
use glam::{Mat4, Vec2, Vec3, Vec4};
use std::collections::HashSet;
use winit::keyboard::KeyCode;

pub struct FlyCamera {
    pub position: Vec3,
    /// Radians around +z, 0 = looking down +x.
    pub yaw: f32,
    /// Radians above the horizon.
    pub pitch: f32,
}

const SPEED: f32 = 4.0;
const FAST: f32 = 4.0;
/// Mouse look, radians per count at the default mouse sensitivity.
const SENSITIVITY: f32 = 0.0025;
/// Widest horizontal field of view, for very wide views (two player
/// splitscreen), so their sides don't stretch.
const MAX_FOV_X: f32 = 100.0;
/// The camera looks no farther up or down than a player can aim
/// (`Spartan::look_with`), so shots always leave through the crosshair.
const MAX_PITCH: f32 = 1.5;

impl FlyCamera {
    pub fn looking_at(position: Vec3, target: Vec3) -> Self {
        let d = (target - position).normalize_or(Vec3::X);
        FlyCamera {
            position,
            yaw: d.y.atan2(d.x),
            pitch: d.z.clamp(-1.0, 1.0).asin(),
        }
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.cos() * self.pitch.cos(),
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
        )
    }

    /// Forward, right and up unit vectors.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let f = self.forward();
        let r = f.cross(Vec3::Z).normalize_or(Vec3::Y);
        (f, r, r.cross(f))
    }

    /// Mouse look; `scale` is the mouse sensitivity, slowed down while
    /// zoomed.
    pub fn look(&mut self, dx: f32, dy: f32, scale: f32) {
        self.turn(Vec2::new(
            -dx * SENSITIVITY * scale,
            -dy * SENSITIVITY * scale,
        ));
    }

    /// Turn by `by` radians: x to the left, y up.
    pub fn turn(&mut self, by: Vec2) {
        self.yaw += by.x;
        self.pitch = (self.pitch + by.y).clamp(-MAX_PITCH, MAX_PITCH);
    }

    pub fn update(&mut self, keys: &HashSet<KeyCode>, dt: f32) {
        let fwd = self.forward();
        let right = fwd.cross(Vec3::Z).normalize_or(Vec3::Y);
        let mut dir = Vec3::ZERO;
        let mut held = |k: KeyCode, v: Vec3| {
            if keys.contains(&k) {
                dir += v;
            }
        };
        held(KeyCode::KeyW, fwd);
        held(KeyCode::KeyS, -fwd);
        held(KeyCode::KeyD, right);
        held(KeyCode::KeyA, -right);
        held(KeyCode::Space, Vec3::Z);
        held(KeyCode::KeyC, -Vec3::Z);
        held(KeyCode::ControlLeft, -Vec3::Z);
        let fast = keys.contains(&KeyCode::ShiftLeft) || keys.contains(&KeyCode::ShiftRight);
        let speed = if fast { SPEED * FAST } else { SPEED };
        self.position += dir.normalize_or_zero() * speed * dt;
    }

    pub fn view(&self) -> Mat4 {
        glam::camera::rh::view::look_to_mat4(self.position, self.forward(), Vec3::Z)
    }

    /// World view-projection through `lens`, narrowed by a zoom
    /// `magnification`.
    pub fn view_proj(&self, lens: &Lens, aspect: f32, magnification: f32) -> Mat4 {
        lens.projection(aspect, magnification, 0.05, 2000.0) * self.view()
    }
}

/// How a view sees the world: Halo 2's field of view, and where on it
/// the crosshair sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    /// Radians across a 4:3 view: the player biped's "camera field of
    /// view", 70 degrees (55.4 up and down). Halo's field of view is
    /// horizontal (as in Halo: Combat Evolved's tags and Halo 2 Vista's
    /// own FOV setting, whose default is 70). A wider view sees more at
    /// the sides; its height stays that of 4:3.
    pub fov: f32,
    /// How far below the middle of the view the crosshair sits, as a
    /// fraction of half its height: the globals' crosshair location,
    /// 0.165 in Halo 2. The view is drawn off centre so that straight
    /// ahead, where shots go, is there.
    pub crosshair: f32,
}

impl Lens {
    /// Halo 2's when the tags don't say: 70 degrees, crosshair in the
    /// middle.
    const FALLBACK_FOV: f32 = 70.0;

    /// A lens of `fov` radians across 4:3 with the crosshair `crosshair`
    /// below the middle (both from the tags; nonsense is left out).
    pub fn new(fov: f32, crosshair: f32) -> Lens {
        let fov = if fov > 0.1 && fov < 3.0 {
            fov
        } else {
            Self::FALLBACK_FOV.to_radians()
        };
        let crosshair = if crosshair.is_finite() {
            crosshair.clamp(-0.9, 0.9)
        } else {
            0.0
        };
        Lens { fov, crosshair }
    }

    /// The same view with the crosshair in the middle (the menus').
    pub fn centred(self) -> Lens {
        Lens {
            crosshair: 0.0,
            ..self
        }
    }

    /// Half the view's height at unit distance (the tangent of half the
    /// vertical field of view), unzoomed.
    pub fn half_height(&self, aspect: f32) -> f32 {
        let widest = (MAX_FOV_X.to_radians() * 0.5).tan() / aspect.max(0.1);
        ((self.fov * 0.5).tan() * 0.75).min(widest)
    }

    /// Reversed-Z perspective (near/far swapped) for far better depth
    /// precision, with straight ahead moved down to the crosshair.
    pub fn projection(&self, aspect: f32, magnification: f32, near: f32, far: f32) -> Mat4 {
        let half = self.half_height(aspect) / magnification.max(1.0);
        let perspective =
            glam::camera::rh::proj::directx::perspective(2.0 * half.atan(), aspect, far, near);
        // Down the screen is -y in clip space: y - crosshair * w.
        let shift = Mat4::from_cols(
            Vec4::X,
            Vec4::Y,
            Vec4::Z,
            Vec4::new(0.0, -self.crosshair, 0.0, 1.0),
        );
        shift * perspective
    }
}

/// A player's look settings and controller layouts, kept in their
/// profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Controls {
    /// A controller's look sensitivity, 1 to 10 (Halo 2's setting).
    pub look_sensitivity: u8,
    /// The mouse's, 1 to 10.
    pub mouse_sensitivity: u8,
    /// Pushing the stick (or the mouse) forward looks down.
    pub invert_look: bool,
    /// What the controller's buttons do.
    pub buttons: ButtonLayout,
    /// Which stick moves and which looks.
    pub sticks: StickLayout,
    /// Controller Vibration: the controller rumbles with the game.
    pub vibration: bool,
    /// Automatic Look Centering: the view levels out as you move forward.
    pub look_centering: bool,
    /// Dual Wield Inversion: the triggers swap guns while dual wielding.
    pub dual_wield_inversion: bool,
}

impl Default for Controls {
    /// Halo 2's: look sensitivity 3, not inverted, its default layouts,
    /// vibration on (its vibration screen picks Enabled for a profile
    /// without the setting), and no look centering or inversion (a guess:
    /// a new profile's flags aren't known).
    fn default() -> Controls {
        Controls {
            look_sensitivity: Controls::DEFAULT_SENSITIVITY,
            mouse_sensitivity: Controls::DEFAULT_SENSITIVITY,
            invert_look: false,
            buttons: ButtonLayout::Default,
            sticks: StickLayout::Default,
            vibration: true,
            look_centering: false,
            dual_wield_inversion: false,
        }
    }
}

impl Controls {
    pub const DEFAULT_SENSITIVITY: u8 = 3;
    pub const MAX_SENSITIVITY: u8 = 10;

    /// How fast a controller turns, as a share of the globals' look rates.
    /// Halo 2 Vista turns 80 + 20s degrees a second across and 40 + 10s
    /// up and down for its sensitivity setting s, counted from 0 (Project
    /// Cartographer's notes on the game): at Halo 2's default of 3 (s = 2)
    /// that's the globals' 120 and 60, so 1 is two thirds of it and 10
    /// over twice.
    pub fn stick_scale(&self) -> f32 {
        (3.0 + self.look_sensitivity.clamp(1, Self::MAX_SENSITIVITY) as f32) / 6.0
    }

    /// How fast the mouse turns, as a share of the remake's 0.0025
    /// radians a count at the default 3. The steps are Halo 2 Vista's for
    /// the mouse, 50 + 20s degrees a second across (Project Cartographer's
    /// notes; s counted from 0): 1 is 0.56 of 3, and 10 over two and a
    /// half times it.
    pub fn mouse_scale(&self) -> f32 {
        (3.0 + 2.0 * self.mouse_sensitivity.clamp(1, Self::MAX_SENSITIVITY) as f32) / 9.0
    }

    /// Turning upside down for inverted look.
    pub fn pitch_sign(&self) -> f32 {
        if self.invert_look {
            -1.0
        } else {
            1.0
        }
    }
}

/// Halo 2's controller look, from the globals' player control: each axis
/// of the stick through the look function to a share of its turn rate,
/// sped up while it's held pegged.
#[derive(Clone, Copy, Debug, Default)]
pub struct StickLook {
    /// Seconds the stick has been held pegged across, and up or down.
    pegged: [f32; 2],
}

impl StickLook {
    /// Radians to turn in `dt` seconds for `stick` (x right, y up): x to
    /// the left, y up. `scale` scales the turn rates (the sensitivity,
    /// and slower zoomed in).
    ///
    /// How the globals' fields combine isn't in the tags; this is the
    /// plain reading of them: the look function's samples are evenly
    /// spaced over the stick's travel, with straight lines between, for
    /// each axis on its own; held past the peg threshold the rate grows
    /// steadily to `scale` times over the acceleration time, and starts
    /// over once the stick comes back under it.
    pub fn turn(&mut self, control: &PlayerControl, stick: Vec2, scale: f32, dt: f32) -> Vec2 {
        let axis = |v: f32, held: &mut f32, rate: f32, (time, most): (f32, f32)| {
            if v.abs() >= control.look_peg_threshold {
                *held += dt;
            } else {
                *held = 0.0;
            }
            let ramp = if time > 0.0 {
                (*held / time).min(1.0)
            } else {
                1.0
            };
            let speed_up = 1.0 + (most - 1.0).max(0.0) * ramp;
            look_function(&control.look_function, v) * rate * speed_up * scale * dt
        };
        let [across, up] = &mut self.pegged;
        let yaw = axis(
            stick.x,
            across,
            control.look_yaw_rate,
            control.yaw_acceleration,
        );
        let pitch = axis(
            stick.y,
            up,
            control.look_pitch_rate,
            control.pitch_acceleration,
        );
        Vec2::new(-yaw, pitch)
    }
}

/// The share of the look rate for a stick at `v` (-1..1): the look
/// function's `samples`, evenly spaced from the middle to the edge, with
/// straight lines between.
pub fn look_function(samples: &[f32], v: f32) -> f32 {
    if samples.len() < 2 {
        return v;
    }
    let x = v.abs().min(1.0) * (samples.len() - 1) as f32;
    let i = (x as usize).min(samples.len() - 2);
    let t = x - i as f32;
    (samples[i] + (samples[i + 1] - samples[i]) * t).copysign(v)
}

/// How much of a controller's look is left with the crosshair `off`
/// radians from an enemy, within a magnetism `angle`: Halo 2's magnetism
/// friction slows it most on them, 1 - `friction` of it, and not at all
/// at the edge. The straight line between is the remake's (the tags give
/// only the strength).
pub fn friction(off: f32, angle: f32, friction: f32) -> f32 {
    if angle <= 0.0 || off >= angle {
        1.0
    } else {
        1.0 - friction.clamp(0.0, 1.0) * (1.0 - off / angle)
    }
}

/// Which way `to` is from `from`: (yaw, pitch) in radians, as `FlyCamera`
/// counts them.
pub fn angles_to(from: Vec3, to: Vec3) -> Vec2 {
    let d = to - from;
    Vec2::new(d.y.atan2(d.x), d.z.atan2(d.truncate().length()))
}

/// The change from angles `was` to `now`, the short way round.
pub fn angle_change(was: Vec2, now: Vec2) -> Vec2 {
    let d = now - was;
    let tau = std::f32::consts::TAU;
    Vec2::new((d.x + tau * 0.5).rem_euclid(tau) - tau * 0.5, d.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looking_at_points_at_target() {
        let cam = FlyCamera::looking_at(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 10.0, 10.0));
        assert!((cam.forward() - Vec3::new(0.0, 1.0, 1.0).normalize()).length() < 1e-5);
    }

    #[test]
    fn basis_is_orthonormal_and_right_handed() {
        let cam = FlyCamera::looking_at(Vec3::ZERO, Vec3::new(1.0, 2.0, 0.5));
        let (f, r, u) = cam.basis();
        assert!((r.cross(f) - u).length() < 1e-5);
        assert!(f.dot(r).abs() < 1e-5);
        assert!(u.z > 0.0);
    }

    #[test]
    fn w_moves_forward() {
        let mut cam = FlyCamera::looking_at(Vec3::ZERO, Vec3::X);
        cam.update(&HashSet::from([KeyCode::KeyW]), 1.0);
        assert!((cam.position - Vec3::new(SPEED, 0.0, 0.0)).length() < 1e-4);
    }

    /// Halo 2's: the biped's 70 degrees and the globals' crosshair.
    fn halo2() -> Lens {
        Lens::new(70f32.to_radians(), 0.165)
    }

    #[test]
    fn the_field_of_view_is_70_degrees_across_4_by_3() {
        let lens = halo2();
        let across = |aspect: f32| 2.0 * (lens.half_height(aspect) * aspect).atan().to_degrees();
        assert!((across(4.0 / 3.0) - 70.0).abs() < 0.01);
        let up = 2.0 * lens.half_height(4.0 / 3.0).atan().to_degrees();
        assert!((up - 55.41).abs() < 0.01, "{up}");
        // Wider screens see more at the sides, as high.
        assert!(
            (across(16.0 / 9.0) - 86.0).abs() < 0.1,
            "{}",
            across(16.0 / 9.0)
        );
        assert_eq!(lens.half_height(16.0 / 9.0), lens.half_height(4.0 / 3.0));
        // Nonsense from the tags falls back to Halo 2's.
        assert_eq!(Lens::new(0.0, f32::NAN), Lens::new(70f32.to_radians(), 0.0));
    }

    #[test]
    fn straight_ahead_is_at_the_lowered_crosshair() {
        let cam = FlyCamera::looking_at(Vec3::ZERO, Vec3::X);
        for magnification in [1.0, 2.0, 9.5] {
            let clip =
                cam.view_proj(&halo2(), 16.0 / 9.0, magnification) * Vec4::new(10.0, 0.0, 0.0, 1.0);
            let ndc = clip.truncate() / clip.w;
            assert!(ndc.x.abs() < 1e-4 && (ndc.y + 0.165).abs() < 1e-4, "{ndc}");
            assert!(ndc.z > 0.0 && ndc.z < 1.0);
        }
        // The menus' camera looks through the middle.
        let clip = cam.view_proj(&halo2().centred(), 1.5, 1.0) * Vec4::new(10.0, 0.0, 0.0, 1.0);
        assert!((clip.y / clip.w).abs() < 1e-4);
    }

    #[test]
    fn the_look_function_follows_the_globals() {
        let f = PlayerControl::default().look_function;
        for (v, want) in [
            (0.0, 0.0),
            (0.2, 0.05),
            (0.4, 0.1),
            (0.5, 0.175),
            (0.6, 0.25),
            (0.8, 0.58),
            (1.0, 1.0),
            (-0.6, -0.25),
        ] {
            assert!((look_function(&f, v) - want).abs() < 1e-5, "{v}");
        }
    }

    const DT: f32 = 1.0 / 60.0;

    /// Degrees a second turning across and up after `secs` of `stick`.
    fn rates_after(stick: Vec2, secs: f32) -> Vec2 {
        let control = PlayerControl::default();
        let mut look = StickLook::default();
        let mut turn = Vec2::ZERO;
        for _ in 0..(secs / DT).round() as usize {
            turn = look.turn(&control, stick, 1.0, DT);
        }
        Vec2::new(-turn.x, turn.y) / DT * 180.0 / std::f32::consts::PI
    }

    #[test]
    fn the_stick_turns_at_halo_2s_rates() {
        // Halo 2's 120 degrees a second across and 60 up at full stick
        // (just pegged), and the look function's share of them.
        let first = rates_after(Vec2::new(0.6, 0.6), DT);
        assert!(
            (first.x - 30.0).abs() < 0.01 && (first.y - 15.0).abs() < 0.01,
            "{first}"
        );
        let full = rates_after(Vec2::ONE, DT);
        assert!(
            (full.x - 120.0 * (1.0 + 1.5 * DT / 0.8)).abs() < 0.01,
            "{full}"
        );
        assert!((full.y * 2.0 - full.x).abs() < 0.01);
        // Held pegged, 2.5 times as fast after 0.8 seconds.
        let held = rates_after(Vec2::X, 1.0);
        assert!((held.x - 300.0).abs() < 0.01, "{held}");
        // Easing off starts it over.
        let control = PlayerControl::default();
        let mut look = StickLook::default();
        for _ in 0..60 {
            look.turn(&control, Vec2::X, 1.0, DT);
        }
        look.turn(&control, Vec2::new(0.5, 0.0), 1.0, DT);
        let again = -look.turn(&control, Vec2::X, 1.0, DT).x / DT;
        assert!(again.to_degrees() < 125.0);
    }

    #[test]
    fn a_full_turn_takes_halo_2s_time() {
        let control = PlayerControl::default();
        let mut look = StickLook::default();
        let (mut turned, mut t) = (0.0f32, 0.0);
        while turned < std::f32::consts::TAU {
            turned -= look.turn(&control, Vec2::X, 1.0, DT).x;
            t += DT;
        }
        assert!((t - 1.45).abs() < 0.03, "{t}");
    }

    #[test]
    fn sensitivity_scales_the_look() {
        let at = |n| Controls {
            look_sensitivity: n,
            mouse_sensitivity: n,
            ..Controls::default()
        };
        assert!((at(1).stick_scale() - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(at(3).stick_scale(), 1.0);
        assert!((at(10).stick_scale() - 13.0 / 6.0).abs() < 1e-6);
        assert_eq!(Controls::default().stick_scale(), 1.0);
        assert_eq!(Controls::default().mouse_scale(), 1.0);
        assert!((at(1).mouse_scale() - 5.0 / 9.0).abs() < 1e-6);
        assert!((at(10).mouse_scale() - 23.0 / 9.0).abs() < 1e-6);
        assert_eq!(at(0).stick_scale(), at(1).stick_scale());
    }

    #[test]
    fn friction_is_strongest_on_the_target() {
        assert!((friction(0.0, 0.1, 0.6) - 0.4).abs() < 1e-6);
        assert!((friction(0.05, 0.1, 0.6) - 0.7).abs() < 1e-6);
        assert_eq!(friction(0.1, 0.1, 0.6), 1.0);
        assert_eq!(friction(0.2, 0.1, 0.6), 1.0);
        assert_eq!(friction(0.0, 0.0, 0.6), 1.0);
    }

    #[test]
    fn the_camera_looks_no_farther_up_than_a_player_aims() {
        let mut cam = FlyCamera::looking_at(Vec3::ZERO, Vec3::X);
        cam.turn(Vec2::new(0.0, 3.0));
        assert_eq!(cam.pitch, MAX_PITCH);
        cam.look(0.0, 1e6, 1.0);
        assert_eq!(cam.pitch, -MAX_PITCH);
    }

    #[test]
    fn angle_changes_go_the_short_way_round() {
        let d = angle_change(Vec2::new(3.1, 0.0), Vec2::new(-3.1, 0.1));
        assert!((d.x - (std::f32::consts::TAU - 6.2)).abs() < 1e-5 && (d.y - 0.1).abs() < 1e-6);
        let a = angles_to(Vec3::ZERO, Vec3::new(0.0, 10.0, 10.0));
        assert!((a.x - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((a.y - std::f32::consts::FRAC_PI_4).abs() < 1e-6);
    }
}
