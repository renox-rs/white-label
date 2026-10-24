//! BL-STK-26 (#245), X-14, 07-N2-a: the outcome of a cancelled sales order
//! line is applied to stock. Return to stock cancels the dispatch or removes
//! the lines and releases the reservation; scrap posts wastage with the
//! reason *cancelled order* and the order as reference; finish to stock moves
//! nothing; a dispatch that already left is logged and the dispatcher gets a
//! task. The events are emitted as wl-sales emits them.

mod support;

use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::dispatch::{
    Dispatch, DispatchLine, DispatchStatus, NewDispatch, NewDispatchLine, SALE_LINE_SUBJECT_TYPE,
    create_draft, lines_of,
};
use wl_inventory::movement::MovementType;
use wl_inventory::sales_cancel::{CancelEffect, SCRAP_REASON};
use wl_inventory::{Item, Location};
use wl_kernel::inventory::{LineComponent, SaleLineStock};
use wl_kernel::sales::{
    AmendedLine, LineOutcome, LineOutcomeKind, SalesOrderAmended, SalesOrderCancelled,
    SalesOrderId, SalesOrderLineCancelled,
};
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

const ORDER: i64 = 7;

fn order() -> Subject {
    SalesOrderId(ORDER).subject()
}

fn line_cancelled(
    line: i64,
    quantity_milli: i64,
    outcome: Option<LineOutcomeKind>,
) -> SalesOrderLineCancelled {
    SalesOrderLineCancelled {
        order: SalesOrderId(ORDER),
        party: PartyId(1),
        line,
        quantity_milli,
        reason: "customer changed mind".to_owned(),
        outcome,
        fee: 0,
        fee_waived: false,
    }
}

/// A picked dispatch of the order with one line per `(item, quantity, sale
/// line)` at `location`, picked by a dispatcher.
async fn picked(t: &Booted, location: &Location, lines: &[(&Item, i64, i64)]) -> Dispatch {
    let dispatch = create_draft(
        t.db(),
        None,
        NewDispatch {
            source: order(),
            recipient: Some(PartyId(1)),
            recipient_name: "Customer".to_owned(),
            address: String::new(),
            location_id: location.id,
            lines: lines
                .iter()
                .map(|(item, quantity, sale_line)| {
                    NewDispatchLine::of(item.id, *quantity).for_sale_line(*sale_line)
                })
                .collect(),
        },
    )
    .await
    .unwrap();
    let dispatcher = t.dispatcher("Dana").await;
    t.app.acting_as(&dispatcher);
    t.app
        .htmx()
        .post(&format!("/dispatches/{}/pick", dispatch.id), &[])
        .await
        .assert_ok();
    reload(t, &dispatch).await
}

async fn reload(t: &Booted, dispatch: &Dispatch) -> Dispatch {
    Dispatch::find_or_404(t.db(), dispatch.id).await.unwrap()
}

async fn wastage_of(t: &Booted, item: &Item) -> Vec<wl_inventory::movement::StockMovement> {
    t.movements_of(item)
        .await
        .into_iter()
        .filter(|m| m.movement_type == MovementType::Wastage)
        .collect()
}

/// AC1: a picked dispatch for one line and `SalesOrderLineCancelled` with
/// *return to stock*: the dispatch is `cancelled` and the reservation is
/// released.
#[renox::test]
async fn a_picked_dispatch_of_the_cancelled_line_is_cancelled_and_its_reservation_released() {
    let t = boot().await;
    let item = t.item("BURGER-KIT").await;
    let store = t.location("OUTLET-1", None).await;
    t.stock_in(&item, &store, None, 5_000, 300).await;
    let dispatch = picked(&t, &store, &[(&item, 3_000, 11)]).await;
    assert_eq!(dispatch.status, DispatchStatus::Picked);
    assert_eq!(t.available(&item, &store, None).await, 2_000);

    t.state()
        .emit(line_cancelled(
            11,
            3_000,
            Some(LineOutcomeKind::ReturnToStock),
        ))
        .await
        .unwrap();

    assert_eq!(
        reload(&t, &dispatch).await.status,
        DispatchStatus::Cancelled
    );
    assert_eq!(t.available(&item, &store, None).await, 5_000);
    assert!(wastage_of(&t, &item).await.is_empty());
    let effects = CancelEffect::of_order(t.db(), ORDER).await.unwrap();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].outcome.as_deref(), Some("return_to_stock"));
}

/// AC2: a two-line dispatch with one line cancelled keeps the other line; the
/// cancelled line's reservation is released.
#[renox::test]
async fn a_dispatch_with_other_lines_keeps_them_when_one_line_is_cancelled() {
    let t = boot().await;
    let rice = t.item("RICE").await;
    let oil = t.item("OIL").await;
    let store = t.location("OUTLET-1", None).await;
    t.stock_in(&rice, &store, None, 5_000, 300).await;
    t.stock_in(&oil, &store, None, 5_000, 300).await;
    let dispatch = picked(&t, &store, &[(&rice, 2_000, 11), (&oil, 1_000, 12)]).await;

    t.state()
        .emit(line_cancelled(
            11,
            2_000,
            Some(LineOutcomeKind::ReturnToStock),
        ))
        .await
        .unwrap();

    assert_eq!(reload(&t, &dispatch).await.status, DispatchStatus::Picked);
    let left = lines_of(t.db(), dispatch.id).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!((left[0].item_id, left[0].quantity_milli), (oil.id, 1_000));
    assert_eq!(t.available(&rice, &store, None).await, 5_000);
    assert_eq!(t.available(&oil, &store, None).await, 4_000);
}

/// AC3: scrap of 3 prepared meals (recipe not yet consumed) worth AED 90 at an
/// outlet: wastage movements of the components are posted with reason
/// *cancelled order* and the order as reference; the cost is posted.
#[renox::test]
async fn scrapping_prepared_meals_posts_wastage_of_the_components_with_the_order_as_reference() {
    let t = boot().await;
    let chicken = t.item("CHICKEN").await;
    let bun = t.item("BUN").await;
    let outlet = t.location("OUTLET-1", None).await;
    t.stock_in(&chicken, &outlet, None, 20_000, 1_000).await;
    t.stock_in(&bun, &outlet, None, 20_000, 500).await;
    let mut chicken_row = Item::find_or_404(t.db(), chicken.id).await.unwrap();
    chicken_row.average_cost = 1_000;
    chicken_row.save(t.db()).await.unwrap();
    let mut bun_row = Item::find_or_404(t.db(), bun.id).await.unwrap();
    bun_row.average_cost = 500;
    bun_row.save(t.db()).await.unwrap();
    // One meal: 0.2 kg of chicken (AED 2) and 1 bun (AED 0.50)... three meals
    // are 0.6 kg and 3 buns.
    t.sale_lines.set(
        21,
        SaleLineStock {
            location: outlet.subject(),
            components: vec![
                LineComponent {
                    item: chicken.subject(),
                    per_unit_milli: 200,
                },
                LineComponent {
                    item: bun.subject(),
                    per_unit_milli: 1_000,
                },
            ],
            quantity_milli: 3_000,
            consumed: false,
            value: 9_000,
        },
    );

    t.state()
        .emit(line_cancelled(21, 3_000, Some(LineOutcomeKind::Scrap)))
        .await
        .unwrap();

    let chicken_waste = wastage_of(&t, &chicken).await;
    let bun_waste = wastage_of(&t, &bun).await;
    assert_eq!(chicken_waste.len(), 1);
    assert_eq!(chicken_waste[0].quantity_milli, 600);
    assert_eq!(bun_waste[0].quantity_milli, 3_000);
    for moved in chicken_waste.iter().chain(&bun_waste) {
        assert_eq!(moved.reason.as_deref(), Some(SCRAP_REASON));
        assert_eq!(moved.source(), order());
        assert!(moved.theoretical);
    }
    let posted = t.costs.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].cost_code.as_deref(), Some("wastage"));
    assert_eq!(posted[0].amount_aed, 600 + 1_500);
}

/// The recipe was already consumed: no second movement, the line is reported
/// as wastage value.
#[renox::test]
async fn scrapping_a_line_that_was_already_consumed_posts_no_second_movement() {
    let t = boot().await;
    let chicken = t.item("CHICKEN").await;
    let outlet = t.location("OUTLET-1", None).await;
    t.stock_in(&chicken, &outlet, None, 20_000, 1_000).await;
    t.sale_lines.set(
        21,
        SaleLineStock {
            location: outlet.subject(),
            components: vec![LineComponent {
                item: chicken.subject(),
                per_unit_milli: 200,
            }],
            quantity_milli: 3_000,
            consumed: true,
            value: 9_000,
        },
    );

    t.state()
        .emit(line_cancelled(21, 3_000, Some(LineOutcomeKind::Scrap)))
        .await
        .unwrap();

    assert!(wastage_of(&t, &chicken).await.is_empty());
    let effects = CancelEffect::of_order(t.db(), ORDER).await.unwrap();
    assert_eq!(effects[0].action, "reported");
    assert_eq!(effects[0].value, 9_000);
    assert_eq!(t.costs.posted()[0].amount_aed, 9_000);
}

/// Scrap of picked goods posts wastage from the dispatch line and removes it.
#[renox::test]
async fn scrapping_picked_goods_posts_wastage_and_removes_the_line() {
    let t = boot().await;
    let item = t.item("CAKE").await;
    let mut row = Item::find_or_404(t.db(), item.id).await.unwrap();
    row.average_cost = 400;
    row.save(t.db()).await.unwrap();
    let store = t.location("OUTLET-1", None).await;
    t.stock_in(&item, &store, None, 5_000, 400).await;
    let dispatch = picked(&t, &store, &[(&item, 2_000, 31)]).await;

    t.state()
        .emit(line_cancelled(31, 2_000, Some(LineOutcomeKind::Scrap)))
        .await
        .unwrap();

    let waste = wastage_of(&t, &item).await;
    assert_eq!(waste.len(), 1);
    assert_eq!((waste[0].quantity_milli, waste[0].value), (2_000, 800));
    assert_eq!(waste[0].source(), order());
    assert_eq!(
        reload(&t, &dispatch).await.status,
        DispatchStatus::Cancelled
    );
}

/// AC4: *finish to stock* moves nothing and posts no wastage.
#[renox::test]
async fn finish_to_stock_posts_no_movement() {
    let t = boot().await;
    let item = t.item("CAKE").await;
    let store = t.location("OUTLET-1", None).await;
    t.stock_in(&item, &store, None, 5_000, 400).await;
    let before = t.movements_of(&item).await.len();

    t.state()
        .emit(line_cancelled(
            41,
            2_000,
            Some(LineOutcomeKind::FinishToStock),
        ))
        .await
        .unwrap();

    assert_eq!(t.movements_of(&item).await.len(), before);
    assert!(t.costs.posted().is_empty());
}

/// A dispatch that already left is not cancelled: the event is logged and the
/// dispatcher gets a task.
#[renox::test]
async fn a_dispatch_that_already_left_is_logged_and_the_dispatcher_gets_a_task() {
    let t = boot().await;
    let item = t.item("CAKE").await;
    let store = t.location("OUTLET-1", None).await;
    let dispatcher = t.dispatcher("Dana").await;
    let mut dispatch = Dispatch {
        location_id: store.id,
        source_type: order().subject_type,
        source_id: ORDER,
        status: DispatchStatus::Dispatched,
        ..Dispatch::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    dispatch
        .insert_in_state(&mut tx, DispatchStatus::Dispatched, "left the outlet")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut line = DispatchLine {
        dispatch_id: dispatch.id,
        item_id: item.id,
        quantity_milli: 2_000,
        source_line_type: Some(SALE_LINE_SUBJECT_TYPE.to_owned()),
        source_line_id: Some(51),
        ..DispatchLine::default()
    };
    line.insert(t.db()).await.unwrap();

    t.state()
        .emit(line_cancelled(
            51,
            2_000,
            Some(LineOutcomeKind::ReturnToStock),
        ))
        .await
        .unwrap();

    assert_eq!(
        reload(&t, &dispatch).await.status,
        DispatchStatus::Dispatched
    );
    assert_eq!(lines_of(t.db(), dispatch.id).await.unwrap().len(), 1);
    let effects = CancelEffect::of_order(t.db(), ORDER).await.unwrap();
    assert_eq!(effects[0].action, "task");
    let tasks = t.tickets.opened();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].assignee,
        Some(wl_kernel::tickets::Assignee::user(dispatcher.id))
    );
}

/// Idempotency: the same event twice acts once.
#[renox::test]
async fn the_same_cancellation_twice_is_applied_once() {
    let t = boot().await;
    let item = t.item("CAKE").await;
    let store = t.location("OUTLET-1", None).await;
    t.stock_in(&item, &store, None, 9_000, 400).await;
    picked(&t, &store, &[(&item, 2_000, 61), (&item, 2_000, 62)]).await;
    let event = line_cancelled(61, 2_000, Some(LineOutcomeKind::Scrap));

    t.state().emit(event.clone()).await.unwrap();
    t.state().emit(event).await.unwrap();

    assert_eq!(wastage_of(&t, &item).await.len(), 1);
    assert_eq!(
        CancelEffect::of_order(t.db(), ORDER).await.unwrap().len(),
        1
    );
}

/// A cancelled order acts on each started line's outcome and cancels what had
/// not started.
#[renox::test]
async fn a_cancelled_order_applies_each_outcome_and_cancels_the_rest_of_its_dispatches() {
    let t = boot().await;
    let rice = t.item("RICE").await;
    let cake = t.item("CAKE").await;
    let mut row = Item::find_or_404(t.db(), cake.id).await.unwrap();
    row.average_cost = 400;
    row.save(t.db()).await.unwrap();
    let store = t.location("OUTLET-1", None).await;
    t.stock_in(&rice, &store, None, 5_000, 300).await;
    t.stock_in(&cake, &store, None, 5_000, 400).await;
    let dispatch = picked(&t, &store, &[(&rice, 2_000, 71), (&cake, 1_000, 72)]).await;

    t.state()
        .emit(SalesOrderCancelled {
            order: SalesOrderId(ORDER),
            party: PartyId(1),
            reason: "closed".to_owned(),
            outcomes: vec![
                LineOutcome {
                    line: 71,
                    kind: LineOutcomeKind::ReturnToStock,
                    fee: 0,
                    fee_waived: false,
                },
                LineOutcome {
                    line: 72,
                    kind: LineOutcomeKind::Scrap,
                    fee: 0,
                    fee_waived: false,
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        reload(&t, &dispatch).await.status,
        DispatchStatus::Cancelled
    );
    assert!(wastage_of(&t, &rice).await.is_empty());
    assert_eq!(wastage_of(&t, &cake).await.len(), 1);
    assert_eq!(t.available(&rice, &store, None).await, 5_000);
}

/// An amendment that lowers a line trims its dispatch line and shrinks the
/// reservation.
#[renox::test]
async fn an_amendment_that_lowers_a_line_shrinks_the_reservation() {
    let t = boot().await;
    let item = t.item("RICE").await;
    let store = t.location("OUTLET-1", None).await;
    t.stock_in(&item, &store, None, 5_000, 300).await;
    let dispatch = picked(&t, &store, &[(&item, 3_000, 81)]).await;

    t.state()
        .emit(SalesOrderAmended {
            order: SalesOrderId(ORDER),
            party: PartyId(1),
            revision: 2,
            lines: vec![AmendedLine {
                line: 81,
                quantity_before_milli: 3_000,
                quantity_after_milli: 2_000,
                unit_price_before: 100,
                unit_price_after: 100,
                held_milli: 0,
            }],
            value_before: 300,
            value_after: 200,
            reason: "fewer".to_owned(),
        })
        .await
        .unwrap();

    let left = lines_of(t.db(), dispatch.id).await.unwrap();
    assert_eq!(left[0].quantity_milli, 2_000);
    assert_eq!(t.available(&item, &store, None).await, 3_000);
    assert_eq!(reload(&t, &dispatch).await.status, DispatchStatus::Picked);
}

/// An amendment that raises a line reserves the extra stock: the picked
/// dispatch line grows and the available quantity falls with it; applied
/// twice it counts once.
#[renox::test]
async fn an_amendment_that_raises_a_line_reserves_the_extra_stock() {
    let t = boot().await;
    let item = t.item("RICE").await;
    let store = t.location("OUTLET-1", None).await;
    t.stock_in(&item, &store, None, 10_000, 300).await;
    let dispatch = picked(&t, &store, &[(&item, 3_000, 81)]).await;
    assert_eq!(t.available(&item, &store, None).await, 7_000);

    let event = SalesOrderAmended {
        order: SalesOrderId(ORDER),
        party: PartyId(1),
        revision: 2,
        lines: vec![AmendedLine {
            line: 81,
            quantity_before_milli: 3_000,
            quantity_after_milli: 5_000,
            unit_price_before: 100,
            unit_price_after: 100,
            held_milli: 0,
        }],
        value_before: 300,
        value_after: 500,
        reason: "two more".to_owned(),
    };
    t.state().emit(event.clone()).await.unwrap();
    t.state().emit(event).await.unwrap();

    let grown = lines_of(t.db(), dispatch.id).await.unwrap();
    assert_eq!(grown[0].quantity_milli, 5_000);
    assert_eq!(t.available(&item, &store, None).await, 5_000);
    assert_eq!(reload(&t, &dispatch).await.status, DispatchStatus::Picked);
}

/// A raise beyond the stock available reserves what is there and notes the
/// rest on the effect; a line with no dispatch in the store gets nothing.
#[renox::test]
async fn an_amendment_that_raises_a_line_reserves_only_what_the_stock_covers() {
    let t = boot().await;
    let item = t.item("RICE").await;
    let store = t.location("OUTLET-1", None).await;
    t.stock_in(&item, &store, None, 4_000, 300).await;
    let dispatch = picked(&t, &store, &[(&item, 3_000, 81)]).await;

    t.state()
        .emit(SalesOrderAmended {
            order: SalesOrderId(ORDER),
            party: PartyId(1),
            revision: 2,
            lines: vec![
                AmendedLine {
                    line: 81,
                    quantity_before_milli: 3_000,
                    quantity_after_milli: 6_000,
                    unit_price_before: 100,
                    unit_price_after: 100,
                    held_milli: 0,
                },
                AmendedLine {
                    line: 82,
                    quantity_before_milli: 1_000,
                    quantity_after_milli: 2_000,
                    unit_price_before: 100,
                    unit_price_after: 100,
                    held_milli: 0,
                },
            ],
            value_before: 400,
            value_after: 800,
            reason: "more".to_owned(),
        })
        .await
        .unwrap();

    let grown = lines_of(t.db(), dispatch.id).await.unwrap();
    assert_eq!(
        grown[0].quantity_milli, 4_000,
        "only the 1 kg that was free"
    );
    assert_eq!(t.available(&item, &store, None).await, 0);
    let effects = wl_inventory::sales_cancel::CancelEffect::of_order(t.db(), ORDER)
        .await
        .unwrap();
    let first = effects.iter().find(|e| e.sale_line_id == 81).unwrap();
    assert_eq!(first.action, "added");
    assert!(
        first
            .note
            .as_deref()
            .unwrap_or_default()
            .contains("2000 short")
    );
    let second = effects.iter().find(|e| e.sale_line_id == 82).unwrap();
    assert_eq!(second.action, "none");
}

/// The screen: a dispatcher picks a draft, cancelling needs a reason, and a
/// user without the permission is refused.
#[renox::test]
async fn the_dispatch_screen_picks_and_cancels_for_a_dispatcher_only() {
    let t = boot().await;
    let item = t.item("RICE").await;
    let store = t.location("OUTLET-1", None).await;
    t.stock_in(&item, &store, None, 5_000, 300).await;
    let dispatch = create_draft(
        t.db(),
        None,
        NewDispatch {
            source: order(),
            recipient: None,
            recipient_name: "Customer".to_owned(),
            address: String::new(),
            location_id: store.id,
            lines: vec![NewDispatchLine::of(item.id, 1_000).for_sale_line(91)],
        },
    )
    .await
    .unwrap();
    let dispatcher = t.dispatcher("Dana").await;
    let clerk = t.holder("Hal").await;

    t.app.acting_as(&clerk);
    t.app
        .htmx()
        .post(&format!("/dispatches/{}/pick", dispatch.id), &[])
        .await
        .assert_status(403);
    t.app.acting_as(&dispatcher);
    t.app
        .htmx()
        .post(
            &format!("/dispatches/{}/cancel", dispatch.id),
            &[("reason", " ")],
        )
        .await
        .assert_status(422);
    t.browse(&dispatcher, "/dispatches").await.assert_ok();
    t.app.acting_as(&dispatcher);
    t.app
        .htmx()
        .post(
            &format!("/dispatches/{}/cancel", dispatch.id),
            &[("reason", "customer called")],
        )
        .await
        .assert_ok();

    let after = reload(&t, &dispatch).await;
    assert_eq!(after.status, DispatchStatus::Cancelled);
    assert_eq!(after.cancel_reason.as_deref(), Some("customer called"));
}
