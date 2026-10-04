//! Power-ups lying on the map: the overshield, which charges the shields to
//! three times their strength and then drains back to normal, and active
//! camouflage, which all but hides the player until firing, getting hurt or
//! the time running out gives them away. Also ammo packs for power weapons.

use super::{Game, Rules, Spartan};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Powerup {
    Overshield,
    Camouflage,
}

/// An overshield adds this many times the normal shield.
const OVERSHIELD_LAYERS: f32 = 2.0;
/// Seconds for a fresh overshield to charge.
const OVERSHIELD_CHARGE_TIME: f32 = 2.0;
/// How visible a still, camouflaged player is (0 unseen, 1 plain sight).
const CAMO_VISIBLE: f32 = 0.06;
/// Running adds up to this much.
const CAMO_MOVING: f32 = 0.12;
const RUN_SPEED: f32 = 2.25;
/// Camouflage fades out over its last seconds.
const CAMO_FADE: f32 = 2.0;
/// How far firing and getting hurt give a camouflaged player away, and how
/// fast that wears off (per second).
pub(super) const FIRE_REVEAL: f32 = 0.7;
pub(super) const HURT_REVEAL: f32 = 0.5;
const REVEAL_FADE: f32 = 0.8;

/// One tick of a player's power-ups.
pub(super) fn step(p: &mut Spartan, rules: &Rules, dt: f32) {
    if p.overshield_charge > 0.0 {
        let rate = rules.shield * OVERSHIELD_LAYERS / OVERSHIELD_CHARGE_TIME;
        let add = (rate * dt).min(p.overshield_charge);
        p.shield += add;
        p.overshield_charge -= add;
    } else if p.shield > rules.shield {
        let drain = rules.shield * OVERSHIELD_LAYERS / rules.overshield_time.max(1.0);
        p.shield = (p.shield - drain * dt).max(rules.shield);
    }
    p.camo = (p.camo - dt).max(0.0);
    p.reveal = (p.reveal - REVEAL_FADE * dt).max(0.0);
}

impl Spartan {
    /// How visible the player is: 1 normally, little more than a shimmer
    /// with active camouflage.
    pub fn visibility(&self) -> f32 {
        if self.camo <= 0.0 {
            return 1.0;
        }
        let ending = 1.0 - self.camo / CAMO_FADE;
        let speed = self.body.velocity.truncate().length() / RUN_SPEED;
        let seen = CAMO_VISIBLE + CAMO_MOVING * speed.min(1.0) + self.reveal;
        seen.max(ending).clamp(0.0, 1.0)
    }

    /// Overshield on top of the normal shields, as a share of the most an
    /// overshield gives (0-1), counting what is still charging.
    pub fn overshield(&self, rules: &Rules) -> f32 {
        if rules.shield <= 0.0 {
            return 0.0;
        }
        let over = self.shield + self.overshield_charge - rules.shield;
        (over / (rules.shield * OVERSHIELD_LAYERS)).clamp(0.0, 1.0)
    }
}

impl Game {
    /// Walking over a power-up takes it unless the player already has as
    /// much as it gives.
    pub fn give_powerup(&mut self, i: usize, kind: Powerup) -> bool {
        let rules = &self.rules;
        let p = &mut self.players[i];
        match kind {
            Powerup::Overshield => {
                let full = rules.shield * (1.0 + OVERSHIELD_LAYERS);
                let room = full - p.shield - p.overshield_charge;
                if rules.shield <= 0.0 || room < rules.shield * 0.25 {
                    return false;
                }
                p.overshield_charge += room;
                true
            }
            Powerup::Camouflage => {
                if p.camo > rules.camo_time - 5.0 {
                    return false;
                }
                p.camo = rules.camo_time;
                true
            }
        }
    }

    /// An ammo pack tops up the weapon it is for, if the player carries it.
    pub(super) fn take_ammo(&mut self, i: usize, weapon: usize, rounds: u32) -> bool {
        let Some(def) = self.weapons.get(weapon) else {
            return false;
        };
        let p = &mut self.players[i];
        let mut held = p.weapons.iter_mut().chain(p.left.as_mut());
        let Some(h) = held.find(|h| h.weapon == weapon) else {
            return false;
        };
        let room = def.maximum_rounds.saturating_sub(h.state.reserve);
        let give = if rounds == 0 { room } else { rounds.min(room) };
        h.state.reserve += give;
        give > 0
    }

    /// Firing or getting hurt shows where a camouflaged player is.
    pub(super) fn reveal(&mut self, i: usize, amount: f32) {
        let p = &mut self.players[i];
        p.reveal = p.reveal.max(amount);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::tests::{floor, game};
    use crate::game::{Command, ItemKind, ItemSpawn};

    fn on_item(kind: ItemKind) -> Game {
        let mut g = game();
        g.add_player();
        let at = g.players[0].body.position;
        g.item_spawns = vec![ItemSpawn {
            kind,
            position: at,
            respawn: 1000.0,
        }];
        g.item_timers = vec![0.0];
        g
    }

    fn run(g: &mut Game, seconds: f32) {
        let world = floor();
        for _ in 0..(seconds * 60.0) as usize {
            let commands = vec![Command::default(); g.players.len()];
            g.step(&world, &commands);
        }
    }

    #[test]
    fn an_overshield_charges_then_drains_back() {
        let mut g = on_item(ItemKind::Powerup(Powerup::Overshield));
        let normal = g.rules.shield;
        run(&mut g, 2.1);
        let p = &g.players[0];
        assert!((p.shield - normal * 3.0).abs() < 1.0, "{}", p.shield);
        assert!(g.item_timers[0] > 0.0);
        assert!(p.overshield(&g.rules) > 0.9);
        // It soaks up what would have killed.
        g.damage(0, None, normal * 2.0, false);
        assert!(g.players[0].alive);
        assert_eq!(g.players[0].health, g.rules.health);
        // And wears off.
        let drain = g.rules.overshield_time;
        run(&mut g, drain + 1.0);
        assert!((g.players[0].shield - normal).abs() < 1e-3);
        assert_eq!(g.players[0].overshield(&g.rules), 0.0);
    }

    #[test]
    fn camouflage_hides_until_firing_gives_it_away() {
        let mut g = on_item(ItemKind::Powerup(Powerup::Camouflage));
        assert_eq!(g.players[0].visibility(), 1.0);
        run(&mut g, 1.0);
        let hidden = g.players[0].visibility();
        assert!(hidden < 0.1, "{hidden}");
        g.reveal(0, FIRE_REVEAL);
        assert!(g.players[0].visibility() > 0.7);
        run(&mut g, 1.5);
        assert!(g.players[0].visibility() < 0.1);
        // Gone with time, and with death.
        let lasts = g.rules.camo_time;
        run(&mut g, lasts);
        assert_eq!(g.players[0].visibility(), 1.0);
        g.players[0].camo = 10.0;
        g.damage(0, None, 1000.0, false);
        assert_eq!(g.players[0].camo, 0.0);
    }

    #[test]
    fn ammo_packs_fill_only_the_weapon_they_are_for() {
        let mut g = on_item(ItemKind::Ammo {
            weapon: 0,
            rounds: 2,
        });
        g.players[0].weapons.clear();
        run(&mut g, 0.1);
        assert_eq!(g.item_timers[0], 0.0, "no weapon, nothing taken");
        let mut state = crate::weapon::WeaponState::new(&g.weapons[0]);
        state.reserve = 0;
        g.players[0]
            .weapons
            .push(crate::game::HeldWeapon { weapon: 0, state });
        run(&mut g, 0.1);
        assert!(g.item_timers[0] > 0.0);
        assert_eq!(g.players[0].weapons[0].state.reserve, 2);
    }
}
