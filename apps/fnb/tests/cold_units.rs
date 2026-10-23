//! BL-FNB-12 (#376), 06 F&B §4.6 row *A fridge or freezer reads out of range*,
//! §7.2 `FNB.RUL.05` and `STK.RUL.07`, `STK.ACL.02`, `STK.ACL.05`,
//! `FNB.ACL.16`: a temperature reading outside the unit's range opens one
//! equipment task and places one stock hold on the stock in the unit, later
//! readings and stock moved in join them, held stock is never consumed, the
//! undecided lines are reminded after 2 hours and escalated after 4, and the
//! chef decides each line recording the reading and the minutes (a discard
//! above AED 200 waits for the kitchen manager). The modules are the real
//! wl-inventory, wl-sales, wl-payments and wl-delivery.

mod support;

use fnb::permissions as perm;
use fnb::{ColdUnit, Fnb, HoldDecisionRecord, TemperatureReading};
use renox::auth::permissions;
use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_inventory::hold_reminder::{ESCALATION_NOTICE, REMINDER_NOTICE};
use wl_inventory::item::value_of;
use wl_inventory::permissions as iperm;
use wl_inventory::transfer::{NewTransfer, NewTransferLine};
use wl_inventory::{
    HoldLineStatus, Item, Location, StockHold, StockHoldLine, StockMovement, WlInventory,
};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::inventory::{ConsumeRequest, stock_service};
use wl_kernel::notify::NotifyRecipient;
use wl_payments::WlPayments;

use support::stock::{Kitchen, kitchen};
use support::{World, boot_all, boot_real_tickets, make_user, own_riders};

/// The 06 client's stock settings: a wastage above AED 200 needs a second
/// person, a temperature hold is reminded after 2 hours and escalated to the
/// kitchen manager after 4.
async fn boot_cold() -> World {
    boot_all(
        own_riders(),
        Fnb::new(),
        WlPayments::new(),
        WlInventory::new()
            .wastage_approval_fils(20_000)
            .hold_reminder_hours_for_reason("temperature", 2)
            .hold_escalation_role_for_reason("temperature", "kitchen_manager"),
    )
    .await
}

/// Fridge 2 with 6 kg of chicken (AED 40 a kilogram) and 2 kg of cream.
struct Fridge {
    unit: ColdUnit,
    location: Location,
    chicken: Item,
    cream: Item,
    logger: User,
}

async fn receive(w: &World, item: &Item, location: &Location, quantity_milli: i64) {
    let source = Subject::new("inventory.goods_receipt", 9_800);
    let mut moved = StockMovement::receive(
        item.id,
        None,
        location.id,
        quantity_milli,
        item.average_cost,
        value_of(quantity_milli, item.average_cost),
        &source,
        None,
    );
    moved.save(w.db()).await.unwrap();
}

/// Fridge 2 (range at most 5 degrees, location OUT1-F2) set up through the
/// real route by the kitchen manager.
async fn fridge(w: &World, k: &Kitchen) -> Fridge {
    let mut location = Location::new("OUT1-F2", "Fridge 2");
    location.save(w.db()).await.unwrap();
    let mut chicken = w.chicken.clone();
    chicken.average_cost = 4_000;
    chicken.save(w.db()).await.unwrap();
    let mut cream = Item::new("CREAM", "Cream", "kg");
    cream.average_cost = 1_200;
    cream.save(w.db()).await.unwrap();
    receive(w, &chicken, &location, 6_000).await;
    receive(w, &cream, &location, 2_000).await;
    let id = location.id.to_string();
    w.post(
        &k.manager,
        "/cold-units",
        &[
            ("code", "OUT1-F2"),
            ("name", "Fridge 2"),
            ("kind", "fridge"),
            ("location", id.as_str()),
        ],
    )
    .await
    .assert_status(303);
    let unit = ColdUnit::query()
        .where_(ColdUnit::CODE.eq("OUT1-F2"))
        .first(w.db())
        .await
        .unwrap()
        .expect("the unit is set up");
    let logger = make_user(
        w.db(),
        "Lena",
        &[perm::TEMPERATURE_LOG, perm::TEMPERATURE_VIEW],
    )
    .await;
    Fridge {
        unit,
        location,
        chicken,
        cream,
        logger,
    }
}

/// `by` records `degrees` for the unit.
async fn read(
    w: &World,
    by: &User,
    unit: &ColdUnit,
    degrees: &str,
) -> renox::testing::TestResponse {
    w.post(
        by,
        &format!("/cold-units/{}/readings", unit.id),
        &[("temperature", degrees)],
    )
    .await
}

/// The holds with the reason temperature.
async fn temperature_holds(w: &World) -> Vec<StockHold> {
    StockHold::query()
        .where_(StockHold::REASON.eq("temperature"))
        .order_by(StockHold::ID)
        .get(w.db())
        .await
        .unwrap()
}

async fn lines_of(w: &World, hold: &StockHold) -> Vec<StockHoldLine> {
    wl_inventory::stock_hold::lines_of(w.db(), hold.id)
        .await
        .unwrap()
}

/// AC1: given Fridge 2 (at most 5 degrees, location OUT1-F2) with 6 kg of
/// chicken and 2 kg of cream, a reading of 9 degrees creates one equipment
/// task and one hold on both lines; the chef and the kitchen manager are told.
#[renox::test]
async fn a_reading_out_of_range_holds_the_food_and_opens_one_task() {
    let w = boot_cold().await;
    let k = kitchen(&w).await;
    let f = fridge(&w, &k).await;

    read(&w, &f.logger, &f.unit, "9").await.assert_status(303);

    let holds = temperature_holds(&w).await;
    assert_eq!(holds.len(), 1, "one hold");
    assert_eq!(
        holds[0].reference_type.as_deref(),
        Some("fnb.temperature_reading")
    );
    assert_eq!(holds[0].location_id, Some(f.location.id));
    let lines = lines_of(&w, &holds[0]).await;
    assert_eq!(lines.len(), 2, "chicken and cream");
    let held: i64 = lines.iter().map(|l| l.quantity_milli).sum();
    assert_eq!(held, 8_000);
    let tasks = w
        .tickets
        .opened_with(&format!("fnb.cold_unit:{}", f.unit.id));
    assert_eq!(tasks.len(), 1, "one equipment task");
    let told = w.notices.users_of_type("fnb.staff_notice");
    assert!(told.contains(&k.chef.id), "the chef is told");
    assert!(told.contains(&k.manager.id), "the kitchen manager is told");
    let unit = ColdUnit::find_or_404(w.db(), f.unit.id).await.unwrap();
    assert_eq!(unit.hold_id, Some(holds[0].id));
    assert!(unit.ticket_number.is_some());
    let reading = TemperatureReading::of_unit(w.db(), unit.id, 5)
        .await
        .unwrap()
        .remove(0);
    assert!(!reading.in_range);
    assert_eq!(reading.tenths, 90);
    assert_eq!(reading.hold_id, Some(holds[0].id));
}

/// AC2: a second reading of 8 degrees adds to the same hold and task.
#[renox::test]
async fn a_second_reading_joins_the_same_hold_and_task() {
    let w = boot_cold().await;
    let k = kitchen(&w).await;
    let f = fridge(&w, &k).await;

    read(&w, &f.logger, &f.unit, "9").await.assert_status(303);
    read(&w, &f.logger, &f.unit, "8").await.assert_status(303);

    let holds = temperature_holds(&w).await;
    assert_eq!(holds.len(), 1, "no second hold");
    assert_eq!(
        w.tickets
            .opened_with(&format!("fnb.cold_unit:{}", f.unit.id))
            .len(),
        1
    );
    let readings = TemperatureReading::of_unit(w.db(), f.unit.id, 5)
        .await
        .unwrap();
    assert_eq!(readings.len(), 2);
    assert!(readings.iter().all(|r| r.hold_id == Some(holds[0].id)));
}

/// A reading inside the range changes nothing.
#[renox::test]
async fn a_reading_in_range_places_no_hold() {
    let w = boot_cold().await;
    let k = kitchen(&w).await;
    let f = fridge(&w, &k).await;

    read(&w, &f.logger, &f.unit, "4").await.assert_status(303);

    assert!(temperature_holds(&w).await.is_empty());
    assert!(w.tickets.opened().is_empty());
    let reading = TemperatureReading::of_unit(w.db(), f.unit.id, 5)
        .await
        .unwrap()
        .remove(0);
    assert!(reading.in_range);
}

/// AC3: an order needing cream from OUT1-F2 while it is held posts no
/// consumption from the held cream: the short quantity becomes a pending
/// issue.
#[renox::test]
async fn held_cream_is_not_consumed_and_becomes_a_pending_issue() {
    let w = boot_cold().await;
    let k = kitchen(&w).await;
    let f = fridge(&w, &k).await;
    read(&w, &f.logger, &f.unit, "9").await.assert_status(303);

    let consumed = stock_service(w.state())
        .consume(
            w.state(),
            ConsumeRequest::new(
                f.cream.subject(),
                f.location.subject(),
                500,
                Subject::new("fnb.order", 1),
            )
            .theoretical(),
        )
        .await
        .unwrap();

    assert_eq!(consumed.posted_milli, 0, "nothing posts from held stock");
    assert_eq!(consumed.short_milli, 500);
    assert!(consumed.pending_issue.is_some());
}

/// AC4: 3 kg of chicken transferred into OUT1-F2 shows the warning and is added
/// to the open hold as a new line.
#[renox::test]
async fn stock_moved_in_warns_and_joins_the_open_hold() {
    let w = boot_cold().await;
    let k = kitchen(&w).await;
    let f = fridge(&w, &k).await;
    read(&w, &f.logger, &f.unit, "9").await.assert_status(303);
    let mover = make_user(w.db(), "Tariq", &[iperm::STOCK_TRANSFER, iperm::STOCK_VIEW]).await;
    let transfer = wl_inventory::transfer::create_draft(
        w.db(),
        None,
        NewTransfer {
            from_location_id: w.store.id,
            to_location_id: f.location.id,
            lines: vec![NewTransferLine::of(f.chicken.id, 3_000)],
        },
    )
    .await
    .unwrap();
    w.post(&mover, &format!("/transfers/{}/send", transfer.id), &[])
        .await;

    let warned = w
        .post(&mover, &format!("/transfers/{}/receive", transfer.id), &[])
        .await;
    assert!(
        warned.text().contains("Fridge 2 is on temperature hold"),
        "{}",
        warned.text()
    );
    w.post(
        &mover,
        &format!("/transfers/{}/receive", transfer.id),
        &[("held_ok", "1")],
    )
    .await;

    let holds = temperature_holds(&w).await;
    assert_eq!(holds.len(), 1);
    let lines = lines_of(&w, &holds[0]).await;
    assert_eq!(
        lines.len(),
        3,
        "the moved stock is a new line of the same hold"
    );
    assert!(lines.iter().any(|l| l.quantity_milli == 3_000));
    assert!(lines.iter().all(|l| l.status == HoldLineStatus::Undecided));
}

/// AC5: at 16:00 (2 hours) undecided lines remind the chef; at 18:00 (4 hours)
/// the kitchen manager.
#[renox::test]
async fn undecided_lines_remind_the_chef_after_two_hours_and_the_manager_after_four() {
    let w = boot_cold().await;
    let k = kitchen(&w).await;
    permissions::define_role(w.db(), "kitchen_manager", &[perm::TEMPERATURE_VIEW])
        .await
        .unwrap();
    k.manager
        .assign_role(w.db(), "kitchen_manager")
        .await
        .unwrap();
    let f = fridge(&w, &k).await;
    read(&w, &f.logger, &f.unit, "9").await.assert_status(303);
    let start = renox::db::now();

    let early = wl_inventory::hold_reminder::run(w.state(), start + TimeDelta::minutes(90))
        .await
        .unwrap();
    assert_eq!((early.reminders, early.escalations), (0, 0), "too early");

    let two = wl_inventory::hold_reminder::run(w.state(), start + TimeDelta::minutes(125))
        .await
        .unwrap();
    assert_eq!(two.reminders, 1);
    assert_eq!(two.escalations, 0);
    assert!(
        w.notices
            .users_of_type(REMINDER_NOTICE)
            .contains(&k.chef.id)
    );
    assert!(w.notices.of_type(ESCALATION_NOTICE).is_empty());

    let four = wl_inventory::hold_reminder::run(w.state(), start + TimeDelta::minutes(245))
        .await
        .unwrap();
    assert_eq!(four.escalations, 1);
    let escalated = w.notices.of_type(ESCALATION_NOTICE);
    assert!(escalated.iter().any(|request| {
        request
            .recipients
            .contains(&NotifyRecipient::User { id: k.manager.id })
    }));
}

/// AC6: the chef releases the cream with reading 9 degrees and 40 minutes
/// alone; discards the chicken worth AED 240, which needs the kitchen
/// manager's approval; discarded chicken is wastage.
#[renox::test]
async fn the_chef_releases_alone_and_a_discard_above_the_limit_needs_the_manager() {
    let w = boot_cold().await;
    let k = kitchen(&w).await;
    let f = fridge(&w, &k).await;
    read(&w, &f.logger, &f.unit, "9").await.assert_status(303);
    let hold = temperature_holds(&w).await.remove(0);
    let lines = lines_of(&w, &hold).await;
    let cream = lines.iter().find(|l| l.item_id == f.cream.id).unwrap();
    let chicken = lines.iter().find(|l| l.item_id == f.chicken.id).unwrap();

    // The cream is released by the chef alone: a temperature hold is not a
    // listed reason.
    w.post(
        &k.chef,
        &format!("/cold-units/{}/lines/{}/decide", f.unit.id, cream.id),
        &[("decision", "release"), ("reading", "9"), ("minutes", "40")],
    )
    .await
    .assert_status(303);
    let released = StockHoldLine::find_or_404(w.db(), cream.id).await.unwrap();
    assert_eq!(released.status, HoldLineStatus::Released);
    let note = HoldDecisionRecord::of_line(w.db(), cream.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(note.reading_tenths, 90);
    assert_eq!(note.minutes_out, 40);
    assert_eq!(note.decision, "release");

    // The chicken, AED 240, waits for the kitchen manager.
    w.post(
        &k.chef,
        &format!("/cold-units/{}/lines/{}/decide", f.unit.id, chicken.id),
        &[("decision", "discard"), ("reading", "9"), ("minutes", "95")],
    )
    .await
    .assert_status(303);
    let waiting = StockHoldLine::find_or_404(w.db(), chicken.id)
        .await
        .unwrap();
    assert_eq!(waiting.status, HoldLineStatus::Undecided);
    let line_subject = waiting.subject();
    let request = ApprovalRequest::pending_about(w.db(), &line_subject)
        .await
        .unwrap()
        .pop()
        .expect("the discard waits for a second person");
    assert!(
        approvals::approve(w.state(), request.id, &k.chef, "")
            .await
            .is_err()
    );
    approvals::approve(w.state(), request.id, &k.manager, "")
        .await
        .expect("the kitchen manager approves");

    let discarded = StockHoldLine::find_or_404(w.db(), chicken.id)
        .await
        .unwrap();
    assert_eq!(discarded.status, HoldLineStatus::Discarded);
    let wastage: Vec<_> = StockMovement::query()
        .where_(StockMovement::ITEM_ID.eq(f.chicken.id))
        .where_(StockMovement::MOVEMENT_TYPE.eq(wl_inventory::MovementType::Wastage))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(wastage.len(), 1, "discarded chicken is wastage");
    assert_eq!(wastage[0].quantity_milli, 6_000);
    assert_eq!(wastage[0].value, 24_000);
    let closed = StockHold::find_or_404(w.db(), hold.id).await.unwrap();
    assert_ne!(
        closed.status,
        wl_inventory::stock_hold::HoldStatus::Open,
        "every line is decided"
    );
}

/// A line is not decided without the reading and the minutes out of range, and
/// only a holder of `lots.release` decides.
#[renox::test]
async fn a_line_needs_the_reading_and_the_minutes_and_the_chef() {
    let w = boot_cold().await;
    let k = kitchen(&w).await;
    let f = fridge(&w, &k).await;
    read(&w, &f.logger, &f.unit, "9").await.assert_status(303);
    let hold = temperature_holds(&w).await.remove(0);
    let line = lines_of(&w, &hold).await.remove(0);
    let path = format!("/cold-units/{}/lines/{}/decide", f.unit.id, line.id);

    w.post(&k.chef, &path, &[("decision", "release"), ("reading", "9")])
        .await
        .assert_status(422);
    w.post(
        &k.chef,
        &path,
        &[("decision", "release"), ("minutes", "40")],
    )
    .await
    .assert_status(422);
    w.post(
        &k.chef,
        &path,
        &[("decision", "keep"), ("reading", "9"), ("minutes", "40")],
    )
    .await
    .assert_status(422);
    w.post(
        &f.logger,
        &path,
        &[("decision", "release"), ("reading", "9"), ("minutes", "40")],
    )
    .await
    .assert_status(403);
    let still = StockHoldLine::find_or_404(w.db(), line.id).await.unwrap();
    assert_eq!(still.status, HoldLineStatus::Undecided);
    assert!(
        HoldDecisionRecord::of_line(w.db(), line.id)
            .await
            .unwrap()
            .is_none()
    );
}

/// A unit cannot be set up for temperature logs without a stock location, and
/// only the kitchen manager sets one up.
#[renox::test]
async fn a_unit_needs_a_stock_location_and_a_manager_to_set_it_up() {
    let w = boot_cold().await;
    let k = kitchen(&w).await;
    let form = |location: &'static str| {
        vec![
            ("code", "F9"),
            ("name", "Fridge 9"),
            ("kind", "fridge"),
            ("location", location),
        ]
    };

    w.post(&k.manager, "/cold-units", &form(""))
        .await
        .assert_status(422);
    w.post(&k.manager, "/cold-units", &form("99999"))
        .await
        .assert_status(422);
    w.post(&k.chef, "/cold-units", &form("1"))
        .await
        .assert_status(403);
    assert!(ColdUnit::query().count(w.db()).await.unwrap() == 0);
}

/// Only a holder of `temperature.log` records a reading.
#[renox::test]
async fn only_the_cooks_and_managers_record_a_reading() {
    let w = boot_cold().await;
    let k = kitchen(&w).await;
    let f = fridge(&w, &k).await;

    read(&w, &w.nobody, &f.unit, "4").await.assert_status(403);
    w.get(&w.nobody, "/cold-units").await.assert_status(403);
    w.get(&f.logger, "/cold-units").await.assert_ok();
    w.get(&f.logger, &format!("/cold-units/{}", f.unit.id))
        .await
        .assert_ok();
}

/// A failed run (the unit's location is gone) is retried and, after the last
/// attempt, alerts the owner; the reading is kept.
#[renox::test]
async fn a_reaction_that_keeps_failing_alerts_the_owner() {
    let w = boot_cold().await;
    let k = kitchen(&w).await;
    let mut broken = ColdUnit {
        code: "GONE".to_owned(),
        name: "Fridge with no location".to_owned(),
        kind: "fridge".to_owned(),
        location_id: 99_999,
        max_tenths: Some(50),
        active: true,
        ..ColdUnit::default()
    };
    broken.insert(w.db()).await.unwrap();
    let cook = make_user(
        w.db(),
        "Lena2",
        &[perm::TEMPERATURE_LOG, perm::TEMPERATURE_VIEW],
    )
    .await;
    let _ = &k;

    read(&w, &cook, &broken, "9").await.assert_status(303);
    for _ in 0..6 {
        w.travel(TimeDelta::hours(2));
        w.app
            .at_travelled_time(fnb::cold::retry_pending(w.state()))
            .await
            .unwrap();
    }

    let kept = TemperatureReading::of_unit(w.db(), broken.id, 5)
        .await
        .unwrap();
    assert_eq!(kept.len(), 1, "the reading is kept");
    assert!(kept[0].reacted_at.is_none());
    let alerts = wl_kernel::compliance::ComplianceAlert::query()
        .where_(wl_kernel::compliance::ComplianceAlert::RULE.eq("FNB.RUL.05"))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(alerts.len(), 1, "the owner is alerted once");
}

/// AC6 (return to supplier): the chef decides a line of yoghurt received from
/// a supplier *return to supplier* with the reading and the minutes: the
/// quantity goes to quarantine onto a draft Return to supplier of the lot's
/// supplier, the decision is noted, and the line is decided.
#[renox::test]
async fn the_chef_returns_a_held_lot_to_its_supplier() {
    let w = boot_cold().await;
    let k = kitchen(&w).await;
    let f = fridge(&w, &k).await;
    let supplier = w.customer("Al Ain Dairy").await;
    let mut receipt = wl_inventory::GoodsReceipt {
        supplier_party_id: supplier.0,
        location_id: f.location.id,
        ..wl_inventory::GoodsReceipt::default()
    };
    receipt.save(w.db()).await.unwrap();
    let mut yoghurt = Item::new("YOGHURT", "Yoghurt", "kg").lot_tracked();
    yoghurt.average_cost = 500;
    yoghurt.save(w.db()).await.unwrap();
    let mut lot = wl_inventory::Lot {
        item_id: yoghurt.id,
        number: "Y-1".to_owned(),
        source_type: Some(receipt.subject().subject_type),
        source_id: Some(receipt.id),
        status: wl_inventory::LotStatus::Released,
        ..wl_inventory::Lot::default()
    };
    lot.save(w.db()).await.unwrap();
    let mut moved = StockMovement::receive(
        yoghurt.id,
        Some(lot.id),
        f.location.id,
        4_000,
        500,
        value_of(4_000, 500),
        &receipt.subject(),
        None,
    );
    moved.save(w.db()).await.unwrap();
    read(&w, &f.logger, &f.unit, "9").await.assert_status(303);
    let hold = temperature_holds(&w).await.remove(0);
    let line = lines_of(&w, &hold)
        .await
        .into_iter()
        .find(|l| l.item_id == yoghurt.id)
        .expect("the yoghurt is held");

    w.post(
        &k.chef,
        &format!("/cold-units/{}/lines/{}/decide", f.unit.id, line.id),
        &[
            ("decision", "return_to_supplier"),
            ("reading", "9"),
            ("minutes", "40"),
        ],
    )
    .await
    .assert_status(303);

    let decided = StockHoldLine::find_or_404(w.db(), line.id).await.unwrap();
    assert_eq!(decided.status, HoldLineStatus::ReturnedToSupplier);
    let note = HoldDecisionRecord::of_line(w.db(), line.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(note.decision, "return_to_supplier");
    assert_eq!((note.reading_tenths, note.minutes_out), (90, 40));
    let drafts = wl_inventory::supplier_return::SupplierReturn::query()
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].supplier_party_id, supplier.0);
    assert_eq!(
        drafts[0].status,
        wl_inventory::supplier_return::ReturnStatus::Draft
    );
    let listed = wl_inventory::supplier_return::lines_of(w.db(), drafts[0].id)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].lot_id, Some(lot.id));
    assert_eq!(listed[0].quantity_milli, 4_000);
}

/// AC1 (real ticket service): with the real wl-ticketing the out-of-range
/// reading opens one real ticket for the unit, a second reading joins it
/// instead of opening another, and the unit remembers its number.
#[renox::test]
async fn the_equipment_ticket_is_a_real_ticket_and_a_second_reading_joins_it() {
    let w = boot_real_tickets(
        own_riders(),
        Fnb::new(),
        WlPayments::new(),
        WlInventory::new(),
    )
    .await;
    let k = kitchen(&w).await;
    let f = fridge(&w, &k).await;

    read(&w, &f.logger, &f.unit, "9").await.assert_status(303);
    read(&w, &f.logger, &f.unit, "8").await.assert_status(303);

    let key = format!("fnb.cold_unit:{}", f.unit.id);
    let ticket = wl_kernel::tickets::task_creator(w.state())
        .find_open(w.state(), &key)
        .await
        .unwrap()
        .expect("a real ticket is open for the unit");
    let real = wl_ticketing::Ticket::query().get(w.db()).await.unwrap();
    assert_eq!(real.len(), 1, "one ticket for both readings");
    assert_eq!(real[0].number, ticket.number);
    assert_eq!(real[0].subject_type, "fnb.cold_unit");
    assert_eq!(real[0].subject_id, f.unit.id);
    assert!(real[0].title.contains("Fridge 2"), "{}", real[0].title);
    let unit = ColdUnit::find_or_404(w.db(), f.unit.id).await.unwrap();
    assert_eq!(unit.ticket_number.as_deref(), Some(ticket.number.as_str()));
    assert!(w.tickets.opened().is_empty(), "the fake desk was not used");
}
