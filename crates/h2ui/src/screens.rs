//! The start screen and the main menu (menu.md phase 1), each a function
//! of the menus' clock, the screen's state, the art source and the fonts,
//! giving a `DrawList`. Without the art they draw flat shapes in Halo 2's
//! colours; the layouts' numbers are the built-in ones (`layout`), or the
//! tags' themselves when a mainmenu.map was read (`tags`, `shell`).
//!
//! Neither has a header or a button legend, as on the Xbox. Both draw
//! the logo, the tracks and the brace; the start screen adds the pulsing
//! "PRESS START" (and no build number), the main menu its list.
//!
//! What is behind them is a list of its own (`background`): it doesn't
//! move, and drawn on the CPU it costs more than the rest of a frame, so
//! a caller draws it once for a window size (`cpu::Kept`) and copies it
//! in under each frame's screen.
//!
//! Any other screen is drawn from its tag's layout (`screen`, from
//! `tags::screen_layout`): its pane's pictures, lists, buttons and texts,
//! its header and its button legend. Which words each text slot shows is
//! the caller's binding (`Words`), never the tag's words unchecked: they
//! go through `wording` first, and a slot the binding doesn't name is
//! hidden.

use std::borrow::Cow;

use crate::anim::{self, Focus};
use crate::art::{last_part, ArtSource};
use crate::chrome;
use crate::layout::{
    self, colour, BitmapWidget, ChromeLayout, Face, LegendItem, ListLayout, MainMenuLayout,
    PaneLayout, ScreenLayout, SlotText, Space, StartLayout, TextWidget,
};
use crate::paint::{self, Anchor, Blend, DrawList, Painter, Quad, TextStyle, Texture};
use crate::text::Fonts;

/// The start screen's words.
pub const PRESS_START: &str = "PRESS START";
/// How strongly the framing shows behind the two screens on a still
/// background. Estimate.
pub const FRAMING_ALPHA: f32 = 0.5;

/// What is behind the two screens (`background`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backdrop {
    /// Nothing: the engine's flythrough is there (option A), or the
    /// caller draws its own.
    Scene,
    /// A still background (option C): a navy gradient, and the framing
    /// faintly over it when the art has it.
    #[default]
    Still,
}

/// The start screen's state.
#[derive(Clone, Copy, Debug)]
pub struct Start<'a> {
    pub layout: &'a StartLayout,
    /// When it opened, on the menus' clock (seconds).
    pub opened: f64,
    /// What it says ("PRESS START").
    pub text: &'a str,
    /// The small print at the bottom right (may be empty).
    pub small_print: &'a str,
}

/// The main menu's state.
#[derive(Clone, Copy, Debug)]
pub struct MainMenu<'a> {
    pub layout: &'a MainMenuLayout,
    /// When it opened, on the menus' clock (seconds).
    pub opened: f64,
    /// Its rows, top first.
    pub rows: &'a [&'a str],
    pub focus: Focus,
    /// The small print at the bottom right (the gamertag; may be empty).
    pub small_print: &'a str,
}

/// What goes behind both screens at `clock`: nothing over the scene, or
/// the navy gradient with `framing` (the layouts' `framing`) faintly over
/// it. The same list from frame to frame unless the framing scrolls.
pub fn background(
    clock: f64,
    backdrop: Backdrop,
    framing: &BitmapWidget,
    art: &dyn ArtSource,
    fonts: &Fonts,
    space: Space,
) -> DrawList {
    let mut p = Painter::new(space, art, fonts, clock);
    if backdrop == Backdrop::Still {
        p.gradient_px(p.space.window(), colour::NAVY_TOP, colour::NAVY_BOTTOM);
        p.bitmap(framing, framing.corner, Anchor::TopLeft, FRAMING_ALPHA);
    }
    p.finish()
}

/// The start screen at `clock`.
pub fn start(clock: f64, s: &Start, art: &dyn ArtSource, fonts: &Fonts, space: Space) -> DrawList {
    let mut p = Painter::new(space, art, fonts, clock);
    let age = (clock - s.opened) as f32;
    p.bitmaps(&s.layout.art, [0.0, 0.0], Anchor::TopLeft, age, 1.0);
    let t = &s.layout.press_start;
    let (fade, offset) = anim::intro(t.intro.as_ref(), age, t.delay_ms);
    let pulse = if t.pulsating {
        anim::pulse(clock - s.opened)
    } else {
        1.0
    };
    p.text_widget(t, offset, fade * pulse, s.text);
    let t = &s.layout.small_print;
    p.text_widget(t, [0.0, 0.0], 1.0, s.small_print);
    p.finish()
}

/// The first row shown of `rows` when `visible` show at once: the list
/// keeps the focused row in sight.
pub fn first_shown(rows: usize, visible: usize, focus: usize) -> usize {
    if rows <= visible || visible == 0 {
        return 0;
    }
    focus.saturating_sub(visible - 1).min(rows - visible)
}

/// The main menu at `clock`.
pub fn main_menu(
    clock: f64,
    m: &MainMenu,
    art: &dyn ArtSource,
    fonts: &Fonts,
    space: Space,
) -> DrawList {
    let mut p = Painter::new(space, art, fonts, clock);
    let age = (clock - m.opened) as f32;
    p.bitmaps(&m.layout.art, [0.0, 0.0], Anchor::TopLeft, age, 1.0);
    let list = &m.layout.list;
    let (fade, [fx, fy]) = paint::relative(list.intro.as_ref(), age, list.delay_ms);
    let first = first_shown(m.rows.len(), list.visible, m.focus.item);
    let shown: Vec<(usize, [f32; 2], f32)> = (first..m.rows.len())
        .take(list.visible)
        .enumerate()
        .map(|(slot, k)| {
            let [x, y] = list.corner(slot);
            let alpha = m.focus.alpha_in(&list.skin.items, k, clock);
            (k, [x + fx, y + fy], alpha * fade)
        })
        .collect();
    // Every row's bitmaps, then every row's text over them (text is the
    // skin's deepest piece).
    for &(_, corner, alpha) in &shown {
        p.bitmaps(&list.skin.bitmaps, corner, Anchor::BottomLeft, age, alpha);
    }
    for &(k, corner, alpha) in &shown {
        // A shadow is listed after its label, and drawn under it.
        for t in list.skin.texts.iter().rev() {
            p.text_widget(t, corner, alpha, m.rows[k]);
        }
    }
    let t = &m.layout.small_print;
    p.text_widget(t, [0.0, 0.0], fade, m.small_print);
    p.finish()
}

/// Which of the main menu's rows is under window pixel `at` (the mouse):
/// the row whose label's box it is in.
pub fn main_menu_row_at(m: &MainMenu, space: Space, at: [f32; 2]) -> Option<usize> {
    let list = &m.layout.list;
    let label = list.skin.texts.first()?.bounds;
    let point = space.unit(at);
    let first = first_shown(m.rows.len(), list.visible, m.focus.item);
    (first..m.rows.len())
        .take(list.visible)
        .enumerate()
        .find(|&(slot, _)| layout::contains(layout::offset(label, list.corner(slot)), point))
        .map(|(_, k)| k)
}

/// What a text slot of a screen shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Words<'a> {
    /// The tag's own words, through `wording` (hidden when it refuses
    /// them, or the tag has none).
    Tag,
    /// The tag's words through `wording`, else these.
    TagOr(Cow<'a, str>),
    /// Our own words.
    Ours(Cow<'a, str>),
    /// Nothing.
    Hide,
}

impl<'a> Words<'a> {
    /// The words shown for a slot whose tag has `tag` (none when hidden).
    pub fn resolve(&self, tag: Option<&str>) -> Option<Cow<'a, str>> {
        let tag = || tag.and_then(wording).map(Cow::Owned);
        match self {
            Words::Tag => tag(),
            Words::TagOr(ours) => tag().or_else(|| Some(ours.clone())),
            Words::Ours(ours) => Some(ours.clone()),
            Words::Hide => None,
        }
    }
}

/// Words the tags would have that never show (menu.md 4): the Xbox's
/// own names, Bungie's, clans and voice.
const REFUSED: [&str; 4] = ["xbox", "bungie", "clan", "voice"];

/// A tag's words as the lobby may show them: "Xbox Live" becomes ONLINE
/// in capitals and "online" otherwise ("Online" starting a sentence), and
/// an "on" just before it goes ("play on Xbox Live" reads "play online");
/// none when they still mention the Xbox, Bungie, clans or voice.
pub fn wording(s: &str) -> Option<String> {
    const NAME: &str = "xbox live";
    let lower = s.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    while let Some(k) = lower[at..].find(NAME) {
        let start = at + k;
        let found = &s[start..start + NAME.len()];
        out.push_str(&s[at..start]);
        let kept = out.trim_end_matches(' ').len();
        let word = out[..kept].rsplit([' ', '\n']).next().unwrap_or("");
        if kept < out.len() && word.eq_ignore_ascii_case("on") {
            out.truncate(kept - word.len());
        }
        let before = out.trim_end();
        let starts = before.is_empty() || before.ends_with(['.', '!', '?', ':', '\n']);
        out.push_str(if found == "XBOX LIVE" {
            "ONLINE"
        } else if starts {
            "Online"
        } else {
            "online"
        });
        at = start + NAME.len();
    }
    out.push_str(&s[at..]);
    let left = out.to_ascii_lowercase();
    if REFUSED.iter().any(|w| left.contains(w)) {
        return None;
    }
    Some(out)
}

/// The format tokens in Halo 2's strings (U+E400 to U+E4FF): each one is
/// the game's value for it (a gamertag, a playlist's name).
pub fn is_token(c: char) -> bool {
    ('\u{e400}'..='\u{e4ff}').contains(&c)
}

/// `text` with its tokens given by `tokens`; none when one isn't known.
fn fill_tokens(text: &str, tokens: &dyn Fn(char) -> Option<String>) -> Option<String> {
    if !text.chars().any(is_token) {
        return Some(text.to_string());
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if is_token(c) {
            out.push_str(&tokens(c)?);
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// One row of a list: a cell for each of its skin's texts in order (the
/// skin's first text, its second...), skin pictures shown at another
/// frame on this row (by their tag's last name: a level icon), and
/// whether it can be chosen (one that can't is drawn dimmer).
#[derive(Clone, Debug, PartialEq)]
pub struct Row<'a> {
    pub cells: Vec<Cow<'a, str>>,
    pub frames: Vec<(&'a str, usize)>,
    pub enabled: bool,
}

impl<'a> Row<'a> {
    /// A row with one cell.
    pub fn new(text: impl Into<Cow<'a, str>>) -> Row<'a> {
        Row {
            cells: vec![text.into()],
            frames: Vec::new(),
            enabled: true,
        }
    }
}

/// How dim a row or button that can't be chosen is. Our own estimate.
pub const DISABLED: f32 = 0.4;

/// A list's rows and where it is: its focus (none when the focus is
/// elsewhere on the screen: every row rests), and its first row shown
/// (`first_shown` keeps the focused one in sight).
#[derive(Clone, Copy, Debug)]
pub struct ListState<'a> {
    pub rows: &'a [Row<'a>],
    pub focus: Option<Focus>,
    pub first: usize,
}

/// What a screen's picture shows instead of its tag's (a map's picture, a
/// level icon).
#[derive(Clone, Debug, PartialEq)]
pub enum Show {
    Keep,
    Hide,
    /// Another bitmap tag at a frame of it, at the same place and scale.
    Picture(String, usize),
}

/// A screen's state for `screen`.
pub struct ScreenState<'a> {
    /// Which pane shows.
    pub pane: usize,
    /// When it opened, on the menus' clock (seconds).
    pub opened: f64,
    /// The shell's chrome, for the header and the legend.
    pub chrome: &'a ChromeLayout,
    /// What each text slot (and button) shows.
    pub words: &'a dyn Fn(&SlotText) -> Words<'a>,
    /// The values of the tags' format tokens; a text with one that has
    /// none is hidden.
    pub tokens: &'a dyn Fn(char) -> Option<String>,
    /// The pane's lists, in its order (one missing draws nothing).
    pub lists: &'a [ListState<'a>],
    /// Which button has the focus, and those that can't be pressed.
    pub button_focus: Option<usize>,
    pub disabled_buttons: &'a [usize],
    /// The header's words (the tag's header string).
    pub header: Words<'a>,
    /// The legend's buttons, and whether to name the keys for them.
    pub legend: &'a [LegendItem],
    pub keyboard: bool,
    /// What each of the pane's pictures shows.
    pub pictures: &'a dyn Fn(&BitmapWidget) -> Show,
}

/// What a click at a hit area presses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiHit {
    /// Row `k` of list `list` (its index in all of the list's rows).
    Row(usize, usize),
    Button(usize),
    Legend(Face),
}

/// Where a click presses something: a box of window pixels (`[x0, y0,
/// x1, y1]`, +y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitArea {
    pub rect: [f32; 4],
    pub hit: UiHit,
}

impl HitArea {
    /// The first area under window pixel `at`.
    pub fn find(areas: &[HitArea], [x, y]: [f32; 2]) -> Option<UiHit> {
        areas
            .iter()
            .find(|a| x >= a.rect[0] && x < a.rect[2] && y >= a.rect[1] && y < a.rect[3])
            .map(|a| a.hit)
    }
}

/// The overlay that dims what is under a dialog (the caller draws it
/// between the screen under and the dialog).
pub fn overlay(chrome: &ChromeLayout, space: Space) -> DrawList {
    let mut list = DrawList {
        size: space.size.map(|v| v as u32),
        ..DrawList::default()
    };
    let [w, h] = space.size;
    if chrome.overlay[3] > 0.0 && w > 0.0 && h > 0.0 {
        list.quads.push(Quad {
            texture: Texture::White,
            rect: space.window(),
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [chrome.overlay; 2],
            blend: Blend::Plain,
            wrap: [false; 2],
        });
    }
    list
}

/// When the last piece of a screen's pane has come in, in seconds after
/// it opened (the lobby's `menu_settled`).
pub fn intro_end(layout: &ScreenLayout, pane: usize) -> f64 {
    let Some(p) = layout.pane(pane) else {
        return 0.0;
    };
    let end = |a: Option<&anim::Animation>, delay_ms: f32| {
        f64::from(a.map_or(0.0, anim::Animation::seconds)) + f64::from(delay_ms.max(0.0)) / 1000.0
    };
    let bitmaps = p.bitmaps.iter().map(|w| end(w.intro.as_ref(), w.delay_ms));
    let texts = p
        .texts
        .iter()
        .map(|t| end(t.widget.intro.as_ref(), t.widget.delay_ms));
    let lists = p.lists.iter().map(|l| end(l.intro.as_ref(), l.delay_ms));
    let buttons = p
        .buttons
        .iter()
        .map(|b| end(b.text.widget.intro.as_ref(), b.text.widget.delay_ms));
    bitmaps
        .chain(texts)
        .chain(lists)
        .chain(buttons)
        .fold(0.0, f64::max)
}

/// The frame of a button's picture: focused or not (`tags::BUTTON_FRAMES`;
/// menu.md 9.2 says what the pregame lobby's button's frames are).
pub fn button_frame(focused: bool) -> usize {
    crate::tags::BUTTON_FRAMES[usize::from(focused)]
}

/// A text slot's words at `age`, through the binding and the tokens:
/// none when hidden.
fn slot_words(slot: &SlotText, s: &ScreenState) -> Option<String> {
    let words = (s.words)(slot).resolve(slot.words.as_deref())?;
    fill_tokens(&words, s.tokens).filter(|w| !w.is_empty())
}

/// A text widget's look `age` seconds after its screen opened.
fn text_alpha(w: &TextWidget, age: f32, since_open: f64) -> (f32, [f32; 2]) {
    let (a, offset) = anim::intro(w.intro.as_ref(), age, w.delay_ms);
    let pulse = if w.pulsating {
        anim::pulse(since_open)
    } else {
        1.0
    };
    (a * pulse, offset)
}

/// A screen of the tags (`tags::screen_layout`) at `clock`, in this order:
/// its pane's pictures and its buttons' pictures (lower depths first), its
/// lists (each row's pictures, then their texts), its texts and buttons'
/// texts (lower depths first), then its header and its legend. The
/// overlay under a dialog is the caller's (`overlay`). Returns where
/// clicks press its rows, buttons and legend items.
pub fn screen(
    clock: f64,
    layout: &ScreenLayout,
    s: &ScreenState,
    art: &dyn ArtSource,
    fonts: &Fonts,
    space: Space,
) -> (DrawList, Vec<HitArea>) {
    let mut p = Painter::new(space, art, fonts, clock);
    let mut hits = Vec::new();
    let Some(pane) = layout.pane(s.pane) else {
        return (p.finish(), hits);
    };
    let since_open = clock - s.opened;
    let age = since_open as f32;
    pictures(&mut p, pane, s, age);
    for (k, list) in pane.lists.iter().enumerate() {
        if let Some(state) = s.lists.get(k) {
            draw_list(&mut p, list, k, state, age, clock, &mut hits);
        }
    }
    words(&mut p, pane, s, age, since_open, &mut hits);
    // The header and legend boxes stay as the globals have them, in
    // Vista's units, whatever the pane's scale (menu.md 9.2: scaled with
    // an Xbox pane, they land outside its frame).
    if layout.has_header {
        let header = s.header.resolve(layout.header_words.as_deref());
        if let Some(text) = header.and_then(|h| fill_tokens(&h, s.tokens)) {
            chrome::header(&mut p, s.chrome, layout.size, &text);
        }
    }
    let legend = chrome::legend(&mut p, s.chrome, layout.size, s.legend, s.keyboard);
    hits.extend(legend.into_iter().map(|(rect, face)| HitArea {
        rect,
        hit: UiHit::Legend(face),
    }));
    (p.finish(), hits)
}

/// A pane's pictures and its buttons' pictures, by depth.
fn pictures(p: &mut Painter, pane: &PaneLayout, s: &ScreenState, age: f32) {
    let mut shown: Vec<(BitmapWidget, f32)> = Vec::new();
    for w in &pane.bitmaps {
        match (s.pictures)(w) {
            Show::Keep => shown.push((w.clone(), 1.0)),
            Show::Hide => {}
            Show::Picture(name, frame) => shown.push((
                BitmapWidget {
                    name,
                    frame,
                    ..w.clone()
                },
                1.0,
            )),
        }
    }
    for (k, b) in pane.buttons.iter().enumerate() {
        let Some(w) = &b.bitmap else { continue };
        let focused = s.button_focus == Some(k);
        let dim = if s.disabled_buttons.contains(&k) {
            DISABLED
        } else {
            1.0
        };
        // A button with one picture shows it focused too.
        let mut frame = button_frame(focused);
        if p.picture(&w.name, frame).is_none() && p.picture(&w.name, button_frame(false)).is_some()
        {
            frame = button_frame(false);
        }
        let w = BitmapWidget { frame, ..w.clone() };
        shown.push((w, dim));
    }
    shown.sort_by_key(|(w, _)| w.depth);
    for (w, dim) in &shown {
        let (a, [dx, dy]) = anim::intro(w.intro.as_ref(), age, w.delay_ms);
        let at = [w.corner[0] + dx, w.corner[1] + dy];
        p.bitmap(w, at, Anchor::TopLeft, a * dim);
    }
}

/// A list's shown rows: each one's pictures, then their texts (one cell a
/// text), its scroll arrows, and where each row is for clicks.
fn draw_list(
    p: &mut Painter,
    list: &ListLayout,
    index: usize,
    state: &ListState,
    age: f32,
    clock: f64,
    hits: &mut Vec<HitArea>,
) {
    let (fade, [fx, fy]) = paint::relative(list.intro.as_ref(), age, list.delay_ms);
    let look = &list.skin.items;
    let rows = state.rows;
    let first = state
        .first
        .min(rows.len().saturating_sub(list.visible.max(1)));
    let shown: Vec<(usize, [f32; 2], f32)> = (first..rows.len())
        .take(list.visible)
        .enumerate()
        .map(|(slot, k)| {
            let [x, y] = list.corner(slot);
            let focus = state
                .focus
                .map_or(look.resting(), |f| f.alpha_in(look, k, clock));
            let dim = if rows[k].enabled { 1.0 } else { DISABLED };
            (k, [x + fx, y + fy], focus * fade * dim)
        })
        .collect();
    for &(k, corner, alpha) in &shown {
        let row = &rows[k];
        let mut skin: Vec<&BitmapWidget> = list.skin.bitmaps.iter().collect();
        skin.sort_by_key(|w| w.depth);
        for w in skin {
            let (a, [dx, dy]) = anim::intro(w.intro.as_ref(), age, w.delay_ms);
            let at = [corner[0] + w.corner[0] + dx, corner[1] + w.corner[1] + dy];
            let frame = row
                .frames
                .iter()
                .find(|(name, _)| last_part(&w.name).eq_ignore_ascii_case(last_part(name)));
            match frame {
                Some(&(_, frame)) => {
                    let w = BitmapWidget { frame, ..w.clone() };
                    p.bitmap(&w, at, Anchor::BottomLeft, a * alpha);
                }
                None => {
                    p.bitmap(w, at, Anchor::BottomLeft, a * alpha);
                }
            }
        }
    }
    for &(k, corner, alpha) in &shown {
        let row = &rows[k];
        // A shadow is listed after its label, and drawn under it.
        for (t, cell) in list.skin.texts.iter().zip(&row.cells).rev() {
            let bounds = layout::offset(t.bounds, corner);
            p.text_box(bounds, TextStyle::of(t, alpha), cell);
        }
        if let Some(rect) = row_box(p, list, corner) {
            hits.push(HitArea {
                rect: p.space.rect(rect),
                hit: UiHit::Row(index, k),
            });
        }
    }
    if let Some(arrows) = &list.skin.arrows {
        // Both from the list's corner, as the skin places them; each shows
        // while there are rows that way.
        let more = [first > 0, first + list.visible < rows.len()];
        for (frame, at) in [arrows.up, arrows.down].into_iter().enumerate() {
            if more[frame] {
                let w = BitmapWidget {
                    frame,
                    ..BitmapWidget::new(&arrows.bitmap, [0.0, 0.0])
                };
                let at = [list.corner[0] + fx + at[0], list.corner[1] + fy + at[1]];
                p.bitmap(&w, at, Anchor::BottomLeft, fade);
            }
        }
    }
}

/// A row's box (UI units) at `corner`: its skin's texts' boxes and the
/// pictures that count towards its size.
fn row_box(p: &Painter, list: &ListLayout, corner: [f32; 2]) -> Option<[f32; 4]> {
    let texts = list.skin.texts.iter().map(|t| t.bounds);
    let pictures = list
        .skin
        .bitmaps
        .iter()
        .filter(|w| !w.ignore_for_list_size)
        .filter_map(|w| {
            let [bw, bh] = p.size(w)?;
            let [x, y] = w.corner;
            Some([x, y + bh, x + bw, y])
        });
    let union = texts.chain(pictures).reduce(|a, b| {
        [
            a[0].min(b[0]),
            a[1].max(b[1]),
            a[2].max(b[2]),
            a[3].min(b[3]),
        ]
    })?;
    Some(layout::offset(union, corner))
}

/// A pane's texts and its buttons' texts by depth, and where each button
/// is for clicks (its text's box and its picture's).
fn words(
    p: &mut Painter,
    pane: &PaneLayout,
    s: &ScreenState,
    age: f32,
    since_open: f64,
    hits: &mut Vec<HitArea>,
) {
    let mut texts: Vec<(&SlotText, f32)> = pane.texts.iter().map(|t| (t, 1.0)).collect();
    for (k, b) in pane.buttons.iter().enumerate() {
        let dim = if s.disabled_buttons.contains(&k) {
            DISABLED
        } else {
            1.0
        };
        texts.push((&b.text, dim));
        let mut rect = b.text.widget.bounds;
        if let Some([bw, bh]) = b.bitmap.as_ref().and_then(|w| p.size(w)) {
            let [x, y] = b.bitmap.as_ref().map_or([0.0; 2], |w| w.corner);
            rect = [
                rect[0].min(x),
                rect[1].max(y),
                rect[2].max(x + bw),
                rect[3].min(y - bh),
            ];
        }
        hits.push(HitArea {
            rect: p.space.rect(rect),
            hit: UiHit::Button(k),
        });
    }
    texts.sort_by_key(|(t, _)| t.widget.depth);
    for (t, dim) in texts {
        let Some(words) = slot_words(t, s) else {
            continue;
        };
        let (a, offset) = text_alpha(&t.widget, age, since_open);
        let bounds = layout::offset(t.widget.bounds, offset);
        p.text_box(bounds, TextStyle::of(&t.widget, a * dim), &words);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::{ArtImage, FlatArt, MemoryArt};
    use crate::layout::ButtonLayout;
    use crate::paint::{Resources, Texture};
    use crate::Font;

    const SIZES: [(u32, u32); 8] = [
        (1920, 1080),
        (1280, 720),
        (1024, 768),
        (2560, 1080),
        (640, 360),
        (600, 1000),
        (1, 1),
        (0, 0),
    ];

    const ROWS: [&str; 3] = ["ONLINE", "SPLIT SCREEN", "SETTINGS"];

    fn start_state(layout: &StartLayout) -> Start<'_> {
        Start {
            layout,
            opened: 0.0,
            text: PRESS_START,
            small_print: "JOHN",
        }
    }

    fn main_state<'a>(layout: &'a MainMenuLayout, rows: &'a [&'a str]) -> MainMenu<'a> {
        MainMenu {
            layout,
            opened: 0.0,
            rows,
            focus: Focus::on(0, 0.0),
            small_print: "JOHN",
        }
    }

    /// Draws a list on the CPU, checking every quad is a real box.
    fn render(list: &DrawList, art: &dyn ArtSource, fonts: &Fonts) -> Vec<u32> {
        let [w, h] = list.size.map(|v| v as usize);
        for q in &list.quads {
            assert!(q.rect.iter().all(|v| v.is_finite()), "{q:?}");
            assert!(q.rect[2] > q.rect[0] && q.rect[3] > q.rect[1], "{q:?}");
        }
        let mut px = vec![0u32; w * h];
        crate::cpu::draw_u32(list, &mut px, w, h, &Resources { art, fonts });
        px
    }

    #[test]
    fn both_screens_draw_flat_at_any_size() {
        let fonts = Fonts::fallback();
        let (start_layout, main_layout) = (StartLayout::default(), MainMenuLayout::default());
        for (w, h) in SIZES {
            let space = Space::new(w, h);
            for clock in [0.0, 0.1, 3.0, 61.7] {
                let s = start(clock, &start_state(&start_layout), &FlatArt, &fonts, space);
                render(&s, &FlatArt, &fonts);
                let m = main_menu(
                    clock,
                    &main_state(&main_layout, &ROWS),
                    &FlatArt,
                    &fonts,
                    space,
                );
                render(&m, &FlatArt, &fonts);
                let framing = &start_layout.framing;
                let b = background(clock, Backdrop::Still, framing, &FlatArt, &fonts, space);
                let px = render(&b, &FlatArt, &fonts);
                if w > 100 {
                    assert!(
                        s.quads.len() > 10 && m.quads.len() > 20,
                        "{w}x{h} at {clock}"
                    );
                    // The navy is everywhere behind.
                    assert!(px.iter().all(|&p| p != 0));
                }
            }
        }
        // More rows than show at once, and none.
        let many: Vec<String> = (0..9).map(|k| format!("ROW {k}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut m = main_state(&main_layout, &many);
        m.focus = Focus::on(8, 0.0);
        main_menu(3.0, &m, &FlatArt, &fonts, Space::new(1280, 720));
        main_menu(
            3.0,
            &main_state(&main_layout, &[]),
            &FlatArt,
            &fonts,
            Space::new(1280, 720),
        );
    }

    /// The main menu colour's glyph quads' alphas, grouped by the row
    /// (screen y) they're on.
    fn row_alphas(list: &DrawList) -> Vec<(i32, f32)> {
        let mut rows: Vec<(i32, f32)> = Vec::new();
        for q in &list.quads {
            let c = q.color[0];
            if q.texture != Texture::Fallback || c[..3] != colour::MAIN_MENU {
                continue;
            }
            let y = q.rect[3].round() as i32;
            if rows.last().is_none_or(|r| r.0 != y) {
                rows.push((y, c[3]));
            }
        }
        rows
    }

    #[test]
    fn the_focused_row_is_lit_and_the_others_at_half() {
        let fonts = Fonts::fallback();
        let layout = MainMenuLayout::default();
        let mut m = main_state(&layout, &ROWS);
        m.focus = Focus::on(0, 0.0).moved(1, 2.0);
        let space = Space::new(1920, 1080);
        let settled = row_alphas(&main_menu(5.0, &m, &FlatArt, &fonts, space));
        assert_eq!(settled.len(), 3);
        assert!(settled[0].0 < settled[1].0 && settled[1].0 < settled[2].0);
        assert_eq!(
            settled.iter().map(|r| r.1).collect::<Vec<_>>(),
            [0.5, 1.0, 0.5]
        );
        // 60 ms after the move, the new row is halfway up and the old one
        // on its way down.
        let moving = row_alphas(&main_menu(2.06, &m, &FlatArt, &fonts, space));
        assert!((moving[1].1 - 0.75).abs() < 1e-4);
        assert!(moving[0].1 > 0.75 && moving[0].1 < 1.0);
        // The rows fade in with the list when it opens.
        let fresh = main_state(&layout, &ROWS);
        let opening = row_alphas(&main_menu(0.125, &fresh, &FlatArt, &fonts, space));
        assert!((opening[0].1 - 0.5).abs() < 1e-4);
        assert!((opening[1].1 - 0.25).abs() < 1e-4);
        // Each label has its shadow drawn first, lower down.
        let list = main_menu(5.0, &m, &FlatArt, &fonts, space);
        let shadow = list
            .quads
            .iter()
            .position(|q| q.color[0][..3] == colour::MAIN_MENU_SHADOW)
            .unwrap();
        let label = list
            .quads
            .iter()
            .position(|q| q.color[0][..3] == colour::MAIN_MENU)
            .unwrap();
        assert!(shadow < label);
        assert!(list.quads[shadow].rect[1] > list.quads[label].rect[1]);
    }

    #[test]
    fn press_start_fades_in_then_pulses() {
        let fonts = Fonts::fallback();
        let layout = StartLayout::default();
        let s = Start {
            small_print: "",
            ..start_state(&layout)
        };
        let space = Space::new(1280, 720);
        let alpha = |clock: f64| {
            start(clock, &s, &FlatArt, &fonts, space)
                .quads
                .iter()
                .find(|q| q.texture == Texture::Fallback && q.color[0][..3] == colour::PRESS_START)
                .map(|q| q.color[0][3])
        };
        // Clear when the screen opens, so nothing is drawn.
        assert_eq!(alpha(0.0), None);
        assert!((alpha(1.5).unwrap() - 1.0).abs() < 1e-4);
        assert!((alpha(2.25).unwrap() - 0.2).abs() < 1e-4);
        // Centred under the logo.
        let list = start(1.5, &s, &FlatArt, &fonts, space);
        let glyphs: Vec<_> = list
            .quads
            .iter()
            .filter(|q| q.texture == Texture::Fallback && q.color[0][..3] == colour::PRESS_START)
            .collect();
        assert_eq!(
            glyphs.len(),
            PRESS_START.chars().filter(|c| *c != ' ').count()
        );
        let (l, r) = (glyphs[0].rect[0], glyphs[glyphs.len() - 1].rect[2]);
        assert!(((l + r) * 0.5 - 640.0).abs() < 4.0, "{l} {r}");
        let y = glyphs[0].rect[1];
        assert!(y > space.at([0.0, -65.0])[1] && y < space.at([0.0, -105.0])[1]);
    }

    #[test]
    fn with_the_art_the_logo_is_its_picture() {
        let fonts = Fonts::fallback();
        let mut art = MemoryArt::new();
        art.add_image(
            layout::LOGO,
            ArtImage {
                width: 1024,
                height: 128,
                rgba: vec![200; 1024 * 128 * 4],
            },
        );
        let layout = StartLayout::default();
        let s = start_state(&layout);
        let list = start(1.0, &s, &art, &fonts, Space::new(1920, 1080));
        let logo = list
            .quads
            .iter()
            .find(|q| q.texture == Texture::Art(0))
            .unwrap();
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(near(logo.rect[0], 960.0 - 511.0 * 0.9));
        assert!(near(logo.rect[1], 540.0 - 90.0 * 0.9));
        assert!(near(logo.rect[2] - logo.rect[0], 1024.0 * 0.9));
        assert!(near(logo.rect[3] - logo.rect[1], 128.0 * 0.9));
        // No "HALO 2" lettering, and the background isn't in the list.
        assert!(list.quads.iter().all(|q| q.color[0] != colour::CHROME_TOP));
        assert!(list.quads.iter().all(|q| q.color[0] != colour::NAVY_TOP));
        // The still background is the navy, then the framing when the art
        // has it; over the scene it is nothing.
        let space = Space::new(1920, 1080);
        art.add_image(
            layout::FRAMING,
            ArtImage {
                width: 256,
                height: 160,
                rgba: vec![90; 256 * 160 * 4],
            },
        );
        let b = background(1.0, Backdrop::Still, &layout.framing, &art, &fonts, space);
        assert_eq!(b.quads.len(), 2);
        assert_eq!(b.quads[0].rect, [0.0, 0.0, 1920.0, 1080.0]);
        assert_eq!(b.quads[0].color, [colour::NAVY_TOP, colour::NAVY_BOTTOM]);
        assert_eq!(b.quads[1].color[0][3], FRAMING_ALPHA);
        // It doesn't change as the clock goes on.
        let later = background(9.0, Backdrop::Still, &layout.framing, &art, &fonts, space);
        assert_eq!(later, b);
        let scene = background(1.0, Backdrop::Scene, &layout.framing, &art, &fonts, space);
        assert!(scene.quads.is_empty());
        // Without the art, the logo is lettering, faded in with it.
        let letters = |clock: f64| {
            start(
                clock,
                &start_state(&layout),
                &FlatArt,
                &fonts,
                Space::new(1920, 1080),
            )
            .quads
            .iter()
            .filter(|q| q.color[0][..3] == colour::CHROME_TOP[..3])
            .map(|q| q.color[0][3])
            .collect::<Vec<_>>()
        };
        assert_eq!(letters(1.0), [1.0; 5]);
        assert!(letters(0.125).iter().all(|a| (a - 0.5).abs() < 1e-4));
    }

    /// A digest of a list's quads (every field's bits) and lines.
    pub(crate) fn digest(list: &DrawList) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |v: u64| {
            for b in v.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        for q in &list.quads {
            let texture = match q.texture {
                Texture::White => 1,
                Texture::Art(k) => 2 + (u64::from(k) << 8),
                Texture::Font(k) => 3 + (u64::from(k) << 8),
                Texture::Fallback => 4,
            };
            eat(texture);
            for v in q.rect.iter().chain(&q.uv).chain(q.color.iter().flatten()) {
                eat(u64::from(v.to_bits()));
            }
            eat(q.blend as u64 + 8 * u64::from(q.wrap[0]) + 16 * u64::from(q.wrap[1]));
        }
        for t in &list.texts {
            t.bytes().for_each(|b| eat(u64::from(b)));
        }
        h
    }

    /// The start screen and the main menu draw exactly as they did before
    /// screens could be drawn from any tag (2.7): their quads are pinned.
    #[test]
    fn the_start_screen_and_main_menu_are_unchanged() {
        let fonts = Fonts::fallback();
        let (start_layout, main_layout) = (StartLayout::default(), MainMenuLayout::default());
        let mut got = Vec::new();
        for (w, h) in [(1280, 720), (1920, 1080)] {
            let space = Space::new(w, h);
            for clock in [0.1, 3.0] {
                let s = start(clock, &start_state(&start_layout), &FlatArt, &fonts, space);
                let m = main_menu(
                    clock,
                    &main_state(&main_layout, &ROWS),
                    &FlatArt,
                    &fonts,
                    space,
                );
                got.push(digest(&s));
                got.push(digest(&m));
            }
        }
        assert_eq!(got, PINNED);
    }

    /// Taken from the code before 2.7, at 1280x720 and 1920x1080, clocks
    /// 0.1 and 3.0, start then main menu.
    const PINNED: [u64; 8] = [
        7197067043366583784,
        4842540297417354659,
        17459350917710010778,
        13459257294002349554,
        3003082628917096320,
        9322449806419217081,
        4061248620304994473,
        13693663462508611468,
    ];

    #[test]
    fn the_mouse_finds_the_row_under_it() {
        let layout = MainMenuLayout::default();
        let m = main_state(&layout, &ROWS);
        let space = Space::new(1280, 720);
        // Row 1's label box is from y -120 to -170, across -240 to 240.
        assert_eq!(
            main_menu_row_at(&m, space, space.at([0.0, -145.0])),
            Some(1)
        );
        assert_eq!(
            main_menu_row_at(&m, space, space.at([-200.0, -75.0])),
            Some(0)
        );
        assert_eq!(main_menu_row_at(&m, space, space.at([300.0, -145.0])), None);
        assert_eq!(main_menu_row_at(&m, space, space.at([0.0, -300.0])), None);
        assert_eq!(first_shown(9, 6, 8), 3);
        assert_eq!(first_shown(9, 6, 2), 0);
        assert_eq!(first_shown(3, 6, 2), 0);
        assert_eq!(Font::MainMenu, layout.list.skin.texts[0].font);
    }

    #[test]
    fn wording_says_online_and_refuses_the_rest() {
        let w = |s: &str| wording(s);
        assert_eq!(w("SETTINGS").as_deref(), Some("SETTINGS"));
        assert_eq!(
            w("FIND A GAME ON XBOX LIVE").as_deref(),
            Some("FIND A GAME ONLINE")
        );
        assert_eq!(w("Xbox Live friends").as_deref(), Some("Online friends"));
        assert_eq!(
            w("Meet up on Xbox Live.").as_deref(),
            Some("Meet up online.")
        );
        assert_eq!(
            w("Done. Xbox Live is next").as_deref(),
            Some("Done. Online is next")
        );
        assert_eq!(w("Upon Xbox Live").as_deref(), Some("Upon online"));
        assert_eq!(w("on Xbox Live").as_deref(), Some("Online"));
        for refused in ["The Xbox pad", "BUNGIE NEWS", "Your clan", "Voice chat"] {
            assert_eq!(w(refused), None, "{refused}");
        }
        // The binding's choices.
        let tag = Some("PLAY ON XBOX LIVE");
        assert_eq!(Words::Tag.resolve(tag).as_deref(), Some("PLAY ONLINE"));
        assert_eq!(Words::Tag.resolve(None), None);
        assert_eq!(Words::Tag.resolve(Some("bungie")), None);
        let ours = Words::TagOr(Cow::Borrowed("OURS"));
        assert_eq!(ours.resolve(tag).as_deref(), Some("PLAY ONLINE"));
        assert_eq!(ours.resolve(Some("bungie")).as_deref(), Some("OURS"));
        assert_eq!(ours.resolve(None).as_deref(), Some("OURS"));
        let ours = Words::Ours(Cow::Borrowed("MINE"));
        assert_eq!(ours.resolve(tag).as_deref(), Some("MINE"));
        assert_eq!(Words::Hide.resolve(tag), None);
        // Tokens: each one's value, or nothing when one has none.
        let tokens = |c: char| (c == '\u{e401}').then(|| "JOHN".to_string());
        assert!(is_token('\u{e400}') && is_token('\u{e4ff}') && !is_token('A'));
        assert_eq!(
            fill_tokens("\u{e401} IS IN", &tokens).as_deref(),
            Some("JOHN IS IN")
        );
        assert_eq!(fill_tokens("\u{e402} IS IN", &tokens), None);
        assert_eq!(fill_tokens("NONE", &tokens).as_deref(), Some("NONE"));
    }

    const LABEL: [f32; 3] = [0.9, 0.9, 0.9];
    const BACKING: [f32; 4] = [0.1, 0.2, 0.3, 1.0];

    fn panel(name: &str, corner: [f32; 2], size: [f32; 2]) -> BitmapWidget {
        BitmapWidget {
            flat: layout::Flat::Panel {
                size,
                color: BACKING,
                edge: [0.5, 0.6, 0.7, 1.0],
            },
            ..BitmapWidget::new(name, corner)
        }
    }

    fn slot(key: &str, words: &str, bounds: [f32; 4]) -> SlotText {
        SlotText {
            key: key.into(),
            words: Some(words.into()),
            widget: TextWidget::new(
                Font::Body,
                [0.7, 0.7, 0.7],
                bounds,
                crate::text::Justify::Left,
            ),
        }
    }

    /// A quarter dialog of one pane in the Xbox's numbers (already
    /// scaled, as `tags::screen_layout` gives it): a plate, texts (one
    /// in the tag's words to be reworded, one refused, one with a token,
    /// one with a token that has no value), a list of three rows of two
    /// cells with arrows, and two buttons.
    fn dialog() -> ScreenLayout {
        let skin = layout::SkinLayout {
            bitmaps: vec![
                panel("made_up_backing", [0.0, 0.0], [300.0, 40.0]),
                BitmapWidget {
                    ignore_for_list_size: true,
                    ..panel("ui\\made_up\\made_up_icon", [260.0, 5.0], [30.0, 30.0])
                },
            ],
            texts: vec![
                TextWidget::new(
                    Font::Body,
                    LABEL,
                    [10.0, 35.0, 150.0, 5.0],
                    crate::text::Justify::Left,
                ),
                TextWidget::new(
                    Font::Body,
                    [0.8, 0.8, 0.8],
                    [160.0, 35.0, 250.0, 5.0],
                    crate::text::Justify::Right,
                ),
            ],
            items: crate::anim::ItemLook::default(),
            arrows: Some(layout::Arrows {
                bitmap: "made_up_arrows".into(),
                up: [-40.0, 10.0],
                down: [-40.0, -160.0],
            }),
        };
        let list = ListLayout {
            skin,
            corner: [-100.0, 20.0],
            step: 45.0,
            visible: 3,
            wraps: false,
            intro: None,
            delay_ms: 0.0,
        };
        let pane = PaneLayout {
            scale: layout::Scale::Xbox,
            bitmaps: vec![
                panel("made_up_plate", [-320.0, 260.0], [200.0, 100.0]),
                panel("made_up_gone", [100.0, 260.0], [50.0, 50.0]),
            ],
            texts: vec![
                slot(
                    "dialog_body",
                    "JOIN US ON XBOX LIVE",
                    [-300.0, 150.0, 300.0, 110.0],
                ),
                slot(
                    "dialog_refused",
                    "BUNGIE THINGS",
                    [-300.0, 100.0, 300.0, 60.0],
                ),
                slot(
                    "dialog_token",
                    "\u{e401} IS HERE",
                    [-300.0, 60.0, 300.0, 30.0],
                ),
                slot(
                    "dialog_unknown",
                    "\u{e402} IS GONE",
                    [-300.0, 30.0, 300.0, 0.0],
                ),
            ],
            lists: vec![list],
            buttons: vec![
                ButtonLayout {
                    text: slot("dialog_button", "PRESS", [-300.0, -200.0, -100.0, -240.0]),
                    bitmap: Some(panel("made_up_button", [-310.0, -195.0], [220.0, 50.0])),
                },
                ButtonLayout {
                    text: slot("dialog_single", "SINGLE", [0.0, -200.0, 200.0, -240.0]),
                    bitmap: Some(panel("made_up_single", [-10.0, -195.0], [220.0, 50.0])),
                },
            ],
            players: Vec::new(),
        };
        ScreenLayout {
            id: None,
            path: "ui\\screens\\made_up\\dialog".into(),
            size: layout::DialogSize::Quarter,
            has_header: true,
            header_id: "dialog_header".into(),
            header_words: Some("MADE UP".into()),
            button_key: 2,
            overlay: true,
            pane_per_item: false,
            panes: vec![pane],
            strings: Vec::new(),
        }
    }

    /// Five rows of two cells, the second disabled and showing frame 2 of
    /// the icon.
    fn dialog_rows() -> Vec<Row<'static>> {
        (0..5)
            .map(|k| Row {
                cells: vec![format!("ROW {k}").into(), "CELL".into()],
                frames: if k == 1 {
                    vec![("made_up_icon", 2)]
                } else {
                    Vec::new()
                },
                enabled: k != 1,
            })
            .collect()
    }

    /// Draws `dialog()` with the second row focused (rows 1 to 3 shown),
    /// `button_focus` and `pictures`.
    fn draw_dialog(
        art: &dyn ArtSource,
        button_focus: Option<usize>,
        pictures: &dyn Fn(&BitmapWidget) -> Show,
    ) -> (DrawList, Vec<HitArea>, ScreenLayout, Space) {
        let fonts = Fonts::fallback();
        let layout = dialog();
        let chrome = ChromeLayout::default();
        let rows = dialog_rows();
        let lists = [ListState {
            rows: &rows,
            focus: Some(Focus::on(1, 0.0)),
            first: 1,
        }];
        let words = |t: &SlotText| match t.key.as_str() {
            "dialog_button" => Words::Ours(Cow::Borrowed("OURS")),
            _ => Words::Tag,
        };
        let tokens = |c: char| (c == '\u{e401}').then(|| "JOHN".to_string());
        let legend = [
            LegendItem::new(Face::A, "SELECT"),
            LegendItem::new(Face::B, "BACK"),
        ];
        let s = ScreenState {
            pane: 0,
            opened: 0.0,
            chrome: &chrome,
            words: &words,
            tokens: &tokens,
            lists: &lists,
            button_focus,
            disabled_buttons: &[],
            header: Words::Tag,
            legend: &legend,
            keyboard: false,
            pictures,
        };
        let space = Space::new(1280, 720);
        let (list, hits) = screen(10.0, &layout, &s, art, &fonts, space);
        (list, hits, layout, space)
    }

    /// The middle of a box (UI units).
    fn middle([l, t, r, b]: [f32; 4]) -> [f32; 2] {
        [(l + r) * 0.5, (t + b) * 0.5]
    }

    #[test]
    fn a_screen_draws_its_words_rows_and_buttons_where_clicks_find_them() {
        let (list, hits, layout, space) = draw_dialog(&FlatArt, Some(0), &|_| Show::Keep);
        render(&list, &FlatArt, &Fonts::fallback());
        let texts = list.texts.join("|");
        // The tag's words reworded, a refused one and one with a token
        // that has no value hidden, our own for the button.
        for shown in [
            "JOIN US ONLINE",
            "JOHN IS HERE",
            "OURS",
            "SINGLE",
            "MADE UP",
        ] {
            assert!(list.texts.iter().any(|t| t == shown), "{shown}: {texts}");
        }
        for hidden in ["BUNGIE", "GONE", "XBOX", "PRESS"] {
            assert!(!texts.contains(hidden), "{hidden}: {texts}");
        }
        // Rows 1 to 3 shown, each cell in its text.
        for k in 1..4 {
            assert!(
                list.texts.iter().any(|t| *t == format!("ROW {k}")),
                "{texts}"
            );
        }
        assert!(
            !texts.contains("ROW 0") && !texts.contains("ROW 4"),
            "{texts}"
        );
        assert_eq!(list.texts.iter().filter(|t| *t == "CELL").count(), 3);
        // A click in a row's label finds that row.
        let l = &layout.panes[0].lists[0];
        let label = l.skin.texts[0].bounds;
        for slot in 0..3 {
            let at = space.at(middle(layout::offset(label, l.corner(slot))));
            assert_eq!(HitArea::find(&hits, at), Some(UiHit::Row(0, slot + 1)));
        }
        // A button's text and its picture are both it.
        let b = &layout.panes[0].buttons[0];
        assert_eq!(
            HitArea::find(&hits, space.at(middle(b.text.widget.bounds))),
            Some(UiHit::Button(0))
        );
        let [x, y] = b.bitmap.as_ref().unwrap().corner;
        assert_eq!(
            HitArea::find(&hits, space.at([x + 210.0, y - 45.0])),
            Some(UiHit::Button(0))
        );
        assert_eq!(HitArea::find(&hits, [1.0, 1.0]), None);
        // Each legend item's box holds its glyph and words.
        let legend: Vec<&HitArea> = hits
            .iter()
            .filter(|h| matches!(h.hit, UiHit::Legend(_)))
            .collect();
        assert_eq!(legend.len(), 2);
        for h in &legend {
            let [x0, y0, x1, y1] = h.rect;
            let inside = list.quads.iter().filter(|q| {
                let [cx, cy] = [(q.rect[0] + q.rect[2]) * 0.5, (q.rect[1] + q.rect[3]) * 0.5];
                q.texture != Texture::White && cx >= x0 && cx < x1 && cy >= y0 && cy < y1
            });
            assert!(inside.count() >= 4, "{h:?}");
            assert_eq!(
                HitArea::find(&hits, [(x0 + x1) * 0.5, (y0 + y1) * 0.5]),
                Some(h.hit)
            );
        }
        assert_eq!(legend[0].hit, UiHit::Legend(Face::A));
        // The header stays in the globals' quarter box (Vista's units),
        // though the pane is the Xbox's.
        let chrome = ChromeLayout::default();
        let header = chrome.header(layout::DialogSize::Quarter);
        let [x0, y0, x1, y1] = space.rect(header.bounds);
        let glyphs: Vec<&Quad> = list
            .quads
            .iter()
            .filter(|q| q.texture == Texture::Fallback && q.color[0][..3] == header.color)
            .collect();
        assert_eq!(glyphs.len(), "MADEUP".len());
        for q in glyphs {
            let [cx, cy] = [(q.rect[0] + q.rect[2]) * 0.5, (q.rect[1] + q.rect[3]) * 0.5];
            assert!(
                cx >= x0 && cx <= x1 && cy >= y0 - 2.0 && cy <= y1 + 2.0,
                "{q:?}"
            );
        }
        // The disabled row is dimmed: the focused row's labels at its
        // focused alpha times DISABLED, the others resting.
        let alphas: Vec<f32> = list
            .quads
            .iter()
            .filter(|q| q.texture == Texture::Fallback && q.color[0][..3] == LABEL)
            .map(|q| q.color[0][3])
            .collect();
        let resting = l.skin.items.resting();
        assert!(
            alphas.iter().any(|a| (a - DISABLED).abs() < 1e-4),
            "{alphas:?}"
        );
        assert!(
            alphas.iter().any(|a| (a - resting).abs() < 1e-4),
            "{alphas:?}"
        );
    }

    /// A bitmap of `frames` images of 8 by 8 pixels.
    fn frames(art: &mut MemoryArt, name: &str, frames: usize) {
        let images = (0..frames)
            .map(|k| ArtImage {
                width: 8,
                height: 8,
                rgba: vec![40 * k as u8 + 20; 8 * 8 * 4],
            })
            .collect();
        art.add(
            name,
            crate::art::Bitmap {
                images,
                sequences: Vec::new(),
            },
        );
    }

    /// Every art quad's id.
    fn art_ids(list: &DrawList) -> Vec<u32> {
        list.quads
            .iter()
            .filter_map(|q| match q.texture {
                Texture::Art(id) => Some(id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn rows_buttons_and_pictures_show_the_frames_asked_for() {
        let mut art = MemoryArt::new();
        frames(&mut art, "made_up_icon", 3); // ids 0 to 2
        frames(&mut art, "made_up_button", 2); // 3 and 4
        frames(&mut art, "made_up_single", 1); // 5
        frames(&mut art, "made_up_arrows", 2); // 6 and 7
        frames(&mut art, "made_up_map", 1); // 8
        frames(&mut art, "made_up_plate", 1); // 9
        let pictures = |w: &BitmapWidget| match w.name.as_str() {
            "made_up_plate" => Show::Picture("made_up_map".into(), 0),
            "made_up_gone" => Show::Hide,
            _ => Show::Keep,
        };
        let (list, _, layout, space) = draw_dialog(&art, Some(0), &pictures);
        render(&list, &art, &Fonts::fallback());
        let ids = art_ids(&list);
        let count = |id: u32| ids.iter().filter(|&&k| k == id).count();
        // The icon: frame 2 on the row that asks for it, else its own.
        assert_eq!((count(0), count(1), count(2)), (2, 0, 1));
        // The focused button is lit; a button with one picture shows it
        // focused or not.
        assert_eq!((count(3), count(4), count(5)), (0, 1, 1));
        // Both arrows: rows above and below those shown.
        assert_eq!((count(6), count(7)), (1, 1));
        // The plate shows the map instead; the hidden picture is gone.
        assert_eq!((count(8), count(9)), (1, 0));
        let gone = layout.panes[0].bitmaps[1].corner;
        assert!(!list
            .quads
            .iter()
            .any(|q| { q.texture == Texture::White && q.rect[0] == space.at(gone)[0] }));
        // The up arrow's bottom left at the list's corner plus its place.
        let l = &layout.panes[0].lists[0];
        let up = list
            .quads
            .iter()
            .find(|q| q.texture == Texture::Art(6))
            .unwrap();
        let arrows = l.skin.arrows.as_ref().unwrap();
        let [x, y] = space.at([l.corner[0] + arrows.up[0], l.corner[1] + arrows.up[1]]);
        assert!(
            (up.rect[0] - x).abs() < 1e-3 && (up.rect[3] - y).abs() < 1e-3,
            "{up:?}"
        );
        // Unfocused, the button rests.
        let (list, ..) = draw_dialog(&art, None, &pictures);
        let ids = art_ids(&list);
        assert!(ids.contains(&3) && !ids.contains(&4));
    }

    #[test]
    fn a_screen_says_when_it_has_come_in_and_dims_what_is_under_it() {
        let mut layout = dialog();
        assert_eq!(intro_end(&layout, 0), 0.0);
        let p = &mut layout.panes[0];
        p.texts[1].widget.intro = Some(crate::anim::Animation::focus_in());
        p.texts[1].widget.delay_ms = 250.0;
        p.lists[0].delay_ms = 100.0;
        let end = f64::from(crate::anim::Animation::focus_in().seconds()) + 0.25;
        assert!((intro_end(&layout, 0) - end).abs() < 1e-6);
        // A pane past the last is the last; no panes, nothing to come in.
        assert!((intro_end(&layout, 7) - end).abs() < 1e-6);
        layout.panes.clear();
        assert_eq!(intro_end(&layout, 0), 0.0);
        let fonts = Fonts::fallback();
        let chrome = ChromeLayout::default();
        let (empty, hits) = screen(
            1.0,
            &layout,
            &ScreenState {
                pane: 0,
                opened: 0.0,
                chrome: &chrome,
                words: &|_| Words::Tag,
                tokens: &|_| None,
                lists: &[],
                button_focus: None,
                disabled_buttons: &[],
                header: Words::Tag,
                legend: &[],
                keyboard: false,
                pictures: &|_| Show::Keep,
            },
            &FlatArt,
            &fonts,
            Space::new(640, 360),
        );
        assert!(empty.quads.is_empty() && hits.is_empty());
        // The overlay: one quad over the window in the globals' colour.
        let o = overlay(&chrome, Space::new(640, 360));
        assert_eq!(o.quads.len(), 1);
        assert_eq!(o.quads[0].rect, [0.0, 0.0, 640.0, 360.0]);
        assert_eq!(o.quads[0].color, [chrome.overlay; 2]);
        assert!(overlay(&chrome, Space::new(0, 0)).quads.is_empty());
        let clear = ChromeLayout {
            overlay: [0.0; 4],
            ..ChromeLayout::default()
        };
        assert!(overlay(&clear, Space::new(640, 360)).quads.is_empty());
    }
}
