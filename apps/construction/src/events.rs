//! The events the vertical emits (02 Construction §7.1).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// A project was terminated mid-way (`terminate`, `CON.ACL.15`): the effective
/// date is its last working day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectTerminated {
    /// The project, `construction.project:4`.
    pub project: Subject,
    /// Why: `terminated_by_client`, `our_default`, `main_contractor_insolvent`
    /// or `agreed`.
    pub reason: String,
    /// The last working day.
    pub effective_on: NaiveDate,
    /// The user who asked for the termination.
    pub requested_by: Option<i64>,
}

impl Event for ProjectTerminated {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_event_round_trips_through_json() {
        let event = ProjectTerminated {
            project: Subject::new("construction.project", 4),
            reason: "agreed".into(),
            effective_on: NaiveDate::from_ymd_opt(2027, 5, 31).unwrap(),
            requested_by: Some(7),
        };
        let json = renox::serde_json::to_string(&event).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<ProjectTerminated>(&json).unwrap(),
            event
        );
    }
}
