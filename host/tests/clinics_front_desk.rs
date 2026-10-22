//! BL-CLN-15 (#363): the front desk's flows through the real host of the demo
//! clinic with real bookings: the call list while SMS is down (rows, the
//! 24-hour window, the tick, a double click), the late entry of a time from
//! the paper sheet, and a booking with a deposit that a walk-out, a no-show
//! and a waiver settle (05 §7.2, §8).

mod support;

use clinics::{CallTick, Visit, VisitTime};
use host::roles::{self, CLINIC_MANAGER, RECEPTIONIST};
use renox::chrono::Duration;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::testing::TestApp;
use support::{clinic_booking, clinic_doctor, clinic_service, clinics_booted, person, tenant};
use wl_kernel::integrations::{self, ConnectionStatus, IntegrationConnection};
use wl_payments::Refund;
use wl_payments::refund::RefundStatus;
use wl_scheduling::BookingStatus;
use wl_scheduling::service::{self, Actor};

/// Puts the SMS connection in `status` as of `when`.
async fn set_sms(app: &TestApp, status: ConnectionStatus, when: DateTime) {
    IntegrationConnection::query()
        .where_(IntegrationConnection::KEY.eq(integrations::SMS))
        .update(
            app.db(),
            &[("status", &status), ("status_changed_at", &Some(when))],
        )
        .await
        .unwrap();
}

/// A moment as the late-entry form gives it, Asia/Dubai wall time.
fn form_time(at: DateTime) -> String {
    (at + Duration::hours(4))
        .format("%Y-%m-%dT%H:%M")
        .to_string()
}

/// Done when (connections), 1: with SMS `down` the front desk sees the call
/// list of the booked appointments of the next 24 hours, per branch, with a
/// *called* tick; a cancelled appointment, one that has started and one
/// beyond the window are not on it; an outage that lasts more than a day
/// keeps listing the next 24 hours; ticking twice (a double click) is one
/// tick.
#[renox::test]
async fn with_sms_down_the_front_desk_calls_the_next_24_hours_and_ticks_each_call() {
    let app = clinics_booted().await;
    let rita = person(&app, "Rita", RECEPTIONIST).await;
    let doctor = clinic_doctor(&app, "Dr A").await;
    let service = clinic_service(&app, "checkup", 0, &doctor).await;
    let pat = tenant(&app, "Pat Lane").await;
    let first = clinic_booking(&app, pat, &service, &doctor, now() + Duration::hours(2)).await;
    let second = clinic_booking(&app, pat, &service, &doctor, now() + Duration::hours(22)).await;
    let beyond = clinic_booking(&app, pat, &service, &doctor, now() + Duration::hours(30)).await;
    let cancelled = clinic_booking(&app, pat, &service, &doctor, now() + Duration::hours(5)).await;
    app.acting_as(&rita);
    app.htmx()
        .post(
            &format!("/scheduling/bookings/{}/cancel", cancelled.id),
            &[("reason_code", "equipment_failure")],
        )
        .await
        .assert_status(303);

    // With SMS working there is nothing to call.
    app.get("/clinics/call-list")
        .await
        .assert_ok()
        .assert_see("Nobody to call");

    // SMS went down 30 hours ago: the list is the next 24 hours from now.
    set_sms(&app, ConnectionStatus::Down, now() - Duration::hours(30)).await;
    let action = |id: i64| format!("/clinics/call-list/{id}\"");
    let page = app.get("/clinics/call-list").await;
    page.assert_ok()
        .assert_see("Phone the patients on the call list")
        .assert_see("Pat Lane")
        .assert_see(&action(first.id))
        .assert_see(&action(second.id))
        .assert_dont_see(&action(beyond.id))
        .assert_dont_see(&action(cancelled.id));

    // The tick is the front desk's; a double click is one tick, no error.
    let tick = format!("/clinics/call-list/{}", first.id);
    let both = renox::tokio::join!(
        app.htmx().post(&tick, &[("x", "y")]),
        app.htmx().post(&tick, &[("x", "y")]),
    );
    both.0.assert_ok();
    both.1.assert_ok();
    assert_eq!(CallTick::query().count(app.db()).await.unwrap(), 1);
    app.htmx()
        .post(&format!("/clinics/call-list/{}", beyond.id), &[("x", "y")])
        .await
        .assert_status(422);

    // Someone who is not at the desk is refused.
    let member = person(&app, "Mia", roles::MEMBER).await;
    app.acting_as(&member);
    app.get("/clinics/call-list").await.assert_forbidden();

    // The ticked row has no tick any more; the patient who arrives is no
    // longer on the list.
    app.acting_as(&rita);
    app.get("/clinics/call-list")
        .await
        .assert_dont_see(&action(first.id))
        .assert_see(&action(second.id));
    app.htmx()
        .post(
            &format!("/scheduling/bookings/{}/check-in", second.id),
            &[("x", "y")],
        )
        .await
        .assert_status(303);
    app.get("/clinics/call-list")
        .await
        .assert_ok()
        .assert_dont_see(&action(second.id));
}

/// Done when (late entry): a time from the paper sheet is entered through
/// the visit's route; up to 30 minutes behind is on time, a later one is
/// flagged and needs a reason, a time ahead is refused, and the visit keeps
/// the business time.
#[renox::test]
async fn a_late_check_in_time_is_flagged_and_needs_a_reason_through_the_host() {
    let app = clinics_booted().await;
    let rita = person(&app, "Rita", RECEPTIONIST).await;
    let doctor = clinic_doctor(&app, "Dr A").await;
    let service = clinic_service(&app, "checkup", 0, &doctor).await;
    let pat = tenant(&app, "Pat Lane").await;
    let booking = clinic_booking(&app, pat, &service, &doctor, now() + Duration::hours(1)).await;
    app.acting_as(&rita);
    app.htmx()
        .post(
            &format!("/scheduling/bookings/{}/check-in", booking.id),
            &[("x", "y")],
        )
        .await
        .assert_status(303);
    let visit = Visit::of_booking(app.db(), booking.id)
        .await
        .unwrap()
        .expect("the patient is checked in");
    let path = format!("/clinics/visits/{}/times", visit.id);

    let ten = now() - Duration::minutes(10);
    app.htmx()
        .post(
            &path,
            &[("kind", "check_in"), ("time", form_time(ten).as_str())],
        )
        .await
        .assert_ok();
    let times = VisitTime::of_visit(app.db(), visit.id).await.unwrap();
    assert_eq!(times.len(), 1);
    assert!(!times[0].late && times[0].late_entry_id.is_none());

    let three = now() - Duration::hours(3);
    app.htmx()
        .post(
            &path,
            &[("kind", "check_in"), ("time", form_time(three).as_str())],
        )
        .await
        .assert_status(422);
    app.htmx()
        .post(
            &path,
            &[
                ("kind", "check_in"),
                ("time", form_time(three).as_str()),
                ("reason", "The network was down"),
            ],
        )
        .await
        .assert_ok();
    let times = VisitTime::of_visit(app.db(), visit.id).await.unwrap();
    assert_eq!(times.len(), 2);
    assert!(times[1].late && times[1].late_entry_id.is_some());
    assert_eq!(
        times[1].late_reason.as_deref(),
        Some("The network was down")
    );

    let ahead = now() + Duration::hours(2);
    app.htmx()
        .post(
            &path,
            &[("kind", "check_in"), ("time", form_time(ahead).as_str())],
        )
        .await
        .assert_status(422);
    let nora = person(&app, "Nora", roles::MEMBER).await;
    app.acting_as(&nora);
    app.htmx()
        .post(
            &path,
            &[("kind", "check_in"), ("time", form_time(ten).as_str())],
        )
        .await
        .assert_forbidden();
}

/// Done when (deposits), 1 and 2: a service type added after the seed starts
/// with the clinic's walk-out setting (the remainder of the deposit is
/// refunded, never forfeited), whatever the module's own default; a patient
/// who walks out after check-in gets the deposit back through a real refund;
/// a no-show forfeits it, and only the clinic manager waives the forfeiture
/// (`bookings.waive_deposit`), which asks the real refund of the remainder.
#[renox::test]
async fn a_booking_with_a_deposit_refunds_a_walk_out_and_the_manager_waives_a_no_show() {
    let app = clinics_booted().await;
    let rita = person(&app, "Rita", RECEPTIONIST).await;
    let mona = person(&app, "Mona", CLINIC_MANAGER).await;
    let doctor = clinic_doctor(&app, "Dr A").await;
    assert!(
        wl_scheduling::ServiceType::new("x", "x", 30).walk_out_forfeits,
        "the module forfeits by default"
    );
    let service = clinic_service(&app, "checkup", 20_000, &doctor).await;
    assert!(!service.walk_out_forfeits, "a type added later refunds");
    let pat = tenant(&app, "Pat Lane").await;

    // The walk-out: checked in, then gone.
    let walked = clinic_booking(&app, pat, &service, &doctor, now() + Duration::hours(1)).await;
    assert_eq!(walked.deposit_held(), 20_000);
    app.acting_as(&rita);
    app.htmx()
        .post(
            &format!("/scheduling/bookings/{}/check-in", walked.id),
            &[("x", "y")],
        )
        .await
        .assert_status(303);
    // Nothing was done: the desk says so before the walk-out is cancelled.
    let visit = Visit::of_booking(app.db(), walked.id)
        .await
        .unwrap()
        .expect("checked in");
    app.htmx()
        .post(&format!("/clinics/visits/{}/none", visit.id), &[("x", "y")])
        .await
        .assert_ok();
    app.htmx()
        .post(
            &format!("/scheduling/bookings/{}/cancel", walked.id),
            &[("reason_code", "walked_out")],
        )
        .await
        .assert_status(303);
    let done = wl_scheduling::Booking::find_or_404(app.db(), walked.id)
        .await
        .unwrap();
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert_eq!(done.deposit_forfeited, 0, "nothing is forfeited");
    assert_eq!(done.deposit_applied + done.deposit_refunded, 20_000);
    let refunds = Refund::all(app.db()).await.unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, done.deposit_refunded);
    assert_eq!(refunds[0].status, RefundStatus::Requested);
    assert_eq!(refunds[0].source_kind, "booking");
    assert_eq!(refunds[0].source_id, walked.id);

    // The no-show forfeits the deposit; the receptionist cannot waive it, the
    // manager can.
    let start = now() + Duration::hours(3);
    let missed = clinic_booking(&app, pat, &service, &doctor, start).await;
    service::mark_no_show(
        app.state(),
        missed.id,
        Actor::staff(1),
        start + Duration::minutes(20),
    )
    .await
    .expect("the grace period is over");
    let forfeited = wl_scheduling::Booking::find_or_404(app.db(), missed.id)
        .await
        .unwrap();
    assert_eq!(forfeited.deposit_forfeited, 20_000);
    let waive = format!("/scheduling/bookings/{}/waive-deposit", missed.id);
    app.acting_as(&rita);
    app.htmx()
        .post(&waive, &[("reason", "goodwill")])
        .await
        .assert_forbidden();
    app.acting_as(&mona);
    app.htmx()
        .post(&waive, &[("reason", "goodwill")])
        .await
        .assert_status(303);
    let waived = wl_scheduling::Booking::find_or_404(app.db(), missed.id)
        .await
        .unwrap();
    assert_eq!(waived.deposit_forfeited, 0);
    assert_eq!(waived.deposit_refunded, 20_000);
    assert_eq!(waived.deposit_waived_by, Some(mona.id));
    let refunds = Refund::all(app.db()).await.unwrap();
    assert_eq!(refunds.len(), 2);
    assert!(
        refunds
            .iter()
            .any(|r| r.source_id == missed.id && r.amount == 20_000)
    );
}
