//! The player's profile: their gamertag, Spartan or Elite, armour colours,
//! look settings and controller layouts (and their splitscreen guests'),
//! and the online server they sign in to if not the usual one. It's kept
//! in a small text file so it lasts between games.

use crate::camera::Controls;
use crate::input::{ButtonLayout, StickLayout};
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

/// Splitscreen guests with settings of their own: NAME(1) to NAME(3).
const GUESTS: usize = crate::MAX_LOCAL - 1;

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// Their gamertag.
    pub name: String,
    pub look: Look,
    /// Player one's look settings (the controller's and the mouse's) and
    /// controller layouts.
    pub controls: Controls,
    /// Each splitscreen guest's (guest 1 is NAME(1)), kept on this PC
    /// with player one's.
    pub guests: [Controls; GUESTS],
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
            guests: [Controls::default(); GUESTS],
            server: String::new(),
        }
    }
}

/// Read one of a player's control settings: false if `key` isn't one.
/// Values that make no sense keep what was there.
fn read_control(c: &mut Controls, key: &str, value: &str) -> bool {
    let sensitivity = |n: u8| {
        value
            .parse::<u8>()
            .ok()
            .filter(|n| (1..=Controls::MAX_SENSITIVITY).contains(n))
            .unwrap_or(n)
    };
    match key {
        "look_sensitivity" => c.look_sensitivity = sensitivity(c.look_sensitivity),
        "mouse_sensitivity" => c.mouse_sensitivity = sensitivity(c.mouse_sensitivity),
        "invert_look" => c.invert_look = value.eq_ignore_ascii_case("yes"),
        "button_layout" => c.buttons = ButtonLayout::from_key(value).unwrap_or(c.buttons),
        "thumbstick_layout" => c.sticks = StickLayout::from_key(value).unwrap_or(c.sticks),
        _ => return false,
    }
    true
}

/// A player's control settings as profile lines, each key after `prefix`;
/// the mouse's only for player one.
fn control_lines(c: &Controls, prefix: &str, mouse: bool) -> String {
    let mut text = format!("{prefix}look_sensitivity={}\n", c.look_sensitivity);
    if mouse {
        text += &format!("{prefix}mouse_sensitivity={}\n", c.mouse_sensitivity);
    }
    let invert = if c.invert_look { "yes" } else { "no" };
    text += &format!(
        "{prefix}invert_look={invert}\n{prefix}button_layout={}\n{prefix}thumbstick_layout={}\n",
        c.buttons.key(),
        c.sticks.key()
    );
    text
}

impl Profile {
    /// Local player `k`'s controls: player one's, or a guest's.
    pub fn controls_of(&self, k: usize) -> Controls {
        match k.checked_sub(1) {
            None => self.controls,
            Some(g) => self.guests.get(g).copied().unwrap_or_default(),
        }
    }

    /// Local player `k`'s controls, to change (past the last guest, the
    /// last guest's).
    pub fn controls_of_mut(&mut self, k: usize) -> &mut Controls {
        match k.checked_sub(1) {
            None => &mut self.controls,
            Some(g) => &mut self.guests[g.min(GUESTS - 1)],
        }
    }

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
            let (key, value) = (key.trim(), value.trim());
            if read_control(&mut p.controls, key, value) {
                continue;
            }
            // guest1_button_layout=... and the like.
            let guest = key
                .strip_prefix("guest")
                .and_then(|k| k.split_once('_'))
                .and_then(|(n, key)| Some((n.parse::<usize>().ok()?, key)))
                .filter(|&(n, _)| (1..=GUESTS).contains(&n));
            if let Some((n, key)) = guest {
                read_control(&mut p.guests[n - 1], key, value);
                continue;
            }
            let color = || {
                COLORS
                    .iter()
                    .position(|c| c.0.eq_ignore_ascii_case(value))
                    .map(|i| i as u8)
                    .or_else(|| value.parse::<u8>().ok().filter(|&c| c < PROFILE_COLORS))
            };
            match key {
                "name" => {
                    let name = clean_name(value);
                    if !name.is_empty() {
                        p.name = name;
                    }
                }
                "model" => p.look.elite = value.eq_ignore_ascii_case("elite"),
                "server" => p.server = value.to_string(),
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
             emblem_primary={}\nemblem_secondary={}\nemblem_background_color={}\n",
            self.name,
            if self.look.elite { "elite" } else { "spartan" },
            color(self.look.colors[0]),
            color(self.look.colors[1]),
            e.foreground,
            e.background,
            color(e.colors[0]),
            color(e.colors[1]),
            color(e.colors[2]),
        );
        text += &control_lines(&self.controls, "", true);
        // Guests who changed theirs (they have no mouse).
        for (k, g) in self.guests.iter().enumerate() {
            let usual = Controls {
                mouse_sensitivity: g.mouse_sensitivity,
                ..Controls::default()
            };
            if *g != usual {
                text += &control_lines(g, &format!("guest{}_", k + 1), false);
            }
        }
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
                ..Controls::default()
            },
            guests: [Controls::default(); GUESTS],
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

    #[test]
    fn controller_layouts_are_saved_by_name() {
        let mut p = Profile::default();
        p.controls.buttons = ButtonLayout::BumperJumper;
        p.controls.sticks = StickLayout::LegacySouthpaw;
        let text = p.to_text();
        assert!(text.contains("button_layout=bumper_jumper\n"), "{text}");
        assert!(
            text.contains("thumbstick_layout=legacy_southpaw\n"),
            "{text}"
        );
        assert_eq!(Profile::parse(&text), p);
        // Nonsense keeps the defaults, and so does a profile from before
        // there were layouts.
        let q = Profile::parse("button_layout=jumpy\nthumbstick_layout=upside_down");
        assert_eq!(q.controls, Controls::default());
        let old = "name=CHIEF\nmodel=spartan\nprimary=olive\nlook_sensitivity=5\n\
                   mouse_sensitivity=4\ninvert_look=no\n";
        let old = Profile::parse(old);
        assert_eq!(old.controls.buttons, ButtonLayout::Default);
        assert_eq!(old.controls.sticks, StickLayout::Default);
        assert_eq!(old.controls.look_sensitivity, 5);
        assert_eq!(old.guests, [Controls::default(); GUESTS]);
        // Spaces and capitals are forgiven.
        let r = Profile::parse(" button_layout = GREEN_THUMB \nthumbstick_layout=Legacy");
        assert_eq!(r.controls.buttons, ButtonLayout::GreenThumb);
        assert_eq!(r.controls.sticks, StickLayout::Legacy);
    }

    #[test]
    fn guests_keep_their_own_controls() {
        let p = Profile::parse(
            "guest2_button_layout=boxer\nguest2_look_sensitivity=8\n\
             guest3_invert_look=yes\nguest4_button_layout=recon\nguest0_button_layout=recon\n\
             guestx_button_layout=recon\n",
        );
        assert_eq!(p.guests[1].buttons, ButtonLayout::Boxer);
        assert_eq!(p.guests[1].look_sensitivity, 8);
        assert!(p.guests[2].invert_look);
        // Player one, guest 1 and guests that don't exist are untouched.
        assert_eq!(p.controls, Controls::default());
        assert_eq!(p.guests[0], Controls::default());
        assert_eq!(p.controls_of(0), p.controls);
        assert_eq!(p.controls_of(2), p.guests[1]);
        assert_eq!(p.controls_of(9), Controls::default());
        // Saved: those who changed theirs, without a mouse.
        let text = p.to_text();
        assert!(text.contains("guest2_button_layout=boxer\n"), "{text}");
        assert!(text.contains("guest2_look_sensitivity=8\n"), "{text}");
        assert!(text.contains("guest3_invert_look=yes\n"), "{text}");
        assert!(
            !text.contains("guest1_") && !text.contains("guest2_mouse"),
            "{text}"
        );
        assert_eq!(Profile::parse(&text), p);
        let mut q = p.clone();
        q.controls_of_mut(1).sticks = StickLayout::Southpaw;
        assert_eq!(q.guests[0].sticks, StickLayout::Southpaw);
        assert_eq!(q.controls, p.controls);
        assert!(q.to_text().contains("guest1_thumbstick_layout=southpaw\n"));
    }
}
