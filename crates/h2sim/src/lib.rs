//! Halo 2 game simulation, independent of rendering.

pub mod collision;
pub mod game;
pub mod player;
pub mod weapon;

pub use collision::World;
pub use game::{Command, Game, ItemKind, ItemSpawn, Rules};
pub use player::{Input, Player};
pub use weapon::{Shot, WeaponDef, WeaponInput, WeaponState};
