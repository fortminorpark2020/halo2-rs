//! Passwords for the accounts the server's owner makes: the rules, the
//! hashes kept in `logins.txt`, the passwords the server makes up, and
//! gamertags the owner gives.
//!
//! A password is easy to read out and type: spaces and dashes don't count,
//! and neither does case, so `k7mp 4qr9-xw2t` is `K7MP-4QR9-XW2T`. What's
//! left is 8 to 64 printable ASCII characters. Those the server makes up
//! are 12 characters from `ALPHABET` (letters and digits that can't be
//! taken for one another), shown as `XXXX-XXXX-XXXX`: about 56 bits.
//!
//! Only argon2id hashes are kept (as PHC strings, which hold their own
//! parameters, so a later cost still checks an old hash). Our code keeps a
//! password in clear only in a `Secret` (or a `Zeroizing` copy), wiped
//! when dropped, and never logs or writes it anywhere. That wiping is best
//! effort: argon2 (built with `zeroize`) wipes its own first and last
//! hashes, but while it hashes the password also sits in the Blake2b
//! buffer under it and in leftover stack copies, which can't be wiped from
//! here. Someone who can read the server's memory is out of scope, as with
//! `secret.txt`.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use h2net::live::MAX_PASSWORD;
use zeroize::{Zeroize, Zeroizing};

/// Bytes a `Secret` holds before it would have to move (and leave a copy
/// behind): far more than any password.
const SECRET_ROOM: usize = 256;
/// What a password breaking the rules is told.
pub const RULES: &str =
    "PASSWORDS ARE 8 TO 64 LETTERS, NUMBERS OR MARKS (SPACES AND DASHES DON'T COUNT)";
/// What a gamertag breaking the rules is told.
pub const GAMERTAG_RULES: &str =
    "GAMERTAGS ARE 1 TO 15 LETTERS, NUMBERS AND SINGLE SPACES, STARTING WITH A LETTER";
/// Fewest characters in a password, and most in a gamertag the owner gives.
const MIN_PASSWORD: usize = 8;
const MAX_GAMERTAG: usize = 15;

/// A password in clear: wiped when dropped, and its Debug shows nothing.
/// It's made with room for `SECRET_ROOM` bytes, so typing into it never
/// moves it (typing past that is ignored).
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(text: impl Into<String>) -> Secret {
        let mut text = text.into();
        let mut secret = Secret::default();
        secret.0.push_str(&text);
        text.zeroize();
        secret
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Type `c` at the end (if there's room for it without moving).
    pub fn push(&mut self, c: char) {
        if self.0.len() + c.len_utf8() <= self.0.capacity() {
            self.0.push(c);
        }
    }

    pub fn pop(&mut self) -> Option<char> {
        self.0.pop()
    }

    pub fn clear(&mut self) {
        self.0.zeroize();
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Default for Secret {
    fn default() -> Secret {
        Secret(Zeroizing::new(String::with_capacity(SECRET_ROOM)))
    }
}

impl Clone for Secret {
    fn clone(&self) -> Secret {
        Secret::new(self.as_str())
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

/// How hard a hash is to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cost {
    /// OWASP's argon2id baseline: 19 MiB, 2 passes, one lane. How long it
    /// takes on the owner's server is an estimate (25 to 80 ms) until the
    /// command line measures it ("hashing took N ms"). After a change to
    /// it, set the passwords made before again (accounts.md, deploy step
    /// 2), so every hash costs what the stand-in does.
    Real,
    /// As little as argon2id allows, for tests.
    Cheap,
}

impl Cost {
    /// The settings its hashes are made with. (After a change to
    /// `Cost::Real`'s, hashes made before it are still checked with their
    /// own; the server's stand-in follows the settings most of them have,
    /// see `Server::stand_in`.)
    pub fn settings(self) -> Settings {
        let (m, t, p) = match self {
            Cost::Real => (19_456, 2, 1),
            Cost::Cheap => (8, 1, 1),
        };
        Settings { m, t, p }
    }
}

/// The settings an argon2id hash is made with: memory (KiB), passes and
/// lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Settings {
    pub m: u32,
    pub t: u32,
    pub p: u32,
}

impl Settings {
    fn params(self) -> Result<Params, &'static str> {
        Params::new(self.m, self.t, self.p, Some(32)).map_err(|_| "BAD HASH SETTINGS")
    }
}

/// The settings the argon2id hash `phc` was made with, if it's one
/// (`is_phc`).
pub fn settings_of(phc: &str) -> Option<Settings> {
    let hash = PasswordHash::new(phc).ok()?;
    if !is_argon2id(&hash) {
        return None;
    }
    let params = Params::try_from(&hash).ok()?;
    Some(Settings {
        m: params.m_cost(),
        t: params.t_cost(),
        p: params.p_cost(),
    })
}

/// A password as it's checked: without spaces or dashes, ASCII in capitals.
/// (Made with room for all of it, so it never moves and leaves a copy
/// behind as it grows; callers wipe it.)
pub fn normalize(typed: &str) -> String {
    let mut normal = String::with_capacity(typed.len());
    let kept = typed.chars().filter(|&c| c != ' ' && c != '-');
    normal.extend(kept.map(|c| c.to_ascii_uppercase()));
    normal
}

/// `typed` keeps the rules: 8 to 64 printable ASCII characters, once
/// normalized.
pub fn check(typed: &str) -> Result<(), &'static str> {
    let text = Zeroizing::new(normalize(typed));
    let printable = text.bytes().all(|b| (0x21..=0x7e).contains(&b));
    let long = (MIN_PASSWORD..=MAX_PASSWORD).contains(&text.len());
    if printable && long {
        Ok(())
    } else {
        Err(RULES)
    }
}

/// `typed`'s hash, as a PHC string, if it keeps the rules.
pub fn hash(typed: &str, cost: Cost) -> Result<String, &'static str> {
    hash_with(typed, cost.settings())
}

/// `typed`'s hash made with `settings`, as a PHC string, if it keeps the
/// rules.
pub fn hash_with(typed: &str, settings: Settings) -> Result<String, &'static str> {
    check(typed)?;
    let params = settings.params()?;
    let text = Zeroizing::new(normalize(typed));
    let mut salt = [0; 16];
    getrandom::fill(&mut salt).map_err(|_| "NO RANDOM NUMBERS")?;
    let salt = SaltString::encode_b64(&salt).map_err(|_| "A BAD SALT")?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let hash = argon2
        .hash_password(text.as_bytes(), &salt)
        .map_err(|_| "THE PASSWORD COULDN'T BE HASHED")?;
    Ok(hash.to_string())
}

/// `typed` is the password `phc` is the hash of (with the parameters the
/// hash names). False for anything else, a broken hash too.
pub fn verify(phc: &str, typed: &str) -> bool {
    let Ok(hash) = PasswordHash::new(phc) else {
        return false;
    };
    if !is_argon2id(&hash) {
        return false;
    }
    let text = Zeroizing::new(normalize(typed));
    Argon2::default()
        .verify_password(text.as_bytes(), &hash)
        .is_ok()
}

/// `s` is an argon2id hash as `hash` writes them. (Its parameters are
/// bounded too, so a hash sent over the admin socket can't make the server
/// spend gigabytes checking it: 256 MiB, 16 passes and 4 lanes at most.)
pub fn is_phc(s: &str) -> bool {
    PasswordHash::new(s).is_ok_and(|hash| is_argon2id(&hash))
}

fn is_argon2id(hash: &PasswordHash) -> bool {
    let Ok(params) = Params::try_from(hash) else {
        return false;
    };
    hash.algorithm == argon2::ARGON2ID_IDENT
        && hash.salt.is_some()
        && hash.hash.is_some()
        && params.m_cost() <= 256 * 1024
        && params.t_cost() <= 16
        && params.p_cost() <= 4
}

/// The characters made-up passwords are drawn from: no 0 or O, 1, I or L,
/// 2 or Z, 5 or S, 8 or B.
pub const ALPHABET: &[u8] = b"ACDEFGHJKMNPQRTUVWXY34679";

/// A new password: 12 characters from `ALPHABET`, each as likely as the
/// next, as `XXXX-XXXX-XXXX`.
pub fn generate() -> Secret {
    let mut secret = Secret::default();
    let mut drawn = 0;
    // The largest multiple of the alphabet's length a byte holds: bytes at
    // or above it are thrown back, so no character comes up more often.
    let fair = (256 / ALPHABET.len() * ALPHABET.len()) as u8;
    while drawn < 12 {
        let mut bytes = Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut *bytes).expect("random numbers");
        for &b in bytes.iter().filter(|&&b| b < fair) {
            if drawn == 12 {
                break;
            }
            if drawn > 0 && drawn % 4 == 0 {
                secret.push('-');
            }
            secret.push(char::from(ALPHABET[usize::from(b) % ALPHABET.len()]));
            drawn += 1;
        }
    }
    secret
}

/// A gamertag the server's owner gives (trimmed, in capitals), if it keeps
/// the rules: 1 to 15 of A-Z, 0-9 and single spaces, starting with a
/// letter. That's stricter than what players could pick for themselves, so
/// it can never look like a splitscreen guest's "NAME(1)".
pub fn check_gamertag(name: &str) -> Result<String, &'static str> {
    let name = name.trim().to_ascii_uppercase();
    let allowed = |c: char| c.is_ascii_uppercase() || c.is_ascii_digit() || c == ' ';
    let good = name == h2sim::game::clean_name(&name)
        && name.chars().all(allowed)
        && !name.contains("  ")
        && name.starts_with(|c: char| c.is_ascii_uppercase())
        && name.len() <= MAX_GAMERTAG;
    if good {
        Ok(name)
    } else {
        Err(GAMERTAG_RULES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_hash_and_verify() {
        let phc = hash("K7MP-4QR9-XW2T", Cost::Cheap).unwrap();
        assert!(phc.starts_with("$argon2id$v=19$m=8,t=1,p=1$"), "{phc}");
        assert!(is_phc(&phc));
        assert!(verify(&phc, "K7MP-4QR9-XW2T"));
        // Spaces, dashes and case don't count.
        assert!(verify(&phc, "k7mp 4qr9-xw2t"));
        assert!(verify(&phc, "K7MP4QR9XW2T"));
        // Anything else is wrong.
        assert!(!verify(&phc, "K7MP-4QR9-XW2R"));
        assert!(!verify(&phc, ""));
        assert!(!verify("not a hash", "K7MP-4QR9-XW2T"));
        // Each hash has its own salt.
        let again = hash("K7MP-4QR9-XW2T", Cost::Cheap).unwrap();
        assert_ne!(again, phc);
        assert!(verify(&again, "k7mp4qr9xw2t"));
        // The real cost is OWASP's baseline, and checks the same way.
        let real = hash("correct horse", Cost::Real).unwrap();
        assert!(
            real.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "{real}"
        );
        assert!(verify(&real, "CORRECTHORSE"));
    }

    #[test]
    fn hashes_say_their_settings() {
        let cheap = hash("K7MP-4QR9-XW2T", Cost::Cheap).unwrap();
        assert_eq!(settings_of(&cheap), Some(Cost::Cheap.settings()));
        // Made with other settings, a hash names them, and still checks.
        let other = Settings { m: 16, t: 3, p: 2 };
        let made = hash_with("K7MP-4QR9-XW2T", other).unwrap();
        assert!(made.starts_with("$argon2id$v=19$m=16,t=3,p=2$"), "{made}");
        assert_eq!(settings_of(&made), Some(other));
        assert!(verify(&made, "k7mp4qr9xw2t"));
        assert_eq!(settings_of("not a hash"), None);
        assert_eq!(settings_of(&cheap.replace("argon2id", "argon2i")), None);
        // Settings argon2 can't use are refused, not a panic.
        let none = Settings { m: 8, t: 0, p: 1 };
        assert!(hash_with("K7MP-4QR9-XW2T", none).is_err());
    }

    #[test]
    fn passwords_keep_the_rules() {
        assert_eq!(normalize("k7mp 4qr9-xw2t"), "K7MP4QR9XW2T");
        assert_eq!(check("abcdefgh"), Ok(()));
        assert_eq!(check("1234567"), Err(RULES));
        assert_eq!(check("1234-567 "), Err(RULES));
        assert_eq!(check(&"x".repeat(64)), Ok(()));
        assert_eq!(check(&"x".repeat(65)), Err(RULES));
        // Spaces and dashes don't count towards the length either way.
        assert_eq!(check(&format!("{} - -", "x".repeat(64))), Ok(()));
        assert_eq!(check("abcdefg\u{e9}"), Err(RULES));
        assert_eq!(check("abcdefg\t"), Err(RULES));
        assert_eq!(check("!#$%&()*"), Ok(()));
        assert_eq!(hash("short", Cost::Cheap), Err(RULES));
    }

    #[test]
    fn a_normalized_password_is_made_where_it_stays() {
        // Grown a character at a time, it would move (at 8, 16 and 32
        // bytes), leaving its start behind each time: it's made with room
        // for all of it instead.
        for typed in ["correct horse battery staple-and-more", "", "\u{e9}-x y"] {
            let normal = normalize(typed);
            assert_eq!(normal.capacity(), typed.len(), "{typed}");
        }
    }

    #[test]
    fn made_up_passwords_read_out_easily() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let p = generate();
            let text = p.as_str();
            assert_eq!(text.len(), 14, "{text}");
            let groups: Vec<&str> = text.split('-').collect();
            assert_eq!(groups.len(), 3, "{text}");
            assert!(groups.iter().all(|g| g.len() == 4), "{text}");
            assert!(groups.concat().bytes().all(|b| ALPHABET.contains(&b)));
            assert_eq!(check(text), Ok(()));
            seen.extend(groups.concat().bytes());
        }
        // 2400 draws come up with every character.
        assert_eq!(seen.len(), ALPHABET.len());
        // No two characters in the alphabet can be taken for each other.
        for confusable in b"0O1IL2Z5S8B" {
            assert!(!ALPHABET.contains(confusable));
        }
    }

    #[test]
    fn only_argon2id_hashes_count() {
        let phc = hash("abcdefgh", Cost::Cheap).unwrap();
        assert!(is_phc(&phc));
        assert!(!is_phc("-"));
        assert!(!is_phc(""));
        assert!(!is_phc(&phc.replace("argon2id", "argon2i")));
        assert!(!is_phc("$argon2id$v=19$m=8,t=1,p=1"));
        // Costs no one would pick.
        assert!(!is_phc(&phc.replace("m=8,", "m=4194304,")));
        assert!(!is_phc(&phc.replace("t=1,", "t=99,")));
    }

    #[test]
    fn secrets_show_nothing_and_type_in_place() {
        let mut s = Secret::new("hunter22");
        assert_eq!(format!("{s:?}"), "Secret(..)");
        assert_eq!(format!("{:?}", Some(s.clone())), "Some(Secret(..))");
        let at = s.as_str().as_ptr();
        for _ in 0..300 {
            s.push('x');
        }
        // Never moved, and stopped at its room.
        assert_eq!(s.as_str().as_ptr(), at);
        assert_eq!(s.len(), SECRET_ROOM);
        assert_eq!(s.pop(), Some('x'));
        s.clear();
        assert!(s.is_empty());
        assert_eq!(Secret::default().len(), 0);
        assert_eq!(s.clone().as_str(), "");
    }

    #[test]
    fn the_owner_gives_plain_gamertags() {
        assert_eq!(check_gamertag("KAT"), Ok("KAT".into()));
        assert_eq!(check_gamertag(" test c "), Ok("TEST C".into()));
        assert_eq!(check_gamertag("A1"), Ok("A1".into()));
        assert_eq!(check_gamertag(&"A".repeat(15)), Ok("A".repeat(15)));
        for bad in [
            "",
            "   ",
            "1KAT",
            "A  B",
            "NAME(1)",
            "NOBLE-6",
            "KAT!",
            &"A".repeat(16),
            "\u{c9}MILE",
            "TAB\tBED",
        ] {
            assert_eq!(check_gamertag(bad), Err(GAMERTAG_RULES), "{bad:?}");
        }
    }
}
