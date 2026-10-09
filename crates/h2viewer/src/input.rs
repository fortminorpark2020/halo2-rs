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
//!
//! A press that works a menu, takes a controller over or skips a cutscene
//! is spent there: the button doesn't count as held in the game until
//! it's let go (`Pads::hold_off`), so B that closes the pause menu doesn't
//! also melee.

use crate::profile::Profile;
use gilrs::ev::filter::{axis_dpad_to_button, Filter};
use gilrs::ff::{BaseEffect, BaseEffectType, Effect, EffectBuilder, Repeat};
use gilrs::{Axis, Button, EventType, GamepadId, Gilrs, GilrsBuilder};
use glam::Vec2;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The sticks' dead zone, all of it (gilrs' own filters are off, so
/// XInput's, about a quarter of the way, doesn't stack on it): a fifth of
/// the way out, measured round each stick. An estimate, not Halo 2's.
const DEAD_ZONE: f32 = 0.2;
/// Triggers count as pulled past this, and let go under the second; a
/// press (a tapped shot, Boxer's melee) comes at the same pull as a hold.
/// Estimates.
const TRIGGER: f32 = 0.3;
const TRIGGER_RELEASE: f32 = 0.2;
/// The stick counts as pushed past this, and let go under the second, for
/// menus: Halo 2's push (0x7332 of 32767, from the CC0 decompilation's
/// menu input); the let go is an estimate (Halo 2's has none).
const STICK_PUSH: (f32, f32) = (0.9, 0.35);
/// A direction held in a menu moves again after the first wait, then
/// every second one: Halo 2's quarter second for both (the CC0
/// decompilation's menu input).
const MENU_REPEAT: (Duration, Duration) = (Duration::from_millis(250), Duration::from_millis(250));

/// A controller: gilrs' number for it (XInput's slot, 0 to 3, on
/// Windows), or a scripted one's (`H2_PAD_SCRIPT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PadId(pub usize);

impl From<GamepadId> for PadId {
    fn from(id: GamepadId) -> PadId {
        PadId(usize::from(id))
    }
}

impl std::fmt::Display for PadId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
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

    /// The button a test script names (`H2_PAD_SCRIPT`): its label, or
    /// LS, RS or the d-pad's direction alone.
    pub fn from_name(name: &str) -> Option<PadButton> {
        let name = name.to_ascii_uppercase();
        let short = match name.as_str() {
            "LS" => Some(PadButton::LeftStick),
            "RS" => Some(PadButton::RightStick),
            "UP" => Some(PadButton::Up),
            "DOWN" => Some(PadButton::Down),
            "LEFT" => Some(PadButton::Left),
            "RIGHT" => Some(PadButton::Right),
            _ => None,
        };
        short.or_else(|| Self::ALL.into_iter().find(|b| b.label() == name))
    }

    /// The d-pad's button that way.
    pub fn dpad(dir: Dir) -> PadButton {
        match dir {
            Dir::Up => PadButton::Up,
            Dir::Down => PadButton::Down,
            Dir::Left => PadButton::Left,
            Dir::Right => PadButton::Right,
        }
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

    pub fn remove(&mut self, b: PadButton) {
        self.0 &= !(1 << b as u16);
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Those in both.
    pub fn and(self, other: ButtonSet) -> ButtonSet {
        ButtonSet(self.0 & other.0)
    }

    /// Those not in `other`.
    pub fn without(self, other: ButtonSet) -> ButtonSet {
        ButtonSet(self.0 & !other.0)
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
    /// The d-pad or left stick held one way a while: for menus only, as
    /// another push.
    Repeat(Dir),
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

impl Dir {
    /// The way a d-pad button points.
    pub fn of_dpad(b: PadButton) -> Option<Dir> {
        match b {
            PadButton::Up => Some(Dir::Up),
            PadButton::Down => Some(Dir::Down),
            PadButton::Left => Some(Dir::Left),
            PadButton::Right => Some(Dir::Right),
            _ => None,
        }
    }
}

/// Where the controllers come from.
enum Source {
    /// Nowhere: a copy of the game with no window.
    None,
    Gilrs(Gilrs),
    /// A test's script (`H2_PAD_SCRIPT`).
    Script(Script),
}

/// What a controller did, before this window's claims and the menus'
/// stick pushes are worked out.
#[derive(Clone, Debug, PartialEq)]
enum Raw {
    /// It came (its name).
    Connected(String),
    Disconnected,
    Down(PadButton),
    /// The left stick moved: where it is now, before the dead zone.
    LeftStick(Vec2),
}

/// A controller's two motors, as Halo 2 rumbles them: the low one (the
/// left, heavy motor) and the high one (the right, light motor), 0 to 1.
struct Motors {
    id: PadId,
    level: [f32; 2],
    /// Played for good, their gain the level: none on a scripted
    /// controller, or one that can't rumble.
    effects: Option<[Effect; 2]>,
}

pub struct Pads {
    source: Source,
    /// Which way each controller's left stick last pointed, for menu
    /// presses.
    sticks: Vec<(PadId, Option<Dir>)>,
    /// A direction held on a controller in a menu, and when it moves
    /// again.
    repeats: Vec<(PadId, Dir, Instant)>,
    /// Buttons whose press was spent outside the game (on a menu, taking
    /// a controller, skipping a cutscene): not held in it until let go.
    held_off: Vec<(PadId, ButtonSet)>,
    motors: Vec<Motors>,
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
        if let Some(path) = std::env::var_os("H2_PAD_SCRIPT") {
            let path = PathBuf::from(path);
            println!("controllers: scripted, from {}", path.display());
            let script = Script::new(Some(path), log);
            return Pads::with(Source::Script(script), Claims::in_dir(None), log);
        }
        let source = match open_gilrs() {
            Ok(g) => Source::Gilrs(g),
            Err(e) => {
                println!("warning: controllers unavailable: {e}");
                Source::None
            }
        };
        if let (true, Source::Gilrs(g)) = (log, &source) {
            println!("controllers: {BACKEND} (gilrs 0.11.2)");
            let pads = list(g);
            if pads.is_empty() {
                println!("controllers: none connected");
            }
            for line in pads {
                println!("controllers: {line}");
            }
        }
        Pads::with(source, Claims::new(), log)
    }

    fn with(source: Source, claims: Claims, log: bool) -> Pads {
        Pads {
            source,
            sticks: Vec::new(),
            repeats: Vec::new(),
            held_off: Vec::new(),
            motors: Vec::new(),
            claims,
            focused: false,
            log,
        }
    }

    /// No controllers, for a copy of the game with no window: XInput
    /// would give it the player's, which are for the windows on this PC.
    pub fn none() -> Pads {
        Pads::with(Source::None, Claims::in_dir(None), false)
    }

    /// Controllers a test plays a line at a time (`Pads::script`).
    #[cfg(test)]
    pub fn scripted() -> Pads {
        let mut pads = Pads::with(
            Source::Script(Script::new(None, false)),
            Claims::in_dir(None),
            false,
        );
        pads.focused = true;
        pads
    }

    /// Run a line of a test's script now (see `Script`).
    #[cfg(test)]
    pub fn script(&mut self, line: &str) {
        if let Source::Script(s) = &mut self.source {
            s.run(line, Instant::now());
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
    pub fn keep_claimed(&mut self, wanted: &[PadId]) {
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
    pub fn events(&mut self) -> Vec<(PadId, PadEvent)> {
        let now = Instant::now();
        let raw = match &mut self.source {
            Source::None => return Vec::new(),
            Source::Gilrs(g) => gilrs_events(g),
            Source::Script(s) => s.events(now),
        };
        let mut out = Vec::new();
        for (id, raw) in raw {
            let event = match raw {
                Raw::Connected(name) => {
                    if self.log {
                        println!("controllers: pad {id} connected ({name})");
                    }
                    PadEvent::Connected
                }
                Raw::Disconnected => {
                    self.forget(id);
                    if self.log {
                        println!("controllers: pad {id} disconnected");
                    }
                    PadEvent::Disconnected
                }
                Raw::Down(b) => PadEvent::Down(b),
                Raw::LeftStick(v) => {
                    let k = match self.sticks.iter().position(|s| s.0 == id) {
                        Some(k) => k,
                        None => {
                            self.sticks.push((id, None));
                            self.sticks.len() - 1
                        }
                    };
                    match stick_dir(&mut self.sticks[k].1, v) {
                        Some(dir) => PadEvent::Stick(dir),
                        None => continue,
                    }
                }
            };
            let presses = matches!(event, PadEvent::Down(_) | PadEvent::Stick(_));
            if presses && !self.claims.accepts(id, self.focused) {
                if self.log && matches!(event, PadEvent::Down(_)) {
                    let why = if self.claims.taken_elsewhere(id) {
                        "belongs to another window"
                    } else {
                        "is free, but this window isn't in front"
                    };
                    println!("controllers: pad {id} {why}: ignored");
                }
                continue;
            }
            let dir = match event {
                PadEvent::Down(b) => Dir::of_dpad(b),
                PadEvent::Stick(d) => Some(d),
                _ => None,
            };
            if let Some(d) = dir {
                self.repeats.retain(|r| r.0 != id);
                self.repeats.push((id, d, now + MENU_REPEAT.0));
            }
            out.push((id, event));
        }
        self.repeat(now, &mut out);
        self.let_go();
        out
    }

    /// A direction held a while moves a menu again (`MENU_REPEAT`), the
    /// d-pad's or the left stick's, until it's let go.
    fn repeat(&mut self, now: Instant, out: &mut Vec<(PadId, PadEvent)>) {
        let held: Vec<bool> = (self.repeats.iter())
            .map(|&(id, dir, _)| {
                let stick = self.sticks.iter().any(|s| s.0 == id && s.1 == Some(dir));
                let dpad = self.raw(id).map(|r| r.held.contains(PadButton::dpad(dir)));
                stick || dpad == Some(true)
            })
            .collect();
        let mut held = held.into_iter();
        self.repeats.retain(|_| held.next() == Some(true));
        for (id, dir, next) in &mut self.repeats {
            if now >= *next {
                *next = now + MENU_REPEAT.1;
                out.push((*id, PadEvent::Repeat(*dir)));
            }
        }
    }

    /// Spent presses end when their buttons are let go.
    fn let_go(&mut self) {
        let held: Vec<ButtonSet> = (self.held_off.iter())
            .map(|h| self.raw(h.0).map(|r| r.held).unwrap_or_default())
            .collect();
        for (h, held) in self.held_off.iter_mut().zip(held) {
            h.1 = h.1.and(held);
        }
        self.held_off.retain(|h| !h.1.is_empty());
    }

    /// Controller `id` went: what's kept about it goes too.
    fn forget(&mut self, id: PadId) {
        self.sticks.retain(|s| s.0 != id);
        self.repeats.retain(|r| r.0 != id);
        self.held_off.retain(|h| h.0 != id);
        self.motors.retain(|m| m.id != id);
    }

    /// Controller `id`'s press of `b` was spent outside the game (on a
    /// menu, taking the controller over, skipping a cutscene): `b` isn't
    /// held in the game until it's let go, so B that closes the pause menu
    /// doesn't melee as the game comes back.
    pub fn hold_off(&mut self, id: PadId, b: PadButton) {
        if self.log {
            println!("controllers: pad {id} {} spent", b.label());
        }
        match self.held_off.iter_mut().find(|h| h.0 == id) {
            Some(h) => h.1.insert(b),
            None => {
                let mut set = ButtonSet::default();
                set.insert(b);
                self.held_off.push((id, set));
            }
        }
    }

    /// Everything held on controller `id` now is spent (`hold_off`).
    pub fn hold_off_all(&mut self, id: PadId) {
        let held = self.raw(id).map(|r| r.held).unwrap_or_default();
        for b in PadButton::ALL.into_iter().filter(|&b| held.contains(b)) {
            self.hold_off(id, b);
        }
    }

    /// Controller `id`'s sticks and buttons now, whatever's spent.
    fn raw(&self, id: PadId) -> Option<PadReading> {
        match &self.source {
            Source::None => None,
            Source::Gilrs(g) => Some(read(&g.connected_gamepad(gilrs_id(g, id)?)?)),
            Source::Script(s) => s.reading(id),
        }
    }

    /// Controller `id`'s sticks and buttons now, if it's connected and
    /// this window takes its input; spent presses aren't held.
    pub fn reading(&self, id: PadId) -> Option<PadReading> {
        if !self.claims.accepts(id, self.focused) {
            return None;
        }
        let mut r = self.raw(id)?;
        if let Some(h) = self.held_off.iter().find(|h| h.0 == id) {
            r.held = r.held.without(h.1);
        }
        Some(r)
    }

    /// Rumble controller `id`: its low (heavy) and high (light) motors, 0
    /// to 1. It keeps going until told otherwise.
    pub fn rumble(&mut self, id: PadId, low: f32, high: f32) {
        // In 64ths: not a message to the motors for every hair of change.
        let level = [low, high].map(|v| (v.clamp(0.0, 1.0) * 64.0).round() / 64.0);
        let k = match self.motors.iter().position(|m| m.id == id) {
            Some(k) => k,
            None if level == [0.0; 2] => return,
            None => {
                let effects = match &mut self.source {
                    Source::Gilrs(g) => motors(g, id),
                    _ => None,
                };
                self.motors.push(Motors {
                    id,
                    level: [0.0; 2],
                    effects,
                });
                self.motors.len() - 1
            }
        };
        let m = &mut self.motors[k];
        if m.level == level {
            return;
        }
        m.level = level;
        if let Some(effects) = &m.effects {
            for (e, v) in effects.iter().zip(level) {
                let _ = e.set_gain(v);
            }
        }
        if self.log {
            let [low, high] = level;
            println!("controllers: pad {id} rumble low {low:.2} high {high:.2}");
        }
    }

    /// What controller `id`'s motors were last told: low and high.
    #[cfg(test)]
    pub fn rumbling(&self, id: PadId) -> [f32; 2] {
        (self.motors.iter().find(|m| m.id == id)).map_or([0.0; 2], |m| m.level)
    }

    /// Every motor off.
    pub fn stop_rumble(&mut self) {
        let ids: Vec<PadId> = self.motors.iter().map(|m| m.id).collect();
        for id in ids {
            self.rumble(id, 0.0, 0.0);
        }
    }
}

/// gilrs' id for controller `id`, if it's connected.
fn gilrs_id(g: &Gilrs, id: PadId) -> Option<GamepadId> {
    g.gamepads().map(|p| p.0).find(|&g| PadId::from(g) == id)
}

/// What gilrs' controllers did since the last call.
fn gilrs_events(g: &mut Gilrs) -> Vec<(PadId, Raw)> {
    let mut out = Vec::new();
    while let Some(ev) = g.next_event() {
        // A d-pad that's a hat (on Linux) as four buttons, as gilrs' own
        // filters have it.
        let ev = ev.filter_ev(&axis_dpad_to_button, g).unwrap_or(ev);
        g.update(&ev);
        let raw = match ev.event {
            EventType::Connected => Raw::Connected(g.gamepad(ev.id).name().to_string()),
            EventType::Disconnected => Raw::Disconnected,
            EventType::ButtonPressed(b, _) => match PadButton::from_gilrs(b) {
                Some(b) => Raw::Down(b),
                None => continue,
            },
            EventType::AxisChanged(Axis::LeftStickX | Axis::LeftStickY, _, _) => {
                let pad = g.gamepad(ev.id);
                let v = Vec2::new(pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY));
                Raw::LeftStick(v)
            }
            _ => continue,
        };
        out.push((PadId::from(ev.id), raw));
    }
    out
}

/// Two effects played for good on controller `id`: the strong (low) and
/// weak (high) motor at full strength, their gain 0 until it rumbles.
fn motors(g: &mut Gilrs, id: PadId) -> Option<[Effect; 2]> {
    let gid = gilrs_id(g, id).filter(|&gid| g.gamepad(gid).is_ff_supported())?;
    let mut effect = |kind| -> Option<Effect> {
        let e = EffectBuilder::new()
            .add_effect(BaseEffect {
                kind,
                scheduling: Default::default(),
                envelope: Default::default(),
            })
            .repeat(Repeat::Infinitely)
            .gamepads(&[gid])
            .gain(0.0)
            .finish(g)
            .ok()?;
        e.play().ok()?;
        Some(e)
    };
    let low = effect(BaseEffectType::Strong {
        magnitude: u16::MAX,
    })?;
    let high = effect(BaseEffectType::Weak {
        magnitude: u16::MAX,
    })?;
    Some([low, high])
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

/// The left stick moved to `v`: Some(which way) when it was pushed that
/// way just now. `last` is the way it last pointed. Only the way it's
/// pushed furthest counts, so a push to one side a little up moves a menu
/// once; another way counts once the stick has come back from the last.
fn stick_dir(last: &mut Option<Dir>, v: Vec2) -> Option<Dir> {
    let along = |d: Dir| match d {
        Dir::Up => v.y,
        Dir::Down => -v.y,
        Dir::Left => -v.x,
        Dir::Right => v.x,
    };
    if last.is_some_and(|d| along(d) >= STICK_PUSH.1) {
        return None;
    }
    *last = None;
    let dir = if v.x.abs() > v.y.abs() {
        if v.x > 0.0 {
            Dir::Right
        } else {
            Dir::Left
        }
    } else if v.y > 0.0 {
        Dir::Up
    } else {
        Dir::Down
    };
    if along(dir) <= STICK_PUSH.0 {
        return None;
    }
    *last = Some(dir);
    Some(dir)
}

/// Controllers a test plays (`H2_PAD_SCRIPT=<file>`): the test writes
/// lines to the file as it goes, and each frame the game reads the new
/// ones, through the same events and readings as real controllers (a test
/// machine can't make a real one). A line each:
///
/// ```text
/// connect <pad>                   disconnect <pad>
/// down <pad> <button>...          up <pad> <button>...
/// press <pad> <button> [ms]       down, and up again ms later (100)
/// stick <pad> left|right <x> <y>  -1 to 1; up and right are positive
/// ```
///
/// Buttons by their labels (A, B, X, Y, LB, RB, LT, RT, BACK, START) or
/// LS, RS, UP, DOWN, LEFT and RIGHT. A controller that does something
/// before `connect` is connected first. Lines starting # are notes.
pub struct Script {
    path: Option<PathBuf>,
    /// How much of the file has been read, and a line not yet finished.
    read: u64,
    partial: String,
    pads: Vec<(PadId, ScriptPad)>,
    /// Presses to let go of, and when.
    releases: Vec<(Instant, PadId, PadButton)>,
    /// What the lines run since the last `events` did.
    raw: Vec<(PadId, Raw)>,
    log: bool,
}

/// A scripted controller now: the sticks before the dead zone.
#[derive(Clone, Copy, Debug, Default)]
struct ScriptPad {
    connected: bool,
    left: Vec2,
    right: Vec2,
    held: ButtonSet,
}

impl Script {
    fn new(path: Option<PathBuf>, log: bool) -> Script {
        Script {
            path,
            read: 0,
            partial: String::new(),
            pads: Vec::new(),
            releases: Vec::new(),
            raw: Vec::new(),
            log,
        }
    }

    /// What the script did since the last call: its new lines, and the
    /// presses due to be let go.
    fn events(&mut self, now: Instant) -> Vec<(PadId, Raw)> {
        for line in self.new_lines() {
            if self.log {
                println!("controllers: script: {line}");
            }
            self.run(&line, now);
        }
        let due: Vec<(PadId, PadButton)> = (self.releases.iter())
            .filter(|r| r.0 <= now)
            .map(|r| (r.1, r.2))
            .collect();
        self.releases.retain(|r| r.0 > now);
        for (id, b) in due {
            self.pad(id).held.remove(b);
        }
        std::mem::take(&mut self.raw)
    }

    /// The lines written to the file since the last call.
    fn new_lines(&mut self) -> Vec<String> {
        let Some(path) = &self.path else {
            return Vec::new();
        };
        let Ok(mut f) = File::open(path) else {
            return Vec::new();
        };
        let mut bytes = Vec::new();
        if f.seek(SeekFrom::Start(self.read)).is_err() || f.read_to_end(&mut bytes).is_err() {
            return Vec::new();
        }
        self.read += bytes.len() as u64;
        self.partial.push_str(&String::from_utf8_lossy(&bytes));
        let mut lines = Vec::new();
        while let Some(end) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=end).collect();
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                lines.push(line.to_string());
            }
        }
        lines
    }

    fn pad(&mut self, id: PadId) -> &mut ScriptPad {
        let k = match self.pads.iter().position(|p| p.0 == id) {
            Some(k) => k,
            None => {
                self.pads.push((id, ScriptPad::default()));
                self.pads.len() - 1
            }
        };
        &mut self.pads[k].1
    }

    fn connect(&mut self, id: PadId) {
        let p = self.pad(id);
        if !p.connected {
            p.connected = true;
            self.raw.push((id, Raw::Connected("scripted controller".into())));
        }
    }

    fn down(&mut self, id: PadId, b: PadButton) {
        self.connect(id);
        let p = self.pad(id);
        if !p.held.contains(b) {
            p.held.insert(b);
            self.raw.push((id, Raw::Down(b)));
        }
    }

    /// Run a line of the script.
    fn run(&mut self, line: &str, now: Instant) {
        let words: Vec<&str> = line.split_whitespace().collect();
        let (Some(&verb), Some(id)) = (words.first(), words.get(1)) else {
            return;
        };
        let Ok(id) = id.parse().map(PadId) else {
            println!("controllers: script: which controller in {line:?}?");
            return;
        };
        let args = &words[2..];
        let buttons: Vec<PadButton> = args.iter().filter_map(|a| PadButton::from_name(a)).collect();
        match verb {
            "connect" => self.connect(id),
            "disconnect" => {
                let p = self.pad(id);
                if p.connected {
                    *p = ScriptPad::default();
                    self.raw.push((id, Raw::Disconnected));
                }
                self.releases.retain(|r| r.1 != id);
            }
            "down" => buttons.into_iter().for_each(|b| self.down(id, b)),
            "up" => {
                let p = self.pad(id);
                buttons.into_iter().for_each(|b| p.held.remove(b));
            }
            "press" => {
                let ms = args.get(1).and_then(|ms| ms.parse().ok()).unwrap_or(100);
                if let Some(&b) = buttons.first() {
                    self.down(id, b);
                    let at = now + Duration::from_millis(ms);
                    self.releases.retain(|r| (r.1, r.2) != (id, b));
                    self.releases.push((at, id, b));
                }
            }
            "stick" => {
                let at = |k: usize| args.get(k).and_then(|v| v.parse::<f32>().ok());
                let (Some(&side), Some(x), Some(y)) = (args.first(), at(1), at(2)) else {
                    println!("controllers: script: which stick, and where, in {line:?}?");
                    return;
                };
                let v = Vec2::new(x, y).clamp(Vec2::splat(-1.0), Vec2::ONE);
                self.connect(id);
                let p = self.pad(id);
                match side {
                    "left" => {
                        p.left = v;
                        self.raw.push((id, Raw::LeftStick(v)));
                    }
                    _ => p.right = v,
                }
            }
            _ => println!("controllers: script: what is {line:?}?"),
        }
    }

    /// Controller `id` now, if it's connected.
    fn reading(&self, id: PadId) -> Option<PadReading> {
        let p = self.pads.iter().find(|p| p.0 == id && p.1.connected)?.1;
        Some(PadReading {
            left: dead_zone(p.left),
            right: dead_zone(p.right),
            held: p.held,
        })
    }
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
    held: Vec<(PadId, Option<File>)>,
    /// Wanted, but another window has them.
    refused: Vec<PadId>,
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

    fn open(&self, id: PadId) -> Option<File> {
        let path = self.dir.as_ref()?.join(format!("pad{id}.lock"));
        File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .ok()
    }

    /// Controller `id` is this window's.
    pub fn ours(&self, id: PadId) -> bool {
        self.dir.is_none() || self.held.iter().any(|h| h.0 == id)
    }

    /// Another copy of the game on this PC claimed controller `id`.
    pub fn taken_elsewhere(&self, id: PadId) -> bool {
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
    pub fn claim(&mut self, id: PadId) -> bool {
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
    pub fn release(&mut self, id: PadId) {
        for (_, file) in self.held.iter().filter(|h| h.0 == id) {
            if let Some(f) = file {
                let _ = f.unlock();
            }
        }
        self.held.retain(|h| h.0 != id);
    }

    /// Whether this window takes controller `id`'s input (see `accepts`).
    pub fn accepts(&self, id: PadId, focused: bool) -> bool {
        accepts(self.ours(id), || self.taken_elsewhere(id), focused)
    }

    /// Hold exactly the controllers in `wanted`: claim the new ones, let
    /// go of the rest. Returns what changed (a refusal once, until it's no
    /// longer wanted).
    pub fn keep(&mut self, wanted: &[PadId]) -> Vec<(PadId, Claim)> {
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
        let gone: Vec<PadId> = (self.held.iter().map(|h| h.0))
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
    use gilrs::ff::{Replay, Ticks};
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
        let mut dir = None;
        let x = |v: f32| Vec2::new(v, 0.0);
        // Halo 2's menus want it pushed nine tenths of the way.
        assert_eq!(stick_dir(&mut dir, x(0.5)), None);
        assert_eq!(stick_dir(&mut dir, x(0.85)), None);
        assert_eq!(stick_dir(&mut dir, x(0.95)), Some(Dir::Right));
        // Held there: once.
        assert_eq!(stick_dir(&mut dir, x(1.0)), None);
        assert_eq!(stick_dir(&mut dir, x(0.4)), None);
        assert_eq!(stick_dir(&mut dir, x(0.2)), None);
        assert_eq!(stick_dir(&mut dir, x(-0.95)), Some(Dir::Left));
        // And in a game the d-pad's functions come only from Down(Up..).
        assert_ne!(PadEvent::Stick(Dir::Up), PadEvent::Down(PadButton::Up));
    }

    #[test]
    fn a_diagonal_push_moves_a_menu_once() {
        let mut dir = None;
        // Mostly right, a little up: right, once, however it wobbles.
        assert_eq!(
            stick_dir(&mut dir, Vec2::new(0.95, 0.6)),
            Some(Dir::Right)
        );
        assert_eq!(stick_dir(&mut dir, Vec2::new(0.6, 0.95)), None);
        assert_eq!(stick_dir(&mut dir, Vec2::new(0.7, 0.7)), None);
        // Swung round to straight up: up, once the right has let go.
        assert_eq!(stick_dir(&mut dir, Vec2::new(0.2, 0.95)), Some(Dir::Up));
        assert_eq!(stick_dir(&mut dir, Vec2::ZERO), None);
        assert_eq!(dir, None);
        assert_eq!(
            stick_dir(&mut dir, Vec2::new(-0.5, -0.95)),
            Some(Dir::Down)
        );
    }

    #[test]
    fn a_held_direction_repeats_in_menus() {
        let mut pads = Pads::scripted();
        pads.script("down 0 DOWN");
        let events = pads.events();
        assert!(events.contains(&(pad(0), PadEvent::Connected)));
        assert!(events.contains(&(pad(0), PadEvent::Down(PadButton::Down))));
        // Not at once...
        assert!(pads.events().is_empty());
        // ...but once it's been held a while, and on until it's let go.
        let past = Instant::now() - MENU_REPEAT.0;
        pads.repeats.iter_mut().for_each(|r| r.2 = past);
        assert_eq!(pads.events(), [(pad(0), PadEvent::Repeat(Dir::Down))]);
        pads.repeats.iter_mut().for_each(|r| r.2 = past);
        assert_eq!(pads.events(), [(pad(0), PadEvent::Repeat(Dir::Down))]);
        pads.script("up 0 DOWN");
        assert!(pads.events().is_empty());
        assert!(pads.repeats.is_empty());
        // The left stick too.
        pads.script("stick 0 left 0 1");
        assert_eq!(pads.events(), [(pad(0), PadEvent::Stick(Dir::Up))]);
        pads.repeats.iter_mut().for_each(|r| r.2 = past);
        assert_eq!(pads.events(), [(pad(0), PadEvent::Repeat(Dir::Up))]);
        pads.script("stick 0 left 0 0");
        assert!(pads.events().is_empty());
        assert!(pads.repeats.is_empty());
    }

    #[test]
    fn a_spent_press_is_not_held_until_let_go() {
        let mut pads = Pads::scripted();
        pads.script("down 0 B RT");
        pads.events();
        // B closed a menu: it's not held in the game; the trigger is.
        pads.hold_off(pad(0), B);
        let held = pads.reading(pad(0)).unwrap().held;
        assert!(!held.contains(B) && held.contains(RT));
        // Still held down, still spent.
        pads.events();
        assert!(!pads.reading(pad(0)).unwrap().held.contains(B));
        // Let go and pressed again, it counts.
        pads.script("up 0 B");
        pads.events();
        pads.script("down 0 B");
        assert_eq!(pads.events(), [(pad(0), PadEvent::Down(B))]);
        assert!(pads.reading(pad(0)).unwrap().held.contains(B));
        // Everything held at once.
        pads.hold_off_all(pad(0));
        assert_eq!(pads.reading(pad(0)).unwrap().held, ButtonSet::default());
    }

    #[test]
    fn a_script_plays_a_controller() {
        let dir = claims_dir("script");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pads.txt");
        std::fs::write(&path, "# a note\nconnect 1\npress 1 A 0\nstick 1 right 0.5 -1").unwrap();
        let mut pads = Pads::with(
            Source::Script(Script::new(Some(path.clone()), false)),
            Claims::in_dir(None),
            false,
        );
        let events = pads.events();
        assert_eq!(
            events,
            [
                (pad(1), PadEvent::Connected),
                (pad(1), PadEvent::Down(A))
            ]
        );
        // The unfinished last line waits for its end.
        let r = pads.reading(pad(1)).unwrap();
        assert_eq!(r.right, Vec2::ZERO);
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "\nstick 1 left -1 0\ndisconnect 1").unwrap();
        let events = pads.events();
        assert_eq!(
            events,
            [
                (pad(1), PadEvent::Stick(Dir::Left)),
                (pad(1), PadEvent::Disconnected)
            ]
        );
        assert!(pads.reading(pad(1)).is_none());
        // A press reconnects it.
        writeln!(f, "down 1 lt\nstick 1 right 0.5 -1").unwrap();
        pads.events();
        let r = pads.reading(pad(1)).unwrap();
        assert!(r.held.contains(LT) && !r.held.contains(A));
        assert!(r.right.x > 0.0 && r.right.y < -0.5, "{}", r.right);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scripts_name_buttons_by_their_labels() {
        for b in PadButton::ALL {
            assert_eq!(PadButton::from_name(b.label()), Some(b), "{b:?}");
        }
        assert_eq!(PadButton::from_name("ls"), Some(LeftStick));
        assert_eq!(PadButton::from_name("start"), Some(PadButton::Start));
        assert_eq!(PadButton::from_name("up"), Some(PadButton::Up));
        assert_eq!(PadButton::from_name("Z"), None);
    }

    #[test]
    fn rumble_goes_to_the_motors_in_steps() {
        let mut pads = Pads::scripted();
        pads.script("connect 2");
        pads.events();
        assert_eq!(pads.rumbling(pad(2)), [0.0, 0.0]);
        pads.rumble(pad(2), 0.5, 2.0);
        assert_eq!(pads.rumbling(pad(2)), [0.5, 1.0]);
        // In 64ths.
        pads.rumble(pad(2), 0.501, 0.0);
        assert_eq!(pads.rumbling(pad(2)), [0.5, 0.0]);
        pads.stop_rumble();
        assert_eq!(pads.rumbling(pad(2)), [0.0, 0.0]);
        // Gone with the controller.
        pads.rumble(pad(2), 1.0, 1.0);
        pads.script("disconnect 2");
        pads.events();
        assert_eq!(pads.rumbling(pad(2)), [0.0, 0.0]);
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

    fn pad(n: usize) -> PadId {
        PadId(n)
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
