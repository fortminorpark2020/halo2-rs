//! The other byte layouts the host hands the engine: the player profile
//! (host slot 34), the per-frame input state (slots 36/37), the size of
//! the gamepad mapping (slot 116; its contents are in `crate::controls`),
//! the language settings (`SetLibrarySettings`) and the sizes of the video
//! and audio settings (slots 32/33). Offsets are from
//! host-interface.md section 8, recomputed under MSVC rules by its verify
//! file.

/// `s_player_profile`, natural alignment.
pub const PROFILE_SIZE: usize = 0xACC;
/// `s_input_state`.
pub const INPUT_STATE_SIZE: usize = 0x130;
/// `s_gamepad_mapping`: one byte per game action.
pub const GAMEPAD_MAPPING_SIZE: usize = 0x42;
/// `s_game_video_settings` and `s_game_audio_settings`.
pub const VIDEO_SETTINGS_SIZE: usize = 368;
pub const AUDIO_SETTINGS_SIZE: usize = 788;
/// `s_language_settings`: three `wchar_t[85]`.
pub const LANGUAGE_CHARS: usize = 85;
pub const LANGUAGE_SETTINGS_SIZE: usize = 3 * LANGUAGE_CHARS * 2;
/// `s_font_character`, packed.
pub const FONT_CHARACTER_SIZE: usize = 26;
/// `s_game_message_parameter` (engine slot 3's argument).
pub const GAME_MESSAGE_PARAMETER_SIZE: usize = 16;
/// `s_game_result` (host slot 6), opaque.
pub const GAME_RESULT_SIZE: usize = 0x5D138;
/// The profile's per-game blob that host slot 15 hands over.
pub const GAME_SPECIFIC_SIZE: usize = 0x100;

/// Offsets in the profile.
pub mod prof {
    pub const FOV: usize = 0x014;
    pub const VEHICLE_FOV: usize = 0x018;
    pub const CROSSHAIR_LOCATION: usize = 0x01C;
    pub const LOOK_INVERTED: usize = 0x01D;
    pub const MOUSE_LOOK_INVERTED: usize = 0x01E;
    pub const VIBRATION_DISABLED: usize = 0x01F;
    pub const IMPULSE_TRIGGERS_DISABLED: usize = 0x020;
    pub const AIRCRAFT_INVERTED: usize = 0x021;
    pub const AUTO_CENTER: usize = 0x023;
    pub const CROUCH_LOCK: usize = 0x024;
    pub const HOLD_TO_ZOOM: usize = 0x028;
    pub const PRIMARY_COLOUR: usize = 0x02C;
    pub const SECONDARY_COLOUR: usize = 0x030;
    pub const TERTIARY_COLOUR: usize = 0x034;
    pub const USE_ELITE: usize = 0x038;
    pub const CUSTOMIZATION: usize = 0x040;
    pub const SERVICE_TAG: usize = 0x1AC;
    pub const VERTICAL_LOOK_SENSITIVITY: usize = 0x1B5;
    pub const HORIZONTAL_LOOK_SENSITIVITY: usize = 0x1B6;
    pub const LOOK_ACCELERATION: usize = 0x1B7;
    pub const AXIAL_DEAD_ZONE: usize = 0x1B8;
    pub const RADIAL_DEAD_ZONE: usize = 0x1BC;
    pub const ZOOM_LOOK_MULTIPLIER: usize = 0x1C0;
    pub const VEHICLE_LOOK_MULTIPLIER: usize = 0x1C4;
    pub const BUTTON_PRESET: usize = 0x1C8;
    pub const STICK_PRESET: usize = 0x1C9;
    pub const DUAL_WIELD_INVERTED: usize = 0x1DA;
    pub const CONTROLLER_DUAL_WIELD_INVERTED: usize = 0x1DB;
    pub const LOADOUTS: usize = 0x1E4;
    pub const GAME_SPECIFIC: usize = 0x310;
    pub const MOUSE_SENSITIVITY: usize = 0x410;
    pub const MOUSE_SMOOTHING: usize = 0x414;
    pub const MOUSE_ACCELERATION: usize = 0x415;
    pub const KEYBOARD_MOUSE_PRESET: usize = 0x428;
    pub const KEYBOARD_MOUSE_MAPPING: usize = 0x42C;
    pub const MASTER_VOLUME: usize = 0xA5C;
    pub const MUSIC_VOLUME: usize = 0xA60;
    pub const SFX_VOLUME: usize = 0xA64;
    pub const BRIGHTNESS: usize = 0xA74;
    pub const WEAPON_DISPLAY_OFFSET: usize = 0xA78;
    pub const COLOUR_BLIND_MODE: usize = 0xAB4;
    pub const REMASTERED_HUD: usize = 0xAC4;
    pub const HUD_SCALE: usize = 0xAC8;
}

/// What we put in the profile. Everything not named here is 0, including
/// `button_preset` (0x1C8), `stick_preset` (0x1C9), `lefty_toggle` (0x1CA)
/// and `swap_triggers_and_bumpers` (0x1D7), so the engine applies no swap
/// of its own over the mapping and the sticks the launcher gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileSettings {
    /// Stick look sensitivity, Halo 2's 1 to 10 (3 is its default),
    /// written to both axes as Halo 2 sets them together. That MCC's byte
    /// uses the same scale is an estimate; with 0 there may be no turning.
    pub look_sensitivity: u8,
    /// MCC's mouse sensitivity scale; 1.6 is HaloX's header default.
    pub mouse_sensitivity: f32,
    /// 0 locks the camera when zoomed / in a vehicle (HaloX), so 1.0.
    pub zoom_look_multiplier: f32,
    pub vehicle_look_multiplier: f32,
    pub volume: f32,
    /// Look inversion, the controller's (0x1D).
    pub look_inverted: bool,
    /// The mouse's look inversion. The launcher flips the mouse itself
    /// (`mouse_motion`), so 0x1E stays 0: on the owner's PC the engine
    /// flipped it a second time when 0x1E was 1, undoing the inversion.
    pub mouse_inverted: bool,
    pub vibration: bool,
    /// Automatic look centering.
    pub auto_center: bool,
    /// Fill the keyboard and mouse table (0x42C) with
    /// `controls::keyboard_table`; false leaves it all zero, as before
    /// (`--no-key-bindings`, for diagnosis).
    pub key_bindings: bool,
}

impl Default for ProfileSettings {
    fn default() -> Self {
        ProfileSettings::from_controls(&crate::controls::Controls::default())
    }
}

impl ProfileSettings {
    /// The profile for these controls.
    pub fn from_controls(c: &crate::controls::Controls) -> ProfileSettings {
        ProfileSettings {
            look_sensitivity: c.look_sensitivity,
            mouse_sensitivity: c.mouse_sensitivity,
            zoom_look_multiplier: 1.0,
            vehicle_look_multiplier: 1.0,
            volume: 1.0,
            look_inverted: c.look_inverted,
            mouse_inverted: c.mouse_inverted,
            vibration: c.vibration,
            auto_center: c.auto_center,
            key_bindings: true,
        }
    }
}

/// The profile bytes. FOV 0 means the game's default.
pub fn build_profile(s: &ProfileSettings) -> Vec<u8> {
    let mut p = vec![0u8; PROFILE_SIZE];
    let f32_at =
        |p: &mut Vec<u8>, at: usize, v: f32| p[at..at + 4].copy_from_slice(&v.to_le_bytes());
    p[prof::LOOK_INVERTED] = s.look_inverted as u8;
    // Not `s.mouse_inverted`: the launcher already flips the mouse.
    p[prof::MOUSE_LOOK_INVERTED] = 0;
    p[prof::VIBRATION_DISABLED] = (!s.vibration) as u8;
    p[prof::AUTO_CENTER] = s.auto_center as u8;
    p[prof::VERTICAL_LOOK_SENSITIVITY] = s.look_sensitivity;
    p[prof::HORIZONTAL_LOOK_SENSITIVITY] = s.look_sensitivity;
    f32_at(&mut p, prof::ZOOM_LOOK_MULTIPLIER, s.zoom_look_multiplier);
    f32_at(
        &mut p,
        prof::VEHICLE_LOOK_MULTIPLIER,
        s.vehicle_look_multiplier,
    );
    f32_at(&mut p, prof::MOUSE_SENSITIVITY, s.mouse_sensitivity);
    if s.key_bindings {
        let t = crate::controls::encode_keyboard_table(&crate::controls::keyboard_table());
        let at = prof::KEYBOARD_MOUSE_MAPPING;
        p[at..at + t.len()].copy_from_slice(&t);
    }
    f32_at(&mut p, prof::MASTER_VOLUME, s.volume);
    f32_at(&mut p, prof::MUSIC_VOLUME, s.volume);
    f32_at(&mut p, prof::SFX_VOLUME, s.volume);
    p
}

/// Offsets in the input state.
pub mod input {
    /// i32: 1 when keyboard and mouse is this player's active device.
    pub const IS_KM: usize = 0x000;
    /// bool[256] indexed by Windows virtual-key code.
    pub const KEYS: usize = 0x004;
    /// f32 relative mouse motion since the last call.
    pub const MOUSE_X: usize = 0x104;
    pub const MOUSE_Y: usize = 0x108;
    /// f32 absolute cursor position (menus).
    pub const CURSOR_X: usize = 0x10C;
    pub const CURSOR_Y: usize = 0x110;
    /// f32 wheel in notches, and its scale (HaloX writes 1.0).
    pub const WHEEL: usize = 0x114;
    pub const WHEEL_SCALE: usize = 0x118;
    /// i32 bits: 0 left, 1 right, 2 middle, 3 X1, 4 X2.
    pub const MOUSE_BUTTONS: usize = 0x11C;
    /// i32 raw XInput `wButtons`.
    pub const PAD_BUTTONS: usize = 0x120;
    /// u8 triggers.
    pub const LEFT_TRIGGER: usize = 0x124;
    pub const RIGHT_TRIGGER: usize = 0x125;
    /// i16 sticks, +Y up.
    pub const THUMB_LX: usize = 0x126;
    pub const THUMB_LY: usize = 0x128;
    pub const THUMB_RX: usize = 0x12A;
    pub const THUMB_RY: usize = 0x12C;
}

/// XInput `wButtons` bits.
pub mod pad {
    pub const DPAD_UP: u16 = 0x0001;
    pub const DPAD_DOWN: u16 = 0x0002;
    pub const DPAD_LEFT: u16 = 0x0004;
    pub const DPAD_RIGHT: u16 = 0x0008;
    pub const START: u16 = 0x0010;
    pub const BACK: u16 = 0x0020;
    pub const LEFT_THUMB: u16 = 0x0040;
    pub const RIGHT_THUMB: u16 = 0x0080;
    pub const LEFT_SHOULDER: u16 = 0x0100;
    pub const RIGHT_SHOULDER: u16 = 0x0200;
    pub const A: u16 = 0x1000;
    pub const B: u16 = 0x2000;
    pub const X: u16 = 0x4000;
    pub const Y: u16 = 0x8000;
}

/// Mouse button bits in the input state.
pub mod mouse {
    pub const LEFT: u32 = 1 << 0;
    pub const RIGHT: u32 = 1 << 1;
    pub const MIDDLE: u32 = 1 << 2;
    pub const X1: u32 = 1 << 3;
    pub const X2: u32 = 1 << 4;
}

/// One player's input for one engine poll.
#[derive(Clone, Debug, PartialEq)]
pub struct InputFrame {
    pub is_km: bool,
    pub keys: [bool; 256],
    pub mouse_dx: f32,
    pub mouse_dy: f32,
    pub cursor_x: f32,
    pub cursor_y: f32,
    pub wheel: f32,
    pub mouse_buttons: u32,
    pub pad_buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub thumb_lx: i16,
    pub thumb_ly: i16,
    pub thumb_rx: i16,
    pub thumb_ry: i16,
}

impl Default for InputFrame {
    fn default() -> Self {
        InputFrame {
            is_km: false,
            keys: [false; 256],
            mouse_dx: 0.0,
            mouse_dy: 0.0,
            cursor_x: 0.0,
            cursor_y: 0.0,
            wheel: 0.0,
            mouse_buttons: 0,
            pad_buttons: 0,
            left_trigger: 0,
            right_trigger: 0,
            thumb_lx: 0,
            thumb_ly: 0,
            thumb_rx: 0,
            thumb_ry: 0,
        }
    }
}

impl InputFrame {
    /// The 0x130 bytes the engine reads.
    pub fn encode(&self, out: &mut [u8; INPUT_STATE_SIZE]) {
        out.fill(0);
        out[input::IS_KM..input::IS_KM + 4].copy_from_slice(&(self.is_km as i32).to_le_bytes());
        for (i, &k) in self.keys.iter().enumerate() {
            out[input::KEYS + i] = k as u8;
        }
        let f = |out: &mut [u8; INPUT_STATE_SIZE], at: usize, v: f32| {
            out[at..at + 4].copy_from_slice(&v.to_le_bytes())
        };
        f(out, input::MOUSE_X, self.mouse_dx);
        f(out, input::MOUSE_Y, self.mouse_dy);
        f(out, input::CURSOR_X, self.cursor_x);
        f(out, input::CURSOR_Y, self.cursor_y);
        f(out, input::WHEEL, self.wheel);
        f(out, input::WHEEL_SCALE, 1.0);
        out[input::MOUSE_BUTTONS..input::MOUSE_BUTTONS + 4]
            .copy_from_slice(&self.mouse_buttons.to_le_bytes());
        out[input::PAD_BUTTONS..input::PAD_BUTTONS + 4]
            .copy_from_slice(&(self.pad_buttons as u32).to_le_bytes());
        out[input::LEFT_TRIGGER] = self.left_trigger;
        out[input::RIGHT_TRIGGER] = self.right_trigger;
        let s = |out: &mut [u8; INPUT_STATE_SIZE], at: usize, v: i16| {
            out[at..at + 2].copy_from_slice(&v.to_le_bytes())
        };
        s(out, input::THUMB_LX, self.thumb_lx);
        s(out, input::THUMB_LY, self.thumb_ly);
        s(out, input::THUMB_RX, self.thumb_rx);
        s(out, input::THUMB_RY, self.thumb_ry);
    }

    /// One line for the log.
    pub fn describe(&self) -> String {
        let keys: Vec<String> = self
            .keys
            .iter()
            .enumerate()
            .filter(|(_, &k)| k)
            .map(|(i, _)| format!("{i:#04x}"))
            .collect();
        format!(
            "km={} pad={:#06x} lt={} rt={} l=({},{}) r=({},{}) mouse=({:.2},{:.2}) buttons={:#x} wheel={} keys=[{}]",
            self.is_km as u8,
            self.pad_buttons,
            self.left_trigger,
            self.right_trigger,
            self.thumb_lx,
            self.thumb_ly,
            self.thumb_rx,
            self.thumb_ry,
            self.mouse_dx,
            self.mouse_dy,
            self.mouse_buttons,
            self.wheel,
            keys.join(" ")
        )
    }
}

/// Mouse motion copied into the right stick, as HaloX does in keyboard and
/// mouse mode, because halo2 reads the stick when zoomed and in vehicles:
/// times 2000, Y flipped (stick +Y is up, mouse +Y is down), clamped.
pub fn mouse_to_stick(dx: f32, dy: f32) -> (i16, i16) {
    let c = |v: f32| (v * 2000.0).clamp(-32767.0, 32767.0) as i16;
    (c(dx), c(-dy))
}

/// Raw mouse counts to the input state's units at MCC's default mouse
/// sensitivity (1.6): counts x sensitivity x 0.10, HaloX's scale with its
/// default sensitivity 0.10 (an estimate for halo2; tune it on the PC).
pub const MOUSE_SCALE: f32 = 0.01;

/// The MCC mouse sensitivity at which `MOUSE_SCALE` applies.
pub const MOUSE_SCALE_AT: f32 = 1.6;

/// Raw mouse counts (`dx`, `dy`, +Y down) to the input state's units for
/// an MCC mouse sensitivity and the mouse's look inversion, as HaloX does
/// it: counts x sensitivity x 0.0625 x 0.10 (so `MOUSE_SCALE` at 1.6),
/// and Y flipped when inverted. The engine may scale on-foot aim by
/// nothing else: HaloX writes the profile's mouse sensitivity (0x410)
/// only because "vehicle code also reads" it.
pub fn mouse_motion(dx: f32, dy: f32, sensitivity: f32, inverted: bool) -> (f32, f32) {
    let k = MOUSE_SCALE * sensitivity / MOUSE_SCALE_AT;
    let y = if inverted { -dy } else { dy };
    (dx * k, y * k)
}

/// `s_language_settings` with one tag (e.g. "en-US") in all three fields
/// (audio, text 1, text 2), as HaloX passes it.
pub fn language_settings(tag: &str) -> [u16; 3 * LANGUAGE_CHARS] {
    let mut out = [0u16; 3 * LANGUAGE_CHARS];
    for field in 0..3 {
        for (i, c) in tag.encode_utf16().take(LANGUAGE_CHARS - 1).enumerate() {
            out[field * LANGUAGE_CHARS + i] = c;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(PROFILE_SIZE, 2764);
        assert_eq!(INPUT_STATE_SIZE, 0x130);
        assert_eq!(GAMEPAD_MAPPING_SIZE, 66);
        assert_eq!(LANGUAGE_SETTINGS_SIZE, 0x1FE);
        assert_eq!(std::mem::size_of::<[u16; 3 * LANGUAGE_CHARS]>(), 0x1FE);
        assert_eq!(GAME_RESULT_SIZE, 0x5D138);
    }

    #[test]
    fn profile_offsets() {
        assert_eq!(prof::FOV, 0x14);
        assert_eq!(prof::LOOK_INVERTED, 0x1D);
        assert_eq!(prof::VIBRATION_DISABLED, 0x1F);
        assert_eq!(prof::VERTICAL_LOOK_SENSITIVITY, 0x1B5);
        assert_eq!(prof::HORIZONTAL_LOOK_SENSITIVITY, 0x1B6);
        assert_eq!(prof::ZOOM_LOOK_MULTIPLIER, 0x1C0);
        assert_eq!(prof::VEHICLE_LOOK_MULTIPLIER, 0x1C4);
        assert_eq!(prof::GAME_SPECIFIC, 0x310);
        assert_eq!(
            prof::GAME_SPECIFIC + GAME_SPECIFIC_SIZE,
            prof::MOUSE_SENSITIVITY
        );
        assert_eq!(prof::MOUSE_SENSITIVITY, 0x410);
        // 66 keyboard entries of 0x18 bytes end at the master volume.
        assert_eq!(
            prof::KEYBOARD_MOUSE_MAPPING + 66 * 0x18,
            prof::MASTER_VOLUME
        );
        assert_eq!(prof::MASTER_VOLUME, 0xA5C);
        assert_eq!(prof::MUSIC_VOLUME, 0xA60);
        assert_eq!(prof::SFX_VOLUME, 0xA64);
        // Five loadouts of 0x3C bytes end at the game-specific blob.
        assert_eq!(prof::LOADOUTS + 5 * 0x3C, prof::GAME_SPECIFIC);
        // Five f32[3] weapon offsets end at the colour-blind settings.
        assert_eq!(
            prof::WEAPON_DISPLAY_OFFSET + 5 * 12,
            prof::COLOUR_BLIND_MODE
        );
        assert_eq!(prof::CUSTOMIZATION + 0x16C, prof::SERVICE_TAG);
        assert_eq!(prof::HUD_SCALE + 4, PROFILE_SIZE);
    }

    #[test]
    fn default_profile_values() {
        let p = build_profile(&ProfileSettings::default());
        assert_eq!(p.len(), PROFILE_SIZE);
        let f = |at: usize| f32::from_le_bytes(p[at..at + 4].try_into().unwrap());
        let i = |at: usize| i32::from_le_bytes(p[at..at + 4].try_into().unwrap());
        assert_eq!(i(prof::FOV), 0);
        assert_eq!(i(prof::VEHICLE_FOV), 0);
        assert_eq!(f(prof::ZOOM_LOOK_MULTIPLIER), 1.0);
        assert_eq!(f(prof::VEHICLE_LOOK_MULTIPLIER), 1.0);
        assert_eq!(f(prof::MASTER_VOLUME), 1.0);
        assert_eq!(f(prof::MUSIC_VOLUME), 1.0);
        assert_eq!(f(prof::SFX_VOLUME), 1.0);
        assert_eq!(f(prof::MOUSE_SENSITIVITY), 1.6);
        assert_eq!(p[prof::VERTICAL_LOOK_SENSITIVITY], 3);
        assert_eq!(p[prof::HORIZONTAL_LOOK_SENSITIVITY], 3);
        assert_eq!(p[prof::VIBRATION_DISABLED], 0);
        assert_eq!(p[prof::LOOK_INVERTED], 0);
        assert_eq!(p[prof::AUTO_CENTER], 0);
        // No preset or swap of the engine's own over ours.
        for at in [prof::BUTTON_PRESET, prof::STICK_PRESET, 0x1CA, 0x1D7] {
            assert_eq!(p[at], 0, "{at:#x}");
        }
        assert_eq!(i(prof::KEYBOARD_MOUSE_PRESET), 0);
        // The keyboard table: entry i is action i, W moves forward.
        let entry = |n: usize| prof::KEYBOARD_MOUSE_MAPPING + n * 0x18;
        assert_eq!(i(entry(0)), 0);
        assert_eq!(i(entry(0) + 4), 0x20);
        assert_eq!(i(entry(16)), 16);
        assert_eq!(i(entry(16) + 4), 'W' as i32);
        assert_eq!(i(entry(65)), 65);
        assert_eq!(&p[entry(66)..entry(66) + 4], &1.0f32.to_le_bytes());
    }

    #[test]
    fn controls_reach_the_profile() {
        let c = crate::controls::Controls {
            look_sensitivity: 10,
            look_inverted: true,
            auto_center: true,
            vibration: false,
            mouse_sensitivity: 2.5,
            ..Default::default()
        };
        let mut ps = ProfileSettings::from_controls(&c);
        let p = build_profile(&ps);
        let f = |at: usize| f32::from_le_bytes(p[at..at + 4].try_into().unwrap());
        assert_eq!(p[prof::VERTICAL_LOOK_SENSITIVITY], 10);
        assert_eq!(p[prof::HORIZONTAL_LOOK_SENSITIVITY], 10);
        assert_eq!(p[prof::LOOK_INVERTED], 1);
        // The controller's inversion leaves the mouse's alone.
        assert_eq!(p[prof::MOUSE_LOOK_INVERTED], 0);
        assert_eq!(p[prof::AUTO_CENTER], 1);
        assert_eq!(p[prof::VIBRATION_DISABLED], 1);
        assert_eq!(f(prof::MOUSE_SENSITIVITY), 2.5);
        let mouse = ProfileSettings::from_controls(&crate::controls::Controls {
            mouse_inverted: true,
            ..Default::default()
        });
        assert!(mouse.mouse_inverted);
        let m = build_profile(&mouse);
        // The launcher flips the mouse itself, so the engine's flag stays off.
        assert_eq!(
            (m[prof::LOOK_INVERTED], m[prof::MOUSE_LOOK_INVERTED]),
            (0, 0)
        );
        ps.key_bindings = false;
        let p = build_profile(&ps);
        let table = prof::KEYBOARD_MOUSE_MAPPING..prof::MASTER_VOLUME;
        assert!(p[table].iter().all(|&b| b == 0));
    }

    #[test]
    fn mouse_motion_follows_sensitivity_and_inversion() {
        let near = |(x, y): (f32, f32), (wx, wy): (f32, f32)| {
            assert!((x - wx).abs() < 1e-5 && (y - wy).abs() < 1e-5, "({x}, {y})");
        };
        // MCC's default: HaloX's 0.01 a count.
        near(mouse_motion(100.0, -50.0, 1.6, false), (1.0, -0.5));
        // Twice the sensitivity, twice the motion.
        near(mouse_motion(100.0, -50.0, 3.2, false), (2.0, -1.0));
        // Inverted: Y flips, X doesn't.
        near(mouse_motion(100.0, -50.0, 1.6, true), (1.0, 0.5));
        assert_eq!(mouse_motion(0.0, 0.0, 9.0, true), (0.0, 0.0));
    }

    #[test]
    fn input_offsets_and_encoding() {
        assert_eq!(input::KEYS + 256, input::MOUSE_X);
        assert_eq!(input::MOUSE_X, 0x104);
        assert_eq!(input::MOUSE_BUTTONS, 0x11C);
        assert_eq!(input::PAD_BUTTONS, 0x120);
        assert_eq!(input::THUMB_RY + 2 + 2, INPUT_STATE_SIZE);
        let mut f = InputFrame {
            is_km: true,
            mouse_dx: 0.5,
            mouse_buttons: mouse::RIGHT,
            pad_buttons: pad::A | pad::LEFT_SHOULDER,
            left_trigger: 7,
            right_trigger: 255,
            thumb_lx: -2,
            thumb_ly: 32767,
            ..Default::default()
        };
        f.keys[0x57] = true;
        let mut out = [0xAAu8; INPUT_STATE_SIZE];
        f.encode(&mut out);
        assert_eq!(&out[0..4], &[1, 0, 0, 0]);
        assert_eq!(out[input::KEYS + 0x57], 1);
        assert_eq!(out[input::KEYS + 0x58], 0);
        assert_eq!(&out[0x104..0x108], &0.5f32.to_le_bytes());
        assert_eq!(&out[0x118..0x11C], &1.0f32.to_le_bytes());
        assert_eq!(&out[0x11C..0x120], &[2, 0, 0, 0]);
        assert_eq!(&out[0x120..0x124], &[0x00, 0x11, 0, 0]);
        assert_eq!(out[0x124], 7);
        assert_eq!(out[0x125], 255);
        assert_eq!(&out[0x126..0x128], &(-2i16).to_le_bytes());
        assert_eq!(&out[0x128..0x12A], &32767i16.to_le_bytes());
        assert_eq!(&out[0x12E..0x130], &[0, 0]);
    }

    #[test]
    fn mouse_stick() {
        assert_eq!(mouse_to_stick(0.0, 0.0), (0, 0));
        assert_eq!(mouse_to_stick(1.0, 1.0), (2000, -2000));
        assert_eq!(mouse_to_stick(100.0, -100.0), (32767, 32767));
    }

    #[test]
    fn language() {
        let l = language_settings("en-US");
        for field in 0..3 {
            let s = &l[field * LANGUAGE_CHARS..(field + 1) * LANGUAGE_CHARS];
            assert_eq!(crate::util::from_wide(s), "en-US");
        }
        // text_1 starts at byte 0xAA, text_2 at 0x154.
        assert_eq!(LANGUAGE_CHARS * 2, 0xAA);
        assert_eq!(l[0xAA / 2], 'e' as u16);
        assert_eq!(l[0x154 / 2], 'e' as u16);
    }
}
