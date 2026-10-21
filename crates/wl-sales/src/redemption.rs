//! A redemption of points on a sale (wl-sales §5.9 step 3, §7; BL-SAL-23):
//! the member pays part of a counter sale or a sales order with points. It
//! has five states and moves only through the machine in
//! [`crate::redemption_machine`]:
//!
//! ```text
//! [*] pending --confirm--> held --apply--> applied
//!        |                   |
//!        |--refuse--> refused
//!        +--release--------> released <--release-- held
//! ```
//!
//! - `pending`: the points were asked for and the code sent; no line yet.
//! - `held`: the member confirmed; the discount line is on the sale and the
//!   points are held (not redeemable elsewhere, not expiring).
//! - `applied`: the sale completed (or the order was confirmed); the `redeem`
//!   movements are written.
//! - `released`: the hold lapsed or the sale was abandoned; nothing was
//!   written.
//! - `refused`: three wrong codes (`too_many_tries`), or the code could not
//!   be sent (`code_not_sent`).
//!
//! Decisions:
//! - One live redemption per sale (a partial unique index); a refused or
//!   released one does not count, so a new request after `code_not_sent` or a
//!   lapsed hold is allowed. After `too_many_tries` no new request is
//!   allowed for that sale.
//! - What a cancelled line or a return gives back is the redeemed points ×
//!   (the given-back value ÷ the sale's value with VAT after the other
//!   discounts, [`Redemption::basis`]), rounded down to whole points, and
//!   `returned_points` and `returned_value` remember what was given back.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// The refusal reason of a redemption whose code was wrong three times.
pub const REFUSED_TOO_MANY_TRIES: &str = "too_many_tries";

/// The refusal reason of a redemption whose code could not be sent.
pub const REFUSED_CODE_NOT_SENT: &str = "code_not_sent";

/// How the member confirmed: the one-time code.
pub const VIA_CODE: &str = "code";

/// How the member confirmed: a portal login.
pub const VIA_PORTAL: &str = "portal";

/// Where a redemption is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RedemptionStatus {
    /// Asked for; the code was sent.
    #[default]
    Pending,
    /// Confirmed; the line is on the sale and the points are held.
    Held,
    /// The sale completed or the order was confirmed; the points are spent.
    Applied,
    /// The hold lapsed or the sale was abandoned.
    Released,
    /// The code was wrong three times or could not be sent.
    Refused,
}

/// A member's redemption of points on a sale.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_redemptions",
    state_machine,
    index(account_id),
    index(sale_type, sale_id),
    index(status)
)]
pub struct Redemption {
    /// The primary key.
    pub id: i64,
    /// The member's account.
    #[model(references = "wl_sales_loyalty_accounts")]
    pub account_id: i64,
    /// The type of the sale: a counter sale or a sales order.
    pub sale_type: String,
    /// The id of the sale.
    pub sale_id: i64,
    /// The redemption line on the sale, once confirmed.
    pub line_id: Option<i64>,
    /// The points to spend.
    pub points: i64,
    /// What they are worth, in fils.
    pub value: i64,
    /// The sale's value with VAT after the other discounts, in fils.
    pub basis: i64,
    /// The points given back so far.
    pub returned_points: i64,
    /// The value of the sale given back so far (returns), in fils.
    pub returned_value: i64,
    /// The user who asked for it.
    pub requested_by: Option<i64>,
    /// `code` or `portal`.
    pub confirmed_via: Option<String>,
    /// The points are held until; empty: until the sale resolves.
    pub hold_until: Option<DateTime>,
    /// Why it was refused.
    pub refusal: Option<String>,
    /// When the points were spent.
    pub applied_at: Option<DateTime>,
    /// Where it is; changed only by the machine.
    pub status: RedemptionStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Redemption {
    /// The sale as a kernel subject.
    pub fn sale(&self) -> Subject {
        Subject::new(&self.sale_type, self.sale_id)
    }

    /// Whether the points are held now: confirmed and not past the hold.
    pub fn holds_at(&self, at: DateTime) -> bool {
        self.status == RedemptionStatus::Held && self.hold_until.is_none_or(|until| until > at)
    }

    /// The points still redeemed: not given back yet.
    pub fn outstanding(&self) -> i64 {
        (self.points - self.returned_points).max(0)
    }

    /// The redemptions of `sale`, oldest first.
    pub async fn of_sale(db: &Db, sale: &Subject) -> Result<Vec<Redemption>> {
        Redemption::query()
            .where_(Redemption::SALE_TYPE.eq(sale.subject_type.clone()))
            .where_(Redemption::SALE_ID.eq(sale.subject_id))
            .order_by(Redemption::ID)
            .get(db)
            .await
    }

    /// The newest redemption of `sale` in `status`, if any.
    pub async fn latest_of_sale(
        db: &Db,
        sale: &Subject,
        status: RedemptionStatus,
    ) -> Result<Option<Redemption>> {
        Redemption::query()
            .where_(Redemption::SALE_TYPE.eq(sale.subject_type.clone()))
            .where_(Redemption::SALE_ID.eq(sale.subject_id))
            .where_(Redemption::STATUS.eq(status))
            .order_by_desc(Redemption::ID)
            .first(db)
            .await
    }

    /// The held redemptions of `account` (some may have lapsed: see
    /// [`Redemption::holds_at`]).
    pub async fn held_of_account(db: &Db, account: i64) -> Result<Vec<Redemption>> {
        Redemption::query()
            .where_(Redemption::ACCOUNT_ID.eq(account))
            .where_(Redemption::STATUS.eq(RedemptionStatus::Held))
            .order_by(Redemption::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::TimeDelta;

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let names: Vec<_> = RedemptionStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(names, ["pending", "held", "applied", "released", "refused"]);
    }

    #[test]
    fn a_hold_lasts_until_its_time_and_an_empty_hold_until_the_sale_resolves() {
        let at = renox::db::now();
        let held = Redemption {
            status: RedemptionStatus::Held,
            hold_until: Some(at + TimeDelta::minutes(30)),
            ..Redemption::default()
        };
        assert!(held.holds_at(at));
        assert!(!held.holds_at(at + TimeDelta::minutes(31)));
        let open = Redemption {
            hold_until: None,
            ..held.clone()
        };
        assert!(open.holds_at(at + TimeDelta::days(3)));
        let pending = Redemption::default();
        assert!(!pending.holds_at(at));
    }

    #[test]
    fn what_is_left_is_what_was_not_given_back() {
        let r = Redemption {
            points: 800,
            returned_points: 300,
            ..Redemption::default()
        };
        assert_eq!(r.outstanding(), 500);
        let sale = Redemption {
            sale_type: "sales.counter_sale".into(),
            sale_id: 4,
            ..Redemption::default()
        };
        assert_eq!(sale.sale(), Subject::new("sales.counter_sale", 4));
    }
}
