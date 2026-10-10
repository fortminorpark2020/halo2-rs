//! The Windows program: window, Direct3D 11, loading halo2.dll through its
//! exports, the host object and event manager the engine calls, input,
//! logging and the quit path.
//!
//! Threads:
//! - the window thread (this one) creates the window and the D3D11 device,
//!   loads the DLL, builds the options, then only pumps messages, keeps
//!   time and quits. While the engine runs it never touches D3D except for
//!   `ResizeBuffers` when the engine asks for a resize (host slot 2 sends
//!   it a message and waits), and it never blocks without pumping
//!   messages;
//! - the launch worker makes the engine and calls `initialize_game`;
//! - the engine's own threads call the host slots.

pub mod check;
mod crash;
mod engine;
mod events;
mod gfx;
mod host;
mod input;
mod log;

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, GetExitCodeThread, GetThreadId, TerminateProcess,
    WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MsgWaitForMultipleObjectsEx, PeekMessageW, PostMessageW, TranslateMessage,
    MSG, MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT, WM_NULL, WM_QUIT,
};

use crate::cli::Args;
use crate::maps::MapEntry;
use crate::script::{Action, Base, Script};
use crate::slots::Outcome;
use log::log;

// ---------------------------------------------------------------- time

static START: OnceLock<Instant> = OnceLock::new();

/// Seconds since h2launch started.
pub(crate) fn now() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

pub(crate) fn tid() -> u32 {
    // SAFETY: no arguments, cannot fail.
    unsafe { GetCurrentThreadId() }
}

/// Records the current time once (later calls keep the first time).
fn mark(at: &AtomicU64) {
    let _ = at.compare_exchange(
        0,
        now().max(1e-9).to_bits(),
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
}

fn marked(at: &AtomicU64) -> Option<f64> {
    match at.load(Ordering::SeqCst) {
        0 => None,
        b => Some(f64::from_bits(b)),
    }
}

// ---------------------------------------------------------------- shared state

/// What the run is set up with; fixed before the engine starts.
pub(crate) struct Setup {
    pub args: Args,
    /// MCC's folder, without a trailing backslash.
    pub root: String,
    /// Our folder for the engine's logs, config and temporary files
    /// (host slots 46/47), without a trailing backslash.
    pub engine_dir: String,
    pub log_dir: std::path::PathBuf,
    pub xuid: u64,
    /// Gamertag, without the terminating zero.
    pub name: Vec<u16>,
    pub script: Option<Script>,
    pub map: &'static MapEntry,
}

static SETUP: OnceLock<Setup> = OnceLock::new();

pub(crate) fn setup() -> Option<&'static Setup> {
    SETUP.get()
}

pub(crate) static HWND_RAW: AtomicUsize = AtomicUsize::new(0);
pub(crate) static WINDOW_TID: AtomicU32 = AtomicU32::new(0);
/// end_frame calls.
pub(crate) static FRAMES: AtomicU64 = AtomicU64::new(0);
/// get_input_state calls.
pub(crate) static INPUT_POLLS: AtomicU64 = AtomicU64::new(0);
/// When set_game_state(1) (map loaded) first came, and the first input poll.
pub(crate) static STATE1_AT: AtomicU64 = AtomicU64::new(0);
pub(crate) static FIRST_INPUT_AT: AtomicU64 = AtomicU64::new(0);
pub(crate) static STATES: Mutex<Vec<i32>> = Mutex::new(Vec::new());
/// Unhandled exception code, 0 = none.
pub(crate) static CRASHED: AtomicU32 = AtomicU32::new(0);
/// The engine's game thread handle once initialize_game returned it.
pub(crate) static GAME_THREAD: AtomicUsize = AtomicUsize::new(0);
pub(crate) static LAUNCH_FAILED: AtomicBool = AtomicBool::new(false);

/// Why we are quitting.
pub(crate) const QUIT_NONE: u32 = 0;
pub(crate) const QUIT_USER: u32 = 1;
pub(crate) const QUIT_ENGINE: u32 = 2;
pub(crate) const QUIT_CONSOLE: u32 = 3;
pub(crate) static QUIT: AtomicU32 = AtomicU32::new(QUIT_NONE);
pub(crate) static RESTART_REASON: AtomicI64 = AtomicI64::new(0);

pub(crate) fn request_quit(kind: u32) {
    let _ = QUIT.compare_exchange(QUIT_NONE, kind, Ordering::SeqCst, Ordering::SeqCst);
    wake();
}

/// Wakes the window thread's wait.
pub(crate) fn wake() {
    let h = HWND_RAW.load(Ordering::SeqCst);
    if h != 0 {
        // SAFETY: posting to our own window; failure is harmless.
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_NULL, WPARAM(0), LPARAM(0));
        }
    }
}

pub(crate) fn hwnd() -> HWND {
    HWND(HWND_RAW.load(Ordering::SeqCst) as *mut _)
}

/// The launch step we are in, for crash reports.
pub(crate) static STEP: AtomicUsize = AtomicUsize::new(0);
pub(crate) const STEPS: [&str; 14] = [
    "starting",
    "window and Direct3D",
    "loading halo2.dll",
    "CreateDataAccess",
    "game options",
    "SetLibrarySettings",
    "CreateGameEngine",
    "initialize_graphics",
    "groundhog",
    "preload_common_begin",
    "preload_level_begin",
    "initialize_game",
    "running",
    "quitting",
];

pub(crate) fn set_step(i: usize) {
    STEP.store(i, Ordering::SeqCst);
    log!("step: {}", STEPS[i.min(STEPS.len() - 1)]);
}

pub(crate) fn step_name() -> &'static str {
    STEPS[STEP.load(Ordering::SeqCst).min(STEPS.len() - 1)]
}

/// Seconds since time 0 of the input script, if it has started.
pub(crate) fn script_time() -> Option<f64> {
    let s = setup()?.script.as_ref()?;
    let base = match s.base {
        Base::Launch => 0.0,
        Base::Input => marked(&FIRST_INPUT_AT)?,
        Base::State1 => marked(&STATE1_AT)?,
    };
    Some(now() - base)
}

// ---------------------------------------------------------------- start

pub fn run() -> i32 {
    let _ = START.set(Instant::now());
    let args = match crate::cli::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("h2launch: {e}");
            return 2;
        }
    };
    if args.help {
        println!("{}", crate::cli::USAGE);
        return 0;
    }
    if args.version {
        println!("h2launch {}", crate::BUILD);
        return 0;
    }
    if args.check {
        return check::run(&args);
    }
    launch(args)
}

/// `%LOCALAPPDATA%\h2launch`, or the folder of the exe.
pub(crate) fn log_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("LOCALAPPDATA") {
        return std::path::PathBuf::from(d).join("h2launch");
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| ".".into())
}

unsafe extern "system" fn on_console(kind: u32) -> windows::core::BOOL {
    // Ctrl+C (0), Ctrl+Break (1) or the console closing (2): quit cleanly.
    if kind <= 2 {
        request_quit(QUIT_CONSOLE);
        return true.into();
    }
    false.into()
}

fn random_xuid() -> u64 {
    // SAFETY: CoCreateGuid needs no COM initialisation.
    let seed = unsafe { windows::Win32::System::Com::CoCreateGuid() }
        .map(|g| g.to_u128().to_le_bytes())
        .unwrap_or_else(|_| {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(1);
            (t ^ ((std::process::id() as u128) << 64)).to_le_bytes()
        });
    crate::util::xuid_from_seed(seed)
}

fn launch(args: Args) -> i32 {
    let dir = log_dir();
    let log_path = log::init(&dir);
    log!("h2launch {}", crate::BUILD);
    log!(
        "arguments: {:?}",
        std::env::args().skip(1).collect::<Vec<_>>()
    );
    match &log_path {
        Ok(p) => log!("log: {}", p.display()),
        Err(e) => log!("log file could not be opened in {}: {e}", dir.display()),
    }
    if let Ok(exe) = std::env::current_exe() {
        log!("exe: {} (pid {})", exe.display(), std::process::id());
    }
    crash::install();
    // SAFETY: registers a plain function.
    if let Err(e) = unsafe { SetConsoleCtrlHandler(Some(on_console), true) } {
        log!("SetConsoleCtrlHandler failed: {e}");
    }
    WINDOW_TID.store(tid(), Ordering::SeqCst);

    // The MCC folder.
    let found = engine::find_root(args.mcc.as_deref());
    for (c, ok) in &found.1 {
        log!(
            "MCC folder candidate {} ({}): {}",
            c.path,
            c.source,
            if *ok { "has halo2\\halo2.dll" } else { "no" }
        );
    }
    let Some(root) = found.0 else {
        log!("MCC was not found; pass --mcc <folder> (the one holding halo2\\halo2.dll)");
        finish("MCC not found");
    };
    log!("MCC folder: {root}");

    let map = match &args.map {
        Some(m) => crate::maps::find(m).expect("checked by the parser"),
        None => crate::maps::by_id(crate::maps::LOCKOUT).expect("Lockout is in the table"),
    };
    let xuid = args.xuid.unwrap_or_else(random_xuid);
    log!(
        "player {:?}, XUID {xuid:#018x}{}",
        args.name,
        if args.xuid.is_some() {
            " (from --xuid)"
        } else {
            " (random)"
        }
    );
    let engine_dir = dir.join("engine");
    for kind in 0..4 {
        if let Some(p) = crate::paths::game_folder_halo2(&engine_dir.to_string_lossy(), kind) {
            if let Err(e) = std::fs::create_dir_all(&p) {
                log!("could not create {p}: {e}");
            }
        }
    }
    let script = match &args.input_script {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(text) => match Script::parse(&text) {
                Ok(s) => {
                    log!(
                        "input script {p}: {} steps, time 0 = {:?}, ends at {:.1} s",
                        s.steps.len(),
                        s.base,
                        s.end()
                    );
                    Some(s)
                }
                Err(e) => {
                    log!("input script {p}: {e}");
                    finish("bad input script");
                }
            },
            Err(e) => {
                log!("input script {p}: {e}");
                finish("input script not readable");
            }
        },
        None => None,
    };
    let name: Vec<u16> = args.name.encode_utf16().collect();
    let _ = SETUP.set(Setup {
        args,
        root: root.clone(),
        engine_dir: engine_dir.to_string_lossy().into_owned(),
        log_dir: dir,
        xuid,
        name,
        script,
        map,
    });
    let s = setup().expect("just set");

    // Window and device first (HaloX's order).
    set_step(1);
    let hwnd = match gfx::create_window(s.args.width, s.args.height) {
        Ok(h) => h,
        Err(e) => {
            log!("window: {e}");
            finish("no window");
        }
    };
    HWND_RAW.store(hwnd.0 as usize, Ordering::SeqCst);
    if let Err(e) = gfx::create_device(hwnd) {
        log!("Direct3D 11: {e}");
        finish("no Direct3D 11 device");
    }
    pump();

    // The DLL.
    set_step(2);
    engine::prepare_search(&root);
    let module = match engine::load_halo2(&root) {
        Ok(m) => m,
        Err(e) => {
            log!("halo2.dll: {e}");
            finish("halo2.dll did not load");
        }
    };
    crash::set_filter("after loading halo2.dll");
    pump();
    set_step(3);
    let data_access = match engine::create_data_access(&module) {
        Ok(d) => d,
        Err(e) => {
            log!("CreateDataAccess: {e}");
            finish("CreateDataAccess failed");
        }
    };
    set_step(4);
    let options = match engine::build_options(data_access, s) {
        Ok(o) => o,
        Err(e) => {
            log!("game options: {e}");
            finish("game options could not be built");
        }
    };
    let host = host::init(s);
    pump();

    // The engine, on its own thread so this one keeps pumping messages.
    let worker = engine::Worker {
        module,
        host: host as usize,
        options: options as usize,
    };
    let spawned = std::thread::Builder::new()
        .name("h2launch-launch".into())
        .spawn(move || engine::launch_worker(worker));
    if let Err(e) = spawned {
        log!("launch thread: {e}");
        finish("launch thread could not start");
    }
    main_loop()
}

/// Handles waiting window messages.
fn pump() -> bool {
    let mut quit = false;
    let mut msg = MSG::default();
    // SAFETY: standard message loop on the thread that owns the window.
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                quit = true;
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    if quit {
        request_quit(QUIT_USER);
    }
    quit
}

fn game_thread() -> Option<HANDLE> {
    match GAME_THREAD.load(Ordering::SeqCst) {
        0 => None,
        h => Some(HANDLE(h as *mut _)),
    }
}

fn thread_exited(h: HANDLE) -> bool {
    // SAFETY: h is the engine's thread handle, which stays open.
    unsafe { WaitForSingleObject(h, 0) == WAIT_OBJECT_0 }
}

fn main_loop() -> i32 {
    let s = setup().expect("set before the loop");
    let mut shots: Vec<f64> = s.args.screenshots.clone();
    let mut next_summary = 10.0;
    let mut modules_logged = false;
    let mut script_t = 0.0f64;
    let mut quit_started: Option<(f64, f64)> = None; // (when, deadline)
    let mut game_exit_logged = false;
    let mut attach = input::Attach::default();
    let mut running_since: Option<f64> = None;
    let mut reason = String::new();

    loop {
        pump();
        let t = now();
        let game = game_thread();

        if let Some(h) = game {
            if running_since.is_none() {
                running_since = Some(t);
                set_step(12);
                crash::set_filter("after initialize_game");
            }
            if !game_exit_logged && thread_exited(h) {
                game_exit_logged = true;
                let mut code = 0u32;
                // SAFETY: valid handle, out pointer to a local.
                let _ = unsafe { GetExitCodeThread(h, &mut code) };
                log!("the engine's game thread exited (code {code:#x})");
                if QUIT.load(Ordering::SeqCst) == QUIT_NONE {
                    reason = "the game thread exited".into();
                    request_quit(QUIT_ENGINE);
                }
            }
        }
        if LAUNCH_FAILED.load(Ordering::SeqCst) && quit_started.is_none() {
            reason = "launch failed".into();
            request_quit(QUIT_ENGINE);
        }

        // Unattended test switches.
        if let Some(q) = s.args.quit_after {
            if t >= q && QUIT.load(Ordering::SeqCst) == QUIT_NONE {
                log!("--quit-after {q} s reached");
                reason = format!("--quit-after {q}");
                request_quit(QUIT_USER);
            }
        }
        while let Some(&at) = shots.first() {
            if t < at {
                break;
            }
            shots.remove(0);
            gfx::request_screenshot(format!("{at:.0}s"));
        }
        if let (Some(sc), Some(st)) = (s.script.as_ref(), script_time()) {
            for step in sc.instants(script_t, st) {
                match &step.action {
                    Action::Screenshot => gfx::request_screenshot(format!("script{:.1}s", step.at)),
                    Action::Log(text) => log!("script (line {}): {text}", step.line),
                    Action::Quit => {
                        log!("script (line {}): quit", step.line);
                        if QUIT.load(Ordering::SeqCst) == QUIT_NONE {
                            reason = "input script quit".into();
                            request_quit(QUIT_USER);
                        }
                    }
                    _ => {}
                }
            }
            script_t = st;
        }

        gfx::resize_if_settled();
        input::update_cursor_clip(game.is_some() && quit_started.is_none());
        if s.args.attach_input {
            if let (Some(h), Some(since)) = (game, running_since) {
                attach.try_attach(h, t - since);
            }
        }

        if t >= next_summary {
            next_summary = t + 10.0;
            summary();
        }
        if let Some(since) = running_since {
            if !modules_logged && t - since >= 20.0 {
                modules_logged = true;
                crash::log_modules();
            }
        }

        // Quitting.
        let q = QUIT.load(Ordering::SeqCst);
        if q != QUIT_NONE && quit_started.is_none() {
            set_step(13);
            if reason.is_empty() {
                reason = match q {
                    QUIT_USER => "window closed".into(),
                    QUIT_CONSOLE => "console Ctrl+C".into(),
                    _ => format!(
                        "engine restart_game({})",
                        RESTART_REASON.load(Ordering::SeqCst)
                    ),
                };
            }
            log!("quitting: {reason}");
            let wait = match game {
                Some(h) if !thread_exited(h) => {
                    engine::post_message(13, "quit");
                    engine::post_message(1, "resume");
                    if q == QUIT_ENGINE {
                        15.0
                    } else {
                        5.0
                    }
                }
                _ => 0.0,
            };
            quit_started = Some((t, t + wait));
        }
        if let Some((_, deadline)) = quit_started {
            let done = game.is_none_or(thread_exited);
            if done || t >= deadline {
                if let Some(h) = game {
                    log!(
                        "game thread {}",
                        if thread_exited(h) {
                            "exited"
                        } else {
                            "still running; ending the process"
                        }
                    );
                }
                finish(&reason);
            }
        }

        // Wait for a message, the game thread, or 5 ms.
        let handles: Vec<HANDLE> = game.filter(|&h| !thread_exited(h)).into_iter().collect();
        // SAFETY: valid handles; returns on input, a signalled handle or the timeout.
        unsafe {
            MsgWaitForMultipleObjectsEx(
                (!handles.is_empty()).then_some(handles.as_slice()),
                5,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            );
        }
    }
}

fn summary() {
    let frames = FRAMES.load(Ordering::SeqCst);
    let polls = INPUT_POLLS.load(Ordering::SeqCst);
    let states = STATES.lock().map(|s| s.clone()).unwrap_or_default();
    log!(
        "summary: step={} frames={frames} presents={} input-polls={polls} states={states:?} {}",
        step_name(),
        gfx::presents().map_or("?".to_string(), |p| p.to_string()),
        input::pad_line()
    );
    host::log_summary();
    events::log_summary();
}

/// Writes the summary and the RESULT line, then ends the process.
/// TerminateProcess skips DLL teardown, which is where engine audio
/// threads are known to fault (launch-sequence 4.10).
pub(crate) fn finish(reason: &str) -> ! {
    let game = game_thread();
    let states = STATES.lock().map(|s| s.clone()).unwrap_or_default();
    let frames = FRAMES.load(Ordering::SeqCst);
    let polls = INPUT_POLLS.load(Ordering::SeqCst);
    let crashed = match CRASHED.load(Ordering::SeqCst) {
        0 => None,
        c => Some(c),
    };
    let outcome = Outcome {
        in_game: states.contains(&1) || (frames >= 300 && polls > 0),
        frames,
        crashed,
        presents: gfx::presents(),
        states,
        game_thread_exited: game.map(thread_exited),
        reason: reason.to_string(),
    };
    gfx::finish_screenshots(3.0);
    summary();
    if let Some(h) = game {
        // SAFETY: valid handle.
        log!("game thread id {}", unsafe { GetThreadId(h) });
    }
    input::stop_rumble();
    log!("{}", outcome.line());
    let code = outcome.exit_code();
    // SAFETY: ends this process.
    unsafe {
        let _ = TerminateProcess(GetCurrentProcess(), code as u32);
    }
    std::process::exit(code)
}
