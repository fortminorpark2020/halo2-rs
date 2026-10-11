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
//!   act in the game. A hold only ever removes input;
//! - `look_flip`: look inversion changed in the menu, applied by flipping
//!   the look stick, since the profile keeps its launch value (the engine
//!   would flip it a second time);
//! - `gate`: the first three together, in the order the input polls run
//!   them.

use crate::controls::Controls;
use crate::profile::{pad, InputFrame};

/// A held trigger stops being held once it reads this or less.
pub const TRIGGER_RELEASE: u8 = 30;
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
/// is open or a hold from it is pending.
pub fn gate(f: &mut InputFrame, ready: bool, open: bool, hold: &mut Hold, frozen_km: Option<bool>) {
    if ready {
        strip(f);
    }
    if open {
        hold.add(f);
        *f = neutral(frozen_km.unwrap_or(f.is_km), (f.cursor_x, f.cursor_y));
        return;
    }
    if !hold.is_empty() {
        hold.apply(f);
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
    /// The triggers, held while above `TRIGGER_RELEASE`.
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

    /// Adds everything held in `f`.
    pub fn add(&mut self, f: &InputFrame) {
        self.pad |= f.pad_buttons;
        self.lt |= f.left_trigger > TRIGGER_RELEASE;
        self.rt |= f.right_trigger > TRIGGER_RELEASE;
        self.mouse |= f.mouse_buttons;
        for (code, _) in f.keys.iter().enumerate().filter(|(_, &k)| k) {
            self.keys[code / 64] |= 1 << (code % 64);
        }
    }

    /// Takes what is still held out of `f`, and lets go of what `f` shows
    /// released: a held button, key or mouse button reads up until it is
    /// up, and a held trigger reads 0 until it is at `TRIGGER_RELEASE` or
    /// less (then it reads what it is). Never adds anything to `f`.
    pub fn apply(&mut self, f: &mut InputFrame) {
        self.pad &= f.pad_buttons;
        f.pad_buttons &= !self.pad;
        for (held, value) in [
            (&mut self.lt, &mut f.left_trigger),
            (&mut self.rt, &mut f.right_trigger),
        ] {
            if *held {
                if *value <= TRIGGER_RELEASE {
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
        h.add(&busy());
        assert!(!h.is_empty());
        // Everything still held: none of it reaches the engine, but the
        // sticks and the mouse's motion do.
        let mut f = busy();
        h.apply(&mut f);
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
        h.apply(&mut f);
        assert_eq!(f.pad_buttons, 0);
        let mut f = busy();
        h.apply(&mut f);
        assert_eq!(f.pad_buttons, pad::A);
        // W released, then pressed again: through. Left Ctrl still held.
        let mut f = busy();
        f.keys[0x57] = false;
        h.apply(&mut f);
        let mut f = busy();
        h.apply(&mut f);
        assert!(f.keys[0x57] && !f.keys[0xA2] && !f.keys[VK_ESCAPE]);
        // The left mouse button released: X2 still held.
        let mut f = busy();
        f.mouse_buttons = mouse::X2;
        h.apply(&mut f);
        assert_eq!(f.mouse_buttons, 0);
        let mut f = busy();
        h.apply(&mut f);
        assert_eq!(f.mouse_buttons, mouse::LEFT);
        // Everything released once: empty.
        let mut f = InputFrame::default();
        h.apply(&mut f);
        assert!(h.is_empty(), "{h:?}");
        assert_eq!(f, InputFrame::default());
        // Cleared by part.
        h.add(&busy());
        h.clear_pad();
        let mut f = busy();
        h.apply(&mut f);
        assert_eq!(f.pad_buttons, busy().pad_buttons);
        assert_eq!(f.left_trigger, 200);
        assert_eq!(f.mouse_buttons, 0);
        h.clear_keys_mouse();
        assert!(h.is_empty());
    }

    #[test]
    fn a_held_trigger_clears_at_30() {
        let mut h = Hold::default();
        h.add(&InputFrame {
            left_trigger: 255,
            right_trigger: TRIGGER_RELEASE,
            ..InputFrame::default()
        });
        // The right trigger at 30 wasn't held at all.
        let mut f = InputFrame {
            left_trigger: 120,
            right_trigger: 200,
            ..InputFrame::default()
        };
        h.apply(&mut f);
        assert_eq!((f.left_trigger, f.right_trigger), (0, 200));
        // Down to 31: still held. At 30: let go, and it reads 30.
        let mut f = InputFrame {
            left_trigger: 31,
            ..InputFrame::default()
        };
        h.apply(&mut f);
        assert_eq!(f.left_trigger, 0);
        assert!(!h.is_empty());
        let mut f = InputFrame {
            left_trigger: 30,
            ..InputFrame::default()
        };
        h.apply(&mut f);
        assert_eq!(f.left_trigger, 30);
        assert!(h.is_empty());
        // Pulled again: through.
        let mut f = InputFrame {
            left_trigger: 255,
            ..InputFrame::default()
        };
        h.apply(&mut f);
        assert_eq!(f.left_trigger, 255);
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
            let mut h = Hold::default();
            h.add(&make(&mut next));
            for _ in 0..4 {
                let before = make(&mut next);
                let mut after = before.clone();
                h.apply(&mut after);
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
        gate(&mut f, false, false, &mut h, None);
        assert_eq!(f, busy());
        assert!(h.is_empty());
        // Ready, closed, nothing held: only Start and Esc go.
        let mut f = busy();
        gate(&mut f, true, false, &mut h, None);
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
        gate(&mut f, true, true, &mut h, Some(true));
        assert_eq!(f, neutral(true, (100.0, 50.0)));
        // Everything but Start and Esc (taken first) went into the hold.
        let mut want = Hold::default();
        let mut stripped = busy();
        strip(&mut stripped);
        want.add(&stripped);
        assert_eq!(h, want);
        assert_eq!(h.pad & pad::START, 0);
        assert_eq!(h.keys[0] & (1 << VK_ESCAPE), 0);
        // With no frozen mode, the frame's own.
        let mut f = busy();
        gate(&mut f, true, true, &mut Hold::default(), None);
        assert!(f.is_km);
        // Even before the menu is ready (it can't be open then, but the
        // order holds): Start stays in the hold's view.
        let mut h = Hold::default();
        let mut f = busy();
        gate(&mut f, false, true, &mut h, None);
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
        h.add(&closing);
        // Still held: masked; the frozen mode stays; the sticks pass.
        closing.thumb_lx = 9000;
        closing.is_km = true;
        let mut f = closing.clone();
        gate(&mut f, true, false, &mut h, Some(false));
        assert_eq!((f.pad_buttons, f.right_trigger, f.thumb_lx), (0, 0, 9000));
        assert!(!f.is_km, "frozen until the hold empties");
        // Start pressed meanwhile is the menu's, never the engine's.
        let mut f = InputFrame {
            pad_buttons: pad::B | pad::START,
            right_trigger: 255,
            ..InputFrame::default()
        };
        gate(&mut f, true, false, &mut h, Some(false));
        assert_eq!(f.pad_buttons, 0);
        // Both released: the hold is empty and the frame passes whole.
        let mut f = InputFrame {
            right_trigger: 20,
            ..InputFrame::default()
        };
        gate(&mut f, true, false, &mut h, Some(false));
        assert!(h.is_empty());
        assert_eq!(f.right_trigger, 20);
        // Pressed again (the caller has unfrozen the mode): through.
        let mut f = busy();
        gate(&mut f, true, false, &mut h, None);
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
