//! The relay server, run by h2live (or `h2relay` on its own): one UDP
//! socket and one thread, passing frames between the members of each
//! room. It never looks inside a payload.
//!
//! - A hello registers (room, src id) at the address it came from and is
//!   answered with a hello. A hello for a member already there from a new
//!   address moves it (a NAT rebind, or a client that started over).
//! - Data, acks and peer pings from a registered (room, id, address) go to
//!   the member named as the destination, or to every other member for
//!   [`BROADCAST`] (unreliable data and peer pings only). Anything from a
//!   sender not registered at that address is dropped and counted, and the
//!   sender is told [`Refusal::NotMember`] (at most twice a second), so it
//!   says hello again.
//! - A keepalive is answered with a pong; a bye removes the member.
//! - Members not heard from for `member_timeout` (15 s) are removed, and a
//!   room left empty for `room_linger` expires.
//!
//! Limits, so one bad client can't spoil it for the others: members per
//! room (17: Halo 2's 16 players and a spare), rooms, members from one IP
//! address, and a packet rate per source address (a token bucket; a
//! broadcast costs one token per member it goes to).
//!
//! Admission: with [`Admission::Open`] (what h2live uses for now), any room
//! token a client names is made on its first hello, so the room token is
//! the only thing keeping strangers out of a match: anyone who knows it
//! can join (and, knowing a member's id too, take that member's place).
//! With [`Admission::Issued`] only rooms made with [`RelayHandle::issue`]
//! can be joined, which is what h2live should switch to once it hands out
//! the match tokens itself.

use crate::frame::{decode, Frame, Kind, Refusal, BROADCAST, FLAG_PEER, FLAG_SERVER, MAX_FRAME};
use crate::sockets::{self, RecvError};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Socket buffers asked for (bytes): a few thousand full frames, for
/// bursts while the thread is busy.
const BUFFERS: usize = 4 << 20;
/// Source addresses tracked for the rate limit at most; past that, new
/// ones are dropped until quiet ones are forgotten.
const MAX_SOURCES: usize = 1 << 16;
/// A source quiet this long is forgotten.
const SOURCE_IDLE: Duration = Duration::from_secs(10);
/// A sender is told it isn't a member at most this often.
const REFUSE_EVERY: Duration = Duration::from_millis(500);
/// The thread looks at the time (stop, sweeps, commands) at least this
/// often.
const TICK: Duration = Duration::from_millis(100);
/// Datagrams read in one go before the time is looked at again.
const BURST: usize = 1024;

/// Who may open a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// Any room a hello names is made then.
    Open,
    /// Only rooms issued through [`RelayHandle::issue`].
    Issued,
}

/// How a server behaves. The defaults are for real use; tests shorten the
/// times.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub admission: Admission,
    /// Members in one room at most.
    pub max_members: usize,
    /// Rooms at most.
    pub max_rooms: usize,
    /// Members from one IP address at most, across all rooms: room for a
    /// house full of PCs behind one router.
    pub max_members_per_ip: usize,
    /// Packets a second one source address may send, and how many it may
    /// send at once after a quiet spell. An estimate: a 16-player host
    /// sending each peer a packet or two per 30 Hz tick, and its acks,
    /// stays well under it.
    pub rate: f64,
    pub burst: f64,
    /// A member quiet this long is removed.
    pub member_timeout: Duration,
    /// An empty room is kept this long (for members coming back), then
    /// expires.
    pub room_linger: Duration,
    /// A stats line this often (none if zero).
    pub stats_every: Duration,
}

impl Default for ServerConfig {
    fn default() -> ServerConfig {
        ServerConfig {
            admission: Admission::Open,
            max_members: 17,
            max_rooms: 1024,
            max_members_per_ip: 64,
            rate: 3000.0,
            burst: 3000.0,
            member_timeout: Duration::from_secs(15),
            room_linger: Duration::from_secs(60),
            stats_every: Duration::from_secs(60),
        }
    }
}

/// Where log lines go.
pub type Logger = Arc<dyn Fn(&str) + Send + Sync>;

/// What a server has done so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ServerStats {
    /// Rooms and members now.
    pub rooms: u64,
    pub members: u64,
    pub rooms_opened: u64,
    pub rooms_expired: u64,
    /// Datagrams and bytes each way.
    pub frames_in: u64,
    pub bytes_in: u64,
    pub frames_out: u64,
    pub bytes_out: u64,
    /// Frames passed from one member to another (a broadcast once per
    /// member it went to).
    pub forwarded: u64,
    /// Datagrams that weren't well-formed client frames.
    pub malformed: u64,
    /// Frames from senders not registered at that address.
    pub unknown_sender: u64,
    /// Frames to members not in the room.
    pub unknown_dst: u64,
    /// Frames over a source's rate limit.
    pub rate_limited: u64,
    /// Hellos refused.
    pub refused: u64,
    /// Members removed for being quiet.
    pub timeouts: u64,
    /// Sends the socket had no room for, or refused.
    pub send_errors: u64,
}

#[derive(Default)]
struct Counters {
    rooms: AtomicU64,
    members: AtomicU64,
    rooms_opened: AtomicU64,
    rooms_expired: AtomicU64,
    frames_in: AtomicU64,
    bytes_in: AtomicU64,
    frames_out: AtomicU64,
    bytes_out: AtomicU64,
    forwarded: AtomicU64,
    malformed: AtomicU64,
    unknown_sender: AtomicU64,
    unknown_dst: AtomicU64,
    rate_limited: AtomicU64,
    refused: AtomicU64,
    timeouts: AtomicU64,
    send_errors: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> ServerStats {
        let get = |a: &AtomicU64| a.load(Relaxed);
        ServerStats {
            rooms: get(&self.rooms),
            members: get(&self.members),
            rooms_opened: get(&self.rooms_opened),
            rooms_expired: get(&self.rooms_expired),
            frames_in: get(&self.frames_in),
            bytes_in: get(&self.bytes_in),
            frames_out: get(&self.frames_out),
            bytes_out: get(&self.bytes_out),
            forwarded: get(&self.forwarded),
            malformed: get(&self.malformed),
            unknown_sender: get(&self.unknown_sender),
            unknown_dst: get(&self.unknown_dst),
            rate_limited: get(&self.rate_limited),
            refused: get(&self.refused),
            timeouts: get(&self.timeouts),
            send_errors: get(&self.send_errors),
        }
    }
}

fn bump(counter: &AtomicU64, n: u64) {
    counter.fetch_add(n, Relaxed);
}

enum Command {
    Issue(u64),
    Revoke(u64),
}

/// What the server's thread and its handles share.
struct Control {
    stop: AtomicBool,
    /// Set when `commands` has something, so the thread needn't lock to look.
    pending: AtomicBool,
    commands: Mutex<Vec<Command>>,
    counters: Counters,
}

/// Talks to a running server from other threads.
#[derive(Clone)]
pub struct RelayHandle {
    control: Arc<Control>,
    addr: SocketAddr,
}

impl RelayHandle {
    /// Ask the server to stop (within a tenth of a second).
    pub fn stop(&self) {
        self.control.stop.store(true, Relaxed);
    }

    /// Make `room` joinable (for [`Admission::Issued`]); it expires like
    /// any room left empty.
    pub fn issue(&self, room: u64) {
        self.command(Command::Issue(room));
    }

    /// Close `room` now, its members with it.
    pub fn revoke(&self, room: u64) {
        self.command(Command::Revoke(room));
    }

    fn command(&self, command: Command) {
        let mut commands = self
            .control
            .commands
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        commands.push(command);
        self.control.pending.store(true, Relaxed);
    }

    pub fn stats(&self) -> ServerStats {
        self.control.counters.snapshot()
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
}

/// A server running on a thread of its own: stopped when dropped.
pub struct RelayThread {
    handle: RelayHandle,
    thread: Option<JoinHandle<()>>,
}

impl RelayThread {
    pub fn handle(&self) -> &RelayHandle {
        &self.handle
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.handle.addr
    }

    pub fn stats(&self) -> ServerStats {
        self.handle.stats()
    }

    /// Stop it and wait until it has.
    pub fn stop(mut self) {
        self.join();
    }

    fn join(&mut self) {
        self.handle.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for RelayThread {
    fn drop(&mut self) {
        self.join();
    }
}

struct Member {
    addr: SocketAddr,
    heard: Instant,
}

struct Room {
    members: HashMap<u64, Member>,
    opened: Instant,
    /// Since when it's had no members (if it has none).
    empty_since: Option<Instant>,
    /// Members that ever joined.
    joined: u64,
}

impl Room {
    fn new(now: Instant) -> Room {
        Room {
            members: HashMap::new(),
            opened: now,
            empty_since: Some(now),
            joined: 0,
        }
    }
}

/// A source address's token bucket.
struct Source {
    tokens: f64,
    last: Instant,
    refused: Option<Instant>,
}

/// The relay server. Make it with [`RelayServer::bind`], then [`run`] it
/// (blocking) or [`spawn`] it on a thread.
///
/// [`run`]: RelayServer::run
/// [`spawn`]: RelayServer::spawn
pub struct RelayServer {
    socket: UdpSocket,
    addr: SocketAddr,
    config: ServerConfig,
    control: Arc<Control>,
    log: Logger,
    rooms: HashMap<u64, Room>,
    members_per_ip: HashMap<IpAddr, usize>,
    sources: HashMap<SocketAddr, Source>,
    next_sweep: Instant,
    /// When the last stats line was, and the stats then.
    stats_at: Instant,
    stats_then: ServerStats,
}

impl RelayServer {
    pub fn bind(addr: impl ToSocketAddrs, config: ServerConfig) -> io::Result<RelayServer> {
        let mut last = None;
        for addr in addr.to_socket_addrs()? {
            match sockets::bind(addr, BUFFERS) {
                Ok(socket) => return RelayServer::over(socket, config),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no address")))
    }

    fn over(socket: UdpSocket, config: ServerConfig) -> io::Result<RelayServer> {
        let addr = socket.local_addr()?;
        let now = Instant::now();
        Ok(RelayServer {
            socket,
            addr,
            config,
            control: Arc::new(Control {
                stop: AtomicBool::new(false),
                pending: AtomicBool::new(false),
                commands: Mutex::new(Vec::new()),
                counters: Counters::default(),
            }),
            log: Arc::new(|line| println!("{line}")),
            rooms: HashMap::new(),
            members_per_ip: HashMap::new(),
            sources: HashMap::new(),
            next_sweep: now,
            stats_at: now,
            stats_then: ServerStats::default(),
        })
    }

    /// Send log lines to `log` rather than standard output.
    pub fn with_log(mut self, log: Logger) -> RelayServer {
        self.log = log;
        self
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn handle(&self) -> RelayHandle {
        RelayHandle {
            control: self.control.clone(),
            addr: self.addr,
        }
    }

    /// Run it on a thread of its own.
    pub fn spawn(self) -> io::Result<RelayThread> {
        let handle = self.handle();
        let thread = std::thread::Builder::new()
            .name("h2relay".into())
            .spawn(move || self.run())?;
        Ok(RelayThread {
            handle,
            thread: Some(thread),
        })
    }

    /// Serve until a handle says stop.
    pub fn run(mut self) {
        let mut buf = vec![0; 2 * MAX_FRAME];
        let tick = TICK.min(self.sweep_every());
        while !self.control.stop.load(Relaxed) {
            sockets::wait(&self.socket, tick);
            for _ in 0..BURST {
                match self.socket.recv_from(&mut buf) {
                    Ok((n, from)) => self.handle_datagram(&buf[..n], from, Instant::now()),
                    Err(e) => match sockets::recv_error(&e) {
                        RecvError::Empty => break,
                        RecvError::Skip => continue,
                        RecvError::Other => {
                            (self.log)(&format!("relay: receiving: {e}"));
                            std::thread::sleep(Duration::from_millis(10));
                            break;
                        }
                    },
                }
            }
            if self.control.pending.swap(false, Relaxed) {
                self.commands(Instant::now());
            }
            let now = Instant::now();
            if now >= self.next_sweep {
                self.sweep(now);
                self.next_sweep = now + self.sweep_every();
            }
            self.stats_line(now);
        }
    }

    fn sweep_every(&self) -> Duration {
        (self.config.member_timeout / 4).clamp(Duration::from_millis(10), Duration::from_secs(1))
    }

    fn commands(&mut self, now: Instant) {
        let commands = std::mem::take(
            &mut *self
                .control
                .commands
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        );
        for command in commands {
            match command {
                Command::Issue(room) => {
                    if !self.rooms.contains_key(&room) && self.rooms.len() < self.config.max_rooms {
                        self.rooms.insert(room, Room::new(now));
                        bump(&self.control.counters.rooms_opened, 1);
                        (self.log)(&format!("relay: room {room:016x} issued"));
                    }
                }
                Command::Revoke(room) => {
                    if let Some(gone) = self.rooms.remove(&room) {
                        for member in gone.members.values() {
                            self.left(member.addr.ip());
                        }
                        (self.log)(&format!(
                            "relay: room {room:016x} closed after {}, {} joined",
                            minutes(now - gone.opened),
                            gone.joined
                        ));
                    }
                }
            }
        }
        self.count_rooms();
    }

    fn count_rooms(&self) {
        let c = &self.control.counters;
        c.rooms.store(self.rooms.len() as u64, Relaxed);
        let members = self.rooms.values().map(|r| r.members.len() as u64).sum();
        c.members.store(members, Relaxed);
    }

    /// A member from `ip` is gone.
    fn left(&mut self, ip: IpAddr) {
        if let Some(n) = self.members_per_ip.get_mut(&ip) {
            *n -= 1;
            if *n == 0 {
                self.members_per_ip.remove(&ip);
            }
        }
    }

    /// Remove quiet members, expire empty rooms, forget quiet sources.
    fn sweep(&mut self, now: Instant) {
        let control = self.control.clone();
        let c = &control.counters;
        let timeout = self.config.member_timeout;
        let mut gone = Vec::new();
        for room in self.rooms.values_mut() {
            let before = room.members.len();
            room.members.retain(|_, m| {
                let quiet = now.saturating_duration_since(m.heard) >= timeout;
                if quiet {
                    gone.push(m.addr.ip());
                }
                !quiet
            });
            if before > 0 && room.members.is_empty() {
                room.empty_since = Some(now);
            }
        }
        bump(&c.timeouts, gone.len() as u64);
        for ip in gone {
            self.left(ip);
        }
        let linger = self.config.room_linger;
        let expired: Vec<u64> = self
            .rooms
            .iter()
            .filter(|(_, r)| r.empty_since.is_some_and(|t| now - t >= linger))
            .map(|(&id, _)| id)
            .collect();
        for id in expired {
            if let Some(room) = self.rooms.remove(&id) {
                bump(&c.rooms_expired, 1);
                (self.log)(&format!(
                    "relay: room {id:016x} expired after {}, {} joined",
                    minutes(now - room.opened),
                    room.joined
                ));
            }
        }
        self.sources
            .retain(|_, s| now.saturating_duration_since(s.last) < SOURCE_IDLE);
        self.count_rooms();
    }

    /// Say what happened since the last stats line, if anything did.
    fn stats_line(&mut self, now: Instant) {
        let every = self.config.stats_every;
        if every.is_zero() || now - self.stats_at < every {
            return;
        }
        let stats = self.control.counters.snapshot();
        let then = std::mem::replace(&mut self.stats_then, stats);
        self.stats_at = now;
        if stats.frames_in == then.frames_in && stats.rooms == 0 {
            return;
        }
        let d = |f: fn(&ServerStats) -> u64| f(&stats) - f(&then);
        (self.log)(&format!(
            "relay: {} rooms, {} members; in {} frames ({}), out {} ({}); \
             dropped {} malformed, {} from strangers, {} to no one, {} over the rate, \
             {} unsent; {} timed out",
            stats.rooms,
            stats.members,
            d(|s| s.frames_in),
            bytes(d(|s| s.bytes_in)),
            d(|s| s.frames_out),
            bytes(d(|s| s.bytes_out)),
            d(|s| s.malformed),
            d(|s| s.unknown_sender),
            d(|s| s.unknown_dst),
            d(|s| s.rate_limited),
            d(|s| s.send_errors),
            d(|s| s.timeouts),
        ));
    }

    /// Take `cost` tokens from `from`'s bucket, if it has them.
    fn spend(&mut self, from: SocketAddr, cost: f64, now: Instant) -> bool {
        let (rate, burst) = (self.config.rate, self.config.burst);
        if !self.sources.contains_key(&from) && self.sources.len() >= MAX_SOURCES {
            return false;
        }
        let source = self.sources.entry(from).or_insert(Source {
            tokens: burst,
            last: now,
            refused: None,
        });
        let elapsed = now.saturating_duration_since(source.last).as_secs_f64();
        source.tokens = (source.tokens + elapsed * rate).min(burst);
        source.last = now;
        if source.tokens < cost {
            return false;
        }
        source.tokens -= cost;
        true
    }

    fn handle_datagram(&mut self, bytes: &[u8], from: SocketAddr, now: Instant) {
        let c = &self.control.counters;
        bump(&c.frames_in, 1);
        bump(&c.bytes_in, bytes.len() as u64);
        if !self.spend(from, 1.0, now) {
            bump(&self.control.counters.rate_limited, 1);
            return;
        }
        let frame = match decode(bytes) {
            // Only the server makes server frames.
            Ok(frame) if frame.flags & FLAG_SERVER == 0 => frame,
            _ => {
                bump(&self.control.counters.malformed, 1);
                return;
            }
        };
        match frame.kind {
            Kind::Hello => self.hello(&frame, from, now),
            Kind::Bye => self.bye(&frame, from),
            Kind::Ping if frame.flags & FLAG_PEER == 0 => {
                if self.member(&frame, from, now) {
                    let pong = Frame::new(Kind::Pong, frame.room, 0, frame.src)
                        .with_flags(FLAG_SERVER)
                        .with_seq(frame.seq);
                    self.reply(&pong, from);
                }
            }
            Kind::Unreliable | Kind::Reliable | Kind::Ack | Kind::Ping | Kind::Pong => {
                self.forward(&frame, bytes, from, now)
            }
            Kind::Refused => {
                bump(&self.control.counters.malformed, 1);
            }
        }
    }

    /// Whether `frame` is from a member registered at `from` (heard from
    /// now, if so; told it isn't, if not).
    fn member(&mut self, frame: &Frame, from: SocketAddr, now: Instant) -> bool {
        let member = self
            .rooms
            .get_mut(&frame.room)
            .and_then(|r| r.members.get_mut(&frame.src))
            .filter(|m| m.addr == from);
        if let Some(member) = member {
            member.heard = now;
            return true;
        }
        bump(&self.control.counters.unknown_sender, 1);
        let told = self.sources.get_mut(&from).is_some_and(|s| {
            let due = s.refused.is_none_or(|t| now - t >= REFUSE_EVERY);
            if due {
                s.refused = Some(now);
            }
            !due
        });
        if !told {
            self.refuse(frame, Refusal::NotMember, from);
        }
        false
    }

    fn reply(&self, frame: &Frame, to: SocketAddr) {
        if let Ok(bytes) = frame.to_vec() {
            self.send(&bytes, to);
        }
    }

    fn refuse(&self, frame: &Frame, why: Refusal, to: SocketAddr) {
        let refused = Frame::new(Kind::Refused, frame.room, 0, frame.src)
            .with_flags(FLAG_SERVER)
            .with_seq(why.code());
        self.reply(&refused, to);
    }

    fn send(&self, bytes: &[u8], to: SocketAddr) {
        let c = &self.control.counters;
        match self.socket.send_to(bytes, to) {
            Ok(_) => {
                bump(&c.frames_out, 1);
                bump(&c.bytes_out, bytes.len() as u64);
            }
            Err(_) => bump(&c.send_errors, 1),
        }
    }

    fn hello(&mut self, frame: &Frame, from: SocketAddr, now: Instant) {
        let config = &self.config;
        let ip = from.ip();
        let from_ip = self.members_per_ip.get(&ip).copied().unwrap_or(0);
        let refusal = match self.rooms.get(&frame.room) {
            None if config.admission == Admission::Issued => Some(Refusal::NoSuchRoom),
            None if self.rooms.len() >= config.max_rooms => Some(Refusal::TooManyRooms),
            room => {
                let member = room.and_then(|r| r.members.get(&frame.src));
                let members = room.map_or(0, |r| r.members.len());
                match member {
                    // Already here from there: just answer again.
                    Some(m) if m.addr == from => None,
                    // Moving here: one more from this address.
                    Some(_) if from_ip >= config.max_members_per_ip => {
                        Some(Refusal::TooManyFromAddress)
                    }
                    Some(_) => None,
                    None if members >= config.max_members => Some(Refusal::RoomFull),
                    None if from_ip >= config.max_members_per_ip => {
                        Some(Refusal::TooManyFromAddress)
                    }
                    None => None,
                }
            }
        };
        if let Some(why) = refusal {
            bump(&self.control.counters.refused, 1);
            self.refuse(frame, why, from);
            return;
        }
        if let Entry::Vacant(room) = self.rooms.entry(frame.room) {
            room.insert(Room::new(now));
            bump(&self.control.counters.rooms_opened, 1);
            (self.log)(&format!("relay: room {:016x} opened by {from}", frame.room));
        }
        let mut moved_from = None;
        let mut joined = false;
        if let Some(room) = self.rooms.get_mut(&frame.room) {
            match room.members.get_mut(&frame.src) {
                Some(member) => {
                    if member.addr != from {
                        moved_from = Some(member.addr.ip());
                        member.addr = from;
                    }
                    member.heard = now;
                }
                None => {
                    room.members.insert(
                        frame.src,
                        Member {
                            addr: from,
                            heard: now,
                        },
                    );
                    room.empty_since = None;
                    room.joined += 1;
                    joined = true;
                }
            }
        }
        if let Some(old) = moved_from {
            self.left(old);
        }
        if joined || moved_from.is_some() {
            *self.members_per_ip.entry(ip).or_default() += 1;
        }
        let members = self.rooms.get(&frame.room).map_or(0, |r| r.members.len());
        let welcome = Frame::new(Kind::Hello, frame.room, 0, frame.src)
            .with_flags(FLAG_SERVER)
            .with_seq(frame.seq)
            .with_port(members as u32);
        self.reply(&welcome, from);
        self.count_rooms();
    }

    fn bye(&mut self, frame: &Frame, from: SocketAddr) {
        let Some(room) = self.rooms.get_mut(&frame.room) else {
            return;
        };
        if room.members.get(&frame.src).is_some_and(|m| m.addr == from) {
            room.members.remove(&frame.src);
            if room.members.is_empty() {
                room.empty_since = Some(Instant::now());
            }
            self.left(from.ip());
            self.count_rooms();
        }
    }

    /// Pass `frame` (as `bytes`, unchanged) on to its destination.
    fn forward(&mut self, frame: &Frame, bytes: &[u8], from: SocketAddr, now: Instant) {
        if !self.member(frame, from, now) {
            return;
        }
        let Some(room) = self.rooms.get(&frame.room) else {
            return;
        };
        let to: Vec<SocketAddr> = if frame.dst == BROADCAST {
            room.members
                .iter()
                .filter(|(&id, _)| id != frame.src)
                .map(|(_, m)| m.addr)
                .collect()
        } else {
            match room.members.get(&frame.dst) {
                Some(m) => vec![m.addr],
                None => {
                    bump(&self.control.counters.unknown_dst, 1);
                    return;
                }
            }
        };
        // A broadcast costs one token per member it goes to (one was
        // spent already).
        let extra = to.len().saturating_sub(1) as f64;
        if extra > 0.0 && !self.spend(from, extra, now) {
            bump(&self.control.counters.rate_limited, 1);
            return;
        }
        bump(&self.control.counters.forwarded, to.len() as u64);
        for addr in to {
            self.send(bytes, addr);
        }
    }
}

/// A duration as minutes and seconds, as "12m05s".
fn minutes(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}m{:02}s", s / 60, s % 60)
}

/// A byte count, readably.
fn bytes(n: u64) -> String {
    match n {
        0..1_000 => format!("{n} B"),
        1_000..1_000_000 => format!("{:.1} kB", n as f64 / 1e3),
        _ => format!("{:.1} MB", n as f64 / 1e6),
    }
}
