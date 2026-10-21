//! The payer bill contract (wl-invoicing §3 *Payer bill*, §4, §6
//! `PayerBilling`): how a vertical that bills a third-party payer (the
//! clinic claim of 05, the insurance claim of 08) drives the money side of
//! its claim without depending on the `wl-invoicing` crate.
//!
//! The vertical keeps its own coding, validation and submission channel and
//! calls the [`PayerBills`] of [`payer_bills`] for what the payer bill
//! does: `issue` when the claim is submitted, `submission_rejected` when the
//! channel refuses it before adjudication, `resubmit`, the remittance and
//! the recoupment, `write_off`, `transfer_to_customer` and `void`. Without
//! the `invoicing` module every call refuses with a 422 and nothing is
//! found ([`NoPayerBills`]).
//!
//! A call that needs a second person (an overpayment, a recoupment or a
//! write-off above the payer agreement's limit, `FIN.ACL.16`, `FIN.ACL.14`)
//! answers [`PayerAnswer::Pending`]: the approval request waits and the
//! payer bill stays as it is. The events are emitted after the commit of the
//! transition that caused them.
//!
//! Money is `i64` fils of the payer bill's currency.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::invoicing::{InvoiceId, VatCategory};
use crate::numbering::{Reset, Series};
use crate::parties::PartyId;
use crate::subject::Subject;

/// The subject type of a payer bill, for [`Subject`] references.
pub const PAYER_BILL_SUBJECT_TYPE: &str = "invoicing.payer_bill";

/// The subject type of a payer's credit balance (what a refund of it names
/// as its source).
pub const PAYER_CREDIT_SUBJECT_TYPE: &str = "invoicing.payer_credit";

/// Payer bill numbers when the client has more than one active legal
/// entity: `PB-ACME-2026-0001`; gapless, per entity, reset yearly (take them
/// with [`Series::next`] inside the `issue` transaction).
pub const PAYER_BILL_SERIES: Series = Series {
    key: "payer_bill",
    pattern: "{prefix}-{entity}-{yyyy}-{0000}",
    prefix: "PB",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: true,
};

/// Payer bill numbers when the client has exactly one active legal entity:
/// `PB-2026-0001`. Same key and count as [`PAYER_BILL_SERIES`].
pub const PAYER_BILL_SERIES_SINGLE: Series = Series {
    key: "payer_bill",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "PB",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: true,
};

/// The payer bill series for a client with `active_entities` active legal
/// entities: [`PAYER_BILL_SERIES_SINGLE`] for exactly one, else
/// [`PAYER_BILL_SERIES`].
pub fn payer_bill_series(active_entities: usize) -> &'static Series {
    if active_entities == 1 {
        &PAYER_BILL_SERIES_SINGLE
    } else {
        &PAYER_BILL_SERIES
    }
}

/// The id of a payer bill (wl-invoicing §3).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct PayerBillId(pub i64);

impl PayerBillId {
    /// The payer bill as a [`Subject`]: `invoicing.payer_bill:7`.
    pub fn subject(self) -> Subject {
        Subject::new(PAYER_BILL_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for PayerBillId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for PayerBillId {
    fn from(id: i64) -> Self {
        PayerBillId(id)
    }
}

/// The lifecycle state of a payer bill (wl-invoicing §4); the same seven
/// states, with the same stored names, as the `wl-invoicing` machine.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PayerBillStatus {
    /// Built from the payer split; not yet sent (or sent back after a
    /// technical rejection).
    #[default]
    Draft,
    /// Sent or submitted to the payer; waiting for the remittance.
    Issued,
    /// Remittance received: some lines paid, some denied or reduced.
    PartiallyPaid,
    /// Remittance received: every line denied.
    Denied,
    /// Every line paid as billed; only a recoupment leaves it.
    Paid,
    /// Open amount written off or moved to the customer; only a recoupment
    /// leaves it.
    Closed,
    /// Created in error before it reached the payer (terminal).
    Voided,
}

impl PayerBillStatus {
    /// Whether the payer owes something on a bill in this state: it was
    /// submitted and a remittance has not settled it.
    pub fn is_receivable(self) -> bool {
        matches!(
            self,
            PayerBillStatus::Issued | PayerBillStatus::PartiallyPaid | PayerBillStatus::Denied
        )
    }
}

/// How a closed payer bill ended (`PayerBillClosed`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PayerBillOutcome {
    /// The open amount moved to the customer.
    #[default]
    Transferred,
    /// The open amount was written off.
    WrittenOff,
    /// The bill was created in error and voided.
    Voided,
}

/// What other modules see of a payer bill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PayerBillSummary {
    /// The id.
    pub id: PayerBillId,
    /// The number; `None` until the first `issue`.
    pub number: Option<String>,
    /// The state now.
    pub status: PayerBillStatus,
    /// The payer (insurer).
    pub payer: PartyId,
    /// The customer (patient, vehicle owner).
    pub customer: PartyId,
    /// The issuing legal entity.
    pub legal_entity_id: i64,
    /// What the bill is for (a clinic claim, an insurance claim).
    pub source: Subject,
    /// The ISO currency code.
    pub currency: String,
    /// The sum of the lines' claimed amounts.
    pub claimed: i64,
    /// What the payer paid and kept.
    pub paid: i64,
    /// What the payer denied and is still open.
    pub denied: i64,
    /// What is still open: claimed less paid, written off and transferred.
    pub open: i64,
    /// How many times it was sent again after a remittance.
    pub resubmission_count: i64,
    /// Whether a remittance was ever recorded on it.
    pub has_remittance: bool,
    /// Whether it was closed from `draft` for a missed first-submission
    /// window (*not submitted in time*).
    pub not_submitted_in_time: bool,
}

/// One line of a payer bill to make: what the payer is asked to pay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPayerBillLine {
    /// What is claimed.
    pub description: String,
    /// The payer's share of the line, net of VAT, in fils.
    pub claimed: i64,
    /// The VAT treatment on the tax document that carries the line.
    pub vat_category: VatCategory,
    /// The line of the customer's tax invoice that carries it, when the
    /// customer is the VAT recipient.
    pub invoice_line: Option<i64>,
}

impl NewPayerBillLine {
    /// A zero-rated line of `description` claiming `claimed` fils.
    pub fn new(description: &str, claimed: i64) -> Self {
        NewPayerBillLine {
            description: description.trim().to_owned(),
            claimed,
            vat_category: VatCategory::ZeroRated,
            invoice_line: None,
        }
    }

    /// With this VAT treatment.
    pub fn vat_category(mut self, category: VatCategory) -> Self {
        self.vat_category = category;
        self
    }

    /// Carried by line `position` of the customer's tax invoice.
    pub fn on_invoice_line(mut self, position: i64) -> Self {
        self.invoice_line = Some(position);
        self
    }
}

/// A payer bill to make from a payer split: [`PayerBills::create`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPayerBill {
    /// The payer agreement (its id in `wl-invoicing`) the bill follows.
    pub agreement: i64,
    /// The customer: the patient, the vehicle owner.
    pub customer: PartyId,
    /// What it is for (a clinic claim, an insurance claim).
    pub source: Subject,
    /// The day of the service; the agreement's windows count from it.
    pub service_date: NaiveDate,
    /// The payer reference (prior authorisation, claim number), if any.
    pub payer_reference: Option<String>,
    /// The customer's tax invoice that carries the payer share, if any.
    pub customer_invoice: Option<InvoiceId>,
    /// The ISO currency code.
    pub currency: String,
    /// 05 `CLN.ACL.14`: the bill may be voided only before its first
    /// submission.
    pub void_before_first_submission: bool,
    /// The lines.
    pub lines: Vec<NewPayerBillLine>,
}

impl NewPayerBill {
    /// A bill in AED under `agreement` for `customer` about `source`, for a
    /// service on `service_date`, with no line yet.
    pub fn new(agreement: i64, customer: PartyId, source: Subject, service_date: NaiveDate) -> Self {
        NewPayerBill {
            agreement,
            customer,
            source,
            service_date,
            payer_reference: None,
            customer_invoice: None,
            currency: crate::invoicing::DEFAULT_CURRENCY.to_owned(),
            void_before_first_submission: false,
            lines: Vec::new(),
        }
    }

    /// With this payer reference.
    pub fn payer_reference(mut self, reference: &str) -> Self {
        self.payer_reference = Some(reference.trim().to_owned()).filter(|r| !r.is_empty());
        self
    }

    /// With the customer's tax invoice that carries the payer share.
    pub fn customer_invoice(mut self, invoice: InvoiceId) -> Self {
        self.customer_invoice = Some(invoice);
        self
    }

    /// Voidable only before the first submission (05).
    pub fn void_before_first_submission(mut self) -> Self {
        self.void_before_first_submission = true;
        self
    }

    /// With `line` added.
    pub fn line(mut self, line: NewPayerBillLine) -> Self {
        self.lines.push(line);
        self
    }
}

/// One error the submission channel gave for a rejected submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmissionError {
    /// The channel's error code, e.g. `E104`.
    pub code: String,
    /// The channel's message, e.g. `invalid member ID`.
    pub message: String,
}

impl SubmissionError {
    /// The error `code` with `message`.
    pub fn new(code: &str, message: &str) -> Self {
        SubmissionError {
            code: code.trim().to_owned(),
            message: message.trim().to_owned(),
        }
    }
}

/// What a technical rejection came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectionDeadline {
    /// The last day the corrected bill can be sent (Asia/Dubai, inclusive):
    /// the end of the first-submission window, or of the resubmission window
    /// after a remittance. `None` when the agreement gives no window.
    pub deadline: Option<NaiveDate>,
}

/// How much of one payer bill line a remittance pays, denies or recoups.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemittanceLine {
    /// The payer bill line.
    pub line: i64,
    /// What the payer paid on it, in all. What is above the line's open
    /// amount (less what the payer denied) is an overpayment: the remittance
    /// is held for a second person's approval and the excess goes to the
    /// payer credit balance (`FIN.ACL.16`).
    pub paid: i64,
    /// What the payer denied on it.
    pub denied: i64,
    /// The payer's denial code, if it denied something.
    pub code: Option<String>,
    /// The payer's reason for the denial.
    pub reason: Option<String>,
}

impl RemittanceLine {
    /// `paid` fils paid on `line`, nothing denied.
    pub fn paid(line: i64, paid: i64) -> Self {
        RemittanceLine {
            line,
            paid,
            denied: 0,
            code: None,
            reason: None,
        }
    }

    /// With `denied` fils denied under `code` and `reason`.
    pub fn denied(mut self, denied: i64, code: &str, reason: &str) -> Self {
        self.denied = denied;
        self.code = Some(code.trim().to_owned()).filter(|c| !c.is_empty());
        self.reason = Some(reason.trim().to_owned()).filter(|r| !r.is_empty());
        self
    }

}

/// The payer's remittance advice for one payer bill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remittance {
    /// The payer's reference; unique per payer.
    pub reference: String,
    /// The advice's date.
    pub date: NaiveDate,
    /// The wl-payments payment that carried the money, if recorded there.
    pub payment: Option<i64>,
    /// What it pays, denies and overpays, per payer bill line.
    pub lines: Vec<RemittanceLine>,
}

impl Remittance {
    /// The advice `reference` dated `date`, with no line yet.
    pub fn new(reference: &str, date: NaiveDate) -> Self {
        Remittance {
            reference: reference.trim().to_owned(),
            date,
            payment: None,
            lines: Vec::new(),
        }
    }

    /// Carried by wl-payments payment `payment`.
    pub fn payment(mut self, payment: i64) -> Self {
        self.payment = Some(payment);
        self
    }

    /// With `line` added.
    pub fn line(mut self, line: RemittanceLine) -> Self {
        self.lines.push(line);
        self
    }
}

/// What a payer takes back from one payer bill line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoupedLine {
    /// The payer bill line.
    pub line: i64,
    /// The amount recouped, in fils.
    pub amount: i64,
}

/// A recoupment: money the payer paid earlier and now takes back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recoupment {
    /// The lines and the amounts recouped.
    pub lines: Vec<RecoupedLine>,
    /// The payer's recoupment code, e.g. `R07`.
    pub code: String,
    /// The payer's reason, if it gave one.
    pub reason: Option<String>,
    /// The reference of the later remittance that carried the recoupment.
    pub carrier_reference: Option<String>,
    /// The date of that remittance.
    pub carrier_date: Option<NaiveDate>,
}

impl Recoupment {
    /// A recoupment under `code`, with no line yet.
    pub fn new(code: &str) -> Self {
        Recoupment {
            lines: Vec::new(),
            code: code.trim().to_owned(),
            reason: None,
            carrier_reference: None,
            carrier_date: None,
        }
    }

    /// With `amount` taken back from `line`.
    pub fn line(mut self, line: i64, amount: i64) -> Self {
        self.lines.push(RecoupedLine { line, amount });
        self
    }

    /// With the payer's `reason`.
    pub fn reason(mut self, reason: &str) -> Self {
        self.reason = Some(reason.trim().to_owned()).filter(|r| !r.is_empty());
        self
    }

    /// Carried by the remittance `reference` dated `date`.
    pub fn carried_by(mut self, reference: &str, date: NaiveDate) -> Self {
        self.carrier_reference = Some(reference.trim().to_owned());
        self.carrier_date = Some(date);
        self
    }
}

/// What a call that may need approval came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayerAnswer {
    /// The transition ran.
    Done,
    /// A second person must approve first: the approval request waits and
    /// the payer bill is as it was. The transition runs when the last
    /// approver says yes.
    Pending {
        /// The approval request.
        request: i64,
    },
}

/// The payer bill was issued (`issue`), the first time or again after a
/// technical rejection or a remittance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayerBillIssued {
    /// The payer bill.
    pub payer_bill: PayerBillId,
    /// The payer.
    pub payer: PartyId,
    /// What it is for.
    pub source: Subject,
    /// The sum of the claimed amounts.
    pub claimed: i64,
    /// What is still open.
    pub open: i64,
}

impl Event for PayerBillIssued {}

/// The submission channel rejected the payer bill before adjudication
/// (`submission_rejected`); it is a `draft` again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayerBillSubmissionRejected {
    /// The payer bill.
    pub payer_bill: PayerBillId,
    /// The payer.
    pub payer: PartyId,
    /// What it is for.
    pub source: Subject,
    /// The channel's error codes.
    pub error_codes: Vec<String>,
}

impl Event for PayerBillSubmissionRejected {}

/// A remittance was recorded on the payer bill (`record_remittance`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayerRemittanceRecorded {
    /// The payer bill.
    pub payer_bill: PayerBillId,
    /// The payer.
    pub payer: PartyId,
    /// What it is for.
    pub source: Subject,
    /// The sum of the claimed amounts.
    pub claimed: i64,
    /// What the payer paid on the bill so far.
    pub paid: i64,
    /// What the payer denied and is still open.
    pub denied: i64,
    /// What is still open.
    pub open: i64,
    /// The payer's reference of the remittance.
    pub reference: String,
    /// What went to the payer credit balance (an approved overpayment).
    pub overpayment: i64,
}

impl Event for PayerRemittanceRecorded {}

/// A payer took money back from the payer bill (`record_recoupment`); the
/// recouped lines are open again as denied. The vertical's denial task
/// (05 `CLN.RUL.05`) reacts to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayerRecoupmentRecorded {
    /// The payer bill.
    pub payer_bill: PayerBillId,
    /// The payer.
    pub payer: PartyId,
    /// What it is for.
    pub source: Subject,
    /// The lines and the amounts recouped.
    pub lines: Vec<RecoupedLine>,
    /// The payer's recoupment code (`payment_reversed` for a reversal).
    pub code: String,
    /// The total recouped.
    pub amount: i64,
    /// The part set off from the payer credit balance.
    pub from_credit: i64,
    /// The part deducted from the later remittance that carried it.
    pub deducted: i64,
    /// Whether a reversed payment caused it (no approval was needed).
    pub reversal: bool,
}

impl Event for PayerRecoupmentRecorded {}

/// The payer bill was closed: its open amount moved to the customer or was
/// written off, or it was voided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayerBillClosed {
    /// The payer bill.
    pub payer_bill: PayerBillId,
    /// The payer.
    pub payer: PartyId,
    /// What it is for.
    pub source: Subject,
    /// The sum of the claimed amounts.
    pub claimed: i64,
    /// What the payer paid.
    pub paid: i64,
    /// What the payer denied.
    pub denied: i64,
    /// The open amount at the close (moved, written off or dropped).
    pub open: i64,
    /// How it ended.
    pub outcome: PayerBillOutcome,
    /// Whether it was closed from `draft` for a missed first-submission
    /// window.
    pub not_submitted_in_time: bool,
}

impl Event for PayerBillClosed {}

/// The future a [`PayerBills`] method returns.
pub type PayerBillFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The 422 of a payer bill action while invoicing is off.
fn payer_bills_off(what: &str) -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        format!("{what}: payer billing is not available, invoicing is not enabled"),
    )
}

/// Drives payer bills for a vertical. `wl-invoicing` provides it with
/// [`provide_payer_bills`]; without that module [`payer_bills`] gives
/// [`NoPayerBills`]. Every action method has a default body that refuses
/// with a 422 and every read a default that finds nothing, so a fake only
/// writes the methods it needs.
///
/// `by` is the signed-in user, who needs the transition's permission
/// (`payer_bills.issue`, `payer_bills.reconcile`, `payer_bills.write_off`);
/// `None` is the system where the lifecycle allows it.
pub trait PayerBills: Send + Sync + 'static {
    /// The payer bill `id`.
    fn find<'a>(
        &'a self,
        _db: &'a Db,
        _id: PayerBillId,
    ) -> PayerBillFuture<'a, Option<PayerBillSummary>> {
        Box::pin(async { Ok(None) })
    }

    /// The payer bills of `source` that are not voided, oldest first.
    fn of_source<'a>(
        &'a self,
        _db: &'a Db,
        _source: &'a Subject,
    ) -> PayerBillFuture<'a, Vec<PayerBillSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Makes a `draft` payer bill from a payer split, under the payer
    /// agreement `request.agreement`, and returns its id. Refused (422) for
    /// an unknown or inactive agreement, a bill without lines or with a line
    /// of 0 or less, and when a payer bill of that agreement for the same
    /// source is not voided yet.
    fn create<'a>(
        &'a self,
        _state: &'a AppState,
        _request: NewPayerBill,
        _by: Option<&'a AuthUser>,
    ) -> PayerBillFuture<'a, PayerBillId> {
        Box::pin(async { Err(payer_bills_off("the payer bill cannot be made")) })
    }

    /// Changes what a `draft` payer bill claims on `line` to `claimed` fils
    /// (a corrected amount after a technical rejection). The tax documents
    /// follow at the next `issue`: a credit note for a lower amount, an
    /// extra invoice for a higher one.
    fn correct_line<'a>(
        &'a self,
        _state: &'a AppState,
        _id: PayerBillId,
        _line: i64,
        _claimed: i64,
    ) -> PayerBillFuture<'a, ()> {
        Box::pin(async { Err(payer_bills_off("the payer bill cannot be corrected")) })
    }

    /// Issues the `draft` payer bill (05 claim `submit`): the first time it
    /// gets its number; after a technical rejection it keeps it.
    fn issue<'a>(
        &'a self,
        _state: &'a AppState,
        _id: PayerBillId,
        _by: Option<&'a AuthUser>,
    ) -> PayerBillFuture<'a, ()> {
        Box::pin(async { Err(payer_bills_off("the payer bill cannot be issued")) })
    }

    /// The submission channel rejected the issued payer bill before
    /// adjudication (05 claim `reject_submission`): back to `draft` with the
    /// channel's `errors`, the resubmission count unchanged. Answers the
    /// deadline the coordinator's task carries.
    fn submission_rejected<'a>(
        &'a self,
        _state: &'a AppState,
        _id: PayerBillId,
        _errors: Vec<SubmissionError>,
        _by: Option<&'a AuthUser>,
    ) -> PayerBillFuture<'a, RejectionDeadline> {
        Box::pin(async { Err(payer_bills_off("the rejection cannot be recorded")) })
    }

    /// Sends a `partially_paid` or `denied` payer bill again (05 claim
    /// `resubmit`) once the `corrections` are written down; the
    /// resubmission count goes up by one.
    fn resubmit<'a>(
        &'a self,
        _state: &'a AppState,
        _id: PayerBillId,
        _corrections: &'a str,
        _by: Option<&'a AuthUser>,
    ) -> PayerBillFuture<'a, ()> {
        Box::pin(async { Err(payer_bills_off("the payer bill cannot be resubmitted")) })
    }

    /// Records the payer's remittance advice on the issued payer bill (05
    /// claim `record_remittance`). An overpayment waits for a second
    /// person's approval (`FIN.ACL.16`): the answer is then
    /// [`PayerAnswer::Pending`] and nothing is recorded.
    fn record_remittance<'a>(
        &'a self,
        _state: &'a AppState,
        _id: PayerBillId,
        _remittance: Remittance,
        _by: &'a AuthUser,
    ) -> PayerBillFuture<'a, PayerAnswer> {
        Box::pin(async { Err(payer_bills_off("the remittance cannot be recorded")) })
    }

    /// Records money the payer took back from the earlier payer bill (05
    /// claim `record_recoupment`), with a second person's approval
    /// (`FIN.ACL.16`).
    fn record_recoupment<'a>(
        &'a self,
        _state: &'a AppState,
        _id: PayerBillId,
        _recoupment: Recoupment,
        _by: &'a AuthUser,
    ) -> PayerBillFuture<'a, PayerAnswer> {
        Box::pin(async { Err(payer_bills_off("the recoupment cannot be recorded")) })
    }

    /// Writes off the open amount of the `draft`, `partially_paid` or
    /// `denied` payer bill (05 claim `write_off` when the contract does not
    /// allow billing the patient). Above the agreement's write-off limit
    /// the answer is [`PayerAnswer::Pending`] (`FIN.ACL.14`).
    fn write_off<'a>(
        &'a self,
        _state: &'a AppState,
        _id: PayerBillId,
        _reason: &'a str,
        _by: &'a AuthUser,
    ) -> PayerBillFuture<'a, PayerAnswer> {
        Box::pin(async { Err(payer_bills_off("the payer bill cannot be written off")) })
    }

    /// Moves the open amount of the `draft`, `partially_paid` or `denied`
    /// payer bill to the customer where the payer agreement allows it.
    fn transfer_to_customer<'a>(
        &'a self,
        _state: &'a AppState,
        _id: PayerBillId,
        _reason: &'a str,
        _by: &'a AuthUser,
    ) -> PayerBillFuture<'a, ()> {
        Box::pin(async { Err(payer_bills_off("the payer bill cannot be moved")) })
    }

    /// Voids the `draft` payer bill created in error (05 claim `void`):
    /// the number is kept, the payer split is released and the source can be
    /// split again.
    fn void<'a>(
        &'a self,
        _state: &'a AppState,
        _id: PayerBillId,
        _reason: &'a str,
        _by: &'a AuthUser,
    ) -> PayerBillFuture<'a, PayerAnswer> {
        Box::pin(async { Err(payer_bills_off("the payer bill cannot be voided")) })
    }
}

/// The payer bills when `invoicing` is off: none exist and every action is
/// refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPayerBills;

impl PayerBills for NoPayerBills {}

/// Makes `bills` the app's [`PayerBills`]; `wl-invoicing` calls it in
/// `register`.
pub fn provide_payer_bills(app: &mut Registry, bills: impl PayerBills) {
    let bills: Arc<dyn PayerBills> = Arc::new(bills);
    app.provide(bills);
}

/// The app's [`PayerBills`], or [`NoPayerBills`] when none is provided.
pub fn payer_bills(state: &AppState) -> Arc<dyn PayerBills> {
    match state.provided::<Arc<dyn PayerBills>>() {
        Some(bills) => Arc::clone(&bills),
        None => Arc::new(NoPayerBills),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payer_bill_id_is_a_subject_and_a_plain_number() {
        let id = PayerBillId(42);
        assert_eq!(id.subject().to_string(), "invoicing.payer_bill:42");
        assert_eq!(renox::serde_json::to_string(&id).unwrap(), "42");
        assert_eq!(PayerBillId::from(7), PayerBillId(7));
    }

    #[test]
    fn the_series_is_gapless_per_entity_and_yearly() {
        assert!(PAYER_BILL_SERIES.gapless && PAYER_BILL_SERIES.per_entity);
        assert_eq!(PAYER_BILL_SERIES.reset, Reset::Yearly);
        assert_eq!(payer_bill_series(1), &PAYER_BILL_SERIES_SINGLE);
        assert_eq!(payer_bill_series(2), &PAYER_BILL_SERIES);
        assert_eq!(PAYER_BILL_SERIES.key, PAYER_BILL_SERIES_SINGLE.key);
    }

    #[test]
    fn states_are_stored_in_snake_case_and_three_are_receivable() {
        let names: Vec<_> = PayerBillStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            names,
            [
                "draft",
                "issued",
                "partially_paid",
                "denied",
                "paid",
                "closed",
                "voided"
            ]
        );
        let receivable: Vec<_> = PayerBillStatus::ALL
            .iter()
            .filter(|s| s.is_receivable())
            .map(|s| s.as_str())
            .collect();
        assert_eq!(receivable, ["issued", "partially_paid", "denied"]);
        let outcomes: Vec<_> = PayerBillOutcome::ALL.iter().map(|o| o.as_str()).collect();
        assert_eq!(outcomes, ["transferred", "written_off", "voided"]);
    }

    #[test]
    fn a_remittance_is_built_with_its_builders() {
        let day = NaiveDate::from_ymd_opt(2026, 12, 10).unwrap();
        let remittance = Remittance::new(" ADV-9 ", day)
            .payment(5)
            .line(RemittanceLine::paid(1, 45_000))
            .line(RemittanceLine::paid(2, 0).denied(10_000, " R12 ", "not covered"));
        assert_eq!(remittance.reference, "ADV-9");
        assert_eq!(remittance.payment, Some(5));
        assert_eq!(remittance.lines[0].paid, 45_000);
        assert_eq!(remittance.lines[1].code.as_deref(), Some("R12"));
        let recoupment = Recoupment::new("R07")
            .line(1, 40_000)
            .reason(" taken back ")
            .carried_by("ADV-10", day);
        assert_eq!(recoupment.lines, [RecoupedLine { line: 1, amount: 40_000 }]);
        assert_eq!(recoupment.reason.as_deref(), Some("taken back"));
        assert_eq!(recoupment.carrier_reference.as_deref(), Some("ADV-10"));
    }

    #[test]
    fn the_events_round_trip_through_json() {
        let closed = PayerBillClosed {
            payer_bill: PayerBillId(3),
            payer: PartyId(9),
            source: Subject::new("clinics.claim", 12),
            claimed: 60_000,
            paid: 0,
            denied: 0,
            open: 60_000,
            outcome: PayerBillOutcome::WrittenOff,
            not_submitted_in_time: true,
        };
        let json = renox::serde_json::to_value(&closed).unwrap();
        assert_eq!(json["payer_bill"], 3);
        assert_eq!(json["outcome"], "written_off");
        let back: PayerBillClosed = renox::serde_json::from_value(json).unwrap();
        assert_eq!(back, closed);
    }
}
