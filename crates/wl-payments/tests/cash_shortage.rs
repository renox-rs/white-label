//! BL-PAY-17 (#110), `FIN.ACL.06`, `FIN.ACL.05`, X-07, D-12: a short handover
//! is recovered from the holder in cash (in parts or at once) or written off
//! with an approval above the limit, as a loss of the holder's branch; the
//! recoveries recorded stay shown. The approvals run through the kernel.

mod support;

use wl_kernel::approvals::{self, ApprovalEnd};
use wl_kernel::custody::HandoverStatus;
use wl_payments::cash_handover::PostingKind;
use wl_payments::cash_recovery::{RecoveryKind, RecoveryStatus};
use wl_payments::cash_settle::ACTION;

use support::boot;

/// AC3: with AED 50.00 short, `recover` with cash AED 50.00 moves the
/// handover to `recovered`; the settlement is announced.
#[renox::test]
async fn recovering_the_whole_shortage_in_cash_moves_it_to_recovered() {
    let t = boot().await;
    let r = t.holder("Rana").await;
    let c = t.counter("Chandra").await;
    let d = t.counter("Dev").await;
    let handover = t.short_handover(&r, "2027-04-01", 230_000, "2250.00", &c).await;
    assert_eq!(handover.difference, 5_000);

    t.cash_post(&d, &format!("/cash/{}/recover", handover.id), &[("amount", "50.00")])
        .await
        .assert_ok();

    let after = t.reloaded(&handover).await;
    assert_eq!(after.status, HandoverStatus::Recovered);
    assert_eq!(after.recovered, 5_000);
    let recoveries = t.recoveries(&after).await;
    assert_eq!(recoveries.len(), 1);
    assert_eq!(recoveries[0].kind, RecoveryKind::Cash);
    assert_eq!(recoveries[0].status, RecoveryStatus::Recorded);
    assert_eq!(recoveries[0].recorded_by, Some(d.id));
    let settled = t.cash_settled.all();
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].status, HandoverStatus::Recovered);
    assert_eq!((settled[0].recovered, settled[0].written_off), (5_000, 0));
}

/// The holder never records a recovery or recovers their own shortage; a
/// recovery above what is left is refused; a handover that is not short takes
/// none.
#[renox::test]
async fn recoveries_are_made_by_someone_else_and_within_the_shortage() {
    let t = boot().await;
    let r = t.counter("Rana").await;
    let c = t.counter("Chandra").await;
    let handover = t.short_handover(&r, "2027-04-01", 230_000, "2250.00", &c).await;

    t.recover_part(&r, &handover, "10.00").await.assert_status(422);
    t.cash_post(&r, &format!("/cash/{}/recover", handover.id), &[("amount", "50.00")])
        .await
        .assert_status(422);
    t.recover_part(&c, &handover, "50.01").await.assert_status(422);
    t.recover_part(&c, &handover, "0").await.assert_status(422);
    assert!(t.recoveries(&handover).await.is_empty());
    assert_eq!(t.reloaded(&handover).await.recovered, 0);

    let open = t.collect(&r, "2027-04-02", 10_000, 2).await;
    t.recover_part(&c, &open, "10.00").await.assert_status(422);
}

/// AC4: `write_off` of AED 50.00 (limit AED 0) needs approval; after another
/// user approves, the loss is posted to R's branch. The requester cannot
/// approve; a user without `cash.write_off` is refused.
#[renox::test]
async fn a_write_off_above_the_limit_waits_for_a_second_user_and_posts_the_loss() {
    let t = boot().await;
    let r = t.holder("Rana").await;
    let c = t.counter("Chandra").await;
    let m = t.cash_manager("Mona").await;
    let n = t.cash_manager("Nadia").await;
    let handover = t.short_handover(&r, "2027-04-01", 230_000, "2250.00", &c).await;

    t.write_off_cash(&c, &handover).await.assert_forbidden();
    t.write_off_cash(&r, &handover).await.assert_forbidden();
    t.write_off_cash(&m, &handover).await.assert_ok();

    assert_eq!(t.reloaded(&handover).await.status, HandoverStatus::Short);
    let request = t.pending_request(&handover.subject()).await.expect("a request waits");
    assert_eq!(request.action, ACTION);
    assert_eq!(request.requester_id, m.id);
    assert_eq!(request.payload.0["amount"], 5_000);
    assert!(approvals::approve(t.state(), request.id, &m, "mine").await.is_err());
    assert!(t.cash_settled.all().is_empty());

    let decided = approvals::approve(t.state(), request.id, &n, "agreed")
        .await
        .unwrap()
        .expect("the last step decides");
    assert_eq!(decided.decision, ApprovalEnd::Approved);

    let after = t.reloaded(&handover).await;
    assert_eq!(after.status, HandoverStatus::WrittenOff);
    assert_eq!(after.written_off, 5_000);
    let postings = t.postings(&after).await;
    assert_eq!(postings.len(), 1);
    assert_eq!(postings[0].kind, PostingKind::ShortageLoss);
    assert_eq!(postings[0].amount, 5_000);
    assert_eq!(postings[0].location.as_deref(), Some("Dubai branch"));
    let settled = t.cash_settled.all();
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].status, HandoverStatus::WrittenOff);
    assert_eq!(settled[0].written_off, 5_000);
}

/// AC5: with AED 50.00 short, a part recovery of AED 20.00 cash is recorded and
/// the handover stays `short`; `recover` is refused; `write_off` of the rest
/// asks approval for AED 30.00 and, after approval, is `written_off` with AED
/// 30.00 posted as a loss and AED 20.00 shown recovered.
#[renox::test]
async fn a_part_recovery_leaves_the_rest_to_write_off() {
    let t = boot().await;
    let r = t.holder("Rana").await;
    let c = t.counter("Chandra").await;
    let m = t.cash_manager("Mona").await;
    let n = t.cash_manager("Nadia").await;
    let handover = t.short_handover(&r, "2027-04-01", 230_000, "2250.00", &c).await;

    t.recover_part(&c, &handover, "20.00").await.assert_ok();

    let part = t.reloaded(&handover).await;
    assert_eq!(part.status, HandoverStatus::Short);
    assert_eq!(part.recovered, 2_000);
    assert_eq!(part.rest(), 3_000);
    // `recover` needs the recoveries to cover the whole difference.
    t.cash_post(&c, &format!("/cash/{}/recover", handover.id), &[])
        .await
        .assert_status(422);
    assert_eq!(t.reloaded(&handover).await.status, HandoverStatus::Short);

    t.write_off_cash(&m, &handover).await.assert_ok();
    let request = t.pending_request(&handover.subject()).await.expect("a request waits");
    assert_eq!(request.payload.0["amount"], 3_000);
    approvals::approve(t.state(), request.id, &n, "agreed").await.unwrap();

    let after = t.reloaded(&handover).await;
    assert_eq!(after.status, HandoverStatus::WrittenOff);
    assert_eq!((after.written_off, after.recovered), (3_000, 2_000));
    let postings = t.postings(&after).await;
    assert_eq!(postings.len(), 1);
    assert_eq!(postings[0].amount, 3_000);
    let recoveries = t.recoveries(&after).await;
    assert_eq!(recoveries.len(), 1, "the recovery stays shown");
    assert_eq!(recoveries[0].amount, 2_000);
    let settled = t.cash_settled.all();
    assert_eq!((settled[0].recovered, settled[0].written_off), (2_000, 3_000));
}

/// A recovery recorded while the write-off waits changes what is left: at the
/// final approval the request fails and nothing is written off.
#[renox::test]
async fn a_recovery_during_the_approval_fails_the_write_off() {
    let t = boot().await;
    let r = t.holder("Rana").await;
    let c = t.counter("Chandra").await;
    let m = t.cash_manager("Mona").await;
    let n = t.cash_manager("Nadia").await;
    let handover = t.short_handover(&r, "2027-04-01", 230_000, "2250.00", &c).await;
    t.write_off_cash(&m, &handover).await.assert_ok();
    let request = t.pending_request(&handover.subject()).await.expect("a request waits");

    t.recover_part(&c, &handover, "10.00").await.assert_ok();
    let decided = approvals::approve(t.state(), request.id, &n, "agreed")
        .await
        .unwrap()
        .expect("the request ends");

    assert_eq!(decided.decision, ApprovalEnd::Failed);
    let after = t.reloaded(&handover).await;
    assert_eq!(after.status, HandoverStatus::Short);
    assert_eq!(after.written_off, 0);
    assert!(t.postings(&after).await.is_empty());
    // Asked again for what is left, it can go through.
    t.write_off_cash(&m, &after).await.assert_ok();
    let again = t.pending_request(&handover.subject()).await.expect("a new request");
    assert_eq!(again.payload.0["amount"], 4_000);
}

/// A write-off needs a reason and something left to write off.
#[renox::test]
async fn a_write_off_needs_a_reason_and_a_rest() {
    let t = boot().await;
    let r = t.holder("Rana").await;
    let c = t.counter("Chandra").await;
    let m = t.cash_manager("Mona").await;
    let handover = t.short_handover(&r, "2027-04-01", 230_000, "2250.00", &c).await;

    t.cash_post(&m, &format!("/cash/{}/write-off", handover.id), &[("reason", "  ")])
        .await
        .assert_status(422);
    assert!(t.pending_request(&handover.subject()).await.is_none());

    t.recover_part(&c, &handover, "50.00").await.assert_ok();
    t.write_off_cash(&m, &t.reloaded(&handover).await).await.assert_status(422);
    assert!(t.pending_request(&handover.subject()).await.is_none());
}
