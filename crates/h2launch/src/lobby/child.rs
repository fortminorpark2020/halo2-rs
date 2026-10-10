//! The engine for one match: a second copy of the launcher, started by the
//! lobby with the match's session (`--session ... --events`), whose
//! standard output says what the engine does. A line `quit` on its
//! standard input closes the engine as closing its window would, and so
//! does that input closing (the lobby is gone).
//! `--fake-engine` stands in for it where there is no MCC (Linux, tests).

use crate::live::{self, Engine};
use crate::session::Session;
use h2net::live::LauncherPlayerResult;
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
        ]);
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
}

impl Running {
    /// Write the session into `folder` and start `exe` (this launcher) on
    /// the match.
    pub fn start(exe: &Path, folder: &Path, m: &Match) -> Result<Running, String> {
        let file: PathBuf = folder.join("match-session.txt");
        let mut session = m.session.clone();
        session.me = Some(m.me);
        std::fs::write(&file, session.to_text()).map_err(|e| format!("{}: {e}", file.display()))?;
        // For tests: more flags for the engine (`--set-option`, `--pad`).
        let extra = std::env::var("H2LOBBY_ENGINE_ARGS").unwrap_or_default();
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
        })
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

impl Drop for Running {
    fn drop(&mut self) {
        if !self.done {
            self.kill();
        }
    }
}

/// The made-up results `--fake-engine` ends with: the same on every PC of
/// the match, so the server counts them. Player `i` of `n` places `i`th
/// with `n - i` points; on a team, everyone takes the team's best place.
pub fn fake_results(s: &Session) -> Vec<LauncherPlayerResult> {
    let n = s.players.len();
    let team_place = |team: i32| {
        s.players
            .iter()
            .position(|p| p.team == team)
            .unwrap_or_default()
    };
    s.players
        .iter()
        .enumerate()
        .map(|(i, p)| LauncherPlayerResult {
            relay_id: p.xuid,
            team: p.team.clamp(0, 255) as u8,
            place: team_place(p.team).min(255) as u8,
            score: (n - i) as i32,
            kills: (n - i) as u16,
            deaths: i as u16,
            left: false,
        })
        .collect()
}

/// `--fake-engine`: say what an engine playing the session would, without
/// one. `H2LOBBY_FAKE_SECONDS` sets how long the game lasts (5 s), and
/// `H2LOBBY_FAKE_QUIT=1` quits it halfway instead of finishing.
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
        if session.is_host(me) {
            "hosting"
        } else {
            "joining"
        }
    );
    let played = wait(0.5)
        && {
            live::tell(Engine::Running);
            wait(0.5)
        }
        && {
            live::tell(Engine::MapLoaded);
            if quit {
                wait(seconds / 2.0);
                false
            } else {
                wait(seconds)
            }
        };
    if played {
        live::tell(Engine::Ended(fake_results(&session)));
        // The engine's postgame, before it closes.
        wait(1.0);
    }
    live::close();
    0
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
        let fake = Match {
            kind: Kind::Fake,
            ..m
        };
        let parsed = crate::cli::parse(fake.args(Path::new("s.txt"))).unwrap();
        assert!(parsed.fake_engine && parsed.mcc.is_none());
    }
}
