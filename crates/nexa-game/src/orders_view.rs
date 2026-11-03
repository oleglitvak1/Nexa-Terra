//! The Caretaker terminal: the colony's Standing Orders and a builder for new cards.
//!
//! Cards are built by picking from lists, never typed (GDD: rule cards). Every
//! change goes through `World::add_order` / `World::remove_order`, so the sim
//! checks each card before storing it.

use crate::terminal::{Ink, Terminal};
use crate::ui::{wrap, ListPicker, Pick};
use macroquad::prelude::KeyCode;
use nexa_sim::colony::{Colony, Module};
use nexa_sim::orders::{format_centi_c, Action, OrderError, StandingOrder, Trigger};
use nexa_sim::World;
use std::ops::RangeInclusive;

const LEFT: usize = 2;
const TOP: usize = 3;
/// Inside the frame, with a column of margin on each side.
const WIDTH: usize = 76;
/// "> 1 P1 " in front of each card in the list.
const CARD_INDENT: usize = 7;

const PRIORITY_RANGE: RangeInclusive<u8> = 1..=9;
/// Starting threshold for a new power trigger.
const POWER_START: u8 = 25;
const POWER_STEP: u8 = 5;
const TEMP_STEP: i32 = 50;
const TEMP_RANGE: RangeInclusive<i32> = -5000..=10000;

const TRIGGER_KINDS: [&str; 3] = ["grid.power < N%", "MODULE.temp < N", "MODULE.temp > N"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionKind {
    Activate,
    Shutdown,
    DivertHeat,
    ShareHeat,
}

const ACTION_KINDS: [ActionKind; 4] = [
    ActionKind::Activate,
    ActionKind::Shutdown,
    ActionKind::DivertHeat,
    ActionKind::ShareHeat,
];

impl ActionKind {
    fn label(self) -> &'static str {
        match self {
            ActionKind::Activate => "activate MODULE",
            ActionKind::Shutdown => "shutdown MODULE",
            ActionKind::DivertHeat => "divert heat TO MODULE",
            ActionKind::ShareHeat => "share heat ON MODULE",
        }
    }

    fn build(self, m: &Module) -> Action {
        let id = m.id().clone();
        match self {
            ActionKind::Activate => Action::Activate(id),
            ActionKind::Shutdown => Action::Deactivate(id),
            ActionKind::DivertHeat => Action::DivertHeat { to: id },
            ActionKind::ShareHeat => Action::ShareHeat { line_of: id },
        }
    }

    /// Only offer modules the action can do something to.
    fn fits(self, m: &Module) -> bool {
        match self {
            ActionKind::Activate | ActionKind::Shutdown => {
                let def = m.def();
                def.draw_kw > 0 || def.output_kw > 0 || def.battery.is_some()
            }
            ActionKind::DivertHeat | ActionKind::ShareHeat => m.heat_net().is_some(),
        }
    }
}

/// Modules whose temperature is worth watching: machines and anything on a heat line.
fn watched(colony: &Colony) -> Vec<&Module> {
    colony
        .modules()
        .iter()
        .filter(|m| m.machine().is_some() || m.heat_net().is_some())
        .collect()
}

fn targets(colony: &Colony, kind: ActionKind) -> Vec<&Module> {
    colony.modules().iter().filter(|m| kind.fits(m)).collect()
}

/// Where the card builder is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Menu,
    PickTrigger(ListPicker),
    PickWatched { above: bool, picker: ListPicker },
    SetThreshold(Trigger),
    PickAction(ListPicker),
    PickTarget(ActionKind, ListPicker),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    List,
    Draft { order: StandingOrder, step: Step },
}

pub struct OrdersEditor {
    mode: Mode,
    list: ListPicker,
    /// The last thing the terminal has to say: a stored card or why it was refused.
    message: Option<String>,
}

impl OrdersEditor {
    pub fn new() -> Self {
        Self {
            mode: Mode::List,
            list: ListPicker::default(),
            message: None,
        }
    }

    pub fn handle(&mut self, key: KeyCode, world: &mut World) -> Outcome {
        match std::mem::replace(&mut self.mode, Mode::List) {
            Mode::List => return self.handle_list(key, world),
            Mode::Draft { order, step } => self.handle_draft(key, world, order, step),
        }
        Outcome::Stay
    }

    fn handle_list(&mut self, key: KeyCode, world: &mut World) -> Outcome {
        let count = world.orders.len();
        match key {
            KeyCode::N => {
                let memory = world.orders.tuning().memory;
                if count >= usize::from(memory) {
                    self.say(OrderError::MemoryFull { memory });
                } else {
                    self.message = None;
                    self.mode = Mode::Draft {
                        order: StandingOrder {
                            triggers: Vec::new(),
                            actions: Vec::new(),
                            priority: *PRIORITY_RANGE.start(),
                        },
                        step: Step::Menu,
                    };
                }
            }
            KeyCode::D | KeyCode::Delete => {
                let slot = self.list.cursor();
                if world.remove_order(slot).is_ok() {
                    self.message = Some(format!("CARD {} DELETED", slot + 1));
                    self.list.clamp(world.orders.len());
                }
            }
            _ => {
                if self.list.handle(key, count) == Pick::Back {
                    return Outcome::Close;
                }
            }
        }
        Outcome::Stay
    }

    fn handle_draft(
        &mut self,
        key: KeyCode,
        world: &mut World,
        mut order: StandingOrder,
        step: Step,
    ) {
        let step = match step {
            Step::Menu => match key {
                KeyCode::T => Step::PickTrigger(ListPicker::default()),
                KeyCode::A => Step::PickAction(ListPicker::default()),
                KeyCode::Left => {
                    order.priority = order
                        .priority
                        .saturating_sub(1)
                        .max(*PRIORITY_RANGE.start());
                    Step::Menu
                }
                KeyCode::Right => {
                    order.priority = order.priority.saturating_add(1).min(*PRIORITY_RANGE.end());
                    Step::Menu
                }
                KeyCode::Backspace => {
                    // Undo the most recent part: actions are always added after triggers.
                    if order.actions.pop().is_none() {
                        order.triggers.pop();
                    }
                    Step::Menu
                }
                KeyCode::Enter | KeyCode::KpEnter => match world.add_order(order.clone()) {
                    Ok(slot) => {
                        self.message = Some(format!("CARD {} STORED", slot + 1));
                        self.list = ListPicker::at(slot);
                        return;
                    }
                    Err(e) => {
                        self.say(e);
                        Step::Menu
                    }
                },
                KeyCode::Escape => {
                    self.message = Some("CARD DISCARDED".into());
                    return;
                }
                _ => Step::Menu,
            },
            Step::PickTrigger(mut picker) => match picker.handle(key, TRIGGER_KINDS.len()) {
                Pick::Chosen(0) => Step::SetThreshold(Trigger::PowerBelowPercent(POWER_START)),
                Pick::Chosen(i) => Step::PickWatched {
                    above: i == 2,
                    picker: ListPicker::default(),
                },
                Pick::Back => Step::Menu,
                Pick::Pending => Step::PickTrigger(picker),
            },
            Step::PickWatched { above, mut picker } => {
                let modules = watched(&world.colony);
                match picker.handle(key, modules.len()) {
                    Pick::Chosen(i) => {
                        let m = modules[i];
                        let module = m.id().clone();
                        // Start from the current reading, rounded down to a whole degree.
                        let centi_c = (m.temp_centi_c().div_euclid(100) * 100)
                            .clamp(*TEMP_RANGE.start(), *TEMP_RANGE.end());
                        Step::SetThreshold(if above {
                            Trigger::TempAbove { module, centi_c }
                        } else {
                            Trigger::TempBelow { module, centi_c }
                        })
                    }
                    Pick::Back => Step::PickTrigger(ListPicker::at(if above { 2 } else { 1 })),
                    Pick::Pending => Step::PickWatched { above, picker },
                }
            }
            Step::SetThreshold(mut trigger) => match key {
                KeyCode::Right | KeyCode::Up => {
                    nudge(&mut trigger, true);
                    Step::SetThreshold(trigger)
                }
                KeyCode::Left | KeyCode::Down => {
                    nudge(&mut trigger, false);
                    Step::SetThreshold(trigger)
                }
                KeyCode::Enter | KeyCode::KpEnter => {
                    order.triggers.push(trigger);
                    Step::Menu
                }
                KeyCode::Escape | KeyCode::Backspace => Step::Menu,
                _ => Step::SetThreshold(trigger),
            },
            Step::PickAction(mut picker) => match picker.handle(key, ACTION_KINDS.len()) {
                Pick::Chosen(i) => Step::PickTarget(ACTION_KINDS[i], ListPicker::default()),
                Pick::Back => Step::Menu,
                Pick::Pending => Step::PickAction(picker),
            },
            Step::PickTarget(kind, mut picker) => {
                let modules = targets(&world.colony, kind);
                match picker.handle(key, modules.len()) {
                    Pick::Chosen(i) => {
                        order.actions.push(kind.build(modules[i]));
                        Step::Menu
                    }
                    Pick::Back => {
                        let back = ACTION_KINDS.iter().position(|k| *k == kind).unwrap_or(0);
                        Step::PickAction(ListPicker::at(back))
                    }
                    Pick::Pending => Step::PickTarget(kind, picker),
                }
            }
        };
        self.mode = Mode::Draft { order, step };
    }

    fn say(&mut self, error: OrderError) {
        self.message = Some(error.to_string().to_uppercase());
    }

    /// Draw everything between the title row and `help_row`, which gets the key help.
    pub fn draw(&self, t: &mut Terminal, world: &World, help_row: usize) {
        let message_row = help_row - 2;
        let help = match &self.mode {
            Mode::List => {
                self.draw_list(t, world, message_row);
                "[UP/DOWN] SELECT   [N] NEW CARD   [D] DELETE   [ESC] BACK TO COLONY"
            }
            Mode::Draft { order, step } => draw_draft(t, world, order, step, message_row),
        };
        if let Some(message) = &self.message {
            t.put_str(LEFT, message_row, message, Ink::Bright);
        }
        t.put_str(LEFT, help_row, help, Ink::Dim);
    }

    fn draw_list(&self, t: &mut Terminal, world: &World, bottom: usize) {
        let orders = &world.orders;
        t.put_str(LEFT, TOP, "STANDING ORDERS", Ink::Bright);
        let memory = format!("MEMORY {}/{}", orders.len(), orders.tuning().memory);
        t.put_str(LEFT + WIDTH - memory.len(), TOP, &memory, Ink::Amber);
        if orders.is_empty() {
            t.put_str(
                LEFT,
                TOP + 2,
                "NO CARDS LOADED. THE CARETAKER WILL DO NOTHING.",
                Ink::Dim,
            );
            return;
        }
        let mut row = TOP + 2;
        for (slot, order) in orders.iter().enumerate() {
            let selected = slot == self.list.cursor();
            let (mark, ink) = if selected {
                ('>', Ink::Bright)
            } else {
                (' ', Ink::Amber)
            };
            let head = format!("{mark} {} P{}", slot + 1, order.priority);
            for (i, line) in wrap(&order.render(), WIDTH - CARD_INDENT)
                .iter()
                .enumerate()
            {
                if row >= bottom {
                    return;
                }
                if i == 0 {
                    t.put_str(LEFT, row, &head, ink);
                }
                t.put_str(LEFT + CARD_INDENT, row, line, ink);
                row += 1;
            }
        }
    }
}

fn draw_draft(
    t: &mut Terminal,
    world: &World,
    order: &StandingOrder,
    step: &Step,
    bottom: usize,
) -> &'static str {
    t.put_str(LEFT, TOP, "NEW CARD", Ink::Bright);
    let priority = format!("PRIORITY {}", order.priority);
    t.put_str(LEFT + WIDTH - priority.len(), TOP, &priority, Ink::Amber);
    let mut row = TOP + 2;
    for line in wrap(&draft_text(order), WIDTH) {
        t.put_str(LEFT, row, &line, Ink::Amber);
        row += 1;
    }
    row += 1;
    let height = bottom.saturating_sub(row + 2);
    let colony = &world.colony;
    match step {
        Step::Menu => {
            "[T] TRIGGER [A] ACTION [<-/->] PRIORITY [BKSP] UNDO [ENTER] STORE [ESC] DROP"
        }
        Step::PickTrigger(picker) => {
            t.put_str(LEFT, row, "ADD TRIGGER: WATCH WHAT?", Ink::Bright);
            let items: Vec<String> = TRIGGER_KINDS.iter().map(|s| s.to_string()).collect();
            picker.draw(t, LEFT, row + 2, height, &items);
            "[UP/DOWN] SELECT   [ENTER] CHOOSE   [ESC] BACK"
        }
        Step::PickWatched { picker, .. } => {
            t.put_str(LEFT, row, "ADD TRIGGER: WHICH MODULE?", Ink::Bright);
            let items: Vec<String> = watched(colony)
                .iter()
                .map(|m| format!("{:<12}{}", m.id(), format_centi_c(m.temp_centi_c())))
                .collect();
            picker.draw(t, LEFT, row + 2, height, &items);
            "[UP/DOWN] SELECT   [ENTER] CHOOSE   [ESC] BACK"
        }
        Step::SetThreshold(trigger) => {
            t.put_str(LEFT, row, "ADD TRIGGER: SET THRESHOLD", Ink::Bright);
            t.put_str(
                LEFT,
                row + 2,
                &format!("> {}", trigger.render()),
                Ink::Bright,
            );
            "[LEFT/RIGHT] ADJUST   [ENTER] ADD TO CARD   [ESC] CANCEL"
        }
        Step::PickAction(picker) => {
            t.put_str(LEFT, row, "ADD ACTION: DO WHAT?", Ink::Bright);
            let items: Vec<String> = ACTION_KINDS.iter().map(|k| k.label().to_string()).collect();
            picker.draw(t, LEFT, row + 2, height, &items);
            "[UP/DOWN] SELECT   [ENTER] CHOOSE   [ESC] BACK"
        }
        Step::PickTarget(kind, picker) => {
            t.put_str(LEFT, row, "ADD ACTION: WHICH MODULE?", Ink::Bright);
            let items: Vec<String> = targets(colony, *kind)
                .iter()
                .map(|m| m.id().to_string())
                .collect();
            picker.draw(t, LEFT, row + 2, height, &items);
            "[UP/DOWN] SELECT   [ENTER] CHOOSE   [ESC] BACK"
        }
    }
}

/// The card as it will render, with `...` in any slot still empty.
fn draft_text(order: &StandingOrder) -> String {
    let join = |parts: Vec<String>| {
        if parts.is_empty() {
            "...".to_string()
        } else {
            parts.join(" AND ")
        }
    };
    format!(
        "IF {} THEN {}",
        join(order.triggers.iter().map(Trigger::render).collect()),
        join(order.actions.iter().map(Action::render).collect())
    )
}

fn nudge(trigger: &mut Trigger, up: bool) {
    match trigger {
        Trigger::PowerBelowPercent(p) => {
            *p = if up {
                p.saturating_add(POWER_STEP).min(100)
            } else {
                p.saturating_sub(POWER_STEP).max(POWER_STEP)
            };
        }
        Trigger::TempBelow { centi_c, .. } | Trigger::TempAbove { centi_c, .. } => {
            let step = if up { TEMP_STEP } else { -TEMP_STEP };
            *centi_c = (*centi_c + step).clamp(*TEMP_RANGE.start(), *TEMP_RANGE.end());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexa_sim::orders::OrdersDef;
    use nexa_types::ModuleId;

    fn id(s: &str) -> ModuleId {
        ModuleId::new(s)
    }

    fn press(editor: &mut OrdersEditor, world: &mut World, keys: &[KeyCode]) -> Outcome {
        keys.iter()
            .fold(Outcome::Stay, |_, &k| editor.handle(k, world))
    }

    /// Move a picker down to item `n` and choose it.
    fn choose(editor: &mut OrdersEditor, world: &mut World, n: usize) {
        for _ in 0..n {
            editor.handle(KeyCode::Down, world);
        }
        editor.handle(KeyCode::Enter, world);
    }

    fn target_index(world: &World, kind: ActionKind, module: &str) -> usize {
        targets(&world.colony, kind)
            .iter()
            .position(|m| m.id().as_str() == module)
            .unwrap()
    }

    fn row_text(t: &Terminal, row: usize) -> String {
        (0..t.cols()).map(|c| t.get(c, row).unwrap().ch).collect()
    }

    fn screen_text(editor: &OrdersEditor, world: &World) -> String {
        let mut t = Terminal::new(80, 36);
        editor.draw(&mut t, world, 34);
        (0..t.rows())
            .map(|r| row_text(&t, r))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn builds_the_design_doc_card_from_lists() {
        let mut world = World::landfall();
        let mut editor = OrdersEditor::new();
        // IF grid.power < 25%
        press(
            &mut editor,
            &mut world,
            &[KeyCode::N, KeyCode::T, KeyCode::Enter, KeyCode::Enter],
        );
        // THEN shutdown smelter_01
        editor.handle(KeyCode::A, &mut world);
        choose(&mut editor, &mut world, 1);
        let smelter = target_index(&world, ActionKind::Shutdown, "smelter_01");
        choose(&mut editor, &mut world, smelter);
        // AND divert heat TO hab_01
        editor.handle(KeyCode::A, &mut world);
        choose(&mut editor, &mut world, 2);
        let hab = target_index(&world, ActionKind::DivertHeat, "hab_01");
        choose(&mut editor, &mut world, hab);
        editor.handle(KeyCode::Enter, &mut world);

        assert_eq!(world.orders.len(), 1);
        assert_eq!(
            world.orders.get(0).unwrap().render(),
            "IF grid.power < 25% THEN shutdown smelter_01 AND divert heat TO hab_01"
        );
        assert_eq!(editor.mode, Mode::List);
        assert_eq!(editor.message.as_deref(), Some("CARD 1 STORED"));
    }

    #[test]
    fn a_temperature_trigger_starts_at_the_reading_and_adjusts() {
        let mut world = World::landfall();
        let mut editor = OrdersEditor::new();
        // MODULE.temp < N, on hab_01 (14.0C at landfall).
        press(&mut editor, &mut world, &[KeyCode::N, KeyCode::T]);
        choose(&mut editor, &mut world, 1);
        let hab = watched(&world.colony)
            .iter()
            .position(|m| m.id().as_str() == "hab_01")
            .unwrap();
        choose(&mut editor, &mut world, hab);
        press(
            &mut editor,
            &mut world,
            &[KeyCode::Left, KeyCode::Left, KeyCode::Left],
        );
        let Mode::Draft {
            step: Step::SetThreshold(trigger),
            ..
        } = &editor.mode
        else {
            panic!("expected the threshold step, got {:?}", editor.mode);
        };
        assert_eq!(trigger.render(), "hab_01.temp < 12.5C");
    }

    #[test]
    fn thresholds_stay_in_range() {
        let mut power = Trigger::PowerBelowPercent(POWER_STEP);
        nudge(&mut power, false);
        assert_eq!(power, Trigger::PowerBelowPercent(POWER_STEP));
        let mut power = Trigger::PowerBelowPercent(100);
        nudge(&mut power, true);
        assert_eq!(power, Trigger::PowerBelowPercent(100));
        let mut temp = Trigger::TempAbove {
            module: id("hab_01"),
            centi_c: *TEMP_RANGE.end(),
        };
        nudge(&mut temp, true);
        assert!(matches!(temp, Trigger::TempAbove { centi_c, .. } if centi_c == *TEMP_RANGE.end()));
    }

    #[test]
    fn heat_actions_only_offer_modules_on_a_heat_line() {
        let world = World::landfall();
        let heat = targets(&world.colony, ActionKind::DivertHeat);
        assert!(heat.iter().all(|m| m.heat_net().is_some()));
        assert!(!heat.iter().any(|m| m.id().as_str() == "scrubber_01"));
        assert!(targets(&world.colony, ActionKind::Shutdown)
            .iter()
            .any(|m| m.id().as_str() == "scrubber_01"));
    }

    #[test]
    fn a_refused_card_stays_open_with_the_reason() {
        let mut world = World::landfall();
        let mut editor = OrdersEditor::new();
        press(&mut editor, &mut world, &[KeyCode::N, KeyCode::Enter]);
        assert!(world.orders.is_empty());
        assert!(matches!(editor.mode, Mode::Draft { .. }));
        assert_eq!(
            editor.message.as_deref(),
            Some("A CARD NEEDS AT LEAST ONE TRIGGER")
        );
    }

    #[test]
    fn backspace_undoes_the_last_part_and_priority_stays_in_range() {
        let mut world = World::landfall();
        let mut editor = OrdersEditor::new();
        press(
            &mut editor,
            &mut world,
            &[KeyCode::N, KeyCode::T, KeyCode::Enter, KeyCode::Enter],
        );
        editor.handle(KeyCode::A, &mut world);
        choose(&mut editor, &mut world, 0);
        choose(&mut editor, &mut world, 0);
        press(
            &mut editor,
            &mut world,
            &[KeyCode::Left, KeyCode::Backspace],
        );
        let Mode::Draft { order, .. } = &editor.mode else {
            panic!("draft closed");
        };
        assert_eq!(order.triggers.len(), 1);
        assert!(order.actions.is_empty());
        assert_eq!(order.priority, 1);
        let ten_rights = [KeyCode::Right; 10];
        press(&mut editor, &mut world, &ten_rights);
        let Mode::Draft { order, .. } = &editor.mode else {
            panic!("draft closed");
        };
        assert_eq!(order.priority, *PRIORITY_RANGE.end());
    }

    fn with_caretaker() -> World {
        let mut world = World::landfall();
        for order in &OrdersDef::shipped().presets["caretaker"] {
            world.add_order(order.clone()).unwrap();
        }
        world
    }

    #[test]
    fn delete_removes_the_selected_card() {
        let mut world = with_caretaker();
        let mut editor = OrdersEditor::new();
        let last = world.orders.get(2).unwrap().clone();
        press(&mut editor, &mut world, &[KeyCode::Up, KeyCode::D]);
        assert_eq!(world.orders.len(), 2);
        assert!(!world.orders.iter().any(|o| *o == last));
        assert_eq!(editor.message.as_deref(), Some("CARD 3 DELETED"));
        // The cursor moves onto the new last card.
        assert_eq!(editor.list.cursor(), 1);
    }

    #[test]
    fn escape_discards_a_draft_then_closes_the_terminal() {
        let mut world = World::landfall();
        let mut editor = OrdersEditor::new();
        assert_eq!(
            press(&mut editor, &mut world, &[KeyCode::N, KeyCode::Escape]),
            Outcome::Stay
        );
        assert_eq!(editor.mode, Mode::List);
        assert_eq!(editor.handle(KeyCode::Escape, &mut world), Outcome::Close);
    }

    #[test]
    fn a_full_terminal_refuses_a_new_card() {
        let mut world = with_caretaker();
        let card = world.orders.get(0).unwrap().clone();
        while world.add_order(card.clone()).is_ok() {}
        let mut editor = OrdersEditor::new();
        editor.handle(KeyCode::N, &mut world);
        assert_eq!(editor.mode, Mode::List);
        assert_eq!(
            editor.message.as_deref(),
            Some("TERMINAL MEMORY FULL: 8 CARDS")
        );
    }

    #[test]
    fn the_list_shows_every_card_with_its_slot_and_priority() {
        let world = with_caretaker();
        let screen = screen_text(&OrdersEditor::new(), &world);
        assert!(screen.contains("MEMORY 3/8"));
        assert!(screen.contains("> 1 P1 IF grid.power < 25% THEN shutdown smelter_01"));
        assert!(screen.contains("  3 P2 IF hab_02.temp > 16.0C THEN divert heat TO hab_01"));
    }

    #[test]
    fn an_empty_terminal_says_the_caretaker_will_do_nothing() {
        let screen = screen_text(&OrdersEditor::new(), &World::landfall());
        assert!(screen.contains("MEMORY 0/8"));
        assert!(screen.contains("THE CARETAKER WILL DO NOTHING"));
    }

    #[test]
    fn the_draft_shows_empty_slots() {
        let mut world = World::landfall();
        let mut editor = OrdersEditor::new();
        editor.handle(KeyCode::N, &mut world);
        let screen = screen_text(&editor, &world);
        assert!(screen.contains("NEW CARD"));
        assert!(screen.contains("IF ... THEN ..."));
    }
}
