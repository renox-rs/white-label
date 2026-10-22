//! `EDU.RUL.12`: a tutor who will not be eligible or available (10 §4.2 rows
//! *A tutor's police clearance or permit will expire* and *A tutor is sick or
//! leaves*, §7.1 *Booking*, §7.2; BL-EDU-10). Compliance-critical.
//!
//! **Triggers.** The rule runs on `DocumentExpiring`, `DocumentExpired` and
//! `QualificationExpired` of a tutor's police clearance or permit, on
//! `EmployeeNoticeGiven`, on a manual availability block on the tutor
//! (`AvailabilityBlockAdded` of wl-scheduling, source `manual`), on
//! `LeaveApproved` (built, but inactive until tutor leave `HRM.LEV.01` is
//! enabled: [`LEAVE_TRIGGER_ENABLED`]) and every day. Each trigger is a
//! [`TutorAlert`], one per tutor and trigger.
//!
//! **The date** the tutor can no longer teach from: the day after the expiry date
//! for a document, the day after the last working day for a notice, the start of
//! the block or of the leave. **The window**: the sessions dated from that day up
//! to [`WINDOW_DAYS`] (30) after the later of that day and today; it moves with
//! the daily run, so later sessions enter one by one (for a clearance expiring on
//! 2026-12-15 the run on 2026-12-21 covers 2026-12-16 to 2027-01-20). For a block
//! or a leave only the sessions inside the period are flagged.
//!
//! **What it does.** It flags each `scheduled` session of the tutor in the window
//! ("tutor not available or not eligible", [`crate::flags`]); it gives the
//! academic coordinator **one task per tutor and trigger** that lists the
//! sessions (and, for a document, the tutor's one-to-one bookings, read through
//! wl-scheduling's `FutureWork`) and is updated by the daily run with a comment
//! when sessions enter the window; and it tells the centre manager, once per
//! session, when a session is still flagged [`ESCALATION_HOURS`] (24) hours before
//! it starts ([`escalate`]). The one-to-one bookings are flagged and escalated by
//! wl-scheduling alone (`BookingConflict`, `SCH.RUL.05` and `SCH.RUL.06`): there
//! is no second flag or message here.
//!
//! **What it does not do.** It changes no tutor. A flag is resolved when the
//! tutor is changed, the session moved to an eligible date or cancelled (the
//! machine's effects, [`crate::flags::clear`]), or the document renewed, the
//! block over, the notice withdrawn (the recompute of the alert, which also
//! completes the task).
//!
//! **Verified, not trusted.** An event only names the tutor and the date: every
//! session is asked again, a document alert flags a session only when the
//! tutor's batch needs that document of them ([`crate::eligibility`]) and
//! `qualification_valid` says no on the session's date.
//!
//! Each event run and the daily run is a kernel rule run (`PLT.RUL.04`): a run
//! that fails is tried again by the job `edtech:tutor-flags` with a backoff and,
//! after the fifth failure, the owner and the company admin are alerted.

use std::collections::BTreeSet;

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::chrono::{Duration, NaiveDate};
use renox::db::{DateTime, now};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::automation::{RuleRun, RuleRunSpec, RuleRunStatus, run_rule};
use wl_kernel::compliance::{CriticalRule, occurrence_key, schedule_subject};
use wl_kernel::documents::{
    BlockEntry, BlockMode, DocumentAccepted, DocumentExpired, DocumentExpiring, OverrideMode,
    dubai_date, future_work, today,
};
use wl_kernel::qualifications::qualifications;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::scheduling::AvailabilityBlockAdded;
use wl_kernel::tickets::{TaskComment, task_creator};
use wl_kernel::workforce::{
    EMPLOYEE_SUBJECT_TYPE, EmployeeId, EmployeeNoticeGiven, EmployeeNoticeWithdrawn, LeaveApproved,
    QualificationExpired,
};

use crate::batch::Batch;
use crate::eligibility;
use crate::flags;
use crate::notices;
use crate::permissions;
use crate::permits::campus_scope;
use crate::seed::{POLICE_CLEARANCE_KIND, TUTOR_PERMIT_KIND};
use crate::session::{SESSION_SUBJECT_TYPE, Session, SessionStatus};
use crate::texts::english;
use crate::tutor_alert::{
    KIND_BLOCK, KIND_DOCUMENT, KIND_LEAVE, KIND_NOTICE, TutorAlert,
};

/// The rule id.
pub const RULE: &str = "EDU.RUL.12";

/// D: the days after the later of the date and today the sessions are flagged.
pub const WINDOW_DAYS: i64 = 30;

/// H: the hours before a session at which an unresolved flag goes to the centre
/// manager.
pub const ESCALATION_HOURS: i64 = 24;

/// The scheduled task and command of the daily run.
pub const TASK: &str = "edtech:tutor-alerts";

/// The time of day it runs, Asia/Dubai.
pub const TASK_AT: &str = "03:00";

/// The time zone of [`TASK_AT`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The scheduled job and command that tell the centre manager of the sessions
/// still flagged a day before they start and try the failed runs again.
pub const JOB: &str = "edtech:tutor-flags";

/// How often [`JOB`] runs.
pub const JOB_EVERY_MINUTES: u64 = 15;

/// Whether `LeaveApproved` is a trigger. Tutor leave (`HRM.LEV.01`) is Later for
/// 10, so it stays off; the handler is built and tested ([`on_leave`]).
pub const LEAVE_TRIGGER_ENABLED: bool = false;

/// The reason code of a flag raised by a document: `document:police_clearance`.
fn document_reason(kind: &str) -> String {
    format!("document:{kind}")
}

/// Whether `kind` is one of the two documents of a tutor.
fn is_tutor_kind(kind: &str) -> bool {
    kind == POLICE_CLEARANCE_KIND || kind == TUTOR_PERMIT_KIND
}

/// What a trigger says.
#[derive(Debug, Clone)]
struct Trigger {
    employee: i64,
    key: String,
    kind: &'static str,
    document_kind: Option<String>,
    from: NaiveDate,
    until: Option<NaiveDate>,
    period: Option<(DateTime, DateTime)>,
    detail: String,
}

/// The reason a flag reads as: `police clearance expires`, `on notice`.
pub fn reason_text(code: &str) -> String {
    if let Some(kind) = code.strip_prefix("document:") {
        return english(
            "edtech.alerts.reason.document",
            &[("kind", &eligibility::kind_text(kind))],
        );
    }
    english(&format!("edtech.alerts.reason.{code}"), &[])
}

/// Whether the employee teaches, or taught, any session here: only a tutor of
/// the centre has an alert.
async fn is_tutor(db: &Db, employee: i64) -> Result<bool> {
    Session::query()
        .where_(Session::TUTOR_EMPLOYEE_ID.eq(employee))
        .exists(db)
        .await
}

/// The alert of `trigger`, made once.
async fn upsert(db: &Db, trigger: &Trigger) -> Result<TutorAlert> {
    let found = TutorAlert::query()
        .where_(TutorAlert::EMPLOYEE_ID.eq(trigger.employee))
        .where_(TutorAlert::TRIGGER_KEY.eq(trigger.key.as_str()))
        .first(db)
        .await?;
    if let Some(found) = found {
        return Ok(found);
    }
    let mut alert = TutorAlert {
        employee_id: trigger.employee,
        trigger_key: trigger.key.clone(),
        kind: trigger.kind.to_owned(),
        document_kind: trigger.document_kind.clone(),
        from_date: trigger.from,
        until_date: trigger.until,
        starts_at: trigger.period.map(|period| period.0),
        ends_at: trigger.period.map(|period| period.1),
        detail: Some(trigger.detail.clone()),
        task_key: format!("edtech.tutor_alert:{}:{}", trigger.employee, trigger.key),
        ..TutorAlert::default()
    };
    alert.save(db).await?;
    Ok(alert)
}

/// The last day of the window of `alert` on `on`.
pub fn window_end(alert: &TutorAlert, on: NaiveDate) -> NaiveDate {
    let end = alert.from_date.max(on) + Duration::days(WINDOW_DAYS);
    alert.until_date.map_or(end, |until| end.min(until))
}

/// The `scheduled` sessions of the tutor that `alert` flags on `on`: dated in its
/// window (inside the block or the leave) and, for a document, taught under a
/// batch that needs the document of the tutor and on a day it does not cover.
///
/// # Errors
///
/// The database's and the guards' errors.
pub async fn affected_sessions(
    state: &AppState,
    alert: &TutorAlert,
    on: NaiveDate,
) -> Result<Vec<Session>> {
    let end = window_end(alert, on);
    if end < alert.from_date {
        return Ok(Vec::new());
    }
    let found = Session::query()
        .where_(Session::TUTOR_EMPLOYEE_ID.eq(alert.employee_id))
        .where_(Session::STATUS.eq(SessionStatus::Scheduled))
        .where_(Session::SESSION_DATE.gte(alert.from_date))
        .where_(Session::SESSION_DATE.lte(end))
        .order_by(Session::STARTS_AT)
        .get(&state.db)
        .await?;
    let mut kept = Vec::new();
    let mut needs: Vec<(i64, bool)> = Vec::new();
    for session in found {
        if let (Some(from), Some(until)) = (alert.starts_at, alert.ends_at)
            && (session.starts_at >= until || session.ends_at <= from)
        {
            continue;
        }
        if let Some(kind) = &alert.document_kind {
            let needed = match needs.iter().find(|(id, _)| *id == session.batch_id) {
                Some((_, needed)) => *needed,
                None => {
                    let needed = match Batch::find(&state.db, session.batch_id).await? {
                        Some(batch) => eligibility::required_kinds(
                            state,
                            Some(alert.employee_id),
                            &batch,
                        )
                        .await?
                        .iter()
                        .any(|required| *required == kind.as_str()),
                        None => false,
                    };
                    needs.push((session.batch_id, needed));
                    needed
                }
            };
            if !needed {
                continue;
            }
            let covered = qualifications(state)
                .qualification_valid(
                    state,
                    EmployeeId(alert.employee_id),
                    kind,
                    session.session_date,
                    None,
                    None,
                )
                .await?
                .valid;
            if covered {
                continue;
            }
        }
        kept.push(session);
    }
    Ok(kept)
}

/// The one-to-one bookings of the tutor in the window of a document alert, as
/// lines for the task. Their flag and escalation are wl-scheduling's.
async fn booking_lines(state: &AppState, alert: &TutorAlert, on: NaiveDate) -> Result<Vec<String>> {
    let Some(kind) = &alert.document_kind else {
        return Ok(Vec::new());
    };
    let Some(work) = future_work("booking.book") else {
        return Ok(Vec::new());
    };
    let entry = BlockEntry {
        kind: kind.clone(),
        guarded: "booking.book".to_owned(),
        mode: BlockMode::Block,
        override_mode: OverrideMode::Never,
        condition: None,
        locked: false,
    };
    let end = window_end(alert, on);
    let records = work
        .planned_from(
            state,
            &entry,
            &EmployeeId(alert.employee_id).subject(),
            alert.from_date,
        )
        .await?;
    Ok(records
        .into_iter()
        .filter(|record| record.date <= end)
        .map(|record| {
            english(
                "edtech.alerts.booking_line",
                &[("booking", &record.label), ("date", &record.date)],
            )
        })
        .collect())
}

/// The line of one session in the task.
fn session_line(batch: &str, session: &Session) -> String {
    english(
        "edtech.alerts.session_line",
        &[
            ("batch", &batch),
            ("number", &session.number),
            ("date", &session.session_date),
            ("time", &crate::sessions::time_text(session.starts_at)),
        ],
    )
}

/// The lines of `sessions`, with the number of their batch.
async fn session_lines(state: &AppState, sessions: &[Session]) -> Result<Vec<String>> {
    let mut batches: Vec<(i64, String)> = Vec::new();
    let mut lines = Vec::new();
    for session in sessions {
        let number = match batches.iter().find(|(id, _)| *id == session.batch_id) {
            Some((_, number)) => number.clone(),
            None => {
                let number = Batch::find(&state.db, session.batch_id)
                    .await?
                    .map_or_else(|| format!("#{}", session.batch_id), |batch| batch.number());
                batches.push((session.batch_id, number.clone()));
                number
            }
        };
        lines.push(session_line(&number, session));
    }
    Ok(lines)
}

/// Flags the sessions of `affected` that are not flagged yet; answers those it
/// flagged now.
async fn flag_sessions(
    state: &AppState,
    alert: &TutorAlert,
    affected: &[Session],
) -> Result<Vec<Session>> {
    let reason = match &alert.document_kind {
        Some(kind) => document_reason(kind),
        None => alert.kind.clone(),
    };
    let mut fresh = Vec::new();
    for session in affected {
        if session.is_flagged() {
            continue;
        }
        Session::query()
            .where_(Session::ID.eq(session.id))
            .update(
                &state.db,
                &[
                    ("flag_reason", &Some(reason.clone())),
                    ("flagged_at", &Some(now())),
                    ("flag_alert_id", &Some(alert.id)),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        fresh.push(session.clone());
    }
    Ok(fresh)
}

/// Takes the flag of `alert` off the sessions that are no longer affected (the
/// document was renewed, the block is over).
async fn unflag_resolved(state: &AppState, alert: &TutorAlert, keep: &[i64]) -> Result {
    let flagged = Session::query()
        .where_(Session::FLAG_ALERT_ID.eq(alert.id))
        .where_(Session::FLAG_REASON.is_not_null())
        .get(&state.db)
        .await?;
    let stale: Vec<i64> = flagged
        .iter()
        .map(|session| session.id)
        .filter(|id| !keep.contains(id))
        .collect();
    if stale.is_empty() {
        return Ok(());
    }
    let mut tx = state.db.begin().await?;
    flags::clear(&mut tx, &stale).await?;
    tx.commit().await?;
    Ok(())
}

/// Whether `alert` is over: the document is valid again on the day it lapsed,
/// the block or the leave is behind us, or the tutor has no session left from
/// the date of a notice.
async fn resolved(state: &AppState, alert: &TutorAlert, on: NaiveDate) -> Result<bool> {
    match alert.kind.as_str() {
        KIND_DOCUMENT => {
            let Some(kind) = &alert.document_kind else {
                return Ok(false);
            };
            Ok(qualifications(state)
                .qualification_valid(
                    state,
                    EmployeeId(alert.employee_id),
                    kind,
                    alert.from_date,
                    None,
                    None,
                )
                .await?
                .valid)
        }
        KIND_BLOCK | KIND_LEAVE => Ok(alert.until_date.is_some_and(|until| until < on)),
        KIND_NOTICE => {
            let left = Session::query()
                .where_(Session::TUTOR_EMPLOYEE_ID.eq(alert.employee_id))
                .where_(Session::STATUS.eq(SessionStatus::Scheduled))
                .where_(Session::SESSION_DATE.gte(alert.from_date))
                .exists(&state.db)
                .await?;
            Ok(!left)
        }
        _ => Ok(false),
    }
}

/// Closes `alert`: its flags come off, its task is completed with `note`.
async fn close(state: &AppState, alert: &TutorAlert, note: &str) -> Result {
    unflag_resolved(state, alert, &[]).await?;
    let creator = task_creator(state);
    if let Some(found) = creator.find_open(state, &alert.task_key).await? {
        creator.complete(state, found.id, note).await?;
    }
    TutorAlert::query()
        .where_(TutorAlert::ID.eq(alert.id))
        .update(
            &state.db,
            &[("closed_at", &Some(now())), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}

/// The coordinator's one task for `alert`: opened when the first sessions are
/// flagged, a comment when more enter the window.
async fn task(
    state: &AppState,
    alert: &TutorAlert,
    affected: &[Session],
    fresh: &[Session],
    bookings: &[String],
) -> Result {
    let creator = task_creator(state);
    let tutor = eligibility::tutor_text(state, Some(alert.employee_id)).await;
    let detail = alert.detail.clone().unwrap_or_default();
    if let Some(found) = creator.find_open(state, &alert.task_key).await? {
        if !fresh.is_empty() {
            let lines = session_lines(state, fresh).await?;
            let text = english(
                "edtech.alerts.task_update",
                &[("lines", &lines.join("\n"))],
            );
            creator
                .comment(state, found.id, TaskComment::internal(&text))
                .await?;
        }
        return Ok(());
    }
    let mut lines = session_lines(state, affected).await?;
    let mut text = english(
        "edtech.alerts.task_body",
        &[
            ("tutor", &tutor),
            ("detail", &detail),
            ("sessions", &lines.join("\n")),
        ],
    );
    if !bookings.is_empty() {
        lines = bookings.to_vec();
        text.push_str(&english(
            "edtech.alerts.task_bookings",
            &[("bookings", &lines.join("\n"))],
        ));
    }
    let title = english(
        "edtech.alerts.task_title",
        &[("tutor", &tutor), ("detail", &detail)],
    );
    let mut task = RoleTask::new(
        EmployeeId(alert.employee_id).subject(),
        permissions::COORDINATOR_ROLE,
        &title,
        &alert.task_key,
    )
    .description(&text);
    if let Some(first) = affected.first() {
        task = task.due_at(first.starts_at - Duration::hours(ESCALATION_HOURS));
    }
    role_tasks::open(state, task).await?;
    Ok(())
}

/// One run of the rule for `alert_id` on `on`: flags the sessions in the window,
/// takes the flag off those no longer affected, keeps the coordinator's task and
/// closes the alert when it is over.
///
/// # Errors
///
/// The database's, the guards' and the task creator's errors (the rule run
/// tries again).
pub async fn apply(state: &AppState, alert_id: i64, on: NaiveDate) -> Result {
    let alert = TutorAlert::find_or_404(&state.db, alert_id).await?;
    if !alert.is_open() {
        return Ok(());
    }
    if resolved(state, &alert, on).await? {
        let note = english("edtech.alerts.resolved", &[]);
        return close(state, &alert, &note).await;
    }
    let affected = affected_sessions(state, &alert, on).await?;
    let ids: Vec<i64> = affected.iter().map(|session| session.id).collect();
    let fresh = flag_sessions(state, &alert, &affected).await?;
    unflag_resolved(state, &alert, &ids).await?;
    let bookings = booking_lines(state, &alert, on).await?;
    if !affected.is_empty() || !bookings.is_empty() {
        task(state, &alert, &affected, &fresh, &bookings).await?;
    }
    Ok(())
}

/// Raises `trigger` for its tutor: the alert, then a rule run named `event`.
async fn raise(state: &AppState, trigger: Trigger, event: &str) -> Result {
    if !is_tutor(&state.db, trigger.employee).await? {
        return Ok(());
    }
    let alert = upsert(&state.db, &trigger).await?;
    let spec = RuleRunSpec::new(
        RULE,
        EmployeeId(trigger.employee).subject(),
        format!("alert:{}:{event}", alert.id),
    )
    .compliance_critical();
    let id = alert.id;
    run_rule(state, spec, move |state: AppState| async move {
        apply(&state, id, today()).await
    })
    .await?;
    Ok(())
}

/// `DocumentExpiring`, `DocumentExpired` and `QualificationExpired` of a tutor's
/// police clearance or permit: from the day after `expires_on`.
///
/// # Errors
///
/// The database's errors (a failing run is tried again, not returned).
pub async fn on_document(
    state: &AppState,
    employee: &Subject,
    kind: &str,
    expires_on: NaiveDate,
    event: &str,
) -> Result {
    if employee.subject_type != EMPLOYEE_SUBJECT_TYPE || !is_tutor_kind(kind) {
        return Ok(());
    }
    let detail = english(
        "edtech.alerts.detail.document",
        &[
            ("kind", &eligibility::kind_text(kind)),
            ("date", &expires_on),
        ],
    );
    let trigger = Trigger {
        employee: employee.subject_id,
        key: format!("document:{kind}:{expires_on}"),
        kind: KIND_DOCUMENT,
        document_kind: Some(kind.to_owned()),
        from: expires_on + Duration::days(1),
        until: None,
        period: None,
        detail,
    };
    raise(state, trigger, event).await
}

/// `EmployeeNoticeGiven`: from the day after the last working day.
///
/// # Errors
///
/// The database's errors (a failing run is tried again, not returned).
pub async fn on_notice(state: &AppState, event: &EmployeeNoticeGiven) -> Result {
    let trigger = Trigger {
        employee: event.employee.0,
        key: format!("notice:{}", event.last_working_day),
        kind: KIND_NOTICE,
        document_kind: None,
        from: event.last_working_day + Duration::days(1),
        until: None,
        period: None,
        detail: english(
            "edtech.alerts.detail.notice",
            &[("date", &event.last_working_day)],
        ),
    };
    raise(state, trigger, "notice").await
}

/// `EmployeeNoticeWithdrawn`: the open notice alerts of the tutor are closed and
/// their sessions are no longer flagged.
///
/// # Errors
///
/// The database's and the task creator's errors.
pub async fn on_notice_withdrawn(state: &AppState, event: &EmployeeNoticeWithdrawn) -> Result {
    for alert in TutorAlert::of_employee(&state.db, event.employee.0).await? {
        if alert.is_open() && alert.kind == KIND_NOTICE {
            let note = english("edtech.alerts.withdrawn", &[]);
            close(state, &alert, &note).await?;
        }
    }
    Ok(())
}

/// A manual availability block on the tutor: only the sessions inside the block
/// are flagged. A block of another source (a leave, a roster, a holiday, or the
/// tutor's own sessions) is not a trigger here.
///
/// # Errors
///
/// The database's errors (a failing run is tried again, not returned).
pub async fn on_block(state: &AppState, event: &AvailabilityBlockAdded) -> Result {
    let Some(employee) = event.employee else {
        return Ok(());
    };
    if event.source != "manual" {
        return Ok(());
    }
    let from = dubai_date(event.starts_at);
    let until = dubai_date(event.ends_at - Duration::seconds(1));
    let trigger = Trigger {
        employee,
        key: format!("block:{}", event.block),
        kind: KIND_BLOCK,
        document_kind: None,
        from,
        until: Some(until),
        period: Some((event.starts_at, event.ends_at)),
        detail: english(
            "edtech.alerts.detail.block",
            &[("from", &from), ("until", &until)],
        ),
    };
    raise(state, trigger, "block").await
}

/// `LeaveApproved`: only the sessions inside the leave are flagged. Does nothing
/// unless `enabled` (tutor leave, `HRM.LEV.01`, is Later for 10).
///
/// # Errors
///
/// The database's errors (a failing run is tried again, not returned).
pub async fn on_leave(state: &AppState, event: &LeaveApproved, enabled: bool) -> Result {
    if !enabled {
        return Ok(());
    }
    let trigger = Trigger {
        employee: event.employee.0,
        key: format!("leave:{}", event.request()),
        kind: KIND_LEAVE,
        document_kind: None,
        from: event.from,
        until: Some(event.to),
        period: None,
        detail: english(
            "edtech.alerts.detail.leave",
            &[("from", &event.from), ("until", &event.to)],
        ),
    };
    raise(state, trigger, "leave").await
}

/// `DocumentAccepted`: a renewal. The open alerts of the tutor for that kind
/// are recomputed at once, so the flags come off without waiting for the night.
///
/// # Errors
///
/// The database's, the guards' and the task creator's errors.
pub async fn on_renewal(state: &AppState, event: &DocumentAccepted) -> Result {
    if event.subject.subject_type != EMPLOYEE_SUBJECT_TYPE || !is_tutor_kind(&event.kind) {
        return Ok(());
    }
    for alert in TutorAlert::of_employee(&state.db, event.subject.subject_id).await? {
        if alert.is_open() && alert.document_kind.as_deref() == Some(event.kind.as_str()) {
            apply(state, alert.id, today()).await?;
        }
    }
    Ok(())
}

/// The daily run on `on`: every open alert is recomputed, so the sessions that
/// entered the window are flagged and added to the task, and what was renewed or
/// is over comes off. It is one rule run per day.
///
/// # Errors
///
/// The database's errors; a failed run is tried again by [`retry`].
pub async fn run_daily(state: &AppState, on: NaiveDate) -> Result {
    let spec = RuleRunSpec::new(RULE, schedule_subject(), occurrence_key(RULE, on))
        .compliance_critical();
    run_rule(state, spec, move |state: AppState| async move {
        recompute_all(&state, on).await
    })
    .await?;
    Ok(())
}

/// Recomputes every open alert on `on`; the first error is returned after all
/// were tried.
async fn recompute_all(state: &AppState, on: NaiveDate) -> Result {
    let mut failure = None;
    for alert in TutorAlert::open_ones(&state.db).await? {
        if let Err(error) = apply(state, alert.id, on).await {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// The centre managers to tell about `batch`: those of its campus, else the
/// global ones.
async fn managers(state: &AppState, batch: &Batch) -> Result<Vec<i64>> {
    let scope = match batch.campus() {
        Some(campus) => campus_scope(&state.db, &campus).await?,
        None => Scope::global(),
    };
    let mut ids = BTreeSet::new();
    for user in users_with_role_in(&state.db, permissions::CENTRE_MANAGER_ROLE, &scope).await? {
        ids.insert(user.id);
    }
    Ok(ids.into_iter().collect())
}

/// Tells the centre manager that `session` is still flagged and starts within
/// [`ESCALATION_HOURS`]. Once: the session keeps `flag_escalated_at`. Answers
/// whether a message was asked for.
async fn escalate_one(state: &AppState, session_id: i64, at: DateTime) -> Result<bool> {
    let session = Session::find_or_404(&state.db, session_id).await?;
    if session.status != SessionStatus::Scheduled
        || !session.is_flagged()
        || session.flag_escalated_at.is_some()
    {
        return Ok(false);
    }
    let batch = Batch::find_or_404(&state.db, session.batch_id).await?;
    let users = managers(state, &batch).await?;
    let reason = session
        .flag_reason
        .as_deref()
        .map(reason_text)
        .unwrap_or_default();
    let label = format!("{} · {}", batch.number(), session.number);
    let time = crate::sessions::time_text(session.starts_at);
    let params: [(&str, &dyn std::fmt::Display); 4] = [
        ("session", &label),
        ("date", &session.session_date),
        ("time", &time),
        ("reason", &reason),
    ];
    let told = notices::tell_users(
        state,
        session.subject(),
        &label,
        &users,
        &english("edtech.alerts.escalation_title", &params),
        &english("edtech.alerts.escalation_body", &params),
    )
    .await?;
    if told {
        Session::query()
            .where_(Session::ID.eq(session.id))
            .update(
                &state.db,
                &[("flag_escalated_at", &Some(at)), ("updated_at", &now())],
            )
            .await?;
    }
    Ok(told)
}

/// At `at`, the centre manager is told once of each session that is still
/// flagged and starts within [`ESCALATION_HOURS`] hours. Each session is one
/// rule run, tried again when the message fails. Answers the sessions told.
///
/// # Errors
///
/// The database's errors.
pub async fn escalate(state: &AppState, at: DateTime) -> Result<usize> {
    let due = Session::query()
        .where_(Session::STATUS.eq(SessionStatus::Scheduled))
        .where_(Session::FLAG_REASON.is_not_null())
        .where_(Session::FLAG_ESCALATED_AT.is_null())
        .where_(Session::STARTS_AT.gt(at))
        .where_(Session::STARTS_AT.lte(at + Duration::hours(ESCALATION_HOURS)))
        .order_by(Session::STARTS_AT)
        .get(&state.db)
        .await?;
    let mut told = 0;
    for session in due {
        let spec = RuleRunSpec::new(RULE, session.subject(), "escalate").compliance_critical();
        let id = session.id;
        run_rule(state, spec, move |state: AppState| async move {
            escalate_one(&state, id, at).await.map(|_| ())
        })
        .await?;
        if Session::find_or_404(&state.db, id)
            .await?
            .flag_escalated_at
            .is_some()
        {
            told += 1;
        }
    }
    Ok(told)
}

/// Tries again the runs of the rule that failed and wait: an alert, an
/// escalation or a daily run (the backoff of the kernel decides when). After
/// five attempts the kernel alerts the owner and the company admin.
///
/// # Errors
///
/// The first error of the runs.
pub async fn retry(state: &AppState) -> Result {
    let waiting = RuleRun::query()
        .where_(RuleRun::RULE.eq(RULE))
        .where_(RuleRun::STATUS.eq(RuleRunStatus::Retrying))
        .order_by(RuleRun::ID)
        .get(&state.db)
        .await?;
    let mut failure = None;
    for run in waiting {
        let spec = RuleRunSpec::new(RULE, run.subject(), run.occurrence_key.clone())
            .compliance_critical();
        let outcome = if run.subject_type == SESSION_SUBJECT_TYPE {
            let id = run.subject_id;
            run_rule(state, spec, move |state: AppState| async move {
                escalate_one(&state, id, now()).await.map(|_| ())
            })
            .await
        } else if run.subject_type == EMPLOYEE_SUBJECT_TYPE {
            let Some(id) = run
                .occurrence_key
                .split(':')
                .nth(1)
                .and_then(|text| text.parse::<i64>().ok())
            else {
                continue;
            };
            run_rule(state, spec, move |state: AppState| async move {
                apply(&state, id, today()).await
            })
            .await
        } else {
            let Some(on) = run
                .occurrence_key
                .rsplit(':')
                .next()
                .and_then(|text| text.parse::<NaiveDate>().ok())
            else {
                continue;
            };
            run_rule(state, spec, move |state: AppState| async move {
                recompute_all(&state, on).await
            })
            .await
        };
        if let Err(error) = outcome {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// Marks `EDU.RUL.12` compliance-critical, owned by the owner and expected every
/// night (`PLT.RUL.04`).
pub(crate) fn register() {
    CriticalRule::new(RULE)
        .owner_role(permissions::OWNER_ROLE)
        .daily_at(TASK_AT)
        .register();
}

/// Registers the listeners, the daily run and the job.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: DocumentExpiring, state: AppState| async move {
        on_document(&state, &event.subject, &event.kind, event.expires_on, "expiring").await
    });
    app.listen(|event: DocumentExpired, state: AppState| async move {
        on_document(&state, &event.subject, &event.kind, event.expires_on, "expired").await
    });
    app.listen(|event: QualificationExpired, state: AppState| async move {
        on_document(
            &state,
            &event.employee.subject(),
            &event.kind,
            event.expires_on,
            "qualification",
        )
        .await
    });
    app.listen(|event: EmployeeNoticeGiven, state: AppState| async move {
        on_notice(&state, &event).await
    });
    app.listen(|event: EmployeeNoticeWithdrawn, state: AppState| async move {
        on_notice_withdrawn(&state, &event).await
    });
    app.listen(|event: AvailabilityBlockAdded, state: AppState| async move {
        on_block(&state, &event).await
    });
    app.listen(|event: LeaveApproved, state: AppState| async move {
        on_leave(&state, &event, LEAVE_TRIGGER_ENABLED).await
    });
    app.listen(|event: DocumentAccepted, state: AppState| async move {
        on_renewal(&state, &event).await
    });
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run_daily(&state, today()).await
        })
        .timezone(TASK_ZONE);
    app.schedule()
        .every_minutes(JOB_EVERY_MINUTES, JOB, |state: AppState| async move {
            escalate(&state, now()).await?;
            retry(&state).await
        });
    app.command(
        TASK,
        "Flag the sessions of the tutors who will not be eligible or available, in the 30 day window",
        |_args, state: AppState| async move {
            run_daily(&state, today()).await?;
            println!("The tutor alerts are up to date.");
            Ok(())
        },
    );
    app.command(
        JOB,
        "Tell the centre manager of the flagged sessions that start within a day and try the failed runs again",
        |_args, state: AppState| async move {
            let told = escalate(&state, now()).await?;
            retry(&state).await?;
            println!("{told} sessions escalated.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(from: (i32, u32, u32), until: Option<(i32, u32, u32)>) -> TutorAlert {
        TutorAlert {
            from_date: NaiveDate::from_ymd_opt(from.0, from.1, from.2).unwrap(),
            until_date: until.map(|u| NaiveDate::from_ymd_opt(u.0, u.1, u.2).unwrap()),
            ..TutorAlert::default()
        }
    }

    #[test]
    fn the_window_ends_thirty_days_after_the_later_of_the_date_and_today() {
        let clearance = alert((2026, 12, 16), None);
        let day = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
        // Before the date: the date to the date plus 30.
        assert_eq!(window_end(&clearance, day(11, 20)), day(12, 16) + Duration::days(30));
        // After it the window moves with the run: the run on 2026-12-21 covers
        // 2026-12-16 to 2027-01-20.
        assert_eq!(
            window_end(&clearance, day(12, 21)),
            NaiveDate::from_ymd_opt(2027, 1, 20).unwrap()
        );
        // The run on 2026-12-20 ends on 2027-01-19.
        assert_eq!(
            window_end(&clearance, day(12, 20)),
            NaiveDate::from_ymd_opt(2027, 1, 19).unwrap()
        );
    }

    #[test]
    fn a_block_ends_the_window_at_its_last_day() {
        let block = alert((2026, 11, 20), Some((2026, 11, 22)));
        let day = |d| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        assert_eq!(window_end(&block, day(1)), day(22));
        assert_eq!(window_end(&block, day(21)), day(22));
    }

    #[test]
    fn the_reasons_read_in_words() {
        assert!(reason_text("document:police_clearance").contains("police clearance"));
        assert!(reason_text("notice").contains("notice"));
        assert!(reason_text("block").contains("unavailable"));
    }

    #[test]
    fn only_the_two_tutor_documents_are_triggers() {
        assert!(is_tutor_kind("police_clearance") && is_tutor_kind("tutor_permit"));
        assert!(!is_tutor_kind("centre_permit"));
        assert_eq!(document_reason("tutor_permit"), "document:tutor_permit");
    }
}
