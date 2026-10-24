//! The insurer services (08 §6 *Insurer*, `AUT.INS.01`, `AUT.INS.02`):
//! recording the terms the workshop has with an insurance company, and
//! reading its rate card.
//!
//! Decisions: the insurer is a party of wl-parties and is recorded once; the
//! payer agreement of wl-invoicing the claims are billed under is an id kept
//! on the row (the agreement and the rate card are kept in wl-invoicing, as
//! for the clinics' payer contracts). A per-insurer rate card is read through
//! the kernel's `RateCards` contract for the source `auto.insurer:N`: a claim
//! pins the card version in force on the day it is registered, so a later
//! change of price does not reprice a repair under way (`FIN.RAT.01`). The
//! card's lines are priced by wl-invoicing; the labour and storage rates
//! agreed with the insurer are kept on the row until the contract can price a
//! line (see `DEFERRED.md`).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::parties::directory;
use wl_kernel::rate_cards::{RateCardVersion, rate_cards};

use crate::db::refused;
use crate::insurer::Insurer;
use crate::permissions;
use crate::texts::english;

/// An insurer to record or to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewInsurer {
    /// The insurance company (a party).
    pub party_id: i64,
    /// The payer agreement of wl-invoicing.
    pub payer_agreement_id: Option<i64>,
    /// Where estimates and letters are emailed.
    pub claim_email: Option<String>,
    /// The surveyors' contacts.
    pub surveyor_contacts: Option<String>,
    /// Days the insurer takes to pay.
    pub payment_terms_days: i64,
    /// The hourly labour rate agreed, in fils.
    pub labour_rate: Option<i64>,
    /// The daily storage rate agreed, in fils.
    pub storage_rate_daily: Option<i64>,
}

impl NewInsurer {
    /// An insurer that is party `party_id`, paying in 30 days.
    pub fn new(party_id: i64) -> Self {
        NewInsurer {
            party_id,
            payer_agreement_id: None,
            claim_email: None,
            surveyor_contacts: None,
            payment_terms_days: 30,
            labour_rate: None,
            storage_rate_daily: None,
        }
    }

    /// Billed under payer agreement `id`.
    #[must_use]
    pub fn agreement(mut self, id: i64) -> Self {
        self.payer_agreement_id = Some(id);
        self
    }

    /// With this claim email.
    #[must_use]
    pub fn email(mut self, email: &str) -> Self {
        self.claim_email = Some(email.to_owned());
        self
    }

    /// With these surveyor contacts.
    #[must_use]
    pub fn surveyors(mut self, contacts: &str) -> Self {
        self.surveyor_contacts = Some(contacts.to_owned());
        self
    }

    /// Paying in `days`.
    #[must_use]
    pub fn terms(mut self, days: i64) -> Self {
        self.payment_terms_days = days;
        self
    }

    /// With this agreed hourly labour rate.
    #[must_use]
    pub fn labour_rate(mut self, fils: i64) -> Self {
        self.labour_rate = Some(fils);
        self
    }

    /// With this agreed daily storage rate.
    #[must_use]
    pub fn storage_rate(mut self, fils: i64) -> Self {
        self.storage_rate_daily = Some(fils);
        self
    }
}

/// The text of a trimmed field, `None` when blank.
fn optional(text: &Option<String>) -> Option<String> {
    text.as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// Checks the fields of `new`, returning the cleaned email.
fn clean(new: &NewInsurer) -> Result<Option<String>> {
    let email = optional(&new.claim_email);
    if let Some(email) = &email
        && (!email.contains('@') || email.contains(char::is_whitespace))
    {
        return Err(refused(english("auto.insurers.problem.email", &[])));
    }
    if !(0..=365).contains(&new.payment_terms_days) {
        return Err(refused(english("auto.insurers.problem.terms", &[])));
    }
    if new.labour_rate.is_some_and(|rate| rate < 0)
        || new.storage_rate_daily.is_some_and(|rate| rate < 0)
    {
        return Err(refused(english("auto.insurers.problem.rate", &[])));
    }
    Ok(email)
}

/// Refuses a party that is not in the directory.
async fn ensure_party(state: &AppState, party: i64) -> Result {
    let found = directory(state)
        .display_name(&state.db, PartyId(party), "en")
        .await?;
    if found.is_none() {
        return Err(refused(english("auto.insurers.problem.party", &[])));
    }
    Ok(())
}

/// Records the insurer `new` (`AUT.INS.01`).
///
/// # Errors
///
/// A 403 without `insurance.manage`; a 422 for a party that is not in the
/// directory or is an insurer already, an email that is not one, payment terms
/// outside 0..=365 days or a negative rate.
pub async fn register(state: &AppState, by: &AuthUser, new: NewInsurer) -> Result<Insurer> {
    if !by.has_permission(permissions::INSURANCE_MANAGE) {
        return Err(Error::Forbidden);
    }
    let email = clean(&new)?;
    ensure_party(state, new.party_id).await?;
    if Insurer::of_party(&state.db, new.party_id).await?.is_some() {
        return Err(refused(english("auto.insurers.problem.taken", &[])));
    }
    let mut insurer = Insurer {
        party_id: new.party_id,
        payer_agreement_id: new.payer_agreement_id,
        claim_email: email,
        surveyor_contacts: optional(&new.surveyor_contacts),
        payment_terms_days: new.payment_terms_days,
        labour_rate: new.labour_rate,
        storage_rate_daily: new.storage_rate_daily,
        active: true,
        created_by: Some(by.id),
        ..Insurer::default()
    };
    insurer.save(&state.db).await?;
    Insurer::find_or_404(&state.db, insurer.id).await
}

/// Changes the terms of insurer `id` to those of `new` (the party stays).
///
/// # Errors
///
/// As for [`register`], without the party checks.
pub async fn update(state: &AppState, by: &AuthUser, id: i64, new: NewInsurer) -> Result<Insurer> {
    if !by.has_permission(permissions::INSURANCE_MANAGE) {
        return Err(Error::Forbidden);
    }
    let mut insurer = Insurer::find_or_404(&state.db, id).await?;
    let email = clean(&new)?;
    insurer.payer_agreement_id = new.payer_agreement_id;
    insurer.claim_email = email;
    insurer.surveyor_contacts = optional(&new.surveyor_contacts);
    insurer.payment_terms_days = new.payment_terms_days;
    insurer.labour_rate = new.labour_rate;
    insurer.storage_rate_daily = new.storage_rate_daily;
    insurer.save(&state.db).await?;
    Insurer::find_or_404(&state.db, id).await
}

/// Stops (`false`) or resumes (`true`) taking new claims for insurer `id`.
///
/// # Errors
///
/// A 403 without `insurance.manage`.
pub async fn set_active(state: &AppState, by: &AuthUser, id: i64, active: bool) -> Result<Insurer> {
    if !by.has_permission(permissions::INSURANCE_MANAGE) {
        return Err(Error::Forbidden);
    }
    let mut insurer = Insurer::find_or_404(&state.db, id).await?;
    insurer.active = active;
    insurer.save(&state.db).await?;
    Insurer::find_or_404(&state.db, id).await
}

/// Every insurer, by id.
///
/// # Errors
///
/// The database's errors.
pub async fn all(db: &Db) -> Result<Vec<Insurer>> {
    Insurer::query().order_by(Insurer::ID).get(db).await
}

/// The accepted version of the rate card of `insurer` in force on `on`
/// (`FIN.RAT.01`), through the kernel's `RateCards`; `None` when the insurer
/// has no card or no accepted version in force.
///
/// # Errors
///
/// The contract's errors.
pub async fn rate_card_of(
    state: &AppState,
    insurer: &Insurer,
    on: NaiveDate,
) -> Result<Option<RateCardVersion>> {
    rate_cards(state)
        .accepted_version(state, &insurer.subject(), on)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_email_needs_an_at_sign_and_no_spaces() {
        let ok = NewInsurer::new(1).email("claims@orient.example");
        assert_eq!(
            clean(&ok).unwrap(),
            Some("claims@orient.example".to_owned())
        );
        assert!(clean(&NewInsurer::new(1).email("claims orient")).is_err());
        assert_eq!(clean(&NewInsurer::new(1).email("  ")).unwrap(), None);
    }

    #[test]
    fn terms_and_rates_are_within_bounds() {
        assert!(clean(&NewInsurer::new(1).terms(366)).is_err());
        assert!(clean(&NewInsurer::new(1).terms(-1)).is_err());
        assert!(clean(&NewInsurer::new(1).terms(0)).is_ok());
        assert!(clean(&NewInsurer::new(1).labour_rate(-5)).is_err());
        assert!(clean(&NewInsurer::new(1).storage_rate(0)).is_ok());
    }
}
