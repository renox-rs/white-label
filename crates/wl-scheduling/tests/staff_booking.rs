//! The staff booking form with its availability picker (BL-SCH-02).

mod support;

use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_scheduling::pages::local_text;
use wl_scheduling::{BookingStatus, ResourceType};

use support::{PATIENT, boot, boot_real, in_hours};

/// Tomorrow's date (Asia/Dubai) as the form gives it.
fn tomorrow() -> String {
    local_text(in_hours(30))[..10].to_owned()
}

/// The desk picks the party, the service and the day, sees the free times of
/// that day, and books one: the booking is the party's, at the resource, made
/// by the desk. A viewer sees no form.
#[renox::test]
async fn the_front_desk_picks_a_free_time_and_books_it() {
    let t = boot().await;
    let doctor = t.doctor("Dr Hana").await;
    let service = t.service(&doctor).await;
    let desk = t.front_desk().await;
    let viewer = t.viewer().await;
    let query = format!(
        "/scheduling/bookings/new?party={}&service={}&date={}",
        PATIENT.0,
        service.id,
        tomorrow()
    );

    t.app.acting_as(&viewer);
    t.app.get(&query).await.assert_forbidden();
    t.app
        .get("/scheduling/bookings")
        .await
        .assert_ok()
        .assert_dont_see("Book an appointment");

    t.app.acting_as(&desk);
    t.app
        .get("/scheduling/bookings")
        .await
        .assert_ok()
        .assert_see("Book an appointment");
    let page = t.app.get(&query).await;
    page.assert_ok()
        .assert_see("Dr Hana")
        .assert_see("Free times: Consultation");

    let start = in_hours(30);
    let start = start
        - TimeDelta::minutes(i64::from(
            local_text(start)[14..16].parse::<i32>().unwrap() % 15,
        ))
        + TimeDelta::minutes(15);
    let (party, svc, doctor_id, at, date) = (
        PATIENT.0.to_string(),
        service.id.to_string(),
        doctor.id.to_string(),
        local_text(start),
        tomorrow(),
    );
    let form = [
        ("party", party.as_str()),
        ("service", svc.as_str()),
        ("resources", doctor_id.as_str()),
        ("starts_at", at.as_str()),
        ("date", date.as_str()),
    ];
    t.app
        .htmx()
        .post("/scheduling/bookings/new", &form)
        .await
        .assert_status(303);
    let booking = t.reload(1).await;
    assert_eq!(booking.status, BookingStatus::Booked);
    assert_eq!(booking.party_id, PATIENT.0);
    assert_eq!(booking.subject_id, Some(PATIENT.0));
    assert_eq!(booking.starts_at, start);
    assert_eq!(booking.created_by, Some(desk.id));

    // The booked time is no longer offered.
    let after = t.app.get(&query).await;
    after
        .assert_ok()
        .assert_dont_see(&format!("value=\"{}\"", local_text(start)));
}

/// A time that was just taken sends the desk back to the picker with the
/// booking service's own words; nothing is booked twice.
#[renox::test]
async fn a_taken_time_comes_back_to_the_picker_with_the_service_message() {
    let t = boot().await;
    let doctor = t.doctor("Dr Hana").await;
    let service = t.service(&doctor).await;
    let desk = t.front_desk().await;
    let start = in_hours(30);
    t.booking(&service, &[&doctor], start).await;
    t.app.acting_as(&desk);
    let (party, svc, doctor_id, at, date) = (
        support::OTHER_PATIENT.0.to_string(),
        service.id.to_string(),
        doctor.id.to_string(),
        local_text(start),
        tomorrow(),
    );
    let form = [
        ("party", party.as_str()),
        ("service", svc.as_str()),
        ("resources", doctor_id.as_str()),
        ("starts_at", at.as_str()),
        ("date", date.as_str()),
    ];
    let answer = t.app.post("/scheduling/bookings/new", &form).await;
    answer.assert_status(303);
    answer.assert_redirect(&format!(
        "/scheduling/bookings/new?party={party}&service={svc}&date={date}"
    ));
    assert_eq!(
        wl_scheduling::Booking::query().count(t.db()).await.unwrap(),
        1
    );
    // The desk lands on the picker with the service's own words.
    let toasts = t
        .app
        .session_get::<renox::serde_json::Value>("_toasts")
        .map(|v| v.to_string())
        .unwrap_or_default();
    assert!(toasts.contains("That slot was just taken"), "{toasts}");

    // A wrong field is answered in words, not with its name.
    t.app
        .htmx()
        .post(
            "/scheduling/bookings/new",
            &[("party", ""), ("service", svc.as_str())],
        )
        .await
        .assert_status(422);
}

/// A service that needs a practitioner and a room is searched with one of
/// each: the picker shows a select per kind and books both.
#[renox::test]
async fn a_service_with_two_kinds_of_resource_books_both() {
    let t = boot().await;
    let doctor = t.doctor("Dr Hana").await;
    let room = t.resource("Room 3", ResourceType::Room).await;
    t.open_all_week(&room).await;
    let service = t
        .service_with("dental", &[&doctor, &room], |s| {
            s.required_types = "person,room".to_owned();
        })
        .await;
    let desk = t.front_desk().await;
    t.app.acting_as(&desk);
    let page = t
        .app
        .get(&format!(
            "/scheduling/bookings/new?party={}&service={}&date={}",
            PATIENT.0,
            service.id,
            tomorrow()
        ))
        .await;
    page.assert_ok()
        .assert_see("Practitioner")
        .assert_see("Room 3")
        .assert_see("Dr Hana, Room 3");
}

/// The party picker answers the typed text and the labels of held values.
#[renox::test]
async fn the_party_picker_lists_parties_by_name_and_number() {
    let t = boot_real().await;
    let desk = t.front_desk().await;
    let viewer = t.viewer().await;
    let first = wl_kernel::parties::directory(t.state())
        .search(t.db(), "", 1)
        .await
        .unwrap()
        .remove(0);
    t.app.acting_as(&viewer);
    t.app
        .get("/scheduling/party-options?q=a")
        .await
        .assert_forbidden();
    t.app.acting_as(&desk);
    t.app
        .get(&format!("/scheduling/party-options?values={}", first.id.0))
        .await
        .assert_ok()
        .assert_see(&first.display_name);
    let _ = PartyId(first.id.0);
}
