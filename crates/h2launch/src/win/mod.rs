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
//! - the engine's own threads call the host slots;
//! - a watchdog notices when the window thread stops running (a call into
//!   the engine on it that never returns), runs the deferred end of a
//!   stack-overflow crash, and makes sure the run always ends with a
//!   RESULT line.

pub mod check;
mod crash;
mod diag;
mod engine;
mod events;
mod gfx;
mod host;
mod input;
mod log;
mod screen;

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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
use crate::slots::{InGame, Outcome};
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
    /// The networked match (`--session`) and which machine we are.
    pub session: Option<(crate::session::Session, usize)>,
}

/// `--instance`: set before the log opens.
static INSTANCE: OnceLock<String> = OnceLock::new();

pub(crate) fn instance() -> Option<&'static str> {
    INSTANCE.get().map(|s| s.as_str())
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
/// halo2.dll's load address, 0 before it is loaded.
pub(crate) static HALO2_BASE: AtomicUsize = AtomicUsize::new(0);
/// The halo2.dll file matched the researched build (version, timestamp,
/// size and hash).
pub(crate) static BUILD_MATCHES: AtomicBool = AtomicBool::new(false);
/// When the window thread last ran (seconds, as f64 bits): its main loop
/// and its window procedure, which a one-second timer keeps ticking even
/// inside Windows' modal move and size loops.
pub(crate) static HEARTBEAT: AtomicU64 = AtomicU64::new(0);
/// A stack overflow the crash filter could not finish on its own stack;
/// the watchdog ends the run for it.
pub(crate) static DEFERRED_CRASH: AtomicU32 = AtomicU32::new(0);
/// The console window is closing: Windows ends the process about 5 s after
/// it tells us, so the quit sequence is kept short.
pub(crate) static CONSOLE_CLOSING: AtomicBool = AtomicBool::new(false);
/// finish() has started (it runs once), and when.
static FINISHING: AtomicBool = AtomicBool::new(false);
static FINISH_AT: AtomicU64 = AtomicU64::new(0);

pub(crate) fn beat() {
    HEARTBEAT.store(now().max(1e-9).to_bits(), Ordering::SeqCst);
}

thread_local! {
    /// How many `guarded` calls this thread is inside.
    static GUARD_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Runs `f`, catching a panic so it never crosses into the engine (a panic
/// through an `extern "system"` function aborts the process with no log).
/// The panic hook logs the message; this returns None for it.
pub(crate) fn guarded<T>(f: impl FnOnce() -> T + std::panic::UnwindSafe) -> Option<T> {
    GUARD_DEPTH.with(|d| d.set(d.get() + 1));
    let r = std::panic::catch_unwind(f);
    GUARD_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    r.ok()
}

fn in_guard() -> bool {
    GUARD_DEPTH.try_with(|d| d.get() > 0).unwrap_or(false)
}

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
/// The furthest launch step reached before quitting, for the RESULT line.
static REACHED: AtomicUsize = AtomicUsize::new(0);
const STEP_QUITTING: usize = 13;
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
    if i < STEP_QUITTING {
        REACHED.fetch_max(i, Ordering::SeqCst);
    }
    beat();
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
    let mut args = match crate::cli::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("h2launch: {e}");
            return 2;
        }
    };
    // Before anything changes the working folder.
    crate::cli::absolutize_paths(&mut args);
    if args.help {
        println!("{}", crate::cli::USAGE);
        return 0;
    }
    if args.version {
        println!("h2launch {}", crate::BUILD);
        return 0;
    }
    if let Some(i) = &args.instance {
        let _ = INSTANCE.set(i.clone());
    }
    if args.check {
        return check::run(&args);
    }
    launch(args)
}

/// `%LOCALAPPDATA%\h2launch` (or the folder of the exe), with
/// `\<instance>` after it for `--instance`.
pub(crate) fn log_dir() -> std::path::PathBuf {
    let base = match std::env::var_os("LOCALAPPDATA") {
        Some(d) => std::path::PathBuf::from(d).join("h2launch"),
        None => std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| ".".into()),
    };
    match instance() {
        Some(i) => base.join(i),
        None => base,
    }
}

unsafe extern "system" fn on_console(kind: u32) -> windows::core::BOOL {
    match kind {
        // Ctrl+C, Ctrl+Break: quit cleanly.
        0 | 1 => {
            request_quit(QUIT_CONSOLE);
            true.into()
        }
        // The console window closing (or logoff/shutdown): Windows ends
        // the process as soon as this returns, or about 5 s later. Wait
        // here so the main loop can write the summary and the RESULT line;
        // finish() ends the process itself.
        2 | 5 | 6 => {
            CONSOLE_CLOSING.store(true, Ordering::SeqCst);
            request_quit(QUIT_CONSOLE);
            for _ in 0..45 {
                std::thread::sleep(Duration::from_millis(100));
            }
            true.into()
        }
        _ => false.into(),
    }
}

/// Logs a Rust panic with its place before anything else happens. Inside
/// a guarded host slot or window message it is caught and the call returns
/// a default; anywhere else it ends the run through finish(), so the log
/// still gets its RESULT line (a panic through an `extern "system"`
/// function would otherwise abort with nothing logged).
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let msg = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".into());
        let at = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "?".into());
        if in_guard() {
            log!("PANIC (caught; the call returns a default): {msg} at {at}");
        } else {
            log!(
                "PANIC: {msg} at {at} during {}; ending the run",
                step_name()
            );
            finish(&format!("panic: {msg}"));
        }
    }));
}

/// A click in a classic console with QuickEdit on pauses all output to
/// it, and every thread that logs (engine threads in host slots included)
/// would wait. Turn it off for this run.
fn quick_edit_off() {
    use windows::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, SetConsoleMode, CONSOLE_MODE, ENABLE_EXTENDED_FLAGS,
        ENABLE_QUICK_EDIT_MODE, STD_INPUT_HANDLE,
    };
    // SAFETY: console mode queries on our own standard input.
    unsafe {
        let Ok(h) = GetStdHandle(STD_INPUT_HANDLE) else {
            return;
        };
        let mut mode = CONSOLE_MODE(0);
        if GetConsoleMode(h, &mut mode).is_err() {
            return; // not a console
        }
        if mode.0 & ENABLE_QUICK_EDIT_MODE.0 == 0 {
            return;
        }
        let new = CONSOLE_MODE((mode.0 & !ENABLE_QUICK_EDIT_MODE.0) | ENABLE_EXTENDED_FLAGS.0);
        match SetConsoleMode(h, new) {
            Ok(()) => log!(
                "console QuickEdit turned off for this run (a click there would pause the log)"
            ),
            Err(e) => log!("console QuickEdit could not be turned off: {e}"),
        }
    }
}

/// Moves the previous run's screenshots into `prev-shots`, so a run that
/// never renders cannot be mistaken for one that did.
fn move_old_shots(dir: &std::path::Path) {
    let is_shot = |p: &std::path::Path| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("h2launch-shot-") && n.ends_with(".png"))
    };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let old: Vec<std::path::PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| is_shot(p))
        .collect();
    if old.is_empty() {
        return;
    }
    let prev = dir.join("prev-shots");
    if let Ok(rd) = std::fs::read_dir(&prev) {
        for e in rd.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let _ = std::fs::create_dir_all(&prev);
    let mut moved = 0;
    for p in old {
        let to = prev.join(p.file_name().unwrap_or_default());
        if std::fs::rename(&p, &to).is_ok() || std::fs::remove_file(&p).is_ok() {
            moved += 1;
        }
    }
    log!(
        "{moved} screenshot(s) from the previous run moved to {}",
        prev.display()
    );
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

fn launch(mut args: Args) -> i32 {
    let dir = log_dir();
    let log_path = log::init(&dir);
    log!("h2launch {}", crate::BUILD);
    log!(
        "arguments: {:?}",
        std::env::args().skip(1).collect::<Vec<_>>()
    );
    install_panic_hook();
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
    quick_edit_off();
    WINDOW_TID.store(tid(), Ordering::SeqCst);
    beat();
    move_old_shots(&dir);

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

    // --live: the server makes the match; it names the map and variant.
    let live = args.live.clone().map(|server| {
        let maps_dir = crate::mccroot::join(&root, r"halo2\h2_maps_win64_dx11");
        let log: crate::live::Log = std::sync::Arc::new(|l: &str| log!("{l}"));
        let ready = crate::live::prepare(
            &server,
            args.playlist,
            Duration::from_secs_f64(args.live_wait),
            &args.name,
            std::path::Path::new(&maps_dir),
            &dir,
            log,
        )
        .unwrap_or_else(|e| {
            log!("live: {e}");
            finish("no match from the server");
        });
        if crate::maps::find(&ready.map).is_none() {
            log!(
                "live: the server picked map {:?}, which isn't known",
                ready.map
            );
            crate::live::install(ready.link);
            crate::live::close();
            finish("unknown map from the server");
        }
        if args.map.is_some() || args.variant.is_some() {
            log!("live: the server's map and variant are used, not --map or --variant");
        }
        args.map = Some(ready.map.clone());
        args.variant = Some(ready.variant.clone());
        ready
    });

    let map = match &args.map {
        Some(m) => crate::maps::find(m).expect("checked by the parser"),
        None => crate::maps::by_id(crate::maps::LOCKOUT).expect("Lockout is in the table"),
    };
    let session = match (&args.session, live) {
        (_, Some(ready)) => {
            log!("{}", ready.session.describe(ready.me));
            crate::live::install(ready.link);
            Some((ready.session, ready.me))
        }
        (Some(p), None) => {
            let text = std::fs::read_to_string(p).unwrap_or_else(|e| {
                log!("session file {p}: {e}");
                finish("session file not readable");
            });
            let sess = crate::session::Session::parse(&text).unwrap_or_else(|e| {
                log!("session file {p}: {e}");
                finish("bad session file");
            });
            let me = sess.resolve_me(args.me).unwrap_or_else(|e| {
                log!("session file {p}: {e}");
                finish("bad session file");
            });
            log!("session file {p}");
            log!("{}", sess.describe(me));
            Some((sess, me))
        }
        (None, None) => {
            if args.me.is_some() {
                log!("--me does nothing without --session");
            }
            None
        }
    };
    let local = session.as_ref().and_then(|(s, me)| s.local_player(*me));
    let (xuid, xuid_from) = match (args.xuid, local) {
        (Some(x), _) => (x, " (from --xuid)"),
        (None, Some(p)) => (p.xuid, " (from the session)"),
        (None, None) => (random_xuid(), " (random)"),
    };
    if !args.name_set {
        if let Some(n) = local.and_then(|p| p.name.clone()) {
            args.name = n;
        }
    }
    log!("player {:?}, XUID {xuid:#018x}{xuid_from}", args.name);
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
        session,
    });
    let s = setup().expect("just set");
    if let Err(e) = std::thread::Builder::new()
        .name("h2launch-watchdog".into())
        .spawn(watchdog)
    {
        log!("watchdog thread: {e}");
    }

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
    engine::log_build(&root);
    engine::prepare_search(&root);
    let module = match engine::load_halo2(&root) {
        Ok(m) => m,
        Err(e) => {
            log!("halo2.dll: {e}");
            finish("halo2.dll did not load");
        }
    };
    HALO2_BASE.store(module.0 as usize, Ordering::SeqCst);
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
    engine::log_vtable(
        "data access",
        data_access,
        crate::expected::DATA_ACCESS_VTABLE_RVA,
    );
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
    // The relay, before the engine can send anything.
    host::start_net(s);
    if let Some(net) = host::net() {
        if net.state().is_some() {
            let until = now() + s.args.relay_wait;
            while net.state() == Some(h2relay::State::Joining) && now() < until {
                beat();
                pump();
                std::thread::sleep(Duration::from_millis(20));
            }
            match net.state() {
                Some(h2relay::State::Joined) => log!("relay: in the room"),
                Some(st) => log!(
                    "relay: {st:?} after {:.0} s; starting the engine anyway (sends fail until the relay takes us in)",
                    s.args.relay_wait
                ),
                None => {}
            }
        }
    }

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
    // Below 0 so script steps at time 0 fire.
    let mut script_t = -1.0f64;
    let mut key_diag = diag::KeyDiag::default();
    let mut watches = diag::Watches::new(&s.args.watch);
    let mut quit_started: Option<(f64, f64)> = None; // (when, deadline)
    let mut game_exit_logged = false;
    let mut attach = input::Attach::default();
    let mut running_since: Option<f64> = None;
    let mut reason = String::new();

    loop {
        beat();
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
            if let Some(h) = game {
                attach.try_attach(h, t);
            }
        }
        if s.args.diag {
            key_diag.tick(t);
        }
        watches.tick(t);

        if t >= next_summary {
            next_summary = t + 10.0;
            summary(true);
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
                    QUIT_CONSOLE if CONSOLE_CLOSING.load(Ordering::SeqCst) => {
                        "console window closed".into()
                    }
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
                    if CONSOLE_CLOSING.load(Ordering::SeqCst) {
                        1.5
                    } else if q == QUIT_ENGINE {
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

/// The 10-second summary. `periodic` adds the checks that only make
/// sense while the run goes on: our crash filter still in place, and when
/// no frame came since the last summary, the process's own windows (an
/// engine error dialog would show up there).
fn summary(periodic: bool) {
    static LAST_FRAMES: AtomicU64 = AtomicU64::new(u64::MAX);
    let frames = FRAMES.load(Ordering::SeqCst);
    let polls = INPUT_POLLS.load(Ordering::SeqCst);
    let states = STATES.try_lock().map(|s| s.clone()).unwrap_or_default();
    log!(
        "summary: step={} frames={frames} presents={} input-polls={polls} states={states:?} {} {}",
        step_name(),
        gfx::presents().map_or("?".to_string(), |p| p.to_string()),
        input::pad_line(),
        host::load_line()
    );
    host::log_summary();
    events::log_summary();
    if let Some(net) = host::net() {
        let lines = net.summary();
        let n = if periodic { 1 } else { lines.len() };
        for l in lines.iter().take(n) {
            log!("{l}");
        }
    }
    if periodic {
        crash::refresh_filter();
        let before = LAST_FRAMES.swap(frames, Ordering::SeqCst);
        if before == frames && STEP.load(Ordering::SeqCst) >= 9 {
            log!("no engine frame since the last summary; the process's windows:");
            screen::log_windows(true);
        }
    }
}

/// What the RESULT line says.
fn outcome(reason: &str) -> Outcome {
    let states = STATES.try_lock().map(|s| s.clone()).unwrap_or_default();
    let frames = FRAMES.load(Ordering::SeqCst);
    let polls = INPUT_POLLS.load(Ordering::SeqCst);
    let crashed = match CRASHED.load(Ordering::SeqCst) {
        0 => None,
        c => Some(c),
    };
    Outcome {
        in_game: InGame::judge(&states, frames, polls),
        frames,
        crashed,
        presents: gfx::presents(),
        states,
        game_thread_exited: game_thread().map(thread_exited),
        step: STEPS[REACHED.load(Ordering::SeqCst).min(STEPS.len() - 1)].to_string(),
        reason: reason.to_string(),
    }
}

/// Writes the summary and the RESULT line, then ends the process. It runs
/// once; a second caller (another thread, or a crash while finishing)
/// waits for the first, and the watchdog ends the process if the first
/// never gets there. TerminateProcess skips DLL teardown, which is where
/// engine audio threads are known to fault (launch-sequence 4.10).
pub(crate) fn finish(reason: &str) -> ! {
    if FINISHING.swap(true, Ordering::SeqCst) {
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    FINISH_AT.store(now().max(1e-9).to_bits(), Ordering::SeqCst);
    // A match from --live: tell the server, before the slower steps.
    crate::live::close();
    let closing = CONSOLE_CLOSING.load(Ordering::SeqCst);
    gfx::finish_screenshots(if closing { 0.5 } else { 3.0 });
    let out = outcome(reason);
    summary(false);
    log!("the process's windows at the end:");
    screen::log_windows(false);
    if out.frames == 0 || gfx::pending_screenshots() > 0 {
        screen::save_window_capture("final-gdi");
    }
    if let Some(h) = game_thread() {
        // SAFETY: valid handle.
        log!("game thread id {}", unsafe { GetThreadId(h) });
    }
    input::stop_rumble();
    log!("{}", out.line());
    let code = out.exit_code();
    // SAFETY: ends this process.
    unsafe {
        let _ = TerminateProcess(GetCurrentProcess(), code as u32);
    }
    std::process::exit(code)
}

/// The watchdog thread (see the module notes).
fn watchdog() {
    let quit_after = setup().and_then(|s| s.args.quit_after);
    // --no-watchdog keeps only the duties that never end a healthy run.
    let judge = setup().is_none_or(|s| s.args.watchdog);
    let mut warned_for: Option<u64> = None;
    loop {
        std::thread::sleep(Duration::from_millis(200));
        let t = now();
        if FINISHING.load(Ordering::SeqCst) {
            if marked(&FINISH_AT).is_some_and(|at| t - at > 15.0) {
                log!("watchdog: the end of the run did not finish in 15 s; ending the process");
                log!("{}", outcome("finish did not complete").line());
                // SAFETY: ends this process.
                unsafe {
                    let _ = TerminateProcess(GetCurrentProcess(), 3);
                }
            }
            continue;
        }
        if DEFERRED_CRASH.load(Ordering::SeqCst) != 0 {
            finish("unhandled exception (stack overflow)");
        }
        if !judge {
            continue;
        }
        let hb_bits = HEARTBEAT.load(Ordering::SeqCst);
        let stale = if hb_bits == 0 {
            0.0
        } else {
            t - f64::from_bits(hb_bits)
        };
        if stale >= 10.0 && warned_for != Some(hb_bits) {
            warned_for = Some(hb_bits);
            log!(
                "watchdog: the window thread has not run for {stale:.0} s (step: {}); a call it made may be stuck",
                step_name()
            );
            summary(false);
        }
        if stale >= 30.0 {
            finish(&format!(
                "watchdog: window thread stuck during {}",
                step_name()
            ));
        }
        if let Some(q) = quit_after {
            if t >= q + 30.0 {
                finish(&format!(
                    "watchdog: --quit-after {q} passed 30 s ago and the run had not ended"
                ));
            }
        }
    }
}
