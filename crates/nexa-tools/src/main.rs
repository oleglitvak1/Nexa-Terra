//! Headless developer tools for Nexa Terra.
//!
//! Runs the landing site forward without a window and no player input,
//! optionally under a preset set of Standing Orders and a confirmed Split,
//! logging machine state changes, order firings and crew stress levels, then
//! prints where the colony and its crew ended up.
//!
//! Usage: `nexa-tools [SOLS] [--orders PRESET] [--split ID,ID,...]`
//! (presets: content/standing_orders.ron; IDs: content/crew.ron).

use nexa_sim::crew::{Assignment, StressLevel};
use nexa_sim::orders::OrdersDef;
use nexa_sim::{SimEvent, World};
use nexa_types::{CrewId, TICKS_PER_SOL};
use std::collections::BTreeMap;

/// The vertical slice's length: the scrubbers' filter life.
const DEFAULT_SOLS: u64 = 45;

/// Per crew member, in roster order: the worst stress seen and hours spent at
/// each level above Calm, for tuning the stress rates.
#[derive(Debug, Default, PartialEq, Eq)]
struct CrewTally {
    peak_bp: Vec<u16>,
    strained_hours: Vec<u64>,
    breaking_hours: Vec<u64>,
}

impl CrewTally {
    fn new(world: &World) -> Self {
        let n = world.crew.members().len();
        Self {
            peak_bp: vec![0; n],
            strained_hours: vec![0; n],
            breaking_hours: vec![0; n],
        }
    }

    /// Call once after each tick.
    fn record(&mut self, world: &World) {
        for (i, c) in world.crew.members().iter().enumerate() {
            let Some(v) = world.crew.vitals(&c.id) else {
                continue;
            };
            self.peak_bp[i] = self.peak_bp[i].max(v.stress_bp);
            match v.level {
                StressLevel::Calm => {}
                StressLevel::Strained => self.strained_hours[i] += 1,
                StressLevel::Breaking => self.breaking_hours[i] += 1,
            }
        }
    }
}

/// `cal,oron` -> both on Flight; everyone else stays on the Ground.
fn parse_split(list: &str) -> BTreeMap<CrewId, Assignment> {
    list.split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| (CrewId::new(id), Assignment::Flight))
        .collect()
}

fn main() {
    let mut sols = DEFAULT_SOLS;
    let mut preset: Option<String> = None;
    let mut split: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--orders" => match args.next() {
                Some(name) => preset = Some(name),
                None => usage("--orders needs a preset name"),
            },
            "--split" => match args.next() {
                Some(list) => split = Some(list),
                None => usage("--split needs crew IDs, e.g. cal,oron"),
            },
            _ => match arg.parse() {
                Ok(n) => sols = n,
                Err(_) => usage(&format!("unexpected argument {arg:?}")),
            },
        }
    }

    let mut world = World::landfall();
    if let Some(name) = &preset {
        let def = OrdersDef::shipped();
        let Some(cards) = def.presets.get(name) else {
            usage(&format!("no preset named {name:?}"));
        };
        for order in cards {
            if let Err(e) = world.add_order(order.clone()) {
                usage(&format!("preset {name:?}: {e}"));
            }
        }
        println!("[SIM] standing orders: {name}");
        for (slot, order) in world.orders.iter().enumerate() {
            println!(
                "[SIM]   {} P{} {}",
                slot + 1,
                order.priority,
                order.render()
            );
        }
    }

    if let Some(list) = &split {
        if let Err(e) = world.confirm_split(&parse_split(list)) {
            usage(&format!("split {list:?}: {e}"));
        }
        println!("[SIM] split confirmed, flying: {list}");
    }

    let mut fires = vec![0u64; world.orders.len()];
    let mut tally = CrewTally::new(&world);
    for _ in 0..sols * TICKS_PER_SOL {
        let now = world.now;
        for event in world.tick() {
            match event {
                SimEvent::SolStarted { sol } => println!("[SIM] SOL {sol:03} BEGINS"),
                SimEvent::MachineStateChanged { module, state } => {
                    println!("[SIM] {now}  {module} -> {}", state.label())
                }
                // Swinging heat between habs fires every couple of hours: list a
                // card's first firing, count the rest.
                SimEvent::OrderFired { slot, order } => {
                    if fires[slot] == 0 {
                        println!("[SIM] {now}  order {} fires: {}", slot + 1, order.render());
                    }
                    fires[slot] += 1;
                }
                SimEvent::OrderActionSkipped {
                    slot,
                    action,
                    reason,
                } => println!(
                    "[SIM] {now}  order {} {}: {}",
                    slot + 1,
                    reason.label(),
                    action.render()
                ),
                SimEvent::SplitConfirmed { flight } => {
                    let names: Vec<_> = flight.iter().map(|c| c.as_str()).collect();
                    println!("[SIM] {now}  split confirmed, flying: {}", names.join(", "))
                }
                SimEvent::CrewStressChanged { crew, level } => {
                    println!("[SIM] {now}  {crew} is {}", level.label())
                }
            }
        }
        tally.record(&world);
    }

    println!(
        "[SIM] ran {sols} Sols headless, now at tick {}",
        world.now.0
    );
    for (slot, count) in fires.iter().enumerate() {
        println!("[SIM] order {} fired {count} times", slot + 1);
    }
    println!("[SIM] grid power {}%", world.colony.power_percent());
    for m in world.colony.modules() {
        if let Some(machine) = m.machine() {
            println!(
                "[SIM]   {:<12} {:<9} {:>6} centi-C  wear {:>3}%",
                m.id().as_str(),
                machine.state().label(),
                m.temp_centi_c(),
                machine.wear_percent()
            );
        }
    }
    // Same widths as the rows below: "{:>5}%" is six wide, "{:>6} h" eight.
    println!(
        "[SIM]   {:<9} {:<7} {:>6} {:<9} {:>6} {:>5}  {:>8}  {:>8}",
        "crew", "side", "stress", "level", "morale", "peak", "strained", "breaking"
    );
    for (i, c) in world.crew.members().iter().enumerate() {
        let side = match world.crew.assignment(&c.id) {
            Some(Assignment::Flight) => "flight",
            _ => "ground",
        };
        let Some(v) = world.crew.vitals(&c.id) else {
            continue;
        };
        println!(
            "[SIM]   {:<9} {side:<7} {:>5}% {:<9} {:>5}% {:>4}%  {:>6} h  {:>6} h",
            c.id.as_str(),
            v.stress_bp / 100,
            v.level.label(),
            v.morale_bp / 100,
            tally.peak_bp[i] / 100,
            tally.strained_hours[i],
            tally.breaking_hours[i],
        );
    }
}

fn usage(problem: &str) -> ! {
    let presets: Vec<&str> = OrdersDef::shipped()
        .presets
        .keys()
        .map(String::as_str)
        .collect();
    eprintln!("nexa-tools: {problem}");
    eprintln!("usage: nexa-tools [SOLS] [--orders PRESET] [--split ID,ID,...]");
    eprintln!("presets: {}", presets.join(", "));
    let crew: Vec<String> = World::landfall()
        .crew
        .members()
        .iter()
        .map(|c| c.id.to_string())
        .collect();
    eprintln!("crew: {}", crew.join(", "));
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_lists_who_flies() {
        let split = parse_split("cal, oron,");
        let flying: Vec<_> = split.keys().map(CrewId::as_str).collect();
        assert_eq!(flying, ["cal", "oron"]);
        assert!(split.values().all(|&a| a == Assignment::Flight));
    }

    #[test]
    fn tally_counts_hours_at_each_level_and_the_peak() {
        let mut world = World::landfall();
        let mut tally = CrewTally::new(&world);
        for _ in 0..8 * TICKS_PER_SOL {
            world.tick();
            tally.record(&world);
        }
        let jonathan = 0;
        let v = world.crew.vitals(&CrewId::new("jonathan")).unwrap();
        assert_eq!(v.level, StressLevel::Breaking);
        assert!(tally.peak_bp[jonathan] >= v.stress_bp);
        assert!(tally.strained_hours[jonathan] > 0);
        assert!(tally.breaking_hours[jonathan] > 0);
        // Hours counted never exceed the hours run.
        let hours = 8 * TICKS_PER_SOL;
        assert!(tally.strained_hours[jonathan] + tally.breaking_hours[jonathan] <= hours);
    }
}
