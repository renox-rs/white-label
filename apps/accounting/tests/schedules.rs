//! The daily schedules of the app are registered at their times in
//! Asia/Dubai (BL-ACC-02, BL-ACC-03, BL-ACC-04, BL-ACC-06): the engagement
//! letter chase at 09:00, the deadline notice follow-up at 08:00, the
//! request lists at 01:00 and the scheduled ends at 01:30. The overdue run
//! of `ACC.RUL.04` (00:30) is the compliance rule wl-ticketing runs.

use accounting::Accounting;
use renox::Module;
use renox::Registry;
use renox::timezone::Zone;

/// The app's scheduled tasks as (name, local time of the next run).
fn scheduled() -> Vec<(String, String)> {
    let mut registry = Registry::default();
    Accounting.register(&mut registry);
    let dubai: Zone = "Asia/Dubai".parse().expect("a zone");
    registry
        .schedule()
        .upcoming(dubai)
        .into_iter()
        .map(|run| {
            let at = run.at.expect("the task has a next run");
            let local = run.zone.local(at.timestamp());
            (run.name, local.format("%H:%M").to_string())
        })
        .collect()
}

#[test]
fn the_daily_tasks_are_registered_at_their_times() {
    let tasks = scheduled();
    for (name, time) in [
        ("accounting:letters", "09:00"),
        ("accounting:deadline_notices", "08:00"),
        ("accounting:request_lists", "01:00"),
        ("accounting:scheduled_ends", "01:30"),
    ] {
        let found: Vec<_> = tasks.iter().filter(|(n, _)| n == name).collect();
        assert_eq!(found.len(), 1, "{name} is registered once: {tasks:?}");
        assert_eq!(found[0].1, time, "{name} runs at {time} Asia/Dubai");
    }
}
