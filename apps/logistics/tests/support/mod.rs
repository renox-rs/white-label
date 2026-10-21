//! Booting the Logistics app in a `TestApp`: the real wl-delivery (shipments
//! are the source of their deliveries), a recording notifier, a portal that
//! knows which client company each client user acts for, and a workforce
//! that maps users to employees. The tests act through the real routes as
//! users who hold the permissions of the roles the host maps.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use logistics::permissions as perm;
use logistics::remit::{self, Built};
use logistics::{Logistics, Remittance, RemittanceStatus, Shipment, ShipmentStatus};
use renox::auth::{Permissions, permissions};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::{TestApp, TestResponse};
use wl_delivery::{Delivery, NewRun, VehicleType, WlDelivery, permissions as dperm, seed};
use wl_kernel::documents::today;
use wl_kernel::notify::Notifier;
use wl_kernel::parties::PartyDirectory;
use wl_kernel::portal::PortalUsers;
use wl_kernel::tickets::TaskCreator;
use wl_kernel::workforce::Workforce;
use wl_kernel::{ClientConfig, PartyId, RecordVoided};

pub mod notices;
pub mod parties;
pub mod portal;
pub mod roster;
pub mod tickets;

use notices::Notices;
use parties::Parties;
use portal::Portal;
use roster::Roster;
use tickets::TicketDesk;

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A logistics client with the delivery module on.
pub const CLIENT: &str = r#"
[client]
name = "Najm Express Cargo LLC"
app = "logistics"

[modules]
enabled = ["parties", "delivery"]

[[legal_entities]]
code = "NEC"
name_en = "Najm Express Cargo LLC"
trn = "100000000000003"
default = true
"#;

/// The vehicle (asset) the runs are driven with.
pub const VAN: i64 = 7;

/// The client "Souq Express" of the tests.
pub const SOUQ: PartyId = PartyId(501);

/// Another client.
pub const OTHER_CLIENT: PartyId = PartyId(502);

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

/// The app under test.
pub struct World {
    /// The app.
    pub app: TestApp,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
    /// The notices asked of the kernel's notifier.
    pub notices: Notices,
    /// The tickets the module opened.
    pub tickets: TicketDesk,
    /// `RecordVoided`.
    pub voided: Recorded<RecordVoided>,
    /// The operations manager.
    pub ops: User,
    /// The dispatcher: manages shipments and plans runs.
    pub dispatcher: User,
    /// A second dispatcher.
    pub dispatcher_b: User,
    /// Hub staff.
    pub hub: User,
    /// The driver of the runs.
    pub driver: User,
    /// A client user of Souq Express: books the shipments of the tests.
    pub noor: User,
    /// Another client user of Souq Express.
    pub nadia: User,
    /// A client user of another client.
    pub farah: User,
    /// An accountant: prepares, approves and pays COD remittances.
    pub amal: User,
    /// Another accountant.
    pub bashar: User,
    /// The party directory (the real wl-payments knows the clients through
    /// it); blocking a client is done here.
    pub parties: Parties,
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

/// The transitions the signed-in user is offered on a shipment, as the
/// shipment page would draw them (`GET /probe/{id}/available`).
async fn probe_available(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<String> {
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    let names: Vec<String> = shipment
        .available(&user, &state.db)
        .await?
        .into_iter()
        .map(|action| action.name)
        .collect();
    Ok(names.join(","))
}

/// Mounts [`probe_available`].
struct Probe;

impl Module for Probe {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/probe/{id}/available", probe_available)
            .require_auth()
    }
}

/// The app booted with the real wl-delivery.
pub async fn boot() -> World {
    boot_with(false).await
}

/// The app booted with the real wl-delivery and the real wl-payments (the
/// payout of a COD remittance is a voucher there).
pub async fn boot_with_payments() -> World {
    boot_with(true).await
}

async fn boot_with(real_payments: bool) -> World {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();

    let notices = Notices::default();
    let notifier: Arc<dyn Notifier> = Arc::new(notices.clone());
    let roster = Roster::default();
    let workforce: Arc<dyn Workforce> = Arc::new(roster);
    let portal = Portal::default();
    let portal_users: Arc<dyn PortalUsers> = Arc::new(portal.clone());
    let tickets = TicketDesk::default();
    let task_creator: Arc<dyn TaskCreator> = Arc::new(tickets.clone());
    let voided = Recorded::new();
    let parties = Parties::default();
    let app = App::new()
        .module(Auth::new())
        .module(renox::audit::Audit)
        .module(Permissions)
        .module(WlDelivery::new().partial_acceptance(logistics::SUBJECT_TYPE, false))
        .module(Logistics)
        .module(Probe)
        .provide(workforce)
        .provide(notifier)
        .provide(portal_users)
        .provide(task_creator);
    let app = if real_payments {
        let directory: Arc<dyn PartyDirectory> = Arc::new(parties.clone());
        app.provide(directory)
            .module(wl_payments::WlPayments::new())
    } else {
        app
    };
    let app = record(app, &voided);
    let client = if real_payments {
        CLIENT.replace(
            "[\"parties\", \"delivery\"]",
            "[\"parties\", \"invoicing\", \"delivery\", \"payments\"]",
        )
    } else {
        CLIENT.to_owned()
    };
    let config = ClientConfig::parse(&client).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    seed::install(app.db()).await.expect("the defaults seed");

    let manage = [
        perm::SHIPMENTS_VIEW,
        perm::SHIPMENTS_BOOK,
        perm::SHIPMENTS_MANAGE,
        perm::SHIPMENTS_VOID,
        perm::SHIPMENTS_CANCEL,
    ];
    let planning = [
        dperm::VIEW,
        dperm::DISPATCH,
        dperm::MANAGE,
        dperm::RUNS_VIEW,
        dperm::RUNS_DISPATCH,
        dperm::POD_VIEW,
        dperm::APPROVE_COD_DIFFERENCE,
        perm::REMITTANCES_VIEW,
    ];
    let dispatcher_grants: Vec<&str> = manage.iter().chain(planning.iter()).copied().collect();
    let ops = make_user(app.db(), "Olivia", &dispatcher_grants).await;
    let dispatcher = make_user(app.db(), "Dina", &dispatcher_grants).await;
    let dispatcher_b = make_user(app.db(), "Basel", &dispatcher_grants).await;
    let hub = make_user(
        app.db(),
        "Hana",
        &[perm::SHIPMENTS_VIEW, perm::SHIPMENTS_SCAN],
    )
    .await;
    let driver = make_user(
        app.db(),
        "Dev",
        &[
            dperm::RUNS_DRIVE,
            dperm::COMPLETE,
            perm::SHIPMENTS_SCAN,
            perm::SHIPMENTS_DELIVER,
        ],
    )
    .await;
    let client_grants = [
        perm::SHIPMENTS_BOOK,
        perm::SHIPMENTS_VOID,
        perm::SHIPMENTS_CANCEL,
    ];
    let noor = make_user(app.db(), "Noor", &client_grants).await;
    let nadia = make_user(app.db(), "Nadia", &client_grants).await;
    let farah = make_user(app.db(), "Farah", &client_grants).await;
    let accountant = [
        perm::REMITTANCES_VIEW,
        perm::REMITTANCES_PREPARE,
        perm::REMITTANCES_APPROVE,
        perm::REMITTANCES_PAY,
    ];
    let amal = make_user(app.db(), "Amal", &accountant).await;
    let bashar = make_user(app.db(), "Bashar", &accountant).await;
    portal.grant(noor.id, SOUQ.0);
    portal.grant(nadia.id, SOUQ.0);
    portal.grant(farah.id, OTHER_CLIENT.0);
    World {
        app,
        _views: views,
        notices,
        tickets,
        voided,
        ops,
        dispatcher,
        dispatcher_b,
        hub,
        driver,
        noor,
        nadia,
        farah,
        amal,
        bashar,
        parties,
    }
}

impl World {
    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// The app state.
    pub fn state(&self) -> &AppState {
        self.app.state()
    }

    /// `by` posts `fields` to `path`, the way a sheet does (htmx).
    pub async fn post(&self, by: &User, path: &str, fields: &[(&str, &str)]) -> TestResponse {
        self.app.acting_as(by);
        self.app.htmx().post(path, fields).await
    }

    /// `by` gets `path`.
    pub async fn get(&self, by: &User, path: &str) -> TestResponse {
        self.app.acting_as(by);
        self.app.get(path).await
    }

    /// `by` books a shipment of one parcel for `client` with the client
    /// `reference` ("" for none); the booking must succeed.
    pub async fn book_as(&self, by: &User, client: PartyId, reference: &str) -> Shipment {
        self.book_with(by, client, reference, &[]).await
    }

    /// [`book_as`](Self::book_as) with more `fields` (they win over the
    /// defaults when named twice, as the last one is read).
    pub async fn book_with(
        &self,
        by: &User,
        client: PartyId,
        reference: &str,
        more: &[(&str, &str)],
    ) -> Shipment {
        let response = self.try_book(by, client, reference, more).await;
        response.assert_status(303);
        Shipment::query()
            .order_by_desc(Shipment::ID)
            .first(self.db())
            .await
            .unwrap()
            .expect("the shipment is booked")
    }

    /// `by` posts a booking and answers the response.
    pub async fn try_book(
        &self,
        by: &User,
        client: PartyId,
        reference: &str,
        more: &[(&str, &str)],
    ) -> TestResponse {
        let client = client.0.to_string();
        let mut fields: Vec<(&str, &str)> = vec![
            ("client_party_id", client.as_str()),
            ("client_reference", reference),
            ("recipient_name", "Mariam Al Hosani"),
            ("recipient_phone", "+971500000000"),
            ("address", "Villa 12, Al Barsha 2"),
            ("area", "Al Barsha"),
            ("parcels", "1"),
            ("weight_grams", "1200"),
            ("cod", "0"),
        ];
        fields.extend_from_slice(more);
        self.post(by, "/shipments", &fields).await
    }

    /// The shipment `id`, as stored now.
    pub async fn shipment(&self, id: i64) -> Shipment {
        Shipment::find_or_404(self.db(), id).await.unwrap()
    }

    /// `shipment`, as stored now.
    pub async fn reload(&self, shipment: &Shipment) -> Shipment {
        self.shipment(shipment.id).await
    }

    /// The label of the first parcel of `shipment`.
    pub async fn label(&self, shipment: &Shipment) -> String {
        shipment.parcels(self.db()).await.unwrap().remove(0).barcode
    }

    /// The delivery of `shipment`.
    pub async fn delivery_of(&self, shipment: &Shipment) -> Delivery {
        let id = self
            .reload(shipment)
            .await
            .delivery_id
            .expect("the shipment has a delivery");
        Delivery::find_or_404(self.db(), id).await.unwrap()
    }

    /// The driver picks up `shipment` with all its labels.
    pub async fn pick_up(&self, shipment: &Shipment) -> Shipment {
        let codes = self.labels(shipment).await;
        self.post(
            &self.driver,
            &format!("/shipments/{}/pickup", shipment.id),
            &[("barcodes", codes.as_str())],
        )
        .await
        .assert_status(303);
        let after = self.reload(shipment).await;
        assert_eq!(after.status, ShipmentStatus::PickedUp);
        after
    }

    /// Hub staff scan `shipment` in with all its labels.
    pub async fn at_hub(&self, shipment: &Shipment) -> Shipment {
        let codes = self.labels(shipment).await;
        self.post(
            &self.hub,
            &format!("/shipments/{}/hub", shipment.id),
            &[("barcodes", codes.as_str())],
        )
        .await
        .assert_status(303);
        let after = self.reload(shipment).await;
        assert_eq!(after.status, ShipmentStatus::AtHub);
        after
    }

    /// Every label of `shipment`, in one text.
    pub async fn labels(&self, shipment: &Shipment) -> String {
        shipment
            .parcels(self.db())
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.barcode)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The shipment picked up, scanned in at the hub, planned on a run of
    /// the driver today and dispatched: it is `out_for_delivery`, and the
    /// delivery with the run.
    pub async fn out_for_delivery(&self, shipment: &Shipment) -> (Shipment, Delivery) {
        let at_hub = self.at_hub(&self.pick_up(shipment).await).await;
        let delivery = self.delivery_of(&at_hub).await;
        let run = wl_delivery::plan::create_run(
            self.state(),
            NewRun::new(today(), self.driver.id).vehicle(VehicleType::Van, Some(VAN)),
        )
        .await
        .expect("the run is planned");
        let run_id = run.id.to_string();
        self.post(
            &self.dispatcher,
            &format!("/deliveries/{}/assign", delivery.id),
            &[("run_id", run_id.as_str())],
        )
        .await
        .assert_ok();
        self.post(&self.dispatcher, &format!("/runs/{}/dispatch", run.id), &[])
            .await
            .assert_ok();
        let after = self.reload(&at_hub).await;
        assert_eq!(after.status, ShipmentStatus::OutForDelivery);
        let delivery = self.delivery_of(&after).await;
        (after, delivery)
    }

    /// The driver fails the stop of `delivery` for `reason`.
    pub async fn fail_stop(&self, delivery: &Delivery, reason: &str) -> TestResponse {
        self.post(
            &self.driver,
            &format!("/driver/stops/{}/fail", delivery.id),
            &[
                ("reason", reason),
                ("latitude", "25.204849"),
                ("longitude", "55.270783"),
            ],
        )
        .await
    }

    /// The driver delivers the stop of `delivery` with `cash` dirhams
    /// collected ("300.00"); every line is delivered in full.
    pub async fn deliver_stop(&self, delivery: &Delivery, cash: &str) -> TestResponse {
        self.deliver_stop_with(delivery, cash, &[]).await
    }

    /// [`deliver_stop`](Self::deliver_stop) with more `fields` (a difference
    /// reason, for instance).
    pub async fn deliver_stop_with(
        &self,
        delivery: &Delivery,
        cash: &str,
        more: &[(&str, &str)],
    ) -> TestResponse {
        let line = delivery.lines(self.db()).await.unwrap().remove(0);
        let delivered_field = format!("delivered_{}", line.id);
        let refused_field = format!("refused_{}", line.id);
        let mut fields: Vec<(&str, &str)> = vec![
            ("recipient_name", "Mariam Al Hosani"),
            ("photo", "photo-0001"),
            ("signature", "signature-0001"),
            ("latitude", "25.204849"),
            ("longitude", "55.270783"),
            ("cash_collected", cash),
            (delivered_field.as_str(), "1"),
            (refused_field.as_str(), "0"),
        ];
        fields.extend_from_slice(more);
        self.post(
            &self.driver,
            &format!("/driver/stops/{}/deliver", delivery.id),
            &fields,
        )
        .await
    }

    /// A shipment of `client` with `cod` dirhams to collect ("300"), booked,
    /// picked up, taken to the hub, dispatched and delivered with the cash
    /// collected in full: it is `delivered`.
    pub async fn delivered(&self, client: PartyId, reference: &str, cod: &str) -> Shipment {
        let shipment = self
            .book_with(&self.dispatcher, client, reference, &[("cod", cod)])
            .await;
        let (_out, delivery) = self.out_for_delivery(&shipment).await;
        let cash = format!("{cod}.00");
        self.deliver_stop(&delivery, &cash).await.assert_status(303);
        let after = self.reload(&shipment).await;
        assert_eq!(after.status, ShipmentStatus::Delivered);
        after
    }

    /// The weekly rule `LOG.RUL.06` for a period that ended an hour from
    /// now: everything delivered so far is in the period.
    pub async fn rule(&self) -> Vec<Built> {
        self.rule_until(renox::db::now() + TimeDelta::hours(1)).await
    }

    /// The weekly rule for the period ending at `period_end`.
    pub async fn rule_until(&self, period_end: renox::db::DateTime) -> Vec<Built> {
        remit::run_weekly(self.state(), period_end)
            .await
            .expect("the weekly rule runs")
    }

    /// The remittances of `client`, oldest first.
    pub async fn remittances_of(&self, client: PartyId) -> Vec<Remittance> {
        Remittance::query()
            .where_(Remittance::CLIENT_PARTY_ID.eq(client.0))
            .order_by(Remittance::ID)
            .get(self.db())
            .await
            .unwrap()
    }

    /// The only remittance of `client`.
    pub async fn the_remittance_of(&self, client: PartyId) -> Remittance {
        let mut all = self.remittances_of(client).await;
        assert_eq!(all.len(), 1, "one remittance");
        all.remove(0)
    }

    /// `remittance`, as stored now.
    pub async fn remittance(&self, remittance: &Remittance) -> Remittance {
        Remittance::find_or_404(self.db(), remittance.id)
            .await
            .unwrap()
    }

    /// The status of `remittance`, as stored now.
    pub async fn remittance_status(&self, remittance: &Remittance) -> RemittanceStatus {
        self.remittance(remittance).await.status
    }

    /// The client's COD terms: fee in basis points and the bank account.
    pub async fn cod_terms(&self, client: PartyId, fee_bp: i64, account: Option<&str>) {
        remit::set_cod_terms(self.db(), client, fee_bp, account)
            .await
            .expect("the terms are set");
    }

    /// The names of the transitions `user` is offered on `shipment`.
    pub async fn offered(&self, user: &User, shipment: &Shipment) -> Vec<String> {
        let response = self
            .get(user, &format!("/probe/{}/available", shipment.id))
            .await;
        response.assert_ok();
        response
            .text()
            .split(',')
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect()
    }
}
