//! Nexa Terra game client.
//!
//! Milestone 1: an amber terminal that boots, waits for a key, then shows the
//! simulation clock running in real time with pause.

mod terminal;

use macroquad::prelude::*;
use nexa_sim::World;
use nexa_types::Tick;
use terminal::{Ink, Terminal};

const COLS: usize = 80;
const ROWS: usize = 30;

/// Real seconds per simulated hour at 1x speed (GDD: 1 hour = 5 s).
const SECONDS_PER_HOUR_AT_1X: f64 = 5.0;

const BOOT_LINES: &[&str] = &[
    "NEXA TERRA // MULLEIN SYSTEMS v0.1",
    "",
    "MEMORY CHECK .................. 640K OK",
    "REACTOR TELEMETRY ............. LINK",
    "HAB 01 ENVIRONMENT ............ DEGRADED",
    "SCRUBBER ARRAY ................ FILTER LIFE 45 SOLS",
    "CARETAKER PROTOCOL ............ STANDBY",
];

/// Seconds between boot lines appearing.
const BOOT_LINE_DELAY: f64 = 0.35;

enum Screen {
    Boot { started: f64 },
    Online,
}

struct App {
    term: Terminal,
    world: World,
    screen: Screen,
    paused: bool,
    speed: u32,
    hour_accumulator: f64,
}

impl App {
    fn new() -> Self {
        Self {
            term: Terminal::new(COLS, ROWS),
            world: World::landfall(),
            screen: Screen::Boot {
                started: get_time(),
            },
            paused: false,
            speed: 1,
            hour_accumulator: 0.0,
        }
    }

    fn update(&mut self, key: Option<KeyCode>) {
        match self.screen {
            Screen::Boot { started } => {
                let boot_done = get_time() - started > BOOT_LINES.len() as f64 * BOOT_LINE_DELAY;
                // Any key press after boot starts the terminal. This first key
                // press is also what browsers require before audio may play.
                if boot_done && key.is_some() {
                    self.screen = Screen::Online;
                }
            }
            Screen::Online => {
                match key {
                    Some(KeyCode::Space) => self.paused = !self.paused,
                    Some(KeyCode::Key1) => self.speed = 1,
                    Some(KeyCode::Key2) => self.speed = 2,
                    Some(KeyCode::Key3) => self.speed = 3,
                    _ => {}
                }
                if !self.paused {
                    self.hour_accumulator += get_frame_time() as f64 * self.speed as f64;
                    while self.hour_accumulator >= SECONDS_PER_HOUR_AT_1X {
                        self.hour_accumulator -= SECONDS_PER_HOUR_AT_1X;
                        self.world.tick();
                    }
                }
            }
        }
    }

    fn render(&mut self) {
        let blink = ((get_time() * 2.0) as u64).is_multiple_of(2);
        let t = &mut self.term;
        t.clear();
        t.frame(0, 0, t.cols(), t.rows(), Ink::Dim);

        match self.screen {
            Screen::Boot { started } => {
                let shown = ((get_time() - started) / BOOT_LINE_DELAY) as usize;
                for (i, line) in BOOT_LINES.iter().take(shown).enumerate() {
                    let ink = if i == 0 { Ink::Bright } else { Ink::Amber };
                    t.put_str(4, 3 + i, line, ink);
                }
                if shown > BOOT_LINES.len() && blink {
                    t.put_centered(ROWS - 5, "PRESS ANY KEY TO BOOT TERMINAL", Ink::Bright);
                }
            }
            Screen::Online => {
                let now: Tick = self.world.now;
                t.put_str(4, 3, "MULLEIN SYSTEMS // TERMINAL ONLINE", Ink::Bright);
                t.put_str(
                    4,
                    5,
                    &format!("SOL {:03}   HOUR {:02}", now.sol(), now.hour_of_sol()),
                    Ink::Amber,
                );
                let status = if self.paused {
                    "PAUSED".to_string()
                } else {
                    format!("RUNNING {}x", self.speed)
                };
                t.put_str(4, 6, &format!("CLOCK     {status}"), Ink::Amber);
                t.put_str(4, 8, "COLONY SIMULATION NOT YET INSTALLED.", Ink::Dim);
                t.put_str(
                    4,
                    ROWS - 3,
                    "[SPACE] PAUSE   [1] [2] [3] SPEED   [ESC] SHUT DOWN",
                    Ink::Dim,
                );
                if blink {
                    t.put_str(4, 10, "_", Ink::Bright);
                }
            }
        }
        t.draw();
    }
}

fn window_conf() -> Conf {
    Conf {
        window_title: "Nexa Terra".to_owned(),
        window_width: 1280,
        window_height: 720,
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut app = App::new();
    loop {
        let key = get_last_key_pressed();
        if key == Some(KeyCode::Escape) {
            break;
        }
        app.update(key);
        app.render();
        next_frame().await;
    }
}
