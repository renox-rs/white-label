//! BL-AUT-15 (#411) and BL-SAL-17 (#136) against the REAL shared modules of
//! the host (wl-sales, wl-inventory, wl-notify): in the 08 deployment the
//! counter, phone and sales rep channels back-order a line without stock, the
//! reorder suggestion adds the back-ordered quantity, a posted goods receipt
//! reserves the arriving stock for the oldest back-orders and tells each
//! customer once, a reversed receipt back-orders the lines again, and an order
//! on `credit_hold` is skipped until it is released. `release` (BL-SAL-11) is
//! not built on this branch: its seam `back_orders::order_released` is called.

mod support;

use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{auto_booted, company, count_messages, person};
use wl_inventory::movement::StockMovement;
use wl_inventory::{
    DraftLine, Item, Location, PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus,
    ReorderSuggestion, reorder,
};
use wl_kernel::{PartyId, Subject};
use wl_sales::sale_line::NewLine;
use wl_sales::sales_orders::{self, NewSalesOrder};
use wl_sales::{SaleLine, SalesOrder};

/// The booted workshop with the signed-in administrator, a store, the part P
/// (no reorder point) and the customer.
struct World {
    app: TestApp,
    store: Location,
    part: Item,
    customer: PartyId,
    supplier: PartyId,
}

async fn world() -> World {
    let app = auto_booted().await;
    let admin = person(&app, "Omar", host::roles::ADMIN).await;
    app.acting_as(&admin);
    let customer = company(&app, "Gulf Fleet LLC").await;
    let supplier = company(&app, "Brake Parts FZE").await;
    let mut part = Item::new("P", "Brake pad set", "pcs");
    part.save(app.db()).await.unwrap();
    let mut store = Location::new("STORE-1", "Branch store");
    store.save(app.db()).await.unwrap();
    World {
        app,
        store,
        part,
        customer,
        supplier,
    }
}

impl World {
    /// `units` of P on hand at the store.
    async fn stock(&self, units: i64) {
        let mut moved = StockMovement::receive(
            self.part.id,
            None,
            self.store.id,
            units * 1000,
            800,
            units * 800,
            &Subject::new("inventory.goods_receipt", 900),
            None,
        );
        moved.save(self.app.db()).await.unwrap();
    }

    /// A submitted order on `channel` with `lines` (code, units).
    async fn order(&self, channel: &str, lines: &[(&str, i64)]) -> SalesOrder {
        let order = sales_orders::create(
            self.app.state(),
            None,
            NewSalesOrder::new(self.customer).channel(channel),
        )
        .await
        .unwrap();
        for (code, units) in lines {
            sales_orders::add_line(
                self.app.state(),
                order.id,
                NewLine::new(code, 5_000).quantity(*units),
            )
            .await
            .unwrap();
        }
        self.app
            .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
            .await
            .assert_status(303);
        SalesOrder::find_or_404(self.app.db(), order.id)
            .await
            .unwrap()
    }

    /// `(reserved, back-ordered)` units of line `n` of `order`.
    async fn split(&self, order: &SalesOrder, n: i64) -> (i64, i64) {
        let line = SaleLine::of(self.app.db(), &order.subject())
            .await
            .unwrap()
            .into_iter()
            .find(|l| l.line_no == n)
            .unwrap();
        (line.reserved_milli / 1000, line.backordered_milli / 1000)
    }

    /// An issued purchase order for `units` of P at the store.
    async fn purchase(&self, units: i64) -> (PurchaseOrder, PurchaseOrderLine) {
        let mut order = PurchaseOrder::new(self.supplier, self.store.id);
        order.status = PurchaseOrderStatus::Issued;
        let mut tx = self.app.db().begin().await.unwrap();
        order
            .insert_in_state(&mut tx, PurchaseOrderStatus::Issued, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let mut line = PurchaseOrderLine::new(order.id, 1, self.part.id, units * 1000, 800);
        line.save(self.app.db()).await.unwrap();
        (order, line)
    }

    /// Posts a goods receipt of `units` of P on a new purchase order.
    async fn receive(&self, units: i64, note: &str) {
        let (po, line) = self.purchase(units).await;
        let receipt = wl_inventory::goods_receipt::create_draft(
            self.app.db(),
            None,
            wl_inventory::DraftReceipt {
                purchase_order_id: po.id,
                location_id: self.store.id,
                delivery_note: Some(note.into()),
                lines: vec![DraftLine::on(line.id, units * 1000, units * 1000)],
            },
        )
        .await
        .unwrap();
        self.app
            .htmx()
            .post(&format!("/goods-receipts/{}/post", receipt.id), &[])
            .await
            .assert_ok();
    }

    async fn suggestion(&self) -> Option<ReorderSuggestion> {
        ReorderSuggestion::query()
            .where_(ReorderSuggestion::ITEM_ID.eq(self.part.id))
            .first(self.app.db())
            .await
            .unwrap()
    }
}

/// BL-AUT-15 point 1: back-orders are on for the counter, phone and sales rep
/// channels and off for the portal.
#[renox::test]
async fn aut_15_real_back_orders_are_on_for_counter_phone_and_sales_rep_and_off_for_the_portal() {
    let w = world().await;
    w.stock(2).await;
    for channel in ["counter", "phone", "sales_rep"] {
        let order = w.order(channel, &[("P", 3)]).await;
        let (reserved, back) = w.split(&order, 1).await;
        assert_eq!(reserved + back, 3, "{channel}");
        assert!(back > 0, "{channel}");
    }
    let portal = w.order("portal", &[("P", 3)]).await;
    assert_eq!(w.split(&portal, 1).await, (0, 0));
}

/// BL-AUT-15 point 2 and BL-SAL-17 point 1: 2 P in stock, 5 P ordered on the
/// counter: 2 reserved, 3 back-ordered, and the 07:00 suggestion for P is 3;
/// with an open purchase order for 1 it is 2.
#[renox::test]
async fn aut_15_real_the_suggestion_is_the_back_ordered_quantity_less_what_is_on_order() {
    let w = world().await;
    w.stock(2).await;
    let order = w.order("counter", &[("P", 5)]).await;
    assert_eq!(w.split(&order, 1).await, (2, 3));

    w.app.run_scheduled(reorder::TASK).await.unwrap();
    let first = w.suggestion().await.expect("a suggestion for P");
    assert_eq!(first.suggested_qty_milli, 3_000);
    assert_eq!(first.location_id, w.store.id);

    w.purchase(1).await;
    w.app.run_scheduled(reorder::TASK).await.unwrap();
    assert_eq!(w.suggestion().await.unwrap().suggested_qty_milli, 2_000);
}

/// BL-AUT-15 points 3 and 4 and BL-SAL-17 points 3, 6 and 7: SO-1 and SO-2
/// each back-order 3 P; a posted receipt of 4 P reserves 3 and 1 and both
/// customers are told once; cancelling SO-2's back-order drops it from the
/// suggestion; reversing the receipt back-orders the lines again.
#[renox::test]
async fn aut_15_real_a_posted_receipt_serves_the_oldest_back_order_and_a_reversed_one_undoes_it() {
    let w = world().await;
    let one = w.order("phone", &[("P", 3), ("Q", 1)]).await;
    let two = w.order("phone", &[("P", 3), ("Q", 1)]).await;
    assert_eq!(w.split(&one, 1).await, (0, 3));
    let (po, line) = w.purchase(4).await;
    let receipt = wl_inventory::goods_receipt::create_draft(
        w.app.db(),
        None,
        wl_inventory::DraftReceipt {
            purchase_order_id: po.id,
            location_id: w.store.id,
            delivery_note: Some("DN-1".into()),
            lines: vec![DraftLine::on(line.id, 4_000, 4_000)],
        },
    )
    .await
    .unwrap();

    w.app
        .htmx()
        .post(&format!("/goods-receipts/{}/post", receipt.id), &[])
        .await
        .assert_ok();
    assert_eq!(w.split(&one, 1).await, (3, 0));
    assert_eq!(w.split(&two, 1).await, (1, 2));
    assert_eq!(count_messages(&w.app, "sales.back_order_reserved").await, 2);

    w.app
        .htmx()
        .post(&format!("/goods-receipts/{}/reverse", receipt.id), &[])
        .await
        .assert_ok();
    assert_eq!(w.split(&one, 1).await, (0, 3));
    assert_eq!(w.split(&two, 1).await, (0, 3));

    // The reversal reopens the purchase order for its 4: of the 6 P back-ordered
    // 2 are not covered by it.
    w.app.run_scheduled(reorder::TASK).await.unwrap();
    assert_eq!(w.suggestion().await.unwrap().suggested_qty_milli, 2_000);

    // SO-2's back-order is cancelled (its other line stays open): the 3 P of
    // SO-1 are covered by the reopened order, so the suggestion goes.
    w.app
        .post(
            &format!("/sales-orders/{}/cancel-line", two.id),
            &[
                ("line_no", "1"),
                ("quantity", "3"),
                ("reason", "not needed"),
            ],
        )
        .await
        .assert_status(303);
    assert_eq!(w.split(&two, 1).await, (0, 0));
    w.app.run_scheduled(reorder::TASK).await.unwrap();
    assert!(
        w.suggestion().await.is_none(),
        "SO-1's 3 P are on the reopened order"
    );
}

/// BL-AUT-15 points 6 to 8 and BL-SAL-17 points 4 and 5: an order on
/// `credit_hold` is skipped by a receipt and served at `release` from the
/// stock then available; lines confirmed in the same second go by order
/// number.
#[renox::test]
async fn aut_15_real_credit_hold_is_skipped_and_the_same_second_goes_by_order_number() {
    let w = world().await;
    let held = w.order("phone", &[("P", 3), ("Q", 1)]).await;
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(held.id))
        .update(w.app.db(), &[("status", &"credit_hold".to_owned())])
        .await
        .unwrap();
    let three = w.order("phone", &[("P", 1), ("Q", 1)]).await;
    let four = w.order("phone", &[("P", 1), ("Q", 1)]).await;
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(four.id))
        .update(w.app.db(), &[("confirmed_at", &three.confirmed_at)])
        .await
        .unwrap();

    // A receipt of 1 P: the held order is skipped, the lower number wins.
    let (po, line) = w.purchase(1).await;
    let receipt = wl_inventory::goods_receipt::create_draft(
        w.app.db(),
        None,
        wl_inventory::DraftReceipt {
            purchase_order_id: po.id,
            location_id: w.store.id,
            delivery_note: Some("DN-2".into()),
            lines: vec![DraftLine::on(line.id, 1_000, 1_000)],
        },
    )
    .await
    .unwrap();
    w.app
        .htmx()
        .post(&format!("/goods-receipts/{}/post", receipt.id), &[])
        .await
        .assert_ok();
    assert_eq!(w.split(&held, 1).await, (0, 3));
    assert_eq!(w.split(&three, 1).await, (1, 0));
    assert_eq!(w.split(&four, 1).await, (0, 1));

    // Released with 4 P unreserved at the branch: 3 are reserved at once.
    w.stock(4).await;
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(held.id))
        .update(w.app.db(), &[("status", &"confirmed".to_owned())])
        .await
        .unwrap();
    let reserved = wl_sales::back_orders::order_released(w.app.state(), held.id)
        .await
        .unwrap();
    assert_eq!(reserved, 3_000);
    assert_eq!(w.split(&held, 1).await, (3, 0));

    // The line SO-4 still lacks is served by the next receipt, and the released
    // order keeps what it got.
    assert_eq!(w.split(&four, 1).await, (0, 1), "SO-4 still lacks its P");
    w.receive(1, "DN-3").await;
    assert_eq!(w.split(&four, 1).await, (1, 0));
    assert_eq!(w.split(&held, 1).await, (3, 0));
}

/// BL-SAL-17 point 5: SO-4 with lines 1 and 2 and SO-5 with line 1, all of P and
/// confirmed in the same second; a receipt of 2 P serves SO-4 line 1 and line
/// 2 and not SO-5.
#[renox::test]
async fn aut_15_real_the_same_second_is_served_by_order_then_line_number() {
    let w = world().await;
    let four = w.order("counter", &[("P", 1), ("P", 1)]).await;
    let five = w.order("counter", &[("P", 1)]).await;
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(five.id))
        .update(w.app.db(), &[("confirmed_at", &four.confirmed_at)])
        .await
        .unwrap();
    assert_eq!(w.split(&four, 1).await, (0, 1));
    assert_eq!(w.split(&five, 1).await, (0, 1));

    w.receive(2, "DN-4").await;

    assert_eq!(w.split(&four, 1).await, (1, 0), "SO-4 line 1");
    assert_eq!(w.split(&four, 2).await, (1, 0), "SO-4 line 2");
    assert_eq!(w.split(&five, 1).await, (0, 1), "SO-5 waits");
}

/// BL-SAL-17 point 2: an item with a reorder point (min 5, max 20) and a
/// preferred supplier. Above the point (10 on hand, SO of 12: 2 back-ordered and
/// not on a purchase order) the suggestion is the 2; at or below it (4 on hand,
/// SO of 6: 2 back-ordered) it is up to the maximum plus the 2.
#[renox::test]
async fn aut_15_real_a_reorder_point_adds_the_back_order_on_top_of_the_policy() {
    let w = world().await;
    let mut above = Item::new("R", "Wiper blade", "pcs");
    above.reorder_point_milli = 5_000;
    above.reorder_max_milli = 20_000;
    above.save(w.app.db()).await.unwrap();
    let mut below = Item::new("S", "Spark plug", "pcs");
    below.reorder_point_milli = 5_000;
    below.reorder_max_milli = 20_000;
    below.save(w.app.db()).await.unwrap();
    for item in [&above, &below] {
        let mut row = wl_inventory::ItemSupplier {
            item_id: item.id,
            supplier_party_id: w.supplier.0,
            role: wl_inventory::SupplierRole::Preferred,
            ..wl_inventory::ItemSupplier::default()
        };
        row.save(w.app.db()).await.unwrap();
    }
    for (item, units) in [(&above, 10), (&below, 4)] {
        let mut moved = StockMovement::receive(
            item.id,
            None,
            w.store.id,
            units * 1000,
            800,
            units * 800,
            &Subject::new("inventory.goods_receipt", 900),
            None,
        );
        moved.save(w.app.db()).await.unwrap();
    }
    let a = w.order("counter", &[("R", 12)]).await;
    let b = w.order("counter", &[("S", 6)]).await;
    assert_eq!(w.split(&a, 1).await, (10, 2));
    assert_eq!(w.split(&b, 1).await, (4, 2));

    w.app.run_scheduled(reorder::TASK).await.unwrap();

    let quantity = |item: &Item| {
        let id = item.id;
        let db = w.app.db().clone();
        async move {
            ReorderSuggestion::query()
                .where_(ReorderSuggestion::ITEM_ID.eq(id))
                .first(&db)
                .await
                .unwrap()
                .map(|s| s.suggested_qty_milli)
        }
    };
    assert_eq!(
        quantity(&above).await,
        Some(2_000),
        "above the point: the 2"
    );
    assert_eq!(
        quantity(&below).await,
        Some(18_000),
        "at the point: up to 20 less the 4 on hand, plus the 2"
    );
}
