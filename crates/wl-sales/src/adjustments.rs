//! Manual points adjustments and the closing of an account by staff
//! (wl-sales §4.7, §5.9 step 5, `SAL.ACL.07`; BL-SAL-24).
//!
//! [`request`] saves an adjustment and runs exactly one transition after
//! reading the limit (`SAL.ACL.07`, 500 points): `apply` at or below it,
//! `submit_over_limit` above it, which asks the kernel for a manager other
//! than the requester. [`close_account`] does the same for an account staff
//! close: with points left it always waits for a manager, and the approval
//! writes an `adjust` of what is left and closes the account. A rejected
//! adjustment writes nothing and its requester is told; a returned one is
//! back in `draft` for [`resubmit`].
//!
//! The kernel's [`PointsAdjusted`] follows the commit of a written
//! adjustment, in a listener on the machine's `Transitioned`.

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::approvals::{self, ApprovalDecided, ApprovalEnd};
use wl_kernel::sales::PointsAdjusted;

use crate::adjustment::{ADJUSTMENT_SUBJECT_TYPE, AdjustmentStatus, PointsAdjustment};
use crate::adjustment_machine::{
    APPLY, AdjustInput, REJECT, SUBMIT_OVER_LIMIT, SUBMIT_OVER_LIMIT_APPROVED,
};
use crate::config::config_of;
use crate::loyalty_account::{AccountStatus, LoyaltyAccount};
use crate::loyalty_programme::LoyaltyProgramme;
use crate::notices;
use crate::texts::english;

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// Checks what does not need the database: a reason and points other than
/// zero.
fn check(points: i64, reason: &str) -> Result {
    if reason.trim().is_empty() {
        return Err(refused("wl_sales.adjust.problem.reason"));
    }
    if points == 0 {
        return Err(refused("wl_sales.adjust.problem.points"));
    }
    Ok(())
}

/// Checks the account of a new or changed adjustment: open, and holding the
/// points a negative adjustment takes off.
fn check_account(account: &LoyaltyAccount, points: i64) -> Result {
    if account.status != AccountStatus::Active {
        return Err(refused("wl_sales.adjust.problem.closed"));
    }
    if points < 0 && -points > account.points_balance.max(0) {
        return Err(refused("wl_sales.adjust.problem.more_than_held"));
    }
    Ok(())
}

/// Runs the one transition the limit decides on the saved `adjustment` inside
/// `tx`, then, after the commit, announces the approval request and emits.
async fn submit(state: &AppState, adjustment: PointsAdjustment) -> Result<PointsAdjustment> {
    let id = adjustment.id;
    let limit = config_of(state).adjustment_limit;
    let name = if adjustment.needs_manager(limit) {
        SUBMIT_OVER_LIMIT
    } else {
        APPLY
    };
    let mut tx = state.db.begin().await?;
    let outcome = adjustment
        .transition(name)
        .by_system()
        .input(&AdjustInput { limit })
        .run_in(&mut tx)
        .await
        .map_err(|e| e.into_error())?;
    tx.commit().await?;
    approvals::announce(state, &outcome).await?;
    outcome.emit(state).await?;
    PointsAdjustment::find_or_404(&state.db, id).await
}

/// Saves an adjustment of `points` (signed) of the account `account` by the
/// user `by`, with `reason`, and applies it or sends it to a manager.
///
/// # Errors
///
/// 422 when the reason is blank, the points are zero, the account is closed,
/// or a negative adjustment takes off more than the member has.
pub async fn request(
    state: &AppState,
    by: i64,
    account: i64,
    points: i64,
    reason: &str,
) -> Result<PointsAdjustment> {
    check(points, reason)?;
    let held = LoyaltyAccount::find_or_404(&state.db, account).await?;
    check_account(&held, points)?;
    let saved = PointsAdjustment::create(
        &state.db,
        PointsAdjustment {
            account_id: account,
            points,
            reason: reason.trim().to_owned(),
            requested_by: by,
            ..PointsAdjustment::default()
        },
    )
    .await?;
    submit_or_discard(state, saved).await
}

/// Submits `saved`; when the transition fails (a manager is missing, the
/// points are gone) the saved row is removed so nothing half-done stays.
async fn submit_or_discard(
    state: &AppState,
    saved: PointsAdjustment,
) -> Result<PointsAdjustment> {
    let id = saved.id;
    match submit(state, saved).await {
        Ok(done) => Ok(done),
        Err(error) => {
            PointsAdjustment::query()
                .where_(PointsAdjustment::ID.eq(id))
                .where_(PointsAdjustment::STATUS.eq(AdjustmentStatus::Draft))
                .delete(&state.db)
                .await?;
            Err(error)
        }
    }
}

/// Staff close the account `account` with `reason`: points left need a
/// manager other than `by`, and the approval writes an `adjust` of them and
/// closes the account; with none left it closes at once.
///
/// # Errors
///
/// 422 when the reason is blank or the account is closed already.
pub async fn close_account(
    state: &AppState,
    by: i64,
    account: i64,
    reason: &str,
) -> Result<PointsAdjustment> {
    if reason.trim().is_empty() {
        return Err(refused("wl_sales.adjust.problem.reason"));
    }
    let held = LoyaltyAccount::find_or_404(&state.db, account).await?;
    check_account(&held, 0)?;
    let saved = PointsAdjustment::create(
        &state.db,
        PointsAdjustment {
            account_id: account,
            points: -held.points_balance.max(0),
            reason: reason.trim().to_owned(),
            closes_account: true,
            requested_by: by,
            ..PointsAdjustment::default()
        },
    )
    .await?;
    submit_or_discard(state, saved).await
}

/// Changes the returned adjustment `id` (back in `draft`) and submits it
/// again.
///
/// # Errors
///
/// 422 when `by` did not request it, it is not in `draft`, or as
/// [`request`].
pub async fn resubmit(
    state: &AppState,
    by: i64,
    id: i64,
    points: i64,
    reason: &str,
) -> Result<PointsAdjustment> {
    let adjustment = PointsAdjustment::find_or_404(&state.db, id).await?;
    if adjustment.requested_by != by {
        return Err(refused("wl_sales.adjust.problem.not_requester"));
    }
    if adjustment.status != AdjustmentStatus::Draft {
        return Err(refused("wl_sales.adjust.problem.not_draft"));
    }
    let points = if adjustment.closes_account {
        adjustment.points
    } else {
        points
    };
    check(points, reason)?;
    let account = LoyaltyAccount::find_or_404(&state.db, adjustment.account_id).await?;
    check_account(&account, points)?;
    PointsAdjustment::query()
        .where_(PointsAdjustment::ID.eq(id))
        .update(
            &state.db,
            &[
                ("points", &points),
                ("reason", &reason.trim().to_owned()),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    let changed = PointsAdjustment::find_or_404(&state.db, id).await?;
    submit(state, changed).await
}

/// Emits [`PointsAdjusted`] after an adjustment was written.
async fn announce(state: &AppState, event: &Transitioned<PointsAdjustment>) -> Result {
    if event.transition != APPLY && event.transition != SUBMIT_OVER_LIMIT_APPROVED {
        return Ok(());
    }
    let Some(adjustment) = PointsAdjustment::find(&state.db, event.key).await? else {
        return Ok(());
    };
    let Some(account) = LoyaltyAccount::find(&state.db, adjustment.account_id).await? else {
        return Ok(());
    };
    let value = match LoyaltyProgramme::find(&state.db, account.programme_id).await? {
        Some(programme) => adjustment.points * programme.point_value,
        None => 0,
    };
    state
        .emit(PointsAdjusted {
            account: account.account_id(),
            party: account.party(),
            points: adjustment.points,
            value,
            reason: adjustment.reason.clone(),
        })
        .await
}

/// A request of ours ended: a rejection runs `reject` on the draft the
/// kernel sent the adjustment back to, and the requester is told of a
/// rejection or a return.
async fn decided(state: &AppState, event: &ApprovalDecided) -> Result {
    if event.subject.subject_type != ADJUSTMENT_SUBJECT_TYPE || event.state_returned_to.is_none() {
        return Ok(());
    }
    if event.decision != ApprovalEnd::Rejected && event.decision != ApprovalEnd::Returned {
        return Ok(());
    }
    let id = event.subject.subject_id;
    let Some(adjustment) = PointsAdjustment::find(&state.db, id).await? else {
        return Ok(());
    };
    if adjustment.status != AdjustmentStatus::Draft {
        return Ok(());
    }
    let mut key = notices::ADJUSTMENT_RETURNED;
    if event.decision == ApprovalEnd::Rejected {
        key = notices::ADJUSTMENT_REJECTED;
        let mut run = adjustment.transition(REJECT).by_system();
        if let Some(reason) = adjustment.outcome_reason.as_deref() {
            run = run.reason(reason);
        }
        let _outcome = run.run(state).await.map_err(|e| e.into_error())?;
    }
    let adjustment = PointsAdjustment::find_or_404(&state.db, id).await?;
    let Some(account) = LoyaltyAccount::find(&state.db, adjustment.account_id).await? else {
        return Ok(());
    };
    let reason = adjustment.outcome_reason.clone().unwrap_or_default();
    notices::adjustment_ended(state, &adjustment, &account.code, key, &reason).await
}

/// Registers the machine, the default policy and the listeners.
pub(crate) fn register(app: &mut Registry) {
    crate::adjustment_machine::subject_machine().register();
    renox::state_machine::register::<PointsAdjustment>();
    crate::policy::adjustment_policy().register();
    approvals::register_subject_link(ADJUSTMENT_SUBJECT_TYPE, |_id| {
        "/loyalty/adjustments".to_owned()
    });
    app.listen(
        |event: Transitioned<PointsAdjustment>, state: AppState| async move {
            announce(&state, &event).await
        },
    );
    app.listen(|event: ApprovalDecided, state: AppState| async move {
        decided(&state, &event).await
    });
}
