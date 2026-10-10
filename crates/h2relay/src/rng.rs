//! A small random number source shared between threads without a lock
//! (splitmix64 over an atomic counter): for the lag hooks, jitter, hello
//! nonces and the first sequence number of each reliable stream. Not for
//! secrets: room tokens come from h2live.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const GOLDEN: u64 = 0x9e37_79b9_7f4a_7c15;

pub(crate) struct Rng(AtomicU64);

impl Rng {
    /// Seeded from the clock, the process, where it lives and `extra`, so
    /// two clients started at once (in one process too) differ.
    pub(crate) fn seeded(extra: u64) -> Rng {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let local = 0u8;
        let here = &local as *const u8 as u64;
        let seed = nanos ^ u64::from(std::process::id()).rotate_left(32) ^ here ^ mix(extra);
        Rng(AtomicU64::new(seed))
    }

    pub(crate) fn next_u64(&self) -> u64 {
        mix(self.0.fetch_add(GOLDEN, Ordering::Relaxed))
    }

    pub(crate) fn next_u32(&self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// A number in 0..1.
    pub(crate) fn unit(&self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// splitmix64's finaliser.
fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(GOLDEN);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_spread_over_the_range() {
        let rng = Rng::seeded(1);
        let units: Vec<f64> = (0..10_000).map(|_| rng.unit()).collect();
        assert!(units.iter().all(|u| (0.0..1.0).contains(u)));
        let below = units.iter().filter(|&&u| u < 0.25).count();
        assert!((2000..3000).contains(&below), "{below}");
        assert_ne!(Rng::seeded(1).next_u64(), Rng::seeded(2).next_u64());
    }
}
