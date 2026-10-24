//! BL-MFG-01 (#382): aborting a work order after materials were issued
//! (`MFG.ACL.12`, `STK.ACL.02`, 07 §4.2 row 1, §7.1 *Work order* `abort`).
//! The planner asks with an outcome, the production manager approves (never
//! the requester), a second step under `STK.ACL.02` waits above AED 2,000 of
//! scrap, and only at the final approval are the movements posted, the cost
//! closed and `WorkOrderAborted` emitted.

mod support;

use manufacturing::{AbortLine, MaterialIssue, WorkOrder, WorkOrderStatus};
use renox::prelude::*;
use support::{Booted, RESIN, STORE, boot, date};
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus, approve, reject};

/// WO-100 on L1 (AED 120 an hour): released, 500 kg of resin lot R1 issued at
/// AED 8 a kg (AED 4,000), started on 2027-03-02 at 08:00 and 50 good panels
/// reported; BOM standard material cost AED 18 and standard cost AED 24 a
/// panel.
struct Scene {
    t: Booted,
    planner: User,
    manager: User,
    keeper: User,
    sami: User,
    order: WorkOrder,
}

async fn scene() -> Scene {
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
    let order = t.reload(order.id).await;
    Scene {
        t,
        planner,
        manager,
        keeper,
        sami,
        order,
    }
}

/// The form of the abort: reason "wrong BOM" and `kg` of R1 back to the store.
fn form(kg: &str) -> Vec<(String, String)> {
    let mut form = vec![("reason".to_owned(), "wrong BOM".to_owned())];
    if !kg.is_empty() {
        form.push((format!("return_{RESIN}_1"), kg.to_owned()));
    }
    form
}

async fn ask(s: &Scene, by: &User, kg: &str) -> renox::testing::TestResponse {
    let form = form(kg);
    let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    s.t.act(by, &s.order, "abort", &form).await
}

async fn requests(s: &Scene) -> Vec<ApprovalRequest> {
    ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_ID.eq(s.order.id))
        .order_by(ApprovalRequest::ID)
        .get(s.t.db())
        .await
        .unwrap()
}

/// AC1: planner A asks to abort WO-100 ("wrong BOM"; return 200 kg R1; scrap
/// the rest; 50 units to quarantine): an approval request exists with a scrap
/// value of AED 1,500 (4,000 - 1,600 - 50 x 18), WO-100 stays `in_progress`
/// and nothing is posted. AC2: A tries to approve and is refused.
#[renox::test]
async fn the_planner_asks_and_cannot_approve_it_herself() {
    let s = scene().await;

    ask(&s, &s.planner, "200").await.assert_status(303);

    let waiting = requests(&s).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].action, "manufacturing.abort");
    assert_eq!(waiting[0].requester_id, s.planner.id);
    assert_eq!(waiting[0].status, ApprovalStatus::Pending);
    assert_eq!(waiting[0].payload["scrap_value"], 150_000);
    assert_eq!(
        waiting[0].steps.0.len(),
        1,
        "AED 1,500 needs no second step"
    );
    let still = s.t.reload(s.order.id).await;
    assert_eq!(still.status, WorkOrderStatus::InProgress);
    assert_eq!(
        s.t.stock.movements(),
        0,
        "nothing posts before the approval"
    );
    assert!(s.t.aborted.all().is_empty());

    let refused = approve(s.t.state(), waiting[0].id, &s.planner, "mine").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(
        s.t.reload(s.order.id).await.status,
        WorkOrderStatus::InProgress
    );
}

/// AC3: production manager B approves at 13:00 on 2027-03-02: 200 kg R1
/// (AED 1,600) are returned to the store, a `wastage` of AED 1,500 is posted,
/// the output lot with 50 units stays in quarantine with a final inspection
/// requested, WO-100 is `aborted`, the loss stored is AED 1,800 (2,400 +
/// 5 h x 120 - 50 x 24), sales is told and `WorkOrderAborted` is emitted.
#[renox::test]
async fn the_production_manager_approves_and_the_movements_post_once() {
    let s = scene().await;
    let sales = s.t.user("Salma").await;
    // The order serves line 1 of sales order 50, owned by Salma.
    WorkOrder::query()
        .where_(WorkOrder::ID.eq(s.order.id))
        .update(
            s.t.db(),
            &[
                ("source", &manufacturing::WorkOrderSource::SalesOrder),
                ("sales_order_id", &Some(50_i64)),
                ("sales_order_line_id", &Some(1_i64)),
            ],
        )
        .await
        .unwrap();
    s.t.lines.open(50, 1, 1_000_000, Some(sales.id));
    ask(&s, &s.planner, "200").await.assert_status(303);
    let request = requests(&s).await[0].id;

    s.t.at(date(2027, 3, 2), 13, 0, async {
        approve(s.t.state(), request, &s.manager, "agreed")
            .await
            .unwrap();
    })
    .await;

    let done = s.t.reload(s.order.id).await;
    assert_eq!(done.status, WorkOrderStatus::Aborted);
    assert_eq!(done.abort_reason.as_deref(), Some("wrong BOM"));
    assert_eq!(done.abort_requested_by, Some(s.planner.id));
    assert_eq!(done.abort_approved_by, Some(s.manager.id));
    assert_eq!(done.abort_scrap_value, Some(150_000));
    assert_eq!(done.abort_loss, Some(180_000));
    assert_eq!(done.actual_cost, Some(240_000 + 60_000));
    assert!(done.aborted_at.is_some() && done.stopped_at == done.aborted_at);
    let returned = s.t.stock.returned.lock().unwrap().clone();
    assert_eq!(returned.len(), 1);
    assert_eq!(
        (
            returned[0].quantity_milli,
            returned[0].unit_cost,
            returned[0].location.subject_id
        ),
        (200_000, 800, STORE)
    );
    assert_eq!(returned[0].lot.as_ref().map(|l| l.subject_id), Some(1));
    let wasted = s.t.stock.wasted.lock().unwrap().clone();
    assert_eq!(wasted.len(), 1);
    assert_eq!(
        (wasted[0].value, wasted[0].quantity_milli),
        (150_000, 187_500)
    );
    assert_eq!(wasted[0].reason, "wrong BOM");
    assert_eq!(wasted[0].source, done.subject());
    // The good output stays in the one output lot; no second lot is made.
    assert_eq!(s.t.stock.produced.lock().unwrap().len(), 1);
    assert_eq!(done.output_lot_id, Some(900 + done.id));
    let lines = AbortLine::query().get(s.t.db()).await.unwrap();
    let kinds: Vec<(&str, i64)> = lines.iter().map(|l| (l.kind.as_str(), l.value)).collect();
    assert_eq!(
        kinds,
        vec![("return", 160_000), ("scrap", 150_000), ("output", 120_000)]
    );
    let issues = MaterialIssue::of_order(s.t.db(), done.id).await.unwrap();
    assert_eq!(issues[0].returned_milli, 200_000);
    // The final inspection of the output lot is asked of QC.
    let tasks = s.t.tickets.all();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].ticket.subject.subject_id, 900 + done.id);
    assert!(tasks[0].ticket.title.contains("Final inspection"));
    // Sales is told, and the event carries the outcome.
    let told = s.t.notices.titles_for(&sales);
    assert!(
        told.iter().any(|title| title.contains("was aborted")),
        "{told:?}"
    );
    let events = s.t.aborted.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].returned, 160_000);
    assert_eq!(events[0].scrapped, 150_000);
    assert_eq!(events[0].loss, 180_000);
    assert_eq!(events[0].approver, Some(s.manager.id));
    assert_eq!(
        events[0].output_lot.as_ref().map(|l| l.subject_id),
        Some(900 + done.id)
    );
    // A replay of the announcement posts nothing again.
    assert!(
        !manufacturing::abort::after_abort(s.t.state(), done.id)
            .await
            .unwrap()
    );
    assert_eq!(s.t.stock.movements(), 2);
}

/// AC4: with no material returned the scrap value is AED 3,100 (4,000 -
/// 50 x 18): the request has a second step under `STK.ACL.02`, and nothing
/// posts until it is approved.
#[renox::test]
async fn a_scrap_above_the_limit_needs_a_second_step_before_anything_posts() {
    let s = scene().await;
    let stock_approver = s.t.stock_approver("Sana").await;
    ask(&s, &s.planner, "").await.assert_status(303);
    let request = requests(&s).await[0].clone();
    assert_eq!(request.payload["scrap_value"], 310_000);
    assert_eq!(request.steps.0.len(), 2);

    approve(s.t.state(), request.id, &s.manager, "step 1")
        .await
        .unwrap();

    assert_eq!(
        s.t.reload(s.order.id).await.status,
        WorkOrderStatus::InProgress
    );
    assert_eq!(s.t.stock.movements(), 0);
    assert!(s.t.aborted.all().is_empty());

    approve(s.t.state(), request.id, &stock_approver, "step 2")
        .await
        .unwrap();

    let done = s.t.reload(s.order.id).await;
    assert_eq!(done.status, WorkOrderStatus::Aborted);
    assert_eq!(done.abort_scrap_value, Some(310_000));
    assert_eq!(s.t.stock.wasted.lock().unwrap()[0].value, 310_000);
    assert!(s.t.stock.returned.lock().unwrap().is_empty());
    assert_eq!(done.abort_approved_by, Some(stock_approver.id));
}

/// AC5: when B rejects, WO-100 stays `in_progress` and no movement exists.
#[renox::test]
async fn a_rejected_abort_leaves_the_work_order_as_it_was() {
    let s = scene().await;
    ask(&s, &s.planner, "200").await.assert_status(303);
    let request = requests(&s).await[0].id;

    reject(s.t.state(), request, &s.manager, "keep it running")
        .await
        .unwrap();

    assert_eq!(requests(&s).await[0].status, ApprovalStatus::Rejected);
    assert_eq!(
        s.t.reload(s.order.id).await.status,
        WorkOrderStatus::InProgress
    );
    assert_eq!(s.t.stock.movements(), 0);
    assert!(s.t.aborted.all().is_empty());
    // The work order takes reports and issues again.
    s.t.act(&s.sami, &s.order, "report", &[("good", "1")])
        .await
        .assert_status(303);
}

/// AC6: while the request is pending, a production report and an issue on
/// WO-100 are refused as *abort pending*; a second request is refused.
#[renox::test]
async fn while_the_request_waits_issues_and_reports_are_refused() {
    let s = scene().await;
    ask(&s, &s.planner, "200").await.assert_status(303);

    let report = s.t.act(&s.sami, &s.order, "report", &[("good", "5")]).await;
    report.assert_status(422);
    assert!(report.text().contains("Abort pending"), "{}", report.text());
    let issue =
        s.t.act(
            &s.keeper,
            &s.order,
            "issue",
            &[("item_id", &RESIN.to_string()), ("quantity", "1")],
        )
        .await;
    issue.assert_status(422);
    assert!(issue.text().contains("Abort pending"), "{}", issue.text());
    let twice = ask(&s, &s.planner, "100").await;
    twice.assert_status(422);
    assert!(twice.text().contains("already waits"), "{}", twice.text());
}

/// A late entry between the request and the final approval fails the
/// request: the posted quantities differ, nothing is posted and a new request
/// is made.
#[renox::test]
async fn a_late_entry_fails_the_request() {
    let s = scene().await;
    ask(&s, &s.planner, "200").await.assert_status(303);
    let request = requests(&s).await[0].id;
    WorkOrder::query()
        .where_(WorkOrder::ID.eq(s.order.id))
        .update(s.t.db(), &[("good_milli", &55_000_i64)])
        .await
        .unwrap();

    approve(s.t.state(), request, &s.manager, "ok")
        .await
        .unwrap();

    let failed = requests(&s).await[0].clone();
    assert_eq!(failed.status, ApprovalStatus::Failed);
    assert_eq!(
        s.t.reload(s.order.id).await.status,
        WorkOrderStatus::InProgress
    );
    assert_eq!(s.t.stock.movements(), 0);
    ask(&s, &s.planner, "200").await.assert_status(303);
}

/// A material line cannot return more than issued less already returned; a
/// reason is needed; only a holder of `work_orders.release` asks.
#[renox::test]
async fn the_request_is_checked_when_it_is_asked() {
    let s = scene().await;

    let too_much = ask(&s, &s.planner, "500.001").await;
    too_much.assert_status(422);
    assert!(
        too_much.text().contains("not more than was issued"),
        "{}",
        too_much.text()
    );
    s.t.act(&s.planner, &s.order, "abort", &[("reason", "")])
        .await
        .assert_status(422);
    ask(&s, &s.keeper, "100").await.assert_forbidden();
    assert!(requests(&s).await.is_empty());
}

/// AC7: a `released` work order with no material issued is not offered
/// `abort` (it is refused and `cancel` is the way); AC8: an `in_progress` one
/// with nothing issued or reported is cancelled with "order cancelled"; AC9:
/// a `completed` one is not offered `abort`.
#[renox::test]
async fn abort_is_not_offered_where_cancel_or_nothing_applies() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let sami = t.supervisor("Sami", &line).await;

    let released = t.released(&planner, &plant, &line).await;
    let refused = t
        .act(&planner, &released, "abort", &[("reason", "x")])
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("cancel"), "{}", refused.text());
    t.act(&planner, &released, "cancel", &[])
        .await
        .assert_status(303);

    let running = t.released(&planner, &plant, &line).await;
    t.act(&sami, &running, "start", &[])
        .await
        .assert_status(303);
    t.act(&planner, &running, "abort", &[("reason", "x")])
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
    assert_eq!(
        t.reload(running.id).await.status,
        WorkOrderStatus::Cancelled
    );

    let completed = t.fixture(&plant, &line, WorkOrderStatus::Completed).await;
    t.act(&planner, &completed, "abort", &[("reason", "x")])
        .await
        .assert_status(422);
    assert_eq!(
        t.reload(completed.id).await.status,
        WorkOrderStatus::Completed
    );
}

/// The scrap is spread over the issued items in proportion to what is left,
/// and the values add up to the scrap value of the request.
#[renox::test]
async fn the_scrap_of_two_materials_adds_up() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let manager = t.manager("Mona", &plant).await;
    let keeper = t.keeper("Karim", &plant).await;
    t.stock.put(RESIN, Some(1), 500_000, 800);
    t.stock.put(support::PIGMENT, None, 100_000, 300);
    let order = t.released(&planner, &plant, &line).await;
    for (item, quantity) in [(RESIN, "500"), (support::PIGMENT, "100")] {
        t.act(
            &keeper,
            &order,
            "issue",
            &[("item_id", &item.to_string()), ("quantity", quantity)],
        )
        .await
        .assert_status(303);
    }
    t.act(&planner, &order, "abort", &[("reason", "material defect")])
        .await
        .assert_status(303);
    let request = ApprovalRequest::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    // AED 4,000 + AED 300 of material and no good output.
    assert_eq!(request.payload["scrap_value"], 430_000);

    let stock_approver = t.stock_approver("Sana").await;
    approve(t.state(), request.id, &manager, "ok")
        .await
        .unwrap();
    approve(t.state(), request.id, &stock_approver, "ok")
        .await
        .unwrap();

    let wasted = t.stock.wasted.lock().unwrap().clone();
    assert_eq!(wasted.len(), 2);
    assert_eq!(wasted.iter().map(|w| w.value).sum::<i64>(), 430_000);
    let done = t.reload(order.id).await;
    assert_eq!(done.status, WorkOrderStatus::Aborted);
    // Never started: no hours; the loss is the whole material.
    assert_eq!(done.abort_loss, Some(430_000));
}
