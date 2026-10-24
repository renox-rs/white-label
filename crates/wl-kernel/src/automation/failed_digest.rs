//! The daily digest of failed rule runs (wl-kernel §4.2, §6 `PLT.RUL.03`):
//! every morning at 07:00 Asia/Dubai each company admin gets one message,
//! by e-mail and in the app, listing the rule runs that reached `failed` or
//! `stopped` from 07:00 the day before to 07:00 today, so that an ordinary
//! failure (a reminder that did not go out) is seen and rerun.
//!
//! - Each line names the rule, the record (with its number when the module
//!   registered one, [`crate::digest::register_info`]), the occurrence, the
//!   reason (*failed* or *stopped*), the last error and the time, and links
//!   to the run's page, where `rerun` is offered (`automations.retry`).
//! - A compliance-critical run is listed too, marked *compliance-critical,
//!   alert sent* once `PLT.RUL.04` told the owner; a run rerun with success
//!   before the digest is marked *rerun, succeeded*.
//! - Nothing failed in the period: no digest. Each admin's digest is a rule
//!   run keyed by the date, so running the job again for the same day sends
//!   nothing new.
//! - The period is by the time the run failed or stopped
//!   ([`RuleRunFailure`]), as for the `PLT.RUL.06` digest
//!   ([`crate::digest::period_for`]).

use renox::auth::permissions::users_with_role;
use renox::auth::{Channel, DatabaseMessage, Notification, Recipient};
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;

use super::failures::{FailureKind, RuleRunFailure};
use super::run::{RuleRun, RuleRunSpec, RuleRunStatus, RunOutcome, run_rule};
use super::web::SHOW_ROUTE;
use crate::compliance::{self, ComplianceAlert};
use crate::digest::{info_of, period_for, record_name, rule_title};
use crate::documents::{dubai_text, today};
use crate::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use crate::scopes::COMPANY_ADMIN_ROLE;
use crate::subject::Subject;
use crate::texts::{en, en_with};

/// The rule id of the digest.
pub const RULE: &str = "PLT.RUL.03";

/// The scheduled task, and the command, that send the digest.
pub const TASK: &str = "digest:failed-runs";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "07:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The notification type of the digest.
pub const NOTIFICATION_TYPE: &str = "kernel.failed_runs_digest";

/// The subject type the digest of one admin is a rule run about.
pub const SUBJECT_TYPE: &str = "kernel.failed_runs_digest";

/// The kind of the in-app notice of the digest.
pub const NOTICE_KIND: &str = "failed-runs-digest";

/// One failed or stopped run, as the digest lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedLine {
    /// The rule run.
    pub run_id: i64,
    /// The rule's title (or its id).
    pub rule: String,
    /// The record, named (`invoice INV-ACME-2026-0310`).
    pub record: String,
    /// The occurrence key.
    pub occurrence: String,
    /// Failed or stopped.
    pub kind: FailureKind,
    /// The last error.
    pub error: Option<String>,
    /// When it failed or stopped.
    pub at: DateTime,
    /// Whether the rule is compliance-critical.
    pub critical: bool,
    /// Whether `PLT.RUL.04` told the owner.
    pub alert_sent: bool,
    /// Whether the run was rerun and succeeded since.
    pub rerun_succeeded: bool,
    /// The run's page.
    pub link: String,
}

/// What a run of the job did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Failed or stopped runs in the period.
    pub runs: usize,
    /// Admins who were sent their digest.
    pub sent: usize,
    /// Admins whose digest for this date was sent before.
    pub skipped: usize,
    /// Admins whose digest failed (it is retried by the next run).
    pub failed: usize,
}

/// The notification type `kernel.failed_runs_digest`.
pub fn notification_type() -> NotificationTypeDef {
    let subject = en("wl_kernel.failed_runs.subject");
    NotificationTypeDef::new(NOTIFICATION_TYPE, "kernel")
        .group("kernel")
        .category(Category::Reminder)
        .template(NotifyChannel::Email, "en", Some(&subject), "{{ digest }}")
}

/// The failed and stopped runs from `from` to `until` (the end not
/// included), oldest first.
pub async fn gather(state: &AppState, from: DateTime, until: DateTime) -> Result<Vec<FailedLine>> {
    let db = &state.db;
    let mut lines = Vec::new();
    for failure in RuleRunFailure::between(db, from, until).await? {
        let subject = failure.subject();
        let mut tx = db.begin().await?;
        let info = info_of(&mut tx, &subject).await?;
        drop(tx);
        let run = RuleRun::find(db, failure.run_id).await?;
        let alert =
            ComplianceAlert::of_occurrence(db, &failure.rule, &subject, &failure.occurrence_key)
                .await?;
        lines.push(FailedLine {
            run_id: failure.run_id,
            rule: rule_title(&failure.rule),
            record: record_name(&subject, info.number.as_deref()),
            occurrence: failure.occurrence_key.clone(),
            kind: failure.kind,
            error: failure.error.clone(),
            at: failure.at,
            critical: compliance::is_critical(&failure.rule) || alert.is_some(),
            alert_sent: alert.is_some_and(|a| a.notified_at.is_some()),
            rerun_succeeded: run.is_some_and(|r| r.status == RuleRunStatus::Succeeded),
            link: state.absolute_url(SHOW_ROUTE, &[&failure.run_id])?,
        });
    }
    Ok(lines)
}

/// One line of the digest, with the link under it.
pub fn line(item: &FailedLine) -> String {
    let reason = match item.kind {
        FailureKind::Failed => en("wl_kernel.failed_runs.failed"),
        FailureKind::Stopped => en("wl_kernel.failed_runs.stopped"),
    };
    let mut text = en_with(
        "wl_kernel.failed_runs.line",
        &[
            ("rule", &item.rule),
            ("record", &item.record),
            ("occurrence", &item.occurrence),
            ("reason", &reason),
            ("at", &dubai_text(item.at)),
            ("error", &item.error.clone().unwrap_or_else(|| "—".to_owned())),
        ],
    );
    if item.critical {
        text.push_str(&en(if item.alert_sent {
            "wl_kernel.failed_runs.critical_alerted"
        } else {
            "wl_kernel.failed_runs.critical"
        }));
    }
    if item.rerun_succeeded {
        text.push_str(&en("wl_kernel.failed_runs.rerun_succeeded"));
    }
    text.push('\n');
    text.push_str(&en_with(
        "wl_kernel.failed_runs.link",
        &[("url", &item.link)],
    ));
    text
}

/// The text for the period `from` to `until`: the heading, then a line per
/// run.
pub fn render(from: DateTime, until: DateTime, lines: &[FailedLine]) -> String {
    let mut out = en_with(
        "wl_kernel.failed_runs.heading",
        &[("from", &dubai_text(from)), ("until", &dubai_text(until))],
    );
    for item in lines {
        out.push('\n');
        out.push_str(&line(item));
    }
    out
}

/// Sends the digest of `date` (07:00 the day before to 07:00 on `date`,
/// Asia/Dubai) to each company admin; nothing when no run failed.
pub async fn run_failed_digest(state: &AppState, date: NaiveDate) -> Result<Report> {
    let (from, until) = period_for(date);
    let lines = gather(state, from, until).await?;
    let mut report = Report {
        runs: lines.len(),
        ..Report::default()
    };
    if lines.is_empty() {
        return Ok(report);
    }
    let text = render(from, until, &lines);
    let admins: Vec<i64> = users_with_role(&state.db, COMPANY_ADMIN_ROLE)
        .await?
        .into_iter()
        .map(|user| user.id)
        .collect();
    for user_id in admins {
        let spec = RuleRunSpec::new(RULE, Subject::new(SUBJECT_TYPE, user_id), date.to_string());
        let (text, runs) = (text.clone(), lines.len());
        let outcome = run_rule(state, spec, move |state: AppState| async move {
            send(&state, user_id, &text, runs).await
        })
        .await?;
        match outcome {
            RunOutcome::Succeeded => report.sent += 1,
            RunOutcome::Skipped => report.skipped += 1,
            _ => report.failed += 1,
        }
    }
    Ok(report)
}

/// Sends one admin the e-mail and the in-app notice.
async fn send(state: &AppState, user_id: i64, text: &str, runs: usize) -> Result {
    let label = en_with("wl_kernel.failed_runs.label", &[("runs", &runs)]);
    let request = NotifyRequest::new(NOTIFICATION_TYPE, Subject::new(SUBJECT_TYPE, user_id))
        .subject_label(&label)
        .to_user(user_id)
        .data(json!({ "digest": text, "runs": runs }));
    notify::notifier(state).notify(state, request).await?;
    if let Some(user) = User::find(&state.db, user_id).await? {
        let notice = DigestNotice {
            title: label,
            body: text.to_owned(),
        };
        state.notify(&user, &notice).await?;
    }
    Ok(())
}

/// The in-app notice (the bell) that goes with the e-mail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestNotice {
    /// `2 rule runs failed or were stopped`.
    pub title: String,
    /// The digest's text.
    pub body: String,
}

impl Notification for DigestNotice {
    fn kind(&self) -> &'static str {
        NOTICE_KIND
    }

    fn channels(&self, _to: &Recipient) -> Vec<Channel> {
        vec![Channel::Database]
    }

    fn to_database(&self, _to: &Recipient, _state: &AppState) -> Result<renox::serde_json::Value> {
        Ok(DatabaseMessage::warning(self.title.clone())
            .body(self.body.clone())
            .into())
    }
}

/// The `--date YYYY-MM-DD` of the command, else `default`.
fn date_arg(value: Option<&str>, default: NaiveDate) -> Result<NaiveDate> {
    match value {
        None => Ok(default),
        Some(text) => text.parse().map_err(|_| {
            Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("--date must be YYYY-MM-DD, got `{text}`"),
            )
        }),
    }
}

/// Registers the notification type, the daily [`TASK`] and the command of
/// the same name.
pub(crate) fn schedule(app: &mut Registry) {
    notify::register_type(notification_type());
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run_failed_digest(&state, today()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Send each company admin the rule runs that failed or were stopped from 07:00 the day \
         before to 07:00 on the date (--date YYYY-MM-DD runs another day)",
        |args, state: AppState| async move {
            let on = date_arg(args.value("--date"), today())?;
            let report = run_failed_digest(&state, on).await?;
            println!(
                "{} failed runs; {} digests sent, {} sent before, {} failed.",
                report.runs, report.sent, report.skipped, report.failed
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime {
        text.parse().expect("an instant")
    }

    fn reminder() -> FailedLine {
        FailedLine {
            run_id: 7,
            rule: "invoice reminder".into(),
            record: "invoice INV-ACME-2026-0310".into(),
            occurrence: "overdue+7".into(),
            kind: FailureKind::Failed,
            error: Some("mail server down".into()),
            at: at("2026-11-04T12:20:00Z"),
            critical: false,
            alert_sent: false,
            rerun_succeeded: false,
            link: "http://localhost/rule-runs/7".into(),
        }
    }

    #[test]
    fn a_line_names_the_rule_record_reason_error_and_time_and_links_the_run() {
        assert_eq!(
            line(&reminder()),
            "- invoice reminder: invoice INV-ACME-2026-0310, occurrence overdue+7; failed at \
             2026-11-04 16:20; last error: mail server down\n  Open the run: \
             http://localhost/rule-runs/7"
        );
    }

    #[test]
    fn a_critical_and_a_rerun_run_are_marked() {
        let mut item = reminder();
        item.critical = true;
        item.alert_sent = true;
        item.rerun_succeeded = true;
        let text = line(&item);
        assert!(text.contains("(compliance-critical, alert sent)"), "{text}");
        assert!(text.contains("(rerun, succeeded)"), "{text}");
        item.kind = FailureKind::Stopped;
        assert!(line(&item).contains("; stopped at "));
    }

    #[test]
    fn the_task_runs_at_seven_in_dubai() {
        assert_eq!((TASK_AT, TASK_ZONE, RULE), ("07:00", "Asia/Dubai", "PLT.RUL.03"));
    }
}
