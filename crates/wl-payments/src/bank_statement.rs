//! The bank statement import (wl-payments §3, issue #118): one uploaded
//! statement file for one bank account, with its period and its closing
//! balance, and the module's [`BankStatements`] for the cash-flow forecast
//! (`FIN.CFL.01`).
//!
//! Only the record is built here: the accountant enters the closing balance
//! at upload when the file carries none. Parsing the lines, the match
//! suggestions and the `parse`, `complete` and `revert` transitions come with
//! the bank reconciliation feature; until then an import stays `uploaded` and
//! no balance is read from it. The closing balance is as of the last day of
//! the period.
//!
//! The latest balance of an account is that of the `completed` import with a
//! closing balance whose period ends last (not the one imported last); on the
//! same period end the one completed last wins; `reverted` and `failed`
//! imports are never used.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::bank_statements::{BankBalance, BankStatementFuture, BankStatements};

/// The lifecycle state of an import (wl-payments §4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ImportStatus {
    /// File received.
    #[default]
    Uploaded,
    /// Lines parsed; matches suggested and being confirmed.
    InReview,
    /// Every line matched, ignored or turned into a task.
    Completed,
    /// File not recognised.
    Failed,
    /// Import undone.
    Reverted,
}

/// A bank statement import. Money is `i64` fils of [`BankStatementImport::currency`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_bank_statement_imports",
    index(legal_entity_id, bank_account, status)
)]
pub struct BankStatementImport {
    /// The primary key.
    pub id: i64,
    /// The legal entity whose account it is.
    pub legal_entity_id: i64,
    /// The bank account label, as on the payments.
    pub bank_account: String,
    /// The name of the uploaded file.
    pub file_name: String,
    /// `csv`, `excel` or `mt940`.
    pub format: String,
    /// The ISO currency code.
    pub currency: String,
    /// The first day of the period.
    pub period_start: NaiveDate,
    /// The last day of the period.
    pub period_end: NaiveDate,
    /// The balance at the end of the period, in fils; may be below 0.
    pub closing_balance: Option<i64>,
    /// How many lines were imported.
    pub line_count: i32,
    /// The user who uploaded it.
    pub imported_by: Option<i64>,
    /// The lifecycle state.
    pub status: ImportStatus,
    /// When it was completed.
    pub completed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The file formats of an import.
pub const FORMATS: [&str; 3] = ["csv", "excel", "mt940"];

/// What the upload form carries.
#[derive(Debug, Clone)]
pub struct Upload {
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The bank account label.
    pub bank_account: String,
    /// The file name.
    pub file_name: String,
    /// `csv`, `excel` or `mt940`.
    pub format: String,
    /// The first day of the period.
    pub period_start: NaiveDate,
    /// The last day of the period.
    pub period_end: NaiveDate,
    /// The closing balance in fils, when given.
    pub closing_balance: Option<i64>,
}

/// Why `upload` cannot be recorded, if it cannot.
pub fn upload_problem(upload: &Upload) -> Option<&'static str> {
    if upload.bank_account.trim().is_empty() {
        return Some("The bank account is required.");
    }
    if upload.file_name.trim().is_empty() {
        return Some("The file name is required.");
    }
    if !FORMATS.contains(&upload.format.as_str()) {
        return Some("The format must be CSV, Excel or MT940.");
    }
    if upload.period_end < upload.period_start {
        return Some("The period ends before it starts.");
    }
    None
}

/// `text` ("1500", "-1,500.5", "0") as fils; negative and zero allowed.
/// `None` for anything else.
pub fn parse_balance(text: &str) -> Option<i64> {
    let text = text.trim().replace(',', "");
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.as_str()),
    };
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if !digits(whole) || !digits(fraction) || fraction.len() > 2 {
        return None;
    }
    let whole: i64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let fraction: i64 = format!("{fraction:0<2}").parse().ok()?;
    let fils = whole.checked_mul(100)?.checked_add(fraction)?;
    Some(if negative { -fils } else { fils })
}

/// Records `upload` as an `uploaded` import by user `by`.
///
/// # Errors
///
/// 422 for a problem of [`upload_problem`] or an inactive legal entity.
pub async fn record(db: &Db, by: Option<i64>, upload: &Upload) -> Result<BankStatementImport> {
    if let Some(problem) = upload_problem(upload) {
        return Err(refused(problem));
    }
    let entity = LegalEntity::find(db, upload.legal_entity_id).await?;
    if !entity.as_ref().is_some_and(LegalEntity::is_active) {
        return Err(refused("The legal entity is not active."));
    }
    let mut import = BankStatementImport {
        legal_entity_id: upload.legal_entity_id,
        bank_account: upload.bank_account.trim().to_owned(),
        file_name: upload.file_name.trim().to_owned(),
        format: upload.format.clone(),
        currency: wl_kernel::payments::DEFAULT_CURRENCY.to_owned(),
        period_start: upload.period_start,
        period_end: upload.period_end,
        closing_balance: upload.closing_balance,
        imported_by: by,
        status: ImportStatus::Uploaded,
        ..Default::default()
    };
    import.insert(db).await?;
    Ok(import)
}

/// A 422 with `message`.
fn refused(message: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

/// The latest closing balance of each bank account of `legal_entity_id`.
///
/// # Errors
///
/// A database error.
pub async fn latest_balances(db: &Db, legal_entity_id: i64) -> Result<Vec<BankBalance>> {
    let imports = BankStatementImport::query()
        .where_(BankStatementImport::LEGAL_ENTITY_ID.eq(legal_entity_id))
        .where_(BankStatementImport::STATUS.eq(ImportStatus::Completed))
        .where_(BankStatementImport::CLOSING_BALANCE.is_not_null())
        .get(db)
        .await?;
    Ok(latest_of(&imports))
}

/// The imports that win per account: the latest period end, then the latest
/// completion, then the highest id.
pub(crate) fn latest_of(imports: &[BankStatementImport]) -> Vec<BankBalance> {
    let mut best: BTreeMap<&str, &BankStatementImport> = BTreeMap::new();
    for import in imports {
        if import.status != ImportStatus::Completed || import.closing_balance.is_none() {
            continue;
        }
        let key = |i: &BankStatementImport| (i.period_end, i.completed_at, i.id);
        match best.get(import.bank_account.as_str()) {
            Some(current) if key(current) >= key(import) => {}
            _ => {
                best.insert(import.bank_account.as_str(), import);
            }
        }
    }
    best.into_values()
        .map(|import| BankBalance {
            legal_entity_id: import.legal_entity_id,
            bank_account: import.bank_account.clone(),
            amount: import.closing_balance.unwrap_or(0),
            currency: import.currency.clone(),
            as_of: import.period_end,
            import_id: import.id,
        })
        .collect()
}

/// Reads `wl_payments_bank_statement_imports` for the kernel contract.
#[derive(Debug, Clone, Copy, Default)]
pub struct Statements;

impl BankStatements for Statements {
    fn latest_closing_balances<'a>(
        &'a self,
        db: &'a Db,
        legal_entity_id: i64,
    ) -> BankStatementFuture<'a, Vec<BankBalance>> {
        Box::pin(latest_balances(db, legal_entity_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    fn done(id: i64, account: &str, end: NaiveDate, balance: Option<i64>) -> BankStatementImport {
        BankStatementImport {
            id,
            legal_entity_id: 1,
            bank_account: account.into(),
            currency: "AED".into(),
            period_start: end,
            period_end: end,
            closing_balance: balance,
            status: ImportStatus::Completed,
            ..Default::default()
        }
    }

    #[test]
    fn a_balance_may_be_zero_or_negative() {
        assert_eq!(parse_balance("1,500.5"), Some(150_050));
        assert_eq!(parse_balance("-20"), Some(-2_000));
        assert_eq!(parse_balance("0"), Some(0));
        assert_eq!(parse_balance("1.234"), None);
        assert_eq!(parse_balance("abc"), None);
        assert_eq!(parse_balance(""), None);
    }

    #[test]
    fn the_period_end_decides_not_the_import_order() {
        let january = done(5, "ENBD", day(1, 31), Some(900));
        let december = done(6, "ENBD", NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(), Some(100));
        let latest = latest_of(&[december, january]);
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].amount, 900);
        assert_eq!(latest[0].as_of, day(1, 31));
    }

    #[test]
    fn an_import_without_a_balance_is_skipped() {
        let with = done(1, "ENBD", day(1, 31), Some(900));
        let without = done(2, "ENBD", day(2, 28), None);
        let latest = latest_of(&[with, without]);
        assert_eq!(latest[0].import_id, 1);
    }

    #[test]
    fn a_reverted_or_unfinished_import_is_never_used() {
        let mut reverted = done(1, "ENBD", day(2, 28), Some(5));
        reverted.status = ImportStatus::Reverted;
        let mut uploaded = done(2, "ENBD", day(3, 31), Some(6));
        uploaded.status = ImportStatus::Uploaded;
        assert!(latest_of(&[reverted, uploaded]).is_empty());
    }

    #[test]
    fn on_the_same_period_end_the_one_completed_last_wins() {
        let mut first = done(1, "ENBD", day(1, 31), Some(100));
        let mut second = done(2, "ENBD", day(1, 31), Some(200));
        first.completed_at = Some(renox::db::now());
        second.completed_at = first.completed_at.map(|t| t + renox::chrono::Duration::seconds(5));
        assert_eq!(latest_of(&[second.clone(), first.clone()])[0].amount, 200);
        // Completed the other way round: the first wins.
        first.completed_at = second.completed_at.map(|t| t + renox::chrono::Duration::seconds(5));
        assert_eq!(latest_of(&[second, first])[0].amount, 100);
    }

    #[test]
    fn accounts_are_kept_apart() {
        let a = done(1, "ENBD", day(1, 31), Some(100));
        let b = done(2, "MASHREQ", day(1, 15), Some(300));
        let latest = latest_of(&[a, b]);
        assert_eq!(latest.len(), 2);
        assert_eq!(latest[0].bank_account, "ENBD");
        assert_eq!(latest[1].bank_account, "MASHREQ");
    }

    #[test]
    fn an_upload_is_checked() {
        let mut upload = Upload {
            legal_entity_id: 1,
            bank_account: "ENBD".into(),
            file_name: "jan.csv".into(),
            format: "csv".into(),
            period_start: day(1, 1),
            period_end: day(1, 31),
            closing_balance: None,
        };
        assert_eq!(upload_problem(&upload), None);
        upload.period_end = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
        assert!(upload_problem(&upload).is_some());
        upload.period_end = day(1, 31);
        upload.format = "pdf".into();
        assert!(upload_problem(&upload).is_some());
    }
}
