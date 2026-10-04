//! Rounds that fly through the air: plasma bolts, needles, rockets, tank
//! shells and mortars. Each moves every tick (speeding up or slowing down,
//! falling, turning toward its target) until it hits something or runs out
//! of range, and explosive ones go off in a blast. Needles stick in whoever
//! they hit and pop a moment later, or all at once in a supercombine.

use super::{Event, Game};
use crate::collision::World;
use crate::player::GRAVITY;
use crate::weapon::{Blast, Flight, Sticky, WeaponState};
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

/// A round stuck in someone, about to go off (a needle).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StuckRound {
    pub weapon: usize,
    pub owner: usize,
    pub victim: usize,
    /// Where it is, from the victim's feet.
    pub offset: Vec3,
    /// Seconds before it goes off.
    pub fuse: f32,
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
        let sticky = def.flight.and_then(|f| f.sticky);
        let hit_player = match what {
            Struck::Player(j) => {
                self.hurt(j, Some(p.owner), damage, false, def.armor);
                if let (Some(s), true) = (sticky, self.players[j].alive) {
                    self.stick(p, j, s);
                }
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

    /// A needle in player `j`: it goes off once its fuse burns down (with
    /// any that follow it in quickly), unless it's the one that makes a
    /// supercombine.
    fn stick(&mut self, p: Projectile, j: usize, s: Sticky) {
        // Fuses vary a little, needle to needle.
        let spread = ((p.position.x * 12.9898 + p.position.y * 78.233).sin() * 43758.547)
            .fract()
            .abs();
        let fuse = s.fuse.0 + (s.fuse.1 - s.fuse.0) * spread;
        // Those already in wait for the newest, to go off together.
        for r in self.stuck.iter_mut().filter(|r| r.victim == j) {
            r.fuse = r.fuse.max(fuse);
        }
        self.stuck.push(StuckRound {
            weapon: p.weapon,
            owner: p.owner,
            victim: j,
            offset: p.position - self.players[j].body.position,
            fuse,
        });
        let count = self.stuck.iter().filter(|r| r.victim == j).count();
        if count >= s.supercombine.max(1) {
            self.supercombine(j, p.owner, p.weapon, s);
        }
    }

    /// Enough needles in player `j` at once: they all go off together.
    fn supercombine(&mut self, j: usize, owner: usize, weapon: usize, s: Sticky) {
        self.stuck.retain(|r| r.victim != j);
        let q = &self.players[j];
        let at = q.body.position + Vec3::Z * q.body.height() * 0.5;
        self.hurt(j, Some(owner), s.super_damage, false, s.super_armor);
        if let Some(b) = s.super_blast {
            // The blast's own reach (no level to shield anyone this close).
            for k in 0..self.players.len() {
                let other = &self.players[k];
                let d = (other.body.position + Vec3::Z * other.body.height() * 0.5).distance(at);
                if k != j && other.alive && d < b.radius.1 {
                    self.hurt(k, Some(owner), b.damage_at(d), false, b.armor);
                }
            }
        }
        self.events.push(Event::Impact {
            weapon,
            position: at,
            normal: Vec3::Z,
            hit_player: Some(j),
            exploded: true,
        });
    }

    /// Needles stuck in people burn down their fuses and pop.
    pub(super) fn step_stuck(&mut self, dt: f32) {
        let mut popped = Vec::new();
        self.stuck.retain_mut(|r| {
            r.fuse -= dt;
            if r.fuse > 0.0 {
                return true;
            }
            popped.push(*r);
            false
        });
        for r in popped {
            let Some(s) = self.weapons.get(r.weapon).and_then(|w| w.flight?.sticky) else {
                continue;
            };
            let q = &self.players[r.victim];
            if !q.alive {
                continue;
            }
            let at = q.body.position + r.offset;
            self.hurt(r.victim, Some(r.owner), s.damage, false, s.armor);
            self.events.push(Event::Impact {
                weapon: r.weapon,
                position: at,
                normal: Vec3::Z,
                hit_player: Some(r.victim),
                exploded: false,
            });
        }
        // The dead shed theirs.
        let players = &self.players;
        self.stuck.retain(|r| players[r.victim].alive);
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
            self.hurt(j, Some(owner), b.damage_at(d), false, b.armor);
        }
        self.blast_vehicles(at, owner, b.damage.1, b.radius);
    }
}

#[cfg(test)]
mod tests {
    use super::Projectile;
    use crate::game::{Command, Event, Game};
    use crate::testing::{floor, game};
    use crate::weapon::{Blast, Flight, Sticky, WeaponDef};
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
                    armor: Default::default(),
                }),
                sticky: None,
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

    /// A needle that sticks: 1 on impact, 2 when it pops, 3 together kill.
    fn needles(g: &mut Game) -> (Projectile, Sticky) {
        let s = Sticky {
            fuse: (0.5, 0.7),
            damage: 2.0,
            armor: Default::default(),
            supercombine: 3,
            super_damage: 400.0,
            super_armor: Default::default(),
            super_blast: None,
        };
        let needler = WeaponDef {
            name: "needler".into(),
            damage: 1.0,
            damage_lower_bound: 1.0,
            range: 100.0,
            flight: Some(Flight {
                speed: (10.0, 10.0),
                acceleration_range: (0.0, 0.0),
                gravity: 0.0,
                homing: 0.0,
                homes_on_vehicles: false,
                blast: None,
                sticky: Some(s),
            }),
            ..g.weapons[0].clone()
        };
        g.weapons.push(needler);
        let p = Projectile {
            weapon: g.weapons.len() - 1,
            owner: 0,
            position: Vec3::new(10.0, 0.0, 1.0),
            velocity: Vec3::X,
            travelled: 1.0,
            target: None,
            age: 0.0,
        };
        (p, s)
    }

    #[test]
    fn needles_stick_and_pop_and_enough_supercombine() {
        let mut g = duel();
        let (p, s) = needles(&mut g);
        let full = g.players[1].shield + g.players[1].health;
        g.stick(p, 1, s);
        assert_eq!(g.stuck.len(), 1);
        // A second later it has popped.
        for _ in 0..60 {
            g.step(&floor(), &[Command::default(), Command::default()]);
        }
        assert!(g.stuck.is_empty());
        let left = g.players[1].shield + g.players[1].health;
        assert!(full - left > 1.0, "the pop hurt: {full} -> {left}");
        // Three at once: a supercombine.
        g.stick(p, 1, s);
        g.stick(p, 1, s);
        assert!(g.players[1].alive);
        g.stick(p, 1, s);
        assert!(!g.players[1].alive, "supercombined");
        assert!(g.stuck.is_empty());
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
