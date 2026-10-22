//! Booting the F&B app in a `TestApp`: the real wl-delivery (the order is the
//! source of its delivery), wl-sales (the order is a sales order), wl-payments
//! (the refund), wl-inventory (the wastage) and wl-parties (the customers), a
//! recording notifier and a ticket desk. The tests act through the real routes
//! as users who hold the permissions of the roles the host maps, and drive
//! the delivery the way the people do: the dispatcher puts it on a run, the
//! rider fails the stop in the driver PWA.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use fnb::permissions as perm;
use fnb::{Fnb, FnbOrder, FnbOrderStatus, KitchenTicket};
use renox::auth::{Permissions, permissions};
use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::{TestApp, TestResponse};
use wl_delivery::reasons::FailureReason;
use wl_delivery::{Delivery, NewRun, PerishableAction, Run, VehicleType, WlDelivery, seed};
use wl_delivery::permissions as dperm;
use wl_inventory::item::value_of;
use wl_inventory::movement::StockMovement;
use wl_inventory::{Item, Location, WlInventory};
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::notify::Notifier;
use wl_kernel::portal::{PortalFuture, PortalUser, PortalUsers, provide_portal_users};
use wl_kernel::tickets::TaskCreator;
use wl_kernel::{ClientConfig, Subject};
use wl_payments::WlPayments;
use wl_payments::permissions as pperm;
use wl_sales::permissions as sperm;
use wl_sales::{SalesOrder, WlSales};

pub mod notices;
pub mod tickets;

use notices::Notices;
use tickets::TicketDesk;

/// The pages extend the app's layout; a package test has no app, so it brings
/// the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// An F&B client with the modules the order touches on.
pub const CLIENT: &str = r#"
[client]
name = "Zaatar & Co. Kitchens LLC"
app = "fnb"

[modules]
enabled = ["parties", "payments", "sales", "inventory", "delivery"]

[[legal_entities]]
code = "ZCK"
name_en = "Zaatar & Co. Kitchens LLC"
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

/// The transitions the signed-in user is offered on an order, as the order
/// page would draw them (`GET /probe/{id}/available`).
async fn probe_available(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<String> {
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    let names: Vec<String> = order
        .available(&user, &state.db)
        .await?
        .into_iter()
        .map(|action| action.name)
        .collect();
    Ok(names.join(","))
}

/// Runs transition `name` of an order as the signed-in user (a test cannot
/// build an `AuthUser`) and answers `ok`, `forbidden` or the error
/// (`POST /probe/{id}/run/{name}`).
async fn probe_run(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, name)): Path<(i64, String)>,
) -> Result<String> {
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    let answer = match order.transition(&name).by(&user).run(&state).await {
        Ok(_) => "ok".to_owned(),
        Err(renox::state_machine::TransitionError::Forbidden) => "forbidden".to_owned(),
        Err(other) => format!("error: {other}"),
    };
    Ok(answer)
}

/// Mounts [`probe_available`] and [`probe_run`].
struct Probe;

impl Module for Probe {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/probe/{id}/available", probe_available)
            .post("/probe/{id}/run/{name}", probe_run)
            .require_auth()
    }
}

/// Who acts for which customer in the portal: the users the tests link to a
/// party (`wl-portal` is not part of the F&B tests).
#[derive(Clone, Default)]
pub struct Portal(Arc<Mutex<Vec<(i64, i64)>>>);

impl PortalUsers for Portal {
    fn parties_of<'a>(
        &'a self,
        _db: &'a Db,
        user: i64,
        _on: NaiveDate,
    ) -> PortalFuture<'a, Vec<PortalUser>> {
        let found: Vec<PortalUser> = self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(linked, _)| *linked == user)
            .map(|(linked, party)| PortalUser {
                user_id: *linked,
                person: PartyId(*party),
                acting_for: PartyId(*party),
                roles: vec!["owner".to_owned()],
            })
            .collect();
        Box::pin(async move { Ok(found) })
    }
}

/// Provides the [`Portal`].
struct PortalProbe(Portal);

impl Module for PortalProbe {
    fn name(&self) -> &'static str {
        "portal_probe"
    }

    fn register(&self, app: &mut Registry) {
        provide_portal_users(app, self.0.clone());
    }
}

/// The app under test.
pub struct World {
    /// The customers' portal links.
    pub portal: Portal,
    /// The app.
    pub app: TestApp,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
    /// The notices asked of the kernel's notifier.
    pub notices: Notices,
    /// The tickets the modules opened.
    pub tickets: TicketDesk,
    /// The kitchen manager.
    pub manager: User,
    /// A second kitchen manager.
    pub manager_b: User,
    /// The cashier: enters orders and asks for refunds.
    pub cashier: User,
    /// A cashier who also approves refunds (to try `FIN.ACL.04`).
    pub lead_cashier: User,
    /// A line cook.
    pub cook: User,
    /// The dispatcher: plans runs.
    pub dispatcher: User,
    /// The rider of the runs.
    pub rider: User,
    /// A second rider.
    pub other_rider: User,
    /// Approves a wastage above the limit (`stock.approve`).
    pub stock_approver: User,
    /// Someone who holds no permission.
    pub nobody: User,
    /// The stock items of the recipes.
    pub chicken: Item,
    /// Rice.
    pub rice: Item,
    /// The stock location of the outlet.
    pub store: Location,
}

/// Installs the failure reasons 06 adds to wl-delivery's list, as the host
/// does.
pub async fn install_failure_reasons(db: &Db) {
    for (code, en, ar, counts, allows) in fnb::failed::FAILURE_REASONS {
        if FailureReason::active_by_code(db, code)
            .await
            .unwrap()
            .is_some()
        {
            continue;
        }
        let mut reason = FailureReason {
            code: code.to_owned(),
            name_en: en.to_owned(),
            name_ar: ar.to_owned(),
            counts,
            allows_reattempt: allows,
            active: true,
            ..FailureReason::default()
        };
        reason.insert(db).await.unwrap();
    }
}

/// The app booted.
pub async fn boot() -> World {
    boot_with(WlDelivery::new(), Fnb::new()).await
}

/// The wl-delivery module as the 06 client sets it (`WRK.RUL.16`, BL-DLV-10):
/// own-rider deliveries are *returned* after a failed visit and *discarded*
/// after an abort, cooked food is not partly accepted, the promise is 45
/// minutes, a wastage above AED 200 needs a second person.
pub fn own_riders() -> WlDelivery {
    WlDelivery::new()
        .perishable_policy(
            "fnb.order",
            PerishableAction::Return,
            PerishableAction::Discard,
        )
        .discard_approval_limit(20_000)
        .promise_minutes("fnb.order", 45)
        .partial_acceptance("fnb.order", false)
}

/// The app booted with the 06 delivery settings and `fnb` as the vertical.
pub async fn boot_own_riders() -> World {
    boot_with(own_riders(), Fnb::new()).await
}

/// The app booted with the delivery module `delivery` and the vertical `fnb`.
pub async fn boot_with(delivery: WlDelivery, fnb: Fnb) -> World {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();

    let notices = Notices::default();
    let portal = Portal::default();
    let notifier: Arc<dyn Notifier> = Arc::new(notices.clone());
    let tickets = TicketDesk::default();
    let task_creator: Arc<dyn TaskCreator> = Arc::new(tickets.clone());
    let app = App::new()
        .module(Auth::new())
        .module(renox::audit::Audit)
        .module(Permissions)
        .module(delivery)
        .module(wl_parties::WlParties::new().exception_approver(wl_kernel::COMPANY_ADMIN_ROLE))
        .module(WlPayments::new())
        .module(
            WlSales::new()
                .cancel_outcomes(&["return_to_stock", "scrap"])
                .order_acceptance_channels(&["website"])
                .aggregator_channels(&["aggregator_a"]),
        )
        .module(WlInventory::new())
        .module(fnb)
        .module(Probe)
        .module(PortalProbe(portal.clone()))
        .provide(notifier)
        .provide(task_creator);
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app.kernel()
        .call("sales:install", Vec::<String>::new())
        .await
        .expect("sales:install runs");
    seed::install(app.db()).await.expect("the defaults seed");
    install_failure_reasons(app.db()).await;

    let manager_grants = [
        perm::ORDERS_VIEW,
        perm::ORDERS_MANAGE,
        perm::ORDERS_CANCEL,
        perm::ORDERS_AMEND,
        perm::KDS_WORK,
        perm::CUSTOMERS_COD,
        sperm::SALES_ORDERS_VIEW,
        sperm::SALES_ORDERS_ACCEPT,
        sperm::SALES_ORDERS_CANCEL,
        sperm::SALES_ORDERS_AMEND,
        sperm::SALES_ORDERS_APPROVE,
        pperm::VIEW,
        pperm::REFUNDS_APPROVE,
        dperm::VIEW,
        dperm::MANAGE,
        dperm::REVERSE,
    ];
    let manager = make_user(app.db(), "Mia", &manager_grants).await;
    let manager_b = make_user(app.db(), "Basel", &manager_grants).await;
    let cashier = make_user(
        app.db(),
        "Carla",
        &[
            perm::ORDERS_VIEW,
            perm::ORDERS_REFUND_REQUEST,
            perm::ORDERS_CANCEL,
            perm::ORDERS_AMEND,
            sperm::SALES_ORDERS_VIEW,
            sperm::SALES_ORDERS_CREATE,
            sperm::SALES_ORDERS_CANCEL,
            sperm::SALES_ORDERS_AMEND,
        ],
    )
    .await;
    let lead_cashier = make_user(
        app.db(),
        "Layla",
        &[
            perm::ORDERS_VIEW,
            perm::ORDERS_REFUND_REQUEST,
            pperm::VIEW,
            pperm::REFUNDS_APPROVE,
        ],
    )
    .await;
    let cook = make_user(app.db(), "Cyrus", &[perm::KDS_WORK]).await;
    let dispatcher = make_user(
        app.db(),
        "Dina",
        &[
            dperm::VIEW,
            dperm::DISPATCH,
            dperm::MANAGE,
            dperm::RUNS_VIEW,
            dperm::RUNS_DISPATCH,
            dperm::POD_VIEW,
        ],
    )
    .await;
    let rider = make_user(app.db(), "Rami", &[dperm::RUNS_DRIVE, dperm::COMPLETE]).await;
    let other_rider = make_user(app.db(), "Omar", &[dperm::RUNS_DRIVE, dperm::COMPLETE]).await;
    let stock_approver = make_user(
        app.db(),
        "Sana",
        &[wl_inventory::permissions::STOCK_APPROVE],
    )
    .await;
    let nobody = make_user(app.db(), "Nadia", &[]).await;

    let mut chicken = Item::new("CHICKEN", "Chicken", "kg");
    chicken.save(app.db()).await.unwrap();
    let mut rice = Item::new("RICE", "Rice", "kg");
    rice.save(app.db()).await.unwrap();
    let mut store = Location::new("MARINA", "Marina Kitchen");
    store.negative_stock_policy = wl_inventory::location::NegativeStockPolicy::AllowConsumption;
    store.save(app.db()).await.unwrap();
    let world = World {
        portal,
        app,
        _views: views,
        notices,
        tickets,
        manager,
        manager_b,
        cashier,
        lead_cashier,
        cook,
        dispatcher,
        rider,
        other_rider,
        stock_approver,
        nobody,
        chicken,
        rice,
        store,
    };
    world.stock_in_both(20_000, 1_000).await;
    fnb::seed::demo_catalogue(
        world.db(),
        Some(world.store.id),
        &[("chicken", world.chicken.id), ("rice", world.rice.id)],
    )
    .await
    .expect("the catalogue seeds");
    world
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

    /// A signed-in user who acts for the customer `party` in the portal and
    /// holds no permission (a customer follows and cancels their own order).
    pub async fn customer_user(&self, name: &str, party: PartyId) -> User {
        let user = make_user(self.db(), name, &[]).await;
        self.portal.0.lock().unwrap().push((user.id, party.0));
        user
    }

    /// 20 kg of chicken and rice in the outlet's store at AED 10 a kilo.
    async fn stock_in_both(&self, quantity_milli: i64, unit_cost: i64) {
        for item in [&self.chicken, &self.rice] {
            let source = Subject::new("inventory.goods_receipt", 900);
            let mut moved = StockMovement::receive(
                item.id,
                None,
                self.store.id,
                quantity_milli,
                unit_cost,
                value_of(quantity_milli, unit_cost),
                &source,
                None,
            );
            moved.save(self.db()).await.unwrap();
            let mut row = Item::find_or_404(self.db(), item.id).await.unwrap();
            row.average_cost = unit_cost;
            row.save(self.db()).await.unwrap();
        }
    }

    /// The quantity of `item` available at the outlet's store, in thousandths.
    pub async fn on_hand(&self, item: &Item) -> i64 {
        wl_inventory::balance::balance(self.db(), item.id, self.store.id, None)
            .await
            .unwrap()
            .available_milli()
    }

    /// The stock movements of `item`, oldest first.
    pub async fn movements_of(&self, item: &Item) -> Vec<StockMovement> {
        StockMovement::query()
            .where_(StockMovement::ITEM_ID.eq(item.id))
            .order_by(StockMovement::ID)
            .get(self.db())
            .await
            .unwrap()
    }

    /// A customer: a party the orders are for.
    pub async fn customer(&self, name: &str) -> PartyId {
        let party = wl_parties::create(
            self.state(),
            wl_parties::NewParty {
                kind: wl_kernel::PartyKind::Company,
                legal_name_en: name.to_owned(),
                source: "test".to_owned(),
                ..wl_parties::NewParty::default()
            },
        )
        .await
        .expect("the party is created");
        PartyId(party.id)
    }

    /// An order of `customer` through the direct site (the `website`
    /// channel), entered by the cashier through the sales routes: `mode` is
    /// `online` (paid in full by the gateway) or `cash_on_delivery`; `lines`
    /// are `(item code, quantity, unit price in AED)`, zero-rated so the
    /// figures are the stories'. The delivery details are set by the manager.
    /// The order is `placed`.
    pub async fn place(
        &self,
        customer: PartyId,
        mode: &str,
        lines: &[(&str, &str, &str)],
    ) -> FnbOrder {
        let order = self.place_via("website", customer, mode, lines).await;
        assert_eq!(order.status, FnbOrderStatus::Placed);
        order
    }

    /// [`place`](Self::place) through `channel`: the website waits for the
    /// outlet to accept (`placed`), the phone does not (`accepted`).
    pub async fn place_via(
        &self,
        channel: &str,
        customer: PartyId,
        mode: &str,
        lines: &[(&str, &str, &str)],
    ) -> FnbOrder {
        let party = customer.0.to_string();
        self.post(
            &self.cashier,
            "/sales-orders",
            &[
                ("party_id", party.as_str()),
                ("channel", channel),
                ("payment_mode", mode),
            ],
        )
        .await
        .assert_status(303);
        let sales = SalesOrder::query()
            .order_by_desc(SalesOrder::ID)
            .first(self.db())
            .await
            .unwrap()
            .expect("the sales order was entered");
        for (code, quantity, price) in lines {
            self.post(
                &self.cashier,
                &format!("/sales-orders/{}/lines", sales.id),
                &[
                    ("item_code", *code),
                    ("quantity", *quantity),
                    ("unit_price", *price),
                    ("vat", "zero_rated"),
                ],
            )
            .await
            .assert_status(303);
        }
        self.post(
            &self.cashier,
            &format!("/sales-orders/{}/submit", sales.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
        if mode == "online" {
            let submitted = SalesOrder::find_or_404(self.db(), sales.id).await.unwrap();
            wl_sales::sales_orders::confirm_payment(
                self.state(),
                sales.id,
                submitted.gross_total,
            )
            .await
            .expect("the gateway confirmed the payment");
        }
        let order = FnbOrder::of_sales_order(self.db(), sales.id)
            .await
            .unwrap()
            .expect("the F&B order was made");
        self.post(
            &self.manager,
            &format!("/orders/{}/details", order.id),
            &[
                // An aggregator's rider collects at the pass.
                ("kind", if channel.starts_with("aggregator") { "pickup" } else { "delivery" }),
                ("recipient_name", "Mariam Al Hosani"),
                ("recipient_phone", "+971500000000"),
                ("address", "Villa 12, Al Barsha 2"),
                ("area", "Al Barsha"),
            ],
        )
        .await
        .assert_status(303);
        self.reload(&order).await
    }

    /// The kitchen manager accepts the placed `order`.
    pub async fn accept(&self, order: &FnbOrder) -> FnbOrder {
        self.post(
            &self.manager,
            &format!("/orders/{}/accept", order.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
        let after = self.reload(order).await;
        assert_eq!(after.status, FnbOrderStatus::Accepted);
        after
    }

    /// The cook starts and bumps every ticket of the accepted `order`: it is
    /// `ready`, and its delivery is requested.
    pub async fn cook(&self, order: &FnbOrder) -> FnbOrder {
        for ticket in KitchenTicket::of_order(self.db(), order.id).await.unwrap() {
            self.post(
                &self.cook,
                &format!("/tickets/{}/start", ticket.id),
                &[("x", "")],
            )
            .await
            .assert_status(303);
            self.post(
                &self.cook,
                &format!("/tickets/{}/bump", ticket.id),
                &[("x", "")],
            )
            .await
            .assert_status(303);
        }
        let after = self.reload(order).await;
        assert_eq!(after.status, FnbOrderStatus::Ready);
        after
    }

    /// An order placed, accepted and cooked: `ready`.
    pub async fn ready(
        &self,
        customer: PartyId,
        mode: &str,
        lines: &[(&str, &str, &str)],
    ) -> FnbOrder {
        self.ready_via("website", customer, mode, lines).await
    }

    /// [`ready`](Self::ready) through `channel`: an order that needs no
    /// acceptance starts `accepted`.
    pub async fn ready_via(
        &self,
        channel: &str,
        customer: PartyId,
        mode: &str,
        lines: &[(&str, &str, &str)],
    ) -> FnbOrder {
        let placed = self.place_via(channel, customer, mode, lines).await;
        let accepted = if placed.status == FnbOrderStatus::Placed {
            self.accept(&placed).await
        } else {
            placed
        };
        self.cook(&accepted).await
    }

    /// `order`, as stored now.
    pub async fn reload(&self, order: &FnbOrder) -> FnbOrder {
        FnbOrder::find_or_404(self.db(), order.id).await.unwrap()
    }

    /// The delivery of `order`.
    pub async fn delivery_of(&self, order: &FnbOrder) -> Delivery {
        let id = self
            .reload(order)
            .await
            .delivery_id
            .expect("the order has a delivery");
        Delivery::find_or_404(self.db(), id).await.unwrap()
    }

    /// `delivery`, as stored now.
    pub async fn delivery(&self, delivery: &Delivery) -> Delivery {
        Delivery::find_or_404(self.db(), delivery.id).await.unwrap()
    }

    /// A run of `driver` today, planned.
    pub async fn run_of(&self, driver: &User) -> Run {
        wl_delivery::plan::create_run(
            self.state(),
            NewRun::new(today(), driver.id).vehicle(VehicleType::Van, Some(VAN)),
        )
        .await
        .expect("the run is planned")
    }

    /// The dispatcher puts the delivery of the `ready` `order` on `run` and
    /// dispatches the run: the order is `dispatched`.
    pub async fn send_out(&self, order: &FnbOrder, run: &Run) -> FnbOrder {
        let delivery = self.delivery_of(order).await;
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
        let after = self.reload(order).await;
        assert_eq!(after.status, FnbOrderStatus::Dispatched);
        after
    }

    /// An order placed, accepted, cooked, put on a new run of the rider and
    /// dispatched: `dispatched`, with its delivery and run.
    pub async fn dispatched(
        &self,
        customer: PartyId,
        mode: &str,
        lines: &[(&str, &str, &str)],
    ) -> (FnbOrder, Delivery, Run) {
        self.dispatched_via("website", customer, mode, lines).await
    }

    /// [`dispatched`](Self::dispatched) through `channel`.
    pub async fn dispatched_via(
        &self,
        channel: &str,
        customer: PartyId,
        mode: &str,
        lines: &[(&str, &str, &str)],
    ) -> (FnbOrder, Delivery, Run) {
        let ready = self.ready_via(channel, customer, mode, lines).await;
        let run = self.run_of(&self.rider).await;
        let order = self.send_out(&ready, &run).await;
        let delivery = self.delivery_of(&order).await;
        (order, delivery, run)
    }

    /// The rider fails the stop of `delivery` for `reason`.
    pub async fn fail_stop(&self, delivery: &Delivery, reason: &str) -> TestResponse {
        self.post(
            &self.rider,
            &format!("/driver/stops/{}/fail", delivery.id),
            &[
                ("reason", reason),
                ("latitude", "25.204849"),
                ("longitude", "55.270783"),
            ],
        )
        .await
    }

    /// The rider delivers the stop of `delivery`, collecting `cash` dirhams
    /// ("85.00"); every line is delivered in full.
    pub async fn deliver_stop(&self, delivery: &Delivery, cash: &str) -> TestResponse {
        let lines = delivery.lines(self.db()).await.unwrap();
        let mut fields: Vec<(String, String)> = vec![
            ("recipient_name".into(), "Mariam Al Hosani".into()),
            ("photo".into(), "photo-0001".into()),
            ("signature".into(), "signature-0001".into()),
            ("latitude".into(), "25.204849".into()),
            ("longitude".into(), "55.270783".into()),
            ("cash_collected".into(), cash.into()),
        ];
        for line in &lines {
            fields.push((format!("delivered_{}", line.id), "1".into()));
            fields.push((format!("refused_{}", line.id), "0".into()));
        }
        let borrowed: Vec<(&str, &str)> = fields
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        self.post(
            &self.rider,
            &format!("/driver/stops/{}/deliver", delivery.id),
            &borrowed,
        )
        .await
    }

    /// The rider takes the failed food back to the depot, and the dispatcher
    /// ticks it received: `DeliveryReturned` follows.
    pub async fn bring_back(&self, delivery: &Delivery, run: &Run) {
        self.post(
            &self.rider,
            &format!("/driver/runs/{}/return", run.id),
            &[("x", "")],
        )
        .await
        .assert_ok();
        let item = format!("item_{}_0", delivery.id);
        self.post(
            &self.dispatcher,
            &format!("/runs/{}/receipts", run.id),
            &[(item.as_str(), "received")],
        )
        .await
        .assert_ok();
    }

    /// The rider discards the failed food of `delivery` with a photo.
    pub async fn discard(&self, delivery: &Delivery) -> TestResponse {
        self.post(
            &self.rider,
            &format!("/driver/stops/{}/discard", delivery.id),
            &[("reason", "Spoilt in the heat"), ("photo", "discard-photo-1")],
        )
        .await
    }

    /// The dispatcher aborts `run` for `reason`.
    pub async fn abort(&self, run: &Run, reason: &str) -> TestResponse {
        self.post(
            &self.dispatcher,
            &format!("/runs/{}/abort", run.id),
            &[("reason", reason)],
        )
        .await
    }

    /// Moves the clock `delta` forward.
    pub fn travel(&self, delta: TimeDelta) {
        self.app
            .travel(delta.to_std().expect("a clock moves forward"));
    }

    /// The names of the transitions `user` is offered on `order`.
    pub async fn offered(&self, user: &User, order: &FnbOrder) -> Vec<String> {
        let response = self
            .get(user, &format!("/probe/{}/available", order.id))
            .await;
        response.assert_ok();
        response
            .text()
            .split(',')
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// What happens when `user` runs transition `name` on `order`: `ok`,
    /// `forbidden` or the error.
    pub async fn try_run(&self, user: &User, order: &FnbOrder, name: &str) -> String {
        let response = self
            .post(user, &format!("/probe/{}/run/{}", order.id, name), &[])
            .await;
        response.assert_ok();
        response.text()
    }
}
