//! What the server keeps in its data folder, as text.
//!
//! `accounts.txt` holds every account: a line for the account and one for
//! each ranked playlist it has played. It's written whole, to a temporary
//! file renamed over the old one, whenever an account changes, so it's never
//! left half written.
//!
//! ```text
//! a <id> <public key> <gamertag> <look> <created> <seq>
//! x <id> <playlist key> <xp> <level> <games> <wins>
//! ```
//!
//! Ids, keys and looks are in hex, `created` is a Unix time, and `seq` goes
//! up with every change to the account. Gamertags can hold spaces.
//!
//! `games.log` gets a line for every match, so levels can be worked out
//! again if the rules ever change:
//!
//! ```text
//! <unix time> <match> <playlist key> <map> <game type> <counted> <player>...
//! ```
//!
//! with each player as
//! `<id>:<team>:<place>:<left>:<old xp>:<new xp>:<old level>:<new level>`.
//! `<counted>` is 1 if the match changed everyone's levels and 0 if it
//! changed none. It's 2 if it counted only as a loss for its host, who
//! quit: then the host is in place 1, having left, and everyone else in
//! place 0, their levels as they were.

use crate::levels::{Rank, MAX_LEVEL, MAX_XP};
use crate::playlists::GAME_TYPES;
use ed25519_dalek::SigningKey;
use h2sim::game::{Look, Reader, Writer};
use h2sim::GameType;
use sha2::{Digest, Sha512};
use std::io::Write;
use std::path::Path;

/// A player's account, named by the public key their PC signs in with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// The first 8 bytes of the key's SHA-512 (see `account_id`).
    pub id: u64,
    pub key: [u8; 32],
    pub gamertag: String,
    pub look: Look,
    /// When it was made (Unix time).
    pub created: u64,
    /// Goes up with every change, so the newer of two copies is known.
    pub seq: u64,
    /// Its rank and record in each ranked playlist it has played.
    pub stats: Vec<Stats>,
}

/// An account's rank and record in one ranked playlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stats {
    /// The playlist's key.
    pub playlist: String,
    pub rank: Rank,
    pub games: u32,
    pub wins: u32,
}

/// The account named by a public key: the first 8 bytes of its SHA-512.
pub fn account_id(key: &[u8; 32]) -> u64 {
    let hash = Sha512::digest(key);
    u64::from_le_bytes(hash[..8].try_into().expect("a SHA-512 is 64 bytes"))
}

impl Account {
    /// A new account for `key`, made at `created`, with no gamertag yet and
    /// nothing played.
    pub fn new(key: [u8; 32], created: u64) -> Account {
        Account {
            id: account_id(&key),
            key,
            gamertag: String::new(),
            look: Look::default(),
            created,
            seq: 0,
            stats: Vec::new(),
        }
    }

    /// Its lines in accounts.txt (and on its stat card).
    pub fn lines(&self) -> String {
        let mut look = Writer::default();
        self.look.write(&mut look);
        let mut text = format!(
            "a {:016x} {} {} {} {} {}\n",
            self.id,
            hex(&self.key),
            self.gamertag,
            hex(&look.0),
            self.created,
            self.seq
        );
        for s in &self.stats {
            text += &format!(
                "x {:016x} {} {} {} {} {}\n",
                self.id, s.playlist, s.rank.xp, s.rank.level, s.games, s.wins
            );
        }
        text
    }

    /// Its rank and record in the playlist with this key, if it has played
    /// it.
    pub fn stats(&self, playlist: &str) -> Option<&Stats> {
        self.stats.iter().find(|s| s.playlist == playlist)
    }

    /// Its highest level in any ranked playlist (1 if it has played none).
    pub fn best_level(&self) -> u8 {
        self.stats.iter().map(|s| s.rank.level).max().unwrap_or(1)
    }
}

/// Accounts written as `Account::lines` writes them.
pub fn parse(text: &str) -> Result<Vec<Account>, String> {
    let mut accounts: Vec<Account> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let at = |e: &str| format!("line {}: {e}", i + 1);
        if let Some(rest) = line.strip_prefix("a ") {
            let account = account_line(rest).ok_or_else(|| at("not an account"))?;
            if accounts.iter().any(|a| a.id == account.id) {
                return Err(at("an account listed twice"));
            }
            accounts.push(account);
        } else if let Some(rest) = line.strip_prefix("x ") {
            let (id, stats) = stats_line(rest).ok_or_else(|| at("not a playlist record"))?;
            let account = accounts.iter_mut().find(|a| a.id == id);
            let account = account.ok_or_else(|| at("a record for no account"))?;
            if account.stats(&stats.playlist).is_some() {
                return Err(at("a playlist listed twice"));
            }
            account.stats.push(stats);
        } else if !line.trim().is_empty() {
            return Err(at("not an account or a playlist record"));
        }
    }
    Ok(accounts)
}

/// `a <id> <key> <gamertag> <look> <created> <seq>`, after the `a `. The
/// gamertag is whatever lies between the key and the look.
fn account_line(line: &str) -> Option<Account> {
    let (id, rest) = line.split_once(' ')?;
    let (key, rest) = rest.split_once(' ')?;
    let (rest, seq) = rest.rsplit_once(' ')?;
    let (rest, created) = rest.rsplit_once(' ')?;
    let (gamertag, look) = rest.rsplit_once(' ')?;
    let key = unhex(key)?;
    let look = Look::read(&mut Reader::new(&unhex::<8>(look)?)).ok()?;
    let account = Account {
        id: id_from_hex(id)?,
        key,
        gamertag: gamertag.to_string(),
        look,
        created: created.parse().ok()?,
        seq: seq.parse().ok()?,
        stats: Vec::new(),
    };
    let named = !gamertag.is_empty() && gamertag == h2sim::game::clean_name(gamertag);
    (named && account.id == account_id(&key)).then_some(account)
}

/// `x <id> <playlist key> <xp> <level> <games> <wins>`, after the `x `.
fn stats_line(line: &str) -> Option<(u64, Stats)> {
    let words: Vec<&str> = line.split(' ').collect();
    let [id, playlist, xp, level, games, wins] = words[..] else {
        return None;
    };
    let rank = Rank {
        xp: xp.parse().ok().filter(|&xp| xp <= MAX_XP)?,
        level: level.parse().ok().filter(|l| (1..=MAX_LEVEL).contains(l))?,
    };
    let key_char = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_';
    if playlist.is_empty() || !playlist.chars().all(key_char) {
        return None;
    }
    let stats = Stats {
        playlist: playlist.to_string(),
        rank,
        games: games.parse().ok()?,
        wins: wins.parse().ok()?,
    };
    Some((id_from_hex(id)?, stats))
}

/// The accounts in `path`; none if there's no such file.
pub fn load(path: &Path) -> Result<Vec<Account>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Write `text` to `path` whole: to a temporary file first, then renamed
/// over the old one.
pub fn replace(path: &Path, text: &str) -> std::io::Result<()> {
    write_whole(path, text, false)
}

/// `replace`, and for a `secret` only its owner can read the file.
fn write_whole(path: &Path, text: &str, secret: bool) -> std::io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let mut file = std::fs::File::create(&temp)?;
    if secret {
        owner_only(&file)?;
    }
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temp, path)
}

/// Let only `file`'s owner read it. (Elsewhere than Unix, the folders
/// keys are kept in are the user's own.)
#[cfg(unix)]
fn owner_only(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn owner_only(_: &std::fs::File) -> std::io::Result<()> {
    Ok(())
}

/// A signing key kept at `path` as its seed (64 hex digits), made at
/// random and saved there the first time.
pub fn signing_key(path: &Path) -> std::io::Result<SigningKey> {
    match std::fs::read_to_string(path) {
        Ok(text) => match unhex::<32>(text.trim()) {
            Some(seed) => Ok(SigningKey::from_bytes(&seed)),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{} isn't a key", path.display()),
            )),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut seed = [0; 32];
            getrandom::fill(&mut seed).map_err(|e| std::io::Error::other(e.to_string()))?;
            write_whole(path, &format!("{}\n", hex(&seed)), true)?;
            Ok(SigningKey::from_bytes(&seed))
        }
        Err(e) => Err(e),
    }
}

/// Write `accounts` to `path` (accounts.txt) whole.
pub fn save<'a>(
    path: &Path,
    accounts: impl IntoIterator<Item = &'a Account>,
) -> std::io::Result<()> {
    let text: String = accounts.into_iter().map(Account::lines).collect();
    replace(path, &text)
}

/// A match as games.log records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRecord {
    /// When it ended (Unix time).
    pub unix: u64,
    pub id: u64,
    /// The playlist's key.
    pub playlist: String,
    pub map: String,
    pub game_type: GameType,
    pub counted: Counted,
    pub players: Vec<RecordedPlayer>,
}

/// What a match did to levels (games.log's `<counted>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counted {
    /// Nothing (0).
    No,
    /// It changed everyone's (1).
    Yes,
    /// It was only a loss for its host, who quit (2).
    HostLoss,
}

/// A player in a recorded match, and their rank in its playlist before
/// and after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedPlayer {
    pub account: u64,
    pub team: u8,
    pub place: u8,
    pub left: bool,
    pub old: Rank,
    pub new: Rank,
}

impl GameRecord {
    /// Its line in games.log.
    pub fn line(&self) -> String {
        let game_type = GAME_TYPES.iter().find(|(t, _)| *t == self.game_type);
        let mut line = format!(
            "{} {} {} {} {} {}",
            self.unix,
            self.id,
            self.playlist,
            self.map,
            game_type.map_or("slayer", |(_, name)| name),
            self.counted as u8
        );
        for p in &self.players {
            line += &format!(
                " {:016x}:{}:{}:{}:{}:{}:{}:{}",
                p.account,
                p.team,
                p.place,
                p.left as u8,
                p.old.xp,
                p.new.xp,
                p.old.level,
                p.new.level
            );
        }
        line
    }
}

/// Add `game` to the end of the log at `path` (games.log).
pub fn log_game(path: &Path, game: &GameRecord) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", game.line())
}

/// Bytes as lowercase hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `N` bytes from exactly `2N` hex digits.
pub(crate) fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != 2 * N || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut bytes = [0; N];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(bytes)
}

/// An id written as 16 hex digits.
fn id_from_hex(text: &str) -> Option<u64> {
    unhex::<8>(text).map(u64::from_be_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(n: u8, gamertag: &str) -> Account {
        let mut a = Account::new([n; 32], 1_700_000_000 + n as u64);
        a.gamertag = gamertag.to_string();
        a.look = Look::default_for(n as usize);
        a.seq = 3;
        a
    }

    #[test]
    fn accounts_read_back_as_written() {
        let mut a = account(1, "MASTER CHIEF");
        a.stats.push(Stats {
            playlist: "double_team".into(),
            rank: Rank {
                xp: 1234,
                level: 13,
            },
            games: 40,
            wins: 22,
        });
        a.stats.push(Stats {
            playlist: "ffa".into(),
            rank: Rank::default(),
            games: 1,
            wins: 0,
        });
        let b = account(2, "A  B");
        let text = a.lines() + &b.lines();
        assert_eq!(parse(&text), Ok(vec![a.clone(), b]));
        assert_eq!(a.best_level(), 13);
        assert_eq!(account(3, "X").best_level(), 1);
        assert!(a
            .lines()
            .starts_with(&format!("a {:016x} {} MASTER CHIEF ", a.id, hex(&a.key))));
    }

    #[test]
    fn broken_accounts_files_are_errors() {
        let good = account(1, "KAT").lines();
        assert!(parse(&good).is_ok());
        let key = hex(&[1; 32]);
        for bad in [
            // A gamertag that isn't one, or an id that isn't the key's.
            good.replace(" KAT ", " kat "),
            good.replace(" KAT ", "  "),
            good.replace(&key, &hex(&[2; 32])),
            // Bad numbers.
            good.replace(" 3\n", " -3\n"),
            format!("{good}x {:016x} ffa 0 51 0 0\n", account_id(&[1; 32])),
            format!("{good}x {:016x} ffa 0 0 0 0\n", account_id(&[1; 32])),
            format!("{good}x {:016x} Big 0 1 0 0\n", account_id(&[1; 32])),
            // A record for no account, an account twice, and nonsense.
            format!("x {:016x} ffa 0 1 0 0\n", account_id(&[1; 32])),
            format!("{good}{good}"),
            format!("{good}hello\n"),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_signing_key_is_made_once_and_kept() {
        let dir = std::env::temp_dir().join(format!("h2live-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("identity.key");
        let made = signing_key(&path).unwrap();
        let again = signing_key(&path).unwrap();
        assert_eq!(made.to_bytes(), again.to_bytes());
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim().len(), 64);
        // It's secret: only its owner can read it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Another file is another key.
        let other = signing_key(&dir.join("other.key")).unwrap();
        assert_ne!(made.to_bytes(), other.to_bytes());
        // A broken file is an error, not a new key.
        std::fs::write(&path, "not a key").unwrap();
        assert!(signing_key(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not a key");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saving_replaces_the_file_whole() {
        let dir = std::env::temp_dir().join(format!("h2live-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.txt");
        assert_eq!(load(&path), Ok(Vec::new()));
        let accounts = [account(1, "CARTER"), account(2, "JUN")];
        save(&path, &accounts).unwrap();
        save(&path, &accounts[..1]).unwrap();
        assert_eq!(load(&path), Ok(accounts[..1].to_vec()));
        // Nothing is left beside it.
        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(files.len(), 1);

        let log = dir.join("games.log");
        let rank = |xp, level| Rank { xp, level };
        let game = GameRecord {
            unix: 1_700_000_100,
            id: 7,
            playlist: "double_team".into(),
            map: "lockout".into(),
            game_type: GameType::TeamSlayer,
            counted: Counted::Yes,
            players: vec![
                RecordedPlayer {
                    account: 0xab,
                    team: 1,
                    place: 0,
                    left: false,
                    old: rank(900, 10),
                    new: rank(978, 10),
                },
                RecordedPlayer {
                    account: 0xcd,
                    team: 0,
                    place: 1,
                    left: true,
                    old: rank(0, 1),
                    new: rank(0, 1),
                },
            ],
        };
        log_game(&log, &game).unwrap();
        log_game(
            &log,
            &GameRecord {
                id: 8,
                counted: Counted::HostLoss,
                ..game.clone()
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "1700000100 7 double_team lockout team_slayer 1 \
             00000000000000ab:1:0:0:900:978:10:10 00000000000000cd:0:1:1:0:0:1:1"
        );
        assert!(lines[1].starts_with("1700000100 8 double_team lockout team_slayer 2 "));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
