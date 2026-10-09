//! Players on foot are solid to each other: a body (an upright capsule of
//! the biped's radius and height) can't walk into another, and one coming
//! down on another's head stands on it, as in Halo 2 (where you can block
//! a doorway or stand on a teammate). Only the player moving is pushed
//! out, so no one is shoved into the level; someone jumping up under a
//! player standing on their head carries them up.

use super::Game;
use glam::Vec2;

/// Overlaps shallower than this (world units) are left alone, so bodies
/// resting on each other don't jitter.
const SLOP: f32 = 1e-4;

impl Game {
    /// A body others bump into: alive, on foot, and a player (campaign
    /// actors pass through each other as before).
    fn solid(&self, i: usize) -> bool {
        let p = &self.players[i];
        p.alive && p.seat.is_none() && p.actor.is_none()
    }

    /// Push player `i` out of anyone they overlap: up onto their head if
    /// that's the shorter way (or carry someone standing on `i`'s head up
    /// with them), out to the side otherwise.
    pub(super) fn keep_apart(&mut self, i: usize) {
        if !self.solid(i) {
            return;
        }
        for j in 0..self.players.len() {
            if j == i || !self.solid(j) {
                continue;
            }
            let (a, b) = (&self.players[i].body, &self.players[j].body);
            let reach = a.biped.radius + b.biped.radius;
            let apart = (a.position - b.position).truncate();
            let d = apart.length();
            if d >= reach {
                continue;
            }
            // How far the higher one would have to go up to stand on the
            // other's head.
            let (upper, lower) = if a.position.z >= b.position.z {
                (i, j)
            } else {
                (j, i)
            };
            let under = &self.players[lower].body;
            let (top, lift) = (under.position.z + under.height(), under.velocity.z);
            let rise = top - self.players[upper].body.position.z;
            let side = reach - d;
            if rise <= SLOP {
                continue;
            }
            if rise < side {
                self.players[upper].body.stand_on(top, lift);
            } else {
                // Right on top of each other: back the way they face.
                let away = if d > 1e-4 {
                    apart / d
                } else {
                    -Vec2::from_angle(self.players[i].yaw)
                };
                let body = &mut self.players[i].body;
                body.position += (away * side).extend(0.0);
                let into = body.velocity.truncate().dot(away);
                if into < 0.0 {
                    body.velocity -= (away * into).extend(0.0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{Command, Game};
    use crate::testing::{floor, game};
    use glam::{Vec2, Vec3};

    /// Two players, the first at `a` and the second at `b`.
    fn pair(a: Vec3, b: Vec3) -> Game {
        let mut g = game();
        g.add_player();
        g.add_player();
        g.players[0].body.position = a;
        g.players[1].body.position = b;
        g
    }

    fn flat_distance(g: &Game) -> f32 {
        (g.players[0].body.position - g.players[1].body.position)
            .truncate()
            .length()
    }

    #[test]
    fn players_cant_walk_through_each_other() {
        let world = floor();
        let mut g = pair(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0));
        let reach = g.players[0].body.biped.radius * 2.0;
        let run = Command {
            movement: Vec2::Y,
            yaw: 0.0,
            ..Command::default()
        };
        let mut closest = f32::MAX;
        for _ in 0..120 {
            g.step(&world, &[run, Command::default()]);
            closest = closest.min(flat_distance(&g));
        }
        assert!(closest >= reach - 1e-3, "got within {closest}");
        assert!(g.players[0].body.position.x < 1.0, "stopped short");
        assert!(
            (g.players[1].body.position - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-3,
            "the one standing still isn't shoved: {}",
            g.players[1].body.position
        );
        // Both running at each other: they never overlap (round bodies
        // pushing at each other slide around each other, never through).
        let back = Command {
            movement: Vec2::Y,
            yaw: std::f32::consts::PI,
            ..Command::default()
        };
        for _ in 0..120 {
            g.step(&world, &[run, back]);
            closest = closest.min(flat_distance(&g));
        }
        assert!(closest >= reach - 1e-3, "got within {closest}");
    }

    #[test]
    fn a_player_can_stand_on_anothers_head() {
        let world = floor();
        let mut g = pair(Vec3::new(0.0, 0.0, 2.0), Vec3::ZERO);
        let idle = [Command::default(); 2];
        for _ in 0..120 {
            g.step(&world, &idle);
        }
        let (top, under) = (&g.players[0].body, &g.players[1].body);
        assert!(
            (top.position.z - (under.position.z + under.height())).abs() < 0.01,
            "feet at {} on a head at {}",
            top.position.z,
            under.position.z + under.height()
        );
        assert!(top.grounded);
        // And jump off it.
        let jump = Command {
            jump: true,
            ..Command::default()
        };
        g.step(&world, &[jump, Command::default()]);
        assert!(g.players[0].body.velocity.z > 1.0);
    }

    #[test]
    fn a_long_drop_onto_a_head_hurts_like_one_onto_the_floor() {
        let world = floor();
        // 8 units above the head, and 8 above the floor beside them.
        let mut g = pair(Vec3::new(0.0, 0.0, 8.725), Vec3::ZERO);
        g.add_player();
        g.players[2].body.position = Vec3::new(3.0, 0.0, 8.0);
        let idle = [Command::default(); 3];
        for _ in 0..240 {
            g.step(&world, &idle);
        }
        let left = |p: &crate::game::Spartan| p.shield + p.health;
        let (on_head, on_floor) = (left(&g.players[0]), left(&g.players[2]));
        assert!(on_floor < 115.0, "{on_floor}");
        assert!((on_head - on_floor).abs() < 5.0, "{on_head} vs {on_floor}");
        // The one landed on isn't hurt.
        assert_eq!(left(&g.players[1]), 115.0);
    }

    #[test]
    fn the_dead_dont_block() {
        let world = floor();
        let mut g = pair(Vec3::ZERO, Vec3::new(0.1, 0.0, 0.0));
        g.damage(1, None, f32::INFINITY, false);
        g.players[1].body.position = Vec3::new(0.1, 0.0, 0.0);
        g.step(&world, &[Command::default(), Command::default()]);
        assert!(flat_distance(&g) < 0.2);
    }
}
