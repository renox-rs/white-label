//! The back office's part of late entries (wl-kernel §5.13): the fields an
//! entry form asks for when the business time is old, and the card on the
//! record's page that lists the rule runs "already fired, review".
//!
//! Both are partials a module's page includes, fed by a value built here
//! (like [`crate::voids::web`]):
//!
//! ```html
//! <rx-include template="wl-kernel/late_entry_fields.html"/>  <!-- in the form -->
//! <rx-include template="wl-kernel/late_entry_card.html"/>    <!-- on the record -->
//! ```

use renox::prelude::*;
use renox::serde_json::{Value, json};

use super::assess::Assessment;
use super::entry::{LateEntry, LateEntryStatus};
use crate::documents::dubai_text;
use crate::subject::Subject;
use crate::texts;

/// The template of the fields, compiled into the kernel.
pub const FIELDS_TEMPLATE: &str = "wl-kernel/late_entry_fields.html";

/// The template of the card, compiled into the kernel.
pub const CARD_TEMPLATE: &str = "wl-kernel/late_entry_card.html";

/// What the fields partial needs for the entry `found` by
/// [`preview`](super::preview): the reason (required) and the outage
/// reference. `None` when the entry is not late, so the partial shows
/// nothing.
pub fn fields_for(found: &Assessment, lang: &Lang) -> Option<Value> {
    if !found.needs_reason() {
        return None;
    }
    let t = |key: &str| texts::t(lang, key);
    let notice = if found.needs_approval() {
        texts::t_with(
            lang,
            "wl_kernel.late_entries.notice_approval",
            &[("minutes", &found.minutes_late())],
        )
    } else {
        texts::t_with(
            lang,
            "wl_kernel.late_entries.notice",
            &[("minutes", &found.minutes_late())],
        )
    };
    Some(json!({
        "labels": {
            "notice": notice,
            "reason": t("wl_kernel.late_entries.reason"),
            "reason_hint": t("wl_kernel.late_entries.reason_hint"),
            "outage": t("wl_kernel.late_entries.outage"),
            "outage_hint": t("wl_kernel.late_entries.outage_hint"),
        },
    }))
}

/// The card of the late entry that created `record`, or `None` when the
/// record was not entered late.
pub async fn card_for(db: &Db, record: &Subject, lang: &Lang) -> Result<Option<Value>> {
    let Some(entry) = LateEntry::of_record(db, &record.subject_type, record.subject_id).await?
    else {
        return Ok(None);
    };
    Ok(Some(card(&entry, lang)))
}

/// The card of `entry`.
pub fn card(entry: &LateEntry, lang: &Lang) -> Value {
    let t = |key: &str| texts::t(lang, key);
    let (status_key, status_kind) = match entry.status {
        LateEntryStatus::Requested => ("requested", "neutral"),
        LateEntryStatus::AwaitingApproval => ("awaiting_approval", "warning"),
        LateEntryStatus::Recorded => ("recorded", "success"),
        LateEntryStatus::Discarded => ("discarded", "neutral"),
    };
    let runs: Vec<Value> = entry
        .fired_runs
        .0
        .iter()
        .map(|run| {
            json!({
                "text": texts::t_with(
                    lang,
                    "wl_kernel.late_entries.fired_run",
                    &[
                        ("rule", &run.rule),
                        ("subject", &run.subject),
                        ("at", &dubai_text(run.at)),
                    ],
                ),
            })
        })
        .collect();
    json!({
        "status_label": t(&format!("wl_kernel.late_entries.status.{status_key}")),
        "status_kind": status_kind,
        "late_by": texts::t_with(
            lang,
            "wl_kernel.late_entries.late_by",
            &[("minutes", &entry.minutes_late)],
        ),
        "occurred_at": dubai_text(entry.occurred_at),
        "entered_at": dubai_text(entry.entered_at),
        "reason": entry.reason,
        "outage_reference": entry.outage_reference.clone().unwrap_or_else(|| "—".into()),
        "approval": entry
            .approval_request_id
            .map_or_else(|| "—".to_owned(), |id| format!("#{id}")),
        "runs": runs,
        "labels": {
            "title": t("wl_kernel.late_entries.title"),
            "occurred_at": t("wl_kernel.late_entries.occurred_at"),
            "entered_at": t("wl_kernel.late_entries.entered_at"),
            "reason": t("wl_kernel.late_entries.reason"),
            "outage": t("wl_kernel.late_entries.outage"),
            "approval": t("wl_kernel.late_entries.approval"),
            "review": t("wl_kernel.late_entries.review"),
        },
    })
}
