//! BL-SAL-10 (#129), the back-office screens: the credit hold of an order
//! with its reason (for a holder of `credit.view`), the list of orders on
//! credit hold, and the customer account page with what the customer owes.

mod support;

use renox::prelude::*;
use wl_kernel::sales::{SalesOrderId, credit_checker};
use wl_sales::permissions as perm;
use wl_sales::{SalesOrder, SalesOrderStatus};

use support::sell::{CUSTOMER, account, boot, confirmed, reload, rep_and_accountant, seller};

/// A confirmed order of AED 8,000 held at dispatch because the customer owes
/// AED 46,000 against a limit of 50,000.
async fn held_order(t: &support::Booted, credit: &support::credit::Credit) -> (SalesOrder, User) {
    let (rep, _) = rep_and_accountant(t).await;
    account(t, &rep).await;
    let order = confirmed(t, &seller(t).await).await;
    credit.ledger.invoice(CUSTOMER, 3_800_000, 10);
    credit.ledger.cheque(CUSTOMER, 800_000);
    credit_checker(t.state())
        .credit_recheck(t.state(), SalesOrderId(order.id))
        .await
        .unwrap();
    let held = reload(t, order.id).await;
    assert_eq!(held.status, SalesOrderStatus::CreditHold);
    (held, rep)
}

/// Surface (back office): the held order shows `credit_hold` with its reason
/// and that its goods stay reserved; a viewer without `credit.view` sees the
/// fact but not the reason.
#[renox::test]
async fn the_held_order_shows_its_reason_only_to_those_who_may_see_credit() {
    let (t, credit) = boot().await;
    let (order, _) = held_order(&t, &credit).await;
    let url = format!("/sales-orders/{}", order.id);

    let accountant = t
        .user("Carla", &[perm::SALES_ORDERS_VIEW, perm::CREDIT_VIEW])
        .await;
    t.app.acting_as(&accountant);
    let page = t.app.get(&url).await;
    page.assert_ok()
        .assert_see("On credit hold")
        .assert_see("The goods stay reserved")
        .assert_see("The customer would be over the credit limit")
        .assert_see("AED 54,000.00")
        .assert_see("AED 50,000.00")
        .assert_see("Confirmed");
    assert!(!page.text().contains("credit.hold_title"));

    let clerk = t.user("Clara", &[perm::SALES_ORDERS_VIEW]).await;
    t.app.acting_as(&clerk);
    t.app
        .get(&url)
        .await
        .assert_ok()
        .assert_see("On credit hold")
        .assert_see("The goods stay reserved")
        .assert_dont_see("over the credit limit")
        .assert_dont_see("AED 54,000.00");
}

/// Surface (back office): the held order appears in "orders on credit hold"
/// for the accountant and the rep, and a confirmed order does not.
#[renox::test]
async fn the_orders_on_credit_hold_are_listed_apart_from_the_others() {
    let (t, credit) = boot().await;
    let (order, _) = held_order(&t, &credit).await;
    let other = {
        credit.ledger.clear();
        let by = t
            .user("Sol", &[perm::SALES_ORDERS_VIEW, perm::SALES_ORDERS_CREATE])
            .await;
        t.app.acting_as(&by);
        confirmed(&t, &by).await
    };

    let viewer = t
        .user("Carla", &[perm::SALES_ORDERS_VIEW, perm::CREDIT_VIEW])
        .await;
    t.app.acting_as(&viewer);
    let all = t.app.get("/sales-orders").await;
    all.assert_ok()
        .assert_see(&order.number)
        .assert_see(&other.number)
        .assert_see("Orders on credit hold (1)");
    let only = t.app.get("/sales-orders?view=credit_hold").await;
    only.assert_ok()
        .assert_see(&order.number)
        .assert_dont_see(&other.number)
        .assert_see("The customer would be over the credit limit")
        .assert_see("All orders");
}

/// Surface (back office): the account page shows the terms, what the
/// customer owes and the orders on hold; the list links to it; both need
/// `credit.view`, and changing the terms needs `credit.limits.approve`.
#[renox::test]
async fn the_account_page_shows_what_the_customer_owes_against_the_limit() {
    let (t, credit) = boot().await;
    let (order, _) = held_order(&t, &credit).await;

    t.app.get("/credit/accounts").await.assert_forbidden();
    let viewer = t.user("Carla", &[perm::CREDIT_VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .get("/credit/accounts")
        .await
        .assert_ok()
        .assert_see("AED 50,000.00")
        .assert_see("AED 46,000.00")
        // The status column says something for an account in good standing.
        .assert_see("Account in good standing");
    let page = t.app.get(&format!("/credit/accounts/{CUSTOMER}")).await;
    page.assert_ok()
        .assert_see("AED 38,000.00")
        .assert_see("AED 8,000.00")
        .assert_see("Credit left")
        .assert_see("AED 4,000.00")
        .assert_see("30 days")
        .assert_see(&order.number)
        .assert_see("The customer would be over the credit limit");
    assert!(!page.text().contains("credit.exposure_title"));
    // A viewer cannot change the terms.
    t.app
        .post(
            "/credit/accounts",
            &[
                ("party_id", &CUSTOMER.to_string()),
                ("credit_limit", "60000"),
            ],
        )
        .await
        .assert_forbidden();

    let approver = t
        .user("Amal", &[perm::CREDIT_VIEW, perm::CREDIT_LIMITS_APPROVE])
        .await;
    t.app.acting_as(&approver);
    t.app
        .post(
            "/credit/accounts",
            &[
                ("party_id", &CUSTOMER.to_string()),
                ("credit_limit", "60,000.00"),
                ("payment_terms_days", "45"),
                ("grace_days", "10"),
            ],
        )
        .await
        .assert_status(303);
    t.app
        .get(&format!("/credit/accounts/{CUSTOMER}"))
        .await
        .assert_ok()
        .assert_see("AED 60,000.00")
        .assert_see("45 days")
        .assert_see("10 days");
    // A limit that is not an amount is sent back to the form and changes nothing.
    t.app
        .post(
            "/credit/accounts",
            &[
                ("party_id", &CUSTOMER.to_string()),
                ("credit_limit", "lots"),
            ],
        )
        .await
        .assert_status(303);
    t.app
        .get(&format!("/credit/accounts/{CUSTOMER}"))
        .await
        .assert_see("AED 60,000.00");
}

/// A new account for a customer is made from the list.
#[renox::test]
async fn an_account_is_made_for_a_customer_who_had_none() {
    let (t, _) = boot().await;
    let approver = t
        .user("Amal", &[perm::CREDIT_VIEW, perm::CREDIT_LIMITS_APPROVE])
        .await;
    t.app.acting_as(&approver);
    t.app
        .post(
            "/credit/accounts",
            &[("party_id", "21"), ("credit_limit", "5,000")],
        )
        .await
        .assert_status(303);
    t.app
        .get("/credit/accounts/21")
        .await
        .assert_ok()
        .assert_see("AED 5,000.00")
        .assert_see("No orders on credit hold");
    t.app.get("/credit/accounts/99").await.assert_not_found();
}

/// Every template of the module compiles with its texts.
#[renox::test]
async fn the_credit_templates_compile() {
    let (t, _) = boot().await;
    t.app.assert_views_compile();
}
