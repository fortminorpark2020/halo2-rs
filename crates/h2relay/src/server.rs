//! The relay server, run by h2live (or `h2relay` on its own): one UDP
//! socket and one thread, passing frames between the members of each
//! room. It never looks inside a payload.
//!
//! - Joining takes two hellos. The first is answered with a challenge
//!   carrying a cookie made from the address it came from (see the `keys`
//!   module), and nothing is kept. The second brings the cookie back from
//!   that address: only then is (room, src id) registered at that address
//!   and answered with a hello. So the server never sends anything to an
//!   address that didn't ask (a forged source address gets one challenge
//!   per hello, smaller than the hello), and no one can sign up an address
//!   that isn't theirs. A checked hello for a member already there from a
//!   new address moves it (a NAT rebind, or a client that started over).
//! - Data, acks and peer pings from a registered (room, id, address) go to
//!   the member named as the destination, or to every other member for
//!   [`BROADCAST`] (unreliable data and peer pings only). Anything from a
//!   sender not registered at that address is dropped and counted, and the
//!   sender is told [`Refusal::NotMember`] (a few times a second at most),
//!   so it says hello again.
//! - A keepalive is answered with a pong; a bye removes the member.
//! - Members not heard from for `member_timeout` (15 s) are removed, and a
//!   room left empty for `room_linger` expires (an open room only one
//!   member ever joined, at once).
//!
//! Admission: rooms h2live issued ([`RelayHandle::issue`]) can only be
//! joined by the players it gave member keys to ([`RelayHandle::member_key`],
//! handed to each player over its signed-in connection): a hello into an
//! issued room must prove the key for its id, so a player can't take over
//! another's id, and no one can make up ids to fill the room. With
//! [`Admission::Issued`] (what h2live runs) those are the only rooms. With
//! [`Admission::Open`] (the standalone `h2relay`, for tests on one PC or a
//! LAN) a hello naming any other room makes it, and anyone who knows its
//! token can join it, or take the place of a member whose id they know.
//!
//! Limits, so one bad client can't spoil it for the others: members per
//! room (17: Halo 2's 16 players and a spare), rooms, members from one IP
//! address, a packet rate per member (a token bucket; a broadcast costs one
//! token per member it goes to), and for everyone else, by IP address: a
//! hello rate, a refusal rate, and (open rooms only) a rate of new rooms.
//! What's kept about senders that aren't members lives in a fixed table
//! (addresses hashed to slots, with a key the server picks), so no flood of
//! addresses can fill anything up.

use crate::frame::{
    decode, Frame, Kind, Refusal, BROADCAST, COOKIE_LEN, FLAG_PEER, FLAG_SERVER, MAX_FRAME,
};
use crate::keys::{self, MemberKey};
use crate::log::{stdout_logger, Logger};
use crate::sockets::{self, RecvError};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};
use std::io;
use std::net::{IpAddr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Socket buffers asked for (bytes): a few thousand full frames, for
/// bursts while the thread is busy.
const BUFFERS: usize = 4 << 20;
/// Slots in the table of what's kept about senders that aren't members.
/// Addresses that land in one slot share it.
const GATES: usize = 4096;
/// "Not a member" refusals one IP address is sent a second at most (and
/// at once, after a quiet spell): enough for a house full of PCs the relay
/// forgot all at once (it restarted, say) to hear soon.
const REFUSALS: f64 = 20.0;
/// How long one cookie is made for; the one before is good too, so a
/// cookie lasts this long at least and twice it at most.
const COOKIE_STEP: u64 = 30;
/// The thread looks at the time (stop, sweeps, commands) at least this
/// often.
const TICK: Duration = Duration::from_millis(100);
/// Datagrams read in one go before the time is looked at again.
const BURST: usize = 1024;
/// Receive errors are logged at most this often.
const ERROR_LOG_EVERY: Duration = Duration::from_secs(10);

/// Which rooms can be joined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// Rooms issued through [`RelayHandle::issue`], and any other room a
    /// hello names (made then, open to anyone who knows its token).
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
    /// Members from one IP address (a /64 for IPv6) at most, across all
    /// rooms: room for a house full of PCs behind one router.
    pub max_members_per_ip: usize,
    /// Packets a second one member may send, and how many it may send at
    /// once after a quiet spell. An estimate: a 16-player host sending each
    /// peer a packet or two per 30 Hz tick, and its acks, stays well under
    /// it. Check it against real 16-player traffic.
    pub rate: f64,
    pub burst: f64,
    /// Hellos a second from one IP address (a /64 for IPv6), and at once:
    /// everyone joining from behind one router shares them. An estimate: a
    /// client joining sends two hellos per attempt, four attempts a second.
    pub hello_rate: f64,
    pub hello_burst: f64,
    /// Open admission: rooms one IP address may open a minute, and at once
    /// after a quiet spell. An estimate, generous for tests.
    pub rooms_per_ip: f64,
    /// A member quiet this long is removed.
    pub member_timeout: Duration,
    /// An empty room is kept this long (for members coming back), then
    /// expires; an open room only one member ever joined expires at once.
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
            hello_rate: 100.0,
            hello_burst: 200.0,
            rooms_per_ip: 10.0,
            member_timeout: Duration::from_secs(15),
            room_linger: Duration::from_secs(60),
            stats_every: Duration::from_secs(60),
        }
    }
}

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
    /// Frames over a member's rate, or hellos over an address's.
    pub rate_limited: u64,
    /// Hellos answered with a challenge (no cookie, or not a good one).
    pub challenged: u64,
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
    challenged: AtomicU64,
    refused: AtomicU64,
    timeouts: AtomicU64,
    send_errors: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> ServerStats {
        let get = |a: &AtomicU64| a.load(Ordering::Relaxed);
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
            challenged: get(&self.challenged),
            refused: get(&self.refused),
            timeouts: get(&self.timeouts),
            send_errors: get(&self.send_errors),
        }
    }
}

fn bump(counter: &AtomicU64, n: u64) {
    counter.fetch_add(n, Ordering::Relaxed);
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
    /// What cookies and member keys are made with: random, picked when the
    /// server starts, never sent anywhere.
    secret: [u8; 32],
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
        self.control.stop.store(true, Ordering::Relaxed);
    }

    /// Make `room` joinable by the players given member keys for it
    /// ([`RelayHandle::member_key`]). Issue it before handing the keys out:
    /// hellos that come after this returns find it. It expires like any
    /// room left empty for `room_linger`; issuing it again starts that wait
    /// over.
    pub fn issue(&self, room: u64) {
        self.command(Command::Issue(room));
    }

    /// Close `room` now: its members are told, and it can't be joined (or,
    /// with open admission, made again) until `room_linger` has passed or
    /// it's issued again.
    pub fn revoke(&self, room: u64) {
        self.command(Command::Revoke(room));
    }

    /// Member `id`'s key for the issued room `room`: for h2live to give to
    /// that player only (over its signed-in connection), and for the
    /// launcher to give its client (`ClientConfig::member_key`). Good for
    /// as long as this server runs.
    pub fn member_key(&self, room: u64, id: u64) -> MemberKey {
        keys::member_key(&self.control.secret, room, id)
    }

    fn command(&self, command: Command) {
        let mut commands = self
            .control
            .commands
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        commands.push(command);
        self.control.pending.store(true, Ordering::Release);
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

/// A token bucket, full until first used.
#[derive(Default)]
struct Bucket {
    tokens: f64,
    last: Option<Instant>,
}

impl Bucket {
    /// Take `cost` tokens, if there are that many, filling up at `rate` a
    /// second to `burst` since last time.
    fn take(&mut self, rate: f64, burst: f64, cost: f64, now: Instant) -> bool {
        let tokens = match self.last {
            None => burst,
            Some(last) => {
                let elapsed = now.saturating_duration_since(last).as_secs_f64();
                (self.tokens + elapsed * rate).min(burst)
            }
        };
        self.last = Some(now);
        let enough = tokens >= cost;
        self.tokens = if enough { tokens - cost } else { tokens };
        enough
    }
}

/// What's kept about the senders (by IP address) in one slot of the table
/// for those that aren't members.
#[derive(Default)]
struct Gate {
    hellos: Bucket,
    refusals: Bucket,
    rooms: Bucket,
}

struct Member {
    addr: SocketAddr,
    heard: Instant,
    bucket: Bucket,
}

struct Room {
    members: HashMap<u64, Member>,
    opened: Instant,
    /// Since when it's had no members (if it has none).
    empty_since: Option<Instant>,
    /// Members that ever joined.
    joined: u64,
    /// Issued by h2live: hellos must prove the member key.
    issued: bool,
}

impl Room {
    fn new(now: Instant, issued: bool) -> Room {
        Room {
            members: HashMap::new(),
            opened: now,
            empty_since: Some(now),
            joined: 0,
            issued,
        }
    }
}

/// What one IP address counts as for the limits: itself (an IPv4 address
/// written as IPv6 as the IPv4 one), or for IPv6 its /64, as one host can
/// use any address in that.
fn ip_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let s = v6.segments();
            Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0).into()
        }
        v4 => v4,
    }
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
    /// Rooms revoked lately, and when.
    revoked: HashMap<u64, Instant>,
    members_per_ip: HashMap<IpAddr, usize>,
    gates: Vec<Gate>,
    gate_hasher: RandomState,
    started: Instant,
    next_sweep: Instant,
    /// When the last stats line was, and the stats then.
    stats_at: Instant,
    stats_then: ServerStats,
    /// Receive errors not yet logged, and when one last was.
    errors: u64,
    error_logged: Option<Instant>,
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
                secret: keys::random_secret()?,
            }),
            log: stdout_logger(),
            rooms: HashMap::new(),
            revoked: HashMap::new(),
            members_per_ip: HashMap::new(),
            gates: (0..GATES).map(|_| Gate::default()).collect(),
            gate_hasher: RandomState::new(),
            started: now,
            next_sweep: now,
            stats_at: now,
            stats_then: ServerStats::default(),
            errors: 0,
            error_logged: None,
        })
    }

    /// Send log lines to `log` rather than standard output. It's called on
    /// the relay's thread, so it must never wait ([`crate::background_logger`]
    /// makes one that doesn't).
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
        while !self.control.stop.load(Ordering::Relaxed) {
            sockets::wait(&self.socket, tick);
            for _ in 0..BURST {
                match self.socket.recv_from(&mut buf) {
                    Ok((n, from)) => {
                        let now = Instant::now();
                        // What h2live asked for first, so a hello sent
                        // after `issue` returned finds its room.
                        self.commands(now);
                        self.handle_datagram(&buf[..n], from, now);
                    }
                    Err(e) => match sockets::recv_error(&e) {
                        RecvError::Empty => break,
                        RecvError::Skip => continue,
                        RecvError::Other => {
                            self.receive_error(&e, Instant::now());
                            std::thread::sleep(Duration::from_millis(10));
                            break;
                        }
                    },
                }
            }
            let now = Instant::now();
            self.commands(now);
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

    /// Log a receive error, but not more than one line every so often.
    fn receive_error(&mut self, e: &io::Error, now: Instant) {
        self.errors += 1;
        if self
            .error_logged
            .is_some_and(|t| now.saturating_duration_since(t) < ERROR_LOG_EVERY)
        {
            return;
        }
        let more = match self.errors {
            1 => String::new(),
            n => format!(" ({n} errors since the last said)"),
        };
        (self.log)(&format!("relay: receiving: {e}{more}"));
        self.errors = 0;
        self.error_logged = Some(now);
    }

    /// Do what the handles asked, if they asked anything.
    fn commands(&mut self, now: Instant) {
        if !self.control.pending.load(Ordering::Acquire)
            || !self.control.pending.swap(false, Ordering::AcqRel)
        {
            return;
        }
        let commands = std::mem::take(
            &mut *self
                .control
                .commands
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        );
        for command in commands {
            match command {
                Command::Issue(room) => self.issue(room, now),
                Command::Revoke(room) => self.revoke(room, now),
            }
        }
        self.count_rooms();
    }

    fn issue(&mut self, id: u64, now: Instant) {
        self.revoked.remove(&id);
        if let Some(room) = self.rooms.get_mut(&id) {
            room.issued = true;
            if room.members.is_empty() {
                room.empty_since = Some(now);
            }
        } else if self.rooms.len() < self.config.max_rooms {
            self.rooms.insert(id, Room::new(now, true));
            bump(&self.control.counters.rooms_opened, 1);
            (self.log)(&format!("relay: room {id:016x} issued"));
        } else {
            (self.log)(&format!(
                "relay: room {id:016x} not issued: the relay has its most rooms ({})",
                self.config.max_rooms
            ));
        }
    }

    fn revoke(&mut self, id: u64, now: Instant) {
        self.revoked.insert(id, now);
        let Some(gone) = self.rooms.remove(&id) else {
            return;
        };
        for (&member, m) in &gone.members {
            self.left(m.addr.ip());
            // So it knows now, not when it next sends something.
            let told = Frame::new(Kind::Refused, id, 0, member)
                .with_flags(FLAG_SERVER)
                .with_seq(Refusal::NoSuchRoom.code());
            self.reply(&told, m.addr);
        }
        (self.log)(&format!(
            "relay: room {id:016x} closed after {}, {} joined",
            minutes(now.saturating_duration_since(gone.opened)),
            gone.joined
        ));
    }

    fn count_rooms(&self) {
        let c = &self.control.counters;
        c.rooms.store(self.rooms.len() as u64, Ordering::Relaxed);
        let members = self.rooms.values().map(|r| r.members.len() as u64).sum();
        c.members.store(members, Ordering::Relaxed);
    }

    /// A member from `ip` is gone.
    fn left(&mut self, ip: IpAddr) {
        let ip = ip_key(ip);
        if let Some(n) = self.members_per_ip.get_mut(&ip) {
            *n -= 1;
            if *n == 0 {
                self.members_per_ip.remove(&ip);
            }
        }
    }

    /// Remove quiet members, expire empty rooms, forget old revocations.
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
        // An open room no one else ever joined isn't kept for anyone: so
        // one address can't keep the table full by opening and leaving.
        let linger = |room: &Room| {
            if room.issued || room.joined > 1 {
                linger
            } else {
                Duration::ZERO
            }
        };
        let expired: Vec<u64> = self
            .rooms
            .iter()
            .filter(|(_, r)| {
                r.empty_since
                    .is_some_and(|t| now.saturating_duration_since(t) >= linger(r))
            })
            .map(|(&id, _)| id)
            .collect();
        for id in expired {
            if let Some(room) = self.rooms.remove(&id) {
                bump(&c.rooms_expired, 1);
                (self.log)(&format!(
                    "relay: room {id:016x} expired after {}, {} joined",
                    minutes(now.saturating_duration_since(room.opened)),
                    room.joined
                ));
            }
        }
        let room_linger = self.config.room_linger;
        self.revoked
            .retain(|_, t| now.saturating_duration_since(*t) < room_linger);
        self.count_rooms();
    }

    /// Say what happened since the last stats line, if anything did.
    fn stats_line(&mut self, now: Instant) {
        let every = self.config.stats_every;
        if every.is_zero() || now.saturating_duration_since(self.stats_at) < every {
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
             {} unsent; {} hellos challenged, {} refused; {} timed out",
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
            d(|s| s.challenged),
            d(|s| s.refused),
            d(|s| s.timeouts),
        ));
    }

    /// The slot of the table for senders that aren't members that `ip`
    /// belongs in.
    fn gate(&self, ip: IpAddr) -> usize {
        (self.gate_hasher.hash_one(ip_key(ip)) % GATES as u64) as usize
    }

    fn handle_datagram(&mut self, bytes: &[u8], from: SocketAddr, now: Instant) {
        let c = &self.control.counters;
        bump(&c.frames_in, 1);
        bump(&c.bytes_in, bytes.len() as u64);
        // Checked before anything is looked up or kept for it, so junk
        // costs no more than this.
        let frame = match decode(bytes) {
            // Only the server makes server frames.
            Ok(frame) if frame.flags & FLAG_SERVER == 0 => frame,
            _ => {
                bump(&c.malformed, 1);
                return;
            }
        };
        match frame.kind {
            Kind::Hello => self.hello(&frame, from, now),
            Kind::Bye => self.bye(&frame, from, now),
            Kind::Ping if frame.flags & FLAG_PEER == 0 => {
                if self.member(&frame, from, now, 1) {
                    let pong = Frame::new(Kind::Pong, frame.room, 0, frame.src)
                        .with_flags(FLAG_SERVER)
                        .with_seq(frame.seq);
                    self.reply(&pong, from);
                }
            }
            Kind::Unreliable | Kind::Reliable | Kind::Ack | Kind::Ping | Kind::Pong => {
                self.forward(&frame, bytes, from, now)
            }
            // Only the server sends these (`decode` passes them only with
            // its flag, refused above).
            Kind::Refused | Kind::Challenge => bump(&c.malformed, 1),
        }
    }

    /// Whether `frame` is from a member registered at `from` with `cost`
    /// tokens to spend (heard from now, and they're spent, if so; told it
    /// isn't a member, if not).
    fn member(&mut self, frame: &Frame, from: SocketAddr, now: Instant, cost: usize) -> bool {
        let (rate, burst) = (self.config.rate, self.config.burst);
        let member = self
            .rooms
            .get_mut(&frame.room)
            .and_then(|r| r.members.get_mut(&frame.src))
            .filter(|m| m.addr == from);
        let Some(member) = member else {
            self.stranger(frame, from, now);
            return false;
        };
        member.heard = now;
        if member.bucket.take(rate, burst, cost as f64, now) {
            return true;
        }
        bump(&self.control.counters.rate_limited, 1);
        false
    }

    /// `frame` came from someone not registered at `from`: count it, and
    /// tell them now and then, so they say hello again.
    fn stranger(&mut self, frame: &Frame, from: SocketAddr, now: Instant) {
        bump(&self.control.counters.unknown_sender, 1);
        let gate = self.gate(from.ip());
        if self.gates[gate].refusals.take(REFUSALS, REFUSALS, 1.0, now) {
            self.refuse(frame, Refusal::NotMember, from);
        }
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

    /// The cookie time step `now` is in.
    fn cookie_step(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.started).as_secs() / COOKIE_STEP
    }

    fn hello(&mut self, frame: &Frame, from: SocketAddr, now: Instant) {
        let gate = self.gate(from.ip());
        let (rate, burst) = (self.config.hello_rate, self.config.hello_burst);
        if !self.gates[gate].hellos.take(rate, burst, 1.0, now) {
            bump(&self.control.counters.rate_limited, 1);
            return;
        }
        // `decode` made sure it's a cookie and a proof.
        let (cookie, proof) = frame.payload.split_at(COOKIE_LEN);
        let step = self.cookie_step(now);
        let secret = &self.control.secret;
        let made = |step: u64| keys::cookie(secret, step, from, frame.room, frame.src);
        let fresh = made(step);
        let good = keys::same(cookie, &fresh) || (step > 0 && keys::same(cookie, &made(step - 1)));
        if !good {
            // Nothing kept, and an answer smaller than the hello: a forged
            // source address gets no more than whoever forged it sent.
            bump(&self.control.counters.challenged, 1);
            let challenge = Frame::new(Kind::Challenge, frame.room, 0, frame.src)
                .with_flags(FLAG_SERVER)
                .with_seq(frame.seq)
                .with_payload(&fresh);
            self.reply(&challenge, from);
            return;
        }
        // From here on, the sender is known to get what's sent to `from`.
        if let Some(why) = self.admit(frame, from, cookie, proof, gate, now) {
            bump(&self.control.counters.refused, 1);
            self.refuse(frame, why, from);
            return;
        }
        if let Entry::Vacant(room) = self.rooms.entry(frame.room) {
            room.insert(Room::new(now, false));
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
                            bucket: Bucket::default(),
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
            *self.members_per_ip.entry(ip_key(from.ip())).or_default() += 1;
        }
        let members = self.rooms.get(&frame.room).map_or(0, |r| r.members.len());
        let welcome = Frame::new(Kind::Hello, frame.room, 0, frame.src)
            .with_flags(FLAG_SERVER)
            .with_seq(frame.seq)
            .with_port(members as u32);
        self.reply(&welcome, from);
        self.count_rooms();
    }

    /// Whether a hello with a good cookie may join (or move), and if not,
    /// why. Opening an open room takes one of the opener's new-room tokens.
    fn admit(
        &mut self,
        frame: &Frame,
        from: SocketAddr,
        cookie: &[u8],
        proof: &[u8],
        gate: usize,
        now: Instant,
    ) -> Option<Refusal> {
        let config = &self.config;
        let revoked = self.revoked.get(&frame.room);
        if revoked.is_some_and(|t| now.saturating_duration_since(*t) < config.room_linger) {
            return Some(Refusal::NoSuchRoom);
        }
        let from_ip = self
            .members_per_ip
            .get(&ip_key(from.ip()))
            .copied()
            .unwrap_or(0);
        let Some(room) = self.rooms.get(&frame.room) else {
            if config.admission == Admission::Issued {
                return Some(Refusal::NoSuchRoom);
            }
            if self.rooms.len() >= config.max_rooms {
                return Some(Refusal::TooManyRooms);
            }
            if from_ip >= config.max_members_per_ip {
                return Some(Refusal::TooManyFromAddress);
            }
            let per_minute = config.rooms_per_ip;
            if !self.gates[gate]
                .rooms
                .take(per_minute / 60.0, per_minute, 1.0, now)
            {
                return Some(Refusal::TooManyFromAddress);
            }
            return None;
        };
        if room.issued {
            let key = keys::member_key(&self.control.secret, frame.room, frame.src);
            if !keys::same(proof, &key.proof(frame.room, frame.src, cookie)) {
                return Some(Refusal::BadKey);
            }
        }
        match room.members.get(&frame.src) {
            // Already here from there: just answer again.
            Some(m) if m.addr == from => None,
            // Moving here: one more from this address.
            Some(_) if from_ip >= config.max_members_per_ip => Some(Refusal::TooManyFromAddress),
            Some(_) => None,
            None if room.members.len() >= config.max_members => Some(Refusal::RoomFull),
            None if from_ip >= config.max_members_per_ip => Some(Refusal::TooManyFromAddress),
            None => None,
        }
    }

    fn bye(&mut self, frame: &Frame, from: SocketAddr, now: Instant) {
        let Some(room) = self.rooms.get_mut(&frame.room) else {
            return;
        };
        if room.members.get(&frame.src).is_some_and(|m| m.addr == from) {
            room.members.remove(&frame.src);
            if room.members.is_empty() {
                room.empty_since = Some(now);
            }
            self.left(from.ip());
            self.count_rooms();
        }
    }

    /// Pass `frame` (as `bytes`, unchanged) on to its destination.
    fn forward(&mut self, frame: &Frame, bytes: &[u8], from: SocketAddr, now: Instant) {
        let room = self.rooms.get(&frame.room);
        let sender = room.and_then(|r| r.members.get(&frame.src));
        let Some(room) = room.filter(|_| sender.is_some_and(|m| m.addr == from)) else {
            self.stranger(frame, from, now);
            return;
        };
        let to: Option<Vec<SocketAddr>> = if frame.dst == BROADCAST {
            let others = room.members.iter().filter(|(&id, _)| id != frame.src);
            Some(others.map(|(_, m)| m.addr).collect())
        } else {
            room.members.get(&frame.dst).map(|m| vec![m.addr])
        };
        // One token per member it goes to (and one for a frame to no one).
        let cost = to.as_ref().map_or(1, |to| to.len().max(1));
        if !self.member(frame, from, now, cost) {
            return;
        }
        let Some(to) = to else {
            bump(&self.control.counters.unknown_dst, 1);
            return;
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_fill_at_their_rate_up_to_their_burst() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut bucket = Bucket::default();
        // Full at first.
        assert!(bucket.take(10.0, 3.0, 3.0, at(0)));
        assert!(!bucket.take(10.0, 3.0, 1.0, at(0)));
        // A tenth of a second: one more.
        assert!(bucket.take(10.0, 3.0, 1.0, at(100)));
        assert!(!bucket.take(10.0, 3.0, 1.0, at(100)));
        // Never more than the burst, however long it waits.
        assert!(!bucket.take(10.0, 3.0, 4.0, at(60_000)));
        assert!(bucket.take(10.0, 3.0, 3.0, at(60_000)));
    }

    #[test]
    fn addresses_count_by_host() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert_eq!(ip_key(ip("203.0.113.9")), ip("203.0.113.9"));
        // An IPv4 address on a dual-stack socket is that address.
        assert_eq!(ip_key(ip("::ffff:203.0.113.9")), ip("203.0.113.9"));
        // An IPv6 host is its /64.
        assert_eq!(
            ip_key(ip("2001:db8:1:2:aaaa:bbbb:cccc:dddd")),
            ip("2001:db8:1:2::")
        );
        assert_ne!(ip_key(ip("2001:db8:1:3::1")), ip_key(ip("2001:db8:1:2::1")));
    }
}
