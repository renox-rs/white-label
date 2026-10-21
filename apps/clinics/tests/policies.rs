//! The base of the 05 insurance policy and its eligibility result (base for
//! #349, #352): the policy is recorded through its page, eligibility is
//! recorded per day, and an appointment is booked with a policy of its own
//! patient.

mod support;

use renox::db::now;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_scheduling::service::{self, Actor};

use clinics::policies::{self, NewPolicy};
use clinics::{EligibilityCheck, InsurancePolicy, appointments};
use support::{PATIENT, at, boot, day};

const PAYER: PartyId = PartyId(900);

/// The receptionist records a policy through the page; the list shows its
/// plan and dates, and a viewer without the permission sees nothing.
#[renox::test]
async fn a_policy_is_recorded_and_listed() {
    let t = boot().await;
    let desk = t.receptionist().await;
    t.app.acting_as(&desk);

    t.app
        .htmx()
        .post(
            "/clinics/policies",
            &[
                ("patient", "70"),
                ("payer", "900"),
                ("plan", "Gold Plus"),
                ("member_id", "M-123"),
                ("valid_from", "2026-01-01"),
                ("valid_to", "2027-03-31"),
                ("card_valid_to", "2026-12-31"),
            ],
        )
        .await
        .assert_ok();

    let policy = InsurancePolicy::query().first(t.db()).await.unwrap().unwrap();
    assert_eq!(policy.patient_party_id, 70);
    assert_eq!(policy.payer_party_id, 900);
    assert_eq!(policy.plan, "Gold Plus");
    assert_eq!(policy.valid_to, Some("2027-03-31".parse().unwrap()));
    t.app
        .get("/clinics/policies")
        .await
        .assert_ok()
        .assert_see("Gold Plus")
        .assert_see("M-123");

    let viewer = t.viewer().await;
    t.app.acting_as(&viewer);
    t.app.get("/clinics/policies").await.assert_forbidden();
}

/// A policy with no plan, no member id, or ending before it starts is refused.
#[renox::test]
async fn a_policy_without_plan_member_or_with_bad_dates_is_refused() {
    let t = boot().await;
    let from = day(0);
    for new in [
        NewPolicy::new(PATIENT, PAYER, " ", "M-1", from),
        NewPolicy::new(PATIENT, PAYER, "Gold", "", from),
        NewPolicy::new(PATIENT, PAYER, "Gold", "M-1", from).valid_to(day(-1)),
    ] {
        assert!(policies::create(t.state(), None, new).await.is_err());
    }
    assert!(InsurancePolicy::query().first(t.db()).await.unwrap().is_none());
}

/// Eligibility is recorded per day: the latest result of the day is the one
/// that counts, and another day has none.
#[renox::test]
async fn eligibility_is_recorded_per_day() {
    let t = boot().await;
    let policy = policies::create(
        t.state(),
        None,
        NewPolicy::new(PATIENT, PAYER, "Gold", "M-1", day(-30)),
    )
    .await
    .unwrap();

    let negative = policies::record_eligibility(
        t.state(),
        Some(1),
        policy.id,
        false,
        Some("REF-1"),
        Some("member not found"),
        now(),
    )
    .await
    .unwrap();
    assert!(!negative.positive);
    assert_eq!(negative.reference.as_deref(), Some("REF-1"));
    let positive = policies::record_eligibility(
        t.state(),
        Some(1),
        policy.id,
        true,
        None,
        Some("  "),
        now(),
    )
    .await
    .unwrap();
    assert_eq!(positive.note, None);

    let today = day(0);
    let latest = EligibilityCheck::latest_on(t.db(), policy.id, today)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.id, positive.id);
    assert!(
        EligibilityCheck::latest_on(t.db(), policy.id, day(1))
            .await
            .unwrap()
            .is_none()
    );
}

/// An appointment is booked with a policy of its own patient; another
/// patient's policy, a cancelled appointment and a patient already checked in
/// are refused.
#[renox::test]
async fn an_appointment_takes_the_policy_of_its_patient() {
    let t = boot().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service_type = t.consultation(&[&doctor]).await;
    let booking = t.book(&service_type, &doctor, at(day(1), 9)).await;
    let mine = policies::create(
        t.state(),
        None,
        NewPolicy::new(PATIENT, PAYER, "Gold", "M-1", day(-30)),
    )
    .await
    .unwrap();
    let other = policies::create(
        t.state(),
        None,
        NewPolicy::new(PartyId(71), PAYER, "Gold", "M-2", day(-30)),
    )
    .await
    .unwrap();

    assert!(appointments::attach_policy(t.state(), booking.id, other.id)
        .await
        .is_err());
    let insured = appointments::attach_policy(t.state(), booking.id, mine.id)
        .await
        .expect("the patient's own policy");
    assert!(insured.is_insured());
    assert_eq!(insured.policy_id, Some(mine.id));

    service::check_in(t.state(), booking.id, Actor::staff(1), now())
        .await
        .unwrap();
    assert!(appointments::attach_policy(t.state(), booking.id, mine.id)
        .await
        .is_err());
    let visit = clinics::Visit::of_booking(t.db(), booking.id)
        .await
        .unwrap()
        .unwrap();
    assert!(visit.is_insured());
    assert_eq!(visit.policy_id, Some(mine.id));

    let cancelled = t.book(&service_type, &doctor, at(day(2), 9)).await;
    service::cancel(
        t.state(),
        cancelled.id,
        service::CancelRequest::new("equipment_failure"),
        Actor::staff(1),
        now(),
    )
    .await
    .unwrap();
    assert!(appointments::attach_policy(t.state(), cancelled.id, mine.id)
        .await
        .is_err());
}
