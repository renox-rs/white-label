//! The alert of a compliance-critical failure (wl-kernel §5.15, `PLT.RUL.04`):
//! one per failed, stopped or missed occurrence, open until the duty is met.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// What went wrong with the occurrence.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AlertKind {
    /// The run failed after its last attempt.
    #[default]
    Failed,
    /// The chain was too deep: nothing ran.
    Stopped,
    /// The occurrence was planned and no rule run exists.
    Missed,
}

/// Whether the owner still has to act.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AlertStatus {
    /// The duty is not met yet.
    #[default]
    Open,
    /// Closed by a successful run or by the owner's note.
    Closed,
}

/// How an alert was closed.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClosedHow {
    /// The rule ran successfully for the occurrence.
    #[default]
    Rerun,
    /// The owner recorded how the duty was met by hand.
    Manual,
}

/// One alert (`wl_compliance_alerts`), unique per rule and occurrence.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_compliance_alerts",
    unique(rule, subject_type, subject_id, occurrence_key),
    index(status)
)]
pub struct ComplianceAlert {
    /// The primary key.
    pub id: i64,
    /// The rule id.
    pub rule: String,
    /// The type of the record the run is about.
    pub subject_type: String,
    /// The record's id.
    pub subject_id: i64,
    /// The occurrence the run is for.
    pub occurrence_key: String,
    /// Failed, stopped or missed.
    pub kind: AlertKind,
    /// The error of the failed or stopped run.
    pub error: Option<String>,
    /// When a missed occurrence was planned.
    pub planned_at: Option<DateTime>,
    /// The role told with the company admin, and given the task.
    pub owner_role: String,
    /// Open or closed.
    pub status: AlertStatus,
    /// How it was closed.
    pub closed_how: Option<ClosedHow>,
    /// The user who recorded the note.
    pub closed_by: Option<i64>,
    /// How the duty was met by hand, or the successful run.
    pub note: Option<String>,
    /// A document (wl-documents) attached to the note, if any.
    pub attachment_document_id: Option<i64>,
    /// When it was closed.
    pub closed_at: Option<DateTime>,
    /// The owner's task, when ticketing is on.
    pub ticket_id: Option<i64>,
    /// The task's number.
    pub ticket_number: Option<String>,
    /// When the owner was told and the task opened. Empty while the notice
    /// is still owed: a raise that failed half way is finished by the next
    /// raise for the occurrence.
    pub notified_at: Option<DateTime>,
    /// When the alert was raised.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ComplianceAlert {
    /// The record the run is about.
    pub fn subject(&self) -> Subject {
        Subject::new(&self.subject_type, self.subject_id)
    }

    /// Whether the owner still has to act.
    pub fn is_open(&self) -> bool {
        self.status == AlertStatus::Open
    }

    /// The alert of the occurrence `key` of `rule` about `subject`, if any.
    pub async fn of_occurrence(
        db: &Db,
        rule: &str,
        subject: &Subject,
        key: &str,
    ) -> Result<Option<ComplianceAlert>> {
        ComplianceAlert::query()
            .where_(ComplianceAlert::RULE.eq(rule))
            .where_(ComplianceAlert::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(ComplianceAlert::SUBJECT_ID.eq(subject.subject_id))
            .where_(ComplianceAlert::OCCURRENCE_KEY.eq(key))
            .first(db)
            .await
    }

    /// The open alerts, oldest first.
    pub async fn open(db: &Db) -> Result<Vec<ComplianceAlert>> {
        ComplianceAlert::query()
            .where_(ComplianceAlert::STATUS.eq(AlertStatus::Open))
            .order_by(ComplianceAlert::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_alert_is_open_and_names_its_subject() {
        let alert = ComplianceAlert {
            rule: "HRM.RUL.03".into(),
            subject_type: "hr.payroll_run".into(),
            subject_id: 4,
            ..Default::default()
        };
        assert!(alert.is_open());
        assert_eq!(alert.subject(), Subject::new("hr.payroll_run", 4));
        assert_eq!(alert.kind, AlertKind::Failed);
    }

    #[test]
    fn the_kinds_and_hows_are_stored_in_snake_case() {
        let kinds: Vec<_> = AlertKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(kinds, ["failed", "stopped", "missed"]);
        let hows: Vec<_> = ClosedHow::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(hows, ["rerun", "manual"]);
    }
}
