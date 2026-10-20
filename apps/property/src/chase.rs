//! Chasing a lease that waits for its signature (`PRP.RUL.12`, 01 Property
//! §4.1, §7.2, BL-PRP-02, #252).
//!
//! A contract that was generated but not signed, or whose deposit was not
//! paid, must not wait forever. The daily run ([`run`], 02:00 Asia/Dubai,
//! also the command [`TASK`]) counts calendar days in Asia/Dubai since the
//! lease entered `pending_signature` (`submitted_on`):
//!
//! - **N = 14 days:** the leasing agent and the property managers of the
//!   building are reminded, once per lease ([`CHASE_REMIND_AFTER`]);
//! - **M = 30 days:** the property manager gets one task to *extend* the
//!   follow-up to a date at most 30 days ahead, or to cancel the lease
//!   ([`CHASE_TASK_AFTER`]). Extending ([`extend`]) records the date on the
//!   lease and closes the task; a lease still `pending_signature` on that date
//!   gets a new task. A task opened for a due date is not opened again once
//!   it is closed.
//!
//! The rule never cancels the lease and never sends the tenant anything. The
//! days are counted from the stored day the lease entered
//! `pending_signature`; a version leaves that state only to `active` or
//! `cancelled`, so the clock never restarts.

use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use wl_kernel::documents::today;
use wl_kernel::role_tasks::RoleTask;
use wl_kernel::tickets::task_creator;

use crate::building::refused;
use crate::lease::{Lease, LeaseStatus};
use crate::notices;
use crate::permissions::{self, PROPERTY_MANAGER_ROLE};
use crate::texts::english;

/// The scheduled task and the command.
pub const TASK: &str = "property:chase-unsigned";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "02:00";

/// N: the days in `pending_signature` after which the leasing agent and the
/// property manager are reminded.
pub const CHASE_REMIND_AFTER: i64 = 14;

/// M: the days in `pending_signature` after which the property manager gets
/// the task to extend or cancel.
pub const CHASE_TASK_AFTER: i64 = 30;

/// The furthest a follow-up can be extended: days ahead of today.
pub const MAX_EXTENSION_DAYS: i64 = 30;

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Leases whose agent and manager were reminded.
    pub reminded: usize,
    /// Tasks opened.
    pub tasked: usize,
    /// Leases that failed; the run moves on.
    pub failed: usize,
}

/// The day the property manager's task is due: the date the follow-up was
/// extended to, else `CHASE_TASK_AFTER` days after submission. `None` for a
/// lease that was never submitted.
pub fn task_due_on(lease: &Lease) -> Option<NaiveDate> {
    lease.follow_up_until.or_else(|| {
        lease
            .submitted_on
            .map(|day| day + Duration::days(CHASE_TASK_AFTER))
    })
}

/// The days `lease` has been waiting on `day`; `None` for a lease that is not
/// waiting.
pub fn days_waiting(lease: &Lease, day: NaiveDate) -> Option<i64> {
    if lease.status != LeaseStatus::PendingSignature {
        return None;
    }
    lease.submitted_on.map(|since| (day - since).num_days())
}

/// The key the extend-or-cancel task of `lease` is found by while it is open,
/// for the due date it was opened for.
pub fn task_key(lease: &Lease, due_on: NaiveDate) -> String {
    format!("property.chase:{}:{due_on}", lease.id)
}

/// Runs `PRP.RUL.12` for `day` (Asia/Dubai).
pub async fn run(state: &AppState, day: NaiveDate) -> Result<Report> {
    let mut report = Report::default();
    let waiting = Lease::query()
        .where_(Lease::STATUS.eq(LeaseStatus::PendingSignature))
        .order_by(Lease::ID)
        .get(&state.db)
        .await?;
    for lease in waiting {
        if let Err(error) = handle(state, &lease, day, &mut report).await {
            report.failed += 1;
            eprintln!("{TASK}: lease {}: {error:?}", lease.id);
        }
    }
    Ok(report)
}

/// Reminds and opens the task of one lease.
async fn handle(state: &AppState, lease: &Lease, day: NaiveDate, report: &mut Report) -> Result {
    let (Some(days), Some(since)) = (days_waiting(lease, day), lease.submitted_on) else {
        return Ok(());
    };
    if days >= CHASE_REMIND_AFTER && lease.chase_reminded_on.is_none() {
        // Stamped first: a failed send is not repeated every day.
        Lease::query()
            .where_(Lease::ID.eq(lease.id))
            .update(
                &state.db,
                &[
                    ("chase_reminded_on", &Some(day)),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
        notices::lease_unsigned(state, lease, days, since).await?;
        report.reminded += 1;
    }
    let Some(due_on) = task_due_on(lease) else {
        return Ok(());
    };
    if day >= due_on && lease.chase_tasked_on != Some(due_on) {
        Lease::query()
            .where_(Lease::ID.eq(lease.id))
            .update(
                &state.db,
                &[
                    ("chase_tasked_on", &Some(due_on)),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
        let task = RoleTask::new(
            lease.subject(),
            PROPERTY_MANAGER_ROLE,
            &english(
                "property.chase.task_title",
                &[("lease", &lease.number_with_version())],
            ),
            &task_key(lease, due_on),
        )
        .description(&english(
            "property.chase.task_description",
            &[("days", &days), ("since", &since)],
        ));
        crate::tasks::open(state, &lease.scope(), task).await?;
        report.tasked += 1;
    }
    Ok(())
}

/// `user` extends the follow-up of lease `id` to `until`, at most
/// [`MAX_EXTENSION_DAYS`] after `day`, and closes the open task. The lease
/// stays `pending_signature`; if it is still there on `until`, the next run
/// opens a new task.
///
/// # Errors
///
/// 403 without `leases.manage` in the lease's building; 422 unless the lease
/// is `pending_signature` and `until` is after `day` and not more than 30 days
/// ahead.
pub async fn extend_on(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    until: NaiveDate,
    day: NaiveDate,
) -> Result<Lease> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::LEASES_MANAGE, &lease.scope()) {
        return Err(Error::Forbidden);
    }
    if lease.status != LeaseStatus::PendingSignature {
        return Err(refused(english("property.chase.not_waiting", &[])));
    }
    if until <= day || until > day + Duration::days(MAX_EXTENSION_DAYS) {
        return Err(refused(english(
            "property.chase.date_range",
            &[("max", &MAX_EXTENSION_DAYS)],
        )));
    }
    // The task of the date being replaced closes with the extension.
    let open_for = task_due_on(&lease).map(|due| task_key(&lease, due));
    Lease::query()
        .where_(Lease::ID.eq(id))
        .update(
            &state.db,
            &[
                ("follow_up_until", &Some(until)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    if let Some(key) = open_for {
        let creator = task_creator(state);
        if let Some(task) = creator.find_open(state, &key).await? {
            // Built before the await: the argument list of `english` is not `Send`.
            let note = english("property.chase.extended", &[("until", &until)]);
            creator.complete(state, task.id, &note).await?;
        }
    }
    Lease::find(&state.db, id).await?.ok_or(Error::NotFound)
}

/// [`extend_on`] for today (Asia/Dubai).
///
/// # Errors
///
/// As [`extend_on`].
pub async fn extend(state: &AppState, user: &AuthUser, id: i64, until: NaiveDate) -> Result<Lease> {
    extend_on(state, user, id, until, today()).await
}

/// Registers the daily task and its command.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        TASK,
        "Remind about leases waiting for signature and give the property manager the extend-or-cancel task (--date YYYY-MM-DD checks another day)",
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
                "{} reminded, {} tasks opened, {} failed.",
                report.reminded, report.tasked, report.failed
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    fn waiting(since: NaiveDate) -> Lease {
        Lease {
            status: LeaseStatus::PendingSignature,
            submitted_on: Some(since),
            ..Default::default()
        }
    }

    #[test]
    fn the_days_run_from_the_day_the_lease_entered_pending_signature() {
        let lease = waiting(day(3, 1));
        assert_eq!(days_waiting(&lease, day(3, 1)), Some(0));
        assert_eq!(days_waiting(&lease, day(3, 15)), Some(14));
        assert_eq!(days_waiting(&lease, day(3, 31)), Some(30));
        let active = Lease {
            status: LeaseStatus::Active,
            ..lease
        };
        assert_eq!(days_waiting(&active, day(3, 15)), None);
    }

    #[test]
    fn the_task_is_due_thirty_days_after_submission_or_on_the_extended_date() {
        let mut lease = waiting(day(3, 1));
        assert_eq!(task_due_on(&lease), Some(day(3, 31)));
        lease.follow_up_until = Some(day(4, 15));
        assert_eq!(task_due_on(&lease), Some(day(4, 15)));
        assert_eq!(task_due_on(&Lease::default()), None);
    }

    #[test]
    fn a_task_key_names_the_lease_and_the_due_date() {
        let lease = Lease {
            id: 9,
            ..Default::default()
        };
        assert_eq!(task_key(&lease, day(3, 31)), "property.chase:9:2027-03-31");
    }
}
