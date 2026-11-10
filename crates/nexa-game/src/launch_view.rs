//! The launch confirmation: what *The Mullein* would carry, and a warning when
//! her hold isn't full. `[L]` on the colony screen opens it; the colony is
//! held while it is open. Only `Enter` launches, through `World::launch`.

use crate::terminal::{Ink, Terminal};
use crate::ui::Outcome;
use macroquad::prelude::KeyCode;
use nexa_sim::World;

const LEFT: usize = 2;
const TOP: usize = 3;
const HELP: &str = "[ENTER] LAUNCH   [ESC] STAY ON THE PAD";

/// What a key asks for on the confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Launch,
    Stay(Outcome),
}

pub fn handle(key: KeyCode) -> Choice {
    match key {
        KeyCode::Enter | KeyCode::KpEnter => Choice::Launch,
        KeyCode::Escape | KeyCode::Backspace => Choice::Stay(Outcome::Close),
        _ => Choice::Stay(Outcome::Stay),
    }
}

/// The confirmation's lines, top to bottom.
pub fn lines(world: &World) -> Vec<(String, Ink)> {
    let ship = &world.ship;
    let def = ship.def();
    let hold = u32::from(def.hold_crates);
    let aboard = ship.cargo().len();
    let mut lines = vec![
        (
            format!(
                "LAUNCH {} FOR {}?",
                def.name.to_uppercase(),
                def.route.name.to_uppercase()
            ),
            Ink::Bright,
        ),
        (String::new(), Ink::Amber),
        (
            format!(
                "CARGO   {aboard}/{hold} CRATES  {} KG",
                ship.cargo_mass_kg()
            ),
            Ink::Amber,
        ),
    ];
    let room = ship.hold_room();
    if room > 0 {
        let crates = if room == 1 { "CRATE" } else { "CRATES" };
        lines.push((
            format!("WARNING: HOLD NOT FULL, ROOM FOR {room} MORE {crates}"),
            Ink::Bright,
        ));
    }
    lines
}

pub fn draw(t: &mut Terminal, world: &World, help_row: usize) {
    for (i, (line, ink)) in lines(world).into_iter().enumerate() {
        t.put_str(LEFT, TOP + i, &line, ink);
    }
    t.put_str(LEFT, help_row, HELP, Ink::Dim);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(world: &World) -> Vec<String> {
        lines(world).into_iter().map(|(l, _)| l).collect()
    }

    #[test]
    fn a_full_hold_shows_its_crates_and_no_warning() {
        let world = World::landfall();
        assert_eq!(
            text(&world),
            [
                "LAUNCH THE MULLEIN FOR SCRAP BARGE 77?",
                "",
                "CARGO   10/10 CRATES  10000 KG",
            ]
        );
    }

    #[test]
    fn a_hold_short_of_full_is_warned_about() {
        let mut world = World::landfall();
        world.stock_hold(7).unwrap();
        let lines = lines(&world);
        assert_eq!(lines[2].0, "CARGO   7/10 CRATES  7000 KG");
        assert_eq!(
            lines[3],
            (
                "WARNING: HOLD NOT FULL, ROOM FOR 3 MORE CRATES".to_string(),
                Ink::Bright
            )
        );
        world.stock_hold(9).unwrap();
        assert_eq!(
            text(&world)[3],
            "WARNING: HOLD NOT FULL, ROOM FOR 1 MORE CRATE"
        );
        world.stock_hold(0).unwrap();
        assert_eq!(text(&world)[2], "CARGO   0/10 CRATES  0 KG");
    }

    #[test]
    fn only_enter_launches() {
        assert_eq!(handle(KeyCode::Enter), Choice::Launch);
        assert_eq!(handle(KeyCode::Escape), Choice::Stay(Outcome::Close));
        assert_eq!(handle(KeyCode::L), Choice::Stay(Outcome::Stay));
    }
}
