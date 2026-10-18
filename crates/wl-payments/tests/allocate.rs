//! Allocating a payment to invoices: the acceptance check AC4 of #94, with
//! a recording `Invoicer`.

mod support;

use renox::prelude::*;
use wl_kernel::payments::PaymentStatus;

use support::{BLUE_DUNES, QUIET_SANDS, boot};

fn refused<T: std::fmt::Debug>(result: Result<T>) {
    assert!(
        matches!(result, Err(Error::Status(code, _)) if code == StatusCode::UNPROCESSABLE_ENTITY),
        "{result:?}"
    );
}

/// AC4: AED 3,000.00 is allocated 2,000.00 to INV-1 and 1,000.00 to INV-2:
/// two rows, two `Invoicer::allocate` calls with the payment as source, and
/// nothing left unallocated.
#[renox::test]
async fn a_payment_is_allocated_to_two_invoices() {
    let t = boot().await;
    let payment = t.receive(BLUE_DUNES, 300_000).await;
    let inv1 = t.invoices.add(BLUE_DUNES, "INV-1", 250_000);
    let inv2 = t.invoices.add(BLUE_DUNES, "INV-2", 100_000);
    assert_eq!(t.summary(&payment).await.unallocated, 300_000);

    t.allocate(&payment, inv1, 200_000).await.unwrap();
    t.allocate(&payment, inv2, 100_000).await.unwrap();

    let rows = t.allocations(&payment).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].target(), inv1.subject());
    assert_eq!(rows[0].amount, 200_000);
    assert_eq!(rows[1].target(), inv2.subject());
    assert_eq!(rows[1].amount, 100_000);
    assert!(rows.iter().all(|r| r.is_active()));

    let calls = t.invoices.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!((calls[0].invoice, calls[0].amount), (inv1, 200_000),);
    assert_eq!((calls[1].invoice, calls[1].amount), (inv2, 100_000));
    assert!(calls.iter().all(|c| c.source == payment.subject()));

    let summary = t.summary(&payment).await;
    assert_eq!(summary.allocated, 300_000);
    assert_eq!(summary.unallocated, 0);
    // The party has no credit left.
    let credit = wl_kernel::payments::payment_recorder(t.state())
        .credit_balance(t.db(), BLUE_DUNES, "AED")
        .await
        .unwrap();
    assert_eq!(credit, 0);
}

/// AC4: more than the unallocated amount, another party's invoice, an
/// invoice that refuses, and a `reversed` payment are all refused and write
/// nothing.
#[renox::test]
async fn bad_allocations_are_refused_and_write_nothing() {
    let t = boot().await;
    let payment = t.receive(BLUE_DUNES, 300_000).await;
    let own = t.invoices.add(BLUE_DUNES, "INV-1", 500_000);
    let other = t.invoices.add(QUIET_SANDS, "INV-9", 500_000);
    let small = t.invoices.add(BLUE_DUNES, "INV-2", 10_000);

    // Above the unallocated amount.
    refused(t.allocate(&payment, own, 300_001).await);
    // Zero.
    refused(t.allocate(&payment, own, 0).await);
    // Another party's invoice.
    refused(t.allocate(&payment, other, 100).await);
    // The invoice refuses (above its balance): nothing is written.
    refused(t.allocate(&payment, small, 20_000).await);
    // An unknown invoice.
    assert!(matches!(
        t.allocate(&payment, wl_kernel::invoicing::InvoiceId(99), 100)
            .await,
        Err(Error::NotFound)
    ));
    assert!(t.allocations(&payment).await.is_empty());
    assert!(t.invoices.calls().is_empty());

    // After a partial allocation the rest is what can still be allocated.
    t.allocate(&payment, own, 250_000).await.unwrap();
    refused(t.allocate(&payment, own, 50_001).await);
    t.allocate(&payment, own, 50_000).await.unwrap();
    assert_eq!(t.allocations(&payment).await.len(), 2);

    // A reversed payment cannot be allocated from.
    let spare = t.receive(BLUE_DUNES, 100_000).await;
    let user = t.user_a().await;
    t.reverse(&user, &spare, "entered_in_error", "")
        .await
        .assert_ok();
    assert_eq!(t.reload(&spare).await.status, PaymentStatus::Reversed);
    refused(t.allocate(&spare, own, 100).await);
    assert!(t.allocations(&spare).await.is_empty());
    assert_eq!(t.invoices.calls().len(), 2);
}
