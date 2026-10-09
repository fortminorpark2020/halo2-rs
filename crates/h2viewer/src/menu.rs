//! Halo 2 style menus, drawn over the map: the start screen, the main
//! menu, the campaign's missions, the multiplayer lobby, the system link
//! browser, the player profile, the online screens (the party lobby, who's
//! online, the matchmaking playlists and searching them), the pause menu,
//! the dialogs that ask before quitting or leaving a game, and the
//! post-game carnage report. Keyboard, mouse and controllers all work them.
//! They're laid out and drawn as mainmenu.map's screens are (`menuart`),
//! or as plainly without it.

use crate::gpu::{hud_mode, MENU_TEXTURES};
use crate::hud::HudBuilder;
use crate::menuart::{self, animate, peak, tag_color, MenuArt, Painter, Space, Style};
use crate::messages::MenuText;
use crate::online::{clock_text, OnlineView};
use crate::options::{presets, GameOptions, RESPAWN_TIMES, TIME_LIMITS};
use crate::profile::{color_name, Profile};
use crate::rank::{self, LiveIcon};
use blam_cache::font::Font;
use blam_cache::ui::{self, ListSkin};
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
    /// Halo 2's start screen: any key or button goes on to the main menu.
    Start,
    Main,
    Campaign,
    Lobby,
    /// The lobby's game options.
    Options,
    SystemLink,
    Profile,
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
    /// A question before quitting, or ending or leaving a game
    /// (`Menu::asking`).
    Confirm,
}

/// What a dialog asks before it's done, as Halo 2 asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Quit,
    EndGame,
    /// A game joined on another PC, or an online match.
    LeaveGame,
    /// Another PC's System Link lobby.
    LeaveLobby,
}

impl Ask {
    /// Its dialog's lines in mainmenu.map (`MenuText`): its title, the
    /// question, and the answers that do it and that don't.
    fn lines(self) -> [&'static str; 4] {
        match self {
            Ask::Quit => [
                "errors_other/exit_confirmation",
                "errors_other/error_confirm_boot_to_dash",
                "errors_other/exit_halo2",
                "errors_other/no",
            ],
            Ask::EndGame => [
                "errors_live/are_you_sure",
                "errors_live/error_confirm_end_game_session",
                "errors_live/end_game",
                "errors_live/cancel",
            ],
            Ask::LeaveGame => [
                "errors_live/leave_game",
                "errors_live/confirm_exit_game_session",
                "errors_live/yes_leave_game",
                "errors_live/cancel",
            ],
            Ask::LeaveLobby => [
                "errors_networking/are_you_sure",
                "errors_networking/error_confirm_leave_system_link_lobby",
                "errors_networking/leave_lobby",
                "errors_networking/cancel",
            ],
        }
    }
}

/// A dialog up: what it asks, and the screen (and row) it was asked from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Asking {
    ask: Ask,
    from: Screen,
    cursor: usize,
}

/// The carnage report's panes, as Halo 2 named them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pane {
    Teams,
    Players,
    Kills,
    Medals,
}

impl Pane {
    /// Its name, in `MenuText`.
    fn title(self) -> &'static str {
        match self {
            Pane::Teams => "werds/team_stats",
            Pane::Players => "werds/player_stats",
            Pane::Kills => "werds/kill_stats",
            Pane::Medals => "werds/medal_stats",
        }
    }
}

/// The carnage report's panes for a game with these scores: TEAM STATS
/// first in team games.
fn panes(scores: &[ScoreLine]) -> Vec<Pane> {
    let teams = scores.iter().any(|l| l.header);
    let first = teams.then_some(Pane::Teams);
    first
        .into_iter()
        .chain([Pane::Players, Pane::Kills, Pane::Medals])
        .collect()
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
#[derive(Clone, Debug, Default)]
pub struct ScoreLine {
    pub name: String,
    pub score: i32,
    /// The score is seconds (shown as minutes and seconds).
    pub timed: bool,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub suicides: u32,
    /// The most kills in one life.
    pub best_spree: u32,
    /// Seconds alive per life, on average.
    pub avg_life: f32,
    /// The medals earned, by name, and how many of each.
    pub medals: Vec<(String, u16)>,
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
    /// A choice that can't be made (Halo 2's flag_fail).
    Error,
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
    /// A dialog's answers: do it, or don't.
    Yes,
    No,
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
    /// Seconds the game has been running, for what pulses.
    pub time: f32,
    pub profile: Profile,
    /// Typing a new gamertag.
    pub editing: bool,
    /// The campaign's difficulty, in `DIFFICULTIES`.
    pub difficulty: usize,
    /// The account picked from the online or recent players, and which
    /// list it was.
    player: u64,
    list: Screen,
    /// Halo 2's words for the dialogs, scoreboard and carnage report.
    pub text: MenuText,
    /// The dialog up (`Screen::Confirm`).
    asking: Option<Asking>,
    /// The carnage report's pane shown, in `panes`.
    pane: usize,
    /// Halo 2's menu art and fonts, as read from mainmenu.map and the
    /// fonts folder (none without them).
    pub art: MenuArt,
    /// Seconds the menus have been up (the art moves by it), and when the
    /// screen came up.
    clock: f32,
    opened: f32,
    /// What's behind the screens (the start screen and main menu's art,
    /// the framing behind the screens past them, or the game), and since
    /// when, so it comes in once and not with each screen.
    chrome: (u8, f32),
    /// The row the cursor was last seen on, the one before, and when it
    /// moved: the list skins animate the move.
    focus: (usize, usize, f32),
    /// Where the mouse is (window pixels), and where it was when a dialog
    /// came up: the dialog ignores it until it moves away from there.
    pointer: Option<[f32; 2]>,
    guard: Option<[f32; 2]>,
    /// A controller was used last: the legends show its buttons, not keys.
    pub controller: bool,
}

/// The carnage report's rows, in Halo 2's 640x480 screen units.
const ROW_X: f32 = 48.0;
const ROW_W: f32 = 300.0;
const ROW_H: f32 = 26.0;
const ROW_STEP: f32 = 32.0;
const TEXT: [f32; 4] = [0.72, 0.84, 1.0, 1.0];
const BRIGHT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const DIM: [f32; 4] = [0.5, 0.62, 0.8, 0.9];
const PANEL: [f32; 4] = [0.02, 0.07, 0.14, 0.72];
const HIGHLIGHT: [f32; 4] = [0.2, 0.45, 0.85, 0.9];
const WARNING: [f32; 4] = [1.0, 0.55, 0.3, 1.0];
/// The lobby's mark for someone in a team game with no team yet.
const UNPICKED: [f32; 3] = [0.5, 0.5, 0.5];

/// The menus' navy (the overlay colour of mainmenu.map's menu globals),
/// without the art.
const VEIL: [f32; 4] = [0.0, 0.08, 0.17, 0.85];
/// How much of the menus' overlay colour goes over a paused game, which
/// shows through.
const PAUSE_VEIL: f32 = 0.55;
/// Text in a list is never fainter than this, however far its item has
/// faded (Halo 2 fades whole items to a third), so it stays readable.
const TEXT_FLOOR: f32 = 0.6;
/// Long enough for any of the menus' animations to be over (seconds).
const SETTLED: f32 = 10.0;
/// How far the mouse moves (window pixels) from where it was when a dialog
/// came up under it before it works the dialog.
const GUARD: f32 = 8.0;
/// Halo 2's menu units to the pixel of the menus' 640x480 frame: 1200 to
/// its 480 lines (see `menuart`).
const UNITS: f32 = 2.5;

/// Where a screen's list goes (menu units): its first item's corner, the
/// step down to each next one, and an item's box from its corner (left,
/// top, right, bottom).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Place {
    corner: [f32; 2],
    step: f32,
    item: [f32; 4],
}

impl Place {
    /// Item `k`'s corner.
    fn corner(&self, k: usize) -> [f32; 2] {
        [self.corner[0], self.corner[1] - self.step * k as f32]
    }
}

/// The lists' places as mainmenu.map's tags have them, for without the
/// art: the main menu's (main_menu, its items its skin's text), the game
/// options' (top_level_settings, each item a settings_list_bkd), System
/// Link's (network_squad_browser, game_browser_list's item_background) and
/// the dialogs' (error_dialog_ok_cancel and error_dialog_large, the
/// default skin's item_background).
const MAIN_PLACE: Place = Place {
    corner: [-178.0, -80.0],
    step: 50.0,
    item: [-62.0, 10.0, 418.0, -40.0],
};
const OPTIONS_PLACE: Place = Place {
    corner: [-500.0, 300.0],
    step: 52.0,
    item: [15.0, 62.0, 1031.0, 10.0],
};
const BROWSER_PLACE: Place = Place {
    corner: [-670.0, 412.0],
    step: 32.0,
    item: [8.0, 32.0, 1358.0, 0.0],
};
const DIALOG_PLACE: Place = Place {
    corner: [-255.0, -140.0],
    step: 46.0,
    item: [40.0, 38.0, 440.0, -4.0],
};
const PAUSE_PLACE: Place = Place {
    corner: [-500.0, 40.0],
    step: 46.0,
    item: [40.0, 38.0, 440.0, -4.0],
};
/// The remake's own: the profile's settings in the setting lists' skin,
/// narrowed for the model beside them, and the online lists in the wide
/// default skin (default_wide) under the title, narrowed beside a panel.
const PROFILE_PLACE: Place = Place {
    corner: [-680.0, 420.0],
    ..OPTIONS_PLACE
};
const PROFILE_WIDTH: f32 = 820.0;
const ONLINE_PLACE: Place = Place {
    corner: [-640.0, 380.0],
    step: 46.0,
    item: [40.0, 38.0, 960.0, -4.0],
};
const ONLINE_NARROW: f32 = 700.0;
/// The panel beside the narrowed online lists (the party, or the player
/// picked), and the step between its rows.
const SIDE_PANEL: [f32; 4] = [160.0, 400.0, 700.0, -200.0];
const SIDE_STEP: f32 = 44.0;
/// The online lists' icons (menu units square).
const ICON: f32 = 34.0;

/// The pregame lobby's places as pregame_lobby has them, for without the
/// art: "Quick Options:" (gametype_options_format), whose lines below it
/// reach as far right as the tag's own lines (-50); the game type's and
/// map's lines (gametype_format, mapname_format); and its first two
/// buttons' text (START GAME's and GAME SETUP's places: GAME OPTIONS takes
/// the first, so the cursor reads on from the last line, left to right),
/// their pictures' offset and size.
const QUICK_OPTIONS: [f32; 4] = [-675.0, -195.0, -300.0, -235.0];
const LINE_RIGHT: f32 = -50.0;
const GAME_TYPE_LINE: [f32; 4] = [-580.0, -95.0, -150.0, -135.0];
const MAP_LINE: [f32; 4] = [-580.0, -130.0, -150.0, -170.0];
const LOBBY_BUTTONS: [[f32; 4]; 2] = [[-690.0, 480.0, -360.0, 440.0], [-320.0, 480.0, 8.0, 440.0]];
const BUTTON_OFFSET: [f32; 2] = [-15.0, -12.0];
const BUTTON_SIZE: [f32; 2] = [358.0, 64.0];
/// The map's picture (unknown_map's place and size).
const MAP_PICTURE_AT: [f32; 2] = [-448.0, -30.0];
const MAP_PICTURE_SIZE: [f32; 2] = [440.0, 414.0];
/// The lobby display's lines (game_cant_start_no_session's box, and the
/// display's lower edge for the description under them).
const LOBBY_STATUS: [f32; 4] = [-650.0, 320.0, -30.0, 280.0];
const LOBBY_ABOUT_BOTTOM: f32 = 60.0;
/// pregame_lobby's art the lobby draws (its display, the panel and bar
/// behind the quick options, the game type's icon and the lines under
/// them).
const LOBBY_ART: [&str; 5] = [
    "\\lobby_display",
    "\\divider",
    "\\game_settings",
    "\\mp_games2",
    "\\settings_framing",
];
/// Without the art: panels behind the lobby's left side and its players.
const LOBBY_PANEL: [f32; 4] = [-705.0, 350.0, -8.0, -480.0];
const PLAYERS_PANEL: [f32; 4] = [80.0, 560.0, 700.0, 60.0];
/// The players down the lobby's right: from pregame_lobby's player count
/// (player_count2, 540) across its speakers (100) to its download bars
/// (654, and their width), each row as far down as the speakers (39), or
/// up to ROSTER_STEP in the pregame's roster when there are few; and the
/// gap between its columns when there are many.
const ROSTER: [f32; 4] = [100.0, 540.0, 680.0, 100.0];
const ROSTER_HEAD: f32 = 47.0;
const PLAYER_STEP: f32 = 39.0;
const ROSTER_STEP: f32 = 56.0;
const ROSTER_GAP: f32 = 20.0;
/// A player's emblem and rank are at most this big (a lobby row's height).
const SEAT_ICON: f32 = PLAYER_STEP - 3.0;

/// System Link's columns: the game_browser_list skin's texts (host, map,
/// players, status) and their boxes without the art.
const BROWSER_COLUMNS: [(usize, [f32; 4]); 4] = [
    (0, [66.0, 30.0, 418.0, 0.0]),
    (1, [422.0, 30.0, 608.0, 0.0]),
    (4, [1002.0, 30.0, 1102.0, 0.0]),
    (5, [1107.0, 30.0, 1355.0, 0.0]),
];
/// Its heads (network_squad_browser's texts, and their boxes).
const BROWSER_HEADS: [(&str, &str, [f32; 4]); 4] = [
    ("host_gamename", "HOST", [-604.0, 495.0, -250.0, 460.0]),
    ("map_head", "MAP", [-250.0, 495.0, -60.0, 460.0]),
    ("player_head", "PLAYERS", [330.0, 495.0, 435.0, 460.0]),
    ("status_head", "STATUS", [435.0, 495.0, 680.0, 460.0]),
];
/// Where it says there are no games (no_games), its help (help_create_game)
/// and the chosen game's map (unknown_map's place, at 0.68).
const NO_GAMES: [f32; 4] = [-300.0, 410.0, 300.0, 370.0];
const NO_GAMES_TEXT: &str = "THERE ARE NO OTHER GAMES ON THE NETWORK AT THIS TIME";
const BROWSER_HELP: [f32; 4] = [-610.0, -235.0, -200.0, -380.0];
const BROWSER_PICTURE_AT: [f32; 2] = [410.0, -175.0];
const BROWSER_PICTURE_SIZE: [f32; 2] = [299.0, 282.0];

/// How much of the menus' overlay colour goes behind a dialog's art.
const DIALOG_BACKING: f32 = 0.9;
/// Dialogs without the art: behind their questions and answers.
const DIALOG_BOX: [f32; 4] = [-290.0, 200.0, 300.0, -260.0];
const PAUSE_BOX: [f32; 4] = [-540.0, 180.0, 560.0, -260.0];
/// A dialog's question (error_dialog_ok_cancel's text), and the large
/// dialog's help (error_dialog_large's), where a mission's objectives go.
const QUESTION_BOX: [f32; 4] = [-250.0, 140.0, 240.0, -80.0];
const OBJECTIVES_BOX: [f32; 4] = [0.0, 100.0, 520.0, -80.0];
/// mainmenu.map's header and button legend bounds by dialog size, for
/// without the art.
const HEADER_BOUNDS: [[f32; 4]; 4] = [
    [-730.0, 567.0, 50.0, 520.0],
    [-500.0, 390.0, 55.0, 310.0],
    [-500.0, 158.0, 255.0, 73.0],
    [-252.0, 182.0, 350.0, 142.0],
];
const LEGEND_BOUNDS: [[f32; 4]; 4] = [
    [100.0, -500.0, 730.0, -540.0],
    [0.0, -354.0, 535.0, -394.0],
    [-400.0, -225.0, 480.0, -270.0],
    [-175.0, -225.0, 235.0, -265.0],
];
/// The legends the menus use, as mainmenu.map words them.
const LEGENDS: [(&str, &str); 3] = [
    ("a_select", "\u{e100} SELECT"),
    ("a_select_b_back", "\u{e100} SELECT \u{e101} BACK"),
    ("a_select_b_cancel", "\u{e100} SELECT \u{e101} CANCEL"),
];
/// How wide the line under a title goes.
const SUBHEADER_W: f32 = 1100.0;
/// Where notices and notes go: on most screens above the legend; on the
/// main menu under its list; on the profile under its settings.
const NOTE: [f32; 4] = [-730.0, -380.0, 300.0, -490.0];
const MAIN_NOTE: [f32; 4] = [-600.0, -370.0, 600.0, -480.0];
const PROFILE_NOTE: [f32; 4] = [-680.0, -60.0, 120.0, -300.0];
/// The start screen without the art: the logo's place and its line.
const LOGO_BOX: [f32; 4] = [-500.0, 200.0, 500.0, 60.0];
const PRESS_ANY_KEY: &str = "PRESS ANY KEY TO CONTINUE";
const PRESS_ANY_KEY_BOX: [f32; 4] = [-400.0, -65.0, 400.0, -105.0];
/// The main menu's gamertag (main_menu's text at its bottom right).
const GAMERTAG_BOX: [f32; 4] = [376.0, -562.0, 610.0, -600.0];
/// The profile's model stands in a window in the framing (main.rs puts it
/// at 470, 290 of the frame: 375, -125 here), its emblem above it.
const MODEL_WINDOW: [f32; 4] = [210.0, 300.0, 560.0, -470.0];
const EMBLEM_BOX: [f32; 4] = [320.0, 520.0, 450.0, 390.0];

/// A tag's box (left, top, right, bottom).
fn bounds(r: ui::Rect) -> [f32; 4] {
    [r.left as f32, r.top as f32, r.right as f32, r.bottom as f32]
}

/// A box moved by `[x, y]`.
fn offset([l, t, r, b]: [f32; 4], [x, y]: [f32; 2]) -> [f32; 4] {
    [l + x, t + y, r + x, b + y]
}

/// Whether a bitmap widget's tag's name ends with `name`.
fn is(b: &ui::Bitmap, name: &str) -> bool {
    b.bitmap.as_ref().is_some_and(|t| t.name.ends_with(name))
}

/// A button's picture as a bitmap widget.
fn button_bitmap(t: &ui::TagRef) -> ui::Bitmap {
    ui::Bitmap {
        flags: 0,
        animation: None,
        delay_ms: 0,
        multiply: false,
        frame: 0,
        corner: [0, 0],
        wraps_per_second: [0.0, 0.0],
        bitmap: Some(t.clone()),
        depth: 0,
        scale: [0.0, 0.0],
    }
}

/// An item lit to `alpha` without the art: Halo 2's blue.
fn glow(alpha: f32) -> [f32; 4] {
    [
        HIGHLIGHT[0],
        HIGHLIGHT[1],
        HIGHLIGHT[2],
        HIGHLIGHT[3] * alpha,
    ]
}

/// A box of menu units in the frame's 640x480 units (x0, y0, x1, y1).
fn frame_box([l, t, r, b]: [f32; 4]) -> [f32; 4] {
    [
        320.0 + l / UNITS,
        240.0 - t / UNITS,
        320.0 + r / UNITS,
        240.0 - b / UNITS,
    ]
}

/// Menu units in a frame: its middle the origin.
fn space(f: &Frame) -> Space {
    Space {
        k: f.s / UNITS,
        origin: f.at(320.0, 240.0),
    }
}

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

/// The smallest text the menus draw, in window pixels: a pixel to each
/// of the 5x7 font's (in its 6x8 cell), which smaller windows and
/// splitscreen views would otherwise shrink past reading.
const MIN_TEXT: f32 = 8.0;

/// Turns 640x480 screen units into window pixels, centred.
struct Frame {
    s: f32,
    ox: f32,
    oy: f32,
}

impl Frame {
    fn new(w: f32, h: f32) -> Frame {
        Frame::scaled(w, h, (h / 480.0).min(w / 640.0))
    }

    /// For the scoreboard over a game: never smaller than a 640x480
    /// screen's own pixels, so a splitscreen view's text stays readable;
    /// what doesn't fit the view is left off rather than shrunk.
    fn readable(w: f32, h: f32) -> Frame {
        Frame::scaled(w, h, (h / 480.0).min(w / 640.0).max(1.0))
    }

    fn scaled(w: f32, h: f32, s: f32) -> Frame {
        Frame {
            s,
            ox: (w - 640.0 * s) * 0.5,
            oy: (h - 480.0 * s) * 0.5,
        }
    }

    /// The screen units a `w` x `h` window shows: left, top, right and
    /// bottom.
    fn shown(&self, w: f32, h: f32) -> [f32; 4] {
        [
            -self.ox / self.s,
            -self.oy / self.s,
            (w - self.ox) / self.s,
            (h - self.oy) / self.s,
        ]
    }

    /// Window pixels for text `size` screen units tall, never under
    /// `MIN_TEXT`.
    fn text(&self, size: f32) -> f32 {
        (size * self.s).max(MIN_TEXT)
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
            time: 0.0,
            profile,
            editing: false,
            difficulty: 1,
            player: 0,
            list: Screen::Players,
            text: MenuText::default(),
            asking: None,
            pane: 0,
            art: MenuArt::default(),
            clock: 0.0,
            opened: 0.0,
            chrome: (0, 0.0),
            focus: (0, usize::MAX, 0.0),
            pointer: None,
            guard: None,
            controller: false,
        }
    }

    pub fn show(&mut self, screen: Screen) {
        // What's behind: the start screen's and main menu's art, the
        // framing behind the rest, or the game; a dialog keeps what it's
        // over.
        let chrome = match screen {
            Screen::Start | Screen::Main => 0,
            Screen::Pause | Screen::PostGame => 2,
            Screen::Confirm => self.chrome.0,
            _ => 1,
        };
        if chrome != self.chrome.0 {
            self.chrome = (chrome, self.clock);
        }
        self.screen = screen;
        self.cursor = 0;
        self.scroll = 0;
        self.asking = None;
        self.pane = 0;
        self.opened = self.clock;
        self.focus = (0, usize::MAX, self.clock);
        self.guard = None;
    }

    /// Time passes in the menus: their art moves, and a list's items light
    /// and dim as the cursor comes and goes.
    pub fn tick(&mut self, dt: f32) {
        self.clock += dt;
        if self.cursor != self.focus.0 {
            self.focus = (self.cursor, self.focus.0, self.clock);
        }
    }

    /// The pause menu is up, or a dialog asked from it.
    pub fn pausing(&self) -> bool {
        self.screen == Screen::Pause || self.asking.is_some_and(|a| a.from == Screen::Pause)
    }

    /// Ask before doing `ask`: its dialog comes up with the cursor on the
    /// answer that does nothing.
    fn ask(&mut self, ask: Ask) -> Action {
        let asking = Asking {
            ask,
            from: self.screen,
            cursor: self.cursor,
        };
        self.show(Screen::Confirm);
        self.asking = Some(asking);
        self.cursor = 1;
        self.sound = Some(Sound::Forward);
        // Whatever comes up under the mouse waits for it to move.
        self.guard = self.pointer;
        Action::None
    }

    /// The dialog answered: back where it was asked, then on with what it
    /// asked about if the answer was yes.
    fn answer(&mut self, yes: bool, ctx: &Context) -> Action {
        let Some(a) = self.asking.take() else {
            return Action::None;
        };
        self.show(a.from);
        self.cursor = a.cursor;
        if !yes {
            self.sound = Some(Sound::Back);
            return Action::None;
        }
        match a.ask {
            Ask::Quit => Action::Quit,
            Ask::EndGame | Ask::LeaveGame => self.end_game(ctx),
            Ask::LeaveLobby => {
                self.sound = Some(Sound::Back);
                Action::Leave
            }
        }
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

    fn title(&self, ctx: &Context) -> String {
        let title = match self.screen {
            Screen::Confirm => {
                let ask = self.asking.map_or(Ask::Quit, |a| a.ask);
                return self.text.get(ask.lines()[0]);
            }
            Screen::PostGame => return self.text.get("werds/postgame_header"),
            Screen::Start | Screen::Main => "HALO 2",
            Screen::Campaign => "CAMPAIGN",
            Screen::Lobby if in_custom(ctx) => "CUSTOM GAME",
            Screen::Lobby => "MULTIPLAYER",
            Screen::Options => "GAME OPTIONS",
            Screen::SystemLink => "SYSTEM LINK",
            Screen::Profile => "PLAYER PROFILE",
            Screen::Pause => "PAUSED",
            Screen::Live => "ONLINE",
            Screen::Players => "ONLINE PLAYERS",
            Screen::RecentPlayers => "RECENT PLAYERS",
            Screen::Player => "PLAYER",
            Screen::Playlists => "PLAYLISTS",
            Screen::Matchmaking => "MATCHMAKING",
            Screen::Pregame => "PREGAME LOBBY",
        };
        title.into()
    }

    fn rows(&self, ctx: &Context) -> Vec<Row> {
        match self.screen {
            Screen::Start => Vec::new(),
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
            ],
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
            Screen::Confirm => vec![Row::Yes, Row::No],
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
            Row::Yes | Row::No => {
                let lines = self.asking.map_or(Ask::Quit, |a| a.ask).lines();
                let line = if row == Row::Yes { lines[2] } else { lines[3] };
                (self.text.get(line), None)
            }
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
        if self.screen == Screen::Start {
            // Any key or button goes on.
            self.show(Screen::Main);
            self.sound = Some(Sound::Forward);
            return Action::None;
        }
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
                // up from RESUME (nor is yes from no on a dialog).
                let end = match input {
                    Input::Up => self.cursor == 0,
                    _ => self.cursor + 1 >= n,
                };
                let stops = matches!(self.screen, Screen::Pause | Screen::Confirm);
                if selectable > 1 && !(stops && end) {
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
            Input::Left | Input::Right if self.screen == Screen::PostGame => {
                let n = panes(ctx.scores).len();
                let step = if input == Input::Left { n - 1 } else { 1 };
                self.pane = (self.pane + step) % n;
                self.sound = Some(Sound::Cursor);
                Action::None
            }
            // The lobby's GAME OPTIONS and START GAME are side by side.
            Input::Left | Input::Right
                if self.screen == Screen::Lobby
                    && matches!(row, Some(Row::GameOptions | Row::StartGame)) =>
            {
                let other = if row == Some(Row::GameOptions) {
                    Row::StartGame
                } else {
                    Row::GameOptions
                };
                if let Some(k) = rows.iter().position(|&r| r == other) {
                    self.cursor = k;
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
            Row::Quit => self.ask(Ask::Quit),
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
            | Row::EmblemBackColor => {
                if self.adjust(row, 1, ctx) {
                    self.sound = Some(Sound::Cursor);
                }
                self.saving(row)
            }
            Row::StartGame if ctx.maps.is_empty() => {
                self.notice = Some("NO MULTIPLAYER MAPS FOUND".into());
                self.sound = Some(Sound::Error);
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
                    self.sound = Some(Sound::Error);
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
            Row::EndGame if ctx.joined || in_match(ctx) => self.ask(Ask::LeaveGame),
            Row::EndGame => self.ask(Ask::EndGame),
            Row::Continue => self.end_game(ctx),
            Row::Yes => self.answer(true, ctx),
            Row::No => self.answer(false, ctx),
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
            self.sound = Some(Sound::Error);
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
                self.sound = Some(Sound::Error);
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
            Screen::Lobby if ctx.joined => self.ask(Ask::LeaveLobby),
            Screen::Confirm => self.answer(false, ctx),
            Screen::Start | Screen::Main => Action::None,
            Screen::Profile => {
                self.back_to_main(Row::Profile);
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

    /// Where row `k` (of those shown) is, in the frame's 640x480 units:
    /// on the carnage report under its stats, elsewhere where it's drawn.
    fn row_rect(&self, k: usize) -> [f32; 4] {
        match self.screen {
            Screen::PostGame => {
                let y = REPORT_BOTTOM + k as f32 * ROW_STEP;
                [ROW_X, y, ROW_X + ROW_W, y + ROW_H]
            }
            _ => frame_box(self.row_box(k)),
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

    /// Whether the mouse, now at `pos` (window pixels), is to be ignored:
    /// a dialog came up under it, and it hasn't moved away yet.
    fn guarded(&mut self, pos: [f32; 2]) -> bool {
        self.pointer = Some(pos);
        match self.guard {
            Some(g) if (pos[0] - g[0]).hypot(pos[1] - g[1]) <= GUARD => true,
            _ => {
                self.guard = None;
                false
            }
        }
    }

    /// The mouse moved to `pos` (window pixels).
    pub fn hover(&mut self, pos: [f32; 2], w: f32, h: f32, ctx: &Context) {
        if self.guarded(pos) {
            return;
        }
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
        if self.guarded(pos) {
            return Action::None;
        }
        if self.screen == Screen::Start {
            return self.input(Input::Select, ctx);
        }
        if self.screen == Screen::PostGame {
            // The carnage report's tabs show their panes.
            let f = Frame::new(w, h);
            let [x, y] = pos;
            let hit = self.tabs(ctx, &f).iter().position(|(_, r)| {
                let [x0, y0, x1, y1] = f.rect(*r);
                x >= x0 && x < x1 && y >= y0 && y < y1
            });
            if let Some(k) = hit {
                if k != self.pane {
                    self.pane = k;
                    self.sound = Some(Sound::Cursor);
                }
                return Action::None;
            }
        }
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
        if self.screen == Screen::PostGame {
            self.draw_postgame(hb, [font, white], &f, [w, h], ctx);
            return;
        }
        let mut p = Painter::new(hb, &self.art, space(&f), [font, white], self.clock);
        self.draw_behind(&mut p, [w, h]);
        match self.screen {
            Screen::Start => self.draw_start(&mut p),
            Screen::Main => self.draw_main(&mut p, ctx),
            Screen::Lobby | Screen::Pregame => self.draw_lobby(&mut p, ctx),
            Screen::SystemLink => self.draw_browser(&mut p, ctx),
            Screen::Confirm | Screen::Pause => self.draw_dialog(&mut p, ctx),
            _ => self.draw_list(&mut p, ctx),
        }
        match (self.screen, ctx.online) {
            (Screen::Profile, _) => self.draw_profile(&mut p),
            (Screen::Live, Some(o)) => draw_party(&mut p, o),
            (Screen::Player, Some(o)) => draw_player(&mut p, o, self.player),
            _ => {}
        }
        self.draw_header(&mut p, ctx);
        self.draw_note(&mut p, ctx);
        self.draw_legend(&mut p, ctx);
    }

    /// The carnage report, as the game's flow drew it: over a dark veil,
    /// its title, tabs and stats, then CONTINUE and any notice under them.
    fn draw_postgame(
        &self,
        hb: &mut HudBuilder,
        [font, white]: [usize; 2],
        f: &Frame,
        [w, h]: [f32; 2],
        ctx: &Context,
    ) {
        let veil = [0.0, 0.0, 0.02, 0.8];
        hb.quad(
            white,
            [0.0, 0.0, w, h],
            [0.0; 4],
            veil,
            hud_mode::PLAIN,
            0.0,
        );
        hb.text_left(
            font,
            f.at(ROW_X, 48.0),
            26.0 * f.s,
            &self.title(ctx),
            BRIGHT,
        );
        let rule = f.rect([ROW_X, 84.0, ROW_X + 300.0, 86.0]);
        hb.quad(white, rule, [0.0; 4], HIGHLIGHT, hud_mode::PLAIN, 0.0);
        self.draw_report(hb, font, white, f, ctx);
        let rows = self.rows(ctx);
        let cursor = self.settled(&rows, ctx);
        for (k, &row) in rows.iter().enumerate() {
            let rect = self.row_rect(k);
            let (bg, fg) = if k == cursor {
                (HIGHLIGHT, BRIGHT)
            } else {
                (PANEL, TEXT)
            };
            hb.quad(white, f.rect(rect), [0.0; 4], bg, hud_mode::PLAIN, 0.0);
            let size = f.text(11.0);
            let middle = f.at(0.0, rect[1] + ROW_H * 0.5)[1] - size * 0.5;
            let at = [f.at(rect[0] + 10.0, 0.0)[0], middle];
            hb.text_left(font, at, size, &self.label(row, ctx).0, fg);
        }
        if let Some(n) = &self.notice {
            let at = f.at(ROW_X, REPORT_BOTTOM + ROW_H + 8.0);
            hb.text_left(font, at, f.text(10.0), n, WARNING);
        }
        let hint = "ENTER OR A: CONTINUE   LEFT OR RIGHT: MORE STATS";
        hb.text_left(font, f.at(ROW_X, 440.0), f.text(8.0), hint, DIM);
    }

    /// What's behind a screen: the start screen's and main menu's own art
    /// over the scene; the framing and moving tracks behind the screens
    /// past them (a navy veil without the art), with a window in it where
    /// the profile's model stands; and behind a dialog the screen it was
    /// asked from (or the game), under the menus' overlay colour.
    fn draw_behind(&self, p: &mut Painter, [w, h]: [f32; 2]) {
        let under = match (self.screen, self.asking) {
            (Screen::Confirm, Some(a)) => a.from,
            (s, _) => s,
        };
        let age = self.clock - self.chrome.1;
        match under {
            Screen::Start | Screen::Main => {
                let name = if under == Screen::Start {
                    menuart::START_SCREEN
                } else {
                    menuart::MAIN_MENU
                };
                p.screen(name, age, |_| true, None);
            }
            // The game is behind these.
            Screen::Pause | Screen::PostGame => {}
            _ if self.art.loaded() => {
                let hole = (under == Screen::Profile).then_some(MODEL_WINDOW);
                p.screen(menuart::BACKGROUND, age, |_| true, hole);
                if let Some(hole) = hole {
                    p.rim(hole, 1.0);
                }
            }
            _ => {
                let veil = [VEIL[0], VEIL[1], VEIL[2], 0.7];
                p.hb.quad(
                    p.white,
                    [0.0, 0.0, w, h],
                    [0.0; 4],
                    veil,
                    hud_mode::PLAIN,
                    0.0,
                );
            }
        }
        if matches!(self.screen, Screen::Confirm | Screen::Pause) {
            let g = &self.art.ui.globals;
            let [r, gr, b, a] = if self.art.loaded() {
                g.overlay_color
            } else {
                VEIL
            };
            // Over a game the pause menu lets it show through.
            let a = if under == Screen::Pause {
                PAUSE_VEIL
            } else {
                a
            };
            let veil = tag_color([r, gr, b], a);
            p.hb.quad(
                p.white,
                [0.0, 0.0, w, h],
                [0.0; 4],
                veil,
                hud_mode::PLAIN,
                0.0,
            );
        }
    }

    /// Halo 2's start screen: its logo and tracks (`draw_behind`), and
    /// "PRESS ANY KEY TO CONTINUE" breathing under them.
    fn draw_start(&self, p: &mut Painter) {
        let texts = self
            .art
            .screen(menuart::START_SCREEN)
            .and_then(|s| s.panes.first())
            .map_or(&[][..], |pane| &pane.texts[..]);
        match texts.iter().find(|t| t.string == "start_screen_0") {
            Some(t) => {
                let text = t.text.as_deref().unwrap_or(PRESS_ANY_KEY).to_uppercase();
                p.line(bounds(t.bounds), Style::of(t, 1.0), &text);
            }
            None => {
                self.draw_logo(p);
                let style = Style::new(Font::Title, ui::PULSATING, TEXT);
                p.line(PRESS_ANY_KEY_BOX, style, PRESS_ANY_KEY);
            }
        }
    }

    /// "HALO 2" where the logo goes, without the art.
    fn draw_logo(&self, p: &mut Painter) {
        if !self.art.loaded() {
            let style = Style::new(Font::SuperLarge, 0, BRIGHT);
            p.line(LOGO_BOX, style, "HALO 2");
        }
    }

    /// The main menu: its items on their glow bars down the middle (the one
    /// chosen lit, the rest at half), and the gamertag at the bottom right.
    fn draw_main(&self, p: &mut Painter, ctx: &Context) {
        self.draw_logo(p);
        let skin = self.art.list_skin(menuart::MAIN_MENU);
        let place = self.place();
        let (fade, [fx, fy]) = self.list_fade(Some(menuart::MAIN_MENU));
        let rows = self.rows(ctx);
        let cursor = self.settled(&rows, ctx);
        for (k, &row) in rows.iter().enumerate() {
            let (look, dx) = self.item_look(k, cursor, skin);
            let [x, y] = place.corner(k);
            let corner = [x + dx + fx, y + fy];
            let label = self.label(row, ctx).0;
            let text_alpha = look.max(TEXT_FLOOR) * fade;
            match skin {
                Some(skin) => {
                    p.item(skin, corner, look * fade, 1.0, |_| true);
                    // The shadow (listed second) under the label.
                    for t in skin.texts.iter().rev() {
                        let style = Style::of(t, text_alpha);
                        p.line(offset(bounds(t.bounds), corner), style, &label);
                    }
                }
                None => {
                    let item = offset(place.item, corner);
                    p.quad(item, glow(look * fade));
                    let style = Style::new(Font::MainMenu, 0, TEXT).alpha(text_alpha);
                    p.line(item, style, &label);
                }
            }
        }
        // The gamertag, at the bottom right.
        let tag = self
            .art
            .screen(menuart::MAIN_MENU)
            .and_then(|s| s.panes.first())
            .and_then(|pane| pane.texts.iter().find(|t| t.string.is_empty()));
        let (box_, style) = match tag {
            Some(t) => (bounds(t.bounds), Style::of(t, fade)),
            None => (GAMERTAG_BOX, Style::new(Font::SplitHudMessage, 1, DIM)),
        };
        p.line(box_, style, &self.profile.name);
    }

    /// A screen's list of rows in its skin: each item's background (lit
    /// as the cursor comes to it), its name and its value, and the online
    /// lists' icons; and on a long list, which of its rows show.
    fn draw_list(&self, p: &mut Painter, ctx: &Context) {
        let rows = self.rows(ctx);
        let cursor = self.settled(&rows, ctx);
        let shown = self.shown(rows.len());
        let place = self.place();
        let skin = self.list_skin();
        let (fade, [fx, fy]) = self.list_fade(self.list_screen());
        let columns = self.columns(skin, &place);
        let skin_width = skin
            .and_then(|s| self.art.item_box(s))
            .map_or(place.item[2] - place.item[0], |b| b[2] - b[0]);
        let stretch = (place.item[2] - place.item[0]) / skin_width.max(1.0);
        let first = shown.start;
        for (k, &row) in rows.iter().enumerate().take(shown.end).skip(first) {
            let selectable = row.selectable(ctx);
            let focused = k == cursor && selectable;
            let (look, dx) = if selectable {
                self.item_look(k, cursor, skin)
            } else {
                (1.0, 0.0)
            };
            let [x, y] = place.corner(k - first);
            let corner = [x + dx + fx, y + fy];
            let item = offset(place.item, corner);
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
            if selectable {
                match skin {
                    Some(skin) => {
                        let plain =
                            |b: &ui::Bitmap| !is(b, "\\hilite") && !is(b, "\\hilite_bracket");
                        p.item(skin, corner, look * fade, stretch, plain);
                        if focused {
                            let hilite = skin.bitmaps.iter().find(|b| is(b, "\\hilite"));
                            if let Some((b, [_, h])) =
                                hilite.and_then(|b| Some((b, self.art.size(b)?)))
                            {
                                let at = [
                                    corner[0] + b.corner[0] as f32,
                                    corner[1] + b.corner[1] as f32 + h,
                                ];
                                p.bitmap(b, at, fade, stretch);
                            }
                        }
                    }
                    None => p.quad(item, glow(look * fade * 0.6)),
                }
            }
            let (label, value) = self.label(row, ctx);
            let alpha = if !selectable {
                fade
            } else if dimmed {
                look.max(TEXT_FLOOR) * fade * 0.6
            } else {
                look.max(TEXT_FLOOR) * fade
            };
            let [(mut lbox, lstyle), (mut vbox, vstyle)] = columns;
            // Players have icons on the left; playlists, on the right.
            match row {
                Row::Player(_) | Row::RecentPlayer(_) => lbox[0] += ICON + 16.0,
                Row::Playlist(_) => vbox[2] -= ICON + 16.0,
                _ => {}
            }
            let bright = matches!(row, Row::SearchStatus | Row::Starting);
            let lstyle = match (selectable, bright) {
                (false, true) => Style {
                    color: BRIGHT,
                    ..lstyle
                },
                (false, false) => Style {
                    color: DIM,
                    ..lstyle
                },
                _ => lstyle,
            }
            .alpha(alpha);
            let lbox = offset(lbox, corner);
            p.line(lbox, lstyle, &label);
            let label_right = lbox[0] + p.width(lstyle.font, &label);
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
                let vbox = offset(vbox, corner);
                let brackets = skin.and_then(|s| {
                    let mut b = s.bitmaps.iter().filter(|b| is(b, "\\hilite_bracket"));
                    Some((b.next()?, b.next()?))
                });
                let v = match brackets {
                    // Halo 2's brackets above and below a setting's value.
                    Some((top, bottom)) if focused && steps => {
                        let middle = (vbox[0] + vbox[2]) * 0.5;
                        for b in [top, bottom] {
                            if let Some([w, h]) = self.art.size(b) {
                                let at = [middle - w * 0.5, corner[1] + b.corner[1] as f32 + h];
                                p.bitmap(b, at, fade, 1.0);
                            }
                        }
                        v
                    }
                    _ if focused && steps => format!("< {v} >"),
                    _ => v,
                };
                // What players are doing is long: smaller.
                let font = match row {
                    Row::Player(_) | Row::RecentPlayer(_) | Row::Playlist(_) => {
                        Font::SplitHudMessage
                    }
                    _ => vstyle.font,
                };
                let vstyle = Style {
                    font,
                    color: lstyle.color,
                    ..vstyle
                };
                let room = vbox[2] - vbox[0].max(label_right + 20.0);
                p.line(vbox, vstyle, &p.fit(font, &v, room.max(0.0)));
            }
            if let Some(o) = ctx.online {
                draw_icons(p, row, item, label_right, o);
            }
        }
        if shown.len() < rows.len() {
            // Where in a long list the rows shown are.
            let [x, y] = place.corner(shown.len());
            let at = offset(place.item, [x, y + place.step * 0.25]);
            let text = format!("{}-{} OF {}", shown.start + 1, shown.end, rows.len());
            let style = Style::new(Font::SplitHudMessage, ui::RIGHT_JUSTIFY, DIM);
            p.line(at, style, &text);
        }
    }

    /// A dialog: Halo 2's corners and gradients, its question (or, over a
    /// mission, its objectives), then its answers.
    fn draw_dialog(&self, p: &mut Painter, ctx: &Context) {
        let name = self.list_screen().unwrap_or(menuart::DIALOG);
        let age = self.clock - self.opened;
        // The menus' overlay colour between the dialog's corners (ul_07's
        // top left, br_07's bottom right), so what's behind doesn't show
        // through its words; its own box, without the art.
        let pane = self.art.screen(name).and_then(|s| s.panes.first());
        let corners = pane.and_then(|pane| {
            let ul = pane.bitmaps.iter().find(|b| is(b, "\\ul_07"))?;
            let br = pane.bitmaps.iter().find(|b| is(b, "\\br_07"))?;
            let [w, h] = self.art.size(br)?;
            let [l, t] = ul.corner.map(f32::from);
            let [x, y] = br.corner.map(f32::from);
            Some([l, t, x + w, y - h])
        });
        match corners {
            Some(back) => {
                let [r, g, b, _] = self.art.ui.globals.overlay_color;
                p.quad(back, tag_color([r, g, b], DIALOG_BACKING));
            }
            None if self.screen == Screen::Confirm => p.quad(DIALOG_BOX, PANEL),
            None => p.quad(PAUSE_BOX, PANEL),
        }
        p.screen(name, age, |_| true, None);
        let text = self
            .art
            .screen(name)
            .and_then(|s| s.panes.first())
            .and_then(|pane| pane.texts.first());
        let (fade, _) = animate(text.and_then(|t| self.art.intro(t.animation)), age);
        let fade = fade / peak(text.and_then(|t| self.art.intro(t.animation)));
        let style = match text {
            Some(t) => Style::of(t, fade),
            None => Style::new(Font::Body, ui::LEFT_JUSTIFY, TEXT),
        };
        match (self.screen, self.asking) {
            (Screen::Confirm, Some(a)) => {
                let question = self.text.get(a.ask.lines()[1]);
                let box_ = text.map_or(QUESTION_BOX, |t| bounds(t.bounds));
                p.paragraph(box_, style, &question);
            }
            _ if !ctx.objectives.is_empty() => {
                let box_ = text.map_or(OBJECTIVES_BOX, |t| bounds(t.bounds));
                draw_objectives(
                    p,
                    [box_[0], box_[1] + 40.0, box_[2], box_[3] - 120.0],
                    style,
                    ctx.objectives,
                );
            }
            _ => {}
        }
        self.draw_list(p, ctx);
    }

    /// The screen's title at the top left, as Halo 2's headers are (a
    /// dialog's over it), and a line under it saying what's below.
    fn draw_header(&self, p: &mut Painter, ctx: &Context) {
        let size = match self.screen {
            Screen::Start | Screen::Main => return,
            Screen::Confirm => self.dialog_size(menuart::DIALOG, ui::DialogSize::Quarter),
            Screen::Pause => self.dialog_size(menuart::LARGE_DIALOG, ui::DialogSize::Half),
            _ => ui::DialogSize::Full,
        } as usize;
        let g = &self.art.ui.globals;
        let (box_, font, color) = if self.art.loaded() {
            let c = tag_color(g.text_color, 1.0);
            (bounds(g.header_bounds[size]), g.header_fonts[size], c)
        } else {
            (HEADER_BOUNDS[size], Font::Title, BRIGHT)
        };
        let title = self.title(ctx).to_uppercase();
        p.line(box_, Style::new(font, ui::LEFT_JUSTIFY, color), &title);
        // Under the title (the lobby and System Link say it in their own
        // panels).
        let below = matches!(
            self.screen,
            Screen::Live | Screen::Playlists | Screen::Matchmaking
        );
        if let Some(header) = self.header(ctx).filter(|_| below) {
            let [l, t, _, b] = box_;
            let style = Style::new(Font::Body, ui::LEFT_JUSTIFY, TEXT);
            let under = [l, b - 10.0, l + SUBHEADER_W, b - 10.0 - (t - b)];
            p.line(under, style, &p.fit(Font::Body, &header, SUBHEADER_W));
        }
    }

    /// A notice (why something can't be done), or else a note on what's
    /// chosen, where the screen has room for it.
    fn draw_note(&self, p: &mut Painter, ctx: &Context) {
        let box_ = match self.screen {
            Screen::Start
            | Screen::Confirm
            | Screen::Pause
            | Screen::SystemLink
            | Screen::Lobby
            | Screen::Pregame
            | Screen::PostGame => return,
            Screen::Main => MAIN_NOTE,
            Screen::Profile => PROFILE_NOTE,
            _ => NOTE,
        };
        if let Some(n) = &self.notice {
            let style = Style::new(Font::Body, ui::LEFT_JUSTIFY, WARNING);
            let style = if self.screen == Screen::Main {
                Style { flags: 0, ..style }
            } else {
                style
            };
            p.paragraph(box_, style, n);
        } else if let Some(note) = self.note(ctx) {
            let style = Style::new(Font::Body, ui::LEFT_JUSTIFY, TEXT);
            p.paragraph(box_, style, &note);
        }
    }

    /// The buttons to press, at the bottom right as Halo 2's legends are.
    fn draw_legend(&self, p: &mut Painter, ctx: &Context) {
        let Some(legend) = self.legend(ctx) else {
            return;
        };
        let size = match self.screen {
            Screen::Confirm => self.dialog_size(menuart::DIALOG, ui::DialogSize::Quarter),
            Screen::Pause => self.dialog_size(menuart::LARGE_DIALOG, ui::DialogSize::Half),
            _ => ui::DialogSize::Full,
        } as usize;
        let g = &self.art.ui.globals;
        let (box_, color) = if self.art.loaded() {
            (
                bounds(g.button_key_bounds[size]),
                tag_color(g.text_color, 1.0),
            )
        } else {
            (LEGEND_BOUNDS[size], DIM)
        };
        let style = Style::new(Font::Body, ui::RIGHT_JUSTIFY, color);
        p.line(box_, style, &legend);
    }

    /// The legend for this screen: Halo 2's own (mainmenu.map's, with its
    /// button glyphs) for a controller, its keys for the keyboard.
    fn legend(&self, ctx: &Context) -> Option<String> {
        if self.editing {
            return Some("TYPE A GAMERTAG, THEN PRESS ENTER".into());
        }
        let signed_in = ctx.online.is_some_and(|o| o.live.is_some());
        let line = match self.screen {
            Screen::Start | Screen::Pregame | Screen::PostGame => return None,
            Screen::Main => self.button_key("a_select"),
            Screen::Confirm => self.button_key("a_select_b_cancel"),
            Screen::Pause => format!("{} \u{e101} RESUME", self.button_key("a_select")),
            Screen::Live if signed_in => {
                format!("{} \u{e101} SIGN OUT", self.button_key("a_select"))
            }
            Screen::Lobby if in_custom(ctx) => {
                format!("{} \u{e101} BACK TO THE PARTY", self.button_key("a_select"))
            }
            Screen::Lobby if ctx.host_lobby.is_some() => "\u{e101} LEAVE".into(),
            _ => self.button_key("a_select_b_back"),
        };
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if self.controller {
            return Some(line);
        }
        // The keys that do what the buttons do.
        let mut keys = String::new();
        for word in line.split(' ') {
            let key = match word {
                "\u{e100}" => "ENTER",
                "\u{e101}" => "ESC",
                w => w,
            };
            if key != word && !keys.is_empty() {
                keys.push_str("   ");
            } else if !keys.is_empty() {
                keys.push(' ');
            }
            keys.push_str(key);
        }
        Some(keys)
    }

    /// A button legend of mainmenu.map's, by name, or the remake's words
    /// for it.
    fn button_key(&self, name: &str) -> String {
        let ours = LEGENDS.iter().find(|l| l.0 == name).map_or("", |l| l.1);
        self.art
            .ui
            .globals
            .button_key(name)
            .unwrap_or(ours)
            .to_string()
    }

    /// A dialog screen's size, from its tag.
    fn dialog_size(&self, name: &str, otherwise: ui::DialogSize) -> ui::DialogSize {
        self.art.screen(name).map_or(otherwise, |s| s.dialog_size())
    }

    /// The carnage report's tabs, left to right: each pane's name and
    /// where it is (screen units), as wide as its name is drawn in frame
    /// `f` (wider in a small window, whose text keeps to `MIN_TEXT`).
    fn tabs(&self, ctx: &Context, f: &Frame) -> Vec<(String, [f32; 4])> {
        let size = f.text(9.0) / f.s;
        let mut x = REPORT_X;
        panes(ctx.scores)
            .into_iter()
            .map(|p| {
                let name = self.text.get(p.title());
                let width = name.chars().count() as f32 * size * crate::font::ASPECT + 12.0;
                let rect = [x, REPORT_TABS, x + width, REPORT_TABS + 16.0];
                x += width + 4.0;
                (name, rect)
            })
            .collect()
    }

    /// The carnage report: how the game ended, the panes' tabs (the one
    /// shown lit), and its pane of stats for each team or player, best
    /// first. Its columns are where Halo 2's carnage report screen puts
    /// them (`REPORT_COLUMNS`).
    fn draw_report(
        &self,
        hb: &mut HudBuilder,
        font: usize,
        white: usize,
        f: &Frame,
        ctx: &Context,
    ) {
        let s = f.s;
        let panes = panes(ctx.scores);
        let pane = panes[self.pane.min(panes.len() - 1)];
        for (k, (name, rect)) in self.tabs(ctx, f).iter().enumerate() {
            let (bg, fg) = if panes[k] == pane {
                (HIGHLIGHT, BRIGHT)
            } else {
                (PANEL, DIM)
            };
            hb.quad(white, f.rect(*rect), [0.0; 4], bg, hud_mode::PLAIN, 0.0);
            let size = f.text(9.0);
            let middle = f.at(0.0, (rect[1] + rect[3]) * 0.5)[1] - size * 0.5;
            hb.text_left(font, [f.at(rect[0] + 6.0, 0.0)[0], middle], size, name, fg);
        }
        if let Some(outcome) = ctx.outcome {
            let size = f.text(10.0);
            let width = outcome.chars().count() as f32 * size * crate::font::ASPECT;
            let at = [
                f.at(REPORT_RIGHT, 0.0)[0] - width,
                f.at(0.0, REPORT_TABS + 3.0)[1],
            ];
            hb.text_left(font, at, size, outcome, TEXT);
        }
        let header = f.text(8.0);
        let columns = report_columns(pane);
        let name = match pane {
            Pane::Teams => "werds/team",
            _ => "werds/player",
        };
        let at = f.at(REPORT_COLUMNS[0], REPORT_HEADER);
        hb.text_left(font, at, header, &self.text.get(name), DIM);
        for (heading, left) in self.report_headings(pane, f) {
            hb.text_left(font, f.at(left, REPORT_HEADER), header, &heading, DIM);
        }
        // Teams on their own pane, players on the rest; places by score.
        let teams = pane == Pane::Teams;
        let lines: Vec<&ScoreLine> = ctx.scores.iter().filter(|l| l.header == teams).collect();
        let size = f.text(9.0);
        let rows = ((REPORT_BOTTOM - REPORT_ROWS) / REPORT_STEP) as usize;
        for (k, line) in lines.iter().take(rows).enumerate() {
            let y = REPORT_ROWS + REPORT_STEP * k as f32;
            let color = line.color;
            let bg = if line.header {
                [color[0] * 0.6, color[1] * 0.6, color[2] * 0.6, 0.85]
            } else if line.local {
                [0.15, 0.3, 0.55, 0.75]
            } else {
                PANEL
            };
            let back = f.rect([REPORT_X, y, REPORT_RIGHT, y + REPORT_STEP - 2.0]);
            hb.quad(white, back, [0.0; 4], bg, hud_mode::PLAIN, 0.0);
            let middle = f.at(0.0, y + (REPORT_STEP - 2.0) * 0.5)[1] - size * 0.5;
            let mut name_x = REPORT_COLUMNS[0];
            if !line.header {
                // Their emblem (or colour) before their name.
                let badge = f.rect([name_x, y + 1.0, name_x + 11.0, y + 12.0]);
                hb.quad(
                    white,
                    badge,
                    [0.0; 4],
                    gamma_color(color),
                    hud_mode::PLAIN,
                    0.0,
                );
                if let Some(e) = line.emblem {
                    crate::emblem::draw(hb, badge, e);
                }
                name_x += 15.0;
            }
            let fg = if line.local || line.header {
                BRIGHT
            } else {
                TEXT
            };
            let room = (columns[0].1 - name_x - 4.0) * s;
            let at = [f.at(name_x, 0.0)[0], middle];
            hb.text_left(font, at, size, &fit(&line.name, room, size), fg);
            let place = 1 + lines.iter().filter(|o| o.score > line.score).count();
            for (k, (stat, x)) in columns.iter().enumerate() {
                let text = stat.value(line, place);
                let right = columns.get(k + 1).map_or(REPORT_RIGHT, |c| c.1);
                let text = fit(&text, (right - x - 6.0) * s, size);
                hb.text_left(font, [f.at(x + 4.0, 0.0)[0], middle], size, &text, fg);
            }
        }
    }

    /// The headings of a carnage report pane's stats and where each starts
    /// (screen units): indented like the values, unless (in a small window,
    /// whose text keeps to `MIN_TEXT`) that would run it into the next.
    fn report_headings(&self, pane: Pane, f: &Frame) -> Vec<(String, f32)> {
        let columns = report_columns(pane);
        let size = f.text(8.0) / f.s;
        columns
            .iter()
            .enumerate()
            .map(|(k, (stat, x))| {
                let heading = self.text.get(stat.heading());
                let width = heading.chars().count() as f32 * size * crate::font::ASPECT;
                let right = columns.get(k + 1).map_or(REPORT_RIGHT, |c| c.1);
                let left = (right - width - 1.0).clamp(*x, x + 4.0);
                (heading, left)
            })
            .collect()
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

    /// The screen's tag in mainmenu.map whose list this screen's rows are
    /// laid out by, if any.
    fn list_screen(&self) -> Option<&'static str> {
        match self.screen {
            Screen::Main => Some(menuart::MAIN_MENU),
            Screen::Options | Screen::Profile => Some(menuart::OPTIONS),
            Screen::SystemLink => Some(menuart::BROWSER),
            Screen::Confirm => Some(menuart::DIALOG),
            Screen::Pause => Some(menuart::LARGE_DIALOG),
            _ => None,
        }
    }

    /// The skin the screen's list is drawn in.
    fn list_skin(&self) -> Option<&ListSkin> {
        match self.list_screen() {
            Some(name) => self.art.list_skin(name),
            None => self.art.online_skin(),
        }
    }

    /// Where the screen's rows go: where its tag's list is, its items as
    /// far apart as its skin's are and each its skin's background; the
    /// tags' own numbers without the art. The profile's settings are
    /// narrowed for the model beside them, and the online lists start
    /// under the title.
    fn place(&self) -> Place {
        let (fallback, width) = match self.screen {
            Screen::Main => (MAIN_PLACE, None),
            Screen::Options => (OPTIONS_PLACE, None),
            Screen::Profile => (PROFILE_PLACE, Some(PROFILE_WIDTH)),
            Screen::SystemLink => (BROWSER_PLACE, None),
            Screen::Confirm => (DIALOG_PLACE, None),
            Screen::Pause => (PAUSE_PLACE, None),
            // Beside the party, or the player picked.
            Screen::Live | Screen::Player | Screen::Matchmaking => {
                (ONLINE_PLACE, Some(ONLINE_NARROW))
            }
            _ => (ONLINE_PLACE, None),
        };
        let skin = self.list_skin();
        let list = self
            .list_screen()
            .and_then(|n| self.art.screen(n))
            .and_then(|s| s.panes.first())
            .and_then(|p| p.lists.first());
        let mut place = fallback;
        if let (Some(skin), Some(item)) = (skin, skin.and_then(|s| self.art.item_box(s))) {
            place.step = self.art.item_height(skin);
            place.item = item;
        }
        if let Some(list) = list.filter(|_| self.screen != Screen::Profile) {
            place.corner = [list.corner[0] as f32, list.corner[1] as f32];
        }
        if let Some(w) = width {
            place.item[2] = place.item[0] + w;
        }
        place
    }

    /// Where a list's names and values go in its items (from an item's
    /// corner), and in what: its skin's first text for the name, and its
    /// second (the setting lists') for the value; otherwise the value is
    /// right-justified in the name's box. Narrowed items narrow them too.
    fn columns(&self, skin: Option<&ListSkin>, place: &Place) -> [([f32; 4], Style); 2] {
        let texts = skin.map_or(&[][..], |s| &s.texts[..]);
        let [l, t, r, b] = place.item;
        let label = match texts.first() {
            Some(t) => (bounds(t.bounds), Style::of(t, 1.0)),
            None => {
                let style = Style::new(Font::Body, ui::LEFT_JUSTIFY, TEXT);
                ([l + 10.0, t, r - 10.0, b], style)
            }
        };
        let value = match texts
            .get(1)
            .filter(|_| self.list_screen() == Some(menuart::OPTIONS))
        {
            Some(t) => (bounds(t.bounds), Style::of(t, 1.0)),
            None => {
                let [l, t, _, b] = label.0;
                let style = Style {
                    flags: ui::RIGHT_JUSTIFY,
                    ..label.1
                };
                ([l, t, r - 10.0, b], style)
            }
        };
        // Squeezed to the item's width.
        let full = skin
            .and_then(|s| self.art.item_box(s))
            .map_or(r - l, |b| b[2] - b[0]);
        let squeeze = (r - l) / full.max(1.0);
        let squeezed = |([bl, bt, br, bb], style): ([f32; 4], Style)| {
            let x = |v: f32| l + (v - l) * squeeze;
            ([x(bl), bt, x(br).min(r - 10.0), bb], style)
        };
        [squeezed(label), squeezed(value)]
    }

    /// How lit list item `k` is, and how far it has slid: its skin's
    /// animations as the cursor comes to it or leaves it, at rest
    /// otherwise; without a skin, lit or at half.
    fn item_look(&self, k: usize, cursor: usize, skin: Option<&ListSkin>) -> (f32, f32) {
        let Some(anims) = skin.map(|s| &s.item_animations).filter(|a| !a.is_empty()) else {
            return (if k == cursor { 1.0 } else { 0.5 }, 0.0);
        };
        let (seen, previous, at) = self.focus;
        let t = self.clock - at;
        let (alpha, [dx, _]) = if k == cursor {
            animate(anims.first(), if seen == cursor { t } else { SETTLED })
        } else if k == previous && seen == cursor {
            animate(anims.get(1), t)
        } else {
            animate(anims.get(2), SETTLED)
        };
        (alpha, dx)
    }

    /// How far a screen's list has come in (its alpha, up to 1, and how far
    /// it has yet to slide): its intro animation, from when the screen came up.
    fn list_fade(&self, screen: Option<&str>) -> (f32, [f32; 2]) {
        let list = screen
            .and_then(|n| self.art.screen(n))
            .and_then(|s| s.panes.first())
            .and_then(|p| p.lists.first());
        let Some(list) = list else {
            return (1.0, [0.0, 0.0]);
        };
        let anim = self.art.intro(list.animation);
        let t = self.clock - self.opened - list.delay_ms as f32 / 1000.0;
        let (alpha, slide) = animate(anim, t);
        (alpha / peak(anim), slide)
    }

    /// Where row `k` (of those shown) is, in menu units.
    fn row_box(&self, k: usize) -> [f32; 4] {
        match (self.screen, k) {
            (Screen::Lobby, 4) => self.lobby_button(0).0,
            (Screen::Lobby, 5) => self.lobby_button(1).0,
            (Screen::Lobby | Screen::Pregame, _) => self.lobby_line(k),
            _ => {
                let place = self.place();
                offset(place.item, place.corner(k))
            }
        }
    }

    /// One of pregame_lobby's texts, by name.
    fn lobby_text(&self, name: &str) -> Option<&ui::Text> {
        let pane = self.art.screen(menuart::LOBBY)?.panes.first()?;
        pane.texts.iter().find(|t| t.string == name)
    }

    /// The lobby's quick option line `k`: under "Quick Options:", each as
    /// tall as it.
    fn lobby_line(&self, k: usize) -> [f32; 4] {
        let [l, t, _, b] = self
            .lobby_text("gametype_options_format")
            .map_or(QUICK_OPTIONS, |t| bounds(t.bounds));
        let h = t - b;
        let top = b - h * k as f32;
        [l, top, LINE_RIGHT, top - h]
    }

    /// The lobby's button `k` (GAME OPTIONS, then START GAME), in two of
    /// pregame_lobby's buttons' places: the box of its picture, and of its
    /// text.
    fn lobby_button(&self, k: usize) -> ([f32; 4], [f32; 4]) {
        let tagged = self
            .art
            .screen(menuart::LOBBY)
            .and_then(|s| s.panes.first())
            .and_then(|p| p.buttons.get(k))
            .map(|b| {
                let size = b
                    .bitmap
                    .as_ref()
                    .and_then(|t| self.art.size(&button_bitmap(t)))
                    .unwrap_or(BUTTON_SIZE);
                let offset = [b.bitmap_offset[0] as f32, b.bitmap_offset[1] as f32];
                (bounds(b.text.bounds), offset, size)
            });
        let (text, [ox, oy], [w, h]) =
            tagged.unwrap_or((LOBBY_BUTTONS[k.min(1)], BUTTON_OFFSET, BUTTON_SIZE));
        let (x, y) = (text[0] + ox, text[1] - oy);
        ([x, y, x + w, y - h], text)
    }

    /// The map the lobby is on, if it's here.
    fn lobby_map<'c>(&self, ctx: &Context<'c>) -> Option<&'c MapChoice> {
        match ctx.host_lobby {
            Some(l) => ctx
                .maps
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case(&l.map)),
            None => ctx.maps.get(self.settings.map),
        }
    }

    /// The lobby, as Halo 2's pregame lobby: GAME OPTIONS and START GAME
    /// at the top left over its display (what's happening, and the map's
    /// description), the game type and map over the map's picture with the
    /// quick options under them, and everyone in the lobby down the right.
    /// In another PC's lobby, or a match's, it's all the host's to change.
    fn draw_lobby(&self, p: &mut Painter, ctx: &Context) {
        let age = self.clock - self.opened;
        let map = self.lobby_map(ctx);
        let picture = map.and_then(|m| m.picture);
        if let Some(pic) = picture {
            let size = self
                .lobby_bitmap("\\unknown_map")
                .and_then(|b| Some(([b.corner[0] as f32, b.corner[1] as f32], self.art.size(b)?)))
                .unwrap_or((MAP_PICTURE_AT, MAP_PICTURE_SIZE));
            let ([x, y], [w, h]) = size;
            let rect = p.sp.rect([x, y, x + w, y - h]);
            let full = [0.0, 0.0, 1.0, 1.0];
            p.hb.quad(
                MENU_TEXTURES + pic,
                rect,
                full,
                [1.0; 4],
                hud_mode::PLAIN,
                0.0,
            );
        }
        let keep = |b: &ui::Bitmap| {
            LOBBY_ART.iter().any(|n| is(b, n)) || is(b, "\\unknown_map") && picture.is_none()
        };
        p.screen(menuart::LOBBY, age, keep, None);
        if !self.art.loaded() {
            p.quad(LOBBY_PANEL, PANEL);
            p.quad(PLAYERS_PANEL, PANEL);
        }
        let fixed = self.screen == Screen::Pregame || ctx.host_lobby.is_some();
        let rows = self.rows(ctx);
        let cursor = self.settled(&rows, ctx);
        let white = Style::new(Font::Body, ui::LEFT_JUSTIFY, BRIGHT);
        let style = |name: &str| self.lobby_text(name).map_or(white, |t| Style::of(t, 1.0));
        let place = |name: &str, otherwise: [f32; 4]| {
            self.lobby_text(name)
                .map_or(otherwise, |t| bounds(t.bounds))
        };
        // The game type, and the map, over the quick options.
        let game_type = self.label(Row::GameType, ctx).1.unwrap_or_default();
        let gt = place("gametype_format", GAME_TYPE_LINE);
        p.line(
            gt,
            style("gametype_format"),
            &p.fit(white.font, &game_type, gt[2] - gt[0]),
        );
        let map_name = self.label(Row::Map, ctx).1.unwrap_or_default();
        let on = format!("ON {map_name}");
        let ml = place("mapname_format", MAP_LINE);
        p.line(
            ml,
            style("mapname_format"),
            &p.fit(white.font, &on, ml[2] - ml[0]),
        );
        let quick = self
            .lobby_text("gametype_options_format")
            .and_then(|t| t.text.clone())
            .unwrap_or("QUICK OPTIONS:".into())
            .to_uppercase();
        let quick_box = place("gametype_options_format", QUICK_OPTIONS);
        p.line(quick_box, style("gametype_options_format"), &quick);
        let mut status: Vec<(String, Option<String>)> = Vec::new();
        if self.screen == Screen::Pregame {
            status.extend(self.header(ctx).map(|h| (h, None)));
        }
        for (k, &row) in rows.iter().enumerate() {
            match row {
                Row::Waiting | Row::Starting => status.push(self.label(row, ctx)),
                Row::GameOptions | Row::StartGame if !fixed => {
                    let b = if row == Row::GameOptions { 0 } else { 1 };
                    let (picture, text) = self.lobby_button(b);
                    let lit = k == cursor;
                    let alpha = if lit { 1.0 } else { 0.55 };
                    let button = self
                        .art
                        .screen(menuart::LOBBY)
                        .and_then(|s| s.panes.first())
                        .and_then(|p| p.buttons.get(b));
                    match button.and_then(|b| b.bitmap.as_ref()) {
                        Some(t) => {
                            p.bitmap(&button_bitmap(t), [picture[0], picture[1]], alpha, 1.0)
                        }
                        None => p.quad(picture, glow(alpha * 0.6)),
                    }
                    let style = button.map_or(Style::new(Font::LargeBody, 0, BRIGHT), |b| {
                        Style::of(&b.text, 1.0)
                    });
                    let label = self.label(row, ctx).0;
                    p.line(text, style.alpha(alpha.max(TEXT_FLOOR)), &label);
                }
                _ => {
                    let line = self.lobby_line(k);
                    let lit = k == cursor && !fixed;
                    if lit {
                        p.quad([line[0] - 10.0, line[1], line[2], line[3]], glow(0.45));
                    }
                    // Each option and its value together, as Halo 2's quick
                    // options read, clear of the map's picture.
                    let text = match self.label(row, ctx) {
                        (label, Some(v)) if lit => format!("{label}: < {v} >"),
                        (label, Some(v)) => format!("{label}: {v}"),
                        (label, None) => label,
                    };
                    let color = if lit { BRIGHT } else { TEXT };
                    let style = Style::new(Font::Body, ui::LEFT_JUSTIFY, color);
                    let style = self.lobby_text("gametype_format").map_or(style, |t| Style {
                        font: t.font,
                        ..style
                    });
                    let text = p.fit(style.font, &text, line[2] - line[0]);
                    p.line(line, style, &text);
                }
            }
        }
        // The display: what's happening, then a notice or the map's
        // description.
        let mut top = LOBBY_STATUS[1];
        let [l, _, r, bottom] = LOBBY_STATUS;
        let big = Style::new(Font::LargeBody, ui::LEFT_JUSTIFY, BRIGHT);
        let step = p.line_height(big.font);
        for (name, value) in &status {
            let text = match value {
                Some(v) => format!("{name}  {v}"),
                None => name.clone(),
            };
            p.line([l, top, r, top - step], big, &p.fit(big.font, &text, r - l));
            top -= step;
        }
        let about = [l, top.min(bottom) - 10.0, r, LOBBY_ABOUT_BOTTOM];
        let body = Style::new(Font::Body, ui::LEFT_JUSTIFY, TEXT);
        match (&self.notice, map) {
            (Some(n), _) => {
                p.paragraph(
                    about,
                    Style {
                        color: WARNING,
                        ..body
                    },
                    n,
                );
            }
            (None, Some(m)) if !m.description.is_empty() => {
                p.paragraph(about, body, &m.description.to_uppercase());
            }
            _ => {}
        }
        if self.screen == Screen::Pregame {
            draw_roster(p, ctx);
        } else {
            self.draw_players(p, ctx);
        }
    }

    /// One of pregame_lobby's bitmaps, by the end of its tag's name.
    fn lobby_bitmap(&self, name: &str) -> Option<&ui::Bitmap> {
        let pane = self.art.screen(menuart::LOBBY)?.panes.first()?;
        pane.bitmaps.iter().find(|b| is(b, name))
    }

    /// Everyone in the lobby, down its right as Halo 2's pregame lobby
    /// lists them: their team's colour (or their own), emblem, gamertag,
    /// how they play and rank; then the bots, and how to join in.
    fn draw_players(&self, p: &mut Painter, ctx: &Context) {
        let (teams, bots) = match ctx.host_lobby {
            Some(l) => (l.teams, l.bots as usize),
            None => (self.settings.game_type().teams(), self.settings.bots),
        };
        let [l, t, r, _] = ROSTER;
        let count = match ctx.seats.len() {
            1 => "1 PLAYER".to_string(),
            n => format!("{n} PLAYERS"),
        };
        let head = Style::new(Font::LargeBody, ui::LEFT_JUSTIFY, BRIGHT);
        p.line([l, t, r, t - ROSTER_HEAD], head, &count);
        let mut y = t - ROSTER_HEAD;
        for seat in ctx.seats {
            let c = match (teams, seat.team) {
                // The host puts them on a team when the game starts.
                (true, NO_TEAM) => UNPICKED,
                (true, t) => crate::local::TEAM_COLORS[t.min(1) as usize],
                (false, _) => crate::local::armor_colors(seat.look)[0],
            };
            seat_row(p, seat, c, [l, y, r, y - PLAYER_STEP + 3.0], Some(seat.how));
            y -= PLAYER_STEP;
        }
        let mut lines = Vec::new();
        if bots > 0 {
            lines.push(format!("+ {bots} BOT{}", if bots == 1 { "" } else { "S" }));
        }
        // Then, a little apart, how to join in.
        let hints_from = lines.len();
        if teams {
            lines.push("T OR X: CHANGE TEAM".into());
        }
        // Player one at the keyboard here: a controller can take over.
        let keyboard =
            ctx.host_lobby.is_none() && ctx.seats.first().is_some_and(|p| p.how == KEYBOARD);
        let hints = match keyboard {
            true => [
                "A ON A CONTROLLER: PLAY AS PLAYER ONE",
                "START ON ANOTHER: PLAY IN SPLITSCREEN",
            ],
            false => ["PRESS START ON A CONTROLLER", "TO PLAY IN SPLITSCREEN"],
        };
        if ctx.local < crate::MAX_LOCAL {
            lines.extend(hints.map(String::from));
        }
        let small = Style::new(Font::Body, ui::LEFT_JUSTIFY, DIM);
        let step = p.line_height(small.font);
        for (k, text) in lines.iter().enumerate() {
            if k == hints_from {
                y -= 10.0;
            }
            p.line([l, y, r, y - step], small, text);
            y -= step;
        }
    }

    /// The profile's emblem, at the top right over the model (which stands
    /// in the framing's window, drawn with the level).
    fn draw_profile(&self, p: &mut Painter) {
        let [l, t, r, b] = EMBLEM_BOX;
        p.quad([l - 12.0, t + 12.0, r + 12.0, b - 50.0], PANEL);
        let rect = p.sp.rect(EMBLEM_BOX);
        crate::emblem::draw(p.hb, rect, self.profile.look.emblem);
        let style = Style::new(Font::Body, 0, DIM);
        p.line([l, b - 5.0, r, b - 45.0], style, "EMBLEM");
    }

    /// System Link's games, as Halo 2's network game browser: each one's
    /// host, map, players and whether it can be joined in its columns, and
    /// the chosen one's map, its picture and description; or that there are
    /// none.
    fn draw_browser(&self, p: &mut Painter, ctx: &Context) {
        let age = self.clock - self.opened;
        let rows = self.rows(ctx);
        let cursor = self.settled(&rows, ctx);
        let chosen = match rows.get(cursor) {
            Some(&Row::Join(i)) => ctx.lan.get(i),
            _ => None,
        };
        let map = chosen.and_then(|g| {
            ctx.maps
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case(&g.map))
        });
        let picture = map.and_then(|m| m.picture);
        let pane = self
            .art
            .screen(menuart::BROWSER)
            .and_then(|s| s.panes.first());
        let keep = |b: &ui::Bitmap| {
            !is(b, "\\live_icons_sm") && !(is(b, "\\unknown_map") && picture.is_some())
        };
        p.screen(menuart::BROWSER, age, keep, None);
        if let Some(pic) = picture {
            let place = pane
                .and_then(|pane| pane.bitmaps.iter().find(|b| is(b, "\\unknown_map")))
                .and_then(|b| Some(([b.corner[0] as f32, b.corner[1] as f32], self.art.size(b)?)));
            let ([x, y], [w, h]) = place.unwrap_or((BROWSER_PICTURE_AT, BROWSER_PICTURE_SIZE));
            let rect = p.sp.rect([x, y, x + w, y - h]);
            let full = [0.0, 0.0, 1.0, 1.0];
            p.hb.quad(
                MENU_TEXTURES + pic,
                rect,
                full,
                [1.0; 4],
                hud_mode::PLAIN,
                0.0,
            );
        }
        let text = |name: &str| pane.and_then(|p| p.texts.iter().find(|t| t.string == name));
        // The columns' heads (there's no game type or variant to show).
        for (name, head, at) in BROWSER_HEADS {
            let (box_, style) = match text(name) {
                Some(t) => (bounds(t.bounds), Style::of(t, 1.0)),
                None => (
                    at,
                    Style::new(Font::SplitHudMessage, ui::LEFT_JUSTIFY, TEXT),
                ),
            };
            p.line(box_, style, head);
        }
        if ctx.lan.is_empty() {
            let (box_, style, line) = match text("no_games") {
                Some(t) => {
                    let line = t.text.clone().unwrap_or_default().to_uppercase();
                    (bounds(t.bounds), Style::of(t, 1.0), line)
                }
                None => (
                    NO_GAMES,
                    Style::new(Font::SplitHudMessage, 0, TEXT),
                    NO_GAMES_TEXT.into(),
                ),
            };
            p.line(box_, style, &line);
            let searching = self.label(Row::Searching, ctx).0;
            let below = [
                box_[0],
                box_[3] - 10.0,
                box_[2],
                box_[3] - 10.0 - (box_[1] - box_[3]),
            ];
            p.line(
                below,
                Style {
                    color: DIM,
                    ..style
                },
                &searching,
            );
        } else {
            self.draw_games(p, ctx, &rows, cursor);
        }
        // The help display: a notice, or what's chosen.
        let help = text("help_create_game").map_or(BROWSER_HELP, |t| bounds(t.bounds));
        let body = Style::new(Font::Body, ui::LEFT_JUSTIFY, TEXT);
        if let Some(n) = &self.notice {
            p.paragraph(
                help,
                Style {
                    color: WARNING,
                    ..body
                },
                n,
            );
        } else if let Some(g) = chosen {
            let about = match map {
                Some(m) if !m.description.is_empty() => m.description.to_uppercase(),
                _ => self.header(ctx).unwrap_or_default(),
            };
            let title = format!("{} ON {}", g.computer.to_uppercase(), map_title(&g.map));
            let step = p.line_height(body.font);
            let [l, t, r, b] = help;
            p.line(
                [l, t, r, t - step],
                Style {
                    color: BRIGHT,
                    ..body
                },
                &p.fit(body.font, &title, r - l),
            );
            p.paragraph([l, t - step, r, b], body, &about);
        } else {
            p.paragraph(help, body, &self.header(ctx).unwrap_or_default());
        }
    }

    /// System Link's games in the browser's list skin: each one's host,
    /// map, players and whether it can be joined, in their columns.
    fn draw_games(&self, p: &mut Painter, ctx: &Context, rows: &[Row], cursor: usize) {
        let skin = self.list_skin();
        let place = self.place();
        let (fade, [fx, fy]) = self.list_fade(Some(menuart::BROWSER));
        let shown = self.shown(rows.len());
        let first = shown.start;
        for (k, &row) in rows.iter().enumerate().take(shown.end).skip(first) {
            let Row::Join(i) = row else {
                continue;
            };
            let g = &ctx.lan[i];
            let (look, dx) = self.item_look(k, cursor, skin);
            let [x, y] = place.corner(k - first);
            let corner = [x + dx + fx, y + fy];
            match skin {
                Some(skin) => p.item(skin, corner, look * fade, 1.0, |b| !is(b, "\\null")),
                None => p.quad(offset(place.item, corner), glow(look * fade * 0.6)),
            }
            let status = if g.protocol != h2net::PROTOCOL {
                "ANOTHER VERSION".to_string()
            } else {
                "JOIN GAME".to_string()
            };
            let cells = [
                g.computer.to_uppercase(),
                map_title(&g.map),
                format!("{}/16", g.players),
                status,
            ];
            let texts = skin.map_or(&[][..], |s| &s.texts[..]);
            for (k, cell) in cells.iter().enumerate() {
                let column = BROWSER_COLUMNS[k];
                let (box_, style) = match texts.get(column.0) {
                    Some(t) => (bounds(t.bounds), Style::of(t, 1.0)),
                    None => (
                        column.1,
                        Style::new(Font::SplitHudMessage, ui::LEFT_JUSTIFY, TEXT),
                    ),
                };
                let style = style.alpha(look.max(TEXT_FLOOR) * fade);
                let cell = p.fit(style.font, cell, box_[2] - box_[0]);
                p.line(offset(box_, corner), style, &cell);
            }
        }
        if shown.len() < rows.len() {
            let [x, y] = place.corner(shown.len());
            let at = offset(place.item, [x, y]);
            let text = format!("{}-{} OF {}", shown.start + 1, shown.end, rows.len());
            let style = Style::new(Font::SplitHudMessage, ui::RIGHT_JUSTIFY, DIM);
            p.line(at, style, &text);
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

/// Most players the pregame lists in one column; more go in two, a team
/// in each in team games.
const ROSTER_ROWS: usize = 8;

/// One of the lobby's players in a row (menu units): their colour,
/// emblem, gamertag, how they play (if shown) and rank; long gamertags
/// get smaller, then cut, to fit beside the rank.
fn seat_row(p: &mut Painter, seat: &SeatInfo, color: [f32; 3], row: [f32; 4], how: Option<&str>) {
    let [l, t, r, b] = row;
    // The emblem and rank, in the middle of the row.
    let h = (t - b).min(SEAT_ICON);
    let [top, bottom] = [(t + b + h) * 0.5, (t + b - h) * 0.5];
    p.quad(row, [PANEL[0], PANEL[1], PANEL[2], 0.45]);
    p.quad([l, t, l + 8.0, b], gamma_color(color));
    let badge = p.sp.rect([l + 14.0, top, l + 14.0 + h, bottom]);
    crate::emblem::draw(p.hb, badge, seat.look.emblem);
    let mut right = r - 6.0;
    if let Some(level) = seat.level {
        let icon = p.sp.rect([r - h - 2.0, top + 1.0, r - 2.0, bottom - 1.0]);
        rank::draw(p.hb, icon, level);
        right -= h + 4.0;
    }
    if let Some(how) = how {
        let style = Style::new(Font::SplitHudMessage, ui::RIGHT_JUSTIFY, DIM);
        p.line([l, t, right, b], style, how);
        right -= p.width(style.font, how) + 16.0;
    }
    let x = l + 20.0 + h;
    let style = Style::new(Font::Body, ui::LEFT_JUSTIFY, TEXT);
    p.line_to_fit([x, t, right.max(x), b], style, &seat.name, 0.5);
}

/// Everyone in an online match, down the right of its pregame lobby:
/// their team's colour (or their own), emblem, gamertag and rank, large;
/// in two columns (a team in each in team games) when there are many.
fn draw_roster(p: &mut Painter, ctx: &Context) {
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
    let [l, t, r, b] = ROSTER;
    let head = Style::new(Font::LargeBody, ui::LEFT_JUSTIFY, BRIGHT);
    let count = format!("{} PLAYERS", seats.len());
    p.line([l, t, r, t - ROSTER_HEAD], head, &count);
    let top = t - ROSTER_HEAD;
    // Rows in two columns are as tall as in a full one.
    let longest = columns.iter().map(Vec::len).max().unwrap_or(0);
    let n = longest.max(if columns.len() > 1 { ROSTER_ROWS } else { 1 }) as f32;
    let step = ((top - b) / n).clamp(PLAYER_STEP, ROSTER_STEP);
    let width = (r - l + ROSTER_GAP) / columns.len() as f32 - ROSTER_GAP;
    for (k, column) in columns.iter().enumerate() {
        let x = l + (width + ROSTER_GAP) * k as f32;
        for (i, seat) in column.iter().enumerate() {
            let c = if teams {
                crate::local::TEAM_COLORS[seat.team.min(1) as usize]
            } else {
                crate::local::armor_colors(seat.look)[0]
            };
            let y = top - step * i as f32;
            seat_row(p, seat, c, [x, y, x + width, y - step + 4.0], None);
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

/// A panel of the remake's (menu units): Halo 2's navy, with a light rim.
fn panel(p: &mut Painter, rect: [f32; 4]) {
    p.quad(rect, PANEL);
    p.rim(rect, 0.6);
}

/// The party beside the online lobby: the leader's crown, then each
/// member's emblem, gamertag (with their splitscreen guests) and level: in
/// the ranked playlist the party searches, plays or just played, as Halo 2
/// showed new levels back in the lobby, otherwise their best.
fn draw_party(p: &mut Painter, o: &OnlineView) {
    let Some(party) = o.party() else {
        return;
    };
    let [l, t, r, _] = SIDE_PANEL;
    let head = Style::new(Font::LargeBody, ui::LEFT_JUSTIFY, BRIGHT);
    let step = SIDE_STEP;
    let bottom = t - ROSTER_HEAD - step * party.members.len() as f32 - 20.0;
    panel(p, [l - 20.0, t + 20.0, r + 20.0, bottom]);
    p.line([l, t, r, t - ROSTER_HEAD], head, "PARTY");
    let count = format!("{}/{}", o.party_size(), live::MAX_PARTY);
    let dim = Style::new(Font::Body, ui::RIGHT_JUSTIFY, DIM);
    p.line([l, t, r, t - ROSTER_HEAD], dim, &count);
    let mut y = t - ROSTER_HEAD;
    for m in &party.members {
        let h = step - 6.0;
        if m.account == party.leader {
            let crown = p.sp.rect([l, y, l + h, y - h]);
            rank::draw_live(p.hb, crown, LiveIcon::Leader);
        }
        let badge = p.sp.rect([l + h + 8.0, y, l + 2.0 * h + 8.0, y - h]);
        crate::emblem::draw(p.hb, badge, m.look.emblem);
        let name = match m.guests {
            0 => m.gamertag.clone(),
            n => format!("{} +{n}", m.gamertag),
        };
        let fg = if m.account == o.me() { BRIGHT } else { TEXT };
        let x = l + 2.0 * h + 24.0;
        let style = Style::new(Font::Body, ui::LEFT_JUSTIFY, fg);
        let name = p.fit(style.font, &name, r - h - 12.0 - x);
        p.line([x, y, r - h - 12.0, y - h], style, &name);
        let icon = p.sp.rect([r - h, y + 1.0, r, y - h - 1.0]);
        rank::draw(p.hb, icon, m.level);
        y -= step;
    }
}

/// The player picked (by account), beside what can be done about them:
/// their level, gamertag and what they're doing (or that they're offline),
/// and what we last played with them if we have lately.
fn draw_player(p: &mut Painter, o: &OnlineView, account: u64) {
    let recent = o.recent_player(account);
    let (level, gamertag) = match (o.player(account), recent) {
        (Some(p), _) => (p.best, &p.gamertag),
        (None, Some(r)) => (r.level, &r.gamertag),
        (None, None) => return,
    };
    let [l, t, r, _] = SIDE_PANEL;
    let icon = 70.0;
    let x = l + icon + 20.0;
    let body = Style::new(Font::Body, ui::LEFT_JUSTIFY, TEXT);
    let mut lines = p.wrap(body.font, &o.status(account), r - x);
    if let Some(r) = recent {
        let played = format!("LAST PLAYED {}", o.last_played(r));
        lines.extend(p.wrap(body.font, &played, SIDE_PANEL[2] - x));
    }
    let step = p.line_height(body.font);
    let head = Style::new(Font::LargeBody, ui::LEFT_JUSTIFY, BRIGHT);
    let height = (p.line_height(head.font) + step * lines.len() as f32).max(icon);
    panel(p, [l - 20.0, t + 20.0, r + 20.0, t - height - 20.0]);
    rank::draw(p.hb, p.sp.rect([l, t, l + icon, t - icon]), level);
    let top = t - p.line_height(head.font);
    p.line([x, t, r, top], head, &p.fit(head.font, gamertag, r - x));
    for (k, line) in lines.iter().enumerate() {
        let y = top - step * k as f32;
        p.line([x, y, r, y - step], body, line);
    }
}

/// The icons on the online lists' rows (an item's box, and where its name
/// ends): a player's level (as it was when we played, for recent players
/// offline), then their party's if they're online (a crown for our
/// party's leader); the level we have in a playlist.
fn draw_icons(p: &mut Painter, row: Row, item: [f32; 4], label_end: f32, o: &OnlineView) {
    let [x0, y0, x1, y1] = item;
    let mid = (y0 + y1) * 0.5;
    let half = ICON * 0.5;
    match row {
        Row::Player(_) | Row::RecentPlayer(_) => {
            let (level, player) = match row {
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
            let at =
                p.sp.rect([x0 + 20.0, mid + half, x0 + 20.0 + ICON, mid - half]);
            rank::draw(p.hb, at, level);
            let Some(player) = player else {
                return;
            };
            let leads = o
                .party()
                .is_some_and(|party| party.leader == player.account);
            let icon = if leads {
                LiveIcon::Leader
            } else if player.size > 1 {
                LiveIcon::Party
            } else {
                return;
            };
            let x = label_end + 12.0;
            rank::draw_live(p.hb, p.sp.rect([x, mid + half, x + ICON, mid - half]), icon);
        }
        Row::Playlist(i) => {
            let Some(pl) = o.playlists().get(i) else {
                return;
            };
            let rect =
                p.sp.rect([x1 - ICON - 16.0, mid + half, x1 - 16.0, mid - half]);
            if !o.missing_maps(pl).is_empty() {
                rank::draw_live(p.hb, rect, LiveIcon::Download);
            } else if pl.level > 0 {
                rank::draw(p.hb, rect, pl.level);
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

/// A mission's objectives, in the pause menu's dialog: done ones dimmed.
fn draw_objectives(p: &mut Painter, box_: [f32; 4], style: Style, objectives: &[(String, bool)]) {
    let [l, t, r, b] = box_;
    let head = Style {
        font: Font::LargeBody,
        color: BRIGHT,
        ..style
    };
    let mut y = t;
    let step = p.line_height(head.font);
    p.line([l, y, r, y - step], head, "OBJECTIVES");
    y -= step;
    let step = p.line_height(style.font);
    for (text, done) in objectives {
        let mark = if *done { "+" } else { "-" };
        let color = if *done { DIM } else { TEXT };
        let lines = p.wrap(style.font, &text.to_uppercase(), r - l - 30.0);
        for (k, line) in lines.iter().enumerate() {
            if y - step < b {
                return;
            }
            let row = Style { color, ..style };
            if k == 0 {
                p.line([l, y, l + 30.0, y - step], row, mark);
            }
            p.line([l + 30.0, y, r, y - step], row, line);
            y -= step;
        }
    }
}

/// Most lines the scoreboard shows: 16 players and two team totals.
const MAX_SCORE_LINES: usize = 18;
/// The scoreboard's widest (screen units), as it was drawn in a full
/// window.
const BOARD_W: f32 = 514.0;

/// What the scoreboard over a game shows.
pub struct Scoreboard<'a> {
    pub lines: &'a [ScoreLine],
    /// Its columns' headings (`MenuText::score_headings`).
    pub headings: &'a [String; 6],
    /// Once the game is over: GAME OVER, and how it went for the player
    /// whose view it's in ("YOU WIN!").
    pub over: Option<(&'a str, &'a str)>,
}

/// The scoreboard over a game (held Tab or Back, and as the game ends) in
/// a view `w` by `h` pixels, best first: Halo 2's columns (place, name,
/// score, kills, assists, deaths). Its text is never under `MIN_TEXT`
/// pixels, so in a small splitscreen view long names are cut and the
/// rows that don't fit are left off rather than shrunk. Places go to
/// teams when there are team totals, otherwise to players; levels show
/// when there are any.
pub fn draw_scoreboard(
    hb: &mut HudBuilder,
    font: usize,
    white: usize,
    w: f32,
    h: f32,
    board: &Scoreboard,
) {
    let f = Frame::readable(w, h);
    let s = f.s;
    let [left, top, right, bottom] = f.shown(w, h);
    let width = (right - left - 8.0).min(BOARD_W);
    let x0 = 320.0 - width * 0.5;
    let x1 = x0 + width;
    let title = if board.over.is_some() { 22.0 } else { 0.0 };
    // Rows close up to fit the view, then the last are left off.
    let n = board.lines.len().min(MAX_SCORE_LINES);
    let room = bottom - top - 8.0 - (12.0 + title + 16.0 + 4.0);
    let step = (room / n.max(1) as f32).clamp(11.0, 16.0);
    let n = n.min((room / step).max(0.0) as usize);
    let height = 12.0 + title + 16.0 + step * n as f32 + 4.0;
    // Below the HUD's top row where there's room (the view's top 70 units),
    // otherwise as high as it fits.
    let y0 = (top + 4.0).max((240.0 - height * 0.5).min(top + 70.0));
    let back = f.rect([x0 - 8.0, y0, x1 + 8.0, y0 + height]);
    hb.quad(
        white,
        back,
        [0.0; 4],
        [0.0, 0.0, 0.02, 0.7],
        hud_mode::PLAIN,
        0.0,
    );
    let mut y = y0 + 12.0;
    if let Some((over, how)) = board.over {
        hb.text_left(font, f.at(x0, y), f.text(14.0), over, BRIGHT);
        let size = f.text(10.0);
        let width = how.chars().count() as f32 * size * crate::font::ASPECT;
        let at = [f.at(x1, 0.0)[0] - width, f.at(0.0, y + 2.0)[1]];
        hb.text_left(font, at, size, how, TEXT);
        y += title;
    }
    // The columns: place, emblem and name, level, then from the right
    // deaths, assists, kills and score.
    let place_x = x0 + 4.0;
    let badge_x = x0 + 36.0;
    let name_x = x0 + 54.0;
    let stats = [x1 - 172.0, x1 - 128.0, x1 - 88.0, x1 - 40.0];
    let levels = board.lines.iter().any(|l| l.level.is_some());
    let level_x = stats[0] - 34.0;
    let header = f.text(8.0);
    let h = &board.headings;
    hb.text_left(font, f.at(place_x, y), header, &h[0], DIM);
    hb.text_left(font, f.at(name_x, y), header, &h[1], DIM);
    for (x, heading) in stats.iter().zip(&h[2..]) {
        hb.text_left(font, f.at(*x, y), header, heading, DIM);
    }
    if levels {
        hb.text_left(font, f.at(level_x, y), header, "LEVEL", DIM);
    }
    y += 16.0;
    let teams = board.lines.iter().any(|l| l.header);
    let ranked: Vec<&ScoreLine> = board.lines.iter().filter(|l| l.header == teams).collect();
    let rank_of =
        |line: &ScoreLine| -> usize { 1 + ranked.iter().filter(|o| o.score > line.score).count() };
    let size = f.text(9.0);
    let name_room = (if levels { level_x } else { stats[0] } - name_x - 6.0) * s;
    for line in board.lines.iter().take(n) {
        let c = line.color;
        let bg = if line.header {
            [c[0] * 0.6, c[1] * 0.6, c[2] * 0.6, 0.85]
        } else if line.local {
            [0.15, 0.3, 0.55, 0.75]
        } else {
            PANEL
        };
        let row = f.rect([x0 - 4.0, y - 2.0, x1 + 4.0, y + step - 3.0]);
        hb.quad(white, row, [0.0; 4], bg, hud_mode::PLAIN, 0.0);
        let middle = f.at(0.0, y + (step - 5.0) * 0.5)[1] - size * 0.5;
        if !line.header {
            // Their emblem, or (without the emblem pictures) their colour.
            let badge = f.rect([badge_x, y - 1.0, badge_x + 12.0, y + 11.0]);
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
            let at = [f.at(place_x, 0.0)[0], middle];
            hb.text_left(font, at, size, &place(rank_of(line)), fg);
        }
        let name = fit(&line.name, name_room, size);
        hb.text_left(font, [f.at(name_x, 0.0)[0], middle], size, &name, fg);
        let values = [
            crate::local::score_text(line.score, line.timed),
            line.kills.to_string(),
            line.assists.to_string(),
            line.deaths.to_string(),
        ];
        for (x, v) in stats.iter().zip(&values) {
            hb.text_left(font, [f.at(*x, 0.0)[0], middle], size, v, fg);
        }
        y += step;
    }
}

/// `text` cut to fit `room` window pixels in text `size` pixels tall,
/// marked ".." where it was cut.
fn fit(text: &str, room: f32, size: f32) -> String {
    let fits = (room / (size * crate::font::ASPECT)).max(0.0) as usize;
    if text.chars().count() <= fits {
        return text.to_string();
    }
    let kept: String = text.chars().take(fits.saturating_sub(2)).collect();
    format!("{}..", kept.trim_end())
}

/// The carnage report's layout (screen units): its left and right edges,
/// its tabs, its column headings, its first row and the step between
/// rows, and where its rows end (CONTINUE goes there).
const REPORT_X: f32 = 64.0;
const REPORT_RIGHT: f32 = 577.0;
const REPORT_TABS: f32 = 92.0;
const REPORT_HEADER: f32 = 116.0;
const REPORT_ROWS: f32 = 130.0;
const REPORT_STEP: f32 = 15.0;
const REPORT_BOTTOM: f32 = 376.0;
/// The carnage report's columns' left edges: the name, then four stats.
/// They are its screen's header bounds in mainmenu.map, halved to
/// 640x480 (-500, -135, 31, 193 and 355 from the middle of 1280, which
/// end at 514).
const REPORT_COLUMNS: [f32; 5] = [70.0, 252.5, 335.5, 416.5, 497.5];
/// TEAM STATS' place and score, and MEDALS' medals earned (its total
/// takes the first stat column and the next).
const REPORT_TEAM_COLUMNS: [f32; 2] = [302.5, 442.5];
const REPORT_MEDALS: f32 = 416.5;

/// A carnage report column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stat {
    Place,
    Score,
    AvgLife,
    BestSpree,
    Kills,
    Assists,
    Deaths,
    Suicides,
    TotalMedals,
    MedalsEarned,
}

impl Stat {
    /// Its heading, in `MenuText`.
    fn heading(self) -> &'static str {
        match self {
            Stat::Place => "werds/place",
            Stat::Score => "werds/score",
            Stat::AvgLife => "werds/avg_life",
            Stat::BestSpree => "werds/best_spree",
            Stat::Kills => "werds/kills",
            Stat::Assists => "werds/assists",
            Stat::Deaths => "werds/deaths",
            Stat::Suicides => "werds/suicides",
            Stat::TotalMedals => "werds/total_medals",
            Stat::MedalsEarned => "werds/medals_earned",
        }
    }

    /// What it says for a line in `place` (1 for first).
    fn value(self, l: &ScoreLine, place: usize) -> String {
        match self {
            Stat::Place => self::place(place),
            Stat::Score => crate::local::score_text(l.score, l.timed),
            // Minutes and seconds, as Halo 2's "%d:%02d".
            Stat::AvgLife => crate::local::score_text(l.avg_life.round() as i32, true),
            Stat::BestSpree => l.best_spree.to_string(),
            Stat::Kills => l.kills.to_string(),
            Stat::Assists => l.assists.to_string(),
            Stat::Deaths => l.deaths.to_string(),
            Stat::Suicides => l.suicides.to_string(),
            Stat::TotalMedals => l.medals.iter().map(|m| m.1 as u32).sum::<u32>().to_string(),
            // By name: the medals' pictures are a font the maps don't have.
            Stat::MedalsEarned => {
                let names: Vec<String> = l
                    .medals
                    .iter()
                    .map(|(name, n)| match n {
                        1 => name.clone(),
                        n => format!("{name} x{n}"),
                    })
                    .collect();
                names.join(", ")
            }
        }
    }
}

/// A carnage report pane's columns after the name: what each shows, and
/// its left edge.
fn report_columns(pane: Pane) -> Vec<(Stat, f32)> {
    let c = REPORT_COLUMNS;
    match pane {
        Pane::Teams => vec![
            (Stat::Place, REPORT_TEAM_COLUMNS[0]),
            (Stat::Score, REPORT_TEAM_COLUMNS[1]),
        ],
        Pane::Players => vec![
            (Stat::Place, c[1]),
            (Stat::AvgLife, c[2]),
            (Stat::BestSpree, c[3]),
            (Stat::Score, c[4]),
        ],
        Pane::Kills => vec![
            (Stat::Kills, c[1]),
            (Stat::Assists, c[2]),
            (Stat::Deaths, c[3]),
            (Stat::Suicides, c[4]),
        ],
        Pane::Medals => vec![
            (Stat::TotalMedals, c[1]),
            (Stat::MedalsEarned, REPORT_MEDALS),
        ],
    }
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
        // QUIT asks first.
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert_eq!(m.screen, Screen::Confirm);
        m.input(Input::Up, &c);
        assert_eq!(m.input(Input::Select, &c), Action::Quit);
    }

    #[test]
    fn quitting_ending_and_leaving_a_game_ask_first() {
        let maps = maps();
        let c = ctx(&maps, &[]);
        let mut m = Menu::new(Settings::default(), Profile::default());
        // QUIT on the main menu: the dialog starts on its answer that
        // does nothing, which takes it back where it was.
        m.cursor = MAIN_ROWS.len() - 1;
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert_eq!(m.screen, Screen::Confirm);
        assert_eq!(m.title(&c), "EXIT HALO 2 ?");
        assert_eq!(m.label(Row::Yes, &c).0, "EXIT HALO 2");
        assert_eq!(m.rows(&c)[m.cursor], Row::No);
        assert_eq!(m.label(Row::No, &c).0, "NO");
        assert_eq!(m.input(Input::Down, &c), Action::None);
        assert_eq!(m.rows(&c)[m.cursor], Row::No, "no wrapping round to yes");
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert_eq!(m.screen, Screen::Main);
        assert_eq!(m.rows(&c)[m.cursor], Row::Quit);
        assert_eq!(m.sound, Some(Sound::Back));
        // Esc (or B) says no too.
        m.input(Input::Select, &c);
        assert_eq!(m.input(Input::Back, &c), Action::None);
        assert_eq!(m.screen, Screen::Main);

        // END GAME on the pause menu.
        m.show(Screen::Pause);
        m.input(Input::Down, &c);
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert_eq!(m.title(&c), "ARE YOU SURE ?");
        assert_eq!(m.label(Row::Yes, &c).0, "END GAME");
        assert_eq!(m.label(Row::No, &c).0, "CANCEL");
        assert!(m.pausing(), "the game stays paused while it asks");
        assert_eq!(m.input(Input::Back, &c), Action::None);
        assert_eq!(m.screen, Screen::Pause);
        assert_eq!(m.rows(&c)[m.cursor], Row::EndGame);
        m.input(Input::Select, &c);
        m.input(Input::Up, &c);
        assert_eq!(m.input(Input::Select, &c), Action::EndGame);
        // QUIT from the pause menu.
        m.show(Screen::Pause);
        m.input(Input::Down, &c);
        m.input(Input::Down, &c);
        m.input(Input::Select, &c);
        assert_eq!(m.title(&c), "EXIT HALO 2 ?");
        m.input(Input::Up, &c);
        assert_eq!(m.input(Input::Select, &c), Action::Quit);

        // Joined to another PC's game: LEAVE GAME.
        let joined = Context {
            joined: true,
            ..ctx(&maps, &[])
        };
        m.show(Screen::Pause);
        m.input(Input::Down, &joined);
        assert_eq!(m.label(Row::EndGame, &joined).0, "LEAVE GAME");
        m.input(Input::Select, &joined);
        assert_eq!(m.title(&joined), "LEAVE GAME ?");
        assert_eq!(m.label(Row::Yes, &joined).0, "LEAVE GAME");
        m.input(Input::Up, &joined);
        assert_eq!(m.input(Input::Select, &joined), Action::Leave);
        // A dialog's text is readable over a small splitscreen view.
        m.show(Screen::Pause);
        m.input(Input::Down, &c);
        m.input(Input::Select, &c);
        assert_eq!(smallest_text(|hb| m.draw(hb, 0, 1, 320.0, 180.0, &c)), 8.0);
    }

    /// The smallest text (glyphs' height, in pixels) `draw` draws with
    /// the font texture 0.
    fn smallest_text(draw: impl FnOnce(&mut HudBuilder)) -> f32 {
        let mut hb = HudBuilder::new(1280.0, 720.0);
        draw(&mut hb);
        hb.finish()
            .iter()
            .filter(|b| b.texture == 0)
            .flat_map(|b| b.vertices.chunks(6))
            .map(|q| {
                let ys = q.iter().map(|v| v.position[1]);
                let (lo, hi) = ys.fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
                hi - lo
            })
            .fold(f32::MAX, f32::min)
    }

    fn report_lines() -> Vec<ScoreLine> {
        let player = |name: &str, score, local| ScoreLine {
            name: name.into(),
            score,
            kills: score as u32,
            deaths: 2,
            assists: 1,
            suicides: 1,
            best_spree: 3,
            avg_life: 75.4,
            medals: vec![("DOUBLE KILL".into(), 2), ("KILLING SPREE".into(), 1)],
            local,
            ..ScoreLine::default()
        };
        let team = |name: &str, score| ScoreLine {
            name: name.into(),
            score,
            header: true,
            ..ScoreLine::default()
        };
        vec![
            team("RED TEAM", 7),
            player("JOHN", 5, true),
            player("A VERY LONG GAMERTAG", 2, false),
            team("BLUE TEAM", 4),
            player("SARGE", 4, false),
        ]
    }

    #[test]
    fn the_carnage_report_pages_through_its_stats() {
        let maps = maps();
        let lines = report_lines();
        let c = Context {
            scores: &lines,
            ..ctx(&maps, &[])
        };
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::PostGame);
        assert_eq!(m.title(&c), "POSTGAME CARNAGE REPORT");
        let f = Frame::new(1280.0, 720.0);
        let names: Vec<String> = m.tabs(&c, &f).into_iter().map(|t| t.0).collect();
        assert_eq!(names, ["TEAM STATS", "PLAYER STATS", "KILLS", "MEDALS"]);
        // In a small window the names keep to 8 pixels, and the tabs
        // widen to fit them without running into each other.
        let small = Frame::new(640.0, 360.0);
        let tabs = m.tabs(&c, &small);
        for (k, (name, r)) in tabs.iter().enumerate() {
            let text = name.chars().count() as f32 * MIN_TEXT * crate::font::ASPECT;
            assert!(6.0 * small.s + text <= (r[2] - r[0]) * small.s, "{name}");
            if let Some(next) = tabs.get(k + 1) {
                assert!(r[2] < next.1[0], "{name}");
            }
        }
        // Nor do the stats' headings (BEST SPREE before SCORE).
        for pane in [Pane::Teams, Pane::Players, Pane::Kills, Pane::Medals] {
            let headings = m.report_headings(pane, &small);
            for (k, (name, left)) in headings.iter().enumerate() {
                let width = name.chars().count() as f32 * MIN_TEXT * crate::font::ASPECT;
                let right = headings.get(k + 1).map_or(REPORT_RIGHT, |h| h.1);
                assert!(left + width / small.s < right, "{name}");
            }
        }
        m.input(Input::Right, &c);
        assert_eq!(m.pane, 1);
        m.input(Input::Left, &c);
        m.input(Input::Left, &c);
        assert_eq!(m.pane, 3, "round to MEDALS");
        // A click on a tab shows it.
        let [x0, y0, x1, y1] = f.rect(m.tabs(&c, &f)[2].1);
        m.click([(x0 + x1) * 0.5, (y0 + y1) * 0.5], 1280.0, 720.0, &c);
        assert_eq!(m.pane, 2);
        // Each pane draws, its text readable even in a small window.
        for pane in 0..4 {
            m.pane = pane;
            assert!(smallest_text(|hb| m.draw(hb, 0, 1, 1280.0, 720.0, &c)) >= 8.0);
            assert!(smallest_text(|hb| m.draw(hb, 0, 1, 640.0, 360.0, &c)) >= 8.0);
        }
        // The stats, as the report words them.
        let john = &lines[1];
        assert_eq!(Stat::AvgLife.value(john, 1), "1:15");
        assert_eq!(Stat::Place.value(john, 2), "2ND");
        assert_eq!(Stat::TotalMedals.value(john, 1), "3");
        assert_eq!(
            Stat::MedalsEarned.value(john, 1),
            "DOUBLE KILL x2, KILLING SPREE"
        );
        assert_eq!(report_columns(Pane::Kills)[3], (Stat::Suicides, 497.5));
        // CONTINUE is still under it all, and a free-for-all has no team
        // pane.
        assert_eq!(m.input(Input::Select, &c), Action::EndGame);
        assert_eq!(
            panes(&lines[1..3]),
            [Pane::Players, Pane::Kills, Pane::Medals]
        );
        // Ten characters fit: eight, and the mark.
        assert_eq!(fit("A VERY LONG GAMERTAG", 60.0, 8.0), "A VERY L..");
        assert_eq!(fit("JOHN", 60.0, 8.0), "JOHN");
    }

    #[test]
    fn the_scoreboard_is_readable_in_every_splitscreen_view() {
        // A full window's scoreboard is as before; splitscreen views (half
        // of 720p, and the quarters of a 1280x720 window, or of a 640x360
        // one) no longer shrink it.
        assert_eq!(Frame::readable(1280.0, 720.0).s, 1.5);
        assert_eq!(Frame::readable(1280.0, 360.0).s, 1.0);
        assert_eq!(Frame::readable(640.0, 360.0).s, 1.0);
        assert!(Frame::new(640.0, 360.0).s < 1.0);
        let mut lines = report_lines();
        while lines.len() < MAX_SCORE_LINES {
            lines.push(lines[4].clone());
        }
        let headings = MenuText::default().score_headings(GameType::TeamSlayer);
        for over in [None, Some(("GAME OVER", "YOU WIN!"))] {
            let board = Scoreboard {
                lines: &lines,
                headings: &headings,
                over,
            };
            for (w, h) in [
                (1280.0, 720.0),
                (1280.0, 360.0),
                (640.0, 360.0),
                (640.0, 180.0),
                (320.0, 180.0),
            ] {
                let mut hb = HudBuilder::new(w, h);
                draw_scoreboard(&mut hb, 0, 1, w, h, &board);
                let batches = hb.finish();
                let glyphs: Vec<&[crate::gpu::HudVertex]> = batches
                    .iter()
                    .filter(|b| b.texture == 0)
                    .flat_map(|b| b.vertices.chunks(6))
                    .collect();
                assert!(!glyphs.is_empty());
                for q in glyphs {
                    let xs = q.iter().map(|v| v.position[0]);
                    let ys: Vec<f32> = q.iter().map(|v| v.position[1]).collect();
                    let height = ys.iter().cloned().fold(f32::MIN, f32::max)
                        - ys.iter().cloned().fold(f32::MAX, f32::min);
                    assert!(height >= 8.0, "{w}x{h}: text {height} pixels tall");
                    for x in xs {
                        assert!((0.0..=w).contains(&x), "{w}x{h}: text off the view at {x}");
                    }
                    for y in ys {
                        assert!((0.0..=h).contains(&y), "{w}x{h}: text off the view at {y}");
                    }
                }
            }
        }
        // Ten lines (four on four with the teams) in a quarter of a 720p
        // window start under the HUD's top row (its grenades and ammo),
        // not over it.
        let mut lines = report_lines();
        lines.truncate(4);
        let player = lines[1].clone();
        lines.extend(std::iter::repeat_n(player, 6));
        let board = Scoreboard {
            lines: &lines,
            headings: &headings,
            over: Some(("GAME OVER", "YOU WIN!")),
        };
        let mut hb = HudBuilder::new(640.0, 360.0);
        draw_scoreboard(&mut hb, 0, 1, 640.0, 360.0, &board);
        let top = hb
            .finish()
            .iter()
            .flat_map(|b| b.vertices.iter().map(|v| v.position[1]))
            .fold(f32::MAX, f32::min);
        assert!((60.0..=80.0).contains(&top), "the board's top at {top}");
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
        // Leaving asks first, as Halo 2's System Link lobby did.
        assert_eq!(m.input(Input::Back, &c), Action::None);
        assert_eq!(m.title(&c), "ARE YOU SURE ?");
        assert_eq!(m.label(Row::Yes, &c).0, "LEAVE LOBBY");
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert_eq!(m.screen, Screen::Lobby);
        m.input(Input::Back, &c);
        m.input(Input::Up, &c);
        assert_eq!(m.input(Input::Select, &c), Action::Leave);
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
            kills: 3,
            deaths: 1,
            color: [1.0; 3],
            level,
            ..ScoreLine::default()
        };
        let headings = MenuText::default().score_headings(GameType::Slayer);
        let scoreboard = |lines: &[ScoreLine]| {
            let mut hb = HudBuilder::new(1280.0, 720.0);
            let board = Scoreboard {
                lines,
                headings: &headings,
                over: None,
            };
            draw_scoreboard(&mut hb, 0, 1, 1280.0, 720.0, &board);
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
        let [x0, y0, x1, y1] = Frame::new(1280.0, 720.0).rect(m.row_rect(0));
        m.hover([(x0 + x1) * 0.5, (y0 + y1) * 0.5], 1280.0, 720.0, &c);
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
            // The roster's text: in its place down the right.
            let [left, top, right, bottom] = space(&f).rect(ROSTER);
            let text: Vec<_> = quads(&batches, |t| t == 0)
                .into_iter()
                .filter(|q| q[0] >= left && q[1] >= top)
                .collect();
            let lowest = text.iter().map(|q| q[3]).fold(0.0, f32::max);
            assert!(lowest <= bottom, "{n}: above the bottom of its place");
            let smallest = text.iter().map(|q| q[3] - q[1]).fold(f32::MAX, f32::min);
            assert!(smallest >= 6.0 * f.s, "{n}: glyphs {smallest} px tall");
            let overlap = |a: &[f32; 4], b: &[f32; 4]| {
                a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
            };
            for q in &text {
                assert!(q[2] <= right + 0.01, "{n}: in its place");
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

    #[test]
    fn the_start_screen_goes_on_to_the_main_menu() {
        let maps = maps();
        let c = ctx(&maps, &[]);
        let mut m = Menu::new(Settings::default(), Profile::default());
        // Any key or button.
        for input in [Input::Back, Input::Down, Input::Select] {
            m.show(Screen::Start);
            assert_eq!(m.input(input, &c), Action::None);
            assert_eq!(m.screen, Screen::Main);
            assert_eq!(m.rows(&c)[m.cursor], Row::Online);
            assert_eq!(m.sound.take(), Some(Sound::Forward));
        }
        // Or a click anywhere.
        m.show(Screen::Start);
        assert_eq!(m.click([5.0, 5.0], 1280.0, 720.0, &c), Action::None);
        assert_eq!(m.screen, Screen::Main);
        // Readable without the art, however small the window.
        m.show(Screen::Start);
        assert_eq!(m.legend(&c), None);
        assert!(smallest_text(|hb| m.draw(hb, 0, 1, 640.0, 360.0, &c)) >= 8.0);
    }

    #[test]
    fn the_quit_dialog_never_comes_up_under_the_mouse() {
        let maps = maps();
        let c = ctx(&maps, &[]);
        let mut m = Menu::new(Settings::default(), Profile::default());
        let f = Frame::new(1280.0, 720.0);
        let middle = |r: [f32; 4]| {
            let [x0, y0, x1, y1] = f.rect(r);
            [(x0 + x1) * 0.5, (y0 + y1) * 0.5]
        };
        let overlap =
            |a: [f32; 4], b: [f32; 4]| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
        // QUIT clicked on the main menu, and on the pause menu.
        for (from, quit) in [(Screen::Main, MAIN_ROWS.len() - 1), (Screen::Pause, 2)] {
            m.show(from);
            let clicked = m.row_rect(quit);
            let at = middle(clicked);
            m.hover(at, 1280.0, 720.0, &c);
            assert_eq!(m.click(at, 1280.0, 720.0, &c), Action::None);
            assert_eq!(m.screen, Screen::Confirm);
            assert_eq!(m.rows(&c)[m.cursor], Row::No);
            // Its answers are elsewhere.
            for k in 0..2 {
                assert!(!overlap(clicked, m.row_rect(k)), "{from:?}");
            }
            // And until the mouse moves away, it's ignored there.
            let yes = middle(m.row_rect(0));
            for pos in [at, [at[0] + 3.0, at[1] - 3.0]] {
                assert_eq!(m.click(pos, 1280.0, 720.0, &c), Action::None);
                assert_eq!(m.screen, Screen::Confirm);
                assert_eq!(m.rows(&c)[m.cursor], Row::No);
            }
            m.hover(yes, 1280.0, 720.0, &c);
            assert_eq!(m.rows(&c)[m.cursor], Row::Yes);
            m.input(Input::Back, &c);
            assert_eq!(m.screen, from);
        }
        // Asked from the keyboard, the mouse works the dialog as soon as
        // it moves.
        m.show(Screen::Main);
        m.hover([1.0, 1.0], 1280.0, 720.0, &c);
        m.cursor = MAIN_ROWS.len() - 1;
        m.input(Input::Select, &c);
        let yes = middle(m.row_rect(0));
        assert_eq!(m.click(yes, 1280.0, 720.0, &c), Action::Quit);
    }

    #[test]
    fn the_lobbys_buttons_are_side_by_side() {
        let maps = maps();
        let c = ctx(&maps, &[]);
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::Lobby);
        m.cursor = 4;
        assert_eq!(m.rows(&c)[m.cursor], Row::GameOptions);
        m.input(Input::Right, &c);
        assert_eq!(m.rows(&c)[m.cursor], Row::StartGame);
        assert_eq!(m.sound, Some(Sound::Cursor));
        m.input(Input::Left, &c);
        assert_eq!(m.rows(&c)[m.cursor], Row::GameOptions);
        assert_eq!(m.label(Row::Variant, &c).1.as_deref(), Some("DEFAULT"));
        // At the top left, GAME OPTIONS first.
        let [a, b] = [m.row_rect(4), m.row_rect(5)];
        assert!(a[2] <= b[0] && a[1] == b[1] && a[1] < m.row_rect(0)[1]);
        // Without maps, a game can't start: the menus say why, with
        // Halo 2's sound for it.
        let none = ctx(&[], &[]);
        m.cursor = 5;
        assert_eq!(m.input(Input::Select, &none), Action::None);
        assert_eq!(m.notice.as_deref(), Some("NO MULTIPLAYER MAPS FOUND"));
        assert_eq!(m.sound, Some(Sound::Error));
    }

    #[test]
    fn legends_show_the_controllers_buttons_or_the_keys() {
        let maps = maps();
        let c = ctx(&maps, &[]);
        let mut m = Menu::new(Settings::default(), Profile::default());
        m.show(Screen::Lobby);
        let keys = m.legend(&c);
        assert_eq!(keys.as_deref(), Some("ENTER SELECT   ESC BACK"));
        m.controller = true;
        let buttons = m.legend(&c);
        assert_eq!(buttons.as_deref(), Some("\u{e100} SELECT \u{e101} BACK"));
        m.show(Screen::Pause);
        let pause = m.legend(&c);
        assert_eq!(pause.as_deref(), Some("\u{e100} SELECT \u{e101} RESUME"));
        // Without Halo 2's fonts, the buttons are drawn as coloured
        // squares, as readable as the rest.
        assert!(smallest_text(|hb| m.draw(hb, 0, 1, 640.0, 360.0, &c)) >= 8.0);
    }
}
