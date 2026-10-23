//! Helpers of the credit control tests: a seller, a rep and an accountant, a
//! customer account, and orders of one zero-rated line (so the value with VAT
//! is the price) entered through the services and submitted through the page.

#![allow(dead_code)]

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::invoicing::VatCategory;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::sales::{CreditVerdict, SalesOrderId, credit_checker};
use wl_sales::customer_account::{self, Terms};
use wl_sales::permissions as perm;
use wl_sales::sale_line::NewLine;
use wl_sales::sales_orders::{self, NewSalesOrder};
use wl_sales::{SalesOrder, SalesOrderStatus, WlSales};

use super::credit::Credit;
use super::{Booted, boot_configured};

/// The customer all orders are for.
pub const CUSTOMER: i64 = 7;

pub async fn boot() -> (Booted, Credit) {
    boot_with(WlSales::new()).await
}

pub async fn boot_with(sales: WlSales) -> (Booted, Credit) {
    let credit = Credit::default();
    let t = boot_configured(sales, credit.clone()).await;
    (t, credit)
}

/// A user who enters, submits and accepts orders.
pub async fn seller(t: &Booted) -> User {
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
pub async fn rep_and_accountant(t: &Booted) -> (User, User) {
    let rep = t.user("Rita", &[perm::SALES_ORDERS_VIEW]).await;
    let accountant = t.user("Ayesha", &[]).await;
    renox::auth::permissions::define_role(t.db(), "accountant", &[])
        .await
        .unwrap();
    accountant.assign_role(t.db(), "accountant").await.unwrap();
    (rep, accountant)
}

/// The customer's account with a limit of AED 50,000 and `rep` as its rep.
pub async fn account(t: &Booted, rep: &User) {
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
pub async fn draft(t: &Booted, by: &User, channel: &str, mode: &str, price: i64) -> SalesOrder {
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

pub async fn add_line(t: &Booted, id: i64, code: &str, price: i64) {
    sales_orders::add_line(
        t.state(),
        id,
        NewLine::new(code, price).vat_category(VatCategory::ZeroRated),
    )
    .await
    .unwrap();
}

pub async fn reload(t: &Booted, id: i64) -> SalesOrder {
    SalesOrder::find_or_404(t.db(), id).await.unwrap()
}

/// Submits the draft `id` through the page, as the seller.
pub async fn submit(t: &Booted, id: i64) {
    t.app
        .post(&format!("/sales-orders/{id}/submit"), &[("x", "")])
        .await
        .assert_status(303);
}

/// A phone order of AED 8,000 on account, submitted while the customer owes
/// nothing, so it is `confirmed`.
pub async fn confirmed(t: &Booted, by: &User) -> SalesOrder {
    t.app.acting_as(by);
    let order = draft(t, by, "phone", "on_account", 800_000).await;
    submit(t, order.id).await;
    let order = reload(t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::Confirmed);
    order
}

pub async fn recheck(t: &Booted, order: &SalesOrder) -> CreditVerdict {
    credit_checker(t.state())
        .credit_recheck(t.state(), SalesOrderId(order.id))
        .await
        .unwrap()
}

pub fn users_told(request: &wl_kernel::notify::NotifyRequest) -> Vec<i64> {
    request
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}
