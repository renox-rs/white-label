//! The block guard contract ([ARCHITECTURE.md] §5, wl-parties §6): how a
//! module refuses new agreements with, and money out to, a blocked party.
//!
//! `wl-parties` provides a [`BlockGuard`] with [`provide_block_guard`]. Other
//! modules ask it through [`guard`], which falls back to [`NoBlockGuard`]
//! (everything allowed) when the `parties` module is off. The guard is
//! provided per app with `Registry::provide`, never process-wide.
//!
//! # What modules must guard
//!
//! Every [`ActionKind`] except [`MoneyIn`](ActionKind::MoneyIn) is guarded:
//!
//! - [`Start`](ActionKind::Start): creating a new agreement, such as a
//!   booking or a lease. Creating a record has no transition, so the module
//!   calls [`BlockGuard::check_in`] before the insert.
//! - [`Renew`](ActionKind::Renew) and [`Extend`](ActionKind::Extend): the
//!   transitions that renew or extend an existing agreement.
//! - [`MoneyOut`](ActionKind::MoneyOut): a supplier payment or a refund. The
//!   module guards the step that **releases** the money, not the drafting or
//!   the scheduling. Money that was already sent, and money out that was
//!   scheduled before the party was blocked, is not the guard's concern.
//! - [`MoneyIn`](ActionKind::MoneyIn): receipts are **never refused**. The
//!   kind exists so that receipt code calls the guard like any other code
//!   and still gets [`GuardOutcome::Allowed`].
//!
//! The call runs in the caller's transaction, so the check and the action
//! see the same state.
//!
//! # Refusals and exceptions
//!
//! A [`BlockRefusal`] shows the neutral text "This party is on hold" (code
//! `on_hold`) to everyone without `parties.block_reason.view`, and the real
//! reasons, each with a link to its Block record, to holders. A system viewer
//! (`None`) always gets the neutral text. [`BlockRefusal::into_error`] is a
//! 422 with the neutral text. A blocked action can go ahead only through an
//! approved exception: an approval request with the action
//! [`BLOCK_EXCEPTION_ACTION`], decided by the role
//! [`BlockGuard::exception_step`] names.
//!
//! [ARCHITECTURE.md]: https://github.com/renox-rs/white-label/blob/main/docs/ARCHITECTURE.md

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::db::Transaction;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::parties::{NEUTRAL_BLOCK_REASON, PartyId};
use crate::texts;

/// The approval action of a block exception: a request to let one blocked
/// action go ahead once.
pub const BLOCK_EXCEPTION_ACTION: &str = "parties.block_exception";

/// What the caller is about to do with a party.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Start a new agreement (a booking, a lease).
    Start,
    /// Renew an existing agreement.
    Renew,
    /// Extend an existing agreement.
    Extend,
    /// Release money to the party (a supplier payment, a refund).
    MoneyOut,
    /// Receive money from the party. Never refused.
    MoneyIn,
}

impl ActionKind {
    /// Every kind.
    pub const ALL: [ActionKind; 5] = [
        ActionKind::Start,
        ActionKind::Renew,
        ActionKind::Extend,
        ActionKind::MoneyOut,
        ActionKind::MoneyIn,
    ];

    /// The stored and displayed key: `start`, `renew`, `extend`, `money_out`
    /// or `money_in`.
    pub fn as_str(self) -> &'static str {
        match self {
            ActionKind::Start => "start",
            ActionKind::Renew => "renew",
            ActionKind::Extend => "extend",
            ActionKind::MoneyOut => "money_out",
            ActionKind::MoneyIn => "money_in",
        }
    }

    /// Whether a blocked party is refused this kind: all but
    /// [`MoneyIn`](ActionKind::MoneyIn).
    pub fn is_guarded(self) -> bool {
        self != ActionKind::MoneyIn
    }
}

impl fmt::Display for ActionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One open Block record a holder of `parties.block_reason.view` sees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockReasonLine {
    /// The reason's label, e.g. `Sanctions`.
    pub label: String,
    /// The link to the Block record on the party page.
    pub link: String,
}

/// The answer of [`BlockGuard::check_in`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardOutcome {
    /// The action may go ahead.
    Allowed,
    /// The party is blocked: the action is refused.
    Blocked(BlockRefusal),
}

impl GuardOutcome {
    /// Whether the action may go ahead.
    pub fn is_allowed(&self) -> bool {
        matches!(self, GuardOutcome::Allowed)
    }

    /// `Ok(())` when allowed, else the refusal.
    pub fn into_result(self) -> std::result::Result<(), BlockRefusal> {
        match self {
            GuardOutcome::Allowed => Ok(()),
            GuardOutcome::Blocked(refusal) => Err(refusal),
        }
    }
}

/// A guarded action refused because the party is blocked. It carries what
/// the viewer may see: only the neutral text, or the real reasons with links.
/// `wl-parties` fills the reasons only for holders of
/// `parties.block_reason.view`, so a restricted code never reaches anyone else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockRefusal {
    /// The party.
    pub party: PartyId,
    /// The refused action.
    pub kind: ActionKind,
    /// The open Block records; empty for the neutral refusal.
    pub reasons: Vec<BlockReasonLine>,
    /// The label of the role that approves an exception.
    pub approver: String,
}

impl BlockRefusal {
    /// The neutral refusal: "This party is on hold".
    pub fn neutral(party: PartyId, kind: ActionKind, approver: impl Into<String>) -> Self {
        BlockRefusal {
            party,
            kind,
            reasons: Vec::new(),
            approver: approver.into(),
        }
    }

    /// The refusal for a holder of `parties.block_reason.view`: one line per
    /// open Block record. With no `reasons` it is the neutral one.
    pub fn detailed(
        party: PartyId,
        kind: ActionKind,
        reasons: Vec<BlockReasonLine>,
        approver: impl Into<String>,
    ) -> Self {
        BlockRefusal {
            party,
            kind,
            reasons,
            approver: approver.into(),
        }
    }

    /// The code everyone sees: [`NEUTRAL_BLOCK_REASON`] (`on_hold`).
    pub fn code(&self) -> &'static str {
        NEUTRAL_BLOCK_REASON
    }

    /// Whether the refusal shows the real reasons.
    pub fn is_detailed(&self) -> bool {
        !self.reasons.is_empty()
    }

    /// The links to the Block records; empty for the neutral refusal.
    pub fn record_links(&self) -> Vec<&str> {
        self.reasons.iter().map(|r| r.link.as_str()).collect()
    }

    /// The message in the viewer's language: "This party is on hold", or
    /// one "Blocked: <reason>" line per open Block record.
    pub fn message(&self, lang: &Lang) -> String {
        if self.reasons.is_empty() {
            return texts::t(lang, "wl_kernel.block_guard.on_hold");
        }
        self.reasons
            .iter()
            .map(|r| {
                texts::t_with(
                    lang,
                    "wl_kernel.block_guard.blocked",
                    &[("reason", &r.label)],
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The offer to request an exception, naming the approver role.
    pub fn exception_offer(&self, lang: &Lang) -> String {
        texts::t_with(
            lang,
            "wl_kernel.block_guard.request_exception",
            &[("approver", &self.approver)],
        )
    }

    /// The English message, for places without a request language.
    pub fn message_en(&self) -> String {
        if self.reasons.is_empty() {
            return texts::en("wl_kernel.block_guard.on_hold");
        }
        self.reasons
            .iter()
            .map(|r| texts::en_with("wl_kernel.block_guard.blocked", &[("reason", &r.label)]))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The refusal as a 422 with the **neutral** text, whoever the viewer
    /// is: an error body must never show a restricted reason.
    pub fn into_error(self) -> Error {
        Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            texts::en("wl_kernel.block_guard.on_hold"),
        )
    }
}

impl fmt::Display for BlockRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message_en())
    }
}

impl std::error::Error for BlockRefusal {}

/// Who approves an exception: the role, and the permission an approver must
/// hold ([`BLOCK_EXCEPTION_ACTION`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExceptionStep {
    /// The role key, e.g. `finance_manager` or `compliance`.
    pub role: String,
    /// The permission the approver needs in the subject's scope.
    pub permission: String,
}

impl ExceptionStep {
    /// The step for `role`, requiring [`BLOCK_EXCEPTION_ACTION`].
    pub fn new(role: impl Into<String>) -> Self {
        ExceptionStep {
            role: role.into(),
            permission: BLOCK_EXCEPTION_ACTION.to_owned(),
        }
    }
}

/// The future a [`BlockGuard`] method returns.
pub type BlockFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Refuses guarded actions on a blocked party. `wl-parties` provides it;
/// without that module, [`guard`] gives [`NoBlockGuard`].
pub trait BlockGuard: Send + Sync + 'static {
    /// Checks `kind` for `party` inside the caller's transaction. Only a
    /// `blocked` party is refused; an active, archived, merged or unknown
    /// party is allowed, and [`ActionKind::MoneyIn`] is always allowed.
    /// `viewer` decides how much of the reason the refusal shows; `None`
    /// (the system) always gets the neutral one.
    fn check_in<'a>(
        &'a self,
        tx: &'a mut Transaction,
        viewer: Option<&'a User>,
        party: PartyId,
        kind: ActionKind,
    ) -> BlockFuture<'a, GuardOutcome>;

    /// The step that approves an exception in this app: its role and the
    /// permission it needs. The default is `finance_manager`.
    fn exception_step(&self) -> ExceptionStep {
        ExceptionStep::new("finance_manager")
    }
}

/// The guard when the `parties` module is off: everything is allowed.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBlockGuard;

impl BlockGuard for NoBlockGuard {
    fn check_in<'a>(
        &'a self,
        _tx: &'a mut Transaction,
        _viewer: Option<&'a User>,
        _party: PartyId,
        _kind: ActionKind,
    ) -> BlockFuture<'a, GuardOutcome> {
        Box::pin(async { Ok(GuardOutcome::Allowed) })
    }
}

/// Makes `guard` the app's [`BlockGuard`]; `wl-parties` calls it in
/// `register`.
pub fn provide_block_guard(app: &mut Registry, guard: impl BlockGuard) {
    let guard: Arc<dyn BlockGuard> = Arc::new(guard);
    app.provide(guard);
}

/// The app's [`BlockGuard`], or [`NoBlockGuard`] when none is provided.
pub fn guard(state: &AppState) -> Arc<dyn BlockGuard> {
    match state.provided::<Arc<dyn BlockGuard>>() {
        Some(guard) => Arc::clone(&guard),
        None => Arc::new(NoBlockGuard),
    }
}

#[cfg(test)]
mod tests {
    use renox::testing::TestApp;

    use super::*;

    #[test]
    fn kinds_have_stable_keys_and_only_money_in_is_unguarded() {
        let keys: Vec<_> = ActionKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(keys, ["start", "renew", "extend", "money_out", "money_in"]);
        let guarded: Vec<_> = ActionKind::ALL.iter().filter(|k| k.is_guarded()).collect();
        assert_eq!(guarded.len(), 4);
        assert!(!ActionKind::MoneyIn.is_guarded());
        assert_eq!(ActionKind::MoneyOut.to_string(), "money_out");
        let json = renox::serde_json::to_string(&ActionKind::MoneyOut).unwrap();
        assert_eq!(json, r#""money_out""#);
    }

    #[test]
    fn the_exception_action_and_step_are_fixed() {
        assert_eq!(BLOCK_EXCEPTION_ACTION, "parties.block_exception");
        let step = ExceptionStep::new("compliance");
        assert_eq!(step.role, "compliance");
        assert_eq!(step.permission, "parties.block_exception");
        assert_eq!(NoBlockGuard.exception_step().role, "finance_manager");
    }

    #[test]
    fn a_neutral_refusal_shows_on_hold_only() {
        let refusal = BlockRefusal::neutral(PartyId(7), ActionKind::MoneyOut, "Finance manager");
        assert_eq!(refusal.code(), "on_hold");
        assert!(!refusal.is_detailed());
        assert!(refusal.record_links().is_empty());
        assert_eq!(refusal.message_en(), "This party is on hold");
        assert_eq!(refusal.to_string(), "This party is on hold");
        // Detailed with no reasons is neutral too.
        let none = BlockRefusal::detailed(PartyId(7), ActionKind::Start, vec![], "Compliance");
        assert_eq!(none.message_en(), "This party is on hold");
    }

    #[test]
    fn a_detailed_refusal_lists_reasons_and_links_but_its_error_is_neutral() {
        let refusal = BlockRefusal::detailed(
            PartyId(7),
            ActionKind::Renew,
            vec![BlockReasonLine {
                label: "Sanctions".into(),
                link: "/parties/7/blocks/3".into(),
            }],
            "Compliance",
        );
        assert!(refusal.is_detailed());
        assert_eq!(refusal.message_en(), "Blocked: Sanctions");
        assert_eq!(refusal.record_links(), ["/parties/7/blocks/3"]);
        assert_eq!(refusal.code(), "on_hold");
        match refusal.into_error() {
            Error::Status(status, message) => {
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
                assert_eq!(message, "This party is on hold");
            }
            other => panic!("expected a 422, got {other:?}"),
        }
    }

    #[test]
    fn an_outcome_converts_to_a_result() {
        assert!(GuardOutcome::Allowed.is_allowed());
        assert!(GuardOutcome::Allowed.into_result().is_ok());
        let refusal = BlockRefusal::neutral(PartyId(1), ActionKind::Start, "x");
        let outcome = GuardOutcome::Blocked(refusal.clone());
        assert!(!outcome.is_allowed());
        assert_eq!(outcome.into_result(), Err(refusal));
    }

    #[renox::test]
    async fn without_parties_every_kind_is_allowed() {
        let app = TestApp::new(App::new()).await;
        let guard = guard(app.state());
        let mut tx = app.state().db.begin().await.expect("a transaction");
        for kind in ActionKind::ALL {
            let outcome = guard
                .check_in(&mut tx, None, PartyId(99), kind)
                .await
                .expect("no failure");
            assert_eq!(outcome, GuardOutcome::Allowed, "{kind}");
        }
        assert_eq!(guard.exception_step().role, "finance_manager");
    }
}
