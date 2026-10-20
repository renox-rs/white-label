//! `FIN.RUL.21`, a failed refund is retried once, otherwise the accountant
//! changes its method (wl-payments §5.5a step 2, §6, §7, issue #100).
//!
//! When a refund goes to `failed`:
//!
//! - a **gateway** refund with retries below the maximum
//!   ([`PaymentsConfig::refund_max_retries`](crate::config::PaymentsConfig),
//!   1) waits `refund_retry_hours` (24) and is then retried by the system
//!   ([`run`], every fifteen minutes): `retry_by_system` and a new gateway
//!   attempt;
//! - any other failed refund (a transfer returned by the bank, a gateway
//!   refund already retried, a retry the gateway refused or the source has no
//!   room for) gets **one task for the accountant** to change the method
//!   ([`FollowupTask::change_method`], with a notice), at once. The task
//!   closes itself when the refund leaves `failed`.
//!
//! The time of a failure is the one in the refund's history. A scheduled retry
//! that finds the refund no longer `failed` (the accountant changed the
//! method first, or cancelled it) does nothing, and a manual retry counts
//! toward the maximum like the automatic one ([`crate::refund_recover`]).

use renox::chrono::TimeDelta;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{TransitionError, Transitioned};
use wl_kernel::payments::PaymentMethod;

use crate::config_of;
use crate::followup::FollowupTask;
use crate::notices;
use crate::refund::{Refund, RefundStatus};
use crate::refund_fail::{FAIL, FAIL_BY_SYSTEM};
use crate::refund_recover::{failed_at, retries_of, retry_by_system};

/// The rule id.
pub const RULE: &str = "FIN.RUL.21";

/// The scheduled task and command.
pub const TASK: &str = "payments:refund_retry";

/// Whether the failed `refund`, retried `retries` times, is retried by the
/// system: a gateway refund below `max` retries.
pub fn retry_eligible(refund: &Refund, retries: i64, max: i64) -> bool {
    refund.status == RefundStatus::Failed
        && refund.method == Some(PaymentMethod::Gateway)
        && retries < max
}

/// Whether a retry waiting since `failed_at` is due at `now`, `hours`
/// later.
pub fn retry_due(failed_at: DateTime, hours: i64, now: DateTime) -> bool {
    failed_at + TimeDelta::hours(hours) <= now
}

/// Creates the accountant's task to change the method of the failed
/// `refund`, unless one is open, and tells the accountants. Returns whether
/// a task was made.
async fn create_task(state: &AppState, refund: &Refund) -> Result<bool> {
    let task = FollowupTask::change_method(refund);
    match FollowupTask::create_once(&state.db, task).await? {
        Some(task) => {
            notices::change_method_task(state, refund, &task).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// After a refund failed: the task for the accountant unless the system will
/// retry it. Does nothing for a refund that is not `failed` any more.
///
/// # Errors
///
/// The database's errors, or a notice that failed.
pub async fn after_failure(state: &AppState, refund_id: i64) -> Result<bool> {
    let Some(refund) = Refund::find(&state.db, refund_id).await? else {
        return Ok(false);
    };
    if refund.status != RefundStatus::Failed {
        return Ok(false);
    }
    let retries = retries_of(&state.db, refund.id).await?;
    if retry_eligible(&refund, retries, config_of(state).refund_max_retries) {
        return Ok(false);
    }
    create_task(state, &refund).await
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Refunds retried.
    pub retried: usize,
    /// Tasks made for a retry that could not be made.
    pub tasks: usize,
}

/// Retries every failed gateway refund whose wait is over at `now`
/// (`FIN.RUL.21`). A retry that cannot be made (the gateway refuses, the
/// source has no room) gives the accountant the task instead.
///
/// # Errors
///
/// The database's errors.
pub async fn run(state: &AppState, now: DateTime) -> Result<Report> {
    let config = config_of(state);
    let failed = Refund::query()
        .where_(Refund::STATUS.eq(RefundStatus::Failed))
        .where_(Refund::METHOD.eq(PaymentMethod::Gateway))
        .order_by(Refund::ID)
        .get(&state.db)
        .await?;
    let mut report = Report::default();
    for refund in failed {
        let retries = retries_of(&state.db, refund.id).await?;
        if !retry_eligible(&refund, retries, config.refund_max_retries) {
            continue;
        }
        let Some(at) = failed_at(&state.db, refund.id).await? else {
            continue;
        };
        if !retry_due(at, config.refund_retry_hours, now) {
            continue;
        }
        match retry_by_system(state, &refund).await {
            Ok(_) => report.retried += 1,
            // The accountant got there first.
            Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => {}
            Err(_) => {
                if create_task(state, &refund).await? {
                    report.tasks += 1;
                }
            }
        }
    }
    Ok(report)
}

/// Registers the listener that makes the task after a failure, and the
/// scheduled task and its command.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Refund>, state: AppState| async move {
        if event.transition != FAIL && event.transition != FAIL_BY_SYSTEM {
            return Ok(());
        }
        after_failure(&state, event.key).await.map(|_| ())
    });
    app.schedule()
        .every_minutes(15, TASK, |state: AppState| async move {
            run(&state, renox::db::now()).await.map(|_| ())
        });
    app.command(
        TASK,
        "Retry the failed gateway refunds whose wait is over (FIN.RUL.21)",
        |_args, state: AppState| async move {
            let report = run(&state, renox::db::now()).await?;
            println!(
                "{} refunds retried, {} tasks made.",
                report.retried, report.tasks
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::TimeZone;
    use renox::chrono::Utc;

    fn failed(method: Option<PaymentMethod>) -> Refund {
        Refund {
            method,
            status: RefundStatus::Failed,
            ..Default::default()
        }
    }

    #[test]
    fn only_a_gateway_refund_below_the_maximum_is_retried_by_the_system() {
        let gateway = failed(Some(PaymentMethod::Gateway));
        assert!(retry_eligible(&gateway, 0, 1));
        assert!(!retry_eligible(&gateway, 1, 1), "already retried once");
        assert!(!retry_eligible(&gateway, 0, 0), "retries off");
        assert!(!retry_eligible(
            &failed(Some(PaymentMethod::Transfer)),
            0,
            1
        ));
        assert!(!retry_eligible(&failed(None), 0, 1));
        let paid = Refund {
            status: RefundStatus::Approved,
            ..gateway
        };
        assert!(!retry_eligible(&paid, 0, 1), "no longer failed");
    }

    #[test]
    fn the_retry_is_due_n_hours_after_the_failure() {
        let at = Utc.with_ymd_and_hms(2027, 1, 10, 10, 0, 0).unwrap();
        let before = Utc.with_ymd_and_hms(2027, 1, 11, 9, 59, 59).unwrap();
        let exactly = Utc.with_ymd_and_hms(2027, 1, 11, 10, 0, 0).unwrap();
        assert!(!retry_due(at, 24, before));
        assert!(retry_due(at, 24, exactly));
        assert!(retry_due(at, 24, exactly + TimeDelta::hours(5)));
    }
}
