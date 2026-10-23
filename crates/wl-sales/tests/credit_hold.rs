//! BL-SAL-10 (#129): the credit check of a sales order on account at
//! `submit`, `accept` and at every dispatch (`credit_recheck`), and the
//! `credit_hold` it leads to (`SAL.CRD.01`, `SAL.ACL.03`, `SAL.RUL.01`).

mod support;

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::invoicing::VatCategory;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::sales::{
    CreditFailure, CreditVerdict, FulfilOrder, FulfilledLine, SalesOrderId, credit_checker,
    order_fulfilment,
};
use wl_sales::customer_account::{self, Terms};
use wl_sales::permissions as perm;
use wl_sales::sale_line::NewLine;
use wl_sales::sales_orders::{self, NewSalesOrder};
use wl_sales::{SaleLine, SalesOrder, SalesOrderStatus, WlSales};

use support::credit::Credit;
use support::{Booted, boot_configured};

/// The customer all orders are for.
const CUSTOMER: i64 = 7;

async fn boot() -> (Booted, Credit) {
    boot_with(WlSales::new()).await
}

async fn boot_with(sales: WlSales) -> (Booted, Credit) {
    let credit = Credit::default();
    let t = boot_configured(sales, credit.clone()).await;
    (t, credit)
}

/// A user who enters, submits and accepts orders.
async fn seller(t: &Booted) -> User {
    t.user(
        "Sam",
        &[
            perm::SALES_ORDERS_VIEW,
            perm::SALES_ORDERS_CREATE,
            perm::SALES_ORDERS_ACCEPT,
            perm::SALES_ORDERS_CANCEL,
        ],
    )
    .await
}

/// The customer's sales rep and an accountant (the holder of the
/// `accountant` role).
async fn rep_and_accountant(t: &Booted) -> (User, User) {
    let rep = t.user("Rita", &[perm::SALES_ORDERS_VIEW]).await;
    let accountant = t.user("Ayesha", &[]).await;
    renox::auth::permissions::define_role(t.db(), "accountant", &[])
        .await
        .unwrap();
    accountant.assign_role(t.db(), "accountant").await.unwrap();
    (rep, accountant)
}

/// The customer's account with a limit of AED 50,000 and `rep` as its rep.
async fn account(t: &Booted, rep: &User) {
    customer_account::save(
        t.state(),
        PartyId(CUSTOMER),
        Terms::default().limit(5_000_000).rep(rep.id),
    )
    .await
    .unwrap();
}

/// A draft order for the customer with one zero-rated line of `price` fils
/// (so the value with VAT is the price).
async fn draft(t: &Booted, by: &User, channel: &str, mode: &str, price: i64) -> SalesOrder {
    let order = sales_orders::create(
        t.state(),
        Some(by.id),
        NewSalesOrder::new(PartyId(CUSTOMER))
            .channel(channel)
            .payment_mode(mode),
    )
    .await
    .unwrap();
    add_line(t, order.id, "pump", price).await;
    reload(t, order.id).await
}

async fn add_line(t: &Booted, id: i64, code: &str, price: i64) {
    sales_orders::add_line(
        t.state(),
        id,
        NewLine::new(code, price).vat_category(VatCategory::ZeroRated),
    )
    .await
    .unwrap();
}

async fn reload(t: &Booted, id: i64) -> SalesOrder {
    SalesOrder::find_or_404(t.db(), id).await.unwrap()
}

/// Submits the draft `id` through the page, as the seller.
async fn submit(t: &Booted, id: i64) {
    t.app
        .post(&format!("/sales-orders/{id}/submit"), &[("x", "")])
        .await
        .assert_status(303);
}

/// A phone order of AED 8,000 on account, submitted while the customer owes
/// nothing, so it is `confirmed`.
async fn confirmed(t: &Booted, by: &User) -> SalesOrder {
    t.app.acting_as(by);
    let order = draft(t, by, "phone", "on_account", 800_000).await;
    submit(t, order.id).await;
    let order = reload(t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::Confirmed);
    order
}

async fn recheck(t: &Booted, order: &SalesOrder) -> CreditVerdict {
    credit_checker(t.state())
        .credit_recheck(t.state(), SalesOrderId(order.id))
        .await
        .unwrap()
}

fn users_told(request: &wl_kernel::notify::NotifyRequest) -> Vec<i64> {
    request
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

/// AC1: AED 46,000 owed (invoices 38,000 + an uncleared cheque 8,000) and an
/// undelivered order of 8,000 against a limit of 50,000: the dispatch check
/// counts 54,000, fails, and holds the order; the accountant and the rep are
/// told.
#[renox::test]
async fn a_dispatch_over_the_limit_holds_the_order_and_tells_the_accountant_and_the_rep() {
    let (t, credit) = boot().await;
    let (rep, accountant) = rep_and_accountant(&t).await;
    account(&t, &rep).await;
    let order = confirmed(&t, &seller(&t).await).await;

    credit.ledger.invoice(CUSTOMER, 3_800_000, 10);
    credit.ledger.cheque(CUSTOMER, 800_000);
    let verdict = recheck(&t, &order).await;

    assert_eq!(verdict, CreditVerdict::Failed(CreditFailure::Limit));
    let held = reload(&t, order.id).await;
    assert_eq!(held.status, SalesOrderStatus::CreditHold);
    assert_eq!(held.state_before_hold.as_deref(), Some("confirmed"));
    assert_eq!(held.hold_reason.as_deref(), Some("limit"));
    assert_eq!(held.hold_open_balance, Some(5_400_000));
    assert_eq!(held.hold_limit, Some(5_000_000));
    let events = credit.held.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].order, SalesOrderId(order.id));
    assert_eq!(events[0].open_balance, 5_400_000);
    assert_eq!(events[0].reason, CreditFailure::Limit);
    let told = credit.mail.of_type("sales.order_credit_hold");
    assert_eq!(told.len(), 1);
    let mut users = users_told(&told[0]);
    users.sort_unstable();
    let mut expected = vec![rep.id, accountant.id];
    expected.sort_unstable();
    assert_eq!(users, expected);
    // The goods stay reserved: nothing cancelled the order or its lines.
    let lines = SaleLine::of(t.db(), &held.subject()).await.unwrap();
    assert_eq!(lines[0].cancelled_milli, 0);
}

/// AC2: AED 40,000 owed counts 48,000 with the order: it passes and the
/// dispatch goes on.
#[renox::test]
async fn a_dispatch_within_the_limit_passes_and_the_order_stays_confirmed() {
    let (t, credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    account(&t, &rep).await;
    let order = confirmed(&t, &seller(&t).await).await;

    credit.ledger.invoice(CUSTOMER, 4_000_000, 10);

    assert_eq!(recheck(&t, &order).await, CreditVerdict::Passed);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );
    assert!(credit.held.lock().unwrap().is_empty());
}

/// AC3: an invoice 45 days past its due date, grace 30, fails the check.
#[renox::test]
async fn an_invoice_overdue_beyond_the_grace_days_holds_the_order() {
    let (t, credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    account(&t, &rep).await;
    let order = confirmed(&t, &seller(&t).await).await;

    // 20 days overdue is inside the grace.
    credit.ledger.invoice(CUSTOMER, 100_000, -20);
    assert_eq!(recheck(&t, &order).await, CreditVerdict::Passed);

    credit.ledger.invoice(CUSTOMER, 100_000, -45);
    assert_eq!(
        recheck(&t, &order).await,
        CreditVerdict::Failed(CreditFailure::Overdue)
    );
    let held = reload(&t, order.id).await;
    assert_eq!(held.status, SalesOrderStatus::CreditHold);
    assert_eq!(held.hold_reason.as_deref(), Some("overdue"));
    assert_eq!(held.hold_overdue, Some(100_000));
}

/// AC4: the on-hold flag of the account fails the check.
#[renox::test]
async fn an_account_on_hold_fails_the_check() {
    let (t, credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    account(&t, &rep).await;
    let order = confirmed(&t, &seller(&t).await).await;

    let customer = customer_account::CustomerAccount::of_party(t.db(), CUSTOMER)
        .await
        .unwrap()
        .unwrap();
    customer_account::add_source(
        t.db(),
        &customer,
        &wl_kernel::Subject::new("payments.cheque", 3),
        customer_account::REASON_BOUNCED_CHEQUE,
    )
    .await
    .unwrap();

    assert_eq!(
        recheck(&t, &order).await,
        CreditVerdict::Failed(CreditFailure::AccountOnHold)
    );
    assert_eq!(
        reload(&t, order.id).await.hold_reason.as_deref(),
        Some("account_on_hold")
    );
    assert_eq!(credit.held.lock().unwrap().len(), 1);
}

/// AC5: a partly fulfilled order keeps `partially_fulfilled` as the state
/// before the hold.
#[renox::test]
async fn an_order_held_while_partly_fulfilled_remembers_that_state() {
    let (t, credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    account(&t, &rep).await;
    let by = seller(&t).await;
    t.app.acting_as(&by);
    let order = draft(&t, &by, "phone", "on_account", 800_000).await;
    add_line(&t, order.id, "valve", 200_000).await;
    submit(&t, order.id).await;
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
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::PartiallyFulfilled
    );

    credit.ledger.invoice(CUSTOMER, 5_000_000, 10);
    let order = reload(&t, order.id).await;
    assert_eq!(
        recheck(&t, &order).await,
        CreditVerdict::Failed(CreditFailure::Limit)
    );
    let held = reload(&t, order.id).await;
    assert_eq!(held.status, SalesOrderStatus::CreditHold);
    assert_eq!(
        held.state_before_hold.as_deref(),
        Some("partially_fulfilled")
    );
    // Only what is not yet delivered counts as the order's value.
    assert_eq!(held.hold_open_balance, Some(5_200_000));
}

/// AC6: a prepaid order is not re-checked, whatever the customer owes.
#[renox::test]
async fn a_prepaid_order_is_not_checked() {
    let (t, credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    account(&t, &rep).await;
    let by = seller(&t).await;
    t.app.acting_as(&by);
    let order = draft(&t, &by, "phone", "online", 800_000).await;
    submit(&t, order.id).await;
    sales_orders::confirm_payment(t.state(), order.id, 800_000)
        .await
        .unwrap();
    credit.ledger.invoice(CUSTOMER, 9_000_000, -90);

    let order = reload(&t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::Confirmed);
    assert_eq!(recheck(&t, &order).await, CreditVerdict::NotApplicable);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );
    assert!(credit.held.lock().unwrap().is_empty());
}

/// The check at `submit`: an order that does not fit goes to `credit_hold`
/// from the draft, never confirmed, and tells the same people.
#[renox::test]
async fn an_order_over_the_limit_at_submit_goes_on_hold_unconfirmed() {
    let (t, credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    account(&t, &rep).await;
    credit.ledger.invoice(CUSTOMER, 4_600_000, 10);
    let by = seller(&t).await;
    t.app.acting_as(&by);
    let order = draft(&t, &by, "phone", "on_account", 800_000).await;
    submit(&t, order.id).await;

    let held = reload(&t, order.id).await;
    assert_eq!(held.status, SalesOrderStatus::CreditHold);
    assert_eq!(held.state_before_hold.as_deref(), Some("confirmed"));
    assert!(held.confirmed_at.is_none());
    assert!(credit.confirmed.lock().unwrap().is_empty());
    assert_eq!(credit.held.lock().unwrap().len(), 1);
}

/// The check at `accept`: a website order waits for the seller, whose
/// acceptance runs the check; the generic transition route cannot skip it.
#[renox::test]
async fn accepting_an_order_over_the_limit_puts_it_on_hold_and_the_generic_route_cannot_skip_it() {
    let (t, credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    account(&t, &rep).await;
    let by = seller(&t).await;
    t.app.acting_as(&by);
    let order = draft(&t, &by, "website", "on_account", 800_000).await;
    submit(&t, order.id).await;
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::PendingAcceptance
    );
    credit.ledger.invoice(CUSTOMER, 4_600_000, 10);

    t.app
        .post(
            &format!("/sales-orders/{}/transitions/accept", order.id),
            &[("x", "")],
        )
        .await
        .assert_status(422);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::PendingAcceptance
    );

    t.app
        .post(&format!("/sales-orders/{}/accept", order.id), &[("x", "")])
        .await
        .assert_status(303);
    let held = reload(&t, order.id).await;
    assert_eq!(held.status, SalesOrderStatus::CreditHold);
    assert_eq!(held.state_before_hold.as_deref(), Some("confirmed"));
    assert!(credit.confirmed.lock().unwrap().is_empty());
}

/// A customer without an account is not checked, and the setting turns the
/// re-check off.
#[renox::test]
async fn no_account_or_the_setting_off_means_no_check() {
    let (t, credit) = boot().await;
    let order = confirmed(&t, &seller(&t).await).await;
    credit.ledger.invoice(CUSTOMER, 9_000_000, -90);
    assert_eq!(recheck(&t, &order).await, CreditVerdict::NotApplicable);

    let (t, credit) = boot_with(WlSales::new().credit_recheck_at_dispatch(false)).await;
    let (rep, _) = rep_and_accountant(&t).await;
    account(&t, &rep).await;
    let order = confirmed(&t, &seller(&t).await).await;
    credit.ledger.invoice(CUSTOMER, 9_000_000, -90);
    assert_eq!(recheck(&t, &order).await, CreditVerdict::NotApplicable);
}

/// A credit group shares a combined limit: another member's exposure counts.
#[renox::test]
async fn a_credit_group_checks_the_combined_limit() {
    let (t, credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    let group =
        customer_account::save_group(t.state(), PartyId(500), "Gulf Holdings", Some(6_000_000))
            .await
            .unwrap();
    for party in [CUSTOMER, 8] {
        customer_account::save(
            t.state(),
            PartyId(party),
            Terms::default()
                .limit(5_000_000)
                .group(group.id)
                .rep(rep.id),
        )
        .await
        .unwrap();
    }
    let order = confirmed(&t, &seller(&t).await).await;
    // The sister company owes AED 55,000, over its own limit of 50,000; the
    // group is at 55,000 + 8,000 against its combined 60,000.
    credit.ledger.invoice(8, 5_500_000, 10);
    assert_eq!(
        recheck(&t, &order).await,
        CreditVerdict::Failed(CreditFailure::Limit)
    );
}

/// The kernel's `check_amount` lets a vertical ask without an order and
/// changes nothing.
#[renox::test]
async fn an_amount_can_be_checked_without_an_order() {
    let (t, credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    account(&t, &rep).await;
    credit.ledger.invoice(CUSTOMER, 4_000_000, 10);
    let checker = credit_checker(t.state());
    assert_eq!(
        checker
            .check_amount(t.state(), PartyId(CUSTOMER), 1_000_000)
            .await
            .unwrap(),
        CreditVerdict::Passed
    );
    assert_eq!(
        checker
            .check_amount(t.state(), PartyId(CUSTOMER), 1_000_001)
            .await
            .unwrap(),
        CreditVerdict::Failed(CreditFailure::Limit)
    );
    assert!(credit.held.lock().unwrap().is_empty());
}
