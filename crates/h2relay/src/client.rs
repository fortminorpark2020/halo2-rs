//! The launcher's end of the relay: one [`RelayClient`] per match, which
//! the engine's three host network functions call into.
//!
//! - `send_unreliable` (the engine's unreliable send) encodes a frame and
//!   hands it straight to a non-blocking UDP socket: no thread hop, no
//!   added delay. A full socket buffer drops it and counts it, as the
//!   network would.
//! - `send_reliable` (the engine's reliable send) does the same and also
//!   keeps the frame until the receiver acks it (see "Reliable" below).
//! - `recv` (the engine's receive) pops what a background thread received;
//!   it never waits.
//!
//! The background thread owns the receiving side and the timers: it says
//! hello to the relay server until the server answers (and gives up, as
//! [`State::Failed`], after `hello_timeout`), sends a keepalive to the
//! server and a ping to every other member each second, resends reliable
//! frames, and says bye when the client is dropped. If the server forgets
//! the client (a restart, or its address changed) it answers with
//! [`Refusal::NotMember`] and the client says hello again by itself.
//!
//! Reliable: each (sender, receiver) pair has its own stream. A stream
//! starts with a SYN frame (a reliable frame with [`FLAG_SYN`] and no
//! payload) whose seq is a random number; the data frames that follow
//! number on from it. The receiver acks every reliable frame with its seq
//! and the next seq it expects, holds frames that arrive early (up to
//! `RECV_WINDOW` ahead), and hands them out in order, each once. A SYN
//! with a new number means the sender started over (a new client), so the
//! receiver starts over too. The sender resends a frame after
//! max(2 x smoothed round trip, 50 ms), doubling each time up to a second,
//! and after `max_resends` gives up on the stream: what was waiting is
//! dropped (and counted) and the next reliable send opens a new stream.
//! Round trips come from acks of frames sent once (Karn's rule) and from
//! the pings, so a peer seen only through unreliable traffic has one too.
//!
//! For testing, the lag hooks ([`Lag`], from H2_RELAY_LAG_MS,
//! H2_RELAY_JITTER_MS and H2_RELAY_LOSS) hold back or lose every frame
//! this client sends. Jitter may reorder frames, as a real network does.

use crate::frame::{
    decode, seq_after, Frame, Kind, Refusal, BROADCAST, FLAG_PEER, FLAG_SERVER, FLAG_SYN,
    MAX_FRAME, MAX_PAYLOAD,
};
use crate::rng::Rng;
use crate::sockets::{self, RecvError};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::fmt;
use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Socket buffers asked for (bytes): a few hundred full frames.
const BUFFERS: usize = 1 << 20;
/// Reliable frames a receiver holds that arrived ahead of one missing.
/// At least the sender's window, so nothing in flight is ever refused.
const RECV_WINDOW: usize = 1024;
/// A reliable frame up to this far behind the next one expected is a
/// duplicate (acked again so the sender stops resending it); further
/// behind, it's from some other stream and dropped.
const DUPLICATE_WINDOW: i32 = 4096;
/// Peers tracked at most (a match has 16 machines at most): frames from
/// more are still delivered, but get no stats or reliable stream.
const MAX_PEERS: usize = 64;
/// Frames the lag hooks may hold at once.
const MAX_DELAYED: usize = 8192;
/// Peer pings remembered, to match their pongs to.
const PINGS_KEPT: usize = 8;
/// The background thread looks at its timers at least this often; with
/// the lag hooks on, as often as `LAG_NAP`, to send held frames on time.
const NAP: Duration = Duration::from_millis(10);
const LAG_NAP: Duration = Duration::from_millis(1);
/// Datagrams read in one go before the timers are looked at again.
const BURST: usize = 256;

/// Test hooks: how much worse to make the network seem, for every frame
/// this client sends.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Lag {
    /// Added to every frame on its way out.
    pub delay: Duration,
    /// Up to this much more, at random (frames may then pass each other).
    pub jitter: Duration,
    /// The share of frames lost (0 to 1).
    pub loss: f64,
}

impl Lag {
    /// From H2_RELAY_LAG_MS (delay), H2_RELAY_JITTER_MS and H2_RELAY_LOSS
    /// (0 to 1); none if they're not set.
    pub fn from_env() -> Lag {
        Lag::from_vars(|name| std::env::var(name).ok())
    }

    /// The same, from `var` (the environment's, say).
    fn from_vars(var: impl Fn(&str) -> Option<String>) -> Lag {
        let var = |name: &str| {
            var(name)
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|v| v.is_finite())
                .unwrap_or(0.0)
        };
        let ms = |v: f64| Duration::from_secs_f64(v.clamp(0.0, 60_000.0) / 1000.0);
        Lag {
            delay: ms(var("H2_RELAY_LAG_MS")),
            jitter: ms(var("H2_RELAY_JITTER_MS")),
            loss: var("H2_RELAY_LOSS").clamp(0.0, 1.0),
        }
    }

    /// Nothing is held back or lost.
    pub fn is_none(&self) -> bool {
        self.delay.is_zero() && self.jitter.is_zero() && self.loss <= 0.0
    }
}

/// How a client behaves. The defaults are for real use; tests shorten the
/// times.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Hello is sent this often until the server answers...
    pub hello_every: Duration,
    /// ...and the client gives up ([`Failure::Timeout`]) after this long.
    pub hello_timeout: Duration,
    /// A keepalive to the server, and a ping to the other members, this
    /// often. The server forgets a member quiet for 15 s.
    pub keepalive_every: Duration,
    /// Nothing from the server for this long: say hello again (and give up
    /// after `hello_timeout` more).
    pub server_silence: Duration,
    /// The least and most a reliable frame waits before it's resent.
    pub min_rto: Duration,
    pub max_rto: Duration,
    /// The round trip to assume to a peer with none measured yet (unless
    /// the server's is known: then twice that, both legs through it).
    pub initial_rtt: Duration,
    /// Resends of one reliable frame before giving up on its stream.
    pub max_resends: u32,
    /// Reliable frames to one peer not yet acked, at most; more are
    /// refused ([`SendError::Full`]) and counted.
    pub send_window: usize,
    /// Received packets waiting for `recv`, at most; more are dropped
    /// (reliable ones aren't acked, so they come again).
    pub recv_queue: usize,
    pub lag: Lag,
}

impl Default for ClientConfig {
    fn default() -> ClientConfig {
        ClientConfig {
            hello_every: Duration::from_millis(250),
            hello_timeout: Duration::from_secs(10),
            keepalive_every: Duration::from_secs(1),
            server_silence: Duration::from_secs(10),
            min_rto: Duration::from_millis(50),
            max_rto: Duration::from_secs(1),
            initial_rtt: Duration::from_millis(100),
            max_resends: 20,
            send_window: 256,
            recv_queue: 4096,
            lag: Lag::default(),
        }
    }
}

impl ClientConfig {
    /// The defaults, with the lag hooks from the environment.
    pub fn from_env() -> ClientConfig {
        ClientConfig {
            lag: Lag::from_env(),
            ..ClientConfig::default()
        }
    }
}

/// Where a client is with the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Saying hello (at the start, or again after the server forgot it).
    Joining,
    /// A member of its room: what it sends is passed on.
    Joined,
    /// Gave up: nothing more is sent.
    Failed(Failure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// The server never answered hello.
    Timeout,
    /// The server said no.
    Refused(Refusal),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Failure::Timeout => write!(f, "the relay didn't answer"),
            Failure::Refused(why) => write!(f, "the relay refused: {why}"),
        }
    }
}

/// Why a send didn't go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// Over [`MAX_PAYLOAD`] (its length).
    TooBig(usize),
    /// Reliable sends need one receiver.
    BroadcastReliable,
    /// No room: the socket's buffer, the peer's reliable window, or the
    /// lag hooks' queue. Counted in the stats.
    Full,
    /// The client gave up (see [`RelayClient::state`]).
    Failed,
    /// The socket said no.
    Socket(ErrorKind),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SendError::TooBig(n) => write!(f, "{n} bytes, over {MAX_PAYLOAD}"),
            SendError::BroadcastReliable => write!(f, "reliable sends can't be broadcast"),
            SendError::Full => write!(f, "no room to send"),
            SendError::Failed => write!(f, "the relay client gave up"),
            SendError::Socket(kind) => write!(f, "socket: {kind}"),
        }
    }
}

impl std::error::Error for SendError {}

/// What a client has done so far.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClientStats {
    /// Frames and bytes on the wire, each way (all kinds, headers too).
    pub frames_sent: u64,
    pub bytes_sent: u64,
    pub frames_received: u64,
    pub bytes_received: u64,
    /// Engine packets handed to `send_*` and sent, and handed out by `recv`.
    pub packets_sent: u64,
    pub packets_received: u64,
    /// Sends that found no room (socket buffer, reliable window, lag queue).
    pub send_drops: u64,
    /// Packets `recv` dropped because they didn't fit the caller's buffer.
    pub too_big: u64,
    /// Packets dropped because `recv` wasn't called for a while.
    pub queue_drops: u64,
    /// Datagrams that weren't well-formed frames.
    pub malformed: u64,
    /// Frames for another room or member, or not from the relay.
    pub foreign: u64,
    /// Reliable frames resent, received again, and given up on.
    pub resends: u64,
    pub duplicates: u64,
    pub gave_up: u64,
    /// Frames the lag hooks lost on purpose.
    pub lag_lost: u64,
    /// The round trip to the relay server.
    pub server_rtt: Option<Duration>,
    /// Members in the room when the server last said hello.
    pub room_members: u32,
    /// Every member heard from or sent to.
    pub peers: Vec<PeerStats>,
}

/// One other member, as a client sees it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PeerStats {
    pub id: u64,
    /// Smoothed round trip, through the relay.
    pub rtt: Option<Duration>,
    /// How long ago it was last heard from (never, if `None`).
    pub last_heard: Option<Duration>,
    /// Engine packets and their bytes, each way.
    pub packets_in: u64,
    pub bytes_in: u64,
    pub packets_out: u64,
    pub bytes_out: u64,
    /// Reliable frames to it not yet acked.
    pub in_flight: usize,
}

/// The launcher's connection to the relay, for one match (room).
pub struct RelayClient {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl RelayClient {
    /// Join `room` as `my_id` through the relay at `relay`, with the lag
    /// hooks from the environment. Returns at once; [`RelayClient::state`]
    /// says when the server has answered.
    pub fn connect(relay: SocketAddr, room: u64, my_id: u64) -> io::Result<RelayClient> {
        RelayClient::connect_with(relay, room, my_id, ClientConfig::from_env())
    }

    pub fn connect_with(
        relay: SocketAddr,
        room: u64,
        my_id: u64,
        config: ClientConfig,
    ) -> io::Result<RelayClient> {
        if my_id == BROADCAST {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "the broadcast id can't be a member's",
            ));
        }
        // On this PC only, for a relay on this PC (no firewall prompt).
        let local = match (relay.is_ipv4(), relay.ip().is_loopback()) {
            (true, true) => Ipv4Addr::LOCALHOST.into(),
            (true, false) => Ipv4Addr::UNSPECIFIED.into(),
            (false, true) => Ipv6Addr::LOCALHOST.into(),
            (false, false) => Ipv6Addr::UNSPECIFIED.into(),
        };
        let socket = sockets::bind(SocketAddr::new(local, 0), BUFFERS)?;
        let port = socket.local_addr()?.port();
        let shared = Arc::new(Shared {
            socket,
            relay,
            room,
            me: my_id,
            rng: Rng::seeded(my_id ^ room.rotate_left(17) ^ u64::from(port)),
            config,
            stop: AtomicBool::new(false),
            state: Mutex::new(State::Joining),
            peers: Mutex::new(HashMap::new()),
            inbox: Mutex::new(VecDeque::new()),
            delayed: Mutex::new(BinaryHeap::new()),
            delayed_count: AtomicU64::new(0),
            unreliable_seq: AtomicU32::new(0),
            server_rtt_us: AtomicU64::new(0),
            room_members: AtomicU32::new(0),
            counters: Counters::default(),
        });
        let worker = Worker::new(shared.clone());
        let thread = std::thread::Builder::new()
            .name("h2relay-client".into())
            .spawn(move || worker.run())?;
        Ok(RelayClient {
            shared,
            thread: Some(thread),
        })
    }

    pub fn state(&self) -> State {
        *lock(&self.shared.state)
    }

    /// The local address frames go out from.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.shared.socket.local_addr()
    }

    /// Send `data` to member `dst` (or every other member, for
    /// [`BROADCAST`]) best-effort, with `port` passed on unchanged. Never
    /// waits.
    pub fn send_unreliable(&self, dst: u64, port: u32, data: &[u8]) -> Result<(), SendError> {
        let s = &self.shared;
        s.sendable(data)?;
        let seq = s.unreliable_seq.fetch_add(1, Relaxed);
        let frame = Frame::new(Kind::Unreliable, s.room, s.me, dst)
            .with_port(port)
            .with_seq(seq)
            .with_payload(data);
        let sent = s.send_frame(&frame);
        if sent.is_ok() {
            s.counters.packets_sent.fetch_add(1, Relaxed);
            if dst != BROADCAST {
                let mut peers = lock(&s.peers);
                if let Some(peer) = peer(&mut peers, dst) {
                    peer.packets_out += 1;
                    peer.bytes_out += data.len() as u64;
                }
            }
        }
        sent
    }

    /// Send `data` to member `dst`, to be handed out there once and in
    /// order with the others sent reliably to it, with `port` passed on
    /// unchanged. Never waits: the frame goes out at once and is kept for
    /// resending.
    pub fn send_reliable(&self, dst: u64, port: u32, data: &[u8]) -> Result<(), SendError> {
        let s = &self.shared;
        s.sendable(data)?;
        if dst == BROADCAST {
            return Err(SendError::BroadcastReliable);
        }
        let now = Instant::now();
        let mut out: Vec<Vec<u8>> = Vec::with_capacity(2);
        {
            let mut peers = lock(&s.peers);
            let Some(peer) = peer(&mut peers, dst) else {
                s.counters.send_drops.fetch_add(1, Relaxed);
                return Err(SendError::Full);
            };
            let rto = s.rto(peer.srtt);
            let stream = &mut peer.out;
            let opening = !stream.open;
            if stream.pending.len() + usize::from(opening) >= s.config.send_window {
                s.counters.send_drops.fetch_add(1, Relaxed);
                return Err(SendError::Full);
            }
            if opening {
                let isn = s.rng.next_u32();
                *stream = Stream {
                    open: true,
                    isn,
                    next: isn,
                    pending: VecDeque::new(),
                };
                let syn = Frame::new(Kind::Reliable, s.room, s.me, dst).with_flags(FLAG_SYN);
                stream.push(syn, now, rto, &mut out);
            }
            let frame = Frame::new(Kind::Reliable, s.room, s.me, dst)
                .with_port(port)
                .with_payload(data);
            stream.push(frame, now, rto, &mut out);
            peer.packets_out += 1;
            peer.bytes_out += data.len() as u64;
        }
        s.counters.packets_sent.fetch_add(1, Relaxed);
        for frame in &out {
            // It's kept, so a frame the socket had no room for goes again
            // with the next resend.
            let _ = s.transmit(frame);
        }
        Ok(())
    }

    /// The next packet received: its sender, port and length, copied into
    /// `buf`. A packet bigger than `buf` is dropped (and counted), never
    /// cut short. Never waits.
    pub fn recv(&self, buf: &mut [u8]) -> Option<(u64, u32, usize)> {
        let s = &self.shared;
        let mut inbox = lock(&s.inbox);
        while let Some(packet) = inbox.pop_front() {
            let n = packet.data.len();
            let Some(to) = buf.get_mut(..n) else {
                s.counters.too_big.fetch_add(1, Relaxed);
                continue;
            };
            to.copy_from_slice(&packet.data);
            s.counters.packets_received.fetch_add(1, Relaxed);
            return Some((packet.src, packet.port, n));
        }
        None
    }

    pub fn stats(&self) -> ClientStats {
        let s = &self.shared;
        let c = &s.counters;
        let get = |a: &AtomicU64| a.load(Relaxed);
        let now = Instant::now();
        let mut peers: Vec<PeerStats> = lock(&s.peers)
            .iter()
            .map(|(&id, p)| PeerStats {
                id,
                rtt: p.srtt,
                last_heard: p.heard.map(|t| now.saturating_duration_since(t)),
                packets_in: p.packets_in,
                bytes_in: p.bytes_in,
                packets_out: p.packets_out,
                bytes_out: p.bytes_out,
                in_flight: p.out.pending.len(),
            })
            .collect();
        peers.sort_by_key(|p| p.id);
        ClientStats {
            frames_sent: get(&c.frames_sent),
            bytes_sent: get(&c.bytes_sent),
            frames_received: get(&c.frames_received),
            bytes_received: get(&c.bytes_received),
            packets_sent: get(&c.packets_sent),
            packets_received: get(&c.packets_received),
            send_drops: get(&c.send_drops),
            too_big: get(&c.too_big),
            queue_drops: get(&c.queue_drops),
            malformed: get(&c.malformed),
            foreign: get(&c.foreign),
            resends: get(&c.resends),
            duplicates: get(&c.duplicates),
            gave_up: get(&c.gave_up),
            lag_lost: get(&c.lag_lost),
            server_rtt: s.server_rtt(),
            room_members: s.room_members.load(Relaxed),
            peers,
        }
    }
}

impl Drop for RelayClient {
    fn drop(&mut self) {
        self.shared.stop.store(true, Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A packet waiting for `recv`.
struct Packet {
    src: u64,
    port: u32,
    data: Vec<u8>,
}

/// Frames the lag hooks hold: when each is due, in the order they came
/// (for frames due at once), and the frame.
type Delayed = BinaryHeap<Reverse<(Instant, u64, Vec<u8>)>>;

#[derive(Default)]
struct Counters {
    frames_sent: AtomicU64,
    bytes_sent: AtomicU64,
    frames_received: AtomicU64,
    bytes_received: AtomicU64,
    packets_sent: AtomicU64,
    packets_received: AtomicU64,
    send_drops: AtomicU64,
    too_big: AtomicU64,
    queue_drops: AtomicU64,
    malformed: AtomicU64,
    foreign: AtomicU64,
    resends: AtomicU64,
    duplicates: AtomicU64,
    gave_up: AtomicU64,
    lag_lost: AtomicU64,
}

/// What the callers and the background thread share.
struct Shared {
    socket: UdpSocket,
    relay: SocketAddr,
    room: u64,
    me: u64,
    config: ClientConfig,
    rng: Rng,
    stop: AtomicBool,
    state: Mutex<State>,
    peers: Mutex<HashMap<u64, Peer>>,
    inbox: Mutex<VecDeque<Packet>>,
    delayed: Mutex<Delayed>,
    delayed_count: AtomicU64,
    unreliable_seq: AtomicU32,
    /// Smoothed, in microseconds plus one (0 for none yet).
    server_rtt_us: AtomicU64,
    room_members: AtomicU32,
    counters: Counters,
}

/// Lock `mutex`, whether or not a thread panicked holding it: what's
/// behind it is counters and queues, fine to go on with.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Peer `id`, made if there's room for one more.
fn peer(peers: &mut HashMap<u64, Peer>, id: u64) -> Option<&mut Peer> {
    if peers.len() >= MAX_PEERS && !peers.contains_key(&id) {
        return None;
    }
    Some(peers.entry(id).or_default())
}

impl Shared {
    fn sendable(&self, data: &[u8]) -> Result<(), SendError> {
        if data.len() > MAX_PAYLOAD {
            self.counters.send_drops.fetch_add(1, Relaxed);
            return Err(SendError::TooBig(data.len()));
        }
        if matches!(*lock(&self.state), State::Failed(_)) {
            return Err(SendError::Failed);
        }
        Ok(())
    }

    fn send_frame(&self, frame: &Frame) -> Result<(), SendError> {
        let mut buf = [0; MAX_FRAME];
        // Our own frames are always well-formed; `sendable` checked the size.
        let Ok(n) = frame.encode(&mut buf) else {
            self.counters.send_drops.fetch_add(1, Relaxed);
            return Err(SendError::TooBig(frame.payload.len()));
        };
        self.transmit(&buf[..n])
    }

    /// Send `bytes` to the relay, through the lag hooks if they're on.
    fn transmit(&self, bytes: &[u8]) -> Result<(), SendError> {
        let lag = self.config.lag;
        if lag.is_none() {
            return self.send_now(bytes);
        }
        if lag.loss > 0.0 && self.rng.unit() < lag.loss {
            // Lost on the way, as far as anyone can tell.
            self.counters.lag_lost.fetch_add(1, Relaxed);
            return Ok(());
        }
        let due = Instant::now() + lag.delay + lag.jitter.mul_f64(self.rng.unit());
        let mut delayed = lock(&self.delayed);
        if delayed.len() >= MAX_DELAYED {
            self.counters.send_drops.fetch_add(1, Relaxed);
            return Err(SendError::Full);
        }
        let order = self.delayed_count.fetch_add(1, Relaxed);
        delayed.push(Reverse((due, order, bytes.to_vec())));
        Ok(())
    }

    fn send_now(&self, bytes: &[u8]) -> Result<(), SendError> {
        match self.socket.send_to(bytes, self.relay) {
            Ok(_) => {
                self.counters.frames_sent.fetch_add(1, Relaxed);
                self.counters
                    .bytes_sent
                    .fetch_add(bytes.len() as u64, Relaxed);
                Ok(())
            }
            Err(e) => {
                self.counters.send_drops.fetch_add(1, Relaxed);
                Err(match e.kind() {
                    ErrorKind::WouldBlock => SendError::Full,
                    kind => SendError::Socket(kind),
                })
            }
        }
    }

    fn server_rtt(&self) -> Option<Duration> {
        match self.server_rtt_us.load(Relaxed) {
            0 => None,
            us => Some(Duration::from_micros(us - 1)),
        }
    }

    /// How long a reliable frame waits for its ack before it's resent,
    /// with `srtt` the peer's smoothed round trip.
    fn rto(&self, srtt: Option<Duration>) -> Duration {
        let rtt = srtt
            .or_else(|| self.server_rtt().map(|r| r * 2))
            .unwrap_or(self.config.initial_rtt);
        (rtt * 2).max(self.config.min_rto)
    }

    /// How long to wait after resend number `resends` (1 for the first).
    fn backoff(&self, rto: Duration, resends: u32) -> Duration {
        let cap = self.config.max_rto.max(rto);
        rto.saturating_mul(1 << resends.min(16)).min(cap)
    }

    fn deliver(&self, src: u64, port: u32, data: &[u8]) {
        let mut inbox = lock(&self.inbox);
        if inbox.len() >= self.config.recv_queue {
            self.counters.queue_drops.fetch_add(1, Relaxed);
            return;
        }
        inbox.push_back(Packet {
            src,
            port,
            data: data.to_vec(),
        });
    }

    fn inbox_full(&self) -> bool {
        lock(&self.inbox).len() >= self.config.recv_queue
    }

    fn set_state(&self, state: State) {
        *lock(&self.state) = state;
    }
}

/// Fold a round-trip `sample` into `srtt` (an eighth of it at a time).
fn smooth(srtt: Option<Duration>, sample: Duration) -> Duration {
    match srtt {
        None => sample,
        Some(srtt) => srtt * 7 / 8 + sample / 8,
    }
}

/// Another member, as this client sees it.
#[derive(Default)]
struct Peer {
    heard: Option<Instant>,
    packets_in: u64,
    bytes_in: u64,
    packets_out: u64,
    bytes_out: u64,
    srtt: Option<Duration>,
    out: Stream,
}

/// The reliable stream to one peer: the sending side.
#[derive(Default)]
struct Stream {
    /// Its SYN has gone out (and it's not been given up on since).
    open: bool,
    /// The SYN's seq, and the next seq to use.
    isn: u32,
    next: u32,
    /// Frames sent and not acked, oldest first.
    pending: VecDeque<Pending>,
}

struct Pending {
    seq: u32,
    frame: Vec<u8>,
    /// When it was first sent, when it's next resent, and how often it
    /// has been.
    sent: Instant,
    due: Instant,
    resends: u32,
}

impl Stream {
    /// Number `frame` next in this stream, keep it, and add it to `out`.
    fn push(&mut self, frame: Frame, now: Instant, rto: Duration, out: &mut Vec<Vec<u8>>) {
        let seq = self.next;
        self.next = seq.wrapping_add(1);
        let Ok(bytes) = frame.with_seq(seq).to_vec() else {
            return;
        };
        out.push(bytes.clone());
        self.pending.push_back(Pending {
            seq,
            frame: bytes,
            sent: now,
            due: now + rto,
            resends: 0,
        });
    }
}

/// The reliable stream from one peer: the receiving side.
struct Inbound {
    isn: u32,
    /// The next seq to hand out.
    expected: u32,
    /// Frames that came early: seq, then port and payload.
    held: HashMap<u32, (u32, Vec<u8>)>,
}

/// The background thread.
struct Worker {
    shared: Arc<Shared>,
    inbound: HashMap<u64, Inbound>,
    /// This attempt's hello number, when it started, and the last sent.
    nonce: u32,
    joining_since: Instant,
    hello_sent: Option<Instant>,
    /// When the last keepalive went, and the server was last heard from.
    keepalive_sent: Instant,
    heard_server: Instant,
    /// The last keepalive's seq and when it went.
    server_ping: (u32, Instant),
    /// The next peer ping's seq, and the last few sent.
    ping_seq: u32,
    peer_pings: VecDeque<(u32, Instant)>,
}

impl Worker {
    fn new(shared: Arc<Shared>) -> Worker {
        let now = Instant::now();
        Worker {
            shared,
            inbound: HashMap::new(),
            nonce: 0,
            joining_since: now,
            hello_sent: None,
            keepalive_sent: now,
            heard_server: now,
            server_ping: (0, now),
            ping_seq: 0,
            peer_pings: VecDeque::new(),
        }
    }

    fn run(mut self) {
        let mut buf = vec![0; 2 * MAX_FRAME];
        let nap = if self.shared.config.lag.is_none() {
            NAP
        } else {
            LAG_NAP
        };
        self.join(Instant::now());
        while !self.shared.stop.load(Relaxed) {
            let wake = self.timers(Instant::now());
            let wait = wake.saturating_duration_since(Instant::now()).min(nap);
            if !wait.is_zero() {
                sockets::wait(&self.shared.socket, wait);
            }
            for _ in 0..BURST {
                match self.shared.socket.recv_from(&mut buf) {
                    Ok((n, from)) => self.handle(&buf[..n], from, Instant::now()),
                    Err(e) => match sockets::recv_error(&e) {
                        RecvError::Empty => break,
                        RecvError::Skip => continue,
                        RecvError::Other => {
                            std::thread::sleep(Duration::from_millis(1));
                            break;
                        }
                    },
                }
            }
        }
        // Gone: tell the server, so the others stop sending here. Twice, in
        // case one is lost, and not held back by the lag hooks.
        if self.state() == State::Joined {
            let s = &self.shared;
            let bye = Frame::new(Kind::Bye, s.room, s.me, 0);
            if let Ok(bytes) = bye.to_vec() {
                for _ in 0..2 {
                    let _ = s.send_now(&bytes);
                }
            }
        }
    }

    fn state(&self) -> State {
        *lock(&self.shared.state)
    }

    /// Start saying hello (again).
    fn join(&mut self, now: Instant) {
        self.nonce = self.shared.rng.next_u32();
        self.joining_since = now;
        self.hello_sent = None;
        self.shared.set_state(State::Joining);
    }

    /// Do what's due: hello, keepalive and pings, resends, held frames.
    /// When something is next due.
    fn timers(&mut self, now: Instant) -> Instant {
        let s = self.shared.clone();
        let config = &s.config;
        let mut wake = now + NAP;
        match self.state() {
            State::Joining => {
                if now >= self.joining_since + config.hello_timeout {
                    s.set_state(State::Failed(Failure::Timeout));
                } else {
                    if self
                        .hello_sent
                        .is_none_or(|sent| now >= sent + config.hello_every)
                    {
                        let hello = Frame::new(Kind::Hello, s.room, s.me, 0).with_seq(self.nonce);
                        let _ = s.send_frame(&hello);
                        self.hello_sent = Some(now);
                    }
                    wake = wake.min(now + config.hello_every);
                }
            }
            State::Joined => {
                if now >= self.heard_server + config.server_silence {
                    self.join(now);
                    return now;
                }
                if now >= self.keepalive_sent + config.keepalive_every {
                    self.keepalive(now);
                }
                wake = wake.min(self.keepalive_sent + config.keepalive_every);
            }
            State::Failed(_) => {}
        }
        wake.min(self.resend(now)).min(self.release(now))
    }

    /// A keepalive to the server, and a ping to every other member.
    fn keepalive(&mut self, now: Instant) {
        let s = &self.shared;
        let seq = self.server_ping.0.wrapping_add(1);
        self.server_ping = (seq, now);
        let _ = s.send_frame(&Frame::new(Kind::Ping, s.room, s.me, 0).with_seq(seq));
        self.ping_seq = self.ping_seq.wrapping_add(1);
        let ping = Frame::new(Kind::Ping, s.room, s.me, BROADCAST)
            .with_flags(FLAG_PEER)
            .with_seq(self.ping_seq);
        let _ = s.send_frame(&ping);
        if self.peer_pings.len() == PINGS_KEPT {
            self.peer_pings.pop_front();
        }
        self.peer_pings.push_back((self.ping_seq, now));
        self.keepalive_sent = now;
    }

    /// Resend reliable frames whose acks are late, and give up on streams
    /// with one resent too often. When the next resend is due.
    fn resend(&mut self, now: Instant) -> Instant {
        let s = &self.shared;
        let mut next = now + NAP;
        let mut out = Vec::new();
        {
            let mut peers = lock(&s.peers);
            for peer in peers.values_mut() {
                let rto = s.rto(peer.srtt);
                let stream = &mut peer.out;
                let late = |p: &Pending| p.due <= now && p.resends >= s.config.max_resends;
                if stream.pending.iter().any(late) {
                    s.counters
                        .gave_up
                        .fetch_add(stream.pending.len() as u64, Relaxed);
                    *stream = Stream::default();
                    continue;
                }
                for p in &mut stream.pending {
                    if p.due <= now {
                        p.resends += 1;
                        p.due = now + s.backoff(rto, p.resends);
                        out.push(p.frame.clone());
                    }
                    next = next.min(p.due);
                }
            }
        }
        s.counters.resends.fetch_add(out.len() as u64, Relaxed);
        for frame in &out {
            let _ = s.transmit(frame);
        }
        next
    }

    /// Send what the lag hooks held that's due. When the next is.
    fn release(&mut self, now: Instant) -> Instant {
        let s = &self.shared;
        let mut next = now + NAP;
        let mut due = Vec::new();
        {
            let mut delayed = lock(&s.delayed);
            while let Some(Reverse((when, _, _))) = delayed.peek() {
                if *when > now {
                    next = *when;
                    break;
                }
                if let Some(Reverse((_, _, frame))) = delayed.pop() {
                    due.push(frame);
                }
            }
        }
        for frame in &due {
            let _ = s.send_now(frame);
        }
        next
    }

    /// One datagram that came in.
    fn handle(&mut self, bytes: &[u8], from: SocketAddr, now: Instant) {
        let s = self.shared.clone();
        let c = &s.counters;
        if from != s.relay {
            c.foreign.fetch_add(1, Relaxed);
            return;
        }
        c.frames_received.fetch_add(1, Relaxed);
        c.bytes_received.fetch_add(bytes.len() as u64, Relaxed);
        let Ok(frame) = decode(bytes) else {
            c.malformed.fetch_add(1, Relaxed);
            return;
        };
        if frame.room != s.room || (frame.dst != s.me && frame.dst != BROADCAST) {
            c.foreign.fetch_add(1, Relaxed);
            return;
        }
        // Whatever comes from the relay says it's there.
        self.heard_server = now;
        if frame.flags & FLAG_SERVER != 0 {
            self.server_frame(&frame, now);
            return;
        }
        if frame.src == s.me {
            c.foreign.fetch_add(1, Relaxed);
            return;
        }
        if let Some(peer) = peer(&mut lock(&s.peers), frame.src) {
            peer.heard = Some(now);
        }
        match frame.kind {
            Kind::Unreliable => {
                s.deliver(frame.src, frame.port, frame.payload);
                self.count_in(frame.src, frame.payload.len());
            }
            Kind::Reliable => self.reliable(&frame),
            Kind::Ack => self.ack(&frame, now),
            Kind::Ping => {
                let pong = Frame::new(Kind::Pong, s.room, s.me, frame.src)
                    .with_flags(FLAG_PEER)
                    .with_seq(frame.seq);
                let _ = s.send_frame(&pong);
            }
            Kind::Pong => {
                let sent = self.peer_pings.iter().find(|p| p.0 == frame.seq);
                if let Some(&(_, sent)) = sent {
                    if let Some(peer) = lock(&s.peers).get_mut(&frame.src) {
                        peer.srtt = Some(smooth(peer.srtt, now - sent));
                    }
                }
            }
            // Only the server says hello, refuses or pongs a keepalive, and
            // the server takes byes.
            Kind::Hello | Kind::Bye | Kind::Refused => {
                c.foreign.fetch_add(1, Relaxed);
            }
        }
    }

    fn count_in(&self, src: u64, bytes: usize) {
        if let Some(peer) = lock(&self.shared.peers).get_mut(&src) {
            peer.packets_in += 1;
            peer.bytes_in += bytes as u64;
        }
    }

    fn server_frame(&mut self, frame: &Frame, now: Instant) {
        let s = self.shared.clone();
        let state = self.state();
        match frame.kind {
            Kind::Hello => {
                if state == State::Joining && frame.seq == self.nonce {
                    s.set_state(State::Joined);
                    s.room_members.store(frame.port, Relaxed);
                    self.keepalive_sent = now;
                    if let Some(sent) = self.hello_sent {
                        self.server_rtt(now - sent);
                    }
                }
            }
            Kind::Pong => {
                if frame.seq == self.server_ping.0 {
                    self.server_rtt(now - self.server_ping.1);
                }
            }
            Kind::Refused => match (state, Refusal::from_code(frame.seq)) {
                // Forgotten: join again.
                (State::Joined, Refusal::NotMember) => self.join(now),
                // About something sent before the hello got there.
                (State::Joining, Refusal::NotMember) => {}
                (State::Joining, why) => s.set_state(State::Failed(Failure::Refused(why))),
                _ => {}
            },
            _ => {
                s.counters.foreign.fetch_add(1, Relaxed);
            }
        }
    }

    fn server_rtt(&self, sample: Duration) {
        let s = &self.shared;
        let smoothed = smooth(s.server_rtt(), sample);
        let us = u64::try_from(smoothed.as_micros()).unwrap_or(u64::MAX - 1);
        s.server_rtt_us.store(us + 1, Relaxed);
    }

    /// A reliable frame from a peer: start or restart its stream (SYN), or
    /// hold and hand out in order, each once; ack what's taken.
    fn reliable(&mut self, frame: &Frame) {
        let s = self.shared.clone();
        let src = frame.src;
        if frame.flags & FLAG_SYN != 0 {
            let known = self.inbound.contains_key(&src);
            if !known && self.inbound.len() >= MAX_PEERS {
                return;
            }
            let stream = self.inbound.entry(src).or_insert(Inbound {
                isn: frame.seq,
                expected: frame.seq.wrapping_add(1),
                held: HashMap::new(),
            });
            if stream.isn != frame.seq {
                // The sender started over.
                *stream = Inbound {
                    isn: frame.seq,
                    expected: frame.seq.wrapping_add(1),
                    held: HashMap::new(),
                };
            } else if known {
                s.counters.duplicates.fetch_add(1, Relaxed);
            }
            let expected = stream.expected;
            self.send_ack(src, frame.seq, expected);
            return;
        }
        // Before its SYN: not acked, so it comes again once the stream is
        // open.
        let Some(stream) = self.inbound.get_mut(&src) else {
            return;
        };
        let ahead = seq_after(frame.seq, stream.expected);
        if ahead < 0 {
            if ahead >= -DUPLICATE_WINDOW {
                s.counters.duplicates.fetch_add(1, Relaxed);
                let expected = stream.expected;
                self.send_ack(src, frame.seq, expected);
            }
            return;
        }
        if ahead as usize >= RECV_WINDOW {
            return;
        }
        if stream.held.contains_key(&frame.seq) {
            s.counters.duplicates.fetch_add(1, Relaxed);
        } else if s.inbox_full() {
            // Not acked: it comes again once `recv` has caught up.
            s.counters.queue_drops.fetch_add(1, Relaxed);
            return;
        } else {
            let held = (frame.port, frame.payload.to_vec());
            stream.held.entry(frame.seq).or_insert(held);
            let mut handed = Vec::new();
            while let Some((port, data)) = stream.held.remove(&stream.expected) {
                stream.expected = stream.expected.wrapping_add(1);
                handed.push((port, data));
            }
            for (port, data) in &handed {
                s.deliver(src, *port, data);
                self.count_in(src, data.len());
            }
        }
        let expected = self.inbound.get(&src).map_or(0, |s| s.expected);
        self.send_ack(src, frame.seq, expected);
    }

    fn send_ack(&self, to: u64, seq: u32, expected: u32) {
        let s = &self.shared;
        let ack = Frame::new(Kind::Ack, s.room, s.me, to)
            .with_seq(seq)
            .with_port(expected);
        let _ = s.send_frame(&ack);
    }

    /// An ack from a peer: forget what it took, and learn the round trip.
    fn ack(&mut self, frame: &Frame, now: Instant) {
        let s = &self.shared;
        let mut peers = lock(&s.peers);
        let Some(peer) = peers.get_mut(&frame.src) else {
            return;
        };
        let stream = &mut peer.out;
        if !stream.open {
            return;
        }
        // Everything is reckoned from the SYN, so the numbers never wrap.
        let sent = stream.next.wrapping_sub(stream.isn);
        let offset = |seq: u32| seq.wrapping_sub(stream.isn);
        let cumulative = offset(frame.port);
        let cumulative = if cumulative <= sent { cumulative } else { 0 };
        let acked = offset(frame.seq);
        let mut sample = None;
        stream.pending.retain(|p| {
            let taken = offset(p.seq) < cumulative || offset(p.seq) == acked;
            if taken && p.seq == frame.seq && p.resends == 0 {
                sample = Some(now - p.sent);
            }
            !taken
        });
        if let Some(sample) = sample {
            peer.srtt = Some(smooth(peer.srtt, sample));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Looks `name` up in `pairs`.
    fn lookup(pairs: &[(&str, &str)], name: &str) -> Option<String> {
        let found = pairs.iter().find(|(n, _)| *n == name);
        found.map(|(_, v)| v.to_string())
    }

    #[test]
    fn lag_hooks_read_their_variables() {
        assert!(Lag::from_vars(|n| lookup(&[], n)).is_none());
        let set = [
            ("H2_RELAY_LAG_MS", "40"),
            ("H2_RELAY_JITTER_MS", " 7.5 "),
            ("H2_RELAY_LOSS", "0.05"),
        ];
        let lag = Lag::from_vars(|n| lookup(&set, n));
        assert_eq!(lag.delay, Duration::from_millis(40));
        assert_eq!(lag.jitter, Duration::from_micros(7500));
        assert_eq!(lag.loss, 0.05);
        // Out of range: clamped; nonsense: none.
        let set = [
            ("H2_RELAY_LAG_MS", "-5"),
            ("H2_RELAY_JITTER_MS", "lots"),
            ("H2_RELAY_LOSS", "3"),
        ];
        let lag = Lag::from_vars(|n| lookup(&set, n));
        assert_eq!(lag.delay, Duration::ZERO);
        assert_eq!(lag.jitter, Duration::ZERO);
        assert_eq!(lag.loss, 1.0);
        let lag = Lag::from_vars(|n| lookup(&[("H2_RELAY_LOSS", "NaN")], n));
        assert!(lag.is_none());
    }

    #[test]
    fn resends_wait_twice_the_round_trip_and_back_off() {
        // A client whose relay never answers: only its sums are used.
        let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
        let relay = silent.local_addr().unwrap();
        let client = RelayClient::connect_with(relay, 1, 1, ClientConfig::default()).unwrap();
        let s = &client.shared;
        let ms = Duration::from_millis;
        // max(2 x round trip, 50 ms), the round trip assumed if unknown.
        assert_eq!(s.rto(Some(ms(5))), ms(50));
        assert_eq!(s.rto(Some(ms(80))), ms(160));
        assert_eq!(s.rto(None), ms(200));
        // Through the server both ways, once its round trip is known.
        s.server_rtt_us.store(30_001, Relaxed);
        assert_eq!(s.rto(None), ms(120));
        // Doubling each time, up to a second (or the wait itself, if longer).
        let waits: Vec<_> = (1..=5).map(|n| s.backoff(ms(160), n)).collect();
        assert_eq!(waits, [ms(320), ms(640), ms(1000), ms(1000), ms(1000)]);
        assert_eq!(s.backoff(ms(1500), 3), ms(1500));
        assert_eq!(s.backoff(ms(50), 1000), ms(1000));
        // Smoothed an eighth at a time.
        assert_eq!(smooth(Some(ms(80)), ms(160)), ms(90));
        assert_eq!(smooth(None, ms(160)), ms(160));
    }
}
