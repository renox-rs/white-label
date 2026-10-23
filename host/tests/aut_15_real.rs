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
        SalesOrder::find_or_404(self.app.db(), order.id).await.unwrap()
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

    // SO-2's back-order is cancelled (its other line stays open) and leaves
    // the suggestion.
    w.app
        .post(
            &format!("/sales-orders/{}/cancel-line", two.id),
            &[("line_no", "1"), ("quantity", "3"), ("reason", "not needed")],
        )
        .await
        .assert_status(303);
    assert_eq!(w.split(&two, 1).await, (0, 0));
    w.app.run_scheduled(reorder::TASK).await.unwrap();
    assert_eq!(w.suggestion().await.unwrap().suggested_qty_milli, 3_000);
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
}
