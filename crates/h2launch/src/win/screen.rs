//! What is on screen when the engine stops drawing: the process's own
//! top-level windows (an engine error dialog shows up here, with its
//! text), and a GDI grab of our window's area for when `end_frame` never
//! ran, so no back-buffer screenshot could be taken.
//!
//! Window text is read with `SendMessageTimeoutW(WM_GETTEXT)` and a short
//! timeout, because `GetWindowTextW` on a window of a hung thread in our
//! own process would block.

use std::ffi::c_void;
use std::sync::Mutex;

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
    GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetClassNameW, GetClientRect, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_GETTEXT,
};

use super::log::log;

unsafe extern "system" fn collect(h: HWND, lp: LPARAM) -> BOOL {
    // SAFETY: lp is the address of the Vec the caller passed in.
    let v = unsafe { &mut *(lp.0 as *mut Vec<usize>) };
    if v.len() < 4096 {
        v.push(h.0 as usize);
    }
    true.into()
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 128];
    // SAFETY: fills our buffer; sends no message.
    let n = unsafe { GetClassNameW(h, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..n.min(buf.len())])
}

fn window_text(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let mut copied = 0usize;
    // SAFETY: WM_GETTEXT into our buffer, given in characters; gives up
    // after 200 ms or at once if the owning thread is hung.
    let r = unsafe {
        SendMessageTimeoutW(
            h,
            WM_GETTEXT,
            WPARAM(buf.len()),
            LPARAM(buf.as_mut_ptr() as isize),
            SMTO_ABORTIFHUNG,
            200,
            Some(&mut copied),
        )
    };
    if r.0 == 0 {
        return "<no answer>".into();
    }
    String::from_utf16_lossy(&buf[..copied.min(buf.len())])
}

/// Our process's top-level windows other than the game window.
fn process_windows() -> Vec<HWND> {
    let mut all: Vec<usize> = Vec::new();
    // SAFETY: the callback only pushes into `all`, which outlives the call.
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut all as *mut Vec<usize> as isize)) };
    let pid = std::process::id();
    let ours = super::hwnd();
    all.into_iter()
        .map(|h| HWND(h as *mut c_void))
        .filter(|&h| {
            let mut p = 0u32;
            // SAFETY: a query on a window handle.
            unsafe { GetWindowThreadProcessId(h, Some(&mut p)) };
            p == pid && h != ours
        })
        .collect()
}

static SEEN: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// Logs the visible ones with their class, title, thread and the text of
/// their child controls (a dialog's message and buttons). `only_new`
/// skips windows logged before.
pub fn log_windows(only_new: bool) {
    let mut hidden = 0;
    let mut visible = 0;
    let mut listed = 0;
    for h in process_windows() {
        // SAFETY: a query on a window handle.
        if !unsafe { IsWindowVisible(h) }.as_bool() {
            hidden += 1;
            continue;
        }
        visible += 1;
        if only_new {
            let Ok(mut seen) = SEEN.try_lock() else {
                continue;
            };
            if seen.contains(&(h.0 as usize)) {
                continue;
            }
            seen.push(h.0 as usize);
        }
        listed += 1;
        // SAFETY: a query on a window handle.
        let thread = unsafe { GetWindowThreadProcessId(h, None) };
        log!(
            "  window {:#x} class {:?} title {:?} (thread {thread})",
            h.0 as usize,
            class_name(h),
            window_text(h)
        );
        let mut kids: Vec<usize> = Vec::new();
        // SAFETY: as for EnumWindows.
        let _ = unsafe {
            EnumChildWindows(
                Some(h),
                Some(collect),
                LPARAM(&mut kids as *mut Vec<usize> as isize),
            )
        };
        for &k in kids.iter().take(24) {
            let k = HWND(k as *mut c_void);
            let text = window_text(k);
            if !text.is_empty() {
                log!("    {:?}: {text:?}", class_name(k));
            }
        }
    }
    log!(
        "  ({visible} visible window(s) besides the game window, {listed} listed here; {hidden} hidden)"
    );
}

/// Saves a GDI grab of the game window's client area from the screen as
/// `h2launch-shot-<label>.png`.
pub fn save_window_capture(label: &str) {
    match capture() {
        Ok((w, h, rgba)) => {
            log!("screen grab {label}: the window's area as shown on screen (anything on top of it included)");
            super::gfx::save_png_now(label, w, h, rgba);
        }
        Err(e) => log!("screen grab {label}: {e}"),
    }
}

fn capture() -> Result<(u32, u32, Vec<u8>), String> {
    let hwnd = super::hwnd();
    if hwnd.0.is_null() {
        return Err("no window".into());
    }
    // SAFETY: GDI calls on our own window and objects we create and free
    // here.
    unsafe {
        if IsIconic(hwnd).as_bool() {
            return Err("the window is minimised".into());
        }
        let mut rc = RECT::default();
        GetClientRect(hwnd, &mut rc).map_err(|e| e.to_string())?;
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        if w <= 0 || h <= 0 {
            return Err("the window has no area".into());
        }
        let mut origin = POINT { x: 0, y: 0 };
        let _ = ClientToScreen(hwnd, &mut origin);
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("no screen device context".into());
        }
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp.into());
        let blit = BitBlt(mem, 0, 0, w, h, Some(screen), origin.x, origin.y, SRCCOPY);
        SelectObject(mem, old);
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u8; w as usize * h as usize * 4];
        let lines = GetDIBits(
            mem,
            bmp,
            0,
            h as u32,
            Some(px.as_mut_ptr() as *mut c_void),
            &mut bi,
            DIB_RGB_COLORS,
        );
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        blit.map_err(|e| format!("BitBlt failed: {e}"))?;
        if lines == 0 {
            return Err("GetDIBits copied nothing".into());
        }
        // BGRA to RGBA, opaque.
        for p in px.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
            p[3] = 255;
        }
        Ok((w as u32, h as u32, px))
    }
}
