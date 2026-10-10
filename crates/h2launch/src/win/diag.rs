//! `--diag` only: two engine values about keyboard polling, read (never
//! written) once a second at known offsets in halo2.dll's data section
//! (launch-sequence.verify V24, host-interface.verify C15/E8):
//!
//! - a flag that picks which branch the engine's key poller takes: 0 means
//!   it asks Windows for key state on its own thread (which is what
//!   AttachThreadInput is meant to help), non-zero means it reads a key
//!   gate array that something else fills;
//! - that gate array, 256 bytes indexed by virtual key.
//!
//! Reading data does not call engine code, so this stays within the
//! launcher's rule of no patches, hooks or calls into the engine. A line is
//! logged when the values change, and at least every 10 s.

use std::sync::atomic::Ordering;

use super::crash::readable;
use super::log::log;
use crate::expected::{DIAG_KEY_GATE_RVA, DIAG_KEY_POLLER_FLAG_RVA};

#[derive(Default)]
pub struct KeyDiag {
    next: f64,
    last: String,
    logged_at: f64,
    announced: bool,
}

impl KeyDiag {
    pub fn tick(&mut self, t: f64) {
        if t < self.next {
            return;
        }
        self.next = t + 1.0;
        let base = super::HALO2_BASE.load(Ordering::SeqCst);
        if base == 0 {
            return;
        }
        if !self.announced {
            self.announced = true;
            log!(
                "diag: reading the key poller flag (halo2+{DIAG_KEY_POLLER_FLAG_RVA:#x}) and key gate array (halo2+{DIAG_KEY_GATE_RVA:#x}) once a second, read only{}",
                if super::BUILD_MATCHES.load(Ordering::SeqCst) {
                    ""
                } else {
                    " (halo2.dll is NOT the researched build, so these offsets may mean nothing)"
                }
            );
        }
        let line = read_line(base);
        if line != self.last || t - self.logged_at >= 10.0 {
            log!("diag: {line}");
            self.last = line;
            self.logged_at = t;
        }
    }
}

fn read_line(base: usize) -> String {
    let flag_at = base + DIAG_KEY_POLLER_FLAG_RVA as usize;
    let gate_at = base + DIAG_KEY_GATE_RVA as usize;
    let flag = if readable(flag_at, 4) {
        // SAFETY: checked readable; a plain read of engine data.
        format!("{:#x}", unsafe {
            std::ptr::read_volatile(flag_at as *const u32)
        })
    } else {
        "unreadable".into()
    };
    if !readable(gate_at, 256) {
        return format!("poller flag {flag}; key gate unreadable");
    }
    let mut gate = [0u8; 256];
    for (i, g) in gate.iter_mut().enumerate() {
        // SAFETY: checked readable above; plain reads of engine data.
        *g = unsafe { std::ptr::read_volatile((gate_at + i) as *const u8) };
    }
    let first: Vec<String> = gate[..16].iter().map(|b| format!("{b:02x}")).collect();
    let set: Vec<String> = gate
        .iter()
        .enumerate()
        .filter(|(_, &b)| b != 0)
        .take(24)
        .map(|(i, b)| format!("{i:#04x}={b}"))
        .collect();
    format!(
        "poller flag {flag}; key gate W(0x57)={} first16 [{}] non-zero [{}]",
        gate[0x57],
        first.join(" "),
        set.join(" ")
    )
}
