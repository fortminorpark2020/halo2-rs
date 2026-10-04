//! Vehicles: Warthogs, Ghosts, Banshees and turrets. Each is a rigid body
//! with the hull from its physics model, held up by wheels on springs or
//! hover pads (or flying), and driven the way Halo 2 drives them: toward
//! where the driver looks.

use crate::collision::{Contact, World};
use crate::player::GRAVITY;
use crate::weapon::WeaponState;
use glam::{Mat3, Mat4, Quat, Vec2, Vec3};

/// Physics steps per game tick.
const SUBSTEPS: usize = 2;
/// How far wheels travel up and down from rest.
pub const SUSPENSION_TRAVEL: f32 = 0.15;
/// Fraction of critical damping for springs.
const SPRING_DAMPING: f32 = 0.55;
/// Sideways grip of tyres, as a fraction of the load on them.
const TIRE_GRIP: f32 = 1.25;
/// Slowing when rolling with no throttle (world units per second squared).
const COAST_DECELERATION: f32 = 0.8;
/// How much faster a boosting Ghost or Banshee goes.
pub const BOOST_SCALE: f32 = 1.75;
/// Bounce and slide off the level.
const RESTITUTION: f32 = 0.1;
const FRICTION: f32 = 0.5;
/// Seconds lying still before a vehicle stops simulating.
const SLEEP_AFTER: f32 = 1.0;

/// How a vehicle moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Drive {
    /// Sprung wheels: the Warthog.
    #[default]
    Wheels,
    /// Treads: the Scorpion. Turns on the spot toward where the driver
    /// looks, as it drives.
    Tank,
    /// Hover pads: the Ghost.
    Hover,
    /// Flies: the Banshee.
    Fly,
    /// Stays where it was placed: turrets.
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatRole {
    Driver,
    Gunner,
    Passenger,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeatDef {
    pub role: SeatRole,
    /// Where the rider sits, in the vehicle's space (on a turret, with the
    /// turret facing straight ahead).
    pub position: Vec3,
    /// Where to stand to get in.
    pub entry: Vec3,
    pub entry_radius: f32,
    /// Where the rider looks and shoots from.
    pub eye: Vec3,
    /// The rider can be shot (not shut inside).
    pub exposed: bool,
    /// The rider's view is a camera following the vehicle.
    pub third_person: bool,
    /// The weapon fired from this seat (index into the game's weapons), and
    /// its second trigger (the Scorpion's machine gun, the Banshee's bomb).
    pub weapon: Option<usize>,
    pub alt_weapon: Option<usize>,
    /// On a turret: the seat turns with the turret's aim around this point.
    pub pivot: Option<Vec3>,
    /// The turret gun this seat aims (a gunner's, or the Scorpion and Wraith
    /// driver's).
    pub turret: Option<TurretGun>,
    /// How far the turret may aim down and up (radians).
    pub pitch_range: [f32; 2],
    /// The rider's animations ("warthog_d"...).
    pub animation: String,
    /// Where a third person camera sits for each look pitch, from looking
    /// down to looking up: (pitch, offset from the eye with x along the
    /// look and z up). Empty if the seat has no camera track.
    pub camera: Vec<(f32, Vec3)>,
}

impl SeatDef {
    /// The camera's offset from the eye while looking along `yaw` and
    /// `pitch`, if the seat has a camera track: smoothly between its points.
    pub fn camera_offset(&self, yaw: f32, pitch: f32) -> Option<Vec3> {
        let track = &self.camera;
        let last = track.len().checked_sub(1)?;
        let k = track
            .iter()
            .position(|p| p.0 > pitch)
            .unwrap_or(last + 1)
            .clamp(1, last.max(1))
            - 1;
        let at = |j: usize| track[j.min(last)].1;
        let (p0, p1) = (track[k].0, track[(k + 1).min(last)].0);
        let t = ((pitch - p0) / (p1 - p0).max(1e-4)).clamp(0.0, 1.0);
        // Catmull-Rom through the points either side.
        let (a, b, c, d) = (at(k.saturating_sub(1)), at(k), at(k + 1), at(k + 2));
        let local = 0.5
            * (2.0 * b
                + (c - a) * t
                + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t
                + (3.0 * b - a - 3.0 * c + d) * t * t * t);
        Some(Quat::from_rotation_z(yaw) * local)
    }
}

/// A gun on a turret.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurretGun {
    /// Where it turns, in the vehicle's space.
    pub pivot: Vec3,
    /// The muzzle, from the pivot, with the gun pointing straight ahead.
    pub muzzle: Vec3,
    /// How far above the aim the barrel points (radians): the Wraith's
    /// mortar lobs its shots.
    pub elevation: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wheel {
    /// The wheel's centre at rest, in the vehicle's space.
    pub position: Vec3,
    pub radius: f32,
    pub powered: bool,
    pub steers: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoverPad {
    pub position: Vec3,
    /// How high the pad pushes off the ground.
    pub height: f32,
    /// Its part of the vehicle's weight.
    pub share: f32,
}

/// A box of the hull, in the vehicle's space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HullBox {
    pub center: Vec3,
    pub half_extents: Vec3,
    /// The box's axes (columns).
    pub axes: Mat3,
}

impl HullBox {
    /// Distance along a ray (in the vehicle's space) to the box, if it hits.
    pub fn raycast(&self, origin: Vec3, dir: Vec3) -> Option<f32> {
        let o = self.axes.transpose() * (origin - self.center);
        let d = self.axes.transpose() * dir;
        let (mut near, mut far) = (f32::MIN, f32::MAX);
        for k in 0..3 {
            let h = self.half_extents[k];
            if d[k].abs() < 1e-8 {
                if o[k].abs() > h {
                    return None;
                }
                continue;
            }
            let (a, b) = ((-h - o[k]) / d[k], (h - o[k]) / d[k]);
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
        (near <= far && far >= 0.0).then_some(near.max(0.0))
    }

    /// The closest point of the box to `p` (vehicle space).
    pub fn closest_point(&self, p: Vec3) -> Vec3 {
        let local = self.axes.transpose() * (p - self.center);
        self.center + self.axes * local.clamp(-self.half_extents, self.half_extents)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct VehicleDef {
    pub name: String,
    pub drive: Drive,
    pub mass: f32,
    pub health: f32,
    pub hull: Vec<HullBox>,
    pub seats: Vec<SeatDef>,
    pub wheels: Vec<Wheel>,
    pub pads: Vec<HoverPad>,
    pub max_forward_speed: f32,
    pub max_reverse_speed: f32,
    pub acceleration: f32,
    pub deceleration: f32,
    /// Wheels: how far the front wheels turn; hovering and flying: the
    /// fastest turn (radians, or radians per second).
    pub max_turn: f32,
    pub max_slide: f32,
    pub gravity_scale: f32,
    /// Spheres covering the hull, for hitting the level (centre, radius).
    pub spheres: Vec<(Vec3, f32)>,
    /// Centre of mass, in the vehicle's space.
    pub center: Vec3,
    /// Principal moments of inertia.
    pub inertia: Vec3,
    /// Radius around `center` that holds the whole hull.
    pub radius: f32,
}

impl VehicleDef {
    /// Work out the collision spheres, centre of mass and inertia from the
    /// hull boxes and mass.
    pub fn with_hull(mut self, hull: Vec<HullBox>) -> VehicleDef {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let mut spheres = Vec::new();
        for b in &hull {
            for corner in 0..8 {
                let s = Vec3::new(
                    if corner & 1 == 0 { -1.0 } else { 1.0 },
                    if corner & 2 == 0 { -1.0 } else { 1.0 },
                    if corner & 4 == 0 { -1.0 } else { 1.0 },
                );
                let p = b.center + b.axes * (s * b.half_extents);
                lo = lo.min(p);
                hi = hi.max(p);
            }
            spheres.extend(box_spheres(b));
        }
        if hull.is_empty() {
            (lo, hi) = (Vec3::new(-0.3, -0.3, 0.0), Vec3::new(0.3, 0.3, 0.5));
            spheres.push((Vec3::new(0.0, 0.0, 0.25), 0.25));
        }
        let size = hi - lo;
        let m = self.mass.max(1.0);
        // Weight sits low, in the chassis and engine.
        self.center = (lo + hi) * 0.5 - Vec3::Z * size.z * 0.2;
        self.inertia = Vec3::new(
            m / 12.0 * (size.y * size.y + size.z * size.z),
            m / 12.0 * (size.x * size.x + size.z * size.z),
            m / 12.0 * (size.x * size.x + size.y * size.y),
        )
        .max(Vec3::splat(1e-3));
        self.radius = spheres
            .iter()
            .map(|(c, r)| c.distance(self.center) + r)
            .fold(0.0, f32::max);
        self.spheres = spheres;
        self.hull = hull;
        self
    }

    pub fn driver_seat(&self) -> Option<usize> {
        self.seats.iter().position(|s| s.role == SeatRole::Driver)
    }
}

/// Spheres filling a box: as big as its thinnest side, along the others.
fn box_spheres(b: &HullBox) -> Vec<(Vec3, f32)> {
    let h = b.half_extents;
    let r = h.min_element().max(0.05);
    let along = |e: f32| -> Vec<f32> {
        let reach = e - r;
        if reach <= r * 0.1 {
            return vec![0.0];
        }
        let n = (reach * 2.0 / r).ceil() as usize + 1;
        (0..n)
            .map(|k| -reach + 2.0 * reach * k as f32 / (n - 1) as f32)
            .collect()
    };
    let (xs, ys, zs) = (along(h.x), along(h.y), along(h.z));
    let mut out = Vec::new();
    for &x in &xs {
        for &y in &ys {
            for &z in &zs {
                out.push((b.center + b.axes * Vec3::new(x, y, z), r));
            }
        }
    }
    out
}

/// A driver's (or a gunner's) controls for a tick.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Controls {
    /// x: right, y: forward.
    pub throttle: Vec2,
    /// Where the driver looks: yaw around +z, pitch up.
    pub yaw: f32,
    pub pitch: f32,
    pub boost: bool,
    /// Someone is driving.
    pub driven: bool,
}

#[derive(Debug, Clone)]
pub struct Vehicle {
    pub def: usize,
    /// Centre of mass, in the world.
    pub center: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    /// Angular velocity (radians per second, world axes).
    pub spin: Vec3,
    pub health: f32,
    /// Who sits in each seat.
    pub riders: Vec<Option<usize>>,
    /// Each seat's weapon, and its second trigger's.
    pub weapons: Vec<Option<WeaponState>>,
    pub alt_weapons: Vec<Option<WeaponState>>,
    /// The turret's aim, relative to the vehicle: yaw and pitch.
    pub aim: Vec2,
    /// How far the front wheels are turned (radians, left positive).
    pub steer: f32,
    /// How far the wheels have rolled (radians).
    pub roll: f32,
    /// Each wheel's (or pad's) spring, 0 fully out to 1 fully in.
    pub compression: Vec<f32>,
    pub controls: Controls,
    /// Fastest the hull has hit something this tick (for crash damage).
    pub impact: f32,
    pub asleep: bool,
    /// Blown up, waiting to come back.
    pub destroyed: bool,
    pub respawn_in: f32,
    /// Seconds left empty away from its spawn point (or on its roof).
    pub abandoned: f32,
    /// Seconds spent on its side or roof.
    pub overturned: f32,
    /// Who drove it last, credited with its splatters.
    pub last_driver: Option<usize>,
    still: f32,
    contacts: Vec<Contact>,
}

impl Vehicle {
    /// A vehicle at rest with its origin at `position`, facing `yaw`.
    pub fn new(def_index: usize, def: &VehicleDef, position: Vec3, yaw: f32) -> Vehicle {
        let rotation = Quat::from_rotation_z(yaw);
        Vehicle {
            def: def_index,
            center: position + rotation * def.center,
            rotation,
            velocity: Vec3::ZERO,
            spin: Vec3::ZERO,
            health: def.health,
            riders: vec![None; def.seats.len()],
            weapons: def.seats.iter().map(|_| None).collect(),
            alt_weapons: def.seats.iter().map(|_| None).collect(),
            aim: Vec2::ZERO,
            steer: 0.0,
            roll: 0.0,
            compression: vec![0.0; def.wheels.len().max(def.pads.len())],
            controls: Controls::default(),
            impact: 0.0,
            asleep: false,
            destroyed: false,
            respawn_in: 0.0,
            abandoned: 0.0,
            overturned: 0.0,
            last_driver: None,
            still: 0.0,
            contacts: Vec::new(),
        }
    }

    /// The model's origin in the world.
    pub fn origin(&self, def: &VehicleDef) -> Vec3 {
        self.center - self.rotation * def.center
    }

    /// The model's placement in the world.
    pub fn transform(&self, def: &VehicleDef) -> Mat4 {
        Mat4::from_rotation_translation(self.rotation, self.origin(def))
    }

    /// A point in the vehicle's space, in the world.
    pub fn to_world(&self, def: &VehicleDef, local: Vec3) -> Vec3 {
        self.center + self.rotation * (local - def.center)
    }

    /// A world point in the vehicle's space.
    pub fn to_local(&self, def: &VehicleDef, world: Vec3) -> Vec3 {
        self.rotation.inverse() * (world - self.center) + def.center
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Z
    }

    /// Facing around +z.
    pub fn yaw(&self) -> f32 {
        let f = self.forward();
        f.y.atan2(f.x)
    }

    /// Turned over: lying on its side or roof.
    pub fn upside_down(&self) -> bool {
        self.up().z < 0.3
    }

    /// The turret's rotation (yaw only) about its pivot, in vehicle space.
    pub fn turret_turn(&self) -> Quat {
        Quat::from_rotation_z(self.aim.x)
    }

    /// Where a seat's rider sits, in the world.
    pub fn seat_position(&self, def: &VehicleDef, seat: usize) -> Vec3 {
        let s = &def.seats[seat];
        self.to_world(def, self.seat_local(s, s.position))
    }

    /// Where a seat's rider looks from, in the world.
    pub fn seat_eye(&self, def: &VehicleDef, seat: usize) -> Vec3 {
        let s = &def.seats[seat];
        self.to_world(def, self.seat_local(s, s.eye))
    }

    fn seat_local(&self, s: &SeatDef, p: Vec3) -> Vec3 {
        match s.pivot {
            Some(pivot) => pivot + self.turret_turn() * (p - pivot),
            None => p,
        }
    }

    /// Which way a seat's turret gun points, and its muzzle, in the world.
    pub fn turret_muzzle(&self, def: &VehicleDef, seat: usize) -> Option<(Vec3, Vec3)> {
        let gun = def.seats.get(seat)?.turret?;
        let (yaw, pitch) = (self.aim.x, self.aim.y + gun.elevation);
        let dir = self.rotation
            * Vec3::new(
                yaw.cos() * pitch.cos(),
                yaw.sin() * pitch.cos(),
                pitch.sin(),
            );
        let turn = Quat::from_rotation_z(self.aim.x) * Quat::from_rotation_y(-self.aim.y);
        Some((dir, self.to_world(def, gun.pivot + turn * gun.muzzle)))
    }

    /// Velocity of the body at a world point.
    pub fn point_velocity(&self, p: Vec3) -> Vec3 {
        self.velocity + self.spin.cross(p - self.center)
    }

    pub fn speed(&self) -> f32 {
        self.velocity.length()
    }

    fn inverse_inertia(&self, def: &VehicleDef, v: Vec3) -> Vec3 {
        let local = self.rotation.inverse() * v;
        self.rotation * (local / def.inertia)
    }

    /// Push the body with an impulse at a world point.
    pub fn apply_impulse(&mut self, def: &VehicleDef, impulse: Vec3, at: Vec3) {
        self.velocity += impulse / def.mass.max(1.0);
        let torque = (at - self.center).cross(impulse);
        self.spin += self.inverse_inertia(def, torque);
        self.asleep = false;
    }

    /// The impulse along `n` that stops the point `at` moving along it,
    /// scaled by (1 + restitution).
    fn normal_impulse(&self, def: &VehicleDef, at: Vec3, n: Vec3, approach: f32, e: f32) -> f32 {
        let r = at - self.center;
        let k = 1.0 / def.mass.max(1.0) + n.dot(self.inverse_inertia(def, r.cross(n)).cross(r));
        -(1.0 + e) * approach / k.max(1e-6)
    }

    /// Advance one game tick.
    pub fn step(&mut self, def: &VehicleDef, world: &World, dt: f32) {
        self.impact = 0.0;
        if def.drive == Drive::Fixed {
            return;
        }
        if self.asleep && !self.controls.driven {
            return;
        }
        let h = dt / SUBSTEPS as f32;
        for _ in 0..SUBSTEPS {
            self.substep(def, world, h);
        }
        // Rest once lying still with no one driving.
        let calm = self.velocity.length() < 0.05 && self.spin.length() < 0.05;
        self.still = if calm && !self.controls.driven {
            self.still + dt
        } else {
            0.0
        };
        if self.still > SLEEP_AFTER {
            self.asleep = true;
            self.velocity = Vec3::ZERO;
            self.spin = Vec3::ZERO;
        }
    }

    fn substep(&mut self, def: &VehicleDef, world: &World, dt: f32) {
        let m = def.mass.max(1.0);
        let gravity = GRAVITY * def.gravity_scale;
        let mut force = Vec3::new(0.0, 0.0, -gravity * m);
        let mut torque = Vec3::ZERO;
        match def.drive {
            Drive::Wheels | Drive::Tank => {
                self.wheel_forces(def, world, dt, &mut force, &mut torque)
            }
            Drive::Hover => self.hover_forces(def, world, dt, &mut force, &mut torque),
            Drive::Fly => self.flight_forces(def, world, dt, &mut force, &mut torque),
            Drive::Fixed => return,
        }
        self.velocity += force / m * dt;
        self.spin += self.inverse_inertia(def, torque) * dt;
        // Air and joints soak up some spin.
        self.spin *= 1.0 - (0.6 * dt).min(1.0);
        self.center += self.velocity * dt;
        let turn = self.spin * dt;
        if turn.length_squared() > 1e-12 {
            self.rotation = (Quat::from_scaled_axis(turn) * self.rotation).normalize();
        }
        self.collide(def, world);
    }

    /// Driving force toward the target speed along `dir`, for a vehicle
    /// going `speed` along it (world units per second squared).
    fn drive_acceleration(&self, def: &VehicleDef, speed: f32, dt: f32) -> f32 {
        let c = self.controls;
        let throttle = if c.driven { c.throttle.y } else { 0.0 };
        let top = if throttle >= 0.0 {
            def.max_forward_speed * if c.boost { BOOST_SCALE } else { 1.0 }
        } else {
            def.max_reverse_speed
        };
        let target = throttle * top;
        let limit = speed.abs() / dt;
        if throttle.abs() < 0.05 {
            -speed.signum() * COAST_DECELERATION.min(limit)
        } else if speed * target < 0.0 {
            // Braking, then on the other way.
            target.signum() * def.deceleration.min((target - speed).abs() / dt)
        } else if speed.abs() > target.abs() {
            // Over the top speed.
            -speed.signum() * def.deceleration.min(limit)
        } else {
            target.signum() * def.acceleration.min((target - speed).abs() / dt)
        }
    }

    fn wheel_forces(
        &mut self,
        def: &VehicleDef,
        world: &World,
        dt: f32,
        force: &mut Vec3,
        torque: &mut Vec3,
    ) {
        let m = def.mass.max(1.0);
        let n = def.wheels.len().max(1) as f32;
        let share = m / n;
        let g = GRAVITY * def.gravity_scale;
        let k = share * g / SUSPENSION_TRAVEL;
        let damping = 2.0 * SPRING_DAMPING * (k * share).sqrt();
        let up = self.up();
        let fwd = self.forward();

        // Steer toward where the driver looks (backing up, the other way).
        // Tanks turn the whole hull instead, as they drive.
        let c = self.controls;
        let speed = self.velocity.dot(fwd);
        let tank = def.drive == Drive::Tank;
        let mut target = 0.0;
        let mut accel = self.drive_acceleration(def, speed, dt);
        if tank {
            if c.driven && c.throttle.y.abs() > 0.05 {
                self.turn_toward(def, c.yaw, None, torque);
                // Swing round before setting off.
                let err = wrap_angle(c.yaw - self.yaw()).abs();
                accel *= (1.0 - err / 1.2).max(0.0);
            } else {
                // Treads hold it still.
                let local = self.rotation.inverse() * (Vec3::Z * -self.spin.z * 8.0);
                *torque += self.rotation * (local * def.inertia);
            }
        } else if c.driven {
            let err = wrap_angle(c.yaw - self.yaw());
            let reversing = c.throttle.y < -0.05 || (c.throttle.y.abs() < 0.05 && speed < -0.5);
            target = if reversing { -err } else { err } * 1.5;
            target = target.clamp(-def.max_turn, def.max_turn);
        }
        let turn_rate = 6.0;
        self.steer += (target - self.steer).clamp(-turn_rate * dt, turn_rate * dt);
        let powered = def.wheels.iter().filter(|w| w.powered).count().max(1) as f32;

        let mut rolling = 0.0;
        let mut touching = 0.0;
        for (i, w) in def.wheels.iter().enumerate() {
            let top = self.to_world(def, w.position + Vec3::Z * SUSPENSION_TRAVEL);
            let reach = 2.0 * SUSPENSION_TRAVEL + w.radius;
            let Some((d, normal)) = world.raycast_hit(top, -up, reach) else {
                self.compression[i] = 0.0;
                continue;
            };
            let s = (reach - d).clamp(0.0, 2.0 * SUSPENSION_TRAVEL);
            self.compression[i] = s / (2.0 * SUSPENSION_TRAVEL);
            let ground = top - up * d;
            let arm = ground - self.center;
            let v = self.point_velocity(ground);
            let load = (k * s - damping * v.dot(up)).max(0.0);
            let mut f = up * load;

            // Tyres: grip sideways, drive and brake along.
            let heading = if w.steers {
                Quat::from_axis_angle(up, self.steer) * fwd
            } else {
                fwd
            };
            let along = (heading - normal * heading.dot(normal)).normalize_or(fwd);
            let side = normal.cross(along);
            // Treads skid round as the hull turns; they only stop it sliding.
            let slip = if tank {
                self.velocity.dot(side)
            } else {
                v.dot(side)
            };
            let grip = TIRE_GRIP * load;
            f += side * (-slip * share / dt * 0.5).clamp(-grip, grip);
            if w.powered {
                let drive = accel * m / powered;
                f += along * drive.clamp(-grip, grip);
            }
            rolling += v.dot(along);
            touching += 1.0;
            *force += f;
            *torque += arm.cross(up * load);
            // Tyre forces act at the axle: hard cornering leans the body
            // without tipping it over.
            let axle = ground + up * w.radius - self.center;
            *torque += axle.cross(f - up * load);
        }
        if touching > 0.0 {
            let radius = def.wheels.first().map_or(0.25, |w| w.radius).max(0.05);
            self.roll += rolling / touching / radius * dt;
        }
    }

    fn hover_forces(
        &mut self,
        def: &VehicleDef,
        world: &World,
        dt: f32,
        force: &mut Vec3,
        torque: &mut Vec3,
    ) {
        let m = def.mass.max(1.0);
        let g = GRAVITY * def.gravity_scale;
        let up = self.up();
        let mut hovering = false;
        for (i, pad) in def.pads.iter().enumerate() {
            let at = self.to_world(def, pad.position);
            let reach = pad.height * 1.5;
            let rest = pad.height * 0.75;
            let Some((d, _)) = world.raycast_hit(at, -up, reach) else {
                self.compression[i] = 0.0;
                continue;
            };
            hovering = true;
            self.compression[i] = 1.0 - d / reach;
            let weight = m * g * pad.share;
            let k = weight / (reach - rest);
            let damping = 2.0 * SPRING_DAMPING * (k * m * pad.share).sqrt();
            let v = self.point_velocity(at);
            let push = (k * (reach - d) - damping * v.dot(up)).max(0.0);
            *force += up * push;
            *torque += (at - self.center).cross(up * push);
        }
        let c = self.controls;
        if !c.driven {
            // Sit still: slide to a stop.
            let flat = Vec3::new(self.velocity.x, self.velocity.y, 0.0);
            let stop = (flat / dt).clamp_length_max(def.deceleration);
            if hovering {
                *force -= stop * m;
            }
            return;
        }
        // Move along the ground: forward and back, strafing to the sides.
        let fwd = flat(self.forward());
        let right = Vec3::new(fwd.y, -fwd.x, 0.0);
        let boost = if c.boost && c.throttle.y > 0.0 {
            BOOST_SCALE
        } else {
            1.0
        };
        let ahead = if c.throttle.y >= 0.0 {
            def.max_forward_speed * boost
        } else {
            def.max_reverse_speed
        };
        let target = fwd * c.throttle.y * ahead + right * c.throttle.x * def.max_slide;
        let now = Vec3::new(self.velocity.x, self.velocity.y, 0.0);
        let rate = if target.length() > now.length() {
            def.acceleration * boost
        } else {
            def.deceleration
        };
        let change = ((target - now) / dt).clamp_length_max(rate);
        if hovering {
            *force += change * m;
        } else {
            *force += change * m * 0.2;
        }
        self.turn_toward(def, c.yaw, None, torque);
    }

    fn flight_forces(
        &mut self,
        def: &VehicleDef,
        _world: &World,
        dt: f32,
        force: &mut Vec3,
        torque: &mut Vec3,
    ) {
        let c = self.controls;
        if !c.driven {
            return;
        }
        let m = def.mass.max(1.0);
        // Hold height against gravity.
        force.z += GRAVITY * def.gravity_scale * m;
        let pitch = c.pitch.clamp(-1.2, 1.2);
        let aim = Vec3::new(
            c.yaw.cos() * pitch.cos(),
            c.yaw.sin() * pitch.cos(),
            pitch.sin(),
        );
        let right = Vec3::new(c.yaw.sin(), -c.yaw.cos(), 0.0);
        let boost = if c.boost { BOOST_SCALE } else { 1.0 };
        let target = aim * c.throttle.y.max(0.0) * def.max_forward_speed * boost
            + right * c.throttle.x * def.max_slide;
        let rate = if target.length() > self.velocity.length() {
            def.acceleration.min(6.0) * boost
        } else {
            def.deceleration.min(8.0)
        };
        *force += ((target - self.velocity) / dt).clamp_length_max(rate) * m;
        self.turn_toward(def, c.yaw, Some(pitch), torque);
    }

    /// Torque to turn toward a heading (and, flying, a pitch, banking into
    /// turns).
    fn turn_toward(&self, def: &VehicleDef, yaw: f32, pitch: Option<f32>, torque: &mut Vec3) {
        let err = wrap_angle(yaw - self.yaw());
        let rate = (err * 4.0).clamp(-def.max_turn, def.max_turn);
        let want = match pitch {
            None => Vec3::Z * rate,
            Some(pitch) => {
                // Orientation wanted: the heading and pitch, banked into the turn.
                let bank = (-rate * 0.25).clamp(-0.6, 0.6);
                let target = Quat::from_rotation_z(self.yaw() + err)
                    * Quat::from_rotation_y(-pitch)
                    * Quat::from_rotation_x(bank);
                let (axis, angle) = (target * self.rotation.inverse()).to_axis_angle();
                let angle = if angle > std::f32::consts::PI {
                    angle - std::f32::consts::TAU
                } else {
                    angle
                };
                let mut w = axis * angle * 4.0;
                w.z = rate;
                w
            }
        };
        let response = 8.0;
        let delta = match pitch {
            None => Vec3::Z * (want.z - self.spin.z),
            Some(_) => want - self.spin,
        };
        let local = self.rotation.inverse() * (delta * response);
        *torque += self.rotation * (local * def.inertia);
    }

    /// Push the hull out of the level, bouncing and sliding off it.
    fn collide(&mut self, def: &VehicleDef, world: &World) {
        let mut contacts = std::mem::take(&mut self.contacts);
        for _ in 0..3 {
            let mut deepest: Option<(Vec3, f32, Contact)> = None;
            for &(local, r) in &def.spheres {
                let c = self.to_world(def, local);
                world.capsule_contacts(c, c, r, &mut contacts);
                for &hit in &contacts {
                    if deepest.is_none_or(|d| hit.depth > d.2.depth) {
                        deepest = Some((c, r, hit));
                    }
                }
            }
            let Some((c, r, hit)) = deepest else { break };
            let n = hit.normal;
            self.center += n * hit.depth;
            let at = c - n * (r - hit.depth);
            let v = self.point_velocity(at);
            let approach = v.dot(n);
            if approach < 0.0 {
                self.impact = self.impact.max(-approach);
                let j = self.normal_impulse(def, at, n, approach, RESTITUTION);
                self.apply_impulse(def, n * j, at);
                let v = self.point_velocity(at);
                let slide = v - n * v.dot(n);
                if slide.length_squared() > 1e-6 {
                    let t = slide.normalize();
                    let jt = self.normal_impulse(def, at, t, slide.length(), 0.0);
                    self.apply_impulse(def, t * jt.max(-FRICTION * j), at);
                }
            }
        }
        self.contacts = contacts;
    }

    /// Set upright where it lies (a player flipping it back over).
    pub fn right_itself(&mut self) {
        self.rotation = Quat::from_rotation_z(self.yaw());
        self.center += Vec3::Z * 0.3;
        self.spin = Vec3::ZERO;
        self.asleep = false;
    }
}

/// The horizontal part of a direction, unit length.
fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, 0.0).normalize_or(Vec3::X)
}

/// An angle in -pi..pi.
pub fn wrap_angle(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    (a + std::f32::consts::PI).rem_euclid(t) - std::f32::consts::PI
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testing::floor;

    pub(crate) fn hull(center: Vec3, half: Vec3) -> HullBox {
        HullBox {
            center,
            half_extents: half,
            axes: Mat3::IDENTITY,
        }
    }

    /// A Warthog-sized jeep.
    pub(crate) fn jeep() -> VehicleDef {
        let wheel = |x: f32, y: f32, front: bool| Wheel {
            position: Vec3::new(x, y, 0.19),
            radius: 0.2,
            powered: true,
            steers: front,
        };
        VehicleDef {
            name: "jeep".into(),
            drive: Drive::Wheels,
            mass: 3800.0,
            health: 250.0,
            wheels: vec![
                wheel(0.67, 0.39, true),
                wheel(0.67, -0.39, true),
                wheel(-0.63, 0.39, false),
                wheel(-0.63, -0.39, false),
            ],
            seats: vec![SeatDef {
                role: SeatRole::Driver,
                position: Vec3::new(0.0, 0.17, 0.39),
                entry: Vec3::new(0.07, 0.26, 0.45),
                entry_radius: 1.25,
                eye: Vec3::new(0.0, 0.17, 0.63),
                exposed: true,
                third_person: true,
                weapon: None,
                alt_weapon: None,
                camera: Vec::new(),
                pivot: None,
                turret: None,
                pitch_range: [-0.8, 0.8],
                animation: "warthog_d".into(),
            }],
            max_forward_speed: 7.65,
            max_reverse_speed: 3.0,
            acceleration: 2.475,
            deceleration: 9.9,
            max_turn: 50f32.to_radians(),
            gravity_scale: 1.0,
            ..VehicleDef::default()
        }
        .with_hull(vec![hull(
            Vec3::new(0.02, 0.0, 0.45),
            Vec3::new(0.9, 0.43, 0.27),
        )])
    }

    /// A Ghost-sized hovercraft.
    pub(crate) fn hovercraft() -> VehicleDef {
        let pad = |x: f32, y: f32, share: f32| HoverPad {
            position: Vec3::new(x, y, 0.1),
            height: 0.5,
            share,
        };
        VehicleDef {
            name: "hover".into(),
            drive: Drive::Hover,
            mass: 1500.0,
            health: 175.0,
            pads: vec![
                pad(-0.27, 0.0, 0.5),
                pad(0.68, 0.4, 0.25),
                pad(0.68, -0.4, 0.25),
            ],
            max_forward_speed: 5.0,
            max_reverse_speed: 2.5,
            acceleration: 6.0,
            deceleration: 8.0,
            max_turn: 3.0,
            max_slide: 2.5,
            gravity_scale: 1.0,
            ..VehicleDef::default()
        }
        .with_hull(vec![hull(
            Vec3::new(0.2, 0.0, 0.2),
            Vec3::new(0.6, 0.23, 0.22),
        )])
    }

    fn run(v: &mut Vehicle, def: &VehicleDef, seconds: f32) {
        let world = floor();
        for _ in 0..(seconds * 60.0) as usize {
            v.step(def, &world, 1.0 / 60.0);
        }
    }

    #[test]
    fn the_camera_rises_as_you_look_down() {
        let mut seat = jeep().seats[0].clone();
        assert_eq!(seat.camera_offset(0.0, 0.0), None);
        let deg = f32::to_radians;
        seat.camera = vec![
            (deg(-90.0), Vec3::new(1.0, 0.0, 3.0)),
            (deg(0.0), Vec3::new(-4.0, 0.0, 1.0)),
            (deg(90.0), Vec3::new(-1.0, 0.0, 0.0)),
        ];
        let level = seat.camera_offset(0.0, 0.0).unwrap();
        assert!(level.distance(Vec3::new(-4.0, 0.0, 1.0)) < 1e-4);
        // Past the ends it stays at the end points.
        let down = seat.camera_offset(0.0, deg(-120.0)).unwrap();
        assert!(down.distance(Vec3::new(1.0, 0.0, 3.0)) < 1e-4);
        let between = seat.camera_offset(0.0, deg(-45.0)).unwrap();
        assert!(between.z > 1.0 && between.z < 3.0, "{between}");
        // It turns with the look.
        let side = seat.camera_offset(deg(90.0), 0.0).unwrap();
        assert!(side.distance(Vec3::new(0.0, -4.0, 1.0)) < 1e-4, "{side}");
    }

    #[test]
    fn a_parked_jeep_settles_on_its_wheels() {
        let def = jeep();
        let mut v = Vehicle::new(0, &def, Vec3::new(0.0, 0.0, 0.3), 0.0);
        run(&mut v, &def, 4.0);
        let origin = v.origin(&def);
        // Wheels of radius 0.2 at 0.19 rest just about on the ground.
        assert!(origin.z.abs() < 0.08, "origin {origin}");
        assert!(v.up().z > 0.99);
        assert!(v.asleep, "comes to rest");
    }

    #[test]
    fn the_jeep_drives_up_to_top_speed_and_steers_toward_the_view() {
        let def = jeep();
        let mut v = Vehicle::new(0, &def, Vec3::new(0.0, 0.0, 0.05), 0.0);
        v.controls = Controls {
            throttle: Vec2::new(0.0, 1.0),
            yaw: 0.0,
            driven: true,
            ..Controls::default()
        };
        run(&mut v, &def, 5.0);
        let speed = v.velocity.dot(v.forward());
        assert!((speed - 7.65).abs() < 0.4, "speed {speed}");
        assert!(v.origin(&def).y.abs() < 0.5, "drives straight");
        v.controls.yaw = std::f32::consts::FRAC_PI_2;
        run(&mut v, &def, 4.0);
        let heading = v.yaw();
        assert!(
            (heading - std::f32::consts::FRAC_PI_2).abs() < 0.3,
            "turned to {heading}"
        );
        assert!(v.up().z > 0.9, "stays on its wheels");
    }

    #[test]
    fn the_jeep_brakes_and_reverses() {
        let def = jeep();
        let mut v = Vehicle::new(0, &def, Vec3::new(0.0, 0.0, 0.05), 0.0);
        v.controls = Controls {
            throttle: Vec2::new(0.0, -1.0),
            driven: true,
            ..Controls::default()
        };
        run(&mut v, &def, 4.0);
        let speed = v.velocity.dot(v.forward());
        assert!((speed + 3.0).abs() < 0.3, "reverse speed {speed}");
        // Throttle forward while rolling back: brakes, then drives forward.
        v.controls.throttle = Vec2::new(0.0, 1.0);
        run(&mut v, &def, 4.0);
        let speed = v.velocity.dot(v.forward());
        assert!(speed > 4.0, "forward again at {speed}");
    }

    #[test]
    fn the_hovercraft_floats_and_turns_to_face_the_view() {
        let def = hovercraft();
        let mut v = Vehicle::new(0, &def, Vec3::new(0.0, 0.0, 0.3), 0.0);
        run(&mut v, &def, 3.0);
        let z = v.origin(&def).z;
        // Pads at 0.1 hold 0.375 off the ground.
        assert!((z + 0.1 - 0.375).abs() < 0.08, "hovers at {z}");
        v.controls = Controls {
            throttle: Vec2::new(0.0, 1.0),
            yaw: 1.0,
            driven: true,
            ..Controls::default()
        };
        run(&mut v, &def, 3.0);
        assert!((v.yaw() - 1.0).abs() < 0.1, "faces {}", v.yaw());
        let speed = v.velocity.truncate().length();
        assert!((speed - 5.0).abs() < 0.3, "speed {speed}");
        v.controls.boost = true;
        run(&mut v, &def, 2.0);
        let speed = v.velocity.truncate().length();
        assert!(speed > 7.0, "boosts to {speed}");
    }

    #[test]
    fn a_dropped_vehicle_lands_without_sinking() {
        let def = jeep();
        let mut v = Vehicle::new(0, &def, Vec3::new(0.0, 0.0, 3.0), 0.0);
        v.spin = Vec3::new(2.0, 1.0, 0.0);
        run(&mut v, &def, 6.0);
        for &(local, r) in &def.spheres {
            let c = v.to_world(&def, local);
            assert!(c.z > r - 0.05, "sphere at {c} sinks");
        }
    }

    #[test]
    fn hull_boxes_are_hit_by_rays() {
        let b = hull(Vec3::new(0.0, 0.0, 0.5), Vec3::new(1.0, 0.5, 0.5));
        assert_eq!(b.raycast(Vec3::new(-5.0, 0.0, 0.5), Vec3::X), Some(4.0));
        assert_eq!(b.raycast(Vec3::new(-5.0, 2.0, 0.5), Vec3::X), None);
        assert_eq!(b.raycast(Vec3::new(5.0, 0.0, 0.5), Vec3::X), None);
    }
}
