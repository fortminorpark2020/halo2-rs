//! The screens of mainmenu.map the lobby draws from their tags (menu.md
//! Phase 2): each one's whole tag name and the scale of each of its panes
//! (`layout::Scale`). Vista redid most screens in its own UI units; the
//! ones it never showed (most of the online screens and a few dialogs)
//! kept the Xbox's numbers.
//!
//! The small dialogs come in two families by their frame pieces' places
//! (ul_07, br_07, dialog1, dialog2). error_dialog_ok's family has them
//! where the globals' quarter header box (in Vista's units) sits inside
//! the frame's top at 1:1; task_progress_dialog's family has them about
//! 2.5 times closer to the middle, and its pieces only meet once drawn at
//! the Xbox's scale, with that header box (kept in Vista's units) in the
//! plate's top. Which scale each dialog takes was settled by drawing each
//! at both scales on the real map (`examples/menu_png.rs`, which also
//! prints the header box's fit; menu.md 9.2 has the results).
//!
//! `H2LOBBY_SCALE` overrides any entry for tuning on the owner's PC:
//! `<id>=xbox|vista` for all of a screen's panes, or `<id>:<pane>=...`
//! for one, several separated by commas (`lobby:0=xbox,dialog_ok=vista`).

use crate::layout::Scale;

/// Where every catalog screen's tag is.
const SCREENS: &str = "ui\\screens\\";

/// A screen the lobby draws from its tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ScreenId {
    DialogOk,
    OkCancel,
    LargeError,
    MultipleChoice,
    Confirm,
    Beeper,
    Online,
    PleaseWait,
    Playlists,
    PlaylistsLobby,
    Lobby,
    GameSetup,
    Privacy,
    Leader,
    Boot,
    SelectMap,
    SelectGame,
    SelectVariant,
    YMenu,
    PlayerOptions,
    Settings,
    Controller,
    Buttons,
    Sticks,
    RestoreControls,
    Keyboard,
    ProfilePicker,
}

use Scale::{Vista, Xbox};

impl ScreenId {
    pub const ALL: [ScreenId; 27] = [
        ScreenId::DialogOk,
        ScreenId::OkCancel,
        ScreenId::LargeError,
        ScreenId::MultipleChoice,
        ScreenId::Confirm,
        ScreenId::Beeper,
        ScreenId::Online,
        ScreenId::PleaseWait,
        ScreenId::Playlists,
        ScreenId::PlaylistsLobby,
        ScreenId::Lobby,
        ScreenId::GameSetup,
        ScreenId::Privacy,
        ScreenId::Leader,
        ScreenId::Boot,
        ScreenId::SelectMap,
        ScreenId::SelectGame,
        ScreenId::SelectVariant,
        ScreenId::YMenu,
        ScreenId::PlayerOptions,
        ScreenId::Settings,
        ScreenId::Controller,
        ScreenId::Buttons,
        ScreenId::Sticks,
        ScreenId::RestoreControls,
        ScreenId::Keyboard,
        ScreenId::ProfilePicker,
    ];

    /// Its tag's name under `ui\screens\`, its own short name (for
    /// `H2LOBBY_SCALE`, menu_png and the log), and its panes' scales (the
    /// last one goes for any pane after them).
    fn entry(self) -> (&'static str, &'static str, &'static [Scale]) {
        match self {
            // error_dialog_ok's family: the quarter header box fits its
            // frame at 1:1 (menu.md 9.2).
            ScreenId::DialogOk => ("misc\\error_dialog_ok", "dialog_ok", &[Vista]),
            ScreenId::OkCancel => ("misc\\error_dialog_ok_cancel", "ok_cancel", &[Vista]),
            ScreenId::LargeError => ("misc\\error_dialog_large", "large_error", &[Vista]),
            ScreenId::MultipleChoice => {
                ("misc\\multiple_choice_dialog", "multiple_choice", &[Vista])
            }
            ScreenId::Confirm => ("misc\\confirmation_dialog", "confirm", &[Vista]),
            ScreenId::Beeper => (
                "game_shell\\xbox_live\\xbox_live_notification_beeper\\online_notification_beeper",
                "beeper",
                &[Xbox],
            ),
            ScreenId::Online => (
                "game_shell\\xbox_live\\xbox_live_main_menu\\xbox_live_main_menu",
                "online",
                &[Xbox],
            ),
            // task_progress_dialog's family: the header box fits its
            // frame only at the Xbox's scale (menu.md 9.2).
            ScreenId::PleaseWait => (
                "game_shell\\xbox_live\\task_progress_dialog\\task_progress_dialog",
                "please_wait",
                &[Xbox],
            ),
            ScreenId::Playlists => (
                "game_shell\\xbox_live\\matchmaking\\optimatch_listing",
                "playlists",
                &[Xbox],
            ),
            ScreenId::PlaylistsLobby => (
                "game_shell\\xbox_live\\matchmaking\\optimatch_listing_lobby",
                "playlists_lobby",
                &[Xbox],
            ),
            // Pane 0 is Vista's own (Cartographer's lobby), panes 1 and 2
            // the Xbox's.
            ScreenId::Lobby => (
                "game_shell\\pregame_lobby\\pregame_lobby",
                "lobby",
                &[Vista, Xbox, Xbox],
            ),
            ScreenId::GameSetup => (
                "game_shell\\pregame_lobby\\squad_settings_dialog",
                "game_setup",
                &[Vista],
            ),
            ScreenId::Privacy => (
                "game_shell\\pregame_lobby\\squad_privacy_settings",
                "privacy",
                &[Vista],
            ),
            ScreenId::Leader => (
                "game_shell\\pregame_lobby\\squad_leader_select",
                "leader",
                &[Vista],
            ),
            ScreenId::Boot => ("game_shell\\pregame_lobby\\boot_player", "boot", &[Xbox]),
            ScreenId::SelectMap => (
                "game_shell\\mp_map_select\\mp_map_select_lobby",
                "select_map",
                &[Vista],
            ),
            ScreenId::SelectGame => (
                "game_shell\\mp_game_select\\game_engine_category_listing_lobby",
                "select_game",
                &[Vista],
            ),
            ScreenId::SelectVariant => (
                "game_shell\\mp_variant_select\\mp_variant_select_lobby",
                "select_variant",
                &[Vista],
            ),
            ScreenId::YMenu => (
                "game_shell\\online_y_menu\\online_y_menu",
                "y_menu",
                &[Xbox],
            ),
            ScreenId::PlayerOptions => (
                "game_shell\\online_y_menu\\player_selected_dialog",
                "player_options",
                &[Xbox],
            ),
            ScreenId::Settings => (
                "game_shell\\settings_screen\\settings",
                "settings",
                &[Vista],
            ),
            ScreenId::Controller => (
                "game_shell\\settings_screen\\player_profile\\controller_settings",
                "controller",
                &[Vista],
            ),
            ScreenId::Buttons => (
                "game_shell\\settings_screen\\player_profile\\button_settings",
                "buttons",
                &[Vista],
            ),
            ScreenId::Sticks => (
                "game_shell\\settings_screen\\player_profile\\thumbstick_settings",
                "sticks",
                &[Vista],
            ),
            ScreenId::RestoreControls => (
                "game_shell\\settings_screen\\player_profile\\restore_default_controller_settings",
                "restore_controls",
                &[Vista],
            ),
            ScreenId::Keyboard => (
                "game_shell\\virtual_keyboard\\virtual_keyboard",
                "keyboard",
                &[Vista],
            ),
            ScreenId::ProfilePicker => (
                "game_shell\\profile_picker\\profile_picker",
                "profile_picker",
                &[Vista],
            ),
        }
    }

    /// Its tag's whole name.
    pub fn path(self) -> String {
        format!("{SCREENS}{}", self.entry().0)
    }

    /// Its short name (`dialog_ok`).
    pub fn name(self) -> &'static str {
        self.entry().1
    }

    /// A screen by its short name or its tag's whole name (any case).
    pub fn find(s: &str) -> Option<ScreenId> {
        let s = s.trim();
        ScreenId::ALL
            .into_iter()
            .find(|id| id.name().eq_ignore_ascii_case(s) || id.path().eq_ignore_ascii_case(s))
    }

    /// Pane `k`'s scale in the catalog.
    pub fn scale(self, pane: usize) -> Scale {
        let scales = self.entry().2;
        scales
            .get(pane)
            .or(scales.last())
            .copied()
            .unwrap_or_default()
    }

    /// Its first `panes` panes' scales, with `overrides` over them.
    pub fn scales(self, panes: usize, overrides: &[Override]) -> Vec<Scale> {
        (0..panes)
            .map(|k| {
                overrides
                    .iter()
                    .rev()
                    .find(|o| o.id == self && o.pane.is_none_or(|p| p == k))
                    .map_or(self.scale(k), |o| o.scale)
            })
            .collect()
    }
}

/// One entry of `H2LOBBY_SCALE`: a screen, one pane of it or all, and the
/// scale it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Override {
    pub id: ScreenId,
    pub pane: Option<usize>,
    pub scale: Scale,
}

/// `H2LOBBY_SCALE`'s entries (`lobby:0=xbox,dialog_ok=vista`), and the
/// entries it couldn't read, for the log.
pub fn overrides(s: &str) -> (Vec<Override>, Vec<String>) {
    let (mut good, mut bad) = (Vec::new(), Vec::new());
    for entry in s.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let read = entry.split_once('=').and_then(|(who, scale)| {
            let scale = Scale::parse(scale)?;
            let (name, pane) = match who.rsplit_once(':') {
                Some((name, pane)) => (name, Some(pane.trim().parse().ok()?)),
                None => (who, None),
            };
            Some(Override {
                id: ScreenId::find(name)?,
                pane,
                scale,
            })
        });
        match read {
            Some(o) => good.push(o),
            None => bad.push(entry.to_string()),
        }
    }
    (good, bad)
}

/// The overrides in the `H2LOBBY_SCALE` environment variable.
pub fn env_overrides() -> (Vec<Override>, Vec<String>) {
    overrides(&std::env::var("H2LOBBY_SCALE").unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_screen_has_a_whole_path_and_a_name_of_its_own() {
        let mut names = std::collections::BTreeSet::new();
        for id in ScreenId::ALL {
            assert!(id.path().starts_with("ui\\screens\\"), "{id:?}");
            assert!(names.insert(id.name()), "{id:?}");
            assert_eq!(ScreenId::find(id.name()), Some(id));
            assert_eq!(ScreenId::find(&id.path().to_uppercase()), Some(id));
        }
        assert_eq!(ScreenId::find("nope"), None);
        // Two tags end in player_selected_dialog: the Y menu's is ours.
        assert!(ScreenId::PlayerOptions.path().contains("online_y_menu"));
    }

    #[test]
    fn panes_take_their_entrys_scale_and_the_last_one_after() {
        let lobby = ScreenId::Lobby;
        assert_eq!(
            lobby.scales(4, &[]),
            [Scale::Vista, Scale::Xbox, Scale::Xbox, Scale::Xbox]
        );
        assert_eq!(ScreenId::Online.scale(0), Scale::Xbox);
        assert_eq!(ScreenId::Settings.scale(3), Scale::Vista);
    }

    #[test]
    fn the_environment_overrides_a_screen_or_one_pane() {
        let (o, bad) = overrides(" lobby:0=xbox, dialog_ok=XBOX,online=vista,lobby=up,x=1,");
        assert_eq!(bad, ["lobby=up", "x=1"]);
        assert_eq!(o.len(), 3);
        assert_eq!(
            ScreenId::Lobby.scales(3, &o),
            [Scale::Xbox, Scale::Xbox, Scale::Xbox]
        );
        assert_eq!(ScreenId::DialogOk.scales(1, &o), [Scale::Xbox]);
        assert_eq!(ScreenId::Online.scales(2, &o), [Scale::Vista; 2]);
        // The last entry for a pane wins.
        let (o, _) = overrides("lobby=xbox,lobby:2=vista");
        assert_eq!(
            ScreenId::Lobby.scales(3, &o),
            [Scale::Xbox, Scale::Xbox, Scale::Vista]
        );
        assert_eq!(overrides("").0, []);
    }
}
