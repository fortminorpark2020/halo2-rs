//! Stat cards: an account's lines from accounts.txt, signed by the server.
//! A PC keeps its card (live-card.txt) and shows it each time it signs in,
//! so a server that lost its accounts (a free host wipes its disk on every
//! restart) gets them back from the players themselves, and no one can
//! raise their own levels without the server's key. The key stays the same
//! from one start to the next: it comes from a secret (H2LIVE_SECRET) or
//! from `secret.txt` in the data folder.
//!
//! A card is the account's lines, then `s <signature>`: the server's
//! Ed25519 signature of the lines before it, in hex.

use crate::store::{self, hex, unhex, Account};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha512};
use std::path::Path;

/// The server's signing key: from `secret` if there is one (the first 32
/// bytes of its SHA-512), otherwise from `secret.txt` in `dir`, made the
/// first time.
pub fn server_key(dir: &Path, secret: Option<&str>) -> Result<SigningKey, String> {
    match secret.filter(|s| !s.is_empty()) {
        Some(secret) => {
            let hash = Sha512::digest(secret.as_bytes());
            let seed = hash[..32].try_into().expect("a SHA-512 is 64 bytes");
            Ok(SigningKey::from_bytes(&seed))
        }
        None => {
            let path = dir.join("secret.txt");
            store::signing_key(&path).map_err(|e| format!("{}: {e}", path.display()))
        }
    }
}

/// `account`'s card, signed with `key`.
pub fn sign(account: &Account, key: &SigningKey) -> String {
    let lines = account.lines();
    let signature = key.sign(lines.as_bytes());
    format!("{lines}s {}\n", hex(&signature.to_bytes()))
}

/// The account on `card`, if `key` signed it just as it is.
pub fn verify(card: &str, key: &VerifyingKey) -> Option<Account> {
    let last = card.strip_suffix('\n')?.rfind('\n').map_or(0, |i| i + 1);
    let (lines, signature) = card.split_at(last);
    let signature = unhex::<64>(signature.trim_end().strip_prefix("s ")?)?;
    key.verify_strict(lines.as_bytes(), &Signature::from_bytes(&signature))
        .ok()?;
    let [account] = <[Account; 1]>::try_from(store::parse(lines).ok()?).ok()?;
    Some(account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::levels::Rank;
    use crate::store::Stats;

    fn account() -> Account {
        let mut a = Account::new([5; 32], 1_700_000_000);
        a.gamertag = "SPARTAN 117".into();
        a.seq = 9;
        a.stats.push(Stats {
            playlist: "team_slayer".into(),
            rank: Rank {
                xp: 2100,
                level: 17,
            },
            games: 60,
            wins: 31,
        });
        a
    }

    #[test]
    fn a_card_holds_the_account_the_server_signed() {
        let key = SigningKey::from_bytes(&[1; 32]);
        let card = sign(&account(), &key);
        assert!(card.starts_with(&account().lines()));
        assert_eq!(verify(&card, &key.verifying_key()), Some(account()));
        // Another server's key doesn't vouch for it.
        let other = SigningKey::from_bytes(&[2; 32]);
        assert_eq!(verify(&card, &other.verifying_key()), None);
    }

    #[test]
    fn a_changed_card_is_refused() {
        let key = SigningKey::from_bytes(&[1; 32]).verifying_key();
        let card = sign(&account(), &SigningKey::from_bytes(&[1; 32]));
        // Any change at all, even a byte of the signature.
        for (i, _) in card.char_indices() {
            let mut changed = card.clone().into_bytes();
            changed[i] ^= 1;
            if let Ok(changed) = String::from_utf8(changed) {
                assert_eq!(verify(&changed, &key), None, "byte {i}");
            }
        }
        let raised = card.replace(" 2100 17 ", " 9999 30 ");
        assert_ne!(raised, card);
        assert_eq!(verify(&raised, &key), None);
        // Cut short, or with nothing on it.
        assert_eq!(verify(&card[..card.len() - 2], &key), None);
        assert_eq!(verify(&card[card.find('\n').unwrap() + 1..], &key), None);
        assert_eq!(verify("", &key), None);
        assert_eq!(verify("\n", &key), None);
    }

    #[test]
    fn the_server_key_comes_from_the_secret_or_is_kept_on_disk() {
        let dir = std::env::temp_dir().join(format!("h2live-card-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The same secret makes the same key, without touching the disk.
        let a = server_key(&dir, Some("correct horse")).unwrap();
        let b = server_key(&dir, Some("correct horse")).unwrap();
        assert_eq!(a.to_bytes(), b.to_bytes());
        assert_ne!(
            a.to_bytes(),
            server_key(&dir, Some("battery staple")).unwrap().to_bytes()
        );
        assert!(!dir.join("secret.txt").exists());
        // Without one, a key is made once and kept.
        let made = server_key(&dir, None).unwrap();
        assert!(dir.join("secret.txt").exists());
        assert_eq!(
            made.to_bytes(),
            server_key(&dir, Some("")).unwrap().to_bytes()
        );
        assert_ne!(made.to_bytes(), a.to_bytes());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
