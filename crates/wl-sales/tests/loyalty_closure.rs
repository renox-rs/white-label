//! A member leaves the programme (BL-SAL-26, #145): never refused, never
//! approved. At the counter the cashier records the member's confirmation, in
//! the portal the member confirms; the points go with an `adjust` and the
//! account closes. A member on hold can leave too.

mod support;

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::parties::{PartyStatus, PartyBlocked};
use wl_sales::notices::LOYALTY_CLOSED;
use wl_sales::{AccountStatus, LoyaltyAccount, PointsAdjustment, PointsKind, Redemption, WlSales};

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

#[renox::test]
async fn the_cashier_records_a_member_leaving_and_the_450_points_are_forfeited_without_approval() {
    let (t, shop, _parties, floor) = world().await;
    let m = floor.enrol(&t, 7, "0501234567").await;
    floor.earn(&t, &m, 450).await;
    let url = format!("/loyalty/accounts/{}/leave", m.id);

    // Without the member's confirmation nothing is forfeited.
    t.app.htmx().post(&url, &[("x", "")]).await.assert_status(422);
    assert_eq!(account(&t, &m).await.status, AccountStatus::Active);

    t.app
        .post(&url, &[("confirm", "1")])
        .await
        .assert_status(303);
    let m = account(&t, &m).await;
    assert_eq!(m.status, AccountStatus::Closed);
    assert_eq!(m.points_balance, 0);
    assert_eq!(m.closed_reason.as_deref(), Some("Member request (counter)"));
    // An `adjust` of -450 that says who recorded it; no request for approval.
    let adjusts = movements(&t, &m, PointsKind::Adjust).await;
    assert_eq!(adjusts.iter().map(|a| a.points).sum::<i64>(), -450);
    assert_eq!(adjusts[0].user_id, Some(floor.cora.id));
    assert_eq!(adjusts[0].reason.as_deref(), Some("Member request (counter)"));
    assert_eq!(adjusts[0].approval_request_id, None);
    assert_eq!(PointsAdjustment::query().count(t.db()).await.unwrap(), 0);
    // The earn lot is used up, the member is told, and the event says -450.
    assert_eq!(shop.mail.of_type(LOYALTY_CLOSED).len(), 1);
    let events = shop.points.adjusted.lock().unwrap().clone();
    assert_eq!(events.last().map(|e| e.points), Some(-450));

    // It cannot be closed twice.
    t.app.htmx().post(&url, &[("confirm", "1")]).await.assert_status(422);
}

#[renox::test]
async fn points_held_for_a_sale_in_progress_are_released_when_the_member_leaves() {
    let (t, shop, _parties, floor) = world().await;
    let m = floor.enrol(&t, 7, "0501234567").await;
    floor.earn(&t, &m, 800).await;
    let cart = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    t.app
        .post(&format!("/counter-sales/{}/redeem", cart.id), &[("points", "600")])
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/counter-sales/{}/redeem/confirm", cart.id),
            &[("code", shop.mail.code_for(&cart.subject()).as_str())],
        )
        .await
        .assert_status(303);

    t.app
        .post(
            &format!("/loyalty/accounts/{}/leave", m.id),
            &[("confirm", "1")],
        )
        .await
        .assert_status(303);
    let m = account(&t, &m).await;
    assert_eq!(m.status, AccountStatus::Closed);
    assert_eq!(m.points_balance, 0);
    let rows = Redemption::of_sale(t.db(), &cart.subject()).await.unwrap();
    assert!(rows.iter().all(|r| r.status != wl_sales::RedemptionStatus::Held));
}

#[renox::test]
async fn the_member_leaves_in_the_portal_and_a_member_on_hold_may_too() {
    let (t, _shop, parties, floor) = world().await;
    let m = floor.enrol(&t, 7, "0501234567").await;
    floor.earn(&t, &m, 450).await;
    let member = t.user("Mona", &[]).await;
    let stranger = t.user("Sam", &[]).await;
    parties.portal.grant(member.id, 7);
    // M's party is on hold: the account is paused.
    parties.directory.set(7, PartyStatus::Blocked);
    t.state()
        .emit(PartyBlocked {
            party: PartyId(7),
            display_reason: "on_hold".to_owned(),
        })
        .await
        .unwrap();
    assert!(account(&t, &m).await.paused);

    let page = format!("/portal/loyalty/{}", m.id);
    let leave = format!("/portal/loyalty/{}/leave", m.id);
    // Somebody who does not act for the member finds nothing.
    t.app.acting_as(&stranger);
    t.app.get(&page).await.assert_not_found();
    t.app.post(&leave, &[("confirm", "1")]).await.assert_not_found();

    // The member sees the points that will be forfeited and confirms.
    t.app.acting_as(&member);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("450")
        .assert_see("LYL-0001");
    t.app.htmx().post(&leave, &[("x", "")]).await.assert_status(422);
    assert_eq!(account(&t, &m).await.status, AccountStatus::Active);
    t.app
        .post(&leave, &[("confirm", "on")])
        .await
        .assert_status(303);
    let closed = LoyaltyAccount::find_or_404(t.db(), m.id).await.unwrap();
    assert_eq!(closed.status, AccountStatus::Closed);
    assert_eq!(closed.closed_reason.as_deref(), Some("Member request (portal)"));
    let adjusts = movements(&t, &closed, PointsKind::Adjust).await;
    assert_eq!(adjusts.iter().map(|a| a.points).sum::<i64>(), -450);
    assert_eq!(adjusts[0].user_id, Some(member.id));
    assert_eq!(PointsAdjustment::query().count(t.db()).await.unwrap(), 0);

    // Enrolling again starts with nothing.
    t.app.acting_as(&floor.cora);
    let again = floor.enrol(&t, 7, "0501234567").await;
    assert_ne!(again.id, m.id);
    assert_eq!(again.points_balance, 0);
}
