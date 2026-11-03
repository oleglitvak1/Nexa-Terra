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
use orders::{Action, OrderError, Orders, SkipReason, StandingOrder};
use serde::{Deserialize, Serialize};

pub mod colony;
pub mod grid;
pub mod machines;
pub mod orders;
pub mod parts;

/// The whole state of one player's game. Grows milestone by milestone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawWorld")]
pub struct World {
    /// Current simulation time.
    pub now: Tick,
    pub colony: Colony,
    /// Standing Orders; add cards through [`World::add_order`] so they're checked.
    pub orders: Orders,
}

/// Unchecked wire form of [`World`]; serde checks it via `TryFrom`.
#[derive(Deserialize)]
struct RawWorld {
    now: Tick,
    colony: Colony,
    /// Saves from before Standing Orders have none.
    #[serde(default)]
    orders: Orders,
}

impl TryFrom<RawWorld> for World {
    type Error = OrderError;

    fn try_from(raw: RawWorld) -> Result<Self, Self::Error> {
        raw.orders.check(&raw.colony)?;
        Ok(Self {
            now: raw.now,
            colony: raw.colony,
            orders: raw.orders,
        })
    }
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
    /// A Standing Order's triggers all held; its actions follow as their own events.
    OrderFired { slot: usize, order: StandingOrder },
    /// One action of a fired card did nothing.
    OrderActionSkipped {
        slot: usize,
        action: Action,
        reason: SkipReason,
    },
}

impl World {
    pub fn new(colony: Colony) -> Self {
        Self {
            now: Tick::default(),
            colony,
            orders: Orders::default(),
        }
    }

    /// The vertical slice's start: landfall on the Ash Plain.
    pub fn landfall() -> Self {
        // The shipped content is checked by colony's tests, so failing here is a build bug.
        Self::new(Colony::from_ron(LANDING_ASH_PLAIN).expect("landing site content is valid"))
    }

    /// Advance the simulation by one fixed step (one planetary hour).
    pub fn tick(&mut self) -> Vec<SimEvent> {
        // The Caretaker reads the gauges and acts, then the hour runs.
        let mut events = self.orders.evaluate(&mut self.colony);
        events.extend(self.colony.tick());
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

    /// Store a Standing Order in the next free slot, checked against this colony.
    pub fn add_order(&mut self, order: StandingOrder) -> Result<usize, OrderError> {
        self.orders.add(order, &self.colony)
    }

    /// Take a Standing Order out of its slot; later cards move up.
    pub fn remove_order(&mut self, slot: usize) -> Result<StandingOrder, OrderError> {
        self.orders.remove(slot)
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
        for order in &OrdersDef::shipped().presets[name] {
            world.add_order(order.clone()).unwrap();
        }
        world
    }

    fn run_sols(world: &mut World, sols: u64) -> Vec<SimEvent> {
        (0..sols * TICKS_PER_SOL)
            .flat_map(|_| world.tick())
            .collect()
    }

    #[test]
    fn orders_and_their_firing_state_survive_a_save() {
        let mut world = with_preset("caretaker");
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
        let world = with_preset("caretaker");
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
        let mut world = with_preset("caretaker");
        let third = world.orders.get(2).unwrap().clone();
        let first = world.remove_order(0).unwrap();
        assert_eq!(first, OrdersDef::shipped().presets["caretaker"][0]);
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
