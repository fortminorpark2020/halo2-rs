//! Messages to and from the online service. A signed-in PC keeps a control
//! link to the server: it signs in, forms parties and searches for games
//! over it, and the server tells it who's online, what its party is doing
//! and when a match is ready. Games go over relay legs: connections the
//! server pairs up by a token and then passes h2net's own game messages
//! between, untouched. A host also has a fan-out leg, for what goes to
//! many of the PCs joining it at once: it sends that once, and the server
//! passes a copy on to each.
//!
//! Each message is a kind and a body, as in LAN games. Reading one checks
//! every count and length, so bytes from a broken or hostile peer are an
//! error, never a crash.
//!
//! Two programs sign in: the game (h2viewer), whose players play each
//! other on its own engine, and the launcher (h2launch), whose players
//! play on MCC's Halo 2 engine with the game traffic going through the
//! UDP relay (`h2relay`). The game signs in as it always has: LOGIN starts
//! with `MAGIC` and the game's `PROTOCOL`, and the server takes it only at
//! its own `PROTOCOL` (games on other versions can't play each other).
//! The launcher's LOGIN starts with `MAGIC_V2` and `LIVE_PROTOCOL`, this
//! service's own version, then says which program it is
//! (`LoginClient`); the server takes it whatever the game's `PROTOCOL`
//! is, so a new game version never locks launchers out. The messages the
//! game sends and is sent are the same as ever (so `PROTOCOL` stays as it
//! was); the launcher's own ones (LAUNCHER_MATCH, JOINED and
//! LAUNCHER_RESULT) are versioned by `LIVE_PROTOCOL` and only ever go to
//! and from launchers.

use crate::conn::Connection;
use crate::PROTOCOL;
use h2sim::game::{Look, Malformed, Reader, Writer};
use h2sim::GameType;
use std::io::Read;
use std::path::Path;

/// The game's LOGIN starts with this, then its `PROTOCOL`.
pub const MAGIC: u32 = u32::from_le_bytes(*b"H2LV");
/// A versioned LOGIN (the launcher's) starts with this, then
/// `LIVE_PROTOCOL` and the kind of program (see `login_header`).
pub const MAGIC_V2: u32 = u32::from_le_bytes(*b"H2L2");
/// The online service's own version, separate from the game's `PROTOCOL`:
/// bump it whenever a versioned LOGIN or a launcher's messages change. The
/// server takes launchers at this version only, whatever the game's
/// `PROTOCOL`.
pub const LIVE_PROTOCOL: u32 = 1;
/// The kinds of program a versioned LOGIN can say it is, as a byte.
pub const CLIENT_VIEWER: u8 = 0;
pub const CLIENT_LAUNCHER: u8 = 1;
/// The length of a relay member key (h2relay's `MemberKey`).
pub const MEMBER_KEY_LEN: usize = 16;
/// Control messages are at most this big (kind and body); a PC that sends a
/// bigger one is dropped.
pub const MAX_MESSAGE: usize = 64 << 10;
/// Most people in a party, splitscreen guests included.
pub const MAX_PARTY: usize = 16;
/// Most splitscreen guests on one PC.
pub const MAX_GUESTS: u8 = 3;
/// Each end pings the other this often (seconds), and gives up on it after
/// `TIMEOUT` seconds with nothing heard.
pub const PING_EVERY: f64 = 1.0;
pub const TIMEOUT: f64 = 15.0;
/// The playlist number that searches whichever playlist fits the party and
/// has the most people searching.
pub const QUICKMATCH: u8 = 255;

/// Why the server turned a PC away (REFUSED), as the PC shows it.
pub const UPDATE_YOUR_GAME: &str = "UPDATE YOUR GAME";
/// A launcher written for another `LIVE_PROTOCOL` than the server's.
pub const UPDATE_YOUR_LAUNCHER: &str = "UPDATE YOUR LAUNCHER";
/// Someone else has the gamertag: the PC asks its player for another.
pub const GAMERTAG_TAKEN: &str = "GAMERTAG TAKEN";
pub const BAD_SIGNATURE: &str = "BAD SIGNATURE";
pub const SERVER_FULL: &str = "SERVER FULL";
/// The same address started signing in too often in the last minute.
pub const TOO_MANY_SIGN_INS: &str = "TOO MANY SIGN-INS, WAIT A MINUTE";
/// The same account signed in again from another PC.
pub const SIGNED_IN_ELSEWHERE: &str = "SIGNED IN ON ANOTHER PC";

/// Longest names (gamertags, maps, keys, presets) and text (reasons,
/// notices), in bytes.
pub const MAX_NAME: usize = 64;
const MAX_TEXT: usize = 1024;
/// Longest stat card, in bytes.
pub const MAX_CARD: usize = 8 << 10;
/// Most maps a PC can list.
pub const MAX_MAPS: usize = 1024;
/// Most maps a playlist can play.
pub const MAX_PLAYLIST_MAPS: usize = 64;
/// Most players in a match or its results.
const MAX_PLAYERS: usize = 16;
/// Most teams a launcher's result scores (Halo 2 has eight team colours).
pub const MAX_TEAMS: usize = 8;
/// Most PCs a FANOUT names (far more than ever join one host), and what it
/// says instead to mean the same PCs as the last.
const MAX_FANOUT: usize = 254;
const SAME_PCS: u8 = 255;
/// Most players an ONLINE list holds.
const MAX_ONLINE: usize = 2000;
/// The bytes of a map file its hash covers.
const HASHED: u64 = 2048;

/// Message kinds.
pub mod kind {
    // PC to server, on the control link.
    pub const LOGIN: u8 = 1;
    pub const PROVE: u8 = 2;
    /// Either way.
    pub const PING: u8 = 3;
    pub const PONG: u8 = 4;
    pub const PROFILE: u8 = 5;
    pub const INVITE: u8 = 10;
    pub const ACCEPT: u8 = 11;
    pub const DECLINE: u8 = 12;
    pub const JOIN_PARTY: u8 = 13;
    pub const LEAVE_PARTY: u8 = 14;
    pub const KICK: u8 = 15;
    pub const PROMOTE: u8 = 16;
    pub const PRIVACY: u8 = 17;
    pub const GUESTS: u8 = 18;
    pub const SEARCH: u8 = 20;
    pub const CANCEL: u8 = 21;
    pub const CUSTOM: u8 = 22;
    pub const CUSTOM_MAP: u8 = 23;
    pub const HOSTING: u8 = 30;
    pub const RESULT: u8 = 31;
    pub const LEFT_MATCH: u8 = 32;
    pub const BACK: u8 = 33;
    /// A launcher's: its game joined the match's host, through the relay.
    pub const JOINED: u8 = 34;
    /// A launcher's: how the match ended, from the engine's results.
    pub const LAUNCHER_RESULT: u8 = 35;
    // PC to server, first on a relay leg.
    pub const LINK_HELLO: u8 = 40;
    // Host to server, on its fan-out leg once linked.
    pub const FANOUT: u8 = 42;
    // Server to PC, on the control link.
    pub const CHALLENGE: u8 = 101;
    pub const WELCOME: u8 = 102;
    pub const REFUSED: u8 = 103;
    pub const PLAYLISTS: u8 = 104;
    pub const ONLINE: u8 = 105;
    pub const PARTY: u8 = 106;
    pub const INVITED: u8 = 107;
    pub const STATUS: u8 = 108;
    pub const MATCH: u8 = 109;
    pub const HOST_MATCH: u8 = 110;
    pub const LINK: u8 = 111;
    pub const GO: u8 = 112;
    pub const MATCH_OVER: u8 = 113;
    pub const NOTICE: u8 = 114;
    pub const CUSTOM_OPEN: u8 = 115;
    /// To a launcher: a match is ready, and how to reach it on the relay.
    pub const LAUNCHER_MATCH: u8 = 116;
    // Server to PC, on a relay leg once its other end is there.
    pub const LINKED: u8 = 41;
}

/// What a PC sends the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToServer {
    /// Sign in (first on the control link).
    Login(Login),
    /// The server's challenge signed with the PC's key (see `proof`).
    Prove([u8; 64]),
    /// Still here; answer with the same number. Both ends ping.
    Ping(u32),
    Pong(u32),
    /// A new gamertag or look.
    Profile {
        gamertag: String,
        look: Look,
    },
    /// Ask a player (by account) into our party.
    Invite(u64),
    /// Answer an invite to this party.
    Accept(u64),
    Decline(u64),
    /// Join this party, if it's open.
    JoinParty(u64),
    /// Leave our party for one of our own.
    LeaveParty,
    /// Party leader only: remove a member, or make them leader.
    Kick(u64),
    Promote(u64),
    Privacy(Privacy),
    /// How many splitscreen guests play on this PC.
    Guests(u8),
    /// Party leader: search this playlist (or `QUICKMATCH`).
    Search(u8),
    /// Party leader: stop searching.
    Cancel,
    /// Party leader: open a custom game, or pick its map. A member: come
    /// back into the party's custom game.
    Custom,
    CustomMap(String),
    /// Hosting this match now.
    Hosting(u64),
    /// How a match ended, as this PC saw it.
    Result {
        id: u64,
        players: Vec<PlayerResult>,
    },
    /// Out of this match before its end: quit, or the host was lost.
    LeftMatch {
        id: u64,
        host_lost: bool,
    },
    /// Back in the party lobby after a game.
    Back,
    /// A launcher joining a match: its game joined the host's game, through
    /// the relay. (The game's joining PCs are seen joining by their relay
    /// legs.)
    Joined(u64),
    /// A launcher: how a match ended, as its engine's results say. The
    /// host's counts; the others' confirm it, as with RESULT.
    LauncherResult(LauncherResult),
    /// First on a relay leg: which leg this is.
    LinkHello {
        token: [u8; 16],
        account: u64,
    },
    /// On a host's fan-out leg: a game message for the PCs joining it with
    /// these accounts (`None`: the same PCs as the last), which the server
    /// passes on to each. Sent to no one, it says the host is still there.
    Fanout {
        to: Option<Vec<u64>>,
        kind: u8,
        body: Vec<u8>,
    },
}

/// What the server sends a PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToPc {
    /// Sign this to prove who you are; and the message of the day.
    Challenge {
        nonce: [u8; 32],
        motd: String,
    },
    /// Signed in (again after a rename): who as, and their levels.
    Welcome(Welcome),
    /// Turned away, and why; the server closes the link.
    Refused(String),
    Playlists(Vec<PlaylistInfo>),
    /// Everyone signed in.
    Online(Vec<OnlinePlayer>),
    /// Our party, whenever it changes.
    Party(PartyInfo),
    /// A player asked us into their party.
    Invited {
        party: u64,
        from: String,
    },
    /// How our party's search is going.
    Status(SearchStatus),
    /// A match is ready: load its map.
    Match(MatchInfo),
    /// Host this match.
    HostMatch(u64),
    /// Dial a relay leg to a player.
    Link(LinkInfo),
    /// Start this match.
    Go(u64),
    MatchOver(MatchOver),
    /// Something to show the player.
    Notice(String),
    /// Our party's leader opened a custom game on this map.
    CustomOpen {
        party: u64,
        leader: u64,
        map: String,
    },
    /// To a launcher: a match is ready (in place of MATCH). It comes again,
    /// with another host, if the one asked didn't start hosting.
    LauncherMatch(LauncherMatch),
    Ping(u32),
    Pong(u32),
    /// On a relay leg: the other end is there, and what follows is theirs.
    Linked,
}

/// Signing in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    /// Which program signs in, at what version. The game's LOGIN is
    /// written as it always was (`MAGIC`, then its `PROTOCOL`); a
    /// launcher's is versioned (`MAGIC_V2`, then `LIVE_PROTOCOL`).
    pub client: LoginClient,
    /// The PC's Ed25519 public key; its account is named by the key.
    pub key: [u8; 32],
    pub gamertag: String,
    pub look: Look,
    /// The stat card the server last gave this PC (empty if none).
    pub card: String,
    /// Maps the PC has: file name (without `.map`) and `map_hash`.
    pub maps: Vec<(String, u64)>,
    /// Splitscreen guests playing on the PC.
    pub guests: u8,
}

/// The program signing in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginClient {
    /// The game (h2viewer), at its `PROTOCOL`: it plays only others at the
    /// same one.
    Viewer { protocol: u32 },
    /// The launcher (h2launch), with its own version (for the server's
    /// log): it plays on MCC's engine, so the game's `PROTOCOL` is no
    /// matter to it.
    Launcher { version: String },
}

impl LoginClient {
    /// The game, at this build's `PROTOCOL`.
    pub fn viewer() -> LoginClient {
        LoginClient::Viewer { protocol: PROTOCOL }
    }

    pub fn kind(&self) -> ClientKind {
        match self {
            LoginClient::Viewer { .. } => ClientKind::Viewer,
            LoginClient::Launcher { .. } => ClientKind::Launcher,
        }
    }
}

/// Which program a PC runs, and so which playlists, parties and players it
/// can play with: the game's players never play the launcher's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClientKind {
    /// The game (h2viewer).
    #[default]
    Viewer,
    /// The launcher (h2launch), on MCC's engine.
    Launcher,
}

/// What a LOGIN says first, read before the rest (which other versions may
/// write differently), so the server can say why it turns a PC away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginHeader {
    /// The game's LOGIN: `MAGIC`, then its `PROTOCOL`.
    Game { protocol: u32 },
    /// A versioned LOGIN: `MAGIC_V2`, then the `LIVE_PROTOCOL` it was
    /// written for, then the kind of program (`CLIENT_VIEWER`,
    /// `CLIENT_LAUNCHER`, or one this version doesn't know). Those three
    /// stay where they are in every version.
    Versioned { live: u32, client: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Welcome {
    pub account: u64,
    pub gamertag: String,
    /// The PC's new stat card, to keep and show next time.
    pub card: String,
    /// The highest level in any ranked playlist (1 if none).
    pub best: u8,
    /// Each ranked playlist played: its id, the level and games played.
    pub levels: Vec<(u8, u8, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistInfo {
    pub id: u8,
    /// Its name in Halo 2's text (like `double_team`), and the name to show
    /// without that.
    pub key: String,
    pub name: String,
    pub ranked: bool,
    pub teams: bool,
    pub guests: bool,
    /// The fewest and most people in a game, and the biggest party.
    pub min: u8,
    pub max: u8,
    pub party_max: u8,
    /// People searching it and playing it now.
    pub searching: u16,
    pub playing: u16,
    /// The player's level in it (0 if it isn't ranked).
    pub level: u8,
    /// The maps it plays: file names, without `.map`.
    pub maps: Vec<String>,
}

/// A signed-in player, as the ONLINE list shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnlinePlayer {
    pub account: u64,
    pub gamertag: String,
    pub look: Look,
    /// Their highest level.
    pub best: u8,
    /// What their party is doing.
    pub activity: Activity,
    pub party: u64,
    /// Their party takes anyone, and how many people are in it (guests
    /// too), and how many more fit.
    pub open: bool,
    pub size: u8,
    pub openings: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyInfo {
    pub id: u64,
    pub leader: u64,
    pub privacy: Privacy,
    pub activity: Activity,
    /// The playlist it searches or plays, while it does, or the one it
    /// played last, back in its lobby.
    pub playlist: u8,
    /// Members, the longest-standing first.
    pub members: Vec<PartyMember>,
    /// Maps every member has (the same file).
    pub maps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyMember {
    pub account: u64,
    pub gamertag: String,
    pub look: Look,
    /// Their highest level, and their level in the party's playlist.
    pub best: u8,
    pub level: u8,
    pub guests: u8,
}

/// Who can join a party.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Privacy {
    /// Anyone (the default).
    #[default]
    Open = 0,
    /// Only those invited.
    InviteOnly = 1,
}

/// What a party (and so each of its members) is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Activity {
    #[default]
    Lobby = 0,
    Searching = 1,
    /// In a matchmade game.
    Playing = 2,
    /// In a custom game.
    Custom = 3,
}

/// How a search is going, in Halo 2's stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Searching = 0,
    Gathering = 1,
    WaitingToFill = 2,
    Balancing = 3,
    Joining = 4,
    Starting = 5,
    Failed = 6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchStatus {
    pub stage: Stage,
    /// People found, and how many more are needed.
    pub have: u8,
    pub need: u8,
    /// Seconds to the start while waiting to fill; otherwise seconds
    /// searched.
    pub seconds: u16,
    /// The levels a player could have to join.
    pub low: u8,
    pub high: u8,
}

/// A match ready to play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchInfo {
    pub id: u64,
    pub playlist: u8,
    pub ranked: bool,
    /// The map's file name and `map_hash`.
    pub map: String,
    pub hash: u64,
    pub game_type: GameType,
    pub preset: String,
    pub score: u32,
    /// Seconds; 0 for none.
    pub time_limit: u16,
    pub bots: u8,
    /// The account asked to host.
    pub host: u64,
    /// Seconds before it starts.
    pub countdown: u8,
    pub players: Vec<MatchPlayer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchPlayer {
    pub account: u64,
    pub gamertag: String,
    pub look: Look,
    pub level: u8,
    pub team: u8,
    pub party: u64,
    pub guests: u8,
}

/// A relay leg to dial, and who's at its other end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInfo {
    /// Names the leg in its LINK_HELLO.
    pub token: [u8; 16],
    /// The match (or custom game's party) it's for.
    pub id: u64,
    pub end: End,
    /// Who's at the other end (no one, on a fan-out leg).
    pub peer: u64,
    pub gamertag: String,
    pub level: u8,
    pub team: u8,
    pub map: String,
}

/// Which end of a relayed game a leg is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// The host's, to a PC joining it.
    Host,
    /// A joining PC's, to the host.
    Joiner,
    /// The host's leg for what goes to many joining PCs at once
    /// (`ToServer::Fanout`).
    Fanout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchOver {
    pub id: u64,
    /// It changed levels; if not, `reason` says why.
    pub counted: bool,
    pub reason: String,
    /// The PC's new stat card.
    pub card: String,
    /// Each playlist the game was rated in (none if it didn't count for
    /// this PC): its id, the old level and the new.
    pub levels: Vec<(u8, u8, u8)>,
    /// Everyone the game was rated for, this PC's player too: their account
    /// and their level in its playlist now, for the carnage report.
    pub players: Vec<(u64, u8)>,
}

/// How one PC's player finished a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerResult {
    pub account: u64,
    pub team: u8,
    pub place: u8,
    pub score: i32,
    pub kills: u16,
    pub deaths: u16,
    /// Quit before the end.
    pub left: bool,
}

/// A match ready for launchers to play on MCC's engine: everything a
/// launcher needs to start halo2.dll's online game through the relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherMatch {
    /// The match, as HOST_MATCH, GO, JOINED, LEFT_MATCH, LAUNCHER_RESULT
    /// and MATCH_OVER name it.
    pub id: u64,
    pub playlist: u8,
    pub ranked: bool,
    /// Teams; otherwise everyone for themselves.
    pub teams: bool,
    /// The MCC map's name (like `lockout`) and game variant's (like
    /// `01_slayer`), as the playlist names them.
    pub map: String,
    pub variant: String,
    /// This PC's way into the match's room on the relay.
    pub relay: RelaySeat,
    /// The relay id of the PC asked to host.
    pub host: u64,
    /// Seconds from the host starting to host to the game's start, at the
    /// latest: a PC that hasn't joined by then is left out.
    pub countdown: u8,
    /// Everyone in the match, this PC too.
    pub players: Vec<LauncherPlayer>,
}

/// A launcher's way into its match's room on the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelaySeat {
    /// The relay's UDP port, at the address the PC dialled the server at.
    pub port: u16,
    /// The match's room on the relay.
    pub room: u64,
    /// This PC's id in it (and its player's XUID in the engine): the same
    /// for an account in every match.
    pub id: u64,
    /// The key that proves the id is this PC's (h2relay's `MemberKey`),
    /// given to this PC only.
    pub key: [u8; MEMBER_KEY_LEN],
}

/// A player in a launcher's match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherPlayer {
    /// Their id on the relay and in the engine.
    pub relay_id: u64,
    pub account: u64,
    pub gamertag: String,
    pub team: u8,
    /// Their level in the playlist (their best, if it isn't ranked).
    pub level: u8,
    pub party: u64,
}

/// How a launcher's match ended, filled in from the engine's results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherResult {
    pub id: u64,
    /// The game ended as its variant says (score or time reached), rather
    /// than being cut short: only then can it count.
    pub finished: bool,
    /// Each team's score, by team number (none in free-for-all games).
    pub team_scores: Vec<i32>,
    pub players: Vec<LauncherPlayerResult>,
}

/// How one player finished a launcher's match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LauncherPlayerResult {
    pub relay_id: u64,
    pub team: u8,
    /// Their standing, 0 for first (in team games, their team's).
    pub place: u8,
    pub score: i32,
    pub kills: u16,
    pub deaths: u16,
    /// Quit before the end.
    pub left: bool,
}

/// What a PC signs to prove it holds `key`: a fixed text, the server's
/// challenge and the key.
pub fn proof(nonce: &[u8; 32], key: &[u8; 32]) -> Vec<u8> {
    [&b"h2live-login"[..], nonce, key].concat()
}

/// What a LOGIN says first, if it's a LOGIN at all.
pub fn login_header(body: &[u8]) -> Option<LoginHeader> {
    let mut r = Reader::new(body);
    match r.u32().ok()? {
        MAGIC => Some(LoginHeader::Game {
            protocol: r.u32().ok()?,
        }),
        MAGIC_V2 => Some(LoginHeader::Versioned {
            live: r.u32().ok()?,
            client: r.u8().ok()?,
        }),
        _ => None,
    }
}

/// What tells copies of a map apart: FNV-1a (64-bit) over its first 2 KiB
/// and then its length (8 bytes, little-endian).
pub fn map_hash(path: &Path) -> std::io::Result<u64> {
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut head = Vec::new();
    file.take(HASHED).read_to_end(&mut head)?;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in head.iter().chain(&len.to_le_bytes()) {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Ok(hash)
}

fn bytes<const N: usize>(w: &mut Writer, b: &[u8; N]) {
    w.0.extend_from_slice(b);
}

fn read_bytes<const N: usize>(r: &mut Reader) -> Result<[u8; N], Malformed> {
    let mut b = [0; N];
    for x in &mut b {
        *x = r.u8()?;
    }
    Ok(b)
}

/// A string of at most `max` bytes.
fn read_str(r: &mut Reader, max: usize) -> Result<String, Malformed> {
    let s = r.str()?;
    if s.len() > max {
        return Err(Malformed);
    }
    Ok(s)
}

/// A 0 or 1.
fn read_bool(r: &mut Reader) -> Result<bool, Malformed> {
    match r.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Malformed),
    }
}

/// A count of at most `max` things, sent as a byte.
fn read_count(r: &mut Reader, max: usize) -> Result<usize, Malformed> {
    Some(r.u8()? as usize)
        .filter(|&n| n <= max)
        .ok_or(Malformed)
}

/// A count of at most `max` things, sent as two bytes.
fn read_count16(r: &mut Reader, max: usize) -> Result<usize, Malformed> {
    Some(r.u16()? as usize)
        .filter(|&n| n <= max)
        .ok_or(Malformed)
}

fn read_guests(r: &mut Reader) -> Result<u8, Malformed> {
    Some(r.u8()?).filter(|&g| g <= MAX_GUESTS).ok_or(Malformed)
}

/// A list's length, `n`, as a byte (the lists sent never hold more).
fn count(w: &mut Writer, n: usize) {
    w.u8(n.min(255) as u8);
}

impl Privacy {
    fn read(r: &mut Reader) -> Result<Privacy, Malformed> {
        match r.u8()? {
            0 => Ok(Privacy::Open),
            1 => Ok(Privacy::InviteOnly),
            _ => Err(Malformed),
        }
    }
}

impl Activity {
    fn read(r: &mut Reader) -> Result<Activity, Malformed> {
        let all = [
            Activity::Lobby,
            Activity::Searching,
            Activity::Playing,
            Activity::Custom,
        ];
        all.get(r.u8()? as usize).copied().ok_or(Malformed)
    }
}

impl Stage {
    fn read(r: &mut Reader) -> Result<Stage, Malformed> {
        let all = [
            Stage::Searching,
            Stage::Gathering,
            Stage::WaitingToFill,
            Stage::Balancing,
            Stage::Joining,
            Stage::Starting,
            Stage::Failed,
        ];
        all.get(r.u8()? as usize).copied().ok_or(Malformed)
    }
}

impl Login {
    fn write(&self, w: &mut Writer) {
        match &self.client {
            // As the game always has, so it signs in to servers from before
            // there were versioned LOGINs too.
            LoginClient::Viewer { protocol } => {
                w.u32(MAGIC);
                w.u32(*protocol);
            }
            LoginClient::Launcher { version } => {
                w.u32(MAGIC_V2);
                w.u32(LIVE_PROTOCOL);
                w.u8(CLIENT_LAUNCHER);
                w.str(version);
            }
        }
        bytes(w, &self.key);
        w.str(&self.gamertag);
        self.look.write(w);
        w.str(&self.card);
        let maps = &self.maps[..self.maps.len().min(MAX_MAPS)];
        w.u16(maps.len() as u16);
        for (name, hash) in maps {
            w.str(name);
            w.u64(*hash);
        }
        w.u8(self.guests);
    }

    /// A LOGIN: the game's at this build's `PROTOCOL`, or a versioned one
    /// at this build's `LIVE_PROTOCOL` (the version of the program in it is
    /// the server's to judge).
    fn read(r: &mut Reader) -> Result<Login, Malformed> {
        let client = match (r.u32()?, r.u32()?) {
            (MAGIC, PROTOCOL) => LoginClient::Viewer { protocol: PROTOCOL },
            (MAGIC_V2, LIVE_PROTOCOL) => match r.u8()? {
                CLIENT_VIEWER => LoginClient::Viewer { protocol: r.u32()? },
                CLIENT_LAUNCHER => LoginClient::Launcher {
                    version: read_str(r, MAX_NAME)?,
                },
                _ => return Err(Malformed),
            },
            _ => return Err(Malformed),
        };
        let key = read_bytes(r)?;
        let gamertag = read_str(r, MAX_NAME)?;
        let look = Look::read(r)?;
        let card = read_str(r, MAX_CARD)?;
        let n = read_count16(r, MAX_MAPS)?;
        let maps = (0..n)
            .map(|_| Ok((read_str(r, MAX_NAME)?, r.u64()?)))
            .collect::<Result<_, _>>()?;
        Ok(Login {
            client,
            key,
            gamertag,
            look,
            card,
            maps,
            guests: read_guests(r)?,
        })
    }
}

impl Welcome {
    fn write(&self, w: &mut Writer) {
        w.u64(self.account);
        w.str(&self.gamertag);
        w.str(&self.card);
        w.u8(self.best);
        count(w, self.levels.len());
        for &(playlist, level, games) in self.levels.iter().take(255) {
            w.u8(playlist);
            w.u8(level);
            w.u32(games);
        }
    }

    fn read(r: &mut Reader) -> Result<Welcome, Malformed> {
        let (account, gamertag, card, best) = (
            r.u64()?,
            read_str(r, MAX_NAME)?,
            read_str(r, MAX_CARD)?,
            r.u8()?,
        );
        let n = read_count(r, 255)?;
        let levels = (0..n)
            .map(|_| Ok((r.u8()?, r.u8()?, r.u32()?)))
            .collect::<Result<_, _>>()?;
        Ok(Welcome {
            account,
            gamertag,
            card,
            best,
            levels,
        })
    }
}

impl PlaylistInfo {
    fn write(&self, w: &mut Writer) {
        w.u8(self.id);
        w.str(&self.key);
        w.str(&self.name);
        w.u8((self.ranked as u8) | (self.teams as u8) << 1 | (self.guests as u8) << 2);
        w.u8(self.min);
        w.u8(self.max);
        w.u8(self.party_max);
        w.u16(self.searching);
        w.u16(self.playing);
        w.u8(self.level);
        let maps = &self.maps[..self.maps.len().min(MAX_PLAYLIST_MAPS)];
        count(w, maps.len());
        for map in maps {
            w.str(map);
        }
    }

    fn read(r: &mut Reader) -> Result<PlaylistInfo, Malformed> {
        let (id, key, name) = (r.u8()?, read_str(r, MAX_NAME)?, read_str(r, MAX_NAME)?);
        let flags = r.u8()?;
        if flags > 7 {
            return Err(Malformed);
        }
        let (min, max, party_max) = (r.u8()?, r.u8()?, r.u8()?);
        let (searching, playing, level) = (r.u16()?, r.u16()?, r.u8()?);
        let n = read_count(r, MAX_PLAYLIST_MAPS)?;
        let maps = (0..n)
            .map(|_| read_str(r, MAX_NAME))
            .collect::<Result<_, _>>()?;
        Ok(PlaylistInfo {
            id,
            key,
            name,
            ranked: flags & 1 != 0,
            teams: flags & 2 != 0,
            guests: flags & 4 != 0,
            min,
            max,
            party_max,
            searching,
            playing,
            level,
            maps,
        })
    }
}

impl OnlinePlayer {
    fn write(&self, w: &mut Writer) {
        w.u64(self.account);
        w.str(&self.gamertag);
        self.look.write(w);
        w.u8(self.best);
        w.u8(self.activity as u8);
        w.u64(self.party);
        w.bool(self.open);
        w.u8(self.size);
        w.u8(self.openings);
    }

    fn read(r: &mut Reader) -> Result<OnlinePlayer, Malformed> {
        Ok(OnlinePlayer {
            account: r.u64()?,
            gamertag: read_str(r, MAX_NAME)?,
            look: Look::read(r)?,
            best: r.u8()?,
            activity: Activity::read(r)?,
            party: r.u64()?,
            open: read_bool(r)?,
            size: r.u8()?,
            openings: r.u8()?,
        })
    }
}

impl PartyInfo {
    fn write(&self, w: &mut Writer) {
        w.u64(self.id);
        w.u64(self.leader);
        w.u8(self.privacy as u8);
        w.u8(self.activity as u8);
        w.u8(self.playlist);
        let members = &self.members[..self.members.len().min(MAX_PARTY)];
        count(w, members.len());
        for m in members {
            w.u64(m.account);
            w.str(&m.gamertag);
            m.look.write(w);
            w.u8(m.best);
            w.u8(m.level);
            w.u8(m.guests);
        }
        let maps = &self.maps[..self.maps.len().min(MAX_MAPS)];
        w.u16(maps.len() as u16);
        for map in maps {
            w.str(map);
        }
    }

    fn read(r: &mut Reader) -> Result<PartyInfo, Malformed> {
        let (id, leader) = (r.u64()?, r.u64()?);
        let (privacy, activity, playlist) = (Privacy::read(r)?, Activity::read(r)?, r.u8()?);
        let n = read_count(r, MAX_PARTY)?;
        let members = (0..n)
            .map(|_| {
                Ok(PartyMember {
                    account: r.u64()?,
                    gamertag: read_str(r, MAX_NAME)?,
                    look: Look::read(r)?,
                    best: r.u8()?,
                    level: r.u8()?,
                    guests: read_guests(r)?,
                })
            })
            .collect::<Result<_, _>>()?;
        let n = read_count16(r, MAX_MAPS)?;
        let maps = (0..n)
            .map(|_| read_str(r, MAX_NAME))
            .collect::<Result<_, _>>()?;
        Ok(PartyInfo {
            id,
            leader,
            privacy,
            activity,
            playlist,
            members,
            maps,
        })
    }
}

impl SearchStatus {
    fn write(&self, w: &mut Writer) {
        w.u8(self.stage as u8);
        w.u8(self.have);
        w.u8(self.need);
        w.u16(self.seconds);
        w.u8(self.low);
        w.u8(self.high);
    }

    fn read(r: &mut Reader) -> Result<SearchStatus, Malformed> {
        Ok(SearchStatus {
            stage: Stage::read(r)?,
            have: r.u8()?,
            need: r.u8()?,
            seconds: r.u16()?,
            low: r.u8()?,
            high: r.u8()?,
        })
    }
}

impl MatchInfo {
    fn write(&self, w: &mut Writer) {
        w.u64(self.id);
        w.u8(self.playlist);
        w.bool(self.ranked);
        w.str(&self.map);
        w.u64(self.hash);
        let game_type = GameType::ALL.iter().position(|&t| t == self.game_type);
        w.u8(game_type.unwrap_or(0) as u8);
        w.str(&self.preset);
        w.u32(self.score);
        w.u16(self.time_limit);
        w.u8(self.bots);
        w.u64(self.host);
        w.u8(self.countdown);
        let players = &self.players[..self.players.len().min(MAX_PLAYERS)];
        count(w, players.len());
        for p in players {
            w.u64(p.account);
            w.str(&p.gamertag);
            p.look.write(w);
            w.u8(p.level);
            w.u8(p.team);
            w.u64(p.party);
            w.u8(p.guests);
        }
    }

    fn read(r: &mut Reader) -> Result<MatchInfo, Malformed> {
        let (id, playlist, ranked) = (r.u64()?, r.u8()?, read_bool(r)?);
        let (map, hash) = (read_str(r, MAX_NAME)?, r.u64()?);
        let game_type = *GameType::ALL.get(r.u8()? as usize).ok_or(Malformed)?;
        let preset = read_str(r, MAX_NAME)?;
        let (score, time_limit, bots) = (r.u32()?, r.u16()?, r.u8()?);
        let (host, countdown) = (r.u64()?, r.u8()?);
        let n = read_count(r, MAX_PLAYERS)?;
        let players = (0..n)
            .map(|_| {
                Ok(MatchPlayer {
                    account: r.u64()?,
                    gamertag: read_str(r, MAX_NAME)?,
                    look: Look::read(r)?,
                    level: r.u8()?,
                    team: r.u8()?,
                    party: r.u64()?,
                    guests: read_guests(r)?,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(MatchInfo {
            id,
            playlist,
            ranked,
            map,
            hash,
            game_type,
            preset,
            score,
            time_limit,
            bots,
            host,
            countdown,
            players,
        })
    }
}

impl LinkInfo {
    fn write(&self, w: &mut Writer) {
        bytes(w, &self.token);
        w.u64(self.id);
        w.u8(self.end as u8);
        w.u64(self.peer);
        w.str(&self.gamertag);
        w.u8(self.level);
        w.u8(self.team);
        w.str(&self.map);
    }

    fn read(r: &mut Reader) -> Result<LinkInfo, Malformed> {
        Ok(LinkInfo {
            token: read_bytes(r)?,
            id: r.u64()?,
            end: match r.u8()? {
                0 => End::Host,
                1 => End::Joiner,
                2 => End::Fanout,
                _ => return Err(Malformed),
            },
            peer: r.u64()?,
            gamertag: read_str(r, MAX_NAME)?,
            level: r.u8()?,
            team: r.u8()?,
            map: read_str(r, MAX_NAME)?,
        })
    }
}

impl MatchOver {
    fn write(&self, w: &mut Writer) {
        w.u64(self.id);
        w.bool(self.counted);
        w.str(&self.reason);
        w.str(&self.card);
        count(w, self.levels.len());
        for &(playlist, old, new) in self.levels.iter().take(255) {
            w.u8(playlist);
            w.u8(old);
            w.u8(new);
        }
        let players = &self.players[..self.players.len().min(MAX_PLAYERS)];
        count(w, players.len());
        for &(account, level) in players {
            w.u64(account);
            w.u8(level);
        }
    }

    fn read(r: &mut Reader) -> Result<MatchOver, Malformed> {
        let (id, counted) = (r.u64()?, read_bool(r)?);
        let (reason, card) = (read_str(r, MAX_TEXT)?, read_str(r, MAX_CARD)?);
        let n = read_count(r, 255)?;
        let levels = (0..n)
            .map(|_| Ok((r.u8()?, r.u8()?, r.u8()?)))
            .collect::<Result<_, _>>()?;
        let n = read_count(r, MAX_PLAYERS)?;
        let players = (0..n)
            .map(|_| Ok((r.u64()?, r.u8()?)))
            .collect::<Result<_, _>>()?;
        Ok(MatchOver {
            id,
            counted,
            reason,
            card,
            levels,
            players,
        })
    }
}

impl LauncherMatch {
    fn write(&self, w: &mut Writer) {
        w.u64(self.id);
        w.u8(self.playlist);
        w.bool(self.ranked);
        w.bool(self.teams);
        w.str(&self.map);
        w.str(&self.variant);
        w.u16(self.relay.port);
        w.u64(self.relay.room);
        w.u64(self.relay.id);
        bytes(w, &self.relay.key);
        w.u64(self.host);
        w.u8(self.countdown);
        let players = &self.players[..self.players.len().min(MAX_PLAYERS)];
        count(w, players.len());
        for p in players {
            w.u64(p.relay_id);
            w.u64(p.account);
            w.str(&p.gamertag);
            w.u8(p.team);
            w.u8(p.level);
            w.u64(p.party);
        }
    }

    fn read(r: &mut Reader) -> Result<LauncherMatch, Malformed> {
        let (id, playlist, ranked, teams) = (r.u64()?, r.u8()?, read_bool(r)?, read_bool(r)?);
        let (map, variant) = (read_str(r, MAX_NAME)?, read_str(r, MAX_NAME)?);
        let relay = RelaySeat {
            port: r.u16()?,
            room: r.u64()?,
            id: r.u64()?,
            key: read_bytes(r)?,
        };
        let (host, countdown) = (r.u64()?, r.u8()?);
        let n = read_count(r, MAX_PLAYERS)?;
        let players = (0..n)
            .map(|_| {
                Ok(LauncherPlayer {
                    relay_id: r.u64()?,
                    account: r.u64()?,
                    gamertag: read_str(r, MAX_NAME)?,
                    team: r.u8()?,
                    level: r.u8()?,
                    party: r.u64()?,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(LauncherMatch {
            id,
            playlist,
            ranked,
            teams,
            map,
            variant,
            relay,
            host,
            countdown,
            players,
        })
    }
}

impl LauncherResult {
    fn write(&self, w: &mut Writer) {
        w.u64(self.id);
        w.bool(self.finished);
        let scores = &self.team_scores[..self.team_scores.len().min(MAX_TEAMS)];
        count(w, scores.len());
        for &score in scores {
            w.u32(score as u32);
        }
        let players = &self.players[..self.players.len().min(MAX_PLAYERS)];
        count(w, players.len());
        for p in players {
            w.u64(p.relay_id);
            w.u8(p.team);
            w.u8(p.place);
            w.u32(p.score as u32);
            w.u16(p.kills);
            w.u16(p.deaths);
            w.bool(p.left);
        }
    }

    fn read(r: &mut Reader) -> Result<LauncherResult, Malformed> {
        let (id, finished) = (r.u64()?, read_bool(r)?);
        let n = read_count(r, MAX_TEAMS)?;
        let team_scores = (0..n)
            .map(|_| Ok(r.u32()? as i32))
            .collect::<Result<_, _>>()?;
        let n = read_count(r, MAX_PLAYERS)?;
        let players = (0..n)
            .map(|_| {
                Ok(LauncherPlayerResult {
                    relay_id: r.u64()?,
                    team: r.u8()?,
                    place: r.u8()?,
                    score: r.u32()? as i32,
                    kills: r.u16()?,
                    deaths: r.u16()?,
                    left: read_bool(r)?,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(LauncherResult {
            id,
            finished,
            team_scores,
            players,
        })
    }
}

impl PlayerResult {
    fn write(&self, w: &mut Writer) {
        w.u64(self.account);
        w.u8(self.team);
        w.u8(self.place);
        w.u32(self.score as u32);
        w.u16(self.kills);
        w.u16(self.deaths);
        w.bool(self.left);
    }

    fn read(r: &mut Reader) -> Result<PlayerResult, Malformed> {
        Ok(PlayerResult {
            account: r.u64()?,
            team: r.u8()?,
            place: r.u8()?,
            score: r.u32()? as i32,
            kills: r.u16()?,
            deaths: r.u16()?,
            left: read_bool(r)?,
        })
    }
}

impl ToServer {
    /// The message's kind and body.
    pub fn write(&self) -> (u8, Vec<u8>) {
        let mut w = Writer::default();
        let kind = match self {
            ToServer::Login(login) => {
                login.write(&mut w);
                kind::LOGIN
            }
            ToServer::Prove(signature) => {
                bytes(&mut w, signature);
                kind::PROVE
            }
            ToServer::Ping(n) => {
                w.u32(*n);
                kind::PING
            }
            ToServer::Pong(n) => {
                w.u32(*n);
                kind::PONG
            }
            ToServer::Profile { gamertag, look } => {
                w.str(gamertag);
                look.write(&mut w);
                kind::PROFILE
            }
            ToServer::Invite(account) => {
                w.u64(*account);
                kind::INVITE
            }
            ToServer::Accept(party) => {
                w.u64(*party);
                kind::ACCEPT
            }
            ToServer::Decline(party) => {
                w.u64(*party);
                kind::DECLINE
            }
            ToServer::JoinParty(party) => {
                w.u64(*party);
                kind::JOIN_PARTY
            }
            ToServer::LeaveParty => kind::LEAVE_PARTY,
            ToServer::Kick(account) => {
                w.u64(*account);
                kind::KICK
            }
            ToServer::Promote(account) => {
                w.u64(*account);
                kind::PROMOTE
            }
            ToServer::Privacy(privacy) => {
                w.u8(*privacy as u8);
                kind::PRIVACY
            }
            ToServer::Guests(n) => {
                w.u8(*n);
                kind::GUESTS
            }
            ToServer::Search(playlist) => {
                w.u8(*playlist);
                kind::SEARCH
            }
            ToServer::Cancel => kind::CANCEL,
            ToServer::Custom => kind::CUSTOM,
            ToServer::CustomMap(map) => {
                w.str(map);
                kind::CUSTOM_MAP
            }
            ToServer::Hosting(id) => {
                w.u64(*id);
                kind::HOSTING
            }
            ToServer::Result { id, players } => {
                w.u64(*id);
                let players = &players[..players.len().min(MAX_PLAYERS)];
                count(&mut w, players.len());
                for p in players {
                    p.write(&mut w);
                }
                kind::RESULT
            }
            ToServer::LeftMatch { id, host_lost } => {
                w.u64(*id);
                w.bool(*host_lost);
                kind::LEFT_MATCH
            }
            ToServer::Back => kind::BACK,
            ToServer::Joined(id) => {
                w.u64(*id);
                kind::JOINED
            }
            ToServer::LauncherResult(result) => {
                result.write(&mut w);
                kind::LAUNCHER_RESULT
            }
            ToServer::LinkHello { token, account } => {
                bytes(&mut w, token);
                w.u64(*account);
                kind::LINK_HELLO
            }
            ToServer::Fanout { to, kind, body } => {
                match to {
                    Some(to) => {
                        let to = &to[..to.len().min(MAX_FANOUT)];
                        w.u8(to.len() as u8);
                        for &account in to {
                            w.u64(account);
                        }
                    }
                    None => w.u8(SAME_PCS),
                }
                w.u8(*kind);
                w.0.extend_from_slice(body);
                kind::FANOUT
            }
        };
        (kind, w.0)
    }

    /// The message of `kind` with this body, if it's a whole, valid one.
    pub fn read(kind: u8, body: &[u8]) -> Result<ToServer, Malformed> {
        let r = &mut Reader::new(body);
        let message = match kind {
            kind::LOGIN => ToServer::Login(Login::read(r)?),
            kind::PROVE => ToServer::Prove(read_bytes(r)?),
            kind::PING => ToServer::Ping(r.u32()?),
            kind::PONG => ToServer::Pong(r.u32()?),
            kind::PROFILE => ToServer::Profile {
                gamertag: read_str(r, MAX_NAME)?,
                look: Look::read(r)?,
            },
            kind::INVITE => ToServer::Invite(r.u64()?),
            kind::ACCEPT => ToServer::Accept(r.u64()?),
            kind::DECLINE => ToServer::Decline(r.u64()?),
            kind::JOIN_PARTY => ToServer::JoinParty(r.u64()?),
            kind::LEAVE_PARTY => ToServer::LeaveParty,
            kind::KICK => ToServer::Kick(r.u64()?),
            kind::PROMOTE => ToServer::Promote(r.u64()?),
            kind::PRIVACY => ToServer::Privacy(Privacy::read(r)?),
            kind::GUESTS => ToServer::Guests(read_guests(r)?),
            kind::SEARCH => ToServer::Search(r.u8()?),
            kind::CANCEL => ToServer::Cancel,
            kind::CUSTOM => ToServer::Custom,
            kind::CUSTOM_MAP => ToServer::CustomMap(read_str(r, MAX_NAME)?),
            kind::HOSTING => ToServer::Hosting(r.u64()?),
            kind::RESULT => {
                let id = r.u64()?;
                let n = read_count(r, MAX_PLAYERS)?;
                let players = (0..n)
                    .map(|_| PlayerResult::read(r))
                    .collect::<Result<_, _>>()?;
                ToServer::Result { id, players }
            }
            kind::LEFT_MATCH => ToServer::LeftMatch {
                id: r.u64()?,
                host_lost: read_bool(r)?,
            },
            kind::BACK => ToServer::Back,
            kind::JOINED => ToServer::Joined(r.u64()?),
            kind::LAUNCHER_RESULT => ToServer::LauncherResult(LauncherResult::read(r)?),
            kind::LINK_HELLO => ToServer::LinkHello {
                token: read_bytes(r)?,
                account: r.u64()?,
            },
            kind::FANOUT => {
                let to = match r.u8()? {
                    SAME_PCS => None,
                    n => Some((0..n).map(|_| r.u64()).collect::<Result<Vec<_>, _>>()?),
                };
                let kind = r.u8()?;
                // The game message's body is the rest, however long.
                let at = 2 + 8 * to.as_ref().map_or(0, Vec::len);
                let body = body[at..].to_vec();
                return Ok(ToServer::Fanout { to, kind, body });
            }
            _ => return Err(Malformed),
        };
        if !r.at_end() {
            return Err(Malformed);
        }
        Ok(message)
    }

    /// Queue it on `conn` (it goes with the next flush).
    pub fn send(&self, conn: &mut Connection) {
        let (kind, body) = self.write();
        conn.send(kind, &body);
    }
}

impl ToPc {
    /// The message's kind and body.
    pub fn write(&self) -> (u8, Vec<u8>) {
        let mut w = Writer::default();
        let kind = match self {
            ToPc::Challenge { nonce, motd } => {
                bytes(&mut w, nonce);
                w.str(motd);
                kind::CHALLENGE
            }
            ToPc::Welcome(welcome) => {
                welcome.write(&mut w);
                kind::WELCOME
            }
            ToPc::Refused(why) => {
                w.str(why);
                kind::REFUSED
            }
            ToPc::Playlists(playlists) => {
                count(&mut w, playlists.len());
                for p in playlists.iter().take(255) {
                    p.write(&mut w);
                }
                kind::PLAYLISTS
            }
            ToPc::Online(players) => {
                let players = &players[..players.len().min(MAX_ONLINE)];
                w.u16(players.len() as u16);
                for p in players {
                    p.write(&mut w);
                }
                kind::ONLINE
            }
            ToPc::Party(party) => {
                party.write(&mut w);
                kind::PARTY
            }
            ToPc::Invited { party, from } => {
                w.u64(*party);
                w.str(from);
                kind::INVITED
            }
            ToPc::Status(status) => {
                status.write(&mut w);
                kind::STATUS
            }
            ToPc::Match(m) => {
                m.write(&mut w);
                kind::MATCH
            }
            ToPc::HostMatch(id) => {
                w.u64(*id);
                kind::HOST_MATCH
            }
            ToPc::Link(link) => {
                link.write(&mut w);
                kind::LINK
            }
            ToPc::Go(id) => {
                w.u64(*id);
                kind::GO
            }
            ToPc::MatchOver(over) => {
                over.write(&mut w);
                kind::MATCH_OVER
            }
            ToPc::Notice(text) => {
                w.str(text);
                kind::NOTICE
            }
            ToPc::CustomOpen { party, leader, map } => {
                w.u64(*party);
                w.u64(*leader);
                w.str(map);
                kind::CUSTOM_OPEN
            }
            ToPc::LauncherMatch(m) => {
                m.write(&mut w);
                kind::LAUNCHER_MATCH
            }
            ToPc::Ping(n) => {
                w.u32(*n);
                kind::PING
            }
            ToPc::Pong(n) => {
                w.u32(*n);
                kind::PONG
            }
            ToPc::Linked => kind::LINKED,
        };
        (kind, w.0)
    }

    /// The message of `kind` with this body, if it's a whole, valid one.
    pub fn read(kind: u8, body: &[u8]) -> Result<ToPc, Malformed> {
        let r = &mut Reader::new(body);
        let message = match kind {
            kind::CHALLENGE => ToPc::Challenge {
                nonce: read_bytes(r)?,
                motd: read_str(r, MAX_TEXT)?,
            },
            kind::WELCOME => ToPc::Welcome(Welcome::read(r)?),
            kind::REFUSED => ToPc::Refused(read_str(r, MAX_TEXT)?),
            kind::PLAYLISTS => {
                let n = read_count(r, 255)?;
                let playlists = (0..n)
                    .map(|_| PlaylistInfo::read(r))
                    .collect::<Result<_, _>>()?;
                ToPc::Playlists(playlists)
            }
            kind::ONLINE => {
                let n = read_count16(r, MAX_ONLINE)?;
                let players = (0..n)
                    .map(|_| OnlinePlayer::read(r))
                    .collect::<Result<_, _>>()?;
                ToPc::Online(players)
            }
            kind::PARTY => ToPc::Party(PartyInfo::read(r)?),
            kind::INVITED => ToPc::Invited {
                party: r.u64()?,
                from: read_str(r, MAX_NAME)?,
            },
            kind::STATUS => ToPc::Status(SearchStatus::read(r)?),
            kind::MATCH => ToPc::Match(MatchInfo::read(r)?),
            kind::HOST_MATCH => ToPc::HostMatch(r.u64()?),
            kind::LINK => ToPc::Link(LinkInfo::read(r)?),
            kind::GO => ToPc::Go(r.u64()?),
            kind::MATCH_OVER => ToPc::MatchOver(MatchOver::read(r)?),
            kind::NOTICE => ToPc::Notice(read_str(r, MAX_TEXT)?),
            kind::CUSTOM_OPEN => ToPc::CustomOpen {
                party: r.u64()?,
                leader: r.u64()?,
                map: read_str(r, MAX_NAME)?,
            },
            kind::LAUNCHER_MATCH => ToPc::LauncherMatch(LauncherMatch::read(r)?),
            kind::PING => ToPc::Ping(r.u32()?),
            kind::PONG => ToPc::Pong(r.u32()?),
            kind::LINKED => ToPc::Linked,
            _ => return Err(Malformed),
        };
        if !r.at_end() {
            return Err(Malformed);
        }
        Ok(message)
    }

    /// Queue it on `conn` (it goes with the next flush).
    pub fn send(&self, conn: &mut Connection) {
        let (kind, body) = self.write();
        conn.send(kind, &body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(i: usize) -> Look {
        Look::default_for(i)
    }

    fn login() -> Login {
        Login {
            client: LoginClient::viewer(),
            key: [7; 32],
            gamertag: "NOBLE SIX".into(),
            look: look(3),
            card: "a 1 2 3\ns 4\n".into(),
            maps: vec![
                ("lockout".into(), 0x1234_5678_9abc_def0),
                ("midship".into(), 9),
            ],
            guests: 2,
        }
    }

    fn launcher_login() -> Login {
        Login {
            client: LoginClient::Launcher {
                version: "h2launch 0.1.0".into(),
            },
            look: Look::default(),
            guests: 0,
            ..login()
        }
    }

    fn launcher_result() -> LauncherResult {
        let player = LauncherPlayerResult {
            relay_id: 0xfeed_0000_0000_0001,
            team: 1,
            place: 0,
            score: 50,
            kills: 50,
            deaths: 12,
            left: false,
        };
        LauncherResult {
            id: 23,
            finished: true,
            team_scores: vec![-1, 50],
            players: vec![
                player,
                LauncherPlayerResult {
                    relay_id: 2,
                    team: 0,
                    place: 1,
                    score: -1,
                    left: true,
                    ..player
                },
            ],
        }
    }

    fn launcher_match() -> LauncherMatch {
        let player = LauncherPlayer {
            relay_id: 0xfeed_0000_0000_0001,
            account: 0xfeed_0000_0000_0001,
            gamertag: "JORGE".into(),
            team: 1,
            level: 7,
            party: 3,
        };
        LauncherMatch {
            id: 24,
            playlist: 11,
            ranked: true,
            teams: true,
            map: "lockout".into(),
            variant: "01_slayer".into(),
            relay: RelaySeat {
                port: 47050,
                room: 24,
                id: 0xfeed_0000_0000_0001,
                key: [5; MEMBER_KEY_LEN],
            },
            host: 2,
            countdown: 20,
            players: vec![
                player.clone(),
                LauncherPlayer {
                    relay_id: 2,
                    account: 2,
                    team: 0,
                    ..player
                },
            ],
        }
    }

    fn every_message_to_the_server() -> Vec<ToServer> {
        let result = PlayerResult {
            account: 5,
            team: 1,
            place: 0,
            score: -3,
            kills: 12,
            deaths: 15,
            left: true,
        };
        vec![
            ToServer::Login(login()),
            ToServer::Prove([9; 64]),
            ToServer::Ping(1),
            ToServer::Pong(u32::MAX),
            ToServer::Profile {
                gamertag: "KAT".into(),
                look: look(1),
            },
            ToServer::Invite(11),
            ToServer::Accept(12),
            ToServer::Decline(13),
            ToServer::JoinParty(14),
            ToServer::LeaveParty,
            ToServer::Kick(15),
            ToServer::Promote(16),
            ToServer::Privacy(Privacy::InviteOnly),
            ToServer::Guests(3),
            ToServer::Search(QUICKMATCH),
            ToServer::Cancel,
            ToServer::Custom,
            ToServer::CustomMap("zanzibar".into()),
            ToServer::Hosting(17),
            ToServer::Result {
                id: 18,
                players: vec![
                    result,
                    PlayerResult {
                        left: false,
                        ..result
                    },
                ],
            },
            ToServer::LeftMatch {
                id: 19,
                host_lost: true,
            },
            ToServer::Back,
            ToServer::Login(launcher_login()),
            ToServer::Joined(21),
            ToServer::LauncherResult(launcher_result()),
            ToServer::LinkHello {
                token: [3; 16],
                account: 20,
            },
            ToServer::Fanout {
                to: Some(vec![21, 22]),
                kind: 107,
                body: Vec::new(),
            },
        ]
    }

    fn every_message_to_a_pc() -> Vec<ToPc> {
        let member = PartyMember {
            account: 1,
            gamertag: "CARTER".into(),
            look: look(2),
            best: 30,
            level: 12,
            guests: 1,
        };
        let player = MatchPlayer {
            account: 2,
            gamertag: "JORGE".into(),
            look: look(4),
            level: 7,
            team: 1,
            party: 3,
            guests: 0,
        };
        vec![
            ToPc::Challenge {
                nonce: [1; 32],
                motd: "Welcome back".into(),
            },
            ToPc::Welcome(Welcome {
                account: 0xdead_beef,
                gamertag: "EMILE".into(),
                card: "a card".into(),
                best: 50,
                levels: vec![(0, 50, 1000), (2, 3, 4)],
            }),
            ToPc::Refused(GAMERTAG_TAKEN.into()),
            ToPc::Playlists(vec![PlaylistInfo {
                id: 2,
                key: "double_team".into(),
                name: "Double Team".into(),
                ranked: true,
                teams: true,
                guests: false,
                min: 4,
                max: 4,
                party_max: 2,
                searching: 6,
                playing: 300,
                level: 9,
                maps: vec!["lockout".into(), "beavercreek".into()],
            }]),
            ToPc::Online(vec![OnlinePlayer {
                account: 4,
                gamertag: "JUN".into(),
                look: look(5),
                best: 1,
                activity: Activity::Custom,
                party: 5,
                open: true,
                size: 3,
                openings: 13,
            }]),
            ToPc::Party(PartyInfo {
                id: 6,
                leader: 1,
                privacy: Privacy::InviteOnly,
                activity: Activity::Searching,
                playlist: 3,
                members: vec![
                    member.clone(),
                    PartyMember {
                        account: 9,
                        ..member
                    },
                ],
                maps: vec!["lockout".into(), "ascension".into()],
            }),
            ToPc::Invited {
                party: 7,
                from: "KAT".into(),
            },
            ToPc::Status(SearchStatus {
                stage: Stage::WaitingToFill,
                have: 3,
                need: 1,
                seconds: 20,
                low: 2,
                high: 14,
            }),
            ToPc::Match(MatchInfo {
                id: 8,
                playlist: 2,
                ranked: true,
                map: "lockout".into(),
                hash: 77,
                game_type: GameType::TeamOddball,
                preset: "DEFAULT".into(),
                score: 120,
                time_limit: 600,
                bots: 1,
                host: 2,
                countdown: 10,
                players: vec![player.clone(), MatchPlayer { team: 0, ..player }],
            }),
            ToPc::HostMatch(9),
            ToPc::Link(LinkInfo {
                token: [8; 16],
                id: 10,
                end: End::Joiner,
                peer: 2,
                gamertag: "JORGE".into(),
                level: 7,
                team: 1,
                map: "lockout".into(),
            }),
            ToPc::Go(11),
            ToPc::MatchOver(MatchOver {
                id: 12,
                counted: false,
                reason: "THE HOST LEFT. THE GAME DIDN'T COUNT.".into(),
                card: "card".into(),
                levels: vec![(2, 3, 4)],
                players: vec![(2, 4), (5, 50)],
            }),
            ToPc::Notice("THE PARTY IS FULL".into()),
            ToPc::CustomOpen {
                party: 13,
                leader: 14,
                map: "beavercreek".into(),
            },
            ToPc::LauncherMatch(launcher_match()),
            ToPc::Ping(15),
            ToPc::Pong(16),
            ToPc::Linked,
        ]
    }

    #[test]
    fn every_message_reads_back_as_written() {
        let to_server = every_message_to_the_server();
        let mut kinds: Vec<u8> = Vec::new();
        for m in &to_server {
            let (kind, body) = m.write();
            assert_eq!(ToServer::read(kind, &body).as_ref(), Ok(m), "{m:?}");
            kinds.push(kind);
        }
        let to_pc = every_message_to_a_pc();
        for m in &to_pc {
            let (kind, body) = m.write();
            assert_eq!(ToPc::read(kind, &body).as_ref(), Ok(m), "{m:?}");
            kinds.push(kind);
        }
        // Every kind once: pings and pongs once each way, and LOGIN twice
        // (the game's and a launcher's).
        assert_eq!(kinds.len(), 27 + 19);
        kinds.sort_unstable();
        kinds.dedup();
        assert_eq!(kinds.len(), 27 + 19 - 2 - 1);
    }

    #[test]
    fn fan_outs_carry_any_game_message() {
        let messages = [
            ToServer::Fanout {
                to: Some(vec![1, 2, 3]),
                kind: 108,
                body: vec![9; 3000],
            },
            ToServer::Fanout {
                to: None,
                kind: 108,
                body: vec![1, 2],
            },
            ToServer::Fanout {
                to: Some(Vec::new()),
                kind: 107,
                body: Vec::new(),
            },
        ];
        for m in &messages {
            let (kind, body) = m.write();
            assert_eq!(ToServer::read(kind, &body).as_ref(), Ok(m));
        }
        // The same PCs as the last is one byte; each PC named is eight.
        let (_, body) = messages[1].write();
        assert_eq!(body, [SAME_PCS, 108, 1, 2]);
        // Naming more PCs than follow is an error.
        assert!(ToServer::read(kind::FANOUT, &[2, 0, 0, 0, 0, 0, 0, 0, 0, 108]).is_err());
    }

    #[test]
    fn broken_messages_are_errors_not_crashes() {
        for m in every_message_to_the_server() {
            let (kind, body) = m.write();
            // Cut short anywhere, or with something extra.
            for end in 0..body.len() {
                assert!(
                    ToServer::read(kind, &body[..end]).is_err(),
                    "{m:?} cut at {end}"
                );
            }
            // (A fan-out's game message runs to its end, so one with more is
            // another fan-out.)
            let longer = [&body[..], &[0]].concat();
            if kind != kind::FANOUT {
                assert!(ToServer::read(kind, &longer).is_err(), "{m:?} with more");
            }
            // Arbitrary bytes, of many lengths, never panic.
            for len in 0..200 {
                let junk: Vec<u8> = (0..len).map(|i| (i * 37 + len) as u8).collect();
                let _ = ToServer::read(kind, &junk);
            }
        }
        for m in every_message_to_a_pc() {
            let (kind, body) = m.write();
            for end in 0..body.len() {
                assert!(
                    ToPc::read(kind, &body[..end]).is_err(),
                    "{m:?} cut at {end}"
                );
            }
            let longer = [&body[..], &[0]].concat();
            assert!(ToPc::read(kind, &longer).is_err(), "{m:?} with more");
            for len in 0..200 {
                let junk: Vec<u8> = (0..len).map(|i| (i * 37 + len) as u8).collect();
                let _ = ToPc::read(kind, &junk);
            }
        }
        assert!(ToServer::read(99, &[]).is_err());
        assert!(ToPc::read(kind::LOGIN, &[]).is_err());
    }

    #[test]
    fn counts_lengths_and_values_out_of_range_are_refused() {
        let (kind, body) = ToServer::Guests(3).write();
        assert!(ToServer::read(kind, &body).is_ok());
        assert!(ToServer::read(kind, &[4]).is_err());
        assert!(ToServer::read(kind::PRIVACY, &[2]).is_err());
        assert!(ToServer::read(kind::LEFT_MATCH, &[0, 0, 0, 0, 0, 0, 0, 0, 2]).is_err());
        // A gamertag far longer than any name.
        let long = ToServer::Profile {
            gamertag: "X".repeat(MAX_NAME + 1),
            look: look(0),
        };
        let (kind, body) = long.write();
        assert!(ToServer::read(kind, &body).is_err());
        // A party of 17.
        let mut w = Writer::default();
        w.u64(1);
        w.u64(1);
        w.u8(0);
        w.u8(0);
        w.u8(QUICKMATCH);
        w.u8(17);
        assert!(ToPc::read(kind::PARTY, &w.0).is_err());
        // A match result for 17 players.
        let mut w = Writer::default();
        w.u64(1);
        w.u8(17);
        assert!(ToServer::read(kind::RESULT, &w.0).is_err());
        // A game type past the last.
        let ToPc::Match(m) = &every_message_to_a_pc()[8] else {
            panic!("not a match");
        };
        let (kind, mut body) = ToPc::Match(m.clone()).write();
        // id, playlist, ranked, map, hash: then the game type.
        let at = 8 + 1 + 1 + 2 + m.map.len() + 8;
        body[at] = GameType::ALL.len() as u8;
        assert!(ToPc::read(kind, &body).is_err());
        // A playlist playing more maps than any can.
        let ToPc::Playlists(p) = &every_message_to_a_pc()[3] else {
            panic!("not the playlists");
        };
        let most = PlaylistInfo {
            maps: vec!["m".into(); MAX_PLAYLIST_MAPS],
            ..p[0].clone()
        };
        let (kind, mut body) = ToPc::Playlists(vec![most]).write();
        assert!(ToPc::read(kind, &body).is_ok());
        // Its maps come last: how many, then each name ("m", 3 bytes).
        let at = body.len() - 3 * MAX_PLAYLIST_MAPS - 1;
        body[at] += 1;
        body.extend([1, 0, b'm']);
        assert!(ToPc::read(kind, &body).is_err());
        // A match over for 17 players.
        let mut w = Writer::default();
        w.u64(1);
        w.bool(true);
        w.str("");
        w.str("");
        w.u8(0);
        w.u8(17);
        assert!(ToPc::read(kind::MATCH_OVER, &w.0).is_err());
    }

    #[test]
    fn a_login_says_its_protocol_first() {
        let (kind, body) = ToServer::Login(login()).write();
        assert_eq!(kind, kind::LOGIN);
        assert_eq!(
            login_header(&body),
            Some(LoginHeader::Game { protocol: PROTOCOL })
        );
        // An older game's LOGIN is told apart before the rest is read.
        let mut w = Writer::default();
        w.u32(MAGIC);
        w.u32(PROTOCOL - 1);
        w.str("anything at all");
        let older = LoginHeader::Game {
            protocol: PROTOCOL - 1,
        };
        assert_eq!(login_header(&w.0), Some(older));
        assert!(ToServer::read(kind::LOGIN, &w.0).is_err());
        assert_eq!(login_header(b"GET / HTTP/1.1"), None);
        assert_eq!(login_header(&[]), None);
        // A launcher's says the service's version and that it's a
        // launcher, whatever comes after.
        let (_, body) = ToServer::Login(launcher_login()).write();
        let launcher = LoginHeader::Versioned {
            live: LIVE_PROTOCOL,
            client: CLIENT_LAUNCHER,
        };
        assert_eq!(login_header(&body), Some(launcher));
        let mut w = Writer::default();
        w.u32(MAGIC_V2);
        w.u32(LIVE_PROTOCOL + 1);
        w.u8(CLIENT_LAUNCHER);
        w.str("from the future, written some other way");
        let newer = LoginHeader::Versioned {
            live: LIVE_PROTOCOL + 1,
            client: CLIENT_LAUNCHER,
        };
        assert_eq!(login_header(&w.0), Some(newer));
        assert!(ToServer::read(kind::LOGIN, &w.0).is_err());
        assert_eq!(login_header(&w.0[..8]), None);
    }

    #[test]
    fn the_games_login_is_written_as_it_always_was() {
        // Byte for byte as games at PROTOCOL 26 (and before) write it, so
        // they still sign in, and a new game signs in to an old server.
        let l = login();
        let mut w = Writer::default();
        w.u32(MAGIC);
        w.u32(PROTOCOL);
        w.0.extend_from_slice(&l.key);
        w.str(&l.gamertag);
        l.look.write(&mut w);
        w.str(&l.card);
        w.u16(l.maps.len() as u16);
        for (name, hash) in &l.maps {
            w.str(name);
            w.u64(*hash);
        }
        w.u8(l.guests);
        let (kind, body) = ToServer::Login(l.clone()).write();
        assert_eq!((kind, &body), (kind::LOGIN, &w.0));
        assert_eq!(&body[..4], b"H2LV");
        assert_eq!(ToServer::read(kind, &body), Ok(ToServer::Login(l)));
    }

    #[test]
    fn launchers_sign_in_with_the_services_version() {
        let l = launcher_login();
        let (kind, body) = ToServer::Login(l.clone()).write();
        assert_eq!(&body[..4], b"H2L2");
        assert_eq!(&body[4..8], &LIVE_PROTOCOL.to_le_bytes());
        assert_eq!(body[8], CLIENT_LAUNCHER);
        assert_eq!(ToServer::read(kind, &body), Ok(ToServer::Login(l.clone())));
        // Nothing in it is the game's PROTOCOL: after the version, the rest
        // is as the game writes it.
        let as_game = Login {
            client: LoginClient::viewer(),
            ..l.clone()
        };
        let (_, game) = ToServer::Login(as_game).write();
        let version = 2 + "h2launch 0.1.0".len();
        assert_eq!(body[9 + version..], game[8..]);
        // A versioned LOGIN can say it's the game, at any PROTOCOL (the
        // server judges that).
        let mut w = Writer::default();
        w.u32(MAGIC_V2);
        w.u32(LIVE_PROTOCOL);
        w.u8(CLIENT_VIEWER);
        w.u32(PROTOCOL + 7);
        w.0.extend_from_slice(&body[9 + version..]);
        let Ok(ToServer::Login(viewer)) = ToServer::read(kind, &w.0) else {
            panic!("not read");
        };
        let protocol = PROTOCOL + 7;
        assert_eq!(viewer.client, LoginClient::Viewer { protocol });
        assert_eq!(viewer.client.kind(), ClientKind::Viewer);
        assert_eq!((viewer.key, viewer.gamertag), (l.key, l.gamertag.clone()));
        // A kind of program this version doesn't know, and a version
        // longer than any name, are errors.
        w.0[8] = 2;
        assert!(ToServer::read(kind, &w.0).is_err());
        let long = Login {
            client: LoginClient::Launcher {
                version: "v".repeat(MAX_NAME + 1),
            },
            ..l
        };
        let (kind, body) = ToServer::Login(long).write();
        assert!(ToServer::read(kind, &body).is_err());
    }

    #[test]
    fn launcher_results_and_matches_have_their_limits() {
        let mut too_many = launcher_result();
        too_many.team_scores = vec![0; MAX_TEAMS];
        let (kind, mut body) = ToServer::LauncherResult(too_many).write();
        assert!(ToServer::read(kind, &body).is_ok());
        // id, finished, then how many team scores.
        body[9] += 1;
        body.splice(10..10, [0; 4]);
        assert!(ToServer::read(kind, &body).is_err());
        let mut w = Writer::default();
        w.u64(1);
        w.bool(true);
        w.u8(0);
        w.u8(17);
        assert!(ToServer::read(kind::LAUNCHER_RESULT, &w.0).is_err());
        // Finished is a 0 or a 1.
        let (_, mut body) = ToServer::LauncherResult(launcher_result()).write();
        body[8] = 2;
        assert!(ToServer::read(kind::LAUNCHER_RESULT, &body).is_err());
        // A match for 17 players.
        let mut m = launcher_match();
        m.players = vec![m.players[0].clone(); 16];
        let (kind, body) = ToPc::LauncherMatch(m.clone()).write();
        assert!(ToPc::read(kind, &body).is_ok());
        m.players.push(m.players[0].clone());
        let (kind, body) = ToPc::LauncherMatch(m).write();
        // Written with its first 16, so it reads back short.
        let Ok(ToPc::LauncherMatch(read)) = ToPc::read(kind, &body) else {
            panic!("not read");
        };
        assert_eq!(read.players.len(), 16);
    }

    #[test]
    fn map_hashes_tell_copies_apart() {
        let dir = std::env::temp_dir().join(format!("h2net-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            map_hash(&path).unwrap()
        };
        let map: Vec<u8> = (0..5000u32).map(|i| (i * 7) as u8).collect();
        let same = write("a.map", &map);
        assert_eq!(write("b.map", &map), same);
        // A change in the first 2 KiB, or in length, shows.
        let mut changed = map.clone();
        changed[100] ^= 1;
        assert_ne!(write("c.map", &changed), same);
        assert_ne!(write("d.map", &map[..4999]), same);
        // FNV-1a of nothing, then a length of 0.
        let mut empty: u64 = 0xcbf2_9ce4_8422_2325;
        for _ in 0..8 {
            empty = empty.wrapping_mul(0x0000_0100_0000_01b3);
        }
        assert_eq!(write("e.map", &[]), empty);
        assert!(map_hash(&dir.join("missing.map")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
