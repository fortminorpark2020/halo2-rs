//! The in-game GAME MENU end to end without MCC: the stand-in engine
//! (`--fake-engine`) runs the real `GameMenu` on the keys in
//! `H2LOBBY_FAKE_MENU`, for the game `--game-kind` and the session say,
//! and tells the lobby what came of it on its standard output, as the
//! real engine's copy does.
//!
//! The GAME MENU's rows are Leave Game (0), Settings (1) and, for a custom
//! game's host or an offline run, End Game (2). Its dialogs start on
//! Cancel, and Up moves to the answer.

use h2launch::controls::{ButtonLayout, Controls};
use h2launch::gamemenu::Leave;
use h2launch::live::{self, Engine};
use h2launch::session::{Player, Session};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// How long one run may take, start to finish.
const TIMEOUT: Duration = Duration::from_secs(20);

/// A two-player session (machine 0 hosts), written as the lobby writes
/// it for its engine, in a folder of its own.
fn session_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("h2gamemenu-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let player = |i: usize, name: &str| Player {
        xuid: 0x100 + i as u64,
        machine: i,
        team: i as i32,
        name: Some(name.into()),
    };
    let session = Session {
        relay: "127.0.0.1:47050".into(),
        room: 1,
        key: None,
        secure: 1,
        host: 0,
        machines: vec![0x100, 0x101],
        players: vec![player(0, "HOST"), player(1, "GUEST")],
        threshold: None,
        me: None,
    };
    let file = dir.join("match-session.txt");
    std::fs::write(&file, session.to_text()).unwrap();
    file
}

/// `keys` as `H2LOBBY_FAKE_MENU` takes them: the first 0.3 s after the
/// map loads, then one each 0.15 s (all well inside the 3 s game).
fn menu(keys: &[&str]) -> String {
    keys.iter()
        .enumerate()
        .map(|(i, k)| format!("{:.2} {k}", 0.3 + 0.15 * i as f64))
        .collect::<Vec<_>>()
        .join(";")
}

/// A stand-in engine for machine `me` of a `kind` game, with the menu
/// keys `menu`, and the events it says as they come.
struct Run {
    child: Child,
    stdin: Option<ChildStdin>,
    events: Receiver<Engine>,
    session: PathBuf,
}

impl Run {
    fn start(name: &str, kind: &str, me: usize, menu: &str) -> Run {
        let session = session_file(name);
        let mut child = Command::new(env!("CARGO_BIN_EXE_h2launch"))
            .args(["--fake-engine", "--session"])
            .arg(&session)
            .args(["--me", &me.to_string()])
            .args(["--map", "lockout", "--variant", "slayer", "--name", "T"])
            .args(["--events", "--game-kind", kind])
            .env("H2LOBBY_FAKE_SECONDS", "3")
            .env("H2LOBBY_FAKE_MENU", menu)
            .env_remove("H2LOBBY_FAKE_QUIT")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the launcher starts");
        let stdin = child.stdin.take();
        let out = child.stdout.take().unwrap();
        let (tx, events) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if let Some(e) = live::parse_event_line(&line) {
                    if tx.send(e).is_err() {
                        break;
                    }
                }
            }
        });
        Run {
            child,
            stdin,
            events,
            session,
        }
    }

    /// The next event, within what is left of the run's time.
    fn next(&self, until: Instant) -> Option<Engine> {
        let left = until.saturating_duration_since(Instant::now());
        self.events.recv_timeout(left).ok()
    }

    /// Every event until the output closes (the engine exited), within
    /// the run's time.
    fn rest(&mut self, until: Instant) -> Vec<Engine> {
        let mut all = Vec::new();
        while let Some(e) = self.next(until) {
            all.push(e);
        }
        assert!(
            Instant::now() < until,
            "the stand-in didn't finish in {} s: {all:?}",
            TIMEOUT.as_secs()
        );
        all
    }

    fn quit(&mut self) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "quit").unwrap();
        stdin.flush().unwrap();
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(dir) = self.session.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// Runs a stand-in to its end and returns every event it said.
fn play(name: &str, kind: &str, me: usize, keys: &[&str]) -> Vec<Engine> {
    let until = Instant::now() + TIMEOUT;
    let mut run = Run::start(name, kind, me, &menu(keys));
    let events = run.rest(until);
    let status = run.child.wait().unwrap();
    assert!(status.success(), "{status:?}");
    events
}

fn is_ended(e: &Engine) -> bool {
    matches!(e, Engine::Ended(_))
}

#[test]
fn a_social_guest_leaves_the_game() {
    let events = play("social-guest", "social", 1, &["start", "a", "up", "a"]);
    assert_eq!(
        events,
        [
            Engine::Running,
            Engine::MapLoaded,
            Engine::Left(Leave::Game),
            Engine::Closing
        ]
    );
}

#[test]
fn a_custom_games_host_ends_it() {
    // End Game's result comes as the host's engine quits, after `left
    // end` (PC check 14): the lobby sends it as a game cut short.
    let events = play(
        "custom-host",
        "custom",
        0,
        &["start", "down", "down", "a", "up", "a"],
    );
    assert_eq!(events.len(), 5, "{events:?}");
    assert_eq!(
        events[..3],
        [Engine::Running, Engine::MapLoaded, Engine::Left(Leave::End)]
    );
    let Engine::Ended(players) = &events[3] else {
        panic!("{events:?}");
    };
    assert_eq!(players.len(), 2, "every player's result: {players:?}");
    assert_eq!(events[4], Engine::Closing);
}

#[test]
fn a_new_button_layout_is_said_and_the_game_ends_normally() {
    // Settings, Button Layout, down to Bumper Jumper (the fifth), A to
    // save it, then B and B out of the menu.
    let keys = [
        "start", "down", "a", "down", "a", "down", "down", "down", "down", "a", "b", "b",
    ];
    let events = play("controls", "social", 1, &keys);
    let want = Controls {
        buttons: ButtonLayout::BumperJumper,
        ..Controls::default()
    };
    assert_eq!(events.len(), 5, "{events:?}");
    assert_eq!(events[..2], [Engine::Running, Engine::MapLoaded]);
    assert_eq!(events[2], Engine::controls(want, false));
    let Engine::Controls { pairs, .. } = &events[2] else {
        unreachable!();
    };
    assert!(pairs
        .iter()
        .any(|(k, v)| k == "button_layout" && v == "bumper_jumper"));
    assert!(is_ended(&events[3]), "{events:?}");
    assert_eq!(events[4], Engine::Closing);
}

/// The controls go to the lobby once a visit to CONTROLLER SETTINGS, not
/// once a step (PC check 6 saved lobby.txt 24 times for 24 steps): here
/// five steps of Mouse Sensitivity, said when B leaves the page.
#[test]
fn many_steps_are_said_once() {
    // Settings, down six rows to Mouse Sensitivity, right five times,
    // then B and B out of the menu.
    let mut keys = vec!["start", "down", "a"];
    keys.extend(["down"; 6]);
    keys.extend(["right"; 5]);
    keys.extend(["b", "b"]);
    let events = play("steps", "social", 1, &keys);
    let want = Controls {
        mouse_sensitivity: 2.1,
        ..Controls::default()
    };
    assert_eq!(events.len(), 5, "{events:?}");
    assert_eq!(events[..2], [Engine::Running, Engine::MapLoaded]);
    assert_eq!(events[2], Engine::controls(want, false));
    assert!(is_ended(&events[3]), "{events:?}");
    assert_eq!(events[4], Engine::Closing);
}

/// A change left on CONTROLLER SETTINGS is said a second after it, with
/// the menu still open, before the game ends.
#[test]
fn a_change_left_open_is_said_a_second_later() {
    // Settings, up twice (the rows wrap) to Mouse Inversion, and A turns
    // it on at 1.05 s; the game ends at 3 s.
    let keys = ["start", "down", "a", "up", "up", "a"];
    let events = play("open", "social", 1, &keys);
    let want = Controls {
        mouse_inverted: true,
        ..Controls::default()
    };
    assert_eq!(events.len(), 5, "{events:?}");
    assert_eq!(events[2], Engine::controls(want, false));
    assert!(is_ended(&events[3]), "{events:?}");
}

/// A change goes before the player leaves the game.
#[test]
fn a_change_goes_before_leaving() {
    // Mouse Inversion on (as above), B back to the GAME MENU's Settings,
    // up to Leave Game, A, up to its answer, A: all within a second of
    // the change, which goes as B leaves CONTROLLER SETTINGS.
    let keys = [
        "start", "down", "a", "up", "up", "a", "b", "up", "a", "up", "a",
    ];
    let events = play("leave", "social", 1, &keys);
    let want = Controls {
        mouse_inverted: true,
        ..Controls::default()
    };
    assert_eq!(
        events,
        [
            Engine::Running,
            Engine::MapLoaded,
            Engine::controls(want, false),
            Engine::Left(Leave::Game),
            Engine::Closing
        ]
    );
}

#[test]
fn use_default_settings_is_said_as_such() {
    // Settings, up to Use Default Settings (the list wraps), A, up to Use
    // Defaults, A; then B and B out of the menu.
    let keys = ["start", "down", "a", "up", "a", "up", "a", "b", "b"];
    let events = play("defaults", "social", 1, &keys);
    assert_eq!(events.len(), 5, "{events:?}");
    assert_eq!(events[..2], [Engine::Running, Engine::MapLoaded]);
    assert_eq!(events[2], Engine::controls(Controls::default(), true));
    assert!(is_ended(&events[3]), "{events:?}");
    assert_eq!(events[4], Engine::Closing);
}

#[test]
fn a_ranked_guest_who_cancels_plays_on() {
    // Two rows: down twice wraps back to Leave Game, and B cancels its
    // question (and closes the menu).
    let events = play(
        "ranked-guest",
        "ranked",
        1,
        &["start", "down", "down", "a", "b"],
    );
    assert!(
        !events.iter().any(|e| matches!(e, Engine::Left(_))),
        "{events:?}"
    );
    assert_eq!(events.len(), 4, "{events:?}");
    assert_eq!(events[..2], [Engine::Running, Engine::MapLoaded]);
    assert!(is_ended(&events[2]), "{events:?}");
    assert_eq!(events[3], Engine::Closing);
}

/// The lobby's quit while a change waits on CONTROLLER SETTINGS: the
/// change goes before the engine closes.
#[test]
fn the_lobbys_quit_says_a_waiting_change_first() {
    let until = Instant::now() + TIMEOUT;
    let keys = menu(&["start", "down", "a", "up", "up", "a"]);
    let mut run = Run::start("quit-change", "social", 1, &keys);
    assert_eq!(run.next(until), Some(Engine::Running));
    assert_eq!(run.next(until), Some(Engine::MapLoaded));
    // Mouse Inversion goes on at 1.05 s; it would be said at 2.05 s.
    std::thread::sleep(Duration::from_millis(1400));
    run.quit();
    let events = run.rest(until);
    let want = Controls {
        mouse_inverted: true,
        ..Controls::default()
    };
    assert_eq!(events, [Engine::controls(want, false), Engine::Closing]);
    assert!(run.child.wait().unwrap().success());
}

#[test]
fn the_lobbys_quit_while_the_menu_is_open_closes_without_a_result() {
    let until = Instant::now() + TIMEOUT;
    let mut run = Run::start("quit", "social", 1, &menu(&["start", "down"]));
    assert_eq!(run.next(until), Some(Engine::Running));
    assert_eq!(run.next(until), Some(Engine::MapLoaded));
    // The menu is open (from 0.3 s) and on Settings (0.45 s).
    std::thread::sleep(Duration::from_millis(900));
    run.quit();
    let events = run.rest(until);
    assert_eq!(events, [Engine::Closing]);
    assert!(run.child.wait().unwrap().success());
}
