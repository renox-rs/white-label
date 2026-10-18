//! Automation rules (wl-kernel §4.2, §5.3, rule runs): the [`RuleRun`] record
//! and its machine, [`run_rule`] which runs an action once per occurrence key
//! with retries, and [`AutomationFailed`], which a module that runs a rule
//! itself (e.g. wl-notify `NTF.RUL.01`) emits after the last attempt. The
//! rule catalogue and the admin page come later.

pub mod run;

pub use run::{
    MAX_ATTEMPTS, MAX_CHAIN_DEPTH, RuleRun, RuleRunSpec, RuleRunStatus, RunOutcome, backoff,
    run_rule, transitions,
};

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// Registers the rule run model and its machine.
pub(crate) fn register(app: &mut Registry) {
    app.model::<RuleRun>();
    renox::state_machine::register::<RuleRun>();
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
