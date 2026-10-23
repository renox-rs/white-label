//! The recall of an item not tracked by lot (wl-inventory §4 *Recall* `open`,
//! the trace table's last row, "Items not tracked by lot", issue #236): every
//! point of "Done when" through the route that opens the recall, the goods
//! receipts and movements a vertical would post, and the notices.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_inventory::movement::MovementType;
use wl_inventory::recall_notice::notice_key;
use wl_inventory::{Item, Location, NoticeStatus, Recall, RecallLine, RecallStatus};
use wl_kernel::inventory::{TraceMark, lot_trace};
use wl_kernel::{PartyId, Subject};

use support::{Booted, SUPPLIER, boot};

fn day(month: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, d).unwrap()
}

fn at(month: u32, d: u32) -> DateTime {
    day(month, d).and_hms_opt(10, 0, 0).unwrap().and_utc()
}

fn job(n: i64) -> Subject {
    Subject::new("auto.job_card", n)
}

/// Issues `quantity_milli` of `item` from `store` to `used` on `on`.
async fn issued(t: &Booted, item: &Item, store: &Location, used: &Subject, on: DateTime, q: i64) {
    t.movement(
        MovementType::Issue,
        item,
        None,
        Some(store),
        None,
        q,
        used,
        Some(used),
        on,
        false,
    )
    .await;
}

/// Part P received from supplier S on 2026-08-05 and 2026-09-01 (10 each),
/// after 5 from another supplier on 2026-07-01, with issues to job cards 1
/// (2026-07-20, before any recalled receipt), 2 (2026-08-10), 3 (2026-09-10)
/// and 4 (2026-09-20, after the received range). 19 are on hand.
async fn parts(t: &Booted) -> (Item, Location) {
    let part = t.item("PART-P").await;
    let store = t.named_location("STORE-1", "Store 1").await;
    t.received_on(&part, &store, PartyId(99), 5_000, day(7, 1))
        .await;
    t.received_on(&part, &store, SUPPLIER, 10_000, day(8, 5))
        .await;
    t.received_on(&part, &store, SUPPLIER, 10_000, day(9, 1))
        .await;
    issued(t, &part, &store, &job(1), at(7, 20), 1_000).await;
    issued(t, &part, &store, &job(2), at(8, 10), 3_000).await;
    issued(t, &part, &store, &job(3), at(9, 10), 1_000).await;
    issued(t, &part, &store, &job(4), at(9, 20), 1_000).await;
    for n in 1..=4 {
        t.trace_parties.set(&job(n), PartyId(40 + n));
    }
    (part, store)
}

/// Opens the recall of `part` from supplier S, received 2026-08-01 to
/// 2026-08-31, with the trace end date `end` when given.
async fn open(t: &Booted, part: &Item, end: Option<&str>) -> Recall {
    let manager = t.recaller("Rita").await;
    let item_id = part.id.to_string();
    let supplier = SUPPLIER.0.to_string();
    let mut form = vec![
        ("source", "supplier"),
        ("reason", "Brake pads below the friction standard"),
        ("supplier", supplier.as_str()),
        ("items", item_id.as_str()),
        ("received_from", "2026-08-01"),
        ("received_to", "2026-08-31"),
    ];
    if let Some(end) = end {
        form.push(("trace_end", end));
    }
    t.recall_as(&manager, &form).await.assert_ok();
    t.all_recalls().await.pop().expect("a recall")
}

/// The lines of `recall` as (subject, party, kind, mark).
async fn listed(t: &Booted, recall: &Recall) -> Vec<(String, Option<i64>, String, TraceMark)> {
    let mut lines: Vec<_> = t
        .recall_lines(recall)
        .await
        .iter()
        .map(|line| {
            (
                line.used_by().map(|s| s.to_string()).unwrap_or_default(),
                line.party_id,
                line.kind.as_str().to_owned(),
                line.mark(),
            )
        })
        .collect();
    lines.sort_by(|a, b| a.0.cmp(&b.0));
    lines
}

/// Done when 1: P received from S on 2026-08-05 and 2026-09-01 and a scope
/// S / P / received 2026-08-01 to 2026-08-31 with no trace end date: all
/// on-hand P is held and every issue of P from 2026-08-05 to today is a
/// *possible* line, also those after 2026-08-31; an issue before the first
/// receipt is not listed.
#[renox::test]
async fn stk_17_all_on_hand_stock_is_held_and_every_later_issue_is_possible() {
    let t = boot().await;
    let (part, store) = parts(&t).await;
    let other = t.item("PART-Q").await;
    t.stock_in(&other, &store, None, 7_000, 500).await;

    let recall = open(&t, &part, None).await;

    assert_eq!(t.recall_now(&recall).await.status, RecallStatus::Traced);
    // 5 + 10 + 10 - 1 - 3 - 1 - 1 = 19 on hand, all of it held.
    assert_eq!(
        wl_inventory::balance::balance(t.db(), part.id, store.id, None)
            .await
            .unwrap()
            .held_milli,
        19_000
    );
    assert_eq!(t.available(&part, &store, None).await, 0);
    let holds = t.all_holds().await;
    assert_eq!(holds.len(), 1);
    assert_eq!(holds[0].reason, "recall");
    let held: Vec<(Option<i64>, i64, i64)> = t
        .hold_lines(&holds[0])
        .await
        .iter()
        .map(|line| (line.lot_id, line.location_id, line.quantity_milli))
        .collect();
    assert_eq!(held, vec![(None, store.id, 19_000)]);
    // Another item is not held.
    assert_eq!(t.available(&other, &store, None).await, 7_000);

    let possible = TraceMark::Possible;
    assert_eq!(
        listed(&t, &recall).await,
        vec![
            (
                "auto.job_card:2".to_owned(),
                Some(42),
                "possible".to_owned(),
                possible
            ),
            (
                "auto.job_card:3".to_owned(),
                Some(43),
                "possible".to_owned(),
                possible
            ),
            (
                "auto.job_card:4".to_owned(),
                Some(44),
                "possible".to_owned(),
                possible
            ),
        ]
    );
    let lines = t.recall_lines(&recall).await;
    assert!(lines.iter().all(|line| line.lot_id.is_none()));
    assert!(lines.iter().all(|line| line.item_id == Some(part.id)));
    // The scope kept the first receipt.
    let scope = wl_inventory::RecallScope::query()
        .where_(wl_inventory::RecallScope::RECALL_ID.eq(recall.id))
        .first(t.db())
        .await
        .unwrap()
        .expect("a scope");
    assert_eq!(scope.first_receipt_on, Some(day(8, 5)));
    assert_eq!(scope.supplier_party_id, Some(SUPPLIER.0));
    assert_eq!(scope.trace_end, None);

    // The kernel's view marks them possible too.
    let holders = lot_trace(t.state())
        .holders(t.state(), &recall.subject())
        .await
        .unwrap();
    assert_eq!(holders.len(), 3);
    assert!(holders.iter().all(|h| h.mark == TraceMark::Possible));
    assert!(holders.iter().all(|h| h.lot.is_none()));
}

/// Done when 2: with the trace end date 2026-09-15, issues after it are not
/// listed.
#[renox::test]
async fn stk_17_issues_after_the_trace_end_date_are_not_listed() {
    let t = boot().await;
    let (part, _store) = parts(&t).await;

    let recall = open(&t, &part, Some("2026-09-15")).await;

    let subjects: Vec<String> = listed(&t, &recall)
        .await
        .into_iter()
        .map(|line| line.0)
        .collect();
    assert_eq!(subjects, vec!["auto.job_card:2", "auto.job_card:3"]);
}

/// Done when 3: *possible* lines default to the softer template; a notice for
/// staff keeps its own.
#[renox::test]
async fn stk_17_possible_lines_default_to_the_softer_template() {
    let t = boot().await;
    let (part, _store) = parts(&t).await;
    let recall = open(&t, &part, None).await;
    let manager = t.recaller("Rita").await;
    let lines = t.recall_lines(&recall).await;
    let all: Vec<&RecallLine> = lines.iter().collect();

    t.notices_as(&manager, &recall, "customer", &all)
        .await
        .assert_ok();

    let now = t.recall_lines(&recall).await;
    assert!(now.iter().all(|l| l.notice_status == NoticeStatus::Sent));
    assert!(now.iter().all(|l| l.template.as_deref() == Some("possible")));
    let sent: Vec<_> = t
        .notices
        .all()
        .into_iter()
        .filter(|request| request.type_key == notice_key("possible"))
        .collect();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0].data["item"], "PART-P");
    assert_eq!(sent[0].data["lot"], "");
    assert!(
        wl_inventory::recall_notice::notice_types()
            .iter()
            .any(|def| def.key == notice_key("possible"))
    );
}

/// A line of a staff notice keeps the template chosen.
#[renox::test]
async fn stk_17_a_notice_for_staff_keeps_its_template() {
    let t = boot().await;
    let (part, _store) = parts(&t).await;
    let recall = open(&t, &part, None).await;
    let manager = t.recaller("Rita").await;
    let lines = t.recall_lines(&recall).await;

    t.notices_as(&manager, &recall, "internal", &[&lines[0]])
        .await
        .assert_ok();

    let now = RecallLine::find_or_404(t.db(), lines[0].id).await.unwrap();
    assert_eq!(now.template.as_deref(), Some("internal"));
}

/// Edge cases: only receipts from the scope's supplier start the window; no
/// receipt in the range refuses the recall; an item tracked by lot cannot be
/// recalled by dates.
#[renox::test]
async fn stk_17_the_supplier_filters_the_receipts_and_no_receipt_refuses_the_recall() {
    let t = boot().await;
    let (part, _store) = parts(&t).await;
    let manager = t.recaller("Rita").await;
    let item_id = part.id.to_string();

    // Receipts of another supplier in the range do not count.
    let other_supplier = t
        .recall_as(
            &manager,
            &[
                ("source", "supplier"),
                ("reason", "Notice of supplier 99"),
                ("supplier", "99"),
                ("items", item_id.as_str()),
                ("received_from", "2026-08-01"),
                ("received_to", "2026-08-31"),
            ],
        )
        .await;
    other_supplier.assert_status(422);
    // A range with no receipt of S.
    t.recall_as(
        &manager,
        &[
            ("source", "supplier"),
            ("reason", "Wrong dates"),
            ("supplier", "7"),
            ("items", item_id.as_str()),
            ("received_from", "2026-06-01"),
            ("received_to", "2026-06-30"),
        ],
    )
    .await
    .assert_status(422);
    // One date only, and an end before the start.
    t.recall_as(
        &manager,
        &[
            ("source", "supplier"),
            ("reason", "Half a range"),
            ("items", item_id.as_str()),
            ("received_from", "2026-08-01"),
        ],
    )
    .await
    .assert_status(422);
    assert!(t.all_recalls().await.is_empty());
    assert!(t.all_holds().await.is_empty());

    // An item tracked by lot is recalled by lot.
    let tracked = t.tracked_item("OIL").await;
    let tracked_id = tracked.id.to_string();
    t.recall_as(
        &manager,
        &[
            ("source", "supplier"),
            ("reason", "Oil"),
            ("items", tracked_id.as_str()),
            ("received_from", "2026-08-01"),
            ("received_to", "2026-08-31"),
        ],
    )
    .await
    .assert_status(422);
    assert!(t.all_recalls().await.is_empty());
}

/// Edge case: the hold covers the item at every location, in transit too, and
/// goods a job card put back are not listed against it.
#[renox::test]
async fn stk_17_the_hold_covers_every_location_and_goods_put_back_are_not_listed() {
    let t = boot().await;
    let (part, store) = parts(&t).await;
    let branch = t.named_location("STORE-2", "Store 2").await;
    // 4 moved to Store 2 by transfer steps.
    t.movement(
        MovementType::TransferOut,
        &part,
        None,
        Some(&store),
        None,
        4_000,
        &Subject::new("inventory.transfer", 1),
        None,
        at(9, 25),
        false,
    )
    .await;
    t.movement(
        MovementType::TransferIn,
        &part,
        None,
        None,
        Some(&branch),
        4_000,
        &Subject::new("inventory.transfer", 1),
        None,
        at(9, 26),
        false,
    )
    .await;
    // Job card 5 took 2 and put 2 back.
    issued(&t, &part, &store, &job(5), at(9, 27), 2_000).await;
    t.movement(
        MovementType::ReturnIn,
        &part,
        None,
        None,
        Some(&store),
        2_000,
        &job(5),
        Some(&job(5)),
        at(9, 28),
        false,
    )
    .await;
    t.trace_parties.set(&job(5), PartyId(45));

    let recall = open(&t, &part, None).await;

    let holds = t.all_holds().await;
    let mut held: Vec<(i64, i64)> = t
        .hold_lines(&holds[0])
        .await
        .iter()
        .map(|line| (line.location_id, line.quantity_milli))
        .collect();
    held.sort_unstable();
    assert_eq!(held, vec![(store.id, 15_000), (branch.id, 4_000)]);
    let subjects: Vec<String> = listed(&t, &recall)
        .await
        .into_iter()
        .map(|line| line.0)
        .collect();
    assert!(!subjects.contains(&"auto.job_card:5".to_owned()), "{subjects:?}");
}
