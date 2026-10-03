//! Short-lived visual effects: bullet holes, impact dust, sparks, muzzle flashes.

use crate::gpu::SpriteVertex;
use glam::Vec3;
use std::collections::VecDeque;

const MAX_DECALS: usize = 128;
const DECAL_LIFE: f32 = 30.0;

struct Decal {
    position: Vec3,
    normal: Vec3,
    size: f32,
    spin: f32,
    age: f32,
}

struct Particle {
    position: Vec3,
    velocity: Vec3,
    age: f32,
    life: f32,
    size: (f32, f32),
    color: [f32; 4],
    gravity: f32,
}

#[derive(Default)]
pub struct Effects {
    decals: VecDeque<Decal>,
    particles: Vec<Particle>,
    rng: u32,
}

impl Effects {
    pub fn new() -> Self {
        Effects {
            rng: 0x1234_5678,
            ..Default::default()
        }
    }

    fn random(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    fn random_unit(&mut self) -> Vec3 {
        Vec3::new(
            self.random() * 2.0 - 1.0,
            self.random() * 2.0 - 1.0,
            self.random() * 2.0 - 1.0,
        )
        .normalize_or(Vec3::Z)
    }

    /// A bullet hitting a wall: a hole, a puff of dust and a few sparks.
    pub fn impact(&mut self, position: Vec3, normal: Vec3) {
        if self.decals.len() >= MAX_DECALS {
            self.decals.pop_front();
        }
        let spin = self.random() * std::f32::consts::TAU;
        let size = 0.012 + self.random() * 0.006;
        self.decals.push_back(Decal {
            position: position + normal * 0.002,
            normal,
            size,
            spin,
            age: 0.0,
        });
        for _ in 0..2 {
            let v = (normal + self.random_unit() * 0.6) * (0.15 + self.random() * 0.15);
            let life = 0.35 + self.random() * 0.2;
            self.particles.push(Particle {
                position: position + normal * 0.01,
                velocity: v,
                age: 0.0,
                life,
                size: (0.02, 0.09),
                color: [0.55, 0.52, 0.48, 0.55],
                gravity: -0.05,
            });
        }
        for _ in 0..4 {
            let v = (normal + self.random_unit() * 0.9).normalize_or(normal)
                * (0.8 + self.random() * 1.2);
            let life = 0.12 + self.random() * 0.1;
            self.particles.push(Particle {
                position: position + normal * 0.01,
                velocity: v,
                age: 0.0,
                life,
                size: (0.008, 0.002),
                color: [1.0, 0.85, 0.45, 1.0],
                gravity: 3.0,
            });
        }
    }

    pub fn update(&mut self, dt: f32) {
        for d in &mut self.decals {
            d.age += dt;
        }
        while self.decals.front().is_some_and(|d| d.age > DECAL_LIFE) {
            self.decals.pop_front();
        }
        for p in &mut self.particles {
            p.age += dt;
            p.velocity.z -= p.gravity * dt;
            p.velocity *= 1.0 - (2.0 * dt).min(0.5);
            p.position += p.velocity * dt;
        }
        self.particles.retain(|p| p.age < p.life);
    }

    /// Triangles for everything alive, facing the camera where it matters.
    pub fn sprites(&self, right: Vec3, up: Vec3) -> Vec<SpriteVertex> {
        let mut out = Vec::new();
        for d in &self.decals {
            let fade = ((DECAL_LIFE - d.age) / 2.0).clamp(0.0, 1.0);
            let t = d.normal.any_orthonormal_vector();
            let b = d.normal.cross(t);
            let (s, c) = d.spin.sin_cos();
            let (r, u) = (t * c + b * s, b * c - t * s);
            quad(
                &mut out,
                d.position,
                r * d.size,
                u * d.size,
                [0.05, 0.045, 0.04, 0.85 * fade],
            );
        }
        for p in &self.particles {
            let t = p.age / p.life;
            let size = p.size.0 + (p.size.1 - p.size.0) * t;
            let mut color = p.color;
            color[3] *= 1.0 - t;
            quad(&mut out, p.position, right * size, up * size, color);
        }
        out
    }
}

/// Two triangles spanning `center ± right ± up`.
pub fn quad(out: &mut Vec<SpriteVertex>, center: Vec3, right: Vec3, up: Vec3, color: [f32; 4]) {
    let v = |p: Vec3, uv: [f32; 2]| SpriteVertex {
        position: p.into(),
        uv,
        color,
    };
    let a = v(center - right - up, [0.0, 1.0]);
    let b = v(center + right - up, [1.0, 1.0]);
    let c = v(center + right + up, [1.0, 0.0]);
    let d = v(center - right + up, [0.0, 0.0]);
    out.extend_from_slice(&[a, b, c, a, c, d]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impacts_leave_decals_that_expire() {
        let mut e = Effects::new();
        e.impact(Vec3::ZERO, Vec3::Z);
        assert!(!e.sprites(Vec3::X, Vec3::Y).is_empty());
        e.update(1.0);
        // Particles are gone, the hole stays.
        assert_eq!(e.sprites(Vec3::X, Vec3::Y).len(), 6);
        e.update(DECAL_LIFE);
        assert!(e.sprites(Vec3::X, Vec3::Y).is_empty());
    }

    #[test]
    fn decal_count_is_bounded() {
        let mut e = Effects::new();
        for _ in 0..MAX_DECALS + 10 {
            e.impact(Vec3::ZERO, Vec3::Z);
        }
        assert_eq!(e.decals.len(), MAX_DECALS);
    }
}
