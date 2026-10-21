//! BL-LOG-21 (#319), 03 §4.2 rows *Customs hold...* and *Mirsal or ATLP
//! down*, §7.1 *Freight job* rows `hold` and `release`, §7.2 `LOG.RUL.09`:
//! the documentation officer holds a job that is in transit, in customs
//! clearance or ready for delivery with a reason and a note; the job keeps
//! the state before the hold and `release` returns to exactly that state; the
//! client reads the reason in the portal; no ETA alert is raised while the
//! job is on hold and the hold periods are left out of on-time measures.

mod support;

use logistics::freight_hold::{self, FreightHold};
use logistics::freight_machine::{FreightInput, RELEASE_IN_TRANSIT};
use logistics::notices::{CLIENT_NOTICE, STAFF_NOTICE};
use logistics::{FreightJob, FreightStatus};
use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::documents::today;

use support::{World, boot};

const NOTE: &str = "physical inspection at Jebel Ali";

fn day(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

/// Sets the ETA of `job`.
async fn set_eta(w: &World, job: &FreightJob, eta: NaiveDate) {
    let eta = Some(eta);
    FreightJob::query()
        .where_(FreightJob::ID.eq(job.id))
        .update(w.db(), &[("eta", &eta)])
        .await
        .unwrap();
}

/// AC1: given a job in `customs_clearance`, the documentation officer holds
/// it with reason *customs hold or inspection* and the note; it is
/// `on_hold`, the stored state is `customs_clearance`, the client and the
/// documentation officer are notified, and the portal shows "Customs hold or
/// inspection" with the note.
#[renox::test]
async fn a_job_in_customs_clearance_is_held_with_its_reason_and_the_portal_shows_it() {
    let w = boot().await;
    let job = w.in_customs().await;
    let before = w.notices.of_type(CLIENT_NOTICE).len();

    w.job_action(
        &w.docs,
        &job,
        "hold",
        &[("reason", "customs_hold_or_inspection"), ("note", NOTE)],
    )
    .await
    .assert_status(303);

    let held = w.job(job.id).await;
    assert_eq!(held.status, FreightStatus::OnHold);
    assert_eq!(held.held_from.as_deref(), Some("customs_clearance"));
    assert_eq!(held.hold_reason.as_deref(), Some("customs_hold_or_inspection"));
    assert_eq!(held.hold_note.as_deref(), Some(NOTE));
    let periods = freight_hold::holds_of(w.db(), job.id).await.unwrap();
    assert_eq!(periods.len(), 1);
    assert_eq!(periods[0].from_state, "customs_clearance");
    assert!(periods[0].ended_at.is_none());

    let client = w.notices.of_type(CLIENT_NOTICE);
    assert_eq!(client.len(), before + 1);
    assert!(
        client
            .last()
            .unwrap()
            .data["title"]
            .as_str()
            .unwrap()
            .contains("Customs hold or inspection")
    );
    assert!(w.notices.users_of_type(STAFF_NOTICE).contains(&w.docs.id));

    // The client reads the reason label and the note in the portal.
    let page = w.get(&w.noor, &format!("/freight/{}", job.id)).await;
    page.assert_ok();
    page.assert_see("Customs hold or inspection");
    page.assert_see(NOTE);
    w.get(&w.farah, &format!("/freight/{}", job.id))
        .await
        .assert_forbidden();
}

/// AC2: `release` with the note returns the job to `customs_clearance` and
/// notifies the client; without a note it is refused.
#[renox::test]
async fn release_returns_the_job_to_the_state_before_the_hold_and_needs_a_note() {
    let w = boot().await;
    let job = w.in_customs().await;
    w.job_action(
        &w.docs,
        &job,
        "hold",
        &[("reason", "customs_hold_or_inspection"), ("note", NOTE)],
    )
    .await
    .assert_status(303);
    let told = w.notices.of_type(CLIENT_NOTICE).len();

    w.job_action(&w.docs, &job, "release", &[("note", " ")])
        .await
        .assert_status(422)
        .assert_see("Note how the reason was resolved");
    assert_eq!(w.job(job.id).await.status, FreightStatus::OnHold);
    w.job_action(&w.dispatcher, &job, "release", &[("note", "inspection passed")])
        .await
        .assert_forbidden();

    w.job_action(
        &w.docs,
        &job,
        "release",
        &[("note", "inspection passed 2026-11-06")],
    )
    .await
    .assert_status(303);
    let released = w.job(job.id).await;
    assert_eq!(released.status, FreightStatus::CustomsClearance);
    assert_eq!(w.notices.of_type(CLIENT_NOTICE).len(), told + 1);
    let periods = freight_hold::holds_of(w.db(), job.id).await.unwrap();
    assert_eq!(periods.len(), 1);
    assert!(periods[0].ended_at.is_some());
    assert_eq!(
        periods[0].release_note.as_deref(),
        Some("inspection passed 2026-11-06")
    );
    assert_eq!(periods[0].released_by, Some(w.docs.id));
    // Held again, it returns to where it is then.
    w.job_action(&w.docs, &job, "release", &[("note", "again")])
        .await
        .assert_status(422);
}

/// Each state a job can be held from comes back as it was.
#[renox::test]
async fn a_job_returns_to_in_transit_or_ready_for_delivery_as_it_was() {
    let w = boot().await;
    let transit = w.in_transit().await;
    let ready = w.ready_for_delivery().await;
    for (job, back) in [
        (&transit, FreightStatus::InTransit),
        (&ready, FreightStatus::ReadyForDelivery),
    ] {
        w.job_action(&w.docs, job, "hold", &[("reason", "duty_unpaid"), ("note", "")])
            .await
            .assert_status(303);
        assert_eq!(w.job(job.id).await.status, FreightStatus::OnHold);
        w.job_action(&w.docs, job, "release", &[("note", "duty paid")])
            .await
            .assert_status(303);
        assert_eq!(w.job(job.id).await.status, back);
    }
}

/// `release` never goes to another state than the stored one: the machine
/// refuses the twin of another state.
#[renox::test]
async fn release_cannot_go_to_a_state_other_than_the_stored_one() {
    let w = boot().await;
    let job = w.in_customs().await;
    w.job_action(&w.docs, &job, "hold", &[("reason", "documents_missing"), ("note", "BL copy")])
        .await
        .assert_status(303);
    let held = w.job(job.id).await;
    let outcome = held
        .transition(RELEASE_IN_TRANSIT)
        .by(&w.docs)
        .input(&FreightInput {
            note: Some("resolved".into()),
            ..FreightInput::default()
        })
        .run(w.state())
        .await;
    assert!(outcome.is_err());
    assert_eq!(w.job(job.id).await.status, FreightStatus::OnHold);
}

/// A job in `booked` or `documents_pending` cannot be held, an unknown reason
/// is refused, and only `jobs.manage` holds.
#[renox::test]
async fn a_booked_job_cannot_be_held_and_the_reason_must_be_known() {
    let w = boot().await;
    let booked = w.open_job().await;
    w.job_action(&w.docs, &booked, "hold", &[("reason", "documents_missing"), ("note", "")])
        .await
        .assert_status(422);
    assert_eq!(w.job(booked.id).await.status, FreightStatus::Booked);
    w.job_action(&w.docs, &booked, "request-documents", &[("missing", "invoice")])
        .await
        .assert_status(303);
    w.job_action(&w.docs, &booked, "hold", &[("reason", "documents_missing"), ("note", "")])
        .await
        .assert_status(422);
    assert_eq!(
        w.job(booked.id).await.status,
        FreightStatus::DocumentsPending
    );

    let transit = w.in_transit().await;
    w.job_action(&w.docs, &transit, "hold", &[("reason", "bad weather"), ("note", "")])
        .await
        .assert_status(422);
    w.job_action(&w.docs, &transit, "hold", &[("reason", "")])
        .await
        .assert_status(422);
    w.job_action(&w.dispatcher, &transit, "hold", &[("reason", "duty_unpaid")])
        .await
        .assert_forbidden();
    assert_eq!(w.job(transit.id).await.status, FreightStatus::InTransit);
}

/// A job already on hold is not held again: a second reason is a note on the
/// open hold.
#[renox::test]
async fn a_second_reason_is_a_note_on_the_open_hold() {
    let w = boot().await;
    let job = w.in_transit().await;
    w.job_action(&w.docs, &job, "hold", &[("reason", "customs_hold_or_inspection"), ("note", NOTE)])
        .await
        .assert_status(303);
    w.job_action(&w.docs, &job, "hold", &[("reason", "duty_unpaid"), ("note", "x")])
        .await
        .assert_status(422);

    w.job_action(
        &w.docs,
        &job,
        "hold-note",
        &[("reason", "duty_unpaid"), ("note", "AED 250 to pay")],
    )
    .await
    .assert_status(303);
    let held = w.job(job.id).await;
    assert_eq!(held.status, FreightStatus::OnHold);
    assert_eq!(held.hold_reason.as_deref(), Some("customs_hold_or_inspection"));
    let note = held.hold_note.unwrap();
    assert!(note.contains(NOTE), "{note}");
    assert!(note.contains("Customs duty unpaid: AED 250 to pay"), "{note}");
    let periods = FreightHold::query().get(w.db()).await.unwrap();
    assert_eq!(periods.len(), 1);
    // A job that is not on hold has no hold to add to.
    let moving = w.in_customs().await;
    w.job_action(
        &w.docs,
        &moving,
        "hold-note",
        &[("reason", "duty_unpaid"), ("note", "x")],
    )
    .await
    .assert_status(422);
}

/// AC3: a job in `in_transit` held for *customs platform down* whose ETA
/// passes raises no `LOG.RUL.09` alert; after the release the alert applies
/// to the ETA as recorded.
#[renox::test]
async fn the_eta_passing_while_on_hold_raises_no_alert_until_the_release() {
    let w = boot().await;
    let job = w.in_transit().await;
    set_eta(&w, &job, day(2026, 11, 20)).await;
    w.job_action(
        &w.docs,
        &job,
        "hold",
        &[("reason", "customs_platform_down"), ("note", "Mirsal is down")],
    )
    .await
    .assert_status(303);
    let before = w.notices.of_type(STAFF_NOTICE).len();

    let alerted = freight_hold::run_eta_alerts(w.state(), day(2026, 11, 21))
        .await
        .unwrap();
    assert_eq!(alerted, 0);
    assert_eq!(w.notices.of_type(STAFF_NOTICE).len(), before);

    w.job_action(&w.docs, &job, "release", &[("note", "Mirsal is back")])
        .await
        .assert_status(303);
    let after_release = w.notices.of_type(STAFF_NOTICE).len();
    let alerted = freight_hold::run_eta_alerts(w.state(), day(2026, 11, 22))
        .await
        .unwrap();
    assert_eq!(alerted, 1);
    let notices = w.notices.of_type(STAFF_NOTICE);
    assert!(notices.len() > after_release);
    assert!(
        notices
            .last()
            .unwrap()
            .data["title"]
            .as_str()
            .unwrap()
            .contains("past its ETA")
    );
    // Once for each ETA.
    assert_eq!(
        freight_hold::run_eta_alerts(w.state(), day(2026, 11, 23))
            .await
            .unwrap(),
        0
    );
}

/// `LOG.RUL.09` leaves out a job that is ready for delivery and one whose ETA
/// is not reached; the rule is on the scheduler.
#[renox::test]
async fn the_eta_rule_skips_a_ready_job_and_an_early_one_and_is_scheduled() {
    let w = boot().await;
    let ready = w.ready_for_delivery().await;
    set_eta(&w, &ready, day(2026, 11, 20)).await;
    let early = w.in_transit().await;
    set_eta(&w, &early, day(2026, 12, 20)).await;
    assert_eq!(
        freight_hold::run_eta_alerts(w.state(), day(2026, 11, 21))
            .await
            .unwrap(),
        0
    );
    // The scheduled task is registered and runs.
    w.app
        .kernel()
        .run_scheduled(freight_hold::ETA_TASK)
        .await
        .expect("the task is registered and runs");
}

/// The hold periods are stored and left out of the on-time measure: a job
/// delivered five days after its ETA after a ten-day hold is on time, one
/// without the hold is late.
#[renox::test]
async fn the_hold_periods_are_left_out_of_the_on_time_measure() {
    let w = boot().await;
    let held = w.in_transit().await;
    let plain = w.in_transit().await;
    let eta = today() - TimeDelta::days(5);
    set_eta(&w, &held, eta).await;
    set_eta(&w, &plain, eta).await;

    w.job_action(&w.docs, &held, "hold", &[("reason", "duty_unpaid"), ("note", "")])
        .await
        .assert_status(303);
    // The hold began ten days ago.
    let began = renox::db::now() - TimeDelta::days(10);
    FreightHold::query()
        .where_(FreightHold::JOB_ID.eq(held.id))
        .update(w.db(), &[("started_at", &began)])
        .await
        .unwrap();
    w.job_action(&w.docs, &held, "release", &[("note", "duty paid")])
        .await
        .assert_status(303);
    assert!(
        freight_hold::days_on_hold(w.db(), held.id, renox::db::now())
            .await
            .unwrap()
            >= 9
    );

    for job in [&held, &plain] {
        w.job_action(&w.docs, job, "arrive", &[]).await.assert_status(303);
        w.job_action(
            &w.docs,
            job,
            "clear",
            &[
                ("declaration_no", "DEC-1"),
                ("duties", "10.00"),
                ("delivery_order", "DO-1"),
            ],
        )
        .await
        .assert_status(303);
        w.job_action(&w.docs, job, "deliver", &[("pod_reference", "epod-1")])
            .await
            .assert_status(303);
    }
    let held_now = w.job(held.id).await;
    let plain_now = w.job(plain.id).await;
    assert_eq!(freight_hold::is_on_time(w.db(), &held_now).await.unwrap(), Some(true));
    assert_eq!(freight_hold::is_on_time(w.db(), &plain_now).await.unwrap(), Some(false));
}
