//! Controllers, laid out like Halo 2 on the Xbox: left stick moves, right
//! stick looks, right trigger fires, left trigger throws a grenade, A jumps,
//! B melees, X reloads (hold to pick up), Y switches weapons, clicking the
//! sticks crouches and zooms, the bumpers switch grenades.

use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};
use glam::Vec2;

const DEAD_ZONE: f32 = 0.2;
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
    Zoom,
    /// Start: join the game as another splitscreen player.
    Join,
    /// Back: leave splitscreen.
    Leave,
    /// A, to take over player one with this controller.
    Claim,
}

pub struct Pads {
    gilrs: Option<Gilrs>,
}

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
        Pads { gilrs }
    }

    /// Presses since the last call.
    pub fn presses(&mut self) -> Vec<(GamepadId, PadPress)> {
        let mut out = Vec::new();
        let Some(g) = &mut self.gilrs else {
            return out;
        };
        while let Some(ev) = g.next_event() {
            if let EventType::ButtonPressed(b, _) = ev.event {
                let press = match b {
                    Button::RightTrigger2 => Some(PadPress::Fire),
                    Button::East => Some(PadPress::Melee),
                    Button::West => Some(PadPress::Reload),
                    Button::North => Some(PadPress::SwitchWeapon),
                    Button::LeftTrigger2 => Some(PadPress::Grenade),
                    Button::LeftTrigger | Button::RightTrigger => Some(PadPress::SwitchGrenade),
                    Button::RightThumb => Some(PadPress::Zoom),
                    Button::Start => Some(PadPress::Join),
                    Button::Select => Some(PadPress::Leave),
                    Button::South => Some(PadPress::Claim),
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticks_ignore_small_movements_and_reach_full_deflection() {
        assert_eq!(dead_zone(Vec2::new(0.1, 0.1)), Vec2::ZERO);
        assert!((dead_zone(Vec2::new(1.0, 0.0)).x - 1.0).abs() < 1e-6);
        let half = dead_zone(Vec2::new(0.6, 0.0)).x;
        assert!(half > 0.4 && half < 0.6);
    }
}
