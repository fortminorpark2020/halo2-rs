//! LAN games, like Halo 2's system link: one PC hosts and runs the game,
//! others join over the local network. Joined PCs send their players'
//! controls every tick; the host sends back the state of the game and what
//! happened in it after every tick. Games announce themselves on the
//! network so other PCs can list and join them without typing addresses.
//!
//! PCs stay together between games: they wait in the host's lobby, and
//! when the host starts a game they load its map and join it.
//!
//! Online games are the same, over connections the online service hands
//! the host and the joining PC (`Host::add_connection`, `Client::over`),
//! with the game sent 30 times a second rather than after every tick, and
//! mostly as how it changed since the last. Those connections are
//! WebSockets: PCs `dial` the service, and it `accept`s them. The messages
//! between a PC and the online service itself are in `live`.

mod client;
mod conn;
mod delta;
mod discovery;
mod host;
pub mod live;
mod lobby;
mod predict;
mod ws;

use std::time::{Duration, Instant};

pub use client::{Client, ClientEvent};
pub use conn::{Connection, Lag};
pub use discovery::{local_ip, Browser, LanGame};
pub use host::{Host, HostEvent, Verified};
pub use lobby::{Lobby, LobbyPlayer};
pub use ws::{accept, dial, reply, Request};

/// Bumped whenever the messages change; PCs on different versions can't play.
pub const PROTOCOL: u32 = 27;
/// A joining player's team when the host is to choose it.
pub const ANY_TEAM: u8 = u8::MAX;
/// How a host turns away a PC that isn't on its game's map (the map's name
/// follows).
pub const HOST_IS_PLAYING: &str = "HOST IS PLAYING ";
/// The host listens here (or the next free port above it).
pub const GAME_PORT: u16 = 47040;
/// Hosts announce their games to this port.
pub const BEACON_PORT: u16 = 47039;
/// A PC not heard from for this long is gone. Each end says it's still
/// there when it has sent nothing for a tenth of this.
pub const TIMEOUT: Duration = Duration::from_secs(10);

const MAGIC: u32 = u32::from_le_bytes(*b"H2RS");

/// Message kinds.
mod kind {
    // Joined PC to host.
    pub const HELLO: u8 = 1;
    /// Controls for a tick, numbered, for each player there.
    pub const INPUT: u8 = 2;
    pub const ADD_LOCAL: u8 = 3;
    pub const REMOVE_LOCAL: u8 = 4;
    /// Still here (after a while with nothing else to say).
    pub const ALIVE: u8 = 5;
    /// The number of the newest snapshot taken on, said as it changes:
    /// online, the host holds the game back from a PC too far behind what
    /// it was sent.
    pub const GOT: u8 = 6;
    // Host to joined PC.
    pub const WELCOME: u8 = 101;
    pub const REFUSED: u8 = 102;
    pub const ADDED: u8 = 103;
    /// The game, numbered, with the number of the controls last run for
    /// each player on a joined PC, then what happened since the last.
    pub const SNAPSHOT: u8 = 104;
    /// The host is in its lobby (sent on joining and when it changes).
    pub const LOBBY: u8 = 105;
    /// The host started a game on this map: load it and say hello again.
    pub const START: u8 = 106;
    /// The host is still here.
    pub const HOST_ALIVE: u8 = 107;
    /// A snapshot as how it differs from the one before: its number, its
    /// length, and the change (see `delta`).
    pub const SNAPSHOT_DELTA: u8 = 108;
}

/// What this PC is called on the network.
pub fn computer_name() -> String {
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    name.unwrap_or_else(|| "PC".into())
}

/// The gamertag of the person at this PC: H2_NAME if set, otherwise their
/// Windows user name (or this PC's name).
pub fn player_name() -> String {
    ["H2_NAME", "USERNAME", "USER"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .chain([computer_name()])
        .map(|n| h2sim::game::clean_name(&n))
        .find(|n| !n.is_empty())
        .unwrap_or_else(|| "PLAYER".into())
}

/// A number that tells this running game apart from others.
pub fn session_id() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    t ^ ((std::process::id() as u64) << 32) ^ 0x9E37_79B9_7F4A_7C15
}

/// Snapshot number `a` comes after `b` (in numbers that wrap around).
fn after(a: u32, b: u32) -> bool {
    a.wrapping_sub(b) as i32 > 0
}

/// Paces something to once every `every` seconds on average, however the
/// frames that ask fall.
struct Pace {
    every: f32,
    /// Time since it last went (up to two periods' worth), as of the last
    /// frame.
    owed: f32,
    frame: Instant,
}

impl Pace {
    fn new(every: f32) -> Pace {
        Pace {
            every,
            owed: 0.0,
            frame: Instant::now(),
        }
    }

    /// Count the time since the last frame. True if it's time to go.
    fn due(&mut self) -> bool {
        let now = Instant::now();
        let since = (now - self.frame).as_secs_f32();
        self.owed = (self.owed + since).min(2.0 * self.every);
        self.frame = now;
        self.owed >= self.every
    }

    /// It went: the next is a period away.
    fn went(&mut self) {
        self.owed = (self.owed - self.every).max(0.0);
    }
}

#[cfg(test)]
mod tests;
