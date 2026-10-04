use super::*;
use crate::client::{LiveClient, LiveEvent, Profile};
use ed25519_dalek::{Signer, SigningKey};
use h2net::live::kind;
use h2sim::game::Writer;
use std::net::Ipv4Addr;

/// Seconds between polls.
const STEP: f64 = 0.05;

/// A folder of a test's own, removed afterwards.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!("h2live-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A server and the PCs signed in to it, on a clock of their own.
struct World {
    /// The server's data folder, and the PCs' (for their stat cards).
    data: TempDir,
    home: TempDir,
    server: Server,
    now: f64,
    pcs: Vec<LiveClient>,
    /// What each PC heard, in order.
    events: Vec<Vec<LiveEvent>>,
    /// PCs that stopped polling.
    frozen: Vec<usize>,
}

/// PC number `n`'s address.
fn ip(n: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(10, 0, 0, n))
}

/// PC number `n`'s key.
fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}

fn profile(gamertag: &str) -> Profile {
    Profile {
        gamertag: gamertag.to_string(),
        look: Look::default_for(3),
        maps: vec![("lockout".into(), 1), ("midship".into(), 2)],
        guests: 0,
    }
}

impl World {
    fn new(name: &str) -> World {
        let data = TempDir::new(&format!("{name}-data"));
        let home = TempDir::new(&format!("{name}-home"));
        let server = Server::open(&data.0).unwrap();
        World {
            data,
            home,
            server,
            now: 0.0,
            pcs: Vec::new(),
            events: Vec::new(),
            frozen: Vec::new(),
        }
    }

    /// Start the server again from its data folder, everyone signed out.
    fn restart(&mut self) {
        self.server = Server::open(&self.data.0).unwrap();
        self.pcs.clear();
        self.events.clear();
        self.frozen.clear();
    }

    /// PC number `n` connects and signs in with `profile`. Returns its
    /// index in `pcs`.
    fn connect_as(&mut self, n: u8, profile: &Profile) -> usize {
        let (server_end, pc_end) = Connection::pair();
        self.server.accept(server_end, Route::Live, ip(n), self.now);
        let card = self.home.0.join(format!("card-{n}.txt"));
        let pc = LiveClient::new(pc_end, key(n), profile, &card, self.now);
        self.pcs.push(pc);
        self.events.push(Vec::new());
        self.pcs.len() - 1
    }

    fn connect(&mut self, n: u8, gamertag: &str) -> usize {
        self.connect_as(n, &profile(gamertag))
    }

    /// PC number `n` signs in as `gamertag` (or what the server gives it).
    fn sign_in(&mut self, n: u8, gamertag: &str) -> usize {
        let i = self.connect(n, gamertag);
        self.until(|w| w.pcs[i].signed_in());
        i
    }

    /// A connection to the server that the test speaks through itself.
    fn raw(&mut self, n: u8) -> Connection {
        let (server_end, pc_end) = Connection::pair();
        self.server.accept(server_end, Route::Live, ip(n), self.now);
        pc_end
    }

    fn step(&mut self) {
        self.now += STEP;
        self.server.poll(self.now);
        for (i, pc) in self.pcs.iter_mut().enumerate() {
            if !self.frozen.contains(&i) {
                self.events[i].extend(pc.poll(self.now));
            }
        }
    }

    fn run(&mut self, seconds: f64) {
        let end = self.now + seconds;
        while self.now < end {
            self.step();
        }
    }

    /// Run until `done`, or fail after a minute.
    fn until(&mut self, done: impl Fn(&World) -> bool) {
        let end = self.now + 60.0;
        while !done(self) {
            assert!(self.now < end, "timed out");
            self.step();
        }
    }

    fn welcome(&self, i: usize) -> &Welcome {
        self.pcs[i].view.welcome.as_ref().unwrap()
    }

    fn refused(&self, i: usize) -> Option<&str> {
        self.events[i].iter().find_map(|e| match e {
            LiveEvent::Refused(why) => Some(why.as_str()),
            _ => None,
        })
    }

    fn accounts_txt(&self) -> String {
        std::fs::read_to_string(self.data.0.join("accounts.txt")).unwrap_or_default()
    }
}

/// Everything that came over a raw connection.
fn heard(conn: &mut Connection) -> Vec<ToPc> {
    let messages = conn.receive().unwrap_or_default();
    messages
        .into_iter()
        .map(|(kind, body)| ToPc::read(kind, &body).unwrap())
        .collect()
}

#[test]
fn a_pc_signs_in_with_its_key() {
    let mut w = World::new("sign-in");
    std::fs::write(w.data.0.join("motd.txt"), "Have fun\n").unwrap();
    w.restart();
    let a = w.sign_in(1, "noble six");
    let welcome = w.welcome(a).clone();
    assert_eq!(
        welcome.account,
        store::account_id(key(1).verifying_key().as_bytes())
    );
    assert_eq!(welcome.gamertag, "NOBLE SIX");
    assert_eq!((welcome.best, welcome.levels.len()), (1, 0));
    assert_eq!(w.pcs[a].view.motd, "Have fun");
    // Every playlist, with a level in the ranked ones.
    let playlists = &w.pcs[a].view.playlists;
    assert_eq!(playlists.len(), crate::playlists::built_in().len());
    assert!(playlists.iter().all(|p| p.level == p.ranked as u8));
    assert_eq!(w.server.players_online(), 1);
    // The account is on disk.
    let account = w.server.account(welcome.account).unwrap();
    assert_eq!((account.seq, account.look), (1, Look::default_for(3)));
    assert_eq!(w.accounts_txt(), account.lines());
}

#[test]
fn a_bad_signature_is_refused() {
    let mut w = World::new("bad-signature");
    let mut conn = w.raw(1);
    let login = Login {
        key: key(1).verifying_key().to_bytes(),
        gamertag: "IMPOSTOR".into(),
        look: Look::default(),
        card: String::new(),
        maps: Vec::new(),
        guests: 0,
    };
    ToServer::Login(login).send(&mut conn);
    conn.flush().unwrap();
    w.step();
    let [ToPc::Challenge { nonce, .. }] = heard(&mut conn)[..] else {
        panic!("no challenge");
    };
    // Signed with another key than the one it signs in with.
    let proof = live::proof(&nonce, key(1).verifying_key().as_bytes());
    let signature = key(2).sign(&proof).to_bytes();
    ToServer::Prove(signature).send(&mut conn);
    conn.flush().unwrap();
    w.step();
    assert_eq!(
        heard(&mut conn),
        [ToPc::Refused(live::BAD_SIGNATURE.into())]
    );
    assert_eq!(w.server.players_online(), 0);
    assert_eq!(w.accounts_txt(), "");
    // The server lets the connection go.
    w.run(3.0);
    assert!(conn.receive().is_err());
}

#[test]
fn gamertags_are_unique_whatever_the_case() {
    let mut w = World::new("gamertags");
    let kat = w.sign_in(1, "Kat");
    // Someone else can't have it, in any case.
    for name in ["KAT", "kat", " kat "] {
        let other = w.connect(2, name);
        w.until(|w| w.refused(other).is_some());
        assert_eq!(w.refused(other), Some(live::GAMERTAG_TAKEN));
    }
    assert_eq!(w.server.players_online(), 1);
    // An account asking for a gamertag that's taken keeps its own.
    let jun = w.sign_in(3, "Jun");
    assert_eq!(w.welcome(jun).gamertag, "JUN");
    let again = w.sign_in(3, "kat");
    assert_eq!(w.welcome(again).gamertag, "JUN");
    // Signing in again elsewhere signs the old PC out.
    w.until(|w| w.refused(jun).is_some());
    assert_eq!(w.refused(jun), Some(live::SIGNED_IN_ELSEWHERE));
    assert!(w.pcs[kat].signed_in());
    assert_eq!(w.server.players_online(), 2);
}

#[test]
fn an_older_game_is_told_to_update() {
    let mut w = World::new("protocol");
    let mut conn = w.raw(1);
    let mut old = Writer::default();
    old.u32(live::MAGIC);
    old.u32(h2net::PROTOCOL - 1);
    old.str("whatever came next then");
    conn.send(kind::LOGIN, &old.0);
    conn.flush().unwrap();
    w.step();
    assert_eq!(
        heard(&mut conn),
        [ToPc::Refused(live::UPDATE_YOUR_GAME.into())]
    );
    assert_eq!(w.server.connections(), 0);
}

#[test]
fn players_rename_themselves() {
    let mut w = World::new("rename");
    let a = w.sign_in(1, "Carter");
    let b = w.sign_in(2, "Emile");
    let id = w.welcome(a).account;
    w.pcs[a].send(ToServer::Profile {
        gamertag: "Kat".into(),
        look: Look::default_for(7),
    });
    w.until(|w| w.welcome(a).gamertag == "KAT");
    let account = w.server.account(id).unwrap();
    assert_eq!((account.seq, account.look), (2, Look::default_for(7)));
    assert!(w.accounts_txt().contains(" KAT "));
    // The old gamertag is free again, and a taken one stays taken.
    w.pcs[b].send(ToServer::Profile {
        gamertag: "carter".into(),
        look: Look::default(),
    });
    w.until(|w| w.welcome(b).gamertag == "CARTER");
    w.pcs[a].send(ToServer::Profile {
        gamertag: "Carter".into(),
        look: Look::default_for(7),
    });
    w.until(|w| !w.pcs[a].view.notices.is_empty());
    assert_eq!(w.pcs[a].view.notices, [live::GAMERTAG_TAKEN]);
    assert_eq!(w.welcome(a).gamertag, "KAT");
    assert_eq!(w.server.account(id).unwrap().seq, 2);
}

#[test]
fn accounts_are_kept_across_a_restart() {
    let mut w = World::new("restart");
    let a = w.sign_in(1, "Jorge");
    let id = w.welcome(a).account;
    let before = w.server.account(id).unwrap().clone();
    w.restart();
    // Someone else can't take the gamertag after the restart.
    let other = w.connect(2, "JORGE");
    w.until(|w| w.refused(other).is_some());
    assert_eq!(w.refused(other), Some(live::GAMERTAG_TAKEN));
    // And its owner signs in to the same account, unchanged.
    let a = w.sign_in(1, "Jorge");
    assert_eq!(w.welcome(a).account, id);
    assert_eq!(w.server.account(id), Some(&before));
}

#[test]
fn round_trips_are_measured_both_ways() {
    let mut w = World::new("round-trips");
    let a = w.sign_in(1, "Six");
    w.run(5.0);
    // Each end answers on its next poll, one step later.
    let id = w.welcome(a).account;
    assert_eq!(w.server.round_trip(id), Some(50));
    assert_eq!(w.pcs[a].view.round_trip, Some(50));
}

#[test]
fn quiet_pcs_are_signed_out() {
    let mut w = World::new("quiet");
    let a = w.sign_in(1, "Alpha");
    let b = w.sign_in(2, "Bravo");
    w.frozen.push(a);
    w.run(13.0);
    assert_eq!(w.server.players_online(), 2);
    w.run(3.0);
    assert_eq!(w.server.players_online(), 1);
    // Once the server lets go, the PC finds the connection gone (after
    // what was sent before).
    w.run(3.0);
    let events = [w.pcs[a].poll(w.now), w.pcs[a].poll(w.now)].concat();
    assert!(matches!(events[..], [LiveEvent::Lost(_)]));
    // A PC gives up on a server that goes quiet too.
    let t = w.now;
    while w.now < t + 16.0 {
        w.now += STEP;
        w.events[b].extend(w.pcs[b].poll(w.now));
    }
    assert!(!w.pcs[b].signed_in());
    assert_eq!(
        w.events[b].last(),
        Some(&LiveEvent::Lost("the server stopped answering".into()))
    );
}

#[test]
fn the_server_keeps_its_limits() {
    let mut w = World::new("limits");
    // Ten sign-ins a minute from one address.
    for _ in 0..10 {
        w.sign_in(1, "Same");
    }
    let eleventh = w.connect(1, "Same");
    w.until(|w| w.refused(eleventh).is_some());
    assert_eq!(w.refused(eleventh), Some(live::TOO_MANY_SIGN_INS));
    w.run(60.0);
    w.sign_in(1, "Same");

    // Too big a message, a broken one, or one before signing in.
    let before = w.server.connections();
    let mut big = w.raw(2);
    big.send(kind::PING, &vec![0; live::MAX_MESSAGE]);
    let mut broken = w.raw(3);
    broken.send(99, &[]);
    let mut early = w.raw(4);
    ToServer::Invite(1).send(&mut early);
    for conn in [&mut big, &mut broken, &mut early] {
        conn.flush().unwrap();
    }
    assert_eq!(w.server.connections(), before + 3);
    w.step();
    assert_eq!(w.server.connections(), before);

    // A PC that never signs in is let go after 15 seconds.
    let _silent = w.raw(5);
    w.run(14.0);
    assert_eq!(w.server.connections(), before + 1);
    w.run(2.0);
    assert_eq!(w.server.connections(), before);

    // Relay legs count too, and wait 20 seconds for their other end.
    let mut legs = Vec::new();
    while w.server.connections() < MAX_CONNECTIONS {
        let (server_end, mut pc_end) = Connection::pair();
        w.server.accept(server_end, Route::Link, ip(6), w.now);
        ToServer::LinkHello {
            token: [1; 16],
            account: 1,
        }
        .send(&mut pc_end);
        pc_end.flush().unwrap();
        legs.push(pc_end);
    }
    let full = w.connect(7, "Late");
    w.step();
    assert_eq!(w.refused(full), Some(live::SERVER_FULL));
    w.run(20.0);
    assert_eq!(w.server.connections(), before);
    w.sign_in(7, "Late");
}
