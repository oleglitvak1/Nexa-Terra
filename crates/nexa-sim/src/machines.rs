//! Machine thermal state: tolerance bands and the operating state they imply.
//!
//! Scope for this task: classifying temperature against a tolerance band and
//! tracking the resulting state. Wear accumulation and the parts needed to
//! clear a failure (architecture.md's `machines` module) land in a later task.

use serde::{Deserialize, Serialize};

/// A machine's operating state, driven by its temperature vs. its [`ToleranceBand`].
///
/// Burnout and Offline are terminal: per the game design's Component-Based
/// Repairs, clearing them needs a replacement part (a later task), not just a
/// temperature change back into the nominal band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MachineState {
    /// Within the tolerance band: 0% wear.
    Nominal,
    /// Above the band's max; sustained overheat burns out the filament.
    Overheat,
    /// Below the band's min; sustained freeze cracks the seal.
    Freeze,
    /// Filament destroyed by overheat. Needs a new Tungsten Heating Filament.
    Burnout,
    /// Seal cracked by freeze, or otherwise shut down. Needs a new Viton Seal.
    Offline,
}

/// The temperature range, in centi-degrees Celsius, where a machine runs at 0% wear.
///
/// Fields are private so min <= max holds however a band is built, including
/// when it is deserialized from content or a save.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawToleranceBand")]
pub struct ToleranceBand {
    min_centi_c: i32,
    max_centi_c: i32,
}

/// Unvalidated wire form of [`ToleranceBand`]; serde checks it via `TryFrom`.
#[derive(Deserialize)]
struct RawToleranceBand {
    min_centi_c: i32,
    max_centi_c: i32,
}

/// A tolerance band whose min is above its max.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvertedBand {
    pub min_centi_c: i32,
    pub max_centi_c: i32,
}

impl std::fmt::Display for InvertedBand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "tolerance band min ({}) must not exceed max ({})",
            self.min_centi_c, self.max_centi_c
        )
    }
}

impl std::error::Error for InvertedBand {}

impl TryFrom<RawToleranceBand> for ToleranceBand {
    type Error = InvertedBand;

    fn try_from(raw: RawToleranceBand) -> Result<Self, Self::Error> {
        Self::try_new(raw.min_centi_c, raw.max_centi_c)
    }
}

impl ToleranceBand {
    /// Panics on an inverted band; for bands defined in code. Data goes through [`Self::try_new`].
    pub fn new(min_centi_c: i32, max_centi_c: i32) -> Self {
        Self::try_new(min_centi_c, max_centi_c).unwrap_or_else(|e| panic!("{e}"))
    }

    pub fn try_new(min_centi_c: i32, max_centi_c: i32) -> Result<Self, InvertedBand> {
        if min_centi_c > max_centi_c {
            return Err(InvertedBand {
                min_centi_c,
                max_centi_c,
            });
        }
        Ok(Self {
            min_centi_c,
            max_centi_c,
        })
    }

    pub fn min_centi_c(&self) -> i32 {
        self.min_centi_c
    }

    pub fn max_centi_c(&self) -> i32 {
        self.max_centi_c
    }

    /// Classify a temperature against this band. Bounds are inclusive.
    pub fn classify(&self, centi_c: i32) -> MachineState {
        if centi_c > self.max_centi_c {
            MachineState::Overheat
        } else if centi_c < self.min_centi_c {
            MachineState::Freeze
        } else {
            MachineState::Nominal
        }
    }
}

/// A single machine: its tolerance band, current temperature and resulting state.
///
/// Fields are private so `state` can't drift from the temperature: every
/// temperature change goes through [`Machine::set_temperature`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Machine {
    tolerance: ToleranceBand,
    temp_centi_c: i32,
    state: MachineState,
}

impl Machine {
    /// A machine at the given starting temperature; state follows from the band.
    pub fn new(tolerance: ToleranceBand, temp_centi_c: i32) -> Self {
        let state = tolerance.classify(temp_centi_c);
        Self {
            tolerance,
            temp_centi_c,
            state,
        }
    }

    pub fn tolerance(&self) -> ToleranceBand {
        self.tolerance
    }

    pub fn temp_centi_c(&self) -> i32 {
        self.temp_centi_c
    }

    pub fn state(&self) -> MachineState {
        self.state
    }

    /// Update temperature and reclassify, unless the machine has already failed:
    /// Burnout and Offline only clear through a repair action, not a temperature change.
    pub fn set_temperature(&mut self, centi_c: i32) {
        self.temp_centi_c = centi_c;
        if !matches!(self.state, MachineState::Burnout | MachineState::Offline) {
            self.state = self.tolerance.classify(centi_c);
        }
    }

    /// Sustained overheat destroys the heating filament (architecture.md, machines module).
    pub fn burnout(&mut self) {
        self.state = MachineState::Burnout;
    }

    /// Sustained freeze cracks the seal and takes the machine offline.
    pub fn take_offline(&mut self) {
        self.state = MachineState::Offline;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band() -> ToleranceBand {
        // Arbitrary smelter-like band: -5.00C to 80.00C, in centi-degrees.
        ToleranceBand::new(-500, 8000)
    }

    #[test]
    fn classify_within_band_is_nominal() {
        assert_eq!(band().classify(2000), MachineState::Nominal);
    }

    #[test]
    fn classify_bounds_are_inclusive_nominal() {
        let b = band();
        assert_eq!(b.classify(b.min_centi_c()), MachineState::Nominal);
        assert_eq!(b.classify(b.max_centi_c()), MachineState::Nominal);
    }

    #[test]
    fn classify_above_max_is_overheat() {
        assert_eq!(band().classify(8001), MachineState::Overheat);
    }

    #[test]
    fn classify_below_min_is_freeze() {
        assert_eq!(band().classify(-501), MachineState::Freeze);
    }

    #[test]
    fn machine_starts_in_the_state_its_initial_temperature_implies() {
        let m = Machine::new(band(), -501);
        assert_eq!(m.state(), MachineState::Freeze);
    }

    #[test]
    fn machine_state_follows_temperature_while_healthy() {
        let mut m = Machine::new(band(), 0);
        assert_eq!(m.state(), MachineState::Nominal);

        m.set_temperature(9000);
        assert_eq!(m.state(), MachineState::Overheat);

        m.set_temperature(0);
        assert_eq!(m.state(), MachineState::Nominal);

        m.set_temperature(-600);
        assert_eq!(m.state(), MachineState::Freeze);
    }

    #[test]
    fn burnout_is_sticky_until_repaired() {
        let mut m = Machine::new(band(), 0);
        m.burnout();
        assert_eq!(m.state(), MachineState::Burnout);

        m.set_temperature(0); // back in nominal range, but still burned out
        assert_eq!(m.state(), MachineState::Burnout);
    }

    #[test]
    fn offline_is_sticky_until_repaired() {
        let mut m = Machine::new(band(), 0);
        m.take_offline();
        assert_eq!(m.state(), MachineState::Offline);

        m.set_temperature(0);
        assert_eq!(m.state(), MachineState::Offline);
    }

    #[test]
    #[should_panic(expected = "must not exceed max")]
    fn tolerance_band_rejects_inverted_bounds() {
        ToleranceBand::new(100, -100);
    }

    // Bands arrive from RON content and saves, so deserializing must enforce min <= max too.
    #[test]
    fn tolerance_band_deserialize_rejects_inverted_bounds() {
        let parsed = ron::from_str::<ToleranceBand>("(min_centi_c: 100, max_centi_c: -100)");
        assert!(parsed.is_err());
    }

    #[test]
    fn tolerance_band_deserializes_valid_bounds() {
        let parsed: ToleranceBand =
            ron::from_str("(min_centi_c: -500, max_centi_c: 8000)").unwrap();
        assert_eq!(parsed, band());
    }

    #[test]
    fn failed_state_survives_a_save_round_trip() {
        let mut m = Machine::new(band(), 0);
        m.burnout();
        let saved = ron::to_string(&m).unwrap();
        let loaded: Machine = ron::from_str(&saved).unwrap();
        assert_eq!(loaded, m);
        assert_eq!(loaded.state(), MachineState::Burnout);
    }
}
