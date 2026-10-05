//! Recent Players, as Halo 2 kept them: the people this PC last played
//! matches and custom games with, newest first, with what and where they
//! played. Each PC keeps its own list, in a file beside its key (see
//! `Online::sign_in`), so it outlasts the game and the server. A line a
//! player:
//!
//! ```text
//! <account> <unix time> <level> <playlist key, or custom> <map> <gamertag>
//! ```
//!
//! with the account in hex. Gamertags can hold spaces; a map's are kept as
//! `_` (which its title shows as spaces again).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Most players the list keeps; the longest ago go first.
pub const MAX_RECENT: usize = 50;
/// What the list says was played in a party's custom game.
pub const CUSTOM: &str = "custom";

/// Someone we played with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recent {
    pub account: u64,
    pub gamertag: String,
    /// Their level then: in the match's playlist, or their best in a custom
    /// game.
    pub level: u8,
    /// The match's playlist, by key, or `CUSTOM`.
    pub played: String,
    /// The map's file name.
    pub map: String,
    /// When, in Unix time.
    pub when: u64,
}

/// This PC's recent players, newest first, and where they're kept.
#[derive(Default)]
pub struct RecentPlayers {
    path: Option<PathBuf>,
    pub list: Vec<Recent>,
}

impl RecentPlayers {
    /// The list kept at `path` (empty if there's none yet).
    pub fn load(path: &Path) -> RecentPlayers {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        RecentPlayers {
            path: Some(path.to_path_buf()),
            list: parse(&text),
        }
    }

    /// We just played with `players`: they go to the top of the list, once
    /// each, and the list is kept.
    pub fn met(&mut self, players: Vec<Recent>) {
        if players.is_empty() {
            return;
        }
        self.list
            .retain(|r| !players.iter().any(|p| p.account == r.account));
        self.list.splice(0..0, players);
        self.list.truncate(MAX_RECENT);
        let Some(path) = &self.path else {
            return;
        };
        if let Err(e) = h2live::store::replace(path, &to_text(&self.list)) {
            println!("live: can't keep recent players in {}: {e}", path.display());
        }
    }
}

/// A list as `to_text` writes it; lines that aren't a player are skipped.
fn parse(text: &str) -> Vec<Recent> {
    let player = |line: &str| {
        let mut words = line.splitn(6, ' ');
        Some(Recent {
            account: u64::from_str_radix(words.next()?, 16).ok()?,
            when: words.next()?.parse().ok()?,
            level: words.next()?.parse().ok()?,
            played: words.next()?.to_string(),
            map: words.next()?.to_string(),
            gamertag: words.next().filter(|g| !g.is_empty())?.to_string(),
        })
    };
    let mut list: Vec<Recent> = text.lines().filter_map(player).collect();
    list.truncate(MAX_RECENT);
    list
}

fn to_text(list: &[Recent]) -> String {
    list.iter()
        .map(|r| {
            let map = r.map.replace(' ', "_");
            format!(
                "{:016x} {} {} {} {map} {}\n",
                r.account, r.when, r.level, r.played, r.gamertag
            )
        })
        .collect()
}

/// Now, in Unix time.
pub fn unix_now() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH);
    since.map_or(0, |d| d.as_secs())
}

/// How long before `now` `then` was (both Unix times): "JUST NOW", "5
/// MINUTES AGO", "2 HOURS AGO" or "3 DAYS AGO".
pub fn ago(then: u64, now: u64) -> String {
    let minutes = now.saturating_sub(then) / 60;
    let (n, unit) = match minutes {
        0 => return "JUST NOW".into(),
        1..=59 => (minutes, "MINUTE"),
        60..=2879 => (minutes / 60, "HOUR"),
        _ => (minutes / 1440, "DAY"),
    };
    let s = if n == 1 { "" } else { "S" };
    format!("{n} {unit}{s} AGO")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recent(account: u64, gamertag: &str, when: u64) -> Recent {
        Recent {
            account,
            gamertag: gamertag.into(),
            level: 9,
            played: "team_snipers".into(),
            map: "lockout".into(),
            when,
        }
    }

    #[test]
    fn players_met_go_to_the_top_once_and_are_kept() {
        let path = std::env::temp_dir().join(format!("h2-recent-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut players = RecentPlayers::load(&path);
        assert!(players.list.is_empty(), "none yet");
        players.met(vec![recent(1, "SARGE", 100), recent(2, "VIPER", 100)]);
        players.met(vec![recent(3, "THE ARBITER", 200)]);
        // VIPER again, renamed, in a custom game: on top, and only there.
        let custom = Recent {
            played: CUSTOM.into(),
            map: "midship".into(),
            level: 30,
            ..recent(2, "COBRA", 300)
        };
        players.met(vec![custom.clone()]);
        let accounts: Vec<u64> = players.list.iter().map(|r| r.account).collect();
        assert_eq!(accounts, [2, 3, 1]);
        // Kept as they were, spaces in gamertags too.
        let again = RecentPlayers::load(&path);
        assert_eq!(
            again.list,
            [
                custom,
                recent(3, "THE ARBITER", 200),
                recent(1, "SARGE", 100)
            ]
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().lines().next(),
            Some("0000000000000002 300 30 custom midship COBRA")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn maps_with_spaces_are_kept_whole() {
        // A custom game on a map file named "my map.map".
        let custom = Recent {
            played: CUSTOM.into(),
            map: "my map".into(),
            ..recent(2, "BRAVO", 300)
        };
        let list = parse(&to_text(&[custom]));
        assert_eq!(list.len(), 1);
        assert_eq!(
            (list[0].map.as_str(), list[0].gamertag.as_str()),
            ("my_map", "BRAVO")
        );
        assert_eq!(crate::menu::map_title(&list[0].map), "MY MAP");
    }

    #[test]
    fn the_list_keeps_the_last_fifty() {
        let mut players = RecentPlayers::default();
        for k in 0..60 {
            players.met(vec![recent(k, "PLAYER", k)]);
        }
        assert_eq!(players.list.len(), MAX_RECENT);
        assert_eq!(players.list[0].account, 59);
        assert_eq!(players.list[MAX_RECENT - 1].account, 10);
        // A file longer than that, or with lines that aren't players.
        let text: String = (0..60)
            .map(|k| format!("{k:x} {k} 1 ffa lockout P{k}\nnot a player\n"))
            .collect();
        let list = parse(&format!("{text}5 5 1 ffa lockout\n"));
        assert_eq!(list.len(), MAX_RECENT);
        assert_eq!(list[MAX_RECENT - 1].gamertag, "P49");
    }

    #[test]
    fn times_are_said_roughly() {
        let now = 1_000_000;
        assert_eq!(ago(now - 59, now), "JUST NOW");
        assert_eq!(ago(now - 60, now), "1 MINUTE AGO");
        assert_eq!(ago(now - 59 * 60, now), "59 MINUTES AGO");
        assert_eq!(ago(now - 3600, now), "1 HOUR AGO");
        assert_eq!(ago(now - 47 * 3600, now), "47 HOURS AGO");
        assert_eq!(ago(now - 48 * 3600, now), "2 DAYS AGO");
        assert_eq!(ago(now + 5, now), "JUST NOW", "a clock set back");
    }
}
