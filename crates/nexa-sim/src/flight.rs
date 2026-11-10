//! The Slip: launching The Mullein, the outbound leg Sol by Sol, the hazards on
//! the way and what settling them costs, and arrival over the station.
//!
//! While a voyage is in transit the colony is held (`World::tick` does
//! nothing); arrival runs it through every Sol the voyage used. Milestone 5
//! ends there: the ship holds over the station until docking (milestone 7).

use crate::check::{check, Outcome, Roll};
use crate::crew::Crew;
use crate::hazards::{HazardDef, HazardError, HazardsDef, Toll};
use crate::ship::{Ship, ShipLocation};
use crate::universe::Universe;
use crate::{SimEvent, World};
use nexa_types::{CrewId, HazardId, LaunchContext, SlipEvent, StationId, Tick, TICKS_PER_SOL};
use serde::{Deserialize, Serialize};

/// Which way the ship is flying. The return leg comes with milestone 8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Leg {
    Outbound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoyageStatus {
    InTransit,
    /// Arrived; holding over the destination.
    Holding,
}

/// What a settled hazard actually cost, after the cap and what the ship had.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Paid {
    pub fuel_l: u32,
    pub pods: u8,
    pub credits: u32,
    pub crates_dumped: u32,
    pub mass_dumped_kg: u32,
    /// Integrity points every crate lost.
    pub cargo_damage_pct: u8,
    /// Sols added to the voyage.
    pub delay_sols: u32,
    /// Sols of random delay the round-trip cap took off.
    pub delay_cut_sols: u32,
}

impl Paid {
    /// What was paid, one short part per kind, e.g. `["150 L", "+6 Sols"]`.
    /// Sols the cap cut aren't a cost, so they're left to the caller.
    pub fn parts(&self) -> Vec<String> {
        let mut parts = Vec::new();
        if self.fuel_l > 0 {
            parts.push(format!("{} L", self.fuel_l));
        }
        if self.pods > 0 {
            parts.push(format!("{} pod", self.pods));
        }
        if self.credits > 0 {
            parts.push(format!("{} cr", self.credits));
        }
        if self.crates_dumped > 0 {
            parts.push(format!(
                "{} crates ({} kg)",
                self.crates_dumped, self.mass_dumped_kg
            ));
        }
        if self.cargo_damage_pct > 0 {
            parts.push(format!("cargo -{}%", self.cargo_damage_pct));
        }
        if self.delay_sols > 0 {
            parts.push(format!("+{} Sols", self.delay_sols));
        }
        parts
    }
}

/// How a hazard was settled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub hazard: HazardId,
    /// The option's id, e.g. `ecm`.
    pub option: String,
    /// `None` for an option with no check.
    pub roll: Option<Roll>,
    /// A certain option counts as a Success.
    pub outcome: Outcome,
    pub paid: Paid,
}

/// One trip through The Slip. Kept after arrival: the return leg (milestone 8)
/// continues it and shares its random-delay cap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Voyage {
    destination: StationId,
    destination_name: String,
    leg: Leg,
    status: VoyageStatus,
    departed: Tick,
    flight_crew: Vec<CrewId>,
    transit_sols: u32,
    sols_flown: u32,
    /// All Sols added to the leg.
    delay_sols: u32,
    /// Random delay over the round trip so far; never above the cap.
    random_delay_sols: u32,
    /// Hazards still ahead, from the launch briefing.
    ahead: Vec<SlipEvent>,
    pending: Option<HazardId>,
    resolutions: Vec<Resolution>,
    /// The hazard content it launched with, so a save resolves the same.
    rules: HazardsDef,
}

impl Voyage {
    pub fn destination(&self) -> &StationId {
        &self.destination
    }

    pub fn destination_name(&self) -> &str {
        &self.destination_name
    }

    pub fn leg(&self) -> Leg {
        self.leg
    }

    pub fn status(&self) -> VoyageStatus {
        self.status
    }

    pub fn in_transit(&self) -> bool {
        self.status == VoyageStatus::InTransit
    }

    /// Colony time at launch.
    pub fn departed(&self) -> Tick {
        self.departed
    }

    pub fn flight_crew(&self) -> &[CrewId] {
        &self.flight_crew
    }

    pub fn transit_sols(&self) -> u32 {
        self.transit_sols
    }

    pub fn sols_flown(&self) -> u32 {
        self.sols_flown
    }

    pub fn delay_sols(&self) -> u32 {
        self.delay_sols
    }

    pub fn random_delay_sols(&self) -> u32 {
        self.random_delay_sols
    }

    /// The leg's length so far: nominal transit plus delays.
    pub fn sols_total(&self) -> u32 {
        self.transit_sols + self.delay_sols
    }

    pub fn rules(&self) -> &HazardsDef {
        &self.rules
    }

    /// The hazard waiting on a choice, if any.
    pub fn pending(&self) -> Option<&HazardDef> {
        self.pending.as_ref().and_then(|id| self.rules.hazard(id))
    }

    pub fn resolutions(&self) -> &[Resolution] {
        &self.resolutions
    }

    /// Make the next hazard due on this Sol pending.
    fn next_hazard(&mut self) -> Option<HazardId> {
        let i = self.ahead.iter().position(|e| e.sol == self.sols_flown)?;
        let hazard = self.ahead.remove(i).hazard;
        self.pending = Some(hazard.clone());
        Some(hazard)
    }

    /// What a loaded save must satisfy.
    pub(crate) fn check(&self, ship: &Ship, crew: &Crew) -> Result<(), String> {
        self.rules
            .check(crew.roster())
            .map_err(|e| format!("hazard content: {e}"))?;
        // Checks roll for whoever is on the Flight side, so it must be who flew.
        if !crew.flight().map(|c| &c.id).eq(&self.flight_crew) {
            return Err("the flight crew isn't the crew's Flight side".into());
        }
        if let Some(id) = &self.pending {
            if self.rules.hazard(id).is_none() {
                return Err(format!("pending hazard {id} isn't in its content"));
            }
        }
        if let Some(e) = self
            .ahead
            .iter()
            .find(|e| self.rules.hazard(&e.hazard).is_none())
        {
            return Err(format!("hazard {} ahead isn't in its content", e.hazard));
        }
        // A hazard behind the ship would never fire; one on this Sol only
        // waits behind the pending one.
        if let Some(e) = self.ahead.iter().find(|e| {
            e.sol < self.sols_flown || (e.sol == self.sols_flown && self.pending.is_none())
        }) {
            return Err(format!(
                "hazard {} on Sol {} is behind the ship",
                e.hazard, e.sol
            ));
        }
        if self.random_delay_sols > self.rules.delays.random_cap_sols
            || self.random_delay_sols > self.delay_sols
        {
            return Err("random delay is over its cap".into());
        }
        let leg_done = match self.status {
            VoyageStatus::InTransit => self.sols_flown < self.sols_total(),
            VoyageStatus::Holding => {
                self.sols_flown == self.sols_total()
                    && self.pending.is_none()
                    && self.ahead.is_empty()
            }
        };
        if !leg_done {
            return Err("the Sols flown don't fit the voyage's status".into());
        }
        let placed = match (self.status, ship.location()) {
            (VoyageStatus::InTransit, ShipLocation::InSlip) => true,
            (VoyageStatus::Holding, ShipLocation::Holding(at)) => at == &self.destination,
            _ => false,
        };
        if !placed {
            return Err("the ship isn't where the voyage says".into());
        }
        Ok(())
    }
}

/// What one voyage step produced: the voyage's own events, and on arrival
/// the colony's catch-up events, kept apart so the base log can summarize them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VoyageStep {
    pub events: Vec<SimEvent>,
    pub colony: Vec<SimEvent>,
}

impl VoyageStep {
    pub fn arrived(&self) -> bool {
        self.events
            .iter()
            .any(|e| matches!(e, SimEvent::Arrived { .. }))
    }
}

/// Why the ship can't launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    /// She isn't on the pad.
    ShipAway,
    /// Nobody is on the Flight side of a confirmed Split.
    NoFlightCrew,
    NotEnoughFuel {
        need_l: u32,
        have_l: u32,
    },
    BadRules(HazardError),
    /// The universe put something on the route this sim can't fly.
    BadBriefing(String),
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::ShipAway => write!(f, "the mullein is away"),
            LaunchError::NoFlightCrew => write!(f, "no flight crew confirmed"),
            LaunchError::NotEnoughFuel { need_l, have_l } => {
                write!(f, "the leg needs {need_l} L, the tank holds {have_l} L")
            }
            LaunchError::BadRules(e) => write!(f, "hazard content: {e}"),
            LaunchError::BadBriefing(e) => write!(f, "launch briefing: {e}"),
        }
    }
}

impl std::error::Error for LaunchError {}

/// Why a hazard option can't be taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    NoPods,
    NotEnoughFuel,
    NotEnoughCredits,
    NotEnoughCrates,
    /// Would leave too little fuel to fly the next leg home.
    FuelReserve,
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Unavailable::NoPods => "no ecm pods left",
            Unavailable::NotEnoughFuel => "not enough fuel",
            Unavailable::NotEnoughCredits => "not enough credits",
            Unavailable::NotEnoughCrates => "not enough crates in the hold",
            Unavailable::FuelReserve => "would leave too little fuel to get home",
        })
    }
}

/// Why a voyage step or a choice was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlightError {
    NoVoyage,
    /// Arrived and holding: nothing to fly until the return leg (milestone 8).
    NotInTransit,
    HazardPending,
    NoHazardPending,
    NoSuchOption(String),
    Unavailable(Unavailable),
}

impl std::fmt::Display for FlightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlightError::NoVoyage => write!(f, "no voyage under way"),
            FlightError::NotInTransit => write!(f, "holding: the voyage has arrived"),
            FlightError::HazardPending => write!(f, "a hazard needs an answer first"),
            FlightError::NoHazardPending => write!(f, "no hazard to answer"),
            FlightError::NoSuchOption(id) => write!(f, "no option named {id}"),
            FlightError::Unavailable(u) => write!(f, "unavailable: {u}"),
        }
    }
}

impl std::error::Error for FlightError {}

impl World {
    /// Whether the ship and purse can pay `cost` in full, up front, and still
    /// burn the legs left to get home.
    pub fn affordable(&self, cost: &Toll) -> Result<(), Unavailable> {
        let ship = &self.ship;
        if ship.pods() < cost.pods {
            return Err(Unavailable::NoPods);
        }
        let fuel = u32::from(cost.overclock_burns).saturating_mul(ship.def().overclock_burn_l);
        if ship.fuel_l() < fuel {
            return Err(Unavailable::NotEnoughFuel);
        }
        if fuel > 0 && ship.fuel_l() - fuel < self.fuel_reserve_l() {
            return Err(Unavailable::FuelReserve);
        }
        if self.credits < cost.credits {
            return Err(Unavailable::NotEnoughCredits);
        }
        if (ship.cargo().len() as u64) < u64::from(cost.dump_crates) {
            return Err(Unavailable::NotEnoughCrates);
        }
        Ok(())
    }

    /// Fuel to keep back for the legs still to fly after this one.
    fn fuel_reserve_l(&self) -> u32 {
        match self.voyage.as_ref().map(|v| v.leg) {
            Some(Leg::Outbound) => self.ship.def().leg_burn_l,
            None => 0,
        }
    }

    /// Tests and tools only, not a player action: replace the hold with up to
    /// `crates` fresh crates of the smelter's ore, as if it had made them, so
    /// a fixture has a known hold whatever the shipped manifest holds. Only on
    /// the pad; returns how many were loaded.
    #[doc(hidden)]
    pub fn stock_hold(&mut self, crates: u32) -> Result<u32, LaunchError> {
        if self.voyage.is_some() || self.ship.location() != &ShipLocation::Pad {
            return Err(LaunchError::ShipAway);
        }
        self.ship.unload();
        Ok(match self.colony.ore_def() {
            Some(ore) => self.ship.load_crates(crates, ore),
            None => 0,
        })
    }

    /// Launch The Mullein into The Slip with the shipped hazard content.
    pub fn launch(&mut self, universe: &mut dyn Universe) -> Result<Vec<SimEvent>, LaunchError> {
        self.launch_with_rules(universe, HazardsDef::shipped().clone())
    }

    /// Launch with given hazard content (tests and tuning runs). A refused
    /// launch changes nothing.
    pub fn launch_with_rules(
        &mut self,
        universe: &mut dyn Universe,
        rules: HazardsDef,
    ) -> Result<Vec<SimEvent>, LaunchError> {
        if self.voyage.is_some() || self.ship.location() != &ShipLocation::Pad {
            return Err(LaunchError::ShipAway);
        }
        let flight_crew: Vec<CrewId> = self.crew.flight().map(|c| c.id.clone()).collect();
        if flight_crew.is_empty() {
            return Err(LaunchError::NoFlightCrew);
        }
        let need_l = self.ship.def().leg_burn_l;
        let route = self.ship.def().route.clone();
        if self.ship.fuel_l() < need_l {
            return Err(LaunchError::NotEnoughFuel {
                need_l,
                have_l: self.ship.fuel_l(),
            });
        }
        rules
            .check(self.crew.roster())
            .map_err(LaunchError::BadRules)?;
        // Draw on a copy, so a refused briefing leaves the world untouched.
        let mut rng = self.rng.clone();
        let ctx = LaunchContext {
            destination: route.destination.clone(),
            departure: self.now,
            transit_sols: route.transit_sols,
            flight_crew: flight_crew.clone(),
            seed: rng.next_u64(),
        };
        let briefing = universe.on_launch(&ctx);
        for e in &briefing.events {
            if rules.hazard(&e.hazard).is_none() {
                return Err(LaunchError::BadBriefing(format!(
                    "unknown hazard {}",
                    e.hazard
                )));
            }
            // Strictly inside the leg, so a hazard never meets arrival.
            if e.sol == 0 || e.sol >= route.transit_sols {
                return Err(LaunchError::BadBriefing(format!(
                    "{} on Sol {} of a {}-Sol leg",
                    e.hazard, e.sol, route.transit_sols
                )));
            }
        }
        self.rng = rng;
        let fuel_burned_l = self.ship.burn(need_l);
        self.ship.set_location(ShipLocation::InSlip);
        self.voyage = Some(Voyage {
            destination: route.destination.clone(),
            destination_name: route.name,
            leg: Leg::Outbound,
            status: VoyageStatus::InTransit,
            departed: self.now,
            flight_crew: flight_crew.clone(),
            transit_sols: route.transit_sols,
            sols_flown: 0,
            delay_sols: 0,
            random_delay_sols: 0,
            ahead: briefing.events,
            pending: None,
            resolutions: Vec::new(),
            rules,
        });
        Ok(vec![SimEvent::Launched {
            destination: route.destination,
            flight: flight_crew,
            fuel_burned_l,
        }])
    }

    /// Fly one Sol. Refused while a hazard waits on a choice. On the last Sol
    /// the ship arrives and holds, and the colony catches up.
    pub fn advance_voyage(&mut self) -> Result<VoyageStep, FlightError> {
        let voyage = self.voyage.as_mut().ok_or(FlightError::NoVoyage)?;
        if !voyage.in_transit() {
            return Err(FlightError::NotInTransit);
        }
        if voyage.pending.is_some() {
            return Err(FlightError::HazardPending);
        }
        voyage.sols_flown += 1;
        let mut step = VoyageStep {
            events: vec![SimEvent::SlipSolPassed {
                sol: voyage.sols_flown,
                of: voyage.sols_total(),
            }],
            colony: Vec::new(),
        };
        if let Some(hazard) = voyage.next_hazard() {
            step.events.push(SimEvent::HazardEncountered { hazard });
            return Ok(step);
        }
        if voyage.sols_flown < voyage.sols_total() {
            return Ok(step);
        }
        voyage.status = VoyageStatus::Holding;
        let destination = voyage.destination.clone();
        let (sols, delay_sols) = (voyage.sols_total(), voyage.delay_sols);
        self.ship
            .set_location(ShipLocation::Holding(destination.clone()));
        step.colony = self.catch_up(sols);
        step.events.push(SimEvent::Arrived {
            destination,
            sols,
            delay_sols,
        });
        Ok(step)
    }

    /// Run the colony through the Sols the voyage used.
    fn catch_up(&mut self, sols: u32) -> Vec<SimEvent> {
        (0..ticks_in(sols)).flat_map(|_| self.tick()).collect()
    }

    /// Answer the pending hazard with one of its options: pay its cost, roll
    /// its check if it has one, and pay the Partial or Fail toll.
    pub fn resolve_hazard(&mut self, option_id: &str) -> Result<Vec<SimEvent>, FlightError> {
        let voyage = self.voyage.as_ref().ok_or(FlightError::NoVoyage)?;
        let hazard = voyage.pending.clone().ok_or(FlightError::NoHazardPending)?;
        let option = voyage
            .rules
            .hazard(&hazard)
            .and_then(|h| h.option(option_id))
            .cloned()
            .ok_or_else(|| FlightError::NoSuchOption(option_id.to_string()))?;
        let tuning = voyage.rules.checks;
        self.affordable(&option.cost)
            .map_err(FlightError::Unavailable)?;

        let mut paid = Paid::default();
        self.pay(&option.cost, &mut paid);
        let roll = option
            .check
            .as_ref()
            .map(|c| check(&self.crew, c, &tuning, &mut self.rng));
        let outcome = roll.as_ref().map_or(Outcome::Success, |r| r.outcome);
        match outcome {
            Outcome::Success => {}
            Outcome::Partial => self.pay(&option.partial, &mut paid),
            Outcome::Fail => self.pay(&option.fail, &mut paid),
        }
        let resolution = Resolution {
            hazard,
            option: option.id,
            roll,
            outcome,
            paid,
        };
        let mut events = vec![SimEvent::HazardResolved(resolution.clone())];
        if let Some(voyage) = self.voyage.as_mut() {
            voyage.pending = None;
            voyage.resolutions.push(resolution);
            // Another hazard due on the same Sol comes straight after.
            if let Some(hazard) = voyage.next_hazard() {
                events.push(SimEvent::HazardEncountered { hazard });
            }
        }
        Ok(events)
    }

    /// Take a toll from the ship, the purse and the voyage, as far as they go.
    fn pay(&mut self, toll: &Toll, paid: &mut Paid) {
        let overclock = u32::from(toll.overclock_burns) * self.ship.def().overclock_burn_l;
        paid.fuel_l += self.ship.burn(overclock);
        paid.pods += self.ship.spend_pods(toll.pods);
        let credits = toll.credits.min(self.credits);
        self.credits -= credits;
        paid.credits += credits;
        let (crates, kg) = self.ship.dump_crates(toll.dump_crates);
        paid.crates_dumped += crates;
        paid.mass_dumped_kg += kg;
        if toll.cargo_damage_pct > 0 {
            self.ship.damage_cargo(toll.cargo_damage_pct);
            paid.cargo_damage_pct = paid.cargo_damage_pct.saturating_add(toll.cargo_damage_pct);
        }
        if let (Some(kind), Some(voyage)) = (toll.delay, self.voyage.as_mut()) {
            let (lo, hi) = voyage.rules.delays.range(kind);
            let drawn = self.rng.range(lo, hi);
            let room = voyage
                .rules
                .delays
                .random_cap_sols
                .saturating_sub(voyage.random_delay_sols);
            let added = drawn.min(room);
            voyage.random_delay_sols += added;
            voyage.delay_sols += added;
            paid.delay_sols += added;
            paid.delay_cut_sols += drawn - added;
        }
    }
}

/// Ticks in `sols` Sols.
pub fn ticks_in(sols: u32) -> u64 {
    u64::from(sols) * TICKS_PER_SOL
}

#[cfg(test)]
mod tests;
