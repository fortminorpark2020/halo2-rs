//! The window, the D3D11 device and swap chain, resizing and screenshots.
//!
//! Device and swap chain as HaloX makes them for halo2
//! (launch-sequence 3.2): hardware device, flags 0, feature levels 11_0
//! and 10_0, R8G8B8A8_UNORM, two buffers, DISCARD, windowed, 60/1,
//! ALLOW_MODE_SWITCH, usage render target + unordered access + shader
//! input. The engine presents; we only draw one black frame before it
//! gets the swap chain.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};

use windows::core::{w, Interface};
use windows::Win32::Foundation::{HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_FLAG, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_MODE_DESC,
    DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIAdapter, IDXGIAdapter1, IDXGIDevice, IDXGIFactory, IDXGISwapChain, DXGI_MWA_NO_ALT_ENTER,
    DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH, DXGI_SWAP_EFFECT_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, DXGI_USAGE_SHADER_INPUT, DXGI_USAGE_UNORDERED_ACCESS,
};
use windows::Win32::Graphics::Gdi::{GetStockObject, UpdateWindow, BLACK_BRUSH, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_F4;
use windows::Win32::UI::Input::{RegisterRawInputDevices, RAWINPUTDEVICE, RAWINPUTDEVICE_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CreateWindowExW, DefWindowProcW, GetClientRect, GetSystemMetrics,
    LoadCursorW, PostQuitMessage, RegisterClassExW, SetCursor, SetForegroundWindow, SetTimer,
    ShowWindow, CS_CLASSDC, HTCLIENT, IDC_ARROW, SC_KEYMENU, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW,
    WINDOW_EX_STYLE, WM_ACTIVATEAPP, WM_APP, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_INPUT,
    WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_MOUSEMOVE, WM_SETCURSOR, WM_SETFOCUS, WM_SIZE,
    WM_SYSCOMMAND, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};

use super::log::log;

/// Host slot 2 asks the window thread to resize the swap chain.
pub const WM_APP_RESIZE: u32 = WM_APP + 1;
/// Host slot 4 (restart_game) asks the window thread to quit.
pub const WM_APP_RESTART: u32 = WM_APP + 2;

struct Gfx {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap: IDXGISwapChain,
}
// SAFETY: D3D11 devices are free-threaded; the immediate context is used
// only by the engine's render thread (in end_frame) once the engine runs,
// and by the window thread before that; the swap chain is resized only
// while the engine thread waits in host slot 2.
unsafe impl Send for Gfx {}
unsafe impl Sync for Gfx {}

static GFX: OnceLock<Gfx> = OnceLock::new();

/// The raw device, immediate context and swap chain pointers for the
/// engine (it gets no extra references; ours live for the whole run).
pub fn raw() -> Option<(*mut c_void, *mut c_void, *mut c_void)> {
    let g = GFX.get()?;
    Some((g.device.as_raw(), g.context.as_raw(), g.swap.as_raw()))
}

static SWAP_W: AtomicU32 = AtomicU32::new(0);
static SWAP_H: AtomicU32 = AtomicU32::new(0);
/// The window's client size from the last WM_SIZE and when it came.
static WANT: Mutex<(u32, u32, f64)> = Mutex::new((0, 0, 0.0));
static RESIZE_POSTED: Mutex<(u32, u32)> = Mutex::new((0, 0));

pub fn create_window(width: u32, height: u32) -> Result<HWND, String> {
    // SAFETY: plain Win32 window creation on this thread.
    unsafe {
        let module: HMODULE = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let instance: HINSTANCE = module.into();
        let class = w!("h2launch_window");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_CLASSDC,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err(format!(
                "RegisterClassExW failed: {}",
                windows::core::Error::from_thread()
            ));
        }
        let style = WS_OVERLAPPEDWINDOW;
        let mut r = RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let _ = AdjustWindowRect(&mut r, style, false);
        let (ww, wh) = (r.right - r.left, r.bottom - r.top);
        let x = ((GetSystemMetrics(SM_CXSCREEN) - ww) / 2).max(0);
        let y = ((GetSystemMetrics(SM_CYSCREEN) - wh) / 2).max(0);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Halo 2 - h2launch"),
            style,
            x,
            y,
            ww,
            wh,
            None,
            None,
            Some(instance),
            None,
        )
        .map_err(|e| format!("CreateWindowExW failed: {e}"))?;
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
        let _ = SetForegroundWindow(hwnd);
        // A tick a second for the watchdog's heartbeat; WM_TIMER also
        // arrives inside Windows' modal move and size loops, where our
        // main loop does not run.
        if SetTimer(Some(hwnd), 1, 1000, None) == 0 {
            log!("SetTimer failed (the watchdog may misjudge a long window drag)");
        }
        // Raw mouse motion for our window while it is in front.
        let rid = RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x02,
            dwFlags: RAWINPUTDEVICE_FLAGS(0),
            hwndTarget: hwnd,
        };
        if let Err(e) =
            RegisterRawInputDevices(&[rid], std::mem::size_of::<RAWINPUTDEVICE>() as u32)
        {
            log!("RegisterRawInputDevices failed: {e} (mouse look will not work)");
        }
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        log!(
            "window {ww}x{wh} at {x},{y}, client {}x{}",
            rc.right - rc.left,
            rc.bottom - rc.top
        );
        Ok(hwnd)
    }
}

pub fn create_device(hwnd: HWND) -> Result<(), String> {
    // SAFETY: D3D11/DXGI creation with valid out pointers.
    unsafe {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let mut level = D3D_FEATURE_LEVEL::default();
        D3D11CreateDevice(
            None::<&IDXGIAdapter>,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG(0),
            Some(&[D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut level),
            Some(&mut context),
        )
        .map_err(|e| format!("D3D11CreateDevice failed: {e}"))?;
        let device = device.ok_or("no device")?;
        let context = context.ok_or("no context")?;
        let dxgi: IDXGIDevice = device.cast().map_err(|e| e.to_string())?;
        let adapter = dxgi.GetAdapter().map_err(|e| e.to_string())?;
        if let Ok(a1) = adapter.cast::<IDXGIAdapter1>() {
            log!("{}", adapter_line(&a1));
        }
        log!("feature level {:#x}", level.0);
        let factory: IDXGIFactory = adapter.GetParent().map_err(|e| e.to_string())?;
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let (w, h) = (
            (rc.right - rc.left).max(1) as u32,
            (rc.bottom - rc.top).max(1) as u32,
        );
        let desc = DXGI_SWAP_CHAIN_DESC {
            BufferDesc: DXGI_MODE_DESC {
                Width: w,
                Height: h,
                RefreshRate: DXGI_RATIONAL {
                    Numerator: 60,
                    Denominator: 1,
                },
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                ..Default::default()
            },
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT
                | DXGI_USAGE_UNORDERED_ACCESS
                | DXGI_USAGE_SHADER_INPUT,
            BufferCount: 2,
            OutputWindow: hwnd,
            Windowed: true.into(),
            SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
            Flags: DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH.0 as u32,
        };
        let mut swap: Option<IDXGISwapChain> = None;
        factory
            .CreateSwapChain(&device, &desc, &mut swap)
            .ok()
            .map_err(|e| format!("CreateSwapChain failed: {e}"))?;
        let swap = swap.ok_or("no swap chain")?;
        // Our choice: no DXGI Alt+Enter fullscreen switch behind the
        // engine's back.
        let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
        SWAP_W.store(w, Ordering::SeqCst);
        SWAP_H.store(h, Ordering::SeqCst);
        log!("swap chain {w}x{h} R8G8B8A8_UNORM, 2 buffers, DISCARD, windowed");

        // One black frame so the window is not left blank while loading.
        if let Ok(back) = swap.GetBuffer::<ID3D11Texture2D>(0) {
            let mut rtv: Option<ID3D11RenderTargetView> = None;
            if device
                .CreateRenderTargetView(&back, None, Some(&mut rtv))
                .is_ok()
            {
                if let Some(rtv) = &rtv {
                    context.ClearRenderTargetView(rtv, &[0.0, 0.0, 0.0, 1.0]);
                }
                let _ = swap.Present(0, DXGI_PRESENT(0));
            }
        }
        context.ClearState();
        context.Flush();
        let _ = GFX.set(Gfx {
            device,
            context,
            swap,
        });
        Ok(())
    }
}

/// One line about a video adapter.
pub fn adapter_line(a: &IDXGIAdapter1) -> String {
    // SAFETY: reads the adapter's description.
    match unsafe { a.GetDesc1() } {
        Ok(d) => format!(
            "adapter: {} (vendor {:04x} device {:04x}, {} MB video memory)",
            crate::util::from_wide(&d.Description),
            d.VendorId,
            d.DeviceId,
            d.DedicatedVideoMemory / (1024 * 1024)
        ),
        Err(e) => format!("adapter: unknown ({e})"),
    }
}

fn note_size(w: u32, h: u32) {
    if let Ok(mut g) = WANT.lock() {
        *g = (w, h, super::now());
    }
}

/// Called by the main loop: once the window has kept a new size for a
/// quarter of a second during play, tell the engine (message 14); it then
/// calls host slot 2, which resizes the swap chain on this thread.
pub fn resize_if_settled() {
    if super::GAME_THREAD.load(Ordering::SeqCst) == 0 {
        return;
    }
    let Ok(want) = WANT.lock().map(|g| *g) else {
        return;
    };
    let (w, h, at) = want;
    if w == 0 || h == 0 || super::now() - at < 0.25 {
        return;
    }
    if (w, h) == (SWAP_W.load(Ordering::SeqCst), SWAP_H.load(Ordering::SeqCst)) {
        return;
    }
    if let Ok(mut posted) = RESIZE_POSTED.lock() {
        if *posted != (w, h) {
            *posted = (w, h);
            log!("window is now {w}x{h}; asking the engine to resize");
            super::engine::post_message(14, "resize");
        }
    }
}

/// Window thread, from host slot 2's message.
fn resize_swap_chain(hwnd: HWND) {
    let Some(g) = GFX.get() else { return };
    let mut rc = RECT::default();
    // SAFETY: our window.
    let _ = unsafe { GetClientRect(hwnd, &mut rc) };
    let (w, h) = ((rc.right - rc.left) as u32, (rc.bottom - rc.top) as u32);
    if w == 0 || h == 0 {
        log!("resize skipped: window is minimised");
        return;
    }
    // SAFETY: the engine thread is waiting in host slot 2, having let go of
    // the back buffers; we hold none.
    match unsafe {
        g.swap
            .ResizeBuffers(0, w, h, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))
    } {
        Ok(()) => {
            SWAP_W.store(w, Ordering::SeqCst);
            SWAP_H.store(h, Ordering::SeqCst);
            log!("swap chain resized to {w}x{h}");
        }
        Err(e) => log!("ResizeBuffers({w}x{h}) failed: {e}"),
    }
}

// ---------------------------------------------------------------- frames

static PRESENTS: AtomicU32 = AtomicU32::new(0);
static PRESENTS_KNOWN: AtomicBool = AtomicBool::new(false);
static CHECKED_SWAP: AtomicBool = AtomicBool::new(false);

pub fn presents() -> Option<u32> {
    PRESENTS_KNOWN
        .load(Ordering::SeqCst)
        .then(|| PRESENTS.load(Ordering::SeqCst))
}

/// Host slot 1, on the engine's render thread: count presents and take a
/// waiting screenshot.
pub fn on_end_frame(engine_swap: *mut c_void) {
    let Some(g) = GFX.get() else { return };
    if !CHECKED_SWAP.swap(true, Ordering::SeqCst) {
        log!(
            "end_frame: the engine's swap chain {:p} is {}",
            engine_swap,
            if engine_swap == g.swap.as_raw() {
                "ours"
            } else {
                "NOT ours"
            }
        );
    }
    // SAFETY: a query on our swap chain.
    if let Ok(n) = unsafe { g.swap.GetLastPresentCount() } {
        PRESENTS.store(n, Ordering::SeqCst);
        PRESENTS_KNOWN.store(true, Ordering::SeqCst);
    }
    let label = SHOT_QUEUE.lock().ok().and_then(|mut q| {
        if q.is_empty() {
            None
        } else {
            Some(q.remove(0))
        }
    });
    if let Some(label) = label {
        match capture(g) {
            Ok(shot) => send_shot(label, shot),
            Err(e) => log!("screenshot {label}: {e}"),
        }
    }
}

// ---------------------------------------------------------------- screenshots

struct Shot {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

static SHOT_QUEUE: Mutex<Vec<String>> = Mutex::new(Vec::new());
static WRITER: Mutex<Option<mpsc::Sender<(String, Shot)>>> = Mutex::new(None);
static WRITING: AtomicUsize = AtomicUsize::new(0);

pub fn request_screenshot(label: String) {
    log!("screenshot {label} requested (taken at the next engine frame)");
    if let Ok(mut q) = SHOT_QUEUE.lock() {
        q.push(label);
    }
}

/// Copies the back buffer through a staging texture. Runs inside
/// end_frame on the engine's render thread, the only thread using the
/// immediate context while the game runs.
fn capture(g: &Gfx) -> Result<Shot, String> {
    // SAFETY: D3D11 calls on our own objects with valid descriptors.
    unsafe {
        let back: ID3D11Texture2D = g.swap.GetBuffer(0).map_err(|e| e.to_string())?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        back.GetDesc(&mut desc);
        let format = desc.Format;
        desc.MipLevels = 1;
        desc.ArraySize = 1;
        desc.SampleDesc = DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        };
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut staging: Option<ID3D11Texture2D> = None;
        g.device
            .CreateTexture2D(&desc, None, Some(&mut staging))
            .map_err(|e| format!("staging texture: {e}"))?;
        let staging = staging.ok_or("no staging texture")?;
        g.context.CopyResource(&staging, &back);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        g.context
            .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(|e| format!("Map: {e}"))?;
        let (w, h) = (desc.Width as usize, desc.Height as usize);
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            let src = std::slice::from_raw_parts(
                (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                w * 4,
            );
            rgba[y * w * 4..(y + 1) * w * 4].copy_from_slice(src);
        }
        g.context.Unmap(&staging, 0);
        if format == DXGI_FORMAT_B8G8R8A8_UNORM {
            for p in rgba.as_chunks_mut::<4>().0 {
                p.swap(0, 2);
            }
        } else if format != DXGI_FORMAT_R8G8B8A8_UNORM {
            log!(
                "screenshot: back buffer format {} saved as if RGBA",
                format.0
            );
        }
        Ok(Shot {
            width: w as u32,
            height: h as u32,
            rgba,
        })
    }
}

fn send_shot(label: String, shot: Shot) {
    let Ok(mut writer) = WRITER.lock() else {
        return;
    };
    if writer.is_none() {
        let (tx, rx) = mpsc::channel::<(String, Shot)>();
        let spawned = std::thread::Builder::new()
            .name("h2launch-png".into())
            .spawn(move || {
                while let Ok((label, shot)) = rx.recv() {
                    write_png(&label, shot);
                    WRITING.fetch_sub(1, Ordering::SeqCst);
                }
            });
        if let Err(e) = spawned {
            log!("screenshot writer thread: {e}");
            return;
        }
        *writer = Some(tx);
    }
    WRITING.fetch_add(1, Ordering::SeqCst);
    if let Some(tx) = writer.as_ref() {
        if tx.send((label, shot)).is_err() {
            WRITING.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Writes a picture now, on this thread (used at the end of the run).
pub fn save_png_now(label: &str, width: u32, height: u32, rgba: Vec<u8>) {
    write_png(
        label,
        Shot {
            width,
            height,
            rgba,
        },
    );
}

/// Screenshots requested but not taken yet.
pub fn pending_screenshots() -> usize {
    SHOT_QUEUE.try_lock().map_or(0, |q| q.len())
}

fn write_png(label: &str, mut shot: Shot) {
    let stats = crate::util::PictureStats::of_rgba(&shot.rgba, shot.width, shot.height);
    // The back buffer's alpha is whatever the engine left there; save the
    // picture opaque.
    for p in shot.rgba.as_chunks_mut::<4>().0 {
        p[3] = 255;
    }
    let dir = super::setup()
        .map(|s| s.log_dir.clone())
        .unwrap_or_else(super::log_dir);
    let path = dir.join(format!("h2launch-shot-{label}.png"));
    let result = (|| -> Result<(), String> {
        let f = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), shot.width, shot.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&shot.rgba).map_err(|e| e.to_string())?;
        Ok(())
    })();
    match result {
        Ok(()) => log!(
            "screenshot {label}: {} ({}x{}): {}",
            path.display(),
            shot.width,
            shot.height,
            stats.describe()
        ),
        Err(e) => log!("screenshot {label}: writing {} failed: {e}", path.display()),
    }
}

/// At exit: wait for screenshots being written and list those never taken.
pub fn finish_screenshots(timeout: f64) {
    let until = super::now() + timeout;
    while WRITING.load(Ordering::SeqCst) > 0 && super::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if let Ok(q) = SHOT_QUEUE.try_lock() {
        for label in q.iter() {
            log!("screenshot {label} was not taken (no engine frame after it was requested)");
        }
    }
}

// ---------------------------------------------------------------- window procedure

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let r = super::guarded(|| handle(hwnd, msg, wp, lp));
    // SAFETY: default handling for our own window.
    r.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wp, lp) })
}

fn handle(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    use super::input;
    super::beat();
    // SAFETY: default handling for our own window.
    let default = || unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
    match msg {
        WM_TIMER => LRESULT(0),
        WM_CLOSE => {
            super::request_quit(super::QUIT_USER);
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_SIZE => {
            note_size((lp.0 & 0xFFFF) as u32, ((lp.0 >> 16) & 0xFFFF) as u32);
            LRESULT(0)
        }
        WM_ACTIVATEAPP => {
            input::set_focus(wp.0 != 0);
            default()
        }
        WM_SETFOCUS => {
            input::set_focus(true);
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            input::set_focus(false);
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            input::on_key(wp.0 as u32, lp.0, down);
            if msg == WM_SYSKEYDOWN && wp.0 == VK_F4.0 as usize {
                // Alt+F4 still closes the window.
                return default();
            }
            // Other Alt combinations must not open the window menu, which
            // would stall this thread.
            LRESULT(0)
        }
        WM_SYSCOMMAND if (wp.0 & 0xFFF0) as u32 == SC_KEYMENU => LRESULT(0),
        WM_INPUT => {
            input::on_raw_input(lp);
            default()
        }
        WM_MOUSEMOVE => {
            input::on_cursor(
                (lp.0 & 0xFFFF) as i16 as i32,
                ((lp.0 >> 16) & 0xFFFF) as i16 as i32,
            );
            LRESULT(0)
        }
        WM_SETCURSOR if (lp.0 & 0xFFFF) as u32 == HTCLIENT && input::cursor_hidden() => {
            // SAFETY: hides the cursor over our client area.
            unsafe { SetCursor(None) };
            LRESULT(1)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_APP_RESIZE => {
            resize_swap_chain(hwnd);
            LRESULT(0)
        }
        WM_APP_RESTART => {
            super::RESTART_REASON.store(wp.0 as i64, Ordering::SeqCst);
            super::request_quit(super::QUIT_ENGINE);
            LRESULT(0)
        }
        _ => default(),
    }
}
