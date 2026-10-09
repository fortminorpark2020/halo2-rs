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
/// How deep in the level a walking step may leave the body (pushes out of
/// several surfaces at once don't always quite settle) and still count as
/// clear of it.
const SNUG: f32 = 1e-3;
/// The steepest slope (radians) that bounds how fast the body follows the
/// ground down or up in a step, whatever a tag says its steepest floor is.
const STEEPEST: f32 = 1.4;

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

    /// Set down on top of something with its top at `top`, moving up or
    /// down at `lift` (another player's head): a landing like one on the
    /// ground, so a long drop onto it still hurts (by the body's own
    /// speed down, so being carried up by it doesn't).
    pub fn stand_on(&mut self, top: f32, lift: f32) {
        let falling = (-self.velocity.z).max(0.0);
        let gravity = GRAVITY * self.movement.gravity_scale * self.gravity;
        if gravity > 0.0 {
            self.fell = self.fell.max(falling * falling / (2.0 * gravity));
        }
        self.position.z = top;
        self.velocity.z = self.velocity.z.max(lift);
        self.grounded = true;
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
        self.crouch_toward(world, input.crouch, dt);

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
        let floating = self.pushed(world, dt);

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
        if !floating {
            self.velocity.z -= gravity * dt;
        }
        let was_grounded = self.grounded;
        let falling = (-self.velocity.z).max(0.0);
        self.position += self.velocity * dt;
        self.grounded = self.resolve(world, None);
        if self.grounded && !was_grounded {
            // How far a fall would have to be to land this fast.
            self.fell = self.fell.max(falling * falling / (2.0 * gravity));
        }

        // Stick to the ground when walking down slopes or small steps.
        if was_grounded && !self.grounded && self.velocity.z <= 0.0 {
            let saved = (self.position, self.velocity);
            self.position.z -= GROUND_SNAP;
            if self.resolve(world, None) {
                self.grounded = true;
            } else {
                (self.position, self.velocity) = saved;
            }
        }
        self.on_level = self.grounded;
    }

    /// The level's phantoms (gravity lifts) that the body is in push it,
    /// and one pushing it upward lifts it off the ground. Returns whether
    /// one holds gravity off.
    fn pushed(&mut self, world: &World, dt: f32) -> bool {
        let phantoms = world.phantoms();
        if phantoms.is_empty() {
            return false;
        }
        let (a, b) = self.capsule(self.position);
        let (middle, rising) = ((a + b) * 0.5, self.velocity.z.max(0.0));
        let mut floating = false;
        for p in phantoms
            .iter()
            .filter(|p| p.touches(a, b, self.biped.radius))
        {
            self.velocity = p.push(middle, self.velocity, dt);
            floating |= p.negates_gravity;
        }
        if self.velocity.z > rising {
            self.grounded = false;
        }
        floating
    }

    /// Crouch down or stand up over the biped's crouch time. On the ground
    /// the body shrinks and grows from the top, the feet staying on the
    /// floor. In the air it shrinks from the bottom, the head staying
    /// where it is, so crouching mid-jump pulls the feet up onto a ledge
    /// they'd otherwise miss (a crouch jump); standing up again lets the
    /// feet down, where there's room below, or else grows upward. With
    /// something in the way either way, it stays crouched.
    fn crouch_toward(&mut self, world: &World, down: bool, dt: f32) {
        let rate = match self.biped.crouch_time {
            t if t > 0.0 => dt / t,
            _ => 1.0,
        };
        let target = if down { 1.0 } else { 0.0 };
        let crouch = self.crouch + (target - self.crouch).clamp(-rate, rate);
        if crouch == self.crouch {
            return;
        }
        // How much taller the body gets (shorter, below zero).
        let b = &self.biped;
        let grow = (b.height_crouching - b.height_standing) * (crouch - self.crouch);
        if self.grounded {
            if grow <= 0.0 || self.fits(world, self.position, crouch) {
                self.crouch = crouch;
            }
            return;
        }
        if grow <= 0.0 {
            self.position.z -= grow;
            self.crouch = crouch;
            return;
        }
        let lower = self.position - Vec3::Z * grow;
        if self.room_below(world, lower) {
            self.position = lower;
            self.crouch = crouch;
        } else if self.fits(world, self.position, crouch) {
            self.crouch = crouch;
        }
    }

    /// Whether the bottom of the body, with its feet let down to `feet`,
    /// is clear of the level.
    fn room_below(&self, world: &World, feet: Vec3) -> bool {
        let r = self.biped.radius;
        let mut c = Vec::new();
        world.capsule_contacts(
            feet + Vec3::Z * r,
            feet + Vec3::Z * (self.height() * 0.5).max(r),
            r * 0.95,
            &mut c,
        );
        c.is_empty()
    }

    /// A step on the ground. The body moves level: floors lift it, the
    /// edges of steps and kerbs it walks into lift it onto them, walls stop
    /// it, and none of them throws it upward, so it doesn't take off over
    /// the top of a ramp or a step. It follows the ground down no faster
    /// than down the steepest floor, or a little more for a small step
    /// (`GROUND_SNAP`); where the ground drops away faster, off a ledge or
    /// over the brink of a step, it leaves the ground and falls. A move
    /// that would leave it stuck in the level (squeezed between a slope
    /// and something low overhead, say) or lift it more than a step isn't
    /// made: it stays where it was and stops going that way.
    fn walk(&mut self, world: &World, dt: f32) {
        let start = self.position;
        self.velocity.z = 0.0;
        // The way it's going, for what counts as a step up ahead of it
        // (walls it runs into take away its speed as it goes).
        let heading = self.velocity.truncate();
        let moved = heading * dt;
        let slope = moved.length() * self.biped.max_slope.clamp(0.0, STEEPEST).tan();
        self.position += moved.extend(0.0);
        let mut grounded = self.resolve(world, Some(heading));
        if !grounded {
            // Down to the ground a little at a time, so as to come down
            // onto a step's edge rather than past it; coming down onto
            // the brink of the edge it's going over (which pushes it up
            // and out) instead, it goes over and falls.
            let saved = (self.position, self.velocity);
            let reach = slope + GROUND_SNAP;
            let drops = (reach / (self.biped.radius / 4.0)).ceil().max(1.0);
            for _ in 0..drops as usize {
                self.position.z -= reach / drops;
                let at = self.position.z;
                grounded = self.resolve(world, Some(heading));
                if grounded || self.position.z > at {
                    break;
                }
            }
            if !grounded {
                (self.position, self.velocity) = saved;
            }
        }
        // No higher than a step (an edge as high as the bottom of the body
        // can touch: its radius) on top of the slope it walks up.
        let climb = self.biped.radius + slope + EDGE;
        let stuck = self.overlap(world, self.position);
        if stuck > SNUG || self.position.z - start.z > climb {
            // Unless it was already stuck (put there from outside) and
            // this gets it out.
            let was = self.overlap(world, start);
            if was <= SNUG || stuck > was {
                self.stop_against(world, start + moved.extend(0.0), moved);
                self.position = start;
                grounded = true;
            }
        }
        self.grounded = grounded;
    }

    /// How deep the body standing at `pos` is in the level: its deepest
    /// contact.
    fn overlap(&mut self, world: &World, pos: Vec3) -> f32 {
        let (p0, p1) = self.capsule(pos);
        let mut contacts = std::mem::take(&mut self.contacts);
        world.capsule_contacts(p0, p1, self.biped.radius, &mut contacts);
        let deepest = contacts.iter().fold(0.0f32, |d, c| d.max(c.depth));
        self.contacts = contacts;
        deepest
    }

    /// The body couldn't make the move `moved` to `to`: take away the part
    /// of its speed going into what it ran into there, or, if nothing it
    /// ran into faces against the move (a floor rising under something
    /// overhead), all of it.
    fn stop_against(&mut self, world: &World, to: Vec3, moved: Vec2) {
        let (p0, p1) = self.capsule(to);
        let mut contacts = std::mem::take(&mut self.contacts);
        world.capsule_contacts(p0, p1, self.biped.radius, &mut contacts);
        let mut v = self.velocity.truncate();
        for c in &contacts {
            let flat = c.normal.truncate().normalize_or_zero();
            let into = v.dot(flat);
            if into < 0.0 {
                v -= flat * into;
            }
        }
        self.contacts = contacts;
        if v.dot(moved) > 0.0 && v == self.velocity.truncate() {
            v = Vec2::ZERO;
        }
        self.velocity = v.extend(0.0);
    }

    /// Walking into the edge of something low (a step, a kerb) with the
    /// bottom of the body (`c`, a contact facing up): how far to lift it to
    /// stand on the edge, if there's floor on top. Only an edge ahead of
    /// it, the way it's `heading`: one behind or beside it is the brink of
    /// where it's going down from, which doesn't hold it up.
    fn step_onto(&self, world: &World, c: &Contact, floor_cos: f32, heading: Vec2) -> Option<f32> {
        let r = self.biped.radius;
        let centre = self.position + Vec3::Z * r;
        let touch = centre - c.normal * (r - c.depth);
        if (touch - centre).truncate().dot(heading) <= 0.0 {
            return None;
        }
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
        Some(touch.z + (r * r - aside * aside).sqrt() - centre.z).filter(|&lift| lift > 0.0)
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
        self.resolve(world, None);
        self.grounded = false;
        self.on_level = false;
    }

    /// Push the capsule out of the world; returns whether it stands on
    /// walkable ground. Walking (`heading` the way it's going), only walls
    /// change the velocity, and only the part going into them sideways.
    fn resolve(&mut self, world: &World, heading: Option<Vec2>) -> bool {
        let walking = heading.is_some();
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
            if let Some(heading) = heading.filter(|_| !walkable && c.normal.z > 0.0) {
                // The edge of a step: up onto it.
                if let Some(lift) = self.step_onto(world, &c, floor_cos, heading) {
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
    fn a_gravity_lift_carries_a_player_up_its_middle() {
        use crate::phantom::{Phantom, Push, Shape};
        let mut w = room();
        // Lockout's lift: a 4.5 tall column that holds gravity off, draws
        // players to its middle and lifts them at up to 4 wu/s.
        w.add_phantom(Phantom {
            shape: Shape::Box {
                center: Vec3::new(0.0, 0.0, 2.25),
                axes: [Vec3::X, Vec3::Y, Vec3::Z],
                half_extents: Vec3::new(0.6, 0.6, 2.25),
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
        });
        let mut p = player_at(0.0);
        p.position.x = 0.4;
        p.update(&w, Input::default(), 0.1);
        let mut top = 0.0f32;
        for k in 0..240 {
            p.update(&w, Input::default(), 1.0 / 60.0);
            top = top.max(p.position.z);
            if k == 60 {
                assert!(!p.grounded && p.position.z > 2.0, "{p:?}");
                assert!(
                    (p.velocity.z - 4.0).abs() < 0.05,
                    "rises at 4: {}",
                    p.velocity.z
                );
                assert!(
                    p.position.x.abs() < 0.05,
                    "drawn to the middle: {}",
                    p.position.x
                );
            }
        }
        // It lets go once the feet clear its top, and the player coasts on.
        let coast = 4.0f32.powi(2) / (2.0 * GRAVITY);
        assert!(top > 4.5 && top < 4.5 + coast + 0.1, "top {top}");
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

    /// The highest the feet get in a jump from the floor, crouching from
    /// `crouch_at` ticks (of 60 a second) after take-off, if at all.
    fn feet_apex(crouch_at: Option<usize>) -> f32 {
        let w = room();
        let mut p = player_at(0.0);
        for _ in 0..30 {
            p.update(&w, Input::default(), 1.0 / 60.0);
        }
        let mut top: f32 = 0.0;
        for k in 0..120 {
            let input = Input {
                jump: k == 0,
                crouch: crouch_at.is_some_and(|c| k >= c),
                ..Default::default()
            };
            p.update(&w, input, 1.0 / 60.0);
            top = top.max(p.position.z);
        }
        top
    }

    #[test]
    fn crouching_in_the_air_pulls_the_feet_up() {
        // A crouch jump clears the difference between the standing and
        // crouching heights higher (0.725 - 0.5).
        let plain = feet_apex(None);
        let crouched = feet_apex(Some(1));
        assert!(
            (crouched - plain - 0.225).abs() < 0.01,
            "{plain} then {crouched}"
        );
        // Crouching on the floor takes the biped's 0.2 s and keeps the
        // feet down.
        let w = room();
        let mut p = player_at(0.0);
        p.update(&w, Input::default(), 0.5);
        let down = Input {
            crouch: true,
            ..Default::default()
        };
        let mut ticks = 0;
        while p.crouch < 1.0 && ticks < 60 {
            p.update(&w, down, 1.0 / 60.0);
            ticks += 1;
        }
        assert_eq!(ticks, 12);
        assert!(p.position.z.abs() < 0.01, "z = {}", p.position.z);
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
        // Coming back down them too (over the edge of each, as off a
        // ledge).
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
        }
        assert!(
            p.grounded && p.position.x < -1.0 && p.position.z.abs() < 0.01,
            "got back to {}",
            p.position
        );
    }

    #[test]
    fn running_off_a_ledge_falls() {
        let b = BipedPhysics::default();
        let m = PlayerMovement::default();
        let dt = 1.0 / 60.0;
        for h in [0.1f32, 0.15, 0.2, 0.25, 0.3, 0.5] {
            let w = strip(&[[-4.0, h, 0.0, h], [0.0, 0.0, 8.0, 0.0]]);
            let mut p = Player::new(Vec3::new(-2.0, 0.0, h), m, b);
            p.update(&w, Input::default(), 0.1);
            let input = Input {
                movement: Vec2::new(0.0, 1.0),
                ..Default::default()
            };
            let (mut t, mut left, mut down) = (0.0, None, None);
            let (mut air, mut fastest) = (0.0, 0.0f32);
            for _ in 0..180 {
                let z = p.position.z;
                p.update(&w, input, dt);
                t += dt;
                fastest = fastest.max(z - p.position.z);
                if p.position.z < h - 1e-3 && left.is_none() {
                    left = Some(t - dt);
                }
                if p.position.z < 1e-3 && down.is_none() {
                    down = Some(t);
                }
                if !p.grounded {
                    air += dt;
                }
            }
            assert!(p.grounded && p.position.x > 4.0, "{h}: at {}", p.position);
            // It goes down no faster than it would fall that far, and
            // never drops faster in a frame than running down the
            // steepest floor or landing from that high.
            let fall = (2.0 * h / GRAVITY).sqrt();
            let took = down.unwrap() - left.unwrap();
            assert!(took >= fall, "{h}: down in {took} s, a fall takes {fall}");
            assert!(air > 0.1, "{h}: off the ground for {air} s");
            let steepest = (m.run_forward * b.max_slope.tan()).max((2.0 * GRAVITY * h).sqrt());
            assert!(
                fastest < steepest * dt + 0.005,
                "{h}: dropped {fastest} in a frame"
            );
        }
        // A wall too high to step up still stops it.
        let w = strip(&[[-4.0, 0.0, 0.0, 0.0], [0.0, 0.5, 8.0, 0.5]]);
        let (p, _) = run_along(&w, Vec3::new(-2.0, 0.0, 0.0), 3.0, 8.0);
        assert!(p.position.x < 0.0 && p.grounded, "at {}", p.position);
    }

    /// Floor at 0 up to y = 0.5, then a 45 degree slope rising along +y,
    /// all under a 0.2 thick slab whose underside is `under` up.
    fn slope_under_slab(under: f32) -> World {
        let (mut p, mut i) = (Vec::new(), Vec::new());
        let (w, top) = (6.0, under + 0.2);
        quad(
            &mut p,
            &mut i,
            [[-w, -w, 0.0], [w, -w, 0.0], [w, 0.5, 0.0], [-w, 0.5, 0.0]],
        );
        quad(
            &mut p,
            &mut i,
            [[-w, 0.5, 0.0], [w, 0.5, 0.0], [w, 3.5, 3.0], [-w, 3.5, 3.0]],
        );
        quad(
            &mut p,
            &mut i,
            [
                [-w, -1.0, top],
                [w, -1.0, top],
                [w, 3.5, top],
                [-w, 3.5, top],
            ],
        );
        quad(
            &mut p,
            &mut i,
            [
                [-w, -1.0, under],
                [-w, 3.5, under],
                [w, 3.5, under],
                [w, -1.0, under],
            ],
        );
        quad(
            &mut p,
            &mut i,
            [
                [-w, -1.0, under],
                [-w, -1.0, top],
                [w, -1.0, top],
                [w, -1.0, under],
            ],
        );
        World::new(&p, &i)
    }

    #[test]
    fn walking_up_a_slope_under_something_low_stops_under_it() {
        for under in [0.8f32, 0.85, 0.9, 1.0, 1.2] {
            let w = slope_under_slab(under);
            for dir in [
                Vec2::new(0.87, 0.48),
                Vec2::new(0.5, 0.86),
                Vec2::new(0.0, 1.0),
            ] {
                let mut p = player_at(0.0);
                p.position.y = -0.5;
                p.update(&w, Input::default(), 0.1);
                let input = Input {
                    movement: Vec2::new(0.0, 1.0),
                    yaw: dir.y.atan2(dir.x),
                    ..Default::default()
                };
                for _ in 0..240 {
                    p.update(&w, input, 1.0 / 60.0);
                    assert!(
                        p.position.z + p.height() < under + 2.0 * SNUG,
                        "{under} high, going {dir}: in the slab at {}",
                        p.position
                    );
                }
                assert!(p.grounded, "{under} high, going {dir}: at {}", p.position);
                if dir.x > 0.8 {
                    // Not stuck where it meets it: it slides along.
                    assert!(p.position.x > 1.5, "{under} high: at {}", p.position);
                }
            }
        }
    }
}
