//! Controllers, laid out like Halo 2 on the Xbox: left stick moves, right
//! stick looks, right trigger fires, left trigger throws a grenade, A jumps,
//! B melees, X reloads (hold to pick up), Y switches weapons, clicking the
//! sticks crouches and zooms, the bumpers switch grenades, Start pauses and
//! Back shows the scoreboard. In menus the d-pad or left stick moves, A
//! chooses and B goes back.

use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};
use glam::Vec2;

/// The sticks' dead zone, after gilrs' own (a tenth of the way, already
/// taken off): about a fifth of the way out in all.
const DEAD_ZONE: f32 = 0.11;
/// Triggers count as pulled past this.
const TRIGGER: f32 = 0.3;

/// A controller's sticks and held buttons this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadState {
    pub left: Vec2,
    pub right: Vec2,
    pub fire: bool,
    pub jump: bool,
    pub crouch: bool,
    pub zoom: bool,
    pub action: bool,
    /// Y and the left trigger held (dual wielding: take a second gun, and
    /// fire it).
    pub switch: bool,
    pub grenade: bool,
    /// Back held: the scoreboard.
    pub scores: bool,
}

/// A button press, as the game sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadPress {
    Fire,
    Melee,
    Reload,
    SwitchWeapon,
    Grenade,
    SwitchGrenade,
    /// The flashlight: the Arbiter's active camouflage.
    Vision,
    Zoom,
    /// Start: join as another splitscreen player, or pause.
    Join,
    /// Back: leave splitscreen (in the lobby).
    Leave,
    /// A, to take over player one with this controller.
    Claim,
    /// The d-pad, or the left stick pushed one way (menus).
    Up,
    Down,
    Left,
    Right,
    /// The controller went (unplugged, or out of battery).
    Disconnected,
    /// It's back.
    Connected,
}

pub struct Pads {
    gilrs: Option<Gilrs>,
    /// Which way each controller's left stick last pointed (x, y), for
    /// menu presses.
    sticks: Vec<(GamepadId, [i8; 2])>,
}

/// The stick counts as pushed past this, and let go under the second.
const STICK_PUSH: (f32, f32) = (0.6, 0.35);

fn dead_zone(v: Vec2) -> Vec2 {
    let len = v.length();
    if len < DEAD_ZONE {
        Vec2::ZERO
    } else {
        v / len * ((len - DEAD_ZONE) / (1.0 - DEAD_ZONE)).min(1.0)
    }
}

impl Pads {
    pub fn new() -> Pads {
        let gilrs = match Gilrs::new() {
            Ok(g) => Some(g),
            Err(e) => {
                println!("warning: controllers unavailable: {e}");
                None
            }
        };
        Pads {
            gilrs,
            sticks: Vec::new(),
        }
    }

    /// Presses since the last call.
    pub fn presses(&mut self) -> Vec<(GamepadId, PadPress)> {
        let mut out = Vec::new();
        let Some(g) = &mut self.gilrs else {
            return out;
        };
        while let Some(ev) = g.next_event() {
            match ev.event {
                EventType::Disconnected => {
                    self.sticks.retain(|s| s.0 != ev.id);
                    out.push((ev.id, PadPress::Disconnected));
                }
                EventType::Connected => out.push((ev.id, PadPress::Connected)),
                _ => {}
            }
            if let EventType::AxisChanged(axis @ (Axis::LeftStickX | Axis::LeftStickY), v, _) =
                ev.event
            {
                let k = match self.sticks.iter().position(|s| s.0 == ev.id) {
                    Some(k) => k,
                    None => {
                        self.sticks.push((ev.id, [0, 0]));
                        self.sticks.len() - 1
                    }
                };
                let a = (axis == Axis::LeftStickY) as usize;
                let dir = &mut self.sticks[k].1[a];
                if v.abs() < STICK_PUSH.1 {
                    *dir = 0;
                } else if v.abs() > STICK_PUSH.0 && *dir != v.signum() as i8 {
                    *dir = v.signum() as i8;
                    let press = match (a, *dir > 0) {
                        (0, false) => PadPress::Left,
                        (0, true) => PadPress::Right,
                        (_, false) => PadPress::Down,
                        (_, true) => PadPress::Up,
                    };
                    out.push((ev.id, press));
                }
            }
            if let EventType::ButtonPressed(b, _) = ev.event {
                let press = match b {
                    Button::RightTrigger2 => Some(PadPress::Fire),
                    Button::East => Some(PadPress::Melee),
                    Button::West => Some(PadPress::Reload),
                    Button::North => Some(PadPress::SwitchWeapon),
                    Button::LeftTrigger2 => Some(PadPress::Grenade),
                    Button::LeftTrigger => Some(PadPress::Vision),
                    Button::RightTrigger => Some(PadPress::SwitchGrenade),
                    Button::RightThumb => Some(PadPress::Zoom),
                    Button::Start => Some(PadPress::Join),
                    Button::Select => Some(PadPress::Leave),
                    Button::South => Some(PadPress::Claim),
                    Button::DPadUp => Some(PadPress::Up),
                    Button::DPadDown => Some(PadPress::Down),
                    Button::DPadLeft => Some(PadPress::Left),
                    Button::DPadRight => Some(PadPress::Right),
                    _ => None,
                };
                if let Some(p) = press {
                    out.push((ev.id, p));
                }
            }
        }
        out
    }

    pub fn state(&self, id: GamepadId) -> Option<PadState> {
        let g = self.gilrs.as_ref()?;
        let pad = g.connected_gamepad(id)?;
        let stick = |x: Axis, y: Axis| dead_zone(Vec2::new(pad.value(x), pad.value(y)));
        let trigger = |b: Button| {
            pad.button_data(b)
                .map_or(pad.is_pressed(b), |d| d.value() > TRIGGER)
        };
        Some(PadState {
            left: stick(Axis::LeftStickX, Axis::LeftStickY),
            right: stick(Axis::RightStickX, Axis::RightStickY),
            fire: trigger(Button::RightTrigger2),
            jump: pad.is_pressed(Button::South),
            crouch: pad.is_pressed(Button::LeftThumb),
            zoom: pad.is_pressed(Button::RightThumb),
            action: pad.is_pressed(Button::West),
            switch: pad.is_pressed(Button::North),
            grenade: trigger(Button::LeftTrigger2),
            scores: pad.is_pressed(Button::Select),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticks_ignore_small_movements_and_reach_full_deflection() {
        assert_eq!(dead_zone(Vec2::new(0.07, 0.07)), Vec2::ZERO);
        assert_ne!(dead_zone(Vec2::new(0.1, 0.1)), Vec2::ZERO);
        assert!((dead_zone(Vec2::new(1.0, 0.0)).x - 1.0).abs() < 1e-6);
        let half = dead_zone(Vec2::new(0.6, 0.0)).x;
        assert!(half > 0.4 && half < 0.6);
    }
}
