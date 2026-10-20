//! The client's tax profile, versioned by effective date (04 §6 *Tax
//! profile*, `ACC.TAX.01`).
//!
//! Each change is a new row with its effective date; the profile in force on
//! a day is the latest row effective on or before it ([`TaxProfile::current`]).
//! Engagement `activate` needs it complete ([`TaxProfile::missing`]), and the
//! compliance calendar is generated from it ([`crate::calendar`]). The change
//! change control of `ACC.ACL.13` (another manager approves, one pending
//! change per profile) is in [`crate::tax_profile_change`]: [`record`] stores
//! only the first version of a client, every later version is the approval
//! of a change.

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
    NotRegistered,
    /// Every month.
    Monthly,
    /// Every quarter, on the client's stagger.
    Quarterly,
}

impl VatPeriod {
    /// The months in one period; 0 for none.
    pub fn months(self) -> u32 {
        match self {
            VatPeriod::NotRegistered => 0,
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
    /// The contact e-mail for tax matters (no approval to change,
    /// `ACC.ACL.13`).
    pub contact_email: Option<String>,
    /// Notes on the profile (no approval to change).
    pub notes: Option<String>,
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

    /// The newest version of `client`, whatever its effective date: the one
    /// a change is made against.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn latest<'c>(db: impl Executor<'c>, client: PartyId) -> Result<Option<TaxProfile>> {
        Ok(TaxProfile::query()
            .where_(TaxProfile::CLIENT_PARTY_ID.eq(client.0))
            .order_by_desc(TaxProfile::EFFECTIVE_DATE)
            .order_by_desc(TaxProfile::ID)
            .limit(1)
            .get(db)
            .await?
            .into_iter()
            .next())
    }

    /// The versions of `client`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn versions<'c>(db: impl Executor<'c>, client: PartyId) -> Result<Vec<TaxProfile>> {
        TaxProfile::query()
            .where_(TaxProfile::CLIENT_PARTY_ID.eq(client.0))
            .order_by(TaxProfile::EFFECTIVE_DATE)
            .order_by(TaxProfile::ID)
            .get(db)
            .await
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
            if self.vat_period == VatPeriod::NotRegistered || self.vat_first_period_end.is_none() {
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

/// The names of the fields that drive the compliance calendar (`ACC.ACL.13`):
/// VAT registration, period and stagger, the TRN, the financial year end,
/// the corporate tax registration, Small Business Relief and QFZP status.
pub const DRIVING_FIELDS: [&str; 8] = [
    "vat_registered",
    "vat_period",
    "vat_first_period_end",
    "trn",
    "financial_year_end",
    "ct_registration_no",
    "small_business_relief",
    "qfzp",
];

/// The calendar-driving fields in which `proposed` differs from `current`,
/// by name ([`DRIVING_FIELDS`]); empty when only contacts or notes differ.
///
/// Decision: a blank text and no text are the same value.
pub fn changed_fields(current: &TaxProfile, proposed: &TaxProfile) -> Vec<&'static str> {
    fn text(value: &Option<String>) -> Option<&str> {
        value.as_deref().map(str::trim).filter(|t| !t.is_empty())
    }
    let mut changed = Vec::new();
    if current.vat_registered != proposed.vat_registered {
        changed.push("vat_registered");
    }
    if current.vat_period != proposed.vat_period {
        changed.push("vat_period");
    }
    if current.vat_first_period_end != proposed.vat_first_period_end {
        changed.push("vat_first_period_end");
    }
    if text(&current.trn) != text(&proposed.trn) {
        changed.push("trn");
    }
    if current.financial_year_end != proposed.financial_year_end {
        changed.push("financial_year_end");
    }
    if text(&current.ct_registration_no) != text(&proposed.ct_registration_no) {
        changed.push("ct_registration_no");
    }
    if current.small_business_relief != proposed.small_business_relief {
        changed.push("small_business_relief");
    }
    if current.qfzp != proposed.qfzp {
        changed.push("qfzp");
    }
    changed
}

/// `by` records `profile` as the first version of its client's tax profile.
///
/// Decision: `ACC.ACL.13` governs *changes* of a profile in use; the first
/// version is the onboarding record (nothing is generated from it yet), so
/// it counts as approved by `by`. Every later version is the approval of a
/// [`TaxProfileChange`](crate::tax_profile_change::TaxProfileChange).
///
/// # Errors
///
/// A 422 when the client has a profile already; the database's errors.
pub async fn record(db: &Db, by: Option<i64>, mut profile: TaxProfile) -> Result<TaxProfile> {
    if TaxProfile::latest(db, profile.client()).await?.is_some() {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("accounting.tax_profile.problem.exists", &[]),
        ));
    }
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
    fn only_the_calendar_driving_fields_count_as_a_change() {
        let base = TaxProfile {
            vat_registered: true,
            vat_period: VatPeriod::Quarterly,
            vat_first_period_end: Some(day("2026-01-31")),
            trn: Some("100000000000003".into()),
            ..TaxProfile::default()
        };
        let mut other = base.clone();
        other.contact_email = Some("tax@example.com".into());
        other.notes = Some("Prefers e-mail".into());
        other.trn = Some(" 100000000000003 ".into());
        assert!(changed_fields(&base, &other).is_empty());
        other.vat_first_period_end = Some(day("2026-02-28"));
        other.qfzp = true;
        assert_eq!(
            changed_fields(&base, &other),
            ["vat_first_period_end", "qfzp"]
        );
        assert!(
            changed_fields(&base, &other)
                .iter()
                .all(|name| DRIVING_FIELDS.contains(name))
        );
    }

    #[test]
    fn vat_periods_count_months() {
        assert_eq!(VatPeriod::NotRegistered.months(), 0);
        assert_eq!(VatPeriod::Monthly.months(), 1);
        assert_eq!(VatPeriod::Quarterly.months(), 3);
        assert_eq!(VatPeriod::default().as_str(), "not_registered");
    }
}
