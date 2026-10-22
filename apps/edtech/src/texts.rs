//! The vertical's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::prelude::*;

/// Every key the app uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("edtech.button_cancel", "Cancel"),
    ("edtech.none", "—"),
    ("edtech.fields.status", "Status"),
    ("edtech.history.system", "System"),
    // The courses.
    ("edtech.courses.title", "Courses"),
    (
        "edtech.courses.subtitle",
        "What the centre teaches, with the regulator approval of each course.",
    ),
    ("edtech.courses.new", "New course"),
    (
        "edtech.courses.new_description",
        "A course belongs to a programme and has hours, a delivery mode and, where the regulator asks for one, an approval.",
    ),
    ("edtech.courses.code", "Code"),
    ("edtech.courses.name", "Name"),
    ("edtech.courses.name_en", "Name in English"),
    ("edtech.courses.name_ar", "Name in Arabic"),
    ("edtech.courses.programme", "Programme"),
    ("edtech.courses.level", "Level"),
    ("edtech.courses.hours", "Hours"),
    ("edtech.courses.delivery", "Delivery"),
    ("edtech.courses.delivery.in_person", "In person"),
    ("edtech.courses.delivery.online", "Online"),
    ("edtech.courses.delivery.hybrid", "In person and online"),
    ("edtech.courses.for_minors", "For minors"),
    ("edtech.courses.approval", "Regulator approval"),
    ("edtech.courses.approval_required", "The regulator must approve this course"),
    ("edtech.courses.approval_no", "Approval number"),
    ("edtech.courses.approval_missing", "Not recorded"),
    ("edtech.courses.approval_not_needed", "Not needed"),
    ("edtech.courses.yes", "Yes"),
    ("edtech.courses.no", "No"),
    ("edtech.courses.empty", "No courses yet"),
    ("edtech.courses.created", "Course added."),
    (
        "edtech.courses.problem.name",
        "A course needs a code and a name.",
    ),
    (
        "edtech.courses.problem.hours",
        "The hours cannot be negative.",
    ),
    (
        "edtech.courses.problem.code_taken",
        "Another course has this code.",
    ),
    (
        "edtech.courses.problem.approval_no",
        "Give the regulator's approval number.",
    ),
    // The batches.
    ("edtech.batches.title", "Batches"),
    (
        "edtech.batches.subtitle",
        "The runs of the courses, with their weekly pattern and their seats.",
    ),
    ("edtech.batches.new", "Plan a batch"),
    (
        "edtech.batches.new_description",
        "A batch runs one course on a weekly pattern between two dates. It stays planned, unseen by students, until it is opened.",
    ),
    ("edtech.batches.number", "Batch"),
    ("edtech.batches.course", "Course"),
    ("edtech.batches.capacity", "Seats"),
    ("edtech.batches.start_date", "First day"),
    ("edtech.batches.end_date", "Last day"),
    ("edtech.batches.weekdays", "Weekdays"),
    (
        "edtech.batches.weekdays_hint",
        "1 = Monday ... 7 = Sunday, for example 1,3",
    ),
    ("edtech.batches.start_time", "Start time"),
    ("edtech.batches.start_time_hint", "For example 18:00, Dubai time"),
    ("edtech.batches.duration", "Minutes per session"),
    ("edtech.batches.tutor_resource", "Tutor (resource number)"),
    ("edtech.batches.tutor_employee", "Tutor (employee number)"),
    ("edtech.batches.room_resource", "Room (resource number)"),
    ("edtech.batches.room_limit", "Persons the room holds"),
    ("edtech.batches.dates", "Dates"),
    ("edtech.batches.pattern", "Weekly pattern"),
    ("edtech.batches.seats", "Seats taken"),
    ("edtech.batches.free_seats", "Seats free"),
    ("edtech.batches.tutor", "Tutor"),
    ("edtech.batches.room", "Room"),
    ("edtech.batches.resource", "Resource"),
    ("edtech.batches.details", "Details"),
    ("edtech.batches.empty", "No batches yet"),
    ("edtech.batches.created", "Batch planned."),
    ("edtech.batches.open", "Open for enrolment"),
    (
        "edtech.batches.open_message",
        "The sessions are generated from the weekly pattern and the batch is listed in the catalogue.",
    ),
    ("edtech.batches.opened", "The batch is open."),
    ("edtech.batches.of", "of"),
    ("edtech.batches.minutes", "min"),
    ("edtech.batches.weekday.1", "Mon"),
    ("edtech.batches.weekday.2", "Tue"),
    ("edtech.batches.weekday.3", "Wed"),
    ("edtech.batches.weekday.4", "Thu"),
    ("edtech.batches.weekday.5", "Fri"),
    ("edtech.batches.weekday.6", "Sat"),
    ("edtech.batches.weekday.7", "Sun"),
    ("edtech.batches.status.planned", "Planned"),
    ("edtech.batches.status.open", "Open"),
    ("edtech.batches.status.running", "Running"),
    ("edtech.batches.status.completed", "Completed"),
    ("edtech.batches.status.cancelled", "Cancelled"),
    ("edtech.batches.status.terminated", "Terminated"),
    (
        "edtech.batches.problem.plan_incomplete",
        "The batch needs seats, a tutor, weekdays, a duration and a first day not after the last day.",
    ),
    (
        "edtech.batches.problem.no_sessions",
        "The weekly pattern gives no session between the two dates.",
    ),
    (
        "edtech.batches.problem.new_tutor",
        "Name the new tutor and the first day they teach.",
    ),
    (
        "edtech.batches.problem.attendance_open",
        "Some sessions are still waiting for their attendance.",
    ),
    (
        "edtech.batches.problem.start_time",
        "The start time must be hours and minutes, for example 18:00.",
    ),
    (
        "edtech.batches.problem.duration",
        "A session must last at least one minute.",
    ),
    (
        "edtech.batches.problem.weekdays",
        "Name at least one weekday, from 1 (Monday) to 7 (Sunday).",
    ),
    (
        "edtech.batches.problem.dates",
        "The last day is before the first day.",
    ),
    (
        "edtech.batches.problem.course_inactive",
        "This course is not active: plan no new batch of it.",
    ),
    (
        "edtech.batches.problem.capacity",
        "A batch needs at least one seat.",
    ),
    // The seats.
    (
        "edtech.seats.problem.full",
        "The batch has no free seat.",
    ),
];

/// The text for `key`: the app's translation when it has one, else the
/// English one.
#[allow(dead_code)]
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    t_with(lang, key, &[])
}

/// Like [`t`], replacing `:name` placeholders with `params`.
#[allow(dead_code)]
pub(crate) fn t_with(lang: &Lang, key: &str, params: &[(&str, &dyn Display)]) -> String {
    let found = lang.t(key, params);
    if found != key {
        return found;
    }
    english(key, params)
}

/// Every text of the app in `lang`, keyed without the `edtech.` prefix
/// (`courses.title`), for the templates: `tx['courses.title']`.
#[allow(dead_code)]
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("edtech."), t(lang, key)))
        .collect()
}

/// The English text for `key` with `params` filled in; the key itself when
/// it has no text.
#[allow(dead_code)]
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
        assert!(keys.iter().all(|key| key.starts_with("edtech.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_placeholder_is_filled_in() {
        assert_eq!(english("edtech.none", &[]), "—");
        assert_eq!(english("edtech.unknown", &[]), "edtech.unknown");
    }
}
