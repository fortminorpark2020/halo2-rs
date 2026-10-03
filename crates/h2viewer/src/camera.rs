//! Free-fly camera in Halo's coordinate system (z up, 1 unit = 10 feet).

use glam::{Mat4, Vec3};
use std::collections::HashSet;
use winit::keyboard::KeyCode;

pub struct FlyCamera {
    pub position: Vec3,
    /// Radians around +z, 0 = looking down +x.
    yaw: f32,
    /// Radians above the horizon.
    pitch: f32,
}

const SPEED: f32 = 4.0;
const FAST: f32 = 4.0;
const SENSITIVITY: f32 = 0.0025;

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

    pub fn look(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * SENSITIVITY;
        self.pitch = (self.pitch - dy * SENSITIVITY).clamp(-1.55, 1.55);
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

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = glam::camera::rh::view::look_to_mat4(self.position, self.forward(), Vec3::Z);
        // Reversed-Z (near/far swapped) for far better depth precision on big levels.
        let proj =
            glam::camera::rh::proj::directx::perspective(70f32.to_radians(), aspect, 2000.0, 0.05);
        proj * view
    }
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
    fn w_moves_forward() {
        let mut cam = FlyCamera::looking_at(Vec3::ZERO, Vec3::X);
        cam.update(&HashSet::from([KeyCode::KeyW]), 1.0);
        assert!((cam.position - Vec3::new(SPEED, 0.0, 0.0)).length() < 1e-4);
    }
}
