//! The Supplier debit note (wl-invoicing §3): a note against a
//! [`SupplierBill`](crate::supplier_bill::SupplierBill), drafted by hand or
//! from a return to the supplier, with its lines
//! ([`crate::debit_note_line::DebitNoteLine`]) and per-line UAE VAT. Its
//! status moves only through the machine in [`crate::debit_note_machine`];
//! the drafting services are in [`crate::debit_notes`]. The bill's own
//! lines, totals and VAT are never written by it.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::invoicing::{
    SUPPLIER_DEBIT_NOTE_SUBJECT_TYPE, SupplierBillId, SupplierDebitNoteId, SupplierDebitNoteStatus,
    SupplierDebitNoteSummary,
};
use wl_kernel::{PartyId, Subject};

use crate::vat::{self, VatTotals};

/// Where a debit note came from.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DebitNoteOrigin {
    /// Drafted by a user against a bill.
    #[default]
    Manual,
    /// Drafted by the system from a dispatched return to the supplier.
    Return,
}

/// Why a manual debit note is raised.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DebitNoteReason {
    /// The supplier billed a higher price than agreed.
    #[default]
    PriceDifference,
    /// Fewer goods arrived than were billed.
    ShortDelivery,
    /// The goods were not of the quality ordered.
    Quality,
    /// Anything else; the reason text is then required.
    Other,
}

impl DebitNoteReason {
    /// The reason with stored name `code`, if it is one.
    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|r| r.as_str() == code)
    }
}

/// A supplier debit note. Money is `i64` fils of
/// [`SupplierDebitNote::currency`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_supplier_debit_notes",
    state_machine,
    unique(number),
    unique(source_type, source_id),
    index(status),
    index(bill_id),
    index(party_id)
)]
pub struct SupplierDebitNote {
    /// The primary key.
    pub id: i64,
    /// The number from the `supplier_debit_note` series, set at `issue`;
    /// `None` while a draft.
    pub number: Option<String>,
    /// The receiving legal entity: the bill's, or the return's while the
    /// note waits for its bill.
    pub legal_entity_id: i64,
    /// The supplier (a party).
    pub party_id: i64,
    /// The bill it reduces; `None` while it waits for the bill.
    pub bill_id: Option<i64>,
    /// Manual, or from a return.
    pub origin: DebitNoteOrigin,
    /// The type of the return it was drafted from, if any.
    pub source_type: Option<String>,
    /// The id of the return it was drafted from, if any.
    pub source_id: Option<i64>,
    /// The code of a [`DebitNoteReason`]; `None` for a return.
    pub reason_code: Option<String>,
    /// The reason text; may be empty.
    pub reason: String,
    /// The ISO currency code: the bill's.
    pub currency: String,
    /// The rate to AED × 1,000,000; `None` for AED or until set.
    pub exchange_rate_micro: Option<i64>,
    /// The sum of the line nets.
    pub net_total: i64,
    /// The sum of the line VAT.
    pub vat_total: i64,
    /// Net plus VAT.
    pub gross_total: i64,
    /// The VAT in AED fils; `None` while the rate is unknown.
    pub vat_total_aed: Option<i64>,
    /// What reduced the bill: 0 until `issue`.
    pub applied_to_bill: i64,
    /// What is open as the supplier's debit balance: 0 until `issue`.
    pub debit_balance: i64,
    /// The issue date (Asia/Dubai), set at `issue`.
    pub issue_date: Option<NaiveDate>,
    /// The user who drafted it; `None` for the system.
    pub created_by: Option<i64>,
    /// The user who issued it.
    pub issued_by: Option<i64>,
    /// The number on the supplier's tax credit note, once recorded.
    pub tax_credit_note_number: Option<String>,
    /// The date on the supplier's tax credit note.
    pub tax_credit_note_date: Option<NaiveDate>,
    /// The file of the supplier's tax credit note (a document id).
    pub tax_credit_note_document: Option<i64>,
    /// The user who recorded the tax credit note.
    pub tax_credit_note_by: Option<i64>,
    /// When it was settled.
    pub settled_at: Option<DateTime>,
    /// The user who settled it; `None` for the system.
    pub settled_by: Option<i64>,
    /// What the supplier paid short, accepted with a reason, in fils.
    pub settle_difference: i64,
    /// Why the difference was accepted.
    pub settle_difference_reason: Option<String>,
    /// When it was cancelled.
    pub cancelled_at: Option<DateTime>,
    /// The user who cancelled it; `None` for the system.
    pub cancelled_by: Option<i64>,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// The lifecycle state; changed only by a transition.
    pub status: SupplierDebitNoteStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SupplierDebitNote {
    /// The debit note's id in the kernel contract.
    pub fn debit_note_id(&self) -> SupplierDebitNoteId {
        SupplierDebitNoteId(self.id)
    }

    /// The debit note as a [`Subject`]: `invoicing.supplier_debit_note:7`.
    pub fn subject(&self) -> Subject {
        self.debit_note_id().subject()
    }

    /// The supplier.
    pub fn supplier(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// The bill it reduces, once linked.
    pub fn bill(&self) -> Option<SupplierBillId> {
        self.bill_id.map(SupplierBillId)
    }

    /// The return it was drafted from, if any.
    pub fn source(&self) -> Option<Subject> {
        match (&self.source_type, self.source_id) {
            (Some(subject_type), Some(subject_id)) => Some(Subject {
                subject_type: subject_type.clone(),
                subject_id,
            }),
            _ => None,
        }
    }

    /// The rate to AED: 1 for AED, else the stored rate.
    pub fn aed_rate(&self) -> Option<i64> {
        vat::aed_rate(&self.currency, self.exchange_rate_micro)
    }

    /// Stores `totals` as the debit note's totals.
    pub fn set_totals(&mut self, totals: &VatTotals) {
        self.net_total = totals.net;
        self.vat_total = totals.vat;
        self.gross_total = totals.gross;
        self.vat_total_aed = totals.vat_aed;
    }

    /// What other modules see of it.
    pub fn summary(&self) -> SupplierDebitNoteSummary {
        SupplierDebitNoteSummary {
            id: self.debit_note_id(),
            number: self.number.clone(),
            status: self.status,
            supplier: self.supplier(),
            bill: self.bill(),
            legal_entity_id: self.legal_entity_id,
            currency: self.currency.clone(),
            gross: self.gross_total,
            applied_to_bill: self.applied_to_bill,
            debit_balance: self.debit_balance,
        }
    }
}

impl SupplierDebitNote {
    /// Whether the supplier's tax credit note is recorded.
    pub fn has_tax_credit_note(&self) -> bool {
        self.tax_credit_note_number.is_some()
    }
}

/// The subject type of a debit note: `invoicing.supplier_debit_note`.
pub const SUBJECT_TYPE: &str = SUPPLIER_DEBIT_NOTE_SUBJECT_TYPE;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vat::LineAmounts;
    use wl_kernel::invoicing::VatCategory;

    #[test]
    fn origins_and_reasons_are_stored_in_snake_case() {
        let origins: Vec<_> = DebitNoteOrigin::ALL.iter().map(|o| o.as_str()).collect();
        assert_eq!(origins, ["manual", "return"]);
        let reasons: Vec<_> = DebitNoteReason::ALL.iter().map(|r| r.as_str()).collect();
        assert_eq!(
            reasons,
            ["price_difference", "short_delivery", "quality", "other"]
        );
        assert_eq!(
            DebitNoteReason::parse("short_delivery"),
            Some(DebitNoteReason::ShortDelivery)
        );
        assert_eq!(DebitNoteReason::parse("bogus"), None);
    }

    #[test]
    fn totals_the_subject_and_the_summary() {
        let mut note = SupplierDebitNote {
            id: 7,
            legal_entity_id: 2,
            party_id: 9,
            bill_id: Some(4),
            currency: "AED".into(),
            ..Default::default()
        };
        let line = LineAmounts::compute(3_000, 10_000, 0, VatCategory::Standard, note.aed_rate());
        note.set_totals(&VatTotals::of(&[line]));
        assert_eq!(
            (note.net_total, note.vat_total, note.gross_total),
            (30_000, 1_500, 31_500)
        );
        assert_eq!(note.vat_total_aed, Some(1_500));
        assert_eq!(note.status, SupplierDebitNoteStatus::Draft);
        let summary = note.summary();
        assert_eq!(summary.id, SupplierDebitNoteId(7));
        assert_eq!(summary.bill, Some(SupplierBillId(4)));
        assert_eq!(summary.supplier, PartyId(9));
        assert_eq!(summary.gross, 31_500);
        assert_eq!((summary.applied_to_bill, summary.debit_balance), (0, 0));
        assert_eq!(summary.number, None);
        assert_eq!(
            note.subject().to_string(),
            "invoicing.supplier_debit_note:7"
        );
        assert_eq!(SUBJECT_TYPE, "invoicing.supplier_debit_note");
        assert_eq!(note.source(), None);
    }
}
