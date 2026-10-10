//! What the lobby keeps between runs (`lobby.txt` in the launcher's
//! folder): the server's address, the gamertag and the controls (the
//! Settings screen's; see `crate::controls`). A file from before the
//! controls were added reads with Halo 2's default controls.
//!
//! ```text
//! server = 192.168.8.102
//! gamertag = JOHN
//! button_layout = bumper_jumper
//! thumbstick_layout = default
//! look_sensitivity = 3
//! look_inversion = off
//! auto_look_centering = off
//! vibration = on
//! mouse_sensitivity = 1.6
//! mouse_inversion = off
//! ```

use crate::controls::Controls;
use std::path::{Path, PathBuf};

/// The owner's matchmaking server on his home network, used until another
/// is typed in.
pub const DEFAULT_SERVER: &str = "192.168.8.102";

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub server: String,
    pub gamertag: String,
    pub controls: Controls,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            server: DEFAULT_SERVER.into(),
            gamertag: String::new(),
            controls: Controls::default(),
        }
    }
}

impl Settings {
    /// Reads the file's text; unknown lines (and controls with a bad
    /// value, which keep their default) are skipped, so a newer launcher's
    /// file still works.
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            match k.trim().to_ascii_lowercase().as_str() {
                "server" if !v.is_empty() => s.server = v.to_string(),
                "gamertag" => s.gamertag = clean_gamertag(v),
                k => {
                    let _ = s.controls.set(k, v);
                }
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        format!(
            "server = {}\ngamertag = {}\n{}",
            self.server,
            self.gamertag,
            self.controls.lines()
        )
    }

    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path)
            .map(|t| Settings::parse(&t))
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        std::fs::write(path, self.to_text()).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The most characters in a gamertag, as on Xbox Live.
pub const GAMERTAG_LEN: usize = 15;

/// A gamertag as the server keeps it: letters, digits and single spaces,
/// upper case, 15 characters at most.
pub fn clean_gamertag(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if out.chars().count() == GAMERTAG_LEN {
            break;
        }
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else if c == ' ' && !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim_end().to_string()
}

/// The launcher's folder: `%LOCALAPPDATA%\h2launch` on Windows (where its
/// logs and its sign-in key for `--live` already are), else
/// `~/.local/share/h2launch`, with `<instance>` under it for `--instance`.
pub fn folder(instance: Option<&str>) -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("h2launch");
    match instance {
        Some(i) => dir.join(i),
        None => dir,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_read_back() {
        let s = Settings {
            server: "example.org:9000".into(),
            gamertag: "MASTER CHIEF".into(),
            controls: Controls {
                buttons: crate::controls::ButtonLayout::BumperJumper,
                sticks: crate::controls::StickLayout::Legacy,
                look_sensitivity: 5,
                look_inverted: true,
                auto_center: true,
                vibration: false,
                mouse_sensitivity: 3.1,
                mouse_inverted: true,
            },
        };
        assert_eq!(Settings::parse(&s.to_text()), s);
        assert_eq!(Settings::parse(""), Settings::default());
        let odd = Settings::parse(
            "# hi\nserver =\ncolour = red\ngamertag = j0hn!!\nlook_sensitivity = 99\nbutton_layout = recon\n",
        );
        assert_eq!(odd.server, DEFAULT_SERVER);
        assert_eq!(odd.gamertag, "J0HN");
        // A bad value keeps the default; a good one is read.
        assert_eq!(odd.controls.look_sensitivity, 3);
        assert_eq!(odd.controls.buttons, crate::controls::ButtonLayout::Recon);
    }

    #[test]
    fn a_file_from_before_the_controls_reads_with_the_defaults() {
        let old = Settings::parse("server = 192.168.8.102\ngamertag = JOHN\n");
        assert_eq!(old.gamertag, "JOHN");
        assert_eq!(old.controls, Controls::default());
        // And writing it back keeps the server and the gamertag first.
        assert!(old
            .to_text()
            .starts_with("server = 192.168.8.102\ngamertag = JOHN\n"));
    }

    #[test]
    fn gamertags_are_cleaned() {
        assert_eq!(clean_gamertag("  master   chief "), "MASTER CHIEF");
        assert_eq!(clean_gamertag("abcdefghijklmnopqrst"), "ABCDEFGHIJKLMNO");
        assert_eq!(clean_gamertag("é#$"), "");
    }
}
