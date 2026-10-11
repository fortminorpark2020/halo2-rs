//! Draws the in-game GAME MENU's panels (`h2ui::gamemenu`) as the launcher
//! copies them into the engine's back buffer, each over a plain grey
//! stand-in for the game's picture, and writes them as PNGs at 960x540
//! (the engine's back buffer on the owner's PC), 1280x720 and 1920x1080,
//! printing how long each panel takes to draw.
//!
//! ```text
//! cargo run --release -p h2ui --example gamemenu_png -- <out dir> [<fonts dir>]
//! ```
//!
//! Text is in Halo 2's fonts from the fonts folder given (MCC's
//! `halo2\h2_fonts`), else in the built-in font. Pictures drawn with the
//! real fonts hold the game's glyphs: they stay on your PC.

use h2ui::gamemenu::{draw_panel, GameMenuFrame, GameMenuView, MenuPlayer, MenuRow, Panel};
use h2ui::text::Fonts;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The stand-in game picture's grey.
const GAME: [u8; 3] = [96, 104, 96];

fn row(label: &str, value: Option<&str>, next_game: bool) -> MenuRow {
    MenuRow {
        label: label.into(),
        value: value.map(str::to_string),
        next_game,
    }
}

/// Every kind of page, with made-up players and values.
fn views() -> Vec<(&'static str, GameMenuView)> {
    let names = [
        "John",
        "Kilo 23",
        "Noble Six",
        "Spartan 117",
        "Arbiter",
        "Johnson",
        "Miranda",
        "Cortana",
        "Thel",
        "Rtas",
        "Tartarus",
        "Sergeant",
        "Buck",
        "Dare",
        "Romeo",
        "Mickey",
    ];
    let players = |n: usize, teams: bool| -> Vec<MenuPlayer> {
        names
            .iter()
            .take(n)
            .enumerate()
            .map(|(k, name)| MenuPlayer {
                name: name.to_string(),
                team: if teams { (k % 2) as i32 } else { -1 },
                me: k == 0,
                host: k == 2,
            })
            .collect()
    };
    let game = |n: usize, teams: bool, end: bool| {
        let mut rows = vec![row("Leave Game", None, false), row("Settings", None, false)];
        if end {
            rows.push(row("End Game", None, false));
        }
        GameMenuView::List {
            header: "GAME MENU".into(),
            rows,
            focus: 0,
            side: Vec::new(),
            note: None,
            players: players(n, teams),
        }
    };
    let controller = GameMenuView::List {
        header: "CONTROLLER SETTINGS".into(),
        rows: vec![
            row("Thumbstick Layout", Some("Legacy Southpaw"), false),
            row("Button Layout", Some("Bumper Jumper"), true),
            row("Look Sensitivity", Some("7 (Very high)"), true),
            row("Look Inversion", Some("Normal"), false),
            row("Auto Look Centering", Some("Off"), false),
            row("Controller Vibration", Some("On"), false),
            row("Mouse Sensitivity", Some("1.6"), false),
            row("Mouse Inversion", Some("Off"), false),
            row("Use Default Settings", None, false),
        ],
        focus: 1,
        side: Vec::new(),
        note: Some("Settings marked NEXT GAME take effect in your next game.".into()),
        players: Vec::new(),
    };
    let buttons = GameMenuView::List {
        header: "BUTTON LAYOUT".into(),
        rows: [
            "Default",
            "Southpaw",
            "Boxer",
            "Green Thumb",
            "Bumper Jumper",
            "Recon",
        ]
        .iter()
        .map(|l| row(l, None, false))
        .collect(),
        focus: 4,
        side: [
            "RT: Fire",
            "LT: Throw grenade / Left weapon",
            "RB: Melee attack",
            "LB: Jump",
            "A: Swap grenades",
            "B: Reload / Action",
            "X: Flashlight",
            "Y: Switch weapons / Dual wield",
            "Left stick click: Crouch",
            "Right stick click: Zoom view",
            "D-pad up: Flashlight",
            "D-pad left: Previous grenade",
            "Back: Multiplayer score",
            "Start: Pause game",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        note: Some("Takes effect in your next game.".into()),
        players: Vec::new(),
    };
    let sensitivity = GameMenuView::List {
        header: "LOOK SENSITIVITY".into(),
        rows: [
            "1 (Low)",
            "2",
            "3 (Default)",
            "4",
            "5 (High)",
            "6",
            "7 (Very high)",
            "8",
            "9",
            "10 (Insane)",
        ]
        .iter()
        .map(|l| row(l, None, false))
        .collect(),
        focus: 2,
        side: Vec::new(),
        note: Some("Takes effect in your next game.".into()),
        players: Vec::new(),
    };
    let leave = GameMenuView::Dialog {
        title: "LEAVE GAME".into(),
        text: "You're hosting. Leaving ends the game for everyone and counts as a loss for you."
            .into(),
        choices: ["Leave Game".into(), "Cancel".into()],
        focus: 1,
    };
    let wait = GameMenuView::Wait {
        title: "PLEASE WAIT".into(),
        text: "Leaving the game.".into(),
    };
    vec![
        ("game-ffa-1", game(1, false, true)),
        ("game-teams-8", game(8, true, false)),
        ("game-teams-16", game(16, true, false)),
        ("controller", controller),
        ("buttons", buttons),
        ("sensitivity", sensitivity),
        ("leave", leave),
        ("wait", wait),
    ]
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(out) = args.next().map(PathBuf::from) else {
        eprintln!("usage: gamemenu_png <out dir> [<fonts dir>]");
        std::process::exit(2);
    };
    let fonts = match args.next() {
        Some(dir) => {
            let f = Fonts::read(Path::new(&dir));
            println!(
                "Halo 2 fonts from {}: {:?}",
                Path::new(&dir).display(),
                f.loaded()
            );
            f
        }
        None => Fonts::fallback(),
    };
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("{}: {e}", out.display());
        std::process::exit(1);
    }
    for (w, h) in [(960u32, 540u32), (1280, 720), (1920, 1080)] {
        for (name, view) in views() {
            for keyboard in [false, true] {
                let frame = GameMenuFrame {
                    view: view.clone(),
                    keyboard,
                };
                let t = Instant::now();
                let panel = draw_panel(&frame, &fonts, w, h);
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                let keys = if keyboard { "-keys" } else { "" };
                let file = out.join(format!("gamemenu-{name}{keys}-{w}x{h}.png"));
                if let Err(e) = save(&file, &panel, w, h) {
                    eprintln!("{}: {e}", file.display());
                    std::process::exit(1);
                }
                println!(
                    "{name:14} {w}x{h}{keys:5}: panel {}x{} at ({}, {}), {ms:.2} ms -> {}",
                    panel.w,
                    panel.h,
                    panel.x,
                    panel.y,
                    file.display()
                );
            }
        }
    }
}

/// The panel copied into a `w` by `h` grey frame, as a PNG.
fn save(path: &Path, panel: &Panel, w: u32, h: u32) -> Result<(), String> {
    let mut rgb: Vec<u8> = GAME.repeat((w * h) as usize);
    for y in 0..panel.h {
        let from = (y * panel.w * 4) as usize;
        let to = (((panel.y + y) * w + panel.x) * 3) as usize;
        for x in 0..panel.w as usize {
            rgb[to + x * 3..to + x * 3 + 3].copy_from_slice(&panel.rgba[from + x * 4..][..3]);
        }
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&rgb).map_err(|e| e.to_string())
}
