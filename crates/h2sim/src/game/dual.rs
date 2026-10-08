//! Dual wielding: a second one-handed weapon in the left hand, fired with
//! the left trigger. Holding the switch button by a weapon on the ground
//! takes it into the left hand; tapping it drops the left weapon and
//! switches to the one on the back.

use super::{
    ready_time, DroppedWeapon, Event, Game, HeldWeapon, ItemKind, DROPPED_WEAPON_LIFETIME,
    PICKUP_RADIUS, SWAP_HOLD,
};
use crate::weapon::WeaponState;
use glam::{Vec2, Vec3};

/// Where a weapon that could go in the left hand lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnTheGround {
    Item(usize),
    Dropped(usize),
}

impl Game {
    /// Whether weapon kind `w` can be held in either hand.
    pub fn one_handed(&self, w: usize) -> bool {
        self.weapons.get(w).is_some_and(|d| d.dual.is_some())
    }

    /// Whether player `i` holds a weapon in each hand.
    pub fn dual_wielding(&self, i: usize) -> bool {
        self.players.get(i).is_some_and(|p| p.left.is_some())
    }

    fn left_hand_pickup(&self, i: usize) -> Option<(OnTheGround, usize)> {
        let p = self.players.get(i)?;
        let right = p.weapons.get(p.current)?;
        if !p.alive || p.left.is_some() || p.objective.is_some() || !self.one_handed(right.weapon) {
            return None;
        }
        let feet = p.body.position;
        let near = |q: Vec3| {
            Vec2::new(q.x - feet.x, q.y - feet.y).length() < PICKUP_RADIUS
                && (q.z - feet.z).abs() < 0.8
        };
        let item = self
            .item_spawns
            .iter()
            .zip(&self.item_timers)
            .enumerate()
            .find_map(|(s, (spawn, t))| match spawn.kind {
                ItemKind::Weapon(w) if *t <= 0.0 && near(spawn.position) && self.one_handed(w) => {
                    Some((OnTheGround::Item(s), w))
                }
                _ => None,
            });
        item.or_else(|| {
            self.dropped
                .iter()
                .enumerate()
                .find(|(_, d)| near(d.position) && self.one_handed(d.weapon))
                .map(|(k, d)| (OnTheGround::Dropped(k), d.weapon))
        })
    }

    /// The weapon on the ground a player could take into their left hand
    /// (switch button prompt).
    pub fn dual_prompt(&self, i: usize) -> Option<usize> {
        self.left_hand_pickup(i).map(|(_, w)| w)
    }

    fn take_left(&mut self, i: usize) {
        let Some((at, w)) = self.left_hand_pickup(i) else {
            return;
        };
        let state = match at {
            OnTheGround::Item(s) => {
                self.item_timers[s] = self.item_spawns[s].respawn.max(1.0);
                WeaponState::new(&self.weapons[w])
            }
            OnTheGround::Dropped(k) => self.dropped.remove(k).state,
        };
        let p = &mut self.players[i];
        if let Some(h) = p.weapons.get_mut(p.current) {
            h.state.zoom = 0;
            h.state.let_go();
        }
        p.left = Some(HeldWeapon { weapon: w, state });
        p.readying = ready_time(&self.weapons, p);
        self.events.push(Event::PickedUp {
            player: i,
            kind: ItemKind::Weapon(w),
        });
        self.events.push(Event::Switched { player: i });
    }

    /// Put the left hand's weapon down where the player stands.
    pub(super) fn drop_left(&mut self, i: usize) {
        let p = &mut self.players[i];
        let Some(mut h) = p.left.take() else {
            return;
        };
        h.state.put_away();
        let (position, yaw) = (p.body.position + Vec3::Z * 0.1, p.yaw);
        self.dropped.push(DroppedWeapon {
            weapon: h.weapon,
            state: h.state,
            position,
            yaw,
            ttl: DROPPED_WEAPON_LIFETIME,
        });
    }

    /// The switch button: a tap switches weapons (dropping the left one
    /// first, or the flag); held by a one-handed weapon it takes that
    /// weapon into the left hand.
    pub(super) fn press_switch(&mut self, i: usize, down: bool, was_down: bool, dt: f32) {
        let could_dual = self.left_hand_pickup(i).is_some();
        let p = &mut self.players[i];
        if down && !was_down {
            p.switch_held = 0.0;
            if !could_dual {
                p.switch_held = f32::MIN;
                self.tap_switch(i);
            }
        } else if down {
            if p.switch_held >= 0.0 {
                p.switch_held += dt;
                if could_dual && p.switch_held >= SWAP_HOLD {
                    p.switch_held = f32::MIN;
                    self.take_left(i);
                }
            }
        } else if was_down && p.switch_held >= 0.0 {
            // Let go before the hold finished: an ordinary switch.
            p.switch_held = f32::MIN;
            self.tap_switch(i);
        }
    }

    fn tap_switch(&mut self, i: usize) {
        if self.players[i].objective.is_some() {
            self.drop_flag(i);
        } else if self.players[i].left.is_some() {
            self.drop_left(i);
            if self.players[i].weapons.len() > 1 {
                self.switch_weapon(i);
            } else {
                self.events.push(Event::Switched { player: i });
            }
        } else {
            self.switch_weapon(i);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::game::tests::game;
    use crate::game::{Command, DroppedWeapon, Event};
    use crate::testing::floor;
    use crate::weapon::{DualWield, WeaponDef, WeaponState};

    const DUAL: DualWield = DualWield {
        minimum_error: 0.05,
        error_angle: (0.05, 0.1),
        damage_scale: 0.5,
    };

    /// The one-handed gun; the test game's others are two-handed.
    const SMG: usize = 2;

    /// Player 0 holding a one-handed gun over a second one.
    fn dual_game() -> crate::Game {
        let mut g = game();
        let smg = WeaponDef {
            name: "smg".into(),
            dual: Some(DUAL),
            ..g.weapons[0].clone()
        };
        g.weapons.push(smg);
        assert_eq!(g.weapons.len(), SMG + 1);
        g.rules.starting_weapons = vec![SMG];
        g.add_player();
        // Full of ammo, so walking over the second gun leaves it loaded.
        g.players[0].weapons[0].state.reserve = g.weapons[SMG].maximum_rounds;
        g.dropped.push(DroppedWeapon {
            weapon: SMG,
            state: WeaponState::new(&g.weapons[SMG]),
            position: g.players[0].body.position,
            yaw: 0.0,
            ttl: 30.0,
        });
        g
    }

    fn hold_switch(g: &mut crate::Game, ticks: usize) {
        let world = floor();
        let cmd = Command {
            switch_weapon: true,
            ..Command::default()
        };
        for _ in 0..ticks {
            g.step(&world, &[cmd]);
        }
        g.step(&world, &[Command::default()]);
    }

    #[test]
    fn holding_switch_takes_a_second_gun() {
        let mut g = dual_game();
        assert_eq!(g.dual_prompt(0), Some(SMG));
        // A tap is just a switch (with one weapon, nothing happens).
        hold_switch(&mut g, 2);
        assert!(!g.dual_wielding(0));
        hold_switch(&mut g, 30);
        assert!(g.dual_wielding(0));
        assert!(g.dropped.is_empty());
        assert_eq!(g.dual_prompt(0), None);
        // A tap now drops the left gun.
        hold_switch(&mut g, 2);
        assert!(!g.dual_wielding(0));
        assert_eq!(g.dropped.len(), 1);
    }

    #[test]
    fn both_guns_fire_and_drop_on_death() {
        let mut g = dual_game();
        hold_switch(&mut g, 30);
        let world = floor();
        let mut hands = (false, false);
        for k in 0..60 {
            // Alternate triggers: right with fire, left with the grenade key.
            let cmd = Command {
                fire: k % 2 == 0,
                throw_grenade: k % 2 == 1,
                ..Command::default()
            };
            g.step(&world, &[cmd]);
            for e in &g.events {
                if let Event::Shot { left, .. } = e {
                    if *left {
                        hands.1 = true;
                    } else {
                        hands.0 = true;
                    }
                }
                assert!(!matches!(e, Event::Thrown { .. }), "no grenades");
            }
        }
        assert_eq!(hands, (true, true));
        let before = g.dropped.len();
        g.damage(0, None, 10_000.0, false);
        assert_eq!(g.dropped.len(), before + 2);
    }

    #[test]
    fn ammo_taken_from_a_one_handed_gun_leaves_it_behind() {
        let mut g = dual_game();
        g.players[0].weapons[0].state.reserve = 0;
        g.step(&floor(), &[Command::default()]);
        assert!(g.players[0].weapons[0].state.reserve > 0);
        assert_eq!(g.dropped.len(), 1, "still there to dual wield");
        assert_eq!(g.dual_prompt(0), Some(SMG));
        // Two-handed guns are used up.
        let mut g = game();
        g.add_player();
        g.players[0].weapons[0].state.reserve = 0;
        g.dropped.push(DroppedWeapon {
            weapon: 0,
            state: WeaponState::new(&g.weapons[0]),
            position: g.players[0].body.position,
            yaw: 0.0,
            ttl: 30.0,
        });
        g.step(&floor(), &[Command::default()]);
        assert!(g.dropped.is_empty());
    }

    #[test]
    fn two_handed_guns_stay_two_handed() {
        let mut g = dual_game();
        g.players[0].weapons[0].weapon = 0;
        assert_eq!(g.dual_prompt(0), None);
        let mut g = dual_game();
        g.dropped[0].weapon = 0;
        assert_eq!(g.dual_prompt(0), None);
    }

    #[test]
    fn dual_wielded_guns_spread_more_and_hit_softer() {
        let g = dual_game();
        let one = &g.weapons[SMG];
        let two = one.dual_wielded();
        assert!(two.error_angle.1 >= DUAL.error_angle.1);
        assert_eq!(two.damage, one.damage * 0.5);
    }
}
