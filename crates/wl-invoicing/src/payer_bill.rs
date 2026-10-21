//! The Payer bill (wl-invoicing §3, §4): what one payer (an insurer) is asked
//! to pay for one source (a clinic claim, an insurance claim), with its
//! lines ([`PayerBillLine`]). Its status moves only through the machine in
//! [`crate::payer_bill_machine`]; the services that gather what the guards
//! need are in [`crate::payer_issue`], [`crate::payer_rejection`],
//! [`crate::payer_remittance`], [`crate::payer_recoupment`] and
//! [`crate::payer_closing`].
//!
//! Decisions:
//!
//! - The line amounts are the net amounts of the tax document line that
//!   carries them. The clinic lines are zero-rated or exempt, so net and
//!   gross are the same there; a standard-rated payer line gets its VAT on
//!   the payer's tax invoice, not in the claimed amount.
//! - A line is *open* for what is claimed less what the payer paid, what was
//!   written off and what moved to the customer. `denied` is the part of the
//!   open amount the payer refused (it can still be resubmitted).
//! - The bill's totals are the sums of its lines and are rewritten by the
//!   transitions that change a line; nothing else writes them.

use renox::chrono::NaiveDate;
use renox::db::Json;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::invoicing::VatCategory;
use wl_kernel::payer_bills::{
    PAYER_BILL_SUBJECT_TYPE, PayerBillId, PayerBillStatus, PayerBillSummary, RecoupedLine,
};
use wl_kernel::{PartyId, Subject};

use crate::payer_agreement::VatRecipient;

/// A payer bill. Money is `i64` fils of [`PayerBill::currency`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_payer_bills",
    state_machine,
    unique(number),
    index(status),
    index(source_type, source_id),
    index(payer_party_id)
)]
pub struct PayerBill {
    /// The primary key.
    pub id: i64,
    /// The number from the `payer_bill` series, set at the first `issue` and
    /// kept; `None` while the bill was never issued.
    pub number: Option<String>,
    /// The issuing legal entity.
    pub legal_entity_id: i64,
    /// The payer agreement it follows.
    pub agreement_id: i64,
    /// The payer (a party).
    pub payer_party_id: i64,
    /// The customer (a party): the patient, the vehicle owner.
    pub customer_party_id: i64,
    /// The type of the source (a clinic claim, an insurance claim).
    pub source_type: String,
    /// The id of the source.
    pub source_id: i64,
    /// Who the tax invoice is addressed to.
    pub vat_recipient: VatRecipient,
    /// The payer reference (prior authorisation, claim number), if any.
    pub payer_reference: Option<String>,
    /// The ISO currency code.
    pub currency: String,
    /// The day of the service: the windows of the payer agreement count from
    /// it.
    pub service_date: NaiveDate,
    /// The customer's tax invoice that carries the payer share.
    pub customer_invoice_id: Option<i64>,
    /// The payer's own tax invoice (the payer is the recipient).
    pub payer_invoice_id: Option<i64>,
    /// The sum of the lines' claimed amounts.
    pub claimed_total: i64,
    /// What the payer paid and kept.
    pub paid_total: i64,
    /// What the payer denied and is still open.
    pub denied_total: i64,
    /// What was written off.
    pub written_off_total: i64,
    /// What moved to the customer.
    pub transferred_total: i64,
    /// What is still open: claimed less paid, written off and transferred.
    pub balance: i64,
    /// How many times it was sent again after a remittance.
    pub resubmission_count: i64,
    /// How many times it was issued.
    pub issue_count: i64,
    /// Whether a remittance was ever recorded on it.
    pub has_remittance: bool,
    /// The date of the last remittance recorded on it.
    pub last_remittance_on: Option<NaiveDate>,
    /// Whether the receivable counts in the ageing.
    pub in_ageing: bool,
    /// 05 `CLN.ACL.14`: a void is allowed only before the first submission.
    pub void_before_first_submission: bool,
    /// Closed from `draft` for a missed first-submission window.
    pub not_submitted_in_time: bool,
    /// Whether a tax document (the payer's invoice, a credit note, an extra
    /// invoice) still has to be issued for it.
    pub tax_document_pending: bool,
    /// Why it was written off or moved to the customer.
    pub close_reason: Option<String>,
    /// How it ended: `transferred`, `written_off` or `voided`.
    pub close_outcome: Option<String>,
    /// The amount that moved at the close, in fils.
    pub close_amount: i64,
    /// The lines and the amounts that moved at the close.
    pub close_lines: Json<Vec<RecoupedLine>>,
    /// The credit note the close made, once made.
    pub close_credit_note_id: Option<i64>,
    /// The tax invoice to the customer the close made, once made.
    pub close_invoice_id: Option<i64>,
    /// When it was written off or moved.
    pub closed_at: Option<DateTime>,
    /// The user who wrote it off or moved it; `None` for the system.
    pub closed_by: Option<i64>,
    /// The user who made the bill; `None` for the system.
    pub created_by: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: PayerBillStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PayerBill {
    /// The id in the kernel contract.
    pub fn payer_bill_id(&self) -> PayerBillId {
        PayerBillId(self.id)
    }

    /// The bill as a [`Subject`]: `invoicing.payer_bill:7`.
    pub fn subject(&self) -> Subject {
        self.payer_bill_id().subject()
    }

    /// The payer.
    pub fn payer(&self) -> PartyId {
        PartyId(self.payer_party_id)
    }

    /// The customer.
    pub fn customer(&self) -> PartyId {
        PartyId(self.customer_party_id)
    }

    /// What the bill is for.
    pub fn source(&self) -> Subject {
        Subject {
            subject_type: self.source_type.clone(),
            subject_id: self.source_id,
        }
    }

    /// The bill as other modules see it.
    pub fn summary(&self) -> PayerBillSummary {
        PayerBillSummary {
            id: self.payer_bill_id(),
            number: self.number.clone(),
            status: self.status,
            payer: self.payer(),
            customer: self.customer(),
            legal_entity_id: self.legal_entity_id,
            source: self.source(),
            currency: self.currency.clone(),
            claimed: self.claimed_total,
            paid: self.paid_total,
            denied: self.denied_total,
            open: self.balance,
            resubmission_count: self.resubmission_count,
            has_remittance: self.has_remittance,
            not_submitted_in_time: self.not_submitted_in_time,
        }
    }

    /// Whether the payer is the VAT recipient.
    pub fn payer_is_recipient(&self) -> bool {
        self.vat_recipient == VatRecipient::Payer
    }
}

/// The subject type of a payer bill: `invoicing.payer_bill`.
pub const SUBJECT_TYPE: &str = PAYER_BILL_SUBJECT_TYPE;

/// One line of a [`PayerBill`]. Money is `i64` fils of the bill's currency.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_payer_bill_lines", unique(payer_bill_id, position))]
pub struct PayerBillLine {
    /// The primary key.
    pub id: i64,
    /// The payer bill.
    pub payer_bill_id: i64,
    /// The order on the bill, from 1.
    pub position: i64,
    /// What is claimed.
    pub description: String,
    /// The VAT treatment on the tax document that carries the line.
    pub vat_category: VatCategory,
    /// The line of the tax document that carries it: the payer's invoice, or
    /// the customer's invoice when the customer is the recipient.
    pub invoice_line_position: Option<i64>,
    /// What the payer is asked to pay.
    pub claimed: i64,
    /// What the payer's tax invoice bills for the line so far.
    pub invoiced: i64,
    /// What the payer paid and kept.
    pub paid: i64,
    /// What the payer denied and is still open.
    pub denied: i64,
    /// What was written off.
    pub written_off: i64,
    /// What moved to the customer.
    pub transferred: i64,
    /// The payer's last denial code.
    pub denial_code: Option<String>,
    /// The payer's last denial reason.
    pub denial_reason: Option<String>,
    /// The payer's last recoupment code.
    pub recoupment_code: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PayerBillLine {
    /// A line `position` of `description` claiming `claimed` fils.
    pub fn new(position: i64, description: &str, claimed: i64) -> Self {
        PayerBillLine {
            position,
            description: description.to_owned(),
            vat_category: VatCategory::ZeroRated,
            claimed,
            ..Default::default()
        }
    }

    /// On the tax document line `position`.
    pub fn on_invoice_line(mut self, position: i64) -> Self {
        self.invoice_line_position = Some(position);
        self
    }

    /// With this VAT treatment.
    pub fn vat_category(mut self, category: VatCategory) -> Self {
        self.vat_category = category;
        self
    }

    /// What is still open on the line: claimed less paid, written off and
    /// moved to the customer.
    pub fn open(&self) -> i64 {
        self.claimed - self.paid - self.written_off - self.transferred
    }
}

/// The totals of `lines`, as the bill stores them.
pub(crate) struct Totals {
    pub claimed: i64,
    pub paid: i64,
    pub denied: i64,
    pub written_off: i64,
    pub transferred: i64,
    pub balance: i64,
}

impl Totals {
    /// The sums over `lines`.
    pub(crate) fn of(lines: &[PayerBillLine]) -> Self {
        let sum = |f: fn(&PayerBillLine) -> i64| lines.iter().map(f).sum::<i64>();
        Totals {
            claimed: sum(|l| l.claimed),
            paid: sum(|l| l.paid),
            denied: sum(|l| l.denied),
            written_off: sum(|l| l.written_off),
            transferred: sum(|l| l.transferred),
            balance: sum(PayerBillLine::open),
        }
    }
}

/// The lines of payer bill `payer_bill_id`, in order.
pub async fn lines_of<'c>(
    db: impl renox::db::Executor<'c>,
    payer_bill_id: i64,
) -> Result<Vec<PayerBillLine>> {
    PayerBillLine::query()
        .where_(PayerBillLine::PAYER_BILL_ID.eq(payer_bill_id))
        .order_by(PayerBillLine::POSITION)
        .get(db)
        .await
}

/// Writes the totals of `lines` to the bill `id` in `tx` (not the status
/// column, so outside the machine's guard).
pub(crate) async fn write_totals_in(
    tx: &mut renox::db::Transaction,
    id: i64,
    lines: &[PayerBillLine],
) -> Result {
    let totals = Totals::of(lines);
    PayerBill::query()
        .where_(PayerBill::ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("claimed_total", &totals.claimed),
                ("paid_total", &totals.paid),
                ("denied_total", &totals.denied),
                ("written_off_total", &totals.written_off),
                ("transferred_total", &totals.transferred),
                ("balance", &totals.balance),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_open_for_what_is_neither_paid_written_off_nor_moved() {
        let mut line = PayerBillLine::new(1, "Consultation", 90_000);
        assert_eq!(line.open(), 90_000);
        line.paid = 40_000;
        line.written_off = 10_000;
        line.transferred = 5_000;
        assert_eq!(line.open(), 35_000);
    }

    #[test]
    fn the_totals_are_the_sums_of_the_lines() {
        let mut a = PayerBillLine::new(1, "A", 60_000);
        a.paid = 60_000;
        let mut b = PayerBillLine::new(2, "B", 40_000);
        b.denied = 40_000;
        let totals = Totals::of(&[a, b]);
        assert_eq!(
            (totals.claimed, totals.paid, totals.denied, totals.balance),
            (100_000, 60_000, 40_000, 40_000)
        );
    }

    #[test]
    fn the_subject_and_the_summary_name_the_bill() {
        let bill = PayerBill {
            id: 7,
            number: Some("PB-2026-0007".into()),
            payer_party_id: 9,
            customer_party_id: 4,
            source_type: "clinics.claim".into(),
            source_id: 12,
            balance: 500,
            ..Default::default()
        };
        assert_eq!(bill.subject().to_string(), "invoicing.payer_bill:7");
        assert_eq!(SUBJECT_TYPE, "invoicing.payer_bill");
        let summary = bill.summary();
        assert_eq!(summary.payer, PartyId(9));
        assert_eq!(summary.customer, PartyId(4));
        assert_eq!(summary.source.to_string(), "clinics.claim:12");
        assert_eq!(summary.open, 500);
        assert_eq!(summary.status, PayerBillStatus::Draft);
    }
}
