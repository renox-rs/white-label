//! BL-CLN-01 (#349): the coverage outcome recorded at check-in
//! (`CLN.ACL.15`), through the real routes of the clinics app and of the real
//! wl-scheduling (check in, cancel, reschedule), with the real wl-hr behind the
//! practitioner's licence.
//!
//! The days are relative to today (the routes read the real clock): the visit
//! is tomorrow (`day(1)`), policy P1 was valid through today, as through 9
//! November for a visit on 10 November in the spec scenario, and P2 is valid
//! for months to come.

mod support;

use renox::db::now;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_scheduling::pages::local_text;
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{Booking, BookingStatus, Refusal};

use clinics::coverage::{self, CoverageOutcome, PendingCheckIn};
use clinics::policies::{self, NewPolicy};
use clinics::{Appointment, InsurancePolicy, SelfPayConsent, Visit, appointments, consent};
use support::{Clinic, PATIENT, at, boot, day, dubai};

const PAYER: PartyId = PartyId(900);

/// The receptionist, a doctor, the booking for tomorrow at 09:00 booked with
/// P1 (expired by the visit), and P2 (valid).
struct Scene {
    t: Clinic,
    rita: User,
    doctor: wl_scheduling::Resource,
    service: wl_scheduling::ServiceType,
    booking: Booking,
    p1: InsurancePolicy,
    p2: InsurancePolicy,
}

async fn policy(t: &Clinic, plan: &str, member: &str, valid_to: i64) -> InsurancePolicy {
    policies::create(
        t.state(),
        None,
        NewPolicy::new(PATIENT, PAYER, plan, member, day(-200)).valid_to(day(valid_to)),
    )
    .await
    .expect("the policy is recorded")
}

async fn scene() -> Scene {
    let t = boot().await;
    let rita = t.receptionist().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.consultation(&[&doctor]).await;
    t.prices.set("consultation", day(-400), 45_000);
    let p1 = policy(&t, "Gold", "M-1", 0).await;
    let p2 = policy(&t, "Silver", "M-2", 200).await;
    let booking = t.book(&service, &doctor, at(day(1), 9)).await;
    appointments::attach_policy(t.state(), booking.id, p1.id)
        .await
        .expect("the appointment is insured");
    Scene {
        t,
        rita,
        doctor,
        service,
        booking,
        p1,
        p2,
    }
}

impl Scene {
    /// Posts `fields` to `path` as the receptionist.
    async fn post(&self, path: &str, fields: &[(&str, &str)]) -> renox::testing::TestResponse {
        self.t.app.acting_as(&self.rita);
        self.t.app.htmx().post(path, fields).await
    }

    fn clinic(&self, booking: &Booking, action: &str) -> String {
        format!("/clinics/appointments/{}/{action}", booking.id)
    }

    /// Records the eligibility result `result` for the appointment's policy
    /// (or for `policy`).
    async fn eligibility(&self, booking: &Booking, result: &str, policy: Option<&InsurancePolicy>) {
        let id = policy.map(|p| p.id.to_string()).unwrap_or_default();
        self.post(
            &self.clinic(booking, "eligibility"),
            &[("result", result), ("policy", id.as_str())],
        )
        .await
        .assert_ok();
    }

    async fn outcome(
        &self,
        booking: &Booking,
        outcome: &str,
        policy: Option<&InsurancePolicy>,
    ) -> renox::testing::TestResponse {
        let id = policy.map(|p| p.id.to_string()).unwrap_or_default();
        self.post(
            &self.clinic(booking, "outcome"),
            &[("outcome", outcome), ("policy", id.as_str())],
        )
        .await
    }

    async fn check_in(&self, booking: &Booking) -> renox::testing::TestResponse {
        self.post(
            &format!("/scheduling/bookings/{}/check-in", booking.id),
            &[("x", "y")],
        )
        .await
    }

    async fn pending(&self, booking: &Booking) -> PendingCheckIn {
        PendingCheckIn::of_booking(self.t.db(), booking.id)
            .await
            .unwrap()
            .pop()
            .expect("the front desk recorded something")
    }

    async fn visit(&self, booking: &Booking) -> Visit {
        Visit::of_booking(self.t.db(), booking.id)
            .await
            .unwrap()
            .expect("the visit is open")
    }

    /// The patient signs the self-pay consent on the tablet.
    async fn sign(&self, booking: &Booking, name: &str) -> renox::testing::TestResponse {
        self.post(
            &self.clinic(booking, "consent"),
            &[("signer_name", name), ("services", "")],
        )
        .await
    }

    /// Another booking tomorrow with the same doctor, at `hour`, insured with
    /// `policy`.
    async fn another(&self, hour: u32, policy: &InsurancePolicy) -> Booking {
        let booking = self
            .t
            .book(&self.service, &self.doctor, at(day(1), hour))
            .await;
        appointments::attach_policy(self.t.state(), booking.id, policy.id)
            .await
            .unwrap();
        booking
    }
}

/// Done when 1: an insured appointment with the eligibility recorded negative
/// and no outcome cannot be checked in: "coverage outcome required", through
/// the route and the service, and nothing changed.
#[renox::test]
async fn check_in_is_refused_until_the_eligibility_and_an_outcome_are_recorded() {
    let s = scene().await;

    // Nothing recorded at all.
    let res = s.check_in(&s.booking).await;
    res.assert_status(422);
    assert!(
        res.text().contains("overage outcome required"),
        "{}",
        res.text()
    );

    // Eligibility recorded negative, no outcome.
    s.eligibility(&s.booking, "negative", None).await;
    let res = s.check_in(&s.booking).await;
    res.assert_status(422);
    assert!(
        res.text().contains("overage outcome required"),
        "{}",
        res.text()
    );
    let refused =
        service::check_in(s.t.state(), s.booking.id, Actor::staff(s.rita.id), now()).await;
    let Err(Refusal::Invalid(text)) = refused else {
        panic!("the check-in is refused: {refused:?}");
    };
    assert!(text.contains("overage outcome required"), "{text}");
    assert_eq!(
        s.t.booking(s.booking.id).await.status,
        BookingStatus::Booked
    );
    assert!(
        Visit::of_booking(s.t.db(), s.booking.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(s.pending(&s.booking).await.outcome, None);
}

/// Done when 2: P1 expired and P2 valid with a positive result: the receptionist
/// re-splits to P2, `check_in` succeeds, and the visit shows `re_split` with P2
/// as the policy of the split (the patient share is read from it).
#[renox::test]
async fn a_re_split_to_a_valid_policy_lets_the_patient_in() {
    let s = scene().await;
    s.eligibility(&s.booking, "negative", None).await;

    // P2 has no positive result yet, and another patient's policy never
    // counts.
    s.outcome(&s.booking, "re_split", Some(&s.p2))
        .await
        .assert_status(422);
    let stranger = policies::create(
        s.t.state(),
        None,
        NewPolicy::new(PartyId(71), PAYER, "Gold", "M-9", day(-200)).valid_to(day(200)),
    )
    .await
    .unwrap();
    s.eligibility(&s.booking, "positive", Some(&s.p2)).await;
    let res = s
        .post(
            &s.clinic(&s.booking, "eligibility"),
            &[
                ("result", "positive"),
                ("policy", stranger.id.to_string().as_str()),
            ],
        )
        .await;
    res.assert_status(422);

    s.outcome(&s.booking, "re_split", Some(&s.p2))
        .await
        .assert_ok();
    s.check_in(&s.booking).await.assert_status(303);

    let visit = s.visit(&s.booking).await;
    assert_eq!(visit.coverage_outcome, Some(CoverageOutcome::ReSplit));
    assert_eq!(visit.coverage_policy_id, Some(s.p2.id));
    assert_eq!(visit.policy_id, Some(s.p1.id));
    assert_eq!(visit.eligibility_positive, Some(false));
    let moved = s.pending(&s.booking).await;
    assert_eq!(moved.moved_visit_id, Some(visit.id));
    assert!(
        PendingCheckIn::active_of(s.t.db(), s.booking.id)
            .await
            .unwrap()
            .is_none()
    );
    s.t.app
        .get(&format!("/clinics/visits/{}", visit.id))
        .await
        .assert_ok()
        .assert_see("Re-split to another policy")
        .assert_see("Silver");
}

/// Done when 3, 4 and 5: self-pay needs a price on the visit day and a signed
/// consent; with both, `check_in` succeeds with outcome `self_pay`, the
/// consent reference and an estimate of AED 450.00.
#[renox::test]
async fn self_pay_needs_a_price_and_a_signed_consent() {
    let s = scene().await;
    s.eligibility(&s.booking, "negative", None).await;

    // No consent yet: refused.
    let res = s.outcome(&s.booking, "self_pay", None).await;
    res.assert_status(422);
    assert!(
        res.text().contains("signed self-pay consent"),
        "{}",
        res.text()
    );

    // The price list has none for the visit day (it starts the day after):
    // refused, naming the service, on the outcome and on the signature.
    let later = day(2);
    let t = &s.t;
    // A second scene service without any price at all.
    let mut peel = wl_scheduling::ServiceType::new("peel", "Peel", 30);
    peel.required_qualifications = clinics::seed::REQUIRED_QUALIFICATIONS.to_owned();
    peel.check_in_required = true;
    peel.track_in_progress = true;
    peel.save(t.db()).await.unwrap();
    wl_scheduling::resource::offer(t.db(), peel.id, s.doctor.id)
        .await
        .unwrap();
    let unpriced = t.book(&peel, &s.doctor, at(day(1), 11)).await;
    appointments::attach_policy(t.state(), unpriced.id, s.p1.id)
        .await
        .unwrap();
    s.eligibility(&unpriced, "negative", None).await;
    let res = s.outcome(&unpriced, "self_pay", None).await;
    res.assert_status(422);
    assert!(
        res.text().contains("no self-pay price for peel"),
        "{}",
        res.text()
    );
    let res = s.sign(&unpriced, "Patient One").await;
    res.assert_status(422);
    assert!(
        res.text().contains("no self-pay price for peel"),
        "{}",
        res.text()
    );
    t.prices.set("peel", later, 9_000);
    let res = s.outcome(&unpriced, "self_pay", None).await;
    res.assert_status(422);
    assert!(
        res.text().contains("no self-pay price for peel"),
        "a price from tomorrow is none today"
    );

    // A blank signature is refused; the signature gives the outcome.
    s.sign(&s.booking, "  ").await.assert_status(422);
    s.sign(&s.booking, "Patient One").await.assert_ok();
    let pending = s.pending(&s.booking).await;
    assert_eq!(pending.outcome, Some(CoverageOutcome::SelfPay));
    let signed = SelfPayConsent::current(t.db(), s.booking.id)
        .await
        .unwrap()
        .expect("the consent is in force");
    assert_eq!(pending.consent_id, Some(signed.id));
    assert_eq!(signed.estimated_fils, 45_000);
    assert_eq!(signed.priced_on, day(1));
    assert_eq!(signed.signer_name, "Patient One");
    assert!(signed.lists("consultation"));
    assert_eq!(signed.document_hash.len(), 16);
    t.app
        .get(&format!("/clinics/appointments/{}", s.booking.id))
        .await
        .assert_ok()
        .assert_see("AED 450.00")
        .assert_see("Self-pay");

    s.check_in(&s.booking).await.assert_status(303);
    let visit = s.visit(&s.booking).await;
    assert_eq!(visit.coverage_outcome, Some(CoverageOutcome::SelfPay));
    assert_eq!(visit.consent_id, Some(signed.id));
    assert_eq!(visit.coverage_policy_id, None);
    let kept = SelfPayConsent::find_or_404(t.db(), signed.id)
        .await
        .unwrap();
    assert_eq!(kept.visit_id, Some(visit.id));
    assert!(!kept.is_superseded());
}

/// Done when 6: the data recorded for an appointment that is not checked in
/// by 23:59 of its day is discarded; at 23:58 it is still there. The daily job
/// is the same function.
#[renox::test]
async fn the_pending_data_is_discarded_at_the_end_of_the_day() {
    let s = scene().await;
    s.eligibility(&s.booking, "negative", None).await;
    s.sign(&s.booking, "Patient One").await.assert_ok();
    let visit_day = day(1).to_string();

    let early = coverage::discard_lapsed(s.t.state(), dubai(&visit_day, 23, 58))
        .await
        .unwrap();
    assert_eq!(early, 0);
    assert!(
        PendingCheckIn::active_of(s.t.db(), s.booking.id)
            .await
            .unwrap()
            .is_some()
    );

    let late = coverage::discard_lapsed(s.t.state(), dubai(&visit_day, 23, 59))
        .await
        .unwrap();
    assert_eq!(late, 1);
    let discarded = s.pending(&s.booking).await;
    assert_eq!(discarded.discarded_on, Some(day(1)));
    assert_eq!(discarded.discard_reason.as_deref(), Some("lapsed"));
    // The consent stays on file, marked superseded, and the patient can no
    // longer be checked in on it.
    let kept = SelfPayConsent::of_booking(s.t.db(), s.booking.id)
        .await
        .unwrap();
    assert_eq!(kept.len(), 1);
    assert!(kept[0].is_superseded());
    s.check_in(&s.booking).await.assert_status(422);

    // The job runs as a command too (nothing lapsed at the real clock).
    s.t.app
        .kernel()
        .call(coverage::TASK, Vec::<String>::new())
        .await
        .expect("the discard command runs");
}

/// Done when 7: a deposit of AED 100.00. Cancelled with
/// `coverage_not_confirmed` the booking is initiated by the business and the
/// whole deposit is refunded; rescheduled instead, the deposit is carried to
/// the new time. In both, what was recorded is discarded.
#[renox::test]
async fn coverage_not_confirmed_refunds_and_a_reschedule_carries_the_deposit() {
    let s = scene().await;
    s.eligibility(&s.booking, "negative", None).await;
    let other = s.another(11, &s.p1).await;
    s.eligibility(&other, "negative", None).await;
    assert_eq!(s.booking.deposit_amount, 10_000);

    // Cancel: business, refunded in full.
    s.post(
        &format!("/scheduling/bookings/{}/cancel", s.booking.id),
        &[("reason_code", clinics::reasons::COVERAGE_NOT_CONFIRMED)],
    )
    .await
    .assert_status(303);
    let cancelled = s.t.booking(s.booking.id).await;
    assert_eq!(cancelled.status, BookingStatus::Cancelled);
    assert_eq!(cancelled.cancel_initiated_by.as_deref(), Some("business"));
    assert_eq!(
        cancelled.cancel_code.as_deref(),
        Some("coverage_not_confirmed")
    );
    assert_eq!(s.t.refunds.amounts(), [10_000]);
    assert_eq!(
        s.pending(&s.booking).await.discard_reason.as_deref(),
        Some("cancelled")
    );
    assert!(
        Appointment::of_booking(s.t.db(), s.booking.id)
            .await
            .unwrap()
            .unwrap()
            .is_cancelled()
    );

    // Reschedule: the deposit goes with the booking, nothing is refunded.
    let new_start = at(day(3), 10);
    s.post(
        &format!("/scheduling/bookings/{}/reschedule", other.id),
        &[("starts_at", local_text(new_start).as_str())],
    )
    .await
    .assert_status(303);
    let moved = s.t.booking(other.id).await;
    assert_eq!(moved.starts_at, new_start);
    assert_eq!(moved.deposit_amount, 10_000);
    assert_eq!(s.t.events.rescheduled.all()[0].deposit_carried, 10_000);
    assert_eq!(
        s.t.refunds.amounts(),
        [10_000],
        "the carried deposit is not refunded"
    );
    assert_eq!(
        s.pending(&other).await.discard_reason.as_deref(),
        Some("rescheduled")
    );
}

/// Done when 8: on a self-pay visit a service not on the consent is refused
/// until an updated consent is signed; the old consent is marked superseded
/// and the visit keeps the new one. (Charge capture is not built: this is the
/// check it will call.)
#[renox::test]
async fn a_service_not_on_the_consent_needs_an_updated_consent() {
    let s = scene().await;
    s.eligibility(&s.booking, "negative", None).await;
    s.sign(&s.booking, "Patient One").await.assert_ok();
    s.check_in(&s.booking).await.assert_status(303);
    let visit = s.visit(&s.booking).await;

    consent::ensure_covered(s.t.state(), visit.id, "consultation")
        .await
        .expect("the booked service is on the consent");
    let refused = consent::ensure_covered(s.t.state(), visit.id, "x_ray").await;
    assert!(refused.is_err(), "x_ray is not on the consent");

    // The new service needs a price before it can be shown to the patient.
    let unpriced = consent::sign_updated(
        s.t.state(),
        Some(s.rita.id),
        visit.id,
        "Patient One",
        None,
        &["x_ray".to_owned()],
        now(),
    )
    .await;
    assert!(unpriced.is_err());
    s.t.prices.set("x_ray", day(-400), 12_000);
    let updated = consent::sign_updated(
        s.t.state(),
        Some(s.rita.id),
        visit.id,
        "Patient One",
        Some("10.0.0.5"),
        &["x_ray".to_owned()],
        now(),
    )
    .await
    .expect("the updated consent is signed");

    assert_eq!(updated.estimated_fils, 57_000);
    assert!(updated.lists("consultation") && updated.lists("x_ray"));
    consent::ensure_covered(s.t.state(), visit.id, "x_ray")
        .await
        .expect("x_ray is on the updated consent");
    let all = SelfPayConsent::of_booking(s.t.db(), s.booking.id)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    assert!(all[0].is_superseded() && !all[1].is_superseded());
    assert_eq!(s.visit(&s.booking).await.consent_id, Some(updated.id));
}

/// Done when 9: a policy valid through the visit day and a positive result:
/// the outcome is *insured* by itself, nothing else is asked, and `check_in`
/// succeeds. A positive result for an expired policy sets nothing.
#[renox::test]
async fn a_policy_valid_to_the_visit_day_needs_nothing_after_a_positive_result() {
    let s = scene().await;
    let valid = policy(&s.t, "Platinum", "M-3", 1).await;
    let booking = s.another(11, &valid).await;

    s.eligibility(&booking, "positive", None).await;
    let pending = s.pending(&booking).await;
    assert_eq!(pending.outcome, Some(CoverageOutcome::Insured));
    assert_eq!(pending.coverage_policy_id, Some(valid.id));
    s.check_in(&booking).await.assert_status(303);
    let visit = s.visit(&booking).await;
    assert_eq!(visit.coverage_outcome, Some(CoverageOutcome::Insured));
    assert_eq!(visit.coverage_policy_id, Some(valid.id));
    assert_eq!(visit.eligibility_positive, Some(true));

    // The expired policy: a positive result alone is not cover.
    s.eligibility(&s.booking, "positive", None).await;
    assert_eq!(s.pending(&s.booking).await.outcome, None);
    s.check_in(&s.booking).await.assert_status(422);
    s.outcome(&s.booking, "insured", None)
        .await
        .assert_status(422);
}

/// A negative result takes back an *insured* outcome the positive one set.
#[renox::test]
async fn a_negative_result_takes_back_insured() {
    let s = scene().await;
    let valid = policy(&s.t, "Platinum", "M-3", 1).await;
    let booking = s.another(11, &valid).await;
    s.eligibility(&booking, "positive", None).await;
    assert_eq!(
        s.pending(&booking).await.outcome,
        Some(CoverageOutcome::Insured)
    );

    s.eligibility(&booking, "negative", None).await;

    assert_eq!(s.pending(&booking).await.outcome, None);
    s.check_in(&booking).await.assert_status(422);
}

/// The eligibility turns positive after self-pay was chosen: before `start`
/// the receptionist changes the visit to *insured* and the consent is marked
/// superseded; after `start` the outcome is fixed.
#[renox::test]
async fn the_outcome_may_change_to_insured_until_the_service_starts() {
    let s = scene().await;
    let valid = policy(&s.t, "Platinum", "M-3", 1).await;
    let before = s.another(11, &valid).await;
    let after = s.another(12, &valid).await;
    for booking in [&before, &after] {
        s.eligibility(booking, "negative", None).await;
        s.sign(booking, "Patient One").await.assert_ok();
        s.check_in(booking).await.assert_status(303);
    }
    let visit = s.visit(&before).await;
    assert_eq!(visit.coverage_outcome, Some(CoverageOutcome::SelfPay));

    s.post(
        &format!("/clinics/visits/{}/insured", visit.id),
        &[("reference", "REF-77")],
    )
    .await
    .assert_ok();
    let insured = s.visit(&before).await;
    assert_eq!(insured.coverage_outcome, Some(CoverageOutcome::Insured));
    assert_eq!(insured.coverage_policy_id, Some(valid.id));
    assert_eq!(insured.eligibility_positive, Some(true));
    let consents = SelfPayConsent::of_booking(s.t.db(), before.id)
        .await
        .unwrap();
    assert_eq!(consents.len(), 1, "the consent stays on file");
    assert!(consents[0].is_superseded());
    s.post(
        &format!("/clinics/visits/{}/insured", visit.id),
        &[("reference", "")],
    )
    .await
    .assert_status(422);

    // Once the service has started the outcome is fixed.
    service::start(s.t.state(), after.id, Actor::staff(s.rita.id), now())
        .await
        .expect("the service starts");
    let started = s.visit(&after).await;
    let res = s
        .post(
            &format!("/clinics/visits/{}/insured", started.id),
            &[("reference", "")],
        )
        .await;
    res.assert_status(422);
    assert!(res.text().contains("outcome is fixed"), "{}", res.text());
    assert_eq!(
        s.visit(&after).await.coverage_outcome,
        Some(CoverageOutcome::SelfPay)
    );
}

/// An appointment booked with another policy forgets what was recorded for the
/// first; a patient who is not insured is asked nothing.
#[renox::test]
async fn a_new_policy_discards_what_was_recorded_and_a_self_pay_patient_is_asked_nothing() {
    let s = scene().await;
    s.eligibility(&s.booking, "negative", None).await;

    appointments::attach_policy(s.t.state(), s.booking.id, s.p2.id)
        .await
        .unwrap();

    assert_eq!(
        s.pending(&s.booking).await.discard_reason.as_deref(),
        Some("policy_changed")
    );
    s.check_in(&s.booking).await.assert_status(422);

    let plain = s.t.book(&s.service, &s.doctor, at(day(1), 14)).await;
    s.check_in(&plain).await.assert_status(303);
    assert_eq!(s.visit(&plain).await.coverage_outcome, None);
    // Not even a booking with no known patient is asked.
    let nobody = service::book(
        s.t.state(),
        NewBooking::new(PATIENT, s.service.id, &[s.doctor.id], at(day(1), 16)),
        Actor::staff(1),
        support::scheduling::now(),
    )
    .await
    .unwrap();
    service::check_in(s.t.state(), nobody.id, Actor::staff(1), now())
        .await
        .expect("an appointment with no policy is let through");
}

/// Without the permission the screens and the actions answer 403; the
/// appointment page shows the cover and no card or reason.
#[renox::test]
async fn only_the_desk_records_the_coverage() {
    let s = scene().await;
    let viewer = s.t.viewer().await;
    s.t.app.acting_as(&viewer);
    s.t.app
        .get(&format!("/clinics/appointments/{}", s.booking.id))
        .await
        .assert_forbidden();
    s.t.app
        .htmx()
        .post(
            &s.clinic(&s.booking, "eligibility"),
            &[("result", "negative")],
        )
        .await
        .assert_forbidden();

    s.t.app.acting_as(&s.rita);
    s.t.app
        .get(&format!("/clinics/appointments/{}", s.booking.id))
        .await
        .assert_ok()
        .assert_see("Gold")
        .assert_see("Not recorded");
    assert!(s.pending_rows().await == 0, "viewing records nothing");
}

impl Scene {
    async fn pending_rows(&self) -> usize {
        PendingCheckIn::of_booking(self.t.db(), self.booking.id)
            .await
            .unwrap()
            .len()
    }
}
