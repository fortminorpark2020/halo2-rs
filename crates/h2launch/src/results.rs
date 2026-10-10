//! The game-results block the engine hands host slot 6 (`set_game_result`,
//! `profile::GAME_RESULT_SIZE` bytes) at the end of a game, read only. The
//! layout was worked out on the owner's PC from the blocks of 60 s Slayer
//! games with a known number of suicides (2026-10-10); the host's and the
//! guest's blocks were the same byte for byte. Fields not listed are not
//! known yet (kills, assists, betrayals, a quit flag, and anything that says
//! the game was cut short: those need games with real kills to find).
//!
//! - 0x08 / 0x10: u32 Unix times of the game's start and end.
//! - 16 players from 0x20, 0x1C8 bytes each: +0x00 u32 in use, +0x08 u64
//!   XUID (with `--live`, the player's relay id), +0x18 i32 team, +0x30 i32
//!   standing (0 = first; a tie shares it), +0x34 i32 score, +0x48 f32
//!   seconds played.
//! - 16 statistics blocks from 0x1CE0, 0x5448 bytes each, in the players'
//!   order: +0x2C u32 deaths, +0x34 u32 suicides (so far only seen equal to
//!   deaths, in games where every death was a suicide, so it may yet turn
//!   out to be another count).

use h2net::live::LauncherPlayerResult;

pub const PLAYERS: usize = 0x20;
pub const PLAYER_STRIDE: usize = 0x1C8;
pub const STATS: usize = 0x1CE0;
pub const STATS_STRIDE: usize = 0x5448;
pub const MAX_PLAYERS: usize = 16;

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
    pub deaths: u32,
    pub suicides: u32,
}

impl PlayerResult {
    /// As LAUNCHER_RESULT names it. Kills aren't in the block as far as is
    /// known, so they go as 0; the server ranks by standing and score.
    pub fn for_server(&self) -> LauncherPlayerResult {
        let byte = |v: i32| v.clamp(0, i32::from(u8::MAX)) as u8;
        LauncherPlayerResult {
            relay_id: self.xuid,
            team: byte(self.team),
            place: byte(self.standing),
            score: self.score,
            kills: 0,
            deaths: self.deaths.min(u32::from(u16::MAX)) as u16,
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
                deaths: u32_at(b, s + 0x2C)?,
                suicides: u32_at(b, s + 0x34)?,
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
            "player {} xuid {:#018x} team {} standing {} score {} deaths {} suicides {} played {:.1} s",
            p.index, p.xuid, p.team, p.standing, p.score, p.deaths, p.suicides, p.seconds
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block with two players, as the engine lays it out.
    fn block() -> Vec<u8> {
        let mut b = vec![0u8; crate::profile::GAME_RESULT_SIZE];
        b[0x08..0x0C].copy_from_slice(&100u32.to_le_bytes());
        b[0x10..0x14].copy_from_slice(&160u32.to_le_bytes());
        let players = [
            (0x0009_0000_0000_0001u64, 0i32, 0i32, -1i32, 1u32),
            (0x0009_0000_0000_0002, 1, 1, -2, 2),
        ];
        for (i, (xuid, team, standing, score, deaths)) in players.into_iter().enumerate() {
            let p = PLAYERS + PLAYER_STRIDE * i;
            let s = STATS + STATS_STRIDE * i;
            b[p..p + 4].copy_from_slice(&1u32.to_le_bytes());
            b[p + 8..p + 16].copy_from_slice(&xuid.to_le_bytes());
            b[p + 0x18..p + 0x1C].copy_from_slice(&team.to_le_bytes());
            b[p + 0x30..p + 0x34].copy_from_slice(&standing.to_le_bytes());
            b[p + 0x34..p + 0x38].copy_from_slice(&score.to_le_bytes());
            b[p + 0x48..p + 0x4C].copy_from_slice(&60.0f32.to_bits().to_le_bytes());
            b[s + 0x2C..s + 0x30].copy_from_slice(&deaths.to_le_bytes());
            b[s + 0x34..s + 0x38].copy_from_slice(&deaths.to_le_bytes());
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
        assert_eq!((ps[1].deaths, ps[1].suicides), (2, 2));
        assert_eq!(ps[0].seconds, 60.0);
        let lines = describe(&b);
        assert_eq!(lines[0], "game from unix 100 to 160 (60 s)");
        assert_eq!(lines.len(), 3);
        // A short block reads what it can and no further.
        assert!(players(&b[..0x100]).is_empty());
    }

    #[test]
    fn a_player_goes_to_the_server_by_relay_id_and_place() {
        let p = &players(&block())[1];
        let r = p.for_server();
        assert_eq!(r.relay_id, 0x0009_0000_0000_0002);
        assert_eq!((r.team, r.place, r.score, r.deaths), (1, 1, -2, 2));
        assert_eq!((r.kills, r.left), (0, false));
        // Out-of-range values are kept in range, not wrapped.
        let odd = PlayerResult {
            team: -1,
            standing: 300,
            deaths: 70_000,
            ..p.clone()
        };
        let r = odd.for_server();
        assert_eq!((r.team, r.place, r.deaths), (0, 255, u16::MAX));
    }
}
