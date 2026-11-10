//! Whole sessions at the terminal, driven key by key, then run in the sim.

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
    /// The terminal as `O` opens it, on the postures.
    fn new(world: World) -> Self {
        let s = Self {
            editor: OrdersEditor::new(),
            world,
        };
        s.check_screen(KeyCode::O);
        s
    }

    /// Opened, then [A] for the card list and builder.
    fn advanced(world: World) -> Self {
        let mut s = Self::new(world);
        s.press(&[KeyCode::A]);
        assert_eq!(s.editor.mode, Mode::List);
        s
    }

    /// Move the posture picker onto `id` and press Enter.
    fn pick_posture(&mut self, id: &str) {
        let i = OrdersDef::shipped()
            .presets
            .keys()
            .position(|k| k == id)
            .unwrap();
        let steps = (i + postures().len() - self.editor.postures.cursor()) % postures().len();
        for _ in 0..steps {
            self.press(&[KeyCode::Down]);
        }
        self.press(&[KeyCode::Enter]);
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
            Mode::Postures | Mode::List => panic!("no draft open"),
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
        let inside =
            |row: usize| -> String { (1..COLS - 1).map(|c| t.get(c, row).unwrap().ch).collect() };
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

        // The selected card or posture is always on screen.
        let something_selected = match self.editor.mode {
            Mode::Postures => true,
            Mode::List => !self.world.orders.is_empty(),
            Mode::Draft { .. } => false,
        };
        if something_selected {
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

fn protect_habs() -> Vec<StandingOrder> {
    OrdersDef::shipped().presets["protect-habs"].cards.clone()
}

/// Key in the shipped Protect the habs posture by hand.
fn key_in_protect_habs(s: &mut Session) {
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
fn keying_in_the_protect_habs_preset_saves_both_habs() {
    let mut s = Session::advanced(World::landfall());
    key_in_protect_habs(&mut s);
    assert_eq!(
        s.world.orders.iter().cloned().collect::<Vec<_>>(),
        protect_habs()
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
    let mut s = Session::advanced(World::landfall());
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
    let mut s = Session::advanced(world);
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
    let mut s = Session::advanced(World::landfall());
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
    // Then the draft, then the card list, then the terminal.
    assert_eq!(s.press(&[KeyCode::Escape]), Outcome::Stay);
    assert_eq!(s.editor.mode, Mode::List);
    assert!(s.world.orders.is_empty());
    assert_eq!(s.press(&[KeyCode::Escape]), Outcome::Stay);
    assert_eq!(s.editor.mode, Mode::Postures);
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
    s.press(&[KeyCode::A]);
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
    let mut s = Session::advanced(World::landfall());
    key_in_protect_habs(&mut s);
    // The cursor sits on the card just stored.
    assert_eq!(s.editor.list.cursor(), 2);
    s.press(&[KeyCode::Up, KeyCode::D, KeyCode::D]);
    assert_eq!(s.editor.message.as_deref(), Some("CARD 2 DELETED"));
    assert_eq!(s.world.orders.get(1), Some(&protect_habs()[2]));
    s.press(&[KeyCode::D, KeyCode::D, KeyCode::Delete, KeyCode::Delete]);
    assert!(s.world.orders.is_empty());
    let message = s.editor.message.clone();
    s.press(&[KeyCode::D, KeyCode::Delete, KeyCode::Up, KeyCode::Enter]);
    assert_eq!(s.editor.message, message, "nothing left to delete");
    assert!(s.text().contains("THE CARETAKER WILL DO NOTHING"));
}

#[test]
fn memory_fills_from_the_editor_and_frees_on_delete() {
    let mut s = Session::advanced(World::landfall());
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
    let mut s = Session::advanced(World::landfall());
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
    let mut s = Session::advanced(World::landfall());
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
    let mut s = Session::advanced(World::landfall());
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
    // Under the postures the list doesn't scroll: it ends in "..." above the message.
    s.press(&[KeyCode::Escape]);
    let text = s.text();
    assert!(text.contains("\n|        ..."), "no cut marker:\n{text}");
}

#[test]
fn backspace_undoes_the_latest_part_whichever_kind_it_was() {
    let mut s = Session::advanced(World::landfall());
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
    let mut s = Session::advanced(World::landfall());
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
    let mut s = Session::advanced(World::landfall());
    key_in_protect_habs(&mut s);
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
    assert_eq!(s.world.orders.get(1), Some(&protect_habs()[2]));
}

#[test]
fn backspace_on_the_list_stays_in_the_terminal() {
    let mut s = Session::advanced(World::landfall());
    s.press(&[KeyCode::N]);
    s.power_trigger(25);
    s.press(&[KeyCode::Backspace, KeyCode::Backspace, KeyCode::Escape]);
    assert_eq!(s.editor.mode, Mode::List);
    assert_eq!(s.press(&[KeyCode::Backspace]), Outcome::Stay);
    assert_eq!(
        s.press(&[KeyCode::Escape, KeyCode::Backspace]),
        Outcome::Stay
    );
    assert_eq!(s.editor.mode, Mode::Postures);
    assert_eq!(s.press(&[KeyCode::Escape]), Outcome::Close);
}

/// The sim puts no limit on parts per card, so the screen has to cope.
#[test]
fn an_endless_draft_keeps_the_message_and_help_on_screen() {
    let mut s = Session::advanced(World::landfall());
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
    let mut s = Session::advanced(World::landfall());
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

fn habs_nominal_after_10_sols(mut world: World) {
    for _ in 0..10 * TICKS_PER_SOL {
        world.tick();
    }
    for hab in ["hab_01", "hab_02"] {
        let m = world.colony.module(&ModuleId::new(hab)).unwrap();
        assert_eq!(m.machine().unwrap().state(), MachineState::Nominal, "{hab}");
    }
}

#[test]
fn picking_protect_the_habs_loads_its_three_cards_and_the_habs_survive_sol_10() {
    let mut s = Session::new(World::landfall());
    s.pick_posture("protect-habs");
    assert_eq!(
        s.world.orders.iter().cloned().collect::<Vec<_>>(),
        protect_habs()
    );
    assert_eq!(
        s.editor.message.as_deref(),
        Some("PROTECT THE HABS LOADED: 3 CARDS")
    );
    let text = s.text();
    assert!(text.contains("send heat to hab 1."), "{text}");
    habs_nominal_after_10_sols(s.world);
}

#[test]
fn loading_over_cards_asks_first_then_replaces_them() {
    let mut world = World::landfall();
    world
        .load_posture(&OrdersDef::shipped().presets["doc-example"])
        .unwrap();
    let doc_example = world.orders.clone();
    let mut s = Session::new(world);
    s.pick_posture("protect-habs");
    assert_eq!(
        s.world.orders, doc_example,
        "nothing changes on the first Enter"
    );
    assert_eq!(
        s.editor.message.as_deref(),
        Some("REPLACE 1 CARD WITH PROTECT THE HABS? [ENTER] AGAIN TO CONFIRM")
    );
    // Moving away cancels, so the next Enter only asks again, about the new pick.
    s.press(&[KeyCode::Up, KeyCode::Enter]);
    assert_eq!(s.world.orders, doc_example);
    // Esc cancels the question without leaving the terminal.
    assert_eq!(s.press(&[KeyCode::Escape]), Outcome::Stay);
    assert_eq!(s.editor.message, None);
    s.press(&[KeyCode::Down, KeyCode::Enter]);
    assert_eq!(s.world.orders, doc_example);
    s.press(&[KeyCode::Enter]);
    assert_eq!(
        s.world.orders.iter().cloned().collect::<Vec<_>>(),
        protect_habs(),
        "the old card is gone, not kept alongside"
    );
}

#[test]
fn a_posture_loaded_after_hand_built_cards_replaces_them() {
    let mut s = Session::advanced(World::landfall());
    s.press(&[KeyCode::N]);
    s.power_trigger(50);
    s.action(ActionKind::Shutdown, "heater_01");
    s.press(&[KeyCode::Enter, KeyCode::Escape]);
    assert_eq!(s.editor.mode, Mode::Postures);
    s.pick_posture("protect-habs");
    s.press(&[KeyCode::Enter]);
    assert_eq!(
        s.world.orders.iter().cloned().collect::<Vec<_>>(),
        protect_habs()
    );
    // The card list shows the loaded posture's cards.
    s.press(&[KeyCode::A]);
    assert!(s.text().contains("> 1 P1 When power drops below 25%"));
}
