//! Base: 07 Work order lifecycle (`MFG.ACL.01`, `MFG.ACL.10`, 07 §7.1 *Work
//! order*): create, release, start, hold, resume, complete, close and cancel,
//! with who may run each and what each refuses. `abort` is in `abort.rs`
//! (BL-MFG-01), `unlink` in `unlink.rs` (BL-MFG-02).

mod support;

use manufacturing::pick_list;
use manufacturing::{MaterialIssue, ProductionReport, WorkOrderSource, WorkOrderStatus};
use renox::prelude::*;

use support::{BOM, PANEL, PIGMENT, RESIN, boot, date};

/// A planner of the plant creates a work order: it is `planned`, numbered,
/// for stock, and on the line chosen. Someone without `work_orders.create`
/// is refused, and so is a quantity of zero.
#[renox::test]
async fn a_planner_creates_a_planned_work_order() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let keeper = t.keeper("Karim", &plant).await;

    let order = t.order(&planner, &plant, &line).await;

    assert_eq!(order.status, WorkOrderStatus::Planned);
    assert!(order.number.starts_with("WO-"), "{}", order.number);
    assert_eq!(order.source, WorkOrderSource::Stock);
    assert_eq!(order.line_id, Some(line.id));
    assert_eq!(order.quantity_milli, 1_000_000);
    assert_eq!(order.created_by, Some(planner.id));

    let form = [
        ("plant_id", plant.id.to_string()),
        ("item_id", PANEL.to_string()),
        ("quantity", "5".to_owned()),
    ];
    let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    t.post(&keeper, "/manufacturing/work-orders", &form)
        .await
        .assert_forbidden();
    let zero = [
        ("plant_id", plant.id.to_string()),
        ("item_id", PANEL.to_string()),
        ("quantity", "0".to_owned()),
    ];
    let zero: Vec<(&str, &str)> = zero.iter().map(|(k, v)| (*k, v.as_str())).collect();
    t.post(&planner, "/manufacturing/work-orders", &zero)
        .await
        .assert_status(422);
}

/// `release` puts the work order on its line, makes the pick list from the
/// BOM (2 kg of resin with 5% scrap and 0.1 kg of pigment a unit) and keeps
/// the standard cost of a unit (AED 18 of material and 0.05 hours at AED 120).
#[renox::test]
async fn release_makes_the_pick_list_and_keeps_the_standard_cost() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;

    let order = t.released(&planner, &plant, &line).await;

    assert_eq!(order.status, WorkOrderStatus::Released);
    assert_eq!(order.std_material_cost, 1_800);
    assert_eq!(order.std_unit_cost, 2_400);
    let list = pick_list::of_order(t.db(), order.id).await.unwrap();
    let wanted: Vec<(i64, i64)> = list.iter().map(|l| (l.item_id, l.required_milli)).collect();
    assert_eq!(wanted, vec![(RESIN, 2_100_000), (PIGMENT, 100_000)]);
}

/// `release` lists what is missing: the line, and a BOM version that is
/// released; a holder without `work_orders.release` is refused.
#[renox::test]
async fn release_is_refused_without_a_line_or_a_released_bom() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let keeper = t.keeper("Karim", &plant).await;
    let order = t.order(&planner, &plant, &line).await;

    // No line.
    manufacturing::WorkOrder::query()
        .where_(manufacturing::WorkOrder::ID.eq(order.id))
        .update(t.db(), &[("line_id", &Option::<i64>::None)])
        .await
        .unwrap();
    let refused = t.act(&planner, &order, "release", &[]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("line"), "{}", refused.text());

    // A BOM version that is a draft.
    manufacturing::WorkOrder::query()
        .where_(manufacturing::WorkOrder::ID.eq(order.id))
        .update(
            t.db(),
            &[("line_id", &Some(line.id)), ("bom_id", &Some(BOM + 1))],
        )
        .await
        .unwrap();
    let refused = t.act(&planner, &order, "release", &[]).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("not released"),
        "{}",
        refused.text()
    );
    assert_eq!(t.reload(order.id).await.status, WorkOrderStatus::Planned);

    t.act(&keeper, &order, "release", &[])
        .await
        .assert_forbidden();
}

/// `MFG.ACL.01`: the supervisor of the line starts, holds and resumes its work
/// orders; the supervisor of another line is refused. `start` records the
/// time and needs a released work order.
#[renox::test]
async fn only_the_line_starts_holds_and_resumes_its_work_orders() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let l1 = t.line(&plant, "L1").await;
    let l2 = t.line(&plant, "L2").await;
    let planner = t.planner("Amal", &plant).await;
    let on_l1 = t.supervisor("Sami", &l1).await;
    let on_l2 = t.supervisor("Sara", &l2).await;
    let order = t.order(&planner, &plant, &l1).await;

    t.act(&on_l1, &order, "start", &[]).await.assert_status(422);
    t.act(&planner, &order, "release", &[])
        .await
        .assert_status(303);
    t.act(&on_l2, &order, "start", &[]).await.assert_forbidden();
    t.act(&on_l1, &order, "start", &[]).await.assert_status(303);
    let started = t.reload(order.id).await;
    assert_eq!(started.status, WorkOrderStatus::InProgress);
    assert!(started.started_at.is_some());

    t.act(&on_l1, &order, "hold", &[]).await.assert_status(422);
    t.act(&on_l1, &order, "hold", &[("reason", "tea")])
        .await
        .assert_status(422);
    t.act(&on_l2, &order, "hold", &[("reason", "machine")])
        .await
        .assert_forbidden();
    t.act(&on_l1, &order, "hold", &[("reason", "machine")])
        .await
        .assert_status(303);
    let held = t.reload(order.id).await;
    assert_eq!(held.status, WorkOrderStatus::OnHold);
    assert_eq!(held.hold_reason.as_deref(), Some("machine"));

    t.act(&on_l2, &order, "resume", &[])
        .await
        .assert_forbidden();
    t.act(&on_l1, &order, "resume", &[])
        .await
        .assert_status(303);
    let resumed = t.reload(order.id).await;
    assert_eq!(resumed.status, WorkOrderStatus::InProgress);
    assert_eq!(resumed.hold_reason, None);
}

/// A line reports output while the work order is `in_progress`: the good
/// quantity is posted as a `produce` into the output lot, the totals move,
/// and the line of another supervisor cannot report. `complete` needs good
/// output, asks QC for the final inspection and emits `WorkOrderCompleted`.
#[renox::test]
async fn output_is_reported_and_the_work_order_completes() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let other = t.line(&plant, "L2").await;
    let planner = t.planner("Amal", &plant).await;
    let sami = t.supervisor("Sami", &line).await;
    let sara = t.supervisor("Sara", &other).await;
    let order = t.released(&planner, &plant, &line).await;

    t.act(&sami, &order, "report", &[("good", "10")])
        .await
        .assert_status(422);
    t.act(&sami, &order, "start", &[]).await.assert_status(303);
    t.act(&sara, &order, "report", &[("good", "10")])
        .await
        .assert_forbidden();
    t.act(&sami, &order, "report", &[("good", "0"), ("scrap", "0")])
        .await
        .assert_status(422);
    t.act(&sami, &order, "complete", &[])
        .await
        .assert_status(422);

    t.act(
        &sami,
        &order,
        "report",
        &[("good", "600"), ("scrap", "20.5")],
    )
    .await
    .assert_status(303);
    t.act(&sami, &order, "report", &[("good", "400")])
        .await
        .assert_status(303);

    let reported = t.reload(order.id).await;
    assert_eq!(reported.good_milli, 1_000_000);
    assert_eq!(reported.scrap_milli, 20_500);
    assert_eq!(reported.output_lot_id, Some(900 + order.id));
    let reports = ProductionReport::query().get(t.db()).await.unwrap();
    assert_eq!(reports.len(), 2);
    let produced = t.stock.produced.lock().unwrap().clone();
    assert_eq!(produced.len(), 2);
    assert_eq!(produced[0].quantity_milli, 600_000);
    assert_eq!(produced[0].unit_cost, 2_400);
    assert_eq!(produced[0].lot, None);
    assert!(
        produced[1].lot.is_some(),
        "the second report goes into the same lot"
    );

    t.act(&sami, &order, "complete", &[])
        .await
        .assert_status(303);
    let done = t.reload(order.id).await;
    assert_eq!(done.status, WorkOrderStatus::Completed);
    assert!(done.stopped_at.is_some());
    assert_eq!(t.completed.all().len(), 1);
    assert_eq!(t.completed.all()[0].good_milli, 1_000_000);
    let tasks = t.tickets.all();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].ticket.title.contains("Final inspection"));
    // `MFG.ACL.10`: a completed work order takes no more output.
    t.act(&sami, &order, "report", &[("good", "1")])
        .await
        .assert_status(422);
}

/// `close` calculates the actual cost (materials issued and the hours of the
/// line); the production manager closes, the supervisor may not, and only a
/// `completed` work order closes.
#[renox::test]
async fn the_production_manager_closes_a_completed_work_order() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let manager = t.manager("Mona", &plant).await;
    let keeper = t.keeper("Karim", &plant).await;
    let sami = t.supervisor("Sami", &line).await;
    t.stock.put(RESIN, Some(1), 500_000, 800);
    let order = t.released(&planner, &plant, &line).await;
    t.act(
        &keeper,
        &order,
        "issue",
        &[("item_id", &RESIN.to_string()), ("quantity", "500")],
    )
    .await
    .assert_status(303);
    t.at(date(2027, 3, 2), 8, 0, async {
        t.act(&sami, &order, "start", &[]).await.assert_status(303);
    })
    .await;
    t.act(&sami, &order, "report", &[("good", "50")])
        .await
        .assert_status(303);
    t.act(&manager, &order, "close", &[])
        .await
        .assert_status(422);
    t.at(date(2027, 3, 2), 13, 0, async {
        t.act(&sami, &order, "complete", &[])
            .await
            .assert_status(303);
    })
    .await;

    t.act(&sami, &order, "close", &[]).await.assert_forbidden();
    t.act(&manager, &order, "close", &[])
        .await
        .assert_status(303);

    let closed = t.reload(order.id).await;
    assert_eq!(closed.status, WorkOrderStatus::Closed);
    // AED 4,000 of resin and 5 hours at AED 120.
    assert_eq!(closed.actual_cost, Some(400_000 + 60_000));
    t.act(&manager, &order, "close", &[])
        .await
        .assert_status(422);
}

/// `cancel` takes a `planned` work order out, withdraws its pick list and
/// needs a reason from `in_progress`.
#[renox::test]
async fn cancel_withdraws_the_pick_list_and_needs_a_reason_once_started() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let keeper = t.keeper("Karim", &plant).await;
    let sami = t.supervisor("Sami", &line).await;

    let released = t.released(&planner, &plant, &line).await;
    t.act(&keeper, &released, "cancel", &[("reason", "x")])
        .await
        .assert_forbidden();
    t.act(&planner, &released, "cancel", &[])
        .await
        .assert_status(303);
    let gone = t.reload(released.id).await;
    assert_eq!(gone.status, WorkOrderStatus::Cancelled);
    let list = pick_list::of_order(t.db(), gone.id).await.unwrap();
    assert!(!list.is_empty() && list.iter().all(|l| l.withdrawn));
    t.act(&planner, &gone, "cancel", &[("reason", "again")])
        .await
        .assert_status(422);

    let running = t.released(&planner, &plant, &line).await;
    t.act(&sami, &running, "start", &[])
        .await
        .assert_status(303);
    t.act(&planner, &running, "cancel", &[])
        .await
        .assert_status(422);
    t.act(
        &planner,
        &running,
        "cancel",
        &[("reason", "order cancelled")],
    )
    .await
    .assert_status(303);
    let stopped = t.reload(running.id).await;
    assert_eq!(stopped.status, WorkOrderStatus::Cancelled);
    assert_eq!(stopped.cancel_reason.as_deref(), Some("order cancelled"));
    assert_eq!(t.cancelled.all().len(), 2);
}

/// Once a material is issued `cancel` is refused (use `abort`, `MFG.ACL.12`).
#[renox::test]
async fn cancel_is_refused_once_material_is_issued() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let keeper = t.keeper("Karim", &plant).await;
    t.stock.put(RESIN, Some(1), 100_000, 800);
    let order = t.released(&planner, &plant, &line).await;
    t.act(
        &keeper,
        &order,
        "issue",
        &[("item_id", &RESIN.to_string()), ("quantity", "100")],
    )
    .await
    .assert_status(303);

    let refused = t
        .act(&planner, &order, "cancel", &[("reason", "oops")])
        .await;

    refused.assert_status(422);
    assert!(refused.text().contains("abort"), "{}", refused.text());
    assert_eq!(t.reload(order.id).await.status, WorkOrderStatus::Released);
}

/// The storekeeper issues a material from the store of the line at the cost
/// of each lot; the short part waits as a pending issue and is not counted.
/// Issues need `work_orders.issue` and a released, in-progress or held work
/// order.
#[renox::test]
async fn the_storekeeper_issues_material_at_the_cost_of_each_lot() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let keeper = t.keeper("Karim", &plant).await;
    let sami = t.supervisor("Sami", &line).await;
    t.stock.put(RESIN, Some(1), 300_000, 800);
    t.stock.put(RESIN, Some(2), 100_000, 900);
    let item = RESIN.to_string();
    let planned = t.order(&planner, &plant, &line).await;
    t.act(
        &keeper,
        &planned,
        "issue",
        &[("item_id", &item), ("quantity", "10")],
    )
    .await
    .assert_status(422);
    let order = t.released(&planner, &plant, &line).await;
    t.act(
        &sami,
        &order,
        "issue",
        &[("item_id", &item), ("quantity", "10")],
    )
    .await
    .assert_forbidden();
    t.act(
        &keeper,
        &order,
        "issue",
        &[("item_id", &item), ("quantity", "0")],
    )
    .await
    .assert_status(422);

    t.act(
        &keeper,
        &order,
        "issue",
        &[("item_id", &item), ("quantity", "450")],
    )
    .await
    .assert_status(303);

    let rows = MaterialIssue::of_order(t.db(), order.id).await.unwrap();
    let posted: Vec<(Option<i64>, i64, i64, bool)> = rows
        .iter()
        .map(|r| {
            (
                r.lot_id,
                r.quantity_milli,
                r.unit_cost,
                r.pending_issue_id.is_some(),
            )
        })
        .collect();
    assert_eq!(
        posted,
        vec![
            (Some(1), 300_000, 800, false),
            (Some(2), 100_000, 900, false),
            (None, 50_000, 0, true),
        ]
    );
    assert!(t.reload(order.id).await.has_issue);
    assert_eq!(manufacturing::cost::issued_value(&rows), 240_000 + 90_000);
}
