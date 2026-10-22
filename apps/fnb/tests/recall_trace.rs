//! BL-FNB-13 (#377), 06 F&B §4.4 recall row, §7.2 *Recall owner*, §9
//! *Recalls*: a recall of an ingredient lot holds the production batch made
//! from it and lists the orders of the outlets that had the batch's lot on hand
//! as *possible* (never confirmed), grouped per customer for direct orders and
//! per outlet and day for aggregator and counter sales; each customer gets one
//! notice listing all their possible orders, and the owner decides the public
//! notice and the aggregators. The modules are the real wl-inventory (lots,
//! recalls, holds), wl-sales, wl-payments and wl-delivery.

mod support;

use fnb::recall::{self, NOTICE_TYPE};
use fnb::{BatchStatus, Fnb, FnbOrderStatus, MenuItem, RecipeLine};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_inventory::permissions as iperm;
use wl_inventory::transfer::{NewTransfer, NewTransferLine};
use wl_inventory::{LotStatus, MovementType, Recall, StockMovement, WlInventory};
use wl_kernel::PartyId;
use wl_kernel::inventory::TraceMark;
use wl_kernel::notify::NotifyRecipient;
use wl_payments::WlPayments;

use support::payouts::completed;
use support::stock::{Kitchen, finish, kitchen, lot, made_lot, plan, reload, start};
use support::{World, boot, boot_all, make_user, own_riders};

/// One shawarma plate, zero-rated.
const PLATE: [(&str, &str, &str); 1] = [("shawarma-plate", "1", "30.00")];

/// The 06 client's values: the recipe is consumed on completion, and the
/// recall page of the stock module links to the F&B trace page.
async fn boot_recall() -> World {
    boot_all(
        own_riders(),
        Fnb::new().recipe_consumption(true),
        WlPayments::new(),
        WlInventory::new().recall_notices_route("fnb.recall_trace"),
    )
    .await
}

/// Every plate uses 50 g of the sauce.
async fn sauce_in_the_plate(w: &World, k: &Kitchen) {
    let plate = MenuItem::by_code(w.db(), "shawarma-plate")
        .await
        .unwrap()
        .expect("the demo menu");
    let mut line = RecipeLine {
        menu_item_id: plate.id,
        item_id: k.sauce.id,
        per_unit_milli: 50,
        ..RecipeLine::default()
    };
    line.insert(w.db()).await.unwrap();
}

/// A direct order of `customer` through the phone, delivered by the rider.
async fn delivered(w: &World, customer: PartyId) -> fnb::FnbOrder {
    let (order, delivery, _run) = w
        .dispatched_via("phone", customer, "cash_on_delivery", &PLATE)
        .await;
    w.deliver_stop(&delivery, "30.00").await.assert_ok();
    let done = w.reload(&order).await;
    assert_eq!(done.status, FnbOrderStatus::Completed);
    done
}

/// The story of the issue: tomato lot T7 is received at the central kitchen,
/// batch B2 is made from it (10 kg) and 6 kg of its lot go to Outlet 1 (the
/// Marina kitchen). Over four days Outlet 1 completes an aggregator order each
/// day and direct orders of customer C (two) and customer D (one); on the
/// fourth day the sauce is used up. Then the head chef opens the recall of T7.
struct Story {
    k: Kitchen,
    t7: wl_inventory::Lot,
    batch: fnb::ProductionBatch,
    c: PartyId,
    d: PartyId,
    c_orders: Vec<fnb::FnbOrder>,
    recall: Recall,
}

async fn story(w: &World) -> Story {
    let k = kitchen(w).await;
    sauce_in_the_plate(w, &k).await;
    let t7 = lot(w, &k.tomato, "T7", &k.central, 30_000, 20).await;
    let batch = plan(w, &k).await;
    start(w, &k.chef, &k, &batch, &t7).await.assert_status(303);
    finish(w, &k.chef, &batch, "10", 10).await.assert_status(303);
    let l20 = made_lot(w, &batch).await;
    // 6 kg of B2's lot go to Outlet 1.
    let mover = make_user(w.db(), "Tariq", &[iperm::STOCK_TRANSFER, iperm::STOCK_VIEW]).await;
    let transfer = wl_inventory::transfer::create_draft(
        w.db(),
        None,
        NewTransfer {
            from_location_id: k.central.id,
            to_location_id: w.store.id,
            lines: vec![NewTransferLine {
                item_id: k.sauce.id,
                lot_id: Some(l20.id),
                quantity_milli: 6_000,
            }],
        },
    )
    .await
    .unwrap();
    w.post(&mover, &format!("/transfers/{}/send", transfer.id), &[]).await;
    w.post(&mover, &format!("/transfers/{}/receive", transfer.id), &[]).await;

    let c = w.customer("Customer C").await;
    let d = w.customer("Customer D").await;
    let mut c_orders = Vec::new();
    for day in 0..4 {
        if day > 0 {
            w.travel(TimeDelta::days(1));
        }
        completed(w, &format!("TB-70{day}"), "30.00").await;
        match day {
            0 | 2 => c_orders.push(delivered(w, c).await),
            1 => {
                delivered(w, d).await;
            }
            _ => {}
        }
    }
    // The sauce is used up at Outlet 1 on the last day.
    for (place, left) in wl_inventory::balance::lot_locations(w.db(), l20.id).await.unwrap() {
        if place == w.store.id {
            let mut gone = StockMovement::leaving(
                MovementType::Wastage,
                k.sauce.id,
                Some(l20.id),
                place,
                left,
                1_500,
                0,
                &wl_kernel::Subject::new("test.use_up", 1),
                Some("used up".to_owned()),
                None,
            );
            gone.save(w.db()).await.unwrap();
        }
    }
    w.travel(TimeDelta::hours(2));
    let items = k.tomato.id.to_string();
    w.post(
        &k.chef,
        "/recalls",
        &[
            ("source", "supplier"),
            ("reason", "The supplier found contamination in tomato lot T7"),
            ("items", items.as_str()),
            ("lot_numbers", "T7"),
        ],
    )
    .await;
    let recall = Recall::query()
        .order_by_desc(Recall::ID)
        .first(w.db())
        .await
        .unwrap()
        .expect("the recall is opened");
    Story {
        k,
        t7,
        batch,
        c,
        d,
        c_orders,
        recall,
    }
}

/// AC1: a recall of T7 holds B2's lot and lists the direct orders of Outlet 1
/// whose recipes contain B2's item as *possible*, grouped per customer; no
/// order is ever confirmed.
#[renox::test]
async fn a_recall_holds_the_batch_and_lists_the_possible_orders_per_customer() {
    let w = boot_recall().await;
    let s = story(&w).await;

    // B2 and its lot are held by the recall.
    assert_eq!(reload(&w, &s.batch).await.status, BatchStatus::OnHold);
    assert_eq!(made_lot(&w, &s.batch).await.status, LotStatus::OnHold);
    assert_eq!(
        wl_inventory::Lot::find_or_404(w.db(), s.t7.id).await.unwrap().status,
        LotStatus::OnHold
    );

    let found = recall::trace(w.state(), s.recall.id).await.unwrap();
    assert_eq!(found.customers.len(), 2, "customers C and D");
    let c = found.customers.iter().find(|g| g.party == s.c.0).unwrap();
    let d = found.customers.iter().find(|g| g.party == s.d.0).unwrap();
    assert_eq!(c.orders.len(), 2, "C's two possible orders");
    assert_eq!(d.orders.len(), 1);
    let numbers: Vec<_> = c.orders.iter().map(|o| o.number.clone()).collect();
    for order in &s.c_orders {
        assert!(numbers.contains(&order.number));
    }
    assert!(
        found
            .customers
            .iter()
            .flat_map(|g| g.orders.iter())
            .all(|o| o.mark == TraceMark::Possible),
        "an order is never confirmed: the lots of a recipe are not recorded"
    );
}

/// AC2: the aggregator orders of Outlet 1 on those days appear as 4
/// unknown-customer groups and the owner gets the decision task; no
/// aggregator customer is named or messaged.
#[renox::test]
async fn aggregator_sales_are_four_groups_and_the_owner_gets_the_decision_task() {
    let w = boot_recall().await;
    let s = story(&w).await;

    let found = recall::trace(w.state(), s.recall.id).await.unwrap();
    assert_eq!(found.unknown.len(), 4, "one group per outlet and day");
    assert!(found.unknown.iter().all(|g| g.sales == 1));
    assert!(found.unknown.iter().all(|g| g.channels.contains("aggregator_a")));
    assert!(found.unknown.iter().all(|g| g.notice == "pending"));
    let tasks = w.tickets.opened_with(&recall::public_task_key(s.recall.id));
    assert_eq!(tasks.len(), 1, "the owner gets one decision task");
    assert!(
        found.customers.iter().all(|g| ["Customer C", "Customer D"].contains(&g.name.as_str())),
        "no aggregator customer is listed as a customer"
    );
}

/// AC3: customer C with 2 possible orders gets one notice listing both; the
/// aggregators' customers are never messaged.
#[renox::test]
async fn a_customer_with_two_possible_orders_gets_one_notice() {
    let w = boot_recall().await;
    let s = story(&w).await;

    w.post(
        &s.k.chef,
        &format!("/recalls/{}/orders/notify", s.recall.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);

    let sent = w.notices.of_type(NOTICE_TYPE);
    assert_eq!(sent.len(), 2, "one notice each for C and D");
    let to_c: Vec<_> = sent
        .iter()
        .filter(|request| {
            request
                .recipients
                .contains(&NotifyRecipient::Party { id: s.c })
        })
        .collect();
    assert_eq!(to_c.len(), 1, "C gets one notice");
    let orders = to_c[0].data["orders"].as_str().unwrap_or_default().to_owned();
    for order in &s.c_orders {
        assert!(orders.contains(&order.number), "{orders} lists {}", order.number);
    }
    for request in &sent {
        assert!(request.recipients.iter().all(|r| matches!(
            r,
            NotifyRecipient::Party { id } if *id == s.c || *id == s.d
        )));
    }
    let after = recall::trace(w.state(), s.recall.id).await.unwrap();
    assert!(after
        .customers
        .iter()
        .all(|g| g.notice() == "sent"), "every line of a told customer is sent");
    assert_eq!(after.notices.len(), 2);
    // Telling again has nobody left to tell.
    w.post(
        &s.k.chef,
        &format!("/recalls/{}/orders/notify", s.recall.id),
        &[("x", "")],
    )
    .await
    .assert_status(422);
}

/// Closing the owner's task records the decision: with no public notice the
/// unknown-customer lines are waived with it; with a public notice they are
/// told. It is decided once, by a holder of `fnb_recalls.decide`.
#[renox::test]
async fn the_owner_decides_the_public_notice_and_the_lines_are_settled() {
    let w = boot_recall().await;
    let s = story(&w).await;
    let path = format!("/recalls/{}/orders/public-notice", s.recall.id);

    // The kitchen manager reads the trace but does not decide.
    w.get(&s.k.manager, &format!("/recalls/{}/orders", s.recall.id))
        .await
        .assert_ok();
    w.post(&s.k.manager, &path, &[("decision", "no_public_notice"), ("note", "x")])
        .await
        .assert_status(403);
    // No public notice needs the owner's reason.
    w.post(&s.k.owner, &path, &[("decision", "no_public_notice")])
        .await
        .assert_status(422);
    w.post(&s.k.owner, &path, &[("decision", "maybe"), ("note", "x")])
        .await
        .assert_status(422);

    w.post(
        &s.k.owner,
        &path,
        &[
            ("decision", "no_public_notice"),
            ("note", "Only cooked meals, none left after four days"),
        ],
    )
    .await
    .assert_status(303);

    let found = recall::trace(w.state(), s.recall.id).await.unwrap();
    assert!(found.unknown.iter().all(|g| g.notice == "waived"));
    let decision = found.decision.expect("the decision is recorded");
    assert_eq!(decision.decision, "no_public_notice");
    assert!(!w.tickets.closed().is_empty(), "the owner's task is closed");
    // Once only.
    w.post(&s.k.owner, &path, &[("decision", "public_notice")])
        .await
        .assert_status(422);
}

/// A public notice marks the unknown-customer lines told and the aggregators
/// told is recorded.
#[renox::test]
async fn a_public_notice_marks_the_unknown_customer_lines_told() {
    let w = boot_recall().await;
    let s = story(&w).await;

    w.post(
        &s.k.owner,
        &format!("/recalls/{}/orders/public-notice", s.recall.id),
        &[("decision", "public_notice"), ("aggregators", "1")],
    )
    .await
    .assert_status(303);

    let found = recall::trace(w.state(), s.recall.id).await.unwrap();
    assert!(found.unknown.iter().all(|g| g.notice == "sent"));
    assert!(found.decision.expect("decided").aggregators_told);
}

/// Who may read the trace: the head chef, the owner and the kitchen manager;
/// not a stranger or a cook.
#[renox::test]
async fn only_the_kitchen_leaders_read_the_trace() {
    let w = boot_recall().await;
    let s = story(&w).await;
    let path = format!("/recalls/{}/orders", s.recall.id);

    for reader in [&s.k.chef, &s.k.owner, &s.k.manager] {
        w.get(reader, &path).await.assert_ok();
    }
    w.get(&w.nobody, &path).await.assert_status(403);
    w.get(&w.cook, &path).await.assert_status(403);
    w.post(&w.nobody, &format!("{path}/notify"), &[("x", "")])
        .await
        .assert_status(403);
}

/// The ingredient lot's own recall page links to the F&B page for the
/// customers (the stock module's `recall_notices_route`).
#[renox::test]
async fn the_recall_page_of_the_stock_module_links_to_the_customers_page() {
    let w = boot_recall().await;
    let s = story(&w).await;

    let page = w
        .get(&s.k.chef, &format!("/recalls/{}", s.recall.id))
        .await;
    page.assert_ok();
    assert!(
        page.text().contains(&format!("/recalls/{}/orders", s.recall.id)),
        "{}",
        page.text()
    );
}

/// Without the setting a completed order consumes nothing: the library's
/// default leaves the stock as it was (the 06 client turns the setting on).
#[renox::test]
async fn a_completed_order_consumes_its_recipe_only_when_the_setting_is_on() {
    let off = boot().await;
    let k = kitchen(&off).await;
    sauce_in_the_plate(&off, &k).await;
    let done = completed(&off, "TB-900", "30.00").await;
    let movements = StockMovement::query()
        .where_(StockMovement::SOURCE_TYPE.eq("fnb.order"))
        .where_(StockMovement::SOURCE_ID.eq(done.id))
        .get(off.db())
        .await
        .unwrap();
    assert!(movements.is_empty(), "nothing is consumed by default");
    assert!(!off.reload(&done).await.consumption_posted);

    let on = boot_recall().await;
    let k = kitchen(&on).await;
    sauce_in_the_plate(&on, &k).await;
    let done = completed(&on, "TB-901", "30.00").await;
    let movements = StockMovement::query()
        .where_(StockMovement::SOURCE_TYPE.eq("fnb.order"))
        .where_(StockMovement::SOURCE_ID.eq(done.id))
        .where_(StockMovement::THEORETICAL.eq(true))
        .get(on.db())
        .await
        .unwrap();
    assert!(!movements.is_empty(), "the recipe is consumed, theoretically");
    assert!(on.reload(&done).await.consumption_posted);
}
