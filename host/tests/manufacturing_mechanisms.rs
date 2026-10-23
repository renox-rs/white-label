//! BL-MFG-14 (#395): the mechanisms that the 07 values drive, run for real
//! in the manufacturing deployment with the shared modules (the modules
//! test them with their own defaults; here the values are the 07 ones):
//! *Credit and cheques* (an invoice 31 days overdue holds the next order, a
//! bounced cheque holds the account and its orders, a cheque is presented
//! twice at most), *Stock* (a wastage of AED 2,100 and a return to a supplier
//! of AED 5,100 need approval, a transfer below zero is refused),
//! *Supplier scorecard*, *Budget and drawings*, *Notifications* (a recall
//! notice that gives up) and *Cancellation* (fee 0%).

mod support;

use host::roles::{ACCOUNTANT, ADMIN, SALES};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::PartyId;
use wl_kernel::cheques::{ChequeStatus, RegisterCheque, cheques};
use wl_kernel::invoicing::VatCategory;
use wl_payments::Cheque;
use wl_sales::customer_account::{self, CustomerAccount, Terms};
use wl_sales::{SalesOrder, SalesOrderStatus};

use support::{company, day, entity, issued_invoice, manufacturing_booted, person};

/// A draft order on account of `customer` for 10 panels, entered through the
/// pages of wl-sales and submitted; the order as stored after the submit.
async fn submitted(app: &TestApp, sales: &User, customer: PartyId) -> renox::testing::TestResponse {
    app.acting_as(sales);
    app.htmx()
        .post(
            "/sales-orders",
            &[
                ("party_id", &customer.0.to_string()),
                ("channel", "phone"),
                ("payment_mode", "on_account"),
            ],
        )
        .await
        .assert_status(303);
    let order = latest_order(app).await;
    app.htmx()
        .post(
            &format!("/sales-orders/{}/lines", order.id),
            &[
                ("item_code", "PANEL"),
                ("quantity", "10"),
                ("unit_price", "10.00"),
                ("vat", "standard"),
            ],
        )
        .await
        .assert_status(303);
    app.htmx()
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
}

async fn latest_order(app: &TestApp) -> SalesOrder {
    SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("an order")
}

async fn credit_for(app: &TestApp, customer: PartyId) {
    customer_account::save(app.state(), customer, Terms::default().limit(50_000_000))
        .await
        .unwrap();
}

async fn status_of(app: &TestApp, id: i64) -> SalesOrderStatus {
    SalesOrder::find_or_404(app.db(), id).await.unwrap().status
}

/// *Credit and cheques*: with grace 30 days an invoice 31 days past its due
/// date holds the customer's next order on account (`credit_hold`), while an
/// invoice 30 days past, or no invoice, lets the order through.
#[renox::test]
async fn an_invoice_31_days_overdue_holds_the_next_order_and_30_days_does_not() {
    let app = manufacturing_booted().await;
    let sales = person(&app, "Sara", SALES).await;
    let accountant = person(&app, "Alia", ADMIN).await;
    let late = company(&app, "Al Noor Trading LLC").await;
    let on_time = company(&app, "Bin Rashid Contracting LLC").await;
    for party in [late, on_time] {
        credit_for(&app, party).await;
    }
    issued_invoice(
        &app,
        &accountant,
        late,
        100_000,
        VatCategory::Standard,
        day(-31),
    )
    .await;
    issued_invoice(
        &app,
        &accountant,
        on_time,
        100_000,
        VatCategory::Standard,
        day(-30),
    )
    .await;

    submitted(&app, &sales, late).await.assert_status(303);
    assert_eq!(
        status_of(&app, latest_order(&app).await.id).await,
        SalesOrderStatus::CreditHold,
        "31 days overdue holds the order"
    );
    submitted(&app, &sales, on_time).await.assert_status(303);
    assert_ne!(
        status_of(&app, latest_order(&app).await.id).await,
        SalesOrderStatus::CreditHold,
        "30 days is within the grace"
    );
}

/// *Credit and cheques*: a bounced cheque holds the customer's account and
/// its order that is not dispatched; a new order on account is refused; the
/// cheque is presented a second time and when it bounces again the third
/// presentation is refused.
#[renox::test]
async fn a_bounced_cheque_holds_the_account_and_a_third_presentation_is_refused() {
    let app = manufacturing_booted().await;
    let sales = person(&app, "Sara", SALES).await;
    let accountant = person(&app, "Alia", ACCOUNTANT).await;
    let customer = company(&app, "Al Noor Trading LLC").await;
    credit_for(&app, customer).await;
    submitted(&app, &sales, customer).await.assert_status(303);
    let open_order = latest_order(&app).await;
    assert_eq!(
        status_of(&app, open_order.id).await,
        SalesOrderStatus::Confirmed
    );

    let epi = entity(&app, "EPI").await;
    let request = RegisterCheque::new(customer, epi.id, "ENBD", "3001", day(0), 2_000_000);
    let cheque = cheques(app.state())
        .register(app.state(), request, Some(accountant.id))
        .await
        .expect("the cheque is registered");
    let id = cheque.id.0;
    let act = |action: &'static str, form: Vec<(&'static str, String)>| {
        let (app, accountant) = (&app, &accountant);
        async move {
            app.acting_as(accountant);
            let pairs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
            app.htmx()
                .post(&format!("/cheques/{id}/{action}"), &pairs)
                .await
        }
    };
    let bounce = || {
        act(
            "bounce",
            vec![
                ("reason", "insufficient_funds".into()),
                ("certificate", "CERT-1".into()),
                ("honoured", String::new()),
            ],
        )
    };
    let represent = || {
        act(
            "represent",
            vec![
                ("date", day(1).format("%Y-%m-%d").to_string()),
                ("funds_confirmed", "on".into()),
                ("note", String::new()),
            ],
        )
    };
    act("deposit", vec![]).await.assert_ok();
    bounce().await.assert_ok();

    let account = CustomerAccount::of_party(app.db(), customer.0)
        .await
        .unwrap()
        .expect("the account");
    assert!(account.is_on_hold(app.db()).await.unwrap(), "account held");
    assert_eq!(
        status_of(&app, open_order.id).await,
        SalesOrderStatus::CreditHold,
        "the order not dispatched is held"
    );
    let refused = submitted(&app, &sales, customer).await;
    refused.assert_status(422);
    assert!(refused.text().contains("on hold"), "{}", refused.text());
    assert_eq!(
        status_of(&app, latest_order(&app).await.id).await,
        SalesOrderStatus::Draft,
        "the new order stays a draft"
    );

    represent().await.assert_ok();
    let second = Cheque::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(second.presentations, 2);
    bounce().await.assert_ok();
    represent().await.assert_status(422);
    let last = Cheque::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(
        (last.status, last.presentations),
        (ChequeStatus::Bounced, 2)
    );
}
