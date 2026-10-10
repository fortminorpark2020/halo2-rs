//! `--variants`: load every matchmaking game variant in
//! `halo2\hopper_game_variants` through halo2.dll's data access (the way a
//! launch loads one), copy each into a scratch options buffer and print its
//! settings. Read-only: no engine, no window; only names and numbers.

use super::engine;
use crate::cli::Args;
use crate::mccroot;
use crate::options::GameOptions;

/// Options offsets the variant copy fills (seen on the owner's PC).
const NAME: usize = 0x304;
const GAME_TYPE: usize = 0x344;
const FLAGS: usize = 0x348;
const SCORE_TO_WIN: usize = 0x350;
const TIME_LIMIT: usize = 0x354;
/// Number of rounds (0 = one).
const ROUNDS: usize = 0x34C;
/// Player limit (16 normally; 3 and 4 in 2-on-1 and 3-on-1).
const MAX_PLAYERS: usize = 0x378;
/// In FLAGS: teams on (Slayer 0x0FDA vs Team Slayer 0x0FDB, Phantoms vs Team
/// Phantoms, King vs Team King; every CTF, Assault and Territories variant).
const FLAG_TEAMS: i32 = 0x1;
/// The rest of the variant's settings block, printed as `offset=value`
/// for every non-zero dword so differences between variants show.
const OTHER: std::ops::Range<usize> = 0x340..0x430;

fn wide_at(b: &[u8], at: usize) -> String {
    let w: Vec<u16> = b[at..at + 64]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
        .take_while(|&c| c != 0)
        .collect();
    String::from_utf16_lossy(&w)
}

pub fn run(args: &Args) -> i32 {
    let (root, _) = engine::find_root(args.mcc.as_deref());
    let Some(root) = root else {
        println!("MCC was not found. Pass --mcc <folder>.");
        return 1;
    };
    let module = match engine::load_halo2(&root) {
        Ok(m) => m,
        Err(e) => {
            println!("halo2.dll: {e}");
            return 1;
        }
    };
    let data_access = match engine::create_data_access(&module) {
        Ok(d) => d,
        Err(e) => {
            println!("CreateDataAccess: {e}");
            return 1;
        }
    };
    let dir = mccroot::join(&root, r"halo2\hopper_game_variants");
    let mut files: Vec<_> = match std::fs::read_dir(&dir) {
        Ok(d) => d
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bin")))
            .collect(),
        Err(e) => {
            println!("{dir}: {e}");
            return 1;
        }
    };
    files.sort();
    println!(
        "h2launch {} --variants: {} files in {dir}",
        crate::BUILD,
        files.len()
    );
    let raw = std::env::var_os("H2LAUNCH_VARIANTS_RAW").is_some();
    println!(
        "file | name | type | teams | flags | score | time s | rounds | players{}",
        if raw {
            " | other non-zero dwords in 0x340..0x430"
        } else {
            ""
        }
    );
    for path in files {
        let file = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                println!("{file} | unreadable: {e}");
                continue;
            }
        };
        let mut opts = Box::new(GameOptions::new());
        if let Err(e) = engine::load_variant(data_access, &bytes, opts.as_mut()) {
            println!("{file} | not loaded: {e}");
            continue;
        }
        let b = opts.bytes();
        let known = [
            GAME_TYPE,
            FLAGS,
            SCORE_TO_WIN,
            TIME_LIMIT,
            ROUNDS,
            MAX_PLAYERS,
        ];
        let other: Vec<String> = OTHER
            .step_by(4)
            .filter(|o| !known.contains(o))
            .filter_map(|o| {
                let v = opts.get_i32(o);
                (v != 0).then(|| format!("{o:#x}={v}"))
            })
            .collect();
        let flags = opts.get_i32(FLAGS);
        println!(
            "{file} | {} | {} | {} | {flags:#06x} | {} | {} | {} | {}{}",
            wide_at(b, NAME),
            opts.get_i32(GAME_TYPE),
            if flags & FLAG_TEAMS != 0 {
                "teams"
            } else {
                "ffa"
            },
            opts.get_i32(SCORE_TO_WIN),
            opts.get_i32(TIME_LIMIT),
            opts.get_i32(ROUNDS).max(1),
            opts.get_i32(MAX_PLAYERS),
            if raw {
                format!(" | {}", other.join(" "))
            } else {
                String::new()
            }
        );
    }
    0
}
