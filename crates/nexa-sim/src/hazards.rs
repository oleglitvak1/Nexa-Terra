//! Slip hazard content: the events a voyage can meet, the options each offers,
//! what they cost, and the check formula's tuning (`content/hazards.ron`).

use crate::check::{CheckDef, CheckTuning};
use crate::crew::Roster;
use crate::ship::MAX_INTEGRITY_PCT;
use nexa_types::{CrewId, HazardId};
use serde::{Deserialize, Serialize};
use std::ops::RangeInclusive;
use std::sync::OnceLock;

/// Shipped hazards, compiled in so loading needs no file I/O.
pub const HAZARDS: &str = include_str!("../../../content/hazards.ron");

/// The design's delay ranges (docs/game-design.md 4.B); content may narrow
/// them for tuning but not leave them.
pub const DESIGN_MINOR_SOLS: RangeInclusive<u32> = 5..=7;
pub const DESIGN_SEVERE_SOLS: RangeInclusive<u32> = 7..=10;
pub const DESIGN_RANDOM_CAP_SOLS: u32 = 12;

/// How bad a random delay is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DelayKind {
    Minor,
    /// Engine drift.
    Severe,
}

/// Random delay ranges, `(lowest, highest)` Sols, and the round-trip cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelayRanges {
    pub minor_sols: (u32, u32),
    pub severe_sols: (u32, u32),
    pub random_cap_sols: u32,
}

impl DelayRanges {
    pub fn range(&self, kind: DelayKind) -> (u32, u32) {
        match kind {
            DelayKind::Minor => self.minor_sols,
            DelayKind::Severe => self.severe_sols,
        }
    }
}

/// What taking an option costs, or what a Partial or Fail adds. All whole units.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Toll {
    /// Overclock Burns, each at the ship's `overclock_burn_l`.
    pub overclock_burns: u8,
    pub pods: u8,
    pub credits: u32,
    pub dump_crates: u32,
    /// Integrity points every crate in the hold loses.
    pub cargo_damage_pct: u8,
    pub delay: Option<DelayKind>,
}

impl Toll {
    pub fn is_free(&self) -> bool {
        self == &Toll::default()
    }
}

/// One way out of a hazard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HazardOption {
    /// Short stable name, e.g. `ecm`; what tools and saves refer to.
    pub id: String,
    pub label: String,
    /// Paid up front; an option the ship can't pay is unavailable.
    #[serde(default)]
    pub cost: Toll,
    /// None: the cost settles it with certainty.
    #[serde(default)]
    pub check: Option<CheckDef>,
    #[serde(default)]
    pub partial: Toll,
    #[serde(default)]
    pub fail: Toll,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HazardDef {
    pub id: HazardId,
    pub name: String,
    /// What the hazard terminal prints when it hits.
    pub text: String,
    pub options: Vec<HazardOption>,
}

impl HazardDef {
    pub fn option(&self, id: &str) -> Option<&HazardOption> {
        self.options.iter().find(|o| o.id == id)
    }
}

/// The hazards content file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HazardsDef {
    pub checks: CheckTuning,
    pub delays: DelayRanges,
    pub hazards: Vec<HazardDef>,
}

/// Why hazard content was rejected. `option` names are `hazard/option`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HazardError {
    Parse(String),
    /// `die_sides` below 2 or `skill_weight` of 0.
    BadCheckTuning,
    /// A delay range is reversed or outside the design's.
    DelayOutsideDesign(&'static str),
    NoHazards,
    DuplicateHazard(HazardId),
    NoOptions(HazardId),
    DuplicateOption(String),
    UnknownSpecialist {
        option: String,
        crew: CrewId,
    },
    ZeroDifficulty(String),
    DamageTooHigh(String),
    /// An option with no check must cost something, or it's a free escape.
    FreeEscape(String),
    /// Partial and Fail tolls need a check to roll them.
    TollWithoutCheck(String),
}

impl std::fmt::Display for HazardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HazardError::Parse(e) => write!(f, "hazard content doesn't parse: {e}"),
            HazardError::BadCheckTuning => {
                write!(f, "checks need die_sides of 2 or more and a skill_weight")
            }
            HazardError::DelayOutsideDesign(field) => {
                write!(f, "{field} is reversed or outside the design's range")
            }
            HazardError::NoHazards => write!(f, "no hazards listed"),
            HazardError::DuplicateHazard(id) => write!(f, "hazard {id} is listed twice"),
            HazardError::NoOptions(id) => write!(f, "hazard {id} has no options"),
            HazardError::DuplicateOption(o) => write!(f, "option {o} is listed twice"),
            HazardError::UnknownSpecialist { option, crew } => {
                write!(f, "option {option} names unknown crew {crew}")
            }
            HazardError::ZeroDifficulty(o) => write!(f, "option {o} has a difficulty of 0"),
            HazardError::DamageTooHigh(o) => {
                write!(
                    f,
                    "option {o} damages cargo by more than {MAX_INTEGRITY_PCT}%"
                )
            }
            HazardError::FreeEscape(o) => write!(f, "option {o} has no check and no cost"),
            HazardError::TollWithoutCheck(o) => {
                write!(f, "option {o} has partial or fail tolls but no check")
            }
        }
    }
}

impl std::error::Error for HazardError {}

fn within(range: (u32, u32), design: &RangeInclusive<u32>) -> bool {
    range.0 <= range.1 && design.contains(&range.0) && design.contains(&range.1)
}

impl HazardsDef {
    /// Parse and cross-check against the crew who may be named as specialists.
    pub fn from_ron(text: &str, roster: &Roster) -> Result<Self, HazardError> {
        let def: Self = ron::from_str(text).map_err(|e| HazardError::Parse(e.to_string()))?;
        def.check(roster)?;
        Ok(def)
    }

    /// Parsed once; checked by this module's tests, so failing here is a build bug.
    pub fn shipped() -> &'static Self {
        static SHIPPED: OnceLock<HazardsDef> = OnceLock::new();
        SHIPPED.get_or_init(|| {
            Self::from_ron(HAZARDS, &Roster::default()).expect("hazard content is valid")
        })
    }

    pub fn hazard(&self, id: &HazardId) -> Option<&HazardDef> {
        self.hazards.iter().find(|h| &h.id == id)
    }

    pub fn check(&self, roster: &Roster) -> Result<(), HazardError> {
        if self.checks.die_sides < 2 || self.checks.skill_weight == 0 {
            return Err(HazardError::BadCheckTuning);
        }
        let d = &self.delays;
        if !within(d.minor_sols, &DESIGN_MINOR_SOLS) {
            return Err(HazardError::DelayOutsideDesign("minor_sols"));
        }
        if !within(d.severe_sols, &DESIGN_SEVERE_SOLS) {
            return Err(HazardError::DelayOutsideDesign("severe_sols"));
        }
        if !(1..=DESIGN_RANDOM_CAP_SOLS).contains(&d.random_cap_sols) {
            return Err(HazardError::DelayOutsideDesign("random_cap_sols"));
        }
        if self.hazards.is_empty() {
            return Err(HazardError::NoHazards);
        }
        for (i, h) in self.hazards.iter().enumerate() {
            if self.hazards[..i].iter().any(|other| other.id == h.id) {
                return Err(HazardError::DuplicateHazard(h.id.clone()));
            }
            if h.options.is_empty() {
                return Err(HazardError::NoOptions(h.id.clone()));
            }
            for (j, o) in h.options.iter().enumerate() {
                check_option(h, o, &h.options[..j], roster)?;
            }
        }
        Ok(())
    }
}

fn check_option(
    h: &HazardDef,
    o: &HazardOption,
    earlier: &[HazardOption],
    roster: &Roster,
) -> Result<(), HazardError> {
    let name = || format!("{}/{}", h.id, o.id);
    if earlier.iter().any(|other| other.id == o.id) {
        return Err(HazardError::DuplicateOption(name()));
    }
    let tolls = [&o.cost, &o.partial, &o.fail];
    if tolls.iter().any(|t| t.cargo_damage_pct > MAX_INTEGRITY_PCT) {
        return Err(HazardError::DamageTooHigh(name()));
    }
    match &o.check {
        Some(check) => {
            if check.difficulty == 0 {
                return Err(HazardError::ZeroDifficulty(name()));
            }
            if let Some(crew) = check
                .specialists
                .keys()
                .find(|id| !roster.crew.iter().any(|c| &c.id == *id))
            {
                return Err(HazardError::UnknownSpecialist {
                    option: name(),
                    crew: crew.clone(),
                });
            }
        }
        None => {
            if o.cost.is_free() {
                return Err(HazardError::FreeEscape(name()));
            }
            if !o.partial.is_free() || !o.fail.is_free() {
                return Err(HazardError::TollWithoutCheck(name()));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crew::Skill;

    fn shipped() -> HazardsDef {
        HazardsDef::from_ron(HAZARDS, &Roster::default()).unwrap()
    }

    fn check(def: &HazardsDef) -> Result<(), HazardError> {
        def.check(&Roster::default())
    }

    fn option<'a>(def: &'a mut HazardsDef, id: &str) -> &'a mut HazardOption {
        def.hazards[0]
            .options
            .iter_mut()
            .find(|o| o.id == id)
            .unwrap()
    }

    #[test]
    fn shipped_interdiction_has_the_five_options() {
        let def = shipped();
        let h = def.hazard(&HazardId::new("pirate_interdiction")).unwrap();
        let ids: Vec<_> = h.options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["ecm", "overclock", "dump", "pay", "bluff"]);
        let skill = |id: &str| h.option(id).unwrap().check.as_ref().map(|c| c.skill);
        assert_eq!(skill("ecm"), Some(Skill::Piloting));
        assert_eq!(skill("overclock"), Some(Skill::Engineering));
        assert_eq!(skill("bluff"), Some(Skill::Negotiation));
        assert_eq!(skill("dump"), None);
        assert_eq!(skill("pay"), None);
        let specialist = |id: &str| {
            let check = h.option(id).unwrap().check.clone().unwrap();
            check.specialists.keys().cloned().collect::<Vec<_>>()
        };
        assert_eq!(specialist("ecm"), [CrewId::new("cal")]);
        assert_eq!(specialist("overclock"), [CrewId::new("jonathan")]);
        assert_eq!(specialist("bluff"), [CrewId::new("dax")]);
        assert_eq!(h.option("ecm").unwrap().cost.pods, 1);
        assert_eq!(h.option("overclock").unwrap().cost.overclock_burns, 1);
        assert_eq!(
            h.option("overclock").unwrap().fail.delay,
            Some(DelayKind::Severe)
        );
    }

    #[test]
    fn specialists_must_be_crew() {
        let mut def = shipped();
        let ecm = option(&mut def, "ecm").check.as_mut().unwrap();
        ecm.specialists.insert(CrewId::new("ghost"), 2);
        assert_eq!(
            check(&def),
            Err(HazardError::UnknownSpecialist {
                option: "pirate_interdiction/ecm".into(),
                crew: CrewId::new("ghost"),
            })
        );
    }

    #[test]
    fn delays_stay_within_the_design() {
        type Bend = fn(&mut DelayRanges);
        let cases: [(Bend, &str); 5] = [
            (|d| d.minor_sols = (4, 7), "minor_sols"),
            (|d| d.minor_sols = (7, 5), "minor_sols"),
            (|d| d.severe_sols = (7, 11), "severe_sols"),
            (|d| d.random_cap_sols = 13, "random_cap_sols"),
            (|d| d.random_cap_sols = 0, "random_cap_sols"),
        ];
        for (bend, field) in cases {
            let mut def = shipped();
            bend(&mut def.delays);
            assert_eq!(check(&def), Err(HazardError::DelayOutsideDesign(field)));
        }
    }

    #[test]
    fn options_must_make_sense() {
        let mut def = shipped();
        option(&mut def, "pay").id = "dump".into();
        assert_eq!(
            check(&def),
            Err(HazardError::DuplicateOption(
                "pirate_interdiction/dump".into()
            ))
        );

        let mut def = shipped();
        option(&mut def, "pay").cost = Toll::default();
        assert_eq!(
            check(&def),
            Err(HazardError::FreeEscape("pirate_interdiction/pay".into()))
        );

        let mut def = shipped();
        option(&mut def, "dump").fail.dump_crates = 2;
        assert_eq!(
            check(&def),
            Err(HazardError::TollWithoutCheck(
                "pirate_interdiction/dump".into()
            ))
        );

        let mut def = shipped();
        option(&mut def, "bluff").check.as_mut().unwrap().difficulty = 0;
        assert_eq!(
            check(&def),
            Err(HazardError::ZeroDifficulty(
                "pirate_interdiction/bluff".into()
            ))
        );

        let mut def = shipped();
        option(&mut def, "ecm").fail.cargo_damage_pct = 101;
        assert_eq!(
            check(&def),
            Err(HazardError::DamageTooHigh("pirate_interdiction/ecm".into()))
        );
    }

    #[test]
    fn check_tuning_must_roll_something() {
        let mut def = shipped();
        def.checks.die_sides = 1;
        assert_eq!(check(&def), Err(HazardError::BadCheckTuning));
    }

    #[test]
    fn hazards_need_options_and_unique_ids() {
        let mut def = shipped();
        def.hazards.push(def.hazards[0].clone());
        assert_eq!(
            check(&def),
            Err(HazardError::DuplicateHazard(HazardId::new(
                "pirate_interdiction"
            )))
        );
        def.hazards.truncate(1);
        def.hazards[0].options.clear();
        assert_eq!(
            check(&def),
            Err(HazardError::NoOptions(HazardId::new("pirate_interdiction")))
        );
        def.hazards.clear();
        assert_eq!(check(&def), Err(HazardError::NoHazards));
    }

    #[test]
    fn misspelled_fields_are_parse_errors() {
        let typo = HAZARDS.replacen("(dump_crates: 3)", "(dump_crate: 3)", 1);
        assert!(matches!(
            HazardsDef::from_ron(&typo, &Roster::default()),
            Err(HazardError::Parse(_))
        ));
    }
}
