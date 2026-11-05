//! The voyage: the cockpit readout and transit log, the hazard terminal when
//! something waits on the route, and the arrival over Scrap Barge 77.
//!
//! Every change goes through the sim (`advance_voyage`, `resolve_hazard`); a
//! refused choice keeps the terminal open and shows the sim's reason.

use crate::crew_view::skill_label;
use crate::terminal::{Ink, Terminal};
use crate::ui::{wrap, ListPicker, Outcome, Pick};
use macroquad::prelude::KeyCode;
use nexa_sim::check::Outcome as Check;
use nexa_sim::flight::{affordable, LaunchError, Paid, Resolution, Voyage, VoyageStatus};
use nexa_sim::hazards::{HazardOption, Toll};
use nexa_sim::machines::MachineState;
use nexa_sim::{SimEvent, World};
use nexa_types::Tick;

const LEFT: usize = 2;
const TOP: usize = 3;
/// Inside the frame, with a column of margin on each side.
const WIDTH: usize = 76;
const BAR_WIDTH: usize = 20;
const LABEL_WIDTH: usize = 10;
/// Lines kept in the transit log; more than the screen shows.
const LOG_CAPACITY: usize = 40;
const HELP_TRANSIT: &str = "[ENTER] FLY ONE SOL";
const HELP_HAZARD: &str = "[UP/DOWN] SELECT   [ENTER] COMMIT";
const HELP_HOLDING: &str = "[ENTER] BACK TO THE COLONY";

/// Short enough for the base log panel after a time stamp.
pub fn launch_refusal(e: &LaunchError) -> &'static str {
    match e {
        LaunchError::ShipAway => "SHIP AWAY",
        LaunchError::NoFlightCrew => "NO FLIGHT CREW",
        LaunchError::NotEnoughFuel { .. } => "LOW FUEL",
        LaunchError::BadRules(_) | LaunchError::BadBriefing(_) => "BAD BRIEFING",
    }
}

/// One line per sim event worth showing in the transit log.
pub fn transit_line(event: &SimEvent, voyage: Option<&Voyage>) -> Option<String> {
    Some(match event {
        SimEvent::Launched { flight, .. } => {
            format!("LAUNCH: {} ABOARD, BURNED THE LEG", flight.len())
        }
        SimEvent::SlipSolPassed { sol, of } => format!("SOL {sol} OF {of} IN THE SLIP"),
        SimEvent::HazardEncountered { hazard } => {
            let name = voyage
                .and_then(|v| v.rules().hazard(hazard))
                .map_or_else(|| hazard.to_string(), |h| h.name.clone());
            format!("ALERT: {}", name.to_uppercase())
        }
        SimEvent::HazardResolved(r) => resolution_line(r),
        SimEvent::Arrived { sols, .. } => format!("ARRIVED AFTER {sols} SOLS"),
        _ => return None,
    })
}

fn outcome_label(outcome: Check) -> &'static str {
    match outcome {
        Check::Success => "CLEAN ESCAPE",
        Check::Partial => "ESCAPED, AT A COST",
        Check::Fail => "FAILED",
    }
}

/// "ECM: CAL 31 VS 30, CLEAN ESCAPE. -1 POD"
pub fn resolution_line(r: &Resolution) -> String {
    let roll = r.roll.as_ref().map_or(String::new(), |roll| {
        let who = roll
            .crew
            .as_ref()
            .map_or("NOBODY".to_string(), |c| c.as_str().to_uppercase());
        format!("{who} {} VS {}, ", roll.score, roll.difficulty)
    });
    let paid = paid_text(&r.paid);
    let sep = if paid.is_empty() { "" } else { ". " };
    format!(
        "{}: {roll}{}{sep}{paid}",
        r.option.to_uppercase(),
        outcome_label(r.outcome)
    )
}

/// What a settled hazard cost, e.g. "-150 L -1 POD +6 SOLS".
pub fn paid_text(p: &Paid) -> String {
    let mut parts = Vec::new();
    if p.fuel_l > 0 {
        parts.push(format!("-{} L", p.fuel_l));
    }
    if p.pods > 0 {
        parts.push(format!("-{} POD", p.pods));
    }
    if p.credits > 0 {
        parts.push(format!("-{} CR", p.credits));
    }
    if p.crates_dumped > 0 {
        parts.push(format!("-{} CRATES", p.crates_dumped));
    }
    if p.cargo_damage_pct > 0 {
        parts.push(format!("CARGO -{}%", p.cargo_damage_pct));
    }
    if p.delay_sols > 0 {
        parts.push(format!("+{} SOLS", p.delay_sols));
    }
    parts.join(" ")
}

/// The up-front cost, as the terminal lists it.
fn cost_text(cost: &Toll, overclock_l: u32) -> String {
    let mut parts = Vec::new();
    if cost.overclock_burns > 0 {
        parts.push(format!(
            "{} L",
            u32::from(cost.overclock_burns) * overclock_l
        ));
    }
    if cost.pods > 0 {
        parts.push(format!("{} POD", cost.pods));
    }
    if cost.credits > 0 {
        parts.push(format!("{} CR", cost.credits));
    }
    if cost.dump_crates > 0 {
        parts.push(format!("{} CRATES", cost.dump_crates));
    }
    parts.join(" ")
}

/// "ECM   PILOTING CHECK   1 POD", or with why it can't be taken.
fn option_line(world: &World, option: &HazardOption) -> (String, bool) {
    let check = option.check.as_ref().map_or("CERTAIN".to_string(), |c| {
        format!("{} CHECK", skill_label(c.skill))
    });
    let cost = cost_text(&option.cost, world.ship.def().overclock_burn_l);
    let line = format!("{:<10}{:<20}{cost}", option.id.to_uppercase(), check);
    match affordable(&world.ship, world.credits, &option.cost) {
        Ok(()) => (line, true),
        Err(why) => (
            format!("{line}  ({})", why.to_string().to_uppercase()),
            false,
        ),
    }
}

/// Two lines for the base log about the hours the colony ran unwatched.
pub fn catch_up_summary(at: Tick, sols: u32, events: &[SimEvent]) -> Vec<String> {
    let failures = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                SimEvent::MachineStateChanged {
                    state: MachineState::Burnout | MachineState::Offline,
                    ..
                }
            )
        })
        .count();
    let fired = events
        .iter()
        .filter(|e| matches!(e, SimEvent::OrderFired { .. }))
        .count();
    let stress = events
        .iter()
        .filter(|e| matches!(e, SimEvent::CrewStressChanged { .. }))
        .count();
    vec![
        format!("{at} CARETAKER LOG: {sols} SOLS"),
        format!("  {failures} FAILED {fired} ORDERS {stress} STRESS"),
    ]
}

pub struct VoyageScreen {
    options: ListPicker,
    log: Vec<String>,
    /// The last refusal from the sim.
    message: Option<String>,
    /// Lines for the base log once the player is back at the colony.
    base_log: Vec<String>,
}

impl VoyageScreen {
    /// Opened right after launch, with the launch's events.
    pub fn new(world: &World, launch: &[SimEvent]) -> Self {
        let mut screen = Self {
            options: ListPicker::default(),
            log: Vec::new(),
            message: None,
            base_log: Vec::new(),
        };
        screen.record(world, launch);
        screen
    }

    fn record(&mut self, world: &World, events: &[SimEvent]) {
        let voyage = world.voyage.as_ref();
        for line in events.iter().filter_map(|e| transit_line(e, voyage)) {
            if self.log.len() == LOG_CAPACITY {
                self.log.remove(0);
            }
            self.log.push(line);
        }
    }

    /// Lines for the base log, handed over once.
    pub fn take_base_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.base_log)
    }

    pub fn handle(&mut self, key: KeyCode, world: &mut World) -> Outcome {
        let Some(voyage) = world.voyage.as_ref() else {
            return Outcome::Close;
        };
        if voyage.status() == VoyageStatus::Holding {
            return match key {
                KeyCode::Enter | KeyCode::KpEnter | KeyCode::Escape => Outcome::Close,
                _ => Outcome::Stay,
            };
        }
        if let Some(hazard) = voyage.pending() {
            let ids: Vec<String> = hazard.options.iter().map(|o| o.id.clone()).collect();
            if let Pick::Chosen(i) = self.options.handle(key, ids.len()) {
                match world.resolve_hazard(&ids[i]) {
                    Ok(events) => {
                        self.message = None;
                        self.options = ListPicker::default();
                        self.record(world, &events);
                    }
                    Err(e) => self.message = Some(e.to_string().to_uppercase()),
                }
            }
            return Outcome::Stay;
        }
        if matches!(key, KeyCode::Enter | KeyCode::KpEnter | KeyCode::Space) {
            match world.advance_voyage() {
                Ok(step) => {
                    self.message = None;
                    self.record(world, &step.events);
                    if let Some(sols) = step.events.iter().find_map(|e| match e {
                        SimEvent::Arrived { sols, .. } => Some(*sols),
                        _ => None,
                    }) {
                        let name = world
                            .voyage
                            .as_ref()
                            .map_or(String::new(), |v| v.destination_name().to_uppercase());
                        let at = world.now;
                        self.base_log.push(format!("{at} MULLEIN HOLDING: {name}"));
                        self.base_log
                            .extend(catch_up_summary(world.now, sols, &step.colony));
                    }
                }
                Err(e) => self.message = Some(e.to_string().to_uppercase()),
            }
        }
        Outcome::Stay
    }

    /// Draw everything between the title row and `help_row`, which gets the key help.
    pub fn draw(&self, t: &mut Terminal, world: &World, help_row: usize) {
        let Some(voyage) = world.voyage.as_ref() else {
            return;
        };
        let dest = voyage.destination_name().to_uppercase();
        let holding = voyage.status() == VoyageStatus::Holding;
        let head = if holding {
            format!("HOLDING OVER {dest}")
        } else {
            format!("THE SLIP // OUTBOUND TO {dest}")
        };
        t.put_str(LEFT, TOP, &head, Ink::Bright);
        let sols = format!("SOL {} OF {}", voyage.sols_flown(), voyage.sols_total());
        t.put_str(LEFT + WIDTH - sols.len(), TOP, &sols, Ink::Amber);
        let mut row = draw_gauges(t, world, voyage, TOP + 2);

        let help = if holding {
            row += 1;
            t.put_str(
                LEFT,
                row,
                &format!("THE MULLEIN HOLDS OVER {dest}. DOCKING COMES LATER."),
                Ink::Bright,
            );
            row += 1;
            let delay = voyage.delay_sols();
            let line = format!(
                "VOYAGE: {} SOLS, {delay} OF THEM DELAY",
                voyage.sols_total()
            );
            t.put_str(LEFT, row, &line, Ink::Amber);
            row += 1;
            t.put_str(
                LEFT,
                row,
                "THE COLONY HAS CAUGHT UP ON THE SOLS AWAY.",
                Ink::Amber,
            );
            row += 2;
            HELP_HOLDING
        } else if let Some(hazard) = voyage.pending() {
            row += 1;
            t.put_str(LEFT, row, "HAZARD TERMINAL", Ink::Dim);
            t.put_str(LEFT + 17, row, &hazard.name.to_uppercase(), Ink::Bright);
            row += 1;
            for line in wrap(&hazard.text, WIDTH) {
                t.put_str(LEFT, row, &line, Ink::Amber);
                row += 1;
            }
            row += 1;
            for (i, option) in hazard.options.iter().enumerate() {
                let (line, available) = option_line(world, option);
                let selected = i == self.options.cursor();
                let mark = if selected { "> " } else { "  " };
                let ink = match (available, selected) {
                    (false, _) => Ink::Dim,
                    (true, true) => Ink::Bright,
                    (true, false) => Ink::Amber,
                };
                t.put_str(LEFT, row, &format!("{mark}{line}"), ink);
                row += 1;
            }
            if let Some(option) = hazard.options.get(self.options.cursor()) {
                t.put_str(LEFT + 2, row, &option.label.to_uppercase(), Ink::Dim);
            }
            row += 2;
            HELP_HAZARD
        } else {
            row += 1;
            HELP_TRANSIT
        };

        // The transit log fills what's left above the message row.
        let bottom = help_row.saturating_sub(2);
        if row < bottom {
            t.put_str(LEFT, row, "TRANSIT LOG", Ink::Dim);
            let room = bottom - row - 1;
            let start = self.log.len().saturating_sub(room);
            for (i, line) in self.log[start..].iter().enumerate() {
                let clipped: String = line.chars().take(WIDTH).collect();
                t.put_str(LEFT, row + 1 + i, &clipped, Ink::Amber);
            }
        }
        if let Some(message) = &self.message {
            t.put_str(LEFT, help_row - 2, message, Ink::Bright);
        }
        t.put_str(LEFT, help_row, help, Ink::Dim);
    }
}

/// Fuel, cargo, pods, credits, delay and who's aboard; returns the next free row.
fn draw_gauges(t: &mut Terminal, world: &World, voyage: &Voyage, top: usize) -> usize {
    let ship = &world.ship;
    let tank = ship.def().tank_l.max(1);
    let filled = (ship.fuel_l() as usize * BAR_WIDTH / tank as usize).min(BAR_WIDTH);
    let fuel = format!(
        "[{}{}] {}/{} L",
        "#".repeat(filled),
        "-".repeat(BAR_WIDTH - filled),
        ship.fuel_l(),
        ship.def().tank_l
    );
    let cargo = format!(
        "{} CRATES  {} KG  INTEGRITY {}%",
        ship.cargo().len(),
        ship.cargo_mass_kg(),
        ship.cargo_integrity_pct()
    );
    let stores = format!("ECM PODS {}   CREDITS {}", ship.pods(), world.credits);
    let delay = format!(
        "+{} SOLS (RANDOM {}/{})",
        voyage.delay_sols(),
        voyage.random_delay_sols(),
        voyage.rules().delays.random_cap_sols
    );
    let crew: Vec<String> = voyage
        .flight_crew()
        .iter()
        .map(|c| c.as_str().to_uppercase())
        .collect();
    let rows = [
        ("FUEL", fuel),
        ("CARGO", cargo),
        ("STORES", stores),
        ("DELAY", delay),
        ("ABOARD", crew.join(", ")),
    ];
    for (i, (label, value)) in rows.iter().enumerate() {
        t.put_str(LEFT, top + i, label, Ink::Dim);
        t.put_str(LEFT + LABEL_WIDTH, top + i, value, Ink::Amber);
    }
    top + rows.len()
}

#[cfg(test)]
mod tests;
