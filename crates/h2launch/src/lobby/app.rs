//! The lobby's screens and what happens on them: Halo 2's intro movie
//! (`intro`), its start screen and main menu (drawn by `h2ui`), signing
//! in, the party and the playlists, searching, custom games, the pregame
//! lobby, the match (its engine runs in a second copy of the launcher, see
//! `child`), the carnage report, the friends list and players' service
//! records. Behind the start screen and main menu goes the main menu's
//! flythrough when it can be drawn (`h2scene`, `Config::scene`), else a
//! still background; every screen after them sits in Halo 2's shell
//! (`shell`): its background, header and button legend.
//! Nothing here opens a window: `window`, or the headless loop in
//! `mod.rs`, gives it input and time and shows what it draws.

use super::canvas::{Align, Canvas, Color, Style, Text};
use super::child::{self, Kind, Match, Running, Said};
use super::intro::{self, End, Intro, Plan};
use super::names;
use super::ranks::{RankIcons, Size};
use super::settings::{self, Settings, GAMERTAG_LEN};
use super::sound::{Cue, SoundOut};
use super::sources::Vista;
use crate::controls::{self, button, Controls};
use crate::gamemenu::Leave;
use crate::live::{self, Engine, Log, Told};
use crate::session::Session;
use h2live::client::{LiveClient, LiveEvent, Profile, View};
use h2net::live::{
    Activity, Friend, LauncherMatch, LauncherPlayerResult, MatchOver, Online, OnlinePlayer,
    PartyInfo, PartyMember, PlaylistInfo, Privacy, Relation, ServiceRecord, Stage, ToServer,
    CUSTOM_GAME, GAMERTAG_TAKEN, MAX_FRIENDS, QUICKMATCH,
};
use h2net::Connection;
use h2scene::{Frame, SceneSource, SceneState};
use h2ui::anim::{Animation, Focus};
use h2ui::art::ImageRef;
use h2ui::cpu::Kept;
use h2ui::layout::{BitmapWidget, Space};
use h2ui::paint::{Blend, DrawList, Quad, Resources, Texture, Textures};
use h2ui::screens::{self, Backdrop, MainMenu, Start};
use h2ui::shell::Shell;
use h2ui::text::Fonts;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod dialogs;
use dialogs::DialogKey;
mod shell;

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
/// The controller settings screen's rows, in the order and words of Halo
/// 2's CONTROLLER screen (in a profile's settings), then the mouse's two,
/// then Halo 2's last, Restore Defaults.
const SETTINGS_ROWS: [&str; 9] = [
    "Thumbstick Layout",
    "Button Layout",
    "Look Sensitivity",
    "Look Inversion",
    "Automatic Look Centering",
    "Controller Vibration",
    "Mouse Sensitivity",
    "Mouse Inversion",
    "Restore Defaults",
];
const STICKS_ROW: usize = 0;
const BUTTONS_ROW: usize = 1;
const MOUSE_ROW: usize = 6;
const MOUSE_INVERT_ROW: usize = 7;
const RESTORE_ROW: usize = 8;
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
/// The main menu's rows, in the original Xbox wording (MCC's own strings
/// say LIVE). Split screen and system link aren't offered.
const MAIN_ROWS: [&str; 2] = ["ONLINE", "SETTINGS"];
const LIVE_ROW: usize = 0;
const MAIN_SETTINGS_ROW: usize = 1;
/// The in-game screen's line while the engine plays.
const IN_GAME_MENU_HINT: &str = "Start or Esc in the game's window opens the game menu.";
/// Frames of a screen timed at each window size before the log says how
/// long one takes.
const MENU_FRAMES: u32 = 30;
/// A click this soon (seconds) after one that changed the screen or shut a
/// popup is the second half of a double-click: on the start screen it
/// doesn't go on, on the main menu it only focuses a row, and elsewhere it
/// presses no button or arrow and only selects a row. Windows' default
/// double-click time is 0.5 s; 0.4 s is our own choice.
const DOUBLE_CLICK: f64 = 0.4;
/// Cues kept for the sound to take (`App::take_sound`); older ones are
/// dropped when nothing takes them (no sound, and the tests).
const CUES_KEPT: usize = 16;
/// How often the window draws a frame: about 60 a second while the intro
/// movie plays (its pictures come 60 a second, going by its name), else
/// about 30 (the menus' animations and the lobby's screens).
const FRAME_MOVIE: Duration = Duration::from_millis(16);
const FRAME: Duration = Duration::from_millis(33);
/// How often the window is drawn while the engine plays and the lobby's
/// window hasn't the focus (`draw_interval`): our own choice, often
/// enough for a lobby left in sight on another screen to keep up.
const FRAME_BEHIND_GAME: Duration = Duration::from_millis(500);
/// The flythrough's first frame fades in over the still background in
/// this long (seconds), when it wasn't ready as the start screen opened
/// (the intro wakes it ahead): our estimate, so the scene doesn't pop in
/// once the GPU has it (Halo 2 had it from the start).
const SCENE_FADE: f64 = 0.25;
/// The flythrough's clock goes on at most this much (seconds) between two
/// frames, so a stall (the window dragged, a slow PC) doesn't jump it.
const SCENE_STEP: f64 = 0.25;

/// The start screen's and main menu's layouts, pictures and fonts
/// (`h2ui`), read once when the lobby opens.
pub struct Menus {
    pub shell: Shell,
    pub fonts: Fonts,
}

impl Menus {
    /// The flat look: the built-in layouts, plain shapes and the built-in
    /// font (what CI and Linux get, and the tests use).
    pub fn flat() -> Menus {
        Menus {
            shell: Shell::from_tags(None, None),
            fonts: Fonts::fallback(),
        }
    }
}

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
    /// The start screen and main menu (`Menus::flat` without the files).
    pub menus: Arc<Menus>,
    /// The Halo 2 Vista (Project Cartographer) files, if `sources::find`
    /// found them: the menus, fonts and rank icons above came from there
    /// first.
    pub vista: Option<Vista>,
    /// The intro movie to play before the start screen (`mod.rs`'s
    /// `intro_plan`: a window, Windows, Vista's movie, not switched off).
    pub intro: Option<Plan>,
    /// For tests: more words for each match's engine
    /// (`H2LOBBY_ENGINE_ARGS`: `--set-option`, `--pad`, a control flag),
    /// read once when the lobby starts.
    pub engine_args: String,
    /// The main menu's flythrough behind the start screen and main menu
    /// (`h2scene::Backdrop`, which `mod.rs` starts when it can be drawn);
    /// None, or a scene that fails, leaves the still background.
    pub scene: Option<Arc<dyn SceneSource>>,
    /// `H2LOBBY_SCENE_FPS=60`: the window draws about 60 frames a second
    /// while the flythrough shows, not 30.
    pub scene_fast: bool,
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
    /// The pad's triggers: only the intro takes them (they skip it, as
    /// every button but the d-pad and the stick clicks does in Halo 2,
    /// 0x156ab0); every other screen leaves them alone.
    Lt,
    Rt,
    Tab,
    Backspace,
    Char(char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    /// Halo 2's intro movie, where the lobby opens when there is one to
    /// play (`Config::intro`); any button but a direction skips it.
    Intro,
    /// Halo 2's start screen ("PRESS START"), where the lobby opens
    /// without the intro, and after it.
    Start,
    /// Halo 2's main menu: ONLINE and SETTINGS.
    Main,
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
    /// The controller settings (`Settings::controls`), from the main
    /// menu's SETTINGS, the playlists' Settings row, or X on the sign-in
    /// and offline screens.
    Settings,
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
    /// Picks a row of the dialog up (`dialogs`), after giving it the
    /// focus.
    Choice(usize),
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
    /// The player left the game from the engine's GAME MENU (or ended it
    /// for everyone).
    left: Option<Leave>,
    /// The engine was started with its GAME MENU (not turned off with
    /// `H2LOBBY_ENGINE_ARGS=--no-game-menu` or `H2LAUNCH_OVERLAY_FAIL=1`),
    /// and hasn't said since that it turned it off (`Engine::MenuOff`).
    menu: bool,
    /// The controls the engine runs with: those it was started with, then
    /// the GAME MENU's last; None before it starts.
    engine_controls: Option<Controls>,
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
    /// The gamertag and server last saved by a sign-in (or read from
    /// `lobby.txt`): what ONLINE signs in with and the main menu's
    /// small print shows, whatever the form or a refused sign-in left in
    /// `fields` and `settings`.
    saved: [String; 2],
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
    /// The settings screen's selected row, and the screen it goes back to.
    srow: usize,
    settings_back: Screen,
    game: Option<Game>,
    popup: Option<Popup>,
    toasts: VecDeque<(String, Instant)>,
    failed: String,
    areas: Vec<Area>,
    /// Layout units to window pixels: scale and offset.
    view: (f32, f32, f32),
    quit: bool,
    focus: bool,
    /// The main menu's focus: which row has it, and its fades. The row
    /// picked last keeps it when the player comes back.
    main_focus: Focus,
    /// The screen drawn last, and when (on the menus' clock) the start
    /// screen or main menu on screen opened, for their intro animations.
    drawn_screen: Option<Screen>,
    menu_opened: f64,
    /// The menus' still background, drawn once for a window size.
    backdrop: Kept<u32>,
    /// The shell's still background behind the other screens
    /// (`shell::draw_shell`), kept apart from the menus' so going between
    /// the main menu and them never draws either again.
    shell_backdrop: Kept<u32>,
    /// The keyboard or the mouse was used last, not the controller: the
    /// button legends name keys rather than show the pad's buttons. The
    /// controller comes first, so it starts false.
    keyboard: bool,
    /// The flythrough (`Config::scene`): how far into it the menus are
    /// (seconds; its clock runs only while the start screen or main menu
    /// is up), the menus' clock at its last step, when its first frame
    /// showed (it fades in from then; None until a frame of the window's
    /// size comes), whether it was told to sleep, whether it failed (the
    /// still background for the rest of the run), whether its frame is
    /// on screen (`frame_interval`), and the window's size the intro
    /// asked for its first frame at (`scene_warm_up`).
    scene_seconds: f64,
    scene_step: Option<f64>,
    scene_since: Option<f64>,
    scene_asleep: bool,
    scene_failed: bool,
    scene_shown: bool,
    scene_warm: Option<(usize, usize)>,
    /// The window's size in pixels at the last draw of the start screen
    /// or main menu: the main menu's rows are found under the mouse at it.
    size: (usize, usize),
    menu_times: MenuTimes,
    /// The last click: when (on the menus' clock), the screen it was on,
    /// and the popup or invitation that took it (`DOUBLE_CLICK`).
    last_click: Option<(f64, Screen, Option<DialogKey>)>,
    /// The sounds asked for since the sound last took them (`take_sound`),
    /// and the intro's soundtrack starting or stopping.
    cues: Vec<Cue>,
    movie_sound: Vec<SoundOut>,
    /// The intro movie playing (from the first draw on its screen), the
    /// picture drawn last (its serial number), and that picture drawn into
    /// the window's size once.
    intro: Option<Intro>,
    intro_serial: u64,
    intro_kept: Kept<u32>,
    /// The menu music is wanted (`music_rule`).
    music_on: bool,
    /// The press `input_inner` acted on last, after letters and Backspace
    /// were turned into buttons: B's changes sound as Back, and only A's
    /// opening a screen as Select too (`cue_for`).
    pressed: Option<Input>,
    /// Something asked for couldn't be done (`refuse`).
    refused: bool,
    /// The invitation shown last (its party), so a new one sounds once.
    invite_seen: Option<u64>,
    /// The dialog up (a popup, the invitation or the offline screen,
    /// `dialogs`): which one was seen last, when it opened (on the menus'
    /// clock; its pieces come in from then) and its focused row.
    dialog_shown: Option<DialogKey>,
    dialog_opened: f64,
    dialog_focus: Focus,
}

/// What the cue of a press is worked out from (`App::cue_state`), taken
/// before and after it.
#[derive(Clone, Debug, PartialEq)]
struct CueState {
    screen: Screen,
    popup: Option<Popup>,
    /// The invitation shown (its party).
    invite: Option<u64>,
    /// The focused row (or field) of the list on show.
    focus: usize,
    /// The values a press can change in place: the custom game's game and
    /// map, and the controller settings.
    values: (usize, usize, Controls),
    refused: bool,
}

/// The sounds of a press, from the state before and after it (`pressed`:
/// the button it was), as Halo 2 plays them: Error if it was refused; if
/// the screen, popup or invitation changed, Back on B (a screen brought
/// back from the history plays no `advance`, 0x23553f); else, when a
/// screen or a popup opens, its `advance` (every screen and dialog loaded
/// forward plays it, 0x147f6d), after Select for A (the list item's or
/// button's `forward1`, 0x24ac05 and 0x2540e8, which only A and Start
/// play; a shortcut such as X or RB opening a screen, like Halo 2's Y
/// opening its Y menu, is the `advance` alone); Select alone between the
/// Y menu's tabs (the players and friends lists: the tab's `forward1`)
/// and when a popup only closes; Cursor if only the focus or a value
/// changed.
fn cue_for(before: &CueState, after: &CueState, pressed: Option<Input>) -> &'static [Cue] {
    // Leaving the intro sounds nothing: the press isn't a list item's or a
    // button's, and the start screen's load plays no `advance`
    // (0x147f6d).
    if before.screen == Screen::Intro {
        return &[];
    }
    if after.refused {
        return &[Cue::Error];
    }
    let screen = before.screen != after.screen;
    let popup = before.popup != after.popup;
    if screen || popup || before.invite != after.invite {
        let y_menu = |s: Screen| matches!(s, Screen::Players | Screen::Friends);
        let tab = y_menu(before.screen) && y_menu(after.screen);
        let opens = (screen && !tab) || (popup && after.popup.is_some());
        return match (pressed, opens) {
            (Some(Input::B), _) => &[Cue::Back],
            (Some(Input::A), true) => &[Cue::Select, Cue::Advance],
            (_, true) => &[Cue::Advance],
            (_, false) => &[Cue::Select],
        };
    }
    if before.focus != after.focus || before.values != after.values {
        return &[Cue::Cursor];
    }
    &[]
}

/// `text` is the server's notice that someone sent us a friend request
/// (`h2net::live::ASKED_YOU`, with whatever gamertag in it).
fn is_asked_you(text: &str) -> bool {
    match h2net::live::ASKED_YOU.split_once("{X}") {
        Some((before, after)) => {
            text.len() > before.len() + after.len()
                && text.starts_with(before)
                && text.ends_with(after)
        }
        None => text == h2net::live::ASKED_YOU,
    }
}

/// Whether the menu music is wanted on `screen`, it being `current` (Halo
/// 2's rule, menu.md section 5): on at the start screen and the main
/// menu; off during the intro movie (it has its own sound; the music
/// starts with the start screen), on the ONLINE screens (searching too),
/// the party, the custom game and the match's screens; elsewhere as it
/// was, so the settings keep it and the carnage report stays silent after
/// a game.
fn music_rule(screen: Screen, current: bool) -> bool {
    match screen {
        Screen::Start | Screen::Main => true,
        Screen::Intro
        | Screen::Live
        | Screen::Party
        | Screen::Custom
        | Screen::Pregame
        | Screen::InGame => false,
        Screen::SignIn
        | Screen::Connecting
        | Screen::Players
        | Screen::Carnage
        | Screen::Failed
        | Screen::Record
        | Screen::Friends
        | Screen::Settings => current,
    }
}

/// How far (0 to 1) the flythrough has faded in over the still
/// background `elapsed` seconds after its first frame showed.
fn scene_alpha(elapsed: f64) -> f32 {
    (elapsed / SCENE_FADE).clamp(0.0, 1.0) as f32
}

/// `top` over `under` (0x00RRGGBB pixels, the same count), `alpha` of the
/// way (0 keeps `under`, 1 is `top`).
fn blend_px(under: &mut [u32], top: &[u32], alpha: f32) {
    let a = (alpha.clamp(0.0, 1.0) * 256.0).round() as u32;
    if a == 0 {
        return;
    }
    if a == 256 {
        under.copy_from_slice(top);
        return;
    }
    let b = 256 - a;
    for (u, t) in under.iter_mut().zip(top) {
        // Red and blue together, then green: each lane's sum stays below
        // its 8 bits times 256 (plus a half for the rounding).
        // Rounded to nearest.
        let rb = (((*u & 0xFF_00FF) * b + (*t & 0xFF_00FF) * a + 0x80_0080) >> 8) & 0xFF_00FF;
        let g = (((*u & 0x00_FF00) * b + (*t & 0x00_FF00) * a + 0x00_8000) >> 8) & 0x00_FF00;
        *u = rb | g;
    }
}

/// How long each screen takes to draw on the CPU, for the log (menu.md,
/// phase 1's "Done when" and phase 2's CPU budget): the menus' kept
/// background once for each window size, and each screen's frames (by
/// `App::timer_name`), averaged over the first `MENU_FRAMES` at a size.
#[derive(Default)]
struct MenuTimes {
    size: (usize, usize),
    /// Each screen's frames timed at this size.
    frames: Vec<ScreenTimes>,
    /// The size the menus' still background was last drawn at: a frame
    /// that draws it again isn't timed.
    kept: (usize, usize),
    /// Each size's background time (ms), the first time it was drawn.
    backgrounds: Vec<((usize, usize), f64)>,
    /// The sizes and screens already logged.
    logged: Vec<((usize, usize), &'static str)>,
}

/// One screen's frames timed at the window's size (`MenuTimes`).
struct ScreenTimes {
    /// Its timer name (`App::timer_name`).
    name: &'static str,
    /// How many frames, and their total time (ms).
    frames: u32,
    total: f64,
    /// The flythrough's frames copied in (`draw_menu`) among them: how
    /// many, and their total time (ms).
    copies: u32,
    copy_total: f64,
}

impl App {
    /// The lobby, on the intro movie if there is one to play
    /// (`Config::intro`), else on the start screen. It signs in when the
    /// player picks ONLINE on the main menu.
    pub fn new(cfg: Config) -> App {
        let mut settings = Settings::load(&cfg.folder.join("lobby.txt"));
        if let Some(s) = &cfg.server {
            settings.server = s.clone();
        }
        let now = Instant::now();
        let screen = match cfg.intro {
            Some(_) => Screen::Intro,
            None => Screen::Start,
        };
        App {
            fields: [settings.gamertag.clone(), settings.server.clone()],
            saved: [settings.gamertag.clone(), settings.server.clone()],
            settings,
            cfg,
            screen,
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
            srow: 0,
            settings_back: Screen::Main,
            game: None,
            popup: None,
            toasts: VecDeque::new(),
            failed: String::new(),
            areas: Vec::new(),
            view: (1.0, 0.0, 0.0),
            quit: false,
            focus: false,
            main_focus: Focus::on(LIVE_ROW, 0.0),
            drawn_screen: None,
            menu_opened: 0.0,
            backdrop: Kept::default(),
            shell_backdrop: Kept::default(),
            keyboard: false,
            scene_seconds: 0.0,
            scene_step: None,
            scene_since: None,
            // Nothing to put to sleep until the menus first ask for it.
            scene_asleep: true,
            scene_failed: false,
            scene_shown: false,
            scene_warm: None,
            size: (0, 0),
            menu_times: MenuTimes::default(),
            last_click: None,
            cues: Vec::new(),
            movie_sound: Vec::new(),
            intro: None,
            intro_serial: 0,
            intro_kept: Kept::default(),
            music_on: screen != Screen::Intro,
            pressed: None,
            refused: false,
            invite_seen: None,
            dialog_shown: None,
            dialog_opened: 0.0,
            dialog_focus: Focus::on(0, 0.0),
        }
    }

    fn log(&self, line: &str) {
        (self.cfg.log)(line);
    }

    /// The window should close.
    pub fn quitting(&self) -> bool {
        self.quit
    }

    /// The window should come to the front (once): the engine closed, and
    /// the carnage report or the menus are up.
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
            Screen::Intro => "intro",
            Screen::Start => "start",
            Screen::Main => "main",
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
            Screen::Settings => "settings",
        }
        .into()
    }

    /// The text drawn in the last frame, each string as it was drawn.
    pub fn drawn(&self) -> &[String] {
        &self.drawn
    }

    /// What the sound should play since the last call: the intro's
    /// soundtrack starting or stopping, then the cues, oldest first.
    pub fn take_sound(&mut self) -> Vec<SoundOut> {
        let mut out = std::mem::take(&mut self.movie_sound);
        out.extend(self.cues.drain(..).map(SoundOut::Cue));
        out
    }

    /// The cues alone (`take_sound`'s), for the tests.
    #[cfg(test)]
    pub fn take_cues(&mut self) -> Vec<Cue> {
        std::mem::take(&mut self.cues)
    }

    /// How long the window waits between frames: shorter while the intro
    /// movie plays, and while the flythrough shows if `Config::scene_fast`
    /// asks for 60 frames a second. It wakes at least this often even
    /// when it draws less often (`draw_interval`), as `tick` polls the
    /// server and the engine.
    pub fn frame_interval(&self) -> Duration {
        match self.screen {
            Screen::Intro => FRAME_MOVIE,
            Screen::Start | Screen::Main if self.cfg.scene_fast && self.scene_shown => FRAME_MOVIE,
            _ => FRAME,
        }
    }

    /// How long the window waits between the frames it draws: each
    /// `frame_interval`, except while the engine plays and the lobby's
    /// window hasn't the focus (`focused`), when the shell's scrolling
    /// strips needn't be drawn 30 times a second under the engine's window
    /// (Windows still sends a covered window its paints): only every
    /// `FRAME_BEHIND_GAME`. A key, a click or the focus draws at once.
    pub fn draw_interval(&self, focused: bool) -> Duration {
        if self.engine_running() && !focused {
            FRAME_BEHIND_GAME
        } else {
            self.frame_interval()
        }
    }

    /// The menu music is wanted (`music_rule`, kept by `tick`).
    pub fn music_on(&self) -> bool {
        self.music_on
    }

    fn cue(&mut self, c: Cue) {
        if self.cues.len() == CUES_KEPT {
            self.cues.remove(0);
        }
        self.cues.push(c);
    }

    /// What a press's cue is worked out from (`cue_for`).
    fn cue_state(&self) -> CueState {
        let focus = match (self.popup, self.screen) {
            (Some(Popup::FriendOptions(_)), _) => self.osel,
            // A dialog's row (`dialogs`).
            _ if self.dialog_key().is_some() => self.dialog_focus.item,
            (Some(_), _) => 0,
            (None, Screen::Main) => self.main_focus.item,
            (None, Screen::Live) => self.sel,
            (None, Screen::Players) => self.psel,
            (None, Screen::Party) => self.msel,
            (None, Screen::Custom) => self.crow,
            (None, Screen::Settings) => self.srow,
            (None, Screen::Friends) => self.fsel,
            (None, Screen::Carnage) => self.csel,
            (None, Screen::Record) => self.record_first,
            (None, Screen::SignIn) => self.field,
            (
                None,
                Screen::Intro
                | Screen::Start
                | Screen::Connecting
                | Screen::Pregame
                | Screen::InGame
                | Screen::Failed,
            ) => 0,
        };
        CueState {
            screen: self.screen,
            popup: self.popup,
            invite: self.invite().map(|(party, _)| party),
            focus,
            values: (self.cgame, self.cmap, self.settings.controls),
            refused: self.refused,
        }
    }

    /// Run a press (`act`), then ask for its sounds: cues it asked for
    /// itself stand, else `cue_for`'s from before and after.
    fn sounding(&mut self, act: impl FnOnce(&mut App)) {
        self.follow_dialog();
        let before = self.cue_state();
        let asked = self.cues.len();
        self.pressed = None;
        self.refused = false;
        act(self);
        self.follow_dialog();
        let after = self.cue_state();
        self.refused = false;
        if !after.refused && self.cues.len() > asked {
            return;
        }
        for &c in cue_for(&before, &after, self.pressed) {
            self.cue(c);
        }
    }

    /// Say why something asked for can't be done, with Halo 2's error
    /// sound.
    fn refuse(&mut self, text: String) {
        self.refused = true;
        self.toast(text);
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
        self.settings.server = server;
        self.settings.gamertag = gamertag;
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
                // A friend request to us is a message: Halo 2 sounds every
                // one (the notification screen, 0x2b740a).
                if is_asked_you(&text) {
                    self.cue(Cue::Notify);
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
        self.saved = [self.settings.gamertag.clone(), self.settings.server.clone()];
        let server = self.settings.server.clone();
        self.log(&format!("lobby: signed in to {server}"));
        // Halo 2's main menu loads the online screen by itself once signed
        // in (0x230ba6 calls the item's handler again), so it advances
        // (0x147f6d) with no press.
        self.cue(Cue::Advance);
    }

    /// Show `why` on the failure screen, once any match being played ends.
    /// On the start screen or the main menu (or the settings opened from
    /// it) the player stays there and `why` is a notice; ONLINE then
    /// signs in again.
    fn fail(&mut self, why: String) {
        self.log(&format!("lobby: {why}"));
        self.failed = why.clone();
        self.dialing = None;
        self.signing_in = None;
        let playing = self.game.as_ref().is_some_and(|g| g.child.is_some());
        if playing {
            return;
        }
        self.game = None;
        if self.away_from_live() {
            self.toast(why);
        } else {
            // Halo 2's error dialog loading by itself (0x1486b8 through
            // `dialog_ok`, then 0x147f6d), once.
            if self.screen != Screen::Failed {
                self.cue(Cue::Advance);
            }
            self.popup = None;
            self.screen = Screen::Failed;
        }
    }

    /// The start screen or the main menu is up.
    fn on_menus(&self) -> bool {
        matches!(self.screen, Screen::Start | Screen::Main)
    }

    /// The flythrough may be drawn: the start screen or main menu is up,
    /// or the intro before them (which wakes it ahead), and no match is
    /// on (its engine has the GPU to itself).
    fn scene_wanted(&self) -> bool {
        matches!(self.screen, Screen::Intro | Screen::Start | Screen::Main) && self.game.is_none()
    }

    /// While the intro plays, ask the flythrough for its first frame at
    /// the window's size (again only if the size changes): its GPU device
    /// opens and the frame is drawn behind the movie, so the start screen
    /// has the scene from its first frame, as Halo 2's did, and the fade
    /// is only for a scene that comes later.
    fn scene_warm_up(&mut self, w: usize, h: usize) {
        if self.scene_failed || self.scene_warm == Some((w, h)) || !self.scene_wanted() {
            return;
        }
        let Some(scene) = self.cfg.scene.clone() else {
            return;
        };
        // A failure is logged by the menus' first draw.
        if matches!(scene.state(), SceneState::Failed(_)) {
            return;
        }
        scene.ask(w, h, self.scene_seconds);
        self.scene_asleep = false;
        self.scene_warm = Some((w, h));
    }

    /// Tell the flythrough to let go of the GPU (once), and stop its
    /// clock until the menus are back.
    fn scene_sleep(&mut self) {
        self.scene_step = None;
        self.scene_shown = false;
        if self.scene_asleep {
            return;
        }
        self.scene_asleep = true;
        if let Some(s) = &self.cfg.scene {
            s.sleep();
        }
    }

    /// The flythrough's newest frame of the window's size `w` by `h`, and
    /// how far it has faded in (0 to 1), after moving its clock on to the
    /// menus' `clock` and asking for the next; None for the still
    /// background (no scene, not drawn yet, a frame of another size, or
    /// it failed, which is logged once).
    fn scene_frame(&mut self, w: usize, h: usize, clock: f64) -> Option<(Arc<Frame>, f32)> {
        if self.scene_failed || !self.scene_wanted() {
            return None;
        }
        let scene = self.cfg.scene.clone()?;
        // The intro's frame counts for the menus' first draw only.
        let warm = self.scene_warm.take();
        if let SceneState::Failed(why) = scene.state() {
            self.scene_failed = true;
            self.scene_since = None;
            self.scene_shown = false;
            self.log(&format!(
                "menus: the scene can't be drawn ({why}); the still background from now on"
            ));
            return None;
        }
        if let Some(last) = self.scene_step {
            self.scene_seconds += (clock - last).clamp(0.0, SCENE_STEP);
        }
        self.scene_step = Some(clock);
        scene.ask(w, h, self.scene_seconds);
        self.scene_asleep = false;
        // The newest frame is the last good one: until one of this size
        // comes (after the window's size changes), the still background.
        let Some(frame) = scene.latest(w, h).filter(|f| f.px.len() == w * h) else {
            self.scene_since = None;
            return None;
        };
        // Drawn during the intro, it is there as the screen opens: shown
        // whole at once, with no fade from the still background.
        let first = if warm == Some((w, h)) {
            clock - SCENE_FADE
        } else {
            clock
        };
        let since = *self.scene_since.get_or_insert(first);
        Some((frame, scene_alpha(clock - since)))
    }

    /// The player is on the start screen or the main menu, or in the
    /// settings opened from one: nowhere that needs the server, so losing
    /// it doesn't move them.
    fn away_from_live(&self) -> bool {
        self.on_menus()
            || (self.screen == Screen::Settings
                && matches!(self.settings_back, Screen::Start | Screen::Main))
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
            Screen::Main,
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

    /// What X (join) on friend `f`, whose party can't be joined, says, in
    /// the server's notices' style. The server's reasons (`can_join`)
    /// aren't all sent: the ONLINE list says whether the party is open and
    /// how much room it has, and anything else (removed from it, say, or
    /// no room for our guests) is "can't be joined".
    fn cannot_join(&self, f: &Friend) -> String {
        let tag = f.gamertag.to_uppercase();
        let theirs = self
            .view()
            .and_then(|v| v.online.iter().find(|o| o.account == f.account));
        match f.online {
            Online::Offline => return format!("{tag} IS OFFLINE"),
            Online::Game => return format!("{tag} IS ON ANOTHER VERSION OF THE GAME"),
            Online::Launcher => {}
        }
        if self.party().is_some_and(|pt| pt.id == f.party) {
            format!("{tag} IS ALREADY IN YOUR PARTY")
        } else if f.activity == Activity::Playing {
            format!("{tag}'S PARTY IS IN A MATCH")
        } else if theirs.is_some_and(|o| !o.open) {
            format!("{tag}'S PARTY IS INVITE ONLY")
        } else if theirs.is_some_and(|o| o.openings == 0) {
            format!("{tag}'S PARTY IS FULL")
        } else {
            format!("{tag}'S PARTY CAN'T BE JOINED")
        }
    }

    /// Y on a player: a friend request when one makes sense (`can_ask`),
    /// else why not, in the server's words (no words for ourselves).
    fn ask_friend_or_refuse(&mut self, account: u64, gamertag: &str) {
        if self.can_ask(account) {
            return self.ask_friend(gamertag);
        }
        let why = match self.friend(account).map(|f| f.relation) {
            Some(Relation::Friend) => h2net::live::ALREADY_FRIENDS,
            Some(Relation::WeAsked) => h2net::live::ALREADY_ASKED,
            Some(Relation::AskedUs) => h2net::live::ASKED_YOU,
            // Ourselves: only the error sound.
            None => {
                self.refused = true;
                return;
            }
        };
        let tag = gamertag.to_uppercase();
        self.refuse(h2net::live::friend_notice(why, &tag));
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
        self.cue(Cue::Select);
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

    /// The playlist the party panel and the party screen show levels for:
    /// the one the party searches while it searches, else the one selected
    /// on the playlists screen (None on the custom game row, and on the
    /// custom game screen, whatever row was selected before it).
    fn panel_playlist(&self) -> Option<&PlaylistInfo> {
        if self.screen == Screen::Custom {
            return None;
        }
        let v = self.view()?;
        if self.searching() {
            let id = v.party.as_ref()?.playlist;
            return v.playlists.iter().find(|p| p.id == id);
        }
        v.playlists.get(self.sel)
    }

    /// The level the party panel shows beside member `m` of party `pt`:
    /// their level in the panel's playlist when it's ranked, else their
    /// highest. Ours is in the playlists list. PARTY gives the others'
    /// only in the playlist the party last searched or played, so for
    /// another a service record kept (`ask_member_records`) gives it; with
    /// neither, 0 (drawn "-") until the record comes.
    fn member_level(&self, pt: &PartyInfo, m: &PartyMember) -> u8 {
        let Some(pl) = self.panel_playlist().filter(|p| p.ranked) else {
            return m.best.max(1);
        };
        if Some(m.account) == self.account() {
            return pl.level.max(1);
        }
        if pt.playlist == pl.id && m.level > 0 {
            return m.level;
        }
        match self.cached_record(m.account) {
            Some((r, _)) => r.found.as_ref().map_or(1, |f| {
                let played = f.playlists.iter().find(|p| p.playlist == pl.id);
                played.map_or(1, |p| p.level.max(1))
            }),
            None => 0,
        }
    }

    /// Ask for the service records the party panel needs for the levels
    /// PARTY doesn't give (`member_level`), on the playlists and party
    /// screens.
    fn ask_member_records(&mut self) {
        if !matches!(self.screen, Screen::Live | Screen::Party) || self.popup.is_some() {
            return;
        }
        let Some(id) = self.panel_playlist().filter(|p| p.ranked).map(|p| p.id) else {
            return;
        };
        let me = self.account();
        let wanted: Vec<u64> = self.party().map_or(Vec::new(), |pt| {
            let others = pt.members.iter().filter(|m| Some(m.account) != me);
            let unknown = others.filter(|m| pt.playlist != id || m.level == 0);
            unknown.map(|m| m.account).collect()
        });
        for account in wanted {
            self.ask_record(account);
        }
    }

    // ------------------------------------------------------------- match

    fn on_match(&mut self, m: LauncherMatch) {
        // The engine is about to start: the GPU is all its own.
        self.scene_sleep();
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
            g.left = None;
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
            left: None,
            menu: true,
            engine_controls: None,
        });
        self.popup = None;
        self.asked = None;
        self.screen = Screen::Pregame;
        // The pregame lobby loading (0x147f6d).
        self.cue(Cue::Advance);
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
                controls: self.settings.controls,
                game_kind: child::game_kind(&g.m),
            };
            match Running::start(&self.cfg.exe, &self.cfg.folder, &m, &self.cfg.engine_args) {
                Ok(c) => {
                    notes.push(format!(
                        "lobby: engine started for match {id:016x} ({} on {}, {})",
                        m.variant,
                        m.map,
                        if host { "hosting" } else { "joining" }
                    ));
                    g.menu = c.game_menu();
                    g.engine_controls = Some(c.launch_controls());
                    g.child = Some(c);
                    self.screen = Screen::InGame;
                }
                Err(e) => {
                    out.extend(g.told.next(id, host, None, Some(Engine::Closing)));
                    toast = Some(format!("The engine didn't start: {e}"));
                }
            }
        }
        // What the engine said, handled once the borrow of the child ends
        // (`on_engine_event` takes the whole lobby).
        let said = g.child.as_mut().map(|c| c.poll()).unwrap_or_default();
        for n in notes.drain(..) {
            self.log(&n);
        }
        for s in said {
            match s {
                Said::Event(e) => self.on_engine_event(&e, now),
                Said::Line(l) => self.log(&format!("engine: {l}")),
                Said::Done => {}
            }
        }
        let Some(g) = self.game.as_mut() else {
            self.send_all(out);
            return;
        };
        let mut exited = false;
        if let Some(c) = g.child.as_mut() {
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
        for n in &notes {
            self.log(n);
        }
        self.send_all(out);
        let mut carnage = false;
        if exited {
            carnage = self.on_engine_exit();
        } else if let Some(t) = toast {
            // The engine didn't start.
            self.screen = self.after_game_screen();
            self.game = None;
            self.toast(t);
        }
        if self.client.is_none() && self.game.as_ref().is_none_or(|g| g.child.is_none()) {
            // The server was lost during the match.
            self.game = None;
            let stay = [Screen::SignIn, Screen::Connecting];
            if !stay.contains(&self.screen) && !self.away_from_live() {
                // The failure screen loading (as in `fail`), unless the
                // carnage report's advance has just sounded for it.
                if self.screen != Screen::Failed && !carnage {
                    self.cue(Cue::Advance);
                }
                self.screen = Screen::Failed;
            }
        }
    }

    // ------------------------------------------------------------- intro

    /// Play the intro on to `now` (it starts at its first draw,
    /// `draw_intro`): its soundtrack goes to the sound when it comes, and
    /// at its end, or when it stalls or fails, the start screen comes in.
    fn play_intro(&mut self, now: Instant) {
        let Some(intro) = &mut self.intro else {
            return;
        };
        let u = intro.update(now);
        if let Some(clip) = u.sound {
            self.movie_sound.push(SoundOut::Movie(clip));
        }
        if let Some(end) = u.end {
            self.leave_intro(Some(end));
        }
    }

    /// Leave the intro for the start screen, whose intro animations start
    /// at its first draw: skipped (`None`), played out, or given up. Its
    /// soundtrack stops (a skip cuts it off, as in Halo 2).
    fn leave_intro(&mut self, end: Option<End>) {
        if self.screen != Screen::Intro {
            return;
        }
        let at = self.intro.as_ref().map_or(0.0, |i| i.clock(self.now));
        let (pictures, late) = self.intro.as_ref().map_or((0, 0), Intro::counts);
        let line = match end {
            None => format!("intro: skipped at {at:.1} s"),
            Some(End::Played) => format!(
                "intro: played to the end ({at:.1} s, {pictures} pictures, {late} late ones passed over)"
            ),
            Some(End::Stalled) => format!(
                "intro: the decoder sent nothing for {} s; skipped",
                intro::STALL.as_secs()
            ),
            Some(End::Failed(e)) => format!("intro: {e}; skipped"),
        };
        self.log(&line);
        self.intro = None;
        self.intro_kept = Kept::default();
        self.movie_sound.push(SoundOut::StopMovie);
        self.screen = Screen::Start;
    }

    /// Draw the intro: black, then the picture showing, as large as fits
    /// (`intro::letterbox`). Its first draw starts it, with the file for
    /// the window's height (`Plan::file`). Whether this draw drew a new
    /// picture (only those are timed: black while none has come, and a
    /// picture drawn again, cost next to nothing), or `None` when it
    /// couldn't start (the start screen is drawn instead).
    fn draw_intro(&mut self, c: &mut Canvas) -> Option<bool> {
        if self.intro.is_none() {
            let Some(plan) = self.cfg.intro.clone() else {
                self.leave_intro(Some(End::Failed("no movie".into())));
                return None;
            };
            let Some(path) = plan.file(c.h) else {
                self.leave_intro(Some(End::Failed("no movie file".into())));
                return None;
            };
            let name = path.file_name().unwrap_or(path.as_os_str());
            self.log(&format!(
                "intro: playing {} ({}x{} window)",
                name.to_string_lossy(),
                c.w,
                c.h
            ));
            let source = (plan.open)(path);
            match Intro::start(source, self.now) {
                Ok(i) => self.intro = Some(i),
                Err(e) => {
                    self.leave_intro(Some(End::Failed(e)));
                    return None;
                }
            }
        }
        let (w, h) = (c.w, c.h);
        self.scene_warm_up(w, h);
        let shown = self.intro.as_ref().and_then(Intro::showing);
        let Some((p, serial)) = shown else {
            c.px.fill(0);
            return Some(false);
        };
        let new = serial != self.intro_serial;
        if new {
            self.intro_serial = serial;
            self.intro_kept.forget();
        }
        let list = DrawList {
            size: [w as u32, h as u32],
            quads: vec![Quad {
                texture: Texture::Art(0),
                rect: intro::letterbox(p.w, p.h, w as f32, h as f32),
                uv: [0.0, 0.0, 1.0, 1.0],
                color: [[1.0; 4]; 2],
                blend: Blend::Plain,
                wrap: [false; 2],
            }],
            texts: Vec::new(),
        };
        self.intro_kept.draw(&list, &mut c.px, w, h, &MovieFrame(p));
        Some(new)
    }

    /// What the engine's launcher said about the engine: kept on the
    /// match, and told to the server as `Told` decides. Controls changed
    /// in the engine's GAME MENU are saved as the Settings screen saves
    /// them, for the next match.
    fn on_engine_event(&mut self, e: &Engine, now: Instant) {
        self.log(&format!("lobby: the engine says {}", live::event_line(e)));
        if let Engine::Controls { pairs, defaults } = e {
            self.on_engine_controls(pairs, *defaults);
        }
        let Some(g) = self.game.as_mut() else {
            return;
        };
        let (id, host) = (g.m.id, g.host());
        let mut note = None;
        match e {
            Engine::Running => {
                g.running = true;
                // Its window is about to open.
                if let Some(c) = &g.child {
                    c.let_to_front();
                }
            }
            Engine::MapLoaded => {
                g.running = true;
                g.loaded = true;
                if let Some(c) = &g.child {
                    c.let_to_front();
                }
            }
            // End Game before the server's GO: h2live hadn't started the
            // match, so its result isn't the game's (`Told` sends only
            // LEFT_MATCH), and the host gets "You ended the game." when
            // the engine closes rather than a carnage report.
            Engine::Ended(_) if g.told.ended_before_go() => {
                note = Some(
                    "lobby: the game was ended before the server started the match; \
                     its result isn't kept",
                );
            }
            Engine::Ended(players) => {
                g.results = Some(players.clone());
                g.ended_at = Some(now);
            }
            Engine::Left(k) => {
                g.left = Some(*k);
                note = Some(match k {
                    Leave::Game => "lobby: the player left the game from its game menu",
                    Leave::End => {
                        "lobby: the player ended the game for everyone from its game menu"
                    }
                });
            }
            Engine::MenuOff => {
                // Start goes to the game now: the in-game screen stops
                // saying it opens the game menu.
                g.menu = false;
                note = Some("lobby: the engine's game menu is off for the rest of the game");
            }
            Engine::Closing | Engine::Controls { .. } => {}
        }
        let out = g.told.next(id, host, None, Some(e.clone()));
        if let Some(n) = note {
            self.log(n);
        }
        self.send_all(out);
    }

    /// The GAME MENU's controls (`Engine::Controls`), saved through
    /// `set_control`: only the ones it changed from the controls the
    /// engine ran with, so a control flag a test gave the engine for one
    /// run (`H2LOBBY_ENGINE_ARGS`) isn't saved with them; every one for
    /// Use Default Settings (`defaults`), even those that flag already
    /// put at the default. With no engine, they are taken from the
    /// lobby's own.
    fn on_engine_controls(&mut self, pairs: &[(String, String)], defaults: bool) {
        let old = self
            .game
            .as_ref()
            .and_then(|g| g.engine_controls)
            .unwrap_or(self.settings.controls);
        let mut new = old;
        for (k, v) in pairs {
            let taken = match new.set(k, v) {
                Ok(true) => Ok(()),
                Ok(false) => Err(format!("no control called {k:?}")),
                Err(why) => Err(why),
            };
            if let Err(why) = taken {
                self.log(&format!(
                    "lobby: the game menu's {k} = {v} wasn't taken: {why}"
                ));
            }
        }
        for ((k, v), (_, was)) in new.pairs().into_iter().zip(old.pairs()) {
            if defaults || v != was {
                if let Err(why) = self.set_control(&k, &v) {
                    self.log(&format!("lobby: {k} = {v} wasn't saved: {why}"));
                }
            }
        }
        if let Some(g) = self.game.as_mut() {
            g.engine_controls = Some(new);
        }
        let c = self.settings.controls.describe();
        self.log(&format!("lobby: controls from the game menu: {c}"));
    }

    /// The engine's copy exited: on to the carnage report if its game
    /// ended (End Game's too: its result came as the host's engine quit,
    /// unless that was before the server's GO, `Told::ended_before_go`),
    /// otherwise back where a player lands after a game, with why (the
    /// server's reason, or that they left or ended it: a leave from the
    /// GAME MENU wins over a verdict with no reason, which the server
    /// gives when the game ended without them, as when the host's engine
    /// ended a Team Slayer game whose team emptied, PC check 13). Either
    /// way the lobby's window comes to the front.
    ///
    /// True when the carnage report comes in (its loading sound plays).
    fn on_engine_exit(&mut self) -> bool {
        // The lobby's own "leave the game?" has nothing left to ask once
        // the engine is gone (the player left from the GAME MENU, or the
        // game ended), and it would cover the screen landed on.
        if self.popup == Some(Popup::LeaveGame) {
            self.popup = None;
        }
        let Some(g) = self.game.as_mut() else {
            return false;
        };
        g.child = None;
        let (id, host) = (g.m.id, g.host());
        let out = if g.told.over() {
            Vec::new()
        } else {
            g.told.next(id, host, None, Some(Engine::Closing))
        };
        self.focus = true;
        let toast = if g.results.is_some() {
            self.screen = Screen::Carnage;
            // Start on this PC's player.
            let me = g.m.relay.id;
            self.csel = carnage_rows(g)
                .iter()
                .position(|r| r.relay_id == me)
                .unwrap_or(0);
            None
        } else {
            let why = g.over.as_ref().map(|o| o.reason.clone());
            Some(match (g.left, why) {
                (Some(Leave::End), _) => "You ended the game.".to_string(),
                (_, Some(w)) if !w.is_empty() => w,
                (Some(Leave::Game), _) => "You left the game.".into(),
                (_, Some(_)) => "The match was called off.".into(),
                (_, None) => "You left the game.".into(),
            })
        };
        self.log("lobby: the engine closed");
        self.send_all(out);
        let Some(t) = toast else {
            // The carnage report loading (0x147f6d).
            self.cue(Cue::Advance);
            return true;
        };
        self.screen = self.after_game_screen();
        self.game = None;
        self.toast(t);
        false
    }

    // -------------------------------------------------------------- time

    /// Take on what the server and the engine said, and start what is due.
    pub fn tick(&mut self, now: Instant) {
        self.now = now;
        self.play_intro(now);
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
        // The playlists, then the custom game and settings rows.
        let n = self.view().map(|v| v.playlists.len()).unwrap_or(0);
        self.sel = self.sel.min(n + 1);
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
        self.ask_member_records();
        self.music_on = !self.engine_running() && music_rule(self.screen, self.music_on);
        if !self.scene_wanted() {
            self.scene_sleep();
        }
        // A new invitation on show sounds once. One that has gone from the
        // list (declined, accepted, taken back) is forgotten, so the same
        // party inviting again sounds again; one only hidden (a popup, a
        // screen without invitations) is kept, and doesn't.
        let seen_gone = self.invite_seen.is_some_and(|seen| {
            !self
                .view()
                .is_some_and(|v| v.invites.iter().any(|(party, _)| *party == seen))
        });
        if seen_gone {
            self.invite_seen = None;
        }
        let shown = self.invite().map(|(party, _)| party);
        if shown.is_some() && shown != self.invite_seen {
            self.cue(Cue::Notify);
        }
        if shown.is_some() {
            self.invite_seen = shown;
        }
        self.follow_dialog();
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
        // The dialog's focus with it (`dialogs`).
        self.focus_dialog(self.osel);
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

    /// A key pressed in the lobby's window (the window's keyboard events
    /// come here; the controller's go to `pad`, the wheel's to `wheel`,
    /// clicks to `click`, and the headless script's presses straight to
    /// `input`). The legends name keys from now on.
    pub fn key(&mut self, i: Input) {
        self.keyboard = true;
        if self.keys_are_the_games(i, self.engine_running()) {
            return;
        }
        self.input(i)
    }

    /// A press from the controller: the legends show its buttons from now
    /// on.
    pub fn pad(&mut self, i: Input) {
        self.keyboard = false;
        self.input(i)
    }

    /// The mouse wheel turned a notch, up or down: a press of Up or Down,
    /// and the legends name keys from now on.
    pub fn wheel(&mut self, up: bool) {
        self.keyboard = true;
        self.input(if up { Input::Up } else { Input::Down })
    }

    /// A key the window repeats while it's held down: the arrows and
    /// typing repeat, but not a press that is a button (Enter, Esc,
    /// PageUp and PageDown, or a letter or Backspace outside a field), as
    /// Halo 2 repeats only directions. So a key held to skip the intro
    /// skips only the movie, and a key held past a popup opening never
    /// answers it.
    pub fn key_repeated(&mut self, i: Input) {
        let button = match i {
            Input::A | Input::B | Input::Lb | Input::Rb => true,
            Input::Backspace | Input::Char(_) => !self.typing(),
            _ => false,
        };
        if !button {
            self.key(i);
        }
    }

    /// A key that belongs to the game rather than the lobby: on the
    /// in-game screen the letters are the game's keys typed into the
    /// wrong window (Space jumps there, and is A here), and while the
    /// engine plays so are Enter, Esc and Backspace (A and B here; Esc is
    /// Halo 2's pause) on that screen and in its Leave Game popup, so
    /// leaving the game takes a click.
    fn keys_are_the_games(&self, i: Input, playing: bool) -> bool {
        let in_game = self.screen == Screen::InGame || self.popup == Some(Popup::LeaveGame);
        match i {
            Input::Char(_) => in_game,
            Input::A | Input::B | Input::Backspace => in_game && playing,
            _ => false,
        }
    }

    /// A press from the controller, a key (`key`), a click on a button or
    /// the headless script, with its sound (`sounding`).
    pub fn input(&mut self, i: Input) {
        self.sounding(|a| a.input_inner(i));
    }

    fn input_inner(&mut self, i: Input) {
        // The intro: any button skips it (the pad's, a key, a letter), but
        // not a direction (the d-pad, the stick, the arrows), as in Halo 2.
        if self.screen == Screen::Intro {
            let direction = matches!(i, Input::Up | Input::Down | Input::Left | Input::Right);
            if !direction && i != Input::Tab {
                self.leave_intro(None);
            }
            return;
        }
        // The triggers do nothing anywhere else: no press, no sound.
        if matches!(i, Input::Lt | Input::Rt) {
            return;
        }
        // Letters are buttons, except where a name is typed; Backspace is
        // B where it doesn't rub a letter out.
        let typing = self.typing();
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
            Input::Backspace if !typing => Input::B,
            other => other,
        };
        self.pressed = Some(i);
        if let Some(p) = self.popup {
            return self.popup_input(p, i);
        }
        if let Some((party, from)) = self.invite() {
            return self.invite_input(party, &from, i);
        }
        match self.screen {
            Screen::Start => match i {
                Input::A => {
                    self.screen = Screen::Main;
                    // The press, the main menu loading (0x147f6d), then
                    // Halo 2's start screen going on to it (sound 5,
                    // 0x22f28c).
                    self.cue(Cue::Select);
                    self.cue(Cue::Advance);
                    self.cue(Cue::Transport);
                }
                Input::B => self.popup = Some(Popup::Quit),
                _ => {}
            },
            Screen::Main => self.main_input(i),
            Screen::SignIn => self.sign_in_input(i),
            Screen::Connecting => {
                if i == Input::B {
                    self.log("lobby: signing in called off");
                    self.dialing = None;
                    self.client = None;
                    self.signing_in = None;
                    self.screen = Screen::Main;
                }
            }
            Screen::Live => self.live_input(i),
            Screen::Players => self.players_input(i),
            Screen::Party => self.party_input(i),
            Screen::Custom => self.custom_input(i),
            // Taken above.
            Screen::Intro | Screen::Pregame => {}
            Screen::InGame => {
                let ended = self.game.as_ref().is_some_and(|g| g.results.is_some());
                if i == Input::B && !ended {
                    self.popup = Some(Popup::LeaveGame);
                }
            }
            Screen::Carnage => self.carnage_input(i),
            Screen::Record => self.record_input(i),
            Screen::Friends => self.friends_input(i),
            Screen::Settings => self.settings_input(i),
            Screen::Failed => self.failed_input(i),
        }
    }

    /// The invitation from `party` (`from`'s) as its presses were before
    /// it had rows (`dialogs::invite_input`): A accepts (from the main
    /// menu going on to the playlists), B declines.
    fn invite_press(&mut self, party: u64, from: &str, i: Input) {
        match i {
            Input::A => {
                self.log(&format!("lobby: joining {from}'s party"));
                self.send(ToServer::Accept(party));
                // From the main menu, to the playlists, where the new
                // party shows (as if ONLINE had been picked).
                if self.screen == Screen::Main {
                    self.main_focus = Focus::on(LIVE_ROW, self.menu_clock());
                    self.screen = Screen::Live;
                }
            }
            Input::B => self.send(ToServer::Decline(party)),
            _ => {}
        }
    }

    /// The offline screen's presses as they were before it had rows
    /// (`dialogs::failed_input`): A tries again, X the controller
    /// settings, Y the sign-in form, B the main menu.
    fn failed_press(&mut self, i: Input) {
        match i {
            Input::A => self.connect(),
            Input::X => self.open_settings(),
            Input::Y => {
                self.form_error = None;
                self.screen = Screen::SignIn;
            }
            Input::B => self.screen = Screen::Main,
            _ => {}
        }
    }

    /// Text is typed where the screen or popup has a field with the
    /// cursor: there letters aren't buttons, and Backspace rubs out.
    fn typing(&self) -> bool {
        match self.popup {
            None => self.screen == Screen::SignIn,
            Some(Popup::AddFriend) => true,
            Some(_) => false,
        }
    }

    fn main_input(&mut self, i: Input) {
        let row = self.main_focus.item;
        match i {
            Input::Up => self.focus_main(row.saturating_sub(1)),
            Input::Down => self.focus_main((row + 1).min(MAIN_ROWS.len() - 1)),
            Input::A => self.pick_main(row),
            Input::B => self.screen = Screen::Start,
            _ => {}
        }
    }

    /// Move the main menu's focus to `row`, with the skin's fades.
    fn focus_main(&mut self, row: usize) {
        let look = &self.cfg.menus.shell.main.list.skin.items;
        self.main_focus = self.main_focus.moved_in(look, row, self.menu_clock());
    }

    fn pick_main(&mut self, row: usize) {
        match row {
            LIVE_ROW => self.online(),
            MAIN_SETTINGS_ROW => self.open_settings(),
            _ => {}
        }
    }

    /// ONLINE on the main menu: the playlists when signed in, the
    /// signing-in screen while that goes on, else sign in with the saved
    /// gamertag and server (the form is put back to them, whatever was
    /// typed there and abandoned), or the sign-in form when there is none.
    fn online(&mut self) {
        let welcomed = self
            .client
            .as_ref()
            .is_some_and(|c| c.view.welcome.is_some());
        if welcomed {
            self.screen = Screen::Live;
        } else if self.dialing.is_some() || self.signing_in.is_some() {
            self.screen = Screen::Connecting;
        } else if !self.saved[0].is_empty() {
            self.fields = self.saved.clone();
            self.connect();
        } else {
            self.field = 0;
            self.form_error = None;
            self.screen = Screen::SignIn;
        }
    }

    /// The menus' clock: seconds since the lobby opened.
    fn menu_clock(&self) -> f64 {
        self.clock.elapsed().as_secs_f64()
    }

    /// A popup's presses as they were before its dialog had rows
    /// (`dialogs::popup_input`).
    fn popup_press(&mut self, p: Popup, i: Input) {
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
                if let Some(p) = picked {
                    self.ask_friend_or_refuse(p.account, &p.gamertag);
                }
            }
            Input::A | Input::B => {
                self.screen = self.after_game_screen();
                self.game = None;
            }
            _ => {}
        }
    }

    /// Where the player lands after a game: the custom game screen for
    /// the leader of a custom game's party (they pick the next), otherwise
    /// the playlists.
    fn after_game_screen(&self) -> Screen {
        let custom = self
            .game
            .as_ref()
            .is_some_and(|g| g.m.playlist == CUSTOM_GAME);
        if custom && self.leader() {
            Screen::Custom
        } else {
            Screen::Live
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
                self.cue(Cue::Select);
            }
            (Input::X, Some(f)) if f.relation == Relation::AskedUs => {
                self.log(&format!("lobby: declining {}'s friend request", f.gamertag));
                self.send(ToServer::FriendDecline(f.account));
                self.cue(Cue::Select);
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
            (Input::X, Some(f)) if f.relation == Relation::Friend => {
                let why = self.cannot_join(&f);
                self.refuse(why);
            }
            (Input::X, Some(f)) if f.relation == Relation::WeAsked => {
                self.log(&format!(
                    "lobby: taking back the friend request to {}",
                    f.gamertag
                ));
                self.send(ToServer::FriendRemove(f.account));
                self.cue(Cue::Select);
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
            // The controller's X: the letter x is typed.
            Input::X => self.open_settings(),
            Input::B => self.screen = Screen::Main,
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
                    self.cue(Cue::Back);
                } else {
                    self.refuse("Only the party leader can stop the search.".into());
                }
            }
            return;
        }
        // The playlists, then the custom game and settings rows.
        let n = self.view().map(|v| v.playlists.len()).unwrap_or(0);
        match i {
            Input::Up => self.sel = self.sel.saturating_sub(1),
            Input::Down => self.sel = (self.sel + 1).min(n + 1),
            Input::A if self.sel == n + 1 => self.open_settings(),
            Input::A if self.sel == n => {
                if !self.leader() {
                    self.refuse("Only the party leader can start a custom game.".into());
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
                    self.refuse("Only the party leader can pick a playlist.".into());
                    return;
                }
                self.log(&format!("lobby: searching {name:?} (playlist {id})"));
                if let Some(c) = &mut self.client {
                    c.view.status = None;
                }
                self.send(ToServer::Search(id));
                // The press, then Halo 2's matchmaking screen loading
                // (here the search shows on this screen).
                self.cue(Cue::Select);
                self.cue(Cue::Advance);
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
            // Back to the main menu, still signed in and in the party.
            Input::B => self.screen = Screen::Main,
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
                self.cue(Cue::Select);
            }
            (Input::X, Some((who, false))) if leader => self.popup = Some(Popup::Kick(who)),
            (Input::Lb, Some((who, _))) => self.open_record(who),
            (Input::Y, _) if leader => {
                let privacy = match self.party().map(|p| p.privacy) {
                    Some(Privacy::Open) => Privacy::InviteOnly,
                    _ => Privacy::Open,
                };
                self.send(ToServer::Privacy(privacy));
                self.cue(Cue::Select);
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
            self.refuse("No Halo 2 maps were found in MCC's folder.".into());
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
        // Halo 2's START GAME is a button: its press only (no screen
        // loads until the match comes, `on_match`).
        self.cue(Cue::Select);
    }

    // ---------------------------------------------------------- settings

    /// The controller settings screen, on its first row.
    fn open_settings(&mut self) {
        if self.screen != Screen::Settings {
            self.settings_back = self.screen;
        }
        self.srow = 0;
        self.screen = Screen::Settings;
    }

    /// An engine is running for the match.
    pub fn engine_running(&self) -> bool {
        self.game.as_ref().is_some_and(|g| g.child.is_some())
    }

    /// Controller presses are for the lobby, except while the engine
    /// plays: its window may not be in front, and B then A here would
    /// leave the game.
    pub fn wants_pad(&self) -> bool {
        self.pad_is_the_lobbys(self.engine_running())
    }

    fn pad_is_the_lobbys(&self, playing: bool) -> bool {
        !(self.screen == Screen::InGame && playing)
    }

    fn save_settings(&mut self) {
        let path = self.cfg.folder.join("lobby.txt");
        if let Err(e) = self.settings.save(&path) {
            self.log(&format!("lobby: {e}"));
        }
    }

    /// Change a control by its `lobby.txt` name and save it, as the
    /// settings screen would (for the headless script's `set`).
    pub fn set_control(&mut self, key: &str, value: &str) -> Result<(), String> {
        if !self.settings.controls.set(key, value)? {
            return Err(format!("no control called {key:?}"));
        }
        self.save_settings();
        Ok(())
    }

    /// Move the value on row `row` on (`forward`) or back. Numbers stop at
    /// their ends unless `wrap` (A goes round).
    fn step_setting(&mut self, row: usize, forward: bool, wrap: bool) {
        let c = &mut self.settings.controls;
        match row {
            STICKS_ROW => c.sticks = c.sticks.next(forward),
            BUTTONS_ROW => c.buttons = c.buttons.next(forward),
            2 => {
                let (lo, hi) = (
                    controls::LOOK_SENSITIVITY_MIN,
                    controls::LOOK_SENSITIVITY_MAX,
                );
                let v = c.look_sensitivity;
                c.look_sensitivity = match (forward, wrap) {
                    (true, true) if v >= hi => lo,
                    (false, true) if v <= lo => hi,
                    (true, _) => (v + 1).min(hi),
                    (false, _) => v.saturating_sub(1).max(lo),
                };
            }
            3 => c.look_inverted = !c.look_inverted,
            4 => c.auto_center = !c.auto_center,
            5 => c.vibration = !c.vibration,
            MOUSE_ROW => {
                let (lo, hi) = (
                    controls::MOUSE_SENSITIVITY_MIN,
                    controls::MOUSE_SENSITIVITY_MAX,
                );
                let v = c.mouse_sensitivity;
                let step = controls::MOUSE_SENSITIVITY_STEP;
                c.mouse_sensitivity = match (forward, wrap) {
                    (true, true) if v >= hi - step / 2.0 => lo,
                    (false, true) if v <= lo + step / 2.0 => hi,
                    (true, _) => controls::clean_mouse_sensitivity(v + step),
                    (false, _) => controls::clean_mouse_sensitivity(v - step),
                };
            }
            MOUSE_INVERT_ROW => c.mouse_inverted = !c.mouse_inverted,
            _ => return,
        }
        self.save_settings();
    }

    fn settings_input(&mut self, i: Input) {
        let last = SETTINGS_ROWS.len() - 1;
        match i {
            Input::Up => self.srow = self.srow.saturating_sub(1),
            Input::Down => self.srow = (self.srow + 1).min(last),
            Input::Left | Input::Right => self.step_setting(self.srow, i == Input::Right, false),
            Input::A if self.srow == RESTORE_ROW => {
                self.settings.controls = Controls::default();
                self.save_settings();
                self.toast("Controller settings are back to Halo 2's defaults.".into());
            }
            Input::A => self.step_setting(self.srow, true, true),
            Input::B => {
                let c = self.settings.controls.describe();
                self.log(&format!("lobby: controls: {c}"));
                self.screen = self.settings_back;
            }
            _ => {}
        }
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
                if let Some(p) = picked {
                    self.ask_friend_or_refuse(p.account, &p.gamertag);
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
                    self.cue(Cue::Select);
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

    /// A click at window pixel (`x`, `y`), with its sound (`sounding`).
    /// The legends name keys from now on.
    pub fn click(&mut self, x: f32, y: f32) {
        self.keyboard = true;
        self.sounding(|a| a.click_inner(x, y));
    }

    fn click_inner(&mut self, x: f32, y: f32) {
        let clock = self.menu_clock();
        self.follow_dialog();
        let over = self.over_key();
        let before = self.last_click.replace((clock, self.screen, over));
        // The second half of a double-click whose first half changed the
        // screen or shut a popup: it mustn't go on again.
        let second = before.is_some_and(|(t, screen, was)| {
            clock - t < DOUBLE_CLICK && (was.is_some() || screen != self.screen)
        });
        // A click anywhere skips the intro (and its double-click's second
        // half doesn't go on from the start screen: above).
        if self.screen == Screen::Intro {
            return self.leave_intro(None);
        }
        // The start screen goes on with a click anywhere, and a main menu
        // row is focused, or picked if it had the focus. A popup over
        // them takes the click as on any screen.
        if over.is_none() {
            match self.screen {
                Screen::Start if second => return,
                Screen::Start => return self.input_inner(Input::A),
                Screen::Main => {
                    if let Some(row) = self.main_row_at(x, y) {
                        if row == self.main_focus.item && !second {
                            // Picked as A picks the focused row, and
                            // sounding as A.
                            self.input_inner(Input::A);
                        } else {
                            self.focus_main(row);
                        }
                    }
                    return;
                }
                _ => {}
            }
        }
        // Elsewhere, the second half of a double-click whose first half
        // changed the screen, or shut a popup or put another in its place,
        // lands on what is shown now (the legend's BACK is in the same
        // place on most screens): a button or an arrow there isn't
        // pressed, and a row (a dialog's too) is only selected. Two clicks
        // in a popup that is still up both count.
        let went_on = before.is_some_and(|(t, screen, was)| {
            clock - t < DOUBLE_CLICK && (screen != self.screen || (was.is_some() && was != over))
        });
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
            Some(Hit::Press(_) | Hit::Step(..)) if went_on => {}
            Some(Hit::Press(i)) => self.input_inner(i),
            Some(Hit::Field(f)) => self.field = f,
            Some(Hit::Step(r, on)) => {
                if self.screen == Screen::Settings {
                    self.srow = r;
                } else {
                    self.crow = r;
                }
                self.input_inner(if on { Input::Right } else { Input::Left });
            }
            Some(Hit::Choice(k)) => self.choose(k, went_on),
            Some(Hit::Row(r)) if self.screen == Screen::Friends => {
                if self.fsel == r && !went_on {
                    self.input_inner(Input::A);
                } else {
                    self.select_friend(r);
                }
            }
            Some(Hit::Row(r)) if self.screen == Screen::Players => {
                if self.psel == r && !went_on {
                    self.input_inner(Input::A);
                } else {
                    self.select_player(r);
                }
            }
            Some(Hit::Row(r)) if self.screen == Screen::Settings => {
                if self.srow == r && !went_on {
                    self.input_inner(Input::A);
                } else {
                    self.srow = r;
                }
            }
            // A row of the carnage report is only selected: A continues.
            Some(Hit::Row(r)) if self.screen == Screen::Carnage => self.csel = r,
            Some(Hit::Row(r)) if self.screen == Screen::Custom => {
                if self.crow == r && r == CUSTOM_ROWS - 1 && !went_on {
                    self.input_inner(Input::A);
                } else {
                    self.crow = r;
                }
            }
            Some(Hit::Row(r)) => {
                let sel = match self.screen {
                    Screen::Party => &mut self.msel,
                    _ => &mut self.sel,
                };
                if *sel == r && !went_on {
                    self.input_inner(Input::A);
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

    /// The main menu's row under window pixel (`x`, `y`), at the size it
    /// was last drawn.
    fn main_row_at(&self, x: f32, y: f32) -> Option<usize> {
        let menus = &self.cfg.menus;
        let (w, h) = self.size;
        let space = Space::new(w as u32, h as u32);
        let m = self.main_menu(&menus.shell, "");
        screens::main_menu_row_at(&m, space, [x, y])
    }

    /// The main menu's state for `h2ui`.
    fn main_menu<'a>(&self, shell: &'a Shell, small_print: &'a str) -> MainMenu<'a> {
        MainMenu {
            layout: &shell.main,
            opened: self.menu_opened,
            rows: &MAIN_ROWS,
            focus: self.main_focus,
            small_print,
        }
    }

    /// The main menu's small print, at the bottom right: the gamertag
    /// signed in, else the one saved, else nothing.
    fn small_print(&self) -> String {
        let signed_in = self.client.as_ref().and_then(|c| c.view.welcome.as_ref());
        match signed_in {
            Some(w) => w.gamertag.clone(),
            None => self.saved[0].clone(),
        }
    }

    /// Draw the screen into `c`: the intro, the start screen or main menu
    /// (`draw_menu`), or another screen in Halo 2's shell (`draw_shell`);
    /// then over it Halo 2's dialog for a popup, the invitation or the
    /// offline screen (`draw_dialog`), then ADD FRIEND's box and the
    /// notices (`draw_over`). The first frames
    /// on each screen at each window size are timed for the log
    /// (`time_frame`): the whole draw here, the start screen's and main
    /// menu's in `draw_menu`.
    pub fn draw(&mut self, c: &mut Canvas, t: &mut Text) {
        let started = Instant::now();
        let intro = match self.screen {
            Screen::Intro => self.draw_intro(c),
            _ => None,
        };
        if let Some(new_picture) = intro {
            self.areas.clear();
            self.drawn.clear();
            self.drawn_screen = Some(Screen::Intro);
            let frame_ms = started.elapsed().as_secs_f64() * 1000.0;
            // Only a new picture's draw is timed (`draw_intro`).
            self.time_frame((c.w, c.h), frame_ms, new_picture, None);
            return;
        }
        let (w, h) = (c.w as f32, c.h as f32);
        let s = (w / 1280.0).min(h / 720.0).max(0.01);
        let (ox, oy) = ((w - 1280.0 * s) / 2.0, (h - 720.0 * s) / 2.0);
        let mut drawn = Vec::new();
        if self.drawn_screen != Some(self.screen) {
            self.drawn_screen = Some(self.screen);
            if self.on_menus() {
                // It opened: its intro animations start now.
                self.menu_opened = self.menu_clock();
                self.main_focus = Focus::on(self.main_focus.item, self.menu_opened);
            }
        }
        let menus = self.on_menus();
        let mut areas = Vec::new();
        // A frame whose still background was drawn afresh isn't timed.
        let mut timed = true;
        if menus {
            // Halo 2's own screen.
            self.draw_menu(c, &mut drawn);
        } else {
            timed = self.draw_shell(c, t, (s, ox, oy), &mut areas, &mut drawn);
        }
        // Halo 2's dialog over it (and the overlay under ADD FRIEND's box).
        self.draw_dialog(c, (s, ox, oy), &mut areas, &mut drawn);
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
            legend: Vec::new(),
        };
        self.draw_over(&mut p);
        self.areas = areas;
        self.drawn = drawn;
        self.view = (s, ox, oy);
        if !menus {
            let frame_ms = started.elapsed().as_secs_f64() * 1000.0;
            self.time_frame((c.w, c.h), frame_ms, timed, None);
        }
    }

    /// The start screen or the main menu, drawn by `h2ui` over the
    /// flythrough's newest frame (`scene_frame`) or, without one, the
    /// still background (drawn once for a window size), which the
    /// flythrough's first frame fades in over. The strings shown are put
    /// in `drawn`. The first frames at each size are timed for the log.
    fn draw_menu(&mut self, c: &mut Canvas, drawn: &mut Vec<String>) {
        let started = Instant::now();
        let clock = self.menu_clock();
        let menus = self.cfg.menus.clone();
        let (shell, fonts) = (&menus.shell, &menus.fonts);
        let art = shell.art();
        let res = Resources { art, fonts };
        let (w, h) = (c.w, c.h);
        self.size = (w, h);
        let space = Space::new(w as u32, h as u32);
        let framing = &shell.main.framing;
        let scene = self.scene_frame(w, h, clock);
        self.scene_shown = scene.is_some();
        let mut fresh = false;
        let mut copy_ms = None;
        match &scene {
            Some((frame, alpha)) if *alpha >= 1.0 => {
                // The flythrough in place of the still background (with
                // `Backdrop::Scene` the background's list is empty).
                let copied = Instant::now();
                c.px.copy_from_slice(&frame.px);
                copy_ms = Some(copied.elapsed().as_secs_f64() * 1000.0);
            }
            _ => {
                let behind =
                    screens::background(clock, Backdrop::Still, framing, art, fonts, space);
                // Drawn again only at a new size (`Kept`): that frame isn't
                // timed, and its time is the background's.
                fresh = self.menu_times.kept != (w, h);
                self.backdrop.draw(&behind, &mut c.px, w, h, &res);
                let background_ms = started.elapsed().as_secs_f64() * 1000.0;
                if fresh {
                    let t = &mut self.menu_times;
                    t.kept = (w, h);
                    if !t.backgrounds.iter().any(|b| b.0 == (w, h)) {
                        t.backgrounds.push(((w, h), background_ms));
                    }
                }
                if let Some((frame, alpha)) = &scene {
                    // The flythrough fading in over it.
                    blend_px(&mut c.px, &frame.px, *alpha);
                }
            }
        }
        drop(scene);
        let small_print = self.small_print();
        // Its text counts as drawn (for `see`) once it has faded in.
        let shown = clock - self.menu_opened >= self.menu_intro();
        let list = match self.screen {
            Screen::Start => {
                let start = Start {
                    layout: &shell.start,
                    opened: self.menu_opened,
                    text: screens::PRESS_START,
                    small_print: "",
                };
                if shown {
                    drawn.push(screens::PRESS_START.to_string());
                }
                screens::start(clock, &start, art, fonts, space)
            }
            _ => {
                let m = self.main_menu(shell, &small_print);
                if shown {
                    drawn.extend(MAIN_ROWS.iter().map(|r| r.to_string()));
                    if !small_print.is_empty() {
                        drawn.push(small_print.clone());
                    }
                }
                screens::main_menu(clock, &m, art, fonts, space)
            }
        };
        h2ui::cpu::draw_u32(&list, &mut c.px, w, h, &res);
        let frame_ms = started.elapsed().as_secs_f64() * 1000.0;
        self.time_frame((w, h), frame_ms, !fresh, copy_ms);
    }

    /// How long (seconds) the start screen or main menu on show takes to
    /// come in after it opens: the end of the last intro animation of its
    /// pictures and text, and on the main menu of its list and the
    /// focused row's fade (each with its delay).
    fn menu_intro(&self) -> f64 {
        let ends = |intro: Option<&Animation>, delay_ms: f32| {
            f64::from(delay_ms.max(0.0) / 1000.0 + intro.map_or(0.0, Animation::seconds))
        };
        let pictures = |art: &[BitmapWidget]| {
            art.iter()
                .map(|b| ends(b.intro.as_ref(), b.delay_ms))
                .fold(0.0, f64::max)
        };
        let shell = &self.cfg.menus.shell;
        match self.screen {
            Screen::Start => {
                let t = &shell.start.press_start;
                pictures(&shell.start.art).max(ends(t.intro.as_ref(), t.delay_ms))
            }
            _ => {
                let list = &shell.main.list;
                let gain = f64::from(list.skin.items.gain.seconds());
                pictures(&shell.main.art)
                    .max(pictures(&list.skin.bitmaps))
                    .max(ends(list.intro.as_ref(), list.delay_ms))
                    .max(gain)
            }
        }
    }

    /// The start screen or main menu has come in and is worth a picture
    /// (always true on the other screens): its intro is over, and on the
    /// start screen PRESS START is back at the top of its pulse; and a
    /// dialog up (`dialogs`) has come in too. The headless run waits for
    /// it before its automatic screenshot.
    pub fn menu_settled(&self) -> bool {
        // A dialog up waits for its pieces to come in (`dialogs`).
        if !self.dialog_settled() {
            return false;
        }
        if !self.on_menus() {
            return true;
        }
        if self.drawn_screen != Some(self.screen) {
            return false;
        }
        let mut at = self.menu_intro();
        if self.screen == Screen::Start && self.cfg.menus.shell.start.press_start.pulsating {
            let pulse = f64::from(h2ui::anim::PULSE_SECONDS);
            at = (at / pulse).ceil().max(1.0) * pulse;
        }
        self.menu_clock() - self.menu_opened >= at
    }

    /// The screen's name in the per-screen timer and the log.
    fn timer_name(&self) -> &'static str {
        match self.screen {
            Screen::Intro => "intro",
            Screen::Start => "start screen",
            Screen::Main => "main menu",
            Screen::SignIn => "sign-in",
            Screen::Connecting => "signing in",
            Screen::Live if self.searching() => "searching",
            Screen::Live => "playlists",
            Screen::Players => "players",
            Screen::Party => "party",
            Screen::Custom => "custom game",
            Screen::Pregame => "pregame lobby",
            Screen::InGame => "in game",
            Screen::Carnage => "carnage report",
            Screen::Failed => "offline",
            Screen::Record => "service record",
            Screen::Friends => "friends",
            Screen::Settings => "settings",
        }
    }

    /// Count a frame's time on the screen shown (`timer_name`) at window
    /// size `size`, unless it isn't `timed`, and log the average once its
    /// first `MENU_FRAMES` there are timed: once per screen and size. The
    /// first frame after the size changes isn't timed either. `copy_ms` is
    /// the part of it spent copying the flythrough's frame in, if it was;
    /// a screen's line gives the average of its own copies only.
    fn time_frame(
        &mut self,
        size: (usize, usize),
        frame_ms: f64,
        timed: bool,
        copy_ms: Option<f64>,
    ) {
        let name = self.timer_name();
        let menus = self.on_menus();
        let t = &mut self.menu_times;
        if t.size != size {
            t.size = size;
            t.frames.clear();
            return;
        }
        if !timed || t.logged.contains(&(size, name)) {
            return;
        }
        let k = match t.frames.iter().position(|f| f.name == name) {
            Some(k) => k,
            None => {
                t.frames.push(ScreenTimes {
                    name,
                    frames: 0,
                    total: 0.0,
                    copies: 0,
                    copy_total: 0.0,
                });
                t.frames.len() - 1
            }
        };
        let f = &mut t.frames[k];
        f.frames += 1;
        f.total += frame_ms;
        if let Some(ms) = copy_ms {
            f.copies += 1;
            f.copy_total += ms;
        }
        if f.frames != MENU_FRAMES {
            return;
        }
        let average = f.total / f64::from(MENU_FRAMES);
        let copies = (f.copies, f.copy_total);
        t.logged.push((size, name));
        let (w, h) = size;
        let line = if menus {
            let background = t.backgrounds.iter().find(|b| b.0 == size).map(|b| b.1);
            let mut parts = Vec::new();
            if copies.0 > 0 {
                let copy = copies.1 / f64::from(copies.0);
                parts.push(format!("the scene's frame copied in {copy:.1} ms"));
            }
            if background.is_some() || parts.is_empty() {
                let background = background.unwrap_or(0.0);
                parts.push(format!("background {background:.0} ms once"));
            }
            format!(
                "menus: {w}x{h}, {name} {average:.1} ms a frame on the CPU ({})",
                parts.join("; ")
            )
        } else {
            format!("lobby: {w}x{h}, {name} {average:.1} ms a frame on the CPU")
        };
        self.log(&line);
    }

    /// A screen's own part in the shell (between its header and legend):
    /// who is signed in at the top right, then the screen's body. Its
    /// button hints are gathered for the legend (`Pen::hints`).
    fn draw_body(&self, p: &mut Pen) {
        self.draw_status(p);
        match self.screen {
            // Drawn already (`draw_intro`, `draw_menu`).
            Screen::Intro | Screen::Start | Screen::Main => {}
            Screen::SignIn => self.draw_sign_in(p),
            Screen::Connecting => self.draw_connecting(p),
            Screen::Live => self.draw_live(p),
            Screen::Players => self.draw_players(p),
            Screen::Party => self.draw_party_screen(p),
            Screen::Custom => self.draw_custom(p),
            Screen::Pregame => self.draw_pregame(p),
            Screen::InGame => self.draw_in_game(p),
            Screen::Carnage => self.draw_carnage(p),
            // Halo 2's large error dialog (`draw_dialog`).
            Screen::Failed => {}
            Screen::Record => self.draw_record(p),
            Screen::Friends => self.draw_friends(p),
            Screen::Settings => self.draw_settings(p),
        }
    }

    /// What goes over any screen after Halo 2's dialog (`draw_dialog`):
    /// ADD FRIEND's box (until 2.14's keyboard), then the notices.
    fn draw_over(&self, p: &mut Pen) {
        if self.popup == Some(Popup::AddFriend) {
            self.draw_add_friend(p);
        }
        // Last, so a dialog's overlay doesn't dim them.
        self.draw_toasts(p);
    }

    /// Who is signed in, on one small line at the top right, level with
    /// the header: the highest level's icon, the gamertag, then the level,
    /// how many are online and the server's round trip.
    fn draw_status(&self, p: &mut Pen) {
        let Some(c) = &self.client else {
            return;
        };
        let Some(w) = &c.view.welcome else {
            return;
        };
        let mut line = format!("Level {}  ·  {} online", w.best.max(1), c.view.online.len());
        if let Some(ms) = c.view.round_trip {
            line += &format!("  ·  {ms} ms");
        }
        let rest = p.text(1220.0, 44.0, 16.0, DIM, Align::Right, &line);
        let x = 1220.0 - rest - 14.0;
        let wd = p.text(x, 44.0, 18.0, WHITE, Align::Right, &w.gamertag);
        if p.has_icons() {
            let x = x - wd - 8.0;
            let level = w.best.max(1);
            p.level(x, 44.0, 18.0, WHITE, Align::Right, level, Size::Small);
        }
        // Friend requests waiting for an answer, under it on every screen:
        // one made while this player was offline gives no notice when
        // they sign in, and one made during a game only a short one. RB
        // (E) opens the friends list, where they come first.
        let asking = c.view.friends.iter();
        let asking = asking.filter(|f| f.relation == Relation::AskedUs).count();
        if asking > 0 {
            let s = if asking == 1 { "" } else { "s" };
            let line = format!("{asking} friend request{s}");
            p.text(1220.0, 68.0, 15.0, GOLD, Align::Right, &line);
        }
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
        p.hints(&[
            (Input::A, "Sign in"),
            (Input::X, "Controller settings"),
            (Input::B, "Back"),
        ]);
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

    fn draw_live(&self, p: &mut Pen) {
        let Some(v) = self.view() else {
            return;
        };
        // (The friend requests waiting are counted by `draw_status`.)
        if self.searching() {
            self.draw_search(p, v);
            let mut hints = Vec::new();
            if self.leader() {
                hints.push((Input::B, "Stop searching"));
            }
            hints.push((Input::Rb, "Friends"));
            hints.push((Input::Lb, "Service record"));
            p.hints(&hints);
        } else {
            self.draw_playlists(p, v);
            let mut hints = Vec::new();
            if self.sel == v.playlists.len() + 1 {
                hints.push((Input::A, "Controller settings"));
            } else if self.leader() {
                let custom = self.sel == v.playlists.len();
                hints.push((Input::A, if custom { "Custom game" } else { "Search" }));
            }
            hints.push((Input::X, "Players"));
            if v.party.is_some() {
                hints.push((Input::Y, "Party"));
            }
            hints.push((Input::Rb, "Friends"));
            hints.push((Input::Lb, "Service record"));
            hints.push((Input::B, "Back"));
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
        // The playlists, then the custom game and settings rows.
        let n = v.playlists.len() + 2;
        let first = first_row(self.sel, n, rows);
        for k in (first..n).take(rows) {
            let y = 156.0 + (k - first) as f32 * 50.0;
            let on = k == self.sel;
            p.row(72.0, y, 696.0, 46.0, on, Hit::Row(k));
            let col = if on { WHITE } else { TEXT };
            if k == v.playlists.len() + 1 {
                p.text(90.0, y + 32.0, 24.0, col, Align::Left, "Settings");
                continue;
            }
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
        if self.sel == v.playlists.len() + 1 {
            let c = &self.settings.controls;
            let line = format!(
                "Controller settings: {} buttons, {} thumbsticks, look sensitivity {}.",
                c.buttons.name(),
                c.sticks.name(),
                c.look_sensitivity
            );
            p.text(84.0, 600.0, 18.0, DIM, Align::Left, &line);
        } else if self.sel == v.playlists.len() {
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
            let level = self.member_level(pt, m);
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
            // As the party panel shows it, so the two agree.
            let level = self.member_level(pt, m);
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

    /// The controller settings: the rows on the left, and on the right
    /// what the layout picked does (or, on the mouse's rows, the keys).
    fn draw_settings(&self, p: &mut Pen) {
        let c = &self.settings.controls;
        p.panel(60.0, 110.0, 720.0, 530.0);
        for (k, label) in SETTINGS_ROWS.iter().enumerate() {
            let y = 120.0 + k as f32 * 52.0;
            let on = self.srow == k;
            p.row(72.0, y, 696.0, 48.0, on, Hit::Row(k));
            let col = if on { WHITE } else { TEXT };
            p.text(90.0, y + 33.0, 22.0, col, Align::Left, label);
            let value = match k {
                STICKS_ROW => c.sticks.name().to_string(),
                BUTTONS_ROW => c.buttons.name().to_string(),
                2 => controls::look_sensitivity_label(c.look_sensitivity),
                3 => controls::enabled(c.look_inverted).into(),
                4 => controls::enabled(c.auto_center).into(),
                5 => controls::enabled(c.vibration).into(),
                MOUSE_ROW => format!("{:.1}", c.mouse_sensitivity),
                MOUSE_INVERT_ROW => controls::enabled(c.mouse_inverted).into(),
                _ => continue,
            };
            p.text(600.0, y + 33.0, 22.0, col, Align::Center, &value);
            for (x, text, forward) in [(455.0, "<", false), (745.0, ">", true)] {
                let arrow = if on { SEL_EDGE } else { DIM };
                p.text(x, y + 33.0, 24.0, arrow, Align::Center, text);
                p.area(x - 22.0, y + 2.0, 44.0, 44.0, Hit::Step(k, forward));
            }
        }
        let help = match self.srow {
            STICKS_ROW => c.sticks.help(),
            BUTTONS_ROW => c.buttons.help(),
            2 => "How fast the controller turns and looks: 1 (low) to 10 (insane).",
            3 => "Pushing the thumbstick up looks down.",
            4 => "The view levels itself as you walk.",
            5 => "The controller rumbles when you fire, take hits and drive.",
            MOUSE_ROW => "How fast the mouse looks. Keys marked ? are MCC's and may not work yet.",
            MOUSE_INVERT_ROW => "Pushing the mouse forward looks down.",
            _ => "Every setting back to Halo 2's default.",
        };
        for (k, line) in p.wrap(17.0, 680.0, help).iter().take(2).enumerate() {
            p.text(84.0, 610.0 + k as f32 * 22.0, 17.0, DIM, Align::Left, line);
        }
        p.panel(810.0, 110.0, 410.0, 530.0);
        if matches!(self.srow, MOUSE_ROW | MOUSE_INVERT_ROW) {
            self.draw_keys(p);
        } else {
            self.draw_bindings(p, c);
        }
        let mut hints = vec![(
            Input::A,
            if self.srow == RESTORE_ROW {
                "Restore defaults"
            } else {
                "Change"
            },
        )];
        hints.push((Input::B, "Back"));
        p.hints(&hints);
    }

    /// The keyboard and mouse keys: halo2.dll's own, then MCC's (marked
    /// `?`: they work only if the engine reads the profile's table).
    fn draw_keys(&self, p: &mut Pen) {
        p.text(834.0, 142.0, 16.0, HEAD, Align::Left, "KEYBOARD AND MOUSE");
        for (k, (keys, what, the_games)) in controls::KEYBOARD_BINDINGS.iter().enumerate() {
            let y = 172.0 + k as f32 * 25.0;
            let (key_col, col) = if *the_games { (HEAD, TEXT) } else { (DIM, DIM) };
            p.text(834.0, y, 17.0, key_col, Align::Left, keys);
            let what = if *the_games {
                what.to_string()
            } else {
                format!("{what} ?")
            };
            let what = p.fit(17.0, 236.0, &what);
            p.text(960.0, y, 17.0, col, Align::Left, &what);
        }
        p.text(
            834.0,
            600.0,
            15.0,
            DIM,
            Align::Left,
            "? MCC's key: not yet known to work",
        );
        p.text(834.0, 620.0, 15.0, DIM, Align::Left, "in this engine.");
    }

    /// What each button of the layout picked does, and the thumbsticks.
    fn draw_bindings(&self, p: &mut Pen, c: &Controls) {
        p.text(834.0, 142.0, 16.0, HEAD, Align::Left, "BUTTONS");
        p.text(1196.0, 142.0, 16.0, DIM, Align::Right, c.buttons.name());
        let bindings = c.buttons.bindings();
        // Up to 14 lines, and the thumbsticks under them.
        let step = if bindings.len() > 12 { 25.0 } else { 28.0 };
        let mut y = 176.0;
        for (b, what) in bindings {
            let face = match b {
                button::A => Some(Input::A),
                button::B => Some(Input::B),
                button::X => Some(Input::X),
                button::Y => Some(Input::Y),
                button::LB => Some(Input::Lb),
                button::RB => Some(Input::Rb),
                _ => None,
            };
            match face {
                Some(i) => p.button(852.0, y - 7.0, i),
                None => {
                    p.text(834.0, y, 17.0, HEAD, Align::Left, short_button(b));
                }
            }
            let what = p.fit(18.0, 248.0, &what);
            p.text(948.0, y, 18.0, TEXT, Align::Left, &what);
            y += step;
        }
        y += 10.0;
        p.text(834.0, y, 16.0, HEAD, Align::Left, "THUMBSTICKS");
        p.text(1196.0, y, 16.0, DIM, Align::Right, c.sticks.name());
        let [left, right] = c.sticks.sticks();
        for (k, (stick, what)) in [("Left", left), ("Right", right)].iter().enumerate() {
            let y = y + 30.0 + k as f32 * 28.0;
            p.text(834.0, y, 17.0, HEAD, Align::Left, stick);
            p.text(948.0, y, 18.0, TEXT, Align::Left, what);
        }
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
        if g.results.is_none() && g.running && g.menu {
            p.text(640.0, 448.0, 20.0, DIM, Align::Center, IN_GAME_MENU_HINT);
        }
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
        // Until the engine's kills and assists are seen right
        // (`results::COUNTS_SEEN`), a column that is 0 for everyone may
        // only mean they weren't sent, so it says "-" rather than 0.
        let unsent = |count: fn(&LauncherPlayerResult) -> u16| {
            !crate::results::COUNTS_SEEN && rows.iter().all(|r| count(r) == 0)
        };
        let (no_kills, no_assists) = (unsent(|r| r.kills), unsent(|r| r.assists));
        let shown = |unsent: bool, n: u16| {
            if unsent {
                "-".to_string()
            } else {
                n.to_string()
            }
        };
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
                (790.0, shown(no_kills, r.kills)),
                (880.0, shown(no_assists, r.assists)),
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

    /// The notices, newest at the bottom, stacked up from just above the
    /// shell's button legend and clear of every popup's box (the lowest
    /// ends at y 520), so none is hidden behind one.
    fn draw_toasts(&self, p: &mut Pen) {
        for (k, (text, _)) in self.toasts.iter().rev().enumerate() {
            let top = toast_top(k);
            let w = p.measure(20.0, text) + 48.0;
            p.fill(
                640.0 - w / 2.0,
                top,
                w,
                TOAST_H,
                Color::rgb(0x0A_1830).alpha(235),
            );
            p.outline(640.0 - w / 2.0, top, w, TOAST_H, 2.0, SEL_EDGE);
            p.text(640.0, top + 27.0, 20.0, WHITE, Align::Center, text);
        }
    }
}

/// A notice's height, in layout units.
const TOAST_H: f32 = 40.0;

/// The intro's picture showing, as the only texture of its draw list
/// (`App::draw_intro`).
struct MovieFrame<'a>(&'a intro::Picture);

impl Textures for MovieFrame<'_> {
    fn image(&self, texture: Texture) -> Option<ImageRef<'_>> {
        let p = self.0;
        let image = ImageRef {
            width: p.w as usize,
            height: p.h as usize,
            rgba: &p.rgba,
        };
        (texture == Texture::Art(0) && image.is_whole()).then_some(image)
    }
}

/// The top of the `k`th notice up from the newest (layout units): the
/// newest ends at y 652, above the shell's button legend (from 660), and
/// the third starts at y 524, below the lowest popup's box (the friend
/// options popup ends at 520).
fn toast_top(k: usize) -> f32 {
    612.0 - k as f32 * 44.0
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

/// A button's name in the settings' list, short (the face buttons and the
/// bumpers are drawn as buttons).
fn short_button(b: u8) -> &'static str {
    match b {
        button::LS => "LS click",
        button::RS => "RS click",
        b => controls::button_name(b),
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
    /// The screen's button hints, for the shell's legend (`hints`).
    legend: Vec<(Input, String)>,
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

    /// The screen's button hints, for the shell's legend at the bottom
    /// right (`App::draw_shell`), which draws them: A, B, X and Y, their
    /// words in capitals as Halo 2's legends are. LB and RB still work
    /// (the service record and the friends list) but are PC extras and
    /// stay off it (menu.md 1.2).
    fn hints(&mut self, hints: &[(Input, &str)]) {
        for &(i, label) in hints {
            if matches!(i, Input::A | Input::B | Input::X | Input::Y) {
                self.legend.push((i, label.to_uppercase()));
            }
        }
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

    /// A popup's box, which alone takes clicks while it is up (over the
    /// globals' overlay, `App::draw_dialog`).
    fn popup_box(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.top = true;
        self.fill(x, y, w, h, Color::rgb(0x08_1A3C));
        self.outline(x, y, w, h, 2.0, EDGE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use h2ui::layout::DialogSize;
    use std::sync::Mutex;

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
            menus: Arc::new(Menus::flat()),
            vista: None,
            intro: None,
            engine_args: String::new(),
            scene: None,
            scene_fast: false,
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

    /// From the start screen through the main menu's ONLINE.
    fn to_online(a: &mut App) {
        assert_eq!(a.label(), "start");
        a.input(Input::A);
        assert_eq!(a.label(), "main");
        assert_eq!(a.main_focus.item, LIVE_ROW);
        a.input(Input::A);
    }

    #[test]
    fn the_sign_in_form_takes_a_gamertag_and_a_server() {
        let dir = scratch("form");
        let mut a = app(&dir);
        // With no gamertag saved, ONLINE opens the form.
        to_online(&mut a);
        assert_eq!(a.label(), "signin");
        assert!(a.dialing.is_none() && a.client.is_none());
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
        // Escape goes back to the main menu, and from there to the start
        // screen, which asks before quitting.
        a.input(Input::B);
        assert_eq!(a.label(), "main");
        a.input(Input::B);
        assert_eq!(a.label(), "start");
        a.input(Input::B);
        assert_eq!(a.label(), "quit");
        a.input(Input::Char('b'));
        assert_eq!(a.label(), "start");
        assert!(!a.quitting());
        // What was typed is still there.
        a.input(Input::A);
        a.input(Input::A);
        assert_eq!(a.label(), "signin");
        assert_eq!(a.fields[0], "MASTER CHIEF");
        a.input(Input::B);
        a.input(Input::B);
        a.input(Input::B);
        a.input(Input::A);
        assert!(a.quitting());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_saved_gamertag_signs_in_from_online() {
        let dir = scratch("saved");
        Settings {
            // Nothing listens on port 9 here, so the dial fails quickly.
            server: "127.0.0.1:9".into(),
            gamertag: "ALPHA".into(),
            ..Settings::default()
        }
        .save(&dir.join("lobby.txt"))
        .unwrap();
        let mut a = app(&dir);
        // Not at start-up: on the start screen nothing is dialled.
        a.tick(Instant::now());
        assert_eq!(a.label(), "start");
        assert!(a.dialing.is_none() && a.signing_in.is_none());
        // ONLINE signs in with it, without the form.
        to_online(&mut a);
        assert_eq!(a.label(), "connecting");
        assert!(a.dialing.is_some());
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
    fn the_main_menu_moves_its_focus_and_keeps_the_row_picked() {
        let dir = scratch("main-menu");
        let mut a = app(&dir);
        a.input(Input::A);
        assert_eq!(a.label(), "main");
        // Up and down stop at the ends.
        a.input(Input::Up);
        assert_eq!(a.main_focus.item, LIVE_ROW);
        a.input(Input::Down);
        a.input(Input::Down);
        assert_eq!(a.main_focus.item, MAIN_SETTINGS_ROW);
        // The row that lost the focus fades out.
        assert_eq!(a.main_focus.previous(), Some(LIVE_ROW));
        // SETTINGS opens the controller settings, which come back here
        // with SETTINGS still focused.
        a.input(Input::A);
        assert_eq!(a.label(), "settings");
        a.input(Input::B);
        assert_eq!(a.label(), "main");
        assert_eq!(a.main_focus.item, MAIN_SETTINGS_ROW);
        // And through the start screen too.
        a.input(Input::B);
        assert_eq!(a.label(), "start");
        a.input(Input::Char(' '));
        assert_eq!(a.label(), "main");
        assert_eq!(a.main_focus.item, MAIN_SETTINGS_ROW);
        // With no gamertag, ONLINE is the form, whose B comes back.
        a.input(Input::Up);
        a.input(Input::A);
        assert_eq!(a.label(), "signin");
        a.input(Input::B);
        assert_eq!(a.label(), "main");
        assert_eq!(a.main_focus.item, LIVE_ROW);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn online_signs_in_with_the_gamertag_saved_not_one_abandoned() {
        let dir = scratch("abandoned");
        Settings {
            server: "192.0.2.1:47050".into(),
            gamertag: "CHIEF".into(),
            ..Settings::default()
        }
        .save(&dir.join("lobby.txt"))
        .unwrap();
        let mut a = app(&dir);
        a.input(Input::A);
        // The form (as after signing out), edited and then left with B.
        a.screen = Screen::SignIn;
        for _ in 0..5 {
            a.input(Input::Backspace);
        }
        a.input(Input::Char('a'));
        a.input(Input::Char('r'));
        a.input(Input::Char('b'));
        a.input(Input::Tab);
        a.input(Input::Backspace);
        assert_eq!(a.fields, ["ARB", "192.0.2.1:4705"]);
        a.input(Input::B);
        assert_eq!(a.label(), "main");
        assert_eq!(a.small_print(), "CHIEF");
        // ONLINE signs in as the gamertag saved, at the server saved.
        a.input(Input::A);
        assert_eq!(a.label(), "connecting");
        assert_eq!(a.fields, ["CHIEF", "192.0.2.1:47050"]);
        assert_eq!(a.settings.gamertag, "CHIEF");
        // A gamertag that is taken isn't tried again from the main menu
        // either, nor shown there.
        a.screen = Screen::SignIn;
        a.fields[0] = "ARB".into();
        a.input(Input::A);
        assert_eq!(a.settings.gamertag, "ARB");
        // The dial is through by the time the server can refuse.
        a.dialing = None;
        a.on_event(LiveEvent::Refused(GAMERTAG_TAKEN.into()));
        assert_eq!(a.label(), "signin");
        a.input(Input::B);
        assert_eq!(a.small_print(), "CHIEF");
        a.input(Input::A);
        assert_eq!(a.label(), "connecting");
        assert_eq!(a.settings.gamertag, "CHIEF");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn signing_in_can_be_called_off_back_to_the_main_menu() {
        let dir = scratch("call-off");
        Settings {
            // A documentation address (TEST-NET-1): the dial is still
            // going when B is pressed.
            server: "192.0.2.1:47050".into(),
            gamertag: "ALPHA".into(),
            ..Settings::default()
        }
        .save(&dir.join("lobby.txt"))
        .unwrap();
        let mut a = app(&dir);
        to_online(&mut a);
        assert_eq!(a.label(), "connecting");
        a.input(Input::B);
        assert_eq!(a.label(), "main");
        assert!(a.dialing.is_none() && a.signing_in.is_none() && a.client.is_none());
        // Nothing comes back from the dial called off.
        a.tick(Instant::now());
        assert_eq!(a.label(), "main");
        // Picked again, it signs in again.
        a.input(Input::A);
        assert_eq!(a.label(), "connecting");
        // While it goes on, ONLINE from the main menu is the same
        // screen, not a second sign-in.
        a.screen = Screen::Main;
        let dial = a.signing_in;
        a.input(Input::A);
        assert_eq!(a.label(), "connecting");
        assert_eq!(a.signing_in, dial);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_offline_screens_b_goes_to_the_main_menu() {
        let dir = scratch("failed-b");
        let mut a = app(&dir);
        a.input(Input::A);
        a.fail("Couldn't reach the server.".into());
        // On the main menu it stays, and says why.
        assert_eq!(a.label(), "main");
        assert_eq!(a.failed, "Couldn't reach the server.");
        assert!(a.toasts.iter().any(|t| t.0 == a.failed));
        // Elsewhere it is the offline screen, whose A, X and Y stay.
        a.screen = Screen::Live;
        a.fail("Lost the server: gone".into());
        assert_eq!(a.label(), "failed");
        a.input(Input::X);
        assert_eq!(a.label(), "settings");
        a.input(Input::B);
        assert_eq!(a.label(), "failed");
        a.input(Input::B);
        assert_eq!(a.label(), "main");
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
    fn the_settings_screen_changes_and_saves_the_controls() {
        use crate::controls::{ButtonLayout, StickLayout};
        let dir = scratch("settings");
        let mut a = app(&dir);
        to_online(&mut a);
        // From the sign-in screen: the controller's X (the letter is typed).
        a.input(Input::Char('x'));
        assert_eq!(a.label(), "signin");
        assert_eq!(a.fields[0], "X");
        a.input(Input::X);
        assert_eq!(a.label(), "settings");
        // Halo 2's order: the thumbsticks first; A steps on.
        assert_eq!(a.srow, STICKS_ROW);
        a.input(Input::A);
        assert_eq!(a.settings.controls.sticks, StickLayout::Southpaw);
        // Down to the buttons; right and left step the layout round.
        a.input(Input::Down);
        a.input(Input::Right);
        assert_eq!(a.settings.controls.buttons, ButtonLayout::Southpaw);
        a.input(Input::Left);
        a.input(Input::Left);
        assert_eq!(a.settings.controls.buttons, ButtonLayout::Recon);
        a.input(Input::Left);
        assert_eq!(a.settings.controls.buttons, ButtonLayout::BumperJumper);
        // Look sensitivity stops at 10 with right, and A goes round.
        a.input(Input::Down);
        for _ in 0..12 {
            a.input(Input::Right);
        }
        assert_eq!(a.settings.controls.look_sensitivity, 10);
        a.input(Input::A);
        assert_eq!(a.settings.controls.look_sensitivity, 1);
        a.input(Input::Left);
        assert_eq!(a.settings.controls.look_sensitivity, 1);
        // The switches flip either way.
        a.input(Input::Down);
        a.input(Input::Left);
        assert!(a.settings.controls.look_inverted);
        a.input(Input::Down);
        a.input(Input::Down);
        a.input(Input::Right);
        assert!(!a.settings.controls.vibration);
        // Mouse sensitivity in tenths.
        a.input(Input::Down);
        assert_eq!(a.srow, MOUSE_ROW);
        a.input(Input::Right);
        a.input(Input::Right);
        assert_eq!(a.settings.controls.mouse_sensitivity, 1.8);
        // The mouse's inversion is its own.
        a.input(Input::Down);
        a.input(Input::A);
        assert!(a.settings.controls.mouse_inverted);
        // Saved as it changes: a new lobby reads them back.
        let saved = Settings::load(&dir.join("lobby.txt"));
        assert_eq!(saved.controls, a.settings.controls);
        assert_eq!(app(&dir).settings.controls, a.settings.controls);
        // Down stops at Restore Defaults, whose A puts every one back.
        for _ in 0..5 {
            a.input(Input::Down);
        }
        assert_eq!(SETTINGS_ROWS[RESTORE_ROW], "Restore Defaults");
        assert_eq!(a.srow, RESTORE_ROW);
        a.input(Input::A);
        assert_eq!(a.settings.controls, Controls::default());
        assert_eq!(
            Settings::load(&dir.join("lobby.txt")).controls,
            Controls::default()
        );
        // B (or Backspace, which rubs nothing out here) goes back.
        a.input(Input::Backspace);
        assert_eq!(a.label(), "signin");
        assert_eq!(a.fields[0], "X");
        // The headless script's `set`.
        assert!(a.set_control("button_layout", "bumper_jumper").is_ok());
        assert_eq!(a.settings.controls.buttons, ButtonLayout::BumperJumper);
        assert!(a.set_control("button_layout", "claw").is_err());
        assert!(a.set_control("colour", "red").is_err());
        assert_eq!(
            Settings::load(&dir.join("lobby.txt")).controls.buttons,
            ButtonLayout::BumperJumper
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_playlists_end_with_custom_game_and_settings() {
        let dir = scratch("settings-row");
        let (mut a, _server) = live_app(&dir);
        view(&mut a).playlists = vec![PlaylistInfo {
            id: 11,
            key: "h2h".into(),
            name: "Head to Head".into(),
            ranked: true,
            teams: false,
            guests: false,
            min: 2,
            max: 2,
            party_max: 1,
            searching: 0,
            playing: 0,
            level: 1,
            maps: Vec::new(),
        }];
        for _ in 0..5 {
            a.input(Input::Down);
        }
        a.tick(a.now);
        // Row 0 the playlist, 1 Custom Game, 2 Settings.
        assert_eq!(a.sel, 2);
        a.input(Input::A);
        assert_eq!(a.label(), "settings");
        a.input(Input::B);
        assert_eq!(a.label(), "live");
        assert_eq!(a.sel, 2);
        // Escape (or Backspace) goes back to the main menu, signed in
        // and in the party; ONLINE comes back to the playlists as they
        // were.
        view(&mut a).party = Some(party(7, Activity::Lobby));
        a.input(Input::Backspace);
        assert_eq!(a.label(), "main");
        assert!(a.client.is_some() && a.party().is_some());
        a.input(Input::A);
        assert_eq!(a.label(), "live");
        assert_eq!(a.sel, 2);
        // From the start screen, the quit popup offers to sign out.
        a.input(Input::B);
        a.input(Input::B);
        assert_eq!(a.label(), "start");
        a.input(Input::B);
        assert_eq!(a.label(), "quit");
        a.input(Input::Y);
        assert_eq!(a.label(), "signin");
        assert!(a.client.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_settings_screen_draws_each_layout_and_its_arrows_work() {
        let Ok(mut text) = Text::system() else {
            return;
        };
        let dir = scratch("settings-draw");
        let mut a = app(&dir);
        // The main menu's SETTINGS.
        a.input(Input::A);
        a.input(Input::Down);
        a.input(Input::A);
        assert_eq!(a.label(), "settings");
        let mut c = Canvas::new(1280, 720);
        for l in crate::controls::ButtonLayout::ALL {
            a.settings.controls.buttons = l;
            a.draw(&mut c, &mut text);
            assert!(a.drawn().iter().any(|d| d == l.name()), "{l:?}");
            assert!(a.drawn().iter().any(|d| d == "Fire"));
        }
        assert!(a.drawn().iter().any(|d| d == "Pause game"));
        assert!(a.drawn().iter().any(|d| d == "Look / rotate"));
        assert!(a.drawn().iter().any(|d| d == "CONTROLLER"));
        for row in [MOUSE_ROW, MOUSE_INVERT_ROW] {
            a.srow = row;
            a.draw(&mut c, &mut text);
            assert!(a.drawn().iter().any(|d| d == "KEYBOARD AND MOUSE"));
            assert!(a.drawn().iter().any(|d| d == "W A S D"));
            assert!(a.drawn().iter().any(|d| d == "Throw grenade"));
            assert!(a.drawn().iter().any(|d| d == "Dual wield ?"));
        }
        // The arrows step the row they are on.
        let (s, ox, oy) = a.view;
        let arrow = a
            .areas
            .iter()
            .find(|ar| ar.hit == Hit::Step(BUTTONS_ROW, true))
            .copied()
            .unwrap();
        let before = a.settings.controls.buttons;
        a.click(ox + (arrow.x + 5.0) * s, oy + (arrow.y + 5.0) * s);
        assert_eq!(a.srow, BUTTONS_ROW);
        assert_eq!(a.settings.controls.buttons, before.next(true));
        // Every row and its arrows are on the panel, above the help line
        // and the shell's legend.
        let last = a
            .areas
            .iter()
            .find(|ar| ar.hit == Hit::Row(RESTORE_ROW))
            .copied()
            .unwrap();
        assert!(last.y + last.h <= 600.0, "{}", last.y + last.h);
        let back = a.areas.iter().find(|ar| ar.hit == Hit::Press(Input::B));
        assert!(back.is_some_and(|b| b.y >= legend_top() - 0.5 && b.y >= last.y + last.h));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_game_gets_the_pad_and_the_keys_while_the_engine_plays() {
        let dir = scratch("ingame");
        let mut a = app(&dir);
        assert!(a.wants_pad());
        assert!(
            a.pad_is_the_lobbys(true),
            "only the in-game screen gives it up"
        );
        a.screen = Screen::InGame;
        // No engine: the lobby still takes the pad. An engine playing:
        // the game has it.
        assert!(a.wants_pad());
        assert!(a.pad_is_the_lobbys(false));
        assert!(!a.pad_is_the_lobbys(true));
        // Space (A here, jump there) and letters do nothing on this screen.
        a.key(Input::Char(' '));
        a.key(Input::Char('b'));
        assert_eq!(a.label(), "ingame");
        assert!(a.popup.is_none());
        // While the engine plays, Enter, Esc and Backspace are the game's
        // too, on the screen and in its Leave Game popup; arrows aren't.
        for i in [Input::A, Input::B, Input::Backspace, Input::Char('x')] {
            assert!(a.keys_are_the_games(i, true), "{i:?}");
        }
        assert!(!a.keys_are_the_games(Input::Up, true));
        a.popup = Some(Popup::LeaveGame);
        assert!(a.keys_are_the_games(Input::A, true));
        // A click on the popup's buttons still leaves (it goes to
        // `input`); with no engine, Esc works as before.
        a.popup = None;
        assert!(!a.keys_are_the_games(Input::B, false));
        a.key(Input::B);
        assert_eq!(a.label(), "leave");
        // Elsewhere the keys are the lobby's whether or not an engine runs.
        a.popup = None;
        a.screen = Screen::Live;
        for i in [Input::A, Input::B, Input::Char('x')] {
            assert!(!a.keys_are_the_games(i, true), "{i:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `rgb` (0 to 1) as the canvas's pixel.
    fn pixel([r, g, b]: [f32; 3]) -> u32 {
        let c = |v: f32| (v * 255.0).round() as u32;
        (c(r) << 16) | (c(g) << 8) | c(b)
    }

    /// The pixels of `c` within 3 of `want` in each of red, green and blue.
    fn pixels_like(c: &Canvas, want: u32) -> Vec<(f32, f32)> {
        let near = |p: u32| {
            (0..3).all(|k| ((p >> (8 * k)) & 0xFF).abs_diff((want >> (8 * k)) & 0xFF) <= 3)
        };
        let mut found = Vec::new();
        for (k, &p) in c.px.iter().enumerate() {
            if near(p) {
                found.push(((k % c.w) as f32 + 0.5, (k / c.w) as f32 + 0.5));
            }
        }
        found
    }

    /// The shell legend's top edge, in layout units (at 16:9 they are the
    /// window's pixels at 1280 by 720).
    fn legend_top() -> f32 {
        let space = Space::new(1280, 720);
        space.rect(h2ui::layout::legend_box(DialogSize::Full))[1]
    }

    #[test]
    fn screens_draw_and_clicks_land_on_buttons() {
        // Without a font on this system the lobby's own words aren't
        // drawn; the shell's header and legend are (h2ui's built-in font).
        let mut text = Text::system().unwrap_or_else(|_| Text::none());
        let dir = scratch("draw");
        let mut a = app(&dir);
        to_online(&mut a);
        assert_eq!(a.label(), "signin");
        let mut c = Canvas::new(640, 480);
        a.draw(&mut c, &mut text);
        // Halo 2's header, in its colour, at the top left in its box.
        let space = Space::new(640, 480);
        let [x0, y0, x1, y1] = space.rect(h2ui::layout::header_box(DialogSize::Full));
        let header = pixels_like(&c, pixel(h2ui::layout::colour::HEADER));
        let inside = |&(x, y): &(f32, f32)| x >= x0 && x <= x1 && y >= y0 && y <= y1;
        assert!(
            header.iter().filter(|p| inside(p)).count() > 20,
            "{header:?}"
        );
        assert!(a.drawn().iter().any(|d| d == "SIGN IN"));
        // No lobby title or keyboard line any more.
        assert!(a.drawn().iter().all(|d| !d.starts_with("Keyboard:")));
        // The legend's B ("BACK") is last, at the bottom right in the
        // legend's box, and a click on it goes back.
        let b = a
            .areas
            .iter()
            .find(|ar| ar.hit == Hit::Press(Input::B))
            .copied()
            .unwrap();
        let (s, ox, oy) = a.view;
        let [l0, t0, r0, b0] = space.rect(h2ui::layout::legend_box(DialogSize::Full));
        let (bx, by) = (ox + b.x * s, oy + b.y * s);
        assert!(
            (by - t0).abs() < 0.5 && (by + b.h * s - b0).abs() < 0.5,
            "{b:?}"
        );
        assert!((bx + b.w * s - r0).abs() <= 1.0 && bx > l0, "{b:?}");
        assert!(!b.top);
        let legend: Vec<&String> = a.drawn().iter().skip_while(|d| *d != "SIGN IN").collect();
        assert_eq!(
            legend.last().map(|d| d.as_str()),
            Some("BACK"),
            "{legend:?}"
        );
        a.click(bx + 5.0, by + 5.0);
        assert_eq!(a.label(), "main");
        a.input(Input::B);
        a.input(Input::B);
        assert_eq!(a.label(), "quit");
        a.draw(&mut c, &mut text);
        // With the popup up, only its buttons take clicks: not even the
        // start screen's click anywhere.
        a.click(ox + 400.0 * s, oy + 270.0 * s);
        assert_eq!(a.label(), "quit");
        let stay = a
            .areas
            .iter()
            .find(|ar| ar.top && ar.hit == Hit::Press(Input::B))
            .copied()
            .unwrap();
        a.click(ox + (stay.x + 5.0) * s, oy + (stay.y + 5.0) * s);
        assert_eq!(a.label(), "start");
        // A second click straight after (a double-click on Stay) stays on
        // the start screen; a click anywhere a moment later goes on.
        a.click(3.0, 3.0);
        assert_eq!(a.label(), "start");
        a.last_click = None;
        a.click(3.0, 3.0);
        assert_eq!(a.label(), "main");
        // Halo 2's screens are drawn in place of the lobby's, and say
        // what the headless script's `see` looks for once they have faded
        // in.
        a.input(Input::B);
        a.draw(&mut c, &mut text);
        assert!(a.drawn().iter().all(|d| d != screens::PRESS_START));
        a.menu_opened -= 5.0;
        a.draw(&mut c, &mut text);
        assert!(a.drawn().iter().any(|d| d == screens::PRESS_START));
        assert!(a.drawn().iter().all(|d| !d.starts_with("Keyboard:")));
        a.input(Input::A);
        a.draw(&mut c, &mut text);
        a.menu_opened -= 5.0;
        a.draw(&mut c, &mut text);
        for row in MAIN_ROWS {
            assert!(a.drawn().iter().any(|d| d == row), "{row}");
        }
        assert!(a.drawn().iter().all(|d| d != "SIGN IN"));
        // The custom game screen's arrows move the choices on and back.
        a.cfg.maps = vec![("midship".into(), 1), ("lockout".into(), 2)];
        a.open_custom();
        a.draw(&mut c, &mut text);
        let arrow = |a: &App, row: usize, on: bool| {
            let hit = a.areas.iter().find(|ar| ar.hit == Hit::Step(row, on));
            hit.copied().unwrap()
        };
        let next_map = arrow(&a, 1, true);
        // (Not the second half of a double-click with the start screen's.)
        a.last_click = None;
        a.click(ox + (next_map.x + 5.0) * s, oy + (next_map.y + 5.0) * s);
        assert_eq!((a.crow, a.cmap), (1, 0));
        let back = arrow(&a, 0, false);
        a.click(ox + (back.x + 5.0) * s, oy + (back.y + 5.0) * s);
        assert_eq!((a.crow, a.cgame), (0, names::CUSTOM_GAMES.len() - 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_legend_names_the_device_used_last() {
        let mut text = Text::none();
        let dir = scratch("legend-device");
        let mut a = app(&dir);
        // The main menu's SETTINGS.
        a.input(Input::A);
        a.input(Input::Down);
        a.input(Input::A);
        assert_eq!(a.label(), "settings");
        let mut c = Canvas::new(1280, 720);
        let shows = |a: &App, s: &str| a.drawn().iter().any(|d| d == s);
        // The controller comes first: its buttons' glyphs before any
        // press.
        a.draw(&mut c, &mut text);
        assert!(!a.keyboard);
        for s in ["\u{e100}", "CHANGE", "\u{e101}", "BACK"] {
            assert!(shows(&a, s), "{s:?} in {:?}", a.drawn());
        }
        assert!(!shows(&a, "ENTER"));
        // A key: the keys' names.
        a.key(Input::A);
        a.draw(&mut c, &mut text);
        for s in ["ENTER", "CHANGE", "ESC", "BACK"] {
            assert!(shows(&a, s), "{s:?} in {:?}", a.drawn());
        }
        assert!(!shows(&a, "\u{e100}"));
        // The pad again: its buttons.
        a.pad(Input::Down);
        assert_eq!(a.srow, 1);
        a.draw(&mut c, &mut text);
        assert!(!shows(&a, "ENTER") && shows(&a, "\u{e100}"));
        // The headless script's presses leave it as it was.
        a.input(Input::Up);
        assert!(!a.keyboard);
        // A click or the wheel is the keyboard's side.
        a.wheel(false);
        assert!(a.keyboard);
        assert_eq!(a.srow, 1);
        a.pad(Input::Up);
        a.click(1.0, 1.0);
        assert!(a.keyboard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_legend_names_no_key_or_button_that_does_nothing() {
        let mut text = Text::none();
        let dir = scratch("legend-keyless");
        let mut c = Canvas::new(1280, 720);
        let shows = |a: &App, s: &str| a.drawn().iter().any(|d| d == s);
        let legend_b = |a: &App| {
            let hit = a.areas.iter().find(|ar| ar.hit == Hit::Press(Input::B));
            hit.copied().unwrap()
        };
        // While the engine plays, the in-game screen's Esc and the pad
        // are the game's: LEAVE GAME alone, after either device.
        let (mut a, _server) = live_app(&dir);
        carnage(&mut a);
        let g = a.game.as_mut().unwrap();
        g.results = None;
        g.ended_at = None;
        g.child = Some(Running::waiting());
        a.screen = Screen::InGame;
        for keyboard in [false, true] {
            a.keyboard = keyboard;
            a.draw(&mut c, &mut text);
            assert!(shows(&a, "LEAVE GAME"), "{:?}", a.drawn());
            for key in ["ESC", "\u{e101}"] {
                assert!(!shows(&a, key), "{key:?} in {:?}", a.drawn());
            }
        }
        // A click on it still asks.
        let (s, ox, oy) = a.view;
        let b = legend_b(&a);
        a.click(ox + (b.x + 3.0) * s, oy + (b.y + 3.0) * s);
        assert_eq!(a.label(), "leave");
        a.popup = None;
        // Once the engine has closed, Esc and the pad's B work there.
        a.game.as_mut().unwrap().child.as_mut().unwrap().kill();
        a.game.as_mut().unwrap().child = None;
        a.keyboard = true;
        a.draw(&mut c, &mut text);
        assert!(shows(&a, "ESC") && shows(&a, "LEAVE GAME"));
        a.keyboard = false;
        a.draw(&mut c, &mut text);
        assert!(shows(&a, "\u{e101}"));
        // On SIGN IN the X key types a letter: after the keyboard its X
        // is CONTROLLER SETTINGS alone (Enter and Esc still named); after
        // the pad, whose X opens them, its glyph shows.
        let mut a = app(&dir);
        to_online(&mut a);
        assert_eq!(a.label(), "signin");
        a.key(Input::Char('a'));
        a.draw(&mut c, &mut text);
        for s in ["ENTER", "CONTROLLER SETTINGS", "ESC", "BACK"] {
            assert!(shows(&a, s), "{s:?} in {:?}", a.drawn());
        }
        assert!(!shows(&a, "X"), "{:?}", a.drawn());
        // A click on it still opens them.
        let (s, ox, oy) = a.view;
        let x = a.areas.iter().find(|ar| ar.hit == Hit::Press(Input::X));
        let x = x.copied().unwrap();
        a.click(ox + (x.x + 3.0) * s, oy + (x.y + 3.0) * s);
        assert_eq!(a.label(), "settings");
        a.input(Input::B);
        assert_eq!(a.label(), "signin");
        a.pad(Input::Down);
        a.draw(&mut c, &mut text);
        assert!(shows(&a, "\u{e102}") && shows(&a, "CONTROLLER SETTINGS"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_playlists_legend_holds_its_buttons_and_presses_them() {
        let mut text = Text::none();
        let dir = scratch("legend-live");
        let (mut a, _server) = live_app(&dir);
        let mut c = Canvas::new(1280, 720);
        a.draw(&mut c, &mut text);
        let drawn = a.drawn().to_vec();
        for s in ["PLAYLISTS", "PLAYERS", "BACK"] {
            assert!(drawn.iter().any(|d| d == s), "{s:?} in {drawn:?}");
        }
        // The bumpers' shortcuts stay off it (they still work).
        assert!(drawn.iter().all(
            |d| !d.eq_ignore_ascii_case("service record") && !d.eq_ignore_ascii_case("friends")
        ));
        // Who is signed in, at the top right.
        assert!(drawn.iter().any(|d| d == "ALPHA"));
        assert!(drawn.iter().any(|d| d.starts_with("Level 3  ·  0 online")));
        // Under it, the friend requests waiting, if any (not those we
        // made, nor friends).
        assert!(drawn.iter().all(|d| !d.contains("friend request")));
        let asked = |id, name: &str, r| friend(id, name, r, Online::Offline);
        view(&mut a).friends = vec![
            asked(12, "YAN", Relation::AskedUs),
            asked(13, "ZED", Relation::WeAsked),
            asked(14, "UMA", Relation::Friend),
        ];
        a.draw(&mut c, &mut text);
        let requests: Vec<&String> = a.drawn().iter().filter(|d| d.contains("request")).collect();
        assert_eq!(requests, ["1 friend request"]);
        view(&mut a)
            .friends
            .push(asked(15, "AMY", Relation::AskedUs));
        a.draw(&mut c, &mut text);
        assert!(a.drawn().iter().any(|d| d == "2 friend requests"));
        // On the other screens too.
        a.screen = Screen::Players;
        a.draw(&mut c, &mut text);
        assert!(a.drawn().iter().any(|d| d == "2 friend requests"));
        a.screen = Screen::Live;
        view(&mut a).friends.clear();
        // With the invitation up, it alone takes clicks: the legend's
        // B doesn't go back.
        let area = |a: &App, i: Input| {
            let hit = a.areas.iter().find(|ar| !ar.top && ar.hit == Hit::Press(i));
            hit.copied().unwrap()
        };
        let (s, ox, oy) = a.view;
        let click = |a: &mut App, ar: Area| a.click(ox + (ar.x + 3.0) * s, oy + (ar.y + 3.0) * s);
        view(&mut a).invites.push((9, "BRAVO".into()));
        a.draw(&mut c, &mut text);
        let back = area(&a, Input::B);
        click(&mut a, back);
        assert_eq!(a.label(), "invite");
        view(&mut a).invites.clear();
        a.last_click = None;
        // Without it, the legend's X opens the players online.
        a.draw(&mut c, &mut text);
        let players = area(&a, Input::X);
        click(&mut a, players);
        assert_eq!(a.label(), "players");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_double_click_on_the_legends_back_goes_back_once() {
        let mut text = Text::none();
        let dir = scratch("double-back");
        let (mut a, _server) = live_app(&dir);
        let mut c = Canvas::new(1280, 720);
        a.input(Input::X);
        assert_eq!(a.label(), "players");
        a.draw(&mut c, &mut text);
        let (s, ox, oy) = a.view;
        let back = |a: &App| {
            let hit = a.areas.iter().find(|ar| ar.hit == Hit::Press(Input::B));
            hit.copied().unwrap()
        };
        // Near the legend's right end, where its BACK is on every screen.
        let b = back(&a);
        let (x, y) = (ox + (b.x + b.w - 3.0) * s, oy + (b.y + 3.0) * s);
        a.click(x, y);
        assert_eq!(a.label(), "live");
        a.draw(&mut c, &mut text);
        let under = back(&a);
        let (ux, uy) = ((x - ox) / s, (y - oy) / s);
        assert!(
            ux >= under.x && ux < under.x + under.w && uy >= under.y && uy < under.y + under.h,
            "the playlists' BACK is under the click: {under:?}"
        );
        // The double-click's second half doesn't go on to the main menu.
        a.click(x, y);
        assert_eq!(a.label(), "live");
        // A click there a moment later does.
        a.last_click = None;
        a.click(x, y);
        assert_eq!(a.label(), "main");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_double_click_that_shuts_a_popup_presses_nothing_under_it() {
        let mut text = Text::none();
        let dir = scratch("double-popup");
        let (mut a, mut server) = live_app(&dir);
        let mut c = Canvas::new(1280, 720);
        // Seven playlists: rows 5 and 6 lie under the invitation's
        // buttons. The sixth is selected, and we lead (no party).
        view(&mut a).playlists = (0..7)
            .map(|k| PlaylistInfo {
                id: 20 + k,
                key: format!("p{k}"),
                name: format!("Playlist {k}"),
                ranked: false,
                teams: false,
                guests: false,
                min: 2,
                max: 8,
                party_max: 4,
                searching: 0,
                playing: 0,
                level: 1,
                maps: Vec::new(),
            })
            .collect();
        a.sel = 5;
        view(&mut a).invites.push((9, "BRAVO".into()));
        a.draw(&mut c, &mut text);
        assert_eq!(a.label(), "invite");
        let (s, ox, oy) = a.view;
        let find = |a: &App, top: bool, hit: Hit| {
            let ar = a.areas.iter().find(|ar| ar.top == top && ar.hit == hit);
            ar.copied().unwrap()
        };
        // The invitation's B DECLINE: a click shuts it.
        let decline = find(&a, true, Hit::Press(Input::B));
        let (ux, uy) = (decline.x + 3.0, decline.y + 3.0);
        let (x, y) = (ox + ux * s, oy + uy * s);
        a.click(x, y);
        assert_eq!(a.label(), "live");
        assert_eq!(sent(&mut server), [ToServer::Decline(9)]);
        a.draw(&mut c, &mut text);
        let row = find(&a, false, Hit::Row(5));
        assert!(
            ux >= row.x && ux < row.x + row.w && uy >= row.y && uy < row.y + row.h,
            "the selected row is under the click: {row:?}"
        );
        // The double-click's second half only selects that row: no
        // search.
        a.click(x, y);
        assert_eq!((a.label().as_str(), a.sel), ("live", 5));
        assert!(sent(&mut server).is_empty());
        // A click there a moment later searches it.
        a.last_click = None;
        a.click(x, y);
        assert_eq!(sent(&mut server), [ToServer::Search(25)]);

        // Two clicks in a popup that stays up both count: the first
        // selects an option, the second picks it.
        view(&mut a).friends = vec![friend(11, "BOB", Relation::Friend, Online::Offline)];
        a.screen = Screen::Friends;
        a.tick(a.now);
        a.input(Input::A);
        assert_eq!(a.label(), "friend");
        // Remove friend selected; Service record above it.
        a.input(Input::Down);
        assert_eq!(a.osel, 1);
        a.draw(&mut c, &mut text);
        let record = find(&a, true, Hit::Row(0));
        let (x, y) = (ox + (record.x + 3.0) * s, oy + (record.y + 3.0) * s);
        a.last_click = None;
        a.click(x, y);
        assert_eq!((a.label().as_str(), a.osel), ("friend", 0));
        a.click(x, y);
        assert_eq!(a.label(), "record");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn typing_a_friends_gamertag_leaves_x_and_y_keyless() {
        let mut text = Text::none();
        let dir = scratch("addfriend-keyless");
        let (mut a, _server) = live_app(&dir);
        let mut c = Canvas::new(1280, 720);
        let shows = |a: &App, s: &str| a.drawn().iter().any(|d| d == s);
        view(&mut a).friends = vec![friend(12, "YAN", Relation::AskedUs, Online::Offline)];
        a.input(Input::Rb);
        assert_eq!(a.label(), "friends");
        // After the keyboard, the friends list names its keys: X and Y
        // are buttons here.
        a.key(Input::Down);
        a.draw(&mut c, &mut text);
        for s in [
            "ENTER",
            "ACCEPT",
            "X",
            "DECLINE",
            "Y",
            "ADD FRIEND",
            "ESC",
            "BACK",
        ] {
            assert!(shows(&a, s), "{s:?} in {:?}", a.drawn());
        }
        // Under ADD FRIEND the X and Y keys type letters: the legend
        // under it gives their words alone.
        a.key(Input::Char('y'));
        assert_eq!(a.label(), "addfriend");
        a.draw(&mut c, &mut text);
        for s in ["ENTER", "DECLINE", "ADD FRIEND", "ESC"] {
            assert!(shows(&a, s), "{s:?} in {:?}", a.drawn());
        }
        for s in ["X", "Y"] {
            assert!(!shows(&a, s), "{s:?} in {:?}", a.drawn());
        }
        a.key(Input::Char('y'));
        assert_eq!(a.friend_tag, "Y");
        // After the pad, whose X and Y aren't letters, the glyphs show.
        a.pad(Input::B);
        a.pad(Input::Down);
        a.input(Input::Y);
        assert_eq!(a.label(), "addfriend");
        a.draw(&mut c, &mut text);
        for s in ["\u{e102}", "\u{e103}"] {
            assert!(shows(&a, s), "{s:?} in {:?}", a.drawn());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_shells_still_background_is_kept_while_its_strips_scroll() {
        let mut text = Text::none();
        let dir = scratch("shell-kept");
        let mut menus = Menus::flat();
        // A framing and one strip that scrolls, with stand-in pictures:
        // the strip's columns differ so its scrolling shows.
        let flat = |w: usize, h: usize| h2ui::art::ArtImage {
            width: w,
            height: h,
            rgba: [40, 60, 120, 200].repeat(w * h),
        };
        let strip = h2ui::art::ArtImage {
            width: 32,
            height: 40,
            rgba: (0..32 * 40)
                .flat_map(|k| [(k % 32 * 8) as u8, 255, 0, 255])
                .collect(),
        };
        let shell = &mut menus.shell;
        shell.art.add_image(h2ui::layout::FRAMING, flat(64, 40));
        shell.art.add_image("strip", strip);
        shell.chrome.still = vec![BitmapWidget {
            scale: [32.0, 32.0],
            depth: 10,
            ..BitmapWidget::new(h2ui::layout::FRAMING, h2ui::layout::FRAMING_CORNER)
        }];
        shell.chrome.moving = vec![BitmapWidget {
            scale: [1.1, 1.1],
            wraps_per_second: [0.08, 0.0],
            depth: 11,
            ..BitmapWidget::new("strip", [-200.0, 300.0])
        }];
        let mut a = app(&dir);
        a.cfg.menus = Arc::new(menus);
        let mut c = Canvas::new(1280, 720);
        a.input(Input::A);
        // The main menu drawn first at the same size, as when SETTINGS is
        // picked from it: the timer's size is set, so only the shell's
        // own fresh background keeps its first frame out.
        assert_eq!(a.label(), "main");
        a.draw(&mut c, &mut text);
        a.input(Input::Down);
        a.input(Input::A);
        assert_eq!(a.label(), "settings");
        let timed = |a: &App| {
            let f = a.menu_times.frames.iter().find(|f| f.name == "settings");
            f.map_or(0, |f| f.frames)
        };
        // The first frame at a size draws the still background (and isn't
        // timed); the next ones copy it in, so they are.
        a.draw(&mut c, &mut text);
        assert_eq!(timed(&a), 0, "the fresh background's frame was timed");
        let first = c.px.clone();
        a.clock -= Duration::from_secs(2);
        a.draw(&mut c, &mut text);
        assert_eq!(timed(&a), 1, "the kept background wasn't drawn again");
        a.clock -= Duration::from_secs(2);
        a.draw(&mut c, &mut text);
        assert_eq!(timed(&a), 2);
        // The strip moved, and nothing else: from 256 to 300 units up and
        // 200 to 165 left of the middle (y 180 to 206 and x 520 to 541
        // here).
        let changed: Vec<usize> = (0..c.px.len()).filter(|&k| c.px[k] != first[k]).collect();
        assert!(!changed.is_empty(), "the strip scrolls");
        for k in changed {
            let (x, y) = (k % 1280, k / 1280);
            assert!((178..=208).contains(&y), "pixel {x}, {y} changed");
            assert!((518..=543).contains(&x), "pixel {x}, {y} changed");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A lobby signed in as ALPHA (account 1) over one end of a pair, on
    /// the playlists, and the server's end, to read what the lobby sends.
    fn live_app(dir: &std::path::Path) -> (App, Connection) {
        let (ours, mut theirs) = Connection::pair();
        let a = live_app_over(dir, ours);
        // The LOGIN.
        sent(&mut theirs);
        (a, theirs)
    }

    /// `live_app` signed in over `ours`.
    fn live_app_over(dir: &std::path::Path, ours: Connection) -> App {
        let mut a = app(dir);
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
        a
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
        let m = a_match();
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
            left: None,
            menu: true,
            engine_controls: None,
        });
        a.screen = Screen::Carnage;
        a.csel = 1;
    }

    /// A Head to Head match on Lockout for ALPHA (us) and BRAVO, ALPHA
    /// hosting.
    fn a_match() -> LauncherMatch {
        let player = |relay_id: u64, account: u64, tag: &str| h2net::live::LauncherPlayer {
            relay_id,
            account,
            gamertag: tag.into(),
            team: 0,
            level: 4,
            party: account,
        };
        LauncherMatch {
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
        }
    }

    #[test]
    fn presses_ask_for_halo_2s_sounds() {
        let dir = scratch("cues");
        let mut a = app(&dir);
        // B on the start screen opens the quit popup, and B shuts it.
        a.input(Input::B);
        assert_eq!(a.take_cues(), [Cue::Back]);
        a.input(Input::Char('b'));
        assert_eq!(a.take_cues(), [Cue::Back]);
        // A on the start screen: the press, the main menu loading, and
        // the start screen going on to it.
        a.input(Input::A);
        assert_eq!(a.take_cues(), [Cue::Select, Cue::Advance, Cue::Transport]);
        assert_eq!(a.label(), "main");
        // Down moves the focus; at the end it doesn't, and is silent.
        a.input(Input::Down);
        assert_eq!(a.take_cues(), [Cue::Cursor]);
        a.input(Input::Down);
        assert!(a.take_cues().is_empty());
        a.input(Input::Up);
        assert_eq!(a.take_cues(), [Cue::Cursor]);
        // A on ONLINE (no gamertag yet: the sign-in form): the press,
        // then the screen loading.
        a.input(Input::A);
        assert_eq!(a.label(), "signin");
        assert_eq!(a.take_cues(), [Cue::Select, Cue::Advance]);
        // Typing is silent; Tab moves to the other field.
        a.input(Input::Char('j'));
        assert!(a.take_cues().is_empty());
        a.input(Input::Tab);
        assert_eq!(a.take_cues(), [Cue::Cursor]);
        a.input(Input::B);
        assert_eq!(a.take_cues(), [Cue::Back]);
        // SETTINGS, a value changed in place, and B back from it.
        a.input(Input::Down);
        a.input(Input::A);
        assert_eq!(a.label(), "settings");
        assert_eq!(a.take_cues(), [Cue::Cursor, Cue::Select, Cue::Advance]);
        a.input(Input::Right);
        assert_eq!(a.take_cues(), [Cue::Cursor]);
        a.input(Input::Backspace);
        assert_eq!(a.label(), "main");
        assert_eq!(a.take_cues(), [Cue::Back]);
        // A click on a main menu row focuses it, then picks it.
        a.size = (1280, 720);
        let row = (0..720)
            .step_by(4)
            .flat_map(|y| (0..1280).step_by(8).map(move |x| (x as f32, y as f32)))
            .find(|&(x, y)| a.main_row_at(x, y) == Some(LIVE_ROW))
            .unwrap();
        a.click(row.0, row.1);
        assert_eq!(a.take_cues(), [Cue::Cursor]);
        a.last_click = None;
        a.click(row.0, row.1);
        assert_eq!(a.take_cues(), [Cue::Select, Cue::Advance]);
        assert_eq!(a.label(), "signin");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refusal_sounds_as_an_error_and_a_search_advances() {
        let dir = scratch("cue-refused");
        let (mut a, mut server) = live_app(&dir);
        view(&mut a).playlists = vec![PlaylistInfo {
            id: 11,
            key: "h2h".into(),
            name: "Head to Head".into(),
            ranked: true,
            teams: false,
            guests: false,
            min: 2,
            max: 2,
            party_max: 1,
            searching: 0,
            playing: 0,
            level: 1,
            maps: Vec::new(),
        }];
        // A party BRAVO leads: we may not pick a playlist.
        let mut pt = party(7, Activity::Lobby);
        pt.leader = 2;
        pt.members.push(PartyMember {
            account: 2,
            gamertag: "BRAVO".into(),
            look: Default::default(),
            best: 1,
            level: 1,
            guests: 0,
        });
        view(&mut a).party = Some(pt);
        a.sel = 0;
        a.input(Input::A);
        assert_eq!(a.take_cues(), [Cue::Error]);
        let said = "Only the party leader can pick a playlist.";
        assert!(a.toasts.iter().any(|(t, _)| t == said));
        assert!(sent(&mut server).is_empty());
        // Nor a custom game (the row after the playlists).
        a.input(Input::Down);
        assert_eq!(a.take_cues(), [Cue::Cursor]);
        a.input(Input::A);
        assert_eq!(a.take_cues(), [Cue::Error]);
        // Leading it, A searches: the press, then Halo 2's matchmaking
        // screen loading.
        view(&mut a).party.as_mut().unwrap().leader = 1;
        a.input(Input::Up);
        a.take_cues();
        a.input(Input::A);
        assert_eq!(a.take_cues(), [Cue::Select, Cue::Advance]);
        assert_eq!(sent(&mut server), [ToServer::Search(11)]);
        // Searching: the leader's B stops it, as Back; a member's is
        // refused.
        view(&mut a).party.as_mut().unwrap().activity = Activity::Searching;
        a.input(Input::B);
        assert_eq!(a.take_cues(), [Cue::Back]);
        assert_eq!(sent(&mut server), [ToServer::Cancel]);
        view(&mut a).party.as_mut().unwrap().leader = 2;
        a.input(Input::B);
        assert_eq!(a.take_cues(), [Cue::Error]);
        assert!(sent(&mut server).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_custom_game_asked_for_sounds_its_press_and_no_maps_is_an_error() {
        let dir = scratch("cue-custom");
        let mut a = app(&dir);
        a.cfg.maps = vec![("midship".into(), 1), ("lockout".into(), 2)];
        a.open_custom();
        // The game and the map change in place.
        a.input(Input::Left);
        assert_eq!(a.take_cues(), [Cue::Cursor]);
        a.input(Input::Down);
        assert_eq!(a.take_cues(), [Cue::Cursor]);
        a.input(Input::Right);
        assert_eq!(a.take_cues(), [Cue::Cursor]);
        // A asks for it: Halo 2's START GAME is a button, so its press
        // only (the pregame lobby's advance comes with the match).
        a.input(Input::A);
        assert!(a.asked.is_some());
        assert_eq!(a.take_cues(), [Cue::Select]);
        // Asked already: nothing happens, silently.
        a.input(Input::A);
        assert!(a.take_cues().is_empty());
        // No maps: refused, with the error sound.
        a.cfg.maps = Vec::new();
        a.open_custom();
        a.input(Input::A);
        assert_eq!(a.take_cues(), [Cue::Error]);
        assert!(a.asked.is_none());
        let said = "No Halo 2 maps were found in MCC's folder.";
        assert!(a.toasts.iter().any(|(t, _)| t == said));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_popup_opening_advances_but_one_closing_or_a_tab_doesnt() {
        let dir = scratch("cue-popups");
        let (mut a, mut server) = live_app(&dir);
        let mut pt = party(7, Activity::Lobby);
        pt.members.push(PartyMember {
            account: 2,
            gamertag: "BRAVO".into(),
            look: Default::default(),
            best: 1,
            level: 1,
            guests: 0,
        });
        view(&mut a).party = Some(pt);
        a.screen = Screen::Party;
        assert!(a.pick("bravo"));
        // X opens the kick confirmation: the dialog loading alone (only
        // A and Start play a list item's or button's `forward1`).
        a.input(Input::X);
        assert!(matches!(a.popup, Some(Popup::Kick(2))));
        assert_eq!(a.take_cues(), [Cue::Advance]);
        // A on it only closes it: the press alone.
        a.input(Input::A);
        assert!(a.popup.is_none());
        assert_eq!(a.take_cues(), [Cue::Select]);
        assert_eq!(sent(&mut server), [ToServer::Kick(2)]);
        // B on a popup, as on a screen: back alone.
        a.input(Input::X);
        a.take_cues();
        a.input(Input::B);
        assert_eq!(a.take_cues(), [Cue::Back]);
        // X on the playlists opens the players list: Halo 2's Y menu
        // loading, which Y opens with no sound of its own (0x1494af), so
        // the advance alone. RB there goes to its friends tab: the tab's
        // press alone.
        a.screen = Screen::Live;
        a.input(Input::X);
        assert_eq!(a.label(), "players");
        assert_eq!(a.take_cues(), [Cue::Advance]);
        a.input(Input::Rb);
        assert_eq!(a.label(), "friends");
        assert_eq!(a.take_cues(), [Cue::Select]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_pregame_lobby_and_the_carnage_report_advance_as_they_open() {
        let dir = scratch("cue-match");
        // A match needs the server's address: a real connection.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let ours = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let _theirs = listener.accept().unwrap();
        let mut a = live_app_over(&dir, Connection::tcp(ours).unwrap());
        a.on_event(LiveEvent::LauncherMatch(a_match()));
        assert_eq!(a.label(), "pregame");
        assert_eq!(a.take_cues(), [Cue::Advance]);
        // The same match again with another host: nothing opens.
        a.on_event(LiveEvent::LauncherMatch(a_match()));
        assert!(a.take_cues().is_empty());
        // The engine closes after the game: the carnage report.
        let (mut a, _server) = live_app(&dir);
        carnage(&mut a);
        a.screen = Screen::InGame;
        let mut engine = Running::waiting();
        engine.kill();
        a.game.as_mut().unwrap().child = Some(engine);
        a.tick(a.now);
        assert_eq!(a.screen, Screen::Carnage);
        assert_eq!(a.take_cues(), [Cue::Advance]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn screens_the_lobby_opens_by_itself_advance() {
        let dir = scratch("cue-itself");
        // Signed in: the playlists load with no press.
        let (mut a, _server) = live_app(&dir);
        a.screen = Screen::Connecting;
        a.signing_in = Some(Instant::now());
        a.on_event(LiveEvent::Welcomed);
        assert_eq!(a.label(), "live");
        assert_eq!(a.take_cues(), [Cue::Advance]);
        // The server lost on the playlists: the failure screen loads,
        // once.
        a.on_event(LiveEvent::Lost("gone".into()));
        assert_eq!(a.label(), "failed");
        assert_eq!(a.take_cues(), [Cue::Advance]);
        a.tick(a.now);
        a.fail("again".into());
        assert!(a.take_cues().is_empty());
        // Lost on the main menu: a notice, and no screen loads.
        let (mut a, _server) = live_app(&dir);
        a.screen = Screen::Main;
        a.on_event(LiveEvent::Lost("gone".into()));
        assert_eq!(a.label(), "main");
        assert!(a.take_cues().is_empty());
        // Lost during a match: nothing until the engine closes, then the
        // failure screen loads.
        let (mut a, _server) = live_app(&dir);
        carnage(&mut a);
        let g = a.game.as_mut().unwrap();
        g.results = None;
        g.ended_at = None;
        g.child = Some(Running::waiting());
        a.screen = Screen::InGame;
        a.on_event(LiveEvent::Lost("gone".into()));
        a.tick(a.now);
        assert_eq!(a.label(), "ingame");
        assert!(a.take_cues().is_empty());
        a.game.as_mut().unwrap().child.as_mut().unwrap().kill();
        a.tick(a.now);
        assert_eq!(a.label(), "failed");
        assert_eq!(a.take_cues(), [Cue::Advance]);
        // The same after a game with results: the carnage report's
        // advance stands for the failure screen it gives way to, once.
        let (mut a, _server) = live_app(&dir);
        carnage(&mut a);
        a.screen = Screen::InGame;
        let mut engine = Running::waiting();
        engine.kill();
        a.game.as_mut().unwrap().child = Some(engine);
        a.client = None;
        a.tick(a.now);
        assert_eq!(a.label(), "failed");
        assert_eq!(a.take_cues(), [Cue::Advance]);
        a.tick(a.now);
        assert!(a.take_cues().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn behind_the_engine_the_window_is_drawn_seldom_but_polled_as_often() {
        let dir = scratch("draw-pace");
        let (mut a, _server) = live_app(&dir);
        carnage(&mut a);
        a.screen = Screen::InGame;
        a.game.as_mut().unwrap().child = None;
        assert_eq!(
            (a.draw_interval(true), a.draw_interval(false)),
            (FRAME, FRAME)
        );
        // While the engine plays, twice a second without the focus, and
        // each frame with it; the window wakes each frame either way.
        a.game.as_mut().unwrap().child = Some(Running::waiting());
        assert_eq!(a.draw_interval(false), FRAME_BEHIND_GAME);
        assert_eq!(a.draw_interval(true), FRAME);
        assert_eq!(a.frame_interval(), FRAME);
        a.game.as_mut().unwrap().child.as_mut().unwrap().kill();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_invitation_sounds_once() {
        let dir = scratch("cue-invite");
        let (mut a, _server) = live_app(&dir);
        a.screen = Screen::Main;
        a.tick(Instant::now());
        assert!(a.take_cues().is_empty());
        view(&mut a).invites = vec![(55, "BRAVO".into())];
        a.tick(Instant::now());
        assert_eq!(a.take_cues(), [Cue::Notify]);
        a.tick(Instant::now());
        assert!(a.take_cues().is_empty());
        assert_eq!(a.label(), "invite");
        // Another: once more. Declining it with B sounds as Back.
        view(&mut a).invites.push((56, "CHARLIE".into()));
        a.tick(Instant::now());
        assert_eq!(a.take_cues(), [Cue::Notify]);
        a.input(Input::B);
        assert_eq!(a.take_cues(), [Cue::Back]);
        // The one under it was seen, but is shown again: it sounds again.
        a.tick(Instant::now());
        assert_eq!(a.take_cues(), [Cue::Notify]);
        // Hidden by a popup, it is still the one seen: no sound when it
        // shows again.
        a.popup = Some(Popup::Quit);
        a.tick(Instant::now());
        a.popup = None;
        a.tick(Instant::now());
        assert!(a.take_cues().is_empty());
        // Declined, then the same party invites again: it sounds.
        a.input(Input::B);
        assert_eq!(a.take_cues(), [Cue::Back]);
        a.tick(Instant::now());
        assert!(a.take_cues().is_empty());
        view(&mut a).invites.push((55, "BRAVO".into()));
        a.tick(Instant::now());
        assert_eq!(a.take_cues(), [Cue::Notify]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_friend_request_to_us_sounds_as_a_message() {
        let dir = scratch("cue-asked");
        let (mut a, _server) = live_app(&dir);
        use h2net::live::{friend_notice, ACCEPTED, ALREADY_ASKED, ASKED_YOU, SLOW_DOWN};
        a.on_event(LiveEvent::Notice(friend_notice(ASKED_YOU, "BRAVO")));
        assert_eq!(a.take_cues(), [Cue::Notify]);
        // Other notices are only shown.
        for text in [
            friend_notice(ACCEPTED, "BRAVO"),
            friend_notice(ALREADY_ASKED, "BRAVO"),
            SLOW_DOWN.to_string(),
            "THAT PARTY IS INVITE ONLY".to_string(),
            // The notice's words with no gamertag in them.
            friend_notice(ASKED_YOU, ""),
        ] {
            a.on_event(LiveEvent::Notice(text.clone()));
            assert!(a.take_cues().is_empty(), "{text}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn presses_that_only_ask_the_server_still_sound() {
        let dir = scratch("cue-asks");
        let (mut a, mut server) = live_app(&dir);
        // The party's leader makes BRAVO leader, and changes its privacy.
        let mut pt = party(7, Activity::Lobby);
        pt.members.push(PartyMember {
            account: 2,
            gamertag: "BRAVO".into(),
            look: Default::default(),
            best: 1,
            level: 1,
            guests: 0,
        });
        view(&mut a).party = Some(pt);
        a.screen = Screen::Party;
        assert!(a.pick("bravo"));
        a.input(Input::A);
        assert_eq!(a.take_cues(), [Cue::Select]);
        assert_eq!(sent(&mut server), [ToServer::Promote(2)]);
        a.input(Input::Y);
        assert_eq!(a.take_cues(), [Cue::Select]);
        assert_eq!(sent(&mut server), [ToServer::Privacy(Privacy::InviteOnly)]);
        // Friend requests answered or taken back.
        view(&mut a).friends = vec![
            friend(12, "YAN", Relation::AskedUs, Online::Offline),
            friend(10, "ZED", Relation::WeAsked, Online::Offline),
        ];
        a.screen = Screen::Friends;
        a.tick(Instant::now());
        a.take_cues();
        for (tag, press, asked) in [
            ("yan", Input::A, ToServer::FriendAccept(12)),
            ("yan", Input::X, ToServer::FriendDecline(12)),
            ("zed", Input::X, ToServer::FriendRemove(10)),
        ] {
            assert!(a.pick(tag));
            a.input(press);
            assert_eq!(a.take_cues(), [Cue::Select], "{tag} {press:?}");
            assert_eq!(sent(&mut server), [asked]);
        }
        // An invitation sent, and a friend request.
        view(&mut a).online = vec![OnlinePlayer {
            account: 21,
            gamertag: "CHARLIE".into(),
            look: Default::default(),
            best: 1,
            activity: Activity::Lobby,
            party: 21,
            open: true,
            size: 1,
            openings: 3,
        }];
        a.screen = Screen::Players;
        a.tick(Instant::now());
        a.take_cues();
        assert!(a.pick("charlie"));
        a.input(Input::A);
        assert_eq!(a.take_cues(), [Cue::Select]);
        assert_eq!(sent(&mut server), [ToServer::Invite(21)]);
        a.input(Input::Y);
        assert_eq!(a.take_cues(), [Cue::Select]);
        assert_eq!(
            sent(&mut server),
            [ToServer::FriendRequest("CHARLIE".into())]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn y_on_someone_who_cant_be_asked_says_why_with_the_error_sound() {
        let dir = scratch("cue-cant-ask");
        let (mut a, mut server) = live_app(&dir);
        use h2net::live::{friend_notice, ALREADY_ASKED, ALREADY_FRIENDS, ASKED_YOU};
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
            player(2, "BRAVO"),
            player(3, "CHARLIE"),
            player(4, "DAN"),
        ];
        view(&mut a).friends = vec![
            friend(2, "BRAVO", Relation::Friend, Online::Launcher),
            friend(3, "CHARLIE", Relation::WeAsked, Online::Launcher),
            friend(4, "DAN", Relation::AskedUs, Online::Launcher),
        ];
        a.screen = Screen::Players;
        a.tick(Instant::now());
        a.take_cues();
        sent(&mut server);
        for (tag, why) in [
            ("BRAVO", ALREADY_FRIENDS),
            ("CHARLIE", ALREADY_ASKED),
            ("DAN", ASKED_YOU),
        ] {
            assert!(a.pick(tag));
            a.input(Input::Y);
            assert_eq!(a.take_cues(), [Cue::Error], "{tag}");
            let said = a.toasts.back().map(|(t, _)| t.clone());
            assert_eq!(said, Some(friend_notice(why, tag)));
            assert!(sent(&mut server).is_empty(), "{tag}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_music_follows_halo_2s_rule_and_stops_for_the_engine() {
        let all = [
            Screen::Intro,
            Screen::Start,
            Screen::Main,
            Screen::SignIn,
            Screen::Connecting,
            Screen::Live,
            Screen::Players,
            Screen::Party,
            Screen::Custom,
            Screen::Pregame,
            Screen::InGame,
            Screen::Carnage,
            Screen::Failed,
            Screen::Record,
            Screen::Friends,
            Screen::Settings,
        ];
        let on = [Screen::Start, Screen::Main];
        let off = [
            Screen::Intro,
            Screen::Live,
            Screen::Party,
            Screen::Custom,
            Screen::Pregame,
            Screen::InGame,
        ];
        for s in all {
            for current in [false, true] {
                let want = if on.contains(&s) {
                    true
                } else if off.contains(&s) {
                    false
                } else {
                    current
                };
                assert_eq!(music_rule(s, current), want, "{s:?} {current}");
            }
        }
        // Main to the playlists turns it off, back to Main on, and the
        // settings keep it.
        let dir = scratch("music");
        let mut a = app(&dir);
        assert!(a.music_on());
        a.input(Input::A);
        a.screen = Screen::Live;
        a.tick(Instant::now());
        assert!(!a.music_on());
        a.screen = Screen::Settings;
        a.tick(Instant::now());
        assert!(!a.music_on());
        a.screen = Screen::Main;
        a.tick(Instant::now());
        assert!(a.music_on());
        a.input(Input::Down);
        a.input(Input::A);
        assert_eq!(a.label(), "settings");
        a.tick(Instant::now());
        assert!(a.music_on());
        // An engine running: off, on any screen; after it, the carnage
        // report stays silent until the main menu.
        carnage(&mut a);
        a.screen = Screen::Main;
        a.game.as_mut().unwrap().child = Some(Running::waiting());
        assert!(a.engine_running());
        a.tick(Instant::now());
        assert!(!a.music_on());
        a.game.as_mut().unwrap().child = None;
        a.screen = Screen::Carnage;
        a.tick(Instant::now());
        assert!(!a.music_on());
        a.screen = Screen::Main;
        a.tick(Instant::now());
        assert!(a.music_on());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kills_and_assists_no_one_has_say_dash_until_the_counts_are_seen() {
        let dir = scratch("carnage-dash");
        let (mut a, _server) = live_app(&dir);
        carnage(&mut a);
        let Ok(mut text) = Text::system() else {
            return;
        };
        let cells = |a: &mut App, text: &mut Text| {
            a.draw(&mut Canvas::new(640, 360), text);
            let d = a.drawn();
            (
                d.iter().filter(|t| *t == "-").count(),
                d.iter().any(|t| t == "5"),
            )
        };
        // Counts someone has show as they are.
        let (dashes, five) = cells(&mut a, &mut text);
        assert!(five);
        // Kills and assists 0 for everyone may not have been sent.
        for r in a.game.as_mut().unwrap().results.iter_mut().flatten() {
            r.kills = 0;
            r.assists = 0;
        }
        let (more, _) = cells(&mut a, &mut text);
        let expected = if crate::results::COUNTS_SEEN { 0 } else { 4 };
        assert_eq!(more - dashes, expected);
        let _ = std::fs::remove_dir_all(&dir);
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
        // Y on ourselves only sounds the error (no notice, nothing sent);
        // on BRAVO it's a friend request.
        a.input(Input::Y);
        assert_eq!(a.take_cues(), [Cue::Error]);
        assert!(a.toasts.is_empty());
        assert!(sent(&mut server).is_empty());
        assert!(a.pick("bravo"));
        assert_eq!(a.csel, 0);
        a.input(Input::Y);
        assert_eq!(a.take_cues(), [Cue::Select]);
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

    #[test]
    fn x_on_a_friend_who_cant_be_joined_says_why() {
        let dir = scratch("cant-join");
        let (mut a, mut server) = live_app(&dir);
        view(&mut a).party = Some(party(500, Activity::Lobby));
        let dan = friend(14, "Dan", Relation::Friend, Online::Launcher);
        view(&mut a).friends = vec![
            dan,
            friend(11, "BOB", Relation::Friend, Online::Offline),
            friend(13, "CAT", Relation::Friend, Online::Game),
        ];
        view(&mut a).online = vec![OnlinePlayer {
            account: 14,
            gamertag: "Dan".into(),
            look: Default::default(),
            best: 2,
            activity: Activity::Lobby,
            party: 114,
            open: false,
            size: 1,
            openings: 0,
        }];
        // RB from the playlists opens the friends list (Halo 2's Y menu
        // loading: the advance alone, as for Y).
        a.input(Input::Rb);
        assert_eq!(a.take_cues(), [Cue::Advance]);
        a.tick(a.now);
        assert!(a.take_cues().is_empty());
        let last = |a: &App| a.toasts.back().map(|(t, _)| t.clone());
        assert!(a.pick("DAN"));
        a.input(Input::X);
        assert_eq!(last(&a).as_deref(), Some("DAN'S PARTY IS INVITE ONLY"));
        assert_eq!(a.take_cues(), [Cue::Error]);
        view(&mut a).online[0].open = true;
        a.input(Input::X);
        assert_eq!(last(&a).as_deref(), Some("DAN'S PARTY IS FULL"));
        assert_eq!(a.take_cues(), [Cue::Error]);
        view(&mut a).online[0].openings = 3;
        a.input(Input::X);
        assert_eq!(last(&a).as_deref(), Some("DAN'S PARTY CAN'T BE JOINED"));
        assert_eq!(a.take_cues(), [Cue::Error]);
        view(&mut a).friends[0].activity = Activity::Playing;
        a.input(Input::X);
        assert_eq!(last(&a).as_deref(), Some("DAN'S PARTY IS IN A MATCH"));
        assert_eq!(a.take_cues(), [Cue::Error]);
        view(&mut a).friends[0].party = 500;
        a.input(Input::X);
        assert_eq!(last(&a).as_deref(), Some("DAN IS ALREADY IN YOUR PARTY"));
        assert_eq!(a.take_cues(), [Cue::Error]);
        assert!(a.pick("BOB"));
        a.input(Input::X);
        assert_eq!(last(&a).as_deref(), Some("BOB IS OFFLINE"));
        assert_eq!(a.take_cues(), [Cue::Error]);
        assert!(a.pick("CAT"));
        a.input(Input::X);
        assert_eq!(
            last(&a).as_deref(),
            Some("CAT IS ON ANOTHER VERSION OF THE GAME")
        );
        assert_eq!(a.take_cues(), [Cue::Error]);
        assert_eq!(a.label(), "friends");
        assert!(sent(&mut server).is_empty());
        // A join the server turns down (the flag was a second old) is told
        // too: its notice is a toast like any other.
        view(&mut a).friends[0].joinable = true;
        assert!(a.pick("DAN"));
        a.input(Input::X);
        assert_eq!(sent(&mut server), [ToServer::JoinParty(500)]);
        // A friend who can be joined: the playlists again with the party
        // joined, loading (X plays no `forward1`).
        assert_eq!(a.take_cues(), [Cue::Advance]);
        assert_eq!(a.label(), "live");
        a.on_event(LiveEvent::Notice("THAT PARTY IS INVITE ONLY".into()));
        assert_eq!(last(&a).as_deref(), Some("THAT PARTY IS INVITE ONLY"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_party_panel_shows_levels_in_the_playlist_selected() {
        let dir = scratch("panel-levels");
        let (mut a, mut server) = live_app(&dir);
        let playlist = |id: u8, name: &str, ranked: bool, level: u8| PlaylistInfo {
            id,
            key: name.to_lowercase(),
            name: name.into(),
            ranked,
            teams: false,
            guests: false,
            min: 2,
            max: 8,
            party_max: 8,
            searching: 0,
            playing: 0,
            level,
            maps: Vec::new(),
        };
        view(&mut a).playlists = vec![
            playlist(11, "Head to Head", true, 3),
            playlist(12, "Rumble Pit", true, 1),
            playlist(13, "Social", false, 0),
        ];
        // The party last played Head to Head: PARTY gives each member's
        // level there (BRAVO's 2).
        let mut pt = party(500, Activity::Lobby);
        pt.members.push(PartyMember {
            account: 2,
            gamertag: "BRAVO".into(),
            look: Default::default(),
            best: 2,
            level: 2,
            guests: 0,
        });
        view(&mut a).party = Some(pt.clone());
        let levels = |a: &App| {
            let pt = a.party().unwrap();
            (
                a.member_level(pt, &pt.members[0]),
                a.member_level(pt, &pt.members[1]),
            )
        };
        a.tick(a.now);
        assert_eq!(levels(&a), (3, 2));
        assert!(sent(&mut server).is_empty());
        // Rumble Pit: ours from the playlists, BRAVO's from his service
        // record, asked for once; "-" until it comes.
        a.input(Input::Down);
        a.tick(a.now);
        assert_eq!(levels(&a), (1, 0));
        assert_eq!(sent(&mut server), [ToServer::Record(2)]);
        a.tick(a.now);
        assert!(sent(&mut server).is_empty());
        let row = h2net::live::PlaylistRecord {
            playlist: 11,
            level: 2,
            games: 4,
            wins: 3,
            tally: Default::default(),
        };
        a.on_event(LiveEvent::ServiceRecord(ServiceRecord {
            account: 2,
            found: Some(h2net::live::Record {
                gamertag: "BRAVO".into(),
                look: Default::default(),
                best: 2,
                created: 0,
                playlists: vec![row],
            }),
        }));
        // He hasn't played Rumble Pit: level 1 there, not his 2.
        assert_eq!(levels(&a), (1, 1));
        // Unranked, and the custom game row: the highest levels.
        a.input(Input::Down);
        assert_eq!(levels(&a), (3, 2));
        a.input(Input::Down);
        assert_eq!(levels(&a), (3, 2));
        a.tick(a.now);
        assert!(sent(&mut server).is_empty());
        // Searching, the playlist searched.
        let mut searching = pt;
        searching.activity = Activity::Searching;
        searching.playlist = 12;
        searching.members[1].level = 1;
        view(&mut a).party = Some(searching);
        assert_eq!(levels(&a), (1, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_party_and_custom_screens_agree_with_the_panel() {
        let dir = scratch("panel-screens");
        let (mut a, mut server) = live_app(&dir);
        let rumble = PlaylistInfo {
            id: 12,
            key: "rumble".into(),
            name: "Rumble Pit".into(),
            ranked: true,
            teams: false,
            guests: false,
            min: 2,
            max: 8,
            party_max: 8,
            searching: 0,
            playing: 0,
            level: 1,
            maps: Vec::new(),
        };
        let mut h2h = rumble.clone();
        (h2h.id, h2h.key, h2h.name, h2h.level) = (11, "h2h".into(), "Head to Head".into(), 3);
        view(&mut a).playlists = vec![h2h, rumble];
        // The party last played Head to Head, where BRAVO is level 2.
        let mut pt = party(500, Activity::Lobby);
        pt.members.push(PartyMember {
            account: 2,
            gamertag: "BRAVO".into(),
            look: Default::default(),
            best: 2,
            level: 2,
            guests: 0,
        });
        view(&mut a).party = Some(pt);
        let bravo = |a: &App| {
            let pt = a.party().unwrap();
            a.member_level(pt, &pt.members[1])
        };
        // Rumble Pit selected, then the party screen (Y): his record is
        // asked for there too, and until it comes he shows "-" as in the
        // panel, not PARTY's 2 from Head to Head.
        a.input(Input::Down);
        a.input(Input::Y);
        assert_eq!(a.label(), "party");
        a.tick(a.now);
        assert_eq!(sent(&mut server), [ToServer::Record(2)]);
        assert_eq!(bravo(&a), 0);
        if let Ok(mut text) = Text::system() {
            let mut c = Canvas::new(1280, 720);
            a.draw(&mut c, &mut text);
            assert!(a.drawn().iter().any(|d| d == "-"));
            assert!(!a.drawn().iter().any(|d| d == "2"));
        }
        // The custom game screen shows the highest levels whatever row was
        // selected before it (a member made leader after a custom game
        // comes back to it from the carnage report), and asks nothing.
        a.screen = Screen::Custom;
        a.tick(a.now);
        assert_eq!(a.label(), "custom");
        assert_eq!(bravo(&a), 2);
        assert!(sent(&mut server).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notices_are_drawn_above_popups() {
        // Clear of every popup's box (the lowest ends at 520) and of the
        // shell's legend at the bottom (from 660).
        assert!(toast_top(2) >= 520.0);
        assert_eq!(legend_top(), 660.0);
        assert!(toast_top(0) + TOAST_H <= legend_top());
        let Ok(mut text) = Text::system() else {
            return;
        };
        let dir = scratch("toasts");
        let mut a = app(&dir);
        for n in ["ONE", "TWO", "THREE"] {
            a.toast(n.into());
        }
        a.popup = Some(Popup::Quit);
        let mut c = Canvas::new(1280, 720);
        a.draw(&mut c, &mut text);
        // Each notice's top edge is drawn at full brightness: not under
        // the popup's shade.
        for k in 0..3 {
            let y = toast_top(k) as usize;
            assert_eq!(c.px[y * 1280 + 640], 0x9F_D0FF, "notice {k}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_server_lost_on_the_main_menu_leaves_the_player_there() {
        let dir = scratch("lost-on-main");
        let (mut a, mut server) = live_app(&dir);
        // B on the playlists: the main menu, still signed in, with the
        // gamertag in the small print.
        a.input(Input::B);
        assert_eq!(a.label(), "main");
        assert_eq!(a.small_print(), "ALPHA");
        // An invitation still pops up here. B declines it.
        view(&mut a).invites.push((9, "BRAVO".into()));
        assert_eq!(a.label(), "invite");
        a.input(Input::B);
        assert_eq!(sent(&mut server), [ToServer::Decline(9)]);
        view(&mut a).invites.clear();
        assert_eq!(a.label(), "main");
        // A accepts it, and shows the playlists, where the new party is;
        // ONLINE has the focus when the player comes back.
        a.input(Input::Down);
        view(&mut a).invites.push((9, "BRAVO".into()));
        a.input(Input::A);
        assert_eq!(sent(&mut server), [ToServer::Accept(9)]);
        view(&mut a).invites.clear();
        assert_eq!(a.label(), "live");
        a.input(Input::B);
        assert_eq!((a.label().as_str(), a.main_focus.item), ("main", LIVE_ROW));
        // The settings opened from here stay up when the server is lost,
        // with the reason as a notice, and still go back here.
        a.input(Input::Down);
        a.input(Input::A);
        assert_eq!(a.label(), "settings");
        a.on_event(LiveEvent::Lost("restarting".into()));
        assert_eq!(a.label(), "settings");
        assert!(a
            .toasts
            .iter()
            .any(|t| t.0 == "Lost the server: restarting"));
        a.tick(Instant::now());
        assert_eq!(a.label(), "settings");
        a.input(Input::B);
        assert_eq!(a.label(), "main");
        // Signed in again for the rest.
        let (mut a, _server) = live_app(&dir);
        a.input(Input::B);
        a.on_event(LiveEvent::Lost("the connection closed".into()));
        assert_eq!(a.label(), "main");
        assert!(a.client.is_none());
        assert_eq!(a.failed, "Lost the server: the connection closed");
        assert!(a.toasts.iter().any(|t| t.0 == a.failed));
        // Signed out, the small print is the gamertag saved (none here).
        assert_eq!(a.small_print(), "");
        // On the start screen too.
        a.input(Input::B);
        a.fail("The server didn't answer.".into());
        assert_eq!(a.label(), "start");
        // Then ONLINE signs in again (the form, as none was saved).
        a.input(Input::A);
        a.input(Input::A);
        assert_eq!(a.label(), "signin");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_click_focuses_a_main_menu_row_then_picks_it() {
        let dir = scratch("main-click");
        let mut a = app(&dir);
        let mut c = Canvas::new(1280, 720);
        a.draw_menu(&mut c, &mut Vec::new());
        // A double-click on PRESS START, which lies over ONLINE: the
        // first click opens the main menu, and the second only focuses
        // the row under it.
        let space = Space::new(1280, 720);
        let [px, py] = space.at([0.0, -85.0]);
        a.click(px, py);
        assert_eq!(a.label(), "main");
        a.draw_menu(&mut c, &mut Vec::new());
        a.click(px, py);
        assert_eq!((a.label().as_str(), a.main_focus.item), ("main", LIVE_ROW));
        a.last_click = None;
        // As drawn last in a 1280 by 720 window, in the built-in layout:
        // row 1's label box is from y -120 to -170 in Halo 2's UI units,
        // across -240 to 240.
        let [x, y] = space.at([0.0, -145.0]);
        a.click(x, y);
        assert_eq!(a.label(), "main");
        assert_eq!(a.main_focus.item, MAIN_SETTINGS_ROW);
        // Beside the rows: nothing.
        let [bx, by] = space.at([300.0, -145.0]);
        a.click(bx, by);
        assert_eq!((a.label().as_str(), a.main_focus.item), ("main", 1));
        a.click(x, y);
        assert_eq!(a.label(), "settings");
        a.input(Input::B);
        // ONLINE, clicked twice.
        let [x, y] = space.at([0.0, -95.0]);
        a.click(x, y);
        assert_eq!(a.main_focus.item, LIVE_ROW);
        a.click(x, y);
        assert_eq!(a.label(), "signin");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_start_screen_and_main_menu_draw_in_the_flat_look() {
        let dir = scratch("menus-draw");
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let kept = lines.clone();
        let mut a = App::new(Config {
            log: Arc::new(move |l: &str| kept.lock().unwrap().push(l.to_string())),
            ..app(&dir).cfg
        });
        let (w, h) = (320, 180);
        // The still background on its own, to tell the screens from.
        let menus = Menus::flat();
        let (art, fonts) = (menus.shell.art(), &menus.fonts);
        let res = Resources { art, fonts };
        let space = Space::new(w as u32, h as u32);
        let framing = &menus.shell.main.framing;
        let behind = screens::background(0.0, Backdrop::Still, framing, art, fonts, space);
        let mut background = vec![0u32; w * h];
        Kept::default().draw(&behind, &mut background, w, h, &res);
        let mut shown = Vec::new();
        for label in ["start", "main"] {
            assert_eq!(a.label(), label);
            // It opens now (as `draw` does when the screen changes).
            a.drawn_screen = Some(a.screen);
            a.menu_opened = a.menu_clock();
            let mut c = Canvas::new(w, h);
            let mut drawn = Vec::new();
            a.draw_menu(&mut c, &mut drawn);
            // Just opened, nothing has faded in: nothing to `see` yet.
            assert!(drawn.is_empty(), "{label}: {drawn:?}");
            // Long after it opened, with every intro over.
            a.menu_opened = a.menu_clock() - 5.0;
            a.main_focus = Focus::on(a.main_focus.item, a.menu_opened);
            assert!(a.menu_settled(), "{label}");
            a.draw_menu(&mut c, &mut drawn);
            assert!(c.px.iter().all(|&p| p >> 24 == 0), "{label}");
            // The screen is drawn over the background, not just it.
            assert_ne!(c.px, background, "{label}");
            match label {
                "start" => assert_eq!(drawn, [screens::PRESS_START]),
                _ => assert_eq!(drawn, MAIN_ROWS),
            }
            shown.push(c.px);
            a.input(Input::A);
        }
        assert_ne!(shown[0], shown[1], "the two screens differ");
        // The frames are timed once the background is kept, and the log
        // says so once for the size.
        let mut c = Canvas::new(w, h);
        a.screen = Screen::Start;
        for _ in 0..=MENU_FRAMES {
            a.draw_menu(&mut c, &mut Vec::new());
        }
        assert_eq!(a.menu_times.logged, [((320, 180), "start screen")]);
        let timed = |lines: &[String]| {
            lines
                .iter()
                .filter(|l| l.starts_with("menus: 320x180, start screen "))
                .inspect(|l| assert!(l.contains(" ms a frame on the CPU (background "), "{l}"))
                .count()
        };
        assert_eq!(timed(&lines.lock().unwrap()), 1);
        for _ in 0..MENU_FRAMES * 2 {
            a.draw_menu(&mut c, &mut Vec::new());
        }
        assert_eq!(timed(&lines.lock().unwrap()), 1);
        // Through `draw`, when there is a font to draw the rest with.
        let Ok(mut text) = Text::system() else {
            return;
        };
        a.saved[0] = "CHIEF".into();
        a.screen = Screen::Main;
        a.draw(&mut c, &mut text);
        a.menu_opened -= 5.0;
        a.draw(&mut c, &mut text);
        assert!(a.drawn().iter().any(|d| d == "ONLINE"));
        assert!(a.drawn().iter().any(|d| d == "CHIEF"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A flythrough (`SceneSource`) whose frames are all one colour and
    /// come as soon as they are asked for, once it is `ready`; it keeps
    /// what it was asked.
    struct FakeScene {
        colour: u32,
        ready: Mutex<bool>,
        state: Mutex<SceneState>,
        asks: Mutex<Vec<(usize, usize, f64)>>,
        sleeps: Mutex<u32>,
        frame: Mutex<Option<Arc<Frame>>>,
    }

    impl FakeScene {
        fn new(colour: u32) -> Arc<FakeScene> {
            Arc::new(FakeScene {
                colour,
                ready: Mutex::new(true),
                state: Mutex::new(SceneState::Ready),
                asks: Mutex::new(Vec::new()),
                sleeps: Mutex::new(0),
                frame: Mutex::new(None),
            })
        }

        fn asks(&self) -> Vec<(usize, usize, f64)> {
            self.asks.lock().unwrap().clone()
        }

        fn sleeps(&self) -> u32 {
            *self.sleeps.lock().unwrap()
        }
    }

    impl SceneSource for FakeScene {
        fn ask(&self, w: usize, h: usize, seconds: f64) {
            self.asks.lock().unwrap().push((w, h, seconds));
            if *self.ready.lock().unwrap() {
                *self.frame.lock().unwrap() = Some(Arc::new(Frame {
                    w,
                    h,
                    seconds,
                    px: vec![self.colour; w * h],
                }));
            }
        }

        fn latest(&self, w: usize, h: usize) -> Option<Arc<Frame>> {
            let frame = self.frame.lock().unwrap().clone();
            frame.filter(|f| f.w == w && f.h == h)
        }

        fn sleep(&self) {
            *self.sleeps.lock().unwrap() += 1;
        }

        fn state(&self) -> SceneState {
            self.state.lock().unwrap().clone()
        }
    }

    /// The lobby with `scene` behind its menus, and the lines it logs.
    fn scene_app(dir: &std::path::Path, scene: &Arc<FakeScene>) -> (App, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let kept = lines.clone();
        let source: Arc<dyn SceneSource> = scene.clone();
        let a = App::new(Config {
            log: Arc::new(move |l: &str| kept.lock().unwrap().push(l.to_string())),
            scene: Some(source),
            ..app(dir).cfg
        });
        (a, lines)
    }

    /// How many of `px` are `colour`.
    fn count_of(px: &[u32], colour: u32) -> usize {
        px.iter().filter(|&&p| p == colour).count()
    }

    /// The flythrough's colour in the fakes: no still background or menu
    /// art has it.
    const SCENE_COLOUR: u32 = 0x0012_3456;

    #[test]
    fn the_flythrough_fades_in_behind_the_start_screen_and_main_menu() {
        let dir = scratch("scene-menus");
        let scene = FakeScene::new(SCENE_COLOUR);
        let (mut a, _) = scene_app(&dir, &scene);
        let (w, h) = (320, 180);
        let mut c = Canvas::new(w, h);
        for label in ["start", "main"] {
            assert_eq!(a.label(), label);
            a.drawn_screen = Some(a.screen);
            a.menu_opened = a.menu_clock() - 5.0;
            a.main_focus = Focus::on(a.main_focus.item, a.menu_opened);
            // With no intro to wake it ahead, the first frame fades in
            // as it comes: this draw is still the still background.
            a.scene_since = None;
            a.draw_menu(&mut c, &mut Vec::new());
            assert_eq!(scene.asks().last().map(|k| (k.0, k.1)), Some((w, h)));
            assert_eq!(count_of(&c.px, SCENE_COLOUR), 0, "{label}");
            let still = c.px.clone();
            // Half way: neither the still background nor the scene.
            a.scene_since = Some(a.menu_clock() - SCENE_FADE / 2.0);
            a.draw_menu(&mut c, &mut Vec::new());
            assert_eq!(count_of(&c.px, SCENE_COLOUR), 0, "{label}");
            assert_ne!(c.px, still, "{label}");
            // Faded in: the scene is behind the screen, which is still
            // drawn over it.
            a.scene_since = Some(a.menu_clock() - 1.0);
            let mut drawn = Vec::new();
            a.draw_menu(&mut c, &mut drawn);
            let n = count_of(&c.px, SCENE_COLOUR);
            assert!(n > w * h / 2 && n < w * h, "{label}: {n}");
            assert!(a.scene_shown, "{label}");
            match label {
                "start" => assert_eq!(drawn, [screens::PRESS_START]),
                _ => assert_eq!(drawn, MAIN_ROWS),
            }
            a.input(Input::A);
        }
        // The flythrough's clock: on with the menus' clock, at most
        // `SCENE_STEP` a frame.
        a.screen = Screen::Main;
        let before = scene.asks().last().unwrap().2;
        a.scene_step = Some(a.menu_clock() - 10.0);
        a.draw_menu(&mut c, &mut Vec::new());
        let after = scene.asks().last().unwrap().2;
        assert!(after > before && after - before <= SCENE_STEP + 1e-9);
        // Another size: the still background until a frame of it comes.
        *scene.ready.lock().unwrap() = false;
        let mut big = Canvas::new(400, 200);
        a.draw_menu(&mut big, &mut Vec::new());
        assert_eq!(count_of(&big.px, SCENE_COLOUR), 0);
        assert!(!a.scene_shown);
        assert_eq!(a.scene_since, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_intro_wakes_the_flythrough_so_the_start_screen_has_it_at_once() {
        let dir = scratch("scene-intro");
        let (mut a, _) = intro_app(&dir, intro::FakeSource::second());
        let scene = FakeScene::new(SCENE_COLOUR);
        let source: Arc<dyn SceneSource> = scene.clone();
        a.cfg.scene = Some(source);
        let mut text = Text::none();
        let (w, h) = (320, 180);
        let mut c = Canvas::new(w, h);
        let t0 = Instant::now();
        // Nothing is asked before the intro's first draw.
        a.tick(t0);
        assert!(scene.asks().is_empty());
        // Its first draw asks for the scene's first frame at the window's
        // size, once; it isn't put to sleep while the movie plays, and
        // isn't drawn under it.
        for _ in 0..5 {
            a.draw(&mut c, &mut text);
            a.tick(t0);
        }
        assert_eq!(a.label(), "intro");
        assert_eq!(scene.asks(), [(w, h, 0.0)]);
        assert_eq!(scene.sleeps(), 0);
        assert_eq!(count_of(&c.px, SCENE_COLOUR), 0);
        // A new size asks again, once.
        let mut big = Canvas::new(400, 200);
        a.draw(&mut big, &mut text);
        a.draw(&mut big, &mut text);
        assert_eq!(scene.asks().len(), 2);
        assert_eq!(scene.asks()[1], (400, 200, 0.0));
        a.draw(&mut c, &mut text);
        assert_eq!(scene.asks().len(), 3);
        // Skipped: the start screen's first draw has the scene whole
        // behind it (no fade from the still background, so no framing
        // showing through), from the loop's start.
        a.input(Input::A);
        assert_eq!(a.label(), "start");
        a.tick(t0);
        assert_eq!(scene.sleeps(), 0);
        a.draw(&mut c, &mut text);
        let n = count_of(&c.px, SCENE_COLOUR);
        assert!(n > w * h / 2, "{n}");
        assert!(a.scene_shown);
        assert_eq!(scene.asks().last(), Some(&(w, h, 0.0)));
        // A frame that only comes later (the window resized) fades in:
        // its first draw is still the still background.
        *scene.ready.lock().unwrap() = false;
        a.draw(&mut big, &mut text);
        *scene.ready.lock().unwrap() = true;
        a.draw(&mut big, &mut text);
        assert_eq!(count_of(&big.px, SCENE_COLOUR), 0);
        assert!(a.scene_since.is_some());
        // A scene that can't be drawn isn't asked for during the intro.
        let (mut b, _) = intro_app(&dir, intro::FakeSource::second());
        let failed = FakeScene::new(SCENE_COLOUR);
        *failed.state.lock().unwrap() = SceneState::Failed("no GPU".into());
        let source: Arc<dyn SceneSource> = failed.clone();
        b.cfg.scene = Some(source);
        b.draw(&mut c, &mut text);
        assert_eq!(b.label(), "intro");
        assert!(failed.asks().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_flythrough_sleeps_away_from_the_menus() {
        let dir = scratch("scene-sleep");
        let scene = FakeScene::new(SCENE_COLOUR);
        let (mut a, _) = scene_app(&dir, &scene);
        let (w, h) = (320, 180);
        let mut c = Canvas::new(w, h);
        let mut text = Text::none();
        // Not asked for before the menus show, nor put to sleep.
        a.tick(Instant::now());
        assert!(scene.asks().is_empty());
        assert_eq!(scene.sleeps(), 0);
        a.input(Input::A);
        assert_eq!(a.label(), "main");
        a.draw(&mut c, &mut text);
        a.scene_since = Some(a.menu_clock() - 1.0);
        a.draw(&mut c, &mut text);
        assert!(count_of(&c.px, SCENE_COLOUR) > w * h / 2);
        assert_eq!(a.frame_interval(), FRAME);
        // The settings: no scene behind them, and it sleeps (once).
        a.input(Input::Down);
        a.input(Input::A);
        assert_eq!(a.label(), "settings");
        let asked = scene.asks().len();
        a.tick(Instant::now());
        a.draw(&mut c, &mut text);
        a.tick(Instant::now());
        assert_eq!(count_of(&c.px, SCENE_COLOUR), 0);
        assert_eq!(scene.asks().len(), asked);
        assert_eq!(scene.sleeps(), 1);
        assert!(!a.scene_shown);
        // Back on the main menu it wakes: its last frame is there at once,
        // and its clock went on from where it stopped.
        let stopped = scene.asks().last().unwrap().2;
        a.input(Input::B);
        assert_eq!(a.label(), "main");
        a.draw(&mut c, &mut text);
        assert!(count_of(&c.px, SCENE_COLOUR) > w * h / 2);
        assert!((scene.asks().last().unwrap().2 - stopped).abs() < 1e-9);
        a.tick(Instant::now());
        assert_eq!(scene.sleeps(), 1);
        // ONLINE's screens have none either.
        let (mut a, _server) = live_app(&dir);
        let scene = FakeScene::new(SCENE_COLOUR);
        let source: Arc<dyn SceneSource> = scene.clone();
        a.cfg.scene = Some(source);
        a.draw(&mut c, &mut text);
        assert_eq!(a.label(), "live");
        assert_eq!(count_of(&c.px, SCENE_COLOUR), 0);
        assert!(scene.asks().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_match_puts_the_flythrough_to_sleep_before_its_engine_starts() {
        let dir = scratch("scene-match");
        let scene = FakeScene::new(SCENE_COLOUR);
        let source: Arc<dyn SceneSource> = scene.clone();
        // A match needs the server's address: a real connection.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let ours = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let _theirs = listener.accept().unwrap();
        let mut a = live_app_over(&dir, Connection::tcp(ours).unwrap());
        a.cfg.scene = Some(source);
        // On the main menu (signed in), drawing the scene.
        a.screen = Screen::Main;
        let mut c = Canvas::new(64, 36);
        a.draw_menu(&mut c, &mut Vec::new());
        assert_eq!(scene.asks().len(), 1);
        a.on_event(LiveEvent::LauncherMatch(a_match()));
        assert_eq!(a.label(), "pregame");
        assert_eq!(scene.sleeps(), 1);
        // While the match is on, nothing is asked for, even on the menus.
        a.screen = Screen::Main;
        a.draw_menu(&mut c, &mut Vec::new());
        assert_eq!(scene.asks().len(), 1);
        assert_eq!(scene.sleeps(), 1);
        assert_eq!(count_of(&c.px, SCENE_COLOUR), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_flythrough_that_fails_leaves_the_still_background_and_says_why_once() {
        let dir = scratch("scene-failed");
        let scene = FakeScene::new(SCENE_COLOUR);
        *scene.state.lock().unwrap() = SceneState::Failed("no GPU".into());
        let (mut a, lines) = scene_app(&dir, &scene);
        let mut c = Canvas::new(64, 36);
        for _ in 0..3 {
            a.draw_menu(&mut c, &mut Vec::new());
            a.tick(Instant::now());
        }
        assert!(scene.asks().is_empty());
        assert_eq!(count_of(&c.px, SCENE_COLOUR), 0);
        let said: Vec<String> = lines
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("scene"))
            .cloned()
            .collect();
        assert_eq!(
            said,
            ["menus: the scene can't be drawn (no GPU); the still background from now on"]
        );
        // Not read yet: the still background, while it is asked for.
        let scene = FakeScene::new(SCENE_COLOUR);
        *scene.state.lock().unwrap() = SceneState::Loading;
        *scene.ready.lock().unwrap() = false;
        let (mut a, _) = scene_app(&dir, &scene);
        a.draw_menu(&mut c, &mut Vec::new());
        assert_eq!(scene.asks().len(), 1);
        assert_eq!(count_of(&c.px, SCENE_COLOUR), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_window_draws_faster_behind_the_flythrough_only_when_asked() {
        let dir = scratch("scene-fps");
        let scene = FakeScene::new(SCENE_COLOUR);
        let (mut a, _) = scene_app(&dir, &scene);
        let mut c = Canvas::new(64, 36);
        a.draw_menu(&mut c, &mut Vec::new());
        assert!(a.scene_shown);
        assert_eq!(a.frame_interval(), FRAME);
        a.cfg.scene_fast = true;
        assert_eq!(a.frame_interval(), FRAME_MOVIE);
        // Not on the other screens, nor before the scene shows.
        a.screen = Screen::Settings;
        assert_eq!(a.frame_interval(), FRAME);
        a.screen = Screen::Start;
        a.scene_shown = false;
        assert_eq!(a.frame_interval(), FRAME);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_flythrough_cross_fades_in_over_a_quarter_second() {
        assert_eq!(scene_alpha(-1.0), 0.0);
        assert_eq!(scene_alpha(0.0), 0.0);
        assert!((scene_alpha(SCENE_FADE / 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(scene_alpha(SCENE_FADE), 1.0);
        assert_eq!(scene_alpha(60.0), 1.0);
        let under = [0x0000_0000, 0x00FF_FFFF, 0x0010_2030];
        let top = [0x00FF_8040, 0x0000_0000, 0x0010_2030];
        let blend = |alpha: f32| {
            let mut px = under;
            blend_px(&mut px, &top, alpha);
            px
        };
        assert_eq!(blend(0.0), under);
        assert_eq!(blend(1.0), top);
        assert_eq!(blend(0.5), [0x0080_4020, 0x0080_8080, 0x0010_2030]);
        assert_eq!(blend(0.25)[0], 0x0040_2010);
    }

    #[test]
    fn the_menus_log_line_says_how_long_the_scenes_copy_took() {
        let dir = scratch("scene-timer");
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let kept = lines.clone();
        let mut a = App::new(Config {
            log: Arc::new(move |l: &str| kept.lock().unwrap().push(l.to_string())),
            ..app(&dir).cfg
        });
        a.screen = Screen::Main;
        a.menu_times.backgrounds.push(((640, 360), 20.0));
        for _ in 0..=MENU_FRAMES {
            a.time_frame((640, 360), 3.0, true, Some(1.5));
        }
        a.screen = Screen::Start;
        for _ in 0..MENU_FRAMES {
            a.time_frame((1280, 720), 3.0, true, Some(1.5));
        }
        a.time_frame((1280, 720), 3.0, true, Some(1.5));
        // At the same size, a screen that copied none in (the scene
        // failed after the start screen's) says nothing of copies.
        a.screen = Screen::Main;
        a.menu_times.backgrounds.push(((1280, 720), 20.0));
        for _ in 0..MENU_FRAMES {
            a.time_frame((1280, 720), 3.0, true, None);
        }
        // Each screen's copy time is its own.
        a.screen = Screen::Start;
        for _ in 0..=MENU_FRAMES {
            a.time_frame((320, 180), 3.0, true, Some(1.0));
        }
        a.screen = Screen::Main;
        for i in 0..MENU_FRAMES {
            a.time_frame((320, 180), 3.0, true, (i % 2 == 0).then_some(4.0));
        }
        assert_eq!(
            *lines.lock().unwrap(),
            [
                "menus: 640x360, main menu 3.0 ms a frame on the CPU (the scene's frame copied in 1.5 ms; background 20 ms once)",
                "menus: 1280x720, start screen 3.0 ms a frame on the CPU (the scene's frame copied in 1.5 ms)",
                "menus: 1280x720, main menu 3.0 ms a frame on the CPU (background 20 ms once)",
                "menus: 320x180, start screen 3.0 ms a frame on the CPU (the scene's frame copied in 1.0 ms)",
                "menus: 320x180, main menu 3.0 ms a frame on the CPU (the scene's frame copied in 4.0 ms)",
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_screen_is_timed_once_per_window_size() {
        let dir = scratch("timer");
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let kept = lines.clone();
        let mut a = App::new(Config {
            log: Arc::new(move |l: &str| kept.lock().unwrap().push(l.to_string())),
            ..app(&dir).cfg
        });
        let frames = |a: &mut App, screen: Screen, size: (usize, usize), n: u32| {
            a.screen = screen;
            for _ in 0..n {
                a.time_frame(size, 1.0, true, None);
            }
        };
        let logged = || lines.lock().unwrap().clone();
        // The first frame at a size isn't timed.
        frames(&mut a, Screen::Players, (640, 360), MENU_FRAMES);
        assert!(logged().is_empty(), "{:?}", logged());
        frames(&mut a, Screen::Players, (640, 360), 1);
        assert_eq!(
            logged(),
            ["lobby: 640x360, players 1.0 ms a frame on the CPU"]
        );
        // Once per screen and size.
        frames(&mut a, Screen::Players, (640, 360), MENU_FRAMES * 2);
        assert_eq!(logged().len(), 1);
        // Frames that aren't timed don't count.
        a.screen = Screen::Party;
        for _ in 0..MENU_FRAMES {
            a.time_frame((640, 360), 9.0, false, None);
        }
        frames(&mut a, Screen::Party, (640, 360), MENU_FRAMES - 1);
        assert_eq!(logged().len(), 1);
        frames(&mut a, Screen::Party, (640, 360), 1);
        assert_eq!(
            logged()[1],
            "lobby: 640x360, party 1.0 ms a frame on the CPU"
        );
        // Another size: once more there; back at the first, nothing new.
        frames(&mut a, Screen::Players, (1920, 1080), MENU_FRAMES + 1);
        assert_eq!(
            logged()[2],
            "lobby: 1920x1080, players 1.0 ms a frame on the CPU"
        );
        frames(&mut a, Screen::Players, (640, 360), MENU_FRAMES * 2);
        assert_eq!(logged().len(), 3);
        // The menus keep their line, with the background's time.
        a.menu_times.backgrounds.push(((640, 360), 20.0));
        frames(&mut a, Screen::Main, (640, 360), MENU_FRAMES);
        assert_eq!(
            logged()[3],
            "menus: 640x360, main menu 1.0 ms a frame on the CPU (background 20 ms once)"
        );
        // Every screen has a name of its own.
        let screens = [
            Screen::Intro,
            Screen::Start,
            Screen::Main,
            Screen::SignIn,
            Screen::Connecting,
            Screen::Live,
            Screen::Players,
            Screen::Party,
            Screen::Custom,
            Screen::Pregame,
            Screen::InGame,
            Screen::Carnage,
            Screen::Failed,
            Screen::Record,
            Screen::Friends,
            Screen::Settings,
        ];
        let mut names: Vec<&str> = screens
            .iter()
            .map(|&s| {
                a.screen = s;
                a.timer_name()
            })
            .collect();
        // The playlists while the party searches have their own.
        let (mut b, _server) = live_app(&dir);
        view(&mut b).party = Some(party(500, Activity::Searching));
        assert_eq!(b.timer_name(), "searching");
        names.push(b.timer_name());
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), screens.len() + 1);
        // Through `draw` on a lobby screen, when there is a font.
        let Ok(mut text) = Text::system() else {
            return;
        };
        a.screen = Screen::Settings;
        let mut c = Canvas::new(320, 180);
        for _ in 0..=MENU_FRAMES {
            a.draw(&mut c, &mut text);
        }
        let line = logged().pop().unwrap();
        assert!(
            line.starts_with("lobby: 320x180, settings ")
                && line.ends_with(" ms a frame on the CPU"),
            "{line}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A lobby opening on the intro, played from the made-up `source`,
    /// with its log.
    fn intro_app(
        dir: &std::path::Path,
        source: intro::FakeSource,
    ) -> (App, Arc<Mutex<Vec<String>>>) {
        intro_app_with(dir, source, None)
    }

    /// `intro_app`, with the small movie too when `low` names it.
    fn intro_app_with(
        dir: &std::path::Path,
        source: intro::FakeSource,
        low: Option<&str>,
    ) -> (App, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let kept = lines.clone();
        let slot = Mutex::new(Some(source));
        let open: intro::Opener = Arc::new(move |_: &std::path::Path| {
            let source = slot.lock().unwrap().take().expect("opened once");
            Box::new(source) as Box<dyn intro::MovieSource>
        });
        let a = App::new(Config {
            log: Arc::new(move |l: &str| kept.lock().unwrap().push(l.to_string())),
            intro: Some(Plan {
                full: Some(PathBuf::from("intro_60.wmv")),
                low: low.map(PathBuf::from),
                open,
            }),
            ..app(dir).cfg
        });
        (a, lines)
    }

    /// Tick `a` at `t` until `done`, giving the decoder's thread time to
    /// send (the lobby's clock stays at `t`).
    fn tick_until(a: &mut App, t: Instant, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            a.tick(t);
            if done(a) {
                return;
            }
            assert!(Instant::now() < deadline, "still waiting on {}", a.label());
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The red of the intro's picture showing, if one is.
    fn intro_red(a: &App) -> Option<u8> {
        let shown = a.intro.as_ref().and_then(Intro::showing);
        shown.map(|(p, _)| p.rgba[0])
    }

    #[test]
    fn the_intro_plays_before_the_start_screen_and_goes_on_at_its_end() {
        let dir = scratch("intro-end");
        let (mut a, lines) = intro_app(&dir, intro::FakeSource::second());
        let mut text = Text::none();
        assert_eq!(a.label(), "intro");
        assert_eq!(a.frame_interval(), FRAME_MOVIE);
        let t0 = Instant::now();
        a.tick(t0);
        assert!(!a.music_on());
        // Nothing is decoded before the first draw, which starts it, on
        // black.
        assert!(a.intro.is_none());
        let mut c = Canvas::new(64, 36);
        c.px.fill(0xFF_FFFF);
        a.draw(&mut c, &mut text);
        assert!(c.px.iter().all(|&p| p == 0));
        assert!(a.intro.is_some());
        assert_eq!(
            lines.lock().unwrap()[0],
            "intro: playing intro_60.wmv (64x36 window)"
        );
        // Its soundtrack comes first, for the sound to play, and the
        // movie's clock starts.
        tick_until(&mut a, t0, |a| intro_red(a).is_some());
        let sound = a.take_sound();
        assert!(
            matches!(sound.as_slice(), [SoundOut::Movie(_)]),
            "{sound:?}"
        );
        // Pictures by their time (red is the picture's number), as large
        // as fits: a 4:3 picture in a 16:9 window has bars at its sides.
        let red = |c: &Canvas, x: usize| (c.px[18 * c.w + x] >> 16) & 0xFF;
        for (ms, k) in [(510, 15), (680, 20)] {
            let t = t0 + Duration::from_millis(ms);
            tick_until(&mut a, t, |a| intro_red(a) == Some(k));
            a.draw(&mut c, &mut text);
            assert!(red(&c, 32).abs_diff(u32::from(k)) <= 1, "{}", red(&c, 32));
            assert_eq!(c.px[18 * 64 + 2], 0);
            assert_eq!(c.px[18 * 64 + 61], 0);
        }
        assert_eq!(a.label(), "intro");
        assert!(!a.music_on());
        // The decoder finished, and the movie played out: the start
        // screen, its music, and no cue.
        tick_until(&mut a, t0 + Duration::from_secs(2), |a| {
            a.screen == Screen::Start
        });
        assert_eq!(a.take_sound(), [SoundOut::StopMovie]);
        let played = lines.lock().unwrap().last().cloned().unwrap();
        assert!(
            played.starts_with("intro: played to the end (") && played.contains(" 30 pictures, "),
            "{played}"
        );
        assert!(a.music_on());
        assert_eq!(a.frame_interval(), FRAME);
        // The start screen opens at its first draw (its intro animations
        // start then).
        a.draw(&mut c, &mut text);
        assert_eq!(a.drawn_screen, Some(Screen::Start));
        assert!(a.intro.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn any_button_or_a_click_skips_the_intro_but_a_direction_doesnt() {
        let dir = scratch("intro-skip");
        let skips = [
            Input::A,
            Input::B,
            Input::X,
            Input::Y,
            Input::Lb,
            Input::Rb,
            Input::Lt,
            Input::Rt,
            Input::Backspace,
            Input::Char(' '),
            Input::Char('z'),
        ];
        for i in skips {
            let (mut a, lines) = intro_app(&dir, intro::FakeSource::second());
            let t0 = Instant::now();
            a.tick(t0);
            a.draw(&mut Canvas::new(64, 36), &mut Text::none());
            tick_until(&mut a, t0, |a| intro_red(a).is_some());
            for d in [
                Input::Up,
                Input::Down,
                Input::Left,
                Input::Right,
                Input::Tab,
            ] {
                a.input(d);
                assert_eq!(a.label(), "intro", "{d:?}");
            }
            a.input(i);
            assert_eq!(a.label(), "start", "{i:?}");
            // The soundtrack stops, and leaving sounds nothing.
            let sound = a.take_sound();
            assert_eq!(sound.last(), Some(&SoundOut::StopMovie), "{i:?}");
            assert!(
                !sound.iter().any(|s| matches!(s, SoundOut::Cue(_))),
                "{i:?}"
            );
            let skipped = lines.lock().unwrap().last().cloned().unwrap();
            assert!(skipped.starts_with("intro: skipped at "), "{skipped}");
            assert!(a.intro.is_none());
            a.tick(t0);
            assert!(a.music_on());
            // The start screen takes the next press as ever.
            if i == Input::A {
                a.input(Input::A);
                assert_eq!(a.label(), "main");
            }
        }
        // A click anywhere, and the second half of a double-click doesn't
        // go on from the start screen.
        let (mut a, _) = intro_app(&dir, intro::FakeSource::second());
        a.tick(Instant::now());
        a.draw(&mut Canvas::new(64, 36), &mut Text::none());
        a.click(10.0, 10.0);
        assert_eq!(a.label(), "start");
        assert_eq!(a.take_sound(), [SoundOut::StopMovie]);
        a.click(10.0, 10.0);
        assert_eq!(a.label(), "start");
        // Without an intro (no movie, headless, switched off) the lobby
        // opens on the start screen, with its music.
        let a = app(&dir);
        assert_eq!(a.label(), "start");
        assert!(a.music_on());
        assert_eq!(a.frame_interval(), FRAME);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_key_held_to_skip_the_intro_skips_only_the_movie() {
        // Windows repeats a held key; Halo 2 repeats only directions, so
        // the repeats of a key that is a button do nothing: Enter held
        // through the intro leaves the start screen showing rather than
        // going on to the main menu and signing in, and Esc held doesn't
        // open and shut the Quit popup.
        let dir = scratch("intro-held");
        let buttons = [
            Input::A,
            Input::B,
            Input::Lb,
            Input::Rb,
            Input::Backspace,
            Input::Char(' '),
            Input::Char('a'),
            Input::Char('b'),
            Input::Char('q'),
            Input::Char('e'),
        ];
        for held in buttons {
            let (mut a, _) = intro_app(&dir, intro::FakeSource::second());
            let t0 = Instant::now();
            a.tick(t0);
            a.draw(&mut Canvas::new(64, 36), &mut Text::none());
            tick_until(&mut a, t0, |a| intro_red(a).is_some());
            a.key(held);
            assert_eq!(a.label(), "start", "{held:?}");
            a.take_sound();
            for _ in 0..5 {
                a.key_repeated(held);
            }
            assert_eq!(a.label(), "start", "{held:?}");
            assert_eq!(a.popup, None, "{held:?}");
            assert!(a.take_cues().is_empty(), "{held:?}");
        }
        // On the main menu the arrows still repeat, and the buttons don't.
        let mut a = app(&dir);
        a.key(Input::A);
        assert_eq!(a.label(), "main");
        let row = a.main_focus.item;
        a.key_repeated(Input::Down);
        assert_eq!(a.main_focus.item, row + 1);
        a.key_repeated(Input::Up);
        assert_eq!(a.main_focus.item, row);
        a.take_cues();
        for held in buttons {
            a.key_repeated(held);
            assert_eq!(a.label(), "main", "{held:?}");
            assert_eq!(a.main_focus.item, row, "{held:?}");
        }
        assert!(a.take_cues().is_empty());
        // Where a name is typed, the letters and Backspace repeat.
        a.key(Input::A);
        assert_eq!(a.label(), "signin");
        a.field = 0;
        for c in "kab".chars() {
            a.key_repeated(Input::Char(c));
        }
        assert_eq!(a.fields[0], "KAB");
        a.key_repeated(Input::Backspace);
        assert_eq!(a.fields[0], "KA");
        a.key_repeated(Input::A);
        assert_eq!(a.label(), "signin");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_triggers_do_nothing_but_skip_the_intro() {
        // Halo 2 skips its movies on any button but the d-pad and the
        // stick clicks, the triggers too (0x156ab0, above); elsewhere the
        // lobby gives them nothing to do, so they change nothing and
        // sound nothing.
        let dir = scratch("triggers");
        let mut a = app(&dir);
        let mut text = Text::none();
        let mut c = Canvas::new(320, 180);
        a.draw(&mut c, &mut text);
        for t in [Input::Lt, Input::Rt] {
            a.input(t);
            assert_eq!(a.label(), "start", "{t:?}");
        }
        a.input(Input::A);
        assert_eq!(a.label(), "main");
        a.take_sound();
        a.draw(&mut c, &mut text);
        let row = a.main_focus.item;
        for t in [Input::Lt, Input::Rt] {
            a.input(t);
            assert_eq!(a.label(), "main", "{t:?}");
            assert_eq!(a.main_focus.item, row);
            assert_eq!(a.popup, None);
        }
        assert!(a.take_sound().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_windows_height_at_the_first_draw_picks_the_movie() {
        // Width and height on opposite sides of 720: only the height
        // counts (`Plan::file`).
        let dir = scratch("intro-low");
        for ((w, h), file) in [
            ((1280, 700), "intro_low_60.wmv"),
            ((640, 720), "intro_60.wmv"),
        ] {
            let source = intro::FakeSource::second();
            let (mut a, lines) = intro_app_with(&dir, source, Some("intro_low_60.wmv"));
            a.tick(Instant::now());
            a.draw(&mut Canvas::new(w, h), &mut Text::none());
            assert_eq!(
                lines.lock().unwrap()[0],
                format!("intro: playing {file} ({w}x{h} window)")
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_intros_new_pictures_are_timed() {
        // The decoder says nothing for a while (Media Foundation reads
        // the whole soundtrack first): those black frames cost next to
        // nothing and aren't timed, nor is a picture drawn again, so the
        // log's `intro` time is a new picture's.
        let dir = scratch("intro-timer");
        let (go, wait) = std::sync::mpsc::channel();
        let source = intro::FakeSource {
            wait: Some(wait),
            ..intro::FakeSource::second()
        };
        let (mut a, lines) = intro_app(&dir, source);
        let mut text = Text::none();
        let mut c = Canvas::new(64, 36);
        let t0 = Instant::now();
        a.tick(t0);
        let timed = |a: &App| {
            let f = a.menu_times.frames.iter().find(|f| f.name == "intro");
            f.map_or(0, |f| f.frames)
        };
        let logged = |lines: &Mutex<Vec<String>>| {
            let lines = lines.lock().unwrap();
            lines
                .iter()
                .filter(|l| l.starts_with("lobby: 64x36, intro "))
                .count()
        };
        for _ in 0..2 * MENU_FRAMES {
            a.draw(&mut c, &mut text);
            a.tick(t0);
        }
        assert!(intro_red(&a).is_none());
        assert_eq!(timed(&a), 0);
        go.send(()).unwrap();
        // Each picture in turn (red is its number, at k/30 s on the
        // movie's clock, which starts at t0 with the decoder's first
        // word), drawn twice: only the first draw counts.
        for k in 0..MENU_FRAMES {
            let after = if k == 0 { 0.0 } else { 0.001 };
            let t = t0 + Duration::from_secs_f64(f64::from(k) / 30.0 + after);
            tick_until(&mut a, t, |a| intro_red(a).map(u32::from) == Some(k));
            a.draw(&mut c, &mut text);
            a.draw(&mut c, &mut text);
            if k + 1 < MENU_FRAMES {
                assert_eq!(timed(&a), k + 1);
                assert_eq!(logged(&lines), 0);
            }
        }
        assert_eq!(logged(&lines), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stalled_or_failing_intro_goes_on_to_the_start_screen() {
        let dir = scratch("intro-stall");
        // Two pictures, then the decoder says nothing (it holds on).
        let (keep, hold) = std::sync::mpsc::channel();
        let source = intro::FakeSource {
            pictures: 2,
            hold: Some(hold),
            ..intro::FakeSource::second()
        };
        let (mut a, lines) = intro_app(&dir, source);
        let t0 = Instant::now();
        a.tick(t0);
        a.draw(&mut Canvas::new(64, 36), &mut Text::none());
        // Its clock starts at its first word (t0).
        tick_until(&mut a, t0, |a| intro_red(a) == Some(0));
        let t1 = t0 + Duration::from_millis(50);
        tick_until(&mut a, t1, |a| intro_red(a) == Some(1));
        a.tick(t1 + intro::STALL - Duration::from_millis(100));
        assert_eq!(a.label(), "intro");
        a.tick(t1 + intro::STALL + Duration::from_millis(100));
        assert_eq!(a.label(), "start");
        assert_eq!(
            lines.lock().unwrap().last().unwrap(),
            "intro: the decoder sent nothing for 5 s; skipped"
        );
        assert_eq!(a.take_sound().last(), Some(&SoundOut::StopMovie));
        drop(keep);
        // One that can't be played says why.
        let source = intro::FakeSource {
            pictures: 0,
            sound_seconds: 0.0,
            fail: Some("Media Foundation: no decoder".into()),
            ..intro::FakeSource::second()
        };
        let (mut a, lines) = intro_app(&dir, source);
        let t0 = Instant::now();
        a.tick(t0);
        a.draw(&mut Canvas::new(64, 36), &mut Text::none());
        tick_until(&mut a, t0, |a| a.screen == Screen::Start);
        assert_eq!(
            lines.lock().unwrap().last().unwrap(),
            "intro: Media Foundation: no decoder; skipped"
        );
        assert!(a.music_on());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A lobby whose log is kept, for the tests to read.
    fn logged_app(dir: &std::path::Path) -> (App, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let kept = lines.clone();
        let a = App::new(Config {
            log: Arc::new(move |l: &str| kept.lock().unwrap().push(l.to_string())),
            ..app(dir).cfg
        });
        (a, lines)
    }

    /// ALPHA (this PC, relay id 0x11, hosting if `host`) and BRAVO in a
    /// match of `playlist`, on the in-game screen with the engine's game
    /// on. There is no engine process: the tests call `on_engine_event`
    /// and `on_engine_exit` as `poll_game` would.
    fn in_game(a: &mut App, playlist: u8, ranked: bool, host: bool) {
        let player = |relay_id: u64, account: u64, tag: &str| h2net::live::LauncherPlayer {
            relay_id,
            account,
            gamertag: tag.into(),
            team: 0,
            level: 4,
            party: account,
        };
        let m = LauncherMatch {
            id: 0x78,
            playlist,
            ranked,
            teams: false,
            map: "lockout".into(),
            variant: "01_slayer".into(),
            relay: h2net::live::RelaySeat {
                port: 47050,
                room: 0x78,
                id: 0x11,
                key: [0; 16],
            },
            host: if host { 0x11 } else { 0x22 },
            countdown: 20,
            players: vec![player(0x11, 1, "ALPHA"), player(0x22, 2, "BRAVO")],
        };
        let ip = "127.0.0.1".parse().unwrap();
        let (session, me) = live::session_from(&m, ip).unwrap();
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
            results: None,
            ended_at: None,
            over: None,
            left: None,
            menu: true,
            engine_controls: None,
        });
        a.screen = Screen::InGame;
        a.focus = false;
    }

    fn last_toast(a: &App) -> Option<String> {
        a.toasts.back().map(|(t, _)| t.clone())
    }

    #[test]
    fn controls_changed_in_the_game_menu_are_saved_for_the_next_match() {
        let dir = scratch("menu-controls");
        let (mut a, lines) = logged_app(&dir);
        let changed = Controls {
            buttons: controls::ButtonLayout::BumperJumper,
            look_sensitivity: 8,
            vibration: false,
            mouse_sensitivity: 2.5,
            ..Controls::default()
        };
        // With a match on or not, they are taken and saved.
        a.on_engine_event(&Engine::controls(changed, false), a.now);
        assert_eq!(a.settings.controls, changed);
        let saved = Settings::load(&dir.join("lobby.txt"));
        assert_eq!(saved.controls, changed);
        in_game(&mut a, 12, false, false);
        let back = Controls::default();
        a.on_engine_event(&Engine::controls(back, false), a.now);
        assert_eq!(a.settings.controls, back);
        assert_eq!(Settings::load(&dir.join("lobby.txt")).controls, back);
        // The next engine is started with them.
        a.on_engine_event(&Engine::controls(changed, false), a.now);
        let g = a.game.as_ref().unwrap();
        let m = Match {
            kind: Kind::Fake,
            map: g.m.map.clone(),
            variant: g.m.variant.clone(),
            session: g.session.clone(),
            me: g.me,
            gamertag: "ALPHA".into(),
            instance: None,
            controls: a.settings.controls,
            game_kind: child::game_kind(&g.m),
        };
        let parsed = crate::cli::parse(m.args(std::path::Path::new("s.txt"))).unwrap();
        assert_eq!(parsed.controls.over(Controls::default()), changed);
        // A pair the lobby won't take is logged, and the others still are.
        let mut pairs = Controls::default().pairs();
        pairs[0].1 = "claw".into();
        pairs[1].0 = "rumble".into();
        a.on_engine_event(
            &Engine::Controls {
                pairs,
                defaults: false,
            },
            a.now,
        );
        assert_eq!(
            a.settings.controls,
            Controls {
                buttons: changed.buttons,
                ..Controls::default()
            }
        );
        let log = lines.lock().unwrap().clone();
        assert!(
            log.iter()
                .any(|l| l.starts_with("lobby: the game menu's button_layout = claw wasn't taken")),
            "{log:?}"
        );
        assert!(
            log.iter()
                .any(|l| l.starts_with("lobby: the game menu's rumble = ")),
            "{log:?}"
        );
        assert!(a.game.as_ref().unwrap().left.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leaving_from_the_game_menu_lands_on_the_playlists_in_front() {
        let dir = scratch("menu-left");
        let (mut a, mut server) = live_app(&dir);
        in_game(&mut a, 12, false, false);
        a.on_engine_event(&Engine::Left(Leave::Game), a.now);
        assert_eq!(a.game.as_ref().unwrap().left, Some(Leave::Game));
        // Nothing new for the server until the engine closes.
        assert!(sent(&mut server).is_empty());
        assert_eq!(a.label(), "ingame");
        a.on_engine_event(&Engine::Closing, a.now);
        assert_eq!(
            sent(&mut server),
            [ToServer::LeftMatch {
                id: 0x78,
                host_lost: false
            }]
        );
        a.on_engine_exit();
        assert_eq!(a.label(), "live");
        assert!(a.game.is_none());
        assert!(a.take_focus() && !a.take_focus());
        assert_eq!(last_toast(&a).as_deref(), Some("You left the game."));
        // Said once.
        assert!(sent(&mut server).is_empty());
        // A custom game's leader who leaves lands on the custom game
        // screen, as after a carnage report; the server's reason, when it
        // came, is the notice.
        in_game(&mut a, CUSTOM_GAME, false, false);
        a.on_engine_event(&Engine::Left(Leave::Game), a.now);
        a.game.as_mut().unwrap().over = Some(MatchOver {
            id: 0x78,
            counted: false,
            reason: "THE HOST LEFT. THE GAME DIDN'T COUNT.".into(),
            card: String::new(),
            levels: Vec::new(),
            players: Vec::new(),
        });
        a.on_engine_exit();
        assert_eq!(a.label(), "custom");
        assert!(a.take_focus());
        assert_eq!(
            last_toast(&a).as_deref(),
            Some("THE HOST LEFT. THE GAME DIDN'T COUNT.")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// PC check 13: the host's engine ended a Team Slayer game when the
    /// guest's team emptied, and the server's verdict (no reason, not
    /// counted) reached the guest's lobby before its engine closed. The
    /// guest left from the GAME MENU, and is told so.
    #[test]
    fn a_leave_wins_over_a_verdict_with_no_reason() {
        let dir = scratch("menu-left-verdict");
        let (mut a, mut server) = live_app(&dir);
        in_game(&mut a, 13, false, false);
        a.on_engine_event(&Engine::Left(Leave::Game), a.now);
        a.game.as_mut().unwrap().over = Some(MatchOver {
            id: 0x78,
            counted: false,
            reason: String::new(),
            card: String::new(),
            levels: Vec::new(),
            players: Vec::new(),
        });
        a.on_engine_exit();
        assert_eq!(a.label(), "live");
        assert!(a.take_focus());
        assert_eq!(last_toast(&a).as_deref(), Some("You left the game."));
        assert!(a.game.is_none());
        let _ = sent(&mut server);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// End Game with no result from the engine (one that closed without
    /// giving it): the host lands on Custom with "You ended the game.".
    #[test]
    fn ending_a_custom_game_from_the_game_menu_lands_on_the_custom_screen() {
        let dir = scratch("menu-end");
        let (mut a, mut server) = live_app(&dir);
        in_game(&mut a, CUSTOM_GAME, false, true);
        a.on_engine_event(&Engine::Left(Leave::End), a.now);
        assert_eq!(a.game.as_ref().unwrap().left, Some(Leave::End));
        assert!(sent(&mut server).is_empty());
        // The server's verdict (the host left) came while the engine
        // closed; the host is told they ended it.
        a.game.as_mut().unwrap().over = Some(MatchOver {
            id: 0x78,
            counted: false,
            reason: "THE HOST LEFT. THE GAME DIDN'T COUNT.".into(),
            card: String::new(),
            levels: Vec::new(),
            players: Vec::new(),
        });
        a.on_engine_exit();
        assert_eq!(a.label(), "custom");
        assert!(a.take_focus());
        assert_eq!(last_toast(&a).as_deref(), Some("You ended the game."));
        assert_eq!(
            sent(&mut server),
            [ToServer::LeftMatch {
                id: 0x78,
                host_lost: false
            }]
        );
        // Not the party's leader: the playlists.
        let mut pt = party(500, Activity::Playing);
        pt.leader = 2;
        view(&mut a).party = Some(pt);
        in_game(&mut a, CUSTOM_GAME, false, true);
        a.on_engine_event(&Engine::Left(Leave::End), a.now);
        a.on_engine_exit();
        assert_eq!(a.label(), "live");
        assert_eq!(last_toast(&a).as_deref(), Some("You ended the game."));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// End Game as PC check 14 saw it, once the server's GO came: `left
    /// end`, then the host's result as its engine quits, holding every
    /// player. It goes to the server as a game cut short (finished false),
    /// then LEFT_MATCH, so h2live takes the host as leaving at the end
    /// rather than HOST_LEFT; the engine's closing says nothing more, and
    /// the host lands on the carnage report like the guests, with no "You
    /// ended the game.".
    #[test]
    fn ending_a_custom_game_with_its_result_shows_the_carnage_report() {
        let dir = scratch("menu-end-result");
        let (mut a, mut server) = live_app(&dir);
        in_game(&mut a, CUSTOM_GAME, false, true);
        a.on_event(LiveEvent::Go(0x78));
        a.on_engine_event(&Engine::Left(Leave::End), a.now);
        assert!(sent(&mut server).is_empty());
        let result = |relay_id: u64, place: u8| LauncherPlayerResult {
            relay_id,
            team: place,
            place,
            score: 5 - i32::from(place),
            kills: 5 - u16::from(place),
            assists: 0,
            deaths: 1,
            betrayals: 0,
            suicides: 0,
            left: false,
        };
        let both = vec![result(0x11, 0), result(0x22, 1)];
        a.on_engine_event(&Engine::Ended(both.clone()), a.now);
        let out = sent(&mut server);
        let [ToServer::LauncherResult(r), ToServer::LeftMatch { id, host_lost }] = out.as_slice()
        else {
            panic!("{out:?}");
        };
        assert_eq!((r.id, r.finished), (0x78, false));
        assert_eq!(r.players, both);
        assert_eq!((*id, *host_lost), (0x78, false));
        a.on_engine_event(&Engine::Closing, a.now);
        a.on_engine_exit();
        assert!(sent(&mut server).is_empty());
        assert_eq!(
            (a.label().as_str(), a.take_focus()),
            ("carnage-waiting", true)
        );
        assert_eq!(a.csel, 0);
        assert_ne!(last_toast(&a).as_deref(), Some("You ended the game."));
        // The server's verdict: no reason, not counted (an unranked
        // custom game cut short).
        a.on_match_over(MatchOver {
            id: 0x78,
            counted: false,
            reason: String::new(),
            card: String::new(),
            levels: Vec::new(),
            players: Vec::new(),
        });
        assert_eq!(a.label(), "carnage");
        // A goes on to the custom game screen, as after any custom game.
        a.input(Input::A);
        assert_eq!(a.label(), "custom");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// End Game before the server's GO (a guest still joining): h2live
    /// hasn't started the match and would drop the result, so End Game's
    /// result isn't kept or sent; the host only leaves (LEFT_MATCH) and,
    /// when the engine closes, lands on Custom with "You ended the
    /// game.", not on a carnage report saying the host left. Another
    /// match's GO doesn't change that.
    #[test]
    fn ending_a_custom_game_before_go_lands_on_the_custom_screen() {
        let dir = scratch("menu-end-before-go");
        let (mut a, mut server) = live_app(&dir);
        in_game(&mut a, CUSTOM_GAME, false, true);
        a.on_event(LiveEvent::Go(0x79));
        a.on_engine_event(&Engine::Left(Leave::End), a.now);
        assert!(sent(&mut server).is_empty());
        let result = |relay_id: u64, place: u8| LauncherPlayerResult {
            relay_id,
            team: place,
            place,
            score: 0,
            kills: 0,
            assists: 0,
            deaths: 0,
            betrayals: 0,
            suicides: 0,
            left: false,
        };
        a.on_engine_event(
            &Engine::Ended(vec![result(0x11, 0), result(0x22, 1)]),
            a.now,
        );
        assert_eq!(
            sent(&mut server),
            [ToServer::LeftMatch {
                id: 0x78,
                host_lost: false
            }]
        );
        let g = a.game.as_ref().unwrap();
        assert!(g.results.is_none() && g.ended_at.is_none());
        // The in-game screen still lets B ask to leave.
        a.input(Input::B);
        assert_eq!(a.popup, Some(Popup::LeaveGame));
        a.on_engine_event(&Engine::Closing, a.now);
        a.on_engine_exit();
        assert!(sent(&mut server).is_empty());
        assert_eq!((a.label().as_str(), a.take_focus()), ("custom", true));
        assert_eq!(a.popup, None);
        assert_eq!(last_toast(&a).as_deref(), Some("You ended the game."));
        assert!(a.game.is_none());
        // The server's verdict (the host left), after the engine closed,
        // changes nothing.
        a.on_match_over(MatchOver {
            id: 0x78,
            counted: false,
            reason: "THE HOST LEFT. THE GAME DIDN'T COUNT.".into(),
            card: String::new(),
            levels: Vec::new(),
            players: Vec::new(),
        });
        assert_eq!(a.label(), "custom");
        assert_eq!(last_toast(&a).as_deref(), Some("You ended the game."));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_engine_that_closes_takes_the_lobby_to_the_front() {
        let dir = scratch("menu-exit");
        let mut a = app(&dir);
        // No results and no leave from the menu: today's notices.
        in_game(&mut a, 11, true, false);
        a.on_engine_exit();
        assert_eq!((a.label().as_str(), a.take_focus()), ("live", true));
        assert_eq!(last_toast(&a).as_deref(), Some("You left the game."));
        in_game(&mut a, 11, true, false);
        a.game.as_mut().unwrap().over = Some(MatchOver {
            id: 0x78,
            counted: false,
            reason: String::new(),
            card: String::new(),
            levels: Vec::new(),
            players: Vec::new(),
        });
        a.on_engine_exit();
        assert_eq!((a.label().as_str(), a.take_focus()), ("live", true));
        assert_eq!(last_toast(&a).as_deref(), Some("The match was called off."));
        // Results: the carnage report, on this PC's player, as before,
        // and A goes on to the playlists.
        in_game(&mut a, 11, true, false);
        let result = |relay_id: u64, place: u8| LauncherPlayerResult {
            relay_id,
            team: place,
            place,
            score: 2 - i32::from(place),
            kills: 1,
            assists: 0,
            deaths: 1,
            betrayals: 0,
            suicides: 0,
            left: false,
        };
        a.on_engine_event(
            &Engine::Ended(vec![result(0x11, 1), result(0x22, 0)]),
            a.now,
        );
        a.on_engine_exit();
        // (Waiting for the server's verdict.)
        assert_eq!(
            (a.label().as_str(), a.take_focus()),
            ("carnage-waiting", true)
        );
        assert_eq!(a.csel, 1);
        assert!(a.game.is_some());
        a.input(Input::A);
        assert_eq!(a.label(), "live");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_one_run_control_flag_is_not_saved_with_the_game_menus_changes() {
        // lobby.txt has vibration on; a test started the engine with
        // H2LOBBY_ENGINE_ARGS=--no-vibration, so it runs with it off.
        let dir = scratch("menu-controls-flag");
        let mut a = app(&dir);
        in_game(&mut a, 12, false, false);
        let saved = a.settings.controls;
        assert!(saved.vibration);
        let launch = child::launch_controls("--no-vibration", saved);
        a.game.as_mut().unwrap().engine_controls = Some(launch);
        // Only Mouse Sensitivity is changed in the game menu; the engine
        // says all eight, vibration off among them.
        let live = Controls {
            mouse_sensitivity: 4.0,
            ..launch
        };
        a.on_engine_event(&Engine::controls(live, false), a.now);
        let want = Controls {
            mouse_sensitivity: 4.0,
            ..saved
        };
        assert_eq!(a.settings.controls, want);
        assert_eq!(Settings::load(&dir.join("lobby.txt")).controls, want);
        // Then vibration itself: that is the player's, and saved.
        let on = Controls {
            vibration: true,
            ..live
        };
        let off = Controls {
            vibration: false,
            ..live
        };
        a.on_engine_event(&Engine::controls(on, false), a.now);
        a.on_engine_event(&Engine::controls(off, false), a.now);
        assert!(!a.settings.controls.vibration);
        assert_eq!(a.settings.controls.mouse_sensitivity, 4.0);
        assert_eq!(
            Settings::load(&dir.join("lobby.txt")).controls,
            a.settings.controls
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn use_default_settings_in_the_game_menu_saves_every_one() {
        // lobby.txt has look sensitivity 7 and vibration off; a test
        // started the engine with H2LOBBY_ENGINE_ARGS=--look-sensitivity 3
        // --vibration, so it ran on Halo 2's defaults already.
        let dir = scratch("menu-controls-defaults");
        let mut a = app(&dir);
        in_game(&mut a, 12, false, false);
        for (k, v) in [("look_sensitivity", "7"), ("vibration", "off")] {
            a.set_control(k, v).unwrap();
        }
        let saved = a.settings.controls;
        let launch = child::launch_controls("--look-sensitivity 3 --vibration", saved);
        assert_eq!(launch, Controls::default());
        a.game.as_mut().unwrap().engine_controls = Some(launch);
        // The same eight as an ordinary change: nothing differs from what
        // the engine ran with, so lobby.txt keeps the player's.
        a.on_engine_event(&Engine::controls(Controls::default(), false), a.now);
        assert_eq!(a.settings.controls, saved);
        assert_eq!(Settings::load(&dir.join("lobby.txt")).controls, saved);
        // Said as Use Default Settings: every one is saved.
        a.on_engine_event(&Engine::controls(Controls::default(), true), a.now);
        assert_eq!(a.settings.controls, Controls::default());
        assert_eq!(
            Settings::load(&dir.join("lobby.txt")).controls,
            Controls::default()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The match `in_game` sets up, before its engine is started.
    fn about_to_start(a: &mut App, playlist: u8, host: bool) {
        in_game(a, playlist, false, host);
        let g = a.game.as_mut().unwrap();
        (g.launch, g.running, g.loaded, g.start_at) = (true, false, false, a.now);
    }

    /// A stand-in for the engine's launcher that takes any words and stays
    /// up (until it is killed). A shell of its own writes it, so no handle
    /// of this process to the file is inherited by another test's child,
    /// which would make starting it fail as "text file busy".
    #[cfg(unix)]
    fn sleeper(dir: &std::path::Path) -> PathBuf {
        let exe = dir.join("engine.sh");
        let made = std::process::Command::new("sh")
            .args([
                "-c",
                "printf '#!/bin/sh\\nexec sleep 30\\n' > \"$1\" && chmod +x \"$1\"",
            ])
            .arg("sh")
            .arg(&exe)
            .status()
            .unwrap();
        assert!(made.success());
        exe
    }

    #[cfg(unix)]
    #[test]
    fn a_started_engine_keeps_its_one_run_flags_apart_from_the_players() {
        // H2LOBBY_ENGINE_ARGS (read once into the lobby's Config) turns
        // vibration off for this run and the game menu off.
        let dir = scratch("engine-start");
        let mut a = app(&dir);
        a.cfg.exe = sleeper(&dir);
        a.cfg.engine_args = "--no-vibration --no-game-menu".into();
        let saved = a.settings.controls;
        assert!(saved.vibration);
        about_to_start(&mut a, 12, false);
        a.poll_game();
        let g = a.game.as_mut().unwrap();
        assert!(g.child.is_some(), "the stand-in started");
        assert_eq!(a.screen, Screen::InGame);
        // The in-game screen won't say Start opens the game menu, and the
        // GAME MENU's changes are told apart from the flag's.
        assert!(!g.menu);
        let launch = Controls {
            vibration: false,
            ..saved
        };
        assert_eq!(g.engine_controls, Some(launch));
        a.on_engine_event(&Engine::controls(launch, false), a.now);
        assert_eq!(Settings::load(&dir.join("lobby.txt")).controls, saved);
        assert!(a.settings.controls.vibration);
        // Without the words, the engine has its menu and the lobby's
        // controls.
        a.game.as_mut().unwrap().child.as_mut().unwrap().kill();
        a.cfg.engine_args.clear();
        about_to_start(&mut a, 12, false);
        a.poll_game();
        let g = a.game.as_mut().unwrap();
        assert!(g.menu);
        assert_eq!(g.engine_controls, Some(saved));
        g.child.as_mut().unwrap().kill();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_engine_that_does_not_start_lands_as_after_a_game() {
        let dir = scratch("engine-no-start");
        let (mut a, mut server) = live_app(&dir);
        a.cfg.exe = dir.join("no-such-engine");
        // A custom game's leader: back on the custom game screen.
        about_to_start(&mut a, CUSTOM_GAME, true);
        a.poll_game();
        assert_eq!(a.label(), "custom");
        assert!(a.game.is_none());
        let toast = last_toast(&a).unwrap();
        assert!(toast.starts_with("The engine didn't start: "), "{toast}");
        assert_eq!(
            sent(&mut server),
            [ToServer::LeftMatch {
                id: 0x78,
                host_lost: false
            }]
        );
        // A playlist's game: the playlists.
        about_to_start(&mut a, 12, false);
        a.poll_game();
        assert_eq!(a.label(), "live");
        assert!(a.game.is_none());
        assert_eq!(
            sent(&mut server),
            [ToServer::LeftMatch {
                id: 0x78,
                host_lost: false
            }]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_in_game_screen_says_how_to_open_the_game_menu() {
        let Ok(mut text) = Text::system() else {
            return;
        };
        let dir = scratch("menu-hint");
        let mut a = app(&dir);
        in_game(&mut a, 12, false, false);
        let mut c = Canvas::new(640, 360);
        a.draw(&mut c, &mut text);
        assert!(a.drawn().iter().any(|d| d == IN_GAME_MENU_HINT));
        // Not once the game is over, nor before the engine runs.
        a.game.as_mut().unwrap().results = Some(Vec::new());
        a.draw(&mut c, &mut text);
        assert!(a.drawn().iter().all(|d| d != IN_GAME_MENU_HINT));
        in_game(&mut a, 12, false, false);
        a.game.as_mut().unwrap().running = false;
        a.draw(&mut c, &mut text);
        assert!(a.drawn().iter().all(|d| d != IN_GAME_MENU_HINT));
        // Nor when the engine runs without its menu (the README's
        // fallback): Start goes to the game then.
        in_game(&mut a, 12, false, false);
        a.game.as_mut().unwrap().menu = false;
        a.draw(&mut c, &mut text);
        assert!(a.drawn().iter().all(|d| d != IN_GAME_MENU_HINT));
        // Nor once the engine turned its menu off mid-game (its overlay
        // failed): the line goes.
        in_game(&mut a, 12, false, false);
        a.draw(&mut c, &mut text);
        assert!(a.drawn().iter().any(|d| d == IN_GAME_MENU_HINT));
        a.on_engine_event(&Engine::MenuOff, a.now);
        assert!(!a.game.as_ref().unwrap().menu);
        a.draw(&mut c, &mut text);
        assert!(a.drawn().iter().all(|d| d != IN_GAME_MENU_HINT));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leaving_from_the_game_menu_clears_the_lobbys_leave_question() {
        // The lobby's "Leave game" asked, unanswered; the player leaves
        // from the engine's GAME MENU instead.
        let dir = scratch("menu-popup");
        let mut a = app(&dir);
        in_game(&mut a, 11, true, false);
        a.input(Input::B);
        assert_eq!(a.popup, Some(Popup::LeaveGame));
        a.on_engine_event(&Engine::Left(Leave::Game), a.now);
        a.on_engine_exit();
        assert_eq!(a.popup, None);
        assert_eq!(a.label(), "live");
        assert_eq!(last_toast(&a).as_deref(), Some("You left the game."));
        // The same when the game ended and the carnage report comes.
        in_game(&mut a, 11, true, false);
        a.input(Input::B);
        assert_eq!(a.popup, Some(Popup::LeaveGame));
        a.on_engine_event(&Engine::Ended(Vec::new()), a.now);
        a.on_engine_exit();
        assert_eq!(a.popup, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
