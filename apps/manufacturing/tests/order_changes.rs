//! BL-MFG-03 (#384): the work orders follow their sales order (07 §4.1 rows 1
//! and 2, §7.1 *Sales order* `amend`, `cancel_line`, `cancel_remaining`,
//! `cancel` and *Work order* `cancel`, `unlink`, `abort`; `MFG.RUL.09`,
//! `SAL.ACL.05`). The sales order events are emitted by hand (wl-sales emits
//! them after any approval); the work orders, their machine, the journal, the
//! tasks and the notices are the real ones.

mod support;

use manufacturing::order_changes::{Adjustment, abort_task_key};
use manufacturing::pick_list;
use manufacturing::{WorkOrder, WorkOrderSource, WorkOrderStatus};
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::sales::FulfilledLine;
use wl_kernel::sales::{
    AmendedLine, LineOutcome, LineOutcomeKind, SalesOrderAmended, SalesOrderCancelled,
    SalesOrderId, SalesOrderLineCancelled, SalesOrderLinesReleased,
};

use support::{Booted, boot};

const ORDER: i64 = 60;

/// A work order in `status` for `quantity_milli` that serves line `line` of
/// sales order 60, planned from `start` (a fixture).
async fn linked(
    t: &Booted,
    plant: &manufacturing::Plant,
    line: &manufacturing::Line,
    status: WorkOrderStatus,
    sales_line: i64,
    quantity_milli: i64,
    start: u32,
) -> WorkOrder {
    let made = t.fixture(plant, line, status).await;
    WorkOrder::query()
        .where_(WorkOrder::ID.eq(made.id))
        .update(
            t.db(),
            &[
                ("source", &WorkOrderSource::SalesOrder),
                ("sales_order_id", &Some(ORDER)),
                ("sales_order_line_id", &Some(sales_line)),
                ("quantity_milli", &quantity_milli),
                ("planned_start", &Some(support::date(2027, 3, start))),
            ],
        )
        .await
        .unwrap();
    t.reload(made.id).await
}

/// Marks material as issued to the work order.
async fn with_issue(t: &Booted, order: &WorkOrder) {
    WorkOrder::query()
        .where_(WorkOrder::ID.eq(order.id))
        .update(t.db(), &[("has_issue", &true)])
        .await
        .unwrap();
}

async fn amended(t: &Booted, revision: i64, line: i64, before: i64, after: i64, held: i64) {
    t.state()
        .emit(SalesOrderAmended {
            order: SalesOrderId(ORDER),
            party: PartyId(7),
            revision,
            lines: vec![AmendedLine {
                line,
                quantity_before_milli: before,
                quantity_after_milli: after,
                unit_price_before: 100,
                unit_price_after: 100,
                held_milli: held,
            }],
            value_before: 0,
            value_after: 0,
            reason: "customer changed the quantity".to_owned(),
        })
        .await
        .unwrap();
}

async fn line_cancelled(
    t: &Booted,
    line: i64,
    quantity_milli: i64,
    outcome: Option<LineOutcomeKind>,
) {
    t.state()
        .emit(SalesOrderLineCancelled {
            order: SalesOrderId(ORDER),
            party: PartyId(7),
            line,
            quantity_milli,
            reason: "customer cancelled".to_owned(),
            outcome,
            fee: 0,
            fee_waived: false,
        })
        .await
        .unwrap();
}

async fn linked_to_line(t: &Booted, line: i64) -> Vec<WorkOrder> {
    WorkOrder::query()
        .where_(WorkOrder::SALES_ORDER_ID.eq(ORDER))
        .where_(WorkOrder::SALES_ORDER_LINE_ID.eq(line))
        .order_by(WorkOrder::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// AC1: Given SO-60 line 1 qty 1,000 with WO-300 `planned` for 1,000, when the
/// amendment to 800 is final, WO-300 is for 800 and the planner is notified.
#[renox::test]
async fn an_amendment_to_800_changes_the_planned_work_order_and_tells_the_planner() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let wo = linked(&t, &plant, &line, WorkOrderStatus::Planned, 1, 1_000_000, 5).await;
    t.lines.set(ORDER, 1, 800_000);

    amended(&t, 2, 1, 1_000_000, 800_000, 0).await;

    let now = t.reload(wo.id).await;
    assert_eq!(now.quantity_milli, 800_000);
    assert_eq!(now.status, WorkOrderStatus::Planned);
    let told = t.notices.titles_for(&planner);
    assert!(
        told.iter()
            .any(|title| title.contains("work orders adjusted")),
        "{told:?}"
    );
    let rows = Adjustment::of_work_order(t.db(), wo.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].from_milli, rows[0].to_milli), (1_000_000, 800_000));
}

/// The pick list of a `released` work order is made again for the new
/// quantity; at 0 the work order is `cancelled` and its pick list withdrawn.
#[renox::test]
async fn a_released_work_order_gets_a_new_pick_list_and_at_zero_is_cancelled_with_it_withdrawn() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let wo = t.released(&planner, &plant, &line).await;
    WorkOrder::query()
        .where_(WorkOrder::ID.eq(wo.id))
        .update(
            t.db(),
            &[
                ("source", &WorkOrderSource::SalesOrder),
                ("sales_order_id", &Some(ORDER)),
                ("sales_order_line_id", &Some(1_i64)),
            ],
        )
        .await
        .unwrap();
    let before = pick_list::of_order(t.db(), wo.id).await.unwrap();
    let resin = before
        .iter()
        .find(|line| line.item_id == support::RESIN)
        .unwrap()
        .required_milli;
    t.lines.set(ORDER, 1, 800_000);

    amended(&t, 2, 1, 1_000_000, 800_000, 0).await;

    let now = t.reload(wo.id).await;
    assert_eq!(now.quantity_milli, 800_000);
    let after = pick_list::of_order(t.db(), wo.id).await.unwrap();
    let new_resin = after
        .iter()
        .find(|line| line.item_id == support::RESIN)
        .unwrap()
        .required_milli;
    assert_eq!(new_resin * 10, resin * 8, "the resin follows the quantity");

    // AC2: the amendment to 0 cancels it and withdraws the list.
    t.lines.set(ORDER, 1, 0);
    amended(&t, 3, 1, 800_000, 0, 0).await;
    let gone = t.reload(wo.id).await;
    assert_eq!(gone.status, WorkOrderStatus::Cancelled);
    assert!(
        pick_list::of_order(t.db(), wo.id)
            .await
            .unwrap()
            .iter()
            .all(|line| line.withdrawn)
    );
}

/// AC3: Given WO-301 `in_progress` for 1,000 and an increase to 1,300, a new
/// `planned` WO for 300 is linked to the line, with the same item and BOM; the
/// planner is told; WO-301 keeps its quantity.
#[renox::test]
async fn an_increase_on_a_running_work_order_makes_a_new_planned_one_for_the_difference() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let running = linked(
        &t,
        &plant,
        &line,
        WorkOrderStatus::InProgress,
        1,
        1_000_000,
        5,
    )
    .await;
    t.lines.set(ORDER, 1, 1_300_000);

    amended(&t, 2, 1, 1_000_000, 1_300_000, 0).await;

    let all = linked_to_line(&t, 1).await;
    assert_eq!(all.len(), 2);
    let extra = all.iter().find(|wo| wo.id != running.id).unwrap();
    assert_eq!(extra.status, WorkOrderStatus::Planned);
    assert_eq!(extra.quantity_milli, 300_000);
    assert_eq!(extra.item_id, running.item_id);
    assert_eq!(extra.bom_id, running.bom_id);
    assert_eq!(extra.source, WorkOrderSource::SalesOrder);
    assert_eq!(t.reload(running.id).await.quantity_milli, 1_000_000);
    let told = t.notices.titles_for(&planner);
    assert!(
        told.iter().any(|title| title.contains("adjusted")),
        "{told:?}"
    );

    // The same amendment again makes nothing more.
    amended(&t, 2, 1, 1_000_000, 1_300_000, 0).await;
    assert_eq!(linked_to_line(&t, 1).await.len(), 2);
}

/// AC4: Given WO-301 `in_progress` and the line cancelled with outcome
/// *finish to stock*, WO-301 is `unlink`ed (it keeps running for stock).
#[renox::test]
async fn finish_to_stock_unlinks_the_running_work_order() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let _planner = t.planner("Amal", &plant).await;
    let running = linked(
        &t,
        &plant,
        &line,
        WorkOrderStatus::InProgress,
        1,
        1_000_000,
        5,
    )
    .await;
    t.lines.cancel(ORDER, 1);

    line_cancelled(&t, 1, 1_000_000, Some(LineOutcomeKind::FinishToStock)).await;

    let now = t.reload(running.id).await;
    assert_eq!(now.status, WorkOrderStatus::InProgress);
    assert_eq!(now.source, WorkOrderSource::Stock);
    assert_eq!(now.quantity_milli, 1_000_000);
    assert_eq!(t.unlinked.all().len(), 1);
}

/// AC5: Given WO-301 `in_progress` with material issued and outcome *scrap*,
/// the production manager gets an abort task with the work order named and the
/// reason ready for the abort sheet; WO-301 stays `in_progress`, nothing is
/// aborted; a replay opens no second task.
#[renox::test]
async fn scrap_asks_the_production_manager_to_request_the_abort_and_aborts_nothing() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let manager = t.manager("Mona", &plant).await;
    let planner = t.planner("Amal", &plant).await;
    let running = linked(
        &t,
        &plant,
        &line,
        WorkOrderStatus::InProgress,
        1,
        1_000_000,
        5,
    )
    .await;
    with_issue(&t, &running).await;
    t.lines.cancel(ORDER, 1);

    line_cancelled(&t, 1, 1_000_000, Some(LineOutcomeKind::Scrap)).await;

    let now = t.reload(running.id).await;
    assert_eq!(now.status, WorkOrderStatus::InProgress);
    assert_eq!(now.source, WorkOrderSource::SalesOrder);
    assert!(t.aborted.all().is_empty());
    let tasks = t.tickets.all();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].ticket.title.contains("Request the abort"));
    assert_eq!(
        tasks[0].ticket.source_key.as_deref(),
        Some(abort_task_key(running.id).as_str())
    );
    assert_eq!(tasks[0].ticket.subject, running.subject());
    assert_eq!(
        tasks[0].ticket.assignee,
        Some(wl_kernel::tickets::Assignee::user(manager.id))
    );
    assert!(tasks[0].ticket.description.contains("scrap"));
    assert!(
        t.notices
            .titles_for(&planner)
            .iter()
            .any(|title| title.contains("adjusted"))
    );

    // The page names the cancelled order and fills the abort sheet.
    let page = t
        .browse(
            &manager,
            &format!("/manufacturing/work-orders/{}", running.id),
        )
        .await;
    page.assert_ok();
    let text = page.text();
    assert!(
        text.contains("Request the abort of this work order"),
        "{text}"
    );
    assert!(text.contains("SO-60 cancelled, outcome scrap"));
    assert!(text.contains("Changes from the sales order"));
    assert!(text.contains("Abort asked for"));

    line_cancelled(&t, 1, 1_000_000, Some(LineOutcomeKind::Scrap)).await;
    assert_eq!(t.tickets.all().len(), 1);
}

/// AC6: Given WO-302 `in_progress` with nothing issued or reported and outcome
/// *scrap*, WO-302 is `cancelled`.
#[renox::test]
async fn scrap_cancels_a_running_work_order_that_made_nothing() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let _planner = t.planner("Amal", &plant).await;
    let idle = linked(
        &t,
        &plant,
        &line,
        WorkOrderStatus::InProgress,
        1,
        1_000_000,
        5,
    )
    .await;
    t.lines.cancel(ORDER, 1);

    line_cancelled(&t, 1, 1_000_000, Some(LineOutcomeKind::Scrap)).await;

    let now = t.reload(idle.id).await;
    assert_eq!(now.status, WorkOrderStatus::Cancelled);
    assert!(t.tickets.all().is_empty());
}

/// AC7: Given WO-301 `in_progress` and a decrease from 1,000 to 600, WO-301
/// keeps 1,000 and 400 of its output is flagged *for stock*; the page says so.
/// An increase back takes the flag off before it makes a work order.
#[renox::test]
async fn a_decrease_flags_the_surplus_of_a_running_work_order_for_stock() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let running = linked(
        &t,
        &plant,
        &line,
        WorkOrderStatus::InProgress,
        1,
        1_000_000,
        5,
    )
    .await;
    t.lines.set(ORDER, 1, 600_000);

    amended(&t, 2, 1, 1_000_000, 600_000, 0).await;

    let now = t.reload(running.id).await;
    assert_eq!(now.quantity_milli, 1_000_000);
    assert_eq!(now.quantity_for_stock_milli, 400_000);
    assert_eq!(now.source, WorkOrderSource::SalesOrder);
    let page = t
        .browse(
            &planner,
            &format!("/manufacturing/work-orders/{}", running.id),
        )
        .await;
    assert!(page.text().contains("Quantity for stock"));

    // Back to 800: 200 of the flag goes, no new work order.
    t.lines.set(ORDER, 1, 800_000);
    amended(&t, 3, 1, 600_000, 800_000, 0).await;
    let after = t.reload(running.id).await;
    assert_eq!(after.quantity_for_stock_milli, 200_000);
    assert_eq!(linked_to_line(&t, 1).await.len(), 1);

    // Up to 1,300: the last 200 of the flag goes and 300 is a new work order.
    t.lines.set(ORDER, 1, 1_300_000);
    amended(&t, 4, 1, 800_000, 1_300_000, 0).await;
    assert_eq!(t.reload(running.id).await.quantity_for_stock_milli, 0);
    let all = linked_to_line(&t, 1).await;
    assert_eq!(all.len(), 2);
    assert_eq!(
        all.iter().map(|wo| wo.quantity_milli).sum::<i64>(),
        1_300_000
    );
}

/// AC8: replaying the same `SalesOrderLineCancelled` changes nothing: no
/// second change, no second notice.
#[renox::test]
async fn replaying_a_line_cancellation_changes_nothing() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let planned = linked(&t, &plant, &line, WorkOrderStatus::Planned, 1, 1_000_000, 5).await;
    t.lines.set(ORDER, 1, 600_000);

    line_cancelled(&t, 1, 400_000, None).await;
    let first = t.notices.titles_for(&planner).len();
    let rows = Adjustment::of_work_order(t.db(), planned.id).await.unwrap();
    assert_eq!(t.reload(planned.id).await.quantity_milli, 600_000);

    line_cancelled(&t, 1, 400_000, None).await;

    assert_eq!(t.reload(planned.id).await.quantity_milli, 600_000);
    assert_eq!(t.notices.titles_for(&planner).len(), first);
    assert_eq!(
        Adjustment::of_work_order(t.db(), planned.id)
            .await
            .unwrap()
            .len(),
        rows.len()
    );
}

/// Several work orders on one line: the not started ones give way first, the
/// latest planned start first; a started one keeps its quantity.
#[renox::test]
async fn the_not_started_work_orders_give_way_first_latest_start_first() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let _planner = t.planner("Amal", &plant).await;
    let running = linked(
        &t,
        &plant,
        &line,
        WorkOrderStatus::InProgress,
        1,
        500_000,
        3,
    )
    .await;
    let early = linked(&t, &plant, &line, WorkOrderStatus::Planned, 1, 300_000, 8).await;
    let late = linked(&t, &plant, &line, WorkOrderStatus::Planned, 1, 400_000, 20).await;
    // 1,200 in all; the line now needs 800.
    t.lines.set(ORDER, 1, 800_000);

    amended(&t, 2, 1, 1_200_000, 800_000, 0).await;

    assert_eq!(t.reload(late.id).await.status, WorkOrderStatus::Cancelled);
    assert_eq!(t.reload(early.id).await.quantity_milli, 300_000);
    let kept = t.reload(running.id).await;
    assert_eq!(kept.quantity_milli, 500_000);
    assert_eq!(kept.quantity_for_stock_milli, 0);
}

/// *Return to stock* concerns picked finished goods: no work order changes.
#[renox::test]
async fn return_to_stock_changes_no_work_order() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planned = linked(&t, &plant, &line, WorkOrderStatus::Planned, 1, 1_000_000, 5).await;
    t.lines.cancel(ORDER, 1);

    line_cancelled(&t, 1, 1_000_000, Some(LineOutcomeKind::ReturnToStock)).await;

    let now = t.reload(planned.id).await;
    assert_eq!(now.status, WorkOrderStatus::Planned);
    assert_eq!(now.quantity_milli, 1_000_000);
}

/// The whole order cancelled: every line's work orders follow, each with the
/// outcome of its line; a line without an outcome has its running work order
/// unlinked.
#[renox::test]
async fn a_cancelled_order_cancels_planned_work_orders_and_follows_the_outcome_of_each_line() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let _manager = t.manager("Mona", &plant).await;
    let planned = linked(&t, &plant, &line, WorkOrderStatus::Planned, 1, 1_000_000, 5).await;
    let running = linked(
        &t,
        &plant,
        &line,
        WorkOrderStatus::InProgress,
        2,
        1_000_000,
        5,
    )
    .await;
    let scrapped = linked(
        &t,
        &plant,
        &line,
        WorkOrderStatus::InProgress,
        3,
        1_000_000,
        5,
    )
    .await;
    with_issue(&t, &scrapped).await;
    for l in [1, 2, 3] {
        t.lines.cancel(ORDER, l);
    }

    t.state()
        .emit(SalesOrderCancelled {
            order: SalesOrderId(ORDER),
            party: PartyId(7),
            reason: "customer left".to_owned(),
            outcomes: vec![LineOutcome {
                line: 3,
                kind: LineOutcomeKind::Scrap,
                fee: 0,
                fee_waived: false,
            }],
        })
        .await
        .unwrap();

    assert_eq!(
        t.reload(planned.id).await.status,
        WorkOrderStatus::Cancelled
    );
    let kept = t.reload(running.id).await;
    assert_eq!(
        (kept.status, kept.source),
        (WorkOrderStatus::InProgress, WorkOrderSource::Stock)
    );
    assert_eq!(
        t.reload(scrapped.id).await.status,
        WorkOrderStatus::InProgress
    );
    assert_eq!(t.tickets.all().len(), 1, "one abort task, for line 3");
}

/// A prepaid increase makes work orders for the paid part; the held part gets
/// its work order once when it is released.
#[renox::test]
async fn the_held_part_of_a_prepaid_increase_gets_its_work_order_when_it_is_paid() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let _planner = t.planner("Amal", &plant).await;
    let running = linked(
        &t,
        &plant,
        &line,
        WorkOrderStatus::InProgress,
        1,
        1_000_000,
        5,
    )
    .await;
    t.lines.set(ORDER, 1, 1_300_000);

    amended(&t, 2, 1, 1_000_000, 1_300_000, 300_000).await;
    assert_eq!(
        linked_to_line(&t, 1).await.len(),
        1,
        "nothing is made before it is paid"
    );

    let released = SalesOrderLinesReleased {
        order: SalesOrderId(ORDER),
        party: PartyId(7),
        lines: vec![FulfilledLine {
            line: 1,
            quantity_milli: 300_000,
        }],
    };
    t.state().emit(released.clone()).await.unwrap();
    let all = linked_to_line(&t, 1).await;
    assert_eq!(all.len(), 2);
    assert_eq!(
        all.iter()
            .find(|wo| wo.id != running.id)
            .unwrap()
            .quantity_milli,
        300_000
    );

    t.state().emit(released).await.unwrap();
    assert_eq!(linked_to_line(&t, 1).await.len(), 2);
}

/// A work order is told about only for a line it is linked to: another line,
/// an unknown line and a stock work order are left alone.
#[renox::test]
async fn work_orders_of_other_lines_and_for_stock_are_left_alone() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let other = linked(&t, &plant, &line, WorkOrderStatus::Planned, 2, 1_000_000, 5).await;
    let stock = t.fixture(&plant, &line, WorkOrderStatus::Planned).await;
    t.lines.set(ORDER, 1, 0);
    t.lines.set(ORDER, 2, 1_000_000);

    line_cancelled(&t, 1, 1_000_000, None).await;
    line_cancelled(&t, 9, 1_000_000, None).await;

    assert_eq!(t.reload(other.id).await.status, WorkOrderStatus::Planned);
    assert_eq!(t.reload(stock.id).await.status, WorkOrderStatus::Planned);
}

/// `SAL.ACL.05` asks the vertical which lines started: a line with a started
/// work order (or one that made output) has, a planned or released one has
/// not, a cancelled one has not.
#[renox::test]
async fn a_line_has_started_when_its_work_order_has() {
    use wl_kernel::sales::FulfilmentProgress;
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    linked(&t, &plant, &line, WorkOrderStatus::Planned, 1, 1_000_000, 5).await;
    linked(
        &t,
        &plant,
        &line,
        WorkOrderStatus::Released,
        2,
        1_000_000,
        5,
    )
    .await;
    linked(
        &t,
        &plant,
        &line,
        WorkOrderStatus::InProgress,
        3,
        1_000_000,
        5,
    )
    .await;
    linked(&t, &plant, &line, WorkOrderStatus::OnHold, 4, 1_000_000, 5).await;
    linked(
        &t,
        &plant,
        &line,
        WorkOrderStatus::Completed,
        5,
        1_000_000,
        5,
    )
    .await;
    let issued = linked(
        &t,
        &plant,
        &line,
        WorkOrderStatus::Released,
        6,
        1_000_000,
        5,
    )
    .await;
    with_issue(&t, &issued).await;
    linked(
        &t,
        &plant,
        &line,
        WorkOrderStatus::Cancelled,
        7,
        1_000_000,
        5,
    )
    .await;

    let started = manufacturing::progress::ProductionProgress
        .started_lines(t.state(), SalesOrderId(ORDER), &[1, 2, 3, 4, 5, 6, 7, 8])
        .await
        .unwrap();

    assert_eq!(started, vec![3, 4, 5, 6]);
}
