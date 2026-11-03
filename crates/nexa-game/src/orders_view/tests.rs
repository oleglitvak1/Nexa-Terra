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
