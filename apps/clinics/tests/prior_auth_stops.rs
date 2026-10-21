//! BL-CLN-03 (#351): `CLN.RUL.09` flags the appointments that relied on a
//! prior authorisation when it is denied, used up or ends, and three days
//! before its validity ends; one task for the billing coordinator per
//! authorisation and trigger; a re-run changes nothing; an authorisation that
//! covers the day clears the flag. Through the real routes, the real
//! authorisation machine and the daily run.

mod support;

use renox::db::now;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::automation::RunOutcome;
use wl_kernel::tickets::Assignee;

use clinics::authorisations;
use clinics::flags::AppointmentFlag;
use clinics::policies::{self, NewPolicy};
use clinics::prior_auth_rules::{Trigger, run_for, task_key};
use clinics::{AuthStatus, InsurancePolicy, PriorAuthorisation, appointments};
use support::{Clinic, PATIENT, at, boot, day};

const PAYER: PartyId = PartyId(900);

/// A doctor who offers laser and consultation, and the patient's policy.
struct Scene {
    t: Clinic,
    doctor: wl_scheduling::Resource,
    consultation: wl_scheduling::ServiceType,
    laser: wl_scheduling::ServiceType,
    policy: InsurancePolicy,
}

async fn scene() -> Scene {
    let t = boot().await;
    t.coordinator().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let consultation = t.consultation(&[&doctor]).await;
    let laser = t.service_type("laser", &[&doctor]).await;
    let policy = policies::create(
        t.state(),
        None,
        NewPolicy::new(PATIENT, PAYER, "Gold", "M-1", day(-200)).valid_to(day(300)),
    )
    .await
    .unwrap();
    Scene {
        t,
        doctor,
        consultation,
        laser,
        policy,
    }
}

impl Scene {
    /// An insured booking of laser for the patient on `date` at 09:00.
    async fn laser_on(&self, date: i64) -> i64 {
        let booking = self
            .t
            .book(&self.laser, &self.doctor, at(day(date), 9))
            .await;
        appointments::attach_policy(self.t.state(), booking.id, self.policy.id)
            .await
            .unwrap();
        booking.id
    }

    async fn flags(&self, booking: i64) -> Vec<AppointmentFlag> {
        AppointmentFlag::open_of(self.t.db(), booking)
            .await
            .unwrap()
    }

    async fn deny(&self, a: &PriorAuthorisation) {
        let bea = self.t.coordinator().await;
        self.t.app.acting_as(&bea);
        self.t
            .app
            .htmx()
            .post(
                &format!("/clinics/authorisations/{}/deny", a.id),
                &[("reason", "Not covered")],
            )
            .await
            .assert_ok();
    }

    fn task(
        &self,
        a: &PriorAuthorisation,
        trigger: Trigger,
    ) -> Vec<wl_kernel::tickets::OpenTicket> {
        self.t.tasks.with_key(&task_key(a.id, trigger))
    }
}

/// Done when 1: A1 (patient, laser, valid until day 10) is denied: both of the
/// patient's laser appointments, inside and after the validity, are flagged
/// and the coordinator gets one task listing both. Another patient's
/// appointment, another service, a self-pay appointment and a cancelled one
/// are left alone.
#[renox::test]
async fn a_denied_authorisation_flags_the_appointments_that_relied_on_it() {
    let s = scene().await;
    let bea = s.t.coordinator().await;
    let inside = s.laser_on(6).await;
    let after = s.laser_on(14).await;
    // Left alone: a consultation, a self-pay laser, another patient's laser
    // and a cancelled one.
    let consultation = s.t.book(&s.consultation, &s.doctor, at(day(8), 9)).await;
    appointments::attach_policy(s.t.state(), consultation.id, s.policy.id)
        .await
        .unwrap();
    let self_pay = s.t.book(&s.laser, &s.doctor, at(day(9), 9)).await;
    let stranger =
        s.t.book_for(PartyId(71), &s.laser, &s.doctor, at(day(10), 9))
            .await;
    let theirs = policies::create(
        s.t.state(),
        None,
        NewPolicy::new(PartyId(71), PAYER, "Gold", "M-9", day(-200)).valid_to(day(300)),
    )
    .await
    .unwrap();
    appointments::attach_policy(s.t.state(), stranger.id, theirs.id)
        .await
        .unwrap();
    let cancelled = s.laser_on(11).await;
    s.t.app.acting_as(&s.t.receptionist().await);
    s.t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{cancelled}/cancel"),
            &[("reason_code", clinics::reasons::COVERAGE_NOT_CONFIRMED)],
        )
        .await
        .assert_status(303);

    let a1 = s.t.requested(&s.policy, "laser", 3).await;
    s.deny(&a1).await;
    assert_eq!(
        PriorAuthorisation::find_or_404(s.t.db(), a1.id)
            .await
            .unwrap()
            .status,
        AuthStatus::Denied
    );

    for booking in [inside, after] {
        let flags = s.flags(booking).await;
        assert_eq!(flags.len(), 1, "booking {booking}");
        assert_eq!(flags[0].origin, "denied");
        assert_eq!(flags[0].authorisation_id, Some(a1.id));
    }
    for booking in [consultation.id, self_pay.id, stranger.id, cancelled] {
        assert!(s.flags(booking).await.is_empty(), "booking {booking}");
    }
    let tasks = s.task(&a1, Trigger::Denied);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee, Some(Assignee::user(bea.id)));
    assert!(
        tasks[0].description.contains(&day(6).to_string()),
        "{}",
        tasks[0].description
    );
    assert!(
        tasks[0].description.contains(&day(14).to_string()),
        "{}",
        tasks[0].description
    );
    s.t.app
        .get(&format!("/clinics/appointments/{after}"))
        .await
        .assert_ok()
        .assert_see("Needs authorisation");
}

/// Done when 2: three days before A1's last valid day (day 10), at 02:00 of
/// day 7, the appointment after it is flagged; the one inside the validity is
/// not. The run is the daily task; a second run does nothing.
#[renox::test]
async fn three_days_before_it_ends_only_the_appointments_after_it_are_flagged() {
    let s = scene().await;
    let a1 = s.t.authorisation(&s.policy, "laser", 3, day(10)).await;
    let inside = s.laser_on(8).await;
    let after = s.laser_on(14).await;

    // Two days before: nothing yet.
    let early = authorisations::run_daily(s.t.state(), at(day(6), 2))
        .await
        .unwrap();
    assert!(early.looked_ahead.is_empty());
    assert!(s.flags(after).await.is_empty());

    let daily = authorisations::run_daily(s.t.state(), at(day(7), 2))
        .await
        .unwrap();
    assert_eq!(daily.looked_ahead, [a1.id]);
    assert!(s.flags(inside).await.is_empty(), "still covered");
    let flags = s.flags(after).await;
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].origin, "look_ahead");
    assert_eq!(s.task(&a1, Trigger::LookAhead).len(), 1);

    // The same day again: nothing new.
    let again = authorisations::run_daily(s.t.state(), at(day(7), 2))
        .await
        .unwrap();
    assert!(again.looked_ahead.is_empty());
    assert_eq!(s.flags(after).await.len(), 1);
    assert_eq!(s.task(&a1, Trigger::LookAhead).len(), 1);
}

/// Done when 3: the last unit is consumed (charge capture's call): A1 is
/// `used` and the later appointment is flagged; the visit that used the unit
/// is checked in and is not.
#[renox::test]
async fn the_last_unit_consumed_flags_the_later_appointments() {
    let s = scene().await;
    let a1 = s.t.authorisation(&s.policy, "laser", 1, day(30)).await;
    let today_visit = s.laser_on(1).await;
    let later = s.laser_on(14).await;
    let rita = s.t.receptionist().await;
    s.t.app.acting_as(&rita);
    s.t.app
        .htmx()
        .post(
            &format!("/clinics/appointments/{today_visit}/eligibility"),
            &[("result", "positive")],
        )
        .await
        .assert_ok();
    s.t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{today_visit}/check-in"),
            &[("x", "y")],
        )
        .await
        .assert_status(303);
    assert!(s.flags(later).await.is_empty(), "A1 still covers it");

    let used = authorisations::consume(s.t.state(), a1.id, 1, now())
        .await
        .unwrap();
    assert_eq!(used.status, AuthStatus::Used);
    let flags = s.flags(later).await;
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].origin, "used");
    assert!(s.flags(today_visit).await.is_empty(), "checked in already");
    assert_eq!(s.task(&a1, Trigger::Used).len(), 1);
}

/// An authorisation that ends: the daily run expires it and flags the
/// appointments after the last valid day (not the ones inside it).
#[renox::test]
async fn an_expired_authorisation_flags_the_appointments_after_its_last_day() {
    let s = scene().await;
    let a1 = s.t.authorisation(&s.policy, "laser", 3, day(10)).await;
    let inside = s.laser_on(6).await;
    let after = s.laser_on(14).await;

    let daily = authorisations::run_daily(s.t.state(), at(day(11), 2))
        .await
        .unwrap();
    assert_eq!(daily.expired, [a1.id]);
    assert!(s.flags(inside).await.is_empty());
    let flags = s.flags(after).await;
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].origin, "expired");
    assert_eq!(s.task(&a1, Trigger::Expired).len(), 1);
}

/// Done when 4: another authorisation A2, valid on the day with units left:
/// nothing is flagged and no task is opened.
#[renox::test]
async fn another_valid_authorisation_keeps_the_appointment_unflagged() {
    let s = scene().await;
    let a1 = s.t.requested(&s.policy, "laser", 3).await;
    s.t.authorisation(&s.policy, "laser", 2, day(20)).await;
    let booking = s.laser_on(14).await;

    s.deny(&a1).await;
    assert!(s.flags(booking).await.is_empty());
    assert!(s.task(&a1, Trigger::Denied).is_empty());
}

/// Done when 5: running the rule twice creates one flag and one task.
#[renox::test]
async fn running_the_rule_twice_creates_one_flag_and_one_task() {
    let s = scene().await;
    let a1 = s.t.requested(&s.policy, "laser", 3).await;
    let booking = s.laser_on(14).await;
    s.deny(&a1).await;
    assert_eq!(s.flags(booking).await.len(), 1);

    let (outcome, flagged) = run_for(s.t.state(), a1.id, Trigger::Denied, day(0))
        .await
        .unwrap();
    assert_eq!(outcome, RunOutcome::Skipped);
    assert!(flagged.bookings.is_empty());
    assert_eq!(s.flags(booking).await.len(), 1);
    assert_eq!(s.task(&a1, Trigger::Denied).len(), 1);
    let all = AppointmentFlag::of_booking(s.t.db(), booking)
        .await
        .unwrap();
    assert_eq!(all.len(), 1, "one flag ever raised");
}

/// Done when 6: A3, approved and covering the day, clears the flag and closes
/// the coordinator's work on it; a reschedule to a covered day clears it too.
#[renox::test]
async fn a_new_authorisation_clears_the_flag() {
    let s = scene().await;
    let a1 = s.t.requested(&s.policy, "laser", 3).await;
    let booking = s.laser_on(14).await;
    s.deny(&a1).await;
    assert_eq!(s.flags(booking).await.len(), 1);
    // The payer wants laser authorised from now on.
    authorisations::require_for(s.t.state(), PAYER, "laser")
        .await
        .unwrap();

    s.t.authorisation(&s.policy, "laser", 2, day(20)).await;
    assert!(s.flags(booking).await.is_empty());
    let all = AppointmentFlag::of_booking(s.t.db(), booking)
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].clear_reason.as_deref(), Some("covered"));
}
