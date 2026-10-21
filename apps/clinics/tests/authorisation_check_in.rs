//! BL-CLN-02 (#350): a service that needs a prior authorisation is checked in
//! only with a valid authorisation that has units left on the visit day, or as
//! self-pay with a consent that lists it, or the appointment is rescheduled
//! while a new authorisation is requested. Through the real routes of the
//! clinics app and of wl-scheduling, with the real authorisation machine.

mod support;

use renox::db::now;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::tickets::Assignee;
use wl_scheduling::pages::local_text;
use wl_scheduling::service::{self, Actor};
use wl_scheduling::{Booking, BookingStatus, ServiceType};

use clinics::auth_check;
use clinics::authorisations;
use clinics::coverage::{CoverageOutcome, PendingCheckIn};
use clinics::flags::AppointmentFlag;
use clinics::policies::{self, NewPolicy};
use clinics::{
    AuthStatus, AuthorisationHold, InsurancePolicy, PriorAuthorisation, SelfPayConsent, Visit,
    appointments,
};
use support::{Clinic, PATIENT, at, boot, day};

const PAYER: PartyId = PartyId(900);
const NEEDED: &str = "Prior authorisation required: self-pay with consent or reschedule";

/// The receptionist, the billing coordinator, a doctor, the consultation and
/// the procedure (which the payer wants authorised) and the patient's policy.
struct Scene {
    t: Clinic,
    rita: User,
    bea: User,
    doctor: wl_scheduling::Resource,
    consultation: ServiceType,
    procedure: ServiceType,
    policy: InsurancePolicy,
}

async fn scene() -> Scene {
    let t = boot().await;
    let rita = t.receptionist().await;
    let bea = t.coordinator().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let consultation = t.consultation(&[&doctor]).await;
    let mut procedure = ServiceType::new("procedure", "Procedure", 30);
    procedure.required_qualifications = clinics::seed::REQUIRED_QUALIFICATIONS.to_owned();
    procedure.check_in_required = true;
    procedure.track_in_progress = true;
    procedure.save(t.db()).await.unwrap();
    wl_scheduling::resource::offer(t.db(), procedure.id, doctor.id)
        .await
        .unwrap();
    t.prices.set("consultation", day(-400), 45_000);
    t.prices.set("procedure", day(-400), 120_000);
    authorisations::require_for(t.state(), PAYER, "procedure")
        .await
        .unwrap();
    let policy = policies::create(
        t.state(),
        None,
        NewPolicy::new(PATIENT, PAYER, "Gold", "M-1", day(-200)).valid_to(day(300)),
    )
    .await
    .unwrap();
    Scene {
        t,
        rita,
        bea,
        doctor,
        consultation,
        procedure,
        policy,
    }
}

impl Scene {
    /// A booking of `service` on `date` at `hour`, insured with the policy.
    async fn insured(&self, service: &ServiceType, date: i64, hour: u32) -> Booking {
        let booking = self.t.book(service, &self.doctor, at(day(date), hour)).await;
        appointments::attach_policy(self.t.state(), booking.id, self.policy.id)
            .await
            .unwrap();
        booking
    }

    async fn post(&self, path: &str, fields: &[(&str, &str)]) -> renox::testing::TestResponse {
        self.t.app.acting_as(&self.rita);
        self.t.app.htmx().post(path, fields).await
    }

    /// The payer confirms the cover today.
    async fn eligible(&self, booking: &Booking) {
        self.post(
            &format!("/clinics/appointments/{}/eligibility", booking.id),
            &[("result", "positive")],
        )
        .await
        .assert_ok();
    }

    async fn check_in(&self, booking: &Booking) -> renox::testing::TestResponse {
        self.post(
            &format!("/scheduling/bookings/{}/check-in", booking.id),
            &[("x", "y")],
        )
        .await
    }

    async fn services(&self, booking: &Booking, services: &str) {
        self.post(
            &format!("/clinics/appointments/{}/services", booking.id),
            &[("services", services)],
        )
        .await
        .assert_ok();
    }

    async fn sign(&self, booking: &Booking) -> renox::testing::TestResponse {
        self.post(
            &format!("/clinics/appointments/{}/authorisation-consent", booking.id),
            &[("signer_name", "Patient One")],
        )
        .await
    }

    async fn visit(&self, booking: &Booking) -> Visit {
        Visit::of_booking(self.t.db(), booking.id)
            .await
            .unwrap()
            .expect("the patient is checked in")
    }

    async fn a1(&self, units: i64, until: i64) -> PriorAuthorisation {
        self.t
            .authorisation(&self.policy, "procedure", units, day(until))
            .await
    }
}

fn assert_refused(res: &renox::testing::TestResponse) {
    res.assert_status(422);
    assert!(res.text().contains(NEEDED), "{}", res.text());
}

/// Done when 1: a procedure needing authorisation, A1 approved for 3 units and
/// all 3 consumed: check-in is refused, the appointment stays booked and
/// nothing is consumed; with a signed self-pay consent for the procedure,
/// check-in succeeds.
#[renox::test]
async fn a_used_up_authorisation_refuses_the_check_in_until_a_consent_is_signed() {
    let s = scene().await;
    let a1 = s.a1(3, 30).await;
    authorisations::consume(s.t.state(), a1.id, 3, now())
        .await
        .unwrap();
    let booking = s.insured(&s.procedure, 1, 9).await;
    s.eligible(&booking).await;

    assert_refused(&s.check_in(&booking).await);
    assert_eq!(s.t.booking(booking.id).await.status, BookingStatus::Booked);
    assert!(Visit::of_booking(s.t.db(), booking.id).await.unwrap().is_none());
    let a1 = PriorAuthorisation::find_or_404(s.t.db(), a1.id).await.unwrap();
    assert_eq!((a1.status, a1.units_consumed), (AuthStatus::Used, 3));

    s.sign(&booking).await.assert_ok();
    s.check_in(&booking).await.assert_status(303);
    let visit = s.visit(&booking).await;
    assert_eq!(visit.coverage_outcome, Some(CoverageOutcome::Insured));
    let consent = SelfPayConsent::find_or_404(s.t.db(), visit.consent_id.unwrap())
        .await
        .unwrap();
    assert_eq!(consent.codes(), ["procedure"]);
    assert_eq!(consent.estimated_fils, 120_000);
    // Nothing was consumed or held by the check-in.
    assert_eq!(
        PriorAuthorisation::find_or_404(s.t.db(), a1.id)
            .await
            .unwrap()
            .units_consumed,
        3
    );
    assert!(AuthorisationHold::of_visit(s.t.db(), visit.id).await.unwrap().is_empty());
}

/// Done when 2: A1 valid until the day before the visit: refused the same way
/// (the last valid day itself would still do).
#[renox::test]
async fn an_authorisation_that_ends_before_the_visit_day_does_not_cover_it() {
    let s = scene().await;
    s.a1(3, 1).await;
    let after = s.insured(&s.procedure, 2, 9).await;
    s.eligible(&after).await;
    assert_refused(&s.check_in(&after).await);

    let on_the_day = s.insured(&s.procedure, 1, 11).await;
    s.eligible(&on_the_day).await;
    s.check_in(&on_the_day).await.assert_status(303);
    let held = AuthorisationHold::of_visit(s.t.db(), s.visit(&on_the_day).await.id)
        .await
        .unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].units, 1);
}

/// Done when 3: A1 with one unit left and V1 checked in holding it: V2 the
/// same day sees none left. The units come back when V1 is completed.
#[renox::test]
async fn a_checked_in_visit_holds_the_unit_the_next_one_would_use() {
    let s = scene().await;
    let a1 = s.a1(1, 30).await;
    let v1 = s.insured(&s.procedure, 1, 9).await;
    let v2 = s.insured(&s.procedure, 1, 11).await;
    s.eligible(&v1).await;
    s.eligible(&v2).await;

    s.check_in(&v1).await.assert_status(303);
    assert_eq!(AuthorisationHold::held(s.t.db(), a1.id, None).await.unwrap(), 1);
    assert_refused(&s.check_in(&v2).await);
    let appointment = clinics::Appointment::of_booking(s.t.db(), v2.id)
        .await
        .unwrap()
        .unwrap();
    let pending = PendingCheckIn::active_of(s.t.db(), v2.id).await.unwrap();
    let plan = auth_check::plan(s.t.state(), &appointment, pending.as_ref())
        .await
        .unwrap();
    assert_eq!((plan[0].covered(), plan[0].self_pay), (0, 1));
    // Nothing was consumed.
    assert_eq!(
        PriorAuthorisation::find_or_404(s.t.db(), a1.id).await.unwrap().units_left(),
        1
    );

    // V1 is over: its unit is free again.
    service::start(s.t.state(), v1.id, Actor::staff(s.rita.id), now())
        .await
        .unwrap();
    service::complete(s.t.state(), v1.id, Actor::staff(s.rita.id), at(day(1), 14))
        .await
        .unwrap();
    assert_eq!(AuthorisationHold::held(s.t.db(), a1.id, None).await.unwrap(), 0);
    s.check_in(&v2).await.assert_status(303);
}

/// Done when 4: A1 with 2 units left and 3 units needed: 2 stay in the payer
/// split, 1 goes self-pay with consent.
#[renox::test]
async fn units_above_the_units_left_go_self_pay_with_consent() {
    let s = scene().await;
    let a1 = s.a1(2, 30).await;
    let booking = s.insured(&s.procedure, 1, 9).await;
    s.eligible(&booking).await;
    s.services(&booking, "procedure:3").await;

    s.t.app
        .get(&format!("/clinics/appointments/{}", booking.id))
        .await
        .assert_ok()
        .assert_see("2 of 3 units covered");
    assert_refused(&s.check_in(&booking).await);
    s.sign(&booking).await.assert_ok();
    s.check_in(&booking).await.assert_status(303);

    let visit = s.visit(&booking).await;
    let holds = AuthorisationHold::of_visit(s.t.db(), visit.id).await.unwrap();
    assert_eq!(holds.len(), 1);
    assert_eq!((holds[0].authorisation_id, holds[0].units), (a1.id, 2));
    assert_eq!(holds[0].service_code, "procedure");
    let consent = SelfPayConsent::find_or_404(s.t.db(), visit.consent_id.unwrap())
        .await
        .unwrap();
    assert!(consent.lists("procedure"));
    s.t.app
        .get(&format!("/clinics/visits/{}", visit.id))
        .await
        .assert_ok()
        .assert_see("procedure: PA-");
}

/// Done when 5: a covered consultation and an unauthorised procedure: the
/// consent lists only the procedure and the consultation stays in the payer
/// split (outcome insured).
#[renox::test]
async fn only_the_unauthorised_service_goes_on_the_consent() {
    let s = scene().await;
    let booking = s.insured(&s.consultation, 1, 9).await;
    s.eligible(&booking).await;
    s.services(&booking, "consultation:1, procedure:1").await;

    assert_refused(&s.check_in(&booking).await);
    s.sign(&booking).await.assert_ok();
    s.check_in(&booking).await.assert_status(303);

    let visit = s.visit(&booking).await;
    assert_eq!(visit.coverage_outcome, Some(CoverageOutcome::Insured));
    assert_eq!(visit.coverage_policy_id, Some(s.policy.id));
    let consent = SelfPayConsent::find_or_404(s.t.db(), visit.consent_id.unwrap())
        .await
        .unwrap();
    assert_eq!(consent.codes(), ["procedure"]);
    assert!(!consent.lists("consultation"));
    assert_eq!(consent.estimated_fils, 120_000);

    // Nothing to sign when everything is covered: refused.
    let other = s.insured(&s.consultation, 1, 11).await;
    s.eligible(&other).await;
    s.sign(&other).await.assert_status(422);
}

/// Done when 6: on a reschedule the billing coordinator gets a task to request
/// a new authorisation (here the payer's list was extended after the booking,
/// so the reschedule is what raises it), and the appointment is flagged.
#[renox::test]
async fn a_reschedule_asks_the_billing_coordinator_for_a_new_authorisation() {
    let s = scene().await;
    authorisations::stop_requiring(s.t.state(), PAYER, "procedure")
        .await
        .unwrap();
    let booking = s.insured(&s.procedure, 1, 9).await;
    assert!(AppointmentFlag::open_of(s.t.db(), booking.id).await.unwrap().is_empty());
    authorisations::require_for(s.t.state(), PAYER, "procedure")
        .await
        .unwrap();
    s.eligible(&booking).await;
    assert_refused(&s.check_in(&booking).await);
    assert!(s.t.tasks.with_key("clinics.prior_auth_needed:").is_empty());

    s.post(
        &format!("/scheduling/bookings/{}/reschedule", booking.id),
        &[("starts_at", local_text(at(day(3), 9)).as_str())],
    )
    .await
    .assert_status(303);

    let tasks = s.t.tasks.with_key("clinics.prior_auth_needed:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee, Some(Assignee::user(s.bea.id)));
    assert!(tasks[0].title.contains("Request a prior authorisation"), "{}", tasks[0].title);
    let flags = AppointmentFlag::open_of(s.t.db(), booking.id).await.unwrap();
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].origin, "rescheduled");
}
