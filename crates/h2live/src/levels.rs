//! Levels 1 to 50, worked out the way Bungie did for Halo 2. Each ranked
//! playlist keeps its own hidden experience (XP) for every player, and only
//! the level shows. A game wins or loses XP against each human opponent:
//! more for beating a higher level, less for losing to one. Low levels lose
//! less and the top levels win less, and a level once reached is kept until
//! XP falls halfway back into the level below.

use std::cmp::Ordering;

/// The highest level.
pub const MAX_LEVEL: u8 = 50;
/// XP never goes above this.
pub const MAX_XP: u32 = 1_000_000_000;

/// XP for a game against one opponent, by the gap between their levels
/// (capped at 15): the higher level beating the lower, or the lower losing
/// to the higher.
const EXPECTED: [i32; 16] = [
    100, 92, 85, 79, 74, 70, 66, 63, 60, 58, 56, 54, 53, 52, 51, 50,
];
/// The lower level beating the higher, or the higher losing to the lower.
const UPSET: [i32; 16] = [
    100, 108, 115, 121, 126, 130, 134, 137, 140, 142, 144, 146, 147, 148, 149, 150,
];
/// Thousandths of a loss that players at levels 1 to 29 really lose (from
/// level 30 they lose all of it).
const LOSS_FACTOR: [i64; 29] = [
    0, 25, 50, 75, 100, 150, 200, 275, 350, 400, 450, 575, 600, 625, 650, 675, 700, 725, 750, 775,
    800, 825, 850, 875, 900, 925, 950, 975, 1000,
];
/// Thousandths of a win that players at levels 42 to 50 really win (below
/// 42 they win all of it).
const WIN_FACTOR: [i64; 9] = [950, 900, 850, 800, 750, 700, 650, 600, 550];
/// The first level whose wins are cut.
const FIRST_CUT_WIN: u8 = 42;
/// Level gaps beyond this count as this.
const MAX_GAP: u8 = 15;

/// The least XP a player can have at `level`.
pub fn min_xp(level: u8) -> u32 {
    let n = u32::from(level.clamp(1, MAX_LEVEL));
    match n {
        1..=12 => 100 * (n - 1),
        13..=16 => 1200 + 200 * (n - 13),
        _ => 2000 + 250 * (n - 17),
    }
}

/// The level `xp` reaches by itself, without the rule that keeps a level
/// once reached.
pub fn level_for_xp(xp: u32) -> u8 {
    (1..=MAX_LEVEL)
        .rev()
        .find(|&n| min_xp(n) <= xp)
        .unwrap_or(1)
}

/// A player's XP and level in one ranked playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rank {
    pub xp: u32,
    pub level: u8,
}

impl Default for Rank {
    fn default() -> Rank {
        Rank { xp: 0, level: 1 }
    }
}

impl Rank {
    /// After winning (or, if negative, losing) `change` XP. A level goes up
    /// as soon as XP reaches it, but only comes down once XP is below
    /// halfway between the level's minimum and the one below's.
    pub fn after(self, change: i32) -> Rank {
        let xp = (i64::from(self.xp) + i64::from(change)).clamp(0, i64::from(MAX_XP)) as u32;
        let mut level = self.level.clamp(1, MAX_LEVEL).max(level_for_xp(xp));
        while level > 1 && 2 * u64::from(xp) < u64::from(min_xp(level - 1) + min_xp(level)) {
            level -= 1;
        }
        Rank { xp, level }
    }
}

/// How many levels away from `level` a player can be matched (Bungie's
/// "maximum match" table).
pub fn level_range(level: u8) -> u8 {
    match level.clamp(1, MAX_LEVEL) {
        l @ 1..=6 => 11 - l,
        7..=16 => 6,
        17..=26 => 7,
        27..=29 => 8,
        l @ 30..=35 => l - 21,
        l => MAX_LEVEL - l,
    }
}

/// Players at levels `a` and `b` can play each other when they're within
/// either one's range, widened by `widened` levels after a long search.
pub fn can_match(a: u8, b: u8, widened: u8) -> bool {
    a.abs_diff(b) <= level_range(a).max(level_range(b)).saturating_add(widened)
}

/// The level a player counts as in a party whose highest level (in this
/// playlist) is `highest`. Members further below than the highest level's
/// range count as the lowest level it can be matched with, for matching,
/// level gaps and the win and loss factors alike.
pub fn effective_level(level: u8, highest: u8) -> u8 {
    level.max(highest.saturating_sub(level_range(highest)))
}

/// How a player finished a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Finish {
    pub team: u8,
    pub score: i32,
    /// Quit before the end.
    pub left: bool,
}

/// Each player's place: 0 is first, and players on the same place tied.
/// In free-for-all games places go by score, and those who left come after
/// everyone who stayed to the end (tied among themselves). In team games
/// players take their team's place, even if they left: `winning_team`
/// first, then the others by team score.
pub fn places(finishes: &[Finish], teams: bool, winning_team: Option<u8>) -> Vec<u8> {
    if teams {
        let team_score = |t: u8| -> i32 {
            finishes
                .iter()
                .filter(|f| f.team == t)
                .map(|f| f.score)
                .sum()
        };
        // Lower is better.
        let order = |t: u8| (Some(t) != winning_team, -team_score(t));
        let mut sides: Vec<u8> = finishes.iter().map(|f| f.team).collect();
        sides.sort_unstable();
        sides.dedup();
        finishes
            .iter()
            .map(|f| {
                let me = order(f.team);
                sides.iter().filter(|&&t| order(t) < me).count() as u8
            })
            .collect()
    } else {
        let stayed = finishes.iter().filter(|f| !f.left).count();
        finishes
            .iter()
            .map(|f| {
                if f.left {
                    stayed as u8
                } else {
                    let ahead = finishes.iter().filter(|o| !o.left && o.score > f.score);
                    ahead.count() as u8
                }
            })
            .collect()
    }
}

/// A player in a finished game, for working out what it was worth to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    /// The level they count as in the playlist (see `effective_level`).
    pub level: u8,
    pub team: u8,
    /// From `places`.
    pub place: u8,
    /// Bots win and lose nothing, and are no one's opponent.
    pub bot: bool,
}

/// What finishing against `other` was worth to `me`, before averaging.
fn versus(me: &Placed, other: &Placed) -> i32 {
    let gap = usize::from(me.level.abs_diff(other.level).min(MAX_GAP));
    let higher = me.level > other.level;
    match me.place.cmp(&other.place) {
        Ordering::Less if higher => EXPECTED[gap],
        Ordering::Less => UPSET[gap],
        Ordering::Greater if higher => -UPSET[gap],
        Ordering::Greater => -EXPECTED[gap],
        Ordering::Equal => 0,
    }
}

/// Thousandths of a loss a player at `level` really loses.
fn loss_factor(level: u8) -> i64 {
    let i = usize::from(level.max(1)) - 1;
    LOSS_FACTOR.get(i).copied().unwrap_or(1000)
}

/// Thousandths of a win a player at `level` really wins.
fn win_factor(level: u8) -> i64 {
    match level.checked_sub(FIRST_CUT_WIN) {
        Some(i) => WIN_FACTOR[usize::from(i).min(WIN_FACTOR.len() - 1)],
        None => 1000,
    }
}

/// `num / den` rounded half away from zero (`den` is positive).
fn round_div(num: i64, den: i64) -> i64 {
    (num.abs() * 2 + den) / (den * 2) * num.signum()
}

/// The XP each player wins (or loses, if negative) in a game: the average
/// of what the game was worth against each human opponent (everyone else in
/// free-for-all games, the other teams' players in team games), times the
/// player's win or loss factor, rounded half away from zero. With no human
/// opponent a game is worth nothing.
pub fn xp_changes(players: &[Placed], teams: bool) -> Vec<i32> {
    players
        .iter()
        .enumerate()
        .map(|(i, me)| {
            let opponents: Vec<&Placed> = players
                .iter()
                .enumerate()
                .filter(|&(j, o)| j != i && !o.bot && !(teams && o.team == me.team))
                .map(|(_, o)| o)
                .collect();
            if me.bot || opponents.is_empty() {
                return 0;
            }
            let total: i32 = opponents.iter().map(|o| versus(me, o)).sum();
            let factor = if total < 0 {
                loss_factor(me.level)
            } else {
                win_factor(me.level)
            };
            let n = opponents.len() as i64;
            round_div(i64::from(total) * factor, n * 1000) as i32
        })
        .collect()
}

/// Whether a game changes levels. It must be in a ranked playlist and have
/// ended on the score to win or the time limit (`finished`), not because the
/// host left. Of the joined PCs that sent their results in time
/// (`reported`), at least half must agree with the host's (`agreeing`). And
/// at least two humans must have played on opposing sides.
pub fn counts(
    ranked: bool,
    finished: bool,
    agreeing: usize,
    reported: usize,
    players: &[Placed],
    teams: bool,
) -> bool {
    let humans: Vec<&Placed> = players.iter().filter(|p| !p.bot).collect();
    let opposed = humans
        .iter()
        .enumerate()
        .any(|(i, a)| humans[i + 1..].iter().any(|b| !teams || a.team != b.team));
    ranked && finished && agreeing * 2 >= reported && opposed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A human in a free-for-all game.
    fn ffa(level: u8, place: u8) -> Placed {
        Placed {
            level,
            team: 0,
            place,
            bot: false,
        }
    }

    fn on_team(level: u8, team: u8, place: u8) -> Placed {
        Placed {
            level,
            team,
            place,
            bot: false,
        }
    }

    fn bot(team: u8, place: u8) -> Placed {
        Placed {
            level: 1,
            team,
            place,
            bot: true,
        }
    }

    fn finish(team: u8, score: i32, left: bool) -> Finish {
        Finish { team, score, left }
    }

    #[test]
    fn levels_start_where_bungies_table_says() {
        assert_eq!(min_xp(1), 0);
        assert_eq!(min_xp(2), 100);
        assert_eq!(min_xp(10), 900);
        assert_eq!(min_xp(12), 1100);
        assert_eq!(min_xp(13), 1200);
        assert_eq!(min_xp(16), 1800);
        assert_eq!(min_xp(17), 2000);
        assert_eq!(min_xp(25), 4000);
        assert_eq!(min_xp(49), 10000);
        assert_eq!(min_xp(50), 10250);
        // Level 10 is 900-999, 13 is 1200-1399, 17 is 2000-2249, 25 is
        // 4000-4249 and 49 is 10000-10249.
        for (level, low, high) in [
            (10, 900, 999),
            (13, 1200, 1399),
            (17, 2000, 2249),
            (25, 4000, 4249),
            (49, 10000, 10249),
        ] {
            assert_eq!(level_for_xp(low - 1), level - 1);
            assert_eq!(level_for_xp(low), level);
            assert_eq!(level_for_xp(high), level);
            assert_eq!(level_for_xp(high + 1), level + 1);
        }
        assert_eq!(level_for_xp(0), 1);
        assert_eq!(level_for_xp(MAX_XP), 50);
    }

    #[test]
    fn a_level_10_beating_levels_7_8_and_5_wins_78() {
        // (79 + 85 + 70) / 3.
        let game = [ffa(10, 0), ffa(7, 1), ffa(8, 2), ffa(5, 3)];
        assert_eq!(xp_changes(&game, false)[0], 78);
        // The same against a team of three.
        let game = [
            on_team(10, 0, 0),
            on_team(7, 1, 1),
            on_team(8, 1, 1),
            on_team(5, 1, 1),
        ];
        assert_eq!(xp_changes(&game, true)[0], 78);
    }

    #[test]
    fn a_level_10_at_900_xp_drops_to_9_only_below_850() {
        let ten = Rank { xp: 900, level: 10 };
        assert_eq!(ten.after(-50), Rank { xp: 850, level: 10 });
        assert_eq!(ten.after(-51), Rank { xp: 849, level: 9 });
        // Coming back up, 9 holds until XP reaches level 10 again.
        let nine = ten.after(-51);
        assert_eq!(nine.after(50).level, 9);
        assert_eq!(nine.after(51), Rank { xp: 900, level: 10 });
    }

    #[test]
    fn xp_moves_levels_up_at_once_and_down_one_halfway_at_a_time() {
        // A big win jumps straight to the level the XP reaches.
        assert_eq!(Rank::default().after(1250).level, 13);
        // A big loss drops through every level whose halfway point it
        // passes: 1150 is halfway between 12 and 13, 1050 between 11 and 12.
        let thirteen = Rank {
            xp: 1200,
            level: 13,
        };
        assert_eq!(thirteen.after(-50).level, 13);
        assert_eq!(thirteen.after(-51).level, 12);
        assert_eq!(thirteen.after(-151).level, 11);
        // A level kept by the halfway rule stays while XP stays above it.
        let held = Rank { xp: 960, level: 11 };
        assert_eq!(held.after(0).level, 11);
        assert_eq!(held.after(-11).level, 10);
        // XP stays between 0 and the cap.
        assert_eq!(
            Rank { xp: 30, level: 1 }.after(-100),
            Rank { xp: 0, level: 1 }
        );
        let top = Rank {
            xp: MAX_XP - 10,
            level: 50,
        };
        assert_eq!(
            top.after(100),
            Rank {
                xp: MAX_XP,
                level: 50
            }
        );
        assert_eq!(Rank { xp: 0, level: 1 }.after(-1).level, 1);
    }

    #[test]
    fn upsets_win_more_and_expected_wins_less() {
        // Five levels apart: 70 for the expected win, 130 for the upset.
        assert_eq!(xp_changes(&[ffa(15, 0), ffa(10, 1)], false)[0], 70);
        assert_eq!(xp_changes(&[ffa(10, 0), ffa(15, 1)], false)[0], 130);
        // Losing as the higher level costs the upset's amount (times level
        // 15's factor, 65%), and as the lower level the expected amount
        // (times level 10's 40%).
        assert_eq!(xp_changes(&[ffa(10, 0), ffa(15, 1)], false)[1], -85); // 130 * 0.65 = 84.5
        assert_eq!(xp_changes(&[ffa(15, 0), ffa(10, 1)], false)[1], -28); // 70 * 0.4
                                                                          // Gaps over 15 count as 15.
        assert_eq!(xp_changes(&[ffa(40, 0), ffa(1, 1)], false)[0], 50);
        assert_eq!(xp_changes(&[ffa(1, 0), ffa(40, 1)], false)[0], 150);
    }

    #[test]
    fn low_levels_lose_less_and_the_top_levels_win_less() {
        let lose = |level| xp_changes(&[ffa(level, 0), ffa(level, 1)], false)[1];
        let win = |level| xp_changes(&[ffa(level, 0), ffa(level, 1)], false)[0];
        assert_eq!(lose(1), 0);
        assert_eq!(lose(2), -3); // 2.5% of 100, rounded away from zero
        assert_eq!(lose(8), -28); // 27.5%
        assert_eq!(lose(12), -58); // 57.5%
        assert_eq!(lose(29), -100);
        assert_eq!(lose(30), -100);
        assert_eq!(lose(50), -100);
        assert_eq!(win(1), 100);
        assert_eq!(win(41), 100);
        assert_eq!(win(42), 95);
        assert_eq!(win(46), 75);
        assert_eq!(win(50), 55);
    }

    #[test]
    fn rounding_goes_half_away_from_zero() {
        assert_eq!(round_div(5, 10), 1);
        assert_eq!(round_div(-5, 10), -1);
        assert_eq!(round_div(4, 10), 0);
        assert_eq!(round_div(-4, 10), 0);
        assert_eq!(round_div(15, 10), 2);
        assert_eq!(round_div(-25, 10), -3);
        assert_eq!(round_div(0, 7), 0);
    }

    #[test]
    fn results_are_averaged_over_every_human_opponent() {
        // Third of four, all level 10: one win and two losses, -100 / 3,
        // then level 10's 40% loss factor: -13.33.
        let game = [ffa(10, 0), ffa(10, 1), ffa(10, 2), ffa(10, 3)];
        assert_eq!(xp_changes(&game, false), [100, 33, -13, -40]);
        // In team games only the other team counts, never teammates.
        let game = [
            on_team(20, 0, 0),
            on_team(5, 0, 0),
            on_team(20, 1, 1),
            on_team(20, 1, 1),
        ];
        let xp = xp_changes(&game, true);
        assert_eq!(xp[0], 100); // two even wins
        assert_eq!(xp[1], 150); // two upsets at the capped gap
                                // (-100 - 150) / 2 times level 20's 77.5%: -96.875.
        assert_eq!(xp[2], -97);
    }

    #[test]
    fn ties_are_worth_nothing() {
        let finishes = [finish(0, 10, false), finish(0, 10, false)];
        let p = places(&finishes, false, None);
        assert_eq!(p, [0, 0]);
        let game = [ffa(10, p[0]), ffa(30, p[1])];
        assert_eq!(xp_changes(&game, false), [0, 0]);
        // A team game that ends level on time.
        let finishes = [
            finish(0, 3, false),
            finish(1, 2, false),
            finish(1, 1, false),
        ];
        assert_eq!(places(&finishes, true, None), [0, 0, 0]);
    }

    #[test]
    fn bots_are_no_ones_opponent() {
        // Beating three bots is worth nothing.
        let game = [ffa(10, 0), bot(0, 1), bot(0, 2), bot(0, 3)];
        assert_eq!(xp_changes(&game, false), [0, 0, 0, 0]);
        // Bots in between humans don't change what the humans are worth.
        let game = [ffa(10, 0), bot(0, 1), ffa(10, 2)];
        assert_eq!(xp_changes(&game, false), [100, 0, -40]);
        // Humans against a team of bots.
        let game = [on_team(10, 0, 0), on_team(12, 0, 0), bot(1, 1)];
        assert_eq!(xp_changes(&game, true), [0, 0, 0]);
    }

    #[test]
    fn free_for_all_leavers_come_after_everyone_who_stayed() {
        let finishes = [
            finish(0, 5, false),
            finish(0, 20, true),
            finish(0, 9, false),
            finish(0, 5, false),
            finish(0, 1, true),
        ];
        assert_eq!(places(&finishes, false, None), [1, 3, 0, 1, 3]);
    }

    #[test]
    fn team_leavers_take_their_teams_result() {
        let finishes = [
            finish(0, 10, false),
            finish(0, 0, true),
            finish(1, 11, false),
            finish(1, 0, false),
        ];
        // Blue has more points.
        assert_eq!(places(&finishes, true, None), [1, 1, 0, 0]);
        // But the winning team comes first whatever the scores say.
        let p = places(&finishes, true, Some(0));
        assert_eq!(p, [0, 0, 1, 1]);
        let game: Vec<Placed> = finishes
            .iter()
            .zip(&p)
            .map(|(f, &place)| on_team(10, f.team, place))
            .collect();
        assert_eq!(xp_changes(&game, true), [100, 100, -40, -40]);
    }

    #[test]
    fn ranges_follow_bungies_maximum_match_table() {
        let table = [
            (1, 10),
            (2, 9),
            (3, 8),
            (4, 7),
            (5, 6),
            (6, 5),
            (7, 6),
            (16, 6),
            (17, 7),
            (26, 7),
            (27, 8),
            (29, 8),
            (30, 9),
            (31, 10),
            (34, 13),
            (35, 14),
            (36, 14),
            (40, 10),
            (49, 1),
            (50, 0),
        ];
        for (level, k) in table {
            assert_eq!(level_range(level), k, "level {level}");
        }
        // A pair is fine within either one's range.
        assert!(can_match(1, 11, 0));
        assert!(!can_match(1, 12, 0));
        assert!(can_match(12, 6, 0));
        assert!(!can_match(12, 5, 0));
        assert!(can_match(50, 36, 0));
        assert!(!can_match(50, 35, 0));
        // A long search widens it.
        assert!(can_match(1, 14, 3));
        assert!(!can_match(1, 15, 3));
    }

    #[test]
    fn low_party_members_count_as_the_lowest_level_the_leader_can_meet() {
        // Bungie's example: a level 12 can only meet 6 and up, so their level
        // 1 friend counts as 6.
        assert_eq!(effective_level(1, 12), 6);
        assert_eq!(effective_level(8, 12), 8);
        assert_eq!(effective_level(12, 12), 12);
        // A low highest level reaches everyone.
        assert_eq!(effective_level(1, 3), 1);
        assert_eq!(effective_level(5, 11), 5);
        assert_eq!(effective_level(4, 11), 5);
        assert_eq!(effective_level(1, 30), 21);
        // Above 35 the range shrinks again, down to none at 50.
        assert_eq!(effective_level(1, 40), 30);
        assert_eq!(effective_level(1, 50), 50);
        // The counted level sets the gap and the factors: a level 1 counted
        // as 6 losing to a 6 loses 15%.
        let game = [ffa(6, 0), ffa(effective_level(1, 12), 1)];
        assert_eq!(xp_changes(&game, false)[1], -15);
    }

    #[test]
    fn only_finished_ranked_games_with_opponents_and_agreed_results_count() {
        let game = [on_team(10, 0, 0), on_team(10, 1, 1)];
        assert!(counts(true, true, 1, 1, &game, true));
        assert!(!counts(false, true, 1, 1, &game, true));
        assert!(!counts(true, false, 1, 1, &game, true));
        // At least half the joined PCs that reported must agree.
        assert!(counts(true, true, 1, 2, &game, true));
        assert!(!counts(true, true, 1, 3, &game, true));
        assert!(counts(true, true, 0, 0, &game, true));
        // Teammates aren't opponents, and bots don't count.
        let teammates = [on_team(10, 0, 0), on_team(10, 0, 0), bot(1, 1)];
        assert!(!counts(true, true, 1, 1, &teammates, true));
        let alone = [ffa(10, 0), bot(0, 1)];
        assert!(!counts(true, true, 0, 0, &alone, false));
        assert!(counts(true, true, 0, 0, &[ffa(10, 0), ffa(10, 1)], false));
    }
}
