//! BL-AUT-01 (#397) against the REAL shared modules of the host (wl-scheduling,
//! wl-inventory, wl-invoicing, wl-ticketing, wl-parties): cancelling a job card
//! after the work started bills the work done on a real invoice, gives the
//! parts not fitted back to the real store, cancels the real booking, waits for
//! the branch manager when the bill is lowered, opens the purchaser's task for a
//! special-order part, and releases the vehicle when the real invoice is paid.
//! The app's own tests (`apps/auto/tests/aut_01_cancel.rs`) show the same
//! points against stand-ins.

mod support;

use std::sync::{Arc, Mutex};

use auto::job_cards::{self, Intake, NewLabour, NewPart, Promise};
use auto::job_stops::{self, StopRequest};
use auto::release::{self, Release};
use auto::vehicles::{self, NewVehicle};
use auto::{Emirate, JobCard, JobCardStatus, LineStatus, PartLine, WorkStatus, parts};
use host::roles::{BRANCH_MANAGER, SERVICE_ADVISOR, TECHNICIAN};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{company, person, tenant};
use wl_inventory::movement::StockMovement;
use wl_inventory::{
    Item, Location, MovementType, PurchaseOrder, PurchaseOrderStatus,
};
use wl_kernel::approvals::approve;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::tickets::task_creator;
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
        "auto-cancel-real-who"
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

/// 5 brake pads and 3 oil filters at the store of the workshop.
async fn stocked(w: &World) -> (Item, Item, Location) {
    let mut pads = Item::new("BRK-PAD", "Brake pad set", "pcs");
    pads.save(w.db()).await.unwrap();
    let mut filter = Item::new("OIL-F", "Oil filter", "pcs");
    filter.save(w.db()).await.unwrap();
    let mut store = Location::new("WORKSHOP-1", "Workshop 1").in_scope(&workshop());
    store.save(w.db()).await.unwrap();
    for (item, quantity, cost, source) in [(&pads, 5_000, 40_000, 900), (&filter, 3_000, 9_000, 901)]
    {
        let mut received = StockMovement::receive(
            item.id,
            None,
            store.id,
            quantity,
            8_000,
            cost,
            &Subject::new("inventory.goods_receipt", source),
            None,
        );
        received.save(w.db()).await.unwrap();
    }
    (pads, filter, store)
}

/// What the store holds of the item, in thousandths.
async fn on_hand(w: &World, item: &Item, store: &Location) -> i64 {
    wl_inventory::balance::on_hand_at(w.db(), item.id, store.id, None)
        .await
        .unwrap()
}

/// JC-10 against the real modules.
struct Jc10 {
    card: JobCard,
    booking: Booking,
    pads: Item,
    filter: Item,
    store: Location,
    owner: PartyId,
    sami: AuthUser,
    sami_user: User,
}

/// JC-10: `in_progress`, opened by the check-in of its real booking, labour
/// "Brake pads" done (AED 200), labour "Discs" not done (AED 250), the pads
/// fitted (AED 300) and an oil filter issued and not fitted (AED 40), all from
/// the real store.
async fn jc10(w: &World) -> Jc10 {
    let (pads, filter, store) = stocked(w).await;
    let (sami_user, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (tareq_user, tareq) = w.staff("Tareq", TECHNICIAN).await;
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
    job_cards::record_intake(
        w.state(),
        &sami,
        card.id,
        Intake {
            complaint: "Brake noise".to_owned(),
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
    let pads_labour = job_cards::add_labour(
        w.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Brake pads", 1_000, 20_000),
    )
    .await
    .unwrap();
    job_cards::add_labour(
        w.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Discs", 1_000, 25_000),
    )
    .await
    .unwrap();
    let pad_set = job_cards::add_part(
        w.state(),
        &tareq,
        card.id,
        NewPart::new("Brake pad set", 1_000, 30_000).item("BRK-PAD"),
    )
    .await
    .unwrap();
    let oil_filter = job_cards::add_part(
        w.state(),
        &tareq,
        card.id,
        NewPart::new("Oil filter", 1_000, 4_000).item("OIL-F"),
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
    job_cards::record_work(w.state(), &tareq, pads_labour.id, WorkStatus::Done, None)
        .await
        .unwrap();
    parts::issue(w.state(), &tareq, pad_set.id, None)
        .await
        .unwrap();
    parts::fit(w.state(), &tareq, pad_set.id, None)
        .await
        .unwrap();
    parts::issue(w.state(), &tareq, oil_filter.id, None)
        .await
        .unwrap();
    assert_eq!(on_hand(w, &filter, &store).await, 2_000);
    assert_eq!(on_hand(w, &pads, &store).await, 4_000);
    let card = JobCard::find_or_404(w.db(), card.id).await.unwrap();
    assert_eq!(card.status, JobCardStatus::InProgress);
    Jc10 {
        card,
        booking,
        pads,
        filter,
        store,
        owner: PartyId(owner.0),
        sami,
        sami_user,
    }
}

/// The customer signs.
fn customer_stopped() -> StopRequest {
    StopRequest::new("customer_stopped").signed("Mona Hassan")
}

/// JC-10 is cancelled with reason "customer stopped" and the customer signs: an
/// invoice for AED 500 + VAT is issued on the real books to the owner, the
/// filter is back in the real store with a `return_in` against the job card,
/// the real booking is `cancelled` with the work done, and the vehicle leaves
/// when the invoice is paid.
#[renox::test]
async fn aut_01_real_a_cancel_invoices_the_work_done_returns_the_filter_and_cancels_the_booking() {
    let w = boot().await;
    let jc = jc10(&w).await;

    let stopped = job_stops::cancel(w.state(), &jc.sami, jc.card.id, customer_stopped())
        .await
        .expect("the cancel runs");

    assert!(stopped.approval_request.is_none());
    let card = JobCard::find_or_404(w.db(), jc.card.id).await.unwrap();
    assert_eq!(card.status, JobCardStatus::Cancelled);
    let invoice_id = card.invoice_id.expect("the invoice for the work done");
    let books = invoicer(w.state());
    let invoice = books
        .find(w.db(), InvoiceId(invoice_id))
        .await
        .unwrap()
        .expect("the invoice exists");
    assert_eq!(invoice.gross_total, 52_500, "AED 500 and 5% VAT");
    assert_eq!(invoice.party, jc.owner);
    assert_ne!(invoice.status, InvoiceStatus::Draft, "issued, not a draft");
    assert_eq!(
        books.billing(w.db(), &card.subject()).await.unwrap(),
        [InvoiceId(invoice_id)]
    );

    assert_eq!(on_hand(&w, &jc.filter, &jc.store).await, 3_000, "back");
    assert_eq!(on_hand(&w, &jc.pads, &jc.store).await, 4_000, "fitted");
    let back = StockMovement::query()
        .where_(StockMovement::ITEM_ID.eq(jc.filter.id))
        .where_(StockMovement::MOVEMENT_TYPE.eq(MovementType::ReturnIn))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].quantity_milli, 1_000);
    assert_eq!(back[0].source(), card.subject());

    let booking = Booking::find_or_404(w.db(), jc.booking.id).await.unwrap();
    assert_eq!(booking.status, BookingStatus::Cancelled);
    assert_eq!(booking.cancel_code.as_deref(), Some("job_stopped"));
    assert_eq!(booking.work_done_amount, 52_500);

    // Cash terms with the invoice unpaid: the vehicle stays; paid, it goes.
    let unpaid = release::release(w.state(), &jc.sami, card.id, Release::safe("Mona Hassan"))
        .await
        .unwrap_err();
    assert!(unpaid.to_string().contains("not paid"), "{unpaid}");
    books
        .allocate(
            w.state(),
            InvoiceId(invoice_id),
            Subject::new("payments.payment", 9_001),
            invoice.gross_total,
        )
        .await
        .unwrap();
    let released = release::release(w.state(), &jc.sami, card.id, Release::safe("Mona Hassan"))
        .await
        .expect("paid: the vehicle is released");
    assert!(released.released_at.is_some());
}

/// A lowered bill (AED 300 instead of AED 500) waits for branch manager B on
/// the real approvals: JC-10 stays `in_progress`, nothing is invoiced, the
/// filter stays out and the real booking stays open; B's approval completes it
/// as above with AED 300. The advisor cannot approve it.
#[renox::test]
async fn aut_01_real_a_lowered_bill_is_invoiced_at_the_approved_amount() {
    let w = boot().await;
    let jc = jc10(&w).await;
    let (basma, _) = w.staff("Basma", BRANCH_MANAGER).await;

    let asked = job_stops::cancel(
        w.state(),
        &jc.sami,
        jc.card.id,
        customer_stopped().bill(30_000),
    )
    .await
    .unwrap();

    let request = asked.approval_request.expect("an approval request waits");
    let card = JobCard::find_or_404(w.db(), jc.card.id).await.unwrap();
    assert_eq!(card.status, JobCardStatus::InProgress);
    assert_eq!(card.invoice_id, None);
    assert_eq!(on_hand(&w, &jc.filter, &jc.store).await, 2_000);
    assert_eq!(
        Booking::find_or_404(w.db(), jc.booking.id)
            .await
            .unwrap()
            .status,
        BookingStatus::CheckedIn
    );
    let own = approve(w.state(), request, &jc.sami_user, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "A cannot approve it");

    approve(w.state(), request, &basma, "").await.unwrap();

    let card = JobCard::find_or_404(w.db(), jc.card.id).await.unwrap();
    assert_eq!(card.status, JobCardStatus::Cancelled);
    let invoice = invoicer(w.state())
        .find(w.db(), InvoiceId(card.invoice_id.expect("the invoice")))
        .await
        .unwrap()
        .expect("the invoice exists");
    assert_eq!(invoice.gross_total, 31_500, "AED 300 and 5% VAT");
    assert_ne!(invoice.status, InvoiceStatus::Draft);
    assert_eq!(on_hand(&w, &jc.filter, &jc.store).await, 3_000);
    assert_eq!(
        Booking::find_or_404(w.db(), jc.booking.id)
            .await
            .unwrap()
            .status,
        BookingStatus::Cancelled
    );
}

/// An open supplier PO for a special-order part on JC-10: the part is billed
/// and goes with the vehicle, the real purchase order is unchanged after the
/// cancel and the purchaser has a task (on the real ticketing) to revise it.
#[renox::test]
async fn aut_01_real_the_purchase_order_of_a_special_part_is_unchanged_and_the_purchaser_has_a_task()
 {
    let w = boot().await;
    let jc = jc10(&w).await;
    let supplier = company(&w.app, "Gulf Gearbox Trading").await;
    let mut order = PurchaseOrder::new(supplier, jc.store.id);
    let mut tx = w.db().begin().await.unwrap();
    order
        .insert_in_state(&mut tx, PurchaseOrderStatus::Issued, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut gearbox = PartLine {
        job_card_id: jc.card.id,
        description: "Gearbox".to_owned(),
        quantity_milli: 1_000,
        unit_price: 90_000,
        special_order: true,
        special_order_agreed: true,
        approval: LineStatus::Approved,
        ..PartLine::default()
    };
    gearbox.save(w.db()).await.unwrap();
    job_stops::link_purchase_order(w.state(), &jc.sami, gearbox.id, Some(order.id))
        .await
        .unwrap();

    job_stops::cancel(w.state(), &jc.sami, jc.card.id, customer_stopped())
        .await
        .unwrap();

    let after = PurchaseOrder::find_or_404(w.db(), order.id).await.unwrap();
    assert_eq!(after.status, PurchaseOrderStatus::Issued, "not cancelled");
    assert_eq!(after.revision, 1, "not revised");
    let card = JobCard::find_or_404(w.db(), jc.card.id).await.unwrap();
    let key = format!("auto.po_revise:{}:{}", card.id, gearbox.id);
    let task = task_creator(w.state())
        .find_open(w.state(), &key)
        .await
        .unwrap();
    assert!(task.is_some(), "the purchaser has a task to revise the order");
    let invoice = invoicer(w.state())
        .find(w.db(), InvoiceId(card.invoice_id.unwrap()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(invoice.gross_total, 147_000, "AED 1,400 and 5% VAT");
}
