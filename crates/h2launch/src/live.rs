//! `--live <server>`: play a match h2live makes, instead of one a session
//! file describes. The launcher signs in to h2live as a launcher (with its
//! own key, kept in its folder), searches a launcher playlist, and when a
//! match forms (LAUNCHER_MATCH) writes the session from it: the relay beside
//! the server, the match's room and this PC's key for it, one machine per
//! player, and who hosts. The engine then starts on the match's map and
//! variant as it does with `--session`. It sends no password: the PC must
//! be one its account knows (the lobby signs a new PC in once with the
//! password), and the server must have the key pinned for it, if one is
//! (`crate::pins`).
//!
//! While the engine plays, a thread keeps the sign-in alive and tells the
//! server what the engine does: HOSTING once the host's engine is running
//! (after the server asked it to host), JOINED when a joining PC's engine
//! has loaded the map (it joined the host's game to get there), and
//! LAUNCHER_RESULT when the engine ends the game (it hands its results to
//! host slot 6 on every PC when the round ends; `crate::results` reads each
//! player's team, standing, score, kills, assists, deaths, betrayals and
//! suicides from them), and LEFT_MATCH if the launcher closes before that.

use crate::controls::{Controls, KEYS};
use crate::gamemenu::Leave;
use crate::pins::Pins;
use crate::session::{Player, Session};
use h2live::client::{Access, LiveClient, LiveEvent, Profile};
use h2net::live::{
    AccountOp, LauncherMatch, LauncherPlayerResult, LauncherResult, ToServer, LIVE_PROTOCOL,
    PASSWORD_NEEDED, SIGNED_OUT, UPDATE_YOUR_LAUNCHER,
};
use h2net::Connection;
use std::collections::VecDeque;
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
    /// Events that came in the same poll as the one `wait` stopped at
    /// (HOST_MATCH comes right after the match itself), for whoever reads
    /// next.
    pending: VecDeque<LiveEvent>,
}

impl Lobby {
    /// Dial `server` and sign in as the holder of the key kept in `folder`
    /// (`live-key.bin`, made the first time), with the stat card kept
    /// there, as `profile` says. The server must have the identity key
    /// pinned for it in `folder` (`crate::pins`), if one is; no password
    /// is sent, so the account must know this PC already (the lobby signs
    /// a new PC in with its password). Waits up to `timeout` for the
    /// server's welcome.
    pub fn sign_in(
        server: &str,
        folder: &Path,
        profile: &Profile,
        log: Log,
        timeout: Duration,
    ) -> Result<Lobby, String> {
        Lobby::sign_in_with(server, folder, profile, log, timeout, |url| {
            h2net::dial(url, timeout)
                .recv()
                .map_err(|_| "dialling stopped".to_string())?
        })
    }

    /// `sign_in`, over the connection `dial` makes to the URL it's given
    /// (a test's own, there).
    fn sign_in_with(
        server: &str,
        folder: &Path,
        profile: &Profile,
        log: Log,
        timeout: Duration,
        dial: impl FnOnce(&str) -> Result<Connection, String>,
    ) -> Result<Lobby, String> {
        let key_path = folder.join("live-key.bin");
        let key = h2live::client::identity(&key_path)
            .map_err(|e| format!("the key at {}: {e}", key_path.display()))?;
        let mut pins = Pins::load(folder).map_err(|e| {
            let path = crate::pins::path(folder);
            format!("the server keys at {}: {e}", path.display())
        })?;
        for note in pins.notes() {
            log(&note);
        }
        let url = server_url(server);
        let conn = dial(&url)?;
        let start = Instant::now();
        let card_path = folder.join("live-card.txt");
        let mut client = LiveClient::launcher(conn, key, profile, crate::BUILD, &card_path, 0.0);
        client.set_access(Access {
            pin: pins.get(server),
            password: None,
            computer: h2net::computer_name(),
        });
        let mut lobby = Lobby {
            client,
            start,
            log,
            pending: VecDeque::new(),
        };
        // (A refusal ends any wait: see `read_until`.)
        lobby.wait(timeout, |e| match e {
            LiveEvent::Welcomed => Some(Ok::<(), String>(())),
            LiveEvent::KeyChanged { .. } => Some(Err(
                "the server's key changed; open the lobby to check it".into(),
            )),
            _ => None,
        })??;
        if let Some(w) = &lobby.client.view.welcome {
            (lobby.log)(&format!(
                "live: signed in to {url} as {:?} (account {:#018x})",
                w.gamertag, w.account
            ));
        }
        // The server proved it holds this key: pin it, as the lobby does.
        if let Some(key) = lobby.client.view.server_key {
            if pins.get(server) != Some(key) {
                pins.set(server, key);
                match pins.save(folder) {
                    Ok(()) => (lobby.log)(&format!(
                        "live: pinned the server's key {}",
                        h2live::seal::short_code(&key)
                    )),
                    Err(e) => (lobby.log)(&format!(
                        "live: the server's key couldn't be pinned in {}: {e}",
                        crate::pins::path(folder).display()
                    )),
                }
            }
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
        self.wait(timeout, |e| match e {
            LiveEvent::LauncherMatch(m) => Some(m.clone()),
            _ => None,
        })
    }

    /// The events to read next: those left over, then the server's new
    /// ones.
    fn events(&mut self) -> VecDeque<LiveEvent> {
        let mut events = std::mem::take(&mut self.pending);
        events.extend(self.client.poll(self.start.elapsed().as_secs_f64()));
        events
    }

    /// Poll until `pick` takes an event, logging every event, for up to
    /// `timeout`.
    fn wait<T>(
        &mut self,
        timeout: Duration,
        mut pick: impl FnMut(&LiveEvent) -> Option<T>,
    ) -> Result<T, String> {
        let until = Instant::now() + timeout;
        loop {
            let mut events = self.events();
            let picked = read_until(&mut events, &self.log, &mut pick);
            self.pending = events;
            if let Some(picked) = picked {
                return picked;
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
    /// What kind of game the match is, for the GAME MENU (ranked, custom
    /// or social, by the lobby's rule).
    pub game_kind: crate::gamemenu::GameKind,
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
    let mut lobby = Lobby::sign_in(
        server,
        folder,
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
        game_kind: crate::lobby::child::game_kind(&game),
        map: game.map,
        variant: game.variant,
        link,
    })
}

/// What `--live` says when the server turns it away for `why`: a PC the
/// account doesn't know (or no longer knows) must sign in once from the
/// lobby with the password, since `--live` sends none. A server at another
/// live protocol says to update the launcher, though it may be the server
/// that's older: said as the lobby says it.
fn refusal(why: &str) -> String {
    if why == PASSWORD_NEEDED || why == SIGNED_OUT {
        "this PC isn't signed in to that account: sign in once from the lobby (same --instance) \
         with your password"
            .into()
    } else if why == UPDATE_YOUR_LAUNCHER {
        format!(
            "the server runs a different version from this launcher (live protocol \
             {LIVE_PROTOCOL}); ask its owner for the right one"
        )
    } else {
        format!("the server turned us away: {why}")
    }
}

/// Read `events` in order, logging each, until `pick` takes one or one
/// says the server is lost or turned us away (signed out by an admin, say:
/// nothing can come after it). What comes after it stays in `events`,
/// unread.
fn read_until<T>(
    events: &mut VecDeque<LiveEvent>,
    log: &Log,
    pick: &mut impl FnMut(&LiveEvent) -> Option<T>,
) -> Option<Result<T, String>> {
    while let Some(e) = events.pop_front() {
        log_event(log, &e);
        if let LiveEvent::Lost(why) = &e {
            return Some(Err(format!("lost the server: {why}")));
        }
        if let LiveEvent::Refused(why) = &e {
            return Some(Err(refusal(why)));
        }
        if let Some(t) = pick(&e) {
            return Some(Ok(t));
        }
    }
    None
}

/// What the launcher tells the thread (or, with `--events`, the lobby)
/// about the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Engine {
    /// The engine is running its online game (hosting it, on the host).
    Running,
    /// The engine loaded the map (a joiner joined the host to get there).
    MapLoaded,
    /// The engine ended the game and handed over its results: each
    /// player's, as the server takes them.
    Ended(Vec<LauncherPlayerResult>),
    /// The launcher is closing.
    Closing,
    /// The player left the game from the in-game GAME MENU (Leave Game),
    /// or its host ended it (End Game); the engine closes next.
    Left(Leave),
    /// The player changed their controls in the GAME MENU: every control
    /// as `(lobby.txt's name, value)`, in `KEYS` order, for the lobby to
    /// save. Text, not `Controls`, as `Engine` is `Eq` and the mouse
    /// sensitivity is an f32. `defaults`: they are Use Default Settings',
    /// so every one is saved, even those the engine already ran with (a
    /// control flag given for one run may have put them there); the
    /// values alone can't say so, as an ordinary change can land on them.
    Controls {
        pairs: Vec<(String, String)>,
        defaults: bool,
    },
    /// The GAME MENU turned itself off for the rest of the run (its
    /// overlay failed, or its panel wasn't drawn three times): Start and
    /// Esc reach the game again.
    MenuOff,
}

impl Engine {
    /// The GAME MENU's controls `c` as the event says them (`defaults`:
    /// Use Default Settings).
    pub fn controls(c: Controls, defaults: bool) -> Engine {
        Engine::Controls {
            pairs: c.pairs(),
            defaults,
        }
    }
}

/// How an engine event is written on the standard output with
/// `--events`, for the lobby that started this launcher: `H2EVENT running`,
/// `maploaded`, `closing`, `left game`, `left end`, `menu off`; `controls`,
/// `defaults` for Use Default Settings, and then the eight controls as
/// `key=value` in `KEYS` order (`lobby.txt`'s names and values); or
/// `ended` and then each player as
/// `<relay id>,<team>,<place>,<score>,<kills>,<assists>,<deaths>,<betrayals>,<suicides>,<left>`
/// (LAUNCHER_RESULT's order).
pub fn event_line(e: &Engine) -> String {
    match e {
        Engine::Running => format!("{EVENT} running"),
        Engine::MapLoaded => format!("{EVENT} maploaded"),
        Engine::Closing => format!("{EVENT} closing"),
        Engine::Left(Leave::Game) => format!("{EVENT} left game"),
        Engine::Left(Leave::End) => format!("{EVENT} left end"),
        Engine::MenuOff => format!("{EVENT} menu off"),
        Engine::Controls { pairs, defaults } => {
            let mut line = format!("{EVENT} controls");
            if *defaults {
                line += " defaults";
            }
            for (k, v) in pairs {
                line += &format!(" {k}={v}");
            }
            line
        }
        Engine::Ended(players) => {
            let mut line = format!("{EVENT} ended");
            for p in players {
                line += &format!(
                    " {:x},{},{},{},{},{},{},{},{},{}",
                    p.relay_id,
                    p.team,
                    p.place,
                    p.score,
                    p.kills,
                    p.assists,
                    p.deaths,
                    p.betrayals,
                    p.suicides,
                    u8::from(p.left)
                );
            }
            line
        }
    }
}

/// The start of every `--events` line.
pub const EVENT: &str = "H2EVENT";

/// Reads a line `event_line` wrote; `None` for any other line (the log
/// goes to the same output). The lobby and the engine's copy are the same
/// build, so only the 10-field form of `ended` is read, and `controls`
/// only with all eight controls in `KEYS` order, each with a value
/// `Controls::set` takes (after `defaults`, for Use Default Settings).
pub fn parse_event_line(line: &str) -> Option<Engine> {
    let mut words = line.split_whitespace().peekable();
    if words.next()? != EVENT {
        return None;
    }
    let e = match words.next()? {
        "running" => Engine::Running,
        "maploaded" => Engine::MapLoaded,
        "closing" => Engine::Closing,
        "left" => match words.next()? {
            "game" => Engine::Left(Leave::Game),
            "end" => Engine::Left(Leave::End),
            _ => return None,
        },
        "menu" => match words.next()? {
            "off" => Engine::MenuOff,
            _ => return None,
        },
        "controls" => {
            let defaults = words.next_if_eq(&"defaults").is_some();
            let mut check = Controls::default();
            let mut pairs = Vec::new();
            for key in KEYS {
                let (k, v) = words.next()?.split_once('=')?;
                if k != key || check.set(k, v) != Ok(true) {
                    return None;
                }
                pairs.push((k.to_string(), v.to_string()));
            }
            Engine::Controls { pairs, defaults }
        }
        "ended" => {
            let mut players = Vec::new();
            for w in words.by_ref() {
                let f: Vec<&str> = w.split(',').collect();
                let [id, team, place, score, kills, assists, deaths, betrayals, suicides, left] =
                    f[..]
                else {
                    return None;
                };
                players.push(LauncherPlayerResult {
                    relay_id: u64::from_str_radix(id, 16).ok()?,
                    team: team.parse().ok()?,
                    place: place.parse().ok()?,
                    score: score.parse().ok()?,
                    kills: kills.parse().ok()?,
                    assists: assists.parse().ok()?,
                    deaths: deaths.parse().ok()?,
                    betrayals: betrayals.parse().ok()?,
                    suicides: suicides.parse().ok()?,
                    left: left == "1",
                });
            }
            Engine::Ended(players)
        }
        _ => return None,
    };
    words.next().is_none().then_some(e)
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

/// Where the engine's events go.
enum Sink {
    /// To the server, through the match's thread (`--live`).
    Server(Link),
    /// To the lobby that started this launcher, on the standard output
    /// (`--events`).
    Lobby,
}

static SINK: OnceLock<Sink> = OnceLock::new();

/// Keep `link` for `tell` and `close` (once).
pub fn install(link: Link) {
    let _ = SINK.set(Sink::Server(link));
}

/// Write the engine's events on the standard output (once).
pub fn install_events() {
    let _ = SINK.set(Sink::Lobby);
}

/// The engine's events go to a lobby that started it (`--events`), which
/// saves the game menu's controls itself; otherwise the engine saves them.
pub fn to_lobby() -> bool {
    matches!(SINK.get(), Some(Sink::Lobby))
}

fn say(what: &Engine) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", event_line(what));
    let _ = out.flush();
}

/// Tell the match's thread, or the lobby, about the engine, if there is a
/// match.
pub fn tell(what: Engine) {
    match SINK.get() {
        Some(Sink::Server(link)) => link.tell(what),
        Some(Sink::Lobby) => say(&what),
        None => {}
    }
}

/// The launcher is closing: let the server (or the lobby) know, if there
/// is a match.
pub fn close() {
    match SINK.get() {
        Some(Sink::Server(link)) => link.close(VERDICT_WAIT + Duration::from_secs(1)),
        Some(Sink::Lobby) => say(&Engine::Closing),
        None => {}
    }
}

/// What has been told to the server about a match, so each thing is said
/// once: by the match's thread with `--live`, and by the lobby for the
/// engine it started.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Told {
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
    /// The player ended the game for everyone from the GAME MENU (End
    /// Game): the result that follows is a game cut short.
    ended_from_menu: bool,
    /// The server's GO for the match came: it started the game, and takes
    /// results for it from now on.
    went: bool,
}

impl Told {
    /// The result went: the game ended on this PC.
    pub fn reported(&self) -> bool {
        self.reported
    }

    /// The server's MATCH_OVER came.
    pub fn verdict(&self) -> bool {
        self.verdict
    }

    /// LAUNCHER_RESULT or LEFT_MATCH went, or the server ended the match.
    pub fn over(&self) -> bool {
        self.over
    }

    /// The player ended the game from the GAME MENU (End Game) before the
    /// server's GO came: the engine's game ran, but h2live hadn't started
    /// the match (a guest was still joining; it waits up to 20 s for them)
    /// and would drop the result. That result, given as the engine quits,
    /// isn't passed on then: this PC only leaves (LEFT_MATCH, which h2live
    /// takes as the host leaving), and the lobby shows "You ended the
    /// game." rather than a carnage report saying the host left.
    pub fn ended_before_go(&self) -> bool {
        self.ended_from_menu && !self.went
    }

    /// What to send after `event` (from the server) or `engine` (from the
    /// launcher), for match `id`, which this PC hosts if `host`.
    pub fn next(
        &mut self,
        id: u64,
        host: bool,
        event: Option<&LiveEvent>,
        engine: Option<Engine>,
    ) -> Vec<ToServer> {
        let mut out = Vec::new();
        match event {
            Some(LiveEvent::HostMatch(m)) if *m == id => self.asked_to_host = true,
            Some(LiveEvent::Go(m)) if *m == id => self.went = true,
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
            // End Game before GO: the server would drop the result, so
            // this PC only leaves, as when its engine closes.
            Some(Engine::Ended(_)) if !self.over && self.ended_before_go() => {
                self.over = true;
                out.push(ToServer::LeftMatch {
                    id,
                    host_lost: false,
                });
            }
            Some(Engine::Ended(players)) if !self.over => {
                self.over = true;
                self.reported = true;
                // The block's team scores aren't known yet; the server
                // places teams by their players' standings.
                out.push(ToServer::LauncherResult(LauncherResult {
                    id,
                    finished: !self.ended_from_menu,
                    team_scores: Vec::new(),
                    players,
                }));
                // End Game's result came as the engine quits: the game was
                // cut short, and this PC has left it. Said in that order,
                // h2live takes the host as leaving once the game was over
                // (not HOST_LEFT): the match ends with no reason, and
                // counts for no one, as an unranked custom game doesn't.
                if self.ended_from_menu {
                    out.push(ToServer::LeftMatch {
                        id,
                        host_lost: false,
                    });
                }
            }
            Some(Engine::Closing) if !self.over => {
                self.over = true;
                out.push(ToServer::LeftMatch {
                    id,
                    host_lost: false,
                });
            }
            // End Game: its result follows as the engine quits, and goes
            // as a game cut short.
            Some(Engine::Left(Leave::End)) => self.ended_from_menu = true,
            // Nothing new goes to the server: a player who leaves from the
            // game menu is told as LEFT_MATCH when the engine closes, as
            // before (or after End Game's result), the controls are the
            // lobby's to save, and the menu turning off is the lobby's to
            // show.
            Some(Engine::Left(Leave::Game) | Engine::Controls { .. } | Engine::MenuOff) => {}
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
        for e in lobby.events() {
            log_event(&lobby.log, &e);
            out.extend(told.next(id, host, Some(&e), None));
        }
        // The game menu's Left, Controls and MenuOff change nothing here:
        // a player who leaves is told as LEFT_MATCH when the launcher
        // closes, and the controls were saved by the engine's copy.
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

pub fn describe(m: &ToServer) -> String {
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
        ToServer::ProvePassword { computer, .. } => {
            format!("PROVE_PASSWORD from {computer:?} (password sealed)")
        }
        ToServer::Account { seq, op, .. } => {
            let what = match op {
                AccountOp::List => "LIST".to_string(),
                AccountOp::Create { gamertag, .. } => format!("CREATE {gamertag} (sealed)"),
                AccountOp::SetPassword { account, .. } => {
                    format!("SET_PASSWORD {account:016x} (sealed)")
                }
                AccountOp::SignOut { account } => format!("SIGN_OUT {account:016x}"),
                AccountOp::Remove { account } => format!("REMOVE {account:016x}"),
                AccountOp::ChangePassword { .. } => "CHANGE_PASSWORD (sealed)".to_string(),
            };
            format!("account request {seq}: {what}")
        }
        other => format!("{other:?}"),
    }
}

pub fn log_event(log: &Log, e: &LiveEvent) {
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
        LiveEvent::KeyChanged { was, now } => format!(
            "live: the server's key is {}, not {} as pinned",
            h2live::seal::short_code(now),
            h2live::seal::short_code(was)
        ),
        LiveEvent::Accounts { answers, ok, .. } => {
            format!("live: accounts (answering {answers}, ok {ok})")
        }
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
    fn a_pc_the_account_doesnt_know_is_sent_to_the_lobby() {
        for why in [PASSWORD_NEEDED, SIGNED_OUT] {
            let said = refusal(why);
            assert!(
                said.starts_with("this PC isn't signed in to that account"),
                "{said}"
            );
            assert!(said.contains("from the lobby (same --instance)"), "{said}");
        }
        assert_eq!(
            refusal(h2net::live::SERVER_FULL),
            "the server turned us away: SERVER FULL"
        );
    }

    #[test]
    fn a_server_at_another_version_is_said_to_be_one() {
        // An older server tells a newer launcher to update: not said so.
        let said = refusal(UPDATE_YOUR_LAUNCHER);
        assert_eq!(
            said,
            format!(
                "the server runs a different version from this launcher (live protocol \
                 {LIVE_PROTOCOL}); ask its owner for the right one"
            )
        );
        assert!(!said.contains("UPDATE"));
    }

    #[test]
    fn server_keys_that_cannot_be_read_stop_live_before_it_dials() {
        let dir = std::env::temp_dir().join(format!("h2launch-live-pins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Saved as UTF-16, say.
        std::fs::write(crate::pins::path(&dir), [0xFF, 0xFE, b'h', 0, b'2', 0]).unwrap();
        let profile = Profile {
            gamertag: "Alpha".into(),
            look: Default::default(),
            maps: Vec::new(),
            guests: 0,
        };
        let log: Log = Arc::new(|_: &str| {});
        // (Port 9 is never dialled: it gives up first.)
        let wait = Duration::from_millis(200);
        let Err(e) = Lobby::sign_in("127.0.0.1:9", &dir, &profile, log, wait) else {
            panic!("signed in");
        };
        assert!(e.starts_with("the server keys at "), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A test folder of its own, emptied.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("h2launch-live-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn alpha() -> Profile {
        Profile {
            gamertag: "Alpha".into(),
            look: Default::default(),
            maps: Vec::new(),
            guests: 0,
        }
    }

    /// What a server with the identity key made from `n` says to a launcher
    /// that signs in by its key: the key to seal to (signed), the
    /// challenge, and WELCOME. A launcher takes them in that order, so they
    /// can all be sent before it dials. The identity key.
    fn sign_in_by_hand(server_end: &mut Connection, n: u8) -> [u8; 32] {
        use ed25519_dalek::{Signer, SigningKey};
        use h2net::live::{ToPc, Welcome};
        let identity = SigningKey::from_bytes(&[n; 32]);
        let nonce = [9; 32];
        let key = h2live::seal::public(&[7; 32]);
        let sig = identity.sign(&h2live::seal::key_proof(&nonce, &key));
        let server = identity.verifying_key().to_bytes();
        let sig = sig.to_bytes();
        ToPc::SealKey { key, server, sig }.send(server_end);
        let motd = String::new();
        ToPc::Challenge { nonce, motd }.send(server_end);
        let welcome = Welcome {
            account: 1,
            gamertag: "ALPHA".into(),
            card: String::new(),
            best: 1,
            levels: Vec::new(),
        };
        ToPc::Welcome(welcome).send(server_end);
        server_end.flush().unwrap();
        server
    }

    /// The kinds of message the launcher sent the server (pings left out).
    fn sent(server_end: &mut Connection) -> Vec<u8> {
        use h2net::live::kind;
        let got = server_end.receive().unwrap_or_default();
        got.into_iter()
            .map(|(k, _)| k)
            .filter(|&k| k != kind::PING && k != kind::PONG)
            .collect()
    }

    /// `--live`'s sign-in to `server` over a connection of the test's own.
    fn live_sign_in(
        dir: &Path,
        server: &str,
        log: Log,
        pc_end: Connection,
    ) -> Result<Lobby, String> {
        let wait = Duration::from_secs(5);
        Lobby::sign_in_with(server, dir, &alpha(), log, wait, |_| Ok(pc_end))
    }

    #[test]
    fn live_checks_the_servers_key_and_pins_it_the_first_time() {
        use h2net::live::kind;
        // Written as host and port: the pin is found however it's written.
        const SERVER: &str = "127.0.0.1:9";
        let said = Arc::new(Mutex::new(Vec::<String>::new()));
        let kept = said.clone();
        let log: Log = Arc::new(move |line: &str| kept.lock().unwrap().push(line.into()));
        // Another key pinned: given up on before anything but the LOGIN is
        // sent, and the pin is kept as it was.
        let dir = scratch("pin-changed");
        let mut pins = Pins::default();
        pins.set(SERVER, [0x12; 32]);
        pins.save(&dir).unwrap();
        let before = std::fs::read(crate::pins::path(&dir)).unwrap();
        let (mut server_end, pc_end) = Connection::pair();
        sign_in_by_hand(&mut server_end, 3);
        let Err(e) = live_sign_in(&dir, SERVER, log.clone(), pc_end) else {
            panic!("signed in to a server with another key");
        };
        assert!(e.starts_with("the server's key changed"), "{e}");
        assert_eq!(sent(&mut server_end), [kind::LOGIN]);
        assert_eq!(std::fs::read(crate::pins::path(&dir)).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
        // No key yet: the first sign-in pins the server's.
        let dir = scratch("pin-first");
        let (mut server_end, pc_end) = Connection::pair();
        let key = sign_in_by_hand(&mut server_end, 3);
        let lobby = live_sign_in(&dir, SERVER, log.clone(), pc_end).unwrap();
        assert_eq!(lobby.client.view.server_key, Some(key));
        assert_eq!(sent(&mut server_end), [kind::LOGIN, kind::PROVE]);
        let pins = Pins::load(&dir).unwrap();
        assert_eq!(pins.get(SERVER), Some(key));
        assert_eq!(pins.get("ws://127.0.0.1:9/live"), Some(key));
        assert!(said
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with("live: pinned the server's key ")));
        drop(lobby);
        // The same key again: signed in, and the file is as it was.
        let before = std::fs::read(crate::pins::path(&dir)).unwrap();
        let (mut server_end, pc_end) = Connection::pair();
        sign_in_by_hand(&mut server_end, 3);
        live_sign_in(&dir, SERVER, log, pc_end).unwrap();
        assert_eq!(std::fs::read(crate::pins::path(&dir)).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_stops_searching_when_the_server_turns_it_away() {
        use h2net::live::ToPc;
        let dir = scratch("refused-searching");
        let log: Log = Arc::new(|_: &str| {});
        let (mut server_end, pc_end) = Connection::pair();
        sign_in_by_hand(&mut server_end, 3);
        let mut lobby = live_sign_in(&dir, "127.0.0.1:9", log, pc_end).unwrap();
        // An admin signs the account out of all PCs while it searches: it
        // says so at once, not when the wait is up.
        ToPc::Refused(SIGNED_OUT.into()).send(&mut server_end);
        server_end.flush().unwrap();
        let start = Instant::now();
        let Err(e) = lobby.find_match(11, Duration::from_secs(60)) else {
            panic!("a match");
        };
        assert!(start.elapsed() < Duration::from_secs(10));
        assert_eq!(e, refusal(SIGNED_OUT));
        assert!(e.contains("sign in once from the lobby"), "{e}");
        // So for any wait, and whatever the refusal.
        let log: Log = Arc::new(|_: &str| {});
        let mut pick = |e: &LiveEvent| match e {
            LiveEvent::LauncherMatch(m) => Some(m.id),
            _ => None,
        };
        let why = h2net::live::ACCOUNT_REMOVED;
        let mut events: VecDeque<LiveEvent> = [LiveEvent::Refused(why.into())].into();
        let picked = read_until(&mut events, &log, &mut pick);
        assert_eq!(picked, Some(Err(refusal(why))));
        let _ = std::fs::remove_dir_all(&dir);
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
    fn what_came_with_the_match_is_kept_for_the_match_thread() {
        // The server sends HOST_MATCH right after the match: one poll can
        // bring both, and the host must still hear it was asked to host.
        let log: Log = Arc::new(|_: &str| {});
        let mut events: VecDeque<LiveEvent> = [
            LiveEvent::Welcomed,
            LiveEvent::LauncherMatch(game(0x1111, 0x1111, false)),
            LiveEvent::HostMatch(0x77),
        ]
        .into();
        let mut pick = |e: &LiveEvent| match e {
            LiveEvent::LauncherMatch(m) => Some(m.id),
            _ => None,
        };
        let picked = read_until(&mut events, &log, &mut pick);
        assert_eq!(picked, Some(Ok(0x77)));
        assert_eq!(events, [LiveEvent::HostMatch(0x77)]);
        let mut t = Told::default();
        t.next(0x77, true, None, Some(Engine::Running));
        let out = t.next(0x77, true, events.front(), None);
        assert_eq!(out, vec![ToServer::Hosting(0x77)]);
        // Nothing left: nothing picked.
        assert_eq!(read_until(&mut VecDeque::new(), &log, &mut pick), None);
    }

    #[test]
    fn engine_events_go_to_the_lobby_as_lines() {
        let player = |id: u64, score: i32, left: bool| LauncherPlayerResult {
            relay_id: id,
            team: 1,
            place: 2,
            score,
            kills: 3,
            assists: 5,
            deaths: 4,
            betrayals: 6,
            suicides: 7,
            left,
        };
        let ended = Engine::Ended(vec![player(0xB5A9, -2, false), player(0x5977, 7, true)]);
        let changed = Controls {
            buttons: crate::controls::ButtonLayout::BumperJumper,
            sticks: crate::controls::StickLayout::LegacySouthpaw,
            look_sensitivity: 10,
            look_inverted: true,
            auto_center: true,
            vibration: false,
            mouse_sensitivity: 0.1,
            mouse_inverted: true,
        };
        for e in [
            Engine::Running,
            Engine::MapLoaded,
            Engine::Closing,
            ended,
            Engine::Left(Leave::Game),
            Engine::Left(Leave::End),
            Engine::MenuOff,
            Engine::controls(Controls::default(), false),
            Engine::controls(changed, false),
            Engine::controls(Controls::default(), true),
        ] {
            let line = event_line(&e);
            assert!(line.starts_with("H2EVENT "), "{line}");
            assert_eq!(parse_event_line(&line), Some(e), "{line}");
        }
        assert_eq!(event_line(&Engine::Left(Leave::Game)), "H2EVENT left game");
        assert_eq!(event_line(&Engine::Left(Leave::End)), "H2EVENT left end");
        assert_eq!(event_line(&Engine::MenuOff), "H2EVENT menu off");
        assert_eq!(
            event_line(&Engine::controls(changed, false)),
            "H2EVENT controls button_layout=bumper_jumper thumbstick_layout=legacy_southpaw \
             look_sensitivity=10 look_inversion=on auto_look_centering=on vibration=off \
             mouse_sensitivity=0.1 mouse_inversion=on"
        );
        // Use Default Settings says so before the eight.
        assert!(event_line(&Engine::controls(Controls::default(), true))
            .starts_with("H2EVENT controls defaults button_layout=default "));
        // The pairs read back as the same controls.
        let Some(Engine::Controls {
            pairs,
            defaults: false,
        }) = parse_event_line(&event_line(&Engine::controls(changed, false)))
        else {
            panic!("no controls");
        };
        let mut back = Controls::default();
        for (k, v) in &pairs {
            assert_eq!(back.set(k, v), Ok(true), "{k}={v}");
        }
        assert_eq!(back, changed);
        assert_eq!(
            event_line(&Engine::Ended(vec![player(0xB5A9, -2, false)])),
            "H2EVENT ended b5a9,1,2,-2,3,5,4,6,7,0"
        );
        assert_eq!(
            parse_event_line("H2EVENT ended"),
            Some(Engine::Ended(Vec::new()))
        );
        for other in [
            "",
            "12:00:01 launch: starting",
            "H2EVENT",
            "H2EVENT jumped",
            "H2EVENT running now",
            "H2EVENT ended b5a9,1,2",
            // The 7-field form of LIVE_PROTOCOL 2's launchers.
            "H2EVENT ended b5a9,1,2,-2,3,4,0",
            "H2EVENT ended b5a9,1,2,-2,3,5,4,6,7,0,1",
            "H2EVENT ended zz,1,2,3,4,5,6,7,8,0",
            "H2EVENT ended b5a9,1,2,-2,3,-5,4,6,7,0",
            // The game menu's: one word after left, of two.
            "H2EVENT left",
            "H2EVENT left now",
            "H2EVENT left game now",
            "H2EVENT left Game",
            // The menu turning off: exactly two words.
            "H2EVENT menu",
            "H2EVENT menu on",
            "H2EVENT menu off now",
        ] {
            assert_eq!(parse_event_line(other), None, "{other:?}");
        }
        // Controls: exactly the eight, in KEYS order, each a value
        // Controls::set takes.
        let good: Vec<String> = Controls::default()
            .pairs()
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let line = |words: &[String]| format!("H2EVENT controls {}", words.join(" "));
        assert!(parse_event_line(&line(&good)).is_some());
        let with = |word: &str, at: usize| {
            let mut w = good.clone();
            w.insert(at, word.into());
            line(&w)
        };
        assert_eq!(
            parse_event_line(&with("defaults", 0)),
            Some(Engine::controls(Controls::default(), true))
        );
        // `defaults` only once, only first, and only that word.
        for l in [
            with("defaults", 8),
            with("defaults", 3),
            format!("H2EVENT controls defaults defaults {}", good.join(" ")),
            with("default", 0),
            with("Defaults", 0),
            "H2EVENT controls defaults".into(),
        ] {
            assert_eq!(parse_event_line(&l), None, "{l:?}");
        }
        let mut bad: Vec<Vec<String>> = Vec::new();
        // Too few, too many, none.
        bad.push(good[..7].to_vec());
        bad.push([good.clone(), vec!["vibration=on".into()]].concat());
        bad.push(Vec::new());
        // Out of order.
        let mut swapped = good.clone();
        swapped.swap(0, 1);
        bad.push(swapped);
        // An unknown key, a short name set takes, a word with no '='.
        for (i, w) in [
            (5, "rumble=on"),
            (0, "layout=default"),
            (3, "look_inversion"),
            (3, "look_inversion="),
        ] {
            let mut b = good.clone();
            b[i] = w.into();
            bad.push(b);
        }
        // Values set refuses.
        for (i, w) in [
            (0, "button_layout=claw"),
            (1, "thumbstick_layout=inverted"),
            (2, "look_sensitivity=11"),
            (2, "look_sensitivity=0"),
            (3, "look_inversion=maybe"),
            (6, "mouse_sensitivity=0"),
            (6, "mouse_sensitivity=fast"),
            (6, "mouse_sensitivity=NaN"),
        ] {
            let mut b = good.clone();
            b[i] = w.into();
            bad.push(b);
        }
        for b in bad {
            let l = line(&b);
            assert_eq!(parse_event_line(&l), None, "{l:?}");
        }
    }

    #[test]
    fn the_game_menus_events_tell_the_server_nothing_new() {
        for host in [false, true] {
            let mut t = Told::default();
            t.next(7, host, Some(&LiveEvent::HostMatch(7)), None);
            t.next(7, host, None, Some(Engine::MapLoaded));
            for e in [
                Engine::Left(Leave::Game),
                Engine::Left(Leave::End),
                Engine::MenuOff,
                Engine::controls(Controls::default(), false),
                Engine::controls(Controls::default(), true),
            ] {
                assert!(t.next(7, host, None, Some(e.clone())).is_empty(), "{e:?}");
            }
            assert!(!t.over());
            // The engine closing after a leave is the LEFT_MATCH.
            assert_eq!(
                t.next(7, host, None, Some(Engine::Closing)),
                vec![ToServer::LeftMatch {
                    id: 7,
                    host_lost: false
                }]
            );
        }
    }

    #[test]
    fn an_ended_game_reports_once_and_closing_after_it_is_no_quit() {
        let mut t = Told::default();
        let player = LauncherPlayerResult {
            relay_id: 0x1111,
            team: 0,
            place: 0,
            score: 3,
            kills: 0,
            assists: 0,
            deaths: 1,
            betrayals: 0,
            suicides: 0,
            left: false,
        };
        let out = t.next(7, true, None, Some(Engine::Ended(vec![player])));
        let [ToServer::LauncherResult(r)] = out.as_slice() else {
            panic!("{out:?}");
        };
        assert_eq!((r.id, r.finished), (7, true));
        assert_eq!(r.players, [player]);
        assert!(t.reported && !t.verdict);
        assert!(t
            .next(7, true, None, Some(Engine::Ended(Vec::new())))
            .is_empty());
        assert!(t.next(7, true, None, Some(Engine::Closing)).is_empty());
    }

    /// End Game once the server's GO came: the host's result as its
    /// engine quits goes as a game cut short, then LEFT_MATCH at once, in
    /// that order (h2live then takes the host as leaving at the end, not
    /// HOST_LEFT); the engine's closing says nothing more. Leave Game's
    /// never goes this way: its quit-time result isn't passed on
    /// (`gamemenu::result_is_the_games`).
    #[test]
    fn end_games_result_goes_cut_short_before_left_match() {
        let player = |relay_id: u64, score: i32| LauncherPlayerResult {
            relay_id,
            team: 0,
            place: 0,
            score,
            kills: score.max(0) as u16,
            assists: 0,
            deaths: 0,
            betrayals: 0,
            suicides: 0,
            left: false,
        };
        let mut t = Told::default();
        t.next(7, true, Some(&LiveEvent::HostMatch(7)), None);
        assert_eq!(
            t.next(7, true, None, Some(Engine::MapLoaded)),
            vec![ToServer::Hosting(7)]
        );
        assert!(t.next(7, true, Some(&LiveEvent::Go(7)), None).is_empty());
        assert!(t
            .next(7, true, None, Some(Engine::Left(Leave::End)))
            .is_empty());
        assert!(!t.over() && !t.ended_before_go());
        let both = vec![player(0x1111, 4), player(0x2222, 2)];
        let out = t.next(7, true, None, Some(Engine::Ended(both.clone())));
        let [ToServer::LauncherResult(r), ToServer::LeftMatch { id, host_lost }] = out.as_slice()
        else {
            panic!("{out:?}");
        };
        assert_eq!((r.id, r.finished), (7, false));
        assert_eq!(r.players, both);
        assert_eq!((*id, *host_lost), (7, false));
        assert!(t.over() && t.reported());
        assert!(t.next(7, true, None, Some(Engine::Closing)).is_empty());
        // A game that ended by itself, with no End Game, still finished.
        let mut t = Told::default();
        let out = t.next(7, true, None, Some(Engine::Ended(both)));
        let [ToServer::LauncherResult(r)] = out.as_slice() else {
            panic!("{out:?}");
        };
        assert!(r.finished);
    }

    /// End Game before the server's GO (a guest still joining): h2live
    /// hasn't started the match and would drop the result, so End Game's
    /// result goes nowhere and this PC only leaves, as when its engine
    /// closes; the closing after it says nothing more. Another match's GO
    /// doesn't count, and a GO that came between End Game and its result
    /// lets the result go as a game cut short.
    #[test]
    fn end_game_before_go_only_leaves() {
        let player = LauncherPlayerResult {
            relay_id: 0x1111,
            team: 0,
            place: 0,
            score: 1,
            kills: 1,
            assists: 0,
            deaths: 0,
            betrayals: 0,
            suicides: 0,
            left: false,
        };
        let hosting = |t: &mut Told| {
            t.next(7, true, Some(&LiveEvent::HostMatch(7)), None);
            assert_eq!(
                t.next(7, true, None, Some(Engine::MapLoaded)),
                vec![ToServer::Hosting(7)]
            );
        };
        let mut t = Told::default();
        hosting(&mut t);
        assert!(t.next(7, true, Some(&LiveEvent::Go(8)), None).is_empty());
        assert!(!t.ended_before_go());
        assert!(t
            .next(7, true, None, Some(Engine::Left(Leave::End)))
            .is_empty());
        assert!(t.ended_before_go());
        assert_eq!(
            t.next(7, true, None, Some(Engine::Ended(vec![player]))),
            vec![ToServer::LeftMatch {
                id: 7,
                host_lost: false
            }]
        );
        assert!(t.over() && !t.reported());
        assert!(t.next(7, true, None, Some(Engine::Closing)).is_empty());
        // GO after End Game, before its result: the match started, and
        // the result goes as a game cut short.
        let mut t = Told::default();
        hosting(&mut t);
        t.next(7, true, None, Some(Engine::Left(Leave::End)));
        assert!(t.next(7, true, Some(&LiveEvent::Go(7)), None).is_empty());
        assert!(!t.ended_before_go());
        let out = t.next(7, true, None, Some(Engine::Ended(vec![player])));
        let [ToServer::LauncherResult(r), ToServer::LeftMatch { .. }] = out.as_slice() else {
            panic!("{out:?}");
        };
        assert!(!r.finished);
        assert!(t.reported());
        // A game that ended by itself before GO is still reported as
        // before (End Game alone is held back).
        let mut t = Told::default();
        hosting(&mut t);
        let out = t.next(7, true, None, Some(Engine::Ended(vec![player])));
        assert!(matches!(out.as_slice(), [ToServer::LauncherResult(r)] if r.finished));
    }
}
