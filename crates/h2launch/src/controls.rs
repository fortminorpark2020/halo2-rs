//! The player's controls: Halo 2's button layouts as the engine's gamepad
//! mapping (host slot 116), its thumbstick layouts as a remap of the stick
//! values in the input state, keyboard and mouse bindings for the
//! profile's table at 0x42C, and the other controller settings the
//! profile holds. The lobby keeps them in `lobby.txt` and hands them to
//! every engine it starts as flags (`ControlFlags`); `--offline` and the
//! other direct runs read `lobby.txt` too.
//!
//! The 66 game actions and the 16 pad buttons are libmcc's numbering
//! (host-interface.md 8.4 and 8.5). Which button does what in each layout
//! comes from Halo 2's own BUTTON LAYOUT screen and the Xbox
//! decompilation (Default, Southpaw, Boxer, Green Thumb), MCC's Halo 2
//! column of its Recon layout, and Halo 3's Bumper Jumper
//! (docs/notes/controller-plan.md, docs/notes/decomp/controller.md and
//! docs/notes/launcher/README.md "Controls"). Comments mark the estimates.

use crate::profile::GAMEPAD_MAPPING_SIZE;

/// How many game actions the mapping and the keyboard table hold.
pub const ACTIONS: usize = GAMEPAD_MAPPING_SIZE;

/// The game actions (libmcc's names; where MCC's own settings file names an
/// index differently, the comment says so).
pub mod action {
    pub const JUMP: usize = 0;
    pub const SWITCH_GRENADE: usize = 1;
    /// Hold to pick up a gun, get in a vehicle, take a flag.
    pub const ACTION: usize = 2;
    pub const RELOAD: usize = 3;
    pub const SWITCH_WEAPON: usize = 4;
    pub const MELEE: usize = 5;
    /// The Arbiter's active camouflage.
    pub const FLASHLIGHT: usize = 6;
    pub const THROW_GRENADE: usize = 7;
    pub const FIRE: usize = 8;
    pub const CROUCH: usize = 9;
    pub const ZOOM: usize = 10;
    pub const ZOOM_IN: usize = 11;
    pub const ZOOM_OUT: usize = 12;
    /// libmcc: swap weapon. MCC's settings file: Dual-Wield (hold Y by a
    /// one-handed gun).
    pub const SWAP_WEAPON: usize = 13;
    pub const SPRINT: usize = 14;
    pub const BANSHEE_BOMB: usize = 15;
    pub const MOVE_FORWARD: usize = 16;
    pub const MOVE_BACK: usize = 17;
    pub const STRAFE_LEFT: usize = 18;
    pub const STRAFE_RIGHT: usize = 19;
    pub const SHOW_SCORES: usize = 20;
    /// MCC: Vehicle Function 2 ("braking, hovering").
    pub const VEHICLE_TRICK_PRIMARY: usize = 21;
    /// MCC: Vehicle Function 3 ("aerial maneuvers").
    pub const VEHICLE_TRICK_SECONDARY: usize = 22;
    /// libmcc: secondary fire. MCC: Vehicle Function 1 (boost, e-brake).
    pub const SECONDARY_FIRE: usize = 24;
    /// libmcc: dual wield. MCC: Fire Secondary (the left hand's gun).
    pub const DUAL_WIELD: usize = 49;
    pub const RELOAD_SECONDARY: usize = 55;
    pub const PREVIOUS_GRENADE: usize = 56;
    pub const FLASHLIGHT_ALT: usize = 64;
    pub const NEXT_GRENADE: usize = 65;
}

/// Every action's name, for the log.
pub fn action_name(i: usize) -> &'static str {
    match i {
        0 => "jump",
        1 => "switch grenade",
        2 => "action",
        3 => "reload",
        4 => "switch weapon",
        5 => "melee",
        6 => "flashlight",
        7 => "throw grenade",
        8 => "fire",
        9 => "crouch",
        10 => "zoom",
        11 => "zoom in",
        12 => "zoom out",
        13 => "swap weapon/dual-wield",
        14 => "sprint",
        15 => "banshee bomb",
        16 => "move forward",
        17 => "move back",
        18 => "strafe left",
        19 => "strafe right",
        20 => "show scores",
        21 => "vehicle trick 1",
        22 => "vehicle trick 2",
        23 => "equipment",
        24 => "secondary fire/vehicle function",
        25..=35 | 51..=54 => "forge",
        36..=48 => "theater",
        49 => "dual wield/fire secondary",
        50 => "theater zoom",
        55 => "reload secondary",
        56 => "previous grenade",
        57 => "special action",
        58 => "loadout menu",
        59 => "activate waypoint",
        60 => "activate waypoint alt",
        61 => "ping navpoints",
        62 => "raise hornet",
        63 => "lower hornet",
        64 => "flashlight alt",
        65 => "next grenade",
        _ => "?",
    }
}

/// The pad buttons a mapping entry names (0xFF = none).
pub mod button {
    pub const LT: u8 = 0;
    pub const RT: u8 = 1;
    pub const DPAD_UP: u8 = 2;
    pub const DPAD_DOWN: u8 = 3;
    pub const DPAD_LEFT: u8 = 4;
    pub const DPAD_RIGHT: u8 = 5;
    pub const START: u8 = 6;
    pub const BACK: u8 = 7;
    /// Left stick click.
    pub const LS: u8 = 8;
    /// Right stick click.
    pub const RS: u8 = 9;
    pub const LB: u8 = 10;
    pub const RB: u8 = 11;
    pub const A: u8 = 12;
    pub const B: u8 = 13;
    pub const X: u8 = 14;
    pub const Y: u8 = 15;
    pub const NONE: u8 = 0xFF;
}

/// A pad button's name as the controller labels it.
pub fn button_name(b: u8) -> &'static str {
    match b {
        button::LT => "LT",
        button::RT => "RT",
        button::DPAD_UP => "D-pad up",
        button::DPAD_DOWN => "D-pad down",
        button::DPAD_LEFT => "D-pad left",
        button::DPAD_RIGHT => "D-pad right",
        button::START => "Start",
        button::BACK => "Back",
        button::LS => "Left stick click",
        button::RS => "Right stick click",
        button::LB => "LB",
        button::RB => "RB",
        button::A => "A",
        button::B => "B",
        button::X => "X",
        button::Y => "Y",
        _ => "none",
    }
}

/// The order the lobby lists buttons in (Start, the pause, is in no
/// mapping: the lobby adds it).
pub const BUTTON_ORDER: [u8; 16] = [
    button::RT,
    button::LT,
    button::RB,
    button::LB,
    button::A,
    button::B,
    button::X,
    button::Y,
    button::LS,
    button::RS,
    button::DPAD_UP,
    button::DPAD_DOWN,
    button::DPAD_LEFT,
    button::DPAD_RIGHT,
    button::BACK,
    button::START,
];

// ---------------------------------------------------------------- button layouts

/// A BUTTON LAYOUT: Halo 2's four, Halo 3's Bumper Jumper and MCC's Recon.
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

/// Lower case, with `-`, spaces and `_` dropped, for matching names.
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '-' | '_' | ' '))
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

impl ButtonLayout {
    pub const ALL: [ButtonLayout; 6] = [
        ButtonLayout::Default,
        ButtonLayout::Southpaw,
        ButtonLayout::Boxer,
        ButtonLayout::GreenThumb,
        ButtonLayout::BumperJumper,
        ButtonLayout::Recon,
    ];

    /// How `lobby.txt` and the flags write it.
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

    /// The key or the name, ignoring case, spaces, `-` and `_`
    /// ("green thumbs", MCC's spelling, too).
    pub fn from_key(s: &str) -> Option<ButtonLayout> {
        let s = squash(s);
        let s = if s == "greenthumbs" {
            "greenthumb".into()
        } else {
            s
        };
        Self::ALL.into_iter().find(|l| squash(l.key()) == s)
    }

    /// Its name, as Halo 2 (and for the last two, Halo 3 and MCC) call it.
    pub fn name(self) -> &'static str {
        match self {
            ButtonLayout::Default => "Default",
            ButtonLayout::Southpaw => "Southpaw",
            ButtonLayout::Boxer => "Boxer",
            ButtonLayout::GreenThumb => "Green Thumb",
            ButtonLayout::BumperJumper => "Bumper Jumper",
            ButtonLayout::Recon => "Recon",
        }
    }

    /// What it is for, in a line.
    pub fn help(self) -> &'static str {
        match self {
            ButtonLayout::Default => "Halo 2's standard layout.",
            ButtonLayout::Southpaw => {
                "The default with the triggers swapped, for left-handed players."
            }
            ButtonLayout::Boxer => "Melee on the left trigger, grenades on B.",
            ButtonLayout::GreenThumb => "Melee on a click of the right stick, zoom on B.",
            ButtonLayout::BumperJumper => {
                "Jump on LB and melee on RB, so your thumb stays on the right stick (Halo 3's)."
            }
            ButtonLayout::Recon => {
                "MCC's default: reload on RB, flashlight on X, grenades on the d-pad."
            }
        }
    }

    pub fn next(self, forward: bool) -> ButtonLayout {
        step_in(&Self::ALL, self, forward)
    }

    /// The engine's gamepad mapping: for each of the 66 actions, the
    /// button that does it (0xFF: none). Start (pause) and the menus'
    /// buttons are not in it; the engine reads those from the raw buttons.
    pub fn mapping(self) -> [u8; ACTIONS] {
        use action::*;
        use button::*;
        let mut m = [NONE; ACTIONS];
        // Halo 2's Default. Two actions share a button where the engine
        // tells them apart by context or by holding: action and reload
        // (hold X to pick up), switch weapon and dual wield (hold Y).
        // 24 and 49 share the trigger that throws grenades in every layout
        // but Boxer (and 4 and 13 always share the switch weapon button),
        // so the table is right whether the engine follows libmcc's names
        // for them or MCC's (secondary fire = vehicle function 1, dual
        // wield = fire secondary).
        m[JUMP] = A;
        m[SWITCH_GRENADE] = RB;
        m[ACTION] = X;
        m[RELOAD] = X;
        m[SWITCH_WEAPON] = Y;
        m[SWAP_WEAPON] = Y;
        m[MELEE] = B;
        m[FLASHLIGHT] = LB;
        m[THROW_GRENADE] = LT;
        m[FIRE] = RT;
        m[CROUCH] = LS;
        m[ZOOM] = RS;
        m[SHOW_SCORES] = BACK;
        m[SECONDARY_FIRE] = LT;
        m[DUAL_WIELD] = LT;
        // Xbox Halo 2 has two more actions fixed to A and B in every
        // layout; MCC's Halo 2 puts its Banshee bomb on B and vehicle
        // functions 2 and 3 on A, so they are taken to be the same
        // (medium confidence).
        m[BANSHEE_BOMB] = B;
        m[VEHICLE_TRICK_PRIMARY] = A;
        m[VEHICLE_TRICK_SECONDARY] = A;
        // Halo 2 reloads both guns with one press; on the reload button
        // in case the engine reads it (an estimate, in every layout).
        m[RELOAD_SECONDARY] = X;
        match self {
            ButtonLayout::Default => {}
            ButtonLayout::Southpaw => {
                m[THROW_GRENADE] = RT;
                m[SECONDARY_FIRE] = RT;
                m[DUAL_WIELD] = RT;
                m[FIRE] = LT;
            }
            ButtonLayout::Boxer => {
                // The left trigger melees, and fires the left gun while
                // dual wielding (medium-high confidence: Vista's BUTTON
                // LAYOUT pane says "Melee/Use Left Weapon"). B throws
                // grenades, and boosts and brakes a vehicle: our notes
                // follow Halopedia there (controller-plan.md 2.3, "For
                // vehicles, follow Halopedia: B is boost / e-brake", as
                // the from-scratch game did); medium confidence, since the
                // Vista pane's "Use Left Weapon" on LT may take the boost
                // with it. So 24 (MCC: vehicle function 1) goes to B and
                // 49 (MCC: fire secondary) stays on LT.
                m[MELEE] = LT;
                m[THROW_GRENADE] = B;
                m[SECONDARY_FIRE] = B;
            }
            ButtonLayout::GreenThumb => {
                m[MELEE] = RS;
                m[ZOOM] = B;
            }
            ButtonLayout::BumperJumper => {
                // Not a Halo 2 layout: Halo 3's, as the from-scratch game
                // has it. Jump, melee, reload/action on B: high
                // confidence. Grenades on A: medium-high. Flashlight on X:
                // medium. The bomb and vehicle functions follow melee and
                // jump to the bumpers: an estimate. The d-pad keeps what
                // MCC's Recon puts there (the other flashlight button on
                // up, the previous grenade on left), as MCC's universal
                // Bumper Jumper is Recon with jump and melee on the
                // bumpers (controller-plan.md 3): an estimate.
                m[JUMP] = LB;
                m[MELEE] = RB;
                m[ACTION] = B;
                m[RELOAD] = B;
                m[RELOAD_SECONDARY] = B;
                m[SWITCH_GRENADE] = A;
                m[FLASHLIGHT] = X;
                m[FLASHLIGHT_ALT] = DPAD_UP;
                m[PREVIOUS_GRENADE] = DPAD_LEFT;
                m[BANSHEE_BOMB] = RB;
                m[VEHICLE_TRICK_PRIMARY] = LB;
                m[VEHICLE_TRICK_SECONDARY] = LB;
            }
            ButtonLayout::Recon => {
                // MCC's Halo 2 column of Recon (high confidence).
                m[ACTION] = RB;
                m[RELOAD] = RB;
                m[RELOAD_SECONDARY] = RB;
                m[SWITCH_GRENADE] = DPAD_RIGHT;
                m[PREVIOUS_GRENADE] = DPAD_LEFT;
                m[FLASHLIGHT] = DPAD_UP;
                m[FLASHLIGHT_ALT] = X;
            }
        }
        // MCC's Recon puts "Switch Grenades" (1) and "Select Next
        // Grenades" (65) on the same button, so every layout does: the
        // grenade button works whichever of the two the engine reads.
        m[NEXT_GRENADE] = m[SWITCH_GRENADE];
        m
    }

    /// What each button does, for the lobby: the buttons in
    /// `BUTTON_ORDER` that do something, with the main actions on them, in
    /// Halo 2's own words where its BUTTON LAYOUT screen has them
    /// ("Fire" stays plainer than its "Use Right Weapon"), then Start.
    pub fn bindings(self) -> Vec<(u8, String)> {
        use action::*;
        // The actions worth naming, in the order they are named; the
        // others (vehicle functions, the bomb, the second reload) ride on
        // these buttons.
        const SHOWN: [(usize, &str); 17] = [
            (FIRE, "Fire"),
            (JUMP, "Jump"),
            (MELEE, "Melee attack"),
            (THROW_GRENADE, "Throw grenade"),
            (DUAL_WIELD, "Left weapon"),
            (RELOAD, "Reload"),
            (ACTION, "Action"),
            (SWITCH_WEAPON, "Switch weapons"),
            (SWAP_WEAPON, "Dual wield"),
            (SWITCH_GRENADE, "Swap grenades"),
            (NEXT_GRENADE, "Swap grenades"),
            (PREVIOUS_GRENADE, "Previous grenade"),
            (FLASHLIGHT, "Flashlight"),
            (FLASHLIGHT_ALT, "Flashlight"),
            (ZOOM, "Zoom view"),
            (CROUCH, "Crouch"),
            (SHOW_SCORES, "Multiplayer score"),
        ];
        let m = self.mapping();
        let mut out = Vec::new();
        for b in BUTTON_ORDER {
            if b == button::START {
                // The engine reads Start itself, in every layout.
                out.push((b, "Pause game".to_string()));
                continue;
            }
            let mut labels: Vec<&str> = Vec::new();
            for (a, label) in SHOWN {
                if m[a] == b && !labels.contains(&label) {
                    labels.push(label);
                }
            }
            if !labels.is_empty() {
                out.push((b, labels.join(" / ")));
            }
        }
        out
    }
}

fn step_in<T: Copy + PartialEq>(all: &[T], at: T, forward: bool) -> T {
    let n = all.len();
    let i = all.iter().position(|&x| x == at).unwrap_or(0);
    all[if forward {
        (i + 1) % n
    } else {
        (i + n - 1) % n
    }]
}

/// The mapping `--pad-map` hands the engine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadMap {
    /// The button layout's (the default).
    #[default]
    Layout,
    /// All zero, what HaloX returns: every action on LT. For diagnosis.
    Zero,
}

impl PadMap {
    pub fn mapping(self, layout: ButtonLayout) -> [u8; ACTIONS] {
        match self {
            PadMap::Layout => layout.mapping(),
            PadMap::Zero => [0; ACTIONS],
        }
    }
}

/// A mapping for the log: `jump=A fire=RT ...`, the actions with a button.
pub fn describe_mapping(m: &[u8; ACTIONS]) -> String {
    let mut parts = Vec::new();
    for (i, &b) in m.iter().enumerate() {
        if b != button::NONE {
            parts.push(format!("{i}:{}={}", action_name(i), button_name(b)));
        }
    }
    if parts.is_empty() {
        "nothing mapped".into()
    } else {
        parts.join(" ")
    }
}

// ---------------------------------------------------------------- thumbstick layouts

/// A THUMBSTICK LAYOUT: which stick moves and which looks.
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

    pub fn key(self) -> &'static str {
        match self {
            StickLayout::Default => "default",
            StickLayout::Southpaw => "southpaw",
            StickLayout::Legacy => "legacy",
            StickLayout::LegacySouthpaw => "legacy_southpaw",
        }
    }

    pub fn from_key(s: &str) -> Option<StickLayout> {
        let s = squash(s);
        Self::ALL.into_iter().find(|l| squash(l.key()) == s)
    }

    pub fn name(self) -> &'static str {
        match self {
            StickLayout::Default => "Default",
            StickLayout::Southpaw => "Southpaw",
            StickLayout::Legacy => "Legacy",
            StickLayout::LegacySouthpaw => "Legacy Southpaw",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            StickLayout::Default => "Move with the left stick, look with the right.",
            StickLayout::Southpaw => "The sticks swapped: look with the left, move with the right.",
            StickLayout::Legacy => {
                "Older console shooters' layout: the left stick moves and turns, the right looks and strafes."
            }
            StickLayout::LegacySouthpaw => "Legacy with the sticks swapped.",
        }
    }

    /// What the left and the right stick do (up and down / left and
    /// right), as Halo 2's THUMBSTICK LAYOUT screen puts it ("Rotate
    /// Left/Right" for turning).
    pub fn sticks(self) -> [&'static str; 2] {
        match self {
            StickLayout::Default => ["Move / strafe", "Look / rotate"],
            StickLayout::Southpaw => ["Look / rotate", "Move / strafe"],
            StickLayout::Legacy => ["Move / rotate", "Look / strafe"],
            StickLayout::LegacySouthpaw => ["Look / strafe", "Move / rotate"],
        }
    }

    pub fn next(self, forward: bool) -> StickLayout {
        step_in(&Self::ALL, self, forward)
    }

    /// The sticks the engine is given (left = move, right = look) from the
    /// controller's left and right sticks, XInput's raw values (+Y up).
    /// Halo 2's own code only moves axes between the sticks (none is
    /// negated). The Legacy layouts first snap each stick (`legacy_snap`).
    /// Halo 2 snaps the values its square map gives (with no dead zone
    /// before the snap); the launcher snaps the raw, round range, which
    /// the engine square-maps later, so the snap's angles are measured on
    /// a slightly different shape: an estimate. A stick inside XInput's
    /// dead zone on both axes is left as it is, so the snap can't push a
    /// resting stick past the engine's dead zone.
    pub fn apply(self, left: (i16, i16), right: (i16, i16)) -> ((i16, i16), (i16, i16)) {
        let snap_both = || {
            (
                snap(left, LEGACY_ZONES[0], XINPUT_DEAD_ZONES[0]),
                snap(right, LEGACY_ZONES[1], XINPUT_DEAD_ZONES[1]),
            )
        };
        match self {
            StickLayout::Default => (left, right),
            StickLayout::Southpaw => (right, left),
            StickLayout::Legacy => {
                let (l, r) = snap_both();
                ((r.0, l.1), (l.0, r.1))
            }
            StickLayout::LegacySouthpaw => {
                let (l, r) = snap_both();
                ((l.0, r.1), (r.0, l.1))
            }
        }
    }
}

/// How far either side of a diagonal the Legacy layouts let a stick mix
/// its two axes, the left stick's and the right's, in radians: 35 and 10
/// degrees (the Xbox decompilation's stick conditioning).
const LEGACY_ZONES: [f32; 2] = [0.610_865_2, 0.174_532_9];

/// XInput's suggested dead zones for the left and the right stick.
pub const XINPUT_DEAD_ZONES: [i16; 2] = [7849, 8689];

fn snap(v: (i16, i16), zone: f32, dead_zone: i16) -> (i16, i16) {
    // Resting (inside the dead zone on both axes): left alone, since the
    // snap would make the stronger axis up to 1.41 times bigger.
    let rest = |a: i16| (a as i32).abs() <= dead_zone as i32;
    if rest(v.0) && rest(v.1) {
        return v;
    }
    let f = |a: i16| (a as f32 / 32767.0).clamp(-1.0, 1.0);
    let (x, y) = legacy_snap(f(v.0), f(v.1), zone);
    let i = |a: f32| (a * 32767.0).round().clamp(-32767.0, 32767.0) as i16;
    (i(x), i(y))
}

/// A stick under the Legacy layouts, as Halo 2 snaps it after its square
/// map (re-implemented from the decompilation, as the from-scratch game
/// did): within `zone`
/// of a diagonal, the nearer axis gets the stick's whole push and the
/// other less the further it is from the diagonal (all of it on the
/// diagonal, none 35 degrees off); outside, the nearer axis gets it all.
pub fn legacy_snap(x: f32, y: f32, zone: f32) -> (f32, f32) {
    use std::f32::consts::{FRAC_PI_4, PI};
    if x == 0.0 && y == 0.0 {
        return (0.0, 0.0);
    }
    let sign = |a: f32| if a < 0.0 { -1.0 } else { 1.0 };
    let angle = y.atan2(x);
    // The diagonal of the quarter it's in.
    let diagonal = match (x < 0.0, y < 0.0) {
        (false, false) => FRAC_PI_4,
        (true, false) => 3.0 * FRAC_PI_4,
        (false, true) => -FRAC_PI_4,
        (true, true) => -3.0 * FRAC_PI_4,
    };
    let off = (angle - diagonal).abs();
    let push = (x * x + y * y).sqrt();
    let across = angle.abs() < FRAC_PI_4 || angle.abs() > PI - FRAC_PI_4;
    let (sx, sy) = if off < zone {
        // Halo 2's 1.6370222: none of the other axis 35 degrees off.
        let less = (1.0 - off * 1.637_022_3) * push;
        if across {
            (sign(x) * push, sign(y) * less)
        } else {
            (sign(x) * less, sign(y) * push)
        }
    } else if x.abs() > y.abs() {
        (sign(x) * push, 0.0)
    } else {
        (0.0, sign(y) * push)
    };
    (sx.clamp(-1.0, 1.0), sy.clamp(-1.0, 1.0))
}

// ---------------------------------------------------------------- keyboard and mouse

/// Windows virtual-key codes the table uses.
mod vk {
    pub const LBUTTON: i32 = 0x01;
    pub const RBUTTON: i32 = 0x02;
    pub const TAB: i32 = 0x09;
    pub const SPACE: i32 = 0x20;
    pub const LCONTROL: i32 = 0xA2;
    pub const fn key(c: u8) -> i32 {
        c as i32
    }
}

/// Keys an action can have.
pub const KEYS_PER_ACTION: usize = 5;

/// The keyboard and mouse bindings for the profile's table at 0x42C: entry
/// `i` is action `i` with up to five Windows virtual-key codes (0 =
/// unused), as MCC's settings file writes every entry.
///
/// halo2.dll has keys of its own: HaloX's reading of it (reference only)
/// found a binding table the engine fills itself, with Space jump, Left
/// Ctrl crouch, G throw grenade, Tab switch weapon (and the scores), Q
/// melee, E action, the left button fire and the right button zoom, and
/// W A S D, R (reload), F (flashlight) and G checked by key code in its
/// player input code. So the table gives those keys the same actions, and
/// never gives one of them another action (F does only the flashlight),
/// whether or not the engine reads the table. The rest are MCC's Halo 2
/// defaults (1, 2, 4, C, and the right button for the left gun), which
/// only work if the engine reads this table: not known yet, so every
/// value is an estimate until the owner's PC shows it.
pub fn keyboard_table() -> [[i32; KEYS_PER_ACTION]; ACTIONS] {
    use action::*;
    use vk::*;
    let mut t = [[0; KEYS_PER_ACTION]; ACTIONS];
    let mut bind = |a: usize, keys: &[i32]| {
        for (slot, &k) in t[a].iter_mut().zip(keys) {
            *slot = k;
        }
    };
    // The engine's own keys.
    bind(MOVE_FORWARD, &[key(b'W')]);
    bind(MOVE_BACK, &[key(b'S')]);
    bind(STRAFE_LEFT, &[key(b'A')]);
    bind(STRAFE_RIGHT, &[key(b'D')]);
    bind(JUMP, &[SPACE]);
    bind(CROUCH, &[LCONTROL]);
    bind(FIRE, &[LBUTTON]);
    bind(ZOOM, &[RBUTTON]);
    bind(MELEE, &[key(b'Q')]);
    bind(BANSHEE_BOMB, &[key(b'Q')]);
    bind(RELOAD, &[key(b'R')]);
    bind(RELOAD_SECONDARY, &[key(b'R')]);
    bind(THROW_GRENADE, &[key(b'G')]);
    bind(ACTION, &[key(b'E')]);
    bind(SHOW_SCORES, &[TAB]);
    // The engine's key first, then MCC's.
    bind(SWITCH_WEAPON, &[TAB, key(b'1')]);
    bind(FLASHLIGHT, &[key(b'F'), key(b'4')]);
    // MCC's only (unconfirmed). The left gun (and, under the other
    // naming, vehicle function 1) on the right button: zoom does nothing
    // while dual wielding. Grenades swap on 2 by both of their actions,
    // as the button layouts do.
    bind(DUAL_WIELD, &[RBUTTON]);
    bind(SECONDARY_FIRE, &[RBUTTON]);
    bind(SWITCH_GRENADE, &[key(b'2')]);
    bind(NEXT_GRENADE, &[key(b'2')]);
    bind(SWAP_WEAPON, &[key(b'C')]);
    bind(VEHICLE_TRICK_PRIMARY, &[LCONTROL]);
    bind(VEHICLE_TRICK_SECONDARY, &[SPACE]);
    t
}

/// The bytes of one table entry: the action, then its five keys, each a
/// little-endian i32 (0x18 bytes).
pub const KEY_ENTRY_SIZE: usize = 4 + 4 * KEYS_PER_ACTION;

/// The table as the profile holds it, 66 entries of 0x18 bytes.
pub fn encode_keyboard_table(t: &[[i32; KEYS_PER_ACTION]; ACTIONS]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ACTIONS * KEY_ENTRY_SIZE);
    for (i, keys) in t.iter().enumerate() {
        out.extend_from_slice(&(i as i32).to_le_bytes());
        for k in keys {
            out.extend_from_slice(&k.to_le_bytes());
        }
    }
    out
}

/// The keyboard and mouse bindings in words, for the lobby: what
/// `keyboard_table` binds (and the mouse's look, which isn't in it). The
/// last field is true for halo2.dll's own keys (as HaloX read them; not
/// yet seen in this launcher), false for MCC's, which work only if the
/// engine reads the profile's table.
pub const KEYBOARD_BINDINGS: [(&str, &str, bool); 17] = [
    ("W A S D", "Move", true),
    ("Mouse", "Look", true),
    ("Left button", "Fire", true),
    ("Right button", "Zoom view", true),
    ("Space", "Jump", true),
    ("Left Ctrl", "Crouch", true),
    ("G", "Throw grenade", true),
    ("F", "Flashlight", true),
    ("Q", "Melee attack", true),
    ("R", "Reload", true),
    ("E", "Action", true),
    ("Tab", "Switch weapons / scores", true),
    ("Right button", "Left weapon", false),
    ("1", "Switch weapons", false),
    ("2", "Swap grenades", false),
    ("4", "Flashlight", false),
    ("C", "Dual wield", false),
];

// ---------------------------------------------------------------- the settings

/// Look sensitivity: Halo 2's 1 to 10.
pub const LOOK_SENSITIVITY_MIN: u8 = 1;
pub const LOOK_SENSITIVITY_MAX: u8 = 10;
/// Halo 2's default.
pub const LOOK_SENSITIVITY_DEFAULT: u8 = 3;
/// Mouse sensitivity: MCC's scale. 1.6 is HaloX's default and the value
/// MCC's settings file held for a game that had been played: an estimate.
pub const MOUSE_SENSITIVITY_DEFAULT: f32 = 1.6;
pub const MOUSE_SENSITIVITY_MIN: f32 = 0.1;
pub const MOUSE_SENSITIVITY_MAX: f32 = 10.0;
/// The lobby's step.
pub const MOUSE_SENSITIVITY_STEP: f32 = 0.1;

/// Halo 2's label for a look sensitivity.
pub fn look_sensitivity_label(n: u8) -> String {
    match n {
        1 => "1 (Low)".into(),
        3 => "3 (Default)".into(),
        5 => "5 (High)".into(),
        7 => "7 (Very high)".into(),
        10 => "10 (Insane)".into(),
        n => n.to_string(),
    }
}

/// A mouse sensitivity, kept to tenths in range.
pub fn clean_mouse_sensitivity(v: f32) -> f32 {
    let v = (v * 10.0).round() / 10.0;
    v.clamp(MOUSE_SENSITIVITY_MIN, MOUSE_SENSITIVITY_MAX)
}

/// A number as typed (a flag or `lobby.txt`): in range, or None.
pub fn parse_mouse_sensitivity(s: &str) -> Option<f32> {
    let v: f32 = s.trim().parse().ok()?;
    (v.is_finite() && (MOUSE_SENSITIVITY_MIN..=MOUSE_SENSITIVITY_MAX).contains(&v)).then_some(v)
}

pub fn parse_look_sensitivity(s: &str) -> Option<u8> {
    let v: u8 = s.trim().parse().ok()?;
    (LOOK_SENSITIVITY_MIN..=LOOK_SENSITIVITY_MAX)
        .contains(&v)
        .then_some(v)
}

/// on/off, yes/no, true/false, 1/0, enabled/disabled.
pub fn parse_switch(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "on" | "yes" | "true" | "enabled" => Some(true),
        "0" | "off" | "no" | "false" | "disabled" => Some(false),
        _ => None,
    }
}

/// Halo 2's word for a setting that is on or off.
pub fn enabled(on: bool) -> &'static str {
    if on {
        "Enabled"
    } else {
        "Disabled"
    }
}

/// The player's controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    pub buttons: ButtonLayout,
    pub sticks: StickLayout,
    /// 1 to 10, both axes (Halo 2 sets them together).
    pub look_sensitivity: u8,
    /// Look inversion: the controller's thumbstick (profile 0x1D).
    pub look_inverted: bool,
    /// Automatic look centering.
    pub auto_center: bool,
    pub vibration: bool,
    pub mouse_sensitivity: f32,
    /// The mouse's look inversion, kept apart from the controller's as MCC
    /// keeps it (profile 0x1E, and the launcher flips the mouse itself).
    pub mouse_inverted: bool,
}

impl Default for Controls {
    /// Halo 2's defaults (auto centering off, as Halopedia gives it; the
    /// decompilation doesn't show it).
    fn default() -> Self {
        Controls {
            buttons: ButtonLayout::Default,
            sticks: StickLayout::Default,
            look_sensitivity: LOOK_SENSITIVITY_DEFAULT,
            look_inverted: false,
            auto_center: false,
            vibration: true,
            mouse_sensitivity: MOUSE_SENSITIVITY_DEFAULT,
            mouse_inverted: false,
        }
    }
}

/// `lobby.txt`'s names for the settings, in the order it writes them.
pub const KEYS: [&str; 8] = [
    "button_layout",
    "thumbstick_layout",
    "look_sensitivity",
    "look_inversion",
    "auto_look_centering",
    "vibration",
    "mouse_sensitivity",
    "mouse_inversion",
];

impl Controls {
    /// Set one by its `lobby.txt` name (or a short one: `layout`,
    /// `buttons`, `sticks`, `invert_look`, `auto_center`, `invert_mouse`).
    /// Ok(false) for a
    /// name that isn't a control; Err for a bad value.
    pub fn set(&mut self, key: &str, value: &str) -> Result<bool, String> {
        let bad = |what: &str| format!("{key}: {value:?} is not {what}");
        match squash(key).as_str() {
            "buttonlayout" | "layout" | "buttons" => {
                self.buttons = ButtonLayout::from_key(value).ok_or_else(|| {
                    bad("a button layout (default, southpaw, boxer, green_thumb, bumper_jumper or recon)")
                })?;
            }
            "thumbsticklayout" | "sticklayout" | "sticks" => {
                self.sticks = StickLayout::from_key(value).ok_or_else(|| {
                    bad("a thumbstick layout (default, southpaw, legacy or legacy_southpaw)")
                })?;
            }
            "looksensitivity" => {
                self.look_sensitivity =
                    parse_look_sensitivity(value).ok_or_else(|| bad("a number from 1 to 10"))?;
            }
            "lookinversion" | "invertlook" | "lookinverted" => {
                self.look_inverted = parse_switch(value).ok_or_else(|| bad("on or off"))?;
            }
            "autolookcentering" | "autocenter" | "autolookcentre" => {
                self.auto_center = parse_switch(value).ok_or_else(|| bad("on or off"))?;
            }
            "vibration" | "controllervibration" => {
                self.vibration = parse_switch(value).ok_or_else(|| bad("on or off"))?;
            }
            "mousesensitivity" => {
                self.mouse_sensitivity =
                    parse_mouse_sensitivity(value).ok_or_else(|| bad("a number from 0.1 to 10"))?;
            }
            "mouseinversion" | "invertmouse" | "mouseinverted" => {
                self.mouse_inverted = parse_switch(value).ok_or_else(|| bad("on or off"))?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The value of the setting `KEYS[i]`, as `lobby.txt` writes it.
    fn value(&self, i: usize) -> String {
        let sw = |on: bool| if on { "on" } else { "off" }.to_string();
        match i {
            0 => self.buttons.key().into(),
            1 => self.sticks.key().into(),
            2 => self.look_sensitivity.to_string(),
            3 => sw(self.look_inverted),
            4 => sw(self.auto_center),
            5 => sw(self.vibration),
            6 => format!("{}", self.mouse_sensitivity),
            _ => sw(self.mouse_inverted),
        }
    }

    /// `lobby.txt`'s lines for them.
    pub fn lines(&self) -> String {
        KEYS.iter()
            .enumerate()
            .map(|(i, k)| format!("{k} = {}\n", self.value(i)))
            .collect()
    }

    /// The flags that give an engine these controls.
    pub fn args(&self) -> Vec<String> {
        let sw = |on: bool, yes: &str, no: &str| if on { yes } else { no }.to_string();
        vec![
            "--layout".into(),
            self.buttons.key().into(),
            "--sticks".into(),
            self.sticks.key().into(),
            "--look-sensitivity".into(),
            self.look_sensitivity.to_string(),
            sw(self.look_inverted, "--invert-look", "--no-invert-look"),
            sw(self.auto_center, "--auto-center", "--no-auto-center"),
            sw(self.vibration, "--vibration", "--no-vibration"),
            "--mouse-sensitivity".into(),
            format!("{}", self.mouse_sensitivity),
            sw(self.mouse_inverted, "--invert-mouse", "--no-invert-mouse"),
        ]
    }

    /// One line for the log.
    pub fn describe(&self) -> String {
        format!(
            "button layout {}, thumbstick layout {}, look sensitivity {}, look inversion {}, auto look centering {}, vibration {}, mouse sensitivity {}, mouse inversion {}",
            self.buttons.name(),
            self.sticks.name(),
            self.look_sensitivity,
            if self.look_inverted { "on" } else { "off" },
            if self.auto_center { "on" } else { "off" },
            if self.vibration { "on" } else { "off" },
            self.mouse_sensitivity,
            if self.mouse_inverted { "on" } else { "off" }
        )
    }
}

/// The controls given on the command line, each over the one in
/// `lobby.txt` (or the default).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ControlFlags {
    pub buttons: Option<ButtonLayout>,
    pub sticks: Option<StickLayout>,
    pub look_sensitivity: Option<u8>,
    pub look_inverted: Option<bool>,
    pub auto_center: Option<bool>,
    pub vibration: Option<bool>,
    pub mouse_sensitivity: Option<f32>,
    pub mouse_inverted: Option<bool>,
}

impl ControlFlags {
    /// `base` with the flags given put over it.
    pub fn over(&self, base: Controls) -> Controls {
        Controls {
            buttons: self.buttons.unwrap_or(base.buttons),
            sticks: self.sticks.unwrap_or(base.sticks),
            look_sensitivity: self.look_sensitivity.unwrap_or(base.look_sensitivity),
            look_inverted: self.look_inverted.unwrap_or(base.look_inverted),
            auto_center: self.auto_center.unwrap_or(base.auto_center),
            vibration: self.vibration.unwrap_or(base.vibration),
            mouse_sensitivity: self.mouse_sensitivity.unwrap_or(base.mouse_sensitivity),
            mouse_inverted: self.mouse_inverted.unwrap_or(base.mouse_inverted),
        }
    }

    /// Any was given.
    pub fn any(&self) -> bool {
        *self != ControlFlags::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use action::*;
    use button::*;

    #[test]
    fn every_layout_binds_the_actions_a_game_needs() {
        for l in ButtonLayout::ALL {
            let m = l.mapping();
            assert_eq!(m.len(), 66);
            for a in [
                FIRE,
                JUMP,
                MELEE,
                THROW_GRENADE,
                RELOAD,
                ACTION,
                SWITCH_WEAPON,
                SWAP_WEAPON,
                CROUCH,
                ZOOM,
                SHOW_SCORES,
                SWITCH_GRENADE,
                FLASHLIGHT,
                SECONDARY_FIRE,
                DUAL_WIELD,
                BANSHEE_BOMB,
            ] {
                assert!(m[a] <= Y, "{} has nothing for {}", l.name(), action_name(a));
                assert_ne!(m[a], START, "{}: Start is the pause", l.name());
            }
            // The left gun stays on a trigger, the one that doesn't fire,
            // and so does the boost except in Boxer (on B, with the
            // grenade); reload and action share a button, and so do
            // switch weapon and dual wield, and both grenade swaps.
            assert!(matches!(m[DUAL_WIELD], LT | RT), "{}", l.name());
            assert_ne!(m[DUAL_WIELD], m[FIRE]);
            if l == ButtonLayout::Boxer {
                assert_eq!(m[SECONDARY_FIRE], m[THROW_GRENADE]);
            } else {
                assert_eq!(m[DUAL_WIELD], m[SECONDARY_FIRE], "{}", l.name());
            }
            assert_eq!(m[RELOAD], m[ACTION]);
            assert_eq!(m[RELOAD_SECONDARY], m[RELOAD]);
            assert_eq!(m[SWITCH_WEAPON], m[SWAP_WEAPON]);
            assert_eq!(m[NEXT_GRENADE], m[SWITCH_GRENADE], "{}", l.name());
            // No button does two things the engine can't tell apart:
            // every button's actions are a set Halo 2 shares on one button.
            for (a, b) in [(JUMP, MELEE), (FIRE, THROW_GRENADE), (FIRE, MELEE)] {
                assert_ne!(
                    m[a],
                    m[b],
                    "{}: {} {}",
                    l.name(),
                    action_name(a),
                    action_name(b)
                );
            }
            // Movement is on the sticks, which bypass the mapping.
            for a in [MOVE_FORWARD, MOVE_BACK, STRAFE_LEFT, STRAFE_RIGHT, SPRINT] {
                assert_eq!(m[a], NONE);
            }
            // Forge, theater and the later games' actions are left off.
            for a in (25..=48).chain(50..=54).chain(57..=63) {
                assert_eq!(m[a], NONE, "{} action {a}", l.name());
            }
            // Fire on a trigger, jump not on one.
            assert!(matches!(m[FIRE], LT | RT));
            assert!(!matches!(m[JUMP], LT | RT));
        }
    }

    #[test]
    fn the_layouts_differ_where_halo_2_says() {
        let d = ButtonLayout::Default.mapping();
        assert_eq!(
            (d[JUMP], d[MELEE], d[FIRE], d[THROW_GRENADE], d[RELOAD]),
            (A, B, RT, LT, X)
        );
        assert_eq!(
            (d[SWITCH_GRENADE], d[FLASHLIGHT], d[CROUCH], d[ZOOM]),
            (RB, LB, LS, RS)
        );
        assert_eq!(d[SHOW_SCORES], BACK);
        let s = ButtonLayout::Southpaw.mapping();
        assert_eq!((s[FIRE], s[THROW_GRENADE], s[DUAL_WIELD]), (LT, RT, RT));
        let b = ButtonLayout::Boxer.mapping();
        assert_eq!((b[MELEE], b[THROW_GRENADE], b[DUAL_WIELD]), (LT, B, LT));
        // Boxer's boost on B (controller-plan.md 2.3).
        assert_eq!(b[SECONDARY_FIRE], B);
        let g = ButtonLayout::GreenThumb.mapping();
        assert_eq!((g[MELEE], g[ZOOM]), (RS, B));
        let bj = ButtonLayout::BumperJumper.mapping();
        assert_eq!((bj[JUMP], bj[MELEE]), (LB, RB));
        assert_eq!(
            (bj[RELOAD], bj[ACTION], bj[SWITCH_GRENADE], bj[FLASHLIGHT]),
            (B, B, A, X)
        );
        assert_eq!(
            (bj[FLASHLIGHT_ALT], bj[PREVIOUS_GRENADE], bj[NEXT_GRENADE]),
            (DPAD_UP, DPAD_LEFT, A)
        );
        let r = ButtonLayout::Recon.mapping();
        assert_eq!((r[RELOAD], r[ACTION]), (RB, RB));
        assert_eq!((r[FLASHLIGHT], r[FLASHLIGHT_ALT]), (DPAD_UP, X));
        assert_eq!(
            (r[SWITCH_GRENADE], r[NEXT_GRENADE], r[PREVIOUS_GRENADE]),
            (DPAD_RIGHT, DPAD_RIGHT, DPAD_LEFT)
        );
        assert_eq!(d[NEXT_GRENADE], RB);
        // Each layout differs from the others.
        for (i, a) in ButtonLayout::ALL.iter().enumerate() {
            for b in &ButtonLayout::ALL[i + 1..] {
                assert_ne!(a.mapping(), b.mapping(), "{a:?} {b:?}");
            }
        }
        assert!(PadMap::Zero
            .mapping(ButtonLayout::Recon)
            .iter()
            .all(|&b| b == 0));
        assert_eq!(PadMap::Layout.mapping(ButtonLayout::Boxer), b);
        assert_eq!(PadMap::default(), PadMap::Layout);
    }

    #[test]
    fn layouts_read_back_by_key_and_name() {
        for l in ButtonLayout::ALL {
            assert_eq!(ButtonLayout::from_key(l.key()), Some(l));
            assert_eq!(ButtonLayout::from_key(l.name()), Some(l));
            assert_eq!(l.next(true).next(false), l);
        }
        assert_eq!(
            ButtonLayout::from_key("Bumper-Jumper"),
            Some(ButtonLayout::BumperJumper)
        );
        assert_eq!(
            ButtonLayout::from_key("GREEN THUMBS"),
            Some(ButtonLayout::GreenThumb)
        );
        assert_eq!(ButtonLayout::from_key("nope"), None);
        assert_eq!(ButtonLayout::Recon.next(true), ButtonLayout::Default);
        for l in StickLayout::ALL {
            assert_eq!(StickLayout::from_key(l.key()), Some(l));
            assert_eq!(StickLayout::from_key(l.name()), Some(l));
        }
        assert_eq!(
            StickLayout::Default.next(false),
            StickLayout::LegacySouthpaw
        );
    }

    #[test]
    fn bindings_name_each_button() {
        let d = ButtonLayout::Default.bindings();
        let find = |v: &[(u8, String)], b: u8| v.iter().find(|x| x.0 == b).map(|x| x.1.clone());
        assert_eq!(find(&d, RT).as_deref(), Some("Fire"));
        assert_eq!(find(&d, LT).as_deref(), Some("Throw grenade / Left weapon"));
        assert_eq!(find(&d, B).as_deref(), Some("Melee attack"));
        assert_eq!(find(&d, X).as_deref(), Some("Reload / Action"));
        assert_eq!(find(&d, Y).as_deref(), Some("Switch weapons / Dual wield"));
        assert_eq!(find(&d, RB).as_deref(), Some("Swap grenades"));
        assert_eq!(find(&d, RS).as_deref(), Some("Zoom view"));
        assert_eq!(find(&d, BACK).as_deref(), Some("Multiplayer score"));
        assert_eq!(find(&d, DPAD_UP), None);
        assert_eq!(d[0].0, RT);
        // Start pauses, last, in every layout.
        for l in ButtonLayout::ALL {
            let b = l.bindings();
            assert_eq!(
                b.last().map(|x| (x.0, x.1.as_str())),
                Some((START, "Pause game"))
            );
            assert!(b.len() <= 14, "{l:?}: {} lines", b.len());
        }
        let bj = ButtonLayout::BumperJumper.bindings();
        assert_eq!(find(&bj, LB).as_deref(), Some("Jump"));
        assert_eq!(find(&bj, RB).as_deref(), Some("Melee attack"));
        assert_eq!(find(&bj, A).as_deref(), Some("Swap grenades"));
        assert_eq!(find(&bj, DPAD_UP).as_deref(), Some("Flashlight"));
        let boxer = ButtonLayout::Boxer.bindings();
        assert_eq!(
            find(&boxer, LT).as_deref(),
            Some("Melee attack / Left weapon")
        );
        assert_eq!(find(&boxer, B).as_deref(), Some("Throw grenade"));
        let r = ButtonLayout::Recon.bindings();
        assert_eq!(find(&r, X).as_deref(), Some("Flashlight"));
        assert_eq!(find(&r, DPAD_LEFT).as_deref(), Some("Previous grenade"));
        assert_eq!(find(&r, DPAD_RIGHT).as_deref(), Some("Swap grenades"));
        assert_eq!(find(&r, LB), None);
        let m = describe_mapping(&ButtonLayout::Default.mapping());
        assert!(m.starts_with("0:jump=A "), "{m}");
        assert_eq!(describe_mapping(&[NONE; 66]), "nothing mapped");
    }

    #[test]
    fn stick_layouts_move_axes() {
        let (l, r) = ((100, 200), (-300, 400));
        assert_eq!(StickLayout::Default.apply(l, r), (l, r));
        assert_eq!(StickLayout::Southpaw.apply(l, r), (r, l));
        // Legacy: straight pushes pass through the snap unchanged; the
        // left stick's X turns and the right stick's X strafes.
        let (l, r) = ((0, 20000), (15000, 0));
        assert_eq!(StickLayout::Legacy.apply(l, r), ((15000, 20000), (0, 0)));
        let (l, r) = ((-12000, 0), (0, -9000));
        assert_eq!(StickLayout::Legacy.apply(l, r), ((0, 0), (-12000, -9000)));
        // Legacy southpaw: the right stick's Y moves, the left's Y looks.
        let ((mx, my), (lx, ly)) = StickLayout::LegacySouthpaw.apply((5000, 0), (0, -6000));
        assert_eq!((mx, my, lx, ly), (5000, -6000, 0, 0));
        let ((mx, my), (lx, ly)) = StickLayout::LegacySouthpaw.apply((0, 7000), (-8000, 0));
        assert_eq!((mx, my, lx, ly), (0, 0, -8000, 7000));
        // Nothing pushed stays nothing.
        for s in StickLayout::ALL {
            assert_eq!(s.apply((0, 0), (0, 0)), ((0, 0), (0, 0)));
        }
        // Full pushes stay in range.
        let ((a, b), (c, d)) = StickLayout::Legacy.apply((-32768, -32768), (32767, 32767));
        for v in [a, b, c, d] {
            assert!(v > i16::MIN, "{v}");
        }
        // A resting stick (inside XInput's dead zone on both axes) isn't
        // snapped bigger: (6000, 6000) would become (8485, 8485), past the
        // dead zone. Only the axes move.
        for s in [StickLayout::Legacy, StickLayout::LegacySouthpaw] {
            let ((mx, my), (lx, ly)) = s.apply((6000, 6000), (-8000, 8000));
            let mut got = [mx, my, lx, ly];
            got.sort();
            assert_eq!(got, [-8000, 6000, 6000, 8000], "{s:?}");
        }
        // Out of the dead zone on one axis, it is snapped.
        let ((_, my), (lx, _)) = StickLayout::Legacy.apply((9000, 9000), (0, 0));
        assert!(my > 9000 && lx > 9000, "{my} {lx}");
    }

    #[test]
    fn the_legacy_snap_mixes_axes_only_near_a_diagonal() {
        let at = |deg: f32, push: f32| {
            let r = deg.to_radians();
            (r.cos() * push, r.sin() * push)
        };
        let close =
            |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
        // On the diagonal: the whole push on both axes.
        let (x, y) = at(45.0, 0.8);
        assert!(close(legacy_snap(x, y, LEGACY_ZONES[0]), (0.8, 0.8)));
        // 20 degrees off: the nearer axis all of it, the other less.
        let (x, y) = at(25.0, 1.0);
        let (sx, sy) = legacy_snap(x, y, LEGACY_ZONES[0]);
        assert!(close((sx, 0.0), (1.0, 0.0)));
        let want = 1.0 - 20f32.to_radians() * 1.637_022_3;
        assert!((sy - want).abs() < 1e-3, "{sy} {want}");
        // Outside the right stick's 10 degrees: one axis only.
        let (x, y) = at(25.0, 1.0);
        assert!(close(legacy_snap(x, y, LEGACY_ZONES[1]), (1.0, 0.0)));
        let (x, y) = at(-120.0, 0.5);
        assert!(close(legacy_snap(x, y, LEGACY_ZONES[1]), (0.0, -0.5)));
        assert_eq!(legacy_snap(0.0, 0.0, LEGACY_ZONES[0]), (0.0, 0.0));
    }

    #[test]
    fn the_keyboard_table_is_well_formed() {
        let t = keyboard_table();
        let bytes = encode_keyboard_table(&t);
        assert_eq!(bytes.len(), 66 * 0x18);
        assert_eq!(KEY_ENTRY_SIZE, 0x18);
        for (i, keys) in t.iter().enumerate() {
            let at = i * 0x18;
            let a = i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
            assert_eq!(a, i as i32, "entry {i} holds its own action");
            for (n, k) in keys.iter().enumerate() {
                assert!((0..=0xFE).contains(k), "entry {i}: {k:#x}");
                let at = at + 4 + n * 4;
                assert_eq!(&bytes[at..at + 4], &k.to_le_bytes());
            }
            // The keys used come first, then zeros.
            let used = keys.iter().take_while(|&&k| k != 0).count();
            assert!(keys[used..].iter().all(|&k| k == 0), "entry {i}");
        }
        assert_eq!(t[MOVE_FORWARD][0], b'W' as i32);
        assert_eq!(t[MOVE_BACK][0], b'S' as i32);
        assert_eq!(t[STRAFE_LEFT][0], b'A' as i32);
        assert_eq!(t[STRAFE_RIGHT][0], b'D' as i32);
        assert_eq!(t[JUMP][0], 0x20);
        assert_eq!(t[FIRE][0], 0x01);
        assert_eq!(t[ZOOM][0], 0x02);
        assert_eq!(t[CROUCH][0], 0xA2);
        assert_eq!(t[SHOW_SCORES][0], 0x09);
        // halo2.dll's own keys do what it does with them: G throws, F is
        // the flashlight only, Tab switches weapons.
        assert_eq!(t[THROW_GRENADE], [b'G' as i32, 0, 0, 0, 0]);
        assert_eq!(t[FLASHLIGHT][0], b'F' as i32);
        assert_eq!(t[SWITCH_WEAPON][0], 0x09);
        assert_eq!(t[MELEE][0], b'Q' as i32);
        assert_eq!(t[ACTION][0], b'E' as i32);
        assert_eq!(t[RELOAD][0], b'R' as i32);
        for (a, keys) in t.iter().enumerate() {
            if a != FLASHLIGHT {
                assert!(!keys.contains(&(b'F' as i32)), "F on {}", action_name(a));
            }
            if a != THROW_GRENADE {
                assert!(!keys.contains(&(b'G' as i32)), "G on {}", action_name(a));
            }
            // Z and X are the engine's for something unknown: unused.
            assert!(!keys.contains(&(b'Z' as i32)) && !keys.contains(&(b'X' as i32)));
        }
        // The lobby's list says what the table binds.
        assert!(KEYBOARD_BINDINGS.contains(&("G", "Throw grenade", true)));
        assert!(KEYBOARD_BINDINGS.contains(&("F", "Flashlight", true)));
        // Every action a game needs has a key.
        for a in [
            JUMP,
            SWITCH_GRENADE,
            ACTION,
            RELOAD,
            SWITCH_WEAPON,
            MELEE,
            FLASHLIGHT,
            THROW_GRENADE,
            FIRE,
            CROUCH,
            ZOOM,
            SHOW_SCORES,
        ] {
            assert_ne!(t[a][0], 0, "{}", action_name(a));
        }
        // Forge and theater are left alone.
        assert!(t[30].iter().all(|&k| k == 0));
    }

    #[test]
    fn controls_read_back_and_become_flags() {
        let c = Controls {
            buttons: ButtonLayout::BumperJumper,
            sticks: StickLayout::LegacySouthpaw,
            look_sensitivity: 7,
            look_inverted: true,
            auto_center: true,
            vibration: false,
            mouse_sensitivity: 2.3,
            mouse_inverted: true,
        };
        let mut back = Controls::default();
        for line in c.lines().lines() {
            let (k, v) = line.split_once('=').unwrap();
            assert_eq!(back.set(k.trim(), v.trim()), Ok(true), "{line}");
        }
        assert_eq!(back, c);
        let mut d = Controls::default();
        assert_eq!(d.set("colour", "red"), Ok(false));
        assert!(d.set("look_sensitivity", "11").is_err());
        assert!(d.set("look_sensitivity", "0").is_err());
        assert!(d.set("vibration", "maybe").is_err());
        assert!(d.set("button_layout", "claw").is_err());
        assert!(d.set("mouse_sensitivity", "-1").is_err());
        assert_eq!(d, Controls::default());
        assert_eq!(d.set("layout", "Green Thumb"), Ok(true));
        assert_eq!(d.buttons, ButtonLayout::GreenThumb);
        let args = c.args();
        assert!(args.contains(&"bumper_jumper".to_string()));
        assert!(args.contains(&"--no-vibration".to_string()));
        assert!(args.contains(&"--invert-look".to_string()));
        assert!(args.contains(&"--invert-mouse".to_string()));
        assert_eq!(Controls::default().look_sensitivity, 3);
        assert!(Controls::default().vibration);
        // The controller's and the mouse's inversion are apart.
        assert!(!Controls::default().mouse_inverted);
        let mut e = Controls::default();
        assert_eq!(e.set("invert_look", "on"), Ok(true));
        assert!(e.look_inverted && !e.mouse_inverted);
        assert_eq!(e.set("mouse_inversion", "on"), Ok(true));
        assert!(e.mouse_inverted);
    }

    #[test]
    fn flags_go_over_the_saved_controls() {
        let base = Controls {
            buttons: ButtonLayout::Recon,
            look_sensitivity: 9,
            ..Controls::default()
        };
        assert_eq!(ControlFlags::default().over(base), base);
        assert!(!ControlFlags::default().any());
        let f = ControlFlags {
            buttons: Some(ButtonLayout::Boxer),
            vibration: Some(false),
            ..ControlFlags::default()
        };
        assert!(f.any());
        let c = f.over(base);
        assert_eq!(c.buttons, ButtonLayout::Boxer);
        assert_eq!(c.look_sensitivity, 9);
        assert!(!c.vibration);
    }

    #[test]
    fn labels_and_numbers() {
        assert_eq!(look_sensitivity_label(3), "3 (Default)");
        assert_eq!(look_sensitivity_label(10), "10 (Insane)");
        assert_eq!(look_sensitivity_label(4), "4");
        assert_eq!(clean_mouse_sensitivity(1.6000001 + 0.1), 1.7);
        assert_eq!(clean_mouse_sensitivity(0.0), MOUSE_SENSITIVITY_MIN);
        assert_eq!(clean_mouse_sensitivity(99.0), MOUSE_SENSITIVITY_MAX);
        assert_eq!(parse_mouse_sensitivity("1.25"), Some(1.25));
        assert_eq!(parse_mouse_sensitivity("NaN"), None);
        assert_eq!(parse_look_sensitivity("10"), Some(10));
        assert_eq!(parse_switch("Enabled"), Some(true));
        assert_eq!(parse_switch("off"), Some(false));
        assert_eq!(enabled(false), "Disabled");
    }
}
