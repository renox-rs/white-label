//! A member leaves the programme (wl-sales §4.7 *Member asks to leave*;
//! BL-SAL-26): never refused, no approval. The member is shown the points
//! that will be forfeited and confirms (staff record the confirmation of a
//! request made at the counter); the points go with an `adjust` movement
//! that records *member request* and who recorded the confirmation, and the
//! account closes. The movements stay for the liability history.
//!
//! Decisions:
//! - Points held for a sale in progress are released first: a closed account
//!   spends nothing.
//! - A per-visit programme has no points to forfeit: the account closes.
//! - Closing at the staff's initiative with points left is the approval flow
//!   of [`crate::adjustments::close_account`] (`SAL.ACL.07`), not this one.
//! - Erasure (`wl-parties` §10) closes the account with the reason *erased*
//!   and forfeits through the same movement; the event that triggers it does
//!   not exist yet, so [`forfeit_and_close`] is what it will call.

use renox::db::now;
use renox::prelude::*;
use wl_kernel::sales::PointsAdjusted;

use crate::loyalty_account::{AccountStatus, LoyaltyAccount};
use crate::loyalty_points::{CAUSE_ADJUST, PointsKind, PointsMovement};
use crate::loyalty_programme::{EarnKind, LoyaltyProgramme};
use crate::notices;
use crate::redeem;
use crate::redemption_machine::consume_lots;
use crate::texts::english;

/// The member asked at the counter and staff recorded the confirmation.
pub const VIA_COUNTER: &str = "counter";
/// The member asked in the portal and confirmed there.
pub const VIA_PORTAL: &str = "portal";

/// What closing the account takes with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Forfeit {
    /// The points that will be forfeited.
    pub points: i64,
    /// Their value in fils.
    pub value: i64,
}

/// What the member loses by leaving: the points in the lots, never more than
/// the balance; none for a per-visit programme.
///
/// # Errors
///
/// The database's errors.
pub async fn forfeit_of(db: &Db, account: &LoyaltyAccount) -> Result<Forfeit> {
    let Some(programme) = LoyaltyProgramme::find(db, account.programme_id).await? else {
        return Ok(Forfeit::default());
    };
    if programme.earn_kind != EarnKind::PerAed {
        return Ok(Forfeit::default());
    }
    let in_lots: i64 = PointsMovement::lots(db, account.id, None)
        .await?
        .iter()
        .map(PointsMovement::lot_remaining)
        .sum();
    let points = in_lots.min(account.points_balance.max(0));
    Ok(Forfeit {
        points,
        value: points * programme.point_value,
    })
}

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// Forfeits what is left of the account `id` with an `adjust` of `reason`
/// (recorded by `by`) and closes it with the same reason. The member is told.
///
/// # Errors
///
/// 422 when the account is closed already.
pub async fn forfeit_and_close(
    state: &AppState,
    id: i64,
    by: Option<i64>,
    reason: &str,
) -> Result<LoyaltyAccount> {
    let account = LoyaltyAccount::find_or_404(&state.db, id).await?;
    if account.status != AccountStatus::Active {
        return Err(refused("wl_sales.closure.problem.closed"));
    }
    redeem::release_account_holds(state, account.id, "account_closed").await?;
    let account = LoyaltyAccount::find_or_404(&state.db, id).await?;
    let programme = LoyaltyProgramme::find_or_404(&state.db, account.programme_id).await?;
    let forfeit = forfeit_of(&state.db, &account).await?;
    let subject = account.subject();
    let mut tx = state.db.begin().await?;
    if forfeit.points > 0 {
        let template = PointsMovement {
            account_id: account.id,
            kind: PointsKind::Adjust,
            source_type: subject.subject_type.clone(),
            source_id: subject.subject_id,
            cause: CAUSE_ADJUST.to_owned(),
            reason: Some(reason.to_owned()),
            user_id: by,
            ..PointsMovement::default()
        };
        consume_lots(
            &mut tx,
            &template,
            forfeit.points,
            None,
            programme.point_value,
        )
        .await?;
    }
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::ID.eq(account.id))
        .update(
            &mut tx,
            &[
                ("status", &AccountStatus::Closed),
                ("closed_reason", &Some(reason.to_owned())),
                ("closed_at", &Some(now())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    tx.commit().await?;
    let closed = LoyaltyAccount::find_or_404(&state.db, id).await?;
    if forfeit.points > 0 {
        state
            .emit(PointsAdjusted {
                account: closed.account_id(),
                party: closed.party(),
                points: -forfeit.points,
                value: -forfeit.value,
                reason: reason.to_owned(),
            })
            .await?;
    }
    notices::loyalty_closed(state, &closed, forfeit.points).await?;
    Ok(closed)
}

/// The member of account `id` asked to leave and confirmed (`via` is
/// [`VIA_COUNTER`] with the staff user `by` who recorded the confirmation, or
/// [`VIA_PORTAL`] with the member's own user). Never refused for any other
/// reason than the account being closed already: no approval, and a paused
/// account (the party is on hold) may leave too.
///
/// # Errors
///
/// 422 when the account is closed already.
pub async fn leave(
    state: &AppState,
    id: i64,
    by: Option<i64>,
    via: &str,
) -> Result<LoyaltyAccount> {
    let reason = english("wl_sales.closure.member_request", &[("via", &via)]);
    forfeit_and_close(state, id, by, &reason).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reason_names_the_member_and_the_way_it_was_recorded() {
        let reason = english("wl_sales.closure.member_request", &[("via", &VIA_COUNTER)]);
        assert!(reason.contains("Member request"), "{reason}");
        assert!(reason.contains("counter"), "{reason}");
    }
}
