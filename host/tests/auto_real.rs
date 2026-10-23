//! 08 Auto against the REAL shared modules of the host (wl-scheduling,
//! wl-inventory, wl-invoicing, wl-parties): the workshop flow of #397 .. #403
//! from the check-in of a booking to the invoice on delivery, and the parts
//! issued from and returned to a real store. The app's own tests
//! (`apps/auto/tests`) show the same flow against stand-ins.

mod support;

use std::sync::{Arc, Mutex};

use auto::job_cards::{self, Delivery, Intake, NewJobCard, NewLabour, NewPart, Promise};
use auto::vehicles::{self, NewVehicle};
use auto::{Emirate, JobCard, JobCardStatus, PartCondition, WorkStatus, parts};
use host::roles::{SERVICE_ADVISOR, TECHNICIAN, WORKSHOP_MANAGER};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::testing::TestApp;
use support::{person, tenant};
use wl_inventory::movement::StockMovement;
use wl_inventory::{Item, Location};
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::{PartyId, Subject};
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{
    AvailabilityRule, Booking, BookingStatus, Resource, ResourceType, ServiceType, resource,
};

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
        "auto-real-who"
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

/// The workshop of the bookings and of the store.
fn workshop() -> Subject {
    Subject::new("scheduling.location", 1)
}

/// 5 brake pads at the store of the workshop.
async fn stocked(w: &World) -> (Item, Location) {
    let mut item = Item::new("BRK-PAD", "Brake pad set", "pcs");
    item.save(w.db()).await.unwrap();
    let mut store = Location::new("WORKSHOP-1", "Workshop 1").in_scope(&workshop());
    store.save(w.db()).await.unwrap();
    let mut received = StockMovement::receive(
        item.id,
        None,
        store.id,
        5_000,
        8_000,
        40_000,
        &Subject::new("inventory.goods_receipt", 900),
        None,
    );
    received.save(w.db()).await.unwrap();
    (item, store)
}

/// What the store holds of the item, in thousandths.
async fn on_hand(w: &World, item: &Item, store: &Location) -> i64 {
    wl_inventory::balance::on_hand_at(w.db(), item.id, store.id, None)
        .await
        .unwrap()
}

/// Check-in of a booking, estimate, approval on the link, work, a part from the
/// real store, QC, delivery, the real invoice and the completed booking.
#[renox::test]
async fn aut_base_real_check_in_estimate_approve_work_deliver_and_invoice() {
    let w = boot().await;
    let (item, store) = stocked(&w).await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (tareq_user, tareq) = w.staff("Tareq", TECHNICIAN).await;
    let (_, waleed) = w.staff("Waleed", WORKSHOP_MANAGER).await;
    let owner = tenant(&w.app, "Mona Hassan").await;
    let vehicle = vehicles::create(
        w.state(),
        &sami,
        NewVehicle::new(Emirate::Dubai, "A", "12345", "Toyota", "Hilux", owner.0)
            .vin("JTHBK262582012345")
            .odometer(52_000),
    )
    .await
    .unwrap();

    // The booking of the periodic service and its check-in.
    let mut bay = Resource::new("Bay 1", ResourceType::Bay).at(&workshop());
    bay.save(w.db()).await.unwrap();
    for weekday in 1..=7 {
        AvailabilityRule::new(bay.id, weekday, 0, 1440)
            .save(w.db())
            .await
            .unwrap();
    }
    let periodic = ServiceType::query()
        .where_(ServiceType::CODE.eq("periodic_service"))
        .first(w.db())
        .await
        .unwrap()
        .expect("the seed stored the periodic service");
    assert!(periodic.check_in_required);
    resource::offer(w.db(), periodic.id, bay.id).await.unwrap();
    let start = renox::db::now() - TimeDelta::hours(2);
    let booking = service::book(
        w.state(),
        NewBooking::new(PartyId(owner.0), periodic.id, &[bay.id], start)
            .for_subject(vehicle.subject())
            .at(workshop()),
        Actor::staff(sami.id),
        start - TimeDelta::hours(1),
    )
    .await
    .expect("the booking is made");
    service::check_in(
        w.state(),
        booking.id,
        Actor::staff(sami.id),
        renox::db::now(),
    )
    .await
    .expect("the vehicle is checked in");

    let card = JobCard::of_booking(w.db(), booking.id)
        .await
        .unwrap()
        .expect("the check-in opened the job card");
    assert_eq!(card.status, JobCardStatus::CheckedIn);
    assert_eq!(card.customer_party_id, owner.0);
    assert_eq!(
        card.workshop().map(|s| s.to_string()),
        Some(workshop().to_string())
    );
    assert_eq!(
        Booking::find_or_404(w.db(), booking.id)
            .await
            .unwrap()
            .work_done(),
        Some(card.subject())
    );

    // Intake, technician, inspection, estimate.
    job_cards::record_intake(
        w.state(),
        &sami,
        card.id,
        Intake {
            complaint: "Periodic service and brake noise".to_owned(),
            odometer_in: 52_100,
            fuel_level: Some("half".to_owned()),
            condition: "Scratch on the left door".to_owned(),
            acknowledged_by: Some("Mona Hassan".to_owned()),
            signature: None,
            accident_damage: false,
            police_report_no: None,
            inspection_fee: 0,
            storage_fee_daily: 0,
        },
    )
    .await
    .unwrap();
    job_cards::assign_technician(w.state(), &sami, card.id, tareq_user.id)
        .await
        .unwrap();
    job_cards::start_inspection(w.state(), &tareq, card.id)
        .await
        .unwrap();
    let labour = job_cards::add_labour(
        w.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Brake pad replacement", 2_000, 15_000),
    )
    .await
    .unwrap();
    let pads = job_cards::add_part(
        w.state(),
        &tareq,
        card.id,
        NewPart::new("Brake pad set", 1_000, 30_000)
            .item("BRK-PAD")
            .condition(PartCondition::New),
    )
    .await
    .unwrap();
    job_cards::send_estimate(w.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    let round = job_cards::pending_round(w.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    job_cards::approve_by_link(w.state(), &round.token, "Mona Hassan", None, true)
        .await
        .unwrap();

    // The work, the part from the real store, the QC.
    job_cards::record_work(w.state(), &tareq, labour.id, WorkStatus::Done, None)
        .await
        .unwrap();
    let issued = parts::issue(w.state(), &tareq, pads.id, None)
        .await
        .unwrap();
    assert_eq!((issued.posted_milli, issued.short_milli), (1_000, 0));
    assert_eq!(on_hand(&w, &item, &store).await, 4_000);
    parts::fit(w.state(), &tareq, pads.id, None).await.unwrap();
    job_cards::finish_work(w.state(), &tareq, card.id)
        .await
        .unwrap();
    let all: Vec<String> = auto::job_card::QC_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();
    job_cards::save_qc(w.state(), &waleed, card.id, &all)
        .await
        .unwrap();
    job_cards::pass_qc(w.state(), &waleed, card.id)
        .await
        .unwrap();

    // Delivery: the invoice and the booking.
    let delivered = job_cards::deliver(
        w.state(),
        &sami,
        card.id,
        Delivery {
            delivered_to: "Mona Hassan".to_owned(),
            odometer_out: Some(52_110),
            ..Delivery::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(delivered.status, JobCardStatus::Delivered);
    let invoice_id = delivered.invoice_id.expect("the invoice was raised");
    let books = invoicer(w.state());
    let invoice = books
        .find(w.db(), InvoiceId(invoice_id))
        .await
        .unwrap()
        .expect("the invoice exists");
    assert_eq!(invoice.gross_total, 63_000, "AED 600.00 and 5% VAT");
    assert_eq!(invoice.party, PartyId(owner.0));
    assert_ne!(invoice.status, InvoiceStatus::Draft, "issued at once");
    let billed = books.billing(w.db(), &card.subject()).await.unwrap();
    assert_eq!(billed, [InvoiceId(invoice_id)]);
    assert_eq!(
        Booking::find_or_404(w.db(), booking.id)
            .await
            .unwrap()
            .status,
        BookingStatus::Completed
    );
}

/// What was issued and not fitted goes back to the real store with a
/// `return_in` movement against the job card.
#[renox::test]
async fn aut_base_real_parts_not_fitted_return_to_the_store() {
    let w = boot().await;
    let (item, store) = stocked(&w).await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (tareq_user, tareq) = w.staff("Tareq", TECHNICIAN).await;
    let owner = tenant(&w.app, "Mona Hassan").await;
    let vehicle = vehicles::create(
        w.state(),
        &sami,
        NewVehicle::new(Emirate::Sharjah, "B", "777", "Nissan", "Patrol", owner.0).odometer(10_000),
    )
    .await
    .unwrap();
    let card = job_cards::check_in(
        w.state(),
        &sami,
        NewJobCard::new(vehicle.id, "Squeaking brakes")
            .odometer(10_050)
            .condition("Clean")
            .technician(tareq_user.id)
            .workshop(workshop()),
    )
    .await
    .unwrap();
    job_cards::start_inspection(w.state(), &tareq, card.id)
        .await
        .unwrap();
    let pads = job_cards::add_part(
        w.state(),
        &tareq,
        card.id,
        NewPart::new("Brake pad set", 3_000, 30_000).item("BRK-PAD"),
    )
    .await
    .unwrap();
    job_cards::send_estimate(w.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    let round = job_cards::pending_round(w.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    job_cards::approve_by_link(w.state(), &round.token, "Mona Hassan", None, true)
        .await
        .unwrap();

    parts::issue(w.state(), &tareq, pads.id, None)
        .await
        .unwrap();
    assert_eq!(on_hand(&w, &item, &store).await, 2_000);
    parts::fit(w.state(), &tareq, pads.id, Some(1_000))
        .await
        .unwrap();
    let back = parts::return_unfitted(w.state(), card.id).await.unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].returned_milli, 2_000);
    assert_eq!(on_hand(&w, &item, &store).await, 4_000);
    let movements = StockMovement::query()
        .where_(StockMovement::ITEM_ID.eq(item.id))
        .order_by(StockMovement::ID)
        .get(w.db())
        .await
        .unwrap();
    let returned = movements
        .iter()
        .find(|m| m.movement_type == wl_inventory::movement::MovementType::ReturnIn)
        .expect("a return_in movement");
    assert_eq!(returned.quantity_milli, 2_000);
    assert_eq!(returned.source(), card.subject());
}
