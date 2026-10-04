//! Snapshots sent as how they differ from the one before: the two XORed
//! (the shorter padded with zeros), which is mostly zeros from one tick to
//! the next, then deflated. That is 4-6 times smaller than the snapshot.

use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use std::io::{Read, Write};

/// What turns `last` into `next`.
pub fn encode(last: &[u8], next: &[u8]) -> Vec<u8> {
    let mut z = DeflateEncoder::new(Vec::new(), Compression::fast());
    // Writing to memory can't fail.
    let _ = z.write_all(&xor(last, next));
    z.finish().unwrap_or_default()
}

/// `next` from `last` and what `encode` made of the two, if it's `len`
/// bytes long as it should be.
pub fn decode(last: &[u8], delta: &[u8], len: usize) -> Option<Vec<u8>> {
    if len > crate::conn::MAX_MESSAGE {
        return None;
    }
    let mut x = Vec::new();
    DeflateDecoder::new(delta)
        .take(len as u64 + 1)
        .read_to_end(&mut x)
        .ok()?;
    (x.len() == len).then(|| xor(last, &x))
}

/// `b` XOR `a`, as long as `b`.
fn xor(a: &[u8], b: &[u8]) -> Vec<u8> {
    let pad = std::iter::repeat(&0u8);
    b.iter()
        .zip(a.iter().chain(pad))
        .map(|(x, y)| x ^ y)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_turn_one_snapshot_into_the_next() {
        let last: Vec<u8> = (0..500).map(|i| (i * 7) as u8).collect();
        let mut next = last.clone();
        next[3] = 99;
        // Longer, then shorter, than the one before.
        next.extend_from_slice(&[1, 2, 3]);
        let d = encode(&last, &next);
        assert!(d.len() < 40, "{}", d.len());
        assert_eq!(decode(&last, &d, next.len()), Some(next.clone()));
        let short = &last[..100];
        assert_eq!(
            decode(&next, &encode(&next, short), 100).as_deref(),
            Some(short)
        );
        // The wrong length, or garbage, is refused.
        assert_eq!(decode(&last, &d, next.len() - 1), None);
        assert_eq!(decode(&last, &d, next.len() + 1), None);
        assert_eq!(decode(&last, &[0xFF; 10], 10), None);
    }
}
