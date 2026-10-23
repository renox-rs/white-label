//! The screens of the EdTech app walked through the real host (the second
//! manual pass): what a user of each role does on the pages, against the real
//! wl-payments, wl-invoicing and wl-sales.

mod support;

use std::sync::{Arc, Mutex};

use edtech::batches::{self, NewBatch};
use edtech::courses::{self, NewCourse};
use edtech::enrolments::{self, NewEnrolment};
use edtech::{Batch, Course, Enrolment, EnrolmentStatus};
use host::roles::{ACADEMIC_COORDINATOR, COUNSELLOR};
use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use renox::testing::TestApp;
use support::{day, noon_clock, person, tenant};

/// The demo training centre.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-edtech/client.toml"
);

/// A test-only module with the route that hands a test the signed-in user the
/// services take.
struct Who(Arc<Mutex<Option<AuthUser>>>);

impl Module for Who {
    fn name(&self) -> &'static str {
        "edtech-walk-who"
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
    who: Arc<Mutex<Option<AuthUser>>>,
}

async fn boot() -> World {
    let who = Arc::new(Mutex::new(None));
    let config = wl_kernel::ClientConfig::load(CLIENT).expect("the edtech client");
    let app = TestApp::new(
        host::app_for_tuned(config, host::Tuning::default())
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
    World { app, who }
}

impl World {
    fn state(&self) -> &AppState {
        self.app.state()
    }

    fn db(&self) -> &Db {
        self.app.db()
    }

    /// A user with the host role `role`, and as the signed-in user the
    /// services take.
    async fn staff(&self, name: &str, role: &str) -> (User, AuthUser) {
        let user = person(&self.app, name, role).await;
        self.app.acting_as(&user);
        self.app.get("/t/whoami").await.assert_ok();
        let auth = self.who.lock().unwrap().take().expect("the probe saw them");
        (user, auth)
    }

    /// An open batch of the course IELTS-B2 from the first Monday on or after
    /// `from`.
    async fn open_batch(&self, coordinator: &AuthUser, from: NaiveDate) -> Batch {
        let entity = wl_kernel::LegalEntity::default_entity(self.db())
            .await
            .unwrap();
        let kind = wl_documents::DocumentKind::by_key(self.db(), "centre_permit")
            .await
            .unwrap()
            .unwrap();
        wl_documents::Document::create(
            self.db(),
            wl_documents::Document {
                subject_type: "kernel.legal_entity".into(),
                subject_id: entity.id,
                kind_id: kind.id,
                title: kind.key.clone(),
                expiry_date: Some(day(400)),
                current_version: 1,
                status: wl_documents::DocumentStatus::Accepted,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mut tutor =
            wl_scheduling::Resource::new("Tutor W", wl_scheduling::ResourceType::Person);
        tutor.save(self.db()).await.unwrap();
        let mut room = wl_scheduling::Resource::new("Room W", wl_scheduling::ResourceType::Room);
        room.capacity = 14;
        room.save(self.db()).await.unwrap();
        let course: Course = courses::create(
            self.state(),
            coordinator,
            NewCourse::new("IELTS-B2", "IELTS preparation", "English language", 40).level("B2"),
        )
        .await
        .expect("the course is defined");
        let mut first = from;
        while renox::chrono::Datelike::weekday(&first).number_from_monday() != 1 {
            first += TimeDelta::days(1);
        }
        let planned = batches::create(
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
        .expect("the batch is planned");
        batches::open(self.state(), coordinator, planned.id)
            .await
            .expect("the batch opens")
    }
}

async fn reload(w: &World, e: &Enrolment) -> Enrolment {
    Enrolment::find_or_404(w.db(), e.id).await.unwrap()
}

/// The desk takes the fee from the enrolment's page: a cash payment of the
/// whole first invoice is recorded and allocated, and activates the enrolment;
/// a transfer without its reference and an amount above what is due are refused
/// in words, with nothing recorded.
#[renox::test]
async fn the_desk_records_the_first_payment_from_the_enrolment_page() {
    let w = boot().await;
    let (_, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let (dina_user, dina) = w.staff("Dina", COUNSELLOR).await;
    let batch = w.open_batch(&coordinator, day(7)).await;
    let sara = tenant(&w.app, "Sara Ali").await;
    let e = enrolments::create(
        w.state(),
        &dina,
        NewEnrolment::course(sara.0, batch.id, 252_000).terms_accepted(),
    )
    .await
    .unwrap();
    assert_eq!(e.status, EnrolmentStatus::PendingPayment);
    let url = format!("/edtech/enrolments/{}/payment", e.id);

    w.app.acting_as(&dina_user);
    w.app
        .get(&format!("/edtech/enrolments/{}", e.id))
        .await
        .assert_ok()
        .assert_see("Record a payment")
        .assert_see("AED 2,520.00");
    // Without the reference of a transfer.
    let refused = w
        .app
        .htmx()
        .post(&url, &[("method", "transfer"), ("amount", "2520")])
        .await;
    refused
        .assert_status(422)
        .assert_see("A bank transfer needs its reference.");
    // More than is due.
    w.app
        .htmx()
        .post(&url, &[("method", "cash"), ("amount", "3000")])
        .await
        .assert_status(422)
        .assert_see("more than what is due, AED 2,520.00");
    assert_eq!(reload(&w, &e).await.status, EnrolmentStatus::PendingPayment);

    w.app
        .htmx()
        .post(&url, &[("method", "cash"), ("amount", "2520")])
        .await
        .assert_ok();
    assert_eq!(reload(&w, &e).await.status, EnrolmentStatus::Active);
    let payments = wl_payments::Payment::query()
        .where_(wl_payments::Payment::PARTY_ID.eq(sara.0))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].amount, 252_000);
    let summary = wl_kernel::payments::payment_recorder(w.state())
        .find(w.db(), wl_kernel::payments::PaymentId(payments[0].id))
        .await
        .unwrap()
        .expect("the payment");
    assert_eq!(
        summary.unallocated, 0,
        "all of it is set against the invoice"
    );
    // Nothing is due any more.
    w.app
        .htmx()
        .post(&url, &[("method", "cash"), ("amount", "10")])
        .await
        .assert_status(422)
        .assert_see("Nothing is due on this enrolment.");
}

/// The centre manager reads the payments and refunds and decides a refund
/// (10 §3.1), and sees the campaigns and their figures; the counsellor has no
/// campaigns (they are marketing's) and the tutor neither; a refusal is a
/// clear page, not a bare status.
#[renox::test]
async fn the_roles_reach_the_money_and_marketing_pages_the_spec_gives_them() {
    use host::roles::{ACCOUNTANT, CENTRE_MANAGER, TUTOR};
    let w = boot().await;
    let (manager, _) = w.staff("Mona", CENTRE_MANAGER).await;
    let (counsellor, _) = w.staff("Dina", COUNSELLOR).await;
    let (tutor, _) = w.staff("Tariq", TUTOR).await;
    let (accountant, _) = w.staff("Faisal", ACCOUNTANT).await;

    w.app.acting_as(&manager);
    for url in ["/refunds", "/payments", "/campaigns"] {
        w.app.get(url).await.assert_ok();
    }
    w.app.acting_as(&counsellor);
    w.app.get("/refunds").await.assert_ok();
    w.app
        .get("/campaigns")
        .await
        .assert_status(403)
        .assert_see("You do not have permission for this action.");
    w.app.acting_as(&tutor);
    w.app.get("/refunds").await.assert_status(403);
    w.app.get("/campaigns").await.assert_status(403);
    w.app.acting_as(&accountant);
    w.app.get("/refunds").await.assert_ok();
}

/// The coordinator plans a batch from the page by picking the tutor and the
/// room by name, not by resource number; the tutor's employee and the room's
/// limit follow from the resource.
#[renox::test]
async fn the_coordinator_plans_a_batch_picking_the_tutor_and_the_room_by_name() {
    let w = boot().await;
    let (coordinator_user, coordinator) = w.staff("Amal", ACADEMIC_COORDINATOR).await;
    let mut tutor =
        wl_scheduling::Resource::new("Layla Hassan", wl_scheduling::ResourceType::Person);
    tutor.save(w.db()).await.unwrap();
    let mut room = wl_scheduling::Resource::new("Kids studio", wl_scheduling::ResourceType::Room);
    room.capacity = 12;
    room.save(w.db()).await.unwrap();
    let course = courses::create(
        w.state(),
        &coordinator,
        NewCourse::new("ENG-A2", "General English A2", "Language", 48),
    )
    .await
    .unwrap();

    w.app.acting_as(&coordinator_user);
    w.app
        .get("/edtech/batches")
        .await
        .assert_ok()
        .assert_see("Layla Hassan")
        .assert_see("Kids studio");
    let start = day(30);
    let end = day(60);
    w.app
        .htmx()
        .post(
            "/edtech/batches",
            &[
                ("course", course.id.to_string().as_str()),
                ("capacity", "10"),
                ("start_date", start.to_string().as_str()),
                ("end_date", end.to_string().as_str()),
                ("weekdays", "1,3"),
                ("start_time", "18:00"),
                ("duration", "90"),
                ("tutor", tutor.id.to_string().as_str()),
                ("room", room.id.to_string().as_str()),
            ],
        )
        .await
        .assert_ok();
    let batch = Batch::query()
        .first(w.db())
        .await
        .unwrap()
        .expect("a batch");
    assert_eq!(batch.tutor_resource_id, Some(tutor.id));
    assert_eq!(batch.room_resource_id, Some(room.id));
    assert_eq!(
        batch.room_limit,
        Some(12),
        "the room's capacity is its limit"
    );
}

/// The dashboard of an EdTech client shows the figures of the centre to the
/// roles whose pages list them: the manager the sessions, the enrolments, the
/// batches and the certificates; the tutor the sessions only.
#[renox::test]
async fn the_dashboard_shows_the_figures_of_the_centre() {
    use host::roles::{CENTRE_MANAGER, TUTOR};
    let w = boot().await;
    let (manager, _) = w.staff("Mona", CENTRE_MANAGER).await;
    let (tutor, _) = w.staff("Tariq", TUTOR).await;
    w.app.acting_as(&manager);
    w.app
        .get("/dashboard")
        .await
        .assert_ok()
        .assert_see("Sessions today")
        .assert_see("Enrolments waiting for payment")
        .assert_see("Batches open for enrolment")
        .assert_see("Certificates to issue")
        .assert_dont_see("Units available");
    w.app.acting_as(&tutor);
    w.app
        .get("/dashboard")
        .await
        .assert_ok()
        .assert_see("Sessions today")
        .assert_dont_see("Enrolments waiting for payment")
        .assert_dont_see("Certificates to issue");
}

/// The enrolment form refuses in sentences: a package without its sessions, an
/// amount that is not a number.
#[renox::test]
async fn the_enrolment_form_refuses_in_words() {
    let w = boot().await;
    let (dina_user, _) = w.staff("Dina", COUNSELLOR).await;
    let sara = tenant(&w.app, "Sara Ali").await;
    w.app.acting_as(&dina_user);
    let student = sara.0.to_string();
    w.app
        .htmx()
        .post(
            "/edtech/enrolments",
            &[
                ("kind", "package"),
                ("student", student.as_str()),
                ("item_code", "PT-8"),
                ("price", "2400"),
            ],
        )
        .await
        .assert_status(422)
        .assert_see("at least one session");
    w.app
        .htmx()
        .post(
            "/edtech/enrolments",
            &[
                ("kind", "package"),
                ("student", student.as_str()),
                ("price", "abc"),
            ],
        )
        .await
        .assert_status(422)
        .assert_see("Give the amount in dirhams");
    w.app
        .htmx()
        .post("/edtech/enrolments", &[("price", "100")])
        .await
        .assert_status(422)
        .assert_see("Pick the person from the list");
}
