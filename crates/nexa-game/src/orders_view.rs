//! The Caretaker terminal: postures to load, the colony's Standing Orders, and
//! (under Advanced) a builder for new cards.
//!
//! It opens on the postures: pick one and its cards replace the loaded ones.
//! Cards are built by picking from lists, never typed (GDD: rule cards). Every
//! change goes through `World::load_posture`, `World::add_order` or
//! `World::remove_order`, so the sim checks each card before storing it.

use crate::terminal::{Ink, Terminal};
use crate::ui::{wrap, ListPicker, Outcome, Pick};
use macroquad::prelude::KeyCode;
use nexa_sim::colony::{Colony, Module};
use nexa_sim::orders::{
    format_centi_c, Action, EditorTuning, OrderError, OrdersDef, Preset, StandingOrder, Trigger,
    POWER_PERCENT,
};
use nexa_sim::World;
use nexa_types::{ModuleId, CENTI_PER_DEGREE};

const LEFT: usize = 2;
const TOP: usize = 3;
/// Inside the frame, with a column of margin on each side.
const WIDTH: usize = 76;
/// "> 1 P1 " in front of each card in the list.
const CARD_INDENT: usize = 7;
/// "> " in front of each posture's name; its summary lines up under the name.
const POSTURE_INDENT: usize = 2;
/// Priority 1 runs first, so it's where every new card starts.
const FIRST_PRIORITY: u8 = 1;

fn tuning() -> EditorTuning {
    OrdersDef::shipped().editor
}

fn card_count(n: usize) -> String {
    match n {
        1 => "1 CARD".into(),
        n => format!("{n} CARDS"),
    }
}

/// The shipped postures, in the order the picker lists them.
fn postures() -> Vec<&'static Preset> {
    OrdersDef::shipped().presets.values().collect()
}

/// Key help for every list-picking step.
const PICK_HELP: &str = "[UP/DOWN] SELECT   [ENTER] CHOOSE   [ESC] BACK";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TriggerKind {
    PowerBelow,
    /// Watches a module, so a module picker comes before the threshold.
    Temp(TempSide),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TempSide {
    Below,
    Above,
}

const TRIGGER_KINDS: [TriggerKind; 3] = [
    TriggerKind::PowerBelow,
    TriggerKind::Temp(TempSide::Below),
    TriggerKind::Temp(TempSide::Above),
];

impl TriggerKind {
    fn label(self) -> &'static str {
        match self {
            TriggerKind::PowerBelow => "grid.power < N%",
            TriggerKind::Temp(TempSide::Below) => "MODULE.temp < N",
            TriggerKind::Temp(TempSide::Above) => "MODULE.temp > N",
        }
    }
}

impl TempSide {
    fn build(self, module: ModuleId, centi_c: i32) -> Trigger {
        match self {
            TempSide::Below => Trigger::TempBelow { module, centi_c },
            TempSide::Above => Trigger::TempAbove { module, centi_c },
        }
    }
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
            ActionKind::Activate | ActionKind::Shutdown => m.def().uses_power(),
            ActionKind::DivertHeat | ActionKind::ShareHeat => m.heat_net().is_some(),
        }
    }
}

/// Where `kind` sits in its picker list, so backing out returns the cursor to it.
fn slot_of<T: PartialEq>(kinds: &[T], kind: &T) -> usize {
    kinds.iter().position(|k| k == kind).unwrap_or(0)
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
    PickWatched(TempSide, ListPicker),
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
    /// Where the terminal opens: pick a posture to load.
    Postures,
    /// Advanced: the cards one by one, to delete or add to.
    List,
    Draft {
        order: StandingOrder,
        step: Step,
        added: Vec<Part>,
    },
}

pub struct OrdersEditor {
    mode: Mode,
    postures: ListPicker,
    list: ListPicker,
    /// The last thing the terminal has to say: a stored card or why it was refused.
    message: Option<String>,
    /// The posture a first Enter asked to load over the cards; a second Enter loads it.
    confirm_load: Option<usize>,
    /// The slot a first D press asked to delete; a second D deletes it.
    confirm_delete: Option<usize>,
}

impl OrdersEditor {
    pub fn new() -> Self {
        Self {
            mode: Mode::Postures,
            postures: ListPicker::default(),
            list: ListPicker::default(),
            message: None,
            confirm_load: None,
            confirm_delete: None,
        }
    }

    pub fn handle(&mut self, key: KeyCode, world: &mut World) -> Outcome {
        match std::mem::replace(&mut self.mode, Mode::List) {
            Mode::Postures => return self.handle_postures(key, world),
            Mode::List => self.handle_list(key, world),
            Mode::Draft { order, step, added } => self.handle_draft(key, world, order, step, added),
        }
        Outcome::Stay
    }

    fn handle_postures(&mut self, key: KeyCode, world: &mut World) -> Outcome {
        self.mode = Mode::Postures;
        let postures = postures();
        let confirming = self.confirm_load.take();
        let enter = matches!(key, KeyCode::Enter | KeyCode::KpEnter);
        if confirming.is_some() && !enter {
            self.message = None;
            // Esc only cancels the question; it takes a second Esc to leave.
            if key == KeyCode::Escape {
                return Outcome::Stay;
            }
        }
        match key {
            KeyCode::A => {
                self.message = None;
                self.mode = Mode::List;
            }
            // Backspace is undo in the builder; here it must not close the terminal.
            KeyCode::Backspace => {}
            _ => match self.postures.handle(key, postures.len()) {
                Pick::Chosen(i) => {
                    let posture = postures[i];
                    if world.orders.is_empty() || confirming == Some(i) {
                        self.message = Some(match world.load_posture(posture) {
                            Ok(()) => {
                                self.list = ListPicker::default();
                                format!(
                                    "{} LOADED: {}",
                                    posture.name.to_uppercase(),
                                    card_count(world.orders.len())
                                )
                            }
                            Err(e) => e.to_string().to_uppercase(),
                        });
                    } else {
                        self.confirm_load = Some(i);
                        self.message = Some(format!(
                            "REPLACE {} WITH {}? [ENTER] AGAIN TO CONFIRM",
                            card_count(world.orders.len()),
                            posture.name.to_uppercase()
                        ));
                    }
                }
                Pick::Back => return Outcome::Close,
                Pick::Pending => {}
            },
        }
        Outcome::Stay
    }

    fn handle_list(&mut self, key: KeyCode, world: &mut World) {
        let count = world.orders.len();
        let confirming = self.confirm_delete.take();
        let delete = matches!(key, KeyCode::D | KeyCode::Delete);
        if confirming.is_some() && !delete {
            self.message = None;
            // Esc only cancels the delete; it takes a second Esc to leave.
            if key == KeyCode::Escape {
                return;
            }
        }
        match key {
            KeyCode::N => {
                if world.orders.is_full() {
                    let memory = world.orders.tuning().memory;
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
                    self.message = None;
                    self.mode = Mode::Postures;
                }
            }
        }
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
        let colony = &world.colony;
        let step = match step {
            Step::Menu => match self.menu_key(key, world, &mut order, &mut added, &tuning) {
                Some(step) => step,
                // Stored or discarded: the draft is gone.
                None => return,
            },
            Step::PickTrigger(picker) => trigger_key(key, picker, &tuning),
            Step::PickWatched(side, picker) => watched_key(key, colony, side, picker, &tuning),
            Step::SetThreshold(trigger) => match threshold_key(key, trigger, &tuning) {
                Threshold::Editing(trigger) => Step::SetThreshold(trigger),
                Threshold::Add(trigger) => {
                    order.triggers.push(trigger);
                    added.push(Part::Trigger);
                    Step::Menu
                }
                Threshold::Cancel => Step::Menu,
            },
            Step::PickAction(picker) => action_key(key, picker),
            Step::PickTarget(kind, mut picker) => {
                let modules = targets(colony, kind);
                match picker.handle(key, modules.len()) {
                    Pick::Chosen(i) => {
                        order.actions.push(kind.build(modules[i]));
                        added.push(Part::Action);
                        Step::Menu
                    }
                    Pick::Back => Step::PickAction(ListPicker::at(slot_of(&ACTION_KINDS, &kind))),
                    Pick::Pending => Step::PickTarget(kind, picker),
                }
            }
        };
        self.mode = Mode::Draft { order, step, added };
    }

    /// The draft's top level. `None` when the card was stored or discarded.
    fn menu_key(
        &mut self,
        key: KeyCode,
        world: &mut World,
        order: &mut StandingOrder,
        added: &mut Vec<Part>,
        tuning: &EditorTuning,
    ) -> Option<Step> {
        match key {
            KeyCode::T => return Some(Step::PickTrigger(ListPicker::default())),
            KeyCode::A => return Some(Step::PickAction(ListPicker::default())),
            KeyCode::Left => {
                order.priority = order.priority.saturating_sub(1).max(FIRST_PRIORITY);
            }
            KeyCode::Right => {
                order.priority = order.priority.saturating_add(1).min(tuning.priority_max);
            }
            KeyCode::Backspace => match added.pop() {
                Some(Part::Trigger) => {
                    order.triggers.pop();
                }
                Some(Part::Action) => {
                    order.actions.pop();
                }
                None => {}
            },
            KeyCode::Enter | KeyCode::KpEnter => match world.add_order(order.clone()) {
                Ok(slot) => {
                    self.message = Some(format!("CARD {} STORED", slot + 1));
                    self.list = ListPicker::at(slot);
                    return None;
                }
                Err(e) => self.say(e),
            },
            KeyCode::Escape => {
                self.message = Some("CARD DISCARDED".into());
                return None;
            }
            _ => {}
        }
        Some(Step::Menu)
    }

    fn say(&mut self, error: OrderError) {
        self.message = Some(error.to_string().to_uppercase());
    }

    /// Draw everything between the title row and `help_row`, which gets the key help.
    pub fn draw(&self, t: &mut Terminal, world: &World, help_row: usize) {
        let message_row = help_row - 2;
        let help = match &self.mode {
            Mode::Postures => {
                self.draw_postures(t, world, message_row);
                "[UP/DOWN] SELECT   [ENTER] LOAD   [A] ADVANCED   [ESC] BACK TO COLONY"
            }
            Mode::List => {
                draw_cards(t, world, TOP, message_row, Some(self.list.cursor()));
                "[UP/DOWN] SELECT   [N] NEW CARD   [D] DELETE   [ESC] POSTURES"
            }
            Mode::Draft { order, step, .. } => draw_draft(t, world, order, step, message_row),
        };
        if let Some(message) = &self.message {
            t.put_str(LEFT, message_row, message, Ink::Bright);
        }
        t.put_str(LEFT, help_row, help, Ink::Dim);
    }

    /// Each posture's name and summary, then the cards loaded now below them.
    fn draw_postures(&self, t: &mut Terminal, world: &World, bottom: usize) {
        t.put_str(LEFT, TOP, "POSTURES", Ink::Bright);
        let mut row = TOP + 2;
        for (i, posture) in postures().into_iter().enumerate() {
            let selected = i == self.postures.cursor();
            let (mark, ink) = if selected {
                ('>', Ink::Bright)
            } else {
                (' ', Ink::Amber)
            };
            let mut lines = vec![(format!("{mark} {}", posture.name), ink)];
            lines.extend(
                wrap(&posture.summary, WIDTH - POSTURE_INDENT)
                    .into_iter()
                    .map(|l| (format!("{:POSTURE_INDENT$}{l}", ""), Ink::Dim)),
            );
            for (line, ink) in lines {
                if row >= bottom {
                    return;
                }
                t.put_str(LEFT, row, &line, ink);
                row += 1;
            }
        }
        draw_cards(t, world, row + 1, bottom, None);
    }
}

/// The loaded cards from `top` to above `bottom`: each card's plain-words line,
/// then its command line dimmer. With a `cursor`, that card is marked and kept
/// on screen; without one, cards that don't fit end in `...`.
fn draw_cards(t: &mut Terminal, world: &World, top: usize, bottom: usize, cursor: Option<usize>) {
    let orders = &world.orders;
    if top >= bottom {
        return;
    }
    t.put_str(LEFT, top, "STANDING ORDERS", Ink::Bright);
    let memory = format!("MEMORY {}/{}", orders.len(), orders.tuning().memory);
    t.put_str(LEFT + WIDTH - memory.len(), top, &memory, Ink::Amber);
    if orders.is_empty() {
        if top + 2 < bottom {
            t.put_str(
                LEFT,
                top + 2,
                "NO CARDS LOADED. THE CARETAKER WILL DO NOTHING.",
                Ink::Dim,
            );
        }
        return;
    }
    let width = WIDTH - CARD_INDENT;
    let cards: Vec<(Vec<String>, Vec<String>)> = orders
        .iter()
        .map(|o| {
            (
                wrap(&o.describe(&world.colony), width),
                wrap(&o.render(), width),
            )
        })
        .collect();
    let height = |c: &(Vec<String>, Vec<String>)| c.0.len() + c.1.len();
    // Scroll by whole cards until the selected one fits above `bottom`.
    let room = bottom.saturating_sub(top + 2);
    let mut first = 0;
    if let Some(cursor) = cursor {
        while first < cursor && cards[first..=cursor].iter().map(height).sum::<usize>() > room {
            first += 1;
        }
    }
    // Without a cursor nothing scrolls, so cards that don't fit end in "...".
    let cut = cursor.is_none() && cards.iter().map(height).sum::<usize>() > room;
    let mut row = top + 2;
    for (slot, (order, (words, command))) in orders.iter().zip(&cards).enumerate().skip(first) {
        let (mark, ink) = if Some(slot) == cursor {
            ('>', Ink::Bright)
        } else {
            (' ', Ink::Amber)
        };
        let lines = words
            .iter()
            .map(|l| (l, ink))
            .chain(command.iter().map(|l| (l, Ink::Dim)));
        for (i, (line, ink)) in lines.enumerate() {
            if row >= bottom {
                return;
            }
            if cut && row + 1 == bottom {
                t.put_str(LEFT + CARD_INDENT, row, "...", Ink::Dim);
                return;
            }
            if i == 0 {
                t.put_str(
                    LEFT,
                    row,
                    &format!("{mark} {} P{}", slot + 1, order.priority),
                    ink,
                );
            }
            t.put_str(LEFT + CARD_INDENT, row, line, ink);
            row += 1;
        }
    }
}

fn trigger_key(key: KeyCode, mut picker: ListPicker, tuning: &EditorTuning) -> Step {
    match picker.handle(key, TRIGGER_KINDS.len()) {
        Pick::Chosen(i) => match TRIGGER_KINDS[i] {
            TriggerKind::PowerBelow => {
                Step::SetThreshold(Trigger::PowerBelowPercent(tuning.power_start_percent))
            }
            TriggerKind::Temp(side) => Step::PickWatched(side, ListPicker::default()),
        },
        Pick::Back => Step::Menu,
        Pick::Pending => Step::PickTrigger(picker),
    }
}

fn watched_key(
    key: KeyCode,
    colony: &Colony,
    side: TempSide,
    mut picker: ListPicker,
    tuning: &EditorTuning,
) -> Step {
    let modules = watched(colony);
    match picker.handle(key, modules.len()) {
        Pick::Chosen(i) => {
            let m = modules[i];
            // Start from the current reading, rounded down to a whole degree.
            let centi_c = (m.temp_centi_c().div_euclid(CENTI_PER_DEGREE) * CENTI_PER_DEGREE)
                .clamp(tuning.temp_min_centi_c, tuning.temp_max_centi_c);
            Step::SetThreshold(side.build(m.id().clone(), centi_c))
        }
        Pick::Back => Step::PickTrigger(ListPicker::at(slot_of(
            &TRIGGER_KINDS,
            &TriggerKind::Temp(side),
        ))),
        Pick::Pending => Step::PickWatched(side, picker),
    }
}

/// A threshold being keyed in, or finished.
enum Threshold {
    Editing(Trigger),
    Add(Trigger),
    Cancel,
}

fn threshold_key(key: KeyCode, mut trigger: Trigger, tuning: &EditorTuning) -> Threshold {
    match key {
        KeyCode::Right | KeyCode::Up => nudge(&mut trigger, true, tuning),
        KeyCode::Left | KeyCode::Down => nudge(&mut trigger, false, tuning),
        KeyCode::Enter | KeyCode::KpEnter => return Threshold::Add(trigger),
        KeyCode::Escape | KeyCode::Backspace => return Threshold::Cancel,
        _ => {}
    }
    Threshold::Editing(trigger)
}

fn action_key(key: KeyCode, mut picker: ListPicker) -> Step {
    match picker.handle(key, ACTION_KINDS.len()) {
        Pick::Chosen(i) => Step::PickTarget(ACTION_KINDS[i], ListPicker::default()),
        Pick::Back => Step::Menu,
        Pick::Pending => Step::PickAction(picker),
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
    let lines = wrap(&order.render(), WIDTH);
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
            let items: Vec<String> = TRIGGER_KINDS
                .iter()
                .map(|k| k.label().to_string())
                .collect();
            picker.draw(t, LEFT, row + 2, height, &items);
            PICK_HELP
        }
        Step::PickWatched(_, picker) => {
            t.put_str(LEFT, row, "ADD TRIGGER: WHICH MODULE?", Ink::Bright);
            let items: Vec<String> = watched(colony)
                .iter()
                .map(|m| format!("{:<12}{}", m.id(), format_centi_c(m.temp_centi_c())))
                .collect();
            picker.draw(t, LEFT, row + 2, height, &items);
            PICK_HELP
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
            PICK_HELP
        }
        Step::PickTarget(kind, picker) => {
            t.put_str(LEFT, row, "ADD ACTION: WHICH MODULE?", Ink::Bright);
            let items: Vec<String> = targets(colony, *kind)
                .iter()
                .map(|m| m.id().to_string())
                .collect();
            picker.draw(t, LEFT, row + 2, height, &items);
            PICK_HELP
        }
    }
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
            .clamp(*POWER_PERCENT.start(), *POWER_PERCENT.end());
        }
        Trigger::TempBelow { centi_c, .. } | Trigger::TempAbove { centi_c, .. } => {
            let step = tuning.temp_step_centi_c;
            let step = if up { step } else { -step };
            *centi_c = (*centi_c + step).clamp(tuning.temp_min_centi_c, tuning.temp_max_centi_c);
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod scenarios;
