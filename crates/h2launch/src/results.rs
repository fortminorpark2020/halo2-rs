//! The game-results block the engine hands host slot 6 (`set_game_result`,
//! `profile::GAME_RESULT_SIZE` bytes) at the end of a game, read only. The
//! layout was worked out on the owner's PC from the blocks of 60 s Slayer
//! games with a known number of suicides (2026-10-10); the host's and the
//! guest's blocks were the same byte for byte. Fields not listed are not
//! known yet (a quit flag, and anything that says the game was cut short).
//!
//! - 0x08 / 0x10: u32 Unix times of the game's start and end.
//! - 16 players from 0x20, 0x1C8 bytes each: +0x00 u32 in use, +0x08 u64
//!   XUID (with `--live`, the player's relay id), +0x18 i32 team, +0x30 i32
//!   standing (0 = first; a tie shares it), +0x34 i32 score, +0x48 f32
//!   seconds played.
//! - 16 statistics blocks from 0x1CE0, 0x5448 bytes each, in the players'
//!   order, u32s: +0x24 kills, +0x28 assists, +0x2C deaths, +0x30
//!   betrayals, +0x34 suicides, +0x38 most kills in a row, +0x3C seconds
//!   alive. Deaths, suicides and seconds alive were seen (+0x00 and +0x04
//!   were 1 in every game, and the lone winner had +0x0C and +0x18 at 1).
//!   Kills, assists, betrayals and most kills in a row are inferred from
//!   the order Halo games keep a player's results statistics in, around
//!   the three seen, and not yet seen above zero: the games read so far had
//!   no kills. `check_kills` says in the log whether the first real games
//!   bear them out.

use h2net::live::LauncherPlayerResult;

pub const PLAYERS: usize = 0x20;
pub const PLAYER_STRIDE: usize = 0x1C8;
pub const STATS: usize = 0x1CE0;
pub const STATS_STRIDE: usize = 0x5448;
pub const MAX_PLAYERS: usize = 16;

/// Kills, in a player's statistics block. Inferred (see the module
/// comment), not yet seen above zero.
pub const KILLS: usize = 0x24;
/// Assists. Inferred, not yet seen above zero.
pub const ASSISTS: usize = 0x28;
/// Deaths. Seen.
pub const DEATHS: usize = 0x2C;
/// Betrayals (teammates killed). Inferred, not yet seen above zero.
pub const BETRAYALS: usize = 0x30;
/// Suicides. Seen (equal to deaths in games where every death was one).
pub const SUICIDES: usize = 0x34;
/// Most kills in a row. Inferred, not yet seen above zero.
pub const IN_A_ROW: usize = 0x38;
/// Seconds alive. Seen (59 in a 60 s game).
pub const ALIVE: usize = 0x3C;

/// One player's line of the block.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerResult {
    /// Their place in the block (0..16).
    pub index: usize,
    pub xuid: u64,
    pub team: i32,
    pub standing: i32,
    pub score: i32,
    pub seconds: f32,
    pub kills: u32,
    pub assists: u32,
    pub deaths: u32,
    pub betrayals: u32,
    pub suicides: u32,
    /// Most kills in a row.
    pub in_a_row: u32,
    /// Seconds alive.
    pub alive: u32,
}

impl PlayerResult {
    /// As LAUNCHER_RESULT names it: counts too big for its u16s are kept
    /// at the largest. The server ranks by standing and score; the counts
    /// go on the carnage report and the service record.
    pub fn for_server(&self) -> LauncherPlayerResult {
        let byte = |v: i32| v.clamp(0, i32::from(u8::MAX)) as u8;
        let count = |v: u32| v.min(u32::from(u16::MAX)) as u16;
        LauncherPlayerResult {
            relay_id: self.xuid,
            team: byte(self.team),
            place: byte(self.standing),
            score: self.score,
            kills: count(self.kills),
            assists: count(self.assists),
            deaths: count(self.deaths),
            betrayals: count(self.betrayals),
            suicides: count(self.suicides),
            left: false,
        }
    }
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn i32_at(b: &[u8], at: usize) -> Option<i32> {
    u32_at(b, at).map(|v| v as i32)
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// The players in use, in block order.
pub fn players(b: &[u8]) -> Vec<PlayerResult> {
    (0..MAX_PLAYERS)
        .filter_map(|i| {
            let p = PLAYERS + PLAYER_STRIDE * i;
            let s = STATS + STATS_STRIDE * i;
            if u32_at(b, p)? == 0 {
                return None;
            }
            Some(PlayerResult {
                index: i,
                xuid: u64_at(b, p + 0x08)?,
                team: i32_at(b, p + 0x18)?,
                standing: i32_at(b, p + 0x30)?,
                score: i32_at(b, p + 0x34)?,
                seconds: f32::from_bits(u32_at(b, p + 0x48)?),
                kills: u32_at(b, s + KILLS)?,
                assists: u32_at(b, s + ASSISTS)?,
                deaths: u32_at(b, s + DEATHS)?,
                betrayals: u32_at(b, s + BETRAYALS)?,
                suicides: u32_at(b, s + SUICIDES)?,
                in_a_row: u32_at(b, s + IN_A_ROW)?,
                alive: u32_at(b, s + ALIVE)?,
            })
        })
        .collect()
}

/// Lines for the log: the game's times, then one per player.
pub fn describe(b: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    if let (Some(start), Some(end)) = (u32_at(b, 0x08), u32_at(b, 0x10)) {
        out.push(format!(
            "game from unix {start} to {end} ({} s)",
            end.wrapping_sub(start)
        ));
    }
    for p in players(b) {
        out.push(format!(
            "player {} xuid {:#018x} team {} standing {} score {} deaths {} suicides {} \
             kills {} assists {} betrayals {} in a row {} alive {} s played {:.1} s",
            p.index,
            p.xuid,
            p.team,
            p.standing,
            p.score,
            p.deaths,
            p.suicides,
            p.kills,
            p.assists,
            p.betrayals,
            p.in_a_row,
            p.alive,
            p.seconds
        ));
    }
    out
}

/// One line for the log on whether each player's kills (+0x24) are their
/// score plus their suicides (plus their betrayals, where the variant
/// takes a point off for those), as they must be in Slayer. It only
/// checks; it changes nothing.
///
/// The variants' own suicide and betrayal penalties aren't read, so either
/// form for betrayals is taken. A "may be wrong" line where the kills
/// equal the score alone points at a variant without a suicide penalty
/// (`h2launch --variants` shows it) before it points at the offset.
pub fn check_kills(players: &[PlayerResult]) -> String {
    // A game with no kills proves nothing: where every death was a
    // suicide, each score was minus the suicides, and the sum is 0. The
    // kills read must be 0 too, so a wrong offset that reads a number in
    // such a game is still reported below.
    let nothing = players
        .iter()
        .all(|p| p.kills == 0 && i64::from(p.score) + i64::from(p.suicides) == 0);
    if nothing {
        return "kills: nothing to check (no one killed anyone)".into();
    }
    let fits = |p: &PlayerResult| {
        let k = i64::from(p.kills);
        let base = i64::from(p.score) + i64::from(p.suicides);
        k == base || k == base + i64::from(p.betrayals)
    };
    match players.iter().find(|p| !fits(p)) {
        None => {
            "kills: every player's kills are their score plus suicides: the offsets hold".into()
        }
        Some(p) => format!(
            "kills: player {} has kills {} but score {} + suicides {} (+ betrayals {}): \
             the kills offset (+0x24) may be wrong",
            p.index, p.kills, p.score, p.suicides, p.betrayals
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(b: &mut [u8], at: usize, v: u32) {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// A block with two players, as the engine lays it out: an all-suicide
    /// game, as the ones seen on the owner's PC.
    fn block() -> Vec<u8> {
        let mut b = vec![0u8; crate::profile::GAME_RESULT_SIZE];
        put(&mut b, 0x08, 100);
        put(&mut b, 0x10, 160);
        let players = [
            (0x0009_0000_0000_0001u64, 0i32, 0i32, -1i32, 1u32),
            (0x0009_0000_0000_0002, 1, 1, -2, 2),
        ];
        for (i, (xuid, team, standing, score, deaths)) in players.into_iter().enumerate() {
            let p = PLAYERS + PLAYER_STRIDE * i;
            let s = STATS + STATS_STRIDE * i;
            put(&mut b, p, 1);
            b[p + 8..p + 16].copy_from_slice(&xuid.to_le_bytes());
            put(&mut b, p + 0x18, team as u32);
            put(&mut b, p + 0x30, standing as u32);
            put(&mut b, p + 0x34, score as u32);
            put(&mut b, p + 0x48, 60.0f32.to_bits());
            put(&mut b, s + DEATHS, deaths);
            put(&mut b, s + SUICIDES, deaths);
            put(&mut b, s + ALIVE, 59);
        }
        b
    }

    #[test]
    fn reads_the_players_in_use() {
        let b = block();
        let ps = players(&b);
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[1].index, 1);
        assert_eq!(ps[1].xuid, 0x0009_0000_0000_0002);
        assert_eq!((ps[1].team, ps[1].standing, ps[1].score), (1, 1, -2));
        assert_eq!((ps[1].deaths, ps[1].suicides, ps[1].alive), (2, 2, 59));
        assert_eq!(ps[0].seconds, 60.0);
        let lines = describe(&b);
        assert_eq!(lines[0], "game from unix 100 to 160 (60 s)");
        assert_eq!(lines.len(), 3);
        assert!(
            lines[2].ends_with(
                "deaths 2 suicides 2 kills 0 assists 0 betrayals 0 in a row 0 alive 59 s played 60.0 s"
            ),
            "{}",
            lines[2]
        );
        // A short block reads what it can and no further.
        assert!(players(&b[..0x100]).is_empty());
    }

    #[test]
    fn kills_assists_and_the_rest_read_at_their_offsets() {
        let mut b = block();
        let s = STATS + STATS_STRIDE;
        for (at, v) in [
            (KILLS, 7),
            (ASSISTS, 3),
            (BETRAYALS, 1),
            (IN_A_ROW, 4),
            (ALIVE, 41),
        ] {
            put(&mut b, s + at, v);
        }
        let p = &players(&b)[1];
        assert_eq!(
            (p.kills, p.assists, p.betrayals, p.in_a_row, p.alive),
            (7, 3, 1, 4, 41)
        );
        // The other player's are untouched.
        assert_eq!(players(&b)[0].kills, 0);
        let r = p.for_server();
        assert_eq!(
            (r.kills, r.assists, r.deaths, r.betrayals, r.suicides),
            (7, 3, 2, 1, 2)
        );
    }

    #[test]
    fn a_player_goes_to_the_server_by_relay_id_and_place() {
        let p = &players(&block())[1];
        let r = p.for_server();
        assert_eq!(r.relay_id, 0x0009_0000_0000_0002);
        assert_eq!((r.team, r.place, r.score, r.deaths), (1, 1, -2, 2));
        assert_eq!((r.kills, r.suicides, r.left), (0, 2, false));
        // Out-of-range values are kept in range, not wrapped.
        let odd = PlayerResult {
            team: -1,
            standing: 300,
            kills: 1_000_000,
            assists: 65_536,
            deaths: 70_000,
            betrayals: u32::MAX,
            suicides: 80_000,
            ..p.clone()
        };
        let r = odd.for_server();
        assert_eq!((r.team, r.place), (0, 255));
        assert_eq!(
            [r.kills, r.assists, r.deaths, r.betrayals, r.suicides],
            [u16::MAX; 5]
        );
    }

    #[test]
    fn the_kills_check_says_one_of_three_things() {
        let player = |index, score, kills, suicides, betrayals| PlayerResult {
            index,
            xuid: index as u64,
            team: index as i32,
            standing: 0,
            score,
            seconds: 60.0,
            kills,
            assists: 0,
            deaths: 0,
            betrayals,
            suicides,
            in_a_row: 0,
            alive: 60,
        };
        // An all-suicide game (score -2, suicides 2, kills 0) proves nothing.
        let suicides = [player(0, -2, 0, 2, 0), player(1, -1, 0, 1, 0)];
        assert_eq!(
            check_kills(&suicides),
            "kills: nothing to check (no one killed anyone)"
        );
        assert_eq!(
            check_kills(&players(&block())),
            "kills: nothing to check (no one killed anyone)"
        );
        // The same game with a number at +0x24 is caught.
        let wrong = [player(0, -2, 5, 2, 0), player(1, -1, 0, 1, 0)];
        assert_eq!(
            check_kills(&wrong),
            "kills: player 0 has kills 5 but score -2 + suicides 2 (+ betrayals 0): \
             the kills offset (+0x24) may be wrong"
        );
        // Kills are the score plus suicides, with or without betrayals.
        let good = [
            player(0, 10, 11, 1, 0),
            player(1, 4, 6, 0, 2),
            player(2, 4, 4, 0, 2),
        ];
        assert_eq!(
            check_kills(&good),
            "kills: every player's kills are their score plus suicides: the offsets hold"
        );
        let off = [player(0, 10, 11, 1, 0), player(3, 4, 9, 0, 2)];
        assert!(check_kills(&off).starts_with("kills: player 3 has kills 9"));
        assert_eq!(
            check_kills(&[]),
            "kills: nothing to check (no one killed anyone)"
        );
    }
}
