//! Headless developer tools for Nexa Terra.
//!
//! For now this runs the landing site forward without a window and no player
//! input, logging machine state changes, then prints where the colony ended up.
//! Balance runs and content checks come later.

use nexa_sim::{SimEvent, World};
use nexa_types::TICKS_PER_SOL;

fn main() {
    let sols: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(45);

    let mut world = World::landfall();
    for _ in 0..sols * TICKS_PER_SOL {
        let now = world.now;
        for event in world.tick() {
            match event {
                SimEvent::SolStarted { sol } => println!("[SIM] SOL {sol:03} BEGINS"),
                SimEvent::MachineStateChanged { module, state } => println!(
                    "[SIM] SOL {:03} H{:02}  {module} -> {state:?}",
                    now.sol(),
                    now.hour_of_sol()
                ),
            }
        }
    }

    println!(
        "[SIM] ran {sols} Sols headless, now at tick {}",
        world.now.0
    );
    println!("[SIM] grid power {}%", world.colony.power_percent());
    for m in world.colony.modules() {
        if let Some(machine) = m.machine() {
            println!(
                "[SIM]   {:<12} {:<9} {:>6} centi-C  wear {:>3}%",
                m.id().as_str(),
                format!("{:?}", machine.state()),
                m.temp_centi_c(),
                machine.wear_bp() / 100
            );
        }
    }
}
