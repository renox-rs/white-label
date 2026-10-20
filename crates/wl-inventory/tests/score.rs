//! The supplier score and its monthly snapshots (wl-inventory §3 *Supplier
//! score*, §5.9 step 2, `STK.SUP.01`, issue #248). The windows are given as
//! dates, so the tests do not depend on the clock.

mod support;

use renox::auth::permissions::define_role;
use renox::chrono::{NaiveDate, TimeZone, Utc};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::goods_receipt::record_arrival;
use wl_inventory::supplier_score::{self, ScoreBand, compute, snapshot_run};
use wl_inventory::{
    GoodsReceipt, GoodsReceiptLine, GoodsReceiptStatus, Item, Location, PurchaseOrder,
    PurchaseOrderLine, SupplierScore,
};
use wl_kernel::invoicing::{
    SupplierBillId, SupplierDebitNoteCancelled, SupplierDebitNoteId, SupplierDebitNoteIssued,
};
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

const GULF: PartyId = PartyId(22);

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Midday in Asia/Dubai on `date`.
fn at(date: NaiveDate) -> renox::db::DateTime {
    Utc.from_utc_datetime(&date.and_hms_opt(8, 0, 0).unwrap())
}

/// A line of a receipt: the order line, what arrived, what was accepted and
/// the reason for the rest.
struct Got<'a> {
    line: &'a PurchaseOrderLine,
    received: i64,
    accepted: i64,
    reason: Option<&'a str>,
}

impl<'a> Got<'a> {
    fn all(line: &'a PurchaseOrderLine, received: i64) -> Self {
        Got {
            line,
            received,
            accepted: received,
            reason: None,
        }
    }

    fn short(line: &'a PurchaseOrderLine, received: i64, accepted: i64, reason: &'a str) -> Self {
        Got {
            line,
            received,
            accepted,
            reason: Some(reason),
        }
    }
}

/// A receipt against `order` posted on `on`, in `status`, arrived at
/// `arrived` when given.
async fn receipt_in(
    t: &Booted,
    order: &PurchaseOrder,
    store: &Location,
    gots: &[Got<'_>],
    on: NaiveDate,
    status: GoodsReceiptStatus,
    arrived: Option<NaiveDate>,
) -> GoodsReceipt {
    let mut receipt = GoodsReceipt {
        purchase_order_id: Some(order.id),
        supplier_party_id: order.supplier_party_id,
        location_id: store.id,
        status,
        posted_at: Some(at(on)),
        arrived_at: arrived.map(at),
        late_entry: arrived.is_some(),
        ..GoodsReceipt::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    receipt
        .insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    for (position, got) in (1..).zip(gots) {
        let mut line = GoodsReceiptLine {
            goods_receipt_id: receipt.id,
            position,
            po_line_id: Some(got.line.id),
            item_id: got.line.item_id,
            received_qty_milli: got.received,
            accepted_qty_milli: got.accepted,
            rejected_qty_milli: got.received - got.accepted,
            rejection_reason: got.reason.map(str::to_owned),
            unit_cost: got.line.unit_price,
            ..GoodsReceiptLine::default()
        };
        line.save(t.db()).await.unwrap();
    }
    receipt
}

/// A posted receipt.
async fn posted(
    t: &Booted,
    order: &PurchaseOrder,
    store: &Location,
    gots: &[Got<'_>],
    on: NaiveDate,
) -> GoodsReceipt {
    receipt_in(t, order, store, gots, on, GoodsReceiptStatus::Posted, None).await
}

/// An issued order to `supplier` for 100 units of `item` at `price`, expected
/// on `expected` and issued on `issued`.
async fn order_to(
    t: &Booted,
    supplier: PartyId,
    store: &Location,
    item: &Item,
    price: i64,
    expected: Option<NaiveDate>,
    issued: NaiveDate,
) -> (PurchaseOrder, PurchaseOrderLine) {
    let (order, line) = t.order_from(supplier, store, item, 100_000, price).await;
    PurchaseOrder::query()
        .where_(PurchaseOrder::ID.eq(order.id))
        .update(
            t.db(),
            &[
                ("expected_date", &expected),
                ("issued_at", &Some(at(issued))),
            ],
        )
        .await
        .unwrap();
    let order = t.order_now(&order).await;
    (order, line)
}

fn window() -> (NaiveDate, NaiveDate) {
    (day(2026, 9, 1), day(2026, 11, 30))
}

/// Done when 1: 10 receipts in the window, 8 on time, rejected value 2%, no
/// incidents, price variance +5%: 40 x 0.8 + 30 x 0.8 + 15 x 1 + 15 x 0.5 =
/// 78.5, so 79 and the band `watch`.
#[renox::test]
async fn ten_receipts_with_eight_on_time_score_79() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    // The price a year ago, the reference of the variance: AED 10.00.
    order_to(&t, GULF, &store, &item, 1_000, None, day(2026, 8, 1)).await;
    for n in 0..10 {
        // Each order is AED 10.50 (+5%), issued before any receipt.
        let (order, line) = order_to(
            &t,
            GULF,
            &store,
            &item,
            1_050,
            Some(day(2026, 11, 10)),
            day(2026, 10, 1),
        )
        .await;
        let on = if n < 8 {
            day(2026, 11, 10)
        } else {
            day(2026, 11, 20)
        };
        let got = if n == 0 {
            // 100 arrive, 20 rejected for quality: 2% of the value received.
            Got::short(&line, 100_000, 80_000, "quality")
        } else {
            Got::all(&line, 100_000)
        };
        posted(&t, &order, &store, &[got], on).await;
    }

    let (from, to) = window();
    let figures = compute(t.state(), GULF.0, from, to).await.unwrap();

    assert_eq!(figures.receipts, 10);
    assert_eq!(figures.on_time, 8);
    assert_eq!(figures.on_time_bp, Some(8_000));
    assert_eq!(figures.rejected_bp, Some(200));
    assert_eq!(figures.incidents, 0);
    assert_eq!(figures.price_variance_bp, Some(500));
    assert_eq!(figures.score, Some(79));
    assert_eq!(figures.band, ScoreBand::Watch);
}

/// Done when 2 and 3: a receipt posted 2026-11-11 for an order expected
/// 2026-11-10 is on time (grace 1), 2026-11-12 is not; a rejection for
/// `excess` is not rejected value.
#[renox::test]
async fn the_grace_day_and_excess_rejections() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    for (on, reason) in [
        (day(2026, 11, 10), "excess"),
        (day(2026, 11, 11), "excess"),
        (day(2026, 11, 12), "excess"),
    ] {
        let (order, line) = order_to(
            &t,
            GULF,
            &store,
            &item,
            1_000,
            Some(day(2026, 11, 10)),
            day(2026, 10, 1),
        )
        .await;
        posted(
            &t,
            &order,
            &store,
            &[Got::short(&line, 100_000, 90_000, reason)],
            on,
        )
        .await;
    }

    let (from, to) = window();
    let figures = compute(t.state(), GULF.0, from, to).await.unwrap();

    assert_eq!(figures.receipts, 3);
    assert_eq!(figures.on_time, 2);
    assert_eq!(figures.rejected_value, 0);
    assert_eq!(figures.rejected_bp, Some(0));
}

/// Done when 4: with 2 receipts in the window the band is `not_enough_data`
/// and no score is shown.
#[renox::test]
async fn two_receipts_are_not_enough_data() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    for _ in 0..2 {
        let (order, line) = order_to(&t, GULF, &store, &item, 1_000, None, day(2026, 10, 1)).await;
        posted(
            &t,
            &order,
            &store,
            &[Got::all(&line, 10_000)],
            day(2026, 11, 3),
        )
        .await;
    }

    let (from, to) = window();
    let figures = compute(t.state(), GULF.0, from, to).await.unwrap();

    assert_eq!(figures.receipts, 2);
    assert_eq!(figures.score, None);
    assert_eq!(figures.band, ScoreBand::NotEnoughData);
}

/// Done when 5 and 9: an incident reported through the source counts; once
/// the source reports it voided, the live figure is back to 0.
#[renox::test]
async fn an_incident_counts_until_the_source_voids_it() {
    let t = boot().await;
    let ncr = Subject::new("manufacturing.ncr", 12);
    t.incidents.report(GULF, day(2026, 10, 20), &ncr);
    // Another supplier's incident is not ours.
    t.incidents.report(
        PartyId(99),
        day(2026, 10, 21),
        &Subject::new("manufacturing.ncr", 13),
    );

    let (from, to) = window();
    let before = compute(t.state(), GULF.0, from, to).await.unwrap();
    assert_eq!(before.incidents, 1);

    t.incidents.void(&ncr);
    let after = compute(t.state(), GULF.0, from, to).await.unwrap();
    assert_eq!(after.incidents, 0);
}

/// Done when 7: goods that arrived on 2026-11-10 and were entered late on
/// 2026-11-13 are on time for an order expected 2026-11-10; the late entry
/// is recorded on a draft receipt only.
#[renox::test]
async fn a_late_entry_keeps_the_day_the_goods_arrived() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let (order, line) = order_to(
        &t,
        GULF,
        &store,
        &item,
        1_000,
        Some(day(2026, 11, 10)),
        day(2026, 10, 1),
    )
    .await;
    let late = receipt_in(
        &t,
        &order,
        &store,
        &[Got::all(&line, 10_000)],
        day(2026, 11, 13),
        GoodsReceiptStatus::Posted,
        Some(day(2026, 11, 10)),
    )
    .await;
    let plain = posted(
        &t,
        &order,
        &store,
        &[Got::all(&line, 10_000)],
        day(2026, 11, 13),
    )
    .await;

    let (from, to) = window();
    let figures = compute(t.state(), GULF.0, from, to).await.unwrap();
    assert_eq!(figures.receipts, 2);
    // The late entry is on time; the same posting without the arrival is not.
    assert_eq!(figures.on_time, 1);

    // A draft receipt takes the arrival; a posted one does not.
    let draft = t
        .receipt(
            &order,
            &store,
            vec![wl_inventory::DraftLine::on(line.id, 1_000, 1_000)],
        )
        .await;
    record_arrival(t.db(), draft.id, at(day(2026, 11, 10)), true)
        .await
        .unwrap();
    let stored = t.reload(&draft).await;
    assert!(stored.late_entry);
    assert!(
        record_arrival(t.db(), late.id, at(day(2026, 11, 9)), true)
            .await
            .is_err()
    );
    assert!(
        record_arrival(t.db(), plain.id, at(day(2026, 11, 9)), true)
            .await
            .is_err()
    );
}

/// Done when 8: an order revised from 2026-11-10 to 2026-11-20 and received
/// on 2026-11-18 is late when the supplier asked for the date and on time
/// when we did.
#[renox::test]
async fn a_later_date_counts_only_when_we_asked_for_it() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let a = t.procurement("Amal").await;
    let supplier_pushed = PartyId(31);
    let we_pushed = PartyId(32);
    for (supplier, by) in [(supplier_pushed, "supplier"), (we_pushed, "us")] {
        let (order, _line) = order_to(
            &t,
            supplier,
            &store,
            &item,
            1_000,
            Some(day(2026, 11, 10)),
            day(2026, 10, 1),
        )
        .await;
        t.revise_as(
            &a,
            &order,
            "date moved",
            &[],
            &[("expected_date", "2026-11-20"), ("requested_by", by)],
        )
        .await
        .assert_ok();
        let revised = t.order_now(&order).await;
        assert_eq!(revised.revision, 2);
        let lines = wl_inventory::purchase_order::lines_of(t.db(), order.id)
            .await
            .unwrap();
        posted(
            &t,
            &revised,
            &store,
            &[Got::all(&lines[0], 10_000)],
            day(2026, 11, 18),
        )
        .await;
    }

    let (from, to) = window();
    let theirs = compute(t.state(), supplier_pushed.0, from, to)
        .await
        .unwrap();
    assert_eq!((theirs.receipts, theirs.on_time), (1, 0));
    let ours = compute(t.state(), we_pushed.0, from, to).await.unwrap();
    assert_eq!((ours.receipts, ours.on_time), (1, 1));
}

/// Done when 6, 11 and 12: the snapshot run twice on 2026-12-01 keeps one
/// snapshot; with the scheduler down on 12-01 the run on 12-02 builds it,
/// once, for the window 2026-09-01 to 2026-11-30; a receipt posted on 12-01
/// is in none of those and counts in the snapshot of 2027-01-01.
#[renox::test]
async fn snapshots_are_built_once_caught_up_and_never_recomputed() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    for on in [day(2026, 10, 5), day(2026, 10, 6), day(2026, 11, 3)] {
        let (order, line) = order_to(&t, GULF, &store, &item, 1_000, None, day(2026, 9, 20)).await;
        posted(&t, &order, &store, &[Got::all(&line, 10_000)], on).await;
    }
    let (order, line) = order_to(&t, GULF, &store, &item, 1_000, None, day(2026, 9, 20)).await;
    posted(
        &t,
        &order,
        &store,
        &[Got::all(&line, 10_000)],
        day(2026, 12, 1),
    )
    .await;

    // No run on 12-01: the run on 12-02 builds it, once.
    let made = snapshot_run(t.state(), day(2026, 12, 2)).await.unwrap();
    assert!(made.iter().any(|s| s.snapshot_date == day(2026, 12, 1)));
    let december: Vec<_> = made
        .iter()
        .filter(|s| s.snapshot_date == day(2026, 12, 1))
        .collect();
    assert_eq!(december.len(), 1);
    assert_eq!(december[0].window_from, day(2026, 9, 1));
    assert_eq!(december[0].window_to, day(2026, 11, 30));
    assert_eq!(december[0].receipts, 3);
    assert_eq!(december[0].score, Some(100));
    assert_eq!(december[0].band, ScoreBand::Good);

    // Twice on the same day: still one per supplier and month.
    assert!(
        snapshot_run(t.state(), day(2026, 12, 1))
            .await
            .unwrap()
            .is_empty()
    );
    let kept = SupplierScore::query()
        .where_(SupplierScore::SUPPLIER_PARTY_ID.eq(GULF.0))
        .where_(SupplierScore::SNAPSHOT_DATE.eq(day(2026, 12, 1)))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(kept.len(), 1);

    // A record changed after a snapshot changes the live view only.
    let late = Subject::new("manufacturing.ncr", 5);
    t.incidents.report(GULF, day(2026, 11, 20), &late);
    assert!(
        snapshot_run(t.state(), day(2026, 12, 20))
            .await
            .unwrap()
            .is_empty()
    );
    let again = SupplierScore::find_or_404(t.db(), kept[0].id)
        .await
        .unwrap();
    assert_eq!(again.incidents, 0);
    assert_eq!(again.computed_at, kept[0].computed_at);
    let live_window = compute(t.state(), GULF.0, day(2026, 9, 1), day(2026, 11, 30))
        .await
        .unwrap();
    assert_eq!(live_window.incidents, 1);

    // 2027-01-01 counts the receipt of 12-01.
    let january = snapshot_run(t.state(), day(2027, 1, 1)).await.unwrap();
    let january = january
        .iter()
        .find(|s| s.snapshot_date == day(2027, 1, 1))
        .expect("the January snapshot");
    assert_eq!(january.window_from, day(2026, 10, 1));
    assert_eq!(january.receipts, 4);
}

/// Done when 10: a USD supplier at USD 10 on both the last receipt and the
/// new order has a price variance of 0%.
#[renox::test]
async fn an_unchanged_price_in_the_same_currency_is_zero_variance() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let (old, old_line) = order_to(&t, GULF, &store, &item, 1_000, None, day(2026, 8, 1)).await;
    let (fresh, _line) = order_to(&t, GULF, &store, &item, 1_000, None, day(2026, 10, 15)).await;
    for order in [&old, &fresh] {
        PurchaseOrder::query()
            .where_(PurchaseOrder::ID.eq(order.id))
            .update(t.db(), &[("currency", &"USD".to_owned())])
            .await
            .unwrap();
    }
    posted(
        &t,
        &old,
        &store,
        &[Got::all(&old_line, 10_000)],
        day(2026, 8, 20),
    )
    .await;

    let figures = compute(t.state(), GULF.0, day(2026, 10, 1), day(2026, 10, 31))
        .await
        .unwrap();

    assert_eq!(figures.price_variance_bp, Some(0));
}

/// Done when 13: receipts from the supplier at AED 10 on 2026-10-05 and
/// AED 11 on 2026-10-20 and an order line issued on 2026-10-15 at AED 12: its
/// last price is AED 10 (the receipt before the issue) and its variance is
/// +20%.
#[renox::test]
async fn the_last_price_is_the_receipt_before_the_issue() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let (first, first_line) = order_to(&t, GULF, &store, &item, 1_000, None, day(2026, 8, 1)).await;
    let (second, second_line) =
        order_to(&t, GULF, &store, &item, 1_100, None, day(2026, 8, 2)).await;
    posted(
        &t,
        &first,
        &store,
        &[Got::all(&first_line, 10_000)],
        day(2026, 10, 5),
    )
    .await;
    posted(
        &t,
        &second,
        &store,
        &[Got::all(&second_line, 10_000)],
        day(2026, 10, 20),
    )
    .await;
    order_to(&t, GULF, &store, &item, 1_200, None, day(2026, 10, 15)).await;

    let figures = compute(t.state(), GULF.0, day(2026, 10, 1), day(2026, 10, 31))
        .await
        .unwrap();

    assert_eq!(figures.price_variance_bp, Some(2_000));
}

/// Done when (debit notes): shown, not scored; a cancelled one drops out.
/// A reversed receipt and a return from stock: the reversed one is not
/// counted, the dispatched return is rejected value.
#[renox::test]
async fn debit_notes_returns_and_reversed_receipts() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let today = wl_kernel::documents::today();
    let (from, to) = (
        today
            .checked_sub_days(renox::chrono::Days::new(30))
            .unwrap(),
        today.checked_add_days(renox::chrono::Days::new(1)).unwrap(),
    );
    let issued = SupplierDebitNoteIssued {
        debit_note: SupplierDebitNoteId(7),
        supplier: GULF,
        bill: SupplierBillId(3),
        return_to_supplier: None,
        amount: 31_500,
        vat: 1_500,
        lines: Vec::new(),
    };
    t.state().emit(issued.clone()).await.unwrap();
    t.state().emit(issued).await.unwrap();
    let live = compute(t.state(), GULF.0, from, to).await.unwrap();
    assert_eq!((live.debit_notes, live.debit_note_value), (1, 31_500));
    t.state()
        .emit(SupplierDebitNoteCancelled {
            debit_note: SupplierDebitNoteId(7),
            supplier: GULF,
            bill: None,
            return_to_supplier: None,
            amount: 31_500,
            reason: "issued in error".into(),
            lines: Vec::new(),
        })
        .await
        .unwrap();
    let after = compute(t.state(), GULF.0, from, to).await.unwrap();
    assert_eq!((after.debit_notes, after.debit_note_value), (0, 0));

    let (order, line) = order_to(&t, GULF, &store, &item, 1_000, None, today).await;
    receipt_in(
        &t,
        &order,
        &store,
        &[Got::all(&line, 10_000)],
        today,
        GoodsReceiptStatus::Reversed,
        None,
    )
    .await;
    assert_eq!(
        compute(t.state(), GULF.0, from, to).await.unwrap().receipts,
        0
    );

    // 4 units go back from stock at AED 100.00 (10,000) a unit, and
    // count when the return is dispatched.
    let (_order, supplier_line) = t.order(&store, &item, 10_000, 1_000).await;
    t.dispatched_credit_return(&store, &[(&item, 4_000, &supplier_line)])
        .await;
    let returned = compute(t.state(), support::SUPPLIER.0, from, to)
        .await
        .unwrap();
    assert_eq!(returned.rejected_value, 40_000);
}

/// A review note is added to a snapshot and changes none of its numbers; the
/// score is for the holders of `supplier_scores.view`, its values for the
/// holders of `stock.cost.view`.
#[renox::test]
async fn the_score_is_read_with_its_permission_and_notes_change_nothing() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    for on in [day(2026, 10, 5), day(2026, 10, 6), day(2026, 11, 3)] {
        let (order, line) = order_to(&t, GULF, &store, &item, 1_000, None, day(2026, 9, 20)).await;
        posted(&t, &order, &store, &[Got::all(&line, 10_000)], on).await;
    }
    let made = snapshot_run(t.state(), day(2026, 12, 1)).await.unwrap();
    let snapshot = made
        .into_iter()
        .find(|s| s.snapshot_date == day(2026, 12, 1))
        .unwrap();
    let a = t.procurement("Amal").await;
    let clerk = t.storekeeper("Sam").await;
    define_role(
        t.db(),
        "scorer",
        &[wl_inventory::permissions::SUPPLIER_SCORES_VIEW],
    )
    .await
    .unwrap();
    let viewer = t.user_with("Vera", "scorer").await;
    viewer.assign_role(t.db(), "cost_viewer").await.unwrap();
    let plain = t.user_with("Pat", "scorer").await;

    t.app.acting_as(&a);
    t.app
        .htmx()
        .post(
            &format!("/supplier-scores/{}/notes", snapshot.id),
            &[("note", "Supplier contests the late delivery")],
        )
        .await
        .assert_ok();
    t.app
        .htmx()
        .post(
            &format!("/supplier-scores/{}/notes", snapshot.id),
            &[("note", " ")],
        )
        .await
        .assert_status(422);
    t.app.acting_as(&clerk);
    t.app
        .htmx()
        .post(
            &format!("/supplier-scores/{}/notes", snapshot.id),
            &[("note", "x")],
        )
        .await
        .assert_status(403);
    let after = SupplierScore::find_or_404(t.db(), snapshot.id)
        .await
        .unwrap();
    assert_eq!(after.score, snapshot.score);
    assert_eq!(after.receipts, snapshot.receipts);
    assert_eq!(
        supplier_score::notes_of(t.db(), snapshot.id)
            .await
            .unwrap()
            .len(),
        1
    );

    let path = format!("/suppliers/{}/score", GULF.0);
    t.app.acting_as(&clerk);
    t.app.get(&path).await.assert_status(403);
    t.app.acting_as(&plain);
    let body: renox::serde_json::Value =
        renox::serde_json::from_str(&t.app.get(&path).await.text()).unwrap();
    assert_eq!(body["snapshots"][0]["receipts"], 3);
    assert!(body["snapshots"][0]["received_value"].is_null(), "{body}");
    assert_eq!(
        body["snapshots"][0]["notes"][0],
        "Supplier contests the late delivery"
    );
    t.app.acting_as(&viewer);
    let body: renox::serde_json::Value =
        renox::serde_json::from_str(&t.app.get(&path).await.text()).unwrap();
    assert_eq!(body["snapshots"][0]["received_value"], 30_000);
}
