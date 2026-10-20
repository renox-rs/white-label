//! The text of a manager's digest (wl-kernel §5.13 step 6).
//!
//! One message with a section per scope. Each section lists the records
//! voided (with the user and the reason) and the late entries (with the
//! user, the reason and the two times), and under each late entry the rule
//! runs "already fired, review". A confidential record shows only its type
//! and number. Times are Asia/Dubai.

use renox::db::DateTime;

use crate::documents::dubai_text;
use crate::subject::Subject;
use crate::texts::{en, en_with};

/// Whether an item is a void or a late entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// A record voided as entered in error.
    Void,
    /// A record entered with the late-entry flag.
    Late,
}

/// A rule run listed for review, as the digest shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiredLine {
    /// The rule's title (or its id).
    pub title: String,
    /// The record the run was about, named (`booking B`).
    pub record: String,
    /// When the run fired.
    pub at: DateTime,
}

/// One void or late entry in the digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Void or late entry.
    pub kind: ItemKind,
    /// The record.
    pub record: Subject,
    /// Its number, if it has one.
    pub number: Option<String>,
    /// Who voided or entered it.
    pub by: String,
    /// The void's reason, or the reason it is late.
    pub reason: String,
    /// When it was voided or entered.
    pub at: DateTime,
    /// The business time of a late entry.
    pub occurred_at: Option<DateTime>,
    /// The runs of a late entry to review.
    pub fired: Vec<FiredLine>,
    /// Whether only the type and number may be shown.
    pub confidential: bool,
}

/// The records of one scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The scope, named (`Tower A`).
    pub scope: String,
    /// Its voids and late entries, oldest first.
    pub items: Vec<Item>,
}

/// A record named by its type and number: `receipt RCP-ACME-2026-0042`; a
/// record with no number by its id: `booking #2`.
pub fn record_name(record: &Subject, number: Option<&str>) -> String {
    let entity = record.entity().replace('_', " ");
    match number {
        Some(number) => format!("{entity} {number}"),
        None => format!("{entity} #{}", record.subject_id),
    }
}

fn line(item: &Item) -> String {
    let record = record_name(&item.record, item.number.as_deref());
    if item.confidential {
        return en_with("wl_kernel.digest.confidential_line", &[("record", &record)]);
    }
    let mut text = match item.kind {
        ItemKind::Void => en_with(
            "wl_kernel.digest.void_line",
            &[
                ("record", &record),
                ("user", &item.by),
                ("reason", &item.reason),
                ("at", &dubai_text(item.at)),
            ],
        ),
        ItemKind::Late => en_with(
            "wl_kernel.digest.late_line",
            &[
                ("record", &record),
                ("user", &item.by),
                ("reason", &item.reason),
                (
                    "occurred",
                    &item.occurred_at.map_or_else(|| "—".to_owned(), dubai_text),
                ),
                ("entered", &dubai_text(item.at)),
            ],
        ),
    };
    for run in &item.fired {
        text.push('\n');
        text.push_str(&en_with(
            "wl_kernel.digest.review_line",
            &[
                ("rule", &run.title),
                ("record", &run.record),
                ("at", &dubai_text(run.at).split_at(11).1.to_owned()),
            ],
        ));
    }
    text
}

/// The text for the period `from` to `until` (the end not included): the
/// heading, then each scope with its voids and its late entries.
pub fn render(from: DateTime, until: DateTime, sections: &[Section]) -> String {
    let mut out = en_with(
        "wl_kernel.digest.heading",
        &[("from", &dubai_text(from)), ("until", &dubai_text(until))],
    );
    for section in sections {
        out.push_str("\n\n");
        out.push_str(&section.scope);
        for (kind, heading) in [
            (ItemKind::Void, "wl_kernel.digest.voided"),
            (ItemKind::Late, "wl_kernel.digest.late"),
        ] {
            let items: Vec<&Item> = section.items.iter().filter(|i| i.kind == kind).collect();
            if items.is_empty() {
                continue;
            }
            out.push('\n');
            out.push_str(&en(heading));
            for item in items {
                out.push('\n');
                out.push_str(&line(item));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime {
        text.parse().expect("an instant")
    }

    fn void() -> Item {
        Item {
            kind: ItemKind::Void,
            record: Subject::new("payments.receipt", 3),
            number: Some("RCP-ACME-2026-0042".into()),
            by: "Vera".into(),
            reason: "duplicate".into(),
            at: at("2026-11-03T10:20:00Z"),
            occurred_at: None,
            fired: Vec::new(),
            confidential: false,
        }
    }

    fn late() -> Item {
        Item {
            kind: ItemKind::Late,
            record: Subject::new("bookings.check_in", 7),
            number: None,
            by: "Anna".into(),
            reason: "router down".into(),
            at: at("2026-11-03T06:30:00Z"),
            occurred_at: Some(at("2026-11-03T05:05:00Z")),
            fired: vec![FiredLine {
                title: "no-show rule".into(),
                record: "booking B".into(),
                at: at("2026-11-03T05:15:00Z"),
            }],
            confidential: false,
        }
    }

    #[test]
    fn a_record_is_named_by_its_type_and_number_or_id() {
        let receipt = Subject::new("payments.receipt", 3);
        assert_eq!(
            record_name(&receipt, Some("RCP-ACME-2026-0042")),
            "receipt RCP-ACME-2026-0042"
        );
        assert_eq!(
            record_name(&Subject::new("bookings.check_in", 7), None),
            "check in #7"
        );
    }

    #[test]
    fn a_void_is_listed_with_its_user_and_reason() {
        let text = line(&void());
        assert_eq!(
            text,
            "- receipt RCP-ACME-2026-0042: voided by Vera, reason: duplicate (2026-11-03 14:20)"
        );
    }

    #[test]
    fn a_late_entry_lists_the_runs_to_review_under_it() {
        let text = line(&late());
        assert_eq!(
            text,
            "- check in #7: entered by Anna, reason: router down; business time 2026-11-03 09:05, entered 2026-11-03 10:30\n  already fired, review: no-show rule, booking B, 09:15"
        );
    }

    #[test]
    fn a_confidential_record_shows_only_its_type_and_number() {
        let mut item = late();
        item.confidential = true;
        item.number = Some("CI-9".into());
        assert_eq!(line(&item), "- check in CI-9");
    }

    #[test]
    fn the_digest_has_a_section_per_scope_with_voids_before_late_entries() {
        let text = render(
            at("2026-11-03T03:00:00Z"),
            at("2026-11-04T03:00:00Z"),
            &[
                Section {
                    scope: "Tower A".into(),
                    items: vec![late(), void()],
                },
                Section {
                    scope: "Tower B".into(),
                    items: vec![void()],
                },
            ],
        );
        let tower_a = text.find("Tower A").expect("Tower A");
        let tower_b = text.find("Tower B").expect("Tower B");
        assert!(tower_a < tower_b);
        assert!(text.starts_with(
            "Voids and late entries from 2026-11-03 07:00 to 2026-11-04 07:00 (Asia/Dubai)"
        ));
        let voided = text.find("Voided").expect("Voided");
        let entries = text.find("Late entries").expect("Late entries");
        assert!(voided < entries, "{text}");
        assert!(text.contains("already fired, review: no-show rule, booking B, 09:15"));
    }
}
