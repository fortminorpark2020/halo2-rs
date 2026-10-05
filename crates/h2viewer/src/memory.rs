//! The game's memory over a long session: giving back what the last level
//! used, and how much the game has, which it prints as each level goes in
//! (so a session's log shows whether it keeps growing).

/// Hand the memory the last level used back to the system, once it's
/// gone. On Linux the C library keeps freed memory for later instead, and
/// a level loads while the last one is still up, so without this the game
/// grows by hundreds of megabytes over its first few maps (to two or three
/// times what it uses).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn release() {
    extern "C" {
        fn malloc_trim(pad: usize) -> std::ffi::c_int;
    }
    // SAFETY: glibc's malloc_trim takes no pointers; it only gives free
    // pages back.
    unsafe {
        malloc_trim(0);
    }
}

/// Elsewhere that's up to the system's allocator. Windows' hasn't been
/// measured over a long session yet: a bot's `memory:` lines there show it.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn release() {}

/// The memory the game has now and the most it has had, in megabytes:
/// resident memory on Linux, the working set on Windows.
#[cfg(target_os = "linux")]
pub fn in_use() -> Option<(usize, usize)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let mb = |field: &str| -> Option<usize> {
        let line = status.lines().find_map(|l| l.strip_prefix(field))?;
        let kb: usize = line.trim().trim_end_matches("kB").trim().parse().ok()?;
        Some(kb / 1024)
    };
    Some((mb("VmRSS:")?, mb("VmHWM:")?))
}

#[cfg(windows)]
pub fn in_use() -> Option<(usize, usize)> {
    use std::ffi::c_void;
    /// Windows' PROCESS_MEMORY_COUNTERS.
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        size: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        pools_and_pagefile: [usize; 6],
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn K32GetProcessMemoryInfo(process: *mut c_void, counters: *mut Counters, size: u32)
            -> i32;
    }
    let mut c = Counters {
        size: size_of::<Counters>() as u32,
        ..Default::default()
    };
    // SAFETY: the counters are as big as `size` says, and the current
    // process's handle needs no closing.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.size) };
    (ok != 0).then_some((c.working_set >> 20, c.peak_working_set >> 20))
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn in_use() -> Option<(usize, usize)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_in_use_is_read() {
        release();
        let (now, most) = in_use().expect("this system's memory counters");
        assert!(now > 0 && now <= most, "{now} MB now, {most} MB at most");
    }
}
