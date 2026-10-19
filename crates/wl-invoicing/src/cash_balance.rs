//! The opening cash balance (wl-invoicing §3, §5.14 step 2, `FIN.CFL.01`): a
//! balance of a bank account entered by hand, for the days no statement
//! balance exists yet.
//!
//! A balance is as of the end of its day, which is today or earlier. It is
//! never edited: a wrong one is voided with a reason (`cashflow.manage`) or
//! replaced by a newer one. A voided balance stays in the history and is
//! never used; the forecast falls back to the next latest one.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::documents::today;

/// A balance entered by hand. Money is `i64` fils of AED.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_opening_cash_balances",
    index(legal_entity_id, bank_account, as_of)
)]
pub struct OpeningCashBalance {
    /// The primary key.
    pub id: i64,
    /// The legal entity whose account it is.
    pub legal_entity_id: i64,
    /// The bank account label.
    pub bank_account: String,
    /// The balance, in AED fils; may be below 0.
    pub amount: i64,
    /// The day it is as of: the end of this day.
    pub as_of: NaiveDate,
    /// A note, if any.
    pub note: Option<String>,
    /// The user who entered it.
    pub entered_by: Option<i64>,
    /// The user who voided it.
    pub voided_by: Option<i64>,
    /// When it was voided.
    pub voided_at: Option<DateTime>,
    /// Why it was voided.
    pub void_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl OpeningCashBalance {
    /// Whether it was voided.
    pub fn is_voided(&self) -> bool {
        self.voided_at.is_some()
    }
}

/// What the form carries.
#[derive(Debug, Clone)]
pub struct EnterBalance {
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The bank account label.
    pub bank_account: String,
    /// The balance in AED fils.
    pub amount: i64,
    /// The day it is as of.
    pub as_of: NaiveDate,
    /// A note, if any.
    pub note: Option<String>,
}

/// A 422 with `message`.
fn refused(message: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

/// Why `balance` cannot be entered on `today`, if it cannot.
pub fn enter_problem(balance: &EnterBalance, today: NaiveDate) -> Option<&'static str> {
    if balance.bank_account.trim().is_empty() {
        return Some("The bank account is required.");
    }
    if balance.as_of > today {
        return Some("A balance cannot be dated in the future.");
    }
    None
}

/// `text` ("60000", "-1,500.5", "0") as AED fils; negative and zero allowed.
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

/// Enters `balance` by user `by`.
///
/// # Errors
///
/// 422 for a blank account, a date in the future or an inactive legal entity.
pub async fn enter(db: &Db, by: Option<i64>, balance: &EnterBalance) -> Result<OpeningCashBalance> {
    if let Some(problem) = enter_problem(balance, today()) {
        return Err(refused(problem));
    }
    let entity = LegalEntity::find(db, balance.legal_entity_id).await?;
    if !entity.as_ref().is_some_and(LegalEntity::is_active) {
        return Err(refused("The legal entity is not active."));
    }
    let mut row = OpeningCashBalance {
        legal_entity_id: balance.legal_entity_id,
        bank_account: balance.bank_account.trim().to_owned(),
        amount: balance.amount,
        as_of: balance.as_of,
        note: balance
            .note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned),
        entered_by: by,
        ..Default::default()
    };
    row.insert(db).await?;
    Ok(row)
}

/// Voids balance `id` by user `by` with `reason`.
///
/// # Errors
///
/// 404 for an unknown balance; 422 for a blank reason or a balance already
/// voided.
pub async fn void(db: &Db, id: i64, by: Option<i64>, reason: &str) -> Result<OpeningCashBalance> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refused("The reason for voiding is required."));
    }
    let mut row = OpeningCashBalance::find_or_404(db, id).await?;
    if row.is_voided() {
        return Err(refused("The balance is already voided."));
    }
    row.voided_by = by;
    row.voided_at = Some(renox::db::now());
    row.void_reason = Some(reason.to_owned());
    row.update(db).await?;
    Ok(row)
}

/// The latest balance of each bank account of `legal_entity_id` that is not
/// voided, by account: the latest day, and of two on the same day the one
/// entered last.
///
/// # Errors
///
/// A database error.
pub async fn latest(db: &Db, legal_entity_id: i64) -> Result<BTreeMap<String, OpeningCashBalance>> {
    let rows = OpeningCashBalance::query()
        .where_(OpeningCashBalance::LEGAL_ENTITY_ID.eq(legal_entity_id))
        .where_(OpeningCashBalance::VOIDED_AT.is_null())
        .get(db)
        .await?;
    Ok(latest_of(rows))
}

/// The winners of `rows` per account.
pub(crate) fn latest_of(rows: Vec<OpeningCashBalance>) -> BTreeMap<String, OpeningCashBalance> {
    let mut best: BTreeMap<String, OpeningCashBalance> = BTreeMap::new();
    for row in rows {
        if row.is_voided() {
            continue;
        }
        let wins = best
            .get(&row.bank_account)
            .is_none_or(|current| (current.as_of, current.id) < (row.as_of, row.id));
        if wins {
            best.insert(row.bank_account.clone(), row);
        }
    }
    best
}

/// Every balance of `legal_entity_id`, newest day first, voided ones too: the
/// history.
///
/// # Errors
///
/// A database error.
pub async fn history(db: &Db, legal_entity_id: i64) -> Result<Vec<OpeningCashBalance>> {
    OpeningCashBalance::query()
        .where_(OpeningCashBalance::LEGAL_ENTITY_ID.eq(legal_entity_id))
        .order_by_desc(OpeningCashBalance::AS_OF)
        .order_by_desc(OpeningCashBalance::ID)
        .get(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    fn row(id: i64, account: &str, as_of: NaiveDate, amount: i64) -> OpeningCashBalance {
        OpeningCashBalance {
            id,
            legal_entity_id: 1,
            bank_account: account.into(),
            amount,
            as_of,
            ..Default::default()
        }
    }

    #[test]
    fn an_amount_may_be_zero_or_negative() {
        assert_eq!(parse_balance("60,000"), Some(6_000_000));
        assert_eq!(parse_balance("-1.5"), Some(-150));
        assert_eq!(parse_balance("0"), Some(0));
        assert_eq!(parse_balance("1.234"), None);
        assert_eq!(parse_balance("x"), None);
    }

    #[test]
    fn a_balance_is_not_dated_in_the_future_and_names_its_account() {
        let mut balance = EnterBalance {
            legal_entity_id: 1,
            bank_account: "ENBD-1".into(),
            amount: 6_000_000,
            as_of: day(1, 11),
            note: None,
        };
        assert_eq!(enter_problem(&balance, day(1, 13)), None);
        assert_eq!(enter_problem(&balance, day(1, 11)), None);
        assert!(enter_problem(&balance, day(1, 10)).is_some());
        balance.bank_account = "  ".into();
        assert!(enter_problem(&balance, day(1, 13)).is_some());
    }

    #[test]
    fn the_latest_day_wins_and_a_voided_balance_is_skipped() {
        let mut voided = row(3, "ENBD-1", day(1, 12), 1);
        voided.voided_at = Some(renox::db::now());
        let latest = latest_of(vec![
            row(1, "ENBD-1", day(1, 5), 100),
            row(2, "ENBD-1", day(1, 11), 200),
            voided,
            row(4, "MASHREQ-2", day(1, 2), 300),
        ]);
        assert_eq!(latest["ENBD-1"].amount, 200);
        assert_eq!(latest["MASHREQ-2"].amount, 300);
    }

    #[test]
    fn on_the_same_day_the_one_entered_last_wins() {
        let latest = latest_of(vec![
            row(1, "ENBD-1", day(1, 11), 100),
            row(2, "ENBD-1", day(1, 11), 150),
        ]);
        assert_eq!(latest["ENBD-1"].amount, 150);
    }
}
