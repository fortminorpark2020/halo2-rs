//! The menu keys a pad's readings give: a key when A, B, Y, Start, Back
//! or a d-pad direction goes down, the left stick read raw (before the
//! thumbstick layout) as a d-pad past 90%, and the directions repeating
//! while held, as Halo 2's menus do (menu.md 1.2: the stick counts past
//! about 90%, holding repeats every 250 ms).

use super::Key;
use crate::profile::pad;

/// How far the stick must go to count as a direction: 90% of full.
pub const STICK_PRESS: i32 = 29_490;
/// How far back it must come before it counts again: 80% of full (an
/// estimate).
pub const STICK_RELEASE: i32 = 26_214;
/// Seconds between repeats of a held direction, and before the first.
pub const REPEAT: f64 = 0.25;

/// The buttons that give a key when they go down, and their keys.
const BUTTONS: [(u16, Key); 5] = [
    (pad::START, Key::Start),
    (pad::BACK, Key::Back),
    (pad::A, Key::A),
    (pad::B, Key::B),
    (pad::Y, Key::Y),
];

/// The four directions: their d-pad bits and keys.
const DIRECTIONS: [(u16, Key); 4] = [
    (pad::DPAD_UP, Key::Up),
    (pad::DPAD_DOWN, Key::Down),
    (pad::DPAD_LEFT, Key::Left),
    (pad::DPAD_RIGHT, Key::Right),
];

/// A pad's readings turned into menu keys.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PadEdges {
    /// The buttons down at the last reading.
    last: u16,
    /// The direction the stick is held in (an index into `DIRECTIONS`),
    /// until it comes back under `STICK_RELEASE`.
    stick: Option<usize>,
    /// When each held direction repeats next (None: not held).
    next: [Option<f64>; 4],
    /// The next reading only sets what is held (after `reset`).
    settle: bool,
}

impl PadEdges {
    pub fn new() -> PadEdges {
        PadEdges::default()
    }

    /// Forget the pad (another one was picked): what the next reading
    /// finds held gives no keys until it is released and pressed again.
    pub fn reset(&mut self) {
        *self = PadEdges {
            settle: true,
            ..PadEdges::default()
        };
    }

    /// The stick's direction with hysteresis: a new one past
    /// `STICK_PRESS` on its axis (the further axis wins), the held one
    /// kept until its axis comes back under `STICK_RELEASE`.
    fn stick_direction(&mut self, lx: i16, ly: i16) -> Option<usize> {
        let (x, y) = (i32::from(lx), i32::from(ly));
        // How far the stick is pushed towards each direction (+Y is up).
        let toward = [y, -y, -x, x];
        if let Some(d) = self.stick {
            if toward[d] >= STICK_RELEASE {
                return Some(d);
            }
        }
        let pushed = if y.abs() >= x.abs() { [0, 1] } else { [2, 3] };
        self.stick = pushed.into_iter().find(|&d| toward[d] >= STICK_PRESS);
        self.stick
    }

    /// One reading: the pad's raw buttons and left stick, at `now`
    /// seconds. Gives the keys that went down (the directions first, then
    /// the buttons) and the held directions that repeat now.
    pub fn update(&mut self, buttons: u16, lx: i16, ly: i16, now: f64) -> Vec<Key> {
        let stick = self.stick_direction(lx, ly);
        let settle = std::mem::take(&mut self.settle);
        let mut keys = Vec::new();
        for (d, &(bit, key)) in DIRECTIONS.iter().enumerate() {
            let held = buttons & bit != 0 || stick == Some(d);
            self.next[d] = match (held, self.next[d]) {
                (false, _) => None,
                // Held through a reset: no key until pressed again.
                (true, None) if settle => Some(f64::INFINITY),
                (true, None) => {
                    keys.push(key);
                    Some(now + REPEAT)
                }
                (true, Some(at)) if now >= at => {
                    keys.push(key);
                    // One repeat per reading; after a long gap the next
                    // comes a whole step later rather than in a burst.
                    let next = at + REPEAT;
                    Some(if next <= now { now + REPEAT } else { next })
                }
                (true, held) => held,
            };
        }
        let down = buttons & !self.last;
        self.last = buttons;
        if !settle {
            keys.extend(
                BUTTONS
                    .iter()
                    .filter(|(bit, _)| down & bit != 0)
                    .map(|&(_, k)| k),
            );
        }
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_key_per_press() {
        let mut e = PadEdges::new();
        assert_eq!(e.update(pad::A, 0, 0, 0.0), [Key::A]);
        // Held: nothing more, however long.
        assert!(e.update(pad::A, 0, 0, 0.1).is_empty());
        assert!(e.update(pad::A, 0, 0, 5.0).is_empty());
        assert!(e.update(0, 0, 0, 5.1).is_empty());
        assert_eq!(e.update(pad::A, 0, 0, 5.2), [Key::A]);
        // Each button its key; two at once give both.
        let mut e = PadEdges::new();
        for (bit, key) in BUTTONS {
            assert_eq!(e.update(bit, 0, 0, 0.0), [key]);
            assert!(e.update(0, 0, 0, 0.0).is_empty());
        }
        assert_eq!(
            e.update(pad::B | pad::START, 0, 0, 1.0),
            [Key::Start, Key::B]
        );
        // Buttons the menu doesn't use give nothing.
        let mut e = PadEdges::new();
        let other =
            pad::X | pad::LEFT_SHOULDER | pad::RIGHT_SHOULDER | pad::LEFT_THUMB | pad::RIGHT_THUMB;
        assert!(e.update(other, 0, 0, 0.0).is_empty());
        // The d-pad's directions, the first right away.
        let mut e = PadEdges::new();
        for (bit, key) in DIRECTIONS {
            assert_eq!(e.update(bit, 0, 0, 0.0), [key]);
            assert!(e.update(0, 0, 0, 0.0).is_empty());
        }
        // A direction and a button together: the direction first.
        assert_eq!(
            e.update(pad::DPAD_DOWN | pad::A, 0, 0, 2.0),
            [Key::Down, Key::A]
        );
    }

    #[test]
    fn directions_repeat_every_quarter_second_and_buttons_never() {
        let mut e = PadEdges::new();
        assert_eq!(e.update(pad::DPAD_DOWN, 0, 0, 10.0), [Key::Down]);
        assert!(e.update(pad::DPAD_DOWN, 0, 0, 10.2).is_empty());
        assert_eq!(e.update(pad::DPAD_DOWN, 0, 0, 10.25), [Key::Down]);
        assert!(e.update(pad::DPAD_DOWN, 0, 0, 10.4).is_empty());
        assert_eq!(e.update(pad::DPAD_DOWN, 0, 0, 10.5), [Key::Down]);
        // After a long gap (a window drag): one repeat, then a step later.
        assert_eq!(e.update(pad::DPAD_DOWN, 0, 0, 13.0), [Key::Down]);
        assert!(e.update(pad::DPAD_DOWN, 0, 0, 13.1).is_empty());
        assert_eq!(e.update(pad::DPAD_DOWN, 0, 0, 13.25), [Key::Down]);
        // Released and pressed again: right away.
        assert!(e.update(0, 0, 0, 13.3).is_empty());
        assert_eq!(e.update(pad::DPAD_DOWN, 0, 0, 13.31), [Key::Down]);
        // A and B held for seconds give one key each.
        let mut e = PadEdges::new();
        let mut got = Vec::new();
        for k in 0..40 {
            got.extend(e.update(pad::A | pad::B, 0, 0, f64::from(k) * 0.1));
        }
        assert_eq!(got, [Key::A, Key::B]);
        // The d-pad and the stick held the same way repeat as one.
        let mut e = PadEdges::new();
        let mut got = Vec::new();
        for k in 0..5 {
            got.extend(e.update(pad::DPAD_UP, 0, 32767, f64::from(k) * 0.1));
        }
        assert_eq!(got, [Key::Up, Key::Up], "at 0 and 0.25 (read at 0.3)");
    }

    #[test]
    fn the_stick_counts_past_90_percent_and_rearms_under_80() {
        let mut e = PadEdges::new();
        // Under 90%: nothing.
        assert!(e.update(0, 0, 29_000, 0.0).is_empty());
        assert_eq!(e.update(0, 0, 29_490, 0.1), [Key::Up]);
        // Down to 85%: still held, no new key; it repeats on time.
        assert!(e.update(0, 0, 27_800, 0.2).is_empty());
        assert_eq!(e.update(0, 0, 27_800, 0.35), [Key::Up]);
        // Back past 90% from 85% is not a new press.
        assert!(e.update(0, 0, 32_767, 0.4).is_empty());
        // Under 80%: re-armed; past 90% again is a new key at once.
        assert!(e.update(0, 0, 26_000, 0.45).is_empty());
        assert_eq!(e.update(0, 0, 30_000, 0.5), [Key::Up]);
        // Every direction; -32768 counts as down and left.
        let mut e = PadEdges::new();
        assert_eq!(e.update(0, 0, -32768, 0.0), [Key::Down]);
        assert!(e.update(0, 0, 0, 0.0).is_empty());
        assert_eq!(e.update(0, -32768, 0, 0.0), [Key::Left]);
        assert!(e.update(0, 0, 0, 0.0).is_empty());
        assert_eq!(e.update(0, 32767, 0, 0.0), [Key::Right]);
        assert!(e.update(0, 0, 0, 0.0).is_empty());
        // A diagonal goes the further axis's way.
        assert_eq!(e.update(0, 30_000, -31_000, 1.0), [Key::Down]);
        assert!(e.update(0, 0, 0, 1.0).is_empty());
        assert_eq!(e.update(0, 31_000, -30_000, 1.0), [Key::Right]);
        // Held right, swinging to up: right is let go only under 80%, then
        // up counts.
        assert!(e.update(0, 27_000, 20_000, 1.1).is_empty());
        assert_eq!(e.update(0, 10_000, 31_000, 1.2), [Key::Up]);
    }

    #[test]
    fn a_reset_ignores_what_is_held_until_released() {
        let mut e = PadEdges::new();
        assert_eq!(e.update(pad::A, 0, 0, 0.0), [Key::A]);
        e.reset();
        // The new pad has A, Start and up held: nothing, and no repeat.
        assert!(e
            .update(pad::A | pad::START | pad::DPAD_UP, 0, 0, 1.0)
            .is_empty());
        assert!(e
            .update(pad::A | pad::START | pad::DPAD_UP, 0, 0, 2.0)
            .is_empty());
        // Released and pressed: keys again.
        assert!(e.update(0, 0, 0, 2.1).is_empty());
        assert_eq!(
            e.update(pad::A | pad::DPAD_UP, 0, 0, 2.2),
            [Key::Up, Key::A]
        );
        // The stick held through a reset doesn't count either.
        let mut e = PadEdges::new();
        e.reset();
        assert!(e.update(0, 0, 32767, 0.0).is_empty());
        assert!(e.update(0, 0, 32767, 0.5).is_empty());
        assert!(e.update(0, 0, 0, 0.6).is_empty());
        assert_eq!(e.update(0, 0, 32767, 0.7), [Key::Up]);
    }
}
