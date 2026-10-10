//! `--live <server>`: play a match h2live makes, instead of one a session
//! file describes. The launcher signs in to h2live as a launcher (with its
//! own key, kept in its folder), searches a launcher playlist, and when a
//! match forms (LAUNCHER_MATCH) writes the session from it: the relay beside
//! the server, the match's room and this PC's key for it, one machine per
//! player, and who hosts. The engine then starts on the match's map and
//! variant as it does with `--session`.
//!
//! While the engine plays, a thread keeps the sign-in alive and tells the
//! server what the engine does: HOSTING once the host's engine is running
//! (after the server asked it to host), JOINED when a joining PC's engine
//! has loaded the map (it joined the host's game to get there), and
//! LAUNCHER_RESULT when the engine ends the game (it hands its results to
//! host slot 6 on every PC when the round ends), and LEFT_MATCH if the
//! launcher closes before that. The results block isn't read yet, so the
//! result says only that the game finished: the server keeps the match
//! unrated, and nobody who closes after the end counts as having quit.

use crate::session::{Player, Session};
use h2live::client::{LiveClient, LiveEvent, Profile};
use h2net::live::{LauncherMatch, LauncherResult, ToServer};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

/// h2live's sign-in port when the address gives none.
pub const DEFAULT_PORT: u16 = 47050;
/// How often the thread polls the server while the engine plays.
const POLL: Duration = Duration::from_millis(50);
/// How long a closing launcher that sent its result waits for the server's
/// verdict (MATCH_OVER), so the log has it.
const VERDICT_WAIT: Duration = Duration::from_secs(5);

/// The address to dial for `server`: a `ws://` or `wss://` URL as given,
/// otherwise `host` or `host:port` (port 47050 when left out) on `/live`.
pub fn server_url(server: &str) -> String {
    let s = server.trim();
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("ws://") || lower.starts_with("wss://") {
        return s.to_string();
    }
    let has_port = match s.rsplit_once(':') {
        // An IPv6 address in brackets has colons of its own.
        Some((host, port)) => {
            !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) && {
                !host.contains(':') || host.ends_with(']')
            }
        }
        None => false,
    };
    if has_port {
        format!("ws://{s}/live")
    } else {
        format!("ws://{s}:{DEFAULT_PORT}/live")
    }
}

/// The session a launcher's match means for this PC, and which of its
/// machines this PC is. `server` is the address the server was dialled at:
/// the relay listens beside it, on the port the match names.
///
/// Each player is on a machine of their own (launchers have no splitscreen
/// guests), and their relay id is both that machine's id (the engine's
/// packets are addressed by it, and the relay knows members by it) and
/// their XUID; the server keeps an account's relay id the same in every
/// match. In team games each player keeps their team; in free-for-all
/// games each gets a team of their own, as the two-launcher runs did.
pub fn session_from(game: &LauncherMatch, server: IpAddr) -> Result<(Session, usize), String> {
    let n = game.players.len();
    if n == 0 || n > crate::options::PEER_SLOTS {
        return Err(format!("the match has {n} players"));
    }
    let machines: Vec<u64> = game.players.iter().map(|p| p.relay_id).collect();
    let players = game
        .players
        .iter()
        .enumerate()
        .map(|(i, p)| Player {
            xuid: p.relay_id,
            machine: i,
            team: if game.teams {
                i32::from(p.team)
            } else {
                i as i32
            },
            // The engine's names are 15 characters at most.
            name: Some(p.gamertag.chars().take(15).collect()),
        })
        .collect();
    let host = machines
        .iter()
        .position(|&m| m == game.host)
        .ok_or("the match's host isn't one of its players")?;
    let me = machines
        .iter()
        .position(|&m| m == game.relay.id)
        .ok_or("this PC isn't one of the match's players")?;
    let key: String = game.relay.key.iter().map(|b| format!("{b:02x}")).collect();
    let session = Session {
        relay: SocketAddr::new(server, game.relay.port).to_string(),
        room: game.relay.room,
        key: Some(key),
        // Any value works as long as every machine has the same: the match.
        secure: game.id.max(1),
        host,
        machines,
        players,
        threshold: None,
        me: Some(me),
    };
    session.check()?;
    Ok((session, me))
}

/// MCC's multiplayer maps in `maps_dir` (`halo2\h2_maps_win64_dx11`), each
/// with the hash h2live tells copies apart by, so the server only picks
/// maps every player has. Only the file's first bytes and its length are
/// read, and only the hash leaves the PC.
pub fn mcc_maps(maps_dir: &Path) -> Vec<(String, u64)> {
    crate::maps::MULTIPLAYER
        .iter()
        .filter_map(|e| {
            let path = maps_dir.join(format!("{}.map", e.file));
            let hash = h2net::live::map_hash(&path).ok()?;
            Some((e.file.to_string(), hash))
        })
        .collect()
}

/// Signed in to h2live, before a match.
pub struct Lobby {
    client: LiveClient,
    start: Instant,
    log: Log,
}

impl Lobby {
    /// Dial `url` and sign in as the holder of the key at `key_path` (made
    /// the first time), with the stat card at `card_path`, as `profile`
    /// says. Waits up to `timeout` for the server's welcome.
    pub fn sign_in(
        url: &str,
        key_path: &Path,
        card_path: &Path,
        profile: &Profile,
        log: Log,
        timeout: Duration,
    ) -> Result<Lobby, String> {
        let key = h2live::client::identity(key_path)
            .map_err(|e| format!("the key at {}: {e}", key_path.display()))?;
        let conn = h2net::dial(url, timeout)
            .recv()
            .map_err(|_| "dialling stopped".to_string())??;
        let start = Instant::now();
        let client = LiveClient::launcher(conn, key, profile, crate::BUILD, card_path, 0.0);
        let mut lobby = Lobby { client, start, log };
        lobby.wait(timeout, |_, e| match e {
            LiveEvent::Welcomed => Some(Ok(())),
            LiveEvent::Refused(why) => Some(Err(format!("the server turned us away: {why}"))),
            _ => None,
        })??;
        if let Some(w) = &lobby.client.view.welcome {
            (lobby.log)(&format!(
                "live: signed in to {url} as {:?} (account {:#018x})",
                w.gamertag, w.account
            ));
        }
        Ok(lobby)
    }

    /// Search `playlist` and wait up to `timeout` for a match.
    pub fn find_match(&mut self, playlist: u8, timeout: Duration) -> Result<LauncherMatch, String> {
        let known = self.client.view.playlists.iter().find(|p| p.id == playlist);
        match known {
            Some(p) => (self.log)(&format!(
                "live: searching {:?} (playlist {playlist})",
                p.name
            )),
            None => (self.log)(&format!(
                "live: searching playlist {playlist} (the server listed {:?})",
                self.client
                    .view
                    .playlists
                    .iter()
                    .map(|p| (p.id, p.name.as_str()))
                    .collect::<Vec<_>>()
            )),
        }
        self.client.send(ToServer::Search(playlist));
        self.wait(timeout, |_, e| match e {
            LiveEvent::LauncherMatch(m) => Some(m.clone()),
            _ => None,
        })
    }

    /// Poll until `pick` takes an event, logging every event, for up to
    /// `timeout`.
    fn wait<T>(
        &mut self,
        timeout: Duration,
        mut pick: impl FnMut(&mut LiveClient, &LiveEvent) -> Option<T>,
    ) -> Result<T, String> {
        let until = Instant::now() + timeout;
        loop {
            for e in self.client.poll(self.start.elapsed().as_secs_f64()) {
                log_event(&self.log, &e);
                if let LiveEvent::Lost(why) = &e {
                    return Err(format!("lost the server: {why}"));
                }
                if let Some(t) = pick(&mut self.client, &e) {
                    return Ok(t);
                }
            }
            if Instant::now() >= until {
                return Err(format!("nothing came in {} s", timeout.as_secs()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The address the server was reached at.
    pub fn server_ip(&self) -> Option<IpAddr> {
        self.client.server_ip()
    }

    /// Hand the sign-in to a thread for the match `game`, which this PC
    /// hosts if `host`.
    pub fn play(self, game: &LauncherMatch, host: bool) -> Link {
        let (tx, rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let id = game.id;
        let log = self.log.clone();
        let spawned = std::thread::Builder::new()
            .name("h2launch-live".into())
            .spawn(move || run(self, id, host, rx, done_tx));
        if let Err(e) = spawned {
            log(&format!(
                "live: no thread for the server ({e}); it will drop us"
            ));
        }
        Link {
            tx: Mutex::new(tx),
            done: Mutex::new(done_rx),
        }
    }
}

/// A match to play: its session, which machine this PC is, the map and
/// variant to start, and the thread that keeps the sign-in.
pub struct Ready {
    pub session: Session,
    pub me: usize,
    pub map: String,
    pub variant: String,
    pub link: Link,
}

/// Sign in to `server` as `gamertag` (with the key and stat card kept in
/// `folder`), search `playlist`, and wait up to `wait` for a match.
pub fn prepare(
    server: &str,
    playlist: u8,
    wait: Duration,
    gamertag: &str,
    maps_dir: &Path,
    folder: &Path,
    log: Log,
) -> Result<Ready, String> {
    let maps = mcc_maps(maps_dir);
    log(&format!(
        "live: {} multiplayer maps found in {}",
        maps.len(),
        maps_dir.display()
    ));
    let profile = Profile {
        gamertag: gamertag.to_string(),
        look: Default::default(),
        maps,
        guests: 0,
    };
    let url = server_url(server);
    let mut lobby = Lobby::sign_in(
        &url,
        &folder.join("live-key.bin"),
        &folder.join("live-card.txt"),
        &profile,
        log.clone(),
        Duration::from_secs(15),
    )?;
    let game = lobby.find_match(playlist, wait)?;
    let ip = lobby.server_ip().ok_or("the server's address is unknown")?;
    let (session, me) = session_from(&game, ip)?;
    let link = lobby.play(&game, me == session.host);
    Ok(Ready {
        session,
        me,
        map: game.map,
        variant: game.variant,
        link,
    })
}

/// What the launcher tells the thread about the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// The engine is running its online game (hosting it, on the host).
    Running,
    /// The engine loaded the map (a joiner joined the host to get there).
    MapLoaded,
    /// The engine ended the game and handed over its results.
    Ended,
    /// The launcher is closing.
    Closing,
}

/// The thread keeping the sign-in alive during a match.
pub struct Link {
    tx: Mutex<Sender<Engine>>,
    done: Mutex<Receiver<()>>,
}

impl Link {
    pub fn tell(&self, what: Engine) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(what);
        }
    }

    /// Say the launcher is closing, and give the thread up to `wait` to
    /// tell the server.
    pub fn close(&self, wait: Duration) {
        self.tell(Engine::Closing);
        if let Ok(done) = self.done.lock() {
            let _ = done.recv_timeout(wait);
        }
    }
}

static LINK: OnceLock<Link> = OnceLock::new();

/// Keep `link` for `tell` and `close` (once).
pub fn install(link: Link) {
    let _ = LINK.set(link);
}

/// Tell the match's thread about the engine, if there is a match.
pub fn tell(what: Engine) {
    if let Some(link) = LINK.get() {
        link.tell(what);
    }
}

/// The launcher is closing: let the server know, if there is a match.
pub fn close() {
    if let Some(link) = LINK.get() {
        link.close(VERDICT_WAIT + Duration::from_secs(1));
    }
}

/// What the thread has told the server, so it says each thing once.
#[derive(Debug, Default, PartialEq, Eq)]
struct Told {
    asked_to_host: bool,
    running: bool,
    hosting: bool,
    joined: bool,
    /// The result went (the game ended on this PC).
    reported: bool,
    /// LAUNCHER_RESULT or LEFT_MATCH went, or the server ended the match.
    over: bool,
    /// The server's MATCH_OVER came.
    verdict: bool,
}

impl Told {
    /// What to send after `event` (from the server) or `engine` (from the
    /// launcher), for match `id`, which this PC hosts if `host`.
    fn next(
        &mut self,
        id: u64,
        host: bool,
        event: Option<&LiveEvent>,
        engine: Option<Engine>,
    ) -> Vec<ToServer> {
        let mut out = Vec::new();
        match event {
            Some(LiveEvent::HostMatch(m)) if *m == id => self.asked_to_host = true,
            Some(LiveEvent::MatchOver(m)) if m.id == id => {
                self.over = true;
                self.verdict = true;
            }
            _ => {}
        }
        // A map loaded means the engine runs too.
        if matches!(engine, Some(Engine::Running | Engine::MapLoaded)) {
            self.running = true;
        }
        match engine {
            Some(Engine::MapLoaded) if !host && !self.joined => {
                self.joined = true;
                out.push(ToServer::Joined(id));
            }
            Some(Engine::Ended) if !self.over => {
                self.over = true;
                self.reported = true;
                // Results block not read yet: no scores, no players.
                out.push(ToServer::LauncherResult(LauncherResult {
                    id,
                    finished: true,
                    team_scores: Vec::new(),
                    players: Vec::new(),
                }));
            }
            Some(Engine::Closing) if !self.over => {
                self.over = true;
                out.push(ToServer::LeftMatch {
                    id,
                    host_lost: false,
                });
            }
            _ => {}
        }
        if host && self.asked_to_host && self.running && !self.hosting {
            self.hosting = true;
            out.push(ToServer::Hosting(id));
        }
        out
    }
}

fn run(mut lobby: Lobby, id: u64, host: bool, rx: Receiver<Engine>, done: Sender<()>) {
    let mut told = Told::default();
    let mut closing_at: Option<Instant> = None;
    loop {
        let mut out = Vec::new();
        let mut closing = false;
        for e in lobby.client.poll(lobby.start.elapsed().as_secs_f64()) {
            log_event(&lobby.log, &e);
            out.extend(told.next(id, host, Some(&e), None));
        }
        while let Ok(what) = rx.try_recv() {
            closing |= what == Engine::Closing;
            out.extend(told.next(id, host, None, Some(what)));
        }
        for m in out {
            (lobby.log)(&format!("live: telling the server {}", describe(&m)));
            lobby.client.send(m);
        }
        if closing {
            closing_at.get_or_insert_with(Instant::now);
        }
        // A PC that reported waits a little for the verdict; any other
        // closes at once.
        if let Some(at) = closing_at {
            if !told.reported || told.verdict || at.elapsed() >= VERDICT_WAIT {
                let _ = done.send(());
                return;
            }
        }
        std::thread::sleep(POLL);
    }
}

fn describe(m: &ToServer) -> String {
    match m {
        ToServer::Hosting(id) => format!("we host match {id:016x}"),
        ToServer::Joined(id) => format!("we joined match {id:016x}"),
        ToServer::LeftMatch { id, .. } => format!("we left match {id:016x}"),
        ToServer::LauncherResult(r) => format!(
            "match {:016x} ended (finished: {}, {} player results)",
            r.id,
            r.finished,
            r.players.len()
        ),
        other => format!("{other:?}"),
    }
}

fn log_event(log: &Log, e: &LiveEvent) {
    let line = match e {
        LiveEvent::LauncherMatch(m) => format!(
            "live: match {:016x} on {} ({}), {} players, host {:#018x}, us {:#018x}",
            m.id,
            m.map,
            m.variant,
            m.players.len(),
            m.host,
            m.relay.id
        ),
        LiveEvent::Notice(n) => format!("live: notice {n:?}"),
        LiveEvent::Refused(why) => format!("live: refused: {why}"),
        LiveEvent::Lost(why) => format!("live: lost the server: {why}"),
        LiveEvent::HostMatch(id) => format!("live: asked to host match {id:016x}"),
        LiveEvent::Go(id) => format!("live: match {id:016x} goes"),
        LiveEvent::MatchOver(m) => format!(
            "live: match {:016x} is over (counted: {}, {:?})",
            m.id, m.counted, m.reason
        ),
        LiveEvent::Welcomed => "live: welcomed".to_string(),
        other => format!("live: {other:?}"),
    };
    log(&line);
}

#[cfg(test)]
mod tests {
    use super::*;
    use h2net::live::{LauncherPlayer, RelaySeat};

    fn game(me: u64, host: u64, teams: bool) -> LauncherMatch {
        let player = |id: u64, team: u8, tag: &str| LauncherPlayer {
            relay_id: id,
            account: id,
            gamertag: tag.into(),
            team,
            level: 1,
            party: id,
        };
        LauncherMatch {
            id: 0x77,
            playlist: 11,
            ranked: true,
            teams,
            map: "lockout".into(),
            variant: "01_slayer".into(),
            relay: RelaySeat {
                port: 47050,
                room: 0x77,
                id: me,
                key: [0xAB; 16],
            },
            host,
            countdown: 20,
            players: vec![player(0x1111, 1, "Alpha"), player(0x2222, 0, "Bravo")],
        }
    }

    #[test]
    fn urls() {
        assert_eq!(server_url("192.168.8.102"), "ws://192.168.8.102:47050/live");
        assert_eq!(server_url("example.org:9000"), "ws://example.org:9000/live");
        assert_eq!(
            server_url("wss://example.org/live"),
            "wss://example.org/live"
        );
        assert_eq!(server_url("[::1]:5"), "ws://[::1]:5/live");
        assert_eq!(server_url("[::1]"), "ws://[::1]:47050/live");
    }

    #[test]
    fn a_match_becomes_a_session() {
        let ip: IpAddr = "192.168.8.102".parse().unwrap();
        let (s, me) = session_from(&game(0x2222, 0x1111, false), ip).unwrap();
        assert_eq!(me, 1);
        assert_eq!(s.host, 0);
        assert_eq!(s.machines, vec![0x1111, 0x2222]);
        assert_eq!(s.relay, "192.168.8.102:47050");
        assert_eq!(s.relay_addr().unwrap().port(), 47050);
        assert_eq!(s.room, 0x77);
        assert_eq!(s.key.as_deref(), Some("abababababababababababababababab"));
        assert_eq!(s.local_player(me).unwrap().name.as_deref(), Some("Bravo"));
        // Free-for-all: a team each.
        let teams: Vec<i32> = s.players.iter().map(|p| p.team).collect();
        assert_eq!(teams, vec![0, 1]);
        // Teams kept in team games.
        let (s, _) = session_from(&game(0x1111, 0x1111, true), ip).unwrap();
        let teams: Vec<i32> = s.players.iter().map(|p| p.team).collect();
        assert_eq!(teams, vec![1, 0]);
    }

    #[test]
    fn a_match_that_leaves_us_out_is_refused() {
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(session_from(&game(0x3333, 0x1111, false), ip).is_err());
        assert!(session_from(&game(0x1111, 0x3333, false), ip).is_err());
    }

    #[test]
    fn the_host_says_hosting_once_asked_and_running() {
        let mut t = Told::default();
        assert!(t.next(7, true, None, Some(Engine::Running)).is_empty());
        let out = t.next(7, true, Some(&LiveEvent::HostMatch(7)), None);
        assert_eq!(out, vec![ToServer::Hosting(7)]);
        // Once only, and a host never says it joined.
        assert!(t.next(7, true, None, Some(Engine::MapLoaded)).is_empty());
        assert!(t
            .next(7, true, Some(&LiveEvent::HostMatch(7)), None)
            .is_empty());
    }

    #[test]
    fn a_joiner_says_joined_when_the_map_loads_and_left_when_closing() {
        let mut t = Told::default();
        assert!(t
            .next(
                7,
                false,
                Some(&LiveEvent::HostMatch(7)),
                Some(Engine::Running)
            )
            .is_empty());
        assert_eq!(
            t.next(7, false, None, Some(Engine::MapLoaded)),
            vec![ToServer::Joined(7)]
        );
        assert!(t.next(7, false, None, Some(Engine::MapLoaded)).is_empty());
        assert_eq!(
            t.next(7, false, None, Some(Engine::Closing)),
            vec![ToServer::LeftMatch {
                id: 7,
                host_lost: false
            }]
        );
    }

    #[test]
    fn an_ended_game_reports_once_and_closing_after_it_is_no_quit() {
        let mut t = Told::default();
        let out = t.next(7, true, None, Some(Engine::Ended));
        assert!(matches!(
            out.as_slice(),
            [ToServer::LauncherResult(LauncherResult {
                id: 7,
                finished: true,
                ..
            })]
        ));
        assert!(t.reported && !t.verdict);
        assert!(t.next(7, true, None, Some(Engine::Ended)).is_empty());
        assert!(t.next(7, true, None, Some(Engine::Closing)).is_empty());
    }
}
