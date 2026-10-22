//! The staff pages and actions of bookings (wl-scheduling §5.1).

mod support;

use renox::chrono::TimeDelta;
use wl_kernel::calendar::BusinessCalendar;
use wl_scheduling::BookingStatus;

use support::{PATIENT, boot, in_hours};

/// The local text of an instant, as a form field.
fn local(at: renox::db::DateTime) -> String {
    BusinessCalendar::default()
        .local(at)
        .format("%Y-%m-%dT%H:%M")
        .to_string()
}

/// The front desk books through the page; a viewer may look but not book.
#[renox::test]
async fn the_front_desk_books_and_a_viewer_cannot() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let desk = t.front_desk().await;
    let viewer = t.viewer().await;
    let start = in_hours(48);
    let (party, svc, resources, at) = (
        PATIENT.0.to_string(),
        service.id.to_string(),
        doctor.id.to_string(),
        local(start),
    );
    let form = [
        ("party", party.as_str()),
        ("service", svc.as_str()),
        ("resources", resources.as_str()),
        ("starts_at", at.as_str()),
    ];

    t.app.acting_as(&viewer);
    t.app
        .htmx()
        .post("/scheduling/bookings", &form)
        .await
        .assert_forbidden();

    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post("/scheduling/bookings", &form)
        .await
        .assert_status(303);
    let booking = t.reload(1).await;
    assert_eq!(booking.status, BookingStatus::Booked);
    assert_eq!(booking.starts_at, start);
    assert_eq!(booking.created_by, Some(desk.id));

    // The same slot again is refused as just taken.
    t.app
        .htmx()
        .post("/scheduling/bookings", &form)
        .await
        .assert_status(409);

    // The pages show it.
    t.app
        .get("/scheduling/bookings")
        .await
        .assert_ok()
        .assert_see(&booking.number);
    t.app
        .get(&format!("/scheduling/bookings/{}", booking.id))
        .await
        .assert_ok()
        .assert_see("Consultation");
    t.app.acting_as(&viewer);
    t.app
        .get(&format!("/scheduling/bookings/{}", booking.id))
        .await
        .assert_ok();
}

/// Cancel, no-show and check-in run from the booking page's actions.
#[renox::test]
async fn the_actions_run_the_transitions() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t
        .service_with("visit", &[&doctor], |s| s.check_in_required = true)
        .await;
    let desk = t.front_desk().await;
    let first = t.booking(&service, &[&doctor], in_hours(48)).await;
    let second = t
        .booking(&service, &[&doctor], in_hours(48) + TimeDelta::hours(1))
        .await;
    t.app.acting_as(&desk);

    t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{}/check-in", first.id),
            &[("x", "y")],
        )
        .await
        .assert_status(303);
    assert_eq!(t.reload(first.id).await.status, BookingStatus::CheckedIn);

    t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{}/cancel", second.id),
            &[("reason_code", "")],
        )
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{}/cancel", second.id),
            &[("reason_code", "site_closed")],
        )
        .await
        .assert_status(303);
    let cancelled = t.reload(second.id).await;
    assert_eq!(cancelled.status, BookingStatus::Cancelled);
    assert_eq!(cancelled.cancel_code.as_deref(), Some("site_closed"));
}
