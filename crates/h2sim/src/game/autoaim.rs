//! Halo 2's autoaim: a round fired close enough to an enemy goes at them,
//! and the crosshair turns red while it would.

use super::{ray_capsule, Game, HEAD_HEIGHT};
use crate::collision::World;
use crate::weapon::WeaponDef;
use glam::Vec3;

/// Forward, right and up for aiming along `dir` (as `Spartan::basis`).
pub(super) fn aim_basis(dir: Vec3) -> (Vec3, Vec3, Vec3) {
    let f = dir.normalize_or(Vec3::X);
    let r = f.cross(Vec3::Z).normalize_or(Vec3::X);
    (f, r, r.cross(f))
}

impl Game {
    /// The enemy `shooter`'s rounds go at, aiming along `dir` from `eye`
    /// with `def` zoomed to level `zoom`, and the point to steer them at to
    /// hit (`None` when the crosshair is already on them).
    ///
    /// Each weapon's tags give an autoaim angle and range: an enemy in
    /// sight, no farther than the range and within the angle of the
    /// crosshair, is hit by rounds aimed near them; the closest to the
    /// crosshair wins. Halo 2 measures the angle to small target spheres
    /// on the enemy's model (chest, head, legs); here it's measured to the
    /// body's centre line, from the bottom of its capsule up to the middle
    /// of the head, so a round aimed just over someone's head is steered
    /// into it. Zoomed in, the range grows and the angle shrinks with the
    /// magnification (Halopedia says autoaim scales with zoom; the tags
    /// don't say how).
    pub fn autoaim(
        &self,
        world: &World,
        shooter: usize,
        eye: Vec3,
        dir: Vec3,
        def: &WeaponDef,
        zoom: u32,
    ) -> Option<(usize, Option<Vec3>)> {
        let zoom = def.magnification(zoom);
        let (angle, range) = (def.autoaim_angle / zoom, def.autoaim_range * zoom);
        if range <= 0.0 {
            return None;
        }
        let own = self.riding(shooter).map(|(v, _)| v);
        // (angle off the crosshair, distance, player, where to steer).
        let mut best: Option<(f32, f32, usize, Option<Vec3>)> = None;
        for (j, q) in self.players.iter().enumerate() {
            if !q.alive || !self.is_enemy(shooter, j) || !self.exposed(j) {
                continue;
            }
            if own.is_some() && q.seat.map(|(v, _)| v) == own {
                continue;
            }
            let (base, height, radius) = (q.body.position, q.body.height(), q.body.biped.radius);
            let found = match ray_capsule(eye, dir, base, height, radius) {
                // Right on them.
                Some(t) => {
                    (t <= range && world.raycast(eye, dir, t).is_none()).then_some((0.0, t, None))
                }
                None => {
                    // The point on the centre line nearest the aim.
                    let bottom = base.z + radius;
                    let top = (base.z + height - HEAD_HEIGHT * 0.5).max(bottom);
                    let w = eye - Vec3::new(base.x, base.y, 0.0);
                    let flat = 1.0 - dir.z * dir.z;
                    let z = if flat > 1e-6 {
                        (w.z - dir.z * dir.dot(w)) / flat
                    } else {
                        eye.z
                    };
                    let point = Vec3::new(base.x, base.y, z.clamp(bottom, top));
                    let to = point - eye;
                    let d = to.length();
                    let off = dir.angle_between(to);
                    (d > 1e-3
                        && d <= range
                        && off <= angle
                        && world.raycast(eye, to / d, d).is_none())
                    .then_some((off, d, Some(point)))
                }
            };
            if let Some((off, d, steer)) = found {
                if best.is_none_or(|b| (off, d) < (b.0, b.1)) {
                    best = Some((off, d, j, steer));
                }
            }
        }
        best.map(|(_, _, j, steer)| (j, steer))
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{Command, Event, Game};
    use crate::testing::{floor, game};
    use glam::Vec3;

    /// The Battle Rifle's autoaim: 3 degrees out to 17 world units (its
    /// tags), on the test game's rifle, which has no spread.
    fn duel(range: f32) -> Game {
        let mut g = game();
        for w in &mut g.weapons {
            w.autoaim_angle = 3f32.to_radians();
            w.autoaim_range = 17.0;
        }
        g.add_player();
        g.add_player();
        g.players[0].body.position = Vec3::ZERO;
        g.players[1].body.position = Vec3::new(range, 0.0, 0.0);
        g
    }

    /// Fire one round at the chest of player 1, `off` radians to the side;
    /// whether it hits them.
    fn shoot(g: &mut Game, off: f32) -> bool {
        let world = floor();
        let eye = g.players[0].eye();
        let q = &g.players[1];
        let chest = q.body.position + Vec3::Z * q.body.height() * 0.6;
        let to = chest - eye;
        let cmd = Command {
            yaw: to.y.atan2(to.x) + off,
            pitch: (to.z / to.length()).asin(),
            fire: true,
            ..Command::default()
        };
        g.events.clear();
        g.step(&world, &[cmd, Command::default()]);
        g.events.iter().any(|e| {
            matches!(
                e,
                Event::Shot {
                    hit_player: Some(1),
                    ..
                }
            )
        })
    }

    #[test]
    fn rounds_near_an_enemy_go_at_them() {
        // 2 degrees off misses a body 10 units away unaided.
        let mut g = duel(10.0);
        assert!(shoot(&mut g, 2f32.to_radians()));
        // Beyond the angle, or beyond the range, they don't.
        let mut g = duel(10.0);
        assert!(!shoot(&mut g, 4f32.to_radians()));
        let mut g = duel(20.0);
        assert!(!shoot(&mut g, 1f32.to_radians()));
    }

    #[test]
    fn rounds_aimed_just_over_the_head_hit_it() {
        let world = floor();
        let mut g = duel(10.0);
        // Shields down: a headshot kills, a body shot doesn't.
        g.players[1].shield = 0.0;
        let eye = g.players[0].eye();
        let q = &g.players[1];
        let over = q.body.position + Vec3::Z * (q.body.height() + 0.2);
        let to = over - eye;
        let cmd = Command {
            yaw: to.y.atan2(to.x),
            pitch: (to.z / to.length()).asin(),
            fire: true,
            ..Command::default()
        };
        g.step(&world, &[cmd, Command::default()]);
        assert!(g
            .events
            .iter()
            .any(|e| matches!(e, Event::Killed { headshot: true, .. })));
    }

    #[test]
    fn zoom_reaches_farther_through_a_narrower_cone() {
        let mut g = duel(30.0);
        let def = {
            let mut d = g.weapons[0].clone();
            d.zoom_levels = 1;
            d.zoom_range = (2.0, 2.0);
            d
        };
        g.weapons[0] = def.clone();
        let world = floor();
        let eye = g.players[0].eye();
        let q = &g.players[1];
        let chest = q.body.position + Vec3::Z * q.body.height() * 0.6;
        let aim = |off: f32| {
            let to = chest - eye;
            let yaw = to.y.atan2(to.x) + off;
            let pitch = (to.z / to.length()).asin();
            Vec3::new(
                yaw.cos() * pitch.cos(),
                yaw.sin() * pitch.cos(),
                pitch.sin(),
            )
        };
        let off = 1f32.to_radians();
        assert_eq!(g.autoaim(&world, 0, eye, aim(off), &def, 0), None);
        let zoomed = g.autoaim(&world, 0, eye, aim(off), &def, 1);
        assert!(matches!(zoomed, Some((1, Some(_)))), "{zoomed:?}");
        assert_eq!(g.autoaim(&world, 0, eye, aim(2.0 * off), &def, 1), None);
    }

    #[test]
    fn teammates_are_not_aimed_at() {
        let mut g = duel(10.0);
        g.rules.game_type = crate::game::GameType::TeamSlayer;
        g.players[1].team = g.players[0].team;
        let world = floor();
        let eye = g.players[0].eye();
        let def = g.weapons[0].clone();
        let to = (g.players[1].eye() - eye).normalize();
        assert_eq!(g.autoaim(&world, 0, eye, to, &def, 0), None);
    }
}
