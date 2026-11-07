//! Everything off-colony is reached through [`Universe`]. Offline first: the
//! [`OfflineUniverse`] stands in for the shared universe, seeded and
//! deterministic. Only the launch sync point exists so far; docking, trades
//! and posts arrive with the systems behind them.

use crate::hazards::HazardsDef;
use crate::rng::SimRng;
use nexa_types::{HazardId, LaunchBriefing, LaunchContext, SlipEvent};

pub trait Universe {
    /// A ship enters The Slip: what waits on the route.
    fn on_launch(&mut self, ctx: &LaunchContext) -> LaunchBriefing;
}

/// The single-player universe. Holds no state between calls: everything it
/// decides comes from the launch's seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineUniverse {
    /// Hazards it may put on a route.
    hazards: Vec<HazardId>,
}

impl OfflineUniverse {
    pub fn new(hazards: Vec<HazardId>) -> Self {
        Self { hazards }
    }
}

impl Default for OfflineUniverse {
    /// Draws from the shipped hazards.
    fn default() -> Self {
        Self::new(
            HazardsDef::shipped()
                .hazards
                .iter()
                .map(|h| h.id.clone())
                .collect(),
        )
    }
}

impl Universe for OfflineUniverse {
    /// One hazard per leg, on a Sol strictly inside it so it hits mid-transit.
    fn on_launch(&mut self, ctx: &LaunchContext) -> LaunchBriefing {
        let mut rng = SimRng::new(ctx.seed);
        if self.hazards.is_empty() || ctx.transit_sols < 2 {
            return LaunchBriefing::default();
        }
        let sol = rng.range(1, ctx.transit_sols - 1);
        let pick = rng.range(0, self.hazards.len() as u32 - 1) as usize;
        LaunchBriefing {
            events: vec![SlipEvent {
                sol,
                hazard: self.hazards[pick].clone(),
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexa_types::{CrewId, StationId, Tick};

    fn ctx(seed: u64, transit_sols: u32) -> LaunchContext {
        LaunchContext {
            destination: StationId::new("barge_77"),
            departure: Tick(260),
            transit_sols,
            flight_crew: vec![CrewId::new("cal")],
            seed,
        }
    }

    #[test]
    fn schedules_the_interdiction_inside_the_leg() {
        let mut universe = OfflineUniverse::default();
        let mut sols = Vec::new();
        for seed in 0..100 {
            let briefing = universe.on_launch(&ctx(seed, 5));
            assert_eq!(briefing.events.len(), 1);
            let event = &briefing.events[0];
            assert_eq!(event.hazard, HazardId::new("pirate_interdiction"));
            assert!((1..=4).contains(&event.sol), "{}", event.sol);
            sols.push(event.sol);
        }
        for sol in 1..=4 {
            assert!(sols.contains(&sol), "never on Sol {sol}");
        }
    }

    #[test]
    fn same_seed_same_briefing() {
        let mut a = OfflineUniverse::default();
        let mut b = OfflineUniverse::default();
        for seed in 0..20 {
            assert_eq!(a.on_launch(&ctx(seed, 5)), b.on_launch(&ctx(seed, 5)));
        }
    }

    #[test]
    fn a_quiet_universe_or_a_one_sol_hop_has_no_hazards() {
        assert!(OfflineUniverse::new(Vec::new())
            .on_launch(&ctx(1, 5))
            .events
            .is_empty());
        assert!(OfflineUniverse::default()
            .on_launch(&ctx(1, 1))
            .events
            .is_empty());
    }
}
