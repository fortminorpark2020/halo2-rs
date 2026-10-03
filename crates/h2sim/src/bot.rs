//! Computer-controlled players. A bot roams the map along the walking graph
//! until it sees someone, then strafes, shoots, throws grenades and melees
//! like a (forgiving) player. Bots produce the same `Command`s as people.

use crate::collision::World;
use crate::game::{Command, Game};
use crate::nav::NavGraph;
use blam_cache::weapon::TriggerBehavior;
use glam::{Vec2, Vec3};

/// How far a bot sees.
const SIGHT: f32 = 35.0;
/// Seconds between seeing someone and shooting at them.
const REACTION: f32 = 0.35;
/// Radians per second a bot can turn.
const TURN_RATE: f32 = 5.0;
/// Within this angle of its target a bot pulls the trigger.
const FIRE_CONE: f32 = 0.06;
/// Close enough to a route point to head for the next one.
const ARRIVED: f32 = 0.5;

pub struct Bot {
    rng: u32,
    route: Vec<usize>,
    target: Option<usize>,
    seen_for: f32,
    strafe: f32,
    strafe_left: f32,
    /// Aim wobble that settles while tracking a target.
    aim_error: Vec2,
    yaw: f32,
    pitch: f32,
    last_position: Vec3,
    stuck_for: f32,
    pulse: bool,
    grenade_wait: f32,
}

fn wrap(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    (a + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI
}

impl Bot {
    pub fn new(seed: u32) -> Bot {
        Bot {
            rng: seed.max(1).wrapping_mul(0x9E37_79B9) | 1,
            route: Vec::new(),
            target: None,
            seen_for: 0.0,
            strafe: 0.0,
            strafe_left: 0.0,
            aim_error: Vec2::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            last_position: Vec3::ZERO,
            stuck_for: 0.0,
            pulse: false,
            grenade_wait: 3.0,
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

    fn visible(world: &World, from: Vec3, to: Vec3) -> bool {
        let d = to - from;
        let len = d.length();
        len < SIGHT && world.raycast(from, d / len.max(1e-4), len).is_none()
    }

    /// The nearest enemy in sight.
    fn find_target(&self, game: &Game, world: &World, me: usize) -> Option<usize> {
        let eye = game.players[me].eye();
        game.players
            .iter()
            .enumerate()
            .filter(|(j, p)| *j != me && p.alive)
            .map(|(j, p)| (j, p.eye().distance(eye), p.eye() - Vec3::Z * 0.1))
            .filter(|&(_, d, chest)| d < SIGHT && Bot::visible(world, eye, chest))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(j, _, _)| j)
    }

    /// Pick somewhere to go and plan the way there.
    fn new_route(&mut self, nav: &NavGraph, world: &World, from: Vec3) {
        self.route.clear();
        let Some(start) = nav.nearest(world, from) else {
            return;
        };
        for _ in 0..4 {
            let goal = (self.random() * nav.points.len() as f32) as usize;
            if let Some(route) = nav.path(start, goal.min(nav.points.len() - 1)) {
                if route.len() > 1 {
                    self.route = route;
                    return;
                }
            }
        }
        self.route = vec![start];
    }

    /// Turn toward a direction at the bot's turn rate.
    fn turn_to(&mut self, yaw: f32, pitch: f32, dt: f32) {
        let step = TURN_RATE * dt;
        self.yaw = wrap(self.yaw + wrap(yaw - self.yaw).clamp(-step, step));
        self.pitch += (pitch - self.pitch).clamp(-step, step);
    }

    /// This tick's controls for player `me`.
    pub fn think(&mut self, game: &Game, world: &World, nav: &NavGraph, me: usize) -> Command {
        let dt = crate::game::TICK;
        let p = &game.players[me];
        if !p.alive {
            self.route.clear();
            self.target = None;
            self.yaw = p.yaw;
            self.pitch = 0.0;
            return Command {
                yaw: p.yaw,
                ..Command::default()
            };
        }
        let feet = p.body.position;
        let eye = p.eye();
        let mut cmd = Command::default();
        // Stuck against something: hop, then give up on the route.
        let moved = (feet - self.last_position).truncate().length();
        self.last_position = feet;
        if moved < 0.4 * dt {
            self.stuck_for += dt;
        } else {
            self.stuck_for = 0.0;
        }
        if self.stuck_for > 0.6 {
            cmd.jump = true;
        }
        if self.stuck_for > 2.0 {
            self.stuck_for = 0.0;
            self.route.clear();
        }

        let target = self.find_target(game, world, me);
        if target != self.target {
            self.target = target;
            self.seen_for = 0.0;
            self.aim_error = Vec2::new(self.random() - 0.5, self.random() - 0.5) * 0.4;
        }
        self.strafe_left -= dt;
        if self.strafe_left <= 0.0 {
            self.strafe_left = 0.4 + self.random() * 1.0;
            self.strafe = [-1.0, 0.0, 1.0][(self.random() * 3.0) as usize % 3];
        }
        self.grenade_wait -= dt;
        self.pulse = !self.pulse;

        let mut walk_to: Option<Vec3> = None;
        if let Some(t) = target {
            self.seen_for += dt;
            self.aim_error *= 1.0 - (2.5 * dt).min(1.0);
            let q = &game.players[t];
            let aim_at = q.eye() - Vec3::Z * 0.12 + q.body.velocity * 0.1;
            let to = aim_at - eye;
            let dist = to.length();
            let yaw = to.y.atan2(to.x) + self.aim_error.x;
            let pitch = (to.z / dist.max(1e-4)).asin() + self.aim_error.y;
            self.turn_to(yaw, pitch, dt);
            let off = wrap(yaw - self.yaw).abs() + (pitch - self.pitch).abs();
            let def = p.held().and_then(|h| game.weapons.get(h.weapon));
            let melee_only = p
                .held()
                .is_some_and(|h| game.rules.lunge_weapons.contains(&h.weapon));
            if self.seen_for > REACTION && off < FIRE_CONE && !melee_only {
                // Semi-automatic weapons need the trigger released between shots.
                cmd.fire = match def.map(|d| d.behavior) {
                    Some(TriggerBehavior::Spew) | None => true,
                    _ => self.pulse,
                };
            }
            let reach = if melee_only { 2.0 } else { 0.9 };
            if dist < reach && self.seen_for > REACTION {
                cmd.melee = self.pulse;
            }
            let has_grenade = p.frags + p.plasmas > 0;
            if has_grenade && self.grenade_wait <= 0.0 && (4.0..12.0).contains(&dist) {
                cmd.throw_grenade = true;
                self.grenade_wait = 4.0 + self.random() * 6.0;
            }
            // Close in from afar, back off when too close, strafe always.
            let fwd = if melee_only || dist > 8.0 {
                1.0
            } else if dist < 2.5 {
                -1.0
            } else {
                0.0
            };
            cmd.movement = Vec2::new(self.strafe, fwd);
            if self.held_empty(game, me) {
                cmd.reload = true;
            }
        } else {
            if self.route.is_empty() {
                self.new_route(nav, world, feet);
            }
            while let Some(&next) = self.route.first() {
                let point = nav.points[next];
                if (point - feet).truncate().length() < ARRIVED && (point.z - feet.z).abs() < 1.0 {
                    self.route.remove(0);
                } else {
                    walk_to = Some(point);
                    break;
                }
            }
            if walk_to.is_none() {
                self.route.clear();
            }
            // Reload while nothing is around.
            cmd.reload = self.held_partly_empty(game, me);
        }
        if let Some(goal) = walk_to {
            let to = (goal - feet).truncate();
            self.turn_to(to.y.atan2(to.x), 0.0, dt);
            // Walk toward the goal whichever way the bot faces.
            let (s, c) = self.yaw.sin_cos();
            let dir = to.normalize_or_zero();
            cmd.movement = Vec2::new(dir.x * s - dir.y * c, dir.x * c + dir.y * s);
        }
        cmd.yaw = self.yaw;
        cmd.pitch = self.pitch;
        cmd
    }

    fn held_empty(&self, game: &Game, me: usize) -> bool {
        let p = &game.players[me];
        p.held().is_some_and(|h| {
            game.weapons
                .get(h.weapon)
                .is_some_and(|d| d.uses_ammo() && h.state.loaded == 0 && h.state.reserve > 0)
        })
    }

    fn held_partly_empty(&self, game: &Game, me: usize) -> bool {
        let p = &game.players[me];
        p.held().is_some_and(|h| {
            game.weapons.get(h.weapon).is_some_and(|d| {
                d.uses_ammo() && h.state.loaded < d.magazine_size && h.state.reserve > 0
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bots_turn_toward_and_shoot_players_they_see() {
        let world = crate::game::tests::floor();
        let mut game = crate::game::tests::game();
        let a = game.add_player();
        let b = game.add_player();
        game.players[a].body.position = Vec3::new(0.0, 0.0, 0.0);
        game.players[b].body.position = Vec3::new(5.0, 3.0, 0.0);
        let nav = NavGraph::build(&world, &[Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0)]);
        let mut bot = Bot::new(7);
        let mut fired = false;
        let mut hurt = false;
        for _ in 0..120 {
            let cmd = bot.think(&game, &world, &nav, a);
            fired |= cmd.fire;
            let mut cmds = vec![Command::default(); 2];
            cmds[a] = cmd;
            game.step(&world, &cmds);
            // Keep the target standing still.
            game.players[b].body.position = Vec3::new(5.0, 3.0, 0.0);
            hurt |= game.players[b].shield < 70.0 || !game.players[b].alive;
        }
        assert!(fired);
        assert!(hurt);
    }
}
