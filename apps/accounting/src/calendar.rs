//! The compliance calendar of an engagement (04 §4.3, `ACC.CMP.01`): the
//! deadline tasks generated from the client's tax profile when the
//! engagement is activated, through the kernel's
//! [`DeadlineTasks`](wl_kernel::deadlines::DeadlineTasks) (wl-ticketing).
//!
//! A simple generator, one task per filing that recurs:
//!
//! - **VAT return** when VAT registered: the first VAT period ending on or
//!   after the activation day, stepping from the profile's first period end by
//!   its period (1 or 3 months, month ends); due 28 days after the period
//!   ends; 21 days of lead time; recurring monthly or quarterly.
//! - **Corporate tax return**: the first financial year whose return (due at
//!   the end of the ninth month after the year end) is due on or after the
//!   activation day; 60 days of lead time; recurring yearly.
//!
//! Each task names the engagement as its source and the client as its party,
//! is owned by the accountant role and reviewed by the manager role, and
//! needs evidence (the FTA acknowledgement). Regeneration after a tax
//! profile change is `ACC.RUL.10` ([`crate::regeneration`], BL-ACC-15), which
//! plans with [`plan_ahead`].

use renox::chrono::{Datelike, Duration, Months, NaiveDate};
use renox::prelude::*;
use wl_kernel::deadlines::{DeadlineTaskId, NewDeadlineTask, TaskRecurrence, deadline_tasks};

use crate::engagement::Engagement;
use crate::permissions::{ACCOUNTANT_ROLE, MANAGER_ROLE};
use crate::tax_profile::{TaxProfile, VatPeriod};
use crate::texts::english;

/// The task type of a VAT return.
pub const VAT_RETURN: &str = "vat_return";
/// The task type of a corporate tax return.
pub const CT_RETURN: &str = "ct_return";

/// Days after the VAT period end the return is due (UAE: 28).
pub const VAT_DUE_DAYS: i64 = 28;
/// Months after the financial year end the corporate tax return is due
/// (UAE: 9, at the end of that month).
pub const CT_DUE_MONTHS: u32 = 9;
/// Lead time of a VAT return, in days.
pub const VAT_LEAD_DAYS: i64 = 21;
/// Lead time of a corporate tax return, in days.
pub const CT_LEAD_DAYS: i64 = 60;

/// One task the calendar plans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedTask {
    /// [`VAT_RETURN`] or [`CT_RETURN`].
    pub task_type: &'static str,
    /// The title.
    pub title: String,
    /// The last day of the period it covers.
    pub period_end: NaiveDate,
    /// The due date.
    pub due_date: NaiveDate,
    /// The lead time, in days.
    pub lead_days: i64,
    /// How often it comes back.
    pub recurrence: TaskRecurrence,
}

/// The last day of the month of `date`.
fn month_end(date: NaiveDate) -> NaiveDate {
    let first = date.with_day(1).unwrap_or(date);
    first
        .checked_add_months(Months::new(1))
        .map_or(date, |next| next - Duration::days(1))
}

/// The month end `months` months after the month of `date`.
pub(crate) fn month_end_after(date: NaiveDate, months: u32) -> NaiveDate {
    let first = date.with_day(1).unwrap_or(date);
    month_end(
        first
            .checked_add_months(Months::new(months))
            .unwrap_or(first),
    )
}

/// The month end twelve months before the month of `date`.
fn year_before(date: NaiveDate) -> NaiveDate {
    let first = date.with_day(1).unwrap_or(date);
    month_end(first.checked_sub_months(Months::new(12)).unwrap_or(first))
}

/// The VAT return for the period ending `end`, recurring as `profile`'s
/// period.
fn vat_task(profile: &TaxProfile, end: NaiveDate) -> PlannedTask {
    PlannedTask {
        task_type: VAT_RETURN,
        title: english("accounting.calendar.vat_return", &[("end", &end)]),
        period_end: end,
        due_date: end + Duration::days(VAT_DUE_DAYS),
        lead_days: VAT_LEAD_DAYS,
        recurrence: match profile.vat_period {
            VatPeriod::Monthly => TaskRecurrence::Monthly,
            _ => TaskRecurrence::Quarterly,
        },
    }
}

/// The corporate tax return for the financial year ending `end`.
fn ct_task(end: NaiveDate) -> PlannedTask {
    PlannedTask {
        task_type: CT_RETURN,
        title: english("accounting.calendar.ct_return", &[("end", &end)]),
        period_end: end,
        due_date: month_end_after(end, CT_DUE_MONTHS),
        lead_days: CT_LEAD_DAYS,
        recurrence: TaskRecurrence::Yearly,
    }
}

/// The tasks `profile` gives from the day `from`.
pub fn plan(profile: &TaxProfile, from: NaiveDate) -> Vec<PlannedTask> {
    let mut planned = Vec::new();
    let months = profile.vat_period.months();
    if profile.vat_registered
        && months > 0
        && let Some(first) = profile.vat_first_period_end
    {
        let mut end = month_end(first);
        while end < from {
            end = month_end_after(end, months);
        }
        planned.push(vat_task(profile, end));
    }
    if let Some(year_end) = profile.financial_year_end {
        let due = |end: NaiveDate| month_end_after(end, CT_DUE_MONTHS);
        let mut end = month_end(year_end);
        while due(end) < from {
            end = month_end_after(end, 12);
        }
        // A year end given ahead steps back to the earliest return still due.
        loop {
            let earlier = year_before(end);
            if due(earlier) >= from {
                end = earlier;
            } else {
                break;
            }
        }
        planned.push(ct_task(end));
    }
    planned
}

/// Why a task of [`plan_ahead`] comes after the first of its type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Follow {
    /// It is the first task of its type from the day.
    First,
    /// It follows a transitional first period (the effective date lies
    /// inside a regular period, as when the stagger changes): it is created
    /// together with the first task.
    Transition,
    /// The earlier calendar reached this far: a task of the same type and
    /// period is created even when the first one already exists.
    Reach,
}

/// A task of [`plan_ahead`] and why it is planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ahead {
    /// The task.
    pub task: PlannedTask,
    /// Why it is planned.
    pub follow: Follow,
}

/// How far the calendar being regenerated reached before, per tax: the last
/// period end of its open tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reach {
    /// The last VAT period end.
    pub vat: Option<NaiveDate>,
    /// The last corporate tax year end.
    pub ct: Option<NaiveDate>,
}

/// The months between two periods of `task_type` under `profile`: the VAT
/// period, or 12 for corporate tax; 0 without a period.
pub fn step_months(profile: &TaxProfile, task_type: &str) -> u32 {
    if task_type == VAT_RETURN {
        profile.vat_period.months()
    } else {
        12
    }
}

/// The first day of the regular period of `months` months that ends on
/// `end`.
fn regular_start(end: NaiveDate, months: u32) -> NaiveDate {
    let first = end.with_day(1).unwrap_or(end);
    first
        .checked_sub_months(Months::new(months.saturating_sub(1)))
        .unwrap_or(first)
}

/// The tasks `profile` gives from the effective day `from` for a
/// regeneration (`ACC.RUL.10`): the first of each type ([`plan`]), the
/// regular VAT period after a transitional first one (the period from `from`
/// to the first period end on or after it, as when the stagger changes), and
/// the periods up to what the earlier calendar reached (`reach`).
pub fn plan_ahead(profile: &TaxProfile, from: NaiveDate, reach: &Reach) -> Vec<Ahead> {
    let mut ahead = Vec::new();
    let mut firsts: Vec<PlannedTask> = plan(profile, from)
        .into_iter()
        .filter(|p| p.task_type == VAT_RETURN)
        .collect();
    // A corporate tax year is planned by the day its period ends, not by
    // the day its return is due: a year that ended before `from` belongs to
    // the profile that was in force.
    if let Some(year_end) = profile.financial_year_end {
        let mut end = month_end(year_end);
        while end < from {
            end = month_end_after(end, 12);
        }
        loop {
            let earlier = year_before(end);
            if earlier >= from {
                end = earlier;
            } else {
                break;
            }
        }
        firsts.push(ct_task(end));
    }
    for first in firsts {
        let vat = first.task_type == VAT_RETURN;
        let months = step_months(profile, first.task_type);
        let reach_to = if vat { reach.vat } else { reach.ct };
        let transitional = vat && months > 0 && from > regular_start(first.period_end, months);
        let mut end = first.period_end;
        ahead.push(Ahead {
            task: first,
            follow: Follow::First,
        });
        let mut index = 0;
        loop {
            if months == 0 {
                break;
            }
            let next = month_end_after(end, months);
            let by_reach = reach_to.is_some_and(|last| next <= last);
            let by_transition = index == 0 && transitional;
            if !by_reach && !by_transition {
                break;
            }
            ahead.push(Ahead {
                task: if vat {
                    vat_task(profile, next)
                } else {
                    ct_task(next)
                },
                follow: if by_reach {
                    Follow::Reach
                } else {
                    Follow::Transition
                },
            });
            end = next;
            index += 1;
        }
    }
    ahead
}

/// The deadline task to create for `planned` under `engagement`, recurring
/// as `recurrence`.
pub(crate) fn new_task(
    engagement: &Engagement,
    planned: &PlannedTask,
    recurrence: TaskRecurrence,
) -> NewDeadlineTask {
    NewDeadlineTask::new(
        engagement.subject(),
        planned.task_type,
        &planned.title,
        planned.due_date,
    )
    .party(engagement.client())
    .lead_days(planned.lead_days)
    .owner_role(ACCOUNTANT_ROLE)
    .reviewer_role(MANAGER_ROLE)
    .evidence_required()
    .recurrence(recurrence)
    .period_end(planned.period_end)
}

/// Generates the calendar of `engagement` from its client's tax profile in
/// force on `on`; answers the tasks created (none without ticketing or a
/// profile).
///
/// # Errors
///
/// The database's and the task service's errors.
pub async fn generate(
    state: &AppState,
    engagement: &Engagement,
    on: NaiveDate,
) -> Result<Vec<DeadlineTaskId>> {
    let Some(profile) = TaxProfile::current(&state.db, engagement.client(), on).await? else {
        return Ok(Vec::new());
    };
    let tasks = deadline_tasks(state);
    let mut made = Vec::new();
    for planned in plan(&profile, on) {
        let task = new_task(engagement, &planned, planned.recurrence);
        if let Some(id) = tasks.create(state, task).await? {
            made.push(id);
        }
    }
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn profile() -> TaxProfile {
        TaxProfile {
            vat_registered: true,
            trn: Some("100000000000003".into()),
            vat_period: VatPeriod::Quarterly,
            vat_first_period_end: Some(day("2026-03-31")),
            financial_year_end: Some(day("2025-12-31")),
            ct_registration_no: Some("CT-1".into()),
            ..TaxProfile::default()
        }
    }

    #[test]
    fn month_ends_step_by_months() {
        assert_eq!(month_end(day("2026-02-10")), day("2026-02-28"));
        assert_eq!(month_end_after(day("2026-11-30"), 3), day("2027-02-28"));
        assert_eq!(month_end_after(day("2026-12-31"), 9), day("2027-09-30"));
    }

    #[test]
    fn the_next_vat_quarter_and_corporate_tax_year_are_planned() {
        let planned = plan(&profile(), day("2026-11-03"));
        assert_eq!(planned.len(), 2);
        let vat = &planned[0];
        assert_eq!(vat.task_type, VAT_RETURN);
        assert_eq!(vat.period_end, day("2026-12-31"));
        assert_eq!(vat.due_date, day("2027-01-28"));
        assert_eq!(vat.recurrence, TaskRecurrence::Quarterly);
        let ct = &planned[1];
        assert_eq!(ct.task_type, CT_RETURN);
        // FY 2025 was due on 2026-09-30, before the day: FY 2026 is next.
        assert_eq!(ct.period_end, day("2026-12-31"));
        assert_eq!(ct.due_date, day("2027-09-30"));
        assert_eq!(ct.recurrence, TaskRecurrence::Yearly);
    }

    #[test]
    fn a_changed_stagger_plans_the_transitional_period_and_the_regular_one_after_it() {
        let mut february = profile();
        february.vat_first_period_end = Some(day("2026-02-28"));
        february.financial_year_end = None;
        let ahead = plan_ahead(&february, day("2027-02-01"), &Reach::default());
        let ends: Vec<_> = ahead
            .iter()
            .map(|a| (a.task.period_end, a.follow))
            .collect();
        assert_eq!(
            ends,
            [
                (day("2027-02-28"), Follow::First),
                (day("2027-05-31"), Follow::Transition)
            ]
        );
        assert_eq!(ahead[1].task.due_date, day("2027-06-28"));
        // Effective on the first day of a regular quarter: no transition.
        let regular = plan_ahead(&february, day("2026-12-01"), &Reach::default());
        assert_eq!(regular.len(), 1);
        assert_eq!(regular[0].task.period_end, day("2027-02-28"));
    }

    #[test]
    fn the_calendar_reaches_as_far_as_the_earlier_one_did() {
        let mut february = profile();
        february.vat_first_period_end = Some(day("2026-02-28"));
        february.financial_year_end = None;
        let reach = Reach {
            vat: Some(day("2027-08-31")),
            ct: None,
        };
        let ends: Vec<_> = plan_ahead(&february, day("2027-02-01"), &reach)
            .iter()
            .map(|a| (a.task.period_end, a.follow))
            .collect();
        assert_eq!(
            ends,
            [
                (day("2027-02-28"), Follow::First),
                (day("2027-05-31"), Follow::Reach),
                (day("2027-08-31"), Follow::Reach)
            ]
        );
    }

    #[test]
    fn a_year_end_given_ahead_steps_back_to_the_return_still_due() {
        let mut later = profile();
        later.vat_registered = false;
        later.financial_year_end = Some(day("2027-12-31"));
        let planned = plan(&later, day("2026-03-01"));
        assert_eq!(planned.len(), 1);
        // FY 2025's return is due 2026-09-30, after the day.
        assert_eq!(planned[0].period_end, day("2025-12-31"));
    }
}
