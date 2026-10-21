//! BL-MPW-01 (#415) and BL-MPW-06 (#420): the daily tasks are registered at the
//! times of the spec in the zone of Asia/Dubai: the credit hold (`MPW.RUL.10`)
//! at 02:00 and the end of the contracts at 00:05, whatever the zone of the app.

use manpower::{CONTRACT_END_TASK, CREDIT_HOLD_TASK, Manpower};
use renox::Module;
use renox::timezone::Zone;

/// The time of day of the next run of `task`, in UTC, and its zone.
fn next_run(task: &str) -> (String, String) {
    let mut registry = renox::Registry::default();
    Manpower.register(&mut registry);
    let runs = registry.schedule().upcoming(Zone::UTC);
    let run = runs
        .iter()
        .find(|run| run.name == task)
        .unwrap_or_else(|| panic!("{task} is registered"));
    let at = run.at.expect("it has a next run");
    (at.format("%H:%M").to_string(), run.zone.to_string())
}

/// AC1 of MPW-01: the credit hold runs daily at 02:00 Asia/Dubai (22:00 UTC).
#[test]
fn the_credit_hold_runs_at_two_in_the_morning_in_dubai() {
    assert_eq!(
        next_run(CREDIT_HOLD_TASK),
        ("22:00".to_owned(), "Asia/Dubai".to_owned())
    );
}

/// AC1 of MPW-06: the contracts past their end date end at 00:05 Asia/Dubai
/// (20:05 UTC the evening before).
#[test]
fn the_contract_end_runs_at_five_past_midnight_in_dubai() {
    assert_eq!(
        next_run(CONTRACT_END_TASK),
        ("20:05".to_owned(), "Asia/Dubai".to_owned())
    );
}
