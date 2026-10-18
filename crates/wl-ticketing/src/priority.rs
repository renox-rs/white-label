//! Ticket priority (wl-ticketing §3, §7).

use renox::prelude::*;

/// How urgent a ticket is; it picks the SLA targets. Stored as `urgent`,
/// `high`, `normal` or `low`; `Priority::ALL` lists them most urgent first.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Priority {
    /// Safety or loss of service: 2 h response.
    Urgent,
    /// Serious but contained.
    High,
    /// The usual job.
    #[default]
    Normal,
    /// Can wait.
    Low,
}

impl Priority {
    /// The English name.
    pub fn name(self) -> &'static str {
        match self {
            Priority::Urgent => "Urgent",
            Priority::High => "High",
            Priority::Normal => "Normal",
            Priority::Low => "Low",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_unknown_keys_are_refused() {
        for priority in Priority::ALL {
            assert_eq!(priority.as_str().parse::<Priority>().ok(), Some(*priority));
        }
        assert!("critical".parse::<Priority>().is_err());
        assert_eq!(Priority::default(), Priority::Normal);
    }

    #[test]
    fn all_runs_from_urgent_to_low() {
        let keys: Vec<_> = Priority::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(keys, ["urgent", "high", "normal", "low"]);
        assert_eq!(Priority::High.name(), "High");
    }
}
