//! Playthrough scripts: a line-based list of player actions and checks that
//! drives a [`World`] with no window, for tests, headless runs and scripted
//! screenshots.
//!
//! The sim only parses text and runs steps; reading the file is the caller's
//! job. One step per line, `#` starts a comment:
//!
//! ```text
//! seed 7                    # first step only: landfall with this seed
//! orders caretaker          # load a preset from content/standing_orders.ron
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
//! `expect` keys: `sol`, `fuel`, `pods`, `credits`, `crates`, `integrity`,
//! `delay`, `random-delay`, `ship` (pad, slip, holding), `machine ID STATE`,
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

/// Verbs a script may use for systems that don't exist yet (milestone 6).
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
    Integrity(u8),
    Delay(u32),
    RandomDelay(u32),
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
            Expect::Integrity(n) => cmp("integrity", *n, world.ship.cargo_integrity_pct()),
            Expect::Delay(n) => cmp("delay", *n, voyage.map_or(0, |v| v.delay_sols())),
            Expect::RandomDelay(n) => cmp(
                "random-delay",
                *n,
                voyage.map_or(0, |v| v.random_delay_sols()),
            ),
            Expect::Ship(want) => {
                let have = match world.ship.location() {
                    ShipLocation::Pad => "pad",
                    ShipLocation::InSlip => "slip",
                    ShipLocation::Holding(_) => "holding",
                };
                same("ship", want, have)
            }
            Expect::Machine(id, want) => {
                let have = world
                    .colony
                    .module(id)
                    .and_then(|m| m.machine())
                    .map_or("none", |m| m.state().label());
                same(&format!("machine {id}"), want, have)
            }
            Expect::Stress(id, want) => {
                let have = world.crew.vitals(id).map_or("none", |v| v.level.label());
                same(&format!("stress {id}"), want, have)
            }
            Expect::Side(id, want) => {
                let have = match world.crew.assignment(id) {
                    Some(Assignment::Flight) => "flight",
                    Some(Assignment::Ground) => "ground",
                    None => "none",
                };
                same(&format!("side {id}"), want, have)
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

fn same(what: &str, want: &str, have: &str) -> Result<(), String> {
    cmp(what, want, have)
}

/// A step and the 1-based line it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub number: usize,
    pub step: Step,
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
        lines.push(Line { number, step });
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
        "run" => Ok(Step::Run(number(&one("number of Sols")?)?)),
        "run-to" => Ok(Step::RunTo(number(&one("Sol")?)?)),
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
        ["integrity", n] => Expect::Integrity(number(n)?),
        ["delay", n] => Expect::Delay(number(n)?),
        ["random-delay", n] => Expect::RandomDelay(number(n)?),
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
            RunError::NotBuilt(v) => write!(f, "not built yet (milestone 6): {v}"),
        }
    }
}

impl std::error::Error for RunError {}

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

/// Load a Standing Orders preset; refused if it doesn't exist or doesn't fit.
pub fn load_preset(world: &mut World, name: &str) -> Result<(), RunError> {
    let cards = OrdersDef::shipped()
        .presets
        .get(name)
        .ok_or_else(|| RunError::Refused(format!("no preset named {name:?}")))?;
    for card in cards {
        world
            .add_order(card.clone())
            .map_err(|e| RunError::Refused(format!("preset {name}: {e}")))?;
    }
    Ok(())
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
    let mut out = Outcome::default();
    match step {
        // `start` already used it.
        Step::Seed(_) | Step::Close | Step::Screenshot(_) => {}
        Step::Orders(name) => load_preset(world, name)?,
        Step::Run(sols) => {
            for _ in 0..sols * TICKS_PER_SOL {
                out.events.extend(world.tick());
            }
        }
        Step::RunTo(sol) => {
            if world.voyage.as_ref().is_some_and(|v| v.in_transit()) {
                return Err(RunError::Refused("the colony is held in transit".into()));
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
            while world.voyage.as_ref().is_some_and(|v| v.in_transit()) {
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
}
