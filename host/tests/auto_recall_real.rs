//! BL-AUT-16 (#412) against the REAL shared modules of the host (wl-inventory,
//! wl-sales, wl-parties): a supplier recalls a part that is not tracked by
//! lot. The Owner / GM or a branch manager opens the recall with the item, the
//! supplier and the received date range of the notice; the whole stock of the
//! part is held; wl-inventory traces every issue and sale from the first
//! receipt in the range, each line *possible*, and the app's
//! `TracePartyResolver` says who is behind each job card and sale. The app's
//! own tests (`apps/auto/tests/aut_16_recall.rs`) show the list, the notice
//! and the page against stand-ins.

mod support;

use std::sync::{Arc, Mutex};

use auto::fleet_contracts::{self, NewContract};
use auto::recalls;
use auto::vehicles::{self, NewVehicle};
use auto::{Emirate, FleetContract, FleetStatus, JobCard, JobCardStatus, Vehicle};
use host::roles::{BRANCH_MANAGER, FLEET_ACCOUNT_MANAGER, OWNER, SERVICE_ADVISOR};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{company, person, tenant};
use wl_inventory::movement::StockMovement;
use wl_inventory::{
    GoodsReceipt, GoodsReceiptStatus, Item, Location, MovementType, Recall, RecallLine,
};
use wl_kernel::inventory::TraceMark;
use wl_kernel::{PartyId, Subject};
use wl_sales::{CounterSale, CounterSaleStatus, SalesOrder, SalesOrderStatus};

/// The demo auto parts trader with a workshop.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-auto/client.toml"
);

/// A test-only module with the route that hands a test the signed-in user the
/// services take.
struct Who(Arc<Mutex<Option<AuthUser>>>);

impl Module for Who {
    fn name(&self) -> &'static str {
        "auto-recall-real-who"
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

/// The booted workshop.
struct World {
    app: TestApp,
    who: Arc<Mutex<Option<AuthUser>>>,
}

/// Boots the demo client with `wl:install` and the seeds.
async fn boot() -> World {
    let who = Arc::new(Mutex::new(None));
    let config = wl_kernel::ClientConfig::load(CLIENT).expect("the auto client");
    let app = TestApp::new(
        host::app_for(config)
            .expect("the app")
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

    /// A user with the host role `role`, as the user the services take.
    async fn staff(&self, name: &str, role: &str) -> (User, AuthUser) {
        let user = person(&self.app, name, role).await;
        self.app.acting_as(&user);
        self.app.get("/t/whoami").await.assert_ok();
        let auth = self.who.lock().unwrap().take().expect("the probe saw them");
        (user, auth)
    }
}

fn day(month: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, d).unwrap()
}

fn at(month: u32, d: u32) -> DateTime {
    day(month, d).and_hms_opt(10, 0, 0).unwrap().and_utc()
}

/// The workshop of the bookings and of the store.
fn workshop() -> Subject {
    Subject::new("scheduling.location", 1)
}

/// The part, the store of Branch 1 and the supplier.
struct Shelf {
    pads: Item,
    store: Location,
    supplier: PartyId,
}

/// Brake pads (not tracked by lot) received from the supplier on each of
/// `received`, 20 each, at the store of Branch 1.
async fn shelf(w: &World, received: &[NaiveDate]) -> Shelf {
    let supplier = company(&w.app, "Brake Parts FZE").await;
    let mut pads = Item::new("BRK-PAD", "Brake pad set", "pcs");
    pads.save(w.db()).await.unwrap();
    let mut store = Location::new("BRANCH-1", "Branch 1").in_scope(&workshop());
    store.save(w.db()).await.unwrap();
    for received_on in received {
        let arrived = received_on.and_hms_opt(8, 0, 0).unwrap().and_utc();
        let mut receipt = GoodsReceipt {
            supplier_party_id: supplier.0,
            location_id: store.id,
            arrived_at: Some(arrived),
            posted_at: Some(arrived),
            status: GoodsReceiptStatus::Posted,
            ..GoodsReceipt::default()
        };
        let mut tx = w.db().begin().await.unwrap();
        receipt
            .insert_in_state(&mut tx, GoodsReceiptStatus::Posted, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let mut moved = StockMovement::receive(
            pads.id,
            None,
            store.id,
            20_000,
            8_000,
            160_000,
            &Subject::new("inventory.goods_receipt", receipt.id),
            None,
        );
        moved.moved_at = arrived;
        moved.save(w.db()).await.unwrap();
    }
    Shelf {
        pads,
        store,
        supplier,
    }
}

/// One brake pad set leaves the store on `on` for the record `used`
/// (`consume` for a job card, `dispatch` for a sales order, `issue` for a
/// counter sale).
async fn leaves(w: &World, shelf: &Shelf, kind: MovementType, used: &Subject, on: DateTime) {
    let mut moved = StockMovement::moving(
        kind,
        shelf.pads.id,
        None,
        Some(shelf.store.id),
        None,
        1_000,
        8_000,
        8_000,
        used,
        None,
    );
    moved.moved_at = on;
    moved.cost_object_type = Some(used.subject_type.clone());
    moved.cost_object_id = Some(used.subject_id);
    moved.save(w.db()).await.unwrap();
}

/// A job card of `vehicle`, `in_progress`, inserted the way a fixture is.
async fn card_of(w: &World, vehicle: &Vehicle) -> JobCard {
    let mut card = JobCard {
        vehicle_id: vehicle.id,
        customer_party_id: vehicle.owner_party_id,
        complaint: "Brake noise".to_owned(),
        odometer_in: vehicle.odometer_km,
        workshop_type: Some("scheduling.location".to_owned()),
        workshop_id: Some(1),
        status: JobCardStatus::InProgress,
        ..JobCard::default()
    };
    let mut tx = w.db().begin().await.unwrap();
    card.insert_in_state(&mut tx, JobCardStatus::InProgress, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    JobCard::find_or_404(w.db(), card.id).await.unwrap()
}

/// A completed counter sale: to `party`, or to nobody (walk-in).
async fn counter_sale(w: &World, n: i64, party: Option<PartyId>) -> Subject {
    let mut sale = CounterSale {
        number: format!("RCPT-2026-{n:04}"),
        party_id: party.map(|p| p.0),
        cashier_user_id: 1,
        currency: "AED".to_owned(),
        status: CounterSaleStatus::Completed,
        ..CounterSale::default()
    };
    let mut tx = w.db().begin().await.unwrap();
    sale.insert_in_state(&mut tx, CounterSaleStatus::Completed, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    sale.subject()
}

/// A confirmed sales order to `party`.
async fn order_to(w: &World, n: i64, party: PartyId) -> Subject {
    let mut order = SalesOrder {
        number: format!("SO-2026-{n:04}"),
        party_id: party.0,
        channel: "phone".to_owned(),
        payment_mode: "on_account".to_owned(),
        currency: "AED".to_owned(),
        revision: 1,
        status: SalesOrderStatus::Confirmed,
        ..SalesOrder::default()
    };
    let mut tx = w.db().begin().await.unwrap();
    order
        .insert_in_state(&mut tx, SalesOrderStatus::Confirmed, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    order.subject()
}

/// Opens the recall of the pads from the supplier, received `from` to `to`,
/// through the route, as `user`.
async fn open(w: &World, user: &User, shelf: &Shelf, from: &str, to: &str) -> Recall {
    w.app.acting_as(user);
    let item = shelf.pads.id.to_string();
    let supplier = shelf.supplier.0.to_string();
    w.app
        .htmx()
        .post(
            "/recalls",
            &[
                ("source", "supplier"),
                ("reason", "Brake pads below the friction standard"),
                ("supplier", supplier.as_str()),
                ("items", item.as_str()),
                ("received_from", from),
                ("received_to", to),
            ],
        )
        .await
        .assert_ok();
    Recall::query()
        .order_by_desc(Recall::ID)
        .first(w.db())
        .await
        .unwrap()
        .expect("a recall")
}

/// Done when 1: the part fitted on a job card of owner O1, on one of a fleet
/// vehicle, sold on a sales order to workshop W1 and in four walk-in sales at
/// Branch 1 on 2026-09-03: lines for O1, F1 and W1, all *possible*, and one
/// line "4 sales, Branch 1, 2026-09-03". Done when 3: the line of a vehicle
/// whose fleet contract has ended names the vehicle owner.
#[renox::test]
async fn aut_16_real_the_trace_names_the_owner_the_fleet_the_customer_and_counts_walk_in_sales() {
    let w = boot().await;
    let shelf = shelf(&w, &[day(8, 5), day(9, 1)]).await;
    let (_sami_user, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (_fatima_user, fatima) = w.staff("Fatima", FLEET_ACCOUNT_MANAGER).await;
    let o1 = tenant(&w.app, "Mona Hassan").await;
    let o2 = tenant(&w.app, "Rashed Salem").await;
    let fleet = company(&w.app, "Al Noor Logistics").await;
    let contact = tenant(&w.app, "Hamdan Saeed").await;
    let w1 = company(&w.app, "Quick Fit Workshop").await;

    let hilux = vehicles::create(
        w.state(),
        &sami,
        NewVehicle::new(Emirate::Dubai, "A", "12345", "Toyota", "Hilux", o1.0).odometer(52_000),
    )
    .await
    .unwrap();
    let van = vehicles::create(
        w.state(),
        &sami,
        NewVehicle::new(Emirate::Dubai, "B", "555", "Nissan", "Urvan", fleet.0).odometer(60_000),
    )
    .await
    .unwrap();
    let patrol = vehicles::create(
        w.state(),
        &sami,
        NewVehicle::new(Emirate::Dubai, "C", "777", "Nissan", "Patrol", o2.0).odometer(80_000),
    )
    .await
    .unwrap();
    let terms = |title: &str| {
        NewContract::new(
            fleet.0,
            title,
            renox::chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            renox::chrono::NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
        )
        .contact(contact.0)
        .labour_rate(12_000)
    };
    let current = fleet_contracts::create(w.state(), &fatima, terms("Fleet 2026"))
        .await
        .unwrap();
    fleet_contracts::add_vehicle(w.state(), &fatima, current.id, van.id)
        .await
        .unwrap();
    FleetContract::query()
        .where_(FleetContract::ID.eq(current.id))
        .update(w.db(), &[("status", &FleetStatus::Active)])
        .await
        .unwrap();
    let old = fleet_contracts::create(w.state(), &fatima, terms("Fleet 2024"))
        .await
        .unwrap();
    fleet_contracts::add_vehicle(w.state(), &fatima, old.id, patrol.id)
        .await
        .unwrap();
    FleetContract::query()
        .where_(FleetContract::ID.eq(old.id))
        .update(w.db(), &[("status", &FleetStatus::Ended)])
        .await
        .unwrap();

    let jc_hilux = card_of(&w, &hilux).await;
    let jc_van = card_of(&w, &van).await;
    let jc_patrol = card_of(&w, &patrol).await;
    for card in [&jc_hilux, &jc_van, &jc_patrol] {
        leaves(&w, &shelf, MovementType::Consume, &card.subject(), at(9, 3)).await;
    }
    let order = order_to(&w, 15, w1).await;
    leaves(&w, &shelf, MovementType::Dispatch, &order, at(9, 3)).await;
    for n in 21..=24 {
        let sale = counter_sale(&w, n, None).await;
        leaves(&w, &shelf, MovementType::Issue, &sale, at(9, 3)).await;
    }
    // A sale on account to a customer is a line of its own.
    let account = counter_sale(&w, 25, Some(w1)).await;
    leaves(&w, &shelf, MovementType::Issue, &account, at(9, 4)).await;

    let (owner_user, owner) = w.staff("Omar", OWNER).await;
    let recall = open(&w, &owner_user, &shelf, "2026-08-01", "2026-08-31").await;

    let rows = recalls::trace(w.state(), recall.id).await.unwrap();
    assert!(rows.iter().all(|row| row.mark == TraceMark::Possible));
    let party_of = |record: &str| {
        rows.iter()
            .find(|row| row.record == record)
            .and_then(|row| row.party)
    };
    assert_eq!(party_of(&jc_hilux.number()), Some(o1.0));
    assert_eq!(party_of(&jc_van.number()), Some(fleet.0));
    // The fleet's contact is told about the fleet's van.
    let van_row = rows
        .iter()
        .find(|row| row.record == jc_van.number())
        .unwrap();
    assert_eq!(van_row.addressee, Some(contact.0));
    assert_eq!(van_row.vehicle.as_deref(), Some("Dubai B 555"));
    // The contract of the Patrol ended: the vehicle owner.
    assert_eq!(party_of(&jc_patrol.number()), Some(o2.0));
    // The sales order and the sale on account are their customers'.
    let customers: Vec<Option<i64>> = rows
        .iter()
        .filter(|row| !row.walk_in && row.vehicle.is_none())
        .map(|row| row.party)
        .collect();
    assert_eq!(customers, vec![Some(w1.0), Some(w1.0)]);
    // One count for the four walk-in sales at Branch 1 on 2026-09-03.
    let counted: Vec<_> = rows.iter().filter(|row| row.walk_in).collect();
    assert_eq!(counted.len(), 1);
    assert_eq!(counted[0].sales, 4);
    assert_eq!(counted[0].branch, "Branch 1");
    assert_eq!(counted[0].on, Some(day(9, 3)));
    assert_eq!(counted[0].party, None);

    // The whole stock is held, so nothing of the pads can be taken.
    assert_eq!(
        wl_inventory::balance::balance(w.db(), shelf.pads.id, shelf.store.id, None)
            .await
            .unwrap()
            .available_milli(),
        0
    );

    // The page of the vertical, for the owner.
    w.app.acting_as(&owner_user);
    let page = w.app.get(&format!("/auto/recalls/{}", recall.id)).await;
    page.assert_ok();
    page.assert_see("Possible");
    page.assert_see("Dubai B 555");
    page.assert_see("Walk-in sales");
    page.assert_see("4 sales");
    page.assert_see("Mona Hassan");
    // The recall page of the inventory links to it.
    w.app
        .get(&format!("/recalls/{}", recall.id))
        .await
        .assert_ok()
        .assert_see(&format!("/auto/recalls/{}", recall.id));
    // The customers are told, each about their own record.
    let told = recalls::send(w.state(), &owner, recall.id).await.unwrap();
    assert_eq!(told, 5);
    let lines = RecallLine::query()
        .where_(RecallLine::RECALL_ID.eq(recall.id))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.notice_status == wl_inventory::NoticeStatus::Sent)
            .count(),
        5
    );
}

/// Done when 2: receipts of the part on 2026-08-20 and 2026-09-01 and a scope
/// received 2026-09-01 to 2026-09-30: a sale on 2026-08-30 is not listed and
/// one on 2026-09-02 is; with a scope received 2026-08-01 to 2026-08-31 the
/// sale of 2026-08-30 is listed.
#[renox::test]
async fn aut_16_real_the_first_receipt_in_the_range_starts_the_trace() {
    for (from, to, listed) in [
        ("2026-09-01", "2026-09-30", vec!["SO-2026-0002"]),
        (
            "2026-08-01",
            "2026-08-31",
            vec!["SO-2026-0001", "SO-2026-0002"],
        ),
    ] {
        let w = boot().await;
        let shelf = shelf(&w, &[day(8, 20), day(9, 1)]).await;
        let w1 = company(&w.app, "Quick Fit Workshop").await;
        let early = order_to(&w, 1, w1).await;
        leaves(&w, &shelf, MovementType::Dispatch, &early, at(8, 30)).await;
        let later = order_to(&w, 2, w1).await;
        leaves(&w, &shelf, MovementType::Dispatch, &later, at(9, 2)).await;
        let (manager, _) = w.staff("Bashir", BRANCH_MANAGER).await;

        let recall = open(&w, &manager, &shelf, from, to).await;

        let rows = recalls::trace(w.state(), recall.id).await.unwrap();
        let mut found: Vec<String> = rows.iter().map(|row| row.record.clone()).collect();
        found.sort();
        let listed: Vec<String> = listed.iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(found, listed, "scope {from} to {to}");
    }
}
