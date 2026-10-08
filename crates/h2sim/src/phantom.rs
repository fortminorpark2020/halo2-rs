//! Phantoms: volumes of a map's objects that push whoever is in them rather
//! than block them (gravity lifts, and the vents and pads that throw players
//! up onto ledges), from the phantom types of the objects' physics models.

use crate::collision::closest_segments;
use blam_cache::vehicle::{self, PhantomShape};
use glam::{Mat4, Vec3};

/// One of a phantom's pushes: an acceleration (world units per second
/// squared) up to a speed (world units per second).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Push {
    pub acceleration: f32,
    pub max_speed: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// A box: its centre, its axes (unit length) and how far it reaches
    /// along each.
    Box {
        center: Vec3,
        axes: [Vec3; 3],
        half_extents: Vec3,
    },
    /// A capsule: within `radius` of the line from `a` to `b`.
    Pill { a: Vec3, b: Vec3, radius: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Phantom {
    pub shape: Shape,
    /// Its marker, which its pushes go by: a point, and the way it faces.
    pub origin: Vec3,
    pub forward: Vec3,
    /// Toward the origin; toward the line through it along `forward`; and
    /// along `forward`.
    pub center: Push,
    pub axis: Push,
    pub direction: Push,
    /// Whatever is in it doesn't fall.
    pub negates_gravity: bool,
}

impl Phantom {
    /// A phantom of an object's physics model, the object placed by
    /// `object` and the phantom's marker by `marker` (in the object).
    pub fn placed(tag: &vehicle::Phantom, object: Mat4, marker: Mat4) -> Phantom {
        let point = |p: [f32; 3]| object.transform_point3(Vec3::from(p));
        let scale = object.transform_vector3(Vec3::X).length();
        let shape = match tag.shape {
            PhantomShape::Box(b) => Shape::Box {
                center: point(b.center),
                axes: b.axes.map(|x| {
                    object
                        .transform_vector3(Vec3::from(x))
                        .normalize_or(Vec3::Z)
                }),
                half_extents: Vec3::from(b.half_extents) * scale,
            },
            PhantomShape::Pill { a, b, radius } => Shape::Pill {
                a: point(a),
                b: point(b),
                radius: radius * scale,
            },
        };
        let marker = object * marker;
        let push = |p: vehicle::Push| Push {
            acceleration: p.acceleration,
            max_speed: p.max_speed,
        };
        Phantom {
            shape,
            origin: marker.transform_point3(Vec3::ZERO),
            forward: marker.transform_vector3(Vec3::X).normalize_or(Vec3::Z),
            center: push(tag.center),
            axis: push(tag.axis),
            direction: push(tag.direction),
            negates_gravity: tag.flags & vehicle::Phantom::NEGATES_GRAVITY != 0,
        }
    }

    /// Whether a body, the capsule within `radius` of the line from `a`
    /// to `b`, is in it.
    pub fn touches(&self, a: Vec3, b: Vec3, radius: f32) -> bool {
        match self.shape {
            Shape::Pill {
                a: pa,
                b: pb,
                radius: r,
            } => {
                let (p, q) = closest_segments(a, b, pa, pb);
                p.distance_squared(q) <= (r + radius) * (r + radius)
            }
            Shape::Box {
                center,
                axes,
                half_extents,
            } => {
                // The line against the box grown by the radius on every side.
                let local = |p: Vec3| Vec3::from(axes.map(|x| (p - center).dot(x)));
                let (from, to) = (local(a), local(b));
                let reach = half_extents + Vec3::splat(radius);
                let d = to - from;
                let (mut near, mut far) = (0.0f32, 1.0f32);
                for k in 0..3 {
                    if d[k].abs() < 1e-8 {
                        if from[k].abs() > reach[k] {
                            return false;
                        }
                        continue;
                    }
                    let (s, t) = ((-reach[k] - from[k]) / d[k], (reach[k] - from[k]) / d[k]);
                    near = near.max(s.min(t));
                    far = far.min(s.max(t));
                }
                near <= far
            }
        }
    }

    /// The velocity of a body at `at`, moving at `velocity`, after `dt`
    /// seconds in it. Each push speeds the body up its way until it goes
    /// that push's speed. The pulls toward the marker and its line ease off
    /// as the body nears them rather than throw it past (the tags don't say
    /// how Halo 2 does that; this is our reading).
    pub fn push(&self, at: Vec3, mut velocity: Vec3, dt: f32) -> Vec3 {
        let toward = |velocity: &mut Vec3, offset: Vec3, push: Push| {
            let distance = offset.length();
            if push.acceleration <= 0.0 || distance < 1e-4 {
                return;
            }
            let way = -offset / distance;
            let wanted = push
                .max_speed
                .min((2.0 * push.acceleration * distance).sqrt());
            let step = push.acceleration * dt;
            *velocity += way * (wanted - velocity.dot(way)).clamp(-step, step);
        };
        let from = at - self.origin;
        toward(&mut velocity, from, self.center);
        toward(
            &mut velocity,
            from - self.forward * from.dot(self.forward),
            self.axis,
        );
        let along = velocity.dot(self.forward);
        if self.direction.acceleration > 0.0 && along < self.direction.max_speed {
            let step = (self.direction.max_speed - along).min(self.direction.acceleration * dt);
            velocity += self.forward * step;
        }
        velocity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column() -> Phantom {
        Phantom {
            shape: Shape::Box {
                center: Vec3::new(0.0, 0.0, 2.0),
                axes: [Vec3::X, Vec3::Y, Vec3::Z],
                half_extents: Vec3::new(0.5, 0.5, 2.0),
            },
            origin: Vec3::ZERO,
            forward: Vec3::Z,
            center: Push::default(),
            axis: Push {
                acceleration: 10.0,
                max_speed: 10.0,
            },
            direction: Push {
                acceleration: 10.0,
                max_speed: 4.0,
            },
            negates_gravity: true,
        }
    }

    #[test]
    fn a_body_touches_a_box_it_reaches_into() {
        let c = column();
        let up = Vec3::Z * 0.5;
        assert!(c.touches(Vec3::ZERO, up, 0.2));
        // Beside it, close enough that its radius reaches in.
        assert!(c.touches(Vec3::new(0.65, 0.0, 1.0), Vec3::new(0.65, 0.0, 1.5), 0.2));
        assert!(!c.touches(Vec3::new(0.75, 0.0, 1.0), Vec3::new(0.75, 0.0, 1.5), 0.2));
        assert!(!c.touches(Vec3::new(0.0, 0.0, 4.3), Vec3::new(0.0, 0.0, 5.0), 0.2));
    }

    #[test]
    fn a_body_touches_a_pill_it_reaches_into() {
        let p = Phantom {
            shape: Shape::Pill {
                a: Vec3::ZERO,
                b: Vec3::Z * 3.0,
                radius: 0.5,
            },
            ..column()
        };
        assert!(p.touches(Vec3::new(0.6, 0.0, 1.0), Vec3::new(0.6, 0.0, 2.0), 0.2));
        assert!(!p.touches(Vec3::new(0.8, 0.0, 1.0), Vec3::new(0.8, 0.0, 2.0), 0.2));
        assert!(p.touches(Vec3::new(0.0, 0.0, 3.6), Vec3::new(0.0, 0.0, 4.0), 0.2));
    }

    #[test]
    fn pushes_up_to_speed_and_draws_in_to_the_middle() {
        let c = column();
        let (mut at, mut v) = (Vec3::new(0.4, 0.0, 0.5), Vec3::ZERO);
        let dt = 1.0 / 120.0;
        for _ in 0..120 {
            v = c.push(at, v, dt);
            at += v * dt;
        }
        assert!((v.z - 4.0).abs() < 1e-3, "rises at its top speed: {v}");
        assert!(
            at.x.abs() < 0.05 && v.x.abs() < 0.1,
            "in the middle: {at} {v}"
        );
    }
}
