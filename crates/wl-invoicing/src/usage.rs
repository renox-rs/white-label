//! Usage records and the usage billing run (wl-invoicing §3 *Usage record*,
//! §5.2 steps 1 to 3), as far as hours billed from timesheets need.
//!
//! A vertical records the hours it bills with [`record`]: the party, the
//! source (`attendance.timesheet:3`), the hours category, the quantity in
//! thousandths of an hour and the price of one hour as the rate card priced
//! it, with the multipliers already applied and the rate date it was taken
//! at. [`bill`] turns the party's unbilled records into one draft invoice per
//! issuing entity and currency, a line per record, and stores on each record
//! the invoice and the line position, so that an adjustment timesheet
//! ([`crate::usage_adjustment`]) can later find what was billed where.
//!
//! The rate card itself (§3 *Rate card*) is not built here: the price comes
//! with the record.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory};
use wl_kernel::{LegalEntity, PartyId, Subject};

use crate::invoice::Invoice;
use crate::invoicer::create_draft;

/// The source type of the draft invoices of the billing run.
pub const BILLING_SOURCE: &str = "invoicing.usage";

/// One priced usage waiting to be billed, or already billed. Money is fils of
/// [`UsageRecord::currency`]; quantities are thousandths of an hour.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_usage_records",
    index(party_id),
    index(source_type, source_id),
    index(invoice_id)
)]
pub struct UsageRecord {
    /// The primary key.
    pub id: i64,
    /// Who is billed.
    pub party_id: i64,
    /// The issuing legal entity.
    pub legal_entity_id: i64,
    /// The ISO currency code.
    pub currency: String,
    /// The type of the source, e.g. `attendance.timesheet`.
    pub source_type: String,
    /// The id of the source.
    pub source_id: i64,
    /// The hours category of the source: `normal`, `overtime`...
    pub category: String,
    /// The text of the invoice line.
    pub description: String,
    /// Thousandths of an hour: 1000 is one hour.
    pub quantity_milli: i64,
    /// The price of one hour, after the multipliers of the rate card line.
    pub unit_price: i64,
    /// The rate date the price was taken at.
    pub rate_date: Option<NaiveDate>,
    /// The VAT treatment of the line.
    pub vat_category: VatCategory,
    /// An extra record of an adjustment: the record it corrects.
    pub adjusts_id: Option<i64>,
    /// An extra record of an adjustment: the type of the adjustment
    /// timesheet.
    pub adjustment_type: Option<String>,
    /// An extra record of an adjustment: the id of the adjustment timesheet.
    pub adjustment_id: Option<i64>,
    /// The invoice it was billed on; `None` while unbilled.
    pub invoice_id: Option<i64>,
    /// The position of its line on that invoice.
    pub invoice_position: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl UsageRecord {
    /// The source, `attendance.timesheet:3`.
    pub fn source(&self) -> Subject {
        Subject::new(&self.source_type, self.source_id)
    }

    /// The adjustment timesheet an extra record names, if it is one.
    pub fn adjustment(&self) -> Option<Subject> {
        match (&self.adjustment_type, self.adjustment_id) {
            (Some(subject_type), Some(id)) => Some(Subject::new(subject_type, id)),
            _ => None,
        }
    }

    /// Who is billed.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// The invoice line of this record, as a draft line: the hours at the
    /// record's own price, billing the source (or the adjustment).
    pub fn draft_line(&self) -> DraftLine {
        let origin = self.adjustment().unwrap_or_else(|| self.source());
        DraftLine::new(&self.description, self.unit_price, self.vat_category)
            .quantity_milli(self.quantity_milli)
            .origin(origin)
    }
}

/// What a vertical records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageDraft {
    /// Who is billed.
    pub party: PartyId,
    /// The issuing legal entity; `None` for the default one.
    pub legal_entity_id: Option<i64>,
    /// The ISO currency code.
    pub currency: String,
    /// The source, e.g. `attendance.timesheet:3`.
    pub source: Subject,
    /// The hours category.
    pub category: String,
    /// The text of the invoice line.
    pub description: String,
    /// Thousandths of an hour.
    pub quantity_milli: i64,
    /// The price of one hour in fils, multipliers applied.
    pub unit_price: i64,
    /// The rate date.
    pub rate_date: Option<NaiveDate>,
    /// The VAT treatment.
    pub vat_category: VatCategory,
}

impl UsageDraft {
    /// `quantity_milli` thousandths of an hour of `category` from `source`
    /// at `unit_price` fils an hour, in AED with standard VAT.
    pub fn hours(
        party: PartyId,
        source: Subject,
        category: &str,
        quantity_milli: i64,
        unit_price: i64,
    ) -> Self {
        UsageDraft {
            party,
            legal_entity_id: None,
            currency: "AED".to_owned(),
            description: format!("{category} hours, {source}"),
            source,
            category: category.to_owned(),
            quantity_milli,
            unit_price,
            rate_date: None,
            vat_category: VatCategory::Standard,
        }
    }

    /// Billed by the legal entity `id`.
    pub fn legal_entity(mut self, id: i64) -> Self {
        self.legal_entity_id = Some(id);
        self
    }

    /// With this line text.
    pub fn description(mut self, text: &str) -> Self {
        self.description = text.to_owned();
        self
    }

    /// Priced at the rate of `date`.
    pub fn rate_date(mut self, date: NaiveDate) -> Self {
        self.rate_date = Some(date);
        self
    }
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Stores `draft` as an unbilled usage record.
///
/// # Errors
///
/// 422 for a quantity of 0 or less, a negative price or a blank line text.
pub async fn record(db: &Db, draft: UsageDraft) -> Result<UsageRecord> {
    if draft.quantity_milli <= 0 || draft.unit_price < 0 {
        return Err(refused("A usage record needs hours above 0 and a price."));
    }
    if draft.description.trim().is_empty() {
        return Err(refused("A usage record needs a line text."));
    }
    let entity = match draft.legal_entity_id {
        Some(id) => LegalEntity::find_or_404(db, id).await?.id,
        None => LegalEntity::for_scope(db, None).await?.id,
    };
    let mut record = UsageRecord {
        party_id: draft.party.0,
        legal_entity_id: entity,
        currency: draft.currency.trim().to_ascii_uppercase(),
        source_type: draft.source.subject_type,
        source_id: draft.source.subject_id,
        category: draft.category,
        description: draft.description.trim().to_owned(),
        quantity_milli: draft.quantity_milli,
        unit_price: draft.unit_price,
        rate_date: draft.rate_date,
        vat_category: draft.vat_category,
        ..Default::default()
    };
    record.insert(db).await?;
    Ok(record)
}

/// The unbilled records of `party`, oldest first.
pub async fn unbilled(db: &Db, party: PartyId) -> Result<Vec<UsageRecord>> {
    UsageRecord::query()
        .where_(UsageRecord::PARTY_ID.eq(party.0))
        .where_(UsageRecord::INVOICE_ID.is_null())
        .order_by(UsageRecord::ID)
        .get(db)
        .await
}

/// The records billed on invoice `invoice_id`, in line order.
pub async fn of_invoice(db: &Db, invoice_id: i64) -> Result<Vec<UsageRecord>> {
    UsageRecord::query()
        .where_(UsageRecord::INVOICE_ID.eq(invoice_id))
        .order_by(UsageRecord::INVOICE_POSITION)
        .get(db)
        .await
}

/// Makes one draft invoice of `records` (one line each, in the given order)
/// and stores on each record the invoice and the position of its line.
/// The records share their party, entity and currency.
pub(crate) async fn bill_records(db: &Db, records: &mut [UsageRecord]) -> Result<Invoice> {
    let first = records
        .first()
        .ok_or_else(|| refused("There is nothing to bill."))?;
    let mut draft = InvoiceDraft::new(first.party(), Subject::new(BILLING_SOURCE, first.id))
        .legal_entity(first.legal_entity_id)
        .currency(&first.currency);
    for record in records.iter() {
        draft = draft.line(record.draft_line());
    }
    let invoice = create_draft(db, draft).await?;
    for (record, position) in records.iter_mut().zip(1..) {
        record.invoice_id = Some(invoice.id);
        record.invoice_position = Some(position);
        record
            .save_only(db, &["invoice_id", "invoice_position"])
            .await?;
    }
    Ok(invoice)
}

/// The usage billing run for `party` (§5.2 step 3): its unbilled records
/// become one draft invoice per issuing entity and currency, a line per
/// record, grouped as the records are, oldest first. Returns the drafts; a
/// party with nothing unbilled gets none.
pub async fn bill(db: &Db, party: PartyId) -> Result<Vec<Invoice>> {
    let mut groups: BTreeMap<(i64, String), Vec<UsageRecord>> = BTreeMap::new();
    for record in unbilled(db, party).await? {
        groups
            .entry((record.legal_entity_id, record.currency.clone()))
            .or_default()
            .push(record);
    }
    let mut invoices = Vec::new();
    for (_, mut records) in groups {
        invoices.push(bill_records(db, &mut records).await?);
    }
    Ok(invoices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_becomes_a_line_of_hours_at_its_own_price() {
        let record = UsageRecord {
            id: 3,
            description: "Normal hours, week 44".into(),
            quantity_milli: 160_000,
            unit_price: 4_000,
            source_type: "attendance.timesheet".into(),
            source_id: 2,
            ..Default::default()
        };
        let line = record.draft_line();
        assert_eq!(line.net(), 640_000);
        assert_eq!(line.origin, Some(Subject::new("attendance.timesheet", 2)));
        assert_eq!(record.adjustment(), None);
    }

    #[test]
    fn an_extra_record_bills_the_adjustment() {
        let record = UsageRecord {
            quantity_milli: 6_000,
            unit_price: 4_000,
            source_type: "attendance.timesheet".into(),
            source_id: 2,
            adjustment_type: Some("attendance.timesheet".into()),
            adjustment_id: Some(9),
            ..Default::default()
        };
        let line = record.draft_line();
        assert_eq!(line.net(), 24_000);
        assert_eq!(line.origin, Some(Subject::new("attendance.timesheet", 9)));
    }

    #[test]
    fn hours_default_to_aed_and_standard_vat() {
        let draft = UsageDraft::hours(
            PartyId(1),
            Subject::new("attendance.timesheet", 2),
            "normal",
            8_000,
            4_000,
        );
        assert_eq!(draft.currency, "AED");
        assert_eq!(draft.vat_category, VatCategory::Standard);
        assert!(draft.description.contains("attendance.timesheet:2"));
    }
}
