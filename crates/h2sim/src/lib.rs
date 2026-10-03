//! Halo 2 game simulation, independent of rendering.

pub mod collision;
pub mod player;

pub use collision::World;
pub use player::{Input, Player};
