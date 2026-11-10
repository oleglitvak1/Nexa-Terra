//! Headless voyages through The Slip: one with a report, or many per option
//! for tuning the check numbers in `content/hazards.ron`.

use nexa_sim::check::Outcome;
use nexa_sim::crew::Assignment;
use nexa_sim::flight::{FlightError, Resolution};
use nexa_sim::universe::OfflineUniverse;
use nexa_sim::{SimEvent, World};
use nexa_types::{CrewId, TICKS_PER_SOL};
use std::collections::BTreeMap;

/// Who flies when no `--split` is given: the pilot and the three specialists'
/// worth of checks (Cal, Jonathan, Dax), leaving Elena and Oron-3 with Maya.
pub const DEFAULT_FLIGHT: &str = "cal,jonathan,dax";

/// Sols on the ground before a launch or a sweep's voyages, unless SOLS says
/// otherwise: the slice's planned launch day.
pub const LAUNCH_SOLS: u64 = 10;

/// The posture the base runs under before a launch or a sweep's voyages,
/// unless `--orders` says otherwise; `--orders none` runs it with no cards.
pub const LAUNCH_ORDERS: &str = "caretaker";

/// What the colony does before launch: `sols` on the ground under `orders`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ground {
    pub sols: u64,
    pub orders: Option<String>,
}

impl Default for Ground {
    fn default() -> Self {
        Self {
            sols: LAUNCH_SOLS,
            orders: Some(LAUNCH_ORDERS.to_string()),
        }
    }
}

impl Ground {
    /// From the command line: each one left out keeps the default, and
    /// `--orders none` runs with no cards.
    pub fn from_args(sols: Option<u64>, orders: Option<&str>) -> Self {
        let default = Self::default();
        Self {
            sols: sols.unwrap_or(default.sols),
            orders: match orders {
                Some("none") => None,
                Some(name) => Some(name.to_string()),
                None => default.orders,
            },
        }
    }
}

/// A voyage's own events, and the colony's catch-up events on arrival.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Flown {
    pub voyage: Vec<SimEvent>,
    pub colony: Vec<SimEvent>,
}

impl Flown {
    pub fn resolutions(&self) -> impl Iterator<Item = &Resolution> {
        self.voyage.iter().filter_map(|e| match e {
            SimEvent::HazardResolved(r) => Some(r),
            _ => None,
        })
    }
}

/// Launch and fly to arrival, answering every hazard with `option`.
pub fn fly(world: &mut World, option: &str) -> Result<Flown, String> {
    let mut flown = Flown {
        voyage: world
            .launch(&mut OfflineUniverse::default())
            .map_err(|e| format!("launch refused: {e}"))?,
        colony: Vec::new(),
    };
    loop {
        match world.advance_voyage() {
            Ok(step) => {
                let arrived = step.arrived();
                flown.voyage.extend(step.events);
                if arrived {
                    flown.colony = step.colony;
                    return Ok(flown);
                }
            }
            Err(FlightError::HazardPending) => {
                let events = world
                    .resolve_hazard(option)
                    .map_err(|e| format!("{option}: {e}"))?;
                flown.voyage.extend(events);
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// "overclock -> partial: jonathan 25 vs 29; paid 150 L, cargo -10%"
pub fn describe(r: &Resolution) -> String {
    let mut line = format!("{} -> {}", r.option, r.outcome.label());
    if let Some(roll) = &r.roll {
        let who = roll.crew.as_ref().map_or("nobody", CrewId::as_str);
        line += &format!(
            ": {who} {} vs {} (die {}, bonus {})",
            roll.score, roll.difficulty, roll.die, roll.bonus
        );
    }
    let mut paid = r.paid.parts();
    if r.paid.delay_cut_sols > 0 {
        paid.push(format!("{} Sols cut by the cap", r.paid.delay_cut_sols));
    }
    if !paid.is_empty() {
        line += &format!("; paid {}", paid.join(", "));
    }
    line
}

/// One line on where the ship and purse ended up.
pub fn ship_line(world: &World) -> String {
    let ship = &world.ship;
    let (delay, random, cap) = world.voyage.as_ref().map_or((0, 0, 0), |v| {
        (
            v.delay_sols(),
            v.random_delay_sols(),
            v.rules().delays.random_cap_sols,
        )
    });
    format!(
        "fuel {}/{} L, pods {}, credits {}, cargo {} crates {} kg integrity {}%, delay +{delay} (random {random}/{cap})",
        ship.fuel_l(),
        ship.def().tank_l,
        ship.pods(),
        world.credits,
        ship.cargo().len(),
        ship.cargo_mass_kg(),
        ship.cargo_integrity_pct(),
    )
}

/// Totals over many voyages that all answered with one option.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub voyages: u64,
    /// Indexed by `Outcome as usize`: Fail, Partial, Success.
    pub outcomes: [u64; 3],
    pub fuel_l: u64,
    pub integrity_pct: u64,
    pub crates: u64,
    pub credits: u64,
    pub delay_sols: u64,
}

impl Tally {
    fn add(&mut self, world: &World, flown: &Flown) {
        self.voyages += 1;
        for r in flown.resolutions() {
            self.outcomes[r.outcome as usize] += 1;
        }
        self.fuel_l += u64::from(world.ship.fuel_l());
        self.integrity_pct += u64::from(world.ship.cargo_integrity_pct());
        self.crates += world.ship.cargo().len() as u64;
        self.credits += u64::from(world.credits);
        self.delay_sols += world
            .voyage
            .as_ref()
            .map_or(0, |v| u64::from(v.delay_sols()));
    }

    /// Share of all rolls with this outcome, in percent.
    pub fn rate(&self, outcome: Outcome) -> f64 {
        let total: u64 = self.outcomes.iter().sum();
        if total == 0 {
            return 0.0;
        }
        self.outcomes[outcome as usize] as f64 * 100.0 / total as f64
    }

    /// Mean per voyage.
    fn mean(&self, sum: u64) -> f64 {
        sum as f64 / self.voyages.max(1) as f64
    }

    pub const HEADER: &'static str =
        "option     success  partial     fail   fuel L  integrity  crates  credits   delay";

    pub fn row(&self, option: &str) -> String {
        format!(
            "{option:<9} {:>7.1}% {:>7.1}% {:>7.1}% {:>8.0} {:>9.1}% {:>7.1} {:>8.0} {:>+7.1}",
            self.rate(Outcome::Success),
            self.rate(Outcome::Partial),
            self.rate(Outcome::Fail),
            self.mean(self.fuel_l),
            self.mean(self.integrity_pct),
            self.mean(self.crates),
            self.mean(self.credits),
            self.mean(self.delay_sols),
        )
    }
}

/// Seed `seed`'s colony after `ground`, with `split` confirmed, ready to launch.
/// As in play, the Split is confirmed at launch: everyone spends the ground
/// Sols on the Ground.
pub fn on_the_pad(
    seed: u64,
    ground: &Ground,
    split: &BTreeMap<CrewId, Assignment>,
) -> Result<World, String> {
    let mut world = World::landfall_seeded(seed);
    if let Some(name) = &ground.orders {
        nexa_sim::script::load_preset(&mut world, name).map_err(|e| e.to_string())?;
    }
    for _ in 0..ground.sols * TICKS_PER_SOL {
        world.tick();
    }
    world
        .confirm_split(split)
        .map_err(|e| format!("split: {e}"))?;
    Ok(world)
}

/// Fly seeds `0..seeds` after `ground` with `split`, every hazard answered by `option`.
pub fn sweep(
    seeds: u64,
    ground: &Ground,
    split: &BTreeMap<CrewId, Assignment>,
    option: &str,
) -> Result<Tally, String> {
    let mut tally = Tally::default();
    for seed in 0..seeds {
        let mut world = on_the_pad(seed, ground, split)?;
        let flown = fly(&mut world, option).map_err(|e| format!("seed {seed}: {e}"))?;
        tally.add(&world, &flown);
    }
    Ok(tally)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_split;

    #[test]
    fn a_voyage_flies_to_arrival_and_is_repeatable() {
        let split = parse_split(DEFAULT_FLIGHT);
        let run = || {
            let mut world = World::landfall_seeded(3);
            world.confirm_split(&split).unwrap();
            let flown = fly(&mut world, "bluff").unwrap();
            (world, flown)
        };
        let (a, flown_a) = run();
        let (b, flown_b) = run();
        assert_eq!((a, &flown_a), (b, &flown_b));
        assert_eq!(flown_a.resolutions().count(), 1);
        assert!(matches!(
            flown_a.voyage.last(),
            Some(SimEvent::Arrived { .. })
        ));
        assert!(!flown_a.colony.is_empty());
    }

    #[test]
    fn a_refused_launch_or_option_is_an_error() {
        let mut world = World::landfall();
        assert!(fly(&mut world, "pay")
            .unwrap_err()
            .starts_with("launch refused"));
        let mut world = World::landfall();
        world.confirm_split(&parse_split("cal")).unwrap();
        assert!(fly(&mut world, "warp").unwrap_err().contains("warp"));
    }

    #[test]
    fn a_sweep_counts_every_roll_and_certain_options_never_roll_badly() {
        let split = parse_split(DEFAULT_FLIGHT);
        let tally = sweep(40, &Ground::default(), &split, "ecm").unwrap();
        assert_eq!(tally.voyages, 40);
        assert_eq!(tally.outcomes.iter().sum::<u64>(), 40);
        let total =
            tally.rate(Outcome::Success) + tally.rate(Outcome::Partial) + tally.rate(Outcome::Fail);
        assert!((total - 100.0).abs() < 1e-9);
        let pay = sweep(10, &Ground::default(), &split, "pay").unwrap();
        assert_eq!(pay.outcomes, [0, 0, 10]);
        assert_eq!(pay.credits, 10 * 200);
        assert!(tally.row("ecm").starts_with("ecm "));
    }

    #[test]
    fn the_pad_is_reached_after_the_ground_sols_under_the_posture() {
        let split = parse_split(DEFAULT_FLIGHT);
        let world = on_the_pad(1, &Ground::default(), &split).unwrap();
        assert_eq!(world.now.sol(), LAUNCH_SOLS);
        let cards = nexa_sim::orders::OrdersDef::shipped().presets[LAUNCH_ORDERS]
            .cards
            .len();
        assert_eq!(world.orders.len(), cards);
        let bare = Ground {
            sols: 0,
            orders: None,
        };
        let world = on_the_pad(1, &bare, &split).unwrap();
        assert_eq!((world.now.0, world.orders.len()), (0, 0));
        let bad = Ground {
            orders: Some("panic".into()),
            ..bare
        };
        assert!(on_the_pad(1, &bad, &split).unwrap_err().contains("panic"));
    }

    #[test]
    fn the_flight_crew_spend_the_ground_sols_on_the_ground() {
        // Unattended, the habs go cold, so time on the Ground shows as stress.
        let ground = Ground::from_args(None, Some("none"));
        let world = on_the_pad(1, &ground, &parse_split(DEFAULT_FLIGHT)).unwrap();
        let mut unsplit = World::landfall_seeded(1);
        for _ in 0..ground.sols * TICKS_PER_SOL {
            unsplit.tick();
        }
        let cal = CrewId::new("cal");
        let stress = |w: &World| w.crew.vitals(&cal).unwrap().stress_bp;
        assert!(stress(&unsplit) > 0, "the cold should stress cal");
        assert_eq!(stress(&world), stress(&unsplit));
        assert_eq!(world.crew.assignment(&cal), Some(Assignment::Flight));
    }

    #[test]
    fn ground_args_keep_the_defaults_they_leave_out() {
        assert_eq!(Ground::from_args(None, None), Ground::default());
        assert_eq!(
            Ground::from_args(Some(3), Some("protect-habs")),
            Ground {
                sols: 3,
                orders: Some("protect-habs".into())
            }
        );
        assert_eq!(
            Ground::from_args(None, Some("none")),
            Ground {
                sols: LAUNCH_SOLS,
                orders: None
            }
        );
    }

    #[test]
    fn a_sweep_carries_the_crates_the_ground_run_left_aboard() {
        // Dumping takes 3 of whatever the hold has at launch.
        let split = parse_split(DEFAULT_FLIGHT);
        let world = on_the_pad(0, &Ground::default(), &split).unwrap();
        let aboard = world.ship.cargo().len() as u64;
        let dump = sweep(5, &Ground::default(), &split, "dump").unwrap();
        assert_eq!(dump.crates, 5 * (aboard - 3));
    }

    #[test]
    fn describe_reads_like_a_log_line() {
        let mut world = on_the_pad(1, &Ground::default(), &parse_split(DEFAULT_FLIGHT)).unwrap();
        let flown = fly(&mut world, "dump").unwrap();
        let r = flown.resolutions().next().unwrap();
        assert_eq!(describe(r), "dump -> success; paid 3 crates (3000 kg)");
    }
}
