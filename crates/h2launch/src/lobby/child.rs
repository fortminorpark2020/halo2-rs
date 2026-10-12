//! The engine for one match: a second copy of the launcher, started by the
//! lobby with the match's session (`--session ... --events`), whose
//! standard output says what the engine does. A line `quit` on its
//! standard input closes the engine as closing its window would, and so
//! does that input closing (the lobby is gone).
//!
//! `--fake-engine` stands in for it where there is no MCC (Linux, tests).
//! Its environment variables, for tests:
//!
//! - `H2LOBBY_FAKE_SECONDS`: how long the game lasts after the map loads
//!   (default 5).
//! - `H2LOBBY_FAKE_QUIT=1`: quit halfway instead of finishing.
//! - `H2LOBBY_FAKE_MENU`: keys for the in-game GAME MENU (the real
//!   `gamemenu::GameMenu`, for the game `--game-kind` and the session
//!   say), as `<seconds> <key>` separated by `;`, the seconds counted from
//!   the map loading: `1.0 start;1.5 a;2.0 up;2.5 a` opens the menu and
//!   leaves the game. The keys are `start back a b y up down left right`;
//!   `start` opens a closed menu. A change of controls is said as the
//!   real engine says it (`H2EVENT controls ...`, once the player is done
//!   with CONTROLLER SETTINGS: `gamemenu::save`). Leave Game says
//!   `H2EVENT left game` and closes without `ended`; End Game says `left
//!   end`, then `ended` with the results, as the host's real engine does
//!   as it quits (PC check 14), and closes.

use crate::controls::Controls;
use crate::gamemenu::save::Saves;
use crate::gamemenu::{self, Effect, GameKind, GameMenu, Key, Leave, MenuContext, Pickup};
use crate::live::{self, Engine};
use crate::session::Session;
use h2net::live::{LauncherMatch, LauncherPlayerResult, CUSTOM_GAME};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Which engine the lobby starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// halo2.dll, through this launcher with `--session` (Windows).
    Real { mcc: Option<String> },
    /// `--fake-engine`.
    Fake,
}

/// What the engine's launcher said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Said {
    Event(Engine),
    /// A log line.
    Line(String),
    /// Its output closed: it has ended or is ending.
    Done,
}

/// A match the lobby started.
pub struct Match {
    pub kind: Kind,
    pub map: String,
    pub variant: String,
    pub session: Session,
    pub me: usize,
    pub gamertag: String,
    pub instance: Option<String>,
    /// The player's controls (the lobby's Settings).
    pub controls: Controls,
    /// What kind of game it is, for the engine's GAME MENU.
    pub game_kind: GameKind,
}

/// The kind of game a match is, for the engine's GAME MENU: a ranked
/// playlist's, a custom game, or else a social playlist's.
pub fn game_kind(m: &LauncherMatch) -> GameKind {
    if m.ranked {
        GameKind::Ranked
    } else if m.playlist == CUSTOM_GAME {
        GameKind::Custom
    } else {
        GameKind::Social
    }
}

impl Match {
    /// The launcher's arguments for this match, with the session file at
    /// `session_file`.
    pub fn args(&self, session_file: &Path) -> Vec<String> {
        let mut a = Vec::new();
        if self.kind == Kind::Fake {
            a.push("--fake-engine".to_string());
        }
        a.extend([
            "--session".into(),
            session_file.display().to_string(),
            "--me".into(),
            self.me.to_string(),
            "--map".into(),
            self.map.clone(),
            "--variant".into(),
            self.variant.clone(),
            "--name".into(),
            self.gamertag.clone(),
            "--events".into(),
            "--game-kind".into(),
            self.game_kind.key().into(),
        ]);
        a.extend(self.controls.args());
        if let Some(i) = &self.instance {
            a.extend(["--instance".into(), i.clone()]);
        }
        if let Kind::Real { mcc: Some(m) } = &self.kind {
            a.extend(["--mcc".into(), m.clone()]);
        }
        a
    }
}

/// How many results blocks the launcher's folder keeps.
const RESULTS_KEPT: usize = 30;

/// `folder`'s `results` folder, made if need be, with only the newest
/// `RESULTS_KEPT - 1` blocks left in it so the next game's fits.
fn results_folder(folder: &Path) -> Option<PathBuf> {
    let dir = folder.join("results");
    std::fs::create_dir_all(&dir).ok()?;
    let mut blocks: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| {
            let e = e.ok()?;
            let name = e.file_name();
            let name = name.to_str()?;
            if !(name.starts_with("result-") && name.ends_with(".bin")) {
                return None;
            }
            Some((e.metadata().ok()?.modified().ok()?, e.path()))
        })
        .collect();
    blocks.sort();
    let extra = blocks.len().saturating_sub(RESULTS_KEPT - 1);
    for (_, path) in blocks.into_iter().take(extra) {
        let _ = std::fs::remove_file(path);
    }
    Some(dir)
}

/// The running engine.
pub struct Running {
    child: Child,
    stdin: Option<ChildStdin>,
    said: Receiver<Said>,
    done: bool,
    /// When it was asked to quit.
    asked: Option<Instant>,
    /// It was started with its GAME MENU (`game_menu_on`).
    game_menu: bool,
    /// The controls it was started with (`launch_controls`).
    controls: Controls,
}

/// Whether an engine started with these extra words
/// (`H2LOBBY_ENGINE_ARGS`) and this `H2LAUNCH_OVERLAY_FAIL` has its
/// in-game GAME MENU: not with `--no-game-menu`, nor with the overlay
/// failure's stand-in set to 1 (the engine inherits the lobby's
/// environment).
pub fn game_menu_on(extra: &str, overlay_fail: Option<&std::ffi::OsStr>) -> bool {
    !extra.split_whitespace().any(|w| w == "--no-game-menu")
        && overlay_fail.is_none_or(|v| v != "1")
}

/// The controls an engine started for a match with these extra words
/// (`H2LOBBY_ENGINE_ARGS`) runs with: the match's (the lobby's
/// Settings), with any control flag among the words put over them (the
/// last flag given wins, and the extra words come last). The lobby
/// keeps them so a test's one-run flag (`--no-vibration`, say) isn't
/// taken for a change the player made in the GAME MENU.
pub fn launch_controls(extra: &str, base: Controls) -> Controls {
    crate::cli::parse(extra.split_whitespace())
        .map(|a| a.controls.over(base))
        .unwrap_or(base)
}

impl Running {
    /// Write the session into `folder` and start `exe` (this launcher) on
    /// the match, with the words `extra` after the match's own (for tests:
    /// `H2LOBBY_ENGINE_ARGS`, as `--set-option`, `--pad` or a control
    /// flag).
    pub fn start(exe: &Path, folder: &Path, m: &Match, extra: &str) -> Result<Running, String> {
        let file: PathBuf = folder.join("match-session.txt");
        let mut session = m.session.clone();
        session.me = Some(m.me);
        std::fs::write(&file, session.to_text()).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut cmd = Command::new(exe);
        // The engine's results blocks are kept in the launcher's folder
        // (the newest few), so their layout can be worked out further
        // from real games. They are the engine's data: they stay on this
        // PC.
        if std::env::var_os("H2LAUNCH_RESULT_DUMP").is_none() {
            if let Some(dir) = results_folder(folder) {
                cmd.env("H2LAUNCH_RESULT_DUMP", dir);
            }
        }
        cmd.args(m.args(&file))
            .args(extra.split_whitespace())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            // Its output goes to the lobby: no console window of its own.
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", exe.display()))?;
        let_to_front(child.id());
        let stdin = child.stdin.take();
        let (tx, rx) = mpsc::channel();
        if let Some(out) = child.stdout.take() {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    let said = match live::parse_event_line(&line) {
                        Some(e) => Said::Event(e),
                        None => Said::Line(line),
                    };
                    if tx.send(said).is_err() {
                        return;
                    }
                }
                let _ = tx.send(Said::Done);
            });
        }
        if let Some(err) = child.stderr.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(err).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(Said::Line(line)).is_err() {
                        return;
                    }
                }
            });
        }
        Ok(Running {
            child,
            stdin,
            said: rx,
            done: false,
            asked: None,
            game_menu: game_menu_on(extra, std::env::var_os("H2LAUNCH_OVERLAY_FAIL").as_deref()),
            controls: launch_controls(extra, m.controls),
        })
    }

    /// It was started with its in-game GAME MENU.
    pub fn game_menu(&self) -> bool {
        self.game_menu
    }

    /// The controls it was started with (`launch_controls`).
    pub fn launch_controls(&self) -> Controls {
        self.controls
    }

    /// Let its window come to the front (again: the engine's window
    /// opens a few seconds after the start).
    pub fn let_to_front(&self) {
        let_to_front(self.child.id());
    }

    /// What it said since the last call.
    pub fn poll(&mut self) -> Vec<Said> {
        let out: Vec<Said> = self.said.try_iter().collect();
        if out.contains(&Said::Done) {
            self.done = true;
        }
        out
    }

    /// It has exited (its output closed and the process is gone).
    pub fn exited(&mut self) -> bool {
        self.done && !matches!(self.child.try_wait(), Ok(None))
    }

    /// Ask it to close the engine (once): the player left the game, or
    /// the engine stayed up after it.
    pub fn ask_to_quit(&mut self) {
        if self.asked.is_some() {
            return;
        }
        self.asked = Some(Instant::now());
        if let Some(stdin) = &mut self.stdin {
            let _ = writeln!(stdin, "quit");
            let _ = stdin.flush();
        }
    }

    /// How long ago it was asked to quit.
    pub fn asked_to_quit(&self) -> Option<Duration> {
        self.asked.map(|t| t.elapsed())
    }

    /// Stop it now (a match that came again with another host, or one
    /// that didn't quit when asked).
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.done = true;
    }
}

#[cfg(test)]
impl Running {
    /// A stand-in engine for tests that runs until it is dropped: a
    /// process waiting on its input, which says nothing.
    pub fn waiting() -> Running {
        #[cfg(windows)]
        let mut cmd = {
            let mut c = Command::new("cmd");
            c.args(["/C", "pause"]);
            c
        };
        #[cfg(not(windows))]
        let mut cmd = Command::new("cat");
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .expect("a stand-in process");
        let stdin = child.stdin.take();
        let (_, said) = mpsc::channel();
        Running {
            child,
            stdin,
            said,
            done: false,
            asked: None,
            game_menu: false,
            controls: Controls::default(),
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if !self.done {
            self.kill();
        }
    }
}

/// Windows gives the foreground only to the process the user last used:
/// while the lobby is in front it may pass that on to the engine's copy,
/// whose window then takes it as it opens (`win::gfx`), so the player
/// doesn't have to click it before the controls work.
#[cfg(windows)]
fn let_to_front(pid: u32) {
    use windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;
    // SAFETY: a plain call with a process id; failing (the lobby isn't in
    // front) only means the engine's window may need a click.
    let _ = unsafe { AllowSetForegroundWindow(pid) };
}

#[cfg(not(windows))]
fn let_to_front(_pid: u32) {}

/// The made-up results `--fake-engine` ends with: the same on every PC of
/// the match, so the server counts them. Player `i` of `n` places `i`th
/// with `n - i` points; on a team, everyone takes the team's best place.
/// The counts are plausible, as Slayer's would be: the last player killed
/// themselves once, each player's kills are their score plus their
/// suicides, and each one's kills are the next player's deaths (the first
/// player's land on the second, the last's on the first). Alone, no one
/// is killed. (In a team game a "kill" may land on a teammate; it isn't
/// called a betrayal, as only plausible numbers are needed.)
pub fn fake_results(s: &Session) -> Vec<LauncherPlayerResult> {
    let n = s.players.len();
    let team_place = |team: i32| {
        s.players
            .iter()
            .position(|p| p.team == team)
            .unwrap_or_default()
    };
    let score = |i: usize| (n - i) as i32;
    let suicides = |i: usize| u16::from(n >= 2 && i + 1 == n);
    let kills = |i: usize| {
        if n < 2 {
            0
        } else {
            score(i) as u16 + suicides(i)
        }
    };
    s.players
        .iter()
        .enumerate()
        .map(|(i, p)| LauncherPlayerResult {
            relay_id: p.xuid,
            team: p.team.clamp(0, 255) as u8,
            place: team_place(p.team).min(255) as u8,
            score: score(i),
            kills: kills(i),
            assists: if n < 2 { 0 } else { ((n - i) / 2) as u16 },
            deaths: if n < 2 {
                0
            } else {
                kills((i + n - 1) % n) + suicides(i)
            },
            betrayals: 0,
            suicides: suicides(i),
            left: false,
        })
        .collect()
}

/// The keys `H2LOBBY_FAKE_MENU` gives, in time order: `<seconds> <key>`
/// separated by `;` (see the module's docs).
pub fn fake_menu_keys(text: &str) -> Result<Vec<(f64, Key)>, String> {
    let mut keys = Vec::new();
    for part in text.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let bad = || format!("H2LOBBY_FAKE_MENU: {part:?} should look like '1.5 a'");
        let (t, k) = part.split_once(char::is_whitespace).ok_or_else(bad)?;
        let t: f64 = t.trim().parse().map_err(|_| bad())?;
        if !t.is_finite() || t < 0.0 {
            return Err(bad());
        }
        let key = match k.trim().to_ascii_lowercase().as_str() {
            "start" => Key::Start,
            "back" => Key::Back,
            "a" => Key::A,
            "b" => Key::B,
            "y" => Key::Y,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            _ => return Err(bad()),
        };
        keys.push((t, key));
    }
    keys.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(keys)
}

/// Whether the session's game has teams, as far as it says: two players
/// on one team. (A free-for-all session gives everyone a team of their
/// own; a team game of one player a team looks the same, and its players
/// column then has no team colours. The real engine reads the variant.)
fn session_has_teams(s: &Session) -> bool {
    s.players
        .iter()
        .enumerate()
        .any(|(i, p)| s.players[..i].iter().any(|q| q.team == p.team))
}

/// How the stand-in's game ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Played {
    /// It ran its length: the game ends with results.
    Finished,
    /// The lobby said `quit` (or went away), or `H2LOBBY_FAKE_QUIT`.
    Quit,
    /// The player left (or ended the game) from the GAME MENU.
    Left(Leave),
}

/// `--fake-engine`: say what an engine playing the session would, without
/// one. See the module's docs for its environment variables.
pub fn run_fake(raw: &[String]) -> i32 {
    let args = match crate::cli::parse(raw.iter().cloned()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("h2launch --fake-engine: {e}");
            return 2;
        }
    };
    let Some(path) = &args.session else {
        eprintln!("h2launch --fake-engine: needs --session");
        return 2;
    };
    let session = match std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|t| Session::parse(&t))
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("h2launch --fake-engine: {path}: {e}");
            return 2;
        }
    };
    let me = match session.resolve_me(args.me) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("h2launch --fake-engine: {e}");
            return 2;
        }
    };
    let seconds: f64 = std::env::var("H2LOBBY_FAKE_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5.0);
    let quit = std::env::var_os("H2LOBBY_FAKE_QUIT").is_some();
    let mut keys: VecDeque<(f64, Key)> =
        match fake_menu_keys(&std::env::var("H2LOBBY_FAKE_MENU").unwrap_or_default()) {
            Ok(k) => k.into(),
            Err(e) => {
                eprintln!("h2launch --fake-engine: {e}");
                return 2;
            }
        };
    // The GAME MENU as the real engine's copy has it: the game's kind,
    // whether this PC hosts, the controls it was started with (the
    // lobby passes all of them; over the defaults here, as lobby.txt's
    // would be in the real engine), when the engine takes a new profile
    // and mapping (after the reload message, unless it is off), and the
    // players.
    let reload = args.reload_message.is_some();
    let ctx = MenuContext {
        kind: args.game_kind,
        host: session.is_host(me),
        launch: args.controls.over(Controls::default()),
        profile: Pickup::of(false, reload),
        mapping: Pickup::of(false, reload),
        players: gamemenu::roster(&session, me, session_has_teams(&session)),
    };
    let mut menu = GameMenu::new(ctx.launch);
    // The controls changed and not said yet.
    let mut saves = Saves::new();
    // The lobby's `quit` (the player left), or the lobby gone.
    let left = Arc::new(AtomicBool::new(false));
    let flag = left.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines() {
            match line {
                Ok(l) if l.trim() == "quit" => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
        flag.store(true, Ordering::SeqCst);
    });
    // Sleeps `s` seconds, or less if the player leaves; false if they did.
    let wait = |s: f64| {
        let until = Instant::now() + Duration::from_secs_f64(s.max(0.0));
        while Instant::now() < until {
            if left.load(Ordering::SeqCst) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        !left.load(Ordering::SeqCst)
    };
    live::install_events();
    println!(
        "fake engine: {} on {} as machine {me} ({})",
        args.variant.as_deref().unwrap_or("?"),
        args.map.as_deref().unwrap_or("?"),
        if ctx.host { "hosting" } else { "joining" }
    );
    println!(
        "fake engine: a {} game's GAME MENU, {} players, {} menu keys",
        ctx.kind.key(),
        ctx.players.len(),
        keys.len()
    );
    let started = wait(0.5) && {
        live::tell(Engine::Running);
        wait(0.5)
    };
    if started {
        live::tell(Engine::MapLoaded);
        let loaded = Instant::now();
        let length = if quit { seconds / 2.0 } else { seconds };
        let played = loop {
            if left.load(Ordering::SeqCst) {
                menu.quitting();
                break Played::Quit;
            }
            let t = loaded.elapsed().as_secs_f64();
            let mut leave = None;
            while keys.front().is_some_and(|&(at, _)| at <= t) {
                let Some((_, key)) = keys.pop_front() else {
                    break;
                };
                let was = menu.live();
                let effects = if key == Key::Start && !menu.is_open() {
                    menu.open()
                } else {
                    menu.press(key, false, &ctx)
                };
                for e in effects {
                    leave = leave.or(fake_effect(e, was, &mut saves, t));
                }
                if leave.is_some() {
                    break;
                }
            }
            if let Some(k) = leave {
                break Played::Left(k);
            }
            if let Some(u) = saves.due(t, menu.page()) {
                live::tell(Engine::controls(u.to, u.defaults));
            }
            if t >= length {
                break if quit { Played::Quit } else { Played::Finished };
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let t = loaded.elapsed().as_secs_f64();
        if played == Played::Finished {
            // The game's end closes the menu, as the real one does.
            let was = menu.live();
            for e in menu.game_over() {
                fake_effect(e, was, &mut saves, t);
            }
        }
        // Whatever is still unsaved goes before the end, as the real
        // engine's quit and game over send it.
        if let Some(u) = saves.take() {
            live::tell(Engine::controls(u.to, u.defaults));
        }
        match played {
            Played::Finished => {
                live::tell(Engine::Ended(fake_results(&session)));
                // The engine's postgame, before it closes.
                wait(1.0);
            }
            // The host's engine gives End Game's result as it quits.
            Played::Left(Leave::End) => live::tell(Engine::Ended(fake_results(&session))),
            Played::Left(Leave::Game) | Played::Quit => {}
        }
    }
    live::close();
    0
}

/// What the stand-in does with a GAME MENU effect at `t` (`was`: the
/// controls before it), as the real engine's copy would tell the lobby:
/// the controls changed (kept in `saves` until it is time to say them),
/// or the player left (returned; what `saves` kept is said first).
fn fake_effect(e: Effect, was: Controls, saves: &mut Saves, t: f64) -> Option<Leave> {
    match e {
        Effect::Opened => println!("fake engine: the game menu opened"),
        Effect::Closed => println!("fake engine: the game menu closed"),
        Effect::Controls(c) => {
            println!("fake engine: controls: {}", c.describe());
            saves.change(was, c, false, t);
        }
        Effect::Defaults => {
            let c = Controls::default();
            println!(
                "fake engine: controls: {} (Use Default Settings)",
                c.describe()
            );
            saves.change(was, c, true, t);
        }
        Effect::Leave(k) => {
            println!(
                "fake engine: {} from the game menu",
                match k {
                    Leave::Game => "left the game",
                    Leave::End => "ended the game for everyone",
                }
            );
            if let Some(u) = saves.take() {
                live::tell(Engine::controls(u.to, u.defaults));
            }
            live::tell(Engine::Left(k));
            return Some(k);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_results_folder_keeps_the_newest_blocks() {
        let folder = std::env::temp_dir().join(format!("h2results-{}", std::process::id()));
        let dir = folder.join("results");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&dir).unwrap();
        let start = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        for i in 0..RESULTS_KEPT + 4 {
            let f = std::fs::File::create(dir.join(format!("result-1-{i:03}.bin"))).unwrap();
            f.set_modified(start + Duration::from_secs(i as u64))
                .unwrap();
        }
        std::fs::write(dir.join("notes.txt"), "kept").unwrap();
        assert_eq!(results_folder(&folder), Some(dir.clone()));
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left.len(), RESULTS_KEPT);
        assert_eq!(left[0], "notes.txt");
        assert_eq!(left[1], "result-1-005.bin");
        let _ = std::fs::remove_dir_all(&folder);
    }

    fn session(teams: &[i32]) -> Session {
        let mut text = String::from("relay = 127.0.0.1:47050\nroom = 1\nsecure = 1\nhost = 0\n");
        for i in 0..teams.len() {
            text += &format!("machine = {:#x}\n", 0x100 + i);
        }
        for (i, t) in teams.iter().enumerate() {
            text += &format!("player = {:#x} machine={i} team={t} name=P{i}\n", 0x100 + i);
        }
        Session::parse(&text).unwrap()
    }

    #[test]
    fn launch_controls_put_the_extra_flags_over_the_lobbys() {
        let base = Controls {
            look_sensitivity: 7,
            ..Controls::default()
        };
        assert_eq!(launch_controls("", base), base);
        assert_eq!(launch_controls("--pad 1 --no-game-menu", base), base);
        assert_eq!(
            launch_controls("--no-vibration --layout boxer --look-sensitivity 2", base),
            Controls {
                vibration: false,
                buttons: crate::controls::ButtonLayout::Boxer,
                look_sensitivity: 2,
                ..base
            }
        );
        // The last flag given wins, as the engine's own parse has it.
        assert!(launch_controls("--no-vibration --vibration", base).vibration);
        // Words the engine wouldn't take: the lobby's (the engine doesn't
        // start then).
        assert_eq!(launch_controls("--layout claw", base), base);
    }

    #[test]
    fn fake_results_place_players_and_teams() {
        let ffa = fake_results(&session(&[0, 1, 2]));
        let places: Vec<u8> = ffa.iter().map(|r| r.place).collect();
        assert_eq!(places, [0, 1, 2]);
        assert_eq!(ffa[0].relay_id, 0x100);
        assert_eq!(ffa[2].score, 1);
        let teams = fake_results(&session(&[1, 0, 1, 0]));
        let places: Vec<u8> = teams.iter().map(|r| r.place).collect();
        assert_eq!(places, [0, 1, 0, 1]);
    }

    #[test]
    fn fake_results_have_kills_that_add_up() {
        for n in 1..=4 {
            let teams: Vec<i32> = (0..n).collect();
            let s = session(&teams);
            let r = fake_results(&s);
            assert_eq!(r.len(), n as usize);
            // The same on every call (every PC).
            assert_eq!(r, fake_results(&s));
            let sum = |f: fn(&LauncherPlayerResult) -> u16| r.iter().map(f).sum::<u16>();
            // Every kill and every suicide is someone's death.
            assert_eq!(sum(|p| p.deaths), sum(|p| p.kills) + sum(|p| p.suicides));
            for p in &r {
                assert_eq!(p.betrayals, 0);
            }
            if n >= 2 {
                // Slayer's sum: what the kills check would pass.
                for p in &r {
                    let kills = i32::from(p.kills);
                    assert_eq!(kills, p.score + i32::from(p.suicides), "{n} players");
                }
                assert!(sum(|p| p.kills) > 0 && sum(|p| p.assists) > 0 && sum(|p| p.deaths) > 0);
                assert_eq!(r[n as usize - 1].suicides, 1);
            } else {
                let p = r[0];
                assert_eq!((p.kills, p.assists, p.deaths, p.suicides), (0, 0, 0, 0));
            }
        }
        // Three players: kills 3, 2, 2 (the last killed themselves once),
        // and each one's kills are the next one's deaths.
        let r = fake_results(&session(&[0, 1, 2]));
        let kills: Vec<u16> = r.iter().map(|p| p.kills).collect();
        let deaths: Vec<u16> = r.iter().map(|p| p.deaths).collect();
        let assists: Vec<u16> = r.iter().map(|p| p.assists).collect();
        assert_eq!(kills, [3, 2, 2]);
        assert_eq!(deaths, [2, 3, 3]);
        assert_eq!(assists, [1, 1, 0]);
    }

    #[test]
    fn the_engine_gets_the_match_on_its_command_line() {
        let m = Match {
            kind: Kind::Real {
                mcc: Some(r"D:\MCC".into()),
            },
            map: "lockout".into(),
            variant: "H2_Team_Slayer".into(),
            session: session(&[0, 1]),
            me: 1,
            gamertag: "MASTER CHIEF".into(),
            instance: Some("la".into()),
            controls: Controls {
                buttons: crate::controls::ButtonLayout::BumperJumper,
                sticks: crate::controls::StickLayout::Southpaw,
                look_sensitivity: 6,
                look_inverted: true,
                auto_center: false,
                vibration: false,
                mouse_sensitivity: 2.2,
                mouse_inverted: true,
            },
            game_kind: GameKind::Ranked,
        };
        let a = m.args(Path::new("s.txt"));
        let parsed = crate::cli::parse(a.iter().cloned()).unwrap();
        assert_eq!(parsed.session.as_deref(), Some("s.txt"));
        assert_eq!(parsed.me, Some(1));
        assert_eq!(parsed.map.as_deref(), Some("lockout"));
        assert_eq!(parsed.variant.as_deref(), Some("H2_Team_Slayer"));
        assert_eq!(parsed.name, "MASTER CHIEF");
        assert_eq!(parsed.instance.as_deref(), Some("la"));
        assert_eq!(parsed.mcc.as_deref(), Some(r"D:\MCC"));
        assert!(parsed.events && !parsed.fake_engine);
        assert_eq!(parsed.game_kind, GameKind::Ranked);
        // The controls, whatever lobby.txt says where it starts.
        assert_eq!(parsed.controls.over(Controls::default()), m.controls);
        let all_on = Controls {
            look_inverted: false,
            auto_center: true,
            vibration: true,
            mouse_inverted: false,
            ..Controls::default()
        };
        assert_eq!(parsed.controls.over(all_on), m.controls);
        let mut fake = Match {
            kind: Kind::Fake,
            ..m
        };
        let parsed = crate::cli::parse(fake.args(Path::new("s.txt"))).unwrap();
        assert!(parsed.fake_engine && parsed.mcc.is_none());
        // Every kind goes through.
        for k in GameKind::ALL {
            let m = Match {
                game_kind: k,
                ..fake
            };
            let parsed = crate::cli::parse(m.args(Path::new("s.txt"))).unwrap();
            assert_eq!(parsed.game_kind, k);
            fake = m;
        }
    }

    #[test]
    fn whether_the_engine_has_its_game_menu() {
        use std::ffi::OsStr;
        assert!(game_menu_on("", None));
        assert!(game_menu_on("--pad none --set-option 0x354=i32:60", None));
        // The README's fallback, anywhere in the words.
        assert!(!game_menu_on("--no-game-menu", None));
        assert!(!game_menu_on("  --pad 1   --no-game-menu ", None));
        // Only the flag itself (the engine refuses anything else).
        assert!(game_menu_on("--no-game-menus", None));
        // The overlay failure's stand-in turns it off at 1 only.
        assert!(!game_menu_on("", Some(OsStr::new("1"))));
        assert!(game_menu_on("", Some(OsStr::new("0"))));
        assert!(game_menu_on("", Some(OsStr::new(""))));
    }

    #[test]
    fn a_matchs_kind_for_the_game_menu() {
        let m = |playlist: u8, ranked: bool| LauncherMatch {
            id: 1,
            playlist,
            ranked,
            teams: false,
            map: "lockout".into(),
            variant: "01_slayer".into(),
            relay: h2net::live::RelaySeat {
                port: 47050,
                room: 1,
                id: 1,
                key: [0; 16],
            },
            host: 1,
            countdown: 20,
            players: Vec::new(),
        };
        assert_eq!(game_kind(&m(11, true)), GameKind::Ranked);
        assert_eq!(game_kind(&m(12, false)), GameKind::Social);
        assert_eq!(game_kind(&m(CUSTOM_GAME, false)), GameKind::Custom);
    }

    #[test]
    fn the_fake_engines_menu_keys() {
        let k = fake_menu_keys("1.0 start;1.5 a; 2.0 UP ;2.5 a;").unwrap();
        assert_eq!(
            k,
            [
                (1.0, Key::Start),
                (1.5, Key::A),
                (2.0, Key::Up),
                (2.5, Key::A)
            ]
        );
        // In time order, every key.
        let k = fake_menu_keys("2 back;0.5 b;1 y;0 down;3 left;3 right").unwrap();
        let keys: Vec<Key> = k.iter().map(|&(_, k)| k).collect();
        assert_eq!(
            keys,
            [Key::Down, Key::B, Key::Y, Key::Back, Key::Left, Key::Right]
        );
        assert!(fake_menu_keys("").unwrap().is_empty());
        for bad in ["start", "1.0", "x a", "-1 a", "1 x", "inf a", "1;a"] {
            assert!(fake_menu_keys(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_fake_engine_finds_teams_in_the_session() {
        assert!(!session_has_teams(&session(&[0, 1, 2])));
        assert!(session_has_teams(&session(&[1, 0, 1, 0])));
        assert!(!session_has_teams(&session(&[0])));
    }
}
