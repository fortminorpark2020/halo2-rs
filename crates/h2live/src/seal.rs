//! Passwords sealed to the server, so they never cross the plain ws://
//! link in clear, and the key that tells launchers the server is the one
//! they know.
//!
//! The server has an Ed25519 identity key, made from its stat-card key
//! (`identity`), so it stays the same while `secret.txt` (or
//! H2LIVE_SECRET) does. For each launcher connection the server makes an
//! X25519 key and signs it with the identity key over the connection's
//! challenge (SEAL_KEY, `key_proof`). The launcher checks that signature
//! (and that the identity key is the one it pinned for the server) before
//! it sends anything.
//!
//! A box: the launcher makes an X25519 key of its own for each box, and
//! both ends work out the same shared point from it and the server's key.
//! The box's key is SHA-256 of a label, that point and both public keys;
//! the box is ChaCha20-Poly1305 over the padded passwords (`pad`), with
//! extra data (`aad`) binding what the box is for, the connection's
//! challenge, the PC's key and what it's about, so a box can't be used
//! anywhere else. Each password is padded to the same size, so a box's
//! length says nothing of it.
//!
//! The nonce is twelve zero bytes. That's safe here because no key ever
//! seals more than one box: the sender's X25519 key is new for every box,
//! so the shared point, and so the box's key, is too. (A nonce must never
//! repeat under one key; with a key used once, any fixed nonce never does.)
//! A server key whose shared point comes out all zeros (a low-order point,
//! which would make the key known to anyone) is refused at both ends.

use crate::password::Secret;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use curve25519_dalek::montgomery::MontgomeryPoint;
use ed25519_dalek::SigningKey;
use h2net::live::{Sealed, MAX_PASSWORD, SEALED_ONE};
use sha2::{Digest, Sha256, Sha512};
use zeroize::Zeroizing;

/// What a box is for (the first byte of its extra data after the label).
/// Signing in: the box's target is the LOGIN's gamertag, as sent.
pub const SIGN_IN: u8 = 1;
/// A new account: the ACCOUNT's gamertag, as sent.
pub const CREATE: u8 = 2;
/// A new password for an account: its id, little-endian.
pub const SET_PASSWORD: u8 = 3;
/// The PC's own account's password changing: its id, little-endian.
pub const CHANGE_PASSWORD: u8 = 4;

/// The labels: for the identity key, its signature over a connection's
/// key, a box's extra data and a box's key.
pub const IDENTITY: &[u8] = b"h2live-identity-4";
pub const KEY_PROOF: &[u8] = b"h2live-seal-key-4";
pub const AAD: &[u8] = b"h2live-seal-4";
pub const KDF: &[u8] = b"h2live-seal-4 key";

/// The server's identity key, from its stat-card key: a label keeps the
/// two keys apart, so neither signs what the other does.
pub fn identity(card_key: &SigningKey) -> SigningKey {
    let seed = Zeroizing::new(card_key.to_bytes());
    let hash = Sha512::new().chain_update(IDENTITY).chain_update(*seed);
    let hash = Zeroizing::new(<[u8; 64]>::from(hash.finalize()));
    let mut seed = Zeroizing::new([0; 32]);
    seed.copy_from_slice(&hash[..32]);
    SigningKey::from_bytes(&seed)
}

/// The X25519 public key of `secret`.
pub fn public(secret: &[u8; 32]) -> [u8; 32] {
    MontgomeryPoint::mul_base_clamped(*secret).to_bytes()
}

/// What the identity key signs to vouch for a connection's X25519 key.
pub fn key_proof(nonce: &[u8; 32], public: &[u8; 32]) -> Vec<u8> {
    [KEY_PROOF, nonce, public].concat()
}

/// A box's extra data: what it's for (`SIGN_IN` and the rest), the
/// connection's challenge, the PC's key, and its target.
pub fn aad(purpose: u8, nonce: &[u8; 32], device: &[u8; 32], target: &[u8]) -> Vec<u8> {
    [AAD, &[purpose], nonce, device, target].concat()
}

/// Passwords as a box holds them: each one byte of length, its bytes, then
/// zeros up to `SEALED_ONE`. None if any is longer than `MAX_PASSWORD`.
pub fn pad(passwords: &[&str]) -> Option<Zeroizing<Vec<u8>>> {
    let mut plain = Zeroizing::new(Vec::with_capacity(passwords.len() * SEALED_ONE));
    for password in passwords {
        let bytes = password.as_bytes();
        if bytes.len() > MAX_PASSWORD {
            return None;
        }
        plain.push(bytes.len() as u8);
        plain.extend_from_slice(bytes);
        let end = plain.len() + MAX_PASSWORD - bytes.len();
        plain.resize(end, 0);
    }
    Some(plain)
}

/// The passwords `pad` padded, if `plain` is whole blocks of them, each
/// with zeros after it.
pub fn unpad(plain: &[u8]) -> Option<Vec<Secret>> {
    if plain.is_empty() || !plain.len().is_multiple_of(SEALED_ONE) {
        return None;
    }
    plain
        .chunks(SEALED_ONE)
        .map(|block| {
            let n = usize::from(block[0]);
            let (text, rest) = block[1..].split_at_checked(n)?;
            if rest.iter().any(|&b| b != 0) {
                return None;
            }
            Some(Secret::new(std::str::from_utf8(text).ok()?))
        })
        .collect()
}

/// The box's key from the shared point and both public keys, or None if
/// the point is all zeros.
fn box_key(
    shared: &MontgomeryPoint,
    sender: &[u8; 32],
    server: &[u8; 32],
) -> Option<Zeroizing<[u8; 32]>> {
    let shared = Zeroizing::new(shared.to_bytes());
    if shared.iter().all(|&b| b == 0) {
        return None;
    }
    let hash = Sha256::new()
        .chain_update(KDF)
        .chain_update(*shared)
        .chain_update(sender)
        .chain_update(server)
        .finalize();
    Some(Zeroizing::new(hash.into()))
}

/// Seal `plain` to the server's X25519 key `server`, with `ephemeral` as
/// this box's own secret (use it for one box only; `seal` makes a new one
/// each time).
pub fn seal_with(
    ephemeral: &[u8; 32],
    server: &[u8; 32],
    aad: &[u8],
    plain: &[u8],
) -> Option<Sealed> {
    let sender = public(ephemeral);
    let shared = MontgomeryPoint(*server).mul_clamped(*ephemeral);
    let key = box_key(&shared, &sender, server)?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&*key));
    let boxed = cipher
        .encrypt(&Nonce::default(), Payload { msg: plain, aad })
        .ok()?;
    Some(Sealed { key: sender, boxed })
}

/// Seal `plain` to `server` with a new secret of its own.
pub fn seal(server: &[u8; 32], aad: &[u8], plain: &[u8]) -> Option<Sealed> {
    let mut ephemeral = Zeroizing::new([0; 32]);
    getrandom::fill(&mut *ephemeral).ok()?;
    seal_with(&ephemeral, server, aad, plain)
}

/// What `sealed` holds, opened with the server's X25519 `secret`, if it
/// was sealed to it with this extra data and holds one or two padded
/// passwords.
pub fn open(secret: &[u8; 32], aad: &[u8], sealed: &Sealed) -> Option<Zeroizing<Vec<u8>>> {
    let shared = MontgomeryPoint(sealed.key).mul_clamped(*secret);
    let key = box_key(&shared, &sealed.key, &public(secret))?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&*key));
    let payload = Payload {
        msg: &sealed.boxed,
        aad,
    };
    let plain = Zeroizing::new(cipher.decrypt(&Nonce::default(), payload).ok()?);
    [SEALED_ONE, 2 * SEALED_ONE]
        .contains(&plain.len())
        .then_some(plain)
}

/// The first 8 bytes of `key` as `AB12-CD34-EF56-7890`, for people to
/// compare by eye or read out.
pub fn short_code(key: &[u8; 32]) -> String {
    let hex: Vec<String> = key[..8]
        .chunks(2)
        .map(|pair| format!("{:02X}{:02X}", pair[0], pair[1]))
        .collect();
    hex.join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::hex;
    use ed25519_dalek::{Signature, Signer};

    const S: [u8; 32] = [1; 32];
    const E: [u8; 32] = [2; 32];
    const NONCE: [u8; 32] = [3; 32];
    const DEVICE: [u8; 32] = [4; 32];

    fn sign_in_aad() -> Vec<u8> {
        aad(SIGN_IN, &NONCE, &DEVICE, b"KAT")
    }

    fn passwords(plain: &[u8]) -> Vec<String> {
        let secrets = unpad(plain).unwrap();
        secrets.iter().map(|s| s.as_str().to_string()).collect()
    }

    #[test]
    fn a_box_opens_with_the_server_key_and_what_it_was_for() {
        let plain = pad(&["K7MP4QR9XW2T"]).unwrap();
        let sealed = seal(&public(&S), &sign_in_aad(), &plain).unwrap();
        assert_eq!(sealed.boxed.len(), SEALED_ONE + 16);
        let opened = open(&S, &sign_in_aad(), &sealed).unwrap();
        assert_eq!(passwords(&opened), ["K7MP4QR9XW2T"]);
        // Each box has a key of its own.
        let again = seal(&public(&S), &sign_in_aad(), &plain).unwrap();
        assert_ne!((again.key, &again.boxed), (sealed.key, &sealed.boxed));
        // Another server's key doesn't open it.
        assert!(open(&[9; 32], &sign_in_aad(), &sealed).is_none());
    }

    #[test]
    fn launchers_and_servers_seal_alike() {
        // Pinned, so a launcher built from one commit and a server from
        // another can't drift apart.
        let plain = pad(&["K7MP4QR9XW2T"]).unwrap();
        let sealed = seal_with(&E, &public(&S), &sign_in_aad(), &plain).unwrap();
        assert_eq!(
            hex(&sealed.key),
            "ce8d3ad1ccb633ec7b70c17814a5c76ecd029685050d344745ba05870e587d59"
        );
        assert_eq!(hex(&sealed.boxed), KNOWN_BOX);
        assert_eq!(
            hex(&public(&S)),
            "a4e09292b651c278b9772c569f5fa9bb13d906b46ab68c9df9dc2b4409f8a209"
        );
    }

    /// The box `launchers_and_servers_seal_alike` pins (checked against
    /// another X25519 and ChaCha20-Poly1305, Python's cryptography package,
    /// when it was pinned).
    const KNOWN_BOX: &str = "6390bef5d3f7c7d198fbf3a7894658608f64955da40e29f0c47a07fead797cec\
                             ab00df4f1360495247730a0bc6a327363d16b66b78c225cefb2ed10df51f1758\
                             d5da67e97f4d3239353fe50ee14dc1e270";

    #[test]
    fn a_box_opens_only_for_what_it_was_sealed_for() {
        let plain = pad(&["K7MP4QR9XW2T"]).unwrap();
        let sealed = seal_with(&E, &public(&S), &sign_in_aad(), &plain).unwrap();
        for other in [
            aad(CREATE, &NONCE, &DEVICE, b"KAT"),
            aad(SIGN_IN, &[5; 32], &DEVICE, b"KAT"),
            aad(SIGN_IN, &NONCE, &[5; 32], b"KAT"),
            aad(SIGN_IN, &NONCE, &DEVICE, b"JUN"),
            aad(SIGN_IN, &NONCE, &DEVICE, b"KAT "),
        ] {
            assert!(open(&S, &other, &sealed).is_none());
        }
        // A bit flipped anywhere.
        for i in 0..sealed.boxed.len() {
            let mut flipped = sealed.clone();
            flipped.boxed[i] ^= 1;
            assert!(open(&S, &sign_in_aad(), &flipped).is_none(), "byte {i}");
        }
        let mut flipped = sealed.clone();
        flipped.key[0] ^= 1;
        assert!(open(&S, &sign_in_aad(), &flipped).is_none());
        // Cut short.
        let mut short = sealed;
        short.boxed.pop();
        assert!(open(&S, &sign_in_aad(), &short).is_none());
    }

    #[test]
    fn a_zero_point_seals_and_opens_nothing() {
        let plain = pad(&["K7MP4QR9XW2T"]).unwrap();
        assert!(seal_with(&E, &[0; 32], &sign_in_aad(), &plain).is_none());
        assert!(seal(&[0; 32], &sign_in_aad(), &plain).is_none());
        let zero = Sealed {
            key: [0; 32],
            boxed: vec![0; SEALED_ONE + 16],
        };
        assert!(open(&S, &sign_in_aad(), &zero).is_none());
    }

    #[test]
    fn padding_hides_the_length() {
        let one = pad(&["abcdefgh"]).unwrap();
        assert_eq!(one.len(), 65);
        assert_eq!(pad(&[&"x".repeat(64)]).unwrap().len(), 65);
        let two = pad(&["old password", "new password"]).unwrap();
        assert_eq!(two.len(), 130);
        assert_eq!(passwords(&two), ["old password", "new password"]);
        assert!(pad(&[&"x".repeat(65)]).is_none());
        assert!(pad(&["fine", &"x".repeat(65)]).is_none());
        // Boxes of one password are all one length, and of two another.
        let short = seal(&public(&S), b"", &pad(&["a"]).unwrap()).unwrap();
        let long = seal(&public(&S), b"", &pad(&[&"x".repeat(64)]).unwrap()).unwrap();
        assert_eq!(short.boxed.len(), long.boxed.len());
        assert_eq!(seal(&public(&S), b"", &two).unwrap().boxed.len(), 146);
        // Not whole blocks, a length past the block, or junk after it.
        assert!(unpad(&[]).is_none());
        assert!(unpad(&one[..64]).is_none());
        let mut bad = one.to_vec();
        bad[0] = 65;
        assert!(unpad(&bad).is_none());
        let mut bad = one.to_vec();
        bad[64] = 1;
        assert!(unpad(&bad).is_none());
        let mut bad = one.to_vec();
        bad[1] = 0xff;
        assert!(unpad(&bad).is_none());
        // A box holding three passwords doesn't open as passwords.
        let three = pad(&["a", "b", "c"]).unwrap();
        let sealed = seal(&public(&S), b"", &three).unwrap();
        assert!(open(&S, b"", &sealed).is_none());
    }

    #[test]
    fn the_identity_key_is_the_servers_own() {
        let card = SigningKey::from_bytes(&[7; 32]);
        let id = identity(&card);
        assert_eq!(id.to_bytes(), identity(&card).to_bytes());
        assert_ne!(id.to_bytes(), card.to_bytes());
        assert_ne!(
            id.to_bytes(),
            identity(&SigningKey::from_bytes(&[8; 32])).to_bytes()
        );
        // It vouches for a connection's key over its challenge.
        let key = public(&S);
        let sig = id.sign(&key_proof(&NONCE, &key));
        let check = |nonce: &[u8; 32], key: &[u8; 32], sig: &Signature| {
            id.verifying_key()
                .verify_strict(&key_proof(nonce, key), sig)
                .is_ok()
        };
        assert!(check(&NONCE, &key, &sig));
        assert!(!check(&[5; 32], &key, &sig));
        assert!(!check(&NONCE, &public(&E), &sig));
        let card_sig = card.sign(&key_proof(&NONCE, &key));
        assert!(!check(&NONCE, &key, &card_sig));
    }

    #[test]
    fn the_identity_key_and_what_it_signs_are_pinned() {
        // Launchers keep a server's identity key (live-servers.txt, and
        // H2LAUNCH_SERVER_KEY in the friends' zip): changing how it's made
        // (the label, say) changes every server's key, and every launcher
        // then says SERVER KEY CHANGED. (Checked against Python's
        // cryptography package when it was pinned.)
        let id = identity(&SigningKey::from_bytes(&[7; 32]));
        assert_eq!(
            hex(&id.to_bytes()),
            "9993a11cb760cf4e70a290bd32d3196d9cae20b17e476adc7f5e4bda898411c7"
        );
        assert_eq!(
            hex(&id.verifying_key().to_bytes()),
            "fdffcf9be426d5fd074df7ea54f1a353defc3d8ee55093bf7ae391af81cc85ed"
        );
        // SEAL_KEY's signature is over its label, the challenge and the key.
        let key = public(&S);
        let expected = [&b"h2live-seal-key-4"[..], &NONCE, &key].concat();
        assert_eq!(key_proof(&NONCE, &key), expected);
    }

    #[test]
    fn short_codes_are_read_out_in_fours() {
        let mut key = [0; 32];
        key[..8].copy_from_slice(&[0xab, 0x12, 0xcd, 0x34, 0xef, 0x56, 0x78, 0x90]);
        assert_eq!(short_code(&key), "AB12-CD34-EF56-7890");
        assert_eq!(short_code(&[0; 32]), "0000-0000-0000-0000");
    }
}
