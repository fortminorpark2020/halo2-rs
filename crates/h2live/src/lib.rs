//! Online play, like Halo 2 on Xbox Live: players sign in to a server,
//! form parties, search matchmaking playlists and earn levels from 1 to 50
//! in the ranked ones. The games themselves still run on a player's PC.
//!
//! So far this holds the rules, with no networking and no clock of its
//! own: how games change levels, the playlists, and how matchmaking puts
//! parties together into matches.

pub mod levels;
pub mod matchmaker;
pub mod playlists;
