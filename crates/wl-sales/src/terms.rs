//! Changing the loyalty terms and turning the programme off (wl-sales §4.7,
//! §7 *Notice before a programme change takes effect*; BL-SAL-26).
//!
//! **A change** of the earn rate, the minimum, the excluded items or the
//! validity is saved with an effective date at least the notice days ahead
//! (30); a shorter notice is refused. The members with an open account are
//! told when it is saved. Sales from the effective date earn on the new terms
//! ([`LoyaltyProgramme::terms_on`]); a sale before it, and every lot already
//! earned, keep the old ones: lots are never touched.
//!
//! **Turning it off** stops earning at once ([`LoyaltyProgramme::earns_on`]
//! is false with a closing date) and sets the closing date the redemption days
//! (90) ahead: members are told, can still redeem through that day, and the
//! expiry run after it expires every open lot (`SAL.RUL.08`, [`crate::expiry`]).
//!
//! Decisions:
//! - The point value is not among the terms a change can carry. Redeeming
//!   values points at the programme's current point value, so a new value
//!   would also reprice lots already earned, which the open question L-39
//!   (`⚠ open: lawyer`) forbids until it is answered: existing lots are never
//!   changed. A per-lot value in the redemption lifts this.
//! - A paused account (the party is on hold) is told nothing.

use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::config::config_of;
use crate::loyalty_account::{AccountStatus, LoyaltyAccount};
use crate::loyalty_programme::LoyaltyProgramme;
use crate::notices;
use crate::texts::english;

/// A change of the terms with its effective date.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_loyalty_terms_changes",
    index(programme_id, effective_from)
)]
pub struct LoyaltyTermsChange {
    /// The primary key.
    pub id: i64,
    /// The programme.
    #[model(references = "wl_sales_loyalty_programmes")]
    pub programme_id: i64,
    /// The first day the new terms apply to a sale.
    pub effective_from: NaiveDate,
    /// The new points per AED, or visits per reward.
    pub earn_rate: Option<i64>,
    /// The new least eligible amount before VAT, in fils.
    pub minimum_amount: Option<i64>,
    /// The new excluded item codes, separated by commas.
    pub excluded_items: Option<String>,
    /// The new months points earned from then on are valid.
    pub validity_months: Option<i64>,
    /// The new version of the terms members accept.
    pub terms_version: Option<String>,
    /// The new terms in English.
    pub terms_text_en: Option<String>,
    /// The new terms in Arabic.
    pub terms_text_ar: Option<String>,
    /// The user who saved it.
    pub created_by: Option<i64>,
    /// The members told when it was saved.
    pub notified_members: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl LoyaltyTermsChange {
    /// Lays the change over `programme`.
    pub fn apply_to(&self, programme: &mut LoyaltyProgramme) {
        if let Some(rate) = self.earn_rate {
            programme.earn_rate = rate;
        }
        if let Some(minimum) = self.minimum_amount {
            programme.minimum_amount = minimum;
        }
        if let Some(items) = &self.excluded_items {
            programme.excluded_items = items.clone();
        }
        if let Some(months) = self.validity_months {
            programme.validity_months = months;
        }
        if let Some(version) = &self.terms_version {
            programme.terms_version = version.clone();
        }
        if let Some(text) = &self.terms_text_en {
            programme.terms_text_en = text.clone();
        }
        if let Some(text) = &self.terms_text_ar {
            programme.terms_text_ar = text.clone();
        }
    }

    /// The changes of `programme` that apply to a sale of `on`, oldest
    /// effective date first.
    pub async fn due(db: &Db, programme: i64, on: NaiveDate) -> Result<Vec<LoyaltyTermsChange>> {
        let mut due = LoyaltyTermsChange::query()
            .where_(LoyaltyTermsChange::PROGRAMME_ID.eq(programme))
            .where_(LoyaltyTermsChange::EFFECTIVE_FROM.lte(on))
            .order_by(LoyaltyTermsChange::ID)
            .get(db)
            .await?;
        due.sort_by_key(|c| (c.effective_from, c.id));
        Ok(due)
    }

    /// What the change does, in one line, for the notice.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(rate) = self.earn_rate {
            parts.push(english(
                "wl_sales.terms.summary.earn_rate",
                &[("rate", &rate)],
            ));
        }
        if let Some(minimum) = self.minimum_amount {
            let amount = notices::aed(minimum);
            parts.push(english(
                "wl_sales.terms.summary.minimum",
                &[("amount", &amount)],
            ));
        }
        if let Some(items) = &self.excluded_items {
            parts.push(english(
                "wl_sales.terms.summary.excluded",
                &[("items", items)],
            ));
        }
        if let Some(months) = self.validity_months {
            parts.push(english(
                "wl_sales.terms.summary.validity",
                &[("months", &months)],
            ));
        }
        if parts.is_empty() {
            parts.push(english("wl_sales.terms.summary.terms", &[]));
        }
        parts.join("; ")
    }
}

impl LoyaltyProgramme {
    /// The programme with the terms that apply to a sale of `on`: every
    /// change whose effective date is on or before it laid over the current
    /// terms.
    pub async fn terms_on(&self, db: &Db, on: NaiveDate) -> Result<LoyaltyProgramme> {
        let mut programme = self.clone();
        for change in LoyaltyTermsChange::due(db, self.id, on).await? {
            change.apply_to(&mut programme);
        }
        Ok(programme)
    }
}

/// What to change; leave a field `None` to keep it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TermsChange {
    /// The first day the new terms apply.
    pub effective_from: NaiveDate,
    /// Points per AED, or visits per reward.
    pub earn_rate: Option<i64>,
    /// The least eligible amount before VAT, in fils.
    pub minimum_amount: Option<i64>,
    /// The item codes that never earn.
    pub excluded_items: Option<Vec<String>>,
    /// The months points earned from then on are valid.
    pub validity_months: Option<i64>,
    /// The version of the terms members accept.
    pub terms_version: Option<String>,
    /// The terms in English.
    pub terms_text_en: Option<String>,
    /// The terms in Arabic.
    pub terms_text_ar: Option<String>,
}

impl TermsChange {
    /// A change that takes effect on `effective_from` and changes nothing yet.
    pub fn new(effective_from: NaiveDate) -> Self {
        TermsChange {
            effective_from,
            earn_rate: None,
            minimum_amount: None,
            excluded_items: None,
            validity_months: None,
            terms_version: None,
            terms_text_en: None,
            terms_text_ar: None,
        }
    }

    /// Points per AED (or visits per reward) become `rate`.
    pub fn earn_rate(mut self, rate: i64) -> Self {
        self.earn_rate = Some(rate);
        self
    }

    /// The least amount before VAT becomes `fils`.
    pub fn minimum_amount(mut self, fils: i64) -> Self {
        self.minimum_amount = Some(fils);
        self
    }

    /// The excluded items become `items`.
    pub fn excluding(mut self, items: &[&str]) -> Self {
        self.excluded_items = Some(items.iter().map(|i| (*i).to_owned()).collect());
        self
    }

    /// Points earned from then on are valid for `months`.
    pub fn validity_months(mut self, months: i64) -> Self {
        self.validity_months = Some(months);
        self
    }

    /// The terms members accept become version `version`.
    pub fn terms(mut self, version: &str, english: &str, arabic: &str) -> Self {
        self.terms_version = Some(version.to_owned());
        self.terms_text_en = Some(english.to_owned());
        self.terms_text_ar = Some(arabic.to_owned());
        self
    }

    /// Whether the change carries anything.
    fn is_empty(&self) -> bool {
        self.earn_rate.is_none()
            && self.minimum_amount.is_none()
            && self.excluded_items.is_none()
            && self.validity_months.is_none()
            && self.terms_version.is_none()
    }
}

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// The accounts of `programme` that are open.
async fn open_accounts(db: &Db, programme: i64) -> Result<Vec<LoyaltyAccount>> {
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::PROGRAMME_ID.eq(programme))
        .where_(LoyaltyAccount::STATUS.eq(AccountStatus::Active))
        .order_by(LoyaltyAccount::ID)
        .get(db)
        .await
}

/// Saves a change of the terms of `programme` for `by` and tells the members.
///
/// # Errors
///
/// 422 when the programme is not running, nothing changes, a number is out of
/// range, or the effective date is less than the notice days (30) ahead.
pub async fn change_terms(
    state: &AppState,
    programme: i64,
    by: Option<i64>,
    change: TermsChange,
) -> Result<LoyaltyTermsChange> {
    let Some(found) = LoyaltyProgramme::find(&state.db, programme).await? else {
        return Err(refused("wl_sales.terms.problem.no_programme"));
    };
    if !found.active || found.closing_date.is_some() {
        return Err(refused("wl_sales.terms.problem.not_running"));
    }
    if change.is_empty() {
        return Err(refused("wl_sales.terms.problem.nothing"));
    }
    if change.earn_rate.is_some_and(|r| r < 1)
        || change.validity_months.is_some_and(|m| m < 1)
        || change.minimum_amount.is_some_and(|m| m < 0)
    {
        return Err(refused("wl_sales.loyalty.problem.terms"));
    }
    let days = config_of(state).programme_notice_days;
    let earliest = today() + TimeDelta::days(days);
    if change.effective_from < earliest {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("wl_sales.terms.problem.notice", &[("days", &days)]),
        ));
    }
    let saved = LoyaltyTermsChange::create(
        &state.db,
        LoyaltyTermsChange {
            programme_id: found.id,
            effective_from: change.effective_from,
            earn_rate: change.earn_rate,
            minimum_amount: change.minimum_amount,
            excluded_items: change.excluded_items.map(|items| items.join(",")),
            validity_months: change.validity_months,
            terms_version: change.terms_version,
            terms_text_en: change.terms_text_en,
            terms_text_ar: change.terms_text_ar,
            created_by: by,
            ..LoyaltyTermsChange::default()
        },
    )
    .await?;
    let summary = saved.summary();
    let mut told: i64 = 0;
    for account in open_accounts(&state.db, found.id).await? {
        if account.paused {
            continue;
        }
        notices::loyalty_terms_changed(state, &account, &summary, saved.effective_from).await?;
        told += 1;
    }
    LoyaltyTermsChange::query()
        .where_(LoyaltyTermsChange::ID.eq(saved.id))
        .update(&state.db, &[("notified_members", &told)])
        .await?;
    LoyaltyTermsChange::find_or_404(&state.db, saved.id).await
}

/// Turns `programme` off: earning stops now, the closing date is the
/// redemption days (90) ahead, the members are told, and every open lot
/// expires in the expiry run after the closing date.
///
/// # Errors
///
/// 422 when the programme is not running.
pub async fn turn_off(state: &AppState, programme: i64) -> Result<LoyaltyProgramme> {
    let Some(found) = LoyaltyProgramme::find(&state.db, programme).await? else {
        return Err(refused("wl_sales.terms.problem.no_programme"));
    };
    if !found.active || found.closing_date.is_some() {
        return Err(refused("wl_sales.terms.problem.not_running"));
    }
    let closing = today() + TimeDelta::days(config_of(state).programme_closing_days);
    LoyaltyProgramme::query()
        .where_(LoyaltyProgramme::ID.eq(found.id))
        .update(
            &state.db,
            &[
                ("closing_date", &Some(closing)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    for account in open_accounts(&state.db, found.id).await? {
        notices::loyalty_programme_off(state, &account, closing).await?;
    }
    LoyaltyProgramme::find_or_404(&state.db, found.id).await
}

/// Ends the programmes whose closing date is before `on`: they no longer
/// count as active, so the brand can start a new one. Their lots expired in
/// the same run. Returns how many.
///
/// # Errors
///
/// The database's errors.
pub async fn end_closed(state: &AppState, on: NaiveDate) -> Result<usize> {
    let closed = LoyaltyProgramme::query()
        .where_(LoyaltyProgramme::ACTIVE.eq(true))
        .where_(LoyaltyProgramme::CLOSING_DATE.lt(on))
        .get(&state.db)
        .await?;
    for programme in &closed {
        LoyaltyProgramme::query()
            .where_(LoyaltyProgramme::ID.eq(programme.id))
            .update(&state.db, &[("active", &false)])
            .await?;
    }
    Ok(closed.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    #[test]
    fn a_change_lays_over_the_programme_and_keeps_the_rest() {
        let mut programme = LoyaltyProgramme {
            earn_rate: 1,
            validity_months: 12,
            excluded_items: "tips".into(),
            ..LoyaltyProgramme::default()
        };
        let change = LoyaltyTermsChange {
            earn_rate: Some(2),
            excluded_items: Some("tips,gift".into()),
            ..LoyaltyTermsChange::default()
        };
        change.apply_to(&mut programme);
        assert_eq!(programme.earn_rate, 2);
        assert_eq!(programme.validity_months, 12);
        assert_eq!(programme.excluded_items, "tips,gift");
    }

    #[test]
    fn a_change_that_carries_nothing_is_empty() {
        assert!(TermsChange::new(day(12, 1)).is_empty());
        assert!(!TermsChange::new(day(12, 1)).earn_rate(2).is_empty());
    }

    #[test]
    fn the_summary_names_what_changes() {
        let change = LoyaltyTermsChange {
            earn_rate: Some(2),
            validity_months: Some(6),
            ..LoyaltyTermsChange::default()
        };
        let summary = change.summary();
        assert!(summary.contains('2') && summary.contains('6'), "{summary}");
    }
}
