//! The names the lobby shows for what the server sends by file name: MCC's
//! game variants (as the launcher playlists name them) and maps, and the
//! colours of the teams.

use super::canvas::Color;

/// What each matchmaking variant in `crates/h2live/src/playlists.txt` is
/// called on screen. Our own wording, after the variants' settings
/// (`h2launch --variants`).
const VARIANTS: [(&str, &str); 29] = [
    ("h2_ffa_slayer_smgPrimary_magSecondary_25kills", "Slayer"),
    ("H2_FFA_Rockets_b", "Rockets"),
    ("H2_FFA_Shotguns_b", "Shotguns"),
    ("h2_ffa_ball_brPrimary_smgSecondary_60points", "Oddball"),
    (
        "h2_ffa_crazyKing_brPrimary_arSecondary_120points",
        "Crazy King",
    ),
    ("H2_FFA_HeadtoHead_b", "Slayer"),
    ("h2_ffa_slayerSwords_25kills", "Swords"),
    ("H2_Team_Slayer_2v2_a", "Team Slayer"),
    (
        "h2_2v2_team_slayer_25kills_brPrimary_magSecondary",
        "Team Slayer",
    ),
    ("H2_Team_Rockets_b", "Team Rockets"),
    (
        "h2_2v2_team_ball_120points_brPrimary_magSecondary",
        "Team Ball",
    ),
    ("H2_Team_Slayer", "Team Slayer"),
    ("H2_Team_BRs", "Team BRs"),
    ("H2_Team_SWAT", "Team SWAT"),
    ("H2_Multi_Flag_CTF_3", "Multi Flag CTF"),
    ("H2_Multi_Bomb", "Multi Bomb"),
    ("H2_Team_Crazy_King", "Team Crazy King"),
    ("H2_Team_Ball", "Team Ball"),
    ("H2_3_Plots", "3 Plots"),
    ("H2_BTB_Multi_Flag", "Multi Flag CTF"),
    ("H2_BTB_Multi_Bomb", "Multi Bomb"),
    ("h2_8v8_team_3plots_br_smg_600points", "3 Plots"),
    ("H2_BTB_Slayer", "Team Slayer"),
    ("H2_Team_Snipers", "Team Snipers"),
    ("h2_arena_snipers_09_2018", "Team Snipers"),
    ("H2_Hardcore_Team_Slayer", "Hardcore Slayer"),
    ("h2_4v4_team_hardcoreFlag_3points", "Hardcore CTF"),
    ("H2_Hardcore_Team_Ball", "Hardcore Ball"),
    ("H2_Hardcore_Team_King", "Hardcore King"),
];

/// A variant's name on screen: from the table, or made from its file name
/// (`H2_Team_Slayer` becomes "Team Slayer").
pub fn variant(file: &str) -> String {
    if let Some((_, name)) = VARIANTS.iter().find(|(f, _)| f.eq_ignore_ascii_case(file)) {
        return name.to_string();
    }
    let lower = file.to_ascii_lowercase();
    let rest = if lower.starts_with("h2_") {
        &file[3..]
    } else {
        file
    };
    let words: Vec<String> = rest
        .split(['_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect(),
                None => String::new(),
            }
        })
        .collect();
    if words.is_empty() {
        file.to_string()
    } else {
        words.join(" ")
    }
}

/// A map's name on screen (Lockout for `lockout`, Ivory Tower for
/// `cyclotron`), or the file name if it isn't one of the 25.
pub fn map(file: &str) -> String {
    match crate::maps::find(file) {
        Some(m) => m.display.to_string(),
        None => file.to_string(),
    }
}

/// Team colours in Halo 2's order (red, blue, yellow, green, purple,
/// orange, brown, pink). The shades are estimates, not read from the game.
const TEAMS: [u32; 8] = [
    0xC8_372D, 0x2F_62C8, 0xD9_C02A, 0x3A_9E3A, 0x8A_3EC2, 0xE0_7A22, 0x8A_5A2B, 0xE0_6AA8,
];

pub fn team_color(team: i32) -> Color {
    Color::rgb(TEAMS[team.rem_euclid(TEAMS.len() as i32) as usize])
}

const TEAM_NAMES: [&str; 8] = [
    "Red", "Blue", "Yellow", "Green", "Purple", "Orange", "Brown", "Pink",
];

pub fn team_name(team: i32) -> &'static str {
    TEAM_NAMES[team.rem_euclid(TEAM_NAMES.len() as i32) as usize]
}

/// "1st", "2nd", ... for a standing counted from 0.
pub fn place(place: u8) -> String {
    let n = u32::from(place) + 1;
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_launcher_variant_has_a_name() {
        let playlists = include_str!("../../../h2live/src/playlists.txt");
        let mut seen = 0;
        for line in playlists.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            if let ["mcc_variant", _, file, ..] = words[..] {
                seen += 1;
                assert!(
                    VARIANTS.iter().any(|(f, _)| f.eq_ignore_ascii_case(file)),
                    "{file} has no name in VARIANTS"
                );
            }
        }
        assert!(seen >= VARIANTS.len(), "{seen} variant lines");
    }

    #[test]
    fn names_on_screen() {
        assert_eq!(variant("H2_Team_SWAT"), "Team SWAT");
        assert_eq!(variant("h2_team_swat"), "Team SWAT");
        assert_eq!(variant("H2_Some_New_variant"), "Some New Variant");
        assert_eq!(variant("01_slayer"), "01 Slayer");
        assert_eq!(map("cyclotron"), "Ivory Tower");
        assert_eq!(map("lockout"), "Lockout");
        assert_eq!(map("somewhere"), "somewhere");
        assert_eq!(team_name(0), "Red");
        assert_eq!(team_name(9), "Blue");
        assert_eq!(team_color(-1), Color::rgb(0xE0_6AA8));
        let places: Vec<String> = [0, 1, 2, 3, 10, 11, 12, 20, 21].map(place).to_vec();
        assert_eq!(
            places,
            ["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd"]
        );
    }
}
