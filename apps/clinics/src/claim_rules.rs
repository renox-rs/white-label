//! The tasks a claim opens for the billing coordinator (05 §7.1 *Claim*,
//! §7.2 `CLN.RUL.05`; BL-CLN-07, BL-CLN-08): the technical rejection of a
//! submission, and a claim that is `denied` or `partially_paid`, also after a
//! recoupment.
//!
//! Both are listeners on the committed transition of the claim: the task is a
//! call into the tickets (a kernel contract), so it never runs inside the
//! transition. A task is a staff task for the holder of the billing
//! coordinator role, deduplicated by its source key, so a listener that runs
//! twice opens it once.
//!
//! - **Rejection.** Key `clinics.claim_rejected:<claim>:<attempt>`; it names
//!   the channel's errors of the attempt and the last day to send the claim
//!   again (the claim's `fix_deadline`: the end of the first-submission
//!   window from the visit, or of the resubmission window from the
//!   remittance, Asia/Dubai, inclusive); it is due at the end of that day.
//! - **Denial (`CLN.RUL.05`).** Key `clinics.claim_denied:<claim>:<answers>`,
//!   one for each remittance or recoupment recorded; it lists the denied
//!   lines with the payer's code and the last day to resubmit: the
//!   resubmission window counted from the date of the advice that denied
//!   them or carried the recoupment.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::role_tasks::{self, RoleTask};

use crate::claim::{Claim, ClaimAttempt};
use crate::claim_machine::REJECT_SUBMISSION;
use crate::permissions::BILLING_COORDINATOR_ROLE;
use crate::texts::english;

/// The source key of the task for the rejection of `attempt` of claim
/// `claim`.
pub fn rejected_key(claim: i64, attempt: i64) -> String {
    format!("clinics.claim_rejected:{claim}:{attempt}")
}

/// The end of `day` in Asia/Dubai, as an instant: when a deadline task is
/// due.
fn end_of(day: NaiveDate) -> Option<DateTime> {
    let local = day.and_hms_opt(23, 59, 0)?;
    Some(BusinessCalendar::default().from_local(local))
}

/// `E104 invalid member ID; E7 missing field`.
fn errors_text(attempt: &ClaimAttempt) -> String {
    attempt
        .errors
        .0
        .iter()
        .map(|e| format!("{} {}", e.code, e.message))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Opens the coordinator's task for the technical rejection of claim `id`:
/// the errors of the attempt and the last day to send it again. Returns
/// whether a task was opened (not when ticketing is off, or it exists).
///
/// # Errors
///
/// The database's and the task creator's errors.
pub async fn on_rejected(state: &AppState, id: i64) -> Result<bool> {
    let claim = Claim::find_or_404(&state.db, id).await?;
    let attempts = ClaimAttempt::of_claim(&state.db, id).await?;
    let Some(attempt) = attempts.iter().find(|a| a.number == claim.attempts) else {
        return Ok(false);
    };
    let deadline = claim.fix_deadline.map_or_else(
        || english("clinics.claim.task.no_deadline", &[]),
        |d| d.to_string(),
    );
    let number = claim.number();
    let title = english("clinics.claim.task.rejected_title", &[("number", &number)]);
    let description = english(
        "clinics.claim.task.rejected_description",
        &[
            ("number", &number),
            ("errors", &errors_text(attempt)),
            ("deadline", &deadline),
        ],
    );
    let mut task = RoleTask::new(
        claim.subject(),
        BILLING_COORDINATOR_ROLE,
        &title,
        &rejected_key(claim.id, attempt.number),
    )
    .description(&description);
    if let Some(due) = claim.fix_deadline.and_then(end_of) {
        task = task.due_at(due);
    }
    Ok(role_tasks::open(state, task).await?.is_some())
}

/// Listens to the claim's transitions: the rejection task and `CLN.RUL.05`.
pub fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Claim>, state: AppState| async move {
        match event.transition.as_str() {
            REJECT_SUBMISSION => on_rejected(&state, event.key).await.map(|_| ()),
            _ => Ok(()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_keys_name_the_claim_and_the_occurrence() {
        assert_eq!(rejected_key(101, 2), "clinics.claim_rejected:101:2");
    }

    #[test]
    fn a_deadline_task_is_due_at_the_end_of_the_day() {
        let day: NaiveDate = "2027-01-09".parse().unwrap();
        let due = end_of(day).expect("a time");
        assert_eq!(wl_kernel::documents::dubai_date(due), day);
        assert_eq!(wl_kernel::documents::dubai_text(due), "2027-01-09 23:59");
    }
}
