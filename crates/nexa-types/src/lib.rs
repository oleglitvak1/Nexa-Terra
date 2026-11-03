//! Shared data types for Nexa Terra.
//!
//! Everything here is plain, serializable data with no behavior beyond simple
//! conversions. The client, the web build and (later) the server all depend on
//! these definitions, so they must stay platform-independent.

use serde::{Deserialize, Serialize};

/// Hours in one Nexa Terra planetary day.
pub const HOURS_PER_SOL: u64 = 26;

/// One simulation tick is one planetary hour.
pub const TICKS_PER_SOL: u64 = HOURS_PER_SOL;

/// Absolute simulation time, counted in ticks (planetary hours) since landfall.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Tick(pub u64);

impl Tick {
    /// The Sol this tick falls in, starting at Sol 0.
    pub fn sol(self) -> u64 {
        self.0 / TICKS_PER_SOL
    }

    /// The hour within the current Sol, 0..HOURS_PER_SOL.
    pub fn hour_of_sol(self) -> u64 {
        self.0 % TICKS_PER_SOL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_maps_to_sol_and_hour() {
        assert_eq!(Tick(0).sol(), 0);
        assert_eq!(Tick(25).sol(), 0);
        assert_eq!(Tick(26).sol(), 1);
        assert_eq!(Tick(27).hour_of_sol(), 1);
    }
}
