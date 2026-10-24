//! Where a part comes from and how long the law gives to supply it (08 §2
//! consumer protection, §9, `AUT.RUL.16`, `docs/OPEN-ITEMS.md` L-35).
//!
//! Spare parts must be supplied within 7, 14 or 30 days depending on demand and
//! source. Which period applies to which source is set at onboarding and is
//! `⚠ open: lawyer (L-35)`; until it is set a source has its default
//! ([`default_days`]). The legal supply date of a wait for parts is its start
//! plus the shortest period among the sources of the parts requested, so that
//! the strictest part counts. Calendar days, Asia/Dubai.

use renox::chrono::{Duration, NaiveDate};
use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::db::refused;
use crate::permissions;
use crate::texts::english;

/// The part is in the workshop's own stock.
pub const STOCK: &str = "stock";
/// The part comes from another branch.
pub const OTHER_BRANCH: &str = "other_branch";
/// The part comes from a local supplier.
pub const LOCAL_SUPPLIER: &str = "local_supplier";
/// The part is imported.
pub const IMPORT: &str = "import";
/// The sources of a part.
pub const SOURCES: [&str; 4] = [STOCK, OTHER_BRANCH, LOCAL_SUPPLIER, IMPORT];
/// The periods the law allows, in days.
pub const PERIODS: [i64; 3] = [7, 14, 30];

/// The period of a source until the onboarding sets it, in days.
pub fn default_days(source: &str) -> i64 {
    match source {
        LOCAL_SUPPLIER => 14,
        IMPORT => 30,
        _ => 7,
    }
}

/// The supply period of a source, set at onboarding.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_supply_periods", unique(source))]
pub struct SupplyPeriod {
    /// The primary key.
    pub id: i64,
    /// The source.
    pub source: String,
    /// The days: 7, 14 or 30.
    pub days: i64,
    /// The user who set it.
    pub updated_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The source as stored: a blank one is the stock.
pub fn clean_source(source: &str) -> String {
    let source = source.trim();
    if source.is_empty() {
        STOCK.to_owned()
    } else {
        source.to_owned()
    }
}

/// The supply period of `source` in days: the one set at onboarding, else the
/// default.
///
/// # Errors
///
/// The database's errors.
pub async fn days_of(db: &Db, source: &str) -> Result<i64> {
    let source = clean_source(source);
    let found = SupplyPeriod::query()
        .where_(SupplyPeriod::SOURCE.eq(source.as_str()))
        .first(db)
        .await?;
    Ok(found.map_or_else(|| default_days(&source), |row| row.days))
}

/// The legal supply date of a wait that starts on `start` for parts from
/// `sources`: the start plus the shortest of their periods; and the source that
/// gave it.
///
/// # Errors
///
/// The database's errors.
pub async fn legal_date(
    db: &Db,
    start: NaiveDate,
    sources: &[String],
) -> Result<(NaiveDate, String)> {
    let mut best: Option<(i64, String)> = None;
    for source in sources {
        let source = clean_source(source);
        let days = days_of(db, &source).await?;
        if best.as_ref().is_none_or(|(shortest, _)| days < *shortest) {
            best = Some((days, source));
        }
    }
    let (days, source) = best.unwrap_or_else(|| (default_days(STOCK), STOCK.to_owned()));
    Ok((start + Duration::days(days), source))
}

/// Sets the supply period of `source` at onboarding.
///
/// # Errors
///
/// A 403 without `job_cards.approve`; a 422 for a source that is not one of
/// the four or days that are not 7, 14 or 30.
pub async fn set_period(
    state: &AppState,
    by: &AuthUser,
    source: &str,
    days: i64,
) -> Result<SupplyPeriod> {
    if !by.has_permission(permissions::JOB_CARDS_APPROVE) {
        return Err(Error::Forbidden);
    }
    if !SOURCES.contains(&source) {
        return Err(refused(english("auto.supply.problem.source", &[])));
    }
    if !PERIODS.contains(&days) {
        return Err(refused(english("auto.supply.problem.days", &[])));
    }
    let existing = SupplyPeriod::query()
        .where_(SupplyPeriod::SOURCE.eq(source))
        .first(&state.db)
        .await?;
    match existing {
        Some(row) => {
            SupplyPeriod::query()
                .where_(SupplyPeriod::ID.eq(row.id))
                .update(
                    &state.db,
                    &[
                        ("days", &days),
                        ("updated_by", &Some(by.id)),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            SupplyPeriod::find_or_404(&state.db, row.id).await
        }
        None => {
            let mut row = SupplyPeriod {
                source: source.to_owned(),
                days,
                updated_by: Some(by.id),
                ..SupplyPeriod::default()
            };
            row.save(&state.db).await?;
            Ok(row)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_has_a_default_period_until_it_is_set() {
        assert_eq!(default_days(STOCK), 7);
        assert_eq!(default_days(OTHER_BRANCH), 7);
        assert_eq!(default_days(LOCAL_SUPPLIER), 14);
        assert_eq!(default_days(IMPORT), 30);
        assert_eq!(clean_source("  "), "stock");
        assert_eq!(clean_source("import"), "import");
    }
}
