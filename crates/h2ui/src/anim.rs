//! Halo 2's menu animations: the screen animations of the UI globals
//! (`wigl`) that bring each piece of a screen in, the list skins' item
//! animations (the focus fades), the pulse of pulsating text, and art that
//! scrolls.
//!
//! An animation is keyframes of alpha and offset over a period. Where each
//! key falls inside the period was never decoded (every key read with a
//! start of 0), so keys are spread evenly over it, as menu-preview did,
//! until that is checked (menu.md 1.3).

use std::f32::consts::TAU;

/// One keyframe: how opaque, and how far moved (UI units: x, y, z).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Keyframe {
    pub alpha: f32,
    pub position: [f32; 3],
}

/// Keyframes over `period_ms`, spread evenly; the last holds once it's
/// over.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Animation {
    pub period_ms: f32,
    pub keyframes: Vec<Keyframe>,
}

/// The screen fade (the globals' animation 29): 250 ms.
pub const SCREEN_FADE_MS: f32 = 250.0;
/// The slide-ins (animations 1 to 15) run from 320 ms down to 180 ms, 10 ms
/// less each, and come from (or go to) this far right (left).
pub const SLIDE_LONGEST_MS: f32 = 320.0;
pub const SLIDE_STEP_MS: f32 = 10.0;
pub const SLIDE_FROM: f32 = 1024.0;
/// The main menu's skin (11): an item gaining focus, losing it, and the
/// hover fades, and the alphas they run between.
pub const FOCUS_IN_MS: f32 = 120.0;
pub const FOCUS_OUT_MS: f32 = 200.0;
pub const HOVER_MS: f32 = 90.0;
pub const FOCUSED: f32 = 1.0;
pub const UNFOCUSED: f32 = 0.5;
pub const HOVERED: f32 = 0.7;
/// Pulsating text ("PRESS START") breathes once in this long, between
/// these alphas. Estimate (menu-preview's): its animation isn't read yet.
pub const PULSE_SECONDS: f32 = 1.5;
pub const PULSE_LOW: f32 = 0.2;
pub const PULSE_HIGH: f32 = 1.0;
/// Items losing focus followed at once; with more (focus moving faster
/// than every 50 ms or so), the oldest goes straight to rest. Our own
/// choice.
const FADING: usize = 3;

impl Animation {
    /// A fade from `from` to `to` alpha over `period_ms`, in place.
    pub fn fade(period_ms: f32, from: f32, to: f32) -> Animation {
        Animation {
            period_ms,
            keyframes: [from, to]
                .map(|alpha| Keyframe {
                    alpha,
                    position: [0.0; 3],
                })
                .to_vec(),
        }
    }

    /// A move from `from` to `to` (UI units) over `period_ms`, fading from
    /// `alpha[0]` to `alpha[1]`.
    pub fn slide(period_ms: f32, from: [f32; 3], to: [f32; 3], alpha: [f32; 2]) -> Animation {
        Animation {
            period_ms,
            keyframes: vec![
                Keyframe {
                    alpha: alpha[0],
                    position: from,
                },
                Keyframe {
                    alpha: alpha[1],
                    position: to,
                },
            ],
        }
    }

    /// The screen fade in (animation 29): clear to opaque over 250 ms.
    pub fn screen_fade() -> Animation {
        Animation::fade(SCREEN_FADE_MS, 0.0, 1.0)
    }

    /// Slide-in `index` (1 to 15; others are taken as the nearest): from
    /// 1,024 units right, fading in.
    pub fn slide_in(index: usize) -> Animation {
        let period = slide_ms(index);
        Animation::slide(period, [SLIDE_FROM, 0.0, 0.0], [0.0; 3], [0.0, 1.0])
    }

    /// Slide-in `index`'s outro: to 1,024 units left, fading out.
    pub fn slide_out(index: usize) -> Animation {
        let period = slide_ms(index);
        Animation::slide(period, [0.0; 3], [-SLIDE_FROM, 0.0, 0.0], [1.0, 0.0])
    }

    /// An item of the main menu's list gaining focus: half to full.
    pub fn focus_in() -> Animation {
        Animation::fade(FOCUS_IN_MS, UNFOCUSED, FOCUSED)
    }

    /// An item losing focus: full to half.
    pub fn focus_out() -> Animation {
        Animation::fade(FOCUS_OUT_MS, FOCUSED, UNFOCUSED)
    }

    /// An unfocused item under the mouse: half to 0.7.
    pub fn hover() -> Animation {
        Animation::fade(HOVER_MS, UNFOCUSED, HOVERED)
    }

    /// How long it runs, in seconds.
    pub fn seconds(&self) -> f32 {
        self.period_ms.max(0.0) / 1000.0
    }

    /// Its alpha and offset (x, y; UI units) `t` seconds in: before it
    /// starts, its first keyframe; after it ends, its last. No keyframes
    /// is opaque and in place.
    pub fn at(&self, t: f32) -> (f32, [f32; 2]) {
        let keys = &self.keyframes;
        let Some(last) = keys.len().checked_sub(1) else {
            return (1.0, [0.0, 0.0]);
        };
        let along = if self.period_ms <= 0.0 || last == 0 {
            last as f32
        } else {
            (t * 1000.0 / self.period_ms).clamp(0.0, 1.0) * last as f32
        };
        let k = (along.floor() as usize).min(last);
        let (from, to) = (&keys[k], &keys[(k + 1).min(last)]);
        let f = along - k as f32;
        let mix = |a: f32, b: f32| a + (b - a) * f;
        (
            mix(from.alpha, to.alpha),
            [
                mix(from.position[0], to.position[0]),
                mix(from.position[1], to.position[1]),
            ],
        )
    }

    /// The highest alpha it reaches (1 with no keyframes or none above 0).
    pub fn peak(&self) -> f32 {
        self.keyframes
            .iter()
            .map(|k| k.alpha)
            .reduce(f32::max)
            .filter(|&p| p > 0.0)
            .unwrap_or(1.0)
    }
}

/// How long slide-in `index` lasts (ms).
fn slide_ms(index: usize) -> f32 {
    let k = index.clamp(1, 15) - 1;
    SLIDE_LONGEST_MS - SLIDE_STEP_MS * k as f32
}

/// `anim` (none is opaque and in place) `t` seconds after a screen came
/// up, for a piece that waits `delay_ms` first.
pub fn intro(anim: Option<&Animation>, t: f32, delay_ms: f32) -> (f32, [f32; 2]) {
    match anim {
        Some(a) => a.at(t - delay_ms / 1000.0),
        None => (1.0, [0.0, 0.0]),
    }
}

/// How a list skin's items light up: the item gaining focus, and the one
/// losing it, whose last keyframe is where unfocused items sit (a skin's
/// first two item animations; skin 11's by default).
#[derive(Clone, Debug, PartialEq)]
pub struct ItemLook {
    pub gain: Animation,
    pub lose: Animation,
}

impl Default for ItemLook {
    fn default() -> ItemLook {
        ItemLook {
            gain: Animation::focus_in(),
            lose: Animation::focus_out(),
        }
    }
}

impl ItemLook {
    /// How opaque an item with no focus, and none just lost, is.
    pub fn resting(&self) -> f32 {
        self.lose.keyframes.last().map_or(UNFOCUSED, |k| k.alpha)
    }
}

/// `anim`'s alpha `t` seconds in, for an item that was `from` opaque when
/// it started rather than at the animation's first alpha (a fade cut short
/// by focus moving on): from `from` to the last alpha, over the share of
/// the period still to go from there. A two-key fade gives the same as
/// joining it part way.
fn resume(anim: &Animation, from: f32, t: f32) -> f32 {
    let (Some(first), Some(last)) = (anim.keyframes.first(), anim.keyframes.last()) else {
        return 1.0;
    };
    if from == first.alpha {
        return anim.at(t).0;
    }
    let (a, z) = (first.alpha, last.alpha);
    let share = if a == z {
        1.0
    } else {
        ((z - from) / (z - a)).clamp(0.0, 1.0)
    };
    let left = anim.seconds() * share;
    if t >= left {
        return z;
    }
    from + (z - from) * (t / left).max(0.0)
}

/// An item losing focus: which, when (on the menus' clock), and how
/// opaque it was then.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Fade {
    item: usize,
    since: f64,
    from: f32,
}

/// Where a list's focus is: which item has it, since when (on the menus'
/// clock, seconds), and the items still fading out after losing it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Focus {
    pub item: usize,
    pub since: f64,
    /// How opaque the item was when it gained focus (none: as the gain
    /// animation starts).
    from: Option<f32>,
    /// The items losing focus, the latest first.
    fading: [Option<Fade>; FADING],
}

impl Focus {
    /// Focus on `item` since `since`, with nothing before it.
    pub fn on(item: usize, since: f64) -> Focus {
        Focus {
            item,
            since,
            from: None,
            fading: [None; FADING],
        }
    }

    /// The item that had focus before, while it is still fading out.
    pub fn previous(&self) -> Option<usize> {
        self.fading[0].map(|f| f.item)
    }

    /// Focus moved to `item` at `clock` (skin 11's item animations).
    pub fn moved(self, item: usize, clock: f64) -> Focus {
        self.moved_in(&ItemLook::default(), item, clock)
    }

    /// As `moved`, with a skin's own item animations: each item's fade
    /// starts from how opaque it is at `clock`, so moving again before a
    /// fade ends carries on from where it was.
    pub fn moved_in(self, look: &ItemLook, item: usize, clock: f64) -> Focus {
        if item == self.item {
            return self;
        }
        let mut fading = [None; FADING];
        fading[0] = Some(Fade {
            item: self.item,
            since: clock,
            from: self.alpha_in(look, self.item, clock),
        });
        let lose = f64::from(look.lose.seconds());
        let still = self
            .fading
            .iter()
            .flatten()
            .filter(|f| f.item != item && clock - f.since < lose);
        for (slot, f) in fading[1..].iter_mut().zip(still) {
            *slot = Some(*f);
        }
        Focus {
            item,
            since: clock,
            from: Some(self.alpha_in(look, item, clock)),
            fading,
        }
    }

    /// How opaque item `k` is at `clock`: the focused one fades up to
    /// full, those that just lost focus down to half, and the rest sit at
    /// half (skin 11's item animations).
    pub fn alpha(&self, k: usize, clock: f64) -> f32 {
        self.alpha_in(&ItemLook::default(), k, clock)
    }

    /// As `alpha`, with a skin's own item animations.
    pub fn alpha_in(&self, look: &ItemLook, k: usize, clock: f64) -> f32 {
        if k == self.item {
            let t = (clock - self.since) as f32;
            return match self.from {
                Some(from) => resume(&look.gain, from, t),
                None => look.gain.at(t).0,
            };
        }
        match self.fading.iter().flatten().find(|f| f.item == k) {
            Some(f) => resume(&look.lose, f.from, (clock - f.since) as f32),
            None => look.resting(),
        }
    }
}

/// Pulsating text's alpha `t` seconds in: full at 0, faintest half a
/// pulse later. (Seconds as f64, so it stays smooth after hours.)
pub fn pulse(t: f64) -> f32 {
    let phase = (t / f64::from(PULSE_SECONDS)).rem_euclid(1.0) as f32;
    let wave = 0.5 + 0.5 * (phase * TAU).cos();
    PULSE_LOW + (PULSE_HIGH - PULSE_LOW) * wave
}

/// How far art scrolling at `wraps_per_second` has moved by `t` seconds on
/// the menus' clock (a share of its width, 0 to 1).
pub fn scroll(t: f64, wraps_per_second: f32) -> f32 {
    (t * f64::from(wraps_per_second)).rem_euclid(1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn the_screen_fade_takes_a_quarter_second() {
        let a = Animation::screen_fade();
        assert_eq!(a.at(-1.0).0, 0.0);
        assert_eq!(a.at(0.0).0, 0.0);
        assert!(near(a.at(0.125).0, 0.5));
        assert_eq!(a.at(0.25).0, 1.0);
        assert_eq!(a.at(9.0), (1.0, [0.0, 0.0]));
        assert_eq!(a.seconds(), 0.25);
    }

    #[test]
    fn slide_ins_come_from_the_right_and_get_quicker() {
        let a = Animation::slide_in(1);
        assert_eq!(a.period_ms, 320.0);
        assert_eq!(a.at(0.0), (0.0, [1024.0, 0.0]));
        let (alpha, [x, _]) = a.at(0.16);
        assert!(near(alpha, 0.5) && near(x, 512.0));
        assert_eq!(a.at(0.32), (1.0, [0.0, 0.0]));
        assert_eq!(Animation::slide_in(15).period_ms, 180.0);
        assert_eq!(Animation::slide_in(99).period_ms, 180.0);
        assert_eq!(Animation::slide_in(0).period_ms, 320.0);
        // The outro leaves to the left.
        assert_eq!(Animation::slide_out(1).at(1.0), (0.0, [-1024.0, 0.0]));
        // A delay holds the first keyframe.
        assert_eq!(intro(Some(&a), 0.1, 100.0), (0.0, [1024.0, 0.0]));
        assert_eq!(intro(None, 0.0, 100.0), (1.0, [0.0, 0.0]));
    }

    #[test]
    fn keys_are_spread_evenly() {
        // Four keyframes over 300 ms: the fade is in the last 100.
        let a = Animation {
            period_ms: 300.0,
            keyframes: [0.0, 0.0, 0.0, 1.0]
                .map(|alpha| Keyframe {
                    alpha,
                    position: [0.0; 3],
                })
                .to_vec(),
        };
        assert_eq!(a.at(0.15).0, 0.0);
        assert!(near(a.at(0.25).0, 0.5));
        assert_eq!(a.peak(), 1.0);
        assert_eq!(Animation::default().at(0.0), (1.0, [0.0, 0.0]));
        assert_eq!(Animation::fade(250.0, 0.0, 0.5).peak(), 0.5);
    }

    #[test]
    fn focus_fades_up_and_down() {
        let f = Focus::on(0, 1.0).moved(2, 5.0);
        assert_eq!((f.item, f.previous()), (2, Some(0)));
        // 60 ms in: halfway up for the new item, 30% of the way down for
        // the old one.
        assert!(near(f.alpha(2, 5.06), 0.75));
        assert!(near(f.alpha(0, 5.06), 1.0 - 0.5 * 0.3));
        assert!(near(f.alpha(0, 5.1), 0.75));
        assert_eq!(f.alpha(1, 5.06), 0.5);
        // Settled.
        assert_eq!(f.alpha(2, 6.0), 1.0);
        assert_eq!(f.alpha(0, 6.0), 0.5);
        // Moving to where it is changes nothing.
        assert_eq!(f.moved(2, 9.0), f);
        assert_eq!(Animation::hover().at(1.0).0, 0.7);
        // A skin of its own: in over 300 ms, the rest at 0.25.
        let look = ItemLook {
            gain: Animation::fade(300.0, 0.25, 1.0),
            lose: Animation::fade(100.0, 1.0, 0.25),
        };
        let f = Focus::on(0, 1.0).moved_in(&look, 2, 5.0);
        assert!(near(f.alpha_in(&look, 2, 5.15), 0.625));
        assert_eq!(f.alpha_in(&look, 0, 5.2), 0.25);
        assert_eq!(f.alpha_in(&look, 1, 5.0), 0.25);
        assert_eq!(ItemLook::default().resting(), UNFOCUSED);
    }

    #[test]
    fn quick_moves_carry_fades_on_from_where_they_were() {
        // Down, then back up 50 ms later: neither row jumps.
        let down = Focus::on(0, 0.0).moved(1, 1.0);
        let (was0, was1) = (down.alpha(0, 1.05), down.alpha(1, 1.05));
        assert!(near(was0, 0.875) && near(was1, 0.5 + 0.5 * 50.0 / 120.0));
        let up = down.moved(0, 1.05);
        assert!(near(up.alpha(0, 1.05), was0));
        assert!(near(up.alpha(1, 1.05), was1));
        // Row 0 has a quarter of its way up left: 30 ms of the 120.
        assert!(near(up.alpha(0, 1.065), 0.9375));
        assert_eq!(up.alpha(0, 1.09), 1.0);
        // Row 1 goes down from where it got to, at the same rate.
        assert!(up.alpha(1, 1.1) < was1 && up.alpha(1, 1.1) > 0.5);
        assert_eq!(up.alpha(1, 1.2), 0.5);
        // A third row: row 0 keeps fading out while 1 does too.
        let on = down.moved(2, 1.05);
        assert!(near(on.alpha(0, 1.1), 0.75));
        assert!(near(on.alpha(1, 1.05), was1));
        assert_eq!(on.alpha(0, 1.3), 0.5);
        assert_eq!(on.previous(), Some(1));
        // Once its fade is over, a row isn't followed any more.
        let later = on.moved(3, 2.0);
        assert_eq!(later.fading.iter().flatten().count(), 1);
    }

    #[test]
    fn the_pulse_breathes_and_scrolling_wraps() {
        // Still smooth a day in: a 60th of a second moves it.
        let day = 86_400.0;
        assert!((pulse(day + 0.3) - pulse(day + 0.3 + 1.0 / 60.0)).abs() > 0.02);
        assert!(scroll(day + 1.0 / 60.0, 0.1) > scroll(day, 0.1));
        assert!(near(pulse(0.0), 1.0));
        assert!(near(pulse(0.75), 0.2));
        assert!(near(pulse(1.5), 1.0));
        assert!(near(pulse(0.375), 0.6));
        assert!(near(scroll(3.0, 0.2), 0.6));
        assert!(near(scroll(6.0, 0.2), 0.2));
        assert!(near(scroll(1.0, -0.2), 0.8));
        assert_eq!(scroll(5.0, 0.0), 0.0);
    }
}
