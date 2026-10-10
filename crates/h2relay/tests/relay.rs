//! The relay end to end on this PC: a server and clients on 127.0.0.1
//! (ports chosen by the system), plus hand-made frames from plain sockets
//! where a test needs a member to misbehave or go quiet.

use h2relay::frame::{COOKIE_LEN, FLAG_SERVER, FLAG_SYN, HELLO_LEN, MAX_PAYLOAD, MTU_PAYLOAD};
use h2relay::{
    decode, Admission, ClientConfig, Failure, Frame, Kind, Lag, MemberKey, Refusal, RelayClient,
    RelayServer, RelayThread, SendError, ServerConfig, ServerStats, State, BROADCAST,
};
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long anything may take.
const WAIT: Duration = Duration::from_secs(5);
const ROOM: u64 = 0x5eed_0000_0000_0001;
const OTHER_ROOM: u64 = 0x5eed_0000_0000_0002;

fn server(config: ServerConfig) -> RelayThread {
    let config = ServerConfig {
        stats_every: Duration::ZERO,
        ..config
    };
    RelayServer::bind("127.0.0.1:0", config)
        .unwrap()
        .with_log(Arc::new(|_| {}))
        .spawn()
        .unwrap()
}

fn issued() -> ServerConfig {
    ServerConfig {
        admission: Admission::Issued,
        ..ServerConfig::default()
    }
}

/// A client's settings with short times, for tests.
fn fast() -> ClientConfig {
    ClientConfig {
        hello_every: Duration::from_millis(20),
        hello_timeout: Duration::from_secs(3),
        keepalive_every: Duration::from_millis(50),
        server_silence: Duration::from_secs(2),
        max_rto: Duration::from_millis(100),
        ..ClientConfig::default()
    }
}

fn keyed(key: MemberKey) -> ClientConfig {
    ClientConfig {
        member_key: Some(key),
        ..fast()
    }
}

fn client(relay: SocketAddr, room: u64, id: u64, config: ClientConfig) -> RelayClient {
    RelayClient::connect_with(relay, room, id, config).unwrap()
}

/// Wait until `done`, or fail saying `what`.
fn until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn joined(clients: &[&RelayClient]) {
    until("everyone to join", || {
        clients.iter().all(|c| c.state() == State::Joined)
    });
}

type Packet = (u64, u32, Vec<u8>);

/// Everything waiting for `client`.
fn drain(client: &RelayClient) -> Vec<Packet> {
    let mut buf = [0; MAX_PAYLOAD];
    let mut got = Vec::new();
    while let Some((src, port, n)) = client.recv(&mut buf) {
        got.push((src, port, buf[..n].to_vec()));
    }
    got
}

/// The next `n` packets for `client`, waiting for them.
fn take(client: &RelayClient, n: usize) -> Vec<Packet> {
    let mut got = Vec::new();
    until(&format!("{n} packets"), || {
        got.extend(drain(client));
        got.len() >= n
    });
    got
}

/// Nothing more comes for any of `clients` for a while.
fn quiet(clients: &[&RelayClient]) {
    std::thread::sleep(Duration::from_millis(100));
    for (i, c) in clients.iter().enumerate() {
        assert_eq!(drain(c), [], "client {i} got something");
    }
}

/// A frame received, owned.
#[derive(Debug)]
struct Got {
    kind: Kind,
    flags: u8,
    src: u64,
    dst: u64,
    port: u32,
    seq: u32,
    payload: Vec<u8>,
    /// The datagram's length.
    len: usize,
}

/// A member made by hand, from a plain socket.
struct Raw {
    socket: UdpSocket,
    relay: SocketAddr,
    room: u64,
    id: u64,
}

impl Raw {
    fn new(relay: SocketAddr, room: u64, id: u64) -> Raw {
        Raw::on("127.0.0.1", relay, room, id).unwrap()
    }

    /// One on another of this PC's addresses (127.0.0.2, say), if the
    /// system lets it have that one.
    fn on(ip: &str, relay: SocketAddr, room: u64, id: u64) -> Option<Raw> {
        let socket = UdpSocket::bind((ip, 0)).ok()?;
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        Some(Raw {
            socket,
            relay,
            room,
            id,
        })
    }

    fn send(&self, frame: Frame) -> usize {
        let bytes = frame.to_vec().unwrap();
        self.socket.send_to(&bytes, self.relay).unwrap();
        bytes.len()
    }

    fn frame(&self, kind: Kind, dst: u64) -> Frame<'static> {
        Frame::new(kind, self.room, self.id, dst)
    }

    /// The next frame, if one comes soon.
    fn recv(&self) -> Option<Got> {
        let mut buf = [0; 2 * MAX_PAYLOAD];
        let (n, from) = self.socket.recv_from(&mut buf).ok()?;
        assert_eq!(from, self.relay);
        let f = decode(&buf[..n]).unwrap();
        Some(Got {
            kind: f.kind,
            flags: f.flags,
            src: f.src,
            dst: f.dst,
            port: f.port,
            seq: f.seq,
            payload: f.payload.to_vec(),
            len: n,
        })
    }

    /// Say hello with `payload` (a cookie and a key proof), and what the
    /// server answered.
    fn hello_with(&self, payload: &[u8; HELLO_LEN]) -> Got {
        self.send(
            self.frame(Kind::Hello, 0)
                .with_seq(77)
                .with_payload(payload),
        );
        let got = self.recv().expect("no answer to hello");
        assert_eq!(got.flags, FLAG_SERVER);
        assert_eq!(got.dst, self.id);
        if got.kind != Kind::Refused {
            // Its own hello's number (a refusal's is why).
            assert_eq!(got.seq, 77);
        }
        got
    }

    /// The cookie the server gives this address, for this room and id.
    fn cookie(&self) -> [u8; COOKIE_LEN] {
        let got = self.hello_with(&[0; HELLO_LEN]);
        assert_eq!(got.kind, Kind::Challenge, "{got:?}");
        got.payload.try_into().unwrap()
    }

    /// A hello's payload: `cookie`, and the proof of `key` with it.
    fn payload(&self, cookie: &[u8; COOKIE_LEN], key: Option<MemberKey>) -> [u8; HELLO_LEN] {
        let mut payload = [0; HELLO_LEN];
        payload[..COOKIE_LEN].copy_from_slice(cookie);
        if let Some(key) = key {
            payload[COOKIE_LEN..].copy_from_slice(&key.proof(self.room, self.id, cookie));
        }
        payload
    }

    /// Say hello as a client does: for a cookie, then with it (and the
    /// proof of `key`), and what the server answered the second time.
    fn hello(&self, key: Option<MemberKey>) -> Got {
        let cookie = self.cookie();
        self.hello_with(&self.payload(&cookie, key))
    }

    fn join(&self) {
        let got = self.hello(None);
        assert_eq!(got.kind, Kind::Hello, "{got:?}");
    }

    fn join_with(&self, key: MemberKey) {
        let got = self.hello(Some(key));
        assert_eq!(got.kind, Kind::Hello, "{got:?}");
    }

    fn refused(&self) -> Refusal {
        self.refused_with(None)
    }

    fn refused_with(&self, key: Option<MemberKey>) -> Refusal {
        let got = self.hello(key);
        assert_eq!(got.kind, Kind::Refused, "{got:?}");
        Refusal::from_code(got.seq)
    }

    fn data(&self, dst: u64, payload: &[u8]) {
        self.send(self.frame(Kind::Unreliable, dst).with_payload(payload));
    }

    fn bye(&self) {
        self.send(self.frame(Kind::Bye, 0));
    }

    /// Data frames received until none come for a moment.
    fn data_received(&self) -> Vec<Vec<u8>> {
        let data = self.all_received().into_iter();
        data.filter(|f| f.kind == Kind::Unreliable)
            .map(|f| f.payload)
            .collect()
    }

    /// Every frame received until none come for a moment (or for a second,
    /// as a client in the room pings it more often than that).
    fn all_received(&self) -> Vec<Got> {
        let start = Instant::now();
        let mut got = Vec::new();
        while start.elapsed() < Duration::from_secs(1) {
            match self.recv() {
                Some(f) => got.push(f),
                None => break,
            }
        }
        got
    }
}

fn stats_until(relay: &RelayThread, what: &str, done: impl Fn(&ServerStats) -> bool) {
    until(what, || done(&relay.stats()));
}

#[test]
fn members_of_a_room_reach_each_other_and_no_one_else() {
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    let (alpha, bravo, charlie) = (
        client(at, ROOM, 1, fast()),
        client(at, ROOM, 2, fast()),
        client(at, ROOM, 3, fast()),
    );
    // Bravo's id, in another room.
    let delta = client(at, OTHER_ROOM, 2, fast());
    joined(&[&alpha, &bravo, &charlie, &delta]);

    // One to one, with the port as it was.
    alpha.send_unreliable(2, 1000, b"to bravo").unwrap();
    assert_eq!(take(&bravo, 1), [(1, 1000, b"to bravo".to_vec())]);

    // To everyone else in the room.
    charlie
        .send_unreliable(BROADCAST, 1001, b"everyone")
        .unwrap();
    let everyone = || (3, 1001, b"everyone".to_vec());
    assert_eq!(take(&alpha, 1), [everyone()]);
    assert_eq!(take(&bravo, 1), [everyone()]);

    // Reliable, in order, both ways at once.
    for i in 0..50u32 {
        alpha
            .send_reliable(3, i, format!("alpha {i}").as_bytes())
            .unwrap();
        charlie
            .send_reliable(1, i, format!("charlie {i}").as_bytes())
            .unwrap();
    }
    let expect = |who: &str, src: u64| -> Vec<Packet> {
        (0..50u32)
            .map(|i| (src, i, format!("{who} {i}").into_bytes()))
            .collect()
    };
    assert_eq!(take(&charlie, 50), expect("alpha", 1));
    assert_eq!(take(&alpha, 50), expect("charlie", 3));

    // Delta's room has no one else: what it sends goes nowhere.
    delta.send_unreliable(1, 1000, b"anyone?").unwrap();
    delta.send_unreliable(BROADCAST, 1000, b"anyone?").unwrap();
    delta.send_reliable(1, 1000, b"anyone?").unwrap();
    stats_until(&relay, "the strays to be counted", |s| s.unknown_dst >= 2);
    quiet(&[&alpha, &bravo, &charlie, &delta]);

    // Every peer gets a round trip (from pings and acks) and stats.
    until("round trips", || {
        let stats = alpha.stats();
        stats.server_rtt.is_some() && stats.peers.iter().filter(|p| p.rtt.is_some()).count() == 2
    });
    let stats = alpha.stats();
    let to_charlie = stats.peers.iter().find(|p| p.id == 3).unwrap();
    // In: the broadcast and the 50 reliable ones.
    assert_eq!((to_charlie.packets_out, to_charlie.packets_in), (50, 51));
    until("acks", || {
        alpha.stats().peers.iter().all(|p| p.in_flight == 0)
    });
    assert_eq!(stats.packets_received, 51);
    assert_eq!((stats.malformed, stats.foreign, stats.too_big), (0, 0, 0));
    let server = relay.stats();
    assert_eq!((server.rooms, server.members), (2, 4));
    assert_eq!(server.unknown_sender, 0);
    // Each said hello for its cookie first (more than once if the answer
    // was slow), then with it.
    assert!(server.challenged >= 4, "{server:?}");
}

#[test]
fn reliable_packets_arrive_once_and_in_order_through_loss() {
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    // A quarter of every frame each sends is lost (data, acks, hellos,
    // keepalives), and the rest come 1-6 ms late, passing each other.
    let lossy = || ClientConfig {
        lag: Lag {
            delay: Duration::from_millis(1),
            jitter: Duration::from_millis(5),
            loss: 0.25,
        },
        ..fast()
    };
    let (alpha, bravo) = (client(at, ROOM, 1, lossy()), client(at, ROOM, 2, lossy()));
    joined(&[&alpha, &bravo]);
    const N: u32 = 100;
    for i in 0..N {
        alpha.send_reliable(2, 1, &i.to_le_bytes()).unwrap();
        alpha.send_unreliable(2, 2, &i.to_le_bytes()).unwrap();
        bravo.send_reliable(1, 1, &i.to_le_bytes()).unwrap();
    }
    for (to, from) in [(&bravo, 1), (&alpha, 2)] {
        let mut reliable = Vec::new();
        let mut unreliable = 0;
        until("every reliable packet", || {
            for (src, port, data) in drain(to) {
                assert_eq!(src, from);
                let n = u32::from_le_bytes(data.try_into().unwrap());
                match port {
                    1 => reliable.push(n),
                    _ => unreliable += 1,
                }
            }
            reliable.len() >= N as usize
        });
        // Each once, in order.
        assert_eq!(reliable, (0..N).collect::<Vec<_>>());
        if from == 1 {
            // Unreliable ones are lost as they were sent: some, not all.
            assert!(unreliable < N, "{unreliable}");
        }
    }
    until("every ack", || {
        alpha.stats().peers.iter().all(|p| p.in_flight == 0)
    });
    let stats = alpha.stats();
    assert!(stats.resends > 0 && stats.lag_lost > 0, "{stats:?}");
    assert_eq!(stats.gave_up, 0);
    // No reliable one comes twice, however late.
    std::thread::sleep(Duration::from_millis(100));
    for c in [&alpha, &bravo] {
        assert!(drain(c).iter().all(|p| p.1 != 1));
    }
}

#[test]
fn a_receiver_that_starts_over_gets_reliable_packets_at_once() {
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    let (alpha, bravo) = (client(at, ROOM, 1, fast()), client(at, ROOM, 2, fast()));
    joined(&[&alpha, &bravo]);
    alpha.send_reliable(2, 0, b"before").unwrap();
    assert_eq!(take(&bravo, 1), [(1, 0, b"before".to_vec())]);
    until("the ack", || {
        alpha.stats().peers.iter().all(|p| p.in_flight == 0)
    });
    // Bravo starts over (the launcher made a new client after a failure):
    // alpha's stream to it was open, its SYN long since taken.
    drop(bravo);
    let bravo = client(at, ROOM, 2, fast());
    joined(&[&bravo]);
    for i in 0..5u32 {
        alpha
            .send_reliable(2, i, format!("after {i}").as_bytes())
            .unwrap();
    }
    // Told "no stream", alpha starts a new one with them: all come, in
    // order, long before alpha would give up on the old one.
    let start = Instant::now();
    let got = take(&bravo, 5);
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
    let expect: Vec<Packet> = (0..5u32)
        .map(|i| (1, i, format!("after {i}").into_bytes()))
        .collect();
    assert_eq!(got, expect);
    let stats = alpha.stats();
    assert_eq!((stats.restarts, stats.gave_up), (1, 0), "{stats:?}");
    // And it goes on as normal.
    alpha.send_reliable(2, 9, b"later").unwrap();
    assert_eq!(take(&bravo, 1), [(1, 9, b"later".to_vec())]);
    quiet(&[&bravo]);
}

#[test]
fn a_late_no_stream_answer_starts_nothing_over() {
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    let alpha = client(at, ROOM, 1, fast());
    joined(&[&alpha]);
    // The receiver, made by hand.
    let bravo = Raw::new(at, ROOM, 2);
    bravo.join();
    // The next reliable frame from alpha that `wanted` picks (skipping
    // resends and pings).
    let next = |wanted: &dyn Fn(&Got) -> bool| -> Got {
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "nothing wanted came");
            if let Some(f) = bravo.recv() {
                if f.kind == Kind::Reliable && wanted(&f) {
                    return f;
                }
            }
        }
    };
    let ack = |seq: u32, expected: u32| {
        bravo.send(bravo.frame(Kind::Ack, 1).with_seq(seq).with_port(expected));
    };
    let no_stream = |seq: u32| {
        bravo.send(bravo.frame(Kind::Ack, 1).with_flags(FLAG_SYN).with_seq(seq));
    };
    alpha.send_reliable(2, 7, b"first").unwrap();
    let syn = next(&|f| f.flags == FLAG_SYN);
    let isn = syn.seq;
    let first = next(&|f| f.seq == isn.wrapping_add(1));
    assert_eq!((first.port, first.payload.as_slice()), (7, &b"first"[..]));
    // Bravo takes the SYN; then comes a "no stream" about the frame sent
    // behind it (as if bravo had sent it before the SYN got there, and
    // the ack had overtaken it). It means nothing.
    ack(isn, isn.wrapping_add(1));
    no_stream(isn.wrapping_add(1));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(alpha.stats().restarts, 0);
    ack(isn.wrapping_add(1), isn.wrapping_add(2));
    until("the ack", || {
        alpha.stats().peers.iter().all(|p| p.in_flight == 0)
    });
    // About a frame sent after the SYN was taken, it means bravo lost its
    // end: a new stream, with the frame in it.
    alpha.send_reliable(2, 8, b"second").unwrap();
    let second = next(&|f| f.seq == isn.wrapping_add(2));
    assert_eq!(second.payload, b"second");
    no_stream(second.seq);
    let new_syn = next(&|f| f.flags == FLAG_SYN && f.seq != isn);
    let again = next(&|f| f.seq == new_syn.seq.wrapping_add(1));
    assert_eq!((again.port, again.payload.as_slice()), (8, &b"second"[..]));
    assert_eq!(alpha.stats().restarts, 1);
}

#[test]
fn reliable_packets_once_acked_are_never_dropped() {
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    // `recv` holds only 4.
    let config = ClientConfig {
        recv_queue: 4,
        ..fast()
    };
    let bravo = client(at, ROOM, 2, config);
    joined(&[&bravo]);
    let alpha = Raw::new(at, ROOM, 1);
    alpha.join();
    let isn = 500u32;
    let reliable = |n: u32| {
        let frame = alpha
            .frame(Kind::Reliable, 2)
            .with_seq(isn + n)
            .with_port(n);
        alpha.send(frame.with_payload(b"r"));
    };
    alpha.send(
        alpha
            .frame(Kind::Reliable, 2)
            .with_flags(FLAG_SYN)
            .with_seq(isn),
    );
    // Four early (held, and acked), then three unreliable ones: one place
    // left in `recv`'s queue.
    for n in 2..=5 {
        reliable(n);
    }
    for _ in 0..3 {
        alpha.data(2, b"u");
    }
    std::thread::sleep(Duration::from_millis(50));
    // The gap filled: it and the four behind it all go to `recv`.
    reliable(1);
    let acks: Vec<(u32, u32)> = alpha
        .all_received()
        .into_iter()
        .filter(|f| f.kind == Kind::Ack)
        .map(|f| (f.seq - isn, f.port - isn))
        .collect();
    assert_eq!(acks, [(0, 1), (2, 1), (3, 1), (4, 1), (5, 1), (1, 6)]);
    // Now it's full: the next reliable frame is refused (not acked, so the
    // sender keeps it) until `recv` catches up.
    reliable(6);
    assert!(alpha.all_received().iter().all(|f| f.kind != Kind::Ack));
    let got: Vec<(u32, Vec<u8>)> = drain(&bravo).into_iter().map(|p| (p.1, p.2)).collect();
    let mut expect = vec![(0, b"u".to_vec()); 3];
    expect.extend((1..=5).map(|n| (n, b"r".to_vec())));
    assert_eq!(got, expect);
    reliable(6);
    assert_eq!(take(&bravo, 1), [(1, 6, b"r".to_vec())]);
    // Only the refused one was dropped, and it came again.
    assert_eq!(bravo.stats().queue_drops, 1);
}

#[test]
fn quiet_members_are_removed_and_empty_rooms_expire() {
    let relay = server(ServerConfig {
        member_timeout: Duration::from_millis(400),
        room_linger: Duration::from_millis(100),
        ..ServerConfig::default()
    });
    let at = relay.local_addr();
    // One that says hello and then nothing more...
    let ghost = Raw::new(at, ROOM, 9);
    ghost.join();
    // ...and one that keeps itself alive.
    let alpha = client(at, ROOM, 1, fast());
    joined(&[&alpha]);
    stats_until(&relay, "two members", |s| s.members == 2);
    stats_until(&relay, "the quiet one to go", |s| {
        s.members == 1 && s.timeouts == 1
    });
    // It's gone: nothing reaches it, and it isn't a member any more.
    alpha.send_unreliable(9, 0, b"still there?").unwrap();
    stats_until(&relay, "a stray", |s| s.unknown_dst == 1);
    assert!(ghost.data_received().is_empty());
    ghost.data(1, b"back");
    let told = ghost.recv().unwrap();
    assert_eq!(
        (told.kind, Refusal::from_code(told.seq)),
        (Kind::Refused, Refusal::NotMember)
    );
    // The one left says bye; the room is empty, and soon expires.
    drop(alpha);
    stats_until(&relay, "the room to expire", |s| {
        s.members == 0 && s.rooms == 0 && s.rooms_expired == 1
    });
    // It said bye, so it didn't time out.
    assert_eq!(relay.stats().timeouts, 1);
}

#[test]
fn one_member_cant_flood_the_others() {
    // A burst of 50 and next to nothing after, so the count doesn't hang
    // on how quickly the relay's thread gets to them.
    let relay = server(ServerConfig {
        rate: 1.0,
        burst: 50.0,
        ..ServerConfig::default()
    });
    let at = relay.local_addr();
    let (flood, victim, fair) = (
        Raw::new(at, ROOM, 1),
        Raw::new(at, ROOM, 2),
        Raw::new(at, ROOM, 3),
    );
    for raw in [&flood, &victim, &fair] {
        raw.join();
    }
    let start = Instant::now();
    for i in 0..500u32 {
        flood.data(2, &i.to_le_bytes());
    }
    let got = victim.data_received().len();
    let most = 50.0 + start.elapsed().as_secs_f64() + 1.0;
    assert!(got >= 45 && got as f64 <= most, "{got} of 500");
    let stats = relay.stats();
    assert!(stats.rate_limited >= 440, "{stats:?}");
    // The others aren't held back.
    for i in 0..10u32 {
        fair.data(2, &i.to_le_bytes());
    }
    assert_eq!(victim.data_received().len(), 10);
}

#[test]
fn strangers_are_dropped_and_told_to_say_hello() {
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    let member = Raw::new(at, ROOM, 1);
    member.join();
    // Data from someone who never said hello.
    let stranger = Raw::new(at, ROOM, 2);
    stranger.data(1, b"let me in");
    let told = stranger.recv().unwrap();
    assert_eq!(told.kind, Kind::Refused);
    assert_eq!(told.flags, FLAG_SERVER);
    assert_eq!(Refusal::from_code(told.seq), Refusal::NotMember);
    // Pretending to be a member from elsewhere is no better.
    let impostor = Raw::new(at, ROOM, 1);
    impostor.data(BROADCAST, b"it's me");
    assert_eq!(impostor.recv().unwrap().kind, Kind::Refused);
    // Told only so often: 20 a second, from one address.
    for _ in 0..100 {
        stranger.data(1, b"let me in");
    }
    let told = stranger.all_received().len();
    assert!((10..=30).contains(&told), "{told}");
    // Junk, and frames only the server may send.
    impostor.socket.send_to(b"hello", at).unwrap();
    impostor.send(
        Frame::new(Kind::Pong, ROOM, 1, 2)
            .with_flags(FLAG_SERVER)
            .with_seq(1),
    );
    stats_until(&relay, "it all counted", |s| {
        s.unknown_sender == 102 && s.malformed == 2
    });
    assert!(member.data_received().is_empty());
    assert_eq!(relay.stats().forwarded, 0);
}

#[test]
fn a_member_that_moves_is_followed_by_saying_hello_again() {
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    let (before, other) = (Raw::new(at, ROOM, 1), Raw::new(at, ROOM, 2));
    before.join();
    other.join();
    other.data(1, b"one");
    assert_eq!(before.data_received(), [b"one".to_vec()]);
    // A new address (a NAT rebind): hello from there moves it.
    let after = Raw::new(at, ROOM, 1);
    after.join();
    other.data(1, b"two");
    assert_eq!(after.data_received(), [b"two".to_vec()]);
    assert!(before.data_received().is_empty());
    before.data(2, b"old address");
    assert_eq!(before.recv().unwrap().kind, Kind::Refused);
    let stats = relay.stats();
    assert_eq!((stats.members, stats.forwarded), (2, 2));
}

#[test]
fn forged_hellos_sign_no_one_up_and_get_back_less_than_they_sent() {
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    // Someone else's address: hellos "from" it are what an attacker
    // forging it would send (made here from its own socket, which never
    // answers anything).
    let victim = Raw::new(at, ROOM, 100);
    let mut sent = 0;
    for id in 100..116 {
        let forged = Frame::new(Kind::Hello, ROOM, id, 0).with_payload(&[0; HELLO_LEN]);
        sent += victim.send(forged);
    }
    // The attacker, in the room from its own address, then broadcasts.
    let attacker = Raw::new(at, ROOM, 1);
    attacker.join();
    attacker.data(BROADCAST, &[0xee; 1000]);
    // The victim got one challenge per hello, each smaller than it, and
    // nothing else; no one was signed up at its address.
    let got = victim.all_received();
    assert_eq!(got.len(), 16);
    assert!(got.iter().all(|f| f.kind == Kind::Challenge));
    let back: usize = got.iter().map(|f| f.len).sum();
    assert!(back < sent, "{back} back for {sent}");
    let stats = relay.stats();
    assert_eq!((stats.rooms, stats.members, stats.forwarded), (1, 1, 0));

    // A cookie works only from the address it was sent to, for its room
    // and id.
    let cookie = attacker.cookie();
    let elsewhere = Raw::new(at, ROOM, 1);
    let got = elsewhere.hello_with(&elsewhere.payload(&cookie, None));
    assert_eq!(got.kind, Kind::Challenge);
    let other_id = Raw {
        socket: attacker.socket.try_clone().unwrap(),
        id: 2,
        ..Raw::new(at, ROOM, 2)
    };
    let got = other_id.hello_with(&other_id.payload(&cookie, None));
    assert_eq!(got.kind, Kind::Challenge);
    let other_room = Raw {
        socket: attacker.socket.try_clone().unwrap(),
        room: OTHER_ROOM,
        ..Raw::new(at, OTHER_ROOM, 1)
    };
    let got = other_room.hello_with(&other_room.payload(&cookie, None));
    assert_eq!(got.kind, Kind::Challenge);
    assert_eq!(relay.stats().members, 1);
    // From its own address it does (and the answer is smaller still).
    let got = attacker.hello_with(&attacker.payload(&cookie, None));
    assert_eq!(got.kind, Kind::Hello);
    assert!(got.len < 40 + HELLO_LEN);
}

#[test]
fn one_address_cant_fill_the_room_table() {
    let relay = server(ServerConfig {
        max_rooms: 8,
        rooms_per_ip: 4.0,
        ..ServerConfig::default()
    });
    let at = relay.local_addr();
    // Open a room, leave it, open the next...
    for room in 0..4 {
        let raw = Raw::new(at, room, 1);
        raw.join();
        raw.bye();
    }
    // ...until this address has opened its share for now.
    assert_eq!(Raw::new(at, 4, 1).refused(), Refusal::TooManyFromAddress);
    // Rooms only one member ever joined aren't kept once empty.
    stats_until(&relay, "the empty rooms to go", |s| s.rooms == 0);
    assert_eq!(relay.stats().rooms_expired, 4);
    // Someone else (another address) opens one as ever.
    if let Some(other) = Raw::on("127.0.0.2", at, 5, 1) {
        other.join();
        // A room two joined is kept a while for them to come back.
        let second = Raw::on("127.0.0.2", at, 5, 2).unwrap();
        second.join();
        other.bye();
        second.bye();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(relay.stats().rooms, 1);
    }
}

#[test]
fn junk_from_many_addresses_shuts_no_one_out() {
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    let member = Raw::new(at, ROOM, 1);
    member.join();
    // Hundreds of senders (new source ports), each sending junk and a frame
    // from no member.
    for _ in 0..300 {
        let junk = Raw::new(at, ROOM, 9);
        junk.socket.send_to(&[0], at).unwrap();
        junk.data(1, b"x");
    }
    stats_until(&relay, "the junk counted", |s| {
        s.malformed >= 250 && s.unknown_sender >= 250
    });
    // A new client still joins, and members still reach each other.
    let alpha = client(at, ROOM, 2, fast());
    joined(&[&alpha]);
    member.data(2, b"through");
    assert_eq!(take(&alpha, 1), [(1, 0, b"through".to_vec())]);
}

#[test]
fn rooms_members_and_addresses_are_limited() {
    // 17 to a room.
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    let members: Vec<Raw> = (0..17).map(|id| Raw::new(at, ROOM, id)).collect();
    for m in &members {
        m.join();
    }
    assert_eq!(Raw::new(at, ROOM, 17).refused(), Refusal::RoomFull);
    // One already in is answered again.
    members[0].join();
    // A broadcast reaches all the others.
    members[0].data(BROADCAST, b"all");
    for m in &members[1..] {
        let got = m.recv().unwrap();
        assert_eq!(
            (got.kind, got.src, got.payload),
            (Kind::Unreliable, 0, b"all".to_vec())
        );
    }
    assert!(members[0].recv().is_none());

    // Rooms, and members from one address.
    let relay = server(ServerConfig {
        max_rooms: 2,
        max_members_per_ip: 3,
        ..ServerConfig::default()
    });
    let at = relay.local_addr();
    let (a, b) = (Raw::new(at, 1, 1), Raw::new(at, 2, 1));
    a.join();
    b.join();
    assert_eq!(Raw::new(at, 3, 1).refused(), Refusal::TooManyRooms);
    let c = Raw::new(at, 1, 2);
    c.join();
    assert_eq!(Raw::new(at, 1, 3).refused(), Refusal::TooManyFromAddress);
    assert_eq!(relay.stats().refused, 2);
}

#[test]
fn issued_rooms_admit_their_players_and_no_one_else() {
    let relay = server(issued());
    let at = relay.local_addr();
    let handle = relay.handle();
    let key = |id: u64| handle.member_key(ROOM, id);
    // One that comes before its room is issued keeps trying...
    let alpha = client(at, ROOM, 1, keyed(key(1)));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(alpha.state(), State::Joining);
    // ...and gets in once it is.
    handle.issue(ROOM);
    let bravo = client(at, ROOM, 2, keyed(key(2)));
    joined(&[&alpha, &bravo]);
    alpha.send_unreliable(2, 5, b"in").unwrap();
    assert_eq!(take(&bravo, 1), [(1, 5, b"in".to_vec())]);
    // A room never issued: refused until the client gives up.
    let config = ClientConfig {
        hello_timeout: Duration::from_millis(300),
        ..keyed(key(1))
    };
    let stray = client(at, OTHER_ROOM, 1, config);
    until("it to give up", || stray.state() != State::Joining);
    assert_eq!(
        stray.state(),
        State::Failed(Failure::Refused(Refusal::NoSuchRoom))
    );
    assert_eq!(stray.send_unreliable(2, 0, b"x"), Err(SendError::Failed));
    // Closed: the members are told, try again, are refused, and give up.
    handle.revoke(ROOM);
    until("refusals", || {
        [&alpha, &bravo]
            .iter()
            .all(|c| c.state() == State::Failed(Failure::Refused(Refusal::NoSuchRoom)))
    });
}

#[test]
fn a_hello_right_after_its_room_is_issued_finds_it() {
    let relay = server(issued());
    let at = relay.local_addr();
    let handle = relay.handle();
    for n in 0..10 {
        let room = ROOM + n;
        let raw = Raw::new(at, room, 1);
        // The cookie first (it doesn't need the room), so the hello that
        // counts goes the moment the room is issued.
        let payload = raw.payload(&raw.cookie(), Some(handle.member_key(room, 1)));
        handle.issue(room);
        let got = raw.hello_with(&payload);
        assert_eq!(got.kind, Kind::Hello, "room {n}: {got:?}");
    }
}

#[test]
fn players_in_an_issued_room_cant_take_each_others_places() {
    let relay = server(issued());
    let at = relay.local_addr();
    let handle = relay.handle();
    handle.issue(ROOM);
    let key = |id: u64| handle.member_key(ROOM, id);
    let host = Raw::new(at, ROOM, 1);
    host.join_with(key(1));
    let player = Raw::new(at, ROOM, 2);
    player.join_with(key(2));
    // The player (who knows the room and the host's id) tries to become
    // the host from another address: without the host's key, or with its
    // own, it's refused.
    let impostor = Raw::new(at, ROOM, 1);
    assert_eq!(impostor.refused(), Refusal::BadKey);
    assert_eq!(impostor.refused_with(Some(key(2))), Refusal::BadKey);
    // Nor can anyone make up ids to fill the room.
    for id in 3..20 {
        let made_up = Raw::new(at, ROOM, id);
        assert_eq!(made_up.refused_with(Some(key(2))), Refusal::BadKey);
    }
    // A proof seen on the wire is no good from elsewhere (it's bound to
    // the cookie, which is bound to the address).
    let seen = host.payload(&host.cookie(), Some(key(1)));
    assert_eq!(impostor.hello_with(&seen).kind, Kind::Challenge);
    // The host is still the host.
    player.data(1, b"still you");
    assert_eq!(host.data_received(), [b"still you".to_vec()]);
    assert_eq!(relay.stats().members, 2);
    // The host itself, from a new address (a NAT rebind), moves.
    let moved = Raw::new(at, ROOM, 1);
    moved.join_with(key(1));
    player.data(1, b"over here");
    assert_eq!(moved.data_received(), [b"over here".to_vec()]);
    assert!(host.data_received().is_empty());
    // Keys belong to their room.
    let other = Raw::new(at, OTHER_ROOM, 1);
    handle.issue(OTHER_ROOM);
    stats_until(&relay, "the other room", |s| s.rooms == 2);
    assert_eq!(other.refused_with(Some(key(1))), Refusal::BadKey);
}

#[test]
fn a_revoked_room_stays_closed_for_a_while() {
    // Open rooms: without a memory of it, the members would make it
    // again at once.
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    let (alpha, bravo) = (client(at, ROOM, 1, fast()), client(at, ROOM, 2, fast()));
    joined(&[&alpha, &bravo]);
    relay.handle().revoke(ROOM);
    until("refusals", || {
        [&alpha, &bravo]
            .iter()
            .all(|c| c.state() == State::Failed(Failure::Refused(Refusal::NoSuchRoom)))
    });
    assert_eq!(relay.stats().rooms, 0);
    assert_eq!(Raw::new(at, ROOM, 3).refused(), Refusal::NoSuchRoom);
    // After the linger, it can be made again.
    let relay = server(ServerConfig {
        room_linger: Duration::from_millis(200),
        ..ServerConfig::default()
    });
    let at = relay.local_addr();
    Raw::new(at, ROOM, 1).join();
    relay.handle().revoke(ROOM);
    stats_until(&relay, "it closed", |s| s.rooms == 0);
    assert_eq!(Raw::new(at, ROOM, 2).refused(), Refusal::NoSuchRoom);
    std::thread::sleep(Duration::from_millis(500));
    Raw::new(at, ROOM, 2).join();
}

#[test]
fn packets_to_oneself_never_leave_this_pc() {
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    let alpha = client(at, ROOM, 1, fast());
    joined(&[&alpha]);
    alpha.send_unreliable(1, 1005, b"loopback").unwrap();
    alpha.send_reliable(1, 1006, b"reliably").unwrap();
    let got = take(&alpha, 2);
    assert_eq!(
        got,
        [
            (1, 1005, b"loopback".to_vec()),
            (1, 1006, b"reliably".to_vec())
        ]
    );
    let stats = relay.stats();
    assert_eq!((stats.forwarded, stats.unknown_dst), (0, 0));
    assert_eq!(alpha.stats().packets_sent, 2);
    // A reliable one with no room to wait in is refused, not lost.
    let full = client(
        at,
        ROOM,
        2,
        ClientConfig {
            recv_queue: 1,
            ..fast()
        },
    );
    joined(&[&full]);
    full.send_reliable(2, 0, b"one").unwrap();
    assert_eq!(full.send_reliable(2, 0, b"two"), Err(SendError::Full));
    assert_eq!(drain(&full), [(2, 0, b"one".to_vec())]);
}

#[test]
fn sends_and_receives_check_sizes() {
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    let (alpha, bravo) = (client(at, ROOM, 1, fast()), client(at, ROOM, 2, fast()));
    joined(&[&alpha, &bravo]);
    let big = [7; MAX_PAYLOAD + 1];
    assert_eq!(
        alpha.send_unreliable(2, 0, &big),
        Err(SendError::TooBig(MAX_PAYLOAD + 1))
    );
    assert_eq!(
        alpha.send_reliable(BROADCAST, 0, b"x"),
        Err(SendError::BroadcastReliable)
    );
    // Up to 4 KiB, either way (fragmented by IP over the MTU).
    alpha.send_unreliable(2, 0, &big[..MAX_PAYLOAD]).unwrap();
    alpha.send_reliable(2, 1, &big[..MTU_PAYLOAD + 1]).unwrap();
    let got = take(&bravo, 2);
    assert_eq!(got[0], (1, 0, big[..MAX_PAYLOAD].to_vec()));
    assert_eq!(got[1], (1, 1, big[..MTU_PAYLOAD + 1].to_vec()));
    let stats = alpha.stats();
    assert_eq!((stats.largest_sent, stats.over_mtu), (MAX_PAYLOAD, 2));
    assert_eq!(bravo.stats().largest_received, MAX_PAYLOAD);
    alpha.send_unreliable(2, 0, &big[..MAX_PAYLOAD]).unwrap();
    alpha.send_unreliable(2, 0, b"small").unwrap();
    // Too big for the buffer: dropped, not cut short; the next one comes.
    let mut buf = [0; 64];
    let mut got = None;
    until("the small one", || {
        got = bravo.recv(&mut buf);
        got.is_some()
    });
    assert_eq!(got, Some((1, 0, 5)));
    assert_eq!(&buf[..5], b"small");
    assert_eq!(bravo.recv(&mut buf), None);
    assert_eq!(bravo.stats().too_big, 1);
}

#[test]
fn a_client_gives_up_when_no_relay_answers() {
    // Takes datagrams, answers none.
    let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
    let config = ClientConfig {
        hello_timeout: Duration::from_millis(200),
        ..fast()
    };
    let alpha = client(silent.local_addr().unwrap(), ROOM, 1, config);
    assert_eq!(alpha.state(), State::Joining);
    until("it to give up", || alpha.state() != State::Joining);
    assert_eq!(alpha.state(), State::Failed(Failure::Timeout));
    assert_eq!(alpha.send_reliable(2, 0, b"x"), Err(SendError::Failed));
    assert!(alpha.stats().frames_sent >= 2);
}

#[test]
fn clients_join_again_after_the_relay_restarts() {
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    let (alpha, bravo) = (client(at, ROOM, 1, fast()), client(at, ROOM, 2, fast()));
    joined(&[&alpha, &bravo]);
    alpha.send_reliable(2, 0, b"before").unwrap();
    assert_eq!(take(&bravo, 1), [(1, 0, b"before".to_vec())]);
    relay.stop();
    // Sent while it's down: kept, and resent.
    alpha.send_reliable(2, 0, b"during").unwrap();
    // The same port again (nothing holds a UDP port after it's closed),
    // and a new secret: the old cookies are no good, so they get new ones.
    let relay = RelayServer::bind(at, ServerConfig::default())
        .unwrap()
        .with_log(Arc::new(|_| {}))
        .spawn()
        .unwrap();
    // It doesn't know them; their next keepalive is refused, and they say
    // hello again.
    stats_until(&relay, "both back", |s| s.members == 2);
    joined(&[&alpha, &bravo]);
    alpha.send_reliable(2, 0, b"after").unwrap();
    let got = take(&bravo, 2);
    assert_eq!(got, [(1, 0, b"during".to_vec()), (1, 0, b"after".to_vec())]);
}

#[test]
fn reliable_frames_are_put_in_order_across_the_sequence_wrap() {
    let relay = server(ServerConfig::default());
    let at = relay.local_addr();
    let bravo = client(at, ROOM, 2, fast());
    joined(&[&bravo]);
    // A sender made by hand, its stream starting just short of the wrap.
    let alpha = Raw::new(at, ROOM, 1);
    alpha.join();
    let syn = |seq: u32| {
        alpha.send(
            alpha
                .frame(Kind::Reliable, 2)
                .with_flags(FLAG_SYN)
                .with_seq(seq),
        )
    };
    let reliable = |seq: u32, data: &[u8]| {
        let frame = alpha.frame(Kind::Reliable, 2).with_seq(seq).with_port(seq);
        alpha.send(frame.with_payload(data));
    };
    let isn = u32::MAX - 2;
    let seq = |n: u32| isn.wrapping_add(n);
    // Before its stream is open: dropped, and answered "no stream" (so a
    // sender whose SYN was taken long ago would start over; this one's SYN
    // is on its way, so it just sends the frame again).
    reliable(seq(2), b"b");
    std::thread::sleep(Duration::from_millis(20));
    syn(isn);
    reliable(seq(1), b"a");
    // Early, twice; then a duplicate of one handed out.
    reliable(seq(3), b"c");
    reliable(seq(3), b"c");
    reliable(seq(4), b"d");
    reliable(seq(1), b"a");
    assert_eq!(take(&bravo, 1), [(1, seq(1), b"a".to_vec())]);
    reliable(seq(2), b"b");
    let got = take(&bravo, 3);
    let expect = [
        (1, seq(2), b"b".to_vec()),
        (1, 0, b"c".to_vec()),
        (1, 1, b"d".to_vec()),
    ];
    assert_eq!(got, expect);
    // Every frame of the open stream is acked, with the next seq expected.
    let mut acks = Vec::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(200) {
        if let Some(got) = alpha.recv().filter(|f| f.kind == Kind::Ack) {
            acks.push((got.seq, got.port, got.flags));
        }
    }
    let expect = [
        (seq(2), 0, FLAG_SYN),
        (isn, seq(1), 0),
        (seq(1), seq(2), 0),
        (seq(3), seq(2), 0),
        (seq(3), seq(2), 0),
        (seq(4), seq(2), 0),
        (seq(1), seq(2), 0),
        (seq(2), seq(5), 0),
    ];
    assert_eq!(acks, expect);
    // A new SYN: the sender started over.
    syn(1000);
    reliable(1001, b"e");
    assert_eq!(take(&bravo, 1), [(1, 1001, b"e".to_vec())]);
    quiet(&[&bravo]);
    assert_eq!(bravo.stats().duplicates, 2);
}
