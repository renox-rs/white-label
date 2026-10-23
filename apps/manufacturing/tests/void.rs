//! BL-MFG-04 (#385): voiding a production report entered on the wrong work
//! order (`PLT.ACL.09`, `MFG.ACL.10`, `MFG.PRD.02`, 07 §7.1 *Work order*).
//! The operator asks with a reason, the line's supervisor approves (never the
//! requester), the report's `produce` and `wastage` movements are reversed and
//! the totals of the work order recalculated; a work order that is completed
//! locks its reports, and a lot that no longer holds the output refuses the
//! void.

mod support;

use manufacturing::{ProductionReport, ReportStatus, WorkOrder, WorkOrderStatus};
use renox::prelude::*;
use support::{Booted, boot};
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus, approve};
use wl_kernel::voids::VoidRecord;

/// WO on L1, `in_progress`, 180 panels reported (PR-1) and then PR-2 of 120
/// good and 5 scrap by operator Olga: the output lot holds 300 units.
struct Scene {
    t: Booted,
    olga: User,
    omar: User,
    sami: User,
    manager: User,
    order: WorkOrder,
    wrong: ProductionReport,
}

async fn scene() -> Scene {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let manager = t.manager("Mona", &plant).await;
    let sami = t.supervisor("Sami", &line).await;
    let olga = t.operator("Olga", &line).await;
    let omar = t.operator("Omar", &line).await;
    let order = t.released(&planner, &plant, &line).await;
    t.act(&sami, &order, "start", &[]).await.assert_status(303);
    t.act(&olga, &order, "report", &[("good", "180")])
        .await
        .assert_status(303);
    t.act(&olga, &order, "report", &[("good", "120"), ("scrap", "5")])
        .await
        .assert_status(303);
    let order = t.reload(order.id).await;
    let wrong = ProductionReport::query()
        .order_by_desc(ProductionReport::ID)
        .first(t.db())
        .await
        .unwrap()
        .expect("a report");
    assert_eq!((order.good_milli, order.scrap_milli), (300_000, 5_000));
    Scene {
        t,
        olga,
        omar,
        sami,
        manager,
        order,
        wrong,
    }
}

async fn void(s: &Scene, by: &User, reason: &str) -> renox::testing::TestResponse {
    s.t.post(
        by,
        &format!("/manufacturing/reports/{}/void", s.wrong.id),
        &[("reason", reason)],
    )
    .await
}

async fn request(s: &Scene) -> ApprovalRequest {
    ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq("manufacturing.production_report"))
        .where_(ApprovalRequest::SUBJECT_ID.eq(s.wrong.id))
        .order_by_desc(ApprovalRequest::ID)
        .first(s.t.db())
        .await
        .unwrap()
        .expect("a void request")
}

async fn report_now(s: &Scene) -> ProductionReport {
    ProductionReport::find_or_404(s.t.db(), s.wrong.id)
        .await
        .unwrap()
}

/// AC1: operator A voids PR-2 (120 good, 5 scrap) with reason "wrong WO" and
/// supervisor B approves: PR-2 is `voided` and keeps its number and reason,
/// its `produce` and `wastage` movements are reversed, the output lot holds
/// 180, the work order shows 120 fewer good units and no scrap, and
/// `RecordVoided` is emitted.
#[renox::test]
async fn the_supervisor_approves_and_the_reports_movements_are_reversed() {
    let s = scene().await;

    void(&s, &s.olga, "wrong WO").await.assert_status(303);
    let waiting = request(&s).await;
    assert_eq!(waiting.action, "manufacturing.report_void");
    assert_eq!(waiting.requester_id, s.olga.id);
    assert_eq!(waiting.status, ApprovalStatus::Pending);
    assert_eq!(report_now(&s).await.status, ReportStatus::Posted);
    assert_eq!(
        s.t.stock.output_left(),
        300_000,
        "nothing posts before the approval"
    );
    assert!(s.t.voided.all().is_empty());

    approve(s.t.state(), waiting.id, &s.sami, "ok")
        .await
        .unwrap();

    let voided = report_now(&s).await;
    assert_eq!(voided.status, ReportStatus::Voided);
    assert_eq!(voided.number(), format!("PR-{}", s.wrong.id));
    assert!(voided.stock_reversed_at.is_some());
    let record = VoidRecord::of_subject(s.t.db(), &voided.subject())
        .await
        .unwrap()
        .expect("the void record");
    assert_eq!(record.reason, "wrong WO");
    assert_eq!(record.voided_by, s.olga.id);
    assert_eq!(record.number.as_deref(), Some(voided.number().as_str()));
    let undone = s.t.stock.output_voided.lock().unwrap().clone();
    assert_eq!(undone.len(), 1);
    assert_eq!(undone[0].quantity_milli, 120_000);
    assert_eq!(undone[0].source, s.order.subject());
    assert!(undone[0].reason.contains(&voided.number()) && undone[0].reason.contains("wrong WO"));
    let waste = s.t.stock.waste_voided.lock().unwrap().clone();
    assert_eq!(waste.len(), 1);
    assert_eq!(waste[0].quantity_milli, 5_000);
    assert_eq!(s.t.stock.output_left(), 180_000);
    let order = s.t.reload(s.order.id).await;
    assert_eq!((order.good_milli, order.scrap_milli), (180_000, 0));
    assert_eq!(order.status, WorkOrderStatus::InProgress);
    let events = s.t.voided.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].subject, voided.subject());
    assert_eq!(events[0].user_id, s.olga.id);
}

/// AC2: the lot holds 100 units (200 were transferred out): the void of
/// PR-2 (120 units) is refused with "output already used" at once; no request
/// is made and nothing changes.
#[renox::test]
async fn a_void_is_refused_when_the_output_was_used() {
    let s = scene().await;
    s.t.stock.use_output(200_000);

    let refused = void(&s, &s.olga, "wrong WO").await;

    refused.assert_status(422);
    assert!(
        refused.text().contains("output already used")
            || refused.text().contains("Output already used")
    );
    assert!(refused.text().contains("100"), "{}", refused.text());
    assert_eq!(report_now(&s).await.status, ReportStatus::Posted);
    assert!(
        ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_TYPE.eq("manufacturing.production_report"))
            .first(s.t.db())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(s.t.reload(s.order.id).await.good_milli, 300_000);
}

/// AC3: A cannot approve their own void, and another operator, who does not
/// hold the approval permission, cannot either.
#[renox::test]
async fn the_operator_cannot_approve_their_own_void() {
    let s = scene().await;
    void(&s, &s.olga, "wrong WO").await.assert_status(303);
    let waiting = request(&s).await;

    let own = approve(s.t.state(), waiting.id, &s.olga, "mine").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    let other = approve(s.t.state(), waiting.id, &s.omar, "me too").await;
    assert!(matches!(other, Err(Error::Forbidden)), "{other:?}");
    assert_eq!(report_now(&s).await.status, ReportStatus::Posted);
    // A second void of the same report is refused while the first waits.
    let again = void(&s, &s.olga, "wrong WO").await;
    again.assert_status(422);
    assert!(again.text().contains("waits for approval"));
}

/// AC4: the work order is `completed`: void is refused with "output already
/// used", and the page does not offer it (it does while the work order is
/// `in_progress`).
#[renox::test]
async fn a_completed_work_orders_reports_cannot_be_voided() {
    let s = scene().await;
    let before = visible(
        &s.t.browse(
            &s.olga,
            &format!("/manufacturing/work-orders/{}", s.order.id),
        )
        .await
        .text(),
    );
    assert!(before.contains("Ask to void"), "{before}");

    s.t.act(&s.sami, &s.order, "complete", &[])
        .await
        .assert_status(303);

    let refused = void(&s, &s.olga, "wrong WO").await;
    refused.assert_status(422);
    assert!(refused.text().contains("utput already used"));
    let page =
        s.t.browse(
            &s.olga,
            &format!("/manufacturing/work-orders/{}", s.order.id),
        )
        .await;
    page.assert_ok();
    assert!(
        !page.text().contains("void-report-"),
        "no Void on a completed work order"
    );
    assert_eq!(report_now(&s).await.status, ReportStatus::Posted);
}

/// AC5: a void without a reason is refused.
#[renox::test]
async fn a_void_needs_a_reason() {
    let s = scene().await;

    let refused = void(&s, &s.olga, "   ").await;

    refused.assert_status(422);
    assert!(refused.text().contains("Say why"));
    assert_eq!(report_now(&s).await.status, ReportStatus::Posted);
}

/// The work order is completed while the void waits: the approval fails the
/// request (the kernel's guard re-check) and the report stays posted.
#[renox::test]
async fn completing_the_work_order_while_the_void_waits_fails_it() {
    let s = scene().await;
    void(&s, &s.olga, "wrong WO").await.assert_status(303);
    let waiting = request(&s).await;
    s.t.act(&s.sami, &s.order, "complete", &[])
        .await
        .assert_status(303);

    let decided = approve(s.t.state(), waiting.id, &s.sami, "ok")
        .await
        .unwrap();

    assert!(decided.is_some());
    let ended = ApprovalRequest::find_or_404(s.t.db(), waiting.id)
        .await
        .unwrap();
    assert_eq!(ended.status, ApprovalStatus::Failed);
    assert_eq!(report_now(&s).await.status, ReportStatus::Posted);
    assert_eq!(s.t.reload(s.order.id).await.good_milli, 300_000);
    assert!(s.t.stock.output_voided.lock().unwrap().is_empty());
    assert!(s.t.voided.all().is_empty());
}

/// A person who does not hold `work_orders.report` on the line cannot ask.
#[renox::test]
async fn only_the_line_can_void_its_reports() {
    let s = scene().await;
    let plant = s.t.plant("Plant 2").await;
    let other_line = s.t.line(&plant, "L2").await;
    let stranger = s.t.operator("Sol", &other_line).await;
    let qc = s.t.inspector("Qasim", &plant).await;

    void(&s, &stranger, "wrong WO").await.assert_status(403);
    void(&s, &qc, "wrong WO").await.assert_status(403);
    // The production manager of the plant approves nothing on the line she
    // is not assigned to, and asks nothing either.
    void(&s, &s.manager, "wrong WO").await.assert_status(403);
}

/// The stock reversal fails after the commit (the output was used between the
/// ask and the approval): the report is voided and its totals are corrected,
/// the claim stays open and the sweep posts the movements once the lot holds
/// the quantity again, then never twice.
#[renox::test]
async fn a_reversal_that_failed_is_posted_by_the_sweep_once() {
    let s = scene().await;
    void(&s, &s.olga, "wrong WO").await.assert_status(303);
    let waiting = request(&s).await;
    s.t.stock.use_output(250_000);

    let _ = approve(s.t.state(), waiting.id, &s.sami, "ok").await;

    let voided = report_now(&s).await;
    assert_eq!(voided.status, ReportStatus::Voided);
    assert!(voided.stock_reversed_at.is_none());
    assert!(s.t.stock.output_voided.lock().unwrap().is_empty());
    assert_eq!(manufacturing::report::sweep(s.t.state()).await.unwrap(), 0);

    s.t.stock.use_output(-250_000);
    assert_eq!(manufacturing::report::sweep(s.t.state()).await.unwrap(), 1);
    assert_eq!(manufacturing::report::sweep(s.t.state()).await.unwrap(), 0);
    assert_eq!(s.t.stock.output_voided.lock().unwrap().len(), 1);
    assert_eq!(s.t.stock.waste_voided.lock().unwrap().len(), 1);
    assert!(report_now(&s).await.stock_reversed_at.is_some());
}

/// The scrap of a report is a `wastage` movement of the finished item
/// against the work order, valued at the standard cost of a unit.
#[renox::test]
async fn the_scrap_of_a_report_gets_its_wastage_movement() {
    let s = scene().await;

    let wasted = s.t.stock.wasted.lock().unwrap().clone();

    assert_eq!(wasted.len(), 1, "only PR-2 had scrap");
    assert_eq!(wasted[0].quantity_milli, 5_000);
    assert_eq!(wasted[0].value, 12_000, "5 panels at AED 24 standard");
    assert_eq!(wasted[0].source, s.order.subject());
    assert_eq!(wasted[0].item.subject_id, support::PANEL);
    assert!(
        wasted[0].reason.contains(&s.wrong.number()),
        "{}",
        wasted[0].reason
    );
}

/// The voided report stays on the page, struck through with its reason, and
/// the request is on the approvals tab for the supervisor, who decides it
/// from there.
#[renox::test]
async fn the_page_shows_the_waiting_void_and_the_voided_report_struck_through() {
    let s = scene().await;
    let path = format!("/manufacturing/work-orders/{}", s.order.id);
    void(&s, &s.olga, "wrong WO").await.assert_status(303);

    let waiting = visible(&s.t.browse(&s.sami, &path).await.text());
    for word in [
        "Void waits for approval",
        "Void a production report",
        &s.wrong.number(),
    ] {
        assert!(waiting.contains(word), "{word}: {waiting}");
    }
    let html = s.t.browse(&s.sami, &path).await.text();
    assert!(html.contains("approve-"), "the supervisor can decide it");
    let operator_html = s.t.browse(&s.olga, &path).await.text();
    assert!(!operator_html.contains("approve-"), "the requester cannot");

    approve(s.t.state(), request(&s).await.id, &s.sami, "ok")
        .await
        .unwrap();
    let after = s.t.browse(&s.olga, &path).await.text();
    assert!(after.contains("<s>"), "a voided report is struck through");
    let after = visible(&after);
    assert!(
        after.contains("Voided") && after.contains("wrong WO"),
        "{after}"
    );
}

/// The visible text of a page: tags dropped.
fn visible(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    text.replace("&#39;", "'").replace("&amp;", "&")
}
