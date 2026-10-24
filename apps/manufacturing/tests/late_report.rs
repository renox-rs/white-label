//! BL-MFG-14 (#395): a production report typed in from the paper job cards
//! keeps its business time and takes the late-entry check (`PLT.ACL.10`,
//! 07 §7.1 *Integrations* and *Late entry*): flagged with a reason when it is
//! more than M minutes late, an approval request to the production manager
//! when it is more than 24 hours late.

mod support;

use manufacturing::{ProductionReport, WorkOrder};
use renox::chrono::Duration;
use renox::prelude::*;
use support::{Booted, boot};
use wl_kernel::approvals::{ApprovalRequest, approve};
use wl_kernel::late_entries::{Limits, set_limits};

struct Scene {
    t: Booted,
    sami: User,
    manager: User,
    order: WorkOrder,
}

async fn scene() -> Scene {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let manager = t.manager("Mona", &plant).await;
    let sami = t.supervisor("Sami", &line).await;
    let order = t.released(&planner, &plant, &line).await;
    t.act(&sami, &order, "start", &[]).await.assert_status(303);
    set_limits(
        t.db(),
        manufacturing::late_report::SUBJECT_TYPE,
        Limits {
            flag_after_minutes: 30,
            approval_after_hours: 24,
        },
    )
    .await
    .unwrap();
    Scene {
        t,
        sami,
        manager,
        order,
    }
}

/// The local form value of `hours` ago.
fn ago(hours: i64) -> String {
    let at = renox::db::now() - Duration::hours(hours);
    wl_kernel::BusinessCalendar::default()
        .local(at)
        .format("%Y-%m-%dT%H:%M")
        .to_string()
}

async fn reports(s: &Scene) -> Vec<ProductionReport> {
    ProductionReport::query()
        .where_(ProductionReport::WORK_ORDER_ID.eq(s.order.id))
        .get(s.t.db())
        .await
        .unwrap()
}

/// Done-when "Integrations and late entry": a report entered 5 hours late
/// gets only the flag: it is posted at once with its business time, no
/// approval request exists.
#[renox::test]
async fn five_hours_late_is_flagged_and_posted_without_approval() {
    let s = scene().await;
    let at = ago(5);
    s.t.act(
        &s.sami,
        &s.order,
        "report",
        &[
            ("good", "100"),
            ("occurred_at", &at),
            ("late_reason", "tablets down"),
        ],
    )
    .await
    .assert_status(303);

    let posted = reports(&s).await;
    assert_eq!(posted.len(), 1);
    assert!(posted[0].reported_at < renox::db::now() - Duration::hours(4));
    assert_eq!(s.t.reload(s.order.id).await.good_milli, 100_000);
    assert!(
        ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_TYPE.eq("kernel.late_entry"))
            .first(s.t.db())
            .await
            .unwrap()
            .is_none()
    );
}

/// A late entry without a reason is refused and posts nothing.
#[renox::test]
async fn a_late_entry_needs_a_reason() {
    let s = scene().await;
    let at = ago(5);
    s.t.act(
        &s.sami,
        &s.order,
        "report",
        &[("good", "100"), ("occurred_at", &at)],
    )
    .await
    .assert_status(422);
    assert!(reports(&s).await.is_empty());
}

/// Done-when "Integrations and late entry": a report entered 30 hours late
/// creates an approval request to the production manager; nothing is posted
/// until the manager approves, then the report is posted with its business
/// time.
#[renox::test]
async fn thirty_hours_late_waits_for_the_production_manager() {
    let s = scene().await;
    let at = ago(30);
    s.t.act(
        &s.sami,
        &s.order,
        "report",
        &[
            ("good", "80"),
            ("occurred_at", &at),
            ("late_reason", "tablets down all weekend"),
        ],
    )
    .await
    .assert_status(303);
    assert!(
        reports(&s).await.is_empty(),
        "nothing posts before approval"
    );
    let request = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq("kernel.late_entry"))
        .first(s.t.db())
        .await
        .unwrap()
        .expect("an approval request");
    assert_eq!(request.requester_id, s.sami.id);

    approve(s.t.state(), request.id, &s.manager, "ok")
        .await
        .unwrap();

    let posted = reports(&s).await;
    assert_eq!(posted.len(), 1);
    assert!(posted[0].reported_at < renox::db::now() - Duration::hours(29));
    assert_eq!(s.t.reload(s.order.id).await.good_milli, 80_000);
}

/// Output made now needs none of it.
#[renox::test]
async fn output_made_now_is_posted_as_before() {
    let s = scene().await;
    s.t.act(&s.sami, &s.order, "report", &[("good", "10")])
        .await
        .assert_status(303);
    assert_eq!(reports(&s).await.len(), 1);
}
