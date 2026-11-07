//! `--script FILE`: play a playthrough script headless and log what happened.

use crate::log_event;
use nexa_sim::script::{self, Step};
use nexa_sim::universe::OfflineUniverse;
use nexa_sim::{SimEvent, World};
use nexa_types::{Tick, TICKS_PER_SOL};

/// Order slots counted for the log; more than the terminal can hold.
const SLOTS: usize = 64;

/// Run the file; the return value is the process exit code.
pub fn run_file(path: &str, seed: Option<u64>) -> i32 {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("[SCRIPT] can't read {path}: {e}");
            return 1;
        }
    };
    let lines = match script::parse(&text) {
        Ok(lines) => lines,
        Err(e) => {
            eprintln!("[SCRIPT] {path}: {e}");
            return 1;
        }
    };
    let mut world = match seed {
        Some(seed) => World::landfall_seeded(seed),
        None => script::start(&lines),
    };
    println!("[SCRIPT] {path}");
    let code = run(&mut world, &lines);
    println!("{}", summary(&world));
    code
}

fn run(world: &mut World, lines: &[script::Line]) -> i32 {
    let mut universe = OfflineUniverse::default();
    let mut fires = vec![0u64; SLOTS];
    for line in lines {
        if matches!(line.step, Step::Seed(_)) {
            continue;
        }
        println!("[SCRIPT] {:>3}: {}", line.number, line.text);
        let now = world.now;
        match script::run_step(world, &mut universe, &line.step) {
            Ok(out) => {
                log_stamped(now, &out.events, &mut fires);
                log_stamped(now, &out.colony, &mut fires);
            }
            Err(e) => {
                println!("[SCRIPT] {:>3}: {e}", line.number);
                return e.exit_code();
            }
        }
    }
    println!("[SCRIPT] ran to the end");
    0
}

/// Events from a step that may span Sols (a run, or the catch-up on
/// arrival), stamped with the Sol they fell in.
fn log_stamped(departed: Tick, events: &[SimEvent], fires: &mut [u64]) {
    let mut stamp = departed;
    for event in events {
        if let SimEvent::SolStarted { sol } = event {
            stamp = Tick(sol * TICKS_PER_SOL);
        }
        log_event(stamp, event, fires);
    }
}

/// One line on where the world ended up.
pub fn summary(world: &World) -> String {
    let ship = &world.ship;
    let voyage = world.voyage.as_ref().map_or("no voyage".to_string(), |v| {
        format!(
            "{:?} {} of {} Sols, delay +{} (random {})",
            v.status(),
            v.sols_flown(),
            v.sols_total(),
            v.delay_sols(),
            v.random_delay_sols()
        )
    });
    format!(
        "[END] {} fuel {} L, pods {}, credits {}, crates {}, integrity {}%, {voyage}",
        world.now,
        ship.fuel_l(),
        ship.pods(),
        world.credits,
        ship.cargo().len(),
        ship.cargo_integrity_pct()
    )
}
