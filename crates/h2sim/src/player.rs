//! Player movement: a capsule that walks, jumps, crouches and slides along
//! the level, tuned with the values from the game's own tags.

use crate::collision::{Contact, World};
use blam_cache::physics::{BipedPhysics, PlayerMovement};
use glam::{Vec2, Vec3};

/// Halo's base gravity in world units per second squared.
pub const GRAVITY: f32 = 4.171_168;
/// Physics steps per second. Halo 2 simulates at 30 ticks; we sub-step finer
/// for smoother collision and scale everything per second.
pub const STEP_HZ: f32 = 120.0;
const PUSH_ITERATIONS: usize = 4;
/// How far down we look for ground to stick to when walking off small drops.
const GROUND_SNAP: f32 = 0.06;
/// How far past the edge of a step, and how far off its height, its top is
/// looked for.
const EDGE: f32 = 0.01;

#[derive(Debug, Clone, Copy, Default)]
pub struct Input {
    /// x: right, y: forward, each in -1..=1.
    pub movement: Vec2,
    /// Radians around +z, 0 = +x.
    pub yaw: f32,
    pub jump: bool,
    pub crouch: bool,
    /// Flying: up (1) or down (-1).
    pub lift: f32,
}

#[derive(Debug, Clone)]
pub struct Player {
    /// Bottom of the capsule (the feet).
    pub position: Vec3,
    pub velocity: Vec3,
    pub grounded: bool,
    /// 0 = standing, 1 = fully crouched (eases between them).
    pub crouch: f32,
    pub movement: PlayerMovement,
    pub biped: BipedPhysics,
    /// Height of the fall just landed from (for falling damage; whoever
    /// reads it clears it).
    pub fell: f32,
    /// The level's gravity, as a share of normal (scripts lower it where
    /// the air's let out).
    pub gravity: f32,
    /// Jump was held at the last step (another jump takes letting go).
    pub jump_held: bool,
    /// Stood on the level's own ground at the end of the last step (not
    /// just held up by something else, a vehicle's hull say).
    pub on_level: bool,
    accumulator: f32,
    contacts: Vec<Contact>,
}

impl Player {
    pub fn new(position: Vec3, movement: PlayerMovement, biped: BipedPhysics) -> Player {
        Player {
            position,
            velocity: Vec3::ZERO,
            grounded: false,
            crouch: 0.0,
            movement,
            biped,
            fell: 0.0,
            gravity: 1.0,
            jump_held: false,
            on_level: false,
            accumulator: 0.0,
            contacts: Vec::new(),
        }
    }

    pub fn height(&self) -> f32 {
        let b = &self.biped;
        b.height_standing + (b.height_crouching - b.height_standing) * self.crouch
    }

    pub fn eye(&self) -> Vec3 {
        let b = &self.biped;
        let h = b.standing_camera_height
            + (b.crouching_camera_height - b.standing_camera_height) * self.crouch;
        self.position + Vec3::Z * (h + self.origin_height())
    }

    /// How far above the feet the body's origin is (its middle, for one
    /// centred there like a Sentinel).
    pub fn origin_height(&self) -> f32 {
        if self.biped.centered {
            self.height() * 0.5
        } else {
            0.0
        }
    }

    fn capsule(&self, pos: Vec3) -> (Vec3, Vec3) {
        let r = self.biped.radius;
        let top = (self.height() - r).max(r);
        (pos + Vec3::Z * r, pos + Vec3::Z * top)
    }

    /// Advance by `dt` seconds of real time.
    pub fn update(&mut self, world: &World, input: Input, dt: f32) {
        self.accumulator = (self.accumulator + dt).min(0.25);
        let step = 1.0 / STEP_HZ;
        while self.accumulator >= step {
            self.accumulator -= step;
            self.step(world, input, step);
        }
    }

    fn step(&mut self, world: &World, input: Input, dt: f32) {
        if self.biped.flying {
            return self.fly(world, input, dt);
        }
        let m = self.movement;
        // Crouch eases over ~0.1 s.
        let target = if input.crouch { 1.0 } else { 0.0 };
        let new_crouch = self.crouch + (target - self.crouch).clamp(-dt * 10.0, dt * 10.0);
        if new_crouch < self.crouch && !self.fits(world, self.position, new_crouch) {
            // Something overhead: stay crouched.
        } else {
            self.crouch = new_crouch;
        }

        // Desired horizontal velocity from input, in world space.
        let fwd = Vec2::new(input.yaw.cos(), input.yaw.sin());
        let right = Vec2::new(fwd.y, -fwd.x);
        let mv = input.movement.clamp_length_max(1.0);
        let sneaking = self.crouch > 0.5;
        let (speed_f, speed_b, speed_s, accel) = if sneaking {
            (
                m.sneak_forward,
                m.sneak_backward,
                m.sneak_sideways,
                m.sneak_acceleration,
            )
        } else {
            (
                m.run_forward,
                m.run_backward,
                m.run_sideways,
                m.run_acceleration,
            )
        };
        let along = if mv.y >= 0.0 {
            mv.y * speed_f
        } else {
            mv.y * speed_b
        };
        let desired = fwd * along + right * (mv.x * speed_s);

        let accel = if self.grounded {
            accel
        } else {
            m.airborne_acceleration
        };
        let horiz = self.velocity.truncate();
        let delta = desired - horiz;
        let change = delta.clamp_length_max(accel * dt);
        self.velocity.x += change.x;
        self.velocity.y += change.y;

        if input.jump && !self.jump_held && self.grounded {
            self.velocity.z = self.biped.jump_velocity;
            self.grounded = false;
        }
        self.jump_held = input.jump;
        // On the level's own ground (and not jumping off it).
        if self.grounded && self.on_level {
            self.walk(world, dt);
            self.on_level = self.grounded;
            return;
        }

        let gravity = GRAVITY * m.gravity_scale * self.gravity;
        self.velocity.z -= gravity * dt;
        let was_grounded = self.grounded;
        let falling = (-self.velocity.z).max(0.0);
        self.position += self.velocity * dt;
        self.grounded = self.resolve(world, false);
        if self.grounded && !was_grounded {
            // How far a fall would have to be to land this fast.
            self.fell = self.fell.max(falling * falling / (2.0 * gravity));
        }

        // Stick to the ground when walking down slopes or small steps.
        if was_grounded && !self.grounded && self.velocity.z <= 0.0 {
            let saved = (self.position, self.velocity);
            self.position.z -= GROUND_SNAP;
            if self.resolve(world, false) {
                self.grounded = true;
            } else {
                (self.position, self.velocity) = saved;
            }
        }
        self.on_level = self.grounded;
    }

    /// A step on the ground. The body moves level: floors lift it, the
    /// edges of steps and kerbs it walks into lift it onto them, walls stop
    /// it, and none of them throws it upward, so it doesn't take off over
    /// the top of a ramp or a step. It follows the ground down over the top
    /// of a slope or down a step, and only leaves the ground where the
    /// floor drops away more than a step.
    fn walk(&mut self, world: &World, dt: f32) {
        self.velocity.z = 0.0;
        self.position += self.velocity * dt;
        let mut grounded = self.resolve(world, true);
        if !grounded {
            // Down to the ground a little at a time, so as to come down
            // onto a step's edge rather than past it. (A step is as high
            // as an edge the bottom of the body can touch: its radius.)
            let saved = (self.position, self.velocity);
            let drop = self.biped.radius / 4.0;
            for _ in 0..4 {
                self.position.z -= drop;
                let at = self.position;
                grounded = self.resolve(world, true);
                if grounded || self.position != at {
                    break;
                }
            }
            if !grounded {
                (self.position, self.velocity) = saved;
            }
        }
        self.grounded = grounded;
    }

    /// Walking into the edge of something low (a step, a kerb) with the
    /// bottom of the body (`c`, a contact facing up): how far to lift it to
    /// stand on the edge, if there's floor on top.
    fn step_onto(&self, world: &World, c: &Contact, floor_cos: f32) -> Option<f32> {
        let r = self.biped.radius;
        let centre = self.position + Vec3::Z * r;
        let touch = centre - c.normal * (r - c.depth);
        // Just past the edge, looking down from the height of the body's
        // bottom.
        let into = (-c.normal.truncate()).normalize_or_zero();
        let above = (touch.truncate() + into * EDGE).extend(centre.z + EDGE);
        let reach = centre.z - touch.z + 2.0 * EDGE;
        let (down, n) = world.raycast_hit(above, Vec3::NEG_Z, reach)?;
        let top = above.z - down;
        if n.z < floor_cos || (top - touch.z).abs() > 2.0 * EDGE {
            return None;
        }
        let aside = (centre - touch).truncate().length().min(r);
        Some((touch.z + (r * r - aside * aside).sqrt() - centre.z).max(0.0))
    }

    /// A flier speeds up toward the way it's steered, up or down too, and
    /// slows when let go; nothing pulls it down.
    fn fly(&mut self, world: &World, input: Input, dt: f32) {
        let b = self.biped;
        let fwd = Vec2::new(input.yaw.cos(), input.yaw.sin());
        let right = Vec2::new(fwd.y, -fwd.x);
        let mv = input.movement.clamp_length_max(1.0);
        let desired = (fwd * mv.y * b.fly_speed + right * mv.x * b.fly_sidestep)
            .extend(input.lift.clamp(-1.0, 1.0) * b.fly_sidestep);
        let rate = if desired.length() > self.velocity.length() {
            b.fly_acceleration
        } else {
            b.fly_deceleration
        };
        self.velocity += (desired - self.velocity).clamp_length_max(rate.max(0.5) * dt);
        self.position += self.velocity * dt;
        self.resolve(world, false);
        self.grounded = false;
        self.on_level = false;
    }

    /// Push the capsule out of the world; returns whether it stands on
    /// walkable ground. Walking, only walls change the velocity, and only
    /// the part going into them sideways.
    fn resolve(&mut self, world: &World, walking: bool) -> bool {
        let floor_cos = self.biped.max_slope.cos();
        let mut grounded = false;
        for _ in 0..PUSH_ITERATIONS {
            let (p0, p1) = self.capsule(self.position);
            let mut contacts = std::mem::take(&mut self.contacts);
            world.capsule_contacts(p0, p1, self.biped.radius, &mut contacts);
            if contacts.is_empty() {
                self.contacts = contacts;
                break;
            }
            // Resolve the deepest contact first, then re-query.
            let c = *contacts
                .iter()
                .max_by(|a, b| a.depth.total_cmp(&b.depth))
                .unwrap();
            self.contacts = contacts;
            let walkable = c.normal.z >= floor_cos;
            if walking && !walkable && c.normal.z > 0.0 {
                // The edge of a step: up onto it.
                if let Some(lift) = self.step_onto(world, &c, floor_cos) {
                    self.position.z += lift + 1e-4;
                    grounded = true;
                    continue;
                }
            }
            // Floors push straight up so we don't slide down gentle slopes.
            let push = if walkable {
                Vec3::Z * (c.depth / c.normal.z)
            } else {
                c.normal * c.depth
            };
            self.position += push + c.normal * 1e-4;
            if walking {
                let flat = c.normal.truncate().normalize_or_zero().extend(0.0);
                let into = self.velocity.dot(flat);
                if !walkable && into < 0.0 {
                    self.velocity -= flat * into;
                }
                grounded |= walkable;
                continue;
            }
            let into = self.velocity.dot(c.normal);
            if into < 0.0 {
                self.velocity -= c.normal * into;
            }
            if walkable {
                grounded = true;
                self.velocity.z = self.velocity.z.max(0.0);
            }
        }
        grounded
    }

    fn fits(&self, world: &World, pos: Vec3, crouch: f32) -> bool {
        let b = &self.biped;
        let h = b.height_standing + (b.height_crouching - b.height_standing) * crouch;
        let r = b.radius;
        let mut c = Vec::new();
        // Only the upper part matters; the feet are already resolved.
        world.capsule_contacts(
            pos + Vec3::Z * (h * 0.5).max(r),
            pos + Vec3::Z * (h - r).max(r),
            r * 0.95,
            &mut c,
        );
        c.iter().all(|c| c.normal.z > -0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(p: &mut Vec<[f32; 3]>, i: &mut Vec<u32>, corners: [[f32; 3]; 4]) {
        let base = p.len() as u32;
        p.extend_from_slice(&corners);
        i.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Floor at z=0 and a wall facing -x at x=2.
    fn room() -> World {
        let (mut p, mut i) = (Vec::new(), Vec::new());
        quad(
            &mut p,
            &mut i,
            [
                [-10.0, -10.0, 0.0],
                [10.0, -10.0, 0.0],
                [10.0, 10.0, 0.0],
                [-10.0, 10.0, 0.0],
            ],
        );
        quad(
            &mut p,
            &mut i,
            [
                [2.0, -10.0, 0.0],
                [2.0, -10.0, 3.0],
                [2.0, 10.0, 3.0],
                [2.0, 10.0, 0.0],
            ],
        );
        World::new(&p, &i)
    }

    fn player_at(z: f32) -> Player {
        Player::new(
            Vec3::new(0.0, 0.0, z),
            PlayerMovement::default(),
            BipedPhysics::default(),
        )
    }

    #[test]
    fn falls_and_lands_on_floor() {
        let w = room();
        let mut p = player_at(1.0);
        for _ in 0..120 {
            p.update(&w, Input::default(), 1.0 / 60.0);
        }
        assert!(p.grounded);
        assert!(p.position.z.abs() < 0.01, "z = {}", p.position.z);
    }

    #[test]
    fn runs_at_tag_speed() {
        let w = room();
        let mut p = player_at(0.0);
        let input = Input {
            movement: Vec2::new(0.0, 1.0),
            yaw: std::f32::consts::FRAC_PI_2,
            ..Default::default()
        };
        for _ in 0..60 {
            p.update(&w, input, 1.0 / 60.0);
        }
        assert!((p.velocity.y - 2.25).abs() < 0.01, "vy = {}", p.velocity.y);
    }

    #[test]
    fn a_flier_hovers_and_climbs_without_falling() {
        let w = room();
        let biped = BipedPhysics {
            flying: true,
            centered: true,
            standing_camera_height: 0.0,
            crouching_camera_height: 0.0,
            height_standing: 0.7,
            height_crouching: 0.7,
            radius: 0.35,
            fly_speed: 2.25,
            fly_sidestep: 2.0,
            fly_acceleration: 2.0,
            fly_deceleration: 3.0,
            ..BipedPhysics::default()
        };
        let mut p = Player::new(Vec3::new(0.0, 0.0, 1.0), PlayerMovement::default(), biped);
        for _ in 0..60 {
            p.update(&w, Input::default(), 1.0 / 60.0);
        }
        assert!(
            (p.position.z - 1.0).abs() < 0.01,
            "hovers, z = {}",
            p.position.z
        );
        let up = Input {
            lift: 1.0,
            ..Default::default()
        };
        for _ in 0..60 {
            p.update(&w, up, 1.0 / 60.0);
        }
        assert!(
            p.position.z > 1.5 && p.velocity.z > 1.5,
            "climbs, z = {}",
            p.position.z
        );
        assert!(
            (p.eye().z - p.position.z - 0.35).abs() < 1e-4,
            "sees from its middle"
        );
    }

    #[test]
    fn wall_stops_movement() {
        let w = room();
        let mut p = player_at(0.0);
        let input = Input {
            movement: Vec2::new(0.0, 1.0),
            yaw: 0.0,
            ..Default::default()
        };
        for _ in 0..240 {
            p.update(&w, input, 1.0 / 60.0);
        }
        let limit = 2.0 - p.biped.radius;
        assert!(
            p.position.x <= limit + 0.01 && p.position.x > limit - 0.05,
            "x = {}",
            p.position.x
        );
    }

    #[test]
    fn jump_reaches_expected_height() {
        let w = room();
        let mut p = player_at(0.0);
        p.update(&w, Input::default(), 0.1);
        let mut peak: f32 = 0.0;
        p.update(
            &w,
            Input {
                jump: true,
                ..Default::default()
            },
            1.0 / 120.0,
        );
        for _ in 0..240 {
            p.update(
                &w,
                Input {
                    jump: true,
                    ..Default::default()
                },
                1.0 / 120.0,
            );
            peak = peak.max(p.position.z);
        }
        let expected = 3.08f32.powi(2) / (2.0 * GRAVITY);
        assert!(
            (peak - expected).abs() < 0.05,
            "peak {peak} expected {expected}"
        );
        assert!(p.grounded);
    }

    /// A strip 4 wide along +x of floor sections at the given heights
    /// (each `[x0, z0, x1, z1]`), with upright faces joining them where
    /// they don't meet.
    fn strip(sections: &[[f32; 4]]) -> World {
        let (mut p, mut i) = (Vec::new(), Vec::new());
        let w = 2.0;
        for (k, &[x0, z0, x1, z1]) in sections.iter().enumerate() {
            quad(
                &mut p,
                &mut i,
                [[x0, -w, z0], [x1, -w, z1], [x1, w, z1], [x0, w, z0]],
            );
            if let Some(&[nx, nz, ..]) = sections.get(k + 1) {
                if (nz - z1).abs() > 1e-4 {
                    quad(
                        &mut p,
                        &mut i,
                        [[nx, -w, z1], [nx, -w, nz], [nx, w, nz], [nx, w, z1]],
                    );
                }
            }
        }
        World::new(&p, &i)
    }

    /// Run along +x for `seconds`; how far it got, and the longest it was
    /// off the ground at a stretch (seconds) before reaching `until` x.
    fn run_along(w: &World, start: Vec3, seconds: f32, until: f32) -> (Player, f32) {
        let mut p = Player::new(start, PlayerMovement::default(), BipedPhysics::default());
        p.update(w, Input::default(), 0.1);
        let input = Input {
            movement: Vec2::new(0.0, 1.0),
            ..Default::default()
        };
        let (mut off, mut longest) = (0.0f32, 0.0f32);
        let dt = 1.0 / 60.0;
        for _ in 0..(seconds / dt) as usize {
            p.update(w, input, dt);
            if p.position.x < until {
                off = if p.grounded { 0.0 } else { off + dt };
                longest = longest.max(off);
            }
        }
        (p, longest)
    }

    #[test]
    fn running_over_the_top_of_a_ramp_keeps_to_the_ground() {
        // Up 30 degrees, level, then down 30 degrees.
        let rise = 2.0 * 30f32.to_radians().tan();
        let w = strip(&[
            [-4.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 2.0, rise],
            [2.0, rise, 3.0, rise],
            [3.0, rise, 5.0, 0.0],
            [5.0, 0.0, 12.0, 0.0],
        ]);
        let (p, longest) = run_along(&w, Vec3::new(-2.0, 0.0, 0.0), 5.0, 7.0);
        assert!(p.position.x > 7.0, "got to x = {}", p.position.x);
        assert!(longest == 0.0, "off the ground for {longest} s");
    }

    #[test]
    fn running_up_stairs_keeps_to_the_ground() {
        // Steps 0.15 up every 0.3, to 0.9 up.
        let mut steps = vec![[-4.0, 0.0, 0.0, 0.0]];
        for k in 0..6 {
            let (x, z) = (k as f32 * 0.3, (k + 1) as f32 * 0.15);
            steps.push([x, z, x + 0.3, z]);
        }
        steps.push([1.8, 0.9, 8.0, 0.9]);
        let w = strip(&steps);
        let (p, longest) = run_along(&w, Vec3::new(-2.0, 0.0, 0.0), 3.0, 4.0);
        assert!(
            p.position.x > 4.0 && (p.position.z - 0.9).abs() < 0.01,
            "got to {}",
            p.position
        );
        assert!(longest == 0.0, "off the ground for {longest} s");
        // Coming back down them too.
        let back = Player {
            velocity: Vec3::ZERO,
            ..p
        };
        let mut p = back;
        let input = Input {
            movement: Vec2::new(0.0, -1.0),
            ..Default::default()
        };
        for _ in 0..200 {
            p.update(&w, input, 1.0 / 60.0);
            assert!(p.grounded, "off the ground at {}", p.position);
        }
        assert!(
            p.position.x < -1.0 && p.position.z.abs() < 0.01,
            "got back to {}",
            p.position
        );
    }

    #[test]
    fn running_off_a_ledge_falls() {
        let w = strip(&[[-4.0, 0.5, 0.0, 0.5], [0.0, 0.0, 8.0, 0.0]]);
        let (p, longest) = run_along(&w, Vec3::new(-2.0, 0.0, 0.5), 3.0, 8.0);
        assert!(longest > 0.2, "off the ground for {longest} s");
        assert!(p.grounded && p.position.z.abs() < 0.01, "at {}", p.position);
        // A wall too high to step up still stops it.
        let w = strip(&[[-4.0, 0.0, 0.0, 0.0], [0.0, 0.5, 8.0, 0.5]]);
        let (p, _) = run_along(&w, Vec3::new(-2.0, 0.0, 0.0), 3.0, 8.0);
        assert!(p.position.x < 0.0 && p.grounded, "at {}", p.position);
    }
}
