//! The forward trace of a recall (wl-inventory §4 *Recall* `trace`, §6
//! `LotTrace`, `TracePartyResolver`, issue #234): every point of "Done when"
//! through the route that opens the recall, the route that traces it again,
//! the real receipt that posts a pending issue, and the kernel's `LotTrace`.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_inventory::movement::MovementType;
use wl_inventory::{
    DraftLine, Item, Location, LotStatus, PendingIssue, PendingStatus, Recall, RecallLine, recall,
};
use wl_kernel::inventory::{TraceMark, lot_trace};
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

/// A moment on `day` of October 2026 at `hour` o'clock UTC (Dubai the same
/// day for the hours used here).
fn at(day: u32, hour: u32) -> DateTime {
    NaiveDate::from_ymd_opt(2026, 10, day)
        .unwrap()
        .and_hms_opt(hour, 0, 0)
        .unwrap()
        .and_utc()
}

fn visit(n: i64) -> Subject {
    Subject::new("clinics.visit", n)
}

fn order(n: i64) -> Subject {
    Subject::new("sales.order", n)
}

/// Opens a recall on lot `number` of `item` through `POST /recalls`.
async fn open_recall(t: &Booted, item: &Item, number: &str) -> Recall {
    let manager = t.recaller("Rita").await;
    let item_id = item.id.to_string();
    t.recall_as(
        &manager,
        &[
            ("source", "supplier"),
            ("reason", "Contamination found by the supplier"),
            ("items", item_id.as_str()),
            ("lot_numbers", number),
        ],
    )
    .await
    .assert_ok();
    t.all_recalls().await.pop().expect("a recall")
}

/// The lines of `recall` as (subject, party, kind).
async fn listed(t: &Booted, recall: &Recall) -> Vec<(String, Option<i64>, String)> {
    let mut lines: Vec<_> = t
        .recall_lines(recall)
        .await
        .iter()
        .map(|line| {
            (
                line.used_by().map(|s| s.to_string()).unwrap_or_default(),
                line.party_id,
                line.kind.as_str().to_owned(),
            )
        })
        .collect();
    lines.sort();
    lines
}

/// Done when 1: L9 issued to visits V1 (patient P1) and V2 (P2) and consumed
/// in batch B producing L20 sold to customer C: lines for V1/P1, V2/P2 and C,
/// and L20 is in scope and held. The kernel's `LotTrace` gives the same
/// parties and where the lots are held.
#[renox::test]
async fn the_trace_lists_visits_and_the_customer_of_a_lot_made_from_the_recalled_one() {
    let t = boot().await;
    let serum = t.tracked_item("SERUM").await;
    let kit = t.tracked_item("KIT").await;
    let l9 = t.lot(&serum, "L9", LotStatus::Released).await;
    let l20 = t.lot(&kit, "L20", LotStatus::Released).await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&serum, &branch, Some(&l9), 20_000, 1_000).await;
    let issue = MovementType::Issue;
    t.movement(
        issue,
        &serum,
        Some(&l9),
        Some(&branch),
        None,
        2_000,
        &visit(1),
        Some(&visit(1)),
        at(2, 8),
        false,
    )
    .await;
    t.movement(
        issue,
        &serum,
        Some(&l9),
        Some(&branch),
        None,
        3_000,
        &visit(2),
        Some(&visit(2)),
        at(3, 8),
        false,
    )
    .await;
    // Batch B consumes 10 of L9 and produces 5 of L20; 4 of them are sold.
    let batch = Subject::new("manufacturing.batch", 1);
    t.movement(
        MovementType::Consume,
        &serum,
        Some(&l9),
        Some(&branch),
        None,
        10_000,
        &batch,
        None,
        at(4, 8),
        false,
    )
    .await;
    t.movement(
        MovementType::Produce,
        &kit,
        Some(&l20),
        None,
        Some(&branch),
        5_000,
        &batch,
        None,
        at(4, 9),
        false,
    )
    .await;
    t.movement(
        issue,
        &kit,
        Some(&l20),
        Some(&branch),
        None,
        4_000,
        &order(5),
        Some(&order(5)),
        at(6, 8),
        false,
    )
    .await;
    t.trace_parties.set(&visit(1), PartyId(11));
    t.trace_parties.set(&visit(2), PartyId(12));
    t.trace_parties.set(&order(5), PartyId(33));

    let opened = open_recall(&t, &serum, "L9").await;

    let after = t.recall_now(&opened).await;
    assert_eq!(after.status, wl_inventory::RecallStatus::Traced);
    assert_eq!(after.trace_runs, 1);
    assert_eq!(
        listed(&t, &opened).await,
        vec![
            (
                "clinics.visit:1".to_owned(),
                Some(11),
                "confirmed".to_owned()
            ),
            (
                "clinics.visit:2".to_owned(),
                Some(12),
                "confirmed".to_owned()
            ),
            ("sales.order:5".to_owned(), Some(33), "confirmed".to_owned()),
        ]
    );
    // The batch itself is no line, L20 is in scope and held.
    let scope = recall::lots_in_scope(t.db(), opened.id).await.unwrap();
    assert_eq!(scope, vec![l9.id, l20.id]);
    assert_eq!(t.lot_now(&l20).await.status, LotStatus::OnHold);
    let holds = t.all_holds().await;
    assert_eq!(holds.len(), 1);
    let held: Vec<(Option<i64>, i64)> = t
        .hold_lines(&holds[0])
        .await
        .iter()
        .map(|line| (line.lot_id, line.quantity_milli))
        .collect();
    assert_eq!(held, vec![(Some(l9.id), 5_000), (Some(l20.id), 1_000)]);

    // The event carries the lots and the lines.
    let events = t.traced.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].recall, opened.subject());
    assert_eq!(events[0].lots.len(), 2);
    assert_eq!(events[0].lines.len(), 3);

    // The kernel's view, for the clinics vertical.
    let trace = lot_trace(t.state());
    let holders = trace.holders(t.state(), &opened.subject()).await.unwrap();
    let mut parties: Vec<(Option<Subject>, Option<i64>, String)> = holders
        .iter()
        .map(|h| (h.subject.clone(), h.party.map(|p| p.0), h.notice.clone()))
        .collect();
    parties.sort();
    assert_eq!(
        parties,
        vec![
            (Some(visit(1)), Some(11), "pending".to_owned()),
            (Some(visit(2)), Some(12), "pending".to_owned()),
            (Some(order(5)), Some(33), "pending".to_owned()),
        ]
    );
    let places = trace.places(t.state(), &opened.subject()).await.unwrap();
    let mut where_: Vec<(Subject, Subject, i64)> = places
        .iter()
        .map(|p| (p.lot.clone(), p.location.clone(), p.quantity_milli))
        .collect();
    where_.sort();
    assert_eq!(
        where_,
        vec![
            (
                Subject::new("inventory.lot", l9.id),
                branch.subject(),
                5_000
            ),
            (
                Subject::new("inventory.lot", l20.id),
                branch.subject(),
                1_000
            ),
        ]
    );
}

/// A subject without a party from the vertical is listed with the party
/// unknown, and a dispatch is listed with its delivery status.
#[renox::test]
async fn a_subject_without_a_resolver_is_unknown_and_a_dispatch_shows_its_delivery() {
    let t = boot().await;
    let serum = t.tracked_item("SERUM").await;
    let l9 = t.lot(&serum, "L9", LotStatus::Released).await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&serum, &branch, Some(&l9), 20_000, 1_000).await;
    t.movement(
        MovementType::Issue,
        &serum,
        Some(&l9),
        Some(&branch),
        None,
        2_000,
        &Subject::new("clinics.job", 4),
        Some(&Subject::new("clinics.job", 4)),
        at(2, 8),
        false,
    )
    .await;
    let dispatch = Subject::new("delivery.dispatch", 8);
    t.movement(
        MovementType::Dispatch,
        &serum,
        Some(&l9),
        Some(&branch),
        None,
        1_000,
        &dispatch,
        None,
        at(3, 8),
        false,
    )
    .await;
    t.trace_parties.set(&dispatch, PartyId(44));
    t.trace_parties.delivery(&dispatch, "out_for_delivery");

    let opened = open_recall(&t, &serum, "L9").await;

    let lines = t.recall_lines(&opened).await;
    assert_eq!(lines.len(), 2);
    let job = lines
        .iter()
        .find(|l| l.kind.as_str() == "confirmed")
        .unwrap();
    assert_eq!((job.party_id, job.subject_id), (None, Some(4)));
    let sent = lines
        .iter()
        .find(|l| l.kind.as_str() == "dispatch")
        .unwrap();
    assert_eq!(sent.party_id, Some(44));
    assert_eq!(sent.delivery_status.as_deref(), Some("out_for_delivery"));
}

/// Posts a receipt of lot `lot` (expiring in December 2027) of `item` at
/// `branch`, through the real receipt route.
async fn receive_lot(t: &Booted, branch: &Location, item: &Item, lot: &str) {
    let (order, po_line) = t.order(branch, item, 100_000, 500).await;
    let line = DraftLine::on(po_line.id, 10_000, 10_000)
        .lot(lot, NaiveDate::from_ymd_opt(2027, 12, 28).unwrap());
    let receipt = t.receipt(&order, branch, vec![line]).await;
    let keeper = t.storekeeper("Amal").await;
    t.post_as(&keeper, &receipt).await.assert_ok();
}

/// Done when 2: a pending issue of L9's item at a location holding L9, posted
/// later with a lot assigned on arrival, is listed "lot uncertain".
#[renox::test]
async fn a_pending_issue_posted_with_a_lot_assigned_on_arrival_is_uncertain() {
    let t = boot().await;
    let serum = t.tracked_item("SERUM").await;
    let branch = t.location("BRANCH-1", None).await;
    // Nothing on hand: the visit's consumption becomes a pending issue.
    t.consume(&serum, &branch, 3_000, &visit(3)).await;
    assert_eq!(t.open_pending().await.len(), 1);
    t.trace_parties.set(&visit(3), PartyId(13));
    receive_lot(&t, &branch, &serum, "L9").await;
    let posted = PendingIssue::query().first(t.db()).await.unwrap().unwrap();
    assert_eq!(posted.status, PendingStatus::Posted);
    assert!(posted.assigned_on_arrival);

    let opened = open_recall(&t, &serum, "L9").await;

    let lines = t.recall_lines(&opened).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].kind.as_str(), "uncertain");
    assert_eq!(lines[0].used_by(), Some(visit(3)));
    assert_eq!(lines[0].party_id, Some(13));
    assert_eq!(lines[0].holder().mark, TraceMark::Uncertain);
}

/// Done when 3: 15 walk-in POS sales of L20 at Outlet 1 on 2026-10-10 give
/// one line "15 sales, Outlet 1, 2026-10-10".
#[renox::test]
async fn walk_in_sales_are_one_line_per_outlet_and_day() {
    let t = boot().await;
    let kit = t.tracked_item("KIT").await;
    let l20 = t.lot(&kit, "L20", LotStatus::Released).await;
    let outlet = t.named_location("OUTLET-1", "Outlet 1").await;
    t.movement(
        MovementType::Receive,
        &kit,
        Some(&l20),
        None,
        Some(&outlet),
        30_000,
        &Subject::new("inventory.goods_receipt", 1),
        None,
        at(1, 8),
        false,
    )
    .await;
    for n in 1..=15 {
        let sale = Subject::new("sales.pos_sale", n);
        t.movement(
            MovementType::Consume,
            &kit,
            Some(&l20),
            Some(&outlet),
            None,
            1_000,
            &sale,
            Some(&sale),
            at(10, 8),
            false,
        )
        .await;
    }
    t.trace_parties.walk_in("sales.pos_sale");

    let opened = open_recall(&t, &kit, "L20").await;

    let lines = t.recall_lines(&opened).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = &lines[0];
    assert_eq!(line.kind.as_str(), "walk_in");
    assert_eq!(line.sales_count, 15);
    assert_eq!(line.location_id, Some(outlet.id));
    assert_eq!(line.occurred_on, NaiveDate::from_ymd_opt(2026, 10, 10));
    assert_eq!(line.party_id, None);
    assert_eq!(line.subject_id, None);
    // The back-office text of the line, as the recall report prints it.
    let report = wl_inventory::recall_close::report(t.db(), &opened)
        .await
        .unwrap();
    assert!(
        report.contains("15 sales, Outlet 1, 2026-10-10"),
        "{report}"
    );
}

/// Done when 4: running `trace` again creates no duplicates, adds only what is
/// new and leaves the notice status of the lines it knows.
#[renox::test]
async fn tracing_again_adds_only_new_lines_and_keeps_the_notice_status() {
    let t = boot().await;
    let serum = t.tracked_item("SERUM").await;
    let l9 = t.lot(&serum, "L9", LotStatus::Released).await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&serum, &branch, Some(&l9), 20_000, 1_000).await;
    t.movement(
        MovementType::Issue,
        &serum,
        Some(&l9),
        Some(&branch),
        None,
        2_000,
        &visit(1),
        Some(&visit(1)),
        at(2, 8),
        false,
    )
    .await;
    t.movement(
        MovementType::Issue,
        &serum,
        Some(&l9),
        Some(&branch),
        None,
        3_000,
        &visit(2),
        Some(&visit(2)),
        at(3, 8),
        false,
    )
    .await;
    t.trace_parties.set(&visit(1), PartyId(11));
    t.trace_parties.set(&visit(2), PartyId(12));
    let opened = open_recall(&t, &serum, "L9").await;
    let manager = t.recaller("Rita").await;
    let first = t.recall_lines(&opened).await;
    assert_eq!(first.len(), 2);
    t.waive_as(&manager, &first[0], "Patient informed in person")
        .await
        .assert_ok();

    // The same movements again: nothing new.
    t.trace_as(&manager, &opened).await.assert_ok();
    let again = t.recall_lines(&opened).await;
    assert_eq!(
        again.iter().map(|l| l.id).collect::<Vec<_>>(),
        first.iter().map(|l| l.id).collect::<Vec<_>>()
    );
    assert_eq!(again[0].notice_status.as_str(), "waived");
    assert_eq!(again[1].notice_status.as_str(), "pending");
    assert_eq!(t.recall_now(&opened).await.trace_runs, 2);

    // A visit that used the lot since is the only line added.
    t.movement(
        MovementType::Issue,
        &serum,
        Some(&l9),
        Some(&branch),
        None,
        1_000,
        &visit(9),
        Some(&visit(9)),
        at(8, 8),
        false,
    )
    .await;
    t.trace_as(&manager, &opened).await.assert_ok();
    let third: Vec<RecallLine> = t.recall_lines(&opened).await;
    assert_eq!(third.len(), 3);
    assert_eq!(third[0].notice_status.as_str(), "waived");
    assert_eq!(third[2].used_by(), Some(visit(9)));
    assert_eq!(t.traced.all().len(), 3);

    // Only a holder of `recalls.manage` traces again.
    let storekeeper = t.storekeeper("Amal").await;
    t.trace_as(&storekeeper, &opened).await.assert_forbidden();
    assert_eq!(t.recall_lines(&opened).await.len(), 3);
}

/// Done when 5: lot T7 on hand at Outlet 1 from 2026-10-03 until its balance
/// there reached zero on 2026-10-06, and orders consuming T7's item
/// theoretically at Outlet 1 on 2026-10-02, 2026-10-04 and 2026-10-07: only
/// the 2026-10-04 order is listed, marked *possible*, even when its posting
/// used another lot.
#[renox::test]
async fn theoretical_consumption_is_possible_only_while_the_lot_was_on_hand() {
    let t = boot().await;
    let item = t.tracked_item("CHICKEN").await;
    let t7 = t.lot(&item, "T7", LotStatus::Released).await;
    let t8 = t.lot(&item, "T8", LotStatus::Released).await;
    let outlet = t.named_location("OUTLET-1", "Outlet 1").await;
    let receipt = Subject::new("inventory.goods_receipt", 1);
    t.movement(
        MovementType::Receive,
        &item,
        Some(&t8),
        None,
        Some(&outlet),
        50_000,
        &receipt,
        None,
        at(1, 8),
        false,
    )
    .await;
    t.movement(
        MovementType::Receive,
        &item,
        Some(&t7),
        None,
        Some(&outlet),
        10_000,
        &receipt,
        None,
        at(3, 8),
        false,
    )
    .await;
    // The rest of T7 is thrown away on the 6th: its balance there is zero.
    t.movement(
        MovementType::Wastage,
        &item,
        Some(&t7),
        Some(&outlet),
        None,
        10_000,
        &Subject::new("inventory.lot", t7.id),
        None,
        at(6, 8),
        false,
    )
    .await;
    // Orders consume the item without a scan; the postings used T8.
    for (n, day) in [(1, 2), (2, 4), (3, 7)] {
        t.movement(
            MovementType::Consume,
            &item,
            Some(&t8),
            Some(&outlet),
            None,
            500,
            &order(n),
            Some(&order(n)),
            at(day, 9),
            true,
        )
        .await;
    }
    t.trace_parties.set(&order(2), PartyId(22));

    let opened = open_recall(&t, &item, "T7").await;

    let lines = t.recall_lines(&opened).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].kind.as_str(), "possible");
    assert_eq!(lines[0].used_by(), Some(order(2)));
    assert_eq!(lines[0].party_id, Some(22));
    assert_eq!(lines[0].holder().mark, TraceMark::Possible);
    // T7 has no balance anywhere: in scope, no hold line.
    assert_eq!(
        recall::lots_in_scope(t.db(), opened.id).await.unwrap(),
        vec![t7.id]
    );
}

/// The trace never changes a movement.
#[renox::test]
async fn the_trace_changes_no_movement() {
    let t = boot().await;
    let serum = t.tracked_item("SERUM").await;
    let l9 = t.lot(&serum, "L9", LotStatus::Released).await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&serum, &branch, Some(&l9), 20_000, 1_000).await;
    t.movement(
        MovementType::Issue,
        &serum,
        Some(&l9),
        Some(&branch),
        None,
        2_000,
        &visit(1),
        Some(&visit(1)),
        at(2, 8),
        false,
    )
    .await;
    let before: Vec<(i64, i64, Option<i64>)> = t
        .movements_of(&serum)
        .await
        .iter()
        .map(|m| (m.id, m.quantity_milli, m.lot_id))
        .collect();

    let opened = open_recall(&t, &serum, "L9").await;
    let manager = t.recaller("Rita").await;
    t.trace_as(&manager, &opened).await.assert_ok();

    let after: Vec<(i64, i64, Option<i64>)> = t
        .movements_of(&serum)
        .await
        .iter()
        .map(|m| (m.id, m.quantity_milli, m.lot_id))
        .collect();
    assert_eq!(before, after);
}

/// A window ends where the balance there reached zero by a transfer out, and
/// begins at the destination of the `transfer_in`: orders consuming
/// theoretically at Outlet 1 after the transfer are not listed, those at
/// Outlet 2 after it are *possible*.
#[renox::test]
async fn a_transfer_out_ends_the_window_and_the_transfer_in_opens_one() {
    let t = boot().await;
    let item = t.tracked_item("CHICKEN").await;
    let t7 = t.lot(&item, "T7", LotStatus::Released).await;
    let outlet_1 = t.named_location("OUTLET-1", "Outlet 1").await;
    let outlet_2 = t.named_location("OUTLET-2", "Outlet 2").await;
    let batch = Subject::new("inventory.transfer", 1);
    t.movement(
        MovementType::Receive,
        &item,
        Some(&t7),
        None,
        Some(&outlet_1),
        10_000,
        &Subject::new("inventory.goods_receipt", 1),
        None,
        at(1, 8),
        false,
    )
    .await;
    // All of it moves to Outlet 2 on the 6th.
    t.movement(
        MovementType::TransferOut,
        &item,
        Some(&t7),
        Some(&outlet_1),
        None,
        10_000,
        &batch,
        None,
        at(6, 8),
        false,
    )
    .await;
    t.movement(
        MovementType::TransferIn,
        &item,
        Some(&t7),
        None,
        Some(&outlet_2),
        10_000,
        &batch,
        None,
        at(6, 9),
        false,
    )
    .await;
    // (order, outlet, day): 1 before the transfer at Outlet 1, 2 after it
    // there, 3 before it at Outlet 2 (nothing there yet), 4 after it there.
    for (n, outlet, day) in [(1, &outlet_1, 3), (2, &outlet_1, 7), (3, &outlet_2, 3), (4, &outlet_2, 8)] {
        t.movement(
            MovementType::Consume,
            &item,
            Some(&t7),
            Some(outlet),
            None,
            500,
            &order(n),
            Some(&order(n)),
            at(day, 9),
            true,
        )
        .await;
    }

    let opened = open_recall(&t, &item, "T7").await;

    let mut found: Vec<_> = t
        .recall_lines(&opened)
        .await
        .iter()
        .map(|line| (line.used_by().unwrap().to_string(), line.kind.as_str().to_owned()))
        .collect();
    found.sort();
    assert_eq!(
        found,
        vec![
            ("sales.order:1".to_owned(), "possible".to_owned()),
            ("sales.order:4".to_owned(), "possible".to_owned()),
        ]
    );
}

/// A window also ends where the recall held the lot: theoretical
/// consumption dated after the recall opened is not listed.
#[renox::test]
async fn a_window_ends_when_the_recall_holds_the_lot() {
    let t = boot().await;
    let item = t.tracked_item("CHICKEN").await;
    let t7 = t.lot(&item, "T7", LotStatus::Released).await;
    let outlet = t.named_location("OUTLET-1", "Outlet 1").await;
    let now = renox::db::now();
    t.movement(
        MovementType::Receive,
        &item,
        Some(&t7),
        None,
        Some(&outlet),
        10_000,
        &Subject::new("inventory.goods_receipt", 1),
        None,
        now - renox::chrono::Duration::days(3),
        false,
    )
    .await;
    for (n, offset) in [(1, -1), (2, 1)] {
        t.movement(
            MovementType::Consume,
            &item,
            Some(&t7),
            Some(&outlet),
            None,
            500,
            &order(n),
            Some(&order(n)),
            now + renox::chrono::Duration::days(offset),
            true,
        )
        .await;
    }

    let opened = open_recall(&t, &item, "T7").await;

    let lines = t.recall_lines(&opened).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].used_by(), Some(order(1)));
    assert_eq!(lines[0].kind.as_str(), "possible");
}

/// Lots made from each other in a circle (L9 into L20 in one batch, L20 back
/// into L9 in another) are each traced once: the trace ends, the scope and
/// the hold name each lot once and a visit using L20 is one line.
#[renox::test]
async fn a_genealogy_cycle_traces_each_lot_once() {
    let t = boot().await;
    let serum = t.tracked_item("SERUM").await;
    let kit = t.tracked_item("KIT").await;
    let l9 = t.lot(&serum, "L9", LotStatus::Released).await;
    let l20 = t.lot(&kit, "L20", LotStatus::Released).await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&serum, &branch, Some(&l9), 20_000, 1_000).await;
    let one = Subject::new("manufacturing.batch", 1);
    let two = Subject::new("manufacturing.batch", 2);
    let steps = [
        (MovementType::Consume, &serum, &l9, Some(&branch), None, &one, 4, 2_000),
        (MovementType::Produce, &kit, &l20, None, Some(&branch), &one, 5, 6_000),
        (MovementType::Consume, &kit, &l20, Some(&branch), None, &two, 6, 2_000),
        (MovementType::Produce, &serum, &l9, None, Some(&branch), &two, 7, 1_000),
    ];
    for (kind, item, lot, from, to, source, day, quantity) in steps {
        t.movement(kind, item, Some(lot), from, to, quantity, source, None, at(day, 8), false)
            .await;
    }
    t.movement(
        MovementType::Issue,
        &kit,
        Some(&l20),
        Some(&branch),
        None,
        1_000,
        &visit(1),
        Some(&visit(1)),
        at(5, 12),
        false,
    )
    .await;
    t.trace_parties.set(&visit(1), PartyId(11));

    let opened = open_recall(&t, &serum, "L9").await;

    let mut scope = recall::lots_in_scope(t.db(), opened.id).await.unwrap();
    scope.sort_unstable();
    assert_eq!(scope, vec![l9.id, l20.id]);
    let holds = t.all_holds().await;
    assert_eq!(holds.len(), 1);
    let mut held: Vec<i64> = t
        .hold_lines(&holds[0])
        .await
        .iter()
        .filter_map(|line| line.lot_id)
        .collect();
    held.sort_unstable();
    held.dedup();
    assert_eq!(held, vec![l9.id, l20.id]);
    assert_eq!(
        listed(&t, &opened).await,
        vec![(
            "clinics.visit:1".to_owned(),
            Some(11),
            "confirmed".to_owned()
        )]
    );
    // Tracing again changes nothing.
    t.trace_as(&t.recaller("Rita").await, &opened).await.assert_ok();
    assert_eq!(t.recall_lines(&opened).await.len(), 1);
    assert_eq!(
        recall::lots_in_scope(t.db(), opened.id).await.unwrap().len(),
        2
    );
}
