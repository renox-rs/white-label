//! The late-entry check (wl-kernel §5.13, `PLT.ACL.10`): a pure decision from
//! the business time, the entry time, the limits and the state of the
//! accounting period.
//!
//! M and N are elapsed wall-clock time between two instants, never business
//! hours. Exactly M minutes late is not flagged and M minutes and 1 second
//! is; exactly N hours is flag-only and anything above needs approval. A
//! business time after the entry time is not a late entry at all: the
//! module's own validation applies.

use renox::chrono::Duration;
use renox::db::DateTime;

/// M: minutes after which an entry is flagged (`PLT.ACL.10`).
pub const DEFAULT_FLAG_AFTER_MINUTES: i64 = 30;

/// N: hours after which an entry needs a manager's approval (`PLT.ACL.10`).
pub const DEFAULT_APPROVAL_AFTER_HOURS: i64 = 24;

/// M and N of one entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Minutes of delay above which an entry is flagged late.
    pub flag_after_minutes: i64,
    /// Hours of delay above which an entry needs approval.
    pub approval_after_hours: i64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            flag_after_minutes: DEFAULT_FLAG_AFTER_MINUTES,
            approval_after_hours: DEFAULT_APPROVAL_AFTER_HOURS,
        }
    }
}

/// What the module says about the accounting period of a business time
/// (a locked timesheet, a locked VAT period, a closed payroll).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Period {
    /// Not in a closed period.
    #[default]
    Open,
    /// In a closed period that the module allows to reopen: the entry needs
    /// approval whatever its delay.
    Closed,
    /// In a period the module forbids to reopen: the entry is refused with
    /// this message.
    Locked(String),
}

/// Why an entry needs approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalWhy {
    /// The delay is above N hours.
    TooLate,
    /// The business time is in a closed period.
    ClosedPeriod,
}

/// What the check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assessment {
    /// Not late: saved without a flag.
    OnTime,
    /// More than M minutes late, up to N hours: the flag and a reason are
    /// enough.
    Flagged {
        /// Whole minutes between the business time and the entry time.
        minutes_late: i64,
    },
    /// Beyond N hours, or in a closed period: an approval request first.
    NeedsApproval {
        /// Whole minutes between the business time and the entry time.
        minutes_late: i64,
        /// Why.
        why: ApprovalWhy,
    },
    /// The module's period lock forbids it; the message is the module's.
    Refused(String),
}

impl Assessment {
    /// Whether the record carries the late-entry flag.
    pub fn is_late(&self) -> bool {
        matches!(
            self,
            Assessment::Flagged { .. } | Assessment::NeedsApproval { .. }
        )
    }

    /// Whether a reason is required.
    pub fn needs_reason(&self) -> bool {
        self.is_late()
    }

    /// Whether the entry waits for an approval.
    pub fn needs_approval(&self) -> bool {
        matches!(self, Assessment::NeedsApproval { .. })
    }

    /// Whole minutes late; 0 when on time or refused.
    pub fn minutes_late(&self) -> i64 {
        match self {
            Assessment::Flagged { minutes_late } | Assessment::NeedsApproval { minutes_late, .. } => {
                *minutes_late
            }
            _ => 0,
        }
    }
}

/// Checks an entry whose business time is `occurred_at`, entered at
/// `entered_at`, against `limits` and the `period` of the business time.
pub fn assess(
    limits: Limits,
    occurred_at: DateTime,
    entered_at: DateTime,
    period: &Period,
) -> Assessment {
    if let Period::Locked(message) = period {
        return Assessment::Refused(message.clone());
    }
    let elapsed = entered_at - occurred_at;
    let minutes_late = elapsed.num_minutes().max(0);
    if *period == Period::Closed {
        return Assessment::NeedsApproval {
            minutes_late,
            why: ApprovalWhy::ClosedPeriod,
        };
    }
    if elapsed > Duration::hours(limits.approval_after_hours) {
        return Assessment::NeedsApproval {
            minutes_late,
            why: ApprovalWhy::TooLate,
        };
    }
    if elapsed > Duration::minutes(limits.flag_after_minutes) {
        return Assessment::Flagged { minutes_late };
    }
    Assessment::OnTime
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime {
        text.parse().unwrap()
    }

    fn check(occurred: &str, entered: &str) -> Assessment {
        assess(
            Limits::default(),
            at(occurred),
            at(entered),
            &Period::Open,
        )
    }

    #[test]
    fn fifteen_minutes_late_is_on_time() {
        // 10:00 Asia/Dubai is 06:00 UTC.
        assert_eq!(
            check("2026-11-03T05:45:00Z", "2026-11-03T06:00:00Z"),
            Assessment::OnTime
        );
    }

    #[test]
    fn exactly_thirty_minutes_is_not_flagged_and_one_second_more_is() {
        assert_eq!(
            check("2026-11-03T05:30:00Z", "2026-11-03T06:00:00Z"),
            Assessment::OnTime
        );
        assert_eq!(
            check("2026-11-03T05:29:59Z", "2026-11-03T06:00:00Z"),
            Assessment::Flagged { minutes_late: 30 }
        );
    }

    #[test]
    fn exactly_twenty_four_hours_is_flag_only_and_above_needs_approval() {
        assert_eq!(
            check("2026-11-02T06:00:00Z", "2026-11-03T06:00:00Z"),
            Assessment::Flagged { minutes_late: 1440 }
        );
        assert!(matches!(
            check("2026-11-02T05:59:59Z", "2026-11-03T06:00:00Z"),
            Assessment::NeedsApproval {
                why: ApprovalWhy::TooLate,
                ..
            }
        ));
    }

    #[test]
    fn twenty_six_hours_needs_approval() {
        let found = check("2026-11-02T04:00:00Z", "2026-11-03T06:00:00Z");
        assert_eq!(
            found,
            Assessment::NeedsApproval {
                minutes_late: 26 * 60,
                why: ApprovalWhy::TooLate
            }
        );
        assert!(found.needs_approval() && found.is_late() && found.needs_reason());
    }

    #[test]
    fn a_business_time_in_the_future_is_not_late() {
        assert_eq!(
            check("2026-11-03T07:00:00Z", "2026-11-03T06:00:00Z"),
            Assessment::OnTime
        );
    }

    #[test]
    fn a_closed_period_needs_approval_even_two_hours_late() {
        let found = assess(
            Limits::default(),
            at("2026-11-03T04:00:00Z"),
            at("2026-11-03T06:00:00Z"),
            &Period::Closed,
        );
        assert_eq!(
            found,
            Assessment::NeedsApproval {
                minutes_late: 120,
                why: ApprovalWhy::ClosedPeriod
            }
        );
    }

    #[test]
    fn a_locked_period_is_refused_with_the_modules_message() {
        let found = assess(
            Limits::default(),
            at("2026-11-03T04:00:00Z"),
            at("2026-11-03T06:00:00Z"),
            &Period::Locked("The timesheet of October is locked.".into()),
        );
        assert_eq!(
            found,
            Assessment::Refused("The timesheet of October is locked.".into())
        );
        assert!(!found.is_late());
    }

    #[test]
    fn sixty_minutes_stops_flagging_a_forty_five_minute_delay() {
        let limits = Limits {
            flag_after_minutes: 60,
            ..Limits::default()
        };
        let found = assess(
            limits,
            at("2026-11-03T05:15:00Z"),
            at("2026-11-03T06:00:00Z"),
            &Period::Open,
        );
        assert_eq!(found, Assessment::OnTime);
        assert_eq!(found.minutes_late(), 0);
    }
}
