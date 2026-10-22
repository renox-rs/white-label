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
