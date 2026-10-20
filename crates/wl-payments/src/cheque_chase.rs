//! `FIN.RUL.18`: a bounced cheque is chased (wl-payments §6, §7, issue #107).
//!
//! A bounced received cheque must not be forgotten. The rule runs every hour
//! ([`run`]) over the cheques that are still `bounced`:
//!
//! - **N days after the bounce (default 3):** the party is reminded, with a
//!   notice that names the open amount and the bounce fee (not sent while the
//!   party is blocked, wl-parties §5.6), and the responsible staff are told
//!   (who collected the cheque, else who registered it, else the accountants).
//! - **M days after the bounce (default 14):** the accountant and the manager
//!   each get a task to re-present, refer to legal or write off the cheque
//!   ([`FollowupKind::DecideCheque`], role `accountant` and `finance_manager`)
//!   and are told.
//!
//! Days are counted from the bounce date in Asia/Dubai (`bounced_on`): a
//! bounce on 2027-02-01 is reminded on 2027-02-04 and gets its tasks on
//! 2027-02-15. A re-presented cheque that bounces again counts from the new
//! bounce: the day of the reminder and of the tasks are kept on the cheque
//! (`reminded_on`, `decision_task_on`) and count only when they are not before
//! the latest bounce. A part payment (`settle_part`) leaves the cheque
//! `bounced`, and the reminders go on with the new open amount.
//!
//! Only a `bounced` cheque is chased: once it is a `legal_case`, `replaced`,
//! `written_off` or cleared, nothing is sent, and the tasks of the decision
//! are closed by the transition that left `bounced`. A run that finds the work
//! late (the job was down) does it at once, once.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::chase_holds;
use wl_kernel::cheques::ChequeStatus;
use wl_kernel::documents::today;
use wl_kernel::payments::Direction;

use crate::cheque::Cheque;
use crate::config_of;
use crate::followup::{ACCOUNTANT_ROLE, FollowupTask, MANAGER_ROLE};
use crate::notices;

/// The rule id.
pub const RULE: &str = "FIN.RUL.18";

/// The scheduled task and command.
pub const TASK: &str = "payments:cheque_chase";

/// Whole days from the bounce to `today`; 0 for a cheque that did not bounce.
pub fn days_since_bounce(cheque: &Cheque, today: NaiveDate) -> i64 {
    cheque
        .bounced_on
        .map_or(0, |bounced| (today - bounced).num_days())
}

/// Whether `marker`, the day something was done for a cheque, counts for its
/// latest bounce (it is not before it).
fn done_for_latest_bounce(marker: Option<NaiveDate>, bounced_on: Option<NaiveDate>) -> bool {
    match (marker, bounced_on) {
        (Some(done), Some(bounced)) => done >= bounced,
        _ => false,
    }
}

/// Whether the cheque is chased at all: a `bounced` received cheque.
fn is_chased(cheque: &Cheque) -> bool {
    cheque.status == ChequeStatus::Bounced
        && cheque.direction == Direction::In
        && cheque.bounced_on.is_some()
}

/// Whether the party and the staff are due their reminder on `today`: `days`
/// (N) have passed since the bounce and it was not sent for this bounce.
pub fn reminder_due(cheque: &Cheque, today: NaiveDate, days: i64) -> bool {
    is_chased(cheque)
        && days_since_bounce(cheque, today) >= days
        && !done_for_latest_bounce(cheque.reminded_on, cheque.bounced_on)
}

/// Whether the decision tasks are due on `today`: `days` (M) have passed since
/// the bounce and they were not made for this bounce.
pub fn decision_due(cheque: &Cheque, today: NaiveDate, days: i64) -> bool {
    is_chased(cheque)
        && days_since_bounce(cheque, today) >= days
        && !done_for_latest_bounce(cheque.decision_task_on, cheque.bounced_on)
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Cheques whose reminders were sent.
    pub reminded: usize,
    /// Cheques whose decision tasks were made.
    pub decisions: usize,
}

/// Marks what was done for `cheque_id` on `today`.
async fn mark(state: &AppState, cheque_id: i64, column: &str, today: NaiveDate) -> Result<()> {
    Cheque::query()
        .where_(Cheque::ID.eq(cheque_id))
        .update(
            &state.db,
            &[(column, &Some(today)), ("updated_at", &renox::db::now())],
        )
        .await?;
    Ok(())
}

/// Makes the decision task of `role` for `cheque` and tells its holders.
async fn make_decision_task(
    state: &AppState,
    cheque: &Cheque,
    role: &str,
    days: i64,
) -> Result<()> {
    let task = FollowupTask::decide_cheque(cheque, role, days);
    if let Some(task) = FollowupTask::create_once(&state.db, task).await? {
        let ids: Vec<i64> = users_with_role_in(&state.db, role, &Scope::global())
            .await?
            .iter()
            .map(|user| user.id)
            .collect();
        notices::cheque_task_to(state, cheque, &task, &ids).await?;
    }
    Ok(())
}

/// Chases the bounced received cheques on `today` (Asia/Dubai): the
/// reminders after N days and the decision tasks after M days. Each is done
/// once per bounce.
///
/// # Errors
///
/// The database's errors, or a notice that failed.
pub async fn run(state: &AppState, today: NaiveDate) -> Result<Report> {
    let config = config_of(state);
    let bounced = Cheque::query()
        .where_(Cheque::STATUS.eq(ChequeStatus::Bounced))
        .where_(Cheque::DIRECTION.eq(Direction::In))
        .order_by(Cheque::ID)
        .get(&state.db)
        .await?;
    let mut report = Report::default();
    for cheque in bounced {
        let days = days_since_bounce(&cheque, today);
        if reminder_due(&cheque, today, config.cheque_reminder_days) {
            // A legal case on the agreement the cheque was received with
            // holds the automatic messages: the reminder is skipped, and not
            // sent late when the case ends.
            let held = match cheque.agreement() {
                Some(agreement) => chase_holds::hold(&state.db, &agreement).await?.is_held(),
                None => false,
            };
            if !held {
                notices::cheque_reminder(state, &cheque, days).await?;
                report.reminded += 1;
            }
            mark(state, cheque.id, "reminded_on", today).await?;
        }
        if decision_due(&cheque, today, config.cheque_decision_days) {
            make_decision_task(state, &cheque, ACCOUNTANT_ROLE, days).await?;
            make_decision_task(state, &cheque, MANAGER_ROLE, days).await?;
            mark(state, cheque.id, "decision_task_on", today).await?;
            report.decisions += 1;
        }
    }
    Ok(report)
}

/// Registers the scheduled task and its command.
pub(crate) fn register(app: &mut Registry) {
    app.schedule().hourly(TASK, |state: AppState| async move {
        run(&state, today()).await.map(|_| ())
    });
    app.command(
        TASK,
        "Remind the party and the staff of bounced cheques and leave the decision tasks (FIN.RUL.18)",
        |_args, state: AppState| async move {
            let report = run(&state, today()).await?;
            println!(
                "{} cheques reminded, {} decision tasks made.",
                report.reminded, report.decisions
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, month, day).unwrap()
    }

    fn bounced() -> Cheque {
        Cheque {
            id: 1,
            amount: 1_400_000,
            open_amount: 1_400_000,
            presentations: 1,
            bounced_on: Some(day(2, 1)),
            status: ChequeStatus::Bounced,
            ..Default::default()
        }
    }

    #[test]
    fn days_are_counted_from_the_bounce_date() {
        let cheque = bounced();
        assert_eq!(days_since_bounce(&cheque, day(2, 1)), 0);
        assert_eq!(days_since_bounce(&cheque, day(2, 4)), 3);
        assert_eq!(days_since_bounce(&cheque, day(2, 15)), 14);
        assert_eq!(days_since_bounce(&Cheque::default(), day(2, 15)), 0);
    }

    #[test]
    fn the_reminder_is_due_three_days_after_the_bounce_and_once() {
        let cheque = bounced();
        assert!(!reminder_due(&cheque, day(2, 3), 3));
        assert!(reminder_due(&cheque, day(2, 4), 3));
        // Late: the job was down on the 4th.
        assert!(reminder_due(&cheque, day(2, 6), 3));
        let sent = Cheque {
            reminded_on: Some(day(2, 4)),
            ..bounced()
        };
        assert!(!reminder_due(&sent, day(2, 5), 3));
    }

    #[test]
    fn the_decision_task_is_due_fourteen_days_after_the_bounce_and_once() {
        let cheque = bounced();
        assert!(!decision_due(&cheque, day(2, 14), 14));
        assert!(decision_due(&cheque, day(2, 15), 14));
        let made = Cheque {
            decision_task_on: Some(day(2, 15)),
            ..bounced()
        };
        assert!(!decision_due(&made, day(2, 16), 14));
    }

    #[test]
    fn a_new_bounce_starts_the_count_again() {
        // Reminded for the first bounce; bounced again on the 20th.
        let again = Cheque {
            reminded_on: Some(day(2, 4)),
            decision_task_on: Some(day(2, 15)),
            bounced_on: Some(day(2, 20)),
            ..bounced()
        };
        assert!(!reminder_due(&again, day(2, 22), 3));
        assert!(reminder_due(&again, day(2, 23), 3));
        assert!(!decision_due(&again, day(3, 5), 14));
        assert!(decision_due(&again, day(3, 6), 14));
    }

    #[test]
    fn only_a_bounced_received_cheque_is_chased() {
        for status in [
            ChequeStatus::LegalCase,
            ChequeStatus::Replaced,
            ChequeStatus::WrittenOff,
            ChequeStatus::Cleared,
            ChequeStatus::Deposited,
        ] {
            let cheque = Cheque {
                status,
                ..bounced()
            };
            assert!(!reminder_due(&cheque, day(3, 1), 3), "{status:?}");
            assert!(!decision_due(&cheque, day(3, 1), 14), "{status:?}");
        }
        let issued = Cheque {
            direction: Direction::Out,
            ..bounced()
        };
        assert!(!reminder_due(&issued, day(3, 1), 3));
    }
}
