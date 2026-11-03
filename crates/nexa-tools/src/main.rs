//! Headless developer tools for Nexa Terra.
//!
//! For now this only runs the simulation forward without a window, as a smoke
//! test that the sim works headless. Balance runs and content checks come later.

use nexa_sim::{SimEvent, World};
use nexa_types::TICKS_PER_SOL;

fn main() {
    let sols: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(45);

    let mut world = World::new();
    for _ in 0..sols * TICKS_PER_SOL {
        for event in world.tick() {
            match event {
                SimEvent::SolStarted { sol } => println!("[SIM] SOL {sol:03} BEGINS"),
            }
        }
    }
    println!(
        "[SIM] ran {sols} Sols headless, now at tick {}",
        world.now.0
    );
}
