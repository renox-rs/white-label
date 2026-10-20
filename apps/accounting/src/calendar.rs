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
//! profile change is `ACC.RUL.10` (BL-ACC-15).

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
fn month_end_after(date: NaiveDate, months: u32) -> NaiveDate {
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
        planned.push(PlannedTask {
            task_type: VAT_RETURN,
            title: english("accounting.calendar.vat_return", &[("end", &end)]),
            period_end: end,
            due_date: end + Duration::days(VAT_DUE_DAYS),
            lead_days: VAT_LEAD_DAYS,
            recurrence: match profile.vat_period {
                VatPeriod::Monthly => TaskRecurrence::Monthly,
                _ => TaskRecurrence::Quarterly,
            },
        });
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
        planned.push(PlannedTask {
            task_type: CT_RETURN,
            title: english("accounting.calendar.ct_return", &[("end", &end)]),
            period_end: end,
            due_date: due(end),
            lead_days: CT_LEAD_DAYS,
            recurrence: TaskRecurrence::Yearly,
        });
    }
    planned
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
        let task = NewDeadlineTask::new(
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
        .recurrence(planned.recurrence)
        .period_end(planned.period_end);
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
