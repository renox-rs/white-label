//! Escalating a handover that is not handed over (wl-payments §5.5 step 4,
//! §6 `FIN.RUL.22`, §7, issue #114).
//!
//! A holder who does not come back at the end of the run, shift or day must not
//! leave cash unaccounted for. [`run`] is the rule, hourly:
//!
//! - **N hours** after the end of the holder's period (12): a task for the
//!   holder's manager to reach the holder (a ticket assigned to the manager,
//!   and a notice).
//! - **M hours** after the end (48, counted from the end of the period, not
//!   from the N task): the company admin is notified and the manager is asked
//!   to `count` at what was actually received, often nothing; the holder's
//!   presence is not needed and the count records *holder absent*
//!   ([`crate::cash_count::count`] with `holder_absent`). The full difference
//!   is a shortage; money owed to third parties (a 03 client's COD remittance)
//!   is not reduced.
//!
//! The end of the period: a run, the earlier of its return to the depot or
//! abort and its planned end (wl-delivery sets both through
//! `CashCustody::end_period` and the period's `ends_at`); a shift, its
//! planned end; a day, 23:59 Asia/Dubai. Run `close` is not used, since it
//! needs the handover counted first. The rule applies to a handover still
//! `open` with cash expected, and never to one whose period says a vertical
//! rule replaces it (`escalate` off: 03's `LOG.RUL.13` for drivers). A holder
//! who is counted in time triggers nothing, and a count closes the task.

use renox::chrono::Duration;
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::custody::{HandoverKind, HandoverStatus};

use crate::cash_handover::CashHandover;
use crate::cash_notices;
use crate::config_of;
use crate::followup::money_text;
use crate::texts::english;

/// The rule, as the specification names it.
pub const RULE: &str = "FIN.RUL.22";

/// The task that runs the rule.
pub const TASK: &str = "payments:cash-escalation";

/// What a run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Escalated {
    /// Handovers whose manager got the task to reach the holder.
    pub tasks: usize,
    /// Handovers whose company admin was told and manager asked to count.
    pub admin_notices: usize,
}

/// Which steps are due for `handover` at `now`: the manager's task after
/// `task_after`, the admin's notice after `admin_after`, each once.
pub(crate) fn due(
    handover: &CashHandover,
    now: DateTime,
    task_after: Duration,
    admin_after: Duration,
) -> (bool, bool) {
    if handover.status != HandoverStatus::Open
        || handover.kind != HandoverKind::Period
        || !handover.escalate
        || handover.expected <= 0
    {
        return (false, false);
    }
    let end = handover.ends_at();
    (
        handover.task_at.is_none() && now >= end + task_after,
        handover.admin_notified_at.is_none() && now >= end + admin_after,
    )
}

/// The rule at `now`.
///
/// # Errors
///
/// Database errors, or a notice or ticket that failed.
pub async fn run(state: &AppState, now: DateTime) -> Result<Escalated> {
    let config = config_of(state);
    let task_after = Duration::hours(config.cash_escalate_hours);
    let admin_after = Duration::hours(config.cash_admin_hours);
    let candidates = CashHandover::query()
        .where_(CashHandover::STATUS.eq(HandoverStatus::Open))
        .where_(CashHandover::KIND.eq(HandoverKind::Period))
        .where_(CashHandover::ESCALATE.eq(true))
        .order_by(CashHandover::ID)
        .get(&state.db)
        .await?;
    let mut done = Escalated::default();
    for handover in candidates {
        let (task, admin) = due(&handover, now, task_after, admin_after);
        if !task && !admin {
            continue;
        }
        let holder = User::find(&state.db, handover.holder_user_id)
            .await?
            .map_or_else(|| format!("#{}", handover.holder_user_id), |user| user.name);
        let manager = cash_notices::manager_of(state, &handover).await?;
        let amount = money_text(&handover.currency, handover.expected);
        let period = cash_notices::period_text(&handover);
        if task {
            let text = english("wl_payments.cash.escalation.reach")
                .replace("{holder}", &holder)
                .replace("{period}", &period)
                .replace("{amount}", &amount);
            cash_notices::open_reach_ticket(
                state,
                &handover,
                manager,
                &english("wl_payments.cash.escalation.reach_title").replace("{holder}", &holder),
                &text,
            )
            .await?;
            if let Some(manager) = manager {
                cash_notices::task(state, &handover, &[manager], &text).await?;
            }
            CashHandover::query()
                .where_(CashHandover::ID.eq(handover.id))
                .update(&state.db, &[("task_at", &Some(now))])
                .await?;
            done.tasks += 1;
        }
        if admin {
            let text = english("wl_payments.cash.escalation.admin")
                .replace("{holder}", &holder)
                .replace("{period}", &period)
                .replace("{amount}", &amount);
            cash_notices::tell_admins(state, &handover, &text).await?;
            if let Some(manager) = manager {
                let ask = english("wl_payments.cash.escalation.count")
                    .replace("{holder}", &holder)
                    .replace("{period}", &period);
                cash_notices::task(state, &handover, &[manager], &ask).await?;
            }
            CashHandover::query()
                .where_(CashHandover::ID.eq(handover.id))
                .update(&state.db, &[("admin_notified_at", &Some(now))])
                .await?;
            done.admin_notices += 1;
        }
    }
    Ok(done)
}

/// Registers the hourly rule and its command.
pub(crate) fn register(app: &mut Registry) {
    app.schedule().hourly(TASK, |state: AppState| async move {
        run(&state, renox::db::now()).await.map(|_| ())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime {
        text.parse().unwrap()
    }

    fn open() -> CashHandover {
        CashHandover {
            id: 1,
            expected: 90_000,
            escalate: true,
            period_date: "2027-05-01".parse().unwrap(),
            status: HandoverStatus::Open,
            ..Default::default()
        }
    }

    #[test]
    fn the_task_comes_12_hours_after_23_59_and_the_admin_48_hours() {
        let h = open();
        let n = Duration::hours(12);
        let m = Duration::hours(48);
        // 2027-05-01 23:59 in Dubai is 19:59 UTC.
        assert_eq!(due(&h, at("2027-05-02T07:58:00Z"), n, m), (false, false));
        assert_eq!(due(&h, at("2027-05-02T07:59:00Z"), n, m), (true, false));
        assert_eq!(due(&h, at("2027-05-03T19:58:00Z"), n, m), (true, false));
        assert_eq!(due(&h, at("2027-05-03T19:59:00Z"), n, m), (true, true));
    }

    #[test]
    fn each_step_happens_once() {
        let mut h = open();
        let n = Duration::hours(12);
        let m = Duration::hours(48);
        h.task_at = Some(at("2027-05-02T08:00:00Z"));
        assert_eq!(due(&h, at("2027-05-02T09:00:00Z"), n, m), (false, false));
        h.admin_notified_at = Some(at("2027-05-03T20:00:00Z"));
        assert_eq!(due(&h, at("2027-05-09T09:00:00Z"), n, m), (false, false));
    }

    #[test]
    fn a_counted_handover_a_replaced_rule_and_no_cash_trigger_nothing() {
        let n = Duration::hours(12);
        let m = Duration::hours(48);
        let late = at("2027-06-01T00:00:00Z");
        let mut counted = open();
        counted.status = HandoverStatus::Counted;
        assert_eq!(due(&counted, late, n, m), (false, false));
        let mut driver = open();
        driver.escalate = false;
        assert_eq!(due(&driver, late, n, m), (false, false));
        let mut empty = open();
        empty.expected = 0;
        assert_eq!(due(&empty, late, n, m), (false, false));
    }

    #[test]
    fn a_run_that_ends_earlier_starts_the_clock_earlier() {
        let mut h = open();
        h.period_end = Some(at("2027-05-01T11:00:00Z"));
        let n = Duration::hours(12);
        let m = Duration::hours(48);
        assert_eq!(due(&h, at("2027-05-01T22:59:00Z"), n, m), (false, false));
        assert_eq!(due(&h, at("2027-05-01T23:00:00Z"), n, m), (true, false));
    }
}
