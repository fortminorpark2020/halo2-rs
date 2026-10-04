//! Halo 2 style menus, drawn over the map: the main menu, the multiplayer
//! lobby, the system link browser, the player profile, the pause menu and
//! the post-game carnage report. Keyboard, mouse and controllers all work
//! them.

use crate::gpu::hud_mode;
use crate::hud::HudBuilder;
use crate::profile::{color_name, Profile};
use h2net::LanGame;
use h2sim::game::{clean_name, Look, MAX_NAME, PROFILE_COLORS};
use h2sim::GameType;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Main,
    Lobby,
    SystemLink,
    Profile,
    Pause,
    PostGame,
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

pub fn map_title(name: &str) -> String {
    let name = name.to_lowercase();
    MAP_TITLES
        .iter()
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
    let mut maps = maps_in(dir);
    if let Some(dlc) = dlc.filter(|d| d.as_path() != dir) {
        maps.extend(maps_in(&dlc));
    }
    maps.sort_by(|a, b| a.title.cmp(&b.title));
    maps
}

fn maps_in(dir: &Path) -> Vec<MapChoice> {
    use std::io::Read;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("map")))
        .filter(|p| {
            let mut head = vec![0u8; 0x800];
            std::fs::File::open(p)
                .and_then(|mut f| f.read_exact(&mut head))
                .is_ok()
                && blam_cache::Header::parse(&head)
                    .is_ok_and(|h| h.map_type == blam_cache::MapType::Multiplayer)
        })
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
    /// Someone playing at this PC.
    pub local: bool,
    pub header: bool,
}

/// Someone at this PC, in the lobby.
#[derive(Clone, Debug)]
pub struct SeatInfo {
    /// Their gamertag.
    pub name: String,
    /// "KEYBOARD" or "CONTROLLER".
    pub how: &'static str,
    pub team: u8,
    pub look: Look,
}

/// What the game should do after a menu input.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    /// Start a game with the lobby's settings.
    Start,
    Join(LanGame),
    /// The player changed their profile: keep it.
    SaveProfile,
    Resume,
    /// Back to the lobby (ending or leaving the game).
    EndGame,
    Quit,
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
    pub lan: &'a [LanGame],
    /// People at this PC.
    pub seats: &'a [SeatInfo],
    pub scores: &'a [ScoreLine],
    /// Playing in another PC's game.
    pub joined: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Multiplayer,
    SystemLink,
    Profile,
    Quit,
    Name,
    Model,
    Primary,
    Secondary,
    GameType,
    Map,
    Score,
    Bots,
    StartGame,
    Join(usize),
    Searching,
    Resume,
    EndGame,
    Continue,
}

impl Row {
    fn selectable(self) -> bool {
        self != Row::Searching
    }
}

pub struct Menu {
    pub screen: Screen,
    cursor: usize,
    pub settings: Settings,
    /// A line to show (why a game ended, a join that failed).
    pub notice: Option<String>,
    pub sound: Option<Sound>,
    pub profile: Profile,
    /// Typing a new gamertag.
    pub editing: bool,
}

/// Layout, in Halo 2's 640x480 screen units.
const ROW_X: f32 = 48.0;
const ROW_Y: f32 = 128.0;
const ROW_W: f32 = 300.0;
const ROW_H: f32 = 26.0;
const ROW_STEP: f32 = 32.0;
/// The lobby's right-hand panels (map, players).
const PANEL_X: f32 = 372.0;
const PANEL_W: f32 = 228.0;
/// The map's picture (Halo 2's are 440 by 414).
const PICTURE: [f32; 2] = [104.0, 98.0];
const TEXT: [f32; 4] = [0.72, 0.84, 1.0, 1.0];
const BRIGHT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const DIM: [f32; 4] = [0.5, 0.62, 0.8, 0.9];
const PANEL: [f32; 4] = [0.02, 0.07, 0.14, 0.72];
const HIGHLIGHT: [f32; 4] = [0.2, 0.45, 0.85, 0.9];
const WARNING: [f32; 4] = [1.0, 0.55, 0.3, 1.0];

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
            settings,
            notice: None,
            sound: None,
            profile,
            editing: false,
        }
    }

    pub fn show(&mut self, screen: Screen) {
        self.screen = screen;
        self.cursor = 0;
    }

    fn title(&self) -> &'static str {
        match self.screen {
            Screen::Main => "HALO 2",
            Screen::Lobby => "MULTIPLAYER",
            Screen::SystemLink => "SYSTEM LINK",
            Screen::Profile => "PLAYER PROFILE",
            Screen::Pause => "PAUSED",
            Screen::PostGame => "GAME OVER",
        }
    }

    fn rows(&self, ctx: &Context) -> Vec<Row> {
        match self.screen {
            Screen::Main => vec![Row::Multiplayer, Row::SystemLink, Row::Profile, Row::Quit],
            Screen::Profile => vec![Row::Name, Row::Model, Row::Primary, Row::Secondary],
            Screen::Lobby => vec![
                Row::GameType,
                Row::Map,
                Row::Score,
                Row::Bots,
                Row::StartGame,
            ],
            Screen::SystemLink if ctx.lan.is_empty() => vec![Row::Searching],
            Screen::SystemLink => (0..ctx.lan.len().min(8)).map(Row::Join).collect(),
            Screen::Pause => vec![Row::Resume, Row::EndGame, Row::Quit],
            Screen::PostGame => vec![Row::Continue],
        }
    }

    fn label(&self, row: Row, ctx: &Context) -> (String, Option<String>) {
        let s = &self.settings;
        match row {
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
            Row::Score => (
                "SCORE TO WIN".into(),
                Some(match s.score_to_win() {
                    0 => "NO LIMIT".into(),
                    n => crate::local::score_text(n as i32, s.game_type().timed()),
                }),
            ),
            Row::Bots => ("BOTS".into(), Some(s.bots.to_string())),
            Row::StartGame => ("START GAME".into(), None),
            Row::Join(i) => {
                let g = &ctx.lan[i];
                (
                    format!("{} - {}", g.computer.to_uppercase(), map_title(&g.map)),
                    Some(format!("{}/16", g.players)),
                )
            }
            Row::Searching => ("SEARCHING FOR GAMES...".into(), None),
            Row::Resume => ("RESUME".into(), None),
            Row::EndGame if ctx.joined => ("LEAVE GAME".into(), None),
            Row::EndGame => ("END GAME".into(), None),
            Row::Continue => ("CONTINUE".into(), None),
        }
    }

    /// Keep the cursor on a row that's there and can be chosen.
    fn settle(&mut self, rows: &[Row]) {
        if rows.is_empty() {
            self.cursor = 0;
            return;
        }
        self.cursor = self.cursor.min(rows.len() - 1);
        if !rows[self.cursor].selectable() {
            if let Some(k) = rows.iter().position(|r| r.selectable()) {
                self.cursor = k;
            }
        }
    }

    pub fn input(&mut self, input: Input, ctx: &Context) -> Action {
        if self.editing {
            return match input {
                Input::Select | Input::Back => self.typed(Typed::Done),
                _ => Action::None,
            };
        }
        let rows = self.rows(ctx);
        self.settle(&rows);
        let n = rows.len();
        let row = rows.get(self.cursor).copied();
        match input {
            Input::Up | Input::Down => {
                let selectable = rows.iter().filter(|r| r.selectable()).count();
                if selectable > 1 {
                    loop {
                        self.cursor = if input == Input::Up {
                            (self.cursor + n - 1) % n
                        } else {
                            (self.cursor + 1) % n
                        };
                        if rows[self.cursor].selectable() {
                            break;
                        }
                    }
                    self.sound = Some(Sound::Cursor);
                }
                Action::None
            }
            Input::Left | Input::Right => {
                let step = if input == Input::Left { -1 } else { 1 };
                match row {
                    Some(r) if self.adjust(r, step, ctx) => {
                        self.sound = Some(Sound::Cursor);
                        self.saving(r)
                    }
                    _ => Action::None,
                }
            }
            Input::Select => match row {
                Some(r) => self.choose(r, ctx),
                None => Action::None,
            },
            Input::Back => self.back(),
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
            Row::Map => s.map = cycle(s.map, ctx.maps.len()),
            Row::Score => s.score = cycle(s.score, scores(s.game_type()).0.len()),
            Row::Bots => s.bots = cycle(s.bots, MAX_BOTS + 1),
            Row::GameType => {
                s.game_type = cycle(s.game_type, GAME_TYPES.len());
                s.score = scores(s.game_type()).1;
            }
            Row::Model => self.profile.look.elite = !self.profile.look.elite,
            Row::Primary | Row::Secondary => {
                let c = &mut self.profile.look.colors[(row == Row::Secondary) as usize];
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
            Row::Multiplayer => forward(self, Screen::Lobby),
            Row::SystemLink => forward(self, Screen::SystemLink),
            Row::Profile => forward(self, Screen::Profile),
            Row::Quit => Action::Quit,
            Row::Name => {
                self.editing = true;
                self.sound = Some(Sound::Forward);
                Action::None
            }
            Row::GameType
            | Row::Map
            | Row::Score
            | Row::Bots
            | Row::Model
            | Row::Primary
            | Row::Secondary => {
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
                Some(g) => {
                    self.sound = Some(Sound::Advance);
                    Action::Join(g.clone())
                }
                None => Action::None,
            },
            Row::Searching => Action::None,
            Row::Resume => {
                self.sound = Some(Sound::Back);
                Action::Resume
            }
            Row::EndGame | Row::Continue => {
                self.sound = Some(Sound::Forward);
                Action::EndGame
            }
        }
    }

    /// After changing a row's value: profile rows are kept.
    fn saving(&self, row: Row) -> Action {
        if matches!(row, Row::Model | Row::Primary | Row::Secondary) {
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
                name.extend(text.chars().filter(|c| c.is_ascii_graphic() || *c == ' '));
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
                self.sound = Some(Sound::Back);
                Action::SaveProfile
            }
        }
    }

    fn back(&mut self) -> Action {
        match self.screen {
            Screen::Main => Action::None,
            Screen::Profile => {
                self.show(Screen::Main);
                self.cursor = 2;
                self.sound = Some(Sound::Back);
                Action::None
            }
            Screen::Lobby | Screen::SystemLink => {
                let from = self.screen;
                self.show(Screen::Main);
                // Back on the item that led here.
                self.cursor = (from == Screen::SystemLink) as usize;
                self.sound = Some(Sound::Back);
                Action::None
            }
            Screen::Pause => {
                self.sound = Some(Sound::Back);
                Action::Resume
            }
            Screen::PostGame => {
                self.sound = Some(Sound::Forward);
                Action::EndGame
            }
        }
    }

    fn row_rect(&self, k: usize) -> [f32; 4] {
        let wide = matches!(self.screen, Screen::SystemLink);
        let y = ROW_Y + k as f32 * ROW_STEP + self.rows_offset();
        let w = if wide { 400.0 } else { ROW_W };
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
        (0..rows.len()).find_map(|k| {
            let [x0, y0, x1, y1] = f.rect(self.row_rect(k));
            (x >= x0 && x < x1 && y >= y0 && y < y1 && rows[k].selectable())
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
        let stepped = matches!(
            row,
            Row::Map
                | Row::Score
                | Row::Bots
                | Row::GameType
                | Row::Model
                | Row::Primary
                | Row::Secondary
        );
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

    /// The mouse wheel over a setting changes it.
    pub fn wheel(&mut self, up: bool, ctx: &Context) {
        let input = if up { Input::Right } else { Input::Left };
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
            Screen::Main | Screen::Lobby | Screen::SystemLink | Screen::Profile => {
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
        hb.text_left(font, f.at(ROW_X, 48.0), 26.0 * s, self.title(), BRIGHT);
        let rule = f.rect([ROW_X, 84.0, ROW_X + 300.0, 86.0]);
        hb.quad(white, rule, [0.0; 4], HIGHLIGHT, hud_mode::PLAIN, 0.0);

        if self.screen == Screen::PostGame {
            draw_scores(hb, font, white, &f, 104.0, ctx.scores);
        }
        let rows = self.rows(ctx);
        for (k, &row) in rows.iter().enumerate() {
            let rect = self.row_rect(k);
            let selected = k == self.cursor && row.selectable();
            let (label, value) = self.label(row, ctx);
            let (bg, fg) = if selected {
                (HIGHLIGHT, BRIGHT)
            } else if row.selectable() {
                (PANEL, TEXT)
            } else {
                ([0.0; 4], DIM)
            };
            hb.quad(white, f.rect(rect), [0.0; 4], bg, hud_mode::PLAIN, 0.0);
            let text_y = rect[1] + (ROW_H - 11.0) * 0.5;
            hb.text_left(font, f.at(rect[0] + 10.0, text_y), 11.0 * s, &label, fg);
            if let Some(v) = value {
                let v = if selected && !matches!(row, Row::Join(_) | Row::Name) {
                    format!("< {v} >")
                } else {
                    v
                };
                let width = v.chars().count() as f32 * 11.0 * crate::font::ASPECT;
                let x = rect[2] - 10.0 - width;
                hb.text_left(font, f.at(x, text_y), 11.0 * s, &v, fg);
            }
        }
        if self.screen == Screen::Lobby {
            let top = self.draw_map(hb, font, &f, ctx, white);
            self.draw_players(hb, font, white, &f, ctx, top);
        }
        if self.screen == Screen::Profile {
            self.draw_profile(hb, font, white, &f);
        }
        if self.screen == Screen::SystemLink {
            let y = ROW_Y - 22.0;
            hb.text_left(font, f.at(ROW_X, y), 9.0 * s, "GAMES ON YOUR NETWORK", DIM);
        }
        if let Some(n) = &self.notice {
            hb.text_left(font, f.at(ROW_X, 400.0), 10.0 * s, n, WARNING);
        }
        let hint = match self.screen {
            Screen::Profile if self.editing => "TYPE A GAMERTAG, THEN PRESS ENTER",
            Screen::Main => "ENTER OR A: SELECT",
            Screen::PostGame => "ENTER OR A: CONTINUE",
            _ => "ENTER OR A: SELECT   ESC OR B: BACK",
        };
        hb.text_left(font, f.at(ROW_X, 440.0), 8.0 * s, hint, DIM);
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
        if map.picture.is_none() && map.description.is_empty() {
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

    /// The profile's armour colours, beside its rows (the model itself
    /// stands to the right, drawn with the level).
    fn draw_profile(&self, hb: &mut HudBuilder, font: usize, white: usize, f: &Frame) {
        let s = f.s;
        let look = self.profile.look;
        let colors = crate::local::armor_colors(look);
        let (x, y) = (ROW_X, ROW_Y + 4.0 * ROW_STEP + 12.0);
        let height = 54.0;
        hb.quad(
            white,
            f.rect([x, y, x + ROW_W, y + height]),
            [0.0; 4],
            PANEL,
            hud_mode::PLAIN,
            0.0,
        );
        for (k, (label, c)) in ["PRIMARY", "SECONDARY"].iter().zip(colors).enumerate() {
            let sx = x + 10.0 + k as f32 * 145.0;
            let swatch = f.rect([sx, y + 10.0, sx + 34.0, y + 44.0]);
            hb.quad(
                white,
                swatch,
                [0.0; 4],
                gamma_color(c),
                hud_mode::PLAIN,
                0.0,
            );
            hb.text_left(font, f.at(sx + 42.0, y + 14.0), 8.0 * s, label, DIM);
            let name = color_name(look.colors[k]);
            hb.text_left(font, f.at(sx + 42.0, y + 27.0), 10.0 * s, name, TEXT);
        }
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
        let teams = self.settings.game_type().teams();
        let lines = ctx.seats.len() + (self.settings.bots > 0) as usize;
        let invite = ctx.seats.len() < crate::MAX_LOCAL;
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
            let c = if teams {
                crate::local::TEAM_COLORS[seat.team.min(1) as usize]
            } else {
                crate::local::armor_colors(seat.look)[0]
            };
            let how = seat.how;
            let swatch = f.rect([x, y, x + 8.0, y + 9.0]);
            hb.quad(
                white,
                swatch,
                [0.0; 4],
                gamma_color(c),
                hud_mode::PLAIN,
                0.0,
            );
            let line = format!("{}  {how}", seat.name);
            hb.text_left(font, f.at(x + 14.0, y), 9.0 * s, &line, TEXT);
            y += 16.0;
        }
        let bots = self.settings.bots;
        if bots > 0 {
            let line = format!("+ {bots} BOT{}", if bots == 1 { "" } else { "S" });
            hb.text_left(font, f.at(x + 14.0, y), 9.0 * s, &line, DIM);
            y += 16.0;
        }
        y += 6.0;
        if teams {
            hb.text_left(font, f.at(x, y), 7.0 * s, "T OR X: CHANGE TEAM", DIM);
            y += 11.0;
        }
        if invite {
            hb.text_left(
                font,
                f.at(x, y),
                7.0 * s,
                "PRESS START ON A CONTROLLER",
                DIM,
            );
            hb.text_left(
                font,
                f.at(x, y + 11.0),
                7.0 * s,
                "TO PLAY IN SPLITSCREEN",
                DIM,
            );
        }
    }
}

/// An armour or team colour as a HUD colour: they're in gamma space, the
/// HUD's colours linear.
fn gamma_color(c: [f32; 3]) -> [f32; 4] {
    [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2), 1.0]
}

/// Break `text` into lines of at most `width` characters, at spaces.
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
/// to teams when there are team totals, otherwise to players.
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
    let mut y = top;
    for (x, h) in cols
        .iter()
        .zip(["PLACE", "PLAYER", "SCORE", "KILLS", "DEATHS"])
    {
        hb.text_left(font, f.at(*x, y), 8.0 * s, h, DIM);
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
            hb.quad(
                white,
                f.rect([cols[1] - 14.0, y + 1.0, cols[1] - 6.0, y + 10.0]),
                [0.0; 4],
                gamma_color(c),
                hud_mode::PLAIN,
                0.0,
            );
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

    fn ctx<'a>(maps: &'a [MapChoice], lan: &'a [LanGame]) -> Context<'a> {
        Context {
            maps,
            lan,
            seats: Vec::leak(vec![SeatInfo {
                name: "JOHN".into(),
                how: "KEYBOARD",
                team: 0,
                look: Look::default(),
            }]),
            scores: &[],
            joined: false,
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
            },
            Profile::default(),
        );
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
        m.input(Input::Down, &c);
        assert_eq!(m.input(Input::Select, &c), Action::Start);
        assert_eq!(m.sound, Some(Sound::Advance));
        assert_eq!(m.input(Input::Back, &c), Action::None);
        assert_eq!(m.screen, Screen::Main);
        m.input(Input::Up, &c);
        assert_eq!(m.input(Input::Select, &c), Action::Quit);
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
}
