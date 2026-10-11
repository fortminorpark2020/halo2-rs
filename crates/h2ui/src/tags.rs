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
use crate::layout::{
    self, BitmapWidget, Flat, ListLayout, MainMenuLayout, SkinLayout, StartLayout, TextWidget,
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
        .map_or(Flat::None, |w| w.flat);
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
    let mut out = ListLayout {
        corner: l.corner.map(f32::from),
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
            format!(
                "main menu list skin from skin {} (number {}): {} bitmaps, {} texts, \
                 focus fades of {} and {} ms",
                tag.name,
                l.skin,
                out.skin.bitmaps.len(),
                out.skin.texts.len(),
                look.gain.period_ms,
                look.lose.period_ms
            )
        }
        None => format!(
            "main menu list skin: built-in (skin {} isn't in the globals)",
            l.skin
        ),
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

/// Every bitmap (name and frame) the layouts draw.
pub fn wanted(start: &StartLayout, main: &MainMenuLayout) -> Vec<(String, usize)> {
    start
        .art
        .iter()
        .chain(&main.art)
        .chain(&main.list.skin.bitmaps)
        .chain([&start.framing, &main.framing])
        .map(|w| (w.name.clone(), w.frame))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::{ArtImage, FlatArt, MemoryArt};
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
        let names = wanted(&start, &main);
        assert_eq!(names.len(), 2 + 1 + 2 + 2);
        assert!(names.contains(&("list_bkd_multiply".to_string(), 0)));
        assert_eq!(
            screen(&ui, MAIN_MENU).unwrap().name,
            "ui\\elsewhere\\main_menu"
        );
    }
}
