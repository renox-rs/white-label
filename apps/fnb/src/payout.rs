//! The aggregator payout statement and its records (06 F&B §4.3, §6, §7.1
//! *Payout statement*; the base for BL-FNB-09 and BL-FNB-10).
//!
//! The accountant imports the file an aggregator sends for a period
//! ([`crate::payout_csv`], [`crate::payouts::import`]). Each line names an
//! order by the aggregator's own number and says what the aggregator earned
//! on it: gross, commission, promotion share, refund and net. The status moves
//! only through [`crate::payout_machine`].

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::numbering::{Reset, Series};

/// The subject type of a payout statement.
pub const SUBJECT_TYPE: &str = "fnb.payout_statement";

/// `PAY-0001`: the number of a statement; kept when it is voided and never
/// given again.
pub const STATEMENT_SERIES: Series = Series {
    key: "fnb_payout_statement",
    pattern: "{prefix}-{0000}",
    prefix: "PAY",
    reset: Reset::Never,
    gapless: false,
    per_entity: false,
};

/// Where a payout statement is (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PayoutStatus {
    /// File loaded; its lines are being matched.
    #[default]
    Imported,
    /// Every line matched or explained; the receivable is booked.
    Matched,
    /// Disputed lines are raised with the aggregator.
    Disputed,
    /// The payout was received and allocated (terminal).
    Settled,
    /// A wrong file, imported in error (terminal).
    Voided,
}

/// How a line stands against the orders.
pub mod line_state {
    /// An order with that number was found.
    pub const MATCHED: &str = "matched";
    /// No order with that number is recorded.
    pub const UNMATCHED: &str = "unmatched";
    /// No order, and the accountant wrote why that is fine.
    pub const EXPLAINED: &str = "explained";
}

/// A payout statement.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "fnb_payout_statements",
    state_machine,
    index(status),
    index(aggregator)
)]
pub struct PayoutStatement {
    /// The primary key.
    pub id: i64,
    /// The number, e.g. `PAY-0001`.
    pub number: String,
    /// The aggregator: the sales channel of its orders, lower case.
    pub aggregator: String,
    /// The aggregator's own reference of the payout, as typed.
    pub reference: String,
    /// The reference trimmed and lower-cased: unique per aggregator while the
    /// statement is not voided.
    pub reference_key: String,
    /// The name of the imported file.
    pub file_name: String,
    /// The name of the column preset the file was read with.
    pub preset: String,
    /// The first day of the period.
    pub period_from: Option<NaiveDate>,
    /// The last day of the period.
    pub period_to: Option<NaiveDate>,
    /// How many lines the file had.
    pub line_count: i64,
    /// The sum of the gross, in fils.
    pub gross_total: i64,
    /// The sum of the commissions, in fils.
    pub commission_total: i64,
    /// The sum of the promotion shares, in fils.
    pub promo_total: i64,
    /// The sum of the refunds, in fils.
    pub refund_total: i64,
    /// The sum of the net, in fils: the payout the aggregator owes.
    pub net_total: i64,
    /// The expected payout booked as owed on `match`, in fils.
    pub receivable_amount: i64,
    /// When the receivable was booked.
    pub receivable_booked_at: Option<DateTime>,
    /// The accountant who imported it.
    pub imported_by: Option<i64>,
    /// When it was matched.
    pub matched_at: Option<DateTime>,
    /// Who matched it.
    pub matched_by: Option<i64>,
    /// Why it is disputed.
    pub dispute_reason: Option<String>,
    /// When it was disputed.
    pub disputed_at: Option<DateTime>,
    /// The follow-up task of the dispute.
    pub dispute_task_id: Option<i64>,
    /// What the aggregator answered.
    pub resolve_note: Option<String>,
    /// When the dispute was resolved.
    pub resolved_at: Option<DateTime>,
    /// The reconciliation task of `FNB.RUL.10`.
    pub task_id: Option<i64>,
    /// When the payout was settled.
    pub settled_at: Option<DateTime>,
    /// Why it was voided.
    pub void_reason: Option<String>,
    /// When it was voided.
    pub voided_at: Option<DateTime>,
    /// Who voided it.
    pub voided_by: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: PayoutStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PayoutStatement {
    /// The statement as a [`Subject`].
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The statements, newest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn newest(db: &Db, limit: u64) -> Result<Vec<PayoutStatement>> {
        PayoutStatement::query()
            .order_by_desc(PayoutStatement::ID)
            .limit(limit)
            .get(db)
            .await
    }

    /// The statement that holds `reference` of `aggregator` and is not voided.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn using(
        db: &Db,
        aggregator: &str,
        reference: &str,
    ) -> Result<Option<PayoutStatement>> {
        PayoutStatement::query()
            .where_(PayoutStatement::AGGREGATOR.eq(aggregator_key(aggregator)))
            .where_(PayoutStatement::REFERENCE_KEY.eq(reference_key(reference)))
            .where_(PayoutStatement::STATUS.ne(PayoutStatus::Voided))
            .first(db)
            .await
    }
}

/// The aggregator as stored: trimmed, lower case.
pub fn aggregator_key(aggregator: &str) -> String {
    aggregator.trim().to_lowercase()
}

/// A reference compared as the spec says: trimmed, case-insensitive.
pub fn reference_key(reference: &str) -> String {
    reference.trim().to_lowercase()
}

/// A line of a payout statement.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_payout_lines", index(statement_id))]
pub struct PayoutLine {
    /// The primary key.
    pub id: i64,
    /// The statement.
    #[model(references = "fnb_payout_statements")]
    pub statement_id: i64,
    /// The position in the file, from 1.
    pub line_no: i64,
    /// The aggregator's order number as printed.
    pub order_number: String,
    /// The number trimmed and lower-cased.
    pub order_key: String,
    /// What the customer paid for the order, in fils.
    pub gross: i64,
    /// The aggregator's commission, in fils (positive).
    pub commission: i64,
    /// The promotion share the business carries, in fils (positive).
    pub promo: i64,
    /// What the aggregator refunded its customer and deducted, in fils
    /// (positive).
    pub refund: i64,
    /// What the aggregator pays for the order, in fils.
    pub net: i64,
    /// The aggregator's refund reason as printed.
    pub reason: String,
    /// The complaint category of the refund (a stored name of the sales
    /// settings).
    pub category: Option<String>,
    /// The responsible party of the refund (a stored name of the sales
    /// settings): `business` or `aggregator`.
    pub party: Option<String>,
    /// The sales order the number was matched to.
    pub sales_order_id: Option<i64>,
    /// The F&B order of it.
    pub order_id: Option<i64>,
    /// The order entered late for it.
    pub late_order_id: Option<i64>,
    /// One of [`line_state`].
    pub match_state: String,
    /// Why an unmatched line is fine.
    pub explanation: Option<String>,
    /// The sales return the refund became (BL-FNB-10).
    pub return_id: Option<i64>,
    /// When that return was taken back.
    pub return_reversed_at: Option<DateTime>,
    /// Whether the line is disputed with the aggregator.
    pub disputed: bool,
    /// Why.
    pub dispute_note: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PayoutLine {
    /// The lines of `statement_id`, in file order.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_statement(db: &Db, statement_id: i64) -> Result<Vec<PayoutLine>> {
        PayoutLine::query()
            .where_(PayoutLine::STATEMENT_ID.eq(statement_id))
            .order_by(PayoutLine::LINE_NO)
            .get(db)
            .await
    }

    /// Whether the line is a refund the aggregator deducted.
    pub fn is_refund(&self) -> bool {
        self.refund > 0
    }

    /// Whether the line still blocks `match`: no order, and not explained.
    pub fn is_unmatched(&self) -> bool {
        self.match_state == line_state::UNMATCHED
    }
}

/// The mapping of an aggregator's refund reason to a complaint category and a
/// proposed responsible party.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_payout_reasons")]
pub struct ReasonMap {
    /// The primary key.
    pub id: i64,
    /// The aggregator.
    pub aggregator: String,
    /// The reason text trimmed and lower-cased.
    pub reason_key: String,
    /// The complaint category.
    pub category: String,
    /// The proposed party.
    pub party: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ReasonMap {
    /// The mapping of `reason` for `aggregator`, if there is one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn find_for(db: &Db, aggregator: &str, reason: &str) -> Result<Option<ReasonMap>> {
        ReasonMap::query()
            .where_(ReasonMap::AGGREGATOR.eq(aggregator_key(aggregator)))
            .where_(ReasonMap::REASON_KEY.eq(reference_key(reason)))
            .first(db)
            .await
    }
}

/// The column mapping of an aggregator's file: the header that holds each
/// value. A column left empty is not in the file.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_import_presets")]
pub struct ImportPreset {
    /// The primary key.
    pub id: i64,
    /// The aggregator.
    pub aggregator: String,
    /// The preset's name, e.g. `Aggregator A weekly CSV`.
    pub name: String,
    /// The field separator, one character.
    pub delimiter: String,
    /// The header of the aggregator's order number.
    pub order_column: String,
    /// The header of the gross.
    pub gross_column: String,
    /// The header of the commission.
    pub commission_column: String,
    /// The header of the promotion share.
    pub promo_column: String,
    /// The header of the refund.
    pub refund_column: String,
    /// The header of the net.
    pub net_column: String,
    /// The header of the refund reason.
    pub reason_column: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ImportPreset {
    /// The preset of `aggregator`, if it has one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of(db: &Db, aggregator: &str) -> Result<Option<ImportPreset>> {
        ImportPreset::query()
            .where_(ImportPreset::AGGREGATOR.eq(aggregator_key(aggregator)))
            .first(db)
            .await
    }

    /// The delimiter as a character (a comma when the column is blank).
    pub fn delimiter_char(&self) -> char {
        match self.delimiter.trim() {
            "\\t" | "tab" => '\t',
            other => other.chars().next().unwrap_or(','),
        }
    }
}

/// An order the statement named and the kitchen never recorded, entered
/// afterwards as a late entry with the aggregator's number and the time it
/// really happened (`PLT.ACL.10`).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_late_orders")]
pub struct LateOrder {
    /// The primary key.
    pub id: i64,
    /// The aggregator.
    pub aggregator: String,
    /// The aggregator's order number as entered.
    pub order_number: String,
    /// The number trimmed and lower-cased.
    pub order_key: String,
    /// The business time: when the order really happened.
    pub occurred_at: Option<DateTime>,
    /// When it was entered.
    pub entered_at: Option<DateTime>,
    /// The late-entry flag.
    pub late: bool,
    /// Why it is late.
    pub late_reason: Option<String>,
    /// The kernel's late entry, when the entry was late.
    pub late_entry_id: Option<i64>,
    /// What the order was worth with VAT, in fils.
    pub gross: i64,
    /// The cashier who entered it.
    pub entered_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl LateOrder {
    /// The late order of `aggregator` with `number`, if one was entered.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn find_number(db: &Db, aggregator: &str, number: &str) -> Result<Option<LateOrder>> {
        LateOrder::query()
            .where_(LateOrder::AGGREGATOR.eq(aggregator_key(aggregator)))
            .where_(LateOrder::ORDER_KEY.eq(reference_key(number)))
            .first(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_is_compared_trimmed_and_without_case() {
        assert_eq!(
            reference_key(" tb-2026-10-W2 "),
            reference_key("TB-2026-10-w2")
        );
        assert_eq!(aggregator_key(" Aggregator_A "), "aggregator_a");
    }

    #[test]
    fn a_tab_preset_reads_its_delimiter() {
        let preset = ImportPreset {
            delimiter: "tab".to_owned(),
            ..ImportPreset::default()
        };
        assert_eq!(preset.delimiter_char(), '\t');
        assert_eq!(ImportPreset::default().delimiter_char(), ',');
    }
}
