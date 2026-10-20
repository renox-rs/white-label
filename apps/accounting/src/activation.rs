//! What engagement `activate` checks (04 §7.1 *Engagement*, row `activate`;
//! BL-ACC-01).
//!
//! Every failing check is gathered first and listed in one refusal, each
//! line saying what to do:
//!
//! - the CDD file approved (`ACC.ACL.07`), the engagement letter signed and
//!   the tax profile complete;
//! - the client's KYC documents valid on the activation day (Asia/Dubai):
//!   the document guard `engagement.activate` ([`crate::documents`]), one
//!   line per failing document naming the party, the kind and its expiry; a
//!   one-use override approved by the MLRO (`DOC.ACL.02`, 72 hours) covers
//!   an expired document for the manager who asked;
//! - the latest screening of every screened party of the CDD file (the
//!   client company, its UBOs, directors and signatories) not run on a stale
//!   list, no list stale now, and no open possible match (`PTY.RUL.02`,
//!   [`crate::screening`]); no override;
//! - the client not blocked (`PTY.ACL.03`: no exception for starting an
//!   engagement): "On hold" to users without `parties.block_reason.view`.
//!
//! The screening and block checks run again in the transaction that moves
//! the status, so a list that turns stale or a block that lands meanwhile
//! still refuses.

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::block_guard::{
    ActionKind, BlockRefusal as PartyRefusal, GuardOutcome, guard as block_guard,
};
use wl_kernel::documents::{BlockRefusal, guard as document_guard, guarded_transition_as};
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::{PartyId, Subject};

use crate::cdd::{CddFile, CddStatus, ScreenedRole};
use crate::db::refused;
use crate::documents::ACTIVATE_GUARDED;
use crate::engagement::{Engagement, EngagementStatus};
use crate::engagement_machine::{ACTIVATE, EngagementInput};
use crate::permissions;
use crate::screening::{self, ScreeningCheck, party_label};
use crate::tax_profile::TaxProfile;
use crate::texts::english;

/// The checks on the engagement itself that fail on `on`, one English line
/// each: the CDD file approved, the letter signed, the tax profile complete.
///
/// # Errors
///
/// The database's errors.
pub async fn readiness(db: &Db, engagement: &Engagement, on: NaiveDate) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    let approved = CddFile::current(db, engagement.client())
        .await?
        .is_some_and(|file| file.status == CddStatus::Approved);
    if !approved {
        problems.push(english("accounting.activation.cdd_not_approved", &[]));
    }
    if !engagement.letter_signed {
        problems.push(english("accounting.activation.letter_not_signed", &[]));
    }
    match TaxProfile::current(db, engagement.client(), on).await? {
        None => problems.push(english("accounting.activation.no_tax_profile", &[])),
        Some(profile) => {
            let missing = profile.missing();
            if !missing.is_empty() {
                problems.push(english(
                    "accounting.activation.tax_profile_incomplete",
                    &[("missing", &missing.join(", "))],
                ));
            }
        }
    }
    Ok(problems)
}

/// The lines of a document refusal: the party, then wl-documents' line
/// (kind, expiry, who renews, whether an override is allowed).
async fn document_lines(state: &AppState, refusal: &BlockRefusal) -> Vec<String> {
    let mut lines = Vec::new();
    for failure in &refusal.failures {
        let who = subject_label(state, &failure.subject).await;
        lines.push(english(
            "accounting.activation.document",
            &[("party", &who), ("problem", &failure.message)],
        ));
    }
    lines
}

/// The name of a document subject: the party's name, else the subject.
async fn subject_label(state: &AppState, subject: &Subject) -> String {
    if subject.subject_type == PARTY_SUBJECT_TYPE {
        party_label(state, PartyId(subject.subject_id)).await
    } else {
        subject.to_string()
    }
}

/// What the screening and block checks found, read in a transaction.
struct Gate {
    /// The screening of the screened parties.
    check: ScreeningCheck,
    /// The block refusal, when the client is blocked.
    blocked: Option<PartyRefusal>,
}

impl Gate {
    /// Whether nothing refuses.
    fn passes(&self) -> bool {
        self.check.settled() && self.blocked.is_none()
    }

    /// One line per refusal for `by`. It reads party names through the
    /// app's database, so call it after the transaction is closed.
    async fn lines(&self, state: &AppState, by: &AuthUser) -> Vec<String> {
        let mut lines: Vec<String> = self.check.stale_line().into_iter().collect();
        lines.extend(self.check.match_lines(state, by).await);
        if let Some(refusal) = &self.blocked {
            lines.push(if refusal.is_detailed() {
                refusal.message_en()
            } else {
                english("accounting.activation.on_hold", &[])
            });
        }
        lines
    }
}

/// The screening and block checks of `parties` and `client` for `by`, in
/// `tx`. Only `tx` is read.
async fn gate_in(
    tx: &mut Transaction,
    state: &AppState,
    by: &AuthUser,
    client: PartyId,
    parties: &[(PartyId, ScreenedRole)],
) -> Result<Gate> {
    let check = screening::check_in(&mut *tx, state, parties).await?;
    let outcome = block_guard(state)
        .check_in(&mut *tx, Some(by), client, ActionKind::Start)
        .await?;
    let blocked = match outcome {
        GuardOutcome::Blocked(refusal) => Some(refusal),
        GuardOutcome::Allowed => None,
    };
    Ok(Gate { check, blocked })
}

/// The refusal that lists `problems`: "This engagement cannot be activated:"
/// and one line each.
pub fn refusal(problems: &[String]) -> Error {
    let mut lines = vec![english("accounting.activation.refused", &[])];
    lines.extend(problems.iter().map(|p| format!("- {p}")));
    refused(lines.join("\n"))
}

/// Every check of `activate` that fails for `by` on `on`, one line each.
///
/// # Errors
///
/// The database's and the guards' errors.
pub async fn problems(
    state: &AppState,
    by: &AuthUser,
    engagement: &Engagement,
    on: NaiveDate,
) -> Result<Vec<String>> {
    let mut problems = readiness(&state.db, engagement, on).await?;
    let documents = document_guard(state)
        .check_as(state, ACTIVATE_GUARDED, engagement.id, on, Some(by.id))
        .await?;
    if let Err(refusal) = &documents {
        problems.extend(document_lines(state, refusal).await);
    }
    let parties = crate::cdd_files::screened_parties(&state.db, engagement.client()).await?;
    let mut tx = state.db.begin().await?;
    let gate = gate_in(&mut tx, state, by, engagement.client(), &parties).await;
    tx.rollback().await?;
    problems.extend(gate?.lines(state, by).await);
    Ok(problems)
}

/// `by` activates `engagement` on `on` once every check passes. The
/// override that covers an expired KYC document is used up with it.
///
/// # Errors
///
/// A 422 listing every failing check; `Forbidden` without
/// `engagements.manage`; a 422 unless `onboarding`.
pub(crate) async fn run(
    state: &AppState,
    by: &AuthUser,
    engagement: &Engagement,
    on: NaiveDate,
) -> Result {
    if !by.has_permission(permissions::ENGAGEMENTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    if engagement.status != EngagementStatus::Onboarding {
        // The machine answers "not from this state".
        let _outcome = engagement
            .transition(ACTIVATE)
            .by(by)
            .input(&EngagementInput::on(on))
            .run(state)
            .await
            .map_err(TransitionError::into_error)?;
        return Ok(());
    }
    let found = problems(state, by, engagement, on).await?;
    if !found.is_empty() {
        return Err(refusal(&found));
    }
    let parties = crate::cdd_files::screened_parties(&state.db, engagement.client()).await?;
    let input = EngagementInput::on(on);
    // The document guard claims the override (if any) around the run.
    guarded_transition_as(state, ACTIVATE_GUARDED, engagement.id, on, Some(by.id), || async {
        let mut tx = state.db.begin().await?;
        // Again, in the transaction that moves the status.
        let gate = gate_in(&mut tx, state, by, engagement.client(), &parties).await?;
        if !gate.passes() {
            tx.rollback().await?;
            return Err(refusal(&gate.lines(state, by).await));
        }
        let outcome = engagement
            .transition(ACTIVATE)
            .by(by)
            .input(&input)
            .run_in(&mut tx)
            .await
            .map_err(TransitionError::into_error)?;
        tx.commit().await?;
        outcome.emit(state).await
    })
    .await
}
