//! The Credit note (wl-invoicing §3): a tax credit note against an
//! [`Invoice`](crate::invoice::Invoice), with its lines
//! ([`crate::credit_note_line::CreditNoteLine`]) and per-line UAE VAT. Its
//! status moves only through the machine in [`crate::credit_note_machine`].
//! The invoice's own lines, totals and VAT are never written by it.

use renox::chrono::NaiveDate;
use renox::db::Json;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::invoicing::{CreditNoteId, CreditNoteStatus, CreditNoteSummary, InvoiceId};
use wl_kernel::{IssuerSnapshot, PartyId, Subject};

use crate::invoice::PartySnapshot;
use crate::vat::{self, VatTotals};

/// How much of the invoice a credit note credits.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CreditNoteKind {
    /// Some lines, or part of their net.
    #[default]
    Partial,
    /// Every line at its full net (the invoice's `credit`).
    Full,
}

/// A credit note. Money is `i64` fils of [`CreditNote::currency`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_credit_notes",
    state_machine,
    unique(number),
    index(status),
    index(invoice_id),
    index(party_id)
)]
pub struct CreditNote {
    /// The primary key.
    pub id: i64,
    /// The number from the `credit_note` series, set at `issue`; `None`
    /// while a draft.
    pub number: Option<String>,
    /// The invoice it credits.
    pub invoice_id: i64,
    /// The issuing legal entity: always the invoice's.
    pub legal_entity_id: Option<i64>,
    /// The issuer as it was at `issue`.
    pub issuer_snapshot: Option<Json<IssuerSnapshot>>,
    /// Who is credited.
    pub party_id: i64,
    /// The party's billing identity, copied from the invoice at `issue`.
    pub party_snapshot: Option<Json<PartySnapshot>>,
    /// The ISO currency code: the invoice's.
    pub currency: String,
    /// The invoice's rate to AED × 1,000,000; `None` for AED or until set.
    pub exchange_rate_micro: Option<i64>,
    /// Why the invoice is credited.
    pub reason: String,
    /// The issue date (Asia/Dubai), set at `issue`.
    pub issue_date: Option<NaiveDate>,
    /// The sum of the line nets.
    pub net_total: i64,
    /// The sum of the line VAT.
    pub vat_total: i64,
    /// Net plus VAT.
    pub gross_total: i64,
    /// The VAT in AED fils; `None` while the rate is unknown.
    pub vat_total_aed: Option<i64>,
    /// What is still unused: 0 for a draft, the gross total at `issue`.
    pub remaining: i64,
    /// Partial, or full (every line at its full net).
    pub kind: CreditNoteKind,
    /// Whether the lines' origins are released to be billed again.
    pub release_origins: bool,
    /// The user who drafted it; `None` for the system.
    pub created_by: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: CreditNoteStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CreditNote {
    /// The credit note's id in the kernel contract.
    pub fn credit_note_id(&self) -> CreditNoteId {
        CreditNoteId(self.id)
    }

    /// The credit note as a [`Subject`]: `invoicing.credit_note:7`.
    pub fn subject(&self) -> Subject {
        self.credit_note_id().subject()
    }

    /// The invoice it credits.
    pub fn invoice(&self) -> InvoiceId {
        InvoiceId(self.invoice_id)
    }

    /// Who is credited.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// The rate to AED: 1 for AED, else the stored rate.
    pub fn aed_rate(&self) -> Option<i64> {
        vat::aed_rate(&self.currency, self.exchange_rate_micro)
    }

    /// Stores `totals` as the credit note's totals.
    pub fn set_totals(&mut self, totals: &VatTotals) {
        self.net_total = totals.net;
        self.vat_total = totals.vat;
        self.gross_total = totals.gross;
        self.vat_total_aed = totals.vat_aed;
    }

    /// What other modules see of it.
    pub fn summary(&self) -> CreditNoteSummary {
        CreditNoteSummary {
            id: self.credit_note_id(),
            number: self.number.clone(),
            status: self.status,
            invoice: self.invoice(),
            party: self.party(),
            legal_entity_id: self.legal_entity_id,
            currency: self.currency.clone(),
            gross_total: self.gross_total,
            remaining: self.remaining,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vat::LineAmounts;
    use wl_kernel::invoicing::VatCategory;

    #[test]
    fn kinds_are_stored_in_snake_case() {
        let kinds: Vec<_> = CreditNoteKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(kinds, ["partial", "full"]);
    }

    #[test]
    fn totals_and_the_summary() {
        let mut note = CreditNote {
            id: 7,
            invoice_id: 4,
            party_id: 9,
            currency: "AED".into(),
            ..Default::default()
        };
        let line = LineAmounts::for_net(20_000, VatCategory::Standard, note.aed_rate());
        note.set_totals(&VatTotals::of(&[line]));
        assert_eq!(
            (note.net_total, note.vat_total, note.gross_total),
            (20_000, 1_000, 21_000)
        );
        assert_eq!(note.vat_total_aed, Some(1_000));
        assert_eq!(note.status, CreditNoteStatus::Draft);
        assert_eq!(note.remaining, 0);
        let summary = note.summary();
        assert_eq!(summary.id, CreditNoteId(7));
        assert_eq!(summary.invoice, InvoiceId(4));
        assert_eq!(summary.party, PartyId(9));
        assert_eq!((summary.gross_total, summary.remaining), (21_000, 0));
        assert_eq!(summary.number, None);
        assert_eq!(note.subject().to_string(), "invoicing.credit_note:7");
    }
}
