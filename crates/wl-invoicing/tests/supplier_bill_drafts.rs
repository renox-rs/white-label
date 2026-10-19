//! Entering, changing and deleting a draft supplier bill (wl-invoicing §3,
//! #74 part 1): per-line UAE VAT, totals, the AED gross and no effect on
//! other bills.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::supplier_bill::{self, BillDraft};
use wl_invoicing::{BillLine, BillLineDraft, SupplierBill, SupplierBillStatus};
use wl_kernel::invoicing::VatCategory;
use wl_kernel::{LegalEntity, PartyId, Subject};

use support::boot;

const SUPPLIER: PartyId = PartyId(1);

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

fn is_refused<T>(result: Result<T>) -> bool {
    matches!(
        result,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    )
}

fn draft(number: &str) -> BillDraft {
    BillDraft::new(SUPPLIER, number, day(5))
        .line(BillLineDraft::new(
            "Pump repair",
            100_000,
            VatCategory::Standard,
        ))
        .line(BillLineDraft::new(
            "Callout",
            50_000,
            VatCategory::ZeroRated,
        ))
}

#[renox::test]
async fn a_draft_bill_has_per_line_vat_and_totals() {
    let t = boot().await;
    let bill = supplier_bill::create(t.db(), &draft("  INV-77 "), Some(7))
        .await
        .unwrap();
    assert_eq!(bill.status, SupplierBillStatus::Draft);
    assert_eq!(bill.supplier_invoice_number, "INV-77");
    assert_eq!(bill.number_key, "inv-77");
    assert_eq!((bill.party_id, bill.created_by), (1, Some(7)));
    assert_eq!(bill.currency, "AED");
    assert_eq!(
        bill.legal_entity_id,
        Some(LegalEntity::default_entity(t.db()).await.unwrap().id)
    );
    assert_eq!(
        (bill.net_total, bill.vat_total, bill.gross_total),
        (150_000, 5_000, 155_000)
    );
    assert_eq!(bill.gross_total_aed, Some(155_000));
    assert_eq!((bill.approved_by, bill.duplicate_of), (None, None));

    let lines = supplier_bill::lines_of(t.db(), bill.id).await.unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(
        (
            lines[0].position,
            lines[0].vat_amount,
            lines[0].gross_amount
        ),
        (1, 5_000, 105_000)
    );
    assert_eq!(lines[1].vat_amount, 0);
    assert_eq!(lines[1].net_amount_aed, Some(50_000));
    let totals = supplier_bill::totals_of(t.db(), bill.id).await.unwrap();
    assert_eq!(totals.gross, 155_000);
}

#[renox::test]
async fn a_foreign_bill_has_its_aed_gross_and_a_contract_with_a_period() {
    let t = boot().await;
    let contract = Subject::new("ticketing.contract", 3);
    let d = BillDraft::new(SUPPLIER, "A-1", day(5))
        .currency(" usd ", Some(3_670_000))
        .contract(contract.clone(), Some((day(1), day(31))))
        .line(
            BillLineDraft::new("Cleaning", 10_000, VatCategory::Standard)
                .service_date(day(2))
                .cost_object(Subject::new("property.unit", 9)),
        );
    let bill = supplier_bill::create(t.db(), &d, None).await.unwrap();
    assert_eq!(bill.currency, "USD");
    assert_eq!(bill.exchange_rate_micro, Some(3_670_000));
    assert_eq!(bill.gross_total, 10_500);
    assert_eq!(bill.gross_total_aed, Some(38_535));
    assert_eq!(bill.contract(), Some(contract));
    assert_eq!(bill.period(), Some((day(1), day(31))));
    let lines = supplier_bill::lines_of(t.db(), bill.id).await.unwrap();
    assert_eq!(lines[0].service_date, Some(day(2)));
    assert_eq!(
        lines[0].cost_object(),
        Some(Subject::new("property.unit", 9))
    );

    // Without a rate the AED gross is unknown.
    let d = BillDraft::new(SUPPLIER, "A-2", day(5))
        .currency("USD", None)
        .line(BillLineDraft::new(
            "Cleaning",
            10_000,
            VatCategory::Standard,
        ));
    let bill = supplier_bill::create(t.db(), &d, None).await.unwrap();
    assert_eq!(bill.gross_total_aed, None);
}

#[renox::test]
async fn a_bad_draft_is_refused() {
    let t = boot().await;
    let mut blank = draft("x");
    blank.supplier_invoice_number = "  ".into();
    assert!(is_refused(
        supplier_bill::create(t.db(), &blank, None).await
    ));
    assert!(is_refused(
        supplier_bill::create(t.db(), &draft("x").currency("DIRHAM", None), None).await
    ));
    assert!(is_refused(
        supplier_bill::create(t.db(), &draft("x").due_date(day(1)), None).await
    ));
    let bad_line =
        draft("x").line(BillLineDraft::new("Bad", 1, VatCategory::Standard).quantity_milli(0));
    assert!(is_refused(
        supplier_bill::create(t.db(), &bad_line, None).await
    ));
    assert_eq!(SupplierBill::query().count(t.db()).await.unwrap(), 0);
}

#[renox::test]
async fn a_draft_is_changed_and_deleted_but_a_submitted_bill_is_not() {
    let t = boot().await;
    let bill = supplier_bill::create(t.db(), &draft("INV-1"), Some(7))
        .await
        .unwrap();

    let changed = BillDraft::new(SUPPLIER, "INV-2", day(6)).line(BillLineDraft::new(
        "Pump",
        20_000,
        VatCategory::Standard,
    ));
    let after = supplier_bill::change(t.db(), bill.id, &changed)
        .await
        .unwrap();
    assert_eq!(after.number_key, "inv-2");
    assert_eq!(after.created_by, Some(7));
    assert_eq!(after.gross_total, 21_000);
    assert_eq!(after.lock_version, bill.lock_version + 1);
    let lines = supplier_bill::lines_of(t.db(), bill.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].description, "Pump");

    // A change that is refused leaves the bill as it was.
    let mut blank = changed.clone();
    blank.supplier_invoice_number = String::new();
    assert!(is_refused(
        supplier_bill::change(t.db(), bill.id, &blank).await
    ));
    assert_eq!(
        SupplierBill::find_or_404(t.db(), bill.id)
            .await
            .unwrap()
            .number_key,
        "inv-2"
    );

    // Once submitted neither is allowed.
    let submitted = wl_invoicing::bill_submit::submit(
        t.state(),
        wl_invoicing::credit_notes::By::System(None),
        bill.id,
    )
    .await
    .unwrap();
    assert_eq!(submitted.status, SupplierBillStatus::Matched);
    assert!(is_refused(
        supplier_bill::change(t.db(), bill.id, &changed).await
    ));
    assert!(is_refused(supplier_bill::discard(t.db(), bill.id).await));
    assert_eq!(BillLine::query().count(t.db()).await.unwrap(), 1);

    let other = supplier_bill::create(t.db(), &draft("INV-9"), None)
        .await
        .unwrap();
    supplier_bill::discard(t.db(), other.id).await.unwrap();
    assert!(
        SupplierBill::find(t.db(), other.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(BillLine::query().count(t.db()).await.unwrap(), 1);
}
