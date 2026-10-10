//! Matchmaking playlists: who can search each one, how many play, and the
//! games and maps it rotates through. Halo 2's launch playlists (and Team
//! Snipers and Team Hardcore, which came later) are built in twice: as far
//! as this game can play them, and for the launcher, on MCC's engine. A
//! `playlists.txt` replaces them all without a rebuild.
//!
//! A playlists file has one setting per line. Blank lines and lines
//! starting with `#` are skipped. For example:
//!
//! ```text
//! playlist 2 double_team Double Team
//! ranked yes
//! humans 4 4
//! party 2
//! variant team_slayer default 25 600
//! variant team_oddball default 120 600
//! maps lockout midship warlock
//! ```
//!
//! - `playlist <id> <key> <name>` starts a playlist, and the lines up to the
//!   next `playlist` line describe it. The id is a number from 0 to 254.
//!   The key is lowercase letters, digits and `_`: the playlist's name in
//!   Halo 2's own text (`<key>_title` and `<key>_description` in
//!   `mainmenu.map`), in saved levels and in settings. The name is the rest
//!   of the line, shown when a PC has no text of its own for the key.
//! - `client viewer|launcher`: who plays it: the game (h2viewer, the
//!   default) or the launcher (h2launch, on MCC's engine). Each sees only
//!   its own playlists, and their players never meet.
//! - `ranked yes|no`: games change levels. The default is no.
//! - `humans <min> <max>`: the fewest and most people in a game, from 2 to
//!   16. Required.
//! - `party <n>`: the biggest party that can search, guests included. The
//!   default is the most humans.
//! - `guests yes|no`: splitscreen guests can play. The default is no, and
//!   ranked playlists can't have them.
//! - `bots none|even|fill <n>`: no bots (the default); one bot to even out
//!   teams one player apart (team games); or bots filling the game up to `n`
//!   players (unranked playlists).
//! - `variant <game type> <preset> <score> <time limit>`: a game in the
//!   rotation, with the points to win (seconds in King, Oddball and
//!   Territories) and the time limit in seconds (0 for none). Game types are
//!   `slayer`, `team_slayer`, `ctf`, `king`, `team_king`, `oddball`,
//!   `team_oddball`, `juggernaut`, `territories` and `assault`; presets are
//!   `default`, `swat`, `rockets`, `snipers`, `swords`, `shotguns`,
//!   `hardcore` (battle rifle starts, no motion sensor) and `team_snipers`
//!   (snipers with no motion sensor). At least one, all team games or all
//!   free-for-all. Not in launcher playlists.
//! - `mcc_variant <game type> <variant>`: in launcher playlists instead, a
//!   game in the rotation: the game type (as above, saying whether it's a
//!   team game) and the MCC game variant's name, which the launcher loads.
//! - `maps <name>...`: map file names without `.map` (MCC's, in launcher
//!   playlists). At least one and at most 64; the line can repeat.
//!
//! Launcher playlists have no bots and no splitscreen guests, and their
//! map and variant names are file names: letters, digits and `_ - .`,
//! not starting with a dot.

use h2net::live::{ClientKind, MAX_NAME, MAX_PLAYLIST_MAPS};
use h2sim::GameType;
use std::path::Path;

/// The playlists used without a `playlists.txt`.
const BUILT_IN: &str = include_str!("playlists.txt");

/// Game types by their names in playlist files.
pub const GAME_TYPES: [(GameType, &str); 10] = [
    (GameType::Slayer, "slayer"),
    (GameType::TeamSlayer, "team_slayer"),
    (GameType::Ctf, "ctf"),
    (GameType::KingOfTheHill, "king"),
    (GameType::TeamKing, "team_king"),
    (GameType::Oddball, "oddball"),
    (GameType::TeamOddball, "team_oddball"),
    (GameType::Juggernaut, "juggernaut"),
    (GameType::Territories, "territories"),
    (GameType::Assault, "assault"),
];

/// The game's built-in variants, as the game's options name them
/// (h2viewer's `options::presets`).
pub const PRESETS: [&str; 8] = [
    "DEFAULT",
    "SWAT",
    "ROCKETS",
    "SNIPERS",
    "SWORDS",
    "SHOTGUNS",
    "HARDCORE",
    "TEAM SNIPERS",
];

/// How a playlists file names a preset: in lowercase, with `_` for
/// spaces.
fn preset_word(name: &str) -> String {
    name.to_lowercase().replace(' ', "_")
}

/// Most people (and bots) in a game.
const MAX_PLAYERS: u8 = 16;

/// A matchmaking playlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    /// Its number in messages.
    pub id: u8,
    /// Its name in Halo 2's text, saved levels and settings, like
    /// `double_team`.
    pub key: String,
    /// The name players see when their PC has no text for `key`.
    pub name: String,
    /// Who plays it: the game, or the launcher (on MCC's engine).
    pub client: ClientKind,
    /// Games change levels.
    pub ranked: bool,
    /// Red against blue; otherwise everyone for themselves.
    pub teams: bool,
    /// The fewest and most people (not bots) in a game.
    pub min: u8,
    pub max: u8,
    /// The biggest party that can search, guests included.
    pub party_max: u8,
    /// Splitscreen guests can play.
    pub guests: bool,
    pub bots: Bots,
    pub variants: Vec<Variant>,
    /// Map file names, without `.map` (MCC's, for the launcher).
    pub maps: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bots {
    None,
    /// One bot evens out teams one player apart.
    Even,
    /// Bots fill the game up to this many players.
    Fill(u8),
}

/// A game in a playlist's rotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub game_type: GameType,
    /// One of `PRESETS` (empty in launcher playlists).
    pub preset: String,
    /// Points to win (seconds in the timed game types).
    pub score: u32,
    /// Seconds before the game ends anyway; 0 for no limit (or, in
    /// launcher playlists, as the MCC variant says).
    pub time_limit: u16,
    /// In launcher playlists, the MCC game variant's name (like
    /// `01_slayer`), which says the rest.
    pub mcc: Option<String>,
}

/// Halo 2's launch playlists and two later ones, as far as this game can
/// play them, and for the launcher.
pub fn built_in() -> Vec<Playlist> {
    parse(BUILT_IN).expect("the built-in playlists are valid")
}

/// The playlists in the file at `path`, or the built-in ones if there's no
/// such file.
pub fn load(path: &Path) -> Result<Vec<Playlist>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(built_in()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Read playlists in the format described at the top of this module.
pub fn parse(text: &str) -> Result<Vec<Playlist>, String> {
    // Each playlist and the line it starts on.
    let mut playlists: Vec<(usize, Playlist)> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |e: String| format!("line {n}: {e}");
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let args: Vec<&str> = rest.split_whitespace().collect();
        if word == "playlist" {
            playlists.push((n, start(&args).map_err(at)?));
            continue;
        }
        let Some((_, p)) = playlists.last_mut() else {
            return Err(at(format!("\"{word}\" comes before any playlist line")));
        };
        setting(p, word, &args).map_err(at)?;
    }
    if playlists.is_empty() {
        return Err("no playlists".into());
    }
    for (n, p) in &mut playlists {
        if let Err(e) = check(p) {
            return Err(format!("line {n}: playlist {}: {e}", p.key));
        }
    }
    for (k, (n, p)) in playlists.iter().enumerate() {
        let at = |e: String| format!("line {n}: playlist {}: {e}", p.key);
        let others = &playlists[..k];
        if others.iter().any(|(_, o)| o.id == p.id) {
            return Err(at(format!("another playlist is already number {}", p.id)));
        }
        if others.iter().any(|(_, o)| o.key == p.key) {
            return Err(at("another playlist already has this key".into()));
        }
    }
    Ok(playlists.into_iter().map(|(_, p)| p).collect())
}

/// A playlist from its `playlist` line, with every other setting at its
/// default.
fn start(args: &[&str]) -> Result<Playlist, String> {
    let (id, key, name) = match args {
        [id, key, name @ ..] if !name.is_empty() => (id, key, name),
        _ => return Err("expected playlist <id> <key> <name>".into()),
    };
    let id: u8 = number(id)?;
    if id == u8::MAX {
        return Err("playlist numbers go from 0 to 254".into());
    }
    let key_char = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_';
    if !key.chars().all(key_char) {
        return Err(format!(
            "\"{key}\" isn't a key: use lowercase letters, digits and _"
        ));
    }
    Ok(Playlist {
        id,
        key: key.to_string(),
        name: name.join(" "),
        client: ClientKind::Viewer,
        ranked: false,
        teams: false,
        min: 0,
        max: 0,
        party_max: 0,
        guests: false,
        bots: Bots::None,
        variants: Vec::new(),
        maps: Vec::new(),
    })
}

/// Apply one setting line to `p`.
fn setting(p: &mut Playlist, word: &str, args: &[&str]) -> Result<(), String> {
    match (word, args) {
        ("client", ["viewer"]) => p.client = ClientKind::Viewer,
        ("client", ["launcher"]) => p.client = ClientKind::Launcher,
        ("ranked", [yes]) => p.ranked = yes_no(yes)?,
        ("guests", [yes]) => p.guests = yes_no(yes)?,
        ("humans", [min, max]) => {
            let (min, max) = (number(min)?, number(max)?);
            if !(2 <= min && min <= max && max <= MAX_PLAYERS) {
                return Err(format!(
                    "humans must be from 2 to {MAX_PLAYERS}, the fewest first"
                ));
            }
            (p.min, p.max) = (min, max);
        }
        ("party", [n]) => {
            p.party_max = number(n)?;
            if !(1..=MAX_PLAYERS).contains(&p.party_max) {
                return Err(format!("parties are 1 to {MAX_PLAYERS} people"));
            }
        }
        ("bots", ["none"]) => p.bots = Bots::None,
        ("bots", ["even"]) => p.bots = Bots::Even,
        ("bots", ["fill", n]) => {
            let n = number(n)?;
            if n > MAX_PLAYERS {
                return Err(format!("bots can fill up to {MAX_PLAYERS} players"));
            }
            p.bots = Bots::Fill(n);
        }
        ("variant", [game_type, preset, score, time_limit]) => {
            let game_type = game_type_named(game_type)?;
            let Some(preset) = PRESETS
                .iter()
                .find(|n| preset_word(n).eq_ignore_ascii_case(preset))
            else {
                let words: Vec<String> = PRESETS.iter().map(|n| preset_word(n)).collect();
                return Err(format!(
                    "unknown preset \"{preset}\" (the presets are {})",
                    words.join(", ")
                ));
            };
            p.variants.push(Variant {
                game_type,
                preset: preset.to_string(),
                score: number(score)?,
                time_limit: number(time_limit)?,
                mcc: None,
            });
        }
        ("mcc_variant", [game_type, name]) => {
            let game_type = game_type_named(game_type)?;
            if !file_name(name) {
                return Err(format!("\"{name}\" isn't an MCC variant's file name"));
            }
            p.variants.push(Variant {
                game_type,
                preset: String::new(),
                score: 0,
                time_limit: 0,
                mcc: Some(name.to_string()),
            });
        }
        ("maps", [_, ..]) => p.maps.extend(args.iter().map(|m| m.to_string())),
        ("client", _) => return Err("expected client viewer or client launcher".into()),
        ("ranked" | "guests", _) => return Err(format!("expected {word} yes or {word} no")),
        ("humans", _) => return Err("expected humans <fewest> <most>".into()),
        ("party", _) => return Err("expected party <most people>".into()),
        ("bots", _) => return Err("expected bots none, bots even or bots fill <n>".into()),
        ("variant", _) => {
            return Err("expected variant <game type> <preset> <score> <time limit>".into())
        }
        ("mcc_variant", _) => return Err("expected mcc_variant <game type> <variant>".into()),
        ("maps", _) => return Err("expected maps <name>...".into()),
        _ => return Err(format!("unknown setting \"{word}\"")),
    }
    Ok(())
}

/// Check a whole playlist once all its lines are read, and fill in what
/// follows from them.
fn check(p: &mut Playlist) -> Result<(), String> {
    if p.max == 0 {
        return Err("no humans line".into());
    }
    if p.party_max == 0 {
        p.party_max = p.max;
    }
    let Some(first) = p.variants.first() else {
        return Err(match p.client {
            ClientKind::Viewer => "no variant lines".into(),
            ClientKind::Launcher => "no mcc_variant lines".into(),
        });
    };
    p.teams = first.game_type.teams();
    if p.variants.iter().any(|v| v.game_type.teams() != p.teams) {
        return Err("mixes team and free-for-all game types".into());
    }
    if p.maps.is_empty() {
        return Err("no maps".into());
    }
    if p.maps.len() > MAX_PLAYLIST_MAPS {
        return Err(format!("more than {MAX_PLAYLIST_MAPS} maps"));
    }
    if let Some(long) = p.maps.iter().find(|m| m.len() > MAX_NAME) {
        return Err(format!("\"{long}\" is longer than a map's name can be"));
    }
    if p.ranked && p.guests {
        return Err("ranked playlists can't have guests".into());
    }
    if p.client == ClientKind::Launcher {
        if p.variants.iter().any(|v| v.mcc.is_none()) {
            return Err("launcher playlists play MCC variants (mcc_variant lines)".into());
        }
        if p.guests {
            return Err("launcher playlists can't have guests".into());
        }
        if p.bots != Bots::None {
            return Err("launcher playlists can't have bots".into());
        }
        if let Some(bad) = p.maps.iter().find(|m| !file_name(m)) {
            return Err(format!("\"{bad}\" isn't an MCC map's file name"));
        }
    } else if p.variants.iter().any(|v| v.mcc.is_some()) {
        return Err("only launcher playlists play MCC variants".into());
    }
    match p.bots {
        Bots::Even if !p.teams => Err("only team games have teams to even out".into()),
        Bots::Fill(_) if p.ranked => Err("ranked playlists can't fill with bots".into()),
        _ => Ok(()),
    }
}

/// The game type a playlists file names `word`.
fn game_type_named(word: &str) -> Result<GameType, String> {
    match GAME_TYPES.iter().find(|(_, name)| *name == word) {
        Some(&(game_type, _)) => Ok(game_type),
        None => {
            let names: Vec<&str> = GAME_TYPES.iter().map(|(_, name)| *name).collect();
            Err(format!(
                "unknown game type \"{word}\" (the game types are {})",
                names.join(", ")
            ))
        }
    }
}

/// `name` can be an MCC file's name, without its extension, as the
/// launcher opens it: letters, digits and `_ - .`, not starting with a dot
/// (so never a path), and no longer than any name in a message.
pub fn file_name(name: &str) -> bool {
    let ok = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.');
    !name.is_empty() && name.len() <= MAX_NAME && !name.starts_with('.') && name.chars().all(ok)
}

fn yes_no(word: &str) -> Result<bool, String> {
    match word {
        "yes" => Ok(true),
        "no" => Ok(false),
        _ => Err(format!("expected yes or no, not \"{word}\"")),
    }
}

fn number<T: std::str::FromStr>(word: &str) -> Result<T, String> {
    word.parse()
        .map_err(|_| format!("\"{word}\" isn't a number in range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A valid playlist to break.
    const DOUBLE_TEAM: &str = "\
playlist 2 double_team Double Team
ranked yes
humans 4 4
party 2
variant team_slayer default 25 600
variant team_oddball default 120 600
maps lockout midship
maps warlock
";

    /// The error from reading `DOUBLE_TEAM` with `line` added at the end.
    fn error_with(line: &str) -> String {
        parse(&format!("{DOUBLE_TEAM}{line}\n")).unwrap_err()
    }

    #[test]
    fn the_launch_playlists_are_built_in() {
        let all = built_in();
        let playlists: Vec<Playlist> = all
            .iter()
            .filter(|p| p.client == ClientKind::Viewer)
            .cloned()
            .collect();
        let names: Vec<(u8, &str, &str)> = playlists
            .iter()
            .map(|p| (p.id, p.key.as_str(), p.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                (0, "ffa", "Rumble Pit"),
                (1, "head_to_head", "Head to Head"),
                (2, "double_team", "Double Team"),
                (3, "team_slayer", "Team Slayer"),
                (4, "small_team", "Team Skirmish"),
                (5, "big_team", "Big Team Battle"),
                (6, "small_team_unranked", "Team Training"),
                (7, "team_snipers", "Team Snipers"),
                (8, "team_hardcore", "Team Hardcore"),
            ]
        );
        let rumble = &playlists[0];
        assert!(rumble.ranked && !rumble.teams && !rumble.guests);
        assert_eq!((rumble.min, rumble.max, rumble.party_max), (3, 8, 1));
        assert_eq!(rumble.bots, Bots::None);
        assert_eq!(
            rumble.variants[3],
            Variant {
                game_type: GameType::Oddball,
                preset: "DEFAULT".into(),
                score: 100,
                time_limit: 720,
                mcc: None,
            }
        );
        assert_eq!(rumble.variants[1].preset, "ROCKETS");
        assert_eq!(rumble.maps.len(), 10);
        assert_eq!(rumble.maps[9], "foundation");
        let team_slayer = &playlists[3];
        assert!(team_slayer.ranked && team_slayer.teams);
        assert_eq!(team_slayer.bots, Bots::Even);
        assert_eq!(team_slayer.party_max, 4);
        let btb = &playlists[5];
        assert!(!btb.ranked && btb.teams && btb.guests);
        assert_eq!((btb.min, btb.max, btb.party_max), (2, 16, 8));
        assert_eq!(btb.bots, Bots::Fill(12));
        assert!(btb.maps.iter().any(|m| m == "dune"));
        // Team Training plays Team Slayer's and Team Skirmish's games and
        // maps.
        let training = &playlists[6];
        assert_eq!(training.bots, Bots::Fill(8));
        for other in [&playlists[3], &playlists[4]] {
            assert!(other.variants.iter().all(|v| training.variants.contains(v)));
            assert!(other.maps.iter().all(|m| training.maps.contains(m)));
        }
        // Team Snipers, as Halo 2 described it: "unranked Team Slayer games
        // using only your wits and a sniper rifle. Guests are allowed and
        // any size party can join" (four, so teams can be even).
        let snipers = &playlists[7];
        assert!(!snipers.ranked && snipers.teams && snipers.guests);
        assert_eq!((snipers.min, snipers.max, snipers.party_max), (2, 8, 4));
        assert_eq!(snipers.bots, Bots::Fill(8));
        assert!(snipers
            .variants
            .iter()
            .all(|v| (v.game_type, v.preset.as_str()) == (GameType::TeamSlayer, "TEAM SNIPERS")));
        // Team Hardcore: "two teams of four wage war in a variety of ranked
        // gametypes featuring non-default starting weapons. Motion sensor is
        // disabled."
        let hardcore = &playlists[8];
        assert!(hardcore.ranked && hardcore.teams && !hardcore.guests);
        assert_eq!((hardcore.min, hardcore.max, hardcore.party_max), (4, 8, 4));
        assert_eq!(hardcore.bots, Bots::Even);
        let types: Vec<GameType> = hardcore.variants.iter().map(|v| v.game_type).collect();
        assert_eq!(
            types,
            [
                GameType::TeamSlayer,
                GameType::TeamSlayer,
                GameType::Ctf,
                GameType::TeamOddball,
                GameType::TeamKing
            ]
        );
        assert!(hardcore
            .variants
            .iter()
            .all(|v| v.preset == "HARDCORE" || v.preset == "TEAM SNIPERS"));
    }

    /// MCC's Halo 2 multiplayer maps, by file name.
    const MCC_MAPS: [&str; 25] = [
        "ascension",
        "backwash",
        "beavercreek",
        "burial_mounds",
        "coagulation",
        "colossus",
        "containment",
        "cyclotron",
        "deltatap",
        "derelict",
        "dune",
        "elongation",
        "foundation",
        "gemini",
        "headlong",
        "highplains",
        "lockout",
        "midship",
        "needle",
        "street_sweeper",
        "triplicate",
        "turf",
        "warlock",
        "waterworks",
        "zanzibar",
    ];

    #[test]
    fn the_launcher_has_halo_2s_playlists_on_mccs_maps() {
        let all = built_in();
        let launcher: Vec<&Playlist> = all
            .iter()
            .filter(|p| p.client == ClientKind::Launcher)
            .collect();
        let names: Vec<(u8, &str, &str, bool, bool)> = launcher
            .iter()
            .map(|p| (p.id, p.key.as_str(), p.name.as_str(), p.ranked, p.teams))
            .collect();
        assert_eq!(
            names,
            [
                (10, "mcc_ffa", "Rumble Pit", true, false),
                (11, "mcc_head_to_head", "Head to Head", true, false),
                (12, "mcc_double_team", "Double Team", true, true),
                (13, "mcc_team_slayer", "Team Slayer", true, true),
                (14, "mcc_small_team", "Team Skirmish", true, true),
                (15, "mcc_big_team", "Big Team Battle", false, true),
                (16, "mcc_team_snipers", "Team Snipers", false, true),
                (17, "mcc_team_hardcore", "Team Hardcore", true, true),
            ]
        );
        for p in &launcher {
            // No bots or guests: MCC's engine plays one person on each PC.
            assert_eq!((p.bots, p.guests), (Bots::None, false), "{}", p.key);
            // The stock variant everywhere, until the hopper variants'
            // file names are read from the owner's install.
            for v in &p.variants {
                assert_eq!(v.mcc.as_deref(), Some("01_slayer"), "{}", p.key);
            }
            for map in &p.maps {
                assert!(MCC_MAPS.contains(&map.as_str()), "{}: {map}", p.key);
            }
            // As many people as the game's own playlist (but for Big Team
            // Battle, which has no bots to make up the numbers).
            let game = all.iter().find(|g| format!("mcc_{}", g.key) == p.key);
            let game = game.unwrap();
            assert_eq!((p.max, p.party_max), (game.max, game.party_max));
        }
        // Every map is played somewhere.
        for map in MCC_MAPS {
            let played = launcher.iter().any(|p| p.maps.iter().any(|m| m == map));
            assert!(played, "{map}");
        }
        // The game's playlists play no MCC variants.
        let games = all.iter().filter(|p| p.client == ClientKind::Viewer);
        assert!(games.flat_map(|p| &p.variants).all(|v| v.mcc.is_none()));
    }

    #[test]
    fn launcher_playlists_name_mcc_variants() {
        let text = "playlist 20 mcc_test Test\nclient launcher\nhumans 2 4\n\
                    mcc_variant team_slayer my_variant-2.v1\nmaps lockout zanzibar\n";
        let p = &parse(text).unwrap()[0];
        assert_eq!(p.client, ClientKind::Launcher);
        assert!(p.teams);
        assert_eq!(
            p.variants,
            [Variant {
                game_type: GameType::TeamSlayer,
                preset: String::new(),
                score: 0,
                time_limit: 0,
                mcc: Some("my_variant-2.v1".into()),
            }]
        );
        let check = |extra: &str| parse(&format!("{text}{extra}\n")).unwrap_err();
        assert_eq!(
            check("variant team_slayer default 50 600"),
            "line 1: playlist mcc_test: launcher playlists play MCC variants (mcc_variant lines)"
        );
        assert_eq!(
            check("bots fill 8"),
            "line 1: playlist mcc_test: launcher playlists can't have bots"
        );
        assert_eq!(
            check("guests yes"),
            "line 1: playlist mcc_test: launcher playlists can't have guests"
        );
        assert_eq!(
            check("maps ..\\lockout"),
            "line 1: playlist mcc_test: \"..\\lockout\" isn't an MCC map's file name"
        );
        assert_eq!(
            check("mcc_variant slayer x"),
            "line 1: playlist mcc_test: mixes team and free-for-all game types"
        );
        assert_eq!(
            check("client xbox"),
            "line 6: expected client viewer or client launcher"
        );
        for bad in ["../secret", ".hidden", "a/b", "c:d", "x*", "ü"] {
            assert_eq!(
                check(&format!("mcc_variant slayer {bad}")),
                format!("line 6: \"{bad}\" isn't an MCC variant's file name")
            );
        }
        assert_eq!(
            check("mcc_variant team_slayer"),
            "line 6: expected mcc_variant <game type> <variant>"
        );
        assert_eq!(
            parse("playlist 20 t T\nclient launcher\nhumans 2 4\nmaps x\n").unwrap_err(),
            "line 1: playlist t: no mcc_variant lines"
        );
        // The game's playlists can't play them.
        assert_eq!(
            error_with("mcc_variant team_slayer 01_slayer"),
            "line 1: playlist double_team: only launcher playlists play MCC variants"
        );
        assert!(file_name("01_slayer") && file_name("street_sweeper"));
        assert!(!file_name("") && !file_name(&"x".repeat(MAX_NAME + 1)));
    }

    #[test]
    fn a_file_replaces_the_built_in_playlists() {
        let playlists = parse(DOUBLE_TEAM).unwrap();
        assert_eq!(playlists.len(), 1);
        let p = &playlists[0];
        assert_eq!(p.client, ClientKind::Viewer);
        assert_eq!(p.maps, ["lockout", "midship", "warlock"]);
        assert_eq!(p.variants[1].game_type, GameType::TeamOddball);
        assert!(p.teams && !p.guests);
        // Comments, blank lines, indents and defaults.
        let text = "# Practice.\n\nplaylist 9 practice Practice  Room\n  humans 2 6\n  \
                    variant slayer SWORDS 10 0\n  maps lockout\n";
        let p = &parse(text).unwrap()[0];
        assert_eq!(p.name, "Practice Room");
        assert!(!p.ranked && !p.teams && !p.guests);
        assert_eq!((p.party_max, p.bots), (6, Bots::None));
        assert_eq!(p.variants[0].preset, "SWORDS");
        assert_eq!(p.variants[0].time_limit, 0);
    }

    #[test]
    fn a_missing_file_means_the_built_in_playlists() {
        let missing = Path::new("/nonexistent/h2live/playlists.txt");
        assert_eq!(load(missing).unwrap(), built_in());
    }

    #[test]
    fn mistakes_say_what_and_where() {
        assert_eq!(
            error_with("variant slayr default 25 600"),
            "line 9: unknown game type \"slayr\" (the game types are slayer, team_slayer, \
             ctf, king, team_king, oddball, team_oddball, juggernaut, territories, assault)"
        );
        assert_eq!(
            error_with("variant team_slayer pistols 25 600"),
            "line 9: unknown preset \"pistols\" (the presets are default, swat, rockets, \
             snipers, swords, shotguns, hardcore, team_snipers)"
        );
        assert_eq!(
            error_with("variant team_slayer default 25"),
            "line 9: expected variant <game type> <preset> <score> <time limit>"
        );
        assert_eq!(
            error_with("variant team_slayer default 25 100000"),
            "line 9: \"100000\" isn't a number in range"
        );
        assert_eq!(
            error_with("humans 1 4"),
            "line 9: humans must be from 2 to 16, the fewest first"
        );
        assert_eq!(
            error_with("humans 8 4"),
            "line 9: humans must be from 2 to 16, the fewest first"
        );
        assert_eq!(error_with("party 0"), "line 9: parties are 1 to 16 people");
        assert_eq!(
            error_with("ranked maybe"),
            "line 9: expected yes or no, not \"maybe\""
        );
        assert_eq!(
            error_with("bots fill"),
            "line 9: expected bots none, bots even or bots fill <n>"
        );
        assert_eq!(error_with("radar off"), "line 9: unknown setting \"radar\"");
        assert_eq!(
            parse("humans 2 4\n").unwrap_err(),
            "line 1: \"humans\" comes before any playlist line"
        );
        assert_eq!(
            parse("playlist 255 x X\n").unwrap_err(),
            "line 1: playlist numbers go from 0 to 254"
        );
        assert_eq!(
            parse("playlist 1 Team-Slayer X\n").unwrap_err(),
            "line 1: \"Team-Slayer\" isn't a key: use lowercase letters, digits and _"
        );
        assert_eq!(
            parse("playlist 1 x\n").unwrap_err(),
            "line 1: expected playlist <id> <key> <name>"
        );
        assert_eq!(parse("# nothing\n").unwrap_err(), "no playlists");
    }

    #[test]
    fn whole_playlists_are_checked() {
        let check = |text: &str| parse(text).unwrap_err();
        assert_eq!(
            check("playlist 1 a A\nvariant slayer default 1 0\nmaps x\n"),
            "line 1: playlist a: no humans line"
        );
        assert_eq!(
            check("playlist 1 a A\nhumans 2 4\nmaps x\n"),
            "line 1: playlist a: no variant lines"
        );
        assert_eq!(
            check("playlist 1 a A\nhumans 2 4\nvariant slayer default 1 0\n"),
            "line 1: playlist a: no maps"
        );
        assert_eq!(
            error_with("variant slayer default 25 600"),
            "line 1: playlist double_team: mixes team and free-for-all game types"
        );
        assert_eq!(
            error_with("guests yes"),
            "line 1: playlist double_team: ranked playlists can't have guests"
        );
        assert_eq!(
            error_with("bots fill 8"),
            "line 1: playlist double_team: ranked playlists can't fill with bots"
        );
        assert_eq!(
            check("playlist 1 a A\nhumans 2 4\nbots even\nvariant slayer default 1 0\nmaps x\n"),
            "line 1: playlist a: only team games have teams to even out"
        );
        // As many maps as a PC is sent, with names as long as it reads.
        let maps =
            |n: usize, name: &str| format!("{DOUBLE_TEAM}maps {}\n", [name].repeat(n).join(" "));
        assert!(parse(&maps(MAX_PLAYLIST_MAPS - 3, "lockout")).is_ok());
        assert_eq!(
            check(&maps(MAX_PLAYLIST_MAPS - 2, "lockout")),
            "line 1: playlist double_team: more than 64 maps"
        );
        assert!(parse(&maps(1, &"x".repeat(MAX_NAME))).is_ok());
        assert_eq!(
            check(&maps(1, &"x".repeat(MAX_NAME + 1))),
            format!(
                "line 1: playlist double_team: \"{}\" is longer than a map's name can be",
                "x".repeat(MAX_NAME + 1)
            )
        );
        assert_eq!(
            check(&format!(
                "{DOUBLE_TEAM}\n{}",
                DOUBLE_TEAM.replace("double_team", "other")
            )),
            "line 10: playlist other: another playlist is already number 2"
        );
        assert_eq!(
            check(&format!(
                "{DOUBLE_TEAM}\n{}",
                DOUBLE_TEAM.replace("playlist 2", "playlist 3")
            )),
            "line 10: playlist double_team: another playlist already has this key"
        );
    }
}
