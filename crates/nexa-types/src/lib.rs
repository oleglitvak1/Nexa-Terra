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

/// Temperatures are whole hundredths of a degree C everywhere (`*_centi_c`).
pub const CENTI_PER_DEGREE: i32 = 100;

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

/// Compact log stamp, e.g. `S001 H02`.
impl std::fmt::Display for Tick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "S{:03} H{:02}", self.sol(), self.hour_of_sol())
    }
}

/// Stable name of a colony module, as written in content files (e.g. `hab_01`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModuleId(pub String);

impl ModuleId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModuleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stable name of a crew member, as written in content files (e.g. `maya`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CrewId(pub String);

impl CrewId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CrewId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stable name of an orbital station, as written in content files (e.g. `barge_77`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StationId(pub String);

impl StationId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for StationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stable name of a Slip hazard, as written in content files (e.g. `pirate_interdiction`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HazardId(pub String);

impl HazardId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for HazardId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Something the player (or a Standing Order) asks the colony to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    Activate(ModuleId),
    Deactivate(ModuleId),
    /// Send all heat on the module's heat line to that module.
    DivertHeat {
        to: ModuleId,
    },
    /// Undo a diversion: heat on this module's line is shared again.
    ShareHeat {
        line_of: ModuleId,
    },
    /// Fit a spare part to a failed machine.
    Repair(ModuleId),
}

/// What the universe is told when a ship launches into The Slip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchContext {
    pub destination: StationId,
    /// Colony time at launch.
    pub departure: Tick,
    /// Nominal Sols of the leg.
    pub transit_sols: u32,
    /// Who flies, in roster order.
    pub flight_crew: Vec<CrewId>,
    /// Drawn from the colony's RNG, so an offline universe is deterministic.
    pub seed: u64,
}

/// A hazard waiting on the route, `sol` Sols into the leg.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlipEvent {
    pub sol: u32,
    pub hazard: HazardId,
}

/// What the universe answers at launch: what lies on the route.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LaunchBriefing {
    pub events: Vec<SlipEvent>,
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

    #[test]
    fn tick_displays_as_a_log_stamp() {
        assert_eq!(Tick(27).to_string(), "S001 H01");
    }

    #[test]
    fn crew_id_displays_its_name() {
        let id = CrewId::new("maya");
        assert_eq!(id.as_str(), "maya");
        assert_eq!(id.to_string(), "maya");
    }
}
