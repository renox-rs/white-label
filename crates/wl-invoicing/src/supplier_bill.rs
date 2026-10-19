//! The Supplier bill (wl-invoicing §3): a bill from a supplier with its
//! lines ([`crate::bill_line::BillLine`]), totals and UAE VAT per line. This
//! file holds the model and the draft service (create, change, discard); the
//! machine is in [`crate::bill_machine`], `submit` in [`crate::bill_submit`]
//! and the duplicate hold in [`crate::duplicates`]. `SupplierBill` is
//! `#[model(state_machine)]`, so `save` refuses a changed status and a new
//! bill is inserted only as `draft`.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{LegalEntity, PartyId, Subject};

use crate::bill_line::{BillLine, BillLineDraft};
use crate::vat::{self, VatTotals};

/// The lifecycle state of a supplier bill (wl-invoicing §4).
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

/// The reason code of a held bill that may duplicate an earlier one.
pub const POSSIBLE_DUPLICATE: &str = "possible_duplicate";

/// A supplier bill. Money is `i64` fils of [`SupplierBill::currency`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_supplier_bills",
    state_machine,
    index(status),
    index(party_id, number_key),
    index(contract_type, contract_id)
)]
pub struct SupplierBill {
    /// The primary key.
    pub id: i64,
    /// The supplier (a party).
    pub party_id: i64,
    /// The receiving legal entity.
    pub legal_entity_id: Option<i64>,
    /// The supplier's invoice number as entered.
    pub supplier_invoice_number: String,
    /// The same number trimmed and lower-cased, for the duplicate check.
    pub number_key: String,
    /// The date on the supplier's bill.
    pub bill_date: NaiveDate,
    /// The due date, if known.
    pub due_date: Option<NaiveDate>,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// The rate to AED × 1,000,000 for a foreign currency; `None` for AED
    /// or until set.
    pub exchange_rate_micro: Option<i64>,
    /// The type of the service contract the bill is against, if any.
    pub contract_type: Option<String>,
    /// The id of the service contract the bill is against, if any.
    pub contract_id: Option<i64>,
    /// The first day of the billing period, if any.
    pub period_start: Option<NaiveDate>,
    /// The last day of the billing period, if any.
    pub period_end: Option<NaiveDate>,
    /// The sum of the line nets.
    pub net_total: i64,
    /// The sum of the line VAT.
    pub vat_total: i64,
    /// Net plus VAT.
    pub gross_total: i64,
    /// The gross in AED fils; `None` while the rate is unknown.
    pub gross_total_aed: Option<i64>,
    /// The sum of the active allocations (payments and debit notes).
    pub allocated_total: i64,
    /// What is still open: gross total minus allocations.
    pub balance: i64,
    /// The user who entered it; `None` for the system.
    pub created_by: Option<i64>,
    /// The user who approved it.
    pub approved_by: Option<i64>,
    /// The reason given with the approval.
    pub approval_reason: Option<String>,
    /// The reason given with the void.
    pub void_reason: Option<String>,
    /// The earlier bill a held bill may duplicate.
    pub duplicate_of: Option<i64>,
    /// Counts the changes of the row.
    pub lock_version: i64,
    /// The lifecycle state.
    pub status: SupplierBillStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SupplierBill {
    /// The bill as a [`Subject`]: `invoicing.supplier_bill:42`.
    pub fn subject(&self) -> Subject {
        Subject::new("invoicing.supplier_bill", self.id)
    }

    /// The supplier.
    pub fn supplier(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// The service contract the bill is against, if any.
    pub fn contract(&self) -> Option<Subject> {
        match (&self.contract_type, self.contract_id) {
            (Some(subject_type), Some(subject_id)) => Some(Subject {
                subject_type: subject_type.clone(),
                subject_id,
            }),
            _ => None,
        }
    }

    /// The billing period, if both ends are set.
    pub fn period(&self) -> Option<(NaiveDate, NaiveDate)> {
        self.period_start.zip(self.period_end)
    }

    /// The rate to AED: 1 for AED, else the stored rate.
    pub fn aed_rate(&self) -> Option<i64> {
        vat::aed_rate(&self.currency, self.exchange_rate_micro)
    }

    /// Stores `totals` as the bill's totals, with the gross in AED when the
    /// rate is known.
    pub fn set_totals(&mut self, totals: &VatTotals) {
        self.net_total = totals.net;
        self.vat_total = totals.vat;
        self.gross_total = totals.gross;
        self.gross_total_aed = self.aed_rate().map(|rate| vat::to_aed(totals.gross, rate));
        self.balance = totals.gross - self.allocated_total;
    }
}

/// The supplier invoice number without surrounding spaces and in lower case:
/// what the duplicate check compares.
pub fn number_key(number: &str) -> String {
    number.trim().to_lowercase()
}

/// What a user enters for a supplier bill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillDraft {
    /// The supplier.
    pub supplier: PartyId,
    /// The receiving legal entity; the default one when `None`.
    pub legal_entity_id: Option<i64>,
    /// The supplier's invoice number.
    pub supplier_invoice_number: String,
    /// The date on the supplier's bill.
    pub bill_date: NaiveDate,
    /// The due date, if known.
    pub due_date: Option<NaiveDate>,
    /// The ISO currency code.
    pub currency: String,
    /// The rate to AED × 1,000,000 of a foreign currency.
    pub exchange_rate_micro: Option<i64>,
    /// The service contract the bill is against, if any.
    pub contract: Option<Subject>,
    /// The billing period (first and last day), if any.
    pub period: Option<(NaiveDate, NaiveDate)>,
    /// The lines.
    pub lines: Vec<BillLineDraft>,
}

impl BillDraft {
    /// A draft in AED with no lines.
    pub fn new(supplier: PartyId, supplier_invoice_number: &str, bill_date: NaiveDate) -> Self {
        Self {
            supplier,
            legal_entity_id: None,
            supplier_invoice_number: supplier_invoice_number.to_owned(),
            bill_date,
            due_date: None,
            currency: wl_kernel::invoicing::DEFAULT_CURRENCY.to_owned(),
            exchange_rate_micro: None,
            contract: None,
            period: None,
            lines: Vec::new(),
        }
    }

    /// With a line.
    pub fn line(mut self, line: BillLineDraft) -> Self {
        self.lines.push(line);
        self
    }

    /// Against a service contract, for a billing period.
    pub fn contract(mut self, contract: Subject, period: Option<(NaiveDate, NaiveDate)>) -> Self {
        self.contract = Some(contract);
        self.period = period;
        self
    }

    /// In a foreign currency at a rate to AED × 1,000,000.
    pub fn currency(mut self, currency: &str, exchange_rate_micro: Option<i64>) -> Self {
        self.currency = currency.to_owned();
        self.exchange_rate_micro = exchange_rate_micro;
        self
    }

    /// With a due date.
    pub fn due_date(mut self, due_date: NaiveDate) -> Self {
        self.due_date = Some(due_date);
        self
    }
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Why `draft` cannot be saved, if it cannot.
fn problem(draft: &BillDraft, currency: &str) -> Option<String> {
    if draft.supplier_invoice_number.trim().is_empty() {
        return Some("the supplier invoice number is empty".to_owned());
    }
    if !crate::invoicer::is_currency(currency) {
        return Some(format!(
            "`{}` is not a currency code like AED",
            draft.currency
        ));
    }
    if draft.exchange_rate_micro.is_some_and(|rate| rate <= 0) {
        return Some("the exchange rate must be above 0".to_owned());
    }
    if draft.due_date.is_some_and(|due| due < draft.bill_date) {
        return Some("the due date is before the bill date".to_owned());
    }
    if draft.period.is_some_and(|(start, end)| end < start) {
        return Some("the billing period ends before it starts".to_owned());
    }
    draft
        .lines
        .iter()
        .enumerate()
        .find_map(|(i, line)| line.problem(i + 1))
}

/// A bill from `draft` and its lines (not yet saved), with totals.
async fn build(
    db: &Db,
    draft: &BillDraft,
    current: Option<&SupplierBill>,
    created_by: Option<i64>,
) -> Result<(SupplierBill, Vec<BillLine>)> {
    let currency = draft.currency.trim().to_ascii_uppercase();
    if let Some(message) = problem(draft, &currency) {
        return Err(refused(message));
    }
    let legal_entity_id = match draft.legal_entity_id {
        Some(id) => LegalEntity::find_or_404(db, id).await?.id,
        None => LegalEntity::default_entity(db).await?.id,
    };
    let mut bill = current.cloned().unwrap_or_default();
    bill.party_id = draft.supplier.0;
    bill.legal_entity_id = Some(legal_entity_id);
    bill.supplier_invoice_number = draft.supplier_invoice_number.trim().to_owned();
    bill.number_key = number_key(&draft.supplier_invoice_number);
    bill.bill_date = draft.bill_date;
    bill.due_date = draft.due_date;
    bill.exchange_rate_micro = if currency == wl_kernel::invoicing::DEFAULT_CURRENCY {
        None
    } else {
        draft.exchange_rate_micro
    };
    bill.currency = currency;
    bill.contract_type = draft.contract.as_ref().map(|c| c.subject_type.clone());
    bill.contract_id = draft.contract.as_ref().map(|c| c.subject_id);
    bill.period_start = draft.period.map(|(start, _)| start);
    bill.period_end = draft.period.map(|(_, end)| end);
    if current.is_none() {
        bill.created_by = created_by;
        bill.status = SupplierBillStatus::Draft;
    }
    let rate = bill.aed_rate();
    let lines: Vec<BillLine> = draft
        .lines
        .iter()
        .zip(1..)
        .map(|(line, position)| BillLine::from_draft(0, position, line, rate))
        .collect();
    let amounts: Vec<_> = lines.iter().map(BillLine::amounts).collect();
    bill.set_totals(&VatTotals::of(&amounts));
    Ok((bill, lines))
}

/// Creates a `draft` supplier bill with its lines and totals in one
/// transaction. The receiving entity is the draft's, else the default one.
///
/// Refused (422) for an empty supplier invoice number, a currency that is
/// not an ISO code, a rate of 0 or less, a due date before the bill date, a
/// period that ends before it starts, or a line with no description, a
/// quantity of 0 or less, or a negative price. A bill with no lines can be
/// saved; `submit` wants at least one.
pub async fn create(db: &Db, draft: &BillDraft, created_by: Option<i64>) -> Result<SupplierBill> {
    let (mut bill, mut lines) = build(db, draft, None, created_by).await?;
    let mut tx = db.begin().await?;
    bill.insert(&mut tx).await?;
    for line in &mut lines {
        line.bill_id = bill.id;
        line.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(bill)
}

/// Replaces the header, lines and totals of draft bill `id` in one
/// transaction. Same refusals as [`create`], and 422 when it is no longer a
/// `draft`.
pub async fn change(db: &Db, id: i64, draft: &BillDraft) -> Result<SupplierBill> {
    let current = SupplierBill::find_or_404(db, id).await?;
    if current.status != SupplierBillStatus::Draft {
        return Err(refused("only a draft supplier bill can be changed"));
    }
    let (mut bill, mut lines) = build(db, draft, Some(&current), None).await?;
    bill.lock_version += 1;
    let mut tx = db.begin().await?;
    bill.save(&mut tx).await?;
    let old_lines = BillLine::query()
        .where_(BillLine::BILL_ID.eq(id))
        .get(&mut tx)
        .await?;
    for mut old in old_lines {
        old.delete(&mut tx).await?;
    }
    for line in &mut lines {
        line.bill_id = id;
        line.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(bill)
}

/// Deletes draft bill `id` and its lines. Refused (422) when it is no longer
/// a `draft`: a submitted bill is never deleted.
pub async fn discard(db: &Db, id: i64) -> Result<()> {
    let mut bill = SupplierBill::find_or_404(db, id).await?;
    if bill.status != SupplierBillStatus::Draft {
        return Err(refused("only a draft supplier bill can be deleted"));
    }
    let mut tx = db.begin().await?;
    let old_lines = BillLine::query()
        .where_(BillLine::BILL_ID.eq(id))
        .get(&mut tx)
        .await?;
    for mut old in old_lines {
        old.delete(&mut tx).await?;
    }
    bill.delete(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

/// The lines of bill `bill_id`, in order.
pub async fn lines_of(db: &Db, bill_id: i64) -> Result<Vec<BillLine>> {
    BillLine::query()
        .where_(BillLine::BILL_ID.eq(bill_id))
        .order_by(BillLine::POSITION)
        .get(db)
        .await
}

/// The totals per VAT category of bill `bill_id`, from its stored lines.
pub async fn totals_of(db: &Db, bill_id: i64) -> Result<VatTotals> {
    let lines = lines_of(db, bill_id).await?;
    let amounts: Vec<_> = lines.iter().map(BillLine::amounts).collect();
    Ok(VatTotals::of(&amounts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::invoicing::VatCategory;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn statuses_are_the_seven_states_in_snake_case() {
        let all: Vec<_> = SupplierBillStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            all,
            [
                "draft",
                "matched",
                "exception",
                "approved",
                "partially_paid",
                "paid",
                "void"
            ]
        );
    }

    #[test]
    fn the_number_key_is_trimmed_and_lower_case() {
        assert_eq!(number_key(" INV-77 "), "inv-77");
        assert_eq!(number_key("inv-77"), number_key("Inv-77\t"));
    }

    #[test]
    fn totals_set_the_aed_gross_when_the_rate_is_known() {
        let line = crate::vat::LineAmounts::compute(1_000, 100_000, 0, VatCategory::Standard, None);
        let mut bill = SupplierBill {
            id: 4,
            currency: "AED".into(),
            ..Default::default()
        };
        bill.set_totals(&VatTotals::of(&[line]));
        assert_eq!(
            (bill.net_total, bill.vat_total, bill.gross_total),
            (100_000, 5_000, 105_000)
        );
        assert_eq!(bill.gross_total_aed, Some(105_000));
        bill.currency = "USD".into();
        bill.set_totals(&VatTotals::of(&[line]));
        assert_eq!(bill.gross_total_aed, None);
        bill.exchange_rate_micro = Some(3_670_000);
        bill.set_totals(&VatTotals::of(&[line]));
        assert_eq!(bill.gross_total_aed, Some(385_350));
        assert_eq!(bill.balance, 105_000);
        bill.allocated_total = 40_000;
        bill.set_totals(&VatTotals::of(&[line]));
        assert_eq!(bill.balance, 65_000);
        assert_eq!(bill.subject().to_string(), "invoicing.supplier_bill:4");
        assert_eq!(bill.contract(), None);
        assert_eq!(bill.period(), None);
    }

    #[test]
    fn a_draft_is_checked_before_saving() {
        let ok = BillDraft::new(PartyId(1), "INV-1", day(5)).line(BillLineDraft::new(
            "Pump",
            100,
            VatCategory::Standard,
        ));
        assert_eq!(problem(&ok, "AED"), None);
        let mut blank = ok.clone();
        blank.supplier_invoice_number = "  ".into();
        assert!(problem(&blank, "AED").unwrap().contains("invoice number"));
        assert!(problem(&ok, "DIRHAM").unwrap().contains("currency"));
        let rate = ok.clone().currency("USD", Some(0));
        assert!(problem(&rate, "USD").unwrap().contains("rate"));
        let due = ok.clone().due_date(day(4));
        assert!(problem(&due, "AED").unwrap().contains("due date"));
        let period = ok.clone().contract(
            Subject::new("ticketing.contract", 1),
            Some((day(9), day(1))),
        );
        assert!(problem(&period, "AED").unwrap().contains("period"));
        let line = ok
            .clone()
            .line(BillLineDraft::new("", 1, VatCategory::Exempt));
        assert!(problem(&line, "AED").unwrap().contains("line 2"));
    }
}
