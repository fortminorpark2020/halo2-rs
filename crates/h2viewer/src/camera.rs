//! Free-fly camera in Halo's coordinate system (z up, 1 unit = 10 feet).

use glam::{Mat4, Vec3};
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
const SENSITIVITY: f32 = 0.0025;
/// Vertical field of view: Halo 2's 78 degrees horizontal at 4:3, widened
/// horizontally on wider screens.
pub const FOV_Y: f32 = 62.0;

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

    /// Mouse look; `scale` slows it down while zoomed.
    pub fn look(&mut self, dx: f32, dy: f32, scale: f32) {
        self.yaw -= dx * SENSITIVITY * scale;
        self.pitch = (self.pitch - dy * SENSITIVITY * scale).clamp(-1.55, 1.55);
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

    /// World view-projection, narrowed by a zoom `magnification`.
    pub fn view_proj(&self, aspect: f32, magnification: f32) -> Mat4 {
        projection(aspect, magnification, 0.05, 2000.0) * self.view()
    }
}

/// Reversed-Z perspective (near/far swapped) for far better depth precision.
pub fn projection(aspect: f32, magnification: f32, near: f32, far: f32) -> Mat4 {
    let half = (FOV_Y.to_radians() * 0.5).tan() / magnification.max(1.0);
    glam::camera::rh::proj::directx::perspective(2.0 * half.atan(), aspect, far, near)
}

impl FlyCamera {}

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
}
