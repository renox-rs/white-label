//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::fmt::Display;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    // Refusals of `record` and the self check-in.
    (
        "wl_attendance.record.not_working",
        "This employee is not at work (not active, on leave or gone): attendance cannot be recorded.",
    ),
    (
        "wl_attendance.record.too_old",
        "Attendance can be marked for today or up to :days day(s) back. :date is outside that window.",
    ),
    (
        "wl_attendance.record.in_future",
        "Attendance cannot be marked for a day that has not come: :date.",
    ),
    (
        "wl_attendance.record.times",
        "The check-out must be after the check-in.",
    ),
    (
        "wl_attendance.record.locked",
        "A locked or rejected record does not change.",
    ),
    (
        "wl_attendance.record.checked_out",
        "This record has a check-out already.",
    ),
    (
        "wl_attendance.self.open_elsewhere",
        "You are still checked in at :location. Check out there first.",
    ),
    (
        "wl_attendance.self.already_here",
        "You are already checked in here.",
    ),
    ("wl_attendance.self.no_employee", "No employee record belongs to your account."),
    ("wl_attendance.record.saved", "Attendance saved."),
    ("wl_attendance.record.bad_time", "Give the time as 2026-11-02T07:00."),
    // Flags.
    ("wl_attendance.flags.outside_geofence", "Outside the geofence"),
    ("wl_attendance.flags.midday_break", "Midday break"),
    ("wl_attendance.flags.missing_checkout", "Missing check-out"),
    ("wl_attendance.flags.late", "Late"),
    ("wl_attendance.flags.late_entry", "Late entry"),
    ("wl_attendance.flags.manual_correction", "Manual correction"),
    ("wl_attendance.flags.duplicate", "Duplicate"),
    ("wl_attendance.flags.overtime_cap", "Hours above the overtime cap"),
    ("wl_attendance.flags.permit_invalid", "No valid permit, visa or qualification"),
    ("wl_attendance.flags.overlap", "Overlaps another record"),
    // Notes of the flags.
    (
        "wl_attendance.overlap.note",
        "Overlaps record :record at :location (:from to :to).",
    ),
    (
        "wl_attendance.overlap.note_locked",
        "Overlaps the locked record :record at :location (:from to :to).",
    ),
    // The overtime cap (`HRM.RUL.13`).
    (
        "wl_attendance.overtime.note_day",
        "The day has :total h of work against :normal h of normal hours plus :cap h of overtime allowed.",
    ),
    (
        "wl_attendance.overtime.note_window",
        "The overtime of the :days days up to :date is :overtime h against :cap h allowed.",
    ),
    // The midday break (`HRM.RUL.13`).
    (
        "wl_attendance.midday.note",
        "Worked :minutes minutes inside the midday break, :from to :to.",
    ),
    // Work permit, visa and qualifications (`HRM.RUL.14`).
    (
        "wl_attendance.permit.note",
        "Not valid on :date: :kinds.",
    ),
    (
        "wl_attendance.submit.flagged_record",
        "The attendance record :record of :date at :location is flagged (:flags) and must be reviewed first.",
    ),
    // The review (`HRM.ACL.09`).
    (
        "wl_attendance.review.not_the_marker",
        "HRM.ACL.09: the person who marked this record, or the employee it is about, cannot accept or correct it while a flag holds it. Another reviewer must.",
    ),
    (
        "wl_attendance.review.reason_required",
        "A flag holds this record for review: give a reason.",
    ),
    (
        "wl_attendance.review.overlap_remains",
        "The hours still overlap. :note Correct or reject one of the records first.",
    ),
    (
        "wl_attendance.review.system_flagged",
        "A record with a flag is not accepted by the system; a reviewer must accept it.",
    ),
    (
        "wl_attendance.review.no_correction",
        "Give the corrected times.",
    ),
    (
        "wl_attendance.review.corrected_note",
        "Corrected by hand: :reason",
    ),
    ("wl_attendance.review.saved", "Saved."),
    // Notifications.
    (
        "wl_attendance.notices.flagged_subject",
        "Attendance flagged: {{ flag_text }}",
    ),
    (
        "wl_attendance.notices.flagged_body",
        "The attendance of {{ employee }} at {{ location }} on {{ date }} (from {{ from }}) was flagged: {{ flag_text }}. {{ note }}",
    ),
    (
        "wl_attendance.notices.over_cap_subject",
        "Hours above the overtime cap: {{ employee }}",
    ),
    (
        "wl_attendance.notices.over_cap_body",
        "{{ employee }} is over the overtime cap on {{ date }}, and every record of the day is locked, so none could be flagged. {{ note }}",
    ),
];

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key, &[])
}

/// The English text for `key` with `params` filled in; the key itself when
/// it has no text.
pub(crate) fn english(key: &str, params: &[(&str, &dyn Display)]) -> String {
    let text = TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| *text);
    let params: Vec<(&str, String)> = params.iter().map(|(k, v)| (*k, v.to_string())).collect();
    renox::i18n::format(text, &params, None)
}

#[cfg(test)]
mod tests {
    use super::{TEXTS, english};

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("wl_attendance.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_placeholder_is_filled_in() {
        assert_eq!(
            english("wl_attendance.self.open_elsewhere", &[("location", &"Site 1")]),
            "You are still checked in at Site 1. Check out there first."
        );
    }
}
