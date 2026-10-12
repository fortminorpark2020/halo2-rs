//! How the menu scene is seen: Halo's coordinate system (z up, 1 unit =
//! 10 feet), a view matrix that banks with the flythrough's roll, and
//! Halo 2's lens with its reversed-Z projection. The part of h2viewer's
//! camera the menus need (menu-preview's `camera.rs`); no input.

use crate::flythrough::{Flythrough, FOV};
use glam::{DQuat, Mat4, Vec3, Vec4};

/// Widest horizontal field of view, for very wide views, so their sides
/// don't stretch.
const MAX_FOV_X: f32 = 100.0;
/// The menu scene's near and far planes (h2viewer's): the scene spans
/// about 470 units across.
pub const NEAR: f32 = 0.05;
pub const FAR: f32 = 2000.0;

/// How Halo 2 turns a camera point or a placed object by its three
/// angles (radians): yaw about +z first, then pitch (raising +x), and the
/// roll last, about the world's +x. Its `camera_set` (0x16c2f0) and the
/// code that places scenario objects (0x0b7300) both make a point's
/// forward and up this way, through 0x11df60 and 0x141ce0. The menu's
/// camera points come out level or nearly so, where turning the roll
/// about the camera's own forward would bank them up to 45 degrees.
pub fn halo_rotation([yaw, pitch, roll]: [f64; 3]) -> DQuat {
    DQuat::from_rotation_x(roll) * DQuat::from_rotation_y(-pitch) * DQuat::from_rotation_z(yaw)
}

/// The view matrix of a camera at `at` facing `forward` with `up` as its
/// up (it need not be +z: the flythrough banks). Right-handed.
pub fn look_to(at: Vec3, forward: Vec3, up: Vec3) -> Mat4 {
    glam::camera::rh::view::look_to_mat4(at, forward, up)
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
    /// fraction of half its height (0.165 in Halo 2's globals). The menus'
    /// view is centred (0).
    pub crosshair: f32,
}

impl Lens {
    /// Halo 2's when the tags don't say: 70 degrees, crosshair in the
    /// middle.
    const FALLBACK_FOV: f32 = 70.0;

    /// A lens of `fov` radians across 4:3 with the crosshair `crosshair`
    /// below the middle (nonsense is left out).
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

    /// The menus' lens: Halo 2's 70 degrees, looking through the middle.
    pub fn menu() -> Lens {
        Lens::new(Self::FALLBACK_FOV.to_radians(), 0.0)
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

    /// The zoom that gives a horizontal field of view of `fov` degrees (a
    /// cutscene camera's, or the menus' flythrough's) at this aspect.
    pub fn fov_magnification(&self, aspect: f32, fov: f32) -> f32 {
        let half_x = (self.half_height(aspect) * aspect).atan();
        half_x.tan() / (fov.to_radians() * 0.5).tan().max(1e-3)
    }

    /// Reversed-Z perspective (near/far swapped) for far better depth
    /// precision, with straight ahead moved down to the crosshair. Depth
    /// is 1 at `near` and 0 at `far`: clear depth to 0 and keep the
    /// greater.
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

/// Where the flythrough's camera is `seconds` into the menus, and its
/// view-projection for a view `aspect` wide (width over height): the
/// menus' lens zoomed to the flythrough's field of view, as h2viewer drew
/// it.
pub fn menu_view_proj(fly: &Flythrough, seconds: f32, aspect: f32) -> (Vec3, Mat4) {
    let (at, forward, up) = fly.camera(seconds);
    let lens = Lens::menu();
    let zoom = lens.fov_magnification(aspect, FOV);
    let view_proj = lens.projection(aspect, zoom, NEAR, FAR) * look_to(at, forward, up);
    (at, view_proj)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Halo 2's: the biped's 70 degrees and the globals' crosshair.
    fn halo2() -> Lens {
        Lens::new(70f32.to_radians(), 0.165)
    }

    #[test]
    fn points_turn_as_halo_2_turns_them() {
        // Halo 2's forward, left and up for yaw a, pitch b, roll c
        // (0x141ce0, in our words).
        let (a, b, c) = (0.7f64, -0.3f64, 1.1f64);
        let ((sa, ca), (sb, cb), (sc, cc)) = (a.sin_cos(), b.sin_cos(), c.sin_cos());
        let forward = glam::DVec3::new(ca * cb, sa * cc - sb * sc * ca, sb * cc * ca + sa * sc);
        let left = glam::DVec3::new(-sa * cb, sb * sc * sa + ca * cc, ca * sc - sb * cc * sa);
        let up = glam::DVec3::new(-sb, -cb * sc, cb * cc);
        let rot = halo_rotation([a, b, c]);
        assert!((rot * glam::DVec3::X).distance(forward) < 1e-12);
        assert!((rot * glam::DVec3::Y).distance(left) < 1e-12);
        assert!((rot * glam::DVec3::Z).distance(up) < 1e-12);
        // One angle at a time: yawed a quarter turn, +x goes to +y;
        // pitched up, +x rises; rolled, up leans to -y.
        let quarter = std::f64::consts::FRAC_PI_2;
        let yawed = halo_rotation([quarter, 0.0, 0.0]) * glam::DVec3::X;
        assert!(yawed.distance(glam::DVec3::Y) < 1e-12);
        assert!((halo_rotation([0.0, 0.5, 0.0]) * glam::DVec3::X).z > 0.4);
        assert!((halo_rotation([0.0, 0.0, 0.5]) * glam::DVec3::Z).y < -0.4);
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
        assert_eq!(Lens::menu(), halo2().centred());
    }

    #[test]
    fn straight_ahead_is_at_the_lowered_crosshair() {
        let view = look_to(Vec3::ZERO, Vec3::X, Vec3::Z);
        for magnification in [1.0, 2.0, 9.5] {
            let clip = halo2().projection(16.0 / 9.0, magnification, NEAR, FAR)
                * view
                * Vec4::new(10.0, 0.0, 0.0, 1.0);
            let ndc = clip.truncate() / clip.w;
            assert!(ndc.x.abs() < 1e-4 && (ndc.y + 0.165).abs() < 1e-4, "{ndc}");
            assert!(ndc.z > 0.0 && ndc.z < 1.0);
        }
        // The menus' camera looks through the middle.
        let clip =
            Lens::menu().projection(1.5, 1.0, NEAR, FAR) * view * Vec4::new(10.0, 0.0, 0.0, 1.0);
        assert!((clip.y / clip.w).abs() < 1e-4);
    }

    #[test]
    fn depth_is_reversed() {
        let proj = Lens::menu().projection(16.0 / 9.0, 1.0, NEAR, FAR);
        let view = look_to(Vec3::ZERO, Vec3::X, Vec3::Z);
        let depth = |d: f32| {
            let c = proj * view * Vec4::new(d, 0.0, 0.0, 1.0);
            c.z / c.w
        };
        assert!((depth(NEAR) - 1.0).abs() < 1e-4);
        assert!(depth(FAR).abs() < 1e-4);
        assert!(depth(10.0) > depth(100.0));
    }

    #[test]
    fn the_flythrough_sees_70_degrees_across_wide_screens() {
        // At 16:9 the menus zoom in from the 4:3 lens to keep 70 degrees
        // across; at 4:3 they need no zoom.
        let lens = Lens::menu();
        assert!((lens.fov_magnification(4.0 / 3.0, FOV) - 1.0).abs() < 1e-5);
        let wide = 16.0 / 9.0;
        let zoom = lens.fov_magnification(wide, FOV);
        let half = lens.half_height(wide) / zoom;
        let across = 2.0 * (half * wide).atan().to_degrees();
        assert!((across - FOV).abs() < 0.01, "{across}");
    }

    #[test]
    fn a_banked_camera_tilts_the_view() {
        // Facing +x with up leaning towards +y (rolled to the left): a
        // point straight up in the world shows up and to the right, as a
        // pole does with your head tilted left.
        let up = Vec3::new(0.0, 0.5, 1.0).normalize();
        let view = look_to(Vec3::ZERO, Vec3::X, up);
        let proj = Lens::menu().projection(1.0, 1.0, NEAR, FAR);
        let clip = proj * view * Vec4::new(10.0, 0.0, 3.0, 1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(ndc.x > 0.05 && ndc.y > 0.05, "{ndc}");
    }
}
