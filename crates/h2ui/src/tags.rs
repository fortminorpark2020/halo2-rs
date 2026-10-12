//! The start screen's and the main menu's layouts from a mainmenu.map's UI
//! tags (`blam_cache::ui`): where each piece goes, which bitmap it shows,
//! the main menu's list and its skin, and the animations that bring the
//! pieces in, read from `wgit start_screen`, `wgit main_menu` and `wgit
//! game_shell_background` (the framing), the list's `skin` and the
//! globals' (`wigl`) screen animations. Screens are found by their whole
//! tag names, else by the last part of them (menu.md 9.2 E and F).
//!
//! Each piece the tags don't have keeps the built-in numbers (`layout`). A
//! bitmap read from the tags keeps the flat shape of the built-in bitmap
//! of the same name, so it still draws when its picture can't be had.
//! Every piece notes where it came from, a line each, for the log.

use crate::anim::{Animation, ItemLook, Keyframe};
use crate::art::{last_part, ArtSource};
use crate::catalog::ScreenId;
use crate::layout::{
    self, colour, Arrows, BitmapWidget, ButtonLayout, ChromeLayout, Flat, ListLayout,
    MainMenuLayout, PaneLayout, PlayersLayout, Scale, ScreenLayout, SkinLayout, SlotText,
    StartLayout, TextWidget,
};
use crate::paint::Blend;
use crate::text::Justify;
use blam_cache::ui::{self, Globals, Pane, Screen, Ui};

/// The screens drawn from the tags.
pub const START_SCREEN: &str = "ui\\screens\\game_shell\\start_screen\\start_screen";
pub const MAIN_MENU: &str = "ui\\screens\\game_shell\\main_menu_screen\\main_menu";
pub const BACKGROUND: &str = "ui\\screens\\game_shell\\main_menu_screen\\game_shell_background";
/// The string id of the start screen's "PRESS ANY KEY TO CONTINUE" in
/// Vista's tags; without it, the screen's first pulsating text is taken.
pub const PRESS_START_ID: &str = "start_screen_0";
/// A list row's step outside this (UI units) is taken as a misread.
const STEP_RANGE: std::ops::RangeInclusive<f32> = 8.0..=400.0;

/// A screen by its whole tag name, else the first whose name ends in the
/// same last part.
pub fn screen<'a>(ui: &'a Ui, name: &str) -> Option<&'a Screen> {
    ui.screen(name).or_else(|| {
        let short = last_part(name);
        ui.screens
            .iter()
            .find(|s| last_part(&s.name).eq_ignore_ascii_case(short))
    })
}

/// A screen's first pane, or why it can't be had.
fn pane<'a>(ui: Option<&'a Ui>, name: &str) -> Result<(&'a Screen, &'a Pane), String> {
    let ui = ui.ok_or("no UI tags were read")?;
    let s = screen(ui, name).ok_or_else(|| format!("no {} screen in the tags", last_part(name)))?;
    let p = s
        .panes
        .first()
        .ok_or_else(|| format!("{} has no panes", s.name))?;
    Ok((s, p))
}

fn note(sources: &mut Vec<String>, line: String) {
    sources.push(format!("menus: {line}"));
}

/// A number from the tags, or `or` when it's no number.
fn finite(v: f32, or: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        or
    }
}

/// A tag animation; none when it has no keyframes.
pub fn animation(a: &ui::Animation) -> Option<Animation> {
    if a.keyframes.is_empty() {
        return None;
    }
    Some(Animation {
        period_ms: a.period_ms.max(0) as f32,
        keyframes: a
            .keyframes
            .iter()
            .map(|k| Keyframe {
                alpha: finite(k.alpha, 1.0),
                position: k.position.map(|v| finite(v, 0.0)),
            })
            .collect(),
    })
}

/// The intro of the globals' screen animation `index`.
fn intro(g: &Globals, index: Option<usize>) -> Option<Animation> {
    animation(&g.animations.get(index?)?.intro)
}

/// A box of the tags as `[left, top, right, bottom]`.
fn bounds(r: ui::Rect) -> [f32; 4] {
    [r.left, r.top, r.right, r.bottom].map(f32::from)
}

/// A tag's bitmap as a widget (none when it names no bitmap tag), with
/// the flat shape of the bitmap of the same name in `built_in`.
pub fn bitmap(b: &ui::Bitmap, g: &Globals, built_in: &[BitmapWidget]) -> Option<BitmapWidget> {
    let name = b.bitmap.as_ref()?.name.clone();
    let flat = built_in
        .iter()
        .find(|w| last_part(&w.name).eq_ignore_ascii_case(last_part(&name)))
        .map_or_else(|| flat_for(&name), |w| w.flat);
    Some(BitmapWidget {
        name,
        frame: usize::try_from(b.frame).unwrap_or(0),
        corner: b.corner.map(f32::from),
        scale: b.scale.map(|s| finite(s, 0.0)),
        blend: if b.multiply {
            Blend::Multiply
        } else {
            Blend::Plain
        },
        wraps_per_second: b.wraps_per_second.map(|w| finite(w, 0.0)),
        depth: b.depth,
        ignore_for_list_size: b.flags & ui::IGNORE_FOR_LIST_SIZE != 0,
        intro: intro(g, b.animation),
        delay_ms: f32::from(b.delay_ms),
        flat,
    })
}

/// A tag's text as a widget.
pub fn text(t: &ui::Text, g: &Globals) -> TextWidget {
    let justify = if t.flags & ui::LEFT_JUSTIFY != 0 {
        Justify::Left
    } else if t.flags & ui::RIGHT_JUSTIFY != 0 {
        Justify::Right
    } else {
        Justify::Center
    };
    TextWidget {
        font: t.font,
        color: t.color.map(|c| finite(c, 1.0).clamp(0.0, 1.0)),
        bounds: bounds(t.bounds),
        justify,
        pulsating: t.flags & ui::PULSATING != 0,
        depth: t.depth,
        intro: intro(g, t.animation),
        delay_ms: f32::from(t.delay_ms),
    }
}

/// A pane's bitmaps as widgets, flat shapes from `built_in`.
fn bitmaps(p: &Pane, g: &Globals, built_in: &[BitmapWidget]) -> Vec<BitmapWidget> {
    p.bitmaps
        .iter()
        .filter_map(|b| bitmap(b, g, built_in))
        .collect()
}

/// The text a screen fills with the profile's name or such at its bottom
/// right: the lowest of those with no string of its own (the start
/// screen's build number, left out, may have none either).
fn small_print(p: &Pane) -> Option<&ui::Text> {
    p.texts
        .iter()
        .filter(|t| t.string.is_empty())
        .min_by_key(|t| t.bounds.bottom)
}

/// A screen's pictures and their places from its tags, or `built_in`.
fn screen_art(
    found: &Result<(&Screen, &Pane), String>,
    g: Option<&Globals>,
    built_in: Vec<BitmapWidget>,
    what: &str,
    sources: &mut Vec<String>,
) -> Vec<BitmapWidget> {
    let art = match (found, g) {
        (Ok((_, p)), Some(g)) => bitmaps(p, g, &built_in),
        _ => Vec::new(),
    };
    match found {
        Ok((s, _)) if !art.is_empty() => {
            let n = art.len();
            note(
                sources,
                format!(
                    "{what} pictures and places from wgit {} ({n} bitmaps)",
                    s.name
                ),
            );
            art
        }
        Ok((s, _)) => {
            let why = format!("{} names no bitmaps", s.name);
            note(
                sources,
                format!("{what} pictures and places: built-in ({why})"),
            );
            built_in
        }
        Err(why) => {
            note(
                sources,
                format!("{what} pictures and places: built-in ({why})"),
            );
            built_in
        }
    }
}

/// The start screen's layout from the tags (`ui`; none when they weren't
/// read), noting where each piece came from. Its framing is `framing`'s.
pub fn start_layout(ui: Option<&Ui>, sources: &mut Vec<String>) -> StartLayout {
    let mut out = StartLayout::default();
    let g = ui.map(|ui| &ui.globals);
    let found = pane(ui, START_SCREEN);
    out.art = screen_art(&found, g, layout::shell_art(), "start screen", sources);
    let (Ok((s, p)), Some(g)) = (&found, g) else {
        note(sources, "start screen texts: built-in".to_string());
        return out;
    };
    let press = p
        .texts
        .iter()
        .find(|t| t.string == PRESS_START_ID)
        .or_else(|| p.texts.iter().find(|t| t.flags & ui::PULSATING != 0));
    match press {
        Some(t) => {
            out.press_start = text(t, g);
            let id = if t.string.is_empty() {
                "a pulsating text"
            } else {
                &t.string
            };
            note(sources, format!("PRESS START's box from {}'s {id}", s.name));
        }
        None => note(
            sources,
            format!("PRESS START's box: built-in ({} has no such text)", s.name),
        ),
    }
    if let Some(t) = small_print(p) {
        out.small_print = text(t, g);
        note(sources, format!("start screen small print from {}", s.name));
    }
    out
}

/// The skin's first two item animations (gaining focus, losing it), each
/// skin 11's when it has none.
fn item_look(s: &ui::ListSkin) -> ItemLook {
    let mut look = ItemLook::default();
    let read = |k: usize| s.item_animations.get(k).and_then(animation);
    if let Some(a) = read(0) {
        look.gain = a;
    }
    if let Some(a) = read(1) {
        look.lose = a;
    }
    look
}

/// A list skin from the tags, flat shapes from the main menu's built-in
/// skin.
pub fn skin(s: &ui::ListSkin, g: &Globals) -> SkinLayout {
    let built_in = layout::main_menu_skin();
    SkinLayout {
        bitmaps: s
            .bitmaps
            .iter()
            .filter_map(|b| bitmap(b, g, &built_in.bitmaps))
            .collect(),
        texts: s.texts.iter().map(|t| text(t, g)).collect(),
        items: item_look(s),
        arrows: s.arrows.as_ref().map(|t| Arrows {
            bitmap: t.name.clone(),
            up: s.arrow_up.map(f32::from),
            down: s.arrow_down.map(f32::from),
        }),
    }
}

/// How far apart a skin's items are, with its bitmaps' heights from
/// `art` (or their flat shapes'), as `ListSkin::item_height` counts.
fn step(s: &ui::ListSkin, g: &Globals, art: &dyn ArtSource) -> f32 {
    let built_in = layout::main_menu_skin();
    s.item_height(|b| {
        let Some(w) = bitmap(b, g, &built_in.bitmaps) else {
            return 0.0;
        };
        match art.picture(&w.name, w.frame) {
            Some(p) => p.size[1] * w.scale()[1],
            None => w.flat_size().map_or(0.0, |s| s[1]),
        }
    })
}

/// The main menu's layout from the tags (`ui`; none when they weren't
/// read), its list's step from the heights of `art`'s pictures, noting
/// where each piece came from. Its framing is `framing`'s.
pub fn main_menu_layout(
    ui: Option<&Ui>,
    art: &dyn ArtSource,
    sources: &mut Vec<String>,
) -> MainMenuLayout {
    let mut out = MainMenuLayout::default();
    let g = ui.map(|ui| &ui.globals);
    let found = pane(ui, MAIN_MENU);
    out.art = screen_art(&found, g, layout::shell_art(), "main menu", sources);
    let (Ok((s, p)), Some(g)) = (&found, g) else {
        note(sources, "main menu list and skin: built-in".to_string());
        return out;
    };
    match p.lists.first() {
        Some(l) => out.list = list(l, &s.name, g, art, out.list, sources),
        None => note(
            sources,
            format!("main menu list and skin: built-in ({} has no list)", s.name),
        ),
    }
    if let Some(t) = small_print(p) {
        out.small_print = text(t, g);
        note(sources, format!("main menu small print from {}", s.name));
    }
    out
}

/// A list of the screen `screen` from the tags over `built_in`: its
/// place, how many show, its intro and its skin (by its number in the
/// globals).
fn list(
    l: &ui::List,
    screen: &str,
    g: &Globals,
    art: &dyn ArtSource,
    built_in: ListLayout,
    sources: &mut Vec<String>,
) -> ListLayout {
    let (out, skin_note) = list_at(l, g, art, built_in, Scale::Vista);
    let skin_note = match skin_note {
        Ok(found) => format!("main menu list skin from {found}"),
        Err(why) => format!("main menu list skin: built-in ({why})"),
    };
    let [x, y] = out.corner;
    note(
        sources,
        format!(
            "main menu list from {screen}: at ({x}, {y}), {} shown, a row every {} units",
            out.visible, out.step
        ),
    );
    note(sources, skin_note);
    out
}

/// A list from the tags over `built_in`, its corner at `scale` (its skin,
/// shared by every screen, as the globals have it: in Vista's units, an
/// assumption checked in menu_png's pictures), and what its skin is or
/// why it is the built-in one.
fn list_at(
    l: &ui::List,
    g: &Globals,
    art: &dyn ArtSource,
    built_in: ListLayout,
    scale: Scale,
) -> (ListLayout, Result<String, String>) {
    let mut out = ListLayout {
        corner: scale.point(l.corner.map(f32::from)),
        visible: usize::try_from(l.visible)
            .ok()
            .filter(|&v| v > 0)
            .unwrap_or(built_in.visible),
        wraps: l.flags & ui::LIST_WRAPS != 0,
        intro: intro(g, l.animation),
        delay_ms: f32::from(l.delay_ms),
        ..built_in
    };
    let tag = usize::try_from(l.skin)
        .ok()
        .and_then(|k| g.skins.get(k))
        .filter(|s| !s.texts.is_empty() || !s.bitmaps.is_empty());
    let skin_note = match tag {
        Some(tag) => {
            out.skin = skin(tag, g);
            let step = step(tag, g, art);
            if STEP_RANGE.contains(&step) {
                out.step = step;
            }
            let look = &out.skin.items;
            Ok(format!(
                "skin {} (number {}): {} bitmaps, {} texts, focus fades of {} and {} ms",
                tag.name,
                l.skin,
                out.skin.bitmaps.len(),
                out.skin.texts.len(),
                look.gain.period_ms,
                look.lose.period_ms
            ))
        }
        None => Err(format!("skin {} isn't in the globals", l.skin)),
    };
    (out, skin_note)
}

/// The framing behind both screens on a still background: the framing
/// picture of `game_shell_background` (or its first bitmap), else the
/// built-in place.
pub fn framing(ui: Option<&Ui>, sources: &mut Vec<String>) -> BitmapWidget {
    let built_in = layout::framing();
    let found = pane(ui, BACKGROUND);
    let read = match (&found, ui) {
        (Ok((_, p)), Some(ui)) => {
            let named = |b: &&ui::Bitmap| {
                b.bitmap.as_ref().is_some_and(|t| {
                    last_part(&t.name).eq_ignore_ascii_case(last_part(layout::FRAMING))
                })
            };
            let b = p
                .bitmaps
                .iter()
                .find(named)
                .or_else(|| p.bitmaps.iter().find(|b| b.bitmap.is_some()));
            b.and_then(|b| bitmap(b, &ui.globals, std::slice::from_ref(&built_in)))
        }
        _ => None,
    };
    match (read, found) {
        (Some(w), Ok((s, _))) => {
            note(sources, format!("framing {} from {}", w.name, s.name));
            w
        }
        (_, Err(why)) => {
            note(sources, format!("framing: built-in ({why})"));
            built_in
        }
        (None, Ok((s, _))) => {
            note(
                sources,
                format!("framing: built-in ({} names no bitmap)", s.name),
            );
            built_in
        }
    }
}

/// A box of the tags that can hold something: wider than it is nothing,
/// and taller (a globals block that didn't read is all zeros).
fn sensible([l, t, r, b]: [f32; 4]) -> bool {
    r > l && t > b
}

/// What every screen after the main menu sits in (menu.md 1.3), from the
/// tags (`ui`; none when they weren't read), noting where each piece came
/// from: `game_shell_background`'s first pane (the framing and the track
/// strips; in Vista's tags framing_center and 17 strips, 16 of them
/// scrolling), split into the pictures that scroll and those that don't;
/// the globals' header and legend boxes by size, the header fonts and
/// the text colour (the headers'; the legends keep their own,
/// `layout::colour::LEGEND`); and the overlay colour. Each piece the tags
/// lack keeps the built-in one (`ChromeLayout::default`).
///
/// Every bitmap of Vista's shell has the globals' first animation as its
/// intro (sliding in from the right): the chrome is drawn settled
/// (`chrome`), so only scrolling decides which list a picture goes in.
pub fn chrome(ui: Option<&Ui>, sources: &mut Vec<String>) -> ChromeLayout {
    let mut out = ChromeLayout::default();
    let found = pane(ui, BACKGROUND);
    match (&found, ui.map(|ui| &ui.globals)) {
        (Ok((s, p)), Some(g)) => {
            let mut built_in = layout::shell_art();
            built_in.push(layout::framing());
            let art = bitmaps(p, g, &built_in);
            if art.is_empty() {
                let why = format!("{} names no bitmaps", s.name);
                note(sources, format!("the shell's pictures: built-in ({why})"));
            } else {
                let (moving, still): (Vec<_>, Vec<_>) = art
                    .into_iter()
                    .partition(|w| w.wraps_per_second.iter().any(|&v| v != 0.0));
                note(
                    sources,
                    format!(
                        "the shell's pictures from wgit {}: {} still, {} scrolling",
                        s.name,
                        still.len(),
                        moving.len()
                    ),
                );
                out.still = still;
                out.moving = moving;
            }
        }
        (Err(why), _) => note(sources, format!("the shell's pictures: built-in ({why})")),
        // A pane is found only in tags that were read.
        (Ok(_), None) => {}
    }
    let Some(g) = ui.map(|ui| &ui.globals) else {
        note(
            sources,
            "headers, button legends and the overlay: built-in (no UI tags were read)".to_string(),
        );
        return out;
    };
    let mut boxes = 0;
    for k in 0..4 {
        let header = bounds(g.header_bounds[k]);
        if sensible(header) {
            out.headers[k].bounds = header;
            out.headers[k].font = g.header_fonts[k];
            boxes += 1;
        }
        let legend = bounds(g.button_key_bounds[k]);
        if sensible(legend) {
            out.legends[k].bounds = legend;
            boxes += 1;
        }
    }
    let colour = g.text_color.map(|c| finite(c, 0.0).clamp(0.0, 1.0));
    let coloured = colour.iter().any(|&c| c > 0.0);
    // The headers' colour; the legends keep their own.
    if coloured {
        for t in out.headers.iter_mut() {
            t.color = colour;
        }
    }
    let overlay = g.overlay_color.map(|c| finite(c, 0.0).clamp(0.0, 1.0));
    let dims = overlay[3] > 0.0;
    if dims {
        out.overlay = overlay;
    }
    let built_in = |read: bool| if read { "the globals" } else { "built-in" };
    note(
        sources,
        format!(
            "headers and button legends: {boxes} of 8 boxes from the globals, the headers' \
             colour from {}, the overlay from {}",
            built_in(coloured),
            built_in(dims)
        ),
    );
    out
}

/// A widget whose place has a number this far out is parked off the
/// screen (Halo 2 parks them at ±9999; a few at about ±9,100 to ±9,600):
/// it is left out.
const PARKED: i16 = 9000;

fn parked(v: &[i16]) -> bool {
    v.iter().any(|v| v.unsigned_abs() >= PARKED as u16)
}

/// A box of the tags as numbers (top, left, bottom, right).
fn rect_numbers(r: ui::Rect) -> [i16; 4] {
    [r.top, r.left, r.bottom, r.right]
}

/// Plain panels for the pieces of Halo 2's dialogs and lists, so the flat
/// look isn't bare text: the dialog frames' corners (ul_07, br_07) and
/// plates (dialog1, dialog2), the pregame lobby's button, and the default
/// list skin's item backing and end cap. Their sizes, in UI units at a
/// scale of 1, are the pictures' own in Vista's mainmenu.map (menu_png
/// `--frames`, menu.md 9.2), so a flat piece takes the room its picture
/// would. Most of each picture is clear: the corners' lines and the
/// plates' glow don't reach their edges, so the flat frame is only near
/// the real one. The colours are our own, near the overlay's navy.
fn flat_for(name: &str) -> Flat {
    const PANEL: [f32; 4] = [0.05, 0.13, 0.25, 0.92];
    const PLATE: [f32; 4] = [0.08, 0.18, 0.32, 0.85];
    const EDGE: [f32; 4] = [0.33, 0.47, 0.64, 0.9];
    const CLEAR: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
    let (size, color, edge) = match last_part(name).to_ascii_lowercase().as_str() {
        "ul_07" | "br_07" => ([256.0, 256.0], CLEAR, EDGE),
        "dialog1" => ([512.0, 512.0], PANEL, EDGE),
        "dialog2" => ([512.0, 512.0], PLATE, CLEAR),
        "button" => ([358.0, 64.0], PLATE, EDGE),
        "item_background" => ([400.0, 42.0], PLATE, CLEAR),
        "end_cap" => ([56.0, 68.0], EDGE, CLEAR),
        _ => return Flat::None,
    };
    Flat::Panel {
        size,
        color: colour_or(color),
        edge,
    }
}

/// A flat panel's colour (a clear one stays clear).
fn colour_or(c: [f32; 4]) -> [f32; 4] {
    if c[3] > 0.0 {
        c
    } else {
        [
            colour::OVERLAY[0],
            colour::OVERLAY[1],
            colour::OVERLAY[2],
            0.0,
        ]
    }
}

/// A bitmap of a pane at `scale`: its corner moved out, its pictures (and
/// flat shape) grown.
fn scaled_bitmap(mut w: BitmapWidget, scale: Scale) -> BitmapWidget {
    w.corner = scale.point(w.corner);
    if scale != Scale::Vista {
        let k = scale.bitmap();
        w.scale = w.scale().map(|s| s * k);
    }
    w.flat = scaled_flat(w.flat, w.scale());
    w
}

/// A flat shape's size times a bitmap's scale (`flat_for`'s sizes are at
/// scale 1; the built-in ones are already at theirs).
fn scaled_flat(flat: Flat, [sx, sy]: [f32; 2]) -> Flat {
    match flat {
        Flat::Panel { size, color, edge } => Flat::Panel {
            size: [size[0] * sx, size[1] * sy],
            color,
            edge,
        },
        other => other,
    }
}

/// A text slot of a pane at `scale`, `k`th of its kind: its box moved
/// out, its glyphs grown as the pane's pictures are.
fn slot(t: &ui::Text, k: usize, g: &Globals, scale: Scale) -> SlotText {
    let mut widget = text(t, g);
    widget.bounds = scale.rect(widget.bounds);
    SlotText {
        key: if t.string.is_empty() {
            format!("#{k}")
        } else {
            t.string.clone()
        },
        words: t.text.clone(),
        widget,
        glyphs: scale.bitmap(),
    }
}

/// The list a screen's list falls back on when its skin isn't in the
/// globals: our own plain skin, one left-justified text in the default
/// skin's colour (menu-preview's dialog place).
fn plain_list() -> ListLayout {
    let place = layout::DIALOG_PLACE;
    ListLayout {
        skin: SkinLayout {
            texts: vec![TextWidget::new(
                crate::Font::Body,
                colour::LIST_TEXT,
                place.item,
                Justify::Left,
            )],
            ..SkinLayout::default()
        },
        corner: place.corner,
        step: place.step,
        visible: 6,
        wraps: false,
        intro: None,
        delay_ms: 0.0,
    }
}

/// A pane of a screen at `scale`: every bitmap, text, list, button and
/// players block, the parked ones left out. Returns the pane and how many
/// widgets were parked.
fn pane_layout(
    p: &Pane,
    g: &Globals,
    art: &dyn ArtSource,
    scale: Scale,
    skins: &mut Vec<String>,
) -> (PaneLayout, usize) {
    let mut dropped = 0;
    let mut keep = |out: bool| {
        dropped += usize::from(out);
        !out
    };
    let bitmaps = p
        .bitmaps
        .iter()
        .filter(|b| keep(parked(&b.corner)))
        .filter_map(|b| bitmap(b, g, &[]))
        .map(|w| scaled_bitmap(w, scale))
        .collect();
    let texts = p
        .texts
        .iter()
        .enumerate()
        .filter(|(_, t)| keep(parked(&rect_numbers(t.bounds))))
        .map(|(k, t)| slot(t, k, g, scale))
        .collect();
    let lists = p
        .lists
        .iter()
        .filter(|l| keep(parked(&l.corner)))
        .map(|l| {
            let (list, found) = list_at(l, g, art, plain_list(), scale);
            let name = match found {
                Ok(_) => usize::try_from(l.skin)
                    .ok()
                    .and_then(|k| g.skins.get(k))
                    .map_or_else(String::new, |t| last_part(&t.name).to_string()),
                Err(why) => format!("our plain one ({why})"),
            };
            if !skins.contains(&name) {
                skins.push(name);
            }
            list
        })
        .collect();
    let buttons = p
        .buttons
        .iter()
        .enumerate()
        .filter(|(_, b)| keep(parked(&rect_numbers(b.text.bounds))))
        .map(|(k, b)| button(b, k, g, scale))
        .collect();
    let players = p
        .players
        .iter()
        .filter(|e| keep(parked(&e.corner)))
        .map(|e| PlayersLayout {
            corner: scale.point(e.corner.map(f32::from)),
            max: usize::from(e.max),
            rows: usize::from(e.rows),
            columns: usize::from(e.columns),
            row_height: f32::from(e.row_height).abs() * scale.position(),
            column_width: f32::from(e.column_width) * scale.position(),
            skin: e.skin.as_ref().map(|t| t.name.clone()),
        })
        .collect();
    let pane = PaneLayout {
        scale,
        bitmaps,
        texts,
        lists,
        buttons,
        players,
    };
    (pane, dropped)
}

/// A button of a pane at `scale`: its text slot, and its picture with its
/// corner at the text box's top left moved by the tag's offset, whose y
/// counts downwards (as menu-preview placed the pregame lobby's, which
/// matches Cartographer's lobby), drawn under the text.
fn button(b: &ui::Button, k: usize, g: &Globals, scale: Scale) -> ButtonLayout {
    let text = slot(&b.text, k, g, scale);
    let bitmap = b.bitmap.as_ref().map(|tag| {
        let [ox, oy] = b.bitmap_offset.map(f32::from);
        let corner = [
            f32::from(b.text.bounds.left) + ox,
            f32::from(b.text.bounds.top) - oy,
        ];
        let w = BitmapWidget {
            depth: b.text.depth.saturating_sub(1),
            intro: text.widget.intro.clone(),
            delay_ms: text.widget.delay_ms,
            flat: flat_for(&tag.name),
            ..BitmapWidget::new(&tag.name, corner)
        };
        scaled_bitmap(w, scale)
    });
    ButtonLayout { text, bitmap }
}

/// A screen of the catalog laid out from the tags (`ui`), each pane at
/// the catalog's scale with `overrides` over it (`H2LOBBY_SCALE`), noting
/// what was found.
pub fn screen_layout(
    ui: &Ui,
    id: ScreenId,
    overrides: &[crate::catalog::Override],
    art: &dyn ArtSource,
    sources: &mut Vec<String>,
) -> Result<ScreenLayout, String> {
    let path = id.path();
    let panes = ui.screen(&path).map_or(0, |s| s.panes.len());
    let scales = id.scales(panes, overrides);
    let mut out = screen_layout_at(ui, &path, &scales, art, sources)?;
    out.id = Some(id);
    Ok(out)
}

/// Any screen laid out from the tags by its whole tag name (only: two
/// tags end in player_selected_dialog), pane `k` at `scales[k]` (the last
/// one after them; Vista's without any), noting what was found.
pub fn screen_layout_at(
    ui: &Ui,
    path: &str,
    scales: &[Scale],
    art: &dyn ArtSource,
    sources: &mut Vec<String>,
) -> Result<ScreenLayout, String> {
    let s = ui
        .screens
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(path))
        .ok_or_else(|| format!("no {path} screen in the tags"))?;
    let g = &ui.globals;
    let mut dropped = 0;
    let mut skins = Vec::new();
    let panes: Vec<PaneLayout> = s
        .panes
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let scale = scales.get(k).or(scales.last()).copied().unwrap_or_default();
            let (pane, parked) = pane_layout(p, g, art, scale, &mut skins);
            dropped += parked;
            pane
        })
        .collect();
    let scale_names: Vec<&str> = panes.iter().map(|p| p.scale.name()).collect();
    note(
        sources,
        format!(
            "screen {}: {} panes ({}), {} parked widgets left out{}",
            s.name,
            panes.len(),
            scale_names.join(", "),
            dropped,
            if skins.is_empty() {
                String::new()
            } else {
                format!(", list skins {}", skins.join(", "))
            }
        ),
    );
    Ok(ScreenLayout {
        id: None,
        path: s.name.clone(),
        size: s.dialog_size(),
        has_header: s.flags & ui::NO_HEADER == 0,
        header_id: s.header.clone(),
        header_words: s.header_text.clone(),
        button_key: s.button_key,
        overlay: s.flags & ui::NO_OVERLAY == 0,
        pane_per_item: s.flags & ui::PANE_PER_ITEM != 0,
        panes,
        strings: s.strings.clone(),
    })
}

/// Bitmaps by their whole names, each with a frame drawn.
pub type Wanted = Vec<(String, usize)>;

/// What screens draw, for `art::read_some`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wants {
    /// Each bitmap and frame drawn: one missing is logged (drawn flat).
    pub needed: Wanted,
    /// Frames read only where their bitmaps have them (a button's focused
    /// look).
    pub extra: Wanted,
    /// Bitmaps read whole, every image: list skins' pictures, which a row
    /// may show at any frame (`screens::Row::frames`, the Y menu's status
    /// icons).
    pub every: Vec<String>,
}

impl Wants {
    /// These and `more`'s (a bitmap read whole named once).
    pub fn add(&mut self, more: Wants) {
        self.needed.extend(more.needed);
        self.extra.extend(more.extra);
        for name in more.every {
            if !self.every.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                self.every.push(name);
            }
        }
    }
}

/// Every bitmap (name and frame) a screen draws: its panes' pictures, its
/// buttons' resting look, its lists' skins' and their arrows (both
/// frames); then the frames drawn only where a bitmap has them (a
/// button's focused look, `art::read_some`); then its skins' pictures
/// again, to be read whole.
pub fn screen_wanted(screen: &ScreenLayout) -> Wants {
    let mut out = Wants::default();
    for p in &screen.panes {
        out.needed
            .extend(p.bitmaps.iter().map(|w| (w.name.clone(), w.frame)));
        for b in p.buttons.iter().filter_map(|b| b.bitmap.as_ref()) {
            out.needed.push((b.name.clone(), BUTTON_FRAMES[0]));
            out.extra.push((b.name.clone(), BUTTON_FRAMES[1]));
        }
        for l in &p.lists {
            let skin = &l.skin.bitmaps;
            out.needed
                .extend(skin.iter().map(|w| (w.name.clone(), w.frame)));
            if let Some(a) = &l.skin.arrows {
                out.needed.extend([0, 1].map(|f| (a.bitmap.clone(), f)));
            }
            out.add(Wants {
                every: skin.iter().map(|w| w.name.clone()).collect(),
                ..Wants::default()
            });
        }
    }
    out
}

/// The frames of a button's picture drawn: when it hasn't focus, and when
/// it has (`screens::screen`; one with a single picture shows it for both).
/// The pregame lobby's button has three (menu.md 9.2): resting, focused
/// (lit, with arrow tips), and a softer third one not used.
pub const BUTTON_FRAMES: [usize; 2] = [0, 1];

/// Every bitmap (name and frame) the layouts draw.
pub fn wanted(
    start: &StartLayout,
    main: &MainMenuLayout,
    chrome: &ChromeLayout,
) -> Vec<(String, usize)> {
    start
        .art
        .iter()
        .chain(&main.art)
        .chain(&main.list.skin.bitmaps)
        .chain([&start.framing, &main.framing])
        .chain(&chrome.still)
        .chain(&chrome.moving)
        .map(|w| (w.name.clone(), w.frame))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::{ArtImage, FlatArt, MemoryArt};
    use crate::layout::DialogSize;
    use crate::Font;
    use blam_cache::ui::{Keyframe as TagKeyframe, ListSkin, Rect, ScreenAnimation, TagRef};
    use blam_cache::DatumIndex;

    fn tag(name: &str) -> Option<TagRef> {
        Some(TagRef {
            datum: DatumIndex(0),
            name: name.to_string(),
        })
    }

    fn tag_bitmap(name: &str, corner: [i16; 2]) -> ui::Bitmap {
        ui::Bitmap {
            flags: 0,
            animation: None,
            delay_ms: 0,
            multiply: false,
            frame: 0,
            corner,
            wraps_per_second: [0.0, 0.0],
            bitmap: tag(name),
            depth: 0,
            scale: [0.0, 0.0],
        }
    }

    fn tag_text(flags: u32, string: &str, bounds: [i16; 4]) -> ui::Text {
        let [top, left, bottom, right] = bounds;
        ui::Text {
            flags,
            animation: None,
            delay_ms: 0,
            font: Font::Title,
            color: [0.5, 0.6, 0.7],
            bounds: Rect {
                top,
                left,
                bottom,
                right,
            },
            string: string.to_string(),
            text: None,
            depth: 3,
        }
    }

    fn fade(period_ms: i32, alphas: [f32; 2]) -> ui::Animation {
        ui::Animation {
            period_ms,
            keyframes: alphas
                .iter()
                .map(|&alpha| TagKeyframe {
                    alpha,
                    position: [0.0; 3],
                })
                .collect(),
        }
    }

    fn screen_of(name: &str, pane: Pane) -> Screen {
        Screen {
            name: name.to_string(),
            flags: ui::NO_HEADER,
            screen_id: 6,
            button_key: 0,
            header: String::new(),
            header_text: None,
            panes: vec![pane],
            strings: Vec::new(),
        }
    }

    /// Menus whose start screen has a logo, a track and texts, whose main
    /// menu has a list in skin 1, and a framing.
    fn menus() -> Ui {
        let mut globals = Globals {
            animations: vec![
                ScreenAnimation::default(),
                ScreenAnimation {
                    intro: fade(400, [0.0, 1.0]),
                    ..ScreenAnimation::default()
                },
            ],
            ..Globals::default()
        };
        let mut track = tag_bitmap("ui\\shell\\track5", [-1200, 77]);
        track.wraps_per_second = [0.25, f32::NAN];
        track.scale = [1.2, 1.2];
        track.multiply = true;
        let mut logo = tag_bitmap(layout::LOGO, [-400, 60]);
        logo.animation = Some(1);
        logo.delay_ms = 50;
        logo.depth = 4;
        let start = Pane {
            bitmaps: vec![logo, track, tag_bitmap("", [0, 0])],
            texts: vec![
                tag_text(0, "", [-170, -200, -190, 200]),
                tag_text(ui::PULSATING, PRESS_START_ID, [-70, -300, -110, 300]),
                tag_text(ui::LEFT_JUSTIFY, "", [-560, 370, -600, 600]),
            ],
            ..Pane::default()
        };
        // The empty tag ref makes no bitmap.
        let mut start_screen = screen_of(START_SCREEN, start);
        start_screen.panes[0].bitmaps[2].bitmap = None;
        let main = Pane {
            bitmaps: vec![tag_bitmap(layout::LOGO, [-511, 90])],
            lists: vec![ui::List {
                flags: ui::LIST_WRAPS,
                skin: 1,
                visible: 0,
                corner: [-150, -60],
                animation: Some(1),
                delay_ms: 20,
            }],
            ..Pane::default()
        };
        let mut bar = tag_bitmap("ui\\list_skins\\list_bkd", [-60, -40]);
        bar.flags = ui::IGNORE_FOR_LIST_SIZE;
        let mut sheen = tag_bitmap("list_bkd_multiply", [150, -40]);
        sheen.multiply = true;
        sheen.flags = ui::IGNORE_FOR_LIST_SIZE;
        globals.skins = vec![
            ListSkin {
                name: "ui\\list_skins\\default".into(),
                arrows: None,
                arrow_up: [0; 2],
                arrow_down: [0; 2],
                item_animations: Vec::new(),
                texts: Vec::new(),
                bitmaps: Vec::new(),
            },
            ListSkin {
                name: "ui\\list_skins\\main_menu\\main_menu".into(),
                arrows: None,
                arrow_up: [0; 2],
                arrow_down: [0; 2],
                item_animations: vec![fade(300, [0.25, 1.0]), fade(150, [1.0, 0.25])],
                texts: vec![tag_text(0, "", [12, -62, -48, 418])],
                bitmaps: vec![bar, sheen],
            },
        ];
        let mut framing = tag_bitmap("ui\\global_bitmaps\\framing_center", [-1000, 600]);
        framing.scale = [1.1, 1.1];
        let background = Pane {
            bitmaps: vec![tag_bitmap("ui\\global_bitmaps\\corner", [0, 0]), framing],
            ..Pane::default()
        };
        Ui {
            globals,
            screens: vec![
                start_screen,
                // Found by the last part of its name.
                screen_of("ui\\elsewhere\\main_menu", main),
                screen_of(BACKGROUND, background),
            ],
        }
    }

    #[test]
    fn the_start_screen_reads_its_pieces_from_its_tags() {
        let ui = menus();
        let mut sources = Vec::new();
        let s = start_layout(Some(&ui), &mut sources);
        assert_eq!(s.art.len(), 2, "the bitmap with no tag is left out");
        let logo = &s.art[0];
        assert_eq!(logo.corner, [-400.0, 60.0]);
        assert_eq!(logo.intro.as_ref().unwrap().period_ms, 400.0);
        assert_eq!((logo.delay_ms, logo.depth), (50.0, 4));
        // The built-in logo's lettering, for when its picture is missing.
        assert!(matches!(logo.flat, Flat::Lettering { .. }));
        let track = &s.art[1];
        assert_eq!(track.blend, Blend::Multiply);
        assert_eq!(track.wraps_per_second, [0.25, 0.0]);
        assert_eq!(track.scale(), [1.2, 1.2]);
        assert!(matches!(track.flat, Flat::Track { .. }));
        assert_eq!(track.intro, None);
        let press = &s.press_start;
        assert!(press.pulsating);
        assert_eq!(press.bounds, [-300.0, -70.0, 300.0, -110.0]);
        assert_eq!((press.font, press.justify), (Font::Title, Justify::Center));
        assert_eq!(s.small_print.justify, Justify::Left);
        assert_eq!(s.small_print.bounds, [370.0, -560.0, 600.0, -600.0]);
        assert_eq!(s.small_print.color, [0.5, 0.6, 0.7]);
        assert!(sources.iter().all(|l| l.starts_with("menus: ")));
        assert!(sources[0].contains("from wgit") && sources[0].contains("(2 bitmaps)"));
        assert!(sources[1].contains(PRESS_START_ID));
    }

    #[test]
    fn the_main_menu_reads_its_list_and_skin() {
        let ui = menus();
        let mut sources = Vec::new();
        let m = main_menu_layout(Some(&ui), &FlatArt, &mut sources);
        assert_eq!(m.art.len(), 1);
        let list = &m.list;
        assert_eq!(list.corner, [-150.0, -60.0]);
        assert_eq!(list.visible, 6, "none shown is taken as the built-in 6");
        assert!(list.wraps);
        assert_eq!(list.intro.as_ref().unwrap().period_ms, 400.0);
        assert_eq!(list.delay_ms, 20.0);
        // The text's 60 units: the bitmaps don't count.
        assert_eq!(list.step, 60.0);
        assert_eq!(list.skin.texts[0].bounds, [-62.0, 12.0, 418.0, -48.0]);
        assert_eq!(list.skin.bitmaps.len(), 2);
        assert!(list.skin.bitmaps.iter().all(|b| b.ignore_for_list_size));
        assert!(matches!(list.skin.bitmaps[0].flat, Flat::Glow { .. }));
        assert!(matches!(list.skin.bitmaps[1].flat, Flat::Sheen { .. }));
        assert_eq!(list.skin.items.gain.period_ms, 300.0);
        assert_eq!(list.skin.items.resting(), 0.25);
        // No small print in the tags: the built-in one stays.
        assert_eq!(m.small_print, MainMenuLayout::default().small_print);
        assert!(sources.iter().any(|l| l.contains("a row every 60 units")));
        assert!(sources
            .iter()
            .any(|l| l.contains("focus fades of 300 and 150 ms")));
        // A bitmap that counts, 62 tall, at -40: the step grows to cover it.
        let mut ui = ui;
        ui.globals.skins[1].bitmaps[0].flags = 0;
        let mut art = MemoryArt::new();
        art.add_image(
            "ui\\list_skins\\list_bkd",
            ArtImage {
                width: 480,
                height: 62,
                rgba: vec![255; 480 * 62 * 4],
            },
        );
        let m = main_menu_layout(Some(&ui), &art, &mut Vec::new());
        assert_eq!(m.list.step, 22.0 + 48.0);
    }

    #[test]
    fn missing_tags_keep_the_built_in_pieces() {
        let mut sources = Vec::new();
        assert_eq!(start_layout(None, &mut sources), StartLayout::default());
        let m = main_menu_layout(None, &FlatArt, &mut sources);
        assert_eq!(m, MainMenuLayout::default());
        assert_eq!(framing(None, &mut sources), layout::framing());
        assert!(
            sources.iter().all(|l| l.contains("built-in")),
            "{sources:?}"
        );
        // A list in a skin the globals haven't keeps the built-in skin.
        let mut ui = menus();
        ui.screens[1].panes[0].lists[0].skin = 9;
        let mut sources = Vec::new();
        let m = main_menu_layout(Some(&ui), &FlatArt, &mut sources);
        assert_eq!(m.list.corner, [-150.0, -60.0]);
        assert_eq!(m.list.skin, layout::main_menu_skin());
        assert_eq!(m.list.step, layout::MAIN_PLACE.step);
        assert!(sources.iter().any(|l| l.contains("skin 9 isn't")));
        // An empty skin (one that didn't read) is the same.
        ui.screens[1].panes[0].lists[0].skin = 0;
        let m = main_menu_layout(Some(&ui), &FlatArt, &mut Vec::new());
        assert_eq!(m.list.skin, layout::main_menu_skin());
    }

    #[test]
    fn the_framing_comes_from_the_shell_background() {
        let ui = menus();
        let mut sources = Vec::new();
        let f = framing(Some(&ui), &mut sources);
        assert_eq!(f.name, "ui\\global_bitmaps\\framing_center");
        assert_eq!((f.corner, f.scale()), ([-1000.0, 600.0], [1.1, 1.1]));
        assert!(sources[0].contains(BACKGROUND));
        let start = start_layout(Some(&ui), &mut Vec::new());
        let main = main_menu_layout(Some(&ui), &FlatArt, &mut Vec::new());
        let shell = chrome(Some(&ui), &mut Vec::new());
        let names = wanted(&start, &main, &shell);
        // The start screen's, the main menu's, its skin's, the two
        // framings, and the chrome's corner and framing.
        assert_eq!(names.len(), 2 + 1 + 2 + 2 + 2);
        assert!(names.contains(&("ui\\global_bitmaps\\corner".to_string(), 0)));
        assert!(names.contains(&("list_bkd_multiply".to_string(), 0)));
        assert_eq!(
            screen(&ui, MAIN_MENU).unwrap().name,
            "ui\\elsewhere\\main_menu"
        );
    }

    /// A game_shell_background of a framing, one strip that stays and two
    /// that scroll (each with the globals' first animation as its intro,
    /// as Vista's have), and globals with boxes, fonts and colours.
    fn shell_menus() -> Ui {
        let mut ui = menus();
        let g = &mut ui.globals;
        let rect = |[top, left, bottom, right]: [i16; 4]| Rect {
            top,
            left,
            bottom,
            right,
        };
        g.header_bounds = [
            rect([560, -720, 520, 40]),
            rect([390, -500, 310, 55]),
            rect([158, -500, 73, 255]),
            rect([182, -252, 142, 350]),
        ];
        g.button_key_bounds = [
            rect([-500, 100, -540, 730]),
            rect([-354, 0, -394, 535]),
            rect([-225, -400, -270, 480]),
            // One that didn't read: the built-in box stays.
            rect([0, 0, 0, 0]),
        ];
        g.header_fonts = [Font::Title, Font::LargeBody, Font::Body, Font::Body];
        g.text_color = [0.5, 0.6, 0.7];
        g.overlay_color = [0.0, 0.1, 0.2, 0.8];
        let strip = |name: &str, y: i16, wraps: f32| {
            let mut b = tag_bitmap(name, [-1100, y]);
            b.scale = [1.1, 1.1];
            b.wraps_per_second = [wraps, 0.0];
            b.depth = 11;
            b.animation = Some(1);
            b
        };
        let mut framing = tag_bitmap("ui\\global_bitmaps\\framing_center", [-1070, 654]);
        framing.scale = [1.078, 1.078];
        framing.depth = 10;
        framing.animation = Some(1);
        let k = ui
            .screens
            .iter()
            .position(|s| s.name == BACKGROUND)
            .unwrap();
        ui.screens[k].panes[0] = Pane {
            bitmaps: vec![
                framing,
                strip("ui\\global_bitmaps\\track5", 136, 0.08),
                strip("ui\\global_bitmaps\\track7a", -284, 0.0),
                strip("ui\\global_bitmaps\\track7c", 475, 0.15),
            ],
            ..Pane::default()
        };
        ui
    }

    #[test]
    fn the_chrome_splits_the_shell_into_still_and_moving_pictures() {
        let ui = shell_menus();
        let mut sources = Vec::new();
        let c = chrome(Some(&ui), &mut sources);
        let names = |ws: &[BitmapWidget]| {
            ws.iter()
                .map(|w| last_part(&w.name).to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&c.still), ["framing_center", "track7a"]);
        assert_eq!(names(&c.moving), ["track5", "track7c"]);
        assert_eq!(c.moving[1].wraps_per_second, [0.15, 0.0]);
        assert_eq!(
            (c.still[0].corner, c.still[0].depth),
            ([-1070.0, 654.0], 10)
        );
        assert!(c.still[0].intro.is_some(), "read, though drawn settled");
        // The track keeps its flat shape for when its picture is missing.
        assert!(matches!(c.moving[0].flat, Flat::Track { .. }));
        // The globals' boxes, fonts and colours.
        assert_eq!(c.headers[0].bounds, [-720.0, 560.0, 40.0, 520.0]);
        assert_eq!(c.headers[1].font, Font::LargeBody);
        assert_eq!(c.legends[2].bounds, [-400.0, -225.0, 480.0, -270.0]);
        assert_eq!(c.legends[3].bounds, layout::LEGEND_BOUNDS[3]);
        assert!(c.headers.iter().all(|h| h.color == [0.5, 0.6, 0.7]));
        assert!(c.legends.iter().all(|l| l.color == layout::colour::LEGEND));
        assert_eq!(c.overlay, [0.0, 0.1, 0.2, 0.8]);
        assert!(sources.iter().all(|l| l.starts_with("menus: ")));
        assert!(sources[0].contains("2 still, 2 scrolling"), "{sources:?}");
        assert!(sources[1].contains("7 of 8 boxes"), "{sources:?}");
        assert!(!sources.join("\n").contains("built-in"), "{sources:?}");
    }

    #[test]
    fn a_chrome_without_tags_is_the_built_in_one() {
        let mut sources = Vec::new();
        assert_eq!(chrome(None, &mut sources), ChromeLayout::default());
        assert!(
            sources.iter().all(|l| l.contains("built-in")),
            "{sources:?}"
        );
        // Tags without a shell background, and globals that didn't read:
        // the built-in pictures and boxes.
        let mut ui = menus();
        ui.screens.retain(|s| s.name != BACKGROUND);
        ui.globals = Globals::default();
        let mut sources = Vec::new();
        assert_eq!(chrome(Some(&ui), &mut sources), ChromeLayout::default());
        assert!(
            sources[0].contains("no game_shell_background screen"),
            "{sources:?}"
        );
        assert!(sources[1].contains("0 of 8 boxes"), "{sources:?}");
        assert!(sources[1].contains("colour from built-in"), "{sources:?}");
    }

    /// The shell's menus with the default skin given a text, a backing
    /// and its arrows, and a quarter dialog at `path` of two panes in the
    /// Xbox's numbers: a picture, a text, a list, a button with its
    /// picture, a players block, and two widgets parked off the screen.
    fn dialog_menus(path: &str) -> Ui {
        let mut ui = shell_menus();
        let skin = &mut ui.globals.skins[0];
        skin.texts = vec![tag_text(ui::LEFT_JUSTIFY, "", [20, 10, -10, 300])];
        skin.bitmaps = vec![tag_bitmap(
            "ui\\list_skins\\default\\item_background",
            [0, 0],
        )];
        skin.arrows = tag("ui\\list_skins\\default\\arrows");
        skin.arrow_up = [-30, 40];
        skin.arrow_down = [-30, -120];
        let mut picture = tag_bitmap("ui\\global_bitmaps\\dialog1", [-100, 80]);
        picture.depth = 2;
        let mut body = tag_text(0, "quarter_body", [40, -80, 0, 80]);
        body.text = Some("SOME WORDS".into());
        let pane = Pane {
            bitmaps: vec![picture, tag_bitmap("ui\\global_bitmaps\\ul_07", [-9999, 0])],
            texts: vec![body, tag_text(0, "", [9999, -9999, 9999, -9999])],
            lists: vec![ui::List {
                flags: 0,
                skin: 0,
                visible: 3,
                corner: [-60, -10],
                animation: None,
                delay_ms: 0,
            }],
            buttons: vec![ui::Button {
                text: tag_text(0, "quarter_button", [-40, -70, -56, 10]),
                bitmap: tag("ui\\screens\\game_shell\\pregame_lobby\\button"),
                bitmap_offset: [-8, -4],
            }],
            players: vec![ui::Players {
                skin: tag("ui\\player_skins\\made_up"),
                corner: [20, 30],
                max: 8,
                rows: 4,
                columns: 2,
                row_height: -22,
                column_width: 90,
            }],
            ..Pane::default()
        };
        let second = Pane {
            texts: vec![tag_text(0, "quarter_second", [10, -20, -10, 20])],
            ..Pane::default()
        };
        ui.screens.push(Screen {
            name: path.to_string(),
            flags: ui::QUARTER_DIALOG,
            screen_id: 9,
            button_key: 2,
            header: "quarter_header".into(),
            header_text: Some("MADE UP".into()),
            panes: vec![pane, second],
            strings: vec![
                ("quarter_header".into(), "MADE UP".into()),
                ("quarter_body".into(), "SOME WORDS".into()),
            ],
        });
        ui
    }

    const DIALOG: &str = "ui\\screens\\made_up\\quarter_dialog";

    #[test]
    fn an_xbox_dialog_grows_its_places_and_its_pictures() {
        let ui = dialog_menus(DIALOG);
        let mut sources = Vec::new();
        let s = screen_layout_at(&ui, DIALOG, &[Scale::Xbox], &FlatArt, &mut sources).unwrap();
        assert_eq!(s.id, None);
        assert_eq!(s.path, DIALOG);
        assert_eq!(s.size, DialogSize::Quarter);
        assert!(s.has_header && s.overlay && !s.pane_per_item);
        assert_eq!(s.button_key, 2);
        assert_eq!(s.header_id, "quarter_header");
        assert_eq!(s.header_words.as_deref(), Some("MADE UP"));
        assert_eq!(s.string("quarter_body"), Some("SOME WORDS"));
        assert_eq!(s.string("nothing"), None);
        // The last scale goes for every pane after the ones given.
        assert_eq!(s.panes.len(), 2);
        assert!(s.panes.iter().all(|p| p.scale == Scale::Xbox));
        let p = &s.panes[0];
        // Places 2.5 times out, pictures 1.25 times their scale (a flat
        // piece too); the parked picture and text are left out.
        assert_eq!(p.bitmaps.len(), 1);
        assert_eq!(p.bitmaps[0].corner, [-250.0, 200.0]);
        assert_eq!(p.bitmaps[0].scale, [1.25, 1.25]);
        assert_eq!(p.bitmaps[0].depth, 2);
        assert!(matches!(
            p.bitmaps[0].flat,
            Flat::Panel {
                size: [640.0, 640.0],
                ..
            }
        ));
        assert_eq!(p.texts.len(), 1);
        assert_eq!(p.texts[0].key, "quarter_body");
        assert_eq!(p.texts[0].words.as_deref(), Some("SOME WORDS"));
        assert_eq!(p.texts[0].widget.bounds, [-200.0, 100.0, 200.0, 0.0]);
        assert_eq!(p.texts[0].glyphs, 1.25);
        // A list's corner moves out; its skin, shared, stays in Vista's
        // units, with its arrows.
        let l = &p.lists[0];
        assert_eq!(l.corner, [-150.0, -25.0]);
        assert_eq!(l.visible, 3);
        assert_eq!(l.skin.texts[0].bounds, [10.0, 20.0, 300.0, -10.0]);
        assert_eq!(l.skin.bitmaps[0].corner, [0.0, 0.0]);
        let arrows = l.skin.arrows.as_ref().unwrap();
        assert_eq!(arrows.bitmap, "ui\\list_skins\\default\\arrows");
        assert_eq!((arrows.up, arrows.down), ([-30.0, 40.0], [-30.0, -120.0]));
        // A button's picture sits at its text box's top left moved by the
        // offset (y counted down), under the text, grown like the rest.
        let b = &p.buttons[0];
        assert_eq!(b.key(), "quarter_button");
        assert_eq!(b.text.widget.bounds, [-175.0, -100.0, 25.0, -140.0]);
        assert_eq!(b.text.glyphs, 1.25);
        let picture = b.bitmap.as_ref().unwrap();
        assert_eq!(picture.corner, [-195.0, -90.0]);
        assert_eq!(picture.depth, b.text.widget.depth - 1);
        assert_eq!(picture.scale, [1.25, 1.25]);
        assert!(matches!(
            picture.flat,
            Flat::Panel {
                size: [447.5, 80.0],
                ..
            }
        ));
        // A players block: rows 22 Xbox pixels apart are 55 units.
        let e = &p.players[0];
        assert_eq!(
            (e.corner, e.row_height, e.column_width),
            ([50.0, 75.0], 55.0, 225.0)
        );
        assert_eq!((e.max, e.rows, e.columns), (8, 4, 2));
        assert_eq!(e.skin.as_deref(), Some("ui\\player_skins\\made_up"));
        assert_eq!(e.corner(0), [50.0, 75.0]);
        assert_eq!(e.corner(5), [275.0, 20.0]);
        let line = sources.last().unwrap();
        assert_eq!(
            line,
            &format!(
                "menus: screen {DIALOG}: 2 panes (xbox, xbox), 2 parked widgets left out, \
                 list skins default"
            )
        );
        // What it draws: the button's focused look only where its bitmap
        // has it.
        let wants = screen_wanted(&s);
        let has = |name: &str, frame: usize| {
            wants
                .needed
                .iter()
                .any(|(n, f)| last_part(n) == name && *f == frame)
        };
        assert!(has("dialog1", 0) && has("button", 0) && has("item_background", 0));
        assert!(has("arrows", 0) && has("arrows", 1));
        assert!(!has("button", 1) && !has("ul_07", 0));
        assert_eq!(
            wants.extra,
            [(
                "ui\\screens\\game_shell\\pregame_lobby\\button".to_string(),
                1
            )]
        );
        // The skin's pictures read whole (a row may show any frame).
        assert_eq!(wants.every, ["ui\\list_skins\\default\\item_background"]);
    }

    /// A map's pictures where every bitmap has three separate 4 by 4
    /// images (no sequences: frame k is image k), noting each image read.
    struct ThreeEach {
        read: Vec<(String, usize)>,
    }

    impl blam_cache::ui::Pictures for ThreeEach {
        fn bitmap(&mut self, name: &str) -> blam_cache::Result<ui::BitmapInfo> {
            let image = blam_cache::bitmap::ImageInfo {
                width: 4,
                height: 4,
                format: blam_cache::bitmap::Format::A8R8G8B8,
            };
            Ok(ui::BitmapInfo {
                name: name.to_string(),
                images: vec![image; 3],
                sequences: Vec::new(),
            })
        }

        fn image(
            &mut self,
            name: &str,
            index: usize,
        ) -> blam_cache::Result<blam_cache::bitmap::Image> {
            self.read.push((last_part(name).to_string(), index));
            Ok(blam_cache::bitmap::Image {
                width: 4,
                height: 4,
                rgba: vec![60 * index as u8 + 30; 64],
            })
        }
    }

    #[test]
    fn a_shell_reads_its_list_skins_pictures_whole_for_their_rows() {
        let ui = dialog_menus(&crate::catalog::ScreenId::Lobby.path());
        let mut pictures = ThreeEach { read: Vec::new() };
        let shell = crate::shell::Shell::with_scales(Some(&ui), Some(&mut pictures), &[]);
        // The skin's picture has every frame; a pane's picture only the
        // one its tag names.
        let icon = "ui\\list_skins\\default\\item_background";
        for frame in 0..3 {
            assert!(shell.art.picture(icon, frame).is_some(), "{frame}");
        }
        let plate = "ui\\global_bitmaps\\dialog1";
        assert!(shell.art.picture(plate, 0).is_some());
        assert!(shell.art.picture(plate, 2).is_none());
        let reads = |name: &str| pictures.read.iter().filter(|r| r.0 == name).count();
        assert_eq!((reads("item_background"), reads("dialog1")), (3, 1));
        assert!(
            shell.sources.iter().all(|l| !l.contains("drawn flat")),
            "{:?}",
            shell.sources
        );
        // Drawn, a row asking for frame 2 shows it; the others their own.
        let layout = shell.screen(crate::catalog::ScreenId::Lobby).unwrap();
        let rows: Vec<crate::screens::Row> = (0..3)
            .map(|k| crate::screens::Row {
                frames: if k == 1 {
                    vec![("item_background", 2)]
                } else {
                    Vec::new()
                },
                ..crate::screens::Row::new("ROW")
            })
            .collect();
        let lists = [crate::screens::ListState {
            rows: &rows,
            focus: None,
            first: 0,
        }];
        let state = crate::screens::ScreenState {
            pane: 0,
            opened: 0.0,
            chrome: &shell.chrome,
            words: &|_| crate::screens::Words::Hide,
            tokens: &|_| None,
            lists: &lists,
            button_focus: None,
            disabled_buttons: &[],
            hidden_buttons: &[],
            header: crate::screens::Words::Hide,
            legend: &[],
            keyboard: false,
            pictures: &|_| crate::screens::Show::Keep,
        };
        let fonts = crate::text::Fonts::fallback();
        let space = layout::Space::new(640, 360);
        let (list, _) = crate::screens::screen(5.0, layout, &state, shell.art(), &fonts, space);
        let texture = |frame| shell.art.picture(icon, frame).unwrap().texture;
        let count = |frame| {
            list.quads
                .iter()
                .filter(|q| q.texture == texture(frame))
                .count()
        };
        assert_eq!((count(0), count(1), count(2)), (2, 0, 1));
    }

    #[test]
    fn a_vista_screen_is_one_to_one_and_scales_either_way() {
        let mut ui = dialog_menus(DIALOG);
        ui.screens.last_mut().unwrap().flags = 0;
        let mut sources = Vec::new();
        let vista = screen_layout_at(&ui, DIALOG, &[], &FlatArt, &mut sources).unwrap();
        assert_eq!(vista.size, DialogSize::Full);
        let p = &vista.panes[0];
        assert_eq!(p.scale, Scale::Vista);
        assert_eq!(p.bitmaps[0].corner, [-100.0, 80.0]);
        assert_eq!(p.bitmaps[0].scale(), [1.0, 1.0]);
        assert!(matches!(
            p.bitmaps[0].flat,
            Flat::Panel {
                size: [512.0, 512.0],
                ..
            }
        ));
        assert_eq!(p.texts[0].widget.bounds, [-80.0, 40.0, 80.0, 0.0]);
        assert_eq!((p.texts[0].glyphs, p.buttons[0].text.glyphs), (1.0, 1.0));
        assert_eq!(p.lists[0].corner, [-60.0, -10.0]);
        assert_eq!(p.buttons[0].bitmap.as_ref().unwrap().corner, [-78.0, -36.0]);
        assert_eq!(p.players[0].row_height, 22.0);
        // The same pane taken to the Xbox's scale is the one laid out
        // there, and back again it is the Vista one.
        let xbox = screen_layout_at(&ui, DIALOG, &[Scale::Xbox], &FlatArt, &mut sources).unwrap();
        assert_eq!(p.at_scale(Scale::Xbox), xbox.panes[0]);
        assert_eq!(p.at_scale(Scale::Vista), *p);
        let back = xbox.panes[0].at_scale(Scale::Vista);
        let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() + (a[1] - b[1]).abs() < 1e-3;
        assert_eq!(back.scale, Scale::Vista);
        assert!(near(back.bitmaps[0].corner, p.bitmaps[0].corner));
        assert!(near(back.bitmaps[0].scale(), [1.0, 1.0]));
        assert!(near(back.lists[0].corner, p.lists[0].corner));
        assert!((back.players[0].row_height - 22.0).abs() < 1e-3);
        assert!((back.texts[0].glyphs - 1.0).abs() < 1e-6);
        // A flag clear shows no header; one set keeps what is under it
        // undimmed; another makes its panes follow its list.
        let screen = ui.screens.last_mut().unwrap();
        screen.flags = ui::NO_HEADER | ui::NO_OVERLAY | ui::PANE_PER_ITEM | ui::HALF_DIALOG;
        let s = screen_layout_at(&ui, DIALOG, &[], &FlatArt, &mut sources).unwrap();
        assert!(!s.has_header && !s.overlay && s.pane_per_item);
        assert_eq!(s.size, DialogSize::Half);
    }

    #[test]
    fn a_screen_is_found_by_its_whole_name_only() {
        let mut ui = dialog_menus("ui\\screens\\one\\same_name");
        let mut other = ui.screens.last().unwrap().clone();
        other.name = "ui\\screens\\two\\same_name".into();
        other.panes[0].texts[0].string = "two_body".into();
        ui.screens.push(other);
        let mut sources = Vec::new();
        let mut key = |path: &str| {
            screen_layout_at(&ui, path, &[], &FlatArt, &mut sources)
                .map(|s| (s.path.clone(), s.panes[0].texts[0].key.clone()))
        };
        assert_eq!(
            key("UI\\Screens\\Two\\Same_Name"),
            Ok(("ui\\screens\\two\\same_name".into(), "two_body".into()))
        );
        assert_eq!(
            key("ui\\screens\\one\\same_name").unwrap().1,
            "quarter_body"
        );
        assert_eq!(
            key("same_name"),
            Err("no same_name screen in the tags".to_string())
        );
    }

    #[test]
    fn a_catalog_screen_takes_its_scales_and_the_overrides() {
        let path = ScreenId::Lobby.path();
        let mut ui = dialog_menus(&path);
        let screen = ui.screens.last_mut().unwrap();
        let third = screen.panes[1].clone();
        screen.panes.push(third);
        let mut sources = Vec::new();
        let scales = |s: &ScreenLayout| s.panes.iter().map(|p| p.scale).collect::<Vec<_>>();
        let s = screen_layout(&ui, ScreenId::Lobby, &[], &FlatArt, &mut sources).unwrap();
        assert_eq!(s.id, Some(ScreenId::Lobby));
        assert_eq!(scales(&s), [Scale::Vista, Scale::Xbox, Scale::Xbox]);
        let (overrides, unread) = crate::catalog::overrides("lobby:0=xbox, lobby:2=vista");
        assert!(unread.is_empty());
        let s = screen_layout(&ui, ScreenId::Lobby, &overrides, &FlatArt, &mut sources).unwrap();
        assert_eq!(scales(&s), [Scale::Xbox, Scale::Xbox, Scale::Vista]);
        assert_eq!(s.panes[0].bitmaps[0].corner, [-250.0, 200.0]);
        let missing = screen_layout(&ui, ScreenId::Online, &[], &FlatArt, &mut sources);
        assert!(missing.unwrap_err().contains("xbox_live_main_menu"));
    }
}
