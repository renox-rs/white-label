//! BL-MFG-08 (#389): QC dispositions an NCR as return to supplier, scrap,
//! rework or use as is, and closes it (07 Manufacturing §7.1 *NCR*, §4.3 row
//! "A lot fails incoming inspection", `MFG.ACL.03`, `STK.ACL.02`,
//! `STK.ACL.06`). The Return to supplier, the scrap and the release are the
//! inventory's, reached through the kernel's `LotDisposal` (a fake here; the
//! real one is in `host/tests/manufacturing_ncr.rs`).

mod support;

use manufacturing::{Cause, Disposition, Ncr, NcrStatus, Plant, WorkOrderStatus};
use renox::prelude::*;
use support::{Booted, boot};
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus, approve};
use wl_kernel::inventory::{ReturnOutcome, SupplierReturnDispatched};
use wl_kernel::lot_disposal::{SupplierReturnCancelled, lot_release_blocked};
use wl_kernel::{PartyId, Subject};

const SUPPLIER: i64 = 7;

/// The plant with two QC inspectors, the production manager and a holder of
/// `stock.approve`; lot 7 (100 kg from supplier 7 at AED 30, `rejected`),
/// lot 8 (100 kg `on_hold` at AED 25), lot 9 (100 kg `quarantine` at AED 30),
/// lot 10 (made in-house) and lot 11 (opening stock, no supplier lot).
struct Scene {
    t: Booted,
    plant: Plant,
    qc: User,
    other_qc: User,
    manager: User,
    approver: User,
}

async fn scene() -> Scene {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let qc = t.inspector("Ines", &plant).await;
    let other_qc = t.inspector("Omar", &plant).await;
    let manager = t.manager("Mona", &plant).await;
    let approver = t.stock_approver("Stella").await;
    t.lots
        .put(7, "L7", Some(SUPPLIER), false, 100_000, 3_000, "rejected");
    t.lots
        .put(8, "L8", Some(SUPPLIER), false, 100_000, 2_500, "on_hold");
    t.lots
        .put(9, "L9", Some(SUPPLIER), false, 100_000, 3_000, "quarantine");
    t.lots
        .put(10, "F10", None, true, 100_000, 3_000, "quarantine");
    t.lots
        .put(11, "OLD", None, false, 100_000, 3_000, "quarantine");
    Scene {
        t,
        plant,
        qc,
        other_qc,
        manager,
        approver,
    }
}

fn credit() -> Vec<(&'static str, &'static str)> {
    vec![("disposition", "return_to_supplier"), ("outcome", "credit")]
}

fn is_close_task(opened: &support::Opened, ncr: &Ncr) -> bool {
    opened.ticket.source_key.as_deref() == Some(&format!("manufacturing.close_ncr:{}", ncr.id))
}

fn is_redisposition_task(opened: &support::Opened, ncr: &Ncr) -> bool {
    opened.ticket.source_key.as_deref()
        == Some(&format!("manufacturing.redisposition_ncr:{}", ncr.id))
}

async fn dispatched(s: &Scene, ncr: &Ncr) {
    s.t.lots.set_return(1, "dispatched");
    s.t.state()
        .emit(SupplierReturnDispatched {
            supplier_return: Subject::new("inventory.supplier_return", 1),
            supplier: PartyId(SUPPLIER),
            source: ncr.subject(),
            lines: Vec::new(),
        })
        .await
        .unwrap();
}

/// AC1: lot L7 (supplier S, 100 kg at AED 30) with NCR-5 for 100 kg, QC
/// dispositions *return to supplier* with outcome *credit*: a draft Return to
/// supplier for 100 kg of L7 with the NCR as its source is asked of the
/// inventory, and NCR-5 is `dispositioned` with the decision kept.
#[renox::test]
async fn returning_to_the_supplier_asks_for_a_draft_return_with_the_ncr_as_source() {
    let s = scene().await;
    let ncr = s.t.ncr(&s.qc, &s.plant, 7, "100").await;

    s.t.ncr_act(&s.qc, &ncr, "disposition", &credit())
        .await
        .assert_status(303);

    let after = s.t.ncr_now(ncr.id).await;
    assert_eq!(after.status, NcrStatus::Dispositioned);
    assert_eq!(after.disposition, Some(Disposition::ReturnToSupplier));
    assert_eq!(after.outcome.as_deref(), Some("credit"));
    assert_eq!(after.dispositioned_by, Some(s.qc.id));
    let made = s.t.lots.made();
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].request.ncr, ncr.subject());
    assert_eq!(made[0].request.lot, Subject::new("inventory.lot", 7));
    assert_eq!(made[0].request.quantity_milli, 100_000);
    assert_eq!(made[0].request.outcome, ReturnOutcome::Credit);
    assert_eq!(made[0].request.reason, "colour off");
    assert_eq!(after.supplier_return(), Some(made[0].subject.clone()));
    let decisions = manufacturing::ncrs::decisions(s.t.db(), ncr.id)
        .await
        .unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].disposition, Disposition::ReturnToSupplier);
    assert!(decisions[0].ended_at.is_none());
}

/// AC3: NCR-5 with its Return to supplier `draft`: `close` is refused. AC2:
/// once the storekeeper dispatched it, QC gets a task to close, the lot is
/// closed out, `close` is allowed and QC closes it (the system never does).
#[renox::test]
async fn close_waits_for_the_dispatch_and_is_qcs_task() {
    let s = scene().await;
    let ncr = s.t.ncr(&s.qc, &s.plant, 7, "100").await;
    s.t.ncr_act(&s.qc, &ncr, "disposition", &credit())
        .await
        .assert_status(303);

    let early = s.t.ncr_act(&s.qc, &ncr, "close", &[]).await;
    dispatched(&s, &ncr).await;
    dispatched(&s, &ncr).await;
    let tasks: Vec<_> =
        s.t.tickets
            .all()
            .into_iter()
            .filter(|o| is_close_task(o, &ncr))
            .collect();
    let still = s.t.ncr_now(ncr.id).await;
    s.t.ncr_act(&s.qc, &ncr, "close", &[])
        .await
        .assert_status(303);

    early.assert_status(422);
    assert!(early.text().contains("not done yet"), "{}", early.text());
    assert_eq!(tasks.len(), 1, "one task, however many times it is told");
    assert_eq!(tasks[0].ticket.title, format!("Close {}", ncr.number));
    assert_eq!(
        still.status,
        NcrStatus::Dispositioned,
        "the system does not close it"
    );
    assert!(still.action_done_at.is_some());
    assert_eq!(
        s.t.lots.closed_out.lock().unwrap().clone(),
        vec![Subject::new("inventory.lot", 7)]
    );
    let closed = s.t.ncr_now(ncr.id).await;
    assert_eq!(closed.status, NcrStatus::Closed);
    assert_eq!(closed.closed_by, Some(s.qc.id));
    assert!(
        s.t.tickets
            .all()
            .iter()
            .all(|o| !is_close_task(o, &ncr) || o.done)
    );
}

/// AC4: lot L8 `on_hold` with NCR-6, disposition *scrap* of AED 2,500: an
/// approval under `STK.ACL.02` is requested and nothing is scrapped (the lot
/// stays as it is) until a holder of `stock.approve` approves; then the scrap
/// is posted once and QC is asked to close. A scrap of AED 250 runs at once.
#[renox::test]
async fn a_scrap_above_the_limit_waits_for_stock_approval() {
    let s = scene().await;
    let ncr = s.t.ncr(&s.qc, &s.plant, 8, "100").await;

    s.t.ncr_act(&s.qc, &ncr, "disposition", &[("disposition", "scrap")])
        .await
        .assert_status(303);

    let waiting = s.t.ncr_now(ncr.id).await;
    assert_eq!(
        waiting.status,
        NcrStatus::Open,
        "decided only after approval"
    );
    assert!(s.t.lots.scrapped.lock().unwrap().is_empty());
    let request = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_ID.eq(ncr.id))
        .where_(ApprovalRequest::ACTION.eq("manufacturing.ncr_scrap"))
        .first(s.t.db())
        .await
        .unwrap()
        .expect("an approval request");
    assert_eq!(request.status, ApprovalStatus::Pending);
    assert_eq!(request.payload["scrap_value"], 250_000);
    let twice =
        s.t.ncr_act(&s.qc, &ncr, "disposition", &[("disposition", "scrap")])
            .await;
    twice.assert_status(422);
    assert!(twice.text().contains("waits for an approval"));
    let own = approve(s.t.state(), request.id, &s.qc, "mine").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");

    approve(s.t.state(), request.id, &s.approver, "agreed")
        .await
        .unwrap();

    let done = s.t.ncr_now(ncr.id).await;
    assert_eq!(done.status, NcrStatus::Dispositioned);
    assert_eq!(done.disposition, Some(Disposition::Scrap));
    assert_eq!(done.scrap_value, 250_000);
    assert!(done.action_done_at.is_some());
    let scrapped = s.t.lots.scrapped.lock().unwrap().clone();
    assert_eq!(scrapped.len(), 1);
    assert_eq!(scrapped[0].quantity_milli, 100_000);
    assert_eq!(scrapped[0].ncr, ncr.subject());
    assert!(s.t.tickets.all().iter().any(|o| is_close_task(o, &ncr)));
    s.t.ncr_act(&s.qc, &ncr, "close", &[])
        .await
        .assert_status(303);

    let small = s.t.ncr(&s.qc, &s.plant, 8, "10").await;
    s.t.ncr_act(&s.qc, &small, "disposition", &[("disposition", "scrap")])
        .await
        .assert_status(303);
    assert_eq!(s.t.ncr_now(small.id).await.status, NcrStatus::Dispositioned);
    assert_eq!(s.t.lots.scrapped.lock().unwrap().len(), 2);
}

/// AC5: a lot made by a work order cannot be returned to a supplier, nor one
/// with no supplier lot (opening stock): 422 with words, and the NCR stays
/// open for another disposition; the outcome must be credit or replace.
#[renox::test]
async fn return_to_supplier_is_refused_without_a_supplier_lot_or_for_own_production() {
    let s = scene().await;
    let own = s.t.ncr(&s.qc, &s.plant, 10, "10").await;
    let old = s.t.ncr(&s.qc, &s.plant, 11, "10").await;
    let kept = s.t.ncr(&s.qc, &s.plant, 9, "10").await;

    let in_house = s.t.ncr_act(&s.qc, &own, "disposition", &credit()).await;
    let unknown = s.t.ncr_act(&s.qc, &old, "disposition", &credit()).await;
    let no_outcome =
        s.t.ncr_act(
            &s.qc,
            &kept,
            "disposition",
            &[("disposition", "return_to_supplier")],
        )
        .await;

    in_house.assert_status(422);
    assert!(
        in_house.text().contains("made in-house"),
        "{}",
        in_house.text()
    );
    unknown.assert_status(422);
    assert!(
        unknown.text().contains("supplier lot is not known"),
        "{}",
        unknown.text()
    );
    no_outcome.assert_status(422);
    assert!(no_outcome.text().contains("credit or replace"));
    for ncr in [&own, &old, &kept] {
        assert_eq!(s.t.ncr_now(ncr.id).await.status, NcrStatus::Open);
    }
    // Nothing was asked of the inventory.
    assert!(s.t.lots.made().is_empty());
    let page =
        s.t.browse(&s.qc, &format!("/manufacturing/ncrs/{}", own.id))
            .await;
    page.assert_ok();
    assert!(
        !page.text().contains("return-ncr"),
        "not offered for own production"
    );
    let page =
        s.t.browse(&s.qc, &format!("/manufacturing/ncrs/{}", old.id))
            .await;
    assert!(page.text().contains("supplier lot is not known"));
}

/// AC6: NCR-7 `dispositioned` *return to supplier* with its Return to
/// supplier `draft`; when the return is cancelled, NCR-7 is `open`, its
/// history shows the earlier disposition, QC has a task, and QC can then
/// disposition *scrap*.
#[renox::test]
async fn a_cancelled_return_reopens_the_ncr_for_a_new_disposition() {
    let s = scene().await;
    let ncr = s.t.ncr(&s.qc, &s.plant, 9, "20").await;
    s.t.ncr_act(&s.qc, &ncr, "disposition", &credit())
        .await
        .assert_status(303);

    s.t.lots.set_return(1, "cancelled");
    for _ in 0..2 {
        s.t.state()
            .emit(SupplierReturnCancelled {
                supplier_return: Subject::new("inventory.supplier_return", 1),
                supplier: PartyId(SUPPLIER),
                source: ncr.subject(),
                reason: Some("the supplier refuses collection".into()),
            })
            .await
            .unwrap();
    }

    let open = s.t.ncr_now(ncr.id).await;
    assert_eq!(open.status, NcrStatus::Open);
    assert_eq!(open.disposition, None);
    assert_eq!(open.return_id, None);
    let decisions = manufacturing::ncrs::decisions(s.t.db(), ncr.id)
        .await
        .unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].disposition, Disposition::ReturnToSupplier);
    assert!(decisions[0].ended_at.is_some());
    assert_eq!(
        decisions[0].ended_reason.as_deref(),
        Some("The Return to supplier was cancelled.")
    );
    let tasks =
        s.t.tickets
            .all()
            .into_iter()
            .filter(|o| is_redisposition_task(o, &ncr))
            .count();
    assert_eq!(tasks, 1);
    s.t.ncr_act(&s.qc, &ncr, "disposition", &[("disposition", "scrap")])
        .await
        .assert_status(303);
    let again = s.t.ncr_now(ncr.id).await;
    assert_eq!(again.status, NcrStatus::Dispositioned);
    assert_eq!(again.disposition, Some(Disposition::Scrap));
    let decisions = manufacturing::ncrs::decisions(s.t.db(), ncr.id)
        .await
        .unwrap();
    assert_eq!(decisions.len(), 2);
    let page =
        s.t.browse(&s.qc, &format!("/manufacturing/ncrs/{}", ncr.id))
            .await;
    assert!(page.text().contains("Return to supplier"));
    assert!(page.text().contains("Scrap"));
}

/// AC7: lot L9 (100 kg in quarantine) with NCR-8 for 30 kg returned: the lot is
/// not released while the Return to supplier is `draft` (and not while the
/// NCR is open); after the dispatch, or when the NCR is cancelled, it may be.
#[renox::test]
async fn a_partial_ncr_keeps_the_lot_from_release_until_its_movement_posts() {
    let s = scene().await;
    let lot = Subject::new("inventory.lot", 9);
    let ncr = s.t.ncr(&s.qc, &s.plant, 9, "30").await;
    let open = lot_release_blocked(s.t.state(), &lot).await.unwrap();
    s.t.ncr_act(&s.qc, &ncr, "disposition", &credit())
        .await
        .assert_status(303);

    let draft = lot_release_blocked(s.t.state(), &lot).await.unwrap();
    dispatched(&s, &ncr).await;
    let posted = lot_release_blocked(s.t.state(), &lot).await.unwrap();

    assert!(open.is_some_and(|why| why.contains("open")));
    assert!(draft.is_some_and(|why| why.contains("dispositioned")));
    assert_eq!(posted, None);
    let other = s.t.ncr(&s.qc, &s.plant, 9, "5").await;
    assert!(
        lot_release_blocked(s.t.state(), &lot)
            .await
            .unwrap()
            .is_some()
    );
    s.t.ncr_act(&s.qc, &other, "cancel", &[("reason", "raised twice")])
        .await
        .assert_status(303);
    assert_eq!(lot_release_blocked(s.t.state(), &lot).await.unwrap(), None);
}

/// *Use as is* (`MFG.ACL.03`): it waits for an approval of someone other than
/// the inspector who asks; the inspector who raised the NCR does not approve
/// it either; the production manager does, and then the lot is released and
/// QC is asked to close.
#[renox::test]
async fn use_as_is_needs_somebody_else_to_approve_and_then_releases_the_lot() {
    let s = scene().await;
    let ncr = s.t.ncr(&s.qc, &s.plant, 9, "100").await;

    s.t.ncr_act(
        &s.other_qc,
        &ncr,
        "disposition",
        &[("disposition", "use_as_is")],
    )
    .await
    .assert_status(303);

    assert_eq!(s.t.ncr_now(ncr.id).await.status, NcrStatus::Open);
    assert!(s.t.lots.released.lock().unwrap().is_empty());
    let request = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_ID.eq(ncr.id))
        .where_(ApprovalRequest::ACTION.eq("manufacturing.ncr_use_as_is"))
        .first(s.t.db())
        .await
        .unwrap()
        .expect("an approval request");
    let asker = approve(s.t.state(), request.id, &s.other_qc, "mine").await;
    assert!(matches!(asker, Err(Error::Forbidden)), "{asker:?}");
    approve(s.t.state(), request.id, &s.qc, "my own NCR")
        .await
        .unwrap();
    let failed = ApprovalRequest::find_or_404(s.t.db(), request.id)
        .await
        .unwrap();
    assert_eq!(
        failed.status,
        ApprovalStatus::Failed,
        "the raiser does not approve"
    );
    assert_eq!(s.t.ncr_now(ncr.id).await.status, NcrStatus::Open);
    assert!(s.t.lots.released.lock().unwrap().is_empty());

    s.t.ncr_act(
        &s.other_qc,
        &ncr,
        "disposition",
        &[("disposition", "use_as_is")],
    )
    .await
    .assert_status(303);
    let again = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_ID.eq(ncr.id))
        .where_(ApprovalRequest::STATUS.eq(ApprovalStatus::Pending))
        .first(s.t.db())
        .await
        .unwrap()
        .expect("asked again");
    approve(s.t.state(), again.id, &s.manager, "fine for the pipes")
        .await
        .unwrap();

    let done = s.t.ncr_now(ncr.id).await;
    assert_eq!(done.status, NcrStatus::Dispositioned);
    assert_eq!(done.disposition, Some(Disposition::UseAsIs));
    assert!(done.action_done_at.is_some());
    assert_eq!(
        s.t.lots.released.lock().unwrap().clone(),
        vec![Subject::new("inventory.lot", 9)]
    );
    assert!(s.t.tickets.all().iter().any(|o| is_close_task(o, &ncr)));
    s.t.ncr_act(&s.qc, &ncr, "close", &[])
        .await
        .assert_status(303);
}

/// Rework: the disposition creates a planned work order of the item for the
/// NCR quantity and tells the planner; when that work order is cancelled the
/// NCR reopens with a task; when a second one completes, QC gets the task to
/// close (the NCR stays `dispositioned`) and can close it.
#[renox::test]
async fn rework_makes_a_work_order_and_follows_it_to_the_end() {
    let s = scene().await;
    let line = s.t.line(&s.plant, "L1").await;
    let planner = s.t.planner("Amal", &s.plant).await;
    let sami = s.t.supervisor("Sami", &line).await;
    let ncr = s.t.ncr(&s.qc, &s.plant, 9, "40").await;
    let line_id = line.id.to_string();
    let form = vec![
        ("disposition", "rework"),
        ("line_id", line_id.as_str()),
        ("bom_id", "3"),
    ];

    s.t.ncr_act(&s.qc, &ncr, "disposition", &form)
        .await
        .assert_status(303);

    let first = s.t.ncr_now(ncr.id).await;
    assert_eq!(first.status, NcrStatus::Dispositioned);
    let order =
        s.t.reload(first.rework_work_order_id.expect("a work order"))
            .await;
    assert_eq!(order.status, WorkOrderStatus::Planned);
    assert_eq!(order.quantity_milli, 40_000);
    assert_eq!(order.line_id, Some(line.id));
    assert_eq!(order.item_id, 11);
    assert_eq!(order.created_by, Some(s.qc.id));
    assert!(
        !s.t.notices.titles_for(&planner).is_empty(),
        "the planner is told"
    );
    let early = s.t.ncr_act(&s.qc, &ncr, "close", &[]).await;
    early.assert_status(422);
    let blocked =
        s.t.ncr_act(&s.qc, &ncr, "reopen", &[("reason", "no")])
            .await;
    blocked.assert_status(422);
    assert!(blocked.text().contains("Cancel the rework work order"));

    s.t.act(&planner, &order, "cancel", &[("reason", "wrong plan")])
        .await
        .assert_status(303);

    let reopened = s.t.ncr_now(ncr.id).await;
    assert_eq!(reopened.status, NcrStatus::Open);
    assert_eq!(reopened.rework_work_order_id, None);
    assert!(
        s.t.tickets
            .all()
            .iter()
            .any(|o| is_redisposition_task(o, &ncr))
    );

    s.t.ncr_act(&s.qc, &ncr, "disposition", &form)
        .await
        .assert_status(303);
    let second = s.t.ncr_now(ncr.id).await;
    let order = s.t.reload(second.rework_work_order_id.unwrap()).await;
    s.t.act(&planner, &order, "release", &[])
        .await
        .assert_status(303);
    s.t.act(&sami, &order, "start", &[])
        .await
        .assert_status(303);
    s.t.act(&sami, &order, "report", &[("good", "40")])
        .await
        .assert_status(303);
    s.t.act(&sami, &order, "complete", &[])
        .await
        .assert_status(303);

    let waiting = s.t.ncr_now(ncr.id).await;
    assert_eq!(
        waiting.status,
        NcrStatus::Dispositioned,
        "QC closes it, not the system"
    );
    assert!(waiting.action_done_at.is_some());
    assert!(s.t.tickets.all().iter().any(|o| is_close_task(o, &ncr)));
    s.t.ncr_act(&s.qc, &ncr, "close", &[])
        .await
        .assert_status(303);
    assert_eq!(s.t.ncr_now(ncr.id).await.status, NcrStatus::Closed);
}

/// A holder of `ncrs.raise` only cannot disposition; an NCR is dispositioned
/// once; a person can see an NCR's page with the actions disabled and the
/// reason in words.
#[renox::test]
async fn only_ncr_dispositioners_decide_and_only_once() {
    let s = scene().await;
    let clerk = s.t.clerk("Clara", &s.plant).await;
    let ncr = s.t.ncr(&s.qc, &s.plant, 9, "10").await;

    let forbidden = s.t.ncr_act(&clerk, &ncr, "disposition", &credit()).await;
    let bad =
        s.t.ncr_act(&s.qc, &ncr, "disposition", &[("disposition", "burn")])
            .await;
    s.t.ncr_act(&s.qc, &ncr, "disposition", &credit())
        .await
        .assert_status(303);
    let twice = s.t.ncr_act(&s.qc, &ncr, "disposition", &credit()).await;
    let page =
        s.t.browse(&s.qc, &format!("/manufacturing/ncrs/{}", ncr.id))
            .await;

    forbidden.assert_status(403);
    bad.assert_status(422);
    twice.assert_status(422);
    assert!(twice.text().contains("Only an open NCR"));
    page.assert_ok();
    assert!(
        page.text()
            .contains("The action of the disposition is not done yet")
    );
    assert_eq!(
        s.t.lots.made().len(),
        1,
        "one Return to supplier, however often it is asked"
    );
    assert_eq!(s.t.ncr_now(ncr.id).await.cause, Cause::Supplier);
}
