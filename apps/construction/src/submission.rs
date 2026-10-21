//! Submitting an IPA, and replacing one that still waits for its certificate
//! (`submit`, `supersede`, `CON.RUL.03`/`CON.ACL.05`, 02 Construction §4.4,
//! §7.1 *IPA*; BL-CON-04).
//!
//! The project manager (`ipas.submit` in the project) runs [`submit`] on a
//! `draft` IPA. The guards:
//!
//! - the progress for the period is recorded: the IPA claims something
//!   ([`progress_problem`]; the BOQ progress that gives the claim comes with
//!   the calculation);
//! - no earlier IPA of the project is still `submitted`, **unless this one
//!   replaces it** (`replaces`). Only the most recent pending IPA can be
//!   replaced, and two IPAs of a project are never `submitted` at once
//!   ([`submit_problem`]).
//!
//! In one transaction the IPA becomes `submitted`, locked (`CON.ACL.05`: a
//! submitted IPA is corrected by a replacement, never edited), and the
//! replaced IPA runs `supersede` (the system's). After the commit the client
//! contacts are told that the earlier IPA was replaced. The PDF with photos
//! and the "claim sent" document come with the documents module.
//!
//! A certificate that arrives later for a superseded IPA is recorded on the
//! IPA that replaced it ([`crate::certificate`]).

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::notify::{
    self, Category, ContactPurpose, ContactRelation, NotificationTypeDef, NotifyChannel,
    NotifyRequest,
};

use crate::ipa::{self, Ipa, IpaStatus, SUBMIT, SUPERSEDE};
use crate::project::Project;
use crate::texts::{english, template};

/// The notification type of the e-mail that tells the client an IPA was
/// replaced.
pub const NOTICE: &str = "construction.ipa_superseded";

/// What a `submit` run carries to its guard: the facts the caller read,
/// because a guard cannot look them up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitInput {
    /// Whether the facts were read; a run without them is refused.
    #[serde(default)]
    pub checked: bool,
    /// The most recent IPA of the project still `submitted`.
    #[serde(default)]
    pub pending: Option<i64>,
    /// Its number in the project.
    #[serde(default)]
    pub pending_number: Option<i64>,
    /// The IPA this one replaces, as the user chose.
    #[serde(default)]
    pub replaces: Option<i64>,
}

/// The input of a `submit` run.
pub(crate) fn submit_input(ctx: &TransitionCtx) -> SubmitInput {
    ctx.input_as::<SubmitInput>().unwrap_or_default()
}

/// Why `ipa` cannot be submitted: it claims nothing yet.
pub fn progress_problem(ipa: &Ipa) -> Option<String> {
    (ipa.claimed_amount <= 0).then(|| english("construction.ipa.progress_needed", &[]))
}

/// Why the IPA cannot be submitted with `input`, if it cannot.
pub fn submit_problem(input: &SubmitInput) -> Option<String> {
    if !input.checked {
        return Some(english("construction.bad_input", &[]));
    }
    match (input.pending, input.replaces) {
        (None, None) => None,
        (None, Some(_)) => Some(english("construction.ipa.nothing_to_replace", &[])),
        (Some(pending), replaces) if replaces == Some(pending) => None,
        (Some(_), Some(_)) => Some(english("construction.ipa.replace_latest_only", &[])),
        (Some(_), None) => {
            let number = input.pending_number.unwrap_or_default();
            Some(english(
                "construction.ipa.replacement_needed",
                &[("number", &number)],
            ))
        }
    }
}

/// The effect of `submit` inside its transaction: the day, and the IPA it
/// replaces.
pub(crate) async fn submit_effects(tx: &mut Transaction, id: i64, input: &SubmitInput) -> Result {
    let on = Some(today());
    let replaces = input.replaces;
    Ipa::query()
        .where_(Ipa::ID.eq(id))
        .update(
            &mut *tx,
            &[("submitted_on", &on), ("replaces_id", &replaces)],
        )
        .await?;
    Ok(())
}

/// What [`submit`] did.
#[derive(Debug)]
pub struct Submitted {
    /// The IPA, `submitted`.
    pub ipa: Ipa,
    /// The IPA it replaced, now `superseded`.
    pub superseded: Option<Ipa>,
}

/// `user` submits IPA `id`; `replaces` is the pending IPA it replaces.
///
/// # Errors
///
/// `Forbidden` without `ipas.submit` in the project, `NotFrom` unless the IPA
/// is a `draft`, `Guard` (422) when no progress is recorded, an earlier IPA
/// is pending and not replaced, or the IPA named is not the most recent
/// pending one. Nothing changes then.
pub async fn submit(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    replaces: Option<i64>,
) -> std::result::Result<Submitted, TransitionError> {
    let ipa = Ipa::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let mut tx = state.db.begin().await?;
    let pending = ipa::in_status_in(&mut tx, ipa.project_id, IpaStatus::Submitted)
        .await?
        .pop();
    let input = SubmitInput {
        checked: true,
        pending: pending.as_ref().map(|found| found.id),
        pending_number: pending.as_ref().map(|found| found.number),
        replaces,
    };
    let submitted = ipa
        .transition(SUBMIT)
        .by(user)
        .input(&input)
        .run_in(&mut tx)
        .await?;
    let mut superseded = None;
    let mut replaced_outcome = None;
    if let Some(old) = pending.filter(|old| Some(old.id) == replaces) {
        replaced_outcome = Some(
            old.transition(SUPERSEDE)
                .by_system()
                .run_in(&mut tx)
                .await?,
        );
        superseded = Some(old.id);
    }
    tx.commit().await?;
    submitted.emit(state).await?;
    if let Some(outcome) = replaced_outcome {
        outcome.emit(state).await?;
    }
    let ipa = Ipa::find_or_404(&state.db, id).await?;
    let superseded = match superseded {
        Some(old) => Some(Ipa::find_or_404(&state.db, old).await?),
        None => None,
    };
    if let Some(old) = &superseded {
        tell_client(state, &ipa, old).await?;
    }
    Ok(Submitted { ipa, superseded })
}

/// Tells the client contacts that IPA `old` was replaced by `replacing`.
async fn tell_client(state: &AppState, replacing: &Ipa, old: &Ipa) -> Result {
    let project = Project::find_or_404(&state.db, replacing.project_id).await?;
    let told = NotifyRequest::new(NOTICE, replacing.subject())
        .subject_label(&project.code)
        .data(renox::serde_json::json!({
            "project": project.name,
            "old": old.number.to_string(),
            "new": replacing.number.to_string(),
        }))
        .to_contact(
            project.client(),
            ContactPurpose {
                relation: ContactRelation::ContactOf,
                position: None,
            },
        );
    notify::notifier(state).notify(state, told).await?;
    Ok(())
}

/// The notification type of the e-mail to the client contacts.
pub fn notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NOTICE, "construction")
        .group("projects")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("construction.ipa.superseded_subject")),
            template("construction.ipa.superseded_body"),
        )
}

/// Registers the notification type.
pub(crate) fn register_types() {
    notify::register_type(notice_type());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(pending: Option<(i64, i64)>, replaces: Option<i64>) -> SubmitInput {
        SubmitInput {
            checked: true,
            pending: pending.map(|(id, _)| id),
            pending_number: pending.map(|(_, number)| number),
            replaces,
        }
    }

    #[test]
    fn with_nothing_pending_the_ipa_submits_on_its_own() {
        assert_eq!(submit_problem(&input(None, None)), None);
    }

    #[test]
    fn a_pending_ipa_must_be_replaced_explicitly() {
        let problem = submit_problem(&input(Some((50, 5)), None));
        assert!(problem.is_some_and(|p| p.contains("IPA 5") && p.contains("replacement")));
        assert_eq!(submit_problem(&input(Some((50, 5)), Some(50))), None);
    }

    #[test]
    fn only_the_most_recent_pending_ipa_can_be_replaced() {
        let problem = submit_problem(&input(Some((50, 5)), Some(40)));
        assert!(problem.is_some_and(|p| p.contains("most recent")));
        let problem = submit_problem(&input(None, Some(40)));
        assert!(problem.is_some_and(|p| p.contains("waiting")));
    }

    #[test]
    fn a_run_without_the_facts_is_refused() {
        assert!(submit_problem(&SubmitInput::default()).is_some());
    }

    #[test]
    fn an_ipa_without_progress_does_not_submit() {
        let mut ipa = Ipa::new(1);
        assert!(progress_problem(&ipa).is_some());
        ipa.claimed_amount = 1;
        assert_eq!(progress_problem(&ipa), None);
    }
}
