//! The in-game GAME MENU's logic: Halo 2's multiplayer screen 0xC3 over
//! a running match, opened with Start (or Esc), with no Resume row
//! (Start, B or Back close it, as in Halo 2). Its rows are Halo 2's in
//! its order, only the ones the launcher can back: Leave Game, Settings
//! (straight to CONTROLLER SETTINGS, as Halo 2 does for a player with no
//! voice options), and End Game for the host of a custom game and in
//! `--offline` runs. The match keeps running while it is open.
//!
//! Everything here is plain Rust with no Windows code, so Linux tests
//! cover it: the pages and keys (`GameMenu`), the pad's presses
//! (`edges`), and what the engine's input polls get while the menu is
//! open and after it closes (`gate`). The panel is drawn by
//! `h2ui::gamemenu` from the `GameMenuFrame` that `GameMenu::frame`
//! builds. The wording is ours; see `words`.

pub mod edges;
pub mod gate;

use crate::controls::{self, ButtonLayout, Controls, StickLayout};
use crate::session::Session;
pub use h2ui::gamemenu::{GameMenuFrame, GameMenuView, MenuPlayer, MenuRow};

/// What kind of game the engine is in (`--game-kind`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GameKind {
    /// A test run with no server (`--offline`).
    #[default]
    Offline,
    /// A ranked matchmaking playlist.
    Ranked,
    /// A social (unranked) matchmaking playlist.
    Social,
    /// A custom game a party leader set up.
    Custom,
}

impl GameKind {
    pub const ALL: [GameKind; 4] = [
        GameKind::Offline,
        GameKind::Ranked,
        GameKind::Social,
        GameKind::Custom,
    ];

    /// How the flag writes it.
    pub fn key(self) -> &'static str {
        match self {
            GameKind::Offline => "offline",
            GameKind::Ranked => "ranked",
            GameKind::Social => "social",
            GameKind::Custom => "custom",
        }
    }

    pub fn from_key(s: &str) -> Option<GameKind> {
        let s = s.trim();
        Self::ALL
            .into_iter()
            .find(|k| k.key().eq_ignore_ascii_case(s))
    }
}

/// How a player left the game from the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leave {
    /// Leave Game.
    Game,
    /// End Game (the host's, for everyone).
    End,
}

/// What the menu needs to know about the game.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuContext {
    pub kind: GameKind,
    /// This PC hosts the game (always, offline).
    pub host: bool,
    /// The controls the engine was started with.
    pub launch: Controls,
    /// The engine picks up a new profile (look sensitivity, auto
    /// centering) mid-match. Until a check on the PC shows it, false.
    pub rereads_profile: bool,
    /// The engine picks up a new gamepad mapping (the button layout)
    /// mid-match. Until a check on the PC shows it, false.
    pub rereads_mapping: bool,
    /// The game's players, for the column beside the GAME MENU's rows.
    pub players: Vec<MenuPlayer>,
}

/// A menu key: from the pad (`edges`) or the keyboard (`menu_key`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Start,
    Back,
    A,
    B,
    Y,
    Up,
    Down,
    Left,
    Right,
}

/// Windows virtual-key codes the menu reads. None of them is bound in the
/// game's keyboard table, so one that leaks to the engine does nothing.
pub mod vk {
    pub const BACK: u32 = 0x08;
    pub const RETURN: u32 = 0x0D;
    pub const ESCAPE: u32 = 0x1B;
    pub const LEFT: u32 = 0x25;
    pub const UP: u32 = 0x26;
    pub const RIGHT: u32 = 0x27;
    pub const DOWN: u32 = 0x28;
}

/// The menu key for a keyboard key: Esc and Backspace work as B, Enter
/// as A, and the arrows move. (With the menu closed, Esc opens it: that
/// is the caller's, as is Start on the pad.)
pub fn menu_key(code: u32) -> Option<Key> {
    Some(match code {
        vk::ESCAPE | vk::BACK => Key::B,
        vk::RETURN => Key::A,
        vk::UP => Key::Up,
        vk::DOWN => Key::Down,
        vk::LEFT => Key::Left,
        vk::RIGHT => Key::Right,
        _ => return None,
    })
}

/// The settings with a value page (CONTROLLER SETTINGS' rows 0 to 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Thumbstick,
    Buttons,
    LookSensitivity,
    LookInversion,
    AutoCenter,
    Vibration,
}

impl Setting {
    pub const ALL: [Setting; 6] = [
        Setting::Thumbstick,
        Setting::Buttons,
        Setting::LookSensitivity,
        Setting::LookInversion,
        Setting::AutoCenter,
        Setting::Vibration,
    ];

    /// Its row on CONTROLLER SETTINGS.
    pub fn row(self) -> usize {
        self as usize
    }

    /// The setting on row `row` of CONTROLLER SETTINGS.
    pub fn at(row: usize) -> Option<Setting> {
        Self::ALL.get(row).copied()
    }

    /// Its value page's choices, in Halo 2's order.
    pub fn choices(self) -> Vec<String> {
        let words = |l: &[(&str, bool)]| l.iter().map(|c| c.0.to_string()).collect();
        match self {
            Setting::Thumbstick => controls::THUMBSTICK_LABELS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            Setting::Buttons => ButtonLayout::ALL
                .iter()
                .map(|l| l.name().to_string())
                .collect(),
            Setting::LookSensitivity => (controls::LOOK_SENSITIVITY_MIN
                ..=controls::LOOK_SENSITIVITY_MAX)
                .map(controls::look_sensitivity_label)
                .collect(),
            Setting::LookInversion => words(&controls::LOOK_INVERSION_LABELS),
            Setting::AutoCenter => words(&controls::AUTO_CENTER_LABELS),
            Setting::Vibration => words(&controls::VIBRATION_LABELS),
        }
    }

    /// Which choice `c` has.
    pub fn index(self, c: &Controls) -> usize {
        let at = |l: &[(&str, bool)], v: bool| l.iter().position(|x| x.1 == v).unwrap_or(0);
        match self {
            Setting::Thumbstick => StickLayout::ALL
                .iter()
                .position(|&s| s == c.sticks)
                .unwrap_or(0),
            Setting::Buttons => ButtonLayout::ALL
                .iter()
                .position(|&b| b == c.buttons)
                .unwrap_or(0),
            Setting::LookSensitivity => usize::from(
                c.look_sensitivity
                    .clamp(
                        controls::LOOK_SENSITIVITY_MIN,
                        controls::LOOK_SENSITIVITY_MAX,
                    )
                    .saturating_sub(controls::LOOK_SENSITIVITY_MIN),
            ),
            Setting::LookInversion => at(&controls::LOOK_INVERSION_LABELS, c.look_inverted),
            Setting::AutoCenter => at(&controls::AUTO_CENTER_LABELS, c.auto_center),
            Setting::Vibration => at(&controls::VIBRATION_LABELS, c.vibration),
        }
    }

    /// `c` with choice `k` (held to the last).
    pub fn with(self, c: Controls, k: usize) -> Controls {
        let n = self.choices().len();
        let k = k.min(n.saturating_sub(1));
        let mut c = c;
        match self {
            Setting::Thumbstick => c.sticks = StickLayout::ALL[k],
            Setting::Buttons => c.buttons = ButtonLayout::ALL[k],
            Setting::LookSensitivity => {
                c.look_sensitivity = controls::LOOK_SENSITIVITY_MIN + k as u8;
            }
            Setting::LookInversion => c.look_inverted = controls::LOOK_INVERSION_LABELS[k].1,
            Setting::AutoCenter => c.auto_center = controls::AUTO_CENTER_LABELS[k].1,
            Setting::Vibration => c.vibration = controls::VIBRATION_LABELS[k].1,
        }
        c
    }

    /// The value `c` has, as its row shows it.
    pub fn value(self, c: &Controls) -> String {
        self.choices()
            .into_iter()
            .nth(self.index(c))
            .unwrap_or_default()
    }

    /// Whether the engine takes a change of it only at the next load (it
    /// reads it from the profile or the mapping at load, and the context
    /// says it doesn't re-read them).
    pub fn next_game_only(self, ctx: &MenuContext) -> bool {
        match self {
            Setting::Buttons => !ctx.rereads_mapping,
            Setting::LookSensitivity | Setting::AutoCenter => !ctx.rereads_profile,
            Setting::Thumbstick | Setting::LookInversion | Setting::Vibration => false,
        }
    }

    /// Whether its row is tagged NEXT GAME: it takes effect only in the
    /// next game, and the live value differs from the launch value.
    pub fn next_game(self, live: &Controls, ctx: &MenuContext) -> bool {
        self.next_game_only(ctx) && self.index(live) != self.index(&ctx.launch)
    }
}

/// CONTROLLER SETTINGS' rows past the value pages.
pub const MOUSE_SENSITIVITY_ROW: usize = 6;
pub const MOUSE_INVERSION_ROW: usize = 7;
pub const DEFAULTS_ROW: usize = 8;

/// The menu's words (ours, matching what h2live does with each case).
pub mod words {
    pub const GAME_MENU: &str = "GAME MENU";
    pub const LEAVE_GAME: &str = "Leave Game";
    pub const SETTINGS: &str = "Settings";
    pub const END_GAME: &str = "End Game";
    pub const CONTROLLER_SETTINGS: &str = "CONTROLLER SETTINGS";
    /// CONTROLLER SETTINGS' rows: Halo 2's screen 0xC8 in its order with
    /// Halo 2 Vista's wording, then our two mouse rows and Vista's last.
    pub const CONTROLLER_ROWS: [&str; 9] = [
        "Thumbstick Layout",
        "Button Layout",
        "Look Sensitivity",
        "Look Inversion",
        "Auto Look Centering",
        "Controller Vibration",
        "Mouse Sensitivity",
        "Mouse Inversion",
        "Use Default Settings",
    ];
    pub const LEAVE_TITLE: &str = "LEAVE GAME";
    /// A ranked game's guest: h2live places a player who leaves last.
    pub const LEAVE_RANKED_GUEST: &str = "Leave this game? It still counts toward your level.";
    /// A ranked game's host: h2live ends the game and counts it a loss.
    pub const LEAVE_RANKED_HOST: &str =
        "You're hosting. Leaving ends the game for everyone and counts as a loss for you.";
    /// A social or custom game's host.
    pub const LEAVE_HOST: &str = "You're hosting. Leaving ends the game for everyone.";
    pub const LEAVE_OTHER: &str = "Leave this game?";
    pub const END_TITLE: &str = "END GAME";
    pub const END_TEXT: &str = "End this game for everyone?";
    pub const DEFAULTS_TITLE: &str = "USE DEFAULT SETTINGS";
    pub const DEFAULTS_TEXT: &str = "Put every controller setting back to Halo 2's defaults?";
    pub const USE_DEFAULTS: &str = "Use Defaults";
    pub const CANCEL: &str = "Cancel";
    pub const WAIT_TITLE: &str = "PLEASE WAIT";
    pub const WAIT_TEXT: &str = "Leaving the game.";
    /// A value page's note for a setting the engine reads only at load.
    pub const NEXT_GAME_NOTE: &str = "Takes effect in your next game.";
    /// CONTROLLER SETTINGS' note while a row is tagged.
    pub const NEXT_GAME_ROWS_NOTE: &str =
        "Settings marked NEXT GAME take effect in your next game.";
    /// What the players column calls a player with no name in the session.
    pub const PLAYER: &str = "Player";
}

/// A row of the GAME MENU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameRow {
    LeaveGame,
    Settings,
    EndGame,
}

impl GameRow {
    pub fn label(self) -> &'static str {
        match self {
            GameRow::LeaveGame => words::LEAVE_GAME,
            GameRow::Settings => words::SETTINGS,
            GameRow::EndGame => words::END_GAME,
        }
    }
}

/// The GAME MENU's rows for a game: Leave Game and Settings always, then
/// End Game for the host of a custom game and in offline runs (Halo 2
/// never offered it in matchmaking).
pub fn game_rows(ctx: &MenuContext) -> Vec<GameRow> {
    let mut rows = vec![GameRow::LeaveGame, GameRow::Settings];
    if ctx.kind == GameKind::Offline || (ctx.kind == GameKind::Custom && ctx.host) {
        rows.push(GameRow::EndGame);
    }
    rows
}

/// The Leave Game question for a game.
pub fn leave_text(kind: GameKind, host: bool) -> &'static str {
    match (kind, host) {
        (GameKind::Ranked, false) => words::LEAVE_RANKED_GUEST,
        (GameKind::Ranked, true) => words::LEAVE_RANKED_HOST,
        (GameKind::Social | GameKind::Custom, true) => words::LEAVE_HOST,
        _ => words::LEAVE_OTHER,
    }
}

/// What a dialog asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Leave,
    End,
    Defaults,
}

/// A dialog's choices: the answer, then Cancel.
pub const CONFIRM_ROW: usize = 0;
pub const CANCEL_ROW: usize = 1;

/// Where the menu is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Closed,
    /// The GAME MENU, on row `row`.
    Game { row: usize },
    /// CONTROLLER SETTINGS.
    Controller { row: usize },
    /// A setting's values, focus on `row`.
    Value { setting: Setting, row: usize },
    /// A dialog (the GAME MENU closed behind it, except for Defaults,
    /// which goes back to CONTROLLER SETTINGS).
    Confirm { ask: Ask, row: usize },
    /// PLEASE WAIT, until the engine's process ends.
    Wait,
}

/// What the caller does after a key.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// The menu opened: hold the engine's input.
    Opened,
    /// The menu closed: release it.
    Closed,
    /// The live controls changed: apply and save them.
    Controls(Controls),
    /// Leave the game (or end it): quit the engine.
    Leave(Leave),
}

/// The game menu: its page, the live controls, and whether the keyboard
/// was used last.
#[derive(Clone, Debug, PartialEq)]
pub struct GameMenu {
    page: Page,
    live: Controls,
    keyboard: bool,
}

impl GameMenu {
    pub fn new(live: Controls) -> GameMenu {
        GameMenu {
            page: Page::Closed,
            live,
            keyboard: false,
        }
    }

    pub fn page(&self) -> Page {
        self.page
    }

    pub fn is_open(&self) -> bool {
        self.page != Page::Closed
    }

    pub fn live(&self) -> Controls {
        self.live
    }

    /// Whether the legend names keys (the keyboard was used last).
    pub fn keyboard(&self) -> bool {
        self.keyboard
    }

    pub fn set_keyboard(&mut self, keyboard: bool) {
        self.keyboard = keyboard;
    }

    /// Opens the GAME MENU on its first row, if it is closed.
    pub fn open(&mut self) -> Vec<Effect> {
        if self.is_open() {
            return Vec::new();
        }
        self.page = Page::Game { row: 0 };
        vec![Effect::Opened]
    }

    fn close(&mut self) -> Vec<Effect> {
        self.page = Page::Closed;
        vec![Effect::Closed]
    }

    /// A quit started (from the menu or not): PLEASE WAIT if it is open.
    pub fn quitting(&mut self) {
        if self.is_open() {
            self.page = Page::Wait;
        }
    }

    /// The game ended: closes the menu, unless it is waiting for the
    /// engine to quit.
    pub fn game_over(&mut self) -> Vec<Effect> {
        match self.page {
            Page::Closed | Page::Wait => Vec::new(),
            _ => self.close(),
        }
    }

    /// The live controls set to `c`: an effect if they changed.
    fn change(&mut self, c: Controls) -> Vec<Effect> {
        if c == self.live {
            return Vec::new();
        }
        self.live = c;
        vec![Effect::Controls(c)]
    }

    /// A key pressed (`from_keyboard`: a keyboard key, so the legend names
    /// keys). A closed menu ignores keys: the caller opens it.
    pub fn press(&mut self, key: Key, from_keyboard: bool, ctx: &MenuContext) -> Vec<Effect> {
        if !self.is_open() || self.page == Page::Wait {
            return Vec::new();
        }
        self.keyboard = from_keyboard;
        let wrap = |row: usize, n: usize, down: bool| {
            if n == 0 {
                0
            } else if down {
                (row + 1) % n
            } else {
                (row + n - 1) % n
            }
        };
        let clamp = |row: usize, n: usize, down: bool| {
            if down {
                (row + 1).min(n.saturating_sub(1))
            } else {
                row.saturating_sub(1)
            }
        };
        match self.page {
            Page::Closed | Page::Wait => Vec::new(),
            Page::Game { row } => {
                let rows = game_rows(ctx);
                match key {
                    Key::Up | Key::Down => {
                        let row = wrap(row, rows.len(), key == Key::Down);
                        self.page = Page::Game { row };
                        Vec::new()
                    }
                    Key::A => match rows.get(row) {
                        Some(GameRow::LeaveGame) => self.ask(Ask::Leave),
                        Some(GameRow::EndGame) => self.ask(Ask::End),
                        Some(GameRow::Settings) => {
                            self.page = Page::Controller { row: 0 };
                            Vec::new()
                        }
                        None => Vec::new(),
                    },
                    Key::B | Key::Back | Key::Start => self.close(),
                    Key::Y | Key::Left | Key::Right => Vec::new(),
                }
            }
            Page::Controller { row } => {
                let n = words::CONTROLLER_ROWS.len();
                match key {
                    Key::Up | Key::Down => {
                        let row = wrap(row, n, key == Key::Down);
                        self.page = Page::Controller { row };
                        Vec::new()
                    }
                    Key::A => {
                        if let Some(setting) = Setting::at(row) {
                            let row = setting.index(&self.live);
                            self.page = Page::Value { setting, row };
                            Vec::new()
                        } else if row == DEFAULTS_ROW {
                            self.ask(Ask::Defaults)
                        } else {
                            self.step(row, true, true)
                        }
                    }
                    Key::Left | Key::Right => self.step(row, key == Key::Right, false),
                    Key::B | Key::Back => {
                        let settings = game_rows(ctx)
                            .iter()
                            .position(|&r| r == GameRow::Settings)
                            .unwrap_or(0);
                        self.page = Page::Game { row: settings };
                        Vec::new()
                    }
                    Key::Start | Key::Y => Vec::new(),
                }
            }
            Page::Value { setting, row } => {
                let n = setting.choices().len();
                match key {
                    Key::Up | Key::Down => {
                        let row = clamp(row, n, key == Key::Down);
                        self.page = Page::Value { setting, row };
                        Vec::new()
                    }
                    Key::A => {
                        self.page = Page::Controller { row: setting.row() };
                        let c = setting.with(self.live, row);
                        self.change(c)
                    }
                    Key::B | Key::Back => {
                        self.page = Page::Controller { row: setting.row() };
                        Vec::new()
                    }
                    Key::Start | Key::Y | Key::Left | Key::Right => Vec::new(),
                }
            }
            Page::Confirm { ask, row } => match key {
                Key::Up | Key::Down => {
                    let row = clamp(row, 2, key == Key::Down);
                    self.page = Page::Confirm { ask, row };
                    Vec::new()
                }
                Key::A if row == CONFIRM_ROW => match ask {
                    Ask::Leave => {
                        self.page = Page::Wait;
                        vec![Effect::Leave(Leave::Game)]
                    }
                    Ask::End => {
                        self.page = Page::Wait;
                        vec![Effect::Leave(Leave::End)]
                    }
                    Ask::Defaults => {
                        self.page = Page::Controller { row: DEFAULTS_ROW };
                        self.change(Controls::default())
                    }
                },
                Key::A | Key::B | Key::Back => match ask {
                    Ask::Defaults => {
                        self.page = Page::Controller { row: DEFAULTS_ROW };
                        Vec::new()
                    }
                    // The GAME MENU closed behind the dialog: Cancel goes
                    // straight back to the game.
                    Ask::Leave | Ask::End => self.close(),
                },
                Key::Start | Key::Y | Key::Left | Key::Right => Vec::new(),
            },
        }
    }

    /// A dialog, its focus on Cancel.
    fn ask(&mut self, ask: Ask) -> Vec<Effect> {
        self.page = Page::Confirm {
            ask,
            row: CANCEL_ROW,
        };
        Vec::new()
    }

    /// A mouse row's value moved on (`forward`) or back, in the lobby's
    /// limits and steps; A goes round at the ends (`wrap`), Left and
    /// Right stop there.
    fn step(&mut self, row: usize, forward: bool, wrap: bool) -> Vec<Effect> {
        let mut c = self.live;
        match row {
            MOUSE_SENSITIVITY_ROW => {
                let (lo, hi) = (
                    controls::MOUSE_SENSITIVITY_MIN,
                    controls::MOUSE_SENSITIVITY_MAX,
                );
                let step = controls::MOUSE_SENSITIVITY_STEP;
                let v = c.mouse_sensitivity;
                c.mouse_sensitivity = match (forward, wrap) {
                    (true, true) if v >= hi - step / 2.0 => lo,
                    (false, true) if v <= lo + step / 2.0 => hi,
                    (true, _) => controls::clean_mouse_sensitivity(v + step),
                    (false, _) => controls::clean_mouse_sensitivity(v - step),
                };
            }
            MOUSE_INVERSION_ROW => c.mouse_inverted = !c.mouse_inverted,
            _ => return Vec::new(),
        }
        self.change(c)
    }

    /// The menu as drawn, for the game in `ctx`. A closed menu gives the
    /// GAME MENU (the caller doesn't draw it).
    pub fn frame(&self, ctx: &MenuContext) -> GameMenuFrame {
        let view = match self.page {
            Page::Closed => self.game_view(0, ctx),
            Page::Game { row } => self.game_view(row, ctx),
            Page::Controller { row } => self.controller_view(row, ctx),
            Page::Value { setting, row } => self.value_view(setting, row, ctx),
            Page::Confirm { ask, row } => {
                let (title, text, answer) = match ask {
                    Ask::Leave => (
                        words::LEAVE_TITLE,
                        leave_text(ctx.kind, ctx.host),
                        words::LEAVE_GAME,
                    ),
                    Ask::End => (words::END_TITLE, words::END_TEXT, words::END_GAME),
                    Ask::Defaults => (
                        words::DEFAULTS_TITLE,
                        words::DEFAULTS_TEXT,
                        words::USE_DEFAULTS,
                    ),
                };
                GameMenuView::Dialog {
                    title: title.into(),
                    text: text.into(),
                    choices: [answer.into(), words::CANCEL.into()],
                    focus: row,
                }
            }
            Page::Wait => GameMenuView::Wait {
                title: words::WAIT_TITLE.into(),
                text: words::WAIT_TEXT.into(),
            },
        };
        GameMenuFrame {
            view,
            keyboard: self.keyboard,
        }
    }

    fn game_view(&self, row: usize, ctx: &MenuContext) -> GameMenuView {
        GameMenuView::List {
            header: words::GAME_MENU.into(),
            rows: game_rows(ctx)
                .into_iter()
                .map(|r| MenuRow {
                    label: r.label().into(),
                    value: None,
                    next_game: false,
                })
                .collect(),
            focus: row,
            side: Vec::new(),
            note: None,
            players: ctx.players.clone(),
        }
    }

    fn controller_view(&self, row: usize, ctx: &MenuContext) -> GameMenuView {
        let c = &self.live;
        let on_off = |on: bool| controls::on_off(on).to_string();
        let rows: Vec<MenuRow> = words::CONTROLLER_ROWS
            .iter()
            .enumerate()
            .map(|(k, &label)| {
                let (value, next_game) = match Setting::at(k) {
                    Some(s) => (Some(s.value(c)), s.next_game(c, ctx)),
                    None => match k {
                        MOUSE_SENSITIVITY_ROW => {
                            (Some(format!("{:.1}", c.mouse_sensitivity)), false)
                        }
                        MOUSE_INVERSION_ROW => (Some(on_off(c.mouse_inverted)), false),
                        _ => (None, false),
                    },
                };
                MenuRow {
                    label: label.into(),
                    value,
                    next_game,
                }
            })
            .collect();
        let note = rows
            .iter()
            .any(|r| r.next_game)
            .then(|| words::NEXT_GAME_ROWS_NOTE.to_string());
        GameMenuView::List {
            header: words::CONTROLLER_SETTINGS.into(),
            rows,
            focus: row,
            side: Vec::new(),
            note,
            players: Vec::new(),
        }
    }

    fn value_view(&self, setting: Setting, row: usize, ctx: &MenuContext) -> GameMenuView {
        let rows = setting
            .choices()
            .into_iter()
            .map(|label| MenuRow {
                label,
                value: None,
                next_game: false,
            })
            .collect();
        // Beside the list: what the focused layout does.
        let side = match setting {
            Setting::Buttons => ButtonLayout::ALL
                .get(row)
                .map(|l| {
                    l.bindings()
                        .into_iter()
                        .map(|(b, what)| format!("{}: {what}", controls::button_name(b)))
                        .collect()
                })
                .unwrap_or_default(),
            Setting::Thumbstick => StickLayout::ALL
                .get(row)
                .map(|s| {
                    let [left, right] = s.sticks();
                    vec![
                        format!("Left stick: {left}"),
                        format!("Right stick: {right}"),
                    ]
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let note = setting
            .next_game_only(ctx)
            .then(|| words::NEXT_GAME_NOTE.to_string());
        GameMenuView::List {
            header: words::CONTROLLER_ROWS[setting.row()].to_uppercase(),
            rows,
            focus: row,
            side,
            note,
            players: Vec::new(),
        }
    }
}

/// The players column for a networked game from its session: everyone in
/// file order, `me` the first player on machine `me` (this PC), the host
/// the first on the session's host machine. In a free-for-all game
/// (`teams` false; each player then has a team of their own in the
/// session) every team is -1, the neutral colour.
pub fn roster(session: &Session, me: usize, teams: bool) -> Vec<MenuPlayer> {
    let first_on = |m: usize| session.on_machine(m).map(|(j, _)| j).next();
    let (mine, host) = (first_on(me), first_on(session.host));
    session
        .players
        .iter()
        .enumerate()
        .map(|(j, p)| MenuPlayer {
            name: p
                .name
                .clone()
                .unwrap_or_else(|| format!("{} {}", words::PLAYER, j + 1)),
            team: if teams { p.team } else { -1 },
            me: Some(j) == mine,
            host: Some(j) == host,
        })
        .collect()
}

/// The players column for an `--offline` run: the one local player, who
/// hosts.
pub fn offline_roster(name: &str) -> Vec<MenuPlayer> {
    vec![MenuPlayer {
        name: if name.trim().is_empty() {
            format!("{} 1", words::PLAYER)
        } else {
            name.to_string()
        },
        team: -1,
        me: true,
        host: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(kind: GameKind, host: bool) -> MenuContext {
        MenuContext {
            kind,
            host,
            ..MenuContext::default()
        }
    }

    /// Every kind of game, hosted and not.
    fn every_ctx() -> Vec<MenuContext> {
        GameKind::ALL
            .iter()
            .flat_map(|&k| [ctx(k, false), ctx(k, true)])
            .collect()
    }

    fn open_menu() -> GameMenu {
        let mut m = GameMenu::new(Controls::default());
        assert_eq!(m.open(), [Effect::Opened]);
        m
    }

    /// Presses keys from the pad, returning every effect.
    fn press(m: &mut GameMenu, keys: &[Key], c: &MenuContext) -> Vec<Effect> {
        keys.iter().flat_map(|&k| m.press(k, false, c)).collect()
    }

    fn rows(view: &GameMenuView) -> Vec<MenuRow> {
        match view {
            GameMenuView::List { rows, .. } => rows.clone(),
            other => panic!("not a list: {other:?}"),
        }
    }

    const ALL_KEYS: [Key; 9] = [
        Key::Start,
        Key::Back,
        Key::A,
        Key::B,
        Key::Y,
        Key::Up,
        Key::Down,
        Key::Left,
        Key::Right,
    ];

    #[test]
    fn game_kinds_read_back() {
        for k in GameKind::ALL {
            assert_eq!(GameKind::from_key(k.key()), Some(k));
        }
        assert_eq!(GameKind::from_key(" Ranked "), Some(GameKind::Ranked));
        assert_eq!(GameKind::from_key("campaign"), None);
        assert_eq!(GameKind::default(), GameKind::Offline);
    }

    #[test]
    fn the_rows_for_every_game() {
        use GameRow::*;
        for c in every_ctx() {
            let want: &[GameRow] = match (c.kind, c.host) {
                (GameKind::Offline, _) | (GameKind::Custom, true) => {
                    &[LeaveGame, Settings, EndGame]
                }
                _ => &[LeaveGame, Settings],
            };
            assert_eq!(game_rows(&c), want, "{:?} host {}", c.kind, c.host);
            let m = open_menu();
            let labels: Vec<String> = rows(&m.frame(&c).view)
                .into_iter()
                .map(|r| r.label)
                .collect();
            let want: Vec<&str> = want.iter().map(|r| r.label()).collect();
            assert_eq!(labels, want);
            match m.frame(&c).view {
                GameMenuView::List { header, focus, .. } => {
                    assert_eq!((header.as_str(), focus), (words::GAME_MENU, 0));
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn the_leave_question_for_every_game() {
        assert_eq!(
            leave_text(GameKind::Ranked, false),
            words::LEAVE_RANKED_GUEST
        );
        assert_eq!(leave_text(GameKind::Ranked, true), words::LEAVE_RANKED_HOST);
        assert_eq!(leave_text(GameKind::Social, true), words::LEAVE_HOST);
        assert_eq!(leave_text(GameKind::Custom, true), words::LEAVE_HOST);
        for (k, host) in [
            (GameKind::Social, false),
            (GameKind::Custom, false),
            (GameKind::Offline, true),
            (GameKind::Offline, false),
        ] {
            assert_eq!(leave_text(k, host), words::LEAVE_OTHER, "{k:?} {host}");
        }
        // The dialog says it.
        for c in every_ctx() {
            let mut m = open_menu();
            press(&mut m, &[Key::A], &c);
            match m.frame(&c).view {
                GameMenuView::Dialog {
                    title,
                    text,
                    choices,
                    focus,
                } => {
                    assert_eq!(title, words::LEAVE_TITLE);
                    assert_eq!(text, leave_text(c.kind, c.host));
                    assert_eq!(choices, [words::LEAVE_GAME, words::CANCEL]);
                    assert_eq!(focus, CANCEL_ROW);
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_closed_menu_and_the_wait_ignore_every_key() {
        let c = ctx(GameKind::Social, false);
        let mut m = GameMenu::new(Controls::default());
        for k in ALL_KEYS {
            assert!(m.press(k, false, &c).is_empty());
            assert!(!m.is_open());
        }
        // Opening twice opens once.
        assert_eq!(m.open(), [Effect::Opened]);
        assert!(m.open().is_empty());
        m.quitting();
        assert_eq!(m.page(), Page::Wait);
        for k in ALL_KEYS {
            assert!(m.press(k, true, &c).is_empty());
            assert_eq!(m.page(), Page::Wait);
        }
        assert!(m.open().is_empty());
        // A quit while closed leaves it closed.
        let mut m = GameMenu::new(Controls::default());
        m.quitting();
        assert_eq!(m.page(), Page::Closed);
    }

    #[test]
    fn every_key_on_the_game_menu() {
        let c = ctx(GameKind::Custom, true);
        // Start, B and Back close it; Y, Left and Right do nothing.
        for k in [Key::Start, Key::B, Key::Back] {
            let mut m = open_menu();
            assert_eq!(m.press(k, false, &c), [Effect::Closed], "{k:?}");
            assert_eq!(m.page(), Page::Closed);
        }
        for k in [Key::Y, Key::Left, Key::Right] {
            let mut m = open_menu();
            assert!(m.press(k, false, &c).is_empty());
            assert_eq!(m.page(), Page::Game { row: 0 });
        }
        // Up and Down wrap over the three rows.
        let mut m = open_menu();
        press(&mut m, &[Key::Up], &c);
        assert_eq!(m.page(), Page::Game { row: 2 });
        press(&mut m, &[Key::Down], &c);
        assert_eq!(m.page(), Page::Game { row: 0 });
        press(&mut m, &[Key::Down, Key::Down, Key::Down], &c);
        assert_eq!(m.page(), Page::Game { row: 0 });
        // Two rows in a matchmade game.
        let mm = ctx(GameKind::Ranked, true);
        let mut m = open_menu();
        press(&mut m, &[Key::Down, Key::Down], &mm);
        assert_eq!(m.page(), Page::Game { row: 0 });
        // A on each row.
        let mut m = open_menu();
        press(&mut m, &[Key::A], &c);
        assert_eq!(
            m.page(),
            Page::Confirm {
                ask: Ask::Leave,
                row: CANCEL_ROW
            }
        );
        let mut m = open_menu();
        press(&mut m, &[Key::Down, Key::A], &c);
        assert_eq!(m.page(), Page::Controller { row: 0 });
        let mut m = open_menu();
        press(&mut m, &[Key::Up, Key::A], &c);
        assert_eq!(
            m.page(),
            Page::Confirm {
                ask: Ask::End,
                row: CANCEL_ROW
            }
        );
        match m.frame(&c).view {
            GameMenuView::Dialog {
                title,
                text,
                choices,
                ..
            } => {
                assert_eq!(
                    (title.as_str(), text.as_str()),
                    (words::END_TITLE, words::END_TEXT)
                );
                assert_eq!(choices, [words::END_GAME, words::CANCEL]);
            }
            other => panic!("{other:?}"),
        }
        // The keyboard's keys set the legend's form.
        let mut m = open_menu();
        assert!(!m.frame(&c).keyboard);
        m.press(Key::Down, true, &c);
        assert!(m.frame(&c).keyboard && m.keyboard());
        m.press(Key::Up, false, &c);
        assert!(!m.frame(&c).keyboard);
        m.set_keyboard(true);
        assert!(m.keyboard());
    }

    #[test]
    fn dialogs_start_on_cancel_and_cancel_goes_back_to_the_game() {
        let c = ctx(GameKind::Offline, true);
        for (open_keys, ask) in [
            (&[Key::A][..], Ask::Leave),
            (&[Key::Up, Key::A][..], Ask::End),
        ] {
            // A on Cancel, B and Back all close the menu: the GAME MENU
            // closed behind the dialog.
            for cancel in [&[Key::A][..], &[Key::B], &[Key::Back], &[Key::Down, Key::A]] {
                let mut m = open_menu();
                press(&mut m, open_keys, &c);
                assert_eq!(
                    m.page(),
                    Page::Confirm {
                        ask,
                        row: CANCEL_ROW
                    }
                );
                assert_eq!(press(&mut m, cancel, &c), [Effect::Closed], "{cancel:?}");
                assert!(!m.is_open());
            }
            // Start, Y, Left and Right do nothing; Up and Down move
            // between the two without wrapping.
            let mut m = open_menu();
            press(&mut m, open_keys, &c);
            for k in [Key::Start, Key::Y, Key::Left, Key::Right] {
                assert!(m.press(k, false, &c).is_empty());
                assert_eq!(
                    m.page(),
                    Page::Confirm {
                        ask,
                        row: CANCEL_ROW
                    }
                );
            }
            press(&mut m, &[Key::Up, Key::Up], &c);
            assert_eq!(
                m.page(),
                Page::Confirm {
                    ask,
                    row: CONFIRM_ROW
                }
            );
            press(&mut m, &[Key::Down, Key::Down], &c);
            assert_eq!(
                m.page(),
                Page::Confirm {
                    ask,
                    row: CANCEL_ROW
                }
            );
        }
    }

    #[test]
    fn confirming_leaves_and_waits() {
        for c in every_ctx() {
            let mut m = open_menu();
            let fx = press(&mut m, &[Key::A, Key::Up, Key::A], &c);
            assert_eq!(fx, [Effect::Leave(Leave::Game)]);
            assert_eq!(m.page(), Page::Wait);
            match m.frame(&c).view {
                GameMenuView::Wait { title, text } => {
                    assert_eq!(
                        (title.as_str(), text.as_str()),
                        (words::WAIT_TITLE, words::WAIT_TEXT)
                    );
                }
                other => panic!("{other:?}"),
            }
            // Waiting: every key ignored, and the game's end doesn't close
            // it.
            for k in ALL_KEYS {
                assert!(m.press(k, false, &c).is_empty());
            }
            assert!(m.game_over().is_empty());
            assert_eq!(m.page(), Page::Wait);
        }
        let c = ctx(GameKind::Custom, true);
        let mut m = open_menu();
        let fx = press(&mut m, &[Key::Up, Key::A, Key::Up, Key::A], &c);
        assert_eq!(fx, [Effect::Leave(Leave::End)]);
        assert_eq!(m.page(), Page::Wait);
    }

    #[test]
    fn every_key_on_controller_settings() {
        let c = ctx(GameKind::Social, false);
        let to_settings = [Key::Down, Key::A];
        let mut m = open_menu();
        press(&mut m, &to_settings, &c);
        let view = m.frame(&c).view;
        let labels: Vec<String> = rows(&view).into_iter().map(|r| r.label).collect();
        assert_eq!(labels, words::CONTROLLER_ROWS);
        if let GameMenuView::List {
            header,
            note,
            players,
            ..
        } = &view
        {
            assert_eq!(header, words::CONTROLLER_SETTINGS);
            assert!(note.is_none() && players.is_empty());
        }
        // Start and Y do nothing; Up wraps to the last row and back.
        for k in [Key::Start, Key::Y] {
            assert!(m.press(k, false, &c).is_empty());
            assert_eq!(m.page(), Page::Controller { row: 0 });
        }
        press(&mut m, &[Key::Up], &c);
        assert_eq!(m.page(), Page::Controller { row: DEFAULTS_ROW });
        press(&mut m, &[Key::Down], &c);
        assert_eq!(m.page(), Page::Controller { row: 0 });
        // Left and Right on a value page's row do nothing.
        assert!(press(&mut m, &[Key::Left, Key::Right], &c).is_empty());
        assert_eq!(m.live(), Controls::default());
        // B and Back go back to the GAME MENU on Settings.
        for k in [Key::B, Key::Back] {
            let mut m = open_menu();
            press(&mut m, &to_settings, &c);
            assert!(m.press(k, false, &c).is_empty());
            assert_eq!(m.page(), Page::Game { row: 1 });
        }
        // A on rows 0 to 5 opens their pages; on 8 the defaults question.
        for s in Setting::ALL {
            let mut m = open_menu();
            press(&mut m, &to_settings, &c);
            for _ in 0..s.row() {
                press(&mut m, &[Key::Down], &c);
            }
            press(&mut m, &[Key::A], &c);
            assert_eq!(
                m.page(),
                Page::Value {
                    setting: s,
                    row: s.index(&Controls::default())
                }
            );
        }
        let mut m = open_menu();
        press(&mut m, &to_settings, &c);
        press(&mut m, &[Key::Up, Key::A], &c);
        assert_eq!(
            m.page(),
            Page::Confirm {
                ask: Ask::Defaults,
                row: CANCEL_ROW
            }
        );
    }

    #[test]
    fn the_mouse_rows_change_in_place() {
        let c = ctx(GameKind::Ranked, false);
        let mut m = open_menu();
        press(&mut m, &[Key::Down, Key::A], &c);
        for _ in 0..MOUSE_SENSITIVITY_ROW {
            press(&mut m, &[Key::Down], &c);
        }
        let d = controls::MOUSE_SENSITIVITY_DEFAULT;
        let fx = press(&mut m, &[Key::Right], &c);
        let up = controls::clean_mouse_sensitivity(d + controls::MOUSE_SENSITIVITY_STEP);
        assert_eq!(
            fx,
            [Effect::Controls(Controls {
                mouse_sensitivity: up,
                ..Controls::default()
            })]
        );
        assert_eq!(m.live().mouse_sensitivity, up);
        press(&mut m, &[Key::Left], &c);
        assert_eq!(m.live().mouse_sensitivity, d);
        press(&mut m, &[Key::A], &c);
        assert_eq!(m.live().mouse_sensitivity, up);
        // The row shows it, to tenths.
        let r = &rows(&m.frame(&c).view)[MOUSE_SENSITIVITY_ROW];
        assert_eq!(r.value.as_deref(), Some("1.7"));
        assert_eq!(
            m.page(),
            Page::Controller {
                row: MOUSE_SENSITIVITY_ROW
            }
        );
        // Left stops at the bottom; A goes round from the top.
        let mut low = GameMenu::new(Controls {
            mouse_sensitivity: controls::MOUSE_SENSITIVITY_MIN,
            ..Controls::default()
        });
        low.open();
        press(&mut low, &[Key::Down, Key::A], &c);
        for _ in 0..MOUSE_SENSITIVITY_ROW {
            press(&mut low, &[Key::Down], &c);
        }
        assert!(press(&mut low, &[Key::Left], &c).is_empty());
        let mut high = GameMenu::new(Controls {
            mouse_sensitivity: controls::MOUSE_SENSITIVITY_MAX,
            ..Controls::default()
        });
        high.open();
        press(&mut high, &[Key::Down, Key::A], &c);
        for _ in 0..MOUSE_SENSITIVITY_ROW {
            press(&mut high, &[Key::Down], &c);
        }
        assert!(press(&mut high, &[Key::Right], &c).is_empty());
        press(&mut high, &[Key::A], &c);
        assert_eq!(
            high.live().mouse_sensitivity,
            controls::MOUSE_SENSITIVITY_MIN
        );
        // Mouse inversion toggles on Left, Right and A.
        press(&mut m, &[Key::Down], &c);
        for (k, on) in [(Key::A, true), (Key::Left, false), (Key::Right, true)] {
            let fx = press(&mut m, &[k], &c);
            assert_eq!(fx.len(), 1);
            assert_eq!(m.live().mouse_inverted, on);
            let r = &rows(&m.frame(&c).view)[MOUSE_INVERSION_ROW];
            assert_eq!(r.value.as_deref(), Some(controls::on_off(on)));
        }
        // Use Default Settings has no value.
        assert!(rows(&m.frame(&c).view)[DEFAULTS_ROW].value.is_none());
    }

    #[test]
    fn value_pages_open_on_the_current_value_and_a_saves() {
        let c = ctx(GameKind::Social, true);
        let live = Controls {
            buttons: ButtonLayout::GreenThumb,
            sticks: StickLayout::Legacy,
            look_sensitivity: 7,
            look_inverted: true,
            auto_center: true,
            vibration: false,
            ..Controls::default()
        };
        let want_rows = [
            (Setting::Thumbstick, 2, "Legacy"),
            (Setting::Buttons, 3, "Green Thumb"),
            (Setting::LookSensitivity, 6, "7 (Very high)"),
            (Setting::LookInversion, 0, "Inverted"),
            (Setting::AutoCenter, 0, "On"),
            (Setting::Vibration, 0, "Off"),
        ];
        for (s, row, value) in want_rows {
            let mut m = GameMenu::new(live);
            m.open();
            press(&mut m, &[Key::Down, Key::A], &c);
            let r = &rows(&m.frame(&c).view)[s.row()];
            assert_eq!(r.value.as_deref(), Some(value), "{s:?}");
            for _ in 0..s.row() {
                press(&mut m, &[Key::Down], &c);
            }
            press(&mut m, &[Key::A], &c);
            assert_eq!(m.page(), Page::Value { setting: s, row });
            let view = m.frame(&c).view;
            let GameMenuView::List {
                header,
                focus,
                rows: choices,
                ..
            } = &view
            else {
                panic!("{view:?}");
            };
            assert_eq!(*focus, row);
            assert_eq!(*header, words::CONTROLLER_ROWS[s.row()].to_uppercase());
            assert_eq!(choices[row].label, value);
            assert!(choices.iter().all(|r| r.value.is_none()));
            // No wrap: Up from the top stays, Down past the end stays.
            for _ in 0..12 {
                press(&mut m, &[Key::Up], &c);
            }
            assert_eq!(m.page(), Page::Value { setting: s, row: 0 });
            for _ in 0..12 {
                press(&mut m, &[Key::Down], &c);
            }
            let last = s.choices().len() - 1;
            assert_eq!(
                m.page(),
                Page::Value {
                    setting: s,
                    row: last
                }
            );
            // Start, Y, Left and Right do nothing; B goes back unchanged.
            for k in [Key::Start, Key::Y, Key::Left, Key::Right] {
                assert!(m.press(k, false, &c).is_empty());
            }
            let mut b = m.clone();
            assert!(b.press(Key::B, false, &c).is_empty());
            assert_eq!(b.page(), Page::Controller { row: s.row() });
            assert_eq!(b.live(), live);
            // A on the last choice saves it and goes back to its row.
            let fx = m.press(Key::A, false, &c);
            let want = s.with(live, last);
            assert_eq!(m.page(), Page::Controller { row: s.row() });
            if want == live {
                assert!(fx.is_empty(), "{s:?}: no change, no save");
            } else {
                assert_eq!(fx, [Effect::Controls(want)], "{s:?}");
            }
            assert_eq!(m.live(), want);
        }
        // Each page's choices, in Halo 2's order.
        assert_eq!(Setting::Thumbstick.choices(), controls::THUMBSTICK_LABELS);
        assert_eq!(Setting::Buttons.choices().len(), 6);
        assert_eq!(Setting::Buttons.choices()[4], "Bumper Jumper");
        assert_eq!(Setting::LookSensitivity.choices().len(), 10);
        assert_eq!(Setting::LookSensitivity.choices()[2], "3 (Default)");
        assert_eq!(Setting::LookInversion.choices(), ["Inverted", "Normal"]);
        assert_eq!(Setting::AutoCenter.choices(), ["On", "Off"]);
        assert_eq!(Setting::Vibration.choices(), ["Off", "On"]);
        // Every choice of every setting reads back.
        for s in Setting::ALL {
            for k in 0..s.choices().len() {
                assert_eq!(s.index(&s.with(live, k)), k, "{s:?} {k}");
            }
        }
    }

    #[test]
    fn the_button_and_thumbstick_pages_show_the_focused_layout() {
        let c = ctx(GameKind::Offline, true);
        let mut m = open_menu();
        press(&mut m, &[Key::Down, Key::A, Key::Down, Key::A], &c);
        for (k, layout) in ButtonLayout::ALL.iter().enumerate() {
            assert_eq!(
                m.page(),
                Page::Value {
                    setting: Setting::Buttons,
                    row: k
                }
            );
            let GameMenuView::List { side, .. } = m.frame(&c).view else {
                panic!();
            };
            assert_eq!(side.len(), layout.bindings().len());
            assert!(side.iter().any(|l| l == "Start: Pause game"), "{side:?}");
            press(&mut m, &[Key::Down], &c);
        }
        let GameMenuView::List { side, .. } = m.frame(&c).view else {
            panic!();
        };
        // Down past the last stays on Recon (no wrap).
        assert!(
            side.contains(&"RB: Reload / Action".to_string()),
            "{side:?}"
        );
        let mut m = open_menu();
        press(&mut m, &[Key::Down, Key::A, Key::A, Key::Down], &c);
        let GameMenuView::List { side, .. } = m.frame(&c).view else {
            panic!();
        };
        assert_eq!(
            side,
            ["Left stick: Look / rotate", "Right stick: Move / strafe"]
        );
    }

    #[test]
    fn next_game_tags_follow_the_rules() {
        let launch = Controls::default();
        let base = MenuContext {
            launch,
            ..ctx(GameKind::Ranked, false)
        };
        let tags = |live: Controls, c: &MenuContext| -> Vec<bool> {
            let m = GameMenu {
                page: Page::Controller { row: 0 },
                live,
                keyboard: false,
            };
            rows(&m.frame(c).view).iter().map(|r| r.next_game).collect()
        };
        // Nothing changed: no tags, no note.
        assert!(tags(launch, &base).iter().all(|&t| !t));
        // The three settings the engine reads at load.
        let cases = [
            (
                Controls {
                    buttons: ButtonLayout::Boxer,
                    ..launch
                },
                1,
            ),
            (
                Controls {
                    look_sensitivity: 9,
                    ..launch
                },
                2,
            ),
            (
                Controls {
                    auto_center: true,
                    ..launch
                },
                4,
            ),
        ];
        for (live, row) in cases {
            let t = tags(live, &base);
            for (k, &tag) in t.iter().enumerate() {
                assert_eq!(tag, k == row, "row {k} for {live:?}");
            }
        }
        // The live settings never are.
        let live = Controls {
            sticks: StickLayout::Southpaw,
            look_inverted: true,
            vibration: false,
            mouse_sensitivity: 4.0,
            mouse_inverted: true,
            ..launch
        };
        assert!(tags(live, &base).iter().all(|&t| !t));
        // Once the engine re-reads them, no tags.
        let all = Controls {
            buttons: ButtonLayout::Recon,
            look_sensitivity: 1,
            auto_center: true,
            ..launch
        };
        assert_eq!(
            tags(all, &base),
            [false, true, true, false, true, false, false, false, false]
        );
        let mapping = MenuContext {
            rereads_mapping: true,
            ..base.clone()
        };
        assert_eq!(
            tags(all, &mapping),
            [false, false, true, false, true, false, false, false, false]
        );
        let profile = MenuContext {
            rereads_profile: true,
            ..base.clone()
        };
        assert_eq!(
            tags(all, &profile),
            [false, true, false, false, false, false, false, false, false]
        );
        let both = MenuContext {
            rereads_mapping: true,
            rereads_profile: true,
            ..base.clone()
        };
        assert!(tags(all, &both).iter().all(|&t| !t));
        // The notes: CONTROLLER SETTINGS' while a row is tagged, a value
        // page's for a setting read at load.
        let note = |page: Page, live: Controls, c: &MenuContext| {
            let m = GameMenu {
                page,
                live,
                keyboard: false,
            };
            match m.frame(c).view {
                GameMenuView::List { note, .. } => note,
                other => panic!("{other:?}"),
            }
        };
        let settings = Page::Controller { row: 0 };
        assert_eq!(note(settings, launch, &base), None);
        assert_eq!(
            note(settings, all, &base).as_deref(),
            Some(words::NEXT_GAME_ROWS_NOTE)
        );
        let page = |setting: Setting| Page::Value { setting, row: 0 };
        for s in [
            Setting::Buttons,
            Setting::LookSensitivity,
            Setting::AutoCenter,
        ] {
            assert_eq!(
                note(page(s), launch, &base).as_deref(),
                Some(words::NEXT_GAME_NOTE)
            );
            assert_eq!(note(page(s), launch, &both), None);
        }
        for s in [
            Setting::Thumbstick,
            Setting::LookInversion,
            Setting::Vibration,
        ] {
            assert_eq!(note(page(s), all, &base), None);
        }
    }

    #[test]
    fn use_default_settings() {
        let c = ctx(GameKind::Social, false);
        let changed = Controls {
            buttons: ButtonLayout::Recon,
            mouse_sensitivity: 3.0,
            vibration: false,
            ..Controls::default()
        };
        let to_defaults = [Key::Down, Key::A, Key::Up, Key::A];
        // Cancel (A on it, B or Back) goes back to the row, unchanged.
        for cancel in [&[Key::A][..], &[Key::B], &[Key::Back]] {
            let mut m = GameMenu::new(changed);
            m.open();
            press(&mut m, &to_defaults, &c);
            match m.frame(&c).view {
                GameMenuView::Dialog {
                    title,
                    text,
                    choices,
                    focus,
                } => {
                    assert_eq!(title, words::DEFAULTS_TITLE);
                    assert_eq!(text, words::DEFAULTS_TEXT);
                    assert_eq!(choices, [words::USE_DEFAULTS, words::CANCEL]);
                    assert_eq!(focus, CANCEL_ROW);
                }
                other => panic!("{other:?}"),
            }
            assert!(press(&mut m, cancel, &c).is_empty());
            assert_eq!(m.page(), Page::Controller { row: DEFAULTS_ROW });
            assert_eq!(m.live(), changed);
        }
        // Use Defaults: Halo 2's defaults, back on the row.
        let mut m = GameMenu::new(changed);
        m.open();
        press(&mut m, &to_defaults, &c);
        let fx = press(&mut m, &[Key::Up, Key::A], &c);
        assert_eq!(fx, [Effect::Controls(Controls::default())]);
        assert_eq!(m.live(), Controls::default());
        assert_eq!(m.page(), Page::Controller { row: DEFAULTS_ROW });
        // Already the defaults: nothing to save.
        press(&mut m, &[Key::A], &c);
        assert!(press(&mut m, &[Key::Up, Key::A], &c).is_empty());
    }

    #[test]
    fn the_games_end_closes_the_menu_but_not_the_wait() {
        let c = ctx(GameKind::Ranked, true);
        let opened: [&[Key]; 4] = [
            &[],
            &[Key::Down, Key::A],
            &[Key::Down, Key::A, Key::A],
            &[Key::A],
        ];
        for keys in opened {
            let mut m = open_menu();
            press(&mut m, keys, &c);
            assert_eq!(m.game_over(), [Effect::Closed], "{keys:?}");
            assert!(!m.is_open());
            assert!(m.game_over().is_empty());
        }
        let mut m = open_menu();
        press(&mut m, &[Key::A, Key::Up, Key::A], &c);
        assert!(m.game_over().is_empty());
        assert_eq!(m.page(), Page::Wait);
    }

    #[test]
    fn the_players_column_is_on_the_game_menu_only() {
        let players = vec![
            MenuPlayer {
                name: "Host".into(),
                team: 0,
                me: false,
                host: true,
            },
            MenuPlayer {
                name: "Me".into(),
                team: 1,
                me: true,
                host: false,
            },
        ];
        let c = MenuContext {
            players: players.clone(),
            ..ctx(GameKind::Social, false)
        };
        let mut m = open_menu();
        let roster = |m: &GameMenu| match m.frame(&c).view {
            GameMenuView::List { players, .. } => Some(players),
            _ => None,
        };
        assert_eq!(roster(&m), Some(players));
        press(&mut m, &[Key::Down, Key::A], &c);
        assert_eq!(roster(&m), Some(Vec::new()));
        press(&mut m, &[Key::A], &c);
        assert_eq!(roster(&m), Some(Vec::new()));
    }

    const SESSION: &str = "\
relay = 127.0.0.1:47051
room = 1
host = 1
machine = 0x10
machine = 0x20
machine = 0x30
player = 0x100 machine=0 team=1 name=Alpha
player = 0x200 machine=1 team=0 name=Bravo Two
player = 0x300 machine=2 team=1
player = 0x301 machine=2 team=0 name=Guest
";

    #[test]
    fn the_roster_comes_from_the_session() {
        let s = Session::parse(SESSION).unwrap();
        let r = roster(&s, 2, true);
        let names: Vec<&str> = r.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "Bravo Two", "Player 3", "Guest"]);
        assert_eq!(r.iter().map(|p| p.team).collect::<Vec<_>>(), [1, 0, 1, 0]);
        // This PC's first player is me; the host machine's first hosts.
        assert_eq!(
            r.iter().map(|p| p.me).collect::<Vec<_>>(),
            [false, false, true, false]
        );
        assert_eq!(
            r.iter().map(|p| p.host).collect::<Vec<_>>(),
            [false, true, false, false]
        );
        // The host's own PC.
        let r = roster(&s, 1, true);
        assert!(r[1].me && r[1].host);
        // Free-for-all: no team colours.
        assert!(roster(&s, 0, false).iter().all(|p| p.team == -1));
        // Offline: the one player, hosting.
        let o = offline_roster("John");
        assert_eq!(
            o,
            [MenuPlayer {
                name: "John".into(),
                team: -1,
                me: true,
                host: true
            }]
        );
        assert_eq!(offline_roster(" ")[0].name, "Player 1");
    }

    #[test]
    fn keyboard_keys_for_the_menu() {
        assert_eq!(menu_key(vk::ESCAPE), Some(Key::B));
        assert_eq!(menu_key(vk::BACK), Some(Key::B));
        assert_eq!(menu_key(vk::RETURN), Some(Key::A));
        assert_eq!(menu_key(vk::UP), Some(Key::Up));
        assert_eq!(menu_key(vk::DOWN), Some(Key::Down));
        assert_eq!(menu_key(vk::LEFT), Some(Key::Left));
        assert_eq!(menu_key(vk::RIGHT), Some(Key::Right));
        assert_eq!(menu_key(u32::from(b'W')), None);
        assert_eq!(gate::VK_ESCAPE as u32, vk::ESCAPE);
        // None of the menu's keys is bound in the game's keyboard table,
        // so one that reaches the engine does nothing.
        let table = controls::keyboard_table();
        for code in [
            vk::BACK,
            vk::RETURN,
            vk::ESCAPE,
            vk::LEFT,
            vk::UP,
            vk::RIGHT,
            vk::DOWN,
        ] {
            assert!(
                table.iter().all(|keys| !keys.contains(&(code as i32))),
                "{code:#x} is bound"
            );
        }
    }
}
