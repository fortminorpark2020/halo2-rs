//! When the GAME MENU's control changes are said to the lobby (`H2EVENT
//! controls`) or saved in `lobby.txt`: once a visit to CONTROLLER
//! SETTINGS rather than once a step. PC check 6 found 24 steps of Mouse
//! Sensitivity wrote `lobby.txt` 24 times. The live controls still change
//! at once; only the saving waits, until the first of these: the player
//! leaves CONTROLLER SETTINGS (its value pages and the Use Default
//! Settings question count as part of it) or the menu, `SAVE_WAIT` passes
//! after the last change, or a quit starts (`Saves::take`).

use super::{Ask, Page};
use crate::controls::Controls;

/// Seconds after the last change that it is saved, if the player is still
/// in CONTROLLER SETTINGS.
pub const SAVE_WAIT: f64 = 1.0;

/// Changes not saved yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Unsaved {
    /// The controls when they were last saved (or the menu's first change
    /// since).
    pub from: Controls,
    /// The controls now.
    pub to: Controls,
    /// Use Default Settings was among the changes: every control is saved
    /// (`Engine::Controls`' `defaults`).
    pub defaults: bool,
    /// When the last change came (seconds).
    pub at: f64,
}

/// The changes waiting to be saved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Saves {
    unsaved: Option<Unsaved>,
}

impl Saves {
    pub fn new() -> Saves {
        Saves::default()
    }

    /// The menu changed the controls from `from` to `to` at `t`
    /// (`defaults`: Use Default Settings).
    pub fn change(&mut self, from: Controls, to: Controls, defaults: bool, t: f64) {
        match &mut self.unsaved {
            Some(u) => {
                u.to = to;
                u.defaults |= defaults;
                u.at = t;
            }
            None => {
                self.unsaved = Some(Unsaved {
                    from,
                    to,
                    defaults,
                    at: t,
                })
            }
        }
    }

    /// Something is waiting to be saved.
    pub fn waiting(&self) -> bool {
        self.unsaved.is_some()
    }

    /// The changes to save now, at `t` with the menu on `page`, if it is
    /// time: the player isn't in CONTROLLER SETTINGS, or `SAVE_WAIT` has
    /// passed since the last change. None also when the changes came back
    /// to where they started (nothing to save; they are dropped).
    pub fn due(&mut self, t: f64, page: Page) -> Option<Unsaved> {
        let u = self.unsaved?;
        if in_settings(page) && t - u.at < SAVE_WAIT {
            return None;
        }
        self.take()
    }

    /// Every change not saved yet, now (a quit is starting); None as in
    /// `due`.
    pub fn take(&mut self) -> Option<Unsaved> {
        self.unsaved.take().filter(|u| u.defaults || u.to != u.from)
    }
}

/// `page` is CONTROLLER SETTINGS, one of its value pages, or its Use
/// Default Settings question.
pub fn in_settings(page: Page) -> bool {
    matches!(
        page,
        Page::Controller { .. }
            | Page::Value { .. }
            | Page::Confirm {
                ask: Ask::Defaults,
                ..
            }
    )
}

#[cfg(test)]
mod tests {
    use super::super::Setting;
    use super::*;

    const SETTINGS: Page = Page::Controller { row: 6 };

    fn mouse(v: f32) -> Controls {
        Controls {
            mouse_sensitivity: v,
            ..Controls::default()
        }
    }

    #[test]
    fn the_pages_inside_controller_settings() {
        assert!(in_settings(SETTINGS));
        assert!(in_settings(Page::Value {
            setting: Setting::Buttons,
            row: 2
        }));
        assert!(in_settings(Page::Confirm {
            ask: Ask::Defaults,
            row: 1
        }));
        for p in [
            Page::Closed,
            Page::Game { row: 1 },
            Page::Wait,
            Page::Confirm {
                ask: Ask::Leave,
                row: 0,
            },
            Page::Confirm {
                ask: Ask::End,
                row: 1,
            },
        ] {
            assert!(!in_settings(p), "{p:?}");
        }
    }

    #[test]
    fn many_steps_are_saved_once_after_a_second() {
        let mut s = Saves::new();
        assert!(!s.waiting());
        assert_eq!(s.due(0.0, Page::Closed), None);
        // 24 steps of Mouse Sensitivity, 0.25 s apart (a held arrow).
        let mut v = 1.6;
        for k in 0..24 {
            let t = 1.0 + 0.25 * k as f64;
            s.change(mouse(v), mouse(v + 0.1), false, t);
            v += 0.1;
            assert_eq!(s.due(t + 0.2, SETTINGS), None, "step {k}");
        }
        // The last came at 6.75: nothing until 7.75, then all of it once.
        assert_eq!(s.due(7.7, SETTINGS), None);
        let u = s.due(7.75, SETTINGS).expect("due a second after");
        assert_eq!(u.from, mouse(1.6));
        assert_eq!(u.to, mouse(v));
        assert!(!u.defaults);
        assert_eq!(u.at, 6.75);
        assert!(!s.waiting());
        assert_eq!(s.due(9.0, Page::Closed), None);
    }

    #[test]
    fn leaving_controller_settings_saves_at_once() {
        for leave in [Page::Game { row: 1 }, Page::Closed, Page::Wait] {
            let mut s = Saves::new();
            s.change(mouse(1.6), mouse(1.7), false, 2.0);
            // A value page and the defaults question are still inside.
            let value = Page::Value {
                setting: Setting::LookSensitivity,
                row: 0,
            };
            assert_eq!(s.due(2.1, value), None);
            let u = s.due(2.2, leave).expect("due once out");
            assert_eq!((u.from, u.to), (mouse(1.6), mouse(1.7)));
        }
    }

    #[test]
    fn a_quit_takes_what_is_waiting() {
        let mut s = Saves::new();
        s.change(mouse(1.6), mouse(1.7), false, 2.0);
        let u = s.take().expect("taken before the quit");
        assert_eq!(u.to, mouse(1.7));
        assert_eq!(s.take(), None);
    }

    #[test]
    fn defaults_stick_and_a_round_trip_saves_nothing() {
        let mut s = Saves::new();
        let recon = Controls {
            buttons: crate::controls::ButtonLayout::Recon,
            ..Controls::default()
        };
        // Use Default Settings, then a mouse step: every control is saved.
        s.change(recon, Controls::default(), true, 1.0);
        s.change(Controls::default(), mouse(1.7), false, 1.5);
        let u = s.due(2.5, SETTINGS).expect("due");
        assert_eq!((u.from, u.to, u.defaults), (recon, mouse(1.7), true));
        // Use Default Settings that changes nothing still saves every one.
        s.change(Controls::default(), Controls::default(), true, 3.0);
        assert!(s.take().is_some_and(|u| u.defaults));
        // There and back again: nothing to save.
        s.change(mouse(1.6), mouse(1.7), false, 4.0);
        s.change(mouse(1.7), mouse(1.6), false, 4.2);
        assert!(s.waiting());
        assert_eq!(s.due(4.3, Page::Closed), None);
        assert!(!s.waiting());
    }
}
