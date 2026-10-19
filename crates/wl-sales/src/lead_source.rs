//! Lead sources (wl-sales §3, §7, #138 A5): where a lead or a campaign's
//! customers come from. The nine §3 defaults are stored by [`crate::seed::install`]
//! (the `sales:install` command and the seeder); a manager adds, renames and
//! deactivates sources afterwards. A source that something names is
//! deactivated, never deleted.

use renox::db::{DateTime, Executor};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// The nine seeded sources as (code, English name).
pub const DEFAULTS: [(&str, &str); 9] = [
    ("referral", "Referral"),
    ("walk_in", "Walk-in"),
    ("website", "Website"),
    ("social", "Social media"),
    ("paid_ads", "Paid ads"),
    ("aggregator", "Aggregator"),
    ("event", "Event"),
    ("partner", "Partner"),
    ("other", "Other"),
];

/// What names a lead source, as (table, column) pairs: a source with a row
/// there is in use. Campaigns, leads and party
/// acquisitions are listed.
const USED_BY: &[(&str, &str)] = &[
    ("wl_sales_campaigns", "lead_source_id"),
    ("wl_sales_leads", "source_id"),
    ("wl_sales_party_acquisitions", "source_id"),
];

/// A lead source, e.g. `referral`.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_sales_lead_sources", unique(code))]
pub struct LeadSource {
    /// The primary key.
    pub id: i64,
    /// The code, e.g. `walk_in`: lower case letters, digits and `_`.
    pub code: String,
    /// The English name.
    pub name_en: String,
    /// The Arabic name.
    pub name_ar: String,
    /// Whether it can be chosen for new campaigns and leads.
    pub active: bool,
    /// Whether the installer stored it.
    pub seeded: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Why a source could not be added or deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeadSourceRefused {
    /// The code is not 2 to 30 characters of `a-z`, `0-9` and `_`.
    BadCode,
    /// A source has that code.
    CodeTaken,
    /// The name is blank.
    NameRequired,
    /// Something names the source; deactivate it instead.
    InUse,
}

impl LeadSource {
    /// The source `code`, if any.
    pub async fn by_code<'c>(db: impl Executor<'c>, code: &str) -> Result<Option<LeadSource>> {
        LeadSource::query()
            .where_(LeadSource::CODE.eq(code))
            .first(db)
            .await
    }

    /// Every source, by code.
    pub async fn ordered(db: &Db) -> Result<Vec<LeadSource>> {
        LeadSource::query().order_by(LeadSource::CODE).get(db).await
    }

    /// The name in `locale`: the Arabic one for `ar` when it is set.
    pub fn name(&self, locale: &str) -> &str {
        if locale == "ar" && !self.name_ar.trim().is_empty() {
            &self.name_ar
        } else {
            &self.name_en
        }
    }

    /// Normalises a typed code: trimmed and lower case.
    pub fn normalize_code(code: &str) -> String {
        code.trim().to_lowercase()
    }

    /// Whether `code` (already normalised) is a valid code.
    pub fn is_valid_code(code: &str) -> bool {
        (2..=30).contains(&code.len())
            && code
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    }

    /// Adds a source. The code is normalised; a bad or taken code and a blank
    /// name are refused.
    pub async fn add(
        db: &Db,
        code: &str,
        name_en: &str,
        name_ar: &str,
    ) -> Result<std::result::Result<LeadSource, LeadSourceRefused>> {
        let code = Self::normalize_code(code);
        if !Self::is_valid_code(&code) {
            return Ok(Err(LeadSourceRefused::BadCode));
        }
        if name_en.trim().is_empty() {
            return Ok(Err(LeadSourceRefused::NameRequired));
        }
        if Self::by_code(db, &code).await?.is_some() {
            return Ok(Err(LeadSourceRefused::CodeTaken));
        }
        let source = LeadSource::create(
            db,
            LeadSource {
                code,
                name_en: name_en.trim().to_owned(),
                name_ar: name_ar.trim().to_owned(),
                active: true,
                ..LeadSource::default()
            },
        )
        .await?;
        Ok(Ok(source))
    }

    /// Whether anything names this source (see `USED_BY`).
    pub async fn in_use(&self, db: &Db) -> Result<bool> {
        for (table, column) in USED_BY {
            let found: Option<i64> =
                renox::db::sql(format!("SELECT 1 FROM {table} WHERE {column} = ? LIMIT 1"))
                    .bind(self.id)
                    .scalar_optional(db)
                    .await?;
            if found.is_some() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Deletes the source unless it is in use ([`LeadSourceRefused::InUse`]).
    pub async fn delete_unused(
        mut self,
        db: &Db,
    ) -> Result<std::result::Result<(), LeadSourceRefused>> {
        if self.in_use(db).await? {
            return Ok(Err(LeadSourceRefused::InUse));
        }
        self.delete(db).await?;
        Ok(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_nine_section_3_codes_and_valid() {
        let codes: Vec<_> = DEFAULTS.iter().map(|(c, _)| *c).collect();
        assert_eq!(
            codes,
            [
                "referral",
                "walk_in",
                "website",
                "social",
                "paid_ads",
                "aggregator",
                "event",
                "partner",
                "other"
            ]
        );
        assert!(codes.iter().all(|c| LeadSource::is_valid_code(c)));
    }

    #[test]
    fn codes_are_lower_case_letters_digits_and_underscores() {
        assert_eq!(LeadSource::normalize_code("  Walk_In "), "walk_in");
        assert!(LeadSource::is_valid_code("tv_2026"));
        assert!(!LeadSource::is_valid_code("x"));
        assert!(!LeadSource::is_valid_code("walk in"));
        assert!(!LeadSource::is_valid_code("walk-in"));
        assert!(!LeadSource::is_valid_code(&"a".repeat(31)));
    }

    #[test]
    fn the_arabic_name_is_used_for_ar_when_set() {
        let mut source = LeadSource {
            name_en: "Referral".into(),
            ..LeadSource::default()
        };
        assert_eq!(source.name("ar"), "Referral");
        source.name_ar = "إحالة".into();
        assert_eq!(source.name("ar"), "إحالة");
        assert_eq!(source.name("en"), "Referral");
    }
}
