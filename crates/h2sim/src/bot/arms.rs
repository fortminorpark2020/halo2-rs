//! Bots and weapons: going for better weapons lying nearby, taking a second
//! gun into the left hand, and switching to the gun that suits the fight.

use super::Bot;
use crate::collision::World;
use crate::game::{Command, Game, HeldWeapon, ItemKind};
use glam::{Vec2, Vec3};

/// How far a bot goes out of its way for a better weapon.
const WEAPON_SEARCH: f32 = 15.0;
/// A weapon must be worth this much more than the one it replaces.
const WORTH_SWAPPING: f32 = 1.0;
/// Seconds between weapon switches in a fight.
const SWITCH_WAIT: f32 = 1.5;
/// Seconds going for one weapon before giving up on it.
const FETCH_TIME: f32 = 8.0;
/// Weapons given up on, remembered until respawning.
const MAX_SHUNNED: usize = 8;

/// How much bots want a weapon kind (power weapons most).
pub(super) fn weapon_value(game: &Game, w: usize) -> f32 {
    let r = &game.rules;
    if [r.flag_weapon, r.ball_weapon, r.bomb_weapon].contains(&Some(w)) {
        return 0.0;
    }
    let Some(def) = game.weapons.get(w) else {
        return 0.0;
    };
    match def.name.as_str() {
        "rocket_launcher" => 10.0,
        "sniper_rifle" | "energy_blade" | "energy_sword" => 9.0,
        "shotgun" | "beam_rifle" => 8.0,
        "flak_cannon" => 7.0,
        "battle_rifle" | "covenant_carbine" => 6.0,
        "brute_shot" => 5.0,
        "smg" | "needler" | "plasma_rifle" | "brute_plasma_rifle" => 4.0,
        _ => 3.0,
    }
}

/// How much a held weapon is worth: nothing once it's out of ammo.
fn held_value(game: &Game, h: &HeldWeapon) -> f32 {
    let empty = game
        .weapons
        .get(h.weapon)
        .is_some_and(|d| d.uses_ammo() && h.state.loaded + h.state.reserve == 0);
    if empty {
        0.0
    } else {
        weapon_value(game, h.weapon)
    }
}

/// How well a held weapon suits a fight `dist` away.
fn fight_value(game: &Game, h: &HeldWeapon, dist: f32) -> f32 {
    let Some(def) = game.weapons.get(h.weapon) else {
        return 0.0;
    };
    let mut v = held_value(game, h);
    if def.uses_ammo() && h.state.loaded == 0 {
        v -= 2.0;
    }
    let lunges = game.rules.lunge_weapons.contains(&h.weapon);
    let blast = def.flight.and_then(|f| f.blast).map_or(0.0, |b| b.radius.1);
    if dist > def.range.max(1.0) || (lunges && dist > 6.0) || (def.name == "shotgun" && dist > 8.0)
    {
        v *= 0.3;
    }
    if (def.zoom_levels > 1 && dist < 4.0) || dist < blast + 1.0 {
        v *= 0.4;
    }
    v.max(0.0)
}

impl Bot {
    /// What a weapon on the ground would add for player `me`: more than a
    /// weapon they'd swap out, or a second gun for the left hand.
    fn weapon_gain(game: &Game, me: usize, w: usize) -> f32 {
        let p = &game.players[me];
        if p.objective.is_some() {
            return 0.0;
        }
        let value = weapon_value(game, w);
        let dual = p.left.is_none()
            && game.one_handed(w)
            && p.weapons
                .get(p.current)
                .is_some_and(|h| game.one_handed(h.weapon));
        if p.weapons.iter().any(|h| h.weapon == w) {
            // Only as a second gun (otherwise it's just ammo).
            return if dual && value >= 3.0 { 2.0 } else { 0.0 };
        }
        let worst = match p.weapons.len() {
            0 | 1 => 0.0,
            _ => p
                .weapons
                .iter()
                .map(|h| held_value(game, h))
                .fold(f32::MAX, f32::min),
        };
        let gain = value - worst;
        if dual && value >= 3.0 {
            gain.max(2.0)
        } else {
            gain
        }
    }

    /// Where to go for a weapon worth having: the one already being
    /// fetched while it's still there, else the best in sight. Gives up on
    /// one that takes too long to get to.
    pub(super) fn weapon_to_fetch(
        &mut self,
        game: &Game,
        world: &World,
        me: usize,
        dt: f32,
    ) -> Option<Vec3> {
        let eye = game.players[me].eye();
        let wanted = self.weapons_wanted(game, me);
        let kept = self.fetching.and_then(|(spot, t)| {
            wanted
                .iter()
                .find(|w| w.0.distance(spot) < 0.5)
                .map(|w| (w.0, t + dt))
        });
        let found = || {
            wanted
                .iter()
                .filter(|w| Bot::visible(world, eye, w.0 + Vec3::Z * 0.2))
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|w| (w.0, 0.0))
        };
        self.fetching = kept.or_else(found);
        let (at, t) = self.fetching?;
        if t > FETCH_TIME {
            self.shun_weapon(at);
            return None;
        }
        Some(at)
    }

    /// Stop going for a weapon there (no way to it, or taking too long).
    pub(super) fn shun_weapon(&mut self, at: Vec3) {
        if self.shunned_weapons.len() >= MAX_SHUNNED {
            self.shunned_weapons.remove(0);
        }
        self.shunned_weapons.push(at);
        self.fetching = None;
    }

    /// Weapons lying close by worth having, and how much (less the farther
    /// away). With an objective to play, only the closest are worth the
    /// detour.
    fn weapons_wanted(&self, game: &Game, me: usize) -> Vec<(Vec3, f32)> {
        let feet = game.players[me].body.position;
        let search = if Bot::objective(game, me).is_some() {
            WEAPON_SEARCH * 0.5
        } else {
            WEAPON_SEARCH
        };
        let lying = game
            .item_spawns
            .iter()
            .zip(&game.item_timers)
            .filter(|(_, t)| **t <= 0.0)
            .filter_map(|(s, _)| match s.kind {
                ItemKind::Weapon(w) => Some((s.position, w)),
                _ => None,
            })
            .chain(game.dropped.iter().map(|d| (d.position, d.weapon)));
        lying
            .filter_map(|(at, w)| {
                let d = at.distance(feet);
                let gain = Bot::weapon_gain(game, me, w);
                let shunned = self.shunned_weapons.iter().any(|s| s.distance(at) < 0.5);
                let wanted = d < search && gain > WORTH_SWAPPING && !shunned;
                wanted.then_some((at, gain - d * 0.1))
            })
            .collect()
    }

    /// Standing on a weapon worth having: stop and hold the button that
    /// takes it (swapping out the weaker gun, or into the left hand).
    pub(super) fn pick_up(&mut self, game: &Game, me: usize, cmd: &mut Command) {
        let p = &game.players[me];
        let worth = |w: usize| Bot::weapon_gain(game, me, w) > WORTH_SWAPPING;
        if game.dual_prompt(me).is_some_and(worth) {
            // Holding switch takes it into the left hand.
            cmd.switch_weapon = true;
            cmd.movement = Vec2::ZERO;
        } else if game.swap_prompt(me).is_some_and(worth) {
            // A swap replaces the gun in hand: have the weaker one out.
            let current = p
                .weapons
                .get(p.current)
                .map_or(0.0, |h| held_value(game, h));
            let other = p
                .weapons
                .get(1 - p.current.min(1))
                .map_or(f32::MAX, |h| held_value(game, h));
            if current > other && self.switch_wait <= 0.0 && p.readying <= 0.0 {
                cmd.switch_weapon = true;
                self.switch_wait = 0.5;
            } else {
                cmd.action = true;
            }
            cmd.movement = Vec2::ZERO;
        }
    }

    /// In a fight `dist` away: switch to the other gun when it suits the
    /// fight better (not while dual wielding, which would drop the left
    /// gun, unless the right one is spent).
    pub(super) fn choose_weapon(&mut self, game: &Game, me: usize, dist: f32, cmd: &mut Command) {
        let p = &game.players[me];
        if self.switch_wait > 0.0 || p.weapons.len() < 2 || p.readying > 0.0 {
            return;
        }
        let (Some(now), Some(other)) = (p.weapons.get(p.current), p.weapons.get(1 - p.current))
        else {
            return;
        };
        let (now, other) = (fight_value(game, now, dist), fight_value(game, other, dist));
        let switch = if p.left.is_some() {
            now == 0.0 && other > 0.0
        } else {
            other > now + 1.0
        };
        if switch {
            cmd.switch_weapon = true;
            self.switch_wait = SWITCH_WAIT;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::tests::{floor, game};
    use crate::nav::NavGraph;
    use crate::{Bot, Command};

    /// A game with an SMG (one-handed) and a rocket launcher.
    fn armory() -> (Game, usize, usize) {
        let mut g = game();
        let mut smg = g.weapons[0].clone();
        smg.name = "smg".into();
        smg.dual = Some(crate::weapon::DualWield {
            minimum_error: 0.0,
            error_angle: (0.0, 0.0),
            damage_scale: 1.0,
        });
        let mut rockets = g.weapons[0].clone();
        rockets.name = "rocket_launcher".into();
        rockets.dual = None;
        g.weapons = vec![smg, rockets];
        g.rules.starting_weapons = vec![0];
        g.item_spawns.clear();
        g.item_timers.clear();
        (g, 0, 1)
    }

    fn play(g: &mut Game, bot: &mut Bot, me: usize, seconds: f32) {
        let world = floor();
        let nav = NavGraph::build(
            &world,
            &[
                Vec3::ZERO,
                Vec3::new(4.0, 0.0, 0.0),
                Vec3::new(8.0, 0.0, 0.0),
            ],
        );
        for _ in 0..(seconds / crate::game::TICK) as usize {
            let cmd = bot.think(g, &world, &nav, me);
            g.step(&world, &[cmd]);
        }
    }

    #[test]
    fn bots_fetch_power_weapons_and_dual_wield() {
        let (mut g, smg, rockets) = armory();
        let me = g.add_player();
        g.players[me].body.position = Vec3::ZERO;
        g.players[me].weapons.truncate(1);
        g.players[me].weapons[0].weapon = smg;
        g.players[me].current = 0;
        // A second SMG at the bot's feet goes in the left hand.
        g.dropped.push(crate::game::DroppedWeapon {
            weapon: smg,
            state: crate::WeaponState::new(&g.weapons[smg]),
            position: Vec3::new(0.0, 0.0, 0.1),
            yaw: 0.0,
            ttl: 60.0,
        });
        let mut bot = Bot::new(3);
        play(&mut g, &mut bot, me, 1.0);
        assert_eq!(g.players[me].left.as_ref().map(|h| h.weapon), Some(smg));
        // A rocket launcher a few steps away is worth walking to.
        g.dropped.push(crate::game::DroppedWeapon {
            weapon: rockets,
            state: crate::WeaponState::new(&g.weapons[rockets]),
            position: Vec3::new(4.0, 0.0, 0.1),
            yaw: 0.0,
            ttl: 60.0,
        });
        play(&mut g, &mut bot, me, 4.0);
        let held: Vec<usize> = g.players[me].weapons.iter().map(|h| h.weapon).collect();
        assert!(held.contains(&rockets), "{held:?}");
    }

    #[test]
    fn bots_switch_off_an_empty_gun() {
        let (mut g, smg, rockets) = armory();
        let me = g.add_player();
        let them = g.add_player();
        g.players[me].body.position = Vec3::ZERO;
        g.players[them].body.position = Vec3::new(6.0, 0.0, 0.0);
        let state = |w: usize| crate::WeaponState::new(&g.weapons[w]);
        let mut empty = state(rockets);
        empty.loaded = 0;
        empty.reserve = 0;
        let full = state(smg);
        let p = &mut g.players[me];
        p.weapons = vec![
            HeldWeapon {
                weapon: rockets,
                state: empty,
            },
            HeldWeapon {
                weapon: smg,
                state: full,
            },
        ];
        p.current = 0;
        let mut bot = Bot::new(5);
        let world = floor();
        let nav = NavGraph::build(&world, &[Vec3::ZERO]);
        let mut switched = false;
        for _ in 0..60 {
            let cmd = bot.think(&g, &world, &nav, me);
            switched |= cmd.switch_weapon;
            g.step(&world, &[cmd, Command::default()]);
        }
        assert!(switched);
        assert_eq!(g.players[me].held().map(|h| h.weapon), Some(smg));
    }
}
