//! Nexa Terra simulation core.
//!
//! Rules for this crate (see CLAUDE.md):
//! - No graphics, audio, input, file system, threads or wall-clock time.
//! - Deterministic: the same starting world and commands always give the same result.
//! - Integer or fixed-point math for anything economic or shared.
//! - Must compile for `wasm32-unknown-unknown`.

use nexa_types::{Tick, TICKS_PER_SOL};
use serde::{Deserialize, Serialize};

pub mod machines;

/// The whole state of one player's game. Grows milestone by milestone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct World {
    /// Current simulation time.
    pub now: Tick,
}

/// Something that happened during a tick, for the front end to display or sound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimEvent {
    /// A new Sol has begun.
    SolStarted { sol: u64 },
}

impl World {
    /// A fresh world at the moment of landfall.
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance the simulation by one fixed step (one planetary hour).
    pub fn tick(&mut self) -> Vec<SimEvent> {
        self.now = Tick(self.now.0 + 1);
        let mut events = Vec::new();
        if self.now.0.is_multiple_of(TICKS_PER_SOL) {
            events.push(SimEvent::SolStarted {
                sol: self.now.sol(),
            });
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_sol_of_ticks_starts_sol_one() {
        let mut world = World::new();
        let mut events = Vec::new();
        for _ in 0..TICKS_PER_SOL {
            events.extend(world.tick());
        }
        assert_eq!(world.now.sol(), 1);
        assert_eq!(events, vec![SimEvent::SolStarted { sol: 1 }]);
    }

    #[test]
    fn simulation_is_deterministic() {
        let mut a = World::new();
        let mut b = World::new();
        for _ in 0..1000 {
            assert_eq!(a.tick(), b.tick());
        }
        assert_eq!(a, b);
    }
}
