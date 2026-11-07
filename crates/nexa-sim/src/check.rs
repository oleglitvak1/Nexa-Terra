//! Skill checks: the one function every hazard (and later haggle) rolls through.
//!
//! The Flight-side member with the best `skill * skill_weight + bonus` rolls a
//! die from 1 to `die_sides` on top. At or above the difficulty is a Success,
//! within `partial_band` below it a Partial, lower a Fail.

use crate::crew::{Crew, Skill};
use crate::rng::SimRng;
use nexa_types::CrewId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The check formula's numbers, from `content/hazards.ron`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckTuning {
    /// Score per skill point.
    pub skill_weight: u32,
    /// The die rolls 1..=die_sides.
    pub die_sides: u32,
    /// How far under the difficulty still counts as a Partial.
    pub partial_band: u32,
}

/// One check an option asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckDef {
    pub skill: Skill,
    pub difficulty: u32,
    /// Flight specializations: extra score for these crew, only while they fly.
    #[serde(default)]
    pub specialists: BTreeMap<CrewId, u32>,
}

/// Declared worst first, so `Ord` ranks a Success highest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Outcome {
    Fail,
    Partial,
    Success,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Outcome::Fail => "fail",
            Outcome::Partial => "partial",
            Outcome::Success => "success",
        }
    }
}

/// What a check rolled, for the transit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roll {
    /// Who rolled; `None` if nobody was on the Flight side.
    pub crew: Option<CrewId>,
    pub skill: Skill,
    /// The specialist bonus that counted (0 if none).
    pub bonus: u32,
    pub die: u32,
    pub score: u32,
    pub difficulty: u32,
    pub outcome: Outcome,
}

/// Roll `def` for whoever on the Flight side is best placed to make it.
pub fn check(crew: &Crew, def: &CheckDef, tuning: &CheckTuning, rng: &mut SimRng) -> Roll {
    // (member, base score, bonus); ties keep the earlier member in the roster.
    let mut best: Option<(&CrewId, u32, u32)> = None;
    for c in crew.flight() {
        let bonus = def.specialists.get(&c.id).copied().unwrap_or(0);
        // Saturating: tuning and bonuses come from content and saves, unbounded.
        let base = u32::from(c.skill(def.skill))
            .saturating_mul(tuning.skill_weight)
            .saturating_add(bonus);
        if best.is_none_or(|(_, b, _)| base > b) {
            best = Some((&c.id, base, bonus));
        }
    }
    let die = rng.range(1, tuning.die_sides.max(1));
    let (who, base, bonus) = match best {
        Some((id, base, bonus)) => (Some(id.clone()), base, bonus),
        None => (None, 0, 0),
    };
    let score = base.saturating_add(die);
    let outcome = if score >= def.difficulty {
        Outcome::Success
    } else if score.saturating_add(tuning.partial_band) >= def.difficulty {
        Outcome::Partial
    } else {
        Outcome::Fail
    };
    Roll {
        crew: who,
        skill: def.skill,
        bonus,
        die,
        score,
        difficulty: def.difficulty,
        outcome,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crew::{Assignment, Roster};

    const TUNING: CheckTuning = CheckTuning {
        skill_weight: 3,
        die_sides: 20,
        partial_band: 6,
    };

    fn flying(roster: Roster, who: &[&str]) -> Crew {
        let mut crew = Crew::new(roster);
        let split: BTreeMap<CrewId, Assignment> = who
            .iter()
            .map(|id| (CrewId::new(*id), Assignment::Flight))
            .collect();
        crew.confirm_split(&split).unwrap();
        crew
    }

    fn def(skill: Skill, difficulty: u32, specialists: &[(&str, u32)]) -> CheckDef {
        CheckDef {
            skill,
            difficulty,
            specialists: specialists
                .iter()
                .map(|(id, b)| (CrewId::new(*id), *b))
                .collect(),
        }
    }

    fn outcomes(crew: &Crew, d: &CheckDef, rolls: usize) -> [usize; 3] {
        let mut rng = SimRng::new(11);
        let mut counts = [0; 3];
        for _ in 0..rolls {
            counts[check(crew, d, &TUNING, &mut rng).outcome as usize] += 1;
        }
        counts
    }

    /// Cal (Piloting 5, +3) against 30: 18 + d20 succeeds on 12+ (45%), is
    /// partial on 6..=11 (30%) and fails on 1..=5 (25%).
    #[test]
    fn outcomes_spread_as_the_formula_says() {
        let crew = flying(Roster::default(), &["cal"]);
        let [fail, partial, success] =
            outcomes(&crew, &def(Skill::Piloting, 30, &[("cal", 3)]), 4000);
        assert!((800..1200).contains(&fail), "{fail}");
        assert!((1000..1400).contains(&partial), "{partial}");
        assert!((1600..2000).contains(&success), "{success}");
    }

    #[test]
    fn the_best_flight_member_rolls() {
        let crew = flying(Roster::default(), &["cal", "jonathan", "dax"]);
        let mut rng = SimRng::default();
        let roll = check(&crew, &def(Skill::Engineering, 20, &[]), &TUNING, &mut rng);
        assert_eq!(roll.crew, Some(CrewId::new("jonathan")));
        assert_eq!(roll.score, 15 + roll.die);
        assert!((1..=20).contains(&roll.die));
        // Cal and Dax both have Rigging 3: the earlier in the roster rolls.
        let roll = check(&crew, &def(Skill::Rigging, 20, &[]), &TUNING, &mut rng);
        assert_eq!(roll.crew, Some(CrewId::new("cal")));
    }

    #[test]
    fn more_skill_never_rolls_worse() {
        for seed in 0..200 {
            let mut last = Outcome::Fail;
            for rating in 0..=5 {
                let mut roster = Roster::default();
                roster.crew[0].skills.insert(Skill::Engineering, rating);
                let crew = flying(roster, &["cal", "jonathan"]);
                let mut rng = SimRng::new(seed);
                let roll = check(&crew, &def(Skill::Engineering, 25, &[]), &TUNING, &mut rng);
                assert!(roll.outcome >= last, "seed {seed} rating {rating}");
                last = roll.outcome;
            }
        }
    }

    #[test]
    fn a_specialist_bonus_counts_only_while_they_fly() {
        let bluff = def(Skill::Negotiation, 25, &[("dax", 4)]);
        let with_dax = flying(Roster::default(), &["cal", "dax"]);
        let roll = check(&with_dax, &bluff, &TUNING, &mut SimRng::new(3));
        assert_eq!(roll.crew, Some(CrewId::new("dax")));
        assert_eq!(roll.bonus, 4);
        assert_eq!(roll.score, 15 + 4 + roll.die);

        let without = flying(Roster::default(), &["cal"]);
        let roll = check(&without, &bluff, &TUNING, &mut SimRng::new(3));
        assert_eq!(roll.crew, Some(CrewId::new("cal")));
        assert_eq!((roll.bonus, roll.score), (0, roll.die));
    }

    #[test]
    fn nobody_flying_rolls_the_bare_die() {
        let crew = Crew::new(Roster::default());
        let roll = check(
            &crew,
            &def(Skill::Piloting, 5, &[]),
            &TUNING,
            &mut SimRng::new(8),
        );
        assert_eq!(roll.crew, None);
        assert_eq!(roll.score, roll.die);
    }

    #[test]
    fn extreme_tuning_saturates_instead_of_overflowing() {
        let crew = flying(Roster::default(), &["cal"]);
        let tuning = CheckTuning {
            skill_weight: u32::MAX,
            die_sides: 20,
            partial_band: u32::MAX,
        };
        let d = def(Skill::Piloting, u32::MAX, &[("cal", u32::MAX)]);
        let roll = check(&crew, &d, &tuning, &mut SimRng::new(1));
        assert_eq!((roll.score, roll.outcome), (u32::MAX, Outcome::Success));
    }

    #[test]
    fn partial_band_edges() {
        let crew = flying(Roster::default(), &["cal"]);
        // Base 15 (Piloting 5 * 3). Find rolls just on each edge by difficulty.
        let mut rng = SimRng::new(4);
        let roll = check(&crew, &def(Skill::Piloting, 0, &[]), &TUNING, &mut rng);
        let score = roll.score;
        let at = |difficulty| {
            check(
                &crew,
                &def(Skill::Piloting, difficulty, &[]),
                &TUNING,
                &mut SimRng::new(4),
            )
            .outcome
        };
        assert_eq!(at(score), Outcome::Success);
        assert_eq!(at(score + 1), Outcome::Partial);
        assert_eq!(at(score + 6), Outcome::Partial);
        assert_eq!(at(score + 7), Outcome::Fail);
    }
}
