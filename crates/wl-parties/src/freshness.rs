//! The hourly sanctions list check (wl-parties §3, `PTY.RUL.02`, issue #40).
//!
//! [`run`] looks at every list once per hour, as a compliance-critical
//! `PTY.RUL.02` rule run keyed by list and hour (a second run in the same
//! hour does nothing). One run of a list:
//!
//! 1. Downloads it through the app's [`ListFeed`](crate::feed::ListFeed) when
//!    it is stale, its last attempt failed, it was never tried, or its last
//!    attempt is [`RETRY_HOURS`] or more old. A failed download is recorded
//!    (`last_error`, attempt time), not raised as a rule failure.
//! 2. A new version sets version, update time and source, clears `last_error`
//!    and `stale_since`, and emits [`SanctionsListUpdated`]. The version
//!    already held only refreshes the update time: no event.
//! 3. Evaluates freshness with [`is_stale`](crate::sanctions_lists::is_stale).
//!    A list that turns stale gets `stale_since` and one
//!    [`SanctionsListStale`]; further failures in the same stale period
//!    change `last_error` only. A fresh list is `current`.
//!
//! With screening off the run does nothing and creates no rule run.

use std::sync::{Arc, Mutex};

use renox::chrono::{Duration, DurationRound};
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::{RuleRunSpec, RunOutcome, Subject, run_rule};

use crate::events::{SanctionsListStale, SanctionsListUpdated};
use crate::feed;
use crate::sanctions_lists::{ListStatus, SanctionsList, UpdateSource};
use crate::screening::ScreeningSettings;

/// The rule id.
pub const RULE: &str = "PTY.RUL.02";

/// The hourly scheduled task, and the command, that run [`run`].
pub const TASK: &str = "wl-parties:sanctions-lists";

/// When [`TASK`] runs (every hour on the hour).
pub const TASK_CRON: &str = "0 * * * *";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// Hours after which a list is downloaded again even if it looks current.
pub const RETRY_HOURS: i64 = 12;

/// The subject type of a list in a rule run.
pub const SUBJECT_TYPE: &str = "parties.sanctions_list";

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Lists looked at.
    pub checked: usize,
    /// Lists whose rule run was skipped (already run this hour).
    pub skipped: usize,
    /// Downloads that failed.
    pub failed_downloads: usize,
    /// `SanctionsListUpdated` events emitted.
    pub updated: usize,
    /// `SanctionsListStale` events emitted.
    pub stale: usize,
    /// Rule runs that did not succeed.
    pub failed: usize,
}

/// The occurrence key at `now`: the hour, UTC.
pub fn occurrence_key(now: DateTime) -> String {
    let hour = now.duration_trunc(Duration::hours(1)).unwrap_or(now);
    hour.format("%Y-%m-%dT%H:00Z").to_string()
}

/// Whether `list` is downloaded at `now`.
pub fn should_download(list: &SanctionsList, now: DateTime, limit_hours: i64) -> bool {
    list.status == ListStatus::Stale
        || list.last_error.is_some()
        || list.is_stale_at(now, limit_hours)
        || match list.last_attempt_at {
            None => true,
            Some(at) => now - at >= Duration::hours(RETRY_HOURS),
        }
}

/// What one list's check did.
#[derive(Debug, Clone, Copy, Default)]
struct ListReport {
    failed_download: bool,
    updated: bool,
    stale: bool,
}

/// Checks every list now.
pub async fn run(state: &AppState) -> Result<Report> {
    let mut report = Report::default();
    let settings = ScreeningSettings::of(state);
    if !settings.on {
        return Ok(report);
    }
    let now = renox::db::now();
    for list in SanctionsList::all(&state.db).await? {
        report.checked += 1;
        let spec = RuleRunSpec::new(
            RULE,
            Subject::new(SUBJECT_TYPE, list.id),
            occurrence_key(now),
        )
        .compliance_critical();
        let seen = Arc::new(Mutex::new(ListReport::default()));
        let (id, hours, out) = (list.id, settings.freshness_hours, Arc::clone(&seen));
        let outcome = run_rule(state, spec, move |state: AppState| async move {
            let done = check(&state, id, now, hours).await?;
            *out.lock().expect("report lock") = done;
            Ok(())
        })
        .await?;
        let done = *seen.lock().expect("report lock");
        match outcome {
            RunOutcome::Succeeded => {}
            RunOutcome::Skipped => report.skipped += 1,
            _ => report.failed += 1,
        }
        report.failed_downloads += usize::from(done.failed_download);
        report.updated += usize::from(done.updated);
        report.stale += usize::from(done.stale);
    }
    Ok(report)
}

/// The action of the rule run: downloads when due, then evaluates freshness.
async fn check(state: &AppState, id: i64, now: DateTime, hours: i64) -> Result<ListReport> {
    let mut done = ListReport::default();
    let Some(mut list) = SanctionsList::find(&state.db, id).await? else {
        return Ok(done);
    };
    let mut new_version = None;
    if should_download(&list, now, hours) {
        list.last_attempt_at = Some(now);
        match feed::feed(state).fetch(&list).await {
            Ok(got) => {
                if list.version.as_deref() != Some(got.version.as_str()) {
                    list.version = Some(got.version.clone());
                    new_version = Some(got.version);
                }
                list.last_updated_at = Some(now);
                list.update_source = Some(UpdateSource::Download);
                // A download replaces a manual upload: no uploader, no URL.
                list.updated_by = None;
                list.source_url = None;
                list.last_error = None;
            }
            Err(error) => {
                list.last_error = Some(error.to_string());
                done.failed_download = true;
            }
        }
    }
    let mut stale_event = None;
    if list.is_stale_at(now, hours) {
        list.status = ListStatus::Stale;
        // A list that is stale without `stale_since` (never checked, or just
        // seeded) starts its stale period now.
        if list.stale_since.is_none() {
            list.stale_since = Some(now);
            stale_event = Some(SanctionsListStale {
                list: list.id,
                name: list.name.clone(),
                last_updated_at: list.last_updated_at,
                last_error: list.last_error.clone(),
            });
        }
    } else {
        list.status = ListStatus::Current;
        list.stale_since = None;
    }
    list.updated_at = Some(now);
    list.save(&state.db).await?;
    if let Some(version) = new_version {
        done.updated = true;
        state
            .emit(SanctionsListUpdated {
                list: list.id,
                name: list.name.clone(),
                version,
            })
            .await?;
    }
    if let Some(event) = stale_event {
        done.stale = true;
        state.emit(event).await?;
    }
    Ok(done)
}

/// Registers the hourly [`TASK`] and the command of the same name.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .cron(TASK_CRON, TASK, |state: AppState| async move {
            run(&state).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Check the sanctions lists: download the stale ones and report the lists that turn stale",
        |_args, state: AppState| async move {
            let report = run(&state).await?;
            println!(
                "{} lists checked, {} updated, {} turned stale, {} downloads failed, {} skipped, {} failed.",
                report.checked,
                report.updated,
                report.stale,
                report.failed_downloads,
                report.skipped,
                report.failed
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hours: i64) -> DateTime {
        renox::chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap() + Duration::hours(hours)
    }

    fn current(updated: i64, attempt: i64) -> SanctionsList {
        SanctionsList {
            status: ListStatus::Current,
            last_updated_at: Some(at(updated)),
            last_attempt_at: Some(at(attempt)),
            ..Default::default()
        }
    }

    #[test]
    fn a_current_list_is_downloaded_after_the_retry_hours() {
        assert!(!should_download(&current(0, 0), at(11), 24));
        assert!(should_download(&current(0, 0), at(12), 24));
    }

    #[test]
    fn a_stale_failed_or_untried_list_is_downloaded() {
        let stale = SanctionsList {
            status: ListStatus::Stale,
            ..current(0, 0)
        };
        assert!(should_download(&stale, at(1), 24));
        let failed = SanctionsList {
            last_error: Some("x".into()),
            ..current(0, 0)
        };
        assert!(should_download(&failed, at(1), 24));
        let untried = SanctionsList {
            last_attempt_at: None,
            ..current(0, 0)
        };
        assert!(should_download(&untried, at(1), 24));
        assert!(should_download(&current(0, 11), at(24), 24));
    }

    #[test]
    fn the_key_is_the_hour() {
        let base = at(0);
        assert_eq!(
            occurrence_key(base),
            occurrence_key(base + Duration::minutes(1))
        );
        assert_ne!(occurrence_key(base), occurrence_key(at(1)));
    }
}
