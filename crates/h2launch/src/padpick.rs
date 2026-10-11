//! Which XInput controller is player 1's when `--pad` doesn't name one
//! (`--pad any`, the default), kept apart from the Windows code so it can
//! be tested anywhere.
//!
//! The first connected pad is player 1's to begin with. Another pad takes
//! over only when it is *used*: one of its buttons goes down, or a trigger
//! or a stick axis comes from rest to a far push, while the current pad
//! has gone `IDLE` seconds without being used and holds no button. A
//! reading that never changes (a worn stick resting outside the dead zone,
//! wheel pedals, a guitar's tilt sensor, a virtual pad's held button)
//! never counts as use, so such a pad can't take player 1 away.

/// How long the current pad must go unused before another can take over.
pub const IDLE: f64 = 2.0;

/// XInput's suggested dead zones (left stick, right stick) and trigger
/// threshold: inside them a stick axis or trigger is at rest.
const STICK_REST: [i32; 2] = [7849, 8689];
const TRIGGER_REST: i32 = 30;
/// A far push: half way.
const STICK_FAR: i32 = 16384;
const TRIGGER_FAR: i32 = 128;

/// One XInput reading, as plain numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    /// XInput's `wButtons`.
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
}

/// What is remembered about one XInput slot between polls.
#[derive(Clone, Copy, Debug)]
pub struct Watch {
    /// A reading has been seen since it connected.
    seen: bool,
    buttons: u16,
    /// Per analog channel (the triggers, then the four stick axes): it
    /// has been at rest since its last far push.
    rested: u8,
    /// When it was last used (seconds).
    pub last_use: f64,
}

impl Default for Watch {
    fn default() -> Self {
        Watch::new()
    }
}

impl Watch {
    /// A slot not seen yet.
    pub const fn new() -> Watch {
        Watch {
            seen: false,
            buttons: 0,
            rested: 0,
            last_use: f64::NEG_INFINITY,
        }
    }

    /// Takes the slot's reading at time `t` (seconds); true when the pad
    /// was just used. The first reading after connecting only sets what
    /// is held, so a button or stick already held then doesn't count.
    pub fn update(&mut self, r: &Reading, t: f64) -> bool {
        let pressed = self.seen && r.buttons & !self.buttons != 0;
        self.buttons = r.buttons;
        self.seen = true;
        let abs = |v: i16| (v as i32).abs();
        let channels = [
            (r.left_trigger as i32, TRIGGER_REST, TRIGGER_FAR),
            (r.right_trigger as i32, TRIGGER_REST, TRIGGER_FAR),
            (abs(r.lx), STICK_REST[0], STICK_FAR),
            (abs(r.ly), STICK_REST[0], STICK_FAR),
            (abs(r.rx), STICK_REST[1], STICK_FAR),
            (abs(r.ry), STICK_REST[1], STICK_FAR),
        ];
        let mut pushed = false;
        for (k, (v, rest, far)) in channels.into_iter().enumerate() {
            let bit = 1u8 << k;
            if v <= rest {
                self.rested |= bit;
            } else if v > far && self.rested & bit != 0 {
                self.rested &= !bit;
                pushed = true;
            }
        }
        let used = pressed || pushed;
        if used {
            self.last_use = t;
        }
        used
    }

    /// A button is held.
    pub fn holding(&self) -> bool {
        self.buttons != 0
    }

    /// Unused for `IDLE` seconds at `t`, holding nothing.
    pub fn idle(&self, t: f64) -> bool {
        !self.holding() && t - self.last_use >= IDLE
    }
}

/// Player 1's pad after a poll at `t`. `cur`: the pad that was player 1's
/// (if any). `connected`: the connected slots, lowest first, each with
/// whether it was just used. `watches`: every slot's `Watch`, updated.
pub fn pick(cur: Option<u32>, connected: &[(u32, bool)], watches: &[Watch], t: f64) -> Option<u32> {
    let cur = cur.filter(|c| connected.iter().any(|(i, _)| i == c));
    let used = connected
        .iter()
        .find(|(i, used)| *used && Some(*i) != cur)
        .map(|p| p.0);
    match cur {
        Some(c) => {
            let idle = watches.get(c as usize).is_none_or(|w| w.idle(t));
            match used {
                Some(u) if idle => Some(u),
                _ => Some(c),
            }
        }
        None => used.or(connected.first().map(|p| p.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u16 = 0x1000;
    const B: u16 = 0x2000;

    /// Polls every slot of `readings` (None: not connected) at `t` and
    /// returns player 1's pad.
    fn poll(
        cur: Option<u32>,
        w: &mut [Watch; 4],
        readings: [Option<Reading>; 4],
        t: f64,
    ) -> Option<u32> {
        let mut connected = Vec::new();
        for (i, r) in readings.iter().enumerate() {
            match r {
                Some(r) => connected.push((i as u32, w[i].update(r, t))),
                None => w[i] = Watch::default(),
            }
        }
        pick(cur, &connected, w, t)
    }

    fn pad(buttons: u16) -> Reading {
        Reading {
            buttons,
            ..Reading::default()
        }
    }

    #[test]
    fn the_first_connected_is_player_ones_until_another_is_used() {
        let mut w = [Watch::default(); 4];
        // Slot 0 a virtual pad that's never touched, slot 2 the real one.
        let mut cur = poll(None, &mut w, [Some(pad(0)), None, Some(pad(0)), None], 0.0);
        assert_eq!(cur, Some(0));
        cur = poll(cur, &mut w, [Some(pad(0)), None, Some(pad(0)), None], 0.1);
        assert_eq!(cur, Some(0));
        // A press on the real one: it takes over at once (slot 0 has
        // never been used).
        cur = poll(cur, &mut w, [Some(pad(0)), None, Some(pad(A)), None], 0.2);
        assert_eq!(cur, Some(2));
        // Holding it, and letting go, keep it.
        cur = poll(cur, &mut w, [Some(pad(0)), None, Some(pad(A)), None], 0.3);
        cur = poll(cur, &mut w, [Some(pad(0)), None, Some(pad(0)), None], 0.4);
        assert_eq!(cur, Some(2));
    }

    #[test]
    fn a_reading_that_never_changes_never_takes_over() {
        let mut w = [Watch::default(); 4];
        // Slot 1: a worn stick resting at 12000, a pedal held down, a
        // button held since it connected.
        let stuck = Reading {
            buttons: B,
            left_trigger: 255,
            lx: 12000,
            ry: -30000,
            ..Reading::default()
        };
        let mut cur = None;
        for k in 0..100 {
            cur = poll(
                cur,
                &mut w,
                [Some(pad(0)), Some(stuck), None, None],
                k as f64 * 0.1,
            );
            assert_eq!(cur, Some(0), "poll {k}");
        }
    }

    #[test]
    fn another_pad_waits_until_the_current_one_is_idle() {
        let mut w = [Watch::default(); 4];
        let mut cur = poll(None, &mut w, [Some(pad(0)), Some(pad(0)), None, None], 0.0);
        // Pad 0 is used at 1 s.
        cur = poll(cur, &mut w, [Some(pad(A)), Some(pad(0)), None, None], 1.0);
        cur = poll(cur, &mut w, [Some(pad(0)), Some(pad(0)), None, None], 1.1);
        assert_eq!(cur, Some(0));
        // A press on pad 1 at 2 s: too soon.
        cur = poll(cur, &mut w, [Some(pad(0)), Some(pad(A)), None, None], 2.0);
        assert_eq!(cur, Some(0));
        // Pad 0 holding a button at 4 s still keeps it.
        cur = poll(cur, &mut w, [Some(pad(B)), Some(pad(0)), None, None], 3.9);
        cur = poll(cur, &mut w, [Some(pad(B)), Some(pad(A)), None, None], 4.0);
        assert_eq!(cur, Some(0));
        // Let go; a new press on pad 1 2 s after pad 0's last use.
        cur = poll(cur, &mut w, [Some(pad(0)), Some(pad(0)), None, None], 5.0);
        cur = poll(cur, &mut w, [Some(pad(0)), Some(pad(A)), None, None], 6.0);
        assert_eq!(cur, Some(1));
    }

    #[test]
    fn a_stick_or_trigger_counts_from_rest_to_a_far_push() {
        let mut w = Watch::default();
        let r = |lt: u8, ly: i16| Reading {
            left_trigger: lt,
            ly,
            ..Reading::default()
        };
        assert!(!w.update(&r(0, 0), 0.0));
        // A little way, then far: one use.
        assert!(!w.update(&r(0, 10000), 0.1));
        assert!(w.update(&r(0, 20000), 0.2));
        assert!(!w.update(&r(0, 32767), 0.3));
        assert_eq!(w.last_use, 0.2);
        // Back to rest and far again: another.
        assert!(!w.update(&r(0, 0), 0.4));
        assert!(w.update(&r(0, -20000), 0.5));
        // The trigger the same.
        assert!(!w.update(&r(20, 0), 0.6));
        assert!(w.update(&r(200, 0), 0.7));
        assert!(!w.update(&r(255, 0), 0.8));
        assert!(w.idle(2.71) && !w.idle(2.6));
    }

    #[test]
    fn a_pad_that_goes_away_hands_over() {
        let mut w = [Watch::default(); 4];
        let mut cur = poll(None, &mut w, [Some(pad(0)), Some(pad(0)), None, None], 0.0);
        assert_eq!(cur, Some(0));
        // Pad 0 unplugged: the next connected one.
        cur = poll(cur, &mut w, [None, Some(pad(0)), None, None], 0.1);
        assert_eq!(cur, Some(1));
        // Plugged back in: pad 1 keeps it until pad 0 is used, and its
        // held button at plugging in doesn't count.
        cur = poll(cur, &mut w, [Some(pad(A)), Some(pad(0)), None, None], 0.2);
        assert_eq!(cur, Some(1));
        cur = poll(
            cur,
            &mut w,
            [Some(pad(A | B)), Some(pad(0)), None, None],
            0.3,
        );
        assert_eq!(cur, Some(0));
        // None at all.
        assert_eq!(poll(cur, &mut w, [None, None, None, None], 0.4), None);
    }
}
