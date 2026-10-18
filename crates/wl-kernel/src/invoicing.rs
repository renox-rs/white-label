//! The invoicing contract (ARCHITECTURE §5, wl-invoicing §6): what every
//! module may do with invoices without depending on the `wl-invoicing`
//! crate.
//!
//! `wl-invoicing` provides an [`Invoicer`] with [`provide_invoicer`] and
//! emits [`InvoiceIssued`], [`InvoiceOverdue`], [`InvoicePaid`] and
//! [`InvoiceWrittenOff`]. Other modules draft, read and allocate invoices
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

/// The subject type of an invoice, for [`Subject`] references.
pub const INVOICE_SUBJECT_TYPE: &str = "invoicing.invoice";

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
}

/// The future an [`Invoicer`] method returns.
pub type InvoiceFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

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
    /// ≤ 0 or above the balance, or an invoice that is not open.
    fn allocate<'a>(
        &'a self,
        state: &'a AppState,
        id: InvoiceId,
        source: Subject,
        amount: i64,
    ) -> InvoiceFuture<'a, ()>;
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

#[cfg(test)]
mod tests {
    use super::*;

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
