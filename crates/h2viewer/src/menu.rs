//! Halo 2 style menus, drawn over the map: the main menu, the campaign's
//! missions, the multiplayer lobby, the system link browser, the player
//! profile, the online screens (the party lobby, who's online, the
//! matchmaking playlists and searching them), the pause menu and the
//! post-game carnage report. Keyboard, mouse and controllers all work them.

use crate::camera::Controls;
use crate::gpu::hud_mode;
use crate::hud::HudBuilder;
use crate::online::{clock_text, OnlineView};
use crate::options::{presets, GameOptions, RESPAWN_TIMES, TIME_LIMITS};
use crate::profile::{color_name, Profile};
use crate::rank::{self, LiveIcon};
use h2net::live::{self, Privacy};
use h2net::{LanGame, Lobby};
use h2sim::game::{
    clean_name, name_char, Emblem, Look, EMBLEM_BACKGROUNDS, EMBLEM_FOREGROUNDS, MAX_NAME,
    PROFILE_COLORS,
};
use h2sim::GameType;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Main,
    Campaign,
    Lobby,
    /// The lobby's game options.
    Options,
    SystemLink,
    Profile,
    /// The profile's look settings (Halo 2 keeps look sensitivity and
    /// invert look among its profile's controller settings).
    Controls,
    Pause,
    PostGame,
    /// Online: the party lobby (and signing in, until signed in).
    Live,
    /// Everyone else online.
    Players,
    /// Who we played with lately.
    RecentPlayers,
    /// What to do about one of them, or of those online (`Menu::player`).
    Player,
    /// The matchmaking playlists.
    Playlists,
    /// The party searching a playlist.
    Matchmaking,
    /// A match's lobby before its game, while everyone's map loads.
    Pregame,
}

/// A multiplayer map in the maps folder.
#[derive(Clone, Debug, PartialEq)]
pub struct MapChoice {
    pub path: PathBuf,
    /// The file's name without `.map`, lowercase (how LAN games name maps).
    pub name: String,
    /// The name players know it by.
    pub title: String,
    /// Halo 2's description of it, when mainmenu.map has one.
    pub description: String,
    /// Its picture in the pictures `mapinfo::describe_maps` returned.
    pub picture: Option<usize>,
}

/// Halo 2's names for its multiplayer maps' files.
const MAP_TITLES: &[(&str, &str)] = &[
    ("ascension", "ASCENSION"),
    ("backwash", "BACKWASH"),
    ("beavercreek", "BEAVER CREEK"),
    ("burial_mounds", "BURIAL MOUNDS"),
    ("coagulation", "COAGULATION"),
    ("colossus", "COLOSSUS"),
    ("containment", "CONTAINMENT"),
    ("cyclotron", "IVORY TOWER"),
    ("deltatap", "SANCTUARY"),
    ("derelict", "DESOLATION"),
    ("dune", "RELIC"),
    ("elongation", "ELONGATION"),
    ("foundation", "FOUNDATION"),
    ("gemini", "GEMINI"),
    ("headlong", "HEADLONG"),
    ("highplains", "TOMBSTONE"),
    ("lockout", "LOCKOUT"),
    ("midship", "MIDSHIP"),
    ("needle", "UPLIFT"),
    ("street_sweeper", "DISTRICT"),
    ("triplicate", "TERMINAL"),
    ("turf", "TURF"),
    ("warlock", "WARLOCK"),
    ("waterworks", "WATERWORKS"),
    ("zanzibar", "ZANZIBAR"),
];

/// Halo 2's campaign missions' files and names, in story order.
const MISSION_TITLES: &[(&str, &str)] = &[
    ("00a_introduction", "THE HERETIC"),
    ("01a_tutorial", "THE ARMORY"),
    ("01b_spacestation", "CAIRO STATION"),
    ("03a_oldmombasa", "OUTSKIRTS"),
    ("03b_newmombasa", "METROPOLIS"),
    ("04a_gasgiant", "THE ARBITER"),
    ("04b_floodlab", "THE ORACLE"),
    ("05a_deltaapproach", "DELTA HALO"),
    ("05b_deltatowers", "REGRET"),
    ("06a_sentinelwalls", "SACRED ICON"),
    ("06b_floodzone", "QUARANTINE ZONE"),
    ("07a_highcharity", "GRAVEMIND"),
    ("07b_forerunnership", "HIGH CHARITY"),
    ("08a_deltacliffs", "UPRISING"),
    ("08b_deltacontrol", "THE GREAT JOURNEY"),
];

/// The campaign's difficulties.
pub const DIFFICULTIES: [&str; 4] = ["EASY", "NORMAL", "HEROIC", "LEGENDARY"];

pub fn map_title(name: &str) -> String {
    let name = name.to_lowercase();
    MAP_TITLES
        .iter()
        .chain(MISSION_TITLES)
        .find(|m| m.0 == name)
        .map_or_else(|| name.replace('_', " ").to_uppercase(), |m| m.1.into())
}

impl MapChoice {
    pub fn new(path: &Path) -> MapChoice {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        MapChoice {
            path: path.to_path_buf(),
            title: map_title(&name),
            name,
            description: String::new(),
            picture: None,
        }
    }
}

/// The multiplayer maps in a folder and in the `dlc` folder beside it
/// (add-on maps), by name.
pub fn find_maps(dir: &Path) -> Vec<MapChoice> {
    let dlc = dir.parent().map(|p| p.join("dlc"));
    let mut maps = maps_in(dir, blam_cache::MapType::Multiplayer);
    if let Some(dlc) = dlc.filter(|d| d.as_path() != dir) {
        maps.extend(maps_in(&dlc, blam_cache::MapType::Multiplayer));
    }
    maps.sort_by(|a, b| a.title.cmp(&b.title));
    maps
}

/// The campaign missions in a folder, in story order.
pub fn find_missions(dir: &Path) -> Vec<MapChoice> {
    let mut missions = maps_in(dir, blam_cache::MapType::Campaign);
    let order = |m: &MapChoice| {
        MISSION_TITLES
            .iter()
            .position(|t| t.0 == m.name)
            .unwrap_or(MISSION_TITLES.len())
    };
    missions.sort_by(|a, b| order(a).cmp(&order(b)).then(a.name.cmp(&b.name)));
    missions
}

/// Whether a map file is a campaign mission.
pub fn is_mission(path: &Path) -> bool {
    map_type(path) == Some(blam_cache::MapType::Campaign)
}

fn map_type(path: &Path) -> Option<blam_cache::MapType> {
    use std::io::Read;
    let mut head = vec![0u8; 0x800];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .ok()?;
    blam_cache::Header::parse(&head).ok().map(|h| h.map_type)
}

fn maps_in(dir: &Path, kind: blam_cache::MapType) -> Vec<MapChoice> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("map")))
        .filter(|p| map_type(p) == Some(kind))
        .map(|p| MapChoice::new(&p))
        .collect()
}

/// Kills to win the lobby offers; 0 plays on without end.
pub const SCORES: [u32; 6] = [5, 10, 15, 25, 50, 0];
/// Captures (or bombs) to win Capture the Flag and Assault.
pub const CAPTURES: [u32; 5] = [1, 3, 5, 10, 0];
/// Seconds to win the timed game types.
pub const TIMES: [u32; 6] = [60, 120, 180, 300, 600, 0];
/// Points to win Juggernaut.
pub const JUGGERNAUT_POINTS: [u32; 5] = [5, 10, 15, 25, 0];
pub const MAX_BOTS: usize = 15;
/// The game types the lobby offers, with their names (and the names
/// H2_GAME takes).
pub const GAME_TYPES: [(GameType, &str, &str); 10] = [
    (GameType::Slayer, "SLAYER", "slayer"),
    (GameType::TeamSlayer, "TEAM SLAYER", "team"),
    (GameType::Ctf, "CAPTURE THE FLAG", "ctf"),
    (GameType::KingOfTheHill, "KING OF THE HILL", "king"),
    (GameType::TeamKing, "TEAM KING", "teamking"),
    (GameType::Oddball, "ODDBALL", "oddball"),
    (GameType::TeamOddball, "TEAM ODDBALL", "teamoddball"),
    (GameType::Juggernaut, "JUGGERNAUT", "juggernaut"),
    (GameType::Territories, "TERRITORIES", "territories"),
    (GameType::Assault, "ASSAULT", "assault"),
];

/// The scores to win a game type offers, and the one it starts on.
pub fn scores(game_type: GameType) -> (&'static [u32], usize) {
    match game_type {
        GameType::Ctf | GameType::Assault => (&CAPTURES, 1),
        GameType::Territories => (&TIMES, 3),
        t if t.timed() => (&TIMES, 1),
        GameType::Juggernaut => (&JUGGERNAUT_POINTS, 1),
        _ => (&SCORES, 3),
    }
}

#[derive(Clone, Debug, Default)]
pub struct Settings {
    /// In `GAME_TYPES`.
    pub game_type: usize,
    /// In `Context::maps`.
    pub map: usize,
    /// In the game type's `scores`.
    pub score: usize,
    pub bots: usize,
    pub options: GameOptions,
}

impl Settings {
    pub fn game_type(&self) -> GameType {
        GAME_TYPES[self.game_type.min(GAME_TYPES.len() - 1)].0
    }

    pub fn score_to_win(&self) -> u32 {
        let (scores, default) = scores(self.game_type());
        scores.get(self.score).copied().unwrap_or(scores[default])
    }
}

/// A line of the scoreboard: a player, or a team's totals heading its
/// players.
#[derive(Clone, Debug)]
pub struct ScoreLine {
    pub name: String,
    pub score: i32,
    /// The score is seconds (shown as minutes and seconds).
    pub timed: bool,
    pub kills: u32,
    pub deaths: u32,
    pub color: [f32; 3],
    /// A player's emblem (team lines have none).
    pub emblem: Option<Emblem>,
    /// A player's level, 1 to 50, when known (online).
    pub level: Option<u8>,
    /// Someone playing at this PC.
    pub local: bool,
    pub header: bool,
}

/// Someone in the lobby.
#[derive(Clone, Debug)]
pub struct SeatInfo {
    /// Their gamertag.
    pub name: String,
    /// "KEYBOARD", "CONTROLLER", "SYSTEM LINK"...
    pub how: &'static str,
    /// Their team, or `NO_TEAM` until the game puts them on one.
    pub team: u8,
    pub look: Look,
    /// Their level, 1 to 50, when known (online).
    pub level: Option<u8>,
}

/// A player in the lobby whose team the game will choose.
pub const NO_TEAM: u8 = h2net::ANY_TEAM;
/// How the lobby says someone here plays with the keyboard and mouse.
pub const KEYBOARD: &str = "KEYBOARD";

/// What the game should do after a menu input.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    /// Start a game with the lobby's settings.
    Start,
    /// Play a campaign mission (one of `Context::missions`).
    Mission(usize),
    Join(LanGame),
    /// The player changed their profile: keep it.
    SaveProfile,
    Resume,
    /// Back to the lobby (ending the game).
    EndGame,
    /// Leave the game or lobby joined on another PC.
    Leave,
    Quit,
    /// Sign in to the online service.
    GoOnline,
    SignOut,
    /// Party leader: search a playlist (none for Quickmatch), or stop.
    Search(Option<u8>),
    CancelSearch,
    /// Party leader: open a custom game for the party.
    Custom,
    /// Ask a player (by account) into our party.
    Invite(u64),
    /// Join an open party, or one that invited us.
    JoinParty(u64),
    Accept(u64),
    LeaveParty,
    /// Party leader: remove a member, or make them leader.
    Kick(u64),
    Promote(u64),
    Privacy(Privacy),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Up,
    Down,
    Left,
    Right,
    Select,
    Back,
}

/// Typing in a text box (the gamertag).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Typed {
    Text(String),
    Erase,
    Done,
}

/// The menu's sounds, for the game to play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Cursor,
    Forward,
    Back,
    Advance,
}

/// What the game knows that the menus show.
pub struct Context<'a> {
    pub maps: &'a [MapChoice],
    pub missions: &'a [MapChoice],
    pub lan: &'a [LanGame],
    /// Everyone in the lobby.
    pub seats: &'a [SeatInfo],
    /// How many people play at this PC.
    pub local: usize,
    pub scores: &'a [ScoreLine],
    /// How the game ended (who won, or a draw), for the carnage report.
    pub outcome: Option<&'a str>,
    /// Playing in another PC's game.
    pub joined: bool,
    /// Joined: the host's lobby, while waiting there for its next game.
    pub host_lobby: Option<&'a Lobby>,
    /// A mission's objectives so far, and whether each is done.
    pub objectives: &'a [(String, bool)],
    /// The online service, when we went online.
    pub online: Option<&'a OnlineView<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Difficulty,
    Mission(usize),
    NoMissions,
    Multiplayer,
    SystemLink,
    Profile,
    Quit,
    Name,
    Model,
    Primary,
    Secondary,
    Emblem,
    EmblemBackground,
    EmblemPrimary,
    EmblemSecondary,
    EmblemBackColor,
    /// On to the profile's look settings.
    Controls,
    LookSensitivity,
    MouseSensitivity,
    InvertLook,
    GameType,
    Map,
    Score,
    Bots,
    GameOptions,
    Variant,
    MapWeapons,
    StartPrimary,
    StartSecondary,
    StartGrenades,
    Shields,
    Radar,
    Vehicles,
    Respawn,
    FriendlyFire,
    TimeLimit,
    StartGame,
    Join(usize),
    Searching,
    /// In another PC's lobby, until its game starts.
    Waiting,
    Resume,
    EndGame,
    Continue,
    Online,
    Connecting,
    SignIn,
    Matchmaking,
    Quickmatch,
    CustomGame,
    OnlinePlayers,
    RecentPlayers,
    /// Who can join the party.
    Privacy,
    /// Join the party that last invited us.
    AcceptInvite,
    LeaveParty,
    /// Someone else online.
    Player(usize),
    /// Someone we played with lately.
    RecentPlayer(usize),
    Invite,
    JoinTheirParty,
    MakeLeader,
    Remove,
    /// Nothing to choose here (what it says depends on the screen).
    Nothing,
    Playlist(usize),
    /// How the search is going, and a line more.
    SearchStatus,
    SearchDetail,
    Cancel,
    /// How soon a match's game starts.
    Starting,
}

/// The main menu. The campaign is left off: multiplayer only.
const MAIN_ROWS: [Row; 5] = [
    Row::Online,
    Row::Multiplayer,
    Row::SystemLink,
    Row::Profile,
    Row::Quit,
];
/// Most rows a list (games on the network, players online, playlists)
/// shows at once; the rest scroll into view.
const MAX_LIST_ROWS: usize = 8;

impl Row {
    fn selectable(self, ctx: &Context) -> bool {
        match self {
            Row::Searching
            | Row::Waiting
            | Row::NoMissions
            | Row::Connecting
            | Row::Nothing
            | Row::SearchStatus
            | Row::SearchDetail
            | Row::Starting => false,
            // The party leader's choices.
            Row::Matchmaking | Row::Quickmatch | Row::Privacy | Row::Cancel => {
                ctx.online.is_some_and(|o| o.leads())
            }
            // Its members can go back into its custom game.
            Row::CustomGame => ctx.online.is_some_and(|o| {
                let custom = o
                    .party()
                    .is_some_and(|p| p.activity == live::Activity::Custom);
                o.leads() || custom
            }),
            _ => true,
        }
    }

    /// A profile setting stepped with left and right.
    fn in_profile(self) -> bool {
        matches!(
            self,
            Row::Model
                | Row::Primary
                | Row::Secondary
                | Row::Emblem
                | Row::EmblemBackground
                | Row::EmblemPrimary
                | Row::EmblemSecondary
                | Row::EmblemBackColor
                | Row::LookSensitivity
                | Row::MouseSensitivity
                | Row::InvertLook
        )
    }
}

pub struct Menu {
    pub screen: Screen,
    cursor: usize,
    /// The first row shown of a list longer than `MAX_LIST_ROWS`.
    scroll: usize,
    pub settings: Settings,
    /// A line to show (why a game ended, a join that failed).
    pub notice: Option<String>,
    pub sound: Option<Sound>,
    pub profile: Profile,
    /// Typing a new gamertag.
    pub editing: bool,
    /// The campaign's difficulty, in `DIFFICULTIES`.
    pub difficulty: usize,
    /// The account picked from the online or recent players, and which
    /// list it was.
    player: u64,
    list: Screen,
}

/// Layout, in Halo 2's 640x480 screen units.
const ROW_X: f32 = 48.0;
const ROW_Y: f32 = 128.0;
const ROW_W: f32 = 300.0;
const ROW_H: f32 = 26.0;
const ROW_STEP: f32 = 32.0;
/// The line of hints at the bottom (rows must end above it).
const HINT_Y: f32 = 440.0;
/// The lobby's right-hand panels (map, players).
const PANEL_X: f32 = 372.0;
const PANEL_W: f32 = 228.0;
/// The profile's emblem: left, top and size.
const PROFILE_EMBLEM: [f32; 3] = [530.0, 104.0, 64.0];
/// The map's picture (Halo 2's are 440 by 414).
const PICTURE: [f32; 2] = [104.0, 98.0];
const TEXT: [f32; 4] = [0.72, 0.84, 1.0, 1.0];
const BRIGHT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const DIM: [f32; 4] = [0.5, 0.62, 0.8, 0.9];
const PANEL: [f32; 4] = [0.02, 0.07, 0.14, 0.72];
const HIGHLIGHT: [f32; 4] = [0.2, 0.45, 0.85, 0.9];
const WARNING: [f32; 4] = [1.0, 0.55, 0.3, 1.0];
/// The lobby's mark for someone in a team game with no team yet.
const UNPICKED: [f32; 3] = [0.5, 0.5, 0.5];

/// The lobby's score to win, as it shows it.
pub fn score_label(settings: &Settings) -> String {
    match settings.score_to_win() {
        0 => "NO LIMIT".into(),
        n => crate::local::score_text(n as i32, settings.game_type().timed()),
    }
}

/// The game options' variant name, or CUSTOM.
pub fn variant_label(options: &GameOptions) -> &'static str {
    options.preset().map_or("CUSTOM", |k| presets()[k].0)
}

/// The game options as the lobby shows them (and sends to PCs that join):
/// the variant, and the time limit if there is one.
pub fn options_label(options: &GameOptions) -> String {
    let variant = variant_label(options);
    match options.time_limit {
        0 => variant.into(),
        t if t % 60 == 0 => format!("{variant}, {} MIN", t / 60),
        t => format!("{variant}, {t} SEC"),
    }
}

fn on_off(on: bool) -> String {
    if on { "ON" } else { "OFF" }.into()
}

/// "1ST", "2ND"...
pub fn place(n: usize) -> String {
    let suffix = match (n % 100, n % 10) {
        (11..=13, _) => "TH",
        (_, 1) => "ST",
        (_, 2) => "ND",
        (_, 3) => "RD",
        _ => "TH",
    };
    format!("{n}{suffix}")
}

/// Turns 640x480 screen units into window pixels, centred.
struct Frame {
    s: f32,
    ox: f32,
    oy: f32,
}

impl Frame {
    fn new(w: f32, h: f32) -> Frame {
        let s = (h / 480.0).min(w / 640.0);
        Frame {
            s,
            ox: (w - 640.0 * s) * 0.5,
            oy: (h - 480.0 * s) * 0.5,
        }
    }

    fn rect(&self, [x0, y0, x1, y1]: [f32; 4]) -> [f32; 4] {
        [
            self.ox + x0 * self.s,
            self.oy + y0 * self.s,
            self.ox + x1 * self.s,
            self.oy + y1 * self.s,
        ]
    }

    fn at(&self, x: f32, y: f32) -> [f32; 2] {
        [self.ox + x * self.s, self.oy + y * self.s]
    }
}

impl Menu {
    pub fn new(settings: Settings, profile: Profile) -> Menu {
        Menu {
            screen: Screen::Main,
            cursor: 0,
            scroll: 0,
            settings,
            notice: None,
            sound: None,
            profile,
            editing: false,
            difficulty: 1,
            player: 0,
            list: Screen::Players,
        }
    }

    pub fn show(&mut self, screen: Screen) {
        self.screen = screen;
        self.cursor = 0;
        self.scroll = 0;
    }

    /// The screen is a list that scrolls when long.
    fn lists(&self) -> bool {
        matches!(
            self.screen,
            Screen::SystemLink | Screen::Players | Screen::RecentPlayers | Screen::Playlists
        )
    }

    /// The rows shown of `n`: on a long list, `MAX_LIST_ROWS` from the
    /// first scrolled to.
    fn shown(&self, n: usize) -> std::ops::Range<usize> {
        if !self.lists() {
            return 0..n;
        }
        let first = self.scroll.min(n.saturating_sub(MAX_LIST_ROWS));
        first..n.min(first + MAX_LIST_ROWS)
    }

    /// Scroll a long list so the cursor's row shows.
    fn follow_cursor(&mut self) {
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + MAX_LIST_ROWS {
            self.scroll = self.cursor + 1 - MAX_LIST_ROWS;
        }
    }

    fn title(&self, ctx: &Context) -> &'static str {
        match self.screen {
            Screen::Main => "HALO 2",
            Screen::Campaign => "CAMPAIGN",
            Screen::Lobby if in_custom(ctx) => "CUSTOM GAME",
            Screen::Lobby => "MULTIPLAYER",
            Screen::Options => "GAME OPTIONS",
            Screen::SystemLink => "SYSTEM LINK",
            Screen::Profile => "PLAYER PROFILE",
            Screen::Controls => "CONTROLS",
            Screen::Pause => "PAUSED",
            Screen::PostGame => "GAME OVER",
            Screen::Live => "ONLINE",
            Screen::Players => "ONLINE PLAYERS",
            Screen::RecentPlayers => "RECENT PLAYERS",
            Screen::Player => "PLAYER",
            Screen::Playlists => "PLAYLISTS",
            Screen::Matchmaking => "MATCHMAKING",
            Screen::Pregame => "PREGAME LOBBY",
        }
    }

    fn rows(&self, ctx: &Context) -> Vec<Row> {
        match self.screen {
            Screen::Main => MAIN_ROWS.to_vec(),
            Screen::Campaign if ctx.missions.is_empty() => vec![Row::Difficulty, Row::NoMissions],
            Screen::Campaign => std::iter::once(Row::Difficulty)
                .chain((0..ctx.missions.len()).map(Row::Mission))
                .collect(),
            Screen::Profile => vec![
                Row::Name,
                Row::Model,
                Row::Primary,
                Row::Secondary,
                Row::Emblem,
                Row::EmblemBackground,
                Row::EmblemPrimary,
                Row::EmblemSecondary,
                Row::EmblemBackColor,
                Row::Controls,
            ],
            Screen::Controls => vec![Row::LookSensitivity, Row::MouseSensitivity, Row::InvertLook],
            Screen::Lobby if ctx.host_lobby.is_some() => vec![
                Row::GameType,
                Row::Map,
                Row::Score,
                Row::Bots,
                Row::GameOptions,
                Row::Waiting,
            ],
            Screen::Lobby => vec![
                Row::GameType,
                Row::Map,
                Row::Score,
                Row::Bots,
                Row::GameOptions,
                Row::StartGame,
            ],
            Screen::Options => vec![
                Row::Variant,
                Row::MapWeapons,
                Row::StartPrimary,
                Row::StartSecondary,
                Row::StartGrenades,
                Row::Shields,
                Row::Radar,
                Row::Vehicles,
                Row::Respawn,
                Row::FriendlyFire,
                Row::TimeLimit,
            ],
            Screen::SystemLink if ctx.lan.is_empty() => vec![Row::Searching],
            Screen::SystemLink => (0..ctx.lan.len()).map(Row::Join).collect(),
            Screen::Pause => vec![Row::Resume, Row::EndGame, Row::Quit],
            Screen::PostGame => vec![Row::Continue],
            Screen::Live => live_rows(ctx),
            Screen::Players => match ctx.online.map_or(0, |o| o.others().len()) {
                0 => vec![Row::Nothing],
                n => (0..n).map(Row::Player).collect(),
            },
            Screen::RecentPlayers => match ctx.online.map_or(0, |o| o.recent.len()) {
                0 => vec![Row::Nothing],
                n => (0..n).map(Row::RecentPlayer).collect(),
            },
            Screen::Player => self.player_rows(ctx),
            Screen::Playlists => match ctx.online.map_or(0, |o| o.playlists().len()) {
                0 => vec![Row::Nothing],
                n => (0..n).map(Row::Playlist).collect(),
            },
            Screen::Matchmaking => {
                let detail = ctx.online.and_then(|o| o.search_status().1);
                let mut rows = vec![Row::SearchStatus];
                rows.extend(detail.map(|_| Row::SearchDetail));
                rows.push(Row::Cancel);
                rows
            }
            Screen::Pregame => vec![
                Row::GameType,
                Row::Map,
                Row::Score,
                Row::GameOptions,
                Row::Starting,
            ],
        }
    }

    /// What can be done about the player picked, if they're online: invite
    /// them, or join their party (if it's open and not in a match); or, in
    /// the party we lead, hand it over to them or remove them.
    fn player_rows(&self, ctx: &Context) -> Vec<Row> {
        let Some((o, p)) = ctx.online.and_then(|o| Some((o, o.player(self.player)?))) else {
            return vec![Row::Nothing];
        };
        let mut rows = Vec::new();
        if !o.with_us(p) {
            rows.push(Row::Invite);
            if p.open && p.activity != live::Activity::Playing {
                rows.push(Row::JoinTheirParty);
            }
        } else if o.leads() {
            rows.extend([Row::MakeLeader, Row::Remove]);
        }
        if rows.is_empty() {
            rows.push(Row::Nothing);
        }
        rows
    }

    fn label(&self, row: Row, ctx: &Context) -> (String, Option<String>) {
        let s = &self.settings;
        if let Some(l) = ctx.host_lobby {
            // The host's choices.
            let value = match row {
                Row::GameType => Some(l.game_type.clone()),
                Row::Map => Some(map_title(&l.map)),
                Row::Score => Some(l.score.clone()),
                Row::Bots => Some(l.bots.to_string()),
                Row::GameOptions => Some(l.options.clone()),
                _ => None,
            };
            if let Some(v) = value {
                return (self.label_name(row, ctx), Some(v));
            }
        }
        match row {
            Row::Difficulty => (
                "DIFFICULTY".into(),
                Some(DIFFICULTIES[self.difficulty.min(3)].into()),
            ),
            Row::Mission(i) => (
                ctx.missions
                    .get(i)
                    .map_or_else(String::new, |m| m.title.clone()),
                None,
            ),
            Row::NoMissions => ("NO CAMPAIGN MAPS FOUND".into(), None),
            Row::Multiplayer => ("MULTIPLAYER".into(), None),
            Row::SystemLink => ("SYSTEM LINK".into(), None),
            Row::Profile => ("PLAYER PROFILE".into(), None),
            Row::Quit => ("QUIT".into(), None),
            Row::Name if self.editing => {
                ("GAMERTAG".into(), Some(format!("{}_", self.profile.name)))
            }
            Row::Name => ("GAMERTAG".into(), Some(self.profile.name.clone())),
            Row::Model => (
                "MODEL".into(),
                Some(
                    if self.profile.look.elite {
                        "ELITE"
                    } else {
                        "SPARTAN"
                    }
                    .into(),
                ),
            ),
            Row::Primary => (
                "PRIMARY COLOR".into(),
                Some(color_name(self.profile.look.colors[0]).into()),
            ),
            Row::Secondary => (
                "SECONDARY COLOR".into(),
                Some(color_name(self.profile.look.colors[1]).into()),
            ),
            Row::Emblem => (
                "EMBLEM".into(),
                Some(format!(
                    "{}",
                    self.profile.look.emblem.foreground as u32 + 1
                )),
            ),
            Row::EmblemBackground => (
                "EMBLEM BACKGROUND".into(),
                Some(format!(
                    "{}",
                    self.profile.look.emblem.background as u32 + 1
                )),
            ),
            Row::EmblemPrimary => (
                "EMBLEM PRIMARY".into(),
                Some(color_name(self.profile.look.emblem.colors[0]).into()),
            ),
            Row::EmblemSecondary => (
                "EMBLEM SECONDARY".into(),
                Some(color_name(self.profile.look.emblem.colors[1]).into()),
            ),
            Row::EmblemBackColor => (
                "EMBLEM BACK COLOR".into(),
                Some(color_name(self.profile.look.emblem.colors[2]).into()),
            ),
            Row::Controls => ("CONTROLS".into(), None),
            Row::LookSensitivity => (
                "LOOK SENSITIVITY".into(),
                Some(self.profile.controls.look_sensitivity.to_string()),
            ),
            Row::MouseSensitivity => (
                "MOUSE SENSITIVITY".into(),
                Some(self.profile.controls.mouse_sensitivity.to_string()),
            ),
            Row::InvertLook => (
                "INVERT LOOK".into(),
                Some(on_off(self.profile.controls.invert_look)),
            ),
            Row::GameType => (
                "GAME TYPE".into(),
                Some(GAME_TYPES[s.game_type.min(GAME_TYPES.len() - 1)].1.into()),
            ),
            Row::Map => (
                "MAP".into(),
                Some(
                    ctx.maps
                        .get(s.map)
                        .map_or("NONE".into(), |m| m.title.clone()),
                ),
            ),
            Row::Score => ("SCORE TO WIN".into(), Some(score_label(s))),
            Row::Bots => ("BOTS".into(), Some(s.bots.to_string())),
            Row::GameOptions => ("GAME OPTIONS".into(), Some(options_label(&s.options))),
            Row::Variant => ("VARIANT".into(), Some(variant_label(&s.options).into())),
            Row::MapWeapons => (
                "WEAPONS ON MAP".into(),
                Some(s.options.map_weapons.label().into()),
            ),
            Row::StartPrimary => (
                "PRIMARY WEAPON".into(),
                Some(s.options.primary.label().into()),
            ),
            Row::StartSecondary => (
                "SECONDARY WEAPON".into(),
                Some(s.options.secondary.label().into()),
            ),
            Row::StartGrenades => ("STARTING GRENADES".into(), Some(on_off(s.options.grenades))),
            Row::Shields => ("SHIELDS".into(), Some(on_off(s.options.shields))),
            Row::Radar => ("MOTION SENSOR".into(), Some(on_off(s.options.radar))),
            Row::Vehicles => ("VEHICLES".into(), Some(on_off(s.options.vehicles))),
            Row::Respawn => (
                "RESPAWN TIME".into(),
                Some(match s.options.respawn_seconds() {
                    0 => "INSTANT".into(),
                    n => format!("{n} SECONDS"),
                }),
            ),
            Row::FriendlyFire => (
                "FRIENDLY FIRE".into(),
                Some(on_off(s.options.friendly_fire)),
            ),
            Row::TimeLimit => (
                "TIME LIMIT".into(),
                Some(match s.options.time_limit {
                    0 => "NONE".into(),
                    n if n % 60 == 0 => format!("{} MINUTES", n / 60),
                    n => format!("{n} SECONDS"),
                }),
            ),
            Row::StartGame => ("START GAME".into(), None),
            Row::Join(i) if ctx.lan[i].protocol != h2net::PROTOCOL => (
                ctx.lan[i].computer.to_uppercase(),
                Some("ANOTHER VERSION".into()),
            ),
            Row::Join(i) => {
                let g = &ctx.lan[i];
                (
                    format!("{} - {}", g.computer.to_uppercase(), map_title(&g.map)),
                    Some(format!("{}/16", g.players)),
                )
            }
            Row::Searching => ("SEARCHING FOR GAMES...".into(), None),
            Row::Waiting if in_custom(ctx) => ("WAITING FOR THE PARTY LEADER".into(), None),
            Row::Waiting => ("WAITING FOR THE HOST TO START".into(), None),
            Row::Resume => ("RESUME".into(), None),
            Row::EndGame if ctx.joined || in_match(ctx) => ("LEAVE GAME".into(), None),
            Row::EndGame => ("END GAME".into(), None),
            Row::Continue => match ctx.online.and_then(|o| o.returning) {
                Some(left) => (format!("RETURNING TO PARTY IN {}", left.ceil()), None),
                None => ("CONTINUE".into(), None),
            },
            Row::Online => ("ONLINE".into(), None),
            Row::Connecting if ctx.online.is_some_and(|o| o.waking) => {
                ("WAKING UP THE SERVER (UP TO A MINUTE)...".into(), None)
            }
            Row::Connecting => ("CONNECTING...".into(), None),
            Row::SignIn => ("SIGN IN".into(), None),
            Row::Matchmaking => ("MATCHMAKING".into(), None),
            Row::Quickmatch => ("QUICKMATCH".into(), None),
            Row::CustomGame => ("CUSTOM GAME".into(), None),
            Row::LeaveParty => ("LEAVE PARTY".into(), None),
            Row::Invite => ("INVITE TO PARTY".into(), None),
            Row::JoinTheirParty => ("JOIN PARTY".into(), None),
            Row::MakeLeader => ("MAKE PARTY LEADER".into(), None),
            Row::Remove => ("REMOVE FROM PARTY".into(), None),
            Row::Cancel => ("CANCEL".into(), None),
            Row::OnlinePlayers
            | Row::RecentPlayers
            | Row::Privacy
            | Row::AcceptInvite
            | Row::Player(_)
            | Row::RecentPlayer(_)
            | Row::Nothing
            | Row::Playlist(_)
            | Row::SearchStatus
            | Row::SearchDetail
            | Row::Starting => match ctx.online {
                Some(o) => self.online_label(row, o),
                None => (String::new(), None),
            },
        }
    }

    /// An online row's name and value, from what the service said.
    fn online_label(&self, row: Row, o: &OnlineView) -> (String, Option<String>) {
        match row {
            Row::OnlinePlayers => ("ONLINE PLAYERS".into(), Some(o.others().len().to_string())),
            Row::RecentPlayers => ("RECENT PLAYERS".into(), Some(o.recent.len().to_string())),
            Row::Privacy => {
                let open = o.party().is_none_or(|p| p.privacy == Privacy::Open);
                let privacy = if open { "OPEN" } else { "INVITE ONLY" };
                ("PARTY".into(), Some(privacy.into()))
            }
            Row::AcceptInvite => {
                let from = o.invite().map_or("", |(_, from)| from.as_str());
                (format!("JOIN {from}'S PARTY"), None)
            }
            Row::Player(i) => match o.others().get(i) {
                Some(p) => (p.gamertag.clone(), Some(o.doing(p))),
                None => (String::new(), None),
            },
            // Known by the gamertag they have now, if they're online.
            Row::RecentPlayer(i) => match o.recent.get(i) {
                Some(r) => {
                    let p = o.player(r.account);
                    let gamertag = p.map_or(&r.gamertag, |p| &p.gamertag);
                    (gamertag.clone(), Some(o.status(r.account)))
                }
                None => (String::new(), None),
            },
            Row::Nothing => {
                let gone = o.player(self.player).is_none();
                let why = match self.screen {
                    Screen::Players => "NO ONE ELSE IS ONLINE".into(),
                    Screen::RecentPlayers => "NO RECENT PLAYERS YET".into(),
                    Screen::Player if gone && self.list == Screen::RecentPlayers => {
                        o.status(self.player)
                    }
                    Screen::Player if gone => "NO LONGER ONLINE".into(),
                    Screen::Player => "IN YOUR PARTY".into(),
                    _ => "NO PLAYLISTS".into(),
                };
                (why, None)
            }
            Row::Playlist(i) => match o.playlists().get(i) {
                Some(p) => {
                    let players = format!("{} SEARCHING, {} PLAYING", p.searching, p.playing);
                    (o.text.playlist_name(p), Some(players))
                }
                None => (String::new(), None),
            },
            Row::SearchStatus => {
                let seconds = o.search.map_or(0.0, |(_, s)| s);
                let headline = progress_dots(&o.search_status().0, seconds);
                (headline, Some(clock_text(seconds)))
            }
            Row::SearchDetail => (o.search_status().1.unwrap_or_default(), None),
            Row::Starting => o.pregame_status(),
            _ => (String::new(), None),
        }
    }

    /// A row's name, without its value.
    fn label_name(&self, row: Row, ctx: &Context) -> String {
        let (name, _) = self.label(
            row,
            &Context {
                host_lobby: None,
                ..*ctx
            },
        );
        name
    }

    /// Keep the cursor on a row that's there and can be chosen, and in
    /// view.
    fn settle(&mut self, rows: &[Row], ctx: &Context) {
        self.cursor = self.settled(rows, ctx);
        self.follow_cursor();
    }

    /// Where `settle` puts the cursor.
    fn settled(&self, rows: &[Row], ctx: &Context) -> usize {
        let k = self.cursor.min(rows.len().saturating_sub(1));
        match rows.get(k) {
            Some(r) if !r.selectable(ctx) => {
                rows.iter().position(|r| r.selectable(ctx)).unwrap_or(k)
            }
            _ => k,
        }
    }

    pub fn input(&mut self, input: Input, ctx: &Context) -> Action {
        if self.editing {
            return match input {
                Input::Select | Input::Back => self.typed(Typed::Done),
                _ => Action::None,
            };
        }
        if self.screen == Screen::Lobby && ctx.host_lobby.is_some() {
            // Only the host changes its lobby.
            return match input {
                Input::Back => self.back(ctx),
                _ => Action::None,
            };
        }
        if self.screen == Screen::Pregame {
            // The match's game starts by itself.
            return Action::None;
        }
        let rows = self.rows(ctx);
        self.settle(&rows, ctx);
        let n = rows.len();
        let row = rows.get(self.cursor).copied();
        match input {
            Input::Up | Input::Down => {
                let selectable = rows.iter().filter(|r| r.selectable(ctx)).count();
                // The pause menu doesn't wrap round: QUIT is never a nudge
                // up from RESUME.
                let end = match input {
                    Input::Up => self.cursor == 0,
                    _ => self.cursor + 1 >= n,
                };
                if selectable > 1 && !(self.screen == Screen::Pause && end) {
                    loop {
                        self.cursor = if input == Input::Up {
                            (self.cursor + n - 1) % n
                        } else {
                            (self.cursor + 1) % n
                        };
                        if rows[self.cursor].selectable(ctx) {
                            break;
                        }
                    }
                    self.follow_cursor();
                    self.sound = Some(Sound::Cursor);
                }
                Action::None
            }
            Input::Left | Input::Right => {
                let step = if input == Input::Left { -1 } else { 1 };
                match row {
                    Some(Row::Privacy) if Row::Privacy.selectable(ctx) => {
                        self.choose(Row::Privacy, ctx)
                    }
                    Some(r) if self.adjust(r, step, ctx) => {
                        self.sound = Some(Sound::Cursor);
                        self.saving(r)
                    }
                    _ => Action::None,
                }
            }
            Input::Select => match row {
                Some(r) if r.selectable(ctx) => self.choose(r, ctx),
                _ => Action::None,
            },
            Input::Back => self.back(ctx),
        }
    }

    /// Change a setting by one step; false for rows without a value.
    fn adjust(&mut self, row: Row, step: i32, ctx: &Context) -> bool {
        let cycle = |v: usize, n: usize| -> usize {
            if n == 0 {
                0
            } else {
                (v as i32 + step).rem_euclid(n as i32) as usize
            }
        };
        let s = &mut self.settings;
        match row {
            Row::Difficulty => self.difficulty = cycle(self.difficulty, DIFFICULTIES.len()),
            Row::Map => s.map = next_map(s.map, step, ctx),
            Row::Score => s.score = cycle(s.score, scores(s.game_type()).0.len()),
            Row::Bots => s.bots = cycle(s.bots, MAX_BOTS + 1),
            Row::GameType => {
                s.game_type = cycle(s.game_type, GAME_TYPES.len());
                s.score = scores(s.game_type()).1;
            }
            Row::Variant => {
                let all = presets();
                let next = match s.options.preset() {
                    Some(k) => cycle(k, all.len()),
                    None if step > 0 => 0,
                    None => all.len() - 1,
                };
                s.options = GameOptions {
                    time_limit: s.options.time_limit,
                    ..all[next].1.clone()
                };
            }
            Row::MapWeapons => s.options.map_weapons = s.options.map_weapons.step(step),
            Row::StartPrimary => s.options.primary = s.options.primary.step(step),
            Row::StartSecondary => s.options.secondary = s.options.secondary.step(step),
            Row::StartGrenades => s.options.grenades ^= true,
            Row::Shields => s.options.shields ^= true,
            Row::Radar => s.options.radar ^= true,
            Row::Vehicles => s.options.vehicles ^= true,
            Row::Respawn => s.options.respawn = cycle(s.options.respawn, RESPAWN_TIMES.len()),
            Row::FriendlyFire => s.options.friendly_fire ^= true,
            Row::TimeLimit => {
                let at = TIME_LIMITS.iter().position(|&t| t == s.options.time_limit);
                s.options.time_limit = TIME_LIMITS[cycle(at.unwrap_or(0), TIME_LIMITS.len())];
            }
            Row::Model => self.profile.look.elite = !self.profile.look.elite,
            Row::Primary | Row::Secondary => {
                let c = &mut self.profile.look.colors[(row == Row::Secondary) as usize];
                *c = cycle(*c as usize, PROFILE_COLORS as usize) as u8;
            }
            Row::Emblem => {
                let e = &mut self.profile.look.emblem.foreground;
                *e = cycle(*e as usize, EMBLEM_FOREGROUNDS as usize) as u8;
            }
            Row::EmblemBackground => {
                let e = &mut self.profile.look.emblem.background;
                *e = cycle(*e as usize, EMBLEM_BACKGROUNDS as usize) as u8;
            }
            Row::EmblemPrimary | Row::EmblemSecondary | Row::EmblemBackColor => {
                let k = match row {
                    Row::EmblemPrimary => 0,
                    Row::EmblemSecondary => 1,
                    _ => 2,
                };
                let c = &mut self.profile.look.emblem.colors[k];
                *c = cycle(*c as usize, PROFILE_COLORS as usize) as u8;
            }
            // Sensitivity runs from 1 to 10.
            Row::LookSensitivity | Row::MouseSensitivity => {
                let c = &mut self.profile.controls;
                let n = if row == Row::LookSensitivity {
                    &mut c.look_sensitivity
                } else {
                    &mut c.mouse_sensitivity
                };
                *n = 1 + cycle(
                    n.saturating_sub(1) as usize,
                    Controls::MAX_SENSITIVITY as usize,
                ) as u8;
            }
            Row::InvertLook => self.profile.controls.invert_look ^= true,
            _ => return false,
        }
        true
    }

    fn choose(&mut self, row: Row, ctx: &Context) -> Action {
        self.notice = None;
        let forward = |m: &mut Menu, screen| {
            m.show(screen);
            m.sound = Some(Sound::Forward);
            Action::None
        };
        match row {
            Row::Difficulty => {
                if self.adjust(row, 1, ctx) {
                    self.sound = Some(Sound::Cursor);
                }
                Action::None
            }
            Row::Mission(i) => {
                self.sound = Some(Sound::Advance);
                Action::Mission(i)
            }
            Row::NoMissions => Action::None,
            Row::Multiplayer => forward(self, Screen::Lobby),
            Row::SystemLink => forward(self, Screen::SystemLink),
            Row::Profile => forward(self, Screen::Profile),
            Row::Controls => forward(self, Screen::Controls),
            Row::Quit => Action::Quit,
            Row::Name => {
                self.editing = true;
                self.sound = Some(Sound::Forward);
                Action::None
            }
            Row::GameOptions => forward(self, Screen::Options),
            Row::GameType
            | Row::Map
            | Row::Score
            | Row::Bots
            | Row::Variant
            | Row::MapWeapons
            | Row::StartPrimary
            | Row::StartSecondary
            | Row::StartGrenades
            | Row::Shields
            | Row::Radar
            | Row::Vehicles
            | Row::Respawn
            | Row::FriendlyFire
            | Row::TimeLimit => {
                if self.adjust(row, 1, ctx) {
                    self.sound = Some(Sound::Cursor);
                }
                Action::None
            }
            Row::Model
            | Row::Primary
            | Row::Secondary
            | Row::Emblem
            | Row::EmblemBackground
            | Row::EmblemPrimary
            | Row::EmblemSecondary
            | Row::EmblemBackColor
            | Row::LookSensitivity
            | Row::MouseSensitivity
            | Row::InvertLook => {
                if self.adjust(row, 1, ctx) {
                    self.sound = Some(Sound::Cursor);
                }
                self.saving(row)
            }
            Row::StartGame if ctx.maps.is_empty() => {
                self.notice = Some("NO MULTIPLAYER MAPS FOUND".into());
                Action::None
            }
            Row::StartGame => {
                self.sound = Some(Sound::Advance);
                Action::Start
            }
            Row::Join(i) => match ctx.lan.get(i) {
                Some(g) if g.protocol != h2net::PROTOCOL => {
                    self.notice = Some(format!(
                        "{} RUNS ANOTHER VERSION, UPDATE BOTH PCS",
                        g.computer.to_uppercase()
                    ));
                    Action::None
                }
                Some(g) => {
                    self.sound = Some(Sound::Advance);
                    Action::Join(g.clone())
                }
                None => Action::None,
            },
            Row::Searching | Row::Waiting => Action::None,
            Row::Resume => {
                self.sound = Some(Sound::Back);
                Action::Resume
            }
            Row::EndGame | Row::Continue => self.end_game(ctx),
            Row::Online | Row::SignIn => {
                self.show(Screen::Live);
                self.sound = Some(Sound::Forward);
                Action::GoOnline
            }
            Row::Matchmaking => forward(self, Screen::Playlists),
            Row::OnlinePlayers => forward(self, Screen::Players),
            Row::RecentPlayers => forward(self, Screen::RecentPlayers),
            Row::Quickmatch => {
                self.show(Screen::Matchmaking);
                self.sound = Some(Sound::Advance);
                Action::Search(None)
            }
            Row::CustomGame => {
                self.sound = Some(Sound::Advance);
                Action::Custom
            }
            Row::Privacy => {
                let party = ctx.online.and_then(|o| o.party());
                let open = party.is_none_or(|p| p.privacy == Privacy::Open);
                self.sound = Some(Sound::Cursor);
                Action::Privacy(if open {
                    Privacy::InviteOnly
                } else {
                    Privacy::Open
                })
            }
            Row::AcceptInvite => match ctx.online.and_then(|o| o.invite()) {
                Some(&(party, _)) => {
                    // On ONLINE PLAYERS: once in, LEAVE PARTY takes this row.
                    self.cursor = 3;
                    self.sound = Some(Sound::Advance);
                    Action::Accept(party)
                }
                None => Action::None,
            },
            Row::LeaveParty => {
                self.sound = Some(Sound::Back);
                Action::LeaveParty
            }
            Row::Player(i) => match ctx.online.and_then(|o| o.others().get(i).copied()) {
                Some(p) => {
                    (self.player, self.list) = (p.account, Screen::Players);
                    forward(self, Screen::Player)
                }
                None => Action::None,
            },
            Row::RecentPlayer(i) => match ctx.online.and_then(|o| o.recent.get(i)) {
                Some(r) => {
                    (self.player, self.list) = (r.account, Screen::RecentPlayers);
                    forward(self, Screen::Player)
                }
                None => Action::None,
            },
            Row::Invite | Row::JoinTheirParty | Row::MakeLeader | Row::Remove => {
                self.player_option(row, ctx)
            }
            Row::Playlist(i) => self.search(i, ctx),
            Row::Cancel => {
                self.back_to(Screen::Live, Row::Matchmaking, ctx);
                Action::CancelSearch
            }
            Row::Connecting
            | Row::Nothing
            | Row::SearchStatus
            | Row::SearchDetail
            | Row::Starting => Action::None,
        }
    }

    /// Do what was chosen about the player picked.
    fn player_option(&mut self, row: Row, ctx: &Context) -> Action {
        let Some(o) = ctx.online else {
            return Action::None;
        };
        let Some(p) = o.player(self.player) else {
            return Action::None;
        };
        if row == Row::JoinTheirParty {
            self.show(Screen::Live);
        } else {
            self.back_to_list(ctx);
        }
        self.sound = Some(Sound::Advance);
        match row {
            Row::Invite => {
                self.notice = Some(format!("PARTY INVITE SENT TO {}", p.gamertag));
                Action::Invite(p.account)
            }
            Row::JoinTheirParty => Action::JoinParty(p.party),
            Row::MakeLeader => Action::Promote(p.account),
            _ => Action::Kick(p.account),
        }
    }

    /// Search playlist `i` with the party, if it can.
    fn search(&mut self, i: usize, ctx: &Context) -> Action {
        let Some(o) = ctx.online else {
            return Action::None;
        };
        let Some(p) = o.playlists().get(i) else {
            return Action::None;
        };
        if let Some(why) = o.cant_search(p) {
            self.notice = Some(why);
            return Action::None;
        }
        self.show(Screen::Matchmaking);
        self.sound = Some(Sound::Advance);
        Action::Search(Some(p.id))
    }

    /// The online service turned down our gamertag: type another, here.
    pub fn ask_gamertag(&mut self, why: &str) {
        self.show(Screen::Live);
        self.editing = true;
        self.notice = Some(why.into());
    }

    /// Leave the game (or the carnage report) for the lobby: the host's
    /// lobby when joined, once the host is back there.
    fn end_game(&mut self, ctx: &Context) -> Action {
        self.sound = Some(Sound::Forward);
        // Online, everyone leaves the match for their party.
        match (ctx.joined && !in_match(ctx), self.screen) {
            (true, Screen::PostGame) => {
                self.notice = Some("WAITING FOR THE HOST".into());
                Action::None
            }
            (true, _) => Action::Leave,
            (false, _) => Action::EndGame,
        }
    }

    /// After changing a row's value: profile rows are kept.
    fn saving(&self, row: Row) -> Action {
        if row.in_profile() {
            Action::SaveProfile
        } else {
            Action::None
        }
    }

    /// A key typed while editing the gamertag.
    pub fn typed(&mut self, typed: Typed) -> Action {
        if !self.editing {
            return Action::None;
        }
        let name = &mut self.profile.name;
        match typed {
            Typed::Text(text) => {
                let before = name.len();
                name.extend(text.chars().filter(|&c| name_char(c)));
                name.make_ascii_uppercase();
                name.truncate(MAX_NAME);
                if name.len() != before {
                    self.sound = Some(Sound::Cursor);
                }
                Action::None
            }
            Typed::Erase => {
                if name.pop().is_some() {
                    self.sound = Some(Sound::Cursor);
                }
                Action::None
            }
            Typed::Done => {
                self.editing = false;
                *name = clean_name(name);
                if name.is_empty() {
                    *name = Profile::default().name;
                }
                if self.screen == Screen::Live {
                    // On to SIGN IN, below.
                    self.cursor += 1;
                }
                self.sound = Some(Sound::Back);
                Action::SaveProfile
            }
        }
    }

    /// Back on the main menu, on the item that led here, leaving any notice
    /// behind.
    fn back_to_main(&mut self, from: Row) {
        self.show(Screen::Main);
        self.notice = None;
        self.cursor = MAIN_ROWS.iter().position(|&r| r == from).unwrap_or(0);
        self.sound = Some(Sound::Back);
    }

    /// Back from the player picked to the list they were picked from, on
    /// their row.
    fn back_to_list(&mut self, ctx: &Context) {
        let o = ctx.online;
        let row = if self.list == Screen::RecentPlayers {
            let recent = o.map_or(&[][..], |o| o.recent);
            let k = recent.iter().position(|r| r.account == self.player);
            Row::RecentPlayer(k.unwrap_or(0))
        } else {
            let others = o.map(|o| o.others()).unwrap_or_default();
            let k = others.iter().position(|p| p.account == self.player);
            Row::Player(k.unwrap_or(0))
        };
        self.back_to(self.list, row, ctx);
    }

    /// Back to `screen`, on `row`, leaving any notice behind.
    fn back_to(&mut self, screen: Screen, row: Row, ctx: &Context) {
        self.show(screen);
        self.notice = None;
        self.cursor = self.rows(ctx).iter().position(|&r| r == row).unwrap_or(0);
        self.sound = Some(Sound::Back);
    }

    fn back(&mut self, ctx: &Context) -> Action {
        match self.screen {
            // Back to the party (the custom game is over, if we host it).
            Screen::Lobby if in_custom(ctx) => {
                self.back_to(Screen::Live, Row::CustomGame, ctx);
                Action::Leave
            }
            Screen::Lobby if ctx.joined => {
                self.sound = Some(Sound::Back);
                Action::Leave
            }
            Screen::Main => Action::None,
            Screen::Profile => {
                self.back_to_main(Row::Profile);
                Action::None
            }
            Screen::Controls => {
                self.back_to(Screen::Profile, Row::Controls, ctx);
                Action::None
            }
            Screen::Campaign => {
                self.back_to_main(Row::Multiplayer);
                Action::None
            }
            Screen::Options => {
                self.show(Screen::Lobby);
                self.cursor = 4;
                self.sound = Some(Sound::Back);
                Action::None
            }
            Screen::Lobby => {
                self.back_to_main(Row::Multiplayer);
                Action::None
            }
            Screen::SystemLink => {
                self.back_to_main(Row::SystemLink);
                Action::None
            }
            Screen::Pause => {
                self.sound = Some(Sound::Back);
                Action::Resume
            }
            Screen::PostGame => self.end_game(ctx),
            Screen::Live => {
                self.back_to_main(Row::Online);
                Action::SignOut
            }
            Screen::Players => {
                self.back_to(Screen::Live, Row::OnlinePlayers, ctx);
                Action::None
            }
            Screen::RecentPlayers => {
                self.back_to(Screen::Live, Row::RecentPlayers, ctx);
                Action::None
            }
            Screen::Player => {
                self.back_to_list(ctx);
                Action::None
            }
            Screen::Playlists => {
                self.back_to(Screen::Live, Row::Matchmaking, ctx);
                Action::None
            }
            Screen::Matchmaking => {
                self.back_to(Screen::Live, Row::Matchmaking, ctx);
                if Row::Cancel.selectable(ctx) {
                    Action::CancelSearch
                } else {
                    Action::None
                }
            }
            Screen::Pregame => Action::None,
        }
    }

    fn row_rect(&self, k: usize) -> [f32; 4] {
        let wide = matches!(self.screen, Screen::SystemLink | Screen::Options);
        // The online lists are wider still, for players' doings.
        let players = matches!(self.screen, Screen::Players | Screen::RecentPlayers);
        let list = players || matches!(self.screen, Screen::Playlists | Screen::Matchmaking);
        // The game options' eleven rows, the profile's ten (and the lists
        // of players) sit closer to fit above the hint.
        let step = if matches!(self.screen, Screen::Options | Screen::Profile) || players {
            ROW_STEP - 4.0
        } else {
            ROW_STEP
        };
        let y = ROW_Y + k as f32 * step + self.rows_offset();
        let w = if list {
            480.0
        } else if wide {
            400.0
        } else {
            ROW_W
        };
        [ROW_X, y, ROW_X + w, y + ROW_H]
    }

    /// The carnage report pushes its rows below the scoreboard.
    fn rows_offset(&self) -> f32 {
        if self.screen == Screen::PostGame {
            280.0
        } else {
            0.0
        }
    }

    fn row_at(&self, [x, y]: [f32; 2], w: f32, h: f32, ctx: &Context) -> Option<(usize, f32)> {
        let f = Frame::new(w, h);
        let rows = self.rows(ctx);
        let shown = self.shown(rows.len());
        let first = shown.start;
        shown.into_iter().find_map(|k| {
            let [x0, y0, x1, y1] = f.rect(self.row_rect(k - first));
            (x >= x0 && x < x1 && y >= y0 && y < y1 && rows[k].selectable(ctx))
                .then(|| (k, (x - x0) / (x1 - x0)))
        })
    }

    /// The mouse moved to `pos` (window pixels).
    pub fn hover(&mut self, pos: [f32; 2], w: f32, h: f32, ctx: &Context) {
        if let Some((k, _)) = self.row_at(pos, w, h, ctx) {
            if k != self.cursor {
                self.cursor = k;
                self.sound = Some(Sound::Cursor);
            }
        }
    }

    /// A click at `pos`: settings step down on their left part and up
    /// elsewhere; other rows are chosen.
    pub fn click(&mut self, pos: [f32; 2], w: f32, h: f32, ctx: &Context) -> Action {
        let Some((k, along)) = self.row_at(pos, w, h, ctx) else {
            return Action::None;
        };
        self.cursor = k;
        let row = self.rows(ctx)[k];
        let (_, value) = self.label(row, ctx);
        let stepped =
            matches!(row, Row::Map | Row::Score | Row::Bots | Row::GameType) || row.in_profile();
        if value.is_some() && stepped && !self.editing {
            let input = if along < 0.6 {
                Input::Left
            } else {
                Input::Right
            };
            return self.input(input, ctx);
        }
        self.input(Input::Select, ctx)
    }

    /// The mouse wheel over a setting changes it; over a list, it moves
    /// along the list.
    pub fn wheel(&mut self, up: bool, ctx: &Context) {
        let input = match (self.lists(), up) {
            (true, true) => Input::Up,
            (true, false) => Input::Down,
            (false, true) => Input::Right,
            (false, false) => Input::Left,
        };
        self.input(input, ctx);
    }

    pub fn draw(
        &self,
        hb: &mut HudBuilder,
        font: usize,
        white: usize,
        w: f32,
        h: f32,
        ctx: &Context,
    ) {
        let f = Frame::new(w, h);
        let s = f.s;
        // Darken behind the menu so it reads over any map.
        match self.screen {
            Screen::Main
            | Screen::Campaign
            | Screen::Lobby
            | Screen::Options
            | Screen::SystemLink
            | Screen::Profile
            | Screen::Controls
            | Screen::Live
            | Screen::Players
            | Screen::RecentPlayers
            | Screen::Player
            | Screen::Playlists
            | Screen::Matchmaking
            | Screen::Pregame => {
                hb.quad(
                    white,
                    [0.0, 0.0, w, h],
                    [0.0; 4],
                    [0.0, 0.0, 0.0, 0.35],
                    hud_mode::PLAIN,
                    0.0,
                );
            }
            Screen::Pause => {
                hb.quad(
                    white,
                    [0.0, 0.0, w, h],
                    [0.0; 4],
                    [0.0, 0.0, 0.0, 0.55],
                    hud_mode::PLAIN,
                    0.0,
                );
            }
            Screen::PostGame => {
                hb.quad(
                    white,
                    [0.0, 0.0, w, h],
                    [0.0; 4],
                    [0.0, 0.0, 0.02, 0.8],
                    hud_mode::PLAIN,
                    0.0,
                );
            }
        }
        hb.text_left(font, f.at(ROW_X, 48.0), 26.0 * s, self.title(ctx), BRIGHT);
        let rule = f.rect([ROW_X, 84.0, ROW_X + 300.0, 86.0]);
        hb.quad(white, rule, [0.0; 4], HIGHLIGHT, hud_mode::PLAIN, 0.0);

        if self.screen == Screen::PostGame {
            if let Some(outcome) = ctx.outcome {
                let width = outcome.chars().count() as f32 * 14.0 * crate::font::ASPECT;
                let at = f.at(ROW_X + 490.0 - width, 60.0);
                hb.text_left(font, at, 14.0 * s, outcome, TEXT);
            }
            draw_scores(hb, font, white, &f, 104.0, ctx.scores);
        }
        let rows = self.rows(ctx);
        let cursor = self.settled(&rows, ctx);
        let shown = self.shown(rows.len());
        if shown.len() < rows.len() {
            // Where in a long list the rows shown are.
            let at = f.at(ROW_X + 10.0, self.row_rect(shown.len())[1] + 1.0);
            let place = format!("{}-{} OF {}", shown.start + 1, shown.end, rows.len());
            hb.text_left(font, at, 8.0 * s, &place, DIM);
        }
        let first = shown.start;
        for (k, &row) in rows.iter().enumerate().take(shown.end).skip(first) {
            let rect = self.row_rect(k - first);
            let fixed = self.screen == Screen::Pregame
                || self.screen == Screen::Lobby && ctx.host_lobby.is_some();
            let selectable = row.selectable(ctx);
            let selected = k == cursor && selectable && !fixed;
            let (label, value) = self.label(row, ctx);
            // Playlists the party lacks maps for are marked, as Halo 2 did,
            // and recent players who are offline dimmed.
            let dimmed = match (row, ctx.online) {
                (Row::Playlist(i), Some(o)) => o
                    .playlists()
                    .get(i)
                    .is_some_and(|p| !o.missing_maps(p).is_empty()),
                (Row::RecentPlayer(i), Some(o)) => o
                    .recent
                    .get(i)
                    .is_some_and(|r| o.player(r.account).is_none()),
                _ => false,
            };
            let (bg, fg) = if selected {
                (HIGHLIGHT, BRIGHT)
            } else if selectable && !dimmed {
                (PANEL, TEXT)
            } else if selectable {
                (PANEL, DIM)
            } else if matches!(row, Row::SearchStatus | Row::Starting) {
                ([0.0; 4], BRIGHT)
            } else {
                ([0.0; 4], DIM)
            };
            hb.quad(white, f.rect(rect), [0.0; 4], bg, hud_mode::PLAIN, 0.0);
            let text_y = rect[1] + (ROW_H - 11.0) * 0.5;
            // Players have icons on the left; playlists, on the right.
            let (left, right) = match row {
                Row::Player(_) | Row::RecentPlayer(_) => (30.0, 10.0),
                Row::Playlist(_) => (10.0, 30.0),
                _ => (10.0, 10.0),
            };
            hb.text_left(font, f.at(rect[0] + left, text_y), 11.0 * s, &label, fg);
            if let Some(v) = value {
                let steps = !matches!(
                    row,
                    Row::Join(_)
                        | Row::Name
                        | Row::OnlinePlayers
                        | Row::RecentPlayers
                        | Row::Player(_)
                        | Row::RecentPlayer(_)
                        | Row::Playlist(_)
                        | Row::SearchStatus
                );
                let v = if selected && steps {
                    format!("< {v} >")
                } else {
                    v
                };
                // What players are doing is long: smaller.
                let size = if left + right > 20.0 { 9.0 } else { 11.0 };
                let width = v.chars().count() as f32 * size * crate::font::ASPECT;
                let at = f.at(rect[2] - right - width, rect[1] + (ROW_H - size) * 0.5);
                hb.text_left(font, at, size * s, &v, fg);
            }
            if let Some(o) = ctx.online {
                draw_icons(hb, &f, row, rect, &label, o);
            }
        }
        if self.screen == Screen::Lobby {
            let top = self.draw_map(hb, font, &f, ctx, white);
            self.draw_players(hb, font, white, &f, ctx, top);
        }
        if self.screen == Screen::Pregame {
            let top = self.draw_map(hb, font, &f, ctx, white);
            draw_roster(hb, font, white, &f, ctx, top);
        }
        if self.screen == Screen::Profile {
            self.draw_profile(hb, font, white, &f);
        }
        if let Some(o) = ctx.online {
            match self.screen {
                Screen::Live => draw_party(hb, font, white, &f, o),
                Screen::Player => draw_player(hb, font, white, &f, o, self.player),
                _ => {}
            }
        }
        if self.screen == Screen::Pause && !ctx.objectives.is_empty() {
            draw_objectives(hb, font, white, &f, ctx.objectives);
        }
        if let Some(header) = self.header(ctx) {
            // System Link's says only what's below, so it's quieter.
            let color = if self.screen == Screen::SystemLink {
                DIM
            } else {
                TEXT
            };
            hb.text_left(font, f.at(ROW_X, ROW_Y - 22.0), 9.0 * s, &header, color);
        }
        if let Some(n) = &self.notice {
            hb.text_left(font, f.at(ROW_X, 400.0), 10.0 * s, n, WARNING);
        } else if let Some(note) = self.note(ctx) {
            let per_line = (540.0 / (9.0 * crate::font::ASPECT)) as usize;
            for (k, line) in wrap(&note, per_line).iter().take(3).enumerate() {
                let at = f.at(ROW_X, 400.0 + 12.0 * k as f32);
                hb.text_left(font, at, 9.0 * s, line, TEXT);
            }
        }
        let hint = match self.screen {
            _ if self.editing => "TYPE A GAMERTAG, THEN PRESS ENTER",
            Screen::Main => "ENTER OR A: SELECT",
            Screen::PostGame => "ENTER OR A: CONTINUE",
            Screen::Pregame => "",
            Screen::Live if ctx.online.is_some_and(|o| o.live.is_some()) => {
                "ENTER OR A: SELECT   ESC OR B: SIGN OUT"
            }
            Screen::Lobby if in_custom(ctx) => "ENTER OR A: SELECT   ESC OR B: BACK TO THE PARTY",
            _ => "ENTER OR A: SELECT   ESC OR B: BACK",
        };
        hb.text_left(font, f.at(ROW_X, HINT_Y), 8.0 * s, hint, DIM);
    }

    /// A line over the rows saying what they are.
    fn header(&self, ctx: &Context) -> Option<String> {
        let o = ctx.online;
        match self.screen {
            Screen::SystemLink => Some("GAMES ON YOUR NETWORK".into()),
            Screen::Live => {
                let o = o?;
                let me = &o.live?.welcome.as_ref()?.gamertag;
                match o.leader_text() {
                    Some(text) if !o.leads() => Some(text),
                    _ => Some(format!("SIGNED IN AS {me}")),
                }
            }
            Screen::Playlists => Some("CHOOSE A PLAYLIST".into()),
            Screen::Matchmaking => Some(o?.search_name()),
            Screen::Pregame => {
                let o = o?;
                Some(o.playlist_name(o.game?.playlist))
            }
            _ => None,
        }
    }

    /// What shows where notices go when there's none: the chosen
    /// playlist's description (or who lacks which of its maps), what we
    /// last played with the chosen recent player and when, or the online
    /// service's message of the day.
    fn note(&self, ctx: &Context) -> Option<String> {
        let o = ctx.online?;
        match self.screen {
            Screen::RecentPlayers => {
                let Some(&Row::RecentPlayer(i)) = self.rows(ctx).get(self.cursor) else {
                    return None;
                };
                Some(o.last_played(o.recent.get(i)?))
            }
            Screen::Playlists => {
                let Some(&Row::Playlist(i)) = self.rows(ctx).get(self.cursor) else {
                    return None;
                };
                let p = o.playlists().get(i)?;
                Some(
                    o.missing_text(p)
                        .unwrap_or_else(|| o.text.playlist_description(p)),
                )
            }
            Screen::Live => Some(o.live?.motd.to_uppercase()),
            _ => None,
        }
        .filter(|note| !note.is_empty())
    }

    /// The chosen map's picture and description over the player list.
    /// Returns where the player list goes.
    fn draw_map(
        &self,
        hb: &mut HudBuilder,
        font: usize,
        f: &Frame,
        ctx: &Context,
        white: usize,
    ) -> f32 {
        let Some(map) = ctx.maps.get(self.settings.map) else {
            return ROW_Y;
        };
        // In another PC's lobby on a map not here, there's none to show.
        let theirs = ctx
            .host_lobby
            .is_some_and(|l| !l.map.eq_ignore_ascii_case(&map.name));
        if theirs || map.picture.is_none() && map.description.is_empty() {
            return ROW_Y;
        }
        let s = f.s;
        let (x, y) = (PANEL_X, ROW_Y);
        let text_x = if map.picture.is_some() {
            x + PICTURE[0] + 10.0
        } else {
            x
        };
        let per_line = ((PANEL_X + PANEL_W - text_x) / (7.0 * crate::font::ASPECT)) as usize;
        let lines = wrap(&map.description.to_uppercase(), per_line);
        let mut height = 18.0 + 10.0 * lines.len() as f32;
        if map.picture.is_some() {
            height = height.max(PICTURE[1]);
        }
        let back = f.rect([x - 8.0, y - 8.0, x + PANEL_W + 8.0, y + height + 8.0]);
        hb.quad(white, back, [0.0; 4], PANEL, hud_mode::PLAIN, 0.0);
        if let Some(p) = map.picture {
            hb.quad(
                crate::gpu::MENU_TEXTURES + p,
                f.rect([x, y, x + PICTURE[0], y + PICTURE[1]]),
                [0.0, 0.0, 1.0, 1.0],
                [1.0; 4],
                hud_mode::PLAIN,
                0.0,
            );
        }
        hb.text_left(font, f.at(text_x, y), 11.0 * s, &map.title, BRIGHT);
        for (k, line) in lines.iter().enumerate() {
            let at = f.at(text_x, y + 18.0 + 10.0 * k as f32);
            hb.text_left(font, at, 7.0 * s, line, TEXT);
        }
        y + height + 20.0
    }

    /// The profile's emblem, at the top right (the model stands below it,
    /// drawn with the level).
    fn draw_profile(&self, hb: &mut HudBuilder, font: usize, white: usize, f: &Frame) {
        let (x, y) = (PROFILE_EMBLEM[0], PROFILE_EMBLEM[1]);
        let size = PROFILE_EMBLEM[2];
        hb.quad(
            white,
            f.rect([x - 6.0, y - 6.0, x + size + 6.0, y + size + 20.0]),
            [0.0; 4],
            PANEL,
            hud_mode::PLAIN,
            0.0,
        );
        crate::emblem::draw(
            hb,
            f.rect([x, y, x + size, y + size]),
            self.profile.look.emblem,
        );
        let label = "EMBLEM";
        let width = label.len() as f32 * 7.0 * crate::font::ASPECT;
        let at = f.at(x + (size - width) * 0.5, y + size + 6.0);
        hb.text_left(font, at, 7.0 * f.s, label, DIM);
    }

    fn draw_players(
        &self,
        hb: &mut HudBuilder,
        font: usize,
        white: usize,
        f: &Frame,
        ctx: &Context,
        top: f32,
    ) {
        let s = f.s;
        let (x, mut y) = (PANEL_X, top);
        let (teams, bots) = match ctx.host_lobby {
            Some(l) => (l.teams, l.bots as usize),
            None => (self.settings.game_type().teams(), self.settings.bots),
        };
        let lines = ctx.seats.len() + (bots > 0) as usize;
        let invite = ctx.local < crate::MAX_LOCAL;
        let hints = invite as usize * 2 + teams as usize;
        let height = 34.0 + 16.0 * lines as f32 + 6.0 + 11.0 * hints as f32;
        hb.quad(
            white,
            f.rect([x - 8.0, y - 8.0, x + PANEL_W + 8.0, y + height]),
            [0.0; 4],
            PANEL,
            hud_mode::PLAIN,
            0.0,
        );
        hb.text_left(font, f.at(x, y), 11.0 * s, "PLAYERS", BRIGHT);
        y += 24.0;
        for seat in ctx.seats {
            let c = match (teams, seat.team) {
                // The host puts them on a team when the game starts.
                (true, NO_TEAM) => UNPICKED,
                (true, t) => crate::local::TEAM_COLORS[t.min(1) as usize],
                (false, _) => crate::local::armor_colors(seat.look)[0],
            };
            let how = seat.how;
            // Their colour (or team's) beside their emblem.
            let bar = f.rect([x, y - 1.0, x + 3.0, y + 10.0]);
            hb.quad(white, bar, [0.0; 4], gamma_color(c), hud_mode::PLAIN, 0.0);
            let badge = f.rect([x + 5.0, y - 1.0, x + 16.0, y + 10.0]);
            crate::emblem::draw(hb, badge, seat.look.emblem);
            let line = format!("{}  {how}", seat.name);
            hb.text_left(font, f.at(x + 21.0, y), 9.0 * s, &line, TEXT);
            // Their rank, at the end of the line.
            if let Some(level) = seat.level {
                let icon = f.rect([x + PANEL_W - 13.0, y - 2.0, x + PANEL_W, y + 11.0]);
                crate::rank::draw(hb, icon, level);
            }
            y += 16.0;
        }
        if bots > 0 {
            let line = format!("+ {bots} BOT{}", if bots == 1 { "" } else { "S" });
            hb.text_left(font, f.at(x + 21.0, y), 9.0 * s, &line, DIM);
            y += 16.0;
        }
        y += 6.0;
        if teams {
            hb.text_left(font, f.at(x, y), 7.0 * s, "T OR X: CHANGE TEAM", DIM);
            y += 11.0;
        }
        // Player one at the keyboard here: a controller can take over.
        let keyboard =
            ctx.host_lobby.is_none() && ctx.seats.first().is_some_and(|p| p.how == KEYBOARD);
        let lines = match keyboard {
            true => [
                "A ON A CONTROLLER: PLAY AS PLAYER ONE",
                "START ON ANOTHER: PLAY IN SPLITSCREEN",
            ],
            false => ["PRESS START ON A CONTROLLER", "TO PLAY IN SPLITSCREEN"],
        };
        if invite {
            for (k, line) in lines.iter().enumerate() {
                hb.text_left(font, f.at(x, y + 11.0 * k as f32), 7.0 * s, line, DIM);
            }
        }
    }
}

/// In an online match.
fn in_match(ctx: &Context) -> bool {
    ctx.online.is_some_and(|o| o.game.is_some())
}

/// In the party's custom game, online.
fn in_custom(ctx: &Context) -> bool {
    ctx.online.is_some_and(|o| o.custom)
}

/// The map `step` on from map `k`, of those that can be played: in a
/// custom game, the ones everyone in the party has.
fn next_map(k: usize, step: i32, ctx: &Context) -> usize {
    let party = ctx.online.filter(|o| o.custom).and_then(|o| o.party());
    let playable = |m: &MapChoice| {
        party.is_none_or(|p| p.maps.iter().any(|n| n.eq_ignore_ascii_case(&m.name)))
    };
    let n = ctx.maps.len() as i32;
    (1..=n)
        .map(|i| (k as i32 + step * i).rem_euclid(n) as usize)
        .find(|&j| playable(&ctx.maps[j]))
        .unwrap_or(k)
}

/// The lowest the pregame's player list reaches (screen units), above the
/// notice line.
const ROSTER_BOTTOM: f32 = 392.0;
/// Most players the pregame lists in one column; more go in two, a team
/// in each in team games.
const ROSTER_ROWS: usize = 8;

/// Everyone in an online match, beside its pregame lobby: their team's
/// colour (or their own), emblem, gamertag and rank, large.
fn draw_roster(hb: &mut HudBuilder, font: usize, white: usize, f: &Frame, ctx: &Context, top: f32) {
    let s = f.s;
    let teams = ctx.host_lobby.is_some_and(|l| l.teams);
    let seats = ctx.seats;
    let columns: Vec<Vec<&SeatInfo>> = if seats.len() <= ROSTER_ROWS {
        vec![seats.iter().collect()]
    } else if teams {
        let team = |t: u8| seats.iter().filter(|p| p.team.min(1) == t).collect();
        vec![team(0), team(1)]
    } else {
        let half = seats.len().div_ceil(2);
        seats.chunks(half).map(|c| c.iter().collect()).collect()
    };
    let x = PANEL_X;
    // Rows in two columns are as small as in a full one, so long gamertags
    // fit beside the ranks.
    let longest = columns.iter().map(Vec::len).max().unwrap_or(0);
    let n = longest.max(if columns.len() > 1 { ROSTER_ROWS } else { 1 }) as f32;
    let step = ((ROSTER_BOTTOM - top - 30.0) / n).clamp(10.0, 22.0);
    let back = f.rect([x - 8.0, top - 8.0, x + PANEL_W + 8.0, top + 30.0 + step * n]);
    hb.quad(white, back, [0.0; 4], PANEL, hud_mode::PLAIN, 0.0);
    hb.text_left(font, f.at(x, top), 11.0 * s, "PLAYERS", BRIGHT);
    // Columns a little apart.
    let width = (PANEL_W + 8.0) / columns.len() as f32 - 8.0;
    for (k, column) in columns.iter().enumerate() {
        let x = PANEL_X + (width + 8.0) * k as f32;
        let mut y = top + 26.0;
        for seat in column {
            let c = if teams {
                crate::local::TEAM_COLORS[seat.team.min(1) as usize]
            } else {
                crate::local::armor_colors(seat.look)[0]
            };
            let h = step - 4.0;
            let bar = f.rect([x, y, x + 3.0, y + h]);
            hb.quad(white, bar, [0.0; 4], gamma_color(c), hud_mode::PLAIN, 0.0);
            let badge = f.rect([x + 6.0, y, x + 6.0 + h, y + h]);
            crate::emblem::draw(hb, badge, seat.look.emblem);
            // Long gamertags shrink to fit beside the rank.
            let room = width - 18.0 - 2.0 * h;
            let fits = room / (seat.name.chars().count().max(1) as f32 * crate::font::ASPECT);
            let size = (h - 4.0).clamp(6.0, 10.0).min(fits);
            let at = f.at(x + 12.0 + h, y + (h - size) * 0.5);
            hb.text_left(font, at, size * s, &seat.name, TEXT);
            if let Some(level) = seat.level {
                let icon = f.rect([x + width - h - 2.0, y - 1.0, x + width, y + h + 1.0]);
                rank::draw(hb, icon, level);
            }
            y += step;
        }
    }
}

/// Text ending in "..." with its dots counting up as `seconds` go by, as
/// Halo 2's progress dots did.
fn progress_dots(text: &str, seconds: f64) -> String {
    match text.strip_suffix("...") {
        Some(rest) => format!("{rest}{}", ".".repeat((seconds * 2.0) as usize % 4)),
        None => text.to_string(),
    }
}

/// The online lobby's rows: signing in until signed in, then the party's
/// choices (some its leader's alone).
fn live_rows(ctx: &Context) -> Vec<Row> {
    let Some(o) = ctx.online else {
        return vec![Row::SignIn];
    };
    if o.live.is_none() {
        return match o.failed {
            Some(live::GAMERTAG_TAKEN) => vec![Row::Name, Row::SignIn],
            Some(_) => vec![Row::SignIn],
            None => vec![Row::Connecting],
        };
    }
    let mut rows = vec![
        Row::Matchmaking,
        Row::Quickmatch,
        Row::CustomGame,
        Row::OnlinePlayers,
        Row::RecentPlayers,
        Row::Privacy,
    ];
    if o.invite().is_some() {
        rows.push(Row::AcceptInvite);
    }
    if o.partied() {
        rows.push(Row::LeaveParty);
    }
    rows
}

/// The party beside the online lobby: the leader's crown, then each
/// member's emblem, gamertag (with their splitscreen guests) and level: in
/// the ranked playlist the party searches, plays or just played, as Halo 2
/// showed new levels back in the lobby, otherwise their best.
fn draw_party(hb: &mut HudBuilder, font: usize, white: usize, f: &Frame, o: &OnlineView) {
    let Some(party) = o.party() else {
        return;
    };
    let s = f.s;
    let (x, mut y) = (PANEL_X, ROW_Y);
    let height = 30.0 + 16.0 * party.members.len() as f32;
    hb.quad(
        white,
        f.rect([x - 8.0, y - 8.0, x + PANEL_W + 8.0, y + height]),
        [0.0; 4],
        PANEL,
        hud_mode::PLAIN,
        0.0,
    );
    hb.text_left(font, f.at(x, y), 11.0 * s, "PARTY", BRIGHT);
    let count = format!("{}/{}", o.party_size(), live::MAX_PARTY);
    let width = count.len() as f32 * 9.0 * crate::font::ASPECT;
    hb.text_left(
        font,
        f.at(x + PANEL_W - width, y + 1.0),
        9.0 * s,
        &count,
        DIM,
    );
    y += 24.0;
    for m in &party.members {
        if m.account == party.leader {
            let crown = f.rect([x - 2.0, y - 3.0, x + 12.0, y + 11.0]);
            rank::draw_live(hb, crown, LiveIcon::Leader);
        }
        let badge = f.rect([x + 15.0, y - 1.0, x + 26.0, y + 10.0]);
        crate::emblem::draw(hb, badge, m.look.emblem);
        let name = match m.guests {
            0 => m.gamertag.clone(),
            n => format!("{} +{n}", m.gamertag),
        };
        let fg = if m.account == o.me() { BRIGHT } else { TEXT };
        hb.text_left(font, f.at(x + 31.0, y), 9.0 * s, &name, fg);
        let icon = f.rect([x + PANEL_W - 13.0, y - 2.0, x + PANEL_W, y + 11.0]);
        rank::draw(hb, icon, m.level);
        y += 16.0;
    }
}

/// The player picked (by account), beside what can be done about them:
/// their level, gamertag and what they're doing (or that they're offline),
/// and what we last played with them if we have lately.
fn draw_player(
    hb: &mut HudBuilder,
    font: usize,
    white: usize,
    f: &Frame,
    o: &OnlineView,
    account: u64,
) {
    let recent = o.recent_player(account);
    let (level, gamertag) = match (o.player(account), recent) {
        (Some(p), _) => (p.best, &p.gamertag),
        (None, Some(r)) => (r.level, &r.gamertag),
        (None, None) => return,
    };
    let s = f.s;
    let (x, y) = (PANEL_X, ROW_Y);
    let text_x = x + 38.0;
    let per_line = ((PANEL_X + PANEL_W - text_x) / (9.0 * crate::font::ASPECT)) as usize;
    let mut lines = wrap(&o.status(account), per_line);
    if let Some(r) = recent {
        let played = format!("LAST PLAYED {}", o.last_played(r));
        lines.extend(wrap(&played, per_line));
    }
    let height = (20.0 + 12.0 * lines.len() as f32).max(26.0);
    hb.quad(
        white,
        f.rect([x - 8.0, y - 8.0, x + PANEL_W + 8.0, y + height + 8.0]),
        [0.0; 4],
        PANEL,
        hud_mode::PLAIN,
        0.0,
    );
    rank::draw(hb, f.rect([x, y, x + 28.0, y + 26.0]), level);
    hb.text_left(font, f.at(text_x, y), 11.0 * s, gamertag, BRIGHT);
    for (k, line) in lines.iter().enumerate() {
        let at = f.at(text_x, y + 18.0 + 12.0 * k as f32);
        hb.text_left(font, at, 9.0 * s, line, TEXT);
    }
}

/// The icons on the online lists' rows: a player's level (as it was when
/// we played, for recent players offline), then their party's if they're
/// online (a crown for our party's leader); the level we have in a
/// playlist.
fn draw_icons(
    hb: &mut HudBuilder,
    f: &Frame,
    row: Row,
    rect: [f32; 4],
    label: &str,
    o: &OnlineView,
) {
    let [x0, y0, x1, _] = rect;
    let mid = y0 + ROW_H * 0.5;
    match row {
        Row::Player(_) | Row::RecentPlayer(_) => {
            let (level, p) = match row {
                Row::Player(i) => match o.others().get(i).copied() {
                    Some(p) => (p.best, Some(p)),
                    None => return,
                },
                Row::RecentPlayer(i) => match o.recent.get(i) {
                    Some(r) => {
                        let p = o.player(r.account);
                        (p.map_or(r.level, |p| p.best), p)
                    }
                    None => return,
                },
                _ => return,
            };
            let at = f.rect([x0 + 9.0, mid - 7.0, x0 + 23.0, mid + 7.0]);
            rank::draw(hb, at, level);
            let Some(p) = p else {
                return;
            };
            let leads = o.party().is_some_and(|party| party.leader == p.account);
            let icon = if leads {
                LiveIcon::Leader
            } else if p.size > 1 {
                LiveIcon::Party
            } else {
                return;
            };
            let x = x0 + 34.0 + label.chars().count() as f32 * 11.0 * crate::font::ASPECT;
            rank::draw_live(hb, f.rect([x, mid - 7.0, x + 14.0, mid + 7.0]), icon);
        }
        Row::Playlist(i) => {
            let Some(p) = o.playlists().get(i) else {
                return;
            };
            let rect = f.rect([x1 - 24.0, mid - 7.0, x1 - 10.0, mid + 7.0]);
            if !o.missing_maps(p).is_empty() {
                rank::draw_live(hb, rect, LiveIcon::Download);
            } else if p.level > 0 {
                rank::draw(hb, rect, p.level);
            }
        }
        _ => {}
    }
}

/// An armour or team colour as a HUD colour: they're in gamma space, the
/// HUD's colours linear.
fn gamma_color(c: [f32; 3]) -> [f32; 4] {
    [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2), 1.0]
}

/// Break `text` into lines of at most `width` characters, at spaces.
/// A mission's objectives, beside the pause menu: done ones dimmed.
fn draw_objectives(
    hb: &mut HudBuilder,
    font: usize,
    white: usize,
    f: &Frame,
    objectives: &[(String, bool)],
) {
    let s = f.s;
    let (x, mut y) = (PANEL_X, ROW_Y);
    let per_line = (PANEL_W / (9.0 * crate::font::ASPECT)) as usize;
    let lines: Vec<(String, bool)> = objectives
        .iter()
        .flat_map(|(text, done)| {
            let mut lines = wrap(&text.to_uppercase(), per_line.saturating_sub(2));
            for (k, l) in lines.iter_mut().enumerate() {
                *l = format!(
                    "{} {l}",
                    if k > 0 {
                        " "
                    } else if *done {
                        "+"
                    } else {
                        "-"
                    }
                );
            }
            lines.into_iter().map(move |l| (l, *done))
        })
        .collect();
    let height = 22.0 + 13.0 * lines.len() as f32;
    let back = f.rect([x - 8.0, y - 8.0, x + PANEL_W + 8.0, y + height + 4.0]);
    hb.quad(white, back, [0.0; 4], PANEL, hud_mode::PLAIN, 0.0);
    hb.text_left(font, f.at(x, y), 10.0 * s, "OBJECTIVES", BRIGHT);
    y += 22.0;
    for (line, done) in lines {
        let color = if done { DIM } else { TEXT };
        hb.text_left(font, f.at(x, y), 9.0 * s, &line, color);
        y += 13.0;
    }
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.len() + 1 + word.len() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// Most lines the scoreboard shows: 16 players and two team totals.
const MAX_SCORE_LINES: usize = 18;

/// The scoreboard, best first, its top at `top` (screen units). Places go
/// to teams when there are team totals, otherwise to players. Levels show
/// when there are any.
fn draw_scores(
    hb: &mut HudBuilder,
    font: usize,
    white: usize,
    f: &Frame,
    top: f32,
    scores: &[ScoreLine],
) {
    let s = f.s;
    let cols = [
        ROW_X + 10.0,
        ROW_X + 70.0,
        ROW_X + 290.0,
        ROW_X + 360.0,
        ROW_X + 430.0,
    ];
    // Rank icons centred under LEVEL, between the names and scores (on a
    // whole pixel at 720p, like the other columns).
    let level_x = ROW_X + 236.0;
    let levels = scores.iter().any(|l| l.level.is_some());
    let mut y = top;
    for (x, h) in cols
        .iter()
        .zip(["PLACE", "PLAYER", "SCORE", "KILLS", "DEATHS"])
    {
        hb.text_left(font, f.at(*x, y), 8.0 * s, h, DIM);
    }
    if levels {
        hb.text_left(font, f.at(level_x, y), 8.0 * s, "LEVEL", DIM);
    }
    y += 16.0;
    let teams = scores.iter().any(|l| l.header);
    let ranked: Vec<&ScoreLine> = scores.iter().filter(|l| l.header == teams).collect();
    let rank_of =
        |line: &ScoreLine| -> usize { 1 + ranked.iter().filter(|o| o.score > line.score).count() };
    for line in scores.iter().take(MAX_SCORE_LINES) {
        let c = line.color;
        let bg = if line.header {
            [c[0] * 0.6, c[1] * 0.6, c[2] * 0.6, 0.85]
        } else if line.local {
            [0.15, 0.3, 0.55, 0.75]
        } else {
            PANEL
        };
        hb.quad(
            white,
            f.rect([ROW_X, y - 2.0, ROW_X + 490.0, y + 13.0]),
            [0.0; 4],
            bg,
            hud_mode::PLAIN,
            0.0,
        );
        if !line.header {
            // Their emblem, or (without the emblem pictures) their colour.
            let badge = f.rect([cols[1] - 18.0, y - 1.0, cols[1] - 6.0, y + 11.0]);
            hb.quad(white, badge, [0.0; 4], gamma_color(c), hud_mode::PLAIN, 0.0);
            if let Some(e) = line.emblem {
                crate::emblem::draw(hb, badge, e);
            }
        }
        if let Some(level) = line.level {
            let icon = f.rect([level_x + 8.5, y - 1.0, level_x + 21.5, y + 12.0]);
            crate::rank::draw(hb, icon, level);
        }
        let fg = if line.local || line.header {
            BRIGHT
        } else {
            TEXT
        };
        if line.header == teams {
            hb.text_left(font, f.at(cols[0], y), 9.0 * s, &place(rank_of(line)), fg);
        }
        let values = [
            line.name.clone(),
            crate::local::score_text(line.score, line.timed),
            line.kills.to_string(),
            line.deaths.to_string(),
        ];
        for (x, v) in cols[1..].iter().zip(&values) {
            hb.text_left(font, f.at(*x, y), 9.0 * s, v, fg);
        }
        y += 16.0;
    }
}

/// The scoreboard on its own, over the game (held Tab / Back).
pub fn draw_scoreboard(
    hb: &mut HudBuilder,
    font: usize,
    white: usize,
    w: f32,
    h: f32,
    scores: &[ScoreLine],
) {
    let f = Frame::new(w, h);
    let rect = f.rect([
        ROW_X - 12.0,
        70.0,
        ROW_X + 502.0,
        120.0 + 16.0 * scores.len().min(MAX_SCORE_LINES) as f32,
    ]);
    hb.quad(
        white,
        rect,
        [0.0; 4],
        [0.0, 0.0, 0.02, 0.7],
        hud_mode::PLAIN,
        0.0,
    );
    draw_scores(hb, font, white, &f, 90.0, scores);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::online::recent::Recent;
    use h2live::client::View;
    use h2net::live::{
        Activity, MatchInfo, OnlinePlayer, PartyInfo, PartyMember, PlaylistInfo, SearchStatus,
        Stage, Welcome,
    };

    fn ctx<'a>(maps: &'a [MapChoice], lan: &'a [LanGame]) -> Context<'a> {
        Context {
            maps,
            missions: &[],
            lan,
            seats: Vec::leak(vec![SeatInfo {
                name: "JOHN".into(),
                how: "KEYBOARD",
                team: 0,
                look: Look::default(),
                level: None,
            }]),
            local: 1,
            scores: &[],
            outcome: None,
            joined: false,
            host_lobby: None,
            objectives: &[],
            online: None,
        }
    }

    fn maps() -> Vec<MapChoice> {
        ["lockout", "midship"]
            .iter()
            .map(|n| MapChoice::new(Path::new(&format!("maps/{n}.map"))))
            .collect()
    }

    #[test]
    fn titles_use_halo_2s_names() {
        assert_eq!(map_title("cyclotron"), "IVORY TOWER");
        assert_eq!(map_title("Lockout"), "LOCKOUT");
        assert_eq!(map_title("my_map"), "MY MAP");
        assert_eq!(place(1), "1ST");
        assert_eq!(place(12), "12TH");
        assert_eq!(place(23), "23RD");
    }

    #[test]
    fn the_lobby_changes_settings_and_starts() {
        let maps = maps();
        let c = ctx(&maps, &[]);
        let mut m = Menu::new(
            Settings {
                game_type: 0,
                map: 0,
                score: 3,
                bots: 3,
                options: GameOptions::default(),
            },
            Profile::default(),
        );
        m.input(Input::Down, &c);
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert_eq!(m.screen, Screen::Lobby);
        m.input(Input::Down, &c);
        m.input(Input::Right, &c);
        assert_eq!(m.settings.map, 1);
        m.input(Input::Right, &c);
        assert_eq!(m.settings.map, 0, "wraps around");
        m.input(Input::Down, &c);
        m.input(Input::Left, &c);
        assert_eq!(m.settings.score_to_win(), 15);
        m.input(Input::Down, &c);
        m.input(Input::Left, &c);
        assert_eq!(m.settings.bots, 2);
        // Game options: pick a variant, then change one thing.
        m.input(Input::Down, &c);
        m.input(Input::Select, &c);
        assert_eq!(m.screen, Screen::Options);
        m.input(Input::Right, &c);
        assert_eq!(m.label(Row::Variant, &c).1.as_deref(), Some("SWAT"));
        assert!(!m.settings.options.shields);
        m.input(Input::Down, &c);
        m.input(Input::Down, &c);
        m.input(Input::Right, &c);
        assert_ne!(m.settings.options.primary, presets()[1].1.primary);
        assert_eq!(m.label(Row::Variant, &c).1.as_deref(), Some("CUSTOM"));
        // The time limit, last, goes with whichever variant.
        m.input(Input::Up, &c);
        m.input(Input::Up, &c);
        m.input(Input::Up, &c);
        assert_eq!(m.label(Row::TimeLimit, &c).1.as_deref(), Some("NONE"));
        m.input(Input::Right, &c);
        m.input(Input::Right, &c);
        assert_eq!(m.settings.options.time_limit, 600);
        assert_eq!(m.label(Row::TimeLimit, &c).1.as_deref(), Some("10 MINUTES"));
        m.input(Input::Down, &c);
        m.input(Input::Right, &c);
        m.input(Input::Right, &c);
        assert_eq!(m.label(Row::Variant, &c).1.as_deref(), Some("SWAT"));
        assert_eq!(m.settings.options.time_limit, 600);
        m.input(Input::Back, &c);
        assert_eq!(m.screen, Screen::Lobby);
        // The lobby (and so PCs that join it) shows the limit too.
        assert_eq!(
            m.label(Row::GameOptions, &c).1.as_deref(),
            Some("SWAT, 10 MIN")
        );
        m.input(Input::Down, &c);
        assert_eq!(m.input(Input::Select, &c), Action::Start);
        assert_eq!(m.sound, Some(Sound::Advance));
        assert_eq!(m.input(Input::Back, &c), Action::None);
        assert_eq!(m.screen, Screen::Main);
        assert_eq!(m.rows(&c)[m.cursor], Row::Multiplayer);
        m.input(Input::Up, &c);
        assert_eq!(m.rows(&c)[m.cursor], Row::Online);
        m.input(Input::Up, &c);
        assert_eq!(m.input(Input::Select, &c), Action::Quit);
    }

    #[test]
    fn the_campaign_picks_a_difficulty_and_a_mission() {
        let maps = maps();
        let missions: Vec<MapChoice> = ["01b_spacestation", "01a_tutorial"]
            .iter()
            .map(|n| MapChoice::new(Path::new(&format!("{n}.map"))))
            .collect();
        let c = Context {
            missions: &missions,
            ..ctx(&maps, &[])
        };
        let mut m = Menu::new(Settings::default(), Profile::default());
        assert!(!m.rows(&c).iter().any(|r| matches!(r, Row::Difficulty)));
        m.show(Screen::Campaign);
        assert_eq!(m.label(Row::Difficulty, &c).1.as_deref(), Some("NORMAL"));
        m.input(Input::Right, &c);
        m.input(Input::Right, &c);
        assert_eq!(m.label(Row::Difficulty, &c).1.as_deref(), Some("LEGENDARY"));
        m.input(Input::Down, &c);
        m.input(Input::Down, &c);
        assert_eq!(m.label(Row::Mission(1), &c).0, "THE ARMORY");
        assert_eq!(m.input(Input::Select, &c), Action::Mission(1));
        m.input(Input::Back, &c);
        assert_eq!(m.screen, Screen::Main);
        // No folder, no missions.
        let dir = std::env::temp_dir();
        assert!(find_missions(&dir.join("no such folder")).is_empty());
    }

    #[test]
    fn system_link_lists_games_to_join() {
        let maps = maps();
        let mut m = Menu::new(
            Settings {
                game_type: 0,
                map: 0,
                score: 3,
                bots: 3,
                options: GameOptions::default(),
            },
            Profile::default(),
        );
        m.show(Screen::SystemLink);
        // Nothing found yet: nothing to choose.
        assert_eq!(m.input(Input::Select, &ctx(&maps, &[])), Action::None);
        let game = LanGame::at("10.0.0.2:4000".parse().unwrap(), "midship");
        let lan = [game.clone()];
        assert_eq!(
            m.input(Input::Select, &ctx(&maps, &lan)),
            Action::Join(game)
        );
        // A game of another version shows, but says so rather than joining.
        let mut other = LanGame::at("10.0.0.3:4000".parse().unwrap(), "midship");
        other.protocol = h2net::PROTOCOL - 1;
        let lan = [other];
        assert_eq!(m.input(Input::Select, &ctx(&maps, &lan)), Action::None);
        assert_eq!(
            m.notice.as_deref(),
            Some("10.0.0.3 RUNS ANOTHER VERSION, UPDATE BOTH PCS")
        );
    }

    #[test]
    fn the_profile_picks_a_model_colours_and_a_gamertag() {
        let maps = maps();
        let c = ctx(&maps, &[]);
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::Profile);
        // Model: Spartan to Elite.
        m.input(Input::Down, &c);
        assert_eq!(m.input(Input::Right, &c), Action::SaveProfile);
        assert!(m.profile.look.elite);
        // Primary colour steps through the 18, wrapping around.
        m.input(Input::Down, &c);
        m.profile.look.colors[0] = 0;
        assert_eq!(m.input(Input::Left, &c), Action::SaveProfile);
        assert_eq!(m.profile.look.colors[0], PROFILE_COLORS - 1);
        // The gamertag is typed, capitalised and kept to 15 characters.
        m.input(Input::Up, &c);
        m.input(Input::Up, &c);
        m.input(Input::Select, &c);
        assert!(m.editing);
        for _ in 0..30 {
            m.typed(Typed::Erase);
        }
        m.typed(Typed::Text("the arbiter of sanghelios".into()));
        assert_eq!(m.profile.name.len(), MAX_NAME);
        // Arrows do nothing while typing; Enter keeps the name.
        assert_eq!(m.input(Input::Down, &c), Action::None);
        assert_eq!(m.typed(Typed::Done), Action::SaveProfile);
        assert!(!m.editing);
        assert_eq!(m.profile.name, "THE ARBITER OF");
        // Back returns to the main menu, on PLAYER PROFILE.
        m.input(Input::Back, &c);
        assert_eq!(m.screen, Screen::Main);
        assert_eq!(m.rows(&c)[m.cursor], Row::Profile);
    }

    #[test]
    fn the_profile_sets_look_sensitivity_and_inversion() {
        let maps = maps();
        let c = ctx(&maps, &[]);
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::Profile);
        let to = |m: &mut Menu, row: Row| {
            while m.rows(&c)[m.cursor] != row {
                m.input(Input::Down, &c);
            }
        };
        // They have a screen of their own, off the profile.
        to(&mut m, Row::Controls);
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert_eq!(m.screen, Screen::Controls);
        // Halo 2's look sensitivity of 3 to start, from 1 to 10.
        to(&mut m, Row::LookSensitivity);
        assert_eq!(m.profile.controls.look_sensitivity, 3);
        assert_eq!(m.input(Input::Right, &c), Action::SaveProfile);
        assert_eq!(m.profile.controls.look_sensitivity, 4);
        for _ in 0..3 {
            m.input(Input::Left, &c);
        }
        assert_eq!(m.profile.controls.look_sensitivity, 1);
        m.input(Input::Left, &c);
        assert_eq!(m.profile.controls.look_sensitivity, 10);
        assert_eq!(m.label(Row::LookSensitivity, &c).1.as_deref(), Some("10"));
        to(&mut m, Row::MouseSensitivity);
        m.input(Input::Right, &c);
        assert_eq!(m.profile.controls.mouse_sensitivity, 4);
        to(&mut m, Row::InvertLook);
        assert_eq!(m.input(Input::Select, &c), Action::SaveProfile);
        assert!(m.profile.controls.invert_look);
        // Back to the profile, on CONTROLS.
        m.input(Input::Back, &c);
        assert_eq!(m.screen, Screen::Profile);
        assert_eq!(m.rows(&c)[m.cursor], Row::Controls);
    }

    #[test]
    fn every_screens_rows_end_above_the_hint() {
        let maps = maps();
        let c = ctx(&maps, &[]);
        let mut m = Menu::new(Settings::default(), Profile::default());
        let screens = [
            Screen::Main,
            Screen::Lobby,
            Screen::Options,
            Screen::Profile,
            Screen::Controls,
            Screen::Pause,
            Screen::PostGame,
            Screen::Live,
            Screen::Player,
            Screen::Matchmaking,
            Screen::Pregame,
        ];
        for screen in screens {
            m.show(screen);
            assert!(!m.lists(), "{screen:?} scrolls");
            // Signed in, the online lobby has as many as eight.
            let n = match screen {
                Screen::Live => 8,
                _ => m.rows(&c).len(),
            };
            let bottom = m.row_rect(n - 1)[3];
            assert!(bottom < HINT_Y, "{screen:?}'s {n} rows end at {bottom}");
        }
        // The lists show as many as fit, and scroll.
        for screen in [
            Screen::SystemLink,
            Screen::Players,
            Screen::RecentPlayers,
            Screen::Playlists,
        ] {
            m.show(screen);
            assert!(m.lists());
            // Their place in the list goes under the last row shown.
            let bottom = m.row_rect(MAX_LIST_ROWS)[1] + 9.0;
            assert!(bottom < HINT_Y, "{screen:?}: {bottom}");
        }
    }

    #[test]
    fn rows_are_found_under_the_mouse() {
        let maps = maps();
        let c = ctx(&maps, &[]);
        let mut m = Menu::new(
            Settings {
                game_type: 0,
                map: 0,
                score: 3,
                bots: 3,
                options: GameOptions::default(),
            },
            Profile::default(),
        );
        m.show(Screen::Lobby);
        // 1280x720: 1.5 pixels per unit, the 640 wide screen centred.
        let f = Frame::new(1280.0, 720.0);
        let [x0, y0, x1, y1] = f.rect(m.row_rect(1));
        let middle = [(x0 + x1) * 0.5, (y0 + y1) * 0.5];
        m.hover(middle, 1280.0, 720.0, &c);
        assert_eq!(m.cursor, 1);
        // The right part of a setting steps it up.
        m.click([x1 - 4.0, middle[1]], 1280.0, 720.0, &c);
        assert_eq!(m.settings.map, 1);
        m.click([x0 + 4.0, middle[1]], 1280.0, 720.0, &c);
        assert_eq!(m.settings.map, 0);
        assert_eq!(m.click([1.0, 1.0], 1280.0, 720.0, &c), Action::None);
    }

    #[test]
    fn a_hosts_lobby_shows_its_choices_and_only_leaves() {
        let maps = maps();
        let lobby = Lobby {
            map: "midship".into(),
            game_type: "TEAM SLAYER".into(),
            score: "50".into(),
            options: "SWAT".into(),
            teams: true,
            players: Vec::new(),
            bots: 3,
        };
        let c = Context {
            joined: true,
            host_lobby: Some(&lobby),
            ..ctx(&maps, &[])
        };
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::Lobby);
        assert_eq!(
            m.label(Row::Map, &c),
            ("MAP".into(), Some("MIDSHIP".into()))
        );
        assert_eq!(
            m.label(Row::GameOptions, &c),
            ("GAME OPTIONS".into(), Some("SWAT".into()))
        );
        assert!(!m.rows(&c).contains(&Row::StartGame));
        assert_eq!(m.input(Input::Right, &c), Action::None);
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert_eq!(m.settings.bots, 0);
        assert_eq!(m.input(Input::Back, &c), Action::Leave);
        // Joined, the carnage report waits for the host.
        m.show(Screen::PostGame);
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert!(m.notice.is_some());
    }

    #[test]
    fn rank_icons_show_only_for_known_levels() {
        // Rank icons drawn, counted by their quads.
        let icons = |hb: HudBuilder| -> usize {
            hb.finish()
                .iter()
                .filter(|b| b.texture >= crate::gpu::RANK_TEXTURES)
                .map(|b| b.vertices.len() / 6)
                .sum()
        };
        let line = |level| ScoreLine {
            name: "JOHN".into(),
            score: 3,
            timed: false,
            kills: 3,
            deaths: 1,
            color: [1.0; 3],
            emblem: None,
            level,
            local: false,
            header: false,
        };
        let scoreboard = |scores: &[ScoreLine]| {
            let mut hb = HudBuilder::new(1280.0, 720.0);
            draw_scoreboard(&mut hb, 0, 1, 1280.0, 720.0, scores);
            icons(hb)
        };
        assert_eq!(scoreboard(&[line(None), line(None)]), 0);
        assert_eq!(scoreboard(&[line(Some(7)), line(None)]), 1);
        // The lobby's list.
        let maps = maps();
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::Lobby);
        let lobby = |c: &Context| {
            let mut hb = HudBuilder::new(1280.0, 720.0);
            m.draw(&mut hb, 0, 1, 1280.0, 720.0, c);
            icons(hb)
        };
        assert_eq!(lobby(&ctx(&maps, &[])), 0);
        let seats = [SeatInfo {
            name: "JOHN".into(),
            how: "KEYBOARD",
            team: 0,
            look: Look::default(),
            level: Some(50),
        }];
        let c = Context {
            seats: &seats,
            ..ctx(&maps, &[])
        };
        assert_eq!(lobby(&c), 1);
    }

    /// Signed in as JOHN (account 1) in party 10, led by `leader`, with
    /// `members`. SARGE (2) and VIPER (3) are online in party 20, which
    /// is open and invited us; and Double Team (ranked, parties up to 2)
    /// and Big Team Battle (guests welcome) are searchable.
    fn signed_in(leader: u64, members: &[(u64, &str)]) -> View {
        let member = |&(account, gamertag): &(u64, &str)| PartyMember {
            account,
            gamertag: gamertag.into(),
            look: Look::default(),
            best: 7,
            level: 7,
            guests: 0,
        };
        let player = |account: u64, gamertag: &str, party: u64| OnlinePlayer {
            account,
            gamertag: gamertag.into(),
            look: Look::default(),
            best: 12,
            activity: Activity::Lobby,
            party,
            open: party == 20,
            size: 2,
            openings: 14,
        };
        let playlist = |id, name: &str, party_max, level| PlaylistInfo {
            id,
            key: name.to_lowercase().replace(' ', "_"),
            name: name.into(),
            ranked: level > 0,
            teams: true,
            guests: level == 0,
            min: 2,
            max: 16,
            party_max,
            searching: 3,
            playing: 8,
            level,
            maps: Vec::new(),
        };
        View {
            welcome: Some(Welcome {
                account: 1,
                gamertag: "JOHN".into(),
                card: String::new(),
                best: 7,
                levels: Vec::new(),
            }),
            party: Some(PartyInfo {
                id: 10,
                leader,
                privacy: Privacy::Open,
                activity: Activity::Lobby,
                playlist: live::QUICKMATCH,
                members: members.iter().map(member).collect(),
                maps: Vec::new(),
            }),
            online: vec![
                player(1, "JOHN", 10),
                player(2, "SARGE", 20),
                player(3, "VIPER", 20),
            ],
            invites: vec![(20, "SARGE".into())],
            playlists: vec![
                playlist(2, "DOUBLE TEAM", 2, 7),
                playlist(5, "BIG TEAM BATTLE", 8, 0),
            ],
            ..View::default()
        }
    }

    /// The online screens' view of `live`.
    fn online(live: Option<&View>) -> OnlineView<'_> {
        OnlineView {
            live,
            failed: None,
            waking: false,
            text: Box::leak(Box::default()),
            search: None,
            game: None,
            countdown: None,
            returning: None,
            custom: false,
            maps: &[],
            recent: &[],
            time: 0,
        }
    }

    #[test]
    fn online_signs_in_and_out() {
        let maps = maps();
        let c = ctx(&maps, &[]);
        let mut m = Menu::new(Settings::default(), Profile::default());
        // ONLINE comes first.
        assert_eq!(m.input(Input::Select, &c), Action::GoOnline);
        assert_eq!(m.screen, Screen::Live);
        // Connecting: nothing to choose.
        let connecting = online(None);
        let c = Context {
            online: Some(&connecting),
            ..ctx(&maps, &[])
        };
        assert_eq!(m.rows(&c), [Row::Connecting]);
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert_eq!(m.label(Row::Connecting, &c).0, "CONNECTING...");
        let waking = OnlineView {
            waking: true,
            ..online(None)
        };
        let c = Context {
            online: Some(&waking),
            ..ctx(&maps, &[])
        };
        assert!(m.label(Row::Connecting, &c).0.starts_with("WAKING UP"));
        // Turned away: SIGN IN again.
        let failed = OnlineView {
            failed: Some("SERVER FULL"),
            ..online(None)
        };
        let c = Context {
            online: Some(&failed),
            ..ctx(&maps, &[])
        };
        assert_eq!(m.input(Input::Select, &c), Action::GoOnline);
        // Our gamertag is taken: type another, then sign in.
        let taken = OnlineView {
            failed: Some(live::GAMERTAG_TAKEN),
            ..online(None)
        };
        let c = Context {
            online: Some(&taken),
            ..ctx(&maps, &[])
        };
        m.ask_gamertag("TAKEN");
        assert!(m.editing);
        assert_eq!(m.rows(&c), [Row::Name, Row::SignIn]);
        m.typed(Typed::Erase);
        m.typed(Typed::Text("2".into()));
        assert_eq!(m.typed(Typed::Done), Action::SaveProfile);
        assert!(m.profile.name.ends_with('2'));
        assert_eq!(m.input(Input::Select, &c), Action::GoOnline);
        let view = signed_in(1, &[(1, "JOHN")]);
        let o = online(Some(&view));
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        assert_eq!(m.rows(&c)[m.cursor], Row::Matchmaking, "once signed in");
        // Back signs out, onto ONLINE, leaving notices behind.
        m.notice = Some("SERVER FULL".into());
        assert_eq!(m.input(Input::Back, &c), Action::SignOut);
        assert_eq!(m.notice, None);
        assert_eq!(m.screen, Screen::Main);
        assert_eq!(m.rows(&c)[m.cursor], Row::Online);
    }

    #[test]
    fn custom_games_are_played_on_the_partys_maps() {
        let maps: Vec<MapChoice> = ["lockout", "midship", "zanzibar"]
            .iter()
            .map(|n| MapChoice::new(Path::new(&format!("maps/{n}.map"))))
            .collect();
        let mut view = signed_in(1, &[(1, "JOHN"), (2, "SARGE")]);
        let party = view.party.as_mut().unwrap();
        party.activity = Activity::Custom;
        party.maps = vec!["zanzibar".into(), "lockout".into()];
        let o = OnlineView {
            custom: true,
            ..online(Some(&view))
        };
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        // The leader's lobby, with only the maps everyone has.
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::Lobby);
        assert_eq!(m.title(&c), "CUSTOM GAME");
        m.input(Input::Down, &c);
        m.input(Input::Right, &c);
        assert_eq!(m.settings.map, 2, "midship isn't everyone's");
        m.input(Input::Right, &c);
        assert_eq!(m.settings.map, 0);
        m.input(Input::Left, &c);
        assert_eq!(m.settings.map, 2);
        // Backing out leaves it, for the party lobby.
        assert_eq!(m.input(Input::Back, &c), Action::Leave);
        assert_eq!(m.screen, Screen::Live);
        assert_eq!(m.rows(&c)[m.cursor], Row::CustomGame);
        // A member who left can go back in while it's on.
        let mut view = signed_in(2, &[(2, "SARGE"), (1, "JOHN")]);
        let o = online(Some(&view));
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        assert!(!Row::CustomGame.selectable(&c));
        view.party.as_mut().unwrap().activity = Activity::Custom;
        let o = online(Some(&view));
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        assert!(Row::CustomGame.selectable(&c));
        assert_eq!(m.input(Input::Select, &c), Action::Custom);
    }

    #[test]
    fn the_party_lobby_is_its_leaders_to_change() {
        let maps = maps();
        let view = signed_in(1, &[(1, "JOHN")]);
        let o = online(Some(&view));
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::Live);
        assert_eq!(
            m.rows(&c),
            [
                Row::Matchmaking,
                Row::Quickmatch,
                Row::CustomGame,
                Row::OnlinePlayers,
                Row::RecentPlayers,
                Row::Privacy,
                Row::AcceptInvite,
            ],
            "alone, there's no party to leave"
        );
        assert_eq!(m.header(&c).as_deref(), Some("SIGNED IN AS JOHN"));
        assert_eq!(m.label(Row::OnlinePlayers, &c).1.as_deref(), Some("2"));
        // QUICKMATCH searches whatever fits, at once.
        m.input(Input::Down, &c);
        assert_eq!(m.input(Input::Select, &c), Action::Search(None));
        assert_eq!(m.screen, Screen::Matchmaking);
        m.show(Screen::Live);
        // The party's privacy steps between open and invite only.
        for _ in 0..5 {
            m.input(Input::Down, &c);
        }
        assert_eq!(m.label(Row::Privacy, &c).1.as_deref(), Some("OPEN"));
        assert_eq!(
            m.input(Input::Right, &c),
            Action::Privacy(Privacy::InviteOnly)
        );
        // An invite: join SARGE's party.
        m.input(Input::Down, &c);
        assert_eq!(m.label(Row::AcceptInvite, &c).0, "JOIN SARGE'S PARTY");
        assert_eq!(m.input(Input::Select, &c), Action::Accept(20));
        // In SARGE's party, the choices are SARGE's.
        let view = signed_in(2, &[(2, "SARGE"), (1, "JOHN")]);
        let o = online(Some(&view));
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        assert_eq!(m.rows(&c)[m.cursor], Row::OnlinePlayers);
        assert!(m.rows(&c).contains(&Row::LeaveParty));
        // They're read only: the cursor passes them by.
        m.input(Input::Down, &c);
        m.input(Input::Down, &c);
        assert_eq!(m.rows(&c)[m.cursor], Row::AcceptInvite);
        m.input(Input::Up, &c);
        m.input(Input::Up, &c);
        assert_eq!(m.rows(&c)[m.cursor], Row::OnlinePlayers);
        assert_eq!(m.input(Input::Right, &c), Action::None);
        m.input(Input::Up, &c);
        assert_eq!(m.rows(&c)[m.cursor], Row::LeaveParty);
        assert_eq!(m.input(Input::Select, &c), Action::LeaveParty);
        assert_eq!(m.header(&c).as_deref(), Some("SARGE IS THE PARTY LEADER"));
    }

    #[test]
    fn the_online_list_invites_and_joins() {
        let maps = maps();
        let view = signed_in(1, &[(1, "JOHN")]);
        let o = online(Some(&view));
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::Live);
        m.input(Input::Down, &c);
        m.input(Input::Down, &c);
        m.input(Input::Down, &c);
        m.input(Input::Select, &c);
        assert_eq!(m.screen, Screen::Players);
        // Everyone else, with what they're doing.
        assert_eq!(m.rows(&c), [Row::Player(0), Row::Player(1)]);
        let (name, doing) = m.label(Row::Player(1), &c);
        assert_eq!(name, "VIPER");
        assert_eq!(doing.as_deref(), Some("IN A PARTY, 14 OPENINGS"));
        // VIPER: invite them, or join their open party.
        m.input(Input::Down, &c);
        m.input(Input::Select, &c);
        assert_eq!(m.screen, Screen::Player);
        assert_eq!(m.rows(&c), [Row::Invite, Row::JoinTheirParty]);
        assert_eq!(m.input(Input::Select, &c), Action::Invite(3));
        assert_eq!(m.screen, Screen::Players);
        assert_eq!(m.rows(&c)[m.cursor], Row::Player(1));
        assert_eq!(m.notice.as_deref(), Some("PARTY INVITE SENT TO VIPER"));
        m.input(Input::Select, &c);
        m.input(Input::Down, &c);
        assert_eq!(m.input(Input::Select, &c), Action::JoinParty(20));
        assert_eq!(m.screen, Screen::Live);
        // Leading SARGE and VIPER, we can hand over the lead or remove one.
        let mut view = signed_in(1, &[(1, "JOHN"), (2, "SARGE"), (3, "VIPER")]);
        for p in &mut view.online {
            p.party = 10;
        }
        let o = online(Some(&view));
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        m.show(Screen::Players);
        assert_eq!(
            m.label(Row::Player(0), &c).1.as_deref(),
            Some("IN YOUR PARTY")
        );
        m.input(Input::Select, &c);
        assert_eq!(m.rows(&c), [Row::MakeLeader, Row::Remove]);
        assert_eq!(m.input(Input::Select, &c), Action::Promote(2));
        m.input(Input::Select, &c);
        m.input(Input::Down, &c);
        assert_eq!(m.input(Input::Select, &c), Action::Kick(2));
        // Back goes up a screen at a time.
        m.input(Input::Back, &c);
        assert_eq!(m.screen, Screen::Live);
        assert_eq!(m.rows(&c)[m.cursor], Row::OnlinePlayers);
    }

    #[test]
    fn recent_players_are_listed_newest_first_with_what_they_do_now() {
        let maps = maps();
        let mut view = signed_in(1, &[(1, "JOHN")]);
        // VIPER's party is playing a match now.
        view.online[2].activity = Activity::Playing;
        let now = 1_000_000;
        let recent = |account, gamertag: &str, played: &str, map: &str, ago| Recent {
            account,
            gamertag: gamertag.into(),
            level: 4,
            played: played.into(),
            map: map.into(),
            when: now - ago,
        };
        // SARGE has a new gamertag since; GHOST is offline.
        let recent = [
            recent(2, "OLD SARGE", "double_team", "lockout", 300),
            recent(9, "GHOST", "custom", "midship", 2 * 86400),
            recent(3, "VIPER", "team_snipers", "cyclotron", 7200),
        ];
        let o = OnlineView {
            recent: &recent,
            time: now,
            ..online(Some(&view))
        };
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        // Beside ONLINE PLAYERS in the party lobby.
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::Live);
        assert_eq!(m.label(Row::RecentPlayers, &c).1.as_deref(), Some("3"));
        for _ in 0..4 {
            m.input(Input::Down, &c);
        }
        m.input(Input::Select, &c);
        assert_eq!(m.screen, Screen::RecentPlayers);
        assert_eq!(m.title(&c), "RECENT PLAYERS");
        assert_eq!(
            m.rows(&c),
            [
                Row::RecentPlayer(0),
                Row::RecentPlayer(1),
                Row::RecentPlayer(2)
            ]
        );
        let label = |m: &Menu, k| m.label(Row::RecentPlayer(k), &c);
        assert_eq!(
            label(&m, 0),
            ("SARGE".into(), Some("IN A PARTY, 14 OPENINGS".into()))
        );
        assert_eq!(label(&m, 1), ("GHOST".into(), Some("OFFLINE".into())));
        // What we played with them, where and when.
        assert_eq!(
            m.note(&c).as_deref(),
            Some("DOUBLE TEAM ON LOCKOUT, 5 MINUTES AGO")
        );
        m.input(Input::Down, &c);
        assert_eq!(
            m.note(&c).as_deref(),
            Some("CUSTOM GAME ON MIDSHIP, 2 DAYS AGO")
        );
        m.input(Input::Down, &c);
        assert_eq!(
            m.note(&c).as_deref(),
            Some("TEAM SNIPERS ON IVORY TOWER, 2 HOURS AGO")
        );
        // Each one's level, and the parties of those online.
        let icons = |m: &Menu| {
            let mut hb = HudBuilder::new(1280.0, 720.0);
            m.draw(&mut hb, 0, 1, 1280.0, 720.0, &c);
            let ranks = |t| t >= crate::gpu::RANK_TEXTURES;
            quads(&hb.finish(), ranks).len()
        };
        assert_eq!(icons(&m), 5);
        // SARGE: invite them, or join their open party, then back here.
        m.input(Input::Up, &c);
        m.input(Input::Up, &c);
        m.input(Input::Select, &c);
        assert_eq!(m.screen, Screen::Player);
        assert_eq!(m.rows(&c), [Row::Invite, Row::JoinTheirParty]);
        assert_eq!(m.input(Input::Select, &c), Action::Invite(2));
        assert_eq!(m.screen, Screen::RecentPlayers);
        assert_eq!(m.rows(&c)[m.cursor], Row::RecentPlayer(0));
        assert_eq!(m.notice.as_deref(), Some("PARTY INVITE SENT TO SARGE"));
        // GHOST is offline: nothing to do but see when we played.
        m.input(Input::Down, &c);
        m.input(Input::Select, &c);
        assert_eq!(m.rows(&c), [Row::Nothing]);
        assert_eq!(m.label(Row::Nothing, &c).0, "OFFLINE");
        assert_eq!(icons(&m), 1, "their level as it was");
        m.input(Input::Back, &c);
        assert_eq!(m.screen, Screen::RecentPlayers);
        assert_eq!(m.rows(&c)[m.cursor], Row::RecentPlayer(1));
        // VIPER is in a match: no joining their party until it's over.
        m.input(Input::Down, &c);
        m.input(Input::Select, &c);
        assert_eq!(m.rows(&c), [Row::Invite]);
        // Back goes up a screen at a time.
        m.input(Input::Back, &c);
        m.input(Input::Back, &c);
        assert_eq!(m.screen, Screen::Live);
        assert_eq!(m.rows(&c)[m.cursor], Row::RecentPlayers);
        // No one yet.
        let o = online(Some(&view));
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        m.show(Screen::RecentPlayers);
        assert_eq!(m.rows(&c), [Row::Nothing]);
        assert_eq!(m.label(Row::Nothing, &c).0, "NO RECENT PLAYERS YET");
    }

    #[test]
    fn long_lists_scroll() {
        let maps = maps();
        let mut view = signed_in(1, &[(1, "JOHN")]);
        for k in 0..13 {
            let mut p = view.online[2].clone();
            p.account = 100 + k;
            p.gamertag = format!("PLAYER {k}");
            view.online.push(p);
        }
        let o = online(Some(&view));
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::Players);
        assert_eq!(m.rows(&c).len(), 15);
        assert_eq!(m.shown(15), 0..8);
        // Down past the eighth scrolls, and the tenth can be chosen.
        for _ in 0..9 {
            m.input(Input::Down, &c);
        }
        assert_eq!(m.shown(15), 2..10);
        m.input(Input::Select, &c);
        assert_eq!(m.screen, Screen::Player);
        assert_eq!(m.player, o.others()[9].account);
        // Up from the top goes round to the end of the list.
        m.show(Screen::Players);
        m.input(Input::Up, &c);
        assert_eq!(m.shown(15), 7..15);
        // The mouse finds the rows where they're shown: the top one is
        // the eighth.
        m.hover([300.0, 200.0], 1280.0, 720.0, &c);
        assert_eq!(m.cursor, 7);
        m.wheel(true, &c);
        assert_eq!((m.cursor, m.shown(15)), (6, 6..14));
        // Short lists don't scroll.
        m.show(Screen::Live);
        assert_eq!(m.shown(6), 0..6);
    }

    #[test]
    fn playlists_are_searched_until_cancelled() {
        let maps = maps();
        let mut view = signed_in(1, &[(1, "JOHN"), (2, "SARGE"), (3, "VIPER")]);
        let o = online(Some(&view));
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::Live);
        m.input(Input::Select, &c);
        assert_eq!(m.screen, Screen::Playlists);
        assert_eq!(
            m.label(Row::Playlist(0), &c),
            ("DOUBLE TEAM".into(), Some("3 SEARCHING, 8 PLAYING".into()))
        );
        // A party of three is too big for Double Team.
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert!(m.notice.is_some());
        m.input(Input::Down, &c);
        assert_eq!(m.input(Input::Select, &c), Action::Search(Some(5)));
        assert_eq!(m.screen, Screen::Matchmaking);
        // How it's going.
        let status = |stage, seconds| SearchStatus {
            stage,
            have: 6,
            need: 1,
            seconds,
            low: 0,
            high: 0,
        };
        view.status = Some(status(Stage::WaitingToFill, 20));
        let o = OnlineView {
            search: Some((5, 75.0)),
            ..online(Some(&view))
        };
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        assert_eq!(m.header(&c).as_deref(), Some("BIG TEAM BATTLE"));
        assert_eq!(
            m.rows(&c),
            [Row::SearchStatus, Row::SearchDetail, Row::Cancel]
        );
        assert_eq!(
            m.label(Row::SearchStatus, &c),
            ("STARTING IN 00:20".into(), Some("1:15".into()))
        );
        assert_eq!(m.label(Row::SearchDetail, &c).0, "PLAYERS: 6");
        // Searching, the dots count up.
        let searching = View {
            status: None,
            ..view.clone()
        };
        let dots = |seconds| {
            let o = OnlineView {
                search: Some((5, seconds)),
                ..online(Some(&searching))
            };
            let c = Context {
                online: Some(&o),
                ..ctx(&maps, &[])
            };
            m.label(Row::SearchStatus, &c).0
        };
        assert_eq!(dots(76.0), "SEARCHING FOR A GAME");
        assert_eq!(dots(76.5), "SEARCHING FOR A GAME.");
        assert_eq!(dots(77.5), "SEARCHING FOR A GAME...");
        // CANCEL is the one row to choose.
        assert_eq!(m.input(Input::Select, &c), Action::CancelSearch);
        assert_eq!(m.screen, Screen::Live);
        // A member sees the search too, but only its leader cancels it.
        view.status = Some(status(Stage::Balancing, 0));
        view.party.as_mut().unwrap().leader = 2;
        let o = online(Some(&view));
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        m.show(Screen::Matchmaking);
        let detail = m.label(Row::SearchDetail, &c).0;
        assert_eq!(detail, "1 MORE PLAYERS NEEDED FOR EVEN TEAMS");
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert_eq!(m.input(Input::Back, &c), Action::None);
        assert_eq!(m.screen, Screen::Live);
    }

    #[test]
    fn playlists_the_party_lacks_maps_for_are_marked_and_not_searched() {
        let maps = maps();
        // Alone, with Lockout and Midship: Double Team plays Warlock too.
        let mut view = signed_in(1, &[(1, "JOHN")]);
        let ours: Vec<String> = vec!["lockout".into(), "midship".into()];
        view.party.as_mut().unwrap().maps = ours.clone();
        view.playlists[0].maps = vec!["lockout".into(), "warlock".into()];
        view.playlists[1].maps = ours.clone();
        let o = OnlineView {
            maps: &ours,
            ..online(Some(&view))
        };
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::Playlists);
        assert_eq!(o.missing_maps(&view.playlists[0]), ["warlock"]);
        assert!(o.missing_maps(&view.playlists[1]).is_empty());
        // Marked with a download arrow where our level would be, and why.
        let download = |m: &Menu, c: &Context| {
            let mut hb = HudBuilder::new(1280.0, 720.0);
            m.draw(&mut hb, 0, 1, 1280.0, 720.0, c);
            let live = |t| t == crate::gpu::RANK_TEXTURES + 2;
            quads(&hb.finish(), live).len()
        };
        assert_eq!(download(&m, &c), 1);
        let why = "YOU'RE MISSING MAPS: WARLOCK";
        assert_eq!(m.note(&c).as_deref(), Some(why));
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert_eq!(m.notice.as_deref(), Some(why));
        assert_eq!(m.screen, Screen::Playlists);
        // Big Team Battle plays only what we have.
        m.input(Input::Down, &c);
        assert_eq!(m.input(Input::Select, &c), Action::Search(Some(5)));

        // Many missing are summed up, ours first. Once we have them all,
        // someone else in the party doesn't (or not the same copy).
        let mut ours = ours.clone();
        ours.push("warlock".into());
        view.playlists[0].maps = ["warlock", "cyclotron", "deltatap", "gemini", "backwash"]
            .map(String::from)
            .to_vec();
        let o = OnlineView {
            maps: &ours,
            ..online(Some(&view))
        };
        assert_eq!(
            o.cant_search(&view.playlists[0]).as_deref(),
            Some("YOU'RE MISSING MAPS: IVORY TOWER, SANCTUARY, GEMINI AND 1 MORE")
        );
        ours.extend(["cyclotron", "deltatap", "gemini", "backwash"].map(String::from));
        let o = OnlineView {
            maps: &ours,
            ..online(Some(&view))
        };
        assert_eq!(
            o.cant_search(&view.playlists[0]).as_deref(),
            Some(
                "SOMEONE IN YOUR PARTY IS MISSING MAPS: WARLOCK, IVORY TOWER, SANCTUARY AND 2 MORE"
            )
        );
    }

    #[test]
    fn online_lists_show_levels_and_parties() {
        // Icons drawn from the rank atlases, counted by their quads.
        let icons = |hb: HudBuilder| -> usize {
            hb.finish()
                .iter()
                .filter(|b| b.texture >= crate::gpu::RANK_TEXTURES)
                .map(|b| b.vertices.len() / 6)
                .sum()
        };
        let maps = maps();
        let view = signed_in(2, &[(2, "SARGE"), (1, "JOHN")]);
        let o = online(Some(&view));
        let c = Context {
            online: Some(&o),
            ..ctx(&maps, &[])
        };
        let draw = |screen| {
            let mut m = Menu::new(Settings::default(), Profile::default());
            m.show(screen);
            let mut hb = HudBuilder::new(1280.0, 720.0);
            m.draw(&mut hb, 0, 1, 1280.0, 720.0, &c);
            icons(hb)
        };
        // The party: a crown and two levels.
        assert_eq!(draw(Screen::Live), 3);
        // Players: each one's level and party (a crown for our leader).
        assert_eq!(draw(Screen::Players), 4);
        // Playlists: our level in the ranked one.
        assert_eq!(draw(Screen::Playlists), 1);
    }

    #[test]
    fn the_party_shows_levels_in_the_playlist_it_played() {
        // The level icon the party shows for us, by its place in the rank
        // atlases, back from Double Team: our best level, and in it.
        let maps = maps();
        let drawn = |best: u8, level: u8| {
            let mut view = signed_in(1, &[(1, "JOHN")]);
            let party = view.party.as_mut().unwrap();
            party.playlist = 2;
            (party.members[0].best, party.members[0].level) = (best, level);
            let o = online(Some(&view));
            let c = Context {
                online: Some(&o),
                ..ctx(&maps, &[])
            };
            let mut m = Menu::new(Settings::default(), Profile::default());
            m.show(Screen::Live);
            let mut hb = HudBuilder::new(1280.0, 720.0);
            m.draw(&mut hb, 0, 1, 1280.0, 720.0, &c);
            let ranks = |t| t == crate::gpu::RANK_TEXTURES || t == crate::gpu::RANK_TEXTURES + 1;
            let batches = hb.finish();
            let icons = batches.into_iter().filter(|b| ranks(b.texture));
            icons
                .flat_map(|b| b.vertices.into_iter().map(|v| v.uv))
                .collect::<Vec<_>>()
        };
        // Level 12 in Double Team, though our best is 30.
        assert_eq!(drawn(30, 12), drawn(12, 12));
        assert_ne!(drawn(30, 12), drawn(30, 30));
    }

    /// The quads drawn with `texture`, as rectangles (pixels).
    fn quads(batches: &[crate::gpu::HudBatch], texture: impl Fn(usize) -> bool) -> Vec<[f32; 4]> {
        let corners = batches.iter().filter(|b| texture(b.texture));
        corners
            .flat_map(|b| b.vertices.chunks(6))
            .map(|v| {
                [
                    v[0].position[0],
                    v[0].position[1],
                    v[2].position[0],
                    v[2].position[1],
                ]
            })
            .collect()
    }

    #[test]
    fn a_big_matchs_pregame_lists_everyone_readably() {
        let mut maps = maps();
        maps[0].picture = Some(0);
        let view = signed_in(1, &[(1, "JOHN")]);
        let info = MatchInfo {
            id: 7,
            playlist: 2,
            ranked: true,
            map: "lockout".into(),
            hash: 0,
            game_type: GameType::TeamSlayer,
            preset: "DEFAULT".into(),
            score: 50,
            time_limit: 600,
            bots: 0,
            host: 1,
            countdown: 20,
            players: Vec::new(),
        };
        let lobby = Lobby {
            map: "lockout".into(),
            game_type: "TEAM SLAYER".into(),
            score: "50".into(),
            options: "DEFAULT, 10 MIN".into(),
            teams: true,
            players: Vec::new(),
            bots: 0,
        };
        let o = OnlineView {
            game: Some(&info),
            countdown: Some(14.2),
            ..online(Some(&view))
        };
        for n in [4, 8, 12, 16] {
            // Gamertags as long as they come.
            let seats: Vec<SeatInfo> = (0..n)
                .map(|k| SeatInfo {
                    name: format!("SPARTAN {k:07}"),
                    how: "ONLINE",
                    team: (k % 2) as u8,
                    look: Look::default(),
                    level: Some(k as u8 + 1),
                })
                .collect();
            let c = Context {
                online: Some(&o),
                host_lobby: Some(&lobby),
                seats: &seats,
                ..ctx(&maps, &[])
            };
            let mut m = Menu::new(Settings::default(), Profile::default());
            m.show(Screen::Pregame);
            let mut hb = HudBuilder::new(1280.0, 720.0);
            m.draw(&mut hb, 0, 1, 1280.0, 720.0, &c);
            let f = Frame::new(1280.0, 720.0);
            let batches = hb.finish();
            let ranks = quads(&batches, |t| {
                t == crate::gpu::RANK_TEXTURES || t == crate::gpu::RANK_TEXTURES + 1
            });
            assert_eq!(ranks.len(), n, "everyone's rank");
            // The roster's text: the panel's, below the map.
            let panel = f.at(PANEL_X, ROW_Y + PICTURE[1] + 20.0);
            let text: Vec<_> = quads(&batches, |t| t == 0)
                .into_iter()
                .filter(|q| q[0] >= panel[0] && q[1] >= panel[1])
                .collect();
            let lowest = text.iter().map(|q| q[3]).fold(0.0, f32::max);
            assert!(
                lowest <= f.at(0.0, ROSTER_BOTTOM)[1],
                "{n}: above the notice line"
            );
            let smallest = text.iter().map(|q| q[3] - q[1]).fold(f32::MAX, f32::min);
            assert!(smallest >= 6.0 * f.s, "{n}: glyphs {smallest} px tall");
            let right = f.at(PANEL_X + PANEL_W, 0.0)[0];
            let overlap = |a: &[f32; 4], b: &[f32; 4]| {
                a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
            };
            for q in &text {
                assert!(q[2] <= right + 0.01, "{n}: in the panel");
                assert!(
                    !ranks.iter().any(|r| overlap(q, r)),
                    "{n}: clear of the ranks"
                );
            }
        }
    }

    #[test]
    fn a_match_waits_in_its_pregame_lobby_and_returns_to_the_party() {
        let maps = maps();
        let view = signed_in(1, &[(1, "JOHN")]);
        let info = MatchInfo {
            id: 7,
            playlist: 2,
            ranked: true,
            map: "lockout".into(),
            hash: 0,
            game_type: GameType::TeamSlayer,
            preset: "DEFAULT".into(),
            score: 25,
            time_limit: 600,
            bots: 0,
            host: 1,
            countdown: 20,
            players: Vec::new(),
        };
        let lobby = Lobby {
            map: "lockout".into(),
            game_type: "TEAM SLAYER".into(),
            score: "25".into(),
            options: "DEFAULT, 10 MIN".into(),
            teams: true,
            players: Vec::new(),
            bots: 0,
        };
        let seats: Vec<SeatInfo> = ["JOHN", "SARGE", "KEYES", "JOHNSON"]
            .iter()
            .zip(1..)
            .map(|(name, level)| SeatInfo {
                name: name.to_string(),
                how: "ONLINE",
                team: level % 2,
                look: Look::default(),
                level: Some(level),
            })
            .collect();
        let o = OnlineView {
            game: Some(&info),
            countdown: Some(14.2),
            ..online(Some(&view))
        };
        let c = Context {
            online: Some(&o),
            host_lobby: Some(&lobby),
            seats: &seats,
            ..ctx(&maps, &[])
        };
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::Pregame);
        assert_eq!(m.header(&c).as_deref(), Some("DOUBLE TEAM"));
        assert_eq!(m.rows(&c)[4], Row::Starting);
        let label = |row| m.label(row, &c);
        assert_eq!(label(Row::GameType).1.as_deref(), Some("TEAM SLAYER"));
        assert_eq!(
            label(Row::GameOptions).1.as_deref(),
            Some("DEFAULT, 10 MIN")
        );
        assert_eq!(
            label(Row::Starting),
            ("GAME ABOUT TO START!".into(), Some("0:15".into()))
        );
        // Nothing to choose, and no way back: the game starts by itself.
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert_eq!(m.input(Input::Back, &c), Action::None);
        assert_eq!(m.screen, Screen::Pregame);
        // Everyone's rank, from the large icons.
        let mut hb = HudBuilder::new(1280.0, 720.0);
        m.draw(&mut hb, 0, 1, 1280.0, 720.0, &c);
        let large: usize = hb
            .finish()
            .iter()
            .filter(|b| b.texture == crate::gpu::RANK_TEXTURES + 1)
            .map(|b| b.vertices.len() / 6)
            .sum();
        assert_eq!(large, 4);
        // Once the service says go, the wait is for everyone's map.
        let going = OnlineView {
            countdown: None,
            ..online(Some(&view))
        };
        let c = Context {
            online: Some(&going),
            ..c
        };
        let waiting = m.label(Row::Starting, &c).0;
        assert_eq!(waiting, "WAITING FOR EVERYONE TO LOAD THE MAP...");
        // After the game, back to the party by itself, or at once; joined
        // too, and leaving mid-game says so.
        let over = OnlineView {
            game: Some(&info),
            returning: Some(11.3),
            ..online(Some(&view))
        };
        let c = Context {
            online: Some(&over),
            joined: true,
            ..ctx(&maps, &[])
        };
        m.show(Screen::PostGame);
        assert_eq!(m.label(Row::Continue, &c).0, "RETURNING TO PARTY IN 12");
        assert_eq!(m.input(Input::Select, &c), Action::EndGame);
        assert_eq!(m.label(Row::EndGame, &c).0, "LEAVE GAME");
    }
}
