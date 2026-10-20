//! Settling the final account of a terminated project
//! (`settle_final_account`, 02 Construction §4.8 steps 6 and 7, §7.1
//! *Project*; BL-CON-02). A terminated project is not an end: it closes when
//! its final account is agreed and invoiced.
//!
//! The project manager (`projects.manage`) runs [`settle_final_account`]. The
//! guards:
//!
//! - the project has a final account IPA ([`crate::ipa`]);
//! - it is **certified**: the client's certificate is recorded, or a court or
//!   an arbitrator decided the account and the decision is recorded as the
//!   IPA's certificate with the judgment attached (`certified`);
//! - it is **invoiced**, or its balance is written off: the IPA is `invoiced`
//!   (a write-off of the balance is the invoice's own state `written_off`,
//!   `FIN.ACL.15`, the MD's approval, and the IPA stays `invoiced`), or the
//!   invoice the IPA names is `written_off` in wl-invoicing.
//!
//! Open retention parts, open guarantees and a stock count still open are
//! **warnings**, not guards: they keep their own lifecycles after `closed`.
//! [`closing_warnings`] lists them for the closing screen and the settled
//! project answers with them ([`Settled`]).
//!
//! The client portal turning read-only (`POR.ACL.01`) is Later in 02
//! (BL-CON-19): the effect here stamps `closed_at`, the day the 90 days of
//! read-only access start.

use renox::db::Transaction;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::inventory::scope_stock;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};

use crate::guarantee;
use crate::ipa::{self, Ipa, IpaStatus};
use crate::permissions;
use crate::project::{Project, SETTLE_FINAL_ACCOUNT};
use crate::retention;
use crate::texts::english;

/// What a `settle_final_account` run carries to its guard: the facts the
/// caller read, because a guard cannot look them up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettleInput {
    /// Whether the facts were read; a run without them is refused.
    #[serde(default)]
    pub checked: bool,
    /// What stops the account from being settled, if anything.
    #[serde(default)]
    pub problem: Option<String>,
}

/// The input of a `settle_final_account` run.
pub(crate) fn settle_input(ctx: &TransitionCtx) -> SettleInput {
    ctx.input_as::<SettleInput>().unwrap_or_default()
}

/// Why the account cannot be settled with `input`, if it cannot.
pub fn settle_problem(input: &SettleInput) -> Option<String> {
    if !input.checked {
        return Some(english("construction.bad_input", &[]));
    }
    input.problem.clone()
}

/// The effect of `settle_final_account` inside its transaction.
pub(crate) async fn settle_effects(tx: &mut Transaction, id: i64) -> Result {
    let at = Some(renox::db::now());
    Project::query()
        .where_(Project::ID.eq(id))
        .update(&mut *tx, &[("closed_at", &at)])
        .await?;
    Ok(())
}

/// What stops the final account `ipa` of a project from being settled: its
/// certificate, then its invoice or the write-off of its balance. `None`
/// when nothing does. `written_off` says whether the invoice the IPA names
/// is `written_off`.
pub fn final_account_problem(ipa: Option<&Ipa>, written_off: bool) -> Option<String> {
    let Some(ipa) = ipa else {
        return Some(english("construction.settle.no_final_account", &[]));
    };
    if !ipa.is_certified() && !written_off {
        return Some(english("construction.settle.not_certified", &[]));
    }
    if ipa.status != IpaStatus::Invoiced && !written_off {
        return Some(english("construction.settle.not_invoiced", &[]));
    }
    None
}

/// Whether the invoice `ipa` names is `written_off` in wl-invoicing.
async fn invoice_written_off(state: &AppState, ipa: &Ipa) -> Result<bool> {
    let Some(invoice) = ipa.invoice_id else {
        return Ok(false);
    };
    Ok(invoicer(state)
        .find(&state.db, InvoiceId(invoice))
        .await?
        .is_some_and(|found| found.status == InvoiceStatus::WrittenOff))
}

/// What is still open on a project that is being closed, as the closing
/// screen warns about it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ClosingWarnings {
    /// Retention parts not finished (`held`, `due`, `invoiced`).
    pub retention_parts: usize,
    /// Guarantees not finished (`active`, `called`).
    pub guarantees: usize,
    /// Stock counts of the project's stores not finished.
    pub stock_counts: usize,
}

impl ClosingWarnings {
    /// Whether nothing is open.
    pub fn is_empty(&self) -> bool {
        self.retention_parts == 0 && self.guarantees == 0 && self.stock_counts == 0
    }

    /// The warnings as texts, in the order of the closing screen.
    pub fn texts(&self) -> Vec<String> {
        let mut texts = Vec::new();
        if self.retention_parts > 0 {
            texts.push(english("construction.settle.warn_retention", &[]));
        }
        if self.guarantees > 0 {
            texts.push(english("construction.settle.warn_guarantees", &[]));
        }
        if self.stock_counts > 0 {
            texts.push(english("construction.settle.warn_stock_count", &[]));
        }
        texts
    }
}

/// The warnings of the closing screen of project `id`.
///
/// # Errors
///
/// A database error.
pub async fn closing_warnings(state: &AppState, id: i64) -> Result<ClosingWarnings> {
    let project = Project::find_or_404(&state.db, id).await?;
    Ok(ClosingWarnings {
        retention_parts: retention::open_of(&state.db, id).await?.len(),
        guarantees: guarantee::open_of(&state.db, id).await?.len(),
        stock_counts: scope_stock(state)
            .open_counts(state, &project.subject())
            .await?
            .len(),
    })
}

/// A settled project and the warnings it closed with.
#[derive(Debug)]
pub struct Settled {
    /// The project, `closed`.
    pub project: Project,
    /// What was still open.
    pub warnings: ClosingWarnings,
}

/// `user` settles the final account of project `id`.
///
/// # Errors
///
/// `Forbidden` without `projects.manage` in the project, `NotFrom` unless the
/// project is `terminated`, `Guard` (422) when the final account is missing,
/// not certified, or neither invoiced nor written off. Nothing changes then.
pub async fn settle_final_account(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Settled, TransitionError> {
    let project = Project::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if !user.has_permission_in(permissions::PROJECTS_MANAGE, &project.scope()) {
        return Err(TransitionError::Forbidden);
    }
    let found = ipa::final_account_of(&state.db, id).await?;
    let written_off = match &found {
        Some(account) => invoice_written_off(state, account).await?,
        None => false,
    };
    let input = SettleInput {
        checked: true,
        problem: final_account_problem(found.as_ref(), written_off),
    };
    let _outcome = project
        .transition(SETTLE_FINAL_ACCOUNT)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    // The warnings are read as committed: they keep their own lifecycles.
    let warnings = closing_warnings(state, id).await?;
    let project = Project::find_or_404(&state.db, id).await?;
    let entry = renox::audit::Entry::new("construction.final_account_settled")
        .user(user.id)
        .subject(Project::TABLE, id)
        .data(json!({
            "retention_parts": warnings.retention_parts,
            "guarantees": warnings.guarantees,
            "stock_counts": warnings.stock_counts,
        }));
    renox::audit::record(&state.db, entry).await?;
    Ok(Settled { project, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(status: IpaStatus) -> Ipa {
        Ipa {
            status,
            ..Ipa::new(1)
        }
    }

    #[test]
    fn an_invoiced_final_account_settles() {
        assert_eq!(
            final_account_problem(Some(&account(IpaStatus::Invoiced)), false),
            None
        );
    }

    #[test]
    fn a_certified_final_account_that_is_not_invoiced_does_not() {
        let problem = final_account_problem(Some(&account(IpaStatus::Certified)), false);
        assert!(problem.is_some_and(|p| p.contains("not invoiced")));
    }

    #[test]
    fn a_final_account_without_a_certificate_does_not() {
        for status in [IpaStatus::Draft, IpaStatus::Submitted] {
            let problem = final_account_problem(Some(&account(status)), false);
            assert!(
                problem.is_some_and(|p| p.contains("not certified")),
                "{status:?}"
            );
        }
        assert!(final_account_problem(None, false).is_some_and(|p| p.contains("no final account")));
    }

    #[test]
    fn a_written_off_invoice_lets_the_account_settle() {
        assert_eq!(
            final_account_problem(Some(&account(IpaStatus::Certified)), true),
            None
        );
    }

    #[test]
    fn a_run_without_the_facts_is_refused() {
        assert!(settle_problem(&SettleInput::default()).is_some());
        let checked = SettleInput {
            checked: true,
            problem: None,
        };
        assert_eq!(settle_problem(&checked), None);
        let blocked = SettleInput {
            checked: true,
            problem: Some("no".into()),
        };
        assert_eq!(settle_problem(&blocked), Some("no".into()));
    }

    #[test]
    fn the_warnings_are_listed_in_the_order_of_the_screen() {
        let warnings = ClosingWarnings {
            retention_parts: 2,
            guarantees: 0,
            stock_counts: 1,
        };
        assert!(!warnings.is_empty());
        assert_eq!(warnings.texts().len(), 2);
        assert!(ClosingWarnings::default().is_empty());
        assert!(ClosingWarnings::default().texts().is_empty());
    }
}
