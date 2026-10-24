//! One bank transaction of an import (wl-payments §3 *Statement line*): its
//! date, signed amount, description and reference, and a line status
//! (`unmatched`, `suggested`, `matched`, `ignored`). Created by `parse`
//! ([`crate::bank_statement_machine`]), removed by `revert`.

use renox::chrono::NaiveDate;
use renox::db::Json;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// Where a line stands in the reconciliation.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineStatus {
    /// Nothing is decided for it.
    #[default]
    Unmatched,
    /// A match waits for its confirmation: for a line marked as an owner
    /// drawing, the approval of the drawing.
    Suggested,
    /// Matched to a payment.
    Matched,
    /// Left out, with a reason (bank charges, an internal transfer).
    Ignored,
}

/// A bank transaction. Money is `i64` fils of [`BankStatementLine::currency`];
/// above 0 is money in, below 0 money out.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_bank_statement_lines",
    index(import_id, status),
    index(drawing_request_id),
    index(legal_entity_id, bank_account, line_date)
)]
pub struct BankStatementLine {
    /// The primary key.
    pub id: i64,
    /// The import it belongs to.
    pub import_id: i64,
    /// The legal entity whose account it is.
    pub legal_entity_id: i64,
    /// The bank account label.
    pub bank_account: String,
    /// The ISO currency code.
    pub currency: String,
    /// The day of the transaction.
    pub line_date: NaiveDate,
    /// The amount in fils, signed.
    pub amount: i64,
    /// The bank's description.
    pub description: String,
    /// The reference the bank prints.
    pub reference: String,
    /// The other side, when the file names it.
    pub counterparty: String,
    /// Where it stands.
    pub status: LineStatus,
    /// What a matched line was matched to: `payment` (a payment, recorded
    /// or already there), `invoices` (confirmed, its payment is recorded at
    /// `complete`), `cheque` (a deposited cheque, cleared at `complete`).
    pub match_kind: Option<String>,
    /// The id of what it was matched to: the payment, or the cheque.
    pub match_id: Option<i64>,
    /// Why an ignored line is left out.
    pub ignore_reason: Option<String>,
    /// The owner drawing request the line was marked with.
    pub drawing_request_id: Option<i64>,
    /// Why the line was put back to `unmatched` (a rejected drawing).
    pub reason: Option<String>,
    /// The user who marked it as a drawing.
    pub marked_by: Option<i64>,
    /// What the match engine suggests (BL-PAY-29): `invoices`, `payment`
    /// (a payment already recorded) or `cheque` (a deposited cheque).
    pub suggestion_kind: Option<String>,
    /// The party of the suggestion.
    pub suggestion_party_id: Option<i64>,
    /// The suggested targets as `[id, amount]` pairs.
    pub suggestion_targets: Option<Json<Vec<(i64, i64)>>>,
    /// Why: `reference`, `payer_name`, `oldest_invoices`, `already_recorded`,
    /// `cheque`.
    pub suggestion_reason: Option<String>,
    /// How sure the suggestion is, 0 to 100.
    pub suggestion_score: Option<i64>,
    /// The party of the confirmed match.
    pub match_party_id: Option<i64>,
    /// The confirmed targets as `[id, amount]` pairs (invoices to allocate to
    /// at `complete`, or the payment or cheque the line is).
    pub match_targets: Option<Json<Vec<(i64, i64)>>>,
    /// The user who confirmed or made the match.
    pub confirmed_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BankStatementLine {
    /// The lines of import `import_id`, oldest first.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn of_import(db: &Db, import_id: i64) -> Result<Vec<BankStatementLine>> {
        BankStatementLine::query()
            .where_(BankStatementLine::IMPORT_ID.eq(import_id))
            .order_by(BankStatementLine::LINE_DATE)
            .order_by(BankStatementLine::ID)
            .get(db)
            .await
    }
}
