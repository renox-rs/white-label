//! The invoicing contract (ARCHITECTURE §5, wl-invoicing §6): what every
//! module may do with invoices without depending on the `wl-invoicing`
//! crate.
//!
//! `wl-invoicing` provides an [`Invoicer`] with [`provide_invoicer`] and
//! emits [`InvoiceIssued`], [`InvoiceOverdue`], [`InvoicePaid`],
//! [`InvoiceWrittenOff`] and [`BadDebtRecovered`]. Other modules draft, read and allocate invoices
//! through [`invoicer`], which falls back to [`NoInvoicer`] when the
//! `invoicing` module is off:
//!
//! ```ignore
//! use wl_kernel::invoicing::{self, DraftLine, InvoiceDraft, VatCategory};
//!
//! let draft = InvoiceDraft::new(tenant, lease.subject())
//!     .line(DraftLine::new("Rent, October", 100_000, VatCategory::Exempt));
//! match invoicing::invoicer(&state).draft(&state.db, draft).await? {
//!     Some(invoice) => { /* link it */ }
//!     None => { /* invoicing is off: hide the feature */ }
//! }
//! ```
//!
//! Money is `i64` in fils (1/100 of the currency unit) of the invoice's
//! currency; quantities are in thousandths.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::numbering::{Reset, Series};
use crate::parties::PartyId;
use crate::subject::Subject;
use crate::tickets::TicketId;

/// The subject type of an invoice, for [`Subject`] references.
pub const INVOICE_SUBJECT_TYPE: &str = "invoicing.invoice";

/// The subject type of a credit note, for [`Subject`] references.
pub const CREDIT_NOTE_SUBJECT_TYPE: &str = "invoicing.credit_note";

/// The currency of a draft that names none.
pub const DEFAULT_CURRENCY: &str = "AED";

/// Tax invoice numbers when the client has more than one active legal
/// entity: `INV-ACME-2026-0001`; gapless, per entity, reset yearly (take
/// them with [`Series::next`] inside the `issue` transaction).
pub const INVOICE_SERIES: Series = Series {
    key: "invoice",
    pattern: "{prefix}-{entity}-{yyyy}-{0000}",
    prefix: "INV",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: true,
};

/// Tax invoice numbers when the client has exactly one active legal
/// entity: `INV-2026-0001`. Same key and count as [`INVOICE_SERIES`].
pub const INVOICE_SERIES_SINGLE: Series = Series {
    key: "invoice",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "INV",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: true,
};

/// The invoice series for a client with `active_entities` active legal
/// entities: [`INVOICE_SERIES_SINGLE`] for exactly one, else
/// [`INVOICE_SERIES`].
pub fn invoice_series(active_entities: usize) -> &'static Series {
    if active_entities == 1 {
        &INVOICE_SERIES_SINGLE
    } else {
        &INVOICE_SERIES
    }
}

/// Credit note numbers when the client has more than one active legal
/// entity: `CN-ACME-2026-0001`; gapless, per entity, reset yearly (take them
/// with [`Series::next`] inside the `issue` transaction).
pub const CREDIT_NOTE_SERIES: Series = Series {
    key: "credit_note",
    pattern: "{prefix}-{entity}-{yyyy}-{0000}",
    prefix: "CN",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: true,
};

/// Credit note numbers when the client has exactly one active legal entity:
/// `CN-2026-0001`. Same key and count as [`CREDIT_NOTE_SERIES`].
pub const CREDIT_NOTE_SERIES_SINGLE: Series = Series {
    key: "credit_note",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "CN",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: true,
};

/// The credit note series for a client with `active_entities` active legal
/// entities: [`CREDIT_NOTE_SERIES_SINGLE`] for exactly one, else
/// [`CREDIT_NOTE_SERIES`].
pub fn credit_note_series(active_entities: usize) -> &'static Series {
    if active_entities == 1 {
        &CREDIT_NOTE_SERIES_SINGLE
    } else {
        &CREDIT_NOTE_SERIES
    }
}

/// The id of an invoice.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct InvoiceId(pub i64);

impl InvoiceId {
    /// The invoice as a [`Subject`]: `invoicing.invoice:42`.
    pub fn subject(self) -> Subject {
        Subject::new(INVOICE_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for InvoiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for InvoiceId {
    fn from(id: i64) -> Self {
        InvoiceId(id)
    }
}

/// The id of a credit note.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct CreditNoteId(pub i64);

impl CreditNoteId {
    /// The credit note as a [`Subject`]: `invoicing.credit_note:7`.
    pub fn subject(self) -> Subject {
        Subject::new(CREDIT_NOTE_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for CreditNoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for CreditNoteId {
    fn from(id: i64) -> Self {
        CreditNoteId(id)
    }
}

/// The lifecycle state of a credit note (wl-invoicing §4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CreditNoteStatus {
    /// Being prepared: no number, no effect on any balance.
    #[default]
    Draft,
    /// Issued with a number; its remaining balance can be used.
    Issued,
    /// Fully used; `reopen` returns it to `issued` when a use is undone.
    Applied,
}

/// The net to credit on one invoice line, in fils of the invoice's currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditLine {
    /// The position of the invoice line.
    pub invoice_line: i64,
    /// The net amount to credit on that line, in fils.
    pub net: i64,
}

impl CreditLine {
    /// Credits `net` fils on the invoice line at `invoice_line`.
    pub fn new(invoice_line: i64, net: i64) -> Self {
        Self { invoice_line, net }
    }
}

/// What other modules see of a credit note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CreditNoteSummary {
    /// The id.
    pub id: CreditNoteId,
    /// The number, e.g. `CN-ACME-2026-0001`; `None` while a draft.
    pub number: Option<String>,
    /// The lifecycle state.
    pub status: CreditNoteStatus,
    /// The invoice it credits.
    pub invoice: InvoiceId,
    /// Who is credited.
    pub party: PartyId,
    /// The issuing legal entity (always the invoice's).
    pub legal_entity_id: Option<i64>,
    /// The ISO currency code.
    pub currency: String,
    /// The total with VAT, in fils.
    pub gross_total: i64,
    /// What is still unused, in fils.
    pub remaining: i64,
}

/// The UAE VAT treatment of an invoice line. Only `standard` carries VAT.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum VatCategory {
    /// Standard rated, 5%.
    #[default]
    Standard,
    /// Zero rated (exports, international transport, …), 0%.
    ZeroRated,
    /// Exempt (residential rent, bare land, …).
    Exempt,
    /// Outside the scope of UAE VAT.
    OutOfScope,
    /// Reverse charge: the recipient accounts for the VAT.
    ReverseCharge,
}

impl VatCategory {
    /// The rate charged on the line, in basis points (500 = 5%).
    pub fn rate_bp(self) -> i64 {
        match self {
            VatCategory::Standard => 500,
            VatCategory::ZeroRated
            | VatCategory::Exempt
            | VatCategory::OutOfScope
            | VatCategory::ReverseCharge => 0,
        }
    }

    /// Whether the line carries VAT on this invoice.
    pub fn carries_vat(self) -> bool {
        self.rate_bp() > 0
    }
}

/// The lifecycle state of an invoice (wl-invoicing §4); the state of the
/// `wl-invoicing` machine.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InvoiceStatus {
    /// Editable, no number yet.
    #[default]
    Draft,
    /// Numbered and sent; not editable.
    Issued,
    /// Part of the balance settled.
    PartiallyPaid,
    /// Due date passed with a balance.
    Overdue,
    /// Balance is zero.
    Paid,
    /// Open balance written off as a bad debt.
    WrittenOff,
    /// Cancelled in full by a credit note (terminal).
    Credited,
}

impl InvoiceStatus {
    /// Whether money can still be allocated to it: `issued`,
    /// `partially_paid` or `overdue`.
    pub fn is_open(self) -> bool {
        matches!(
            self,
            InvoiceStatus::Issued | InvoiceStatus::PartiallyPaid | InvoiceStatus::Overdue
        )
    }

    /// Whether [`Invoicer::allocate`] takes money for it: open, or
    /// `written_off` (the money is then a recovery of the bad debt).
    pub fn accepts_allocation(self) -> bool {
        self.is_open() || self == InvoiceStatus::WrittenOff
    }
}

/// One line of an [`InvoiceDraft`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftLine {
    /// What is billed.
    pub description: String,
    /// The quantity in thousandths (1000 = 1).
    pub quantity_milli: i64,
    /// The price of one unit, in fils, before VAT.
    pub unit_price: i64,
    /// The discount on the line, in fils, before VAT.
    pub discount: i64,
    /// The VAT treatment.
    pub vat_category: VatCategory,
    /// The record the line bills (a charge, a usage record, a job card…).
    pub origin: Option<Subject>,
}

impl DraftLine {
    /// One unit of `description` at `unit_price` fils, no discount.
    pub fn new(description: &str, unit_price: i64, vat_category: VatCategory) -> Self {
        DraftLine {
            description: description.to_owned(),
            quantity_milli: 1000,
            unit_price,
            discount: 0,
            vat_category,
            origin: None,
        }
    }

    /// With this quantity, in thousandths.
    pub fn quantity_milli(mut self, quantity_milli: i64) -> Self {
        self.quantity_milli = quantity_milli;
        self
    }

    /// With this discount, in fils.
    pub fn discount(mut self, discount: i64) -> Self {
        self.discount = discount;
        self
    }

    /// Billing `origin`.
    pub fn origin(mut self, origin: Subject) -> Self {
        self.origin = Some(origin);
        self
    }
}

/// A draft invoice another module asks for: [`Invoicer::draft`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceDraft {
    /// Who is billed.
    pub party: PartyId,
    /// The record it bills for (a lease, a sales order, a ticket…).
    pub source: Subject,
    /// The scope unit (a building, a project…) whose legal entity issues
    /// it, if any.
    pub scope: Option<Subject>,
    /// The issuing legal entity, when chosen; otherwise the scope's, else
    /// the default one.
    pub legal_entity_id: Option<i64>,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// The due date; `None` for the issue date plus the payment terms.
    pub due_date: Option<NaiveDate>,
    /// The date of supply, when not the issue date.
    pub date_of_supply: Option<NaiveDate>,
    /// The lines, in order.
    pub lines: Vec<DraftLine>,
}

impl InvoiceDraft {
    /// A draft in AED for `party` about `source`, with no lines yet.
    pub fn new(party: PartyId, source: Subject) -> Self {
        InvoiceDraft {
            party,
            source,
            scope: None,
            legal_entity_id: None,
            currency: DEFAULT_CURRENCY.to_owned(),
            due_date: None,
            date_of_supply: None,
            lines: Vec::new(),
        }
    }

    /// Issued by the entity of `scope`.
    pub fn scope(mut self, scope: Subject) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Issued by the legal entity `id`.
    pub fn legal_entity(mut self, id: i64) -> Self {
        self.legal_entity_id = Some(id);
        self
    }

    /// In this currency.
    pub fn currency(mut self, currency: &str) -> Self {
        self.currency = currency.to_owned();
        self
    }

    /// Due on `date`.
    pub fn due_date(mut self, date: NaiveDate) -> Self {
        self.due_date = Some(date);
        self
    }

    /// Supplied on `date`.
    pub fn date_of_supply(mut self, date: NaiveDate) -> Self {
        self.date_of_supply = Some(date);
        self
    }

    /// With `line` added at the end.
    pub fn line(mut self, line: DraftLine) -> Self {
        self.lines.push(line);
        self
    }
}

/// What other modules see of an invoice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InvoiceSummary {
    /// The id.
    pub id: InvoiceId,
    /// The number, e.g. `INV-ACME-2026-0001`; `None` while a draft.
    pub number: Option<String>,
    /// The lifecycle state.
    pub status: InvoiceStatus,
    /// Who is billed.
    pub party: PartyId,
    /// The issuing legal entity, once chosen.
    pub legal_entity_id: Option<i64>,
    /// The ISO currency code.
    pub currency: String,
    /// The total with VAT, in fils.
    pub gross_total: i64,
    /// What is still open, in fils.
    pub balance: i64,
    /// The due date, once known.
    pub due_date: Option<NaiveDate>,
    /// Whether the invoice has an open dispute. A flag, not a state.
    pub disputed: bool,
    /// Chasing (reminders, escalation, write-off) is paused until this day
    /// after a dispute was upheld; `None` when there is no grace period.
    pub chasing_paused_until: Option<NaiveDate>,
}

impl InvoiceSummary {
    /// Whether chasing the invoice is paused on `today`: it is disputed, or
    /// `today` is before the end of the grace period after an upheld dispute.
    pub fn chasing_paused(&self, today: NaiveDate) -> bool {
        self.disputed || self.chasing_paused_until.is_some_and(|until| today < until)
    }
}

/// How a dispute was settled.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DisputeOutcome {
    /// The invoice stands; reminders resume after the grace period.
    #[default]
    Upheld,
    /// A full or partial credit note was issued for the agreed amount.
    Credited,
}

/// A disputed invoice line, as an event carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisputedLine {
    /// The line's position on the invoice (1-based).
    pub position: i32,
    /// The line's gross amount (net + VAT), in fils.
    pub amount: i64,
    /// The record the line bills for, when it has one.
    pub origin: Option<Subject>,
}

/// Who raises a dispute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DisputeBy {
    /// A staff user, by user id (needs `invoices.dispute`).
    User {
        /// The user's id.
        id: i64,
    },
    /// The billed party, from the portal (when the client allows it).
    Party {
        /// The party.
        id: PartyId,
    },
    /// The system (e.g. a delivery source); no permission check.
    System,
}

/// A request to flag an invoice as disputed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisputeRequest {
    /// The invoice.
    pub invoice: InvoiceId,
    /// Why it is disputed; must not be blank.
    pub reason: String,
    /// The disputed amount in fils; `None` means the gross sum of `lines`.
    pub amount: Option<i64>,
    /// Positions of the disputed lines; empty when not tied to lines.
    pub lines: Vec<i32>,
    /// Who raises it.
    pub by: DisputeBy,
    /// An existing ticket to use as the billing dispute ticket (no new one
    /// is opened); `None` opens one as usual.
    pub ticket: Option<TicketId>,
    /// The record that caused the dispute (e.g. a delivery); `None` when
    /// raised on the invoice itself.
    pub source: Option<Subject>,
}

impl DisputeRequest {
    /// A dispute of `invoice` for `reason` by `by`, with no amount and no
    /// lines.
    pub fn new(invoice: InvoiceId, reason: impl Into<String>, by: DisputeBy) -> Self {
        DisputeRequest {
            invoice,
            reason: reason.into(),
            amount: None,
            lines: Vec::new(),
            by,
            ticket: None,
            source: None,
        }
    }

    /// With the disputed `amount` in fils.
    pub fn amount(mut self, amount: i64) -> Self {
        self.amount = Some(amount);
        self
    }

    /// With the disputed line `positions`.
    pub fn lines(mut self, positions: impl IntoIterator<Item = i32>) -> Self {
        self.lines = positions.into_iter().collect();
        self
    }

    /// With an existing `ticket` as the dispute ticket (none is opened).
    pub fn ticket(mut self, ticket: TicketId) -> Self {
        self.ticket = Some(ticket);
        self
    }

    /// With the `source` record that caused the dispute.
    pub fn source(mut self, source: Subject) -> Self {
        self.source = Some(source);
        self
    }
}

/// How much of `available` fils to allocate to `invoice`: the smaller of
/// `available` and its balance when it accepts allocation
/// ([`InvoiceStatus::accepts_allocation`]), else 0.
pub fn allocation_proposal(invoice: &InvoiceSummary, available: i64) -> i64 {
    if invoice.status.accepts_allocation() {
        available.min(invoice.balance).max(0)
    } else {
        0
    }
}

/// The future an [`Invoicer`] method returns.
pub type InvoiceFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Why an invoice is reopened (`unallocate`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReopenReason {
    /// The payment was reversed (`PaymentReversed`).
    #[default]
    PaymentReversed,
    /// The allocation was moved to another target.
    AllocationMoved,
    /// A recovery of a written-off invoice was reversed.
    RecoveryReversed,
}

/// A request to undo an allocation to an invoice (`Invoicer::unallocate`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unallocate {
    /// The invoice to reopen.
    pub invoice: InvoiceId,
    /// The money's source: the payment or credit note allocated from.
    pub source: Subject,
    /// The amount to give back to the balance, in fils.
    pub amount: i64,
    /// The idempotency key, e.g. `payments.allocation:<id>`. A key that was
    /// already used gives `skipped` and changes nothing.
    pub key: String,
    /// Why it is reopened.
    pub reason: ReopenReason,
    /// The payment's reversal reason key (`bank_return`, `entered_in_error`,
    /// `chargeback`), as text.
    pub reversal_reason: Option<String>,
    /// The user who ran it; `None` for the system.
    pub ran_by: Option<i64>,
}

impl Unallocate {
    /// An unallocation of `amount` fils from `source` on `invoice`, under
    /// the idempotency `key`.
    pub fn new(
        invoice: InvoiceId,
        source: Subject,
        amount: i64,
        key: impl Into<String>,
        reason: ReopenReason,
    ) -> Self {
        Self {
            invoice,
            source,
            amount,
            key: key.into(),
            reason,
            reversal_reason: None,
            ran_by: None,
        }
    }

    /// Sets the payment's reversal reason key.
    pub fn reversal_reason(mut self, reason: impl Into<String>) -> Self {
        self.reversal_reason = Some(reason.into());
        self
    }

    /// Sets the user who ran it.
    pub fn ran_by(mut self, user: i64) -> Self {
        self.ran_by = Some(user);
        self
    }
}

/// The outcome of [`Invoicer::unallocate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reopened {
    /// The invoice's state after the call.
    pub new_status: InvoiceStatus,
    /// The invoice's balance after the call, in fils.
    pub balance: i64,
    /// Whether the key had been used before: nothing was changed.
    pub skipped: bool,
}

/// Drafts, reads and allocates invoices for other modules. `wl-invoicing`
/// provides it; without that module, [`invoicer`] gives [`NoInvoicer`].
pub trait Invoicer: Send + Sync + 'static {
    /// Creates a `draft` invoice from `draft` and returns its id; `None`
    /// when invoicing is off.
    fn draft<'a>(&'a self, db: &'a Db, draft: InvoiceDraft)
    -> InvoiceFuture<'a, Option<InvoiceId>>;

    /// The invoice with this id.
    fn find<'a>(&'a self, db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>>;

    /// Allocates `amount` fils from `source` (a payment or a credit note) to
    /// invoice `id`, through its `allocate` transition. Refused for an amount
    /// ≤ 0 or above the balance, or an invoice that does not accept
    /// allocation ([`InvoiceStatus::accepts_allocation`]). On an open invoice
    /// it runs `allocate`; on a `written_off` one it recovers the bad debt:
    /// the invoice stays `written_off` until the whole write-off is
    /// recovered, then becomes `paid`, and [`BadDebtRecovered`] is emitted.
    fn allocate<'a>(
        &'a self,
        state: &'a AppState,
        id: InvoiceId,
        source: Subject,
        amount: i64,
    ) -> InvoiceFuture<'a, ()>;

    /// Undoes `request.amount` fils of an allocation from `request.source`
    /// to invoice `request.invoice` and reopens it: the balance goes back up
    /// and the state is set from the balance and the due date. Idempotent
    /// per `request.key` (`skipped`). Refused for an amount <= 0, an amount
    /// above what is allocated from that source, or a state that cannot be
    /// reopened (`credited`; `written_off` too, unless the reason is
    /// `recovery_reversed`). Lines, totals and VAT are never written;
    /// [`InvoiceReopened`] is emitted. The default refuses (422): invoicing
    /// is not enabled.
    fn unallocate<'a>(
        &'a self,
        _state: &'a AppState,
        request: Unallocate,
    ) -> InvoiceFuture<'a, Reopened> {
        Box::pin(async move { Err(unallocating_off(request.invoice)) })
    }

    /// Removes every active allocation of invoice `id` (reopening it as
    /// [`Invoicer::unallocate`] does, with `reason`) and returns each source
    /// with the amount removed. The caller releases its own side of the
    /// money. The default refuses (422): invoicing is not enabled.
    fn unallocate_all<'a>(
        &'a self,
        _state: &'a AppState,
        id: InvoiceId,
        _reason: ReopenReason,
    ) -> InvoiceFuture<'a, Vec<(Subject, i64)>> {
        Box::pin(async move { Err(unallocating_off(id)) })
    }

    /// The `written_off` invoices of `party` in `currency` with something
    /// left to recover, oldest first. The default has none (invoicing is not
    /// enabled). The payment page lists them beside the allocate form.
    fn written_off_of<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _currency: &'a str,
    ) -> InvoiceFuture<'a, Vec<InvoiceSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Flags an issued, partly paid or overdue invoice as disputed and
    /// returns the dispute's id. The flag is not a state change. The default
    /// refuses (422): invoicing is not enabled.
    fn dispute<'a>(
        &'a self,
        _state: &'a AppState,
        request: DisputeRequest,
    ) -> InvoiceFuture<'a, i64> {
        Box::pin(async move {
            Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "invoice {} cannot be disputed: invoicing is not enabled",
                    request.invoice
                ),
            ))
        })
    }

    /// Credits an invoice in full: drafts and issues a credit note for every
    /// line, applies it and makes the invoice `credited`. Refused while
    /// anything is allocated to the invoice, or for a blank `reason`. `by` is
    /// the user, `None` for the system. The default refuses (422): invoicing
    /// is not enabled.
    fn credit_in_full<'a>(
        &'a self,
        _state: &'a AppState,
        invoice: InvoiceId,
        _reason: String,
        _by: Option<i64>,
    ) -> InvoiceFuture<'a, CreditNoteId> {
        Box::pin(async move { Err(crediting_off(invoice)) })
    }

    /// Drafts and issues, in one call, a credit note for `lines` of an
    /// invoice (each within the line's remaining credit). The default refuses
    /// (422): invoicing is not enabled.
    fn credit_partly<'a>(
        &'a self,
        _state: &'a AppState,
        invoice: InvoiceId,
        _lines: Vec<CreditLine>,
        _reason: String,
        _by: Option<i64>,
    ) -> InvoiceFuture<'a, CreditNoteId> {
        Box::pin(async move { Err(crediting_off(invoice)) })
    }

    /// The credit note with this id; the default knows none.
    fn find_credit_note<'a>(
        &'a self,
        _db: &'a Db,
        _id: CreditNoteId,
    ) -> InvoiceFuture<'a, Option<CreditNoteSummary>> {
        Box::pin(async { Ok(None) })
    }

    /// The credit notes of an invoice, oldest first; the default has none.
    fn credit_notes_for<'a>(
        &'a self,
        _db: &'a Db,
        _invoice: InvoiceId,
    ) -> InvoiceFuture<'a, Vec<CreditNoteSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// The sum of the remaining balances of the party's `issued` credit notes
    /// in `currency`, in fils: the credit-note part of the party's available
    /// credit. The default is 0.
    fn open_credit<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _currency: &'a str,
    ) -> InvoiceFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }

    /// Takes `amount` fils of a credit note's remaining balance for `refund`
    /// (a refund subject) on its numbered `attempt`. Idempotent per refund
    /// and attempt; refused when the remaining is too low. The default
    /// refuses (422): invoicing is not enabled.
    fn take_credit_for_refund<'a>(
        &'a self,
        _state: &'a AppState,
        credit_note: CreditNoteId,
        _refund: Subject,
        _attempt: i64,
        _amount: i64,
    ) -> InvoiceFuture<'a, ()> {
        Box::pin(async move {
            Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("credit note {credit_note} cannot be used: invoicing is not enabled"),
            ))
        })
    }
}

/// The 422 of crediting an invoice while invoicing is off.
fn crediting_off(id: InvoiceId) -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        format!("invoice {id} cannot be credited: invoicing is not enabled"),
    )
}

/// The invoicer when the `invoicing` module is off: nothing is drafted or
/// found, so callers hide the feature, and an allocation is refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoInvoicer;

impl Invoicer for NoInvoicer {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        _draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        Box::pin(async { Ok(None) })
    }

    fn find<'a>(
        &'a self,
        _db: &'a Db,
        _id: InvoiceId,
    ) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        Box::pin(async { Ok(None) })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        id: InvoiceId,
        _source: Subject,
        _amount: i64,
    ) -> InvoiceFuture<'a, ()> {
        Box::pin(async move { Err(invoicing_off(id)) })
    }
}

/// The 422 of unallocating an invoice while invoicing is off.
fn unallocating_off(id: InvoiceId) -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        format!("invoice {id} cannot be reopened: invoicing is not enabled"),
    )
}

/// The 422 of an allocation while invoicing is off.
fn invoicing_off(id: InvoiceId) -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        format!("invoice {id} cannot be allocated: invoicing is not enabled"),
    )
}

/// Makes `invoicer` the app's [`Invoicer`]; `wl-invoicing` calls it in
/// `register`.
pub fn provide_invoicer(app: &mut Registry, invoicer: impl Invoicer) {
    let invoicer: Arc<dyn Invoicer> = Arc::new(invoicer);
    app.provide(invoicer);
}

/// The app's [`Invoicer`], or [`NoInvoicer`] when none is provided.
pub fn invoicer(state: &AppState) -> Arc<dyn Invoicer> {
    match state.provided::<Arc<dyn Invoicer>>() {
        Some(invoicer) => Arc::clone(&invoicer),
        None => Arc::new(NoInvoicer),
    }
}

/// An invoice was issued (`issue`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceIssued {
    /// The invoice.
    pub invoice: InvoiceId,
    /// Who is billed.
    pub party: PartyId,
    /// The record it bills for.
    pub source: Subject,
    /// The total with VAT, in fils of the invoice's currency.
    pub total: i64,
    /// The due date.
    pub due_date: NaiveDate,
}

impl Event for InvoiceIssued {}

/// An invoice became overdue (`mark_overdue`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceOverdue {
    /// The invoice.
    pub invoice: InvoiceId,
    /// Who is billed.
    pub party: PartyId,
    /// What is still open, in fils.
    pub balance: i64,
    /// Days since the due date.
    pub days_overdue: i64,
}

impl Event for InvoiceOverdue {}

/// An invoice's balance reached zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoicePaid {
    /// The invoice.
    pub invoice: InvoiceId,
    /// Who is billed.
    pub party: PartyId,
    /// The record it bills for.
    pub source: Subject,
}

impl Event for InvoicePaid {}

/// An invoice's open balance was written off as a bad debt (`write_off`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceWrittenOff {
    /// The invoice.
    pub invoice: InvoiceId,
    /// Who is billed.
    pub party: PartyId,
    /// The record it bills for.
    pub source: Subject,
    /// The amount written off, in fils.
    pub amount: i64,
    /// The reason code, e.g. `insolvent`.
    pub reason_code: String,
    /// The user who approved it; `None` when no approval was needed.
    pub approver: Option<i64>,
}

impl Event for InvoiceWrittenOff {}

/// An invoice was flagged as disputed (`wl-invoicing` `dispute`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceDisputed {
    /// The invoice.
    pub invoice: InvoiceId,
    /// Who is billed.
    pub party: PartyId,
    /// The record it bills for.
    pub source: Subject,
    /// The dispute's id.
    pub dispute: i64,
    /// The disputed amount, in fils.
    pub amount: i64,
    /// The disputed lines; empty when not tied to lines.
    pub lines: Vec<DisputedLine>,
    /// The billing dispute ticket; `None` when ticketing is off.
    pub ticket: Option<TicketId>,
    /// Who raised it.
    pub raised_by: DisputeBy,
}

impl Event for InvoiceDisputed {}

/// A dispute was resolved (`wl-invoicing` `resolve_dispute`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceDisputeResolved {
    /// The invoice.
    pub invoice: InvoiceId,
    /// Who is billed.
    pub party: PartyId,
    /// The dispute's id.
    pub dispute: i64,
    /// How it was settled.
    pub outcome: DisputeOutcome,
    /// The amount credited, in fils; `None` for `upheld`.
    pub credited_amount: Option<i64>,
    /// The credit note, for `credited`.
    pub credit_note: Option<Subject>,
}

impl Event for InvoiceDisputeResolved {}

/// A credit note was issued (`issue`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditNoteIssued {
    /// The credit note.
    pub credit_note: CreditNoteId,
    /// The invoice it credits.
    pub invoice: InvoiceId,
    /// Who is credited.
    pub party: PartyId,
    /// The total with VAT, in fils of the invoice's currency.
    pub amount: i64,
    /// Why it was issued.
    pub reason: String,
}

impl Event for CreditNoteIssued {}

/// An invoice was credited in full (`credit`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceCredited {
    /// The invoice.
    pub invoice: InvoiceId,
    /// The credit note that cancelled it.
    pub credit_note: CreditNoteId,
    /// The amount credited, in fils.
    pub amount: i64,
    /// Why it was credited.
    pub reason: String,
    /// The origins of the invoice's lines when the caller chose to release
    /// them for re-billing; empty otherwise.
    pub released_origins: Vec<Subject>,
}

impl Event for InvoiceCredited {}

/// An `applied` credit note became `issued` again (`reopen`) because a use of
/// it was undone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditNoteReopened {
    /// The credit note.
    pub credit_note: CreditNoteId,
    /// Who is credited.
    pub party: PartyId,
    /// The amount given back to the remaining balance, in fils.
    pub amount_restored: i64,
    /// What undid the use: the failed refund (`payments.refund:<id>`) or the
    /// removed allocation.
    pub cause: Subject,
}

impl Event for CreditNoteReopened {}

/// An invoice was reopened because an allocation was undone (`unallocate`).
/// Lines, totals and VAT are untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceReopened {
    /// The invoice.
    pub invoice: InvoiceId,
    /// Who is billed.
    pub party: PartyId,
    /// The record it bills for.
    pub source: Subject,
    /// The record the money came from (the payment).
    pub money_source: Subject,
    /// The amount given back to the balance, in fils.
    pub amount: i64,
    /// The balance after, in fils.
    pub balance: i64,
    /// The state after.
    pub new_status: InvoiceStatus,
    /// Why it was reopened.
    pub reason: ReopenReason,
    /// The payment's reversal reason key, when there is one.
    pub reversal_reason: Option<String>,
}

impl Event for InvoiceReopened {}

/// Money was recovered on a written-off invoice (`recover`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BadDebtRecovered {
    /// The invoice.
    pub invoice: InvoiceId,
    /// Who is billed.
    pub party: PartyId,
    /// The record the money came from (the payment).
    pub source: Subject,
    /// The amount recovered, in fils.
    pub amount: i64,
    /// Whether the VAT bad debt relief had been claimed (`included`).
    pub relief_claimed: bool,
    /// The day it was recovered (Asia/Dubai).
    pub recovered_on: NaiveDate,
}

impl Event for BadDebtRecovered {}

/// The subject type of a supplier bill, for [`Subject`] references.
pub const SUPPLIER_BILL_SUBJECT_TYPE: &str = "invoicing.supplier_bill";

/// The id of a supplier bill (wl-invoicing §3).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct SupplierBillId(pub i64);

impl SupplierBillId {
    /// The bill as a [`Subject`]: `invoicing.supplier_bill:5`.
    pub fn subject(self) -> Subject {
        Subject::new(SUPPLIER_BILL_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for SupplierBillId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for SupplierBillId {
    fn from(id: i64) -> Self {
        SupplierBillId(id)
    }
}

/// The id of one line of a supplier bill.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct SupplierBillLineId(pub i64);

impl fmt::Display for SupplierBillLineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for SupplierBillLineId {
    fn from(id: i64) -> Self {
        SupplierBillLineId(id)
    }
}

/// The lifecycle state of a supplier bill (wl-invoicing §4); the same seven
/// states, with the same stored names, as the `wl-invoicing` machine.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SupplierBillStatus {
    /// Editable, not yet submitted.
    #[default]
    Draft,
    /// Submitted and not held.
    Matched,
    /// Held, e.g. as a possible duplicate.
    Exception,
    /// Approved for payment.
    Approved,
    /// Part of the bill is paid.
    PartiallyPaid,
    /// Paid in full.
    Paid,
    /// Voided (terminal).
    Void,
}

impl SupplierBillStatus {
    /// Whether the bill has been approved and not voided: `approved`,
    /// `partially_paid` or `paid`. Only then do its amounts count elsewhere
    /// (e.g. as a campaign cost).
    pub fn is_approved(self) -> bool {
        matches!(
            self,
            SupplierBillStatus::Approved
                | SupplierBillStatus::PartiallyPaid
                | SupplierBillStatus::Paid
        )
    }
}

/// One line of a supplier bill, as other modules see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierBillLine {
    /// The bill.
    pub bill: SupplierBillId,
    /// The line.
    pub line: SupplierBillLineId,
    /// The supplier's invoice number as entered.
    pub bill_number: String,
    /// The supplier.
    pub supplier: PartyId,
    /// The receiving legal entity, if set.
    pub legal_entity_id: Option<i64>,
    /// The line's description.
    pub description: String,
    /// The line amount in fils of AED, excluding recoverable VAT and already
    /// converted at the bill's exchange rate.
    pub amount_aed: i64,
    /// The bill's state now.
    pub bill_status: SupplierBillStatus,
}

/// Why a supplier bill is reopened (`unallocate`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BillReopenReason {
    /// The outgoing payment was reversed (`PaymentReversed`).
    #[default]
    PaymentReversed,
    /// A debit note that reduced the bill was cancelled.
    DebitNoteCancelled,
}

/// A request to apply money to a supplier bill (`SupplierBills::pay`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayBill {
    /// The bill to pay.
    pub bill: SupplierBillId,
    /// The money's source: the outgoing payment or debit note.
    pub source: Subject,
    /// The amount to apply, in fils of the bill's currency.
    pub amount: i64,
    /// The idempotency key, e.g. `payments.allocation:<id>`. A key that was
    /// already used gives `skipped` and changes nothing.
    pub key: String,
}

impl PayBill {
    /// A payment of `amount` fils from `source` on `bill`, under the
    /// idempotency `key`.
    pub fn new(bill: SupplierBillId, source: Subject, amount: i64, key: impl Into<String>) -> Self {
        Self {
            bill,
            source,
            amount,
            key: key.into(),
        }
    }
}

/// A request to undo money applied to a supplier bill
/// (`SupplierBills::unallocate`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnallocateBill {
    /// The bill to reopen.
    pub bill: SupplierBillId,
    /// The money's source: the payment or debit note applied from.
    pub source: Subject,
    /// The amount to give back to the balance, in fils.
    pub amount: i64,
    /// The idempotency key, e.g. `payments.allocation:<id>`. A key that was
    /// already used gives `skipped` and changes nothing.
    pub key: String,
    /// Why it is reopened.
    pub reason: BillReopenReason,
    /// The payment's reversal reason key (`bank_return`, `entered_in_error`,
    /// `chargeback`), as text.
    pub reversal_reason: Option<String>,
}

impl UnallocateBill {
    /// An unallocation of `amount` fils from `source` on `bill`, under the
    /// idempotency `key`.
    pub fn new(
        bill: SupplierBillId,
        source: Subject,
        amount: i64,
        key: impl Into<String>,
        reason: BillReopenReason,
    ) -> Self {
        Self {
            bill,
            source,
            amount,
            key: key.into(),
            reason,
            reversal_reason: None,
        }
    }

    /// Sets the payment's reversal reason key.
    pub fn reversal_reason(mut self, reason: impl Into<String>) -> Self {
        self.reversal_reason = Some(reason.into());
        self
    }
}

/// The outcome of [`SupplierBills::pay`] and [`SupplierBills::unallocate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BillBalance {
    /// The bill's state after the call.
    pub new_status: SupplierBillStatus,
    /// The bill's open balance after the call, in fils.
    pub balance: i64,
    /// Whether the key had been used before: nothing was changed.
    pub skipped: bool,
}

/// The 422 of a payment or reopening while invoicing is off.
fn bill_money_off(id: SupplierBillId) -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        format!("supplier bill {id} cannot take money: invoicing is not enabled"),
    )
}

/// The future a [`SupplierBills`] method returns.
pub type SupplierBillFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Reads supplier bill lines. `wl-invoicing` provides it with
/// [`provide_supplier_bills`]; without that module, [`supplier_bills`] gives
/// [`NoSupplierBills`].
pub trait SupplierBills: Send + Sync + 'static {
    /// The line `id`, or `None` when there is no such line.
    fn line<'a>(
        &'a self,
        db: &'a Db,
        id: SupplierBillLineId,
    ) -> SupplierBillFuture<'a, Option<SupplierBillLine>>;

    /// Applies money to an `approved` or `partially_paid` bill (system
    /// `pay`/`pay_in_full`). 422 for an amount of 0 or less or above the
    /// open balance, or a state that takes no payment. The default refuses
    /// (422): invoicing is not enabled.
    fn pay<'a>(
        &'a self,
        _state: &'a AppState,
        request: PayBill,
    ) -> SupplierBillFuture<'a, BillBalance> {
        Box::pin(async move { Err(bill_money_off(request.bill)) })
    }

    /// Gives money back to a `partially_paid` or `paid` bill (system
    /// `unallocate`), returning it to `approved` or `partially_paid`.
    /// [`SupplierBillReopened`] is emitted; approval, cost, VAT, lines and
    /// totals are never written. 422 for an amount above what `source`
    /// applied, or a state that cannot be reopened. The default refuses
    /// (422): invoicing is not enabled.
    fn unallocate<'a>(
        &'a self,
        _state: &'a AppState,
        request: UnallocateBill,
    ) -> SupplierBillFuture<'a, BillBalance> {
        Box::pin(async move { Err(bill_money_off(request.bill)) })
    }
}

/// The supplier bills when `invoicing` is off: no line exists, so a module
/// hides its "link a bill line" option.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSupplierBills;

impl SupplierBills for NoSupplierBills {
    fn line<'a>(
        &'a self,
        _db: &'a Db,
        _id: SupplierBillLineId,
    ) -> SupplierBillFuture<'a, Option<SupplierBillLine>> {
        Box::pin(async { Ok(None) })
    }
}

/// Makes `bills` the app's [`SupplierBills`]; `wl-invoicing` calls it in
/// `register`.
pub fn provide_supplier_bills(app: &mut Registry, bills: impl SupplierBills) {
    let bills: Arc<dyn SupplierBills> = Arc::new(bills);
    app.provide(bills);
}

/// The app's [`SupplierBills`], or [`NoSupplierBills`] when none is provided.
pub fn supplier_bills(state: &AppState) -> Arc<dyn SupplierBills> {
    match state.provided::<Arc<dyn SupplierBills>>() {
        Some(bills) => Arc::clone(&bills),
        None => Arc::new(NoSupplierBills),
    }
}

/// A supplier bill was approved (`approve`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierBillApproved {
    /// The bill.
    pub bill: SupplierBillId,
    /// The supplier.
    pub supplier: PartyId,
    /// The bill total in fils of the bill's currency.
    pub amount: i64,
}

impl Event for SupplierBillApproved {}

/// A supplier bill was voided (`void`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierBillVoided {
    /// The bill.
    pub bill: SupplierBillId,
    /// Why it was voided.
    pub reason: String,
}

impl Event for SupplierBillVoided {}

/// A supplier bill was reopened because money applied to it was undone
/// (`unallocate`). Approval, cost, VAT, lines and totals are untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierBillReopened {
    /// The bill.
    pub bill: SupplierBillId,
    /// The supplier.
    pub supplier: PartyId,
    /// The record the money came from (the payment or debit note).
    pub source: Subject,
    /// The amount given back to the balance, in fils.
    pub amount: i64,
    /// The open balance after, in fils.
    pub balance: i64,
    /// The state after: `approved` or `partially_paid`.
    pub new_status: SupplierBillStatus,
    /// Why it was reopened.
    pub reason: BillReopenReason,
}

impl Event for SupplierBillReopened {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_supplier_bill_id_is_a_subject_and_a_plain_number() {
        let id = SupplierBillId::from(5);
        assert_eq!(id.to_string(), "5");
        assert_eq!(id.subject(), Subject::new("invoicing.supplier_bill", 5));
        assert_eq!(renox::serde_json::to_string(&id).unwrap(), "5");
        let line: SupplierBillLineId = renox::serde_json::from_str("9").unwrap();
        assert_eq!(line, SupplierBillLineId(9));
        assert_eq!(line.to_string(), "9");
    }

    #[test]
    fn a_supplier_bill_counts_once_approved_and_not_voided() {
        use SupplierBillStatus::*;
        for s in [Approved, PartiallyPaid, Paid] {
            assert!(s.is_approved(), "{s:?}");
        }
        for s in [Draft, Matched, Exception, Void] {
            assert!(!s.is_approved(), "{s:?}");
        }
        assert_eq!(SupplierBillStatus::default(), Draft);
    }

    #[renox::test]
    async fn without_invoicing_no_bill_line_exists() {
        let app = renox::testing::TestApp::new(App::new()).await;
        let line = supplier_bills(app.state())
            .line(&app.state().db, SupplierBillLineId(1))
            .await
            .expect("no failure");
        assert_eq!(line, None);
    }

    #[test]
    fn the_bill_events_round_trip_through_json() {
        let approved = SupplierBillApproved {
            bill: SupplierBillId(3),
            supplier: PartyId(4),
            amount: 300_000,
        };
        let json = renox::serde_json::to_string(&approved).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<SupplierBillApproved>(&json).unwrap(),
            approved
        );
        let voided = SupplierBillVoided {
            bill: SupplierBillId(3),
            reason: "duplicate".into(),
        };
        let json = renox::serde_json::to_string(&voided).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<SupplierBillVoided>(&json).unwrap(),
            voided
        );
    }

    #[test]
    fn the_reopening_event_and_requests_round_trip_through_json() {
        let event = SupplierBillReopened {
            bill: SupplierBillId(3),
            supplier: PartyId(4),
            source: Subject::new("payments.payment", 8),
            amount: 450_000,
            balance: 450_000,
            new_status: SupplierBillStatus::PartiallyPaid,
            reason: BillReopenReason::PaymentReversed,
        };
        let json = renox::serde_json::to_string(&event).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<SupplierBillReopened>(&json).unwrap(),
            event
        );
        let request = UnallocateBill::new(
            SupplierBillId(3),
            Subject::new("payments.payment", 8),
            450_000,
            "payments.allocation:1",
            BillReopenReason::DebitNoteCancelled,
        )
        .reversal_reason("bank_return");
        assert_eq!(request.reversal_reason.as_deref(), Some("bank_return"));
        let json = renox::serde_json::to_string(&request).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<UnallocateBill>(&json).unwrap(),
            request
        );
        assert_eq!(
            BillReopenReason::default(),
            BillReopenReason::PaymentReversed
        );
    }

    #[renox::test]
    async fn without_invoicing_a_bill_takes_no_money() {
        let app = renox::testing::TestApp::new(App::new()).await;
        let state = app.state();
        let bills = supplier_bills(state);
        let pay = PayBill::new(
            SupplierBillId(1),
            Subject::new("payments.payment", 1),
            100,
            "payments.allocation:1",
        );
        assert!(matches!(
            bills.pay(state, pay).await,
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
        let undo = UnallocateBill::new(
            SupplierBillId(1),
            Subject::new("payments.payment", 1),
            100,
            "payments.allocation:1",
            BillReopenReason::PaymentReversed,
        );
        assert!(matches!(
            NoSupplierBills.unallocate(state, undo).await,
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
    }

    fn summary(status: InvoiceStatus, balance: i64) -> InvoiceSummary {
        InvoiceSummary {
            id: InvoiceId(1),
            number: None,
            status,
            party: PartyId(2),
            legal_entity_id: None,
            currency: "AED".into(),
            gross_total: 105_000,
            balance,
            due_date: None,
            disputed: false,
            chasing_paused_until: None,
        }
    }

    #[test]
    fn written_off_accepts_allocation_like_open() {
        use InvoiceStatus::*;
        for s in [Issued, PartiallyPaid, Overdue, WrittenOff] {
            assert!(s.accepts_allocation(), "{s:?}");
        }
        for s in [Draft, Paid, Credited] {
            assert!(!s.accepts_allocation(), "{s:?}");
        }
        assert!(!WrittenOff.is_open());
    }

    #[test]
    fn chasing_is_paused_while_disputed_and_through_the_grace_period() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        let mut s = summary(InvoiceStatus::Overdue, 1_000);
        assert!(!s.chasing_paused(day(20)));
        s.disputed = true;
        assert!(s.chasing_paused(day(20)));
        s.disputed = false;
        s.chasing_paused_until = Some(day(27));
        assert!(s.chasing_paused(day(26)));
        assert!(!s.chasing_paused(day(27)));
    }

    #[test]
    fn a_dispute_request_is_built_with_its_builder() {
        let req = DisputeRequest::new(InvoiceId(3), "wrong hours", DisputeBy::User { id: 9 })
            .amount(120_000)
            .lines([2, 3]);
        assert_eq!(req.amount, Some(120_000));
        assert_eq!(req.lines, vec![2, 3]);
        let bare = DisputeRequest::new(InvoiceId(3), "x", DisputeBy::System);
        assert_eq!((bare.amount, bare.lines.len()), (None, 0));
        assert_eq!((bare.ticket, bare.source.clone()), (None, None));
        let linked = bare
            .ticket(TicketId(4))
            .source(Subject::new("delivery.delivery", 8));
        assert_eq!(linked.ticket, Some(TicketId(4)));
        assert_eq!(linked.source, Some(Subject::new("delivery.delivery", 8)));
        assert_eq!(DisputeOutcome::default(), DisputeOutcome::Upheld);
    }

    #[test]
    fn dispute_by_and_events_serialize_tagged() {
        use renox::serde_json::{json, to_value};
        assert_eq!(
            to_value(DisputeBy::User { id: 4 }).unwrap(),
            json!({ "kind": "user", "id": 4 })
        );
        assert_eq!(
            to_value(DisputeBy::System).unwrap(),
            json!({ "kind": "system" })
        );
        let event = InvoiceDisputed {
            invoice: InvoiceId(1),
            party: PartyId(2),
            source: Subject::new("leasing.lease", 5),
            dispute: 7,
            amount: 120_000,
            lines: vec![DisputedLine {
                position: 2,
                amount: 80_000,
                origin: None,
            }],
            ticket: Some(TicketId(11)),
            raised_by: DisputeBy::Party { id: PartyId(2) },
        };
        let json = to_value(&event).unwrap();
        assert_eq!(json["lines"][0]["position"], 2);
        assert_eq!(json["raised_by"], json!({ "kind": "party", "id": 2 }));
    }

    #[test]
    fn allocation_proposal_is_capped_by_the_balance() {
        let open = summary(InvoiceStatus::Issued, 105_000);
        assert_eq!(allocation_proposal(&open, 200_000), 105_000);
        assert_eq!(allocation_proposal(&open, 50_000), 50_000);
        let off = summary(InvoiceStatus::WrittenOff, 52_500);
        assert_eq!(allocation_proposal(&off, 200_000), 52_500);
        assert_eq!(allocation_proposal(&summary(InvoiceStatus::Paid, 0), 9), 0);
        assert_eq!(
            allocation_proposal(&summary(InvoiceStatus::Credited, 0), 9),
            0
        );
        assert_eq!(allocation_proposal(&open, -5), 0);
    }

    #[test]
    fn bad_debt_recovered_serializes_with_plain_ids() {
        let e = BadDebtRecovered {
            invoice: InvoiceId(1),
            party: PartyId(2),
            source: Subject::new("payments.payment", 3),
            amount: 52_500,
            relief_claimed: true,
            recovered_on: NaiveDate::from_ymd_opt(2026, 10, 18).unwrap(),
        };
        let json = renox::serde_json::to_value(&e).unwrap();
        assert_eq!(json["amount"], 52_500);
        assert_eq!(json["relief_claimed"], true);
        assert_eq!(json["recovered_on"], "2026-10-18");
    }

    #[test]
    fn only_standard_carries_vat() {
        assert_eq!(VatCategory::Standard.rate_bp(), 500);
        assert!(VatCategory::Standard.carries_vat());
        for category in [
            VatCategory::ZeroRated,
            VatCategory::Exempt,
            VatCategory::OutOfScope,
            VatCategory::ReverseCharge,
        ] {
            assert_eq!(category.rate_bp(), 0);
            assert!(!category.carries_vat());
        }
    }

    #[test]
    fn categories_and_statuses_are_stored_in_snake_case() {
        let names: Vec<_> = VatCategory::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            names,
            [
                "standard",
                "zero_rated",
                "exempt",
                "out_of_scope",
                "reverse_charge"
            ]
        );
        assert_eq!(
            "zero_rated".parse::<VatCategory>().unwrap(),
            VatCategory::ZeroRated
        );
        let states: Vec<_> = InvoiceStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            states,
            [
                "draft",
                "issued",
                "partially_paid",
                "overdue",
                "paid",
                "written_off",
                "credited"
            ]
        );
        assert_eq!(InvoiceStatus::default(), InvoiceStatus::Draft);
    }

    #[test]
    fn only_issued_partly_paid_and_overdue_invoices_are_open() {
        let open: Vec<_> = InvoiceStatus::ALL
            .iter()
            .filter(|s| s.is_open())
            .copied()
            .collect();
        assert_eq!(
            open,
            [
                InvoiceStatus::Issued,
                InvoiceStatus::PartiallyPaid,
                InvoiceStatus::Overdue
            ]
        );
    }

    #[test]
    fn an_invoice_id_is_a_subject() {
        let invoice = InvoiceId(42);
        assert_eq!(invoice.subject().to_string(), "invoicing.invoice:42");
        assert_eq!(invoice.to_string(), "42");
        assert_eq!(InvoiceId::from(7), InvoiceId(7));
    }

    #[test]
    fn invoice_series_are_gapless_per_entity_and_yearly() {
        for series in [&INVOICE_SERIES, &INVOICE_SERIES_SINGLE] {
            assert_eq!(series.key, "invoice");
            assert_eq!(series.prefix, "INV");
            assert_eq!(series.reset, Reset::Yearly);
            assert!(series.gapless && series.per_entity);
        }
        assert_eq!(INVOICE_SERIES.pattern, "{prefix}-{entity}-{yyyy}-{0000}");
        assert_eq!(INVOICE_SERIES_SINGLE.pattern, "{prefix}-{yyyy}-{0000}");
        assert_eq!(invoice_series(1), &INVOICE_SERIES_SINGLE);
        assert_eq!(invoice_series(2), &INVOICE_SERIES);
        assert_eq!(invoice_series(0), &INVOICE_SERIES);
    }

    #[test]
    fn a_draft_defaults_to_aed_and_keeps_its_lines_in_order() {
        let lease = Subject::new("property.lease", 9);
        let draft = InvoiceDraft::new(PartyId(3), lease.clone())
            .line(DraftLine::new("Rent", 100_000, VatCategory::Exempt))
            .line(
                DraftLine::new("Parking", 25_000, VatCategory::Standard)
                    .quantity_milli(2_000)
                    .discount(5_000)
                    .origin(Subject::new("property.charge", 4)),
            );
        assert_eq!(draft.currency, "AED");
        assert_eq!(draft.source, lease);
        assert_eq!((draft.scope.as_ref(), draft.legal_entity_id), (None, None));
        assert_eq!(draft.lines.len(), 2);
        assert_eq!(draft.lines[0].quantity_milli, 1_000);
        assert_eq!(draft.lines[0].discount, 0);
        let parking = &draft.lines[1];
        assert_eq!(
            (parking.quantity_milli, parking.unit_price, parking.discount),
            (2_000, 25_000, 5_000)
        );
        assert_eq!(parking.origin, Some(Subject::new("property.charge", 4)));
        let usd = InvoiceDraft::new(PartyId(3), lease)
            .currency("USD")
            .legal_entity(2);
        assert_eq!(
            (usd.currency.as_str(), usd.legal_entity_id),
            ("USD", Some(2))
        );
    }

    #[test]
    fn the_unallocate_fallback_is_a_422() {
        match unallocating_off(InvoiceId(5)) {
            Error::Status(code, message) => {
                assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
                assert!(message.contains("invoice 5"), "{message}");
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn an_unallocation_is_built_with_its_builder() {
        let request = Unallocate::new(
            InvoiceId(3),
            Subject::new("payments.payment", 9),
            200_000,
            "payments.allocation:4",
            ReopenReason::PaymentReversed,
        );
        assert_eq!(request.reversal_reason, None);
        assert_eq!(request.ran_by, None);
        let request = request.reversal_reason("bank_return").ran_by(7);
        assert_eq!(request.reversal_reason.as_deref(), Some("bank_return"));
        assert_eq!(request.ran_by, Some(7));
        assert_eq!(request.key, "payments.allocation:4");
    }

    #[test]
    fn reopen_reasons_are_stored_in_snake_case() {
        for (reason, text) in [
            (ReopenReason::PaymentReversed, "payment_reversed"),
            (ReopenReason::AllocationMoved, "allocation_moved"),
            (ReopenReason::RecoveryReversed, "recovery_reversed"),
        ] {
            assert_eq!(
                renox::serde_json::to_value(reason).unwrap(),
                renox::serde_json::json!(text)
            );
        }
    }

    #[test]
    fn invoice_reopened_serializes_with_plain_ids() {
        let reopened = InvoiceReopened {
            invoice: InvoiceId(1),
            party: PartyId(2),
            source: Subject::new("property.lease", 3),
            money_source: Subject::new("payments.payment", 4),
            amount: 200_000,
            balance: 200_000,
            new_status: InvoiceStatus::Issued,
            reason: ReopenReason::PaymentReversed,
            reversal_reason: Some("bank_return".into()),
        };
        let json = renox::serde_json::to_value(&reopened).unwrap();
        assert_eq!(json["invoice"], 1);
        assert_eq!(json["party"], 2);
        assert_eq!(json["amount"], 200_000);
        assert_eq!(json["new_status"], "issued");
        assert_eq!(json["reason"], "payment_reversed");
        assert_eq!(json["reversal_reason"], "bank_return");
    }

    #[test]
    fn the_fallback_refuses_an_allocation_with_422() {
        match invoicing_off(InvoiceId(5)) {
            Error::Status(code, message) => {
                assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
                assert!(message.contains("invoice 5"), "{message}");
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn a_credit_note_id_is_a_subject() {
        let note = CreditNoteId(7);
        assert_eq!(note.subject().to_string(), "invoicing.credit_note:7");
        assert_eq!(note.to_string(), "7");
        assert_eq!(CreditNoteId::from(3), CreditNoteId(3));
        assert_eq!(
            CreditLine::new(2, 500),
            CreditLine {
                invoice_line: 2,
                net: 500
            }
        );
    }

    #[test]
    fn credit_note_series_are_gapless_per_entity_and_yearly() {
        for series in [&CREDIT_NOTE_SERIES, &CREDIT_NOTE_SERIES_SINGLE] {
            assert_eq!(series.key, "credit_note");
            assert_eq!(series.prefix, "CN");
            assert_eq!(series.reset, Reset::Yearly);
            assert!(series.gapless && series.per_entity);
        }
        assert_eq!(
            CREDIT_NOTE_SERIES.pattern,
            "{prefix}-{entity}-{yyyy}-{0000}"
        );
        assert_eq!(CREDIT_NOTE_SERIES_SINGLE.pattern, "{prefix}-{yyyy}-{0000}");
        assert_eq!(credit_note_series(1), &CREDIT_NOTE_SERIES_SINGLE);
        assert_eq!(credit_note_series(2), &CREDIT_NOTE_SERIES);
        assert_eq!(credit_note_series(0), &CREDIT_NOTE_SERIES);
        assert_ne!(CREDIT_NOTE_SERIES.key, INVOICE_SERIES.key);
    }

    #[test]
    fn a_credit_note_starts_as_a_draft() {
        assert_eq!(CreditNoteStatus::default(), CreditNoteStatus::Draft);
    }

    #[test]
    fn the_fallback_refuses_crediting_with_422() {
        match crediting_off(InvoiceId(5)) {
            Error::Status(code, message) => {
                assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
                assert!(message.contains("invoice 5"), "{message}");
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn credit_note_events_serialize_with_plain_ids() {
        let issued = CreditNoteIssued {
            credit_note: CreditNoteId(7),
            invoice: InvoiceId(1),
            party: PartyId(2),
            amount: 21_000,
            reason: "price error".into(),
        };
        let json = renox::serde_json::to_value(&issued).unwrap();
        assert_eq!(json["credit_note"], 7);
        assert_eq!(json["invoice"], 1);
        assert_eq!(json["party"], 2);
        assert_eq!(json["amount"], 21_000);

        let credited = InvoiceCredited {
            invoice: InvoiceId(1),
            credit_note: CreditNoteId(7),
            amount: 105_000,
            reason: "cancelled".into(),
            released_origins: vec![Subject::new("property.charge", 4)],
        };
        let json = renox::serde_json::to_value(&credited).unwrap();
        assert_eq!(json["invoice"], 1);
        assert_eq!(json["credit_note"], 7);
        assert_eq!(json["released_origins"].as_array().unwrap().len(), 1);

        let reopened = CreditNoteReopened {
            credit_note: CreditNoteId(7),
            party: PartyId(2),
            amount_restored: 50_000,
            cause: Subject::new("payments.refund", 9),
        };
        let json = renox::serde_json::to_value(&reopened).unwrap();
        assert_eq!(json["credit_note"], 7);
        assert_eq!(json["party"], 2);
        assert_eq!(json["amount_restored"], 50_000);
    }

    #[test]
    fn events_serialize_with_plain_ids() {
        let written_off = InvoiceWrittenOff {
            invoice: InvoiceId(1),
            party: PartyId(2),
            source: Subject::new("property.lease", 3),
            amount: 105_000,
            reason_code: "insolvent".into(),
            approver: None,
        };
        let json = renox::serde_json::to_value(&written_off).unwrap();
        assert_eq!(json["invoice"], 1);
        assert_eq!(json["party"], 2);
        assert_eq!(json["amount"], 105_000);
        assert!(json["approver"].is_null());
    }
}
