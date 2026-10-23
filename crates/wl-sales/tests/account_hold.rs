//! BL-SAL-12 (#131): a bounced cheque puts the customer account (and its
//! credit group) on hold and holds its orders on account that are not yet
//! dispatched (`SAL.RUL.05`); new orders and counter sales on account are
//! refused; *Clear account hold* needs another credit releaser (`SAL.ACL.03`).

mod support;

use renox::prelude::*;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::cheques::{ChequeBounced, ChequeId};
use wl_kernel::invoicing::VatCategory;
use wl_kernel::sales::{FulfilOrder, FulfilledLine, SalesOrderId, order_fulfilment};
use wl_kernel::{PartyId, Subject};
use wl_sales::customer_account::{self, CustomerAccount, Terms};
use wl_sales::permissions as perm;
use wl_sales::sale_line::NewLine;
use wl_sales::sales_orders::{self, NewSalesOrder};
use wl_sales::{
    AccountHoldClearing, AccountHoldSource, SaleLine, SalesOrder, SalesOrderStatus, WlSales,
};

use support::Booted;
use support::sell::{
    CUSTOMER, add_line, boot, boot_with, confirmed, draft, reload, rep_and_accountant, seller,
    submit, users_told,
};

/// The sister company of the customer, in the same credit group.
const SISTER: i64 = 8;

/// The accounts of the customer and its sister in one credit group.
async fn group(t: &Booted, rep: &User) {
    let group = customer_account::save_group(t.state(), PartyId(500), "Gulf Holdings", None)
        .await
        .unwrap();
    for party in [CUSTOMER, SISTER] {
        customer_account::save(
            t.state(),
            PartyId(party),
            Terms::default()
                .limit(50_000_000)
                .group(group.id)
                .rep(rep.id),
        )
        .await
        .unwrap();
    }
}

/// The cheque `n` of `party` bounces.
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

async fn account_of(t: &Booted, party: i64) -> CustomerAccount {
    CustomerAccount::of_party(t.db(), party)
        .await
        .unwrap()
        .expect("the customer has an account")
}

async fn on_hold(t: &Booted, party: i64) -> bool {
    account_of(t, party).await.is_on_hold(t.db()).await.unwrap()
}

/// A user who may release held orders and clear account holds.
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

/// A confirmed order of AED 8,000 on account and a partly fulfilled one with
/// a second line still to dispatch, both for the customer.
async fn two_open_orders(t: &Booted, maker: &User) -> (SalesOrder, SalesOrder) {
    let first = confirmed(t, maker).await;
    let second = draft(t, maker, "phone", "on_account", 800_000).await;
    add_line(t, second.id, "valve", 200_000).await;
    submit(t, second.id).await;
    let lines = SaleLine::of(t.db(), &second.subject()).await.unwrap();
    order_fulfilment(t.state())
        .fulfil(
            t.state(),
            FulfilOrder {
                order: SalesOrderId(second.id),
                lines: vec![FulfilledLine {
                    line: lines[0].id,
                    quantity_milli: 1000,
                }],
                source: Subject::new("inventory.dispatch", 1),
            },
        )
        .await
        .unwrap();
    let second = reload(t, second.id).await;
    assert_eq!(second.status, SalesOrderStatus::PartiallyFulfilled);
    (first, second)
}

/// AC1: customer C in group G with member D (whole group on): O1 `confirmed`
/// and O2 `partially_fulfilled` with lines not dispatched; the cheque of C
/// bounces: C and D are on hold with the cheque as source, O1 and O2 are on
/// `credit_hold`, the accountant and the rep are told.
#[renox::test]
async fn a_bounced_cheque_holds_the_account_its_group_and_the_orders_not_yet_dispatched() {
    let (t, credit) = boot().await;
    let (rep, accountant) = rep_and_accountant(&t).await;
    group(&t, &rep).await;
    let maker = seller(&t).await;
    t.app.acting_as(&maker);
    let (o1, o2) = two_open_orders(&t, &maker).await;
    // Untouched: an order delivered in full and an order paid online.
    let delivered = confirmed(&t, &maker).await;
    let lines = SaleLine::of(t.db(), &delivered.subject()).await.unwrap();
    order_fulfilment(t.state())
        .fulfil(
            t.state(),
            FulfilOrder {
                order: SalesOrderId(delivered.id),
                lines: vec![FulfilledLine {
                    line: lines[0].id,
                    quantity_milli: 1000,
                }],
                source: Subject::new("inventory.dispatch", 2),
            },
        )
        .await
        .unwrap();
    let prepaid = draft(&t, &maker, "phone", "online", 800_000).await;
    submit(&t, prepaid.id).await;
    sales_orders::confirm_payment(t.state(), prepaid.id, 800_000)
        .await
        .unwrap();

    bounce(&t, CUSTOMER, 31).await;

    assert!(on_hold(&t, CUSTOMER).await);
    assert!(on_hold(&t, SISTER).await);
    for party in [CUSTOMER, SISTER] {
        let sources = account_of(&t, party)
            .await
            .open_sources(t.db())
            .await
            .unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].reason, "bounced_cheque");
        assert_eq!(sources[0].source(), Subject::new("payments.cheque", 31));
    }
    let (h1, h2) = (reload(&t, o1.id).await, reload(&t, o2.id).await);
    assert_eq!(h1.status, SalesOrderStatus::CreditHold);
    assert_eq!(h1.state_before_hold.as_deref(), Some("confirmed"));
    assert_eq!(h1.hold_reason.as_deref(), Some("account_on_hold"));
    assert_eq!(h2.status, SalesOrderStatus::CreditHold);
    assert_eq!(h2.state_before_hold.as_deref(), Some("partially_fulfilled"));
    assert_eq!(
        reload(&t, delivered.id).await.status,
        SalesOrderStatus::Fulfilled
    );
    assert_eq!(
        reload(&t, prepaid.id).await.status,
        SalesOrderStatus::Confirmed
    );
    assert_eq!(credit.held.lock().unwrap().len(), 2);
    let told = credit.mail.of_type("sales.order_credit_hold");
    assert_eq!(told.len(), 2);
    let mut users = users_told(&told[0]);
    users.sort_unstable();
    let mut expected = vec![rep.id, accountant.id];
    expected.sort_unstable();
    assert_eq!(users, expected);
    // The account itself: one notice per account put on hold.
    assert_eq!(credit.mail.of_type("sales.account_on_hold").len(), 2);
}

/// The same `ChequeBounced` twice adds one source; a second cheque adds
/// another; a drawer without an account changes nothing; with the group
/// setting off only the drawer is held.
#[renox::test]
async fn the_sources_are_idempotent_per_cheque_and_the_group_follows_the_setting() {
    let (t, credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    group(&t, &rep).await;
    bounce(&t, CUSTOMER, 31).await;
    bounce(&t, CUSTOMER, 31).await;
    assert_eq!(
        account_of(&t, CUSTOMER)
            .await
            .open_sources(t.db())
            .await
            .unwrap()
            .len(),
        1
    );
    bounce(&t, CUSTOMER, 32).await;
    assert_eq!(
        account_of(&t, CUSTOMER)
            .await
            .open_sources(t.db())
            .await
            .unwrap()
            .len(),
        2
    );
    // Nobody with that party has an account.
    bounce(&t, 99, 40).await;
    assert!(AccountHoldSource::query().get(t.db()).await.unwrap().len() == 4);
    assert_eq!(credit.mail.of_type("sales.account_on_hold").len(), 4);

    let (t, _) = boot_with(WlSales::new().bounced_cheque_hold_group(false)).await;
    let (rep, _) = rep_and_accountant(&t).await;
    group(&t, &rep).await;
    bounce(&t, CUSTOMER, 31).await;
    assert!(on_hold(&t, CUSTOMER).await);
    assert!(!on_hold(&t, SISTER).await);
}

/// AC2: a new order on account for D is refused at `submit` with "account on
/// hold" and stays `draft` (no `credit_hold` order is made); the same order
/// paid by card is submitted; a counter sale on account for C is refused,
/// one in cash is not.
#[renox::test]
async fn new_orders_and_counter_sales_on_account_are_refused_while_the_account_is_on_hold() {
    let (t, _credit) = boot_with(WlSales::new().counter_sales_on_account(true)).await;
    let (rep, _) = rep_and_accountant(&t).await;
    group(&t, &rep).await;
    bounce(&t, CUSTOMER, 31).await;
    let maker = t
        .user(
            "Sam",
            &[
                perm::SALES_ORDERS_VIEW,
                perm::SALES_ORDERS_CREATE,
                perm::POS_SELL,
            ],
        )
        .await;
    t.app.acting_as(&maker);

    // The sister company (D): the order on account is refused...
    let order = sales_orders::create(
        t.state(),
        Some(maker.id),
        NewSalesOrder::new(PartyId(SISTER)),
    )
    .await
    .unwrap();
    sales_orders::add_line(
        t.state(),
        order.id,
        NewLine::new("pump", 800_000).vat_category(VatCategory::ZeroRated),
    )
    .await
    .unwrap();
    let refused = t
        .app
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Account on hold"),
        "{}",
        refused.text()
    );
    assert_eq!(reload(&t, order.id).await.status, SalesOrderStatus::Draft);
    assert_eq!(
        SalesOrder::query()
            .where_(SalesOrder::STATUS.eq(SalesOrderStatus::CreditHold))
            .get(t.db())
            .await
            .unwrap()
            .len(),
        0
    );
    // ...and the same order paid by card goes through.
    let by_card = sales_orders::create(
        t.state(),
        Some(maker.id),
        NewSalesOrder::new(PartyId(SISTER)).payment_mode("card"),
    )
    .await
    .unwrap();
    sales_orders::add_line(
        t.state(),
        by_card.id,
        NewLine::new("pump", 800_000).vat_category(VatCategory::ZeroRated),
    )
    .await
    .unwrap();
    t.app
        .post(
            &format!("/sales-orders/{}/submit", by_card.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    assert_eq!(
        reload(&t, by_card.id).await.status,
        SalesOrderStatus::Confirmed
    );

    // A counter sale on account for C.
    t.app
        .post("/counter-sales", &[("party_id", &CUSTOMER.to_string())])
        .await
        .assert_status(303);
    let sale = wl_sales::CounterSale::query()
        .order_by_desc(wl_sales::CounterSale::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    t.app
        .post(
            &format!("/counter-sales/{}/lines", sale.id),
            &[("item_code", "tee"), ("unit_price", "100.00")],
        )
        .await
        .assert_status(303);
    let refused = t
        .app
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("account", "105.00")],
        )
        .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Account on hold"),
        "{}",
        refused.text()
    );
    t.app
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("cash", "105.00")],
        )
        .await
        .assert_status(303);
}

/// AC3: an order in `pending_acceptance` when the cheque bounced is not held;
/// its `accept` sends it to `credit_hold`.
#[renox::test]
async fn an_order_waiting_for_acceptance_is_held_by_its_acceptance() {
    let (t, credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    group(&t, &rep).await;
    let maker = seller(&t).await;
    t.app.acting_as(&maker);
    let order = draft(&t, &maker, "website", "on_account", 800_000).await;
    submit(&t, order.id).await;
    bounce(&t, CUSTOMER, 31).await;
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
    assert_eq!(held.hold_reason.as_deref(), Some("account_on_hold"));
    assert!(credit.confirmed.lock().unwrap().is_empty());
}

/// The first source of C: its id.
async fn first_source(t: &Booted, party: i64) -> i64 {
    account_of(t, party)
        .await
        .open_sources(t.db())
        .await
        .unwrap()[0]
        .id
}

async fn clearing_request(t: &Booted, clearing: &AccountHoldClearing) -> ApprovalRequest {
    ApprovalRequest::find(t.db(), clearing.approval_request_id.unwrap())
        .await
        .unwrap()
        .unwrap()
}

/// AC4: user B asks to clear the hold of C with the reason *cheque replaced*
/// and E approves: the flags from that cheque go from C and D; O1 stays on
/// credit hold until it is released.
#[renox::test]
async fn clearing_the_hold_with_a_second_releasers_approval_clears_the_group_but_not_the_orders() {
    let (t, _credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    group(&t, &rep).await;
    let maker = seller(&t).await;
    t.app.acting_as(&maker);
    let o1 = confirmed(&t, &maker).await;
    bounce(&t, CUSTOMER, 31).await;
    bounce(&t, CUSTOMER, 32).await;
    let (b, e) = (releaser(&t, "Bina").await, releaser(&t, "Essa").await);

    let source = first_source(&t, CUSTOMER).await;
    t.app.acting_as(&b);
    t.app
        .post(
            &format!("/credit/accounts/{CUSTOMER}/clear-hold"),
            &[
                ("source", &source.to_string()),
                ("kind", "cheque_replaced"),
                ("note", "New cheque 1099 received"),
            ],
        )
        .await
        .assert_status(303);
    let clearing = AccountHoldClearing::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(clearing.reason_kind, "cheque_replaced");
    // Nothing is cleared until the second releaser approves.
    assert!(on_hold(&t, CUSTOMER).await);
    let request = clearing_request(&t, &clearing).await;
    assert_eq!(request.requester_id, b.id);
    // A second request for the same hold is refused while one waits.
    t.app
        .post(
            &format!("/credit/accounts/{CUSTOMER}/clear-hold"),
            &[("source", &source.to_string()), ("kind", "balance_paid")],
        )
        .await
        .assert_status(422);
    approvals::approve(t.state(), request.id, &e, "Cheque is good")
        .await
        .unwrap();

    // The first cheque's flags are gone from C and D; the second cheque still
    // holds both.
    let c_open = account_of(&t, CUSTOMER)
        .await
        .open_sources(t.db())
        .await
        .unwrap();
    assert_eq!(c_open.len(), 1);
    assert_eq!(c_open[0].source(), Subject::new("payments.cheque", 32));
    let d_open = account_of(&t, SISTER)
        .await
        .open_sources(t.db())
        .await
        .unwrap();
    assert_eq!(d_open.len(), 1);
    assert_eq!(d_open[0].source(), Subject::new("payments.cheque", 32));
    let cleared = AccountHoldSource::find(t.db(), source)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cleared.cleared_by, Some(e.id));
    assert!(cleared.clearing_id.is_some());
    assert_eq!(reload(&t, o1.id).await.status, SalesOrderStatus::CreditHold);

    // The second cheque is cleared the same way: now the flag is gone.
    let second = first_source(&t, CUSTOMER).await;
    t.app.acting_as(&b);
    t.app
        .post(
            &format!("/credit/accounts/{CUSTOMER}/clear-hold"),
            &[("source", &second.to_string()), ("kind", "balance_paid")],
        )
        .await
        .assert_status(303);
    let again = AccountHoldClearing::query()
        .order_by_desc(AccountHoldClearing::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    let request = clearing_request(&t, &again).await;
    approvals::approve(t.state(), request.id, &e, "")
        .await
        .unwrap();
    assert!(!on_hold(&t, CUSTOMER).await);
    assert!(!on_hold(&t, SISTER).await);
    // The held order is released by its own action, which still needs the
    // approval of another releaser.
    assert_eq!(reload(&t, o1.id).await.status, SalesOrderStatus::CreditHold);
    t.app
        .post(
            &format!("/sales-orders/{}/release", o1.id),
            &[("note", "Hold cleared")],
        )
        .await
        .assert_status(303);
    let request = ApprovalRequest::pending_about(t.db(), &o1.subject())
        .await
        .unwrap()
        .remove(0);
    approvals::approve(t.state(), request.id, &e, "")
        .await
        .unwrap();
    assert_eq!(reload(&t, o1.id).await.status, SalesOrderStatus::Confirmed);
}

/// AC5: user B cannot approve their own clearing request, even holding every
/// permission; a rejection keeps the hold; a clearing needs a known reason
/// and a note for *other*, and the permission.
#[renox::test]
async fn the_requester_cannot_approve_their_own_clearing_and_a_rejection_keeps_the_hold() {
    let (t, _credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    group(&t, &rep).await;
    bounce(&t, CUSTOMER, 31).await;
    let admin = t
        .user(
            "Bina",
            &[
                perm::SALES_ORDERS_VIEW,
                perm::CREDIT_VIEW,
                perm::CREDIT_RELEASE,
                perm::CREDIT_LIMITS_APPROVE,
                perm::SALES_ORDERS_CREATE,
                perm::SALES_ORDERS_CANCEL,
            ],
        )
        .await;
    let source = first_source(&t, CUSTOMER).await;
    let url = format!("/credit/accounts/{CUSTOMER}/clear-hold");
    let form = |kind: &str, note: &str| {
        vec![
            ("source".to_owned(), source.to_string()),
            ("kind".to_owned(), kind.to_owned()),
            ("note".to_owned(), note.to_owned()),
        ]
    };
    let post = |fields: Vec<(String, String)>| {
        let t = &t;
        let url = url.clone();
        async move {
            let pairs: Vec<(&str, &str)> = fields
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            t.app.post(&url, &pairs).await
        }
    };

    // Without `credit.release` there is no clearing.
    let clerk = t.user("Clara", &[perm::CREDIT_VIEW]).await;
    t.app.acting_as(&clerk);
    post(form("cheque_replaced", "")).await.assert_forbidden();

    t.app.acting_as(&admin);
    post(form("because", "")).await.assert_status(422);
    post(form("other", "")).await.assert_status(422);
    post(form("other", "Paid in cash at the counter"))
        .await
        .assert_status(303);
    let clearing = AccountHoldClearing::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    let request = clearing_request(&t, &clearing).await;
    assert!(
        approvals::approve(t.state(), request.id, &admin, "")
            .await
            .is_err()
    );
    assert!(on_hold(&t, CUSTOMER).await);

    let e = releaser(&t, "Essa").await;
    approvals::reject(t.state(), request.id, &e, "Not proven")
        .await
        .unwrap();
    assert!(on_hold(&t, CUSTOMER).await);
    let rejected = AccountHoldClearing::find_or_404(t.db(), clearing.id)
        .await
        .unwrap();
    assert_eq!(rejected.status, wl_sales::ClearingStatus::Rejected);
    // After a rejection a new request can be made.
    t.app.acting_as(&admin);
    post(form("balance_paid", "")).await.assert_status(303);
}

/// Surface (back office): the account page lists why the account is on hold
/// to a viewer, offers *Clear account hold* to a releaser, shows the pending
/// request as a card with the approver's buttons, and the release sheet of a
/// held order warns and offers to ask for the clearing in the same action.
#[renox::test]
async fn the_screens_show_the_hold_and_offer_its_clearing_with_the_release() {
    let (t, _credit) = boot().await;
    let (rep, _) = rep_and_accountant(&t).await;
    group(&t, &rep).await;
    let maker = seller(&t).await;
    t.app.acting_as(&maker);
    let order = confirmed(&t, &maker).await;
    bounce(&t, CUSTOMER, 31).await;
    let page = format!("/credit/accounts/{CUSTOMER}");

    let viewer = t.user("Vera", &[perm::CREDIT_VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("Account on hold")
        .assert_see("Bounced cheque no. 31")
        .assert_dont_see("Clear account hold");

    let (b, e) = (releaser(&t, "Bina").await, releaser(&t, "Essa").await);
    t.app.acting_as(&b);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("Clear account hold");
    // The order's release sheet warns and offers the clearing too.
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("The customer account is on hold")
        .assert_see("Also ask to clear the account hold");
    t.app
        .post(
            &format!("/sales-orders/{}/release", order.id),
            &[
                ("note", "Customer called"),
                ("clear_hold", "on"),
                ("kind", "cheque_replaced"),
                ("clear_note", "New cheque"),
            ],
        )
        .await
        .assert_status(303);
    // Both requests wait: the release of the order and the clearing of the
    // customer's one source.
    assert_eq!(
        AccountHoldClearing::query()
            .get(t.db())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        ApprovalRequest::pending_about(t.db(), &order.subject())
            .await
            .unwrap()
            .len(),
        1
    );
    let clearing = AccountHoldClearing::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    let request = clearing_request(&t, &clearing).await;
    let approve = format!("/approvals/{}/approve", request.id);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("A clearing is waiting for approval.")
        .assert_dont_see(&approve);
    t.app.acting_as(&e);
    t.app.get(&page).await.assert_ok().assert_see(&approve);
    // The approval card's link leads to the account.
    t.app
        .get(&format!("/credit/clearings/{}", clearing.id))
        .await
        .assert_redirect(&page);
}

/// Every template of the module compiles with its texts.
#[renox::test]
async fn the_templates_still_compile() {
    let (t, _) = boot().await;
    t.app.assert_views_compile();
}
