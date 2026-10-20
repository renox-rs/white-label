//! Sending the digest (wl-kernel §5.13 step 6, `PLT.RUL.06`).
//!
//! [`run_digest`] sends each manager one message for a date: the sections of
//! every scope they manage that had a void or a late entry in the period
//! ([`period_for`]). A manager with nothing in the period gets nothing; an
//! empty digest is not sent. Each manager's digest is a rule run with the
//! date as its occurrence key, so a second run for the same date sends
//! nothing new, and one manager's failure does not stop the others.
//!
//! The scheduled task [`TASK`](super::TASK) runs it daily at 07:00
//! Asia/Dubai for today's date; the command of the same name takes
//! `--date YYYY-MM-DD`.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;

use super::collect::{by_manager, gather};
use super::render::{Section, render};
use super::{RULE, TASK, TASK_AT, TASK_ZONE, period_for};
use crate::automation::{RuleRunSpec, RunOutcome, run_rule};
use crate::documents::today;
use crate::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use crate::subject::Subject;
use crate::texts::{en, en_with};

/// The notification type of the digest.
pub const NOTIFICATION_TYPE: &str = "kernel.voids_late_digest";

/// The subject type the digest of one manager is a rule run about.
pub const SUBJECT_TYPE: &str = "kernel.digest";

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Managers who were sent their digest.
    pub sent: usize,
    /// Managers whose digest for this date was sent before.
    pub skipped: usize,
    /// Managers whose digest failed (it is retried by the next run).
    pub failed: usize,
}

/// The notification type `kernel.voids_late_digest`.
pub fn notification_type() -> NotificationTypeDef {
    let subject = en("wl_kernel.digest.subject");
    NotificationTypeDef::new(NOTIFICATION_TYPE, "kernel")
        .group("kernel")
        .category(Category::Reminder)
        .template(NotifyChannel::Email, "en", Some(&subject), "{{ digest }}")
}

/// Sends the digest of `date` (the period of [`period_for`]) to each manager
/// who has something in it.
pub async fn run_digest(state: &AppState, date: NaiveDate) -> Result<Report> {
    let (from, until) = period_for(date);
    let groups = gather(&state.db, from, until).await?;
    let told = by_manager(&state.db, groups).await?;
    let mut report = Report::default();
    for (user_id, sections) in told {
        let spec = RuleRunSpec::new(RULE, Subject::new(SUBJECT_TYPE, user_id), date.to_string());
        let outcome = run_rule(state, spec, move |state: AppState| async move {
            send(&state, user_id, from, until, &sections).await
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

/// Sends one manager's message.
async fn send(
    state: &AppState,
    user_id: i64,
    from: DateTime,
    until: DateTime,
    sections: &[Section],
) -> Result {
    let records: usize = sections.iter().map(|s| s.items.len()).sum();
    let request = NotifyRequest::new(NOTIFICATION_TYPE, Subject::new(SUBJECT_TYPE, user_id))
        .subject_label(&en_with("wl_kernel.digest.label", &[("records", &records)]))
        .to_user(user_id)
        .data(json!({
            "digest": render(from, until, sections),
            "records": records,
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
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
            run_digest(&state, today()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Send each manager the voids and late entries of their scope from 07:00 the day before \
         to 07:00 on the date (--date YYYY-MM-DD runs another day)",
        |args, state: AppState| async move {
            let on = date_arg(args.value("--date"), today())?;
            let report = run_digest(&state, on).await?;
            println!(
                "{} digests sent, {} sent before, {} failed.",
                report.sent, report.skipped, report.failed
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_date_argument_defaults_to_today_and_must_be_a_date() {
        let default: NaiveDate = "2026-11-04".parse().unwrap();
        assert_eq!(date_arg(None, default).unwrap(), default);
        assert_eq!(
            date_arg(Some("2026-11-05"), default).unwrap(),
            "2026-11-05".parse::<NaiveDate>().unwrap()
        );
        assert!(date_arg(Some("tomorrow"), default).is_err());
    }
}
