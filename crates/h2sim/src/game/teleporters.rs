//! Teleporters: stepping onto one sends a player to the exit on the same
//! channel, keeping the way they face and move. Where a pair sends both
//! ways, arriving on the far pad doesn't send them straight back: it takes
//! stepping off a pad and onto one again.

use super::{Event, Game};
use glam::Vec3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Teleporter {
    pub entry: Vec3,
    pub exit: Vec3,
}

/// How close to a pad's centre counts as on it. The maps mark pads a
/// third of a unit above the floor.
const RADIUS: f32 = 0.4;
const BELOW: f32 = 0.6;
const ABOVE: f32 = 0.6;

fn on_pad(feet: Vec3, pad: Vec3) -> bool {
    let d = feet - pad;
    d.truncate().length() < RADIUS && d.z > -BELOW && d.z < ABOVE
}

impl Game {
    /// Send a player standing on a teleporter to its exit.
    pub(super) fn teleport(&mut self, i: usize) {
        let p = &self.players[i];
        if !p.alive || p.seat.is_some() {
            return;
        }
        let feet = p.body.position;
        let entry = self.teleporters.iter().find(|t| on_pad(feet, t.entry));
        if p.teleported {
            // Just arrived: nothing until they step off.
            let on_exit = self.teleporters.iter().any(|t| on_pad(feet, t.exit));
            self.players[i].teleported = entry.is_some() || on_exit;
            return;
        }
        let Some(t) = entry.copied() else {
            return;
        };
        let p = &mut self.players[i];
        p.body.position = t.exit;
        p.teleported = true;
        self.events.push(Event::Teleported {
            player: i,
            from: feet,
            to: p.body.position,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::tests::{floor, game};
    use crate::game::Command;
    use glam::Vec2;

    #[test]
    fn a_two_way_teleporter_sends_once_per_step_onto_it() {
        let world = floor();
        let mut g = game();
        let me = g.add_player();
        g.item_spawns.clear();
        g.item_timers.clear();
        let (a, b) = (Vec3::new(2.0, 0.0, 0.0), Vec3::new(-10.0, 0.0, 0.0));
        g.teleporters = vec![
            Teleporter { entry: a, exit: b },
            Teleporter { entry: b, exit: a },
        ];
        g.players[me].body.position = Vec3::ZERO;
        g.players[me].yaw = 0.0;
        let walk = |dir: f32| Command {
            movement: Vec2::new(0.0, 1.0),
            yaw: dir,
            ..Command::default()
        };
        // Walk onto pad A: off to B, and stay there.
        let mut sent = 0;
        for _ in 0..120 {
            g.step(&world, &[walk(0.0)]);
            sent += g
                .events
                .drain(..)
                .filter(|e| matches!(e, Event::Teleported { .. }))
                .count();
            if sent > 0 {
                break;
            }
        }
        assert_eq!(sent, 1);
        assert!(g.players[me].body.position.distance(b) < 0.2);
        for _ in 0..30 {
            g.step(&world, &[Command::default()]);
        }
        // It carries on a little with the speed it came in at.
        assert!(g.players[me].body.position.distance(b) < 0.6, "sent back");
        // Off the pad and back on: back to A.
        for _ in 0..60 {
            g.step(&world, &[walk(std::f32::consts::PI)]);
        }
        for _ in 0..120 {
            g.step(&world, &[walk(0.0)]);
            if g.players[me].body.position.x > 0.0 {
                break;
            }
        }
        assert!(g.players[me].body.position.x > 0.0);
    }
}
