//! The sim's only source of randomness: a small seeded generator that lives in
//! the save, so a loaded game rolls exactly what the saved one would have.
//!
//! SplitMix64: one `u64` of state, full period, good enough statistics for
//! dice, and trivially the same on every platform. No extra crate needed.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimRng {
    state: u64,
}

impl SimRng {
    /// Seed for new games that don't pick one, and for saves from before the RNG.
    pub const DEFAULT_SEED: u64 = 0x4E45_5841_5445_5252; // "NEXATERR"

    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A whole number from `lo` to `hi`, both included. A reversed range gives `lo`.
    pub fn range(&mut self, lo: u32, hi: u32) -> u32 {
        if hi <= lo {
            return lo;
        }
        let span = u128::from(hi - lo) + 1;
        // Multiply-shift instead of `%`: no division, bias below 2^-32.
        let offset = (u128::from(self.next_u64()) * span) >> 64;
        lo + offset as u32
    }
}

impl Default for SimRng {
    fn default() -> Self {
        Self::new(Self::DEFAULT_SEED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_splitmix64_reference_sequence() {
        let mut rng = SimRng::new(0);
        assert_eq!(rng.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(rng.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(rng.next_u64(), 0x06C4_5D18_8009_454F);
    }

    #[test]
    fn same_seed_same_rolls() {
        let mut a = SimRng::new(42);
        let mut b = SimRng::new(42);
        for _ in 0..100 {
            assert_eq!(a.range(1, 20), b.range(1, 20));
        }
        assert_ne!(SimRng::new(1).next_u64(), SimRng::new(2).next_u64());
    }

    #[test]
    fn range_stays_inside_and_reaches_both_ends() {
        let mut rng = SimRng::default();
        let mut seen = [false; 7];
        for _ in 0..1000 {
            let n = rng.range(5, 11);
            assert!((5..=11).contains(&n), "{n}");
            seen[(n - 5) as usize] = true;
        }
        assert!(seen.iter().all(|&s| s));
        assert_eq!(rng.range(3, 3), 3);
        assert_eq!(rng.range(9, 2), 9);
    }

    #[test]
    fn survives_a_save_mid_sequence() {
        let mut rng = SimRng::new(7);
        rng.next_u64();
        let mut loaded: SimRng = ron::from_str(&ron::to_string(&rng).unwrap()).unwrap();
        assert_eq!(loaded.next_u64(), rng.next_u64());
    }
}
