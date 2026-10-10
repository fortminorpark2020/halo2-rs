//! The host object (`i_game_manager`) the engine calls back into. It is a
//! vtable pointer, the event-manager pointer at +8, and padding out to
//! 0xB788 bytes (MCC's size, in case the engine writes into it). The
//! vtable has 256 entries: the slots that need real behaviour
//! (launch-sequence.verify C2) and, for every other slot, a distinct
//! logging stub that returns 0. Slots 38 and 69 return an f32 (0.0) so
//! XMM0 is set. Every function uses `extern "system"` with an explicit
//! `this` first argument, and catches panics so none crosses into the
//! engine.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

use super::gfx;
use super::log::log;
use super::Setup;
use crate::paths;
use crate::profile::{self, PadMap};
use crate::slots::{host_slot_name, summary_lines, SlotCount, HOST_SLOTS};

// ---------------------------------------------------------------- call records

static COUNTS: [AtomicU64; HOST_SLOTS] = [const { AtomicU64::new(0) }; HOST_SLOTS];
static LAST: [AtomicU64; HOST_SLOTS] = [const { AtomicU64::new(0) }; HOST_SLOTS];
/// Up to four distinct calling thread ids per slot, for AttachThreadInput
/// and the thread audit.
static THREADS: [[AtomicU64; 4]; HOST_SLOTS] =
    [const { [const { AtomicU64::new(0) }; 4] }; HOST_SLOTS];

fn note_thread(slot: usize) -> bool {
    let tid = super::tid() as u64 + 1;
    let mut fresh = false;
    for a in &THREADS[slot] {
        match a.compare_exchange(0, tid, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => {
                fresh = true;
                break;
            }
            Err(x) if x == tid => break,
            Err(_) => {}
        }
    }
    fresh
}

/// Records a call. Logs the first one (with its arguments) and every new
/// calling thread, both only in --diag, except the very first call of a
/// slot which is always logged.
fn record(slot: usize, a1: usize, a2: usize, a3: usize, a4: usize) {
    let n = COUNTS[slot].fetch_add(1, Ordering::Relaxed);
    let fresh_thread = note_thread(slot);
    let diag = super::setup().is_some_and(|s| s.args.diag);
    if n == 0 {
        log!(
            "host slot {slot} {} first call: a1={a1:#x} a2={a2:#x} a3={a3:#x} a4={a4:#x} (t {})",
            host_slot_name(slot),
            super::tid()
        );
    } else if fresh_thread && diag {
        log!(
            "host slot {slot} {} also called from thread {}",
            host_slot_name(slot),
            super::tid()
        );
    }
}

/// Thread ids seen calling get_input_state (36/37), for AttachThreadInput.
pub fn input_threads() -> Vec<u32> {
    let mut out = Vec::new();
    for slot in [36usize, 37] {
        for a in &THREADS[slot] {
            let v = a.load(Ordering::SeqCst);
            if v != 0 {
                out.push((v - 1) as u32);
            }
        }
    }
    out
}

fn panic_guard<T: Default>(slot: usize, f: impl FnOnce() -> T + std::panic::UnwindSafe) -> T {
    std::panic::catch_unwind(f).unwrap_or_else(|_| {
        log!(
            "host slot {slot} {} panicked (ignored)",
            host_slot_name(slot)
        );
        T::default()
    })
}

// ---------------------------------------------------------------- the object

#[repr(C)]
struct Host {
    vtable: *const *const c_void,
    event_manager: *mut c_void,
    /// Padding to MCC's 0xB788 bytes.
    _pad: [u8; 0xB788 - 16],
}
// SAFETY: the object is set up once and then only read by the engine (it
// has no interior fields we mutate); any writes the engine makes land in
// the padding.
unsafe impl Sync for Host {}
unsafe impl Send for Host {}

static HOST: OnceLock<Box<Host>> = OnceLock::new();

/// Builds the host object and returns the pointer the engine is given.
pub fn init(_s: &Setup) -> *mut c_void {
    let table: &'static [*const c_void; HOST_SLOTS] = Box::leak(Box::new(vtable()));
    let host = HOST.get_or_init(|| {
        Box::new(Host {
            vtable: table.as_ptr(),
            event_manager: super::events::pointer(),
            _pad: [0; 0xB788 - 16],
        })
    });
    log!(
        "host object {:p}: {} vtable slots, event manager {:p}, {:#x} bytes",
        host.as_ref() as *const Host,
        HOST_SLOTS,
        host.event_manager,
        0xB788
    );
    host.as_ref() as *const Host as *mut c_void
}

pub fn log_summary() {
    let counts: Vec<SlotCount> = (0..HOST_SLOTS)
        .map(|i| {
            let total = COUNTS[i].load(Ordering::Relaxed);
            let delta = total - LAST[i].swap(total, Ordering::Relaxed);
            SlotCount {
                index: i,
                total,
                delta,
            }
        })
        .collect();
    for line in summary_lines("host", &counts, host_slot_name, 5) {
        log!("{line}");
    }
}

// ---------------------------------------------------------------- stubs

/// A logging stub for any slot: ignores its arguments and returns 0.
unsafe extern "system" fn stub<const N: usize>(
    _this: *mut c_void,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
) -> usize {
    record(N, a1, a2, a3, a4);
    0
}

/// A stub for a slot that returns f32 (XMM0): slots 38 and 69.
unsafe extern "system" fn stub_f32<const N: usize>(
    _this: *mut c_void,
    a1: usize,
    a2: usize,
    a3: usize,
) -> f32 {
    record(N, a1, a2, a3, 0);
    0.0
}

macro_rules! row {
    ($($n:expr),+ $(,)?) => {
        [ $( stub::<$n> as *const c_void ),+ ]
    };
}

fn vtable() -> [*const c_void; HOST_SLOTS] {
    let mut v: [*const c_void; HOST_SLOTS] = [std::ptr::null(); HOST_SLOTS];
    // Generic logging stubs for all 256 slots, in 16-wide rows so each
    // const generic is spelled out.
    let rows: [[*const c_void; 16]; 16] = [
        row![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        row![16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31],
        row![32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47],
        row![48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63],
        row![64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79],
        row![80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95],
        row![96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111],
        row![112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127],
        row![128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143],
        row![144, 145, 146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159],
        row![160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170, 171, 172, 173, 174, 175],
        row![176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 188, 189, 190, 191],
        row![192, 193, 194, 195, 196, 197, 198, 199, 200, 201, 202, 203, 204, 205, 206, 207],
        row![208, 209, 210, 211, 212, 213, 214, 215, 216, 217, 218, 219, 220, 221, 222, 223],
        row![224, 225, 226, 227, 228, 229, 230, 231, 232, 233, 234, 235, 236, 237, 238, 239],
        row![240, 241, 242, 243, 244, 245, 246, 247, 248, 249, 250, 251, 252, 253, 254, 255],
    ];
    for (i, r) in rows.iter().enumerate() {
        v[i * 16..i * 16 + 16].copy_from_slice(r);
    }
    // Real behaviour where the engine relies on it.
    v[0] = begin_frame as *const c_void;
    v[1] = end_frame as *const c_void;
    v[2] = resize as *const c_void;
    v[3] = set_game_state as *const c_void;
    v[4] = restart_game as *const c_void;
    v[6] = set_game_result as *const c_void;
    v[10] = get_game_event_manager as *const c_void;
    v[14] = set_player_look_control as *const c_void;
    v[15] = set_player_profile_game_specific as *const c_void;
    v[23] = update_launch_timer as *const c_void;
    v[32] = get_video_setting as *const c_void;
    v[33] = get_audio_setting as *const c_void;
    v[34] = get_player_profile as *const c_void;
    v[36] = get_input_state as *const c_void;
    v[37] = get_input_state_gamepad as *const c_void;
    v[38] = stub_f32::<38> as *const c_void;
    v[39] = set_input_state as *const c_void;
    v[46] = get_folder_path as *const c_void;
    v[47] = get_game_folder_path as *const c_void;
    v[48] = get_scenario_path_a as *const c_void;
    v[49] = get_scenario_path_w as *const c_void;
    v[51] = get_game_setting as *const c_void;
    v[52] = validate_cache_file as *const c_void;
    v[59] = return_true::<59> as *const c_void;
    v[60] = return_true::<60> as *const c_void;
    v[61] = return_true::<61> as *const c_void;
    v[68] = return_true::<68> as *const c_void;
    v[69] = stub_f32::<69> as *const c_void;
    v[88] = get_player_xuid as *const c_void;
    v[97] = chud_blend_color as *const c_void;
    v[116] = get_player_gamepad_mapping as *const c_void;
    v
}

// ---------------------------------------------------------------- real slots

unsafe extern "system" fn begin_frame(_this: *mut c_void) -> usize {
    record(0, 0, 0, 0, 0);
    0
}

unsafe extern "system" fn end_frame(
    _this: *mut c_void,
    swapchain: *mut c_void,
    _flags: *mut u32,
) -> usize {
    record(1, swapchain as usize, 0, 0, 0);
    super::FRAMES.fetch_add(1, Ordering::Relaxed);
    // Leave *flags untouched (it is an output the engine reads).
    panic_guard(1, move || gfx::on_end_frame(swapchain));
    0
}

unsafe extern "system" fn resize(_this: *mut c_void) -> usize {
    record(2, 0, 0, 0, 0);
    // The engine waits here; the window thread runs ResizeBuffers.
    // SAFETY: a blocking message to our window; the window thread never
    // waits on this thread without pumping, so this cannot deadlock.
    unsafe {
        SendMessageW(
            super::hwnd(),
            gfx::WM_APP_RESIZE,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        );
    }
    0
}

unsafe extern "system" fn set_game_state(_this: *mut c_void, state: i32) -> usize {
    record(3, state as usize, 0, 0, 0);
    log!(
        "set_game_state({state}){}",
        match state {
            0 => " (initial)",
            1 => " (map loaded)",
            5 => " (exit/restart)",
            8 => " (enter leaderboard)",
            9 => " (leave leaderboard)",
            _ => "",
        }
    );
    if let Ok(mut s) = super::STATES.lock() {
        s.push(state);
    }
    if state == 1 {
        super::mark(&super::STATE1_AT);
    }
    0
}

unsafe extern "system" fn restart_game(
    _this: *mut c_void,
    reason: i32,
    message: *const u8,
) -> usize {
    record(4, reason as usize, message as usize, 0, 0);
    let mut msg = String::new();
    if !message.is_null() && super::crash::readable(message as usize, 1) {
        let mut line = super::log::StackLine::new();
        super::crash::c_string(message as usize, &mut line);
        msg = line.text();
    }
    log!("restart_game(reason={reason}, message={msg:?})");
    // Hand off to the window thread and return at once (C19).
    // SAFETY: posts to our window; non-blocking.
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            Some(super::hwnd()),
            gfx::WM_APP_RESTART,
            WPARAM(reason as usize),
            LPARAM(0),
        );
    }
    0
}

unsafe extern "system" fn set_game_result(_this: *mut c_void, result: *mut c_void) -> usize {
    record(6, result as usize, 0, 0, 0);
    log!(
        "set_game_result (0x{:x} bytes) at {:.3} s",
        profile::GAME_RESULT_SIZE,
        super::now()
    );
    0
}

unsafe extern "system" fn get_game_event_manager(_this: *mut c_void) -> *mut c_void {
    record(10, 0, 0, 0, 0);
    super::events::pointer()
}

unsafe extern "system" fn set_player_look_control(
    _this: *mut c_void,
    player: i32,
    inverted: bool,
) -> usize {
    record(14, player as usize, inverted as usize, 0, 0);
    log!("set_player_look_control(player={player}, inverted={inverted})");
    0
}

unsafe extern "system" fn set_player_profile_game_specific(
    _this: *mut c_void,
    player: i32,
    _blob: *const u8,
) -> usize {
    record(15, player as usize, 0, 0, 0);
    0
}

unsafe extern "system" fn update_launch_timer(
    _this: *mut c_void,
    phase: i32,
    _progress: f32,
) -> usize {
    record(23, phase as usize, 0, 0, 0);
    0
}

unsafe extern "system" fn get_video_setting(_this: *mut c_void, out: *mut u8) -> usize {
    record(32, out as usize, 0, 0, 0);
    if !out.is_null() {
        // SAFETY: a 368-byte s_game_video_settings.
        unsafe { std::ptr::write_bytes(out, 0, profile::VIDEO_SETTINGS_SIZE) };
    }
    1
}

unsafe extern "system" fn get_audio_setting(_this: *mut c_void, out: *mut u8) -> usize {
    record(33, out as usize, 0, 0, 0);
    if !out.is_null() {
        // SAFETY: a 788-byte s_game_audio_settings.
        unsafe { std::ptr::write_bytes(out, 0, profile::AUDIO_SETTINGS_SIZE) };
    }
    1
}

unsafe extern "system" fn get_player_profile(_this: *mut c_void, xuid: u64) -> *const u8 {
    record(34, xuid as usize, 0, 0, 0);
    match super::setup() {
        Some(s) if xuid == s.xuid => profile_buffer().as_ptr(),
        _ => std::ptr::null(),
    }
}

/// The profile bytes, built once and kept for the run.
fn profile_buffer() -> &'static [u8] {
    static BUF: OnceLock<Vec<u8>> = OnceLock::new();
    BUF.get_or_init(|| {
        let mut p = profile::build_profile(&profile::ProfileSettings::default());
        if let Some(s) = super::setup() {
            for w in &s.args.set_profile {
                match w.apply(&mut p) {
                    Ok(()) => log!("profile override applied: {}", w.text),
                    Err(e) => log!("profile override skipped: {e}"),
                }
            }
        }
        p
    })
}

unsafe extern "system" fn get_input_state(_this: *mut c_void, player: i32, out: *mut u8) -> usize {
    record(36, player as usize, out as usize, 0, 0);
    if player == 0 {
        super::INPUT_POLLS.fetch_add(1, Ordering::Relaxed);
        super::mark(&super::FIRST_INPUT_AT);
    }
    panic_guard(36, move || super::input::fill(player, out, false)) as usize
}

unsafe extern "system" fn get_input_state_gamepad(
    _this: *mut c_void,
    player: i32,
    out: *mut u8,
) -> usize {
    record(37, player as usize, out as usize, 0, 0);
    panic_guard(37, move || super::input::fill(player, out, true)) as usize
}

unsafe extern "system" fn set_input_state(
    _this: *mut c_void,
    player: i32,
    rumble: *const u8,
) -> usize {
    record(39, player as usize, rumble as usize, 0, 0);
    panic_guard(39, move || super::input::rumble(player, rumble));
    0
}

/// Writes a wide path into `buf` (`len` characters including the zero).
fn write_wide_path(buf: *mut u16, len: usize, path: &str) -> bool {
    if buf.is_null() || len == 0 {
        return false;
    }
    let wide: Vec<u16> = path.encode_utf16().collect();
    if wide.len() + 1 > len {
        return false;
    }
    // SAFETY: the engine gave a buffer of `len` u16s; we write at most that.
    unsafe {
        std::ptr::copy_nonoverlapping(wide.as_ptr(), buf, wide.len());
        *buf.add(wide.len()) = 0;
    }
    true
}

unsafe extern "system" fn get_folder_path(
    _this: *mut c_void,
    kind: i32,
    buf: *mut u16,
    len: usize,
) -> usize {
    record(46, kind as usize, buf as usize, len, 0);
    let path = super::setup().and_then(|s| paths::game_folder_halo2(&s.engine_dir, kind));
    match path {
        Some(p) => write_wide_path(buf, len, &p) as usize,
        None => 0,
    }
}

unsafe extern "system" fn get_game_folder_path(
    _this: *mut c_void,
    kind: i32,
    buf: *mut u16,
    len: usize,
) -> usize {
    record(47, kind as usize, buf as usize, len, 0);
    let path = super::setup().and_then(|s| paths::game_folder(&s.engine_dir, kind));
    match path {
        Some(p) => write_wide_path(buf, len, &p) as usize,
        None => 0,
    }
}

unsafe extern "system" fn get_scenario_path_a(
    _this: *mut c_void,
    builtin: bool,
    buf: *mut u8,
    len: usize,
) -> usize {
    record(48, builtin as usize, buf as usize, len, 0);
    if buf.is_null() || len == 0 {
        return 0;
    }
    // SAFETY: the engine put a NUL-terminated relative path in `buf`.
    let rel: Vec<u8> = unsafe {
        let mut v = Vec::new();
        for i in 0..len {
            let b = *buf.add(i);
            if b == 0 {
                break;
            }
            v.push(b);
        }
        v
    };
    log!(
        "get_scenario_path_a(builtin={builtin}, {:?})",
        String::from_utf8_lossy(&rel)
    );
    let Some(root) = super::setup().map(|s| format!("{}\\", s.root)) else {
        return 0;
    };
    let out = paths::prefix_root(root.as_bytes(), &rel, len);
    match out {
        Some(o) => {
            // SAFETY: prefix_root kept it under `len`; add the terminator.
            unsafe {
                std::ptr::copy_nonoverlapping(o.as_ptr(), buf, o.len());
                *buf.add(o.len()) = 0;
            }
            1
        }
        None => {
            log!("get_scenario_path_a: path did not fit in {len} bytes");
            0
        }
    }
}

unsafe extern "system" fn get_scenario_path_w(
    _this: *mut c_void,
    builtin: bool,
    buf: *mut u16,
    len: usize,
) -> usize {
    record(49, builtin as usize, buf as usize, len, 0);
    if buf.is_null() || len == 0 {
        return 0;
    }
    // SAFETY: the engine put a NUL-terminated relative path in `buf`.
    let rel: Vec<u16> = unsafe {
        let mut v = Vec::new();
        for i in 0..len {
            let c = *buf.add(i);
            if c == 0 {
                break;
            }
            v.push(c);
        }
        v
    };
    log!(
        "get_scenario_path_w(builtin={builtin}, {:?})",
        String::from_utf16_lossy(&rel)
    );
    let Some(root) = super::setup().map(|s| format!("{}\\", s.root)) else {
        return 0;
    };
    let root: Vec<u16> = root.encode_utf16().collect();
    match paths::prefix_root(&root, &rel, len) {
        Some(o) => {
            // SAFETY: fits in `len`; add the terminator.
            unsafe {
                std::ptr::copy_nonoverlapping(o.as_ptr(), buf, o.len());
                *buf.add(o.len()) = 0;
            }
            1
        }
        None => {
            log!("get_scenario_path_w: path did not fit in {len} units");
            0
        }
    }
}

unsafe extern "system" fn get_game_setting(
    _this: *mut c_void,
    setting: i32,
    v1: *mut u8,
    _v2: *mut u64,
) -> usize {
    record(51, setting as usize, v1 as usize, 0, 0);
    // 5 enable_subtitle, 6 new_font_package.
    let value = match setting {
        5 => true,
        6 => super::setup().is_some_and(|s| s.args.host_fonts),
        _ => false,
    };
    if !v1.is_null() {
        // SAFETY: a bool output.
        unsafe { *v1 = value as u8 };
    }
    // Known settings return true (we answered them), others false.
    matches!(setting, 5 | 6) as usize
}

unsafe extern "system" fn validate_cache_file(_this: *mut c_void, which: i32) -> usize {
    record(52, which as usize, 0, 0, 0);
    1
}

unsafe extern "system" fn return_true<const N: usize>(
    _this: *mut c_void,
    a1: usize,
    a2: usize,
    a3: usize,
) -> usize {
    record(N, a1, a2, a3, 0);
    1
}

unsafe extern "system" fn get_player_xuid(
    _this: *mut c_void,
    out_xuid: *mut u64,
    name: *mut u16,
    size_bytes: i32,
    player: i32,
) -> usize {
    record(
        88,
        out_xuid as usize,
        name as usize,
        size_bytes as usize,
        player as usize,
    );
    let Some(s) = super::setup() else { return 0 };
    if player != 0 {
        return 0;
    }
    if !out_xuid.is_null() {
        // SAFETY: an XUID output.
        unsafe { *out_xuid = s.xuid };
    }
    if !name.is_null() && size_bytes > 0 {
        let cap = (size_bytes as usize / 2).max(1);
        let n = s.name.len().min(cap - 1);
        // SAFETY: the engine gave `size_bytes` bytes; we write at most
        // cap-1 characters and a terminator.
        unsafe {
            std::ptr::copy_nonoverlapping(s.name.as_ptr(), name, n);
            *name.add(n) = 0;
        }
    }
    1
}

unsafe extern "system" fn chud_blend_color(_this: *mut c_void, player: i32, rgba: u32) -> u32 {
    record(97, player as usize, rgba as usize, 0, 0);
    rgba
}

unsafe extern "system" fn get_player_gamepad_mapping(_this: *mut c_void, xuid: u64) -> *const u8 {
    record(116, xuid as usize, 0, 0, 0);
    match super::setup() {
        Some(s) if xuid == s.xuid => mapping_buffer(s.args.pad_map).as_ptr(),
        _ => std::ptr::null(),
    }
}

fn mapping_buffer(kind: PadMap) -> &'static [u8] {
    static ZERO: OnceLock<[u8; profile::GAMEPAD_MAPPING_SIZE]> = OnceLock::new();
    static H2: OnceLock<[u8; profile::GAMEPAD_MAPPING_SIZE]> = OnceLock::new();
    match kind {
        PadMap::Zero => ZERO.get_or_init(|| profile::gamepad_mapping(PadMap::Zero)),
        PadMap::Halo2 => H2.get_or_init(|| profile::gamepad_mapping(PadMap::Halo2)),
    }
}
