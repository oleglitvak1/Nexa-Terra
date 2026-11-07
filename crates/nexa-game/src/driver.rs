//! Plays a script through the real screens: player actions become the key
//! presses a player would make (the Split on the crew screen, launch and
//! hazards on the cockpit screen), so a scripted run exercises the same input
//! code. Running the colony and loading an orders preset go straight to the
//! sim; the orders editor builds cards one slot at a time, which a script
//! doesn't need to repeat.

use crate::platform::ShotAt;
use macroquad::prelude::KeyCode;
use nexa_sim::crew::Assignment;
use nexa_sim::script::{Expect, Line, Step};
use nexa_sim::ship::ShipLocation;
use nexa_sim::World;
use nexa_types::{CrewId, TICKS_PER_SOL};
use std::collections::VecDeque;

/// Which screen the app shows, as far as the driver needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum On {
    Boot,
    Colony,
    Orders,
    Crew,
    Voyage,
}

/// What the app should do this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    Key(KeyCode),
    /// Run the colony this many hours, logging as it goes.
    Hours(u64),
    /// Load an orders preset or check an expectation, straight on the sim.
    Sim(Step),
    Shot(String),
    /// Nothing this frame (a step finished without needing input).
    Idle,
    Done,
    /// Stop: what went wrong, and the exit code.
    Fail(String, i32),
}

pub struct Driver {
    lines: Vec<Line>,
    next: usize,
    /// The line being played, for error messages.
    line: usize,
    queue: VecDeque<Act>,
    /// `fly OPTION` while it lasts, and how many hazards were settled when it
    /// last answered one, so a refused answer stops instead of looping.
    flying: Option<(String, Option<usize>)>,
}

impl Driver {
    pub fn new(lines: Vec<Line>) -> Self {
        Self {
            lines,
            next: 0,
            line: 0,
            queue: VecDeque::new(),
            flying: None,
        }
    }

    /// The line being played.
    pub fn line(&self) -> usize {
        self.line
    }

    /// The text of the line being played.
    pub fn text(&self) -> Option<&str> {
        self.lines
            .iter()
            .find(|l| l.number == self.line)
            .map(|l| l.text.as_str())
    }

    pub fn act(&mut self, on: On, world: &World) -> Act {
        if let Some(act) = self.queue.pop_front() {
            return act;
        }
        if let Some((option, tried)) = self.flying.clone() {
            return self.fly(on, world, &option, tried);
        }
        let Some(line) = self.lines.get(self.next).cloned() else {
            return Act::Done;
        };
        self.next += 1;
        self.line = line.number;
        self.plan(&line.step, on, world)
    }

    fn fail(&self, why: impl std::fmt::Display) -> Act {
        Act::Fail(format!("line {}: {why}", self.line), 1)
    }

    fn need(&self, on: On, want: On, step: &str) -> Option<Act> {
        (on != want).then(|| self.fail(format!("{step} needs the {want:?} screen, not {on:?}")))
    }

    fn plan(&mut self, step: &Step, on: On, world: &World) -> Act {
        match step {
            Step::Seed(_) => Act::Idle,
            Step::Orders(_) | Step::Expect(_) => Act::Sim(step.clone()),
            Step::Screenshot(name) => Act::Shot(name.clone()),
            Step::Run(sols) => self
                .need(on, On::Colony, "run")
                .unwrap_or(Act::Hours(sols * TICKS_PER_SOL)),
            Step::RunTo(sol) => self
                .need(on, On::Colony, "run-to")
                .unwrap_or_else(|| Act::Hours((sol * TICKS_PER_SOL).saturating_sub(world.now.0))),
            Step::Split(flight) => {
                if let Some(no) = self.need(on, On::Colony, "split") {
                    return no;
                }
                self.queue.extend(split_keys(world, flight));
                Act::Idle
            }
            Step::Launch => {
                if let Some(no) = self.need(on, On::Colony, "launch") {
                    return no;
                }
                self.queue
                    .push_back(Act::Sim(Step::Expect(Expect::Ship("slip".into()))));
                Act::Key(KeyCode::L)
            }
            Step::Advance => self
                .need(on, On::Voyage, "advance")
                .unwrap_or(Act::Key(KeyCode::Enter)),
            Step::Resolve(option) => {
                if let Some(no) = self.need(on, On::Voyage, "resolve") {
                    return no;
                }
                match resolve_keys(world, option) {
                    Ok(keys) => {
                        let settled = settled(world);
                        self.queue.extend(keys);
                        // Refused answers leave the hazard pending: catch that.
                        self.queue
                            .push_back(Act::Sim(Step::Expect(Expect::Settled(settled + 1))));
                        Act::Idle
                    }
                    Err(e) => self.fail(e),
                }
            }
            Step::Fly(option) => {
                if let Some(no) = self.need(on, On::Voyage, "fly") {
                    return no;
                }
                self.flying = Some((option.clone(), None));
                Act::Idle
            }
            Step::Close => match on {
                On::Crew | On::Orders => Act::Key(KeyCode::Escape),
                On::Voyage if world.voyage.as_ref().is_some_and(|v| !v.in_transit()) => {
                    Act::Key(KeyCode::Escape)
                }
                _ => self.fail(format!("nothing to close on the {on:?} screen")),
            },
            Step::NotBuilt(verb) => Act::Fail(
                format!("line {}: not built yet (milestone 6): {verb}", self.line),
                2,
            ),
        }
    }

    fn fly(&mut self, on: On, world: &World, option: &str, tried: Option<usize>) -> Act {
        if let Some(no) = self.need(on, On::Voyage, "fly") {
            self.flying = None;
            return no;
        }
        let Some(voyage) = world.voyage.as_ref() else {
            self.flying = None;
            return self.fail("no voyage to fly");
        };
        if !voyage.in_transit() {
            self.flying = None;
            return Act::Idle;
        }
        if voyage.pending().is_none() {
            return Act::Key(KeyCode::Enter);
        }
        let now = settled(world);
        if tried == Some(now) {
            self.flying = None;
            return self.fail(format!("the ship refused {option}"));
        }
        self.flying = Some((option.to_string(), Some(now)));
        match resolve_keys(world, option) {
            Ok(keys) => {
                self.queue.extend(keys);
                Act::Idle
            }
            Err(e) => {
                self.flying = None;
                self.fail(e)
            }
        }
    }
}

fn settled(world: &World) -> usize {
    world.voyage.as_ref().map_or(0, |v| v.resolutions().len())
}

/// Open the crew screen, flip whoever is on the wrong side, confirm, and check
/// the sim took it. The screen stays open for a screenshot; `close` leaves it.
fn split_keys(world: &World, flight: &[CrewId]) -> Vec<Act> {
    let mut acts = vec![Act::Key(KeyCode::C)];
    for c in world.crew.members() {
        let want = if flight.contains(&c.id) {
            Assignment::Flight
        } else {
            Assignment::Ground
        };
        if world.crew.assignment(&c.id).unwrap_or_default() != want {
            acts.push(Act::Key(KeyCode::Space));
        }
        acts.push(Act::Key(KeyCode::Down));
    }
    acts.push(Act::Key(KeyCode::Enter));
    for c in world.crew.members() {
        let side = if flight.contains(&c.id) {
            "flight"
        } else {
            "ground"
        };
        acts.push(Act::Sim(Step::Expect(Expect::Side(
            c.id.clone(),
            side.into(),
        ))));
    }
    acts
}

/// Move the option cursor (back at the top for each new hazard) and pick.
fn resolve_keys(world: &World, option: &str) -> Result<Vec<Act>, String> {
    let hazard = world
        .voyage
        .as_ref()
        .and_then(|v| v.pending())
        .ok_or("no hazard pending")?;
    let i = hazard
        .options
        .iter()
        .position(|o| o.id == option)
        .ok_or(format!("{} has no option {option}", hazard.id))?;
    let mut acts = vec![Act::Key(KeyCode::Down); i];
    acts.push(Act::Key(KeyCode::Enter));
    Ok(acts)
}

/// Watches the world between frames for the moments `--screenshot-at` names.
pub struct Moments {
    wanted: Vec<ShotAt>,
    flight: Vec<CrewId>,
    launched: bool,
    pending: bool,
    settled: usize,
    holding: bool,
}

impl Moments {
    pub fn new(wanted: Vec<ShotAt>, world: &World) -> Self {
        let mut m = Self {
            wanted,
            flight: Vec::new(),
            launched: false,
            pending: false,
            settled: 0,
            holding: false,
        };
        m.seen(world);
        m
    }

    fn seen(&mut self, world: &World) -> [bool; 5] {
        let flight: Vec<CrewId> = world.crew.flight().map(|c| c.id.clone()).collect();
        let voyage = world.voyage.as_ref();
        let launched = voyage.is_some() && world.ship.location() != &ShipLocation::Pad;
        let pending = voyage.is_some_and(|v| v.pending().is_some());
        let settled = settled(world);
        let holding = voyage.is_some_and(|v| !v.in_transit());
        let changed = [
            flight != self.flight,
            launched && !self.launched,
            pending && !self.pending,
            settled > self.settled,
            holding && !self.holding,
        ];
        (
            self.flight,
            self.launched,
            self.pending,
            self.settled,
            self.holding,
        ) = (flight, launched, pending, settled, holding);
        changed
    }

    /// Names of the wanted moments that happened since the last call.
    pub fn check(&mut self, world: &World) -> Vec<String> {
        let changed = self.seen(world);
        let sol = world.now.sol();
        let mut hits = Vec::new();
        self.wanted.retain(|w| match w {
            ShotAt::Sol(s) if sol >= *s => {
                hits.push(format!("sol{s:03}"));
                false
            }
            _ => true,
        });
        for w in &self.wanted {
            if let ShotAt::Event(name) = w {
                let i = crate::platform::SHOT_EVENTS
                    .iter()
                    .position(|e| e == name)
                    .unwrap_or(usize::MAX);
                if changed.get(i).copied().unwrap_or(false) {
                    hits.push(name.clone());
                }
            }
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexa_sim::script::parse;

    #[test]
    fn a_split_is_keyed_in_on_the_crew_screen_and_checked() {
        let world = World::landfall();
        let acts = split_keys(&world, &[CrewId::new("cal")]);
        assert_eq!(acts[0], Act::Key(KeyCode::C));
        // Roster order: jonathan, elena, maya, cal, ...; only Cal flips.
        let spaces = acts
            .iter()
            .filter(|a| **a == Act::Key(KeyCode::Space))
            .count();
        assert_eq!(spaces, 1);
        assert!(acts.contains(&Act::Key(KeyCode::Enter)));
        assert!(acts.contains(&Act::Sim(Step::Expect(Expect::Side(
            CrewId::new("cal"),
            "flight".into()
        )))));
    }

    #[test]
    fn steps_on_the_wrong_screen_fail_with_their_line() {
        let mut d = Driver::new(parse("advance").unwrap());
        match d.act(On::Colony, &World::landfall()) {
            Act::Fail(why, 1) => assert!(why.starts_with("line 1:"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unbuilt_steps_exit_with_code_two() {
        let mut d = Driver::new(parse("land").unwrap());
        assert!(matches!(
            d.act(On::Colony, &World::landfall()),
            Act::Fail(_, 2)
        ));
    }

    #[test]
    fn moments_fire_once() {
        let mut world = World::landfall();
        let mut m = Moments::new(vec![ShotAt::Sol(1), ShotAt::Event("split".into())], &world);
        assert!(m.check(&world).is_empty());
        for _ in 0..TICKS_PER_SOL {
            world.tick();
        }
        assert_eq!(m.check(&world), vec!["sol001"]);
        assert!(m.check(&world).is_empty());
    }
}
