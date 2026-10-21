//! The loyalty programme (wl-sales §3 *Loyalty programme*, `SAL.LOY.01`;
//! BL-SAL-22): the earn and redeem terms of the client. A client turns it on
//! by creating one; customers then enrol ([`crate::loyalty_account`]) and
//! earn points per AED spent or a visit per qualifying sale
//! ([`crate::loyalty_rules`]).
//!
//! Decisions of this issue:
//! - Money terms are fils: the *minimum qualifying amount* is before VAT, the
//!   *point value* is the fils one point is worth (§7 default AED 0.01 = 1).
//! - The terms text is stored with the programme (English and Arabic); the
//!   versioned documents of `wl-documents` replace it when that module has the
//!   terms template.
//! - Changing a programme with its effective date and notice, and turning it
//!   off (`closing_date`), are BL-SAL-26: the columns are here, the services
//!   are not.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::texts::english;

/// What a programme counts.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EarnKind {
    /// Points per AED spent.
    #[default]
    PerAed,
    /// One visit per qualifying sale; a reward after a number of visits.
    PerVisit,
}

/// The earn and redeem terms of a client.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_sales_loyalty_programmes", index(active))]
pub struct LoyaltyProgramme {
    /// The primary key.
    pub id: i64,
    /// The name members see.
    pub name: String,
    /// The brand it belongs to (one programme per brand at most).
    pub brand: Option<String>,
    /// Points per AED, or visits per reward.
    pub earn_kind: EarnKind,
    /// Points per `earn_unit_aed` dirhams spent, or visits per reward.
    pub earn_rate: i64,
    /// The dirhams of an eligible sale that earn `earn_rate` points (1: the
    /// points are per AED 1; 05 Clinics: 10, one point per AED 10).
    pub earn_unit_aed: i64,
    /// The least eligible amount of a sale before VAT, in fils.
    pub minimum_amount: i64,
    /// Item codes that never earn, separated by commas.
    pub excluded_items: String,
    /// What one point is worth, in fils.
    pub point_value: i64,
    /// The fewest points a member can redeem.
    pub min_redeem_points: i64,
    /// The largest part of a sale points may pay, in basis points.
    pub max_share_bps: i64,
    /// What a full set of visits gives (a fixed discount or a free item).
    pub visit_reward: String,
    /// The months points are valid after earning.
    pub validity_months: i64,
    /// The version of the terms members accept.
    pub terms_version: String,
    /// The terms in English.
    pub terms_text_en: String,
    /// The terms in Arabic.
    pub terms_text_ar: String,
    /// The day the current terms apply from; sales before it use the old ones.
    pub effective_from: Option<NaiveDate>,
    /// The last day points can be redeemed after the programme is turned off.
    pub closing_date: Option<NaiveDate>,
    /// Whether it earns.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl LoyaltyProgramme {
    /// The programme earning for sales of `on`: it is active, it applies from
    /// a day on or before `on`, and it has not been turned off.
    pub fn earns_on(&self, on: NaiveDate) -> bool {
        self.active
            && self.closing_date.is_none()
            && self.effective_from.is_none_or(|from| from <= on)
    }

    /// Whether the item `code` never earns.
    pub fn excludes(&self, code: &str) -> bool {
        self.excluded_items
            .split(',')
            .map(str::trim)
            .any(|c| !c.is_empty() && c.eq_ignore_ascii_case(code))
    }

    /// The points a sale with `eligible_net` fils earns: whole points,
    /// rounded down (AED 99.90 → 99 points); one visit for a per-visit
    /// programme.
    pub fn points_for(&self, eligible_net: i64) -> i64 {
        match self.earn_kind {
            EarnKind::PerAed => {
                let unit = i128::from(self.earn_unit_aed.max(1));
                let points = i128::from(eligible_net) * i128::from(self.earn_rate) / (100 * unit);
                i64::try_from(points).unwrap_or(0).max(0)
            }
            EarnKind::PerVisit => 1,
        }
    }

    /// The active programme of `brand` (the client's own when `None`).
    pub async fn active_of(db: &Db, brand: Option<&str>) -> Result<Option<LoyaltyProgramme>> {
        let all = LoyaltyProgramme::query()
            .where_(LoyaltyProgramme::ACTIVE.eq(true))
            .order_by(LoyaltyProgramme::ID)
            .get(db)
            .await?;
        Ok(all
            .into_iter()
            .find(|p| p.brand.as_deref().map(str::to_lowercase) == brand.map(str::to_lowercase)))
    }

    /// The first active programme, whatever its brand.
    pub async fn first_active(db: &Db) -> Result<Option<LoyaltyProgramme>> {
        LoyaltyProgramme::query()
            .where_(LoyaltyProgramme::ACTIVE.eq(true))
            .order_by(LoyaltyProgramme::ID)
            .first(db)
            .await
    }
}

/// A programme to create, with the §7 defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NewProgramme {
    /// The name members see.
    pub name: String,
    /// The brand, when the client has several.
    pub brand: Option<String>,
    /// Points per AED or visits.
    pub earn_kind: EarnKind,
    /// Points per AED, or visits per reward.
    pub earn_rate: i64,
    /// The dirhams of an eligible sale that earn `earn_rate` points.
    pub earn_unit_aed: i64,
    /// The least eligible amount before VAT, in fils.
    pub minimum_amount: i64,
    /// Item codes that never earn.
    pub excluded_items: Vec<String>,
    /// What one point is worth, in fils.
    pub point_value: i64,
    /// The fewest points a member can redeem.
    pub min_redeem_points: i64,
    /// The largest part of a sale points may pay, in basis points.
    pub max_share_bps: i64,
    /// What a full set of visits gives.
    pub visit_reward: String,
    /// The months points are valid.
    pub validity_months: i64,
    /// The version of the terms.
    pub terms_version: String,
    /// The terms in English.
    pub terms_text_en: String,
    /// The terms in Arabic.
    pub terms_text_ar: String,
}

impl NewProgramme {
    /// A per-AED programme called `name`: 1 point per AED 1, no minimum,
    /// AED 0.01 per point, 500 points to redeem, up to 100% of a sale, 12
    /// months of validity (§7).
    pub fn new(name: &str) -> Self {
        NewProgramme {
            name: name.trim().to_owned(),
            brand: None,
            earn_kind: EarnKind::PerAed,
            earn_rate: 1,
            earn_unit_aed: 1,
            minimum_amount: 0,
            excluded_items: Vec::new(),
            point_value: 1,
            min_redeem_points: 500,
            max_share_bps: 10_000,
            visit_reward: String::new(),
            validity_months: 12,
            terms_version: "1".to_owned(),
            terms_text_en: String::new(),
            terms_text_ar: String::new(),
        }
    }

    /// For the brand `brand`.
    pub fn brand(mut self, brand: &str) -> Self {
        self.brand = Some(brand.trim().to_owned()).filter(|b| !b.is_empty());
        self
    }

    /// One visit per qualifying sale, a reward after `visits` visits.
    pub fn per_visit(mut self, visits: i64, reward: &str) -> Self {
        self.earn_kind = EarnKind::PerVisit;
        self.earn_rate = visits;
        self.visit_reward = reward.to_owned();
        self
    }

    /// `points` points per AED 1.
    pub fn points_per_aed(mut self, points: i64) -> Self {
        self.earn_kind = EarnKind::PerAed;
        self.earn_rate = points;
        self.earn_unit_aed = 1;
        self
    }

    /// `points` points for every `aed` dirhams (05 Clinics: `1, 10`, one
    /// point per AED 10).
    pub fn points_per_aed_units(mut self, points: i64, aed: i64) -> Self {
        self.earn_kind = EarnKind::PerAed;
        self.earn_rate = points;
        self.earn_unit_aed = aed;
        self
    }

    /// With this many fils of value for a point and this many points to
    /// redeem at least (05 Clinics: 10 fils, 100 points: AED 10).
    pub fn redeeming(mut self, point_value: i64, min_points: i64) -> Self {
        self.point_value = point_value;
        self.min_redeem_points = min_points;
        self
    }

    /// With this least eligible amount before VAT, in fils.
    pub fn minimum_amount(mut self, fils: i64) -> Self {
        self.minimum_amount = fils;
        self
    }

    /// With these items left out.
    pub fn excluding(mut self, items: &[&str]) -> Self {
        self.excluded_items = items.iter().map(|i| (*i).to_owned()).collect();
        self
    }

    /// With this point value in fils.
    pub fn point_value(mut self, fils: i64) -> Self {
        self.point_value = fils;
        self
    }

    /// With these terms.
    pub fn terms(mut self, version: &str, english: &str, arabic: &str) -> Self {
        self.terms_version = version.to_owned();
        self.terms_text_en = english.to_owned();
        self.terms_text_ar = arabic.to_owned();
        self
    }

    /// Valid for `months`.
    pub fn validity_months(mut self, months: i64) -> Self {
        self.validity_months = months;
        self
    }
}

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// Creates the programme and turns it on.
///
/// # Errors
///
/// 422 for a blank name, a rate, point value or validity below 1, a negative
/// minimum, or an active programme of that brand already.
pub async fn create(db: &Db, new: NewProgramme) -> Result<LoyaltyProgramme> {
    if new.name.is_empty() {
        return Err(refused("wl_sales.loyalty.problem.name"));
    }
    if new.earn_rate < 1 || new.earn_unit_aed < 1 || new.point_value < 1 || new.validity_months < 1
    {
        return Err(refused("wl_sales.loyalty.problem.terms"));
    }
    if new.minimum_amount < 0
        || new.min_redeem_points < 0
        || !(0..=10_000).contains(&new.max_share_bps)
    {
        return Err(refused("wl_sales.loyalty.problem.terms"));
    }
    if LoyaltyProgramme::active_of(db, new.brand.as_deref())
        .await?
        .is_some()
    {
        return Err(refused("wl_sales.loyalty.problem.brand_taken"));
    }
    LoyaltyProgramme::create(
        db,
        LoyaltyProgramme {
            name: new.name,
            brand: new.brand,
            earn_kind: new.earn_kind,
            earn_rate: new.earn_rate,
            earn_unit_aed: new.earn_unit_aed,
            minimum_amount: new.minimum_amount,
            excluded_items: new.excluded_items.join(","),
            point_value: new.point_value,
            min_redeem_points: new.min_redeem_points,
            max_share_bps: new.max_share_bps,
            visit_reward: new.visit_reward,
            validity_months: new.validity_months,
            terms_version: new.terms_version,
            terms_text_en: new.terms_text_en,
            terms_text_ar: new.terms_text_ar,
            active: true,
            ..LoyaltyProgramme::default()
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn programme(rate: i64) -> LoyaltyProgramme {
        LoyaltyProgramme {
            earn_rate: rate,
            active: true,
            ..LoyaltyProgramme::default()
        }
    }

    #[test]
    fn points_are_whole_numbers_rounded_down() {
        // AED 99.90 → 99 points; AED 100 → 100; 2 points per AED doubles.
        assert_eq!(programme(1).points_for(9_990), 99);
        assert_eq!(programme(1).points_for(10_000), 100);
        assert_eq!(programme(2).points_for(10_000), 200);
        assert_eq!(programme(1).points_for(-500), 0);
    }

    #[test]
    fn a_programme_can_earn_per_ten_dirhams() {
        // 05 Clinics: 1 point per AED 10, so AED 200 earns 20 points, AED
        // 19.90 one and AED 9.99 none.
        let p = LoyaltyProgramme {
            earn_rate: 1,
            earn_unit_aed: 10,
            ..programme(1)
        };
        assert_eq!(p.points_for(20_000), 20);
        assert_eq!(p.points_for(1_990), 1);
        assert_eq!(p.points_for(999), 0);
    }

    #[test]
    fn a_per_visit_programme_counts_one_visit_per_sale() {
        let p = LoyaltyProgramme {
            earn_kind: EarnKind::PerVisit,
            earn_rate: 10,
            ..LoyaltyProgramme::default()
        };
        assert_eq!(p.points_for(1), 1);
    }

    #[test]
    fn excluded_items_are_matched_by_code_ignoring_case() {
        let p = LoyaltyProgramme {
            excluded_items: "gift_card, Tips".into(),
            ..LoyaltyProgramme::default()
        };
        assert!(p.excludes("GIFT_CARD") && p.excludes("tips"));
        assert!(!p.excludes("tee") && !p.excludes(""));
    }

    #[test]
    fn a_programme_earns_from_its_effective_date_until_it_is_turned_off() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        let mut p = programme(1);
        assert!(p.earns_on(day(1)));
        p.effective_from = Some(day(15));
        assert!(!p.earns_on(day(14)) && p.earns_on(day(15)));
        p.closing_date = Some(day(30));
        assert!(!p.earns_on(day(16)));
        p.closing_date = None;
        p.active = false;
        assert!(!p.earns_on(day(16)));
    }

    #[test]
    fn the_defaults_are_the_spec_values() {
        let new = NewProgramme::new(" Friends ");
        assert_eq!(new.name, "Friends");
        assert_eq!(
            (
                new.earn_rate,
                new.point_value,
                new.min_redeem_points,
                new.max_share_bps
            ),
            (1, 1, 500, 10_000)
        );
        assert_eq!(new.validity_months, 12);
        assert_eq!(
            NewProgramme::new("x").per_visit(10, "free").earn_kind,
            EarnKind::PerVisit
        );
    }
}
