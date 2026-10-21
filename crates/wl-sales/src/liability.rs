//! The points liability report (wl-sales §5.9 step 6, §10; BL-SAL-24): the
//! points outstanding as at a date and what they are worth, by brand, entity,
//! age and expiry month, for the accountant. Negative balances count as zero,
//! and the value is the points at the point value (`⚠ open: accountant
//! (A-15)`: how it is booked is not decided; until answered the liability is
//! valued at the point value).
//!
//! Decisions:
//! - "As at a date" is read from the movements: a lot counts with what
//!   movements up to the end of that Dubai day had left of it (`redeem`,
//!   `expire`, negative `adjust` and `reverse_earn` against it), and only when
//!   it was created by then and its last day is not before the date.
//! - Each movement carries the legal entity of its sale, so points earned in
//!   one entity and redeemed in another are in the entity that earned them.
//! - Age is the days from the day the lot was created to the date: up to 90,
//!   91 to 180, 181 to 365 and over a year.
//! - When an account's lots hold more than its balance as at the date (a
//!   reversal went past what was left), the latest-expiring lots give way, so
//!   the account never counts more than its balance.

use std::collections::BTreeMap;

use renox::chrono::{Datelike, NaiveDate};
use renox::prelude::*;
use wl_kernel::documents::dubai_date;

use crate::loyalty_account::LoyaltyAccount;
use crate::loyalty_points::{PointsKind, PointsMovement};
use crate::loyalty_programme::LoyaltyProgramme;

/// The age bucket of a lot created `days` before the date.
pub fn age_bucket(days: i64) -> &'static str {
    match days {
        ..=90 => "0_90",
        91..=180 => "91_180",
        181..=365 => "181_365",
        _ => "over_365",
    }
}

/// One line of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiabilityRow {
    /// The brand of the programme; empty for the client's own.
    pub brand: String,
    /// The legal entity of the sale that earned the points.
    pub entity: Option<i64>,
    /// The age bucket: `0_90`, `91_180`, `181_365` or `over_365`.
    pub age: &'static str,
    /// The month the points expire, `2027-06`.
    pub expiry_month: String,
    /// The outstanding points.
    pub points: i64,
    /// Their value in fils at the point value.
    pub value: i64,
}

/// The points liability as at a date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Liability {
    /// The date.
    pub as_at: NaiveDate,
    /// The lines, by brand, entity, age and expiry month.
    pub rows: Vec<LiabilityRow>,
    /// All outstanding points.
    pub points: i64,
    /// Their value in fils.
    pub value: i64,
}

/// The day a movement was made (Dubai), and the end of the day it counts for.
fn made_on(m: &PointsMovement) -> Option<NaiveDate> {
    m.created_at.map(dubai_date)
}

/// What `movements` left of the lot `lot` as at the end of `as_at`.
fn remaining_at(lot: &PointsMovement, movements: &[PointsMovement], as_at: NaiveDate) -> i64 {
    let used: i64 = movements
        .iter()
        .filter(|m| m.lot_id == Some(lot.id) && made_on(m).is_none_or(|d| d <= as_at))
        .filter(|m| {
            matches!(
                m.kind,
                PointsKind::Redeem | PointsKind::Expire | PointsKind::ReverseEarn
            ) || (m.kind == PointsKind::Adjust && m.points < 0)
        })
        .map(|m| -m.points)
        .sum();
    (lot.points - used).max(0)
}

/// The points liability as at `as_at`.
pub async fn report(db: &Db, as_at: NaiveDate) -> Result<Liability> {
    let mut grouped: BTreeMap<(String, Option<i64>, &'static str, String), (i64, i64)> =
        BTreeMap::new();
    for account in LoyaltyAccount::query().order_by(LoyaltyAccount::ID).get(db).await? {
        let Some(programme) = LoyaltyProgramme::find(db, account.programme_id).await? else {
            continue;
        };
        let movements: Vec<PointsMovement> = PointsMovement::of_account(db, account.id)
            .await?
            .into_iter()
            .filter(|m| made_on(m).is_none_or(|d| d <= as_at))
            .collect();
        let balance: i64 = movements.iter().map(|m| m.points).sum();
        if balance <= 0 {
            continue;
        }
        let mut lots: Vec<(PointsMovement, i64)> = movements
            .iter()
            .filter(|m| m.is_lot() && m.lot_expires_on.is_some_and(|last| last >= as_at))
            .map(|lot| (lot.clone(), remaining_at(lot, &movements, as_at)))
            .filter(|(_, left)| *left > 0)
            .collect();
        lots.sort_by_key(|(lot, _)| (lot.lot_expires_on, lot.id));
        // Never more than the balance: the latest lots give way.
        let mut room = balance;
        for (lot, left) in lots {
            let counted = left.min(room);
            if counted <= 0 {
                break;
            }
            room -= counted;
            let Some(last) = lot.lot_expires_on else {
                continue;
            };
            let age = made_on(&lot).map_or(0, |day| (as_at - day).num_days());
            let key = (
                programme.brand.clone().unwrap_or_default(),
                lot.legal_entity_id,
                age_bucket(age),
                format!("{}-{:02}", last.year(), last.month()),
            );
            let entry = grouped.entry(key).or_insert((0, 0));
            entry.0 += counted;
            entry.1 += counted * programme.point_value;
        }
    }
    let rows: Vec<LiabilityRow> = grouped
        .into_iter()
        .map(|((brand, entity, age, expiry_month), (points, value))| LiabilityRow {
            brand,
            entity,
            age,
            expiry_month,
            points,
            value,
        })
        .collect();
    Ok(Liability {
        as_at,
        points: rows.iter().map(|r| r.points).sum(),
        value: rows.iter().map(|r| r.value).sum(),
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_age_buckets_follow_the_days() {
        assert_eq!(age_bucket(0), "0_90");
        assert_eq!(age_bucket(90), "0_90");
        assert_eq!(age_bucket(91), "91_180");
        assert_eq!(age_bucket(181), "181_365");
        assert_eq!(age_bucket(366), "over_365");
        assert_eq!(age_bucket(-3), "0_90");
    }

    #[test]
    fn a_lot_loses_what_redemptions_expiries_and_reversals_used() {
        let lot = PointsMovement {
            id: 1,
            points: 300,
            ..PointsMovement::default()
        };
        let used = |kind: PointsKind, points: i64| PointsMovement {
            kind,
            points,
            lot_id: Some(1),
            ..PointsMovement::default()
        };
        let day = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
        let movements = vec![
            lot.clone(),
            used(PointsKind::Redeem, -100),
            used(PointsKind::ReverseEarn, -50),
            used(PointsKind::ReverseRedeem, 100),
            used(PointsKind::Adjust, 40),
        ];
        // The given-back redemption is a lot of its own and the positive
        // adjustment adds points; neither is used up.
        assert_eq!(remaining_at(&lot, &movements, day), 150);
    }
}
