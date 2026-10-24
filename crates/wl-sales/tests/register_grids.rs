//! The sales registers as `renox::grid`s (F7 grid standard): /sales-orders and
//! /credit/accounts sort, filter, search and page on the server, say so in
//! words when a filter matches nothing, and keep the views the tiles and
//! tabs link to.

mod support;

use wl_kernel::PartyId;
use wl_kernel::cheques::{ChequeBounced, ChequeId};
use wl_sales::SalesOrder;
use wl_sales::customer_account::{self, Terms};
use wl_sales::permissions as perm;

use support::Booted;
use support::sell::{CUSTOMER, account, boot, draft, rep_and_accountant, seller};

/// Twenty-seven draft orders (more than a page of 25), oldest first.
async fn many_orders(t: &Booted) -> Vec<SalesOrder> {
    let by = seller(t).await;
    let mut orders = Vec::new();
    for n in 0..27 {
        orders.push(draft(t, &by, "phone", "on_account", 100_000 + n * 1_000).await);
    }
    orders
}

/// The orders list pages, sorts, searches and filters on the server.
#[renox::test]
async fn the_sales_orders_grid_pages_sorts_searches_and_filters() {
    let (t, _credit) = boot().await;
    let orders = many_orders(&t).await;
    let (first, last) = (&orders[0], &orders[26]);
    let viewer = t.user("Vera", &[perm::SALES_ORDERS_VIEW]).await;
    t.app.acting_as(&viewer);

    // Newest first, 25 to a page.
    t.app
        .get("/sales-orders")
        .await
        .assert_ok()
        .assert_see(&last.number)
        .assert_dont_see(&first.number);
    // The second page has the two oldest.
    t.app
        .get("/sales-orders?page=2")
        .await
        .assert_ok()
        .assert_see(&first.number)
        .assert_dont_see(&last.number);
    // `sort=` changes the order: oldest first.
    t.app
        .get("/sales-orders?sort=id")
        .await
        .assert_ok()
        .assert_see(&first.number)
        .assert_dont_see(&last.number);
    // A search narrows to the order asked for.
    let wanted = &orders[12];
    t.app
        .get(&format!("/sales-orders?search={}", wanted.number))
        .await
        .assert_ok()
        .assert_see(&wanted.number)
        .assert_dont_see(&first.number)
        .assert_dont_see(&last.number);
    // A status filter narrows; the drafts match, the cancelled do not.
    t.app
        .get("/sales-orders?in.status=draft&per_page=10")
        .await
        .assert_ok()
        .assert_see(&last.number)
        .assert_dont_see(&first.number);
    t.app
        .get("/sales-orders?in.status=cancelled")
        .await
        .assert_ok()
        .assert_dont_see(&last.number)
        .assert_see("Clear filters");
    // The views the tabs and tiles link to keep working.
    t.app
        .get("/sales-orders?in.status=credit_hold")
        .await
        .assert_ok()
        .assert_dont_see(&last.number);
    t.app
        .get("/sales-orders?awaiting=1")
        .await
        .assert_ok()
        .assert_dont_see(&last.number);
}

/// The orders of a user who may not view them are not listed: the route is
/// closed.
#[renox::test]
async fn a_user_without_the_view_permission_sees_no_order() {
    let (t, _credit) = boot().await;
    let orders = many_orders(&t).await;
    let outsider = t.user("Otto", &[perm::CREDIT_VIEW]).await;
    t.app.acting_as(&outsider);
    t.app.get("/sales-orders").await.assert_forbidden();
    let _ = orders;
}

/// A bounced cheque puts the account on hold.
async fn bounce(t: &Booted, party: i64, n: i64) {
    t.state()
        .emit(ChequeBounced {
            cheque: ChequeId(n),
            party: PartyId(party),
            credit_group: None,
            amount: 1_000_000,
            honoured_amount: 0,
            open_amount: 1_000_000,
            currency: "AED".to_owned(),
            reason: "insufficient funds".to_owned(),
            presentation: 1,
            linked: vec![],
        })
        .await
        .unwrap();
}

/// The accounts list pages, sorts, filters on hold and says so when nothing
/// matches.
#[renox::test]
async fn the_credit_accounts_grid_pages_sorts_and_filters_the_accounts_on_hold() {
    let (t, _credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    account(&t, &rep).await;
    for n in 0..26 {
        customer_account::save(
            t.state(),
            PartyId(100 + n),
            Terms::default().limit(1_000_000 + n * 100_000),
        )
        .await
        .unwrap();
    }
    bounce(&t, CUSTOMER, 31).await;
    let viewer = t.user("Carla", &[perm::CREDIT_VIEW]).await;
    t.app.acting_as(&viewer);

    // 27 accounts: the first page is 25, the next has the rest.
    let page_one = t.app.get("/credit/accounts").await;
    page_one.assert_ok().assert_see("Account in good standing");
    assert!(page_one.text().contains("/credit/accounts/100"));
    assert!(!page_one.text().contains("/credit/accounts/125"));
    let page_two = t.app.get("/credit/accounts?page=2").await;
    page_two.assert_ok();
    assert!(page_two.text().contains("/credit/accounts/125"));
    assert!(!page_two.text().contains("/credit/accounts/100"));
    // `sort=` changes the order: the biggest limit first.
    let by_limit = t.app.get("/credit/accounts?sort=-limit&per_page=10").await;
    by_limit.assert_ok();
    assert!(by_limit.text().contains("/credit/accounts/125"));
    assert!(!by_limit.text().contains("/credit/accounts/100"));
    // The on-hold filter narrows to the account on hold.
    let held = t.app.get("/credit/accounts?in.standing=on_hold").await;
    held.assert_ok().assert_see("Account on hold");
    assert!(
        held.text()
            .contains(&format!("/credit/accounts/{CUSTOMER}"))
    );
    assert!(!held.text().contains("/credit/accounts/100"));
    // A search finds a customer by number; one that matches nothing offers
    // to clear the filters.
    let found = t.app.get("/credit/accounts?search=125").await;
    found.assert_ok();
    assert!(found.text().contains("/credit/accounts/125"));
    assert!(!found.text().contains("/credit/accounts/100"));
    t.app
        .get("/credit/accounts?search=nobody-is-called-this")
        .await
        .assert_ok()
        .assert_see("Clear filters");
    // Without `credit.view` the list is closed.
    let clerk = t.user("Clara", &[perm::SALES_ORDERS_VIEW]).await;
    t.app.acting_as(&clerk);
    t.app.get("/credit/accounts").await.assert_forbidden();
}
