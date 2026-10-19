//! The daily digest of voids and late entries (wl-kernel §5.13 step 6,
//! `PLT.RUL.06`): every morning each manager gets one message listing the
//! records voided and the late entries made in their scope during the
//! previous day, with the user and the reason, and under each late entry the
//! rule runs it lists as "already fired, review", so that misuse of void and
//! late entry is visible and the automations that fired get checked.
//!
//! A manager here is a user who holds `late_entries.approve` in the scope of
//! the records. The period is 07:00 the previous day to 07:00 today
//! (Asia/Dubai) by the Void record time and the entry time ([`period_for`]).
//! This part is the contract: what a module registers so the digest can name
//! a record and respect its visibility ([`register_info`]), the titles of the
//! rules ([`register_rule_title`]), and the text itself ([`render`]).

pub mod collect;
pub mod info;
pub mod render;

use renox::chrono::{Duration, NaiveDate, NaiveTime};
use renox::db::DateTime;

use crate::calendar::BusinessCalendar;

pub use collect::{by_manager, gather, managers};
pub use info::{RecordInfo, info_of, register_info, register_rule_title, rule_title};
pub use render::{FiredLine, Item, ItemKind, Section, record_name, render};

/// The rule id of the digest (`PLT.RUL.06`).
pub const RULE: &str = "PLT.RUL.06";

/// The scheduled task, and the command, that send the digest.
pub const TASK: &str = "digest:voids-late";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "07:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The period the digest of `date` covers: from 07:00 Asia/Dubai the day
/// before to 07:00 on `date` (the end not included).
pub fn period_for(date: NaiveDate) -> (DateTime, DateTime) {
    let calendar = BusinessCalendar::default();
    let seven = NaiveTime::from_hms_opt(7, 0, 0).expect("07:00");
    let until = calendar.from_local(date.and_time(seven));
    (until - Duration::days(1), until)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digest_of_a_date_covers_07_00_to_07_00_in_dubai() {
        let date: NaiveDate = "2026-11-04".parse().unwrap();
        let (from, until) = period_for(date);
        // 07:00 Dubai is 03:00 UTC.
        assert_eq!(from, "2026-11-03T03:00:00Z".parse::<DateTime>().unwrap());
        assert_eq!(until, "2026-11-04T03:00:00Z".parse::<DateTime>().unwrap());
    }

    #[test]
    fn the_task_runs_at_seven_in_dubai() {
        assert_eq!(TASK_AT, "07:00");
        assert_eq!(TASK_ZONE, "Asia/Dubai");
        assert_eq!(RULE, "PLT.RUL.06");
    }
}
