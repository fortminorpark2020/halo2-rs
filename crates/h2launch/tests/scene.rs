//! The lobby's flythrough wiring end to end, with no GPU and no game
//! files: a headless lobby (`h2launch --lobby` with `H2LOBBY_HEADLESS`)
//! given a made-up Halo 2 Vista folder whose mainmenu.map is a few junk
//! bytes. With `H2LOBBY_SCENE=on` the lobby starts the scene on that map
//! (`start_scene`, into `Config::scene`), the scene fails to read it
//! before any GPU is asked for, and the menus say so once; a bad
//! `H2LOBBY_VEIL` is logged as it opens. Without `H2LOBBY_SCENE`, a
//! headless lobby has no scene and says why.
//!
//! The lobby runs headless on Linux only here (on Windows it would start
//! MCC's engine), so this file is empty on Windows.
#![cfg(not(windows))]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a run may take (its script takes under 3 s).
const TIMEOUT: Duration = Duration::from_secs(60);

/// The start screen, then quit a moment later: time enough for the scene
/// to fail and the menus to see it.
const SCRIPT: &str = "0 wait start 10\n2.5 quit\n";

/// A folder of its own for the run `name`, holding a Halo 2 Vista folder
/// (`vista`) whose `maps/mainmenu.map` isn't a map, and the script.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("h2scene-wiring-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("vista").join("maps")).unwrap();
    // More than the 8 bytes the scene reads first, so it reads them all
    // and finds no map header.
    std::fs::write(
        dir.join("vista").join("maps").join("mainmenu.map"),
        b"no Halo 2 map here, only junk\n",
    )
    .unwrap();
    std::fs::write(dir.join("script.txt"), SCRIPT).unwrap();
    dir
}

/// Run the headless lobby in `dir` (`folder`) with these scene variables
/// (`H2LOBBY_SCENE`, `H2LOBBY_VEIL`; None leaves them unset): the lines
/// of its log, without their times. Panics if it fails or takes too long.
fn lobby(dir: &Path, scene: Option<&str>, veil: Option<&str>) -> Vec<String> {
    let stderr = std::fs::File::create(dir.join("stderr.txt")).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_h2launch"));
    cmd.arg("--lobby")
        // The lobby's folder: lobby.txt and the log.
        .env("XDG_DATA_HOME", dir)
        .env_remove("LOCALAPPDATA")
        .env("H2LOBBY_HEADLESS", "1")
        .env("H2LOBBY_SCRIPT", dir.join("script.txt"))
        .env("H2LOBBY_VISTA", dir.join("vista"))
        // Nothing else reads the junk map: the flat look, level numbers,
        // the system font, no sound and no intro.
        .env("H2LOBBY_MENU", "off")
        .env("H2LOBBY_RANKS", "off")
        .env("H2LOBBY_H2FONTS", "off")
        .env("H2LOBBY_SOUND", "off")
        .env("H2LOBBY_INTRO", "off")
        .env("H2LOBBY_FAKE", "1")
        .env_remove("H2LOBBY_SCENE")
        .env_remove("H2LOBBY_VEIL")
        .env_remove("H2LOBBY_SCENE_FPS")
        .env_remove("WGPU_BACKEND")
        .env_remove("H2LOBBY_SHOTS")
        .env_remove("H2LOBBY_ENGINE_ARGS")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr));
    if let Some(s) = scene {
        cmd.env("H2LOBBY_SCENE", s);
    }
    if let Some(v) = veil {
        cmd.env("H2LOBBY_VEIL", v);
    }
    let mut child = cmd.spawn().expect("start h2launch --lobby");
    let until = Instant::now() + TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let log = std::fs::read_to_string(dir.join("h2launch").join("lobby.log")).unwrap_or_default();
    let err = std::fs::read_to_string(dir.join("stderr.txt")).unwrap_or_default();
    assert!(
        status.is_some_and(|s| s.success()),
        "the lobby ended with {status:?}\nlog:\n{log}\nstderr:\n{err}"
    );
    // Each line starts with its time (`{:9.3} `).
    log.lines()
        .map(|l| l.trim_start().split_once(' ').map_or(l, |(_, rest)| rest))
        .map(str::to_string)
        .collect()
}

fn count(lines: &[String], want: &str) -> usize {
    lines.iter().filter(|l| *l == want).count()
}

#[test]
fn a_lobby_asked_for_the_scene_starts_it_on_vistas_mainmenu_map() {
    let dir = folder("on");
    let lines = lobby(&dir, Some("on"), Some("junk"));
    let veil = "scene: H2LOBBY_VEIL=junk isn't r,g,b,k[,s[,gain]]; the usual veil";
    assert_eq!(count(&lines, veil), 1, "{lines:#?}");
    // The scene was started on mainmenu.map itself (not its folder), and
    // the menus said once that it can't be drawn.
    let map = dir.join("vista").join("maps").join("mainmenu.map");
    let failed = format!(
        "menus: the scene can't be drawn ({}: not a Halo 2 map); the still background from now on",
        map.display()
    );
    assert_eq!(count(&lines, &failed), 1, "{lines:#?}");
    let drawn = lines
        .iter()
        .filter(|l| l.starts_with("menus: the scene can't be drawn ("))
        .count();
    assert_eq!(drawn, 1, "{lines:#?}");
    assert!(
        !lines.iter().any(|l| l.starts_with("scene: none (")),
        "{lines:#?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_headless_lobby_has_no_scene_unless_asked() {
    let dir = folder("headless");
    let lines = lobby(&dir, None, None);
    let none = "scene: none (headless); the menus have the still background";
    assert_eq!(count(&lines, none), 1, "{lines:#?}");
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("can't be drawn") || l.contains("H2LOBBY_VEIL")),
        "{lines:#?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
