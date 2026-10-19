//! The Invoice (wl-invoicing §3): a tax invoice to a party, with its lines
//! ([`crate::line::InvoiceLine`]), allocations
//! ([`crate::allocation::Allocation`]), totals and write-off. Its status
//! moves only through the machine in [`crate::machine`].

use renox::chrono::NaiveDate;
use renox::db::Json;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, InvoiceSummary};
use wl_kernel::parties::BillingIdentity;
use wl_kernel::{IssuerSnapshot, PartyId, Subject};

use crate::vat::{self, VatTotals};

/// The kind of invoice document. Only `tax_invoice` is issued today; the
/// guards of the other two come with e-invoicing.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InvoiceKind {
    /// A full tax invoice.
    #[default]
    TaxInvoice,
    /// A simplified tax invoice (to a non-registered recipient).
    SimplifiedTaxInvoice,
    /// A commercial invoice, with no VAT.
    CommercialInvoice,
}

/// The party's billing identity as it was at `issue`, stored on the invoice
/// (as JSON) and never updated afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartySnapshot {
    /// The legal name in English.
    pub legal_name_en: String,
    /// The legal name in Arabic, if known.
    pub legal_name_ar: Option<String>,
    /// The TRN, when VAT registered.
    pub trn: Option<String>,
    /// The billing address, one text.
    pub billing_address: Option<String>,
    /// The e-mail address invoices and notices go to.
    pub billing_email: Option<String>,
    /// When it was taken.
    pub taken_at: DateTime,
}

impl PartySnapshot {
    /// The snapshot of `identity`, taken now.
    pub fn of(identity: &BillingIdentity) -> Self {
        PartySnapshot {
            legal_name_en: identity.legal_name_en.clone(),
            legal_name_ar: identity.legal_name_ar.clone(),
            trn: identity.trn.clone(),
            billing_address: identity.billing_address.clone(),
            billing_email: identity.billing_email.clone(),
            taken_at: renox::db::now(),
        }
    }
}

/// A tax invoice. Money is `i64` fils of [`Invoice::currency`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_invoices",
    state_machine,
    unique(number),
    index(status),
    index(party_id),
    index(source_type, source_id)
)]
pub struct Invoice {
    /// The primary key.
    pub id: i64,
    /// The number from the `invoice` series, set at `issue`; `None` while a
    /// draft.
    pub number: Option<String>,
    /// The kind of document.
    pub kind: InvoiceKind,
    /// The issuing legal entity.
    pub legal_entity_id: Option<i64>,
    /// The issuer as it was at `issue`.
    pub issuer_snapshot: Option<Json<IssuerSnapshot>>,
    /// Who is billed.
    pub party_id: i64,
    /// The party's billing identity as it was at `issue`.
    pub party_snapshot: Option<Json<PartySnapshot>>,
    /// The type of the record it bills for, e.g. `property.lease`.
    pub source_type: String,
    /// The id of the record it bills for.
    pub source_id: i64,
    /// The type of the scope unit, if any.
    pub scope_type: Option<String>,
    /// The id of the scope unit, if any.
    pub scope_id: Option<i64>,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// The rate to AED × 1,000,000 for a foreign currency; `None` for AED or
    /// until set.
    pub exchange_rate_micro: Option<i64>,
    /// The issue date (Asia/Dubai), set at `issue`.
    pub issue_date: Option<NaiveDate>,
    /// The date of supply, when not the issue date.
    pub date_of_supply: Option<NaiveDate>,
    /// The due date: given on the draft, else set at `issue`.
    pub due_date: Option<NaiveDate>,
    /// The sum of the line nets.
    pub net_total: i64,
    /// The sum of the line VAT.
    pub vat_total: i64,
    /// Net plus VAT.
    pub gross_total: i64,
    /// The VAT in AED fils; `None` while the rate is unknown.
    pub vat_total_aed: Option<i64>,
    /// The sum of the allocations (payments and credit notes).
    pub allocated_total: i64,
    /// What is still open: gross total minus allocations.
    pub balance: i64,
    /// The user who issued it.
    pub owner_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: InvoiceStatus,
    /// Whether the party disputes it.
    pub disputed: bool,
    /// The open dispute (`disputed` is set exactly while there is one).
    pub dispute_id: Option<i64>,
    /// The last day chasing stays paused after an upheld dispute.
    pub dispute_grace_until: Option<NaiveDate>,
    /// The open balance written off as a bad debt.
    pub write_off_amount: Option<i64>,
    /// The write-off reason code, e.g. `insolvent`.
    pub write_off_reason_code: Option<String>,
    /// The text given with the reason (`other`).
    pub write_off_reason_text: Option<String>,
    /// The approval request that approved the write-off, if one was needed.
    pub write_off_approval_id: Option<i64>,
    /// The write-off date (Asia/Dubai).
    pub write_off_date: Option<NaiveDate>,
    /// The date the customer write-off notice was sent.
    pub write_off_notice_on: Option<NaiveDate>,
    /// What was recovered after the write-off.
    pub write_off_recovered_amount: i64,
    /// Whether the write-off waits for the accounting export.
    pub write_off_export_pending: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Invoice {
    /// The invoice's id in the kernel contract.
    pub fn invoice_id(&self) -> InvoiceId {
        InvoiceId(self.id)
    }

    /// The invoice as a [`Subject`]: `invoicing.invoice:42`.
    pub fn subject(&self) -> Subject {
        self.invoice_id().subject()
    }

    /// Who is billed.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// The record it bills for.
    pub fn source(&self) -> Subject {
        Subject {
            subject_type: self.source_type.clone(),
            subject_id: self.source_id,
        }
    }

    /// The scope unit, if any.
    pub fn scope(&self) -> Option<Subject> {
        match (&self.scope_type, self.scope_id) {
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

    /// Stores `totals` as the invoice's totals and resets the balance to
    /// gross minus what is allocated.
    pub fn set_totals(&mut self, totals: &VatTotals) {
        self.net_total = totals.net;
        self.vat_total = totals.vat;
        self.gross_total = totals.gross;
        self.vat_total_aed = totals.vat_aed;
        self.balance = totals.gross - self.allocated_total;
    }

    /// Whether chasing is paused on `today` (`FIN.RUL.01`, escalation,
    /// write-off): the invoice is disputed, or `today` is before the end of
    /// the grace period after an upheld dispute.
    pub fn chasing_paused(&self, today: NaiveDate) -> bool {
        self.disputed || self.dispute_grace_until.is_some_and(|until| today < until)
    }

    /// What other modules see of it.
    pub fn summary(&self) -> InvoiceSummary {
        InvoiceSummary {
            id: self.invoice_id(),
            number: self.number.clone(),
            status: self.status,
            party: self.party(),
            legal_entity_id: self.legal_entity_id,
            currency: self.currency.clone(),
            gross_total: self.gross_total,
            balance: self.balance,
            due_date: self.due_date,
            disputed: self.disputed,
            chasing_paused_until: self.dispute_grace_until,
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
        let kinds: Vec<_> = InvoiceKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "tax_invoice",
                "simplified_tax_invoice",
                "commercial_invoice"
            ]
        );
    }

    #[test]
    fn totals_set_the_balance_and_the_summary() {
        let mut invoice = Invoice {
            id: 4,
            party_id: 7,
            source_type: "property.lease".into(),
            source_id: 9,
            currency: "AED".into(),
            allocated_total: 42_000,
            ..Default::default()
        };
        let line =
            LineAmounts::compute(1_000, 100_000, 0, VatCategory::Standard, invoice.aed_rate());
        invoice.set_totals(&VatTotals::of(&[line]));
        assert_eq!(
            (invoice.net_total, invoice.vat_total, invoice.gross_total),
            (100_000, 5_000, 105_000)
        );
        assert_eq!(invoice.vat_total_aed, Some(5_000));
        assert_eq!(invoice.balance, 63_000);
        let summary = invoice.summary();
        assert_eq!(summary.id, InvoiceId(4));
        assert_eq!(summary.party, PartyId(7));
        assert_eq!(summary.status, InvoiceStatus::Draft);
        assert_eq!((summary.gross_total, summary.balance), (105_000, 63_000));
        assert_eq!(invoice.source(), Subject::new("property.lease", 9));
        assert_eq!(invoice.subject().to_string(), "invoicing.invoice:4");
        assert_eq!(invoice.scope(), None);
    }

    #[test]
    fn chasing_is_paused_while_disputed_and_through_the_grace_period() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        let mut invoice = Invoice {
            disputed: true,
            dispute_id: Some(1),
            ..Default::default()
        };
        assert!(invoice.chasing_paused(day(20)));
        invoice.disputed = false;
        invoice.dispute_id = None;
        assert!(!invoice.chasing_paused(day(20)));
        invoice.dispute_grace_until = Some(day(27));
        assert!(invoice.chasing_paused(day(26)));
        assert!(!invoice.chasing_paused(day(27)));
        assert_eq!(invoice.summary().chasing_paused_until, Some(day(27)));
    }

    #[test]
    fn a_party_snapshot_copies_the_billing_identity() {
        let identity = BillingIdentity {
            party: PartyId(7),
            legal_name_en: "Blue Dunes LLC".into(),
            legal_name_ar: None,
            trn: Some("100000000000003".into()),
            billing_address: Some("Dubai".into()),
            billing_email: Some("ap@bluedunes.ae".into()),
            einvoice_recipient_id: None,
        };
        let snapshot = PartySnapshot::of(&identity);
        assert_eq!(snapshot.legal_name_en, "Blue Dunes LLC");
        assert_eq!(snapshot.trn.as_deref(), Some("100000000000003"));
        assert_eq!(snapshot.billing_email.as_deref(), Some("ap@bluedunes.ae"));
    }
}
