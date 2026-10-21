//! Booting the clinics app in a `TestApp` with the real `wl-scheduling` (the
//! bookings the visits come from), the real `wl-hr` (the practitioner
//! licences behind `qualification_valid`) and the fakes the app talks to
//! through the kernel's contracts: the document desk, the notifier, the task
//! creator, the payment recorder (deposit refunds) and the self-pay price
//! list.
//!
//! The fakes of wl-scheduling's own support are reused as they are: the file
//! is included below, so the two crates test against the same stand-ins.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use renox::auth::{Auth, Permissions, permissions};
use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::DateTime;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::documents::{dubai_date, provide_guard};
use wl_kernel::notify::provide_notifier;
use wl_kernel::payments::provide_payment_recorder;
use wl_kernel::sales::{SalesFuture, SelfPayPrices, provide_self_pay_prices};
use wl_kernel::scheduling::{BookingId, Initiator, provide_work_done_status};
use wl_kernel::tickets::provide_task_creator;
use wl_kernel::workforce::{EmployeeId, QualificationExpired};
use wl_kernel::{PartyId, Subject};
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{
    AvailabilityRule, Booking, Resource, ResourceType, ServiceType, WlScheduling, resource,
};

/// wl-scheduling's own stand-ins (documents, tasks, messages, refunds, the
/// events recorded).
#[path = "../../../../crates/wl-scheduling/tests/support/mod.rs"]
pub mod scheduling;

pub use scheduling::{Docs, Events, Messages, Recorded, Refunds, Tasks, Work, dubai, record};

/// The kind of the practitioner's licence.
pub const LICENCE: &str = clinics::seed::LICENCE_KIND;
/// The kind of the branch privileges.
pub const PRIVILEGES: &str = clinics::seed::PRIVILEGES_KIND;

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A clinic with scheduling and HR on.
const CLIENT: &str = r#"
[client]
name = "Palm Grove Clinic LLC"
app = "clinics"

[modules]
enabled = ["parties", "hr", "scheduling"]

[roles]
owner = "owner"

[[legal_entities]]
code = "PGC"
name_en = "Palm Grove Clinic LLC"
default = true
"#;

/// The branch the practitioners work at.
pub fn branch() -> Subject {
    scheduling::branch()
}

/// The second branch.
pub fn other_branch() -> Subject {
    Subject::new("test.branch", 3)
}

/// The patient of the tests (a party).
pub const PATIENT: PartyId = PartyId(70);

/// The self-pay price list: `(code, from, price)`; the latest row that
/// starts on or before the day gives the price.
#[derive(Clone, Default)]
pub struct Prices(Arc<Mutex<Vec<(String, NaiveDate, i64)>>>);

impl Prices {
    /// From `from` on, `code` costs `fils` self-pay.
    pub fn set(&self, code: &str, from: NaiveDate, fils: i64) {
        self.0.lock().unwrap().push((code.to_owned(), from, fils));
    }
}

impl SelfPayPrices for Prices {
    fn price_on<'a>(
        &'a self,
        _db: &'a Db,
        code: &'a str,
        on: NaiveDate,
    ) -> SalesFuture<'a, Option<i64>> {
        Box::pin(async move {
            let all = self.0.lock().unwrap();
            Ok(all
                .iter()
                .filter(|(c, from, _)| c == code && *from <= on)
                .max_by_key(|(_, from, _)| *from)
                .map(|(_, _, price)| *price))
        })
    }
}

/// Provides the fakes.
struct Harness {
    refunds: Refunds,
    work: Work,
    messages: Messages,
    docs: Docs,
    tasks: Tasks,
    prices: Prices,
}

impl Module for Harness {
    fn name(&self) -> &'static str {
        "clinics-test-harness"
    }

    fn register(&self, app: &mut Registry) {
        provide_payment_recorder(app, self.refunds.clone());
        provide_work_done_status(app, self.work.clone());
        provide_notifier(app, self.messages.clone());
        provide_guard(app, self.docs.clone());
        provide_task_creator(app, self.tasks.clone());
        provide_self_pay_prices(app, self.prices.clone());
    }
}

/// The app under test.
pub struct Clinic {
    /// The app.
    pub app: TestApp,
    /// The events wl-scheduling emitted.
    pub events: Events,
    /// The refunds asked of the payment recorder.
    pub refunds: Refunds,
    /// The notifications sent.
    pub messages: Messages,
    /// The documents the employees hold.
    pub docs: Docs,
    /// The staff tasks opened.
    pub tasks: Tasks,
    /// The self-pay price list.
    pub prices: Prices,
    /// The `QualificationExpired` events wl-hr emitted.
    pub expired: Recorded<QualificationExpired>,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
}

/// The day `days` from today in Asia/Dubai.
pub fn day(days: i64) -> NaiveDate {
    dubai_date(renox::db::now()) + TimeDelta::days(days)
}

/// Asia/Dubai `date` at `hour:00`, as an instant.
pub fn at(date: NaiveDate, hour: u32) -> DateTime {
    dubai(&date.to_string(), hour, 0)
}

/// Boots the clinics app: scheduling, HR and the fakes, with the 05 reason
/// codes and a self-pay price of AED 450.00 for the consultation.
pub async fn boot() -> Clinic {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let events = Events::default();
    let refunds = Refunds::default();
    let work = Work::default();
    let messages = Messages::default();
    let docs = Docs::default();
    let tasks = Tasks::default();
    let prices = Prices::default();
    let expired = Recorded::<QualificationExpired>::default();
    let mut scheduling_module = WlScheduling::new();
    for (code, initiated_by) in clinics::reasons::SCHEDULING_REASON_CODES {
        scheduling_module = scheduling_module.reason_code(code, *initiated_by);
    }
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(wl_hr::WlHr::new())
        .module(scheduling_module)
        .module(clinics::Clinics)
        .module(Harness {
            refunds: refunds.clone(),
            work,
            messages: messages.clone(),
            docs: docs.clone(),
            tasks: tasks.clone(),
            prices: prices.clone(),
        });
    let app = record(app, &events.booked);
    let app = record(app, &events.rescheduled);
    let app = record(app, &events.checked_in);
    let app = record(app, &events.completed);
    let app = record(app, &events.cancelled);
    let app = record(app, &events.conflicts);
    let app = record(app, &expired);
    let config = wl_kernel::ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    Clinic {
        app,
        events,
        refunds,
        messages,
        docs,
        tasks,
        prices,
        expired,
        _views: views,
    }
}

impl Clinic {
    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// The app state.
    pub fn state(&self) -> &AppState {
        self.app.state()
    }

    /// A user holding exactly `grants` (through a role of their own). The
    /// same name gives the same user.
    pub async fn user(&self, name: &str, grants: &[&str]) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        if let Some(found) = User::find_by_email(self.db(), &email).await.unwrap() {
            return found;
        }
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        let role = format!("role_{}", name.to_lowercase());
        permissions::define_role(self.db(), &role, grants)
            .await
            .unwrap();
        user.assign_role(self.db(), &role).await.unwrap();
        user
    }

    /// The receptionist: books, checks in and records the coverage.
    pub async fn receptionist(&self) -> User {
        self.user(
            "Rita",
            &[
                wl_scheduling::permissions::VIEW,
                wl_scheduling::permissions::MANAGE,
                wl_scheduling::permissions::CHECKIN,
                clinics::permissions::VISITS_VIEW,
                clinics::permissions::POLICIES_VIEW,
                clinics::permissions::POLICIES_MANAGE,
                clinics::permissions::COVERAGE_RECORD,
            ],
        )
        .await
    }

    /// The billing coordinator: prepares the prior authorisations and holds
    /// the role the rules send their tasks to.
    pub async fn coordinator(&self) -> User {
        let grants = [
            wl_scheduling::permissions::VIEW,
            clinics::permissions::VISITS_VIEW,
            clinics::permissions::POLICIES_VIEW,
            clinics::permissions::POLICIES_MANAGE,
            clinics::permissions::AUTHORISATIONS_MANAGE,
        ];
        let email = "bea@example.com";
        if let Some(found) = User::find_by_email(self.db(), email).await.unwrap() {
            return found;
        }
        let user = self.user("Bea", &grants).await;
        permissions::define_role(
            self.db(),
            clinics::permissions::BILLING_COORDINATOR_ROLE,
            &grants,
        )
        .await
        .unwrap();
        user.assign_role(self.db(), clinics::permissions::BILLING_COORDINATOR_ROLE)
            .await
            .unwrap();
        user
    }

    /// An authorisation of `units` of `service` on `policy`, prepared,
    /// requested and approved through the billing coordinator's routes,
    /// valid through `until`.
    pub async fn authorisation(
        &self,
        policy: &clinics::InsurancePolicy,
        service: &str,
        units: i64,
        until: NaiveDate,
    ) -> clinics::PriorAuthorisation {
        let bea = self.coordinator().await;
        let created = clinics::authorisations::create(
            self.state(),
            Some(bea.id),
            clinics::authorisations::NewAuthorisation::new(policy.id, units).service(service),
        )
        .await
        .expect("the authorisation is prepared");
        self.app.acting_as(&bea);
        self.app
            .htmx()
            .post(
                &format!("/clinics/authorisations/{}/request", created.id),
                &[("reference", "REQ")],
            )
            .await
            .assert_ok();
        self.app
            .htmx()
            .post(
                &format!("/clinics/authorisations/{}/approve", created.id),
                &[("reference", "A-1"), ("valid_until", until.to_string().as_str())],
            )
            .await
            .assert_ok();
        clinics::PriorAuthorisation::find_or_404(self.db(), created.id)
            .await
            .unwrap()
    }

    /// Someone who may only look at the appointments.
    pub async fn viewer(&self) -> User {
        self.user("Vera", &[wl_scheduling::permissions::VIEW])
            .await
    }

    /// An active employee of the real `wl-hr`.
    pub async fn employee(&self, party: i64) -> wl_hr::Employee {
        let mut employee = wl_hr::Employee {
            party_id: party,
            job_role: "practitioner".into(),
            job_title: "Doctor".into(),
            hire_date: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
            status: wl_hr::EmployeeStatus::Active,
            ..wl_hr::Employee::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        employee
            .insert_in_state(&mut tx, wl_hr::EmployeeStatus::Active, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        employee
    }

    /// Records a qualification of `kind` for `employee` at `branch`, whose
    /// document is valid through `until` (for ever when `None`).
    pub async fn qualification(
        &self,
        employee: &wl_hr::Employee,
        kind: &str,
        branch: &Subject,
        until: Option<NaiveDate>,
    ) {
        let mut licence =
            wl_hr::qualification::NewQualification::new(employee.employee_id(), kind)
                .role("practitioner")
                .scope(branch.clone())
                .number(&format!("{kind}-{}", employee.party_id));
        if let Some(until) = until {
            licence = licence.valid_to(until);
        }
        wl_hr::qualification::record(self.db(), licence)
            .await
            .unwrap();
        self.docs
            .accepted_until(&employee.subject(), kind, employee.party_id, until);
    }

    /// A practitioner: a real employee of `wl-hr` with the licence and the
    /// privileges of `at_branch`, both valid through `until` (for ever when
    /// `None`), and a person resource there open every day of the week.
    pub async fn practitioner_at(
        &self,
        name: &str,
        party: i64,
        at_branch: &Subject,
        until: Option<NaiveDate>,
    ) -> Resource {
        let employee = self.employee(party).await;
        self.qualification(&employee, LICENCE, at_branch, until)
            .await;
        self.qualification(&employee, PRIVILEGES, at_branch, until)
            .await;
        let mut resource = Resource::new(name, ResourceType::Person)
            .employee(employee.id)
            .role("practitioner")
            .at(at_branch);
        resource.save(self.db()).await.unwrap();
        for weekday in 1..=7 {
            let mut rule = AvailabilityRule::new(resource.id, weekday, 0, 1440);
            rule.save(self.db()).await.unwrap();
        }
        resource
    }

    /// A practitioner at [`branch`].
    pub async fn practitioner(&self, name: &str, party: i64, until: Option<NaiveDate>) -> Resource {
        self.practitioner_at(name, party, &branch(), until).await
    }

    /// The consultation: 30 minutes, checked in and tracked, with a deposit
    /// of AED 100.00, needing the licence and the privileges, offered by
    /// `doctors`.
    pub async fn consultation(&self, doctors: &[&Resource]) -> ServiceType {
        let existing = ServiceType::query()
            .where_(ServiceType::CODE.eq("consultation"))
            .first(self.db())
            .await
            .unwrap();
        let service = match existing {
            Some(service) => service,
            None => {
                let mut service = ServiceType::new("consultation", "Consultation", 30);
                service.required_qualifications = clinics::seed::REQUIRED_QUALIFICATIONS.to_owned();
                service.check_in_required = true;
                service.track_in_progress = true;
                service.deposit_amount = 10_000;
                service.deposit_forfeit_hours = 24;
                service.cancel_cutoff_hours = 2;
                service.save(self.db()).await.unwrap();
                service
            }
        };
        for doctor in doctors {
            resource::offer(self.db(), service.id, doctor.id)
                .await
                .unwrap();
        }
        service
    }

    /// A booking of `service` with `doctor` from `start` for [`PATIENT`]
    /// (the patient is the booking's subject), made by staff.
    pub async fn book(&self, service: &ServiceType, doctor: &Resource, start: DateTime) -> Booking {
        self.book_for(PATIENT, service, doctor, start).await
    }

    /// Like [`book`](Clinic::book) for `patient`.
    pub async fn book_for(
        &self,
        patient: PartyId,
        service: &ServiceType,
        doctor: &Resource,
        start: DateTime,
    ) -> Booking {
        service::book(
            self.state(),
            NewBooking::new(patient, service.id, &[doctor.id], start)
                .for_subject(patient.subject()),
            Actor::staff(1),
            scheduling::now(),
        )
        .await
        .expect("the booking is made")
    }

    /// The booking, read again.
    pub async fn booking(&self, id: i64) -> Booking {
        Booking::find_or_404(self.db(), id).await.unwrap()
    }

    /// The booking as the kernel names it.
    pub fn id(&self, booking: &Booking) -> BookingId {
        booking.booking_id()
    }

    /// The employee behind a practitioner.
    pub fn employee_id(&self, doctor: &Resource) -> EmployeeId {
        EmployeeId(doctor.employee_id.expect("an employee"))
    }
}

/// The reason codes 05 adds to wl-scheduling, as the host sets them.
pub fn reason_codes() -> BTreeMap<&'static str, Initiator> {
    clinics::reasons::SCHEDULING_REASON_CODES
        .iter()
        .copied()
        .collect()
}
