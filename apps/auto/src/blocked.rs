//! A blocked party (08 §7.1 settings *Party blocked*, wl-parties §5.6, `X-18`,
//! BL-AUT-17, `PTY.ACL.03`): each agreement reacts by its kind. Blocking ends
//! nothing by itself; the decider gets a review task (wl-parties) and chooses.
//!
//! | Kind | Effect | Decider | What the review offers |
//! |---|---|---|---|
//! | [`FLEET_KIND`] `fleet_contract` | review | the owner | *suspend* (the contract is suspended with the neutral reason *on hold*), *terminate* (the contract ends with the owner's decision as its approval, see [`crate::fleet_contracts::terminate_by_decision`]), *continue* |
//! | [`JOB_CARD_KIND`] `job_card` | review | the branch manager | *finish safety lines only* (pause), *continue* |
//! | [`CLAIM_KIND`] `insurance_claim` | continue | the insurance coordinator | nothing: a claim of a blocked insurer keeps its state and the insurer's remittances are recorded, because money comes in |
//!
//! Decisions:
//!
//! - **Nothing is decided automatically.** The sources list the open
//!   agreements and carry out the decider's choice only (`decide`); the real
//!   reason of the block never reaches the app (tasks and messages use the
//!   neutral *on hold*).
//! - **A fleet.** Agreements are the contracts in force of the blocked fleet,
//!   one review task for the owner (the contracts are global). *Continue*
//!   changes nothing.
//! - **A customer on a job card.** The review lists the party's open job cards
//!   (not those only waiting for collection). *Finish safety lines only* takes
//!   off every line **not yet started** that the advisor did not mark as
//!   *safety work* with the reason [`PARTY_BLOCKED`]: a labour line with nothing
//!   done, a part line with nothing issued or fitted (a special order already
//!   on a purchase order stays). The lines taken off are not billed and stay
//!   visible on the job card; lines already done or part done stay and are
//!   billed. The job card then goes on with `finish_work`, `pass_qc` and
//!   `deliver`. *End* is not offered here: stopping a job is the advisor's
//!   `cancel`.
//! - **New estimate approvals are refused** while the customer is blocked: the
//!   customer's approval (link, signature or recorded) is refused with the
//!   neutral text; the branch manager's review decides what goes on
//!   ([`ensure_not_blocked`]). Lines the insurer approves on a claim are the
//!   claim's, and the claim continues.
//! - **Lifted.** The lines that were taken off stay off; the advisor may send a
//!   new estimate for them.

use renox::db::now;
use renox::prelude::*;
use wl_kernel::block_effects::{
    AgreementFuture, AgreementKind, AgreementRef, AgreementSource, BlockEffect, Decision,
};
use wl_kernel::parties::{PartyStatus, directory};
use wl_kernel::{PartyId, Subject};

use crate::db::refused;
use crate::estimate::Estimate;
use crate::fleet_contract::{FLEET_CONTRACT_SUBJECT_TYPE, FleetContract, FleetStatus};
use crate::fleet_contracts;
use crate::job_card::{JOB_CARD_SUBJECT_TYPE, JobCard, JobCardStatus};
use crate::labour_line::{LabourLine, LineStatus, WorkStatus};
use crate::part_line::PartLine;
use crate::permissions;
use crate::texts::english;
use crate::vehicle::Vehicle;

/// The agreement kind of a fleet contract.
pub const FLEET_KIND: &str = "fleet_contract";
/// The agreement kind of a job card.
pub const JOB_CARD_KIND: &str = "job_card";
/// The agreement kind of an insurance claim.
pub const CLAIM_KIND: &str = "insurance_claim";
/// The reason of a line taken off by the review of a blocked customer.
pub const PARTY_BLOCKED: &str = "party_blocked";

/// Whether `party` is blocked now.
///
/// # Errors
///
/// The directory's errors.
pub async fn party_blocked(state: &AppState, party: i64) -> Result<bool> {
    Ok(directory(state).status(&state.db, PartyId(party)).await? == Some(PartyStatus::Blocked))
}

/// Refuses a new estimate approval on `card` while its customer is blocked,
/// with the neutral text *on hold* (never the reason of the block).
///
/// # Errors
///
/// A 422 for a blocked customer; the directory's errors.
pub async fn ensure_not_blocked(state: &AppState, card: &JobCard) -> Result {
    if party_blocked(state, card.customer_party_id).await? {
        return Err(refused(english("auto.blocked.problem.approval", &[])));
    }
    Ok(())
}

/// A line of the estimate, by kind and id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineRef {
    /// A labour line.
    Labour(i64),
    /// A part line.
    Part(i64),
}

impl LineRef {
    /// The line written as `labour:12` or `part:5`.
    pub fn parse(text: &str) -> Option<LineRef> {
        let (kind, id) = text.trim().split_once(':')?;
        let id = id.parse::<i64>().ok().filter(|id| *id > 0)?;
        match kind {
            "labour" => Some(LineRef::Labour(id)),
            "part" => Some(LineRef::Part(id)),
            _ => None,
        }
    }

    /// The line as the form writes it.
    pub fn key(self) -> String {
        match self {
            LineRef::Labour(id) => format!("labour:{id}"),
            LineRef::Part(id) => format!("part:{id}"),
        }
    }
}

/// Marks a line of the estimate as *safety work* (or not): needed to make the
/// vehicle safe to drive, so that it continues when the customer is blocked.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 404 for a line that does not exist or
/// is not of job card `card_id`; a 422 for a line that was taken off, declined
/// or whose job card is closed.
pub async fn mark_safety(
    state: &AppState,
    by: &AuthUser,
    card_id: i64,
    line: LineRef,
    safety: bool,
) -> Result<JobCard> {
    if !by.has_permission(permissions::JOB_CARDS_ADVISE) {
        return Err(Error::Forbidden);
    }
    let (of_card, approval) = match line {
        LineRef::Labour(id) => {
            let found = LabourLine::find_or_404(&state.db, id).await?;
            (found.job_card_id, found.approval)
        }
        LineRef::Part(id) => {
            let found = PartLine::find_or_404(&state.db, id).await?;
            (found.job_card_id, found.approval)
        }
    };
    if of_card != card_id {
        return Err(Error::NotFound);
    }
    let card = JobCard::find_or_404(&state.db, card_id).await?;
    if card.status.is_closed() || !approval.is_live() {
        return Err(refused(english("auto.blocked.problem.safety_line", &[])));
    }
    match line {
        LineRef::Labour(id) => {
            LabourLine::query()
                .where_(LabourLine::ID.eq(id))
                .update(
                    &state.db,
                    &[("safety_work", &safety), ("updated_at", &now())],
                )
                .await?;
        }
        LineRef::Part(id) => {
            PartLine::query()
                .where_(PartLine::ID.eq(id))
                .update(
                    &state.db,
                    &[("safety_work", &safety), ("updated_at", &now())],
                )
                .await?;
        }
    }
    JobCard::find_or_404(&state.db, card_id).await
}

/// What a review of a job card took off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Review {
    /// Labour lines taken off.
    pub labour: usize,
    /// Part lines taken off.
    pub parts: usize,
}

/// *Finish safety lines only* (the branch manager's review, BL-AUT-17): takes
/// off job card `id` every line not yet started that is not safety work, with
/// the reason [`PARTY_BLOCKED`]. `user` is the decider. Lines already done or
/// part done, lines marked as safety work and special orders already ordered
/// stay. A job card that is closed has nothing to take off.
///
/// # Errors
///
/// The database's errors.
pub async fn finish_safety_lines_only(state: &AppState, user: i64, id: i64) -> Result<Review> {
    let _ = user;
    let card = JobCard::find_or_404(&state.db, id).await?;
    let mut review = Review::default();
    if card.status.is_closed() {
        return Ok(review);
    }
    let estimate = Estimate::load(&state.db, card.id).await?;
    for line in &estimate.labour {
        let live = matches!(
            line.approval,
            LineStatus::Draft | LineStatus::Sent | LineStatus::Approved
        );
        if live && line.work == WorkStatus::NotDone && !line.safety_work {
            LabourLine::query()
                .where_(LabourLine::ID.eq(line.id))
                .update(
                    &state.db,
                    &[
                        ("approval", &LineStatus::Removed),
                        ("removed_reason", &Some(PARTY_BLOCKED.to_owned())),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            review.labour += 1;
        }
    }
    for line in &estimate.parts {
        let live = matches!(
            line.approval,
            LineStatus::Draft | LineStatus::Sent | LineStatus::Approved
        );
        let ordered = line.special_order && line.purchase_order_id.is_some();
        if live
            && line.issued_milli == 0
            && line.fitted_milli == 0
            && !line.safety_work
            && !ordered
        {
            PartLine::query()
                .where_(PartLine::ID.eq(line.id))
                .update(
                    &state.db,
                    &[
                        ("approval", &LineStatus::Removed),
                        ("removed_reason", &Some(PARTY_BLOCKED.to_owned())),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            review.parts += 1;
        }
    }
    Ok(review)
}

/// The open fleet contracts of a blocked fleet, for the owner's review.
struct FleetAgreements;

impl AgreementSource for FleetAgreements {
    fn open_for<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> AgreementFuture<'a, Vec<AgreementRef>> {
        Box::pin(async move {
            let contracts = FleetContract::query()
                .where_(FleetContract::FLEET_PARTY_ID.eq(party.0))
                .where_(FleetContract::STATUS.is_in([
                    FleetStatus::Active,
                    FleetStatus::Suspended,
                    FleetStatus::RenewalDue,
                ]))
                .order_by(FleetContract::ID)
                .get(&state.db)
                .await?;
            Ok(contracts
                .into_iter()
                .map(|contract| AgreementRef {
                    label: english(
                        "auto.blocked.fleet_label",
                        &[("number", &contract.number()), ("title", &contract.title)],
                    ),
                    subject: contract.subject(),
                    scope: None,
                })
                .collect())
        })
    }

    fn decide<'a>(
        &'a self,
        state: &'a AppState,
        user: i64,
        subject: &'a Subject,
        decision: Decision,
    ) -> AgreementFuture<'a, ()> {
        Box::pin(async move {
            if subject.subject_type != FLEET_CONTRACT_SUBJECT_TYPE {
                return Err(Error::NotFound);
            }
            match decision {
                Decision::Continue => Ok(()),
                Decision::Pause => fleet_contracts::suspend_by_decision(
                    state,
                    user,
                    subject.subject_id,
                )
                .await
                .map(|_| ()),
                Decision::End => fleet_contracts::terminate_by_decision(
                    state,
                    user,
                    subject.subject_id,
                )
                .await
                .map(|_| ()),
            }
        })
    }
}

/// The open job cards of a blocked customer, for the branch manager's review.
struct JobCardAgreements;

impl AgreementSource for JobCardAgreements {
    fn open_for<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> AgreementFuture<'a, Vec<AgreementRef>> {
        Box::pin(async move {
            let cards = JobCard::query()
                .where_(JobCard::CUSTOMER_PARTY_ID.eq(party.0))
                .where_(JobCard::STATUS.is_in([
                    JobCardStatus::CheckedIn,
                    JobCardStatus::Inspecting,
                    JobCardStatus::AwaitingApproval,
                    JobCardStatus::InProgress,
                    JobCardStatus::AwaitingParts,
                    JobCardStatus::QualityCheck,
                ]))
                .order_by(JobCard::ID)
                .get(&state.db)
                .await?;
            let mut refs = Vec::new();
            for card in cards {
                let plate = Vehicle::find(&state.db, card.vehicle_id)
                    .await?
                    .map_or_else(String::new, |vehicle| vehicle.plate());
                let status = english(&format!("auto.jobs.status.{}", card.status.as_str()), &[]);
                refs.push(AgreementRef {
                    label: english(
                        "auto.blocked.card_label",
                        &[
                            ("number", &card.number()),
                            ("plate", &plate),
                            ("status", &status),
                        ],
                    ),
                    subject: card.subject(),
                    scope: None,
                });
            }
            Ok(refs)
        })
    }

    fn decide<'a>(
        &'a self,
        state: &'a AppState,
        user: i64,
        subject: &'a Subject,
        decision: Decision,
    ) -> AgreementFuture<'a, ()> {
        Box::pin(async move {
            if subject.subject_type != JOB_CARD_SUBJECT_TYPE {
                return Err(Error::NotFound);
            }
            match decision {
                Decision::Continue => Ok(()),
                Decision::Pause => finish_safety_lines_only(state, user, subject.subject_id)
                    .await
                    .map(|_| ()),
                Decision::End => Err(refused(english("auto.blocked.problem.end_job", &[]))),
            }
        })
    }
}

/// Registers the agreement kinds: a fleet contract and a job card are reviewed,
/// an insurance claim continues.
pub(crate) fn register() {
    AgreementKind::new(FLEET_KIND)
        .owner("auto")
        .effect(BlockEffect::Review)
        .decider(permissions::OWNER_ROLE)
        .source(FleetAgreements)
        .register();
    AgreementKind::new(JOB_CARD_KIND)
        .owner("auto")
        .effect(BlockEffect::Review)
        .decider(permissions::BRANCH_MANAGER_ROLE)
        .source(JobCardAgreements)
        .register();
    AgreementKind::new(CLAIM_KIND)
        .owner("auto")
        .effect(BlockEffect::Continue)
        .decider(permissions::INSURANCE_COORDINATOR_ROLE)
        .register();
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::auth::permissions::Scope;
    use wl_kernel::block_effects::effect_for;

    #[test]
    fn the_kinds_are_registered_with_their_effect_and_decider() {
        register();
        let global = Scope::global();
        let fleet = effect_for(FLEET_KIND, &global);
        assert_eq!(fleet.effect, BlockEffect::Review);
        assert_eq!(fleet.decider_role.as_deref(), Some("owner"));
        let card = effect_for(JOB_CARD_KIND, &global);
        assert_eq!(card.effect, BlockEffect::Review);
        assert_eq!(card.decider_role.as_deref(), Some("branch_manager"));
        let claim = effect_for(CLAIM_KIND, &global);
        assert_eq!(claim.effect, BlockEffect::Continue);
        assert!(wl_kernel::block_effects::source_for(FLEET_KIND).is_some());
        assert!(wl_kernel::block_effects::source_for(JOB_CARD_KIND).is_some());
    }

    #[test]
    fn a_line_is_written_as_its_kind_and_id() {
        assert_eq!(LineRef::parse("labour:12"), Some(LineRef::Labour(12)));
        assert_eq!(LineRef::parse(" part:5 "), Some(LineRef::Part(5)));
        assert_eq!(LineRef::parse("part:0"), None);
        assert_eq!(LineRef::parse("tyre:3"), None);
        assert_eq!(LineRef::Labour(12).key(), "labour:12");
    }
}
