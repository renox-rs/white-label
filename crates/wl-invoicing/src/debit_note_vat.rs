//! The input VAT a supplier debit note takes back (wl-invoicing §5.13 step 3,
//! §7 *input VAT timing*, `⚠ open: accountant (A-11)`).
//!
//! An `issued` or `settled` debit note reduces the input VAT of the VAT
//! period that holds its counting date, in AED fils: its issue date by
//! default ([`InputVatTiming::OnIssue`]), or the date of the supplier's tax
//! credit note ([`InputVatTiming::OnTaxCreditNote`]; a note without one
//! counts nowhere yet). A `draft` and a `cancelled` note count nowhere, so a
//! cancel reverses the reduction by itself; a period that was locked when
//! the note was cancelled is not rewritten, the VAT summary of the next open
//! period simply no longer carries the reduction (`FIN.ACL.09`).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::invoicing::SupplierDebitNoteStatus;

use crate::config::{InputVatTiming, InvoicingConfig};
use crate::supplier_debit_note::SupplierDebitNote;

/// The date `note` counts on under `timing`, if it counts yet.
pub fn counted_on(note: &SupplierDebitNote, timing: InputVatTiming) -> Option<NaiveDate> {
    if !matches!(
        note.status,
        SupplierDebitNoteStatus::Issued | SupplierDebitNoteStatus::Settled
    ) {
        return None;
    }
    match timing {
        InputVatTiming::OnIssue => note.issue_date,
        InputVatTiming::OnTaxCreditNote => note.tax_credit_note_date,
    }
}

/// The input VAT that the debit notes of `entity` take back in the period
/// `from` to `to`, in AED fils.
///
/// # Errors
///
/// The database's errors.
pub async fn reduction(
    db: &Db,
    config: &InvoicingConfig,
    entity: i64,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<i64> {
    let notes = SupplierDebitNote::query()
        .where_(SupplierDebitNote::LEGAL_ENTITY_ID.eq(entity))
        .where_(SupplierDebitNote::STATUS.is_in([
            SupplierDebitNoteStatus::Issued,
            SupplierDebitNoteStatus::Settled,
        ]))
        .get(db)
        .await?;
    Ok(notes
        .iter()
        .filter(|n| counted_on(n, config.debit_note_vat_timing).is_some_and(|d| d >= from && d <= to))
        .map(|n| n.vat_total_aed.unwrap_or(0))
        .sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    fn note(status: SupplierDebitNoteStatus) -> SupplierDebitNote {
        SupplierDebitNote {
            status,
            issue_date: Some(day(5)),
            tax_credit_note_date: Some(day(20)),
            ..Default::default()
        }
    }

    #[test]
    fn a_note_counts_on_its_issue_date_or_its_tax_credit_note() {
        let issued = note(SupplierDebitNoteStatus::Issued);
        assert_eq!(counted_on(&issued, InputVatTiming::OnIssue), Some(day(5)));
        assert_eq!(
            counted_on(&issued, InputVatTiming::OnTaxCreditNote),
            Some(day(20))
        );
        let waiting = SupplierDebitNote {
            tax_credit_note_date: None,
            ..issued
        };
        assert_eq!(counted_on(&waiting, InputVatTiming::OnTaxCreditNote), None);
    }

    #[test]
    fn a_draft_and_a_cancelled_note_count_nowhere() {
        for status in [SupplierDebitNoteStatus::Draft, SupplierDebitNoteStatus::Cancelled] {
            assert_eq!(counted_on(&note(status), InputVatTiming::OnIssue), None);
        }
        let settled = note(SupplierDebitNoteStatus::Settled);
        assert_eq!(counted_on(&settled, InputVatTiming::OnIssue), Some(day(5)));
    }
}
