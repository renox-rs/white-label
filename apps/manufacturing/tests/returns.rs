//! BL-MFG-07 (#388): returned or refused goods are inspected and sales
//! decides to deliver again or cancel (07 §4.1 rows "accepts only part" and
//! "whole delivery is refused", §7.1 *Dispatch* `confirm_delivery` and
//! `return`, §7.2 `MFG.RUL.10`, `MFG.ACL.02`). The quarantine, dispatch and
//! sales events are emitted by hand (wl-inventory and wl-sales emit them); the
//! rule, its inspections, tasks, the pages and the actions are the real ones.

mod support;

use manufacturing::returns::{ReturnDecision, ReturnInspection, inspection};
use renox::prelude::*;
use wl_kernel::inventory::{
    DispatchGoodsQuarantined, DispatchSent, DispatchedLine, QuarantinedLine,
};
use wl_kernel::sales::{SalesOrderCancelled, SalesOrderId, SalesOrderLineCancelled};
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

const ORDER: i64 = 90;

/// The quarantined lines of dispatch 90: lot `lot` of sale line 1, `quantity`
/// units, with the reason.
fn booked(lot: i64, quantity: i64, reason: Option<&str>) -> QuarantinedLine {
    QuarantinedLine {
        line: Subject::new("inventory.dispatch_line", lot),
        item: Subject::new("inventory.item", support::PANEL),
        lot: Some(Subject::new("inventory.lot", lot)),
        source_line: Some(Subject::new("sales.sale_line", 1)),
        quantity_milli: quantity * 1_000,
        reason: reason.map(str::to_owned),
    }
}

async fn quarantined(
    t: &Booted,
    dispatch: i64,
    returned: bool,
    complaint: Option<i64>,
    lines: Vec<QuarantinedLine>,
) {
    t.state()
        .emit(DispatchGoodsQuarantined {
            dispatch: Subject::new("inventory.dispatch", dispatch),
            source: Subject::new("sales.sales_order", ORDER),
            recipient: Some(PartyId(7)),
            returned,
            complaint: complaint.map(|id| Subject::new("ticketing.ticket", id)),
            lines,
        })
        .await
        .unwrap();
}

async fn sent(t: &Booted, dispatch: i64, quantity: i64) {
    t.state()
        .emit(DispatchSent {
            dispatch: Subject::new("inventory.dispatch", dispatch),
            source: Subject::new("sales.sales_order", ORDER),
            recipient: Some(PartyId(7)),
            handover: "own_delivery".to_owned(),
            delivery: None,
            tracking_number: None,
            lines: vec![DispatchedLine {
                line: Subject::new("inventory.dispatch_line", 900 + dispatch),
                item: Subject::new("inventory.item", support::PANEL),
                lot: None,
                source_line: Some(Subject::new("sales.sale_line", 1)),
                quantity_milli: quantity * 1_000,
                delivered_milli: 0,
                refused_milli: 0,
                refusal_reason: None,
                value: 0,
            }],
        })
        .await
        .unwrap();
}

async fn cancelled(t: &Booted, quantity: i64) {
    t.state()
        .emit(SalesOrderLineCancelled {
            order: SalesOrderId(ORDER),
            party: PartyId(7),
            line: 1,
            quantity_milli: quantity * 1_000,
            reason: "customer does not want it".to_owned(),
            outcome: None,
            fee: 0,
            fee_waived: false,
        })
        .await
        .unwrap();
}

/// The scene: a plant with a QC inspector, a sales person who owns SO-90 (line
/// 1 has 20 open) and lot L5 of panels.
async fn scene() -> (Booted, manufacturing::Plant, User, User) {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let qc = t.inspector("Ines", &plant).await;
    let sales = t.sales("Sara").await;
    t.lines.open(ORDER, 1, 20_000, Some(sales.id));
    t.dispatches
        .directory(90, "delivered", ORDER, Some(7), None);
    t.lots.put(5, "L5", None, true, 20_000, 2_000, "released");
    (t, plant, qc, sales)
}

/// AC1 and AC4: Given DN-90 delivered with 80 accepted and 20 refused (reason
/// "damaged") of lot L5, when the goods are booked to quarantine, one
/// inspection on L5 for 20 exists, the sales person has a task for SO line 1
/// (20 open) and is notified; no complaint ticket exists for a partial
/// refusal.
#[renox::test]
async fn refused_goods_booked_to_quarantine_get_an_inspection_and_a_sales_task() {
    let (t, _plant, qc, sales) = scene().await;

    quarantined(&t, 90, false, None, vec![booked(5, 20, Some("damaged"))]).await;

    let inspections = ReturnInspection::query().get(t.db()).await.unwrap();
    assert_eq!(inspections.len(), 1);
    assert_eq!(inspections[0].lot_id, Some(5));
    assert_eq!(inspections[0].quantity_milli, 20_000);
    assert_eq!(inspections[0].reason.as_deref(), Some("damaged"));
    assert!(!inspections[0].returned);
    assert_eq!(inspections[0].complaint_id, None);
    assert_eq!(inspections[0].status, inspection::OPEN);
    let decisions = ReturnDecision::query().get(t.db()).await.unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!((decisions[0].line_id, decisions[0].open_milli), (1, 20_000));
    assert_eq!(decisions[0].owner_id, Some(sales.id));
    let tasks = t.tickets.all();
    // The QC task (no checklist provider) and the sales task.
    assert_eq!(tasks.len(), 2);
    let mine = tasks
        .iter()
        .find(|task| task.ticket.title.contains("Order SO-90"))
        .expect("the sales task");
    assert_eq!(
        mine.ticket.assignee,
        Some(wl_kernel::tickets::Assignee::user(sales.id))
    );
    assert!(mine.ticket.description.contains("damaged"));
    assert!(mine.ticket.description.contains("20"));
    let qc_task = tasks
        .iter()
        .find(|task| task.ticket.title.contains("Inspect returned lot"))
        .expect("the QC task");
    assert_eq!(
        qc_task.ticket.assignee,
        Some(wl_kernel::tickets::Assignee::user(qc.id))
    );
    let told = t.notices.titles_for(&sales);
    assert!(
        told.iter().any(|title| title.contains("came back")),
        "{told:?}"
    );
}

/// AC2: Given DN-91 `returned`, an inspection per lot and one sales task
/// exist, linked to the complaint ticket; and with checklists (`WRK.CHK.01`)
/// each inspection is a checklist run on the lot, and no QC task is opened.
#[renox::test]
async fn a_returned_dispatch_gets_an_inspection_per_lot_one_sales_task_and_the_complaint_link() {
    let (t, _plant, _qc, _sales) = scene().await;
    t.runs.enable();

    quarantined(
        &t,
        91,
        true,
        Some(77),
        vec![
            booked(5, 12, Some("customer_unavailable")),
            booked(6, 8, Some("customer_unavailable")),
        ],
    )
    .await;

    let inspections = ReturnInspection::query()
        .order_by(ReturnInspection::ID)
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(inspections.len(), 2, "one per lot");
    assert!(inspections.iter().all(|found| found.returned));
    assert!(
        inspections
            .iter()
            .all(|found| found.complaint_id == Some(77))
    );
    assert!(inspections.iter().all(|found| found.run_id.is_some()));
    let runs = t.runs.started();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].0, "returned_goods_inspection");
    assert_eq!(runs[0].1, Subject::new("inventory.lot", 5));
    let decisions = ReturnDecision::query().get(t.db()).await.unwrap();
    assert_eq!(decisions.len(), 1, "one sales task for the line");
    assert_eq!(decisions[0].open_milli, 20_000);
    let tasks = t.tickets.all();
    assert_eq!(
        tasks.len(),
        1,
        "no QC task: the checklist is the inspection"
    );
    let comments = t.tickets.comments();
    assert_eq!(comments.iter().filter(|(id, _)| *id == 77).count(), 2);
    assert!(comments[0].1.contains("Inspection started"));
}

/// AC3: when sales runs `cancel_line` for the 20, the task closes.
#[renox::test]
async fn cancelling_the_line_closes_the_decision() {
    let (t, _plant, _qc, _sales) = scene().await;
    quarantined(&t, 90, false, None, vec![booked(5, 20, Some("damaged"))]).await;
    t.lines.set(ORDER, 1, 0);

    cancelled(&t, 20).await;

    let decision = ReturnDecision::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decision.status, "closed");
    assert_eq!(decision.closed_why.as_deref(), Some("cancelled"));
    assert_eq!(decision.open_milli, 0);
    assert!(
        t.tickets
            .all()
            .iter()
            .any(|task| task.done && task.ticket.title.contains("Order SO-90"))
    );
    // Nothing is cancelled by the rule itself.
    assert_eq!(t.tickets.all().len(), 2);
}

/// AC5: Given a new dispatch DN-92 for 10 of the 20 that is `dispatched`, the
/// task stays open with 10 open; after `cancel_line` for the other 10 it
/// closes.
#[renox::test]
async fn a_part_delivered_again_lowers_the_open_quantity_and_the_rest_closes_it() {
    let (t, _plant, _qc, _sales) = scene().await;
    quarantined(&t, 90, false, None, vec![booked(5, 20, Some("damaged"))]).await;

    sent(&t, 92, 10).await;

    let decision = ReturnDecision::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decision.status, "open");
    assert_eq!(decision.open_milli, 10_000);
    assert!(
        t.tickets
            .comments()
            .iter()
            .any(|(_, text)| text.contains("10 is still open"))
    );
    // The same dispatch told again takes nothing more.
    sent(&t, 92, 10).await;
    let again = ReturnDecision::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.open_milli, 10_000);

    t.lines.set(ORDER, 1, 0);
    cancelled(&t, 10).await;
    let closed = ReturnDecision::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(closed.status, "closed");
    assert_eq!(closed.open_milli, 0);
}

/// AC6: replaying the event creates no second inspection, decision or task.
#[renox::test]
async fn replaying_the_quarantine_event_creates_nothing_new() {
    let (t, _plant, _qc, _sales) = scene().await;
    let lines = || vec![booked(5, 20, Some("damaged"))];
    quarantined(&t, 90, false, None, lines()).await;
    let tasks = t.tickets.all().len();

    quarantined(&t, 90, false, None, lines()).await;

    assert_eq!(
        ReturnInspection::query().get(t.db()).await.unwrap().len(),
        1
    );
    assert_eq!(ReturnDecision::query().get(t.db()).await.unwrap().len(), 1);
    assert_eq!(t.tickets.all().len(), tasks);
}

/// The account has no sales person on the order: the task goes to a holder of
/// the sales role.
#[renox::test]
async fn an_order_without_an_owner_goes_to_the_sales_role() {
    let t = boot().await;
    let _plant = t.plant("Plant 1").await;
    let sales = t.sales("Sara").await;
    t.lines.open(ORDER, 1, 20_000, None);
    t.dispatches
        .directory(90, "delivered", ORDER, Some(7), None);

    quarantined(&t, 90, false, None, vec![booked(5, 20, Some("wrong_item"))]).await;

    let decision = ReturnDecision::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decision.owner_id, Some(sales.id));
}

/// A cancelled order closes its decisions.
#[renox::test]
async fn a_cancelled_order_closes_the_decision() {
    let (t, _plant, _qc, _sales) = scene().await;
    quarantined(&t, 90, false, None, vec![booked(5, 20, Some("damaged"))]).await;

    t.state()
        .emit(SalesOrderCancelled {
            order: SalesOrderId(ORDER),
            party: PartyId(7),
            reason: "customer left".to_owned(),
            outcomes: Vec::new(),
        })
        .await
        .unwrap();

    let decision = ReturnDecision::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decision.closed_why.as_deref(), Some("order_cancelled"));
}

/// `MFG.ACL.02`: the lot is not released while its inspection waits; QC passes
/// it on the page and the lot is released. A failed inspection raises an NCR
/// and the lot stays out.
#[renox::test]
async fn a_lot_is_released_only_through_a_passed_inspection() {
    use wl_kernel::lot_disposal::lot_release_blocked;
    let (t, plant, qc, _sales) = scene().await;
    quarantined(&t, 90, false, None, vec![booked(5, 20, Some("damaged"))]).await;
    let lot = Subject::new("inventory.lot", 5);
    let blocked = lot_release_blocked(t.state(), &lot).await.unwrap();
    assert!(
        blocked.is_some_and(|why| why.contains("wait for their inspection")),
        "the guard names the inspection"
    );
    let found = ReturnInspection::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();

    t.post(
        &qc,
        &format!("/manufacturing/inspections/{}/pass", found.id),
        &[("note", "packaging fine")],
    )
    .await
    .assert_status(303);

    let after = ReturnInspection::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.status, inspection::PASSED);
    assert_eq!(after.decided_by, Some(qc.id));
    assert_eq!(t.lots.released.lock().unwrap().len(), 1);
    assert!(
        lot_release_blocked(t.state(), &lot)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        t.tickets
            .all()
            .iter()
            .any(|task| task.done && task.ticket.title.contains("Inspect returned lot"))
    );
    // A second result is refused.
    t.post(
        &qc,
        &format!("/manufacturing/inspections/{}/fail", found.id),
        &[("defect", "x")],
    )
    .await
    .assert_status(422);

    // Another lot fails: an NCR is raised on it.
    quarantined(&t, 93, false, None, vec![booked(8, 4, Some("wet"))]).await;
    t.lots.put(8, "L8", None, true, 4_000, 2_000, "released");
    let second = ReturnInspection::query()
        .where_(ReturnInspection::LOT_ID.eq(Some(8_i64)))
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    t.post(
        &qc,
        &format!("/manufacturing/inspections/{}/fail", second.id),
        &[("defect", "")],
    )
    .await
    .assert_status(303);
    let failed = ReturnInspection::find_or_404(t.db(), second.id)
        .await
        .unwrap();
    assert_eq!(failed.status, inspection::FAILED);
    let ncr = manufacturing::Ncr::find_or_404(t.db(), failed.ncr_id.unwrap())
        .await
        .unwrap();
    assert_eq!(ncr.lot_id, Some(8));
    assert_eq!(ncr.quantity_milli, 4_000);
    assert_eq!(ncr.defect, "wet");
    assert_eq!(ncr.plant_id, plant.id);
    // The NCR keeps the lot from release.
    assert!(
        lot_release_blocked(t.state(), &Subject::new("inventory.lot", 8))
            .await
            .unwrap()
            .is_some()
    );
}

/// Only QC records the result; the pages are open to the roles that work with
/// the returned goods.
#[renox::test]
async fn the_pages_show_the_returns_and_the_inspections_to_the_right_people() {
    let (t, plant, qc, sales) = scene().await;
    quarantined(&t, 91, true, Some(77), vec![booked(5, 20, Some("damaged"))]).await;
    let operator = t.operator("Omar", &t.line(&plant, "L1").await).await;

    let returns = t.browse(&sales, "/manufacturing/returns").await;
    returns.assert_ok();
    let text = returns.text();
    for word in [
        "Returned goods",
        "SO-90",
        "damaged",
        "New dispatch",
        "Still open",
    ] {
        assert!(text.contains(word), "{word} missing in {text}");
    }
    t.browse(&operator, "/manufacturing/returns")
        .await
        .assert_forbidden();
    t.browse(&sales, "/manufacturing/inspections")
        .await
        .assert_forbidden();
    let page = t.browse(&qc, "/manufacturing/inspections").await;
    page.assert_ok();
    let text = page.text();
    for word in [
        "Returned lots",
        "L5",
        "Pass",
        "Fail",
        "Waiting",
        "The whole dispatch came back",
    ] {
        assert!(text.contains(word), "{word} missing in {text}");
    }
    let json = t.get(&sales, "/manufacturing/returns").await;
    assert!(json.text().contains("\"open\":\"20\""), "{}", json.text());
}

/// *New dispatch* makes a draft for the open quantity (or less) from the
/// original dispatch; more than is open is refused; a closed decision offers
/// nothing.
#[renox::test]
async fn new_dispatch_asks_the_inventory_for_a_draft_copy_of_the_original() {
    let (t, _plant, qc, sales) = scene().await;
    quarantined(&t, 90, false, None, vec![booked(5, 20, Some("damaged"))]).await;
    let decision = ReturnDecision::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    let path = format!("/manufacturing/returns/{}/dispatch", decision.id);

    t.post(&sales, &path, &[("quantity", "30")])
        .await
        .assert_status(422);
    t.post(&qc, &path, &[("quantity", "5")])
        .await
        .assert_forbidden();
    let made = t.post(&sales, &path, &[("quantity", "")]).await;
    made.assert_status(303);

    let asked = t.maker.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].0, Subject::new("inventory.dispatch", 90));
    assert_eq!(
        asked[0].1[0].source_line,
        Subject::new("sales.sale_line", 1)
    );
    assert_eq!(asked[0].1[0].quantity_milli, 20_000);
    t.lines.set(ORDER, 1, 0);
    cancelled(&t, 20).await;
    t.post(&sales, &path, &[("quantity", "5")])
        .await
        .assert_status(422);
}
