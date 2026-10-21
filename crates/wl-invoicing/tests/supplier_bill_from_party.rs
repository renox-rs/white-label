//! A vertical records a bill from a party through the kernel's
//! `SupplierBills::create_submitted` (02 Construction #285): what a client
//! deducts from a certificate becomes a supplier bill from the client, with no
//! purchase order to match, which waits for the accountant's approval and is
//! then set off against the client's invoice.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::bill_machine::approve;
use wl_invoicing::credit_notes::By;
use wl_invoicing::{SupplierBill, SupplierBillStatus};
use wl_kernel::invoicing::{BillRequest, DraftLine, SupplierBillId, VatCategory, supplier_bills};
use wl_kernel::{PartyId, Subject};

use support::{BLUE_DUNES, boot};

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()
}

fn request(supplier: PartyId, number: &str) -> BillRequest {
    BillRequest::new(supplier, number, day(), Subject::new("construction.ipa", 6)).line(
        DraftLine::new("BC-12 back-charge", 500_000, VatCategory::Standard),
    )
}

/// A bill of AED 5,000 + 250 VAT is recorded and submitted as the system:
/// `matched`, no purchase order needed, and after the accountant approves it
/// its open balance is AED 5,250.
#[renox::test]
async fn a_requested_bill_is_submitted_and_waits_for_approval() {
    let t = boot().await;
    let made = supplier_bills(t.state())
        .create_submitted(t.state(), request(BLUE_DUNES, "TWR-IPA6-DEDUCTIONS"))
        .await
        .unwrap()
        .expect("invoicing is on");

    let bill = SupplierBill::find_or_404(t.db(), made.0).await.unwrap();
    assert_eq!(bill.status, SupplierBillStatus::Matched);
    assert_eq!(bill.party_id, BLUE_DUNES.0);
    assert_eq!(bill.supplier_invoice_number, "TWR-IPA6-DEDUCTIONS");
    assert_eq!(bill.currency, "AED");

    approve(t.state(), By::System(Some(8)), made.0, "")
        .await
        .unwrap();
    let summary = supplier_bills(t.state())
        .find(t.db(), SupplierBillId(made.0))
        .await
        .unwrap()
        .expect("the bill");
    assert_eq!(summary.supplier, BLUE_DUNES);
    assert_eq!(summary.balance, 525_000);
}

/// A bill with no line, or a line with a discount, is refused (422).
#[renox::test]
async fn a_request_without_lines_or_with_a_discount_is_refused() {
    let t = boot().await;
    let empty = BillRequest::new(
        BLUE_DUNES,
        "E-1",
        day(),
        Subject::new("construction.ipa", 6),
    );
    let refused = supplier_bills(t.state())
        .create_submitted(t.state(), empty)
        .await;
    assert!(matches!(
        refused,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    let discounted = BillRequest::new(
        BLUE_DUNES,
        "D-1",
        day(),
        Subject::new("construction.ipa", 6),
    )
    .line(DraftLine::new("BC-1", 100_000, VatCategory::Standard).discount(1));
    let refused = supplier_bills(t.state())
        .create_submitted(t.state(), discounted)
        .await;
    assert!(matches!(
        refused,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
}
