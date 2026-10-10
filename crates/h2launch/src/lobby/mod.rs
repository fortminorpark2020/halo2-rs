//! The lobby (`h2launch` with no arguments, or `--lobby`): a window of our
//! own that signs in to h2live, shows the party and the playlists, and
//! plays each match in a second copy of the launcher, then shows the
//! carnage report. The design is in `docs/notes/launcher/lobby.md`.
//!
//! `H2LOBBY_HEADLESS=1` runs it without a window, driven by the script in
//! `H2LOBBY_SCRIPT` (see `Step`), saving a PNG in `H2LOBBY_SHOTS` each
//! time the screen changes. `H2LOBBY_FAKE=1` plays matches with
//! `--fake-engine`, as the lobby always does where there is no MCC.
//! Levels are drawn with Halo 2's icons when a Halo 2 Vista mainmenu.map is
//! found (`ranks`, `H2LOBBY_RANKS`).

pub mod app;
pub mod canvas;
pub mod child;
pub mod names;
pub mod ranks;
pub mod settings;
mod window;

use crate::cli::Args;
use crate::live::Log;
use app::{App, Config, Input};
use canvas::{Canvas, Text};
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
    let (kind, maps) = engine_and_maps(args, &log);
    let fonts = halo2_fonts(args);
    let text = match Text::halo2(fonts.as_deref()) {
        Ok(t) => t,
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
    let ranks = ranks::load(mcc_maps(args).as_deref(), &*log).map(Arc::new);
    let cfg = Config {
        folder,
        exe,
        kind,
        maps,
        server: args.live.clone(),
        instance: args.instance.clone(),
        log: log.clone(),
        ranks,
    };
    let app = App::new(cfg);
    if std::env::var_os("H2LOBBY_HEADLESS").is_some() {
        return headless(app, text, &log);
    }
    hide_own_console(&log);
    match window::run(app, text) {
        Ok(()) => 0,
        Err(e) => {
            log(&format!("lobby: {e}"));
            1
        }
    }
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

/// Where Halo 2's fonts are: `H2LOBBY_H2FONTS` (a folder, or `off` for
/// the system font), else MCC's `halo2\h2_fonts`.
fn halo2_fonts(args: &Args) -> Option<PathBuf> {
    match std::env::var_os("H2LOBBY_H2FONTS") {
        Some(v) if v == "off" => None,
        Some(v) => Some(PathBuf::from(v)),
        None => mcc_fonts(args),
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
/// carnage screen; the run fails if there is none), `shot <name>` and
/// `quit`.
#[derive(Clone, Debug, PartialEq)]
enum Step {
    Press(Input),
    Type(String),
    Wait(String, f64),
    See(String, f64),
    Pick(String),
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
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
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
            "shot" => Step::Shot(arg.to_string()),
            "quit" => Step::Quit,
            _ => return Err(bad("unknown command")),
        };
        steps.push((delay, step));
    }
    Ok(steps)
}

/// The lobby without a window, for tests: runs the script, saves a
/// screenshot whenever the screen changes, and returns 0 if every wait was
/// met.
fn headless(mut app: App, mut text: Text, log: &Log) -> i32 {
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
    let mut code = 0;
    loop {
        let now = Instant::now();
        app.tick(now);
        app.draw(&mut canvas, &mut text);
        let label = app.label();
        if label != last {
            log(&format!("lobby: screen {label}"));
            save(&canvas, &label);
            last = label.clone();
        }
        if app.quitting() {
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
        } else if now >= due {
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
            "# sign in\n0 type alpha one\n0.5 a\n0 wait live 30\n1 down\n0 wait carnage\n0 shot end\n2 quit\n",
        )
        .unwrap();
        assert_eq!(
            s,
            vec![
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
        assert!(parse_script("1 see\n").is_err());
        assert!(parse_script("1 pick\n").is_err());
        assert!(parse_script("a\n").is_err());
        assert!(parse_script("1 jump\n").is_err());
        assert!(parse_script("1 wait\n").is_err());
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
