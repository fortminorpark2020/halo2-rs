//! Crash reports. A vectored handler logs fatal exceptions as they are
//! raised ("first chance": the engine may still handle some of them, for
//! example a delay-load failure it expects), with the module and offset of
//! the faulting address, the registers and a short stack. The
//! unhandled-exception filter logs the crash, writes the RESULT line and
//! ends the process. Rust cannot catch SEH exceptions raised inside engine
//! code, so this is the whole plan for milestone 1.
//!
//! A stack overflow leaves only a page or so of stack, so for it both
//! handlers write one fixed line with no module lookup, and the filter
//! leaves the summary and RESULT line to the watchdog thread.

use std::ffi::c_void;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::core::PCWSTR;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, RtlLookupFunctionEntry, RtlVirtualUnwind,
    SetUnhandledExceptionFilter, CONTEXT, EXCEPTION_POINTERS, EXCEPTION_RECORD, UNW_FLAG_NHANDLER,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::Memory::{
    VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE,
    PAGE_WRITECOPY,
};
use windows::Win32::System::ProcessStatus::{
    K32EnumProcessModules, K32GetModuleInformation, MODULEINFO,
};
use windows::Win32::System::Threading::GetCurrentProcess;

use super::log::{log, StackLine};

const STACK_OVERFLOW: u32 = 0xC000_00FD;

/// Codes worth a report, with a name.
fn fatal_name(code: u32) -> Option<&'static str> {
    Some(match code {
        0xC000_0005 => "access violation",
        0xC000_001D => "illegal instruction",
        0xC000_0096 => "privileged instruction",
        STACK_OVERFLOW => "stack overflow",
        0xC000_0094 => "integer divide by zero",
        0xC000_0095 => "integer overflow",
        0xC000_008E => "float divide by zero",
        0xC06D_007E => "delay-load: module not found",
        0xC06D_007F => "delay-load: procedure not found",
        0xC000_0374 => "heap corruption",
        0xC000_0409 => "stack buffer overrun",
        0xC000_0420 => "assertion failure",
        0xC000_0006 => "in-page error",
        0xC000_0602 => "fail fast",
        0x8000_0003 => "breakpoint",
        _ => return None,
    })
}

static REPORTS: AtomicU32 = AtomicU32::new(0);
static BUSY: AtomicBool = AtomicBool::new(false);
/// At most this many early reports; later ones are only counted.
const MAX_REPORTS: u32 = 24;

pub fn install() {
    // SAFETY: registers a plain function as the first vectored handler.
    let h = unsafe { AddVectoredExceptionHandler(1, Some(vectored)) };
    if h.is_null() {
        log!("AddVectoredExceptionHandler failed");
    }
    set_filter("at start");
}

/// Puts our filter back if something replaced it (the engine has a crash
/// reporter of its own), logging only when it had been replaced. Called
/// from every 10-second summary.
pub fn refresh_filter() {
    // SAFETY: installs a plain function.
    let prev = unsafe { SetUnhandledExceptionFilter(Some(unhandled)) };
    let ours: usize = unhandled as unsafe extern "system" fn(_) -> _ as usize;
    if let Some(f) = prev {
        if f as usize != ours {
            let mut s = StackLine::new();
            where_is(f as usize, &mut s);
            log!(
                "unhandled-exception filter had been replaced by {}; ours is back",
                s.text()
            );
        }
    }
}

/// (Re)installs our unhandled-exception filter, logging the one it
/// replaces: loading the DLL or starting the game may install another.
pub fn set_filter(when: &str) {
    // SAFETY: installs a plain function.
    let prev = unsafe { SetUnhandledExceptionFilter(Some(unhandled)) };
    let ours: usize = unhandled as unsafe extern "system" fn(_) -> _ as usize;
    match prev {
        None => log!("unhandled-exception filter set {when} (none before)"),
        Some(f) if f as usize == ours => {
            log!("unhandled-exception filter set {when} (ours was still in place)")
        }
        Some(f) => {
            let mut s = StackLine::new();
            where_is(f as usize, &mut s);
            log!(
                "unhandled-exception filter set {when}; it replaced {}",
                s.text()
            );
        }
    }
}

/// Whether `len` bytes at `addr` are committed and readable.
pub fn readable(addr: usize, len: usize) -> bool {
    if addr < 0x10000 {
        return false;
    }
    let mut mbi = MEMORY_BASIC_INFORMATION::default();
    // SAFETY: VirtualQuery only describes the address space.
    let n = unsafe {
        VirtualQuery(
            Some(addr as *const c_void),
            &mut mbi,
            std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if n == 0 || mbi.State != MEM_COMMIT {
        return false;
    }
    let p = mbi.Protect.0;
    if p & (PAGE_GUARD.0 | PAGE_NOACCESS.0) != 0 {
        return false;
    }
    let read = PAGE_READONLY.0
        | PAGE_READWRITE.0
        | PAGE_WRITECOPY.0
        | PAGE_EXECUTE_READ.0
        | PAGE_EXECUTE_READWRITE.0
        | PAGE_EXECUTE_WRITECOPY.0;
    let end = mbi.BaseAddress as usize + mbi.RegionSize;
    p & read != 0 && addr.saturating_add(len) <= end
}

/// A zero-terminated byte string at `p`, if readable (at most 120 bytes).
pub fn c_string(p: usize, out: &mut StackLine) {
    for i in 0..120 {
        if !readable(p + i, 1) {
            let _ = out.write_str("<unreadable>");
            return;
        }
        // SAFETY: checked readable just above.
        let b = unsafe { *((p + i) as *const u8) };
        if b == 0 {
            return;
        }
        let _ = out.write_char(if (0x20..0x7F).contains(&b) {
            b as char
        } else {
            '?'
        });
    }
}

/// `module.dll+0x1234` for an address, or the bare address.
pub fn where_is(addr: usize, out: &mut StackLine) {
    let mut h = HMODULE::default();
    // SAFETY: FROM_ADDRESS treats the "name" as an address inside a module;
    // UNCHANGED_REFCOUNT means nothing needs freeing.
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(addr as *const u16),
            &mut h,
        )
    }
    .is_ok();
    if !found {
        let _ = write!(out, "{addr:#x}");
        return;
    }
    let mut name = [0u16; 260];
    // SAFETY: fills our buffer.
    let n = unsafe { GetModuleFileNameW(Some(h), &mut name) } as usize;
    let name = &name[..n.min(260)];
    let start = name
        .iter()
        .rposition(|&c| c == b'\\' as u16 || c == b'/' as u16)
        .map_or(0, |i| i + 1);
    for c in char::decode_utf16(name[start..].iter().copied()) {
        let _ = out.write_char(c.unwrap_or('?'));
    }
    let _ = write!(out, "+{:#x}", addr - h.0 as usize);
}

unsafe extern "system" fn vectored(info: *mut EXCEPTION_POINTERS) -> i32 {
    const CONTINUE_SEARCH: i32 = 0;
    // SAFETY: the system passes valid records.
    let (rec, ctx) = unsafe { (&*(*info).ExceptionRecord, &*(*info).ContextRecord) };
    let code = rec.ExceptionCode.0 as u32;
    if fatal_name(code).is_none() {
        return CONTINUE_SEARCH;
    }
    if code == STACK_OVERFLOW {
        overflow_line(b"raised", ctx.Rip);
        return CONTINUE_SEARCH;
    }
    let n = REPORTS.fetch_add(1, Ordering::SeqCst);
    if n < MAX_REPORTS {
        report("raised (first chance; the engine may handle it)", rec, ctx);
    } else if n == MAX_REPORTS {
        let mut s = StackLine::start();
        let _ = write!(s, "more exceptions follow; no longer reporting them");
        s.emit();
    }
    CONTINUE_SEARCH
}

unsafe extern "system" fn unhandled(info: *const EXCEPTION_POINTERS) -> i32 {
    // SAFETY: the system passes valid records.
    let (rec, ctx) = unsafe { (&*(*info).ExceptionRecord, &*(*info).ContextRecord) };
    let code = rec.ExceptionCode.0 as u32;
    super::CRASHED.store(code.max(1), Ordering::SeqCst);
    if code == STACK_OVERFLOW {
        overflow_line(b"UNHANDLED", ctx.Rip);
        // Too little stack left here for the summary: the watchdog thread
        // writes it and ends the process. If it never does, let Windows
        // end the process after 20 s.
        super::DEFERRED_CRASH.store(code, Ordering::SeqCst);
        for _ in 0..200 {
            // SAFETY: a plain sleep.
            unsafe { windows::Win32::System::Threading::Sleep(100) };
        }
        return 1; // EXCEPTION_EXECUTE_HANDLER: the process ends.
    }
    report("UNHANDLED", rec, ctx);
    super::finish("unhandled exception")
}

/// One fixed line for a stack overflow: no formatting machinery, no module
/// lookup, a 128-byte buffer.
fn overflow_line(kind: &[u8], rip: u64) {
    let mut buf = [0u8; 128];
    let mut n = 0;
    let mut hex = [0u8; 16];
    for (i, h) in hex.iter_mut().enumerate() {
        let d = ((rip >> ((15 - i) * 4)) & 0xF) as u8;
        *h = if d < 10 { b'0' + d } else { b'a' + d - 10 };
    }
    let parts: [&[u8]; 5] = [
        b"[stack overflow] exception ",
        kind,
        b": 0xc00000fd stack overflow at rip 0x",
        &hex,
        b" (no module lookup)\n",
    ];
    for part in parts {
        for &c in part {
            if n < buf.len() {
                buf[n] = c;
                n += 1;
            }
        }
    }
    super::log::raw(&buf[..n]);
}

fn report(kind: &str, rec: &EXCEPTION_RECORD, ctx: &CONTEXT) {
    let code = rec.ExceptionCode.0 as u32;
    let busy = BUSY.swap(true, Ordering::SeqCst);
    let mut s = StackLine::start();
    let _ = write!(
        s,
        "exception {kind}: {code:#010x} {} at ",
        fatal_name(code).unwrap_or("?")
    );
    where_is(rec.ExceptionAddress as usize, &mut s);
    let _ = write!(s, " during {}", super::step_name());
    s.emit();
    if busy {
        // Another report is being written; keep this one to a line.
        return;
    }
    let info = |i: usize| {
        if (i as u32) < rec.NumberParameters {
            rec.ExceptionInformation[i]
        } else {
            0
        }
    };
    if code == 0xC000_0005 || code == 0xC000_0006 {
        let what = match info(0) {
            0 => "reading",
            1 => "writing",
            8 => "executing",
            _ => "touching",
        };
        let _ = write!(s, "  {what} address {:#x}", info(1));
        s.emit();
    }
    if code == 0xC06D_007E || code == 0xC06D_007F {
        // DelayLoadInfo (delayimp.h): szDll at +0x18, fImportByName at
        // +0x20, the procedure name or ordinal at +0x28.
        let dli = info(0);
        if readable(dli, 0x48) {
            // SAFETY: checked readable.
            let (dll, by_name, proc_) = unsafe {
                (
                    *((dli + 0x18) as *const usize),
                    *((dli + 0x20) as *const i32),
                    *((dli + 0x28) as *const usize),
                )
            };
            let _ = write!(s, "  delay-loaded DLL ");
            c_string(dll, &mut s);
            let _ = write!(s, ", procedure ");
            if by_name != 0 {
                c_string(proc_, &mut s);
            } else {
                let _ = write!(s, "#{proc_}");
            }
            s.emit();
        }
    }
    let _ = write!(
        s,
        "  rax={:#x} rbx={:#x} rcx={:#x} rdx={:#x} rsi={:#x} rdi={:#x}",
        ctx.Rax, ctx.Rbx, ctx.Rcx, ctx.Rdx, ctx.Rsi, ctx.Rdi
    );
    s.emit();
    let _ = write!(
        s,
        "  rsp={:#x} rbp={:#x} r8={:#x} r9={:#x} r10={:#x} r11={:#x}",
        ctx.Rsp, ctx.Rbp, ctx.R8, ctx.R9, ctx.R10, ctx.R11
    );
    s.emit();
    let _ = write!(
        s,
        "  r12={:#x} r13={:#x} r14={:#x} r15={:#x} rip={:#x}",
        ctx.R12, ctx.R13, ctx.R14, ctx.R15, ctx.Rip
    );
    s.emit();
    if code != STACK_OVERFLOW {
        stack_top(ctx, &mut s);
        stack(ctx, &mut s);
    }
    BUSY.store(false, Ordering::SeqCst);
}

/// The first 8 qwords at rsp, as module+offset where they point into a
/// module: the caller's return address is usually among them, even when
/// the unwinder cannot start (a call through a null pointer).
fn stack_top(ctx: &CONTEXT, s: &mut StackLine) {
    let _ = write!(s, "  [rsp]:");
    for i in 0..8u64 {
        let at = ctx.Rsp + 8 * i;
        if !readable(at as usize, 8) {
            let _ = write!(s, " <unreadable>");
            break;
        }
        // SAFETY: checked readable.
        let v = unsafe { *(at as *const u64) };
        let _ = write!(s, " ");
        if v >= 0x10000 {
            where_is(v as usize, s);
        } else {
            let _ = write!(s, "{v:#x}");
        }
    }
    s.emit();
}

/// Up to 12 frames, unwound with the modules' own unwind tables.
fn stack(ctx: &CONTEXT, s: &mut StackLine) {
    let mut c: CONTEXT = *ctx;
    let _ = write!(s, "  stack:");
    for frame in 0..12 {
        let pc = c.Rip;
        if pc == 0 && frame > 0 {
            break;
        }
        let _ = write!(s, " ");
        where_is(pc as usize, s);
        let mut base = 0u64;
        // SAFETY: looks up unwind data for an address; null when none (and
        // always for 0, a call through a null pointer, where the return
        // address is still at the top of the stack).
        let f = if pc == 0 {
            std::ptr::null_mut()
        } else {
            unsafe { RtlLookupFunctionEntry(pc, &mut base, None) }
        };
        if f.is_null() {
            // A leaf function: the return address is at the top of the stack.
            if !readable(c.Rsp as usize, 8) {
                break;
            }
            // SAFETY: checked readable.
            c.Rip = unsafe { *(c.Rsp as *const u64) };
            c.Rsp += 8;
        } else {
            let mut handler: *mut c_void = std::ptr::null_mut();
            let mut frame = 0u64;
            // SAFETY: unwinds our copy of the context by one frame.
            unsafe {
                RtlVirtualUnwind(
                    UNW_FLAG_NHANDLER,
                    base,
                    pc,
                    f,
                    &mut c,
                    &mut handler,
                    &mut frame,
                    None,
                );
            }
        }
    }
    s.emit();
}

/// The modules loaded in this process, with base and size.
pub fn log_modules() {
    let mut mods = vec![HMODULE::default(); 512];
    let mut needed = 0u32;
    // SAFETY: fills our array.
    let ok = unsafe {
        K32EnumProcessModules(
            GetCurrentProcess(),
            mods.as_mut_ptr(),
            (mods.len() * std::mem::size_of::<HMODULE>()) as u32,
            &mut needed,
        )
    };
    if !ok.as_bool() {
        log!("module list unavailable");
        return;
    }
    let count = (needed as usize / std::mem::size_of::<HMODULE>()).min(mods.len());
    log!("{count} modules loaded:");
    for &m in &mods[..count] {
        let mut info = MODULEINFO::default();
        // SAFETY: fills our struct.
        let _ = unsafe {
            K32GetModuleInformation(
                GetCurrentProcess(),
                m,
                &mut info,
                std::mem::size_of::<MODULEINFO>() as u32,
            )
        };
        let mut name = [0u16; 260];
        // SAFETY: fills our buffer.
        let n = unsafe { GetModuleFileNameW(Some(m), &mut name) } as usize;
        log!(
            "  {:#014x} {:#010x} {}",
            info.lpBaseOfDll as usize,
            info.SizeOfImage,
            String::from_utf16_lossy(&name[..n.min(260)])
        );
    }
}
