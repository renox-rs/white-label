//! The late-entry event (wl-kernel §6), emitted after the commit.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// A rule run that fired between the business time and the entry time of a
/// late entry, on the late record or on the record named by its
/// parent-subject field: listed as "already fired, review". The kernel does
/// not undo it; the module that owns the affected record corrects its own
/// state by its own transitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiredRun {
    /// The rule run's id.
    pub run_id: i64,
    /// The rule id, e.g. `BKG.RUL.02`.
    pub rule: String,
    /// The record the run was about.
    pub subject: Subject,
    /// When the run was triggered.
    pub at: DateTime,
}

/// A record was entered with the late-entry flag (and, beyond N hours or in
/// a closed period, approved).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LateEntryRecorded {
    /// The record that was entered.
    pub subject: Subject,
    /// The late entry's id (`wl_late_entries`).
    pub late_entry: i64,
    /// The business time: when the work really happened.
    pub occurred_at: DateTime,
    /// The entry time: when it was typed in.
    pub entered_at: DateTime,
    /// Why it is late.
    pub reason: String,
    /// The outage reference, if the user gave one.
    pub outage_reference: Option<String>,
    /// The user who entered it.
    pub user_id: i64,
    /// The approval request, when the entry needed approval.
    pub approval_request_id: Option<i64>,
    /// The rule runs already fired, to review.
    pub fired_runs: Vec<FiredRun>,
}

impl Event for LateEntryRecorded {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_event_carries_the_runs_to_review() {
        let at: DateTime = "2026-11-03T05:15:00Z".parse().unwrap();
        let event = LateEntryRecorded {
            subject: Subject::new("bookings.check_in", 4),
            late_entry: 1,
            occurred_at: "2026-11-03T05:05:00Z".parse().unwrap(),
            entered_at: "2026-11-03T06:30:00Z".parse().unwrap(),
            reason: "router down".into(),
            outage_reference: None,
            user_id: 7,
            approval_request_id: None,
            fired_runs: vec![FiredRun {
                run_id: 9,
                rule: "BKG.RUL.02".into(),
                subject: Subject::new("bookings.booking", 2),
                at,
            }],
        };
        let json = renox::serde_json::to_value(&event).unwrap();
        assert_eq!(json["fired_runs"][0]["rule"], "BKG.RUL.02");
        assert_eq!(json["subject"]["subject_type"], "bookings.check_in");
        let back: LateEntryRecorded = renox::serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }
}
