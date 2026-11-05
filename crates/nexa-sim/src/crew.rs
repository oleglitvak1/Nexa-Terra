//! The crew: who they are, what they're rated at, where they sleep, and the
//! Split rules for who flies and who stays on the ground.
//!
//! [`Crew`] is the part of the world that holds them, the confirmed Split,
//! and each member's stress and morale, stepped once an hour.

use crate::colony::{Colony, Module, ModuleKind};
use crate::machines::MachineState;
use crate::SimEvent;
use nexa_types::{CrewId, ModuleId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Shipped crew and tuning, compiled in so loading needs no file I/O.
pub const CREW: &str = include_str!("../../../content/crew.ron");

/// The vertical slice has exactly this many crew.
pub const CREW_SIZE: usize = 6;

/// Skills are rated 0..=MAX_SKILL.
pub const MAX_SKILL: u8 = 5;

/// Stress, morale and their rates are basis points, 0..=MAX_BP.
pub const MAX_BP: u16 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Skill {
    Piloting,
    Engineering,
    Medicine,
    Negotiation,
    Astrogation,
    Sensors,
    Rigging,
}

/// Each crew member's psychological dynamic (docs/game-design.md section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Trait {
    FathersVigilance,
    HippocraticAnchor,
    LivingCompass,
    CombatFatigue,
    ClinicalDetachment,
    FlightRisk,
}

/// Which side of the Split a crew member is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Assignment {
    #[default]
    Ground,
    Flight,
}

/// One crew member's fixed data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrewMember {
    pub id: CrewId,
    pub name: String,
    /// Children need an adult on their side of the Split.
    pub adult: bool,
    /// Rated 0..=MAX_SKILL; a missing skill is 0.
    #[serde(default)]
    pub skills: BTreeMap<Skill, u8>,
    /// Written `dynamic` in content, after the design doc's "Psychological Dynamic".
    #[serde(rename = "dynamic")]
    pub trait_: Trait,
    /// The hab module this crew member sleeps in.
    pub bunk: ModuleId,
}

impl CrewMember {
    pub fn skill(&self, skill: Skill) -> u8 {
        self.skills.get(&skill).copied().unwrap_or(0)
    }
}

/// Stress and morale tuning; every `_bp` field is basis points (per hour for rates).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrewTuning {
    /// Piloting rating needed to fly the freighter.
    pub pilot_min: u8,
    /// Stress gained per hour while one's bunk is cold or failed.
    pub stress_rise_cold_bp: u16,
    /// Stress shed per hour while one's bunk is nominal.
    pub stress_recover_bp: u16,
    /// Father's Vigilance: extra stress per hour while Maya's bunk is cold.
    pub vigilance_extra_bp: u16,
    pub calm_bp: u16,
    pub strained_bp: u16,
    pub breaking_bp: u16,
    pub morale_fall_bp: u16,
    pub morale_recover_bp: u16,
}

impl CrewTuning {
    fn check(&self) -> Result<(), CrewError> {
        // Zero would make everyone a pilot; above MAX_SKILL, nobody.
        if !(1..=MAX_SKILL).contains(&self.pilot_min) {
            return Err(CrewError::PilotMinOutOfRange(self.pilot_min));
        }
        let rates = [
            ("stress_rise_cold_bp", self.stress_rise_cold_bp),
            ("stress_recover_bp", self.stress_recover_bp),
            ("vigilance_extra_bp", self.vigilance_extra_bp),
            ("morale_fall_bp", self.morale_fall_bp),
            ("morale_recover_bp", self.morale_recover_bp),
        ];
        if let Some(&(field, bp)) = rates.iter().find(|(_, bp)| *bp > MAX_BP) {
            return Err(CrewError::RateOutOfRange { field, bp });
        }
        // Morale recovers below calm, so calm must be above zero.
        if self.calm_bp == 0
            || self.calm_bp >= self.strained_bp
            || self.strained_bp >= self.breaking_bp
            || self.breaking_bp > MAX_BP
        {
            return Err(CrewError::ThresholdsOutOfOrder);
        }
        Ok(())
    }
}

/// The crew content file, checked against the colony the crew lives in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Roster {
    pub tuning: CrewTuning,
    pub crew: Vec<CrewMember>,
}

/// Why a crew content file was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrewError {
    Parse(String),
    WrongCrewSize(usize),
    DuplicateId(CrewId),
    SkillTooHigh {
        crew: CrewId,
        skill: Skill,
        rating: u8,
    },
    UnknownBunk {
        crew: CrewId,
        bunk: ModuleId,
    },
    BunkNotHab {
        crew: CrewId,
        bunk: ModuleId,
    },
    /// Nobody could fly the freighter.
    NoQualifiedPilot,
    /// Must hold 0 < calm < strained < breaking <= MAX_BP.
    ThresholdsOutOfOrder,
    /// `pilot_min` must be 1..=MAX_SKILL.
    PilotMinOutOfRange(u8),
    RateOutOfRange {
        field: &'static str,
        bp: u16,
    },
    /// A saved Split or set of vitals doesn't list exactly the crew.
    CrewMismatch,
    /// A saved Split breaks the Split rules.
    BadSplit(SplitError),
    /// A crew member's saved stress or morale is missing, out of range, or at
    /// a level its stress can't be at.
    BadVitals(CrewId),
}

impl std::fmt::Display for CrewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CrewError::Parse(e) => write!(f, "crew content doesn't parse: {e}"),
            CrewError::WrongCrewSize(n) => write!(f, "{n} crew listed, expected {CREW_SIZE}"),
            CrewError::DuplicateId(id) => write!(f, "crew id {id} is used twice"),
            CrewError::SkillTooHigh {
                crew,
                skill,
                rating,
            } => write!(f, "{crew}'s {skill:?} is {rating}, above {MAX_SKILL}"),
            CrewError::UnknownBunk { crew, bunk } => {
                write!(f, "{crew}'s bunk {bunk} is not a colony module")
            }
            CrewError::BunkNotHab { crew, bunk } => {
                write!(f, "{crew}'s bunk {bunk} is not a hab")
            }
            CrewError::NoQualifiedPilot => write!(f, "no crew member can fly the freighter"),
            CrewError::ThresholdsOutOfOrder => write!(
                f,
                "stress thresholds must be 0 < calm < strained < breaking <= {MAX_BP}"
            ),
            CrewError::PilotMinOutOfRange(n) => {
                write!(f, "pilot_min is {n}, expected 1..={MAX_SKILL}")
            }
            CrewError::RateOutOfRange { field, bp } => {
                write!(f, "{field} is {bp}, above {MAX_BP}")
            }
            CrewError::CrewMismatch => {
                write!(f, "the saved Split or vitals don't list exactly the crew")
            }
            CrewError::BadSplit(e) => write!(f, "the saved Split is refused: {e}"),
            CrewError::BadVitals(id) => write!(f, "{id}'s saved stress or morale is invalid"),
        }
    }
}

impl std::error::Error for CrewError {}

fn is_hab(module: &Module) -> bool {
    module.def().kind == ModuleKind::Hab
}

impl Roster {
    /// Parse and check a crew content file against `colony`.
    pub fn from_ron(text: &str, colony: &Colony) -> Result<Self, CrewError> {
        let roster: Self = ron::from_str(text).map_err(|e| CrewError::Parse(e.to_string()))?;
        roster.check(colony)?;
        Ok(roster)
    }

    pub fn check(&self, colony: &Colony) -> Result<(), CrewError> {
        self.tuning.check()?;
        if self.crew.len() != CREW_SIZE {
            return Err(CrewError::WrongCrewSize(self.crew.len()));
        }
        for (i, c) in self.crew.iter().enumerate() {
            if self.crew[..i].iter().any(|other| other.id == c.id) {
                return Err(CrewError::DuplicateId(c.id.clone()));
            }
            if let Some((&skill, &rating)) = c.skills.iter().find(|(_, &r)| r > MAX_SKILL) {
                return Err(CrewError::SkillTooHigh {
                    crew: c.id.clone(),
                    skill,
                    rating,
                });
            }
            let is_bunk = colony.module(&c.bunk).map(is_hab);
            if is_bunk != Some(true) {
                let (crew, bunk) = (c.id.clone(), c.bunk.clone());
                return Err(match is_bunk {
                    None => CrewError::UnknownBunk { crew, bunk },
                    Some(_) => CrewError::BunkNotHab { crew, bunk },
                });
            }
        }
        if !self
            .crew
            .iter()
            .any(|c| c.skill(Skill::Piloting) >= self.tuning.pilot_min)
        {
            return Err(CrewError::NoQualifiedPilot);
        }
        Ok(())
    }
}

impl Roster {
    /// The shipped crew, checked against the colony they will live in
    /// (e.g. a save's colony, not necessarily the landing site).
    pub fn shipped(colony: &Colony) -> Result<Self, CrewError> {
        Self::from_ron(CREW, colony)
    }
}

impl Default for Roster {
    /// The shipped crew, bunked on the Ash Plain landing site.
    fn default() -> Self {
        // Both files are checked by tests, so failing here is a build bug.
        let colony = Colony::from_ron(crate::colony::LANDING_ASH_PLAIN)
            .expect("landing site content is valid");
        Self::shipped(&colony).expect("crew content is valid")
    }
}

/// Why a proposed Split can't launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitError {
    /// Nobody on the Flight side has Piloting >= `pilot_min`.
    NoQualifiedPilot,
    /// A child's side of the Split has no adult on it.
    MayaWithoutAdult { child: CrewId },
    /// An assignment names someone who isn't in the crew.
    UnknownCrew(CrewId),
    /// The Mullein is away: who's aboard can't change until she lands.
    ShipAway,
}

impl std::fmt::Display for SplitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SplitError::NoQualifiedPilot => write!(f, "no qualified pilot is flying"),
            SplitError::MayaWithoutAdult { child } => {
                write!(f, "{child} has no adult on their side")
            }
            SplitError::UnknownCrew(id) => write!(f, "no crew member named {id}"),
            SplitError::ShipAway => write!(f, "the mullein is away; the split holds"),
        }
    }
}

impl std::error::Error for SplitError {}

/// Check a proposed Split. Crew missing from `assignments` stay on the Ground,
/// but every id in `assignments` must be a crew member.
/// Children are the crew with `adult: false` (only Maya in the shipped roster).
pub fn check_split(
    crew: &[CrewMember],
    tuning: &CrewTuning,
    assignments: &BTreeMap<CrewId, Assignment>,
) -> Result<(), SplitError> {
    if let Some(unknown) = assignments
        .keys()
        .find(|id| !crew.iter().any(|c| &c.id == *id))
    {
        return Err(SplitError::UnknownCrew(unknown.clone()));
    }
    let side = |c: &CrewMember| assignments.get(&c.id).copied().unwrap_or_default();
    if !crew
        .iter()
        .any(|c| side(c) == Assignment::Flight && c.skill(Skill::Piloting) >= tuning.pilot_min)
    {
        return Err(SplitError::NoQualifiedPilot);
    }
    for child in crew.iter().filter(|c| !c.adult) {
        if !crew.iter().any(|c| c.adult && side(c) == side(child)) {
            return Err(SplitError::MayaWithoutAdult {
                child: child.id.clone(),
            });
        }
    }
    Ok(())
}

/// How hard stress is pressing on someone. Levels have no consequences yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StressLevel {
    Calm,
    Strained,
    Breaking,
}

impl StressLevel {
    pub fn label(self) -> &'static str {
        match self {
            StressLevel::Calm => "calm",
            StressLevel::Strained => "strained",
            StressLevel::Breaking => "breaking",
        }
    }

    /// The level after stress moves to `stress_bp`. A level is reached at its
    /// threshold but only left below the threshold under it, so a bunk
    /// flickering in and out of Freeze doesn't announce a level every hour.
    fn next(self, stress_bp: u16, t: &CrewTuning) -> Self {
        use StressLevel::*;
        if stress_bp >= t.breaking_bp {
            Breaking
        } else if stress_bp < t.calm_bp {
            Calm
        } else if stress_bp >= t.strained_bp {
            // Between strained and breaking: Breaking holds, Calm rises.
            if self == Breaking {
                Breaking
            } else {
                Strained
            }
        } else if self == Breaking {
            // Between calm and strained: Calm holds, Breaking eases.
            Strained
        } else {
            self
        }
    }
}

/// One crew member's changing state; both values are basis points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vitals {
    pub stress_bp: u16,
    pub morale_bp: u16,
    /// The last level announced; see [`StressLevel::next`].
    pub level: StressLevel,
}

impl Vitals {
    const LANDFALL: Self = Self {
        stress_bp: 0,
        morale_bp: MAX_BP,
        level: StressLevel::Calm,
    };
}

/// A bunk counts as cold while its hab is in Freeze or has failed.
fn is_cold(state: Option<MachineState>) -> bool {
    matches!(
        state,
        Some(MachineState::Freeze | MachineState::Offline | MachineState::Burnout)
    )
}

/// The crew aboard a world: their content and the side each is on.
/// Fields are private so a `Crew` is only built checked (or loaded and then
/// checked by `World`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crew {
    roster: Roster,
    /// One entry per crew member; all Ground until a Split is confirmed.
    assignments: BTreeMap<CrewId, Assignment>,
    /// One entry per crew member.
    vitals: BTreeMap<CrewId, Vitals>,
}

impl Crew {
    /// Everyone starts on the Ground, calm, with full morale.
    pub fn new(roster: Roster) -> Self {
        let ids = || roster.crew.iter().map(|c| c.id.clone());
        Self {
            assignments: ids().map(|id| (id, Assignment::Ground)).collect(),
            vitals: ids().map(|id| (id, Vitals::LANDFALL)).collect(),
            roster,
        }
    }

    pub fn roster(&self) -> &Roster {
        &self.roster
    }

    pub fn members(&self) -> &[CrewMember] {
        &self.roster.crew
    }

    pub fn assignment(&self, id: &CrewId) -> Option<Assignment> {
        self.assignments.get(id).copied()
    }

    pub fn vitals(&self, id: &CrewId) -> Option<&Vitals> {
        self.vitals.get(id)
    }

    /// One hour of stress and morale, given the state of each hab's machine.
    /// Ground crew feel their own bunk; Father's Vigilance also feels any
    /// Ground child's bunk going cold, wherever the holder is. Morale follows
    /// stress.
    pub fn step(
        &mut self,
        bunk_state: impl Fn(&ModuleId) -> Option<MachineState>,
    ) -> Vec<SimEvent> {
        let t = self.roster.tuning;
        let on_ground = |c: &CrewMember| self.assignments.get(&c.id) == Some(&Assignment::Ground);
        // A flying child isn't in their bunk, so its cold exposes no one.
        let child_cold = self
            .roster
            .crew
            .iter()
            .any(|c| !c.adult && on_ground(c) && is_cold(bunk_state(&c.bunk)));
        let mut events = Vec::new();
        for c in &self.roster.crew {
            let mut change = 0i32;
            if self.assignments.get(&c.id) == Some(&Assignment::Ground) {
                let state = bunk_state(&c.bunk);
                if is_cold(state) {
                    change += i32::from(t.stress_rise_cold_bp);
                } else if state == Some(MachineState::Nominal) {
                    change -= i32::from(t.stress_recover_bp);
                }
            }
            if c.trait_ == Trait::FathersVigilance && child_cold {
                change += i32::from(t.vigilance_extra_bp);
            }
            // Every member has vitals once checked; skip rather than panic.
            let Some(v) = self.vitals.get_mut(&c.id) else {
                continue;
            };
            let stress = (i32::from(v.stress_bp) + change).clamp(0, i32::from(MAX_BP));
            v.stress_bp = stress as u16;
            // Same edges as the levels: strained from its threshold, calm below its own.
            if v.stress_bp >= t.strained_bp {
                v.morale_bp = v.morale_bp.saturating_sub(t.morale_fall_bp);
            } else if v.stress_bp < t.calm_bp {
                v.morale_bp = (v.morale_bp + t.morale_recover_bp).min(MAX_BP);
            }
            let level = v.level.next(v.stress_bp, &t);
            if level != v.level {
                v.level = level;
                events.push(SimEvent::CrewStressChanged {
                    crew: c.id.clone(),
                    level,
                });
            }
        }
        events
    }

    /// Check the Split and, if it passes, make it the crew's assignments.
    /// Crew left out of `assignments` go to the Ground. Returns who flies,
    /// in roster order.
    pub fn confirm_split(
        &mut self,
        assignments: &BTreeMap<CrewId, Assignment>,
    ) -> Result<Vec<CrewId>, SplitError> {
        check_split(&self.roster.crew, &self.roster.tuning, assignments)?;
        for (id, side) in &mut self.assignments {
            *side = assignments.get(id).copied().unwrap_or_default();
        }
        Ok(self
            .roster
            .crew
            .iter()
            .filter(|c| self.assignments[&c.id] == Assignment::Flight)
            .map(|c| c.id.clone())
            .collect())
    }

    /// What a loaded save must satisfy: valid content for `colony`, one side
    /// per crew member, either nobody flying or a Split that passes, and
    /// stress and morale in range at a level that fits.
    pub fn check(&self, colony: &Colony) -> Result<(), CrewError> {
        self.roster.check(colony)?;
        if self.assignments.len() != self.roster.crew.len()
            || !self
                .members()
                .iter()
                .all(|c| self.assignments.contains_key(&c.id))
        {
            return Err(CrewError::CrewMismatch);
        }
        let t = &self.roster.tuning;
        let vitals_ok = |v: &Vitals| {
            v.stress_bp <= MAX_BP
                && v.morale_bp <= MAX_BP
                && v.level.next(v.stress_bp, t) == v.level
        };
        for c in self.members() {
            if !self.vitals.get(&c.id).is_some_and(vitals_ok) {
                return Err(CrewError::BadVitals(c.id.clone()));
            }
        }
        // Every member has an entry, so a longer map holds someone else too.
        if self.vitals.len() != self.roster.crew.len() {
            return Err(CrewError::CrewMismatch);
        }
        let anyone_flying = self.assignments.values().any(|&a| a == Assignment::Flight);
        if anyone_flying {
            check_split(&self.roster.crew, &self.roster.tuning, &self.assignments)
                .map_err(CrewError::BadSplit)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colony::LANDING_ASH_PLAIN;
    use crate::machines::MachineState;
    use crate::SimEvent;

    fn ash_plain() -> Colony {
        Colony::from_ron(LANDING_ASH_PLAIN).unwrap()
    }

    fn shipped() -> Roster {
        Roster::from_ron(CREW, &ash_plain()).unwrap()
    }

    fn id(s: &str) -> CrewId {
        CrewId::new(s)
    }

    fn check(roster: &Roster) -> Result<(), CrewError> {
        roster.check(&ash_plain())
    }

    #[test]
    fn shipped_crew_loads_and_validates() {
        let roster = shipped();
        assert_eq!(roster, Roster::default());
        let ids: Vec<_> = roster.crew.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["jonathan", "elena", "maya", "cal", "oron", "dax"]);
        let minors: Vec<_> = roster.crew.iter().filter(|c| !c.adult).collect();
        assert_eq!(minors.len(), 1);
        assert_eq!(minors[0].id, id("maya"));
        let pilots: Vec<_> = roster
            .crew
            .iter()
            .filter(|c| c.skill(Skill::Piloting) >= roster.tuning.pilot_min)
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(pilots, ["cal"]);
        // The Winters share a bunk.
        let bunk = |who: &str| &roster.crew.iter().find(|c| c.id == id(who)).unwrap().bunk;
        assert_eq!(bunk("jonathan"), bunk("maya"));
        assert_eq!(bunk("elena"), bunk("maya"));
    }

    #[test]
    fn bad_ron_is_a_parse_error() {
        assert!(matches!(
            Roster::from_ron("(crew: [])", &ash_plain()),
            Err(CrewError::Parse(_))
        ));
    }

    #[test]
    fn crew_must_number_six() {
        let mut r = shipped();
        r.crew.pop();
        assert_eq!(check(&r), Err(CrewError::WrongCrewSize(5)));
    }

    #[test]
    fn crew_ids_must_be_unique() {
        let mut r = shipped();
        r.crew[5].id = id("cal");
        assert_eq!(check(&r), Err(CrewError::DuplicateId(id("cal"))));
    }

    #[test]
    fn skills_top_out_at_five() {
        let mut r = shipped();
        r.crew[1].skills.insert(Skill::Medicine, 6);
        assert_eq!(
            check(&r),
            Err(CrewError::SkillTooHigh {
                crew: id("elena"),
                skill: Skill::Medicine,
                rating: 6
            })
        );
    }

    #[test]
    fn bunks_must_be_colony_habs() {
        let mut r = shipped();
        r.crew[4].bunk = ModuleId::new("ghost");
        assert_eq!(
            check(&r),
            Err(CrewError::UnknownBunk {
                crew: id("oron"),
                bunk: ModuleId::new("ghost")
            })
        );
        // Has a machine, but isn't a hab.
        r.crew[4].bunk = ModuleId::new("smelter_01");
        assert_eq!(
            check(&r),
            Err(CrewError::BunkNotHab {
                crew: id("oron"),
                bunk: ModuleId::new("smelter_01")
            })
        );
    }

    #[test]
    fn someone_must_be_able_to_fly() {
        let mut r = shipped();
        r.crew[3].skills.insert(Skill::Piloting, 3);
        assert_eq!(check(&r), Err(CrewError::NoQualifiedPilot));
    }

    #[test]
    fn pilot_min_stays_within_skill_ratings() {
        for bad in [0, MAX_SKILL + 1] {
            let mut r = shipped();
            r.tuning.pilot_min = bad;
            assert_eq!(check(&r), Err(CrewError::PilotMinOutOfRange(bad)));
        }
    }

    #[test]
    fn misspelled_fields_are_parse_errors() {
        let typo = CREW.replacen("skills:", "skils:", 1);
        assert!(matches!(
            Roster::from_ron(&typo, &ash_plain()),
            Err(CrewError::Parse(_))
        ));
    }

    #[test]
    fn shipped_crew_checks_against_the_given_colony() {
        let mut def = crate::colony::ColonyDef::from_ron(LANDING_ASH_PLAIN).unwrap();
        def.modules.retain(|m| m.id.as_str() != "hab_02");
        def.layout = def.layout.iter().map(|row| row.replace('h', ".")).collect();
        let colony = Colony::from_def(def).unwrap();
        assert!(matches!(
            Roster::shipped(&colony),
            Err(CrewError::UnknownBunk { .. })
        ));
    }

    #[test]
    fn stress_thresholds_must_be_ordered() {
        let base = shipped();
        let cases: [fn(&mut CrewTuning); 4] = [
            |t| t.calm_bp = 0,
            |t| t.strained_bp = t.calm_bp,
            |t| t.breaking_bp = t.strained_bp,
            |t| t.breaking_bp = MAX_BP + 1,
        ];
        for bend in cases {
            let mut r = base.clone();
            bend(&mut r.tuning);
            assert_eq!(check(&r), Err(CrewError::ThresholdsOutOfOrder));
        }
    }

    #[test]
    fn rates_stay_within_basis_points() {
        let mut r = shipped();
        r.tuning.morale_fall_bp = MAX_BP + 1;
        assert_eq!(
            check(&r),
            Err(CrewError::RateOutOfRange {
                field: "morale_fall_bp",
                bp: MAX_BP + 1
            })
        );
    }

    fn split(flying: &[&str]) -> Result<(), SplitError> {
        let r = shipped();
        let assignments: BTreeMap<_, _> = flying
            .iter()
            .map(|who| (id(who), Assignment::Flight))
            .collect();
        check_split(&r.crew, &r.tuning, &assignments)
    }

    #[test]
    fn the_usual_split_passes() {
        assert_eq!(split(&["cal", "dax", "oron"]), Ok(()));
    }

    #[test]
    fn a_split_needs_a_qualified_pilot_aloft() {
        assert_eq!(split(&[]), Err(SplitError::NoQualifiedPilot));
        // Dax and Jonathan can handle a stick, but not well enough.
        assert_eq!(
            split(&["dax", "jonathan"]),
            Err(SplitError::NoQualifiedPilot)
        );
    }

    #[test]
    fn maya_is_never_left_without_an_adult() {
        // Maya flies with Cal as her only adult.
        assert_eq!(split(&["cal", "maya"]), Ok(()));
        // Maya on the ground alone.
        assert_eq!(
            split(&["cal", "jonathan", "elena", "oron", "dax"]),
            Err(SplitError::MayaWithoutAdult { child: id("maya") })
        );
    }

    #[test]
    fn everyone_may_fly() {
        assert_eq!(
            split(&["jonathan", "elena", "maya", "cal", "oron", "dax"]),
            Ok(())
        );
    }

    #[test]
    fn oron_counts_as_mayas_adult() {
        assert_eq!(split(&["cal", "jonathan", "elena", "dax"]), Ok(()));
    }

    #[test]
    fn unknown_crew_in_a_split_is_rejected() {
        assert_eq!(
            split(&["cal", "Maya"]),
            Err(SplitError::UnknownCrew(id("Maya")))
        );
    }

    #[test]
    fn missing_assignments_mean_ground() {
        let r = shipped();
        let mut assignments = BTreeMap::new();
        assignments.insert(id("cal"), Assignment::Flight);
        assignments.insert(id("elena"), Assignment::Ground);
        assert_eq!(check_split(&r.crew, &r.tuning, &assignments), Ok(()));
    }

    // Stress and morale. Shipped tuning: rise 160, recover 55, vigilance 150,
    // calm 2500, strained 5000, breaking 8000, morale fall 60, recover 25.

    /// Steps `crew` one hour with each hab in the given state.
    fn step_with(crew: &mut Crew, habs: &[(&str, MachineState)]) -> Vec<SimEvent> {
        crew.step(|bunk| {
            habs.iter()
                .find(|(id, _)| bunk.as_str() == *id)
                .map(|&(_, state)| state)
        })
    }

    fn stress(crew: &Crew, who: &str) -> u16 {
        crew.vitals(&id(who)).unwrap().stress_bp
    }

    fn morale(crew: &Crew, who: &str) -> u16 {
        crew.vitals(&id(who)).unwrap().morale_bp
    }

    fn set_stress(crew: &mut Crew, who: &str, bp: u16) {
        crew.vitals.get_mut(&id(who)).unwrap().stress_bp = bp;
    }

    fn levels(events: &[SimEvent]) -> Vec<(&str, StressLevel)> {
        events
            .iter()
            .filter_map(|e| match e {
                SimEvent::CrewStressChanged { crew, level } => Some((crew.as_str(), *level)),
                _ => None,
            })
            .collect()
    }

    const COLD_01: [(&str, MachineState); 2] = [
        ("hab_01", MachineState::Freeze),
        ("hab_02", MachineState::Nominal),
    ];

    #[test]
    fn the_crew_lands_calm_and_in_good_spirits() {
        let crew = Crew::new(shipped());
        for c in crew.members() {
            let v = crew.vitals(&c.id).unwrap();
            assert_eq!(
                (v.stress_bp, v.morale_bp, v.level),
                (0, MAX_BP, StressLevel::Calm)
            );
        }
    }

    #[test]
    fn a_cold_bunk_raises_stress_each_hour() {
        let mut crew = Crew::new(shipped());
        step_with(&mut crew, &COLD_01);
        step_with(&mut crew, &COLD_01);
        assert_eq!(stress(&crew, "elena"), 320);
        assert_eq!(stress(&crew, "maya"), 320);
        assert_eq!(stress(&crew, "cal"), 0, "hab_02 is nominal");
    }

    #[test]
    fn failed_bunks_are_cold_and_overheat_is_neither() {
        for (state, expected) in [
            (MachineState::Offline, 1160),
            (MachineState::Burnout, 1160),
            (MachineState::Overheat, 1000),
            (MachineState::Nominal, 945),
        ] {
            let mut crew = Crew::new(shipped());
            set_stress(&mut crew, "cal", 1000);
            step_with(&mut crew, &[("hab_02", state)]);
            assert_eq!(stress(&crew, "cal"), expected, "{state:?}");
        }
    }

    #[test]
    fn stress_stays_within_its_range() {
        let mut crew = Crew::new(shipped());
        step_with(&mut crew, &[("hab_02", MachineState::Nominal)]);
        assert_eq!(stress(&crew, "cal"), 0);
        set_stress(&mut crew, "cal", MAX_BP - 10);
        step_with(&mut crew, &[("hab_02", MachineState::Freeze)]);
        assert_eq!(stress(&crew, "cal"), MAX_BP);
    }

    #[test]
    fn jonathan_feels_mayas_cold_bunk_wherever_he_is() {
        let mut crew = Crew::new(shipped());
        step_with(&mut crew, &COLD_01);
        assert_eq!(stress(&crew, "jonathan"), 160 + 150);

        let mut crew = Crew::new(shipped());
        let flight = [
            (id("cal"), Assignment::Flight),
            (id("jonathan"), Assignment::Flight),
        ];
        crew.confirm_split(&flight.into_iter().collect()).unwrap();
        step_with(&mut crew, &COLD_01);
        assert_eq!(stress(&crew, "jonathan"), 150, "only vigilance aboard");
        assert_eq!(stress(&crew, "elena"), 160);
    }

    #[test]
    fn jonathan_ignores_mayas_empty_bunk_while_she_flies() {
        let mut crew = Crew::new(shipped());
        let flight = [
            (id("cal"), Assignment::Flight),
            (id("jonathan"), Assignment::Flight),
            (id("maya"), Assignment::Flight),
        ];
        crew.confirm_split(&flight.into_iter().collect()).unwrap();
        step_with(&mut crew, &COLD_01);
        assert_eq!(stress(&crew, "jonathan"), 0);
        assert_eq!(stress(&crew, "maya"), 0);
        assert_eq!(stress(&crew, "elena"), 160);
    }

    #[test]
    fn flight_crew_are_not_stressed_by_their_bunks() {
        let mut crew = Crew::new(shipped());
        let flight = [(id("cal"), Assignment::Flight)];
        crew.confirm_split(&flight.into_iter().collect()).unwrap();
        step_with(&mut crew, &[("hab_02", MachineState::Offline)]);
        assert_eq!(stress(&crew, "cal"), 0);
        assert_eq!(stress(&crew, "dax"), 160);
    }

    #[test]
    fn morale_falls_when_strained_and_recovers_when_calm() {
        let mut crew = Crew::new(shipped());
        crew.vitals.get_mut(&id("cal")).unwrap().morale_bp = 5000;
        // Morale falls from the strained threshold itself, like the level.
        for (stress_bp, expected) in [(6000, 4940), (5000, 4880), (4000, 4880), (2000, 4905)] {
            set_stress(&mut crew, "cal", stress_bp);
            // Overheat holds stress where it is.
            step_with(&mut crew, &[("hab_02", MachineState::Overheat)]);
            assert_eq!(morale(&crew, "cal"), expected, "stress {stress_bp}");
        }
        assert_eq!(morale(&crew, "dax"), MAX_BP, "full morale stays full");
    }

    #[test]
    fn crossing_a_stress_level_is_announced_once() {
        let mut crew = Crew::new(shipped());
        let cold = [("hab_02", MachineState::Freeze)];
        let warm = [("hab_02", MachineState::Nominal)];
        set_stress(&mut crew, "cal", 4900);
        let events = step_with(&mut crew, &cold);
        assert_eq!(levels(&events), [("cal", StressLevel::Strained)]);
        // A bunk flickering cold and warm moves stress back and forth across
        // the line; that doesn't re-announce it.
        for _ in 0..5 {
            set_stress(&mut crew, "cal", 5010);
            assert_eq!(levels(&step_with(&mut crew, &warm)), []);
            assert_eq!(stress(&crew, "cal"), 4955);
            assert_eq!(levels(&step_with(&mut crew, &cold)), []);
        }
        // Strained eases back to Calm only below the calm threshold.
        set_stress(&mut crew, "cal", 2555);
        assert_eq!(levels(&step_with(&mut crew, &warm)), []);
        assert_eq!(
            levels(&step_with(&mut crew, &warm)),
            [("cal", StressLevel::Calm)]
        );
        set_stress(&mut crew, "cal", 7900);
        assert_eq!(
            levels(&step_with(&mut crew, &cold)),
            [("cal", StressLevel::Breaking)]
        );
        // Breaking eases to Strained only below the strained threshold.
        set_stress(&mut crew, "cal", 5100);
        assert_eq!(levels(&step_with(&mut crew, &warm)), []);
        assert_eq!(
            levels(&step_with(&mut crew, &warm)),
            [("cal", StressLevel::Strained)]
        );
    }
}
