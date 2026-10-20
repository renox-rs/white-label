//! Booting wl-delivery in a `TestApp`, as `crates/wl-ticketing/tests/support`
//! boots wl-ticketing: a document guard of its own that answers
//! `document_valid` from a table the test fills (the crate does not depend on
//! wl-documents), a workforce that maps users to employees and knows who is
//! on leave (it does not depend on wl-hr), and the kernel events recorded.
//! The tests act through the real routes as users who hold the permissions
//! of the roles a host maps.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::auth::{Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::{TestApp, TestResponse};
use wl_delivery::{
    Deliveries, Delivery, DeliveryLine, NewRun, Run, VehicleType, WlDelivery,
    permissions as perm, seed,
};
use wl_kernel::documents::{DocumentGuard, today};
use wl_kernel::workforce::Workforce;
use wl_kernel::{
    ClientConfig, DeliveryAssigned, DeliveryAttemptFailed, DeliveryDelivered, DeliveryItem,
    DeliveryDisputeClosed, DeliveryDisputed, DeliveryFound, DeliveryLost, DeliveryOutForDelivery,
    DeliveryRefusedReceived, DeliveryRequest, PodMethod, RunClosed, RunDispatched, Subject,
};
use wl_kernel::tickets::TaskCreator;

pub mod documents;
pub mod roster;
pub mod tickets;

use documents::Documents;
use roster::{Roster, employee_of};
use tickets::TicketDesk;

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A client with the delivery module (and the parties it needs) on.
pub const CLIENT: &str = r#"
[client]
name = "Gulf Express Logistics LLC"
app = "logistics"

[modules]
enabled = ["parties", "delivery"]

[[legal_entities]]
code = "GEL"
name_en = "Gulf Express Logistics LLC"
trn = "100000000000003"
default = true
"#;

/// The vehicle (asset) the runs are driven with.
pub const VAN: i64 = 7;

/// Everything of one type `E` emitted so far.
#[derive(Clone)]
pub struct Recorded<E>(Arc<Mutex<Vec<E>>>);

impl<E: Clone> Recorded<E> {
    /// Nothing recorded yet.
    pub fn new() -> Self {
        Recorded(Arc::new(Mutex::new(Vec::new())))
    }

    /// The events, oldest first.
    pub fn all(&self) -> Vec<E> {
        self.0.lock().unwrap().clone()
    }
}

/// `app` listening for `E`, recorded into `into`.
pub fn record<E: Event + Clone>(app: App, into: &Recorded<E>) -> App {
    let into = into.clone();
    app.listen(move |event: E, _state: AppState| {
        let into = into.clone();
        async move {
            into.0.lock().unwrap().push(event);
            Ok(())
        }
    })
}

/// `2026-11-12` as a date.
pub fn date(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

/// The app under test with what it emitted.
pub struct World {
    /// The app.
    pub app: TestApp,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
    /// The document guard.
    pub docs: Documents,
    /// Who is on leave.
    pub roster: Roster,
    /// `DeliveryAssigned`.
    pub assigned: Recorded<DeliveryAssigned>,
    /// `DeliveryOutForDelivery`.
    pub out: Recorded<DeliveryOutForDelivery>,
    /// `DeliveryDelivered`.
    pub delivered: Recorded<DeliveryDelivered>,
    /// `DeliveryAttemptFailed`.
    pub failed: Recorded<DeliveryAttemptFailed>,
    /// `RunDispatched`.
    pub dispatched: Recorded<RunDispatched>,
    /// `DeliveryDisputed`.
    pub disputed: Recorded<DeliveryDisputed>,
    /// `DeliveryDisputeClosed`.
    pub dispute_closed: Recorded<DeliveryDisputeClosed>,
    /// `DeliveryLost`.
    pub lost: Recorded<DeliveryLost>,
    /// `DeliveryFound`.
    pub found: Recorded<DeliveryFound>,
    /// `DeliveryRefusedReceived`.
    pub refused_received: Recorded<DeliveryRefusedReceived>,
    /// `RunClosed`.
    pub run_closed: Recorded<RunClosed>,
    /// The tickets the module opened, resolved and commented on.
    pub tickets: TicketDesk,
    /// Asks for a loss: holds `deliveries.reverse` and manages deliveries.
    pub ops_a: User,
    /// Approves a loss: holds `deliveries.reverse` and manages deliveries.
    pub ops_b: User,
    /// Dispatches and assigns deliveries.
    pub dispatcher: User,
    /// Also manages deliveries.
    pub manager: User,
    /// Drives the runs of the tests.
    pub driver: User,
    /// Another driver.
    pub other_driver: User,
}

/// The next source id, so every delivery has its own sales order.
static SOURCE: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);

impl World {
    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// The app state.
    pub fn state(&self) -> &AppState {
        self.app.state()
    }

    /// A user holding exactly `grants` (through a role of their own).
    async fn user(&self, name: &str, grants: &[&str]) -> User {
        make_user(self.db(), name, grants).await
    }

    /// The employee subject of the driver of the tests (the document
    /// subject of a licence).
    pub fn driver_subject(&self) -> Subject {
        employee_of(self.driver.id).subject()
    }

    /// The vehicle subject (the document subject of an insurance).
    pub fn vehicle_subject(&self) -> Subject {
        Subject::new("inventory.asset", VAN)
    }

    /// Every document of the driver and the van is valid through `until`.
    pub fn valid_documents(&self, until: &str) {
        let until = Some(date(until));
        self.docs
            .set(&self.driver_subject(), documents::LICENCE, until);
        self.docs
            .set(&self.driver_subject(), documents::RIDER_PERMIT, until);
        self.docs
            .set(&self.vehicle_subject(), documents::MULKIYA, until);
        self.docs
            .set(&self.vehicle_subject(), documents::INSURANCE, until);
    }

    /// Every document of the user `user_id` as a driver and of the van is
    /// valid through `until`.
    pub fn valid_documents_for(&self, user_id: i64, until: &str) {
        let until = Some(date(until));
        let subject = employee_of(user_id).subject();
        self.docs.set(&subject, documents::LICENCE, until);
        self.docs.set(&subject, documents::RIDER_PERMIT, until);
        self.docs
            .set(&self.vehicle_subject(), documents::MULKIYA, until);
        self.docs
            .set(&self.vehicle_subject(), documents::INSURANCE, until);
    }

    /// Requests a delivery of `items` (`description`, quantity in
    /// thousandths, amount for the whole line) from a new sales order, with
    /// `collect` fils to collect.
    pub async fn request(&self, items: &[(&str, i64, Option<i64>)], collect: i64) -> Delivery {
        self.request_from("sales.order", items, collect).await
    }

    /// [`request`](Self::request) for a source of `source_type`.
    pub async fn request_from(
        &self,
        source_type: &str,
        items: &[(&str, i64, Option<i64>)],
        collect: i64,
    ) -> Delivery {
        let source = SOURCE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut request = DeliveryRequest::new(Subject::new(source_type, source), "Mariam Al Hosani")
            .address("Villa 12, Al Barsha 2", "Al Barsha")
            .phone("+971500000000")
            .collect(collect);
        for (description, quantity, amount) in items {
            let mut item = DeliveryItem::new(description, *quantity);
            if let Some(amount) = amount {
                item = item.amount(*amount);
            }
            request = request.item(item);
        }
        Deliveries::request(self.state(), request)
            .await
            .expect("the delivery is requested")
    }

    /// A delivery of one tile line, nothing to collect.
    pub async fn simple_delivery(&self) -> Delivery {
        self.request(&[("Tiles", 10_000, None)], 0).await
    }

    /// A planned run of the driver on `day`, with the van.
    pub async fn run_on(&self, day: &str) -> Run {
        wl_delivery::plan::create_run(
            self.state(),
            NewRun::new(date(day), self.driver.id).vehicle(VehicleType::Van, Some(VAN)),
        )
        .await
        .expect("the run is planned")
    }

    /// A planned run of the driver on `day`, on foot.
    pub async fn on_foot_run_on(&self, day: &str) -> Run {
        wl_delivery::plan::create_run(
            self.state(),
            NewRun::new(date(day), self.driver.id).vehicle(VehicleType::OnFoot, None),
        )
        .await
        .expect("the run is planned")
    }

    /// The delivery `id`, as stored now.
    pub async fn delivery(&self, id: i64) -> Delivery {
        Delivery::find_or_404(self.db(), id).await.unwrap()
    }

    /// `delivery`, as stored now.
    pub async fn reload(&self, delivery: &Delivery) -> Delivery {
        self.delivery(delivery.id).await
    }

    /// `run`, as stored now.
    pub async fn reload_run(&self, run: &Run) -> Run {
        Run::find_or_404(self.db(), run.id).await.unwrap()
    }

    /// The lines of `delivery`, as stored now.
    pub async fn lines(&self, delivery: &Delivery) -> Vec<DeliveryLine> {
        delivery.lines(self.db()).await.unwrap()
    }

    /// `by` puts `delivery` on `run`.
    pub async fn assign(&self, by: &User, delivery: &Delivery, run: &Run) -> TestResponse {
        self.app.acting_as(by);
        let run_id = run.id.to_string();
        self.app
            .htmx()
            .post(
                &format!("/deliveries/{}/assign", delivery.id),
                &[("run_id", run_id.as_str())],
            )
            .await
    }

    /// `by` dispatches `run`.
    pub async fn dispatch(&self, by: &User, run: &Run) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(&format!("/runs/{}/dispatch", run.id), &[])
            .await
    }

    /// A delivery on a run of today that has left: the documents are made
    /// valid, the delivery is assigned and the run dispatched.
    pub async fn out_for_delivery(&self, delivery: &Delivery) -> Delivery {
        self.valid_documents("2099-12-31");
        let run = self.run_on(&today().format("%Y-%m-%d").to_string()).await;
        self.assign(&self.dispatcher, delivery, &run)
            .await
            .assert_ok();
        self.dispatch(&self.dispatcher, &run).await.assert_ok();
        let after = self.reload(delivery).await;
        assert_eq!(after.status.as_str(), "out_for_delivery");
        after
    }

    /// `by` posts the outcome *delivered* of `delivery` with `pairs`.
    pub async fn deliver(
        &self,
        by: &User,
        delivery: &Delivery,
        pairs: &[(String, String)],
    ) -> TestResponse {
        self.app.acting_as(by);
        let fields: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        self.app
            .htmx()
            .post(&format!("/driver/stops/{}/deliver", delivery.id), &fields)
            .await
    }

    /// A delivery of one tile line with `collect` fils to collect, delivered
    /// by the driver with the cash collected in full.
    pub async fn delivered_cod(&self, collect: i64) -> Delivery {
        let delivery = self.request(&[("Tiles", 10_000, None)], collect).await;
        let out = self.out_for_delivery(&delivery).await;
        let line = self.lines(&out).await.remove(0);
        let cash = format!("{}.{:02}", collect / 100, collect % 100);
        let fields = stop_fields(&[(line.id, "10", "0", "", "")], &cash);
        self.deliver(&self.driver, &out, &fields).await.assert_ok();
        let after = self.reload(&out).await;
        assert_eq!(after.status.as_str(), "delivered");
        after
    }

    /// `by` posts `fields` to `path`.
    pub async fn post(&self, by: &User, path: &str, fields: &[(&str, &str)]) -> TestResponse {
        self.app.acting_as(by);
        self.app.htmx().post(path, fields).await
    }

    /// `by` puts `delivery` under review for `reason`.
    pub async fn open_review(&self, by: &User, delivery: &Delivery, reason: &str) -> TestResponse {
        self.post(
            by,
            &format!("/deliveries/{}/open-review", delivery.id),
            &[("reason", reason)],
        )
        .await
    }

    /// `by` closes the review of `delivery` as upheld with `note`.
    pub async fn close_review(&self, by: &User, delivery: &Delivery, note: &str) -> TestResponse {
        self.post(
            by,
            &format!("/deliveries/{}/close-review", delivery.id),
            &[("outcome", "upheld"), ("note", note)],
        )
        .await
    }

    /// `by` posts a failed visit of `delivery` for `reason`.
    pub async fn fail(&self, by: &User, delivery: &Delivery, reason: &str) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(
                &format!("/driver/stops/{}/fail", delivery.id),
                &[
                    ("reason", reason),
                    ("latitude", "25.204849"),
                    ("longitude", "55.270783"),
                ],
            )
            .await
    }
}

/// The fields of a delivered stop: the proof (photo and signature, the
/// default setting), the location and, per line, the quantities
/// `(line id, delivered, refused, reason, photo)` as the driver types them.
pub fn stop_fields(
    lines: &[(i64, &str, &str, &str, &str)],
    cash: &str,
) -> Vec<(String, String)> {
    let mut fields: Vec<(String, String)> = vec![
        ("recipient_name".into(), "Mariam Al Hosani".into()),
        ("photo".into(), "photo-0001".into()),
        ("signature".into(), "signature-0001".into()),
        ("latitude".into(), "25.204849".into()),
        ("longitude".into(), "55.270783".into()),
        ("cash_collected".into(), cash.into()),
    ];
    for (id, delivered, refused, reason, photo) in lines {
        fields.push((format!("delivered_{id}"), (*delivered).into()));
        fields.push((format!("refused_{id}"), (*refused).into()));
        if !reason.is_empty() {
            fields.push((format!("reason_{id}"), (*reason).into()));
        }
        if !photo.is_empty() {
            fields.push((format!("photo_{id}"), (*photo).into()));
        }
    }
    fields
}

/// `fields` without the field `name`.
pub fn without(fields: Vec<(String, String)>, name: &str) -> Vec<(String, String)> {
    fields.into_iter().filter(|(k, _)| k != name).collect()
}

async fn make_user(db: &Db, name: &str, grants: &[&str]) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    let user = User::register(db, name, &email, "password123")
        .await
        .unwrap();
    let role = format!("role_{}", name.to_lowercase());
    permissions::define_role(db, &role, grants).await.unwrap();
    user.assign_role(db, &role).await.unwrap();
    user
}

/// wl-delivery booted with the 03 document blocks, seeded.
pub async fn boot() -> World {
    boot_with(WlDelivery::new(), Documents::logistics()).await
}

/// `module` booted with the document guard `docs`.
pub async fn boot_with(module: WlDelivery, docs: Documents) -> World {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();

    let assigned = Recorded::new();
    let out = Recorded::new();
    let delivered = Recorded::new();
    let failed = Recorded::new();
    let dispatched = Recorded::new();
    let disputed = Recorded::new();
    let dispute_closed = Recorded::new();
    let lost = Recorded::new();
    let found = Recorded::new();
    let refused_received = Recorded::new();
    let run_closed = Recorded::new();
    let tickets = TicketDesk::default();
    let roster = Roster::default();
    let guard: Arc<dyn DocumentGuard> = Arc::new(docs.clone());
    let workforce: Arc<dyn Workforce> = Arc::new(roster.clone());
    let creator: Arc<dyn TaskCreator> = Arc::new(tickets.clone());
    let app = App::new()
        .module(Auth::new())
        .module(renox::audit::Audit)
        .module(Permissions)
        .module(module)
        .provide(guard)
        .provide(workforce)
        .provide(creator);
    let app = record(app, &assigned);
    let app = record(app, &out);
    let app = record(app, &delivered);
    let app = record(app, &failed);
    let app = record(app, &dispatched);
    let app = record(app, &disputed);
    let app = record(app, &dispute_closed);
    let app = record(app, &lost);
    let app = record(app, &found);
    let app = record(app, &refused_received);
    let app = record(app, &run_closed);
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    seed::install(app.db()).await.expect("the defaults seed");

    let dispatcher = make_user(
        app.db(),
        "Dina",
        &[
            perm::VIEW,
            perm::DISPATCH,
            perm::RUNS_VIEW,
            perm::RUNS_DISPATCH,
            perm::POD_VIEW,
        ],
    )
    .await;
    let manager = make_user(
        app.db(),
        "Mona",
        &[
            perm::VIEW,
            perm::DISPATCH,
            perm::MANAGE,
            perm::RUNS_VIEW,
            perm::RUNS_DISPATCH,
            perm::POD_VIEW,
        ],
    )
    .await;
    let ops_grants = [
        perm::VIEW,
        perm::MANAGE,
        perm::REVERSE,
        perm::RUNS_VIEW,
        perm::POD_VIEW,
    ];
    let ops_a = make_user(app.db(), "Olivia", &ops_grants).await;
    let ops_b = make_user(app.db(), "Basel", &ops_grants).await;
    let driver = make_user(app.db(), "Dev", &[perm::RUNS_DRIVE, perm::COMPLETE]).await;
    let other_driver = make_user(app.db(), "Omar", &[perm::RUNS_DRIVE, perm::COMPLETE]).await;
    World {
        app,
        _views: views,
        docs,
        roster,
        assigned,
        out,
        delivered,
        failed,
        dispatched,
        disputed,
        dispute_closed,
        lost,
        found,
        refused_received,
        run_closed,
        tickets,
        ops_a,
        ops_b,
        dispatcher,
        manager,
        driver,
        other_driver,
    }
}

/// The default proof methods, for assertions.
pub const DEFAULT_PROOF: [PodMethod; 2] = [PodMethod::Photo, PodMethod::Signature];

/// A chief driver: drives runs and also holds `deliveries.reverse`, to show
/// that the driver of a run still cannot request its loss.
pub async fn chief_driver(w: &World) -> User {
    make_user(
        w.db(),
        "Chief",
        &[perm::RUNS_DRIVE, perm::COMPLETE, perm::REVERSE, perm::MANAGE, perm::VIEW],
    )
    .await
}

/// A user holding exactly `grants`.
pub async fn user_with(w: &World, name: &str, grants: &[&str]) -> User {
    make_user(w.db(), name, grants).await
}
