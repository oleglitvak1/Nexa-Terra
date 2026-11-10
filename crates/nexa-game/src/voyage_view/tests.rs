//! The voyage screens driven key by key, with the frame checked after every key.

use super::*;
use nexa_sim::crew::Assignment;
use nexa_sim::flight::Paid;
use nexa_sim::hazards::HazardsDef;
use nexa_sim::universe::OfflineUniverse;
use nexa_types::CrewId;
use std::collections::BTreeMap;

const COLS: usize = 80;
const ROWS: usize = 36;
const HELP_ROW: usize = ROWS - 2;

struct Session {
    screen: VoyageScreen,
    world: World,
}

/// Cal, Jonathan and Dax fly; launched with `rules`.
fn launched_with(rules: HazardsDef, credits: u32) -> Session {
    let mut world = World::landfall_seeded(9);
    world.credits = credits;
    // A full hold of the smelter's crates, whatever the shipped manifest holds.
    let ore = world.colony.ore_def().expect("the smelter makes ore");
    world.ship.unload();
    assert_eq!(world.ship.load_crates(10, ore), 10);
    let split: BTreeMap<CrewId, Assignment> = ["cal", "jonathan", "dax"]
        .iter()
        .map(|id| (CrewId::new(*id), Assignment::Flight))
        .collect();
    world.confirm_split(&split).unwrap();
    let events = world
        .launch_with_rules(&mut OfflineUniverse::default(), rules)
        .unwrap();
    Session {
        screen: VoyageScreen::new(&world, &events),
        world,
    }
}

fn launched() -> Session {
    launched_with(HazardsDef::shipped().clone(), 500)
}

impl Session {
    fn press(&mut self, keys: &[KeyCode]) -> Outcome {
        let mut outcome = Outcome::Stay;
        for &key in keys {
            outcome = self.screen.handle(key, &mut self.world);
            self.check_frame(key);
        }
        outcome
    }

    /// Fly Sol by Sol until the hazard terminal opens.
    fn fly_to_hazard(&mut self) {
        for _ in 0..10 {
            if self.world.voyage.as_ref().unwrap().pending().is_some() {
                return;
            }
            self.press(&[KeyCode::Enter]);
        }
        panic!("no hazard:\n{}", self.text());
    }

    /// Pick the option with `id` on the hazard terminal.
    fn choose(&mut self, id: &str) {
        let hazard = self.world.voyage.as_ref().unwrap().pending().unwrap();
        let i = hazard.options.iter().position(|o| o.id == id).unwrap();
        while self.screen.options.cursor() != i {
            self.press(&[KeyCode::Down]);
        }
        self.press(&[KeyCode::Enter]);
    }

    fn terminal(&self) -> Terminal {
        let mut t = Terminal::new(COLS, ROWS);
        t.frame(0, 0, COLS, ROWS, Ink::Dim);
        self.screen.draw(&mut t, &self.world, HELP_ROW);
        t
    }

    fn text(&self) -> String {
        let t = self.terminal();
        (0..ROWS)
            .map(|r| row_text(&t, r))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn row_with(&self, needle: &str) -> (String, Ink) {
        let t = self.terminal();
        for r in 0..ROWS {
            let text = row_text(&t, r);
            if let Some(col) = text.find(needle) {
                return (text, t.get(col, r).unwrap().ink);
            }
        }
        panic!("no row with {needle}:\n{}", self.text())
    }

    fn message(&self) -> String {
        row_text(&self.terminal(), HELP_ROW - 2)
            .trim()
            .trim_matches('|')
            .trim()
            .to_string()
    }

    /// Nothing may cover the frame or the column of margin inside it.
    fn check_frame(&self, after: KeyCode) {
        let t = self.terminal();
        for row in 1..ROWS - 1 {
            assert_eq!(
                t.get(COLS - 2, row).unwrap().ch,
                ' ',
                "no right margin on row {row} after {after:?}:\n{}",
                self.text()
            );
            for col in [0, COLS - 1] {
                assert_eq!(
                    t.get(col, row).unwrap().ch,
                    '|',
                    "frame overwritten at ({col}, {row}) after {after:?}:\n{}",
                    self.text()
                );
            }
        }
    }
}

fn row_text(t: &Terminal, row: usize) -> String {
    (0..t.cols()).map(|c| t.get(c, row).unwrap().ch).collect()
}

#[test]
fn shows_the_ship_after_launch() {
    let s = launched();
    let text = s.text();
    assert!(
        text.contains("THE SLIP // OUTBOUND TO SCRAP BARGE 77"),
        "{text}"
    );
    assert!(text.contains("SOL 0 OF 5"), "{text}");
    assert!(text.contains("650/1000 L"), "{text}");
    assert!(
        text.contains("10 CRATES  10000 KG  INTEGRITY 100%"),
        "{text}"
    );
    assert!(text.contains("ECM PODS 1   CREDITS 500"), "{text}");
    assert!(text.contains("JONATHAN, CAL, DAX"), "{text}");
    assert!(text.contains("LAUNCH: 3 ABOARD"), "{text}");
    assert!(text.contains(HELP_TRANSIT), "{text}");
}

#[test]
fn flying_stops_at_the_hazard_terminal() {
    let mut s = launched();
    s.fly_to_hazard();
    let text = s.text();
    assert!(text.contains("PIRATE INTERDICTION"), "{text}");
    for id in ["ECM", "OVERCLOCK", "DUMP", "PAY", "BLUFF"] {
        assert!(text.contains(id), "{id}:\n{text}");
    }
    assert!(s.row_with("ECM       ").0.contains("PILOTING CHECK"));
    assert!(s.row_with("OVERCLOCK").0.contains("150 L"));
    assert!(s.row_with("PAY       ").0.contains("300 CR"));
    assert!(text.contains(HELP_HAZARD), "{text}");
    // Enter doesn't fly past it: it commits the selected option.
    let flown = s.world.voyage.as_ref().unwrap().sols_flown();
    s.press(&[KeyCode::Escape]);
    assert_eq!(s.world.voyage.as_ref().unwrap().sols_flown(), flown);
    assert!(s.world.voyage.as_ref().unwrap().pending().is_some());
}

#[test]
fn ransom_is_greyed_without_the_credits() {
    let mut s = launched_with(HazardsDef::shipped().clone(), 100);
    s.fly_to_hazard();
    let (row, ink) = s.row_with("PAY       ");
    assert!(row.contains("(NOT ENOUGH CREDITS)"), "{row}");
    assert_eq!(ink, Ink::Dim);
    let before = s.world.clone();
    s.choose("pay");
    assert_eq!(s.message(), "UNAVAILABLE: NOT ENOUGH CREDITS");
    assert_eq!(s.world, before);
}

#[test]
fn ecm_is_greyed_with_no_pods() {
    let mut rules = HazardsDef::shipped().clone();
    rules.hazards[0].options[0].cost.pods = 2;
    let mut s = launched_with(rules, 500);
    s.fly_to_hazard();
    let (row, ink) = s.row_with("ECM       ");
    assert!(row.contains("(NO ECM PODS LEFT)"), "{row}");
    assert_eq!(ink, Ink::Dim);
    s.choose("ecm");
    assert_eq!(s.message(), "UNAVAILABLE: NO ECM PODS LEFT");
}

#[test]
fn a_choice_settles_the_hazard_and_logs_it() {
    let mut s = launched();
    s.fly_to_hazard();
    s.choose("pay");
    assert!(s.world.voyage.as_ref().unwrap().pending().is_none());
    assert_eq!(s.world.credits, 200);
    let text = s.text();
    assert!(text.contains("PAY: CLEAN ESCAPE. PAID 300 CR"), "{text}");
    assert!(text.contains("CREDITS 200"), "{text}");
    assert!(text.contains(HELP_TRANSIT), "{text}");
}

#[test]
fn arrival_holds_and_hands_the_base_log_a_summary() {
    let mut s = launched();
    s.fly_to_hazard();
    s.choose("dump");
    for _ in 0..10 {
        if s.world.voyage.as_ref().unwrap().status() == VoyageStatus::Holding {
            break;
        }
        s.press(&[KeyCode::Enter]);
    }
    let text = s.text();
    assert!(text.contains("HOLDING OVER SCRAP BARGE 77"), "{text}");
    assert!(text.contains("ARRIVED AFTER 5 SOLS"), "{text}");
    assert!(text.contains(HELP_HOLDING), "{text}");
    let lines = s.screen.take_base_log();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(
        lines[0].ends_with("MULLEIN HOLDING: SCRAP BARGE 77"),
        "{lines:?}"
    );
    assert!(lines[1].ends_with("CARETAKER LOG: 5 SOLS"), "{lines:?}");
    assert!(lines.iter().all(|l| l.len() <= 41), "{lines:?}");
    assert!(s.screen.take_base_log().is_empty());
    assert_eq!(s.press(&[KeyCode::Enter]), Outcome::Close);
}

#[test]
fn resolution_lines_show_the_roll_and_the_cost() {
    let r = Resolution {
        hazard: nexa_types::HazardId::new("pirate_interdiction"),
        option: "overclock".into(),
        roll: Some(nexa_sim::check::Roll {
            crew: Some(CrewId::new("jonathan")),
            skill: nexa_sim::crew::Skill::Engineering,
            bonus: 3,
            die: 4,
            score: 22,
            difficulty: 30,
            outcome: Check::Fail,
        }),
        outcome: Check::Fail,
        paid: Paid {
            fuel_l: 150,
            delay_sols: 8,
            ..Paid::default()
        },
    };
    assert_eq!(
        resolution_line(&r),
        "OVERCLOCK: JONATHAN 22 VS 30, FAILED. PAID 150 L, +8 SOLS"
    );
}

#[test]
fn catch_up_summary_counts_what_happened() {
    let events = [
        SimEvent::MachineStateChanged {
            module: nexa_types::ModuleId::new("smelter_01"),
            state: MachineState::Offline,
        },
        SimEvent::MachineStateChanged {
            module: nexa_types::ModuleId::new("hab_01"),
            state: MachineState::Freeze,
        },
        SimEvent::CrewStressChanged {
            crew: CrewId::new("elena"),
            level: nexa_sim::crew::StressLevel::Strained,
        },
    ];
    assert_eq!(
        catch_up_summary(Tick(130), 5, &events),
        [
            "S005 H00 CARETAKER LOG: 5 SOLS",
            "  1 FAILED 0 ORDERS 1 STRESS"
        ]
    );
}

#[test]
fn launch_refusals_fit_the_base_log() {
    let mut world = World::landfall();
    let e = world.launch(&mut OfflineUniverse::default()).unwrap_err();
    assert_eq!(launch_refusal(&e), "NO FLIGHT CREW");
    let line = format!("{} LAUNCH REFUSED: {}", world.now, launch_refusal(&e));
    assert!(line.len() <= 41, "{line}");
    assert_eq!(launch_refusal(&LaunchError::ShipAway), "SHIP AWAY");
    let bad_data = LaunchError::BadRules(nexa_sim::hazards::HazardError::NoHazards);
    let line = format!(
        "{} LAUNCH REFUSED: {}",
        world.now,
        launch_refusal(&bad_data)
    );
    assert_eq!(line, "S000 H00 LAUNCH REFUSED: BAD HAZARD DATA");
    assert!(line.len() <= 41);
}
