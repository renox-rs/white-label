//! The recall screens: the list, the page of one recall with its lots, the
//! hold and the affected lines, the owner's buttons, and the close with what
//! stands in its way (BL-STK-14, BL-STK-15, BL-STK-16).

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_inventory::movement::MovementType;
use wl_inventory::{LotStatus, NoticeStatus, Recall, RecallLine, RecallStatus};
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

fn at(day: u32) -> DateTime {
    NaiveDate::from_ymd_opt(2026, 10, day)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap()
        .and_utc()
}

/// A recall on lot L9 of SERUM at Branch 1 after two visits used it; the
/// patient behind visit `n` is party `10 + n`.
async fn traced_recall(t: &Booted) -> Recall {
    let serum = t.tracked_item("SERUM").await;
    let l9 = t.lot(&serum, "L9", LotStatus::Released).await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&serum, &branch, Some(&l9), 7_000, 1_000).await;
    for n in 1..=2_i64 {
        let used = Subject::new("clinics.visit", n);
        t.movement(
            MovementType::Issue,
            &serum,
            Some(&l9),
            Some(&branch),
            None,
            1_000,
            &used,
            Some(&used),
            at(u32::try_from(n).unwrap() + 1),
            false,
        )
        .await;
        t.trace_parties.set(&used, PartyId(10 + n));
    }
    let manager = t.recaller("Rita").await;
    let item_id = serum.id.to_string();
    t.recall_as(
        &manager,
        &[
            ("source", "supplier"),
            ("reason", "Contamination found by the supplier"),
            ("items", item_id.as_str()),
            ("lot_numbers", "L9"),
        ],
    )
    .await
    .assert_ok();
    t.all_recalls().await.pop().expect("a recall")
}

/// The list and the page show the recall, its lot and where it is, the hold
/// and how far the notices are, and name nobody; a user with neither
/// permission is refused; a viewer has no buttons, the owner has.
#[renox::test]
async fn the_recall_pages_show_how_far_it_is_and_name_nobody() {
    let t = boot().await;
    let recall = traced_recall(&t).await;
    let owner = t.recaller("Rita").await;
    let viewer = t.recall_viewer("Vera").await;
    let nobody = t.user_with("Nora", "stock_clerk").await;
    let number = recall.number.clone().unwrap();

    t.browse(&nobody, "/recalls").await.assert_forbidden();
    t.browse(&nobody, &format!("/recalls/{}", recall.id))
        .await
        .assert_forbidden();
    t.browse(&viewer, "/recalls")
        .await
        .assert_ok()
        .assert_see(&number)
        .assert_see("Contamination found by the supplier")
        .assert_see("L9")
        .assert_see("0 / 2");

    let page = t.browse(&viewer, &format!("/recalls/{}", recall.id)).await;
    page.assert_ok()
        .assert_see(&number)
        .assert_see("Branch 1: 5 kg")
        .assert_see("HLD-")
        .assert_see("Still to tell: 2 line(s)")
        .assert_see("visit #1")
        .assert_dont_see("Close the recall")
        .assert_dont_see("Trace again")
        .assert_dont_see("Why the party need not be told");

    let page = t.browse(&owner, &format!("/recalls/{}", recall.id)).await;
    page.assert_ok()
        .assert_see("Trace again")
        .assert_see("Why the party need not be told")
        .assert_see("Send the notices")
        // With the hold open and two lines to tell, the close is not offered
        // and the page says what stands in its way.
        .assert_see("The stock hold still has 1 line(s) to decide.")
        .assert_see("Still to tell: 2 line(s)")
        .assert_dont_see("Close the recall");
}

/// The owner waives each line with a reason and decides the held stock; then
/// the close is offered, and closing it keeps the report which the page links.
#[renox::test]
async fn the_owner_closes_the_recall_once_nothing_stands_in_its_way() {
    let t = boot().await;
    let recall = traced_recall(&t).await;
    let owner = t.recaller("Rita").await;
    let releaser = t.releaser("Rana").await;
    let second = t.user_with("Sam", "lot_releaser").await;
    let lines = t.recall_lines(&recall).await;

    for line in &lines {
        t.waive_as(&owner, line, "Told by phone").await.assert_ok();
    }
    assert!(
        t.recall_lines(&recall)
            .await
            .iter()
            .all(|line: &RecallLine| line.notice_status == NoticeStatus::Waived)
    );
    // The held stock still waits for its decision: close is refused and not
    // offered.
    t.close_recall_as(&owner, &recall).await.assert_status(422);
    t.browse(&owner, &format!("/recalls/{}", recall.id))
        .await
        .assert_see("The stock hold still has 1 line(s) to decide.")
        .assert_dont_see("Close the recall");

    // Discard the held lot (a second person is not needed below the value).
    let hold = t.all_holds().await.pop().expect("the recall's hold");
    let line = t.hold_lines(&hold).await.remove(0);
    t.decide_as(&releaser, &line, "discard").await.assert_ok();
    let _ = second;
    assert_eq!(t.recall_now(&recall).await.status, RecallStatus::Traced);

    let page = t.browse(&owner, &format!("/recalls/{}", recall.id)).await;
    page.assert_ok().assert_see("Close the recall");
    t.close_recall_as(&owner, &recall).await.assert_ok();
    assert_eq!(t.recall_now(&recall).await.status, RecallStatus::Closed);
    t.browse(&owner, &format!("/recalls/{}", recall.id))
        .await
        .assert_see("Closed")
        .assert_see("Report")
        .assert_dont_see("Close the recall");
    t.app
        .get(&format!("/recalls/{}/report", recall.id))
        .await
        .assert_ok();
}
