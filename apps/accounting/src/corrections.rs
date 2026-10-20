//! The correction of a filed return (04 §3.3 `ACC.ACL.04`, §4.4 step 7;
//! BL-ACC-18).
//!
//! A filed return is locked; `correct` supersedes it with a new `preparing`
//! version that copies its figures and goes through review and client
//! approval again. The route is fixed at `correct`:
//!
//! - **voluntary disclosure**: always for corporate tax and for a VAT error
//!   above [`THRESHOLD`] of tax. A deadline task due [`DISCLOSURE_DAYS`]
//!   business days after the day the error was found tracks it; it needs
//!   evidence to complete, which is the disclosure reference, so it cannot be
//!   completed without one. The reference may be entered at `correct` or
//!   later, on the task.
//! - **next return**: a VAT error of [`THRESHOLD`] or less. The new version
//!   names a later VAT return of the client that is not filed yet, or only
//!   its period; the adjustment attaches to that return ([`attach`], when it
//!   is opened if it was named by period). The new version is marked
//!   *corrected via next return* ([`Return::is_corrected_via_next_return`]),
//!   is never filed itself, and is cancelled as *not required* once the named
//!   return is filed ([`on_filed`]).
//!
//! Decision: the threshold is the `ACC.ACL.04` parameter, fixed at AED
//! 10,000.00; a corporate tax correction is always a voluntary disclosure
//! until an accountant answers A-14 (`blocked:open-item`); an error of
//! exactly the threshold may take either route.

use renox::chrono::{NaiveDate, NaiveTime};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::calendar::calendar;
use wl_kernel::deadlines::{NewDeadlineTask, deadline_tasks};

use crate::engagement::Engagement;
use crate::permissions::ACCOUNTANT_ROLE;
use crate::reasons::NOT_REQUIRED;
use crate::return_machine::AUTO_CANCEL;
use crate::tax_return::{CorrectionRoute, Return, ReturnType};
use crate::texts::english;

/// The largest VAT error, in fils of tax, that may be corrected in the next
/// return (`ACC.ACL.04`: AED 10,000.00).
pub const THRESHOLD: i64 = 1_000_000;
/// Business days between the day the error was found and the due date of the
/// voluntary disclosure.
pub const DISCLOSURE_DAYS: u32 = 20;
/// The task type of a voluntary disclosure.
pub const DISCLOSURE_TASK: &str = "voluntary_disclosure";

/// The routes offered for an error of `error_amount` fils in a return of
/// `kind`: only the voluntary disclosure above [`THRESHOLD`] and for
/// corporate tax, both for a VAT error of [`THRESHOLD`] or less.
pub fn routes_for(kind: ReturnType, error_amount: i64) -> Vec<CorrectionRoute> {
    if kind == ReturnType::Vat && error_amount <= THRESHOLD {
        vec![
            CorrectionRoute::VoluntaryDisclosure,
            CorrectionRoute::NextReturn,
        ]
    } else {
        vec![CorrectionRoute::VoluntaryDisclosure]
    }
}

/// The due date of the disclosure for an error found on `found_on`: the day
/// is day 0, [`DISCLOSURE_DAYS`] business days of the kernel calendar later
/// (weekends and the client's public holidays excluded).
pub fn disclosure_due(state: &AppState, found_on: NaiveDate) -> NaiveDate {
    let calendar = calendar(state);
    let start = calendar.from_local(found_on.and_time(NaiveTime::MIN));
    calendar
        .local(calendar.add_working_days(start, DISCLOSURE_DAYS))
        .date_naive()
}

/// The new version `correct` made of the superseded return `old`, if any.
async fn new_version(state: &AppState, old: i64) -> Result<Option<Return>> {
    Return::query()
        .where_(Return::SUPERSEDES_ID.eq(old))
        .order_by_desc(Return::ID)
        .first(&state.db)
        .await
}

/// After the commit of `correct` on return `old`: the disclosure task for
/// the *voluntary disclosure* route, the attachment of the adjustment to an
/// existing return named by period for the *next return* route.
///
/// # Errors
///
/// The database's and the task service's errors.
pub async fn after_correct(state: &AppState, old: i64) -> Result {
    let Some(new) = new_version(state, old).await? else {
        return Ok(());
    };
    match new.correction_route {
        Some(CorrectionRoute::VoluntaryDisclosure) => open_disclosure(state, &new).await,
        Some(CorrectionRoute::NextReturn) => {
            if new.correction_return_id.is_none() {
                attach_waiting(state, &new).await?;
            }
            Ok(())
        }
        None => Ok(()),
    }
}

/// Opens the disclosure task of `new` and stores its id on it.
async fn open_disclosure(state: &AppState, new: &Return) -> Result {
    if new.disclosure_task_id.is_some() {
        return Ok(());
    }
    let Some(found_on) = new.error_found_on else {
        return Ok(());
    };
    let engagement = Engagement::find_or_404(&state.db, new.engagement_id).await?;
    let due = disclosure_due(state, found_on);
    let title = english(
        "accounting.corrections.disclosure_title",
        &[("number", &new.number()), ("period", &new.period_end)],
    );
    let description = english(
        "accounting.corrections.disclosure_description",
        &[
            ("number", &new.number()),
            ("amount", &crate::money_text(new.error_amount.unwrap_or(0))),
            ("found", &found_on),
            ("days", &DISCLOSURE_DAYS),
            ("link", &format!("/accounting/returns/{}", new.id)),
        ],
    );
    let task = NewDeadlineTask::new(engagement.subject(), DISCLOSURE_TASK, &title, due)
        .description(&description)
        .party(engagement.client())
        .owner_role(ACCOUNTANT_ROLE)
        .evidence_required();
    if let Some(id) = deadline_tasks(state).create(state, task).await? {
        Return::query()
            .where_(Return::ID.eq(new.id))
            .update(&state.db, &[("disclosure_task_id", &Some(id.0))])
            .await?;
    }
    Ok(())
}

/// The return of `new`'s client for the period `new` names, when one exists
/// and is still to be filed.
async fn named_by_period(state: &AppState, new: &Return) -> Result<Option<Return>> {
    let Some(period) = new.correction_period_end else {
        return Ok(None);
    };
    Ok(Return::of_client(&state.db, new.client())
        .await?
        .into_iter()
        .find(|ret| {
            ret.return_type == ReturnType::Vat
                && ret.period_end == period
                && ret.status.is_open()
                && ret.id != new.id
        }))
}

/// Attaches the adjustment of `new` to the return it names by period, if one
/// exists.
async fn attach_waiting(state: &AppState, new: &Return) -> Result {
    if let Some(target) = named_by_period(state, new).await? {
        set_target(state, new.id, target.id).await?;
    }
    Ok(())
}

/// Records that the adjustment of correction `correction` goes into return
/// `target`.
async fn set_target(state: &AppState, correction: i64, target: i64) -> Result {
    Return::query()
        .where_(Return::ID.eq(correction))
        .update(&state.db, &[("correction_return_id", &Some(target))])
        .await?;
    Ok(())
}

/// A VAT return `opened` for a period: the adjustments waiting for a return
/// of that period are attached to it.
///
/// # Errors
///
/// The database's errors.
pub async fn attach(state: &AppState, opened: &Return) -> Result<Vec<i64>> {
    if opened.return_type != ReturnType::Vat {
        return Ok(Vec::new());
    }
    let waiting = Return::query()
        .where_(Return::CORRECTION_PERIOD_END.eq(opened.period_end))
        .get(&state.db)
        .await?;
    let mut attached = Vec::new();
    for correction in waiting {
        if correction.is_corrected_via_next_return()
            && correction.status.is_open()
            && correction.correction_return_id.is_none()
            && correction.client_party_id == opened.client_party_id
            && correction.id != opened.id
        {
            set_target(state, correction.id, opened.id).await?;
            attached.push(correction.id);
        }
    }
    Ok(attached)
}

/// The return `filed` was filed: the corrections made through it are
/// cancelled as *not required*, since their adjustment went into the return.
/// Answers the ids cancelled.
///
/// # Errors
///
/// The database's errors.
pub async fn on_filed(state: &AppState, filed: &Return) -> Result<Vec<i64>> {
    let named = Return::query()
        .where_(Return::CORRECTION_RETURN_ID.eq(filed.id))
        .get(&state.db)
        .await?;
    let by_period = if filed.return_type == ReturnType::Vat {
        Return::query()
            .where_(Return::CORRECTION_PERIOD_END.eq(filed.period_end))
            .get(&state.db)
            .await?
            .into_iter()
            .filter(|c| {
                c.correction_return_id.is_none() && c.client_party_id == filed.client_party_id
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut cancelled = Vec::new();
    for correction in named.into_iter().chain(by_period) {
        if !correction.is_corrected_via_next_return()
            || !correction.status.is_open()
            || correction.id == filed.id
        {
            continue;
        }
        match correction
            .transition(AUTO_CANCEL)
            .by_system()
            .reason(NOT_REQUIRED)
            .run(state)
            .await
        {
            Ok(_) => cancelled.push(correction.id),
            Err(
                TransitionError::NotFrom(_) | TransitionError::Guard(_) | TransitionError::Conflict,
            ) => {}
            Err(other) => return Err(other.into_error()),
        }
    }
    Ok(cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_vat_error_up_to_the_threshold_may_go_in_the_next_return() {
        use CorrectionRoute::{NextReturn, VoluntaryDisclosure};
        assert_eq!(
            routes_for(ReturnType::Vat, 1_250_000),
            [VoluntaryDisclosure]
        );
        assert_eq!(
            routes_for(ReturnType::Vat, 1_000_000),
            [VoluntaryDisclosure, NextReturn]
        );
        assert_eq!(
            routes_for(ReturnType::Vat, 1_000_001),
            [VoluntaryDisclosure]
        );
        assert_eq!(routes_for(ReturnType::Ct, 100), [VoluntaryDisclosure]);
    }
}
