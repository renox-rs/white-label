//! The VAT adjustment (wl-invoicing §3): a correction carried into a VAT
//! period. A bad-debt write-off creates one of kind
//! [`AdjustmentKind::BadDebtRelief`] as [`AdjustmentStatus::Pending`]
//! (§5.9 step 3); `FIN.RUL.14` proposes it later. Amounts are AED fils.
//!
//! An invoice has at most one bad-debt relief: the table has a unique index
//! on (kind, source) limited to `bad_debt_relief`, so the other kinds (a
//! recovery adjustment per payment, for one) may repeat per source.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::invoice::Invoice;

/// What an adjustment corrects (§3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AdjustmentKind {
    /// A document that arrived after its period was locked.
    LateDocument,
    /// A credit note for a supply of an earlier period.
    EarlierPeriodCreditNote,
    /// Output VAT given back on a written-off invoice.
    #[default]
    BadDebtRelief,
    /// Relief paid back when a written-off invoice is paid after all.
    BadDebtRecovery,
    /// A debit note to a supplier.
    SupplierDebitNote,
    /// Entered by hand.
    Manual,
}

/// Where an adjustment stands (§3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AdjustmentStatus {
    /// Waiting until its conditions are met (bad-debt relief).
    #[default]
    Pending,
    /// On a VAT summary, for the preparer to include or exclude.
    Proposed,
    /// Included in the summary.
    Included,
    /// Excluded, with a reason.
    Excluded,
}

/// A VAT adjustment. The amount is in AED fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_vat_adjustments",
    index(source_type, source_id),
    index(status),
    index(summary_id)
)]
pub struct VatAdjustment {
    /// The primary key.
    pub id: i64,
    /// The VAT summary it is carried into; `None` while pending.
    pub summary_id: Option<i64>,
    /// The issuing legal entity of the source document.
    pub legal_entity_id: Option<i64>,
    /// What it corrects.
    pub kind: AdjustmentKind,
    /// The type of the source document, e.g. `invoicing.invoice`.
    pub source_type: String,
    /// The id of the source document.
    pub source_id: i64,
    /// The VAT amount, in AED fils.
    pub amount: i64,
    /// Why, when given.
    pub reason: Option<String>,
    /// Where it stands.
    pub status: AdjustmentStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl VatAdjustment {
    /// The pending bad-debt relief of `amount` AED fils (the VAT share of
    /// the written-off amount) for `invoice`, not yet stored.
    pub fn bad_debt_relief(invoice: &Invoice, amount: i64) -> Self {
        let source = invoice.subject();
        VatAdjustment {
            legal_entity_id: invoice.legal_entity_id,
            kind: AdjustmentKind::BadDebtRelief,
            source_type: source.subject_type,
            source_id: source.subject_id,
            amount,
            status: AdjustmentStatus::Pending,
            ..Default::default()
        }
    }

    /// The source document.
    pub fn source(&self) -> Subject {
        Subject {
            subject_type: self.source_type.clone(),
            subject_id: self.source_id,
        }
    }

    /// The bad-debt relief of `invoice`, if one was created.
    pub async fn relief_of(db: &Db, invoice: &Invoice) -> Result<Option<VatAdjustment>> {
        let source = invoice.subject();
        VatAdjustment::query()
            .where_(VatAdjustment::KIND.eq(AdjustmentKind::BadDebtRelief))
            .where_(VatAdjustment::SOURCE_TYPE.eq(source.subject_type))
            .where_(VatAdjustment::SOURCE_ID.eq(source.subject_id))
            .first(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_statuses_are_stored_in_snake_case() {
        let kinds: Vec<_> = AdjustmentKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "late_document",
                "earlier_period_credit_note",
                "bad_debt_relief",
                "bad_debt_recovery",
                "supplier_debit_note",
                "manual"
            ]
        );
        let statuses: Vec<_> = AdjustmentStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(statuses, ["pending", "proposed", "included", "excluded"]);
    }

    #[test]
    fn a_relief_is_pending_for_the_invoice_and_its_entity() {
        let invoice = Invoice {
            id: 42,
            legal_entity_id: Some(3),
            ..Default::default()
        };
        let relief = VatAdjustment::bad_debt_relief(&invoice, 5_000);
        assert_eq!(relief.kind, AdjustmentKind::BadDebtRelief);
        assert_eq!(relief.status, AdjustmentStatus::Pending);
        assert_eq!(relief.amount, 5_000);
        assert_eq!(relief.legal_entity_id, Some(3));
        assert_eq!(relief.summary_id, None);
        assert_eq!(relief.source().to_string(), "invoicing.invoice:42");
    }
}
