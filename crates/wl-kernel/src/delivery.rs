//! The delivery contract (wl-delivery §6): the events a delivery module
//! emits when a delivery is disputed, reviewed, reversed or lost.
//!
//! `wl-delivery` is not built yet; `wl-invoicing` listens to these events
//! through the kernel, so neither crate depends on the other. Each event
//! carries the delivery's `lines`, the source line references
//! ([`Subject`]) the delivery was requested for, so a listener can find the
//! records that bill them (an invoice line's `origin`).

use std::fmt;

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;
use crate::tickets::TicketId;

/// The subject type of a delivery, for [`Subject`] references.
pub const DELIVERY_SUBJECT_TYPE: &str = "delivery.delivery";

/// The id of a delivery.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct DeliveryId(pub i64);

impl DeliveryId {
    /// The delivery as a [`Subject`]: `delivery.delivery:42`.
    pub fn subject(self) -> Subject {
        Subject::new(DELIVERY_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for DeliveryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for DeliveryId {
    fn from(id: i64) -> Self {
        DeliveryId(id)
    }
}

/// How a delivery review ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewOutcome {
    /// The delivery stands: it was made as agreed.
    #[default]
    Upheld,
}

/// A delivery was disputed and went under review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryDisputed {
    /// The delivery.
    pub delivery: DeliveryId,
    /// The record the delivery was requested for.
    pub source: Subject,
    /// Why it is disputed.
    pub reason: String,
    /// The review ticket; `None` when ticketing is off.
    pub ticket: Option<TicketId>,
    /// The source line references of the delivery's lines.
    pub lines: Vec<Subject>,
}

impl Event for DeliveryDisputed {}

/// A delivery review was closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryDisputeClosed {
    /// The delivery.
    pub delivery: DeliveryId,
    /// The record the delivery was requested for.
    pub source: Subject,
    /// How the review ended.
    pub outcome: ReviewOutcome,
    /// The review ticket; `None` when ticketing is off.
    pub ticket: Option<TicketId>,
    /// The source line references of the delivery's lines.
    pub lines: Vec<Subject>,
}

impl Event for DeliveryDisputeClosed {}

/// A delivery was reversed (taken back).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryReversed {
    /// The delivery.
    pub delivery: DeliveryId,
    /// The record the delivery was requested for.
    pub source: Subject,
    /// Why it was reversed.
    pub reason: String,
    /// The source line references of the delivery's lines.
    pub lines: Vec<Subject>,
}

impl Event for DeliveryReversed {}

/// A delivery was lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryLost {
    /// The delivery.
    pub delivery: DeliveryId,
    /// The record the delivery was requested for.
    pub source: Subject,
    /// Whether the delivery was under review when it was lost.
    pub was_under_review: bool,
    /// The review ticket, when it was under review and ticketing is on.
    pub ticket: Option<TicketId>,
    /// The source line references of the delivery's lines.
    pub lines: Vec<Subject>,
}

impl Event for DeliveryLost {}

#[cfg(test)]
mod tests {
    use renox::serde_json::{json, to_value};

    use super::*;

    #[test]
    fn a_delivery_id_is_a_subject_and_prints_bare() {
        let id = DeliveryId::from(42);
        assert_eq!(id.subject(), Subject::new("delivery.delivery", 42));
        assert_eq!(id.to_string(), "42");
        assert_eq!(to_value(id).unwrap(), json!(42));
    }

    #[test]
    fn the_review_outcome_defaults_to_upheld_and_serializes_snake_case() {
        assert_eq!(ReviewOutcome::default(), ReviewOutcome::Upheld);
        assert_eq!(to_value(ReviewOutcome::Upheld).unwrap(), json!("upheld"));
    }

    #[test]
    fn events_carry_the_delivery_ticket_and_lines() {
        let line = Subject::new("leasing.lease", 5);
        let lost = DeliveryLost {
            delivery: DeliveryId(7),
            source: line.clone(),
            was_under_review: true,
            ticket: Some(TicketId(11)),
            lines: vec![line.clone()],
        };
        let value = to_value(&lost).unwrap();
        assert_eq!(value["delivery"], json!(7));
        assert_eq!(value["ticket"], json!(11));
        assert_eq!(value["was_under_review"], json!(true));
        assert_eq!(value["lines"].as_array().unwrap().len(), 1);
    }
}
