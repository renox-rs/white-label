//! The adjustment timesheet (wl-attendance §3, §4 `open_adjustment`, §5.8;
//! `HRM.ACL.08`, BL-ATT-07).
//!
//! A `locked` timesheet is never edited or reopened. A correction is a new
//! timesheet of kind `adjustment` that holds only the signed difference
//! (minutes, + or −, per day, cost object and category), with a reason, an
//! origin and where it flows (`affects`). [`open`] creates it `open`, linked
//! to the original, which it does not touch: the original's entries, totals,
//! state, consumers and `updated_at` stay as they are, and its `adjusted_by`
//! is the list of the adjustments that point to it
//! ([`Timesheet::adjusted_by`]).
//!
//! It is refused unless:
//!
//! - the original is `locked` and of kind `regular` (an `approved` one that
//!   nothing used is reopened instead; an adjustment never points to
//!   another adjustment);
//! - a reason and an origin are given; for `client_dispute` a linked ticket
//!   or invoice dispute too;
//! - `affects` is given (default `billing` for a client dispute: the worker's
//!   pay is not cut because the client disputes an invoice);
//! - there is at least one entry, every entry's day lies in the original's
//!   period, and no entry takes the original plus its approved adjustments
//!   below 0 h for a day and category (the entries of the request add up).
//!
//! The adjustment copies owner, period, context, source of hours, approver
//! kind, approver role and party from the original: it runs through the same
//! machine and is approved by the same approver kind (BL-ATT-08). Its
//! creator is kept in `created_by` for that rule.
//!
//! Decisions:
//!
//! - The permission `timesheets.adjust` is checked in the scope of the
//!   original's context. The system's own `provisional_reconciliation`
//!   adjustment (BL-ATT-13) and the check that a linked dispute is still
//!   open (BL-ATT-09, which owns the dispute) are not here: this issue
//!   checks that a link is given.
//! - A refusal writes nothing: the timesheet and its entries are created in
//!   one transaction.

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::Subject;

use crate::access;
use crate::entry::{self, Category, NewEntry, TimesheetEntry};
use crate::permissions;
use crate::service::refuse;
use crate::texts::english;
use crate::timesheet::{AdjustmentOrigin, Affects, Timesheet, TimesheetKind, TimesheetStatus};

/// What a user with `timesheets.adjust` opens.
#[derive(Debug, Clone)]
pub struct OpenAdjustment {
    /// The locked timesheet to correct.
    pub original: i64,
    /// Why.
    pub reason: String,
    /// The origin; none when not given.
    pub origin: Option<AdjustmentOrigin>,
    /// The linked ticket or invoice dispute (`client_dispute`).
    pub linked: Option<Subject>,
    /// Where it flows; none takes the default of the origin.
    pub affects: Option<Affects>,
    /// The signed entries.
    pub entries: Vec<NewEntry>,
}

/// The default `affects` of an origin, if it has one.
pub fn default_affects(origin: AdjustmentOrigin) -> Option<Affects> {
    (origin == AdjustmentOrigin::ClientDispute).then_some(Affects::Billing)
}

/// Opens an adjustment timesheet on a `locked` one.
///
/// # Errors
///
/// `NotFound`; `Forbidden` without `timesheets.adjust` in the scope of the
/// original's context; `422` as the module docs list.
pub async fn open(state: &AppState, actor: &AuthUser, input: OpenAdjustment) -> Result<Timesheet> {
    let original = Timesheet::find_or_404(&state.db, input.original).await?;
    if !access::holds_in_context(
        state,
        actor.id,
        permissions::TIMESHEETS_ADJUST,
        original.context().as_ref(),
    )
    .await?
    {
        return Err(Error::Forbidden);
    }
    if original.kind == TimesheetKind::Adjustment {
        return Err(refuse(english("wl_attendance.adjustment.of_adjustment", &[])));
    }
    if original.status != TimesheetStatus::Locked {
        return Err(refuse(english("wl_attendance.adjustment.not_locked", &[])));
    }
    let reason = input.reason.trim().to_owned();
    if reason.is_empty() {
        return Err(refuse(english("wl_attendance.adjustment.reason_required", &[])));
    }
    let Some(origin) = input.origin else {
        return Err(refuse(english("wl_attendance.adjustment.origin_required", &[])));
    };
    if origin == AdjustmentOrigin::ClientDispute && input.linked.is_none() {
        return Err(refuse(english("wl_attendance.adjustment.link_required", &[])));
    }
    let Some(affects) = input.affects.or_else(|| default_affects(origin)) else {
        return Err(refuse(english("wl_attendance.adjustment.affects_required", &[])));
    };
    if input.entries.is_empty() || input.entries.iter().any(|e| e.minutes.iter().all(|(_, m)| *m == 0)) {
        return Err(refuse(english("wl_attendance.adjustment.no_entries", &[])));
    }
    for new in &input.entries {
        if new.date < original.period_from || new.date > original.period_to {
            return Err(refuse(english(
                "wl_attendance.adjustment.outside_original",
                &[
                    ("date", &new.date),
                    ("from", &original.period_from),
                    ("to", &original.period_to),
                ],
            )));
        }
    }
    let standing = entry::standing_entries(&state.db, original.id, None).await?;
    if let Some(message) = entry::below_zero(&standing, &input.entries) {
        return Err(refuse(message));
    }

    let mut tx = state.db.begin().await?;
    let adjustment = create_in(&mut tx, &original, actor.id, &reason, origin, &input, affects).await?;
    tx.commit().await?;
    Timesheet::find_or_404(&state.db, adjustment.id).await
}

/// Inserts the adjustment and its entries inside `tx`.
async fn create_in(
    tx: &mut Transaction,
    original: &Timesheet,
    by: i64,
    reason: &str,
    origin: AdjustmentOrigin,
    input: &OpenAdjustment,
    affects: Affects,
) -> Result<Timesheet> {
    let adjustment = Timesheet::create(
        &mut *tx,
        Timesheet {
            owner_employee_id: original.owner_employee_id,
            period_from: original.period_from,
            period_to: original.period_to,
            context_type: original.context_type.clone(),
            context_id: original.context_id,
            kind: TimesheetKind::Adjustment,
            hours_source: original.hours_source,
            approver_kind: original.approver_kind,
            approver_role: original.approver_role.clone(),
            approver_party: original.approver_party,
            adjusts_id: Some(original.id),
            reason: Some(reason.to_owned()),
            origin: Some(origin),
            linked_type: input.linked.as_ref().map(|s| s.subject_type.clone()),
            linked_id: input.linked.as_ref().map(|s| s.subject_id),
            affects: Some(affects),
            created_by: Some(by),
            ..Timesheet::default()
        },
    )
    .await?;
    for new in &input.entries {
        let mut row = TimesheetEntry {
            timesheet_id: adjustment.id,
            employee_id: original.owner_employee_id,
            date: new.date,
            cost_object_type: new.cost_object.as_ref().map(|s| s.subject_type.clone()),
            cost_object_id: new.cost_object.as_ref().map(|s| s.subject_id),
            billable: new.billable,
            source: original.hours_source,
            note: new.note.clone(),
            ..TimesheetEntry::default()
        };
        for category in Category::ALL {
            let minutes = new
                .minutes
                .iter()
                .filter(|(c, _)| *c == category)
                .map(|(_, m)| *m)
                .sum();
            match category {
                Category::Normal => row.normal_minutes = minutes,
                Category::Overtime => row.overtime_minutes = minutes,
                Category::Night => row.night_minutes = minutes,
                Category::RestDay => row.rest_day_minutes = minutes,
                Category::PublicHoliday => row.public_holiday_minutes = minutes,
            }
        }
        TimesheetEntry::create(&mut *tx, row).await?;
    }
    Ok(adjustment)
}

/// One signed entry as sent by a form, in JSON:
/// `{"date": "2026-11-02", "category": "normal", "minutes": -480,
/// "cost_object": "construction.cost_code:3"}`.
#[derive(serde::Deserialize, Debug)]
struct EntryInput {
    date: NaiveDate,
    #[serde(default = "normal")]
    category: String,
    minutes: i64,
    #[serde(default)]
    cost_object: Option<String>,
}

fn normal() -> String {
    "normal".to_owned()
}

/// Reads the JSON list of signed entries of a form.
///
/// # Errors
///
/// `422` naming what could not be read.
pub fn parse_entries(text: &str) -> Result<Vec<NewEntry>> {
    let bad = |detail: String| refuse(english("wl_attendance.adjustment.bad_entries", &[("detail", &detail)]));
    let rows: Vec<EntryInput> = renox::serde_json::from_str(text).map_err(|e| bad(e.to_string()))?;
    rows.into_iter()
        .map(|row| {
            let category = Category::parse(&row.category)
                .ok_or_else(|| bad(format!("category {}", row.category)))?;
            let mut new = NewEntry::of(row.date, category, row.minutes);
            if let Some(text) = row.cost_object.filter(|t| !t.trim().is_empty()) {
                let subject: Subject = text
                    .trim()
                    .parse()
                    .map_err(|_| bad(format!("cost object {text}")))?;
                new = new.on(&subject);
            }
            Ok(new)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_dispute_defaults_to_billing_and_the_others_have_no_default() {
        assert_eq!(
            default_affects(AdjustmentOrigin::ClientDispute),
            Some(Affects::Billing)
        );
        assert_eq!(default_affects(AdjustmentOrigin::AttendanceError), None);
        assert_eq!(default_affects(AdjustmentOrigin::PayrollError), None);
    }

    #[test]
    fn entries_are_read_from_json() {
        let entries = parse_entries(
            r#"[{"date": "2026-11-02", "category": "overtime", "minutes": -60,
                 "cost_object": "construction.cost_code:3"}]"#,
        )
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].cost_object, Some(Subject::new("construction.cost_code", 3)));
        assert_eq!(entries[0].minutes, vec![(Category::Overtime, -60)]);
        assert!(parse_entries("not json").is_err());
        assert!(parse_entries(r#"[{"date": "2026-11-02", "category": "x", "minutes": 1}]"#).is_err());
    }
}
