//! BL-LOG-12 (#310), 03 §3.3 `LOG.ACL.14`, §4.1 row *Shipment cannot be
//! found*, §7.1 row `mark_lost`, §7.2 `LOG.RUL.12`: a shipment that is not on
//! a delivery gets a trace task after 3 days without a scan and is escalated
//! after 7; marking it lost needs an operations manager's approval by someone
//! other than the requester. The delivery module is the real wl-delivery and
//! approvals run through the kernel.

mod support;

use logistics::notices::CLIENT_NOTICE;
use logistics::trace::{self, TraceRun, TraceTask};
use logistics::{Shipment, ShipmentStatus, TrackingEvent};
use renox::auth::permissions;
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::approvals::{self, ApprovalEnd, ApprovalRequest, ApprovalStatus};
use wl_kernel::tickets::Assignee;

use logistics::permissions as perm;
use support::{SOUQ, World, boot};

/// An instant, from RFC 3339 text in UTC.
fn at(text: &str) -> DateTime {
    text.parse().expect("an instant")
}

/// 2026-11-01 10:00 Dubai.
const LAST_SCAN: &str = "2026-11-01T06:00:00Z";
/// 2026-11-04 02:00 Dubai: 64 hours after the last scan.
const NOV_4: &str = "2026-11-03T22:00:00Z";
/// 2026-11-05 02:00 Dubai: 88 hours after it.
const NOV_5: &str = "2026-11-04T22:00:00Z";
/// 2026-11-09 02:00 Dubai: 184 hours after it.
const NOV_9: &str = "2026-11-08T22:00:00Z";

/// The operations manager: holds the role and approves a loss.
async fn manager(w: &World, name: &str) -> User {
    permissions::define_role(
        w.db(),
        perm::OPERATIONS_MANAGER_ROLE,
        &[perm::SHIPMENTS_VIEW],
    )
    .await
    .unwrap();
    let user = w
        .user_with(
            name,
            &[
                perm::SHIPMENTS_VIEW,
                perm::SHIPMENTS_MANAGE,
                perm::SHIPMENTS_APPROVE_LOSS,
            ],
        )
        .await;
    user.assign_role(w.db(), perm::OPERATIONS_MANAGER_ROLE)
        .await
        .unwrap();
    user
}

/// Moves every tracking event of `shipment` to `when`, as if nothing had
/// happened since.
async fn touched_at(w: &World, shipment: &Shipment, when: &str) {
    let when = at(when);
    TrackingEvent::query()
        .where_(TrackingEvent::SHIPMENT_ID.eq(shipment.id))
        .update(w.db(), &[("occurred_at", &when)])
        .await
        .unwrap();
}

/// The clock of the app reaches `when` (the scans of these tests lie in
/// November; a request made earlier would be older than the scan and the
/// final re-check of the approval would see it as activity since).
fn clock_reaches(w: &World, when: &str) {
    let secs = (at(when) - renox::db::now()).num_seconds().max(0);
    w.app
        .travel(std::time::Duration::from_secs(secs.unsigned_abs() + 3_600));
}

/// A shipment `at_hub` whose last scan was `when`.
async fn at_hub(w: &World, when: &str) -> Shipment {
    let booked = w.book_as(&w.dispatcher, SOUQ, "ORD-1").await;
    let hub = w.at_hub(&w.pick_up(&booked).await).await;
    touched_at(w, &hub, when).await;
    hub
}

async fn tasks(w: &World) -> Vec<TraceTask> {
    TraceTask::query()
        .order_by(TraceTask::ID)
        .get(w.db())
        .await
        .unwrap()
}

fn trace_tickets(w: &World) -> usize {
    w.tickets
        .opened()
        .iter()
        .filter(|t| t.title.starts_with("Trace shipment"))
        .count()
}

async fn ask(
    w: &World,
    by: &User,
    shipment: &Shipment,
    reason: &str,
    liability: &str,
) -> renox::testing::TestResponse {
    w.post(
        by,
        &format!("/shipments/{}/mark-lost", shipment.id),
        &[("reason", reason), ("liability", liability)],
    )
    .await
}

async fn pending(w: &World, shipment: &Shipment) -> ApprovalRequest {
    ApprovalRequest::pending_about(w.db(), &shipment.subject())
        .await
        .unwrap()
        .pop()
        .expect("a request waits for approval")
}

/// AC1: given an `at_hub` shipment last scanned 2026-11-01 10:00, the run of
/// 2026-11-04 02:00 creates nothing; the run of 2026-11-05 02:00 creates a
/// trace task for the operations manager.
#[renox::test]
async fn the_rule_opens_a_trace_task_after_three_full_days_without_a_scan() {
    let w = boot().await;
    let boss = manager(&w, "Mona").await;
    let shipment = at_hub(&w, LAST_SCAN).await;

    let early = trace::run(w.state(), at(NOV_4)).await.unwrap();
    assert_eq!(early, TraceRun::default());
    assert!(tasks(&w).await.is_empty());
    assert_eq!(trace_tickets(&w), 0);

    let run = trace::run(w.state(), at(NOV_5)).await.unwrap();
    assert_eq!(run.opened, 1);
    assert_eq!(run.escalated, 0);
    let rows = tasks(&w).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].shipment_id, shipment.id);
    assert_eq!(rows[0].last_seen_at, at(LAST_SCAN));
    assert!(rows[0].closed_at.is_none() && rows[0].escalated_at.is_none());
    let ticket = w
        .tickets
        .opened()
        .into_iter()
        .find(|t| t.title.starts_with("Trace shipment"))
        .expect("a trace task");
    assert!(ticket.title.contains(&shipment.number));
    assert_eq!(ticket.assignee, Some(Assignee::user(boss.id)));
    // The rule is registered to run daily.
    assert_eq!(trace::TASK_AT, "02:00");
}

/// AC2: on 2026-11-09 02:00, still untraced, the trace task asks for
/// `mark_lost`; when dispatcher D requests it with reason and liability
/// *business* and operations manager M approves, the shipment is `lost` with
/// the BL-LOG-11 effects (claim ticket opened, client notified).
#[renox::test]
async fn after_seven_days_the_task_asks_for_mark_lost_and_another_manager_approves() {
    let w = boot().await;
    let boss = manager(&w, "Mona").await;
    let shipment = at_hub(&w, LAST_SCAN).await;
    clock_reaches(&w, NOV_9);
    trace::run(w.state(), at(NOV_5)).await.unwrap();

    let run = trace::run(w.state(), at(NOV_9)).await.unwrap();
    assert_eq!(run.opened, 0);
    assert_eq!(run.escalated, 1);
    assert!(tasks(&w).await[0].escalated_at.is_some());
    assert!(
        w.tickets
            .comments()
            .iter()
            .any(|(_, comment)| comment.text.contains("request mark lost now"))
    );
    // The rule never requests the loss itself.
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::AtHub);
    assert!(
        ApprovalRequest::pending_about(w.db(), &shipment.subject())
            .await
            .unwrap()
            .is_empty()
    );

    ask(&w, &w.dispatcher, &shipment, "Not at the hub", "business")
        .await
        .assert_status(303);
    // It waits for the second manager.
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::AtHub);
    let request = pending(&w, &shipment).await;
    assert_eq!(request.action, "logistics.mark_lost");
    assert_eq!(request.requester_id, w.dispatcher.id);

    approvals::approve(w.state(), request.id, &boss, "agreed")
        .await
        .unwrap()
        .expect("the request ends");

    let lost = w.reload(&shipment).await;
    assert_eq!(lost.status, ShipmentStatus::Lost);
    assert_eq!(lost.liability.as_deref(), Some("business"));
    assert_eq!(lost.lost_reason.as_deref(), Some("Not at the hub"));
    assert_eq!(lost.lost_from.as_deref(), Some("at_hub"));
    assert_eq!(lost.cod_amount, 0);
    // The shipment opens the claim: the delivery module has none.
    assert!(lost.claim_ticket_id.is_some());
    assert_eq!(
        w.tickets
            .opened()
            .iter()
            .filter(|t| t.category_key == "claim")
            .count(),
        1
    );
    let told: Vec<_> = w
        .notices
        .of_type(CLIENT_NOTICE)
        .into_iter()
        .filter(|n| n.data["title"].as_str().unwrap().contains("is lost"))
        .collect();
    assert_eq!(told.len(), 1);
    assert_eq!(w.shipment_lost.all().len(), 1);
    assert_eq!(w.shipment_lost.all()[0].from, "at_hub");
    assert!(Shipment::billable(w.db(), SOUQ).await.unwrap().is_empty());
    // The task closed with the change.
    assert!(tasks(&w).await[0].closed_at.is_some());
    let history = renox::state_machine::history(w.db(), Shipment::TABLE, shipment.id)
        .await
        .unwrap();
    assert!(
        history
            .iter()
            .any(|row| row.transition == "mark_lost_approved")
    );
}

/// AC3: D cannot approve; a request without a reason or liability is refused;
/// in a depot whose only operations manager is D, the company admin can
/// approve (and a requester who could approve still cannot approve their own).
#[renox::test]
async fn the_requester_cannot_approve_and_the_company_admin_can() {
    let w = boot().await;
    let shipment = at_hub(&w, LAST_SCAN).await;
    clock_reaches(&w, NOV_9);

    // Refused requests.
    ask(&w, &w.dispatcher, &shipment, "", "business")
        .await
        .assert_status(422);
    ask(&w, &w.dispatcher, &shipment, "Not at the hub", "")
        .await
        .assert_status(422);
    ask(
        &w,
        &w.dispatcher,
        &shipment,
        "Not at the hub",
        "the carrier",
    )
    .await
    .assert_status(422);
    ask(&w, &w.noor, &shipment, "Not at the hub", "business")
        .await
        .assert_forbidden();
    assert!(
        ApprovalRequest::pending_about(w.db(), &shipment.subject())
            .await
            .unwrap()
            .is_empty()
    );

    ask(&w, &w.dispatcher, &shipment, "Not at the hub", "none")
        .await
        .assert_status(303);
    let request = pending(&w, &shipment).await;
    // D holds no approval permission.
    assert!(
        approvals::approve(w.state(), request.id, &w.dispatcher, "mine")
            .await
            .is_err()
    );
    // A second request while one is pending is refused.
    ask(&w, &w.dispatcher, &shipment, "Again", "none")
        .await
        .assert_status(422);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::AtHub);

    // A manager who requested cannot approve their own request.
    let boss = manager(&w, "Mona").await;
    approvals::reject(w.state(), request.id, &boss, "start again")
        .await
        .unwrap();
    ask(&w, &boss, &shipment, "Not at the hub", "none")
        .await
        .assert_status(303);
    let own = pending(&w, &shipment).await;
    assert!(
        approvals::approve(w.state(), own.id, &boss, "mine")
            .await
            .is_err()
    );

    // The company admin holds every permission.
    let admin = w
        .user_with(
            "Admin",
            &[
                perm::SHIPMENTS_VIEW,
                perm::SHIPMENTS_MANAGE,
                perm::SHIPMENTS_APPROVE_LOSS,
            ],
        )
        .await;
    approvals::approve(w.state(), own.id, &admin, "agreed")
        .await
        .unwrap();
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Lost);
}

/// AC4: given an `at_hub` shipment last scanned 2026-11-03, dispatcher D can
/// request `mark_lost` on 2026-11-04 (before any trace task) and it still
/// needs another approver.
#[renox::test]
async fn a_request_before_any_trace_task_still_needs_another_approver() {
    let w = boot().await;
    let shipment = at_hub(&w, "2026-11-03T06:00:00Z").await;
    let run = trace::run(w.state(), at(NOV_4)).await.unwrap();
    assert_eq!(run, TraceRun::default());
    assert!(tasks(&w).await.is_empty());

    ask(&w, &w.dispatcher, &shipment, "Cannot find it", "driver")
        .await
        .assert_status(303);

    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::AtHub);
    let request = pending(&w, &shipment).await;
    assert_eq!(request.status, ApprovalStatus::Pending);
    assert!(tasks(&w).await.is_empty());
}

/// AC5: a scan on 2026-11-06 closes the trace task and restarts the clock.
#[renox::test]
async fn a_scan_closes_the_trace_task_and_restarts_the_clock() {
    let w = boot().await;
    manager(&w, "Mona").await;
    let shipment = at_hub(&w, LAST_SCAN).await;
    trace::run(w.state(), at(NOV_5)).await.unwrap();
    assert_eq!(tasks(&w).await.len(), 1);

    // Only a scanner scans, and a depot is named.
    w.post(
        &w.noor,
        &format!("/shipments/{}/depot-scan", shipment.id),
        &[("depot", "Depot A")],
    )
    .await
    .assert_forbidden();
    w.post(
        &w.hub,
        &format!("/shipments/{}/depot-scan", shipment.id),
        &[("depot", "")],
    )
    .await
    .assert_status(422);
    w.post(
        &w.hub,
        &format!("/shipments/{}/depot-scan", shipment.id),
        &[("depot", "Depot A")],
    )
    .await
    .assert_status(303);
    TrackingEvent::query()
        .where_(TrackingEvent::KIND.eq("depot_scan"))
        .update(w.db(), &[("occurred_at", &at("2026-11-06T06:00:00Z"))])
        .await
        .unwrap();

    assert!(tasks(&w).await[0].closed_at.is_some());
    assert!(
        w.tickets
            .completed()
            .iter()
            .any(|(_, note)| note.contains("Scanned at a depot"))
    );
    // The clock restarted at the scan: nothing on 2026-11-09 (64 hours).
    let run = trace::run(w.state(), at(NOV_9)).await.unwrap();
    assert_eq!(run, TraceRun::default());
    // 88 hours after the scan, a new task.
    let later = trace::run(w.state(), at("2026-11-09T22:00:00Z"))
        .await
        .unwrap();
    assert_eq!(later.opened, 1);
    assert_eq!(tasks(&w).await.len(), 2);
}

/// A note restarts the clock too, and closes the task.
#[renox::test]
async fn a_note_closes_the_trace_task_too() {
    let w = boot().await;
    manager(&w, "Mona").await;
    let shipment = at_hub(&w, LAST_SCAN).await;
    trace::run(w.state(), at(NOV_5)).await.unwrap();

    w.post(
        &w.noor,
        &format!("/shipments/{}/note", shipment.id),
        &[("note", "Called")],
    )
    .await
    .assert_forbidden();
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/note", shipment.id),
        &[("note", " ")],
    )
    .await
    .assert_status(422);
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/note", shipment.id),
        &[("note", "Called the driver, checking the run sheet")],
    )
    .await
    .assert_status(303);

    assert!(tasks(&w).await[0].closed_at.is_some());
    let events = w.reload(&shipment).await.events(w.db()).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "note"));
}

/// AC6: running the rule twice on the same day creates no second task.
#[renox::test]
async fn running_the_rule_twice_on_the_same_day_creates_one_task() {
    let w = boot().await;
    manager(&w, "Mona").await;
    at_hub(&w, LAST_SCAN).await;

    let first = trace::run(w.state(), at(NOV_5)).await.unwrap();
    let second = trace::run(w.state(), at(NOV_5)).await.unwrap();

    assert_eq!(first.opened, 1);
    assert_eq!(second, TraceRun::default());
    assert_eq!(tasks(&w).await.len(), 1);
    assert_eq!(trace_tickets(&w), 1);
    // Nor is the escalation repeated.
    trace::run(w.state(), at(NOV_9)).await.unwrap();
    let again = trace::run(w.state(), at(NOV_9)).await.unwrap();
    assert_eq!(again, TraceRun::default());
}

/// Approval pending and the shipment is scanned: the request fails at the
/// final approval and nothing happens.
#[renox::test]
async fn a_scan_after_the_request_fails_it_at_the_final_approval() {
    let w = boot().await;
    let boss = manager(&w, "Mona").await;
    let shipment = at_hub(&w, LAST_SCAN).await;
    ask(&w, &w.dispatcher, &shipment, "Not at the hub", "business")
        .await
        .assert_status(303);
    let request = pending(&w, &shipment).await;

    w.post(
        &w.hub,
        &format!("/shipments/{}/depot-scan", shipment.id),
        &[("depot", "Depot A")],
    )
    .await
    .assert_status(303);
    let decided = approvals::approve(w.state(), request.id, &boss, "agreed")
        .await
        .unwrap()
        .expect("the request ends");

    assert_eq!(decided.decision, ApprovalEnd::Failed);
    let stored = ApprovalRequest::find(w.db(), request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ApprovalStatus::Failed);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::AtHub);
    assert!(w.tickets.opened().iter().all(|t| t.category_key != "claim"));
    assert!(w.shipment_lost.all().is_empty());
}

/// Rejected: the shipment keeps its state and the trace task stays open.
#[renox::test]
async fn a_rejected_request_leaves_the_shipment_and_the_trace_task() {
    let w = boot().await;
    let boss = manager(&w, "Mona").await;
    let shipment = at_hub(&w, LAST_SCAN).await;
    trace::run(w.state(), at(NOV_9)).await.unwrap();
    ask(&w, &w.dispatcher, &shipment, "Not at the hub", "none")
        .await
        .assert_status(303);
    let request = pending(&w, &shipment).await;

    approvals::reject(w.state(), request.id, &boss, "check the other depot")
        .await
        .unwrap();

    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::AtHub);
    let rows = tasks(&w).await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].closed_at.is_none());
}

/// A shipment on a delivery is not marked lost here: the delivery module marks
/// it, and the request is refused; only the four states off a delivery are
/// traced (a delivered one is not).
#[renox::test]
async fn a_shipment_on_a_delivery_is_not_requested_or_traced_here() {
    let w = boot().await;
    let booked = w.book_as(&w.dispatcher, SOUQ, "ORD-1").await;
    let (out, _delivery) = w.out_for_delivery(&booked).await;
    touched_at(&w, &out, LAST_SCAN).await;

    ask(&w, &w.dispatcher, &out, "Not found", "none")
        .await
        .assert_status(422)
        .assert_see("delivery module");
    let run = trace::run(w.state(), at(NOV_9)).await.unwrap();
    assert_eq!(run, TraceRun::default());
    assert_eq!(w.reload(&out).await.status, ShipmentStatus::OutForDelivery);
}
