//! Halo 2's shell around every screen after the main menu (menu.md 1.3,
//! Phase 2.6): `game_shell_background` behind it (the navy and the
//! framing drawn once for a window size, the track strips scrolling over
//! them every frame), the screen's header at the top left and its button
//! legend at the bottom right, all drawn by `h2ui` (`h2ui::chrome`) from
//! the main menu map's tags or the built-in numbers. The screen's own body
//! is drawn between them by the lobby's `Pen` (`App::draw_body`), whose
//! button hints become the legend.

use super::{App, Area, Canvas, Hit, Input, Pen, Screen, Text};
use h2ui::chrome;
use h2ui::layout::{DialogSize, Face, LegendItem, Space};
use h2ui::paint::{Painter, Resources};

/// The legend's button for a hint's press: A, B, X and Y only.
fn face_of(i: Input) -> Option<Face> {
    match i {
        Input::A => Some(Face::A),
        Input::B => Some(Face::B),
        Input::X => Some(Face::X),
        Input::Y => Some(Face::Y),
        _ => None,
    }
}

/// The press a click on a legend's button makes.
fn input_of(face: Face) -> Option<Input> {
    match face {
        Face::A => Some(Input::A),
        Face::B => Some(Input::B),
        Face::X => Some(Input::X),
        Face::Y => Some(Input::Y),
        _ => None,
    }
}

impl App {
    /// The header of the screen on show; none on Halo 2's own screens
    /// before the shell (the intro, the start screen, the main menu).
    pub(super) fn screen_title(&self) -> &'static str {
        match self.screen {
            Screen::Intro | Screen::Start | Screen::Main => "",
            Screen::SignIn => "SIGN IN",
            Screen::Connecting => "SIGNING IN",
            Screen::Live if self.searching() => "MATCHMAKING",
            Screen::Live => "PLAYLISTS",
            Screen::Players => "PLAYERS ONLINE",
            Screen::Party => "PARTY",
            Screen::Custom => "CUSTOM GAME",
            Screen::Pregame => "PREGAME LOBBY",
            Screen::InGame => "IN GAME",
            Screen::Carnage => "CARNAGE REPORT",
            Screen::Failed => "OFFLINE",
            Screen::Record => "SERVICE RECORD",
            Screen::Friends => "FRIENDS",
            Screen::Settings => "CONTROLLER",
        }
    }

    /// A screen after the main menu in Halo 2's shell, into `c` (`view`
    /// is the lobby layout's scale and offset in it): the shell's still
    /// background (drawn again only when it changes, `shell_backdrop`),
    /// its scrolling strips, the screen's body (`draw_body`), then the
    /// header and the legend of the body's hints, which name keys or show
    /// the pad's buttons by the device used last (`keyboard`). A click on
    /// a legend's button presses it: its part of the legend goes into
    /// `areas` (under any popup's). The header and legend's words go into
    /// `drawn`. False when the still background was drawn afresh, so the
    /// frame isn't timed.
    pub(super) fn draw_shell(
        &mut self,
        c: &mut Canvas,
        t: &mut Text,
        view: (f32, f32, f32),
        areas: &mut Vec<Area>,
        drawn: &mut Vec<String>,
    ) -> bool {
        let clock = self.menu_clock();
        let menus = self.cfg.menus.clone();
        let (shell, fonts) = (&menus.shell, &menus.fonts);
        let art = shell.art();
        let res = Resources { art, fonts };
        let (w, h) = (c.w, c.h);
        let space = Space::new(w as u32, h as u32);
        let still = chrome::still(clock, &shell.chrome, art, fonts, space);
        let fresh = self.shell_backdrop.draw(&still, &mut c.px, w, h, &res);
        let moving = chrome::moving(clock, &shell.chrome, art, fonts, space);
        h2ui::cpu::draw_u32(&moving, &mut c.px, w, h, &res);
        let (s, ox, oy) = view;
        let ranks = self.cfg.ranks.clone();
        let mut p = Pen {
            c: &mut *c,
            t,
            s,
            ox,
            oy,
            areas: &mut *areas,
            top: false,
            ranks: ranks.as_deref(),
            drawn: &mut *drawn,
            legend: Vec::new(),
        };
        self.draw_body(&mut p);
        let hints = std::mem::take(&mut p.legend);
        let items: Vec<LegendItem> = hints
            .iter()
            .filter_map(|(i, label)| face_of(*i).map(|f| LegendItem::new(f, label)))
            .collect();
        let mut painter = Painter::new(space, art, fonts, clock);
        let (size, cl) = (DialogSize::Full, &shell.chrome);
        chrome::header(&mut painter, cl, size, self.screen_title());
        let boxes = chrome::legend(&mut painter, cl, size, &items, self.keyboard);
        let list = painter.finish();
        h2ui::cpu::draw_u32(&list, &mut c.px, w, h, &res);
        drawn.extend(list.texts.iter().cloned());
        for ([x0, y0, x1, y1], face) in boxes {
            let Some(input) = input_of(face) else {
                continue;
            };
            areas.push(Area {
                x: (x0 - ox) / s,
                y: (y0 - oy) / s,
                w: (x1 - x0) / s,
                h: (y1 - y0) / s,
                hit: Hit::Press(input),
                top: false,
            });
        }
        !fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_legends_buttons_are_the_hints_presses() {
        for i in [Input::A, Input::B, Input::X, Input::Y] {
            assert_eq!(face_of(i).and_then(input_of), Some(i));
        }
        // The bumpers and the rest stay off the legend.
        for i in [Input::Lb, Input::Rb, Input::Up, Input::Tab] {
            assert_eq!(face_of(i), None);
        }
        assert_eq!(input_of(Face::Start), None);
    }
}
