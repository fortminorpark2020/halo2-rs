//! The in-game GAME MENU (Halo 2's multiplayer screen 0xC3) and its pages,
//! drawn as one opaque panel that the launcher copies into the engine's
//! back buffer (`docs/notes/launcher/menu.md` 1.2, and the game menu's
//! design). Halo 2's multiplayer drew no dimming behind it, so the panel
//! is the globals' overlay navy made opaque, with no art, animations or
//! sounds.
//!
//! What to show comes as a `GameMenuFrame` (the launcher's menu logic
//! builds it): a list page (the GAME MENU itself with the game's players
//! beside it, CONTROLLER SETTINGS, or a setting's values), a dialog, or
//! the PLEASE WAIT panel. `game_menu` lays it out in window pixels and
//! `draw_panel` rasterises just the panel's rectangle on the CPU.
//!
//! Every number here is in Halo 2's UI units (`layout::Space`) and is our
//! own estimate unless its comment says otherwise.

use crate::art::FlatArt;
use crate::layout::{self, colour, header_box, legend_box, DialogSize, Space};
use crate::paint::{DrawList, Painter, Resources, TextStyle};
use crate::text::{Fonts, Justify};
use crate::{rgba, Font, Rgba};

/// The list pages' panel (estimate).
pub const LIST_PANEL: [f32; 4] = [-560.0, 430.0, 600.0, -420.0];
/// The dialogs' and the PLEASE WAIT panel (estimate).
pub const DIALOG_PANEL: [f32; 4] = [-440.0, 210.0, 440.0, -210.0];
/// The first row's baseline and the step down to the next (estimates).
pub const ROW_TOP: f32 = 270.0;
pub const ROW_STEP: f32 = 48.0;
/// Where a row's label starts and where its value ends (estimates).
pub const LABEL_X: f32 = -500.0;
pub const VALUE_RIGHT: f32 = 40.0;
/// The side area right of the rows: the players on the GAME MENU, the
/// focused layout's buttons on a value page (estimates).
pub const SIDE_LEFT: f32 = 90.0;
pub const SIDE_RIGHT: f32 = 560.0;
/// The most rows a list page shows at once (the look sensitivity page has
/// ten).
pub const MAX_ROWS: usize = 10;
/// The most players the column shows (Halo 2's 16-player games).
pub const MAX_PLAYERS: usize = 16;
/// What a value that only takes effect in the next game is tagged with.
pub const NEXT_GAME: &str = "NEXT GAME";
/// The players column's heading (followed by the count).
pub const PLAYERS: &str = "PLAYERS";
/// What the host's row says at its right end.
pub const HOST: &str = "HOST";
/// Halo 2's team colours, indexed by the engine's team number, in the
/// order of the multiplayer globals' Team Colors block (blam-cache
/// `colors.rs`): red, blue, yellow, green, purple, orange, brown and pink.
/// The shades are estimates, the same ones the lobby's roster uses
/// (h2launch `lobby/names.rs`), so both screens agree.
pub const TEAM_COLOURS: [[f32; 3]; 8] = [
    hex(0xC8_372D),
    hex(0x2F_62C8),
    hex(0xD9_C02A),
    hex(0x3A_9E3A),
    hex(0x8A_3EC2),
    hex(0xE0_7A22),
    hex(0x8A_5A2B),
    hex(0xE0_6AA8),
];

/// `0xRRGGBB` as 0 to 1 channels.
const fn hex(c: u32) -> [f32; 3] {
    [
        ((c >> 16) & 0xFF) as f32 / 255.0,
        ((c >> 8) & 0xFF) as f32 / 255.0,
        (c & 0xFF) as f32 / 255.0,
    ]
}

/// How far a row's band reaches above and below its baseline, and past
/// its label and value (estimates: the bands of two rows stay apart).
const BAND_UP: f32 = 28.0;
const BAND_DOWN: f32 = 16.0;
const BAND_PAD: f32 = 20.0;
/// The unfocused rows' alpha (estimate; Halo 2's lists put them at half).
const ROW_ALPHA: f32 = 0.6;
/// The other players' alpha in the column (estimate).
const PLAYER_ALPHA: f32 = 0.85;
/// The gap between a label and its value, and a value and its tag.
const GAP: f32 = 16.0;
/// The widest a value may be before it is cut.
const VALUE_ROOM: f32 = 320.0;
/// The side lines: the first one's baseline, the step, and the lowest
/// baseline one may have (above the note).
const SIDE_TOP: f32 = 280.0;
const SIDE_STEP: f32 = 30.0;
const SIDE_BOTTOM: f32 = -240.0;
/// The players column: its heading's box, the first player's baseline and
/// the lowest a player's baseline may be (above the legend).
const PLAYERS_HEAD: [f32; 4] = [SIDE_LEFT, 350.0, SIDE_RIGHT, 310.0];
const PLAYER_TOP: f32 = 280.0;
const PLAYER_BOTTOM: f32 = -320.0;
/// A player's team bar: its width and how far above and below the
/// baseline it reaches; the name starts this far right of the bar's left.
const BAR_WIDTH: f32 = 10.0;
const BAR_UP: f32 = 20.0;
const BAR_DOWN: f32 = 6.0;
const NAME_X: f32 = SIDE_LEFT + 24.0;
/// The note line above the legend.
const NOTE_BOX: [f32; 4] = [LABEL_X, -290.0, SIDE_RIGHT, -340.0];
/// The dialog's pieces: its title, its text (first baseline, step, lines,
/// left and width), its two choices (first baseline, label x, band
/// across) and its legend.
const DIALOG_TITLE: [f32; 4] = [-400.0, 190.0, 400.0, 140.0];
const DIALOG_TEXT_TOP: f32 = 100.0;
const DIALOG_TEXT_STEP: f32 = 32.0;
const DIALOG_TEXT_LINES: usize = 3;
const DIALOG_TEXT_X: f32 = -400.0;
const DIALOG_TEXT_ROOM: f32 = 800.0;
const CHOICE_TOP: f32 = -40.0;
const CHOICE_X: f32 = -360.0;
const CHOICE_BAND: [f32; 2] = [-380.0, 380.0];
const DIALOG_LEGEND: [f32; 4] = [-20.0, -150.0, 420.0, -190.0];
/// The PLEASE WAIT panel's title and text, centred.
const WAIT_TITLE: [f32; 4] = [-400.0, 70.0, 400.0, 20.0];
const WAIT_TEXT: [f32; 4] = [-400.0, -10.0, 400.0, -60.0];

/// The fonts each piece is drawn in.
const HEADER_FONT: Font = Font::Title;
const ROW_FONT: Font = Font::Body;
const SMALL_FONT: Font = Font::SplitHudMessage;

/// A row of a list page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuRow {
    pub label: String,
    /// Its current value, right-aligned (a setting's).
    pub value: Option<String>,
    /// Tag the value NEXT GAME: it takes effect in the next game.
    pub next_game: bool,
}

/// A player in the GAME MENU's players column.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuPlayer {
    pub name: String,
    /// 0 to 7 for Halo 2's team colours; -1 (or any other) in a
    /// free-for-all game.
    pub team: i32,
    /// The player on this PC.
    pub me: bool,
    pub host: bool,
}

/// What the menu shows.
#[derive(Clone, Debug, PartialEq)]
pub enum GameMenuView {
    /// A list page: the GAME MENU, CONTROLLER SETTINGS or a setting's
    /// values.
    List {
        header: String,
        rows: Vec<MenuRow>,
        focus: usize,
        /// Lines in the side area (a button layout's buttons); under the
        /// players when there are both.
        side: Vec<String>,
        /// A line above the legend.
        note: Option<String>,
        /// The game's players, beside the GAME MENU's rows (none on the
        /// other pages).
        players: Vec<MenuPlayer>,
    },
    /// A question with two choices; `focus` is the chosen one's index.
    Dialog {
        title: String,
        text: String,
        choices: [String; 2],
        focus: usize,
    },
    /// PLEASE WAIT, with no choices.
    Wait { title: String, text: String },
}

/// The menu as drawn: the view, and whether the keyboard was used last
/// (the legend then names keys).
#[derive(Clone, Debug, PartialEq)]
pub struct GameMenuFrame {
    pub view: GameMenuView,
    pub keyboard: bool,
}

/// The panel's colour: the globals' overlay navy composited over black,
/// opaque.
pub fn panel_colour() -> Rgba {
    let [r, g, b, a] = colour::OVERLAY;
    [r * a, g * a, b * a, 1.0]
}

/// A team's colour; the list text's for a free-for-all game (-1) or a
/// team past Halo 2's eight.
pub fn team_colour(team: i32) -> Rgba {
    let c = usize::try_from(team)
        .ok()
        .and_then(|t| TEAM_COLOURS.get(t).copied())
        .unwrap_or(colour::LIST_TEXT);
    rgba(c, 1.0)
}

/// The panel a view is drawn in (UI units).
pub fn panel_box(view: &GameMenuView) -> [f32; 4] {
    match view {
        GameMenuView::List { .. } => LIST_PANEL,
        GameMenuView::Dialog { .. } | GameMenuView::Wait { .. } => DIALOG_PANEL,
    }
}

/// Row slot `slot`'s baseline on a list page.
pub fn row_baseline(slot: usize) -> f32 {
    ROW_TOP - ROW_STEP * slot as f32
}

/// The band behind row slot `slot` of a list page (UI units).
pub fn row_band(slot: usize) -> [f32; 4] {
    let y = row_baseline(slot);
    [
        LABEL_X - BAND_PAD,
        y + BAND_UP,
        VALUE_RIGHT + BAND_PAD,
        y - BAND_DOWN,
    ]
}

/// The band behind a dialog's choice `k` (UI units).
pub fn choice_band(k: usize) -> [f32; 4] {
    let y = choice_baseline(k);
    [CHOICE_BAND[0], y + BAND_UP, CHOICE_BAND[1], y - BAND_DOWN]
}

fn choice_baseline(k: usize) -> f32 {
    CHOICE_TOP - ROW_STEP * k as f32
}

/// The step between two players' rows: the rows' own, or less so that
/// `n` of them fit between the first baseline and the lowest.
pub fn player_step(n: usize) -> f32 {
    if n < 2 {
        return ROW_STEP;
    }
    ROW_STEP.min((PLAYER_TOP - PLAYER_BOTTOM) / (n.min(MAX_PLAYERS) - 1) as f32)
}

/// The first row shown of `rows` when `visible` show at once, keeping the
/// focused row in sight.
fn first_shown(rows: usize, visible: usize, focus: usize) -> usize {
    if rows <= visible || visible == 0 {
        return 0;
    }
    focus.saturating_sub(visible - 1).min(rows - visible)
}

/// A line of `text` in `font` with its baseline start at `[x, baseline]`
/// (its end there when `right`); returns its width.
fn line_at(
    p: &mut Painter,
    font: Font,
    [x, baseline]: [f32; 2],
    right: bool,
    color: Rgba,
    text: &str,
) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let line = p.fonts.line(font, 1.0, text);
    let x = if right { x - line.width } else { x };
    p.line(
        &line,
        [x, baseline],
        TextStyle::new(font, color, Justify::Left),
    );
    line.width
}

/// `text` broken into lines no wider than `room`, at most `max` of them
/// (the last cut with "..." when there is more).
fn wrap(fonts: &Fonts, font: Font, text: &str, room: f32, max: usize) -> Vec<String> {
    if max == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let tried = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if fonts.width(font, &tried) <= room || current.is_empty() {
            current = tried;
        } else {
            lines.push(std::mem::take(&mut current));
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.len() > max {
        let rest = lines.split_off(max - 1).join(" ");
        lines.push(rest);
    }
    lines
        .into_iter()
        .map(|l| fonts.fit(font, &l, room))
        .collect()
}

/// The menu in window pixels on `space`, everything inside its panel
/// (`panel_box`): the panel's opaque fill first, then its pieces.
pub fn game_menu(frame: &GameMenuFrame, fonts: &Fonts, space: Space) -> DrawList {
    let mut p = Painter::new(space, &FlatArt, fonts, 0.0);
    p.fill(panel_box(&frame.view), panel_colour());
    match &frame.view {
        GameMenuView::List {
            header,
            rows,
            focus,
            side,
            note,
            players,
        } => {
            list_page(&mut p, header, rows, *focus);
            let mut top = SIDE_TOP;
            if !players.is_empty() {
                top = players_column(&mut p, players) - SIDE_STEP;
            }
            side_lines(&mut p, side, top);
            if let Some(note) = note {
                let style = TextStyle::new(ROW_FONT, rgba(colour::LIST_TEXT, 1.0), Justify::Left);
                let room = NOTE_BOX[2] - NOTE_BOX[0];
                p.text(NOTE_BOX, style, &fonts.fit(ROW_FONT, note, room));
            }
            legend(
                &mut p,
                "a_select_b_back",
                frame.keyboard,
                legend_box(DialogSize::Large),
            );
        }
        GameMenuView::Dialog {
            title,
            text,
            choices,
            focus,
        } => {
            let header = TextStyle::new(HEADER_FONT, rgba(colour::HEADER, 1.0), Justify::Left);
            let room = DIALOG_TITLE[2] - DIALOG_TITLE[0];
            p.text(DIALOG_TITLE, header, &fonts.fit(HEADER_FONT, title, room));
            let body = rgba(colour::LIST_TEXT, 1.0);
            let lines = wrap(fonts, ROW_FONT, text, DIALOG_TEXT_ROOM, DIALOG_TEXT_LINES);
            for (k, l) in lines.iter().enumerate() {
                let y = DIALOG_TEXT_TOP - DIALOG_TEXT_STEP * k as f32;
                line_at(&mut p, ROW_FONT, [DIALOG_TEXT_X, y], false, body, l);
            }
            for (k, choice) in choices.iter().enumerate() {
                let color = if k == *focus {
                    p.fill(choice_band(k), colour::TRACK);
                    rgba(colour::HEADER, 1.0)
                } else {
                    rgba(colour::LIST_TEXT, ROW_ALPHA)
                };
                let room = CHOICE_BAND[1] - CHOICE_X;
                let label = fonts.fit(ROW_FONT, choice, room);
                line_at(
                    &mut p,
                    ROW_FONT,
                    [CHOICE_X, choice_baseline(k)],
                    false,
                    color,
                    &label,
                );
            }
            legend(&mut p, "a_select_b_cancel", frame.keyboard, DIALOG_LEGEND);
        }
        GameMenuView::Wait { title, text } => {
            let header = TextStyle::new(HEADER_FONT, rgba(colour::HEADER, 1.0), Justify::Center);
            let room = WAIT_TITLE[2] - WAIT_TITLE[0];
            p.text(WAIT_TITLE, header, &fonts.fit(HEADER_FONT, title, room));
            let body = TextStyle::new(ROW_FONT, rgba(colour::LIST_TEXT, 1.0), Justify::Center);
            p.text(WAIT_TEXT, body, &fonts.fit(ROW_FONT, text, room));
        }
    }
    p.finish()
}

/// A list page's header and rows.
fn list_page(p: &mut Painter, header: &str, rows: &[MenuRow], focus: usize) {
    let fonts = p.fonts;
    let hbox = header_box(DialogSize::Large);
    let style = TextStyle::new(HEADER_FONT, rgba(colour::HEADER, 1.0), Justify::Left);
    p.text(
        hbox,
        style,
        &fonts.fit(HEADER_FONT, header, hbox[2] - hbox[0]),
    );
    let first = first_shown(rows.len(), MAX_ROWS, focus);
    for (slot, (k, row)) in rows
        .iter()
        .enumerate()
        .skip(first)
        .take(MAX_ROWS)
        .enumerate()
    {
        let y = row_baseline(slot);
        let color = if k == focus {
            p.fill(row_band(slot), colour::TRACK);
            rgba(colour::HEADER, 1.0)
        } else {
            rgba(colour::LIST_TEXT, ROW_ALPHA)
        };
        // From the right: the tag, the value, then the label in the room
        // left.
        let mut right = VALUE_RIGHT;
        if let Some(value) = &row.value {
            if row.next_game {
                right -= line_at(p, SMALL_FONT, [right, y], true, color, NEXT_GAME) + GAP;
            }
            let value = fonts.fit(ROW_FONT, value, VALUE_ROOM);
            right -= line_at(p, ROW_FONT, [right, y], true, color, &value) + GAP;
        }
        let label = fonts.fit(ROW_FONT, &row.label, (right - LABEL_X).max(0.0));
        line_at(p, ROW_FONT, [LABEL_X, y], false, color, &label);
    }
}

/// The players column in the side area: its heading with the count, then
/// a row each (up to `MAX_PLAYERS`): the team's bar, the gamertag, and
/// HOST at the right end for the host. Returns the last row's baseline.
fn players_column(p: &mut Painter, players: &[MenuPlayer]) -> f32 {
    let fonts = p.fonts;
    let head = format!("{PLAYERS} ({})", players.len());
    let style = TextStyle::new(SMALL_FONT, rgba(colour::HEADER, 1.0), Justify::Left);
    let room = PLAYERS_HEAD[2] - PLAYERS_HEAD[0];
    p.text(PLAYERS_HEAD, style, &fonts.fit(SMALL_FONT, &head, room));
    let step = player_step(players.len());
    let mut y = PLAYER_TOP;
    for (k, player) in players.iter().take(MAX_PLAYERS).enumerate() {
        y = PLAYER_TOP - step * k as f32;
        p.fill(
            [SIDE_LEFT, y + BAR_UP, SIDE_LEFT + BAR_WIDTH, y - BAR_DOWN],
            team_colour(player.team),
        );
        let color = if player.me {
            rgba(colour::HEADER, 1.0)
        } else {
            rgba(colour::LIST_TEXT, PLAYER_ALPHA)
        };
        let mut right = SIDE_RIGHT;
        if player.host {
            right -= line_at(p, SMALL_FONT, [right, y], true, color, HOST) + GAP;
        }
        let name = fonts.fit(ROW_FONT, &player.name, (right - NAME_X).max(0.0));
        line_at(p, ROW_FONT, [NAME_X, y], false, color, &name);
    }
    y
}

/// Lines in the side area from the baseline `top` down, as many as fit
/// above the note.
fn side_lines(p: &mut Painter, side: &[String], top: f32) {
    let fonts = p.fonts;
    let color = rgba(colour::LIST_TEXT, 0.9);
    for (k, text) in side.iter().enumerate() {
        let y = top - SIDE_STEP * k as f32;
        if y < SIDE_BOTTOM {
            break;
        }
        let text = fonts.fit(SMALL_FONT, text, SIDE_RIGHT - SIDE_LEFT);
        line_at(p, SMALL_FONT, [SIDE_LEFT, y], false, color, &text);
    }
}

/// The button legend `name` in `bounds`, right-justified, in its keyboard
/// form when the keyboard was used last.
fn legend(p: &mut Painter, name: &str, keyboard: bool, bounds: [f32; 4]) {
    let Some(text) = layout::legend(name, keyboard) else {
        return;
    };
    let fonts = p.fonts;
    let style = TextStyle::new(ROW_FONT, rgba(colour::LEGEND, 1.0), Justify::Right);
    p.text(
        bounds,
        style,
        &fonts.fit(ROW_FONT, &text, bounds[2] - bounds[0]),
    );
}

/// A draw list cut to the window pixels `[x, y, w, h]`: every quad moved
/// by `(-x, -y)`, the ones wholly outside dropped, and the size set.
pub fn crop(list: &DrawList, [x, y, w, h]: [u32; 4]) -> DrawList {
    let (dx, dy) = (x as f32, y as f32);
    let (fw, fh) = (w as f32, h as f32);
    let quads = list
        .quads
        .iter()
        .filter_map(|q| {
            let [x0, y0, x1, y1] = q.rect;
            let rect = [x0 - dx, y0 - dy, x1 - dx, y1 - dy];
            let shows = rect[2] > 0.0 && rect[3] > 0.0 && rect[0] < fw && rect[1] < fh;
            shows.then_some(crate::paint::Quad { rect, ..*q })
        })
        .collect();
    DrawList {
        size: [w, h],
        quads,
        texts: list.texts.clone(),
    }
}

/// The menu drawn: its panel's place in the window (pixels) and its RGBA
/// bytes, `w` by `h`, rows top first, every alpha 255.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Panel {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// The panel's rectangle in a `w` by `h` window: whole pixels covering
/// `panel_box`, inside the window.
pub fn panel_rect(view: &GameMenuView, w: u32, h: u32) -> [u32; 4] {
    let [x0, y0, x1, y1] = Space::new(w, h).rect(panel_box(view));
    let clamp = |v: f32, max: u32| (v.max(0.0) as u32).min(max);
    let (l, t) = (clamp(x0.floor(), w), clamp(y0.floor(), h));
    let (r, b) = (clamp(x1.ceil(), w), clamp(y1.ceil(), h));
    [l, t, r.saturating_sub(l), b.saturating_sub(t)]
}

/// The menu for a `w` by `h` window (the engine's back buffer), drawn on
/// the CPU over the opaque panel colour: only the panel's rectangle.
pub fn draw_panel(frame: &GameMenuFrame, fonts: &Fonts, w: u32, h: u32) -> Panel {
    let rect = panel_rect(&frame.view, w, h);
    let [x, y, pw, ph] = rect;
    let list = crop(&game_menu(frame, fonts, Space::new(w, h)), rect);
    let (pw, ph) = (pw as usize, ph as usize);
    let bg = panel_colour().map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8);
    let mut rgba = bg.repeat(pw * ph);
    crate::cpu::draw_rgba(
        &list,
        &mut rgba,
        pw,
        ph,
        &Resources {
            art: &FlatArt,
            fonts,
        },
    );
    for px in rgba.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }
    Panel {
        x,
        y,
        w: pw as u32,
        h: ph as u32,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::Texture;

    const SIZES: [(u32, u32); 3] = [(640, 360), (1280, 720), (1920, 1080)];

    fn rows(n: usize) -> Vec<MenuRow> {
        (0..n)
            .map(|k| MenuRow {
                label: format!("Row {k}"),
                value: (k % 2 == 1).then(|| format!("Value {k}")),
                next_game: k % 3 == 1,
            })
            .collect()
    }

    fn players(n: usize) -> Vec<MenuPlayer> {
        (0..n)
            .map(|k| MenuPlayer {
                name: format!("Player {k}"),
                team: k as i32 % 2,
                me: k == 1,
                host: k == 0,
            })
            .collect()
    }

    fn list(n: usize, focus: usize) -> GameMenuView {
        GameMenuView::List {
            header: "GAME MENU".into(),
            rows: rows(n),
            focus,
            side: vec!["RT: Fire".into(), "A: Jump".into()],
            note: Some("Takes effect in your next game.".into()),
            players: Vec::new(),
        }
    }

    fn dialog() -> GameMenuView {
        GameMenuView::Dialog {
            title: "LEAVE GAME".into(),
            text:
                "You're hosting. Leaving ends the game for everyone and counts as a loss for you."
                    .into(),
            choices: ["Leave Game".into(), "Cancel".into()],
            focus: 1,
        }
    }

    fn wait() -> GameMenuView {
        GameMenuView::Wait {
            title: "PLEASE WAIT".into(),
            text: "Leaving the game.".into(),
        }
    }

    fn views() -> Vec<GameMenuView> {
        let mut top = list(3, 0);
        if let GameMenuView::List {
            players: p,
            side,
            note,
            ..
        } = &mut top
        {
            *p = players(8);
            side.clear();
            *note = None;
        }
        vec![list(9, 2), top, dialog(), wait()]
    }

    fn frame(view: GameMenuView) -> GameMenuFrame {
        GameMenuFrame {
            view,
            keyboard: false,
        }
    }

    /// Every quad of the menu's list is inside its panel (in pixels, with
    /// half a pixel for rounding).
    fn inside_panel(view: &GameMenuView, fonts: &Fonts, w: u32, h: u32) {
        let space = Space::new(w, h);
        let list = game_menu(&frame(view.clone()), fonts, space);
        let [l, t, r, b] = space.rect(panel_box(view));
        for q in &list.quads {
            let [x0, y0, x1, y1] = q.rect;
            assert!(
                x0 >= l - 0.5 && y0 >= t - 0.5 && x1 <= r + 0.5 && y1 <= b + 0.5,
                "{w}x{h}: {q:?} outside [{l}, {t}, {r}, {b}] in {view:?}"
            );
        }
    }

    /// The mean brightness of a box (UI units) of a drawn panel.
    fn brightness(panel: &Panel, space: Space, rect: [f32; 4]) -> f32 {
        let [x0, y0, x1, y1] = space.rect(rect);
        let (mut sum, mut n) = (0u64, 0u64);
        for y in (y0.ceil() as u32)..(y1.floor() as u32) {
            for x in (x0.ceil() as u32)..(x1.floor() as u32) {
                let (px, py) = (x - panel.x, y - panel.y);
                let at = ((py * panel.w + px) * 4) as usize;
                sum += panel.rgba[at..at + 3]
                    .iter()
                    .map(|&v| u64::from(v))
                    .sum::<u64>();
                n += 1;
            }
        }
        sum as f32 / n.max(1) as f32
    }

    #[test]
    fn every_view_is_an_opaque_panel_inside_the_window() {
        let fonts = Fonts::fallback();
        for (w, h) in SIZES {
            for view in views() {
                let panel = draw_panel(&frame(view.clone()), &fonts, w, h);
                assert!(panel.w > 0 && panel.h > 0, "{w}x{h}");
                assert!(panel.x + panel.w <= w && panel.y + panel.h <= h, "{w}x{h}");
                assert_eq!(panel.rgba.len(), (panel.w * panel.h * 4) as usize);
                assert!(panel.rgba.chunks(4).all(|p| p[3] == 255), "{w}x{h}");
                inside_panel(&view, &fonts, w, h);
                // The panel's own colour is most of it, and something is
                // drawn over it.
                let bg = panel_colour().map(|c| (c * 255.0).round() as u8);
                let plain = panel.rgba.chunks(4).filter(|p| p[..3] == bg[..3]).count();
                let all = (panel.w * panel.h) as usize;
                assert!(plain * 2 > all && plain < all, "{w}x{h}: {plain} of {all}");
            }
        }
        // No window, no panel.
        let none = draw_panel(&frame(list(3, 0)), &fonts, 0, 0);
        assert_eq!((none.w, none.h, none.rgba.len()), (0, 0, 0));
    }

    #[test]
    fn the_focused_row_is_brighter_and_the_header_drawn() {
        let fonts = Fonts::fallback();
        for (w, h) in SIZES {
            let space = Space::new(w, h);
            let panel = draw_panel(&frame(list(5, 2)), &fonts, w, h);
            let focused = brightness(&panel, space, row_band(2));
            for other in [0, 1, 3, 4] {
                let b = brightness(&panel, space, row_band(other));
                assert!(focused > b, "{w}x{h}: row {other} {b} vs {focused}");
            }
            // The header's box has more than the panel's colour in it.
            let bg = brightness(&panel, space, [400.0, -100.0, 550.0, -200.0]);
            let header = brightness(&panel, space, header_box(DialogSize::Large));
            assert!(header > bg + 1.0, "{w}x{h}: {header} vs {bg}");
            // The dialog's focused choice is brighter than the other.
            let panel = draw_panel(&frame(dialog()), &fonts, w, h);
            let cancel = brightness(&panel, space, choice_band(1));
            let leave = brightness(&panel, space, choice_band(0));
            assert!(cancel > leave, "{w}x{h}: {cancel} vs {leave}");
        }
    }

    #[test]
    fn the_dialog_panel_is_smaller_than_the_list_panel() {
        let [l, t, r, b] = DIALOG_PANEL;
        let [ll, lt, lr, lb] = LIST_PANEL;
        assert!(l > ll && t < lt && r < lr && b > lb);
        assert_eq!(panel_box(&dialog()), DIALOG_PANEL);
        assert_eq!(panel_box(&wait()), DIALOG_PANEL);
        assert_eq!(panel_box(&list(1, 0)), LIST_PANEL);
        let fonts = Fonts::fallback();
        let small = draw_panel(&frame(dialog()), &fonts, 1280, 720);
        let big = draw_panel(&frame(list(3, 0)), &fonts, 1280, 720);
        assert!(small.w < big.w && small.h < big.h);
        // Both panels are within the Xbox's frame, so inside any window.
        for [l, t, r, b] in [LIST_PANEL, DIALOG_PANEL] {
            assert!(l >= layout::FRAME[0] && r <= layout::FRAME[2]);
            assert!(t <= layout::FRAME[1] && b >= layout::FRAME[3]);
        }
    }

    /// The glyph quads' rows (window y of their bottoms), top first.
    fn glyph_rows(list: &DrawList, color: [f32; 3]) -> Vec<i32> {
        let mut ys: Vec<i32> = list
            .quads
            .iter()
            .filter(|q| q.texture == Texture::Fallback && q.color[0][..3] == color)
            .map(|q| q.rect[3].round() as i32)
            .collect();
        ys.sort_unstable();
        ys.dedup();
        ys
    }

    #[test]
    fn ten_rows_fit_and_more_scroll() {
        let fonts = Fonts::fallback();
        let space = Space::new(1920, 1080);
        let mut view = list(MAX_ROWS, 0);
        if let GameMenuView::List { note, .. } = &mut view {
            *note = None;
        }
        let drawn = game_menu(&frame(view.clone()), &fonts, space);
        // The header and the focused row in the header's colour, the
        // legend in its own (its words, and the built-in font's letters
        // on their discs) in its box; the other nine rows and the two
        // side lines in the list's; every row above the note's line.
        assert_eq!(glyph_rows(&drawn, colour::HEADER).len(), 2);
        let legend = glyph_rows(&drawn, colour::LEGEND);
        let [_, top, _, bottom] = space.rect(legend_box(DialogSize::Large));
        let inside = |y: &i32| (top - 1.0..=bottom + 1.0).contains(&(*y as f32));
        assert!(
            !legend.is_empty() && legend.iter().all(inside),
            "{legend:?}"
        );
        assert_eq!(glyph_rows(&drawn, colour::LIST_TEXT).len(), 9 + 2);
        assert!(row_band(MAX_ROWS - 1)[3] > NOTE_BOX[1]);
        for (w, h) in SIZES {
            inside_panel(&view, &fonts, w, h);
        }
        // Twelve rows with the last focused: rows 2 to 11 show.
        let view = list(12, 11);
        let drawn = game_menu(&frame(view.clone()), &fonts, space);
        let focused = glyph_rows(&drawn, colour::HEADER);
        let last = space.at([0.0, row_baseline(MAX_ROWS - 1)])[1].round() as i32;
        assert!(focused.contains(&last), "{focused:?} {last}");
        assert_eq!(first_shown(12, MAX_ROWS, 11), 2);
        assert_eq!(first_shown(12, MAX_ROWS, 3), 0);
        inside_panel(&view, &fonts, 640, 360);
    }

    #[test]
    fn next_game_tags_and_long_text_stay_in_the_panel() {
        let fonts = Fonts::fallback();
        let space = Space::new(1280, 720);
        let tagged = |next_game: bool| GameMenuView::List {
            header: "CONTROLLER SETTINGS".into(),
            rows: vec![MenuRow {
                label: "Button Layout".into(),
                value: Some("Bumper Jumper".into()),
                next_game,
            }],
            focus: 0,
            side: Vec::new(),
            note: None,
            players: Vec::new(),
        };
        let with = game_menu(&frame(tagged(true)), &fonts, space);
        let without = game_menu(&frame(tagged(false)), &fonts, space);
        let glyphs = NEXT_GAME.chars().filter(|c| *c != ' ').count();
        assert_eq!(with.quads.len(), without.quads.len() + glyphs);
        // Far too long for anything: cut, and still inside.
        let long = "W".repeat(200);
        let view = GameMenuView::List {
            header: long.clone(),
            rows: vec![MenuRow {
                label: long.clone(),
                value: Some(long.clone()),
                next_game: true,
            }],
            focus: 0,
            side: vec![long.clone()],
            note: Some(long.clone()),
            players: vec![MenuPlayer {
                name: long.clone(),
                team: 3,
                me: true,
                host: true,
            }],
        };
        for (w, h) in SIZES {
            inside_panel(&view, &fonts, w, h);
            let d = GameMenuView::Dialog {
                title: long.clone(),
                text: format!("{long} {long} {long} {long} {long}"),
                choices: [long.clone(), long.clone()],
                focus: 0,
            };
            inside_panel(&d, &fonts, w, h);
            let wv = GameMenuView::Wait {
                title: long.clone(),
                text: long.clone(),
            };
            inside_panel(&wv, &fonts, w, h);
        }
        // The dialog's text wraps onto lines of its own.
        let lines = wrap(&fonts, ROW_FONT, "a b c", 1e9, 3);
        assert_eq!(lines, ["a b c"]);
        let words = "one two three four five six seven eight nine ten";
        let room = fonts.width(ROW_FONT, "one two three");
        let lines = wrap(&fonts, ROW_FONT, words, room, 3);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "one two three");
        assert!(lines[2].ends_with("..."));
    }

    #[test]
    fn the_players_column_fits_one_eight_or_sixteen() {
        let fonts = Fonts::fallback();
        for n in [1, 8, 16] {
            let view = GameMenuView::List {
                header: "GAME MENU".into(),
                rows: rows(2),
                focus: 0,
                side: Vec::new(),
                note: None,
                players: players(n),
            };
            for (w, h) in SIZES {
                inside_panel(&view, &fonts, w, h);
                let panel = draw_panel(&frame(view.clone()), &fonts, w, h);
                assert!(panel.rgba.chunks(4).all(|p| p[3] == 255));
            }
            // Every player's bar is drawn in its team's colour, and the
            // last row stays above the legend.
            let space = Space::new(1280, 720);
            let drawn = game_menu(&frame(view), &fonts, space);
            let bars = drawn
                .quads
                .iter()
                .filter(|q| q.texture == Texture::White && TEAM_COLOURS.contains(&rgb(q)))
                .count();
            assert_eq!(bars, n, "{n} players");
            let last = PLAYER_TOP - player_step(n) * (n - 1) as f32;
            assert!(last >= PLAYER_BOTTOM - 1e-3, "{n}: {last}");
            assert!(last - BAR_DOWN > legend_box(DialogSize::Large)[1]);
        }
        assert_eq!(player_step(1), ROW_STEP);
        assert_eq!(player_step(8), ROW_STEP.min(600.0 / 7.0));
        assert!((player_step(16) - 40.0).abs() < 1e-4);
        // More than sixteen: only sixteen drawn.
        let view = GameMenuView::List {
            header: "GAME MENU".into(),
            rows: rows(2),
            focus: 0,
            side: Vec::new(),
            note: None,
            players: players(20),
        };
        inside_panel(&view, &fonts, 640, 360);
    }

    fn rgb(q: &crate::paint::Quad) -> [f32; 3] {
        [q.color[0][0], q.color[0][1], q.color[0][2]]
    }

    #[test]
    fn team_colours_by_index_and_the_local_player_lit() {
        assert_eq!(team_colour(0), rgba(hex(0xC8_372D), 1.0));
        // Halo 2's order (the mulg Team Colors block), not Halo 3's: team
        // 2 is yellow, 3 green and 5 orange.
        let [r, g, b] = TEAM_COLOURS[2];
        assert!(r > 0.8 && g > 0.7 && b < 0.2, "yellow: {r} {g} {b}");
        let [r, g, b] = TEAM_COLOURS[3];
        assert!(g > r + 0.3 && g > b + 0.3, "green: {r} {g} {b}");
        let [r, g, b] = TEAM_COLOURS[5];
        assert!(
            r > 0.8 && g > 0.4 && g < 0.6 && b < 0.2,
            "orange: {r} {g} {b}"
        );
        assert_eq!(team_colour(1), rgba(TEAM_COLOURS[1], 1.0));
        assert_eq!(team_colour(7), rgba(TEAM_COLOURS[7], 1.0));
        // Free-for-all and past the eighth: the list's colour.
        assert_eq!(team_colour(-1), rgba(colour::LIST_TEXT, 1.0));
        assert_eq!(team_colour(8), rgba(colour::LIST_TEXT, 1.0));
        assert_eq!(team_colour(i32::MIN), rgba(colour::LIST_TEXT, 1.0));
        // Every team colour differs.
        for (i, a) in TEAM_COLOURS.iter().enumerate() {
            assert!(TEAM_COLOURS[i + 1..].iter().all(|b| b != a));
        }
        let fonts = Fonts::fallback();
        let space = Space::new(1920, 1080);
        let mut ps = players(3);
        ps[2].team = -1;
        let view = GameMenuView::List {
            header: "GAME MENU".into(),
            rows: rows(2),
            // No row in the header's colour, so it is the player's.
            focus: 99,
            side: Vec::new(),
            note: None,
            players: ps,
        };
        let drawn = game_menu(&frame(view), &fonts, space);
        // The header itself and player 1 (this PC's) in the header's
        // colour: player 1's row is the second, one step down.
        let lit = glyph_rows(&drawn, colour::HEADER);
        let me = space.at([0.0, PLAYER_TOP - player_step(3)])[1].round() as i32;
        assert!(lit.contains(&me), "{lit:?} {me}");
        let other = space.at([0.0, PLAYER_TOP])[1].round() as i32;
        assert!(!lit.contains(&other) || other == me);
        // The third player's bar is the neutral colour.
        let neutral = drawn
            .quads
            .iter()
            .filter(|q| q.texture == Texture::White && rgb(q) == colour::LIST_TEXT)
            .count();
        assert_eq!(neutral, 1);
        // HOST is written once, for player 0.
        let host_glyphs = HOST.len();
        let row0 = drawn
            .quads
            .iter()
            .filter(|q| {
                q.texture == Texture::Fallback
                    && (q.rect[3].round() as i32 - other).abs() <= 1
                    && q.rect[0] > space.at([SIDE_LEFT, 0.0])[0]
            })
            .count();
        assert_eq!(row0, "Player 0".replace(' ', "").len() + host_glyphs);
    }

    #[test]
    fn the_legend_follows_the_view_and_the_keyboard() {
        let fonts = Fonts::fallback();
        let space = Space::new(1280, 720);
        let count = |view: GameMenuView, keyboard: bool| {
            game_menu(&GameMenuFrame { view, keyboard }, &fonts, space)
                .quads
                .len()
        };
        // "A SELECT B BACK" draws two button discs with letters; "ENTER
        // SELECT   ESC BACK" draws plain letters.
        let pad = count(list(1, 0), false);
        let keys = count(list(1, 0), true);
        assert_ne!(pad, keys);
        assert_ne!(count(dialog(), false), count(dialog(), true));
        // The wait panel has no legend.
        assert_eq!(count(wait(), false), count(wait(), true));
    }

    #[test]
    fn crop_moves_quads_and_drops_the_ones_outside() {
        let fonts = Fonts::fallback();
        let space = Space::new(100, 100);
        let mut p = Painter::new(space, &FlatArt, &fonts, 0.0);
        p.fill_px([20.0, 30.0, 40.0, 50.0], [1.0; 4]);
        p.fill_px([90.0, 90.0, 99.0, 99.0], [1.0; 4]);
        let list = p.finish();
        let cut = crop(&list, [10, 20, 50, 50]);
        assert_eq!(cut.size, [50, 50]);
        assert_eq!(cut.quads.len(), 1);
        assert_eq!(cut.quads[0].rect, [10.0, 10.0, 30.0, 30.0]);
        assert_eq!(cut.quads[0].color, list.quads[0].color);
        // The panel's rectangle at 1280x720 (0.6 pixels a unit).
        let rect = panel_rect(&list_view(), 1280, 720);
        assert_eq!(rect, [304, 102, 696, 510]);
    }

    fn list_view() -> GameMenuView {
        list(1, 0)
    }
}
