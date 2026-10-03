//! Halo 2 style menus, drawn over the map: the main menu, the multiplayer
//! lobby, the system link browser, the pause menu and the post-game
//! carnage report. Keyboard, mouse and controllers all work them.

use crate::gpu::hud_mode;
use crate::hud::HudBuilder;
use h2net::LanGame;
use h2sim::GameType;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Main,
    Lobby,
    SystemLink,
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
        }
    }
}

/// The multiplayer maps in a folder, by name.
pub fn find_maps(dir: &Path) -> Vec<MapChoice> {
    use std::io::Read;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut maps: Vec<MapChoice> = entries
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
        .collect();
    maps.sort_by(|a, b| a.title.cmp(&b.title));
    maps
}

/// Kills to win the lobby offers; 0 plays on without end.
pub const SCORES: [u32; 6] = [5, 10, 15, 25, 50, 0];
pub const MAX_BOTS: usize = 15;
/// The game types the lobby offers, with their names.
pub const GAME_TYPES: [(GameType, &str); 2] = [
    (GameType::Slayer, "SLAYER"),
    (GameType::TeamSlayer, "TEAM SLAYER"),
];

#[derive(Clone, Debug)]
pub struct Settings {
    /// In `GAME_TYPES`.
    pub game_type: usize,
    /// In `Context::maps`.
    pub map: usize,
    /// In `SCORES`.
    pub score: usize,
    pub bots: usize,
}

impl Settings {
    pub fn game_type(&self) -> GameType {
        GAME_TYPES[self.game_type.min(GAME_TYPES.len() - 1)].0
    }
}

/// A line of the scoreboard: a player, or a team's totals heading its
/// players.
#[derive(Clone, Debug)]
pub struct ScoreLine {
    pub name: String,
    pub score: i32,
    pub kills: u32,
    pub deaths: u32,
    pub color: [f32; 3],
    /// Someone playing at this PC.
    pub local: bool,
    pub header: bool,
}

/// Someone at this PC, in the lobby.
#[derive(Clone, Copy, Debug)]
pub struct SeatInfo {
    /// "KEYBOARD" or "CONTROLLER".
    pub how: &'static str,
    pub team: u8,
}

/// What the game should do after a menu input.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    /// Start a game with the lobby's settings.
    Start,
    Join(LanGame),
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
    Quit,
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
}

/// Layout, in Halo 2's 640x480 screen units.
const ROW_X: f32 = 48.0;
const ROW_Y: f32 = 128.0;
const ROW_W: f32 = 300.0;
const ROW_H: f32 = 26.0;
const ROW_STEP: f32 = 32.0;
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
    pub fn new(settings: Settings) -> Menu {
        Menu {
            screen: Screen::Main,
            cursor: 0,
            settings,
            notice: None,
            sound: None,
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
            Screen::Pause => "PAUSED",
            Screen::PostGame => "GAME OVER",
        }
    }

    fn rows(&self, ctx: &Context) -> Vec<Row> {
        match self.screen {
            Screen::Main => vec![Row::Multiplayer, Row::SystemLink, Row::Quit],
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
            Row::Quit => ("QUIT".into(), None),
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
                Some(match SCORES[s.score] {
                    0 => "NO LIMIT".into(),
                    n => n.to_string(),
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
                if let Some(r) = row {
                    if self.adjust(r, step, ctx) {
                        self.sound = Some(Sound::Cursor);
                    }
                }
                Action::None
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
            Row::Score => s.score = cycle(s.score, SCORES.len()),
            Row::Bots => s.bots = cycle(s.bots, MAX_BOTS + 1),
            Row::GameType => s.game_type = cycle(s.game_type, GAME_TYPES.len()),
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
            Row::Quit => Action::Quit,
            Row::GameType | Row::Map | Row::Score | Row::Bots => {
                if self.adjust(row, 1, ctx) {
                    self.sound = Some(Sound::Cursor);
                }
                Action::None
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

    fn back(&mut self) -> Action {
        match self.screen {
            Screen::Main => Action::None,
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
        if value.is_some() && matches!(row, Row::Map | Row::Score | Row::Bots | Row::GameType) {
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
            Screen::Main | Screen::Lobby | Screen::SystemLink => {
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
                let v = if selected && !matches!(row, Row::Join(_)) {
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
            self.draw_players(hb, font, white, &f, ctx);
        }
        if self.screen == Screen::SystemLink {
            let y = ROW_Y - 22.0;
            hb.text_left(font, f.at(ROW_X, y), 9.0 * s, "GAMES ON YOUR NETWORK", DIM);
        }
        if let Some(n) = &self.notice {
            hb.text_left(font, f.at(ROW_X, 400.0), 10.0 * s, n, WARNING);
        }
        let hint = match self.screen {
            Screen::Main => "ENTER OR A: SELECT",
            Screen::PostGame => "ENTER OR A: CONTINUE",
            _ => "ENTER OR A: SELECT   ESC OR B: BACK",
        };
        hb.text_left(font, f.at(ROW_X, 440.0), 8.0 * s, hint, DIM);
    }

    fn draw_players(
        &self,
        hb: &mut HudBuilder,
        font: usize,
        white: usize,
        f: &Frame,
        ctx: &Context,
    ) {
        let s = f.s;
        let (x, mut y) = (372.0, ROW_Y);
        let teams = self.settings.game_type().teams();
        let lines = ctx.seats.len() + (self.settings.bots > 0) as usize;
        let invite = ctx.seats.len() < crate::MAX_LOCAL;
        let hints = invite as usize * 2 + teams as usize;
        let height = 34.0 + 16.0 * lines as f32 + 6.0 + 11.0 * hints as f32;
        hb.quad(
            white,
            f.rect([x - 8.0, y - 8.0, x + 236.0, y + height]),
            [0.0; 4],
            PANEL,
            hud_mode::PLAIN,
            0.0,
        );
        hb.text_left(font, f.at(x, y), 11.0 * s, "PLAYERS", BRIGHT);
        y += 24.0;
        for (i, seat) in ctx.seats.iter().enumerate() {
            let c = if teams {
                crate::local::TEAM_COLORS[seat.team.min(1) as usize]
            } else {
                crate::local::armor_colors(i)[0]
            };
            let how = seat.how;
            let swatch = f.rect([x, y, x + 8.0, y + 9.0]);
            hb.quad(
                white,
                swatch,
                [0.0; 4],
                [c[0], c[1], c[2], 1.0],
                hud_mode::PLAIN,
                0.0,
            );
            let line = format!("PLAYER {}  {how}", i + 1);
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
                [c[0], c[1], c[2], 1.0],
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
            line.score.to_string(),
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
            seats: &[SeatInfo {
                how: "KEYBOARD",
                team: 0,
            }],
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
        let mut m = Menu::new(Settings {
            game_type: 0,
            map: 0,
            score: 3,
            bots: 3,
        });
        assert_eq!(m.input(Input::Select, &c), Action::None);
        assert_eq!(m.screen, Screen::Lobby);
        m.input(Input::Down, &c);
        m.input(Input::Right, &c);
        assert_eq!(m.settings.map, 1);
        m.input(Input::Right, &c);
        assert_eq!(m.settings.map, 0, "wraps around");
        m.input(Input::Down, &c);
        m.input(Input::Left, &c);
        assert_eq!(SCORES[m.settings.score], 15);
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
        let mut m = Menu::new(Settings {
            game_type: 0,
            map: 0,
            score: 3,
            bots: 3,
        });
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
    fn rows_are_found_under_the_mouse() {
        let maps = maps();
        let c = ctx(&maps, &[]);
        let mut m = Menu::new(Settings {
            game_type: 0,
            map: 0,
            score: 3,
            bots: 3,
        });
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
