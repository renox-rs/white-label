//! The integration event (wl-kernel §6), emitted after the commit.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// A connection changed state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationStatusChanged {
    /// The connection's key, e.g. `asp.acme`.
    pub connection: String,
    /// The state it left: `up`, `degraded`, `down` or `disabled`.
    pub from: String,
    /// The state it entered.
    pub to: String,
    /// When the current failure run began (the first failed call); `None`
    /// when the connection is `up` or `disabled` without a failure.
    pub failed_since: Option<DateTime>,
}

impl Event for IntegrationStatusChanged {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_event_names_the_connection_and_both_states() {
        let event = IntegrationStatusChanged {
            connection: "asp.acme".into(),
            from: "degraded".into(),
            to: "down".into(),
            failed_since: Some("2026-11-03T06:00:00Z".parse().unwrap()),
        };
        let json = renox::serde_json::to_value(&event).unwrap();
        assert_eq!(json["connection"], "asp.acme");
        assert_eq!(json["from"], "degraded");
        assert_eq!(json["to"], "down");
        let back: IntegrationStatusChanged = renox::serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }
}
