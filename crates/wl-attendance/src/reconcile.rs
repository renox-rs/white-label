//! Reconciling provisional hours when the timesheet is approved
//! (wl-attendance §5.7 point 3, §6 *Events emitted*; BL-ATT-13).
//!
//! Payroll paid a timesheet that the client had not approved on provisional
//! hours ([`crate::payroll`]). When the timesheet is approved, by the
//! approver, on behalf of the client or by deemed approval, it is `lock`ed at
//! once (payroll is its first consumer), and the approved hours are compared
//! with the hours that were paid, per day and category. A difference makes
//! the system, in the same step:
//!
//! 1. open an adjustment timesheet of origin `provisional_reconciliation`
//!    that affects `payroll` only, linked to the original, with the signed
//!    difference per day and category;
//! 2. submit it and approve it ([`crate::approval::APPROVE_BY_SYSTEM`]),
//!    copying the approver and the approval basis of the original's approval:
//!    the original's approval already covers these hours, so the approver is
//!    not asked again;
//! 3. emit `TimesheetReconciliationNeeded` and nothing else: the adjustment
//!    emits no `TimesheetApproved` of its own, so that payroll applies the
//!    difference once.
//!
//! [`on_approved`] is the listener of `TimesheetApproved`. It is idempotent:
//! one reconciliation per timesheet and payroll run, found by the adjustment
//! linked to the run, so the event handled twice, or the run repeated after a
//! failure, creates one adjustment and says nothing the second time. With no
//! difference nothing is created and nothing is emitted. A category that
//! moves without the total moving counts (150 normal and 10 overtime
//! approved against 160 normal paid gives -10 normal and +10 overtime).
//!
//! Decisions:
//!
//! - **The link of the adjustment is the payroll run** (`linked`), which is
//!   what one reconciliation per run is found by. The adjustment has no
//!   linked ticket or dispute.
//! - **The difference is against the hours paid, not against the original's
//!   hours**, so the rule of [`crate::entry::below_zero`] (no net below 0 h
//!   for a day and category) does not apply to it: approving 0 h against 160
//!   h paid is a difference of -160 h on an original that has none.
//! - **The entries have no cost object.** The difference is for payroll; the
//!   original's approval has posted the labour cost of the approved hours
//!   already, so nothing is posted for the adjustment.
//! - **The adjustment's `created_by` is empty**: the system made it.
//! - The reconciliation is driven by the event and not by the approval
//!   itself so that it can be run again (emit the event, or call
//!   [`reconcile`]) if it failed after the approval had committed.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::Subject;
use wl_kernel::attendance::{CategoryMinutes, TimesheetApproved};
use wl_kernel::timesheets::{HoursLine, TIMESHEET_SUBJECT_TYPE, TimesheetReconciliationNeeded};

use crate::adjustment::{self, OpenAdjustment};
use crate::approval::APPROVE_BY_SYSTEM;
use crate::entry::{Category, NewEntry};
use crate::payroll::{ProvisionalHours, approved_lines};
use crate::service::refuse;
use crate::submit::SUBMIT_BY_SYSTEM;
use crate::texts::english;
use crate::timesheet::{AdjustmentOrigin, Affects, Timesheet, TimesheetKind, TimesheetStatus};

/// The signed difference per day and category, approved minus provisional;
/// pairs with no difference are left out.
fn delta_of(approved: &[HoursLine], provisional: &[HoursLine]) -> Vec<HoursLine> {
    let mut sums: BTreeMap<(NaiveDate, String), i64> = BTreeMap::new();
    for line in approved {
        *sums.entry((line.date, line.category.clone())).or_default() += line.minutes;
    }
    for line in provisional {
        *sums.entry((line.date, line.category.clone())).or_default() -= line.minutes;
    }
    sums.into_iter()
        .filter(|(_, minutes)| *minutes != 0)
        .map(|((date, category), minutes)| HoursLine {
            date,
            category,
            minutes,
        })
        .collect()
}

/// The minutes per category over `lines`, in the order of
/// [`Category::ALL`]; categories that add up to nothing are left out.
fn totals(lines: &[HoursLine]) -> Vec<CategoryMinutes> {
    Category::ALL
        .iter()
        .filter_map(|category| {
            let minutes: i64 = lines
                .iter()
                .filter(|line| line.category == category.as_str())
                .map(|line| line.minutes)
                .sum();
            (minutes != 0).then(|| CategoryMinutes {
                category: category.as_str().to_owned(),
                minutes,
            })
        })
        .collect()
}

/// The entries of the adjustment: one per day, with the signed minutes of each
/// category.
fn entries_of(delta: &[HoursLine], source: crate::timesheet::HoursSource) -> Vec<NewEntry> {
    let mut by_day: BTreeMap<NaiveDate, Vec<(Category, i64)>> = BTreeMap::new();
    for line in delta {
        if let Some(category) = Category::parse(&line.category) {
            by_day
                .entry(line.date)
                .or_default()
                .push((category, line.minutes));
        }
    }
    by_day
        .into_iter()
        .map(|(date, minutes)| NewEntry {
            date,
            cost_object: None,
            minutes,
            billable: false,
            source,
            note: String::new(),
        })
        .collect()
}

/// The reconciliation adjustment of `original` for payroll run `run`, if one
/// was made.
async fn existing(db: &Db, original: &Timesheet, run: &Subject) -> Result<Option<Timesheet>> {
    Timesheet::query()
        .where_(Timesheet::ADJUSTS_ID.eq(original.id))
        .where_(Timesheet::ORIGIN.eq(AdjustmentOrigin::ProvisionalReconciliation))
        .where_(Timesheet::LINKED_TYPE.eq(run.subject_type.as_str()))
        .where_(Timesheet::LINKED_ID.eq(run.subject_id))
        .first(db)
        .await
}

/// Reconciles the approved, `locked` timesheet `timesheet_id` with the hours
/// payroll paid for it provisionally (see the module docs). Returns the
/// adjustment, `None` when the timesheet was not paid provisionally or the
/// hours agree.
///
/// # Errors
///
/// `NotFound`; `422` when the timesheet is not `locked` (it is locked when
/// it is approved); the database's errors.
pub async fn reconcile(state: &AppState, timesheet_id: i64) -> Result<Option<Timesheet>> {
    let original = Timesheet::find_or_404(&state.db, timesheet_id).await?;
    let Some(reference) = original.paid_provisionally_by.clone() else {
        return Ok(None);
    };
    if original.kind != TimesheetKind::Regular
        || !matches!(
            original.status,
            TimesheetStatus::Approved | TimesheetStatus::Locked
        )
    {
        return Ok(None);
    }
    if original.status != TimesheetStatus::Locked {
        return Err(refuse(english(
            "wl_attendance.reconcile.not_locked",
            &[("timesheet", &original.subject())],
        )));
    }
    let run: Subject = reference.parse().map_err(|_| {
        refuse(english(
            "wl_attendance.reconcile.bad_run",
            &[("run", &reference)],
        ))
    })?;
    if existing(&state.db, &original, &run).await?.is_some() {
        return Ok(None);
    }

    let paid: Vec<HoursLine> = ProvisionalHours::of_timesheet(&state.db, original.id)
        .await?
        .into_iter()
        .filter(|row| row.payroll_run == reference)
        .map(|row| HoursLine {
            date: row.date,
            category: row.category,
            minutes: row.minutes,
        })
        .collect();
    let approved = approved_lines(&state.db, &original).await?;
    let delta = delta_of(&approved, &paid);
    if delta.is_empty() {
        return Ok(None);
    }

    let input = OpenAdjustment {
        original: original.id,
        reason: english("wl_attendance.reconcile.reason", &[("run", &run)]),
        origin: Some(AdjustmentOrigin::ProvisionalReconciliation),
        linked: Some(run.clone()),
        affects: Some(Affects::Payroll),
        entries: entries_of(&delta, original.hours_source),
        evidence: Vec::new(),
    };
    let mut tx = state.db.begin().await?;
    let created = adjustment::create_in(
        &mut tx,
        &original,
        None,
        &input.reason,
        AdjustmentOrigin::ProvisionalReconciliation,
        &input,
        Affects::Payroll,
    )
    .await?;
    tx.commit().await?;

    // Submitted and approved in the same step, as the system: the original's
    // approval stands for these hours.
    let _submitted = created
        .transition(SUBMIT_BY_SYSTEM)
        .by_system()
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    let submitted = Timesheet::find_or_404(&state.db, created.id).await?;
    let _approved = submitted
        .transition(APPROVE_BY_SYSTEM)
        .by_system()
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    let adjustment = Timesheet::find_or_404(&state.db, created.id).await?;

    state
        .emit(TimesheetReconciliationNeeded {
            timesheet: original.subject(),
            owner: original.owner(),
            payroll_run: run,
            provisional: totals(&paid),
            approved: totals(&approved),
            delta: totals(&delta),
            adjustment: adjustment.subject(),
        })
        .await?;
    Ok(Some(adjustment))
}

/// The listener of `TimesheetApproved`: a regular timesheet that payroll paid
/// provisionally is reconciled.
///
/// # Errors
///
/// What [`reconcile`] answers.
pub async fn on_approved(state: &AppState, event: &TimesheetApproved) -> Result {
    if event.kind != TimesheetKind::Regular.as_str()
        || event.timesheet.subject_type != TIMESHEET_SUBJECT_TYPE
    {
        return Ok(());
    }
    reconcile(state, event.timesheet.subject_id)
        .await
        .map(|_| ())
}

/// Registers the listener of `TimesheetApproved`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: TimesheetApproved, state: AppState| async move {
        on_approved(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(day: u32, category: &str, minutes: i64) -> HoursLine {
        HoursLine {
            date: NaiveDate::from_ymd_opt(2026, 11, day).unwrap(),
            category: category.into(),
            minutes,
        }
    }

    #[test]
    fn the_difference_is_approved_minus_paid_per_day_and_category() {
        let approved = [line(2, "normal", 480), line(3, "normal", 420)];
        let paid = [line(2, "normal", 480), line(3, "normal", 480)];
        assert_eq!(delta_of(&approved, &paid), vec![line(3, "normal", -60)]);
        assert!(delta_of(&paid, &paid).is_empty());
    }

    #[test]
    fn a_category_that_moves_counts_even_when_the_total_does_not() {
        let approved = [line(2, "normal", 480), line(3, "overtime", 600)];
        let paid = [line(2, "normal", 480), line(3, "normal", 600)];
        let delta = delta_of(&approved, &paid);
        assert_eq!(
            totals(&delta),
            vec![
                CategoryMinutes {
                    category: "normal".into(),
                    minutes: -600
                },
                CategoryMinutes {
                    category: "overtime".into(),
                    minutes: 600
                },
            ]
        );
        let entries = entries_of(&delta, crate::timesheet::HoursSource::Attendance);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].minutes.len(), 2);
    }

    #[test]
    fn hours_paid_and_not_approved_are_taken_back() {
        let paid = [line(2, "normal", 480)];
        assert_eq!(delta_of(&[], &paid), vec![line(2, "normal", -480)]);
    }
}
