//! The game event manager the host hands back from slot 10. It is the
//! Xbox Live telemetry sink: 151 slots, every one a stub that returns 0
//! (and nothing out), except slot 147 `GetGUID`, which returns a pointer
//! to a fixed non-zero GUID (host-interface section 7). The object is a
//! vtable pointer with that GUID beside it; it is static and outlives the
//! run.
//!
//! The slots' overloads follow MSVC order: slot 4 is the 20-argument
//! `Base`, slot 5 the 2-argument one. Both are the same stub here, so the
//! order only matters for the names in the log.

use std::sync::atomic::{AtomicU64, Ordering};

use windows::core::GUID;

use super::log::log;
use crate::slots::{event_name, summary_lines, SlotCount, EVENT_GET_GUID, EVENT_SLOTS};

static COUNTS: [AtomicU64; EVENT_SLOTS] = [const { AtomicU64::new(0) }; EVENT_SLOTS];
static LAST: [AtomicU64; EVENT_SLOTS] = [const { AtomicU64::new(0) }; EVENT_SLOTS];

/// A fixed, non-zero session GUID. Made once at start.
static GUID_BYTES: [u8; 16] = [
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x01,
];

fn hit(slot: usize) {
    let n = COUNTS[slot].fetch_add(1, Ordering::Relaxed);
    if n == 0 && super::setup().is_some_and(|s| s.args.diag) {
        log!("event-manager slot {slot} {} first call", event_name(slot));
    }
}

/// A stub for every event slot. The engine passes all arguments; we read
/// none and return 0.
unsafe extern "system" fn stub<const N: usize>(
    _this: *mut core::ffi::c_void,
    _a1: usize,
    _a2: usize,
    _a3: usize,
) -> usize {
    hit(N);
    0
}

/// Slot 147: return the address of our static GUID.
unsafe extern "system" fn get_guid(_this: *mut core::ffi::c_void) -> *const GUID {
    hit(EVENT_GET_GUID);
    GUID_BYTES.as_ptr() as *const GUID
}

#[repr(C)]
struct EventManager {
    vtable: *const *const core::ffi::c_void,
}
// SAFETY: the object is immutable after init; the engine calls its slots
// from its own threads.
unsafe impl Sync for EventManager {}
unsafe impl Send for EventManager {}

/// A 151-entry vtable in a wrapper that is safe to share: the pointers are
/// `fn` addresses, set once and only read.
struct VTable([*const core::ffi::c_void; EVENT_SLOTS]);
// SAFETY: the array holds only function addresses, never mutated after
// construction.
unsafe impl Sync for VTable {}
unsafe impl Send for VTable {}

macro_rules! row {
    ($($n:expr),+ $(,)?) => {
        [ $( stub::<$n> as *const core::ffi::c_void ),+ ]
    };
}

/// All 151 vtable entries, overridable where a real function is needed.
fn vtable() -> [*const core::ffi::c_void; EVENT_SLOTS] {
    let mut v: [*const core::ffi::c_void; EVENT_SLOTS] = [std::ptr::null(); EVENT_SLOTS];
    let r0: [*const core::ffi::c_void; 76] = row![
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
        48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70,
        71, 72, 73, 74, 75
    ];
    let r1: [*const core::ffi::c_void; 75] = row![
        76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98,
        99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116,
        117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134,
        135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150
    ];
    v[..76].copy_from_slice(&r0);
    v[76..].copy_from_slice(&r1);
    v[EVENT_GET_GUID] = get_guid as *const core::ffi::c_void;
    v
}

/// The pointer the host returns from slot 10.
pub fn pointer() -> *mut core::ffi::c_void {
    use std::sync::OnceLock;
    static TABLE: OnceLock<VTable> = OnceLock::new();
    static OBJECT: OnceLock<EventManager> = OnceLock::new();
    let table = TABLE.get_or_init(|| VTable(vtable()));
    let obj = OBJECT.get_or_init(|| EventManager {
        vtable: table.0.as_ptr(),
    });
    obj as *const EventManager as *mut core::ffi::c_void
}

pub fn log_summary() {
    let counts: Vec<SlotCount> = (0..EVENT_SLOTS)
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
    for line in summary_lines("events", &counts, event_name, 6) {
        log!("{line}");
    }
}
