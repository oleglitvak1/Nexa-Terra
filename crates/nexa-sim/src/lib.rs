//! Nexa Terra simulation core.
//!
//! Rules for this crate (see CLAUDE.md):
//! - No graphics, audio, input, file system, threads or wall-clock time.
//! - Deterministic: the same starting world and commands always give the same result.
//! - Integer or fixed-point math for anything economic or shared.
//! - Must compile for `wasm32-unknown-unknown`.

use colony::{Colony, CommandError, LANDING_ASH_PLAIN};
use crew::{Assignment, Crew, CrewError, Roster, SplitError, StressLevel};
use flight::{Resolution, Voyage};
use machines::MachineState;
use nexa_types::{Command, CrewId, HazardId, ModuleId, StationId, Tick, TICKS_PER_SOL};
use orders::{Action, OrderError, Orders, Preset, PresetError, SkipReason, StandingOrder};
use rng::SimRng;
use serde::{Deserialize, Serialize};
use ship::{Ship, ShipDef, ShipError, ShipLocation};
use std::collections::BTreeMap;

pub mod check;
pub mod colony;
pub mod crew;
pub mod flight;
pub mod grid;
pub mod hazards;
pub mod machines;
pub mod orders;
pub mod parts;
pub mod rng;
pub mod script;
pub mod ship;
pub mod universe;

/// The whole state of one player's game. Grows milestone by milestone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawWorld")]
pub struct World {
    /// Current simulation time.
    pub now: Tick,
    pub colony: Colony,
    /// Standing Orders; add cards through [`World::add_order`] so they're checked.
    pub orders: Orders,
    /// Change the Split through [`World::confirm_split`] so it's checked.
    pub crew: Crew,
    /// Every random roll the sim makes draws from here.
    pub rng: SimRng,
    /// The Mullein; flown through [`World::launch`].
    pub ship: Ship,
    /// The colony's purse, in whole credits.
    pub credits: u32,
    /// The trip through The Slip, from launch on; driven through
    /// [`World::advance_voyage`] and [`World::resolve_hazard`].
    pub voyage: Option<Voyage>,
}

/// Unchecked wire form of [`World`]; serde checks it via `TryFrom`.
#[derive(Deserialize)]
struct RawWorld {
    now: Tick,
    colony: Colony,
    /// Saves from before Standing Orders have none.
    #[serde(default)]
    orders: Orders,
    /// Saves from before crew have none.
    #[serde(default, deserialize_with = "present")]
    crew: Option<Crew>,
    /// Saves from before the RNG start from the fixed default seed.
    #[serde(default)]
    rng: SimRng,
    /// Saves from before the ship get the shipped Mullein on the pad...
    #[serde(default, deserialize_with = "present")]
    ship: Option<Ship>,
    /// ... and the shipped starting credits.
    #[serde(default, deserialize_with = "present")]
    credits: Option<u32>,
    /// Saves from before The Slip have no voyage.
    #[serde(default)]
    voyage: Option<Voyage>,
}

/// Writes a present optional field bare, for [`present`] to read back. Pair it
/// with `skip_serializing_if = "Option::is_none"`.
pub(crate) fn bare<S, T>(value: &Option<T>, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    T: Serialize,
{
    match value {
        Some(v) => v.serialize(s),
        None => s.serialize_none(),
    }
}

/// Reads a field a save writes bare, not as an RON `Some(..)`.
pub(crate) fn present<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}

/// Why a saved world was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    Orders(OrderError),
    Crew(CrewError),
    Ship(ShipError),
    Voyage(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Orders(e) => write!(f, "standing orders: {e}"),
            LoadError::Crew(e) => write!(f, "crew: {e}"),
            LoadError::Ship(e) => write!(f, "ship: {e}"),
            LoadError::Voyage(e) => write!(f, "voyage: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl TryFrom<RawWorld> for World {
    type Error = LoadError;

    fn try_from(raw: RawWorld) -> Result<Self, Self::Error> {
        let mut colony = raw.colony;
        raw.orders.check(&colony).map_err(LoadError::Orders)?;
        let crew = match raw.crew {
            Some(crew) => crew,
            None => {
                // Pre-crew saves also predate ModuleKind, so their habs load as
                // Utility and the shipped bunks would be refused.
                colony.mark_legacy_habs();
                Crew::new(Roster::shipped(&colony).map_err(LoadError::Crew)?)
            }
        };
        crew.check(&colony).map_err(LoadError::Crew)?;
        let ship = raw.ship.unwrap_or_default();
        ship.check().map_err(LoadError::Ship)?;
        match &raw.voyage {
            Some(voyage) => voyage.check(&ship, &crew).map_err(LoadError::Voyage)?,
            None if ship.location() != &ShipLocation::Pad => {
                return Err(LoadError::Voyage("the ship is away with no voyage".into()))
            }
            None => {}
        }
        let credits = raw.credits.unwrap_or(ShipDef::shipped().starting_credits);
        Ok(Self {
            now: raw.now,
            colony,
            orders: raw.orders,
            crew,
            rng: raw.rng,
            ship,
            credits,
            voyage: raw.voyage,
        })
    }
}

/// Something that happened during a tick, for the front end to display or sound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimEvent {
    /// A new Sol has begun.
    SolStarted {
        sol: u64,
    },
    MachineStateChanged {
        module: ModuleId,
        state: MachineState,
    },
    /// A Standing Order's triggers all held; its actions follow as their own events.
    OrderFired {
        slot: usize,
        order: StandingOrder,
    },
    /// One action of a fired card did nothing.
    OrderActionSkipped {
        slot: usize,
        action: Action,
        reason: SkipReason,
    },
    /// A Split passed its rules and is now the crew's assignments.
    SplitConfirmed {
        flight: Vec<CrewId>,
    },
    /// A full crate of the smelter's ore went aboard *The Mullein* on the
    /// pad; `crates` is how many she holds now.
    OreCrateLoaded {
        id: String,
        crates: u32,
    },
    /// A crew member's stress reached a new level.
    CrewStressChanged {
        crew: CrewId,
        level: StressLevel,
    },
    /// The Mullein left the pad for The Slip.
    Launched {
        destination: StationId,
        flight: Vec<CrewId>,
        fuel_burned_l: u32,
    },
    /// One Sol of the leg flown, of `of` (which grows with delays).
    SlipSolPassed {
        sol: u32,
        of: u32,
    },
    /// A hazard is waiting on a choice.
    HazardEncountered {
        hazard: HazardId,
    },
    HazardResolved(Resolution),
    /// The leg is over; the ship holds over the station.
    Arrived {
        destination: StationId,
        sols: u32,
        delay_sols: u32,
    },
}

impl World {
    /// A new world with no orders and everyone on the Ground.
    /// `roster` must already be checked against `colony`.
    pub fn new(colony: Colony, roster: Roster) -> Self {
        Self {
            now: Tick::default(),
            colony,
            orders: Orders::default(),
            crew: Crew::new(roster),
            rng: SimRng::default(),
            ship: Ship::default(),
            credits: ShipDef::shipped().starting_credits,
            voyage: None,
        }
    }

    /// The vertical slice's start: landfall on the Ash Plain.
    pub fn landfall() -> Self {
        // The shipped content is checked by colony's and crew's tests, so failing
        // here is a build bug.
        let colony = Colony::from_ron(LANDING_ASH_PLAIN).expect("landing site content is valid");
        let roster = Roster::shipped(&colony).expect("crew content is valid");
        Self::new(colony, roster)
    }

    /// Landfall with its own seed, so each new game rolls differently.
    pub fn landfall_seeded(seed: u64) -> Self {
        Self {
            rng: SimRng::new(seed),
            ..Self::landfall()
        }
    }

    /// Advance the simulation by one fixed step (one planetary hour).
    /// Held while a voyage is in transit: arrival catches the colony up.
    pub fn tick(&mut self) -> Vec<SimEvent> {
        if self.voyage.as_ref().is_some_and(Voyage::in_transit) {
            return Vec::new();
        }
        // The Caretaker reads the gauges and acts, then the hour runs.
        let mut events = self.orders.evaluate(&mut self.colony);
        events.extend(self.colony.tick());
        events.extend(self.load_ore());
        // The crew feel the hour that just ran.
        let colony = &self.colony;
        events.extend(self.crew.step(|bunk| {
            colony
                .module(bunk)
                .and_then(|m| m.machine())
                .map(|m| m.state())
        }));
        self.now = Tick(self.now.0 + 1);
        if self.now.0.is_multiple_of(TICKS_PER_SOL) {
            events.push(SimEvent::SolStarted {
                sol: self.now.sol(),
            });
        }
        events
    }

    /// While she is on the pad, every full crate in the stockpile goes aboard,
    /// as far as her hold has room; the rest waits on the ground.
    fn load_ore(&mut self) -> Vec<SimEvent> {
        let Some(ore) = self.colony.ore_def() else {
            return Vec::new();
        };
        if self.ship.location() != &ShipLocation::Pad {
            return Vec::new();
        }
        let loaded = self.ship.load_crates(self.colony.full_crates(), ore);
        self.colony.pack_crates(loaded);
        let cargo = self.ship.cargo();
        let first = cargo.len() - loaded as usize;
        cargo[first..]
            .iter()
            .enumerate()
            .map(|(k, c)| SimEvent::OreCrateLoaded {
                id: c.id.clone(),
                crates: (first + k + 1) as u32,
            })
            .collect()
    }

    pub fn apply(&mut self, cmd: &Command) -> Result<Vec<SimEvent>, CommandError> {
        self.colony.apply(cmd)
    }

    /// Store a Standing Order in the next free slot, checked against this colony.
    pub fn add_order(&mut self, order: StandingOrder) -> Result<usize, OrderError> {
        self.orders.add(order, &self.colony)
    }

    /// Load a posture: its cards replace the current ones, each checked as
    /// [`Self::add_order`] checks it, and its guards are checked against the
    /// colony (they are not applied yet). A refused posture changes nothing.
    pub fn load_posture(&mut self, posture: &Preset) -> Result<(), PresetError> {
        self.orders = posture.orders(&self.colony, self.orders.tuning())?;
        Ok(())
    }

    /// Take a Standing Order out of its slot; later cards move up.
    pub fn remove_order(&mut self, slot: usize) -> Result<StandingOrder, OrderError> {
        self.orders.remove(slot)
    }

    /// Decide who flies. Crew left out of `assignments` stay on the Ground; a
    /// refused Split leaves the current one in place, and so does any Split
    /// while the ship is away.
    pub fn confirm_split(
        &mut self,
        assignments: &BTreeMap<CrewId, Assignment>,
    ) -> Result<Vec<SimEvent>, SplitError> {
        if self.ship.location() != &ShipLocation::Pad {
            return Err(SplitError::ShipAway);
        }
        let flight = self.crew.confirm_split(assignments)?;
        Ok(vec![SimEvent::SplitConfirmed { flight }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orders::OrdersDef;

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

    fn with_preset(name: &str) -> World {
        let mut world = World::landfall();
        world.load_posture(posture(name)).unwrap();
        world
    }

    fn posture(name: &str) -> &'static Preset {
        &OrdersDef::shipped().presets[name]
    }

    fn cards(world: &World) -> Vec<StandingOrder> {
        world.orders.iter().cloned().collect()
    }

    #[test]
    fn loading_a_posture_replaces_the_cards() {
        let mut world = with_preset("doc-example");
        world.load_posture(posture("protect-habs")).unwrap();
        assert_eq!(cards(&world), posture("protect-habs").cards);
    }

    #[test]
    fn a_refused_posture_changes_nothing() {
        let mut world = with_preset("doc-example");
        run_sols(&mut world, 3);
        let before = world.clone();
        let mut ghost = posture("protect-habs").clone();
        ghost.cards[2].actions = vec![Action::DivertHeat {
            to: ModuleId::new("ghost"),
        }];
        assert_eq!(
            world.load_posture(&ghost),
            Err(PresetError::Card(OrderError::UnknownModule(ModuleId::new(
                "ghost"
            ))))
        );
        let mut too_many = posture("protect-habs").clone();
        let memory = usize::from(world.orders.tuning().memory);
        too_many.cards = vec![too_many.cards[0].clone(); memory + 1];
        assert!(matches!(
            world.load_posture(&too_many),
            Err(PresetError::Card(OrderError::MemoryFull { .. }))
        ));
        let mut bad_guard = posture("protect-habs").clone();
        bad_guard.guards.insert(ModuleId::new("ghost"), None);
        assert!(matches!(
            world.load_posture(&bad_guard),
            Err(PresetError::Guard(CommandError::UnknownModule(_)))
        ));
        assert_eq!(world, before);
    }

    #[test]
    fn protect_habs_keeps_both_habs_nominal_to_sol_10() {
        let mut world = with_preset("doc-example");
        world.load_posture(posture("protect-habs")).unwrap();
        run_sols(&mut world, 10);
        for hab in ["hab_01", "hab_02"] {
            let m = world.colony.module(&ModuleId::new(hab)).unwrap();
            assert_eq!(m.machine().unwrap().state(), MachineState::Nominal, "{hab}");
        }
    }

    fn run_sols(world: &mut World, sols: u64) -> Vec<SimEvent> {
        (0..sols * TICKS_PER_SOL)
            .flat_map(|_| world.tick())
            .collect()
    }

    #[test]
    fn orders_and_their_firing_state_survive_a_save() {
        let mut world = with_preset("protect-habs");
        // Mid-run, so some cards are disarmed and a heat diversion is live.
        run_sols(&mut world, 3);
        let saved = ron::to_string(&world).unwrap();
        let mut loaded: World = ron::from_str(&saved).unwrap();
        assert_eq!(loaded, world);
        assert_eq!(run_sols(&mut loaded, 2), run_sols(&mut world, 2));
    }

    #[test]
    fn saves_from_before_standing_orders_still_load() {
        #[derive(Serialize)]
        struct OldWorld<'a> {
            now: Tick,
            colony: &'a Colony,
        }
        let world = World::landfall();
        let old = ron::to_string(&OldWorld {
            now: world.now,
            colony: &world.colony,
        })
        .unwrap();
        let loaded: World = ron::from_str(&old).unwrap();
        assert!(loaded.orders.is_empty());
        assert_eq!(loaded, world);
    }

    #[test]
    fn a_save_with_a_bad_card_is_rejected() {
        let world = with_preset("protect-habs");
        let saved = ron::to_string(&world).unwrap();
        let ghost = saved.replace("\"hab_02\")", "\"ghost\")");
        assert_ne!(ghost, saved);
        assert!(ron::from_str::<World>(&ghost).is_err());
        let tiny_memory = saved.replace("memory:8", "memory:2");
        assert_ne!(tiny_memory, saved);
        assert!(ron::from_str::<World>(&tiny_memory).is_err());
    }

    #[test]
    fn removing_an_order_moves_later_cards_up() {
        let mut world = with_preset("protect-habs");
        let third = world.orders.get(2).unwrap().clone();
        let first = world.remove_order(0).unwrap();
        assert_eq!(first, OrdersDef::shipped().presets["protect-habs"].cards[0]);
        assert_eq!(world.orders.len(), 2);
        assert_eq!(world.orders.get(1), Some(&third));
        assert_eq!(world.remove_order(2), Err(OrderError::NoSuchSlot(2)));
    }

    #[test]
    fn orders_are_deterministic() {
        let mut a = with_preset("caretaker");
        let mut b = with_preset("caretaker");
        for _ in 0..10 * TICKS_PER_SOL {
            assert_eq!(a.tick(), b.tick());
        }
        assert_eq!(a, b);
    }

    fn split(flight: &[&str]) -> BTreeMap<CrewId, Assignment> {
        flight
            .iter()
            .map(|id| (CrewId::new(*id), Assignment::Flight))
            .collect()
    }

    fn flying(world: &World) -> Vec<&str> {
        world
            .crew
            .members()
            .iter()
            .filter(|c| world.crew.assignment(&c.id) == Some(Assignment::Flight))
            .map(|c| c.id.as_str())
            .collect()
    }

    #[test]
    fn landfall_has_the_shipped_crew_all_on_the_ground() {
        let world = World::landfall();
        assert_eq!(world.crew.members().len(), crew::CREW_SIZE);
        assert_eq!(flying(&world), Vec::<&str>::new());
    }

    #[test]
    fn a_confirmed_split_is_recorded_and_announced() {
        let mut world = World::landfall();
        let events = world.confirm_split(&split(&["cal", "oron"])).unwrap();
        assert_eq!(
            events,
            vec![SimEvent::SplitConfirmed {
                flight: vec![CrewId::new("cal"), CrewId::new("oron")],
            }]
        );
        assert_eq!(flying(&world), ["cal", "oron"]);
        assert_eq!(
            world.crew.assignment(&CrewId::new("maya")),
            Some(Assignment::Ground)
        );
    }

    #[test]
    fn a_refused_split_changes_nothing() {
        let mut world = World::landfall();
        world.confirm_split(&split(&["cal"])).unwrap();
        let before = world.clone();
        let refusals = [
            (split(&["dax"]), SplitError::NoQualifiedPilot),
            (split(&[]), SplitError::NoQualifiedPilot),
            (
                split(&["cal", "jonathan", "elena", "oron", "dax"]),
                SplitError::MayaWithoutAdult {
                    child: CrewId::new("maya"),
                },
            ),
            (
                split(&["cal", "ghost"]),
                SplitError::UnknownCrew(CrewId::new("ghost")),
            ),
        ];
        for (assignments, err) in refusals {
            assert_eq!(world.confirm_split(&assignments), Err(err));
            assert_eq!(world, before);
        }
    }

    #[test]
    fn reconfirming_replaces_the_split() {
        let mut world = World::landfall();
        world
            .confirm_split(&split(&["cal", "dax", "oron"]))
            .unwrap();
        // Crew left out of the new split go back to the Ground.
        world.confirm_split(&split(&["cal"])).unwrap();
        assert_eq!(flying(&world), ["cal"]);
    }

    #[test]
    fn the_split_survives_a_save() {
        let mut world = World::landfall();
        world.confirm_split(&split(&["cal", "oron"])).unwrap();
        let saved = ron::to_string(&world).unwrap();
        assert_eq!(ron::from_str::<World>(&saved).unwrap(), world);
    }

    /// A save written before crew existed, and before modules had a `kind`.
    fn pre_crew_save() -> String {
        #[derive(Serialize)]
        struct OldWorld<'a> {
            now: Tick,
            colony: &'a Colony,
            orders: &'a Orders,
        }
        let world = World::landfall();
        let old = ron::to_string(&OldWorld {
            now: world.now,
            colony: &world.colony,
            orders: &world.orders,
        })
        .unwrap();
        let stripped = old.replace("kind:Hab,", "").replace("kind:Utility,", "");
        assert!(!stripped.contains("kind:"), "kind format changed: {old}");
        stripped
    }

    #[test]
    fn the_rng_survives_a_save_mid_sequence() {
        let mut world = World::landfall_seeded(99);
        world.rng.next_u64();
        let mut loaded: World = ron::from_str(&ron::to_string(&world).unwrap()).unwrap();
        assert_eq!(loaded, world);
        assert_eq!(loaded.rng.next_u64(), world.rng.next_u64());
    }

    #[test]
    fn seeds_change_only_the_rng() {
        let mut a = World::landfall_seeded(1);
        let b = World::landfall_seeded(2);
        assert_ne!(a, b);
        a.rng = b.rng.clone();
        assert_eq!(a, b);
        assert_eq!(
            World::landfall_seeded(SimRng::DEFAULT_SEED),
            World::landfall()
        );
    }

    #[test]
    fn saves_from_before_the_rng_get_the_default_seed() {
        #[derive(Serialize)]
        struct OldWorld<'a> {
            now: Tick,
            colony: &'a Colony,
            orders: &'a Orders,
            crew: &'a Crew,
        }
        let world = World::landfall_seeded(5);
        let old = ron::to_string(&OldWorld {
            now: world.now,
            colony: &world.colony,
            orders: &world.orders,
            crew: &world.crew,
        })
        .unwrap();
        let loaded: World = ron::from_str(&old).unwrap();
        assert_eq!(loaded.rng, SimRng::default());
        assert_eq!(loaded, World::landfall());
    }

    /// Every shipped content file parses and passes its cross-reference checks.
    #[test]
    fn all_shipped_content_loads() {
        let colony = Colony::from_ron(LANDING_ASH_PLAIN).unwrap();
        let roster = Roster::shipped(&colony).unwrap();
        OrdersDef::from_ron(orders::STANDING_ORDERS).unwrap();
        ShipDef::from_ron(ship::MULLEIN).unwrap();
        hazards::HazardsDef::from_ron(hazards::HAZARDS, &roster).unwrap();
    }

    #[test]
    fn landfall_has_the_mullein_on_the_pad_and_the_starting_purse() {
        let world = World::landfall();
        assert_eq!(world.ship.location(), &ship::ShipLocation::Pad);
        assert_eq!(world.ship.fuel_l(), world.ship.def().tank_l);
        assert_eq!(world.credits, ShipDef::shipped().starting_credits);
    }

    #[test]
    fn a_save_with_a_bad_ship_is_rejected() {
        let saved = ron::to_string(&World::landfall()).unwrap();
        let reject = |from: &str, to: &str| {
            let edited = saved.replacen(from, to, 1);
            assert_ne!(edited, saved, "{from} not in save");
            assert!(ron::from_str::<World>(&edited).is_err(), "{from} -> {to}");
        };
        reject(",fuel_l:1000", ",fuel_l:1001");
        reject(",pods:1", ",pods:2");
        reject("integrity_pct:100", "integrity_pct:101");
    }

    #[test]
    fn saves_from_before_crew_get_the_shipped_crew_and_their_habs() {
        let loaded: World = ron::from_str(&pre_crew_save()).unwrap();
        assert_eq!(loaded, World::landfall());
    }

    #[test]
    fn a_save_with_a_bad_crew_is_rejected() {
        let mut world = World::landfall();
        world.confirm_split(&split(&["cal", "oron"])).unwrap();
        let saved = ron::to_string(&world).unwrap();
        let reject = |from: &str, to: &str| {
            let edited = saved.replacen(from, to, 1);
            assert_ne!(edited, saved, "{from} not in save");
            assert!(ron::from_str::<World>(&edited).is_err(), "{from} -> {to}");
        };
        // A bunk that isn't a hab.
        reject("bunk:\"hab_01\"", "bunk:\"smelter_01\"");
        // An assignment for someone not in the crew.
        reject("\"dax\":Ground", "\"ghost\":Ground");
        // A crew member with no assignment.
        reject("\"dax\":Ground,", "");
        // A flight that breaks the Split rules: Oron-3 can't fly alone.
        reject("\"cal\":Flight", "\"cal\":Ground");
        // Stress past its range, and a level its stress can't be at.
        reject("stress_bp:0", "stress_bp:10001");
        reject("level:Calm", "level:Breaking");
    }

    fn hab_failures(events: &[SimEvent]) -> Vec<&SimEvent> {
        events
            .iter()
            .filter(|e| match e {
                SimEvent::MachineStateChanged { module, state } => {
                    module.as_str().starts_with("hab_")
                        && matches!(state, MachineState::Offline | MachineState::Burnout)
                }
                _ => false,
            })
            .collect()
    }

    #[test]
    fn unattended_ash_plain_loses_both_habs_by_sol_10() {
        let mut world = World::landfall();
        let events = run_sols(&mut world, 10);
        assert_eq!(hab_failures(&events).len(), 2);
    }

    fn stress_levels(events: &[SimEvent], who: &str) -> Vec<StressLevel> {
        events
            .iter()
            .filter_map(|e| match e {
                SimEvent::CrewStressChanged { crew, level } if crew.as_str() == who => Some(*level),
                _ => None,
            })
            .collect()
    }

    /// With nobody tending the habs, everyone strains and then breaks, each
    /// announced once, and Father's Vigilance breaks Jonathan first.
    #[test]
    fn unattended_ash_plain_breaks_the_crew() {
        let mut world = World::landfall();
        let events = run_sols(&mut world, 10);
        for c in world.crew.members() {
            assert_eq!(
                stress_levels(&events, c.id.as_str()),
                [StressLevel::Strained, StressLevel::Breaking],
                "{}",
                c.id
            );
            assert!(world.crew.vitals(&c.id).unwrap().morale_bp < crew::MAX_BP);
        }
        let first_break = events
            .iter()
            .find(|e| {
                matches!(
                    e,
                    SimEvent::CrewStressChanged {
                        level: StressLevel::Breaking,
                        ..
                    }
                )
            })
            .unwrap();
        assert!(matches!(
            first_break,
            SimEvent::CrewStressChanged { crew, .. } if crew.as_str() == "jonathan"
        ));
    }

    // --- Ore: a test colony whose smelter makes `kg_per_hour` into 1,000 kg
    // crates, with the shipped crew in two warm habs and an empty hold.

    fn ore_world(kg_per_hour: u32) -> World {
        let colony = Colony::from_ron(&format!(
            r#"(
                ambient_centi_c: 2000, heat_loss_percent: 100, layout: ["GSHh", "===="],
                modules: [
                    (id: "gen", glyph: 'G', output_kw: 20),
                    (id: "smelter_01", glyph: 'S', draw_kw: 5, machine: Some((
                        tolerance: (min_centi_c: 0, max_centi_c: 5000), wear_bp_per_hour: 1)),
                        ore: Some((kg_per_hour: {kg_per_hour}, crate_kg: 1000, cargo: "raw silicon"))),
                    (id: "hab_01", kind: Hab, glyph: 'H', machine: Some((
                        tolerance: (min_centi_c: 0, max_centi_c: 5000), wear_bp_per_hour: 1))),
                    (id: "hab_02", kind: Hab, glyph: 'h', machine: Some((
                        tolerance: (min_centi_c: 0, max_centi_c: 5000), wear_bp_per_hour: 1))),
                ],
            )"#
        ))
        .unwrap();
        let roster = Roster::shipped(&colony).unwrap();
        let mut world = World::new(colony, roster);
        world.ship.unload();
        world
    }

    fn run_hours(world: &mut World, hours: u64) -> Vec<SimEvent> {
        (0..hours).flat_map(|_| world.tick()).collect()
    }

    fn loaded(events: &[SimEvent]) -> Vec<(String, u32)> {
        events
            .iter()
            .filter_map(|e| match e {
                SimEvent::OreCrateLoaded { id, crates } => Some((id.clone(), *crates)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_full_crate_of_ore_is_loaded_the_hour_it_is_made() {
        let mut world = ore_world(250);
        assert!(loaded(&run_hours(&mut world, 3)).is_empty());
        assert_eq!((world.colony.ore_kg(), world.ship.cargo().len()), (750, 0));
        // The fourth hour makes exactly one crate's worth.
        let events = run_hours(&mut world, 1);
        assert_eq!(loaded(&events), [("crate_01".to_string(), 1)]);
        assert_eq!((world.colony.ore_kg(), world.ship.cargo().len()), (0, 1));
        let c = &world.ship.cargo()[0];
        assert_eq!(
            (c.cargo.as_str(), c.mass_kg, c.integrity_pct),
            ("raw silicon", 1000, 100)
        );
        run_hours(&mut world, 5);
        assert_eq!((world.colony.ore_kg(), world.ship.cargo().len()), (250, 2));
    }

    #[test]
    fn the_hold_takes_only_what_it_has_room_for() {
        // Five crates an hour into a ten-crate hold.
        let mut world = ore_world(5000);
        let events = run_hours(&mut world, 3);
        let ids: Vec<u32> = loaded(&events).iter().map(|(_, n)| *n).collect();
        assert_eq!(ids, (1..=10).collect::<Vec<_>>());
        assert_eq!(world.ship.hold_room(), 0);
        assert_eq!(world.colony.ore_kg(), 5000);
        // Full: the ore keeps piling up on the ground.
        assert!(loaded(&run_hours(&mut world, 2)).is_empty());
        assert_eq!(world.colony.ore_kg(), 15_000);
        assert_eq!(world.ship.check(), Ok(()));
    }

    #[test]
    fn ore_stockpiles_while_she_is_away_and_loads_when_she_is_back() {
        let mut world = ore_world(500);
        world
            .ship
            .set_location(ShipLocation::Holding(StationId::new("barge_77")));
        assert!(loaded(&run_hours(&mut world, 10)).is_empty());
        assert_eq!((world.colony.ore_kg(), world.ship.cargo().len()), (5000, 0));
        // Milestone 8 flies her home; here she is simply put back on the pad.
        world.ship.set_location(ShipLocation::Pad);
        let events = run_hours(&mut world, 1);
        assert_eq!(loaded(&events).len(), 5);
        assert_eq!((world.colony.ore_kg(), world.ship.cargo().len()), (500, 5));
    }

    #[test]
    fn ore_is_deterministic_and_survives_a_save() {
        let mut a = ore_world(370);
        let mut b = ore_world(370);
        for _ in 0..200 {
            assert_eq!(a.tick(), b.tick());
        }
        assert_eq!(a, b);
        assert!(a.colony.ore_kg() > 0 && !a.ship.cargo().is_empty());
        let mut loaded: World = ron::from_str(&ron::to_string(&a).unwrap()).unwrap();
        assert_eq!(loaded, a);
        assert_eq!(run_hours(&mut loaded, 50), run_hours(&mut a, 50));
    }

    #[test]
    fn on_shipped_content_the_full_hold_leaves_ore_in_the_stockpile() {
        let mut world = World::landfall();
        let events = run_hours(&mut world, 26);
        assert!(loaded(&events).is_empty());
        assert_eq!(world.ship.cargo().len(), 10);
        // The smelter runs Nominal all Sol 0 at the shipped rate.
        let rate = world.colony.ore_def().unwrap().kg_per_hour;
        assert_eq!(world.colony.ore_kg(), 26 * rate);
    }

    #[test]
    fn caretaker_orders_keep_the_crew_calm() {
        let mut world = with_preset("caretaker");
        let events = run_sols(&mut world, 45);
        assert!(!events
            .iter()
            .any(|e| matches!(e, SimEvent::CrewStressChanged { .. })));
        for c in world.crew.members() {
            let v = world.crew.vitals(&c.id).unwrap();
            assert_eq!((v.stress_bp, v.morale_bp), (0, crew::MAX_BP), "{}", c.id);
        }
    }

    /// The Caretaker scenario: three cards keep both habs alive through Sol 10.
    /// The price is the smelter, which is shut down and left to freeze.
    #[test]
    fn caretaker_orders_keep_both_habs_through_sol_10() {
        let mut world = with_preset("caretaker");
        let events = run_sols(&mut world, 10);
        assert_eq!(hab_failures(&events), Vec::<&SimEvent>::new());
        for hab in ["hab_01", "hab_02"] {
            let m = world.colony.module(&ModuleId::new(hab)).unwrap();
            let machine = m.machine().unwrap();
            assert_eq!(machine.state(), MachineState::Nominal, "{hab}");
        }
        // Landing damage only: the habs never left their band.
        let wear = |hab: &str| {
            let m = world.colony.module(&ModuleId::new(hab)).unwrap();
            m.machine().unwrap().wear_bp()
        };
        assert_eq!((wear("hab_01"), wear("hab_02")), (4000, 0));
        assert!(events
            .iter()
            .any(|e| matches!(e, SimEvent::OrderFired { slot: 0, .. })));
    }
}
