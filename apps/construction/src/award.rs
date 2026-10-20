//! Starting a project from an awarded quotation, and what a blocked client
//! does to it (02 Construction §7.1 *Quotation* `accept`, §7.2
//! "Negative-flow settings" row Project; BL-CON-03, `PTY.ACL.03`).
//!
//! The quotation itself belongs to wl-invoicing (`FIN.QUO.01`) and is not
//! built yet. What the vertical owns is the step the `accept` transition ends
//! with: the project made from the award (event `QuotationAccepted` ->
//! project created). [`accept`] is that step, so the guard of the issue sits
//! here and moves to the quotation's `accept` when the quotation exists.
//!
//! **A blocked client** (block effect `continue`, decided by the MD,
//! [`register_block_effect`]):
//!
//! - no new project is started for it: [`accept`] asks
//!   `BlockGuard::check(client, start)` in the transaction that creates the
//!   project, and a blocked client is refused with the text the viewer may
//!   see (the neutral *on hold* unless the viewer holds
//!   `parties.block_reason.view`); no project row is written;
//! - the running projects go on: attendance, IPAs and variations are not
//!   guarded, and nothing holds or terminates a project by itself (stopping
//!   one is the MD's decision through `terminate`);
//! - money out to the client stops (`MoneyOut`): the set-off of a supplier
//!   bill from the client is wl-payments' `set_off`, which asks the guard and
//!   waits for an exception approved by the guard's approver (compliance);
//!   the IPA invoice stays open for that amount meanwhile. Receipts from the
//!   client are `MoneyIn` and are never refused.
//!
//! Quotations may still be prepared and sent: only starting a project is
//! guarded.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::PartyId;
use wl_kernel::block_effects::{BlockEffect, BlockEffectOverride};
use wl_kernel::block_guard::{self, ActionKind, BlockRefusal, GuardOutcome};

use crate::permissions;
use crate::project::{Project, ProjectStatus};
use crate::texts::english;

/// The agreement kind of wl-parties §5.6 a project is.
pub const BLOCK_KIND: &str = "project";

/// What the award gives: the facts of the accepted quotation.
#[derive(Debug, Clone)]
pub struct Award {
    /// The client or main contractor.
    pub client: PartyId,
    /// The code of the new project, e.g. `TWR-01`.
    pub code: String,
    /// The name of the project.
    pub name: String,
    /// The contract value, in fils.
    pub contract_value: i64,
    /// The award document (the LPO or the subcontract), a document reference.
    pub award_document: String,
    /// The project manager, if chosen.
    pub manager_user_id: Option<i64>,
}

/// What [`accept`] did.
#[derive(Debug)]
pub enum Awarded {
    /// The project was made, in `setup`.
    Started(Project),
    /// The client is blocked: nothing was written.
    Refused(BlockRefusal),
}

/// Why the award cannot start a project, if it cannot: the code, the name and
/// the award document are given, and the value is not negative.
pub fn problem(award: &Award) -> Option<String> {
    if award.award_document.trim().is_empty() {
        return Some(english("construction.award.document_needed", &[]));
    }
    if award.code.trim().is_empty() || award.name.trim().is_empty() {
        return Some(english("construction.award.code_and_name_needed", &[]));
    }
    if award.contract_value < 0 {
        return Some(english("construction.bad_input", &[]));
    }
    None
}

/// `user` accepts the award: the project is made in `setup` for the client.
///
/// # Errors
///
/// `Forbidden` without `projects.award`, `Guard` (422) when [`problem`] says
/// so or the code is taken. A blocked client is not an error: it gives
/// [`Awarded::Refused`] and nothing is written.
pub async fn accept(
    state: &AppState,
    user: &AuthUser,
    award: &Award,
) -> std::result::Result<Awarded, TransitionError> {
    if !user.has_permission_anywhere(permissions::PROJECTS_AWARD) {
        return Err(TransitionError::Forbidden);
    }
    if let Some(message) = problem(award) {
        return Err(TransitionError::Guard(message));
    }
    let mut tx = state.db.begin().await?;
    let taken = Project::query()
        .where_(Project::CODE.eq(award.code.trim()))
        .first(&mut tx)
        .await?;
    if taken.is_some() {
        return Err(TransitionError::Guard(english(
            "construction.award.code_taken",
            &[("code", &award.code.trim())],
        )));
    }
    // The guard and the insert see the same state.
    let outcome = block_guard::guard(state)
        .check_in(&mut tx, Some(user), award.client, ActionKind::Start)
        .await?;
    if let GuardOutcome::Blocked(refusal) = outcome {
        return Ok(Awarded::Refused(refusal));
    }
    let mut project = Project::new(&award.code, &award.name, award.client);
    project.contract_value = award.contract_value;
    project.manager_user_id = award.manager_user_id;
    project.status = ProjectStatus::Setup;
    let reason = english("construction.award.history", &[]);
    project
        .insert_in_state(&mut tx, ProjectStatus::Setup, &reason)
        .await?;
    tx.commit().await?;
    Ok(Awarded::Started(project))
}

/// Whether a new project may be started for `client`: the refusal when it
/// may not. Asked as the system, so a restricted reason never shows.
///
/// # Errors
///
/// A database error.
pub async fn refusal(state: &AppState, client: PartyId) -> Result<Option<BlockRefusal>> {
    let mut tx = state.db.begin().await?;
    let outcome = block_guard::guard(state)
        .check_in(&mut tx, None, client, ActionKind::Start)
        .await?;
    // Only a question: nothing was written.
    drop(tx);
    Ok(match outcome {
        GuardOutcome::Allowed => None,
        GuardOutcome::Blocked(refusal) => Some(refusal),
    })
}

/// Registers what a blocked client does to a project: it **continues**,
/// decided by the MD (the company admin). No review task is created.
pub(crate) fn register_block_effect() {
    BlockEffectOverride::kind(BLOCK_KIND)
        .effect(BlockEffect::Continue)
        .decider(wl_kernel::COMPANY_ADMIN_ROLE)
        .register();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn award() -> Award {
        Award {
            client: PartyId(9),
            code: "TWR-01".into(),
            name: "Tower".into(),
            contract_value: 1_000_000,
            award_document: "LPO-77.pdf".into(),
            manager_user_id: None,
        }
    }

    #[test]
    fn an_award_needs_its_document_a_code_and_a_name() {
        assert_eq!(problem(&award()), None);
        let mut no_document = award();
        no_document.award_document = " ".into();
        assert!(problem(&no_document).is_some_and(|p| p.contains("award document")));
        let mut no_code = award();
        no_code.code = String::new();
        assert!(problem(&no_code).is_some());
    }

    #[test]
    fn a_blocked_client_leaves_its_projects_running() {
        register_block_effect();
        let rule = wl_kernel::block_effects::effect_for(
            BLOCK_KIND,
            &renox::auth::permissions::Scope::global(),
        );
        assert_eq!(rule.effect, BlockEffect::Continue);
        assert_eq!(
            rule.decider_role.as_deref(),
            Some(wl_kernel::COMPANY_ADMIN_ROLE)
        );
    }
}
