//! The void event (wl-kernel §6), emitted after the commit.

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// A record was voided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordVoided {
    /// The record that was voided.
    pub subject: Subject,
    /// The state it was in before.
    pub state_before: String,
    /// Why it was voided.
    pub reason: String,
    /// The user who voided it (the requester when it was approved).
    pub user_id: i64,
    /// The approval request, when money or stock was posted.
    pub approval_request_id: Option<i64>,
}

impl Event for RecordVoided {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_event_names_the_subject_and_the_request() {
        let event = RecordVoided {
            subject: Subject::new("payments.receipt", 3),
            state_before: "issued".into(),
            reason: "duplicate".into(),
            user_id: 5,
            approval_request_id: Some(9),
        };
        let json = renox::serde_json::to_value(&event).unwrap();
        assert_eq!(json["subject"]["subject_type"], "payments.receipt");
        assert_eq!(json["state_before"], "issued");
        assert_eq!(json["approval_request_id"], 9);
    }
}
