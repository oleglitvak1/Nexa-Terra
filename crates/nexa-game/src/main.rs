//! Nexa Terra game client.
//!
//! An amber terminal that boots, waits for a key, then shows the colony map
//! and its status, with the simulation running in real time with pause. The
//! Caretaker terminal edits Standing Orders and the crew screen sets the Split,
//! both while the colony is held.

mod colony_view;
mod crew_view;
mod orders_view;
mod terminal;
mod ui;

use crew_view::CrewScreen;
use macroquad::prelude::*;
use nexa_sim::{SimEvent, World};
use nexa_types::Tick;
use orders_view::OrdersEditor;
use std::collections::VecDeque;
use terminal::{Ink, Terminal};
use ui::Outcome;

const COLS: usize = 80;
/// Tall enough for the 32-row map plus a header and key-help row inside the frame.
const ROWS: usize = 36;

/// Columns between the frame and text on every screen.
const MARGIN: usize = 2;
const TITLE_ROW: usize = 1;
/// Key help sits on the last row inside the frame.
const HELP_ROW: usize = ROWS - 2;
const BOOT_COL: usize = 4;
const BOOT_ROW: usize = 3;
const BOOT_PROMPT_ROW: usize = ROWS - 5;

/// Log lines kept for the base log; more than the screen shows.
const LOG_CAPACITY: usize = 64;

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
    Boot {
        started: f64,
    },
    Online,
    /// The colony doesn't run while the terminal is open.
    Orders(OrdersEditor),
    /// Held too: the Split is set between hours.
    Crew(CrewScreen),
}

struct App {
    term: Terminal,
    world: World,
    screen: Screen,
    paused: bool,
    speed: u32,
    hour_accumulator: f64,
    log: VecDeque<String>,
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
            log: VecDeque::new(),
        }
    }

    /// False when the player shuts the game down.
    fn update(&mut self, key: Option<KeyCode>) -> bool {
        match &mut self.screen {
            Screen::Boot { started } => {
                if key == Some(KeyCode::Escape) {
                    return false;
                }
                let boot_done = get_time() - *started > BOOT_LINES.len() as f64 * BOOT_LINE_DELAY;
                // Any key press after boot starts the terminal. This first key
                // press is also what browsers require before audio may play.
                if boot_done && key.is_some() {
                    self.screen = Screen::Online;
                }
            }
            Screen::Online => {
                match key {
                    Some(KeyCode::Escape) => return false,
                    Some(KeyCode::O) => self.screen = Screen::Orders(OrdersEditor::new()),
                    Some(KeyCode::C) => self.screen = Screen::Crew(CrewScreen::new(&self.world)),
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
                        let at = self.world.now;
                        let events = self.world.tick();
                        self.log_events(at, &events);
                    }
                }
            }
            Screen::Orders(editor) => {
                if let Some(key) = key {
                    if editor.handle(key, &mut self.world) == Outcome::Close {
                        self.screen = Screen::Online;
                    }
                }
            }
            Screen::Crew(screen) => {
                if let Some(key) = key {
                    let mut events = Vec::new();
                    let outcome = screen.handle(key, &mut self.world, &mut events);
                    let at = self.world.now;
                    self.log_events(at, &events);
                    if outcome == Outcome::Close {
                        self.screen = Screen::Online;
                    }
                }
            }
        }
        true
    }

    fn log_events(&mut self, at: Tick, events: &[SimEvent]) {
        for event in events {
            if self.log.len() == LOG_CAPACITY {
                self.log.pop_front();
            }
            self.log.push_back(colony_view::log_line(at, event));
        }
    }

    fn render(&mut self) {
        let blink = ((get_time() * 2.0) as u64).is_multiple_of(2);
        let t = &mut self.term;
        t.clear();
        t.frame(0, 0, t.cols(), t.rows(), Ink::Dim);

        match &self.screen {
            Screen::Boot { started } => {
                let shown = ((get_time() - *started) / BOOT_LINE_DELAY) as usize;
                for (i, line) in BOOT_LINES.iter().take(shown).enumerate() {
                    let ink = if i == 0 { Ink::Bright } else { Ink::Amber };
                    t.put_str(BOOT_COL, BOOT_ROW + i, line, ink);
                }
                if shown > BOOT_LINES.len() && blink {
                    t.put_centered(
                        BOOT_PROMPT_ROW,
                        "PRESS ANY KEY TO BOOT TERMINAL",
                        Ink::Bright,
                    );
                }
            }
            Screen::Online => {
                let status = if self.paused {
                    "PAUSED".to_string()
                } else {
                    format!("RUNNING {}x", self.speed)
                };
                draw_title(
                    t,
                    "MULLEIN SYSTEMS // ASH PLAIN COLONY",
                    self.world.now,
                    &status,
                );
                colony_view::draw_map(t, &self.world.colony, blink);
                let next = colony_view::draw_status(t, &self.world.colony, colony_view::MAP_ROW);
                colony_view::draw_log(t, self.log.iter(), next + 1, HELP_ROW);
                t.put_str(
                    MARGIN,
                    HELP_ROW,
                    "[SPACE] PAUSE   [1-3] SPEED   [O] ORDERS   [C] CREW   [ESC] SHUT DOWN",
                    Ink::Dim,
                );
            }
            Screen::Orders(editor) => {
                let title = "MULLEIN SYSTEMS // CARETAKER TERMINAL";
                draw_title(t, title, self.world.now, "HELD");
                editor.draw(t, &self.world, HELP_ROW);
            }
            Screen::Crew(screen) => {
                let title = "MULLEIN SYSTEMS // CREW QUARTERS";
                draw_title(t, title, self.world.now, "HELD");
                screen.draw(t, &self.world, HELP_ROW);
            }
        }
        t.draw();
    }
}

/// Title on the left; the clock right-aligned inside the frame so it never runs into the title.
fn draw_title(t: &mut Terminal, title: &str, now: Tick, status: &str) {
    t.put_str(MARGIN, TITLE_ROW, title, Ink::Bright);
    let clock = format!(
        "SOL {:03}  HOUR {:02}  {status}",
        now.sol(),
        now.hour_of_sol()
    );
    let clock_col = COLS.saturating_sub(MARGIN + clock.chars().count());
    t.put_str(clock_col, TITLE_ROW, &clock, Ink::Amber);
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
        if !app.update(get_last_key_pressed()) {
            break;
        }
        app.render();
        next_frame().await;
    }
}
