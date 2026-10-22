//! Helpers of the payout statement and report tests: the accountant, a second
//! holder of the reconciliation permission, the cashier who enters late
//! orders, aggregator orders that are completed with the aggregator's own
//! number, and the statement files in the shape of the demo preset.

#![allow(dead_code)]

use std::sync::atomic::{AtomicI64, Ordering};

use fnb::permissions as perm;
use fnb::{FnbOrder, FnbOrderStatus, PayoutStatement};
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_sales::SalesOrder;
use wl_sales::permissions as sperm;

use super::{World, make_user};

/// The header of an `aggregator_a` file, as the preset reads it.
pub const HEADER: &str = "Order ID,Gross,Commission,Promo,Refund,Net,Refund reason";

/// The aggregator of the tests' sales module.
pub const AGGREGATOR: &str = "aggregator_a";

static CUSTOMERS: AtomicI64 = AtomicI64::new(1);

/// The people of the payout tests.
pub struct Staff {
    /// Imports, matches, disputes, resolves and voids.
    pub accountant: User,
    /// A second holder of `reconciliation.manage`: approves a void.
    pub approver: User,
    /// Enters a missing order as a late entry; may read the statements.
    pub late_cashier: User,
    /// Reads the statements and nothing more.
    pub reader: User,
    /// Reads the reports.
    pub reports: User,
    /// HR: reads the rider faults.
    pub hr: User,
}

/// The column presets and reason mappings installed, and the people made.
pub async fn staff(w: &World) -> Staff {
    fnb::seed::install_payouts(w.db()).await.unwrap();
    let manage = [perm::RECONCILIATION_VIEW, perm::RECONCILIATION_MANAGE];
    Staff {
        accountant: make_user(w.db(), "Nour", &manage).await,
        approver: make_user(w.db(), "Omar2", &manage).await,
        late_cashier: make_user(
            w.db(),
            "Lina",
            &[perm::RECONCILIATION_VIEW, perm::LATE_ORDERS_ENTER, perm::ORDERS_VIEW],
        )
        .await,
        reader: make_user(w.db(), "Rana", &[perm::RECONCILIATION_VIEW]).await,
        reports: make_user(w.db(), "Reem", &[perm::REPORTS_VIEW, perm::ORDERS_VIEW]).await,
        hr: make_user(w.db(), "Hana", &[perm::RIDER_FAULTS_VIEW]).await,
    }
}

/// A completed aggregator order with the aggregator's number `external`: one
/// shawarma plate at `price` (zero-rated), cooked and collected.
pub async fn completed(w: &World, external: &str, price: &str) -> FnbOrder {
    let n = CUSTOMERS.fetch_add(1, Ordering::SeqCst);
    let customer = w.customer(&format!("Aggregator customer {n}")).await;
    let placed = w
        .place_via_external(
            AGGREGATOR,
            Some(external),
            customer,
            "on_account",
            &[("shawarma-plate", "1", price)],
        )
        .await;
    let ready = w.cook(&placed).await;
    w.post(
        &w.manager,
        &format!("/orders/{}/pickup/late", ready.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    let done = w.reload(&ready).await;
    assert_eq!(done.status, FnbOrderStatus::Completed);
    done
}

/// A statement file: the header and `rows`.
pub fn file(rows: &[&str]) -> String {
    let mut text = String::from(HEADER);
    for row in rows {
        text.push('\n');
        text.push_str(row);
    }
    text.push('\n');
    text
}

/// `by` imports `content` as statement `reference` of the aggregator.
pub async fn import(w: &World, by: &User, reference: &str, content: &str) -> TestResponse {
    w.post(
        by,
        "/payouts",
        &[
            ("aggregator", AGGREGATOR),
            ("reference", reference),
            ("file_name", "statement.csv"),
            ("period_from", "2026-10-05"),
            ("period_to", "2026-10-11"),
            ("content", content),
        ],
    )
    .await
}

/// The statement that holds `reference`, not voided.
pub async fn statement(w: &World, reference: &str) -> PayoutStatement {
    PayoutStatement::using(w.db(), AGGREGATOR, reference)
        .await
        .unwrap()
        .expect("the statement is imported")
}

/// `statement`, as stored now.
pub async fn reload(w: &World, statement: &PayoutStatement) -> PayoutStatement {
    PayoutStatement::find_or_404(w.db(), statement.id)
        .await
        .unwrap()
}

/// The sales order of `order`, as stored now.
pub async fn sales_of(w: &World, order: &FnbOrder) -> SalesOrder {
    SalesOrder::find_or_404(w.db(), order.sales_order_id)
        .await
        .unwrap()
}

/// `by` voids the statement `id` for `reason` (the kernel's route).
pub async fn void(w: &World, by: &User, id: i64, reason: &str) -> TestResponse {
    w.post(
        by,
        &format!("/voids/{}/{}", fnb::payout::SUBJECT_TYPE, id),
        &[("reason", reason)],
    )
    .await
}

/// A user who files and handles the returns of the sales module.
pub async fn complaints_clerk(w: &World) -> User {
    make_user(
        w.db(),
        "Cleo",
        &[
            sperm::SALES_ORDERS_VIEW,
            sperm::RETURNS_REQUEST,
            sperm::RETURNS_HANDLE,
        ],
    )
    .await
}
