//! The game menu's overlay: one opaque panel, drawn on the CPU by
//! `h2ui::gamemenu::draw_panel`, kept in a texture of our own and copied
//! into the engine's back buffer 0 in `end_frame`, before its Present
//! (the 1.6 us variant of the 2026-10-10 proof, menu.md 9.1 B).
//!
//! The window thread (the menu's `Driver`) makes the texture through the
//! shared device, which is free-threaded, and publishes it. The engine's
//! render thread copies the latest one each frame while the menu shows.
//! The copy:
//!
//! - never touches pipeline state (one `CopySubresourceRegion`);
//! - never blocks the render thread (`try_lock`, and its own copy of the
//!   last panel);
//! - never keeps the back buffer past `end_frame`, so the engine's
//!   `ResizeBuffers` keeps working;
//! - is skipped when the back buffer isn't the size, format or sample
//!   count the panel was drawn for. The menu draws for the swap chain's
//!   size (`gfx::swap_size`), which only our resize changes, so this is a
//!   safety net: a resize's new panel comes on the menu's next tick.
//!
//! A panic or a Direct3D error, in the copy or in making the texture
//! (`fail`), turns the overlay off for the run (`failed`); the menu then
//! turns itself off and Start goes back to the engine (once PLEASE WAIT,
//! if it shows, is over), while screenshots and present counting go on.
//!
//! `H2LAUNCH_OVERLAY_FAIL` stands in for those failures in the PC checks
//! (menu.md 9.5, check 17): `stall` never copies the panel, as if it were
//! never drawn, and `copy` fails the first copy, as a Direct3D error
//! would. (`1` keeps the menu off from the start: `menu::prepare`.)

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE, D3D11_BOX,
    D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;

use super::log::log;

/// A published panel: its texture and where it goes in a back buffer of
/// the size it was drawn for.
pub struct Panel {
    tex: ID3D11Texture2D,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    /// The back buffer size it was laid out for.
    target: (u32, u32),
    gen: u64,
}
// SAFETY: D3D11 resources may be used from any thread; the texture is
// written once at creation (initial data) and only read after that, by
// the render thread's copy on the immediate context, which only that
// thread uses while the engine runs (as `gfx::Gfx`).
unsafe impl Send for Panel {}
unsafe impl Sync for Panel {}

/// The latest panel, swapped in by the window thread.
static PANEL: Mutex<Option<Arc<Panel>>> = Mutex::new(None);
thread_local! {
    /// The render thread's copy of the last panel it got, so a busy lock
    /// never costs it a frame.
    static CACHE: RefCell<Option<Arc<Panel>>> = const { RefCell::new(None) };
}
/// The panel is copied each frame.
static VISIBLE: AtomicBool = AtomicBool::new(false);
/// The newest generation published, and the newest the render thread
/// copied.
static PUBLISHED_GEN: AtomicU64 = AtomicU64::new(0);
static DRAWN_GEN: AtomicU64 = AtomicU64::new(0);
/// The overlay failed; it stays off for the run.
static FAILED: AtomicBool = AtomicBool::new(false);
/// Since the last summary: copies, failures, and microseconds in all and
/// at most.
static COPIES: AtomicU64 = AtomicU64::new(0);
static FAILURES: AtomicU64 = AtomicU64::new(0);
static TOTAL_NS: AtomicU64 = AtomicU64::new(0);
static MAX_NS: AtomicU64 = AtomicU64::new(0);
/// Mismatches already logged, of each kind (format, samples, size): a
/// few each, not every frame.
static ODD_LOGS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
/// `H2LAUNCH_OVERLAY_FAIL`'s stand-in for this run (`set_stand_in`).
static STAND_IN: AtomicU8 = AtomicU8::new(NO_STAND_IN);
const NO_STAND_IN: u8 = 0;
const STALL: u8 = 1;
const COPY_FAILS: u8 = 2;

/// Takes `H2LAUNCH_OVERLAY_FAIL`'s value for a stand-in that fails at run
/// time (`stall` or `copy`, see the module notes); false for any other.
pub fn set_stand_in(which: &str) -> bool {
    let v = match which {
        "stall" => STALL,
        "copy" => COPY_FAILS,
        _ => return false,
    };
    STAND_IN.store(v, Ordering::SeqCst);
    true
}

/// Shows or hides the panel.
pub fn show(on: bool) {
    VISIBLE.store(on, Ordering::SeqCst);
}

pub fn failed() -> bool {
    FAILED.load(Ordering::SeqCst)
}

/// The overlay failed: off for the run, said once. Never blocks (the
/// render thread calls it).
fn set_failed(why: &str) {
    VISIBLE.store(false, Ordering::SeqCst);
    if !FAILED.swap(true, Ordering::SeqCst) {
        log!("overlay: {why}; the overlay is off for this run");
    }
}

/// The window thread couldn't publish the panel (`publish` failed): the
/// overlay is off for the run, as for a failed copy, and the last panel
/// is dropped so an older page is never shown in its place.
pub fn fail(why: &str) {
    set_failed(why);
    clear();
}

/// The newest generation the render thread copied.
pub fn drawn() -> u64 {
    DRAWN_GEN.load(Ordering::SeqCst)
}

/// Makes a texture from `panel` (drawn for a `target` back buffer) and
/// makes it the one copied. Window thread.
pub fn publish(
    device: &ID3D11Device,
    panel: &h2ui::gamemenu::Panel,
    target: (u32, u32),
) -> Result<u64, String> {
    if failed() {
        return Err("the overlay is off".into());
    }
    let (w, h) = (panel.w, panel.h);
    if w == 0 || h == 0 || panel.rgba.len() != (w as usize) * (h as usize) * 4 {
        return Err(format!(
            "a {w}x{h} panel with {} bytes can't be published",
            panel.rgba.len()
        ));
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: panel.rgba.as_ptr().cast(),
        SysMemPitch: w * 4,
        SysMemSlicePitch: 0,
    };
    let mut tex: Option<ID3D11Texture2D> = None;
    // SAFETY: a valid descriptor, and initial data of h rows of w*4 bytes
    // that outlive the call (it copies them).
    unsafe { device.CreateTexture2D(&desc, Some(&data), Some(&mut tex)) }
        .map_err(|e| format!("CreateTexture2D({w}x{h}): {e}"))?;
    let tex = tex.ok_or("CreateTexture2D gave no texture")?;
    let gen = PUBLISHED_GEN.load(Ordering::SeqCst) + 1;
    let p = Arc::new(Panel {
        tex,
        x: panel.x,
        y: panel.y,
        w,
        h,
        target,
        gen,
    });
    match PANEL.lock() {
        Ok(mut g) => *g = Some(p),
        Err(e) => *e.into_inner() = Some(p),
    }
    PUBLISHED_GEN.store(gen, Ordering::SeqCst);
    Ok(gen)
}

/// Drops the published panel (the menu closed) so its texture goes once
/// the render thread lets go of its copy.
pub fn clear() {
    if let Ok(mut g) = PANEL.lock() {
        *g = None;
    }
}

/// Host slot 1 (`end_frame`), on the engine's render thread, before the
/// screenshot capture: copies the panel into back buffer 0 while it shows.
pub fn draw(context: &ID3D11DeviceContext, swap: &IDXGISwapChain) {
    if !VISIBLE.load(Ordering::Relaxed) || FAILED.load(Ordering::Relaxed) {
        return;
    }
    let start = Instant::now();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| copy(context, swap)));
    let error = match r {
        Ok(Ok(copied)) => {
            if copied {
                let ns = start.elapsed().as_nanos() as u64;
                COPIES.fetch_add(1, Ordering::Relaxed);
                TOTAL_NS.fetch_add(ns, Ordering::Relaxed);
                MAX_NS.fetch_max(ns, Ordering::Relaxed);
            }
            return;
        }
        Ok(Err(e)) => e,
        Err(_) => "a panic in the copy".to_string(),
    };
    FAILURES.fetch_add(1, Ordering::Relaxed);
    set_failed(&error);
}

/// One frame's copy: true when it copied.
fn copy(context: &ID3D11DeviceContext, swap: &IDXGISwapChain) -> Result<bool, String> {
    let panel = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Ok(g) = PANEL.try_lock() {
            let fresh = match (&*c, &*g) {
                (Some(old), Some(new)) => old.gen != new.gen,
                (None, None) => false,
                _ => true,
            };
            if fresh {
                *c = g.clone();
            }
        }
        c.clone()
    });
    let Some(p) = panel else {
        return Ok(false);
    };
    match STAND_IN.load(Ordering::Relaxed) {
        STALL => return Ok(false),
        COPY_FAILS => {
            return Err("H2LAUNCH_OVERLAY_FAIL=copy (a stand-in for a Direct3D error)".into())
        }
        _ => {}
    }
    // SAFETY: our own swap chain; the reference is dropped when this
    // returns, inside end_frame.
    let back: ID3D11Texture2D =
        unsafe { swap.GetBuffer(0) }.map_err(|e| format!("GetBuffer(0): {e}"))?;
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: fills our struct.
    unsafe { back.GetDesc(&mut desc) };
    let odd = if desc.Format != DXGI_FORMAT_R8G8B8A8_UNORM {
        Some((0, format!("back buffer format {}", desc.Format.0)))
    } else if desc.SampleDesc.Count != 1 {
        Some((
            1,
            format!("back buffer has {} samples", desc.SampleDesc.Count),
        ))
    } else if (desc.Width, desc.Height) != p.target {
        Some((
            2,
            format!(
                "back buffer {}x{}, panel drawn for {}x{}; the menu draws the next for the swap chain's size",
                desc.Width, desc.Height, p.target.0, p.target.1
            ),
        ))
    } else {
        None
    };
    if let Some((kind, why)) = odd {
        if ODD_LOGS[kind].fetch_add(1, Ordering::Relaxed) < 8 {
            log!("overlay: not copied ({why})");
        }
        return Ok(false);
    }
    // Clipped to the back buffer.
    if p.x >= desc.Width || p.y >= desc.Height {
        return Ok(false);
    }
    let right = p.w.min(desc.Width - p.x);
    let bottom = p.h.min(desc.Height - p.y);
    let bx = D3D11_BOX {
        left: 0,
        top: 0,
        front: 0,
        right,
        bottom,
        back: 1,
    };
    // SAFETY: both resources are ours, R8G8B8A8_UNORM with one sample,
    // and the box lies inside both; the immediate context is the render
    // thread's while the engine runs.
    unsafe { context.CopySubresourceRegion(&back, 0, p.x, p.y, 0, &p.tex, 0, Some(&bx)) };
    DRAWN_GEN.fetch_max(p.gen, Ordering::SeqCst);
    Ok(true)
}

/// The 10-second summary's line, for the interval since the last one;
/// None while the overlay has nothing to say (never shown, no failure).
pub fn summary_line(menu_on: bool) -> Option<String> {
    let n = COPIES.swap(0, Ordering::Relaxed);
    let f = FAILURES.swap(0, Ordering::Relaxed);
    let total = TOTAL_NS.swap(0, Ordering::Relaxed);
    let max = MAX_NS.swap(0, Ordering::Relaxed);
    if n == 0 && f == 0 && !menu_on {
        return None;
    }
    let mean = if n == 0 {
        "-".to_string()
    } else {
        format!("{:.1}", total as f64 / n as f64 / 1000.0)
    };
    Some(format!(
        "overlay: {n} copies, {f} failures, mean {mean} us, max {:.1} us",
        max as f64 / 1000.0
    ))
}
