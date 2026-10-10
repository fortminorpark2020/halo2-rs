//! The relay's two secrets-based checks, both HMAC-SHA256 cut to 16 bytes
//! under keys only the server holds:
//!
//! - **Address cookies.** The server answers a hello that has no valid
//!   cookie with a [`Kind::Challenge`](crate::Kind::Challenge) carrying
//!   one, made from the address the hello came from, the room, the
//!   sender's id and the time (in 30-second steps). Only a hello that
//!   brings it back from that same address counts, so the server never
//!   registers anyone at an address they can't receive at (a forged source
//!   address), and keeps nothing for a hello until it does.
//! - **Member keys.** For a room h2live issued, each player's id has a key
//!   made from the room and the id, which h2live hands only to that player
//!   (over its signed-in connection). A hello into such a room proves the
//!   key with an HMAC over its cookie, so no one else (another player in
//!   the match included) can join under that id, move it, or make up ids.

use sha2::{Digest, Sha256};
use std::fmt;
use std::net::{IpAddr, SocketAddr};

/// The length of a cookie, of a hello's key proof, and of a member key.
pub const COOKIE_LEN: usize = 16;
/// A client's hello payload: its cookie (zeros before it has one), then
/// its key proof (zeros without a key).
pub const HELLO_LEN: usize = 2 * COOKIE_LEN;

/// What a player proves it's a room's member with, for rooms h2live issued
/// (see [`RelayHandle::member_key`](crate::RelayHandle::member_key)).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MemberKey(pub [u8; COOKIE_LEN]);

impl fmt::Debug for MemberKey {
    // Not the key itself, so it stays out of logs.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "MemberKey(..)")
    }
}

impl MemberKey {
    /// What a hello into `room` as `src` carries after `cookie` (the one
    /// the server's challenge gave): proof that it holds this key, good
    /// only with that cookie (and so from that address, for a minute).
    pub fn proof(&self, room: u64, src: u64, cookie: &[u8]) -> [u8; COOKIE_LEN] {
        let parts: [&[u8]; 4] = [
            b"h2relay hello",
            &room.to_le_bytes(),
            &src.to_le_bytes(),
            cookie,
        ];
        mac16(&self.0, &parts)
    }

    /// As 32 hex digits, to pass along as text.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// From 32 hex digits.
    pub fn from_hex(text: &str) -> Option<MemberKey> {
        let text = text.trim();
        if text.len() != 2 * COOKIE_LEN || !text.is_ascii() {
            return None;
        }
        let mut key = [0; COOKIE_LEN];
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
        }
        Some(MemberKey(key))
    }
}

/// HMAC-SHA256 (RFC 2104) of `parts`, one after the other, under `key`
/// (at most 64 bytes, as all of ours are).
fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    debug_assert!(key.len() <= 64);
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for (i, k) in key.iter().take(64).enumerate() {
        ipad[i] ^= k;
        opad[i] ^= k;
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    for part in parts {
        inner.update(part);
    }
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn mac16(key: &[u8], parts: &[&[u8]]) -> [u8; COOKIE_LEN] {
    let mut out = [0; COOKIE_LEN];
    out.copy_from_slice(&hmac(key, parts)[..COOKIE_LEN]);
    out
}

/// Whether `a` and `b` are the same, taking as long either way.
pub(crate) fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |d, (x, y)| d | (x ^ y)) == 0
}

/// `addr` as bytes: the IP address as IPv6 (an IPv4 one mapped), then the
/// port.
fn addr_bytes(addr: SocketAddr) -> [u8; 18] {
    let ip = match addr.ip() {
        IpAddr::V4(v4) => v4.to_ipv6_mapped(),
        IpAddr::V6(v6) => v6,
    };
    let mut out = [0; 18];
    out[..16].copy_from_slice(&ip.octets());
    out[16..].copy_from_slice(&addr.port().to_le_bytes());
    out
}

/// The cookie for a hello from `from` into `room` as `src`, in time step
/// `epoch`.
pub(crate) fn cookie(
    secret: &[u8; 32],
    epoch: u64,
    from: SocketAddr,
    room: u64,
    src: u64,
) -> [u8; COOKIE_LEN] {
    let parts: [&[u8]; 5] = [
        b"h2relay cookie",
        &epoch.to_le_bytes(),
        &addr_bytes(from),
        &room.to_le_bytes(),
        &src.to_le_bytes(),
    ];
    mac16(secret, &parts)
}

/// Member `id`'s key for issued room `room`.
pub(crate) fn member_key(secret: &[u8; 32], room: u64, id: u64) -> MemberKey {
    let parts: [&[u8]; 3] = [b"h2relay member", &room.to_le_bytes(), &id.to_le_bytes()];
    MemberKey(mac16(secret, &parts))
}

/// A new random server secret.
pub(crate) fn random_secret() -> std::io::Result<[u8; 32]> {
    let mut secret = [0; 32];
    getrandom::fill(&mut secret).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn hmac_matches_rfc_4231() {
        // Test cases 1 and 2.
        let one = hmac(&[0x0b; 20], &[b"Hi ", b"There"]);
        assert_eq!(
            hex(&one),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        let two = hmac(b"Jefe", &[b"what do ya want for nothing?"]);
        assert_eq!(
            hex(&two),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn cookies_and_keys_depend_on_everything_they_cover() {
        let secret = [7; 32];
        let a: SocketAddr = "192.0.2.1:5000".parse().unwrap();
        let b: SocketAddr = "192.0.2.1:5001".parse().unwrap();
        let base = cookie(&secret, 1, a, 10, 20);
        assert_eq!(base, cookie(&secret, 1, a, 10, 20));
        for other in [
            cookie(&[8; 32], 1, a, 10, 20),
            cookie(&secret, 2, a, 10, 20),
            cookie(&secret, 1, b, 10, 20),
            cookie(&secret, 1, a, 11, 20),
            cookie(&secret, 1, a, 10, 21),
        ] {
            assert_ne!(base, other);
        }
        let key = member_key(&secret, 10, 20);
        assert_ne!(key, member_key(&secret, 10, 21));
        assert_ne!(key, member_key(&secret, 11, 20));
        let p = key.proof(10, 20, &base);
        assert_ne!(p, member_key(&secret, 10, 21).proof(10, 20, &base));
        assert_ne!(p, key.proof(10, 20, &[0; COOKIE_LEN]));
        assert!(same(&p, &key.proof(10, 20, &base)));
        assert!(!same(&p, &[0; COOKIE_LEN]));
        assert!(!same(&p, &p[..15]));
    }

    #[test]
    fn member_keys_go_to_text_and_back() {
        let key = MemberKey(*b"0123456789abcdef");
        let text = key.to_hex();
        assert_eq!(text, "30313233343536373839616263646566");
        assert_eq!(MemberKey::from_hex(&text), Some(key));
        assert_eq!(MemberKey::from_hex(&text.to_uppercase()), Some(key));
        assert_eq!(MemberKey::from_hex(&text[1..]), None);
        assert_eq!(MemberKey::from_hex(&format!("{}zz", &text[2..])), None);
        // 32 bytes, but not all ASCII (no slicing inside a character).
        assert_eq!(MemberKey::from_hex("é313233343536373839616263646566"), None);
        assert_eq!(format!("{key:?}"), "MemberKey(..)");
        assert_ne!(random_secret().unwrap(), random_secret().unwrap());
    }
}
