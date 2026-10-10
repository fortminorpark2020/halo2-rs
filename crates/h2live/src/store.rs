//! What the server keeps in its data folder, as text.
//!
//! `accounts.txt` holds every account: a line for the account and one for
//! each ranked playlist it has played. It's written whole, to a temporary
//! file renamed over the old one, whenever an account changes, so it's never
//! left half written.
//!
//! ```text
//! a <id> <public key> <gamertag> <look> <created> <seq>
//! x <id> <playlist key> <xp> <level> <games> <wins> [<kills> <assists> <deaths> <betrayals> <suicides>]
//! ```
//!
//! Ids, keys and looks are in hex, `created` is a Unix time, and `seq` goes
//! up with every change to the account. Gamertags can hold spaces. The
//! five numbers at the end of an `x` line are what the player did in the
//! games that counted for them there (`Tally`); a line without them (as
//! servers before them wrote every line) is all zeros, and a tally of
//! zeros is written that way, so an account nothing has added to is
//! written as it always was.
//!
//! `games.log` gets a line for every match, so levels can be worked out
//! again if the rules ever change:
//!
//! ```text
//! <unix time> <match> <playlist key> <map> <game type> <counted> <player>...
//! ```
//!
//! where a custom game, which has no playlist, gives `-` as its key, and
//! with each player as
//! `<id>:<team>:<place>:<left>:<old xp>:<new xp>:<old level>:<new level>`,
//! followed by
//! `:<score>:<kills>:<assists>:<deaths>:<betrayals>:<suicides>` when the
//! host's result names them (a player it doesn't, or every player of a
//! match whose host sent none, has the first 8 fields only). The game's
//! results have no assists, betrayals or suicides: those are 0 there.
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
    /// What they did in the games that counted for everyone.
    pub tally: Tally,
}

/// What a player did in the games that counted for them in a playlist.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub kills: u32,
    pub assists: u32,
    pub deaths: u32,
    pub betrayals: u32,
    pub suicides: u32,
}

impl Tally {
    /// Add `other` in (each number stopping at the largest a u32 holds).
    pub fn add(&mut self, other: Tally) {
        self.kills = self.kills.saturating_add(other.kills);
        self.assists = self.assists.saturating_add(other.assists);
        self.deaths = self.deaths.saturating_add(other.deaths);
        self.betrayals = self.betrayals.saturating_add(other.betrayals);
        self.suicides = self.suicides.saturating_add(other.suicides);
    }

    fn is_zero(&self) -> bool {
        *self == Tally::default()
    }
}

impl From<Tally> for h2net::live::Tally {
    fn from(t: Tally) -> Self {
        h2net::live::Tally {
            kills: t.kills,
            assists: t.assists,
            deaths: t.deaths,
            betrayals: t.betrayals,
            suicides: t.suicides,
        }
    }
}

impl From<h2net::live::Tally> for Tally {
    fn from(t: h2net::live::Tally) -> Self {
        Tally {
            kills: t.kills,
            assists: t.assists,
            deaths: t.deaths,
            betrayals: t.betrayals,
            suicides: t.suicides,
        }
    }
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
                "x {:016x} {} {} {} {} {}",
                self.id, s.playlist, s.rank.xp, s.rank.level, s.games, s.wins
            );
            let t = &s.tally;
            if !t.is_zero() {
                text += &format!(
                    " {} {} {} {} {}",
                    t.kills, t.assists, t.deaths, t.betrayals, t.suicides
                );
            }
            text += "\n";
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

/// `x <id> <playlist key> <xp> <level> <games> <wins>`, after the `x `,
/// then the five numbers of its tally or none (all zeros).
fn stats_line(line: &str) -> Option<(u64, Stats)> {
    let words: Vec<&str> = line.split(' ').collect();
    let (words, tally) = match words.len() {
        6 => (&words[..], Tally::default()),
        11 => {
            let n = |i: usize| words[i].parse::<u32>().ok();
            let tally = Tally {
                kills: n(6)?,
                assists: n(7)?,
                deaths: n(8)?,
                betrayals: n(9)?,
                suicides: n(10)?,
            };
            (&words[..6], tally)
        }
        _ => return None,
    };
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
        tally,
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
    /// The playlist's key (empty for a custom game).
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
    /// Their score and tally as the host's result says, if it names them.
    pub result: Option<(i32, Tally)>,
}

impl GameRecord {
    /// Its line in games.log.
    pub fn line(&self) -> String {
        let game_type = GAME_TYPES.iter().find(|(t, _)| *t == self.game_type);
        let mut line = format!(
            "{} {} {} {} {} {}",
            self.unix,
            self.id,
            // A custom game has no playlist: `-`, so the words still line
            // up (no key has a `-`).
            if self.playlist.is_empty() {
                "-"
            } else {
                &self.playlist
            },
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
            if let Some((score, t)) = p.result {
                line += &format!(
                    ":{score}:{}:{}:{}:{}:{}",
                    t.kills, t.assists, t.deaths, t.betrayals, t.suicides
                );
            }
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
            tally: Tally {
                kills: 300,
                assists: 45,
                deaths: 280,
                betrayals: 2,
                suicides: u32::MAX,
            },
        });
        a.stats.push(Stats {
            playlist: "ffa".into(),
            rank: Rank::default(),
            games: 1,
            wins: 0,
            tally: Tally::default(),
        });
        let b = account(2, "A  B");
        let text = a.lines() + &b.lines();
        assert_eq!(parse(&text), Ok(vec![a.clone(), b]));
        assert_eq!(a.best_level(), 13);
        assert_eq!(account(3, "X").best_level(), 1);
        assert!(a
            .lines()
            .starts_with(&format!("a {:016x} {} MASTER CHIEF ", a.id, hex(&a.key))));
        // A tally of zeros is written as servers before tallies wrote every
        // line, and one that isn't with its five numbers after.
        let id = format!("{:016x}", a.id);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[1],
            format!("x {id} double_team 1234 13 40 22 300 45 280 2 4294967295")
        );
        assert_eq!(lines[2], format!("x {id} ffa 0 1 1 0"));
    }

    #[test]
    fn todays_accounts_files_read_as_they_are() {
        // As every server before tallies wrote it, byte for byte.
        let id = format!("{:016x}", account_id(&[1; 32]));
        let text = format!(
            "{}x {id} double_team 1234 13 40 22\n",
            account(1, "KAT").lines()
        );
        let accounts = parse(&text).unwrap();
        let stats = &accounts[0].stats[0];
        assert_eq!((stats.games, stats.wins), (40, 22));
        assert_eq!(stats.tally, Tally::default());
        // Untouched, it's written back the same.
        assert_eq!(accounts[0].lines(), text);
    }

    #[test]
    fn tallies_add_up_and_stop_at_the_largest() {
        let mut t = Tally {
            kills: 1,
            assists: 2,
            deaths: 3,
            betrayals: 4,
            suicides: u32::MAX - 1,
        };
        t.add(Tally {
            kills: 10,
            assists: 20,
            deaths: 30,
            betrayals: 40,
            suicides: 5,
        });
        let sum = Tally {
            kills: 11,
            assists: 22,
            deaths: 33,
            betrayals: 44,
            suicides: u32::MAX,
        };
        assert_eq!(t, sum);
        let wire = h2net::live::Tally::from(t);
        assert_eq!((wire.kills, wire.suicides), (11, u32::MAX));
        assert_eq!(Tally::from(wire), t);
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
            // Tallies of 1 to 4 numbers, or 6, or numbers that aren't.
            format!("{good}x {:016x} ffa 0 1 0 0 1\n", account_id(&[1; 32])),
            format!("{good}x {:016x} ffa 0 1 0 0 1 2\n", account_id(&[1; 32])),
            format!("{good}x {:016x} ffa 0 1 0 0 1 2 3\n", account_id(&[1; 32])),
            format!(
                "{good}x {:016x} ffa 0 1 0 0 1 2 3 4\n",
                account_id(&[1; 32])
            ),
            format!(
                "{good}x {:016x} ffa 0 1 0 0 1 2 3 4 5 6\n",
                account_id(&[1; 32])
            ),
            format!(
                "{good}x {:016x} ffa 0 1 0 0 1 2 3 4 -5\n",
                account_id(&[1; 32])
            ),
            format!(
                "{good}x {:016x} ffa 0 1 0 0 1 2 3 4 4294967296\n",
                account_id(&[1; 32])
            ),
            format!(
                "{good}x {:016x} ffa 0 1 0 0 1 2 x 4 5\n",
                account_id(&[1; 32])
            ),
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
                    result: Some((
                        -2,
                        Tally {
                            kills: 5,
                            assists: 1,
                            deaths: 9,
                            betrayals: 1,
                            suicides: 6,
                        },
                    )),
                },
                RecordedPlayer {
                    account: 0xcd,
                    team: 0,
                    place: 1,
                    left: true,
                    old: rank(0, 1),
                    new: rank(0, 1),
                    result: None,
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
             00000000000000ab:1:0:0:900:978:10:10:-2:5:1:9:1:6 00000000000000cd:0:1:1:0:0:1:1"
        );
        assert!(lines[1].starts_with("1700000100 8 double_team lockout team_slayer 2 "));
        // A custom game has no playlist key.
        let custom = GameRecord {
            playlist: String::new(),
            counted: Counted::No,
            ..game.clone()
        };
        assert!(custom
            .line()
            .starts_with("1700000100 7 - lockout team_slayer 0 "));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
