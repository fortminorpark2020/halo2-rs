//! Halo 2 game simulation, independent of rendering.

pub mod collision;
pub mod player;
pub mod weapon;

pub use collision::World;
pub use player::{Input, Player};
pub use weapon::{Shot, WeaponDef, WeaponInput, WeaponState};
