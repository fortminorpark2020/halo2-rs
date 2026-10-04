//! LAN games, like Halo 2's system link: one PC hosts and runs the game,
//! others join over the local network. Joined PCs send their players'
//! controls every frame; the host sends back the state of the game and what
//! happened in it after every tick. Games announce themselves on the network
//! so other PCs can list and join them without typing addresses.

mod client;
mod conn;
mod discovery;
mod host;

pub use client::{Client, ClientEvent};
pub use discovery::{local_ip, Browser, LanGame};
pub use host::{Host, HostEvent};

/// Bumped whenever the messages change; PCs on different versions can't play.
pub const PROTOCOL: u32 = 14;
/// The host listens here (or the next free port above it).
pub const GAME_PORT: u16 = 47040;
/// Hosts announce their games to this port.
pub const BEACON_PORT: u16 = 47039;

const MAGIC: u32 = u32::from_le_bytes(*b"H2RS");

/// Message kinds.
mod kind {
    // Joined PC to host.
    pub const HELLO: u8 = 1;
    pub const INPUT: u8 = 2;
    pub const ADD_LOCAL: u8 = 3;
    pub const REMOVE_LOCAL: u8 = 4;
    // Host to joined PC.
    pub const WELCOME: u8 = 101;
    pub const REFUSED: u8 = 102;
    pub const ADDED: u8 = 103;
    pub const SNAPSHOT: u8 = 104;
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

#[cfg(test)]
mod tests;
