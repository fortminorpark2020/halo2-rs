//! h2relay: carries MCC Halo 2 engine packets between the launchers in one
//! match, over UDP, through a relay server that runs inside h2live (or on
//! its own as the `h2relay` program, for tests on one PC).
//!
//! # Why
//!
//! halo2.dll hands every network packet to its host through three host
//! functions: an unreliable send and a reliable send, each taking a
//! destination network id (an opaque u64), the bytes and a port number,
//! and a receive that never waits and returns 0 when nothing has come.
//! The launcher (`h2launch`) implements them with a [`RelayClient`]. Halo
//! 2's multiplayer simulation is synchronous (the host gathers everyone's
//! actions, and everyone waits for the slowest), so the packets go over
//! UDP: over TCP, one lost packet would hold up every packet behind it,
//! and so every player. Going through a relay means no player has to open
//! a port on their router; only the server does (UDP 47050, beside
//! h2live's TCP 47050).
//!
//! # Pieces
//!
//! - [`frame`]: the wire format. A fixed 40-byte header (magic `H2RL`,
//!   [`RELAY_PROTOCOL`], kind, flags, room, src id, dst id, port, seq) and
//!   on data frames the engine's packet exactly as it was handed over, at
//!   most [`MAX_PAYLOAD`] bytes. [`RELAY_PROTOCOL`] is this crate's own
//!   version, separate from h2net's `PROTOCOL`: bump it on any change to
//!   the format or to what the frames mean.
//! - The `keys` module: the server's address cookies (a hello only counts
//!   once it brings back a cookie sent to its address, so no one can sign
//!   up someone else's address) and the member keys for rooms h2live
//!   issued ([`MemberKey`]).
//! - [`RelayClient`] (the [`client`] module): one per match in the
//!   launcher. Sends go straight to a non-blocking socket, a background
//!   thread receives and keeps the timers, and reliable sends are acked,
//!   resent and handed out in order, each once. Its module docs have the
//!   details.
//! - [`RelayServer`] (the [`server`] module): rooms keyed by the match
//!   token, members keyed by network id and checked against the address
//!   they said hello from, forwarding by destination id (or to the whole
//!   room), timeouts, and limits on members, rooms and packet rates. Its
//!   module docs have the details, including the two kinds of admission:
//!   rooms h2live issued (with member keys), and open rooms for tests.
//!
//! The server never looks inside a payload, and the client never
//! interprets the network ids or ports it carries: both belong to the
//! engine.

pub mod client;
pub mod frame;
mod keys;
mod log;
mod rng;
pub mod server;
mod sockets;

pub use client::{
    ClientConfig, ClientStats, Failure, Lag, PeerStats, RelayClient, SendError, State,
};
pub use frame::{
    decode, Frame, FrameError, Kind, Refusal, BROADCAST, MAX_PAYLOAD, MTU_PAYLOAD, RELAY_PROTOCOL,
};
pub use keys::MemberKey;
pub use log::{background_logger, stdout_logger, Logger};
pub use server::{Admission, RelayHandle, RelayServer, RelayThread, ServerConfig, ServerStats};

/// The relay's UDP port unless told otherwise: h2live's TCP port's number.
pub const DEFAULT_PORT: u16 = 47050;
