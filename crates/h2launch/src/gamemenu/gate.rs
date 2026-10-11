//! What player 0's input polls (host slots 36 and 37) hand the engine
//! around the game menu, over `profile::InputFrame`:
//!
//! - `strip`: once the menu is ready, Start and Esc are the menu's on
//!   every poll, open or not, so the engine never pauses on them;
//! - `neutral`: while the menu is open, a frame with nothing pressed
//!   (keyboard-and-mouse mode and the cursor kept), so our player stands
//!   still while the match goes on;
//! - `Hold`: what was held when the menu closed stays away from the
//!   engine until it is released, one button, key or trigger at a time,
//!   so the press that closed the menu (or one held through it) doesn't
//!   act in the game. A hold only ever removes input. A trigger counts as
//!   released near where it rests (`Troughs`), so a worn pad's trigger
//!   resting above XInput's 30 isn't held for good;
//! - `look_flip`: look inversion changed in the menu, applied by flipping
//!   the look stick, since the profile keeps its launch value (the engine
//!   would flip it a second time);
//! - `gate`: the first three together, in the order the input polls run
//!   them.

use crate::controls::Controls;
use crate::profile::{pad, InputFrame};

/// A held trigger stops being held once it reads this or less, on a pad
/// whose triggers rest at 0 as XInput's should (XInput's own threshold).
pub const TRIGGER_RELEASE: u8 = 30;
/// For a trigger that rests higher (a worn or third-party pad): how far
/// above its trough, its lowest reading since the pad was picked, it
/// counts as pulled. An estimate.
pub const TRIGGER_REST_MARGIN: u8 = 16;
/// The highest trough taken as where a trigger rests: one higher is more
/// likely a pull held since the pad was picked. So a trigger pulled past
/// `TRIGGER_REST_MAX + TRIGGER_REST_MARGIN` (80) is always held. An
/// estimate.
pub const TRIGGER_REST_MAX: u8 = 64;
/// A trough with no reading yet (above any reading, so `min` works).
pub const NO_TROUGH: u16 = 0x100;
/// Windows' virtual-key code for Esc.
pub const VK_ESCAPE: usize = 0x1B;

/// The menu's own input, taken from every poll once it is ready: the
/// pad's Start and the Esc key.
pub fn strip(f: &mut InputFrame) {
    f.pad_buttons &= !pad::START;
    f.keys[VK_ESCAPE] = false;
}

/// A frame with nothing pressed, moved or turned, in keyboard-and-mouse
/// mode or not as `is_km` says (frozen at its value when the menu
/// opened), with the cursor where it was.
pub fn neutral(is_km: bool, (cursor_x, cursor_y): (f32, f32)) -> InputFrame {
    InputFrame {
        is_km,
        cursor_x,
        cursor_y,
        ..InputFrame::default()
    }
}

/// The look stick flipped up for down when the live look inversion
/// differs from the one the engine's profile was given at launch.
pub fn look_flip(f: &mut InputFrame, live: &Controls, launch: &Controls) {
    if live.look_inverted != launch.look_inverted {
        f.thumb_ry = f.thumb_ry.saturating_neg();
    }
}

/// The level at or below which a trigger counts as let go, for a trigger
/// whose trough (lowest reading since its pad was picked) is `trough`
/// (`NO_TROUGH`: none yet): `TRIGGER_RELEASE`, or for a trigger that
/// rests higher, `TRIGGER_REST_MARGIN` above where it rests.
pub fn trigger_release(trough: u16) -> u8 {
    match u8::try_from(trough) {
        Ok(rest) => TRIGGER_RELEASE.max(rest.min(TRIGGER_REST_MAX) + TRIGGER_REST_MARGIN),
        Err(_) => TRIGGER_RELEASE,
    }
}

/// Each trigger's lowest reading since player 0's pad was picked: where
/// it rests, for `Hold`. The input polls keep it (in atomics) and start
/// it again when another pad is picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Troughs {
    pub lt: u16,
    pub rt: u16,
}

impl Troughs {
    /// No reading yet: the triggers count as released at
    /// `TRIGGER_RELEASE`.
    pub const NONE: Troughs = Troughs {
        lt: NO_TROUGH,
        rt: NO_TROUGH,
    };

    /// Takes the triggers' readings in `f`.
    pub fn observe(&mut self, f: &InputFrame) {
        self.lt = self.lt.min(u16::from(f.left_trigger));
        self.rt = self.rt.min(u16::from(f.right_trigger));
    }

    /// Each trigger's release level (`trigger_release`): left, right.
    pub fn release(&self) -> [u8; 2] {
        [trigger_release(self.lt), trigger_release(self.rt)]
    }
}

impl Default for Troughs {
    fn default() -> Troughs {
        Troughs::NONE
    }
}

/// Player 0's frame through the menu's gate, after the pad, keys, mouse
/// and script are merged and before the mouse's scaling, the thumbstick
/// layout, the look flip and mouse-to-stick (the game menu's design,
/// section 3, steps 2 to 4):
///
/// 2. once the menu is `ready`, Start and Esc are taken out;
/// 3. while it is `open`, everything held goes into `hold` and the frame
///    becomes `neutral`, with `is_km` frozen and the cursor kept (the
///    caller then skips the steps after this one);
/// 4. otherwise `hold` takes out what is still held and lets go of what
///    was released, and a frozen `is_km` stays frozen (the caller
///    unfreezes it once `hold` is empty).
///
/// `frozen_km` is `is_km` as it was when the menu opened, while the menu
/// is open or a hold from it is pending. `troughs` say where the
/// triggers rest.
pub fn gate(
    f: &mut InputFrame,
    ready: bool,
    open: bool,
    hold: &mut Hold,
    frozen_km: Option<bool>,
    troughs: Troughs,
) {
    if ready {
        strip(f);
    }
    if open {
        hold.add(f, troughs);
        *f = neutral(frozen_km.unwrap_or(f.is_km), (f.cursor_x, f.cursor_y));
        return;
    }
    if !hold.is_empty() {
        hold.apply(f, troughs);
    }
    if let Some(km) = frozen_km {
        f.is_km = km;
    }
}

/// Input held when the menu closed, kept from the engine until released.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hold {
    /// Pad buttons (XInput bits).
    pad: u16,
    /// The triggers, held while above their release level
    /// (`trigger_release`).
    lt: bool,
    rt: bool,
    /// Mouse buttons (`profile::mouse` bits).
    mouse: u32,
    /// Keys by virtual-key code, 64 to a word.
    keys: [u64; 4],
}

impl Hold {
    /// Nothing held (`Default`, usable in a static).
    pub const fn new() -> Hold {
        Hold {
            pad: 0,
            lt: false,
            rt: false,
            mouse: 0,
            keys: [0; 4],
        }
    }

    /// Adds everything held in `f`: a trigger only when pulled past its
    /// release level, not for resting where it rests.
    pub fn add(&mut self, f: &InputFrame, troughs: Troughs) {
        let [lt, rt] = troughs.release();
        self.pad |= f.pad_buttons;
        self.lt |= f.left_trigger > lt;
        self.rt |= f.right_trigger > rt;
        self.mouse |= f.mouse_buttons;
        for (code, _) in f.keys.iter().enumerate().filter(|(_, &k)| k) {
            self.keys[code / 64] |= 1 << (code % 64);
        }
    }

    /// Takes what is still held out of `f`, and lets go of what `f` shows
    /// released: a held button, key or mouse button reads up until it is
    /// up, and a held trigger reads 0 until it is at its release level or
    /// less (then it reads what it is). Never adds anything to `f`.
    pub fn apply(&mut self, f: &mut InputFrame, troughs: Troughs) {
        self.pad &= f.pad_buttons;
        f.pad_buttons &= !self.pad;
        let [lt, rt] = troughs.release();
        for (held, value, release) in [
            (&mut self.lt, &mut f.left_trigger, lt),
            (&mut self.rt, &mut f.right_trigger, rt),
        ] {
            if *held {
                if *value <= release {
                    *held = false;
                } else {
                    *value = 0;
                }
            }
        }
        self.mouse &= f.mouse_buttons;
        f.mouse_buttons &= !self.mouse;
        for (word, held) in self.keys.iter_mut().enumerate() {
            for bit in 0..64 {
                if *held & (1 << bit) == 0 {
                    continue;
                }
                let key = &mut f.keys[word * 64 + bit];
                if *key {
                    *key = false;
                } else {
                    *held &= !(1 << bit);
                }
            }
        }
    }

    /// Lets go of the pad's buttons and triggers (another pad was picked).
    pub fn clear_pad(&mut self) {
        self.pad = 0;
        self.lt = false;
        self.rt = false;
    }

    /// Lets go of the keys and mouse buttons (the window lost focus, so
    /// their releases won't be seen).
    pub fn clear_keys_mouse(&mut self) {
        self.mouse = 0;
        self.keys = [0; 4];
    }

    pub fn is_empty(&self) -> bool {
        *self == Hold::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::mouse;

    fn busy() -> InputFrame {
        let mut f = InputFrame {
            is_km: true,
            mouse_dx: 3.0,
            mouse_dy: -2.0,
            cursor_x: 100.0,
            cursor_y: 50.0,
            wheel: 1.0,
            mouse_buttons: mouse::LEFT | mouse::X2,
            pad_buttons: pad::A | pad::START | pad::DPAD_UP | pad::BACK,
            left_trigger: 200,
            right_trigger: 31,
            thumb_lx: 1000,
            thumb_ly: -2000,
            thumb_rx: 3000,
            thumb_ry: -32768,
            ..InputFrame::default()
        };
        f.keys[VK_ESCAPE] = true;
        f.keys[0x57] = true;
        f.keys[0xA2] = true;
        f
    }

    #[test]
    fn strip_takes_start_and_esc_only() {
        let mut f = busy();
        strip(&mut f);
        assert_eq!(f.pad_buttons, pad::A | pad::DPAD_UP | pad::BACK);
        assert!(!f.keys[VK_ESCAPE]);
        assert!(f.keys[0x57] && f.keys[0xA2]);
        // The rest untouched.
        let mut want = busy();
        want.pad_buttons &= !pad::START;
        want.keys[VK_ESCAPE] = false;
        assert_eq!(f, want);
        // Nothing to take: no change.
        let mut quiet = InputFrame::default();
        strip(&mut quiet);
        assert_eq!(quiet, InputFrame::default());
    }

    #[test]
    fn neutral_keeps_the_mode_and_the_cursor() {
        for is_km in [false, true] {
            let f = neutral(is_km, (12.5, -3.0));
            assert_eq!(f.is_km, is_km);
            assert_eq!((f.cursor_x, f.cursor_y), (12.5, -3.0));
            let rest = InputFrame {
                is_km,
                cursor_x: 12.5,
                cursor_y: -3.0,
                ..InputFrame::default()
            };
            assert_eq!(f, rest);
            assert!(f.keys.iter().all(|&k| !k));
            assert_eq!((f.mouse_dx, f.mouse_dy, f.wheel), (0.0, 0.0, 0.0));
            assert_eq!((f.pad_buttons, f.left_trigger, f.right_trigger), (0, 0, 0));
        }
    }

    #[test]
    fn a_hold_lets_go_bit_by_bit() {
        let mut h = Hold::default();
        assert!(h.is_empty());
        assert_eq!(Hold::new(), h);
        h.add(&busy(), Troughs::NONE);
        assert!(!h.is_empty());
        // Everything still held: none of it reaches the engine, but the
        // sticks and the mouse's motion do.
        let mut f = busy();
        h.apply(&mut f, Troughs::NONE);
        assert_eq!(f.pad_buttons, 0);
        assert_eq!(f.mouse_buttons, 0);
        assert!(f.keys.iter().all(|&k| !k));
        assert_eq!(f.left_trigger, 0);
        assert_eq!(f.right_trigger, 0);
        assert_eq!((f.thumb_lx, f.mouse_dx), (1000, 3.0));
        // A released: it clears; the others still held. A pressed again
        // then reaches the engine.
        let mut f = busy();
        f.pad_buttons &= !pad::A;
        h.apply(&mut f, Troughs::NONE);
        assert_eq!(f.pad_buttons, 0);
        let mut f = busy();
        h.apply(&mut f, Troughs::NONE);
        assert_eq!(f.pad_buttons, pad::A);
        // W released, then pressed again: through. Left Ctrl still held.
        let mut f = busy();
        f.keys[0x57] = false;
        h.apply(&mut f, Troughs::NONE);
        let mut f = busy();
        h.apply(&mut f, Troughs::NONE);
        assert!(f.keys[0x57] && !f.keys[0xA2] && !f.keys[VK_ESCAPE]);
        // The left mouse button released: X2 still held.
        let mut f = busy();
        f.mouse_buttons = mouse::X2;
        h.apply(&mut f, Troughs::NONE);
        assert_eq!(f.mouse_buttons, 0);
        let mut f = busy();
        h.apply(&mut f, Troughs::NONE);
        assert_eq!(f.mouse_buttons, mouse::LEFT);
        // Everything released once: empty.
        let mut f = InputFrame::default();
        h.apply(&mut f, Troughs::NONE);
        assert!(h.is_empty(), "{h:?}");
        assert_eq!(f, InputFrame::default());
        // Cleared by part.
        h.add(&busy(), Troughs::NONE);
        h.clear_pad();
        let mut f = busy();
        h.apply(&mut f, Troughs::NONE);
        assert_eq!(f.pad_buttons, busy().pad_buttons);
        assert_eq!(f.left_trigger, 200);
        assert_eq!(f.mouse_buttons, 0);
        h.clear_keys_mouse();
        assert!(h.is_empty());
    }

    #[test]
    fn a_held_trigger_clears_at_30() {
        let mut h = Hold::default();
        h.add(
            &InputFrame {
                left_trigger: 255,
                right_trigger: TRIGGER_RELEASE,
                ..InputFrame::default()
            },
            Troughs::NONE,
        );
        // The right trigger at 30 wasn't held at all.
        let mut f = InputFrame {
            left_trigger: 120,
            right_trigger: 200,
            ..InputFrame::default()
        };
        h.apply(&mut f, Troughs::NONE);
        assert_eq!((f.left_trigger, f.right_trigger), (0, 200));
        // Down to 31: still held. At 30: let go, and it reads 30.
        let mut f = InputFrame {
            left_trigger: 31,
            ..InputFrame::default()
        };
        h.apply(&mut f, Troughs::NONE);
        assert_eq!(f.left_trigger, 0);
        assert!(!h.is_empty());
        let mut f = InputFrame {
            left_trigger: 30,
            ..InputFrame::default()
        };
        h.apply(&mut f, Troughs::NONE);
        assert_eq!(f.left_trigger, 30);
        assert!(h.is_empty());
        // Pulled again: through.
        let mut f = InputFrame {
            left_trigger: 255,
            ..InputFrame::default()
        };
        h.apply(&mut f, Troughs::NONE);
        assert_eq!(f.left_trigger, 255);
    }

    /// A trough from a random word: none one time in eight.
    fn n_trough(w: u64) -> Option<u8> {
        (w & 7 != 0).then_some((w >> 8) as u8)
    }

    #[test]
    fn trigger_release_levels() {
        // No reading yet, or a pad that rests low: XInput's 30.
        assert_eq!(trigger_release(NO_TROUGH), TRIGGER_RELEASE);
        for rest in [0, 1, 14] {
            assert_eq!(trigger_release(rest), TRIGGER_RELEASE, "{rest}");
        }
        // Resting higher: 16 above it, up to 80 for a trough of 64 or
        // more (more likely a pull held since the pad was picked).
        assert_eq!(trigger_release(20), 36);
        assert_eq!(trigger_release(40), 56);
        for rest in [64, 65, 200, 255] {
            assert_eq!(trigger_release(rest), 80, "{rest}");
        }
        // The troughs follow the lowest reading.
        let mut t = Troughs::default();
        assert_eq!(t, Troughs::NONE);
        assert_eq!(t.release(), [TRIGGER_RELEASE; 2]);
        for (lt, rt) in [(200, 45), (40, 60), (90, 255)] {
            t.observe(&InputFrame {
                left_trigger: lt,
                right_trigger: rt,
                ..InputFrame::default()
            });
        }
        assert_eq!(t, Troughs { lt: 40, rt: 45 });
        assert_eq!(t.release(), [56, 61]);
    }

    /// The gate for one poll of a pad whose triggers rest where `troughs`
    /// say, the troughs taking the poll's reading first as the input
    /// polls do.
    fn poll(f: &InputFrame, open: bool, h: &mut Hold, troughs: &mut Troughs) -> InputFrame {
        troughs.observe(f);
        let mut f = f.clone();
        gate(&mut f, true, open, h, Some(false), *troughs);
        f
    }

    #[test]
    fn a_trigger_that_rests_above_30_is_not_held_for_good() {
        // A worn pad's right trigger rests at 40, and the game had it
        // there before the menu opened.
        let rest = InputFrame {
            right_trigger: 40,
            ..InputFrame::default()
        };
        let pulled = InputFrame {
            right_trigger: 255,
            ..InputFrame::default()
        };
        let mut troughs = Troughs::NONE;
        let mut h = Hold::default();
        assert_eq!(poll(&rest, false, &mut h, &mut troughs), rest);
        // Opened and closed with the trigger at rest: nothing is held.
        assert_eq!(poll(&rest, true, &mut h, &mut troughs).right_trigger, 0);
        h.add(&rest, troughs);
        assert!(h.is_empty(), "{h:?}");
        assert_eq!(poll(&rest, false, &mut h, &mut troughs), rest);
        assert_eq!(poll(&pulled, false, &mut h, &mut troughs), pulled);
        // Closed with it pulled: held until it is back near its rest,
        // then the next pull reaches the engine.
        assert_eq!(poll(&rest, true, &mut h, &mut troughs).right_trigger, 0);
        h.add(&pulled, troughs);
        assert_eq!(poll(&pulled, false, &mut h, &mut troughs).right_trigger, 0);
        assert_eq!(poll(&rest, false, &mut h, &mut troughs), rest);
        assert!(h.is_empty(), "{h:?}");
        assert_eq!(poll(&pulled, false, &mut h, &mut troughs), pulled);
        // A pad that rests at 0 keeps XInput's 30: 31 is still held.
        let mut troughs = Troughs::NONE;
        let mut h = Hold::default();
        poll(&InputFrame::default(), false, &mut h, &mut troughs);
        h.add(&pulled, troughs);
        let mut f = InputFrame {
            right_trigger: 31,
            ..InputFrame::default()
        };
        assert_eq!(poll(&f, false, &mut h, &mut troughs).right_trigger, 0);
        f.right_trigger = 30;
        assert_eq!(poll(&f, false, &mut h, &mut troughs).right_trigger, 30);
        assert!(h.is_empty());
    }

    #[test]
    fn a_trigger_pulled_as_the_menu_opens_stays_held_until_let_go() {
        // Firing (rest 40) when Start was pressed: held through the menu
        // and after it closes, until let go.
        let mut troughs = Troughs::NONE;
        let mut h = Hold::default();
        let at = |v: u8| InputFrame {
            left_trigger: v,
            ..InputFrame::default()
        };
        poll(&at(40), false, &mut h, &mut troughs);
        for v in [255, 255, 200] {
            assert_eq!(poll(&at(v), true, &mut h, &mut troughs).left_trigger, 0);
        }
        h.add(&at(255), troughs);
        for v in [255, 120, 57] {
            assert_eq!(
                poll(&at(v), false, &mut h, &mut troughs).left_trigger,
                0,
                "{v}"
            );
        }
        assert_eq!(poll(&at(56), false, &mut h, &mut troughs).left_trigger, 56);
        assert!(h.is_empty());
        // A pad picked while its trigger was pulled (its lowest reading
        // so far is the pull): a pull past 80 is still held.
        let mut troughs = Troughs::NONE;
        let mut h = Hold::default();
        poll(&at(250), true, &mut h, &mut troughs);
        h.add(&at(250), troughs);
        assert_eq!(poll(&at(250), false, &mut h, &mut troughs).left_trigger, 0);
        assert_eq!(poll(&at(81), false, &mut h, &mut troughs).left_trigger, 0);
        assert_eq!(poll(&at(0), false, &mut h, &mut troughs).left_trigger, 0);
        assert!(h.is_empty());
        assert_eq!(
            poll(&at(250), false, &mut h, &mut troughs).left_trigger,
            250
        );
    }

    #[test]
    fn apply_never_adds_input() {
        // Holds and frames from a small generator, each frame checked
        // against itself after the hold.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..500 {
            let make = |n: &mut dyn FnMut() -> u64| {
                let mut f = InputFrame {
                    is_km: n() & 1 == 1,
                    mouse_buttons: (n() & 0x1F) as u32,
                    pad_buttons: n() as u16,
                    left_trigger: n() as u8,
                    right_trigger: n() as u8,
                    thumb_ry: n() as i16,
                    ..InputFrame::default()
                };
                for _ in 0..8 {
                    f.keys[(n() & 0xFF) as usize] = true;
                }
                f
            };
            // Troughs anywhere, or none yet.
            let mut trough = || match n_trough(next()) {
                Some(t) => u16::from(t),
                None => NO_TROUGH,
            };
            let troughs = Troughs {
                lt: trough(),
                rt: trough(),
            };
            let mut h = Hold::default();
            h.add(&make(&mut next), troughs);
            for _ in 0..4 {
                let before = make(&mut next);
                let mut after = before.clone();
                h.apply(&mut after, troughs);
                assert_eq!(after.pad_buttons & !before.pad_buttons, 0);
                assert_eq!(after.mouse_buttons & !before.mouse_buttons, 0);
                assert!(after.left_trigger <= before.left_trigger);
                assert!(after.right_trigger <= before.right_trigger);
                for k in 0..256 {
                    assert!(!after.keys[k] || before.keys[k]);
                }
                // What a hold doesn't cover is passed as it was.
                assert_eq!(
                    (after.is_km, after.thumb_ry, after.cursor_x),
                    (before.is_km, before.thumb_ry, before.cursor_x)
                );
            }
        }
    }

    #[test]
    fn the_gate_before_the_menu_is_ready_changes_nothing() {
        let mut h = Hold::default();
        let mut f = busy();
        gate(&mut f, false, false, &mut h, None, Troughs::NONE);
        assert_eq!(f, busy());
        assert!(h.is_empty());
        // Ready, closed, nothing held: only Start and Esc go.
        let mut f = busy();
        gate(&mut f, true, false, &mut h, None, Troughs::NONE);
        let mut want = busy();
        strip(&mut want);
        assert_eq!(f, want);
        assert!(h.is_empty());
    }

    #[test]
    fn the_gate_while_open_holds_everything_and_sends_nothing() {
        let mut h = Hold::default();
        // Opened in keyboard-and-mouse mode; the pad was used since, so
        // the raw frame says pad mode: the frozen mode wins.
        let mut f = busy();
        f.is_km = false;
        gate(&mut f, true, true, &mut h, Some(true), Troughs::NONE);
        assert_eq!(f, neutral(true, (100.0, 50.0)));
        // Everything but Start and Esc (taken first) went into the hold.
        let mut want = Hold::default();
        let mut stripped = busy();
        strip(&mut stripped);
        want.add(&stripped, Troughs::NONE);
        assert_eq!(h, want);
        assert_eq!(h.pad & pad::START, 0);
        assert_eq!(h.keys[0] & (1 << VK_ESCAPE), 0);
        // With no frozen mode, the frame's own.
        let mut f = busy();
        gate(
            &mut f,
            true,
            true,
            &mut Hold::default(),
            None,
            Troughs::NONE,
        );
        assert!(f.is_km);
        // Even before the menu is ready (it can't be open then, but the
        // order holds): Start stays in the hold's view.
        let mut h = Hold::default();
        let mut f = busy();
        gate(&mut f, false, true, &mut h, None, Troughs::NONE);
        assert_ne!(h.pad & pad::START, 0);
    }

    #[test]
    fn the_gate_after_closing_masks_until_release_then_unfreezes() {
        let mut h = Hold::default();
        // The menu closed with B and the right trigger held.
        let mut closing = InputFrame {
            pad_buttons: pad::B,
            right_trigger: 255,
            ..InputFrame::default()
        };
        h.add(&closing, Troughs::NONE);
        // Still held: masked; the frozen mode stays; the sticks pass.
        closing.thumb_lx = 9000;
        closing.is_km = true;
        let mut f = closing.clone();
        gate(&mut f, true, false, &mut h, Some(false), Troughs::NONE);
        assert_eq!((f.pad_buttons, f.right_trigger, f.thumb_lx), (0, 0, 9000));
        assert!(!f.is_km, "frozen until the hold empties");
        // Start pressed meanwhile is the menu's, never the engine's.
        let mut f = InputFrame {
            pad_buttons: pad::B | pad::START,
            right_trigger: 255,
            ..InputFrame::default()
        };
        gate(&mut f, true, false, &mut h, Some(false), Troughs::NONE);
        assert_eq!(f.pad_buttons, 0);
        // Both released: the hold is empty and the frame passes whole.
        let mut f = InputFrame {
            right_trigger: 20,
            ..InputFrame::default()
        };
        gate(&mut f, true, false, &mut h, Some(false), Troughs::NONE);
        assert!(h.is_empty());
        assert_eq!(f.right_trigger, 20);
        // Pressed again (the caller has unfrozen the mode): through.
        let mut f = busy();
        gate(&mut f, true, false, &mut h, None, Troughs::NONE);
        let mut want = busy();
        strip(&mut want);
        assert_eq!(f, want);
    }

    #[test]
    fn look_flip_turns_the_look_stick_over_when_inversion_changed() {
        let launch = Controls::default();
        let flipped = Controls {
            look_inverted: true,
            ..launch
        };
        let mut f = InputFrame {
            thumb_ry: -32768,
            thumb_ly: 500,
            ..InputFrame::default()
        };
        look_flip(&mut f, &flipped, &launch);
        assert_eq!(f.thumb_ry, 32767);
        assert_eq!(f.thumb_ly, 500, "only the look stick");
        look_flip(&mut f, &flipped, &launch);
        assert_eq!(f.thumb_ry, -32767);
        // The same as at launch (the profile already has it): no flip.
        let mut g = InputFrame {
            thumb_ry: 1234,
            ..InputFrame::default()
        };
        look_flip(&mut g, &launch, &launch);
        look_flip(&mut g, &flipped, &flipped);
        assert_eq!(g.thumb_ry, 1234);
        // Launched inverted and switched back to normal: flipped too.
        look_flip(&mut g, &launch, &flipped);
        assert_eq!(g.thumb_ry, -1234);
    }
}
