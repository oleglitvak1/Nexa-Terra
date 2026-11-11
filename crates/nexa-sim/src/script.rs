//! Playthrough scripts: a line-based list of player actions and checks that
//! drives a [`World`] with no window, for tests, headless runs and scripted
//! screenshots.
//!
//! The sim only parses text and runs steps; reading the file is the caller's
//! job. One step per line, `#` starts a comment:
//!
//! ```text
//! seed 7                    # first step only: landfall with this seed
//! orders caretaker          # replace the cards with a posture from content/standing_orders.ron
//! run 10                    # run the colony 10 Sols
//! run-to 10                 # run the colony until Sol 10 begins
//! split cal,jonathan,dax    # confirm a Split: these fly, the rest stay
//! launch
//! advance                   # fly one Sol of The Slip
//! resolve overclock         # answer the pending hazard
//! fly overclock             # fly to arrival, answering every hazard this way
//! close                     # front ends: back to the colony screen
//! screenshot hazard         # front ends: capture the screen as hazard.png
//! expect fuel 500           # fail the script unless this holds
//! ```
//!
//! `expect` keys: `sol`, `fuel`, `pods`, `credits`, `crates`, `ore` (kg in the
//! ground stockpile), `integrity`,
//! `delay`, `random-delay`, `settled` (hazards answered), `ship` (pad, slip, holding), `machine ID STATE`,
//! `stress CREW LEVEL`, `side CREW SIDE`. Verbs for systems not built yet
//! (`dock`, `haggle`, `give-fuel`, `audit`, `return`, `land`, `install-filters`)
//! parse, and stop the run with [`RunError::NotBuilt`] when reached.

use crate::crew::Assignment;
use crate::flight::VoyageStep;
use crate::orders::OrdersDef;
use crate::ship::ShipLocation;
use crate::universe::Universe;
use crate::{SimEvent, World};
use nexa_types::{CrewId, ModuleId, TICKS_PER_SOL};
use std::collections::BTreeMap;

/// Verbs a script may use for systems that don't exist yet (milestones 6 to 8).
pub const NOT_BUILT: &[&str] = &[
    "dock",
    "haggle",
    "give-fuel",
    "audit",
    "return",
    "land",
    "install-filters",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Only valid as the first step.
    Seed(u64),
    Orders(String),
    Run(u64),
    RunTo(u64),
    Split(Vec<CrewId>),
    Launch,
    Advance,
    Resolve(String),
    Fly(String),
    Close,
    Screenshot(String),
    Expect(Expect),
    /// A verb for a system that isn't built yet.
    NotBuilt(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expect {
    Sol(u64),
    Fuel(u32),
    Pods(u8),
    Credits(u32),
    Crates(usize),
    /// Ore in the ground stockpile, in kg.
    Ore(u32),
    Integrity(u8),
    Delay(u32),
    RandomDelay(u32),
    /// Hazards settled on the voyage so far.
    Settled(usize),
    Ship(String),
    Machine(ModuleId, String),
    Stress(CrewId, String),
    Side(CrewId, String),
}

impl Expect {
    /// `Err` says what was found instead.
    pub fn check(&self, world: &World) -> Result<(), String> {
        let voyage = world.voyage.as_ref();
        match self {
            Expect::Sol(n) => cmp("sol", *n, world.now.sol()),
            Expect::Fuel(n) => cmp("fuel", *n, world.ship.fuel_l()),
            Expect::Pods(n) => cmp("pods", *n, world.ship.pods()),
            Expect::Credits(n) => cmp("credits", *n, world.credits),
            Expect::Crates(n) => cmp("crates", *n, world.ship.cargo().len()),
            Expect::Ore(n) => cmp("ore", *n, world.colony.ore_kg()),
            Expect::Integrity(n) => cmp("integrity", *n, world.ship.cargo_integrity_pct()),
            Expect::Delay(n) => cmp("delay", *n, voyage.map_or(0, |v| v.delay_sols())),
            Expect::RandomDelay(n) => cmp(
                "random-delay",
                *n,
                voyage.map_or(0, |v| v.random_delay_sols()),
            ),
            Expect::Settled(n) => cmp("settled", *n, voyage.map_or(0, |v| v.resolutions().len())),
            Expect::Ship(want) => {
                let have = match world.ship.location() {
                    ShipLocation::Pad => "pad",
                    ShipLocation::InSlip => "slip",
                    ShipLocation::Holding(_) => "holding",
                };
                cmp("ship", want.as_str(), have)
            }
            Expect::Machine(id, want) => {
                let have = world
                    .colony
                    .module(id)
                    .and_then(|m| m.machine())
                    .map_or("none", |m| m.state().label());
                cmp(&format!("machine {id}"), want.as_str(), have)
            }
            Expect::Stress(id, want) => {
                let have = world.crew.vitals(id).map_or("none", |v| v.level.label());
                cmp(&format!("stress {id}"), want.as_str(), have)
            }
            Expect::Side(id, want) => {
                let have = match world.crew.assignment(id) {
                    Some(Assignment::Flight) => "flight",
                    Some(Assignment::Ground) => "ground",
                    None => "none",
                };
                cmp(&format!("side {id}"), want.as_str(), have)
            }
        }
    }
}

fn cmp<T: PartialEq + std::fmt::Display>(what: &str, want: T, have: T) -> Result<(), String> {
    if want == have {
        Ok(())
    } else {
        Err(format!("expected {what} {want}, found {have}"))
    }
}

/// A step and the 1-based line it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub number: usize,
    pub step: Step,
    /// The line as written, comment stripped.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

pub fn parse(text: &str) -> Result<Vec<Line>, ParseError> {
    let mut lines = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let number = i + 1;
        let content = raw.split('#').next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        let words: Vec<&str> = content.split_whitespace().collect();
        let step = parse_step(&words).map_err(|message| ParseError {
            line: number,
            message,
        })?;
        if matches!(step, Step::Seed(_)) && !lines.is_empty() {
            return Err(ParseError {
                line: number,
                message: "seed must be the first step".into(),
            });
        }
        lines.push(Line {
            number,
            step,
            text: content.to_string(),
        });
    }
    Ok(lines)
}

fn parse_step(words: &[&str]) -> Result<Step, String> {
    let verb = words[0];
    let args = &words[1..];
    let one = |name: &str| -> Result<String, String> {
        match args {
            [a] => Ok((*a).to_string()),
            _ => Err(format!("{verb} takes one {name}")),
        }
    };
    let none = |step: Step| -> Result<Step, String> {
        if args.is_empty() {
            Ok(step)
        } else {
            Err(format!("{verb} takes no arguments"))
        }
    };
    match verb {
        "seed" => Ok(Step::Seed(number(&one("number")?)?)),
        "orders" => Ok(Step::Orders(one("preset")?)),
        "run" => Ok(Step::Run(sols(&one("number of Sols")?)?)),
        "run-to" => Ok(Step::RunTo(sols(&one("Sol")?)?)),
        "split" => {
            let ids: Vec<CrewId> = one("crew list")?
                .split(',')
                .filter(|s| !s.is_empty())
                .map(CrewId::new)
                .collect();
            Ok(Step::Split(ids))
        }
        "launch" => none(Step::Launch),
        "advance" => none(Step::Advance),
        "resolve" => Ok(Step::Resolve(one("option")?)),
        "fly" => Ok(Step::Fly(one("option")?)),
        "close" => none(Step::Close),
        "screenshot" => Ok(Step::Screenshot(one("name")?)),
        "expect" => parse_expect(args).map(Step::Expect),
        v if NOT_BUILT.contains(&v) => Ok(Step::NotBuilt(words.join(" "))),
        v => Err(format!("unknown step {v:?}")),
    }
}

fn parse_expect(args: &[&str]) -> Result<Expect, String> {
    let s = |a: &str| a.to_string();
    Ok(match args {
        ["sol", n] => Expect::Sol(number(n)?),
        ["fuel", n] => Expect::Fuel(number(n)?),
        ["pods", n] => Expect::Pods(number(n)?),
        ["credits", n] => Expect::Credits(number(n)?),
        ["crates", n] => Expect::Crates(number(n)?),
        ["ore", n] => Expect::Ore(number(n)?),
        ["integrity", n] => Expect::Integrity(number(n)?),
        ["delay", n] => Expect::Delay(number(n)?),
        ["random-delay", n] => Expect::RandomDelay(number(n)?),
        ["settled", n] => Expect::Settled(number(n)?),
        ["ship", w] => Expect::Ship(s(w)),
        ["machine", id, w] => Expect::Machine(ModuleId::new(*id), s(w)),
        ["stress", id, w] => Expect::Stress(CrewId::new(*id), s(w)),
        ["side", id, w] => Expect::Side(CrewId::new(*id), s(w)),
        _ => return Err(format!("can't read expect {:?}", args.join(" "))),
    })
}

fn number<T: std::str::FromStr>(text: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("{text:?} is not a whole number"))
}

/// A count of Sols small enough to turn into ticks.
fn sols(text: &str) -> Result<u64, String> {
    let n: u64 = number(text)?;
    n.checked_mul(TICKS_PER_SOL)
        .map(|_| n)
        .ok_or_else(|| format!("{n} Sols is too many"))
}

/// Why a step couldn't run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The sim refused the action.
    Refused(String),
    /// An `expect` didn't hold.
    Expectation(String),
    /// The script reached a system that isn't built yet.
    NotBuilt(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Refused(e) => write!(f, "refused: {e}"),
            RunError::Expectation(e) => write!(f, "expectation failed: {e}"),
            RunError::NotBuilt(v) => write!(f, "not built yet: {v}"),
        }
    }
}

impl std::error::Error for RunError {}

impl RunError {
    /// The process exit code for a script that stopped here: 2 for a system
    /// that isn't built yet, 1 for anything else.
    pub fn exit_code(&self) -> i32 {
        match self {
            RunError::NotBuilt(_) => 2,
            RunError::Refused(_) | RunError::Expectation(_) => 1,
        }
    }
}

/// What one step did. `colony` holds the hours caught up on arrival.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub events: Vec<SimEvent>,
    pub colony: Vec<SimEvent>,
}

impl Outcome {
    fn add_voyage(&mut self, step: VoyageStep) {
        self.events.extend(step.events);
        self.colony.extend(step.colony);
    }
}

/// The world a script starts from: its `seed` step if it has one.
pub fn start(lines: &[Line]) -> World {
    match lines.first().map(|l| &l.step) {
        Some(Step::Seed(seed)) => World::landfall_seeded(*seed),
        _ => World::landfall(),
    }
}

/// Load a shipped posture through [`World::load_posture`]: its cards replace
/// the current ones. Refused, changing nothing, if it doesn't exist or doesn't fit.
pub fn load_preset(world: &mut World, name: &str) -> Result<(), RunError> {
    let preset = OrdersDef::shipped()
        .presets
        .get(name)
        .ok_or_else(|| RunError::Refused(format!("no preset named {name:?}")))?;
    world
        .load_posture(preset)
        .map_err(|e| RunError::Refused(format!("preset {name}: {e}")))
}

/// The Split a `split` step asks for: those listed fly, the rest stay.
pub fn assignments(flight: &[CrewId]) -> BTreeMap<CrewId, Assignment> {
    flight
        .iter()
        .map(|id| (id.clone(), Assignment::Flight))
        .collect()
}

/// Run one step at full speed. `Close` and `Screenshot` do nothing here; they
/// are for front ends that show screens.
pub fn run_step(
    world: &mut World,
    universe: &mut dyn Universe,
    step: &Step,
) -> Result<Outcome, RunError> {
    let refused = |e: &dyn std::fmt::Display| RunError::Refused(e.to_string());
    let in_transit = |world: &World| world.voyage.as_ref().is_some_and(|v| v.in_transit());
    // The colony is held in transit: ticking would pass no time.
    let colony_held = || RunError::Refused("the colony is held in transit".into());
    let mut out = Outcome::default();
    match step {
        // `start` already used it.
        Step::Seed(_) | Step::Close | Step::Screenshot(_) => {}
        Step::Orders(name) => load_preset(world, name)?,
        Step::Run(sols) => {
            if in_transit(world) {
                return Err(colony_held());
            }
            for _ in 0..sols * TICKS_PER_SOL {
                out.events.extend(world.tick());
            }
        }
        Step::RunTo(sol) => {
            if in_transit(world) {
                return Err(colony_held());
            }
            while world.now.sol() < *sol {
                out.events.extend(world.tick());
            }
        }
        Step::Split(flight) => {
            out.events = world
                .confirm_split(&assignments(flight))
                .map_err(|e| refused(&e))?;
        }
        Step::Launch => out.events = world.launch(universe).map_err(|e| refused(&e))?,
        Step::Advance => out.add_voyage(world.advance_voyage().map_err(|e| refused(&e))?),
        Step::Resolve(option) => {
            out.events = world.resolve_hazard(option).map_err(|e| refused(&e))?;
        }
        Step::Fly(option) => {
            if !in_transit(world) {
                return Err(RunError::Refused("no voyage in transit to fly".into()));
            }
            while in_transit(world) {
                if world.voyage.as_ref().is_some_and(|v| v.pending().is_some()) {
                    out.events
                        .extend(world.resolve_hazard(option).map_err(|e| refused(&e))?);
                } else {
                    out.add_voyage(world.advance_voyage().map_err(|e| refused(&e))?);
                }
            }
        }
        Step::Expect(e) => e.check(world).map_err(RunError::Expectation)?,
        Step::NotBuilt(verb) => return Err(RunError::NotBuilt(verb.clone())),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::universe::OfflineUniverse;

    fn run_all(text: &str) -> (World, Result<(), (usize, RunError)>) {
        let lines = parse(text).unwrap();
        let mut world = start(&lines);
        let mut universe = OfflineUniverse::default();
        for line in &lines {
            if let Err(e) = run_step(&mut world, &mut universe, &line.step) {
                return (world, Err((line.number, e)));
            }
        }
        (world, Ok(()))
    }

    #[test]
    fn parses_every_verb_and_skips_comments() {
        let lines = parse(
            "seed 7\n# a comment\n\norders caretaker  # trailing\nrun 2\nrun-to 3\n\
             split cal,dax\nlaunch\nadvance\nresolve pay\nfly dump\nclose\n\
             screenshot x\nexpect fuel 650\nexpect machine hab_01 nominal\nhaggle hard",
        )
        .unwrap();
        assert_eq!(lines.len(), 14);
        assert_eq!(lines[1].number, 4);
        assert_eq!(
            lines[4].step,
            Step::Split(vec![CrewId::new("cal"), CrewId::new("dax")])
        );
        assert_eq!(lines[13].step, Step::NotBuilt("haggle hard".into()));
    }

    #[test]
    fn parse_errors_name_the_line() {
        assert_eq!(parse("run 1\nwarp 9").unwrap_err().line, 2);
        assert_eq!(parse("run x").unwrap_err().line, 1);
        assert_eq!(parse("run 1\nseed 3").unwrap_err().line, 2);
        assert_eq!(parse("expect fuel").unwrap_err().line, 1);
    }

    #[test]
    fn a_script_flies_the_slip_and_checks_the_result() {
        let (world, result) = run_all(
            "seed 7\nsplit cal,jonathan,dax\nlaunch\nexpect fuel 650\n\
             fly pay\nexpect ship holding\nexpect credits 200\nexpect fuel 650",
        );
        assert_eq!(result, Ok(()));
        assert!(world.voyage.is_some());
    }

    #[test]
    fn same_script_same_world() {
        let script = "seed 11\norders caretaker\nrun 3\nsplit cal,jonathan,dax\nlaunch\nfly bluff";
        assert_eq!(run_all(script).0, run_all(script).0);
    }

    #[test]
    fn failures_stop_the_run_with_the_line() {
        let (_, r) = run_all("run 1\nexpect sol 2");
        assert!(matches!(r, Err((2, RunError::Expectation(_)))));
        let (_, r) = run_all("launch");
        assert!(matches!(r, Err((1, RunError::Refused(_)))));
        let (_, r) = run_all("land");
        assert!(matches!(r, Err((1, RunError::NotBuilt(_)))));
    }

    #[test]
    fn steps_that_would_do_nothing_are_refused() {
        let (_, r) = run_all("fly pay");
        assert!(matches!(r, Err((1, RunError::Refused(_)))));
        let (_, r) = run_all("seed 7\nsplit cal,jonathan,dax\nlaunch\nrun 1");
        assert!(matches!(r, Err((4, RunError::Refused(_)))));
        let (_, r) = run_all("seed 7\nsplit cal,jonathan,dax\nlaunch\nfly pay\nfly pay");
        assert!(matches!(r, Err((5, RunError::Refused(_)))));
        let (_, r) = run_all("split cal,jonathon,dax");
        assert!(matches!(r, Err((1, RunError::Refused(_)))));
    }

    #[test]
    fn orders_replaces_the_cards_and_a_refused_preset_keeps_them() {
        let (world, r) = run_all("orders doc-example\norders protect-habs");
        assert_eq!(r, Ok(()));
        let protect = &OrdersDef::shipped().presets["protect-habs"].cards;
        assert_eq!(
            world.orders.iter().collect::<Vec<_>>(),
            protect.iter().collect::<Vec<_>>()
        );
        let (world, r) = run_all("orders protect-habs\norders nonsense");
        assert!(matches!(r, Err((2, RunError::Refused(_)))));
        assert_eq!(world.orders.len(), protect.len());
    }

    #[test]
    fn expect_ore_checks_the_stockpile() {
        // The shipped hold is full, so a Sol of smelting stays on the ground.
        let rate = World::landfall().colony.ore_def().unwrap().kg_per_hour;
        let sol = u64::from(rate) * TICKS_PER_SOL;
        let (_, r) = run_all(&format!("expect ore 0\nrun 1\nexpect ore {sol}"));
        assert_eq!(r, Ok(()));
        let (_, r) = run_all("run 1\nexpect ore 0");
        assert!(matches!(r, Err((2, RunError::Expectation(_)))));
        assert_eq!(parse("expect ore lots").unwrap_err().line, 1);
    }

    #[test]
    fn sol_counts_that_overflow_ticks_dont_parse() {
        assert_eq!(parse("run 1\nrun 800000000000000000").unwrap_err().line, 2);
        assert!(parse("run-to 18446744073709551615").is_err());
    }
}
