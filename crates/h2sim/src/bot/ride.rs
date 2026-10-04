//! Bots and vehicles: getting into one nearby, driving it where the bot is
//! going (at enemies, given the chance), manning its gun, and getting out
//! when it's stuck, when there's nothing to shoot, or near the objective.

use super::{wrap, Bot, FIRE_CONE, REACTION};
use crate::collision::World;
use crate::game::{Command, Game, Homing, VehicleAction, TICK};
use crate::nav::NavGraph;
use crate::vehicle::{Drive, SeatRole};
use blam_cache::weapon::{TriggerBehavior, TriggerInput};
use glam::{Vec2, Vec3};

/// How far a bot goes out of its way for a vehicle.
const BOARD_RANGE: f32 = 8.0;
/// Close enough to a route point, driving.
const DRIVE_ARRIVED: f32 = 1.8;
/// Seconds barely moving before a driver backs up, and how long it does.
const STALLED: f32 = 1.2;
const BACK_UP: f32 = 1.0;
/// Times stuck before giving up on the vehicle.
const GIVE_UP_STALLS: u32 = 3;
/// Seconds of driving freely that forgive being stuck before.
const UNSTUCK: f32 = 6.0;
/// Seconds on a gun with no one to shoot (or no driver) before getting off.
const BORED: f32 = 10.0;
const NO_DRIVER: f32 = 4.0;
/// Get out and walk this close to an objective.
const OBJECTIVE_ON_FOOT: f32 = 4.0;
/// Drive at enemies this close to run them over.
const RAM_RANGE: f32 = 15.0;
/// Fliers keep this high over where they're heading.
const FLY_HEIGHT: f32 = 1.5;

/// A bot's dealings with vehicles.
#[derive(Default)]
pub(super) struct Riding {
    /// Takes vehicles this life.
    pub rides: bool,
    /// Holding the action key to get out.
    leaving: bool,
    /// Vehicles given up on (until the bot respawns).
    shunned: Vec<usize>,
    /// The vehicle it was last in, to shun once out.
    last: Option<usize>,
    /// Driving: seconds barely moving, backing up, times stuck, and
    /// seconds since last stuck.
    stalled: f32,
    backing: f32,
    stalls: u32,
    free: f32,
    /// On a gun: seconds with nothing to shoot, and without a driver.
    idle: f32,
    driverless: f32,
}

impl Riding {
    pub fn describe(&self) -> String {
        format!(
            "rides {} in {:?} stalls {} leaving {} shuns {:?}",
            self.rides, self.last, self.stalls, self.leaving, self.shunned
        )
    }

    /// Back to the start (a new life).
    pub fn reset(&mut self, rides: bool) {
        *self = Riding {
            rides,
            ..Riding::default()
        };
    }
}

impl Bot {
    /// Where player `me` stands to board `target`'s seat, if they ride an
    /// enemy vehicle close by and slow enough to jump on.
    pub(super) fn board_point(game: &Game, me: usize, target: usize) -> Option<Vec3> {
        let (v, s) = game.riding(target)?;
        let veh = &game.vehicles[v];
        let def = &game.vehicle_defs[veh.def];
        let seat = &def.seats[s];
        let feet = game.players[me].body.position;
        let near = veh.center.distance(feet) < def.radius + 5.0;
        (near && seat.role != SeatRole::Passenger && veh.speed() < 3.0 && !veh.destroyed)
            .then(|| veh.to_world(def, seat.entry))
    }

    /// An empty seat worth taking near player `me`: the vehicle, the seat,
    /// and where to stand to get in.
    pub(super) fn seat_nearby(
        &self,
        game: &Game,
        world: &World,
        me: usize,
    ) -> Option<(usize, usize, Vec3)> {
        let p = &game.players[me];
        let feet = p.body.position;
        let mut best: Option<(usize, usize, Vec3, f32)> = None;
        for (v, veh) in game.vehicles.iter().enumerate() {
            if veh.destroyed || veh.upside_down() || self.riding.shunned.contains(&v) {
                continue;
            }
            let def = &game.vehicle_defs[veh.def];
            if veh.center.distance(feet) > BOARD_RANGE + def.radius {
                continue;
            }
            // Enemies aboard: not a ride.
            if veh.riders.iter().flatten().any(|&r| game.is_enemy(me, r)) {
                continue;
            }
            let driven = def.driver_seat().is_some_and(|d| veh.riders[d].is_some());
            for (s, seat) in def.seats.iter().enumerate() {
                if veh.riders[s].is_some() {
                    continue;
                }
                let wanted = match seat.role {
                    SeatRole::Driver => def.drive != Drive::Fixed,
                    // A gun: a turret on its own, or behind a teammate
                    // driving.
                    SeatRole::Gunner => def.drive == Drive::Fixed || driven,
                    SeatRole::Passenger => false,
                };
                let entry = veh.to_world(def, seat.entry);
                let d = entry.distance(feet);
                if wanted
                    && best.is_none_or(|b| d < b.3)
                    && Bot::visible(world, p.eye(), entry + Vec3::Z * 0.3)
                {
                    best = Some((v, s, entry, d));
                }
            }
        }
        best.map(|b| (b.0, b.1, b.2))
    }

    /// Whether a vehicle helps: roaming for kills. (With an objective to
    /// play, vehicles cost bots more than they gain: stuck on Zanzibar's
    /// paths, they stopped reaching the flags at all.)
    pub(super) fn worth_a_ride(game: &Game, me: usize) -> bool {
        Bot::objective(game, me).is_none()
    }

    /// On foot by a seat it wants: whether to hold the action key now.
    pub(super) fn board_now(game: &Game, me: usize, vehicle: usize) -> bool {
        matches!(
            game.vehicle_action(me),
            Some(VehicleAction::Enter { vehicle: v, .. }) if v == vehicle
        )
    }

    /// Out of a vehicle (or never in one): forget it, shunning one just
    /// left.
    pub(super) fn on_foot(&mut self) {
        let r = &mut self.riding;
        if let Some(v) = r.last.take() {
            if r.leaving && !r.shunned.contains(&v) {
                r.shunned.push(v);
            }
        }
        r.leaving = false;
    }

    /// This tick's controls while riding in seat `s` of vehicle `v`.
    pub(super) fn ride(
        &mut self,
        game: &Game,
        world: &World,
        nav: &NavGraph,
        me: usize,
        (v, s): (usize, usize),
    ) -> Command {
        let dt = TICK;
        self.riding.last = Some(v);
        let veh = &game.vehicles[v];
        let def = &game.vehicle_defs[veh.def];
        let seat = &def.seats[s];
        let mut cmd = Command::default();
        if self.riding.leaving {
            cmd.action = true;
            cmd.yaw = self.yaw;
            cmd.pitch = self.pitch;
            return cmd;
        }
        let target = self.find_target(game, world, me);
        if target != self.target {
            self.target = target;
            self.seen_for = 0.0;
            self.aim_error = Vec2::new(self.random() - 0.5, self.random() - 0.5) * 0.2;
        }
        self.pulse = !self.pulse;
        let gun = seat.weapon.and_then(|w| game.weapons.get(w));
        // Aim from where the crosshair's line starts.
        let eye = game.view_point(me);

        // Aim at whoever is in sight, if this seat has a gun.
        let mut aiming = false;
        if let (Some(t), Some(gun)) = (target, gun) {
            self.seen_for += dt;
            self.aim_error *= 1.0 - (2.5 * dt).min(1.0);
            let q = &game.players[t];
            // A gun that lobs its shots (the Wraith's) aims above the target
            // by its barrel's tilt.
            let lob = seat.turret.map_or(0.0, |g| g.elevation);
            let to = Bot::aim_point(Some(gun), eye, q) - eye;
            let dist = to.length();
            let yaw = to.y.atan2(to.x) + self.aim_error.x;
            let pitch = (to.z / dist.max(1e-4)).asin() + self.aim_error.y - lob;
            self.turn_to(yaw, pitch, dt);
            let off = wrap(yaw - self.yaw).abs() + (pitch - self.pitch).abs();
            let ready = self.seen_for > REACTION && off < FIRE_CONE * 2.0;
            let pull = |d: &crate::weapon::WeaponDef, pulse: bool| match d.behavior {
                TriggerBehavior::Spew => true,
                _ => pulse,
            };
            // Not into a blast of its own.
            let clear = |d: &crate::weapon::WeaponDef| {
                d.flight
                    .and_then(|f| f.blast)
                    .is_none_or(|b| dist > b.radius.1 + 1.0)
            };
            if ready && clear(gun) {
                cmd.fire = pull(gun, self.pulse);
            }
            // The second trigger: the Scorpion's machine gun up close, the
            // Banshee's bomb from further off.
            if let Some(alt) = seat.alt_weapon.and_then(|w| game.weapons.get(w)) {
                let wanted = match alt.flight.and_then(|f| f.blast) {
                    Some(_) => clear(alt),
                    None => dist < 15.0,
                };
                if ready && wanted {
                    match alt.input {
                        TriggerInput::Melee => cmd.melee = self.pulse,
                        _ => cmd.throw_grenade = pull(alt, self.pulse),
                    }
                }
            }
            aiming = true;
        }

        match seat.role {
            SeatRole::Driver if def.drive != Drive::Fixed => {
                self.drive(game, world, nav, me, v, target, aiming, &mut cmd);
                // A rocket closing on the Banshee: loop or roll out of its way.
                let incoming = game.projectiles.iter().any(|r| {
                    r.target == Some(Homing::Vehicle(v)) && r.position.distance(veh.center) < 20.0
                });
                if def.drive == Drive::Fly && incoming {
                    cmd.jump = self.pulse;
                    let roll = self.random();
                    if roll < 0.66 {
                        cmd.movement.x = if roll < 0.33 { -1.0 } else { 1.0 };
                    }
                }
            }
            _ => {
                // A gunner (or a passenger) stays while there's something to
                // shoot and someone driving (or it's a turret on its own).
                let driven = def.driver_seat().is_some_and(|d| veh.riders[d].is_some());
                let r = &mut self.riding;
                r.idle = if target.is_some() { 0.0 } else { r.idle + dt };
                r.driverless = if driven || def.drive == Drive::Fixed {
                    0.0
                } else {
                    r.driverless + dt
                };
                if r.idle > BORED || r.driverless > NO_DRIVER || seat.role == SeatRole::Passenger {
                    r.leaving = true;
                }
                if !aiming {
                    // Look around, slowly.
                    let sweep = self.yaw + 0.6 * dt;
                    self.turn_to(sweep, 0.0, dt);
                }
            }
        }
        cmd.yaw = self.yaw;
        cmd.pitch = self.pitch;
        cmd
    }

    /// Drive vehicle `v` along the route to where the bot is going (or at
    /// an enemy to run them over), backing up when stuck.
    #[allow(clippy::too_many_arguments)]
    fn drive(
        &mut self,
        game: &Game,
        world: &World,
        nav: &NavGraph,
        me: usize,
        v: usize,
        target: Option<usize>,
        aiming: bool,
        cmd: &mut Command,
    ) {
        let dt = TICK;
        let veh = &game.vehicles[v];
        let def = &game.vehicle_defs[veh.def];
        let at = veh.center;
        let flying = def.drive == Drive::Fly;

        // Stuck: back up a moment; stuck too often: get out.
        let r = &mut self.riding;
        let speed = veh.speed();
        if r.backing > 0.0 {
            r.backing -= dt;
            cmd.movement = Vec2::new(0.0, -1.0);
            return;
        }
        if speed < 0.6 {
            r.stalled += dt;
            r.free = 0.0;
        } else {
            r.stalled = 0.0;
            r.free += dt;
            if r.free > UNSTUCK {
                r.stalls = 0;
            }
        }
        if r.stalled > STALLED {
            r.stalled = 0.0;
            r.stalls += 1;
            r.backing = BACK_UP;
            self.route.clear();
            if r.stalls >= GIVE_UP_STALLS {
                r.leaving = true;
            }
            return;
        }

        // Where to: at an enemy nearby (to run them over), else along the
        // route to the objective or somewhere new.
        let objective = Bot::objective(game, me);
        if let Some((goal, _)) = objective {
            if goal.distance(at) < OBJECTIVE_ON_FOOT {
                self.riding.leaving = true;
                return;
            }
        }
        let ram = target
            .map(|t| game.players[t].body.position)
            .filter(|p| p.distance(at) < RAM_RANGE && def.drive == Drive::Wheels);
        let heading = match ram {
            Some(p) => Some(p),
            None => self.next_stop(world, nav, at, objective.map(|o| o.0)),
        };
        let Some(heading) = heading else {
            return;
        };
        let to = heading - at;
        cmd.movement = Vec2::new(0.0, 1.0);
        // Shelling someone: keep out of the blast.
        let shelling = target
            .zip(def.seats.iter().find_map(|s| s.weapon))
            .and_then(|(t, w)| Some((t, game.weapons.get(w)?.flight?.blast?)))
            .map(|(t, b)| (game.players[t].body.position.distance(at), b));
        if let (true, Some((d, b))) = (aiming, shelling) {
            if d < (b.radius.1 * 4.0).max(10.0) {
                cmd.movement.y = if d < b.radius.1 * 2.0 { -1.0 } else { 0.0 };
                self.riding.stalled = 0.0;
            }
        }
        // Teammates on foot ahead: brake rather than run them over.
        let fwd = veh.forward();
        let reach = 2.0 + speed * 0.6;
        let in_the_way = game.players.iter().enumerate().any(|(j, q)| {
            let off = q.body.position - at;
            j != me
                && q.alive
                && q.seat.is_none()
                && !game.is_enemy(me, j)
                && off.length() < reach
                && off.normalize_or_zero().dot(fwd) > 0.8
        });
        if in_the_way {
            cmd.movement.y = if speed > 1.0 { -1.0 } else { 0.0 };
            self.riding.stalled = 0.0;
        }
        if !aiming {
            let pitch = if flying {
                let up = heading.z + FLY_HEIGHT - at.z;
                (up / to.truncate().length().max(1.0))
                    .atan()
                    .clamp(-0.6, 0.6)
            } else {
                0.0
            };
            self.turn_to(to.y.atan2(to.x), pitch, dt);
        }
    }

    /// The next point on the bot's route from `at` (planning one to `goal`,
    /// or anywhere, when there's none).
    fn next_stop(
        &mut self,
        world: &World,
        nav: &NavGraph,
        at: Vec3,
        goal: Option<Vec3>,
    ) -> Option<Vec3> {
        if self.route.is_empty() {
            match goal.zip(nav.nearest(world, at)) {
                Some((goal, from)) => {
                    let to = nav.nearest(world, goal);
                    self.route = to
                        .and_then(|to| nav.path(from, to))
                        .map(|r| nav.smooth(world, &r))
                        .unwrap_or_default();
                }
                None => self.new_route(nav, world, at),
            }
        }
        while let Some(&next) = self.route.first() {
            let point = nav.points[next];
            if (point - at).truncate().length() < DRIVE_ARRIVED {
                self.route.remove(0);
            } else {
                return Some(point);
            }
        }
        goal
    }
}
