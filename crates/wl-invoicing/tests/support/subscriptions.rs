//! What the subscription tests (#82, #83, #84) share: a module that issues
//! cycle invoices by itself, a monthly retainer of Blue Dunes for AED
//! 3,100.00 a cycle (AED 2,952.38 plus 5% VAT), started through the kernel's
//! `Subscriptions` as a vertical does, and readers of what it became.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::subscriptions;
use wl_invoicing::{CreditNote, Invoice, Subscription, SubscriptionCycle, SubscriptionPause};
use wl_kernel::Subject;
use wl_kernel::invoicing::VatCategory;
use wl_kernel::subscriptions::{
    SUBSCRIPTION_SUBJECT_TYPE, StartSubscription, SubscriptionId, subscriptions as contract,
};

use super::{BLUE_DUNES, Booted, boot_with};

/// The net price of a cycle: AED 2,952.38, AED 3,100.00 with 5% VAT.
pub const NET: i64 = 295_238;
/// The cycle with VAT.
pub const GROSS: i64 = 310_000;

/// A day, from `YYYY-MM-DD`.
pub fn d(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

/// `module`, issuing the cycle invoices itself, booted.
pub async fn boot_subscriptions(module: wl_invoicing::WlInvoicing) -> Booted {
    boot_with(module.auto_issue_source(SUBSCRIPTION_SUBJECT_TYPE)).await
}

/// The vertical's agreement `n`: an engagement of 04.
pub fn engagement(n: i64) -> Subject {
    Subject::new("accounting.engagement", n)
}

/// The vertical's agreement `n`: an enrolment of 10.
pub fn enrolment(n: i64) -> Subject {
    Subject::new("edtech.enrolment", n)
}

/// A monthly subscription of Blue Dunes for `agreement` from `start`,
/// started through the kernel contract.
pub async fn start(t: &Booted, agreement: &Subject, start: NaiveDate) -> Subscription {
    let id = contract(t.state())
        .start(
            t.state(),
            StartSubscription::new(BLUE_DUNES, agreement.clone(), "Retainer", NET, start)
                .vat_category(VatCategory::Standard),
        )
        .await
        .expect("the subscription starts")
        .expect("invoicing is on");
    subscription(t, id).await
}

/// Subscription `id`, as stored now.
pub async fn subscription(t: &Booted, id: SubscriptionId) -> Subscription {
    Subscription::find(t.db(), id.0).await.unwrap().unwrap()
}

/// `subscription`, as stored now.
pub async fn now(t: &Booted, subscription: &Subscription) -> Subscription {
    Subscription::find(t.db(), subscription.id)
        .await
        .unwrap()
        .unwrap()
}

/// Bills the cycles due on `on`.
pub async fn bill(t: &Booted, on: NaiveDate) -> usize {
    subscriptions::bill_due(t.state(), on)
        .await
        .expect("the billing run")
}

/// Runs the daily check (`FIN.RUL.06`) on `on`.
pub async fn evaluate(t: &Booted, on: NaiveDate) {
    subscriptions::evaluate(t.state(), on)
        .await
        .expect("the daily check");
}

/// The cycle invoices of `subscription`, oldest cycle first.
pub async fn invoices(t: &Booted, subscription: &Subscription) -> Vec<Invoice> {
    subscriptions::cycle_invoices(t.db(), subscription.id)
        .await
        .unwrap()
        .into_iter()
        .map(|(_, invoice)| invoice)
        .collect()
}

/// The cycles of `subscription`, oldest first.
pub async fn cycles(t: &Booted, subscription: &Subscription) -> Vec<SubscriptionCycle> {
    SubscriptionCycle::of(t.db(), subscription.id)
        .await
        .unwrap()
}

/// The open pause causes of `subscription`.
pub async fn causes(t: &Booted, subscription: &Subscription) -> Vec<SubscriptionPause> {
    SubscriptionPause::open_of(t.db(), subscription.id)
        .await
        .unwrap()
}

/// The credit notes against `invoice`, oldest first.
pub async fn credit_notes(t: &Booted, invoice: &Invoice) -> Vec<CreditNote> {
    CreditNote::query()
        .where_(CreditNote::INVOICE_ID.eq(invoice.id))
        .order_by(CreditNote::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// A subscription started on `start` whose first cycle invoice is issued and
/// was marked `overdue` (its due date, the cycle start, has passed).
pub async fn with_overdue_cycle(
    t: &Booted,
    agreement: &Subject,
    start: NaiveDate,
) -> (Subscription, Invoice) {
    let subscription = self::start(t, agreement, start).await;
    assert_eq!(bill(t, start).await, 1);
    wl_invoicing::overdue::run(t.state())
        .await
        .expect("the overdue run");
    let invoice = invoices(t, &subscription).await.remove(0);
    assert_eq!(
        invoice.status,
        wl_kernel::invoicing::InvoiceStatus::Overdue,
        "the cycle invoice is overdue"
    );
    (subscription, invoice)
}
