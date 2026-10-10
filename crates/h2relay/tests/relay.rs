//! The relay end to end on this PC: a server and clients on 127.0.0.1
//! (ports chosen by the system), plus hand-made frames from plain sockets
//! where a test needs a member to misbehave or go quiet.

use h2relay::frame::{FLAG_SERVER, FLAG_SYN, MAX_PAYLOAD};
use h2relay::{
    decode, Admission, ClientConfig, Failure, Frame, Kind, Lag, Refusal, RelayClient, RelayServer,
    RelayThread, SendError, ServerConfig, ServerStats, State, BROADCAST,
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
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        Raw {
            socket,
            relay,
            room,
            id,
        }
    }

    fn send(&self, frame: Frame) {
        self.socket
            .send_to(&frame.to_vec().unwrap(), self.relay)
            .unwrap();
    }

    fn frame(&self, kind: Kind, dst: u64) -> Frame<'static> {
        Frame::new(kind, self.room, self.id, dst)
    }

    /// The next frame, if one comes soon.
    fn recv(&self) -> Option<Got> {
        let mut buf = [0; 2048];
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
        })
    }

    /// Say hello, and what the server answered.
    fn hello(&self) -> Got {
        self.send(self.frame(Kind::Hello, 0).with_seq(77));
        let got = self.recv().expect("no answer to hello");
        assert_eq!(got.flags, FLAG_SERVER);
        assert_eq!(got.dst, self.id);
        if got.kind == Kind::Hello {
            // Its own hello's number (a refusal's is why).
            assert_eq!(got.seq, 77);
        }
        got
    }

    fn join(&self) {
        let got = self.hello();
        assert_eq!(got.kind, Kind::Hello, "{got:?}");
    }

    fn refused(&self) -> Refusal {
        let got = self.hello();
        assert_eq!(got.kind, Kind::Refused, "{got:?}");
        Refusal::from_code(got.seq)
    }

    fn data(&self, dst: u64, payload: &[u8]) {
        self.send(self.frame(Kind::Unreliable, dst).with_payload(payload));
    }

    /// Data frames received until none come for a moment.
    fn data_received(&self) -> Vec<Vec<u8>> {
        let mut got = Vec::new();
        while let Some(f) = self.recv() {
            if f.kind == Kind::Unreliable {
                got.push(f.payload);
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
    let relay = server(ServerConfig {
        rate: 100.0,
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
    let sent_in = start.elapsed().as_secs_f64();
    let got = victim.data_received().len();
    // The burst (less the hello), and what came back meanwhile.
    let most = 50.0 + 100.0 * sent_in + 5.0;
    assert!(
        got >= 40 && got as f64 <= most,
        "{got} of 500 in {sent_in} s"
    );
    let stats = relay.stats();
    assert!(stats.rate_limited >= 400, "{stats:?}");
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
    // Told only now and then.
    stranger.data(1, b"let me in");
    assert!(stranger.recv().is_none());
    // Pretending to be a member from elsewhere is no better.
    let impostor = Raw::new(at, ROOM, 1);
    impostor.data(BROADCAST, b"it's me");
    assert_eq!(impostor.recv().unwrap().kind, Kind::Refused);
    // Junk, and frames only the server may send.
    impostor.socket.send_to(b"hello", at).unwrap();
    impostor.send(
        Frame::new(Kind::Pong, ROOM, 1, 2)
            .with_flags(FLAG_SERVER)
            .with_seq(1),
    );
    stats_until(&relay, "it all counted", |s| {
        s.unknown_sender == 3 && s.malformed == 2
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
fn issued_rooms_admit_only_rooms_the_server_issued() {
    let relay = server(ServerConfig {
        admission: Admission::Issued,
        ..ServerConfig::default()
    });
    let at = relay.local_addr();
    let early = client(at, ROOM, 1, fast());
    until("a refusal", || early.state() != State::Joining);
    assert_eq!(
        early.state(),
        State::Failed(Failure::Refused(Refusal::NoSuchRoom))
    );
    assert_eq!(early.send_unreliable(2, 0, b"x"), Err(SendError::Failed));
    relay.handle().issue(ROOM);
    stats_until(&relay, "the room", |s| s.rooms == 1);
    let (alpha, bravo) = (client(at, ROOM, 1, fast()), client(at, ROOM, 2, fast()));
    joined(&[&alpha, &bravo]);
    alpha.send_unreliable(2, 5, b"in").unwrap();
    assert_eq!(take(&bravo, 1), [(1, 5, b"in".to_vec())]);
    // Closed: the members are told they aren't in it, try again, and are
    // refused.
    relay.handle().revoke(ROOM);
    until("refusals", || {
        [&alpha, &bravo]
            .iter()
            .all(|c| c.state() == State::Failed(Failure::Refused(Refusal::NoSuchRoom)))
    });
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
    // The same port again (nothing holds a UDP port after it's closed).
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
    // Before its stream is open: dropped, not acked, so it's sent again.
    reliable(seq(2), b"b");
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
            acks.push((got.seq, got.port));
        }
    }
    let expect = [
        (isn, seq(1)),
        (seq(1), seq(2)),
        (seq(3), seq(2)),
        (seq(3), seq(2)),
        (seq(4), seq(2)),
        (seq(1), seq(2)),
        (seq(2), seq(5)),
    ];
    assert_eq!(acks, expect);
    // A new SYN: the sender started over.
    syn(1000);
    reliable(1001, b"e");
    assert_eq!(take(&bravo, 1), [(1, 1001, b"e".to_vec())]);
    quiet(&[&bravo]);
    assert_eq!(bravo.stats().duplicates, 2);
}
