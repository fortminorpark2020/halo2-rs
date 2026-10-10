//! The engine's network host functions (slots 41 to 44) on top of the
//! relay: what the Windows host object calls with the engine's arguments,
//! kept free of raw pointers so it is tested on any platform.
//!
//! Seen on the owner's PC (stages 2 and 3, 2026-10-10): halo2.dll opens no
//! sockets of its own and polls slot 43 about twice a frame, with a
//! 0x1000-byte buffer, an out pointer, and 1000 (0x3E8) as the fourth
//! argument; slots 41, 42 and 44 were never called by a host waiting for a
//! peer. The fourth argument of slot 43 is not a pointer, so it is most
//! likely the port the engine is polling (1000 is the Xbox in-band port,
//! research 2.4) rather than libmcc's `port_out` pointer. `RecvPort` says
//! how it is read; the default (`Auto`) hands a poll of port P a packet
//! sent to port P, and a packet sent to a port nobody polls to any poll,
//! so a wrong guess about the argument can't strand packets.
//!
//! Everything is logged: the first calls of each slot with their sizes,
//! ports and first bytes, each new (destination, port) pair, each new value
//! polled, and a summary with the relay's counters at the end. The bytes
//! themselves are the engine's own traffic; only their first 16 go in the
//! log, which stays on the PC.

use h2relay::{ClientConfig, MemberKey, RelayClient, SendError, State};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

/// How slot 43's fourth argument is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecvPort {
    /// A port value; packets for ports never polled go to any poll.
    Auto,
    /// A port value, strictly: a poll gets only packets sent to that port.
    Value,
    /// libmcc's `u32* port_out`: any packet, and its port written there.
    Pointer,
    /// Ignored: any packet, oldest first.
    Ignore,
}

impl RecvPort {
    pub fn parse(s: &str) -> Option<RecvPort> {
        Some(match s.to_ascii_lowercase().as_str() {
            "auto" => RecvPort::Auto,
            "value" => RecvPort::Value,
            "pointer" => RecvPort::Pointer,
            "ignore" => RecvPort::Ignore,
            _ => return None,
        })
    }
}

/// What slots 41 and 42 return for a packet that went (a failed send
/// returns 0 in every case). Unknown in this build: libmcc's signature
/// returns a u32 and research 2.3 guesses the length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendReturn {
    Len,
    Zero,
    One,
}

impl SendReturn {
    pub fn parse(s: &str) -> Option<SendReturn> {
        Some(match s.to_ascii_lowercase().as_str() {
            "len" => SendReturn::Len,
            "0" | "zero" => SendReturn::Zero,
            "1" | "one" => SendReturn::One,
            _ => return None,
        })
    }
}

/// Packets kept for the engine at most; the oldest go first.
const QUEUE: usize = 1024;
/// Calls of each slot logged in full.
const DETAIL: u64 = 24;
/// Distinct values remembered for "first seen" lines.
const DISTINCT: usize = 32;
/// Bytes of each packet shown in the log.
const HEAD: usize = 16;

pub struct Settings {
    pub recv_port: RecvPort,
    pub send_return: SendReturn,
    /// The match's machine ids, to name destinations in the log.
    pub machines: Vec<u64>,
}

struct Packet {
    src: u64,
    port: u32,
    data: Vec<u8>,
}

#[derive(Default)]
struct Inbox {
    packets: VecDeque<Packet>,
    /// Ports the engine has polled (low 32 bits of the fourth argument).
    polled: Vec<u32>,
    /// Ports packets came in on.
    arrived: Vec<u32>,
    /// Ports whose packets were handed to a poll of another port.
    rerouted: Vec<u32>,
    dropped_old: u64,
    dropped_big: u64,
    largest_in: usize,
}

#[derive(Default)]
struct Seen {
    pairs: Vec<(bool, u64, u32)>,
    pair_counts: Vec<(u64, u64)>,
    unknown_dst: Vec<u64>,
}

pub struct Net {
    client: Option<RelayClient>,
    me: u64,
    settings: Settings,
    log: Log,
    inbox: Mutex<Inbox>,
    seen: Mutex<Seen>,
    calls: [AtomicU64; 4],
    sent_ok: AtomicU64,
    sent_failed: AtomicU64,
    bytes_out: AtomicU64,
    delivered: AtomicU64,
    bytes_in: AtomicU64,
    started: Instant,
}

fn head(data: &[u8]) -> String {
    let mut s = String::new();
    for b in data.iter().take(HEAD) {
        s.push_str(&format!("{b:02x}"));
    }
    if data.len() > HEAD {
        s.push('…');
    }
    s
}

fn remember<T: PartialEq>(list: &mut Vec<T>, v: T) -> bool {
    if list.len() >= DISTINCT || list.contains(&v) {
        return false;
    }
    list.push(v);
    true
}

impl Net {
    /// No relay: every send fails (returns 0) and nothing is received, but
    /// every call is still logged.
    pub fn solo(settings: Settings, log: Log) -> Net {
        Net::with(None, 0, settings, log)
    }

    /// Joins `room` on the relay at `relay` as `me`. Returns at once; see
    /// `wait_joined`.
    pub fn connect(
        relay: SocketAddr,
        room: u64,
        me: u64,
        key: Option<&str>,
        settings: Settings,
        log: Log,
    ) -> Result<Net, String> {
        let mut config = ClientConfig::from_env();
        if let Some(k) = key {
            config.member_key =
                Some(MemberKey::from_hex(k).ok_or("the member key isn't 32 hex digits")?);
        }
        if !config.lag.is_none() {
            log(&format!("relay lag hooks on: {:?}", config.lag));
        }
        let client = RelayClient::connect_with(relay, room, me, config)
            .map_err(|e| format!("relay client for {relay}: {e}"))?;
        Ok(Net::with(Some(client), me, settings, log))
    }

    fn with(client: Option<RelayClient>, me: u64, settings: Settings, log: Log) -> Net {
        Net {
            client,
            me,
            settings,
            log,
            inbox: Mutex::new(Inbox::default()),
            seen: Mutex::new(Seen::default()),
            calls: Default::default(),
            sent_ok: AtomicU64::new(0),
            sent_failed: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
            delivered: AtomicU64::new(0),
            bytes_in: AtomicU64::new(0),
            started: Instant::now(),
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn state(&self) -> Option<State> {
        self.client.as_ref().map(|c| c.state())
    }

    /// Waits until the relay took us into the room, it gave up, or
    /// `timeout` passed. Returns the state then (`None` when solo).
    pub fn wait_joined(&self, timeout: Duration) -> Option<State> {
        let c = self.client.as_ref()?;
        let until = Instant::now() + timeout;
        loop {
            let st = c.state();
            if st != State::Joining || Instant::now() >= until {
                return Some(st);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn name(&self, id: u64) -> String {
        if id == self.me && self.client.is_some() {
            return format!("{id:#018x} (us)");
        }
        match self.settings.machines.iter().position(|&m| m == id) {
            Some(i) => format!("{id:#018x} (machine {i})"),
            None if id == u64::MAX => "everyone".into(),
            None => format!("{id:#018x} (not in the session)"),
        }
    }

    fn t(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Slots 41 (`reliable` false) and 42: one engine packet for `dst`.
    /// Returns what the engine gets back.
    pub fn send(&self, reliable: bool, dst: u64, data: &[u8], port: u32) -> u32 {
        let slot = if reliable { 42 } else { 41 };
        let n = self.calls[usize::from(reliable)].fetch_add(1, Relaxed);
        let result = match &self.client {
            None => Err(None),
            Some(c) if reliable => c.send_reliable(dst, port, data).map_err(Some),
            Some(c) => c.send_unreliable(dst, port, data).map_err(Some),
        };
        let back = match result {
            Ok(()) => {
                self.sent_ok.fetch_add(1, Relaxed);
                self.bytes_out.fetch_add(data.len() as u64, Relaxed);
                match self.settings.send_return {
                    SendReturn::Len => data.len() as u32,
                    SendReturn::Zero => 0,
                    SendReturn::One => 1,
                }
            }
            Err(_) => {
                self.sent_failed.fetch_add(1, Relaxed);
                0
            }
        };
        let why = match &result {
            Ok(()) => "sent".to_string(),
            Err(None) => "no relay (solo)".to_string(),
            Err(Some(SendError::Full)) => "dropped: no room to send".to_string(),
            Err(Some(e)) => format!("failed: {e}"),
        };
        if let Ok(mut seen) = self.seen.lock() {
            if remember(&mut seen.pairs, (reliable, dst, port)) {
                (self.log)(&format!(
                    "net: slot {slot} first send to {} port {port} ({} bytes, {why})",
                    self.name(dst),
                    data.len()
                ));
            }
            if !self.settings.machines.contains(&dst)
                && dst != u64::MAX
                && remember(&mut seen.unknown_dst, dst)
            {
                (self.log)(&format!(
                    "net: the engine sends to {dst:#018x}, which isn't a machine in the session"
                ));
            }
            if let Some((_, c)) = seen.pair_counts.iter_mut().find(|(d, _)| *d == dst) {
                *c += 1;
            } else if seen.pair_counts.len() < DISTINCT {
                seen.pair_counts.push((dst, 1));
            }
        }
        if n < DETAIL || (result.is_err() && n < 4 * DETAIL) {
            (self.log)(&format!(
                "net: slot {slot} call {} at {:.3} s: to {} port {port}, {} bytes [{}], {why}, returned {back}",
                n + 1,
                self.t(),
                self.name(dst),
                data.len(),
                head(data)
            ));
        }
        back
    }

    /// Moves what the relay received into our queue.
    fn drain(&self, inbox: &mut Inbox) {
        let Some(c) = &self.client else { return };
        let mut buf = vec![0u8; h2relay::MAX_PAYLOAD];
        while let Some((src, port, n)) = c.recv(&mut buf) {
            if remember(&mut inbox.arrived, port) {
                (self.log)(&format!(
                    "net: first packet on port {port} from {} ({n} bytes) at {:.3} s",
                    self.name(src),
                    self.t()
                ));
            }
            if inbox.packets.len() >= QUEUE {
                inbox.packets.pop_front();
                inbox.dropped_old += 1;
                if inbox.dropped_old == 1 || inbox.dropped_old.is_power_of_two() {
                    (self.log)(&format!(
                        "net: {} packets dropped because the engine isn't taking them (polled ports {:?}, packets on {:?})",
                        inbox.dropped_old, inbox.polled, inbox.arrived
                    ));
                }
            }
            inbox.largest_in = inbox.largest_in.max(n);
            inbox.packets.push_back(Packet {
                src,
                port,
                data: buf[..n].to_vec(),
            });
        }
    }

    /// Slot 43: the next packet for a poll whose fourth argument was
    /// `arg`, copied into `buf`. Returns (sender, port, length), or `None`
    /// when nothing is waiting. A packet longer than `buf` is dropped and
    /// logged, never cut short.
    pub fn recv(&self, buf: &mut [u8], arg: u64) -> Option<(u64, u32, usize)> {
        let n = self.calls[2].fetch_add(1, Relaxed);
        let mode = self.settings.recv_port;
        let want = arg as u32;
        let Ok(mut inbox) = self.inbox.lock() else {
            return None;
        };
        if matches!(mode, RecvPort::Auto | RecvPort::Value) && remember(&mut inbox.polled, want) {
            (self.log)(&format!(
                "net: slot 43 polls port {want} (fourth argument {arg:#x}, buffer {} bytes) at {:.3} s",
                buf.len(),
                self.t()
            ));
        } else if n == 0 {
            (self.log)(&format!(
                "net: slot 43 first poll: fourth argument {arg:#x}, buffer {} bytes, read as {mode:?}",
                buf.len()
            ));
        }
        self.drain(&mut inbox);
        loop {
            let at = match mode {
                RecvPort::Pointer | RecvPort::Ignore => (!inbox.packets.is_empty()).then_some(0),
                RecvPort::Value => inbox.packets.iter().position(|p| p.port == want),
                RecvPort::Auto => inbox
                    .packets
                    .iter()
                    .position(|p| p.port == want)
                    .or_else(|| {
                        let polled = &inbox.polled;
                        inbox.packets.iter().position(|p| !polled.contains(&p.port))
                    }),
            }?;
            let p = inbox.packets.remove(at)?;
            if p.data.len() > buf.len() {
                inbox.dropped_big += 1;
                (self.log)(&format!(
                    "net: a {}-byte packet from {} on port {} doesn't fit the engine's {}-byte buffer; dropped",
                    p.data.len(),
                    self.name(p.src),
                    p.port,
                    buf.len()
                ));
                continue;
            }
            if p.port != want && mode == RecvPort::Auto && remember(&mut inbox.rerouted, p.port) {
                (self.log)(&format!(
                    "net: packets on port {} go to polls of port {want}, as the engine never polls {}",
                    p.port, p.port
                ));
            }
            buf[..p.data.len()].copy_from_slice(&p.data);
            let d = self.delivered.fetch_add(1, Relaxed);
            self.bytes_in.fetch_add(p.data.len() as u64, Relaxed);
            if d < DETAIL {
                (self.log)(&format!(
                    "net: slot 43 delivery {} at {:.3} s: from {} port {} (poll {arg:#x}), {} bytes [{}]",
                    d + 1,
                    self.t(),
                    self.name(p.src),
                    p.port,
                    p.data.len(),
                    head(&p.data)
                ));
            }
            return Some((p.src, p.port, p.data.len()));
        }
    }

    /// Slot 44 (meaning unknown): counted, and its first calls logged.
    pub fn note_slot44(&self, a1: usize, a2: usize, a3: usize) {
        let n = self.calls[3].fetch_add(1, Relaxed);
        if n < DETAIL {
            (self.log)(&format!(
                "net: slot 44 call {} at {:.3} s: a1={a1:#x} a2={a2:#x} a3={a3:#x}",
                n + 1,
                self.t()
            ));
        }
    }

    /// The end-of-run summary.
    pub fn summary(&self) -> Vec<String> {
        let mut out = vec![format!(
            "net summary: slot 41 {} calls, slot 42 {}, slot 43 {} polls, slot 44 {}; {} packets sent ({} bytes), {} failed; {} delivered ({} bytes)",
            self.calls[0].load(Relaxed),
            self.calls[1].load(Relaxed),
            self.calls[2].load(Relaxed),
            self.calls[3].load(Relaxed),
            self.sent_ok.load(Relaxed),
            self.bytes_out.load(Relaxed),
            self.sent_failed.load(Relaxed),
            self.delivered.load(Relaxed),
            self.bytes_in.load(Relaxed),
        )];
        if let Ok(inbox) = self.inbox.lock() {
            out.push(format!(
                "net summary: ports polled {:?}, ports received on {:?}, rerouted {:?}; {} waiting, {} dropped old, {} dropped too big, largest in {} bytes",
                inbox.polled,
                inbox.arrived,
                inbox.rerouted,
                inbox.packets.len(),
                inbox.dropped_old,
                inbox.dropped_big,
                inbox.largest_in
            ));
        }
        if let Ok(seen) = self.seen.lock() {
            let per: Vec<String> = seen
                .pair_counts
                .iter()
                .map(|(d, c)| format!("{} x{c}", self.name(*d)))
                .collect();
            if !per.is_empty() {
                out.push(format!(
                    "net summary: sends by destination: {}",
                    per.join(", ")
                ));
            }
        }
        if let Some(c) = &self.client {
            let s = c.stats();
            out.push(format!(
                "net summary: relay {:?}; frames out {} in {}; drops {} queue {} too big {}; resends {} gave up {}; largest out {} in {}; over MTU {}; server round trip {:?}; room members {}",
                c.state(),
                s.frames_sent,
                s.frames_received,
                s.send_drops,
                s.queue_drops,
                s.too_big,
                s.resends,
                s.gave_up,
                s.largest_sent,
                s.largest_received,
                s.over_mtu,
                s.server_rtt,
                s.room_members
            ));
            for p in &s.peers {
                out.push(format!(
                    "net summary: peer {}: round trip {:?}, last heard {:?} ago, in {} ({} bytes), out {} ({} bytes), unacked {}",
                    self.name(p.id),
                    p.rtt,
                    p.last_heard,
                    p.packets_in,
                    p.bytes_in,
                    p.packets_out,
                    p.bytes_out,
                    p.in_flight
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use h2relay::{RelayServer, ServerConfig};

    const HOST: u64 = 0x4832_0000_0000_0001;
    const GUEST: u64 = 0x4832_0000_0000_0002;
    const ROOM: u64 = 0x5EED_0000_0000_0001;

    fn quiet() -> Log {
        Arc::new(|_: &str| {})
    }

    fn collect() -> (Log, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let l = lines.clone();
        (
            Arc::new(move |s: &str| l.lock().unwrap().push(s.to_string())),
            lines,
        )
    }

    fn settings(recv_port: RecvPort) -> Settings {
        Settings {
            recv_port,
            send_return: SendReturn::Len,
            machines: vec![HOST, GUEST],
        }
    }

    fn pair(mode: RecvPort) -> (h2relay::RelayThread, Net, Net) {
        let server = RelayServer::bind("127.0.0.1:0", ServerConfig::default())
            .unwrap()
            .spawn()
            .unwrap();
        let addr = server.local_addr();
        let host = Net::connect(addr, ROOM, HOST, None, settings(mode), quiet()).unwrap();
        let guest = Net::connect(addr, ROOM, GUEST, None, settings(mode), quiet()).unwrap();
        for n in [&host, &guest] {
            assert_eq!(n.wait_joined(Duration::from_secs(5)), Some(State::Joined));
        }
        (server, host, guest)
    }

    /// Polls until a packet comes or two seconds pass.
    fn poll(n: &Net, arg: u64) -> Option<(u64, u32, Vec<u8>)> {
        let mut buf = [0u8; 0x1000];
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until {
            if let Some((src, port, len)) = n.recv(&mut buf, arg) {
                return Some((src, port, buf[..len].to_vec()));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        None
    }

    #[test]
    fn solo_logs_and_returns_nothing() {
        let (log, lines) = collect();
        let n = Net::solo(settings(RecvPort::Auto), log);
        assert_eq!(n.send(false, HOST, b"hello", 1000), 0);
        let mut buf = [0u8; 64];
        assert_eq!(n.recv(&mut buf, 1000), None);
        assert_eq!(n.recv(&mut buf, 1001), None);
        assert_eq!(n.state(), None);
        let lines = lines.lock().unwrap();
        assert!(lines
            .iter()
            .any(|l| l.contains("slot 41 first send") && l.contains("machine 0")));
        assert!(lines.iter().any(|l| l.contains("polls port 1000")));
        assert!(lines.iter().any(|l| l.contains("polls port 1001")));
        assert!(n.summary()[0].contains("slot 41 1 calls"));
    }

    #[test]
    fn packets_cross_the_relay_both_ways() {
        let (_server, host, guest) = pair(RecvPort::Auto);
        assert_eq!(guest.send(false, HOST, b"join request", 1000), 12);
        assert_eq!(
            poll(&host, 1000),
            Some((GUEST, 1000, b"join request".to_vec()))
        );
        assert_eq!(host.send(true, GUEST, b"membership", 1000), 10);
        assert_eq!(
            poll(&guest, 1000),
            Some((HOST, 1000, b"membership".to_vec()))
        );
        let s = host.summary();
        assert!(s[0].contains("1 delivered"), "{s:?}");
    }

    #[test]
    fn auto_keeps_ports_apart_but_strands_nothing() {
        let (_server, host, guest) = pair(RecvPort::Auto);
        guest.send(false, HOST, b"in-band", 1000);
        guest.send(false, HOST, b"out-of-band", 1001);
        // Port 1001 hasn't been polled yet: a poll of 1000 takes 1000's.
        assert_eq!(poll(&host, 1000).unwrap().2, b"in-band");
        // Now 1001 is polled; a poll of 1000 must not take its packet.
        let mut buf = [0u8; 64];
        assert_eq!(host.recv(&mut buf, 1001).map(|r| r.1), Some(1001));
        guest.send(false, HOST, b"odd port", 7);
        // Port 7 is never polled: it goes to whichever poll comes.
        assert_eq!(poll(&host, 1000).map(|r| r.1), Some(7));
    }

    #[test]
    fn value_mode_is_strict() {
        let (_server, host, guest) = pair(RecvPort::Value);
        guest.send(false, HOST, b"x", 1001);
        let mut buf = [0u8; 64];
        let until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < until {
            assert_eq!(host.recv(&mut buf, 1000), None);
        }
        assert_eq!(poll(&host, 1001).map(|r| r.1), Some(1001));
    }

    #[test]
    fn ignore_mode_takes_any_port() {
        let (_server, host, guest) = pair(RecvPort::Ignore);
        guest.send(false, HOST, b"x", 1001);
        assert_eq!(poll(&host, 0xDEAD_BEEF).map(|r| r.1), Some(1001));
    }

    #[test]
    fn too_big_for_the_buffer_is_dropped_not_cut() {
        let (_server, host, guest) = pair(RecvPort::Auto);
        guest.send(false, HOST, &[7u8; 100], 1000);
        guest.send(false, HOST, b"small", 1000);
        let mut buf = [0u8; 64];
        let until = Instant::now() + Duration::from_secs(2);
        let mut got = None;
        while got.is_none() && Instant::now() < until {
            got = host.recv(&mut buf, 1000);
        }
        let (_, _, len) = got.unwrap();
        assert_eq!(&buf[..len], b"small");
        assert!(host.summary()[1].contains("1 dropped too big"));
    }

    #[test]
    fn talking_to_ourselves_stays_local() {
        let (_server, host, _guest) = pair(RecvPort::Auto);
        assert_eq!(host.send(false, HOST, b"loopback", 1005), 8);
        assert_eq!(poll(&host, 1005), Some((HOST, 1005, b"loopback".to_vec())));
    }

    #[test]
    fn send_return_choices() {
        let (_server, host, _guest) = pair(RecvPort::Auto);
        let mut s = settings(RecvPort::Auto);
        s.send_return = SendReturn::One;
        let one = Net {
            settings: s,
            ..host
        };
        assert_eq!(one.send(false, GUEST, b"abc", 1000), 1);
        assert_eq!(SendReturn::parse("zero"), Some(SendReturn::Zero));
        assert_eq!(SendReturn::parse("len"), Some(SendReturn::Len));
        assert_eq!(SendReturn::parse("2"), None);
        assert_eq!(RecvPort::parse("POINTER"), Some(RecvPort::Pointer));
        assert_eq!(RecvPort::parse("maybe"), None);
    }

    #[test]
    fn unknown_destinations_are_named() {
        let (log, lines) = collect();
        let n = Net::solo(settings(RecvPort::Auto), log);
        n.send(false, 0x1234, b"?", 1000);
        assert!(lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("isn't a machine in the session")));
    }
}
