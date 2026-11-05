//! The crew screen driven key by key, with the frame checked after every key.

use super::*;
use nexa_sim::crew::StressLevel;
use nexa_types::TICKS_PER_SOL;

const COLS: usize = 80;
const ROWS: usize = 36;
const HELP_ROW: usize = ROWS - 2;

struct Session {
    screen: CrewScreen,
    world: World,
    events: Vec<SimEvent>,
}

impl Session {
    fn new(world: World) -> Self {
        Self {
            screen: CrewScreen::new(&world),
            world,
            events: Vec::new(),
        }
    }

    fn press(&mut self, keys: &[KeyCode]) -> Outcome {
        let mut outcome = Outcome::Stay;
        for &key in keys {
            outcome = self.screen.handle(key, &mut self.world, &mut self.events);
            self.check_frame(key);
        }
        outcome
    }

    /// Move the cursor to `who` and flip their side.
    fn toggle(&mut self, who: &str) {
        let i = self
            .world
            .crew
            .members()
            .iter()
            .position(|c| c.id.as_str() == who)
            .unwrap();
        while self.screen.list.cursor() != i {
            self.press(&[KeyCode::Down]);
        }
        self.press(&[KeyCode::Space]);
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

    fn row_with(&self, needle: &str) -> String {
        let t = self.terminal();
        (0..ROWS)
            .map(|r| row_text(&t, r))
            .find(|r| r.contains(needle))
            .unwrap_or_else(|| panic!("no row with {needle}:\n{}", self.text()))
    }

    fn message(&self) -> String {
        row_text(&self.terminal(), HELP_ROW - 2)
            .trim()
            .trim_matches('|')
            .trim()
            .to_string()
    }

    fn flying(&self) -> Vec<&str> {
        let crew = &self.world.crew;
        crew.members()
            .iter()
            .filter(|c| crew.assignment(&c.id) == Some(Assignment::Flight))
            .map(|c| c.id.as_str())
            .collect()
    }

    /// Nothing the screen draws may cover the frame or the column of margin
    /// inside it.
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
fn lists_the_crew_all_on_the_ground_and_calm() {
    let s = Session::new(World::landfall());
    for name in [
        "DR. JONATHAN WINTER",
        "DR. ELENA WINTER",
        "MAYA WINTER",
        "CAPT. CAL CALLAHAN",
        "ORON-3",
        "DAX THORNE",
    ] {
        let row = s.row_with(name);
        assert!(row.contains("GROUND"), "{row}");
        assert!(row.contains("0% CALM"), "{row}");
        assert!(row.contains("100%"), "{row}");
    }
    assert!(s.text().contains("0 FLY  6 STAY"));
}

#[test]
fn shows_the_selected_members_skills_trait_and_bunk() {
    let mut s = Session::new(World::landfall());
    s.press(&[KeyCode::Down; 3]);
    assert!(s.text().contains("CAPT. CAL CALLAHAN  ADULT  BUNK HAB_02"));
    assert!(s.text().contains("PILOTING 5  SENSORS 2  RIGGING 3"));
    assert!(s.text().contains("COMBAT FATIGUE"));
    s.press(&[KeyCode::Up]);
    assert!(s.text().contains("MAYA WINTER  CHILD  BUNK HAB_01"));
}

#[test]
fn confirming_a_split_records_it_and_logs_it() {
    let mut s = Session::new(World::landfall());
    s.toggle("cal");
    s.toggle("oron");
    assert!(s.row_with("CAL CALLAHAN").contains("FLIGHT*"));
    assert_eq!(s.flying(), Vec::<&str>::new(), "not confirmed yet");
    s.press(&[KeyCode::Enter]);
    assert_eq!(s.flying(), ["cal", "oron"]);
    assert_eq!(
        s.events,
        [SimEvent::SplitConfirmed {
            flight: vec![CrewId::new("cal"), CrewId::new("oron")],
        }]
    );
    assert_eq!(s.message(), "SPLIT CONFIRMED: 2 FLY, 4 STAY");
    assert!(!s.row_with("CAL CALLAHAN").contains('*'));
    assert!(s.text().contains("2 FLY  4 STAY"));
}

#[test]
fn a_refused_split_shows_why_and_changes_nothing() {
    let mut s = Session::new(World::landfall());
    s.toggle("dax");
    s.press(&[KeyCode::Enter]);
    assert_eq!(s.message(), "NO QUALIFIED PILOT IS FLYING");
    assert_eq!(s.flying(), Vec::<&str>::new());
    assert!(s.events.is_empty());
    // The draft is kept, so the player can fix it.
    assert!(s.row_with("DAX THORNE").contains("FLIGHT*"));

    for who in ["cal", "jonathan", "elena", "oron"] {
        s.toggle(who);
    }
    s.press(&[KeyCode::Enter]);
    assert_eq!(s.message(), "MAYA HAS NO ADULT ON THEIR SIDE");
    assert_eq!(s.flying(), Vec::<&str>::new());
}

#[test]
fn closing_drops_an_unconfirmed_draft() {
    let mut world = World::landfall();
    let mut s = Session::new(world.clone());
    s.toggle("cal");
    s.press(&[KeyCode::Enter]);
    s.toggle("dax");
    assert_eq!(s.press(&[KeyCode::Escape]), Outcome::Close);
    world = s.world;
    let reopened = Session::new(world);
    assert!(reopened.row_with("CAL CALLAHAN").contains("FLIGHT "));
    assert!(reopened.row_with("DAX THORNE").contains("GROUND "));
}

#[test]
fn shows_stress_and_morale_as_they_change() {
    let mut world = World::landfall();
    for _ in 0..6 * TICKS_PER_SOL {
        world.tick();
    }
    let jonathan = world.crew.vitals(&CrewId::new("jonathan")).unwrap();
    assert_eq!(jonathan.level, StressLevel::Breaking);
    let s = Session::new(world.clone());
    let row = s.row_with("JONATHAN");
    let stress = format!("{}% BREAKING", jonathan.stress_bp / 100);
    let morale = format!("{}%", jonathan.morale_bp / 100);
    assert!(row.contains(&stress), "{row}");
    assert!(row.trim_end_matches([' ', '|']).ends_with(&morale), "{row}");
}
