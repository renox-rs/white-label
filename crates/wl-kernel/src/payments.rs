//! The payment contract (ARCHITECTURE §5, wl-payments §6): what every
//! module may do with payments without depending on the `wl-payments`
//! crate.
//!
//! `wl-payments` provides a [`PaymentRecorder`] with
//! [`provide_payment_recorder`] and emits [`PaymentReceived`],
//! [`PaymentReversed`], [`RefundPaid`] and [`RefundFailed`]. Other modules record payments,
//! read a party's credit and reverse payments through [`payment_recorder`],
//! which falls back to [`NoPayments`] when the `payments` module is off:
//!
//! ```ignore
//! use wl_kernel::payments;
//!
//! let recorder = payments::payment_recorder(&state);
//! if recorder.credit_balance(&state.db, party, "AED").await? > 0 {
//!     /* offer to apply the credit */
//! }
//! ```
//!
//! Money is `i64` in fils (1/100 of the currency unit) of the payment's
//! currency. A reversal never changes a target: the listener of
//! [`PaymentReversed`] in the target's module reopens it.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::invoicing::{
    CREDIT_NOTE_SUBJECT_TYPE, INVOICE_SUBJECT_TYPE, InvoiceId, SUPPLIER_BILL_SUBJECT_TYPE,
    SUPPLIER_DEBIT_NOTE_SUBJECT_TYPE, SupplierBillId, SupplierDebitNoteId,
};
use crate::numbering::{Reset, Series};
use crate::parties::{PARTY_SUBJECT_TYPE, PartyId};
use crate::subject::Subject;

/// The subject type of a payment, for [`Subject`] references.
pub const PAYMENT_SUBJECT_TYPE: &str = "payments.payment";

/// The subject type of a refund, for [`Subject`] references.
pub const REFUND_SUBJECT_TYPE: &str = "payments.refund";

/// The subject type of a deposit, for [`Subject`] references.
pub const DEPOSIT_SUBJECT_TYPE: &str = "payments.deposit";

/// The currency of a payment that names none.
pub const DEFAULT_CURRENCY: &str = "AED";

/// Receipt numbers (incoming payments) when the client has more than one
/// active legal entity: `RCT-ACME-2026-0001`; gapless, per entity, reset
/// yearly (take them with [`Series::next`] inside the `record`
/// transaction).
pub const RECEIPT_SERIES: Series = Series {
    key: "receipt",
    pattern: "{prefix}-{entity}-{yyyy}-{0000}",
    prefix: "RCT",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: true,
};

/// Receipt numbers when the client has exactly one active legal entity:
/// `RCT-2026-0001`. Same key and count as [`RECEIPT_SERIES`].
pub const RECEIPT_SERIES_SINGLE: Series = Series {
    key: "receipt",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "RCT",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: true,
};

/// Payment voucher numbers (outgoing payments) when the client has more
/// than one active legal entity: `PV-ACME-2026-0001`; gapless, per entity,
/// reset yearly.
pub const PAYMENT_VOUCHER_SERIES: Series = Series {
    key: "payment_voucher",
    pattern: "{prefix}-{entity}-{yyyy}-{0000}",
    prefix: "PV",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: true,
};

/// Payment voucher numbers when the client has exactly one active legal
/// entity: `PV-2026-0001`. Same key and count as
/// [`PAYMENT_VOUCHER_SERIES`].
pub const PAYMENT_VOUCHER_SERIES_SINGLE: Series = Series {
    key: "payment_voucher",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "PV",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: true,
};

/// The receipt series for a client with `active_entities` active legal
/// entities: [`RECEIPT_SERIES_SINGLE`] for exactly one, else
/// [`RECEIPT_SERIES`].
pub fn receipt_series(active_entities: usize) -> &'static Series {
    if active_entities == 1 {
        &RECEIPT_SERIES_SINGLE
    } else {
        &RECEIPT_SERIES
    }
}

/// The payment voucher series for a client with `active_entities` active
/// legal entities: [`PAYMENT_VOUCHER_SERIES_SINGLE`] for exactly one, else
/// [`PAYMENT_VOUCHER_SERIES`].
pub fn payment_voucher_series(active_entities: usize) -> &'static Series {
    if active_entities == 1 {
        &PAYMENT_VOUCHER_SERIES_SINGLE
    } else {
        &PAYMENT_VOUCHER_SERIES
    }
}

/// The id of a payment.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct PaymentId(pub i64);

impl PaymentId {
    /// The payment as a [`Subject`]: `payments.payment:42`.
    pub fn subject(self) -> Subject {
        Subject::new(PAYMENT_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for PaymentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for PaymentId {
    fn from(id: i64) -> Self {
        PaymentId(id)
    }
}

/// The id of a refund.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct RefundId(pub i64);

impl RefundId {
    /// The refund as a [`Subject`]: `payments.refund:7`.
    pub fn subject(self) -> Subject {
        Subject::new(REFUND_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for RefundId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for RefundId {
    fn from(id: i64) -> Self {
        RefundId(id)
    }
}

/// The id of a deposit.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct DepositId(pub i64);

impl DepositId {
    /// The deposit as a [`Subject`]: `payments.deposit:3`.
    pub fn subject(self) -> Subject {
        Subject::new(DEPOSIT_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for DepositId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for DepositId {
    fn from(id: i64) -> Self {
        DepositId(id)
    }
}

/// What a refund is paid back from (wl-payments §3). All sources are
/// listed, so later features add no variant. A refund stores it as a
/// [`Subject`] (type and id); [`kind`](RefundSource::kind) tells the
/// sources apart when their subject types belong to other modules.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "subject", rename_all = "snake_case")]
pub enum RefundSource {
    /// The unallocated amount of an incoming payment.
    Payment(PaymentId),
    /// A credit note (`invoicing.credit_note`).
    CreditNote(Subject),
    /// A deposit held for the party.
    Deposit(DepositId),
    /// The party's credit balance.
    PartyCredit(PartyId),
    /// A payer's credit balance, owned by another module.
    PayerCredit(Subject),
    /// A cancelled or reduced order, owned by another module.
    Order(Subject),
    /// A cancelled booking, owned by another module.
    Booking(Subject),
}

impl RefundSource {
    /// The source as a [`Subject`], e.g. `payments.payment:42`.
    pub fn subject(&self) -> Subject {
        match self {
            RefundSource::Payment(id) => id.subject(),
            RefundSource::Deposit(id) => id.subject(),
            RefundSource::PartyCredit(id) => id.subject(),
            RefundSource::CreditNote(s)
            | RefundSource::PayerCredit(s)
            | RefundSource::Order(s)
            | RefundSource::Booking(s) => s.clone(),
        }
    }

    /// The stored name of the kind: `payment`, `credit_note`, `deposit`,
    /// `party_credit`, `payer_credit`, `order` or `booking`.
    pub fn kind(&self) -> &'static str {
        match self {
            RefundSource::Payment(_) => "payment",
            RefundSource::CreditNote(_) => "credit_note",
            RefundSource::Deposit(_) => "deposit",
            RefundSource::PartyCredit(_) => "party_credit",
            RefundSource::PayerCredit(_) => "payer_credit",
            RefundSource::Order(_) => "order",
            RefundSource::Booking(_) => "booking",
        }
    }

    /// The source a stored [`Subject`] names, for the subject types this
    /// module knows (payment, credit note, deposit, party); `None` for
    /// another type, whose owner gives the kind through
    /// [`RefundSource::from_kind`].
    pub fn from_subject(subject: &Subject) -> Option<Self> {
        match subject.subject_type.as_str() {
            PAYMENT_SUBJECT_TYPE => Some(RefundSource::Payment(PaymentId(subject.subject_id))),
            CREDIT_NOTE_SUBJECT_TYPE => Some(RefundSource::CreditNote(subject.clone())),
            DEPOSIT_SUBJECT_TYPE => Some(RefundSource::Deposit(DepositId(subject.subject_id))),
            PARTY_SUBJECT_TYPE => Some(RefundSource::PartyCredit(PartyId(subject.subject_id))),
            _ => None,
        }
    }

    /// The source of this [`kind`](RefundSource::kind) for `subject`;
    /// `None` for an unknown kind.
    pub fn from_kind(kind: &str, subject: &Subject) -> Option<Self> {
        match kind {
            "payment" => Some(RefundSource::Payment(PaymentId(subject.subject_id))),
            "credit_note" => Some(RefundSource::CreditNote(subject.clone())),
            "deposit" => Some(RefundSource::Deposit(DepositId(subject.subject_id))),
            "party_credit" => Some(RefundSource::PartyCredit(PartyId(subject.subject_id))),
            "payer_credit" => Some(RefundSource::PayerCredit(subject.clone())),
            "order" => Some(RefundSource::Order(subject.clone())),
            "booking" => Some(RefundSource::Booking(subject.clone())),
            _ => None,
        }
    }
}

/// Whether money comes in or goes out.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Direction {
    /// Money received from a party (a receipt).
    #[default]
    In,
    /// Money paid to a party (a payment voucher).
    Out,
}

/// How the money moved (wl-payments §3). All methods are listed, so later
/// features add no variant.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PaymentMethod {
    /// Cash.
    #[default]
    Cash,
    /// A card, at a terminal or by slip.
    Card,
    /// A bank transfer.
    Transfer,
    /// A cheque.
    Cheque,
    /// An online payment through a payment gateway.
    Gateway,
    /// A direct debit.
    DirectDebit,
    /// A wallet.
    Wallet,
    /// A set-off against what the party owes the other way.
    SetOff,
}

/// Where a payment came from.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PaymentSource {
    /// Entered by a user.
    #[default]
    Manual,
    /// Created by the cheque register.
    Cheque,
    /// Created by a payment gateway.
    Gateway,
    /// Matched from a bank statement line.
    BankStatement,
    /// Created by the platform (for example the payment of a refund).
    System,
}

/// The lifecycle state of a payment (wl-payments §4); the state of the
/// `wl-payments` machine.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PaymentStatus {
    /// The money is in; can be allocated.
    #[default]
    Received,
    /// An online payment started; waiting for the gateway.
    Pending,
    /// An online payment that did not go through.
    Failed,
    /// The card holder disputed it and the gateway took the funds back.
    ChargedBack,
    /// Undone (terminal).
    Reversed,
}

impl PaymentStatus {
    /// Whether the payment holds money that can be allocated: `received`.
    pub fn is_received(self) -> bool {
        self == PaymentStatus::Received
    }
}

/// Why a payment is reversed. A user gives `entered_in_error` or
/// `bank_return` to `reverse`; `chargeback` belongs to `charge_back` and
/// only appears in the event.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReversalReason {
    /// Recorded by mistake.
    #[default]
    EnteredInError,
    /// The bank returned the money (a bounced cheque, a rejected transfer).
    BankReturn,
    /// The card holder's dispute took the money back.
    Chargeback,
}

impl ReversalReason {
    /// Whether a user may give it to `reverse`: not `chargeback`.
    pub fn is_manual(self) -> bool {
        self != ReversalReason::Chargeback
    }
}

/// Why a refund failed. A user gives `bank_return` or `card_unavailable`;
/// `gateway_declined` only comes from the system.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RefundFailureKind {
    /// The payment gateway declined the refund.
    #[default]
    GatewayDeclined,
    /// The card cannot take the refund (closed, expired).
    CardUnavailable,
    /// The bank returned the transfer or cheque.
    BankReturn,
}

impl RefundFailureKind {
    /// Whether a user may give it to `fail`: not `gateway_declined`.
    pub fn is_manual(self) -> bool {
        self != RefundFailureKind::GatewayDeclined
    }
}

/// What a payment is allocated to. Credit notes, supplier bills and the
/// payer's credit balance come with their issues; each is stored as a
/// [`Subject`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AllocationTarget {
    /// An invoice.
    Invoice(InvoiceId),
    /// A deposit held for a source agreement; the money leaves the party's
    /// credit.
    Deposit(DepositId),
    /// A supplier bill; an outgoing payment pays it.
    SupplierBill(SupplierBillId),
    /// A supplier debit note; the supplier's payment back settles it.
    SupplierDebitNote(SupplierDebitNoteId),
}

impl AllocationTarget {
    /// The target as a [`Subject`], e.g. `invoicing.invoice:42`.
    pub fn subject(self) -> Subject {
        match self {
            AllocationTarget::Invoice(id) => id.subject(),
            AllocationTarget::Deposit(id) => id.subject(),
            AllocationTarget::SupplierBill(id) => id.subject(),
            AllocationTarget::SupplierDebitNote(id) => id.subject(),
        }
    }

    /// The target a [`Subject`] names; `None` for a type not known here.
    pub fn from_subject(subject: &Subject) -> Option<Self> {
        if subject.subject_type == INVOICE_SUBJECT_TYPE {
            Some(AllocationTarget::Invoice(InvoiceId(subject.subject_id)))
        } else if subject.subject_type == DEPOSIT_SUBJECT_TYPE {
            Some(AllocationTarget::Deposit(DepositId(subject.subject_id)))
        } else if subject.subject_type == SUPPLIER_BILL_SUBJECT_TYPE {
            Some(AllocationTarget::SupplierBill(SupplierBillId(
                subject.subject_id,
            )))
        } else if subject.subject_type == SUPPLIER_DEBIT_NOTE_SUBJECT_TYPE {
            Some(AllocationTarget::SupplierDebitNote(SupplierDebitNoteId(
                subject.subject_id,
            )))
        } else {
            None
        }
    }
}

/// A payment another module asks to record: [`PaymentRecorder::record`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordPayment {
    /// Money in (receipt) or out (voucher).
    pub direction: Direction,
    /// Who paid, or is paid.
    pub party: PartyId,
    /// The legal entity whose books it belongs to.
    pub legal_entity_id: i64,
    /// How the money moved.
    pub method: PaymentMethod,
    /// The amount in fils; must be above 0.
    pub amount: i64,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// The day the money moved; `None` for today (Asia/Dubai).
    pub date: Option<NaiveDate>,
    /// The payer's or bank's reference.
    pub reference: Option<String>,
    /// The method's details: cheque number, bank and date; card slip;
    /// transfer reference.
    pub method_details: renox::serde_json::Value,
    /// The record the money was paid in advance for (a deposit, an
    /// application fee…), if any.
    pub prepaid: Option<Subject>,
    /// Where it came from.
    pub source: PaymentSource,
    /// The bank statement line it was matched from, if any.
    pub statement_line_id: Option<i64>,
    /// The user who collected the money, if any.
    pub collected_by: Option<i64>,
}

impl RecordPayment {
    /// An incoming manual payment of `amount` fils in AED from `party` to
    /// `legal_entity_id`, by `method`.
    pub fn new(party: PartyId, legal_entity_id: i64, method: PaymentMethod, amount: i64) -> Self {
        RecordPayment {
            direction: Direction::In,
            party,
            legal_entity_id,
            method,
            amount,
            currency: DEFAULT_CURRENCY.to_owned(),
            date: None,
            reference: None,
            method_details: renox::serde_json::json!({}),
            prepaid: None,
            source: PaymentSource::Manual,
            statement_line_id: None,
            collected_by: None,
        }
    }

    /// Money going out.
    pub fn outgoing(mut self) -> Self {
        self.direction = Direction::Out;
        self
    }

    /// In this currency.
    pub fn currency(mut self, currency: &str) -> Self {
        self.currency = currency.to_owned();
        self
    }

    /// Paid on `date`.
    pub fn date(mut self, date: NaiveDate) -> Self {
        self.date = Some(date);
        self
    }

    /// With this reference.
    pub fn reference(mut self, reference: &str) -> Self {
        self.reference = Some(reference.to_owned());
        self
    }

    /// With these method details.
    pub fn method_details(mut self, details: renox::serde_json::Value) -> Self {
        self.method_details = details;
        self
    }

    /// Paid in advance for `prepaid`.
    pub fn prepaid(mut self, prepaid: Subject) -> Self {
        self.prepaid = Some(prepaid);
        self
    }

    /// From this source.
    pub fn source(mut self, source: PaymentSource) -> Self {
        self.source = source;
        self
    }

    /// Matched from this bank statement line.
    pub fn statement_line(mut self, id: i64) -> Self {
        self.statement_line_id = Some(id);
        self
    }

    /// Collected by this user.
    pub fn collected_by(mut self, user: i64) -> Self {
        self.collected_by = Some(user);
        self
    }
}

/// What other modules see of a payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaymentSummary {
    /// The id.
    pub id: PaymentId,
    /// The receipt or voucher number, e.g. `RCT-ACME-2026-0001`; `None`
    /// while `pending`.
    pub number: Option<String>,
    /// Who paid, or is paid.
    pub party: PartyId,
    /// The legal entity whose books it belongs to.
    pub legal_entity_id: i64,
    /// Money in or out.
    pub direction: Direction,
    /// How the money moved.
    pub method: PaymentMethod,
    /// The amount, in fils.
    pub amount: i64,
    /// The part allocated to targets, in fils.
    pub allocated: i64,
    /// The part still free (the party's credit), in fils.
    pub unallocated: i64,
    /// The ISO currency code.
    pub currency: String,
    /// The lifecycle state.
    pub status: PaymentStatus,
    /// The day the money moved.
    pub date: NaiveDate,
}

/// The future a [`PaymentRecorder`] method returns.
pub type PaymentFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Records, allocates, reverses and reads payments for other modules.
/// `wl-payments` provides it; without that module, [`payment_recorder`]
/// gives [`NoPayments`].
pub trait PaymentRecorder: Send + Sync + 'static {
    /// Records `payment` (status `received`) with its receipt or voucher
    /// number, by user `by` (`None` = the system), and returns it. Refused
    /// for an amount ≤ 0, a disabled method, incomplete method details or an
    /// unknown party.
    fn record<'a>(
        &'a self,
        state: &'a AppState,
        payment: RecordPayment,
        by: Option<i64>,
    ) -> PaymentFuture<'a, PaymentSummary>;

    /// Allocates `amount` fils of `payment` to `target`, through the
    /// target's own contract (for an invoice,
    /// [`Invoicer::allocate`](crate::invoicing::Invoicer::allocate)). Refused
    /// for an amount ≤ 0 or above the unallocated amount, a payment that is
    /// not `received`, or a target its module refuses.
    fn allocate<'a>(
        &'a self,
        state: &'a AppState,
        payment: PaymentId,
        target: AllocationTarget,
        amount: i64,
    ) -> PaymentFuture<'a, ()>;

    /// Reverses `payment` (`received` → `reversed`) with `reason` (never
    /// [`ReversalReason::Chargeback`]) and a `note`, by user `by` (`None` =
    /// the system). Every allocation is marked removed and kept, and
    /// [`PaymentReversed`] is emitted after the commit. Refused without a
    /// reason, twice, or for a gateway payment with an open dispute.
    fn reverse<'a>(
        &'a self,
        state: &'a AppState,
        payment: PaymentId,
        reason: ReversalReason,
        note: Option<String>,
        by: Option<i64>,
    ) -> PaymentFuture<'a, ()>;

    /// The payment with this id.
    fn find<'a>(&'a self, db: &'a Db, id: PaymentId) -> PaymentFuture<'a, Option<PaymentSummary>>;

    /// The party's credit in `currency`, in fils: the unallocated amounts of
    /// its `received` incoming payments, less the refunds already paid out
    /// of them or of the credit.
    fn credit_balance<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
        currency: &'a str,
    ) -> PaymentFuture<'a, i64>;

    /// The payments made in advance for `prepaid`, oldest first.
    fn payments_for<'a>(
        &'a self,
        db: &'a Db,
        prepaid: &'a Subject,
    ) -> PaymentFuture<'a, Vec<PaymentSummary>>;

    /// Asks a refund of a source the caller owns (a credit note, payer
    /// credit, order or booking), by user `by` (`None` = the system), and
    /// returns its id. The refund starts `requested` and still waits for an
    /// approval by another user (`FIN.ACL.04`); the caller has authorized
    /// its user and vouches for the source's balance. The default refuses
    /// (422): payments is not enabled.
    fn request_refund<'a>(
        &'a self,
        _state: &'a AppState,
        _request: RefundRequest,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, RefundId> {
        Box::pin(async { Err(payments_off("a refund cannot be requested")) })
    }
}

/// A refund another module asks for: [`PaymentRecorder::request_refund`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundRequest {
    /// What it is paid out of.
    pub source: RefundSource,
    /// Who is paid back.
    pub party: PartyId,
    /// The legal entity paying.
    pub legal_entity_id: i64,
    /// The amount, in fils; above 0.
    pub amount: i64,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// How it is to be paid, if already known.
    pub method: Option<PaymentMethod>,
    /// The gateway transaction a `gateway` refund goes back to.
    pub gateway_txn: Option<String>,
    /// Why.
    pub reason: String,
}

/// The recorder when the `payments` module is off: nothing is found and
/// there is no credit, so callers hide the feature, and a record,
/// allocation or reversal is refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPayments;

impl PaymentRecorder for NoPayments {
    fn record<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: RecordPayment,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, PaymentSummary> {
        Box::pin(async { Err(payments_off("a payment cannot be recorded")) })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        payment: PaymentId,
        _target: AllocationTarget,
        _amount: i64,
    ) -> PaymentFuture<'a, ()> {
        Box::pin(async move {
            Err(payments_off(&format!(
                "payment {payment} cannot be allocated"
            )))
        })
    }

    fn reverse<'a>(
        &'a self,
        _state: &'a AppState,
        payment: PaymentId,
        _reason: ReversalReason,
        _note: Option<String>,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, ()> {
        Box::pin(async move {
            Err(payments_off(&format!(
                "payment {payment} cannot be reversed"
            )))
        })
    }

    fn find<'a>(
        &'a self,
        _db: &'a Db,
        _id: PaymentId,
    ) -> PaymentFuture<'a, Option<PaymentSummary>> {
        Box::pin(async { Ok(None) })
    }

    fn credit_balance<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _currency: &'a str,
    ) -> PaymentFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }

    fn payments_for<'a>(
        &'a self,
        _db: &'a Db,
        _prepaid: &'a Subject,
    ) -> PaymentFuture<'a, Vec<PaymentSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The 422 of a write while payments is off.
fn payments_off(what: &str) -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        format!("{what}: payments is not enabled"),
    )
}

/// Makes `recorder` the app's [`PaymentRecorder`]; `wl-payments` calls it
/// in `register`.
pub fn provide_payment_recorder(app: &mut Registry, recorder: impl PaymentRecorder) {
    let recorder: Arc<dyn PaymentRecorder> = Arc::new(recorder);
    app.provide(recorder);
}

/// The app's [`PaymentRecorder`], or [`NoPayments`] when none is provided.
pub fn payment_recorder(state: &AppState) -> Arc<dyn PaymentRecorder> {
    match state.provided::<Arc<dyn PaymentRecorder>>() {
        Some(recorder) => Arc::clone(&recorder),
        None => Arc::new(NoPayments),
    }
}

/// One allocation a reversal removed. It carries its id, so the listener
/// runs one `unallocate` per allocation and ignores a repeated event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedAllocation {
    /// The allocation row.
    pub allocation: i64,
    /// What the money was allocated to.
    pub target: Subject,
    /// The amount, in fils.
    pub amount: i64,
}

/// An allocation of a payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentAllocation {
    /// What the money was allocated to.
    pub target: Subject,
    /// The amount, in fils.
    pub amount: i64,
}

/// A payment was received (`record`, `confirm`, `win_chargeback`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentReceived {
    /// The payment.
    pub payment: PaymentId,
    /// Who paid.
    pub party: PartyId,
    /// The amount, in fils.
    pub amount: i64,
    /// How the money moved.
    pub method: PaymentMethod,
    /// The allocations it already has.
    pub allocations: Vec<PaymentAllocation>,
}

impl Event for PaymentReceived {}

/// A payment was reversed (`reverse`, `charge_back`): its allocations are
/// removed and each target must be reopened by its own module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentReversed {
    /// The payment.
    pub payment: PaymentId,
    /// Who paid.
    pub party: PartyId,
    /// The legal entity whose books it belongs to.
    pub legal_entity_id: i64,
    /// Why it was reversed.
    pub reason: ReversalReason,
    /// Where the payment came from.
    pub source: PaymentSource,
    /// The bank statement line it was matched from; the import puts it back
    /// to `unmatched`.
    pub statement_line: Option<i64>,
    /// The allocations removed, each with its id.
    pub removed: Vec<RemovedAllocation>,
    /// The record it was paid in advance for; the vertical decides what to
    /// do with it.
    pub prepaid: Option<Subject>,
    /// The day it was reversed (Asia/Dubai).
    pub reversed_on: NaiveDate,
}

impl Event for PaymentReversed {}

/// A refund was paid out (`pay`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundPaid {
    /// The refund.
    pub refund: RefundId,
    /// Who is paid.
    pub party: PartyId,
    /// The amount, in fils.
    pub amount: i64,
    /// What the refund was paid out of.
    pub source: RefundSource,
}

impl Event for RefundPaid {}

/// The state a refund failed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundFailedFrom {
    /// Approved and not yet paid (a gateway decline).
    Approved,
    /// Paid out and then returned.
    Paid,
}

/// The attempt to pay a refund that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundAttemptRef {
    /// The attempt row.
    pub id: i64,
    /// Its number: 1, 2, ...
    pub number: i32,
}

/// A refund failed (`fail`): the money is back on its source. The owner of
/// a source outside `wl-payments` (credit note, payer credit, order,
/// booking) restores its side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundFailed {
    /// The refund.
    pub refund: RefundId,
    /// Who was to be paid.
    pub party: PartyId,
    /// The amount, in fils.
    pub amount: i64,
    /// The ISO currency code.
    pub currency: String,
    /// What the refund was to be paid out of.
    pub source: RefundSource,
    /// How it was to be paid.
    pub method: PaymentMethod,
    /// Why it failed.
    pub kind: RefundFailureKind,
    /// The reason given.
    pub reason: String,
    /// The attempt that failed.
    pub attempt: RefundAttemptRef,
    /// The state it failed from.
    pub from: RefundFailedFrom,
    /// The outgoing payment that was reversed, if the refund was paid.
    pub outgoing_payment: Option<PaymentId>,
}

impl Event for RefundFailed {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_subjects() {
        assert_eq!(PaymentId(42).subject().to_string(), "payments.payment:42");
        assert_eq!(RefundId(7).subject().to_string(), "payments.refund:7");
        assert_eq!(PaymentId(42).to_string(), "42");
        assert_eq!(PaymentId::from(3), PaymentId(3));
        assert_eq!(RefundId::from(4), RefundId(4));
    }

    #[test]
    fn receipt_and_voucher_series_are_gapless_per_entity_and_yearly() {
        for (series, key, prefix) in [
            (&RECEIPT_SERIES, "receipt", "RCT"),
            (&RECEIPT_SERIES_SINGLE, "receipt", "RCT"),
            (&PAYMENT_VOUCHER_SERIES, "payment_voucher", "PV"),
            (&PAYMENT_VOUCHER_SERIES_SINGLE, "payment_voucher", "PV"),
        ] {
            assert_eq!((series.key, series.prefix), (key, prefix));
            assert_eq!(series.reset, Reset::Yearly);
            assert!(series.gapless && series.per_entity);
        }
        assert_eq!(RECEIPT_SERIES.pattern, "{prefix}-{entity}-{yyyy}-{0000}");
        assert_eq!(RECEIPT_SERIES_SINGLE.pattern, "{prefix}-{yyyy}-{0000}");
        assert_eq!(receipt_series(1), &RECEIPT_SERIES_SINGLE);
        assert_eq!(receipt_series(2), &RECEIPT_SERIES);
        assert_eq!(receipt_series(0), &RECEIPT_SERIES);
        assert_eq!(payment_voucher_series(1), &PAYMENT_VOUCHER_SERIES_SINGLE);
        assert_eq!(payment_voucher_series(3), &PAYMENT_VOUCHER_SERIES);
    }

    #[test]
    fn enums_are_stored_in_snake_case() {
        let directions: Vec<_> = Direction::ALL.iter().map(|d| d.as_str()).collect();
        assert_eq!(directions, ["in", "out"]);
        let methods: Vec<_> = PaymentMethod::ALL.iter().map(|m| m.as_str()).collect();
        assert_eq!(
            methods,
            [
                "cash",
                "card",
                "transfer",
                "cheque",
                "gateway",
                "direct_debit",
                "wallet",
                "set_off"
            ]
        );
        let sources: Vec<_> = PaymentSource::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            sources,
            ["manual", "cheque", "gateway", "bank_statement", "system"]
        );
        let states: Vec<_> = PaymentStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            states,
            ["received", "pending", "failed", "charged_back", "reversed"]
        );
        let reasons: Vec<_> = ReversalReason::ALL.iter().map(|r| r.as_str()).collect();
        assert_eq!(reasons, ["entered_in_error", "bank_return", "chargeback"]);
        assert_eq!(
            "bank_return".parse::<ReversalReason>().unwrap(),
            ReversalReason::BankReturn
        );
        assert_eq!(PaymentStatus::default(), PaymentStatus::Received);
    }

    #[test]
    fn a_chargeback_is_not_a_manual_reversal_reason() {
        assert!(ReversalReason::EnteredInError.is_manual());
        assert!(ReversalReason::BankReturn.is_manual());
        assert!(!ReversalReason::Chargeback.is_manual());
        assert!(PaymentStatus::Received.is_received());
        assert!(!PaymentStatus::Reversed.is_received());
    }

    #[test]
    fn an_allocation_target_round_trips_through_its_subject() {
        let target = AllocationTarget::Invoice(InvoiceId(42));
        let subject = target.subject();
        assert_eq!(subject.to_string(), "invoicing.invoice:42");
        assert_eq!(AllocationTarget::from_subject(&subject), Some(target));
        assert_eq!(
            AllocationTarget::from_subject(&Subject::new("payments.refund", 1)),
            None
        );
    }

    #[test]
    fn a_supplier_bill_allocation_target_round_trips_through_its_subject() {
        let target = AllocationTarget::SupplierBill(SupplierBillId(9));
        let subject = target.subject();
        assert_eq!(subject.to_string(), "invoicing.supplier_bill:9");
        assert_eq!(AllocationTarget::from_subject(&subject), Some(target));
    }

    #[test]
    fn a_supplier_debit_note_allocation_target_round_trips_through_its_subject() {
        let target = AllocationTarget::SupplierDebitNote(SupplierDebitNoteId(7));
        let subject = target.subject();
        assert_eq!(subject.to_string(), "invoicing.supplier_debit_note:7");
        assert_eq!(AllocationTarget::from_subject(&subject), Some(target));
    }

    #[test]
    fn a_deposit_allocation_target_round_trips_through_its_subject() {
        let target = AllocationTarget::Deposit(DepositId(7));
        let subject = target.subject();
        assert_eq!(subject.to_string(), "payments.deposit:7");
        assert_eq!(AllocationTarget::from_subject(&subject), Some(target));
    }

    #[test]
    fn a_record_defaults_to_an_incoming_manual_aed_payment() {
        let p = RecordPayment::new(PartyId(3), 1, PaymentMethod::Transfer, 300_000);
        assert_eq!(p.direction, Direction::In);
        assert_eq!(p.currency, "AED");
        assert_eq!(p.source, PaymentSource::Manual);
        assert_eq!((p.date, p.reference.as_deref()), (None, None));
        assert_eq!(p.prepaid, None);
        let p = p
            .outgoing()
            .currency("USD")
            .reference("TRF-1")
            .source(PaymentSource::BankStatement)
            .statement_line(9)
            .collected_by(5)
            .prepaid(Subject::new("property.application", 2))
            .date(NaiveDate::from_ymd_opt(2026, 11, 4).unwrap());
        assert_eq!(p.direction, Direction::Out);
        assert_eq!(p.currency, "USD");
        assert_eq!(p.reference.as_deref(), Some("TRF-1"));
        assert_eq!((p.statement_line_id, p.collected_by), (Some(9), Some(5)));
        assert_eq!(p.date, NaiveDate::from_ymd_opt(2026, 11, 4));
    }

    #[test]
    fn the_fallback_refuses_writes_with_422() {
        match payments_off("payment 5 cannot be reversed") {
            Error::Status(code, message) => {
                assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
                assert!(message.contains("payment 5"), "{message}");
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn the_reversal_event_serializes_with_plain_ids() {
        let e = PaymentReversed {
            payment: PaymentId(1),
            party: PartyId(2),
            legal_entity_id: 3,
            reason: ReversalReason::BankReturn,
            source: PaymentSource::BankStatement,
            statement_line: Some(8),
            removed: vec![RemovedAllocation {
                allocation: 11,
                target: Subject::new("invoicing.invoice", 4),
                amount: 200_000,
            }],
            prepaid: None,
            reversed_on: NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(),
        };
        let json = renox::serde_json::to_value(&e).unwrap();
        assert_eq!(json["payment"], 1);
        assert_eq!(json["removed"][0]["allocation"], 11);
        assert_eq!(json["removed"][0]["amount"], 200_000);
        assert_eq!(json["reversed_on"], "2026-11-04");
        assert_eq!(json["statement_line"], 8);
    }

    #[test]
    fn the_received_and_refund_events_serialize_with_plain_ids() {
        let received = PaymentReceived {
            payment: PaymentId(1),
            party: PartyId(2),
            amount: 300_000,
            method: PaymentMethod::Transfer,
            allocations: vec![PaymentAllocation {
                target: Subject::new("invoicing.invoice", 4),
                amount: 300_000,
            }],
        };
        let json = renox::serde_json::to_value(&received).unwrap();
        assert_eq!(json["amount"], 300_000);
        assert_eq!(json["allocations"][0]["amount"], 300_000);
        let paid = RefundPaid {
            refund: RefundId(7),
            party: PartyId(2),
            amount: 50_000,
            source: RefundSource::Payment(PaymentId(1)),
        };
        let json = renox::serde_json::to_value(&paid).unwrap();
        assert_eq!(json["refund"], 7);
        assert_eq!(json["amount"], 50_000);
        assert_eq!(json["source"]["kind"], "payment");
        assert_eq!(json["source"]["subject"], 1);
    }

    #[test]
    fn deposit_ids_are_subjects() {
        assert_eq!(DepositId(3).subject().to_string(), "payments.deposit:3");
        assert_eq!(DepositId(3).to_string(), "3");
        assert_eq!(DepositId::from(5), DepositId(5));
    }

    #[test]
    fn every_refund_source_has_a_subject_and_round_trips() {
        let note = Subject::new(CREDIT_NOTE_SUBJECT_TYPE, 7);
        let sources = [
            (RefundSource::Payment(PaymentId(1)), "payments.payment:1"),
            (
                RefundSource::CreditNote(note.clone()),
                "invoicing.credit_note:7",
            ),
            (RefundSource::Deposit(DepositId(2)), "payments.deposit:2"),
            (RefundSource::PartyCredit(PartyId(3)), "parties.party:3"),
            (
                RefundSource::PayerCredit(Subject::new("invoicing.payer_credit", 4)),
                "invoicing.payer_credit:4",
            ),
            (
                RefundSource::Order(Subject::new("sales.order", 5)),
                "sales.order:5",
            ),
            (
                RefundSource::Booking(Subject::new("booking.booking", 6)),
                "booking.booking:6",
            ),
        ];
        for (source, subject) in sources {
            assert_eq!(source.subject().to_string(), subject);
            let json = renox::serde_json::to_string(&source).unwrap();
            let back: RefundSource = renox::serde_json::from_str(&json).unwrap();
            assert_eq!(back, source);
            assert_eq!(
                RefundSource::from_kind(source.kind(), &source.subject()),
                Some(source.clone())
            );
        }
        assert_eq!(
            RefundSource::from_subject(&note),
            Some(RefundSource::CreditNote(note.clone()))
        );
        assert_eq!(
            RefundSource::from_subject(&PaymentId(9).subject()),
            Some(RefundSource::Payment(PaymentId(9)))
        );
        assert_eq!(
            RefundSource::from_subject(&Subject::new("sales.order", 1)),
            None
        );
        assert_eq!(RefundSource::from_kind("nope", &note), None);
    }

    #[test]
    fn failure_kinds_are_stored_in_snake_case_and_the_system_owns_a_decline() {
        let kinds: Vec<_> = RefundFailureKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            kinds,
            ["gateway_declined", "card_unavailable", "bank_return"]
        );
        assert!(!RefundFailureKind::GatewayDeclined.is_manual());
        assert!(RefundFailureKind::CardUnavailable.is_manual());
        assert!(RefundFailureKind::BankReturn.is_manual());
    }

    #[test]
    fn the_refund_failed_event_serializes_with_plain_ids() {
        let e = RefundFailed {
            refund: RefundId(7),
            party: PartyId(2),
            amount: 50_000,
            currency: "AED".into(),
            source: RefundSource::CreditNote(Subject::new(CREDIT_NOTE_SUBJECT_TYPE, 7)),
            method: PaymentMethod::Gateway,
            kind: RefundFailureKind::GatewayDeclined,
            reason: "card expired".into(),
            attempt: RefundAttemptRef { id: 11, number: 1 },
            from: RefundFailedFrom::Approved,
            outgoing_payment: None,
        };
        let json = renox::serde_json::to_value(&e).unwrap();
        assert_eq!(json["refund"], 7);
        assert_eq!(json["amount"], 50_000);
        assert_eq!(json["source"]["kind"], "credit_note");
        assert_eq!(json["source"]["subject"]["subject_id"], 7);
        assert_eq!(json["attempt"]["number"], 1);
        assert_eq!(json["from"], "approved");
        assert_eq!(json["outgoing_payment"], renox::serde_json::Value::Null);
        let back: RefundFailed = renox::serde_json::from_value(json).unwrap();
        assert_eq!(back, e);
    }
}
