//! Halo 2 game simulation, independent of rendering.

pub mod bot;
pub mod collision;
pub mod game;
pub mod nav;
pub mod player;
pub mod testing;
pub mod weapon;

pub use bot::Bot;
pub use collision::World;
pub use game::{Command, Game, ItemKind, ItemSpawn, Rules};
pub use nav::NavGraph;
pub use player::{Input, Player};
pub use weapon::{Shot, WeaponDef, WeaponInput, WeaponState};
