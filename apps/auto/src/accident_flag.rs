//! Clearing the accident damage flag of a job card (08 §7.1 *Job card*,
//! `AUT.ACL.20`, BL-AUT-07).
//!
//! The flag makes the police report a condition of `approve`. A flag set by
//! mistake is cleared only with a reason and the approval of the workshop
//! manager (a holder of `job_cards.accident`, never the one who asked), and
//! never while an insurance claim is linked to the job card. The request waits
//! on the job card (`accident_clear_*`) until it is decided; deciding it
//! either clears the flag or drops the request.

use renox::db::now;
use renox::prelude::*;

use crate::claim::{ClaimStatus, InsuranceClaim};
use crate::db::refused;
use crate::job_card::JobCard;
use crate::permissions;
use crate::texts::english;

/// Whether an insurance claim (other than one registered in error) is linked
/// to job card `id`: the flag then stays.
///
/// # Errors
///
/// The database's errors.
pub async fn claim_linked(db: &Db, id: i64) -> Result<bool> {
    Ok(InsuranceClaim::of_job_card(db, id)
        .await?
        .iter()
        .any(|claim| claim.status != ClaimStatus::Voided))
}

/// The advisor asks to clear the accident flag of job card `id`, with `reason`.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 in words for a job card without the
/// flag, a blank reason, a request already waiting, or a job card with a
/// linked insurance claim.
pub async fn request_clear(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
) -> Result<JobCard> {
    if !by.has_permission(permissions::JOB_CARDS_ADVISE) {
        return Err(Error::Forbidden);
    }
    let card = JobCard::find_or_404(&state.db, id).await?;
    if !card.accident_damage {
        return Err(refused(english("auto.police.problem.not_flagged", &[])));
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refused(english("auto.police.problem.clear_reason", &[])));
    }
    if card.accident_clear_at.is_some() {
        return Err(refused(english("auto.police.problem.clear_waiting", &[])));
    }
    if claim_linked(&state.db, card.id).await? {
        return Err(refused(english("auto.police.problem.clear_claim", &[])));
    }
    let stamp = now();
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(
            &state.db,
            &[
                ("accident_clear_reason", &Some(reason.to_owned())),
                ("accident_clear_by", &Some(by.id)),
                ("accident_clear_at", &Some(stamp)),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    JobCard::find_or_404(&state.db, id).await
}

/// The workshop manager approves (`approve` true: the flag is cleared) or
/// refuses (the flag stays) the request on job card `id`.
///
/// # Errors
///
/// A 403 without `job_cards.accident`; a 422 in words for a job card with no
/// request waiting, the one who asked deciding it, or a claim linked since.
pub async fn decide_clear(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    approve: bool,
) -> Result<JobCard> {
    if !by.has_permission(permissions::JOB_CARDS_ACCIDENT) {
        return Err(Error::Forbidden);
    }
    let card = JobCard::find_or_404(&state.db, id).await?;
    let Some(requester) = card
        .accident_clear_by
        .filter(|_| card.accident_clear_at.is_some())
    else {
        return Err(refused(english("auto.police.problem.clear_none", &[])));
    };
    if requester == by.id {
        return Err(refused(english("auto.police.problem.clear_self", &[])));
    }
    if approve && claim_linked(&state.db, card.id).await? {
        return Err(refused(english("auto.police.problem.clear_claim", &[])));
    }
    let none_text: Option<String> = None;
    let none_user: Option<i64> = None;
    let none_at: Option<renox::db::DateTime> = None;
    let stamp = now();
    let flag = if approve { false } else { card.accident_damage };
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(
            &state.db,
            &[
                ("accident_damage", &flag),
                ("accident_clear_reason", &none_text),
                ("accident_clear_by", &none_user),
                ("accident_clear_at", &none_at),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    JobCard::find_or_404(&state.db, id).await
}
