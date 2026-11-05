//! The crew screen: who's who, how they're holding up, and the Split.
//!
//! The player flips crew between Ground and Flight on a draft, then confirms it
//! through `World::confirm_split`, so the sim checks the Split rules. A refused
//! Split keeps the draft and shows the sim's reason.

use crate::terminal::{Ink, Terminal};
use crate::ui::{ListPicker, Outcome, Pick};
use macroquad::prelude::KeyCode;
use nexa_sim::crew::{Assignment, CrewMember, Skill, Trait};
use nexa_sim::{SimEvent, World};
use nexa_types::CrewId;
use std::collections::BTreeMap;

const LEFT: usize = 2;
const TOP: usize = 3;
/// Inside the frame, with a column of margin on each side.
const WIDTH: usize = 76;
const NAME_WIDTH: usize = 22;
const SIDE_WIDTH: usize = 9;
const HELP: &str = "[UP/DOWN] SELECT   [SPACE] GROUND/FLIGHT   [ENTER] CONFIRM   [ESC] BACK";

fn side_label(side: Assignment) -> &'static str {
    match side {
        Assignment::Ground => "GROUND",
        Assignment::Flight => "FLIGHT",
    }
}

fn flipped(side: Assignment) -> Assignment {
    match side {
        Assignment::Ground => Assignment::Flight,
        Assignment::Flight => Assignment::Ground,
    }
}

fn skill_label(skill: Skill) -> &'static str {
    match skill {
        Skill::Piloting => "PILOTING",
        Skill::Engineering => "ENGINEERING",
        Skill::Medicine => "MEDICINE",
        Skill::Negotiation => "NEGOTIATION",
        Skill::Astrogation => "ASTROGATION",
        Skill::Sensors => "SENSORS",
        Skill::Rigging => "RIGGING",
    }
}

fn trait_label(t: Trait) -> &'static str {
    match t {
        Trait::FathersVigilance => "FATHER'S VIGILANCE",
        Trait::HippocraticAnchor => "HIPPOCRATIC ANCHOR",
        Trait::LivingCompass => "LIVING COMPASS",
        Trait::CombatFatigue => "COMBAT FATIGUE",
        Trait::ClinicalDetachment => "CLINICAL DETACHMENT",
        Trait::FlightRisk => "FLIGHT RISK",
    }
}

/// Basis points as a whole percent; floats stay out of it.
fn percent(bp: u16) -> u16 {
    bp / 100
}

pub struct CrewScreen {
    list: ListPicker,
    /// The Split being edited; starts as the confirmed one.
    draft: BTreeMap<CrewId, Assignment>,
    /// The last confirmation, or why the sim refused it.
    message: Option<String>,
}

impl CrewScreen {
    pub fn new(world: &World) -> Self {
        let crew = &world.crew;
        let draft = crew
            .members()
            .iter()
            .map(|c| (c.id.clone(), crew.assignment(&c.id).unwrap_or_default()))
            .collect();
        Self {
            list: ListPicker::default(),
            draft,
            message: None,
        }
    }

    fn drafted(&self, id: &CrewId) -> Assignment {
        self.draft.get(id).copied().unwrap_or_default()
    }

    /// Confirmed events go to `events` for the base log.
    pub fn handle(
        &mut self,
        key: KeyCode,
        world: &mut World,
        events: &mut Vec<SimEvent>,
    ) -> Outcome {
        let members = world.crew.members();
        match key {
            KeyCode::Space | KeyCode::Left | KeyCode::Right => {
                if let Some(c) = members.get(self.list.cursor()) {
                    let side = flipped(self.drafted(&c.id));
                    self.draft.insert(c.id.clone(), side);
                    self.message = None;
                }
            }
            KeyCode::Enter | KeyCode::KpEnter => match world.confirm_split(&self.draft) {
                Ok(confirmed) => {
                    // Count what the sim confirmed, not the draft.
                    let flying = confirmed
                        .iter()
                        .find_map(|e| match e {
                            SimEvent::SplitConfirmed { flight } => Some(flight.len()),
                            _ => None,
                        })
                        .unwrap_or(0);
                    let staying = world.crew.members().len() - flying;
                    self.message = Some(format!("SPLIT CONFIRMED: {flying} FLY, {staying} STAY"));
                    events.extend(confirmed);
                }
                Err(e) => self.message = Some(e.to_string().to_uppercase()),
            },
            // Backspace must not close the screen by accident.
            KeyCode::Backspace => {}
            _ => {
                if self.list.handle(key, members.len()) == Pick::Back {
                    return Outcome::Close;
                }
            }
        }
        Outcome::Stay
    }

    /// Draw everything between the title row and `help_row`, which gets the key help.
    pub fn draw(&self, t: &mut Terminal, world: &World, help_row: usize) {
        let crew = &world.crew;
        let members = crew.members();
        t.put_str(LEFT, TOP, "CREW MANIFEST // THE SPLIT", Ink::Bright);
        let flying = crew
            .members()
            .iter()
            .filter(|c| crew.assignment(&c.id) == Some(Assignment::Flight))
            .count();
        let split = format!("{flying} FLY  {} STAY", members.len() - flying);
        t.put_str(LEFT + WIDTH - split.len(), TOP, &split, Ink::Amber);

        let header = format!(
            "  {:<NAME_WIDTH$}{:<SIDE_WIDTH$}{:<16}{}",
            "NAME", "SIDE", "STRESS", "MORALE"
        );
        t.put_str(LEFT, TOP + 2, &header, Ink::Dim);
        let mut changed = false;
        for (i, c) in members.iter().enumerate() {
            let side = self.drafted(&c.id);
            // A star marks a side the player has changed but not confirmed.
            let unconfirmed = crew.assignment(&c.id) != Some(side);
            changed |= unconfirmed;
            let star = if unconfirmed { "*" } else { "" };
            let side = format!("{}{star}", side_label(side));
            let (stress, morale) = match crew.vitals(&c.id) {
                Some(v) => (
                    format!(
                        "{:>3}% {}",
                        percent(v.stress_bp),
                        v.level.label().to_uppercase()
                    ),
                    format!("{:>3}%", percent(v.morale_bp)),
                ),
                None => ("--".into(), "--".into()),
            };
            let (mark, ink) = if i == self.list.cursor() {
                ("> ", Ink::Bright)
            } else {
                ("  ", Ink::Amber)
            };
            let name = c.name.to_uppercase();
            let row = format!("{mark}{name:<NAME_WIDTH$}{side:<SIDE_WIDTH$}{stress:<16}{morale}");
            t.put_str(LEFT, TOP + 3 + i, &row, ink);
        }
        let mut row = TOP + 4 + members.len();
        if changed {
            t.put_str(LEFT, row, "* NOT YET CONFIRMED", Ink::Dim);
        }
        row += 2;
        if let Some(c) = members.get(self.list.cursor()) {
            draw_details(t, c, row);
        }

        if let Some(message) = &self.message {
            t.put_str(LEFT, help_row - 2, message, Ink::Bright);
        }
        t.put_str(LEFT, help_row, HELP, Ink::Dim);
    }
}

fn draw_details(t: &mut Terminal, c: &CrewMember, row: usize) {
    let age = if c.adult { "ADULT" } else { "CHILD" };
    let head = format!(
        "{}  {age}  BUNK {}",
        c.name.to_uppercase(),
        c.bunk.as_str().to_uppercase()
    );
    t.put_str(LEFT, row, &head, Ink::Bright);
    let skills: Vec<String> = c
        .skills
        .iter()
        .filter(|(_, &rating)| rating > 0)
        .map(|(&skill, rating)| format!("{} {rating}", skill_label(skill)))
        .collect();
    t.put_str(LEFT, row + 1, "SKILLS", Ink::Dim);
    t.put_str(LEFT + 8, row + 1, &skills.join("  "), Ink::Amber);
    t.put_str(LEFT, row + 2, "TRAIT", Ink::Dim);
    t.put_str(LEFT + 8, row + 2, trait_label(c.trait_), Ink::Amber);
}

#[cfg(test)]
mod tests;
