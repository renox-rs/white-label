//! The call list while the SMS connection is down (05 section 8, BL-CLN-15):
//! the booked appointments of the 24 hours after the moment it went down, per
//! branch, with a *called* tick; a new outage starts a new list.

mod support;

use clinics::call_list::{self, SCREEN};
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::integrations::SMS;
use wl_kernel::{ConnectionStatus, Delivery, IntegrationConnection, IntegrationDef};
use wl_scheduling::service::{self, Actor, CancelRequest};

use support::{PATIENT, at, boot, day};

fn register_sms() {
    IntegrationDef::of(SMS, "notify", |_state, _call| {
        Box::pin(async { Ok(Delivery::Sent) })
    })
    .screens([SCREEN])
    .fallback("The front desk calls the patients.", None)
    .register();
}

/// Puts the SMS connection in `status` as of `when`, as a failure run that
/// lasted long enough would.
async fn set_status(t: &support::Clinic, status: ConnectionStatus, when: DateTime) {
    IntegrationConnection::query()
        .where_(IntegrationConnection::KEY.eq(SMS))
        .update(
            t.db(),
            &[("status", &status), ("status_changed_at", &Some(when))],
        )
        .await
        .unwrap();
}

#[renox::test]
async fn while_sms_is_down_the_front_desk_calls_the_next_24_hours_and_ticks_each_call() {
    register_sms();
    let t = boot().await;
    t.guardians.name(PATIENT.0, "Pat Lane");
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.consultation(&[&doctor]).await;
    let first = t.book(&service, &doctor, at(day(1), 9)).await;
    let second = t.book(&service, &doctor, at(day(1), 11)).await;
    // Outside the 24 hours, and cancelled: neither is on the list.
    let later = t.book(&service, &doctor, at(day(3), 9)).await;
    let cancelled = t.book(&service, &doctor, at(day(1), 13)).await;
    service::cancel(
        t.state(),
        cancelled.id,
        CancelRequest::new("equipment_failure"),
        Actor::staff(1),
        renox::db::now(),
    )
    .await
    .unwrap();

    let rita = t
        .user(
            "Rita",
            &[
                clinics::permissions::VISITS_VIEW,
                clinics::permissions::APPOINTMENTS_CALL,
            ],
        )
        .await;
    let nora = t.user("Nora", &[clinics::permissions::VISITS_VIEW]).await;
    let rita_auth = t.auth(&rita).await;

    // With SMS working there is nothing to call.
    t.app.acting_as(&rita);
    t.app
        .get("/clinics/call-list")
        .await
        .assert_ok()
        .assert_see("Nobody to call");
    assert!(
        call_list::entries(t.state(), &rita_auth, "en")
            .await
            .unwrap()
            .is_empty()
    );

    // The connection goes down 5 hours before the first appointment.
    let down = at(day(1), 4);
    set_status(&t, ConnectionStatus::Down, down).await;
    let rows = call_list::entries(t.state(), &rita_auth, "en")
        .await
        .unwrap();
    let bookings: Vec<i64> = rows.iter().map(|r| r.appointment.booking_id).collect();
    assert_eq!(bookings, [first.id, second.id], "{later:?}");
    assert!(rows.iter().all(|r| r.called.is_none()));
    assert!(rows.iter().all(|r| r.patient == "Pat Lane"));

    // The page shows the way to work meanwhile, and the rows.
    t.app
        .get("/clinics/call-list")
        .await
        .assert_ok()
        .assert_see("The front desk calls the patients.")
        .assert_see("Pat Lane");

    // Someone without the permission sees nothing of it.
    t.app.acting_as(&nora);
    t.app.get("/clinics/call-list").await.assert_forbidden();
    t.app
        .htmx()
        .post(&format!("/clinics/call-list/{}", first.id), &[])
        .await
        .assert_forbidden();

    // The tick is the front desk's, once per appointment.
    t.app.acting_as(&rita);
    t.app
        .htmx()
        .post(&format!("/clinics/call-list/{}", first.id), &[])
        .await
        .assert_ok();
    t.app
        .htmx()
        .post(&format!("/clinics/call-list/{}", first.id), &[])
        .await
        .assert_ok();
    let rows = call_list::entries(t.state(), &rita_auth, "en")
        .await
        .unwrap();
    assert_eq!(
        rows[0].called.as_ref().map(|c| c.called_by),
        Some(Some(rita.id))
    );
    assert!(rows[1].called.is_none());
    assert_eq!(
        clinics::CallTick::query().count(t.db()).await.unwrap(),
        1,
        "ticking twice changes nothing"
    );

    // An appointment that is not on the list cannot be ticked.
    t.app
        .htmx()
        .post(&format!("/clinics/call-list/{}", later.id), &[])
        .await
        .assert_status(422);

    // A new outage starts a new list: the ticks of the old one do not carry.
    set_status(&t, ConnectionStatus::Up, at(day(1), 5)).await;
    assert!(
        call_list::entries(t.state(), &rita_auth, "en")
            .await
            .unwrap()
            .is_empty()
    );
    set_status(&t, ConnectionStatus::Down, at(day(1), 6)).await;
    let rows = call_list::entries(t.state(), &rita_auth, "en")
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.called.is_none()));
}
