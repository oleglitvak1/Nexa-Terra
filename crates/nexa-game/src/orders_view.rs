//! The Caretaker terminal: the colony's Standing Orders and a builder for new cards.
//!
//! Cards are built by picking from lists, never typed (GDD: rule cards). Every
//! change goes through `World::add_order` / `World::remove_order`, so the sim
//! checks each card before storing it.

use crate::terminal::{Ink, Terminal};
use crate::ui::{wrap, ListPicker, Pick};
use macroquad::prelude::KeyCode;
use nexa_sim::colony::{Colony, Module};
use nexa_sim::orders::{
    format_centi_c, Action, EditorTuning, OrderError, OrdersDef, StandingOrder, Trigger,
};
use nexa_sim::World;

const LEFT: usize = 2;
const TOP: usize = 3;
/// Inside the frame, with a column of margin on each side.
const WIDTH: usize = 76;
/// "> 1 P1 " in front of each card in the list.
const CARD_INDENT: usize = 7;
/// Priority 1 runs first, so it's where every new card starts.
const FIRST_PRIORITY: u8 = 1;

fn tuning() -> EditorTuning {
    OrdersDef::shipped().editor
}

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

/// A part of a draft card, kept in the order added so Backspace undoes the latest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Trigger,
    Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    List,
    Draft {
        order: StandingOrder,
        step: Step,
        added: Vec<Part>,
    },
}

pub struct OrdersEditor {
    mode: Mode,
    list: ListPicker,
    /// The last thing the terminal has to say: a stored card or why it was refused.
    message: Option<String>,
    /// The slot a first D press asked to delete; a second D deletes it.
    confirm_delete: Option<usize>,
}

impl OrdersEditor {
    pub fn new() -> Self {
        Self {
            mode: Mode::List,
            list: ListPicker::default(),
            message: None,
            confirm_delete: None,
        }
    }

    pub fn handle(&mut self, key: KeyCode, world: &mut World) -> Outcome {
        match std::mem::replace(&mut self.mode, Mode::List) {
            Mode::List => return self.handle_list(key, world),
            Mode::Draft { order, step, added } => self.handle_draft(key, world, order, step, added),
        }
        Outcome::Stay
    }

    fn handle_list(&mut self, key: KeyCode, world: &mut World) -> Outcome {
        let count = world.orders.len();
        let confirming = self.confirm_delete.take();
        let delete = matches!(key, KeyCode::D | KeyCode::Delete);
        if confirming.is_some() && !delete {
            self.message = None;
            // Esc only cancels the delete; it takes a second Esc to leave.
            if key == KeyCode::Escape {
                return Outcome::Stay;
            }
        }
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
                            priority: FIRST_PRIORITY,
                        },
                        step: Step::Menu,
                        added: Vec::new(),
                    };
                }
            }
            _ if delete => {
                let slot = self.list.cursor();
                if confirming == Some(slot) {
                    if world.remove_order(slot).is_ok() {
                        self.message = Some(format!("CARD {} DELETED", slot + 1));
                        self.list.clamp(world.orders.len());
                    }
                } else if slot < count {
                    self.confirm_delete = Some(slot);
                    self.message = Some(format!("DELETE CARD {}? [D] AGAIN TO CONFIRM", slot + 1));
                }
            }
            // Backspace is undo in the builder; on the list it must not close the terminal.
            KeyCode::Backspace => {}
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
        mut added: Vec<Part>,
    ) {
        let tuning = tuning();
        let step = match step {
            Step::Menu => match key {
                KeyCode::T => Step::PickTrigger(ListPicker::default()),
                KeyCode::A => Step::PickAction(ListPicker::default()),
                KeyCode::Left => {
                    order.priority = order.priority.saturating_sub(1).max(FIRST_PRIORITY);
                    Step::Menu
                }
                KeyCode::Right => {
                    order.priority = order.priority.saturating_add(1).min(tuning.priority_max);
                    Step::Menu
                }
                KeyCode::Backspace => {
                    match added.pop() {
                        Some(Part::Trigger) => {
                            order.triggers.pop();
                        }
                        Some(Part::Action) => {
                            order.actions.pop();
                        }
                        None => {}
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
                Pick::Chosen(0) => {
                    Step::SetThreshold(Trigger::PowerBelowPercent(tuning.power_start_percent))
                }
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
                            .clamp(tuning.temp_min_centi_c, tuning.temp_max_centi_c);
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
                    nudge(&mut trigger, true, &tuning);
                    Step::SetThreshold(trigger)
                }
                KeyCode::Left | KeyCode::Down => {
                    nudge(&mut trigger, false, &tuning);
                    Step::SetThreshold(trigger)
                }
                KeyCode::Enter | KeyCode::KpEnter => {
                    order.triggers.push(trigger);
                    added.push(Part::Trigger);
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
                        added.push(Part::Action);
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
        self.mode = Mode::Draft { order, step, added };
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
            Mode::Draft { order, step, .. } => draw_draft(t, world, order, step, message_row),
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
        let cards: Vec<Vec<String>> = orders
            .iter()
            .map(|o| wrap(&o.render(), WIDTH - CARD_INDENT))
            .collect();
        let cursor = self.list.cursor();
        // Scroll by whole cards until the selected one fits above `bottom`.
        let height = bottom.saturating_sub(TOP + 2);
        let mut first = 0;
        while first < cursor && cards[first..=cursor].iter().map(Vec::len).sum::<usize>() > height {
            first += 1;
        }
        let mut row = TOP + 2;
        for (slot, (order, lines)) in orders.iter().zip(&cards).enumerate().skip(first) {
            let (mark, ink) = if slot == cursor {
                ('>', Ink::Bright)
            } else {
                (' ', Ink::Amber)
            };
            let head = format!("{mark} {} P{}", slot + 1, order.priority);
            for (i, line) in lines.iter().enumerate() {
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
    // Leave a blank row, the step's label, a blank row and one list row above `bottom`;
    // a card too long for that shows its latest parts, the ones being edited.
    let room = bottom.saturating_sub(row + 4).max(1);
    let lines = wrap(&draft_text(order), WIDTH);
    let skip = if lines.len() > room {
        t.put_str(LEFT, row, "...", Ink::Dim);
        row += 1;
        lines.len() - room + 1
    } else {
        0
    };
    for line in &lines[skip..] {
        t.put_str(LEFT, row, line, Ink::Amber);
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

/// Power moves in whole steps, but by 1 below the first step so every value the sim
/// accepts (1 to 100%) can be keyed in.
fn nudge(trigger: &mut Trigger, up: bool, tuning: &EditorTuning) {
    match trigger {
        Trigger::PowerBelowPercent(p) => {
            let step = tuning.power_step_percent;
            *p = match (up, *p) {
                (true, p) if p < step => p + 1,
                (true, p) => p.saturating_add(step),
                (false, p) if p <= step => p.saturating_sub(1),
                (false, p) => p - step,
            }
            .clamp(1, 100);
        }
        Trigger::TempBelow { centi_c, .. } | Trigger::TempAbove { centi_c, .. } => {
            let step = tuning.temp_step_centi_c;
            let step = if up { step } else { -step };
            *centi_c = (*centi_c + step).clamp(tuning.temp_min_centi_c, tuning.temp_max_centi_c);
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
        let tuning = tuning();
        let mut power = Trigger::PowerBelowPercent(1);
        nudge(&mut power, false, &tuning);
        assert_eq!(power, Trigger::PowerBelowPercent(1));
        let mut power = Trigger::PowerBelowPercent(100);
        nudge(&mut power, true, &tuning);
        assert_eq!(power, Trigger::PowerBelowPercent(100));
        let max = tuning.temp_max_centi_c;
        let mut temp = Trigger::TempAbove {
            module: id("hab_01"),
            centi_c: max,
        };
        nudge(&mut temp, true, &tuning);
        assert!(matches!(temp, Trigger::TempAbove { centi_c, .. } if centi_c == max));
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
        assert_eq!(order.priority, tuning().priority_max);
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
        press(
            &mut editor,
            &mut world,
            &[KeyCode::Up, KeyCode::D, KeyCode::D],
        );
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

/// Whole sessions at the terminal, driven key by key, then run in the sim.
#[cfg(test)]
mod scenarios {
    use super::*;
    use nexa_sim::machines::MachineState;
    use nexa_sim::orders::OrdersDef;
    use nexa_sim::SimEvent;
    use nexa_types::{ModuleId, TICKS_PER_SOL};

    const COLS: usize = 80;
    const ROWS: usize = 36;

    struct Session {
        editor: OrdersEditor,
        world: World,
    }

    impl Session {
        fn new(world: World) -> Self {
            Self {
                editor: OrdersEditor::new(),
                world,
            }
        }

        fn press(&mut self, keys: &[KeyCode]) -> Outcome {
            let mut outcome = Outcome::Stay;
            for &key in keys {
                outcome = self.editor.handle(key, &mut self.world);
                self.check_screen(key);
            }
            outcome
        }

        fn down_and_enter(&mut self, n: usize) {
            for _ in 0..n {
                self.press(&[KeyCode::Down]);
            }
            self.press(&[KeyCode::Enter]);
        }

        fn power_trigger(&mut self, percent: u8) {
            self.press(&[KeyCode::T, KeyCode::Enter]);
            let tuning = tuning();
            let steps = (i32::from(percent) - i32::from(tuning.power_start_percent))
                / i32::from(tuning.power_step_percent);
            self.nudge(steps);
            self.press(&[KeyCode::Enter]);
        }

        fn temp_trigger(&mut self, above: bool, module: &str, centi_c: i32) {
            self.press(&[KeyCode::T]);
            self.down_and_enter(if above { 2 } else { 1 });
            let modules = watched(&self.world.colony);
            let i = modules
                .iter()
                .position(|m| m.id().as_str() == module)
                .unwrap();
            let start = modules[i].temp_centi_c().div_euclid(100) * 100;
            self.down_and_enter(i);
            self.nudge((centi_c - start) / tuning().temp_step_centi_c);
            self.press(&[KeyCode::Enter]);
        }

        fn nudge(&mut self, steps: i32) {
            let key = if steps > 0 {
                KeyCode::Right
            } else {
                KeyCode::Left
            };
            for _ in 0..steps.abs() {
                self.press(&[key]);
            }
        }

        fn action(&mut self, kind: ActionKind, module: &str) {
            self.press(&[KeyCode::A]);
            self.down_and_enter(ACTION_KINDS.iter().position(|k| *k == kind).unwrap());
            let i = targets(&self.world.colony, kind)
                .iter()
                .position(|m| m.id().as_str() == module)
                .unwrap_or_else(|| panic!("{module} is not offered for {kind:?}"));
            self.down_and_enter(i);
        }

        fn priority(&mut self, p: u8) {
            self.press(&[KeyCode::Left; 9]);
            for _ in 1..p {
                self.press(&[KeyCode::Right]);
            }
        }

        fn draft(&self) -> &StandingOrder {
            match &self.editor.mode {
                Mode::Draft { order, .. } => order,
                Mode::List => panic!("no draft open"),
            }
        }

        fn screen(&self) -> Terminal {
            // As main.rs draws it: frame, then the editor above the help row.
            let mut t = Terminal::new(COLS, ROWS);
            t.frame(0, 0, COLS, ROWS, Ink::Dim);
            self.editor.draw(&mut t, &self.world, ROWS - 2);
            t
        }

        /// Nothing the editor draws may cover the frame or run off it.
        fn check_screen(&self, after: KeyCode) {
            let t = self.screen();
            for row in 1..ROWS - 1 {
                for col in [0, COLS - 1] {
                    assert_eq!(
                        t.get(col, row).unwrap().ch,
                        '|',
                        "frame overwritten at ({col}, {row}) after {after:?} in {:?}",
                        self.editor.mode
                    );
                }
            }
            let bottom: String = (0..COLS).map(|c| t.get(c, ROWS - 1).unwrap().ch).collect();
            assert_eq!(bottom, format!("+{}+", "-".repeat(COLS - 2)));

            // Content stays clear of the message and the key help.
            let inside = |row: usize| -> String {
                (1..COLS - 1).map(|c| t.get(c, row).unwrap().ch).collect()
            };
            let (help_row, message_row) = (ROWS - 2, ROWS - 4);
            let message = self.editor.message.as_deref().unwrap_or("");
            let expected = format!(" {message:<width$}", width = COLS - 3);
            assert_eq!(
                inside(message_row),
                expected,
                "message row covered after {after:?}"
            );
            assert!(
                inside(ROWS - 3).trim().is_empty(),
                "content under the message after {after:?}"
            );
            let help = t.get(LEFT, help_row).unwrap();
            assert_eq!(
                (help.ch, help.ink),
                ('[', Ink::Dim),
                "help covered after {after:?}"
            );

            // The selected card is always on screen.
            if self.editor.mode == Mode::List && !self.world.orders.is_empty() {
                assert!(
                    (TOP..message_row).any(|r| t.get(LEFT, r).unwrap().ch == '>'),
                    "selected card scrolled off after {after:?}:\n{}",
                    self.text()
                );
            }
        }

        fn text(&self) -> String {
            let t = self.screen();
            (0..ROWS)
                .map(|r| {
                    (0..COLS)
                        .map(|c| t.get(c, r).unwrap().ch)
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    fn caretaker() -> Vec<StandingOrder> {
        OrdersDef::shipped().presets["caretaker"].clone()
    }

    /// Key in the shipped caretaker preset by hand.
    fn key_in_caretaker(s: &mut Session) {
        s.press(&[KeyCode::N]);
        s.power_trigger(25);
        s.action(ActionKind::Shutdown, "smelter_01");
        s.action(ActionKind::Activate, "heater_02");
        s.action(ActionKind::DivertHeat, "hab_01");
        s.priority(1);
        s.press(&[KeyCode::Enter]);
        for (watch, to) in [("hab_01", "hab_02"), ("hab_02", "hab_01")] {
            s.press(&[KeyCode::N]);
            s.temp_trigger(true, watch, 1600);
            s.action(ActionKind::DivertHeat, to);
            s.priority(2);
            s.press(&[KeyCode::Enter]);
        }
    }

    #[test]
    fn keying_in_the_caretaker_preset_saves_both_habs() {
        let mut s = Session::new(World::landfall());
        key_in_caretaker(&mut s);
        assert_eq!(
            s.world.orders.iter().cloned().collect::<Vec<_>>(),
            caretaker()
        );
        assert_eq!(s.editor.message.as_deref(), Some("CARD 3 STORED"));

        let mut world = s.world;
        for _ in 0..10 * TICKS_PER_SOL {
            world.tick();
        }
        for hab in ["hab_01", "hab_02"] {
            let m = world.colony.module(&ModuleId::new(hab)).unwrap();
            assert_eq!(m.machine().unwrap().state(), MachineState::Nominal, "{hab}");
        }
    }

    #[test]
    fn a_keyed_in_card_fires_in_the_sim() {
        let mut s = Session::new(World::landfall());
        s.press(&[KeyCode::N]);
        s.power_trigger(50);
        s.action(ActionKind::Shutdown, "smelter_01");
        s.press(&[KeyCode::Enter]);
        let mut world = s.world;
        let mut reading_when_fired = None;
        for _ in 0..5 * TICKS_PER_SOL {
            // Cards read the gauges before the hour runs; freeing the smelter's
            // load can recharge the battery past 50% within that same hour.
            let before = world.colony.power_percent();
            if world
                .tick()
                .iter()
                .any(|e| matches!(e, SimEvent::OrderFired { slot: 0, .. }))
            {
                reading_when_fired = Some(before);
                break;
            }
        }
        let reading = reading_when_fired.expect("the card never fired");
        assert!(reading < 50, "fired at {reading}%");
        assert!(!world
            .colony
            .module(&ModuleId::new("smelter_01"))
            .unwrap()
            .active());
    }

    #[test]
    fn a_card_built_mid_run_starts_from_the_live_reading() {
        let mut world = World::landfall();
        for _ in 0..4 * TICKS_PER_SOL {
            world.tick();
        }
        let reading = world
            .colony
            .module(&ModuleId::new("hab_02"))
            .unwrap()
            .temp_centi_c();
        let mut s = Session::new(world);
        s.press(&[KeyCode::N, KeyCode::T]);
        s.down_and_enter(1);
        let i = watched(&s.world.colony)
            .iter()
            .position(|m| m.id().as_str() == "hab_02")
            .unwrap();
        s.down_and_enter(i);
        let Mode::Draft {
            step: Step::SetThreshold(Trigger::TempBelow { centi_c, .. }),
            ..
        } = &s.editor.mode
        else {
            panic!("expected the threshold step");
        };
        assert_eq!(*centi_c, reading.div_euclid(100) * 100);
    }

    #[test]
    fn escape_backs_out_one_step_at_a_time_and_keeps_the_draft() {
        let mut s = Session::new(World::landfall());
        s.press(&[KeyCode::N]);
        s.power_trigger(25);
        // Into the action module list, then back out level by level.
        s.press(&[KeyCode::A, KeyCode::Down, KeyCode::Enter]);
        assert!(matches!(
            s.editor.mode,
            Mode::Draft {
                step: Step::PickTarget(ActionKind::Shutdown, _),
                ..
            }
        ));
        s.press(&[KeyCode::Escape]);
        assert!(matches!(
            s.editor.mode,
            Mode::Draft { step: Step::PickAction(p), .. } if p.cursor() == 1
        ));
        s.press(&[KeyCode::Escape]);
        assert!(matches!(
            s.editor.mode,
            Mode::Draft {
                step: Step::Menu,
                ..
            }
        ));
        assert_eq!(s.draft().triggers.len(), 1, "backing out kept the trigger");
        // Threshold and module steps back out too.
        s.press(&[KeyCode::T, KeyCode::Down, KeyCode::Enter, KeyCode::Escape]);
        assert!(matches!(
            s.editor.mode,
            Mode::Draft { step: Step::PickTrigger(p), .. } if p.cursor() == 1
        ));
        s.press(&[KeyCode::Escape, KeyCode::T, KeyCode::Enter, KeyCode::Escape]);
        assert!(matches!(
            s.editor.mode,
            Mode::Draft {
                step: Step::Menu,
                ..
            }
        ));
        assert_eq!(
            s.draft().triggers.len(),
            1,
            "a cancelled threshold adds nothing"
        );
        // Then the draft, then the terminal.
        assert_eq!(s.press(&[KeyCode::Escape]), Outcome::Stay);
        assert_eq!(s.editor.mode, Mode::List);
        assert!(s.world.orders.is_empty());
        assert_eq!(s.press(&[KeyCode::Escape]), Outcome::Close);
    }

    #[test]
    fn unbound_keys_change_nothing_at_any_step() {
        let stray = [
            KeyCode::Key0,
            KeyCode::Key9,
            KeyCode::Z,
            KeyCode::Tab,
            KeyCode::Space,
        ];
        let mut s = Session::new(World::landfall());
        let check = |s: &mut Session| {
            let before = s.editor.mode.clone();
            let orders = s.world.orders.clone();
            for key in stray {
                assert_eq!(s.press(&[key]), Outcome::Stay);
            }
            assert_eq!(s.editor.mode, before);
            assert_eq!(s.world.orders, orders);
        };
        check(&mut s);
        s.press(&[KeyCode::N]);
        check(&mut s);
        s.press(&[KeyCode::T]);
        check(&mut s);
        s.press(&[KeyCode::Down, KeyCode::Enter]);
        check(&mut s);
        s.press(&[KeyCode::Enter]);
        check(&mut s);
        s.press(&[KeyCode::Escape, KeyCode::Escape, KeyCode::A]);
        check(&mut s);
        s.press(&[KeyCode::Enter]);
        check(&mut s);
    }

    #[test]
    fn deleting_every_card_then_one_more() {
        let mut s = Session::new(World::landfall());
        key_in_caretaker(&mut s);
        // The cursor sits on the card just stored.
        assert_eq!(s.editor.list.cursor(), 2);
        s.press(&[KeyCode::Up, KeyCode::D, KeyCode::D]);
        assert_eq!(s.editor.message.as_deref(), Some("CARD 2 DELETED"));
        assert_eq!(s.world.orders.get(1), Some(&caretaker()[2]));
        s.press(&[KeyCode::D, KeyCode::D, KeyCode::Delete, KeyCode::Delete]);
        assert!(s.world.orders.is_empty());
        let message = s.editor.message.clone();
        s.press(&[KeyCode::D, KeyCode::Delete, KeyCode::Up, KeyCode::Enter]);
        assert_eq!(s.editor.message, message, "nothing left to delete");
        assert!(s.text().contains("THE CARETAKER WILL DO NOTHING"));
    }

    #[test]
    fn memory_fills_from_the_editor_and_frees_on_delete() {
        let mut s = Session::new(World::landfall());
        let memory = s.world.orders.tuning().memory;
        for slot in 0..memory {
            s.press(&[KeyCode::N]);
            s.power_trigger(5 * (slot + 1));
            s.action(ActionKind::Activate, "heater_01");
            s.press(&[KeyCode::Enter]);
        }
        assert_eq!(s.world.orders.len(), usize::from(memory));
        s.press(&[KeyCode::N]);
        assert_eq!(s.editor.mode, Mode::List);
        assert_eq!(
            s.editor.message.as_deref(),
            Some("TERMINAL MEMORY FULL: 8 CARDS")
        );
        s.press(&[KeyCode::D, KeyCode::D, KeyCode::N]);
        assert!(matches!(s.editor.mode, Mode::Draft { .. }));
    }

    #[test]
    fn refused_cards_explain_themselves_and_can_be_fixed() {
        let mut s = Session::new(World::landfall());
        s.press(&[KeyCode::N, KeyCode::Enter]);
        assert_eq!(
            s.editor.message.as_deref(),
            Some("A CARD NEEDS AT LEAST ONE TRIGGER")
        );
        s.power_trigger(25);
        s.press(&[KeyCode::Enter]);
        assert_eq!(
            s.editor.message.as_deref(),
            Some("A CARD NEEDS AT LEAST ONE ACTION")
        );
        s.action(ActionKind::ShareHeat, "hab_02");
        s.press(&[KeyCode::Enter]);
        assert_eq!(s.editor.message.as_deref(), Some("CARD 1 STORED"));
        assert!(s
            .text()
            .contains("IF grid.power < 25% THEN share heat ON hab_02"));
    }

    #[test]
    fn power_thresholds_clamp_at_both_ends_from_the_keyboard() {
        let mut s = Session::new(World::landfall());
        s.press(&[KeyCode::N, KeyCode::T, KeyCode::Enter]);
        s.nudge(-30);
        s.press(&[KeyCode::Enter, KeyCode::T, KeyCode::Enter]);
        s.nudge(30);
        s.press(&[KeyCode::Enter]);
        assert_eq!(
            s.draft().triggers,
            vec![
                Trigger::PowerBelowPercent(1),
                Trigger::PowerBelowPercent(100)
            ]
        );
    }

    /// The biggest cards the editor allows still leave the selected card, the
    /// message and the key help on screen.
    #[test]
    fn a_full_terminal_of_long_cards_fits_the_screen() {
        let mut s = Session::new(World::landfall());
        let memory = s.world.orders.tuning().memory;
        for _ in 0..memory {
            s.press(&[KeyCode::N]);
            s.temp_trigger(false, "smelter_01", -450);
            s.temp_trigger(true, "hab_01", 2950);
            s.power_trigger(95);
            for (kind, m) in [
                (ActionKind::Shutdown, "smelter_01"),
                (ActionKind::Activate, "heater_02"),
                (ActionKind::DivertHeat, "hab_02"),
                (ActionKind::ShareHeat, "heater_01"),
            ] {
                s.action(kind, m);
            }
            s.priority(9);
            s.press(&[KeyCode::Enter]);
        }
        assert_eq!(s.world.orders.len(), usize::from(memory));
        let text = s.text();
        assert!(text.contains("CARD 8 STORED"), "message hidden:\n{text}");
        assert!(text.contains("[N] NEW CARD"), "help hidden:\n{text}");
        assert!(
            text.contains("> 8 P9"),
            "selected card scrolled off:\n{text}"
        );
    }

    #[test]
    fn backspace_undoes_the_latest_part_whichever_kind_it_was() {
        let mut s = Session::new(World::landfall());
        s.press(&[KeyCode::N]);
        s.action(ActionKind::Shutdown, "smelter_01");
        s.power_trigger(25);
        s.press(&[KeyCode::Backspace]);
        assert!(s.draft().triggers.is_empty(), "the trigger came last");
        assert_eq!(s.draft().actions.len(), 1);
        s.press(&[KeyCode::Backspace, KeyCode::Backspace]);
        assert!(s.draft().actions.is_empty());
    }

    #[test]
    fn power_steps_by_one_below_the_first_step() {
        let mut s = Session::new(World::landfall());
        s.press(&[KeyCode::N]);
        s.power_trigger(5);
        s.press(&[KeyCode::T, KeyCode::Enter]);
        // 25 down to 5 in whole steps, then 4, 3, 2, 1 (and no lower), then back up by one.
        s.nudge(-9);
        s.press(&[KeyCode::Right, KeyCode::Enter]);
        assert_eq!(
            s.draft().triggers,
            vec![Trigger::PowerBelowPercent(5), Trigger::PowerBelowPercent(2)]
        );
    }

    #[test]
    fn delete_asks_first_and_any_other_key_cancels() {
        let mut s = Session::new(World::landfall());
        key_in_caretaker(&mut s);
        s.press(&[KeyCode::D]);
        assert_eq!(s.world.orders.len(), 3);
        assert_eq!(
            s.editor.message.as_deref(),
            Some("DELETE CARD 3? [D] AGAIN TO CONFIRM")
        );
        // Moving away cancels, so the second D only asks about card 2.
        s.press(&[KeyCode::Up, KeyCode::D]);
        assert_eq!(s.world.orders.len(), 3);
        // Esc cancels the question without leaving the terminal.
        assert_eq!(s.press(&[KeyCode::Escape]), Outcome::Stay);
        assert_eq!(s.editor.message, None);
        s.press(&[KeyCode::D]);
        assert_eq!(s.world.orders.len(), 3);
        s.press(&[KeyCode::D]);
        assert_eq!(s.world.orders.len(), 2);
        assert_eq!(s.world.orders.get(1), Some(&caretaker()[2]));
    }

    #[test]
    fn backspace_on_the_list_stays_in_the_terminal() {
        let mut s = Session::new(World::landfall());
        s.press(&[KeyCode::N]);
        s.power_trigger(25);
        s.press(&[KeyCode::Backspace, KeyCode::Backspace, KeyCode::Escape]);
        assert_eq!(s.editor.mode, Mode::List);
        assert_eq!(s.press(&[KeyCode::Backspace]), Outcome::Stay);
        assert_eq!(s.press(&[KeyCode::Escape]), Outcome::Close);
    }

    /// The sim puts no limit on parts per card, so the screen has to cope.
    #[test]
    fn an_endless_draft_keeps_the_message_and_help_on_screen() {
        let mut s = Session::new(World::landfall());
        s.press(&[KeyCode::N]);
        for i in 0..20 {
            s.temp_trigger(i % 2 == 0, "hab_01", 1000 + 50 * i);
        }
        s.action(ActionKind::Activate, "heater_01");
        // check_screen has run after every key; the latest part is still visible.
        let text = s.text();
        assert!(text.contains("activate heater_01"), "{text}");
        s.press(&[KeyCode::A]);
        assert!(s.text().contains("ADD ACTION: DO WHAT?"));
    }

    #[test]
    fn the_list_scrolls_to_the_selected_card() {
        let mut s = Session::new(World::landfall());
        let memory = s.world.orders.tuning().memory;
        for _ in 0..memory {
            s.press(&[KeyCode::N]);
            for i in 0..8 {
                s.temp_trigger(i % 2 == 0, "smelter_01", -4000 + 50 * i);
            }
            s.action(ActionKind::Shutdown, "smelter_01");
            s.press(&[KeyCode::Enter]);
        }
        assert_eq!(s.world.orders.len(), usize::from(memory));
        // check_screen asserts the selected card is drawn after each key; a full lap
        // visits every card and ends back on the last.
        for _ in 0..memory {
            s.press(&[KeyCode::Down]);
        }
        s.press(&[KeyCode::Down]);
        let text = s.text();
        assert!(text.contains("> 1 P1"), "wrapped back to the top:\n{text}");
        s.press(&[KeyCode::Up]);
        assert!(s.text().contains("> 8 P1"));
    }
}
