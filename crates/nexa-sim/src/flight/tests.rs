//! Launch, transit, every interdiction option at each outcome, the delay cap,
//! arrival, saves and determinism.

use super::*;
use crate::crew::{Assignment, SplitError};
use crate::universe::OfflineUniverse;
use nexa_types::LaunchBriefing;
use std::collections::BTreeMap;
use std::ops::RangeInclusive;

/// A universe that answers every launch with the same briefing.
struct Scripted(LaunchBriefing);

impl Universe for Scripted {
    fn on_launch(&mut self, _ctx: &LaunchContext) -> LaunchBriefing {
        self.0.clone()
    }
}

fn interdiction_on(sols: &[u32]) -> Scripted {
    Scripted(LaunchBriefing {
        events: sols
            .iter()
            .map(|&sol| SlipEvent {
                sol,
                hazard: HazardId::new("pirate_interdiction"),
            })
            .collect(),
    })
}

fn split(flight: &[&str]) -> BTreeMap<CrewId, Assignment> {
    flight
        .iter()
        .map(|id| (CrewId::new(*id), Assignment::Flight))
        .collect()
}

/// Cal, Jonathan and Dax fly; Elena and Oron-3 stay with Maya.
fn crewed(seed: u64) -> World {
    let mut world = World::landfall_seeded(seed);
    world
        .confirm_split(&split(&["cal", "jonathan", "dax"]))
        .unwrap();
    world
}

/// Shipped rules, bent so `option`'s check always lands on `outcome`.
fn rigged(option: &str, outcome: Outcome) -> HazardsDef {
    let mut rules = HazardsDef::shipped().clone();
    for o in &mut rules.hazards[0].options {
        if let (true, Some(check)) = (o.id == option, o.check.as_mut()) {
            check.difficulty = match outcome {
                Outcome::Success => 1,
                Outcome::Partial | Outcome::Fail => 1000,
            };
        }
    }
    rules.checks.partial_band = match outcome {
        Outcome::Partial => 1000,
        _ => 0,
    };
    rules
}

/// Launched and stopped at the interdiction on Sol 2.
fn at_hazard(rules: HazardsDef) -> World {
    let mut world = crewed(7);
    world
        .launch_with_rules(&mut interdiction_on(&[2]), rules)
        .unwrap();
    assert_eq!(world.advance_voyage().unwrap().events.len(), 1);
    let step = world.advance_voyage().unwrap();
    assert_eq!(
        step.events.last(),
        Some(&SimEvent::HazardEncountered {
            hazard: HazardId::new("pirate_interdiction")
        })
    );
    world
}

fn resolution(events: &[SimEvent]) -> &Resolution {
    events
        .iter()
        .find_map(|e| match e {
            SimEvent::HazardResolved(r) => Some(r),
            _ => None,
        })
        .unwrap()
}

/// Fly to arrival, answering every hazard with `option`: the voyage's
/// events, then the colony's catch-up events.
fn fly_apart(world: &mut World, option: &str) -> (Vec<SimEvent>, Vec<SimEvent>) {
    let mut events = Vec::new();
    loop {
        match world.advance_voyage() {
            Ok(step) => {
                let arrived = step.arrived();
                events.extend(step.events);
                if arrived {
                    return (events, step.colony);
                }
                assert!(step.colony.is_empty());
            }
            Err(FlightError::HazardPending) => events.extend(world.resolve_hazard(option).unwrap()),
            Err(e) => panic!("{e:?}"),
        }
    }
}

fn fly(world: &mut World, option: &str) -> Vec<SimEvent> {
    let (mut events, colony) = fly_apart(world, option);
    events.extend(colony);
    events
}

fn voyage(world: &World) -> &Voyage {
    world.voyage.as_ref().unwrap()
}

#[test]
fn launch_needs_a_flight_crew() {
    let mut world = World::landfall();
    let before = world.clone();
    assert_eq!(
        world.launch(&mut OfflineUniverse::default()),
        Err(LaunchError::NoFlightCrew)
    );
    assert_eq!(world, before);
}

#[test]
fn launch_burns_a_leg_and_holds_the_colony() {
    let mut world = crewed(1);
    let events = world.launch(&mut OfflineUniverse::default()).unwrap();
    assert_eq!(
        events,
        [SimEvent::Launched {
            destination: StationId::new("barge_77"),
            flight: ["jonathan", "cal", "dax"].map(CrewId::new).to_vec(),
            fuel_burned_l: 350,
        }]
    );
    assert_eq!(world.ship.fuel_l(), 650);
    assert_eq!(world.ship.location(), &ShipLocation::InSlip);
    let v = voyage(&world);
    assert_eq!((v.sols_flown(), v.transit_sols()), (0, 5));
    assert_eq!(v.destination_name(), "Scrap Barge 77");
    // The colony waits for the voyage.
    let now = world.now;
    let colony = world.colony.clone();
    assert!(world.tick().is_empty());
    assert_eq!((world.now, &world.colony), (now, &colony));
}

#[test]
fn the_ship_launches_once_and_the_split_holds_while_she_is_away() {
    let mut world = crewed(1);
    world.launch(&mut OfflineUniverse::default()).unwrap();
    assert_eq!(
        world.launch(&mut OfflineUniverse::default()),
        Err(LaunchError::ShipAway)
    );
    assert_eq!(
        world.confirm_split(&split(&["cal"])),
        Err(SplitError::ShipAway)
    );
}

#[test]
fn a_bad_briefing_refuses_the_launch() {
    let mut world = crewed(1);
    let before = world.clone();
    let mut ghost = Scripted(LaunchBriefing {
        events: vec![SlipEvent {
            sol: 2,
            hazard: HazardId::new("ghost"),
        }],
    });
    assert!(matches!(
        world.launch(&mut ghost),
        Err(LaunchError::BadBriefing(_))
    ));
    for sol in [0, 5] {
        assert!(matches!(
            world.launch(&mut interdiction_on(&[sol])),
            Err(LaunchError::BadBriefing(_))
        ));
    }
    assert_eq!(world, before);
}

#[test]
fn a_hazard_stops_the_voyage_until_answered() {
    let mut world = at_hazard(HazardsDef::shipped().clone());
    assert_eq!(world.advance_voyage(), Err(FlightError::HazardPending));
    let pending = voyage(&world).pending().unwrap();
    assert_eq!(pending.id, HazardId::new("pirate_interdiction"));
    assert_eq!(
        world.resolve_hazard("warp"),
        Err(FlightError::NoSuchOption("warp".into()))
    );
    world.resolve_hazard("pay").unwrap();
    assert!(voyage(&world).pending().is_none());
    assert_eq!(
        world.resolve_hazard("pay"),
        Err(FlightError::NoHazardPending)
    );
    world.advance_voyage().unwrap();
}

/// The ship after an option, compared with the ship at the hazard.
struct After {
    fuel_l: u32,
    pods: u8,
    credits: u32,
    crates: usize,
    integrity: u8,
    delay: RangeInclusive<u32>,
}

const CLEAN: After = After {
    fuel_l: 650,
    pods: 1,
    credits: 500,
    crates: 10,
    integrity: 100,
    delay: 0..=0,
};

fn check_option(option: &str, outcome: Outcome, expect: After) {
    let mut world = at_hazard(rigged(option, outcome));
    let events = world.resolve_hazard(option).unwrap();
    let r = resolution(&events);
    let label = format!("{option} {outcome:?}");
    assert_eq!(r.option, option, "{label}");
    // Rigging only moves a check; a certain option always counts as a Success.
    let rolls = HazardsDef::shipped().hazards[0]
        .option(option)
        .is_some_and(|o| o.check.is_some());
    let expected = if rolls { outcome } else { Outcome::Success };
    assert_eq!(r.outcome, expected, "{label}");
    assert_eq!(world.ship.fuel_l(), expect.fuel_l, "{label}");
    assert_eq!(world.ship.pods(), expect.pods, "{label}");
    assert_eq!(world.credits, expect.credits, "{label}");
    assert_eq!(world.ship.cargo().len(), expect.crates, "{label}");
    assert_eq!(
        world.ship.cargo_integrity_pct(),
        expect.integrity,
        "{label}"
    );
    let v = voyage(&world);
    assert!(expect.delay.contains(&v.delay_sols()), "{label}");
    assert_eq!(v.random_delay_sols(), v.delay_sols(), "{label}");
    assert_eq!(r.paid.delay_sols, v.delay_sols(), "{label}");
    assert_eq!(v.resolutions(), std::slice::from_ref(r), "{label}");
}

#[test]
fn ecm_spends_a_pod_and_cal_flies_it() {
    let spent = After { pods: 0, ..CLEAN };
    check_option("ecm", Outcome::Success, spent);
    let spent = After { pods: 0, ..CLEAN };
    check_option(
        "ecm",
        Outcome::Partial,
        After {
            delay: 5..=7,
            ..spent
        },
    );
    let spent = After { pods: 0, ..CLEAN };
    check_option(
        "ecm",
        Outcome::Fail,
        After {
            integrity: 80,
            delay: 5..=7,
            ..spent
        },
    );
    let mut world = at_hazard(HazardsDef::shipped().clone());
    let events = world.resolve_hazard("ecm").unwrap();
    let roll = resolution(&events).roll.clone().unwrap();
    assert_eq!((roll.crew, roll.bonus), (Some(CrewId::new("cal")), 3));
}

#[test]
fn overclock_burns_150_and_jonathan_tunes_it() {
    let burned = || After {
        fuel_l: 500,
        ..CLEAN
    };
    check_option("overclock", Outcome::Success, burned());
    check_option(
        "overclock",
        Outcome::Partial,
        After {
            integrity: 90,
            ..burned()
        },
    );
    // Failure is engine drift: a severe delay.
    check_option(
        "overclock",
        Outcome::Fail,
        After {
            delay: 7..=10,
            ..burned()
        },
    );
    let mut world = at_hazard(HazardsDef::shipped().clone());
    let events = world.resolve_hazard("overclock").unwrap();
    let roll = resolution(&events).roll.clone().unwrap();
    assert_eq!((roll.crew, roll.bonus), (Some(CrewId::new("jonathan")), 3));
}

#[test]
fn bluffing_is_dax_s_game() {
    check_option("bluff", Outcome::Success, CLEAN);
    check_option(
        "bluff",
        Outcome::Partial,
        After {
            delay: 5..=7,
            ..CLEAN
        },
    );
    check_option(
        "bluff",
        Outcome::Fail,
        After {
            crates: 6,
            integrity: 90,
            ..CLEAN
        },
    );
    let mut world = at_hazard(HazardsDef::shipped().clone());
    let events = world.resolve_hazard("bluff").unwrap();
    let roll = resolution(&events).roll.clone().unwrap();
    assert_eq!((roll.crew, roll.bonus), (Some(CrewId::new("dax")), 3));
}

#[test]
fn concessions_are_certain_and_cost_what_they_say() {
    // Rigging can't touch an option with no check.
    for outcome in [Outcome::Success, Outcome::Partial, Outcome::Fail] {
        check_option("dump", outcome, After { crates: 7, ..CLEAN });
        check_option(
            "pay",
            outcome,
            After {
                credits: 200,
                ..CLEAN
            },
        );
    }
    let mut world = at_hazard(HazardsDef::shipped().clone());
    let events = world.resolve_hazard("dump").unwrap();
    let r = resolution(&events);
    assert_eq!((r.roll.clone(), r.outcome), (None, Outcome::Success));
    assert_eq!((r.paid.crates_dumped, r.paid.mass_dumped_kg), (3, 3000));
}

#[test]
fn options_the_ship_cant_pay_are_unavailable() {
    let mut rules = HazardsDef::shipped().clone();
    for o in &mut rules.hazards[0].options {
        match o.id.as_str() {
            "ecm" => o.cost.pods = 2,
            "pay" => o.cost.credits = 501,
            "overclock" => o.cost.overclock_burns = 5,
            "dump" => o.cost.dump_crates = 11,
            _ => {}
        }
    }
    let mut world = at_hazard(rules);
    let before = world.clone();
    for (option, why) in [
        ("ecm", Unavailable::NoPods),
        ("pay", Unavailable::NotEnoughCredits),
        ("overclock", Unavailable::NotEnoughFuel),
        ("dump", Unavailable::NotEnoughCrates),
    ] {
        assert_eq!(
            world.resolve_hazard(option),
            Err(FlightError::Unavailable(why))
        );
        assert_eq!(world, before);
    }
}

#[test]
fn an_overclock_must_leave_fuel_for_the_way_home() {
    let mut rules = HazardsDef::shipped().clone();
    for o in &mut rules.hazards[0].options {
        if o.id == "overclock" {
            // 650 L after launch: 300 L would leave 350 L, 450 L only 200 L.
            o.cost.overclock_burns = 3;
        }
    }
    let mut world = at_hazard(rules);
    assert_eq!(
        world.resolve_hazard("overclock"),
        Err(FlightError::Unavailable(Unavailable::FuelReserve))
    );
    let mut world = at_hazard(HazardsDef::shipped().clone());
    world.resolve_hazard("overclock").unwrap();
    assert_eq!(world.ship.fuel_l(), 500);
}

#[test]
fn random_delays_stop_at_the_round_trip_cap() {
    let mut world = crewed(3);
    let mut rules = rigged("overclock", Outcome::Fail);
    // Free burns, so three in a row don't run into the fuel reserve.
    for o in &mut rules.hazards[0].options {
        if o.id == "overclock" {
            o.cost.overclock_burns = 0;
        }
    }
    world
        .launch_with_rules(&mut interdiction_on(&[1, 2, 3]), rules)
        .unwrap();
    let events = fly(&mut world, "overclock");
    let paid: Vec<&Paid> = events
        .iter()
        .filter_map(|e| match e {
            SimEvent::HazardResolved(r) => Some(&r.paid),
            _ => None,
        })
        .collect();
    assert_eq!(paid.len(), 3);
    // 7..=10 each: the first fits, the second hits the cap, the third adds nothing.
    assert!((7..=10).contains(&paid[0].delay_sols));
    assert_eq!(paid[0].delay_sols + paid[1].delay_sols, 12);
    assert!(paid[1].delay_cut_sols > 0);
    assert_eq!(paid[2].delay_sols, 0);
    let v = voyage(&world);
    assert_eq!((v.delay_sols(), v.random_delay_sols()), (12, 12));
    assert_eq!(v.sols_total(), 17);
    assert_eq!(world.ship.fuel_l(), 650);
}

#[test]
fn arrival_holds_over_barge_77() {
    let mut world = crewed(5);
    world.launch(&mut OfflineUniverse::default()).unwrap();
    let (events, _colony) = fly_apart(&mut world, "pay");
    let v = voyage(&world);
    assert_eq!(v.status(), VoyageStatus::Holding);
    assert_eq!(
        events.last(),
        Some(&SimEvent::Arrived {
            destination: StationId::new("barge_77"),
            sols: v.sols_total(),
            delay_sols: 0,
        })
    );
    assert_eq!(
        world.ship.location(),
        &ShipLocation::Holding(StationId::new("barge_77"))
    );
    assert_eq!(world.advance_voyage(), Err(FlightError::NotInTransit));
    assert_eq!(
        world.launch(&mut OfflineUniverse::default()),
        Err(LaunchError::ShipAway)
    );
    // Holding, the colony runs again.
    let now = world.now;
    world.tick();
    assert_eq!(world.now, Tick(now.0 + 1));
}

#[test]
fn same_seed_same_choices_same_voyage() {
    for option in ["ecm", "overclock", "bluff"] {
        let mut a = crewed(11);
        let mut b = crewed(11);
        let launch_a = a.launch(&mut OfflineUniverse::default()).unwrap();
        let launch_b = b.launch(&mut OfflineUniverse::default()).unwrap();
        assert_eq!(launch_a, launch_b);
        assert_eq!(fly(&mut a, option), fly(&mut b, option));
        assert_eq!(a, b);
    }
}

#[test]
fn a_voyage_survives_a_save_at_the_hazard() {
    let mut world = at_hazard(HazardsDef::shipped().clone());
    let mut loaded: World = ron::from_str(&ron::to_string(&world).unwrap()).unwrap();
    assert_eq!(loaded, world);
    assert_eq!(fly(&mut loaded, "bluff"), fly(&mut world, "bluff"));
    assert_eq!(loaded, world);
}

#[test]
fn a_save_with_the_ship_out_of_place_is_rejected() {
    let mut world = crewed(1);
    world.launch(&mut OfflineUniverse::default()).unwrap();
    let saved = ron::to_string(&world).unwrap();
    let edited = saved.replacen("location:InSlip", "location:Pad", 1);
    assert_ne!(edited, saved);
    assert!(ron::from_str::<World>(&edited).is_err());
    // On the pad with no voyage is fine; away with none is not.
    let docked = ron::to_string(&World::landfall()).unwrap();
    let edited = docked.replacen("location:Pad", "location:InSlip", 1);
    assert_ne!(edited, docked);
    assert!(ron::from_str::<World>(&edited).is_err());
}

#[test]
fn a_saved_voyage_must_add_up() {
    let world = at_hazard(HazardsDef::shipped().clone());
    let ok = voyage(&world).clone();
    assert_eq!(ok.check(&world.ship, &world.crew), Ok(()));
    let bent: [fn(&mut Voyage); 5] = [
        // Someone aboard who isn't on the Flight side.
        |v| v.flight_crew.push(CrewId::new("elena")),
        // A hazard on a Sol already flown.
        |v| {
            v.pending = None;
            v.ahead.push(SlipEvent {
                sol: 1,
                hazard: HazardId::new("pirate_interdiction"),
            });
        },
        // In transit with the whole leg flown.
        |v| v.sols_flown = v.sols_total(),
        // Holding short of the station.
        |v| {
            v.status = VoyageStatus::Holding;
            v.pending = None;
        },
        // Holding with a hazard still waiting.
        |v| {
            v.status = VoyageStatus::Holding;
            v.sols_flown = v.sols_total();
        },
    ];
    for (i, bend) in bent.iter().enumerate() {
        let mut v = ok.clone();
        bend(&mut v);
        let ship = if v.status == VoyageStatus::Holding {
            let mut ship = world.ship.clone();
            ship.set_location(ShipLocation::Holding(v.destination.clone()));
            ship
        } else {
            world.ship.clone()
        };
        assert!(v.check(&ship, &world.crew).is_err(), "case {i}");
    }
}

fn sols_started(events: &[SimEvent]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::SolStarted { sol } => Some(*sol),
            _ => None,
        })
        .collect()
}

#[test]
fn arrival_runs_the_colony_through_every_sol_flown() {
    let mut world = crewed(5);
    // Launch on Sol 2, hour 0.
    for _ in 0..2 * TICKS_PER_SOL {
        world.tick();
    }
    let departed = world.now;
    let mut drifting = world.clone();

    world.launch(&mut OfflineUniverse::default()).unwrap();
    let (_, colony) = fly_apart(&mut world, "pay");
    assert_eq!(world.now, Tick(departed.0 + ticks_in(5)));
    assert_eq!(sols_started(&colony), [3, 4, 5, 6, 7]);

    // A failed Overclock Burn drifts the engine: 7 to 10 more Sols to catch up.
    drifting
        .launch_with_rules(
            &mut interdiction_on(&[2]),
            rigged("overclock", Outcome::Fail),
        )
        .unwrap();
    let (_, colony) = fly_apart(&mut drifting, "overclock");
    let sols = voyage(&drifting).sols_total();
    assert!((12..=15).contains(&sols), "{sols}");
    assert_eq!(drifting.now, Tick(departed.0 + ticks_in(sols)));
    assert_eq!(
        sols_started(&colony),
        (3..=u64::from(2 + sols)).collect::<Vec<_>>()
    );
}

#[test]
fn standing_orders_run_while_the_ship_is_away() {
    let mut world = crewed(2);
    for order in &crate::orders::OrdersDef::shipped().presets["caretaker"] {
        world.add_order(order.clone()).unwrap();
    }
    world.launch(&mut OfflineUniverse::default()).unwrap();
    let (voyage_events, colony) = fly_apart(&mut world, "dump");
    assert!(colony
        .iter()
        .any(|e| matches!(e, SimEvent::OrderFired { .. })));
    // The Caretaker's work shows up only in the catch-up, never mid-flight.
    assert!(!voyage_events
        .iter()
        .any(|e| matches!(e, SimEvent::OrderFired { .. })));
}

#[test]
fn ground_crew_feel_the_sols_away_and_the_flight_crew_do_not() {
    let mut world = crewed(4);
    world
        .launch_with_rules(
            &mut interdiction_on(&[2]),
            rigged("overclock", Outcome::Fail),
        )
        .unwrap();
    let (_, colony) = fly_apart(&mut world, "overclock");
    let changed = |who: &str| {
        colony
            .iter()
            .any(|e| matches!(e, SimEvent::CrewStressChanged { crew, .. } if crew.as_str() == who))
    };
    // Unattended, the habs go cold within the dozen-odd Sols away.
    for ground in ["elena", "maya", "oron"] {
        assert!(changed(ground), "{ground}");
    }
    // Father's Vigilance reaches Jonathan in The Slip; nobody else aboard.
    assert!(changed("jonathan"));
    for flying in ["cal", "dax"] {
        assert!(!changed(flying), "{flying}");
        let v = world.crew.vitals(&CrewId::new(flying)).unwrap();
        assert_eq!(v.stress_bp, 0, "{flying}");
    }
}
