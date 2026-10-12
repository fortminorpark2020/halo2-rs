//! The game menu's safety rules (the game menu's design, I5), as plain
//! Rust so Linux tests cover them. The driver on the window thread
//! (`win::menu`) asks `Watch` twice a tick and does what it says (`Act`):
//!
//! - `start`, every tick: an overlay failure turns the menu off for the
//!   run, so Start and Esc reach the engine again; the game's end closes
//!   the menu (or, after End Game's engine message, shows the end); and
//!   End Game's wait quits once it runs out, with the menu off too.
//! - `panel`, while the menu shows, after it was drawn: a panel not drawn
//!   for `HEALTH_WAIT` while the window isn't minimised closes the menu
//!   and lets go of the game's input, and the `HEALTH_CLOSES`th such close
//!   turns the menu off; a failure to publish the panel turns it off.
//!
//! PLEASE WAIT is exempt from both: it lasts until the process ends (the
//! quit's wait, or End Game's), with the game's input neutral and Start
//! the menu's, whatever the panel does. A failure seen under it turns the
//! menu off once it closes.

use super::Page;

/// How long the panel may go undrawn while the menu shows.
pub const HEALTH_WAIT: f64 = 1.0;
/// How many such closes turn the menu off for the run.
pub const HEALTH_CLOSES: u32 = 3;
/// With `--end-game-message`, how long End Game waits for the game to end
/// before it quits anyway.
pub const END_GAME_WAIT: f64 = 10.0;

/// Why the menu was turned off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Off {
    /// The overlay failed (a Direct3D error or a panic).
    Failed,
    /// Its panel wasn't drawn `HEALTH_CLOSES` times.
    NotDrawn,
}

/// What the driver does, in the order given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// The panel wasn't drawn for `HEALTH_WAIT`: close the menu from any
    /// page and let go of the game's input (the `n`th such close).
    NotDrawn(u32),
    /// The game ended after End Game's engine message: close the menu from
    /// any page, PLEASE WAIT too, so the game's end shows.
    EndGameEnded,
    /// The game ended: `GameMenu::game_over` (it closes the menu unless it
    /// is waiting for the engine to quit).
    GameOver,
    /// Turn the menu off for the run, closing it first: Start and Esc
    /// reach the engine again.
    TurnOff(Off),
    /// End Game's wait ran out with the game still on: quit as Leave Game
    /// does.
    Quit,
}

/// The rules' state over a run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Watch {
    /// Since when the shown panel has been waiting to be drawn.
    behind_since: Option<f64>,
    /// Closes for a panel not drawn.
    closes: u32,
    /// Turned off for the run.
    off: bool,
    /// The game's end was seen.
    ended: bool,
    /// `--end-game-message`: when End Game quits anyway.
    end_game_at: Option<f64>,
}

impl Watch {
    pub fn new() -> Watch {
        Watch::default()
    }

    /// The menu is off for the rest of the run.
    pub fn is_off(&self) -> bool {
        self.off
    }

    /// End Game posted its engine message at `t` (`--end-game-message`):
    /// it quits `END_GAME_WAIT` later unless the game ends first, menu off
    /// or not.
    pub fn end_game(&mut self, t: f64) {
        self.end_game_at = Some(t + END_GAME_WAIT);
    }

    /// The start of a tick at `t`: `failed`, the overlay is off; `page`,
    /// the menu's; `ended`, the game has ended.
    pub fn start(&mut self, t: f64, failed: bool, page: Page, ended: bool) -> Vec<Act> {
        let mut acts = Vec::new();
        if failed && !self.off && page != Page::Wait {
            self.off = true;
            acts.push(Act::TurnOff(Off::Failed));
        }
        if ended && !self.ended {
            self.ended = true;
            if self.end_game_at.take().is_some() {
                acts.push(Act::EndGameEnded);
            } else if !self.off {
                acts.push(Act::GameOver);
            }
        }
        if self.end_game_at.is_some_and(|at| t >= at) {
            self.end_game_at = None;
            acts.push(Act::Quit);
        }
        acts
    }

    /// While the menu shows, after the driver drew it at `t`: `failed`,
    /// the overlay is off (publishing the panel failed, say); `behind`,
    /// the newest panel isn't drawn yet (`behind`); `iconic`, the window
    /// is minimised, so nothing is drawn and nothing is wrong.
    pub fn panel(
        &mut self,
        t: f64,
        failed: bool,
        page: Page,
        behind: bool,
        iconic: bool,
    ) -> Vec<Act> {
        if self.off || page == Page::Closed {
            return Vec::new();
        }
        if page == Page::Wait {
            // It stays until the process ends: a slow or failed panel must
            // not let go of the game's input, nor let the menu open again.
            self.behind_since = None;
            return Vec::new();
        }
        if failed {
            self.off = true;
            return vec![Act::TurnOff(Off::Failed)];
        }
        if !behind || iconic {
            self.behind_since = None;
            return Vec::new();
        }
        let since = *self.behind_since.get_or_insert(t);
        if t - since < HEALTH_WAIT {
            return Vec::new();
        }
        self.behind_since = None;
        self.closes += 1;
        let mut acts = vec![Act::NotDrawn(self.closes)];
        if self.closes >= HEALTH_CLOSES {
            self.off = true;
            acts.push(Act::TurnOff(Off::NotDrawn));
        }
        acts
    }

    /// The menu closed: its next panel gets a full `HEALTH_WAIT`.
    pub fn closed(&mut self) {
        self.behind_since = None;
    }
}

/// Whether the menu's panel is still waiting to be drawn: none was
/// published for this page and size (`shown` false), or the render
/// thread hasn't copied the newest (`drawn`, its generation, below
/// `published`).
pub fn behind(shown: bool, published: u64, drawn: u64) -> bool {
    !shown || drawn < published
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAME: Page = Page::Game { row: 0 };

    #[test]
    fn behind_until_the_newest_panel_is_drawn() {
        assert!(behind(false, 0, 0));
        assert!(behind(false, 3, 3));
        assert!(behind(true, 3, 2));
        assert!(!behind(true, 3, 3));
        assert!(!behind(true, 3, 4));
    }

    #[test]
    fn a_panel_not_drawn_for_a_second_closes_the_menu() {
        let mut w = Watch::new();
        // Drawn: nothing to do, however long.
        for k in 0..300 {
            assert!(w
                .panel(k as f64 * 0.005, false, GAME, false, false)
                .is_empty());
        }
        // Behind from 2.0: closed at 3.0, not before.
        assert!(w.panel(2.0, false, GAME, true, false).is_empty());
        assert!(w.panel(2.999, false, GAME, true, false).is_empty());
        assert_eq!(w.panel(3.0, false, GAME, true, false), [Act::NotDrawn(1)]);
        assert!(!w.is_off());
        // The timer starts again after a close, and when it catches up.
        w.closed();
        assert!(w.panel(10.0, false, GAME, true, false).is_empty());
        assert!(w.panel(10.9, false, GAME, false, false).is_empty());
        assert!(w.panel(11.0, false, GAME, true, false).is_empty());
        assert!(w.panel(11.9, false, GAME, true, false).is_empty());
        assert_eq!(w.panel(12.0, false, GAME, true, false), [Act::NotDrawn(2)]);
        // Any page but PLEASE WAIT.
        for page in [
            Page::Controller { row: 3 },
            Page::Value {
                setting: crate::gamemenu::Setting::Buttons,
                row: 1,
            },
            Page::Confirm {
                ask: crate::gamemenu::Ask::Leave,
                row: 1,
            },
        ] {
            let mut w = Watch::new();
            w.panel(0.0, false, page, true, false);
            assert_eq!(
                w.panel(1.0, false, page, true, false),
                [Act::NotDrawn(1)],
                "{page:?}"
            );
        }
    }

    #[test]
    fn minimised_never_closes_and_the_second_starts_over() {
        let mut w = Watch::new();
        assert!(w.panel(0.0, false, GAME, true, false).is_empty());
        for k in 1..=100 {
            assert!(w
                .panel(0.5 + k as f64 * 0.1, false, GAME, true, true)
                .is_empty());
        }
        // Restored at 11: a full second from there.
        assert!(w.panel(11.0, false, GAME, true, false).is_empty());
        assert!(w.panel(11.9, false, GAME, true, false).is_empty());
        assert_eq!(w.panel(12.0, false, GAME, true, false), [Act::NotDrawn(1)]);
    }

    #[test]
    fn the_third_close_turns_the_menu_off() {
        let mut w = Watch::new();
        for n in 1..=HEALTH_CLOSES {
            let t = n as f64 * 10.0;
            assert!(w.panel(t, false, GAME, true, false).is_empty());
            let acts = w.panel(t + HEALTH_WAIT, false, GAME, true, false);
            if n < HEALTH_CLOSES {
                assert_eq!(acts, [Act::NotDrawn(n)]);
                assert!(!w.is_off());
                w.closed();
            } else {
                assert_eq!(acts, [Act::NotDrawn(n), Act::TurnOff(Off::NotDrawn)]);
                assert!(w.is_off());
            }
        }
        // Off: nothing more from the panel's checks or a failure.
        assert!(w.panel(100.0, false, GAME, true, false).is_empty());
        assert!(w.panel(102.0, true, GAME, true, false).is_empty());
        assert!(w.start(103.0, true, GAME, false).is_empty());
    }

    #[test]
    fn please_wait_never_closes() {
        let mut w = Watch::new();
        // Behind for a while on the GAME MENU, then PLEASE WAIT: nothing,
        // however long, and a failure doesn't turn it off.
        assert!(w.panel(0.0, false, GAME, true, false).is_empty());
        for k in 0..2000 {
            let t = 0.5 + k as f64 * 0.005;
            assert!(w.panel(t, false, Page::Wait, true, false).is_empty());
            assert!(w.start(t, false, Page::Wait, false).is_empty());
        }
        assert!(w.panel(20.0, true, Page::Wait, true, false).is_empty());
        assert!(w.start(20.0, true, Page::Wait, false).is_empty());
        assert!(!w.is_off());
        // Once it closes (End Game's message ended the game), the failure
        // turns the menu off.
        assert_eq!(
            w.start(21.0, true, Page::Closed, false),
            [Act::TurnOff(Off::Failed)]
        );
        assert!(w.is_off());
    }

    #[test]
    fn a_failure_turns_the_menu_off_at_once() {
        // Seen at a tick's start (the render thread's copy failed) on any
        // page but PLEASE WAIT, open or closed.
        for page in [Page::Closed, GAME, Page::Controller { row: 0 }] {
            let mut w = Watch::new();
            assert_eq!(
                w.start(5.0, true, page, false),
                [Act::TurnOff(Off::Failed)],
                "{page:?}"
            );
            assert!(w.is_off());
            assert!(w.start(5.1, true, page, false).is_empty());
        }
        // Seen after drawing (the panel couldn't be published).
        let mut w = Watch::new();
        assert_eq!(
            w.panel(5.0, true, GAME, true, false),
            [Act::TurnOff(Off::Failed)]
        );
        assert!(w.is_off());
    }

    #[test]
    fn the_games_end_closes_the_menu_once() {
        let mut w = Watch::new();
        assert!(w.start(1.0, false, GAME, false).is_empty());
        assert_eq!(w.start(2.0, false, GAME, true), [Act::GameOver]);
        assert!(w.start(2.1, false, Page::Closed, true).is_empty());
        // Turned off before the end: nothing to close.
        let mut w = Watch::new();
        w.start(1.0, true, GAME, false);
        assert!(w.start(2.0, false, Page::Closed, true).is_empty());
    }

    #[test]
    fn end_games_wait_quits_on_time_even_with_the_menu_off() {
        // The game doesn't end: a quit at 10 s, once.
        let mut w = Watch::new();
        w.end_game(5.0);
        assert!(w.start(14.9, false, Page::Wait, false).is_empty());
        assert_eq!(w.start(15.0, false, Page::Wait, false), [Act::Quit]);
        assert!(w.start(16.0, false, Page::Wait, false).is_empty());
        // The game ends first: the menu closes to show it, and no quit.
        let mut w = Watch::new();
        w.end_game(5.0);
        assert_eq!(w.start(8.0, false, Page::Wait, true), [Act::EndGameEnded]);
        assert!(w.start(20.0, false, Page::Closed, true).is_empty());
        // Turned off meanwhile (PLEASE WAIT keeps it on now, but the rule
        // holds whatever turns it off): still quits on time.
        let mut w = Watch::new();
        w.end_game(5.0);
        assert_eq!(
            w.start(6.0, true, Page::Closed, false),
            [Act::TurnOff(Off::Failed)]
        );
        assert!(w.panel(7.0, false, GAME, true, false).is_empty());
        assert!(w.start(14.0, false, Page::Closed, false).is_empty());
        assert_eq!(w.start(15.0, false, Page::Closed, false), [Act::Quit]);
        // Off, and the game ends first: no quit, nothing to close.
        let mut w = Watch::new();
        w.end_game(5.0);
        w.start(6.0, true, Page::Closed, false);
        assert_eq!(w.start(8.0, false, Page::Closed, true), [Act::EndGameEnded]);
        assert!(w.start(20.0, false, Page::Closed, true).is_empty());
    }
}
