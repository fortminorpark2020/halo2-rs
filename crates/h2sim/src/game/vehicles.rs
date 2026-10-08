//! Vehicles in the game: where they appear, getting in and out, driving and
//! gunning, running people over, and blowing up.

use super::projectiles::Homing;
use super::{Command, Event, Game, SWAP_HOLD};
use crate::collision::World;
use crate::vehicle::{Controls, Drive, SeatRole, TrickKind, Vehicle, VehicleDef};
use crate::weapon::{WeaponDef, WeaponInput, WeaponState};
use blam_cache::weapon::TriggerInput;
use glam::{Vec2, Vec3};

/// Moving into someone faster than this (world units per second) kills them.
const SPLATTER_SPEED: f32 = 2.2;
/// Bullets barely scratch vehicles; explosions and heavy rounds don't.
const BULLET_DAMAGE_SCALE: f32 = 0.25;
const HEAVY_DAMAGE: f32 = 50.0;
/// Seconds on its roof before the riders bail out.
const BAIL_OUT_AFTER: f32 = 1.5;
/// How close to stand to flip an overturned vehicle back over.
const FLIP_REACH: f32 = 1.6;
/// Damage the blast of a destroyed vehicle does nearby, and how far.
const WRECK_DAMAGE: f32 = 100.0;
const WRECK_RADIUS: f32 = 2.5;
/// How far the crosshair reaches when it points at nothing.
const CROSSHAIR_RANGE: f32 = 200.0;
/// Seconds to hold the action key by an enemy tank to board it.
const TANK_BOARD_HOLD: f32 = 1.0;

/// A place a vehicle appears on the map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleSpawn {
    /// Index into the game's vehicle kinds.
    pub def: usize,
    pub position: Vec3,
    pub yaw: f32,
    /// Seconds before it comes back once destroyed or abandoned.
    pub respawn: f32,
}

/// What getting in or acting on a vehicle nearby would do (HUD prompt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VehicleAction {
    Enter {
        vehicle: usize,
        seat: usize,
    },
    /// Throw an enemy out of their seat (or, in a tank, finish them) and
    /// take it.
    Hijack {
        vehicle: usize,
        seat: usize,
    },
    Flip {
        vehicle: usize,
    },
}

impl Game {
    /// Put the map's vehicles in the game, each at its spawn point.
    pub fn set_vehicles(&mut self, defs: Vec<VehicleDef>, spawns: Vec<VehicleSpawn>) {
        self.vehicle_defs = defs;
        self.vehicles = spawns
            .iter()
            .map(|s| Vehicle::new(s.def, &self.vehicle_defs[s.def], s.position, s.yaw))
            .collect();
        self.vehicle_spawns = spawns;
        for v in 0..self.vehicles.len() {
            self.arm_vehicle(v);
        }
    }

    fn arm_vehicle(&mut self, v: usize) {
        let def = &self.vehicle_defs[self.vehicles[v].def];
        let arm = |w: Option<usize>| w.and_then(|w| self.weapons.get(w)).map(WeaponState::new);
        self.vehicles[v].weapons = def.seats.iter().map(|s| arm(s.weapon)).collect();
        self.vehicles[v].alt_weapons = def.seats.iter().map(|s| arm(s.alt_weapon)).collect();
    }

    /// Where player `i` sees from: their eyes, a seat's, or (in a seat with
    /// a camera following the vehicle) the point on the camera's line of
    /// sight nearest the seat's eye.
    pub fn view_point(&self, i: usize) -> Vec3 {
        self.view(i).0
    }

    /// How far behind the view point the camera follows player `i`'s
    /// vehicle, if it does.
    pub fn chase_distance(&self, i: usize) -> Option<f32> {
        self.view(i).1
    }

    fn view(&self, i: usize) -> (Vec3, Option<f32>) {
        let p = &self.players[i];
        let Some((v, s)) = self.riding(i) else {
            return (p.eye(), None);
        };
        let veh = &self.vehicles[v];
        let def = &self.vehicle_defs[veh.def];
        let seat = &def.seats[s];
        let eye = veh.seat_eye(def, s);
        if !seat.third_person {
            return (eye, None);
        }
        // The seat's camera track places the camera off the eye by how far
        // up or down they look (higher and closer looking down).
        if let Some(offset) = seat.camera_offset(p.yaw, p.pitch) {
            let look = p.aim();
            let camera = eye + offset;
            let back = (eye - camera).dot(look).max(0.0);
            return (camera + look * back, Some(back));
        }
        match seat.pivot {
            // A turret's gunner: over the gun, to see past them.
            Some(_) => (eye + Vec3::Z * 0.45, Some(0.8 + def.radius * 0.6)),
            None => (
                veh.center + Vec3::Z * (def.radius * 0.3 + 0.35),
                Some(1.2 + def.radius * 1.4),
            ),
        }
    }

    /// The point player `i`'s crosshair rests on, looking along `look`
    /// (unit length): the first wall, player or vehicle on the line through
    /// their view point. Seat guns fire at it, so their shots meet where
    /// the crosshair is wherever the gun sits.
    pub fn crosshair(&self, world: &World, i: usize, look: Vec3) -> Vec3 {
        let from = self.view_point(i);
        let mut t = world
            .raycast(from, look, CROSSHAIR_RANGE)
            .unwrap_or(CROSSHAIR_RANGE);
        if let Some((_, d, _)) = self.trace_players(i, from, look) {
            t = t.min(d);
        }
        if let Some((_, d)) = self.trace_vehicles(i, from, look) {
            t = t.min(d);
        }
        from + look * t
    }

    /// The vehicle player `i` rides and their seat.
    pub fn riding(&self, i: usize) -> Option<(usize, usize)> {
        self.players.get(i).and_then(|p| p.seat)
    }

    /// What holding the action key would do to a vehicle nearby.
    pub fn vehicle_action(&self, i: usize) -> Option<VehicleAction> {
        let p = self.players.get(i)?;
        if !p.alive || p.seat.is_some() {
            return None;
        }
        let me = p.body.position + Vec3::Z * 0.4;
        let mut best: Option<(f32, VehicleAction)> = None;
        for (v, veh) in self.vehicles.iter().enumerate() {
            if veh.destroyed {
                continue;
            }
            let def = &self.vehicle_defs[veh.def];
            if me.distance(veh.center) > def.radius + 2.0 {
                continue;
            }
            if veh.upside_down() {
                let empty = veh.riders.iter().all(Option::is_none);
                if empty && me.distance(veh.center) < def.radius + FLIP_REACH {
                    best.get_or_insert((0.0, VehicleAction::Flip { vehicle: v }));
                }
                continue;
            }
            for (s, seat) in def.seats.iter().enumerate() {
                if seat.ai_only {
                    continue;
                }
                // An enemy driving or on the gun can be boarded.
                let hijack = match veh.riders[s] {
                    None => false,
                    Some(r) if seat.role != SeatRole::Passenger && self.is_enemy(i, r) => true,
                    Some(_) => continue,
                };
                // Hands full with the flag: ride along, nothing more.
                if p.objective.is_some() && seat.role != SeatRole::Passenger {
                    continue;
                }
                // Near where the seat is got into, or the seat itself
                // (standing on the back of a Warthog by its gun); height
                // matters less than how far away along the ground.
                let reach = |p: Vec3| {
                    let to = p - me;
                    to.truncate().length() + (to.z.abs() - 0.6).max(0.0)
                };
                let d = reach(veh.to_world(def, seat.entry)).min(reach(veh.seat_position(def, s)));
                if d > seat.entry_radius {
                    continue;
                }
                // Driving first, then the gun, then a ride.
                let rank = match seat.role {
                    SeatRole::Driver => 0.0,
                    SeatRole::Gunner => 10.0,
                    SeatRole::Passenger => 20.0,
                } + d;
                if best.is_none_or(|b| rank < b.0) {
                    let (vehicle, seat) = (v, s);
                    let action = if hijack {
                        VehicleAction::Hijack { vehicle, seat }
                    } else {
                        VehicleAction::Enter { vehicle, seat }
                    };
                    best = Some((rank, action));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// Hold the action key by a vehicle to get in (or flip it back over).
    /// Returns whether a vehicle took the key.
    pub(super) fn board(&mut self, world: &World, i: usize, action: bool, dt: f32) -> bool {
        let Some(what) = self.vehicle_action(i) else {
            self.players[i].board_held = 0.0;
            return false;
        };
        // Climbing onto a tank takes longer.
        let hold = match what {
            VehicleAction::Hijack { vehicle, .. }
                if self.vehicle_defs[self.vehicles[vehicle].def].drive == Drive::Tank =>
            {
                TANK_BOARD_HOLD
            }
            _ => SWAP_HOLD,
        };
        let p = &mut self.players[i];
        p.board_held = if action { p.board_held + dt } else { 0.0 };
        if p.board_held < hold {
            return true;
        }
        p.board_held = f32::MIN;
        match what {
            VehicleAction::Enter { vehicle, seat } => self.enter(i, vehicle, seat),
            VehicleAction::Hijack { vehicle, seat } => self.hijack(world, i, vehicle, seat),
            VehicleAction::Flip { vehicle } => self.vehicles[vehicle].right_itself(),
        }
        true
    }

    /// Board an enemy's seat: they're thrown out onto the ground (in a
    /// tank, beaten to death through the hatch) and the boarder takes it.
    fn hijack(&mut self, world: &World, i: usize, v: usize, s: usize) {
        let Some(victim) = self.vehicles[v].riders[s] else {
            return self.enter(i, v, s);
        };
        if self.vehicle_defs[self.vehicles[v].def].drive == Drive::Tank {
            self.kill(victim, Some(i), false);
        } else {
            self.exit(world, victim);
            // Thrown clear.
            let centre = self.vehicles[v].center;
            let body = &mut self.players[victim].body;
            let away = (body.position - centre).truncate().normalize_or_zero();
            body.velocity += away.extend(0.0) * 3.0 + Vec3::Z * 2.0;
        }
        self.seat_rider(i, v, s);
        self.events.push(Event::Hijacked {
            player: i,
            victim,
            vehicle: v,
            seat: s,
        });
    }

    /// Put player (or actor) `i` straight into seat `s` of vehicle `v`
    /// (out of any seat they're in), if it's free.
    pub fn enter_vehicle(&mut self, i: usize, v: usize, s: usize) {
        let free = self
            .vehicles
            .get(v)
            .is_some_and(|veh| !veh.destroyed && veh.riders.get(s) == Some(&None));
        if !free || !self.players.get(i).is_some_and(|p| p.alive) {
            return;
        }
        self.leave_seat(i);
        self.enter(i, v, s);
    }

    /// Get out of a vehicle in the air (dropped from a dropship), landing
    /// unhurt.
    pub fn unload(&mut self, world: &World, i: usize) {
        if self.players.get(i).is_some_and(|p| p.seat.is_some()) {
            self.exit(world, i);
            self.players[i].soft_landing = true;
        }
    }

    /// Move player `i` to `position` facing `yaw` at once, out of any
    /// seat (a script teleporting them).
    pub fn move_player(&mut self, i: usize, position: Vec3, yaw: f32) {
        if i >= self.players.len() {
            return;
        }
        self.leave_seat(i);
        let p = &mut self.players[i];
        p.body.position = position;
        p.body.velocity = Vec3::ZERO;
        p.yaw = yaw;
        p.pitch = 0.0;
    }

    /// Put vehicle `v` in the level at `position` facing `yaw`, new and
    /// empty (a script placing it).
    pub fn place_vehicle(&mut self, v: usize, position: Vec3, yaw: f32) {
        let Some(s) = self.vehicle_spawns.get_mut(v) else {
            return;
        };
        s.position = position;
        s.yaw = yaw;
        self.respawn_vehicle(v);
    }

    /// Take vehicle `v` out of the level until it's placed again; its
    /// riders stay where they are, and what it carries drops.
    pub fn remove_vehicle(&mut self, v: usize) {
        let Some(veh) = self.vehicles.get(v) else {
            return;
        };
        let riders: Vec<usize> = veh.riders.iter().flatten().copied().collect();
        for r in riders {
            self.leave_seat(r);
        }
        for o in &mut self.vehicles {
            if o.carrier.is_some_and(|(c, _)| c == v) {
                o.carrier = None;
                o.asleep = false;
            }
        }
        let veh = &mut self.vehicles[v];
        veh.destroyed = true;
        veh.respawn_in = f32::MAX;
        veh.flight = None;
        veh.carrier = None;
    }

    /// Load vehicle `cargo` into seat `s` of vehicle `v` (a Ghost into a
    /// Phantom's hold), or drop it (`None`).
    pub fn carry_vehicle(&mut self, cargo: usize, into: Option<(usize, usize)>) {
        if let Some(veh) = self.vehicles.get_mut(cargo) {
            veh.carrier = into;
            veh.asleep = false;
        }
    }

    fn enter(&mut self, i: usize, v: usize, s: usize) {
        self.seat_rider(i, v, s);
        self.events.push(Event::Entered {
            player: i,
            vehicle: v,
            seat: s,
        });
    }

    fn seat_rider(&mut self, i: usize, v: usize, s: usize) {
        let p = &mut self.players[i];
        p.seat = Some((v, s));
        p.board_held = f32::MIN;
        p.body.crouch = 1.0;
        if let Some(h) = p.weapons.get_mut(p.current) {
            h.state.zoom = 0;
            h.state.let_go();
        }
        let veh = &mut self.vehicles[v];
        veh.riders[s] = Some(i);
        veh.asleep = false;
        veh.abandoned = 0.0;
        if self.vehicle_defs[veh.def].seats[s].role == SeatRole::Driver {
            veh.last_driver = Some(i);
        }
        self.place_rider(i);
    }

    /// Leave the seat without moving (dying in it).
    pub(super) fn leave_seat(&mut self, i: usize) {
        let Some((v, s)) = self.players[i].seat.take() else {
            return;
        };
        let veh = &mut self.vehicles[v];
        veh.riders[s] = None;
        if self.vehicle_defs[veh.def].seats[s].role == SeatRole::Driver {
            veh.controls = Controls::default();
        }
    }

    /// Get out, beside the seat (or wherever there is room).
    pub fn exit(&mut self, world: &World, i: usize) {
        let Some((v, s)) = self.players[i].seat else {
            return;
        };
        self.leave_seat(i);
        let veh = &self.vehicles[v];
        let def = &self.vehicle_defs[veh.def];
        let seat = &def.seats[s];
        // Out the side the seat is on, else the other side, behind, on top.
        let side = if seat.position.y >= 0.0 { 1.0 } else { -1.0 };
        let half = def.radius.min(1.2);
        // A big vehicle (a dropship): clear of its hull, beside the seat,
        // behind, in front or below.
        let [lo, hi] = def.bounds;
        let beside = |s: f32| if s > 0.0 { hi.y + 0.4 } else { lo.y - 0.4 };
        let floor = lo.z + 0.1;
        let places = [
            Vec3::new(seat.position.x, side * (half + 0.3), 0.1),
            Vec3::new(seat.position.x, -side * (half + 0.3), 0.1),
            Vec3::new(-(half + 0.4), 0.0, 0.1),
            Vec3::new(seat.position.x, beside(side), floor),
            Vec3::new(seat.position.x, beside(-side), floor),
            Vec3::new(lo.x - 0.5, seat.position.y, floor),
            Vec3::new(hi.x + 0.5, seat.position.y, floor),
            Vec3::new(seat.position.x, seat.position.y, lo.z - 1.9),
            Vec3::new(seat.position.x, seat.position.y, def.radius + 0.2),
        ];
        let p = &self.players[i];
        let (r, h) = (p.body.biped.radius, p.body.biped.height_standing);
        let mut out = veh.seat_position(def, s) + Vec3::Z * def.radius;
        let mut contacts = Vec::new();
        for local in places {
            let mut at = veh.to_world(def, local);
            // Stand on the ground below, if there is some.
            if let Some(d) = world.raycast(at + Vec3::Z * 0.5, -Vec3::Z, 2.0) {
                at.z = at.z + 0.5 - d + 0.02;
            }
            world.capsule_contacts(at + Vec3::Z * r, at + Vec3::Z * (h - r), r, &mut contacts);
            if contacts.is_empty() && !self.inside_vehicle(at, r, h) {
                out = at;
                break;
            }
        }
        let velocity = veh.velocity * 0.5;
        let p = &mut self.players[i];
        p.body.position = out;
        p.body.velocity = velocity;
        p.body.grounded = false;
        p.body.crouch = 0.0;
        p.board_held = f32::MIN;
        self.events.push(Event::Exited {
            player: i,
            vehicle: v,
            seat: s,
        });
    }

    fn inside_vehicle(&self, feet: Vec3, r: f32, h: f32) -> bool {
        self.vehicles.iter().any(|veh| {
            let def = &self.vehicle_defs[veh.def];
            !veh.destroyed && touch(veh, def, feet, r, h).is_some()
        })
    }

    /// A rider's tick: drive, aim and fire the seat's gun, or get out.
    /// Returns whether they can use their own weapon (a passenger).
    pub(super) fn ride(&mut self, world: &World, i: usize, cmd: Command, dt: f32) -> bool {
        let Some((v, s)) = self.players[i].seat else {
            return true;
        };
        let p = &mut self.players[i];
        p.board_held = if cmd.action {
            if p.board_held < 0.0 && p.last.action {
                p.board_held
            } else {
                p.board_held.max(0.0) + dt
            }
        } else {
            0.0
        };
        if p.board_held >= SWAP_HOLD {
            self.exit(world, i);
            return false;
        }
        let def = &self.vehicle_defs[self.vehicles[v].def];
        let seat = def.seats[s].clone();
        let drive = def.drive;
        let (cp, sp) = (cmd.pitch.cos(), cmd.pitch.sin());
        let look = Vec3::new(cmd.yaw.cos() * cp, cmd.yaw.sin() * cp, sp);
        let armed = seat.weapon.is_some() || seat.alt_weapon.is_some();
        let target = if armed {
            self.crosshair(world, i, look)
        } else {
            Vec3::ZERO
        };
        let veh = &mut self.vehicles[v];
        // Aim the turret at what its gunner (or the tank's driver) looks at.
        if let Some(gun) = seat.turret {
            let pivot = veh.to_world(def, gun.pivot);
            let want = (target - pivot).normalize_or(look);
            let local = veh.rotation.inverse() * want;
            let [lo, hi] = seat.pitch_range;
            veh.aim = Vec2::new(
                local.y.atan2(local.x),
                local
                    .z
                    .clamp(-1.0, 1.0)
                    .asin()
                    .clamp(lo.min(hi), hi.max(lo)),
            );
        }
        match seat.role {
            SeatRole::Driver => {
                veh.controls = Controls {
                    throttle: cmd.movement,
                    yaw: cmd.yaw,
                    pitch: cmd.pitch,
                    boost: cmd.throw_grenade && matches!(drive, Drive::Hover | Drive::Fly),
                    horn: def.horn && seat.weapon.is_none() && cmd.fire,
                    driven: true,
                };
                veh.last_driver = Some(i);
                veh.asleep = false;
                // Jump: a Banshee loops, or rolls aside steering sideways.
                if cmd.jump && !self.players[i].last.jump {
                    let side = cmd.movement.x;
                    let kind = if side.abs() > 0.5 {
                        TrickKind::Roll(side.signum())
                    } else {
                        TrickKind::Flip
                    };
                    if veh.start_trick(def, kind) {
                        // Homing rounds lose it.
                        for r in &mut self.projectiles {
                            if r.target == Some(Homing::Vehicle(v)) {
                                r.target = None;
                            }
                        }
                    }
                }
            }
            SeatRole::Gunner => {}
            SeatRole::Passenger => return true,
        }
        let boosting = self.vehicles[v].controls.boost;
        if let Some(w) = seat.weapon {
            let pulled = cmd.fire && !boosting;
            self.fire_seat(world, i, (v, s), (w, false), target, pulled, dt);
        }
        if let Some(w) = seat.alt_weapon {
            // The second trigger: the left trigger, or melee.
            let pulled = match self.weapons.get(w).map(|d| d.input) {
                Some(TriggerInput::Melee) => cmd.melee,
                _ => cmd.throw_grenade && !boosting,
            };
            self.fire_seat(world, i, (v, s), (w, true), target, pulled, dt);
        }
        false
    }

    /// Fire seat `s` of vehicle `v`'s gun `w` (its second trigger's, if
    /// `alt`) at `target` while `pulled`.
    #[allow(clippy::too_many_arguments)]
    fn fire_seat(
        &mut self,
        world: &World,
        i: usize,
        (v, s): (usize, usize),
        (w, alt): (usize, bool),
        target: Vec3,
        pulled: bool,
        dt: f32,
    ) {
        let Some(def) = self.weapons.get(w).cloned() else {
            return;
        };
        let veh = &self.vehicles[v];
        let vdef = &self.vehicle_defs[veh.def];
        // Rounds that fly leave a turret's muzzle the way it points; bullets
        // (and built-in guns' rounds) go from the rider's eye straight to
        // what the crosshair is on.
        let eye = veh.seat_eye(vdef, s);
        let at_target = (target - eye).normalize_or(self.players[i].aim());
        let (dir, origin) = match veh.turret_muzzle(vdef, s) {
            Some((dir, muzzle)) if def.flight.is_some() => (dir, muzzle),
            _ => (at_target, eye),
        };
        let states = if alt {
            &mut self.vehicles[v].alt_weapons
        } else {
            &mut self.vehicles[v].weapons
        };
        let Some(state) = states.get_mut(s) else {
            return;
        };
        let state = state.get_or_insert_with(|| WeaponState::new(&def));
        let input = WeaponInput {
            fire: pulled,
            reload: false,
            zoom: false,
        };
        let shots = state.update(&def, input, dt);
        // Vehicle guns never run dry.
        if def.uses_ammo() {
            state.reserve = def.maximum_rounds.max(state.reserve);
        }
        if shots.is_empty() {
            return;
        }
        // Bullets go where autoaim steers them, as from a gun in hand.
        let assisted = def.flight.is_none() && self.players[i].actor.is_none();
        let dir = match assisted.then(|| self.autoaim(world, i, origin, dir, &def, 0)) {
            Some(Some((_, Some(point)))) => (point - origin).normalize_or(dir),
            _ => dir,
        };
        let right = dir.cross(Vec3::Z).normalize_or(Vec3::X);
        let up = right.cross(dir);
        for shot in shots {
            self.fire(
                world,
                i,
                origin,
                shot.direction(dir, right, up),
                &def,
                w,
                false,
            );
        }
    }

    /// Who a ray hits first among vehicles (not the one `shooter` rides):
    /// (vehicle, distance).
    pub(super) fn trace_vehicles(
        &self,
        shooter: usize,
        origin: Vec3,
        dir: Vec3,
    ) -> Option<(usize, f32)> {
        let own = self.riding(shooter).map(|(v, _)| v);
        let mut best: Option<(usize, f32)> = None;
        for (v, veh) in self.vehicles.iter().enumerate() {
            if veh.destroyed || Some(v) == own {
                continue;
            }
            let def = &self.vehicle_defs[veh.def];
            // Bounding sphere first.
            let oc = origin - veh.center;
            let b = oc.dot(dir);
            if b > 0.0 && oc.length_squared() > def.radius * def.radius {
                continue;
            }
            if oc.length_squared() - b * b > def.radius * def.radius {
                continue;
            }
            let o = veh.to_local(def, origin);
            let d = veh.rotation.inverse() * dir;
            for hull in &def.hull {
                if let Some(t) = hull.raycast(o, d) {
                    if best.is_none_or(|bt| t < bt.1) {
                        best = Some((v, t));
                    }
                }
            }
        }
        best
    }

    /// Damage a vehicle; at no health left it blows up with its riders.
    pub fn damage_vehicle(&mut self, v: usize, attacker: Option<usize>, amount: f32) {
        let veh = &mut self.vehicles[v];
        if veh.destroyed || amount <= 0.0 {
            return;
        }
        let amount = if amount >= HEAVY_DAMAGE {
            amount
        } else {
            amount * BULLET_DAMAGE_SCALE
        };
        veh.health -= amount;
        veh.asleep = false;
        if veh.health <= 0.0 {
            self.destroy_vehicle(v, attacker);
        }
    }

    fn destroy_vehicle(&mut self, v: usize, attacker: Option<usize>) {
        let veh = &mut self.vehicles[v];
        veh.destroyed = true;
        veh.health = 0.0;
        veh.respawn_in = self
            .vehicle_spawns
            .get(v)
            .map_or(30.0, |s| s.respawn.max(5.0));
        let position = veh.center;
        let riders: Vec<usize> = veh.riders.iter().flatten().copied().collect();
        for r in riders {
            self.leave_seat(r);
            self.kill(r, attacker.or(Some(r)), false);
        }
        self.events.push(Event::VehicleDestroyed {
            vehicle: v,
            position,
        });
        // The blast hurts whoever stands close.
        for j in 0..self.players.len() {
            let p = &self.players[j];
            if !p.alive {
                continue;
            }
            let d = p.body.position.distance(position);
            if d < WRECK_RADIUS {
                self.damage(j, attacker, WRECK_DAMAGE * (1.0 - d / WRECK_RADIUS), false);
            }
        }
    }

    /// Hurt vehicles caught in a blast.
    pub(super) fn blast_vehicles(
        &mut self,
        at: Vec3,
        attacker: usize,
        damage: f32,
        radius: (f32, f32),
    ) {
        for v in 0..self.vehicles.len() {
            let veh = &self.vehicles[v];
            if veh.destroyed {
                continue;
            }
            let def = &self.vehicle_defs[veh.def];
            let local = veh.to_local(def, at);
            let d = def
                .hull
                .iter()
                .map(|h| h.closest_point(local).distance(local))
                .fold(f32::MAX, f32::min);
            if d >= radius.1 {
                continue;
            }
            let falloff = if d <= radius.0 {
                1.0
            } else {
                1.0 - (d - radius.0) / (radius.1 - radius.0)
            };
            // Blasts count in full.
            self.damage_vehicle(v, Some(attacker), (damage * falloff).max(HEAVY_DAMAGE));
            let veh = &mut self.vehicles[v];
            let def = &self.vehicle_defs[veh.def];
            let push = (veh.center - at).normalize_or(Vec3::Z) + Vec3::Z;
            let impulse = push * def.mass * 1.5 * falloff;
            veh.apply_impulse(def, impulse, at);
        }
    }

    /// Advance the vehicles: physics, running people over, coming back.
    pub(super) fn step_vehicles(&mut self, world: &World, dt: f32) {
        for v in 0..self.vehicles.len() {
            if self.vehicles[v].destroyed {
                self.vehicles[v].respawn_in -= dt;
                if self.vehicles[v].respawn_in <= 0.0 && self.spawn_clear(v) {
                    self.respawn_vehicle(v);
                }
                continue;
            }
            // Carried: held in the carrier's seat.
            if let Some((c, s)) = self.vehicles[v].carrier {
                let held = self
                    .vehicles
                    .get(c)
                    .filter(|o| !o.destroyed && c != v)
                    .map(|o| {
                        let d = &self.vehicle_defs[o.def];
                        (o.seat_position(d, s), o.rotation, o.velocity)
                    });
                let veh = &mut self.vehicles[v];
                match held {
                    Some((at, rotation, velocity)) => {
                        let def = &self.vehicle_defs[veh.def];
                        veh.rotation = rotation;
                        veh.center = at + rotation * def.center;
                        veh.velocity = velocity;
                        veh.spin = Vec3::ZERO;
                        veh.asleep = false;
                        continue;
                    }
                    None => veh.carrier = None,
                }
            }
            let veh = &mut self.vehicles[v];
            let def = &self.vehicle_defs[veh.def];
            if let Some(d) = def.driver_seat() {
                if veh.riders[d].is_none() {
                    veh.controls.driven = false;
                    veh.flight = None;
                }
            }
            veh.step(def, world, dt);

            // Sunk in the sea, fallen out of the level.
            let below = veh.center.z < world.min.z - 2.0;
            if below
                || self
                    .kill_zones
                    .iter()
                    .any(|z| z.kills(self.vehicles[v].center))
            {
                let blame = self.vehicles[v].last_driver;
                let riders: Vec<usize> =
                    self.vehicles[v].riders.iter().flatten().copied().collect();
                for r in riders {
                    self.leave_seat(r);
                    self.kill(r, None, false);
                }
                let _ = blame;
                self.vehicles[v].destroyed = true;
                self.vehicles[v].respawn_in =
                    self.vehicle_spawns.get(v).map_or(30.0, |s| s.respawn);
                continue;
            }

            // Riders bail out of a vehicle on its roof.
            let veh = &mut self.vehicles[v];
            veh.overturned = if veh.upside_down() && veh.trick.is_none() {
                veh.overturned + dt
            } else {
                0.0
            };
            if veh.overturned > BAIL_OUT_AFTER {
                let riders: Vec<usize> = veh.riders.iter().flatten().copied().collect();
                for r in riders {
                    self.exit(world, r);
                }
            }

            // Left empty somewhere (or wrecked on its roof): back to its spawn.
            let veh = &self.vehicles[v];
            let empty = veh.riders.iter().all(Option::is_none);
            let away = self.vehicle_spawns.get(v).is_some_and(|s| {
                veh.origin(&self.vehicle_defs[veh.def]).distance(s.position) > 3.0
            });
            let wait = self.vehicle_spawns.get(v).map_or(f32::MAX, |s| s.respawn);
            let veh = &mut self.vehicles[v];
            veh.abandoned = if empty && (away || veh.upside_down()) {
                veh.abandoned + dt
            } else {
                0.0
            };
            if veh.abandoned > wait && self.spawn_clear(v) {
                self.respawn_vehicle(v);
            }
        }
        self.collide_vehicles();
        self.run_over(dt);
        self.place_riders();
    }

    fn spawn_clear(&self, v: usize) -> bool {
        let Some(s) = self.vehicle_spawns.get(v) else {
            return false;
        };
        let reach = self.vehicle_defs[s.def].radius + 0.5;
        let near = |p: Vec3| p.distance(s.position) < reach;
        !self
            .players
            .iter()
            .any(|p| p.alive && near(p.body.position))
            && !self
                .vehicles
                .iter()
                .enumerate()
                .any(|(k, o)| k != v && !o.destroyed && near(o.center))
    }

    fn respawn_vehicle(&mut self, v: usize) {
        let Some(s) = self.vehicle_spawns.get(v).copied() else {
            return;
        };
        let riders: Vec<usize> = self.vehicles[v].riders.iter().flatten().copied().collect();
        for r in riders {
            self.leave_seat(r);
        }
        self.vehicles[v] = Vehicle::new(s.def, &self.vehicle_defs[s.def], s.position, s.yaw);
        self.arm_vehicle(v);
        self.events.push(Event::VehicleSpawned { vehicle: v });
    }

    /// Vehicles bump into each other.
    fn collide_vehicles(&mut self) {
        let n = self.vehicles.len();
        for a in 0..n {
            for b in a + 1..n {
                let (va, vb) = (&self.vehicles[a], &self.vehicles[b]);
                // A vehicle in another's hold stays put in it.
                let carried = va.carrier.is_some() || vb.carrier.is_some();
                if va.destroyed || vb.destroyed || carried {
                    continue;
                }
                let (da, db) = (&self.vehicle_defs[va.def], &self.vehicle_defs[vb.def]);
                if va.center.distance(vb.center) > da.radius + db.radius {
                    continue;
                }
                // Deepest touching pair of hull spheres.
                let mut deepest: Option<(f32, Vec3, Vec3)> = None;
                for &(la, ra) in &da.spheres {
                    let pa = va.to_world(da, la);
                    for &(lb, rb) in &db.spheres {
                        let pb = vb.to_world(db, lb);
                        let d = pa.distance(pb);
                        let depth = ra + rb - d;
                        if depth > 0.0 && deepest.is_none_or(|x| depth > x.0) {
                            let n = (pa - pb).normalize_or(Vec3::Z);
                            deepest = Some((depth, n, pb + n * rb));
                        }
                    }
                }
                let Some((depth, n, at)) = deepest else {
                    continue;
                };
                let fixed = |d: &VehicleDef| d.drive == Drive::Fixed;
                let (ma, mb) = (
                    if fixed(da) { f32::INFINITY } else { da.mass },
                    if fixed(db) { f32::INFINITY } else { db.mass },
                );
                let wa = if ma.is_infinite() {
                    0.0
                } else if mb.is_infinite() {
                    1.0
                } else {
                    mb / (ma + mb)
                };
                let approach = (va.point_velocity(at) - vb.point_velocity(at)).dot(n);
                let (da, db) = (da.clone(), db.clone());
                self.vehicles[a].center += n * depth * wa;
                self.vehicles[b].center -= n * depth * (1.0 - wa);
                if approach < 0.0 {
                    let m = if ma.is_infinite() {
                        mb
                    } else if mb.is_infinite() {
                        ma
                    } else {
                        ma * mb / (ma + mb)
                    };
                    let j = -1.2 * approach * m;
                    if !fixed(&da) {
                        self.vehicles[a].apply_impulse(&da, n * j, at);
                    }
                    if !fixed(&db) {
                        self.vehicles[b].apply_impulse(&db, -n * j, at);
                    }
                }
            }
        }
    }

    /// Players in a vehicle's way: run over at speed, pushed aside
    /// otherwise (or standing on top).
    fn run_over(&mut self, _dt: f32) {
        for j in 0..self.players.len() {
            let p = &self.players[j];
            if !p.alive || p.seat.is_some() {
                continue;
            }
            for v in 0..self.vehicles.len() {
                let Some((n, depth, at)) = self.touching(j, v) else {
                    continue;
                };
                // How fast the hull comes at them, along the way out of it.
                let veh = &self.vehicles[v];
                let def = &self.vehicle_defs[veh.def];
                let closing = (veh.point_velocity(at) - self.players[j].body.velocity).dot(n);
                let driver = def
                    .driver_seat()
                    .and_then(|d| veh.riders[d])
                    .or(veh.last_driver);
                if closing > SPLATTER_SPEED && veh.speed() > SPLATTER_SPEED * 0.8 {
                    self.events.push(Event::Splattered {
                        player: j,
                        vehicle: v,
                    });
                    let killer = driver.filter(|&d| d != j);
                    self.damage(j, killer, f32::INFINITY, false);
                    if self.players[j].alive {
                        // Teammates without friendly fire just get shoved
                        // (off their feet, if it's upward).
                        let body = &mut self.players[j].body;
                        body.velocity += n * closing;
                        if body.velocity.z > 0.0 {
                            body.grounded = false;
                        }
                    }
                    break;
                }
                self.push_off(j, v, n, depth);
            }
        }
    }

    /// Push player `j`, on foot, out of any vehicle in their way, or up
    /// onto one they stand on, as each tick does (but never run over).
    pub(super) fn keep_clear(&mut self, j: usize) {
        if self.players[j].seat.is_some() {
            return;
        }
        for v in 0..self.vehicles.len() {
            if let Some((n, depth, _)) = self.touching(j, v) {
                self.push_off(j, v, n, depth);
            }
        }
    }

    /// How player `j` overlaps vehicle `v`'s hull, if they do (see `touch`).
    fn touching(&self, j: usize, v: usize) -> Option<(Vec3, f32, Vec3)> {
        let veh = &self.vehicles[v];
        if veh.destroyed {
            return None;
        }
        let body = &self.players[j].body;
        let def = &self.vehicle_defs[veh.def];
        touch(veh, def, body.position, body.biped.radius, body.height())
    }

    /// Push player `j` `depth` out of vehicle `v` along `n`: up onto it if
    /// they stand on it, aside otherwise.
    fn push_off(&mut self, j: usize, v: usize, n: Vec3, depth: f32) {
        let body = &mut self.players[j].body;
        if n.z > 0.7 {
            // Standing on it.
            body.position.z += depth;
            body.velocity.z = body.velocity.z.max(self.vehicles[v].velocity.z);
            body.grounded = true;
        } else {
            let flat = Vec3::new(n.x, n.y, 0.0).normalize_or(Vec3::X);
            body.position += flat * depth;
            let into = body.velocity.dot(flat);
            if into < 0.0 {
                body.velocity -= flat * into;
            }
        }
    }

    /// Riders sit where their seats are.
    pub(super) fn place_riders(&mut self) {
        for i in 0..self.players.len() {
            self.place_rider(i);
        }
    }

    fn place_rider(&mut self, i: usize) {
        let Some((v, s)) = self.players[i].seat else {
            return;
        };
        let veh = &self.vehicles[v];
        let def = &self.vehicle_defs[veh.def];
        let at = veh.seat_position(def, s);
        let velocity = veh.velocity;
        let body = &mut self.players[i].body;
        body.position = at;
        body.velocity = velocity;
        body.grounded = true;
        body.crouch = 1.0;
    }

    /// Whether a ray may hit player `j` where they sit (riders shut inside
    /// can't be shot).
    pub(super) fn exposed(&self, j: usize) -> bool {
        match self.players[j].seat {
            Some((v, s)) => {
                let def = &self.vehicle_defs[self.vehicles[v].def];
                def.seats[s].exposed
            }
            None => true,
        }
    }

    /// The weapon definition fired from a seat, if any.
    pub fn seat_weapon(&self, v: usize, s: usize) -> Option<&WeaponDef> {
        let def = &self.vehicle_defs[self.vehicles.get(v)?.def];
        self.weapons.get(def.seats.get(s)?.weapon?)
    }
}

/// How a standing player (feet at `feet`) overlaps a vehicle's hull:
/// (normal out of the hull, depth, contact point).
fn touch(veh: &Vehicle, def: &VehicleDef, feet: Vec3, r: f32, h: f32) -> Option<(Vec3, f32, Vec3)> {
    if (feet + Vec3::Z * h * 0.5).distance(veh.center) > def.radius + h {
        return None;
    }
    let mut best: Option<(Vec3, f32, Vec3)> = None;
    for k in 0..3 {
        let q = feet + Vec3::Z * (r + (h - 2.0 * r).max(0.0) * k as f32 * 0.5);
        let local = veh.to_local(def, q);
        for hull in &def.hull {
            let c = hull.closest_point(local);
            let d = local.distance(c);
            let (n, depth) = if d > 1e-4 {
                ((local - c) / d, r - d)
            } else {
                // Inside: out the nearest face.
                let o = hull.axes.transpose() * (local - hull.center);
                let gap = hull.half_extents - o.abs();
                let axis = if gap.x < gap.y && gap.x < gap.z {
                    0
                } else if gap.y < gap.z {
                    1
                } else {
                    2
                };
                let sign = if o[axis] >= 0.0 { 1.0 } else { -1.0 };
                (hull.axes.col(axis) * sign, gap[axis] + r)
            };
            if depth > 0.0 && best.is_none_or(|b| depth > b.1) {
                best = Some((veh.rotation * n, depth, veh.to_world(def, c)));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::tests::game;
    use crate::testing::floor;
    use crate::vehicle::tests::jeep;
    use crate::vehicle::{Flight, SeatDef};

    fn with_jeep() -> Game {
        let mut g = game();
        g.set_vehicles(
            vec![jeep()],
            vec![VehicleSpawn {
                def: 0,
                position: Vec3::new(3.0, 3.0, 0.05),
                yaw: 0.0,
                respawn: 30.0,
            }],
        );
        g.add_player();
        g
    }

    fn hold(g: &mut Game, cmd: Command, ticks: usize) {
        let world = floor();
        for _ in 0..ticks {
            g.step(&world, &[cmd]);
        }
    }

    fn walk_to_driver_door(g: &mut Game) {
        let veh = &g.vehicles[0];
        let def = &g.vehicle_defs[0];
        let door = veh.to_world(def, def.seats[0].entry + Vec3::Y * 0.6);
        g.players[0].body.position = Vec3::new(door.x, door.y, 0.0);
    }

    #[test]
    fn hold_action_to_drive_and_again_to_get_out() {
        let mut g = with_jeep();
        hold(&mut g, Command::default(), 120);
        walk_to_driver_door(&mut g);
        hold(&mut g, Command::default(), 1);
        assert_eq!(
            g.vehicle_action(0),
            Some(VehicleAction::Enter {
                vehicle: 0,
                seat: 0
            })
        );
        let act = Command {
            action: true,
            ..Command::default()
        };
        hold(&mut g, act, 30);
        assert_eq!(g.riding(0), Some((0, 0)));
        // Still holding the key from getting in: stays in.
        hold(&mut g, act, 30);
        assert_eq!(g.riding(0), Some((0, 0)));
        // Drive off, toward +x.
        let drive = Command {
            movement: Vec2::new(0.0, 1.0),
            ..Command::default()
        };
        hold(&mut g, drive, 120);
        let veh = &g.vehicles[0];
        assert!(veh.velocity.x > 2.0, "driving at {}", veh.velocity);
        let seat = veh.seat_position(&g.vehicle_defs[0], 0);
        assert!(g.players[0].body.position.distance(seat) < 1e-3);
        // Let go, then hold to get out.
        hold(&mut g, Command::default(), 120);
        hold(&mut g, act, 30);
        assert_eq!(g.riding(0), None);
        assert!(g.vehicles[0].riders[0].is_none());
        let p = &g.players[0];
        assert!(p.body.position.distance(g.vehicles[0].center) > 0.5);
    }

    #[test]
    fn a_joined_game_sees_the_vehicles_where_the_host_has_them() {
        use crate::game::{Reader, Writer};
        let mut host = with_jeep();
        hold(&mut host, Command::default(), 120);
        walk_to_driver_door(&mut host);
        let act = Command {
            action: true,
            ..Command::default()
        };
        hold(&mut host, act, 60);
        let drive = Command {
            movement: Vec2::new(0.0, 1.0),
            yaw: 0.4,
            ..Command::default()
        };
        hold(&mut host, drive, 90);
        let mut w = Writer::default();
        host.write_state(&mut w);

        let mut joined = with_jeep();
        joined.read_state(&mut Reader::new(&w.0)).unwrap();
        let (h, j) = (&host.vehicles[0], &joined.vehicles[0]);
        assert_eq!(h.center, j.center);
        assert_eq!(h.rotation, j.rotation);
        assert_eq!(h.velocity, j.velocity);
        assert_eq!(h.riders, j.riders);
        assert_eq!(h.steer, j.steer);
        assert_eq!(joined.riding(0), Some((0, 0)));
        assert!(h.speed() > 1.0);
    }

    #[test]
    fn a_speeding_jeep_splatters_whoever_is_in_the_way() {
        let mut g = with_jeep();
        g.add_player();
        hold(&mut g, Command::default(), 60);
        walk_to_driver_door(&mut g);
        let act = Command {
            action: true,
            ..Command::default()
        };
        hold(&mut g, act, 30);
        assert_eq!(g.riding(0), Some((0, 0)));
        // A victim ahead.
        let ahead = g.vehicles[0].center + Vec3::X * 8.0;
        g.players[1].body.position = Vec3::new(ahead.x, ahead.y, 0.0);
        let world = floor();
        let drive = Command {
            movement: Vec2::new(0.0, 1.0),
            ..Command::default()
        };
        let mut splat = false;
        for _ in 0..300 {
            g.step(&world, &[drive, Command::default()]);
            splat |= g
                .events
                .iter()
                .any(|e| matches!(e, Event::Splattered { player: 1, .. }));
        }
        assert!(splat);
        assert_eq!(g.players[0].kills, 1);
    }

    #[test]
    fn a_slow_jeep_pushes_people_aside() {
        let mut g = with_jeep();
        hold(&mut g, Command::default(), 60);
        let c = g.vehicles[0].center;
        g.players[0].body.position = Vec3::new(c.x + 0.3, c.y + 0.2, 0.0);
        hold(&mut g, Command::default(), 30);
        let p = g.players[0].body.position;
        assert!(g.players[0].alive);
        assert!(
            !g.inside_vehicle(p, 0.15, 0.7),
            "pushed out to {p} (vehicle at {c})"
        );
    }

    #[test]
    fn shots_and_grenades_wreck_vehicles_and_their_riders() {
        let mut g = with_jeep();
        g.add_player();
        hold(&mut g, Command::default(), 60);
        walk_to_driver_door(&mut g);
        let act = Command {
            action: true,
            ..Command::default()
        };
        hold(&mut g, act, 30);
        assert_eq!(g.riding(0), Some((0, 0)));
        let health = g.vehicles[0].health;
        g.damage_vehicle(0, Some(1), 20.0);
        assert!(
            (g.vehicles[0].health - (health - 5.0)).abs() < 1e-3,
            "bullets scratch"
        );
        let at = g.vehicles[0].center;
        g.blast_vehicles(at, 1, 150.0, (0.75, 1.75));
        g.blast_vehicles(at, 1, 150.0, (0.75, 1.75));
        assert!(g.vehicles[0].destroyed);
        assert!(!g.players[0].alive, "the driver goes with it");
        assert_eq!(g.players[1].kills, 1);
        // It comes back after its respawn time.
        hold(&mut g, Command::default(), 31 * 60);
        assert!(!g.vehicles[0].destroyed);
    }

    /// The jeep as a tank whose driver works a turret: a cannon on the
    /// trigger and a machine gun on the left trigger.
    fn with_tank() -> Game {
        let mut g = game();
        let mg = WeaponDef {
            name: "machine_gun".into(),
            input: TriggerInput::Left,
            ..g.weapons[0].clone()
        };
        g.weapons.push(mg);
        let mut tank = jeep();
        tank.drive = Drive::Tank;
        tank.seats[0].weapon = Some(0);
        tank.seats[0].alt_weapon = Some(g.weapons.len() - 1);
        tank.seats[0].turret = Some(crate::vehicle::TurretGun {
            pivot: Vec3::new(0.0, 0.0, 0.6),
            muzzle: Vec3::new(1.0, 0.0, 0.0),
            elevation: 0.0,
        });
        g.set_vehicles(
            vec![tank],
            vec![VehicleSpawn {
                def: 0,
                position: Vec3::new(3.0, 3.0, 0.05),
                yaw: 0.0,
                respawn: 30.0,
            }],
        );
        g.add_player();
        g
    }

    /// Player 1 driving the parked vehicle, player 0 at its door.
    fn carjack(mut g: Game) -> Game {
        g.add_player();
        hold(&mut g, Command::default(), 120);
        g.enter(1, 0, 0);
        walk_to_driver_door(&mut g);
        g
    }

    fn act_alone(g: &mut Game, ticks: usize) {
        let idle = Command::default();
        let act = Command {
            action: true,
            ..idle
        };
        for _ in 0..ticks {
            g.step(&floor(), &[act, idle]);
        }
    }

    #[test]
    fn hold_action_by_an_enemy_driver_to_throw_them_out() {
        let mut g = carjack(with_jeep());
        assert_eq!(
            g.vehicle_action(0),
            Some(VehicleAction::Hijack {
                vehicle: 0,
                seat: 0
            })
        );
        act_alone(&mut g, 30);
        assert_eq!(g.riding(0), Some((0, 0)));
        assert_eq!(g.riding(1), None);
        assert!(g.players[1].alive, "thrown out, not killed");
        assert!(g
            .events
            .iter()
            .any(|e| matches!(e, Event::Hijacked { victim: 1, .. })));
    }

    #[test]
    fn boarding_a_tank_takes_longer_and_kills_the_driver() {
        let mut g = carjack(with_tank());
        act_alone(&mut g, 30);
        assert_eq!(g.riding(1), Some((0, 0)), "still climbing on");
        act_alone(&mut g, 40);
        assert_eq!(g.riding(0), Some((0, 0)));
        assert!(!g.players[1].alive);
    }

    #[test]
    fn a_tank_driver_aims_the_turret_and_fires_both_guns() {
        let mut g = with_tank();
        hold(&mut g, Command::default(), 120);
        walk_to_driver_door(&mut g);
        let act = Command {
            action: true,
            ..Command::default()
        };
        hold(&mut g, act, 30);
        assert_eq!(g.riding(0), Some((0, 0)));
        hold(&mut g, Command::default(), 30);
        // Look left: the turret turns, the hull stays put.
        let look = Command {
            yaw: 1.0,
            ..Command::default()
        };
        hold(&mut g, look, 30);
        assert!(
            (g.vehicles[0].aim.x - 1.0).abs() < 0.05,
            "{:?}",
            g.vehicles[0].aim
        );
        assert!(g.vehicles[0].yaw().abs() < 0.05, "hull turned");
        let fired = |g: &Game, w: usize| {
            g.events
                .iter()
                .any(|e| matches!(e, Event::Shot { weapon, .. } if *weapon == w))
        };
        hold(&mut g, Command { fire: true, ..look }, 1);
        assert!(fired(&g, 0), "the cannon fires on the trigger");
        let mg = g.weapons.len() - 1;
        hold(&mut g, look, 30);
        hold(
            &mut g,
            Command {
                throw_grenade: true,
                ..look
            },
            1,
        );
        assert!(fired(&g, mg), "the machine gun fires on the left trigger");
        // Driving off, the hull swings round to where the driver looks.
        hold(
            &mut g,
            Command {
                movement: Vec2::new(0.0, 1.0),
                ..look
            },
            120,
        );
        assert!(
            (g.vehicles[0].yaw() - 1.0).abs() < 0.2,
            "{}",
            g.vehicles[0].yaw()
        );
    }

    /// A jeep that flies, with a hold and a troop seat: a dropship.
    fn dropship() -> VehicleDef {
        let mut def = jeep();
        def.drive = Drive::Fly;
        let driver = def.seats[0].clone();
        let seat = |role, animation: &str, y: f32| SeatDef {
            role,
            position: Vec3::new(-0.5, y, 0.39),
            animation: animation.into(),
            ai_only: true,
            ..driver.clone()
        };
        def.seats.push(seat(SeatRole::Passenger, "dropship_p", 0.3));
        def.seats
            .push(seat(SeatRole::Passenger, "dropship_sc", -0.3));
        def
    }

    #[test]
    fn a_dropship_flies_its_course_and_drops_its_cargo_and_troops_unhurt() {
        let mut g = game();
        g.set_vehicles(
            vec![dropship(), jeep()],
            [
                (0, Vec3::new(0.0, 0.0, 10.0)),
                (1, Vec3::new(20.0, 0.0, 0.05)),
            ]
            .map(|(def, position)| VehicleSpawn {
                def,
                position,
                yaw: 0.0,
                respawn: 1e9,
            })
            .to_vec(),
        );
        // Not in the level until placed.
        g.remove_vehicle(1);
        assert!(g.vehicles[1].destroyed);
        g.place_vehicle(1, Vec3::new(0.0, 0.0, 12.0), 0.0);
        assert!(!g.vehicles[1].destroyed);
        let pilot = g.add_player();
        let rider = g.add_player();
        g.enter_vehicle(pilot, 0, 0);
        g.enter_vehicle(rider, 0, 1);
        g.carry_vehicle(1, Some((0, 2)));
        assert_eq!(g.riding(rider), Some((0, 1)));
        // The players' seats hold only actors.
        g.players[0].body.position = g.vehicles[0].to_world(&g.vehicle_defs[0], Vec3::ZERO);
        assert!(g.vehicle_action(0).is_none_or(
            |a| !matches!(a, VehicleAction::Enter { seat, .. } | VehicleAction::Hijack { seat, .. } if seat > 0)
        ));

        let to = Vec3::new(15.0, 5.0, 8.0);
        g.vehicles[0].flight = Some(Flight {
            to,
            speed: 6.0,
            face: Some(1.0),
            stop: true,
        });
        hold(&mut g, Command::default(), 600);
        let def = &g.vehicle_defs[0];
        let ship = &g.vehicles[0];
        assert!(ship.origin(def).distance(to) < 0.3, "{}", ship.origin(def));
        assert!(
            (ship.yaw() - 1.0).abs() < 0.05,
            "turned to face {}",
            ship.yaw()
        );
        // The cargo came along in the hold.
        let hold_at = ship.seat_position(def, 2);
        let cargo = &g.vehicles[1];
        assert!(cargo.origin(&g.vehicle_defs[1]).distance(hold_at) < 0.05);

        // Dropped: the jeep falls to the ground and the trooper lands
        // unhurt from 8 metres.
        g.carry_vehicle(1, None);
        let world = floor();
        g.unload(&world, rider);
        assert!(g.riding(rider).is_none());
        hold(&mut g, Command::default(), 300);
        assert!(g.vehicles[1].center.z < 1.0, "{}", g.vehicles[1].center);
        let r = &g.players[rider];
        assert!(r.alive && r.health >= r.full.health, "{}", r.health);
        assert!(r.body.position.z < 0.5);
    }
}
