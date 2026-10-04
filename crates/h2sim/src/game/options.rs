//! Halo 2's game variant settings that change the map and what every PC
//! shows: the weapons lying on the map, vehicles, shields and the motion
//! sensor. (What players start with lives in `Rules`; only the host needs
//! it.)

use super::{Game, ItemKind, Malformed, Reader, Writer};

/// What the map's weapon spots hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapWeapons {
    #[default]
    MapDefault,
    None,
    /// Every weapon spot holds this weapon (index into the weapon list).
    Only(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub map_weapons: MapWeapons,
    pub vehicles: bool,
    pub shields: bool,
    /// The motion sensor.
    pub radar: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            map_weapons: MapWeapons::MapDefault,
            vehicles: true,
            shields: true,
            radar: true,
        }
    }
}

/// A weapon spot left empty for the whole game.
const NEVER: f32 = 1e9;

impl Options {
    pub fn write(&self, w: &mut Writer) {
        let (kind, weapon) = match self.map_weapons {
            MapWeapons::MapDefault => (0, 0),
            MapWeapons::None => (1, 0),
            MapWeapons::Only(k) => (2, k),
        };
        w.u8(kind);
        w.u16(weapon as u16);
        w.u8(self.vehicles as u8 | (self.shields as u8) << 1 | (self.radar as u8) << 2);
    }

    /// `weapons`: how many weapons the game has.
    pub fn read(r: &mut Reader, weapons: usize) -> Result<Options, Malformed> {
        let kind = r.u8()?;
        let weapon = r.u16()? as usize;
        let map_weapons = match kind {
            0 => MapWeapons::MapDefault,
            1 => MapWeapons::None,
            2 if weapon < weapons => MapWeapons::Only(weapon),
            _ => return Err(Malformed),
        };
        let flags = r.u8()?;
        Ok(Options {
            map_weapons,
            vehicles: flags & 1 != 0,
            shields: flags & 2 != 0,
            radar: flags & 4 != 0,
        })
    }
}

impl Game {
    /// Set up the map for `options`. Call once on a fresh game (the map
    /// as it comes, before anyone plays).
    pub fn apply_options(&mut self, options: Options) {
        self.rules.options = options;
        for (spawn, timer) in self.item_spawns.iter_mut().zip(&mut self.item_timers) {
            if let ItemKind::Weapon(w) = &mut spawn.kind {
                match options.map_weapons {
                    MapWeapons::MapDefault => {}
                    MapWeapons::None => *timer = NEVER,
                    MapWeapons::Only(k) => *w = k,
                }
            }
        }
        if !options.vehicles {
            self.vehicles.clear();
            self.vehicle_spawns.clear();
        }
        if !options.shields {
            self.rules.shield = 0.0;
            for p in &mut self.players {
                p.shield = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::game;

    fn round_trip(options: Options, weapons: usize) -> Result<Options, Malformed> {
        let mut w = Writer::default();
        options.write(&mut w);
        Options::read(&mut Reader::new(&w.0), weapons)
    }

    #[test]
    fn options_change_the_map_and_survive_the_wire() {
        let mut g = game();
        g.add_player();
        let options = Options {
            map_weapons: MapWeapons::None,
            vehicles: false,
            shields: false,
            radar: false,
        };
        g.apply_options(options);
        assert_eq!(g.rules.shield, 0.0);
        assert!(g.players.iter().all(|p| p.shield == 0.0));
        assert!(g.item_timers[0] > 1e6, "the weapon never appears");
        assert_eq!(round_trip(options, g.weapons.len()), Ok(options));

        let mut g = game();
        g.apply_options(Options {
            map_weapons: MapWeapons::Only(0),
            ..Options::default()
        });
        assert_eq!(g.item_spawns[0].kind, ItemKind::Weapon(0));
        assert_eq!(g.item_timers[0], 0.0);
        let only = |k| Options {
            map_weapons: MapWeapons::Only(k),
            ..Options::default()
        };
        assert_eq!(round_trip(only(1), 2), Ok(only(1)));
        assert!(round_trip(only(2), 2).is_err(), "no such weapon");
    }
}
