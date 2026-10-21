//! `CLN.ACL.02` (Must not, #349): the receptionist sees the schedule,
//! demographics and balances, but not the visit reason. The notes the booker
//! left with the booking ("chest pain, suspected angina") and the reason code
//! never reach the clinics desk screens: appointments, the appointment, the
//! visit, the policies and the authorisations.

mod support;

use wl_kernel::PartyId;
use wl_scheduling::service::{self, Actor, NewBooking};

use clinics::appointments;
use clinics::policies::{self, NewPolicy};
use support::{PATIENT, at, boot, day, scheduling};

const PAYER: PartyId = PartyId(900);
const REASON: &str = "Chest pain, suspected angina";
const CODE: &str = "I20.9";

fn assert_no_reason(res: &renox::testing::TestResponse, page: &str) {
    let text = res.text();
    for secret in [REASON, "angina", "Chest pain", CODE] {
        assert!(!text.contains(secret), "{page} shows `{secret}`");
    }
}

#[renox::test]
async fn the_desk_screens_show_no_visit_reason() {
    let t = boot().await;
    let rita = t.receptionist().await;
    let bea = t.coordinator().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service_type = t.consultation(&[&doctor]).await;
    let policy = policies::create(
        t.state(),
        None,
        NewPolicy::new(PATIENT, PAYER, "Gold", "M-1", day(-200)).valid_to(day(300)),
    )
    .await
    .unwrap();

    // The booker's notes carry the reason for the visit.
    let mut new = NewBooking::new(PATIENT, service_type.id, &[doctor.id], at(day(1), 9))
        .for_subject(PATIENT.subject());
    new.notes = Some(format!("{REASON} ({CODE})"));
    let booking = service::book(t.state(), new, Actor::staff(1), scheduling::now())
        .await
        .expect("the booking is made");
    assert_eq!(booking.notes.as_deref(), Some("Chest pain, suspected angina (I20.9)"));
    appointments::attach_policy(t.state(), booking.id, policy.id)
        .await
        .unwrap();
    let authorisation = t.authorisation(&policy, "consultation", 2, day(30)).await;

    t.app.acting_as(&rita);
    t.app
        .htmx()
        .post(
            &format!("/clinics/appointments/{}/eligibility", booking.id),
            &[("result", "positive")],
        )
        .await
        .assert_ok();
    for (page, path) in [
        ("appointments", "/clinics/appointments".to_owned()),
        ("appointment", format!("/clinics/appointments/{}", booking.id)),
        ("policies", "/clinics/policies".to_owned()),
    ] {
        let res = t.app.get(&path).await;
        res.assert_ok();
        assert_no_reason(&res, page);
    }

    t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{}/check-in", booking.id),
            &[("x", "y")],
        )
        .await
        .assert_status(303);
    let visit = clinics::Visit::of_booking(t.db(), booking.id)
        .await
        .unwrap()
        .expect("checked in");
    let res = t.app.get(&format!("/clinics/visits/{}", visit.id)).await;
    res.assert_ok();
    assert_no_reason(&res, "visit");

    t.app.acting_as(&bea);
    for (page, path) in [
        ("authorisations", "/clinics/authorisations".to_owned()),
        (
            "authorisation",
            format!("/clinics/authorisations/{}", authorisation.id),
        ),
    ] {
        let res = t.app.get(&path).await;
        res.assert_ok();
        assert_no_reason(&res, page);
    }
}
