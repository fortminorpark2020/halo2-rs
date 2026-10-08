//! Short-lived visual effects: bullet holes, impact dust, sparks, muzzle
//! flashes, and the trails rounds leave (tracers).

use crate::gpu::SpriteVertex;
use blam_cache::effect::Contrail;
use glam::Vec3;
use std::collections::VecDeque;

const MAX_DECALS: usize = 128;
const DECAL_LIFE: f32 = 30.0;
/// Most tracers alive at once; the oldest go first.
const MAX_RIBBONS: usize = 256;
/// Most stretches of flying rounds' trails alive at once. A stretch is laid
/// every frame for every round, so how many are alive grows with the frame
/// rate (a Needler's stream at 144 frames a second keeps about 1100); they
/// are kept apart from the tracers so they never push those out, and this
/// is only a bound on memory.
const MAX_TRAIL_STRETCHES: usize = 8192;
/// Pieces each tracer is drawn in, so its colour and width can change
/// along it. A trail's stretch, one frame's flight, is one piece.
const RIBBON_PIECES: usize = 8;

/// A tag's colour (gamma-encoded, as Halo 2 stores them) in the linear
/// light sprites blend in, so it shows on screen as the tag gives it.
pub fn linear([r, g, b]: [f32; 3]) -> [f32; 3] {
    [r, g, b].map(|v| v.max(0.0).powf(2.2))
}

/// One state of a trail's points, with its times picked in the tag's
/// ranges.
#[derive(Clone, Copy, Debug)]
struct RibbonState {
    /// Seconds a point stays in it, then takes to become the next.
    hold: f32,
    fade: f32,
    width: f32,
    color: [f32; 4],
}

/// A contrail laid along a straight line: a tracer from the muzzle to what
/// the shot hit, or a stretch of a flying round's trail. Its head moves
/// along at `speed`, and the points it lays age through the contrail's
/// states.
struct Ribbon {
    from: Vec3,
    to: Vec3,
    /// World units a second (0: all there at once).
    speed: f32,
    states: Vec<RibbonState>,
    age: f32,
    /// Pieces it's drawn in.
    pieces: usize,
}

impl Ribbon {
    /// Seconds a point lasts: through every state to the last.
    fn life(&self) -> f32 {
        let n = self.states.len().saturating_sub(1);
        self.states[..n].iter().map(|s| s.hold + s.fade).sum()
    }

    /// Seconds the head takes from end to end.
    fn travel(&self) -> f32 {
        match self.speed > 0.0 {
            true => self.from.distance(self.to) / self.speed,
            false => 0.0,
        }
    }

    /// A point's width and colour `age` seconds after it was laid, while
    /// it lasts.
    fn look(&self, age: f32) -> Option<(f32, [f32; 4])> {
        let mut t = age.max(0.0);
        for w in self.states.windows(2) {
            let (a, b) = (w[0], w[1]);
            if t < a.hold {
                return Some((a.width, a.color));
            }
            t -= a.hold;
            if t < a.fade {
                let k = t / a.fade;
                let color = std::array::from_fn(|i| a.color[i] + (b.color[i] - a.color[i]) * k);
                return Some((a.width + (b.width - a.width) * k, color));
            }
            t -= a.fade;
        }
        None
    }
}

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
    ribbons: VecDeque<Ribbon>,
    /// Flying rounds' trails, a stretch a frame.
    trails: VecDeque<Ribbon>,
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

    /// A grenade going off: a flash, a fireball (blue for plasma), sparks
    /// and smoke.
    pub fn explosion(&mut self, position: Vec3, plasma: bool) {
        let (core, edge) = fireball(plasma);
        self.blast(position, core, edge, !plasma, 1.0);
    }

    /// Something going off: a flash, a fireball in `edge` colours, sparks
    /// and (`smoke`) smoke, `scale` times grenade-sized.
    pub fn blast(
        &mut self,
        position: Vec3,
        core: [f32; 4],
        edge: [f32; 4],
        smoke: bool,
        scale: f32,
    ) {
        self.particles.push(Particle {
            position,
            velocity: Vec3::ZERO,
            age: 0.0,
            life: 0.18,
            size: (0.6 * scale, 1.4 * scale),
            color: core,
            gravity: 0.0,
        });
        for _ in 0..14 {
            let v = self.random_unit() * (1.0 + self.random() * 2.0) * scale;
            let life = 0.25 + self.random() * 0.25;
            self.particles.push(Particle {
                position: position + v * 0.1,
                velocity: v,
                age: 0.0,
                life,
                size: (0.25 * scale, 0.6 * scale),
                color: edge,
                gravity: -0.5,
            });
        }
        for _ in 0..20 {
            let v = self.random_unit() * (3.0 + self.random() * 5.0) * scale;
            let life = 0.3 + self.random() * 0.4;
            self.particles.push(Particle {
                position,
                velocity: v,
                age: 0.0,
                life,
                size: (0.02, 0.005),
                color: core,
                gravity: 4.0,
            });
        }
        if smoke {
            for _ in 0..10 {
                let v = (self.random_unit() + Vec3::Z * 0.5) * (0.3 + self.random() * 0.6) * scale;
                let life = 1.5 + self.random() * 1.5;
                self.particles.push(Particle {
                    position: position + v * 0.3,
                    velocity: v,
                    age: 0.0,
                    life,
                    size: (0.3 * scale, 1.0 * scale),
                    color: [0.3, 0.29, 0.27, 0.5],
                    gravity: -0.15,
                });
            }
        }
    }

    /// A badly damaged vehicle: smoke rising from it over `dt` seconds,
    /// and (`burning`) flames.
    pub fn smolder(&mut self, position: Vec3, burning: bool, dt: f32) {
        let puffs = if burning { 14.0 } else { 7.0 } * dt;
        let count = puffs as usize + (self.random() < puffs.fract()) as usize;
        for _ in 0..count {
            let drift = self.random_unit() * 0.15;
            let v = Vec3::new(drift.x, drift.y, 0.5 + self.random() * 0.4);
            let shade = if burning { 0.12 } else { 0.35 };
            let (life, flame) = (1.6 + self.random(), 0.25 + self.random() * 0.15);
            self.particles.push(Particle {
                position: position + drift,
                velocity: v,
                age: 0.0,
                life,
                size: (0.2, 0.9),
                color: [shade, shade * 0.97, shade * 0.94, 0.45],
                gravity: -0.3,
            });
            if burning {
                self.particles.push(Particle {
                    position: position + drift * 0.5,
                    velocity: v * 0.6,
                    age: 0.0,
                    life: flame,
                    size: (0.1, 0.03),
                    color: [1.0, 0.5, 0.12, 0.7],
                    gravity: -0.5,
                });
            }
        }
    }

    /// A plasma bolt or needle splashing on a wall: a scorch and a few
    /// glowing sparks of its colour.
    pub fn splash(&mut self, position: Vec3, normal: Vec3, color: [f32; 4]) {
        self.impact(position, normal);
        for _ in 0..6 {
            let v = (normal + self.random_unit() * 0.8) * (0.4 + self.random() * 0.8);
            let life = 0.15 + self.random() * 0.15;
            self.particles.push(Particle {
                position: position + normal * 0.02,
                velocity: v,
                age: 0.0,
                life,
                size: (0.05, 0.01),
                color,
                gravity: 1.0,
            });
        }
    }

    /// A round in flight this frame: its glow, and what it leaves behind
    /// (smoke if it burns; a fading streak if plasma and `streak`, as
    /// rounds with a contrail leave that instead).
    pub fn round(&mut self, position: Vec3, color: [f32; 4], size: f32, fiery: bool, streak: bool) {
        // Gone at the next update: drawn this frame only.
        self.particles.push(Particle {
            position,
            velocity: Vec3::ZERO,
            age: 0.0,
            life: 1e-4,
            size: (size, size),
            color,
            gravity: 0.0,
        });
        if !fiery && !streak {
            return;
        }
        let (life, trail, gravity) = if fiery {
            (
                0.9,
                ([0.6, 0.58, 0.55, 0.45], (size * 0.6, size * 3.0)),
                -0.1,
            )
        } else {
            (
                0.12,
                (
                    [color[0], color[1], color[2], 0.6],
                    (size * 0.8, size * 0.3),
                ),
                0.0,
            )
        };
        self.particles.push(Particle {
            position,
            velocity: Vec3::ZERO,
            age: 0.0,
            life,
            size: trail.1,
            color: trail.0,
            gravity,
        });
    }

    /// A shot hitting a player whose shields are down: blood. (Shields
    /// taking a hit flare over the body instead; see `ShieldFlares`.)
    pub fn blood(&mut self, position: Vec3) {
        for _ in 0..6 {
            let v = self.random_unit() * (0.4 + self.random() * 0.6);
            let life = 0.15 + self.random() * 0.15;
            self.particles.push(Particle {
                position,
                velocity: v,
                age: 0.0,
                life,
                size: (0.03, 0.06),
                color: [0.55, 0.05, 0.03, 0.9],
                gravity: 3.0,
            });
        }
    }

    /// Shields knocked out: a burst of sparks of their colour from the
    /// body's middle. (Halo 2 plays the shield's depleted effect, not read
    /// here; this look is the remake's own.)
    pub fn shield_pop(&mut self, position: Vec3, color: [f32; 3]) {
        let [r, g, b] = linear(color);
        self.particles.push(Particle {
            position,
            velocity: Vec3::ZERO,
            age: 0.0,
            life: 0.15,
            size: (0.2, 0.5),
            color: [r, g, b, 0.6],
            gravity: 0.0,
        });
        for _ in 0..16 {
            let v = self.random_unit() * (1.5 + self.random() * 1.5);
            let life = 0.2 + self.random() * 0.2;
            self.particles.push(Particle {
                position: position + v * 0.08,
                velocity: v,
                age: 0.0,
                life,
                size: (0.025, 0.006),
                color: [r, g, b, 1.0],
                gravity: 1.0,
            });
        }
    }

    /// A tracer: a contrail laid from `from` to `to`, its head moving at
    /// `speed` world units a second (0: there at once).
    pub fn ribbon(&mut self, from: Vec3, to: Vec3, speed: f32, contrail: &Contrail) {
        if let Some(r) = self.new_ribbon(from, to, speed, 0.0, contrail, RIBBON_PIECES) {
            if self.ribbons.len() >= MAX_RIBBONS {
                self.ribbons.pop_front();
            }
            self.ribbons.push_back(r);
        }
    }

    /// A stretch of a flying round's trail: the `age` seconds it flew from
    /// `from` to `to` at `speed`, laid as it went.
    pub fn trail(&mut self, from: Vec3, to: Vec3, speed: f32, age: f32, contrail: &Contrail) {
        if let Some(r) = self.new_ribbon(from, to, speed, age, contrail, 1) {
            if self.trails.len() >= MAX_TRAIL_STRETCHES {
                self.trails.pop_front();
            }
            self.trails.push_back(r);
        }
    }

    /// A contrail laid from `from` to `to`, its head moving at `speed`
    /// world units a second (0: there at once) and already `age` seconds
    /// along. Each picks its times and colour in the tag's ranges.
    fn new_ribbon(
        &mut self,
        from: Vec3,
        to: Vec3,
        speed: f32,
        age: f32,
        contrail: &Contrail,
        pieces: usize,
    ) -> Option<Ribbon> {
        let mut states: Vec<RibbonState> = Vec::with_capacity(contrail.states.len() + 1);
        for s in &contrail.states {
            let pick = |(lo, hi): (f32, f32), k: f32| lo + (hi - lo) * k;
            let (k0, k1, k2) = (self.random(), self.random(), self.random());
            let c: [f32; 4] =
                std::array::from_fn(|i| s.color.0[i] + (s.color.1[i] - s.color.0[i]) * k2);
            let [r, g, b] = linear([c[0], c[1], c[2]]);
            states.push(RibbonState {
                hold: pick(s.duration, k0).max(0.0),
                fade: pick(s.transition, k1).max(0.0),
                width: s.width,
                color: [r, g, b, c[3]],
            });
        }
        // A contrail of one state fades out of it.
        if let [only] = states[..] {
            states.push(RibbonState {
                color: [only.color[0], only.color[1], only.color[2], 0.0],
                ..only
            });
        }
        (states.len() >= 2).then_some(Ribbon {
            from,
            to,
            speed,
            states,
            age,
            pieces,
        })
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
        for list in [&mut self.ribbons, &mut self.trails] {
            for r in list.iter_mut() {
                r.age += dt;
            }
            list.retain(|r| r.age < r.travel() + r.life());
        }
    }

    /// Triangles for the trails, facing a camera at `eye`: each a strip
    /// whose width and colour follow the age of its points.
    pub fn ribbons(&self, eye: Vec3) -> Vec<SpriteVertex> {
        let mut out = Vec::new();
        for r in self.ribbons.iter().chain(&self.trails) {
            let length = r.from.distance(r.to);
            let dir = (r.to - r.from).normalize_or_zero();
            // The head, and the oldest point still showing.
            let (head, tail) = match r.speed > 0.0 {
                true => (
                    (r.age * r.speed).min(length),
                    ((r.age - r.life()) * r.speed).max(0.0),
                ),
                false => (length, 0.0),
            };
            if head <= tail || dir == Vec3::ZERO {
                continue;
            }
            let point = |k: usize| {
                let s = tail + (head - tail) * k as f32 / r.pieces as f32;
                let age = match r.speed > 0.0 {
                    true => r.age - s / r.speed,
                    false => r.age,
                };
                let at = r.from + dir * s;
                let side = dir.cross(at - eye).normalize_or_zero();
                let (width, color) = r.look(age).unwrap_or((0.0, [0.0; 4]));
                (at, side * width * 0.5, color)
            };
            let v = |p: Vec3, u: f32, color: [f32; 4]| SpriteVertex {
                position: p.into(),
                uv: [u, 0.5],
                color,
            };
            let mut last = point(0);
            for k in 1..=r.pieces {
                let next = point(k);
                let (a0, w0, c0) = last;
                let (a1, w1, c1) = next;
                let quad = [
                    v(a0 - w0, 0.0, c0),
                    v(a0 + w0, 1.0, c0),
                    v(a1 + w1, 1.0, c1),
                    v(a1 - w1, 0.0, c1),
                ];
                out.extend_from_slice(&[quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]);
                last = next;
            }
        }
        out
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
/// A fireball's colours, core and edge: orange, or blue for plasma.
pub fn fireball(plasma: bool) -> ([f32; 4], [f32; 4]) {
    if plasma {
        ([0.55, 0.75, 1.0, 1.0], [0.25, 0.45, 1.0, 0.9])
    } else {
        ([1.0, 0.9, 0.6, 1.0], [1.0, 0.45, 0.12, 0.9])
    }
}

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

    /// The Battle Rifle's tracer: gold, two hundredths wide, gone in a
    /// fifth of a second.
    fn tracer() -> Contrail {
        use blam_cache::effect::PointState;
        let state = |transition, width, alpha| PointState {
            duration: (0.0, 0.0),
            transition: (transition, transition),
            width,
            color: ([1.0, 0.8, 0.35, alpha], [1.0, 0.8, 0.35, alpha]),
        };
        Contrail {
            rate: 60.0,
            states: vec![state(0.2, 0.02, 0.5), state(0.0, 0.005, 0.0)],
        }
    }

    #[test]
    fn tracers_run_from_the_muzzle_and_fade() {
        let mut e = Effects::new();
        e.ribbon(Vec3::ZERO, Vec3::X * 40.0, 400.0, &tracer());
        let eye = Vec3::new(0.0, 0.0, 1.0);
        // Nothing yet: the head hasn't left the muzzle.
        assert!(e.ribbons(eye).is_empty());
        e.update(0.05);
        let v = e.ribbons(eye);
        assert_eq!(v.len(), RIBBON_PIECES * 6);
        // The head is 20 units out, as wide as the tag says when new.
        let far = v.iter().map(|p| p.position[0]).fold(0.0, f32::max);
        assert!((far - 20.0).abs() < 1e-3, "{far}");
        let head = v.iter().find(|p| p.position[0] == far).unwrap();
        assert!((head.position[1].abs() - 0.01).abs() < 1e-4);
        assert!((head.color[3] - 0.5).abs() < 1e-3);
        // The head reaches the end at 0.1 s; its last point is gone 0.2 s
        // after that.
        e.update(0.24);
        assert!(!e.ribbons(eye).is_empty());
        e.update(0.07);
        assert!(e.ribbons(eye).is_empty() && e.ribbons.is_empty());
    }

    #[test]
    fn flying_rounds_trails_leave_the_tracers_be() {
        let mut e = Effects::new();
        e.ribbon(Vec3::ZERO, Vec3::X * 40.0, 0.0, &tracer());
        // A Needler's stream at a high frame rate: far more stretches
        // than tracers are kept.
        for k in 0..2000 {
            let at = Vec3::new(0.0, 1.0, k as f32 * 0.01);
            e.trail(at, at + Vec3::Z * 0.01, 1.0, 0.01, &tracer());
        }
        assert_eq!((e.ribbons.len(), e.trails.len()), (1, 2000));
        // Each stretch is one piece.
        let eye = Vec3::new(0.0, -5.0, 0.0);
        assert_eq!(e.ribbons(eye).len(), (RIBBON_PIECES + 2000) * 6);
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
