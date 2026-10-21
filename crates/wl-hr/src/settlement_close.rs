//! Closing and cancelling a final settlement (wl-hr §4 *Final settlement*:
//! `close`, `cancel`; §5.4; §7 *Settlement closes without dispute after*;
//! issue #214).
//!
//! - **`close`** (paid -> closed, system): N days after `pay` with no dispute
//!   (default 30 calendar days, Asia/Dubai, [`crate::HrConfig::settlement_close_days`]),
//!   or as soon as the acknowledgement is signed ([`acknowledge`]). The end of
//!   the period is kept on the row (`closes_on`, set after `pay` by
//!   [`crate::settlement_events`]) so the guard reads it without the settings.
//!   The daily job [`TASK`] runs it ([`close_due`]). A dispute leaves `paid`, so
//!   a disputed settlement never closes by the clock; a later claim after
//!   `closed` is handled outside the machine, on a labour-case ticket on the
//!   employee.
//! - The **signed acknowledgement** is a receipt of payment, not a waiver:
//!   signing closes the settlement in the system only and does not decide
//!   whether the worker may still claim. The payee *estate* has no
//!   acknowledgement; it closes at N days.
//! - **`cancel`** (draft, approved -> cancelled): `settlements.prepare`, or the
//!   system for `withdraw_notice` and `reinstate` ([`cancel_open`], run as
//!   `cancel_by_system`). Refused when any payment is recorded against it.

use renox::chrono::{Duration, NaiveDate};
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionError};
use wl_kernel::documents::today;

use crate::config::config_of;
use crate::permissions;
use crate::settlement::{
    CANCEL, CANCEL_BY_SYSTEM, CLOSE, FinalSettlement, PAYEE_ESTATE, SettlementStatus, refuse,
};
use crate::settlement_payment::paid_total_in;
use crate::texts::english;

/// The daily job, and the command, that run [`close_due`].
pub const TASK: &str = "hr:settlements-close";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "00:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// `close`: the period to dispute ended, or the acknowledgement is signed.
pub(crate) fn close() -> Transition<FinalSettlement> {
    Transition::new(CLOSE)
        .label("Close")
        .system()
        .from([SettlementStatus::Paid])
        .to(SettlementStatus::Closed)
        .guard("period_ended_or_signed", |s: &FinalSettlement, _ctx| {
            let day = today();
            let due =
                s.acknowledgement_signed_on.is_some() || s.closes_on.is_some_and(|d| d <= day);
            Box::pin(async move {
                if due {
                    Ok(())
                } else {
                    Err(english("wl_hr.settlement.close_not_due", &[]))
                }
            })
        })
        .effect(|s: &FinalSettlement, _ctx, tx| {
            let id = s.id;
            Box::pin(async move {
                FinalSettlement::query()
                    .where_(FinalSettlement::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[("closed_on", &Some(today())), ("updated_at", &now())],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `cancel` by a user, or `cancel_by_system` (`by_system`): nothing may be
/// paid against the settlement.
pub(crate) fn cancel(by_system: bool) -> Transition<FinalSettlement> {
    let transition: Transition<FinalSettlement> = if by_system {
        Transition::new(CANCEL_BY_SYSTEM).system()
    } else {
        Transition::new(CANCEL)
            .tone("danger")
            .permission(permissions::SETTLEMENTS_PREPARE)
    };
    transition
        .label("Cancel")
        .from([SettlementStatus::Draft, SettlementStatus::Approved])
        .to(SettlementStatus::Cancelled)
        .effect(|s: &FinalSettlement, _ctx, tx| {
            let id = s.id;
            Box::pin(async move {
                if paid_total_in(tx, id).await? > 0 {
                    return Err(refuse("wl_hr.settlement.cancel_paid"));
                }
                Ok(())
            })
        })
}

/// Cancels the open settlement (`draft` or `approved`) of `employee_id`, as the
/// system: what `withdraw_notice` and `reinstate` call. Returns the settlement,
/// or `None` when the employee has none open.
///
/// # Errors
///
/// 422 when a payment is recorded against it.
pub async fn cancel_open(state: &AppState, employee_id: i64) -> Result<Option<FinalSettlement>> {
    let open = {
        let mut tx = state.db.begin().await?;
        FinalSettlement::open_in(&mut tx, employee_id).await?
    };
    let Some(open) = open else {
        return Ok(None);
    };
    let _outcome = open
        .transition(CANCEL_BY_SYSTEM)
        .by_system()
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    FinalSettlement::find(&state.db, open.id).await
}

/// The signed acknowledgement of settlement `id` arrived (`DOC.SIG.01`): it
/// is recorded for `signed_on` (today when `None`) and the settlement closes
/// at once. Run by wl-documents when the signature completes, or by HR from the
/// page.
///
/// # Errors
///
/// 422 when the settlement is not `paid`, or the payee is the estate (no
/// acknowledgement is asked of it).
pub async fn acknowledge(
    state: &AppState,
    id: i64,
    signed_on: Option<NaiveDate>,
) -> Result<FinalSettlement> {
    let settlement = FinalSettlement::find_or_404(&state.db, id).await?;
    if settlement.status != SettlementStatus::Paid {
        return Err(refuse("wl_hr.settlement.acknowledge_state"));
    }
    if settlement.payee == PAYEE_ESTATE {
        return Err(refuse("wl_hr.settlement.acknowledge_not_asked"));
    }
    FinalSettlement::query()
        .where_(FinalSettlement::ID.eq(id))
        .update(
            &state.db,
            &[
                (
                    "acknowledgement_signed_on",
                    &Some(signed_on.unwrap_or_else(today)),
                ),
                ("updated_at", &now()),
            ],
        )
        .await?;
    let settlement = FinalSettlement::find_or_404(&state.db, id).await?;
    let _outcome = settlement
        .transition(CLOSE)
        .by_system()
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    FinalSettlement::find_or_404(&state.db, id).await
}

/// What a run of [`close_due`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Settlements closed.
    pub closed: usize,
    /// Paid settlements not due yet.
    pub waiting: usize,
}

/// The end of the period to dispute for a settlement paid on `paid_on`.
pub fn closes_on(paid_on: NaiveDate, days: i64) -> NaiveDate {
    paid_on + Duration::days(days)
}

/// Closes every `paid` settlement whose period to dispute has ended on or
/// before `day`, or whose acknowledgement is signed. Idempotent. A paid
/// settlement without an end date (paid before the date was kept) gets one
/// from its payment day.
pub async fn close_due(state: &AppState, day: NaiveDate) -> Result<Report> {
    let days = config_of(state).settlement_close_days;
    let paid = FinalSettlement::query()
        .where_(FinalSettlement::STATUS.eq(SettlementStatus::Paid))
        .order_by(FinalSettlement::ID)
        .get(&state.db)
        .await?;
    let mut report = Report::default();
    for settlement in paid {
        let ends = settlement
            .closes_on
            .or_else(|| settlement.paid_on.map(|paid_on| closes_on(paid_on, days)));
        let due = settlement.acknowledgement_signed_on.is_some() || ends.is_some_and(|e| e <= day);
        if !due {
            report.waiting += 1;
            continue;
        }
        if settlement.closes_on.is_none() {
            FinalSettlement::query()
                .where_(FinalSettlement::ID.eq(settlement.id))
                .update(&state.db, &[("closes_on", &ends)])
                .await?;
        }
        let settlement = FinalSettlement::find_or_404(&state.db, settlement.id).await?;
        match settlement.transition(CLOSE).by_system().run(state).await {
            Ok(_outcome) => report.closed += 1,
            Err(TransitionError::Guard(_) | TransitionError::NotFrom(_)) => report.waiting += 1,
            Err(error) => return Err(error.into_error()),
        }
    }
    Ok(report)
}

/// Registers the daily [`TASK`] and the command of the same name.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            close_due(&state, today()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Close the paid final settlements whose period to dispute has ended \
         (--date YYYY-MM-DD checks another day)",
        |args, state: AppState| async move {
            let on = match args.value("--date") {
                None => today(),
                Some(text) => text.parse().map_err(|_| {
                    Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("--date must be YYYY-MM-DD, got `{text}`"),
                    )
                })?,
            };
            let report = close_due(&state, on).await?;
            println!(
                "{} settlements closed, {} waiting.",
                report.closed, report.waiting
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_period_ends_n_calendar_days_after_payment() {
        let paid = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        assert_eq!(
            closes_on(paid, 30),
            NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()
        );
    }
}
