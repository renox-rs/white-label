//! 10 EdTech against the REAL shared modules of the host (wl-documents,
//! wl-payments, wl-invoicing, wl-sales, wl-parties): the points of #436, #438,
//! #440, #441, #442 and #443 whose outcome belongs to those modules and that the
//! app's own tests (`apps/edtech/tests`) can only show against stand-ins. The
//! stand-in tests stay; these run the same flows with nothing faked but the
//! payment gateway.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use edtech::batches::{self, NewBatch};
use edtech::courses::{self, NewCourse};
use edtech::enrolments::{self, NewEnrolment};
use edtech::{Batch, Course, Enrolment, EnrolmentStatus};
use host::roles::{ACADEMIC_COORDINATOR, COUNSELLOR};
use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use renox::testing::TestApp;
use support::{day, noon_clock, person, scoped_person, tenant};
use wl_kernel::PartyId;
use wl_kernel::payments::{PaymentFailureKind, PaymentStatus};
use wl_payments::Payment;
use wl_payments::gateway::FakeGateway;
use wl_payments::online;
use wl_payments::payment_link::PaymentLink;
use wl_payments::refund::Refund;

/// The demo training centre.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-edtech/client.toml"
);

/// A test-only module with the route that hands a test the signed-in user the
/// services take (a test cannot build an `AuthUser`).
struct Who(Arc<Mutex<Option<AuthUser>>>);

impl Module for Who {
    fn name(&self) -> &'static str {
        "edtech-real-who"
    }

    fn routes(&self) -> Routes {
        let slot = self.0.clone();
        Routes::new().get("/t/whoami", move |user: AuthUser| {
            let slot = slot.clone();
            async move {
                *slot.lock().unwrap() = Some(user);
                "ok"
            }
        })
    }
}

/// The booted centre.
struct World {
    app: TestApp,
    gateway: FakeGateway,
    who: Arc<Mutex<Option<AuthUser>>>,
}

/// Boots the demo centre with `wl:install` and the seeds, a fake gateway.
async fn boot() -> World {
    boot_with(None).await
}

/// [`boot`] with `mail` as the e-mail provider, when given.
async fn boot_with(mail: Option<support::BouncingMail>) -> World {
    let gateway = FakeGateway::new();
    let who = Arc::new(Mutex::new(None));
    let config = wl_kernel::ClientConfig::load(CLIENT).expect("the edtech client");
    let tuned = {
        let gateway = gateway.clone();
        host::Tuning::default().payments(move |p| p.gateway(gateway))
    };
    let tuned = match mail {
        Some(mail) => tuned.notify(move |n| n.provider(mail)),
        None => tuned,
    };
    let app = TestApp::new(
        host::app_for_tuned(config, tuned)
            .expect("the app")
            .provide(noon_clock())
            .module(Who(who.clone())),
    )
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    World { app, gateway, who }
}

impl World {
    fn state(&self) -> &AppState {
        self.app.state()
    }

    fn db(&self) -> &Db {
        self.app.db()
    }

    /// `user` as the signed-in user the services take.
    async fn auth(&self, user: &User) -> AuthUser {
        self.app.acting_as(user);
        self.app.get("/t/whoami").await.assert_ok();
        self.who.lock().unwrap().take().expect("the probe saw them")
    }

    /// A user with the host role `role`.
    async fn staff(&self, name: &str, role: &str) -> (User, AuthUser) {
        let user = person(&self.app, name, role).await;
        let auth = self.auth(&user).await;
        (user, auth)
    }

    /// The centre permit of the default entity, valid through `until`.
    async fn permit_until(&self, until: NaiveDate) -> wl_documents::Document {
        let entity = wl_kernel::LegalEntity::default_entity(self.db())
            .await
            .unwrap();
        record_document(
            &self.app,
            "centre_permit",
            wl_kernel::Subject::new("kernel.legal_entity", entity.id),
            until,
        )
        .await
    }

    /// The adult course IELTS-B2 (defined once).
    async fn course(&self, coordinator: &AuthUser) -> Course {
        if let Some(found) = Course::by_code(self.db(), "IELTS-B2").await.unwrap() {
            return found;
        }
        courses::create(
            self.state(),
            coordinator,
            NewCourse::new("IELTS-B2", "IELTS preparation", "English language", 40).level("B2"),
        )
        .await
        .expect("the course is defined")
    }

    /// A planned batch of 12 seats on Monday and Wednesday, four weeks from
    /// the first Monday on or after `from`, with a tutor and a room of their
    /// own.
    async fn planned(&self, coordinator: &AuthUser, name: &str, from: NaiveDate) -> Batch {
        let mut tutor = wl_scheduling::Resource::new(
            &format!("Tutor {name}"),
            wl_scheduling::ResourceType::Person,
        );
        tutor.save(self.db()).await.unwrap();
        let mut room = wl_scheduling::Resource::new(
            &format!("Room {name}"),
            wl_scheduling::ResourceType::Room,
        );
        room.capacity = 14;
        room.save(self.db()).await.unwrap();
        let course = self.course(coordinator).await;
        let first = monday_from(from);
        batches::create(
            self.state(),
            coordinator,
            NewBatch::new(
                course.id,
                12,
                (first, first + TimeDelta::days(27)),
                &[1, 3],
                "18:00",
                90,
            )
            .tutor(tutor.id, None)
            .room(room.id, Some(14)),
        )
        .await
        .expect("the batch is planned")
    }

    /// An open batch (the centre permit is on record).
    async fn open_batch(&self, coordinator: &AuthUser, name: &str, from: NaiveDate) -> Batch {
        self.permit_until(day(400)).await;
        let planned = self.planned(coordinator, name, from).await;
        batches::open(self.state(), coordinator, planned.id)
            .await
            .expect("the batch opens")
    }
}

/// The first Monday on or after `from`.
fn monday_from(from: NaiveDate) -> NaiveDate {
    use renox::chrono::Datelike;
    let mut date = from;
    while date.weekday().number_from_monday() != 1 {
        date += TimeDelta::days(1);
    }
    date
}

/// An accepted document of `kind_key` for `subject`, valid through `until`.
async fn record_document(
    app: &TestApp,
    kind_key: &str,
    subject: wl_kernel::Subject,
    until: NaiveDate,
) -> wl_documents::Document {
    use wl_documents::{Document, DocumentKind, DocumentStatus};
    let kind = DocumentKind::by_key(app.db(), kind_key)
        .await
        .unwrap()
        .unwrap();
    Document::create(
        app.db(),
        Document {
            subject_type: subject.subject_type,
            subject_id: subject.subject_id,
            kind_id: kind.id,
            title: kind.key.clone(),
            expiry_date: Some(until),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

/// The enrolment as it is stored now.
async fn reload(w: &World, e: &Enrolment) -> Enrolment {
    Enrolment::find_or_404(w.db(), e.id).await.unwrap()
}

/// The link of `e` as the payer's page starts a payment on it (logged out,
/// as the guardian is), answering the payment that was made.
async fn start_payment(w: &World, e: &Enrolment) -> Payment {
    let path = e.payment_link_path.clone().expect("a payment link");
    w.app.logout();
    w.app
        .htmx()
        .post(&format!("{path}/start"), &[])
        .await
        .assert_status(303);
    let link = PaymentLink::query()
        .where_(PaymentLink::ID.eq(e.payment_link_id.expect("a link")))
        .first(w.db())
        .await
        .unwrap()
        .expect("the link");
    Payment::query()
        .where_(Payment::PAYMENT_LINK_ID.eq(link.id))
        .first(w.db())
        .await
        .unwrap()
        .expect("a payment on the link")
}

/// #436 points 4 and 5, with the real wl-payments: the first payment of E1 is
/// on its way at the gateway when the 48 hours pass; the gateway gives up
/// (timeout) and E1 lapses; the student is enrolled again (E2, own link, seat
/// at creation) and then the gateway confirms E1's payment late. wl-payments
/// itself requests the refund of the whole AED 2,520.00 (E1 is no active
/// subject any more) and approves it with no person; the money goes back to
/// the same card; E2 stays `pending_payment`; both counsellors are told.
#[renox::test]
async fn edu_01_a_late_payment_is_refunded_by_wl_payments_and_the_new_enrolment_stays() {
    let w = boot().await;
    let (_, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (dina_user, dina) = w.staff("Dina", COUNSELLOR).await;
    let (dalia_user, dalia) = w.staff("Dalia", COUNSELLOR).await;
    let batch = w.open_batch(&coordinator, "A", day(7)).await;
    let sara = tenant(&w.app, "Sara Ali").await;

    let e1 = enrolments::create(
        w.state(),
        &dina,
        NewEnrolment::course(sara.0, batch.id, 252_000).terms_accepted(),
    )
    .await
    .unwrap();
    assert!(
        e1.payment_link_path.is_some(),
        "the first payment has a link"
    );
    let payment = start_payment(&w, &e1).await;
    assert_eq!(payment.status, PaymentStatus::Pending);
    let txn = payment.gateway_transaction.clone().expect("a transaction");
    let paid = payment.amount;
    assert_eq!(paid, 252_000);

    // 48 hours and 30 minutes later the gateway gives up: the enrolment lapses.
    w.app.travel(Duration::from_secs(48 * 3600 + 30 * 60));
    w.app
        .at_travelled_time(online::gateway_failed(
            w.state(),
            &txn,
            PaymentFailureKind::Timeout,
            "",
        ))
        .await
        .unwrap()
        .expect("the payment failed");
    w.app.travel_back();
    assert_eq!(reload(&w, &e1).await.status, EnrolmentStatus::Lapsed);
    assert_eq!(
        edtech::seats::available(w.db(), batch.id).await.unwrap(),
        12
    );

    // Another counsellor enrols the student again before the money arrives.
    let e2 = enrolments::create(
        w.state(),
        &dalia,
        NewEnrolment::course(sara.0, batch.id, 252_000).terms_accepted(),
    )
    .await
    .expect("a lapsed enrolment never reopens: this is a new one");
    assert_ne!(e1.id, e2.id);
    assert_eq!(
        edtech::seats::available(w.db(), batch.id).await.unwrap(),
        11
    );
    assert_ne!(
        e2.payment_link_path,
        reload(&w, &e1).await.payment_link_path,
        "E2 is paid through its own link"
    );

    // The gateway confirms E1's payment late.
    let late = online::gateway_succeeded(w.state(), &txn, paid)
        .await
        .unwrap()
        .expect("confirmed late");
    assert_eq!(late.status, PaymentStatus::Received);

    let refunds = Refund::query()
        .where_(Refund::SOURCE_KIND.eq("payment"))
        .where_(Refund::SOURCE_ID.eq(payment.id))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(refunds.len(), 1, "one refund of the late payment");
    assert_eq!(refunds[0].amount, 252_000);
    assert_eq!(refunds[0].reason, wl_payments::late::REASON);
    assert_eq!(refunds[0].requested_by, None, "no person asked");
    assert_eq!(refunds[0].approved_by, None, "and none approved");
    assert_eq!(
        w.gateway.refunded(),
        [(txn, 252_000)],
        "back to the same card"
    );
    let second = reload(&w, &e2).await;
    assert_eq!(second.status, EnrolmentStatus::PendingPayment);
    assert!(second.payment_link_path.is_some());
    assert_eq!(reload(&w, &e1).await.status, EnrolmentStatus::Lapsed);
    for user in [&dina_user, &dalia_user] {
        let told = support::messages_to(&w.app, edtech::notices::NOTICE, user).await;
        assert!(
            told.iter().any(|m| m
                .rendered_body
                .contains("arrived after the seat hold ended and was refunded")
                && m.rendered_body.contains("AED 2,520.00")
                && m.rendered_body.contains(&e1.number())),
            "{} is told of the refund",
            user.name
        );
    }
    let _ = PartyId(sara.0);
}

// ---- #442: the centre permit and the course approval, the real guard ----------------------------

/// The message of a refusal.
fn refusal(error: &Error) -> String {
    match error {
        Error::Status(status, message) => {
            assert_eq!(*status, StatusCode::UNPROCESSABLE_ENTITY, "{message}");
            message.clone()
        }
        other => panic!("not a refusal: {other:?}"),
    }
}

/// The course ILT-A that the regulator must approve.
async fn approved_course(w: &World, coordinator: &AuthUser) -> Course {
    let mut new = NewCourse::new("ILT-A", "IELTS approved", "English language", 40).level("B2");
    new.approval_required = true;
    new.approval_no = Some("KHDA-2026-77".to_owned());
    courses::create(w.state(), coordinator, new).await.unwrap()
}

/// A planned batch of `course` from the next Monday after `day(7)`, with a
/// tutor and a room of their own.
async fn planned_of(
    w: &World,
    coordinator: &AuthUser,
    course: &Course,
    name: &str,
    tutor: (i64, Option<i64>),
) -> Batch {
    let mut room =
        wl_scheduling::Resource::new(&format!("Room {name}"), wl_scheduling::ResourceType::Room);
    room.capacity = 14;
    room.save(w.db()).await.unwrap();
    let first = monday_from(day(7));
    batches::create(
        w.state(),
        coordinator,
        NewBatch::new(
            course.id,
            12,
            (first, first + TimeDelta::days(27)),
            &[1, 3],
            "18:00",
            90,
        )
        .tutor(tutor.0, tutor.1)
        .room(room.id, Some(14)),
    )
    .await
    .expect("the batch is planned")
}

/// A person resource for a tutor.
async fn tutor_resource(w: &World, name: &str, employee: Option<i64>) -> wl_scheduling::Resource {
    let mut resource = wl_scheduling::Resource::new(name, wl_scheduling::ResourceType::Person);
    resource.employee_id = employee;
    resource.save(w.db()).await.unwrap();
    resource
}

/// #442 points 1 and 2 through the real wl-documents: the approval ends before
/// the last session, so `batches::open` is refused naming the approval and the
/// day, with no override offered, by the service and by the page; with an
/// approval that covers the last session (the very last day is enough) the same
/// batch opens.
#[renox::test]
async fn edu_07_open_follows_the_course_approval_in_the_real_guard() {
    let w = boot().await;
    let (coordinator_user, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    w.permit_until(day(400)).await;
    let course = approved_course(&w, &coordinator).await;
    let tutor = tutor_resource(&w, "Layla", None).await;
    let batch = planned_of(&w, &coordinator, &course, "A", (tutor.id, None)).await;
    let last = batches::plan_for(w.state(), &batch)
        .unwrap()
        .last()
        .expect("sessions")
        .date;
    let approval = record_document(&w.app, "course_approval", course.subject(), day(20)).await;

    let error = batches::open(w.state(), &coordinator, batch.id)
        .await
        .unwrap_err();
    let message = refusal(&error);
    assert!(message.contains("Course approval"), "{message}");
    assert!(message.contains(&day(20).to_string()), "{message}");
    assert!(!message.contains("Centre permit"), "{message}");
    assert_eq!(
        Batch::find_or_404(w.db(), batch.id).await.unwrap().status,
        edtech::BatchStatus::Planned
    );
    w.app.acting_as(&coordinator_user);
    let page = w
        .app
        .htmx()
        .post(&format!("/edtech/batches/{}/open", batch.id), &[("x", "y")])
        .await;
    page.assert_status(422);
    assert!(page.text().contains("Course approval"));

    // The approval runs to the very last session: the batch opens.
    wl_documents::Document::query()
        .where_(wl_documents::Document::ID.eq(approval.id))
        .update(w.db(), &[("expiry_date", &Some(last))])
        .await
        .unwrap();
    let opened = batches::open(w.state(), &coordinator, batch.id)
        .await
        .unwrap();
    assert_eq!(opened.status, edtech::BatchStatus::Open);
}

/// #442 point 3 through the real guard: the centre permit expired, so
/// `certificates::issue` is refused naming the permit although the certificate
/// states no approval; no number is spent; after the renewal it is issued.
#[renox::test]
async fn edu_07_issue_follows_the_centre_permit_in_the_real_guard() {
    let w = boot().await;
    let (_, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (_, dina) = w.staff("Dina", COUNSELLOR).await;
    let permit = w.permit_until(day(400)).await;
    let course = w.course(&coordinator).await;
    let tutor = tutor_resource(&w, "Layla", None).await;
    let batch = planned_of(&w, &coordinator, &course, "A", (tutor.id, None)).await;
    batches::open(w.state(), &coordinator, batch.id)
        .await
        .unwrap();
    let sara = tenant(&w.app, "Sara Ali").await;
    let e = enrolments::create(
        w.state(),
        &dina,
        NewEnrolment::course(sara.0, batch.id, 252_000).terms_accepted(),
    )
    .await
    .unwrap();
    enrolments::activate(w.state(), Some(&dina), e.id)
        .await
        .unwrap();
    enrolments::complete(w.state(), e.id).await.unwrap();
    let draft = edtech::Certificate::of_enrolment(w.db(), e.id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("a draft certificate");
    assert!(!draft.states_approval);

    wl_documents::Document::query()
        .where_(wl_documents::Document::ID.eq(permit.id))
        .update(w.db(), &[("expiry_date", &Some(day(-5)))])
        .await
        .unwrap();
    let error = edtech::certificates::issue(w.state(), &coordinator, draft.id)
        .await
        .unwrap_err();
    let message = refusal(&error);
    assert!(message.contains("Centre permit"), "{message}");
    assert!(message.contains(&day(-5).to_string()), "{message}");
    let still = edtech::Certificate::find_or_404(w.db(), draft.id)
        .await
        .unwrap();
    assert_eq!(still.status, edtech::CertificateStatus::Draft);
    assert!(still.number.is_none(), "no number is spent");

    wl_documents::Document::query()
        .where_(wl_documents::Document::ID.eq(permit.id))
        .update(w.db(), &[("expiry_date", &Some(day(400)))])
        .await
        .unwrap();
    let issued = edtech::certificates::issue(w.state(), &coordinator, draft.id)
        .await
        .unwrap();
    assert_eq!(issued.status, edtech::CertificateStatus::Issued);
    assert_eq!(issued.number.as_deref(), Some("CRT-0001"));
}

// ---- #443: the tutor's documents, the real guard ------------------------------------------------

/// A tutor who is a wl-hr employee, active, and the person resource behind them.
async fn employed(w: &World, name: &str) -> (wl_scheduling::Resource, i64) {
    let party = tenant(&w.app, name).await;
    let hired = wl_hr::employee::create(
        w.state(),
        wl_hr::employee::NewEmployee::new(party, "tutor", day(-400))
            .status(wl_hr::employee::EmployeeStatus::Active),
    )
    .await
    .expect("the tutor is hired");
    (tutor_resource(w, name, Some(hired.id)).await, hired.id)
}

/// #443 points 1, 2 and 4 with real documents: a police clearance that ends on
/// the Tuesday of the second week. `open` of a batch for minors is refused
/// naming the clearance and the first failing date (the Wednesday after), by
/// the service and the page; the same tutor opens an adult batch; a renewal
/// that covers the last date lets the minors' batch open; then `reschedule` to a
/// date beyond a later-ending clearance is refused and to an earlier date moves.
#[renox::test]
async fn edu_08_the_clearance_decides_open_and_reschedule_in_the_real_guard() {
    let w = boot().await;
    let (coordinator_user, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    w.permit_until(day(400)).await;
    let (layla, employee) = employed(&w, "Layla Hassan").await;
    let monday = monday_from(day(7));
    let clearance = record_document(
        &w.app,
        "police_clearance",
        wl_kernel::workforce::EmployeeId(employee).subject(),
        monday + TimeDelta::days(8),
    )
    .await;
    let kids = courses::create(
        w.state(),
        &coordinator,
        NewCourse::new("KIDS-1", "Kids mathematics", "Mathematics", 30).for_minors(),
    )
    .await
    .unwrap();
    let minors = planned_of(&w, &coordinator, &kids, "K", (layla.id, Some(employee))).await;

    let error = batches::open(w.state(), &coordinator, minors.id)
        .await
        .unwrap_err();
    let message = refusal(&error);
    assert!(
        message.to_lowercase().contains("police clearance"),
        "{message}"
    );
    let first_failing = monday + TimeDelta::days(9);
    assert!(message.contains(&first_failing.to_string()), "{message}");
    assert!(
        message.contains(&(monday + TimeDelta::days(23)).to_string()),
        "every failing date is listed: {message}"
    );
    assert!(message.contains("Layla Hassan"), "{message}");
    assert!(!message.to_lowercase().contains("minor"), "{message}");
    assert_eq!(
        Batch::find_or_404(w.db(), minors.id).await.unwrap().status,
        edtech::BatchStatus::Planned
    );
    w.app.acting_as(&coordinator_user);
    let page = w
        .app
        .htmx()
        .post(
            &format!("/edtech/batches/{}/open", minors.id),
            &[("x", "y")],
        )
        .await;
    page.assert_status(422);
    assert!(page.text().to_lowercase().contains("police clearance"));

    // A tutor with no clearance on record teaches an adult course: none is asked.
    let (omar, omar_employee) = employed(&w, "Omar Nasser").await;
    let adult_course = w.course(&coordinator).await;
    let adult = planned_of(
        &w,
        &coordinator,
        &adult_course,
        "B",
        (omar.id, Some(omar_employee)),
    )
    .await;
    assert_eq!(
        batches::open(w.state(), &coordinator, adult.id)
            .await
            .unwrap()
            .status,
        edtech::BatchStatus::Open
    );

    // The renewal covers the last session of the minors' batch.
    wl_documents::Document::query()
        .where_(wl_documents::Document::ID.eq(clearance.id))
        .update(
            w.db(),
            &[("expiry_date", &Some(monday + TimeDelta::days(27)))],
        )
        .await
        .unwrap();
    let opened = batches::open(w.state(), &coordinator, minors.id)
        .await
        .unwrap();
    assert_eq!(opened.status, edtech::BatchStatus::Open);

    // The clearance now ends on the Tuesday of the second week: a session moves
    // to a day it covers and not beyond it.
    wl_documents::Document::query()
        .where_(wl_documents::Document::ID.eq(clearance.id))
        .update(
            w.db(),
            &[("expiry_date", &Some(monday + TimeDelta::days(8)))],
        )
        .await
        .unwrap();
    let session = edtech::Session::of_batch(w.db(), minors.id)
        .await
        .unwrap()
        .remove(0);
    let late = monday + TimeDelta::days(11);
    let error = edtech::sessions::reschedule(w.state(), &coordinator, session.id, late, "18:00")
        .await
        .unwrap_err();
    let message = refusal(&error);
    assert!(
        message.to_lowercase().contains("police clearance"),
        "{message}"
    );
    assert!(message.contains(&late.to_string()), "{message}");
    let early = monday + TimeDelta::days(4);
    let moved = edtech::sessions::reschedule(w.state(), &coordinator, session.id, early, "18:00")
        .await
        .unwrap();
    assert_eq!(moved.session_date, early);
}

// ---- #441: a chargeback through the real wl-payments and wl-invoicing --------------------------

/// The first invoice of `e` as stored by the real wl-invoicing.
async fn invoice_of(w: &World, e: &Enrolment) -> wl_invoicing::Invoice {
    wl_invoicing::Invoice::find_or_404(w.db(), e.first_invoice_id.expect("an invoice"))
        .await
        .unwrap()
}

/// The accountant allocates the card payment `payment` to the first invoice
/// of `e` (wl-payments does not allocate a payment on a link by itself, see
/// the deferred line "#441 payment link allocation").
async fn allocate_to_invoice(w: &World, e: &Enrolment, payment: &Payment) {
    wl_payments::allocate::allocate(
        w.state(),
        wl_kernel::payments::PaymentId(payment.id),
        wl_kernel::payments::AllocationTarget::Invoice(wl_kernel::invoicing::InvoiceId(
            e.first_invoice_id.expect("an invoice"),
        )),
        payment.amount,
        None,
    )
    .await
    .expect("the payment is allocated");
}

/// #441 points 1 and 2 with the real modules. The AED 2,520.00 fee is paid by
/// card (wl-payments allocates it, wl-invoicing marks it paid, the enrolment
/// activates); the bank charges the payment back (`gateway_dispute_opened`):
/// wl-payments reverses it, wl-invoicing reopens the invoice, and the counsellor
/// gets one task with the amount and both links while the enrolment stays
/// `active`. The counsellor sends a payment link for the money; when it is paid
/// the invoice is paid again and the task closes.
#[renox::test]
async fn edu_06_a_charged_back_card_payment_reopens_the_invoice_and_gives_one_task() {
    use wl_kernel::invoicing::InvoiceStatus;
    use wl_kernel::tickets::task_creator;
    let w = boot().await;
    let (_, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (dina_user, dina) = w.staff("Dina", COUNSELLOR).await;
    let batch = w.open_batch(&coordinator, "A", day(7)).await;
    let sara = tenant(&w.app, "Sara Ali").await;
    let e = enrolments::create(
        w.state(),
        &dina,
        NewEnrolment::course(sara.0, batch.id, 252_000).terms_accepted(),
    )
    .await
    .unwrap();
    let payment = start_payment(&w, &e).await;
    let txn = payment.gateway_transaction.clone().expect("a transaction");
    online::gateway_succeeded(w.state(), &txn, payment.amount)
        .await
        .unwrap()
        .expect("received");
    allocate_to_invoice(&w, &e, &payment).await;
    assert_eq!(
        invoice_of(&w, &e).await.status,
        InvoiceStatus::Paid,
        "the payment settled INV"
    );
    assert_eq!(reload(&w, &e).await.status, EnrolmentStatus::Active);

    wl_payments::chargeback::gateway_dispute_opened(
        w.state(),
        &txn,
        &wl_payments::chargeback::DisputeOpened::new("dp_1"),
    )
    .await
    .unwrap()
    .expect("charged back");

    let reopened = invoice_of(&w, &e).await;
    assert_ne!(reopened.status, InvoiceStatus::Paid, "the invoice reopened");
    assert_eq!(reload(&w, &e).await.status, EnrolmentStatus::Active);
    let key = edtech::chargebacks::task_key(e.id);
    let found = task_creator(w.state())
        .find_open(w.state(), &key)
        .await
        .unwrap()
        .expect("the counsellor's task");
    let ticket = wl_ticketing::Ticket::find_or_404(w.db(), found.id.0)
        .await
        .unwrap();
    assert_eq!(ticket.assignee_user_id, Some(dina_user.id));
    let text = format!("{} {}", ticket.title, ticket.description);
    assert!(text.contains("2,520.00"), "{text}");
    assert!(
        text.contains(&edtech::chargebacks::collect_path(e.id)),
        "{text}"
    );
    assert!(
        text.contains(&format!("/edtech/enrolments/{}", e.id)),
        "{text}"
    );
    assert_eq!(
        wl_ticketing::Ticket::query()
            .where_(wl_ticketing::Ticket::SOURCE_KEY.eq(key.clone()))
            .count(w.db())
            .await
            .unwrap(),
        1,
        "one task"
    );

    // The counsellor sends a link for the money; it is paid; the task closes.
    let path = edtech::chargebacks::send_link(w.state(), &dina, e.id)
        .await
        .unwrap();
    let after_link = reload(&w, &e).await;
    assert_eq!(after_link.payment_link_path.as_deref(), Some(path.as_str()));
    let again = start_payment(&w, &after_link).await;
    assert_eq!(again.amount, 252_000);
    online::gateway_succeeded(
        w.state(),
        again.gateway_transaction.as_deref().unwrap(),
        again.amount,
    )
    .await
    .unwrap()
    .expect("received");
    allocate_to_invoice(&w, &e, &again).await;
    assert_eq!(invoice_of(&w, &e).await.status, InvoiceStatus::Paid);
    assert!(
        task_creator(w.state())
            .find_open(w.state(), &key)
            .await
            .unwrap()
            .is_none(),
        "the task closed"
    );
    assert_eq!(reload(&w, &e).await.status, EnrolmentStatus::Active);
}

// ---- #440: the membership and the enrolment, with the real wl-invoicing ------------------------

/// An active membership enrolment of a new student (AED 315.00 a month) whose
/// real subscription was started on activation.
async fn membership(
    w: &World,
    desk: &AuthUser,
    name: &str,
) -> (Enrolment, wl_invoicing::Subscription) {
    let student = tenant(&w.app, name).await;
    let e = enrolments::create(
        w.state(),
        desk,
        NewEnrolment::membership(student.0, "club", "Conversation club", 31_500).terms_accepted(),
    )
    .await
    .unwrap();
    let active = enrolments::activate(w.state(), Some(desk), e.id)
        .await
        .unwrap();
    assert_eq!(active.status, EnrolmentStatus::Active);
    let id = active.subscription_id.expect("a subscription was started");
    let subscription = wl_invoicing::Subscription::find_or_404(w.db(), id)
        .await
        .unwrap();
    assert_eq!(
        subscription.status,
        wl_kernel::subscriptions::SubscriptionStatus::Active
    );
    (active, subscription)
}

async fn subscription_now(w: &World, s: &wl_invoicing::Subscription) -> wl_invoicing::Subscription {
    wl_invoicing::Subscription::find_or_404(w.db(), s.id)
        .await
        .unwrap()
}

/// #440 points 1 and 4 with the real subscription: the guardian pauses the
/// membership for 14 days (a user pause of wl-invoicing), the enrolment is
/// frozen with the *linked* cause and no day counted against the allowance; the
/// enrolment history shows each step once; when the pause is ended the
/// enrolment is active and nothing is asked of the subscription a second time.
#[renox::test]
async fn edu_05_a_real_pause_freezes_the_enrolment_and_its_end_unfreezes_it_once() {
    use wl_kernel::subscriptions::{PauseCause, PauseSubscription, SubscriptionStatus};
    let w = boot().await;
    let (_, desk) = w.staff("Dina", COUNSELLOR).await;
    let (_, accountant) = w.staff("Rita", host::roles::ACCOUNTANT).await;
    let (e, subscription) = membership(&w, &desk, "Sara Ali").await;

    wl_invoicing::subscriptions::pause(
        w.state(),
        &subscription,
        &PauseSubscription::new(PauseCause::User, day(0))
            .reason("travelling")
            .until(day(14)),
        Some(&accountant),
    )
    .await
    .expect("the user pause runs");

    let paused = subscription_now(&w, &subscription).await;
    assert_eq!(paused.status, SubscriptionStatus::Paused);
    assert_eq!(
        paused.pause_days_used, 14,
        "the module counts the allowance"
    );
    let frozen = reload(&w, &e).await;
    assert_eq!(frozen.status, EnrolmentStatus::Frozen);
    let rows = edtech::EnrolmentFreeze::open_of(w.db(), e.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].cause, edtech::FreezeCause::Linked);
    assert_eq!(rows[0].counted_days, 0, "the days are counted once, by M");
    assert_eq!(rows[0].until_on, Some(day(14)));
    let open = wl_invoicing::SubscriptionPause::open_of(w.db(), subscription.id)
        .await
        .unwrap();
    assert_eq!(open.len(), 1, "no second pause was asked of M");

    wl_invoicing::subscriptions::resume(
        w.state(),
        &subscription,
        PauseCause::User,
        day(0),
        Some(&accountant),
    )
    .await
    .expect("the pause ends");

    assert_eq!(
        subscription_now(&w, &subscription).await.status,
        SubscriptionStatus::Active
    );
    assert_eq!(reload(&w, &e).await.status, EnrolmentStatus::Active);
    assert!(
        edtech::EnrolmentFreeze::open_of(w.db(), e.id)
            .await
            .unwrap()
            .is_empty()
    );
    let steps: Vec<String> = renox::state_machine::history(w.db(), "edtech_enrolments", e.id)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.transition)
        .collect();
    assert_eq!(
        steps.iter().filter(|s| s.starts_with("freeze")).count(),
        1,
        "{steps:?}"
    );
    assert_eq!(
        steps.iter().filter(|s| s.starts_with("unfreeze")).count(),
        1,
        "{steps:?}"
    );
}

/// #440 point 2 with the real subscription: staff freeze the enrolment for 14
/// days; the real subscription is paused with the agreement's cause (one pause,
/// asked once) and counts none of the days against the member's allowance, the
/// enrolment counted its 15 days; unfreezing resumes the subscription.
#[renox::test]
async fn edu_05_a_real_freeze_pauses_the_subscription_once_and_unfreezing_resumes_it() {
    use wl_kernel::subscriptions::{PauseCause, SubscriptionStatus};
    let w = boot().await;
    let (_, desk) = w.staff("Dina", COUNSELLOR).await;
    let (e, subscription) = membership(&w, &desk, "Omar Hassan").await;

    enrolments::freeze(
        w.state(),
        Some(&desk),
        e.id,
        edtech::FreezeCause::Student,
        Some(day(14)),
    )
    .await
    .unwrap();

    let paused = subscription_now(&w, &subscription).await;
    assert_eq!(paused.status, SubscriptionStatus::Paused);
    assert_eq!(
        paused.pause_days_used, 0,
        "the days are counted by the enrolment"
    );
    let open = wl_invoicing::SubscriptionPause::open_of(w.db(), subscription.id)
        .await
        .unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].cause, PauseCause::Agreement);
    assert_eq!(
        edtech::EnrolmentFreeze::counted_in_year(w.db(), e.student_party_id, day(0))
            .await
            .unwrap(),
        15
    );

    enrolments::unfreeze(w.state(), Some(&desk), e.id, edtech::FreezeCause::Student)
        .await
        .unwrap();

    assert_eq!(reload(&w, &e).await.status, EnrolmentStatus::Active);
    assert_eq!(
        subscription_now(&w, &subscription).await.status,
        SubscriptionStatus::Active
    );
}

// ---- #438: the withdrawal of a membership, with the real wl-invoicing ---------------------------

/// #438 point 4 with the real subscription: the first cycle of the membership
/// is billed and paid; the student withdraws; the subscription is cancelled at
/// once by wl-invoicing, which makes its own pro rata credit note for the unused
/// part of the cycle; the vertical makes no second credit note, asks for no
/// refund and the enrolment is `withdrawn`.
#[renox::test]
async fn edu_03_a_withdrawn_membership_is_cancelled_with_its_own_pro_rata_credit() {
    use wl_kernel::subscriptions::{SubscriptionStatus, subscriptions};
    let w = boot().await;
    let (_, desk) = w.staff("Dina", COUNSELLOR).await;
    let (e, subscription) = membership(&w, &desk, "Sara Ali").await;
    assert_eq!(
        wl_invoicing::subscriptions::bill_due(w.state(), day(0))
            .await
            .unwrap(),
        1
    );
    let cycle = wl_invoicing::subscriptions::cycle_invoices(w.db(), subscription.id)
        .await
        .unwrap()
        .remove(0)
        .1;
    wl_kernel::invoicing::invoicer(w.state())
        .allocate(
            w.state(),
            wl_kernel::invoicing::InvoiceId(cycle.id),
            wl_kernel::Subject::new("payments.payment", 1),
            cycle.gross_total,
        )
        .await
        .expect("the cycle is paid");

    let asked = enrolments::request_withdrawal(w.state(), Some(&desk), e.id, "moving")
        .await
        .unwrap();

    assert_eq!(asked.enrolment.status, EnrolmentStatus::Withdrawn);
    assert!(asked.approval_request.is_none());
    let summaries = subscriptions(w.state())
        .of_agreement(w.db(), &e.subject())
        .await
        .unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].status, SubscriptionStatus::Cancelled);
    assert_eq!(summaries[0].cancelled_on, Some(day(0)));
    let note = summaries[0]
        .final_credit_note
        .expect("wl-invoicing credited the unused part");
    let notes = wl_invoicing::CreditNote::query().get(w.db()).await.unwrap();
    assert_eq!(
        notes.len(),
        1,
        "the subscription's credit note is the only one"
    );
    assert_eq!(notes[0].id, note);
    assert!(notes[0].gross_total > 0 && notes[0].gross_total <= cycle.gross_total);
    assert!(
        Refund::query().count(w.db()).await.unwrap() == 0,
        "no refund was asked by the vertical"
    );
}

/// #438 points 1 and 6 with the real modules. The AED 2,520.00 fee is paid on the
/// card and allocated; the student withdraws at once and the refund waits for
/// the centre manager (`FIN.ACL.04`); wl-invoicing makes the one credit note and
/// asks wl-payments for the refund out of it; the accountant approves, pays and
/// then marks the transfer failed (the bank returned it): wl-invoicing gives the
/// credit note its balance back, the enrolment stays `withdrawn` and no second
/// credit note is made.
#[renox::test]
async fn edu_03_a_failed_refund_reopens_the_credit_note_balance() {
    use wl_kernel::payments::{PaymentMethod, RefundFailureKind};
    let w = boot().await;
    let (_, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (_, dina) = w.staff("Dina", COUNSELLOR).await;
    let (manager_user, _) = w.staff("Mona", host::roles::CENTRE_MANAGER).await;
    let (_, rita) = w.staff("Rita", host::roles::ACCOUNTANT).await;
    let batch = w.open_batch(&coordinator, "A", day(7)).await;
    let sara = tenant(&w.app, "Sara Ali").await;
    let e = enrolments::create(
        w.state(),
        &dina,
        NewEnrolment::course(sara.0, batch.id, 252_000).terms_accepted(),
    )
    .await
    .unwrap();
    let payment = start_payment(&w, &e).await;
    online::gateway_succeeded(
        w.state(),
        payment.gateway_transaction.as_deref().unwrap(),
        payment.amount,
    )
    .await
    .unwrap()
    .expect("received");
    allocate_to_invoice(&w, &e, &payment).await;
    assert_eq!(reload(&w, &e).await.status, EnrolmentStatus::Active);

    let asked = enrolments::request_withdrawal(w.state(), Some(&dina), e.id, "moving")
        .await
        .unwrap();
    let request = asked
        .approval_request
        .expect("the refund waits for approval");
    assert_eq!(asked.enrolment.status, EnrolmentStatus::Active);
    wl_kernel::approvals::approve(w.state(), request, &manager_user, "")
        .await
        .unwrap();

    let gone = reload(&w, &e).await;
    assert_eq!(gone.status, EnrolmentStatus::Withdrawn);
    let notes = wl_invoicing::CreditNote::query().get(w.db()).await.unwrap();
    assert_eq!(notes.len(), 1, "one credit note");
    let note = &notes[0];
    assert!(note.gross_total > 0);
    let refunds = Refund::query().get(w.db()).await.unwrap();
    assert_eq!(refunds.len(), 1);
    let refund = &refunds[0];
    assert_eq!(refund.amount, note.gross_total);
    assert_eq!(
        note.remaining, 0,
        "the refund holds the balance of the note"
    );

    let approved = wl_payments::refund::approve(w.state(), &rita, refund)
        .await
        .expect("a person other than the recorder approves");
    let paid = wl_payments::refund::pay(
        w.state(),
        &rita,
        &approved,
        PaymentMethod::Transfer,
        "BANK-9",
    )
    .await
    .expect("the transfer is paid");
    let after_pay = wl_invoicing::CreditNote::find_or_404(w.db(), note.id)
        .await
        .unwrap();
    assert_eq!(after_pay.remaining, 0, "and still does once paid");

    wl_payments::refund::fail(
        w.state(),
        &rita,
        &paid,
        RefundFailureKind::BankReturn,
        "returned by the bank",
    )
    .await
    .expect("the refund is marked failed");

    let reopened = wl_invoicing::CreditNote::find_or_404(w.db(), note.id)
        .await
        .unwrap();
    assert_eq!(
        reopened.remaining, note.gross_total,
        "the balance of the credit note is open again"
    );
    assert_eq!(reload(&w, &e).await.status, EnrolmentStatus::Withdrawn);
    assert_eq!(
        wl_invoicing::CreditNote::query()
            .count(w.db())
            .await
            .unwrap(),
        1,
        "no second credit note"
    );
}

// ---- #446: the attendance of a session whose tutor lacks the documents, the real wl-documents ---

/// The first session of `batch`, moved to `date` (a day already passed) with its
/// start and end, as a fixture of a class that has been given.
async fn session_moved_to(w: &World, batch: &Batch, date: NaiveDate) -> edtech::Session {
    let mut session = edtech::Session::of_batch(w.db(), batch.id)
        .await
        .unwrap()
        .remove(0);
    let shift = TimeDelta::days((session.session_date - date).num_days());
    session.session_date = date;
    session.starts_at -= shift;
    session.ends_at -= shift;
    session.save(w.db()).await.unwrap();
    session
}

/// A batch for minors taught by the tutor `name` (an employee), opened while
/// the police clearance ran to `day(400)`; the clearance is the document answered.
async fn minors_batch_of(
    w: &World,
    coordinator: &AuthUser,
    name: &str,
) -> (Batch, wl_documents::Document) {
    w.permit_until(day(400)).await;
    let (tutor, employee) = employed(w, name).await;
    let clearance = record_document(
        &w.app,
        "police_clearance",
        wl_kernel::workforce::EmployeeId(employee).subject(),
        day(400),
    )
    .await;
    // A class of the past is judged by the documents accepted by its day.
    wl_documents::Document::query()
        .where_(wl_documents::Document::ID.eq(clearance.id))
        .update(
            w.db(),
            &[(
                "accepted_at",
                &Some(renox::db::now() - TimeDelta::days(400)),
            )],
        )
        .await
        .unwrap();
    let kids = match Course::by_code(w.db(), "KIDS-1").await.unwrap() {
        Some(course) => course,
        None => courses::create(
            w.state(),
            coordinator,
            NewCourse::new("KIDS-1", "Kids mathematics", "Mathematics", 30).for_minors(),
        )
        .await
        .unwrap(),
    };
    let planned = planned_of(w, coordinator, &kids, name, (tutor.id, Some(employee))).await;
    let batch = batches::open(w.state(), coordinator, planned.id)
        .await
        .expect("the batch opens while the clearance is valid");
    (batch, clearance)
}

/// Sets the end of `document`.
async fn expires(document: &wl_documents::Document, on: NaiveDate, w: &World) {
    wl_documents::Document::query()
        .where_(wl_documents::Document::ID.eq(document.id))
        .update(w.db(), &[("expiry_date", &Some(on))])
        .await
        .unwrap();
}

/// #446 points 1 and 2 with the real wl-documents: the tutor's police clearance
/// ended five days ago; the tutor submits the attendance of the class of three
/// days ago: it is accepted (`held`), the guard records ONE flag naming the
/// clearance and its date, and the centre manager closes it with a note from the
/// session page (the coordinator and a missing note are refused); the history
/// keeps the flag, the note and who closed it.
#[renox::test]
async fn edu_11_an_expired_clearance_flags_the_attendance_and_the_manager_closes_it() {
    let w = boot().await;
    let (_, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (batch, clearance) = minors_batch_of(&w, &coordinator, "Layla Hassan").await;
    let session = session_moved_to(&w, &batch, day(-3)).await;
    expires(&clearance, day(-5), &w).await;
    let (_, tariq) = w.staff("Tariq", host::roles::TUTOR).await;

    let held = edtech::sessions::submit_attendance(w.state(), &tariq, session.id)
        .await
        .expect("the attendance is always accepted");
    assert_eq!(held.status, edtech::SessionStatus::Held);
    let flags = edtech::sessions::document_flags(w.state(), session.id)
        .await
        .unwrap();
    assert_eq!(flags.len(), 1, "one flag");
    assert!(
        flags[0].label.contains("Police clearance expired")
            && flags[0].label.contains(&day(-5).to_string()),
        "{}",
        flags[0].label
    );
    assert!(!flags[0].is_reviewed());
    assert_eq!(flags[0].on_date, day(-3));

    let (manager, _) = w.staff("Mona", host::roles::CENTRE_MANAGER).await;
    w.app.acting_as(&manager);
    w.app
        .get(&format!("/edtech/sessions/{}", session.id))
        .await
        .assert_ok()
        .assert_see("Documents flagged")
        .assert_see("Police clearance expired")
        .assert_see("Close the flag");

    let url = format!("/edtech/sessions/{}/flag", session.id);
    let flag_id = flags[0].id.to_string();
    w.app
        .htmx()
        .post(&url, &[("flag", flag_id.as_str()), ("note", " ")])
        .await
        .assert_status(422);
    let (coordinator_user, _) = w.staff("Cora", ACADEMIC_COORDINATOR).await;
    w.app.acting_as(&coordinator_user);
    w.app
        .htmx()
        .post(&url, &[("flag", flag_id.as_str()), ("note", "Seen")])
        .await
        .assert_forbidden();
    w.app.acting_as(&manager);
    w.app
        .htmx()
        .post(
            &url,
            &[("flag", flag_id.as_str()), ("note", "Renewal filed; seen")],
        )
        .await
        .assert_ok();

    let closed = edtech::sessions::document_flags(w.state(), session.id)
        .await
        .unwrap();
    assert_eq!(closed.len(), 1, "the flag is kept");
    assert!(closed[0].is_reviewed());
    assert_eq!(
        closed[0].review_note.as_deref(),
        Some("Renewal filed; seen")
    );
    assert_eq!(closed[0].reviewed_by, Some(manager.id));
    w.app
        .get(&format!("/edtech/sessions/{}", session.id))
        .await
        .assert_ok()
        .assert_see("Renewal filed; seen")
        .assert_see("Closed");
    w.app
        .htmx()
        .post(&url, &[("flag", flag_id.as_str()), ("note", "Again")])
        .await
        .assert_status(422);
}

/// #446 point 3 with the real wl-documents: the clearance ended yesterday at
/// the earliest day of the class (valid through the class day): no flag.
#[renox::test]
async fn edu_11_a_valid_clearance_gives_no_flag_in_the_real_guard() {
    let w = boot().await;
    let (_, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (batch, clearance) = minors_batch_of(&w, &coordinator, "Layla Hassan").await;
    let session = session_moved_to(&w, &batch, day(-3)).await;
    expires(&clearance, day(-3), &w).await;
    let (_, tariq) = w.staff("Tariq", host::roles::TUTOR).await;

    let held = edtech::sessions::submit_attendance(w.state(), &tariq, session.id)
        .await
        .unwrap();
    assert_eq!(held.status, edtech::SessionStatus::Held);
    assert!(
        edtech::sessions::document_flags(w.state(), session.id)
            .await
            .unwrap()
            .is_empty(),
        "valid through the class day"
    );
}

// ---- #450, #449, #446: the credits of a package, against the real wl-sales ledger ----------------

const PACK: &str = "pack10";

/// The local time `hour:minute` of `date` (Asia/Dubai) as an instant.
fn local(date: NaiveDate, hour: u32, minute: u32) -> renox::db::DateTime {
    wl_kernel::BusinessCalendar::default()
        .from_local(date.and_hms_opt(hour, minute, 0).expect("a time"))
}

/// The `n`th session (from 0) of `batch`, set to start at `starts` and last
/// 90 minutes, as a class that has been (or will be) given then.
async fn session_at(
    w: &World,
    batch: &Batch,
    n: usize,
    starts: renox::db::DateTime,
) -> edtech::Session {
    let mut session = edtech::Session::of_batch(w.db(), batch.id)
        .await
        .unwrap()
        .remove(n);
    session.session_date = wl_kernel::documents::dubai_date(starts);
    session.starts_at = starts;
    session.ends_at = starts + TimeDelta::minutes(90);
    session.save(w.db()).await.unwrap();
    session
}

/// A student with an ACTIVE package of `credits` credits (AED 150.00 each plus
/// VAT is the price asked), sold by wl-sales and paid on its invoice; answers
/// the enrolment and the party.
async fn package_student(
    w: &World,
    desk: &AuthUser,
    name: &str,
    credits: i64,
    policy: Option<(bool, i64)>,
) -> (Enrolment, i64) {
    use wl_kernel::sales::package_ledger;
    let party = tenant(&w.app, name).await;
    let mut new = NewEnrolment::package(party.0, PACK, "Ten classes", credits, credits * 15_750)
        .terms_accepted();
    if let Some((consumes, hours)) = policy {
        new = new.absence_policy(consumes, hours);
    }
    let e = enrolments::create(w.state(), desk, new).await.unwrap();
    let invoice = invoice_of(w, &e).await;
    wl_kernel::invoicing::invoicer(w.state())
        .allocate(
            w.state(),
            wl_kernel::invoicing::InvoiceId(invoice.id),
            wl_kernel::Subject::new("payments.payment", 9_000 + e.id),
            invoice.gross_total,
        )
        .await
        .expect("the package is paid");
    let active = reload(w, &e).await;
    assert_eq!(active.status, EnrolmentStatus::Active, "paid, so active");
    assert_eq!(
        package_ledger(w.state())
            .balance(w.db(), PartyId(party.0), PACK)
            .await
            .unwrap(),
        credits,
        "wl-sales holds the credits"
    );
    (active, party.0)
}

/// The credits wl-sales says `party` can use.
async fn credits_left(w: &World, party: i64) -> i64 {
    wl_kernel::sales::package_ledger(w.state())
        .balance(w.db(), PartyId(party), PACK)
        .await
        .unwrap()
}

/// The open batch with a coordinator, a desk and a tutor, ready for classes.
struct Class {
    batch: Batch,
    coordinator: AuthUser,
    desk: AuthUser,
    tutor: AuthUser,
}

async fn class(w: &World) -> Class {
    let (_, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (_, desk) = w.staff("Dina", COUNSELLOR).await;
    let (_, tutor) = w.staff("Tariq", host::roles::TUTOR).await;
    let batch = w.open_batch(&coordinator, "A", day(7)).await;
    Class {
        batch,
        coordinator,
        desk,
        tutor,
    }
}

async fn mark_row(
    w: &World,
    session: &edtech::Session,
    e: &Enrolment,
) -> edtech::attendance::Attendance {
    edtech::attendance::Attendance::find_of(w.db(), session.id, e.id)
        .await
        .unwrap()
        .expect("a row")
}

/// #450 points 1 to 3 against the real ledger: a 10-credit package; absent with
/// no notice uses one credit (nine left in wl-sales, the row consumed); a
/// notice 26 hours ahead excuses (nothing used, one make-up right); a notice two
/// hours ahead leaves the absence unexcused and it uses a credit.
#[renox::test]
async fn edu_15_the_real_ledger_charges_the_unexcused_absence_only() {
    use edtech::attendance::Mark;
    let w = boot().await;
    let c = class(&w).await;
    let (e, party) = package_student(&w, &c.desk, "Sara Ali", 10, None).await;
    let first = session_at(&w, &c.batch, 0, local(day(-6), 18, 0)).await;
    let early = session_at(&w, &c.batch, 1, local(day(-4), 18, 0)).await;
    let late = session_at(&w, &c.batch, 2, local(day(-2), 18, 0)).await;
    for s in [&first, &early, &late] {
        edtech::roster::add_student(w.state(), &c.desk, s.id, e.id)
            .await
            .unwrap();
    }

    edtech::sessions::submit_attendance_marked(
        w.state(),
        &c.tutor,
        first.id,
        &[(e.id, Mark::Absent)],
        false,
    )
    .await
    .unwrap();
    let absent = mark_row(&w, &first, &e).await;
    assert!(absent.credit_due && absent.consumed);
    assert_eq!(credits_left(&w, party).await, 9, "one credit used");

    // 26 hours ahead excuses.
    let noticed = edtech::roster::record_notice(
        w.state(),
        &c.desk,
        early.id,
        e.id,
        early.starts_at - TimeDelta::hours(26),
    )
    .await
    .unwrap();
    assert_eq!(noticed.mark, Mark::Excused);
    edtech::sessions::submit_attendance_marked(w.state(), &c.tutor, early.id, &[], false)
        .await
        .unwrap();
    assert!(!mark_row(&w, &early, &e).await.consumed);
    assert_eq!(
        credits_left(&w, party).await,
        9,
        "an excused absence is free"
    );
    let rights = edtech::make_up::open_rights(w.db(), &reload(&w, &e).await)
        .await
        .unwrap();
    assert_eq!(rights.len(), 1);
    assert_eq!(rights[0].session_id, early.id);

    // Two hours ahead is too late: the absence is unexcused.
    let tardy = edtech::roster::record_notice(
        w.state(),
        &c.desk,
        late.id,
        e.id,
        late.starts_at - TimeDelta::hours(2),
    )
    .await
    .unwrap();
    assert_eq!(tardy.mark, Mark::Pending);
    edtech::sessions::submit_attendance_marked(
        w.state(),
        &c.tutor,
        late.id,
        &[(e.id, Mark::Absent)],
        false,
    )
    .await
    .unwrap();
    assert!(mark_row(&w, &late, &e).await.consumed);
    assert_eq!(credits_left(&w, party).await, 8);
}

/// #450 point 4 against the real ledger: an enrolment whose policy says an
/// unexcused absence consumes nothing keeps its credits when absent, and the
/// student who came still uses one.
#[renox::test]
async fn edu_15_the_real_ledger_keeps_the_credit_when_the_policy_does_not_charge_absences() {
    use edtech::attendance::Mark;
    let w = boot().await;
    let c = class(&w).await;
    let (e, party) = package_student(&w, &c.desk, "Sara Ali", 10, Some((false, 24))).await;
    let absent_day = session_at(&w, &c.batch, 0, local(day(-5), 18, 0)).await;
    let present_day = session_at(&w, &c.batch, 1, local(day(-3), 18, 0)).await;
    for s in [&absent_day, &present_day] {
        edtech::roster::add_student(w.state(), &c.desk, s.id, e.id)
            .await
            .unwrap();
    }
    edtech::sessions::submit_attendance_marked(
        w.state(),
        &c.tutor,
        absent_day.id,
        &[(e.id, Mark::Absent)],
        false,
    )
    .await
    .unwrap();
    assert!(!mark_row(&w, &absent_day, &e).await.consumed);
    assert_eq!(credits_left(&w, party).await, 10, "nothing consumed");

    edtech::sessions::submit_attendance_marked(
        w.state(),
        &c.tutor,
        present_day.id,
        &[(e.id, Mark::Present)],
        false,
    )
    .await
    .unwrap();
    assert_eq!(credits_left(&w, party).await, 9, "attending uses one");
}

/// #450 point 5 against the real ledger: a cancelled session consumes nothing
/// of a package student on its roster, and a membership student of the batch
/// consumes nothing of any ledger.
#[renox::test]
async fn edu_15_the_real_ledger_is_not_asked_for_a_cancelled_session() {
    let w = boot().await;
    let c = class(&w).await;
    let (e, party) = package_student(&w, &c.desk, "Sara Ali", 10, None).await;
    let cancelled = session_at(&w, &c.batch, 0, local(day(2), 18, 0)).await;
    edtech::roster::add_student(w.state(), &c.desk, cancelled.id, e.id)
        .await
        .unwrap();

    let done =
        edtech::sessions::cancel(w.state(), &c.coordinator, cancelled.id, "Tutor sick", None)
            .await
            .unwrap();
    assert_eq!(done.status, edtech::SessionStatus::Cancelled);
    assert_eq!(credits_left(&w, party).await, 10, "nothing consumed");
    let package = wl_sales::Package::query()
        .where_(wl_sales::Package::HOLDER_PARTY_ID.eq(party))
        .first(w.db())
        .await
        .unwrap()
        .expect("the package");
    assert_eq!(package.credits_used, 0);
}

/// #449 points 3 and 4 against the real ledger: the tutor marks absent without
/// notice (a credit used), then corrects to excused (the credit comes back in
/// wl-sales) and then to present (one is used again); each only the difference.
#[renox::test]
async fn edu_14_a_correction_moves_the_real_ledger_by_the_difference() {
    use edtech::attendance::Mark;
    let w = boot().await;
    let c = class(&w).await;
    let (e, party) = package_student(&w, &c.desk, "Sara Ali", 10, None).await;
    // Ended two hours ago: the tutor is inside the 24 hours.
    let ended = renox::db::now() - TimeDelta::hours(2);
    let session = session_at(&w, &c.batch, 0, ended - TimeDelta::minutes(90)).await;
    edtech::roster::add_student(w.state(), &c.desk, session.id, e.id)
        .await
        .unwrap();
    edtech::sessions::submit_attendance_marked(
        w.state(),
        &c.tutor,
        session.id,
        &[(e.id, Mark::Absent)],
        false,
    )
    .await
    .unwrap();
    assert_eq!(credits_left(&w, party).await, 9);

    edtech::corrections::correct_attendance(
        w.state(),
        &c.tutor,
        session.id,
        &[(e.id, Mark::Excused)],
        "the mother called before",
    )
    .await
    .unwrap();
    assert_eq!(credits_left(&w, party).await, 10, "the credit came back");
    assert!(!mark_row(&w, &session, &e).await.consumed);

    edtech::corrections::correct_attendance(
        w.state(),
        &c.tutor,
        session.id,
        &[(e.id, Mark::Present)],
        "she came after all",
    )
    .await
    .unwrap();
    assert_eq!(credits_left(&w, party).await, 9, "attending uses one");
    assert!(mark_row(&w, &session, &e).await.consumed);

    // Present to present changes nothing.
    edtech::corrections::correct_attendance(
        w.state(),
        &c.tutor,
        session.id,
        &[(e.id, Mark::Late)],
        "arrived late",
    )
    .await
    .unwrap();
    assert_eq!(credits_left(&w, party).await, 9);
}

/// #446 point 1, the credits: a flagged session still uses the credit of a
/// package student who came, in the real wl-sales ledger.
#[renox::test]
async fn edu_11_a_flagged_session_still_uses_the_real_credit() {
    use edtech::attendance::Mark;
    let w = boot().await;
    let (_, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (_, desk) = w.staff("Dina", COUNSELLOR).await;
    let (batch, clearance) = minors_batch_of(&w, &coordinator, "Layla Hassan").await;
    let (e, party) = package_student(&w, &desk, "Sara Ali", 10, None).await;
    let session = session_at(&w, &batch, 0, local(day(-3), 18, 0)).await;
    edtech::roster::add_student(w.state(), &desk, session.id, e.id)
        .await
        .unwrap();
    expires(&clearance, day(-5), &w).await;
    let (_, tariq) = w.staff("Tariq", host::roles::TUTOR).await;

    edtech::sessions::submit_attendance_marked(
        w.state(),
        &tariq,
        session.id,
        &[(e.id, Mark::Present)],
        false,
    )
    .await
    .unwrap();

    assert_eq!(
        edtech::sessions::document_flags(w.state(), session.id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        credits_left(&w, party).await,
        9,
        "the flag changes no credit"
    );
}

// ---- #447 and #448: a batch stops and the refunds, through the real wl-payments and wl-invoicing -

/// The student pays the first invoice of `e` on the card at its link and the
/// accountant allocates the payment: the enrolment is active.
async fn pay_by_card(w: &World, e: &Enrolment) {
    let payment = start_payment(w, e).await;
    online::gateway_succeeded(
        w.state(),
        payment
            .gateway_transaction
            .as_deref()
            .expect("a transaction"),
        payment.amount,
    )
    .await
    .unwrap()
    .expect("received");
    allocate_to_invoice(w, e, &payment).await;
    assert_eq!(reload(w, e).await.status, EnrolmentStatus::Active);
}

/// The refunds of wl-payments, oldest first.
async fn refunds(w: &World) -> Vec<Refund> {
    Refund::query()
        .order_by(Refund::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// A batch of 12 sessions (Monday and Wednesday for six weeks) that began five
/// weeks ago, open (nobody has taught yet).
async fn started_batch(w: &World, coordinator: &AuthUser, name: &str) -> Batch {
    w.permit_until(day(400)).await;
    let course = w.course(coordinator).await;
    let tutor = tutor_resource(w, &format!("Tutor {name}"), None).await;
    let mut room =
        wl_scheduling::Resource::new(&format!("Room {name}"), wl_scheduling::ResourceType::Room);
    room.capacity = 14;
    room.save(w.db()).await.unwrap();
    let first = monday_from(day(-35));
    let planned = batches::create(
        w.state(),
        coordinator,
        NewBatch::new(
            course.id,
            12,
            (first, first + TimeDelta::days(41)),
            &[1, 3],
            "18:00",
            90,
        )
        .tutor(tutor.id, None)
        .room(room.id, Some(14)),
    )
    .await
    .expect("the batch is planned");
    batches::open(w.state(), coordinator, planned.id)
        .await
        .expect("the batch opens")
}

/// #447 point 1 with the real modules: running batch B (12 sessions, 5 held)
/// with E1 (AED 2,520.00 paid on the card, an admin fee) and E2 (paid too). The
/// coordinator asks to terminate for "venue", E1 -> withdraw, E2 -> transfer to
/// B2; the centre manager approves. 7 sessions are cancelled, wl-invoicing
/// makes the one credit note of AED 1,470.00 (2,520.00 x 7/12, no admin fee)
/// for E1, wl-payments holds the refund of the same amount REQUESTED by the
/// coordinator and waiting for another person's approval; E2 is transferred
/// with a new active enrolment in B2.
#[renox::test]
async fn edu_12_the_approved_termination_asks_the_real_payments_for_the_refund() {
    let w = boot().await;
    let (coordinator_user, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (_, dina) = w.staff("Dina", COUNSELLOR).await;
    let (manager, _) = w.staff("Mona", host::roles::CENTRE_MANAGER).await;
    let (_, tariq) = w.staff("Tariq", host::roles::TUTOR).await;
    let (_, rita) = w.staff("Rita", host::roles::ACCOUNTANT).await;
    let b = started_batch(&w, &coordinator, "B").await;
    let b2 = w.open_batch(&coordinator, "B2", day(30)).await;
    let (omar, sara) = (
        tenant(&w.app, "Omar Khan").await,
        tenant(&w.app, "Sara Ali").await,
    );
    let e1 = enrolments::create(
        w.state(),
        &dina,
        NewEnrolment::course(omar.0, b.id, 252_000)
            .admin_fee(10_500)
            .terms_accepted(),
    )
    .await
    .unwrap();
    pay_by_card(&w, &e1).await;
    let e2 = enrolments::create(
        w.state(),
        &dina,
        NewEnrolment::course(sara.0, b.id, 252_000).terms_accepted(),
    )
    .await
    .unwrap();
    pay_by_card(&w, &e2).await;
    let planned = edtech::Session::of_batch(w.db(), b.id).await.unwrap();
    for session in planned.iter().take(5) {
        edtech::sessions::submit_attendance(w.state(), &tariq, session.id)
            .await
            .expect("the attendance is submitted");
    }
    assert_eq!(
        Batch::find_or_404(w.db(), b.id).await.unwrap().status,
        edtech::BatchStatus::Running
    );

    let asked = batches::terminate(
        w.state(),
        &coordinator,
        b.id,
        "venue",
        None,
        vec![
            edtech::exit::ExitChoice::withdraw(e1.id),
            edtech::exit::ExitChoice::transfer(e2.id, b2.id),
        ],
    )
    .await
    .unwrap();
    let request = asked.approval_request.expect("the centre manager decides");
    assert!(
        refunds(&w).await.is_empty(),
        "nothing moves before the approval"
    );
    wl_kernel::approvals::approve(w.state(), request, &manager, "")
        .await
        .unwrap();

    let sessions = edtech::Session::of_batch(w.db(), b.id).await.unwrap();
    let count =
        |status: edtech::SessionStatus| sessions.iter().filter(|s| s.status == status).count();
    assert_eq!(count(edtech::SessionStatus::Cancelled), 7);
    assert_eq!(count(edtech::SessionStatus::Held), 5);
    let one = reload(&w, &e1).await;
    assert_eq!(one.status, EnrolmentStatus::Withdrawn);
    assert_eq!(one.refund_amount, 147_000);
    let notes = wl_invoicing::CreditNote::query().get(w.db()).await.unwrap();
    assert_eq!(notes.len(), 1, "one credit note, for E1 only");
    assert_eq!(notes[0].gross_total, 147_000, "AED 1,470.00");
    let all = refunds(&w).await;
    assert_eq!(all.len(), 1, "one refund");
    assert_eq!(all[0].amount, 147_000);
    assert_eq!(all[0].status, wl_payments::refund::RefundStatus::Requested);
    assert_eq!(
        all[0].requested_by,
        Some(coordinator_user.id),
        "recorded by the coordinator"
    );
    assert_eq!(all[0].approved_by, None, "and waiting for another person");
    // The recorder cannot approve it; the accountant can.
    assert!(
        wl_payments::refund::approve(w.state(), &coordinator, &all[0])
            .await
            .is_err(),
        "the person who recorded it may not approve it"
    );
    let approved = wl_payments::refund::approve(w.state(), &rita, &all[0])
        .await
        .expect("another person approves");
    assert_eq!(approved.status, wl_payments::refund::RefundStatus::Approved);

    let two = reload(&w, &e2).await;
    assert_eq!(two.status, EnrolmentStatus::Transferred);
    let next = Enrolment::find_or_404(w.db(), two.transferred_to_id.unwrap())
        .await
        .unwrap();
    assert_eq!(next.status, EnrolmentStatus::Active);
    assert_eq!(next.batch_id, Some(b2.id));
}

/// #448 point 1 with the real modules: open batch B (not started) with E1 who
/// paid AED 2,520.00 and the AED 210.00 registration fee on the card. The
/// coordinator cancels B for "too few students" and E1 -> withdraw: wl-invoicing
/// makes one credit note of AED 2,730.00 and wl-payments holds a refund of
/// AED 2,730.00 requested by the coordinator and awaiting approval; no admin fee
/// and no tier is kept.
#[renox::test]
async fn edu_13_a_cancelled_batch_refunds_everything_through_the_real_payments() {
    let w = boot().await;
    let (coordinator_user, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (_, dina) = w.staff("Dina", COUNSELLOR).await;
    let b = w.open_batch(&coordinator, "B", day(7)).await;
    let omar = tenant(&w.app, "Omar Khan").await;
    let e1 = enrolments::create(
        w.state(),
        &dina,
        NewEnrolment::course(omar.0, b.id, 252_000)
            .registration_fee(21_000)
            .admin_fee(10_500)
            .terms_accepted(),
    )
    .await
    .unwrap();
    pay_by_card(&w, &e1).await;

    let done = batches::cancel(
        w.state(),
        &coordinator,
        b.id,
        "too few students",
        vec![edtech::exit::ExitChoice::withdraw(e1.id)],
    )
    .await
    .unwrap();

    assert_eq!(done.status, edtech::BatchStatus::Cancelled);
    let one = reload(&w, &e1).await;
    assert_eq!(one.status, EnrolmentStatus::Withdrawn);
    assert_eq!(one.refund_amount, 273_000);
    let notes = wl_invoicing::CreditNote::query().get(w.db()).await.unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].gross_total, 273_000, "AED 2,730.00");
    let all = refunds(&w).await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].amount, 273_000);
    assert_eq!(all[0].status, wl_payments::refund::RefundStatus::Requested);
    assert_eq!(all[0].requested_by, Some(coordinator_user.id));
    assert_eq!(all[0].approved_by, None);
}

// ---- #451: who may reissue a certificate, with the roles of the host ----------------------------

/// #451 point 1 and the refusal: with the host's own roles, the academic
/// coordinator reissues an issued certificate (superseded, a linked draft with the
/// corrected name); a counsellor and a tutor, who lack
/// `certificates.issue`, are refused by the service and by the page, for the
/// reissue and for the copy, and nothing changes.
#[renox::test]
async fn edu_16_only_a_user_who_may_issue_reissues_or_copies() {
    let w = boot().await;
    let (_, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (dina_user, dina) = w.staff("Dina", COUNSELLOR).await;
    let (tariq_user, tariq) = w.staff("Tariq", host::roles::TUTOR).await;
    let batch = w.open_batch(&coordinator, "A", day(7)).await;
    let student = tenant(&w.app, "Ahmad Yusuf").await;
    let e = enrolments::create(
        w.state(),
        &dina,
        NewEnrolment::course(student.0, batch.id, 252_000).terms_accepted(),
    )
    .await
    .unwrap();
    enrolments::activate(w.state(), Some(&dina), e.id)
        .await
        .unwrap();
    enrolments::complete(w.state(), e.id).await.unwrap();
    let draft = edtech::Certificate::of_enrolment(w.db(), e.id)
        .await
        .unwrap()
        .remove(0);
    let issued = edtech::certificates::issue(w.state(), &coordinator, draft.id)
        .await
        .unwrap();
    assert_eq!(issued.number.as_deref(), Some("CRT-0001"));
    let corrected = || edtech::certificates::Corrections {
        student_name: Some("Ahmed Yusuf".to_owned()),
        ..Default::default()
    };

    for (who, user) in [(&dina, &dina_user), (&tariq, &tariq_user)] {
        let refused = edtech::certificates::reissue(
            w.state(),
            who,
            issued.id,
            "name correction",
            corrected(),
        )
        .await;
        assert!(matches!(refused, Err(Error::Forbidden)), "{}", user.name);
        let copy = edtech::certificates::print_copy(w.state(), who, issued.id).await;
        assert!(matches!(copy, Err(Error::Forbidden)), "{}", user.name);
        w.app.acting_as(user);
        for action in ["reissue", "copy"] {
            w.app
                .htmx()
                .post(
                    &format!("/edtech/certificates/{}/{action}", issued.id),
                    &[("reason", "name correction")],
                )
                .await
                .assert_forbidden();
        }
    }
    let still = edtech::Certificate::find_or_404(w.db(), issued.id)
        .await
        .unwrap();
    assert_eq!(still.status, edtech::CertificateStatus::Issued);
    assert_eq!(
        edtech::Certificate::of_enrolment(w.db(), e.id)
            .await
            .unwrap()
            .len(),
        1,
        "no draft was made"
    );

    let draft = edtech::certificates::reissue(
        w.state(),
        &coordinator,
        issued.id,
        "name correction",
        corrected(),
    )
    .await
    .expect("the coordinator reissues");
    assert_eq!(draft.status, edtech::CertificateStatus::Draft);
    assert_eq!(draft.supersedes_id, Some(issued.id));
    assert_eq!(
        edtech::Certificate::find_or_404(w.db(), issued.id)
            .await
            .unwrap()
            .status,
        edtech::CertificateStatus::Superseded
    );
}

// ---- #452: a notice that cannot be delivered to guardian 1 goes to guardian 2 -----------------

/// A student whose two guardians are related in wl-parties (the mother is the
/// primary one) and whose enrolment names the mother.
async fn family(w: &World, dina: &AuthUser) -> (Enrolment, PartyId, PartyId) {
    use wl_parties::relationships::{NewRelationship, RelationshipType, relate};
    let batch = w.open_batch(&person_auth(w).await, "A", day(7)).await;
    let child = tenant(&w.app, "Omar Khan").await;
    let mother = tenant(&w.app, "Huda Khan").await;
    let father = tenant(&w.app, "Khalid Khan").await;
    for (guardian, primary) in [(mother, true), (father, false)] {
        relate(
            w.state(),
            NewRelationship {
                from: guardian.0,
                to: child.0,
                relation_type: RelationshipType::GuardianOf,
                primary,
                ..NewRelationship::default()
            },
        )
        .await
        .expect("the guardian is related");
    }
    let e = enrolments::create(
        w.state(),
        dina,
        NewEnrolment::course(child.0, batch.id, 252_000)
            .guardian(mother.0)
            .terms_accepted(),
    )
    .await
    .unwrap();
    (e, mother, father)
}

/// A coordinator to open the batch of [`family`].
async fn person_auth(w: &World) -> AuthUser {
    w.staff("Amal", ACADEMIC_COORDINATOR).await.1
}

/// The messages of `type_key`, oldest first.
async fn sent_of(w: &World, type_key: &str) -> Vec<wl_notify::Message> {
    wl_notify::Message::query()
        .where_(wl_notify::Message::TYPE_KEY.eq(type_key))
        .order_by(wl_notify::Message::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// The tasks opened for a family notice that gave up.
async fn give_up_tasks_of_enrolments(w: &World) -> Vec<wl_ticketing::Ticket> {
    let prefix = "notify.give_up:edtech.enrolment:";
    wl_ticketing::Ticket::query()
        .get(w.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|t| {
            t.source_key
                .as_deref()
                .is_some_and(|k| k.starts_with(prefix))
        })
        .collect()
}

/// #452 point 5, run: a family notice whose e-mail to the mother (guardian 1)
/// bounces is sent to the father (guardian 2) and, since it arrives, no task is
/// opened for the counsellor.
#[renox::test]
async fn edu_17_a_notice_that_bounces_at_guardian_one_is_delivered_to_guardian_two() {
    let mail = support::BouncingMail::default();
    let w = boot_with(Some(mail.clone())).await;
    let (_, dina) = w.staff("Dina", COUNSELLOR).await;
    w.staff("Cora", COUNSELLOR).await;
    let (e, _, _) = family(&w, &dina).await;
    mail.accept("khalid.khan@parties.example");

    edtech::notices::tell_family(w.state(), &e, "Class moved", "Monday moves to Tuesday")
        .await
        .unwrap();
    w.app.run_all_jobs().await;

    let messages = sent_of(&w, edtech::notices::FAMILY_NOTICE).await;
    let to_mother = messages
        .iter()
        .find(|m| m.address.as_deref() == Some("huda.khan@parties.example"))
        .expect("guardian 1 was tried first");
    assert_eq!(to_mother.status, wl_notify::MessageStatus::Abandoned);
    let to_father = messages
        .iter()
        .find(|m| m.address.as_deref() == Some("khalid.khan@parties.example"))
        .expect("guardian 2 was tried next");
    assert_eq!(to_father.status, wl_notify::MessageStatus::Sent);
    assert_eq!(to_father.alternative_of, Some(to_mother.id));
    assert!(to_father.rendered_body.contains("Monday moves to Tuesday"));
    assert!(
        give_up_tasks_of_enrolments(&w).await.is_empty(),
        "delivered, so no task"
    );
}

/// #452 point 5, run: both guardians bounce, then the counsellor has ONE task.
#[renox::test]
async fn edu_17_a_notice_that_bounces_at_both_guardians_gives_the_counsellor_one_task() {
    let mail = support::BouncingMail::default();
    let w = boot_with(Some(mail.clone())).await;
    let (counsellor, dina) = w.staff("Dina", COUNSELLOR).await;
    let (e, _, _) = family(&w, &dina).await;

    edtech::notices::tell_family(w.state(), &e, "Class moved", "Monday moves to Tuesday")
        .await
        .unwrap();
    w.app.run_all_jobs().await;

    let messages = sent_of(&w, edtech::notices::FAMILY_NOTICE).await;
    let addresses: Vec<_> = messages.iter().filter_map(|m| m.address.clone()).collect();
    assert!(
        addresses.contains(&"huda.khan@parties.example".to_owned()),
        "{addresses:?}"
    );
    assert!(
        addresses.contains(&"khalid.khan@parties.example".to_owned()),
        "{addresses:?}"
    );
    assert!(
        messages
            .iter()
            .all(|m| m.status == wl_notify::MessageStatus::Abandoned),
        "none arrived"
    );
    let tasks = give_up_tasks_of_enrolments(&w).await;
    assert_eq!(tasks.len(), 1, "one task");
    assert_eq!(tasks[0].assignee_user_id, Some(counsellor.id));
}

// ---- #452: the clearance of a tutor who teaches in two campuses -----------------------------------

/// A minors batch of `tutor` at `campus`, on `days` of the week, from the
/// first Monday on or after `from`, opened while the clearance is valid.
async fn campus_batch(
    w: &World,
    coordinator: &AuthUser,
    course: &Course,
    tutor: (i64, i64),
    campus: i64,
    days: &[i64],
    from: NaiveDate,
) -> Batch {
    let mut room =
        wl_scheduling::Resource::new(&format!("Room {campus}"), wl_scheduling::ResourceType::Room);
    room.capacity = 14;
    room.save(w.db()).await.unwrap();
    let first = monday_from(from);
    let planned = batches::create(
        w.state(),
        coordinator,
        NewBatch::new(
            course.id,
            12,
            (first, first + TimeDelta::days(27)),
            days,
            "18:00",
            90,
        )
        .tutor(tutor.0, Some(tutor.1))
        .room(room.id, Some(14))
        .campus(wl_kernel::Subject::new("test.campus", campus)),
    )
    .await
    .expect("the batch is planned");
    batches::open(w.state(), coordinator, planned.id)
        .await
        .expect("the batch opens")
}

/// #452 point 7, run: the tutor Layla teaches a minors batch at campus 3 and
/// another at campus 4; her police clearance ends in 25 days, so sessions of both
/// come after it. The look-ahead run (30 days) tells the academic coordinator of
/// campus 3 about the sessions of campus 3 only, the coordinator of campus 4
/// about campus 4 only, and the coordinator of campus 5, who has no session
/// affected, nothing.
#[renox::test]
async fn edu_17_a_tutors_clearance_expiring_tells_the_coordinator_of_each_campus_affected() {
    use renox::auth::permissions::Scope;
    let w = boot().await;
    let (_, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    w.permit_until(day(400)).await;
    let (layla, employee) = employed(&w, "Layla Hassan").await;
    let clearance = record_document(
        &w.app,
        "police_clearance",
        wl_kernel::workforce::EmployeeId(employee).subject(),
        day(400),
    )
    .await;
    let kids = courses::create(
        w.state(),
        &coordinator,
        NewCourse::new("KIDS-1", "Kids mathematics", "Mathematics", 30).for_minors(),
    )
    .await
    .unwrap();
    let three = campus_batch(
        &w,
        &coordinator,
        &kids,
        (layla.id, employee),
        3,
        &[1, 3],
        day(7),
    )
    .await;
    let four = campus_batch(
        &w,
        &coordinator,
        &kids,
        (layla.id, employee),
        4,
        &[2, 4],
        day(7),
    )
    .await;
    let cora = scoped_person(
        &w.app,
        "Cora",
        ACADEMIC_COORDINATOR,
        &Scope::new("test.campus", 3),
    )
    .await;
    let dalia = scoped_person(
        &w.app,
        "Dalia",
        ACADEMIC_COORDINATOR,
        &Scope::new("test.campus", 4),
    )
    .await;
    let eman = scoped_person(
        &w.app,
        "Eman",
        ACADEMIC_COORDINATOR,
        &Scope::new("test.campus", 5),
    )
    .await;

    expires(&clearance, day(25), &w).await;
    wl_documents::expiry::run(w.state(), wl_kernel::documents::today())
        .await
        .unwrap();

    let text = |messages: &[wl_notify::Message]| {
        messages
            .iter()
            .map(|m| {
                format!(
                    "{} {}",
                    m.rendered_subject.clone().unwrap_or_default(),
                    m.rendered_body
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let for_cora = support::messages_to(&w.app, "documents.expiring", &cora).await;
    let for_dalia = support::messages_to(&w.app, "documents.expiring", &dalia).await;
    assert_eq!(for_cora.len(), 1, "campus 3 is told once");
    assert_eq!(for_dalia.len(), 1, "campus 4 is told once");
    assert!(
        support::messages_to(&w.app, "documents.expiring", &eman)
            .await
            .is_empty(),
        "campus 5 has no affected session"
    );
    let (cora_text, dalia_text) = (text(&for_cora), text(&for_dalia));
    assert!(cora_text.contains(&three.number()), "{cora_text}");
    assert!(!cora_text.contains(&four.number()), "{cora_text}");
    assert!(dalia_text.contains(&four.number()), "{dalia_text}");
    assert!(!dalia_text.contains(&three.number()), "{dalia_text}");
}

// ---- #452: a campaign with cost entries, its report and the over-budget wait --------------------

/// #452 point 10, run with the real wl-sales: the campaign "Instagram" (channel
/// recorded as meta, source social) has a budget of AED 5,000 and two cost
/// entries (AED 3,000 and AED 1,000), 20 leads of which 5 are won and 4 new
/// customers with a first enrolment invoice of AED 1,500 each excluding VAT.
/// The report row shows leads, conversion, cost per lead, CAC and ROAS; a cost
/// entry of AED 2,000 takes the campaign over its budget, so it waits for a
/// manager (not the requester) and is counted only once approved.
#[renox::test]
async fn edu_17_a_campaign_reports_its_cost_per_lead_cac_and_roas_and_waits_over_budget() {
    use wl_kernel::sales::{Attribute, FirstSale, campaign_attribution};
    use wl_sales::costs::{self, CostInput};
    use wl_sales::report::{self, Filter, GroupBy, Viewer};
    let w = boot().await;
    let (marketer, _) = w.staff("Maya", host::roles::MARKETING).await;
    let (manager, _) = w.staff("Mo", host::roles::MARKETING_MANAGER).await;
    let source = wl_sales::LeadSource::by_code(w.db(), "social")
        .await
        .unwrap()
        .expect("social is offered")
        .id;
    let campaign = wl_sales::Campaign {
        name: "Instagram October".into(),
        code: "IG-OCT".into(),
        purpose: Some(wl_sales::Purpose::Acquisition),
        marketing_channel: Some(wl_sales::MarketingChannel::Meta.into()),
        lead_source_id: Some(source),
        owner_id: Some(marketer.id),
        start_date: Some(day(-20)),
        end_date: Some(day(20)),
        budget: Some(500_000),
        ..wl_sales::Campaign::default()
    }
    .store(w.db())
    .await
    .unwrap()
    .unwrap();
    wl_sales::Campaign::query()
        .where_(wl_sales::Campaign::ID.eq(campaign.id))
        .update(w.db(), &[("status", &"active")])
        .await
        .unwrap();
    let entry = |amount: i64| CostInput {
        campaign_id: campaign.id,
        entry_date: day(-5),
        amount,
        description: "Instagram ads".into(),
        requested_by: marketer.id,
        manages_costs: false,
    };
    for amount in [300_000, 100_000] {
        let saved = costs::save(w.state(), entry(amount))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.status,
            wl_sales::CostStatus::Approved,
            "within the budget"
        );
    }
    for i in 0..20 {
        let new = wl_sales::NewLead::web_form(None, &format!("Lead {i}")).campaign_code("IG-OCT");
        let wl_sales::Captured::Created(lead) = wl_sales::lead::capture(w.state(), new, day(-4))
            .await
            .unwrap()
            .unwrap()
        else {
            panic!("a lead was expected");
        };
        if i < 5 {
            wl_sales::Lead::query()
                .where_(wl_sales::Lead::ID.eq(lead.id))
                .update(
                    w.db(),
                    &[("status", &"won"), ("won_at", &Some(renox::db::now()))],
                )
                .await
                .unwrap();
        }
    }
    let attribution = campaign_attribution(w.state());
    for n in 0..4 {
        let party = PartyId(500 + n);
        attribution
            .attribute(
                w.state(),
                Attribute::party(party, "social").campaign("IG-OCT"),
            )
            .await
            .unwrap();
        attribution
            .record_first_sale(
                w.state(),
                FirstSale::new(
                    party,
                    wl_kernel::Subject::new("invoicing.invoice", 700 + n),
                    150_000,
                    day(-2),
                ),
            )
            .await
            .unwrap();
    }

    let config = wl_sales::config::config_of(w.state());
    let row = |facts: &report::Facts| {
        report::compute(facts, &Filter::default(), GroupBy::Campaign, &Viewer::all()).rows[0]
            .cells()
    };
    let facts = report::load(w.db(), &config).await.unwrap();
    let cells = row(&facts);
    assert_eq!(cells.leads, "20");
    assert_eq!(cells.won, "5");
    assert_eq!(cells.conversion, "25.0%");
    assert_eq!(cells.customers, "4");
    assert_eq!(cells.value, "6,000.00");
    assert_eq!(cells.cost, "4,000.00");
    assert_eq!(cells.cost_per_lead, "200.00");
    assert_eq!(cells.cac, "1,000.00");
    assert_eq!(cells.roas, "1.50");

    // AED 2,000 more takes the campaign over its AED 5,000 budget.
    let over = costs::save(w.state(), entry(200_000))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        over.status,
        wl_sales::CostStatus::PendingApproval,
        "it waits"
    );
    let request = over.approval_request_id.expect("a request for a manager");
    let facts = report::load(w.db(), &config).await.unwrap();
    assert_eq!(
        row(&facts).cost,
        "4,000.00",
        "a waiting entry is not counted"
    );
    assert!(
        wl_kernel::approvals::approve(w.state(), request, &marketer, "")
            .await
            .is_err(),
        "the requester does not approve it"
    );
    wl_kernel::approvals::approve(w.state(), request, &manager, "")
        .await
        .expect("a manager approves");
    let facts = report::load(w.db(), &config).await.unwrap();
    let cells = row(&facts);
    assert_eq!(cells.cost, "6,000.00");
    assert_eq!(cells.cost_per_lead, "300.00");
    assert_eq!(cells.cac, "1,500.00");
    assert_eq!(cells.roas, "1.00");
}

// ---- #452: hiring a tutor starts the onboarding checklist ---------------------------------------

/// #452 point 12, run with the real wl-hr: hiring a tutor in `onboarding` creates
/// the tutor / trainer checklist with steps for the centre manager, the academic
/// coordinator and the tutor's own account; the checklist blocks nothing, so the
/// employee is activated while steps are open; a step still open after its due
/// date reminds its assignee on the first run after it and again two days later,
/// and not in between (`HRM.RUL.17`).
#[renox::test]
async fn edu_17_hiring_a_tutor_creates_the_checklist_and_reminds_the_open_steps() {
    let w = boot().await;
    let (manager, _) = w.staff("Mona", host::roles::CENTRE_MANAGER).await;
    let (coordinator, _) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (account, _) = w.staff("Tariq Nasser", host::roles::TUTOR).await;
    let party = tenant(&w.app, "Tariq Nasser").await;

    let hired = wl_hr::employee::create(
        w.state(),
        wl_hr::employee::NewEmployee::new(party, "tutor", day(0))
            .user(account.id)
            .status(wl_hr::employee::EmployeeStatus::Onboarding),
    )
    .await
    .expect("the tutor is hired");

    let started = wl_hr::Onboarding::query()
        .where_(wl_hr::Onboarding::EMPLOYEE_ID.eq(hired.id))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(started.len(), 1, "one onboarding");
    assert_eq!(started[0].start_trigger, "hire");
    let tutor_checklist = wl_hr::OnboardingTemplate::query()
        .where_(wl_hr::OnboardingTemplate::NAME_EN.eq("Tutor and trainer onboarding"))
        .first(w.db())
        .await
        .unwrap()
        .expect("the tutor checklist is seeded");
    let applied = started[0].templates(w.db()).await.unwrap();
    assert!(
        applied.iter().any(|t| t.template_id == tutor_checklist.id),
        "the tutor checklist applies"
    );
    let steps = started[0].steps(w.db()).await.unwrap();
    let mut told = Vec::new();
    for step in &steps {
        for user in step.assignees(w.db()).await.unwrap() {
            told.push((
                user,
                step.title_en.clone(),
                step.due_date,
                step.owner.clone(),
            ));
        }
    }
    for (name, user) in [
        ("the centre manager", manager.id),
        ("the academic coordinator", coordinator.id),
        ("the employee", account.id),
    ] {
        assert!(
            told.iter().any(|(u, ..)| *u == user),
            "{name} has a step: {told:?}"
        );
    }
    assert!(
        told.iter()
            .filter(|(u, ..)| *u == account.id)
            .all(|(.., owner)| owner == "employee"),
        "the employee holds the steps that are theirs"
    );

    // The role is active before the checklist is complete.
    let (hr, _) = w.staff("Hana", host::roles::HR).await;
    w.app.acting_as(&hr);
    let page = w
        .app
        .htmx()
        .post(
            &format!("/hr/employees/{}/transitions/activate", hired.id),
            &[("reason", "")],
        )
        .await;
    assert!(
        page.status.is_success() || page.status.is_redirection(),
        "completion blocks nothing: {} {}",
        page.status,
        page.text()
    );
    assert_eq!(
        wl_hr::Employee::find_or_404(w.db(), hired.id)
            .await
            .unwrap()
            .status,
        wl_hr::employee::EmployeeStatus::Active
    );
    assert!(
        steps.iter().any(|s| s.status == wl_hr::StepStatus::Open),
        "and steps are still open"
    );

    // An open step of the employee's own is reminded after its due date.
    let (_, title, due, _) = told
        .iter()
        .find(|(u, title, ..)| {
            *u == account.id && title == "Upload your Emirates ID and your documents"
        })
        .cloned()
        .expect("a step of the employee");
    let reminded = |user: i64| {
        let title = title.clone();
        let w = &w;
        async move {
            support::messages_to(
                &w.app,
                "hr.onboarding_step_reminder",
                &User::find_or_404(w.db(), user).await.unwrap(),
            )
            .await
            .into_iter()
            .filter(|m| {
                m.rendered_body.contains(&title)
                    || m.subject_label.as_deref() == Some(title.as_str())
            })
            .count()
        }
    };
    wl_hr::reminders::run(w.state(), due).await.unwrap();
    w.app.run_all_jobs().await;
    assert_eq!(reminded(account.id).await, 0, "not on the due date");
    wl_hr::reminders::run(w.state(), due + TimeDelta::days(1))
        .await
        .unwrap();
    w.app.run_all_jobs().await;
    assert_eq!(reminded(account.id).await, 1, "the first run after it");
    wl_hr::reminders::run(w.state(), due + TimeDelta::days(2))
        .await
        .unwrap();
    w.app.run_all_jobs().await;
    assert_eq!(reminded(account.id).await, 1, "not the next day");
    wl_hr::reminders::run(w.state(), due + TimeDelta::days(3))
        .await
        .unwrap();
    w.app.run_all_jobs().await;
    assert_eq!(
        reminded(account.id).await,
        2,
        "and two days after the first"
    );
}

// ---- #453: the refund of a withdrawal through the app's own path --------------------------------

/// The student of a card-paid, active enrolment withdraws through the app (the
/// counsellor asks, the centre manager approves): the refund of the credit note
/// waits in wl-payments, REQUESTED by the counsellor.
async fn withdrawn_refund(w: &World) -> (Enrolment, Refund, User, String) {
    let (_, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (dina_user, dina) = w.staff("Dina", COUNSELLOR).await;
    let (manager_user, _) = w.staff("Mona", host::roles::CENTRE_MANAGER).await;
    let batch = w.open_batch(&coordinator, "A", day(7)).await;
    let sara = tenant(&w.app, "Sara Ali").await;
    let e = enrolments::create(
        w.state(),
        &dina,
        NewEnrolment::course(sara.0, batch.id, 252_000).terms_accepted(),
    )
    .await
    .unwrap();
    pay_by_card(w, &e).await;
    let txn = start_payment_txn(w, &e).await;
    let asked = enrolments::request_withdrawal(w.state(), Some(&dina), e.id, "moving away")
        .await
        .unwrap();
    wl_kernel::approvals::approve(
        w.state(),
        asked.approval_request.expect("the centre manager decides"),
        &manager_user,
        "",
    )
    .await
    .unwrap();
    let refund = refunds(w).await.remove(0);
    assert_eq!(refund.status, wl_payments::refund::RefundStatus::Requested);
    assert_eq!(refund.requested_by, Some(dina_user.id), "A recorded it");
    (e, refund, dina_user, txn)
}

/// The gateway transaction of the card payment of `e`.
async fn start_payment_txn(w: &World, e: &Enrolment) -> String {
    Payment::query()
        .where_(Payment::PAYMENT_LINK_ID.eq(e.payment_link_id.expect("a link")))
        .first(w.db())
        .await
        .unwrap()
        .expect("the payment")
        .gateway_transaction
        .expect("a transaction")
}

/// #453 point 1 on the app's own withdraw-to-refund path (no probe routes): the
/// refund is recorded by A (the counsellor); A cannot approve it, not even holding
/// the accountant role; B (another accountant) approves, the accountant pays the
/// student by bank transfer and the refund is `paid`.
#[renox::test]
async fn edu_18_a_withdrawal_refund_needs_another_approver_and_is_paid() {
    use wl_kernel::payments::PaymentMethod;
    use wl_payments::refund::RefundStatus;
    let w = boot().await;
    let (_, rita) = w.staff("Rita", host::roles::ACCOUNTANT).await;
    let (e, refund, dina_user, _) = withdrawn_refund(&w).await;

    dina_user
        .assign_role(w.db(), host::roles::ACCOUNTANT)
        .await
        .unwrap();
    let dina_accountant = w.auth(&dina_user).await;
    assert!(
        wl_payments::refund::approve(w.state(), &dina_accountant, &refund)
            .await
            .is_err(),
        "the recorder does not approve their own refund"
    );
    assert_eq!(refunds(&w).await[0].status, RefundStatus::Requested);
    let approved = wl_payments::refund::approve(w.state(), &rita, &refund)
        .await
        .expect("B approves");
    assert!(approved.approved_by.is_some());
    assert_ne!(approved.approved_by, approved.requested_by);
    let paid = wl_payments::refund::pay(
        w.state(),
        &rita,
        &approved,
        PaymentMethod::Transfer,
        "BANK-88",
    )
    .await
    .expect("the transfer is recorded");
    assert_eq!(paid.status, RefundStatus::Paid);
    assert_eq!(reload(&w, &e).await.status, EnrolmentStatus::Withdrawn);
}

/// #453 point 1, the failure: the refund of the withdrawal goes back to the card
/// (the card and its transaction are set on the refund by the test: the app
/// does not know the transaction, see the deferred line) and the gateway
/// declines it. 24 hours later (not before) the system retries once; it is
/// declined again and the accountant has ONE task to pay by another method; the
/// accountant records a bank transfer to new bank details, another accountant
/// approves the details, the transfer is paid and the task closes.
#[renox::test]
async fn edu_18_a_declined_card_refund_is_retried_once_then_the_accountant_pays_by_transfer() {
    use wl_kernel::payments::PaymentMethod;
    use wl_payments::gateway::RefundOutcome;
    use wl_payments::refund::{RefundStatus, gateway_refund_outcome};
    use wl_payments::refund_recover::{change_method, retries_of};
    let w = boot().await;
    let (_, rita) = w.staff("Rita", host::roles::ACCOUNTANT).await;
    let (_, rana) = w.staff("Rana", host::roles::ACCOUNTANT).await;
    let (e, refund, _, txn) = withdrawn_refund(&w).await;
    Refund::query()
        .where_(Refund::ID.eq(refund.id))
        .update(
            w.db(),
            &[
                ("method", &Some(PaymentMethod::Gateway)),
                ("gateway_txn", &Some(txn)),
            ],
        )
        .await
        .unwrap();
    let refund = Refund::find_or_404(w.db(), refund.id).await.unwrap();
    let approved = wl_payments::refund::approve(w.state(), &rita, &refund)
        .await
        .expect("B approves");

    let attempt = wl_payments::refund::send_to_gateway(w.state(), &rita, &approved)
        .await
        .expect("sent to the gateway");
    let failed = gateway_refund_outcome(
        w.state(),
        attempt.gateway_ref.as_deref().expect("a reference"),
        RefundOutcome::Declined {
            reason: "card expired".to_owned(),
        },
    )
    .await
    .unwrap()
    .expect("applied");
    assert_eq!(failed.status, RefundStatus::Failed);
    assert!(
        wl_payments::FollowupTask::open_for(w.db(), "accountant")
            .await
            .unwrap()
            .is_empty(),
        "the system retries first"
    );

    // Not two minutes before 24 hours after the failure; once after it.
    w.app.travel(Duration::from_secs(24 * 3600 - 120));
    w.app
        .run_scheduled(wl_payments::refund_retry::TASK)
        .await
        .unwrap();
    assert_eq!(retries_of(w.db(), refund.id).await.unwrap(), 0);
    w.app.travel(Duration::from_secs(300));
    w.app
        .run_scheduled(wl_payments::refund_retry::TASK)
        .await
        .unwrap();
    assert_eq!(retries_of(w.db(), refund.id).await.unwrap(), 1);
    assert_eq!(
        Refund::find_or_404(w.db(), refund.id).await.unwrap().status,
        RefundStatus::Approved
    );

    // The retry is declined too: no third attempt, one task for the accountant.
    let second = wl_payments::RefundAttempt::latest(w.db(), refund.id)
        .await
        .unwrap()
        .expect("an attempt")
        .gateway_ref
        .expect("a reference");
    gateway_refund_outcome(
        w.state(),
        &second,
        RefundOutcome::Declined {
            reason: "card closed".to_owned(),
        },
    )
    .await
    .unwrap();
    w.app.travel(Duration::from_secs(200 * 3600));
    w.app
        .run_scheduled(wl_payments::refund_retry::TASK)
        .await
        .unwrap();
    assert_eq!(
        retries_of(w.db(), refund.id).await.unwrap(),
        1,
        "no third attempt"
    );
    let tasks = wl_payments::FollowupTask::open_for(w.db(), "accountant")
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1, "one task to pay by another method");
    assert_eq!(tasks[0].refund_id, Some(refund.id));

    // The accountant records the other method.
    let failed = Refund::find_or_404(w.db(), refund.id).await.unwrap();
    let changed = change_method(
        w.state(),
        &rita,
        &failed,
        PaymentMethod::Transfer,
        Some("AE070331234567890123456"),
    )
    .await
    .expect("the accountant changes the method");
    let ready = if changed.status == RefundStatus::Requested {
        wl_payments::refund::approve(w.state(), &rana, &changed)
            .await
            .expect("another accountant approves the new details")
    } else {
        changed
    };
    assert_eq!(ready.method, Some(PaymentMethod::Transfer));
    let paid =
        wl_payments::refund::pay(w.state(), &rita, &ready, PaymentMethod::Transfer, "BANK-89")
            .await
            .expect("the transfer is recorded as paid");
    assert_eq!(paid.status, RefundStatus::Paid);
    assert!(
        wl_payments::FollowupTask::open_for(w.db(), "accountant")
            .await
            .unwrap()
            .is_empty(),
        "the task closed"
    );
    assert_eq!(reload(&w, &e).await.status, EnrolmentStatus::Withdrawn);
}

// ---- #453: the membership charge that fails and the pause allowance, with the real wl-invoicing --

/// The retry notices the parent `payer` has had.
async fn retries(w: &World, payer: PartyId) -> usize {
    support::messages_to_party(&w.app, wl_invoicing::subscriptions::RETRY_NOTICE, payer)
        .await
        .len()
}

/// The daily check of the subscriptions on the day `40 + on`, then the jobs.
async fn check(w: &World, on: i64) {
    w.app
        .at_travelled_time(wl_invoicing::subscriptions::evaluate(
            w.state(),
            day(40 + on),
        ))
        .await
        .unwrap();
    w.app.run_all_jobs().await;
}

/// #453 point 3, run: the membership's first cycle invoice is not paid. The
/// daily check makes the subscription `past_due`, chases it on days 1, 3 and 7
/// (the parent gets one retry notice each time and not on the days between) and
/// then `suspended`, with the host's values 1, 3, 7 and suspension on.
#[renox::test]
async fn edu_18_a_membership_charge_that_fails_is_retried_on_days_1_3_7_then_suspended() {
    use wl_kernel::subscriptions::SubscriptionStatus;
    let w = boot().await;
    let (_, desk) = w.staff("Dina", COUNSELLOR).await;
    let (e, subscription) = membership(&w, &desk, "Sara Ali").await;
    assert_eq!(
        wl_invoicing::subscriptions::bill_due(w.state(), day(0))
            .await
            .unwrap(),
        1
    );
    // Forty days pass with the invoice unpaid.
    w.app.travel(Duration::from_secs(40 * 86_400));
    w.app
        .at_travelled_time(wl_invoicing::overdue::run(w.state()))
        .await
        .unwrap();
    let payer = PartyId(e.payer_party_id);
    check(&w, 0).await;
    assert_eq!(
        subscription_now(&w, &subscription).await.status,
        SubscriptionStatus::PastDue
    );
    assert_eq!(retries(&w, payer).await, 0);
    for (on, expected) in [(1, 1), (2, 1), (3, 2), (5, 2)] {
        check(&w, on).await;
        assert_eq!(retries(&w, payer).await, expected, "day {on}");
        assert_eq!(
            subscription_now(&w, &subscription).await.status,
            SubscriptionStatus::PastDue,
            "day {on}"
        );
    }
    // The third retry is the last: with it the subscription is suspended.
    check(&w, 7).await;
    assert_eq!(retries(&w, payer).await, 3, "the third retry, on day 7");
    assert_eq!(
        subscription_now(&w, &subscription).await.status,
        SubscriptionStatus::Suspended
    );
    check(&w, 10).await;
    let suspended = subscription_now(&w, &subscription).await;
    assert_eq!(suspended.status, SubscriptionStatus::Suspended);
    assert_eq!(retries(&w, payer).await, 3, "no fourth retry");
}

/// #453 point 3, run: a user may pause the membership for 30 days in a year; the
/// 31st day is refused. The refusal changes nothing; after the 30 days the
/// allowance is used and a further day is refused.
#[renox::test]
async fn edu_18_a_membership_pause_of_30_days_is_allowed_and_the_31st_day_is_refused() {
    use wl_kernel::subscriptions::{PauseCause, PauseSubscription, SubscriptionStatus};
    let w = boot().await;
    let (_, desk) = w.staff("Dina", COUNSELLOR).await;
    let (_, accountant) = w.staff("Rita", host::roles::ACCOUNTANT).await;
    let (e, subscription) = membership(&w, &desk, "Sara Ali").await;

    let too_long = wl_invoicing::subscriptions::pause(
        w.state(),
        &subscription,
        &PauseSubscription::new(PauseCause::User, day(0))
            .reason("travelling")
            .until(day(31)),
        Some(&accountant),
    )
    .await;
    assert!(too_long.is_err(), "31 days are refused");
    assert_eq!(
        subscription_now(&w, &subscription).await.status,
        SubscriptionStatus::Active,
        "nothing changed"
    );
    assert_eq!(reload(&w, &e).await.status, EnrolmentStatus::Active);

    wl_invoicing::subscriptions::pause(
        w.state(),
        &subscription,
        &PauseSubscription::new(PauseCause::User, day(0))
            .reason("travelling")
            .until(day(30)),
        Some(&accountant),
    )
    .await
    .expect("30 days are allowed");
    let paused = subscription_now(&w, &subscription).await;
    assert_eq!(paused.status, SubscriptionStatus::Paused);
    assert_eq!(paused.pause_days_used, 30);
    assert_eq!(reload(&w, &e).await.status, EnrolmentStatus::Frozen);

    wl_invoicing::subscriptions::resume(
        w.state(),
        &subscription,
        PauseCause::User,
        day(30),
        Some(&accountant),
    )
    .await
    .unwrap();
    let again = wl_invoicing::subscriptions::pause(
        w.state(),
        &subscription,
        &PauseSubscription::new(PauseCause::User, day(30))
            .reason("again")
            .until(day(31)),
        Some(&accountant),
    )
    .await;
    assert!(again.is_err(), "the year's 30 days are used");
}

// ---- #453: a desk cash shortage, through the real wl-payments cash pages -------------------------

/// #453 point 5, run: the counsellor Dina took AED 2,250.00 in cash at the desk
/// (the handover is made as the delivery of cash into custody, which the app does
/// not do yet, see the deferred line); the accountant counts AED 2,230.00: AED 20.00
/// short. The accountant cannot write it off; the centre manager asks to, and
/// since the limit is AED 0 it waits for the company admin, not for the
/// requester; the holder may not approve. Salary recovery is not offered: no sheet
/// on the page and the route refuses; after the approval the shortage is written off.
#[renox::test]
async fn edu_18_a_desk_shortage_of_aed_20_waits_for_approval_and_never_comes_off_the_salary() {
    use wl_kernel::custody::{CashCollection, HandoverPeriod, custody};
    let w = boot().await;
    let (dina, _) = w.staff("Dina", COUNSELLOR).await;
    let (rita, _) = w.staff("Rita", host::roles::ACCOUNTANT).await;
    let (mona, _) = w.staff("Mona", host::roles::CENTRE_MANAGER).await;
    let (ada, _) = w.staff("Ada", host::roles::ADMIN).await;
    let id = custody(w.state())
        .collect(
            w.state(),
            CashCollection::new(
                dina.id,
                HandoverPeriod::day(wl_kernel::documents::today()),
                225_000,
                wl_kernel::Subject::new("edtech.enrolment", 1),
            )
            .location("Dubai desk"),
        )
        .await
        .expect("the cash is collected")
        .expect("payments is on");
    let handover = wl_payments::CashHandover::find_or_404(w.db(), id.0)
        .await
        .unwrap();

    let post = |user: &User, path: String, pairs: Vec<(&'static str, &'static str)>| {
        let w = &w;
        let user = user.clone();
        async move {
            w.app.acting_as(&user);
            w.app.htmx().post(&path, &pairs).await
        }
    };
    post(
        &rita,
        format!("/cash/{}/count", handover.id),
        vec![("counted", "2230.00")],
    )
    .await
    .assert_ok();
    let short = wl_payments::CashHandover::find_or_404(w.db(), id.0)
        .await
        .unwrap();
    assert_eq!(short.status, wl_kernel::custody::HandoverStatus::Short);
    assert_eq!(short.difference, 2_000, "AED 20.00 short");

    // Salary recovery is not offered: no sheet, and the route refuses.
    w.app.acting_as(&rita);
    let page = w.app.get(&format!("/cash/{}", handover.id)).await;
    page.assert_ok();
    assert!(
        !page.text().to_lowercase().contains("salary"),
        "no salary deduction sheet"
    );
    let deduction = post(
        &rita,
        format!("/cash/{}/deductions", handover.id),
        vec![("amount", "20.00"), ("note", "from the salary")],
    )
    .await;
    assert_eq!(deduction.status.as_u16(), 422, "salary recovery is off");

    // The accountant counts but does not write off; the manager asks.
    post(
        &rita,
        format!("/cash/{}/write-off", handover.id),
        vec![("reason", "Till error")],
    )
    .await
    .assert_forbidden();
    post(
        &mona,
        format!("/cash/{}/write-off", handover.id),
        vec![("reason", "Till error")],
    )
    .await
    .assert_ok();
    assert_eq!(
        wl_payments::CashHandover::find_or_404(w.db(), id.0)
            .await
            .unwrap()
            .status,
        wl_kernel::custody::HandoverStatus::Short,
        "AED 20 needs an approval"
    );
    let request = wl_kernel::approvals::ApprovalRequest::pending_about(w.db(), &short.subject())
        .await
        .unwrap()
        .pop()
        .expect("an approval waits");
    assert_eq!(request.requester_id, mona.id);
    assert!(
        wl_kernel::approvals::approve(w.state(), request.id, &mona, "mine")
            .await
            .is_err(),
        "the requester does not approve"
    );
    assert!(
        wl_kernel::approvals::approve(w.state(), request.id, &dina, "mine")
            .await
            .is_err(),
        "the holder does not approve"
    );
    wl_kernel::approvals::approve(w.state(), request.id, &ada, "agreed")
        .await
        .expect("the company admin approves");
    let after = wl_payments::CashHandover::find_or_404(w.db(), id.0)
        .await
        .unwrap();
    assert_eq!(after.status, wl_kernel::custody::HandoverStatus::WrittenOff);
    assert_eq!(after.written_off, 2_000);
}

// ---- #446 / EDU.ACL.03: a tutor marks only the sessions of their own batches --------------------

/// A tutor with an account: a wl-hr employee linked to a user holding the tutor
/// role, and the person resource behind them.
async fn tutor_with_account(
    w: &World,
    name: &str,
) -> (wl_scheduling::Resource, i64, User, AuthUser) {
    let (user, auth) = w.staff(name, host::roles::TUTOR).await;
    let party = tenant(&w.app, name).await;
    let hired = wl_hr::employee::create(
        w.state(),
        wl_hr::employee::NewEmployee::new(party, "tutor", day(-400))
            .user(user.id)
            .status(wl_hr::employee::EmployeeStatus::Active),
    )
    .await
    .expect("the tutor is hired");
    (
        tutor_resource(w, name, Some(hired.id)).await,
        hired.id,
        user,
        auth,
    )
}

/// `EDU.ACL.03`: Layla teaches the batch; Omar, another tutor of the centre, may
/// neither submit its attendance nor correct it, by the service or by the page;
/// Layla does both, and the coordinator corrects any session.
#[renox::test]
async fn edu_14_a_tutor_marks_and_corrects_only_the_sessions_of_their_own_batch() {
    use edtech::attendance::Mark;
    let w = boot().await;
    let (_, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (_, desk) = w.staff("Dina", COUNSELLOR).await;
    w.permit_until(day(400)).await;
    let (layla, layla_employee, _, layla_auth) = tutor_with_account(&w, "Layla Hassan").await;
    let (_, _, omar_user, omar_auth) = tutor_with_account(&w, "Omar Nasser").await;
    let course = w.course(&coordinator).await;
    let batch = planned_of(
        &w,
        &coordinator,
        &course,
        "A",
        (layla.id, Some(layla_employee)),
    )
    .await;
    let batch = batches::open(w.state(), &coordinator, batch.id)
        .await
        .unwrap();
    let (e, _) = package_student(&w, &desk, "Sara Ali", 10, None).await;
    let ended = renox::db::now() - TimeDelta::hours(2);
    let session = session_at(&w, &batch, 0, ended - TimeDelta::minutes(90)).await;
    edtech::roster::add_student(w.state(), &desk, session.id, e.id)
        .await
        .unwrap();

    // Omar is refused, by the service and by the page; nothing changed.
    let refused = edtech::sessions::submit_attendance_marked(
        w.state(),
        &omar_auth,
        session.id,
        &[(e.id, Mark::Absent)],
        false,
    )
    .await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    w.app.acting_as(&omar_user);
    w.app
        .htmx()
        .post(
            &format!("/edtech/sessions/{}/attendance", session.id),
            &[("x", "y")],
        )
        .await
        .assert_forbidden();
    assert_eq!(
        edtech::Session::find_or_404(w.db(), session.id)
            .await
            .unwrap()
            .status,
        edtech::SessionStatus::Scheduled
    );

    // Layla marks, then Omar may not correct, and Layla and the coordinator may.
    edtech::sessions::submit_attendance_marked(
        w.state(),
        &layla_auth,
        session.id,
        &[(e.id, Mark::Absent)],
        false,
    )
    .await
    .expect("the assigned tutor marks");
    let wrong = edtech::corrections::correct_attendance(
        w.state(),
        &omar_auth,
        session.id,
        &[(e.id, Mark::Present)],
        "not mine",
    )
    .await;
    assert!(matches!(wrong, Err(Error::Forbidden)), "{wrong:?}");
    assert_eq!(mark_row(&w, &session, &e).await.mark, Mark::Absent);
    edtech::corrections::correct_attendance(
        w.state(),
        &layla_auth,
        session.id,
        &[(e.id, Mark::Present)],
        "she came",
    )
    .await
    .expect("the assigned tutor corrects");
    edtech::corrections::correct_attendance(
        w.state(),
        &coordinator,
        session.id,
        &[(e.id, Mark::Late)],
        "arrived late",
    )
    .await
    .expect("the coordinator corrects any session");
    assert_eq!(mark_row(&w, &session, &e).await.mark, Mark::Late);
}

// ---- #452: booking a one-to-one with a tutor, against the real wl-documents ----------------------

/// #452 point 3, run with the real wl-documents and the host's service types:
/// a one-to-one booking with a tutor who has no police clearance on record is
/// refused naming the clearance; once the clearance is accepted for a date that
/// covers the lesson the same booking is made. (The clearance is asked of every
/// tutor, not only for a minor: the eligibility query names no student, see the
/// deferred line.)
#[renox::test]
async fn edu_17_a_one_to_one_booking_needs_the_tutors_clearance_in_the_real_guard() {
    use wl_scheduling::{AvailabilityRule, ServiceType, resource, service};
    let w = boot().await;
    let (layla, employee) = employed(&w, "Layla Hassan").await;
    let service_type = ServiceType::query()
        .where_(ServiceType::CODE.eq(edtech::seed::ONE_TO_ONE_SERVICE))
        .first(w.db())
        .await
        .unwrap()
        .expect("the host seeds the one-to-one service type");
    resource::offer(w.db(), service_type.id, layla.id)
        .await
        .unwrap();
    for weekday in 1..=7 {
        AvailabilityRule::new(layla.id, weekday, 0, 1440)
            .save(w.db())
            .await
            .unwrap();
    }
    let parent = tenant(&w.app, "Huda Khan").await;
    let lesson = day(8);
    let book = |w: &World| {
        let state = w.state().clone();
        let (service_id, tutor_id) = (service_type.id, layla.id);
        async move {
            match service::book(
                &state,
                wl_scheduling::NewBooking::new(
                    parent,
                    service_id,
                    &[tutor_id],
                    local(lesson, 10, 0),
                ),
                wl_scheduling::Actor::staff(1),
                renox::db::now(),
            )
            .await
            {
                Ok(_) => "booked".to_owned(),
                Err(refusal) => format!("{refusal:?}"),
            }
        }
    };

    let refused = book(&w).await;
    assert_ne!(refused, "booked");
    assert!(
        refused.to_lowercase().contains("police clearance"),
        "{refused}"
    );

    record_document(
        &w.app,
        "police_clearance",
        wl_kernel::workforce::EmployeeId(employee).subject(),
        day(365),
    )
    .await;
    assert_eq!(book(&w).await, "booked");
}
