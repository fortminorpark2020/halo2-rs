//! The lobby (`h2launch` with no arguments, or `--lobby`): a window of our
//! own that signs in to h2live, shows the party and the playlists, and
//! plays each match in a second copy of the launcher, then shows the
//! carnage report. The design is in `docs/notes/launcher/lobby.md`.
//!
//! `H2LOBBY_HEADLESS=1` runs it without a window, driven by the script in
//! `H2LOBBY_SCRIPT` (see `Step`), saving a PNG in `H2LOBBY_SHOTS` each
//! time the screen changes. `H2LOBBY_FAKE=1` plays matches with
//! `--fake-engine`, as the lobby always does where there is no MCC.
//! Halo 2's files come from Halo 2 Vista (Project Cartographer's folder,
//! `sources`, `H2LOBBY_VISTA`) first and MCC second: the fonts, the level
//! icons (`ranks`, `H2LOBBY_RANKS`), and the start screen and main menu,
//! drawn by `h2ui` from a mainmenu.map (`H2LOBBY_MENU`), and plain shapes
//! without one.

pub mod app;
pub mod audio;
pub mod canvas;
pub mod child;
pub mod names;
pub mod ranks;
pub mod settings;
pub mod sound;
pub mod sources;
mod window;

use crate::cli::Args;
use crate::live::Log;
use app::{App, Config, Input, Menus};
use canvas::{Canvas, Text};
use sound::MenuSound;
use sources::Vista;
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The program: the lobby, the stand-in engine, or the launcher itself.
pub fn main(raw: Vec<String>) -> i32 {
    let args = match crate::cli::parse(raw.iter().cloned()) {
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
    if args.fake_engine {
        return child::run_fake(&raw);
    }
    if raw.is_empty() || args.lobby {
        return run(&args);
    }
    engine()
}

#[cfg(windows)]
fn engine() -> i32 {
    crate::win::run()
}

#[cfg(not(windows))]
fn engine() -> i32 {
    println!(
        "h2launch runs MCC's engine on Windows only; here only the lobby \
         (--lobby) works, playing its matches with --fake-engine"
    );
    0
}

/// Open the lobby.
pub fn run(args: &Args) -> i32 {
    let folder = settings::folder(args.instance.as_deref());
    if let Err(e) = std::fs::create_dir_all(&folder) {
        eprintln!("h2launch: {}: {e}", folder.display());
        return 1;
    }
    let log = file_log(&folder.join("lobby.log"));
    log(&format!("h2launch {} lobby", crate::BUILD));
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            log(&format!("lobby: where is this launcher? {e}"));
            return 1;
        }
    };
    let no_window = std::env::var_os("H2LOBBY_HEADLESS").is_some();
    let (kind, maps) = engine_and_maps(args, &log);
    // The menus' lines in lobby.txt.
    let prefs = settings::Settings::load(&folder.join("lobby.txt")).menu;
    // Halo 2 Vista's folder (Project Cartographer's), found once: the
    // fonts, the rank icons, the menus and their sounds come from it
    // first.
    let (vista, vista_off) = find_vista(&prefs, &log);
    let usual = sources::usual_dirs();
    // The Vista folders the rank icons and menus may also look in.
    let vista_dirs: &[&Path] = if vista_off { &[] } else { &usual };
    let vista_mainmenu = vista.as_ref().map(Vista::mainmenu);
    let fonts = halo2_fonts(args, vista.as_ref());
    let text = match Text::halo2(fonts.as_deref()) {
        Ok(t) => t,
        // Headless, the text needn't be seen: the script's `see` reads
        // what was drawn by name.
        Err(e) if no_window => {
            log(&format!("lobby: {e}; headless, so text isn't drawn"));
            Text::none()
        }
        Err(e) => {
            log(&format!("lobby: {e}"));
            return 1;
        }
    };
    match (&fonts, text.halo2_slots()) {
        (Some(dir), slots) if !slots.is_empty() => log(&format!(
            "lobby: Halo 2's fonts from {} ({})",
            dir.display(),
            slots.join(", ")
        )),
        (Some(dir), _) => log(&format!(
            "lobby: no Halo 2 fonts in {}; using the system font",
            dir.display()
        )),
        (None, _) => log("lobby: using the system font"),
    }
    let mcc = mcc_maps(args);
    let ranks =
        ranks::load(vista_mainmenu.as_deref(), vista_dirs, mcc.as_deref(), &*log).map(Arc::new);
    let found = Found {
        vista_mainmenu: vista_mainmenu.as_deref(),
        vista_dirs,
        mcc_maps: mcc.as_deref(),
    };
    let menus = Arc::new(menus(&found, fonts.as_deref(), &log));
    // Read on a thread of its own; silent without Vista's files.
    let sound = MenuSound::start(vista.as_ref(), &prefs, !no_window, &log);
    let cfg = Config {
        folder,
        exe,
        kind,
        maps,
        server: args.live.clone(),
        instance: args.instance.clone(),
        log: log.clone(),
        ranks,
        menus,
        vista,
    };
    let app = App::new(cfg);
    if no_window {
        return headless(app, text, sound, &log);
    }
    // Scripts drive the lobby without a window only (tests and checks).
    if std::env::var_os("H2LOBBY_SCRIPT").is_some() {
        log("lobby: H2LOBBY_SCRIPT runs only with H2LOBBY_HEADLESS=1; ignored with a window");
    }
    hide_own_console(&log);
    match window::run(app, text, sound) {
        Ok(()) => 0,
        Err(e) => {
            log(&format!("lobby: {e}"));
            1
        }
    }
}

/// Halo 2 Vista's folder (`sources::find`, with `H2LOBBY_VISTA` and
/// lobby.txt's `vista` in `prefs`), the log saying where or why not; and
/// whether Vista is switched off, when nothing may come from the folders
/// it is usually in either.
fn find_vista(prefs: &settings::MenuPrefs, log: &Log) -> (Option<Vista>, bool) {
    let saved = prefs.vista.as_deref();
    let env = std::env::var_os(sources::ENV);
    let env = env.as_deref();
    let vista = sources::find(env, saved, &sources::usual_dirs());
    let off = sources::off(env, saved);
    for (what, dir) in sources::named_but_missing(env, saved) {
        log(&format!(
            "lobby: {what} names {}, which holds no mainmenu.map (nor a maps folder with one)",
            dir.display()
        ));
    }
    match &vista {
        Some(v) => log(&format!(
            "lobby: Halo 2 Vista files (Project Cartographer) in {}",
            v.maps.display()
        )),
        None => {
            if off {
                let why = match sources::Named::read(env) {
                    sources::Named::Off => "H2LOBBY_VISTA=off",
                    _ => "vista = off in lobby.txt",
                };
                log(&format!("lobby: Halo 2 Vista files off ({why})"));
            }
            log("lobby: no Halo 2 Vista folder; menus from MCC, no music, scene or intro");
        }
    }
    (vista, off)
}

/// The variable that names the mainmenu.map the start screen and main
/// menu are read from (a file, or a folder holding one; MCC's or Halo 2
/// Vista's), or `off` for plain shapes.
const MENU_ENV: &str = "H2LOBBY_MENU";

/// The mainmenu.maps the lobby found, for `menus`: the Halo 2 Vista one
/// (`sources::find`), the Vista folders still to look in (none when
/// Vista is off), and MCC's maps folder.
struct Found<'a> {
    vista_mainmenu: Option<&'a Path>,
    vista_dirs: &'a [&'a Path],
    mcc_maps: Option<&'a Path>,
}

/// The start screen's and main menu's layouts and pictures, from the
/// first mainmenu.map that gives pictures: `H2LOBBY_MENU`'s, else Halo 2
/// Vista's, then MCC's, where the rank icons are looked for
/// (`ranks::candidates`); else the plain shapes. Their text is in Halo 2's
/// fonts from `fonts` (the folder the lobby's text uses), else in the
/// built-in font. Where each piece came from goes in the log.
fn menus(found: &Found, fonts: Option<&Path>, log: &Log) -> Menus {
    let log_line = |line: &str| log(line);
    let env = std::env::var_os(MENU_ENV);
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_deref().and_then(Path::parent);
    let list = ranks::candidates(
        env.as_deref(),
        found.vista_mainmenu,
        found.vista_dirs,
        found.mcc_maps,
        exe_dir,
    );
    let shell = match list {
        None => {
            log("menus: off (H2LOBBY_MENU=off): plain shapes");
            h2ui::shell::Shell::from_tags(None, None)
        }
        Some(list) => {
            let maps = ranks::existing(&list);
            // The first map opened, kept for its layouts (read from its
            // tags, where they read) in case no map gives pictures.
            let mut first: Option<(h2ui::shell::Shell, PathBuf)> = None;
            let mut found = None;
            for map in maps {
                let shell = h2ui::shell::Shell::open(&map, &log_line);
                if !shell.art.is_empty() {
                    found = Some(shell);
                    break;
                }
                log(&format!("menus: no pictures from {}", map.display()));
                if first.is_none() {
                    first = Some((shell, map));
                }
            }
            match (found, first) {
                (Some(shell), _) => shell,
                (None, Some((shell, map))) => {
                    log(&format!(
                        "menus: no mainmenu.map gave pictures: plain shapes, laid out as {} says (above)",
                        map.display()
                    ));
                    shell
                }
                (None, None) => {
                    log("menus: no mainmenu.map found (Halo 2 Vista's or MCC's): plain shapes");
                    h2ui::shell::Shell::from_tags(None, None)
                }
            }
        }
    };
    let fonts = match fonts {
        Some(dir) => {
            let f = h2ui::text::Fonts::read(dir);
            let loaded = f.loaded();
            if loaded.is_empty() {
                log(&format!(
                    "menus: no Halo 2 fonts in {}; the built-in font",
                    dir.display()
                ));
            } else {
                log(&format!(
                    "menus: Halo 2's fonts from {} ({})",
                    dir.display(),
                    loaded.join(", ")
                ));
            }
            f
        }
        None => {
            log("menus: the built-in font (no Halo 2 fonts folder)");
            h2ui::text::Fonts::fallback()
        }
    };
    Menus { shell, fonts }
}

/// Started by a double-click, the launcher has a console window of its own
/// beside the lobby's: let it go (the log is in `lobby.log`). Started from
/// a terminal, it shares that one, and keeps it.
#[cfg(windows)]
fn hide_own_console(log: &Log) {
    use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut ids = [0u32; 4];
    // SAFETY: the buffer is ours and its length is passed with it.
    let sharing = unsafe { GetConsoleProcessList(&mut ids) };
    if sharing == 1 {
        log("lobby: closing the console window (the log is in lobby.log)");
        // SAFETY: no arguments; output after it is dropped by std.
        let _ = unsafe { FreeConsole() };
    }
}

#[cfg(not(windows))]
fn hide_own_console(_: &Log) {}

/// Where Halo 2's fonts are (`fonts_choice`).
fn halo2_fonts(args: &Args, vista: Option<&Vista>) -> Option<PathBuf> {
    let env = std::env::var_os("H2LOBBY_H2FONTS");
    let vista_fonts = vista.and_then(Vista::fonts);
    fonts_choice(env.as_deref(), vista_fonts, mcc_fonts(args))
}

/// Where Halo 2's fonts are: `env` (`H2LOBBY_H2FONTS`: a folder, or `off`
/// for the system font; empty counts as unset), else Halo 2 Vista's
/// `maps\fonts` (`vista_fonts`, when it has its table), else MCC's
/// `halo2\h2_fonts` (`mcc_fonts`).
fn fonts_choice(
    env: Option<&OsStr>,
    vista_fonts: Option<PathBuf>,
    mcc_fonts: Option<PathBuf>,
) -> Option<PathBuf> {
    match env.filter(|v| !v.is_empty()) {
        Some(v) if v == "off" => None,
        Some(v) => Some(PathBuf::from(v)),
        None => vista_fonts.or(mcc_fonts),
    }
}

#[cfg(windows)]
fn mcc_fonts(args: &Args) -> Option<PathBuf> {
    let root = crate::win::mcc_root(args.mcc.as_deref())?;
    Some(PathBuf::from(crate::mccroot::join(
        &root,
        r"halo2\h2_fonts",
    )))
}

#[cfg(not(windows))]
fn mcc_fonts(_: &Args) -> Option<PathBuf> {
    None
}

/// MCC's Halo 2 maps folder, where the rank icons are looked for first.
#[cfg(windows)]
fn mcc_maps(args: &Args) -> Option<PathBuf> {
    let root = crate::win::mcc_root(args.mcc.as_deref())?;
    Some(PathBuf::from(crate::mccroot::join(&root, ranks::MCC_MAPS)))
}

#[cfg(not(windows))]
fn mcc_maps(_: &Args) -> Option<PathBuf> {
    None
}

/// Which engine matches are played on, and the maps the server is told
/// this PC has.
fn engine_and_maps(args: &Args, log: &Log) -> (child::Kind, Vec<(String, u64)>) {
    let fake = !cfg!(windows) || std::env::var_os("H2LOBBY_FAKE").is_some();
    if fake {
        log("lobby: matches play on the stand-in engine (--fake-engine)");
        return (child::Kind::Fake, fake_maps());
    }
    let maps = real_maps(args, log);
    let kind = child::Kind::Real {
        mcc: args.mcc.clone(),
    };
    (kind, maps)
}

#[cfg(windows)]
fn real_maps(args: &Args, log: &Log) -> Vec<(String, u64)> {
    let Some(root) = crate::win::mcc_root(args.mcc.as_deref()) else {
        log("lobby: MCC wasn't found (give its folder with --mcc)");
        return Vec::new();
    };
    let dir = crate::mccroot::join(&root, r"halo2\h2_maps_win64_dx11");
    let maps = crate::live::mcc_maps(Path::new(&dir));
    log(&format!("lobby: {} multiplayer maps in {dir}", maps.len()));
    maps
}

#[cfg(not(windows))]
fn real_maps(_: &Args, _: &Log) -> Vec<(String, u64)> {
    Vec::new()
}

/// Every multiplayer map, with a made-up hash that is the same on every PC
/// (the stand-in engine reads no map).
fn fake_maps() -> Vec<(String, u64)> {
    crate::maps::MULTIPLAYER
        .iter()
        .map(|m| {
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            for b in b"h2launch fake map ".iter().chain(m.file.as_bytes()) {
                hash = (hash ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3);
            }
            (m.file.to_string(), hash)
        })
        .collect()
}

/// A log that writes to `path` (started afresh) and the standard output,
/// each line with the seconds since the lobby started.
fn file_log(path: &Path) -> Log {
    let file = std::fs::File::create(path).ok().map(Mutex::new);
    let start = Instant::now();
    Arc::new(move |line: &str| {
        let line = format!("{:9.3} {line}", start.elapsed().as_secs_f64());
        println!("{line}");
        if let Some(f) = &file {
            if let Ok(mut f) = f.lock() {
                let _ = writeln!(f, "{line}");
            }
        }
    })
}

/// One line of `H2LOBBY_SCRIPT`: `<seconds> <command>`, the seconds counted
/// from the step before. The commands are a key (`up`, `down`, `left`,
/// `right`, `a`, `b`, `x`, `y`, `lb`, `rb`, `tab`, `back`), `type <text>`,
/// `wait <screen> [<seconds>]` (until the screen's label is `<screen>`, or
/// starts with it if it ends in `*`; 120 s at most unless given),
/// `see <text> [<seconds>]` (until text containing `<text>`, ignoring
/// case, is drawn; the last word is the seconds when it is a number),
/// `pick <gamertag>` (select their row on the players, friends, party or
/// carnage screen; the run fails if there is none), `set <control>
/// <value>` (change a controller setting by its `lobby.txt` name, as the
/// settings screen would; the run fails on a bad one), `shot <name>` and
/// `quit`. A `#` after a space starts a comment, as does one at the start
/// of a line.
#[derive(Clone, Debug, PartialEq)]
enum Step {
    Press(Input),
    Type(String),
    Wait(String, f64),
    See(String, f64),
    Pick(String),
    Set(String, String),
    Shot(String),
    Quit,
}

/// What the script waits for.
enum Want {
    /// A screen, by its label (a `*` at the end matches the start).
    Screen(String),
    /// Text drawn on the screen, ignoring case.
    Text(String),
}

fn parse_script(text: &str) -> Result<Vec<(f64, Step)>, String> {
    let mut steps = Vec::new();
    for (n, line) in text.lines().enumerate() {
        // A comment: from a `#` that starts the line or follows a space.
        let line = line
            .char_indices()
            .find(|&(at, c)| c == '#' && (at == 0 || line[..at].ends_with(char::is_whitespace)))
            .map_or(line, |(at, _)| &line[..at]);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let bad = |what: &str| format!("script line {}: {what}: {line}", n + 1);
        let (delay, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let delay: f64 = delay.parse().map_err(|_| bad("no delay"))?;
        let rest = rest.trim();
        let (cmd, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let arg = arg.trim();
        let step = match cmd {
            "up" => Step::Press(Input::Up),
            "down" => Step::Press(Input::Down),
            "left" => Step::Press(Input::Left),
            "right" => Step::Press(Input::Right),
            "a" => Step::Press(Input::A),
            "b" => Step::Press(Input::B),
            "x" => Step::Press(Input::X),
            "y" => Step::Press(Input::Y),
            "lb" => Step::Press(Input::Lb),
            "rb" => Step::Press(Input::Rb),
            "tab" => Step::Press(Input::Tab),
            "back" => Step::Press(Input::Backspace),
            "type" => Step::Type(arg.to_string()),
            "wait" => {
                let mut w = arg.split_whitespace();
                let label = w.next().ok_or_else(|| bad("wait for what?"))?;
                let most = match w.next() {
                    Some(s) => s.parse().map_err(|_| bad("bad seconds"))?,
                    None => 120.0,
                };
                Step::Wait(label.to_string(), most)
            }
            "see" => {
                if arg.is_empty() {
                    return Err(bad("see what?"));
                }
                // The last word is the seconds when it reads as a number
                // (and isn't all there is).
                match arg.rsplit_once(char::is_whitespace) {
                    Some((text, last)) if last.parse::<f64>().is_ok() => {
                        Step::See(text.trim().to_string(), last.parse().unwrap_or(120.0))
                    }
                    _ => Step::See(arg.to_string(), 120.0),
                }
            }
            "pick" if !arg.is_empty() => Step::Pick(arg.to_string()),
            "pick" => return Err(bad("pick whom?")),
            "set" => match arg.split_once(char::is_whitespace) {
                Some((k, v)) if !v.trim().is_empty() => Step::Set(k.into(), v.trim().into()),
                _ => return Err(bad("set what to what?")),
            },
            "shot" => Step::Shot(arg.to_string()),
            "quit" => Step::Quit,
            _ => return Err(bad("unknown command")),
        };
        steps.push((delay, step));
    }
    Ok(steps)
}

/// The lobby without a window, for tests: runs `H2LOBBY_SCRIPT`
/// (`run_script`), with screenshots in `H2LOBBY_SHOTS`.
fn headless(app: App, text: Text, sound: MenuSound, log: &Log) -> i32 {
    let script = match std::env::var("H2LOBBY_SCRIPT") {
        Ok(path) => match std::fs::read_to_string(&path)
            .map_err(|e| format!("{path}: {e}"))
            .and_then(|t| parse_script(&t))
        {
            Ok(s) => s,
            Err(e) => {
                log(&format!("lobby: {e}"));
                return 2;
            }
        },
        Err(_) => Vec::new(),
    };
    let shots: Option<PathBuf> = std::env::var_os("H2LOBBY_SHOTS").map(PathBuf::from);
    run_script(app, text, sound, log, script, shots)
}

/// Runs a script on a lobby with no window, saves a screenshot in
/// `shots` whenever the screen changes (on the start screen and main menu
/// once their intros are over, with the script held until then), and
/// returns 0 if every wait was met. A lobby that quits with steps still
/// to come, or a wait unmet, fails. The sound goes where `sound` says
/// (nowhere, or a WAV file).
fn run_script(
    mut app: App,
    mut text: Text,
    mut sound: MenuSound,
    log: &Log,
    script: Vec<(f64, Step)>,
    shots: Option<PathBuf>,
) -> i32 {
    if let Some(dir) = &shots {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut canvas = Canvas::new(1280, 720);
    let mut shot_no = 0;
    let mut save = |canvas: &Canvas, name: &str| {
        if let Some(dir) = &shots {
            shot_no += 1;
            let path = dir.join(format!("{shot_no:02}-{name}.png"));
            if let Err(e) = canvas.save_png(&path) {
                log(&format!("lobby: screenshot: {e}"));
            }
        }
    };
    let mut steps = script.into_iter();
    let mut next = steps.next();
    let mut due = Instant::now() + secs(next.as_ref().map_or(0.0, |s| s.0));
    let mut waiting: Option<(Want, Instant)> = None;
    let mut last = String::new();
    // The screen's automatic picture is still to be taken.
    let mut pending = false;
    let mut code = 0;
    loop {
        let now = Instant::now();
        app.tick(now);
        sound.update(&app.take_cues(), app.music_on(), app.engine_running());
        app.draw(&mut canvas, &mut text);
        let label = app.label();
        if label != last {
            log(&format!("lobby: screen {label}"));
            // The start screen and main menu come in over a moment: their
            // picture waits until they have, and the script with it.
            if shots.is_some() && !app.menu_settled() {
                pending = true;
            } else {
                save(&canvas, &label);
                pending = false;
            }
            last = label.clone();
        } else if pending && app.menu_settled() {
            save(&canvas, &label);
            pending = false;
        }
        if app.quitting() {
            if next.is_some() || waiting.is_some() {
                log(&format!(
                    "lobby: script: quit before the script ended (on {label})"
                ));
                code = 1;
            }
            break;
        }
        if let Some((want, until)) = &waiting {
            let met = match want {
                Want::Screen(want) => match want.strip_suffix('*') {
                    Some(start) => label.starts_with(start),
                    None => label == *want,
                },
                Want::Text(text) => {
                    let text = text.to_lowercase();
                    app.drawn().iter().any(|d| d.to_lowercase().contains(&text))
                }
            };
            if met {
                waiting = None;
                due = now + secs(next.as_ref().map_or(0.0, |s| s.0));
            } else if now >= *until {
                match want {
                    Want::Screen(want) => log(&format!(
                        "lobby: script: no {want} screen in time (on {label})"
                    )),
                    Want::Text(text) => log(&format!(
                        "lobby: script: {text:?} wasn't drawn in time (on {label})"
                    )),
                }
                code = 1;
                break;
            }
        } else if now >= due && !pending {
            let Some((_, step)) = next.take() else {
                break;
            };
            match step {
                Step::Press(i) => app.input(i),
                Step::Type(t) => t.chars().for_each(|c| app.input(Input::Char(c))),
                Step::Wait(want, most) => waiting = Some((Want::Screen(want), now + secs(most))),
                Step::See(text, most) => waiting = Some((Want::Text(text), now + secs(most))),
                Step::Pick(who) => {
                    if !app.pick(&who) {
                        log(&format!("lobby: script: no {who} to pick on {label}"));
                        code = 1;
                        break;
                    }
                }
                Step::Set(key, value) => {
                    if let Err(e) = app.set_control(&key, &value) {
                        log(&format!("lobby: script: set: {e}"));
                        code = 1;
                        break;
                    }
                }
                Step::Shot(name) => save(&canvas, &name),
                Step::Quit => break,
            }
            next = steps.next();
            if waiting.is_none() {
                due = now + secs(next.as_ref().map_or(0.0, |s| s.0));
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    app.shutdown();
    log(&format!("lobby: headless run done (exit {code})"));
    code
}

fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_read() {
        let s = parse_script(
            "0 wait start 5\n0.3 a\n0 wait main\n0 a\n\
             # sign in\n0 type alpha one\n0.5 a\n0 wait live 30\n1 down\n0 wait carnage\n0 shot end\n2 quit\n",
        )
        .unwrap();
        assert_eq!(
            s,
            vec![
                (0.0, Step::Wait("start".into(), 5.0)),
                (0.3, Step::Press(Input::A)),
                (0.0, Step::Wait("main".into(), 120.0)),
                (0.0, Step::Press(Input::A)),
                (0.0, Step::Type("alpha one".into())),
                (0.5, Step::Press(Input::A)),
                (0.0, Step::Wait("live".into(), 30.0)),
                (1.0, Step::Press(Input::Down)),
                (0.0, Step::Wait("carnage".into(), 120.0)),
                (0.0, Step::Shot("end".into())),
                (2.0, Step::Quit),
            ]
        );
        let s = parse_script(
            "0 lb\n0 rb\n0 see Wants to be your friend 20\n0 see 12 of 100 120\n\
             0 see CHARLIE\n0 see Head to Head: 60\n0 see 60\n0 pick bravo one\n",
        )
        .unwrap();
        assert_eq!(
            s,
            vec![
                (0.0, Step::Press(Input::Lb)),
                (0.0, Step::Press(Input::Rb)),
                (0.0, Step::See("Wants to be your friend".into(), 20.0)),
                (0.0, Step::See("12 of 100".into(), 120.0)),
                (0.0, Step::See("CHARLIE".into(), 120.0)),
                (0.0, Step::See("Head to Head:".into(), 60.0)),
                (0.0, Step::See("60".into(), 120.0)),
                (0.0, Step::Pick("bravo one".into())),
            ]
        );
        let s =
            parse_script("0 set button_layout bumper_jumper\n0 set look_sensitivity 5\n").unwrap();
        assert_eq!(
            s,
            vec![
                (
                    0.0,
                    Step::Set("button_layout".into(), "bumper_jumper".into())
                ),
                (0.0, Step::Set("look_sensitivity".into(), "5".into())),
            ]
        );
        // A comment after a space; a # without one is part of the text.
        let s = parse_script(
            "0 see Head to Head 10 # until it is drawn\n0.2 a\t# accept\n0 see No#1 5\n",
        )
        .unwrap();
        assert_eq!(
            s,
            vec![
                (0.0, Step::See("Head to Head".into(), 10.0)),
                (0.2, Step::Press(Input::A)),
                (0.0, Step::See("No#1".into(), 5.0)),
            ]
        );
        assert!(parse_script("1 set layout\n").is_err());
        assert!(parse_script("1 set\n").is_err());
        assert!(parse_script("1 see\n").is_err());
        assert!(parse_script("1 pick\n").is_err());
        assert!(parse_script("a\n").is_err());
        assert!(parse_script("1 jump\n").is_err());
        assert!(parse_script("1 wait\n").is_err());
    }

    /// Runs `script` on a lobby with no server: its exit code and log.
    fn run(script: &str) -> (i32, Vec<String>) {
        let dir = std::env::temp_dir().join(format!("h2script-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let kept = lines.clone();
        let log: Log = Arc::new(move |l: &str| kept.lock().unwrap().push(l.to_string()));
        let app = App::new(Config {
            folder: dir.clone(),
            exe: PathBuf::from("h2launch"),
            kind: child::Kind::Fake,
            maps: Vec::new(),
            server: None,
            instance: None,
            log: log.clone(),
            ranks: None,
            menus: Arc::new(Menus::flat()),
            vista: None,
        });
        let script = parse_script(script).unwrap();
        let sound = MenuSound::off(&log);
        let code = run_script(app, Text::none(), sound, &log, script, None);
        let _ = std::fs::remove_dir_all(&dir);
        let lines = lines.lock().unwrap().clone();
        (code, lines)
    }

    #[test]
    fn quitting_before_the_script_ends_fails() {
        // B on the start screen opens the quit popup, and A on it quits.
        let quits = "0 wait start 5\n0 b\n0 wait quit 5\n0 a\n";
        assert_eq!(run(quits).0, 0);
        // With steps still to come (a wait, or a key) it fails.
        for more in ["0 wait main 5\n", "1 down\n"] {
            let (code, lines) = run(&format!("{quits}{more}"));
            assert_eq!(code, 1, "{more:?}");
            let said = "lobby: script: quit before the script ended";
            assert!(lines.iter().any(|l| l.starts_with(said)), "{lines:?}");
        }
        // `quit` ends the run itself.
        assert_eq!(run("0 wait start 5\n0.1 quit\n").0, 0);
    }

    #[test]
    fn fonts_come_from_the_variable_then_vista_then_mcc() {
        let (vista, mcc) = (PathBuf::from("vista/fonts"), PathBuf::from("mcc/h2_fonts"));
        let choose = |env: Option<&str>, v: bool, m: bool| {
            fonts_choice(
                env.map(OsStr::new),
                v.then(|| vista.clone()),
                m.then(|| mcc.clone()),
            )
        };
        assert_eq!(
            choose(Some("mine"), true, true),
            Some(PathBuf::from("mine"))
        );
        assert_eq!(choose(Some("off"), true, true), None);
        assert_eq!(choose(None, true, true), Some(vista.clone()));
        assert_eq!(choose(Some(""), true, true), Some(vista.clone()));
        assert_eq!(choose(None, false, true), Some(mcc.clone()));
        assert_eq!(choose(None, false, false), None);
    }

    #[test]
    fn fake_maps_are_the_same_everywhere_and_differ_from_each_other() {
        let a = fake_maps();
        assert_eq!(a, fake_maps());
        assert_eq!(a.len(), crate::maps::MULTIPLAYER.len());
        let mut hashes: Vec<u64> = a.iter().map(|m| m.1).collect();
        hashes.sort();
        hashes.dedup();
        assert_eq!(hashes.len(), a.len());
    }
}
