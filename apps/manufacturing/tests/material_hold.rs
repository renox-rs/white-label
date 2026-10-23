//! BL-MFG-05 (#386): holding a work order for material when an issue is
//! pending (07 §4.2 last row of "When it goes wrong", §7.1 `hold` and
//! `resume`, `STK.RUL.06`): the line's supervisor gets one task to hold the
//! work order, then one to resume it, and the system never holds or resumes
//! by itself.

mod support;

use manufacturing::{MaterialIssue, WorkOrder, WorkOrderStatus};
use renox::prelude::*;
use support::{Booted, PIGMENT, boot};
use wl_kernel::Subject;
use wl_kernel::inventory::{PENDING_ISSUE_SUBJECT_TYPE, PendingIssueClosed};
use wl_kernel::tickets::Assignee;

/// WO-500 on L1 `in_progress`; Sami supervises the line, Karim keeps the
/// store; 0 kg of pigment P at the store.
struct Scene {
    t: Booted,
    keeper: User,
    sami: User,
    order: WorkOrder,
}

async fn scene() -> Scene {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let keeper = t.keeper("Karim", &plant).await;
    let sami = t.supervisor("Sami", &line).await;
    let order = t.released(&planner, &plant, &line).await;
    t.act(&sami, &order, "start", &[]).await.assert_status(303);
    let order = t.reload(order.id).await;
    Scene {
        t,
        keeper,
        sami,
        order,
    }
}

async fn issue_short(s: &Scene, kg: &str) {
    s.t.act(
        &s.keeper,
        &s.order,
        "issue",
        &[("item_id", &PIGMENT.to_string()), ("quantity", kg)],
    )
    .await
    .assert_status(303);
}

/// The pending issue of the first short row of the work order.
async fn pending_of(s: &Scene) -> i64 {
    MaterialIssue::of_order(s.t.db(), s.order.id)
        .await
        .unwrap()
        .iter()
        .find_map(|row| row.pending_issue_id)
        .expect("a pending issue")
}

/// What wl-inventory announces when the pending issue `id` of the work order
/// posted (`lot` and `cost` of what posted) and `open` more are still open.
async fn closed(s: &Scene, id: i64, how: &str, lot: Option<i64>, open: i64) {
    let row = MaterialIssue::of_order(s.t.db(), s.order.id)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.pending_issue_id == Some(id))
        .expect("the row");
    let pending = Subject::new(PENDING_ISSUE_SUBJECT_TYPE, id);
    s.t.state()
        .emit(PendingIssueClosed {
            pending_issue: pending.clone(),
            original: pending,
            source: s.order.subject(),
            item: row.item(),
            location: support::location(row.location_id),
            how: how.to_owned(),
            lot: lot.map(|lot| Subject::new(wl_kernel::inventory::LOT_SUBJECT_TYPE, lot)),
            quantity_milli: row.quantity_milli,
            unit_cost: 950,
            open_for_source: open,
        })
        .await
        .unwrap();
}

/// AC1 and AC2: a short issue of 20 kg P on an `in_progress` work order gives
/// supervisor B one task to hold it for material (a second short issue adds
/// none); B runs `hold`, the work order is `on_hold` for material, the task
/// closes and no breakdown ticket exists.
#[renox::test]
async fn a_short_issue_asks_the_supervisor_to_hold_and_hold_closes_the_task() {
    let s = scene().await;

    issue_short(&s, "20").await;
    issue_short(&s, "5").await;

    let tasks = s.t.tickets.all();
    assert_eq!(tasks.len(), 1, "one task per work order");
    assert_eq!(
        tasks[0].ticket.assignee,
        Some(Assignee::user(s.sami.id)),
        "the supervisor of the line"
    );
    assert!(tasks[0].ticket.title.contains(&s.order.number));
    assert!(tasks[0].ticket.description.contains("material"));
    assert_eq!(
        tasks[0].ticket.source_key.as_deref(),
        Some(manufacturing::material_hold::hold_key(s.order.id).as_str())
    );
    assert!(!tasks[0].done);
    assert_eq!(
        s.t.reload(s.order.id).await.status,
        WorkOrderStatus::InProgress,
        "the system never holds by itself"
    );

    s.t.act(&s.sami, &s.order, "hold", &[("reason", "material")])
        .await
        .assert_status(303);

    let held = s.t.reload(s.order.id).await;
    assert_eq!(held.status, WorkOrderStatus::OnHold);
    assert_eq!(held.hold_reason.as_deref(), Some("material"));
    let tasks = s.t.tickets.all();
    assert_eq!(tasks.len(), 1, "no breakdown ticket, no resume task yet");
    assert!(tasks[0].done, "the hold task closed");
}

/// AC3: 20 kg P arrive and the pending issue posts: B has a task to resume
/// the work order, the row takes the lot and the cost it posted with, and the
/// task closes when B resumes.
#[renox::test]
async fn when_the_material_arrives_the_supervisor_is_asked_to_resume() {
    let s = scene().await;
    issue_short(&s, "20").await;
    s.t.act(&s.sami, &s.order, "hold", &[("reason", "material")])
        .await
        .assert_status(303);
    let pending = pending_of(&s).await;

    closed(&s, pending, "posted", Some(7), 0).await;

    let tasks = s.t.tickets.all();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[1].ticket.assignee, Some(Assignee::user(s.sami.id)));
    assert_eq!(
        tasks[1].ticket.source_key.as_deref(),
        Some(manufacturing::material_hold::resume_key(s.order.id).as_str())
    );
    assert!(!tasks[1].done);
    assert_eq!(
        s.t.reload(s.order.id).await.status,
        WorkOrderStatus::OnHold,
        "the system never resumes by itself"
    );
    let rows = MaterialIssue::of_order(s.t.db(), s.order.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].lot_id, rows[0].unit_cost, rows[0].pending_issue_id),
        (Some(7), 950, None)
    );

    s.t.act(&s.sami, &s.order, "resume", &[])
        .await
        .assert_status(303);
    assert!(s.t.tickets.all()[1].done, "the resume task closed");
}

/// AC4: WO-501 `in_progress` with a pending issue of 10 kg P and the hold task
/// open: the storekeeper clears it ("used, not booked"); the hold task
/// closes and no resume task is made.
#[renox::test]
async fn clearing_the_last_pending_issue_closes_the_hold_task_without_a_resume() {
    let s = scene().await;
    issue_short(&s, "10").await;
    let pending = pending_of(&s).await;

    closed(&s, pending, "cleared", Some(7), 0).await;

    let tasks = s.t.tickets.all();
    assert_eq!(tasks.len(), 1, "no resume task");
    assert!(tasks[0].done, "the hold task closed");
    assert_eq!(
        s.t.reload(s.order.id).await.status,
        WorkOrderStatus::InProgress
    );
}

/// While another pending issue of the work order is open nothing closes or
/// opens; a work order held for another reason gets no resume task.
#[renox::test]
async fn several_pending_issues_and_other_hold_reasons_change_nothing() {
    let s = scene().await;
    issue_short(&s, "10").await;
    let pending = pending_of(&s).await;
    closed(&s, pending, "posted", Some(7), 1).await;
    assert!(!s.t.tickets.all()[0].done, "another pending issue is open");

    s.t.act(&s.sami, &s.order, "hold", &[("reason", "machine")])
        .await
        .assert_status(303);
    let before = s.t.tickets.all().len();
    // The machine hold closed the hold task (any reason does) and the last
    // pending issue posting later asks for no resume.
    assert!(s.t.tickets.all()[0].done);
    let second =
        s.t.act(
            &s.keeper,
            &s.order,
            "issue",
            &[("item_id", &PIGMENT.to_string()), ("quantity", "3")],
        )
        .await;
    second.assert_status(303);
    let pending = MaterialIssue::of_order(s.t.db(), s.order.id)
        .await
        .unwrap()
        .iter()
        .filter_map(|row| row.pending_issue_id)
        .next_back()
        .unwrap();
    closed(&s, pending, "posted", Some(7), 0).await;
    assert_eq!(s.t.tickets.all().len(), before, "no resume task");
}

/// A `released` work order is not held: a short issue gives no task, and the
/// pending issue shows on its page.
#[renox::test]
async fn a_released_work_order_gets_no_task_and_the_page_says_it_waits() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let keeper = t.keeper("Karim", &plant).await;
    let order = t.released(&planner, &plant, &line).await;

    t.act(
        &keeper,
        &order,
        "issue",
        &[("item_id", &PIGMENT.to_string()), ("quantity", "20")],
    )
    .await
    .assert_status(303);

    assert!(t.tickets.all().is_empty());
    let page = t
        .browse(
            &planner,
            &format!("/manufacturing/work-orders/{}", order.id),
        )
        .await
        .text();
    assert!(page.contains("Waiting for material"), "{page}");
}

/// A pending issue cleared because the work order was cancelled never became
/// an issue: its row goes.
#[renox::test]
async fn a_pending_issue_of_a_cancelled_source_leaves_no_row() {
    let s = scene().await;
    issue_short(&s, "10").await;
    let pending = pending_of(&s).await;

    closed(&s, pending, "source_cancelled", None, 0).await;

    assert!(
        MaterialIssue::of_order(s.t.db(), s.order.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// A pending issue that arriving stock covered in part: the covered part is a
/// row of its own with its lot and cost, the rest keeps waiting, and nothing
/// is asked of the supervisor while it does.
#[renox::test]
async fn a_partly_covered_pending_issue_splits_its_row() {
    let s = scene().await;
    issue_short(&s, "20").await;
    let pending = pending_of(&s).await;
    let row = MaterialIssue::of_order(s.t.db(), s.order.id)
        .await
        .unwrap()
        .remove(0);

    s.t.state()
        .emit(PendingIssueClosed {
            pending_issue: Subject::new(PENDING_ISSUE_SUBJECT_TYPE, 99),
            original: Subject::new(PENDING_ISSUE_SUBJECT_TYPE, pending),
            source: s.order.subject(),
            item: row.item(),
            location: support::location(row.location_id),
            how: "posted".to_owned(),
            lot: Some(Subject::new(wl_kernel::inventory::LOT_SUBJECT_TYPE, 7)),
            quantity_milli: 8_000,
            unit_cost: 950,
            open_for_source: 1,
        })
        .await
        .unwrap();

    let rows = MaterialIssue::of_order(s.t.db(), s.order.id).await.unwrap();
    let seen: Vec<(Option<i64>, i64, bool)> = rows
        .iter()
        .map(|r| (r.lot_id, r.quantity_milli, r.pending_issue_id.is_some()))
        .collect();
    assert_eq!(seen, vec![(None, 12_000, true), (Some(7), 8_000, false)]);
    assert_eq!(rows[1].unit_cost, 950);
    assert!(!s.t.tickets.all()[0].done, "a pending issue is still open");
}
