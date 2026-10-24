//! The police report guard of the accident repair (08 §3.3 `AUT.ACL.20`,
//! D-08, BL-AUT-07).
//!
//! Accident damage is repaired only when the police accident report (the
//! repair permit) is attached and accepted. The report is a document of the
//! kind [`crate::seed::POLICE_REPORT_KIND`], a *legal block* the client seeds
//! (`host/src/app/auto_seed.rs`): its `blocks` name four transitions and its
//! override mode is *never*, locked by the platform.
//!
//! | Guarded transition | Judged on |
//! |---|---|
//! | `job_card.approve` | an accident job card (the flag is set); a job card without the flag needs no report |
//! | `insurance_claim.book_survey` | every claim |
//! | `insurance_claim.submit_estimate` | every claim |
//! | `insurance_claim.approve` | every claim |
//!
//! Decisions:
//!
//! - A report on the claim counts for its job card and the other way round: the
//!   [`BlockSubjects`] providers answer with the one subject of the pair that
//!   holds a valid report, or with the record's own subject when neither does
//!   (so the refusal names the record the user works on).
//! - `start_inspection` and `send_estimate` are not guarded: the vehicle may
//!   stay checked in for the inspection (§4.7). A later `approve` of added work
//!   asks again, and passes because the report stays attached.
//! - The guards of the machines cannot reach the database, so the services ask
//!   wl-documents here ([`police_verdict`]) and hand the answer to the run in
//!   its input ([`PoliceVerdict`]); a run that carries no answer is refused when
//!   a report is needed. Without wl-documents nothing is guarded.
//! - There is no override: the kind is a legal block and wl-documents offers no
//!   override request for it.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::{
    BlockEntry, BlockSubjects, DocumentFuture, guard, register_block_subjects, today,
};

use crate::claim::{ClaimStatus, InsuranceClaim};
use crate::job_card::JobCard;
use crate::seed::POLICE_REPORT_KIND;
use crate::texts::english;

/// The guarded transition `approve` of a job card.
pub const GUARDED_JOB_CARD_APPROVE: &str = "job_card.approve";
/// The guarded transition `book_survey` of an insurance claim.
pub const GUARDED_CLAIM_BOOK_SURVEY: &str = "insurance_claim.book_survey";
/// The guarded transition `submit_estimate` of an insurance claim.
pub const GUARDED_CLAIM_SUBMIT_ESTIMATE: &str = "insurance_claim.submit_estimate";
/// The guarded transition `approve` of an insurance claim.
pub const GUARDED_CLAIM_APPROVE: &str = "insurance_claim.approve";

/// What wl-documents answered for the police report, handed to the machine in
/// the run's input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoliceVerdict {
    /// The report is attached and accepted.
    Valid,
    /// It is missing or was archived; the text says so in words.
    Refused(String),
}

/// `Ok` when a run that `needed` the report carries the verdict *valid* (or
/// needed none); else the words of the refusal. The guard `police_report` of
/// the job card and the claim machines.
pub fn police_result(
    needed: bool,
    verdict: &Option<PoliceVerdict>,
) -> std::result::Result<(), String> {
    if !needed {
        return Ok(());
    }
    match verdict {
        Some(PoliceVerdict::Valid) => Ok(()),
        Some(PoliceVerdict::Refused(words)) => Err(words.clone()),
        None => Err(english("auto.police.required", &[("detail", &"")])
            .trim()
            .to_owned()),
    }
}

/// Asks wl-documents whether record `id` may run `guarded` today: the verdict
/// the run carries. `actor` is the user who runs it (none for the system); it
/// matters only for an override, which this kind never has.
///
/// # Errors
///
/// The guard's own failures (an entry whose transition has no provider); a
/// refusal is the [`PoliceVerdict::Refused`] answer, not an error.
pub async fn police_verdict(
    state: &AppState,
    guarded: &str,
    id: i64,
    actor: Option<i64>,
) -> Result<PoliceVerdict> {
    let outcome = guard(state)
        .check_as(state, guarded, id, today(), actor)
        .await?;
    Ok(match outcome {
        Ok(_covered) => PoliceVerdict::Valid,
        Err(refusal) => {
            let detail = refusal.message();
            PoliceVerdict::Refused(
                english("auto.police.required", &[("detail", &detail)])
                    .trim()
                    .to_owned(),
            )
        }
    })
}

/// Of the pair `own` and `other` the subject that holds a valid police report,
/// else `own`.
async fn holder(state: &AppState, own: Subject, other: Option<Subject>) -> Result<Subject> {
    let Some(other) = other else {
        return Ok(own);
    };
    let documents = guard(state);
    let on = today();
    if documents
        .is_valid(&state.db, &own, POLICE_REPORT_KIND, on)
        .await?
        .is_valid()
    {
        return Ok(own);
    }
    if documents
        .is_valid(&state.db, &other, POLICE_REPORT_KIND, on)
        .await?
        .is_valid()
    {
        return Ok(other);
    }
    Ok(own)
}

/// `job_card.approve`: the job card, or its claim, for an accident job card.
struct JobCardApprove;

impl BlockSubjects for JobCardApprove {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            if entry.kind != POLICE_REPORT_KIND {
                return Ok(Vec::new());
            }
            let Some(card) = JobCard::find(&state.db, record_id).await? else {
                return Ok(Vec::new());
            };
            if !card.accident_damage {
                return Ok(Vec::new());
            }
            let claim = InsuranceClaim::of_job_card(&state.db, card.id)
                .await?
                .into_iter()
                .find(|claim| claim.status != ClaimStatus::Voided)
                .map(|claim| claim.subject());
            Ok(vec![holder(state, card.subject(), claim).await?])
        })
    }
}

/// `insurance_claim.*`: the claim, or its job card.
struct ClaimTransition;

impl BlockSubjects for ClaimTransition {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            if entry.kind != POLICE_REPORT_KIND {
                return Ok(Vec::new());
            }
            let Some(claim) = InsuranceClaim::find(&state.db, record_id).await? else {
                return Ok(Vec::new());
            };
            let card = match claim.job_card_id {
                Some(job_id) => JobCard::find(&state.db, job_id)
                    .await?
                    .map(|card| card.subject()),
                None => None,
            };
            Ok(vec![holder(state, claim.subject(), card).await?])
        })
    }
}

/// Registers the providers of the four guarded transitions.
pub(crate) fn register() {
    register_block_subjects(GUARDED_JOB_CARD_APPROVE, JobCardApprove);
    register_block_subjects(GUARDED_CLAIM_BOOK_SURVEY, ClaimTransition);
    register_block_subjects(GUARDED_CLAIM_SUBMIT_ESTIMATE, ClaimTransition);
    register_block_subjects(GUARDED_CLAIM_APPROVE, ClaimTransition);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_that_needs_no_report_passes_whatever_it_carries() {
        assert!(police_result(false, &None).is_ok());
        assert!(police_result(false, &Some(PoliceVerdict::Refused("no".into()))).is_ok());
    }

    #[test]
    fn a_job_that_needs_the_report_passes_only_with_a_valid_answer() {
        assert!(police_result(true, &Some(PoliceVerdict::Valid)).is_ok());
        let none = police_result(true, &None).unwrap_err();
        assert!(none.contains("Police report required"), "{none}");
        let refused = police_result(true, &Some(PoliceVerdict::Refused("missing".into())));
        assert_eq!(refused.unwrap_err(), "missing");
    }

    #[test]
    fn registering_makes_the_four_transitions_guardable() {
        register();
        let guarded = wl_kernel::documents::guarded_transitions();
        for name in [
            GUARDED_JOB_CARD_APPROVE,
            GUARDED_CLAIM_BOOK_SURVEY,
            GUARDED_CLAIM_SUBMIT_ESTIMATE,
            GUARDED_CLAIM_APPROVE,
        ] {
            assert!(guarded.iter().any(|g| g == name), "{name}");
        }
    }
}
