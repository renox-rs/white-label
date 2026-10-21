//! A day at the clinic for the charge capture tests: the receptionist, the
//! practitioner, a consultation priced at AED 450.00, insured and self-pay
//! visits driven through the real routes.

use renox::db::now;
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_kernel::PartyId;
use wl_scheduling::service::{self, Actor};
use wl_scheduling::{Booking, Resource, ServiceType};

use clinics::contracts::{self, NewContract};
use clinics::policies::{self, NewPolicy};
use clinics::{InsurancePolicy, PayerContract, Visit, appointments};

use super::{Clinic, PATIENT, at, boot, day};

/// The payer of the insured visits.
pub const PAYER: PartyId = PartyId(900);

/// The clinic's day.
pub struct Desk {
    /// The app.
    pub t: Clinic,
    /// The receptionist.
    pub rita: User,
    /// The practitioner.
    pub dana: User,
    /// The doctor's resource.
    pub doctor: Resource,
    /// The consultation (deposit AED 100.00).
    pub consultation: ServiceType,
}

/// Boots the clinic with the self-pay price of the consultation.
pub async fn desk() -> Desk {
    let t = boot().await;
    let rita = t.receptionist().await;
    let dana = t.doctor_user().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let consultation = t.consultation(&[&doctor]).await;
    t.prices.set("consultation", day(-400), 45_000);
    Desk {
        t,
        rita,
        dana,
        doctor,
        consultation,
    }
}

impl Desk {
    /// Posts `fields` to `path` as `who`.
    pub async fn post(&self, who: &User, path: &str, fields: &[(&str, &str)]) -> TestResponse {
        self.t.app.acting_as(who);
        self.t.app.htmx().post(path, fields).await
    }

    /// Reads `path` as `who`.
    pub async fn get(&self, who: &User, path: &str) -> TestResponse {
        self.t.app.acting_as(who);
        self.t.app.get(path).await
    }

    /// A service type of `code` offered by the doctor, with a deposit of
    /// `deposit` fils and the walk-out setting (the remainder is refunded).
    pub async fn service(&self, code: &str, deposit: i64) -> ServiceType {
        let mut service = self.t.service_type(code, &[&self.doctor]).await;
        service.deposit_amount = deposit;
        service.save(self.t.db()).await.unwrap();
        service
    }

    /// A policy of the patient with the payer `payer`, valid for months.
    pub async fn policy(&self, plan: &str, payer: PartyId) -> InsurancePolicy {
        policies::create(
            self.t.state(),
            None,
            NewPolicy::new(PATIENT, payer, plan, "M-1", day(-200)).valid_to(day(300)),
        )
        .await
        .expect("the policy is recorded")
    }

    /// A contract of `payer` for every plan with a copay of `copay_bp`,
    /// billed under the payer agreement `agreement`.
    pub async fn contract(&self, payer: PartyId, copay_bp: i64, agreement: i64) -> PayerContract {
        contracts::create(
            self.t.state(),
            None,
            NewContract::new(payer, day(-300))
                .copay_bp(copay_bp)
                .agreement(agreement),
        )
        .await
        .expect("the contract is recorded")
    }

    /// A booking of `service` tomorrow at `hour` for the patient, paid for by
    /// a deposit of the service's amount held for it.
    pub async fn booking(&self, service: &ServiceType, hour: u32) -> Booking {
        let booking = self
            .t
            .book(service, &self.doctor, at(day(1), hour))
            .await;
        if booking.deposit_amount > 0 {
            self.t
                .deposits
                .paid(booking.id, PATIENT, booking.deposit_amount);
        }
        booking
    }

    /// Like [`booking`](Desk::booking), insured with `policy` and with a
    /// positive eligibility result.
    pub async fn insured(
        &self,
        service: &ServiceType,
        hour: u32,
        policy: &InsurancePolicy,
    ) -> Booking {
        let booking = self.booking(service, hour).await;
        appointments::attach_policy(self.t.state(), booking.id, policy.id)
            .await
            .expect("the appointment is insured");
        self.post(
            &self.rita,
            &format!("/clinics/appointments/{}/eligibility", booking.id),
            &[("result", "positive")],
        )
        .await
        .assert_ok();
        booking
    }

    /// The receptionist checks the patient in; the visit.
    pub async fn check_in(&self, booking: &Booking) -> Visit {
        self.post(
            &self.rita,
            &format!("/scheduling/bookings/{}/check-in", booking.id),
            &[("x", "y")],
        )
        .await
        .assert_status(303);
        self.visit_of(booking).await
    }

    /// The visit of `booking`.
    pub async fn visit_of(&self, booking: &Booking) -> Visit {
        Visit::of_booking(self.t.db(), booking.id)
            .await
            .unwrap()
            .expect("the patient is checked in")
    }

    /// The visit, read again.
    pub async fn visit(&self, id: i64) -> Visit {
        Visit::find_or_404(self.t.db(), id).await.unwrap()
    }

    /// The service starts.
    pub async fn start(&self, booking: &Booking) {
        service::start(self.t.state(), booking.id, Actor::staff(self.rita.id), now())
            .await
            .expect("the service starts");
    }

    /// The service is complete (the booking's end is a minute away, so the
    /// clock the call carries is the one the booking is judged by).
    pub async fn complete(&self, booking: &Booking) {
        service::complete(
            self.t.state(),
            booking.id,
            Actor::staff(self.rita.id),
            at(day(1), 18),
        )
        .await
        .expect("the service is complete");
    }

    /// The practitioner records `units` of the service `code` on the visit.
    pub async fn deliver(&self, visit: &Visit, code: &str, units: i64) -> TestResponse {
        self.post(
            &self.dana,
            &format!("/clinics/visits/{}/charges", visit.id),
            &[
                ("kind", "service"),
                ("code", code),
                ("units", units.to_string().as_str()),
            ],
        )
        .await
    }

    /// The desk records `units` of the consumable `code` on the visit.
    pub async fn issue(&self, visit: &Visit, code: &str, units: i64) -> TestResponse {
        self.post(
            &self.rita,
            &format!("/clinics/visits/{}/charges", visit.id),
            &[
                ("kind", "consumable"),
                ("code", code),
                ("units", units.to_string().as_str()),
            ],
        )
        .await
    }

    /// Staff cancel the booking with `reason`.
    pub async fn cancel(&self, booking: &Booking, reason: &str) -> TestResponse {
        self.post(
            &self.rita,
            &format!("/scheduling/bookings/{}/cancel", booking.id),
            &[("reason_code", reason)],
        )
        .await
    }
}
