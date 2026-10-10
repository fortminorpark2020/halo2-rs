//! The host object (`i_game_manager`) the engine calls back into. It is a
//! vtable pointer, the event-manager pointer at +8, and padding out to
//! 0xB788 bytes (MCC's size, in case the engine writes into it). The
//! vtable has 256 entries: the slots that need real behaviour
//! (launch-sequence.verify C2) and, for every other slot, a distinct
//! logging stub that returns 0. Slots 38 and 69 return an f32 (0.0) so
//! XMM0 is set. Every function uses `extern "system"` with an explicit
//! `this` first argument, and the real ones catch panics so none crosses
//! into the engine.
//!
//! The network slots 41 to 44 go to `crate::net` (the relay, or with no
//! `--session` a logger that sends nothing and receives nothing).
//!
//! Fonts are never served in milestone 1, so the font slots answer "no":
//! the probes 59, 60 and 68 say yes only with `--host-fonts` (setting 6),
//! and 61, 62, 64 and 67 always say no (host-interface.verify C25,
//! launch-sequence.verify C2). They log the font-name argument, which may
//! be a string, null or a small id (host-interface.verify A5/E11).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

use super::gfx;
use super::log::log;
use super::Setup;
use crate::net::{self, Net, RecvPort};
use crate::paths;
use crate::profile;
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
            "host slot {slot} {} first call: a1={a1:#x} a2={a2:#x} a3={a3:#x} a4={a4:#x} (t {}) from {}",
            host_slot_name(slot),
            super::tid(),
            super::crash::callers()
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
    super::guarded(f).unwrap_or_else(|| {
        log!(
            "host slot {slot} {} panicked (returned the default)",
            host_slot_name(slot)
        );
        T::default()
    })
}

fn host_fonts() -> bool {
    super::setup().is_some_and(|s| s.args.host_fonts)
}

/// Logs each of the first few distinct values a slot is asked about that
/// we have no answer for (an XUID that is not ours, a player that is not
/// local, a font name...).
struct Oddities {
    seen: Mutex<Vec<(usize, u64)>>,
    limit: usize,
}

impl Oddities {
    const fn new(limit: usize) -> Oddities {
        Oddities {
            seen: Mutex::new(Vec::new()),
            limit,
        }
    }

    /// True the first time (slot, value) is seen, while under the limit.
    fn fresh(&self, slot: usize, value: u64) -> bool {
        let Ok(mut seen) = self.seen.try_lock() else {
            return false;
        };
        if seen.len() >= self.limit || seen.contains(&(slot, value)) {
            return false;
        }
        seen.push((slot, value));
        true
    }
}

static ODD_XUIDS: Oddities = Oddities::new(8);
static ODD_QUERIES: Oddities = Oddities::new(8);
static FONT_NAMES: Oddities = Oddities::new(16);

fn note_other_xuid(slot: usize, xuid: u64) {
    if ODD_XUIDS.fresh(slot, xuid) {
        let ours = super::setup().map_or(0, |s| s.xuid);
        log!(
            "host slot {slot} {} was asked for XUID {xuid:#018x}, not ours ({ours:#018x}); returned null",
            host_slot_name(slot)
        );
    }
}

/// A font name argument: a C string, null, or a small id.
fn note_font_name(slot: usize, v: usize) {
    if !FONT_NAMES.fresh(slot, v as u64) {
        return;
    }
    let what = if v == 0 {
        "null".to_string()
    } else if v < 0x10000 {
        format!("id {v}")
    } else if super::crash::readable(v, 1) {
        let mut line = super::log::StackLine::new();
        super::crash::c_string(v, &mut line);
        format!("{:?} at {v:#x}", line.text())
    } else {
        format!("{v:#x} (unreadable)")
    };
    log!(
        "host slot {slot} {} font name: {what}",
        host_slot_name(slot)
    );
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
pub fn init(s: &Setup) -> *mut c_void {
    let table: &'static [*const c_void; HOST_SLOTS] = Box::leak(Box::new(vtable()));
    for &(slot, v) in &s.args.slot_return {
        // Only the generic stubs read it; a slot with its own function
        // keeps its behaviour.
        RETURNS[slot].store(v, Ordering::Relaxed);
        log!(
            "--slot-return: host slot {slot} {} returns {v:#x} if it is a logging stub",
            host_slot_name(slot)
        );
    }
    super::events::set_returns(&s.args.event_return);
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

// ---------------------------------------------------------------- network

static NET: OnceLock<Net> = OnceLock::new();

pub fn net() -> Option<&'static Net> {
    NET.get()
}

/// Sets up the network slots: the relay for `--session`, else the solo
/// logger. Returns at once; the caller waits for the relay.
pub fn start_net(s: &Setup) {
    let log: net::Log = std::sync::Arc::new(|l: &str| log!("{l}"));
    let settings = |machines: Vec<u64>| net::Settings {
        recv_port: s.args.recv_port,
        send_return: s.args.send_return,
        self_send: s.args.self_send,
        machines,
    };
    let n = match &s.session {
        None => Net::solo(settings(Vec::new()), log),
        Some((sess, me)) => {
            let joined = sess.relay_addr().and_then(|addr| {
                log!(
                    "relay {addr}: joining room {:#x} as {:#018x}",
                    sess.room,
                    sess.machines[*me]
                );
                Net::connect(
                    addr,
                    sess.room,
                    sess.machines[*me],
                    sess.key.as_deref(),
                    settings(sess.machines.clone()),
                    log.clone(),
                )
            });
            match joined {
                Ok(n) => n,
                Err(e) => {
                    log!("relay: {e}; the engine's sends will fail");
                    Net::solo(settings(sess.machines.clone()), log)
                }
            }
        }
    };
    log!(
        "network slots: receive port argument read as {:?}, sends return {:?}",
        n.settings().recv_port,
        n.settings().send_return
    );
    let _ = NET.set(n);
}

static NET_ODD: Oddities = Oddities::new(16);

/// Slots 41 and 42: `(network_id id, const char* buf, u32 len, u32 port)
/// -> u32` (libmcc game_manager.h; research 2.1).
fn net_send(reliable: bool, id: u64, buf: *const u8, len: u32, port: u32) -> u32 {
    let slot = if reliable { 42 } else { 41 };
    let Some(n) = NET.get() else { return 0 };
    let size = len as usize;
    if size > h2relay::MAX_PAYLOAD || (buf.is_null() && size > 0) {
        if NET_ODD.fresh(slot, size as u64) {
            log!("net: slot {slot} send of {size} bytes from {buf:p} to {id:#018x} port {port} not read (over {} bytes, or no buffer)", h2relay::MAX_PAYLOAD);
        }
        return 0;
    }
    let data: &[u8] = if size == 0 {
        &[]
    } else {
        // SAFETY: the engine hands us `len` readable bytes at `buf` for the
        // length of the call; they are copied before we return.
        unsafe { std::slice::from_raw_parts(buf, size) }
    };
    n.send(reliable, id, data, port)
}

unsafe extern "system" fn network_sendto_unreliable(
    _this: *mut c_void,
    id: u64,
    buf: *const u8,
    len: u32,
    port: u32,
) -> u32 {
    record(41, id as usize, buf as usize, len as usize, port as usize);
    panic_guard(41, || net_send(false, id, buf, len, port))
}

unsafe extern "system" fn network_sendto_reliable(
    _this: *mut c_void,
    id: u64,
    buf: *const u8,
    len: u32,
    port: u32,
) -> u32 {
    record(42, id as usize, buf as usize, len as usize, port as usize);
    panic_guard(42, || net_send(true, id, buf, len, port))
}

/// Slot 43: `(char* buf, u32 len, network_id* id_out, u16 port) -> u32`,
/// the bytes copied or 0. The engine polls port 1000 (its game links) and
/// 1002 (out-of-band transport and session messages, the first contact),
/// and fills the rest of the sender's address itself (static read of
/// 1.3528 on the owner's PC). `--recv-port` can still read the fourth
/// argument differently; only `pointer` writes through it.
unsafe extern "system" fn network_recvfrom(
    _this: *mut c_void,
    buf: *mut u8,
    len: u32,
    id_out: *mut u64,
    a4: usize,
) -> u32 {
    record(43, buf as usize, len as usize, id_out as usize, a4);
    note_recv_caller(a4);
    panic_guard(43, || {
        let Some(n) = NET.get() else { return 0 };
        if buf.is_null() || len == 0 {
            return 0;
        }
        // SAFETY: the engine's receive buffer of `len` bytes, ours to fill
        // during the call.
        let out = unsafe { std::slice::from_raw_parts_mut(buf, len as usize) };
        let Some((src, port, size)) = n.recv(out, a4 as u64) else {
            return 0;
        };
        if !id_out.is_null() {
            // SAFETY: the engine's out parameter for the sender's id.
            unsafe { id_out.write_unaligned(src) };
        }
        if n.settings().recv_port == RecvPort::Pointer && a4 > 0xFFFF {
            // SAFETY: only with --recv-port pointer, which says a4 is the
            // engine's u32 port out parameter.
            unsafe { (a4 as *mut u32).write_unaligned(port) };
        }
        size as u32
    })
}

/// Logs which engine code polls each new value of the receive call's
/// fourth argument (the first eight values).
fn note_recv_caller(a4: usize) {
    static SEEN: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    let fresh = SEEN.lock().is_ok_and(|mut v| {
        if v.len() < 8 && !v.contains(&a4) {
            v.push(a4);
            true
        } else {
            false
        }
    });
    if fresh {
        log!(
            "net: slot 43 with a4={a4:#x} polled from {}",
            super::crash::callers()
        );
    }
}

/// Slot 44: `network_send(buf)`, meaning unknown: logged.
unsafe extern "system" fn network_send(
    _this: *mut c_void,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
) -> usize {
    record(44, a1, a2, a3, a4);
    if let Some(n) = NET.get() {
        n.note_slot44(a1, a2, a3);
    }
    0
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

/// `--slot-return`: what a logging stub returns instead of 0.
static RETURNS: [AtomicU64; HOST_SLOTS] = [const { AtomicU64::new(0) }; HOST_SLOTS];

/// A logging stub for any slot: ignores its arguments and returns 0, or
/// what `--slot-return` set.
unsafe extern "system" fn stub<const N: usize>(
    _this: *mut c_void,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
) -> usize {
    record(N, a1, a2, a3, a4);
    RETURNS[N].load(Ordering::Relaxed) as usize
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

/// Generic logging stubs for all 256 slots, in 16-wide rows so each const
/// generic is spelled out.
fn generic_stubs() -> [*const c_void; HOST_SLOTS] {
    let mut v: [*const c_void; HOST_SLOTS] = [std::ptr::null(); HOST_SLOTS];
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
    v
}

fn vtable() -> [*const c_void; HOST_SLOTS] {
    let mut v = generic_stubs();
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
    v[41] = network_sendto_unreliable as *const c_void;
    v[42] = network_sendto_reliable as *const c_void;
    v[43] = network_recvfrom as *const c_void;
    v[44] = network_send as *const c_void;
    v[46] = get_folder_path as *const c_void;
    v[47] = get_game_folder_path as *const c_void;
    v[48] = get_scenario_path_a as *const c_void;
    v[49] = get_scenario_path_w as *const c_void;
    v[51] = get_game_setting as *const c_void;
    v[52] = validate_cache_file as *const c_void;
    v[59] = font_probe::<59> as *const c_void;
    v[60] = font_probe::<60> as *const c_void;
    v[61] = font_test_string as *const c_void;
    v[62] = font_precache_character as *const c_void;
    v[63] = font_get_texture as *const c_void;
    v[64] = font_test_char as *const c_void;
    v[65] = font_get_kerning_pair_offset as *const c_void;
    v[67] = font_set_selection as *const c_void;
    v[68] = font_probe::<68> as *const c_void;
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
    flags: *mut u32,
) -> usize {
    record(1, swapchain as usize, flags as usize, 0, 0);
    super::FRAMES.fetch_add(1, Ordering::Relaxed);
    // Leave *flags untouched (libmcc marks it an output), but log what the
    // engine left there (launch-sequence.verify C15/V12).
    panic_guard(1, move || {
        note_end_frame_flags(flags);
        gfx::on_end_frame(swapchain)
    });
    0
}

const FLAGS_NONE: u64 = u64::MAX;
static FLAGS_PTR: AtomicUsize = AtomicUsize::new(0);
static FLAGS_PTR_READABLE: AtomicBool = AtomicBool::new(false);
static FLAGS_VALUE: AtomicU64 = AtomicU64::new(FLAGS_NONE);
static FLAGS_LOGS: AtomicU32 = AtomicU32::new(0);

/// Logs end_frame's `*flags` on the first call and whenever it changes
/// (the first 20 changes).
fn note_end_frame_flags(flags: *mut u32) {
    let p = flags as usize;
    if FLAGS_PTR.swap(p, Ordering::Relaxed) != p {
        FLAGS_PTR_READABLE.store(super::crash::readable(p, 4), Ordering::Relaxed);
    }
    let v = if p == 0 || !FLAGS_PTR_READABLE.load(Ordering::Relaxed) {
        FLAGS_NONE - 1 - (p == 0) as u64
    } else {
        // SAFETY: checked readable for this pointer; a read only.
        unsafe { std::ptr::read_volatile(flags) as u64 }
    };
    if FLAGS_VALUE.swap(v, Ordering::Relaxed) != v
        && FLAGS_LOGS.fetch_add(1, Ordering::Relaxed) < 20
    {
        let shown = match v {
            x if x == FLAGS_NONE - 2 => "(null pointer)".to_string(),
            x if x == FLAGS_NONE - 1 => "(unreadable)".to_string(),
            x => format!("{x:#x}"),
        };
        log!(
            "end_frame flags at {flags:p} = {shown} (frame {})",
            super::FRAMES.load(Ordering::Relaxed)
        );
    }
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
    panic_guard(3, move || {
        log!(
            "set_game_state({state}){} from {}",
            match state {
                0 => " (initial)",
                1 => " (map loaded)",
                5 => " (exit/restart)",
                8 => " (enter leaderboard)",
                9 => " (leave leaderboard)",
                _ => "",
            },
            super::crash::callers()
        );
        if let Ok(mut s) = super::STATES.lock() {
            s.push(state);
        }
        if state == 1 {
            super::mark(&super::STATE1_AT);
            crate::live::tell(crate::live::Engine::MapLoaded);
        }
    });
    0
}

unsafe extern "system" fn restart_game(
    _this: *mut c_void,
    reason: i32,
    message: *const u8,
) -> usize {
    record(4, reason as usize, message as usize, 0, 0);
    panic_guard(4, move || {
        let mut msg = String::new();
        if !message.is_null() && super::crash::readable(message as usize, 1) {
            let mut line = super::log::StackLine::new();
            super::crash::c_string(message as usize, &mut line);
            msg = line.text();
        }
        log!(
            "restart_game(reason={reason}, message={msg:?}) from {}",
            super::crash::callers()
        );
    });
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
    let block = result as usize;
    panic_guard(6, move || game_result(block));
    0
}

/// What `set_game_result` does with the block at `block`: log what it says,
/// keep a copy if asked, and tell the match's thread (see `crate::live`).
fn game_result(block: usize) {
    let bytes: &[u8] = if block == 0 {
        &[]
    } else {
        // SAFETY: the engine hands us a GAME_RESULT_SIZE block for the
        // length of the call; it is only read, and copied before we return.
        unsafe { std::slice::from_raw_parts(block as *const u8, profile::GAME_RESULT_SIZE) }
    };
    for line in crate::results::describe(bytes) {
        log!("result: {line}");
    }
    // In Slayer, each player's kills are their score plus their suicides:
    // say whether the inferred offsets bear that out.
    let slayer = super::variant().is_some_and(crate::lobby::names::slayer);
    if slayer {
        let line = crate::results::check_kills(&crate::results::players(bytes));
        log!("result: {line}");
    }
    // H2LAUNCH_RESULT_DUMP=<folder>: keep a copy of the block on this PC,
    // for working out more of its layout. It is the engine's data: never
    // commit or upload it.
    if let (Some(dir), false) = (std::env::var_os("H2LAUNCH_RESULT_DUMP"), bytes.is_empty()) {
        let path = std::path::Path::new(&dir).join(format!(
            "result-{}-{:.0}.bin",
            std::process::id(),
            super::now() * 1000.0
        ));
        match std::fs::write(&path, bytes) {
            Ok(()) => log!("result block kept in {}", path.display()),
            Err(e) => log!("result block not kept: {e}"),
        }
    }
    // The game ended (every PC gets this at the end), unless we asked the
    // engine to quit: then it is our own result as a quitter (seen on the
    // owner's PC), and closing says we left.
    let quitting = matches!(
        super::QUIT.load(Ordering::SeqCst),
        super::QUIT_USER | super::QUIT_CONSOLE
    );
    if !quitting {
        // Kills reach the server only when this game bore them out (see
        // `results::COUNTS_SEEN`).
        let players = crate::results::players(bytes);
        let kills = crate::results::kills_hold(&players, slayer);
        if !kills && !crate::results::COUNTS_SEEN {
            log!("result: kills not sent (not borne out by this game)");
        }
        let players = players.iter().map(|p| p.for_server(kills)).collect();
        crate::live::tell(crate::live::Engine::Ended(players));
    }
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
    blob: *const u8,
) -> usize {
    record(15, player as usize, blob as usize, 0, 0);
    // The engine's own 0x100-byte per-game settings block (HaloX keeps it
    // in the profile at 0x310). Logged once to see what Halo 2 puts there.
    static LOGGED: AtomicBool = AtomicBool::new(false);
    let p = blob as usize;
    if !LOGGED.swap(true, Ordering::Relaxed)
        && super::crash::readable(p, profile::GAME_SPECIFIC_SIZE)
    {
        panic_guard(15, move || {
            // SAFETY: checked readable for the whole block; a read only.
            let b =
                unsafe { std::slice::from_raw_parts(p as *const u8, profile::GAME_SPECIFIC_SIZE) };
            log!(
                "set_player_profile_game_specific(player {player}), the engine's settings block:\n{}",
                crate::util::hexdump(b, 0)
            );
        });
    }
    0
}

static LOAD_PHASE: AtomicI32 = AtomicI32::new(i32::MIN);
static LOAD_PROGRESS: AtomicU32 = AtomicU32::new(0);
static LOAD_LOGS: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn update_launch_timer(
    _this: *mut c_void,
    phase: i32,
    progress: f32,
) -> usize {
    record(23, phase as usize, 0, 0, 0);
    LOAD_PROGRESS.store(progress.to_bits(), Ordering::Relaxed);
    if LOAD_PHASE.swap(phase, Ordering::Relaxed) != phase
        && LOAD_LOGS.fetch_add(1, Ordering::Relaxed) < 30
    {
        log!("update_launch_timer: phase {phase}, progress {progress:.3}");
    }
    0
}

/// The last loading phase and progress the engine reported, for the
/// summaries (a load that stalls shows where).
pub fn load_line() -> String {
    match LOAD_PHASE.load(Ordering::Relaxed) {
        i32::MIN => "load=none".into(),
        phase => format!(
            "load={phase}/{:.3}",
            f32::from_bits(LOAD_PROGRESS.load(Ordering::Relaxed))
        ),
    }
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
    panic_guard(34, move || match super::setup() {
        Some(s) if xuid == s.xuid => profile_buffer().as_ptr() as usize,
        _ => {
            note_other_xuid(34, xuid);
            0
        }
    }) as *const u8
}

/// The profile bytes, built once and kept for the run.
fn profile_buffer() -> &'static [u8] {
    static BUF: OnceLock<Vec<u8>> = OnceLock::new();
    BUF.get_or_init(|| {
        let mut ps = match super::setup() {
            Some(s) => profile::ProfileSettings::from_controls(&s.controls),
            None => profile::ProfileSettings::default(),
        };
        ps.key_bindings = super::setup().is_none_or(|s| s.args.key_bindings);
        log!(
            "player profile ({:#x} bytes): stick look sensitivity {} (1-10; the scale is an estimate), mouse sensitivity {}, zoom/vehicle look multipliers {}/{}, volumes {}, FOV 0 (game default), look inverted {} (mouse {}), auto look centering {}, vibration {}, keyboard and mouse bindings {}",
            profile::PROFILE_SIZE,
            ps.look_sensitivity,
            ps.mouse_sensitivity,
            ps.zoom_look_multiplier,
            ps.vehicle_look_multiplier,
            ps.volume,
            ps.look_inverted,
            ps.mouse_inverted,
            ps.auto_center,
            ps.vibration,
            if ps.key_bindings {
                "halo2.dll's own keys plus MCC's (an estimate)"
            } else {
                "empty (--no-key-bindings)"
            }
        );
        let mut p = profile::build_profile(&ps);
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
    panic_guard(46, move || {
        let path = super::setup().and_then(|s| paths::game_folder_halo2(&s.engine_dir, kind));
        match path {
            Some(p) => write_wide_path(buf, len, &p) as usize,
            None => 0,
        }
    })
}

unsafe extern "system" fn get_game_folder_path(
    _this: *mut c_void,
    kind: i32,
    buf: *mut u16,
    len: usize,
) -> usize {
    record(47, kind as usize, buf as usize, len, 0);
    panic_guard(47, move || {
        let path = super::setup().and_then(|s| paths::game_folder(&s.engine_dir, kind));
        match path {
            Some(p) => write_wide_path(buf, len, &p) as usize,
            None => 0,
        }
    })
}

/// A path in the ANSI code page, which is what the engine's `char*` file
/// calls use; None (logged) when the path has characters that code page
/// cannot hold.
fn ansi_path(path: &str) -> Option<Vec<u8>> {
    if path.is_ascii() {
        return Some(path.as_bytes().to_vec());
    }
    use windows::Win32::Globalization::{WideCharToMultiByte, CP_ACP, WC_NO_BEST_FIT_CHARS};
    let wide: Vec<u16> = path.encode_utf16().collect();
    let mut used_default = windows::core::BOOL(0);
    // SAFETY: a size query, then a conversion into our buffer.
    let out = unsafe {
        let n = WideCharToMultiByte(
            CP_ACP,
            WC_NO_BEST_FIT_CHARS,
            &wide,
            None,
            windows::core::PCSTR::null(),
            None,
        );
        if n <= 0 {
            None
        } else {
            let mut buf = vec![0u8; n as usize];
            let m = WideCharToMultiByte(
                CP_ACP,
                WC_NO_BEST_FIT_CHARS,
                &wide,
                Some(&mut buf),
                windows::core::PCSTR::null(),
                Some(&mut used_default),
            );
            (m > 0).then(|| {
                buf.truncate(m as usize);
                buf
            })
        }
    };
    match out {
        Some(b) if !used_default.as_bool() => Some(b),
        _ => {
            static WARNED: AtomicBool = AtomicBool::new(false);
            if !WARNED.swap(true, Ordering::Relaxed) {
                log!("the MCC folder {path:?} has characters the ANSI code page cannot hold; get_scenario_path_a fails (move MCC to a plain-ASCII folder)");
            }
            None
        }
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
    panic_guard(48, move || {
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
        let Some(root) = super::setup().and_then(|s| ansi_path(&format!("{}\\", s.root))) else {
            return 0;
        };
        match paths::prefix_root(&root, &rel, len) {
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
    })
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
    panic_guard(49, move || {
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
    })
}

unsafe extern "system" fn get_game_setting(
    _this: *mut c_void,
    setting: i32,
    v1: *mut u8,
    _v2: *mut u64,
) -> usize {
    record(51, setting as usize, v1 as usize, 0, 0);
    // 5 enable_subtitle, 6 new_font_package. As HaloX: answer only those
    // two (write *v1, return true); for any other setting return false and
    // leave *v1 alone, in case the engine put its default there.
    let value = match setting {
        5 => true,
        6 => host_fonts(),
        _ => {
            if ODD_QUERIES.fresh(51, setting as u64) {
                log!("get_game_setting({setting}): not a setting we know; returned false");
            }
            return 0;
        }
    };
    if !v1.is_null() {
        // SAFETY: a bool output.
        unsafe { *v1 = value as u8 };
    }
    1
}

unsafe extern "system" fn validate_cache_file(_this: *mut c_void, which: i32) -> usize {
    record(52, which as usize, 0, 0, 0);
    1
}

/// Slots 59, 60 and 68: "font system ready" probes. Yes only when setting
/// 6 (`new_font_package`) says the host draws text.
unsafe extern "system" fn font_probe<const N: usize>(_this: *mut c_void) -> usize {
    record(N, 0, 0, 0, 0);
    host_fonts() as usize
}

/// Slot 61: can this string be drawn? Always no: no fonts are served.
/// The scale is in XMM3 and the font name is the fifth argument.
unsafe extern "system" fn font_test_string(
    _this: *mut c_void,
    text: *const u16,
    size: i32,
    _scale: f32,
    font_name: usize,
) -> usize {
    record(61, text as usize, size as usize, 0, font_name);
    note_font_name(61, font_name);
    0
}

/// Slot 62: rasterise a glyph. Never done here.
unsafe extern "system" fn font_precache_character(
    _this: *mut c_void,
    ch: u16,
    out: *mut u8,
    size: i32,
    _scale: f32,
    font_name: usize,
) -> usize {
    record(62, ch as usize, out as usize, size as usize, font_name);
    note_font_name(62, font_name);
    0
}

/// Slot 63: the atlas texture for a glyph from slot 62; there is none.
unsafe extern "system" fn font_get_texture(_this: *mut c_void, texture: i32) -> *mut c_void {
    record(63, texture as usize, 0, 0, 0);
    std::ptr::null_mut()
}

/// Slot 64: can this glyph be drawn? No.
unsafe extern "system" fn font_test_char(
    _this: *mut c_void,
    ch: u16,
    size: i32,
    _scale: f32,
    font_name: usize,
) -> usize {
    record(64, ch as usize, size as usize, 0, font_name);
    note_font_name(64, font_name);
    0
}

/// Slot 65: kerning between two glyphs: 0.
unsafe extern "system" fn font_get_kerning_pair_offset(
    _this: *mut c_void,
    left: u16,
    right: u16,
    size: i32,
    _scale: f32,
    font_name: usize,
) -> i32 {
    record(65, left as usize, right as usize, size as usize, font_name);
    note_font_name(65, font_name);
    0
}

/// Slot 67: select a font and report its metrics. No (metrics untouched).
unsafe extern "system" fn font_set_selection(
    _this: *mut c_void,
    size: i32,
    _scale: f32,
    font_name: usize,
    _ascender: *mut u16,
    _descender: *mut u16,
) -> usize {
    record(67, size as usize, 0, font_name, 0);
    note_font_name(67, font_name);
    0
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
    panic_guard(88, move || {
        let Some(s) = super::setup() else { return 0 };
        if player != 0 {
            if ODD_QUERIES.fresh(88, player as u64) {
                log!("get_player_xuid(player {player}): not a local player; returned false");
            }
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
    })
}

unsafe extern "system" fn chud_blend_color(_this: *mut c_void, player: i32, rgba: u32) -> u32 {
    record(97, player as usize, rgba as usize, 0, 0);
    rgba
}

unsafe extern "system" fn get_player_gamepad_mapping(_this: *mut c_void, xuid: u64) -> *const u8 {
    record(116, xuid as usize, 0, 0, 0);
    panic_guard(116, move || match super::setup() {
        Some(s) if xuid == s.xuid => mapping_buffer(s).as_ptr() as usize,
        _ => {
            note_other_xuid(116, xuid);
            0
        }
    }) as *const u8
}

/// The gamepad mapping, built once from the run's button layout (or all
/// zero with `--pad-map zero`) and kept for the run.
fn mapping_buffer(s: &Setup) -> &'static [u8] {
    static BUF: OnceLock<[u8; profile::GAMEPAD_MAPPING_SIZE]> = OnceLock::new();
    BUF.get_or_init(|| {
        let m = s.args.pad_map.mapping(s.controls.buttons);
        let which = match s.args.pad_map {
            crate::controls::PadMap::Layout => s.controls.buttons.name(),
            crate::controls::PadMap::Zero => "all zero (--pad-map zero)",
        };
        log!(
            "gamepad mapping: {which}: {}",
            crate::controls::describe_mapping(&m)
        );
        m
    })
}
