//! Controllers. On Windows they're read through XInput, so an Xbox
//! controller keeps working its window whether or not the window has
//! focus; a window claims the controllers its players use, so two copies
//! of the game on one PC don't share one (`Claims`). Elsewhere gilrs reads
//! the system's own (evdev on Linux).
//!
//! In a game, what the buttons do is the player's BUTTON LAYOUT: Halo 2's
//! four (DEFAULT, SOUTHPAW, BOXER and GREEN THUMB, as its own BUTTON LAYOUT
//! screen in mainmenu.map draws them), and two from later Halos (BUMPER
//! JUMPER and RECON). Which stick moves and which looks is their THUMBSTICK
//! LAYOUT, Halo 2's four. In every layout Start pauses and Back (held)
//! shows the scoreboard. Menus use the buttons themselves, whatever the
//! layout: the d-pad or left stick moves, A chooses, B goes back, X
//! changes team, Start joins and Back leaves.

use crate::profile::Profile;
use gilrs::ev::filter::{axis_dpad_to_button, Filter};
use gilrs::{Axis, Button, EventType, GamepadId, Gilrs, GilrsBuilder};
use glam::Vec2;
use std::fs::File;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The sticks' dead zone, all of it (gilrs' own filters are off, so
/// XInput's, about a quarter of the way, doesn't stack on it): a fifth of
/// the way out, measured round each stick.
const DEAD_ZONE: f32 = 0.2;
/// Triggers count as pulled past this, and let go under the second; a
/// press (a tapped shot, Boxer's melee) comes at the same pull as a hold.
const TRIGGER: f32 = 0.3;
const TRIGGER_RELEASE: f32 = 0.2;
/// The stick counts as pushed past this, and let go under the second.
const STICK_PUSH: (f32, f32) = (0.6, 0.35);
/// Which system the controllers are read through.
pub const BACKEND: &str = if cfg!(windows) {
    "XInput"
} else if cfg!(target_os = "linux") {
    "evdev"
} else if cfg!(target_os = "macos") {
    "IOKit"
} else {
    "none"
};

/// A controller's buttons by where they are, with Xbox 360 names (an
/// original Xbox controller's White and Black are LB and RB, as Halo 2
/// Vista has them). gilrs calls the bumpers LeftTrigger and RightTrigger,
/// and the triggers LeftTrigger2 and RightTrigger2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PadButton {
    A,
    B,
    X,
    Y,
    LB,
    RB,
    LT,
    RT,
    LeftStick,
    RightStick,
    Back,
    Start,
    Up,
    Down,
    Left,
    Right,
}

/// gilrs' name for each button.
const GILRS: [(Button, PadButton); 16] = [
    (Button::RightTrigger2, PadButton::RT),
    (Button::LeftTrigger2, PadButton::LT),
    (Button::RightTrigger, PadButton::RB),
    (Button::LeftTrigger, PadButton::LB),
    (Button::South, PadButton::A),
    (Button::East, PadButton::B),
    (Button::West, PadButton::X),
    (Button::North, PadButton::Y),
    (Button::LeftThumb, PadButton::LeftStick),
    (Button::RightThumb, PadButton::RightStick),
    (Button::DPadUp, PadButton::Up),
    (Button::DPadDown, PadButton::Down),
    (Button::DPadLeft, PadButton::Left),
    (Button::DPadRight, PadButton::Right),
    (Button::Select, PadButton::Back),
    (Button::Start, PadButton::Start),
];

impl PadButton {
    /// Every button, in the order the controller screen lists them.
    pub const ALL: [PadButton; 16] = [
        PadButton::RT,
        PadButton::LT,
        PadButton::RB,
        PadButton::LB,
        PadButton::A,
        PadButton::B,
        PadButton::X,
        PadButton::Y,
        PadButton::LeftStick,
        PadButton::RightStick,
        PadButton::Up,
        PadButton::Down,
        PadButton::Left,
        PadButton::Right,
        PadButton::Back,
        PadButton::Start,
    ];

    pub fn from_gilrs(b: Button) -> Option<PadButton> {
        GILRS.iter().find(|g| g.0 == b).map(|g| g.1)
    }

    fn to_gilrs(self) -> Button {
        GILRS
            .iter()
            .find(|g| g.1 == self)
            .map_or(Button::Unknown, |g| g.0)
    }

    /// Its name in prompts and on the controller screen.
    pub fn label(self) -> &'static str {
        match self {
            PadButton::A => "A",
            PadButton::B => "B",
            PadButton::X => "X",
            PadButton::Y => "Y",
            PadButton::LB => "LB",
            PadButton::RB => "RB",
            PadButton::LT => "LT",
            PadButton::RT => "RT",
            PadButton::LeftStick => "LEFT STICK",
            PadButton::RightStick => "RIGHT STICK",
            PadButton::Back => "BACK",
            PadButton::Start => "START",
            PadButton::Up => "D-PAD UP",
            PadButton::Down => "D-PAD DOWN",
            PadButton::Left => "D-PAD LEFT",
            PadButton::Right => "D-PAD RIGHT",
        }
    }
}

/// What a button does in a game: the labels of Halo 2's BUTTON LAYOUT
/// screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Function {
    /// Fire (the right hand's gun dual wielding).
    RightWeapon,
    /// Throw a grenade, fire the left hand's gun dual wielding, or a
    /// vehicle's boost or second weapon.
    LeftWeapon,
    /// Boxer's B: a grenade (or a vehicle's boost), never the left gun.
    ThrowGrenade,
    /// Boxer's left trigger: melee, or dual wielding the left gun.
    MeleeOrLeftWeapon,
    /// Reload, and hold to pick up, swap weapons, get in a vehicle or take
    /// a flag.
    Reload,
    /// Switch weapons, and hold by a one-handed gun to dual wield it.
    SwitchWeapons,
    Melee,
    Jump,
    SwapGrenades,
    /// The flashlight: the Arbiter's active camouflage.
    Flashlight,
    Zoom,
    Crouch,
}

impl Function {
    pub const ALL: [Function; 12] = [
        Function::RightWeapon,
        Function::LeftWeapon,
        Function::ThrowGrenade,
        Function::MeleeOrLeftWeapon,
        Function::Reload,
        Function::SwitchWeapons,
        Function::Melee,
        Function::Jump,
        Function::SwapGrenades,
        Function::Flashlight,
        Function::Zoom,
        Function::Crouch,
    ];

    /// Halo 2's name for it (its "Flashlight/Team Chat" without the team
    /// chat: there's no voice).
    pub fn name(self) -> &'static str {
        match self {
            Function::RightWeapon => "USE RIGHT WEAPON",
            Function::LeftWeapon => "USE LEFT WEAPON",
            Function::ThrowGrenade => "THROW GRENADE",
            Function::MeleeOrLeftWeapon => "MELEE/USE LEFT WEAPON",
            Function::Reload => "RELOAD",
            Function::SwitchWeapons => "SWITCH WEAPONS",
            Function::Melee => "MELEE ATTACK",
            Function::Jump => "JUMP",
            Function::SwapGrenades => "SWAP GRENADES",
            Function::Flashlight => "FLASHLIGHT",
            Function::Zoom => "ZOOM VIEW",
            Function::Crouch => "CROUCH",
        }
    }
}

/// What each button does: a BUTTON LAYOUT.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonLayout {
    #[default]
    Default,
    Southpaw,
    Boxer,
    GreenThumb,
    BumperJumper,
    Recon,
}

use Function::*;
use PadButton::{LeftStick, RightStick, A, B, LB, LT, RB, RT, X, Y};

/// Halo 2's own layouts, as its BUTTON LAYOUT screen labels the Xbox 360
/// controller (mainmenu.map's `button_settings` panes over the
/// `button_config` picture).
const DEFAULT: &[(PadButton, Function)] = &[
    (RT, RightWeapon),
    (LT, LeftWeapon),
    (RB, SwapGrenades),
    (LB, Flashlight),
    (A, Jump),
    (B, Melee),
    (X, Reload),
    (Y, SwitchWeapons),
    (LeftStick, Crouch),
    (RightStick, Zoom),
];
/// The triggers swapped.
const SOUTHPAW: &[(PadButton, Function)] = &[
    (RT, LeftWeapon),
    (LT, RightWeapon),
    (RB, SwapGrenades),
    (LB, Flashlight),
    (A, Jump),
    (B, Melee),
    (X, Reload),
    (Y, SwitchWeapons),
    (LeftStick, Crouch),
    (RightStick, Zoom),
];
/// Melee on the left trigger, grenades on B.
const BOXER: &[(PadButton, Function)] = &[
    (RT, RightWeapon),
    (LT, MeleeOrLeftWeapon),
    (RB, SwapGrenades),
    (LB, Flashlight),
    (A, Jump),
    (B, ThrowGrenade),
    (X, Reload),
    (Y, SwitchWeapons),
    (LeftStick, Crouch),
    (RightStick, Zoom),
];
/// Melee on the right stick's click, zoom on B.
const GREEN_THUMB: &[(PadButton, Function)] = &[
    (RT, RightWeapon),
    (LT, LeftWeapon),
    (RB, SwapGrenades),
    (LB, Flashlight),
    (A, Jump),
    (B, Zoom),
    (X, Reload),
    (Y, SwitchWeapons),
    (LeftStick, Crouch),
    (RightStick, Melee),
];
/// Halo 3's: jump and melee on the bumpers, reload and the action on B,
/// grenades swapped with A, and the flashlight where Halo 3 has its
/// equipment, on X.
const BUMPER_JUMPER: &[(PadButton, Function)] = &[
    (RT, RightWeapon),
    (LT, LeftWeapon),
    (RB, Melee),
    (LB, Jump),
    (A, SwapGrenades),
    (B, Reload),
    (X, Flashlight),
    (Y, SwitchWeapons),
    (LeftStick, Crouch),
    (RightStick, Zoom),
];
/// The Master Chief Collection's universal default, its Halo 2 column:
/// reload and the action on RB, the flashlight on X or up on the d-pad,
/// grenades swapped on the d-pad's left and right.
const RECON: &[(PadButton, Function)] = &[
    (RT, RightWeapon),
    (LT, LeftWeapon),
    (RB, Reload),
    (A, Jump),
    (B, Melee),
    (X, Flashlight),
    (Y, SwitchWeapons),
    (LeftStick, Crouch),
    (RightStick, Zoom),
    (PadButton::Up, Flashlight),
    (PadButton::Left, SwapGrenades),
    (PadButton::Right, SwapGrenades),
];

impl ButtonLayout {
    pub const ALL: [ButtonLayout; 6] = [
        ButtonLayout::Default,
        ButtonLayout::Southpaw,
        ButtonLayout::Boxer,
        ButtonLayout::GreenThumb,
        ButtonLayout::BumperJumper,
        ButtonLayout::Recon,
    ];

    /// How the profile saves it.
    pub fn key(self) -> &'static str {
        match self {
            ButtonLayout::Default => "default",
            ButtonLayout::Southpaw => "southpaw",
            ButtonLayout::Boxer => "boxer",
            ButtonLayout::GreenThumb => "green_thumb",
            ButtonLayout::BumperJumper => "bumper_jumper",
            ButtonLayout::Recon => "recon",
        }
    }

    pub fn from_key(key: &str) -> Option<ButtonLayout> {
        Self::ALL
            .into_iter()
            .find(|l| l.key().eq_ignore_ascii_case(key))
    }

    /// Its name in the menus (Halo 2's for its own).
    pub fn name(self) -> &'static str {
        match self {
            ButtonLayout::Default => "DEFAULT",
            ButtonLayout::Southpaw => "SOUTHPAW",
            ButtonLayout::Boxer => "BOXER",
            ButtonLayout::GreenThumb => "GREEN THUMB",
            ButtonLayout::BumperJumper => "BUMPER JUMPER",
            ButtonLayout::Recon => "RECON",
        }
    }

    /// What it's for: Halo 2's help for its own, the remake's for the
    /// others.
    pub fn help(self) -> &'static str {
        match self {
            ButtonLayout::Default => {
                "THE DEFAULT SETTING. THIS IS BUNGIE'S RECOMMENDED BUTTON LAYOUT FOR THE TYPICAL PLAYER."
            }
            ButtonLayout::Southpaw => {
                "THIS SETTING IS IDENTICAL TO THE DEFAULT SETTING EXCEPT THE TRIGGERS ARE SWAPPED FOR LEFTIES."
            }
            ButtonLayout::Boxer => {
                "IF YOU'RE TIRED OF HITTING THE WRONG BUTTON IN THE HEAT OF A MELEE FIGHT, THIS SETTING IS FOR YOU."
            }
            ButtonLayout::GreenThumb => {
                "THIS SETTING ALLOWS YOU TO MELEE ATTACK BY MASHING HARDER ON THE RIGHT THUMBSTICK."
            }
            ButtonLayout::BumperJumper => {
                "JUMP AND MELEE MOVE TO THE BUMPERS, SO YOUR THUMB NEVER LEAVES THE RIGHT THUMBSTICK."
            }
            ButtonLayout::Recon => {
                "THE MASTER CHIEF COLLECTION'S DEFAULT: RELOAD ON RB, FLASHLIGHT ON X, SWAP GRENADES ON THE D-PAD."
            }
        }
    }

    /// Each button it gives a function, and the function.
    pub fn table(self) -> &'static [(PadButton, Function)] {
        match self {
            ButtonLayout::Default => DEFAULT,
            ButtonLayout::Southpaw => SOUTHPAW,
            ButtonLayout::Boxer => BOXER,
            ButtonLayout::GreenThumb => GREEN_THUMB,
            ButtonLayout::BumperJumper => BUMPER_JUMPER,
            ButtonLayout::Recon => RECON,
        }
    }

    /// What button `b` does.
    pub fn function(self, b: PadButton) -> Option<Function> {
        self.table().iter().find(|t| t.0 == b).map(|t| t.1)
    }

    /// The button that does `f` (the first, if two do), for prompts.
    pub fn button(self, f: Function) -> Option<PadButton> {
        self.table().iter().find(|t| t.1 == f).map(|t| t.0)
    }

    /// What button `b` does, in words: Start and Back are the same in
    /// every layout.
    pub fn meaning(self, b: PadButton) -> &'static str {
        match (b, self.function(b)) {
            (PadButton::Back, _) => "MULTIPLAYER SCORE",
            (PadButton::Start, _) => "PAUSE GAME",
            (_, Some(f)) => f.name(),
            (_, None) => "NOTHING",
        }
    }
}

/// Which stick moves and which looks: a THUMBSTICK LAYOUT.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StickLayout {
    #[default]
    Default,
    Southpaw,
    Legacy,
    LegacySouthpaw,
}

impl StickLayout {
    pub const ALL: [StickLayout; 4] = [
        StickLayout::Default,
        StickLayout::Southpaw,
        StickLayout::Legacy,
        StickLayout::LegacySouthpaw,
    ];

    /// How the profile saves it.
    pub fn key(self) -> &'static str {
        match self {
            StickLayout::Default => "default",
            StickLayout::Southpaw => "southpaw",
            StickLayout::Legacy => "legacy",
            StickLayout::LegacySouthpaw => "legacy_southpaw",
        }
    }

    pub fn from_key(key: &str) -> Option<StickLayout> {
        Self::ALL
            .into_iter()
            .find(|l| l.key().eq_ignore_ascii_case(key))
    }

    /// Halo 2's name for it.
    pub fn name(self) -> &'static str {
        match self {
            StickLayout::Default => "DEFAULT",
            StickLayout::Southpaw => "SOUTHPAW",
            StickLayout::Legacy => "LEGACY",
            StickLayout::LegacySouthpaw => "LEGACY SOUTHPAW",
        }
    }

    /// Halo 2's description of it.
    pub fn help(self) -> &'static str {
        match self {
            StickLayout::Default => {
                "DEFAULT CONTROLS YOUR MOVEMENT WITH THE LEFT THUMBSTICK AND YOUR LOOK WITH THE RIGHT THUMBSTICK."
            }
            StickLayout::Southpaw => {
                "SOUTHPAW SWAPS LEFT AND RIGHT THUMBSTICKS FROM THE DEFAULT SETTING. A SPECIAL OPTION FOR LEFTIES."
            }
            StickLayout::Legacy => {
                "LEGACY IS BASED OFF OLDER CONSOLE FPS CONFIGURATIONS. LOOKING AND STRAFING ARE COMBINED."
            }
            StickLayout::LegacySouthpaw => {
                "LEGACY SOUTHPAW SWAPS LEFT AND RIGHT THUMBSTICKS FROM THE LEGACY SETTING. AN OPTION FOR ANCIENT LEFTIES."
            }
        }
    }

    /// What the left and right sticks do, up and down / left and right,
    /// as Halo 2's THUMBSTICK LAYOUT screen labels them.
    pub fn sticks(self) -> [&'static str; 2] {
        match self {
            StickLayout::Default => ["MOVE / STRAFE", "LOOK / ROTATE"],
            StickLayout::Southpaw => ["LOOK / ROTATE", "MOVE / STRAFE"],
            StickLayout::Legacy => ["MOVE / ROTATE", "LOOK / STRAFE"],
            StickLayout::LegacySouthpaw => ["LOOK / STRAFE", "MOVE / ROTATE"],
        }
    }

    /// Movement (x strafe, y forward) and look (x turn, y up) from the
    /// left and right sticks (each through the dead zone already).
    pub fn apply(self, left: Vec2, right: Vec2) -> (Vec2, Vec2) {
        match self {
            StickLayout::Default => (left, right),
            StickLayout::Southpaw => (right, left),
            StickLayout::Legacy => (Vec2::new(right.x, left.y), Vec2::new(left.x, right.y)),
            StickLayout::LegacySouthpaw => (Vec2::new(left.x, right.y), Vec2::new(right.x, left.y)),
        }
    }
}

/// Buttons held: a bit each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonSet(u16);

impl ButtonSet {
    pub fn insert(&mut self, b: PadButton) {
        self.0 |= 1 << b as u16;
    }

    pub fn contains(self, b: PadButton) -> bool {
        self.0 & 1 << b as u16 != 0
    }
}

/// Functions held or pressed: a bit each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FunctionSet(u16);

impl FunctionSet {
    pub fn insert(&mut self, f: Function) {
        self.0 |= 1 << f as u16;
    }

    pub fn contains(self, f: Function) -> bool {
        self.0 & 1 << f as u16 != 0
    }

    pub fn iter(self) -> impl Iterator<Item = Function> {
        Function::ALL.into_iter().filter(move |&f| self.contains(f))
    }
}

/// A controller this frame, by its buttons: the sticks through the dead
/// zone, and the buttons held (the triggers pulled past `TRIGGER`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadReading {
    pub left: Vec2,
    pub right: Vec2,
    pub held: ButtonSet,
}

/// What a controller asks of the game this frame, under its player's
/// layouts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadState {
    /// x: strafe right, y: forward.
    pub movement: Vec2,
    /// x: turn right, y: look up.
    pub look: Vec2,
    pub held: FunctionSet,
    /// Back held: the scoreboard.
    pub scores: bool,
}

impl PadReading {
    /// What this asks of the game under the `buttons` and `sticks`
    /// layouts.
    pub fn state(&self, buttons: ButtonLayout, sticks: StickLayout) -> PadState {
        let (movement, look) = sticks.apply(self.left, self.right);
        let mut held = FunctionSet::default();
        for &(b, f) in buttons.table() {
            if self.held.contains(b) {
                held.insert(f);
            }
        }
        PadState {
            movement,
            look,
            held,
            scores: self.held.contains(PadButton::Back),
        }
    }
}

/// Which way the left stick was pushed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// Something a controller did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadEvent {
    /// A button went down (the d-pad included).
    Down(PadButton),
    /// The left stick pushed one way: for menus only (never the d-pad's
    /// functions in a game, or walking would work Recon's flashlight).
    Stick(Dir),
    /// The controller went (unplugged, or out of battery).
    Disconnected,
    /// It's back.
    Connected,
}

fn dead_zone(v: Vec2) -> Vec2 {
    let len = v.length();
    if len < DEAD_ZONE {
        Vec2::ZERO
    } else {
        v / len * ((len - DEAD_ZONE) / (1.0 - DEAD_ZONE)).min(1.0)
    }
}

/// Our own dead zone and trigger pull, the same on every system: gilrs'
/// filters off, its trigger presses at `TRIGGER`.
fn open_gilrs() -> Result<Gilrs, String> {
    GilrsBuilder::new()
        .with_default_filters(false)
        .set_axis_to_btn(TRIGGER, TRIGGER_RELEASE)
        .build()
        .map_err(|e| e.to_string())
}

/// The controllers connected, a line each.
fn list(g: &Gilrs) -> Vec<String> {
    g.gamepads()
        .map(|(id, pad)| {
            let rumble = if pad.is_ff_supported() { "yes" } else { "no" };
            format!(
                "pad {id}: {}, power {:?}, rumble {rumble}",
                pad.name(),
                pad.power_info()
            )
        })
        .collect()
}

pub struct Pads {
    gilrs: Option<Gilrs>,
    /// Which way each controller's left stick last pointed (x, y), for
    /// menu presses.
    sticks: Vec<(GamepadId, [i8; 2])>,
    /// The controllers this window plays with.
    claims: Claims,
    /// The window has focus (not before it's made): controllers no window
    /// claimed work it.
    focused: bool,
    /// H2_PADS=log: say what the controllers do.
    pub log: bool,
}

impl Pads {
    pub fn new() -> Pads {
        let log = std::env::var("H2_PADS").is_ok_and(|v| v == "log");
        let gilrs = match open_gilrs() {
            Ok(g) => Some(g),
            Err(e) => {
                println!("warning: controllers unavailable: {e}");
                None
            }
        };
        if let (true, Some(g)) = (log, &gilrs) {
            println!("controllers: {BACKEND} (gilrs 0.11.2)");
            let pads = list(g);
            if pads.is_empty() {
                println!("controllers: none connected");
            }
            for line in pads {
                println!("controllers: {line}");
            }
        }
        Pads {
            gilrs,
            sticks: Vec::new(),
            claims: Claims::new(),
            focused: false,
            log,
        }
    }

    /// No controllers, for a copy of the game with no window: XInput
    /// would give it the player's, which are for the windows on this PC.
    pub fn none() -> Pads {
        Pads {
            gilrs: None,
            sticks: Vec::new(),
            claims: Claims::in_dir(None),
            focused: false,
            log: false,
        }
    }

    /// The window gained or lost focus.
    pub fn set_focused(&mut self, focused: bool) {
        if self.log && focused != self.focused {
            let now = if focused { "has" } else { "lost" };
            println!("controllers: the window {now} focus");
        }
        self.focused = focused;
    }

    /// Hold exactly the controllers in `wanted` for this window (those
    /// its players use, or lost and wait for): claim the new ones, let go
    /// of the rest.
    pub fn keep_claimed(&mut self, wanted: &[GamepadId]) {
        for (id, change) in self.claims.keep(wanted) {
            if self.log {
                match change {
                    Claim::Claimed => println!("controllers: pad {id} claimed by this window"),
                    Claim::Released => println!("controllers: pad {id} released"),
                    Claim::Refused => {
                        println!("controllers: pad {id} belongs to another window: ignored")
                    }
                }
            }
        }
    }

    /// What the controllers did since the last call. Only this window's
    /// controllers count, and while it has focus those no other window
    /// claimed.
    pub fn events(&mut self) -> Vec<(GamepadId, PadEvent)> {
        let mut out = Vec::new();
        let Pads {
            gilrs: Some(g),
            sticks,
            claims,
            focused,
            log,
        } = self
        else {
            return out;
        };
        while let Some(ev) = g.next_event() {
            // A d-pad that's a hat (on Linux) as four buttons, as gilrs'
            // own filters have it.
            let ev = ev.filter_ev(&axis_dpad_to_button, g).unwrap_or(ev);
            g.update(&ev);
            let event = match ev.event {
                EventType::Connected => {
                    if *log {
                        let name = g.gamepad(ev.id).name().to_string();
                        println!("controllers: pad {} connected ({name})", ev.id);
                    }
                    Some(PadEvent::Connected)
                }
                EventType::Disconnected => {
                    sticks.retain(|s| s.0 != ev.id);
                    if *log {
                        println!("controllers: pad {} disconnected", ev.id);
                    }
                    Some(PadEvent::Disconnected)
                }
                EventType::ButtonPressed(b, _) => PadButton::from_gilrs(b).map(PadEvent::Down),
                EventType::AxisChanged(axis @ (Axis::LeftStickX | Axis::LeftStickY), v, _) => {
                    let k = match sticks.iter().position(|s| s.0 == ev.id) {
                        Some(k) => k,
                        None => {
                            sticks.push((ev.id, [0, 0]));
                            sticks.len() - 1
                        }
                    };
                    let a = (axis == Axis::LeftStickY) as usize;
                    stick_push(&mut sticks[k].1[a], v).map(|positive| {
                        PadEvent::Stick(match (a, positive) {
                            (0, false) => Dir::Left,
                            (0, true) => Dir::Right,
                            (_, false) => Dir::Down,
                            (_, true) => Dir::Up,
                        })
                    })
                }
                _ => None,
            };
            let Some(event) = event else {
                continue;
            };
            let presses = matches!(event, PadEvent::Down(_) | PadEvent::Stick(_));
            if presses && !claims.accepts(ev.id, *focused) {
                if *log && matches!(event, PadEvent::Down(_)) {
                    let why = if claims.taken_elsewhere(ev.id) {
                        "belongs to another window"
                    } else {
                        "is free, but this window isn't in front"
                    };
                    println!("controllers: pad {} {why}: ignored", ev.id);
                }
                continue;
            }
            out.push((ev.id, event));
        }
        out
    }

    /// Controller `id`'s sticks and buttons now, if it's connected and
    /// this window takes its input.
    pub fn reading(&self, id: GamepadId) -> Option<PadReading> {
        let g = self.gilrs.as_ref()?;
        if !self.claims.accepts(id, self.focused) {
            return None;
        }
        Some(read(&g.connected_gamepad(id)?))
    }
}

/// A controller's sticks and buttons now (a trigger held from `TRIGGER`
/// down to `TRIGGER_RELEASE`, as gilrs' presses go).
fn read(pad: &gilrs::Gamepad) -> PadReading {
    let stick = |x: Axis, y: Axis| dead_zone(Vec2::new(pad.value(x), pad.value(y)));
    let mut held = ButtonSet::default();
    for b in PadButton::ALL {
        if pad.is_pressed(b.to_gilrs()) {
            held.insert(b);
        }
    }
    PadReading {
        left: stick(Axis::LeftStickX, Axis::LeftStickY),
        right: stick(Axis::RightStickX, Axis::RightStickY),
        held,
    }
}

/// The left stick along one axis moved to `v`: Some(which way) when it
/// was pushed that way just now. `dir` is the way it last pointed.
fn stick_push(dir: &mut i8, v: f32) -> Option<bool> {
    if v.abs() < STICK_PUSH.1 {
        *dir = 0;
    } else if v.abs() > STICK_PUSH.0 && *dir != v.signum() as i8 {
        *dir = v.signum() as i8;
        return Some(v > 0.0);
    }
    None
}

/// Controllers this window plays with, held against other copies of the
/// game on this PC: XInput gives every program every controller, and
/// slot n is the same controller in each. A claim is a lock on a file
/// (`pad<n>.lock` in the temporary folder's `halo2-rs`), which the system
/// lets go if the game stops. Only on Windows: elsewhere controllers'
/// numbers differ between programs, and every window takes every
/// controller.
pub struct Claims {
    dir: Option<PathBuf>,
    /// Ours, and each one's locked file (none if it couldn't be made:
    /// ours all the same).
    held: Vec<(GamepadId, Option<File>)>,
    /// Wanted, but another window has them.
    refused: Vec<GamepadId>,
}

/// A change in which controllers a window holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    Claimed,
    Released,
    /// Wanted, but another window has it.
    Refused,
}

/// Whether a window takes a controller's input: always its own, and
/// others only while it has focus, if no other window claimed them
/// (`elsewhere`, asked only then).
fn accepts(ours: bool, elsewhere: impl FnOnce() -> bool, focused: bool) -> bool {
    ours || focused && !elsewhere()
}

impl Claims {
    pub fn new() -> Claims {
        let dir = cfg!(windows).then(|| std::env::temp_dir().join("halo2-rs"));
        Claims::in_dir(dir)
    }

    fn in_dir(dir: Option<PathBuf>) -> Claims {
        if let Some(d) = &dir {
            let _ = std::fs::create_dir_all(d);
        }
        Claims {
            dir,
            held: Vec::new(),
            refused: Vec::new(),
        }
    }

    fn open(&self, id: GamepadId) -> Option<File> {
        let path = self.dir.as_ref()?.join(format!("pad{id}.lock"));
        File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .ok()
    }

    /// Controller `id` is this window's.
    pub fn ours(&self, id: GamepadId) -> bool {
        self.dir.is_none() || self.held.iter().any(|h| h.0 == id)
    }

    /// Another copy of the game on this PC claimed controller `id`.
    pub fn taken_elsewhere(&self, id: GamepadId) -> bool {
        !self.ours(id)
            && self.open(id).is_some_and(|f| match f.try_lock() {
                // Let go at once: closing the file lets go only when
                // Windows gets round to it.
                Ok(()) => {
                    let _ = f.unlock();
                    false
                }
                Err(e) => matches!(e, std::fs::TryLockError::WouldBlock),
            })
    }

    /// Take controller `id` for this window; false if another has it.
    pub fn claim(&mut self, id: GamepadId) -> bool {
        if self.ours(id) {
            return true;
        }
        let file = self.open(id);
        if file.as_ref().is_some_and(|f| f.try_lock().is_err()) {
            return false;
        }
        self.held.push((id, file));
        true
    }

    /// Let controller `id` go (at once, as `taken_elsewhere`).
    pub fn release(&mut self, id: GamepadId) {
        for (_, file) in self.held.iter().filter(|h| h.0 == id) {
            if let Some(f) = file {
                let _ = f.unlock();
            }
        }
        self.held.retain(|h| h.0 != id);
    }

    /// Whether this window takes controller `id`'s input (see `accepts`).
    pub fn accepts(&self, id: GamepadId, focused: bool) -> bool {
        accepts(self.ours(id), || self.taken_elsewhere(id), focused)
    }

    /// Hold exactly the controllers in `wanted`: claim the new ones, let
    /// go of the rest. Returns what changed (a refusal once, until it's no
    /// longer wanted).
    pub fn keep(&mut self, wanted: &[GamepadId]) -> Vec<(GamepadId, Claim)> {
        let mut changed = Vec::new();
        if self.dir.is_none() {
            return changed;
        }
        for &id in wanted {
            if self.ours(id) {
                continue;
            }
            if self.claim(id) {
                self.refused.retain(|&r| r != id);
                changed.push((id, Claim::Claimed));
            } else if !self.refused.contains(&id) {
                self.refused.push(id);
                changed.push((id, Claim::Refused));
            }
        }
        self.refused.retain(|r| wanted.contains(r));
        let gone: Vec<GamepadId> = (self.held.iter().map(|h| h.0))
            .filter(|id| !wanted.contains(id))
            .collect();
        for id in gone {
            self.release(id);
            changed.push((id, Claim::Released));
        }
        changed
    }
}

/// H2_PADS=<seconds>: which controllers there are, a short buzz on each,
/// and for that long every button, trigger and stick, with what player
/// one and the guests (by controller number) would do with it under their
/// layouts. Needs no maps or window: on Windows XInput reports to any
/// program, focused or not.
pub fn probe(seconds: f32, profile: &Profile) {
    use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Repeat, Replay, Ticks};
    println!("controllers: {BACKEND} (gilrs 0.11.2)");
    for k in 0..crate::MAX_LOCAL {
        let c = profile.controls_of(k);
        let who = who(k);
        println!(
            "{who}: button layout {}, thumbstick layout {}, look sensitivity {}{}",
            c.buttons.name(),
            c.sticks.name(),
            c.look_sensitivity,
            if c.invert_look { ", inverted" } else { "" }
        );
    }
    let mut g = match open_gilrs() {
        Ok(g) => g,
        Err(e) => {
            println!("controllers unavailable: {e}");
            return;
        }
    };
    let pads = list(&g);
    if pads.is_empty() {
        println!("no controllers connected");
    }
    for line in pads {
        println!("{line}");
    }
    let ids: Vec<GamepadId> = g
        .gamepads()
        .filter(|(_, p)| p.is_ff_supported())
        .map(|(id, _)| id)
        .collect();
    // Kept to the end: dropped, the buzz would stop there and then.
    let mut buzz = None;
    if !ids.is_empty() {
        let effect = EffectBuilder::new()
            .add_effect(BaseEffect {
                kind: BaseEffectType::Strong { magnitude: 40_000 },
                scheduling: Replay {
                    play_for: Ticks::from_ms(500),
                    ..Default::default()
                },
                envelope: Default::default(),
            })
            // Once: gilrs repeats an effect for good unless told.
            .repeat(Repeat::For(Ticks::from_ms(500)))
            .gamepads(&ids)
            .finish(&mut g);
        match effect.map(|e| e.play().map(|_| e)) {
            Ok(Ok(effect)) => {
                println!("a half second buzz on each");
                buzz = Some(effect);
            }
            Ok(Err(e)) => println!("rumble failed: {e}"),
            Err(e) => println!("rumble failed: {e}"),
        }
    }
    // At least until the buzz is over and gilrs has turned the motors
    // off (a tick or two of 50 ms later): XInput leaves them running if
    // the program ends first. At most a day (infinity is a day, and NaN
    // no time).
    let least = if buzz.is_some() { 1.0 } else { 0.0 };
    let seconds = seconds.max(least).min(86_400.0);
    println!("press buttons, pull the triggers and move the sticks for {seconds} s...");
    let end = Instant::now() + Duration::from_secs_f32(seconds);
    let mut shown = Instant::now();
    while Instant::now() < end {
        let Some(ev) = g.next_event_blocking(Some(Duration::from_millis(50))) else {
            continue;
        };
        let ev = ev.filter_ev(&axis_dpad_to_button, &mut g).unwrap_or(ev);
        g.update(&ev);
        let n = usize::from(ev.id);
        match ev.event {
            EventType::Connected => {
                println!("pad {} connected", ev.id);
                list(&g).iter().for_each(|l| println!("  {l}"));
            }
            EventType::Disconnected => println!("pad {} disconnected", ev.id),
            EventType::ButtonPressed(b, _) => {
                let Some(b) = PadButton::from_gilrs(b) else {
                    continue;
                };
                let layout = profile.controls_of(n).buttons;
                let default = ButtonLayout::Default.meaning(b);
                println!(
                    "pad {} ({}, {}): {} down: {}   (DEFAULT: {default})",
                    ev.id,
                    who(n),
                    layout.name(),
                    b.label(),
                    layout.meaning(b),
                );
            }
            EventType::ButtonReleased(b, _) => {
                if let Some(b) = PadButton::from_gilrs(b) {
                    println!("pad {}: {} up", ev.id, b.label());
                }
            }
            EventType::ButtonChanged(b @ (Button::LeftTrigger2 | Button::RightTrigger2), v, _)
                if shown.elapsed() > Duration::from_millis(100) =>
            {
                shown = Instant::now();
                let name = PadButton::from_gilrs(b).map_or("?", PadButton::label);
                println!("pad {}: {name} {v:.2}", ev.id);
            }
            EventType::AxisChanged(..) if shown.elapsed() > Duration::from_millis(100) => {
                shown = Instant::now();
                if let Some(pad) = g.connected_gamepad(ev.id) {
                    let raw = |a| pad.value(a);
                    let r = read(&pad);
                    println!(
                        "pad {}: left stick ({:+.2}, {:+.2}) right stick ({:+.2}, {:+.2}); \
                         past the dead zone ({:+.2}, {:+.2}) ({:+.2}, {:+.2})",
                        ev.id,
                        raw(Axis::LeftStickX),
                        raw(Axis::LeftStickY),
                        raw(Axis::RightStickX),
                        raw(Axis::RightStickY),
                        r.left.x,
                        r.left.y,
                        r.right.x,
                        r.right.y,
                    );
                }
            }
            _ => {}
        }
    }
    println!("done");
}

/// Local player `k`, in words.
fn who(k: usize) -> String {
    match k {
        0 => "player one".into(),
        k => format!("guest {k}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dead_zone_is_ours_on_every_backend() {
        assert_eq!(dead_zone(Vec2::new(0.19, 0.0)), Vec2::ZERO);
        assert_eq!(dead_zone(Vec2::new(0.13, 0.13)), Vec2::ZERO);
        assert_ne!(dead_zone(Vec2::new(0.15, 0.15)), Vec2::ZERO);
        assert!((dead_zone(Vec2::new(1.0, 0.0)).x - 1.0).abs() < 1e-6);
        assert!((dead_zone(Vec2::new(0.0, -1.0)).y + 1.0).abs() < 1e-6);
        let half = dead_zone(Vec2::new(0.5, 0.0)).x;
        assert!((half - 0.375).abs() < 1e-6, "{half}");
        // Each stick on its own, before Legacy mixes them: a stick in its
        // dead zone moves nothing, whichever way the other is pushed.
        let (left, right) = (Vec2::new(0.1, 0.9), Vec2::new(0.9, 0.1));
        let (movement, look) = StickLayout::Legacy.apply(dead_zone(left), dead_zone(right));
        assert!(movement.x > 0.8 && movement.y > 0.8, "{movement}");
        assert!(look.x.abs() < 0.2 && look.y.abs() < 0.2, "{look}");
    }

    #[test]
    fn halo_2s_layouts_match_its_button_layout_screen() {
        // mainmenu.map's button_settings, pane 0 over the Xbox 360
        // picture: its labels top to bottom down each side.
        let screen = [
            (LT, "USE LEFT WEAPON"),
            (LB, "FLASHLIGHT"),
            (LeftStick, "CROUCH"),
            (RT, "USE RIGHT WEAPON"),
            (RB, "SWAP GRENADES"),
            (X, "RELOAD"),
            (Y, "SWITCH WEAPONS"),
            (B, "MELEE ATTACK"),
            (A, "JUMP"),
            (RightStick, "ZOOM VIEW"),
        ];
        assert_eq!(DEFAULT.len(), screen.len());
        for (b, name) in screen {
            assert_eq!(ButtonLayout::Default.meaning(b), name, "{b:?}");
        }
        assert_eq!(
            ButtonLayout::Default.meaning(PadButton::Back),
            "MULTIPLAYER SCORE"
        );
        assert_eq!(
            ButtonLayout::Default.meaning(PadButton::Start),
            "PAUSE GAME"
        );
        // The others change only what their panes do.
        let changed = |layout: ButtonLayout| -> Vec<(PadButton, Function)> {
            let mut c: Vec<_> = layout
                .table()
                .iter()
                .copied()
                .filter(|t| !DEFAULT.contains(t))
                .collect();
            c.sort_by_key(|t| t.0 as u8);
            c
        };
        assert_eq!(
            changed(ButtonLayout::Southpaw),
            [(LT, RightWeapon), (RT, LeftWeapon)]
        );
        assert_eq!(
            changed(ButtonLayout::Boxer),
            [(B, ThrowGrenade), (LT, MeleeOrLeftWeapon)]
        );
        assert_eq!(
            changed(ButtonLayout::GreenThumb),
            [(B, Zoom), (RightStick, Melee)]
        );
        for layout in [
            ButtonLayout::Southpaw,
            ButtonLayout::Boxer,
            ButtonLayout::GreenThumb,
        ] {
            assert_eq!(layout.table().len(), DEFAULT.len());
        }
    }

    #[test]
    fn every_layout_does_everything_once() {
        for layout in ButtonLayout::ALL {
            let has = |f: Function| layout.button(f).is_some();
            // Fire, a grenade, melee, jump, reload, switch weapons, swap
            // grenades, flashlight, zoom and crouch.
            assert!(has(RightWeapon), "{layout:?}");
            assert!(has(LeftWeapon) || has(ThrowGrenade), "{layout:?}");
            assert!(has(Melee) || has(MeleeOrLeftWeapon), "{layout:?}");
            for f in [
                Jump,
                Reload,
                SwitchWeapons,
                SwapGrenades,
                Flashlight,
                Zoom,
                Crouch,
            ] {
                assert!(has(f), "{layout:?} {f:?}");
            }
            // Dual wielding, something fires the left gun.
            assert!(has(LeftWeapon) || has(MeleeOrLeftWeapon), "{layout:?}");
            let table = layout.table();
            for (k, t) in table.iter().enumerate() {
                assert!(
                    !table[k + 1..].iter().any(|u| u.0 == t.0),
                    "{layout:?} maps {:?} twice",
                    t.0
                );
                assert!(!matches!(t.0, PadButton::Start | PadButton::Back));
            }
        }
    }

    #[test]
    fn bumper_jumper_jumps_and_melees_with_the_bumpers() {
        let bj = ButtonLayout::BumperJumper;
        assert_eq!(bj.function(LB), Some(Jump));
        assert_eq!(bj.function(RB), Some(Melee));
        assert_eq!(bj.function(B), Some(Reload));
        assert_eq!(bj.function(A), Some(SwapGrenades));
        assert_eq!(bj.function(X), Some(Flashlight));
        assert_eq!(bj.function(Y), Some(SwitchWeapons));
        assert_eq!(bj.function(RT), Some(RightWeapon));
        assert_eq!(bj.function(LT), Some(LeftWeapon));
        assert_eq!(bj.function(LeftStick), Some(Crouch));
        assert_eq!(bj.function(RightStick), Some(Zoom));
    }

    #[test]
    fn recon_reloads_with_rb_and_lights_with_x_or_up() {
        let recon = ButtonLayout::Recon;
        assert_eq!(recon.function(RB), Some(Reload));
        assert_eq!(recon.function(X), Some(Flashlight));
        assert_eq!(recon.function(PadButton::Up), Some(Flashlight));
        assert_eq!(recon.function(PadButton::Left), Some(SwapGrenades));
        assert_eq!(recon.function(PadButton::Right), Some(SwapGrenades));
        assert_eq!(recon.function(PadButton::Down), None);
        assert_eq!(recon.function(LB), None);
        assert_eq!(recon.function(B), Some(Melee));
        assert_eq!(recon.function(A), Some(Jump));
        // Prompts name the first button that does it.
        assert_eq!(recon.button(Flashlight), Some(X));
    }

    #[test]
    fn layouts_are_saved_by_name() {
        for l in ButtonLayout::ALL {
            assert_eq!(ButtonLayout::from_key(l.key()), Some(l));
        }
        for l in StickLayout::ALL {
            assert_eq!(StickLayout::from_key(l.key()), Some(l));
        }
        assert_eq!(
            ButtonLayout::from_key("bumper_jumper"),
            Some(ButtonLayout::BumperJumper)
        );
        assert_eq!(
            ButtonLayout::from_key("GREEN_THUMB"),
            Some(ButtonLayout::GreenThumb)
        );
        assert_eq!(ButtonLayout::from_key("jumpy"), None);
        assert_eq!(
            StickLayout::from_key("legacy_southpaw"),
            Some(StickLayout::LegacySouthpaw)
        );
        assert_eq!(StickLayout::from_key(""), None);
        assert_eq!(ButtonLayout::default(), ButtonLayout::Default);
        assert_eq!(StickLayout::default(), StickLayout::Default);
    }

    #[test]
    fn stick_layouts_move_and_look_as_halo_2_describes() {
        let (left, right) = (Vec2::new(0.5, 0.0), Vec2::new(0.0, 0.5));
        let cases = [
            (StickLayout::Default, (0.5, 0.0), (0.0, 0.5)),
            (StickLayout::Southpaw, (0.0, 0.5), (0.5, 0.0)),
            (StickLayout::Legacy, (0.0, 0.0), (0.5, 0.5)),
            (StickLayout::LegacySouthpaw, (0.5, 0.5), (0.0, 0.0)),
        ];
        for (layout, movement, look) in cases {
            let got = layout.apply(left, right);
            assert_eq!(got.0, Vec2::from(movement), "{layout:?}");
            assert_eq!(got.1, Vec2::from(look), "{layout:?}");
        }
        // Halo 2's screen: Legacy turns with the left stick and strafes
        // with the right.
        assert_eq!(
            StickLayout::Legacy.sticks(),
            ["MOVE / ROTATE", "LOOK / STRAFE"]
        );
    }

    #[test]
    fn gilrs_buttons_have_xbox_names() {
        assert_eq!(PadButton::from_gilrs(Button::LeftTrigger), Some(LB));
        assert_eq!(PadButton::from_gilrs(Button::LeftTrigger2), Some(LT));
        assert_eq!(PadButton::from_gilrs(Button::RightTrigger), Some(RB));
        assert_eq!(PadButton::from_gilrs(Button::RightTrigger2), Some(RT));
        assert_eq!(PadButton::from_gilrs(Button::Select), Some(PadButton::Back));
        assert_eq!(PadButton::from_gilrs(Button::South), Some(A));
        assert_eq!(PadButton::from_gilrs(Button::East), Some(B));
        assert_eq!(PadButton::from_gilrs(Button::West), Some(X));
        assert_eq!(PadButton::from_gilrs(Button::North), Some(Y));
        assert_eq!(PadButton::from_gilrs(Button::DPadUp), Some(PadButton::Up));
        assert_eq!(
            PadButton::from_gilrs(Button::DPadDown),
            Some(PadButton::Down)
        );
        assert_eq!(
            PadButton::from_gilrs(Button::DPadLeft),
            Some(PadButton::Left)
        );
        assert_eq!(
            PadButton::from_gilrs(Button::DPadRight),
            Some(PadButton::Right)
        );
        assert_eq!(PadButton::from_gilrs(Button::Mode), None);
        for b in PadButton::ALL {
            assert_eq!(PadButton::from_gilrs(b.to_gilrs()), Some(b));
        }
    }

    #[test]
    fn the_left_stick_is_not_the_dpad() {
        let mut dir = 0;
        assert_eq!(stick_push(&mut dir, 0.5), None);
        assert_eq!(stick_push(&mut dir, 0.7), Some(true));
        // Held there: once.
        assert_eq!(stick_push(&mut dir, 0.9), None);
        assert_eq!(stick_push(&mut dir, 0.4), None);
        assert_eq!(stick_push(&mut dir, 0.2), None);
        assert_eq!(stick_push(&mut dir, -0.7), Some(false));
        // And in a game the d-pad's functions come only from Down(Up..).
        assert_ne!(PadEvent::Stick(Dir::Up), PadEvent::Down(PadButton::Up));
    }

    #[test]
    fn trigger_presses_and_holds_agree() {
        const { assert!(TRIGGER_RELEASE < TRIGGER) };
        let reading = |held: &[PadButton]| {
            let mut r = PadReading::default();
            held.iter().for_each(|&b| r.held.insert(b));
            r
        };
        // What read() counts as held at 0.3, the layout turns into its
        // functions.
        let r = reading(&[RT, LT, PadButton::Back]);
        let s = r.state(ButtonLayout::Default, StickLayout::Default);
        assert!(s.held.contains(RightWeapon) && s.held.contains(LeftWeapon));
        assert!(s.scores);
        let s = r.state(ButtonLayout::Southpaw, StickLayout::Default);
        assert_eq!(s.held.iter().collect::<Vec<_>>(), [RightWeapon, LeftWeapon]);
        let s = reading(&[LT]).state(ButtonLayout::Boxer, StickLayout::Default);
        assert_eq!(s.held.iter().collect::<Vec<_>>(), [MeleeOrLeftWeapon]);
        assert!(!s.scores);
    }

    #[test]
    fn prompts_name_the_layouts_button() {
        let label = |l: ButtonLayout, f| l.button(f).map(PadButton::label);
        assert_eq!(label(ButtonLayout::Default, Reload), Some("X"));
        assert_eq!(label(ButtonLayout::BumperJumper, Reload), Some("B"));
        assert_eq!(label(ButtonLayout::Recon, Reload), Some("RB"));
        for l in ButtonLayout::ALL {
            assert_eq!(label(l, SwitchWeapons), Some("Y"));
        }
    }

    /// A controller's id, which gilrs hands out only for controllers it
    /// finds.
    fn pad(n: usize) -> GamepadId {
        // It is a usize inside (transmute checks that the sizes match).
        unsafe { std::mem::transmute::<usize, GamepadId>(n) }
    }

    /// A folder of its own for a test's claims.
    fn claims_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("halo2-rs-test-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_controller_plays_in_one_window() {
        let dir = claims_dir("one-window");
        let mut first = Claims::in_dir(Some(dir.clone()));
        let mut second = Claims::in_dir(Some(dir.clone()));
        assert!(!first.taken_elsewhere(pad(1)));
        assert!(first.claim(pad(1)));
        assert!(first.ours(pad(1)) && !second.ours(pad(1)));
        assert!(second.taken_elsewhere(pad(1)));
        assert!(!second.claim(pad(1)));
        // Not even while that window has focus; another pad is free.
        assert!(!second.accepts(pad(1), true));
        assert!(first.accepts(pad(1), false));
        assert!(second.accepts(pad(2), true));
        assert!(!second.accepts(pad(2), false));
        // Let go, the other window can have it.
        first.release(pad(1));
        assert!(!second.taken_elsewhere(pad(1)));
        assert!(second.claim(pad(1)));
        assert!(first.taken_elsewhere(pad(1)));
        drop(second);
        // Its file closed (the game stopped), it's free.
        assert!(!first.taken_elsewhere(pad(1)));
        let _ = std::fs::remove_dir_all(&dir);
        // Without claims (not Windows), every window takes every pad.
        let none = Claims::in_dir(None);
        assert!(none.ours(pad(3)) && none.accepts(pad(3), false));
    }

    #[test]
    fn unclaimed_controllers_work_only_the_focused_window() {
        let table = [
            // ours, elsewhere, focused: accepted
            (true, false, false, true),
            (true, false, true, true),
            (false, false, true, true),
            (false, false, false, false),
            (false, true, true, false),
            (false, true, false, false),
        ];
        for (ours, elsewhere, focused, want) in table {
            assert_eq!(
                accepts(ours, || elsewhere, focused),
                want,
                "{ours} {elsewhere} {focused}"
            );
        }
        // A window's own controllers don't ask about the others.
        assert!(accepts(true, || panic!("asked"), false));
        assert!(!accepts(false, || panic!("asked"), false));
    }

    #[test]
    fn a_copy_with_no_window_has_no_controllers() {
        let mut none = Pads::none();
        assert!(none.events().is_empty());
        assert!(none.reading(pad(0)).is_none());
        // Nor does a window before it has focus take free ones.
        assert!(!none.focused);
    }

    #[test]
    fn a_lost_controller_stays_claimed() {
        let dir = claims_dir("lost");
        let mut here = Claims::in_dir(Some(dir.clone()));
        let other = Claims::in_dir(Some(dir.clone()));
        // Player one's pad 0 and a guest's pad 1; then the guest's goes
        // (still wanted as their lost controller) and player one leaves
        // pad 0.
        let changed = here.keep(&[pad(0), pad(1)]);
        assert_eq!(
            changed,
            [(pad(0), Claim::Claimed), (pad(1), Claim::Claimed)]
        );
        assert!(here.keep(&[pad(0), pad(1)]).is_empty());
        let changed = here.keep(&[pad(1)]);
        assert_eq!(changed, [(pad(0), Claim::Released)]);
        assert!(here.ours(pad(1)) && other.taken_elsewhere(pad(1)));
        assert!(!other.taken_elsewhere(pad(0)));
        // Another window wanting it is refused, and told once.
        let mut other = other;
        assert_eq!(other.keep(&[pad(1)]), [(pad(1), Claim::Refused)]);
        assert!(other.keep(&[pad(1)]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
