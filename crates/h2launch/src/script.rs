//! `--input-script <file>`: timed synthetic input for unattended tests.
//!
//! One command a line, `#` starts a comment:
//!
//! ```text
//! base state1                 # time 0 = when the engine reports map loaded
//! 5     stick L 0 1 for 2     # left stick full forward for 2 s
//! 7.5   press A               # tap A (0.15 s)
//! 8     press RT for 1        # triggers can be pressed like buttons
//! 9     trigger L 0.5 for 1
//! 10    key W for 2           # sets keyboard[W] in the input state only
//! 14    mouse 300 0 for 1     # 300 counts right, spread over 1 s
//! 15    click left for 0.2
//! 16    screenshot
//! 17    log reached the end
//! 18    quit
//! ```
//!
//! `base` picks time 0: `input` (default) the engine's first input poll,
//! `state1` its `set_game_state(1)` (map loaded), `launch` the start of
//! h2launch. Pad and keyboard/mouse input are merged into what the real
//! devices give for local player 0. `stick` lines stand for the physical
//! sticks: the thumbstick layout (`--sticks`) moves them as it does a
//! pad's, and the button presses go through the button layout's mapping
//! in the engine, as a pad's do.

use crate::profile::{mouse, pad};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    Input,
    State1,
    Launch,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Press {
        buttons: u16,
        lt: bool,
        rt: bool,
    },
    Stick {
        right: bool,
        x: i16,
        y: i16,
    },
    Trigger {
        right: bool,
        value: u8,
    },
    Key {
        vk: u8,
    },
    /// Counts spread evenly over the step.
    Mouse {
        dx: f64,
        dy: f64,
    },
    Click {
        bits: u32,
    },
    Screenshot,
    Log(String),
    Quit,
}

impl Action {
    fn instant(&self) -> bool {
        matches!(self, Action::Screenshot | Action::Log(_) | Action::Quit)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub at: f64,
    pub dur: f64,
    pub action: Action,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Script {
    pub base: Base,
    pub steps: Vec<Step>,
}

/// The script's input at one moment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Synth {
    pub pad_buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub left_stick: Option<(i16, i16)>,
    pub right_stick: Option<(i16, i16)>,
    pub keys: Vec<u8>,
    pub mouse_dx: f64,
    pub mouse_dy: f64,
    pub mouse_buttons: u32,
}

impl Synth {
    pub fn pad_active(&self) -> bool {
        self.pad_buttons != 0
            || self.left_trigger != 0
            || self.right_trigger != 0
            || self.left_stick.is_some()
            || self.right_stick.is_some()
    }

    pub fn km_active(&self) -> bool {
        !self.keys.is_empty()
            || self.mouse_dx != 0.0
            || self.mouse_dy != 0.0
            || self.mouse_buttons != 0
    }
}

pub fn button(name: &str) -> Option<(u16, bool, bool)> {
    Some(match name.to_ascii_lowercase().as_str() {
        "a" => (pad::A, false, false),
        "b" => (pad::B, false, false),
        "x" => (pad::X, false, false),
        "y" => (pad::Y, false, false),
        "lb" => (pad::LEFT_SHOULDER, false, false),
        "rb" => (pad::RIGHT_SHOULDER, false, false),
        "ls" => (pad::LEFT_THUMB, false, false),
        "rs" => (pad::RIGHT_THUMB, false, false),
        "start" => (pad::START, false, false),
        "back" => (pad::BACK, false, false),
        "up" => (pad::DPAD_UP, false, false),
        "down" => (pad::DPAD_DOWN, false, false),
        "left" => (pad::DPAD_LEFT, false, false),
        "right" => (pad::DPAD_RIGHT, false, false),
        "lt" => (0, true, false),
        "rt" => (0, false, true),
        _ => return None,
    })
}

/// A Windows virtual-key code by name, or `0x57` / `vk:0x57`.
pub fn key(name: &str) -> Option<u8> {
    let n = name.to_ascii_lowercase();
    if let Some(v) = n.strip_prefix("vk:").map(crate::util::parse_u64) {
        return v.and_then(|v| u8::try_from(v).ok());
    }
    if n.len() == 1 {
        let c = n.as_bytes()[0];
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase());
        }
    }
    if let Some(f) = n.strip_prefix('f').and_then(|d| d.parse::<u8>().ok()) {
        if (1..=12).contains(&f) {
            return Some(0x6F + f);
        }
    }
    Some(match n.as_str() {
        "space" => 0x20,
        "enter" | "return" => 0x0D,
        "esc" | "escape" => 0x1B,
        "tab" => 0x09,
        "backspace" => 0x08,
        "shift" => 0x10,
        "lshift" => 0xA0,
        "rshift" => 0xA1,
        "ctrl" | "control" => 0x11,
        "lctrl" => 0xA2,
        "rctrl" => 0xA3,
        "alt" => 0x12,
        "lalt" => 0xA4,
        "ralt" => 0xA5,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        _ => {
            return n
                .starts_with("0x")
                .then(|| crate::util::parse_u64(&n))
                .flatten()
                .and_then(|v| u8::try_from(v).ok())
        }
    })
}

fn stick_value(s: &str) -> Option<i16> {
    let v: f64 = s.parse().ok()?;
    (-1.0..=1.0)
        .contains(&v)
        .then(|| (v * 32767.0).round() as i16)
}

fn side(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "l" | "left" => Some(false),
        "r" | "right" => Some(true),
        _ => None,
    }
}

impl Script {
    pub fn parse(text: &str) -> Result<Script, String> {
        let mut base = Base::Input;
        let mut steps = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let line = n + 1;
            let body = raw.split('#').next().unwrap_or("").trim();
            if body.is_empty() {
                continue;
            }
            let err = |m: &str| format!("input script line {line}: {m}: {raw:?}");
            let mut words: Vec<&str> = body.split_whitespace().collect();
            if words[0].eq_ignore_ascii_case("base") {
                base = match words.get(1).map(|w| w.to_ascii_lowercase()).as_deref() {
                    Some("input") => Base::Input,
                    Some("state1") => Base::State1,
                    Some("launch") => Base::Launch,
                    _ => return Err(err("base must be input, state1 or launch")),
                };
                continue;
            }
            let at: f64 = words[0]
                .parse()
                .ok()
                .filter(|t: &f64| t.is_finite() && *t >= 0.0)
                .ok_or_else(|| err("expected a time in seconds first"))?;
            // Optional "for <secs>" at the end.
            let mut dur = None;
            if words.len() >= 2 && words[words.len() - 2].eq_ignore_ascii_case("for") {
                let d: f64 = words[words.len() - 1]
                    .parse()
                    .ok()
                    .filter(|d: &f64| d.is_finite() && *d > 0.0)
                    .ok_or_else(|| err("bad duration after 'for'"))?;
                dur = Some(d);
                words.truncate(words.len() - 2);
            }
            let cmd = words
                .get(1)
                .ok_or_else(|| err("missing command"))?
                .to_ascii_lowercase();
            let args = &words[2.min(words.len())..];
            let need = |k: usize| -> Result<(), String> {
                if args.len() == k {
                    Ok(())
                } else {
                    Err(err(&format!("{cmd} takes {k} argument(s)")))
                }
            };
            let (action, default_dur) = match cmd.as_str() {
                "press" => {
                    need(1)?;
                    let (mut buttons, mut lt, mut rt) = (0u16, false, false);
                    for b in args[0].split('+') {
                        let (bits, l, r) = button(b).ok_or_else(|| err("unknown button"))?;
                        buttons |= bits;
                        lt |= l;
                        rt |= r;
                    }
                    (Action::Press { buttons, lt, rt }, 0.15)
                }
                "stick" => {
                    need(3)?;
                    let right = side(args[0]).ok_or_else(|| err("stick L or R"))?;
                    let x = stick_value(args[1]).ok_or_else(|| err("stick x is -1..1"))?;
                    let y = stick_value(args[2]).ok_or_else(|| err("stick y is -1..1"))?;
                    (Action::Stick { right, x, y }, 0.5)
                }
                "trigger" => {
                    need(2)?;
                    let right = side(args[0]).ok_or_else(|| err("trigger L or R"))?;
                    let v: f64 = args[1]
                        .parse()
                        .ok()
                        .filter(|v| (0.0..=1.0).contains(v))
                        .ok_or_else(|| err("trigger value is 0..1"))?;
                    (
                        Action::Trigger {
                            right,
                            value: (v * 255.0).round() as u8,
                        },
                        0.5,
                    )
                }
                "key" => {
                    need(1)?;
                    let vk = key(args[0]).ok_or_else(|| err("unknown key"))?;
                    (Action::Key { vk }, 0.15)
                }
                "mouse" => {
                    need(2)?;
                    let dx: f64 = args[0].parse().map_err(|_| err("bad mouse dx"))?;
                    let dy: f64 = args[1].parse().map_err(|_| err("bad mouse dy"))?;
                    (Action::Mouse { dx, dy }, 0.25)
                }
                "click" => {
                    need(1)?;
                    let bits = match args[0].to_ascii_lowercase().as_str() {
                        "left" => mouse::LEFT,
                        "right" => mouse::RIGHT,
                        "middle" => mouse::MIDDLE,
                        "x1" => mouse::X1,
                        "x2" => mouse::X2,
                        _ => return Err(err("click left, right, middle, x1 or x2")),
                    };
                    (Action::Click { bits }, 0.15)
                }
                "screenshot" => {
                    need(0)?;
                    (Action::Screenshot, 0.0)
                }
                "quit" => {
                    need(0)?;
                    (Action::Quit, 0.0)
                }
                "log" => (Action::Log(args.join(" ")), 0.0),
                _ => return Err(err("unknown command")),
            };
            if action.instant() && dur.is_some() {
                return Err(err("this command takes no 'for'"));
            }
            steps.push(Step {
                at,
                dur: dur.unwrap_or(default_dur),
                action,
                line,
            });
        }
        steps.sort_by(|a, b| a.at.total_cmp(&b.at));
        Ok(Script { base, steps })
    }

    fn active(&self, t: f64) -> impl Iterator<Item = &Step> {
        self.steps
            .iter()
            .filter(move |s| !s.action.instant() && s.at <= t && t < s.at + s.dur)
    }

    /// The input at time `t1`, with mouse motion summed over `(t0, t1]`.
    pub fn sample(&self, t0: f64, t1: f64) -> Synth {
        let mut out = Synth::default();
        for s in self.active(t1) {
            match &s.action {
                Action::Press { buttons, lt, rt } => {
                    out.pad_buttons |= buttons;
                    if *lt {
                        out.left_trigger = 255;
                    }
                    if *rt {
                        out.right_trigger = 255;
                    }
                }
                Action::Stick { right: false, x, y } => out.left_stick = Some((*x, *y)),
                Action::Stick { right: true, x, y } => out.right_stick = Some((*x, *y)),
                Action::Trigger {
                    right: false,
                    value,
                } => out.left_trigger = out.left_trigger.max(*value),
                Action::Trigger { right: true, value } => {
                    out.right_trigger = out.right_trigger.max(*value)
                }
                Action::Key { vk } => {
                    if !out.keys.contains(vk) {
                        out.keys.push(*vk);
                    }
                }
                Action::Click { bits } => out.mouse_buttons |= bits,
                _ => {}
            }
        }
        if t1 > t0 {
            for s in &self.steps {
                if let Action::Mouse { dx, dy } = s.action {
                    let overlap = (t1.min(s.at + s.dur) - t0.max(s.at)).max(0.0);
                    if overlap > 0.0 {
                        out.mouse_dx += dx * overlap / s.dur;
                        out.mouse_dy += dy * overlap / s.dur;
                    }
                }
            }
        }
        out
    }

    /// Screenshot, log and quit steps whose time falls in `(t0, t1]`.
    pub fn instants(&self, t0: f64, t1: f64) -> Vec<&Step> {
        self.steps
            .iter()
            .filter(|s| s.action.instant() && s.at > t0 && s.at <= t1)
            .collect()
    }

    pub fn end(&self) -> f64 {
        self.steps.iter().map(|s| s.at + s.dur).fold(0.0, f64::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: &str = "
# a test
base state1
5     stick L 0 1 for 2
7.5   press A
8     press RT+lb for 1
9     trigger L 0.5 for 1
10    key W for 2
14    mouse 300 -100 for 1
15    click right for 0.2
16    screenshot
17    log reached the end
18    quit
";

    #[test]
    fn parses() {
        let s = Script::parse(S).unwrap();
        assert_eq!(s.base, Base::State1);
        assert_eq!(s.steps.len(), 10);
        assert_eq!(
            s.steps[0],
            Step {
                at: 5.0,
                dur: 2.0,
                action: Action::Stick {
                    right: false,
                    x: 0,
                    y: 32767
                },
                line: 4
            }
        );
        assert_eq!(s.steps[1].dur, 0.15);
        assert_eq!(s.steps[8].action, Action::Log("reached the end".into()));
        assert_eq!(s.end(), 18.0);
    }

    #[test]
    fn samples() {
        let s = Script::parse(S).unwrap();
        assert_eq!(s.sample(0.0, 1.0), Synth::default());
        let a = s.sample(5.9, 6.0);
        assert_eq!(a.left_stick, Some((0, 32767)));
        assert!(a.pad_active());
        assert!(!a.km_active());
        assert_eq!(s.sample(7.0, 7.0).left_stick, None);
        assert_eq!(s.sample(7.5, 7.6).pad_buttons, pad::A);
        assert_eq!(s.sample(7.6, 7.7).pad_buttons, 0);
        let p = s.sample(8.4, 8.5);
        assert_eq!(p.pad_buttons, pad::LEFT_SHOULDER);
        assert_eq!(p.right_trigger, 255);
        assert_eq!(s.sample(9.5, 9.5).left_trigger, 128);
        let k = s.sample(11.0, 11.0);
        assert_eq!(k.keys, vec![b'W']);
        assert!(k.km_active());
        assert_eq!(s.sample(15.1, 15.1).mouse_buttons, mouse::RIGHT);
    }

    #[test]
    fn mouse_motion_adds_up() {
        let s = Script::parse(S).unwrap();
        let mut t = 13.0;
        let (mut x, mut y) = (0.0, 0.0);
        while t < 16.0 {
            let m = s.sample(t, t + 1.0 / 60.0);
            x += m.mouse_dx;
            y += m.mouse_dy;
            t += 1.0 / 60.0;
        }
        assert!((x - 300.0).abs() < 1e-6, "{x}");
        assert!((y + 100.0).abs() < 1e-6, "{y}");
    }

    #[test]
    fn instants() {
        let s = Script::parse(S).unwrap();
        let i = s.instants(15.0, 16.0);
        assert_eq!(i.len(), 1);
        assert_eq!(i[0].action, Action::Screenshot);
        assert!(s.instants(16.0, 16.5).is_empty());
        assert_eq!(s.instants(17.5, 18.0)[0].action, Action::Quit);
    }

    #[test]
    fn errors_name_the_line() {
        let e = Script::parse("1 press Q").unwrap_err();
        assert!(e.contains("line 1") && e.contains("unknown button"), "{e}");
        assert!(Script::parse("x press A").is_err());
        assert!(Script::parse("1 stick L 2 0").is_err());
        assert!(Script::parse("1 screenshot for 2").is_err());
        assert!(Script::parse("1 press A for -1").is_err());
        assert!(Script::parse("base later").is_err());
        assert!(Script::parse("1 dance").is_err());
    }

    #[test]
    fn keys() {
        assert_eq!(key("w"), Some(b'W'));
        assert_eq!(key("7"), Some(b'7'));
        assert_eq!(key("F1"), Some(0x70));
        assert_eq!(key("f12"), Some(0x7B));
        assert_eq!(key("lctrl"), Some(0xA2));
        assert_eq!(key("vk:0x57"), Some(0x57));
        assert_eq!(key("0x41"), Some(0x41));
        assert_eq!(key("nope"), None);
        assert_eq!(button("RT"), Some((0, false, true)));
    }
}
