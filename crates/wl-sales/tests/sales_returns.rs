//! The sales return base (wl-sales §4.5, §5.6; base for BL-SAL-22): a return
//! of a completed counter sale or of a handed-over order is requested,
//! received inside the return window, inspected and resolved, or rejected;
//! a refund made by the channel can be reversed. `ReturnResolved` and
//! `ReturnReversed` follow the commit.

mod support;

use std::sync::{Arc, Mutex};

use renox::chrono::Duration;
use renox::prelude::*;
use wl_kernel::sales::{ReturnResolved, ReturnReversed};
use wl_sales::permissions as perm;
use wl_sales::{
    CounterSale, ReturnKind, ReturnResolution, ReturnStatus, SaleLine, SalesReturn, SalesReturnLine,
};

use support::{Booted, boot_configured};

/// The events a test saw.
#[derive(Clone, Default)]
struct Seen {
    resolved: Arc<Mutex<Vec<ReturnResolved>>>,
    reversed: Arc<Mutex<Vec<ReturnReversed>>>,
}

/// Records the events of type `E` into `log`.
fn record<E: Event + Clone + Send + Sync + 'static>(app: &mut Registry, log: &Arc<Mutex<Vec<E>>>) {
    let log = log.clone();
    app.listen(move |event: E, _state: AppState| {
        let log = log.clone();
        async move {
            log.lock().unwrap().push(event);
            Ok(())
        }
    });
}

impl Module for Seen {
    fn name(&self) -> &'static str {
        "return_events"
    }

    fn register(&self, app: &mut Registry) {
        record(app, &self.resolved);
        record(app, &self.reversed);
    }
}

async fn boot() -> (Booted, Seen) {
    let seen = Seen::default();
    let t = boot_configured(wl_sales::WlSales::new(), seen.clone()).await;
    (t, seen)
}

/// A clerk who rings up sales and handles returns.
async fn clerk(t: &Booted) -> User {
    t.user(
        "Clara",
        &[perm::POS_SELL, perm::RETURNS_REQUEST, perm::RETURNS_HANDLE],
    )
    .await
}

/// A completed counter sale of 2 tees at AED 50 before VAT (AED 105.00 per
/// tee with VAT... AED 52.50 each), paid in cash.
async fn sold(t: &Booted) -> (CounterSale, Vec<SaleLine>) {
    t.app
        .post("/counter-sales", &[("x", "")])
        .await
        .assert_status(303);
    let sale = CounterSale::query()
        .order_by_desc(CounterSale::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    t.app
        .post(
            &format!("/counter-sales/{}/lines", sale.id),
            &[
                ("item_code", "tee"),
                ("unit_price", "50.00"),
                ("quantity", "2"),
            ],
        )
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("cash", "105.00")],
        )
        .await
        .assert_status(303);
    let sale = CounterSale::find_or_404(t.db(), sale.id).await.unwrap();
    let lines = SaleLine::of(t.db(), &sale.subject()).await.unwrap();
    (sale, lines)
}

/// Requests a return of `quantity` of the first line of `sale` through the
/// page and returns it.
async fn asked(t: &Booted, sale: &CounterSale, line: &SaleLine, quantity: &str) -> SalesReturn {
    t.app
        .post(
            "/sales-returns",
            &[
                ("sale_type", "counter_sale"),
                ("sale_id", &sale.id.to_string()),
                ("lines", &format!("{}:{quantity}", line.id)),
                ("reason", "too small"),
            ],
        )
        .await
        .assert_status(303);
    SalesReturn::query()
        .order_by_desc(SalesReturn::ID)
        .first(t.db())
        .await
        .unwrap()
        .expect("the return was requested")
}

async fn reload(t: &Booted, r: &SalesReturn) -> SalesReturn {
    SalesReturn::find_or_404(t.db(), r.id).await.unwrap()
}

#[renox::test]
async fn half_of_a_sale_is_returned_received_inspected_and_resolved() {
    let (t, seen) = boot().await;
    t.app.acting_as(&clerk(&t).await);
    let (sale, lines) = sold(&t).await;
    let r = asked(&t, &sale, &lines[0], "1").await;
    assert!(r.number.starts_with("RET-"), "{}", r.number);
    assert_eq!(r.status, ReturnStatus::Requested);
    assert_eq!(r.kind, ReturnKind::Goods);
    // One of the two tees: AED 50 before VAT, AED 2.50 VAT.
    assert_eq!(
        (r.net_value, r.vat_value, r.gross_value),
        (5_000, 250, 5_250)
    );
    assert_eq!(r.eligible_net, 5_000);
    assert_eq!(r.party_id, None);
    let taken = r.lines(t.db()).await.unwrap();
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].quantity_milli, 1000);

    // The goods must be received and inspected before they are resolved.
    let resolve = format!("/sales-returns/{}/resolve", r.id);
    t.app
        .post(&resolve, &[("resolution", "refund"), ("inspected", "1")])
        .await
        .assert_status(422);
    t.app
        .post(&format!("/sales-returns/{}/receive", r.id), &[("x", "")])
        .await
        .assert_status(303);
    assert_eq!(reload(&t, &r).await.status, ReturnStatus::Received);
    assert!(reload(&t, &r).await.received_at.is_some());
    // Not inspected, no resolution, a credit note without a number.
    t.app
        .post(&resolve, &[("resolution", "refund")])
        .await
        .assert_status(422);
    t.app
        .post(&resolve, &[("inspected", "1")])
        .await
        .assert_status(422);
    t.app
        .post(
            &resolve,
            &[("resolution", "credit_note"), ("inspected", "1")],
        )
        .await
        .assert_status(422);
    assert!(seen.resolved.lock().unwrap().is_empty());

    t.app
        .post(&resolve, &[("resolution", "refund"), ("inspected", "1")])
        .await
        .assert_status(303);
    let done = reload(&t, &r).await;
    assert_eq!(done.status, ReturnStatus::Resolved);
    assert_eq!(done.resolution, Some(ReturnResolution::Refund));
    assert!(done.resolved_at.is_some());
    let events = seen.resolved.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sales_return.0, r.id);
    assert_eq!(events[0].kind, "goods");
    assert_eq!(events[0].resolution, "refund");
    assert_eq!(events[0].value, 5_250);
    assert_eq!(events[0].sale, sale.subject());
    t.app
        .get(&format!("/sales-returns/{}", r.id))
        .await
        .assert_ok()
        .assert_see(&r.number)
        .assert_see("AED 52.50");
}

#[renox::test]
async fn a_credit_note_resolution_names_the_credit_note() {
    let (t, seen) = boot().await;
    t.app.acting_as(&clerk(&t).await);
    let (sale, lines) = sold(&t).await;
    let r = asked(&t, &sale, &lines[0], "1").await;
    t.app
        .post(&format!("/sales-returns/{}/receive", r.id), &[("x", "")])
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/sales-returns/{}/resolve", r.id),
            &[
                ("resolution", "credit_note"),
                ("credit_note", "31"),
                ("inspected", "1"),
            ],
        )
        .await
        .assert_status(303);
    let done = reload(&t, &r).await;
    assert_eq!(done.credit_note_id, Some(31));
    assert_eq!(
        SalesReturn::of_credit_note(t.db(), 31)
            .await
            .unwrap()
            .map(|x| x.id),
        Some(r.id)
    );
    assert_eq!(seen.resolved.lock().unwrap()[0].credit_note, Some(31));
}

#[renox::test]
async fn a_request_asks_for_a_sale_that_was_completed_and_for_quantities_it_holds() {
    let (t, _) = boot().await;
    t.app.acting_as(&clerk(&t).await);
    // An open cart cannot be returned.
    t.app
        .post("/counter-sales", &[("x", "")])
        .await
        .assert_status(303);
    let cart = CounterSale::query()
        .order_by_desc(CounterSale::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    t.app
        .post(
            &format!("/counter-sales/{}/lines", cart.id),
            &[("item_code", "tee"), ("unit_price", "10.00")],
        )
        .await
        .assert_status(303);
    let cart_line = SaleLine::of(t.db(), &cart.subject())
        .await
        .unwrap()
        .remove(0);
    let fields = |sale: i64, line: i64, quantity: &str| {
        vec![
            ("sale_type".to_owned(), "counter_sale".to_owned()),
            ("sale_id".to_owned(), sale.to_string()),
            ("lines".to_owned(), format!("{line}:{quantity}")),
        ]
    };
    let post = |fields: Vec<(String, String)>| {
        let t = &t;
        async move {
            let borrowed: Vec<(&str, &str)> = fields
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            t.app.htmx().post("/sales-returns", &borrowed).await
        }
    };
    post(fields(cart.id, cart_line.id, "1"))
        .await
        .assert_status(422);

    let (sale, lines) = sold(&t).await;
    // More than was sold, nothing, a line of another sale, a bad format.
    post(fields(sale.id, lines[0].id, "3"))
        .await
        .assert_status(422);
    post(fields(sale.id, lines[0].id, "0"))
        .await
        .assert_status(422);
    post(fields(sale.id, cart_line.id, "1"))
        .await
        .assert_status(422);
    post(fields(sale.id, lines[0].id, "two"))
        .await
        .assert_status(422);
    // Two tees were sold: one now, one later, but not a third.
    asked(&t, &sale, &lines[0], "1").await;
    asked(&t, &sale, &lines[0], "1").await;
    post(fields(sale.id, lines[0].id, "1"))
        .await
        .assert_status(422);
    assert_eq!(SalesReturnLine::query().count(t.db()).await.unwrap(), 2);
}

#[renox::test]
async fn goods_outside_the_return_window_need_approval() {
    let (t, _) = boot().await;
    t.app.acting_as(&clerk(&t).await);
    let (sale, lines) = sold(&t).await;
    // The sale was completed 20 days ago.
    CounterSale::query()
        .where_(CounterSale::ID.eq(sale.id))
        .update(
            t.db(),
            &[("completed_at", &Some(renox::db::now() - Duration::days(20)))],
        )
        .await
        .unwrap();
    let r = asked(&t, &sale, &lines[0], "1").await;
    let receive = format!("/sales-returns/{}/receive", r.id);
    t.app.post(&receive, &[("x", "")]).await.assert_status(422);
    assert_eq!(reload(&t, &r).await.status, ReturnStatus::Requested);
    t.app
        .post(&receive, &[("approved", "1")])
        .await
        .assert_status(303);
    assert_eq!(reload(&t, &r).await.status, ReturnStatus::Received);
}

#[renox::test]
async fn a_return_is_rejected_with_a_reason_and_only_handlers_act() {
    let (t, _) = boot().await;
    let clara = clerk(&t).await;
    t.app.acting_as(&clara);
    let (sale, lines) = sold(&t).await;
    let r = asked(&t, &sale, &lines[0], "1").await;
    let reject = format!("/sales-returns/{}/transitions/reject", r.id);
    t.app
        .post(&reject, &[("reason", "")])
        .await
        .assert_status(422);

    // Someone who may request but not handle cannot reject.
    let asker = t.user("Ali", &[perm::RETURNS_REQUEST]).await;
    t.app.acting_as(&asker);
    t.app
        .post(&reject, &[("reason", "no")])
        .await
        .assert_forbidden();
    t.app
        .post(&format!("/sales-returns/{}/receive", r.id), &[("x", "")])
        .await
        .assert_forbidden();
    t.app
        .get("/sales-returns")
        .await
        .assert_ok()
        .assert_see("RET-");

    t.app.acting_as(&clara);
    t.app
        .post(&reject, &[("reason", "Worn out")])
        .await
        .assert_status(303);
    let rejected = reload(&t, &r).await;
    assert_eq!(rejected.status, ReturnStatus::Rejected);
    assert_eq!(rejected.closing_reason.as_deref(), Some("Worn out"));
    // A rejected return releases the quantity: the full amount can be asked
    // for again.
    asked(&t, &sale, &lines[0], "2").await;
}

#[renox::test]
async fn a_return_with_no_goods_is_resolved_from_requested_and_a_channel_refund_reverses() {
    let (t, seen) = boot().await;
    t.app.acting_as(&clerk(&t).await);
    let (sale, lines) = sold(&t).await;
    let line = lines[0].id;
    t.app
        .post(
            "/sales-returns",
            &[
                ("sale_type", "counter_sale"),
                ("sale_id", &sale.id.to_string()),
                ("kind", "no_goods"),
                ("lines", &format!("{line}:1")),
                ("reason", "arrived cold"),
            ],
        )
        .await
        .assert_status(303);
    let r = SalesReturn::query()
        .order_by_desc(SalesReturn::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.kind, ReturnKind::NoGoods);
    let resolve = format!("/sales-returns/{}/resolve", r.id);
    // Nothing to receive; the complaint and the responsible party are needed.
    t.app
        .post(&format!("/sales-returns/{}/receive", r.id), &[("x", "")])
        .await
        .assert_status(422);
    t.app
        .post(&resolve, &[("resolution", "refunded_by_channel")])
        .await
        .assert_status(422);
    t.app
        .post(
            &resolve,
            &[
                ("resolution", "refunded_by_channel"),
                ("complaint_category", "cold"),
                ("responsible_party", "aggregator"),
            ],
        )
        .await
        .assert_status(303);
    let done = reload(&t, &r).await;
    assert_eq!(done.status, ReturnStatus::Resolved);
    assert_eq!(done.complaint_category.as_deref(), Some("cold"));
    assert_eq!(done.responsible_party.as_deref(), Some("aggregator"));

    let reverse = format!("/sales-returns/{}/transitions/reverse", r.id);
    t.app
        .post(&reverse, &[("reason", "")])
        .await
        .assert_status(422);
    t.app
        .post(&reverse, &[("reason", "Statement voided")])
        .await
        .assert_status(303);
    assert_eq!(reload(&t, &r).await.status, ReturnStatus::Reversed);
    let events = seen.reversed.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason, "Statement voided");
    assert_eq!(events[0].value, 5_250);

    // A refund that is not made by the channel is never reversed, and goods
    // cannot be refunded by the channel.
    let other = asked(&t, &sale, &lines[0], "1").await;
    t.app
        .post(
            &format!("/sales-returns/{}/receive", other.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    let resolve = format!("/sales-returns/{}/resolve", other.id);
    t.app
        .post(
            &resolve,
            &[("resolution", "refunded_by_channel"), ("inspected", "1")],
        )
        .await
        .assert_status(422);
    t.app
        .post(&resolve, &[("resolution", "refund"), ("inspected", "1")])
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/sales-returns/{}/transitions/reverse", other.id),
            &[("reason", "oops")],
        )
        .await
        .assert_status(422);
}
