//! The owner alert of a compliance-critical failure (wl-kernel §5.15,
//! `PLT.RUL.04`).
//!
//! - **A run fails or is stopped.** [`crate::automation::run_rule`] calls
//!   [`raise`] in the same job as the transition, for a rule registered with
//!   [`CriticalRule`](super::CriticalRule) (or marked on its
//!   `RuleRunSpec`). `retrying` alerts nothing.
//! - **A scheduled run is missed.** [`check_missed`], the scheduled task
//!   `compliance:check` every 15 minutes (:00, :15, :30, :45), finds the
//!   occurrences of watched rules planned 60 minutes or more ago with no rule
//!   run and calls [`raise`] for each, once.
//!
//! [`raise`] stores one alert per occurrence, tells the business owner role
//! and the company admin (in-app and e-mail, through the notice contract) and
//! opens a task for the owner role. The alert closes when the rule runs
//! successfully for the occurrence ([`run_succeeded`], called from
//! `run_rule`; the task is completed with "rule ran successfully at <time>")
//! or when the owner records how the duty was met by hand ([`close_by_hand`],
//! a note is required).

use std::collections::BTreeSet;

use renox::AuthUser;
use renox::auth::permissions::users_with_role;
use renox::auth::{Channel, DatabaseMessage, Notification, Recipient};
use renox::chrono::Duration;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::serde_json::json;

use super::alert::{AlertKind, AlertStatus, ClosedHow, ComplianceAlert};
use super::registry::{LOOK_BACK_DAYS, all, missed_before};
use super::rules::{ComplianceRule, owner_role};
use crate::automation::RuleRun;
use crate::documents::dubai_text;
use crate::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest, Urgency};
use crate::role_tasks::{self, RoleTask};
use crate::scopes::COMPANY_ADMIN_ROLE;
use crate::subject::Subject;
use crate::texts::{en, en_with};
use crate::tickets::{self, TicketId};

/// The scheduled task that runs [`check_missed`] every 15 minutes.
pub const TASK: &str = "compliance:check";

/// The rule id of the alert (`PLT.RUL.04`).
pub const RULE: &str = "PLT.RUL.04";

/// The notification type of the owner alert.
pub const NOTIFICATION_TYPE: &str = "kernel.compliance_alert";

/// The notification type `kernel.compliance_alert`.
pub fn notification_type() -> NotificationTypeDef {
    let subject = en("wl_kernel.compliance.alert.subject");
    let body = en("wl_kernel.compliance.alert.body");
    NotificationTypeDef::new(NOTIFICATION_TYPE, "kernel")
        .group("kernel")
        .category(Category::Reminder)
        .urgency(Urgency::Urgent)
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// What to raise an alert for: the occurrence that failed, was stopped or
/// was missed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Raised {
    /// The rule id.
    pub rule: String,
    /// The record the run is about.
    pub subject: Subject,
    /// The occurrence key.
    pub occurrence_key: String,
    /// Failed, stopped or missed.
    pub kind: AlertKind,
    /// The error of a failed or stopped run.
    pub error: Option<String>,
    /// When a missed occurrence was planned.
    pub planned_at: Option<DateTime>,
}

impl Raised {
    /// The occurrence `occurrence_key` of `rule` about `subject`.
    pub fn new(rule: &str, subject: Subject, occurrence_key: &str, kind: AlertKind) -> Self {
        Raised {
            rule: rule.to_owned(),
            subject,
            occurrence_key: occurrence_key.to_owned(),
            kind,
            error: None,
            planned_at: None,
        }
    }

    /// With the run's error.
    pub fn error(mut self, error: &str) -> Self {
        self.error = Some(error.to_owned());
        self
    }

    /// With the time a missed occurrence was planned.
    pub fn planned_at(mut self, at: DateTime) -> Self {
        self.planned_at = Some(at);
        self
    }
}

/// The users to tell: the company admins and the holders of `owner_role`,
/// each once, by id.
pub async fn recipients(db: &Db, owner_role: &str) -> Result<Vec<i64>> {
    let mut ids = BTreeSet::new();
    for role in [COMPANY_ADMIN_ROLE, owner_role] {
        for user in users_with_role(db, role).await? {
            ids.insert(user.id);
        }
    }
    Ok(ids.into_iter().collect())
}

/// What happened, in words, for the alert and the task.
fn what(alert: &ComplianceAlert) -> String {
    match alert.kind {
        AlertKind::Failed => en("wl_kernel.compliance.what.failed"),
        AlertKind::Stopped => en("wl_kernel.compliance.what.stopped"),
        AlertKind::Missed => en_with(
            "wl_kernel.compliance.what.missed",
            &[(
                "planned",
                &alert.planned_at.map_or_else(|| "—".to_owned(), dubai_text),
            )],
        ),
    }
}

/// Raises the alert of an occurrence: stores it, tells the owner role and
/// the company admin at once, and opens a task for the owner role. One alert
/// per occurrence: a second call for the same occurrence (a retry, a later
/// check) does nothing and returns `None`.
pub async fn raise(state: &AppState, raised: Raised) -> Result<Option<ComplianceAlert>> {
    let db = &state.db;
    if ComplianceAlert::of_occurrence(db, &raised.rule, &raised.subject, &raised.occurrence_key)
        .await?
        .is_some()
    {
        return Ok(None);
    }
    let owner = owner_role(state, &raised.rule).await?;
    let created = ComplianceAlert::create(
        db,
        ComplianceAlert {
            rule: raised.rule.clone(),
            subject_type: raised.subject.subject_type.clone(),
            subject_id: raised.subject.subject_id,
            occurrence_key: raised.occurrence_key.clone(),
            kind: raised.kind,
            error: raised.error.clone(),
            planned_at: raised.planned_at,
            owner_role: owner,
            status: AlertStatus::Open,
            ..Default::default()
        },
    )
    .await;
    let alert = match created {
        Ok(alert) => alert,
        // Another worker raised it first.
        Err(error) if error.is_unique_violation() => return Ok(None),
        Err(error) => return Err(error),
    };
    tell(state, &alert).await?;
    Ok(Some(open_task(state, alert).await?))
}

/// Tells the company admin and the owner role, in the app and by e-mail.
async fn tell(state: &AppState, alert: &ComplianceAlert) -> Result {
    let users = recipients(&state.db, &alert.owner_role).await?;
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(NOTIFICATION_TYPE, alert.subject())
        .subject_label(&alert.rule)
        .data(json!({
            "rule": alert.rule,
            "what": what(alert),
            "error": alert.error.clone().unwrap_or_default(),
            "at": dubai_text(now()),
        }));
    for id in &users {
        request = request.to_user(*id);
    }
    notify::notifier(state).notify(state, request).await?;
    // The in-app side: the bell of every user the e-mail went to.
    let notice = AlertNotice::of(alert);
    for id in users {
        if let Some(user) = User::find(&state.db, id).await? {
            state.notify(&user, &notice).await?;
        }
    }
    Ok(())
}

/// The kind of the in-app notice of an owner alert.
pub const ALERT_KIND: &str = "compliance-alert";

/// The in-app notice (the bell) that goes with the e-mail of an owner alert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertNotice {
    /// The alert's id.
    pub alert: i64,
    /// The rule.
    pub rule: String,
    /// What happened, in words.
    pub what: String,
    /// The record the run is about.
    pub subject: Subject,
}

impl AlertNotice {
    /// The notice of `alert`.
    pub fn of(alert: &ComplianceAlert) -> Self {
        AlertNotice {
            alert: alert.id,
            rule: alert.rule.clone(),
            what: what(alert),
            subject: alert.subject(),
        }
    }
}

impl Notification for AlertNotice {
    fn kind(&self) -> &'static str {
        ALERT_KIND
    }

    fn channels(&self, _to: &Recipient) -> Vec<Channel> {
        vec![Channel::Database]
    }

    fn to_database(&self, _to: &Recipient, _state: &AppState) -> Result<renox::serde_json::Value> {
        let title = en_with(
            "wl_kernel.compliance.task_title",
            &[("rule", &self.rule), ("what", &self.what)],
        );
        let body = en("wl_kernel.compliance.inapp_body");
        Ok(DatabaseMessage::error(title)
            .body(body)
            .with("compliance_alert", self.alert)
            .with("rule", &self.rule)
            .with("subject_type", &self.subject.subject_type)
            .with("subject_id", self.subject.subject_id)
            .into())
    }
}

/// The key the owner's task is found by while it is open.
fn source_key(alert: &ComplianceAlert) -> String {
    format!("kernel.compliance:{}", alert.id)
}

/// Opens the owner's task and records it on the alert; the alert as it is
/// then. Without the ticketing module there is no task.
async fn open_task(state: &AppState, alert: ComplianceAlert) -> Result<ComplianceAlert> {
    let title = en_with(
        "wl_kernel.compliance.task_title",
        &[("rule", &alert.rule), ("what", &what(&alert))],
    );
    let description = en_with(
        "wl_kernel.compliance.task_description",
        &[
            ("rule", &alert.rule),
            (
                "error",
                &alert.error.clone().unwrap_or_else(|| "—".to_owned()),
            ),
        ],
    );
    let task = RoleTask::new(
        alert.subject(),
        &alert.owner_role,
        &title,
        &source_key(&alert),
    )
    .description(&description);
    let Some(ticket) = role_tasks::open(state, task).await? else {
        return Ok(alert);
    };
    ComplianceAlert::where_eq("id", alert.id)
        .update(
            &state.db,
            &[
                ("ticket_id", &Some(ticket.id.0)),
                ("ticket_number", &Some(ticket.number.clone())),
                ("updated_at", &Some(now())),
            ],
        )
        .await?;
    ComplianceAlert::find(&state.db, alert.id)
        .await?
        .ok_or(Error::NotFound)
}

/// Closes the open alert `id` with `how` and `note`, as one conditional
/// update. Returns whether it closed.
async fn close(
    db: &Db,
    id: i64,
    how: ClosedHow,
    by: Option<i64>,
    note: &str,
    attachment: Option<i64>,
) -> Result<bool> {
    let at = now();
    let closed = ComplianceAlert::where_eq("id", id)
        .where_eq("status", AlertStatus::Open)
        .update(
            db,
            &[
                ("status", &AlertStatus::Closed),
                ("closed_how", &Some(how)),
                ("closed_by", &by),
                ("note", &Some(note.to_owned())),
                ("attachment_document_id", &attachment),
                ("closed_at", &Some(at)),
                ("updated_at", &Some(at)),
            ],
        )
        .await?;
    Ok(closed > 0)
}

/// Completes the alert's task, if it has one.
async fn complete_task(state: &AppState, alert: &ComplianceAlert, note: &str) -> Result {
    if let Some(ticket) = alert.ticket_id {
        tickets::task_creator(state)
            .complete(state, TicketId(ticket), note)
            .await?;
    }
    Ok(())
}

/// The rule ran successfully for the occurrence (a rerun by
/// `automations.retry`, or the run a missed occurrence was waiting for): its
/// open alert closes, and the task with it, "rule ran successfully at
/// <time>". Does nothing for an occurrence with no open alert.
pub async fn run_succeeded(
    state: &AppState,
    rule: &str,
    subject: &Subject,
    occurrence_key: &str,
) -> Result<bool> {
    let Some(alert) =
        ComplianceAlert::of_occurrence(&state.db, rule, subject, occurrence_key).await?
    else {
        return Ok(false);
    };
    if !alert.is_open() {
        return Ok(false);
    }
    let note = en_with(
        "wl_kernel.compliance.rerun_note",
        &[("time", &dubai_text(now()))],
    );
    if !close(&state.db, alert.id, ClosedHow::Rerun, None, &note, None).await? {
        return Ok(false);
    }
    complete_task(state, &alert, &note).await?;
    Ok(true)
}

/// The owner records how the duty was met by hand: the alert and its task
/// close. A note is required; the attachment is the id of a document
/// (wl-documents) the owner uploaded, if any.
///
/// # Errors
///
/// 403 unless the user holds the alert's owner role or is a company admin;
/// 404 for an unknown alert; 422 for a blank note, or an alert that is
/// already closed.
pub async fn close_by_hand(
    state: &AppState,
    user: &AuthUser,
    alert_id: i64,
    note: &str,
    attachment: Option<i64>,
) -> Result<ComplianceAlert> {
    let alert = ComplianceAlert::find(&state.db, alert_id)
        .await?
        .ok_or(Error::NotFound)?;
    if !(user.has_role(&alert.owner_role) || user.has_role(COMPANY_ADMIN_ROLE)) {
        return Err(Error::Forbidden);
    }
    let note = note.trim();
    if note.is_empty() {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            en("wl_kernel.compliance.note_required"),
        ));
    }
    if !alert.is_open()
        || !close(
            &state.db,
            alert.id,
            ClosedHow::Manual,
            Some(user.id),
            note,
            attachment,
        )
        .await?
    {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            en("wl_kernel.compliance.already_closed"),
        ));
    }
    complete_task(state, &alert, note).await?;
    ComplianceAlert::find(&state.db, alert.id)
        .await?
        .ok_or(Error::NotFound)
}

/// The check for missed runs (wl-kernel §5.15 step 4), meant to run at :00,
/// :15, :30 and :45 Asia/Dubai: for each watched rule it lists the
/// occurrences planned from the time the rule is watched (at most
/// [`LOOK_BACK_DAYS`] days back) to 60 minutes ago, and raises an alert for
/// each one with no rule run. An occurrence counts as missed once. Returns
/// how many alerts were raised.
pub async fn check_missed(state: &AppState) -> Result<usize> {
    let db = &state.db;
    let at = now();
    let until = missed_before(at);
    let oldest = at - Duration::days(LOOK_BACK_DAYS);
    let mut raised = 0;
    for critical in all() {
        if critical.schedule().is_none() {
            continue;
        }
        let watched = ComplianceRule::of(db, critical.rule_id())
            .await?
            .and_then(|row| row.watched_since);
        let Some(watched) = watched else {
            continue;
        };
        let from = watched.max(oldest);
        if from > until {
            continue;
        }
        for planned in critical.planned(state, from, until).await? {
            if RuleRun::find_occurrence(
                db,
                critical.rule_id(),
                &planned.subject,
                &planned.occurrence_key,
            )
            .await?
            .is_some()
            {
                continue;
            }
            let found = raise(
                state,
                Raised::new(
                    critical.rule_id(),
                    planned.subject.clone(),
                    &planned.occurrence_key,
                    AlertKind::Missed,
                )
                .planned_at(planned.planned_at),
            )
            .await?;
            raised += usize::from(found.is_some());
        }
    }
    Ok(raised)
}

/// Registers the notification type, the scheduled task [`TASK`] (every 15
/// minutes) and the command of the same name.
pub(crate) fn schedule(app: &mut Registry) {
    notify::register_type(notification_type());
    app.schedule()
        .every_minutes(15, TASK, |state: AppState| async move {
            check_missed(&state).await.map(|_| ())
        });
    app.command(
        TASK,
        "Raise the owner alert for compliance-critical scheduled runs that are 60 minutes or \
         more late",
        |_args, state: AppState| async move {
            let raised = check_missed(&state).await?;
            println!("{raised} missed runs raised.");
            Ok(())
        },
    );
}
