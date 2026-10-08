//! The player's profile: their gamertag, Spartan or Elite, armour colours
//! and look settings, and the online server they sign in to if not the
//! usual one. It's kept in a small text file so it lasts between games.

use crate::camera::Controls;
use h2sim::game::{clean_name, Look, EMBLEM_BACKGROUNDS, EMBLEM_FOREGROUNDS, PROFILE_COLORS};
use std::path::PathBuf;

/// Halo 2's 18 profile colours, as its profile lists them, with the RGB
/// from the game's globals (`h2tool colors` prints them).
pub const COLORS: [(&str, [f32; 3]); PROFILE_COLORS as usize] = [
    ("WHITE", [0.992, 0.996, 1.000]),
    ("STEEL", [0.325, 0.325, 0.325]),
    ("RED", [0.745, 0.173, 0.173]),
    ("ORANGE", [0.961, 0.478, 0.122]),
    ("GOLD", [0.961, 0.824, 0.173]),
    ("OLIVE", [0.624, 0.675, 0.349]),
    ("GREEN", [0.129, 0.573, 0.184]),
    ("SAGE", [0.137, 0.337, 0.267]),
    ("CYAN", [0.086, 0.627, 0.627]),
    ("TEAL", [0.212, 0.455, 0.478]),
    ("COBALT", [0.255, 0.424, 0.561]),
    ("BLUE", [0.157, 0.271, 0.608]),
    ("VIOLET", [0.416, 0.306, 0.714]),
    ("PURPLE", [0.459, 0.275, 0.427]),
    ("PINK", [0.984, 0.608, 0.788]),
    ("CRIMSON", [0.596, 0.071, 0.267]),
    ("BROWN", [0.400, 0.306, 0.243]),
    ("TAN", [0.694, 0.573, 0.337]),
];

/// A profile colour's name.
pub fn color_name(color: u8) -> &'static str {
    COLORS[color as usize % COLORS.len()].0
}

/// A profile colour's RGB (gamma space).
pub fn color(color: u8) -> [f32; 3] {
    COLORS[color as usize % COLORS.len()].1
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// Their gamertag.
    pub name: String,
    pub look: Look,
    /// Look sensitivity, and whether looking up and down is inverted.
    pub controls: Controls,
    /// The online server to sign in to, from a `server=` line players add
    /// themselves; empty for the usual one.
    pub server: String,
}

impl Default for Profile {
    fn default() -> Profile {
        Profile {
            name: h2net::player_name(),
            look: Look::default_for(0),
            controls: Controls::default(),
            server: String::new(),
        }
    }
}

impl Profile {
    /// The saved profile, or a new one named after the person signed in to
    /// this computer. `H2_NAME` overrides the name.
    pub fn load() -> Profile {
        let mut profile = path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map_or_else(Profile::default, |text| Profile::parse(&text));
        if let Some(name) = std::env::var("H2_NAME").ok().map(|n| clean_name(&n)) {
            if !name.is_empty() {
                profile.name = name;
            }
        }
        profile
    }

    /// Keep the profile for next time.
    pub fn save(&self) {
        let Some(path) = path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, self.to_text()) {
            println!(
                "warning: couldn't save the profile to {}: {e}",
                path.display()
            );
        }
    }

    fn parse(text: &str) -> Profile {
        let mut p = Profile::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            let sensitivity = || {
                value
                    .parse::<u8>()
                    .ok()
                    .filter(|n| (1..=Controls::MAX_SENSITIVITY).contains(n))
            };
            let color = || {
                COLORS
                    .iter()
                    .position(|c| c.0.eq_ignore_ascii_case(value))
                    .map(|i| i as u8)
                    .or_else(|| value.parse::<u8>().ok().filter(|&c| c < PROFILE_COLORS))
            };
            match key.trim() {
                "name" => {
                    let name = clean_name(value);
                    if !name.is_empty() {
                        p.name = name;
                    }
                }
                "model" => p.look.elite = value.eq_ignore_ascii_case("elite"),
                "server" => p.server = value.to_string(),
                "look_sensitivity" => {
                    let c = &mut p.controls.look_sensitivity;
                    *c = sensitivity().unwrap_or(*c);
                }
                "mouse_sensitivity" => {
                    let c = &mut p.controls.mouse_sensitivity;
                    *c = sensitivity().unwrap_or(*c);
                }
                "invert_look" => p.controls.invert_look = value.eq_ignore_ascii_case("yes"),
                "primary" => p.look.colors[0] = color().unwrap_or(p.look.colors[0]),
                "secondary" => p.look.colors[1] = color().unwrap_or(p.look.colors[1]),
                "emblem" => {
                    if let Some(n) = value.parse::<u8>().ok().filter(|&n| n < EMBLEM_FOREGROUNDS) {
                        p.look.emblem.foreground = n;
                    }
                }
                "emblem_background" => {
                    if let Some(n) = value.parse::<u8>().ok().filter(|&n| n < EMBLEM_BACKGROUNDS) {
                        p.look.emblem.background = n;
                    }
                }
                "emblem_primary" | "emblem_secondary" | "emblem_background_color" => {
                    let k = match key.trim() {
                        "emblem_primary" => 0,
                        "emblem_secondary" => 1,
                        _ => 2,
                    };
                    let c = &mut p.look.emblem.colors[k];
                    *c = color().unwrap_or(*c);
                }
                _ => {}
            }
        }
        p
    }

    fn to_text(&self) -> String {
        let color = |c: u8| color_name(c).to_lowercase();
        let e = &self.look.emblem;
        let mut text = format!(
            "name={}\nmodel={}\nprimary={}\nsecondary={}\n\
             emblem={}\nemblem_background={}\n\
             emblem_primary={}\nemblem_secondary={}\nemblem_background_color={}\n\
             look_sensitivity={}\nmouse_sensitivity={}\ninvert_look={}\n",
            self.name,
            if self.look.elite { "elite" } else { "spartan" },
            color(self.look.colors[0]),
            color(self.look.colors[1]),
            e.foreground,
            e.background,
            color(e.colors[0]),
            color(e.colors[1]),
            color(e.colors[2]),
            self.controls.look_sensitivity,
            self.controls.mouse_sensitivity,
            if self.controls.invert_look {
                "yes"
            } else {
                "no"
            },
        );
        if !self.server.is_empty() {
            text += &format!("server={}\n", self.server);
        }
        text
    }
}

/// A file kept beside the profile, like the key the PC signs in online
/// with.
pub fn beside(name: &str) -> Option<PathBuf> {
    Some(path()?.with_file_name(name))
}

/// Where the profile lives: `H2_PROFILE`, or `halo2-rs\profile.txt` in the
/// user's application data (on Windows `%APPDATA%`, elsewhere `~/.config`).
fn path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("H2_PROFILE") {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("halo2-rs").join("profile.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_read_back_what_was_saved() {
        let p = Profile {
            name: "MASTER CHIEF".into(),
            look: Look {
                elite: true,
                colors: [15, 4],
                emblem: h2sim::game::Emblem {
                    foreground: 63,
                    background: 31,
                    colors: [0, 17, 9],
                },
            },
            controls: Controls {
                look_sensitivity: 7,
                mouse_sensitivity: 1,
                invert_look: true,
            },
            server: "https://h2live.example.com".into(),
        };
        assert!(p.to_text().contains("primary=crimson"));
        assert!(p.to_text().contains("look_sensitivity=7\n"));
        assert!(p.to_text().contains("invert_look=yes\n"));
        assert_eq!(Profile::parse(&p.to_text()), p);
        // No server line for the usual server.
        let usual = Profile {
            server: String::new(),
            ..p
        };
        assert!(!usual.to_text().contains("server"));
        assert_eq!(Profile::parse(&usual.to_text()), usual);
        // Numbers work for colours too; nonsense is skipped.
        let q = Profile::parse("name=  arbiter \nprimary=3\nsecondary=plaid\nmodel=ELITE");
        assert_eq!(q.name, "ARBITER");
        assert!(q.look.elite);
        assert_eq!(q.look.colors, [3, Look::default_for(0).colors[1]]);
        // An older profile has Halo 2's look settings; ones out of range
        // are skipped.
        assert_eq!(q.controls, Controls::default());
        let r = Profile::parse("look_sensitivity=11\nmouse_sensitivity=0\ninvert_look=YES");
        assert_eq!(r.controls.look_sensitivity, 3);
        assert_eq!(r.controls.mouse_sensitivity, 3);
        assert!(r.controls.invert_look);
    }
}
