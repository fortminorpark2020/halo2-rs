//! Halo 2's autoaim: a round fired close enough to an enemy goes at them,
//! and the crosshair turns red while one is in reach; and magnetism, which
//! draws a controller's aim to them.

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
    /// with `def` zoomed to level `zoom`, and the point to steer them at
    /// (`None`: leave them where the crosshair is).
    ///
    /// Each weapon's tags give an autoaim angle and range: an enemy in
    /// sight, no farther than the range and within the angle of the
    /// crosshair, is hit by rounds aimed near them; the closest to the
    /// crosshair wins. Halo 2 measures the angle to small target spheres
    /// on the enemy's model (chest, head, legs) and pulls the round to the
    /// one it's on or near; here the body's centre line stands in for
    /// them, from the bottom of its capsule up to the middle of the head.
    /// A round aimed just over someone's head is steered into it, and one
    /// aimed at the edge of their body is steered to the middle, so it's
    /// as sure to hit as one aimed just off it. Zoomed in, the range grows
    /// and the angle shrinks with the magnification (Halopedia says
    /// autoaim scales with zoom; the tags don't say how). Weapons whose
    /// tags say aim assists work only zoomed (the Sniper Rifle and the
    /// Beam Rifle) get none unzoomed.
    pub fn autoaim(
        &self,
        world: &World,
        shooter: usize,
        eye: Vec3,
        dir: Vec3,
        def: &WeaponDef,
        zoom: u32,
    ) -> Option<(usize, Option<Vec3>)> {
        let cone = aim_cone(def, zoom, def.autoaim_angle, def.autoaim_range)?;
        self.pick(world, shooter, eye, dir, cone, |j| {
            self.is_enemy(shooter, j)
        })
        .map(|p| (p.player, p.steer))
    }

    /// Who the crosshair is on, as Halo 2's HUD shows it: the player
    /// autoaim picks, teammates included, and whether they're a teammate.
    /// Over a teammate the crosshair turns green (each weapon's HUD tag
    /// has a green "crosshair_friendly" for an autoaim on a friend). Only
    /// enemies have rounds steered at them (`autoaim`).
    pub fn autoaim_target(
        &self,
        world: &World,
        shooter: usize,
        eye: Vec3,
        dir: Vec3,
        def: &WeaponDef,
        zoom: u32,
    ) -> Option<(usize, bool)> {
        let cone = aim_cone(def, zoom, def.autoaim_angle, def.autoaim_range)?;
        self.pick(world, shooter, eye, dir, cone, |_| true)
            .map(|p| (p.player, !self.is_enemy(shooter, p.player)))
    }

    /// The enemy a controller's aim is drawn to, aiming along `dir` from
    /// `eye` with `def` zoomed to level `zoom`: one in sight within the
    /// weapon's magnetism angle and range (its tags; wider and farther
    /// than its autoaim's), picked as autoaim picks, and narrowing and
    /// reaching farther zoomed in the same way. Halo 2's controller aim
    /// slows on them and follows them (the globals' magnetism friction and
    /// adhesion).
    pub fn magnetism(
        &self,
        world: &World,
        shooter: usize,
        eye: Vec3,
        dir: Vec3,
        def: &WeaponDef,
        zoom: u32,
    ) -> Option<Magnet> {
        let (angle, range) = aim_cone(def, zoom, def.magnetism_angle, def.magnetism_range)
            .filter(|&(angle, _)| angle > 0.0)?;
        let p = self.pick(world, shooter, eye, dir, (angle, range), |j| {
            self.is_enemy(shooter, j)
        })?;
        let body = &self.players[p.player].body;
        Some(Magnet {
            player: p.player,
            off: p.off,
            angle,
            centre: body.position + Vec3::Z * body.height() * 0.5,
        })
    }

    /// The player among those `wanted` says that `shooter`'s aim assists
    /// pick, aiming along `dir` from `eye` within an `(angle, range)`
    /// cone.
    fn pick(
        &self,
        world: &World,
        shooter: usize,
        eye: Vec3,
        dir: Vec3,
        (angle, range): (f32, f32),
        wanted: impl Fn(usize) -> bool,
    ) -> Option<Pick> {
        let own = self.riding(shooter).map(|(v, _)| v);
        // (angle off the crosshair, distance, player, where to steer).
        let mut best: Option<(f32, f32, usize, Option<Vec3>)> = None;
        for (j, q) in self.players.iter().enumerate() {
            if j == shooter || !q.alive || !wanted(j) || !self.exposed(j) {
                continue;
            }
            if own.is_some() && q.seat.map(|(v, _)| v) == own {
                continue;
            }
            let (base, height, radius) = (q.body.position, q.body.height(), q.body.biped.radius);
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
            let found = match ray_capsule(eye, dir, base, height, radius) {
                // Right on them: to the middle too, unless that would turn
                // a headshot into a body shot (or back), as it can looking
                // steeply up or down, or the level is in the way (someone
                // half behind cover).
                Some(t) => (t <= range && world.raycast(eye, dir, t).is_none()).then(|| {
                    let head =
                        |t: f32, dir: Vec3| eye.z + dir.z * t > base.z + height - HEAD_HEIGHT;
                    let steer = (d > 1e-3)
                        .then(|| to / d)
                        .and_then(|s| {
                            ray_capsule(eye, s, base, height, radius).filter(|&u| {
                                head(u, s) == head(t, dir) && world.raycast(eye, s, u).is_none()
                            })
                        })
                        .map(|_| point);
                    (0.0, t, steer)
                }),
                None => {
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
        best.map(|(off, _, player, steer)| Pick { player, off, steer })
    }
}

/// A player the aim assists pick.
struct Pick {
    player: usize,
    /// Radians off the crosshair: 0 with the crosshair on them.
    off: f32,
    /// Where to steer rounds at them (`None`: where the crosshair is).
    steer: Option<Vec3>,
}

/// The enemy a controller's aim is drawn to (`Game::magnetism`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Magnet {
    pub player: usize,
    /// Radians off the crosshair: 0 with the crosshair on them.
    pub off: f32,
    /// The weapon's magnetism angle as zoomed now: how far off the pull
    /// reaches.
    pub angle: f32,
    /// Their middle, which moves with them.
    pub centre: Vec3,
}

/// A weapon's aim assist `angle` and `range` zoomed to level `zoom`:
/// narrower and farther by the magnification. None when it has none
/// (no range, or unzoomed for a weapon that assists only zoomed).
fn aim_cone(def: &WeaponDef, zoom: u32, angle: f32, range: f32) -> Option<(f32, f32)> {
    if def.autoaim_zoomed_only && zoom == 0 {
        return None;
    }
    let m = def.magnification(zoom);
    (range > 0.0).then_some((angle / m, range * m))
}

#[cfg(test)]
mod tests {
    use crate::collision::World;
    use crate::game::{ray_capsule, Command, Event, Game, HEAD_HEIGHT};
    use crate::testing::{floor, game};
    use crate::weapon::WeaponDef;
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

    /// Of `n` rounds aimed at `at`, how many hit player 1.
    fn hits(g: &mut Game, at: Vec3, n: usize) -> usize {
        let world = floor();
        let eye = g.players[0].eye();
        let to = at - eye;
        let aim = Command {
            yaw: to.y.atan2(to.x),
            pitch: (to.z / to.length()).asin(),
            ..Command::default()
        };
        let (mut fired, mut hit) = (0, 0);
        // One pull every 8 ticks: the test rifle fires 10 rounds a second.
        for tick in 0..n * 8 {
            // Never stopping to reload.
            for h in &mut g.players[0].weapons {
                h.state.loaded = 12;
            }
            let cmd = Command {
                fire: tick % 8 == 0,
                ..aim
            };
            g.events.clear();
            g.step(&world, &[cmd, Command::default()]);
            for e in &g.events {
                if let Event::Shot { hit_player, .. } = e {
                    fired += 1;
                    hit += usize::from(*hit_player == Some(1));
                }
            }
        }
        assert_eq!(fired, n);
        hit
    }

    #[test]
    fn rounds_on_the_edge_of_a_body_hit_as_often_as_in_the_middle() {
        // 0.6 degrees of spread, which strays up to 0.16 units off at 15:
        // nearly the body's radius.
        let spread = |g: &mut Game| {
            for w in &mut g.weapons {
                w.error_angle = (0.6f32.to_radians(), 0.6f32.to_radians());
                // Harmless, so no one dies.
                w.damage = 0.0;
                w.damage_lower_bound = 0.0;
            }
        };
        let n = 40;
        let shoot_at = |side: f32, assisted: bool| {
            let mut g = duel(15.0);
            spread(&mut g);
            if !assisted {
                g.weapons[0].autoaim_range = 0.0;
            }
            let q = &g.players[1];
            let at = q.body.position + Vec3::new(0.0, side, q.body.height() * 0.6);
            hits(&mut g, at, n)
        };
        let radius = duel(15.0).players[1].body.biped.radius;
        let middle = shoot_at(0.0, true);
        assert_eq!(middle, n);
        // Unaided, the spread takes rounds aimed at the edge off it...
        assert!(shoot_at(radius - 0.02, false) < n * 3 / 4);
        // ...but autoaim sends them at the middle, as it does those aimed
        // just off the body.
        assert_eq!(shoot_at(radius - 0.02, true), middle);
        assert_eq!(shoot_at(radius + 0.02, true), middle);
    }

    #[test]
    fn someone_half_behind_cover_is_hit_where_they_show() {
        // A wall a unit in front of them hides their left half (and a bit).
        let s = 50.0;
        let world = World::new(
            &[
                [-s, -s, 0.0],
                [s, -s, 0.0],
                [s, s, 0.0],
                [-s, s, 0.0],
                [14.0, -2.0, 0.0],
                [14.0, 0.02, 0.0],
                [14.0, 0.02, 3.0],
                [14.0, -2.0, 3.0],
            ],
            &[0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
        );
        let g = duel(15.0);
        let def = g.weapons[0].clone();
        let eye = g.players[0].eye();
        let q = &g.players[1];
        let visible = q.body.position + Vec3::new(0.0, q.body.biped.radius - 0.03, 0.4);
        let to = (visible - eye).normalize();
        // Still aimed at (the crosshair is red), but not steered into the wall.
        assert_eq!(g.autoaim(&world, 0, eye, to, &def, 0), Some((1, None)));
        assert!(world.raycast(eye, to, 15.0).is_none());
    }

    #[test]
    fn steering_never_turns_a_body_shot_into_a_headshot_or_back() {
        let g = duel(3.0);
        // Nothing in the way, not even the floor.
        let world = World::new(&[], &[]);
        let def = g.weapons[0].clone();
        let q = &g.players[1];
        let (base, height, radius) = (q.body.position, q.body.height(), q.body.biped.radius);
        let head = |eye: Vec3, dir: Vec3| {
            ray_capsule(eye, dir, base, height, radius)
                .map(|t| eye.z + dir.z * t > base.z + height - HEAD_HEIGHT)
        };
        let (mut steered, mut kept) = (0, 0);
        // From well below to well above them, at points all over them.
        for eye_z in [-2.5f32, -1.0, 0.62, 2.0, 3.5] {
            let eye = Vec3::new(0.0, 0.0, eye_z);
            for i in 0..=20 {
                for k in 0..=40 {
                    let at = base
                        + Vec3::new(
                            0.0,
                            radius * (i as f32 / 10.0 - 1.0),
                            height * k as f32 / 40.0,
                        );
                    let dir = (at - eye).normalize();
                    let Some(was) = head(eye, dir) else {
                        continue;
                    };
                    match g.autoaim(&world, 0, eye, dir, &def, 0) {
                        Some((1, Some(point))) => {
                            steered += 1;
                            assert_eq!(head(eye, (point - eye).normalize()), Some(was));
                        }
                        Some((1, None)) => kept += 1,
                        other => panic!("{other:?}"),
                    }
                }
            }
        }
        // Most are steered; a few, looking steeply up or down, aren't.
        assert!(steered > kept * 4 && kept > 0, "{steered} {kept}");
    }

    #[test]
    fn snipers_aim_for_you_only_zoomed_in() {
        // The Sniper Rifle's tags: 1 degree out to 10 units, only zoomed.
        let mut g = duel(8.0);
        for w in &mut g.weapons {
            w.autoaim_angle = 1f32.to_radians();
            w.autoaim_range = 10.0;
            w.autoaim_zoomed_only = true;
            w.zoom_levels = 2;
            w.zoom_range = (3.5, 9.5);
        }
        let world = floor();
        let def = g.weapons[0].clone();
        let eye = g.players[0].eye();
        let q = &g.players[1];
        // At the edge of the body: steered to the middle zoomed in, not
        // aimed for at all (no red crosshair) unzoomed.
        let edge = q.body.position + Vec3::new(0.0, q.body.biped.radius - 0.02, 0.4);
        let over = q.body.position + Vec3::Z * (q.body.height() + 0.03);
        let to = (edge - eye).normalize();
        assert_eq!(g.autoaim(&world, 0, eye, to, &def, 0), None);
        let zoomed = g.autoaim(&world, 0, eye, to, &def, 1);
        assert!(matches!(zoomed, Some((1, Some(_)))), "{zoomed:?}");
        // Just over the head no-scoped: a clean miss.
        assert_eq!(hits(&mut g, over, 1), 0);
        // Without the flag the same round goes into the head.
        g.weapons[0].autoaim_zoomed_only = false;
        assert_eq!(hits(&mut g, over, 1), 1);
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
        // But the crosshair over them knows they're a friend (green).
        assert_eq!(
            g.autoaim_target(&world, 0, eye, to, &def, 0),
            Some((1, true))
        );
        g.players[1].team = 1 - g.players[0].team;
        assert_eq!(
            g.autoaim_target(&world, 0, eye, to, &def, 0),
            Some((1, false))
        );
    }

    /// Aiming from player 0's eye `off` radians to the side of player
    /// 1's chest.
    fn aim_beside(g: &Game, off: f32) -> (Vec3, Vec3) {
        let eye = g.players[0].eye();
        let q = &g.players[1];
        let to = q.body.position + Vec3::Z * q.body.height() * 0.6 - eye;
        let yaw = to.y.atan2(to.x) + off;
        let pitch = (to.z / to.length()).asin();
        let dir = Vec3::new(
            yaw.cos() * pitch.cos(),
            yaw.sin() * pitch.cos(),
            pitch.sin(),
        );
        (eye, dir)
    }

    /// The Battle Rifle's magnetism: 6 degrees out to 21 world units.
    fn magnetic(range: f32) -> Game {
        let mut g = duel(range);
        for w in &mut g.weapons {
            w.magnetism_angle = 6f32.to_radians();
            w.magnetism_range = 21.0;
        }
        g
    }

    #[test]
    fn magnetism_draws_aim_to_an_enemy_in_its_cone() {
        let world = floor();
        let g = magnetic(15.0);
        let def = g.weapons[0].clone();
        let (eye, dir) = aim_beside(&g, 5f32.to_radians());
        let m = g
            .magnetism(&world, 0, eye, dir, &def, 0)
            .expect("drawn to them");
        assert_eq!(m.player, 1);
        assert!(m.off > 4f32.to_radians() && m.off < m.angle, "{m:?}");
        // Right on them: no angle off at all.
        let (eye, on) = aim_beside(&g, 0.0);
        assert_eq!(
            g.magnetism(&world, 0, eye, on, &def, 0).map(|m| m.off),
            Some(0.0)
        );
        // Wider than its autoaim's 3 degrees, but not beyond its own.
        assert_eq!(g.autoaim(&world, 0, eye, dir, &def, 0), None);
        let (eye, wide) = aim_beside(&g, 7f32.to_radians());
        assert_eq!(g.magnetism(&world, 0, eye, wide, &def, 0), None);
        // Not beyond its range.
        let far = magnetic(25.0);
        let (eye, dir) = aim_beside(&far, 2f32.to_radians());
        assert_eq!(far.magnetism(&world, 0, eye, dir, &def, 0), None);
    }

    #[test]
    fn magnetism_skips_teammates_walls_and_unzoomed_snipers() {
        let g = magnetic(15.0);
        let def = g.weapons[0].clone();
        let (eye, dir) = aim_beside(&g, 2f32.to_radians());
        // A wall between them.
        let s = 50.0;
        let walled = World::new(
            &[
                [-s, -s, 0.0],
                [s, -s, 0.0],
                [s, s, 0.0],
                [-s, s, 0.0],
                [7.0, -5.0, 0.0],
                [7.0, 5.0, 0.0],
                [7.0, 5.0, 3.0],
                [7.0, -5.0, 3.0],
            ],
            &[0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
        );
        assert_eq!(g.magnetism(&walled, 0, eye, dir, &def, 0), None);
        let world = floor();
        let mut team = magnetic(15.0);
        team.rules.game_type = crate::game::GameType::TeamSlayer;
        team.players[1].team = team.players[0].team;
        assert_eq!(team.magnetism(&world, 0, eye, dir, &def, 0), None);
        // The Sniper Rifle (4 degrees to 14, zoomed only).
        let sniper = WeaponDef {
            magnetism_angle: 4f32.to_radians(),
            magnetism_range: 14.0,
            autoaim_zoomed_only: true,
            zoom_levels: 2,
            zoom_range: (3.5, 9.5),
            ..def
        };
        let (eye, near) = aim_beside(&g, 0.5f32.to_radians());
        assert_eq!(g.magnetism(&world, 0, eye, near, &sniper, 0), None);
        assert!(g.magnetism(&world, 0, eye, near, &sniper, 1).is_some());
    }
}
