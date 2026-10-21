//! `HRM.RUL.15`: an external timesheet nobody answers (wl-attendance §5.6,
//! §6 *Automations*, §7; BL-ATT-10).
//!
//! The reminder ladder of `HRM.RUL.08` goes on past the reminders. A daily
//! task at 08:00 Asia/Dubai ([`TASK`]) looks at the timesheets that an
//! external approver approves and that are still `submitted`. E calendar days
//! after the date of the latest `submit` (default 6,
//! [`AttendanceConfig::escalation_days`](crate::AttendanceConfig)) it, once per
//! `submit`:
//!
//! - tells the client's escalation contact (the app's
//!   [`ApproverResolver`](wl_kernel::timesheets::ApproverResolver)) and the
//!   account manager's manager;
//! - gives the account manager one task, "collect a signed sheet", so that
//!   staff can `approve_on_behalf` ([`crate::approval`]). The task closes by
//!   itself when the timesheet is approved or sent back ([`close_task`]).
//!
//! The timesheet's state does not change at E. The rule is idempotent: the
//! escalation is claimed with `escalated_at` (set in one update that only the
//! first run wins), so a repeated run sends nothing and opens no second task;
//! a new `submit` after a `reject` clears it and the clock restarts. A failure
//! after the claim releases it, so that the next run tries again.
//!
//! The deemed approval at N days (BL-ATT-11) belongs to the same daily run and
//! comes first, so that with N at or below E the escalation finds nothing
//! `submitted`.
//!
//! Decisions:
//!
//! - **The account manager's manager** is the line manager in wl-hr: the
//!   `managers` of the staff profile of the employee behind the account
//!   manager's user. With none (no account manager, no employee behind the
//!   user, no manager), the users holding `timesheets.approve_on_behalf` in
//!   the scope of the context are told instead, and the task says no manager
//!   was found.
//! - **No escalation contact** on the contract: only the manager side is told
//!   and the task says so.
//! - **Internal timesheets** are not escalated by this rule.
//! - **Adjustments** that an external approver has not answered are escalated
//!   like any other timesheet.
//! - A rejected timesheet no longer waits, so its task is closed with the
//!   `reject` as well as with the approval.

use renox::auth::permissions::users_with_permission_in;
use renox::chrono::{Duration, NaiveDate};
use renox::db::{DateTime, now};
use renox::prelude::*;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::{self, Assignee, OpenTicket, TASK_CATEGORY};
use wl_kernel::timesheets::approver_resolver;
use wl_kernel::workforce::workforce;

use crate::access;
use crate::config::config_of;
use crate::notices;
use crate::permissions;
use crate::texts::english;
use crate::timesheet::{ApproverKind, Timesheet, TimesheetStatus};

/// The daily scheduled task, and the command, that run [`run`].
pub const TASK: &str = "attendance:timesheet-escalation";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "08:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Timesheets whose approver was escalated to.
    pub escalated: usize,
    /// Timesheets that failed; the run moves on and the next run tries
    /// again.
    pub failed: usize,
}

/// The source key of the task to collect a signed sheet for a timesheet.
pub fn task_key(timesheet_id: i64) -> String {
    format!("attendance.timesheet_escalation:{timesheet_id}")
}

/// Who an escalation is for.
#[derive(Debug, Clone, Default)]
pub struct Audience {
    /// The account manager, who gets the task.
    pub account_manager: Option<i64>,
    /// The client's escalation contacts.
    pub contacts: Vec<NotifyRecipient>,
    /// The account manager's managers, or the users who may approve on
    /// behalf when there is none.
    pub staff: Vec<i64>,
    /// Whether no line manager of the account manager was found.
    pub no_manager: bool,
}

/// Works out who an escalation of `sheet` goes to.
///
/// # Errors
///
/// The resolver's, the workforce's and the database's errors.
pub async fn audience_of(state: &AppState, sheet: &Timesheet) -> Result<Audience> {
    let context = sheet.context();
    let resolver = approver_resolver(state);
    let account_manager = resolver.account_manager(state, context.as_ref()).await?;
    let contacts = resolver
        .escalation_contacts(state, context.as_ref())
        .await?;
    let mut staff: Vec<i64> = Vec::new();
    if let Some(user) = account_manager
        && let Some(employee) = workforce(state).employee_of_user(&state.db, user).await?
        && let Some(profile) = workforce(state).staff(state, employee).await?
    {
        staff = profile.managers;
    }
    let no_manager = staff.is_empty();
    if no_manager {
        let scope = access::scope_of_subject(&state.db, context.as_ref()).await?;
        staff =
            users_with_permission_in(&state.db, permissions::TIMESHEETS_APPROVE_ON_BEHALF, &scope)
                .await?
                .iter()
                .map(|user| user.id)
                .collect();
    }
    staff.sort_unstable();
    staff.dedup();
    Ok(Audience {
        account_manager,
        contacts,
        staff,
        no_manager,
    })
}

/// The task text: the ask, and what was missing.
fn task_description(
    employee: &str,
    sheet: &Timesheet,
    submitted: NaiveDate,
    days: i64,
    audience: &Audience,
) -> String {
    let mut text = english(
        "wl_attendance.escalation.task_body",
        &[
            ("employee", &employee),
            ("from", &sheet.period_from),
            ("to", &sheet.period_to),
            ("submitted", &submitted),
            ("days", &days),
        ],
    );
    if audience.no_manager {
        text.push(' ');
        text.push_str(&english("wl_attendance.escalation.no_manager", &[]));
    }
    if audience.contacts.is_empty() {
        text.push(' ');
        text.push_str(&english("wl_attendance.escalation.no_contact", &[]));
    }
    if audience.account_manager.is_none() {
        text.push(' ');
        text.push_str(&english("wl_attendance.escalation.no_account_manager", &[]));
    }
    text
}

/// Tells the audience and opens the task, for the claimed `sheet`.
async fn escalate_claimed(state: &AppState, sheet: &Timesheet, on: NaiveDate) -> Result {
    let submitted = sheet.submitted_at.map(dubai_date).unwrap_or(on);
    let days = (on - submitted).num_days();
    let audience = audience_of(state, sheet).await?;
    let employee = notices::employee_name(state, sheet.owner()).await?;

    let creator = tickets::task_creator(state);
    let key = task_key(sheet.id);
    if creator.find_open(state, &key).await?.is_none() {
        let title = english(
            "wl_attendance.escalation.task_title",
            &[
                ("employee", &employee),
                ("from", &sheet.period_from),
                ("to", &sheet.period_to),
            ],
        );
        let mut ticket = OpenTicket::new(sheet.subject(), TASK_CATEGORY, &title)
            .description(&task_description(
                &employee, sheet, submitted, days, &audience,
            ))
            .source_key(&key);
        if let Some(user) = audience.account_manager {
            ticket = ticket.assign_to(Assignee::user(user));
        }
        creator.open(state, ticket).await?;
    }
    notices::timesheet_escalated(state, sheet, &employee, submitted, days, &audience).await
}

/// Escalates `sheet` if it is due and nobody has yet for this `submit`.
/// Returns whether it did.
async fn escalate_one(state: &AppState, sheet: &Timesheet, on: NaiveDate, e: i64) -> Result<bool> {
    let Some(submitted) = sheet.submitted_at else {
        return Ok(false);
    };
    if dubai_date(submitted) + Duration::days(e) > on {
        return Ok(false);
    }
    // One run wins the claim: a repeat, or a second run at the same moment,
    // finds `escalated_at` set.
    let claimed = Timesheet::query()
        .where_(Timesheet::ID.eq(sheet.id))
        .where_(Timesheet::STATUS.eq(TimesheetStatus::Submitted))
        .where_(Timesheet::ESCALATED_AT.is_null())
        .update(&state.db, &[("escalated_at", &Some(now()))])
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    match escalate_claimed(state, sheet, on).await {
        Ok(()) => Ok(true),
        Err(error) => {
            // Let the next run try again.
            Timesheet::query()
                .where_(Timesheet::ID.eq(sheet.id))
                .update(&state.db, &[("escalated_at", &Option::<DateTime>::None)])
                .await?;
            Err(error)
        }
    }
}

/// Escalates every external timesheet still `submitted` for E days or more
/// (calendar days from the date of its latest `submit`, in Asia/Dubai) on
/// `on` and not escalated since that `submit`.
///
/// # Errors
///
/// A database error while listing the timesheets.
pub async fn escalate(state: &AppState, on: NaiveDate) -> Result<Report> {
    let e = config_of(state).escalation_days;
    let waiting = Timesheet::query()
        .where_(Timesheet::STATUS.eq(TimesheetStatus::Submitted))
        .where_(Timesheet::APPROVER_KIND.eq(ApproverKind::External))
        .where_(Timesheet::ESCALATED_AT.is_null())
        .order_by(Timesheet::ID)
        .get(&state.db)
        .await?;
    let mut report = Report::default();
    for sheet in waiting {
        match escalate_one(state, &sheet, on, e).await {
            Ok(true) => report.escalated += 1,
            Ok(false) => {}
            Err(_) => report.failed += 1,
        }
    }
    Ok(report)
}

/// The daily run of `HRM.RUL.15` on `on`.
///
/// # Errors
///
/// A database error while listing the timesheets.
pub async fn run(state: &AppState, on: NaiveDate) -> Result<Report> {
    escalate(state, on).await
}

/// Closes the task to collect a signed sheet for `timesheet_id`, if it is
/// open: the timesheet was approved or sent back.
///
/// # Errors
///
/// The task creator's errors.
pub(crate) async fn close_task(state: &AppState, timesheet_id: i64) -> Result {
    let creator = tickets::task_creator(state);
    if let Some(found) = creator.find_open(state, &task_key(timesheet_id)).await? {
        let note = english("wl_attendance.escalation.task_done", &[]);
        creator.complete(state, found.id, &note).await?;
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
        "Escalate the external timesheets nobody answers (--date YYYY-MM-DD checks another day)",
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
            println!("{} escalated, {} failed.", report.escalated, report.failed);
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_of_a_timesheet_has_its_own_key() {
        assert_eq!(task_key(7), "attendance.timesheet_escalation:7");
    }

    #[test]
    fn the_task_says_what_was_missing() {
        let sheet = Timesheet::default();
        let day = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let none = Audience {
            no_manager: true,
            ..Audience::default()
        };
        let text = task_description("Amal", &sheet, day, 6, &none);
        assert!(text.contains("No line manager"));
        assert!(text.contains("No escalation contact"));
        assert!(text.contains("No account manager"));
        let all = Audience {
            account_manager: Some(1),
            contacts: vec![NotifyRecipient::User { id: 2 }],
            staff: vec![3],
            no_manager: false,
        };
        let text = task_description("Amal", &sheet, day, 6, &all);
        assert!(!text.contains("No line manager"));
        assert!(!text.contains("No escalation contact"));
        assert!(!text.contains("No account manager"));
    }
}
