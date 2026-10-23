//! BL-MFG-02 (#383): unlinking a work order from its sales order to stock
//! (07 §4.2 row 2, §7.1 *Work order* `unlink`, `MFG.RUL.09`). The state does
//! not change; the source becomes stock, the planner is told, the history has
//! the reason, and output lots reserved for a cancelled line are released.

mod support;

use manufacturing::unlink::unlink_by_system;
use manufacturing::{WorkOrder, WorkOrderSource, WorkOrderStatus};
use renox::prelude::*;
use support::{Booted, boot};

/// A work order in `status` that serves line `line` of sales order `order`,
/// with output lot 77 when `lot` is set (a fixture).
async fn linked(
    t: &Booted,
    plant: &manufacturing::Plant,
    line: &manufacturing::Line,
    status: WorkOrderStatus,
    order: i64,
    sales_line: i64,
    lot: bool,
) -> WorkOrder {
    let made = t.fixture(plant, line, status).await;
    WorkOrder::query()
        .where_(WorkOrder::ID.eq(made.id))
        .update(
            t.db(),
            &[
                ("source", &WorkOrderSource::SalesOrder),
                ("sales_order_id", &Some(order)),
                ("sales_order_line_id", &Some(sales_line)),
                ("output_lot_id", &lot.then_some(77_i64)),
            ],
        )
        .await
        .unwrap();
    t.reload(made.id).await
}

async fn history(t: &Booted, id: i64) -> Vec<(String, Option<String>)> {
    renox::state_machine::history(t.db(), WorkOrder::TABLE, id)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.transition, row.reason))
        .collect()
}

/// AC1: planner A runs `unlink` on WO-200 (`in_progress`, linked to SO-50
/// line 1) with the reason "customer cancelled, standard item": it stays
/// `in_progress`, the source is stock, the planner is told and the history
/// row has the reason. AC2: later the output is not for SO-50.
#[renox::test]
async fn the_planner_unlinks_a_running_work_order() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let order = linked(&t, &plant, &line, WorkOrderStatus::InProgress, 50, 1, false).await;
    t.lines.open(50, 1, 1_000_000, None);

    t.act(
        &planner,
        &order,
        "unlink",
        &[("reason", "customer cancelled, standard item")],
    )
    .await
    .assert_status(303);

    let now = t.reload(order.id).await;
    assert_eq!(now.status, WorkOrderStatus::InProgress);
    assert_eq!(now.source, WorkOrderSource::Stock);
    assert_eq!(now.order(), None, "its output is not reserved for SO-50");
    assert_eq!(now.quantity_milli, order.quantity_milli);
    assert_eq!(now.line_id, order.line_id);
    assert_eq!(
        now.unlink_reason.as_deref(),
        Some("customer cancelled, standard item")
    );
    let rows = history(&t, order.id).await;
    assert_eq!(
        rows.last().unwrap(),
        &(
            "unlink_in_progress".to_owned(),
            Some("customer cancelled, standard item".to_owned())
        )
    );
    let told = t.notices.titles_for(&planner);
    assert!(
        told.iter().any(|title| title.contains("no longer serves")),
        "{told:?}"
    );
    let events = t.unlinked.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sales_order.subject_id, 50);
    assert_eq!(events[0].sales_order_line, 1);
    // The line is still open: nothing was released.
    assert!(t.stock.released.lock().unwrap().is_empty());
}

/// Every state `unlink` keeps works, and the state stays as it was.
#[renox::test]
async fn unlink_keeps_each_state_it_leaves() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    for (n, status) in [
        WorkOrderStatus::Planned,
        WorkOrderStatus::Released,
        WorkOrderStatus::InProgress,
        WorkOrderStatus::OnHold,
    ]
    .into_iter()
    .enumerate()
    {
        let order = linked(
            &t,
            &plant,
            &line,
            status,
            60,
            i64::try_from(n).unwrap(),
            false,
        )
        .await;
        t.act(&planner, &order, "unlink", &[("reason", "cancelled")])
            .await
            .assert_status(303);
        let now = t.reload(order.id).await;
        assert_eq!((now.status, now.source), (status, WorkOrderSource::Stock));
    }
}

/// AC3: `unlink` without a reason is refused. AC4: a work order already
/// sourced from stock is not offered it; a completed one is not either. Only
/// a holder of `work_orders.release` for the work order may.
#[renox::test]
async fn unlink_needs_a_reason_a_link_and_the_permission() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let other = t.line(&plant, "L2").await;
    let planner = t.planner("Amal", &plant).await;
    let keeper = t.keeper("Karim", &plant).await;
    let stranger = t.supervisor("Sara", &other).await;
    let order = linked(&t, &plant, &line, WorkOrderStatus::InProgress, 50, 1, false).await;

    t.act(&planner, &order, "unlink", &[("reason", " ")])
        .await
        .assert_status(422);
    t.act(&keeper, &order, "unlink", &[("reason", "x")])
        .await
        .assert_forbidden();
    t.act(&stranger, &order, "unlink", &[("reason", "x")])
        .await
        .assert_forbidden();
    assert_eq!(t.reload(order.id).await.source, WorkOrderSource::SalesOrder);

    t.act(&planner, &order, "unlink", &[("reason", "x")])
        .await
        .assert_status(303);
    let again = t.act(&planner, &order, "unlink", &[("reason", "x")]).await;
    again.assert_status(422);
    assert!(again.text().contains("not linked"), "{}", again.text());

    let done = linked(&t, &plant, &line, WorkOrderStatus::Completed, 50, 2, true).await;
    t.act(&planner, &done, "unlink", &[("reason", "x")])
        .await
        .assert_status(422);
}

/// AC5: lot F9 of WO-201 is reserved for SO-51 line 2; line 2 is cancelled
/// and WO-201 is unlinked: F9 is released to free stock. A line that is still
/// open keeps its reservation.
#[renox::test]
async fn the_output_reserved_for_a_cancelled_line_is_released() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let order = linked(&t, &plant, &line, WorkOrderStatus::InProgress, 51, 2, true).await;
    t.lines.open(51, 2, 500_000, None);
    t.lines.cancel(51, 2);

    t.act(&planner, &order, "unlink", &[("reason", "line cancelled")])
        .await
        .assert_status(303);

    let released = t.stock.released.lock().unwrap().clone();
    assert_eq!(released.len(), 1);
    assert_eq!(
        (released[0].0.subject_id, released[0].1, released[0].2),
        (77, 51, 2)
    );
}

/// The system's `unlink` (`MFG.RUL.09`) does the same, and a call on a work
/// order that is already unlinked does nothing: no history row, no notice.
#[renox::test]
async fn the_system_unlinks_once_and_a_replay_changes_nothing() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let order = linked(&t, &plant, &line, WorkOrderStatus::OnHold, 52, 1, false).await;

    let first = unlink_by_system(t.state(), order.id, "finish to stock")
        .await
        .unwrap();
    assert_eq!(first.map(|o| o.source), Some(WorkOrderSource::Stock));
    let rows = history(&t, order.id).await.len();
    let titles = t.notices.titles_for(&planner).len();
    let second = unlink_by_system(t.state(), order.id, "finish to stock")
        .await
        .unwrap();

    assert!(second.is_none());
    assert_eq!(history(&t, order.id).await.len(), rows);
    assert_eq!(t.notices.titles_for(&planner).len(), titles);
    assert_eq!(t.unlinked.all().len(), 1);
    assert_eq!(t.reload(order.id).await.status, WorkOrderStatus::OnHold);
}
