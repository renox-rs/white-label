//! `CLN.RUL.07` (base for BL-CLN-03): an insured appointment for a service
//! that needs a prior authorisation, with none valid on its day, is flagged
//! and the billing coordinator gets a task; an authorisation that covers it,
//! a reschedule to a covered day or a cancellation clears the flag.

mod support;

use wl_kernel::PartyId;
use wl_kernel::tickets::Assignee;
use wl_scheduling::pages::local_text;

use clinics::authorisations;
use clinics::flags::{AppointmentFlag, FlagCause};
use clinics::policies::{self, NewPolicy};
use clinics::prior_auth_rules::needed_key;
use clinics::{InsurancePolicy, appointments};
use support::{Clinic, PATIENT, at, boot, day};

const PAYER: PartyId = PartyId(900);
const OTHER_PAYER: PartyId = PartyId(901);

async fn policy_with(t: &Clinic, payer: PartyId, member: &str) -> InsurancePolicy {
    policies::create(
        t.state(),
        None,
        NewPolicy::new(PATIENT, payer, "Gold", member, day(-200)).valid_to(day(300)),
    )
    .await
    .expect("the policy is recorded")
}

async fn open(t: &Clinic, booking: i64) -> Vec<AppointmentFlag> {
    AppointmentFlag::open_of(t.db(), booking).await.unwrap()
}

/// Flagged with a task for the billing coordinator when booked with a policy
/// whose payer wants an authorisation; once, however often it is checked;
/// not for another payer's policy or a self-pay appointment; cleared when an
/// authorisation that covers the day is approved, which also closes the task.
#[renox::test]
async fn an_appointment_needing_an_authorisation_is_flagged_and_cleared_when_one_covers_it() {
    let t = boot().await;
    let bea = t.coordinator().await;
    let rita = t.receptionist().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.consultation(&[&doctor]).await;
    authorisations::require_for(t.state(), PAYER, "consultation")
        .await
        .unwrap();
    let policy = policy_with(&t, PAYER, "M-1").await;
    let free = policy_with(&t, OTHER_PAYER, "M-2").await;

    let booking = t.book(&service, &doctor, at(day(5), 9)).await;
    assert!(
        open(&t, booking.id).await.is_empty(),
        "no policy yet: self-pay"
    );
    appointments::attach_policy(t.state(), booking.id, policy.id)
        .await
        .unwrap();
    let flags = open(&t, booking.id).await;
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].cause, FlagCause::NeedsAuthorisation);
    assert_eq!(flags[0].origin, "policy_attached");
    let tasks = t.tasks.with_key("clinics.prior_auth_needed:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].source_key.as_deref(),
        Some(needed_key(booking.id).as_str())
    );
    assert_eq!(tasks[0].assignee, Some(Assignee::user(bea.id)));

    // Checked again: one flag, one task.
    appointments::attach_policy(t.state(), booking.id, policy.id)
        .await
        .unwrap();
    assert_eq!(open(&t, booking.id).await.len(), 1);
    assert_eq!(t.tasks.with_key("clinics.prior_auth_needed:").len(), 1);

    // The front desk sees it on the appointment and on the list.
    t.app.acting_as(&rita);
    t.app
        .get(&format!("/clinics/appointments/{}", booking.id))
        .await
        .assert_ok()
        .assert_see("Needs authorisation");
    t.app
        .get("/clinics/appointments")
        .await
        .assert_ok()
        .assert_see("Needs authorisation");

    // Another payer's policy, and a self-pay appointment, are not flagged.
    let elsewhere = t.book(&service, &doctor, at(day(5), 11)).await;
    appointments::attach_policy(t.state(), elsewhere.id, free.id)
        .await
        .unwrap();
    assert!(open(&t, elsewhere.id).await.is_empty());
    let self_pay = t.book(&service, &doctor, at(day(6), 9)).await;
    assert!(open(&t, self_pay.id).await.is_empty());

    // An authorisation that ends before the day covers nothing …
    t.authorisation(&policy, "consultation", 2, day(3)).await;
    assert_eq!(open(&t, booking.id).await.len(), 1);
    // … one that covers the day clears the flag and closes the task.
    t.authorisation(&policy, "consultation", 2, day(10)).await;
    assert!(open(&t, booking.id).await.is_empty());
    let all = AppointmentFlag::of_booking(t.db(), booking.id)
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].clear_reason.as_deref(), Some("covered"));
    assert!(t.tasks.is_resolved(1), "the coordinator's task is closed");
}

/// A reschedule checks the flag for the new day: out of the authorisation's
/// validity the appointment is flagged, back inside it the flag clears.
#[renox::test]
async fn a_reschedule_checks_the_flag_for_the_new_day() {
    let t = boot().await;
    let rita = t.receptionist().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.consultation(&[&doctor]).await;
    authorisations::require_for(t.state(), PAYER, "consultation")
        .await
        .unwrap();
    let policy = policy_with(&t, PAYER, "M-1").await;
    t.authorisation(&policy, "consultation", 2, day(10)).await;
    let booking = t.book(&service, &doctor, at(day(5), 9)).await;
    appointments::attach_policy(t.state(), booking.id, policy.id)
        .await
        .unwrap();
    assert!(open(&t, booking.id).await.is_empty(), "covered on day 5");

    t.app.acting_as(&rita);
    let path = format!("/scheduling/bookings/{}/reschedule", booking.id);
    t.app
        .htmx()
        .post(&path, &[("starts_at", local_text(at(day(12), 9)).as_str())])
        .await
        .assert_status(303);
    let flags = open(&t, booking.id).await;
    assert_eq!(flags.len(), 1, "not covered on day 12");
    assert_eq!(flags[0].origin, "rescheduled");

    t.app
        .htmx()
        .post(&path, &[("starts_at", local_text(at(day(7), 9)).as_str())])
        .await
        .assert_status(303);
    assert!(
        open(&t, booking.id).await.is_empty(),
        "covered again on day 7"
    );
}

/// A cancelled appointment carries no flag and its task is closed.
#[renox::test]
async fn a_cancelled_appointment_loses_its_flag() {
    let t = boot().await;
    let rita = t.receptionist().await;
    t.coordinator().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.consultation(&[&doctor]).await;
    authorisations::require_for(t.state(), PAYER, "consultation")
        .await
        .unwrap();
    let policy = policy_with(&t, PAYER, "M-1").await;
    let booking = t.book(&service, &doctor, at(day(5), 9)).await;
    appointments::attach_policy(t.state(), booking.id, policy.id)
        .await
        .unwrap();
    assert_eq!(open(&t, booking.id).await.len(), 1);

    t.app.acting_as(&rita);
    t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{}/cancel", booking.id),
            &[("reason_code", clinics::reasons::COVERAGE_NOT_CONFIRMED)],
        )
        .await
        .assert_status(303);
    assert!(open(&t, booking.id).await.is_empty());
    let all = AppointmentFlag::of_booking(t.db(), booking.id)
        .await
        .unwrap();
    assert_eq!(all[0].clear_reason.as_deref(), Some("cancelled"));
    assert!(t.tasks.is_resolved(1));
}
