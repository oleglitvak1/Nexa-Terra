//! Headless developer tools for Nexa Terra.
//!
//! Runs the landing site forward without a window and no player input,
//! optionally under a preset set of Standing Orders, logging machine state
//! changes and order firings, then prints where the colony ended up.
//!
//! Usage: `nexa-tools [SOLS] [--orders PRESET]` (presets: content/standing_orders.ron).

use nexa_sim::orders::OrdersDef;
use nexa_sim::{SimEvent, World};
use nexa_types::TICKS_PER_SOL;

/// The vertical slice's length: the scrubbers' filter life.
const DEFAULT_SOLS: u64 = 45;

fn main() {
    let mut sols = DEFAULT_SOLS;
    let mut preset: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--orders" => match args.next() {
                Some(name) => preset = Some(name),
                None => usage("--orders needs a preset name"),
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

    let mut fires = vec![0u64; world.orders.len()];
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
            }
        }
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
}

fn usage(problem: &str) -> ! {
    let presets: Vec<&str> = OrdersDef::shipped()
        .presets
        .keys()
        .map(String::as_str)
        .collect();
    eprintln!("nexa-tools: {problem}");
    eprintln!("usage: nexa-tools [SOLS] [--orders PRESET]");
    eprintln!("presets: {}", presets.join(", "));
    std::process::exit(2);
}
