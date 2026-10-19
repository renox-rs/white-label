//! Confirming a sanctions match (wl-parties §4 `confirm_match`, §5.4, §5.6
//! step 4, `PTY.SCR.01`, issue #39).
//!
//! A reviewer holding `screening.review` confirms a `possible_match` with a
//! review note ([`confirm_match`]). The Screening machine has two
//! transitions with the same guard and effect, as a write-off has:
//!
//! - `confirm_match`, the reviewer's;
//! - `confirm_match_approved`, which the kernel runs at the final approval.
//!
//! The action `parties.confirm_match` has a default approval policy
//! ([`policy`]): one step by the `compliance` role, who must also hold
//! `screening.review`, with segregation of duties on, so the reviewer cannot
//! approve their own request. The policy is the setting "second reviewer for
//! a confirmed match": the admin may remove it. While the request waits, or
//! when it is rejected or expires, the Screening stays `possible_match` and
//! nothing below runs.
//!
//! The effects run after the commit of whichever transition moved the
//! status, in [`apply_confirmed_match`] (called by the listener of
//! [`register`]): the person and, by the setting, the companies it is a UBO,
//! director or signatory of are blocked with the restricted reason
//! `sanctions` (a party already `blocked` or `archived` only gets a new
//! restricted Block record), the freeze and report tasks are created, the
//! review task of #38 is closed, the compliance holders get an in-app alert
//! and `ScreeningMatchConfirmed` is emitted. Nothing is sent to the party.
//! The function is idempotent: a rerun adds no second record or task, and
//! sends no second alert or event.

use std::collections::BTreeSet;

use renox::auth::permissions::{Scope, users_with_permission_in};
use renox::auth::{Channel, DatabaseMessage, Notification, Recipient};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{
    Outcome, StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{PolicyDefault, Step};
use wl_kernel::{
    ApprovalRequest, PartyId, PartyStatus, ScreeningId, ScreeningMatchConfirmed, ScreeningStatus,
    SubjectMachine,
};

use crate::block::block_by_system_for_screening;
use crate::block_record::{BlockRecord, add_record_without_transition};
use crate::compliance_tasks::{
    ComplianceTask, ComplianceTasks, NamedParty, PartyStanding, SANCTIONS, create_for_screening,
};
use crate::party::Party;
use crate::permissions;
use crate::relationships::related_companies;
use crate::screening::Screening;
use crate::screening_task::ScreeningTask;
use crate::settings::ConfirmedMatchSettings;
use crate::texts;

/// The reviewer's transition.
pub const CONFIRM_MATCH: &str = "confirm_match";
/// The transition the kernel runs at the final approval.
pub const CONFIRM_MATCH_APPROVED: &str = "confirm_match_approved";
/// The approval policies' action key of `confirm_match`.
pub const ACTION: &str = "parties.confirm_match";
/// The subject type of a Screening in the kernel's registry.
pub const SUBJECT_TYPE: &str = "parties.screening";
/// The role that gives the second review.
pub const SECOND_REVIEWER_ROLE: &str = "compliance";
/// The kind of the in-app notice of the opened tasks.
pub const TASKS_NOTICE_KIND: &str = "party-compliance-tasks";

/// What a `confirm_match` run carries to its guard and effect; it is also
/// the approval request's payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmInput {
    /// The reviewer's note on the decision.
    pub note: String,
}

/// The run's input, if it is a [`ConfirmInput`].
fn input(ctx: &TransitionCtx) -> Option<ConfirmInput> {
    ctx.input_as::<ConfirmInput>().ok()
}

/// The `confirm_match` and `confirm_match_approved` transitions, for the
/// Screening machine.
pub(crate) fn transitions() -> [Transition<Screening>; 2] {
    [
        rules(
            Transition::new(CONFIRM_MATCH)
                .label("Confirm the match")
                .tone("danger")
                .permission(permissions::SCREENING_REVIEW),
        ),
        rules(
            Transition::new(CONFIRM_MATCH_APPROVED)
                .label("Confirm the match (approved)")
                .system(),
        ),
    ]
}

/// The state, guard and effect both transitions share.
fn rules(transition: Transition<Screening>) -> Transition<Screening> {
    transition
        .from([ScreeningStatus::PossibleMatch])
        .to(ScreeningStatus::ConfirmedMatch)
        .guard("review_note", |_s: &Screening, ctx| {
            let given = input(ctx).is_some_and(|input| !input.note.trim().is_empty());
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(texts::english("wl_parties.errors.review_note_required"))
                }
            })
        })
        .effect(|screening: &Screening, ctx, tx| {
            let id = screening.id;
            let note = input(ctx).unwrap_or_default().note.trim().to_owned();
            Box::pin(async move {
                // Not the status column, so outside the machine's guard.
                renox::db::sql(format!(
                    "UPDATE {} SET review_note = ?, updated_at = ? WHERE id = ?",
                    Screening::TABLE
                ))
                .bind(note)
                .bind(renox::db::now())
                .bind(id)
                .execute(&mut *tx)
                .await?;
                Ok(())
            })
        })
}

/// The default policy of the action: one step by the `compliance` role, who
/// must hold `screening.review`, with segregation of duties on.
pub fn policy() -> PolicyDefault {
    PolicyDefault::new(ACTION)
        .with_step(Step::one_of([SECOND_REVIEWER_ROLE]).requiring(permissions::SCREENING_REVIEW))
}

/// `user` confirms the match of `screening` with the review `note`. With the
/// second-reviewer policy in place the kernel defers it as an approval
/// request (the outcome is pending, the screening stays `possible_match`),
/// announced after the commit; else the status moves and the effects run.
///
/// # Errors
///
/// `Forbidden` without `screening.review`, `NotFrom` when the screening is
/// not a `possible_match`, `Guard` (422) for a blank note, or when a request
/// for it is already pending. Nothing is written then.
pub async fn confirm_match(
    state: &AppState,
    user: &AuthUser,
    screening: &Screening,
    note: &str,
) -> std::result::Result<Outcome<Screening>, TransitionError> {
    let input = ConfirmInput {
        note: note.trim().to_owned(),
    };
    let outcome = screening
        .transition(CONFIRM_MATCH)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    wl_kernel::approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(outcome)
}

/// What [`apply_confirmed_match`] did.
#[derive(Debug, Clone)]
pub struct Applied {
    /// The freeze and report tasks of the screening.
    pub tasks: ComplianceTasks,
    /// Whether this run created them (so alerted and emitted); `false` for a
    /// rerun.
    pub first_run: bool,
    /// The parties the tasks name, the matched person first.
    pub parties: Vec<NamedParty>,
}

/// An error of the transition as the error of the effects.
fn failure(error: TransitionError) -> renox::Error {
    match error {
        TransitionError::Failed(error) => error,
        other => renox::Error::permanent_message(other.to_string()),
    }
}

/// Blocks `party`, or adds the restricted record, for the confirmed match of
/// `screening`; `None` for a party that is `merged`.
async fn touch(
    state: &AppState,
    screening: &Screening,
    party: &Party,
    note: &str,
    ran_by: Option<i64>,
) -> Result<Option<NamedParty>> {
    let mut current = Party::find(&state.db, party.id)
        .await?
        .ok_or(renox::Error::NotFound)?;
    if current.status == PartyStatus::Active {
        match block_by_system_for_screening(state, &current, screening.id, SANCTIONS, note, ran_by)
            .await
        {
            Ok(()) => {}
            // Blocked by someone else in between: it only gets the record.
            Err(TransitionError::NotFrom(_)) => {}
            Err(error) => return Err(failure(error)),
        }
        current = Party::find(&state.db, party.id)
            .await?
            .ok_or(renox::Error::NotFound)?;
    }
    if !matches!(current.status, PartyStatus::Blocked | PartyStatus::Archived) {
        return Ok(None);
    }
    let mut record = screening_record(state, party.id, screening.id).await?;
    if record.is_none() {
        add_record_without_transition(
            &state.db,
            party.id,
            screening.id,
            SANCTIONS,
            true,
            note,
            ran_by,
        )
        .await?;
        record = screening_record(state, party.id, screening.id).await?;
    }
    let record = record.ok_or(renox::Error::NotFound)?;
    let standing = if current.status == PartyStatus::Archived {
        PartyStanding::Archived
    } else {
        // Ours when no older open record holds the party.
        let oldest = BlockRecord::open_for_party(&state.db, party.id)
            .await?
            .first()
            .map(|r| r.id);
        if oldest == Some(record.id) {
            PartyStanding::Blocked
        } else {
            PartyStanding::BlockedAlready
        }
    };
    Ok(Some(NamedParty {
        party_id: party.id,
        block_record_id: Some(record.id),
        standing,
    }))
}

/// The party's Block record of the screening, if any.
async fn screening_record(
    state: &AppState,
    party_id: i64,
    screening_id: i64,
) -> Result<Option<BlockRecord>> {
    BlockRecord::query()
        .where_(BlockRecord::PARTY_ID.eq(party_id))
        .where_(BlockRecord::SCREENING_ID.eq(screening_id))
        .order_by(BlockRecord::ID)
        .first(&state.db)
        .await
}

/// Runs the effects of the confirmed match of `screening`, which must be
/// `confirmed_match`. `ran_by` is the reviewer who ran `confirm_match` (the
/// approval request's requester), stored as "blocked by".
///
/// The matched person is blocked first, then (setting
/// [`ConfirmedMatchSettings::related_company_block`]) each company it is an
/// active UBO, director or signatory of; a `merged` party is skipped. A party
/// that is `blocked` or `archived` gets a restricted Block record without a
/// transition. The freeze and report tasks name every party touched. The
/// review task of the screened party is closed, and, the first time only,
/// the compliance holders get one in-app alert and `ScreeningMatchConfirmed`
/// is emitted.
///
/// Idempotent: each record and task is keyed by the screening.
///
/// # Errors
///
/// A database error, or a refusal of a block (e.g. the `sanctions` code is
/// not configured).
pub async fn apply_confirmed_match(
    state: &AppState,
    screening: &Screening,
    ran_by: Option<i64>,
) -> Result<Applied> {
    let person = Party::find(&state.db, screening.party_id)
        .await?
        .ok_or(renox::Error::NotFound)?;
    let note = match screening.review_note.as_deref().map(str::trim) {
        Some(note) if !note.is_empty() => note.to_owned(),
        _ => texts::english("wl_parties.confirm.default_note"),
    };
    let mut targets = vec![person.clone()];
    if ConfirmedMatchSettings::of(state).related_company_block {
        for company in
            related_companies(&state.db, person.id, wl_kernel::documents::today()).await?
        {
            if targets.iter().all(|t| t.id != company.id) {
                targets.push(company);
            }
        }
    }
    let mut parties = Vec::new();
    for target in &targets {
        if let Some(named) = touch(state, screening, target, &note, ran_by).await? {
            parties.push(named);
        }
    }
    let first_run = ComplianceTask::for_screening(&state.db, screening.id)
        .await?
        .is_empty();
    let tasks = create_for_screening(state, screening, &parties).await?;
    let mut tx = state.db.begin_immediate().await?;
    ScreeningTask::close_for_party(&mut tx, screening.party_id).await?;
    tx.commit().await?;
    if first_run {
        alert_tasks_opened(state, screening.id).await?;
        state
            .emit(ScreeningMatchConfirmed {
                party: PartyId(screening.party_id),
                screening: ScreeningId(screening.id),
            })
            .await?;
    }
    Ok(Applied {
        tasks,
        first_run,
        parties,
    })
}

/// The in-app bell entry of the opened tasks: no party name, list or reason.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TasksOpenedNotice {
    screening: i64,
}

impl Notification for TasksOpenedNotice {
    fn kind(&self) -> &'static str {
        TASKS_NOTICE_KIND
    }

    fn channels(&self, _to: &Recipient) -> Vec<Channel> {
        vec![Channel::Database]
    }

    fn to_database(&self, _to: &Recipient, state: &AppState) -> Result<Value> {
        let lang = state.current_lang();
        Ok(
            DatabaseMessage::info(texts::t(&lang, "wl_parties.confirm.notice_title"))
                .body(texts::t(&lang, "wl_parties.confirm.notice_body"))
                .with("screening", self.screening)
                .into(),
        )
    }
}

/// Tells every holder of `parties.block_reason.view` (global scope) that the
/// freeze and report tasks are open: one bell entry each, staff only, no
/// e-mail (the possible-match alert already went out). Returns the user ids
/// told, ordered.
async fn alert_tasks_opened(state: &AppState, screening_id: i64) -> Result<Vec<i64>> {
    let holders =
        users_with_permission_in(&state.db, permissions::BLOCK_REASON_VIEW, &Scope::global())
            .await?;
    let mut seen = BTreeSet::new();
    let notice = TasksOpenedNotice {
        screening: screening_id,
    };
    for user in holders.iter().filter(|u| seen.insert(u.id)) {
        state.notify(user, &notice).await?;
    }
    Ok(seen.into_iter().collect())
}

/// The reviewer behind a committed confirmation: the actor of a direct run,
/// else the requester of the approval request that was approved.
async fn ran_by(state: &AppState, event: &Transitioned<Screening>) -> Result<Option<i64>> {
    if event.transition == CONFIRM_MATCH {
        return Ok(event.actor_id);
    }
    let Some(request) = event
        .meta
        .get(wl_kernel::approvals::submit::REQUEST_KEY)
        .and_then(Value::as_i64)
    else {
        return Ok(None);
    };
    Ok(ApprovalRequest::find(&state.db, request)
        .await?
        .map(|r| r.requester_id))
}

/// Registers the Screening with the kernel (action `parties.confirm_match`),
/// the default second-reviewer policy and the after-commit listener.
pub(crate) fn register(app: &mut Registry) {
    SubjectMachine::of::<Screening>(SUBJECT_TYPE)
        .action(CONFIRM_MATCH, ACTION)
        .register();
    policy().register();
    app.listen(
        |event: Transitioned<Screening>, state: AppState| async move {
            if ![CONFIRM_MATCH, CONFIRM_MATCH_APPROVED].contains(&event.transition.as_str()) {
                return Ok(());
            }
            let screening = Screening::find_or_404(&state.db, event.key).await?;
            let by = ran_by(&state, &event).await?;
            apply_confirmed_match(&state, &screening, by)
                .await
                .map(drop)
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_transitions_leave_possible_match_for_confirmed_match() {
        let machine = Screening::machine();
        for (name, system) in [(CONFIRM_MATCH, false), (CONFIRM_MATCH_APPROVED, true)] {
            let transition = machine.find(name).expect("transition");
            assert_eq!(transition.from_states(), ["possible_match"]);
            assert_eq!(transition.to_state(), Some("confirmed_match"));
            assert_eq!(transition.is_system(), system);
            assert_eq!(transition.guard_names(), ["review_note"]);
        }
        assert_eq!(
            wl_kernel::machines::approved(CONFIRM_MATCH),
            CONFIRM_MATCH_APPROVED
        );
    }

    #[renox::test]
    async fn the_guard_needs_a_review_note() {
        let machine = Screening::machine();
        let transition = machine.find(CONFIRM_MATCH_APPROVED).expect("transition");
        let run = |note: &str| {
            let input = ConfirmInput { note: note.into() };
            TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
        };
        let screening = Screening::default();
        assert!(
            transition
                .check_guards(&screening, &run("Same DOB and passport"))
                .await
                .is_ok()
        );
        for blank in ["", "   "] {
            assert!(matches!(
                transition.check_guards(&screening, &run(blank)).await,
                Err(TransitionError::Guard(_))
            ));
        }
        assert!(matches!(
            transition
                .check_guards(&screening, &TransitionCtx::new(None))
                .await,
            Err(TransitionError::Guard(_))
        ));
    }

    #[test]
    fn the_policy_has_one_step_by_compliance_with_the_review_permission() {
        let expected = PolicyDefault::new("parties.confirm_match")
            .with_step(Step::one_of(["compliance"]).requiring("screening.review"));
        assert_eq!(policy(), expected);
        assert_eq!(policy().action(), ACTION);
    }
}
