//! Automation rules (wl-kernel §4, rule runs): what modules share about a
//! rule run that failed. The kernel's rule-run engine and its admin page come
//! later; until then a module that runs a rule itself (e.g. wl-notify
//! `NTF.RUL.01`) emits [`AutomationFailed`] after the last attempt.

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

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
