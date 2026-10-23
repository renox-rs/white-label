//! BL-SAL-13 (#132): the order follows its deliveries (wl-sales §4.2 rows
//! `fulfil` and `reverse_fulfilment`, *Fulfilment from deliveries*, §5.2
//! step 4, §6). Only accepted quantities are fulfilled, once per hand-over;
//! a returned delivery changes no state; a reversed or lost delivery takes the
//! fulfilment back. The events of wl-inventory and wl-delivery are emitted by
//! hand; the order, its machine, listeners, tasks and pages are the real
//! ones; invoicing, tasks and notices are the fakes of `support::changes`.

mod support;

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::delivery::{
    DeliveredLine, DeliveryDelivered, DeliveryId, DeliveryLost, DeliveryReturned, DeliveryReversed,
};
use wl_kernel::inventory::{
    DispatchDelivered, DispatchDirectory, DispatchFacts, DispatchLost, DispatchReturned,
    DispatchReversed, DispatchedLine, TraceFuture, provide_dispatch_directory,
};
use wl_kernel::invoicing::InvoiceStatus;
use wl_kernel::sales::{SalesOrderFulfilmentReversed, SalesOrderId};
use wl_kernel::{PartyId, Subject};
use wl_sales::deliveries::DELIVERY_RETURNED;
use wl_sales::{SaleLine, SalesOrder, SalesOrderStatus, WlSales, sales_orders};

use support::Booted;
use support::changes::{Changes, clerk, lines, place, reload};

/// The dispatches wl-inventory would list for the order, set by the test.
#[derive(Clone, Default)]
struct Directory(Arc<Mutex<Vec<DispatchFacts>>>);

impl Directory {
    fn set(&self, facts: Vec<DispatchFacts>) {
        *self.0.lock().unwrap() = facts;
    }
}

impl DispatchDirectory for Directory {
    fn facts<'a>(
        &'a self,
        _state: &'a AppState,
        dispatch: &'a Subject,
    ) -> TraceFuture<'a, Option<DispatchFacts>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .find(|facts| &facts.dispatch == dispatch)
                .cloned())
        })
    }

    fn of_source<'a>(
        &'a self,
        _state: &'a AppState,
        _source: &'a Subject,
    ) -> TraceFuture<'a, Vec<DispatchFacts>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().clone()) })
    }
}

/// The fakes, the directory and the reversal events recorded.
#[derive(Clone, Default)]
struct World {
    directory: Directory,
    reversed: Arc<Mutex<Vec<SalesOrderFulfilmentReversed>>>,
}

impl Module for World {
    fn name(&self) -> &'static str {
        "deliveries_world"
    }

    fn register(&self, app: &mut Registry) {
        provide_dispatch_directory(app, self.directory.clone());
        let log = self.reversed.clone();
        app.listen(
            move |event: SalesOrderFulfilmentReversed, _state: AppState| {
                let log = log.clone();
                async move {
                    log.lock().unwrap().push(event);
                    Ok(())
                }
            },
        );
    }
}

async fn boot(on_delivery: bool) -> (Booted, Changes, World) {
    let changes = Changes::default();
    let world = World::default();
    let t = support::boot_pair(
        WlSales::new().invoice_on_delivery(on_delivery),
        changes.clone(),
        world.clone(),
    )
    .await;
    wl_kernel::approvals::install(t.db()).await.unwrap();
    (t, changes, world)
}

/// A confirmed order of party 7 with a line of `quantity` pumps at AED
/// 100.00 and one cap, entered by Sam, and the pump line.
async fn order_of(t: &Booted, c: &Changes, quantity: &str) -> (SalesOrder, SaleLine, User) {
    let sam = clerk(t).await;
    t.app.acting_as(&sam);
    let order = place(
        t,
        c,
        "phone",
        "on_account",
        &[("pump", quantity, "100.00"), ("cap", "1", "10.00")],
    )
    .await;
    let line = lines(t, &order).await.remove(0);
    (order, line, sam)
}

/// The cap line of the order.
async fn cap_of(t: &Booted, order: &SalesOrder) -> SaleLine {
    lines(t, order).await.remove(1)
}

fn dispatch(n: i64) -> Subject {
    Subject::new("inventory.dispatch", n)
}

/// A dispatch line for `line`: sent, accepted and refused (in units).
fn sent(line: &SaleLine, sent: i64, delivered: i64, refused: i64) -> DispatchedLine {
    DispatchedLine {
        line: Subject::new("inventory.dispatch_line", line.id),
        item: Subject::new("inventory.item", 9),
        lot: Some(Subject::new("inventory.lot", 5)),
        source_line: Some(Subject::new("sales.sale_line", line.id)),
        quantity_milli: sent * 1000,
        delivered_milli: delivered * 1000,
        refused_milli: refused * 1000,
        refusal_reason: (refused > 0).then(|| "damaged".to_owned()),
        value: 0,
    }
}

async fn delivered(t: &Booted, order: &SalesOrder, n: i64, lines: Vec<DispatchedLine>) {
    t.state()
        .emit(DispatchDelivered {
            dispatch: dispatch(n),
            source: order.subject(),
            recipient: Some(PartyId(7)),
            lines,
        })
        .await
        .unwrap();
}

async fn reversed(t: &Booted, order: &SalesOrder, n: i64, lines: Vec<DispatchedLine>) {
    t.state()
        .emit(DispatchReversed {
            dispatch: dispatch(n),
            source: order.subject(),
            recipient: Some(PartyId(7)),
            lines,
        })
        .await
        .unwrap();
}

/// AC1: Given a `confirmed` order line of 10 units, when the delivery is
/// delivered with 8 accepted and 2 refused, the line's fulfilled quantity is
/// 8, the order is `partially_fulfilled`, the refused 2 stay open, and a draft
/// invoice (invoice on delivery) bills 8.
#[renox::test]
async fn only_the_accepted_quantity_is_fulfilled_and_billed() {
    let (t, c, _w) = boot(true).await;
    let (order, line, _sam) = order_of(&t, &c, "10").await;

    delivered(&t, &order, 5, vec![sent(&line, 10, 8, 2)]).await;

    let order = reload(&t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::PartiallyFulfilled);
    let line = lines(&t, &order).await.remove(0);
    assert_eq!(line.fulfilled_milli, 8_000);
    assert_eq!(line.open_milli(), 2_000);
    assert_eq!(line.cancelled_milli, 0);
    let drafts = c.ledger.drafts();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].1.lines.len(), 1);
    assert_eq!(drafts[0].1.lines[0].quantity_milli, 8_000);
    assert_eq!(order.invoice_id, Some(drafts[0].0));
}

/// AC2: Given both `DeliveryDelivered` (wl-delivery) and `DispatchDelivered`
/// (wl-inventory) for that delivery, fulfilled is 8, not 16; the same event
/// twice changes nothing either.
#[renox::test]
async fn a_delivery_reaching_the_order_by_two_events_is_counted_once() {
    let (t, c, _w) = boot(true).await;
    let (order, line, _sam) = order_of(&t, &c, "10").await;

    t.state()
        .emit(DeliveryDelivered {
            delivery: DeliveryId(1),
            source: dispatch(5),
            attempt: 1,
            proof: 1,
            lines: vec![DeliveredLine {
                line: Some(Subject::new("inventory.dispatch_line", line.id)),
                description: "Pump".to_owned(),
                quantity_milli: 10_000,
                delivered_milli: 8_000,
                refused_milli: 2_000,
                refusal_reason: Some("damaged".to_owned()),
            }],
            cash_expected: 0,
            cash_collected: 0,
            cod_difference: 0,
            cod_difference_reason: None,
            absorbed_by: None,
            cheque: None,
        })
        .await
        .unwrap();
    delivered(&t, &order, 5, vec![sent(&line, 10, 8, 2)]).await;
    delivered(&t, &order, 5, vec![sent(&line, 10, 8, 2)]).await;

    let line = lines(&t, &reload(&t, order.id).await).await.remove(0);
    assert_eq!(line.fulfilled_milli, 8_000);
    assert_eq!(c.ledger.drafts().len(), 1);
}

/// AC3 and AC4: `DispatchReturned` for a delivery of 5 units leaves the order
/// state, shows the lines *not dispatched* on the order page, tells the sales
/// rep and the customer; the 5 units can then be cancelled with `cancel_line`.
#[renox::test]
async fn a_returned_delivery_changes_no_state_and_the_lines_can_be_cancelled() {
    let (t, c, w) = boot(false).await;
    let (order, line, sam) = order_of(&t, &c, "5").await;
    w.directory
        .set(vec![facts(&order, "out", "dispatched", &line, 5)]);
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Out for delivery");

    t.state()
        .emit(DispatchReturned {
            dispatch: dispatch(5),
            source: order.subject(),
            recipient: Some(PartyId(7)),
            reason: "customer closed".to_owned(),
            complaint: None,
            lines: vec![sent(&line, 5, 0, 0)],
        })
        .await
        .unwrap();
    w.directory
        .set(vec![facts(&order, "out", "returned", &line, 5)]);

    let after = reload(&t, order.id).await;
    assert_eq!(after.status, SalesOrderStatus::Confirmed);
    assert_eq!(lines(&t, &after).await[0].fulfilled_milli, 0);
    t.app.acting_as(&sam);
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Not dispatched");
    let notices = c.mail.of_type(DELIVERY_RETURNED);
    assert_eq!(notices.len(), 1);
    let to_user = notices[0]
        .recipients
        .iter()
        .any(|r| matches!(r, wl_kernel::notify::NotifyRecipient::User { id } if *id == sam.id));
    let to_party = notices[0].recipients.iter().any(
        |r| matches!(r, wl_kernel::notify::NotifyRecipient::Party { id } if *id == PartyId(7)),
    );
    assert!(to_user && to_party, "the rep and the customer are told");

    t.app
        .post(
            &format!("/sales-orders/{}/cancel-line", order.id),
            &[
                ("line_no", "1"),
                ("quantity", "5"),
                ("reason", "customer closed"),
            ],
        )
        .await
        .assert_status(303);
    assert_eq!(lines(&t, &after).await[0].cancelled_milli, 5_000);
}

fn facts(
    order: &SalesOrder,
    number: &str,
    status: &str,
    line: &SaleLine,
    quantity: i64,
) -> DispatchFacts {
    DispatchFacts {
        dispatch: dispatch(5),
        number: number.to_owned(),
        status: status.to_owned(),
        source: order.subject(),
        recipient: Some(PartyId(7)),
        dispatched_on: None,
        lines: vec![sent(line, quantity, 0, 0)],
    }
}

/// A delivery requested for the order itself (no dispatch note) returned: the
/// same notice, no state change.
#[renox::test]
async fn a_delivery_without_a_dispatch_note_that_comes_back_tells_the_rep_too() {
    let (t, c, _w) = boot(false).await;
    let (order, _line, _sam) = order_of(&t, &c, "5").await;

    t.state()
        .emit(DeliveryReturned {
            delivery: DeliveryId(2),
            source: order.subject(),
            reason: "no one home".to_owned(),
        })
        .await
        .unwrap();

    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );
    assert_eq!(c.mail.of_type(DELIVERY_RETURNED).len(), 1);
}

/// AC5: Given a `fulfilled` order (10 of 10 from one dispatch), on
/// `DispatchReversed` it is `confirmed`, fulfilled = 0, its draft invoice
/// follows (nothing delivered: the draft is discarded), and
/// `SalesOrderFulfilmentReversed` is emitted; `DeliveryReversed` for the same
/// delivery changes nothing more; the dispatch delivered again fulfils again.
#[renox::test]
async fn a_reversed_delivery_takes_the_fulfilment_back_once() {
    let (t, c, w) = boot(true).await;
    let (order, line, _sam) = order_of(&t, &c, "10").await;
    let cap = cap_of(&t, &order).await;
    delivered(
        &t,
        &order,
        5,
        vec![sent(&line, 10, 10, 0), sent(&cap, 1, 1, 0)],
    )
    .await;
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Fulfilled
    );

    reversed(&t, &order, 5, vec![sent(&line, 10, 0, 0)]).await;

    let after = reload(&t, order.id).await;
    assert_eq!(after.status, SalesOrderStatus::Confirmed);
    assert_eq!(lines(&t, &after).await[0].fulfilled_milli, 0);
    assert_eq!(c.ledger.discarded().len(), 1);
    assert_eq!(after.invoice_id, None);
    let events = w.reversed.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].order, SalesOrderId(order.id));
    assert_eq!(events[0].lines[0].quantity_milli, 10_000);
    assert_eq!(events[0].cause, "reversed");

    // The delivery's own event names the dispatch: nothing more happens.
    t.state()
        .emit(DeliveryReversed {
            delivery: DeliveryId(1),
            source: dispatch(5),
            reason: "not delivered".to_owned(),
            liability: "none".to_owned(),
            proof: Some(1),
            cash_treatment: None,
            approver: None,
            target_state: None,
            driver: None,
            cash_amount: 0,
            lines: Vec::new(),
        })
        .await
        .unwrap();
    reversed(&t, &order, 5, vec![sent(&line, 10, 0, 0)]).await;
    assert_eq!(w.reversed.lock().unwrap().len(), 1);

    // The dispatch is delivered again: the same hand-over fulfils again.
    delivered(
        &t,
        &order,
        5,
        vec![sent(&line, 10, 10, 0), sent(&cap, 1, 1, 0)],
    )
    .await;
    let again = reload(&t, order.id).await;
    assert_eq!(again.status, SalesOrderStatus::Fulfilled);
    assert_eq!(lines(&t, &again).await[0].fulfilled_milli, 10_000);
    assert_eq!(c.ledger.drafts().len(), 2);
}

/// A reversal of one of two deliveries brings the draft invoice back to the
/// quantity that stays delivered; an issued invoice is not changed and the
/// accountant gets a task.
#[renox::test]
async fn a_draft_invoice_follows_a_reversal_and_an_issued_one_gets_a_task() {
    let (t, c, _w) = boot(true).await;
    let (order, line, _sam) = order_of(&t, &c, "10").await;
    delivered(&t, &order, 5, vec![sent(&line, 6, 6, 0)]).await;
    delivered(&t, &order, 6, vec![sent(&line, 4, 4, 0)]).await;

    reversed(&t, &order, 6, vec![sent(&line, 4, 0, 0)]).await;

    let after = reload(&t, order.id).await;
    assert_eq!(after.status, SalesOrderStatus::PartiallyFulfilled);
    assert_eq!(lines(&t, &after).await[0].fulfilled_milli, 6_000);
    let updates = c.ledger.updates();
    assert_eq!(updates.last().unwrap().1.lines[0].quantity_milli, 6_000);

    let invoice = after.invoice_id.expect("the draft");
    c.ledger.set_status(invoice, InvoiceStatus::Issued);
    reversed(&t, &order, 5, vec![sent(&line, 6, 0, 0)]).await;
    let tasks = c.desk.opened();
    assert!(
        tasks
            .iter()
            .any(|task| task.title.contains("issued invoice is affected")),
        "{:?}",
        tasks.iter().map(|t| t.title.clone()).collect::<Vec<_>>()
    );
}

/// AC6: Given a `closed` order, `DispatchReversed` leaves it `closed` and the
/// accountant and the sales rep get a task.
#[renox::test]
async fn a_reversal_after_the_order_closed_leaves_it_closed_and_opens_two_tasks() {
    let (t, c, _w) = boot(false).await;
    let (order, line, sam) = order_of(&t, &c, "10").await;
    let cap = cap_of(&t, &order).await;
    delivered(
        &t,
        &order,
        5,
        vec![sent(&line, 10, 10, 0), sent(&cap, 1, 1, 0)],
    )
    .await;
    assert!(sales_orders::close(t.state(), order.id).await.unwrap());

    reversed(&t, &order, 5, vec![sent(&line, 10, 0, 0)]).await;

    let after = reload(&t, order.id).await;
    assert_eq!(after.status, SalesOrderStatus::Closed);
    assert_eq!(lines(&t, &after).await[0].fulfilled_milli, 10_000);
    let tasks = c.desk.opened();
    let closed: Vec<_> = tasks
        .iter()
        .filter(|task| task.title.contains("is closed but delivery"))
        .collect();
    assert_eq!(closed.len(), 2);
    assert!(closed.iter().any(|task| task.assignee.is_some()));
    let rep_task = closed
        .iter()
        .find(|task| {
            task.source_key
                .as_deref()
                .is_some_and(|k| k.ends_with(":rep"))
        })
        .expect("the rep's task");
    assert_eq!(
        rep_task.assignee,
        Some(wl_kernel::tickets::Assignee::user(sam.id))
    );
}

/// AC7: Given `DispatchLost` of a `dispatched` dispatch, nothing is fulfilled,
/// the lines show *lost* and the sales rep gets a task; from a `delivered`
/// dispatch `reverse_fulfilment` runs first (cause *lost*).
#[renox::test]
async fn a_lost_delivery_fulfils_nothing_and_gives_the_rep_a_task() {
    let (t, c, w) = boot(false).await;
    let (order, line, sam) = order_of(&t, &c, "10").await;
    w.directory
        .set(vec![facts(&order, "DSP-1", "lost", &line, 10)]);

    t.state()
        .emit(DispatchLost {
            dispatch: dispatch(5),
            source: order.subject(),
            recipient: Some(PartyId(7)),
            was_delivered: false,
            value: 100_000,
            lines: vec![sent(&line, 10, 0, 0)],
        })
        .await
        .unwrap();

    let after = reload(&t, order.id).await;
    assert_eq!(after.status, SalesOrderStatus::Confirmed);
    assert_eq!(lines(&t, &after).await[0].fulfilled_milli, 0);
    let tasks = c.desk.opened();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].title.contains("DSP-1"), "{}", tasks[0].title);
    assert_eq!(
        tasks[0].assignee,
        Some(wl_kernel::tickets::Assignee::user(sam.id))
    );
    t.app.acting_as(&sam);
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Lost");
}

#[renox::test]
async fn a_delivered_dispatch_lost_under_review_reverses_the_fulfilment_first() {
    let (t, c, w) = boot(false).await;
    let (order, line, _sam) = order_of(&t, &c, "10").await;
    delivered(&t, &order, 5, vec![sent(&line, 10, 8, 2)]).await;
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::PartiallyFulfilled
    );

    t.state()
        .emit(DispatchLost {
            dispatch: dispatch(5),
            source: order.subject(),
            recipient: Some(PartyId(7)),
            was_delivered: true,
            value: 80_000,
            lines: vec![sent(&line, 10, 0, 2)],
        })
        .await
        .unwrap();

    let after = reload(&t, order.id).await;
    assert_eq!(after.status, SalesOrderStatus::Confirmed);
    assert_eq!(lines(&t, &after).await[0].fulfilled_milli, 0);
    let events = w.reversed.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cause, "lost");
    assert_eq!(c.desk.opened().len(), 1);

    // The delivery's own loss, naming the dispatch, adds nothing.
    t.state()
        .emit(DeliveryLost {
            delivery: DeliveryId(1),
            source: dispatch(5),
            was_under_review: true,
            ticket: None,
            liability: "none".to_owned(),
            run: None,
            claim_ticket: None,
            lines: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(c.desk.opened().len(), 1);
}

/// An order on credit hold takes the reversal of a delivery: the quantity
/// comes off and the state the release returns to follows (`confirmed` when
/// nothing stays fulfilled); a delivery it cannot take opens a task instead of
/// failing the delivery.
#[renox::test]
async fn an_order_on_credit_hold_takes_a_reversal_and_refuses_a_delivery_with_a_task() {
    let (t, c, _w) = boot(false).await;
    let (order, line, _sam) = order_of(&t, &c, "10").await;
    delivered(&t, &order, 5, vec![sent(&line, 4, 4, 0)]).await;
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(order.id))
        .update(
            t.db(),
            &[
                ("status", &"credit_hold"),
                ("held_from", &Some("partially_fulfilled".to_owned())),
            ],
        )
        .await
        .unwrap();

    delivered(&t, &order, 6, vec![sent(&line, 3, 3, 0)]).await;
    let held = reload(&t, order.id).await;
    assert_eq!(held.status, SalesOrderStatus::CreditHold);
    assert_eq!(lines(&t, &held).await[0].fulfilled_milli, 4_000);
    assert!(
        c.desk
            .opened()
            .iter()
            .any(|task| task.title.contains("was not counted"))
    );

    reversed(&t, &order, 5, vec![sent(&line, 4, 0, 0)]).await;

    let after = reload(&t, order.id).await;
    assert_eq!(after.status, SalesOrderStatus::CreditHold);
    assert_eq!(lines(&t, &after).await[0].fulfilled_milli, 0);
    assert_eq!(after.held_from.as_deref(), Some("confirmed"));
}
