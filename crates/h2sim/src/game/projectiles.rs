//! Rounds that fly through the air: plasma bolts, needles, rockets, tank
//! shells and mortars. Each moves every tick (speeding up or slowing down,
//! falling, turning toward its target) until it hits something or runs out
//! of range, and explosive ones go off in a blast.

use super::{Event, Game};
use crate::collision::World;
use crate::player::GRAVITY;
use crate::weapon::{Blast, Flight, WeaponState};
use glam::Vec3;

/// Seconds a round flies at most.
const LIFETIME: f32 = 12.0;
/// Radians either side of the aim a homing round picks its target within.
const LOCK_CONE: f32 = 0.25;

/// What a homing round chases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Homing {
    Player(usize),
    Vehicle(usize),
}

/// A round in flight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projectile {
    /// The weapon that fired it (index into the game's weapons).
    pub weapon: usize,
    pub owner: usize,
    pub position: Vec3,
    pub velocity: Vec3,
    /// Distance flown so far.
    pub travelled: f32,
    pub target: Option<Homing>,
    pub age: f32,
}

/// What a round ran into.
#[derive(Clone, Copy)]
enum Struck {
    Level,
    Player(usize),
    Vehicle(usize),
    /// Nothing: out of range or time.
    Nothing,
}

impl Game {
    /// Send a round of `weapon` from `eye` along `dir`.
    pub(super) fn launch(&mut self, i: usize, eye: Vec3, dir: Vec3, weapon: usize, flight: Flight) {
        let target = (flight.homing > 0.0)
            .then(|| self.homing_target(i, eye, dir, flight.homes_on_vehicles))
            .flatten();
        self.projectiles.push(Projectile {
            weapon,
            owner: i,
            position: eye,
            velocity: dir * flight.speed.0.max(0.1),
            travelled: 0.0,
            target,
            age: 0.0,
        });
    }

    /// The enemy (or, for rockets, the enemy vehicle) closest to where
    /// player `i` aims.
    fn homing_target(&self, i: usize, eye: Vec3, dir: Vec3, vehicles: bool) -> Option<Homing> {
        let in_cone = |p: Vec3| {
            let to = p - eye;
            let d = to.length();
            (d > 0.5 && d < 60.0)
                .then(|| to.dot(dir) / d)
                .filter(|c| c.acos() < LOCK_CONE)
        };
        let own = self.riding(i).map(|(v, _)| v);
        let mut best: Option<(Homing, f32)> = None;
        if vehicles {
            for (v, veh) in self.vehicles.iter().enumerate() {
                let enemy = veh.riders.iter().flatten().any(|&r| self.is_enemy(i, r));
                if veh.destroyed || Some(v) == own || !enemy {
                    continue;
                }
                if let Some(c) = in_cone(veh.center) {
                    if best.is_none_or(|b| c > b.1) {
                        best = Some((Homing::Vehicle(v), c));
                    }
                }
            }
        } else {
            for (j, q) in self.players.iter().enumerate() {
                if j == i || !q.alive || !self.is_enemy(i, j) || !self.exposed(j) {
                    continue;
                }
                if let Some(c) = in_cone(q.body.position + Vec3::Z * q.body.height() * 0.5) {
                    if best.is_none_or(|b| c > b.1) {
                        best = Some((Homing::Player(j), c));
                    }
                }
            }
        }
        best.map(|b| b.0)
    }

    /// Where a homing round's target is now, if it's still there.
    fn target_point(&self, t: Homing) -> Option<Vec3> {
        match t {
            Homing::Player(j) => {
                let q = self.players.get(j).filter(|q| q.alive)?;
                Some(q.body.position + Vec3::Z * q.body.height() * 0.5)
            }
            Homing::Vehicle(v) => {
                let veh = self.vehicles.get(v).filter(|v| !v.destroyed)?;
                Some(veh.center)
            }
        }
    }

    pub(super) fn step_projectiles(&mut self, world: &World, dt: f32) {
        let mut done = Vec::new();
        for k in 0..self.projectiles.len() {
            let mut p = self.projectiles[k];
            let Some(def) = self.weapons.get(p.weapon) else {
                done.push((k, p, Struck::Nothing, Vec3::Z));
                continue;
            };
            let Some(flight) = def.flight else {
                done.push((k, p, Struck::Nothing, Vec3::Z));
                continue;
            };
            let range = def.range;
            p.age += dt;

            // Turn toward the target, up to the round's turn rate.
            if let Some(aim) = p.target.and_then(|t| self.target_point(t)) {
                let speed = p.velocity.length();
                let now = p.velocity / speed.max(1e-4);
                let want = (aim - p.position).normalize_or(now);
                let angle = now.angle_between(want);
                if angle > 1e-4 {
                    let turn = (flight.homing * dt / angle).min(1.0);
                    p.velocity = now.lerp(want, turn).normalize_or(now) * speed;
                }
            }
            // Speed up or slow down with distance, and fall.
            let before = flight.speed_at(p.travelled);
            let after = flight.speed_at(p.travelled + p.velocity.length() * dt);
            p.velocity *= after / before.max(1e-3);
            p.velocity.z -= GRAVITY * flight.gravity * dt;

            let step = p.velocity * dt;
            let len = step.length();
            let dir = step / len.max(1e-6);
            let wall = world.raycast_hit(p.position, dir, len);
            let mut hit = wall.map(|(t, n)| (t, n, Struck::Level));
            let closer =
                |hit: &Option<(f32, Vec3, Struck)>, t: f32| t <= len && hit.is_none_or(|h| t < h.0);
            if let Some((j, t, _)) = self.trace_players(p.owner, p.position, dir) {
                if closer(&hit, t) {
                    hit = Some((t, -dir, Struck::Player(j)));
                }
            }
            if let Some((v, t)) = self.trace_vehicles(p.owner, p.position, dir) {
                if closer(&hit, t) {
                    hit = Some((t, -dir, Struck::Vehicle(v)));
                }
            }
            match hit {
                Some((t, n, what)) => {
                    p.position += dir * t;
                    p.travelled += t;
                    done.push((k, p, what, n));
                }
                None => {
                    p.position += step;
                    p.travelled += len;
                    if p.travelled > range || p.age > LIFETIME {
                        done.push((k, p, Struck::Nothing, -dir));
                    }
                }
            }
            self.projectiles[k] = p;
        }
        for &(k, ..) in done.iter().rev() {
            self.projectiles.remove(k);
        }
        for (_, p, what, normal) in done {
            self.land(world, p, what, normal);
        }
    }

    /// A round ends: it hurts whoever it hit and, if explosive, goes off.
    fn land(&mut self, world: &World, p: Projectile, what: Struck, normal: Vec3) {
        let Some(def) = self.weapons.get(p.weapon).cloned() else {
            return;
        };
        let blast = def.flight.and_then(|f| f.blast);
        // Out of range: plain bolts just fizzle.
        if matches!(what, Struck::Nothing) && blast.is_none() {
            return;
        }
        let damage = WeaponState::damage_at(&def, p.travelled);
        let hit_player = match what {
            Struck::Player(j) => {
                self.damage(j, Some(p.owner), damage, false);
                Some(j)
            }
            Struck::Vehicle(v) => {
                self.damage_vehicle(v, Some(p.owner), damage);
                None
            }
            _ => None,
        };
        // Go off just off the surface, so walls don't shield the blast.
        let at = p.position + normal * 0.05;
        if let Some(b) = blast {
            self.blast(world, at, p.owner, b, hit_player);
        }
        self.events.push(Event::Impact {
            weapon: p.weapon,
            position: at,
            normal,
            hit_player,
            exploded: blast.is_some(),
        });
    }

    /// A blast at `at` set off by `owner`: it hurts and throws players
    /// in reach (`through_walls` is one it reaches regardless, a grenade
    /// stuck to them) and vehicles.
    pub(super) fn blast(
        &mut self,
        world: &World,
        at: Vec3,
        owner: usize,
        b: Blast,
        through_walls: Option<usize>,
    ) {
        for j in 0..self.players.len() {
            let p = &self.players[j];
            if !p.alive {
                continue;
            }
            let centre = p.body.position + Vec3::Z * p.body.height() * 0.5;
            let d = centre.distance(at);
            if d >= b.radius.1 {
                continue;
            }
            // Walls shield players from the blast.
            let to = centre - at;
            if through_walls != Some(j)
                && world
                    .raycast(at, to / d.max(1e-4), d)
                    .is_some_and(|t| t < d - 0.05)
            {
                continue;
            }
            // Riders inside are hurt only by what wrecks their vehicle.
            if p.seat.is_some() && !self.exposed(j) {
                continue;
            }
            let push = b.push * b.falloff(d);
            if push > 0.0 && p.seat.is_none() {
                let away = (to.normalize_or(Vec3::Z) + Vec3::Z * 0.5).normalize();
                let body = &mut self.players[j].body;
                body.velocity += away * push;
                if body.velocity.z > 0.0 {
                    body.grounded = false;
                }
            }
            self.damage(j, Some(owner), b.damage_at(d), false);
        }
        self.blast_vehicles(at, owner, b.damage.1, b.radius);
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{Command, Event, Game};
    use crate::testing::{floor, game};
    use crate::weapon::{Blast, Flight, WeaponDef};
    use glam::Vec3;

    /// A launcher whose rockets fly at 10 units a second and blow up.
    fn launcher(g: &mut Game) -> usize {
        let rocket = WeaponDef {
            name: "launcher".into(),
            damage: 150.0,
            damage_lower_bound: 150.0,
            range: 100.0,
            flight: Some(Flight {
                speed: (10.0, 10.0),
                acceleration_range: (0.0, 0.0),
                gravity: 0.0,
                homing: 0.0,
                homes_on_vehicles: true,
                blast: Some(Blast {
                    damage: (75.0, 200.0),
                    radius: (1.0, 2.0),
                    push: 2.0,
                }),
            }),
            ..g.weapons[0].clone()
        };
        g.weapons.push(rocket);
        g.weapons.len() - 1
    }

    /// Player 0 with the launcher, 10 units from player 1.
    fn duel() -> Game {
        let mut g = game();
        let w = launcher(&mut g);
        g.rules.starting_weapons = vec![w];
        g.add_player();
        g.add_player();
        g.players[0].body.position = Vec3::ZERO;
        g.players[1].body.position = Vec3::new(10.0, 0.0, 0.0);
        g.players[0].readying = 0.0;
        g
    }

    #[test]
    fn rockets_fly_then_blow_up() {
        let mut g = duel();
        let world = floor();
        let fire = Command {
            fire: true,
            ..Command::default()
        };
        g.step(&world, &[fire, Command::default()]);
        assert_eq!(g.projectiles.len(), 1, "the rocket is in the air");
        assert_eq!(g.players[1].health, g.rules.health, "not there yet");
        // About a second later it arrives (10 units at 10 a second).
        for _ in 0..90 {
            g.step(&world, &[Command::default(), Command::default()]);
        }
        assert!(g.projectiles.is_empty());
        assert!(!g.players[1].alive, "a direct hit kills");
        assert!(g.events.iter().any(|e| matches!(
            e,
            Event::Impact {
                exploded: true,
                hit_player: Some(1),
                ..
            }
        )));
    }

    #[test]
    fn blasts_hurt_less_further_out_and_throw_people() {
        let mut g = duel();
        let w = g.weapons.len() - 1;
        let b = g.weapons[w].flight.unwrap().blast.unwrap();
        assert_eq!(b.damage_at(0.5), 200.0);
        assert!((b.damage_at(1.5) - 137.5).abs() < 1e-3);
        assert_eq!(b.damage_at(2.5), 0.0);
        g.players[1].body.position = Vec3::new(1.85, 0.0, 0.0);
        g.blast(&floor(), Vec3::new(0.0, 0.0, 0.3), 0, b, None);
        assert!(!g.players[0].alive, "too close to your own rocket");
        assert!(g.players[1].alive, "at the edge");
        assert!(g.players[1].body.velocity.length() > 0.1, "thrown");
    }
}
