//! Random sources.
//!
//! The seeded generator is implemented here (xoshiro256** seeded through
//! SplitMix64) instead of taken from a dependency, so the stream for a given
//! seed can never change because a dependency was upgraded. A test fixture
//! recorded today replays with the same seed next year.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// A source of random 64-bit values.
pub trait RngSource: Send {
    /// The next random value.
    fn next_u64(&mut self) -> u64;
}

/// Deterministic xoshiro256** generator. Not cryptographically secure.
#[derive(Debug, Clone)]
pub struct SeededRng {
    s: [u64; 4],
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl SeededRng {
    /// A generator whose whole stream is determined by `seed`.
    pub fn new(seed: u64) -> Self {
        let mut st = seed;
        let s = [
            splitmix64(&mut st),
            splitmix64(&mut st),
            splitmix64(&mut st),
            splitmix64(&mut st),
        ];
        SeededRng { s }
    }
}

impl RngSource for SeededRng {
    fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }
}

/// A non-deterministic generator, seeded from the process's per-run hash
/// randomization and the current time. Use it for "live" runs that you want
/// to *record* rather than control; never for anything security-related.
#[derive(Debug, Clone)]
pub struct EntropyRng {
    inner: SeededRng,
}

impl EntropyRng {
    /// A freshly and unpredictably seeded generator.
    pub fn new() -> Self {
        let mut h = RandomState::new().build_hasher();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        h.write_u128(nanos);
        h.write_usize(std::process::id() as usize);
        EntropyRng {
            inner: SeededRng::new(h.finish()),
        }
    }
}

impl Default for EntropyRng {
    fn default() -> Self {
        Self::new()
    }
}

impl RngSource for EntropyRng {
    fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_stream_is_stable() {
        // Golden values pin the algorithm: changing them breaks every
        // recorded fixture that relies on seed-only reproduction.
        let mut r = SeededRng::new(0);
        let got: Vec<u64> = (0..3).map(|_| r.next_u64()).collect();
        assert_eq!(
            got,
            vec![
                11091344671253066420,
                13793997310169335082,
                1900383378846508768
            ]
        );
    }

    #[test]
    fn splitmix_matches_the_reference_output() {
        // First SplitMix64 output for state 0, as published with the
        // reference implementation; the xoshiro values above were also
        // cross-checked against an independent implementation.
        let mut st = 0u64;
        assert_eq!(splitmix64(&mut st), 0xe220_a839_7b1d_cdaf);
    }

    #[test]
    fn different_seeds_differ() {
        assert_ne!(SeededRng::new(1).next_u64(), SeededRng::new(2).next_u64());
    }
}
