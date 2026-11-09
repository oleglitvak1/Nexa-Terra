//! The numbers and rules `docs/game-design.md` states, checked against the
//! shipped content and the sim. Each test quotes the rule it checks. Rules for
//! systems that don't exist yet (milestones 6 to 8) are `#[ignore]`d with the
//! expected values in the reason, so they're recorded but don't fail.
//!
//! The GDD's pacing rule (1 h = 5 s at 1x, pause/1x/2x/3x) is presentation
//! and is tested in `nexa-game/src/pacing.rs`.

use nexa_sim::crew::{Assignment, SplitError};
use nexa_sim::flight::{FlightError, Unavailable};
use nexa_sim::hazards::{DelayKind, HazardsDef};
use nexa_sim::machines::{Machine, MachineState, ToleranceBand};
use nexa_sim::parts::PartKind;
use nexa_sim::script;
use nexa_sim::ship::{Ship, ShipDef};
use nexa_sim::universe::{OfflineUniverse, Universe};
use nexa_sim::{SimEvent, World};
use nexa_types::{
    CrewId, HazardId, LaunchBriefing, LaunchContext, SlipEvent, Tick, HOURS_PER_SOL, TICKS_PER_SOL,
};

const DEFAULT_FLIGHT: &[&str] = &["cal", "jonathan", "dax"];

fn ids(list: &[&str]) -> Vec<CrewId> {
    list.iter().map(|s| CrewId::new(*s)).collect()
}

fn split(world: &mut World, flight: &[&str]) -> Result<Vec<SimEvent>, SplitError> {
    world.confirm_split(&script::assignments(&ids(flight)))
}

/// A world ready to launch with the default flight crew.
fn ready(seed: u64) -> World {
    let mut world = World::landfall_seeded(seed);
    split(&mut world, DEFAULT_FLIGHT).unwrap();
    world
}

/// Every launch meets the same briefing.
struct Briefed(LaunchBriefing);

impl Universe for Briefed {
    fn on_launch(&mut self, _ctx: &LaunchContext) -> LaunchBriefing {
        self.0.clone()
    }
}

fn interdictions_on(sols: &[u32]) -> Briefed {
    Briefed(LaunchBriefing {
        events: sols
            .iter()
            .map(|&sol| SlipEvent {
                sol,
                hazard: HazardId::new("pirate_interdiction"),
            })
            .collect(),
    })
}

/// Fly to arrival, answering every hazard with `option`.
fn fly(world: &mut World, option: &str) -> Vec<SimEvent> {
    fly_any(world, &[option])
}

/// Fly to arrival, answering each hazard with the first option the ship can pay.
fn fly_any(world: &mut World, options: &[&str]) -> Vec<SimEvent> {
    let mut events = Vec::new();
    while world.voyage.as_ref().is_some_and(|v| v.in_transit()) {
        if world.voyage.as_ref().unwrap().pending().is_some() {
            let answered = options
                .iter()
                .find_map(|o| world.resolve_hazard(o).ok())
                .expect("some option is affordable");
            events.extend(answered);
        } else {
            let step = world.advance_voyage().unwrap();
            events.extend(step.events);
            events.extend(step.colony);
        }
    }
    events
}

// ---------------------------------------------------------------- Time

#[test]
fn time_one_sol_is_26_hours_and_one_tick_is_one_hour() {
    // GDD 6: "One Sol is 26 hours". CLAUDE.md: one tick = one planetary hour.
    assert_eq!(HOURS_PER_SOL, 26);
    assert_eq!(TICKS_PER_SOL, HOURS_PER_SOL);
    assert_eq!(Tick(25).sol(), 0);
    assert_eq!(Tick(26).sol(), 1);
    assert_eq!(Tick(27).hour_of_sol(), 1);
    let mut world = World::landfall();
    for _ in 0..26 {
        world.tick();
    }
    assert_eq!(world.now, Tick(26));
}

// ---------------------------------------------------------------- Fuel

#[test]
fn fuel_ledger_matches_the_gdd() {
    // GDD 4.B: "1,000 L reaction mass tank. A clean round trip burns 700 L
    // (350 L each way), leaving a 300 L reserve. One Overclock Burn costs 150 L."
    let def = ShipDef::shipped();
    assert_eq!(def.tank_l, 1000);
    assert_eq!(def.start_fuel_l, 1000, "she lands with a full tank");
    assert_eq!(def.leg_burn_l, 350);
    assert_eq!(2 * def.leg_burn_l, 700, "clean round trip");
    assert_eq!(def.tank_l - 2 * def.leg_burn_l, 300, "reserve");
    assert_eq!(def.overclock_burn_l, 150);
    // "15% of tank capacity (150 L)"
    assert_eq!(def.overclock_burn_l * 100 / def.tank_l, 15);
}

#[test]
fn fuel_is_burned_in_whole_liters_at_launch_and_on_an_overclock() {
    let mut world = ready(7);
    world.launch(&mut interdictions_on(&[2])).unwrap();
    assert_eq!(world.ship.fuel_l(), 650);
    world.advance_voyage().unwrap();
    world.advance_voyage().unwrap();
    world.resolve_hazard("overclock").unwrap();
    assert_eq!(world.ship.fuel_l(), 500);
}

#[test]
fn an_overclock_that_would_strand_the_ship_is_unavailable() {
    // GDD 6, prospector dilemma: with too little fuel "a return interdiction
    // must be solved by dumping cargo, paying, bluffing or a risky
    // countermeasure roll". Outbound, the same rule keeps one leg's fuel back:
    // 450 L at launch leaves 100 L, so no burn; dump, pay, bluff, ECM remain.
    let mut world = ready(7);
    world.ship = Ship::new(ShipDef {
        start_fuel_l: 450,
        ..ShipDef::shipped().clone()
    });
    world.launch(&mut interdictions_on(&[1])).unwrap();
    world.advance_voyage().unwrap();
    assert_eq!(world.ship.fuel_l(), 100);
    assert_eq!(
        world.resolve_hazard("overclock"),
        Err(FlightError::Unavailable(Unavailable::NotEnoughFuel))
    );
    for option in ["ecm", "dump", "pay", "bluff"] {
        let mut w = world.clone();
        assert!(w.resolve_hazard(option).is_ok(), "{option}");
    }
}

#[test]
fn an_overclock_must_leave_the_return_legs_fuel() {
    // 650 L after launch: one burn leaves 500 (fine), a second 350 (fine), a
    // third would leave 200 < 350 for the way home.
    let mut world = ready(7);
    world.launch(&mut interdictions_on(&[1, 1, 1])).unwrap();
    world.advance_voyage().unwrap();
    world.resolve_hazard("overclock").unwrap();
    world.resolve_hazard("overclock").unwrap();
    assert_eq!(world.ship.fuel_l(), 350);
    assert_eq!(
        world.resolve_hazard("overclock"),
        Err(FlightError::Unavailable(Unavailable::FuelReserve))
    );
}

#[test]
#[ignore = "milestone 7: prospector dilemma. Expected: at the station with 650 L, \
            giving 200 L leaves 450; the return leg burns 350 -> 100 L at the return \
            interdiction, so overclock is unavailable and the ship still lands with \
            100 L. Keeping it: 300 L on the return leg, overclock available. After \
            an outbound overclock (500 L) at most 150 L can be given."]
fn prospector_gift_leaves_100_l_enough_to_get_home_but_not_to_burn() {
    unimplemented!("needs docking, the prospector dilemma and the return leg")
}

// ---------------------------------------------------------------- Delays

#[test]
fn delay_ranges_and_cap_match_the_gdd() {
    // GDD 4.B: "+5 to +7 Sols (minor) or +7 to +10 Sols (severe drift), and
    // are capped at +12 Sols per round trip".
    let d = &HazardsDef::shipped().delays;
    assert_eq!(d.range(DelayKind::Minor), (5, 7));
    assert_eq!(d.range(DelayKind::Severe), (7, 10));
    assert_eq!(d.random_cap_sols, 12);
}

#[test]
fn drawn_delays_stay_in_their_ranges() {
    // Fly many seeds; every delay a toll draws (paid + cut by the cap) is in
    // range for its kind. ECM partial and fail are Minor, overclock fail Severe.
    let mut seen_minor = (u32::MAX, 0);
    let mut seen_severe = (u32::MAX, 0);
    for seed in 0..400 {
        for (option, kind) in [("ecm", DelayKind::Minor), ("overclock", DelayKind::Severe)] {
            let mut world = ready(seed);
            world.launch(&mut OfflineUniverse::default()).unwrap();
            fly(&mut world, option);
            for r in world.voyage.as_ref().unwrap().resolutions() {
                let drawn = r.paid.delay_sols + r.paid.delay_cut_sols;
                if drawn == 0 {
                    continue;
                }
                let seen = match kind {
                    DelayKind::Minor => &mut seen_minor,
                    DelayKind::Severe => &mut seen_severe,
                };
                *seen = (seen.0.min(drawn), seen.1.max(drawn));
            }
        }
    }
    assert_eq!(seen_minor, (5, 7), "minor delays drawn");
    assert_eq!(seen_severe, (7, 10), "severe delays drawn");
}

#[test]
fn random_delays_never_pass_twelve_sols() {
    // Bad luck alone: three hazards answered with delay-risking checks (two
    // overclocks, then ECM; 15..=30 Sols if all fail) still add at most 12.
    let mut worst = 0;
    for seed in 0..200 {
        let mut world = ready(seed);
        world.launch(&mut interdictions_on(&[1, 2, 3])).unwrap();
        fly_any(&mut world, &["overclock", "ecm", "bluff"]);
        let v = world.voyage.as_ref().unwrap();
        assert!(v.random_delay_sols() <= 12, "seed {seed}");
        assert_eq!(v.delay_sols(), v.random_delay_sols(), "all delay is random");
        worst = worst.max(v.random_delay_sols());
    }
    assert_eq!(worst, 12, "some seed reaches the cap");
}

#[test]
#[ignore = "milestone 7: audits. Expected: a player-caused audit adds 7..=14 Sols on \
            top of the random cap, so random 12 + audit 14 = 26 Sols of delay."]
fn player_caused_audits_are_not_covered_by_the_cap() {
    unimplemented!("needs Station Heat and orbital audits")
}

// ---------------------------------------------------------------- Day budget

#[test]
fn nominal_outbound_arrives_on_sol_15() {
    // GDD 6 day budget: ground 10, flight 5 out. (Station 3 + 5 back -> Sol 23
    // needs milestones 7 and 8.)
    let mut world = ready(7);
    while world.now.sol() < 10 {
        world.tick();
    }
    world.launch(&mut OfflineUniverse::default()).unwrap();
    fly(&mut world, "pay"); // certain, no delay
    assert_eq!(world.now.sol(), 15);
    assert_eq!(world.now.hour_of_sol(), 0);
}

#[test]
fn worst_random_outbound_lands_well_inside_the_deadline() {
    // Sol 10 launch + 5 + 12 (cap) = Sol 27 on arrival at the barge.
    let mut world = ready(1);
    while world.now.sol() < 10 {
        world.tick();
    }
    world.launch(&mut interdictions_on(&[1, 2, 3])).unwrap();
    let mut seed_world = None;
    for seed in 0..200 {
        let mut w = world.clone();
        w.rng = nexa_sim::rng::SimRng::new(seed);
        fly_any(&mut w, &["overclock", "ecm", "bluff"]);
        if w.voyage.as_ref().unwrap().random_delay_sols() == 12 {
            seed_world = Some(w);
            break;
        }
    }
    let w = seed_world.expect("a seed reaches the cap");
    assert_eq!(w.now.sol(), 27);
}

#[test]
#[ignore = "milestone 8: return leg, station Sols. Expected: nominal run (10 ground, 5 \
            out, 3 docked, 5 back) lands on Sol 23; with the +12 cap Sol 35."]
fn nominal_round_trip_lands_on_sol_23() {
    unimplemented!("needs docking and the return leg")
}

#[test]
#[ignore = "milestones 6-8: audits + hypoxia. Expected: 35 + a 14-Sol audit lands on Sol \
            49 and the colony is lost."]
fn a_14_sol_audit_after_max_bad_luck_loses_the_colony_on_sol_49() {
    unimplemented!("needs audits and the hypoxia fail state")
}

// ---------------------------------------------------------------- Hypoxia

#[test]
#[ignore = "milestone 6: Base Hypoxia. Expected: Sols 45-47 degraded (half production, O2 \
            candles burn power), Sol 48 ground crew unconscious and production stops, \
            Sol 49+ colony lost and the autopsy log printed."]
fn hypoxia_runs_degraded_then_unconscious_then_lost() {
    unimplemented!("needs the scrubber clock and the hypoxia fail state")
}

// ---------------------------------------------------------------- Machines

#[test]
fn no_wear_inside_the_nominal_band() {
    // GDD 4.A: "Running a machine within its Nominal Band produces 0% wear."
    let band = ToleranceBand::new(500, 4000);
    for temp in [500, 2000, 4000] {
        let mut m = Machine::new(band, temp);
        for _ in 0..1000 {
            m.wear_for_an_hour(500);
        }
        assert_eq!(
            (m.state(), m.wear_bp()),
            (MachineState::Nominal, 0),
            "{temp}"
        );
    }
}

#[test]
fn overheat_burns_out_the_filament_and_freeze_cracks_the_seal() {
    // GDD 4.A diagram: overheat -> Burnout -> new Filament; freeze -> seal
    // cracks -> new Viton Seal. The sim calls the frozen failure `Offline`.
    let band = ToleranceBand::new(500, 4000);
    let mut hot = Machine::new(band, 4001);
    let mut cold = Machine::new(band, 499);
    assert_eq!(hot.state(), MachineState::Overheat);
    assert_eq!(cold.state(), MachineState::Freeze);
    for _ in 0..10_000 {
        hot.wear_for_an_hour(100);
        cold.wear_for_an_hour(100);
    }
    assert_eq!(hot.state(), MachineState::Burnout);
    assert_eq!(hot.part_needed(), Some(PartKind::TungstenFilament));
    assert_eq!(cold.state(), MachineState::Offline);
    assert_eq!(cold.part_needed(), Some(PartKind::VitonSeal));
}

#[test]
fn caretaker_keeps_the_habs_nominal_with_no_wear_to_sol_45() {
    // architecture.md: the caretaker preset "keeps both habs nominal with no
    // added wear through Sol 45". hab_01 lands damaged (start_wear_bp 4000).
    let mut world = World::landfall();
    let start = world.colony.clone();
    script::load_preset(&mut world, "caretaker").unwrap();
    while world.now.sol() < 45 {
        world.tick();
    }
    for hab in ["hab_01", "hab_02"] {
        let m = world
            .colony
            .module(&nexa_types::ModuleId::new(hab))
            .and_then(|m| m.machine())
            .unwrap();
        assert_eq!(m.state(), MachineState::Nominal, "{hab}");
        let landed = start
            .module(&nexa_types::ModuleId::new(hab))
            .and_then(|m| m.machine())
            .unwrap();
        assert_eq!(m.wear_bp(), landed.wear_bp(), "{hab} gained wear");
    }
}

// ---------------------------------------------------------------- The Split

#[test]
fn every_split_obeys_the_two_rules() {
    // GDD 8.3: "at least one pilot aboard, at least one adult with Maya".
    // Checked for all 64 ways to split six crew.
    let base = World::landfall();
    let members = base.crew.members().to_vec();
    let pilot_min = base.crew.roster().tuning.pilot_min;
    for mask in 0u32..64 {
        let flying: Vec<&str> = members
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, c)| c.id.as_str())
            .collect();
        let on = |id: &CrewId| flying.contains(&id.as_str());
        let pilot = members
            .iter()
            .any(|c| on(&c.id) && c.skill(nexa_sim::crew::Skill::Piloting) >= pilot_min);
        let child_covered = members.iter().filter(|c| !c.adult).all(|child| {
            members
                .iter()
                .any(|a| a.adult && a.id != child.id && on(&a.id) == on(&child.id))
        });
        let mut world = base.clone();
        let result = split(&mut world, &flying);
        assert_eq!(
            result.is_ok(),
            pilot && child_covered,
            "flying {flying:?}: {result:?}"
        );
        if !pilot {
            assert_eq!(result, Err(SplitError::NoQualifiedPilot), "{flying:?}");
        }
    }
}

#[test]
fn oron_counts_as_the_adult_with_maya_and_the_split_holds_while_away() {
    let mut world = World::landfall();
    assert!(split(&mut world, &["cal", "jonathan", "elena", "dax"]).is_ok());
    assert_eq!(
        world.crew.assignment(&CrewId::new("oron")),
        Some(Assignment::Ground)
    );
    world.launch(&mut OfflineUniverse::default()).unwrap();
    assert_eq!(split(&mut world, &["cal"]), Err(SplitError::ShipAway));
}

// ---------------------------------------------------------------- Orders while away

#[test]
fn standing_orders_run_while_the_flight_crew_is_away() {
    // GDD 6: "the colony runs every hour of those Sols, so the Standing Orders
    // and the Ground crew's stress play out while the ship is away."
    // The catch-up on arrival must equal running the same hours on the ground.
    let mut away = ready(3);
    script::load_preset(&mut away, "caretaker").unwrap();
    let mut home = away.clone();

    away.launch(&mut OfflineUniverse::default()).unwrap();
    let before = away.now;
    assert!(away.tick().is_empty(), "the colony is held in transit");
    assert_eq!(away.now, before);
    let events = fly(&mut away, "pay");
    let sols = away.voyage.as_ref().unwrap().sols_total();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SimEvent::OrderFired { .. })),
        "the Caretaker acted while the crew was away"
    );

    for _ in 0..u64::from(sols) * TICKS_PER_SOL {
        home.tick();
    }
    assert_eq!(away.now, home.now);
    assert_eq!(away.colony, home.colony);
    assert_eq!(away.orders, home.orders);
    for c in away.crew.members() {
        assert_eq!(away.crew.vitals(&c.id), home.crew.vitals(&c.id), "{}", c.id);
    }
}

#[test]
fn the_same_script_gives_the_same_world() {
    // CLAUDE.md principle 3: same seed + same commands = same result.
    let text = "seed 99\norders caretaker\nrun 4\nsplit cal,jonathan,dax\nlaunch\nfly bluff";
    let play = || {
        let lines = script::parse(text).unwrap();
        let mut world = script::start(&lines);
        let mut universe = OfflineUniverse::default();
        for line in &lines {
            script::run_step(&mut world, &mut universe, &line.step).unwrap();
        }
        world
    };
    assert_eq!(play(), play());
}
