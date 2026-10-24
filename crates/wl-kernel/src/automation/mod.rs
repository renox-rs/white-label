//! Automation rules (wl-kernel §4.2, §5.3, rule runs): the [`RuleRun`] record
//! and its machine, [`run_rule`] which runs an action once per occurrence key
//! with retries, and [`AutomationFailed`], which a module that runs a rule
//! itself (e.g. wl-notify `NTF.RUL.01`) emits after the last attempt.
//! Each failure is recorded ([`RuleRunFailure`]) for the daily digest of
//! failed runs to the company admin ([`failed_digest`], `PLT.RUL.03`), whose
//! lines open the run's page ([`web`]). The rule catalogue comes later.

pub mod failed_digest;
pub mod failures;
pub mod run;
pub mod web;

pub use failures::{FailureKind, RuleRunFailure};

pub use run::{
    MAX_ATTEMPTS, MAX_CHAIN_DEPTH, RuleRun, RuleRunSpec, RuleRunStatus, RunOutcome, backoff,
    run_rule, transitions,
};

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// Registers the rule run model and its machine, the failures, the run's
/// page and the daily digest of failed runs.
pub(crate) fn register(app: &mut Registry) {
    app.model::<RuleRun>()
        .model::<RuleRunFailure>()
        .view_data::<web::RuleRunPage>();
    renox::state_machine::register::<RuleRun>();
    failed_digest::schedule(app);
    app.templates(|env| {
        // The app's own file of that name wins.
        if env.get_template(web::TEMPLATE).is_err() {
            renox::view::add_template(
                env,
                web::TEMPLATE,
                include_str!("../../views/wl-kernel/rule_run.html"),
            )
            .expect("a built-in template compiles");
        }
    });
}

/// A rule run failed after its last attempt, or was stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationFailed {
    /// The rule id, e.g. `NTF.RUL.01`.
    pub rule: String,
    /// The record the run was about.
    pub subject: Subject,
    /// The last error.
    pub error: String,
    /// Whether the rule is compliance-critical (an admin alert either way).
    pub compliance_critical: bool,
}

impl Event for AutomationFailed {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_event_names_the_rule_and_subject() {
        let failed = AutomationFailed {
            rule: "NTF.RUL.01".into(),
            subject: Subject::new("notify.message", 4),
            error: "timeout".into(),
            compliance_critical: false,
        };
        let json = renox::serde_json::to_string(&failed).unwrap();
        assert_eq!(
            json,
            r#"{"rule":"NTF.RUL.01","subject":{"subject_type":"notify.message","subject_id":4},"error":"timeout","compliance_critical":false}"#
        );
    }
}
