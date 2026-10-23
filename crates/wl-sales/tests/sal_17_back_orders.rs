//! BL-SAL-17 (#136): back-order lines without stock and serve them oldest
//! first (wl-sales §4.2 *Back-orders*). The inventory module is answered by a
//! stand-in of the kernel's `StockReservations`; `host/tests/sal_17_real.rs`
//! runs the real wl-inventory. `release` (BL-SAL-11) is not built on this
//! branch: its seam `back_orders::order_released` is called directly.

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::backorders::{
    ArrivedLine, BackOrderFuture, Reserved, ReserveStock, StockArrivalReversed, StockArrived,
    StockReservations, back_orders, provide_stock_reservations,
};
use wl_sales::permissions as perm;
use wl_sales::sale_line::NewLine;
use wl_sales::sales_orders::{self, NewSalesOrder};
use wl_sales::{SaleLine, SalesOrder, SalesOrderStatus, WlSales};

use support::shop::Shop;
use support::{Booted, boot_pair};

/// One hold: (order id, line, item code, milli).
type Hold = (i64, i64, String, i64);

/// The stock of item codes at one location, and what is set aside.
#[derive(Clone, Default)]
struct Stock {
    free: Arc<Mutex<BTreeMap<String, i64>>>,
    held: Arc<Mutex<Vec<Hold>>>,
}

impl Stock {
    fn add(&self, code: &str, milli: i64) {
        *self.free.lock().unwrap().entry(code.to_owned()).or_insert(0) += milli;
    }

    fn free_of(&self, code: &str) -> i64 {
        self.free.lock().unwrap().get(code).copied().unwrap_or(0)
    }
}

fn location() -> Subject {
    Subject::new("inventory.location", 1)
}

impl StockReservations for Stock {
    fn reserve<'a>(
        &'a self,
        _state: &'a AppState,
        request: ReserveStock,
    ) -> BackOrderFuture<'a, Reserved> {
        Box::pin(async move {
            let mut free = self.free.lock().unwrap();
            let have = free.entry(request.item_code.clone()).or_insert(0);
            let take = request.quantity_milli.min(*have).max(0);
            *have -= take;
            if take > 0 {
                self.held.lock().unwrap().push((
                    request.owner.subject_id,
                    request.line,
                    request.item_code.clone(),
                    take,
                ));
            }
            Ok(Reserved {
                reserved_milli: take,
                location: Some(location()),
            })
        })
    }

    fn release<'a>(
        &'a self,
        _state: &'a AppState,
        owner: &'a Subject,
        line: Option<i64>,
        quantity_milli: Option<i64>,
    ) -> BackOrderFuture<'a, i64> {
        Box::pin(async move {
            let mut held = self.held.lock().unwrap();
            let mut left = quantity_milli.unwrap_or(i64::MAX);
            let mut given = 0;
            for row in held.iter_mut().rev() {
                if row.0 != owner.subject_id || line.is_some_and(|l| l != row.1) || left <= 0 {
                    continue;
                }
                let take = row.3.min(left);
                row.3 -= take;
                left -= take;
                given += take;
                *self.free.lock().unwrap().entry(row.2.clone()).or_insert(0) += take;
            }
            Ok(given)
        })
    }
}

impl Module for Stock {
    fn name(&self) -> &'static str {
        "stock_stand_in"
    }

    fn register(&self, app: &mut Registry) {
        provide_stock_reservations(app, self.clone());
    }
}

async fn boot(sales: WlSales) -> (Booted, Shop, Stock) {
    let shop = Shop::default();
    let stock = Stock::default();
    let t = boot_pair(sales, shop.clone(), stock.clone()).await;
    let seller = t
        .user(
            "Sam",
            &[
                perm::SALES_ORDERS_VIEW,
                perm::SALES_ORDERS_CREATE,
                perm::SALES_ORDERS_CANCEL,
            ],
        )
        .await;
    t.app.acting_as(&seller);
    (t, shop, stock)
}

fn on() -> WlSales {
    WlSales::new().back_orders_for(&["counter", "phone", "sales_rep"])
}

/// A confirmed order on `channel` for party 7 with `lines` (code, units).
async fn confirmed(t: &Booted, channel: &str, lines: &[(&str, i64)]) -> SalesOrder {
    let order = sales_orders::create(
        t.state(),
        None,
        NewSalesOrder::new(wl_kernel::PartyId(7)).channel(channel),
    )
    .await
    .unwrap();
    for (code, units) in lines {
        sales_orders::add_line(t.state(), order.id, NewLine::new(code, 5_000).quantity(*units))
            .await
            .unwrap();
    }
    t.app
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
        .assert_status(303);
    reload(t, order.id).await
}

async fn reload(t: &Booted, id: i64) -> SalesOrder {
    SalesOrder::find_or_404(t.db(), id).await.unwrap()
}

async fn line(t: &Booted, order: &SalesOrder, n: i64) -> SaleLine {
    SaleLine::of(t.db(), &order.subject())
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.line_no == n)
        .unwrap()
}

/// `(reserved, back-ordered)` of line `n`, in units.
async fn split(t: &Booted, order: &SalesOrder, n: i64) -> (i64, i64) {
    let l = line(t, order, n).await;
    (l.reserved_milli / 1000, l.backordered_milli / 1000)
}

async fn arrive(t: &Booted, stock: &Stock, source: i64, code: &str, units: i64) {
    stock.add(code, units * 1000);
    t.state()
        .emit(StockArrived {
            source: Subject::new("inventory.goods_receipt", source),
            kind: "receive".to_owned(),
            location: location(),
            lines: vec![ArrivedLine {
                item: Subject::new("inventory.item", 1),
                item_code: code.to_owned(),
                quantity_milli: units * 1000,
            }],
        })
        .await
        .unwrap();
}

/// Done when 1: 2 P in stock and 5 P ordered: 2 reserved, 3 back-ordered, and
/// the 3 are the demand the 07:00 suggestion adds.
#[renox::test]
async fn sal_17_a_line_without_enough_stock_reserves_what_there_is_and_back_orders_the_rest() {
    let (t, _shop, stock) = boot(on()).await;
    stock.add("P", 2_000);
    let order = confirmed(&t, "counter", &[("P", 5)]).await;
    assert_eq!(order.status, SalesOrderStatus::Confirmed);
    assert_eq!(split(&t, &order, 1).await, (2, 3));
    assert_eq!(order.location_id, Some(1));
    let demand = back_orders(t.state()).open(t.state()).await.unwrap();
    assert_eq!(demand.len(), 1);
    assert_eq!(
        (demand[0].item_code.as_str(), demand[0].quantity_milli),
        ("P", 3_000)
    );
}

/// Done when 3: SO-1 (confirmed first) and SO-2 each back-order 3 P; a
/// receipt of 4 P reserves 3 for SO-1 and 1 for SO-2; each customer gets one
/// message.
#[renox::test]
async fn sal_17_arriving_stock_serves_the_oldest_back_order_first_and_tells_each_customer_once() {
    let (t, shop, stock) = boot(on()).await;
    let one = confirmed(&t, "phone", &[("P", 3), ("Q", 1)]).await;
    let two = confirmed(&t, "phone", &[("P", 3), ("Q", 1)]).await;
    arrive(&t, &stock, 10, "P", 4).await;
    assert_eq!(split(&t, &one, 1).await, (3, 0));
    assert_eq!(split(&t, &two, 1).await, (1, 2));
    assert_eq!(shop.mail.of_type("sales.back_order_reserved").len(), 2);
    // The same arrival twice reserves nothing more.
    arrive(&t, &stock, 10, "P", 0).await;
    assert_eq!(split(&t, &two, 1).await, (1, 2));
    assert_eq!(shop.mail.of_type("sales.back_order_reserved").len(), 2);
}

/// Done when 4: SO-3 on `credit_hold` (confirmed first) is skipped by a
/// receipt of 2 P that serves SO-1; released with 1 P available, 1 P is
/// reserved for it at once and the other waits for the next receipt.
#[renox::test]
async fn sal_17_an_order_on_credit_hold_is_skipped_and_served_at_release() {
    let (t, _shop, stock) = boot(on()).await;
    let held = confirmed(&t, "phone", &[("P", 2), ("Q", 1)]).await;
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(held.id))
        .update(t.db(), &[("status", &"credit_hold".to_owned())])
        .await
        .unwrap();
    let one = confirmed(&t, "phone", &[("P", 3), ("Q", 1)]).await;
    arrive(&t, &stock, 11, "P", 2).await;
    assert_eq!(split(&t, &one, 1).await, (2, 1));
    assert_eq!(split(&t, &held, 1).await, (0, 2));

    stock.add("P", 1_000);
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(held.id))
        .update(t.db(), &[("status", &"confirmed".to_owned())])
        .await
        .unwrap();
    let reserved = wl_sales::back_orders::order_released(t.state(), held.id)
        .await
        .unwrap();
    assert_eq!(reserved, 1_000);
    assert_eq!(split(&t, &held, 1).await, (1, 1));
}

/// Done when 5: lines confirmed in the same second are served by order
/// number, then line number.
#[renox::test]
async fn sal_17_the_same_second_is_served_by_order_number_then_line_number() {
    let (t, _shop, stock) = boot(on()).await;
    let four = confirmed(&t, "phone", &[("P", 1), ("P", 1), ("Q", 1)]).await;
    let five = confirmed(&t, "phone", &[("P", 1), ("Q", 1)]).await;
    let at = four.confirmed_at;
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(five.id))
        .update(t.db(), &[("confirmed_at", &at)])
        .await
        .unwrap();
    arrive(&t, &stock, 12, "P", 2).await;
    assert_eq!(split(&t, &four, 1).await, (1, 0));
    assert_eq!(split(&t, &four, 2).await, (1, 0));
    assert_eq!(split(&t, &five, 1).await, (0, 1));
}

/// Done when 6: `cancel_line` on a back-order needs no outcome and the
/// quantity leaves the demand.
#[renox::test]
async fn sal_17_cancelling_a_back_order_removes_it_from_the_demand() {
    let (t, _shop, _stock) = boot(on()).await;
    let two = confirmed(&t, "phone", &[("P", 3), ("Q", 1)]).await;
    assert_eq!(split(&t, &two, 1).await, (0, 3));
    t.app
        .post(
            &format!("/sales-orders/{}/cancel-line", two.id),
            &[("line_no", "1"), ("quantity", "3"), ("reason", "not needed")],
        )
        .await
        .assert_status(303);
    assert_eq!(split(&t, &two, 1).await, (0, 0));
    let demand = back_orders(t.state()).open(t.state()).await.unwrap();
    assert!(demand.iter().all(|d| d.item_code != "P"));
}

/// Done when 7: reversing the receipt releases the reservations and both
/// lines are back-ordered again.
#[renox::test]
async fn sal_17_a_reversed_receipt_gives_the_reservations_back() {
    let (t, _shop, stock) = boot(on()).await;
    let one = confirmed(&t, "phone", &[("P", 3), ("Q", 1)]).await;
    let two = confirmed(&t, "phone", &[("P", 3), ("Q", 1)]).await;
    arrive(&t, &stock, 13, "P", 4).await;
    t.state()
        .emit(StockArrivalReversed {
            source: Subject::new("inventory.goods_receipt", 13),
            location: location(),
            lines: vec![ArrivedLine {
                item: Subject::new("inventory.item", 1),
                item_code: "P".to_owned(),
                quantity_milli: 4_000,
            }],
        })
        .await
        .unwrap();
    assert_eq!(split(&t, &one, 1).await, (0, 3));
    assert_eq!(split(&t, &two, 1).await, (0, 3));
    assert_eq!(stock.free_of("P"), 4_000);
}

/// Done when 8: with the setting off a line without stock is not back-ordered.
#[renox::test]
async fn sal_17_with_the_setting_off_a_line_without_stock_is_not_back_ordered() {
    let (t, _shop, stock) = boot(WlSales::new()).await;
    stock.add("P", 2_000);
    let order = confirmed(&t, "phone", &[("P", 5)]).await;
    assert_eq!(split(&t, &order, 1).await, (0, 0));
    assert_eq!(stock.free_of("P"), 2_000);
    assert!(back_orders(t.state()).open(t.state()).await.unwrap().is_empty());
}

/// Must not hold the rest of the order back: the other line is fulfilled
/// while one is back-ordered.
#[renox::test]
async fn sal_17_the_rest_of_the_order_is_fulfilled_while_a_line_is_back_ordered() {
    use wl_kernel::sales::{FulfilOrder, FulfilledLine, FulfilmentOutcome, order_fulfilment};
    let (t, _shop, stock) = boot(on()).await;
    stock.add("Q", 1_000);
    let order = confirmed(&t, "phone", &[("P", 3), ("Q", 1)]).await;
    let q = line(&t, &order, 2).await;
    let outcome = order_fulfilment(t.state())
        .fulfil(
            t.state(),
            FulfilOrder {
                order: order.order_id(),
                source: Subject::new("inventory.dispatch", 1),
                lines: vec![FulfilledLine {
                    line: q.id,
                    quantity_milli: 1_000,
                }],
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        FulfilmentOutcome::Fulfilled {
            complete: false,
            ..
        }
    ));
    let order = reload(&t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::PartiallyFulfilled);
    assert_eq!(split(&t, &order, 1).await, (0, 3));
    assert_eq!(split(&t, &order, 2).await, (0, 0));
    assert_eq!(stock.free_of("Q"), 1_000);
}
