//! The points ledger (wl-sales §3 *Points movement*, §4.7, §6
//! `LoyaltyLedger`; BL-SAL-22): every change of a loyalty account is a
//! [`PointsMovement`] with the record that caused it, and a sale that earned
//! is a [`LoyaltyEarning`], the key of "each sale earns once".
//!
//! Decisions:
//! - A movement's `points` are signed: positive adds to the balance
//!   (`earn`, `reverse_redeem`, `transfer` in), negative takes off
//!   (`reverse_earn`, `redeem`, `expire`, `transfer` out). For a per-visit
//!   programme a movement counts visits.
//! - An `earn` movement is a **lot**: it carries the day it expires, what was
//!   used (`consumed`, by `redeem` and `expire`, BL-SAL-23 and BL-SAL-24) and
//!   what was taken back (`reversed`). Lots are consumed oldest expiry first.
//! - A **lot** is a positive movement with a `lot_expires_on`: an `earn`, a
//!   `reverse_redeem` (points given back keep the expiry of the lot they came
//!   from, or the setting's days when that has passed) and a positive `adjust`.
//!   Redemptions, expiries and negative adjustments consume lots oldest expiry
//!   first, one movement per lot slice.
//! - `cause` says why a reversal happened (`return`, `credit_note`,
//!   `payment_reversed`, `written_off`, `fulfilment_reversed`); a reversal is
//!   once per cause and source (`SAL.RUL.09`), and a money cause is earned
//!   again when the sale is paid or fulfilled again (`reearned`).

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Executor};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::loyalty_programme::EarnKind;

/// What a movement does to the balance (§3 *Points movement*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PointsKind {
    /// Points (or a visit) earned by a sale.
    #[default]
    Earn,
    /// Points spent on a redemption line (BL-SAL-23).
    Redeem,
    /// Earned points taken back in proportion.
    ReverseEarn,
    /// Redeemed points given back (BL-SAL-23).
    ReverseRedeem,
    /// The rest of a lot after its date (BL-SAL-24).
    Expire,
    /// A correction by hand (BL-SAL-24); a positive one is a lot.
    Adjust,
    /// A lot moved by a party merge (BL-SAL-26): negative on the merged
    /// account, a lot with the same expiry on the survivor's.
    Transfer,
}

/// The causes of a reversal that is earned again when the sale is paid or
/// fulfilled again (`SAL.RUL.09`).
pub const MONEY_CAUSES: &[&str] = &[
    CAUSE_PAYMENT_REVERSED,
    CAUSE_WRITTEN_OFF,
    CAUSE_FULFILMENT_REVERSED,
];

/// The cause of the first earn of a sale.
pub const CAUSE_EARN: &str = "earn";
/// The cause of an earn again after a reversal.
pub const CAUSE_REEARN: &str = "reearn";
/// A sales return.
pub const CAUSE_RETURN: &str = "return";
/// A credit note.
pub const CAUSE_CREDIT_NOTE: &str = "credit_note";
/// A reversed or charged-back payment, or a reopened invoice.
pub const CAUSE_PAYMENT_REVERSED: &str = "payment_reversed";
/// A written-off invoice.
pub const CAUSE_WRITTEN_OFF: &str = "written_off";
/// A reversed fulfilment.
pub const CAUSE_FULFILMENT_REVERSED: &str = "fulfilment_reversed";

/// The cause of a `redeem` movement.
pub const CAUSE_REDEEM: &str = "redeem";
/// The cause of an `expire` movement.
pub const CAUSE_EXPIRE: &str = "expire";
/// The cause of an `adjust` movement.
pub const CAUSE_ADJUST: &str = "adjust";
/// The cause of a `transfer` movement: a party merge moved the lot.
pub const CAUSE_TRANSFER: &str = "transfer";
/// Redeemed points given back for a cancelled order line.
pub const CAUSE_CANCEL_LINE: &str = "cancel_line";
/// Redeemed points given back because the order was cancelled.
pub const CAUSE_ORDER_CANCELLED: &str = "order_cancelled";
/// Redeemed points given back because the order was rejected.
pub const CAUSE_ORDER_REJECTED: &str = "order_rejected";

/// A change to a loyalty account.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_points_movements",
    index(account_id),
    index(source_type, source_id, kind),
    index(earning_id)
)]
pub struct PointsMovement {
    /// The primary key.
    pub id: i64,
    /// The account.
    #[model(references = "wl_sales_loyalty_accounts")]
    pub account_id: i64,
    /// What it does.
    pub kind: PointsKind,
    /// Points (or visits), signed: positive adds to the balance.
    pub points: i64,
    /// Their value in fils.
    pub value: i64,
    /// The legal entity of the sale.
    pub legal_entity_id: Option<i64>,
    /// The type of the record that caused it.
    pub source_type: String,
    /// The id of the record that caused it.
    pub source_id: i64,
    /// The earning it belongs to.
    pub earning_id: Option<i64>,
    /// Why: `earn`, `reearn`, `return`, `credit_note`, `payment_reversed`...
    pub cause: String,
    /// Earn lots: the last day the points can be redeemed.
    pub lot_expires_on: Option<NaiveDate>,
    /// Earn lots: the points used by redemptions and expiries.
    pub consumed: i64,
    /// Earn lots: the points taken back.
    pub reversed: i64,
    /// Reversals and expiries: the earn movement they act on.
    pub lot_id: Option<i64>,
    /// A reversal of a money cause that was earned again.
    pub reearned: bool,
    /// The reason or note.
    pub reason: Option<String>,
    /// The approval request of a manual adjustment.
    pub approval_request_id: Option<i64>,
    /// The user who did it, if a user did.
    pub user_id: Option<i64>,
    /// Lots: the day the member was reminded that the lot expires.
    pub reminded_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PointsMovement {
    /// The record that caused it.
    pub fn source(&self) -> Subject {
        Subject::new(&self.source_type, self.source_id)
    }

    /// The movements of `account`, oldest first.
    pub async fn of_account(db: &Db, account: i64) -> Result<Vec<PointsMovement>> {
        PointsMovement::query()
            .where_(PointsMovement::ACCOUNT_ID.eq(account))
            .order_by(PointsMovement::ID)
            .get(db)
            .await
    }

    /// The points left in an earn lot: used and taken back come off, and
    /// never below zero.
    pub fn lot_remaining(&self) -> i64 {
        (self.points - self.consumed - self.reversed).max(0)
    }

    /// Whether the movement is a lot of points that can be redeemed or
    /// expire: positive and with an expiry day (an `earn`, a `reverse_redeem`,
    /// a positive `adjust`, or a `transfer` in from a merged account).
    pub fn is_lot(&self) -> bool {
        self.points > 0
            && self.lot_expires_on.is_some()
            && matches!(
                self.kind,
                PointsKind::Earn
                    | PointsKind::ReverseRedeem
                    | PointsKind::Adjust
                    | PointsKind::Transfer
            )
    }

    /// The lots of `account` with points left, oldest expiry first (then
    /// oldest movement first). With `on`, only those whose last day of use
    /// is `on` or later.
    pub async fn lots<'c>(
        db: impl Executor<'c>,
        account: i64,
        on: Option<NaiveDate>,
    ) -> Result<Vec<PointsMovement>> {
        let mut lots: Vec<PointsMovement> = PointsMovement::query()
            .where_(PointsMovement::ACCOUNT_ID.eq(account))
            .order_by(PointsMovement::ID)
            .get(db)
            .await?
            .into_iter()
            .filter(|m| {
                m.is_lot()
                    && m.lot_remaining() > 0
                    && on.is_none_or(|day| m.lot_expires_on.is_some_and(|d| d >= day))
            })
            .collect();
        lots.sort_by_key(|m| (m.lot_expires_on, m.id));
        Ok(lots)
    }
}

/// A sale that earned points or a visit. The source is unique: each sale
/// earns once.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_loyalty_earnings",
    unique(source_type, source_id),
    index(account_id, earned_on)
)]
pub struct LoyaltyEarning {
    /// The primary key.
    pub id: i64,
    /// The account.
    #[model(references = "wl_sales_loyalty_accounts")]
    pub account_id: i64,
    /// The type of the sale: a counter sale, a sales order, an invoice.
    pub source_type: String,
    /// The id of the sale.
    pub source_id: i64,
    /// What was counted before VAT, in fils.
    pub eligible_net: i64,
    /// What the member paid or owes with VAT, the base of a payment
    /// reversal, in fils.
    pub basis_gross: i64,
    /// The points earned, or 1 for a visit.
    pub points: i64,
    /// Points or visits.
    pub earn_kind: EarnKind,
    /// The points (or visits) taken back so far.
    pub reversed_points: i64,
    /// The earn movement that holds the lot (the newest after an earn again).
    pub movement_id: Option<i64>,
    /// Whether the sale made a reward available.
    pub reward_made: bool,
    /// The day (Asia/Dubai) the sale earned.
    pub earned_on: NaiveDate,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl LoyaltyEarning {
    /// The sale as a kernel subject.
    pub fn source(&self) -> Subject {
        Subject::new(&self.source_type, self.source_id)
    }

    /// The earning of the sale `source`, if it earned.
    pub async fn of_source(db: &Db, source: &Subject) -> Result<Option<LoyaltyEarning>> {
        LoyaltyEarning::query()
            .where_(LoyaltyEarning::SOURCE_TYPE.eq(source.subject_type.clone()))
            .where_(LoyaltyEarning::SOURCE_ID.eq(source.subject_id))
            .first(db)
            .await
    }

    /// The points (or visits) not taken back yet.
    pub fn outstanding(&self) -> i64 {
        (self.points - self.reversed_points).max(0)
    }
}

/// An account that earned on more sales in a day than the limit, listed for
/// the manager's review (`SAL.ACL.08`).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_sales_loyalty_reviews", unique(account_id, on_date))]
pub struct LoyaltyReview {
    /// The primary key.
    pub id: i64,
    /// The account.
    #[model(references = "wl_sales_loyalty_accounts")]
    pub account_id: i64,
    /// The day.
    pub on_date: NaiveDate,
    /// How many sales earned that day so far.
    pub sales_count: i64,
    /// When a manager reviewed it.
    pub resolved_at: Option<DateTime>,
    /// The manager.
    pub resolved_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_stored_in_snake_case() {
        let names: Vec<_> = PointsKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            names,
            [
                "earn",
                "redeem",
                "reverse_earn",
                "reverse_redeem",
                "expire",
                "adjust",
                "transfer"
            ]
        );
    }

    #[test]
    fn a_lot_never_shows_less_than_zero() {
        let lot = PointsMovement {
            points: 100,
            consumed: 30,
            reversed: 20,
            ..PointsMovement::default()
        };
        assert_eq!(lot.lot_remaining(), 50);
        let over = PointsMovement {
            points: 100,
            reversed: 150,
            ..PointsMovement::default()
        };
        assert_eq!(over.lot_remaining(), 0);
    }

    #[test]
    fn an_earning_has_what_is_not_taken_back() {
        let earning = LoyaltyEarning {
            points: 100,
            reversed_points: 40,
            ..LoyaltyEarning::default()
        };
        assert_eq!(earning.outstanding(), 60);
    }
}
