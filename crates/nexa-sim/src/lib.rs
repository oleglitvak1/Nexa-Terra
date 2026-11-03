//! Nexa Terra simulation core.
//!
//! Rules for this crate (see CLAUDE.md):
//! - No graphics, audio, input, file system, threads or wall-clock time.
//! - Deterministic: the same starting world and commands always give the same result.
//! - Integer or fixed-point math for anything economic or shared.
//! - Must compile for `wasm32-unknown-unknown`.

use colony::{Colony, CommandError, LANDING_ASH_PLAIN};
use machines::MachineState;
use nexa_types::{Command, ModuleId, Tick, TICKS_PER_SOL};
use serde::{Deserialize, Serialize};

pub mod colony;
pub mod grid;
pub mod machines;
pub mod parts;

/// The whole state of one player's game. Grows milestone by milestone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct World {
    /// Current simulation time.
    pub now: Tick,
    pub colony: Colony,
}

/// Something that happened during a tick, for the front end to display or sound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimEvent {
    /// A new Sol has begun.
    SolStarted { sol: u64 },
    MachineStateChanged {
        module: ModuleId,
        state: MachineState,
    },
}

impl World {
    pub fn new(colony: Colony) -> Self {
        Self {
            now: Tick::default(),
            colony,
        }
    }

    /// The vertical slice's start: landfall on the Ash Plain.
    pub fn landfall() -> Self {
        // The shipped content is checked by colony's tests, so failing here is a build bug.
        Self::new(Colony::from_ron(LANDING_ASH_PLAIN).expect("landing site content is valid"))
    }

    /// Advance the simulation by one fixed step (one planetary hour).
    pub fn tick(&mut self) -> Vec<SimEvent> {
        let mut events = self.colony.tick();
        self.now = Tick(self.now.0 + 1);
        if self.now.0.is_multiple_of(TICKS_PER_SOL) {
            events.push(SimEvent::SolStarted {
                sol: self.now.sol(),
            });
        }
        events
    }

    pub fn apply(&mut self, cmd: &Command) -> Result<Vec<SimEvent>, CommandError> {
        self.colony.apply(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_sol_of_ticks_starts_sol_one() {
        let mut world = World::landfall();
        let mut events = Vec::new();
        for _ in 0..TICKS_PER_SOL {
            events.extend(world.tick());
        }
        assert_eq!(world.now.sol(), 1);
        let sols: Vec<_> = events
            .into_iter()
            .filter(|e| matches!(e, SimEvent::SolStarted { .. }))
            .collect();
        assert_eq!(sols, vec![SimEvent::SolStarted { sol: 1 }]);
    }

    #[test]
    fn simulation_is_deterministic() {
        let mut a = World::landfall();
        let mut b = World::landfall();
        let heater_off = Command::Deactivate(ModuleId::new("heater_01"));
        a.apply(&heater_off).unwrap();
        b.apply(&heater_off).unwrap();
        for _ in 0..1000 {
            assert_eq!(a.tick(), b.tick());
        }
        assert_eq!(a, b);
    }

    #[test]
    fn world_survives_a_save_round_trip() {
        let mut world = World::landfall();
        for _ in 0..100 {
            world.tick();
        }
        let saved = ron::to_string(&world).unwrap();
        assert_eq!(ron::from_str::<World>(&saved).unwrap(), world);
    }
}
