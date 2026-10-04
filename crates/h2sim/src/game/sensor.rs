//! The motion sensor: who shows up on a player's radar. Teammates always
//! do; enemies only while they move faster than a crouch-walk or have just
//! fired.

use super::Game;
use glam::Vec2;

/// How far the motion sensor reaches: 25 metres.
pub const SENSOR_RANGE: f32 = 25.0 / 3.048;
/// Seconds a shot keeps the shooter on others' sensors.
const SHOT_SHOWS: f32 = 1.5;
/// Moving faster than a crouch-walk by this much shows.
const SNEAK_MARGIN: f32 = 1.15;

/// Someone on a player's motion sensor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blip {
    /// Where they are from the viewer, across the ground (world axes).
    pub offset: Vec2,
    /// A teammate rather than an enemy.
    pub ally: bool,
    /// A vehicle with people in it rather than someone on foot.
    pub vehicle: bool,
}

impl Game {
    /// What player `viewer`'s motion sensor shows (nothing when the game
    /// has it off).
    pub fn sensor_blips(&self, viewer: usize) -> Vec<Blip> {
        let Some(me) = self.players.get(viewer) else {
            return Vec::new();
        };
        if !self.rules.options.radar || !me.alive {
            return Vec::new();
        }
        let here = me.body.position;
        let teams = self.rules.game_type.teams();
        let fast = self.movement.sneak_forward.max(0.1) * SNEAK_MARGIN;
        let fired = |j: usize| {
            let p = &self.players[j];
            let held = p.weapons.iter().chain(&p.left);
            let seat_guns = p.seat.and_then(|(v, s)| {
                let veh = self.vehicles.get(v)?;
                let main = veh.weapons.get(s)?.as_ref();
                let alt = veh.alt_weapons.get(s)?.as_ref();
                Some([main, alt])
            });
            held.map(|h| &h.state)
                .chain(seat_guns.into_iter().flatten().flatten())
                .any(|w| w.since_shot < SHOT_SHOWS)
        };
        // Riders show as one blip for their vehicle.
        let mut blips: Vec<(Option<usize>, Blip)> = Vec::new();
        for (j, p) in self.players.iter().enumerate() {
            if j == viewer || !p.alive {
                continue;
            }
            let offset = (p.body.position - here).truncate();
            if offset.length() > SENSOR_RANGE {
                continue;
            }
            let ally = teams && !self.is_enemy(viewer, j);
            let moving = p.body.velocity.length() > fast;
            if !(ally || moving || fired(j)) {
                continue;
            }
            let Some((v, _)) = p.seat else {
                let vehicle = false;
                blips.push((
                    None,
                    Blip {
                        offset,
                        ally,
                        vehicle,
                    },
                ));
                continue;
            };
            if let Some((_, b)) = blips.iter_mut().find(|(seen, _)| *seen == Some(v)) {
                // An enemy aboard makes it an enemy.
                b.ally &= ally;
                continue;
            }
            let at = self
                .vehicles
                .get(v)
                .map_or(p.body.position, |veh| veh.center);
            let offset = (at - here).truncate();
            let vehicle = true;
            blips.push((
                Some(v),
                Blip {
                    offset,
                    ally,
                    vehicle,
                },
            ));
        }
        blips.into_iter().map(|(_, b)| b).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{GameType, Options};
    use crate::testing::game;
    use glam::Vec3;

    #[test]
    fn the_sensor_shows_movers_shooters_and_teammates() {
        let mut g = game();
        for _ in 0..4 {
            g.add_player();
        }
        for p in &mut g.players {
            p.body.velocity = Vec3::ZERO;
        }
        g.players[0].body.position = Vec3::ZERO;
        g.players[1].body.position = Vec3::new(3.0, 0.0, 0.0);
        g.players[2].body.position = Vec3::new(0.0, 4.0, 0.0);
        g.players[3].body.position = Vec3::new(SENSOR_RANGE + 1.0, 0.0, 0.0);
        // Standing still and quiet: nobody shows.
        for p in &mut g.players {
            for h in &mut p.weapons {
                h.state.since_shot = f32::INFINITY;
            }
        }
        assert!(g.sensor_blips(0).is_empty());

        // Running and shooting show; out of range doesn't.
        g.players[1].body.velocity = Vec3::new(2.25, 0.0, 0.0);
        g.players[2].weapons[0].state.since_shot = 0.2;
        g.players[3].body.velocity = Vec3::new(2.25, 0.0, 0.0);
        let blips = g.sensor_blips(0);
        assert_eq!(blips.len(), 2);
        assert!(blips.iter().all(|b| !b.ally && !b.vehicle));
        assert!(blips.iter().any(|b| b.offset == Vec2::new(3.0, 0.0)));

        // Crouch-walking stays off it.
        g.players[1].body.velocity = Vec3::new(0.9, 0.0, 0.0);
        assert_eq!(g.sensor_blips(0).len(), 1);

        // A still teammate shows, as an ally.
        g.rules.game_type = GameType::TeamSlayer;
        g.players[1].team = g.players[0].team;
        let blips = g.sensor_blips(0);
        assert!(blips
            .iter()
            .any(|b| b.ally && b.offset == Vec2::new(3.0, 0.0)));

        // Radar off: nothing.
        g.rules.options = Options {
            radar: false,
            ..Options::default()
        };
        assert!(g.sensor_blips(0).is_empty());
    }
}
