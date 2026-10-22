//! Late entry after an outage (05 section 7.2, wl-kernel 5.13; BL-CLN-15):
//! the front desk enters the business time of a check-in, a check-out and a
//! consumable from the paper sheet. A time up to 30 minutes behind is on
//! time; a later one is flagged and needs a reason; the visit keeps the
//! business time.

mod support;

use clinics::{ChargeLine, Visit, VisitTime};
use renox::chrono::Duration;
use renox::db::{DateTime, now};
use support::desk::desk;

/// A moment as the form gives it, Asia/Dubai wall time.
fn form_time(at: DateTime) -> String {
    (at + Duration::hours(4))
        .format("%Y-%m-%dT%H:%M")
        .to_string()
}

#[renox::test]
async fn a_check_in_a_check_out_and_a_consumable_take_the_business_time_with_the_late_flag() {
    let d = desk().await;
    d.t.prices.set("gauze", support::day(-400), 500);
    let booking = d.booking(&d.consultation, 9).await;
    let visit = d.check_in(&booking).await;
    let path = format!("/clinics/visits/{}/times", visit.id);
    let first_check_in = visit.checked_in_at;

    // Ten minutes behind: on time, no flag, and the visit takes the time.
    let ten = now() - Duration::minutes(10);
    d.post(
        &d.rita,
        &path,
        &[("kind", "check_in"), ("time", form_time(ten).as_str())],
    )
    .await
    .assert_ok();
    let times = VisitTime::of_visit(d.t.db(), visit.id).await.unwrap();
    assert_eq!(times.len(), 1);
    assert!(!times[0].late && times[0].late_entry_id.is_none());
    let moved = d.visit(visit.id).await;
    assert!(moved.checked_in_at < first_check_in);

    // Three hours behind: flagged, so it needs a reason.
    let three = now() - Duration::hours(3);
    d.post(
        &d.rita,
        &path,
        &[("kind", "check_in"), ("time", form_time(three).as_str())],
    )
    .await
    .assert_status(422);
    assert_eq!(
        VisitTime::of_visit(d.t.db(), visit.id).await.unwrap().len(),
        1
    );
    d.post(
        &d.rita,
        &path,
        &[
            ("kind", "check_in"),
            ("time", form_time(three).as_str()),
            ("reason", "The network was down"),
        ],
    )
    .await
    .assert_ok();
    let times = VisitTime::of_visit(d.t.db(), visit.id).await.unwrap();
    assert_eq!(times.len(), 2);
    assert!(times[1].late && times[1].late_entry_id.is_some());
    assert_eq!(
        times[1].late_reason.as_deref(),
        Some("The network was down")
    );
    assert!(d.visit(visit.id).await.checked_in_at < moved.checked_in_at);

    // A time in the future, and a check-out before the visit is over, are
    // refused.
    let ahead = now() + Duration::hours(2);
    d.post(
        &d.rita,
        &path,
        &[("kind", "check_in"), ("time", form_time(ahead).as_str())],
    )
    .await
    .assert_status(422);
    d.post(
        &d.rita,
        &path,
        &[("kind", "check_out"), ("time", form_time(ten).as_str())],
    )
    .await
    .assert_status(422);

    // The consumable: the time of its line, with the line named.
    d.issue(&visit, "gauze", 1).await.assert_ok();
    let line = ChargeLine::active_of(d.t.db(), visit.id)
        .await
        .unwrap()
        .remove(0);
    d.post(
        &d.rita,
        &path,
        &[
            ("kind", "consumable"),
            ("time", form_time(ten).as_str()),
            ("line", line.id.to_string().as_str()),
        ],
    )
    .await
    .assert_ok();
    d.post(
        &d.rita,
        &path,
        &[
            ("kind", "consumable"),
            ("time", form_time(ten).as_str()),
            ("line", "999999"),
        ],
    )
    .await
    .assert_status(422);
    let times = VisitTime::of_visit(d.t.db(), visit.id).await.unwrap();
    assert_eq!(times.last().and_then(|t| t.charge_line_id), Some(line.id));

    // After the visit is completed, the check-out time is entered, and the
    // visit keeps it as its end.
    d.start(&booking).await;
    d.deliver(&visit, "consultation", 1).await.assert_ok();
    d.complete(&booking).await;
    let done: Visit = d.visit(visit.id).await;
    let before_end = done.ended_at.expect("the visit ended");
    let two = now() - Duration::minutes(5);
    d.post(
        &d.rita,
        &path,
        &[("kind", "check_out"), ("time", form_time(two).as_str())],
    )
    .await
    .assert_ok();
    let after = d.visit(visit.id).await;
    assert_ne!(after.ended_at, Some(before_end));
    assert!(after.ended_at.is_some_and(|end| end <= now()));

    // The visit page lists the times.
    d.get(&d.rita, &format!("/clinics/visits/{}", visit.id))
        .await
        .assert_ok()
        .assert_see("The network was down");

    // Someone who is not at the desk cannot enter a time.
    let nora = d.t.user("Nora", &[clinics::permissions::VISITS_VIEW]).await;
    d.post(
        &nora,
        &path,
        &[("kind", "check_in"), ("time", form_time(ten).as_str())],
    )
    .await
    .assert_forbidden();
}
