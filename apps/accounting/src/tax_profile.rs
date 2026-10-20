//! The client's tax profile, versioned by effective date (04 §6 *Tax
//! profile*, `ACC.TAX.01`).
//!
//! Each change is a new row with its effective date; the profile in force on
//! a day is the latest row effective on or before it ([`TaxProfile::current`]).
//! Engagement `activate` needs it complete ([`TaxProfile::missing`]), and the
//! compliance calendar is generated from it ([`crate::calendar`]). The change
//! control of `ACC.ACL.13` (another manager approves, one pending change per
//! profile) comes with BL-ACC-14; until then [`record`] stores a version as
//! approved by the user who records it.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Executor, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyId, Subject};

use crate::texts::english;

/// The subject type of a tax profile version.
pub const TAX_PROFILE_SUBJECT_TYPE: &str = "accounting.tax_profile";

/// How often VAT returns are due.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VatPeriod {
    /// Not VAT registered.
    #[default]
    None,
    /// Every month.
    Monthly,
    /// Every quarter, on the client's stagger.
    Quarterly,
}

impl VatPeriod {
    /// The months in one period; 0 for none.
    pub fn months(self) -> u32 {
        match self {
            VatPeriod::None => 0,
            VatPeriod::Monthly => 1,
            VatPeriod::Quarterly => 3,
        }
    }
}

/// One version of a client's tax profile.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "accounting_tax_profiles", index(client_party_id))]
pub struct TaxProfile {
    /// The primary key.
    pub id: i64,
    /// The client company, a party.
    pub client_party_id: i64,
    /// The VAT tax registration number (15 digits).
    pub trn: Option<String>,
    /// Whether the client is registered for VAT.
    pub vat_registered: bool,
    /// How often its VAT returns are due.
    pub vat_period: VatPeriod,
    /// The last day of its first VAT period: the stagger.
    pub vat_first_period_end: Option<NaiveDate>,
    /// The last day of its financial year (any year; the month and day
    /// count).
    pub financial_year_end: Option<NaiveDate>,
    /// The corporate tax registration number.
    pub ct_registration_no: Option<String>,
    /// Whether it elected Small Business Relief.
    pub small_business_relief: bool,
    /// Whether it is a Qualifying Free Zone Person.
    pub qfzp: bool,
    /// The day this version takes effect.
    pub effective_date: NaiveDate,
    /// Who approved this version.
    pub approved_by: Option<i64>,
    /// When it was approved.
    pub approved_at: Option<DateTime>,
    /// Who recorded it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// `text` holds something.
fn given(text: &Option<String>) -> bool {
    text.as_deref().is_some_and(|t| !t.trim().is_empty())
}

impl TaxProfile {
    /// The version as a kernel subject: `accounting.tax_profile:5`.
    pub fn subject(&self) -> Subject {
        Subject::new(TAX_PROFILE_SUBJECT_TYPE, self.id)
    }

    /// The client company.
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The version of `client` in force on `on`: the latest effective on or
    /// before it.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn current<'c>(
        db: impl Executor<'c>,
        client: PartyId,
        on: NaiveDate,
    ) -> Result<Option<TaxProfile>> {
        Ok(TaxProfile::query()
            .where_(TaxProfile::CLIENT_PARTY_ID.eq(client.0))
            .where_(TaxProfile::EFFECTIVE_DATE.lte(on))
            .order_by_desc(TaxProfile::EFFECTIVE_DATE)
            .order_by_desc(TaxProfile::ID)
            .limit(1)
            .get(db)
            .await?
            .into_iter()
            .next())
    }

    /// What is missing for the profile to be complete, one English line
    /// each: the financial year end and the corporate tax registration
    /// always; the TRN, the VAT period and its first period end when VAT
    /// registered. Empty when complete.
    ///
    /// Decision: every UAE company registers for corporate tax, so its
    /// registration number is required; Small Business Relief and QFZP are
    /// yes/no and always given.
    pub fn missing(&self) -> Vec<String> {
        let mut missing = Vec::new();
        if self.financial_year_end.is_none() {
            missing.push(english("accounting.tax_profile.financial_year_end", &[]));
        }
        if !given(&self.ct_registration_no) {
            missing.push(english("accounting.tax_profile.ct_registration_no", &[]));
        }
        if self.vat_registered {
            let trn_ok = self
                .trn
                .as_deref()
                .is_some_and(|t| t.len() == 15 && t.chars().all(|c| c.is_ascii_digit()));
            if !trn_ok {
                missing.push(english("accounting.tax_profile.trn", &[]));
            }
            if self.vat_period == VatPeriod::None || self.vat_first_period_end.is_none() {
                missing.push(english("accounting.tax_profile.vat_period", &[]));
            }
        }
        missing
    }

    /// Whether nothing is missing.
    pub fn is_complete(&self) -> bool {
        self.missing().is_empty()
    }
}

/// `by` records `profile` as a new version of its client's tax profile.
///
/// Decision: until BL-ACC-14 adds the approval by another manager
/// (`ACC.ACL.13`), a recorded version counts as approved by `by`.
///
/// # Errors
///
/// The database's errors.
pub async fn record(db: &Db, by: Option<i64>, mut profile: TaxProfile) -> Result<TaxProfile> {
    profile.id = 0;
    profile.created_by = by;
    profile.approved_by = by;
    profile.approved_at = Some(now());
    profile.insert(db).await?;
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn a_profile_is_complete_with_its_year_end_and_registrations() {
        let mut profile = TaxProfile {
            financial_year_end: Some(day("2026-12-31")),
            ct_registration_no: Some("CT-1001".into()),
            ..TaxProfile::default()
        };
        assert!(profile.is_complete());
        profile.vat_registered = true;
        assert_eq!(profile.missing().len(), 2);
        profile.trn = Some("100000000000003".into());
        profile.vat_period = VatPeriod::Quarterly;
        profile.vat_first_period_end = Some(day("2026-12-31"));
        assert!(profile.is_complete());
        profile.financial_year_end = None;
        assert_eq!(
            profile.missing(),
            vec![english("accounting.tax_profile.financial_year_end", &[])]
        );
    }

    #[test]
    fn vat_periods_count_months() {
        assert_eq!(VatPeriod::None.months(), 0);
        assert_eq!(VatPeriod::Monthly.months(), 1);
        assert_eq!(VatPeriod::Quarterly.months(), 3);
        assert_eq!(VatPeriod::default().as_str(), "none");
    }
}
