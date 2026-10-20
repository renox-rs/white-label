//! `DOC.RUL.04`, the daily part (wl-documents §6, issue #53): every day at
//! 08:00 Asia/Dubai [`run`] first asks the required readers who have no
//! acknowledgement of the current version, then reminds and escalates:
//!
//! - **Remind:** a `pending` acknowledgement past its due date is reminded on
//!   the first run after it, then every [`REMIND_EVERY`] (R) days. A reader
//!   without an account cannot be reminded: their manager is.
//! - **Escalate:** once, [`ESCALATE_AFTER`] (E) days after the due date, the
//!   reader's manager and the holders of the SOP's owner role are told.
//! - **Pause:** while the reader is `on_leave` or not listed by the workforce
//!   (`absconded`), nothing is sent.
//!
//! The run takes the day as a parameter, so the command can check another day.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use wl_kernel::documents::today;

use super::model::{AckStatus, SopAcknowledgement};
use super::notices;
use super::service::Acknowledgements;
use crate::expiry::responsible::holders_anywhere;
use crate::sops::Sop;

/// The daily scheduled task, and the command, that run [`run`].
pub const TASK: &str = "documents:sop-acknowledgements";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "08:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// R: days between two reminders of the same acknowledgement.
pub const REMIND_EVERY: i64 = 3;

/// E: days overdue after which the manager and the owner role are told, once.
pub const ESCALATE_AFTER: i64 = 7;

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Acknowledgements requested because a reader had none.
    pub requested: usize,
    /// Pending acknowledgements cancelled because the reader is not
    /// required any more.
    pub cancelled: usize,
    /// Reminders sent.
    pub reminded: usize,
    /// Escalations sent.
    pub escalated: usize,
    /// Acknowledgements or SOPs that failed; the run moves on.
    pub failed: usize,
}

/// What an acknowledgement needs on a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    /// Remind the reader (or their manager).
    pub remind: bool,
    /// Tell the manager and the owner role.
    pub escalate: bool,
    /// Calendar days past the due date.
    pub days: i64,
}

/// What `ack` needs on `day`: nothing unless it is pending and past its due
/// date.
pub fn plan(ack: &SopAcknowledgement, day: NaiveDate) -> Plan {
    let days = (day - ack.due_date).num_days();
    if ack.status != AckStatus::Pending || days < 1 {
        return Plan {
            remind: false,
            escalate: false,
            days,
        };
    }
    Plan {
        remind: ack
            .last_reminded_on
            .is_none_or(|last| (day - last).num_days() >= REMIND_EVERY),
        escalate: ack.escalated_on.is_none() && days >= ESCALATE_AFTER,
        days,
    }
}

/// Runs `DOC.RUL.04` for `day` (Asia/Dubai).
pub async fn run(state: &AppState, day: NaiveDate) -> Result<Report> {
    let mut report = Report::default();
    match Acknowledgements::reconcile_all(state, day).await {
        Ok(done) => {
            report.requested += done.requested;
            report.cancelled += done.cancelled;
        }
        Err(error) => {
            report.failed += 1;
            eprintln!("{TASK}: {error:?}");
        }
    }
    let pending = SopAcknowledgement::query()
        .where_(SopAcknowledgement::STATUS.eq(AckStatus::Pending))
        .order_by(SopAcknowledgement::ID)
        .get(&state.db)
        .await?;
    for ack in pending {
        if let Err(error) = handle(state, &ack, day, &mut report).await {
            report.failed += 1;
            eprintln!("{TASK}: acknowledgement {}: {error:?}", ack.id);
        }
    }
    Ok(report)
}

/// Reminds and escalates one acknowledgement.
async fn handle(
    state: &AppState,
    ack: &SopAcknowledgement,
    day: NaiveDate,
    report: &mut Report,
) -> Result {
    let needs = plan(ack, day);
    if !needs.remind && !needs.escalate {
        return Ok(());
    }
    let sop = Sop::find_or_404(&state.db, ack.sop_id).await?;
    if sop.current_document_id != Some(ack.document_id) {
        return Ok(());
    }
    let Some(profile) = Acknowledgements::profile(state, ack.employee_id).await? else {
        return Ok(());
    };
    // Paused: on leave, or not listed (absconded).
    if !profile.listed || profile.on_leave {
        return Ok(());
    }
    if needs.remind {
        let users = match profile.user_id {
            Some(user) => vec![user],
            None => profile.managers.clone(),
        };
        SopAcknowledgement::query()
            .where_(SopAcknowledgement::ID.eq(ack.id))
            .update(
                &state.db,
                &[
                    ("last_reminded_on", &Some(day)),
                    ("reminders_sent", &(ack.reminders_sent + 1)),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        notices::reminder(state, ack, &sop, &profile, &users, needs.days).await?;
        report.reminded += 1;
    }
    if needs.escalate {
        let mut told = profile.managers.clone();
        told.extend(holders_anywhere(&state.db, &sop.owner_role).await?);
        told.sort_unstable();
        told.dedup();
        SopAcknowledgement::query()
            .where_(SopAcknowledgement::ID.eq(ack.id))
            .update(
                &state.db,
                &[("escalated_on", &Some(day)), ("updated_at", &now())],
            )
            .await?;
        notices::escalated(state, ack, &sop, &profile, &told, needs.days).await?;
        report.escalated += 1;
    }
    Ok(())
}

/// Registers the daily [`TASK`] and the command of the same name.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Ask the required readers of each SOP to acknowledge it, remind the overdue ones and \
         tell their manager and the owner role (--date YYYY-MM-DD checks another day)",
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
            let report = run(&state, on).await?;
            println!(
                "{} requested, {} cancelled, {} reminded, {} escalated, {} failed.",
                report.requested,
                report.cancelled,
                report.reminded,
                report.escalated,
                report.failed
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn pending(due: NaiveDate) -> SopAcknowledgement {
        SopAcknowledgement {
            due_date: due,
            ..SopAcknowledgement::default()
        }
    }

    #[test]
    fn nothing_is_due_on_the_due_date_itself() {
        let p = plan(&pending(day(11, 9)), day(11, 9));
        assert!(!p.remind && !p.escalate);
    }

    #[test]
    fn the_first_run_after_the_due_date_reminds_then_every_third_day() {
        let mut ack = pending(day(11, 9));
        assert!(plan(&ack, day(11, 10)).remind);
        ack.last_reminded_on = Some(day(11, 10));
        assert!(!plan(&ack, day(11, 12)).remind);
        assert!(plan(&ack, day(11, 13)).remind);
    }

    #[test]
    fn the_manager_and_the_owner_role_are_told_once_after_seven_days() {
        let mut ack = pending(day(11, 9));
        assert!(!plan(&ack, day(11, 15)).escalate);
        assert!(plan(&ack, day(11, 16)).escalate);
        assert_eq!(plan(&ack, day(11, 16)).days, 7);
        ack.escalated_on = Some(day(11, 16));
        assert!(!plan(&ack, day(11, 20)).escalate);
    }

    #[test]
    fn a_settled_acknowledgement_needs_nothing() {
        let done = SopAcknowledgement {
            status: AckStatus::Acknowledged,
            ..pending(day(11, 9))
        };
        let p = plan(&done, day(12, 30));
        assert!(!p.remind && !p.escalate);
    }
}
