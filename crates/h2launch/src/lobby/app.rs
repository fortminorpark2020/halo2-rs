//! The lobby's screens and what happens on them: signing in, the party and
//! the playlists, searching, custom games, the pregame lobby, the match
//! (its engine runs in a second copy of the launcher, see `child`), the
//! carnage report, the friends list and players' service records.
//! Nothing here opens a window: `window`, or the headless loop in
//! `mod.rs`, gives it input and time and shows what it draws.

use super::canvas::{Align, Canvas, Color, Style, Text};
use super::child::{Kind, Match, Running, Said};
use super::names;
use super::ranks::{RankIcons, Size};
use super::settings::{self, Settings, GAMERTAG_LEN};
use crate::live::{self, Engine, Log, Told};
use crate::session::Session;
use h2live::client::{LiveClient, LiveEvent, Profile, View};
use h2net::live::{
    Activity, Friend, LauncherMatch, LauncherPlayerResult, MatchOver, Online, OnlinePlayer,
    PartyInfo, PartyMember, Privacy, Relation, ServiceRecord, Stage, ToServer, CUSTOM_GAME,
    GAMERTAG_TAKEN, MAX_FRIENDS, QUICKMATCH,
};
use h2net::Connection;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
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
/// How many service records the lobby keeps.
const RECORDS_KEPT: usize = 32;
/// A service record younger than this is shown without asking again.
const RECORD_FRESH: Duration = Duration::from_secs(60);
/// How long the lobby waits for a service record before saying the server
/// didn't answer.
const RECORD_WAIT: Duration = Duration::from_secs(5);
/// Rows a list shows at once on the friends, players, party and service
/// record screens.
const LIST_ROWS: usize = 9;

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
    /// Halo 2's level icons, if a Halo 2 Vista mainmenu.map was found
    /// (`ranks::load`); without them levels are drawn as numbers.
    pub ranks: Option<Arc<RankIcons>>,
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
    /// The bumpers (Page Up and Page Down, or Q and E where letters are
    /// buttons): LB is always a service record, RB the friends list.
    Lb,
    Rb,
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
    /// A player's service record (`App::record_of`).
    Record,
    /// The friends list and friend requests.
    Friends,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Popup {
    Quit,
    LeaveGame,
    /// The leader asked to remove this member from the party.
    Kick(u64),
    /// Typing a gamertag to send a friend request to.
    AddFriend,
    /// What to do with this friend (`App::osel` is the option picked).
    FriendOptions(u64),
    /// Asking before removing this friend.
    Unfriend(u64),
}

/// What the options popup offers for a friend, in the order shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FriendOption {
    Invite,
    Join,
    Record,
    Remove,
}

impl FriendOption {
    fn label(self) -> &'static str {
        match self {
            FriendOption::Invite => "Invite to party",
            FriendOption::Join => "Join party",
            FriendOption::Record => "Service record",
            FriendOption::Remove => "Remove friend",
        }
    }
}

/// A service record shown, or why there isn't one.
enum RecordView<'a> {
    Shown(&'a ServiceRecord),
    Loading,
    NoAnswer,
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
    /// The selected playlist (or the custom game row after them).
    sel: usize,
    /// The selected player online, and their account: the selection
    /// follows the player when a new ONLINE list reorders the rows.
    psel: usize,
    paccount: Option<u64>,
    /// The selected party member.
    msel: usize,
    /// The friends screen's selected row and its account, which the
    /// selection follows as `paccount` does; and the options popup's
    /// selected row and its option, which the selection follows when a
    /// new list adds or drops Invite or Join.
    fsel: usize,
    faccount: Option<u64>,
    osel: usize,
    opick: Option<FriendOption>,
    /// The gamertag typed in the add friend popup.
    friend_tag: String,
    /// The carnage report's selected row (in the order shown).
    csel: usize,
    /// The service record screen: whose, the screen it was opened from,
    /// and its first row shown.
    record_of: u64,
    record_back: Screen,
    record_first: usize,
    /// The service records that came, the newest last, with when.
    records: VecDeque<(ServiceRecord, Instant)>,
    /// The records asked for and not come yet, with when.
    record_asks: Vec<(u64, Instant)>,
    /// What was drawn in the last frame, for the headless script's `see`.
    drawn: Vec<String>,
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
            paccount: None,
            msel: 0,
            fsel: 0,
            faccount: None,
            osel: 0,
            opick: None,
            friend_tag: String::new(),
            csel: 0,
            record_of: 0,
            record_back: Screen::Live,
            record_first: 0,
            records: VecDeque::new(),
            record_asks: Vec::new(),
            drawn: Vec::new(),
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
                Popup::AddFriend => "addfriend",
                Popup::FriendOptions(_) => "friend",
                Popup::Unfriend(_) => "unfriend",
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
            Screen::Record => "record",
            Screen::Friends => "friends",
        }
        .into()
    }

    /// The text drawn in the last frame, each string as it was drawn.
    pub fn drawn(&self) -> &[String] {
        &self.drawn
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
        // Another server, maybe, with the same account numbers: nothing
        // kept from the last one counts.
        self.records.clear();
        self.record_asks.clear();
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
                // A friend notice isn't the answer to the custom game asked
                // for.
                if !h2net::live::is_friend_notice(&text) {
                    self.asked = None;
                }
                self.toast(text);
            }
            LiveEvent::LauncherMatch(m) => self.on_match(m),
            LiveEvent::MatchOver(over) => self.on_match_over(over),
            LiveEvent::ServiceRecord(r) => self.keep_record(r),
            // Selections find their player again at the end of the tick.
            LiveEvent::Friends => {}
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
        let party_screens = [
            Screen::Live,
            Screen::Players,
            Screen::Party,
            Screen::Friends,
            Screen::Record,
        ];
        if !party_screens.contains(&self.screen) || self.popup.is_some() {
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

    /// The friends list in the order the lobby shows it: requests to us,
    /// friends on the launcher, friends on the game, friends offline, then
    /// requests we sent; by gamertag (ignoring case) within each.
    fn friend_rows(&self) -> Vec<&Friend> {
        let Some(v) = self.view() else {
            return Vec::new();
        };
        let group = |f: &Friend| match (f.relation, f.online) {
            (Relation::AskedUs, _) => 0,
            (Relation::Friend, Online::Launcher) => 1,
            (Relation::Friend, Online::Game) => 2,
            (Relation::Friend, Online::Offline) => 3,
            (Relation::WeAsked, _) => 4,
        };
        let mut rows: Vec<&Friend> = v.friends.iter().collect();
        rows.sort_by_cached_key(|f| (group(f), f.gamertag.to_ascii_lowercase(), f.account));
        rows
    }

    /// `account`'s entry on our friends list (a friend, or a request
    /// either way).
    fn friend(&self, account: u64) -> Option<&Friend> {
        self.view()?.friends.iter().find(|f| f.account == account)
    }

    /// A friend request to `account` makes sense: it isn't us, and they
    /// aren't on our list already (as a friend, or a request either way).
    fn can_ask(&self, account: u64) -> bool {
        Some(account) != self.account() && self.friend(account).is_none()
    }

    /// Send a friend request to `gamertag` (it's cleaned first; nothing
    /// goes for an empty one).
    fn ask_friend(&mut self, gamertag: &str) {
        let tag = settings::clean_gamertag(gamertag);
        if tag.is_empty() {
            return;
        }
        self.log(&format!("lobby: sending {tag} a friend request"));
        self.send(ToServer::FriendRequest(tag));
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

    // --------------------------------------------------- service records

    /// A service record that came: kept (the newest `RECORDS_KEPT`), and
    /// no longer waited for.
    fn keep_record(&mut self, r: ServiceRecord) {
        self.record_asks.retain(|(a, _)| *a != r.account);
        self.records.retain(|(old, _)| old.account != r.account);
        if self.records.len() == RECORDS_KEPT {
            self.records.pop_front();
        }
        self.records.push_back((r, self.now));
    }

    /// The record kept for `account`, and when it came.
    fn cached_record(&self, account: u64) -> Option<&(ServiceRecord, Instant)> {
        self.records.iter().find(|(r, _)| r.account == account)
    }

    /// Show `account`'s service record: the one kept if it's fresh;
    /// otherwise ask the server (once while waiting), showing the one kept
    /// meanwhile.
    fn open_record(&mut self, account: u64) {
        if self.screen != Screen::Record {
            self.record_back = self.screen;
        }
        self.record_of = account;
        self.record_first = 0;
        self.screen = Screen::Record;
        self.ask_record(account);
    }

    fn ask_record(&mut self, account: u64) {
        let now = self.now;
        let fresh = self
            .cached_record(account)
            .is_some_and(|(_, at)| now.saturating_duration_since(*at) < RECORD_FRESH);
        let waiting = self
            .record_asks
            .iter()
            .any(|(a, at)| *a == account && now.saturating_duration_since(*at) < RECORD_WAIT);
        if fresh || waiting {
            return;
        }
        self.record_asks.retain(|(a, _)| *a != account);
        self.record_asks.push((account, now));
        self.send(ToServer::Record(account));
    }

    /// What the service record screen shows.
    fn record_view(&self) -> RecordView<'_> {
        if let Some((r, _)) = self.cached_record(self.record_of) {
            return RecordView::Shown(r);
        }
        if self.record_asks.iter().any(|(a, at)| {
            *a == self.record_of && self.now.saturating_duration_since(*at) < RECORD_WAIT
        }) {
            RecordView::Loading
        } else {
            RecordView::NoAnswer
        }
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
        // The next look at anyone in the match shows the new numbers.
        let mut played: Vec<u64> = over.players.iter().map(|&(a, _)| a).collect();
        if let Some(g) = self.game.as_ref().filter(|g| g.m.id == over.id) {
            played.extend(g.m.players.iter().map(|p| p.account));
        }
        self.records.retain(|(r, _)| !played.contains(&r.account));
        // A record on screen that was dropped is asked for again, rather
        // than saying the server didn't answer.
        if self.screen == Screen::Record && played.contains(&self.record_of) {
            self.ask_record(self.record_of);
        }
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
                // Start on this PC's player.
                let me = g.m.relay.id;
                self.csel = carnage_rows(g)
                    .iter()
                    .position(|r| r.relay_id == me)
                    .unwrap_or(0);
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
        self.follow();
        self.record_asks
            .retain(|(_, at)| now.saturating_duration_since(*at) < RECORD_WAIT);
    }

    /// Keep the selections on the friends and players screens on the
    /// player picked, when a new list moves them to another row (the same
    /// row number, clamped, if they've gone), and close a popup naming a
    /// friend who has left the list.
    fn follow(&mut self) {
        let rows: Vec<u64> = self.friend_rows().iter().map(|f| f.account).collect();
        self.fsel = follow(&rows, self.faccount, self.fsel);
        self.faccount = rows.get(self.fsel).copied();
        let rows: Vec<u64> = self.others().iter().map(|o| o.account).collect();
        self.psel = follow(&rows, self.paccount, self.psel);
        self.paccount = rows.get(self.psel).copied();
        if let Some(Popup::FriendOptions(a) | Popup::Unfriend(a)) = self.popup {
            if self
                .friend(a)
                .is_none_or(|f| f.relation != Relation::Friend)
            {
                self.popup = None;
            }
        }
        // The option picked, where it is now (the same row, clamped, if
        // it's gone).
        if let Some(Popup::FriendOptions(a)) = self.popup {
            let options = self.friend_options(a);
            let at = self
                .opick
                .and_then(|o| options.iter().position(|&p| p == o));
            self.pick_option(a, at.unwrap_or(self.osel));
        }
    }

    /// Select row `i` of friend `who`'s options popup (clamped).
    fn pick_option(&mut self, who: u64, i: usize) {
        let options = self.friend_options(who);
        self.osel = i.min(options.len().saturating_sub(1));
        self.opick = options.get(self.osel).copied();
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
        let typing = (self.screen == Screen::SignIn && self.popup.is_none())
            || self.popup == Some(Popup::AddFriend);
        let i = match i {
            Input::Char(c) if !typing => match c.to_ascii_lowercase() {
                'a' | ' ' => Input::A,
                'b' => Input::B,
                'x' => Input::X,
                'y' => Input::Y,
                'q' => Input::Lb,
                'e' => Input::Rb,
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
            Screen::Carnage => self.carnage_input(i),
            Screen::Record => self.record_input(i),
            Screen::Friends => self.friends_input(i),
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
            (Popup::AddFriend, Input::A) => {
                self.popup = None;
                let tag = std::mem::take(&mut self.friend_tag);
                self.ask_friend(&tag);
            }
            (Popup::AddFriend, Input::Backspace) => {
                self.friend_tag.pop();
            }
            (Popup::AddFriend, Input::Char(c)) => {
                let full = self.friend_tag.chars().count() >= GAMERTAG_LEN;
                if (c.is_ascii_alphanumeric() || c == ' ') && !full {
                    self.friend_tag.push(c.to_ascii_uppercase());
                }
            }
            (Popup::FriendOptions(who), _) => self.options_input(who, i),
            (Popup::Unfriend(who), Input::A) => {
                self.popup = None;
                self.log(&format!("lobby: removing {who:#x} from the friends list"));
                self.send(ToServer::FriendRemove(who));
            }
            (_, Input::B) => self.popup = None,
            _ => {}
        }
    }

    /// What the options popup offers for friend `who`, in order: only
    /// what applies, with Remove friend always last.
    fn friend_options(&self, who: u64) -> Vec<FriendOption> {
        let Some(f) = self.friend(who) else {
            return Vec::new();
        };
        let ours = self.party().map(|p| p.id);
        let mut out = Vec::new();
        if f.online == Online::Launcher && ours != Some(f.party) {
            out.push(FriendOption::Invite);
        }
        if f.joinable {
            out.push(FriendOption::Join);
        }
        out.push(FriendOption::Record);
        out.push(FriendOption::Remove);
        out
    }

    fn options_input(&mut self, who: u64, i: Input) {
        let options = self.friend_options(who);
        let n = options.len().max(1);
        match i {
            // Up from the first goes to the last, and down from the last
            // to the first.
            Input::Up => self.pick_option(who, (self.osel + n - 1) % n),
            Input::Down => self.pick_option(who, (self.osel + 1) % n),
            Input::B => self.popup = None,
            Input::A => {
                let Some(&pick) = options.get(self.osel) else {
                    return;
                };
                let Some(f) = self.friend(who).cloned() else {
                    self.popup = None;
                    return;
                };
                self.popup = None;
                match pick {
                    FriendOption::Invite => {
                        self.send(ToServer::Invite(who));
                        self.toast(format!("Invited {} to your party.", f.gamertag));
                    }
                    FriendOption::Join => {
                        self.log(&format!("lobby: joining {}'s party", f.gamertag));
                        self.send(ToServer::JoinParty(f.party));
                        self.screen = Screen::Live;
                    }
                    FriendOption::Record => self.open_record(who),
                    FriendOption::Remove => self.popup = Some(Popup::Unfriend(who)),
                }
            }
            _ => {}
        }
    }

    fn carnage_input(&mut self, i: Input) {
        let rows: Vec<LauncherPlayerResult> = self
            .game
            .as_ref()
            .map(|g| carnage_rows(g).into_iter().copied().collect())
            .unwrap_or_default();
        let picked = rows.get(self.csel).and_then(|r| {
            let g = self.game.as_ref()?;
            g.m.players
                .iter()
                .find(|p| p.relay_id == r.relay_id)
                .cloned()
        });
        match i {
            Input::Up => self.csel = self.csel.saturating_sub(1),
            Input::Down => self.csel = (self.csel + 1).min(rows.len().saturating_sub(1)),
            Input::Lb => {
                if let Some(p) = picked {
                    self.open_record(p.account);
                }
            }
            Input::Y => {
                if let Some(p) = picked.filter(|p| self.can_ask(p.account)) {
                    self.ask_friend(&p.gamertag);
                }
            }
            Input::A | Input::B => {
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
            _ => {}
        }
    }

    fn record_input(&mut self, i: Input) {
        let rows = match self.record_view() {
            RecordView::Shown(r) => r.found.as_ref().map_or(0, |f| f.playlists.len()),
            _ => 0,
        };
        let most = rows.saturating_sub(LIST_ROWS);
        match i {
            Input::Up => self.record_first = self.record_first.saturating_sub(1),
            Input::Down => self.record_first = (self.record_first + 1).min(most),
            Input::A if matches!(self.record_view(), RecordView::NoAnswer) => {
                let who = self.record_of;
                self.ask_record(who);
            }
            Input::B => {
                self.screen = match self.record_back {
                    // The game it was opened from may be over.
                    Screen::Carnage if self.game.is_none() => Screen::Live,
                    back => back,
                }
            }
            _ => {}
        }
    }

    /// Select row `k` of the friends screen.
    fn select_friend(&mut self, k: usize) {
        let rows: Vec<u64> = self.friend_rows().iter().map(|f| f.account).collect();
        self.fsel = k.min(rows.len().saturating_sub(1));
        self.faccount = rows.get(self.fsel).copied();
    }

    fn friends_input(&mut self, i: Input) {
        let picked = self.friend_rows().get(self.fsel).map(|f| (*f).clone());
        let n = self.friend_rows().len();
        match (i, picked) {
            (Input::Up, _) => self.select_friend(self.fsel.saturating_sub(1)),
            (Input::Down, _) => self.select_friend((self.fsel + 1).min(n.saturating_sub(1))),
            (Input::Y, _) => {
                self.friend_tag.clear();
                self.popup = Some(Popup::AddFriend);
            }
            (Input::B, _) => self.screen = Screen::Live,
            (Input::Lb, Some(f)) => self.open_record(f.account),
            (Input::A, Some(f)) if f.relation == Relation::AskedUs => {
                self.log(&format!("lobby: accepting {}'s friend request", f.gamertag));
                self.send(ToServer::FriendAccept(f.account));
            }
            (Input::X, Some(f)) if f.relation == Relation::AskedUs => {
                self.log(&format!("lobby: declining {}'s friend request", f.gamertag));
                self.send(ToServer::FriendDecline(f.account));
            }
            (Input::A, Some(f)) if f.relation == Relation::Friend => {
                let who = f.account;
                self.pick_option(who, 0);
                self.popup = Some(Popup::FriendOptions(who));
            }
            (Input::X, Some(f)) if f.relation == Relation::Friend && f.joinable => {
                self.log(&format!("lobby: joining {}'s party", f.gamertag));
                self.send(ToServer::JoinParty(f.party));
                self.screen = Screen::Live;
            }
            (Input::X, Some(f)) if f.relation == Relation::WeAsked => {
                self.log(&format!(
                    "lobby: taking back the friend request to {}",
                    f.gamertag
                ));
                self.send(ToServer::FriendRemove(f.account));
            }
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
        // The service record and the friends list, searching or not (the
        // search goes on).
        match i {
            Input::Lb => {
                if let Some(me) = self.account() {
                    self.open_record(me);
                }
                return;
            }
            Input::Rb => {
                self.screen = Screen::Friends;
                return;
            }
            _ => {}
        }
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
                self.select_player(0);
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
            (Input::Lb, Some((who, _))) => self.open_record(who),
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

    /// Select row `k` of the players screen.
    fn select_player(&mut self, k: usize) {
        let rows: Vec<u64> = self.others().iter().map(|o| o.account).collect();
        self.psel = k.min(rows.len().saturating_sub(1));
        self.paccount = rows.get(self.psel).copied();
    }

    fn players_input(&mut self, i: Input) {
        let others: Vec<OnlinePlayer> = self.others().into_iter().cloned().collect();
        let picked = others.get(self.psel);
        match i {
            Input::Up => self.select_player(self.psel.saturating_sub(1)),
            Input::Down => self.select_player((self.psel + 1).min(others.len().saturating_sub(1))),
            Input::Y => {
                if let Some(p) = picked.filter(|p| self.can_ask(p.account)) {
                    self.ask_friend(&p.gamertag);
                }
            }
            Input::Lb => {
                if let Some(p) = picked {
                    self.open_record(p.account);
                }
            }
            Input::Rb => self.screen = Screen::Friends,
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
            Some(Hit::Row(r)) if matches!(self.popup, Some(Popup::FriendOptions(_))) => {
                if self.osel == r {
                    self.input(Input::A);
                } else if let Some(Popup::FriendOptions(who)) = self.popup {
                    self.pick_option(who, r);
                }
            }
            Some(Hit::Row(r)) if self.screen == Screen::Friends => {
                if self.fsel == r {
                    self.input(Input::A);
                } else {
                    self.select_friend(r);
                }
            }
            Some(Hit::Row(r)) if self.screen == Screen::Players => {
                if self.psel == r {
                    self.input(Input::A);
                } else {
                    self.select_player(r);
                }
            }
            // A row of the carnage report is only selected: A continues.
            Some(Hit::Row(r)) if self.screen == Screen::Carnage => self.csel = r,
            Some(Hit::Row(r)) if self.screen == Screen::Custom => {
                if self.crow == r && r == CUSTOM_ROWS - 1 {
                    self.input(Input::A);
                } else {
                    self.crow = r;
                }
            }
            Some(Hit::Row(r)) => {
                let sel = match self.screen {
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

    /// Select `gamertag`'s row (ignoring case) on the players, friends,
    /// party or carnage screen, for the headless script's `pick`; false if
    /// there is none.
    pub fn pick(&mut self, gamertag: &str) -> bool {
        let is = |t: &str| t.eq_ignore_ascii_case(gamertag.trim());
        match self.screen {
            Screen::Players => {
                let at = self.others().iter().position(|o| is(&o.gamertag));
                at.map(|k| self.select_player(k)).is_some()
            }
            Screen::Friends => {
                let at = self.friend_rows().iter().position(|f| is(&f.gamertag));
                at.map(|k| self.select_friend(k)).is_some()
            }
            Screen::Party => {
                let at = self
                    .party()
                    .and_then(|p| p.members.iter().position(|m| is(&m.gamertag)));
                at.map(|k| self.msel = k).is_some()
            }
            Screen::Carnage => {
                let at = self.game.as_ref().and_then(|g| {
                    carnage_rows(g).iter().position(|r| {
                        g.m.players
                            .iter()
                            .any(|p| p.relay_id == r.relay_id && is(&p.gamertag))
                    })
                });
                at.map(|k| self.csel = k).is_some()
            }
            _ => false,
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
        let mut drawn = Vec::new();
        let ranks = self.cfg.ranks.clone();
        let mut p = Pen {
            c,
            t,
            s,
            ox,
            oy,
            areas: &mut areas,
            top: false,
            ranks: ranks.as_deref(),
            drawn: &mut drawn,
        };
        self.draw_screen(&mut p);
        self.areas = areas;
        self.drawn = drawn;
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
            Screen::Record => "SERVICE RECORD",
            Screen::Friends => "FRIENDS",
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
            Screen::Record => self.draw_record(p),
            Screen::Friends => self.draw_friends(p),
        }
        p.text(
            1220.0,
            712.0,
            14.5,
            DIM,
            Align::Right,
            "Keyboard: Enter = A, Esc = B, X, Y, Q = LB, E = RB, arrows",
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
                Popup::AddFriend => self.draw_add_friend(p),
                Popup::FriendOptions(who) => self.draw_friend_options(p, who),
                Popup::Unfriend(who) => {
                    let name = self
                        .friend(who)
                        .map_or("this player".into(), |f| f.gamertag.clone());
                    p.popup(
                        "REMOVE FRIEND",
                        &format!("Remove {name} from your friends? They won't be told."),
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
        let wd = p.text(1220.0, 44.0, 24.0, WHITE, Align::Right, &w.gamertag);
        // The highest level's icon left of the gamertag (the line below
        // has the number).
        if p.has_icons() {
            let x = 1220.0 - wd - 10.0;
            p.level(
                x,
                44.0,
                24.0,
                WHITE,
                Align::Right,
                w.best.max(1),
                Size::Small,
            );
        }
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
        // The RB hint counts the friend requests waiting for an answer.
        let asking = v
            .friends
            .iter()
            .filter(|f| f.relation == Relation::AskedUs)
            .count();
        let friends = if asking > 0 {
            format!("Friends ({asking})")
        } else {
            "Friends".into()
        };
        if self.searching() {
            self.draw_search(p, v);
            let mut hints = Vec::new();
            if self.leader() {
                hints.push((Input::B, "Stop searching"));
            }
            hints.push((Input::Rb, friends.as_str()));
            hints.push((Input::Lb, "Service record"));
            p.hints(&hints);
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
            hints.push((Input::Rb, friends.as_str()));
            hints.push((Input::Lb, "Service record"));
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
            // Level 0 draws "-": unranked.
            let level = if pl.ranked { pl.level.max(1) } else { 0 };
            p.level(
                560.0,
                y + 32.0,
                22.0,
                col,
                Align::Center,
                level,
                Size::Small,
            );
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
            p.level(
                1196.0,
                y + 32.0,
                22.0,
                TEXT,
                Align::Right,
                level,
                Size::Small,
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
            p.level(
                560.0,
                y + 32.0,
                22.0,
                col,
                Align::Center,
                level,
                Size::Small,
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
        if self.picked_member().is_some() {
            hints.push((Input::Lb, "Service record"));
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
            let level = o.best.max(1);
            p.level(
                560.0,
                y + 32.0,
                22.0,
                col,
                Align::Center,
                level,
                Size::Small,
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
            if self.can_ask(o.account) {
                hints.push((Input::Y, "Add friend"));
            }
            hints.push((Input::Lb, "Service record"));
        }
        hints.push((Input::Rb, "Friends"));
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
            let level = pl.level.max(1);
            p.level(
                x + w - 16.0,
                base,
                size,
                TEXT,
                Align::Right,
                level,
                Size::Big,
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
        // Halo 2 Xbox's columns.
        let heads = [
            (100.0, Align::Left, "PLACE"),
            (200.0, Align::Left, "PLAYER"),
            (700.0, Align::Center, "SCORE"),
            (790.0, Align::Center, "KILLS"),
            (880.0, Align::Center, "ASSISTS"),
            (970.0, Align::Center, "DEATHS"),
            (1090.0, Align::Center, "LEVEL"),
        ];
        for (x, a, h) in heads {
            p.text(x, 228.0, 16.0, HEAD, a, h);
        }
        let rows = carnage_rows(g);
        let h = (380.0 / rows.len().max(1) as f32).min(44.0);
        for (k, r) in rows.iter().enumerate() {
            let y = 242.0 + k as f32 * h;
            let pl = g.m.players.iter().find(|pl| pl.relay_id == r.relay_id);
            let me = r.relay_id == g.m.relay.id;
            let on = k == self.csel;
            if me && !on {
                p.fill(72.0, y, 1136.0, h - 4.0, SEL.alpha(110));
            }
            p.row(72.0, y, 1136.0, h - 4.0, on, Hit::Row(k));
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
            let mut name = p.fit(size, 440.0, name);
            if r.left {
                name += "  (quit)";
            }
            let col = if me || on { WHITE } else { TEXT };
            p.text(200.0, base, size, col, Align::Left, &name);
            for (x, n) in [
                (700.0, r.score.to_string()),
                (790.0, r.kills.to_string()),
                (880.0, r.assists.to_string()),
                (970.0, r.deaths.to_string()),
            ] {
                p.text(x, base, size, col, Align::Center, &n);
            }
            let level = pl.map(|pl| {
                g.over
                    .as_ref()
                    .and_then(|o| o.players.iter().find(|(a, _)| *a == pl.account))
                    .map_or(pl.level, |&(_, l)| l)
            });
            // An unknown player's level is 0, which draws "-".
            let level = level.map_or(0, |l| l.max(1));
            p.level(1090.0, base, size, col, Align::Center, level, Size::Big);
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
        let mut hints = vec![(Input::A, "Continue")];
        let picked = rows
            .get(self.csel)
            .and_then(|r| g.m.players.iter().find(|p| p.relay_id == r.relay_id));
        if let Some(pl) = picked {
            if self.can_ask(pl.account) {
                hints.push((Input::Y, "Add friend"));
            }
            hints.push((Input::Lb, "Service record"));
        }
        p.hints(&hints);
    }

    /// The service record screen: the player's totals on the left, and
    /// each ranked playlist they've played on the right.
    fn draw_record(&self, p: &mut Pen) {
        p.panel(60.0, 110.0, 380.0, 530.0);
        p.panel(460.0, 110.0, 760.0, 530.0);
        let found = match self.record_view() {
            RecordView::Shown(r) => match &r.found {
                Some(f) => f,
                None => {
                    p.text(840.0, 330.0, 22.0, TEXT, Align::Center, "No such player.");
                    p.hints(&[(Input::B, "Back")]);
                    return;
                }
            },
            RecordView::Loading => {
                let line = "Loading the service record.";
                p.text(840.0, 330.0, 22.0, TEXT, Align::Center, line);
                p.spinner(840.0, 420.0, self.clock.elapsed().as_secs_f32());
                p.hints(&[(Input::B, "Back")]);
                return;
            }
            RecordView::NoAnswer => {
                let line = "The server didn't answer.";
                p.text(840.0, 330.0, 22.0, TEXT, Align::Center, line);
                p.hints(&[(Input::A, "Try again"), (Input::B, "Back")]);
                return;
            }
        };
        let name = p.fit(32.0, 340.0, &found.gamertag);
        p.text(84.0, 170.0, 32.0, WHITE, Align::Left, &name);
        // The highest level's icon at three times its size, if there are
        // icons; without them the number alone sits there.
        let best = found.best.max(1);
        let x = if p.icon(84.0, 196.0, 84.0, 78.0, best, Size::Big) {
            184.0
        } else {
            84.0
        };
        p.text(x, 222.0, 16.0, HEAD, Align::Left, "HIGHEST LEVEL");
        p.text(x, 262.0, 36.0, WHITE, Align::Left, &best.to_string());
        let mut total = h2net::live::Tally::default();
        let (mut games, mut wins) = (0u64, 0u64);
        for pr in &found.playlists {
            games += u64::from(pr.games);
            wins += u64::from(pr.wins);
            let t = &mut total;
            t.kills = t.kills.saturating_add(pr.tally.kills);
            t.deaths = t.deaths.saturating_add(pr.tally.deaths);
            t.assists = t.assists.saturating_add(pr.tally.assists);
        }
        let totals = [
            ("Ranked games", games.to_string()),
            ("Wins", wins.to_string()),
            ("Kills", total.kills.to_string()),
            ("Deaths", total.deaths.to_string()),
            ("Assists", total.assists.to_string()),
            ("K/D", kd(total.kills, total.deaths)),
        ];
        for (k, (label, value)) in totals.iter().enumerate() {
            let y = 330.0 + k as f32 * 36.0;
            p.text(84.0, y, 20.0, DIM, Align::Left, label);
            p.text(416.0, y, 20.0, TEXT, Align::Right, value);
        }
        let since = format!("Member since {}", utc_date(found.created));
        p.text(84.0, 610.0, 16.0, DIM, Align::Left, &since);
        p.text(484.0, 142.0, 16.0, HEAD, Align::Left, "PLAYLIST");
        for (x, h) in [
            (740.0, "LEVEL"),
            (815.0, "GAMES"),
            (880.0, "WINS"),
            (945.0, "KILLS"),
            (1015.0, "DEATHS"),
            (1085.0, "ASSISTS"),
            (1160.0, "K/D"),
        ] {
            p.text(x, 142.0, 16.0, HEAD, Align::Center, h);
        }
        let first = self
            .record_first
            .min(found.playlists.len().saturating_sub(LIST_ROWS));
        for (k, pr) in found
            .playlists
            .iter()
            .enumerate()
            .skip(first)
            .take(LIST_ROWS)
        {
            let y = 156.0 + (k - first) as f32 * 50.0;
            if (k - first) % 2 == 1 {
                p.fill(472.0, y, 736.0, 46.0, Color::rgb(0x06_1430).alpha(90));
            }
            let base = y + 32.0;
            let name = match self.playlist_name(pr.playlist) {
                n if n.is_empty() => format!("Playlist {}", pr.playlist),
                n => n,
            };
            let name = p.fit(22.0, 220.0, &name);
            p.text(484.0, base, 22.0, TEXT, Align::Left, &name);
            let level = pr.level.max(1);
            p.level(740.0, base, 20.0, TEXT, Align::Center, level, Size::Small);
            let t = &pr.tally;
            for (x, n) in [
                (815.0, pr.games.to_string()),
                (880.0, pr.wins.to_string()),
                (945.0, t.kills.to_string()),
                (1015.0, t.deaths.to_string()),
                (1085.0, t.assists.to_string()),
                (1160.0, kd(t.kills, t.deaths)),
            ] {
                p.text(x, base, 20.0, TEXT, Align::Center, &n);
            }
        }
        if found.playlists.is_empty() {
            p.text(
                840.0,
                330.0,
                22.0,
                DIM,
                Align::Center,
                "No ranked games yet.",
            );
        }
        p.hints(&[(Input::B, "Back")]);
    }

    /// A friend's status on the friends screen, and its colour.
    fn friend_status(&self, f: &Friend) -> (String, Color) {
        let status = match (f.relation, f.online) {
            (Relation::AskedUs, _) => return ("Wants to be your friend".into(), GOLD),
            (Relation::WeAsked, _) => "Friend request sent".into(),
            (Relation::Friend, Online::Offline) => "Offline".into(),
            (Relation::Friend, Online::Game) => "Online (h2viewer)".into(),
            (Relation::Friend, Online::Launcher) => {
                // A friend who left a match their party still plays has
                // no map or game type: then the playlist alone.
                let known = !f.map.is_empty() && !f.variant.is_empty();
                let game = |what: String, alone: String| match known {
                    true => format!(
                        "{what}: {} on {}",
                        names::variant(&f.variant),
                        names::map(&f.map)
                    ),
                    false => alone,
                };
                match f.activity {
                    Activity::Lobby => "In a lobby".into(),
                    Activity::Searching if f.playlist == QUICKMATCH => {
                        "Searching (quickmatch)".into()
                    }
                    Activity::Searching => format!("Searching {}", self.playlist_name(f.playlist)),
                    // A launcher's custom game is a match on playlist 255.
                    Activity::Playing | Activity::Custom if f.playlist == CUSTOM_GAME => {
                        game("Custom game".into(), "In a custom game".into())
                    }
                    Activity::Playing | Activity::Custom => {
                        let name = self.playlist_name(f.playlist);
                        game(name.clone(), format!("Playing {name}"))
                    }
                }
            }
        };
        (status, DIM)
    }

    fn draw_friends(&self, p: &mut Pen) {
        p.panel(60.0, 110.0, 1160.0, 530.0);
        p.text(84.0, 142.0, 16.0, HEAD, Align::Left, "GAMERTAG");
        p.text(560.0, 142.0, 16.0, HEAD, Align::Center, "LEVEL");
        p.text(660.0, 142.0, 16.0, HEAD, Align::Left, "STATUS");
        let rows = self.friend_rows();
        // Friends and the requests we sent count towards the most.
        let count = rows
            .iter()
            .filter(|f| f.relation != Relation::AskedUs)
            .count();
        let of = format!("{count} of {MAX_FRIENDS}");
        p.text(1196.0, 142.0, 16.0, DIM, Align::Right, &of);
        let ours = self.party().map(|pt| pt.id);
        let first = first_row(self.fsel, rows.len(), LIST_ROWS);
        for (k, f) in rows.iter().enumerate().skip(first).take(LIST_ROWS) {
            let y = 156.0 + (k - first) as f32 * 50.0;
            let on = k == self.fsel;
            p.row(72.0, y, 1136.0, 46.0, on, Hit::Row(k));
            let col = if on { WHITE } else { TEXT };
            if f.relation == Relation::Friend {
                match f.online {
                    Online::Launcher => p.circle(90.0, y + 23.0, 6.0, GOOD),
                    Online::Game => p.circle(90.0, y + 23.0, 6.0, DIM),
                    Online::Offline => {}
                }
            }
            let name = p.fit(24.0, 400.0, &f.gamertag);
            p.text(108.0, y + 32.0, 24.0, col, Align::Left, &name);
            let level = f.best.max(1);
            p.level(
                560.0,
                y + 32.0,
                22.0,
                col,
                Align::Center,
                level,
                Size::Small,
            );
            let (status, scol) = self.friend_status(f);
            let status = p.fit(20.0, 380.0, &status);
            p.text(660.0, y + 32.0, 20.0, scol, Align::Left, &status);
            let open = self
                .view()
                .and_then(|v| v.online.iter().find(|o| o.account == f.account))
                .map(|o| o.open);
            let party = if f.relation != Relation::Friend || f.online != Online::Launcher {
                ""
            } else if ours == Some(f.party) {
                "Your party"
            } else if f.joinable {
                "Open party"
            } else if open == Some(false) {
                "Invite only"
            } else {
                ""
            };
            p.text(1196.0, y + 32.0, 20.0, DIM, Align::Right, party);
        }
        if rows.is_empty() {
            let line = "No friends yet. Press Y to send a friend request.";
            p.text(640.0, 320.0, 22.0, DIM, Align::Center, line);
        }
        let mut hints = Vec::new();
        match rows.get(self.fsel) {
            Some(f) if f.relation == Relation::AskedUs => {
                hints.push((Input::A, "Accept"));
                hints.push((Input::X, "Decline"));
            }
            Some(f) if f.relation == Relation::Friend => {
                hints.push((Input::A, "Options"));
                if f.joinable {
                    hints.push((Input::X, "Join party"));
                }
            }
            Some(_) => hints.push((Input::X, "Cancel request")),
            None => {}
        }
        if !rows.is_empty() {
            hints.push((Input::Lb, "Service record"));
        }
        hints.push((Input::Y, "Add friend"));
        hints.push((Input::B, "Back"));
        p.hints(&hints);
    }

    fn draw_add_friend(&self, p: &mut Pen) {
        p.popup_box(320.0, 230.0, 640.0, 260.0);
        p.text(
            640.0,
            278.0,
            30.0,
            WHITE,
            Align::Center,
            "SEND FRIEND REQUEST",
        );
        let line = "Type their gamertag on the keyboard.";
        p.text(640.0, 314.0, 20.0, TEXT, Align::Center, line);
        // Drawn like the sign-in screen's gamertag field.
        p.fill(380.0, 330.0, 520.0, 52.0, Color::rgb(0x06_1430).alpha(220));
        p.outline(380.0, 330.0, 520.0, 52.0, 2.0, SEL_EDGE);
        let shown = p.fit(26.0, 480.0, &self.friend_tag);
        let wd = p.text(396.0, 366.0, 26.0, WHITE, Align::Left, &shown);
        p.fill(398.0 + wd, 342.0, 2.0, 30.0, SEL_EDGE);
        let hints = [(Input::A, "Send"), (Input::B, "Cancel")];
        let width = p.hints_width(&hints);
        p.hint_row(&hints, 640.0 - width / 2.0, 462.0);
    }

    fn draw_friend_options(&self, p: &mut Pen, who: u64) {
        p.popup_box(320.0, 200.0, 640.0, 320.0);
        let name = self.friend(who).map_or("", |f| f.gamertag.as_str());
        p.text(640.0, 250.0, 30.0, WHITE, Align::Center, name);
        for (k, o) in self.friend_options(who).iter().enumerate() {
            let y = 280.0 + k as f32 * 44.0;
            let on = k == self.osel;
            p.row(360.0, y, 560.0, 40.0, on, Hit::Row(k));
            let col = if on { WHITE } else { TEXT };
            p.text(640.0, y + 28.0, 22.0, col, Align::Center, o.label());
        }
        let hints = [(Input::A, "Select"), (Input::B, "Back")];
        let width = p.hints_width(&hints);
        p.hint_row(&hints, 640.0 - width / 2.0, 492.0);
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

/// The carnage report's rows: the game's results by place, then team
/// (so a team game lists each team together in its finishing order), then
/// score, highest first.
fn carnage_rows(g: &Game) -> Vec<&LauncherPlayerResult> {
    let mut rows: Vec<&LauncherPlayerResult> = g.results.iter().flatten().collect();
    rows.sort_by_key(|r| (r.place, r.team, -r.score));
    rows
}

/// The row to select in a list of accounts `rows` that just came: the one
/// with `picked`'s account if it's still there, else `sel` kept in range.
fn follow(rows: &[u64], picked: Option<u64>, sel: usize) -> usize {
    picked
        .and_then(|a| rows.iter().position(|&r| r == a))
        .unwrap_or(sel)
        .min(rows.len().saturating_sub(1))
}

/// Kills per death with two decimals ("-" with neither).
fn kd(kills: u32, deaths: u32) -> String {
    if kills == 0 && deaths == 0 {
        return "-".into();
    }
    format!("{:.2}", f64::from(kills) / f64::from(deaths.max(1)))
}

/// The UTC date of Unix time `unix`, as 2026-10-10.
fn utc_date(unix: u64) -> String {
    // Days to a civil date (Howard Hinnant's method).
    let z = (unix / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
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

/// How wide a button's face is in a hint: 28 for the round ones, 34 for
/// the bumpers.
fn face_width(i: Input) -> f32 {
    match i {
        Input::Lb | Input::Rb => 34.0,
        _ => 28.0,
    }
}

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
    /// Halo 2's level icons, if they were found.
    ranks: Option<&'a RankIcons>,
    /// Every string drawn, for `App::drawn`.
    drawn: &'a mut Vec<String>,
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
        if !s.is_empty() {
            self.drawn.push(s.to_string());
        }
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

    /// There are level icons to draw.
    fn has_icons(&self) -> bool {
        self.ranks.is_some()
    }

    /// `level`'s icon scaled into the rectangle at (`x`, `y`), `w` by `h`;
    /// false (nothing drawn) without icons or for a level outside 1 to 50.
    fn icon(&mut self, x: f32, y: f32, w: f32, h: f32, level: u8, size: Size) -> bool {
        let Some(icon) = self.ranks.and_then(|r| r.icon(level, size)) else {
            return false;
        };
        let s = self.s;
        self.c.image(
            self.ox + x * s,
            self.oy + y * s,
            w * s,
            h * s,
            icon.width,
            icon.height,
            &icon.rgba,
        );
        true
    }

    /// A level where its number would be drawn (the same arguments as
    /// `text`, baseline `base`): Halo 2's icon for it when there are icons
    /// (`size * 1.25` high and square for Small, `size * 1.45` high and
    /// 28:26 wide for Big), centred on the middle of the capitals with `x`
    /// its left edge, centre or right edge by `align`; otherwise the
    /// number, and "-" for level 0.
    #[allow(clippy::too_many_arguments)]
    fn level(
        &mut self,
        x: f32,
        base: f32,
        size: f32,
        col: Color,
        align: Align,
        level: u8,
        kind: Size,
    ) {
        let (w, h) = match kind {
            Size::Small => (size * 1.25, size * 1.25),
            Size::Big => (size * 1.45 * 28.0 / 26.0, size * 1.45),
        };
        let left = match align {
            Align::Left => x,
            Align::Center => x - w / 2.0,
            Align::Right => x - w,
        };
        let middle = base - size * 0.35;
        if level > 0 && self.icon(left, middle - h / 2.0, w, h, level, kind) {
            return;
        }
        let n = if level == 0 {
            "-".to_string()
        } else {
            level.to_string()
        };
        self.text(x, base, size, col, align, &n);
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

    /// A button's face with its letter: round, or for the bumpers rounded
    /// and 34 by 22.
    fn button(&mut self, x: f32, y: f32, i: Input) {
        if let Input::Lb | Input::Rb = i {
            let col = Color::rgb(0x4A_5568);
            self.fill(x - 6.0, y - 11.0, 12.0, 22.0, col);
            self.circle(x - 6.0, y, 11.0, col);
            self.circle(x + 6.0, y, 11.0, col);
            let label = if i == Input::Lb { "LB" } else { "RB" };
            self.text(x, y + 4.5, 13.0, WHITE, Align::Center, label);
            return;
        }
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
            let face = face_width(i);
            self.button(x + face / 2.0, y - 7.0, i);
            let w = self.text(x + face + 8.0, y, 20.0, TEXT, Align::Left, label);
            self.area(x, y - 24.0, w + face + 12.0, 34.0, Hit::Press(i));
            x += w + face + 42.0;
        }
    }

    fn hints_width(&mut self, hints: &[(Input, &str)]) -> f32 {
        let w: f32 = hints
            .iter()
            .map(|&(i, label)| self.measure(20.0, label) + face_width(i) + 42.0)
            .sum();
        w - 30.0
    }

    /// A popup's box, which alone takes clicks while it is up.
    fn popup_box(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.top = true;
        let (cw, ch) = (self.c.w as f32, self.c.h as f32);
        self.c.fill(0.0, 0.0, cw, ch, Color::rgb(0).alpha(150));
        self.fill(x, y, w, h, Color::rgb(0x08_1A3C));
        self.outline(x, y, w, h, 2.0, EDGE);
    }

    /// A box over the screen, which alone takes clicks while it is up.
    fn popup(&mut self, title: &str, body: &str, hints: &[(Input, &str)]) {
        self.popup_box(320.0, 230.0, 640.0, 260.0);
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

    fn app(folder: &std::path::Path) -> App {
        App::new(Config {
            folder: folder.to_path_buf(),
            exe: PathBuf::from("h2launch"),
            kind: Kind::Fake,
            maps: Vec::new(),
            server: None,
            instance: None,
            log: Arc::new(|_: &str| {}),
            ranks: None,
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

    /// A lobby signed in as ALPHA (account 1) over one end of a pair, on
    /// the playlists, and the server's end, to read what the lobby sends.
    fn live_app(dir: &std::path::Path) -> (App, Connection) {
        let mut a = app(dir);
        let (ours, mut theirs) = Connection::pair();
        let key = h2live::client::identity(&dir.join("key.bin")).unwrap();
        let profile = Profile {
            gamertag: "ALPHA".into(),
            look: Default::default(),
            maps: Vec::new(),
            guests: 0,
        };
        let card = dir.join("card.txt");
        let mut c = LiveClient::launcher(ours, key, &profile, "test", &card, 0.0);
        c.view.welcome = Some(h2net::live::Welcome {
            account: 1,
            gamertag: "ALPHA".into(),
            card: String::new(),
            best: 3,
            levels: Vec::new(),
        });
        a.client = Some(c);
        a.screen = Screen::Live;
        // The LOGIN.
        sent(&mut theirs);
        (a, theirs)
    }

    /// What the lobby sent since the last look (pings left out).
    fn sent(conn: &mut Connection) -> Vec<ToServer> {
        conn.receive()
            .unwrap()
            .into_iter()
            .map(|(k, b)| ToServer::read(k, &b).unwrap())
            .filter(|m| !matches!(m, ToServer::Ping(_) | ToServer::Pong(_)))
            .collect()
    }

    fn view(a: &mut App) -> &mut View {
        &mut a.client.as_mut().unwrap().view
    }

    fn friend(account: u64, tag: &str, relation: Relation, online: Online) -> Friend {
        Friend {
            account,
            gamertag: tag.into(),
            relation,
            best: 2,
            online,
            activity: Activity::Lobby,
            playlist: 0,
            map: String::new(),
            variant: String::new(),
            party: account + 100,
            joinable: false,
        }
    }

    fn party(id: u64, activity: Activity) -> PartyInfo {
        PartyInfo {
            id,
            leader: 1,
            privacy: Privacy::Open,
            activity,
            playlist: 11,
            members: vec![PartyMember {
                account: 1,
                gamertag: "ALPHA".into(),
                look: Default::default(),
                best: 3,
                level: 3,
                guests: 0,
            }],
            maps: Vec::new(),
        }
    }

    fn tags(a: &App) -> Vec<String> {
        a.friend_rows().iter().map(|f| f.gamertag.clone()).collect()
    }

    #[test]
    fn the_friends_screen_orders_rows_and_answers_requests() {
        let dir = scratch("friends");
        let (mut a, mut server) = live_app(&dir);
        view(&mut a).friends = vec![
            friend(10, "ZED", Relation::WeAsked, Online::Offline),
            friend(11, "bob", Relation::Friend, Online::Offline),
            friend(12, "YAN", Relation::AskedUs, Online::Offline),
            friend(13, "CAT", Relation::Friend, Online::Game),
            friend(14, "DAN", Relation::Friend, Online::Launcher),
            friend(15, "AMY", Relation::AskedUs, Online::Offline),
            friend(16, "ABE", Relation::Friend, Online::Launcher),
        ];
        a.input(Input::Rb);
        assert_eq!(a.label(), "friends");
        a.tick(Instant::now());
        assert_eq!(tags(&a), ["AMY", "YAN", "ABE", "DAN", "CAT", "bob", "ZED"]);
        // A accepts the request selected, X declines it.
        a.input(Input::A);
        assert_eq!(sent(&mut server), [ToServer::FriendAccept(15)]);
        a.input(Input::Down);
        a.input(Input::X);
        assert_eq!(sent(&mut server), [ToServer::FriendDecline(12)]);
        // A request we sent: X takes it back, A does nothing.
        assert!(a.pick("zed"));
        a.input(Input::A);
        a.input(Input::X);
        assert_eq!(sent(&mut server), [ToServer::FriendRemove(10)]);
        // An offline friend can't be joined.
        assert!(a.pick("BOB"));
        a.input(Input::X);
        assert!(sent(&mut server).is_empty());
        assert!(!a.pick("NOBODY"));
        a.input(Input::B);
        assert_eq!(a.label(), "live");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn selections_follow_the_player_when_a_list_moves_them() {
        let dir = scratch("follow");
        let (mut a, mut server) = live_app(&dir);
        view(&mut a).friends = vec![
            friend(12, "YAN", Relation::AskedUs, Online::Offline),
            friend(15, "AMY", Relation::AskedUs, Online::Offline),
            friend(14, "DAN", Relation::Friend, Online::Launcher),
        ];
        a.input(Input::Rb);
        a.tick(Instant::now());
        a.input(Input::Down);
        assert_eq!(a.faccount, Some(12));
        // A new request sorts first: YAN moves down a row and stays picked,
        // so X declines YAN, not the one now on YAN's old row.
        view(&mut a)
            .friends
            .push(friend(17, "ABE", Relation::AskedUs, Online::Offline));
        a.tick(Instant::now());
        assert_eq!(a.fsel, 2);
        a.input(Input::X);
        assert_eq!(sent(&mut server), [ToServer::FriendDecline(12)]);
        // YAN goes: the same row number, now DAN's.
        view(&mut a).friends.retain(|f| f.account != 12);
        a.tick(Instant::now());
        assert_eq!((a.fsel, a.faccount), (2, Some(14)));
        // A popup naming DAN closes when DAN leaves the list.
        a.input(Input::A);
        assert_eq!(a.label(), "friend");
        a.input(Input::Up);
        a.input(Input::A);
        assert_eq!(a.label(), "unfriend");
        view(&mut a).friends.retain(|f| f.account != 14);
        a.tick(Instant::now());
        assert_eq!(a.label(), "friends");
        assert!(sent(&mut server).is_empty());
        // The players screen's selection follows ONLINE lists the same way.
        let player = |account: u64, tag: &str| OnlinePlayer {
            account,
            gamertag: tag.into(),
            look: Default::default(),
            best: 1,
            activity: Activity::Lobby,
            party: account,
            open: true,
            size: 1,
            openings: 3,
        };
        view(&mut a).online = vec![
            player(1, "ALPHA"),
            player(20, "BRAVO"),
            player(21, "CHARLIE"),
        ];
        a.screen = Screen::Players;
        a.tick(Instant::now());
        assert!(a.pick("charlie"));
        view(&mut a).online.insert(1, player(22, "ABLE"));
        a.tick(Instant::now());
        a.input(Input::A);
        assert_eq!(sent(&mut server), [ToServer::Invite(21)]);
        // Y asks the player picked to be friends, by gamertag.
        a.input(Input::Y);
        assert_eq!(
            sent(&mut server),
            [ToServer::FriendRequest("CHARLIE".into())]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bumpers_work_while_searching_and_friend_notices_leave_asked() {
        let dir = scratch("search");
        let (mut a, mut server) = live_app(&dir);
        view(&mut a).party = Some(party(500, Activity::Searching));
        assert_eq!(a.label(), "searching");
        a.input(Input::Rb);
        assert_eq!(a.label(), "friends");
        a.input(Input::B);
        assert_eq!(a.label(), "searching");
        // Q is LB where letters are buttons: our own service record.
        a.input(Input::Char('q'));
        assert_eq!(a.label(), "record");
        assert_eq!(sent(&mut server), [ToServer::Record(1)]);
        a.input(Input::B);
        assert_eq!(a.label(), "searching");
        a.input(Input::Char('e'));
        assert_eq!(a.label(), "friends");
        // The custom game asked for is still waited for after a friend
        // notice, but not after another notice.
        a.asked = Some(a.now);
        let notice = h2net::live::friend_notice(h2net::live::ASKED_YOU, "BRAVO");
        a.on_event(LiveEvent::Notice(notice));
        assert!(a.asked.is_some());
        a.on_event(LiveEvent::Notice(h2net::live::SLOW_DOWN.into()));
        assert!(a.asked.is_some());
        a.on_event(LiveEvent::Notice("THAT MAP ISN'T IN A PLAYLIST".into()));
        assert!(a.asked.is_none());
        assert_eq!(a.toasts.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_add_friend_popup_takes_typed_letters() {
        let dir = scratch("addfriend");
        let (mut a, mut server) = live_app(&dir);
        a.input(Input::Rb);
        a.input(Input::Y);
        assert_eq!(a.label(), "addfriend");
        // Letters are typed, not buttons (B would close it, X and Y...).
        for c in "bravo xy!".chars() {
            a.input(Input::Char(c));
        }
        assert_eq!(a.label(), "addfriend");
        assert_eq!(a.friend_tag, "BRAVO XY");
        a.input(Input::Backspace);
        a.input(Input::Backspace);
        a.input(Input::Backspace);
        a.input(Input::A);
        assert_eq!(a.label(), "friends");
        assert_eq!(sent(&mut server), [ToServer::FriendRequest("BRAVO".into())]);
        // Once: and an empty one sends nothing.
        a.input(Input::Y);
        assert!(a.friend_tag.is_empty());
        a.input(Input::A);
        a.input(Input::Y);
        a.input(Input::Char('z'));
        a.input(Input::B);
        assert_eq!(a.label(), "friends");
        assert!(sent(&mut server).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_options_popup_lists_only_what_applies() {
        let dir = scratch("options");
        let (mut a, mut server) = live_app(&dir);
        view(&mut a).party = Some(party(500, Activity::Lobby));
        let mut dan = friend(14, "DAN", Relation::Friend, Online::Launcher);
        dan.joinable = true;
        let mut eve = friend(18, "EVE", Relation::Friend, Online::Launcher);
        eve.party = 500;
        view(&mut a).friends = vec![
            dan,
            eve,
            friend(11, "BOB", Relation::Friend, Online::Offline),
        ];
        use FriendOption::*;
        assert_eq!(a.friend_options(14), [Invite, Join, Record, Remove]);
        // Already in our party: no invite.
        assert_eq!(a.friend_options(18), [Record, Remove]);
        assert_eq!(a.friend_options(11), [Record, Remove]);
        a.input(Input::Rb);
        a.tick(Instant::now());
        assert!(a.pick("dan"));
        a.input(Input::A);
        assert_eq!(a.label(), "friend");
        // Up from the top is Remove friend, the last.
        a.input(Input::Up);
        assert_eq!(a.osel, 3);
        a.input(Input::Down);
        assert_eq!(a.osel, 0);
        a.input(Input::A);
        assert_eq!(sent(&mut server), [ToServer::Invite(14)]);
        assert_eq!(a.label(), "friends");
        a.input(Input::A);
        a.input(Input::Down);
        a.input(Input::A);
        assert_eq!(sent(&mut server), [ToServer::JoinParty(114)]);
        assert_eq!(a.label(), "live");
        a.input(Input::Rb);
        assert!(a.pick("bob"));
        a.input(Input::A);
        a.input(Input::Up);
        a.input(Input::A);
        assert_eq!(a.label(), "unfriend");
        a.input(Input::A);
        assert_eq!(sent(&mut server), [ToServer::FriendRemove(11)]);

        // A new list that drops Join keeps Remove friend picked...
        a.screen = Screen::Friends;
        assert!(a.pick("dan"));
        a.input(Input::A);
        a.input(Input::Up);
        assert_eq!(a.opick, Some(Remove));
        view(&mut a).friends[0].joinable = false;
        a.tick(a.now);
        assert_eq!((a.osel, a.opick), (2, Some(Remove)));
        // ... and one that drops Invite keeps Service record picked.
        a.input(Input::Up);
        assert_eq!(a.opick, Some(Record));
        view(&mut a).friends[0].party = 500;
        a.tick(a.now);
        assert_eq!((a.osel, a.opick), (0, Some(Record)));
        // Join back: the same option still, on its new row.
        view(&mut a).friends[0].joinable = true;
        a.tick(a.now);
        assert_eq!((a.osel, a.opick), (1, Some(Record)));
        a.input(Input::A);
        assert_eq!(a.label(), "record");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_friend_who_left_their_partys_match_shows_its_playlist_alone() {
        let dir = scratch("left-match");
        let (a, _server) = live_app(&dir);
        let mut dan = friend(14, "DAN", Relation::Friend, Online::Launcher);
        dan.activity = Activity::Playing;
        dan.playlist = CUSTOM_GAME;
        assert_eq!(a.friend_status(&dan).0, "In a custom game");
        dan.map = "lockout".into();
        dan.variant = "H2_Team_Slayer".into();
        let status = a.friend_status(&dan).0;
        assert!(status.starts_with("Custom game: ") && status.ends_with(" on Lockout"));
        dan.playlist = 11;
        dan.map.clear();
        assert!(a.friend_status(&dan).0.starts_with("Playing "));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn signing_in_again_forgets_the_records_kept() {
        let dir = scratch("records-forgotten");
        let (mut a, _server) = live_app(&dir);
        a.keep_record(ServiceRecord {
            account: 2,
            found: None,
        });
        a.record_asks.push((3, a.now));
        a.fields[0] = "ALPHA".into();
        a.fields[1] = "127.0.0.1:1".into();
        a.connect();
        assert!(a.cached_record(2).is_none() && a.record_asks.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A game of ALPHA (this PC) against BRAVO, over, on the carnage
    /// report.
    fn carnage(a: &mut App) {
        let player = |relay_id: u64, account: u64, tag: &str| h2net::live::LauncherPlayer {
            relay_id,
            account,
            gamertag: tag.into(),
            team: 0,
            level: 4,
            party: account,
        };
        let m = LauncherMatch {
            id: 0x77,
            playlist: 11,
            ranked: true,
            teams: false,
            map: "lockout".into(),
            variant: "H2_FFA_HeadtoHead_b".into(),
            relay: h2net::live::RelaySeat {
                port: 47050,
                room: 0x77,
                id: 0x11,
                key: [0; 16],
            },
            host: 0x11,
            countdown: 20,
            players: vec![player(0x11, 1, "ALPHA"), player(0x22, 2, "BRAVO")],
        };
        let ip = "127.0.0.1".parse().unwrap();
        let (session, me) = live::session_from(&m, ip).unwrap();
        let result = |relay_id, place, score| LauncherPlayerResult {
            relay_id,
            team: place,
            place,
            score,
            kills: 5,
            assists: 2,
            deaths: 3,
            betrayals: 0,
            suicides: 0,
            left: false,
        };
        let results = vec![result(0x11, 1, 3), result(0x22, 0, 5)];
        a.game = Some(Game {
            m,
            session,
            me,
            told: Told::default(),
            launch: false,
            start_at: a.now,
            child: None,
            running: true,
            loaded: true,
            results: Some(results),
            ended_at: Some(a.now),
            over: None,
        });
        a.screen = Screen::Carnage;
        a.csel = 1;
    }

    #[test]
    fn the_carnage_report_opens_service_records_and_asks_friends() {
        let dir = scratch("carnage");
        let (mut a, mut server) = live_app(&dir);
        carnage(&mut a);
        // BRAVO won: first. We're second.
        let rows: Vec<u64> = carnage_rows(a.game.as_ref().unwrap())
            .iter()
            .map(|r| r.relay_id)
            .collect();
        assert_eq!(rows, [0x22, 0x11]);
        // Y on ourselves does nothing; on BRAVO it's a friend request.
        a.input(Input::Y);
        assert!(sent(&mut server).is_empty());
        assert!(a.pick("bravo"));
        assert_eq!(a.csel, 0);
        a.input(Input::Y);
        assert_eq!(sent(&mut server), [ToServer::FriendRequest("BRAVO".into())]);
        a.input(Input::Lb);
        assert_eq!(a.label(), "record");
        assert_eq!(sent(&mut server), [ToServer::Record(2)]);
        a.on_event(LiveEvent::ServiceRecord(ServiceRecord {
            account: 2,
            found: None,
        }));
        if let Ok(mut text) = Text::system() {
            a.draw(&mut Canvas::new(640, 360), &mut text);
            assert!(a.drawn().iter().any(|d| d == "No such player."));
        }
        a.input(Input::B);
        assert_eq!(a.label(), "carnage-waiting");
        // Within a minute the record kept is shown, with nothing asked.
        a.input(Input::Lb);
        assert_eq!(a.label(), "record");
        assert!(sent(&mut server).is_empty());
        // MATCH_OVER drops the records of everyone in the match, and the
        // one on screen is asked for again (it's loading, not unanswered).
        a.on_event(LiveEvent::MatchOver(MatchOver {
            id: 0x77,
            counted: true,
            reason: String::new(),
            card: String::new(),
            levels: Vec::new(),
            players: vec![(1, 5), (2, 4)],
        }));
        assert!(a.cached_record(2).is_none());
        assert_eq!(a.label(), "record");
        assert!(matches!(a.record_view(), RecordView::Loading));
        assert_eq!(sent(&mut server), [ToServer::Record(2)]);
        // A second ask while waiting sends nothing; after RECORD_WAIT with
        // no answer, the screen says so and A asks again.
        a.input(Input::B);
        a.input(Input::Lb);
        assert!(sent(&mut server).is_empty());
        a.tick(a.now + RECORD_WAIT);
        assert!(matches!(a.record_view(), RecordView::NoAnswer));
        a.input(Input::A);
        assert_eq!(sent(&mut server), [ToServer::Record(2)]);
        // A continues once back on the report.
        a.input(Input::B);
        a.input(Input::A);
        assert_eq!(a.label(), "live");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Icons of one colour each: red for the big set, green for the small.
    fn icons() -> RankIcons {
        let set = |w: u32, h: u32, rgba: [u8; 4]| -> Vec<blam_cache::bitmap::Image> {
            (0..50)
                .map(|_| blam_cache::bitmap::Image {
                    width: w,
                    height: h,
                    rgba: rgba.repeat((w * h) as usize),
                })
                .collect()
        };
        let big = set(28, 26, [255, 0, 0, 255]);
        let small = set(17, 17, [0, 255, 0, 255]);
        RankIcons::new(big, small, PathBuf::from("mainmenu.map")).unwrap()
    }

    #[test]
    fn the_service_record_draws_with_and_without_icons() {
        let Ok(mut text) = Text::system() else {
            return;
        };
        let dir = scratch("record");
        let (mut a, _server) = live_app(&dir);
        let tally = h2net::live::Tally {
            kills: 7,
            assists: 2,
            deaths: 2,
            betrayals: 0,
            suicides: 1,
        };
        let row = |playlist| h2net::live::PlaylistRecord {
            playlist,
            level: 8,
            games: 3,
            wins: 2,
            tally,
        };
        a.on_event(LiveEvent::ServiceRecord(ServiceRecord {
            account: 2,
            found: Some(h2net::live::Record {
                gamertag: "BRAVO".into(),
                look: Default::default(),
                best: 9,
                created: 1_791_633_600,
                playlists: vec![row(11), row(40)],
            }),
        }));
        a.open_record(2);
        let mut c = Canvas::new(1280, 720);
        a.draw(&mut c, &mut text);
        let drawn = a.drawn().to_vec();
        for want in [
            "BRAVO",
            "HIGHEST LEVEL",
            "9",
            "Ranked games",
            "6",
            "8",
            "14",
            "3.50",
            "Member since 2026-10-10",
            "Playlist 40",
        ] {
            assert!(drawn.iter().any(|d| d == want), "{want} in {drawn:?}");
        }
        let red = |c: &Canvas| c.px.contains(&0xFF_0000);
        let green = |c: &Canvas| c.px.contains(&0x00_FF00);
        assert!(!red(&c) && !green(&c));
        a.cfg.ranks = Some(Arc::new(icons()));
        let mut c = Canvas::new(1280, 720);
        a.draw(&mut c, &mut text);
        // The highest level big, and each row's level small.
        assert!(red(&c) && green(&c));
        assert!(!a.drawn().iter().any(|d| d == "8"));
        // The highest level's icon goes where the layout puts it.
        assert_eq!(c.px[230 * 1280 + 126], 0xFF_0000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn levels_dates_and_kills_per_death() {
        assert_eq!(kd(0, 0), "-");
        assert_eq!(kd(3, 0), "3.00");
        assert_eq!(kd(7, 2), "3.50");
        assert_eq!(kd(0, 4), "0.00");
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(1_791_633_600), "2026-10-10");
        assert_eq!(utc_date(951_868_799), "2000-02-29");
        assert_eq!(follow(&[1, 2, 3], Some(3), 0), 2);
        assert_eq!(follow(&[1, 2], Some(9), 5), 1);
        assert_eq!(follow(&[], Some(9), 5), 0);
    }
}
