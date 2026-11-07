//! Nexa Terra game client.
//!
//! An amber terminal that boots, waits for a key, then shows the colony map
//! and its status, with the simulation running in real time with pause. The
//! Caretaker terminal edits Standing Orders and the crew screen sets the Split,
//! both while the colony is held. `[L]` launches The Mullein into The Slip:
//! the voyage screen flies her Sol by Sol to Scrap Barge 77, and the colony
//! catches up on arrival.
//!
//! Testing: `--seed N` fixes the RNG. `--script FILE` plays a script (see
//! `nexa_sim::script`) through the real screens at full speed, printing the
//! base log; `--headless` plays it with no window. `--screenshot-at
//! MOMENT,... --out DIR` saves PNGs at sim moments or Sols, as well as at the
//! script's own `screenshot` steps.

mod colony_view;
mod crew_view;
mod driver;
mod orders_view;
mod pacing;
mod platform;
mod terminal;
mod ui;
mod voyage_view;

use crew_view::CrewScreen;
use driver::{Act, Driver, Moments, On};
use macroquad::prelude::*;
use nexa_sim::script::{self, Step};
use nexa_sim::universe::OfflineUniverse;
use nexa_sim::{SimEvent, World};
use nexa_types::Tick;
use orders_view::OrdersEditor;
use pacing::Clock;
use platform::Options;
use std::collections::VecDeque;
use terminal::{Ink, Terminal};
use ui::Outcome;
use voyage_view::VoyageScreen;

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
    /// In The Slip the colony is held by the sim until arrival.
    Voyage(VoyageScreen),
}

struct App {
    term: Terminal,
    world: World,
    /// Everything off-colony; offline until the shared universe exists.
    universe: OfflineUniverse,
    screen: Screen,
    clock: Clock,
    /// `--seed`: used at boot instead of the boot timing.
    seed: Option<u64>,
    /// Driven by a script: the clock only moves on `run` steps, and the base
    /// log is printed as well as shown.
    scripted: bool,
    log: VecDeque<String>,
}

impl App {
    fn new(seed: Option<u64>) -> Self {
        Self::with(World::landfall(), seed)
    }

    fn with(world: World, seed: Option<u64>) -> Self {
        Self {
            term: Terminal::new(COLS, ROWS),
            world,
            universe: OfflineUniverse::default(),
            screen: Screen::Boot {
                started: get_time(),
            },
            clock: Clock::default(),
            seed,
            scripted: false,
            log: VecDeque::new(),
        }
    }

    /// Straight to the colony screen with a script's world.
    fn scripted(world: World) -> Self {
        Self {
            screen: Screen::Online,
            scripted: true,
            ..Self::with(world, None)
        }
    }

    fn on(&self) -> On {
        match self.screen {
            Screen::Boot { .. } => On::Boot,
            Screen::Online => On::Colony,
            Screen::Orders(_) => On::Orders,
            Screen::Crew(_) => On::Crew,
            Screen::Voyage(_) => On::Voyage,
        }
    }

    /// Run the colony `hours` hours, logging each.
    fn run_hours(&mut self, hours: u64) {
        for _ in 0..hours {
            let at = self.world.now;
            let events = self.world.tick();
            self.log_events(at, &events);
        }
    }

    /// A script step that goes straight to the sim.
    fn sim_step(&mut self, step: &Step) -> Result<(), String> {
        match step {
            Step::Orders(name) => {
                script::load_preset(&mut self.world, name).map_err(|e| e.to_string())?;
                self.push_log(format!("{} ORDERS LOADED: {}", self.world.now, name));
                Ok(())
            }
            Step::Expect(e) => e.check(&self.world),
            other => Err(format!("{other:?} is not a sim step")),
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
                    // Seed from how long the player took to boot, so each game
                    // rolls differently; the sim itself never reads the clock.
                    let seed = self.seed.unwrap_or_else(|| get_time().to_bits());
                    self.world = World::landfall_seeded(seed);
                    self.screen = Screen::Online;
                }
            }
            Screen::Online => {
                match key {
                    Some(KeyCode::Escape) => return false,
                    Some(KeyCode::O) => self.screen = Screen::Orders(OrdersEditor::new()),
                    Some(KeyCode::C) => self.screen = Screen::Crew(CrewScreen::new(&self.world)),
                    Some(KeyCode::L) => self.launch(),
                    Some(KeyCode::Space) => self.clock.paused = !self.clock.paused,
                    Some(KeyCode::Key1) => self.clock.set_speed(1),
                    Some(KeyCode::Key2) => self.clock.set_speed(2),
                    Some(KeyCode::Key3) => self.clock.set_speed(3),
                    _ => {}
                }
                if !self.scripted {
                    let hours = self.clock.hours(f64::from(get_frame_time()));
                    self.run_hours(u64::from(hours));
                }
            }
            Screen::Orders(editor) => {
                if let Some(key) = key {
                    if editor.handle(key, &mut self.world) == Outcome::Close {
                        self.screen = Screen::Online;
                    }
                }
            }
            Screen::Voyage(screen) => {
                if let Some(key) = key {
                    let outcome = screen.handle(key, &mut self.world);
                    for line in screen.take_base_log() {
                        self.push_log(line);
                    }
                    if outcome == Outcome::Close {
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

    /// Launch, or say in the base log why not.
    fn launch(&mut self) {
        let at = self.world.now;
        match self.world.launch(&mut self.universe) {
            Ok(events) => {
                self.log_events(at, &events);
                self.screen = Screen::Voyage(VoyageScreen::new(&self.world, &events));
            }
            Err(e) => self.push_log(format!(
                "{at} LAUNCH REFUSED: {}",
                voyage_view::launch_refusal(&e)
            )),
        }
    }

    fn push_log(&mut self, line: String) {
        if self.scripted {
            println!("[LOG] {line}");
        }
        if self.log.len() == LOG_CAPACITY {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }

    fn log_events(&mut self, at: Tick, events: &[SimEvent]) {
        for event in events {
            self.push_log(colony_view::log_line(at, event));
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
                let status = if self.clock.paused {
                    "PAUSED".to_string()
                } else {
                    format!("RUNNING {}x", self.clock.speed())
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
                    "[SPACE] PAUSE  [1-3] SPEED  [O] ORDERS  [C] CREW  [L] LAUNCH  [ESC] QUIT",
                    Ink::Dim,
                );
            }
            Screen::Orders(editor) => {
                let title = "MULLEIN SYSTEMS // CARETAKER TERMINAL";
                draw_title(t, title, self.world.now, "HELD");
                editor.draw(t, &self.world, HELP_ROW);
            }
            Screen::Voyage(screen) => {
                let title = "MULLEIN SYSTEMS // COCKPIT";
                let status = match self.world.voyage.as_ref() {
                    Some(v) if v.in_transit() => "IN THE SLIP",
                    _ => "HOLDING",
                };
                draw_title(t, title, self.world.now, status);
                screen.draw(t, &self.world, HELP_ROW);
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

fn main() {
    let opts = match platform::options() {
        Ok(opts) => opts,
        Err(e) => {
            eprintln!("nexa-game: {e}");
            eprintln!("usage: nexa-game [--seed N] [--script FILE [--headless | --screenshot-at MOMENT,... --out DIR]]");
            platform::exit(64);
            return;
        }
    };
    if opts.headless {
        platform::exit(run_headless(&opts));
        return;
    }
    macroquad::Window::from_config(window_conf(), run(opts));
}

/// A script and the world it starts from, or why not.
fn load_script(opts: &Options) -> Result<(Vec<script::Line>, World), String> {
    let path = opts.script.as_deref().unwrap_or_default();
    let lines = script::parse(&platform::read_text(path)?).map_err(|e| format!("{path}: {e}"))?;
    let world = match opts.seed {
        Some(seed) => World::landfall_seeded(seed),
        None => script::start(&lines),
    };
    Ok((lines, world))
}

/// `--headless --script`: the sim alone, logged in the base log's words.
fn run_headless(opts: &Options) -> i32 {
    let (lines, mut world) = match load_script(opts) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("[SCRIPT] {e}");
            return 1;
        }
    };
    let mut universe = OfflineUniverse::default();
    for line in lines.iter().filter(|l| !matches!(l.step, Step::Seed(_))) {
        println!("[SCRIPT] {:>3}: {}", line.number, line.text);
        let mut at = world.now;
        match script::run_step(&mut world, &mut universe, &line.step) {
            Ok(out) => {
                for event in out.events.iter().chain(&out.colony) {
                    if let SimEvent::SolStarted { sol } = event {
                        at = Tick(sol * nexa_types::TICKS_PER_SOL);
                    }
                    println!("[LOG] {}", colony_view::log_line(at, event));
                }
            }
            Err(e) => {
                println!("[SCRIPT] {:>3}: {e}", line.number);
                return e.exit_code();
            }
        }
    }
    println!("[SCRIPT] ran to the end at {}", world.now);
    0
}

async fn run(opts: Options) {
    let code = match &opts.script {
        None => {
            let mut app = App::new(opts.seed);
            while app.update(get_last_key_pressed()) {
                app.render();
                next_frame().await;
            }
            0
        }
        Some(path) => match load_script(&opts) {
            Ok((lines, world)) => {
                println!("[SCRIPT] {path}");
                play(lines, world, &opts).await
            }
            Err(e) => {
                eprintln!("[SCRIPT] {e}");
                1
            }
        },
    };
    platform::exit(code);
}

/// Play a script through the screens, one action per frame, saving screenshots.
async fn play(lines: Vec<script::Line>, world: World, opts: &Options) -> i32 {
    let mut app = App::scripted(world);
    let mut driver = Driver::new(lines);
    let mut moments = Moments::new(opts.shots_at.clone(), &app.world);
    let mut shots = 0;
    let mut last_line = 0;
    loop {
        let mut names = Vec::new();
        let mut exit = None;
        let act = driver.act(app.on(), &app.world);
        if driver.line() != last_line {
            last_line = driver.line();
            if let Some(text) = driver.text() {
                println!("[SCRIPT] {last_line:>3}: {text}");
            }
        }
        let key = match act {
            Act::Key(k) => Some(k),
            Act::Hours(h) => {
                app.run_hours(h);
                None
            }
            Act::Sim(step) => {
                if let Err(e) = app.sim_step(&step) {
                    exit = Some((format!("line {last_line}: {e}"), 1));
                }
                None
            }
            Act::Shot(name) => {
                names.push(name);
                None
            }
            Act::Idle => None,
            Act::Done => {
                println!("[SCRIPT] ran to the end at {}", app.world.now);
                return 0;
            }
            Act::Fail(why, code) => {
                exit = Some((why, code));
                None
            }
        };
        if !app.update(key) {
            return 0;
        }
        names.extend(moments.check(&app.world));
        if exit.is_some() {
            names.push("stopped".into());
        }
        app.render();
        if let (Some(dir), false) = (&opts.out, names.is_empty()) {
            let image = get_screen_data();
            for name in names {
                shots += 1;
                match platform::save_png(&image, dir, &format!("{shots:02}-{name}")) {
                    Ok(path) => println!("[SHOT] {}", path.display()),
                    Err(e) => eprintln!("[SHOT] {e}"),
                }
            }
        }
        if let Some((why, code)) = exit {
            println!("[SCRIPT] {why}");
            return code;
        }
        next_frame().await;
    }
}
