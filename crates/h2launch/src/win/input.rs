//! Input for local player 0: an XInput pad, plus keyboard and mouse from
//! our own window (key messages and raw mouse motion while the window is
//! in front), plus the optional input script. Everything reaches the
//! engine only through the input state the host fills in slot 36. The
//! thumbstick layout is applied here, to the pad's sticks and the
//! script's alike (the script stands for the physical sticks); the button
//! layout is the gamepad mapping (host slot 116), not done here.
//!
//! Halo 2 also asks Windows for key state itself on its own thread, which
//! never sees our window's key messages (host-interface.verify A3). The
//! no-hook answer the brief picks is AttachThreadInput: the engine thread
//! shares the window thread's input state. `Attach` retries it until the
//! engine thread has a message queue.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

use windows::Win32::Foundation::{HANDLE, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Threading::{AttachThreadInput, GetThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, MAPVK_VSC_TO_VK_EX};
use windows::Win32::UI::Input::XboxController::{
    XInputGetState, XInputSetState, XINPUT_GAMEPAD, XINPUT_STATE, XINPUT_VIBRATION,
};
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, MOUSE_MOVE_ABSOLUTE, RAWINPUT, RAWINPUTHEADER, RID_INPUT,
    RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::{ClipCursor, GetClientRect, GetForegroundWindow};

use super::log::log;
use crate::padpick::{self, Reading, Watch};
use crate::profile::{mouse, mouse_motion, mouse_to_stick, InputFrame, INPUT_STATE_SIZE};

static KEYS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static MOUSE_DX: AtomicI64 = AtomicI64::new(0);
static MOUSE_DY: AtomicI64 = AtomicI64::new(0);
static WHEEL: AtomicI32 = AtomicI32::new(0);
static MOUSE_BUTTONS: AtomicU32 = AtomicU32::new(0);
static CURSOR_X: AtomicI32 = AtomicI32::new(0);
static CURSOR_Y: AtomicI32 = AtomicI32::new(0);
/// Keyboard or mouse was used after the pad (HaloX's sticky rule).
static KM_LAST: AtomicBool = AtomicBool::new(true);
static FOCUSED: AtomicBool = AtomicBool::new(true);
static HIDE_CURSOR: AtomicBool = AtomicBool::new(false);
static CLIP: Mutex<Option<(RECT, f64)>> = Mutex::new(None);

fn set_key(vk: u8, down: bool) {
    let bit = 1u64 << (vk % 64);
    let word = &KEYS[vk as usize / 64];
    if down {
        word.fetch_or(bit, Ordering::SeqCst);
    } else {
        word.fetch_and(!bit, Ordering::SeqCst);
    }
}

fn key_down(vk: u8) -> bool {
    KEYS[vk as usize / 64].load(Ordering::SeqCst) & (1u64 << (vk % 64)) != 0
}

/// A key message for our window. Shift, Ctrl and Alt also set their
/// left/right codes, as the keyboard array is indexed by virtual key.
pub fn on_key(vk: u32, lparam: isize, down: bool) {
    if vk == 0 || vk > 0xFF {
        return;
    }
    let vk = vk as u8;
    let scan = ((lparam >> 16) & 0xFF) as u32;
    let extended = (lparam >> 24) & 1 != 0;
    let (generic, left, right) = match vk {
        0x10 => (true, 0xA0, 0xA1),
        0x11 => (true, 0xA2, 0xA3),
        0x12 => (true, 0xA4, 0xA5),
        _ => (false, 0, 0),
    };
    if generic {
        let side = match vk {
            // SAFETY: a table lookup.
            0x10 => (unsafe { MapVirtualKeyW(scan, MAPVK_VSC_TO_VK_EX) }) as u8,
            _ if extended => right,
            _ => left,
        };
        let side = if side == right { right } else { left };
        set_key(side, down);
        set_key(vk, key_down(left) || key_down(right));
    } else {
        set_key(vk, down);
    }
    if down {
        KM_LAST.store(true, Ordering::SeqCst);
    }
}

/// Raw mouse input for our window.
pub fn on_raw_input(lp: LPARAM) {
    // SAFETY: RAWINPUT is plain data; GetRawInputData fills at most `size`
    // bytes of it.
    let raw = unsafe {
        let mut raw: RAWINPUT = std::mem::zeroed();
        let mut size = std::mem::size_of::<RAWINPUT>() as u32;
        let n = GetRawInputData(
            HRAWINPUT(lp.0 as *mut c_void),
            RID_INPUT,
            Some(&mut raw as *mut RAWINPUT as *mut c_void),
            &mut size,
            std::mem::size_of::<RAWINPUTHEADER>() as u32,
        );
        if n == u32::MAX || n == 0 || raw.header.dwType != RIM_TYPEMOUSE.0 {
            return;
        }
        raw
    };
    if !FOCUSED.load(Ordering::SeqCst) {
        return;
    }
    // SAFETY: dwType says the union holds mouse data.
    let m = unsafe { raw.data.mouse };
    let mut used = false;
    if m.usFlags.0 & MOUSE_MOVE_ABSOLUTE.0 == 0 && (m.lLastX != 0 || m.lLastY != 0) {
        MOUSE_DX.fetch_add(m.lLastX as i64, Ordering::SeqCst);
        MOUSE_DY.fetch_add(m.lLastY as i64, Ordering::SeqCst);
        used = true;
    }
    // SAFETY: the button fields of the same union.
    let (flags, data) = unsafe {
        (
            m.Anonymous.Anonymous.usButtonFlags as u32,
            m.Anonymous.Anonymous.usButtonData,
        )
    };
    // (down flag, up flag, input-state bit, virtual key)
    const BUTTONS: [(u32, u32, u32, u8); 5] = [
        (0x001, 0x002, mouse::LEFT, 0x01),
        (0x004, 0x008, mouse::RIGHT, 0x02),
        (0x010, 0x020, mouse::MIDDLE, 0x04),
        (0x040, 0x080, mouse::X1, 0x05),
        (0x100, 0x200, mouse::X2, 0x06),
    ];
    for (down, up, bit, vk) in BUTTONS {
        if flags & down != 0 {
            MOUSE_BUTTONS.fetch_or(bit, Ordering::SeqCst);
            set_key(vk, true);
            used = true;
        }
        if flags & up != 0 {
            MOUSE_BUTTONS.fetch_and(!bit, Ordering::SeqCst);
            set_key(vk, false);
        }
    }
    if flags & 0x400 != 0 {
        WHEEL.fetch_add(data as i16 as i32, Ordering::SeqCst);
        used = true;
    }
    if used {
        KM_LAST.store(true, Ordering::SeqCst);
    }
}

pub fn on_cursor(x: i32, y: i32) {
    CURSOR_X.store(x, Ordering::SeqCst);
    CURSOR_Y.store(y, Ordering::SeqCst);
}

/// Focus changes: drop held keys and buttons so nothing sticks down.
pub fn set_focus(focused: bool) {
    let was = FOCUSED.swap(focused, Ordering::SeqCst);
    if !focused {
        for w in &KEYS {
            w.store(0, Ordering::SeqCst);
        }
        MOUSE_BUTTONS.store(0, Ordering::SeqCst);
        MOUSE_DX.store(0, Ordering::SeqCst);
        MOUSE_DY.store(0, Ordering::SeqCst);
        WHEEL.store(0, Ordering::SeqCst);
    }
    if was != focused {
        log!("window {}", if focused { "focused" } else { "lost focus" });
    }
}

pub fn cursor_hidden() -> bool {
    HIDE_CURSOR.load(Ordering::SeqCst)
}

/// Keeps the cursor inside the window and hidden while the game runs and
/// the window is in front; frees it otherwise.
pub fn update_cursor_clip(playing: bool) {
    let hwnd = super::hwnd();
    // SAFETY: plain queries.
    let front = unsafe { GetForegroundWindow() } == hwnd;
    let want = playing && front && FOCUSED.load(Ordering::SeqCst);
    let Ok(mut clip) = CLIP.lock() else { return };
    if !want {
        if clip.take().is_some() {
            // SAFETY: frees the cursor.
            let _ = unsafe { ClipCursor(None) };
            HIDE_CURSOR.store(false, Ordering::SeqCst);
        }
        return;
    }
    let mut rc = RECT::default();
    // SAFETY: our window; points converted in place.
    let rect = unsafe {
        if GetClientRect(hwnd, &mut rc).is_err() {
            return;
        }
        let mut a = POINT {
            x: rc.left,
            y: rc.top,
        };
        let mut b = POINT {
            x: rc.right,
            y: rc.bottom,
        };
        let _ = ClientToScreen(hwnd, &mut a);
        let _ = ClientToScreen(hwnd, &mut b);
        RECT {
            left: a.x,
            top: a.y,
            right: b.x,
            bottom: b.y,
        }
    };
    let t = super::now();
    let stale = match *clip {
        Some((r, at)) => r != rect || t - at > 1.0,
        None => true,
    };
    if stale {
        // SAFETY: clips to our client area.
        let _ = unsafe { ClipCursor(Some(&rect)) };
        *clip = Some((rect, t));
        HIDE_CURSOR.store(true, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------- pads

/// The XInput slot player 0 uses, -1 = none yet.
static PAD: AtomicI32 = AtomicI32::new(-1);
/// What `player0_pad` remembers of each slot between polls (`--pad any`).
static WATCH: Mutex<[Watch; 4]> = Mutex::new([Watch::new(); 4]);
/// Per XInput slot: when (seconds, f64 bits) an empty slot may be asked
/// again. Asking about an empty slot is slow, and the engine polls every
/// local player every tick, so an empty slot is asked at most once a
/// second; a connected one every time.
static RETRY_AT: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

fn read_pad(index: u32) -> Option<XINPUT_GAMEPAD> {
    let slot = RETRY_AT.get(index as usize)?;
    let t = super::now();
    if t < f64::from_bits(slot.load(Ordering::Relaxed)) {
        return None;
    }
    let mut st = XINPUT_STATE::default();
    // SAFETY: fills our struct.
    let ok = unsafe { XInputGetState(index, &mut st) } == 0;
    if ok {
        slot.store(0, Ordering::Relaxed);
        Some(st.Gamepad)
    } else {
        slot.store((t + 1.0).to_bits(), Ordering::Relaxed);
        None
    }
}

/// Player 0's pad, and whether it was just used (a button pressed, or a
/// trigger or stick pushed far from rest): the one `--pad` names;
/// otherwise the one in use (`crate::padpick`). That starts as the first
/// connected, and moves to another connected pad when that one is used
/// while the current one has been idle for 2 s, so an idle or virtual
/// controller in an earlier slot (Steam's, DS4Windows') can't keep the
/// one in the player's hands from working, and a pad whose reading never
/// changes can't take player 1 away.
fn player0_pad() -> (Option<XINPUT_GAMEPAD>, bool) {
    let t = super::now();
    let reading = |g: &XINPUT_GAMEPAD| Reading {
        buttons: g.wButtons.0,
        left_trigger: g.bLeftTrigger,
        right_trigger: g.bRightTrigger,
        lx: g.sThumbLX,
        ly: g.sThumbLY,
        rx: g.sThumbRX,
        ry: g.sThumbRY,
    };
    let Ok(mut watch) = WATCH.lock() else {
        return (None, false);
    };
    match super::setup().map(|s| s.args.pad) {
        Some(crate::cli::Pad::None) => return (None, false),
        Some(crate::cli::Pad::Slot(i)) => {
            let g = read_pad(i);
            let was = PAD.swap(if g.is_some() { i as i32 } else { -1 }, Ordering::SeqCst);
            if was != -1 && g.is_none() {
                log!("pad {i} disconnected");
            } else if was == -1 && g.is_some() {
                log!("pad {i} connected (--pad {i}); it is player 1's controller");
            }
            let used = match (&g, watch.get_mut(i as usize)) {
                (Some(g), Some(w)) => w.update(&reading(g), t),
                (None, Some(w)) => {
                    *w = Watch::new();
                    false
                }
                _ => false,
            };
            return (g, used);
        }
        _ => {}
    }
    let mut pads: Vec<(u32, XINPUT_GAMEPAD, bool)> = Vec::new();
    for i in 0..4u32 {
        match read_pad(i) {
            Some(g) => {
                let used = watch[i as usize].update(&reading(&g), t);
                pads.push((i, g, used));
            }
            None => watch[i as usize] = Watch::new(),
        }
    }
    let cur = PAD.load(Ordering::SeqCst);
    let cur = (cur >= 0).then_some(cur as u32);
    if let Some(c) = cur.filter(|c| !pads.iter().any(|p| p.0 == *c)) {
        log!("pad {c} disconnected");
    }
    let connected: Vec<(u32, bool)> = pads.iter().map(|p| (p.0, p.2)).collect();
    let pick = padpick::pick(cur, &connected, &watch[..], t);
    let Some((i, g, used)) = pick.and_then(|i| pads.iter().find(|p| p.0 == i).copied()) else {
        PAD.store(-1, Ordering::SeqCst);
        return (None, false);
    };
    if cur != Some(i) {
        PAD.store(i as i32, Ordering::SeqCst);
        let others: Vec<u32> = pads.iter().map(|p| p.0).filter(|&o| o != i).collect();
        log!(
            "pad {i} {}; it is player 1's controller (other pads connected: {others:?})",
            if used { "is in use" } else { "connected" }
        );
    }
    (Some(g), used)
}

pub fn pad_line() -> String {
    match PAD.load(Ordering::SeqCst) {
        -1 => "pad=none".into(),
        i => format!("pad={i}"),
    }
}

// ---------------------------------------------------------------- the input state

static SCRIPT_T: Mutex<f64> = Mutex::new(0.0);
static LAST_LOG: Mutex<f64> = Mutex::new(-10.0);
static FIRST_NONEMPTY: AtomicBool = AtomicBool::new(false);
static FIRST_SCRIPT: AtomicBool = AtomicBool::new(false);

/// Host slots 36 (`pad_only` false) and 37: fills the engine's 0x130-byte
/// input state for a local player.
pub fn fill(player: i32, out: *mut u8, pad_only: bool) -> bool {
    if out.is_null() {
        return false;
    }
    let mut f = InputFrame::default();
    let (pad, used) = match player {
        0 => player0_pad(),
        1..=3 => (read_pad(player as u32), false),
        _ => (None, false),
    };
    if let Some(g) = &pad {
        f.pad_buttons = g.wButtons.0;
        f.left_trigger = g.bLeftTrigger;
        f.right_trigger = g.bRightTrigger;
        f.thumb_lx = g.sThumbLX;
        f.thumb_ly = g.sThumbLY;
        f.thumb_rx = g.sThumbRX;
        f.thumb_ry = g.sThumbRY;
        // The pad takes over from keyboard and mouse when it is used, not
        // for a reading that never changes.
        if used {
            KM_LAST.store(false, Ordering::SeqCst);
        }
    }
    let setup = super::setup();
    // Player 0 through slot 36: keyboard, mouse and the script too.
    let full = player == 0 && !pad_only;
    // Raw mouse counts this poll (the window's and the script's), scaled
    // below.
    let (mut mouse_dx, mut mouse_dy) = (0.0f32, 0.0f32);
    if full {
        if FOCUSED.load(Ordering::SeqCst) {
            for (i, w) in KEYS.iter().enumerate() {
                let bits = w.load(Ordering::SeqCst);
                for b in 0..64 {
                    f.keys[i * 64 + b] = bits & (1u64 << b) != 0;
                }
            }
            mouse_dx = MOUSE_DX.swap(0, Ordering::SeqCst) as f32;
            mouse_dy = MOUSE_DY.swap(0, Ordering::SeqCst) as f32;
            f.wheel = WHEEL.swap(0, Ordering::SeqCst) as f32 / 120.0;
            f.mouse_buttons = MOUSE_BUTTONS.load(Ordering::SeqCst);
            f.cursor_x = CURSOR_X.load(Ordering::SeqCst) as f32;
            f.cursor_y = CURSOR_Y.load(Ordering::SeqCst) as f32;
        }
        f.is_km = KM_LAST.load(Ordering::SeqCst) || pad.is_none();
        if let (Some(sc), Some(t)) = (setup.and_then(|s| s.script.as_ref()), super::script_time()) {
            let t0 = SCRIPT_T
                .lock()
                .map(|mut g| std::mem::replace(&mut *g, t))
                .unwrap_or(t);
            let syn = sc.sample(t0.min(t), t);
            f.pad_buttons |= syn.pad_buttons;
            f.left_trigger = f.left_trigger.max(syn.left_trigger);
            f.right_trigger = f.right_trigger.max(syn.right_trigger);
            if let Some((x, y)) = syn.left_stick {
                f.thumb_lx = x;
                f.thumb_ly = y;
            }
            if let Some((x, y)) = syn.right_stick {
                f.thumb_rx = x;
                f.thumb_ry = y;
            }
            for &k in &syn.keys {
                f.keys[k as usize] = true;
            }
            mouse_dx += syn.mouse_dx as f32;
            mouse_dy += syn.mouse_dy as f32;
            f.mouse_buttons |= syn.mouse_buttons;
            if syn.pad_active() {
                f.is_km = false;
            } else if syn.km_active() {
                f.is_km = true;
            }
            if (syn.pad_active() || syn.km_active()) && !FIRST_SCRIPT.swap(true, Ordering::SeqCst) {
                log!("input script is now driving player 1 (script time {t:.2} s)");
            }
        }
        // The mouse sensitivity and the mouse's look inversion are done
        // here, as HaloX does them, before the motion is copied into the
        // right stick (the profile has them too, for vehicles).
        let c = setup.map(|s| s.controls).unwrap_or_default();
        (f.mouse_dx, f.mouse_dy) =
            mouse_motion(mouse_dx, mouse_dy, c.mouse_sensitivity, c.mouse_inverted);
    }
    // The thumbstick layout (player 0's: the only one with our profile):
    // which physical stick moves and which looks.
    if let Some(s) = setup.filter(|_| player == 0) {
        let ((lx, ly), (rx, ry)) = s
            .controls
            .sticks
            .apply((f.thumb_lx, f.thumb_ly), (f.thumb_rx, f.thumb_ry));
        (f.thumb_lx, f.thumb_ly, f.thumb_rx, f.thumb_ry) = (lx, ly, rx, ry);
    }
    if full && f.is_km {
        // HaloX copies mouse motion into the right stick in keyboard
        // and mouse mode, because Halo 2 reads the stick when zoomed
        // and in vehicles.
        let (rx, ry) = mouse_to_stick(f.mouse_dx, f.mouse_dy);
        if rx != 0 || ry != 0 {
            f.thumb_rx = rx;
            f.thumb_ry = ry;
        }
    }
    let empty = f
        == InputFrame {
            is_km: f.is_km,
            ..InputFrame::default()
        };
    if player == 0 && !empty && !FIRST_NONEMPTY.swap(true, Ordering::SeqCst) {
        log!("first input sent to the engine: {}", f.describe());
    }
    if player == 0 && !empty && setup.is_some_and(|s| s.args.diag) {
        let t = super::now();
        if let Ok(mut last) = LAST_LOG.try_lock() {
            if t - *last >= 1.0 {
                *last = t;
                log!("input: {}", f.describe());
            }
        }
    }
    let mut bytes = [0u8; INPUT_STATE_SIZE];
    f.encode(&mut bytes);
    // SAFETY: the engine passes a 0x130-byte s_input_state.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, INPUT_STATE_SIZE) };
    true
}

static RUMBLE_LOGGED: AtomicBool = AtomicBool::new(false);

/// Host slot 39: two motor speeds (declared signed in libmcc; the bits go
/// to XInput unchanged, host-interface.verify A14).
pub fn rumble(player: i32, state: *const u8) {
    if state.is_null() {
        return;
    }
    // SAFETY: the engine passes a 4-byte s_rumble_state.
    let (left, right) = unsafe {
        let p = state as *const [u8; 4];
        let b = *p;
        (
            u16::from_le_bytes([b[0], b[1]]),
            u16::from_le_bytes([b[2], b[3]]),
        )
    };
    let index = if player == 0 {
        PAD.load(Ordering::SeqCst)
    } else {
        player
    };
    if !(0..4).contains(&index) {
        return;
    }
    // Vibration off: the profile says so, and none goes to the pad here
    // even if the engine asks.
    if super::setup().is_some_and(|s| !s.controls.vibration) {
        return;
    }
    if !RUMBLE_LOGGED.swap(true, Ordering::SeqCst) {
        log!("first rumble: player {player} pad {index} left {left} right {right}");
    }
    let v = XINPUT_VIBRATION {
        wLeftMotorSpeed: left,
        wRightMotorSpeed: right,
    };
    // SAFETY: plain call with our struct.
    unsafe { XInputSetState(index as u32, &v) };
}

pub fn stop_rumble() {
    let v = XINPUT_VIBRATION::default();
    for i in 0..4 {
        // SAFETY: plain call with our struct.
        unsafe { XInputSetState(i, &v) };
    }
}

// ---------------------------------------------------------------- AttachThreadInput

/// Retries `AttachThreadInput(engine thread, window thread, TRUE)` until
/// it works, for the game thread `initialize_game` returned and for the
/// engine threads seen polling input. The input threads only show up once
/// the map has loaded, so it keeps trying until 60 s after the first input
/// poll (or 10 minutes, if input never comes).
#[derive(Default)]
pub struct Attach {
    done: Vec<u32>,
    failed: Vec<u32>,
    next: f64,
    started: Option<f64>,
    gave_up: bool,
}

impl Attach {
    /// `t`: seconds since start.
    pub fn try_attach(&mut self, game: HANDLE, t: f64) {
        if self.gave_up || t < self.next {
            return;
        }
        self.next = t + 0.25;
        let started = *self.started.get_or_insert(t);
        let window = super::WINDOW_TID.load(Ordering::SeqCst);
        // SAFETY: a query on the engine's thread handle.
        let mut threads = vec![unsafe { GetThreadId(game) }];
        threads.extend(super::host::input_threads());
        threads.dedup();
        let mut waiting = Vec::new();
        for t in threads {
            if t == 0 || t == window || self.done.contains(&t) {
                continue;
            }
            // SAFETY: documented call on thread ids of this process.
            if unsafe { AttachThreadInput(t, window, true) }.as_bool() {
                log!("AttachThreadInput: engine thread {t} now shares the window thread's input");
                self.done.push(t);
            } else {
                if !self.failed.contains(&t) {
                    log!(
                        "AttachThreadInput({t}, {window}) failed: {} (retrying; the thread may not have a message queue yet)",
                        windows::core::Error::from_thread()
                    );
                    self.failed.push(t);
                }
                waiting.push(t);
            }
        }
        let first_input = super::marked(&super::FIRST_INPUT_AT);
        let over = match first_input {
            Some(at) => t - at > 60.0,
            None => t - started > 600.0,
        };
        if over {
            self.gave_up = true;
            if waiting.is_empty() {
                log!(
                    "AttachThreadInput: done ({} threads attached)",
                    self.done.len()
                );
            } else {
                log!("AttachThreadInput: giving up on threads {waiting:?}");
            }
        }
    }
}
