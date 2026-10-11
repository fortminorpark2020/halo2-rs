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

use crate::anim::{self, Focus};
use crate::art::ArtSource;
use crate::layout::{self, colour, BitmapWidget, MainMenuLayout, Space, StartLayout};
use crate::paint::{self, Anchor, DrawList, Painter};
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
fn first_shown(rows: usize, visible: usize, focus: usize) -> usize {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::{ArtImage, FlatArt, MemoryArt};
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

    const ROWS: [&str; 3] = ["XBOX LIVE", "SPLIT SCREEN", "SETTINGS"];

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
}
