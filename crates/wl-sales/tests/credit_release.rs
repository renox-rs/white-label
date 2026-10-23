//! BL-SAL-11 (#130): a holder of `credit.release` asks to release an order on
//! credit hold, another one approves, and the order goes back to the state it
//! was held from (`SAL.ACL.03`, `PLT.APR.01`).

mod support;

use renox::prelude::*;
use wl_kernel::approvals::{self, ApprovalRequest, ApprovalStatus};
use wl_kernel::sales::{
    CreditFailure, CreditVerdict, FulfilOrder, FulfilledLine, SalesOrderId, credit_checker,
    order_fulfilment,
};
use wl_sales::permissions as perm;
use wl_sales::{SaleLine, SalesOrder, SalesOrderStatus};

use support::Booted;
use support::credit::Credit;
use support::sell::{
    CUSTOMER, account, add_line, boot, confirmed, draft, recheck, reload, rep_and_accountant,
    seller, submit,
};

/// A user who may release held orders and see the credit figures.
async fn releaser(t: &Booted, name: &str) -> User {
    t.user(
        name,
        &[
            perm::SALES_ORDERS_VIEW,
            perm::CREDIT_VIEW,
            perm::CREDIT_RELEASE,
        ],
    )
    .await
}

/// The order held at dispatch while `confirmed` (or `partially_fulfilled`
/// when `partly`), entered by `maker`.
async fn held_at_dispatch(t: &Booted, credit: &Credit, maker: &User, partly: bool) -> SalesOrder {
    let (rep, _) = rep_and_accountant(t).await;
    account(t, &rep).await;
    t.app.acting_as(maker);
    let order = draft(t, maker, "phone", "on_account", 800_000).await;
    if partly {
        add_line(t, order.id, "valve", 200_000).await;
    }
    submit(t, order.id).await;
    if partly {
        let lines = SaleLine::of(t.db(), &order.subject()).await.unwrap();
        order_fulfilment(t.state())
            .fulfil(
                t.state(),
                FulfilOrder {
                    order: SalesOrderId(order.id),
                    lines: vec![FulfilledLine {
                        line: lines[0].id,
                        quantity_milli: 1000,
                    }],
                    source: wl_kernel::Subject::new("inventory.dispatch", 1),
                },
            )
            .await
            .unwrap();
    }
    credit.ledger.invoice(CUSTOMER, 5_000_000, 10);
    let order = reload(t, order.id).await;
    assert_eq!(
        recheck(t, &order).await,
        CreditVerdict::Failed(CreditFailure::Limit)
    );
    let held = reload(t, order.id).await;
    assert_eq!(held.status, SalesOrderStatus::CreditHold);
    held
}

/// The order held at `submit`: the customer already owes too much.
async fn held_at_submit(t: &Booted, credit: &Credit, maker: &User) -> SalesOrder {
    let (rep, _) = rep_and_accountant(t).await;
    account(t, &rep).await;
    credit.ledger.invoice(CUSTOMER, 5_000_000, 10);
    t.app.acting_as(maker);
    let order = draft(t, maker, "phone", "on_account", 800_000).await;
    submit(t, order.id).await;
    let held = reload(t, order.id).await;
    assert_eq!(held.status, SalesOrderStatus::CreditHold);
    held
}

/// `by` asks to release `order` through the page.
async fn ask(t: &Booted, by: &User, order: &SalesOrder) -> renox::testing::TestResponse {
    t.app.acting_as(by);
    t.app
        .post(
            &format!("/sales-orders/{}/release", order.id),
            &[("note", "Customer paid part of it")],
        )
        .await
}

async fn request_of(t: &Booted, order: &SalesOrder) -> ApprovalRequest {
    ApprovalRequest::pending_about(t.db(), &order.subject())
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("a release request waits")
}

/// AC1: held at dispatch from `partially_fulfilled`; user B (not the creator)
/// releases with E's approval; it is `partially_fulfilled` and no
/// `SalesOrderConfirmed` follows.
#[renox::test]
async fn an_order_held_at_dispatch_returns_to_partially_fulfilled_without_a_second_confirmation() {
    let (t, credit) = boot().await;
    let maker = seller(&t).await;
    let order = held_at_dispatch(&t, &credit, &maker, true).await;
    assert_eq!(order.held_from.as_deref(), Some("partially_fulfilled"));
    let confirmations = credit.confirmed.lock().unwrap().len();
    assert_eq!(confirmations, 1);
    let (b, e) = (releaser(&t, "Bina").await, releaser(&t, "Essa").await);

    ask(&t, &b, &order).await.assert_status(303);
    // The request waits: nothing moved yet.
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::CreditHold
    );
    let request = request_of(&t, &order).await;
    assert_eq!(request.requester_id, b.id);
    approvals::approve(t.state(), request.id, &e, "Paid")
        .await
        .unwrap();

    let released = reload(&t, order.id).await;
    assert_eq!(released.status, SalesOrderStatus::PartiallyFulfilled);
    assert!(released.released_at.is_some());
    assert_eq!(credit.confirmed.lock().unwrap().len(), confirmations);
}

/// AC2: held at submit, the release makes it `confirmed` and emits
/// `SalesOrderConfirmed` once.
#[renox::test]
async fn an_order_held_at_submit_is_confirmed_by_its_release_and_announced_once() {
    let (t, credit) = boot().await;
    let maker = seller(&t).await;
    let order = held_at_submit(&t, &credit, &maker).await;
    assert!(credit.confirmed.lock().unwrap().is_empty());
    assert!(order.confirmed_at.is_none());
    let (b, e) = (releaser(&t, "Bina").await, releaser(&t, "Essa").await);

    ask(&t, &b, &order).await.assert_status(303);
    let request = request_of(&t, &order).await;
    approvals::approve(t.state(), request.id, &e, "")
        .await
        .unwrap();

    let released = reload(&t, order.id).await;
    assert_eq!(released.status, SalesOrderStatus::Confirmed);
    assert!(released.confirmed_at.is_some());
    let events = credit.confirmed.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].order, SalesOrderId(order.id));
}

/// AC3: the creator A cannot release, even holding every permission, and
/// cannot approve someone else's release of their own order.
#[renox::test]
async fn the_creator_can_neither_release_nor_approve_the_release_of_their_own_order() {
    let (t, credit) = boot().await;
    let creator = t
        .user(
            "Adam",
            &[
                perm::SALES_ORDERS_VIEW,
                perm::SALES_ORDERS_CREATE,
                perm::SALES_ORDERS_CANCEL,
                perm::CREDIT_VIEW,
                perm::CREDIT_RELEASE,
                perm::CREDIT_LIMITS_APPROVE,
            ],
        )
        .await;
    let order = held_at_dispatch(&t, &credit, &creator, false).await;

    ask(&t, &creator, &order).await.assert_forbidden();
    assert!(
        ApprovalRequest::pending_about(t.db(), &order.subject())
            .await
            .unwrap()
            .is_empty()
    );

    let b = releaser(&t, "Bina").await;
    ask(&t, &b, &order).await.assert_status(303);
    let request = request_of(&t, &order).await;
    assert!(
        approvals::approve(t.state(), request.id, &creator, "")
            .await
            .is_err()
    );
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::CreditHold
    );
}

/// The requester cannot approve their own request, and a rejection leaves
/// the order on hold.
#[renox::test]
async fn a_release_is_decided_by_another_releaser_and_a_rejection_keeps_the_hold() {
    let (t, credit) = boot().await;
    let maker = seller(&t).await;
    let order = held_at_dispatch(&t, &credit, &maker, false).await;
    let (b, e) = (releaser(&t, "Bina").await, releaser(&t, "Essa").await);

    ask(&t, &b, &order).await.assert_status(303);
    let request = request_of(&t, &order).await;
    assert!(
        approvals::approve(t.state(), request.id, &b, "")
            .await
            .is_err()
    );
    // A second request while one waits is refused.
    ask(&t, &e, &order).await.assert_status(422);

    approvals::reject(t.state(), request.id, &e, "Still owes too much")
        .await
        .unwrap();
    let stored = ApprovalRequest::find(t.db(), request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ApprovalStatus::Rejected);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::CreditHold
    );
    // Asking again is possible after a rejection.
    ask(&t, &e, &order).await.assert_status(303);
}

/// A user without `credit.release` cannot ask, and only a held order can be
/// released.
#[renox::test]
async fn only_a_credit_releaser_releases_and_only_a_held_order() {
    let (t, credit) = boot().await;
    let maker = seller(&t).await;
    let order = held_at_dispatch(&t, &credit, &maker, false).await;
    let viewer = t.user("Vera", &[perm::SALES_ORDERS_VIEW]).await;
    ask(&t, &viewer, &order).await.assert_forbidden();

    credit.ledger.clear();
    let other = confirmed(&t, &seller_two(&t).await).await;
    let b = releaser(&t, "Bina").await;
    ask(&t, &b, &other).await.assert_status(422);
}

async fn seller_two(t: &Booted) -> User {
    t.user(
        "Sol",
        &[
            perm::SALES_ORDERS_VIEW,
            perm::SALES_ORDERS_CREATE,
            perm::SALES_ORDERS_ACCEPT,
        ],
    )
    .await
}

/// After a release the next dispatch passes without checking again while the
/// customer owes no more; owing more checks (and holds) again.
#[renox::test]
async fn a_released_order_passes_the_next_dispatch_until_the_customer_owes_more() {
    let (t, credit) = boot().await;
    let maker = seller(&t).await;
    let order = held_at_dispatch(&t, &credit, &maker, false).await;
    let (b, e) = (releaser(&t, "Bina").await, releaser(&t, "Essa").await);
    ask(&t, &b, &order).await.assert_status(303);
    let request = request_of(&t, &order).await;
    approvals::approve(t.state(), request.id, &e, "")
        .await
        .unwrap();
    let released = reload(&t, order.id).await;
    assert_eq!(released.status, SalesOrderStatus::Confirmed);
    assert_eq!(released.release_exposure, Some(5_000_000));

    // Still over the limit (5,000,000 + 800,000), yet the release stands.
    assert_eq!(recheck(&t, &released).await, CreditVerdict::Passed);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );

    credit.ledger.invoice(CUSTOMER, 100_000, 10);
    assert_eq!(
        credit_checker(t.state())
            .credit_recheck(t.state(), SalesOrderId(order.id))
            .await
            .unwrap(),
        CreditVerdict::Failed(CreditFailure::Limit)
    );
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::CreditHold
    );
}

/// Surface (back office): the creator sees the release button disabled with
/// the reason, another releaser sees the sheet, the request shows as a card
/// with the approver's buttons, and a viewer without `credit.release` has
/// no button.
#[renox::test]
async fn the_order_page_offers_the_release_to_the_right_people() {
    let (t, credit) = boot().await;
    let maker = t
        .user(
            "Adam",
            &[
                perm::SALES_ORDERS_VIEW,
                perm::SALES_ORDERS_CREATE,
                perm::CREDIT_RELEASE,
            ],
        )
        .await;
    let order = held_at_dispatch(&t, &credit, &maker, false).await;
    let url = format!("/sales-orders/{}", order.id);

    t.app.acting_as(&maker);
    t.app
        .get(&url)
        .await
        .assert_ok()
        .assert_see("You entered this order, so another credit releaser must release it.");

    let b = releaser(&t, "Bina").await;
    t.app.acting_as(&b);
    t.app
        .get(&url)
        .await
        .assert_ok()
        .assert_see("Release the order")
        .assert_see("Note for the approver")
        .assert_dont_see("You entered this order");
    ask(&t, &b, &order).await.assert_status(303);
    let request = request_of(&t, &order).await;
    let approve = format!("/approvals/{}/approve", request.id);
    t.app
        .get(&url)
        .await
        .assert_ok()
        .assert_see("A release is waiting for approval.")
        .assert_see("Release an order from a credit hold")
        .assert_dont_see(&approve);

    let e = releaser(&t, "Essa").await;
    t.app.acting_as(&e);
    t.app.get(&url).await.assert_ok().assert_see(&approve);

    let viewer = t.user("Vera", &[perm::SALES_ORDERS_VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .get(&url)
        .await
        .assert_ok()
        .assert_dont_see("Release the order");
}
