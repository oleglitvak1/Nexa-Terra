//! The crew: who they are, what they're rated at, where they sleep, and the
//! Split rules for who flies and who stays on the ground.
//!
//! Stress and morale tuning is loaded and checked here; nothing steps it yet.

use crate::colony::{Colony, Module};
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

/// Hab modules are named `hab_*` until modules carry a kind.
const HAB_PREFIX: &str = "hab_";

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
pub struct CrewDef {
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

impl CrewDef {
    pub fn skill(&self, skill: Skill) -> u8 {
        self.skills.get(&skill).copied().unwrap_or(0)
    }
}

/// Stress and morale tuning; every `_bp` field is basis points (per hour for rates).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
        if self.calm_bp >= self.strained_bp
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
pub struct Roster {
    pub tuning: CrewTuning,
    pub crew: Vec<CrewDef>,
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
    /// Must hold calm < strained < breaking <= MAX_BP.
    ThresholdsOutOfOrder,
    RateOutOfRange {
        field: &'static str,
        bp: u16,
    },
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
                "stress thresholds must be calm < strained < breaking <= {MAX_BP}"
            ),
            CrewError::RateOutOfRange { field, bp } => {
                write!(f, "{field} is {bp}, above {MAX_BP}")
            }
        }
    }
}

impl std::error::Error for CrewError {}

fn is_hab(module: &Module) -> bool {
    module.id().as_str().starts_with(HAB_PREFIX) && module.machine().is_some()
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

impl Default for Roster {
    /// The shipped crew, bunked on the Ash Plain landing site.
    fn default() -> Self {
        // Both files are checked by tests, so failing here is a build bug.
        let colony = Colony::from_ron(crate::colony::LANDING_ASH_PLAIN)
            .expect("landing site content is valid");
        Self::from_ron(CREW, &colony).expect("crew content is valid")
    }
}

/// Why a proposed Split can't launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitError {
    /// Nobody on the Flight side has Piloting >= `pilot_min`.
    NoQualifiedPilot,
    /// A child's side of the Split has no adult on it.
    MayaWithoutAdult,
}

impl std::fmt::Display for SplitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SplitError::NoQualifiedPilot => write!(f, "no qualified pilot is flying"),
            SplitError::MayaWithoutAdult => write!(f, "Maya has no adult on her side"),
        }
    }
}

impl std::error::Error for SplitError {}

/// Check a proposed Split. Crew missing from `assignments` stay on the Ground.
/// Children are the crew with `adult: false` (only Maya in the shipped roster).
pub fn check_split(
    crew: &[CrewDef],
    tuning: &CrewTuning,
    assignments: &BTreeMap<CrewId, Assignment>,
) -> Result<(), SplitError> {
    let side = |c: &CrewDef| assignments.get(&c.id).copied().unwrap_or_default();
    if !crew
        .iter()
        .any(|c| side(c) == Assignment::Flight && c.skill(Skill::Piloting) >= tuning.pilot_min)
    {
        return Err(SplitError::NoQualifiedPilot);
    }
    for child in crew.iter().filter(|c| !c.adult) {
        if !crew.iter().any(|c| c.adult && side(c) == side(child)) {
            return Err(SplitError::MayaWithoutAdult);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colony::LANDING_ASH_PLAIN;

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
        r.tuning.pilot_min = MAX_SKILL + 1;
        assert_eq!(check(&r), Err(CrewError::NoQualifiedPilot));
    }

    #[test]
    fn stress_thresholds_must_be_ordered() {
        let base = shipped();
        let cases: [fn(&mut CrewTuning); 3] = [
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
            Err(SplitError::MayaWithoutAdult)
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
    fn missing_assignments_mean_ground() {
        let r = shipped();
        let mut assignments = BTreeMap::new();
        assignments.insert(id("cal"), Assignment::Flight);
        assignments.insert(id("elena"), Assignment::Ground);
        assert_eq!(check_split(&r.crew, &r.tuning, &assignments), Ok(()));
    }
}
