//! The customer withdraws an insurance claim, also after the insurer approved
//! it (08 §4.7 row *The customer withdraws the claim after approval*, §7.1
//! *Insurance repair claim* `cancel`, `AUT.ACL.15`, `AUT.RUL.17`; BL-AUT-08).
//!
//! The coordinator cancels the claim with a reason and, from `approved` and
//! `supplementary_pending`, the customer's written withdrawal
//! ([`crate::claims::withdraw`]). What follows the commit is [`follow`], once
//! and safe to run again:
//!
//! 1. every line of the job card is the customer's (`payer`): the insurer is
//!    never invoiced for it again;
//! 2. the insurer is told in writing ([`crate::insurer_mail::WITHDRAWAL_TYPE`];
//!    the coordinator gets a task when the email cannot be sent);
//! 3. the job card follows:
//!    - **no work started** (no labour done or part done, no part issued) in
//!      `checked_in`, `inspecting` or `awaiting_approval` with nothing
//!      approved: the system `decline`s it, which invoices the inspection fee
//!      agreed at check-in to the customer, cancels the booking and asks the
//!      customer to collect the vehicle ([`crate::declines`]);
//!    - **stop** (the customer's choice, and the default for a job in progress
//!      with nothing done): the system `cancel`s it with the reason *claim
//!      withdrawn*, which invoices the work done to the customer and returns
//!      the parts ([`crate::job_stops::cancel_by_system`]);
//!    - **continue privately**: every line of the job (done and remaining)
//!      goes to the customer in a private estimate (`send_private_estimate`),
//!      the job card is `awaiting_approval`, and no labour or part is recorded
//!      until it is signed ([`crate::job_stops::ensure_not_pending`]). Unsigned
//!      after 7 days, [`cancel_unsigned`] runs the system `cancel` with the
//!      reason *claim withdrawn* (`AUT.RUL.17`, [`crate::declines`]).
//!
//! Decisions:
//!
//! - The insurer is **not** invoiced after the withdrawal: a cancelled claim
//!   funds nothing ([`crate::claim::ClaimStatus::funds_job_card`]), so the job
//!   card is billed to the customer like any other.
//! - A supplementary the insurer never answered is cancelled with the claim;
//!   with *continue privately* its added lines are declined (the customer may
//!   ask for them again), the rest goes into the private estimate.
//! - *Continue privately* needs the job card in progress (or waiting for the
//!   supplementary): from `awaiting_parts` or `quality_check` the estimate
//!   cannot be sent, and the coordinator is told so before the claim changes.
//! - The police report still applies to a private repair (BL-AUT-07): the
//!   report on the cancelled claim counts for the job card.

use renox::db::now;
use renox::prelude::*;
use wl_kernel::documents::today;

use crate::approval::EstimateApproval;
use crate::claim::{CONTINUE_PRIVATELY, ClaimStatus, InsuranceClaim};
use crate::estimate::Estimate;
use crate::insurer_mail::{self, WITHDRAWAL_TYPE};
use crate::job_card::{JobCard, JobCardStatus};
use crate::job_card_machine::{
    DECLINE_ADDED_SYSTEM, DECLINE_SYSTEM, JobInput, SEND_PRIVATE_ESTIMATE,
};
use crate::job_cards::run_system;
use crate::job_stops::{self, reason_text};
use crate::labour_line::{LineStatus, WorkStatus};
use crate::stop::CLAIM_WITHDRAWN;
use crate::texts::english;

/// Whether work started on the job card with this estimate: labour done or
/// part done on a line still in the estimate, or a part issued.
pub fn work_started(estimate: &Estimate) -> bool {
    estimate.labour.iter().any(|line| {
        line.approval != LineStatus::Removed
            && (line.work != WorkStatus::NotDone || line.hours_done_milli > 0)
    }) || estimate
        .parts
        .iter()
        .any(|line| line.approval != LineStatus::Removed && line.issued_milli > 0)
}

/// What the cancelled claim of job card `job_card_id` says the customer's
/// written withdrawal is: its document, else the claim's number in words (the
/// withdrawal needed none because it was made before the approval).
///
/// # Errors
///
/// The database's errors.
pub async fn reference_of(db: &Db, job_card_id: i64) -> Result<String> {
    let claims = InsuranceClaim::of_job_card(db, job_card_id).await?;
    let claim = claims
        .into_iter()
        .find(|claim| claim.status == ClaimStatus::Cancelled);
    Ok(match claim {
        Some(claim) => claim.withdrawal_document.clone().unwrap_or_else(|| {
            let number = claim.number();
            english("auto.claims.withdrawal_reference", &[("claim", &number)])
        }),
        None => english("auto.claims.withdrawal_reference", &[("claim", &"")]),
    })
}

/// The system cancels the job card whose private estimate nobody signed: the
/// reason is *claim withdrawn* and the work done is invoiced to the customer
/// (`AUT.RUL.17`, second branch).
///
/// # Errors
///
/// The refusal of the cancel, and the database's errors.
pub async fn cancel_unsigned(state: &AppState, card: &JobCard) -> Result<JobCard> {
    let document = reference_of(&state.db, card.id).await?;
    job_stops::cancel_by_system(state, card.id, CLAIM_WITHDRAWN, &document).await
}

/// The job card follows the withdrawn claim, and the insurer is told in
/// writing. Done once; a step already done is left as it is. `by` is the user
/// who withdrew the claim.
///
/// # Errors
///
/// The database's, the machine's, the invoicer's, the notifier's and the
/// scheduling contract's errors.
pub async fn follow(state: &AppState, claim_id: i64, by: Option<i64>) -> Result {
    let claim = InsuranceClaim::find_or_404(&state.db, claim_id).await?;
    if claim.status != ClaimStatus::Cancelled {
        return Ok(());
    }
    if let Some(job_id) = claim.job_card_id {
        let mut tx = state.db.begin().await?;
        crate::payer::make_private(&mut tx, job_id).await?;
        tx.commit().await?;
    }
    if claim.letter_sent_at.is_none() {
        send_letter(state, &claim).await?;
    }
    let Some(job_id) = claim.job_card_id else {
        return Ok(());
    };
    let card = JobCard::find_or_404(&state.db, job_id).await?;
    if card.status.is_closed() || card.private_estimate {
        return Ok(());
    }
    let estimate = Estimate::load(&state.db, card.id).await?;
    let started = work_started(&estimate);
    let can_decline = matches!(
        card.status,
        JobCardStatus::CheckedIn | JobCardStatus::Inspecting | JobCardStatus::AwaitingApproval
    ) && !card.work_approved();
    if !started && can_decline {
        decline(state, &card, by).await?;
    } else if claim.withdrawal_choice.as_deref() == Some(CONTINUE_PRIVATELY) {
        send_private_estimate(state, &claim, card, &estimate).await?;
    } else {
        let document = reference_of(&state.db, card.id).await?;
        job_stops::cancel_by_system(state, card.id, CLAIM_WITHDRAWN, &document).await?;
    }
    Ok(())
}

/// The system declines the job card: nothing was done or approved. `by` is the
/// coordinator who withdrew the claim: the booking is cancelled for them when
/// the job card has no advisor yet (a vehicle only checked in).
async fn decline(state: &AppState, card: &JobCard, by: Option<i64>) -> Result {
    let round = EstimateApproval::pending_of(&state.db, card.id).await?;
    // The lines still drafted are not work recorded: the estimate was never
    // approved, so the decline stands whatever is drafted.
    let input = JobInput {
        by,
        on: Some(today()),
        approval_id: round.map(|round| round.id),
        code: Some(CLAIM_WITHDRAWN.to_owned()),
        lines_recorded: Some(0),
        ..JobInput::default()
    };
    let reason = reason_text(CLAIM_WITHDRAWN, &None);
    run_system(state, card, DECLINE_SYSTEM, Some(reason.as_str()), input).await?;
    Ok(())
}

/// The customer continues privately: the job card goes `awaiting_approval`
/// with a private estimate of every line.
async fn send_private_estimate(
    state: &AppState,
    claim: &InsuranceClaim,
    card: JobCard,
    estimate: &Estimate,
) -> Result {
    let mut card = card;
    let reason = reason_text(CLAIM_WITHDRAWN, &None);
    if card.status == JobCardStatus::AwaitingApproval {
        // The supplementary the insurer never answered: its added lines are
        // declined and the job goes on without them.
        let round = EstimateApproval::pending_of(&state.db, card.id).await?;
        let input = JobInput {
            by: None,
            on: Some(today()),
            approval_id: round.map(|round| round.id),
            decided_by_name: Some(claim.claim_no.clone()),
            method: Some("recorded".to_owned()),
            ..JobInput::default()
        };
        card = run_system(
            state,
            &card,
            DECLINE_ADDED_SYSTEM,
            Some(reason.as_str()),
            input,
        )
        .await?;
    }
    let priced = estimate.has_live_lines() && estimate.live_totals().net() > 0;
    let input = JobInput {
        on: Some(today()),
        priced: Some(priced),
        ..JobInput::default()
    };
    run_system(state, &card, SEND_PRIVATE_ESTIMATE, None, input).await?;
    Ok(())
}

/// The letter to the insurer: the claim is withdrawn by the customer.
async fn send_letter(state: &AppState, claim: &InsuranceClaim) -> Result {
    let mut data = insurer_mail::letter_data(state, claim).await?;
    let (reason, choice) = {
        let reason = claim.closed_reason.clone().unwrap_or_default();
        let choice = match claim.withdrawal_choice.as_deref() {
            Some(CONTINUE_PRIVATELY) => english("auto.claims.choice.continue_privately", &[]),
            Some(_) => english("auto.claims.choice.stop", &[]),
            None => String::new(),
        };
        (reason, choice)
    };
    if let Some(fields) = data.as_object_mut() {
        fields.insert("reason".to_owned(), reason.into());
        fields.insert("choice".to_owned(), choice.into());
        fields.insert(
            "document".to_owned(),
            claim.withdrawal_document.clone().unwrap_or_default().into(),
        );
    }
    let queued = insurer_mail::email_insurer(state, claim, WITHDRAWAL_TYPE, data).await?;
    if queued {
        InsuranceClaim::query()
            .where_(InsuranceClaim::ID.eq(claim.id))
            .update(
                &state.db,
                &[("letter_sent_at", &Some(now())), ("updated_at", &now())],
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::labour_line::LabourLine;
    use crate::part_line::PartLine;

    fn approved_labour(work: WorkStatus) -> LabourLine {
        LabourLine {
            approval: LineStatus::Approved,
            work,
            ..LabourLine::default()
        }
    }

    #[test]
    fn work_started_when_labour_is_done_or_a_part_was_issued() {
        let none = Estimate::default();
        assert!(!work_started(&none));
        let waiting = Estimate {
            labour: vec![approved_labour(WorkStatus::NotDone)],
            parts: Vec::new(),
        };
        assert!(!work_started(&waiting));
        let done = Estimate {
            labour: vec![approved_labour(WorkStatus::PartDone)],
            parts: Vec::new(),
        };
        assert!(work_started(&done));
        let issued = Estimate {
            labour: Vec::new(),
            parts: vec![PartLine {
                approval: LineStatus::Approved,
                issued_milli: 1_000,
                ..PartLine::default()
            }],
        };
        assert!(work_started(&issued));
        let removed = Estimate {
            labour: vec![LabourLine {
                approval: LineStatus::Removed,
                work: WorkStatus::Done,
                ..LabourLine::default()
            }],
            parts: Vec::new(),
        };
        assert!(!work_started(&removed), "a removed line is not work");
    }
}
