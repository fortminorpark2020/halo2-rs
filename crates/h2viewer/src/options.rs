//! The lobby's game options: Halo 2's game variant settings (the weapons
//! on the map, what players start with, shields, the motion sensor,
//! vehicles, respawning, friendly fire and the time limit) and some of its
//! built-in variants.

use crate::scene::Scene;
use h2sim::game::{MapWeapons, Options, Rules};

/// The weapons the options offer, by tag name, with Halo 2's names.
pub const WEAPONS: [(&str, &str); 15] = [
    ("battle_rifle", "BATTLE RIFLE"),
    ("smg", "SMG"),
    ("magnum", "MAGNUM"),
    ("shotgun", "SHOTGUN"),
    ("sniper_rifle", "SNIPER RIFLE"),
    ("rocket_launcher", "ROCKET LAUNCHER"),
    ("covenant_carbine", "COVENANT CARBINE"),
    ("beam_rifle", "BEAM RIFLE"),
    ("plasma_rifle", "PLASMA RIFLE"),
    ("brute_plasma_rifle", "BRUTE PLASMA RIFLE"),
    ("plasma_pistol", "PLASMA PISTOL"),
    ("needler", "NEEDLER"),
    ("brute_shot", "BRUTE SHOT"),
    ("flak_cannon", "FUEL ROD GUN"),
    ("energy_blade", "ENERGY SWORD"),
];

/// Respawn times offered, in seconds; the first is Halo 2's usual.
pub const RESPAWN_TIMES: [u32; 6] = [5, 0, 3, 10, 15, 30];

/// Time limits offered, in seconds; 0 plays until someone reaches the score.
pub const TIME_LIMITS: [u32; 7] = [0, 300, 600, 900, 1200, 1500, 1800];

/// What the map's weapon spots hold, or a starting weapon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Choice {
    #[default]
    MapDefault,
    None,
    /// One of `WEAPONS`.
    Weapon(usize),
}

impl Choice {
    fn weapon(name: &str) -> Choice {
        WEAPONS
            .iter()
            .position(|w| w.0 == name)
            .map_or(Choice::None, Choice::Weapon)
    }

    /// The next choice `step` along (map default, none, then the weapons).
    pub fn step(self, step: i32) -> Choice {
        let at = match self {
            Choice::MapDefault => 0,
            Choice::None => 1,
            Choice::Weapon(k) => k + 2,
        };
        match (at as i32 + step).rem_euclid(WEAPONS.len() as i32 + 2) {
            0 => Choice::MapDefault,
            1 => Choice::None,
            k => Choice::Weapon(k as usize - 2),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Choice::MapDefault => "MAP DEFAULT",
            Choice::None => "NONE",
            Choice::Weapon(k) => WEAPONS[k % WEAPONS.len()].1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameOptions {
    pub map_weapons: Choice,
    pub primary: Choice,
    pub secondary: Choice,
    pub grenades: bool,
    pub shields: bool,
    pub radar: bool,
    pub vehicles: bool,
    /// In `RESPAWN_TIMES`.
    pub respawn: usize,
    pub friendly_fire: bool,
    /// Seconds of play before the game ends on time; 0 for none. Any
    /// variant can have one.
    pub time_limit: u32,
}

impl Default for GameOptions {
    fn default() -> GameOptions {
        GameOptions {
            map_weapons: Choice::MapDefault,
            primary: Choice::MapDefault,
            secondary: Choice::MapDefault,
            grenades: true,
            shields: true,
            radar: true,
            vehicles: true,
            respawn: 0,
            friendly_fire: true,
            time_limit: 0,
        }
    }
}

/// Halo 2's built-in variants offered, by name.
pub fn presets() -> [(&'static str, GameOptions); 6] {
    let only = |name: &str, backup: Choice| GameOptions {
        map_weapons: Choice::weapon(name),
        primary: Choice::weapon(name),
        secondary: backup,
        vehicles: false,
        ..GameOptions::default()
    };
    [
        ("DEFAULT", GameOptions::default()),
        (
            "SWAT",
            GameOptions {
                map_weapons: Choice::None,
                primary: Choice::weapon("battle_rifle"),
                secondary: Choice::weapon("magnum"),
                grenades: false,
                shields: false,
                radar: false,
                vehicles: false,
                ..GameOptions::default()
            },
        ),
        ("ROCKETS", only("rocket_launcher", Choice::None)),
        ("SNIPERS", only("sniper_rifle", Choice::weapon("magnum"))),
        ("SWORDS", only("energy_blade", Choice::None)),
        ("SHOTGUNS", only("shotgun", Choice::None)),
    ]
}

impl GameOptions {
    /// The built-in variant these options are, if any (whatever the time
    /// limit).
    pub fn preset(&self) -> Option<usize> {
        let untimed = GameOptions {
            time_limit: 0,
            ..self.clone()
        };
        presets().iter().position(|(_, o)| *o == untimed)
    }

    pub fn respawn_seconds(&self) -> u32 {
        RESPAWN_TIMES[self.respawn % RESPAWN_TIMES.len()]
    }

    /// `base` (the map's rules) changed by these options.
    pub fn rules(&self, scene: &Scene, base: Rules) -> Rules {
        let index = |k: usize| {
            scene
                .weapons
                .iter()
                .position(|w| w.def.name == WEAPONS[k].0)
        };
        let start = |choice: Choice, default: Option<usize>| match choice {
            Choice::MapDefault => default,
            Choice::None => None,
            Choice::Weapon(k) => index(k),
        };
        let defaults = &base.starting_weapons;
        let starting_weapons = [
            start(self.primary, defaults.first().copied()),
            start(self.secondary, defaults.get(1).copied()),
        ]
        .into_iter()
        .flatten()
        .collect();
        let grenades = |n: u8| if self.grenades { n } else { 0 };
        Rules {
            starting_weapons,
            starting_frags: grenades(base.starting_frags),
            starting_plasmas: grenades(base.starting_plasmas),
            respawn_time: self.respawn_seconds() as f32,
            friendly_fire: self.friendly_fire,
            time_limit: self.time_limit,
            ..base
        }
    }

    /// The options every PC in the game needs.
    pub fn shared(&self, scene: &Scene) -> Options {
        let map_weapons = match self.map_weapons {
            Choice::MapDefault => MapWeapons::MapDefault,
            Choice::None => MapWeapons::None,
            Choice::Weapon(k) => scene
                .weapons
                .iter()
                .position(|w| w.def.name == WEAPONS[k].0)
                .map_or(MapWeapons::MapDefault, MapWeapons::Only),
        };
        Options {
            map_weapons,
            vehicles: self.vehicles,
            shields: self.shields,
            radar: self.radar,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_step_through_every_weapon_and_wrap() {
        assert_eq!(Choice::MapDefault.step(1), Choice::None);
        assert_eq!(Choice::None.step(1), Choice::Weapon(0));
        assert_eq!(
            Choice::MapDefault.step(-1),
            Choice::Weapon(WEAPONS.len() - 1)
        );
        assert_eq!(
            Choice::Weapon(WEAPONS.len() - 1).step(1),
            Choice::MapDefault
        );
        assert_eq!(Choice::weapon("energy_blade").label(), "ENERGY SWORD");
    }

    #[test]
    fn presets_are_recognised() {
        let swat = &presets()[1].1;
        assert_eq!(swat.preset(), Some(1));
        assert!(!swat.shields && !swat.radar);
        assert_eq!(GameOptions::default().preset(), Some(0));
        let custom = GameOptions {
            radar: false,
            ..GameOptions::default()
        };
        assert_eq!(custom.preset(), None);
        // A time limit doesn't make a variant custom.
        let timed = GameOptions {
            time_limit: 600,
            ..swat.clone()
        };
        assert_eq!(timed.preset(), Some(1));
    }
}
