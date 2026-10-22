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
use support::{day, noon_clock, person, tenant};
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
    let gateway = FakeGateway::new();
    let who = Arc::new(Mutex::new(None));
    let config = wl_kernel::ClientConfig::load(CLIENT).expect("the edtech client");
    let tuned = {
        let gateway = gateway.clone();
        host::Tuning::default().payments(move |p| p.gateway(gateway))
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
            &[("accepted_at", &Some(renox::db::now() - TimeDelta::days(400)))],
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
    wl_kernel::BusinessCalendar::default().from_local(
        date.and_hms_opt(hour, minute, 0).expect("a time"),
    )
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
    assert_eq!(credits_left(&w, party).await, 9, "an excused absence is free");
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

    let done = edtech::sessions::cancel(w.state(), &c.coordinator, cancelled.id, "Tutor sick", None)
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
    assert_eq!(credits_left(&w, party).await, 9, "the flag changes no credit");
}
