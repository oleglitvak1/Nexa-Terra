//! Standing Orders: rule cards the Caretaker runs every hour while the crew is away.
//!
//! Cards are typed data, never strings; [`StandingOrder::render`] gives the
//! terminal's command syntax. Firing rule (see docs/architecture.md): a card
//! fires once when all its triggers hold, then stays disarmed until one of its
//! triggers has moved back past its threshold by the tuning margin.

use crate::colony::Colony;
use crate::SimEvent;
use nexa_types::{Command, ModuleId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// A condition a card watches. Water and emergency candles come with those systems.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trigger {
    /// Stored charge across all batteries is below this percentage.
    PowerBelowPercent(u8),
    TempBelow {
        module: ModuleId,
        centi_c: i32,
    },
    TempAbove {
        module: ModuleId,
        centi_c: i32,
    },
}

/// What a card does when it fires; each maps to one colony [`Command`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Activate(ModuleId),
    Deactivate(ModuleId),
    DivertHeat { to: ModuleId },
    ShareHeat { line_of: ModuleId },
}

/// One rule card: IF all triggers THEN every action, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingOrder {
    pub triggers: Vec<Trigger>,
    pub actions: Vec<Action>,
    /// 1 runs first; ties run in list order.
    pub priority: u8,
}

/// Tuning numbers from `content/standing_orders.ron`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderTuning {
    /// How many cards the terminal holds.
    pub memory: u8,
    /// How far back past its threshold a temperature must move to re-arm a card.
    pub temp_margin_centi_c: u16,
    /// The same for power, in percentage points.
    pub power_margin_percent: u8,
}

/// How the terminal's card builder steps thresholds and priorities. Content only:
/// unlike [`OrderTuning`] it isn't stored in saves, since cards already hold their values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorTuning {
    /// Priorities offered run from 1 up to this.
    pub priority_max: u8,
    /// Where a new power trigger starts, and how far one key press moves it.
    pub power_start_percent: u8,
    pub power_step_percent: u8,
    pub temp_step_centi_c: i32,
    pub temp_min_centi_c: i32,
    pub temp_max_centi_c: i32,
}

/// The Standing Orders content file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrdersDef {
    pub tuning: OrderTuning,
    pub editor: EditorTuning,
    /// Named starter card sets, e.g. `caretaker`.
    #[serde(default)]
    pub presets: BTreeMap<String, Vec<StandingOrder>>,
}

/// Shipped tuning and presets, compiled in so loading needs no file I/O.
pub const STANDING_ORDERS: &str = include_str!("../../../content/standing_orders.ron");

impl OrdersDef {
    pub fn from_ron(text: &str) -> Result<Self, String> {
        ron::from_str(text).map_err(|e| e.to_string())
    }

    /// Parsed once: every new [`Orders`] and every pre-orders save reads the tuning.
    pub fn shipped() -> &'static Self {
        static SHIPPED: OnceLock<OrdersDef> = OnceLock::new();
        // Checked by this module's tests, so failing here is a build bug.
        SHIPPED.get_or_init(|| {
            Self::from_ron(STANDING_ORDERS).expect("standing orders content is valid")
        })
    }
}

/// Why a card can't be stored, or a saved set of cards was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderError {
    /// The terminal's memory is full.
    MemoryFull {
        memory: u8,
    },
    NoTriggers,
    NoActions,
    UnknownModule(ModuleId),
    NotOnHeatLine(ModuleId),
    /// A power trigger outside 1..=100 would never fire, or never re-arm.
    PercentOutOfRange(u8),
    NoSuchSlot(usize),
}

impl std::fmt::Display for OrderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderError::MemoryFull { memory } => {
                write!(f, "terminal memory full: {memory} cards")
            }
            OrderError::NoTriggers => write!(f, "a card needs at least one trigger"),
            OrderError::NoActions => write!(f, "a card needs at least one action"),
            OrderError::UnknownModule(id) => write!(f, "no module named {id}"),
            OrderError::NotOnHeatLine(id) => write!(f, "{id} is not on a heat line"),
            OrderError::PercentOutOfRange(p) => write!(f, "power must be 1-100%, not {p}%"),
            OrderError::NoSuchSlot(slot) => write!(f, "no card in slot {slot}"),
        }
    }
}

impl std::error::Error for OrderError {}

/// Why a fired card's action did nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkipReason {
    /// A higher-priority card already switched that module or steered that heat line this hour.
    Overridden,
    /// The colony refused the command.
    Rejected,
}

impl SkipReason {
    pub fn label(self) -> &'static str {
        match self {
            SkipReason::Overridden => "overridden",
            SkipReason::Rejected => "rejected",
        }
    }
}

/// What an action takes control of for the hour, so lower-priority cards can't undo it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Claim {
    Switch(ModuleId),
    HeatLine(Option<usize>),
}

impl Trigger {
    fn holds(&self, colony: &Colony) -> bool {
        match self {
            Trigger::PowerBelowPercent(p) => colony.power_percent() < *p,
            Trigger::TempBelow { module, centi_c } => {
                temp(colony, module).is_some_and(|t| t < *centi_c)
            }
            Trigger::TempAbove { module, centi_c } => {
                temp(colony, module).is_some_and(|t| t > *centi_c)
            }
        }
    }

    /// Back past the threshold by the margin: the card may fire again.
    fn cleared(&self, colony: &Colony, tuning: &OrderTuning) -> bool {
        match self {
            Trigger::PowerBelowPercent(p) => {
                let rearm = (u16::from(*p) + u16::from(tuning.power_margin_percent)).min(100);
                u16::from(colony.power_percent()) >= rearm
            }
            Trigger::TempBelow { module, centi_c } => temp(colony, module).is_some_and(|t| {
                i64::from(t) >= i64::from(*centi_c) + i64::from(tuning.temp_margin_centi_c)
            }),
            Trigger::TempAbove { module, centi_c } => temp(colony, module).is_some_and(|t| {
                i64::from(t) <= i64::from(*centi_c) - i64::from(tuning.temp_margin_centi_c)
            }),
        }
    }

    fn module(&self) -> Option<&ModuleId> {
        match self {
            Trigger::PowerBelowPercent(_) => None,
            Trigger::TempBelow { module, .. } | Trigger::TempAbove { module, .. } => Some(module),
        }
    }

    /// Command syntax, e.g. `hab_01.temp < -5.0C`.
    pub fn render(&self) -> String {
        match self {
            Trigger::PowerBelowPercent(p) => format!("grid.power < {p}%"),
            Trigger::TempBelow { module, centi_c } => {
                format!("{module}.temp < {}", format_centi_c(*centi_c))
            }
            Trigger::TempAbove { module, centi_c } => {
                format!("{module}.temp > {}", format_centi_c(*centi_c))
            }
        }
    }
}

impl Action {
    pub fn command(&self) -> Command {
        match self {
            Action::Activate(id) => Command::Activate(id.clone()),
            Action::Deactivate(id) => Command::Deactivate(id.clone()),
            Action::DivertHeat { to } => Command::DivertHeat { to: to.clone() },
            Action::ShareHeat { line_of } => Command::ShareHeat {
                line_of: line_of.clone(),
            },
        }
    }

    /// The module this action switches, or whose heat line it steers.
    pub fn module(&self) -> &ModuleId {
        match self {
            Action::Activate(id) | Action::Deactivate(id) => id,
            Action::DivertHeat { to: id } | Action::ShareHeat { line_of: id } => id,
        }
    }

    fn steers_heat(&self) -> bool {
        match self {
            Action::Activate(_) | Action::Deactivate(_) => false,
            Action::DivertHeat { .. } | Action::ShareHeat { .. } => true,
        }
    }

    fn claim(&self, colony: &Colony) -> Claim {
        if self.steers_heat() {
            Claim::HeatLine(colony.module(self.module()).and_then(|m| m.heat_net()))
        } else {
            Claim::Switch(self.module().clone())
        }
    }

    /// Command syntax, e.g. `shutdown smelter_01`.
    pub fn render(&self) -> String {
        match self {
            Action::Activate(id) => format!("activate {id}"),
            Action::Deactivate(id) => format!("shutdown {id}"),
            Action::DivertHeat { to } => format!("divert heat TO {to}"),
            Action::ShareHeat { line_of } => format!("share heat ON {line_of}"),
        }
    }
}

impl StandingOrder {
    /// The card in the terminal's command syntax.
    pub fn render(&self) -> String {
        let join = |parts: Vec<String>| parts.join(" AND ");
        format!(
            "IF {} THEN {}",
            join(self.triggers.iter().map(Trigger::render).collect()),
            join(self.actions.iter().map(Action::render).collect())
        )
    }

    /// Every module the card names must exist; heat actions need a heat line.
    fn check(&self, colony: &Colony) -> Result<(), OrderError> {
        if self.triggers.is_empty() {
            return Err(OrderError::NoTriggers);
        }
        if self.actions.is_empty() {
            return Err(OrderError::NoActions);
        }
        for trigger in &self.triggers {
            if let Trigger::PowerBelowPercent(p) = trigger {
                if !(1..=100).contains(p) {
                    return Err(OrderError::PercentOutOfRange(*p));
                }
            }
        }
        for id in self.triggers.iter().filter_map(Trigger::module) {
            if colony.module(id).is_none() {
                return Err(OrderError::UnknownModule(id.clone()));
            }
        }
        for action in &self.actions {
            let id = action.module();
            let module = colony
                .module(id)
                .ok_or_else(|| OrderError::UnknownModule(id.clone()))?;
            if action.steers_heat() && module.heat_net().is_none() {
                return Err(OrderError::NotOnHeatLine(id.clone()));
            }
        }
        Ok(())
    }
}

/// A stored card and whether it may fire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Card {
    order: StandingOrder,
    armed: bool,
}

/// The cards loaded in the Caretaker terminal, with their firing state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Orders {
    tuning: OrderTuning,
    cards: Vec<Card>,
}

impl Default for Orders {
    fn default() -> Self {
        Self::new(OrdersDef::shipped().tuning)
    }
}

impl Orders {
    pub fn new(tuning: OrderTuning) -> Self {
        Self {
            tuning,
            cards: Vec::new(),
        }
    }

    pub fn tuning(&self) -> OrderTuning {
        self.tuning
    }

    pub fn len(&self) -> usize {
        self.cards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    pub fn get(&self, slot: usize) -> Option<&StandingOrder> {
        self.cards.get(slot).map(|c| &c.order)
    }

    /// Cards in slot (list) order.
    pub fn iter(&self) -> impl Iterator<Item = &StandingOrder> {
        self.cards.iter().map(|c| &c.order)
    }

    /// Store a card in the next slot. It starts armed, so it fires on the next
    /// tick if its triggers already hold.
    pub fn add(&mut self, order: StandingOrder, colony: &Colony) -> Result<usize, OrderError> {
        if self.cards.len() >= usize::from(self.tuning.memory) {
            return Err(OrderError::MemoryFull {
                memory: self.tuning.memory,
            });
        }
        order.check(colony)?;
        self.cards.push(Card { order, armed: true });
        Ok(self.cards.len() - 1)
    }

    /// Take a card out; later cards move up a slot.
    pub fn remove(&mut self, slot: usize) -> Result<StandingOrder, OrderError> {
        if slot >= self.cards.len() {
            return Err(OrderError::NoSuchSlot(slot));
        }
        Ok(self.cards.remove(slot).order)
    }

    /// Slots in the order the Caretaker runs them: by priority, ties in list order.
    pub fn run_order(&self) -> Vec<usize> {
        let mut slots: Vec<usize> = (0..self.cards.len()).collect();
        // Stable sort keeps list order within a priority.
        slots.sort_by_key(|&s| self.cards[s].order.priority);
        slots
    }

    /// Check saved cards against the colony they will run on.
    pub(crate) fn check(&self, colony: &Colony) -> Result<(), OrderError> {
        if self.cards.len() > usize::from(self.tuning.memory) {
            return Err(OrderError::MemoryFull {
                memory: self.tuning.memory,
            });
        }
        self.cards.iter().try_for_each(|c| c.order.check(colony))
    }

    /// One hour of the Caretaker: every card in run order, against the colony
    /// as it stands before the hour runs. A card's actions go through
    /// [`Colony::apply`]; once a card switches a module or steers a heat line,
    /// lower-priority cards can't touch it this hour.
    pub fn evaluate(&mut self, colony: &mut Colony) -> Vec<SimEvent> {
        let tuning = self.tuning;
        let mut events = Vec::new();
        let mut claimed: Vec<Claim> = Vec::new();
        for slot in self.run_order() {
            let card = &mut self.cards[slot];
            if !card.armed {
                if card
                    .order
                    .triggers
                    .iter()
                    .any(|t| t.cleared(colony, &tuning))
                {
                    card.armed = true;
                }
                // A card that re-arms waits an hour: the reading has only just crossed back.
                continue;
            }
            if !card.order.triggers.iter().all(|t| t.holds(colony)) {
                continue;
            }
            events.push(SimEvent::OrderFired {
                slot,
                order: card.order.clone(),
            });
            let mut taken = Vec::new();
            for action in &card.order.actions {
                let claim = action.claim(colony);
                if claimed.contains(&claim) {
                    events.push(SimEvent::OrderActionSkipped {
                        slot,
                        action: action.clone(),
                        reason: SkipReason::Overridden,
                    });
                    continue;
                }
                match colony.apply(&action.command()) {
                    Ok(more) => {
                        taken.push(claim);
                        events.extend(more);
                    }
                    Err(_) => events.push(SimEvent::OrderActionSkipped {
                        slot,
                        action: action.clone(),
                        reason: SkipReason::Rejected,
                    }),
                }
            }
            // A card that changed nothing stays armed and tries again next hour;
            // otherwise a blocked card could wait forever for its trigger to clear.
            card.armed = taken.is_empty();
            claimed.extend(taken);
        }
        events
    }
}

fn temp(colony: &Colony, id: &ModuleId) -> Option<i32> {
    colony.module(id).map(|m| m.temp_centi_c())
}

/// Centi-degrees as degrees C with one decimal place, rounding half away from zero.
pub fn format_centi_c(centi_c: i32) -> String {
    let deci = (i64::from(centi_c).abs() + 5) / 10;
    let sign = if centi_c < 0 && deci != 0 { "-" } else { "" };
    format!("{sign}{}.{}C", deci / 10, deci % 10)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colony::LANDING_ASH_PLAIN;
    use crate::machines::MachineState;
    use crate::World;

    fn id(s: &str) -> ModuleId {
        ModuleId::new(s)
    }

    fn ash_plain() -> Colony {
        Colony::from_ron(LANDING_ASH_PLAIN).unwrap()
    }

    fn card(priority: u8, triggers: Vec<Trigger>, actions: Vec<Action>) -> StandingOrder {
        StandingOrder {
            triggers,
            actions,
            priority,
        }
    }

    /// Always true on the Ash Plain: nothing gets that hot.
    fn always() -> Trigger {
        Trigger::TempBelow {
            module: id("hab_01"),
            centi_c: 100_000,
        }
    }

    fn fired(events: &[SimEvent]) -> Vec<usize> {
        events
            .iter()
            .filter_map(|e| match e {
                SimEvent::OrderFired { slot, .. } => Some(*slot),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn renders_the_design_doc_card() {
        let order = card(
            1,
            vec![Trigger::PowerBelowPercent(25)],
            vec![
                Action::Deactivate(id("smelter_01")),
                Action::DivertHeat { to: id("hab_01") },
            ],
        );
        assert_eq!(
            order.render(),
            "IF grid.power < 25% THEN shutdown smelter_01 AND divert heat TO hab_01"
        );
    }

    #[test]
    fn renders_every_trigger_and_action() {
        let order = card(
            1,
            vec![
                Trigger::TempBelow {
                    module: id("hab_01"),
                    centi_c: -500,
                },
                Trigger::TempAbove {
                    module: id("hab_02"),
                    centi_c: 1450,
                },
            ],
            vec![
                Action::Activate(id("heater_02")),
                Action::ShareHeat {
                    line_of: id("hab_01"),
                },
            ],
        );
        assert_eq!(
            order.render(),
            "IF hab_01.temp < -5.0C AND hab_02.temp > 14.5C \
             THEN activate heater_02 AND share heat ON hab_01"
        );
    }

    #[test]
    fn temperatures_render_to_one_decimal() {
        assert_eq!(format_centi_c(0), "0.0C");
        assert_eq!(format_centi_c(1234), "12.3C");
        assert_eq!(format_centi_c(1235), "12.4C");
        assert_eq!(format_centi_c(-1235), "-12.4C");
        assert_eq!(format_centi_c(-4), "0.0C");
        assert_eq!(format_centi_c(-5), "-0.1C");
        assert_eq!(format_centi_c(i32::MIN), "-21474836.5C");
    }

    #[test]
    fn shipped_content_loads_and_every_preset_fits_the_ash_plain() {
        let def = OrdersDef::shipped();
        assert!(def.tuning.memory >= 3);
        assert!(def.presets.contains_key("caretaker"));
        let colony = ash_plain();
        for (name, cards) in &def.presets {
            let mut orders = Orders::new(def.tuning);
            for order in cards {
                orders
                    .add(order.clone(), &colony)
                    .unwrap_or_else(|e| panic!("preset {name}: {e}"));
            }
        }
    }

    #[test]
    fn shipped_editor_tuning_can_key_in_every_preset() {
        let def = OrdersDef::shipped();
        let e = def.editor;
        assert!(e.priority_max >= 1);
        assert!((1..=100).contains(&e.power_start_percent));
        assert!(e.power_step_percent >= 1 && e.temp_step_centi_c >= 1);
        assert!(e.temp_min_centi_c < e.temp_max_centi_c);
        for card in def.presets.values().flatten() {
            assert!((1..=e.priority_max).contains(&card.priority));
            for trigger in &card.triggers {
                if let Trigger::TempBelow { centi_c, .. } | Trigger::TempAbove { centi_c, .. } =
                    trigger
                {
                    assert!((e.temp_min_centi_c..=e.temp_max_centi_c).contains(centi_c));
                }
            }
        }
    }

    #[test]
    fn runs_by_priority_then_list_order() {
        let colony = ash_plain();
        let mut orders = Orders::default();
        for priority in [2, 1, 2, 1] {
            orders
                .add(
                    card(
                        priority,
                        vec![always()],
                        vec![Action::Activate(id("heater_01"))],
                    ),
                    &colony,
                )
                .unwrap();
        }
        assert_eq!(orders.run_order(), vec![1, 3, 0, 2]);
        let mut colony = colony;
        assert_eq!(fired(&orders.evaluate(&mut colony)), vec![1, 3, 0, 2]);
    }

    #[test]
    fn higher_priority_card_wins_a_conflict() {
        let mut colony = ash_plain();
        let mut orders = Orders::default();
        // Listed first but lower priority: must not undo the shutdown.
        orders
            .add(
                card(2, vec![always()], vec![Action::Activate(id("heater_01"))]),
                &colony,
            )
            .unwrap();
        orders
            .add(
                card(1, vec![always()], vec![Action::Deactivate(id("heater_01"))]),
                &colony,
            )
            .unwrap();
        let events = orders.evaluate(&mut colony);
        assert!(!colony.module(&id("heater_01")).unwrap().active());
        assert_eq!(fired(&events), vec![1, 0]);
        assert!(events.contains(&SimEvent::OrderActionSkipped {
            slot: 0,
            action: Action::Activate(id("heater_01")),
            reason: SkipReason::Overridden,
        }));
    }

    #[test]
    fn heat_line_claims_cover_the_whole_line() {
        let mut colony = ash_plain();
        let mut orders = Orders::default();
        orders
            .add(
                card(
                    1,
                    vec![always()],
                    vec![Action::DivertHeat { to: id("hab_01") }],
                ),
                &colony,
            )
            .unwrap();
        orders
            .add(
                card(
                    2,
                    vec![always()],
                    vec![Action::DivertHeat { to: id("hab_02") }],
                ),
                &colony,
            )
            .unwrap();
        let events = orders.evaluate(&mut colony);
        assert!(events.contains(&SimEvent::OrderActionSkipped {
            slot: 1,
            action: Action::DivertHeat { to: id("hab_02") },
            reason: SkipReason::Overridden,
        }));
    }

    #[test]
    fn a_card_that_was_wholly_overridden_tries_again_next_hour() {
        let mut colony = ash_plain();
        let mut orders = Orders::default();
        for (priority, to) in [(1, "hab_01"), (2, "hab_02")] {
            orders
                .add(
                    card(
                        priority,
                        vec![always()],
                        vec![Action::DivertHeat { to: id(to) }],
                    ),
                    &colony,
                )
                .unwrap();
        }
        assert_eq!(fired(&orders.evaluate(&mut colony)), vec![0, 1]);
        // Card 2 did nothing, so it stays armed and gets the heat line now.
        let events = orders.evaluate(&mut colony);
        assert_eq!(fired(&events), vec![1]);
        assert!(!events
            .iter()
            .any(|e| matches!(e, SimEvent::OrderActionSkipped { .. })));
    }

    #[test]
    fn a_card_can_steer_the_same_thing_twice() {
        // Claims block lower-priority cards, not the card's own later actions.
        let mut colony = ash_plain();
        let mut orders = Orders::default();
        orders
            .add(
                card(
                    1,
                    vec![always()],
                    vec![
                        Action::Deactivate(id("heater_02")),
                        Action::Activate(id("heater_02")),
                    ],
                ),
                &colony,
            )
            .unwrap();
        orders.evaluate(&mut colony);
        assert!(colony.module(&id("heater_02")).unwrap().active());
    }

    #[test]
    fn memory_caps_the_card_count() {
        let colony = ash_plain();
        let mut orders = Orders::default();
        let memory = orders.tuning().memory;
        let order = card(1, vec![always()], vec![Action::Activate(id("heater_01"))]);
        for slot in 0..usize::from(memory) {
            assert_eq!(orders.add(order.clone(), &colony), Ok(slot));
        }
        assert_eq!(
            orders.add(order.clone(), &colony),
            Err(OrderError::MemoryFull { memory })
        );
        orders.remove(0).unwrap();
        assert!(orders.add(order, &colony).is_ok());
    }

    #[test]
    fn cards_are_checked_against_the_colony() {
        let colony = ash_plain();
        let mut orders = Orders::default();
        let activate = vec![Action::Activate(id("heater_01"))];
        assert_eq!(
            orders.add(card(1, vec![], activate.clone()), &colony),
            Err(OrderError::NoTriggers)
        );
        assert_eq!(
            orders.add(card(1, vec![always()], vec![]), &colony),
            Err(OrderError::NoActions)
        );
        let ghost = Trigger::TempBelow {
            module: id("ghost"),
            centi_c: 0,
        };
        assert_eq!(
            orders.add(card(1, vec![ghost], activate.clone()), &colony),
            Err(OrderError::UnknownModule(id("ghost")))
        );
        assert_eq!(
            orders.add(
                card(
                    1,
                    vec![always()],
                    vec![Action::DivertHeat {
                        to: id("scrubber_01")
                    }]
                ),
                &colony
            ),
            Err(OrderError::NotOnHeatLine(id("scrubber_01")))
        );
        // 0% can never hold; past 100% always holds and never re-arms.
        for p in [0, 101] {
            assert_eq!(
                orders.add(
                    card(1, vec![Trigger::PowerBelowPercent(p)], activate.clone()),
                    &colony
                ),
                Err(OrderError::PercentOutOfRange(p))
            );
        }
        assert_eq!(orders.remove(0), Err(OrderError::NoSuchSlot(0)));
        assert!(orders.is_empty());
    }

    #[test]
    fn a_card_fires_once_while_its_trigger_holds() {
        let mut colony = ash_plain();
        let mut orders = Orders::default();
        orders
            .add(
                card(1, vec![always()], vec![Action::Activate(id("heater_01"))]),
                &colony,
            )
            .unwrap();
        let mut count = 0;
        for _ in 0..50 {
            count += fired(&orders.evaluate(&mut colony)).len();
            colony.tick();
        }
        assert_eq!(count, 1);
    }

    #[test]
    fn a_power_card_rearms_past_the_margin() {
        let tuning = OrderTuning {
            memory: 8,
            temp_margin_centi_c: 200,
            power_margin_percent: 5,
        };
        // Landfall: 300 of 400 kWh, 75%.
        let colony = ash_plain();
        assert_eq!(colony.power_percent(), 75);
        assert!(Trigger::PowerBelowPercent(76).holds(&colony));
        assert!(!Trigger::PowerBelowPercent(71).cleared(&colony, &tuning));
        assert!(Trigger::PowerBelowPercent(70).cleared(&colony, &tuning));
        // A 100% card re-arms at a full battery rather than never.
        assert!(!Trigger::PowerBelowPercent(100).cleared(&colony, &tuning));
    }

    /// No orders: from Sol 3 the starved habs flutter across 5.0C every hour or two.
    /// A freeze card must not fire on every dip.
    #[test]
    fn the_margin_stops_a_card_firing_on_every_flutter() {
        let freeze_card = card(
            1,
            vec![Trigger::TempBelow {
                module: id("hab_01"),
                centi_c: 500,
            }],
            // Already on: changes nothing, so the colony flutters as it would alone.
            vec![Action::Activate(id("heater_01"))],
        );
        let fires = |margin: u16| {
            let mut world = World::landfall();
            world.orders = Orders::new(OrderTuning {
                temp_margin_centi_c: margin,
                ..OrdersDef::shipped().tuning
            });
            world.add_order(freeze_card.clone()).unwrap();
            let mut count = 0;
            let mut froze = false;
            for _ in 0..6 * nexa_types::TICKS_PER_SOL {
                let events = world.tick();
                count += fired(&events).len();
                froze |= events.contains(&SimEvent::MachineStateChanged {
                    module: id("hab_01"),
                    state: MachineState::Freeze,
                });
            }
            assert!(froze, "the colony should flutter without help");
            count
        };
        assert!(fires(0) > 5, "with no margin the card chatters");
        assert_eq!(fires(OrdersDef::shipped().tuning.temp_margin_centi_c), 1);
    }
}
