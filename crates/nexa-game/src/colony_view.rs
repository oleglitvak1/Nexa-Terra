//! The colony screen: the Ash Plain map, module status and an event log.

use crate::terminal::{Ink, Terminal};
use nexa_sim::colony::{Colony, Module};
use nexa_sim::grid::Tile;
use nexa_sim::orders::format_centi_c;
use nexa_sim::SimEvent;
use nexa_types::Tick;

pub const MAP_COL: usize = 2;
pub const MAP_ROW: usize = 2;
pub const PANEL_COL: usize = 36;
const POWER_BAR_WIDTH: usize = 20;

/// "+14.0C": the orders terminal's rounding, with an explicit sign so the column lines up.
pub fn fmt_temp(centi_c: i32) -> String {
    let text = format_centi_c(centi_c);
    if text.starts_with('-') {
        text
    } else {
        format!("+{text}")
    }
}

/// The POWER column: battery charge, generator output, or a consumer's supply.
pub fn power_label(m: &Module) -> String {
    let def = m.def();
    if !m.active() && def.uses_power() {
        "OFF".into()
    } else if let Some(b) = def.battery {
        format!("{}/{}", m.charge_kwh(), b.capacity_kwh)
    } else if m.failed() {
        "--".into()
    } else if def.output_kw > 0 {
        format!("+{}kW", def.output_kw)
    } else if def.draw_kw > 0 {
        if m.powered() { "ON" } else { "DARK" }.into()
    } else {
        "--".into()
    }
}

pub fn log_line(at: Tick, event: &SimEvent) -> String {
    // Cards show 1-based, as the terminal numbers its slots.
    match event {
        SimEvent::SolStarted { sol } => format!("SOL {sol:03} BEGINS"),
        SimEvent::MachineStateChanged { module, state } => format!(
            "{at} {} {}",
            module.as_str().to_uppercase(),
            state.label().to_uppercase()
        ),
        SimEvent::OrderFired { slot, .. } => format!("{at} ORDER {} FIRED", slot + 1),
        // The module, not the whole action: the full text overflows the log panel.
        SimEvent::OrderActionSkipped {
            slot,
            action,
            reason,
        } => format!(
            "{at} ORDER {} {} {}",
            slot + 1,
            reason.label().to_uppercase(),
            action.module().as_str().to_uppercase()
        ),
    }
}

/// Draw the map with its top-left at (MAP_COL, MAP_ROW). Failed modules blink.
pub fn draw_map(t: &mut Terminal, colony: &Colony, blink: bool) {
    let grid = colony.grid();
    for row in 0..grid.height() {
        for col in 0..grid.width() {
            let Some(tile) = grid.tile(col, row) else {
                continue;
            };
            let (ch, ink) = match tile {
                Tile::Ground => ('.', Ink::Dim),
                Tile::Fence => ('#', Ink::Amber),
                Tile::Pad => ('_', Ink::Amber),
                Tile::PowerConduit => ('=', Ink::Dim),
                Tile::HeatLine => ('~', Ink::Dim),
                Tile::Trunk => ('+', Ink::Dim),
                Tile::Module(i) => {
                    let m = &colony.modules()[i];
                    let glyph = m.def().glyph;
                    if m.failed() {
                        (if blink { glyph } else { ' ' }, Ink::Bright)
                    } else if m.running() {
                        (glyph, Ink::Bright)
                    } else {
                        (glyph, Ink::Dim)
                    }
                }
            };
            t.put_char(MAP_COL + col, MAP_ROW + row, ch, ink);
        }
    }
}

/// Grid power bar and one status line per module; returns the next free row.
pub fn draw_status(t: &mut Terminal, colony: &Colony, top: usize) -> usize {
    let pct = colony.power_percent() as usize;
    let filled = (pct * POWER_BAR_WIDTH / 100).min(POWER_BAR_WIDTH);
    let bar = format!(
        "GRID POWER [{}{}] {pct:>3}%",
        "#".repeat(filled),
        "-".repeat(POWER_BAR_WIDTH - filled)
    );
    let bar_ink = if pct < 25 { Ink::Bright } else { Ink::Amber };
    t.put_str(PANEL_COL, top, &bar, bar_ink);

    let header = format!(
        "{:<12}{:<9}{:>7} {:>4}  {}",
        "MODULE", "STATE", "TEMP", "WEAR", "POWER"
    );
    t.put_str(PANEL_COL, top + 2, &header, Ink::Dim);
    let mut row = top + 3;
    for m in colony.modules() {
        let (state, temp, wear) = match m.machine() {
            Some(machine) => (
                machine.state().label().to_uppercase(),
                fmt_temp(m.temp_centi_c()),
                format!("{}%", machine.wear_bp() / 100),
            ),
            None => ("--".into(), String::new(), "--".into()),
        };
        let line = format!(
            "{:<12}{:<9}{:>7} {:>4}  {}",
            m.id().as_str(),
            state,
            temp,
            wear,
            power_label(m)
        );
        let ink = if m.failed() {
            Ink::Bright
        } else if m.active() {
            Ink::Amber
        } else {
            Ink::Dim
        };
        t.put_str(PANEL_COL, row, &line, ink);
        row += 1;
    }
    row
}

/// The most recent log lines that fit between `top` and `bottom` (exclusive), oldest first.
pub fn draw_log<'a>(
    t: &mut Terminal,
    lines: impl DoubleEndedIterator<Item = &'a String>,
    top: usize,
    bottom: usize,
) {
    t.put_str(PANEL_COL, top, "BASE LOG", Ink::Dim);
    let room = bottom.saturating_sub(top + 1);
    let recent: Vec<&String> = lines.rev().take(room).collect();
    for (i, line) in recent.into_iter().rev().enumerate() {
        t.put_str(PANEL_COL, top + 1 + i, line, Ink::Amber);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexa_sim::machines::MachineState;
    use nexa_sim::World;
    use nexa_types::{Command, ModuleId};

    #[test]
    fn temps_format_with_one_decimal_and_a_sign() {
        assert_eq!(fmt_temp(1400), "+14.0C");
        assert_eq!(fmt_temp(-4000), "-40.0C");
        assert_eq!(fmt_temp(0), "+0.0C");
    }

    // The orders terminal shows the same readings; both screens must agree.
    #[test]
    fn temps_round_like_the_orders_terminal() {
        assert_eq!(fmt_temp(1455), "+14.6C");
        assert_eq!(fmt_temp(-405), "-4.1C");
        assert_eq!(fmt_temp(-4), "+0.0C");
    }

    #[test]
    fn map_draws_the_layout_at_its_offset() {
        let world = World::landfall();
        let mut t = Terminal::new(80, 36);
        draw_map(&mut t, &world.colony, true);
        assert_eq!(t.get(MAP_COL, MAP_ROW).unwrap().ch, '#');
        // hab_01's top-left tile in the Ash Plain layout.
        assert_eq!(t.get(MAP_COL + 3, MAP_ROW + 7).unwrap().ch, 'H');
    }

    #[test]
    fn switched_off_modules_draw_dim() {
        let mut world = World::landfall();
        world
            .apply(&Command::Deactivate(ModuleId::new("heater_01")))
            .unwrap();
        world.tick();
        let mut t = Terminal::new(80, 36);
        draw_map(&mut t, &world.colony, true);
        let heater = t.get(MAP_COL + 15, MAP_ROW + 7).unwrap();
        assert_eq!((heater.ch, heater.ink), ('R', Ink::Dim));
    }

    #[test]
    fn power_labels_describe_each_kind_of_module() {
        let mut world = World::landfall();
        world.tick();
        let label = |id: &str| power_label(world.colony.module(&ModuleId::new(id)).unwrap());
        assert_eq!(label("gen_01"), "+30kW");
        assert_eq!(label("bat_01"), "296/400");
        assert_eq!(label("hab_01"), "ON");
        assert_eq!(label("heater_02"), "OFF");
    }

    #[test]
    fn log_lines_name_the_hour_module_and_state() {
        let event = SimEvent::MachineStateChanged {
            module: ModuleId::new("hab_02"),
            state: MachineState::Freeze,
        };
        assert_eq!(log_line(Tick(27), &event), "S001 H01 HAB_02 FREEZE");
    }

    #[test]
    fn log_lines_number_order_cards_from_one() {
        let order = nexa_sim::orders::StandingOrder {
            triggers: vec![nexa_sim::orders::Trigger::PowerBelowPercent(25)],
            actions: vec![nexa_sim::orders::Action::Deactivate(ModuleId::new(
                "smelter_01",
            ))],
            priority: 1,
        };
        let fired = SimEvent::OrderFired { slot: 0, order };
        assert_eq!(log_line(Tick(51), &fired), "S001 H25 ORDER 1 FIRED");
        let skipped = SimEvent::OrderActionSkipped {
            slot: 1,
            action: nexa_sim::orders::Action::DivertHeat {
                to: ModuleId::new("smelter_01"),
            },
            reason: nexa_sim::orders::SkipReason::Overridden,
        };
        assert_eq!(
            log_line(Tick(51), &skipped),
            "S001 H25 ORDER 2 OVERRIDDEN SMELTER_01"
        );
    }

    #[test]
    fn log_keeps_the_newest_lines_that_fit() {
        let lines: Vec<String> = (0..5).map(|i| format!("line {i}")).collect();
        let mut t = Terminal::new(80, 10);
        draw_log(&mut t, lines.iter(), 0, 3); // header + 2 lines
        assert_eq!(t.get(PANEL_COL + 5, 1).unwrap().ch, '3');
        assert_eq!(t.get(PANEL_COL + 5, 2).unwrap().ch, '4');
    }
}
