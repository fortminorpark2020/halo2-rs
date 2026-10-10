//! Finding MCC, setting up the DLL search path, loading `halo2.dll`,
//! calling its three exports, and the launch worker that drives the engine
//! object to a running match. The engine, data-access and variant objects
//! are called through their raw vtables with explicit `this` pointers.

use std::ffi::c_void;
use std::sync::atomic::Ordering;

use windows::core::{PCSTR, PCWSTR};
use windows::Win32::Foundation::{ERROR_SUCCESS, HMODULE};
use windows::Win32::System::Environment::SetCurrentDirectoryW;
use windows::Win32::System::LibraryLoader::{
    AddDllDirectory, GetProcAddress, LoadLibraryExW, SetDefaultDllDirectories, LOAD_LIBRARY_FLAGS,
    LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
    LOAD_LIBRARY_SEARCH_USER_DIRS, LOAD_WITH_ALTERED_SEARCH_PATH,
};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6432KEY,
};

use super::log::log;
use super::Setup;
use crate::mccroot::{self, Candidate};
use crate::options::{self, GameOptions, Launch};
use crate::util::wide;
use crate::vdf;

// ---------------------------------------------------------------- MCC root

/// Steam's own install folder, from the registry.
fn steam_roots() -> Vec<String> {
    let mut out = Vec::new();
    let reads = [
        (
            HKEY_CURRENT_USER,
            r"Software\Valve\Steam",
            "SteamPath",
            false,
        ),
        (
            HKEY_LOCAL_MACHINE,
            r"Software\Valve\Steam",
            "InstallPath",
            true,
        ),
    ];
    for (hive, subkey, value, wow32) in reads {
        if let Some(s) = reg_string(hive, subkey, value, wow32) {
            let s = s.replace('/', "\\");
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

fn reg_string(
    hive: windows::Win32::System::Registry::HKEY,
    subkey: &str,
    value: &str,
    wow32: bool,
) -> Option<String> {
    let sub = wide(subkey);
    let val = wide(value);
    let mut flags = RRF_RT_REG_SZ;
    if wow32 {
        flags |= RRF_SUBKEY_WOW6432KEY;
    }
    let mut size = 0u32;
    // SAFETY: a size query, then a read into our buffer.
    unsafe {
        if RegGetValueW(
            hive,
            PCWSTR(sub.as_ptr()),
            PCWSTR(val.as_ptr()),
            flags,
            None,
            None,
            Some(&mut size),
        ) != ERROR_SUCCESS
            || size == 0
        {
            return None;
        }
        let mut buf = vec![0u16; (size as usize / 2) + 1];
        let mut got = size;
        if RegGetValueW(
            hive,
            PCWSTR(sub.as_ptr()),
            PCWSTR(val.as_ptr()),
            flags,
            None,
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut got),
        ) != ERROR_SUCCESS
        {
            return None;
        }
        Some(crate::util::from_wide(&buf))
    }
}

fn steam_libraries(steam_roots: &[String]) -> Vec<vdf::Library> {
    for root in steam_roots {
        for rel in [
            r"steamapps\libraryfolders.vdf",
            r"config\libraryfolders.vdf",
        ] {
            let path = mccroot::join(root, rel);
            if let Ok(text) = std::fs::read_to_string(&path) {
                match vdf::libraries(&text) {
                    Ok(libs) if !libs.is_empty() => {
                        log!("read {path}: {} Steam libraries", libs.len());
                        return libs;
                    }
                    Ok(_) => {}
                    Err(e) => log!("{path}: {e}"),
                }
            }
        }
    }
    Vec::new()
}

fn has_halo2(root: &str) -> bool {
    std::path::Path::new(&mccroot::join(root, r"halo2\halo2.dll")).is_file()
}

/// Returns the chosen root (first candidate with `halo2\halo2.dll`) and
/// the whole list with a found/not flag, for the log.
pub fn find_root(cli: Option<&str>) -> (Option<String>, Vec<(Candidate, bool)>) {
    let env: Vec<(String, String)> = mccroot::ENV_VARS
        .iter()
        .filter_map(|n| std::env::var(n).ok().map(|v| (n.to_string(), v)))
        .collect();
    let steam = steam_roots();
    let libs = steam_libraries(&steam);
    let cands = mccroot::candidates(cli, &env, &steam, &libs);
    let mut chosen = None;
    let mut listed = Vec::new();
    for c in cands {
        let ok = has_halo2(&c.path);
        if ok && chosen.is_none() {
            chosen = Some(absolute(&c.path));
        }
        listed.push((c, ok));
    }
    (chosen, listed)
}

/// An absolute form of a folder the user may have given relatively (`--mcc
/// .`): AddDllDirectory refuses relative paths, and the working folder
/// changes to the MCC folder before the DLL is loaded.
fn absolute(p: &str) -> String {
    match std::path::absolute(p) {
        Ok(a) => mccroot::normalize_root(&a.to_string_lossy()),
        Err(_) => p.to_string(),
    }
}

// ---------------------------------------------------------------- the build

/// Logs which halo2.dll build is about to load (version, timestamp, size,
/// SHA-256) and warns when it is not the one the research was checked on:
/// every slot number and offset would then be suspect.
pub fn log_build(root: &str) {
    let path = mccroot::join(root, r"halo2\halo2.dll");
    match std::fs::read(&path) {
        Ok(bytes) => match crate::pe::BuildFacts::read(&bytes) {
            Ok(f) => {
                log!("halo2.dll: {}", f.describe());
                let off = f.mismatches();
                if off.is_empty() {
                    log!("halo2.dll is the researched build (1.3528.0.0)");
                    super::BUILD_MATCHES.store(true, Ordering::SeqCst);
                } else {
                    log!(
                        "WARNING: halo2.dll differs from the researched build 1.3528.0.0 in {}; MCC may have updated, and the slot numbers and layouts may no longer hold",
                        off.join(", ")
                    );
                }
            }
            Err(e) => log!("halo2.dll could not be parsed: {e}"),
        },
        Err(e) => log!("{path} could not be read: {e}"),
    }
}

/// Logs where an object's vtable points (read-only: the object's first
/// field), which should be a fixed place in halo2.dll for this build.
pub fn log_vtable(label: &str, obj: *mut c_void, expected_rva: u32) {
    let at = obj as usize;
    if !super::crash::readable(at, 8) {
        log!("{label} object {obj:p}: not readable");
        return;
    }
    // SAFETY: checked readable; the first field of the object.
    let vptr = unsafe { *(at as *const usize) };
    let mut where_ = super::log::StackLine::new();
    super::crash::where_is(vptr, &mut where_);
    let base = super::HALO2_BASE.load(Ordering::SeqCst);
    let verdict = if base != 0 && vptr == base + expected_rva as usize {
        "as expected".to_string()
    } else {
        format!("expected halo2.dll+{expected_rva:#x}")
    };
    log!(
        "{label} object {obj:p}: vtable {} ({verdict})",
        where_.text()
    );
}

// ---------------------------------------------------------------- DLL search

/// Sets the working directory to the MCC root and registers the folders
/// the engine's delay-loaded helpers live in (launch-sequence step 2-5).
pub fn prepare_search(root: &str) {
    // SAFETY: documented loader calls with valid wide strings.
    unsafe {
        if let Err(e) = SetDefaultDllDirectories(
            LOAD_LIBRARY_SEARCH_DEFAULT_DIRS | LOAD_LIBRARY_SEARCH_USER_DIRS,
        ) {
            log!("SetDefaultDllDirectories failed: {e}");
        }
        let root_w = wide(root);
        if SetCurrentDirectoryW(PCWSTR(root_w.as_ptr())).as_bool() {
            log!("working directory set to {root}");
        } else {
            log!(
                "SetCurrentDirectoryW({root}) failed: {}",
                windows::core::Error::from_thread()
            );
        }
        for sub in ["", r"\halo2", r"\MCC\Binaries\Win64"] {
            let dir = format!("{root}{sub}");
            let w = wide(&dir);
            if AddDllDirectory(PCWSTR(w.as_ptr())).is_null() {
                log!(
                    "AddDllDirectory({dir}) failed: {}",
                    windows::core::Error::from_thread()
                );
            } else {
                log!("search path += {dir}");
            }
        }
    }
}

/// Loads `halo2.dll` by absolute path.
pub fn load_halo2(root: &str) -> Result<HMODULE, String> {
    let path = mccroot::join(root, r"halo2\halo2.dll");
    let w = wide(&path);
    // First the search-dir flags, then the altered-search-path fallback.
    let attempts: [(LOAD_LIBRARY_FLAGS, &str); 2] = [
        (
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            "search dirs",
        ),
        (LOAD_WITH_ALTERED_SEARCH_PATH, "altered search path"),
    ];
    let mut last = String::new();
    for (flags, how) in attempts {
        // SAFETY: loads a DLL by absolute path.
        match unsafe { LoadLibraryExW(PCWSTR(w.as_ptr()), None, flags) } {
            Ok(m) if !m.is_invalid() => {
                log!("loaded {path} ({how}) at {:#x}", m.0 as usize);
                return Ok(m);
            }
            Ok(_) => last = "returned a null module".into(),
            Err(e) => last = format!("{how}: {e}"),
        }
    }
    Err(format!("{path}: {last}"))
}

fn proc(module: &HMODULE, name: &str) -> Result<*const c_void, String> {
    let mut n = name.as_bytes().to_vec();
    n.push(0);
    // SAFETY: looks up an export by name.
    match unsafe { GetProcAddress(*module, PCSTR(n.as_ptr())) } {
        Some(p) => Ok(p as *const c_void),
        None => Err(format!("export {name} not found")),
    }
}

// ---------------------------------------------------------------- objects

/// Reads vtable slot `n` of a COM-like object as a function pointer.
///
/// # Safety
/// `obj` must point to an object whose first field is a vtable pointer
/// with at least `n + 1` entries.
unsafe fn slot(obj: *mut c_void, n: usize) -> *const c_void {
    // SAFETY: *obj is the vtable pointer; [n] is one of its entries.
    unsafe {
        let vtable = *(obj as *const *const c_void);
        *(vtable as *const *const c_void).add(n)
    }
}

type Create = unsafe extern "system" fn(*mut *mut c_void) -> u64;

pub fn create_data_access(module: &HMODULE) -> Result<*mut c_void, String> {
    let p = proc(module, "CreateDataAccess")?;
    // SAFETY: the export has this signature (research 2.1).
    let f: Create = unsafe { std::mem::transmute::<*const c_void, Create>(p) };
    let mut out: *mut c_void = std::ptr::null_mut();
    // SAFETY: the export fills `out` with the data-access object.
    let r = unsafe { f(&mut out) };
    log!("CreateDataAccess returned {r:#x}, object {out:p}");
    if out.is_null() {
        return Err("CreateDataAccess gave a null object".into());
    }
    Ok(out)
}

fn create_game_engine(module: &HMODULE) -> Result<*mut c_void, String> {
    let p = proc(module, "CreateGameEngine")?;
    // SAFETY: the export has this signature.
    let f: Create = unsafe { std::mem::transmute::<*const c_void, Create>(p) };
    let mut out: *mut c_void = std::ptr::null_mut();
    // SAFETY: fills `out` with the engine object.
    let r = unsafe { f(&mut out) };
    log!("CreateGameEngine returned {r:#x}, object {out:p}");
    if out.is_null() {
        return Err("CreateGameEngine gave a null object".into());
    }
    Ok(out)
}

/// `SetLibrarySettings`: three wchar_t[85] fields (audio, text, text2),
/// all "en-US". The return looks like a pointer; only 1..0xFFFF is an
/// error (research 2.2 / A10).
fn set_library_settings(module: &HMODULE) -> Result<(), String> {
    let p = proc(module, "SetLibrarySettings")?;
    type Set = unsafe extern "system" fn(*const u16) -> u64;
    // SAFETY: the export takes a pointer to the language block.
    let f: Set = unsafe { std::mem::transmute::<*const c_void, Set>(p) };
    let mut block = [0u16; 85 * 3];
    let lang = wide("en-US");
    for i in 0..3 {
        block[i * 85..i * 85 + lang.len()].copy_from_slice(&lang);
    }
    // SAFETY: the block is 255 u16s as the export expects.
    let r = unsafe { f(block.as_ptr()) };
    log!("SetLibrarySettings returned {r:#x}");
    if (1..=0xFFFF).contains(&r) {
        log!("SetLibrarySettings looks like an error ({r:#x}); carrying on anyway");
    }
    Ok(())
}

// Engine vtable slots (research 4).
const E_FREE: usize = 0;
const E_INIT_GRAPHICS: usize = 1;
const E_INIT_GAME: usize = 2;
const E_POST_MESSAGE: usize = 3;
const E_PRELOAD_COMMON: usize = 4;
const E_PRELOAD_LEVEL: usize = 5;

static ENGINE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Queues an engine message (research 4.1): 0 pause, 1 resume, 13 quit,
/// 14 resize. Called from the window thread.
pub fn post_message(msg: u32, label: &str) {
    let engine = ENGINE.load(Ordering::SeqCst);
    if engine == 0 {
        return;
    }
    type Post = unsafe extern "system" fn(*mut c_void, u32, *const c_void) -> *mut c_void;
    // SAFETY: engine is live; slot 3 has this signature.
    unsafe {
        let p = slot(engine as *mut c_void, E_POST_MESSAGE);
        let f: Post = std::mem::transmute::<*const c_void, Post>(p);
        f(engine as *mut c_void, msg, std::ptr::null());
    }
    log!("post_message({msg}) [{label}]");
}

// ---------------------------------------------------------------- options

/// Builds the game options for an offline Lockout Slayer match: load the
/// variant through data access and copy it in first, then write the base
/// fields, the map and the single local player (game-options C1). Returns
/// the buffer pointer `initialize_game` gets (never the `GameOptions`
/// header). A variant that cannot be loaded stops the launch, as HaloX
/// never starts a multiplayer game without one, unless `--no-variant`.
pub fn build_options(data_access: *mut c_void, s: &Setup) -> Result<*mut u8, String> {
    let launch = Launch::offline(s.map.id, s.xuid);
    let mut opts = Box::new(GameOptions::new());
    opts.apply_base(&launch);

    let variant_path = crate::cli::variant_path(&s.root, s.args.variant.as_deref());
    if s.args.no_variant {
        log!("--no-variant: no game variant loaded; its part of the options stays zero (engine type 0, which is not Slayer)");
    } else {
        let bytes = std::fs::read(&variant_path).map_err(|e| {
            format!("variant {variant_path} could not be read ({e}); pass --variant <file>, or --no-variant to try without one")
        })?;
        log!("variant {variant_path}: {} bytes", bytes.len());
        let name = load_variant(data_access, &bytes, opts.as_mut()).map_err(|e| {
            format!("variant {variant_path} could not be applied ({e}); pass --no-variant to try without one")
        })?;
        log!("variant loaded: {name:?}");
    }

    // These win over whatever the variant wrote.
    opts.apply_match(&launch);
    for w in &s.args.set_option {
        match w.apply(opts.bytes_mut()) {
            Ok(()) => log!("option override applied: {}", w.text),
            Err(e) => log!("option override skipped: {e}"),
        }
    }
    log!(
        "options: map {} (id {}), mode multiplayer, flags {:#06x}, 1 player, XUID {:#018x}",
        s.map.display,
        s.map.id,
        opts.get_u16(options::off::FLAGS),
        s.xuid
    );
    if s.args.diag {
        let b = opts.bytes();
        log!(
            "options 0x000-0x300:\n{}",
            crate::util::hexdump(&b[..0x300], 0)
        );
        log!(
            "options 0x1CF00-0x1CF20 (custom game variant head):\n{}",
            crate::util::hexdump(&b[0x1CF00..0x1CF20], 0x1CF00)
        );
        let nz = crate::util::nonzero_ranges(b, 32);
        log!(
            "options non-zero ranges: {}",
            crate::util::format_ranges(&nz)
        );
    }
    let header = opts.as_ref() as *const GameOptions;
    let buffer = opts.leak_for_engine();
    log!(
        "options buffer at {buffer:p} ({:#x} bytes; the GameOptions header was at {header:p}); initialize_game gets the buffer",
        options::SIZE
    );
    Ok(buffer)
}

// Data-access and variant vtable slots (research 5).
const DA_FREE: usize = 0;
const DA_CREATE_GAME_VARIANT_FROM_FILE: usize = 4;
const V_FREE: usize = 0;
const V_GET_NAME: usize = 1;
const V_COPY_TO_GAME_OPTIONS: usize = 7;

/// Parses a game-variant `.bin` and copies it into the options.
fn load_variant(
    data_access: *mut c_void,
    bytes: &[u8],
    opts: &mut GameOptions,
) -> Result<String, String> {
    type FromFile = unsafe extern "system" fn(*mut c_void, *const u8, u64) -> *mut c_void;
    type CopyTo = unsafe extern "system" fn(*mut c_void, *mut c_void);
    type GetName = unsafe extern "system" fn(*mut c_void) -> *const u16;
    type Free = unsafe extern "system" fn(*mut c_void);

    let before = if super::setup().is_some_and(|s| s.args.diag) {
        Some(opts.bytes().to_vec())
    } else {
        None
    };
    // SAFETY: data_access is the live object; slot 4 parses (buf, len).
    let variant = unsafe {
        let p = slot(data_access, DA_CREATE_GAME_VARIANT_FROM_FILE);
        let f: FromFile = std::mem::transmute::<*const c_void, FromFile>(p);
        f(data_access, bytes.as_ptr(), bytes.len() as u64)
    };
    if variant.is_null() {
        return Err("data access returned no variant".into());
    }
    // SAFETY: variant is live; slot 1 returns its wide name.
    let name = unsafe {
        let p = slot(variant, V_GET_NAME);
        let f: GetName = std::mem::transmute::<*const c_void, GetName>(p);
        let w = f(variant);
        read_wide(w)
    };
    // SAFETY: slot 7 copies the variant into our options buffer.
    unsafe {
        let p = slot(variant, V_COPY_TO_GAME_OPTIONS);
        let f: CopyTo = std::mem::transmute::<*const c_void, CopyTo>(p);
        f(variant, opts.as_mut_ptr() as *mut c_void);
    }
    // SAFETY: slot 0 frees the variant; we keep only the copy in options.
    unsafe {
        let p = slot(variant, V_FREE);
        let f: Free = std::mem::transmute::<*const c_void, Free>(p);
        f(variant);
    }
    if let Some(before) = before {
        let changed = crate::util::changed_ranges(&before, opts.bytes(), 16);
        log!(
            "variant copy changed options ranges: {}",
            crate::util::format_ranges(&changed)
        );
    }
    Ok(name)
}

fn read_wide(p: *const u16) -> String {
    if p.is_null() || !super::crash::readable(p as usize, 2) {
        return String::new();
    }
    // SAFETY: a readable wide string; stop at the terminator or 128 chars.
    unsafe {
        let mut v = Vec::new();
        for i in 0..128 {
            if !super::crash::readable(p.add(i) as usize, 2) {
                break;
            }
            let c = *p.add(i);
            if c == 0 {
                break;
            }
            v.push(c);
        }
        String::from_utf16_lossy(&v)
    }
}

// ---------------------------------------------------------------- the worker

pub struct Worker {
    pub module: HMODULE,
    pub host: usize,
    pub options: usize,
}
// SAFETY: the pointers are long-lived and only used on the worker thread.
unsafe impl Send for Worker {}

/// groundhog.dll's module handle, kept alive for the run (as a usize so
/// the static is Sync).
static GROUNDHOG: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Runs the launch on its own thread so the window thread keeps pumping
/// messages (research step 10 onward).
pub fn launch_worker(w: Worker) {
    if let Err(e) = run_launch(&w) {
        log!("launch failed during {}: {e}", super::step_name());
        super::LAUNCH_FAILED.store(true, Ordering::SeqCst);
        super::wake();
    }
}

fn run_launch(w: &Worker) -> Result<(), String> {
    let s = super::setup().ok_or("no setup")?;

    super::set_step(5);
    set_library_settings(&w.module)?;

    super::set_step(6);
    let engine = create_game_engine(&w.module)?;
    ENGINE.store(engine as usize, Ordering::SeqCst);
    log_vtable("engine", engine, crate::expected::ENGINE_VTABLE_RVA);

    let (device, context, swap) = super::gfx::raw().ok_or("no Direct3D device")?;

    super::set_step(7);
    init_graphics(engine, device, context, swap)?;

    super::set_step(8);
    if s.args.groundhog {
        if let Err(e) = start_groundhog(&s.root, device, context, swap) {
            log!("groundhog: {e} (carrying on without it)");
        }
    } else {
        log!("groundhog not loaded (default; pass --groundhog to load it)");
    }

    super::set_step(9);
    preload_common(engine, device)?;
    super::set_step(10);
    preload_level(engine, -1)?;

    super::set_step(11);
    log_options_handed_over(w.options as *const u8, s);
    let thread = init_game(engine, w.host as *mut c_void, w.options as *mut c_void)?;
    super::GAME_THREAD.store(thread as usize, Ordering::SeqCst);
    // SAFETY: a valid thread handle.
    log!(
        "initialize_game returned thread handle {thread:p} (id {})",
        unsafe {
            windows::Win32::System::Threading::GetThreadId(windows::Win32::Foundation::HANDLE(
                thread,
            ))
        }
    );
    super::wake();
    Ok(())
}

/// What `initialize_game` is about to read, read back through the very
/// pointer it gets (our own memory): the map ids, mode, counts and our
/// XUID, and the first 0x20 bytes.
fn log_options_handed_over(p: *const u8, s: &Setup) {
    // SAFETY: p is the leaked options buffer of options::SIZE bytes.
    let b = unsafe { std::slice::from_raw_parts(p, options::SIZE) };
    let i32_at = |at: usize| i32::from_le_bytes(b[at..at + 4].try_into().unwrap_or([0; 4]));
    let u64_at = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or([0; 8]));
    let xuid = u64_at(options::off::PLAYERS + options::player::XUID);
    log!(
        "initialize_game will read options at {p:p}: map {} / {}, mode {}, flags {:#06x}, players {}, peers {}, player 0 XUID {xuid:#018x}{}, host address {}",
        i32_at(options::off::LEGACY_MAP_ID),
        i32_at(options::off::MAP_ID),
        i32_at(options::off::GAME_MODE),
        u16::from_le_bytes([b[0], b[1]]),
        i32_at(options::off::PLAYER_COUNT),
        i32_at(options::off::PEER_COUNT),
        if xuid == s.xuid { " (ours)" } else { " (NOT ours)" },
        u64_at(options::off::HOST_ADDRESS)
    );
    log!(
        "options as handed over, 0x00-0x20:\n{}",
        crate::util::hexdump(&b[..0x20], 0)
    );
}

fn init_graphics(
    engine: *mut c_void,
    device: *mut c_void,
    context: *mut c_void,
    swap: *mut c_void,
) -> Result<(), String> {
    type Init =
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void);
    // SAFETY: slot 1; the 4th object must be null for halo2 (research 3.3).
    unsafe {
        let p = slot(engine, E_INIT_GRAPHICS);
        let f: Init = std::mem::transmute::<*const c_void, Init>(p);
        f(engine, device, context, swap, std::ptr::null_mut());
    }
    log!("initialize_graphics(device, context, swapchain, null) done");
    Ok(())
}

fn preload_common(engine: *mut c_void, device: *mut c_void) -> Result<(), String> {
    type F = unsafe extern "system" fn(*mut c_void, *mut c_void);
    // SAFETY: slot 4.
    unsafe {
        let p = slot(engine, E_PRELOAD_COMMON);
        let f: F = std::mem::transmute::<*const c_void, F>(p);
        f(engine, device);
    }
    log!("preload_common_begin(device) done");
    Ok(())
}

fn preload_level(engine: *mut c_void, map_id: i32) -> Result<(), String> {
    type F = unsafe extern "system" fn(*mut c_void, i32);
    // SAFETY: slot 5; -1 skips the per-level preload.
    unsafe {
        let p = slot(engine, E_PRELOAD_LEVEL);
        let f: F = std::mem::transmute::<*const c_void, F>(p);
        f(engine, map_id);
    }
    log!("preload_level_begin({map_id}) done");
    Ok(())
}

fn init_game(
    engine: *mut c_void,
    host: *mut c_void,
    options: *mut c_void,
) -> Result<*mut c_void, String> {
    type F = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void;
    // SAFETY: slot 2 starts the game thread and returns its handle.
    let thread = unsafe {
        let p = slot(engine, E_INIT_GAME);
        let f: F = std::mem::transmute::<*const c_void, F>(p);
        f(engine, host, options)
    };
    if thread.is_null() {
        return Err("initialize_game returned null (no game thread)".into());
    }
    Ok(thread)
}

/// groundhog.dll initialised as a second engine (never gets
/// initialize_game), as HaloX does for halo2 (launch-sequence C4).
fn start_groundhog(
    root: &str,
    device: *mut c_void,
    context: *mut c_void,
    swap: *mut c_void,
) -> Result<(), String> {
    let path = mccroot::join(root, r"groundhog\groundhog.dll");
    let w = wide(&path);
    // SAFETY: loads by absolute path with the search dirs already set.
    let module = unsafe {
        LoadLibraryExW(
            PCWSTR(w.as_ptr()),
            None,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        )
    }
    .map_err(|e| format!("{path}: {e}"))?;
    log!("loaded {path} at {:#x}", module.0 as usize);
    GROUNDHOG.store(module.0 as usize, Ordering::SeqCst);
    set_library_settings(&module)?;
    let engine = create_game_engine(&module)?;
    init_graphics(engine, device, context, swap)?;
    log!("groundhog initialised as a second engine");
    Ok(())
}

/// For diagnostics only: the engine object's `free` slot address, so a
/// reader can confirm the vtable looks right. Not called in milestone 1.
#[allow(dead_code)]
pub fn engine_free_slot() -> Option<usize> {
    let e = ENGINE.load(Ordering::SeqCst);
    (e != 0).then(|| {
        // SAFETY: e is a live engine object.
        unsafe { slot(e as *mut c_void, E_FREE) as usize }
    })
}

/// Kept so the DA_FREE constant documents the slot even though milestone 1
/// never frees the data-access object (the process exits instead).
#[allow(dead_code)]
const _DA_FREE: usize = DA_FREE;
