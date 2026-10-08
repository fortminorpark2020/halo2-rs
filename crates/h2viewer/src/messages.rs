//! What players are told during a game, in Halo 2's own words: the kill
//! feed, medals and sprees, the lead, the game type and team at the start,
//! the time left, people joining and quitting, the end of the game and
//! grenade pickups. The words are the maps' (shared.map's string lists
//! `multiplayer\in_game_multiplayer_messages`, `multiplayer\team_names` and
//! `ui\hud\hud_messages`), with the same English built in for a map
//! without them. Which players see which line follows each message's
//! audience in the maps' multiplayer globals (the killer, the victim, or
//! their teams). The menus' own words (the dialogs that ask before leaving
//! a game, the scoreboard's columns and the carnage report's) are
//! mainmenu.map's, in `MenuText`.

use blam_cache::{text, GroupTag, MapSet};
use h2sim::game::{Death, LeadChange, Medal};
use h2sim::{Game, GameType};
use std::collections::HashMap;
use std::path::Path;

/// The string lists the messages come from, and the list of team names.
const LISTS: [&str; 2] = [
    "multiplayer\\in_game_multiplayer_messages",
    "ui\\hud\\hud_messages",
];
const TEAM_NAMES: &str = "multiplayer\\team_names";

/// Halo 2's English for the messages used, as the maps have it (for a map
/// without its string lists).
const ENGLISH: &[(&str, &str)] = &[
    ("gen_kill_cause_player", "You killed #effect_player"),
    ("gen_kill_effect_player", "You were killed by #cause_player"),
    (
        "gen_kill_cause_team",
        "#effect_player killed by #cause_player",
    ),
    (
        "gen_kill_effect_team",
        "#cause_player killed #effect_player",
    ),
    (
        "gen_kill_melee_cause_player",
        "You beat down #effect_player",
    ),
    (
        "gen_kill_melee_effect_player",
        "You were beat down by #cause_player",
    ),
    (
        "gen_kill_melee_cause_team",
        "#cause_player beat down #effect_player",
    ),
    (
        "gen_kill_melee_effect_team",
        "#effect_player beat down by #cause_player",
    ),
    ("gen_kill_col_cause_player", "You splattered #effect_player"),
    (
        "gen_kill_col_effect_player",
        "You were splattered by #cause_player",
    ),
    (
        "gen_kill_col_cause_team",
        "#cause_player splattered #effect_player",
    ),
    (
        "gen_kill_col_effect_team",
        "#effect_player splattered by #cause_player",
    ),
    ("gen_suicide_cause_player", "You committed suicide"),
    ("gen_suicide_cause_team", "#cause_player committed suicide"),
    (
        "gen_kill_teammate_cause_team",
        "#cause_player betrayed #effect_player",
    ),
    (
        "gen_kill_teammate_cause_player",
        "You betrayed #effect_player",
    ),
    (
        "gen_kill_teammate_effect_player",
        "#cause_player betrayed you",
    ),
    (
        "gen_killed_by_unknown_effect_player",
        "Killed by the Guardians",
    ),
    (
        "gen_killed_by_falling_effect_player",
        "You fell to your death",
    ),
    ("gen_victory_all", "#cause_player wins!"),
    ("gen_team_victory_all", "#cause_team wins!"),
    ("gen_game_over_all", "Game over"),
    ("gen_gained_lead_cause_player", "You took the lead!"),
    (
        "gen_gained_team_lead_cause_team",
        "Your team took the lead!",
    ),
    ("gen_lost_lead_cause_player", "You lost the lead"),
    ("gen_lost_team_lead_cause_team", "Your team lost the lead"),
    ("gen_tied_leader_cause_player", "You are tied for the lead!"),
    (
        "gen_tied_team_leader_cause_team",
        "Your team is tied for the lead!",
    ),
    ("gen_30_minutes_left_all", "30 minutes remaining"),
    ("gen_15_minutes_left_all", "15 minutes remaining"),
    ("gen_5_minutes_left_all", "5 minutes remaining"),
    ("gen_1_minute_left_all", "1 minute remaining"),
    ("gen_30_seconds_left_all", "30 seconds remaining"),
    ("gen_10_seconds_left_all", "10 seconds remaining"),
    ("gen_1_min_to_win_all", "#cause_player 1 minute to win!"),
    ("gen_1_min_to_win_cause_player", "1 minute to win!"),
    ("gen_team_1_min_to_win_all", "#cause_team 1 minute to win!"),
    ("gen_team_1_min_to_win_cause_team", "1 minute to win!"),
    ("gen_30_secs_to_win_all", "#cause_player 30 seconds to win!"),
    ("gen_30_secs_to_win_cause_player", "30 seconds to win!"),
    (
        "gen_team_30_secs_to_win_all",
        "#cause_team 30 seconds to win!",
    ),
    ("gen_team_30_secs_to_win_cause_team", "30 seconds to win!"),
    ("gen_10_secs_to_win_all", "#cause_player 10 seconds to win!"),
    ("gen_10_secs_to_win_cause_player", "10 seconds to win!"),
    (
        "gen_team_10_secs_to_win_all",
        "#cause_team 10 seconds to win!",
    ),
    ("gen_team_10_secs_to_win_cause_team", "10 seconds to win!"),
    ("gen_player_quit_all", "#cause_player quit"),
    ("gen_player_joined_all", "#cause_player joined the game"),
    (
        "gen_start_team_notice_cause_player",
        "You are on the #cause_team",
    ),
    ("flavor_double_kill_cause_player", "Double kill!"),
    ("flavor_triple_kill_cause_player", "Triple kill!"),
    ("flavor_killtacular_cause_player", "Killtacular!"),
    ("flavor_killing_frenzy_cause_player", "Kill frenzy!"),
    ("flavor_killtrocity_cause_player", "Killtrocity!"),
    ("flavor_killimanjaro_cause_player", "Killimanjaro!"),
    ("flavor_killing_spree_cause_player", "Killing spree!"),
    ("flavor_running_riot_cause_player", "Running riot!"),
    ("flavor_15_kills_cause_player", "Rampage!"),
    ("flavor_20_kills_cause_player", "Berserker!"),
    ("flavor_25_kills_cause_player", "Overkill!"),
    ("medal_double_kill", "Double Kill"),
    ("medal_triple_kill", "Triple kill"),
    ("medal_killtacular", "Killtacular"),
    ("medal_kill_frenzy", "Kill Frenzy"),
    ("medal_killtrocity", "Killtrocity"),
    ("medal_killimanjaro", "Killimanjaro"),
    ("medal_killing_spree", "Killing Spree"),
    ("medal_running_riot", "Running Riot"),
    ("slayer_game_start_all", "Slayer"),
    ("ctf_game_start_all", "Capture the Flag"),
    ("oddball_game_start_all", "Oddball"),
    ("king_game_start_all", "King of the Hill"),
    ("jug_game_start_all", "Juggernaut"),
    ("ter_game_start_all", "Territories"),
    ("invasion_game_start_all", "Assault"),
    ("state_game_over_won", "You win!"),
    ("state_game_over_tied", "Tie game!"),
    ("state_game_over_lost", "You lose!"),
    ("fg_ammo_singular", "Picked up a frag grenade"),
    ("fg_ammo_plural", "Picked up \u{e422} frag grenades"),
    ("pg_ammo_singular", "Picked up a plasma grenade"),
    ("pg_ammo_plural", "Picked up \u{e422} plasma grenades"),
];
const ENGLISH_TEAMS: [&str; 2] = ["Red Team", "Blue Team"];

/// Where a count goes in a HUD message ("Picked up 2 plasma grenades").
const COUNT: char = '\u{e422}';

/// Halo 2's in-game messages, by name, and its teams' names.
#[derive(Debug, Default)]
pub struct GameText {
    lines: HashMap<String, String>,
    teams: Vec<String>,
}

/// Who a message is about: the player (or team) who caused it and the one
/// it happened to.
#[derive(Debug, Default)]
struct About<'a> {
    cause: Option<&'a str>,
    effect: Option<&'a str>,
    cause_team: Option<&'a str>,
}

impl GameText {
    /// The messages in a level's string lists (in shared.map for a
    /// multiplayer map); lines missing there keep their built-in English.
    pub fn load(set: &mut MapSet) -> GameText {
        let mut out = GameText::default();
        let Ok(table) = text::language_table(set) else {
            println!("warning: no language table: the game's messages are built in");
            return out;
        };
        let unic = GroupTag::parse("unic");
        let datum = |set: &MapSet, name: &str| {
            unic.and_then(|g| set.map.find_tag(g, name))
                .map(|t| t.datum)
        };
        for name in LISTS {
            let Some(d) = datum(set, name) else {
                println!("warning: no {name}: its messages are built in");
                continue;
            };
            for (id, line) in text::unicode_strings(set, &table, d).unwrap_or_default() {
                if let Some(key) = set.map.string_id(id) {
                    out.lines.insert(key.to_string(), line);
                }
            }
        }
        if let Some(d) = datum(set, TEAM_NAMES) {
            let names = text::unicode_strings(set, &table, d).unwrap_or_default();
            out.teams = names.into_iter().map(|(_, n)| n).collect();
        }
        out
    }

    /// How many of the messages came from the map.
    pub fn loaded(&self) -> usize {
        ENGLISH
            .iter()
            .filter(|(k, _)| self.lines.contains_key(*k))
            .count()
    }

    /// A message as the map words it, or its built-in English.
    fn line(&self, key: &str) -> &str {
        self.lines.get(key).map_or_else(
            || ENGLISH.iter().find(|e| e.0 == key).map_or("", |e| e.1),
            String::as_str,
        )
    }

    /// A team's name ("Red Team").
    pub fn team(&self, team: u8) -> String {
        let t = team as usize;
        let name = self
            .teams
            .get(t)
            .map_or(ENGLISH_TEAMS[t.min(1)], String::as_str);
        tidy(name)
    }

    /// Message `key` about these players or teams, ready for the HUD.
    fn say(&self, key: &str, about: &About) -> String {
        let mut line = self.line(key).to_string();
        for (token, name) in [
            ("#cause_player", about.cause),
            ("#effect_player", about.effect),
            ("#cause_team", about.cause_team),
        ] {
            if let Some(name) = name {
                line = line.replace(token, name);
            }
        }
        tidy(&line)
    }

    /// The kill feed line player `me` sees for a death: `killer` (none
    /// for the level) killed `victim`, `how`. None when Halo 2 tells
    /// them nothing: another team's suicide or betrayal.
    pub fn kill(
        &self,
        game: &Game,
        me: usize,
        killer: Option<usize>,
        victim: usize,
        how: Death,
    ) -> Option<String> {
        let teams = game.rules.game_type.teams();
        let team = |p: usize| game.players.get(p).map(|p| p.team);
        // No one in the game (`usize::MAX`, for the log) hears it as
        // everyone's teammate would.
        let outsider = me >= game.players.len();
        let teammate = |p: usize| outsider || teams && p != me && team(p) == team(me);
        let name = |p: usize| crate::local::player_name(game, usize::MAX, p);
        let (k, v) = (killer.map(name), name(victim));
        let about = About {
            cause: k.as_deref(),
            effect: Some(&v),
            cause_team: None,
        };
        let Some(k) = killer.filter(|&k| k != victim) else {
            // Killed themselves, or by the level.
            let mine = About {
                cause: Some(&v),
                ..About::default()
            };
            let key = match how {
                _ if me != victim => {
                    // Another team's suicide isn't news.
                    if teams && !teammate(victim) {
                        return None;
                    }
                    "gen_suicide_cause_team"
                }
                Death::Fall => "gen_killed_by_falling_effect_player",
                Death::Guardians => "gen_killed_by_unknown_effect_player",
                _ => "gen_suicide_cause_player",
            };
            return Some(self.say(key, &mine));
        };
        let key = match k {
            k if !game.is_enemy(k, victim) => {
                if me == k {
                    "gen_kill_teammate_cause_player"
                } else if me == victim {
                    "gen_kill_teammate_effect_player"
                } else if teammate(k) {
                    "gen_kill_teammate_cause_team"
                } else {
                    return None;
                }
            }
            k => {
                let kind = match how {
                    Death::Melee => "melee_",
                    Death::Splatter => "col_",
                    _ => "",
                };
                let whose = if me == k {
                    "cause_player"
                } else if me == victim {
                    "effect_player"
                } else if teammate(k) {
                    "cause_team"
                } else if teammate(victim) {
                    "effect_team"
                } else {
                    // Halo 2 sends kill messages only to the killer, the
                    // victim and their teams. Someone else in a free-for-
                    // all sees who killed whom, worded so.
                    return Some(match how {
                        Death::Melee | Death::Splatter => {
                            self.say(&format!("gen_kill_{kind}cause_team"), &about)
                        }
                        _ => self.say("gen_kill_effect_team", &about),
                    });
                };
                return Some(self.say(&format!("gen_kill_{kind}{whose}"), &about));
            }
        };
        Some(self.say(key, &about))
    }

    /// The line for a medal the player earned ("Double kill!").
    pub fn medal(&self, medal: Medal) -> String {
        const FLAVORS: [&str; Medal::KINDS] = [
            "flavor_double_kill_cause_player",
            "flavor_triple_kill_cause_player",
            "flavor_killtacular_cause_player",
            "flavor_killing_frenzy_cause_player",
            "flavor_killtrocity_cause_player",
            "flavor_killimanjaro_cause_player",
            "flavor_killing_spree_cause_player",
            "flavor_running_riot_cause_player",
            "flavor_15_kills_cause_player",
            "flavor_20_kills_cause_player",
            "flavor_25_kills_cause_player",
        ];
        self.say(FLAVORS[medal.index()], &About::default())
    }

    /// A medal's name, for the carnage report ("Double Kill"). The sprees
    /// past Running Riot have no medal name in the maps, so their line
    /// stands for it ("Rampage").
    pub fn medal_name(&self, medal: Medal) -> String {
        const NAMES: [&str; 8] = [
            "medal_double_kill",
            "medal_triple_kill",
            "medal_killtacular",
            "medal_kill_frenzy",
            "medal_killtrocity",
            "medal_killimanjaro",
            "medal_killing_spree",
            "medal_running_riot",
        ];
        match NAMES.get(medal.index()) {
            Some(key) => self.say(key, &About::default()),
            None => self.medal(medal).trim_end_matches('!').to_string(),
        }
    }

    /// The line when a player's (or in team games, their team's) hold on
    /// the lead changes.
    pub fn lead(&self, change: LeadChange, teams: bool) -> String {
        let key = match (change, teams) {
            (LeadChange::Gained, false) => "gen_gained_lead_cause_player",
            (LeadChange::Gained, true) => "gen_gained_team_lead_cause_team",
            (LeadChange::Lost, false) => "gen_lost_lead_cause_player",
            (LeadChange::Lost, true) => "gen_lost_team_lead_cause_team",
            (LeadChange::Tied, false) => "gen_tied_leader_cause_player",
            (LeadChange::Tied, true) => "gen_tied_team_leader_cause_team",
        };
        self.say(key, &About::default())
    }

    /// What everyone is told as play begins: the game type, and in team
    /// games which team they're on.
    pub fn start(&self, game: &Game, me: usize) -> Vec<String> {
        let key = match game.rules.game_type {
            GameType::Slayer | GameType::TeamSlayer => "slayer_game_start_all",
            GameType::Ctf => "ctf_game_start_all",
            GameType::KingOfTheHill | GameType::TeamKing => "king_game_start_all",
            GameType::Oddball | GameType::TeamOddball => "oddball_game_start_all",
            GameType::Juggernaut => "jug_game_start_all",
            GameType::Territories => "ter_game_start_all",
            GameType::Assault => "invasion_game_start_all",
            GameType::Campaign => return Vec::new(),
        };
        let mut lines = vec![self.say(key, &About::default())];
        if let Some(p) = game
            .players
            .get(me)
            .filter(|_| game.rules.game_type.teams())
        {
            let team = self.team(p.team);
            let about = About {
                cause_team: Some(&team),
                ..About::default()
            };
            lines.push(self.say("gen_start_team_notice_cause_player", &about));
        }
        lines
    }

    /// Someone joined the game.
    pub fn joined(&self, name: &str) -> String {
        self.say("gen_player_joined_all", &self.cause(name))
    }

    /// Someone left it.
    pub fn quit(&self, name: &str) -> String {
        self.say("gen_player_quit_all", &self.cause(name))
    }

    fn cause<'a>(&self, name: &'a str) -> About<'a> {
        About {
            cause: Some(name),
            ..About::default()
        }
    }

    /// Once the game is over, how it went for player `me`: "You win!",
    /// "You lose!" or "Tie game!" (tied at the top when time ran out).
    pub fn game_over(&self, game: &Game, me: usize) -> String {
        let key = match (game.winning_team, game.winner) {
            (Some(t), _) if game.players.get(me).is_some_and(|p| p.team == t) => {
                "state_game_over_won"
            }
            (None, Some(w)) if w == me => "state_game_over_won",
            (Some(_), _) | (None, Some(_)) => "state_game_over_lost",
            (None, None) => {
                let top = (0..game.players.len())
                    .map(|i| game.side_score(i))
                    .max()
                    .unwrap_or(0);
                let mine = (me < game.players.len()).then(|| game.side_score(me));
                match mine {
                    Some(s) if s == top => "state_game_over_tied",
                    _ => "state_game_over_lost",
                }
            }
        };
        self.say(key, &About::default())
    }

    /// Who won, for the kill feed ("Red Team wins!"), or "Game over" for
    /// a draw.
    pub fn winner(&self, game: &Game) -> String {
        match (game.winning_team, game.winner) {
            (Some(t), _) => {
                let team = self.team(t);
                let about = About {
                    cause_team: Some(&team),
                    ..About::default()
                };
                self.say("gen_team_victory_all", &about)
            }
            (None, Some(w)) => {
                let name = crate::local::player_name(game, usize::MAX, w);
                self.say("gen_victory_all", &self.cause(&name))
            }
            (None, None) => self.say("gen_game_over_all", &About::default()),
        }
    }

    /// The time left, as the announcer calls it at `TIME_WARNINGS`.
    pub fn time_left(&self, warning: usize) -> String {
        const KEYS: [&str; 6] = [
            "gen_30_minutes_left_all",
            "gen_15_minutes_left_all",
            "gen_5_minutes_left_all",
            "gen_1_minute_left_all",
            "gen_30_seconds_left_all",
            "gen_10_seconds_left_all",
        ];
        self.say(KEYS[warning.min(KEYS.len() - 1)], &About::default())
    }

    /// A side close to winning a timed game (`TO_WIN` warning `warning`):
    /// told to that player or team as "1 minute to win!", and to everyone
    /// else with their name.
    pub fn to_win(&self, warning: usize, teams: bool, theirs: Option<&str>) -> String {
        const KEYS: [&str; 3] = ["1_min", "30_secs", "10_secs"];
        let when = KEYS[warning.min(KEYS.len() - 1)];
        let key = match (teams, theirs) {
            (false, None) => format!("gen_{when}_to_win_cause_player"),
            (false, Some(_)) => format!("gen_{when}_to_win_all"),
            (true, None) => format!("gen_team_{when}_to_win_cause_team"),
            (true, Some(_)) => format!("gen_team_{when}_to_win_all"),
        };
        let about = match teams {
            true => About {
                cause_team: theirs,
                ..About::default()
            },
            false => About {
                cause: theirs,
                ..About::default()
            },
        };
        self.say(&key, &about)
    }

    /// Grenades picked up at once ("Picked up 2 plasma grenades").
    pub fn grenades(&self, plasma: bool, count: usize) -> String {
        let kind = if plasma { "pg" } else { "fg" };
        let number = if count == 1 { "singular" } else { "plural" };
        let line = self.line(&format!("{kind}_ammo_{number}"));
        tidy(&line.replace(COUNT, &count.to_string()))
    }
}

/// Seconds of play left at which the announcer calls the time: 30, 15, 5
/// and 1 minutes, 30 and 10 seconds (the maps' multiplayer globals).
pub const TIME_WARNINGS: [f64; 6] = [1800.0, 900.0, 300.0, 60.0, 30.0, 10.0];
/// Seconds short of the score to win at which a game whose points are
/// seconds warns that a side is about to win: 1 minute, 30 and 10 seconds.
pub const TO_WIN: [f64; 3] = [60.0, 30.0, 10.0];

/// The warnings whose moment passed between `before` and `after` (values
/// counting down, like the time left): each once, on the way past it, and
/// none for a moment already behind at the start.
pub fn passed(marks: &[f64], before: f64, after: f64) -> Vec<usize> {
    (0..marks.len())
        .filter(|&k| before > marks[k] && after <= marks[k])
        .collect()
}

/// A player in a free-for-all, or a team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Player(usize),
    Team(u8),
}

/// What a game warns of as it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warning {
    /// Time running out: `TIME_WARNINGS` warning `k`.
    TimeLeft(usize),
    /// A side `TO_WIN` warning `k` short of winning a game whose points
    /// are seconds (the hill, the ball, territories).
    ToWin(usize, Side),
}

/// Watches a game for its warnings, each once as its moment passes.
#[derive(Debug, Default)]
pub struct Warnings {
    /// The game's time, the time left and each side's score when last
    /// looked at (nothing before the first look).
    last: Option<(f64, Option<f64>, Vec<i32>)>,
}

impl Warnings {
    /// The warnings since the last look. A game started afresh (its clock
    /// gone back) starts the watch afresh.
    pub fn update(&mut self, game: &Game) -> Vec<Warning> {
        let sides = sides(game);
        let scores: Vec<i32> = sides.iter().map(|s| s.1).collect();
        let now = (game.time, game.time_left(), scores);
        let mut out = Vec::new();
        let last = self.last.replace(now).filter(|l| l.0 <= game.time);
        let Some((_, left, before)) = last.filter(|_| !game.over()) else {
            return out;
        };
        if let (Some(b), Some(a)) = (left, game.time_left()) {
            out.extend(
                passed(&TIME_WARNINGS, b, a)
                    .into_iter()
                    .map(Warning::TimeLeft),
            );
        }
        let target = game.rules.score_to_win as f64;
        if game.rules.game_type.timed() && target > 0.0 {
            for (k, &(side, score)) in sides.iter().enumerate() {
                let was = before.get(k).copied().unwrap_or(score);
                let (b, a) = (target - was as f64, target - score as f64);
                out.extend(
                    passed(&TO_WIN, b, a)
                        .into_iter()
                        .map(|w| Warning::ToWin(w, side)),
                );
            }
        }
        out
    }
}

/// The game's sides and their scores: its teams, or in a free-for-all its
/// players.
fn sides(game: &Game) -> Vec<(Side, i32)> {
    if game.rules.game_type.teams() {
        let mut teams: Vec<u8> = game.players.iter().map(|p| p.team).collect();
        teams.sort_unstable();
        teams.dedup();
        teams
            .into_iter()
            .map(|t| (Side::Team(t), game.team_score(t)))
            .collect()
    } else {
        (0..game.players.len())
            .map(|i| (Side::Player(i), game.players[i].score))
            .collect()
    }
}

/// The menus' string lists in mainmenu.map, and the names their lines go
/// by here ("errors_live/end_game").
const MENU_LISTS: [(&str, &str); 6] = [
    ("errors_other", "ui\\global_strings\\errors_other"),
    ("errors_live", "ui\\global_strings\\errors_live"),
    ("errors_networking", "ui\\global_strings\\errors_networking"),
    ("global_strings", "ui\\global_strings\\global_strings"),
    (
        "global_multiplayer_messages",
        "multiplayer\\global_multiplayer_messages",
    ),
    (
        "werds",
        "ui\\screens\\game_shell\\postgame_statistics\\werds",
    ),
];

/// Halo 2's English for the menus' lines used, as mainmenu.map has it.
const MENU_ENGLISH: &[(&str, &str)] = &[
    ("errors_other/exit_confirmation", "EXIT HALO 2 ?"),
    (
        "errors_other/error_confirm_boot_to_dash",
        "You're about to exit Halo 2. Are you sure you want to do this?",
    ),
    ("errors_other/exit_halo2", "Exit Halo 2"),
    ("errors_other/no", "No"),
    ("errors_live/are_you_sure", "ARE YOU SURE ?"),
    (
        "errors_live/error_confirm_end_game_session",
        "You are about to end this game. Are you sure you want to do this?",
    ),
    ("errors_live/end_game", "End Game"),
    ("errors_live/cancel", "Cancel"),
    ("errors_live/leave_game", "Leave Game ?"),
    (
        "errors_live/confirm_exit_game_session",
        "Are you sure you want to leave this game?",
    ),
    ("errors_live/yes_leave_game", "Leave Game"),
    ("errors_networking/are_you_sure", "ARE YOU SURE ?"),
    (
        "errors_networking/error_confirm_leave_system_link_lobby",
        "Are you sure you want to leave this game lobby?",
    ),
    ("errors_networking/leave_lobby", "Leave Lobby"),
    ("errors_networking/cancel", "Cancel"),
    ("global_strings/mp_score_place_header", "Place"),
    ("global_strings/mp_score_name_header", "Name"),
    ("global_strings/mp_score_kills_header", "Kills"),
    ("global_strings/mp_score_assists_header", "Assists"),
    ("global_strings/mp_score_deaths_header", "Deaths"),
    ("global_strings/mp_score_flags_header", "Flags"),
    ("global_strings/mp_time_header", "Time"),
    ("global_multiplayer_messages/mp_score", "Score"),
    ("global_multiplayer_messages/mp_game_over", "GAME OVER"),
    ("werds/postgame_header", "POSTGAME CARNAGE REPORT"),
    ("werds/team_stats", "TEAM STATS"),
    ("werds/player_stats", "PLAYER STATS"),
    ("werds/kill_stats", "KILLS"),
    ("werds/medal_stats", "MEDALS"),
    ("werds/team", "Team"),
    ("werds/player", "Player"),
    ("werds/place", "Place"),
    ("werds/score", "Score"),
    ("werds/avg_life", "Avg. Life"),
    ("werds/best_spree", "Best Spree"),
    ("werds/kills", "Kills"),
    ("werds/assists", "Assists"),
    ("werds/deaths", "Deaths"),
    ("werds/suicides", "Suicides"),
    ("werds/total_medals", "Total Medals"),
    ("werds/medals_earned", "Medals Earned"),
];

/// Halo 2's words for the menus over a game: the dialogs that ask before
/// quitting or leaving, the scoreboard's columns and the carnage report's
/// panes, from mainmenu.map's string lists, by list and name
/// ("werds/kills"). The same English is built in.
#[derive(Debug, Default)]
pub struct MenuText {
    lines: HashMap<String, String>,
}

impl MenuText {
    /// The words in the mainmenu.map in the maps folder `dir`.
    pub fn load(dir: &Path) -> MenuText {
        let mut out = MenuText::default();
        let Ok(mut set) = MapSet::open(dir.join("mainmenu.map")) else {
            return out;
        };
        let Ok(table) = text::language_table(&mut set) else {
            return out;
        };
        let unic = GroupTag::parse("unic");
        for (list, name) in MENU_LISTS {
            let tag = unic.and_then(|g| set.map.find_tag(g, name));
            let Some(datum) = tag.map(|t| t.datum) else {
                println!("warning: no {name} in mainmenu.map: its words are built in");
                continue;
            };
            for (id, line) in text::unicode_strings(&mut set, &table, datum).unwrap_or_default() {
                if let Some(key) = set.map.string_id(id) {
                    out.lines.insert(format!("{list}/{key}"), line);
                }
            }
        }
        out
    }

    /// Line `key` ("werds/kills") as the map words it, or its built-in
    /// English, in the menus' capitals.
    pub fn get(&self, key: &str) -> String {
        let english = || MENU_ENGLISH.iter().find(|e| e.0 == key).map_or("", |e| e.1);
        tidy(self.lines.get(key).map_or_else(english, String::as_str))
    }

    /// The scoreboard's column headings in a game type: place, name, its
    /// score (flags in Capture the Flag, time where points are seconds),
    /// kills, assists and deaths.
    pub fn score_headings(&self, game_type: GameType) -> [String; 6] {
        let score = match game_type {
            GameType::Ctf => "global_strings/mp_score_flags_header",
            t if t.timed() => "global_strings/mp_time_header",
            _ => "global_multiplayer_messages/mp_score",
        };
        [
            "global_strings/mp_score_place_header",
            "global_strings/mp_score_name_header",
            score,
            "global_strings/mp_score_kills_header",
            "global_strings/mp_score_assists_header",
            "global_strings/mp_score_deaths_header",
        ]
        .map(|k| self.get(k))
    }
}

/// A message as the HUD's font shows it: in capitals, without Halo 2's
/// button and medal glyphs (its fonts' private characters).
fn tidy(line: &str) -> String {
    line.chars()
        .filter(|c| !('\u{e000}'..='\u{f8ff}').contains(c))
        .collect::<String>()
        .trim()
        .to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(game_type: GameType) -> Game {
        let mut g = h2sim::testing::game();
        g.rules.game_type = game_type;
        for _ in 0..4 {
            g.add_player();
        }
        g.set_name(2, "Sarge");
        g
    }

    #[test]
    fn the_kill_feed_is_worded_for_each_player() {
        let t = GameText::default();
        let g = game(GameType::Slayer);
        let kill = |me, killer, victim, how| t.kill(&g, me, killer, victim, how);
        let w = Death::Weapon;
        assert_eq!(kill(0, Some(0), 1, w).unwrap(), "YOU KILLED PLAYER 2");
        assert_eq!(
            kill(0, Some(1), 0, w).unwrap(),
            "YOU WERE KILLED BY PLAYER 2"
        );
        // Someone else in a free-for-all.
        assert_eq!(kill(3, Some(1), 0, w).unwrap(), "PLAYER 2 KILLED PLAYER 1");
        assert_eq!(
            kill(0, Some(0), 2, Death::Melee).unwrap(),
            "YOU BEAT DOWN SARGE"
        );
        assert_eq!(
            kill(2, Some(0), 2, Death::Melee).unwrap(),
            "YOU WERE BEAT DOWN BY PLAYER 1"
        );
        assert_eq!(
            kill(1, Some(0), 2, Death::Splatter).unwrap(),
            "PLAYER 1 SPLATTERED SARGE"
        );
        assert_eq!(kill(0, Some(0), 0, w).unwrap(), "YOU COMMITTED SUICIDE");
        assert_eq!(kill(1, Some(2), 2, w).unwrap(), "SARGE COMMITTED SUICIDE");
        assert_eq!(
            kill(2, None, 2, Death::Fall).unwrap(),
            "YOU FELL TO YOUR DEATH"
        );
        assert_eq!(
            kill(2, None, 2, Death::Guardians).unwrap(),
            "KILLED BY THE GUARDIANS"
        );
        // Others are told a fall (or a pit) was a suicide, which it counts as.
        assert_eq!(
            kill(0, None, 2, Death::Fall).unwrap(),
            "SARGE COMMITTED SUICIDE"
        );
    }

    #[test]
    fn team_games_tell_each_team_its_own_way() {
        let t = GameText::default();
        let mut g = game(GameType::TeamSlayer);
        // 0 and 2 red, 1 and 3 blue.
        for (i, team) in [0, 1, 0, 1].into_iter().enumerate() {
            g.players[i].team = team;
        }
        let kill = |me, killer, victim, how| t.kill(&g, me, killer, victim, how);
        let w = Death::Weapon;
        // The killer's teammate, and the victim's.
        assert_eq!(
            kill(2, Some(0), 1, w).unwrap(),
            "PLAYER 2 KILLED BY PLAYER 1"
        );
        assert_eq!(kill(3, Some(0), 1, w).unwrap(), "PLAYER 1 KILLED PLAYER 2");
        assert_eq!(kill(0, Some(0), 2, w).unwrap(), "YOU BETRAYED SARGE");
        assert_eq!(kill(2, Some(0), 2, w).unwrap(), "PLAYER 1 BETRAYED YOU");
        // The other team hears nothing of a betrayal or a suicide.
        assert_eq!(kill(1, Some(0), 2, w), None);
        assert_eq!(kill(1, Some(2), 2, w), None);
        assert_eq!(kill(0, Some(2), 2, w).unwrap(), "SARGE COMMITTED SUICIDE");
        // The log hears everything.
        let log = usize::MAX;
        assert_eq!(kill(log, Some(0), 2, w).unwrap(), "PLAYER 1 BETRAYED SARGE");
        assert_eq!(kill(log, Some(2), 2, w).unwrap(), "SARGE COMMITTED SUICIDE");
        assert_eq!(
            kill(log, Some(0), 1, Death::Melee).unwrap(),
            "PLAYER 1 BEAT DOWN PLAYER 2"
        );
        assert_eq!(
            t.start(&g, 1),
            ["SLAYER".to_string(), "YOU ARE ON THE BLUE TEAM".to_string()]
        );
    }

    #[test]
    fn medals_the_lead_and_the_end_are_halo_2s_lines() {
        let t = GameText::default();
        assert_eq!(t.medal(Medal::MultiKill(2)), "DOUBLE KILL!");
        assert_eq!(t.medal(Medal::MultiKill(9)), "KILLIMANJARO!");
        assert_eq!(t.medal(Medal::Spree(15)), "RAMPAGE!");
        assert_eq!(t.medal_name(Medal::Spree(10)), "RUNNING RIOT");
        assert_eq!(t.medal_name(Medal::Spree(25)), "OVERKILL");
        assert_eq!(t.lead(LeadChange::Gained, false), "YOU TOOK THE LEAD!");
        assert_eq!(
            t.lead(LeadChange::Tied, true),
            "YOUR TEAM IS TIED FOR THE LEAD!"
        );
        let mut g = game(GameType::Slayer);
        assert_eq!(t.start(&g, 0), ["SLAYER"]);
        g.players[1].score = 3;
        g.players[2].score = 3;
        assert_eq!(t.game_over(&g, 1), "TIE GAME!");
        assert_eq!(t.game_over(&g, 0), "YOU LOSE!");
        assert_eq!(t.winner(&g), "GAME OVER");
        g.winner = Some(2);
        assert_eq!(t.game_over(&g, 2), "YOU WIN!");
        assert_eq!(t.game_over(&g, 1), "YOU LOSE!");
        assert_eq!(t.winner(&g), "SARGE WINS!");
        assert_eq!(t.joined("Sarge"), "SARGE JOINED THE GAME");
        assert_eq!(t.quit("Sarge"), "SARGE QUIT");
        assert_eq!(t.grenades(true, 1), "PICKED UP A PLASMA GRENADE");
        assert_eq!(t.grenades(false, 2), "PICKED UP 2 FRAG GRENADES");
        assert_eq!(t.time_left(3), "1 MINUTE REMAINING");
        assert_eq!(t.to_win(0, false, None), "1 MINUTE TO WIN!");
        assert_eq!(
            t.to_win(2, true, Some("RED TEAM")),
            "RED TEAM 10 SECONDS TO WIN!"
        );
    }

    #[test]
    fn the_maps_glyphs_are_left_out() {
        assert_eq!(tidy("\u{e046} You beat down X"), "YOU BEAT DOWN X");
    }

    #[test]
    fn warnings_come_once_on_the_way_past() {
        // A 10 minute game: nothing at the start.
        assert!(passed(&TIME_WARNINGS, 600.0, 599.9).is_empty());
        assert!(passed(&TIME_WARNINGS, 900.0, 899.9).is_empty());
        assert_eq!(passed(&TIME_WARNINGS, 300.01, 299.98), [2]);
        assert!(passed(&TIME_WARNINGS, 299.98, 299.95).is_empty());
        // A long frame past two at once.
        assert_eq!(passed(&TIME_WARNINGS, 31.0, 9.0), [4, 5]);
    }

    #[test]
    fn a_timed_game_warns_of_the_time_and_who_is_about_to_win() {
        let mut g = game(GameType::KingOfTheHill);
        g.rules.time_limit = 120;
        g.rules.score_to_win = 90;
        let mut w = Warnings::default();
        assert!(w.update(&g).is_empty());
        g.time = 59.5;
        assert!(w.update(&g).is_empty());
        g.time = 60.5;
        assert_eq!(w.update(&g), [Warning::TimeLeft(3)]);
        // Sarge on the hill: a minute short of winning, then 30 seconds.
        g.players[2].score = 31;
        assert_eq!(w.update(&g), [Warning::ToWin(0, Side::Player(2))]);
        assert!(w.update(&g).is_empty());
        g.players[2].score = 60;
        g.time = 90.0;
        assert_eq!(
            w.update(&g),
            [Warning::TimeLeft(4), Warning::ToWin(1, Side::Player(2))]
        );
        // The next game: nothing at its start.
        g.time = 0.0;
        g.players[2].score = 0;
        assert!(w.update(&g).is_empty());
        g.time = 1.0;
        assert!(w.update(&g).is_empty());
        // A game whose points aren't seconds has only the time.
        g.rules.game_type = GameType::Slayer;
        g.players[2].score = 89;
        assert!(w.update(&g).is_empty());
    }

    #[test]
    fn the_menus_words_are_built_in_too() {
        let t = MenuText::default();
        assert_eq!(t.get("errors_live/leave_game"), "LEAVE GAME ?");
        assert_eq!(t.get("werds/avg_life"), "AVG. LIFE");
        assert_eq!(t.get("no such line"), "");
        assert_eq!(t.score_headings(GameType::Ctf)[2], "FLAGS");
        assert_eq!(t.score_headings(GameType::Oddball)[2], "TIME");
        assert_eq!(
            t.score_headings(GameType::TeamSlayer),
            ["PLACE", "NAME", "SCORE", "KILLS", "ASSISTS", "DEATHS"]
        );
    }
}
