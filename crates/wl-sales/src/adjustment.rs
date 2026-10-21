//! A manual correction of a member's points, or the closing of an account
//! that still has points, by staff (wl-sales §4.7, §5.9 step 5, `SAL.ACL.07`;
//! BL-SAL-24). Its status moves only through the machine in
//! [`crate::adjustment_machine`]:
//!
//! ```text
//! [*] draft --apply--------------------------> applied
//!       |---submit_over_limit--> pending_approval --submit_over_limit_approved--> applied
//!       |                              |--back_to_draft--> draft --reject--> rejected
//! ```
//!
//! Decisions:
//! - Up to the limit (500 points) the adjustment is applied on the spot with
//!   its reason. Above it, and for any closing of an account with points left,
//!   a holder of `loyalty.manage` other than the requester decides under
//!   "Needs your action".
//! - A positive adjustment is a new lot expiring after the programme's
//!   validity; a negative one takes the points from the lots, oldest expiry
//!   first, and never more than the member has. Closing writes an `adjust`
//!   of everything that is left, then closes the account.
//! - A rejected adjustment writes nothing and the requester is told; a
//!   returned one is back in `draft` for its requester to edit and submit
//!   again.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// The subject type of an adjustment in the kernel registry.
pub const ADJUSTMENT_SUBJECT_TYPE: &str = "sales.points_adjustment";

/// Where an adjustment is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AdjustmentStatus {
    /// Being submitted, or returned to its requester.
    #[default]
    Draft,
    /// Waiting for a manager other than the requester.
    PendingApproval,
    /// The `adjust` movement is written.
    Applied,
    /// A manager rejected it; nothing was written.
    Rejected,
}

/// A manual correction of a member's points.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_points_adjustments",
    state_machine,
    index(account_id),
    index(status)
)]
pub struct PointsAdjustment {
    /// The primary key.
    pub id: i64,
    /// The member's account.
    #[model(references = "wl_sales_loyalty_accounts")]
    pub account_id: i64,
    /// The points added (positive) or taken off (negative).
    pub points: i64,
    /// Why.
    pub reason: String,
    /// Staff closing the account: what is left goes with it.
    pub closes_account: bool,
    /// The user who asked for it.
    pub requested_by: i64,
    /// The approval request of the kernel, when it waits for a manager.
    pub approval_request_id: Option<i64>,
    /// The manager who gave the last yes.
    pub approved_by: Option<i64>,
    /// The `adjust` movement it wrote.
    pub movement_id: Option<i64>,
    /// Why it was rejected or returned.
    pub outcome_reason: Option<String>,
    /// Where it is; changed only by the machine.
    pub status: AdjustmentStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PointsAdjustment {
    /// The adjustment as a kernel subject.
    pub fn subject(&self) -> Subject {
        Subject::new(ADJUSTMENT_SUBJECT_TYPE, self.id)
    }

    /// Whether it needs a manager: closing an account with points left, or
    /// more than `limit` points either way (`SAL.ACL.07`).
    pub fn needs_manager(&self, limit: i64) -> bool {
        (self.closes_account && self.points != 0) || self.points.abs() > limit
    }

    /// The adjustments of `account`, newest first.
    pub async fn of_account(db: &Db, account: i64) -> Result<Vec<PointsAdjustment>> {
        PointsAdjustment::query()
            .where_(PointsAdjustment::ACCOUNT_ID.eq(account))
            .order_by_desc(PointsAdjustment::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let names: Vec<_> = AdjustmentStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(names, ["draft", "pending_approval", "applied", "rejected"]);
    }

    #[test]
    fn above_500_points_or_closing_with_points_needs_a_manager() {
        let plus = |points: i64| PointsAdjustment {
            points,
            ..PointsAdjustment::default()
        };
        assert!(!plus(400).needs_manager(500));
        assert!(!plus(500).needs_manager(500));
        assert!(plus(501).needs_manager(500));
        assert!(plus(-900).needs_manager(500));
        assert!(!plus(-500).needs_manager(500));
        let closing = PointsAdjustment {
            points: -50,
            closes_account: true,
            ..PointsAdjustment::default()
        };
        assert!(closing.needs_manager(500));
        let empty = PointsAdjustment {
            points: 0,
            closes_account: true,
            ..PointsAdjustment::default()
        };
        assert!(!empty.needs_manager(500));
    }
}
