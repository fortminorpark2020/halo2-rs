//! The lobby's screens and what happens on them: signing in, the party and
//! the playlists, searching, custom games, the pregame lobby, the match
//! (its engine runs in a second copy of the launcher, see `child`) and the
//! carnage report.
//! Nothing here opens a window: `window`, or the headless loop in
//! `mod.rs`, gives it input and time and shows what it draws.

use super::canvas::{Align, Canvas, Color, Style, Text};
use super::child::{Kind, Match, Running, Said};
use super::names;
use super::settings::{self, Settings, GAMERTAG_LEN};
use crate::live::{self, Engine, Log, Told};
use crate::session::Session;
use h2live::client::{LiveClient, LiveEvent, Profile, View};
use h2net::live::{
    Activity, LauncherMatch, LauncherPlayerResult, MatchOver, OnlinePlayer, PartyInfo, PartyMember,
    Privacy, Stage, ToServer, CUSTOM_GAME, GAMERTAG_TAKEN,
};
use h2net::Connection;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// How long the pregame lobby shows the match before the engine starts.
const PREGAME: Duration = Duration::from_secs(5);
/// How long reaching the server and signing in may take.
const SIGN_IN_WAIT: Duration = Duration::from_secs(15);
/// How long a notice stays up.
const TOAST: Duration = Duration::from_secs(6);
/// How long the engine may stay up after its game ended (its own
/// postgame) before it is asked to close. An estimate.
const POSTGAME: Duration = Duration::from_secs(20);
/// How long an engine asked to close has before it is stopped. (The
/// launcher gives the engine 5 s to close itself.)
const QUIT_WAIT: Duration = Duration::from_secs(10);
/// The longest server address the sign-in screen takes.
const SERVER_LEN: usize = 100;
/// How long the custom game screen says the game is starting, unless the
/// match (or a notice) comes first.
const CUSTOM_WAIT: Duration = Duration::from_secs(10);
/// The custom game screen's rows: the game, the map, and the start.
const CUSTOM_ROWS: usize = 3;

/// What the lobby starts with.
pub struct Config {
    /// The launcher's folder: `lobby.txt`, the sign-in key and stat card,
    /// and the match's session file.
    pub folder: PathBuf,
    /// This launcher, started again for each match's engine.
    pub exe: PathBuf,
    pub kind: Kind,
    /// The multiplayer maps on this PC, with their hashes.
    pub maps: Vec<(String, u64)>,
    /// `--live`: the server for this run, over the one in `lobby.txt`.
    pub server: Option<String>,
    /// `--instance`, passed on to the engine.
    pub instance: Option<String>,
    pub log: Log,
}

/// A key, button or typed character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Up,
    Down,
    Left,
    Right,
    /// Enter, or the controller's A.
    A,
    /// Escape, or B.
    B,
    X,
    Y,
    Tab,
    Backspace,
    Char(char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    SignIn,
    Connecting,
    /// The party and the playlists, or the search while the party searches.
    Live,
    Players,
    /// The party's members: the leader makes another leader, removes
    /// them or sets who can join; anyone leaves.
    Party,
    /// The party leader picks a custom game's game type and map.
    Custom,
    Pregame,
    InGame,
    Carnage,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Popup {
    Quit,
    LeaveGame,
    /// The leader asked to remove this member from the party.
    Kick(u64),
}

/// What a click on part of the screen does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    /// Selects a row of the screen's list, or picks it if it was selected.
    Row(usize),
    /// Puts the cursor in a field of the sign-in screen.
    Field(usize),
    /// Selects a row of the custom game screen and moves its choice on
    /// (`true`) or back.
    Step(usize, bool),
    Press(Input),
}

/// Part of the screen that takes clicks, in layout units.
#[derive(Clone, Copy, Debug)]
struct Area {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    hit: Hit,
    /// On a popup: while one is up, nothing under it takes clicks.
    top: bool,
}

/// The match being played, from LAUNCHER_MATCH to the carnage report.
struct Game {
    m: LauncherMatch,
    session: Session,
    me: usize,
    told: Told,
    /// The engine should be started (at `start_at`).
    launch: bool,
    start_at: Instant,
    child: Option<Running>,
    running: bool,
    loaded: bool,
    /// The engine's results, and when they came.
    results: Option<Vec<LauncherPlayerResult>>,
    ended_at: Option<Instant>,
    over: Option<MatchOver>,
}

impl Game {
    fn host(&self) -> bool {
        self.me == self.session.host
    }
}

pub struct App {
    cfg: Config,
    settings: Settings,
    screen: Screen,
    /// The sign-in screen's gamertag and server, which has the cursor, and
    /// why the last try failed.
    fields: [String; 2],
    field: usize,
    form_error: Option<String>,
    dialing: Option<(Receiver<Result<Connection, String>>, Instant)>,
    client: Option<LiveClient>,
    signing_in: Option<Instant>,
    /// The client's clock starts here.
    clock: Instant,
    now: Instant,
    /// The selected playlist (or the custom game row after them) and
    /// player.
    sel: usize,
    psel: usize,
    /// The selected party member.
    msel: usize,
    /// The custom game screen's selected row, the game picked (in
    /// `names::CUSTOM_GAMES`) and the map (in `Config::maps`), and when
    /// the server was asked for it.
    crow: usize,
    cgame: usize,
    cmap: usize,
    asked: Option<Instant>,
    game: Option<Game>,
    popup: Option<Popup>,
    toasts: VecDeque<(String, Instant)>,
    failed: String,
    areas: Vec<Area>,
    /// Layout units to window pixels: scale and offset.
    view: (f32, f32, f32),
    quit: bool,
    focus: bool,
}

impl App {
    /// The lobby, signing in at once if `lobby.txt` has a gamertag.
    pub fn new(cfg: Config) -> App {
        let mut settings = Settings::load(&cfg.folder.join("lobby.txt"));
        if let Some(s) = &cfg.server {
            settings.server = s.clone();
        }
        let now = Instant::now();
        let mut app = App {
            fields: [settings.gamertag.clone(), settings.server.clone()],
            settings,
            cfg,
            screen: Screen::SignIn,
            field: 0,
            form_error: None,
            dialing: None,
            client: None,
            signing_in: None,
            clock: now,
            now,
            sel: 0,
            psel: 0,
            msel: 0,
            crow: 0,
            cgame: 0,
            cmap: 0,
            asked: None,
            game: None,
            popup: None,
            toasts: VecDeque::new(),
            failed: String::new(),
            areas: Vec::new(),
            view: (1.0, 0.0, 0.0),
            quit: false,
            focus: false,
        };
        if !app.settings.gamertag.is_empty() {
            app.connect();
        }
        app
    }

    fn log(&self, line: &str) {
        (self.cfg.log)(line);
    }

    /// The window should close.
    pub fn quitting(&self) -> bool {
        self.quit
    }

    /// The window should come to the front (once): the carnage report is up.
    pub fn take_focus(&mut self) -> bool {
        std::mem::take(&mut self.focus)
    }

    /// What is on screen, for the log and the test screenshots' names.
    pub fn label(&self) -> String {
        if let Some(p) = self.popup {
            return match p {
                Popup::Quit => "quit",
                Popup::LeaveGame => "leave",
                Popup::Kick(_) => "remove",
            }
            .into();
        }
        if self.invite().is_some() {
            return "invite".into();
        }
        let waiting = self.game.as_ref().is_some_and(|g| g.over.is_none());
        match self.screen {
            Screen::SignIn => "signin",
            Screen::Connecting => "connecting",
            Screen::Live if self.searching() => "searching",
            Screen::Live => "live",
            Screen::Players => "players",
            Screen::Party => "party",
            Screen::Custom => "custom",
            Screen::Pregame => "pregame",
            Screen::InGame => "ingame",
            Screen::Carnage if waiting => "carnage-waiting",
            Screen::Carnage => "carnage",
            Screen::Failed => "failed",
        }
        .into()
    }

    // ------------------------------------------------------------ server

    /// Sign in with what the sign-in screen says.
    fn connect(&mut self) {
        let gamertag = settings::clean_gamertag(&self.fields[0]);
        let server = self.fields[1].trim().to_string();
        self.fields[0] = gamertag.clone();
        self.screen = Screen::SignIn;
        if gamertag.is_empty() {
            self.field = 0;
            self.form_error = Some("Type a gamertag.".into());
            return;
        }
        if server.is_empty() {
            self.field = 1;
            self.form_error = Some("Type the server's address.".into());
            return;
        }
        self.form_error = None;
        self.settings = Settings { server, gamertag };
        let url = live::server_url(&self.settings.server);
        self.log(&format!(
            "lobby: signing in to {url} as {:?}",
            self.settings.gamertag
        ));
        self.client = None;
        self.dialing = Some((h2net::dial(&url, SIGN_IN_WAIT), Instant::now()));
        self.signing_in = Some(Instant::now());
        self.screen = Screen::Connecting;
    }

    fn poll_dial(&mut self) {
        let Some((rx, _)) = &self.dialing else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(conn)) => {
                self.dialing = None;
                self.start_client(conn);
            }
            Ok(Err(e)) => {
                self.dialing = None;
                self.fail(format!("Couldn't reach the server: {e}"));
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.dialing = None;
                self.fail("Couldn't reach the server.".into());
            }
        }
    }

    fn start_client(&mut self, conn: Connection) {
        let key_path = self.cfg.folder.join("live-key.bin");
        let key = match h2live::client::identity(&key_path) {
            Ok(k) => k,
            Err(e) => {
                return self.fail(format!("The sign-in key at {}: {e}", key_path.display()));
            }
        };
        let profile = Profile {
            gamertag: self.settings.gamertag.clone(),
            look: Default::default(),
            maps: self.cfg.maps.clone(),
            guests: 0,
        };
        let card = self.cfg.folder.join("live-card.txt");
        let now = self.clock.elapsed().as_secs_f64();
        let client = LiveClient::launcher(conn, key, &profile, crate::BUILD, &card, now);
        self.client = Some(client);
    }

    fn send(&mut self, m: ToServer) {
        if let Some(c) = &mut self.client {
            (self.cfg.log)(&format!("lobby: telling the server {}", live::describe(&m)));
            c.send(m);
        }
    }

    fn send_all(&mut self, out: Vec<ToServer>) {
        for m in out {
            self.send(m);
        }
    }

    fn poll_server(&mut self) {
        let now = self.clock.elapsed().as_secs_f64();
        let Some(c) = &mut self.client else {
            return;
        };
        let events = c.poll(now);
        for e in events {
            live::log_event(&self.cfg.log, &e);
            self.on_event(e);
        }
    }

    fn on_event(&mut self, e: LiveEvent) {
        if let Some(g) = &mut self.game {
            let out = g.told.next(g.m.id, g.host(), Some(&e), None);
            self.send_all(out);
        }
        match e {
            LiveEvent::Welcomed if self.screen == Screen::Connecting => self.signed_in(),
            LiveEvent::Refused(why) => {
                self.client = None;
                self.signing_in = None;
                if why == GAMERTAG_TAKEN {
                    self.screen = Screen::SignIn;
                    self.field = 0;
                    self.form_error = Some("Someone else has that gamertag. Pick another.".into());
                } else {
                    self.fail(format!("The server turned us away: {why}"));
                }
            }
            LiveEvent::Lost(why) => {
                self.client = None;
                self.fail(format!("Lost the server: {why}"));
            }
            LiveEvent::Notice(text) => {
                self.asked = None;
                self.toast(text);
            }
            LiveEvent::LauncherMatch(m) => self.on_match(m),
            LiveEvent::MatchOver(over) => self.on_match_over(over),
            _ => {}
        }
    }

    fn signed_in(&mut self) {
        self.signing_in = None;
        self.screen = Screen::Live;
        self.sel = 0;
        let path = self.cfg.folder.join("lobby.txt");
        if let Err(e) = self.settings.save(&path) {
            self.log(&format!("lobby: {e}"));
        }
        let server = self.settings.server.clone();
        self.log(&format!("lobby: signed in to {server}"));
    }

    /// Show `why` on the failure screen, once any match being played ends.
    fn fail(&mut self, why: String) {
        self.log(&format!("lobby: {why}"));
        self.failed = why;
        self.dialing = None;
        self.signing_in = None;
        let playing = self.game.as_ref().is_some_and(|g| g.child.is_some());
        if !playing {
            self.game = None;
            self.popup = None;
            self.screen = Screen::Failed;
        }
    }

    fn toast(&mut self, text: String) {
        self.log(&format!("lobby: notice: {text}"));
        if self.toasts.len() == 3 {
            self.toasts.pop_front();
        }
        self.toasts.push_back((text, self.now));
    }

    fn view(&self) -> Option<&View> {
        self.client.as_ref().map(|c| &c.view)
    }

    fn account(&self) -> Option<u64> {
        self.client.as_ref().and_then(|c| c.account())
    }

    /// The party searches.
    fn searching(&self) -> bool {
        self.view()
            .and_then(|v| v.party.as_ref())
            .is_some_and(|p| p.activity == Activity::Searching)
    }

    /// This PC's player leads the party (or there is none yet).
    fn leader(&self) -> bool {
        match (self.view().and_then(|v| v.party.as_ref()), self.account()) {
            (Some(p), Some(me)) => p.leader == me,
            _ => true,
        }
    }

    /// The invitation to show: the latest, on the party screens.
    fn invite(&self) -> Option<(u64, String)> {
        if !matches!(self.screen, Screen::Live | Screen::Players | Screen::Party)
            || self.popup.is_some()
        {
            return None;
        }
        self.view()?.invites.last().cloned()
    }

    /// Everyone signed in but us.
    fn others(&self) -> Vec<&OnlinePlayer> {
        let me = self.account();
        match self.view() {
            Some(v) => v.online.iter().filter(|o| Some(o.account) != me).collect(),
            None => Vec::new(),
        }
    }

    fn playlist_name(&self, id: u8) -> String {
        if id == CUSTOM_GAME {
            return "Custom Game".into();
        }
        self.view()
            .and_then(|v| v.playlists.iter().find(|p| p.id == id))
            .map(|p| p.name.clone())
            .unwrap_or_default()
    }

    // ------------------------------------------------------------- match

    fn on_match(&mut self, m: LauncherMatch) {
        let ip = self.client.as_ref().and_then(|c| c.server_ip());
        let made = ip
            .ok_or_else(|| "the server's address is unknown".to_string())
            .and_then(|ip| live::session_from(&m, ip));
        let (session, me) = match made {
            Ok(s) => s,
            Err(e) => {
                self.toast(format!("The match couldn't be joined: {e}"));
                self.send(ToServer::LeftMatch {
                    id: m.id,
                    host_lost: false,
                });
                return;
            }
        };
        if let Some(g) = self.game.as_mut().filter(|g| g.m.id == m.id) {
            // Again, with another host: start over with it.
            let restart = g.child.is_some();
            if let Some(mut c) = g.child.take() {
                c.kill();
            }
            g.m = m;
            g.session = session;
            g.me = me;
            g.told = Told::default();
            g.running = false;
            g.loaded = false;
            g.launch = true;
            if restart {
                g.start_at = self.now;
            }
            self.log("lobby: the match has a new host");
            return;
        }
        self.game = Some(Game {
            m,
            session,
            me,
            told: Told::default(),
            launch: true,
            start_at: self.now + PREGAME,
            child: None,
            running: false,
            loaded: false,
            results: None,
            ended_at: None,
            over: None,
        });
        self.popup = None;
        self.asked = None;
        self.screen = Screen::Pregame;
    }

    fn on_match_over(&mut self, over: MatchOver) {
        let Some(g) = self.game.as_mut().filter(|g| g.m.id == over.id) else {
            return;
        };
        let why = over.reason.clone();
        g.over = Some(over);
        if g.results.is_some() {
            return;
        }
        // The server ended the match before the game ended here.
        match g.child.as_mut() {
            Some(c) => c.ask_to_quit(),
            None => {
                self.game = None;
                if matches!(self.screen, Screen::Pregame | Screen::InGame) {
                    self.screen = Screen::Live;
                }
                self.toast(if why.is_empty() {
                    "The match was called off.".into()
                } else {
                    why
                });
            }
        }
    }

    fn poll_game(&mut self) {
        let now = self.now;
        let Some(g) = self.game.as_mut() else {
            return;
        };
        let (id, host) = (g.m.id, g.host());
        let mut out = Vec::new();
        let mut notes = Vec::new();
        let mut toast = None;
        if g.launch && g.child.is_none() && now >= g.start_at {
            g.launch = false;
            let m = Match {
                kind: self.cfg.kind.clone(),
                map: g.m.map.clone(),
                variant: g.m.variant.clone(),
                session: g.session.clone(),
                me: g.me,
                gamertag: self.settings.gamertag.clone(),
                instance: self.cfg.instance.clone(),
            };
            match Running::start(&self.cfg.exe, &self.cfg.folder, &m) {
                Ok(c) => {
                    notes.push(format!(
                        "lobby: engine started for match {id:016x} ({} on {}, {})",
                        m.variant,
                        m.map,
                        if host { "hosting" } else { "joining" }
                    ));
                    g.child = Some(c);
                    self.screen = Screen::InGame;
                }
                Err(e) => {
                    out.extend(g.told.next(id, host, None, Some(Engine::Closing)));
                    toast = Some(format!("The engine didn't start: {e}"));
                }
            }
        }
        let mut exited = false;
        if let Some(c) = g.child.as_mut() {
            for said in c.poll() {
                match said {
                    Said::Event(e) => {
                        notes.push(format!("lobby: the engine says {}", live::event_line(&e)));
                        match &e {
                            Engine::Running => g.running = true,
                            Engine::MapLoaded => {
                                g.running = true;
                                g.loaded = true;
                            }
                            Engine::Ended(players) => {
                                g.results = Some(players.clone());
                                g.ended_at = Some(now);
                            }
                            Engine::Closing => {}
                        }
                        out.extend(g.told.next(id, host, None, Some(e)));
                    }
                    Said::Line(l) => notes.push(format!("engine: {l}")),
                    Said::Done => {}
                }
            }
            // An engine that stays up after its game is asked to close,
            // and one that doesn't close when asked is stopped.
            if g.ended_at
                .is_some_and(|t| now.duration_since(t) >= POSTGAME)
            {
                c.ask_to_quit();
            }
            if c.asked_to_quit().is_some_and(|d| d >= QUIT_WAIT) {
                notes.push("lobby: the engine didn't close when asked; stopping it".into());
                c.kill();
            }
            exited = c.exited();
        }
        let mut done = false;
        if exited {
            g.child = None;
            notes.push("lobby: the engine closed".into());
            if !g.told.over() {
                out.extend(g.told.next(id, host, None, Some(Engine::Closing)));
            }
            if g.results.is_some() {
                self.screen = Screen::Carnage;
                self.focus = true;
            } else {
                let why = g.over.as_ref().map(|o| o.reason.clone());
                toast = Some(match why {
                    Some(w) if !w.is_empty() => w,
                    Some(_) => "The match was called off.".into(),
                    None => "You left the game.".into(),
                });
                done = true;
            }
        } else if toast.is_some() {
            done = true;
        }
        for n in &notes {
            self.log(n);
        }
        self.send_all(out);
        if done {
            self.game = None;
            self.screen = Screen::Live;
        }
        if let Some(t) = toast {
            self.toast(t);
        }
        if self.client.is_none() && self.game.as_ref().is_none_or(|g| g.child.is_none()) {
            // The server was lost during the match.
            self.game = None;
            if !matches!(self.screen, Screen::SignIn | Screen::Connecting) {
                self.screen = Screen::Failed;
            }
        }
    }

    // -------------------------------------------------------------- time

    /// Take on what the server and the engine said, and start what is due.
    pub fn tick(&mut self, now: Instant) {
        self.now = now;
        self.poll_dial();
        self.poll_server();
        self.poll_game();
        if self.screen == Screen::Connecting
            && self
                .signing_in
                .is_some_and(|t| now.duration_since(t) > SIGN_IN_WAIT + Duration::from_secs(1))
        {
            self.client = None;
            self.fail("The server didn't answer.".into());
        }
        self.toasts
            .retain(|(_, at)| now.saturating_duration_since(*at) < TOAST);
        // The playlists, then the custom game row.
        let n = self.view().map(|v| v.playlists.len()).unwrap_or(0);
        self.sel = self.sel.min(n);
        if self.screen == Screen::Custom && (!self.leader() || self.searching()) {
            self.screen = Screen::Live;
        }
        if self.screen == Screen::Party && self.searching() {
            self.screen = Screen::Live;
        }
        let n = self.party().map_or(0, |p| p.members.len());
        self.msel = self.msel.min(n.saturating_sub(1));
        if self
            .asked
            .is_some_and(|t| now.saturating_duration_since(t) >= CUSTOM_WAIT)
        {
            self.asked = None;
        }
        let n = self.others().len();
        self.psel = self.psel.min(n.saturating_sub(1));
    }

    /// The lobby is closing: close the engine of a match being played
    /// (it counts as leaving), waiting a little for it, then let go of
    /// the server.
    pub fn shutdown(&mut self) {
        if let Some(c) = self.game.as_mut().and_then(|g| g.child.as_mut()) {
            c.ask_to_quit();
            (self.cfg.log)("lobby: closing, and the engine with it");
        }
        let until = Instant::now() + QUIT_WAIT;
        while self.game.as_ref().is_some_and(|g| g.child.is_some()) && Instant::now() < until {
            self.tick(Instant::now());
            std::thread::sleep(Duration::from_millis(50));
        }
        if let Some(g) = self.game.as_mut() {
            if let Some(mut c) = g.child.take() {
                c.kill();
                if !g.told.over() {
                    let out = g.told.next(g.m.id, g.host(), None, Some(Engine::Closing));
                    self.send_all(out);
                }
            }
        }
        self.client = None;
    }

    // ------------------------------------------------------------- input

    pub fn input(&mut self, i: Input) {
        // Letters are buttons, except where a name is typed.
        let typing = self.screen == Screen::SignIn && self.popup.is_none();
        let i = match i {
            Input::Char(c) if !typing => match c.to_ascii_lowercase() {
                'a' | ' ' => Input::A,
                'b' => Input::B,
                'x' => Input::X,
                'y' => Input::Y,
                _ => return,
            },
            other => other,
        };
        if let Some(p) = self.popup {
            return self.popup_input(p, i);
        }
        if let Some((party, from)) = self.invite() {
            match i {
                Input::A => {
                    self.log(&format!("lobby: joining {from}'s party"));
                    self.send(ToServer::Accept(party));
                }
                Input::B => self.send(ToServer::Decline(party)),
                _ => {}
            }
            return;
        }
        match self.screen {
            Screen::SignIn => self.sign_in_input(i),
            Screen::Connecting => {
                if i == Input::B {
                    self.dialing = None;
                    self.client = None;
                    self.signing_in = None;
                    self.screen = Screen::SignIn;
                }
            }
            Screen::Live => self.live_input(i),
            Screen::Players => self.players_input(i),
            Screen::Party => self.party_input(i),
            Screen::Custom => self.custom_input(i),
            Screen::Pregame => {}
            Screen::InGame => {
                let ended = self.game.as_ref().is_some_and(|g| g.results.is_some());
                if i == Input::B && !ended {
                    self.popup = Some(Popup::LeaveGame);
                }
            }
            Screen::Carnage => {
                if matches!(i, Input::A | Input::B) {
                    // The leader of a custom game picks the next.
                    let custom = self
                        .game
                        .as_ref()
                        .is_some_and(|g| g.m.playlist == CUSTOM_GAME);
                    self.game = None;
                    self.screen = if custom && self.leader() {
                        Screen::Custom
                    } else {
                        Screen::Live
                    };
                }
            }
            Screen::Failed => match i {
                Input::A => self.connect(),
                Input::Y => {
                    self.form_error = None;
                    self.screen = Screen::SignIn;
                }
                Input::B => self.popup = Some(Popup::Quit),
                _ => {}
            },
        }
    }

    fn popup_input(&mut self, p: Popup, i: Input) {
        match (p, i) {
            (Popup::Quit, Input::A) => self.quit = true,
            (Popup::Quit, Input::Y) if self.client.is_some() => {
                self.log("lobby: signing out");
                self.client = None;
                self.popup = None;
                self.screen = Screen::SignIn;
            }
            (Popup::Kick(who), Input::A) => {
                self.popup = None;
                self.log(&format!("lobby: removing {who:#x} from the party"));
                self.send(ToServer::Kick(who));
            }
            (Popup::LeaveGame, Input::A) => {
                self.popup = None;
                if let Some(c) = self.game.as_mut().and_then(|g| g.child.as_mut()) {
                    c.ask_to_quit();
                }
                self.log("lobby: leaving the game");
            }
            (_, Input::B) => self.popup = None,
            _ => {}
        }
    }

    fn sign_in_input(&mut self, i: Input) {
        let f = &mut self.fields[self.field];
        match i {
            Input::Tab | Input::Up | Input::Down => self.field ^= 1,
            Input::Backspace => {
                f.pop();
            }
            Input::Char(c) if self.field == 0 => {
                if (c.is_ascii_alphanumeric() || c == ' ') && f.chars().count() < GAMERTAG_LEN {
                    f.push(c.to_ascii_uppercase());
                }
            }
            Input::Char(c) => {
                if !c.is_control() && f.chars().count() < SERVER_LEN {
                    f.push(c);
                }
            }
            Input::A => self.connect(),
            Input::B => self.popup = Some(Popup::Quit),
            _ => {}
        }
    }

    fn live_input(&mut self, i: Input) {
        if self.searching() {
            if i == Input::B {
                if self.leader() {
                    self.send(ToServer::Cancel);
                } else {
                    self.toast("Only the party leader can stop the search.".into());
                }
            }
            return;
        }
        // The playlists, then the custom game row.
        let n = self.view().map(|v| v.playlists.len()).unwrap_or(0);
        match i {
            Input::Up => self.sel = self.sel.saturating_sub(1),
            Input::Down => self.sel = (self.sel + 1).min(n),
            Input::A if self.sel == n => {
                if !self.leader() {
                    self.toast("Only the party leader can start a custom game.".into());
                    return;
                }
                self.open_custom();
            }
            Input::A => {
                let Some(pl) = self.view().and_then(|v| v.playlists.get(self.sel)) else {
                    return;
                };
                let (id, name) = (pl.id, pl.name.clone());
                if !self.leader() {
                    self.toast("Only the party leader can pick a playlist.".into());
                    return;
                }
                self.log(&format!("lobby: searching {name:?} (playlist {id})"));
                if let Some(c) = &mut self.client {
                    c.view.status = None;
                }
                self.send(ToServer::Search(id));
            }
            Input::X => {
                self.psel = 0;
                self.screen = Screen::Players;
            }
            Input::Y if self.party().is_some() => {
                // Start on this PC's player.
                let me = self.account();
                self.msel = self
                    .party()
                    .and_then(|p| p.members.iter().position(|m| Some(m.account) == me))
                    .unwrap_or(0);
                self.screen = Screen::Party;
            }
            Input::B => self.popup = Some(Popup::Quit),
            _ => {}
        }
    }

    fn party(&self) -> Option<&PartyInfo> {
        self.view().and_then(|v| v.party.as_ref())
    }

    /// The selected party member, and whether it is this PC's player.
    fn picked_member(&self) -> Option<(&PartyMember, bool)> {
        let m = self.party()?.members.get(self.msel)?;
        Some((m, Some(m.account) == self.account()))
    }

    fn party_input(&mut self, i: Input) {
        let n = self.party().map_or(0, |p| p.members.len());
        let leader = self.leader();
        let picked = self.picked_member().map(|(m, me)| (m.account, me));
        match (i, picked) {
            (Input::Up, _) => self.msel = self.msel.saturating_sub(1),
            (Input::Down, _) => self.msel = (self.msel + 1).min(n.saturating_sub(1)),
            (Input::A, Some((_, true))) if n > 1 => {
                self.log("lobby: leaving the party");
                self.send(ToServer::LeaveParty);
                self.screen = Screen::Live;
            }
            (Input::A, Some((who, false))) if leader => {
                self.log(&format!("lobby: making {who:#x} party leader"));
                self.send(ToServer::Promote(who));
            }
            (Input::X, Some((who, false))) if leader => self.popup = Some(Popup::Kick(who)),
            (Input::Y, _) if leader => {
                let privacy = match self.party().map(|p| p.privacy) {
                    Some(Privacy::Open) => Privacy::InviteOnly,
                    _ => Privacy::Open,
                };
                self.send(ToServer::Privacy(privacy));
            }
            (Input::B, _) => self.screen = Screen::Live,
            _ => {}
        }
    }

    /// The custom game screen, with a map picked: Lockout the first time,
    /// if this PC has it.
    fn open_custom(&mut self) {
        if self.screen != Screen::Custom && self.cmap == 0 {
            let lockout = self.cfg.maps.iter().position(|(m, _)| m == "lockout");
            self.cmap = lockout.unwrap_or(0);
        }
        self.crow = 0;
        self.asked = None;
        self.screen = Screen::Custom;
    }

    fn custom_input(&mut self, i: Input) {
        let maps = self.cfg.maps.len();
        let step = |at: usize, n: usize, on: bool| match (n, on) {
            (0, _) => 0,
            (_, true) => (at + 1) % n,
            (_, false) => (at + n - 1) % n,
        };
        match i {
            Input::Up => self.crow = self.crow.saturating_sub(1),
            Input::Down => self.crow = (self.crow + 1).min(CUSTOM_ROWS - 1),
            Input::Left | Input::Right => {
                let on = i == Input::Right;
                match self.crow {
                    0 => self.cgame = step(self.cgame, names::CUSTOM_GAMES.len(), on),
                    1 => self.cmap = step(self.cmap, maps, on),
                    _ => {}
                }
            }
            Input::A => self.start_custom(),
            Input::B => self.screen = Screen::Live,
            _ => {}
        }
    }

    /// Ask the server for the custom game picked.
    fn start_custom(&mut self) {
        if self.asked.is_some() {
            return;
        }
        let Some((map, _)) = self.cfg.maps.get(self.cmap).cloned() else {
            self.toast("No Halo 2 maps were found in MCC's folder.".into());
            return;
        };
        let (variant, _) = names::CUSTOM_GAMES[self.cgame % names::CUSTOM_GAMES.len()];
        self.log(&format!(
            "lobby: starting a custom game of {variant} on {map}"
        ));
        self.asked = Some(self.now);
        self.send(ToServer::LauncherCustom {
            map,
            variant: variant.into(),
        });
    }

    fn players_input(&mut self, i: Input) {
        let others: Vec<OnlinePlayer> = self.others().into_iter().cloned().collect();
        let picked = others.get(self.psel);
        match i {
            Input::Up => self.psel = self.psel.saturating_sub(1),
            Input::Down => self.psel = (self.psel + 1).min(others.len().saturating_sub(1)),
            Input::A => {
                if let Some(p) = picked {
                    self.send(ToServer::Invite(p.account));
                    self.toast(format!("Invited {} to your party.", p.gamertag));
                }
            }
            Input::X => {
                if let Some(p) = picked.filter(|p| self.can_join(p)) {
                    self.send(ToServer::JoinParty(p.party));
                    self.screen = Screen::Live;
                }
            }
            Input::B => self.screen = Screen::Live,
            _ => {}
        }
    }

    /// `p`'s party takes anyone, has room, and isn't ours.
    fn can_join(&self, p: &OnlinePlayer) -> bool {
        let ours = self.view().and_then(|v| v.party.as_ref()).map(|pt| pt.id);
        p.open && p.openings > 0 && ours != Some(p.party)
    }

    /// A click at window pixel (`x`, `y`).
    pub fn click(&mut self, x: f32, y: f32) {
        let (s, ox, oy) = self.view;
        let (x, y) = ((x - ox) / s, (y - oy) / s);
        let top = self.areas.iter().any(|a| a.top);
        let hit = self
            .areas
            .iter()
            .rev()
            .filter(|a| a.top == top)
            .find(|a| x >= a.x && x < a.x + a.w && y >= a.y && y < a.y + a.h)
            .map(|a| a.hit);
        match hit {
            Some(Hit::Press(i)) => self.input(i),
            Some(Hit::Field(f)) => self.field = f,
            Some(Hit::Step(r, on)) => {
                self.crow = r;
                self.input(if on { Input::Right } else { Input::Left });
            }
            Some(Hit::Row(r)) if self.screen == Screen::Custom => {
                if self.crow == r && r == CUSTOM_ROWS - 1 {
                    self.input(Input::A);
                } else {
                    self.crow = r;
                }
            }
            Some(Hit::Row(r)) => {
                let sel = match self.screen {
                    Screen::Players => &mut self.psel,
                    Screen::Party => &mut self.msel,
                    _ => &mut self.sel,
                };
                if *sel == r {
                    self.input(Input::A);
                } else {
                    *sel = r;
                }
            }
            None => {}
        }
    }

    // -------------------------------------------------------------- draw

    /// Draw the screen into `c`.
    pub fn draw(&mut self, c: &mut Canvas, t: &mut Text) {
        let (w, h) = (c.w as f32, c.h as f32);
        let s = (w / 1280.0).min(h / 720.0).max(0.01);
        let (ox, oy) = ((w - 1280.0 * s) / 2.0, (h - 720.0 * s) / 2.0);
        c.gradient(0.0, 0.0, w, h, BG_TOP, BG_BOTTOM);
        c.fill(0.0, 0.0, w, oy + 88.0 * s, Color::rgb(0).alpha(90));
        c.fill(0.0, oy + 88.0 * s, w, (2.0 * s).max(1.0), EDGE.alpha(120));
        let mut areas = Vec::new();
        let mut p = Pen {
            c,
            t,
            s,
            ox,
            oy,
            areas: &mut areas,
            top: false,
        };
        self.draw_screen(&mut p);
        self.areas = areas;
        self.view = (s, ox, oy);
    }

    fn draw_screen(&self, p: &mut Pen) {
        let title = match self.screen {
            Screen::SignIn => "SIGN IN",
            Screen::Connecting => "SIGNING IN",
            Screen::Live if self.searching() => "MATCHMAKING",
            Screen::Live => "PLAYLISTS",
            Screen::Players => "PLAYERS ONLINE",
            Screen::Party => "PARTY",
            Screen::Custom => "CUSTOM GAME",
            Screen::Pregame => "PREGAME LOBBY",
            Screen::InGame => "IN GAME",
            Screen::Carnage => "CARNAGE REPORT",
            Screen::Failed => "OFFLINE",
        };
        p.text(60.0, 58.0, 36.0, WHITE, Align::Left, title);
        self.draw_header(p);
        match self.screen {
            Screen::SignIn => self.draw_sign_in(p),
            Screen::Connecting => self.draw_connecting(p),
            Screen::Live => self.draw_live(p),
            Screen::Players => self.draw_players(p),
            Screen::Party => self.draw_party_screen(p),
            Screen::Custom => self.draw_custom(p),
            Screen::Pregame => self.draw_pregame(p),
            Screen::InGame => self.draw_in_game(p),
            Screen::Carnage => self.draw_carnage(p),
            Screen::Failed => self.draw_failed(p),
        }
        p.text(
            1220.0,
            712.0,
            14.5,
            DIM,
            Align::Right,
            "Keyboard: Enter = A, Esc = B, X, Y, arrows",
        );
        self.draw_toasts(p);
        if let Some(popup) = self.popup {
            match popup {
                Popup::Quit => {
                    let mut hints = vec![(Input::A, "Quit"), (Input::B, "Stay")];
                    if self.client.is_some() {
                        hints.insert(1, (Input::Y, "Sign out"));
                    }
                    p.popup("QUIT", "Close the launcher?", &hints);
                }
                Popup::LeaveGame => p.popup(
                    "LEAVE GAME",
                    "Leave the game in progress? In a ranked playlist it counts as a loss.",
                    &[(Input::A, "Leave"), (Input::B, "Stay")],
                ),
                Popup::Kick(who) => {
                    let name = self
                        .party()
                        .and_then(|pt| pt.members.iter().find(|m| m.account == who))
                        .map_or("this player".into(), |m| m.gamertag.clone());
                    p.popup(
                        "REMOVE PLAYER",
                        &format!(
                            "Remove {name} from your party? They can come back only if invited."
                        ),
                        &[(Input::A, "Remove"), (Input::B, "Cancel")],
                    );
                }
            }
        } else if let Some((_, from)) = self.invite() {
            p.popup(
                "PARTY INVITATION",
                &format!("{from} invited you to their party."),
                &[(Input::A, "Accept"), (Input::B, "Decline")],
            );
        }
    }

    fn draw_header(&self, p: &mut Pen) {
        let Some(c) = &self.client else {
            return;
        };
        let Some(w) = &c.view.welcome else {
            return;
        };
        p.text(1220.0, 44.0, 24.0, WHITE, Align::Right, &w.gamertag);
        let mut line = format!("Level {}  ·  {} online", w.best.max(1), c.view.online.len());
        if let Some(ms) = c.view.round_trip {
            line += &format!("  ·  {ms} ms");
        }
        p.text(1220.0, 70.0, 16.0, DIM, Align::Right, &line);
    }

    fn draw_sign_in(&self, p: &mut Pen) {
        p.panel(340.0, 150.0, 600.0, 380.0);
        p.text(
            640.0,
            200.0,
            20.0,
            TEXT,
            Align::Center,
            "Pick a gamertag and the matchmaking server.",
        );
        let labels = ["Gamertag", "Server"];
        for (k, label) in labels.iter().enumerate() {
            let y = 240.0 + k as f32 * 110.0;
            p.text(380.0, y + 10.0, 18.0, DIM, Align::Left, label);
            let on = self.field == k;
            p.fill(
                380.0,
                y + 20.0,
                520.0,
                52.0,
                Color::rgb(0x06_1430).alpha(220),
            );
            p.outline(
                380.0,
                y + 20.0,
                520.0,
                52.0,
                2.0,
                if on { SEL_EDGE } else { EDGE },
            );
            let shown = p.fit(26.0, 480.0, &self.fields[k]);
            let wd = p.text(396.0, y + 56.0, 26.0, WHITE, Align::Left, &shown);
            if on {
                p.fill(398.0 + wd, y + 32.0, 2.0, 30.0, SEL_EDGE);
            }
            p.area(380.0, y + 20.0, 520.0, 52.0, Hit::Field(k));
        }
        if let Some(e) = &self.form_error {
            p.text(640.0, 470.0, 20.0, ERROR, Align::Center, e);
        }
        p.text(
            640.0,
            505.0,
            16.0,
            DIM,
            Align::Center,
            "Tab moves between the two.",
        );
        p.hints(&[(Input::A, "Sign in"), (Input::B, "Quit")]);
    }

    fn draw_connecting(&self, p: &mut Pen) {
        let line = format!(
            "Signing in to {} as {}",
            self.settings.server, self.settings.gamertag
        );
        p.text(640.0, 320.0, 26.0, TEXT, Align::Center, &line);
        p.spinner(640.0, 420.0, self.clock.elapsed().as_secs_f32());
        p.hints(&[(Input::B, "Cancel")]);
    }

    fn draw_failed(&self, p: &mut Pen) {
        let lines = p.wrap(26.0, 1000.0, &self.failed);
        for (k, l) in lines.iter().enumerate() {
            p.text(640.0, 280.0 + k as f32 * 36.0, 26.0, TEXT, Align::Center, l);
        }
        let who = format!(
            "Server {}  ·  Gamertag {}",
            self.settings.server, self.settings.gamertag
        );
        p.text(640.0, 460.0, 18.0, DIM, Align::Center, &who);
        p.hints(&[
            (Input::A, "Try again"),
            (Input::Y, "Change gamertag or server"),
            (Input::B, "Quit"),
        ]);
    }

    fn draw_live(&self, p: &mut Pen) {
        let Some(v) = self.view() else {
            return;
        };
        if self.searching() {
            self.draw_search(p, v);
            if self.leader() {
                p.hints(&[(Input::B, "Stop searching")]);
            }
        } else {
            self.draw_playlists(p, v);
            let mut hints = Vec::new();
            if self.leader() {
                let custom = self.sel == v.playlists.len();
                hints.push((Input::A, if custom { "Custom game" } else { "Search" }));
            }
            hints.push((Input::X, "Players"));
            if v.party.is_some() {
                hints.push((Input::Y, "Party"));
            }
            hints.push((Input::B, "Quit"));
            p.hints(&hints);
        }
        self.draw_party(p, v);
    }

    fn draw_playlists(&self, p: &mut Pen, v: &View) {
        p.panel(60.0, 110.0, 720.0, 530.0);
        p.text(84.0, 142.0, 16.0, HEAD, Align::Left, "PLAYLIST");
        p.text(560.0, 142.0, 16.0, HEAD, Align::Center, "LEVEL");
        p.text(650.0, 142.0, 16.0, HEAD, Align::Center, "SEARCHING");
        p.text(735.0, 142.0, 16.0, HEAD, Align::Center, "PLAYING");
        let rows = 8;
        // The playlists, then the custom game row.
        let n = v.playlists.len() + 1;
        let first = first_row(self.sel, n, rows);
        for k in (first..n).take(rows) {
            let y = 156.0 + (k - first) as f32 * 50.0;
            let on = k == self.sel;
            p.row(72.0, y, 696.0, 46.0, on, Hit::Row(k));
            let col = if on { WHITE } else { TEXT };
            let Some(pl) = v.playlists.get(k) else {
                p.text(90.0, y + 32.0, 24.0, col, Align::Left, "Custom Game");
                p.text(560.0, y + 32.0, 22.0, col, Align::Center, "-");
                continue;
            };
            let name = p.fit(24.0, 420.0, &pl.name);
            p.text(90.0, y + 32.0, 24.0, col, Align::Left, &name);
            let level = if pl.ranked {
                pl.level.max(1).to_string()
            } else {
                "-".into()
            };
            p.text(560.0, y + 32.0, 22.0, col, Align::Center, &level);
            let searching = pl.searching.to_string();
            p.text(650.0, y + 32.0, 20.0, DIM, Align::Center, &searching);
            p.text(
                735.0,
                y + 32.0,
                20.0,
                DIM,
                Align::Center,
                &pl.playing.to_string(),
            );
        }
        if v.playlists.is_empty() {
            p.text(
                420.0,
                320.0,
                22.0,
                DIM,
                Align::Center,
                "The server has no playlists for the launcher.",
            );
        }
        if self.sel == v.playlists.len() {
            let line = "Any map and game type, with your party. Unranked.";
            p.text(84.0, 600.0, 18.0, DIM, Align::Left, line);
        } else if let Some(pl) = v.playlists.get(self.sel) {
            let mut parts = vec![if pl.min == pl.max {
                format!("{} players", pl.max)
            } else {
                format!("{} to {} players", pl.min, pl.max)
            }];
            parts.push(if pl.teams { "Teams" } else { "Free for all" }.into());
            parts.push(if pl.ranked { "Ranked" } else { "Unranked" }.into());
            if pl.party_max > 1 {
                parts.push(format!("parties of up to {}", pl.party_max));
            }
            p.text(84.0, 600.0, 18.0, DIM, Align::Left, &parts.join("  ·  "));
        }
        if self.cfg.maps.is_empty() {
            p.text(
                84.0,
                626.0,
                17.0,
                WARN,
                Align::Left,
                "No Halo 2 maps were found in MCC's folder, so no match can be played.",
            );
        }
    }

    fn draw_search(&self, p: &mut Pen, v: &View) {
        p.panel(60.0, 110.0, 720.0, 530.0);
        let playlist = v
            .party
            .as_ref()
            .map(|pt| self.playlist_name(pt.playlist))
            .unwrap_or_default();
        p.text(420.0, 190.0, 34.0, WHITE, Align::Center, &playlist);
        let stage = match v.status.map(|s| s.stage) {
            None | Some(Stage::Searching) => "Searching for a match",
            Some(Stage::Gathering) => "Gathering players",
            Some(Stage::WaitingToFill) => "Waiting for more players",
            Some(Stage::Balancing) => "Balancing teams",
            Some(Stage::Joining) => "Joining the match",
            Some(Stage::Starting) => "Starting the match",
            Some(Stage::Failed) => "No match found",
        };
        p.text(420.0, 290.0, 30.0, TEXT, Align::Center, stage);
        if let Some(s) = v.status {
            if s.have > 0 || s.need > 0 {
                let found = format!("{} found, {} more needed", s.have, s.need);
                p.text(420.0, 340.0, 22.0, DIM, Align::Center, &found);
            }
            if s.high > 0 {
                let levels = format!("Levels {} to {}", s.low, s.high);
                p.text(420.0, 374.0, 22.0, DIM, Align::Center, &levels);
            }
            let clock = format!("{}:{:02}", s.seconds / 60, s.seconds % 60);
            let line = if s.stage == Stage::WaitingToFill {
                format!("Starting in {clock}")
            } else {
                clock
            };
            p.text(420.0, 408.0, 22.0, DIM, Align::Center, &line);
        }
        p.spinner(420.0, 510.0, self.clock.elapsed().as_secs_f32());
    }

    fn draw_party(&self, p: &mut Pen, v: &View) {
        p.panel(810.0, 110.0, 410.0, 530.0);
        let Some(pt) = &v.party else {
            p.text(834.0, 142.0, 16.0, HEAD, Align::Left, "PARTY");
            return;
        };
        let title = if pt.members.len() > 1 {
            format!("PARTY ({})", pt.members.len())
        } else {
            "PARTY".into()
        };
        p.text(834.0, 142.0, 16.0, HEAD, Align::Left, &title);
        let privacy = match pt.privacy {
            Privacy::Open => "Open",
            Privacy::InviteOnly => "Invite only",
        };
        p.text(1196.0, 142.0, 16.0, DIM, Align::Right, privacy);
        let me = self.account();
        for (k, m) in pt.members.iter().enumerate().take(9) {
            let y = 156.0 + k as f32 * 50.0;
            if Some(m.account) == me {
                p.fill(822.0, y, 386.0, 46.0, SEL.alpha(90));
            }
            if m.account == pt.leader {
                p.circle(842.0, y + 23.0, 6.0, GOLD);
            }
            let name = p.fit(24.0, 260.0, &m.gamertag);
            p.text(860.0, y + 32.0, 24.0, TEXT, Align::Left, &name);
            let level = if m.level > 0 { m.level } else { m.best.max(1) };
            p.text(
                1196.0,
                y + 32.0,
                22.0,
                TEXT,
                Align::Right,
                &level.to_string(),
            );
        }
        if pt.members.len() == 1 {
            p.text(
                1015.0,
                260.0,
                17.0,
                DIM,
                Align::Center,
                "Press X to invite players.",
            );
        }
    }

    /// The party screen: its members, each one's level and role.
    fn draw_party_screen(&self, p: &mut Pen) {
        let Some(pt) = self.party() else {
            return;
        };
        p.panel(60.0, 110.0, 1160.0, 530.0);
        p.text(84.0, 142.0, 16.0, HEAD, Align::Left, "GAMERTAG");
        p.text(560.0, 142.0, 16.0, HEAD, Align::Center, "LEVEL");
        p.text(660.0, 142.0, 16.0, HEAD, Align::Left, "ROLE");
        let privacy = match pt.privacy {
            Privacy::Open => "Anyone can join",
            Privacy::InviteOnly => "Invite only",
        };
        p.text(1196.0, 142.0, 16.0, DIM, Align::Right, privacy);
        let me = self.account();
        let rows = 9;
        let first = first_row(self.msel, pt.members.len(), rows);
        for (k, m) in pt.members.iter().enumerate().skip(first).take(rows) {
            let y = 156.0 + (k - first) as f32 * 50.0;
            let on = k == self.msel;
            p.row(72.0, y, 1136.0, 46.0, on, Hit::Row(k));
            let col = if on { WHITE } else { TEXT };
            if m.account == pt.leader {
                p.circle(90.0, y + 23.0, 6.0, GOLD);
            }
            let name = p.fit(24.0, 400.0, &m.gamertag);
            p.text(108.0, y + 32.0, 24.0, col, Align::Left, &name);
            let level = if m.level > 0 { m.level } else { m.best.max(1) };
            p.text(
                560.0,
                y + 32.0,
                22.0,
                col,
                Align::Center,
                &level.to_string(),
            );
            let mut role = if m.account == pt.leader {
                "Party leader".to_string()
            } else {
                "Member".to_string()
            };
            if Some(m.account) == me {
                role.push_str(" (you)");
            }
            if m.guests > 0 {
                role.push_str(&format!(", +{} guest", m.guests));
                if m.guests > 1 {
                    role.push('s');
                }
            }
            p.text(660.0, y + 32.0, 20.0, DIM, Align::Left, &role);
        }
        if pt.members.len() == 1 {
            p.text(
                640.0,
                260.0,
                17.0,
                DIM,
                Align::Center,
                "No one else is in your party. Invite players from Players (X on the playlists).",
            );
        }
        let mut hints = Vec::new();
        match self.picked_member() {
            Some((_, true)) if pt.members.len() > 1 => hints.push((Input::A, "Leave party")),
            Some((_, false)) if self.leader() => {
                hints.push((Input::A, "Make party leader"));
                hints.push((Input::X, "Remove"));
            }
            _ => {}
        }
        if self.leader() {
            hints.push((
                Input::Y,
                match pt.privacy {
                    Privacy::Open => "Make invite only",
                    Privacy::InviteOnly => "Let anyone join",
                },
            ));
        }
        hints.push((Input::B, "Back"));
        p.hints(&hints);
    }

    fn draw_custom(&self, p: &mut Pen) {
        p.panel(60.0, 110.0, 720.0, 530.0);
        let (variant, teams) = names::CUSTOM_GAMES[self.cgame % names::CUSTOM_GAMES.len()];
        let map = self
            .cfg
            .maps
            .get(self.cmap)
            .map_or("No maps".to_string(), |(m, _)| names::map(m));
        let choices = [("GAME TYPE", names::variant(variant)), ("MAP", map)];
        for (k, (label, value)) in choices.iter().enumerate() {
            let y = 140.0 + k as f32 * 110.0;
            let on = self.crow == k;
            p.row(72.0, y, 696.0, 96.0, on, Hit::Row(k));
            p.text(96.0, y + 28.0, 16.0, HEAD, Align::Left, label);
            let col = if on { WHITE } else { TEXT };
            p.text(420.0, y + 72.0, 32.0, col, Align::Center, value);
            for (x, text, forward) in [(110.0, "<", false), (730.0, ">", true)] {
                let arrow = if on { SEL_EDGE } else { DIM };
                p.text(x, y + 72.0, 32.0, arrow, Align::Center, text);
                p.area(x - 30.0, y + 30.0, 60.0, 60.0, Hit::Step(k, forward));
            }
        }
        let y = 370.0;
        let on = self.crow == CUSTOM_ROWS - 1;
        p.row(72.0, y, 696.0, 60.0, on, Hit::Row(CUSTOM_ROWS - 1));
        let label = if self.asked.is_some() {
            "STARTING..."
        } else {
            "START GAME"
        };
        let col = if on { WHITE } else { TEXT };
        p.text(420.0, y + 40.0, 26.0, col, Align::Center, label);
        let kind = if teams { "Teams" } else { "Free for all" };
        let people = self
            .view()
            .and_then(|v| v.party.as_ref())
            .map_or(1, |pt| pt.members.len());
        let who = if people > 1 {
            format!("{kind}  ·  Unranked  ·  Your party of {people} plays; you host.")
        } else {
            format!("{kind}  ·  Unranked  ·  Invite players with X on the playlists.")
        };
        p.text(84.0, 480.0, 18.0, DIM, Align::Left, &who);
        p.text(
            84.0,
            510.0,
            18.0,
            DIM,
            Align::Left,
            "Left and right change the game type and the map.",
        );
        if self.cfg.maps.is_empty() {
            p.text(
                84.0,
                600.0,
                17.0,
                WARN,
                Align::Left,
                "No Halo 2 maps were found in MCC's folder, so no game can be played.",
            );
        }
        if let Some(v) = self.view() {
            self.draw_party(p, v);
        }
        p.hints(&[(Input::A, "Start game"), (Input::B, "Back")]);
    }

    fn draw_players(&self, p: &mut Pen) {
        p.panel(60.0, 110.0, 1160.0, 530.0);
        p.text(84.0, 142.0, 16.0, HEAD, Align::Left, "GAMERTAG");
        p.text(560.0, 142.0, 16.0, HEAD, Align::Center, "LEVEL");
        p.text(660.0, 142.0, 16.0, HEAD, Align::Left, "DOING");
        p.text(900.0, 142.0, 16.0, HEAD, Align::Left, "PARTY");
        let others = self.others();
        let ours = self.view().and_then(|v| v.party.as_ref()).map(|pt| pt.id);
        let rows = 9;
        let first = first_row(self.psel, others.len(), rows);
        for (k, o) in others.iter().enumerate().skip(first).take(rows) {
            let y = 156.0 + (k - first) as f32 * 50.0;
            let on = k == self.psel;
            p.row(72.0, y, 1136.0, 46.0, on, Hit::Row(k));
            let col = if on { WHITE } else { TEXT };
            let name = p.fit(24.0, 420.0, &o.gamertag);
            p.text(90.0, y + 32.0, 24.0, col, Align::Left, &name);
            p.text(
                560.0,
                y + 32.0,
                22.0,
                col,
                Align::Center,
                &o.best.max(1).to_string(),
            );
            let doing = match o.activity {
                Activity::Lobby => "In a lobby",
                Activity::Searching => "Searching",
                Activity::Playing => "Playing",
                Activity::Custom => "Custom game",
            };
            p.text(660.0, y + 32.0, 20.0, DIM, Align::Left, doing);
            let party = if ours == Some(o.party) {
                "Your party".to_string()
            } else {
                let people = if o.size == 1 { "person" } else { "people" };
                let state = match (o.open, o.openings) {
                    (true, 0) => "full",
                    (true, _) => "open",
                    (false, _) => "invite only",
                };
                format!("{} {people}, {state}", o.size)
            };
            p.text(900.0, y + 32.0, 20.0, DIM, Align::Left, &party);
        }
        if others.is_empty() {
            p.text(
                640.0,
                320.0,
                22.0,
                DIM,
                Align::Center,
                "No one else is signed in.",
            );
        }
        let mut hints = Vec::new();
        if let Some(o) = others.get(self.psel) {
            hints.push((Input::A, "Invite"));
            if self.can_join(o) {
                hints.push((Input::X, "Join their party"));
            }
        }
        hints.push((Input::B, "Back"));
        p.hints(&hints);
    }

    /// The match's name lines: the map big, then the game and playlist.
    fn draw_match_title(&self, p: &mut Pen, g: &Game, x: f32, align: Align) {
        p.text(x, 160.0, 48.0, WHITE, align, &names::map(&g.m.map));
        let mut line = names::variant(&g.m.variant);
        let playlist = self.playlist_name(g.m.playlist);
        if !playlist.is_empty() {
            line += &format!("  ·  {playlist}");
        }
        p.text(x, 200.0, 24.0, TEXT, align, &line);
    }

    fn draw_pregame(&self, p: &mut Pen) {
        let Some(g) = &self.game else {
            return;
        };
        self.draw_match_title(p, g, 60.0, Align::Left);
        let left = g.start_at.saturating_duration_since(self.now).as_secs_f32();
        p.text(1220.0, 150.0, 20.0, DIM, Align::Right, "Starting in");
        let secs = format!("{}", left.ceil() as u32);
        p.text(1220.0, 200.0, 44.0, WHITE, Align::Right, &secs);
        p.panel(60.0, 240.0, 1160.0, 400.0);
        // Teams together, in team order; everyone for themselves as listed.
        let mut players: Vec<(usize, i32)> = g
            .session
            .players
            .iter()
            .enumerate()
            .map(|(i, pl)| (i, pl.team))
            .collect();
        if g.m.teams {
            players.sort_by_key(|&(i, team)| (team, i));
        }
        let two = players.len() > 7;
        let rows = if two {
            players.len().div_ceil(2)
        } else {
            players.len()
        };
        let h = (370.0 / rows.max(1) as f32).min(48.0);
        for (k, &(i, team)) in players.iter().enumerate() {
            let (col, row) = if two { (k / rows, k % rows) } else { (0, k) };
            let x = 76.0 + col as f32 * 572.0;
            let w = if two { 556.0 } else { 1128.0 };
            let y = 256.0 + row as f32 * h;
            let pl = &g.m.players[i];
            p.fill(x, y, w, h - 4.0, Color::rgb(0x06_1430).alpha(160));
            p.fill(x, y, 10.0, h - 4.0, names::team_color(team));
            let size = (h * 0.5).min(24.0);
            let name = p.fit(size, w - 200.0, &pl.gamertag);
            let col = if i == g.me { WHITE } else { TEXT };
            let base = y + h * 0.5 + size * 0.35;
            p.text(x + 26.0, base, size, col, Align::Left, &name);
            if pl.relay_id == g.m.host {
                p.text(x + w - 70.0, base, size * 0.7, GOLD, Align::Right, "HOST");
            }
            p.text(
                x + w - 16.0,
                base,
                size,
                TEXT,
                Align::Right,
                &pl.level.max(1).to_string(),
            );
        }
    }

    fn draw_in_game(&self, p: &mut Pen) {
        let Some(g) = &self.game else {
            return;
        };
        self.draw_match_title(p, g, 640.0, Align::Center);
        let status = if g.results.is_some() {
            "The game is over. Waiting for the engine to close."
        } else if g.loaded {
            "The game is on in the engine's window."
        } else if g.running {
            "Loading the map."
        } else if g.child.is_some() {
            "Starting the engine."
        } else {
            "Starting again with a new host."
        };
        p.text(640.0, 360.0, 28.0, TEXT, Align::Center, status);
        let role = if g.host() {
            "You host this game."
        } else {
            "You join this game."
        };
        p.text(640.0, 404.0, 20.0, DIM, Align::Center, role);
        if g.results.is_none() {
            p.hints(&[(Input::B, "Leave game")]);
        }
    }

    fn draw_carnage(&self, p: &mut Pen) {
        let Some(g) = &self.game else {
            return;
        };
        let title = format!(
            "{} on {}",
            names::variant(&g.m.variant),
            names::map(&g.m.map)
        );
        p.text(60.0, 138.0, 28.0, WHITE, Align::Left, &title);
        let playlist = self.playlist_name(g.m.playlist);
        p.text(1220.0, 138.0, 22.0, DIM, Align::Right, &playlist);
        let (verdict, col) = match &g.over {
            None => ("Waiting for everyone's results.".to_string(), DIM),
            Some(o) if o.counted => match o.levels.iter().find(|l| l.0 == g.m.playlist) {
                Some(&(_, old, new)) if new > old => (format!("Level up! {old} to {new}"), GOOD),
                Some(&(_, old, new)) if new < old => (format!("Level {old} to {new}"), WARN),
                Some(&(_, _, new)) => (format!("Level {new}"), TEXT),
                None => ("The game counted.".to_string(), TEXT),
            },
            Some(o) if o.reason.is_empty() && !g.m.ranked => {
                ("Unranked: levels don't change.".to_string(), DIM)
            }
            Some(o) if o.reason.is_empty() => ("The game didn't count.".to_string(), WARN),
            Some(o) => (o.reason.clone(), WARN),
        };
        p.text(60.0, 176.0, 20.0, col, Align::Left, &verdict);
        p.panel(60.0, 196.0, 1160.0, 444.0);
        let results = g.results.clone().unwrap_or_default();
        let kills = results.iter().any(|r| r.kills > 0);
        let heads = [
            (100.0, Align::Left, "PLACE"),
            (200.0, Align::Left, "PLAYER"),
            (760.0, Align::Center, "SCORE"),
            (860.0, Align::Center, "KILLS"),
            (960.0, Align::Center, "DEATHS"),
            (1080.0, Align::Center, "LEVEL"),
        ];
        for (x, a, h) in heads {
            p.text(x, 228.0, 16.0, HEAD, a, h);
        }
        let mut rows: Vec<&LauncherPlayerResult> = results.iter().collect();
        rows.sort_by_key(|r| (r.place, -r.score));
        let h = (380.0 / rows.len().max(1) as f32).min(44.0);
        for (k, r) in rows.iter().enumerate() {
            let y = 242.0 + k as f32 * h;
            let pl = g.m.players.iter().find(|pl| pl.relay_id == r.relay_id);
            let me = r.relay_id == g.m.relay.id;
            if me {
                p.fill(72.0, y, 1136.0, h - 4.0, SEL.alpha(110));
            }
            let team = g
                .session
                .players
                .iter()
                .find(|s| s.xuid == r.relay_id)
                .map_or(i32::from(r.team), |s| s.team);
            p.fill(76.0, y, 10.0, h - 4.0, names::team_color(team));
            let size = (h * 0.5).min(22.0);
            let base = y + h * 0.5 + size * 0.35;
            p.text(100.0, base, size, TEXT, Align::Left, &names::place(r.place));
            let name = pl.map_or("?", |pl| pl.gamertag.as_str());
            let mut name = p.fit(size, 520.0, name);
            if r.left {
                name += "  (quit)";
            }
            let col = if me { WHITE } else { TEXT };
            p.text(200.0, base, size, col, Align::Left, &name);
            p.text(760.0, base, size, col, Align::Center, &r.score.to_string());
            let k = if kills {
                r.kills.to_string()
            } else {
                "-".into()
            };
            p.text(860.0, base, size, col, Align::Center, &k);
            p.text(960.0, base, size, col, Align::Center, &r.deaths.to_string());
            let level = pl.map(|pl| {
                g.over
                    .as_ref()
                    .and_then(|o| o.players.iter().find(|(a, _)| *a == pl.account))
                    .map_or(pl.level, |&(_, l)| l)
            });
            let level = level.map_or("-".into(), |l| l.max(1).to_string());
            p.text(1080.0, base, size, col, Align::Center, &level);
        }
        if rows.is_empty() {
            p.text(
                640.0,
                400.0,
                22.0,
                DIM,
                Align::Center,
                "The engine's results couldn't be read.",
            );
        }
        p.hints(&[(Input::A, "Continue")]);
    }

    fn draw_toasts(&self, p: &mut Pen) {
        for (k, (text, _)) in self.toasts.iter().rev().enumerate() {
            let y = 600.0 - k as f32 * 46.0;
            let w = p.measure(20.0, text) + 48.0;
            p.fill(
                640.0 - w / 2.0,
                y - 30.0,
                w,
                40.0,
                Color::rgb(0x0A_1830).alpha(235),
            );
            p.outline(640.0 - w / 2.0, y - 30.0, w, 40.0, 2.0, SEL_EDGE);
            p.text(640.0, y - 3.0, 20.0, WHITE, Align::Center, text);
        }
    }
}

/// The first row to show so that row `sel` of `len` is in view, `rows` at a
/// time.
fn first_row(sel: usize, len: usize, rows: usize) -> usize {
    if sel < rows {
        0
    } else {
        (sel + 1 - rows).min(len.saturating_sub(rows))
    }
}

const BG_TOP: Color = Color::rgb(0x0B_2246);
const BG_BOTTOM: Color = Color::rgb(0x02_0814);
const PANEL: Color = Color::rgb(0x10_2A52).alpha(200);
const EDGE: Color = Color::rgb(0x4F_86C6);
const SEL: Color = Color::rgb(0x2F_6DB5);
const SEL_EDGE: Color = Color::rgb(0x9F_D0FF);
const WHITE: Color = Color::rgb(0xFF_FFFF);
const TEXT: Color = Color::rgb(0xCF_E3FA);
const DIM: Color = Color::rgb(0x86_A8D0);
const HEAD: Color = Color::rgb(0x9F_D0FF);
const GOLD: Color = Color::rgb(0xF2_C14E);
const GOOD: Color = Color::rgb(0x7B_E07B);
const WARN: Color = Color::rgb(0xFF_C04A);
const ERROR: Color = Color::rgb(0xFF_6B5B);

/// Draws in layout units (a 1280 by 720 screen, scaled to the window and
/// centred in it) and keeps where clicks go.
struct Pen<'a> {
    c: &'a mut Canvas,
    t: &'a mut Text,
    s: f32,
    ox: f32,
    oy: f32,
    areas: &'a mut Vec<Area>,
    top: bool,
}

impl Pen<'_> {
    fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, col: Color) {
        let s = self.s;
        self.c
            .fill(self.ox + x * s, self.oy + y * s, w * s, h * s, col);
    }

    fn outline(&mut self, x: f32, y: f32, w: f32, h: f32, t: f32, col: Color) {
        let s = self.s;
        let t = (t * s).max(1.0);
        self.c
            .outline(self.ox + x * s, self.oy + y * s, w * s, h * s, t, col);
    }

    fn circle(&mut self, x: f32, y: f32, r: f32, col: Color) {
        let s = self.s;
        self.c.circle(self.ox + x * s, self.oy + y * s, r * s, col);
    }

    /// Text with its baseline at `y`; returns its width.
    fn text(&mut self, x: f32, y: f32, size: f32, col: Color, align: Align, s: &str) -> f32 {
        let k = self.s;
        let w = self.c.text(
            self.t,
            Style::for_size(size),
            self.ox + x * k,
            self.oy + y * k,
            size * k,
            col,
            align,
            s,
        );
        w / k
    }

    fn measure(&mut self, size: f32, s: &str) -> f32 {
        self.t.measure(Style::for_size(size), size * self.s, s) / self.s
    }

    fn fit(&mut self, size: f32, width: f32, s: &str) -> String {
        self.t
            .fit(Style::for_size(size), size * self.s, width * self.s, s)
    }

    /// `s` broken into lines no wider than `width`.
    fn wrap(&mut self, size: f32, width: f32, s: &str) -> Vec<String> {
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in s.split_whitespace() {
            let next = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && self.measure(size, &next) > width {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = next;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }

    fn area(&mut self, x: f32, y: f32, w: f32, h: f32, hit: Hit) {
        self.areas.push(Area {
            x,
            y,
            w,
            h,
            hit,
            top: self.top,
        });
    }

    fn panel(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.fill(x, y, w, h, PANEL);
        self.outline(x, y, w, h, 2.0, EDGE.alpha(200));
    }

    /// A list row, lit up when selected.
    fn row(&mut self, x: f32, y: f32, w: f32, h: f32, on: bool, hit: Hit) {
        if on {
            self.fill(x, y, w, h, SEL.alpha(230));
            self.outline(x, y, w, h, 2.0, SEL_EDGE);
        }
        self.area(x, y, w, h, hit);
    }

    /// Dots going round.
    fn spinner(&mut self, x: f32, y: f32, t: f32) {
        for i in 0..8 {
            let a = i as f32 / 8.0 * std::f32::consts::TAU;
            let lag = (t * 8.0 - i as f32).rem_euclid(8.0);
            let alpha = (255.0 - lag * 28.0).clamp(40.0, 255.0) as u8;
            self.circle(
                x + a.cos() * 36.0,
                y + a.sin() * 36.0,
                6.0,
                SEL_EDGE.alpha(alpha),
            );
        }
    }

    /// A button's round face with its letter.
    fn button(&mut self, x: f32, y: f32, i: Input) {
        let (letter, col) = match i {
            Input::A => ("A", 0x5D_BB3A),
            Input::B => ("B", 0xD8_412F),
            Input::X => ("X", 0x2F_7FD8),
            Input::Y => ("Y", 0xE0_B81F),
            _ => ("", 0x80_8080),
        };
        self.circle(x, y, 14.0, Color::rgb(col));
        self.text(x, y + 6.5, 18.0, WHITE, Align::Center, letter);
    }

    /// Button hints along the bottom, from the left.
    fn hints(&mut self, hints: &[(Input, &str)]) {
        self.hint_row(hints, 60.0, 680.0);
    }

    fn hint_row(&mut self, hints: &[(Input, &str)], mut x: f32, y: f32) {
        for &(i, label) in hints {
            self.button(x + 14.0, y - 7.0, i);
            let w = self.text(x + 36.0, y, 20.0, TEXT, Align::Left, label);
            self.area(x, y - 24.0, w + 40.0, 34.0, Hit::Press(i));
            x += w + 70.0;
        }
    }

    fn hints_width(&mut self, hints: &[(Input, &str)]) -> f32 {
        let w: f32 = hints
            .iter()
            .map(|&(_, label)| self.measure(20.0, label) + 70.0)
            .sum();
        w - 30.0
    }

    /// A box over the screen, which alone takes clicks while it is up.
    fn popup(&mut self, title: &str, body: &str, hints: &[(Input, &str)]) {
        self.top = true;
        let (w, h) = (self.c.w as f32, self.c.h as f32);
        self.c.fill(0.0, 0.0, w, h, Color::rgb(0).alpha(150));
        self.fill(320.0, 230.0, 640.0, 260.0, Color::rgb(0x08_1A3C));
        self.outline(320.0, 230.0, 640.0, 260.0, 2.0, EDGE);
        self.text(640.0, 290.0, 30.0, WHITE, Align::Center, title);
        let lines = self.wrap(22.0, 560.0, body);
        for (k, l) in lines.iter().enumerate() {
            self.text(640.0, 340.0 + k as f32 * 30.0, 22.0, TEXT, Align::Center, l);
        }
        let width = self.hints_width(hints);
        self.hint_row(hints, 640.0 - width / 2.0, 462.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn app(folder: &std::path::Path) -> App {
        App::new(Config {
            folder: folder.to_path_buf(),
            exe: PathBuf::from("h2launch"),
            kind: Kind::Fake,
            maps: Vec::new(),
            server: None,
            instance: None,
            log: Arc::new(|_: &str| {}),
        })
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("h2lobby-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rows_scroll_to_keep_the_selection_in_view() {
        assert_eq!(first_row(0, 20, 8), 0);
        assert_eq!(first_row(7, 20, 8), 0);
        assert_eq!(first_row(8, 20, 8), 1);
        assert_eq!(first_row(19, 20, 8), 12);
        assert_eq!(first_row(3, 2, 8), 0);
    }

    #[test]
    fn the_sign_in_form_takes_a_gamertag_and_a_server() {
        let dir = scratch("form");
        let mut a = app(&dir);
        assert_eq!(a.label(), "signin");
        assert_eq!(a.fields[1], settings::DEFAULT_SERVER);
        for c in "master chief!x".chars() {
            a.input(Input::Char(c));
        }
        // The cleaned name, cut at 15 characters; letters don't press buttons here.
        assert_eq!(a.fields[0], "MASTER CHIEFX");
        a.input(Input::Backspace);
        a.input(Input::Tab);
        for _ in 0..settings::DEFAULT_SERVER.len() {
            a.input(Input::Backspace);
        }
        // Nothing to sign in to.
        a.input(Input::A);
        assert_eq!(a.label(), "signin");
        assert_eq!(a.field, 1);
        assert!(a.form_error.is_some());
        // Escape asks before quitting.
        a.input(Input::B);
        assert_eq!(a.label(), "quit");
        a.input(Input::Char('b'));
        assert_eq!(a.label(), "signin");
        assert!(!a.quitting());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_saved_gamertag_signs_in_at_once() {
        let dir = scratch("saved");
        Settings {
            // Nothing listens on port 9 here, so the dial fails quickly.
            server: "127.0.0.1:9".into(),
            gamertag: "ALPHA".into(),
        }
        .save(&dir.join("lobby.txt"))
        .unwrap();
        let mut a = app(&dir);
        assert_eq!(a.label(), "connecting");
        let until = Instant::now() + Duration::from_secs(20);
        while a.label() == "connecting" && Instant::now() < until {
            a.tick(Instant::now());
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(a.label(), "failed");
        // Y goes back to the form with what was typed.
        a.input(Input::Char('y'));
        assert_eq!(a.label(), "signin");
        assert_eq!(a.fields, ["ALPHA".to_string(), "127.0.0.1:9".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_custom_game_screen_picks_a_game_and_a_map() {
        let dir = scratch("custom");
        let mut a = app(&dir);
        a.cfg.maps = vec![("midship".into(), 1), ("lockout".into(), 2)];
        a.open_custom();
        assert_eq!(a.label(), "custom");
        // Lockout first, Slayer first.
        assert_eq!((a.crow, a.cgame, a.cmap), (0, 0, 1));
        a.input(Input::Left);
        assert_eq!(a.cgame, names::CUSTOM_GAMES.len() - 1);
        a.input(Input::Right);
        a.input(Input::Right);
        assert_eq!(a.cgame, 1);
        a.input(Input::Down);
        a.input(Input::Right);
        assert_eq!(a.cmap, 0);
        a.input(Input::Down);
        a.input(Input::Down);
        assert_eq!(a.crow, CUSTOM_ROWS - 1);
        // Asked once, until the match or a notice comes.
        a.input(Input::A);
        assert!(a.asked.is_some());
        a.tick(a.now + CUSTOM_WAIT);
        assert!(a.asked.is_none());
        assert_eq!(a.label(), "custom");
        a.input(Input::B);
        assert_eq!(a.label(), "live");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn screens_draw_and_clicks_land_on_buttons() {
        let Ok(mut text) = Text::system() else {
            // No font on this system: nothing to draw with.
            return;
        };
        let dir = scratch("draw");
        let mut a = app(&dir);
        let mut c = Canvas::new(640, 480);
        a.draw(&mut c, &mut text);
        assert!(c.px.contains(&0xFF_FFFF), "the title is drawn");
        // The B hint ("Quit") sits second in the row along the bottom.
        let b = a
            .areas
            .iter()
            .find(|ar| ar.hit == Hit::Press(Input::B))
            .copied()
            .unwrap();
        let (s, ox, oy) = a.view;
        a.click(ox + (b.x + 5.0) * s, oy + (b.y + 5.0) * s);
        assert_eq!(a.label(), "quit");
        a.draw(&mut c, &mut text);
        // With the popup up, only its buttons take clicks.
        a.click(ox + 400.0 * s, oy + 270.0 * s);
        assert_eq!(a.label(), "quit");
        let stay = a
            .areas
            .iter()
            .find(|ar| ar.top && ar.hit == Hit::Press(Input::B))
            .copied()
            .unwrap();
        a.click(ox + (stay.x + 5.0) * s, oy + (stay.y + 5.0) * s);
        assert_eq!(a.label(), "signin");
        // The custom game screen's arrows move the choices on and back.
        a.cfg.maps = vec![("midship".into(), 1), ("lockout".into(), 2)];
        a.open_custom();
        a.draw(&mut c, &mut text);
        let arrow = |a: &App, row: usize, on: bool| {
            let hit = a.areas.iter().find(|ar| ar.hit == Hit::Step(row, on));
            hit.copied().unwrap()
        };
        let next_map = arrow(&a, 1, true);
        a.click(ox + (next_map.x + 5.0) * s, oy + (next_map.y + 5.0) * s);
        assert_eq!((a.crow, a.cmap), (1, 0));
        let back = arrow(&a, 0, false);
        a.click(ox + (back.x + 5.0) * s, oy + (back.y + 5.0) * s);
        assert_eq!((a.crow, a.cgame), (0, names::CUSTOM_GAMES.len() - 1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
