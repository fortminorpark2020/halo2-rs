//! The matchmaking servers' identity keys, as this launcher last saw them
//! (`live-servers.txt` in the launcher's folder, one per `--instance`).
//!
//! Each time a launcher signs in, the server proves it holds its identity
//! key before anything goes to it (h2live's SEAL_KEY). A launcher that has
//! a key "pinned" for the address it dialled gives up at once if the server
//! answers with another (`LiveEvent::KeyChanged`), so nobody pretending to
//! be the server hears a password. A key is pinned the first time a server
//! is signed in to, or, for the owner's server (`settings::DEFAULT_SERVER`),
//! when the launcher is built with `H2LAUNCH_SERVER_KEY` (its 64 hex
//! digits, which `h2live user key` prints). A key learned from the server
//! wins over the built-in one.
//!
//! Keys are kept by the address the launcher dials (`live::server_url`,
//! in lower case), so every way of writing one server ("h2.mohnjorrow.com",
//! "h2.mohnjorrow.com:47050", "ws://H2.mohnjorrow.com:47050/live") has one
//! key, the built-in one too. A file written with addresses as typed (by an
//! earlier launcher) reads the same way.
//!
//! ```text
//! # h2launch: the matchmaking servers' identity keys, as last seen.
//! ws://h2.mohnjorrow.com:47050/live 3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29
//! ```

use crate::lobby::settings::DEFAULT_SERVER;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// The file's name, in the launcher's folder.
pub const FILE: &str = "live-servers.txt";

/// The owner's server's identity key, as built in (`H2LAUNCH_SERVER_KEY`
/// when the launcher was built), if it was.
const BUILT_IN: Option<&str> = option_env!("H2LAUNCH_SERVER_KEY");

/// The keys pinned: those learned (from the file), and the built-in one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pins {
    learned: BTreeMap<String, [u8; 32]>,
    built_in: Option<[u8; 32]>,
    /// The built-in key wasn't 64 hex digits, so it's ignored.
    bad_built_in: bool,
    /// Lines of the file that didn't read, skipped.
    skipped: usize,
}

impl Pins {
    /// The keys in `folder`'s `live-servers.txt` (none if there isn't one)
    /// and the built-in one. A file that's there but can't be read is an
    /// error: taken as no file, the keys learned would be unpinned, and the
    /// next one pinned would write over them all.
    pub fn load(folder: &Path) -> io::Result<Pins> {
        let text = match std::fs::read_to_string(path(folder)) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        Ok(Pins::parse(&text, BUILT_IN))
    }

    /// The keys in `text` (lines that don't read are skipped), with
    /// `built_in` (64 hex digits) for the owner's server.
    pub fn parse(text: &str, built_in: Option<&str>) -> Pins {
        let mut learned = BTreeMap::new();
        let mut skipped = 0;
        // (Notepad's "UTF-8 with BOM" starts the file with one.)
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let read = line
                .rsplit_once(char::is_whitespace)
                .and_then(|(server, key)| {
                    let server = clean(server);
                    Some((server, unhex(key.trim())?)).filter(|(s, _)| !s.is_empty())
                });
            match read {
                Some((server, key)) => {
                    learned.insert(server, key);
                }
                None => skipped += 1,
            }
        }
        let built_in = built_in.map(str::trim).filter(|k| !k.is_empty());
        Pins {
            learned,
            built_in: built_in.and_then(unhex),
            bad_built_in: built_in.is_some_and(|k| unhex(k).is_none()),
            skipped,
        }
    }

    /// What the log should say about the keys, if anything is wrong: the
    /// built-in one, or lines of the file that didn't read.
    pub fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if self.bad_built_in {
            notes.push(
                "pins: the built-in server key (H2LAUNCH_SERVER_KEY) isn't 64 hex digits: ignored"
                    .to_string(),
            );
        }
        if self.skipped > 0 {
            notes.push(format!(
                "pins: {} of {FILE} didn't read (an address, a space, 64 hex digits): skipped",
                if self.skipped == 1 {
                    "1 line".to_string()
                } else {
                    format!("{} lines", self.skipped)
                }
            ));
        }
        notes
    }

    /// The key pinned for `server` (an address in any of the ways it may be
    /// written): the one learned, else for the owner's server the built-in
    /// one.
    pub fn get(&self, server: &str) -> Option<[u8; 32]> {
        let server = clean(server);
        self.learned
            .get(&server)
            .copied()
            .or_else(|| self.built_in.filter(|_| server == clean(DEFAULT_SERVER)))
    }

    /// Pin `key` for `server`.
    pub fn set(&mut self, server: &str, key: [u8; 32]) {
        let server = clean(server);
        if !server.is_empty() {
            self.learned.insert(server, key);
        }
    }

    /// The file's text: the keys learned.
    pub fn to_text(&self) -> String {
        let mut text =
            String::from("# h2launch: the matchmaking servers' identity keys, as last seen.\n");
        for (server, key) in &self.learned {
            text += &format!("{server} {}\n", hex(key));
        }
        text
    }

    /// Write the keys learned to `folder`'s `live-servers.txt` (whole, so
    /// a crash never leaves half a file).
    pub fn save(&self, folder: &Path) -> io::Result<()> {
        h2live::store::replace(&path(folder), &self.to_text())
    }
}

/// `live-servers.txt` in `folder`.
pub fn path(folder: &Path) -> PathBuf {
    folder.join(FILE)
}

/// A server's address as pins are kept by: the URL the launcher dials for
/// it, in lower case. (Nothing, for no address.)
fn clean(server: &str) -> String {
    let server = server.trim();
    if server.is_empty() {
        return String::new();
    }
    crate::live::server_url(server).to_ascii_lowercase()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 64 hex digits as 32 bytes.
fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (k, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * k..2 * k + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [0xAB; 32];
    const OTHER: [u8; 32] = [0x12; 32];

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("h2pins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn pins_read_back_and_bad_lines_are_skipped() {
        let dir = scratch("round-trip");
        let mut pins = Pins::load(&dir).unwrap();
        assert_eq!(pins.get("192.168.8.102:47050"), None);
        pins.set("  192.168.8.102:47050 ", KEY);
        pins.set("Example.ORG", OTHER);
        pins.save(&dir).unwrap();
        let text = std::fs::read_to_string(path(&dir)).unwrap();
        assert!(
            text.contains(&format!("ws://example.org:47050/live {}\n", hex(&OTHER))),
            "{text}"
        );
        let back = Pins::parse(&text, None);
        assert_eq!(back.get("192.168.8.102:47050"), Some(KEY));
        // The address is found in any case, with spaces around it.
        assert_eq!(back.get(" EXAMPLE.org"), Some(OTHER));
        assert_eq!(back.learned, pins.learned);
        // Lines that don't read are skipped; the rest still count.
        let odd = format!(
            "# a comment\n\nnokey\nshort abcd\nbad {}\nzz {}\n good.example {}\n",
            "g".repeat(64),
            "ab".repeat(33),
            hex(&KEY)
        );
        let odd = Pins::parse(&odd, None);
        assert_eq!(odd.learned.len(), 1);
        assert_eq!(odd.get("good.example"), Some(KEY));
        // The log is told how many were skipped.
        assert_eq!(
            odd.notes(),
            [format!(
                "pins: 4 lines of {FILE} didn't read (an address, a space, 64 hex digits): skipped"
            )]
        );
        assert!(back.notes().is_empty());
        // A file Notepad saved with a byte-order mark reads the same.
        let marked = Pins::parse(&format!("\u{feff}{text}"), None);
        assert_eq!(marked.learned, pins.learned);
        // Set again: the newer key.
        pins.set("example.org", KEY);
        assert_eq!(pins.get("example.org"), Some(KEY));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_built_in_pin_is_only_the_owners_server_and_a_learned_one_wins() {
        let built = hex(&KEY);
        let pins = Pins::parse("", Some(&built));
        assert_eq!(pins.get(DEFAULT_SERVER), Some(KEY));
        assert_eq!(pins.get(&DEFAULT_SERVER.to_uppercase()), Some(KEY));
        assert_eq!(pins.get("192.168.8.102"), None);
        assert_eq!(pins.get("example.org"), None);
        assert!(pins.notes().is_empty());
        // A key learned for the owner's server wins over the built-in one.
        let text = format!("{DEFAULT_SERVER} {}\n", hex(&OTHER));
        let pins = Pins::parse(&text, Some(&built));
        assert_eq!(pins.get(DEFAULT_SERVER), Some(OTHER));
        // The built-in one isn't written to the file.
        let mut pins = Pins::parse("", Some(&built));
        pins.set("example.org", OTHER);
        assert!(!pins.to_text().contains(&built));
        // A bad built-in key is ignored, and the log is told.
        let pins = Pins::parse("", Some("not hex"));
        assert_eq!(pins.get(DEFAULT_SERVER), None);
        assert!(pins.notes()[0].contains("H2LAUNCH_SERVER_KEY"));
        assert!(Pins::parse("", Some("")).notes().is_empty());
    }

    #[test]
    fn every_way_of_writing_a_server_finds_its_one_key() {
        // The owner's server, however it's written, has the built-in key.
        let built = hex(&KEY);
        let pins = Pins::parse("", Some(&built));
        for server in [
            "h2.mohnjorrow.com",
            "h2.mohnjorrow.com:47050",
            " ws://H2.mohnjorrow.com:47050/live ",
        ] {
            assert_eq!(pins.get(server), Some(KEY), "{server}");
        }
        // Another port, or another way in, is another server.
        assert_eq!(pins.get("h2.mohnjorrow.com:47051"), None);
        assert_eq!(pins.get("wss://h2.mohnjorrow.com/live"), None);
        // A key learned under one spelling is found under the others.
        let mut pins = Pins::default();
        pins.set("Example.org:47050", OTHER);
        assert_eq!(pins.get("example.org"), Some(OTHER));
        assert_eq!(pins.get("ws://EXAMPLE.ORG:47050/live"), Some(OTHER));
        pins.set("ws://example.org:47050/live", KEY);
        assert_eq!(pins.get("example.org"), Some(KEY));
        assert_eq!(pins.learned.len(), 1);
        // So with IPv6 addresses, in brackets.
        pins.set("[::1]", OTHER);
        assert_eq!(pins.get("[::1]:47050"), Some(OTHER));
        // A file an earlier launcher wrote, by the address as typed, reads
        // the same way, and the key learned for the owner's server still
        // wins over the built-in one.
        let old = format!(
            "h2.mohnjorrow.com {}\nexample.org:47050 {}\n",
            hex(&OTHER),
            hex(&KEY)
        );
        let pins = Pins::parse(&old, Some(&built));
        assert_eq!(pins.get("ws://h2.mohnjorrow.com:47050/live"), Some(OTHER));
        assert_eq!(pins.get("example.org"), Some(KEY));
        assert!(pins
            .to_text()
            .contains("ws://h2.mohnjorrow.com:47050/live "));
        // No address has no key.
        assert_eq!(Pins::parse("", Some(&built)).get("  "), None);
    }

    #[test]
    fn a_file_that_cannot_be_read_is_an_error_not_an_empty_one() {
        let dir = scratch("unreadable");
        // No file: no keys learned yet.
        assert_eq!(Pins::load(&dir).unwrap().learned.len(), 0);
        // One that isn't UTF-8 (saved as UTF-16, say) isn't taken as none.
        std::fs::write(path(&dir), [0xFF, 0xFE, b'h', 0, b'2', 0]).unwrap();
        let e = Pins::load(&dir).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        // Nor is one that's there but can't be opened (a folder, here).
        std::fs::remove_file(path(&dir)).unwrap();
        std::fs::create_dir(path(&dir)).unwrap();
        assert!(Pins::load(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
