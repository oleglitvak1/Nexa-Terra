//! The playthrough scripts in `scripts/playthrough/` must run cleanly up to
//! the first step that isn't built yet, and stop there.

use nexa_sim::script::{self, RunError, Step};
use nexa_sim::universe::OfflineUniverse;

/// Run to the end or the first failure: the line it stopped on, and why.
fn play(text: &str) -> Option<(usize, RunError)> {
    let lines = script::parse(text).expect("script parses");
    let mut world = script::start(&lines);
    let mut universe = OfflineUniverse::default();
    for line in &lines {
        if let Err(e) = script::run_step(&mut world, &mut universe, &line.step) {
            return Some((line.number, e));
        }
    }
    None
}

/// Where the script first uses a verb for an unbuilt system.
fn first_unbuilt(text: &str) -> usize {
    script::parse(text)
        .unwrap()
        .iter()
        .find(|l| matches!(l.step, Step::NotBuilt(_)))
        .map(|l| l.number)
        .expect("script reaches milestone 6")
}

fn stops_at_milestone_6(text: &str) {
    match play(text) {
        Some((line, RunError::NotBuilt(_))) => assert_eq!(line, first_unbuilt(text)),
        other => panic!("expected to stop at milestone 6, got {other:?}"),
    }
}

#[test]
fn a_nominal() {
    stops_at_milestone_6(include_str!("../../../scripts/playthrough/a_nominal.nts"));
}

#[test]
fn b_worst_luck() {
    stops_at_milestone_6(include_str!(
        "../../../scripts/playthrough/b_worst_luck.nts"
    ));
}

#[test]
fn c_audit_hypoxia() {
    stops_at_milestone_6(include_str!(
        "../../../scripts/playthrough/c_audit_hypoxia.nts"
    ));
}

#[test]
fn d_prospector() {
    stops_at_milestone_6(include_str!(
        "../../../scripts/playthrough/d_prospector.nts"
    ));
}
