//! The registry of compliance-critical rules (wl-kernel §5.15), keyed by
//! rule id like [`crate::voids`].
//!
//! A module marks its rules once, in `register()`:
//!
//! ```ignore
//! // Fails or stops: the owner is alerted. Nothing else to watch.
//! CriticalRule::new("HRM.RUL.03").owner_role("md").register();
//!
//! // Also watched for a missed run: nightly at 01:00 Asia/Dubai.
//! CriticalRule::new("DOC.RUL.03").daily_at("01:00").register();
//!
//! // A date-reached rule: the module lists the occurrences it planned.
//! CriticalRule::new("FIN.RUL.09")
//!     .occurrences(|state, from, until| Box::pin(async move { vat_due(state, from, until).await }))
//!     .register();
//! ```
//!
//! A module's job that runs a scheduled critical rule uses the run it was
//! planned under: the subject [`schedule_subject`] and the key
//! [`occurrence_key`] of the date, so the kernel finds that the run exists.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use renox::chrono::{Duration, NaiveDate, NaiveTime};
use renox::db::DateTime;
use renox::{AppState, Result};

use crate::calendar::BusinessCalendar;
use crate::documents::dubai_date;
use crate::machines::BoxFuture;
use crate::subject::Subject;

/// Minutes after its planned time that an occurrence with no rule run is
/// missed (wl-kernel §5.15 step 4).
pub const MISSED_AFTER_MINUTES: i64 = 60;

/// How many days back the check looks for planned occurrences.
pub const LOOK_BACK_DAYS: i64 = 3;

/// One occurrence of a scheduled or date-reached rule: when it was planned,
/// and the subject and key its rule run would carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// The record the run is about; [`schedule_subject`] for a nightly job.
    pub subject: Subject,
    /// The run's occurrence key.
    pub occurrence_key: String,
    /// When it was planned to run.
    pub planned_at: DateTime,
}

/// The module's list of the occurrences of a rule planned between `from`
/// and `until`, both included.
pub type PlannedFn = Arc<
    dyn for<'a> Fn(&'a AppState, DateTime, DateTime) -> BoxFuture<'a, Result<Vec<Planned>>>
        + Send
        + Sync,
>;

/// How the kernel finds the occurrences of a rule to watch.
#[derive(Clone)]
pub enum Schedule {
    /// Every day at `HH:MM`, Asia/Dubai.
    DailyAt(String),
    /// The module lists them.
    Listed(PlannedFn),
}

impl fmt::Debug for Schedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Schedule::DailyAt(at) => f.debug_tuple("DailyAt").field(at).finish(),
            Schedule::Listed(_) => f.write_str("Listed(..)"),
        }
    }
}

/// A rule whose failure can break a legal duty. Built with
/// [`CriticalRule::new`].
#[derive(Clone, Debug)]
#[must_use = "a rule is only critical once registered"]
pub struct CriticalRule {
    rule: String,
    owner_role: Option<String>,
    schedule: Option<Schedule>,
}

impl CriticalRule {
    /// Marks `rule` (e.g. `HRM.RUL.03`) compliance-critical. Its owner role
    /// is the deployment's business owner role until
    /// [`CriticalRule::owner_role`] says otherwise.
    pub fn new(rule: &str) -> Self {
        CriticalRule {
            rule: rule.to_owned(),
            owner_role: None,
            schedule: None,
        }
    }

    /// The vertical's owner role for this rule (09: MD).
    pub fn owner_role(mut self, role: &str) -> Self {
        self.owner_role = Some(role.to_owned());
        self
    }

    /// A rule that runs every day at `HH:MM` Asia/Dubai: its missed runs are
    /// watched.
    ///
    /// # Panics
    ///
    /// When `time` is not `HH:MM`.
    pub fn daily_at(mut self, time: &str) -> Self {
        assert!(
            NaiveTime::parse_from_str(time, "%H:%M").is_ok(),
            "{}: `{time}` is not HH:MM",
            self.rule
        );
        self.schedule = Some(Schedule::DailyAt(time.to_owned()));
        self
    }

    /// A date-reached rule whose planned occurrences the module lists: its
    /// missed runs are watched.
    pub fn occurrences<F>(mut self, planned: F) -> Self
    where
        F: for<'a> Fn(&'a AppState, DateTime, DateTime) -> BoxFuture<'a, Result<Vec<Planned>>>
            + Send
            + Sync
            + 'static,
    {
        self.schedule = Some(Schedule::Listed(Arc::new(planned)));
        self
    }

    /// The rule id.
    pub fn rule_id(&self) -> &str {
        &self.rule
    }

    /// The owner role the module named, if any.
    pub fn owner_role_name(&self) -> Option<&str> {
        self.owner_role.as_deref()
    }

    /// How its occurrences are found, if it is watched for a missed run.
    pub fn schedule(&self) -> Option<&Schedule> {
        self.schedule.as_ref()
    }

    /// The occurrences planned between `from` and `until`, oldest first;
    /// none for a rule with no schedule.
    pub async fn planned(
        &self,
        state: &AppState,
        from: DateTime,
        until: DateTime,
    ) -> Result<Vec<Planned>> {
        let mut found = match &self.schedule {
            None => Vec::new(),
            Some(Schedule::DailyAt(time)) => planned_daily(&self.rule, time, from, until),
            Some(Schedule::Listed(planned)) => planned(state, from, until).await?,
        };
        found.sort_by_key(|p| p.planned_at);
        Ok(found)
    }

    /// Adds this rule to the process-wide registry, replacing one of the
    /// same id.
    pub fn register(self) {
        register(self);
    }
}

/// The subject of the run of a nightly (not record-bound) rule.
pub fn schedule_subject() -> Subject {
    Subject::new("kernel.schedule", 0)
}

/// The occurrence key of the run of `rule` planned for the Asia/Dubai
/// `date`: `DOC.RUL.03:2026-11-03`.
pub fn occurrence_key(rule: &str, date: NaiveDate) -> String {
    format!("{rule}:{date}")
}

/// The instants `rule` planned every day at `time` (`HH:MM`, Asia/Dubai)
/// between `from` and `until`, both included, with the nightly job's subject
/// and the key of the date.
pub fn planned_daily(rule: &str, time: &str, from: DateTime, until: DateTime) -> Vec<Planned> {
    let Ok(at) = NaiveTime::parse_from_str(time, "%H:%M") else {
        return Vec::new();
    };
    let calendar = BusinessCalendar::default();
    let mut found = Vec::new();
    let mut date = dubai_date(from) - Duration::days(1);
    let last = dubai_date(until) + Duration::days(1);
    while date <= last {
        let planned_at = calendar.from_local(date.and_time(at));
        if planned_at >= from && planned_at <= until {
            found.push(Planned {
                subject: schedule_subject(),
                occurrence_key: occurrence_key(rule, date),
                planned_at,
            });
        }
        date += Duration::days(1);
    }
    found
}

/// The latest planned time that counts as missed at `now`: 60 minutes ago.
pub fn missed_before(now: DateTime) -> DateTime {
    now - Duration::minutes(MISSED_AFTER_MINUTES)
}

static REGISTRY: RwLock<BTreeMap<String, Arc<CriticalRule>>> = RwLock::new(BTreeMap::new());

/// Adds `rule` to the process-wide registry; registering the same rule id
/// again replaces the entry.
pub fn register(rule: CriticalRule) {
    REGISTRY
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(rule.rule.clone(), Arc::new(rule));
}

/// The critical rule `rule`, if registered.
pub fn rule(rule: &str) -> Option<Arc<CriticalRule>> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(rule)
        .cloned()
}

/// Whether `rule` is compliance-critical.
pub fn is_critical(rule_id: &str) -> bool {
    rule(rule_id).is_some()
}

/// The registered critical rules, by id.
pub fn all() -> Vec<Arc<CriticalRule>> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime {
        text.parse().expect("an instant")
    }

    #[test]
    fn a_nightly_rule_is_planned_every_dubai_day_at_its_time() {
        // 01:00 Dubai is 21:00 UTC the day before.
        let found = planned_daily(
            "DOC.RUL.03",
            "01:00",
            at("2026-11-02T00:00:00Z"),
            at("2026-11-04T00:00:00Z"),
        );
        let times: Vec<_> = found.iter().map(|p| p.planned_at).collect();
        assert_eq!(
            times,
            [at("2026-11-02T21:00:00Z"), at("2026-11-03T21:00:00Z")]
        );
        assert_eq!(found[0].occurrence_key, "DOC.RUL.03:2026-11-03");
        assert_eq!(found[1].occurrence_key, "DOC.RUL.03:2026-11-04");
        assert_eq!(found[0].subject, schedule_subject());
    }

    #[test]
    fn the_ends_of_the_window_are_included() {
        let found = planned_daily(
            "DOC.RUL.03",
            "01:10",
            at("2026-11-02T21:10:00Z"),
            at("2026-11-02T21:10:00Z"),
        );
        assert_eq!(found.len(), 1);
        let none = planned_daily(
            "DOC.RUL.03",
            "01:10",
            at("2026-11-02T21:10:01Z"),
            at("2026-11-03T21:09:59Z"),
        );
        assert!(none.is_empty());
    }

    #[test]
    fn a_bad_time_plans_nothing() {
        assert!(
            planned_daily(
                "DOC.RUL.03",
                "25:99",
                at("2026-11-02T00:00:00Z"),
                at("2026-11-04T00:00:00Z")
            )
            .is_empty()
        );
    }

    #[test]
    fn sixty_minutes_is_missed_and_fifty_is_not() {
        // The check at 02:00 Dubai (22:00 UTC) sees what was planned by 01:00.
        let check = at("2026-11-02T22:00:00Z");
        assert_eq!(missed_before(check), at("2026-11-02T21:00:00Z"));
        let planned_0100 = at("2026-11-02T21:00:00Z");
        let planned_0110 = at("2026-11-02T21:10:00Z");
        assert!(planned_0100 <= missed_before(check));
        assert!(planned_0110 > missed_before(check));
        // The check at 02:15 sees the one planned at 01:10 (65 minutes).
        assert!(planned_0110 <= missed_before(at("2026-11-02T22:15:00Z")));
    }

    #[test]
    fn the_key_names_the_rule_and_the_date() {
        let date: NaiveDate = "2026-11-03".parse().unwrap();
        assert_eq!(occurrence_key("DOC.RUL.03", date), "DOC.RUL.03:2026-11-03");
    }

    #[test]
    fn the_registry_keeps_what_the_module_declared() {
        CriticalRule::new("TST.RUL.01")
            .owner_role("md")
            .daily_at("01:00")
            .register();
        CriticalRule::new("TST.RUL.02").register();
        let found = rule("TST.RUL.01").expect("registered");
        assert_eq!(found.owner_role_name(), Some("md"));
        assert!(matches!(found.schedule(), Some(Schedule::DailyAt(t)) if t == "01:00"));
        assert!(is_critical("TST.RUL.02"));
        assert_eq!(rule("TST.RUL.02").unwrap().owner_role_name(), None);
        assert!(rule("TST.RUL.02").unwrap().schedule().is_none());
        assert!(!is_critical("TST.RUL.99"));
        assert!(all().iter().any(|r| r.rule_id() == "TST.RUL.01"));
    }

    #[test]
    #[should_panic(expected = "is not HH:MM")]
    fn a_bad_daily_time_panics() {
        let _ = CriticalRule::new("TST.RUL.03").daily_at("1am");
    }
}
