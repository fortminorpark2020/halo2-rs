//! The lobby's flows, end to end: real headless lobbies (`h2launch
//! --lobby` with `H2LOBBY_HEADLESS`, each driven by a script in
//! `tests/flows/`) against an h2live server and its relay running in this
//! process, playing their matches on the stand-in engine. Every later
//! menu phase keeps these passing, so every flow still works after it:
//! when a screen's label or the way to it changes, its scripts change in
//! the same commit (docs/notes/launcher/lobby.md, "The flows test").
//!
//! The lobbies wait for each other only through their scripts (`wait`,
//! `see`, `pick`). A flow fails when any lobby's script does, or when it
//! takes too long; the message holds the end of each lobby's log and of
//! what it wrote to stderr (`<instance>.stderr` in the flow's folder: a
//! panic's message, or an error from before the log opened).
//! `H2FLOWS_SHOTS=<folder>` keeps each lobby's screenshots there, and
//! the flows' folders (with each lobby's `lobby.log`) after they pass.
//!
//! The lobby runs headless on Linux only here (on Windows it would start
//! MCC's engine), so this file is empty on Windows.
#![cfg(not(windows))]

use h2live::server::{Route, Server};
use h2net::Request;
use h2relay::{Admission, RelayServer, RelayThread, ServerConfig};
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Where games come in, as h2live's own program takes them.
const PATHS: &[&str] = &["/live", "/link"];
/// How long one stage of a flow (its lobbies together) may take.
const STAGE: Duration = Duration::from_secs(150);
/// Lines of each lobby's log (and of its stderr) shown when a flow fails.
const TAIL: usize = 40;

/// h2live on a port of its own on this PC, with its relay beside it, until
/// dropped: what `h2live` itself does, without its web pages and limits.
struct Live {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _relay: RelayThread,
}

impl Live {
    fn start(data: &Path) -> Live {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut server = Server::open(data, Some("test")).unwrap();
        // On the same port number over UDP, as h2live does, else any.
        let config = || ServerConfig {
            admission: Admission::Issued,
            ..ServerConfig::default()
        };
        let quiet: h2relay::Logger = Arc::new(|_: &str| {});
        let relay = RelayServer::bind((Ipv4Addr::LOCALHOST, port), config())
            .or_else(|_| RelayServer::bind((Ipv4Addr::LOCALHOST, 0), config()))
            .and_then(|r| r.with_log(quiet).spawn())
            .unwrap();
        server.set_relay(relay.handle().clone());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            let start = Instant::now();
            while !stopping.load(Ordering::SeqCst) {
                let now = start.elapsed().as_secs_f64();
                while let Ok((stream, from)) = listener.accept() {
                    if let Ok(Request::WebSocket(conn, path, _)) = h2net::accept(stream, PATHS) {
                        let route = match path.as_str() {
                            "/live" => Route::Live,
                            _ => Route::Link,
                        };
                        server.accept(conn, route, from.ip(), now);
                    }
                }
                server.poll(now);
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        Live {
            port,
            stop,
            thread: Some(thread),
            _relay: relay,
        }
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// One flow: its folder (the server's data, and the lobbies' folders as
/// `XDG_DATA_HOME`) and its server.
struct Flow {
    name: &'static str,
    dir: PathBuf,
    live: Live,
}

/// A lobby running a script, and where its log and its stderr are.
struct Lobby {
    instance: String,
    child: Child,
    log: PathBuf,
    stderr: PathBuf,
}

/// The last `TAIL` lines of the file at `path`, and how many there are
/// (none when it is missing).
fn tail(path: &Path) -> (usize, String) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let tail = &lines[lines.len().saturating_sub(TAIL)..];
    (tail.len(), tail.join("\n"))
}

impl Flow {
    fn new(name: &'static str) -> Flow {
        let dir = std::env::temp_dir().join(format!("h2flows-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let live = Live::start(&dir.join("live"));
        Flow { name, dir, live }
    }

    /// Start a lobby as `instance` (its own folder, kept between stages)
    /// running `tests/flows/<script>`.
    fn lobby(&self, instance: &str, script: &str) -> Lobby {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("flows")
            .join(script);
        assert!(script.is_file(), "no script {}", script.display());
        let instance = format!("{}-{instance}", self.name);
        let log = self.dir.join("h2launch").join(&instance).join("lobby.log");
        // Its stderr, which only Rust's panic message and the errors from
        // before the log opens go to (its stdout repeats the log). Added
        // to by each stage that runs this instance.
        let stderr = self.dir.join(format!("{instance}.stderr"));
        let stderr_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stderr)
            .expect("open the lobby's stderr file");
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_h2launch"));
        cmd.args(["--lobby", "--live"])
            .arg(format!("127.0.0.1:{}", self.live.port))
            .args(["--instance", &instance])
            // The lobby's folder: lobby.txt, the sign-in key, the log.
            .env("XDG_DATA_HOME", &self.dir)
            .env_remove("LOCALAPPDATA")
            .env("H2LOBBY_HEADLESS", "1")
            .env("H2LOBBY_SCRIPT", &script)
            // No game files: the flat look, level numbers, the system
            // font, and no Halo 2 Vista folder.
            .env("H2LOBBY_MENU", "off")
            .env("H2LOBBY_RANKS", "off")
            .env("H2LOBBY_H2FONTS", "off")
            .env("H2LOBBY_VISTA", "off")
            // For the phases that add them: no sound, intro or scene.
            .env("H2LOBBY_SOUND", "off")
            .env("H2LOBBY_INTRO", "off")
            .env("H2LOBBY_SCENE", "off")
            // Matches on the stand-in engine, a second long.
            .env("H2LOBBY_FAKE", "1")
            .env("H2LOBBY_FAKE_SECONDS", "1")
            .env_remove("H2LOBBY_FAKE_QUIT")
            .env_remove("H2LOBBY_ENGINE_ARGS")
            .env_remove("H2LOBBY_SHOTS")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr_file));
        if let Some(shots) = std::env::var_os("H2FLOWS_SHOTS") {
            cmd.env("H2LOBBY_SHOTS", Path::new(&shots).join(&instance));
        }
        let child = cmd.spawn().expect("start h2launch --lobby");
        Lobby {
            instance,
            child,
            log,
            stderr,
        }
    }

    /// Run a stage: the lobbies (instance, script) together, until every
    /// one has finished its script. Panics, with the end of each one's
    /// log and stderr, if any fails or the stage takes longer than
    /// `STAGE`.
    fn stage(&self, lobbies: &[(&str, &str)]) {
        let mut running: Vec<Lobby> = lobbies
            .iter()
            .map(|(instance, script)| self.lobby(instance, script))
            .collect();
        let until = Instant::now() + STAGE;
        let mut failed = None;
        loop {
            let mut done = 0;
            for l in &mut running {
                match l.child.try_wait() {
                    Ok(Some(status)) if status.success() => done += 1,
                    Ok(Some(status)) => {
                        failed = Some(format!("{} ended with {status}", l.instance));
                    }
                    Ok(None) => {}
                    Err(e) => failed = Some(format!("{}: {e}", l.instance)),
                }
            }
            if failed.is_some() || done == running.len() {
                break;
            }
            if Instant::now() >= until {
                failed = Some(format!("not done within {} s", STAGE.as_secs()));
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let Some(why) = failed else {
            return;
        };
        let mut message = format!("flow {}: {why}\n", self.name);
        for l in &mut running {
            let _ = l.child.kill();
            let _ = l.child.wait();
            let (n, lines) = tail(&l.log);
            message.push_str(&format!(
                "--- {} ({}), last {n} lines:\n{lines}\n",
                l.instance,
                l.log.display(),
            ));
            let (n, lines) = tail(&l.stderr);
            if n > 0 {
                message.push_str(&format!(
                    "--- {} stderr ({}), last {n} lines:\n{lines}\n",
                    l.instance,
                    l.stderr.display(),
                ));
            }
        }
        panic!("{message}");
    }
}

impl Drop for Flow {
    fn drop(&mut self) {
        // Kept for a look when the flow failed, or when asked.
        if !std::thread::panicking() && std::env::var_os("H2FLOWS_SHOTS").is_none() {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// (a) Matchmaking: ALPHA and BRAVO sign in, ALPHA invites BRAVO from the
/// players list and BRAVO accepts, then leaves again (no default launcher
/// playlist starts with two players in one party: Head to Head takes
/// parties of one, and Team Snipers wants a third to even its teams).
/// Both search Head to Head, meet in the pregame lobby, play, see the
/// carnage report, and are back on the playlists. A party searching
/// together is `party_matchmaking`.
#[test]
fn matchmaking() {
    let flow = Flow::new("mm");
    flow.stage(&[("alpha", "mm-alpha.txt"), ("bravo", "mm-bravo.txt")]);
}

/// (a2) A party searching together: ALPHA invites BRAVO and, once BRAVO
/// has accepted, searches Team Snipers for the party; BRAVO's lobby
/// follows into the search. CHARLIE searches it alone, which makes even
/// teams (two against one; Team Snipers is unranked, so one apart is
/// even enough). After the countdown all three meet in the pregame
/// lobby, play, see the carnage report, and are back on the playlists.
#[test]
fn party_matchmaking() {
    let flow = Flow::new("pmm");
    flow.stage(&[
        ("alpha", "pmm-alpha.txt"),
        ("bravo", "pmm-bravo.txt"),
        ("charlie", "pmm-charlie.txt"),
    ]);
}

/// (b) A custom game: ALPHA invites BRAVO, goes down to the Custom Game
/// row, picks Team Slayer on Lockout and starts it; both play it and see
/// the carnage report, ALPHA (the leader) is back on the custom game
/// screen and BRAVO on the playlists.
#[test]
fn custom_game() {
    let flow = Flow::new("custom");
    flow.stage(&[("alpha", "custom-alpha.txt"), ("bravo", "custom-bravo.txt")]);
}

/// (c) The party screen with three lobbies: ALPHA invites BRAVO and
/// CHARLIE, makes the party invite only, removes BRAVO, makes CHARLIE
/// leader and leaves; CHARLIE is left leading an invite-only party of one.
#[test]
fn party_admin() {
    let flow = Flow::new("party");
    flow.stage(&[
        ("alpha", "party-alpha.txt"),
        ("bravo", "party-bravo.txt"),
        ("charlie", "party-charlie.txt"),
    ]);
}

/// (d) Friends: ALPHA asks BRAVO from the players list, BRAVO accepts on
/// the friends list, ALPHA opens BRAVO's options and then BRAVO's service
/// record.
#[test]
fn friends() {
    let flow = Flow::new("friends");
    flow.stage(&[
        ("alpha", "friends-alpha.txt"),
        ("bravo", "friends-bravo.txt"),
    ]);
}

/// (e) A setting is kept: one run sets the button layout and quits, the
/// next sees it on the settings screen.
#[test]
fn a_setting_is_kept() {
    let flow = Flow::new("setting");
    flow.stage(&[("alpha", "setting-set.txt")]);
    flow.stage(&[("alpha", "setting-see.txt")]);
}

/// (f) A gamertag someone else has is turned away and another is taken;
/// then B back to the start screen and its quit popup.
#[test]
fn a_taken_gamertag_then_quit() {
    let flow = Flow::new("taken");
    flow.stage(&[("alpha", "taken-alpha.txt")]);
    flow.stage(&[("bravo", "taken-bravo.txt")]);
}
