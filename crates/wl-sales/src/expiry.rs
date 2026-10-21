//! Expiring points and reminding members (wl-sales §4.7, §5.9 step 5,
//! `SAL.RUL.08`; BL-SAL-24): the daily task `sales:points_expiry` at 02:00
//! Asia/Dubai, also callable as [`run_daily`] and as a command with
//! `--date YYYY-MM-DD`.
//!
//! For each account, in order:
//!
//! 1. **Reminder.** One message per member per run, summing the lots that
//!    expire within the reminder days (30) and were not reminded yet; the
//!    lots are marked so a lot is reminded once. The type is a *reminder*
//!    (the member's opt-out is respected by `wl-notify`) with the give-up
//!    reaction `none`.
//! 2. **Expiry.** The rest of each lot whose last day is before the run date
//!    expires with an `expire` movement and [`PointsExpired`] (a lot can be
//!    redeemed through its last day, so a lot ending 30 Nov expires in the run
//!    of 1 Dec). Only the unused rest expires; a lot that is empty, or an
//!    account with a zero or negative balance, has nothing to expire; points
//!    held for an open sale are not expired until the hold ends.
//!
//! Decisions:
//! - A reminder goes in the first run on or after the day the lot comes
//!   within the reminder days, not only on the exact day, so a missed run
//!   does not lose it; lots earned with less validity than the days (points
//!   given back, 30 days) are reminded in the next run.
//! - A reminder that gave up does not stop the lot expiring: the account only
//!   shows it ([`LoyaltyAccount::expiry_reminder_failed_at`]).
//! - Lapsed holds are released first, so a hold that ended no longer shields
//!   its points.
//! - Each account is handled on its own; a failure on one is printed and
//!   counted and the run goes on. A second run finds nothing to do.

use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::{DateTime, now};
use renox::prelude::*;
use wl_kernel::documents::today;
use wl_kernel::notify::MessageGaveUp;
use wl_kernel::sales::PointsExpired;

use crate::config::config_of;
use crate::loyalty_account::LoyaltyAccount;
use crate::loyalty_points::{CAUSE_EXPIRE, PointsKind, PointsMovement};
use crate::loyalty_programme::LoyaltyProgramme;
use crate::notices;
use crate::redeem;
use crate::redemption::Redemption;

/// The daily scheduled task, and the command, that run [`run_daily`].
pub const TASK: &str = "sales:points_expiry";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "02:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Holds released because their time passed.
    pub released: usize,
    /// Members sent a reminder.
    pub reminded: usize,
    /// Lots (or the rest of them) that expired.
    pub expired_lots: usize,
    /// The points that expired.
    pub expired_points: i64,
    /// Accounts that failed.
    pub failed: usize,
}

/// What one account came to.
#[derive(Debug, Clone, Copy, Default)]
struct Done {
    reminded: bool,
    lots: usize,
    points: i64,
}

/// Runs the pass for the run date `on`, as of the instant `at`.
pub async fn run_daily(state: &AppState, on: NaiveDate, at: DateTime) -> Result<Report> {
    let mut report = Report {
        released: redeem::release_lapsed(state, at).await?,
        ..Report::default()
    };
    let accounts = LoyaltyAccount::query()
        .order_by(LoyaltyAccount::ID)
        .get(&state.db)
        .await?;
    for account in accounts {
        match one_account(state, account.id, on, at).await {
            Ok(done) => {
                report.reminded += usize::from(done.reminded);
                report.expired_lots += done.lots;
                report.expired_points += done.points;
            }
            Err(error) => {
                eprintln!("{TASK}: account {} failed: {error}", account.id);
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

/// The reminder and the expiry of the account `id`.
async fn one_account(state: &AppState, id: i64, on: NaiveDate, at: DateTime) -> Result<Done> {
    let mut done = Done::default();
    let Some(account) = LoyaltyAccount::find(&state.db, id).await? else {
        return Ok(done);
    };
    let Some(programme) = LoyaltyProgramme::find(&state.db, account.programme_id).await? else {
        return Ok(done);
    };
    if account.is_active() && !account.paused {
        done.reminded = remind(state, &account, &programme, on).await?;
    }
    let (lots, points) = expire(state, id, &programme, on, at).await?;
    done.lots = lots;
    done.points = points;
    Ok(done)
}

/// Reminds the member of the lots that come within the reminder days and were
/// not reminded yet. Returns whether a message was sent.
async fn remind(
    state: &AppState,
    account: &LoyaltyAccount,
    programme: &LoyaltyProgramme,
    on: NaiveDate,
) -> Result<bool> {
    let horizon = on + TimeDelta::days(config_of(state).expiry_reminder_days);
    let lots: Vec<PointsMovement> = PointsMovement::lots(&state.db, account.id, Some(on))
        .await?
        .into_iter()
        .filter(|l| l.reminded_on.is_none() && l.lot_expires_on.is_some_and(|d| d <= horizon))
        .collect();
    let Some(first) = lots.first().and_then(|l| l.lot_expires_on) else {
        return Ok(false);
    };
    let points: i64 = lots.iter().map(PointsMovement::lot_remaining).sum();
    // The newest reminder may still give up: the account shows it from then.
    let none: Option<DateTime> = None;
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::ID.eq(account.id))
        .update(&state.db, &[("expiry_reminder_failed_at", &none)])
        .await?;
    notices::loyalty_expiry(state, account, points, points * programme.point_value, first).await?;
    for lot in &lots {
        PointsMovement::query()
            .where_(PointsMovement::ID.eq(lot.id))
            .update(&state.db, &[("reminded_on", &Some(on))])
            .await?;
    }
    Ok(true)
}

/// Expires the rest of every lot of the account `id` whose last day is before
/// `on`, leaving what open sales hold. Returns the lots and the points.
async fn expire(
    state: &AppState,
    id: i64,
    programme: &LoyaltyProgramme,
    on: NaiveDate,
    at: DateTime,
) -> Result<(usize, i64)> {
    let Some(account) = LoyaltyAccount::find(&state.db, id).await? else {
        return Ok((0, 0));
    };
    let lots = PointsMovement::lots(&state.db, id, None).await?;
    let mut shield: i64 = Redemption::held_of_account(&state.db, id)
        .await?
        .iter()
        .filter(|r| r.holds_at(at))
        .map(|r| r.points)
        .sum();
    let mut room = account.points_balance.max(0);
    let mut plan: Vec<(PointsMovement, i64)> = Vec::new();
    for lot in lots {
        let remaining = lot.lot_remaining();
        let shielded = shield.min(remaining);
        shield -= shielded;
        if !lot.lot_expires_on.is_some_and(|last| last < on) {
            continue;
        }
        let amount = (remaining - shielded).min(room);
        if amount > 0 {
            room -= amount;
            plan.push((lot, amount));
        }
    }
    if plan.is_empty() {
        return Ok((0, 0));
    }
    let subject = account.subject();
    let mut tx = state.db.begin().await?;
    for (lot, amount) in &plan {
        PointsMovement {
            account_id: account.id,
            kind: PointsKind::Expire,
            points: -amount,
            value: amount * programme.point_value,
            legal_entity_id: lot.legal_entity_id,
            source_type: subject.subject_type.clone(),
            source_id: subject.subject_id,
            earning_id: lot.earning_id,
            cause: CAUSE_EXPIRE.to_owned(),
            lot_id: Some(lot.id),
            lot_expires_on: lot.lot_expires_on,
            ..PointsMovement::default()
        }
        .insert(&mut tx)
        .await?;
        PointsMovement::query()
            .where_(PointsMovement::ID.eq(lot.id))
            .increment(&mut tx, "consumed", *amount)
            .await?;
        LoyaltyAccount::query()
            .where_(LoyaltyAccount::ID.eq(account.id))
            .increment(&mut tx, "points_balance", -amount)
            .await?;
    }
    tx.commit().await?;
    for (lot, amount) in &plan {
        if let Some(last) = lot.lot_expires_on {
            state
                .emit(PointsExpired {
                    account: account.account_id(),
                    party: account.party(),
                    points: *amount,
                    value: amount * programme.point_value,
                    lot_expired_on: last,
                })
                .await?;
        }
    }
    Ok((plan.len(), plan.iter().map(|(_, amount)| amount).sum()))
}

/// A reminder gave up: the account shows *reminder not delivered*. The lot
/// still expires on its date.
async fn reminder_gave_up(state: &AppState, event: &MessageGaveUp) -> Result {
    if event.type_key != notices::LOYALTY_EXPIRY
        || event.subject.subject_type != wl_kernel::sales::LOYALTY_ACCOUNT_SUBJECT_TYPE
    {
        return Ok(());
    }
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::ID.eq(event.subject.subject_id))
        .update(
            &state.db,
            &[
                ("expiry_reminder_failed_at", &Some(event.gave_up_at)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Parses `--date`, else `default`.
fn date_arg(value: Option<&str>, default: NaiveDate) -> Result<NaiveDate> {
    match value {
        None => Ok(default),
        Some(text) => text.parse().map_err(|_| {
            Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("--date must be YYYY-MM-DD, got `{text}`"),
            )
        }),
    }
}

/// Registers the daily [`TASK`], the command of the same name and the
/// listener of the reminder's give-up.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run_daily(&state, today(), now()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Remind members of points about to expire and expire the rest of lots past their last day \
         (--date YYYY-MM-DD runs another day)",
        |args, state: AppState| async move {
            let on = date_arg(args.value("--date"), today())?;
            let report = run_daily(&state, on, now()).await?;
            println!(
                "{} members reminded, {} lots expired ({} points), {} holds released, {} failed.",
                report.reminded,
                report.expired_lots,
                report.expired_points,
                report.released,
                report.failed
            );
            Ok(())
        },
    );
    app.listen(|event: MessageGaveUp, state: AppState| async move {
        reminder_gave_up(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_is_at_two_in_the_morning_dubai() {
        assert_eq!(TASK, "sales:points_expiry");
        assert_eq!((TASK_AT, TASK_ZONE), ("02:00", "Asia/Dubai"));
    }

    #[test]
    fn a_date_argument_is_parsed_or_refused() {
        let default = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
        assert_eq!(date_arg(None, default).unwrap(), default);
        assert_eq!(
            date_arg(Some("2026-11-30"), default).unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 30).unwrap()
        );
        assert!(date_arg(Some("30/11"), default).is_err());
    }
}
