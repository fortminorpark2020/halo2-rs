//! Names of the host slots (`i_game_manager`, 121 known) and the event
//! manager's slots (`i_game_event_manager`, 151), for readable logs, plus
//! the formatting of the call-count summaries and the final RESULT line.

use std::fmt::Write as _;

/// Slots in the host vtable we hand out. MCC's real table is longer than
/// the 121 known ones; the rest log and return 0.
pub const HOST_SLOTS: usize = 256;
/// Known host slots (libmcc's `i_game_manager`).
pub const HOST_KNOWN: usize = 121;
pub const EVENT_SLOTS: usize = 151;

pub fn host_slot_name(i: usize) -> &'static str {
    match i {
        0 => "begin_frame",
        1 => "end_frame",
        2 => "resize",
        3 => "set_game_state",
        4 => "restart_game",
        5 => "save_game",
        6 => "set_game_result",
        7 => "pause_game",
        8 => "pause_game_2",
        9 => "set_game_objectives",
        10 => "get_game_event_manager",
        11 => "set_game_engine_variant",
        12 => "set_scenario_map_variant",
        14 => "set_player_look_control",
        15 => "set_player_profile_game_specific",
        16 => "get_map_info",
        17 => "get_campaign_map_info",
        18 => "get_multiplayer_map_info",
        23 => "update_launch_timer",
        32 => "get_video_setting",
        33 => "get_audio_setting",
        34 => "get_player_profile",
        36 => "get_input_state",
        37 => "get_input_state_gamepad",
        38 => "update_input_time",
        39 => "set_input_state(rumble)",
        41 => "network_sendto_unreliable",
        42 => "network_sendto_reliable",
        43 => "network_recvfrom",
        44 => "network_send",
        46 => "get_folder_path",
        47 => "get_game_folder_path",
        48 => "get_scenario_path_a",
        49 => "get_scenario_path_w",
        50 => "get_ugc_id",
        51 => "get_game_setting",
        52 => "validate_cache_file",
        54 => "get_mcc_string",
        55 => "use_custom_string_mapping",
        56 => "insert_string",
        57 => "get_string",
        58 => "get_subtitle",
        59 => "font_probe_1",
        60 => "font_probe_2",
        61 => "font_test_string",
        62 => "font_precache_character",
        63 => "font_get_texture",
        64 => "font_test_char",
        65 => "font_get_kerning_pair_offset",
        66 => "font_set_glyph",
        67 => "font_set_selection",
        68 => "font_probe_3",
        69 => "unknown_f32",
        71 => "get_player_skin",
        72 => "draw_player_emblem",
        73 => "get_player_emblem",
        74 => "get_player_emblem_attribute",
        75 => "get_player_weapon_offset",
        88 => "get_player_xuid",
        95 => "chud_query_1",
        96 => "chud_query_2",
        97 => "chud_blend_color",
        116 => "get_player_gamepad_mapping",
        i if i < HOST_KNOWN => "unknown",
        _ => "beyond_libmcc",
    }
}

const EVENT_NAMES: [&str; 128] = [
    "AchievementEarned",
    "AshesToAshes",
    "Assist",
    "AudioLogClaimed",
    "Base(20 args)",
    "Base(2 args)",
    "BIFactControllerSettings",
    "BIFactDeepLink",
    "BIFactDeepLinkRecieve",
    "BIFactDeepLinkSend",
    "BIFactDualWield",
    "BIFactGameSession",
    "BIFactLoadout",
    "BIFactMatchmaking",
    "BIFactMatchmakingDetails",
    "BIFactMedia",
    "BirdOfPrey",
    "BitsAndPiecesDestroyed",
    "BroadcastingAssist",
    "BroadcastingDeath",
    "BroadcastingHeartbeat",
    "BroadcastingKill",
    "BroadcastingMatchEnd",
    "BroadcastingMatchRoundEnd",
    "BroadcastingMatchRoundStart",
    "BroadcastingMatchStart",
    "BroadcastingMedal",
    "BroadcastingPlayerJoined",
    "BroadcastingPlayerLeft",
    "BroadcastingPlayerSpawn",
    "BroadcastingPlayerSwitchedTeams",
    "BroadcastingScore",
    "BroadcastingStart",
    "CampaignDifficulty",
    "ChallengeCompleted",
    "ClassicModeSwitched",
    "CleverGirl",
    "ClueClaimed",
    "CompletionCount",
    "CoopMissionCompleted",
    "CoopSpartanOpsMissionCompleted",
    "Customization",
    "DashboardContext",
    "Death",
    "DollFound",
    "EliteWin",
    "Emblem",
    "EnemyDefeated",
    "FriendsBestedOnHeroLeaderboard",
    "GameProgress",
    "GameVarSaved",
    "GrenadeStick",
    "HelloNurse",
    "InGamePresence",
    "ISeeYou",
    "Joinability",
    "Lobby",
    "MainMenuPresence",
    "MapVarSaved",
    "MatchmakingHopper",
    "MediaUsage",
    "MeldOfferPresented",
    "MeldOfferResponded",
    "MeldPageAction",
    "MeldPageView",
    "MissionCompleted",
    "MortardomWraithsKilled",
    "MultiplayerGameEngine",
    "MultiplayerMap",
    "MultiplayerRoundEnd",
    "MultiplayerRoundStart",
    "NappersCaught",
    "NewsStoryRead",
    "ObjectiveEnd",
    "ObjectiveStart",
    "PageAction",
    "PageView",
    "PhantomHunter",
    "PigsCanFly",
    "PlayerCheckedInToday",
    "PlayerDefeated",
    "PlayerGameResults",
    "PlayerGameResultsDamageStat",
    "PlayerGameResultsGriefingStat",
    "PlayerGameResultsGriefingStats",
    "PlayerGameResultsInterestStats",
    "PlayerGameResultsMedal",
    "PlayerSessionEnd",
    "PlayerSessionPause",
    "PlayerSessionResume",
    "PlayerSessionStart",
    "PlayerSpawned",
    "PlaylistCompleted",
    "PlaylistProgress",
    "RankedStatsDNFInfo",
    "RankedStatsOverride",
    "RankedStatsPenalty",
    "RankedStatsUpdate",
    "RankedUpSpartanIv",
    "RealtimeFlagCaptured",
    "RealtimeMedal",
    "RealtimePilotedVehicle",
    "RivalID",
    "SectionEnd",
    "SectionStart",
    "SectionStats",
    "SessionSizeUpdate",
    "SizeIsEverything",
    "SkeetShooter",
    "SkullClaimed",
    "SoloMissionCompleted",
    "SoloSpartanOpsMissionCompleted",
    "SpartanOpsMissionCompleted",
    "Supercombine",
    "SurvivalSpace",
    "TerminalFound",
    "TerminalId",
    "TicketsEarned",
    "TitleCompleted",
    "TitleLaunched",
    "ValhallaSign",
    "ViewOffer",
    "VIPStatusEarned",
    "WhatAboutTanksDestroyed",
    "WonWarGame",
    "ZanzibarSign",
    "FirefightGameResults",
    "ScavengerHuntObjectFound",
];

/// The event manager slot that must hand back a valid GUID pointer.
pub const EVENT_GET_GUID: usize = 147;

pub fn event_name(i: usize) -> &'static str {
    match i {
        i if i < EVENT_NAMES.len() => EVENT_NAMES[i],
        128..=137 => "unnamed_void",
        138 => "unnamed_i32",
        139 => "SetBoolTrue",
        140 => "SetBoolFalse",
        141 => "GetBool",
        142 => "SetBool",
        143 => "MetaGameUpdateGameTicks",
        144 => "GameResultUpdatePlayerTime",
        145 => "unnamed_bool_arg",
        146 => "MetaGameGetFlags",
        147 => "GetGUID",
        148 => "unnamed_bool_out",
        149 | 150 => "unnamed_void",
        _ => "out_of_range",
    }
}

/// One slot's calls: in total and since the last summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotCount {
    pub index: usize,
    pub total: u64,
    pub delta: u64,
}

/// Summary lines: `<label>: 0 begin_frame=1200(+600) 1 end_frame=...`,
/// `per_line` slots a line, only slots that were called.
pub fn summary_lines(
    label: &str,
    counts: &[SlotCount],
    name: fn(usize) -> &'static str,
    per_line: usize,
) -> Vec<String> {
    let called: Vec<&SlotCount> = counts.iter().filter(|c| c.total > 0).collect();
    if called.is_empty() {
        return vec![format!("{label}: no calls")];
    }
    called
        .chunks(per_line.max(1))
        .map(|chunk| {
            let mut s = format!("{label}:");
            for c in chunk {
                let _ = write!(s, " {} {}={}", c.index, name(c.index), c.total);
                if c.delta != c.total {
                    let _ = write!(s, "(+{})", c.delta);
                }
            }
            s
        })
        .collect()
}

/// How the run ended.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub in_game: bool,
    pub frames: u64,
    pub crashed: Option<u32>,
    pub presents: Option<u32>,
    pub states: Vec<i32>,
    pub game_thread_exited: Option<bool>,
    pub reason: String,
}

impl Outcome {
    /// The last line of the log:
    /// `RESULT: in-game=<yes/no> frames=<n> crashed=<code or none>`, then
    /// more fields.
    pub fn line(&self) -> String {
        let mut s = format!(
            "RESULT: in-game={} frames={} crashed={}",
            if self.in_game { "yes" } else { "no" },
            self.frames,
            match self.crashed {
                Some(c) => format!("{c:#010x}"),
                None => "none".into(),
            }
        );
        if let Some(p) = self.presents {
            let _ = write!(s, " presents={p}");
        }
        let states: Vec<String> = self.states.iter().map(|v| v.to_string()).collect();
        let _ = write!(
            s,
            " states=[{}] game-thread-exited={} reason={:?}",
            states.join(","),
            match self.game_thread_exited {
                Some(true) => "yes",
                Some(false) => "no",
                None => "n/a",
            },
            self.reason
        );
        s
    }

    /// 0 when the launch reached a running game and nothing crashed;
    /// 2 after a crash; 1 otherwise.
    pub fn exit_code(&self) -> i32 {
        match (self.crashed, self.in_game) {
            (Some(_), _) => 2,
            (None, true) => 0,
            (None, false) => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(host_slot_name(0), "begin_frame");
        assert_eq!(host_slot_name(10), "get_game_event_manager");
        assert_eq!(host_slot_name(34), "get_player_profile");
        assert_eq!(host_slot_name(36), "get_input_state");
        assert_eq!(host_slot_name(51), "get_game_setting");
        assert_eq!(host_slot_name(88), "get_player_xuid");
        assert_eq!(host_slot_name(97), "chud_blend_color");
        assert_eq!(host_slot_name(116), "get_player_gamepad_mapping");
        assert_eq!(host_slot_name(120), "unknown");
        assert_eq!(host_slot_name(121), "beyond_libmcc");
        assert_eq!(event_name(0), "AchievementEarned");
        assert_eq!(event_name(81), "PlayerGameResults");
        assert_eq!(event_name(127), "ScavengerHuntObjectFound");
        assert_eq!(event_name(143), "MetaGameUpdateGameTicks");
        assert_eq!(event_name(EVENT_GET_GUID), "GetGUID");
        assert_eq!(event_name(150), "unnamed_void");
        assert_eq!(event_name(151), "out_of_range");
    }

    #[test]
    fn summaries() {
        let counts = [
            SlotCount {
                index: 0,
                total: 1200,
                delta: 600,
            },
            SlotCount {
                index: 5,
                total: 0,
                delta: 0,
            },
            SlotCount {
                index: 36,
                total: 3,
                delta: 3,
            },
        ];
        let lines = summary_lines("host", &counts, host_slot_name, 8);
        assert_eq!(
            lines,
            vec!["host: 0 begin_frame=1200(+600) 36 get_input_state=3".to_string()]
        );
        let lines = summary_lines("host", &counts, host_slot_name, 1);
        assert_eq!(lines.len(), 2);
        assert_eq!(
            summary_lines("events", &[], event_name, 8),
            vec!["events: no calls"]
        );
    }

    #[test]
    fn result_line() {
        let o = Outcome {
            in_game: true,
            frames: 7200,
            crashed: None,
            presents: Some(7199),
            states: vec![1, 5],
            game_thread_exited: Some(true),
            reason: "quit-after".into(),
        };
        assert_eq!(
            o.line(),
            "RESULT: in-game=yes frames=7200 crashed=none presents=7199 states=[1,5] game-thread-exited=yes reason=\"quit-after\""
        );
        assert_eq!(o.exit_code(), 0);
        let c = Outcome {
            crashed: Some(0xC000_0005),
            ..Default::default()
        };
        assert!(c
            .line()
            .starts_with("RESULT: in-game=no frames=0 crashed=0xc0000005"));
        assert_eq!(c.exit_code(), 2);
        assert_eq!(Outcome::default().exit_code(), 1);
    }
}
