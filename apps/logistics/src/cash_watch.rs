//! A driver who does not hand over the cash (03 Logistics §4.1 row *Driver
//! does not hand over the cash, or disappears*, §7.1 *Cash handover*, §7.2
//! `LOG.RUL.13`; BL-LOG-13).
//!
//! `LOG.RUL.13` replaces wl-payments' `FIN.RUL.22` for driver handovers: the
//! run's cash period is collected with the escalation replaced
//! (`HandoverPeriod::replaced_escalation`, set by the host with
//! `WlDelivery::replace_cash_escalation`), so wl-payments' own rule skips it
//! and the driver gets one task and one admin notice, not two; other holders
//! (counter staff) keep `FIN.RUL.22`.
//!
//! [`run`] reads the open handovers whose escalation is replaced
//! (`CashCustody::replaced_open`) every 15 minutes. The **clock start** is
//! the run's return to the depot (an abort returns it), else, for a run still
//! out, the end of the driver's shift on the run date (`ShiftCalendar`), else
//! 23:59 Asia/Dubai of the run date.
//!
//! - **N = 12 hours after the start:** a task for the operations manager
//!   ([`crate::permissions::OPERATIONS_MANAGER_ROLE`]) to call the driver; for
//!   a run still out it also asks to abort the run first (reason *driver
//!   unreachable*), which opens the count.
//! - **M = 48 hours after the same start** (not after the task): the company
//!   admin is told, the operations manager is asked to count the handover at
//!   what was received (often nothing, so the whole expected amount is a
//!   shortage, `short`), and, only when the driver has no attendance check-in
//!   after the start (`AttendanceHistory::checked_in_since`, `HRM.ATT.01`), an
//!   absence case is opened on the driver's HR record
//!   (`AbsenceCaseOpened`: the absconding path of wl-hr starts from it; the
//!   report itself is an HR decision).
//! - A handover counted (or recovered, written off) closes the tasks and
//!   nothing more is sent: a driver who returns before M is counted as usual.
//!
//! The rule never touches the client's COD remittance: that is computed from
//! the shipments delivered ([`crate::remit`]), whatever the cash did. Counting,
//! recovering in cash, the salary deduction (off while the client's
//! `cash_salary_deduction` setting is off, `FIN.ACL.19`, open item L-20) and
//! the write-off with approval are wl-payments' (BL-PAY-17). N and M are
//! constants until the logistics settings page exists.

use renox::auth::permissions::users_with_role;
use renox::chrono::{Duration, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::attendance::{attendance_history, shift_calendar};
use wl_kernel::custody::{CashHandoverCounted, HandoverStatus, HandoverSummary, custody};
use wl_kernel::documents::today;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::scopes::COMPANY_ADMIN_ROLE;
use wl_kernel::tickets::{self, TicketId};
use wl_kernel::workforce::{AbsenceCaseOpened, workforce};

use crate::permissions;
use crate::rates::aed;
use crate::texts::{english, template};

/// The rule, as the specification names it.
pub const RULE: &str = "LOG.RUL.13";

/// The task that runs the rule.
pub const TASK: &str = "logistics:cash-watch";

/// How often the rule runs, in minutes: the 12 and 48 hour instants are met
/// to within this many minutes.
pub const EVERY_MINUTES: u64 = 15;

/// N: hours after the clock start before the operations manager's task.
pub const TASK_HOURS: i64 = 12;

/// M: hours after the same start before the admin's notice and the count.
pub const ADMIN_HOURS: i64 = 48;

/// The notification type of the company admin's notice.
pub const CASH_ESCALATED: &str = "logistics.cash_escalated";

/// The notification type: a title and a body the caller words.
pub fn cash_escalated_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CASH_ESCALATED, "logistics")
        .group("cash")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("logistics.notice.subject")),
            template("logistics.notice.body"),
        )
}

/// What the rule did about one handover.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_cash_watches", unique(handover_id))]
pub struct CashWatch {
    /// The primary key.
    pub id: i64,
    /// The wl-payments cash handover of the run.
    pub handover_id: i64,
    /// The run (the handover's period record).
    pub run_id: i64,
    /// The day of the run.
    pub run_date: NaiveDate,
    /// The driver, a user.
    pub driver_user_id: i64,
    /// The clock start.
    pub started_at: DateTime,
    /// Whether the run was still out when the first step was taken.
    pub run_out: bool,
    /// When the operations manager got the task to call the driver.
    pub task_at: Option<DateTime>,
    /// The ticket of that task.
    pub task_ticket_id: Option<i64>,
    /// When the admin was told and the manager asked to count.
    pub admin_at: Option<DateTime>,
    /// The ticket of the request to count.
    pub count_ticket_id: Option<i64>,
    /// When the absence note was written.
    pub absence_noted_at: Option<DateTime>,
    /// Why no absence note was written: `checked_in` or `no_employee`.
    pub absence_skipped: Option<String>,
    /// When the handover left `open` and the tasks were closed.
    pub closed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What a run of the rule did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Watched {
    /// Tasks to call the driver opened.
    pub tasks: usize,
    /// Handovers whose admin was told and manager asked to count.
    pub admin_notices: usize,
    /// Absence notes written.
    pub absence_notes: usize,
    /// Handovers whose tasks were closed because they were counted.
    pub closed: usize,
}

/// 23:59 Asia/Dubai (UTC+4, no daylight saving) of `date`, as an instant.
fn end_of_day(date: NaiveDate) -> DateTime {
    date.and_hms_opt(19, 59, 0)
        .expect("19:59 is a time")
        .and_utc()
}

/// The clock start of `handover`: the end of the period when the run ended
/// it, else the driver's shift end, else 23:59 of the run date.
///
/// # Errors
///
/// The calendar's errors.
pub async fn clock_start(state: &AppState, handover: &HandoverSummary) -> Result<DateTime> {
    if let Some(ended) = handover.ended_at {
        return Ok(ended);
    }
    let shift = shift_calendar(state)
        .shift_end(state, handover.holder_user, handover.date)
        .await?;
    Ok(shift.unwrap_or_else(|| end_of_day(handover.date)))
}

/// The watch row of `handover`, made when the first step is taken.
async fn watch_of(
    state: &AppState,
    handover: &HandoverSummary,
    start: DateTime,
) -> Result<CashWatch> {
    if let Some(found) = CashWatch::query()
        .where_(CashWatch::HANDOVER_ID.eq(handover.id.0))
        .first(&state.db)
        .await?
    {
        return Ok(found);
    }
    let mut row = CashWatch {
        handover_id: handover.id.0,
        run_id: handover.period.as_ref().map_or(0, |run| run.subject_id),
        run_date: handover.date,
        driver_user_id: handover.holder_user,
        started_at: start,
        run_out: handover.ended_at.is_none(),
        ..CashWatch::default()
    };
    row.insert(&state.db).await?;
    Ok(row)
}

/// The name of user `id` for a text, else `#id`.
async fn user_name(state: &AppState, id: i64) -> String {
    match User::find(&state.db, id).await {
        Ok(Some(user)) => user.name,
        _ => format!("#{id}"),
    }
}

/// The task for the operations manager to call the driver (and abort the run
/// while it is still out), N hours after the start.
async fn open_task(
    state: &AppState,
    handover: &HandoverSummary,
    row: &CashWatch,
    now: DateTime,
) -> Result {
    let driver = user_name(state, handover.holder_user).await;
    let amount = aed(handover.expected);
    let title = english(
        "logistics.cash.task_title",
        &[
            ("driver", &driver),
            ("run", &row.run_id),
            ("amount", &amount),
        ],
    );
    let description = english(
        if row.run_out {
            "logistics.cash.task_out"
        } else {
            "logistics.cash.task_returned"
        },
        &[
            ("driver", &driver),
            ("run", &row.run_id),
            ("amount", &amount),
        ],
    );
    let task = RoleTask::new(
        handover.id.subject(),
        permissions::OPERATIONS_MANAGER_ROLE,
        &title,
        &format!("logistics.cash_call:{}", handover.id.0),
    )
    .description(&description);
    let ticket = role_tasks::open(state, task).await?.map(|t| t.id.0);
    CashWatch::query()
        .where_(CashWatch::ID.eq(row.id))
        .update(
            &state.db,
            &[("task_at", &Some(now)), ("task_ticket_id", &ticket)],
        )
        .await?;
    Ok(())
}

/// M hours after the start: the company admin is told and the operations
/// manager is asked to count; the driver's HR record gets the absence note
/// when no check-in followed the start. Returns whether a note was written.
async fn escalate(
    state: &AppState,
    handover: &HandoverSummary,
    row: &CashWatch,
    now: DateTime,
) -> Result<bool> {
    let driver = user_name(state, handover.holder_user).await;
    let amount = aed(handover.expected);
    let args: [(&str, &(dyn std::fmt::Display + Sync)); 3] = [
        ("driver", &driver),
        ("run", &row.run_id),
        ("amount", &amount),
    ];
    // The admin is told.
    let title = english("logistics.cash.admin_title", &args);
    let body = english("logistics.cash.admin_body", &args);
    let mut request =
        NotifyRequest::new(CASH_ESCALATED, handover.id.subject()).data(renox::serde_json::json!({
            "title": title,
            "body": body,
            "link": format!("/cash/{}", handover.id.0),
        }));
    let mut told = false;
    for admin in users_with_role(&state.db, COMPANY_ADMIN_ROLE).await? {
        request = request.to_user(admin.id);
        told = true;
    }
    if told {
        notify::notifier(state).notify(state, request).await?;
    }
    // The operations manager counts what was received.
    let count_title = english("logistics.cash.count_title", &args);
    let count_description = english(
        if row.run_out {
            "logistics.cash.count_out"
        } else {
            "logistics.cash.count_returned"
        },
        &args,
    );
    let task = RoleTask::new(
        handover.id.subject(),
        permissions::OPERATIONS_MANAGER_ROLE,
        &count_title,
        &format!("logistics.cash_count:{}", handover.id.0),
    )
    .description(&count_description);
    let count_ticket = role_tasks::open(state, task).await?.map(|t| t.id.0);
    // The absence note, once, when the driver did not check in since.
    let (noted, skipped) = absence_note(state, handover, row).await?;
    CashWatch::query()
        .where_(CashWatch::ID.eq(row.id))
        .update(
            &state.db,
            &[
                ("admin_at", &Some(now)),
                ("count_ticket_id", &count_ticket),
                ("absence_noted_at", &noted.then_some(now)),
                ("absence_skipped", &skipped),
            ],
        )
        .await?;
    Ok(noted)
}

/// Writes the absence note of the driver of `handover`: an absence case on the
/// HR record, unless an attendance check-in followed the clock start (then
/// `checked_in`) or the driver is no employee (`no_employee`).
async fn absence_note(
    state: &AppState,
    handover: &HandoverSummary,
    row: &CashWatch,
) -> Result<(bool, Option<String>)> {
    let Some(employee) = workforce(state)
        .employee_of_user(&state.db, handover.holder_user)
        .await?
    else {
        return Ok((false, Some("no_employee".to_owned())));
    };
    if attendance_history(state)
        .checked_in_since(state, employee, row.started_at)
        .await?
    {
        return Ok((false, Some("checked_in".to_owned())));
    }
    // Day 0 of the absence is the day of the run: the last day the driver
    // is known to have worked.
    state
        .emit(AbsenceCaseOpened {
            employee,
            case: wl_kernel::delivery::RunId(row.run_id).subject(),
            opened_on: today(),
            last_present_on: row.run_date,
        })
        .await?;
    Ok((true, None))
}

/// Closes the tasks of the handover `handover_id` once it is no longer open.
async fn close_tasks(state: &AppState, row: &CashWatch, note: &str) -> Result {
    for ticket in [row.task_ticket_id, row.count_ticket_id]
        .into_iter()
        .flatten()
    {
        tickets::task_creator(state)
            .complete(state, TicketId(ticket), note)
            .await?;
    }
    CashWatch::query()
        .where_(CashWatch::ID.eq(row.id))
        .update(&state.db, &[("closed_at", &Some(renox::db::now()))])
        .await?;
    Ok(())
}

/// Closes the tasks of the watched handover `handover_id`, if it has any open:
/// the driver handed over and it was counted.
///
/// # Errors
///
/// Database and ticketing errors.
pub(crate) async fn close_for(state: &AppState, handover_id: i64) -> Result<bool> {
    let row = CashWatch::query()
        .where_(CashWatch::HANDOVER_ID.eq(handover_id))
        .where_(CashWatch::CLOSED_AT.is_null())
        .first(&state.db)
        .await?;
    match row {
        Some(row) => {
            close_tasks(state, &row, &english("logistics.cash.closed", &[])).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// The rule `LOG.RUL.13` at `now`: closes the tasks of handovers that were
/// counted, and takes the N and M hour steps that are due for the open ones,
/// each once. Returns what it did.
///
/// # Errors
///
/// Database, custody, ticketing and notifier errors.
pub async fn run(state: &AppState, now: DateTime) -> Result<Watched> {
    let mut done = Watched::default();
    // A handover counted since closes its tasks.
    let open_rows = CashWatch::query()
        .where_(CashWatch::CLOSED_AT.is_null())
        .order_by(CashWatch::ID)
        .get(&state.db)
        .await?;
    for row in open_rows {
        let still_open = custody(state)
            .find(&state.db, wl_kernel::custody::HandoverId(row.handover_id))
            .await?
            .is_some_and(|handover| handover.status == HandoverStatus::Open);
        if !still_open {
            close_tasks(state, &row, &english("logistics.cash.closed", &[])).await?;
            done.closed += 1;
        }
    }
    for handover in custody(state).replaced_open(&state.db).await? {
        let start = clock_start(state, &handover).await?;
        let existing = CashWatch::query()
            .where_(CashWatch::HANDOVER_ID.eq(handover.id.0))
            .first(&state.db)
            .await?;
        let task_due = now >= start + Duration::hours(TASK_HOURS)
            && existing.as_ref().is_none_or(|row| row.task_at.is_none());
        let admin_due = now >= start + Duration::hours(ADMIN_HOURS)
            && existing.as_ref().is_none_or(|row| row.admin_at.is_none());
        if !task_due && !admin_due {
            continue;
        }
        let row = watch_of(state, &handover, start).await?;
        if task_due {
            open_task(state, &handover, &row, now).await?;
            done.tasks += 1;
        }
        if admin_due {
            let row = CashWatch::find_or_404(&state.db, row.id).await?;
            if escalate(state, &handover, &row, now).await? {
                done.absence_notes += 1;
            }
            done.admin_notices += 1;
        }
    }
    Ok(done)
}

/// Registers the notification type, the rule on its schedule and the
/// listener that closes the tasks when the handover is counted.
pub(crate) fn register(app: &mut Registry) {
    notify::register_type(cash_escalated_type());
    app.schedule()
        .every_minutes(EVERY_MINUTES, TASK, |state: AppState| async move {
            run(&state, renox::db::now()).await.map(|_| ())
        });
    app.listen(|event: CashHandoverCounted, state: AppState| async move {
        close_for(&state, event.handover.0).await.map(|_| ())
    });
}
