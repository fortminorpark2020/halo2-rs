//! Online play, like Halo 2 on Xbox Live: players sign in to a server,
//! form parties, search matchmaking playlists and earn levels from 1 to 50
//! in the ranked ones. The games themselves still run on a player's PC.
//!
//! The server (`server`) keeps accounts on disk (`store`) and on the stat
//! cards it gives players (`card`), and launcher players' friends lists
//! (`friends`), and PCs sign in to it through `client`.
//! Nothing here reads a clock (but for log lines) or listens on the
//! network: the caller passes the time and hands over connections, so tests
//! run in an instant. The `h2live` program (`main.rs`) does that for a real
//! server. The rules are here too: how games change levels, the
//! playlists, and how matchmaking puts parties together into matches.

pub mod card;
pub mod client;
pub mod friends;
pub mod levels;
pub mod matchmaker;
pub mod playlists;
pub mod server;
pub mod store;
