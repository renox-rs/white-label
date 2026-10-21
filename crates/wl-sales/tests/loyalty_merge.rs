//! A party merge, and a party that is blocked or unblocked, move or pause the
//! loyalty account (BL-SAL-26, #145): the points go to the survivor with their
//! expiry dates and nothing is forfeited; a blocked member earns and redeems
//! nothing and is refused with the neutral text *on hold*. The events are the
//! ones `wl-parties` emits; the listeners are the module's own.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::parties::{PartiesMerged, PartyBlocked, PartyStatus, PartyUnblocked};
use wl_sales::{
    AccountStatus, LoyaltyAccount, PointsAdjustment, PointsKind, PointsMovement, WlSales,
};

use support::floor::{Floor, account, movements};
use support::retention::Parties;
use support::shop::Shop;
use support::{Booted, boot_pair};

async fn world() -> (Booted, Shop, Parties, Floor) {
    let shop = Shop::default();
    let parties = Parties::default();
    let t = boot_pair(WlSales::new(), shop.clone(), parties.clone()).await;
    let floor = Floor::new(&t).await;
    floor.turn_on(&t, &[]).await;
    (t, shop, parties, floor)
}

fn merged(survivor: i64, merged: i64) -> PartiesMerged {
    PartiesMerged {
        survivor: PartyId(survivor),
        merged: PartyId(merged),
    }
}

fn blocked(party: i64) -> PartyBlocked {
    PartyBlocked {
        party: PartyId(party),
        display_reason: "fraud_review".to_owned(),
    }
}

/// Sets the last day of the lot `lot`.
async fn end_on(t: &Booted, lot: &PointsMovement, last: NaiveDate) {
    let last = Some(last);
    PointsMovement::query()
        .where_(PointsMovement::ID.eq(lot.id))
        .update(t.db(), &[("lot_expires_on", &last)])
        .await
        .unwrap();
}

#[renox::test]
async fn merging_moves_the_lots_with_their_expiry_and_closes_the_merged_account() {
    let (t, _shop, parties, floor) = world().await;
    let p = floor.enrol(&t, 1, "0501111111").await;
    floor.earn(&t, &p, 200).await;
    let p2 = floor.enrol(&t, 2, "0502222222").await;
    floor.earn(&t, &p2, 300).await;
    let lot = movements(&t, &p2, PointsKind::Earn).await.remove(0);
    let march = NaiveDate::from_ymd_opt(2027, 3, 1).unwrap();
    end_on(&t, &lot, march).await;

    t.state().emit(merged(1, 2)).await.unwrap();
    t.app.run_all_jobs().await;

    // P has 500 points; the moved lot keeps its last day.
    let p = account(&t, &p).await;
    assert_eq!(p.points_balance, 500);
    assert_eq!(p.status, AccountStatus::Active);
    let moved: Vec<_> = movements(&t, &p, PointsKind::Transfer).await;
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].points, 300);
    assert_eq!(moved[0].lot_expires_on, Some(march));
    assert!(moved[0].is_lot());
    let lots = PointsMovement::lots(t.db(), p.id, None).await.unwrap();
    assert_eq!(
        lots.iter().map(PointsMovement::lot_remaining).sum::<i64>(),
        500
    );
    assert!(lots.iter().any(|l| l.lot_expires_on == Some(march)));

    // P2's account is closed *merged into* P, with the points out as a
    // transfer: no forfeit, no `adjust`, no approval.
    let p2 = account(&t, &p2).await;
    assert_eq!(p2.status, AccountStatus::Closed);
    assert_eq!(p2.closed_reason.as_deref(), Some("Merged into LYL-0001"));
    assert_eq!(p2.points_balance, 0);
    let out = movements(&t, &p2, PointsKind::Transfer).await;
    assert_eq!(out.iter().map(|m| m.points).sum::<i64>(), -300);
    assert!(movements(&t, &p2, PointsKind::Adjust).await.is_empty());
    assert_eq!(PointsAdjustment::query().count(t.db()).await.unwrap(), 0);
    let events = parties.transferred.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!((events[0].points, events[0].party), (300, PartyId(1)));
}

#[renox::test]
async fn a_negative_balance_moves_too_and_the_visits_add_up() {
    let (t, _shop, _parties, floor) = world().await;
    let p = floor.enrol(&t, 1, "0501111111").await;
    floor.earn(&t, &p, 200).await;
    let p2 = floor.enrol(&t, 2, "0502222222").await;
    floor.earn(&t, &p2, 100).await;
    // A reversal beyond the lot left P2 with 60 points while its lot holds
    // 100: the balance is 40 below what the lots hold.
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::ID.eq(p2.id))
        .update(t.db(), &[("points_balance", &60_i64), ("visits", &3_i64)])
        .await
        .unwrap();
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::ID.eq(p.id))
        .update(t.db(), &[("visits", &2_i64)])
        .await
        .unwrap();
    t.state().emit(merged(1, 2)).await.unwrap();
    let p = account(&t, &p).await;
    assert_eq!(p.points_balance, 260);
    assert_eq!(p.visits, 5);
    let moved = movements(&t, &p, PointsKind::Transfer).await;
    assert_eq!(moved.iter().map(|m| m.points).sum::<i64>(), 60);
    assert_eq!(account(&t, &p2).await.points_balance, 0);
}

#[renox::test]
async fn without_an_account_of_its_own_the_survivor_gets_the_account_itself() {
    let (t, _shop, _parties, floor) = world().await;
    let p2 = floor.enrol(&t, 2, "0502222222").await;
    floor.earn(&t, &p2, 300).await;
    t.state().emit(merged(1, 2)).await.unwrap();
    let moved = account(&t, &p2).await;
    assert_eq!(moved.party_id, 1);
    assert_eq!(moved.status, AccountStatus::Active);
    assert_eq!(moved.points_balance, 300);
    assert!(!moved.paused);
    assert!(movements(&t, &moved, PointsKind::Transfer).await.is_empty());
    let of_p = LoyaltyAccount::of_party(t.db(), PartyId(1)).await.unwrap();
    assert_eq!(of_p.map(|a| a.id), Some(p2.id));
    // The earlier party has nothing left.
    assert!(
        LoyaltyAccount::of_party(t.db(), PartyId(2))
            .await
            .unwrap()
            .is_none()
    );
}

#[renox::test]
async fn a_blocked_member_earns_nothing_is_refused_on_hold_and_earns_again_after_unblock() {
    let (t, _shop, parties, floor) = world().await;
    let m = floor.enrol(&t, 7, "0501234567").await;
    floor.earn(&t, &m, 800).await;
    // A sale in progress with M attached.
    let open = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;

    parties.directory.set(7, PartyStatus::Blocked);
    t.state().emit(blocked(7)).await.unwrap();
    assert!(account(&t, &m).await.paused);

    // Redemption is refused with the neutral text, never the reason.
    t.app
        .post(
            &format!("/counter-sales/{}/redeem", open.id),
            &[("points", "600")],
        )
        .await
        .assert_status(422)
        .assert_see("This party is on hold")
        .assert_dont_see("fraud_review");
    // The sale completes and earns nothing.
    t.app
        .post(
            &format!("/counter-sales/{}/complete", open.id),
            &[("cash", "21.00")],
        )
        .await
        .assert_status(303);
    assert_eq!(account(&t, &m).await.points_balance, 800);
    assert_eq!(movements(&t, &m, PointsKind::Earn).await.len(), 1);
    // A member on hold cannot be attached to a new sale either.
    let next = floor.open_cart(&t, &[("tee", "10.00", "1")], None).await;
    t.app
        .post(
            &format!("/counter-sales/{}/member", next.id),
            &[("code", m.code.as_str())],
        )
        .await
        .assert_status(422)
        .assert_see("This party is on hold")
        .assert_dont_see("fraud_review");

    // Unblocked: the pause clears and the next sale earns.
    parties.directory.set(7, PartyStatus::Active);
    t.state()
        .emit(PartyUnblocked {
            party: PartyId(7),
            display_reason: "on_hold".to_owned(),
        })
        .await
        .unwrap();
    assert!(!account(&t, &m).await.paused);
    floor.earn(&t, &m, 100).await;
    assert_eq!(account(&t, &m).await.points_balance, 900);
}

#[renox::test]
async fn merging_into_a_blocked_survivor_moves_the_points_paused() {
    let (t, shop, parties, floor) = world().await;
    let p = floor.enrol(&t, 1, "0501111111").await;
    floor.earn(&t, &p, 200).await;
    let p2 = floor.enrol(&t, 2, "0502222222").await;
    floor.earn(&t, &p2, 300).await;
    let cart = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&p.code))
        .await;

    parties.directory.set(1, PartyStatus::Blocked);
    t.state().emit(blocked(1)).await.unwrap();
    t.state().emit(merged(1, 2)).await.unwrap();
    let p = account(&t, &p).await;
    assert_eq!(p.points_balance, 500);
    assert!(p.paused);
    t.app
        .post(
            &format!("/counter-sales/{}/redeem", cart.id),
            &[("points", "500")],
        )
        .await
        .assert_status(422)
        .assert_see("This party is on hold")
        .assert_dont_see("fraud_review");
    assert!(shop.mail.otp_requests().is_empty());

    // Until P is unblocked.
    parties.directory.set(1, PartyStatus::Active);
    t.state()
        .emit(PartyUnblocked {
            party: PartyId(1),
            display_reason: "on_hold".to_owned(),
        })
        .await
        .unwrap();
    t.app
        .post(
            &format!("/counter-sales/{}/redeem", cart.id),
            &[("points", "500")],
        )
        .await
        .assert_status(303);
}

#[renox::test]
async fn an_account_that_moves_to_a_blocked_survivor_is_paused() {
    let (t, _shop, parties, floor) = world().await;
    let p2 = floor.enrol(&t, 2, "0502222222").await;
    floor.earn(&t, &p2, 300).await;
    parties.directory.set(1, PartyStatus::Blocked);
    t.state().emit(merged(1, 2)).await.unwrap();
    let moved = account(&t, &p2).await;
    assert_eq!(moved.party_id, 1);
    assert!(moved.paused);
}
