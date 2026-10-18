//! The Document state machine (wl-documents §4.1).
//!
//! ```text
//! [*] --submit--> pending_review --accept--> accepted --supersede--> superseded
//! [*] --record--> accepted         |                  --archive---> archived
//!                 pending_review --reject--> rejected --archive---> archived
//!                 rejected --resubmit--> pending_review
//! ```
//!
//! `submit` and `record` create the document: the Renox engine has no
//! transition without a `from` state, so [`crate::Documents::attach`] and
//! [`crate::Documents::record`] insert it in `pending_review` or `accepted`
//! (the machine's [`initial`](Machine::initial) states) and write the
//! history row under the transition's name ([`SUBMIT`], [`RECORD`]). The
//! other five are transitions of this machine. The machine has no app
//! state, so the services gather what a guard needs into the run's input
//! (the kind's number and expiry rules, the new version, the replacing
//! document); a run without it is refused.
//!
//! The `DOC.ACL.03` rules (no `record` for kinds with `blocks`, the
//! reviewer is not the uploader) come with #51; the `PLT.ACL.07` archive
//! guard with the modules whose evidence it protects.

use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::documents::{Document, DocumentStatus};
use crate::permissions;

/// The creation of a document waiting for review.
pub const SUBMIT: &str = "submit";
/// A reviewer accepts a pending document.
pub const ACCEPT: &str = "accept";
/// A reviewer rejects a pending document, with a reason.
pub const REJECT: &str = "reject";
/// The uploader sends a new version of a rejected document.
pub const RESUBMIT: &str = "resubmit";
/// The creation of a document accepted at once (kinds without review).
pub const RECORD: &str = "record";
/// The system replaces an accepted document by a newer one.
pub const SUPERSEDE: &str = "supersede";
/// A manager archives an accepted or rejected document.
pub const ARCHIVE: &str = "archive";
/// The owner role accepts a pending SOP version (#52).
pub const ACCEPT_SOP: &str = "accept_sop";
/// The owner role rejects a pending SOP version, with a reason (#52).
pub const REJECT_SOP: &str = "reject_sop";
/// An SOP manager archives (withdraws) an SOP version (#52).
pub const ARCHIVE_SOP: &str = "archive_sop";

/// What `accept` (and `record`) check: the fields the kind needs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AcceptInput {
    /// The kind has a number (e.g. passport no.).
    pub needs_number: bool,
    /// The kind has an expiry date.
    pub needs_expiry: bool,
    /// The kind is an SOP kind: only `accept_sop` accepts it.
    #[serde(default)]
    pub sop_kind: bool,
}

/// What `reject` and `archive` carry: whether the document is of an SOP
/// kind, which only `reject_sop` and `archive_sop` handle. A run without
/// it is not refused (the services always give it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KindInput {
    /// The document is of an SOP kind.
    pub sop_kind: bool,
}

/// What the SOP transitions carry (#52).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SopRunInput {
    /// The document is of the SOP version kind.
    pub sop_kind: bool,
    /// The actor holds the SOP's owner role.
    pub owner_role: bool,
    /// The kind has a review date (expiry) to fill.
    pub needs_expiry: bool,
}

impl AcceptInput {
    /// Why `document` cannot be accepted yet, if it cannot.
    pub fn missing(self, document: &Document) -> Option<&'static str> {
        if self.needs_number
            && document
                .number
                .as_deref()
                .is_none_or(|n| n.trim().is_empty())
        {
            Some("The document needs its number before it is accepted.")
        } else if self.needs_expiry && document.expiry_date.is_none() {
            Some("The document needs its expiry date before it is accepted.")
        } else {
            None
        }
    }
}

/// What `resubmit` carries: the version just attached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ResubmitInput {
    /// Its number.
    pub version: i64,
}

/// What `supersede` carries: the document that replaces this one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SupersedeInput {
    /// The newer accepted document.
    pub replaced_by: i64,
}

/// The run's input as a `T`, if it is one.
fn input<T: for<'de> Deserialize<'de>>(ctx: &TransitionCtx) -> Option<T> {
    ctx.input_as::<T>().ok()
}

/// The actor's user id, `None` for the system.
fn actor_id(ctx: &TransitionCtx) -> Option<i64> {
    ctx.actor().map(|user| user.id)
}

impl StateMachine for Document {
    type State = DocumentStatus;
    const COLUMN: renox::db::Col<Document, DocumentStatus> = Document::STATUS;

    fn machine() -> Machine<Self> {
        use DocumentStatus::{Accepted, Archived, PendingReview, Rejected, Superseded};
        Machine::new()
            // `submit` and `record` (see the module docs).
            .initial([PendingReview, Accepted])
            .state(PendingReview)
            .tone("warning")
            .state(Accepted)
            .tone("success")
            .state(Rejected)
            .tone("error")
            .state(Superseded)
            .tone("neutral")
            .terminal()
            .state(Archived)
            .tone("neutral")
            .terminal()
            .transition(accept())
            .transition(reject())
            .transition(resubmit())
            .transition(supersede())
            .transition(archive())
            .transition(accept_sop())
            .transition(reject_sop())
            .transition(archive_sop())
    }
}

/// `accept`: pending_review → accepted, by `documents.review`, once the
/// number and expiry date the kind has are filled. Stamps the review; the
/// service supersedes the previous accepted document in the same
/// transaction.
fn accept() -> Transition<Document> {
    accepting(ACCEPT, permissions::REVIEW)
        .guard("not_sop", |_: &Document, ctx| {
            let sop = input::<AcceptInput>(ctx).is_some_and(|i| i.sop_kind);
            Box::pin(async move { not_sop(sop) })
        })
        .guard("fields_filled", |document: &Document, ctx| {
            let missing = input::<AcceptInput>(ctx).map(|i| i.missing(document));
            Box::pin(async move {
                match missing {
                    Some(None) => Ok(()),
                    Some(Some(message)) => Err(message.to_owned()),
                    None => Err("The document's kind was not checked.".to_owned()),
                }
            })
        })
}

/// The refusal of a generic transition on a document of an SOP kind.
fn not_sop(sop: bool) -> std::result::Result<(), String> {
    if sop {
        Err("An SOP version is handled through the SOP library.".to_owned())
    } else {
        Ok(())
    }
}

/// The refusal of an SOP transition on any other document.
fn is_sop(sop: bool) -> std::result::Result<(), String> {
    if sop {
        Ok(())
    } else {
        Err("Only an SOP version is handled through the SOP library.".to_owned())
    }
}

/// `accept_sop`: as `accept`, by `sops.manage` holding the SOP's owner role,
/// for a document of the SOP kind with its review date filled.
fn accept_sop() -> Transition<Document> {
    accepting(ACCEPT_SOP, permissions::SOPS_MANAGE)
        .guard("sop_kind", |_: &Document, ctx| {
            let sop = input::<SopRunInput>(ctx).is_some_and(|i| i.sop_kind);
            Box::pin(async move { is_sop(sop) })
        })
        .guard("owner_role", |_: &Document, ctx| {
            let owner = input::<SopRunInput>(ctx).is_some_and(|i| i.owner_role);
            Box::pin(async move { holds_owner_role(owner) })
        })
        .guard("review_date", |document: &Document, ctx| {
            let needs = input::<SopRunInput>(ctx).is_some_and(|i| i.needs_expiry);
            let missing = needs && document.expiry_date.is_none();
            Box::pin(async move {
                if missing {
                    Err("The SOP version needs its review date before it is accepted.".to_owned())
                } else {
                    Ok(())
                }
            })
        })
}

/// Only a holder of the SOP's owner role.
fn holds_owner_role(owner: bool) -> std::result::Result<(), String> {
    if owner {
        Ok(())
    } else {
        Err("Only a holder of the SOP's owner role can do this.".to_owned())
    }
}

/// pending_review → accepted by `permission`; stamps the review.
fn accepting(name: &'static str, permission: &'static str) -> Transition<Document> {
    Transition::new(name)
        .label("Accept")
        .tone("success")
        .permission(permission)
        .from([DocumentStatus::PendingReview])
        .to(DocumentStatus::Accepted)
        .effect(|document: &Document, ctx, tx| {
            let id = document.id;
            let by = actor_id(ctx);
            Box::pin(async move {
                let at = now();
                // Not the status column, so outside the machine's guard.
                Document::query()
                    .where_(Document::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("accepted_at", &at),
                            ("reviewed_at", &at),
                            ("reviewed_by", &by),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `reject`: pending_review → rejected, by `documents.review`, with a
/// reason, which the document keeps for the uploader.
fn reject() -> Transition<Document> {
    rejecting(REJECT, permissions::REVIEW).guard("not_sop", |_: &Document, ctx| {
        let sop = input::<KindInput>(ctx).is_some_and(|i| i.sop_kind);
        Box::pin(async move { not_sop(sop) })
    })
}

/// `reject_sop`: as `reject`, by `sops.manage` holding the SOP's owner role.
fn reject_sop() -> Transition<Document> {
    rejecting(REJECT_SOP, permissions::SOPS_MANAGE)
        .guard("sop_kind", |_: &Document, ctx| {
            let sop = input::<SopRunInput>(ctx).is_some_and(|i| i.sop_kind);
            Box::pin(async move { is_sop(sop) })
        })
        .guard("owner_role", |_: &Document, ctx| {
            let owner = input::<SopRunInput>(ctx).is_some_and(|i| i.owner_role);
            Box::pin(async move { holds_owner_role(owner) })
        })
}

/// pending_review → rejected by `permission`, with a reason.
fn rejecting(name: &'static str, permission: &'static str) -> Transition<Document> {
    Transition::new(name)
        .label("Reject")
        .tone("error")
        .permission(permission)
        .from([DocumentStatus::PendingReview])
        .to(DocumentStatus::Rejected)
        .reason_required()
        .effect(|document: &Document, ctx, tx| {
            let id = document.id;
            let by = actor_id(ctx);
            let reason = ctx.reason().map(|r| r.trim().to_owned());
            Box::pin(async move {
                Document::query()
                    .where_(Document::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("rejection_reason", &reason),
                            ("reviewed_at", &now()),
                            ("reviewed_by", &by),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `resubmit`: rejected → pending_review, by the uploader, with the new
/// version attached in the same transaction.
fn resubmit() -> Transition<Document> {
    Transition::new(RESUBMIT)
        .label("Resubmit")
        .tone("primary")
        .from([DocumentStatus::Rejected])
        .to(DocumentStatus::PendingReview)
        .guard("new_version", |document: &Document, ctx| {
            let fresh = input::<ResubmitInput>(ctx)
                .is_some_and(|input| input.version > document.current_version);
            Box::pin(async move {
                if fresh {
                    Ok(())
                } else {
                    Err("Attach a new version of the file to resubmit the document.".to_owned())
                }
            })
        })
        .effect(|document: &Document, ctx, tx| {
            let id = document.id;
            let version = input::<ResubmitInput>(ctx).unwrap_or_default().version;
            Box::pin(async move {
                let none: Option<String> = None;
                let no_review: Option<i64> = None;
                Document::query()
                    .where_(Document::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("current_version", &version),
                            ("submitted_at", &now()),
                            ("rejection_reason", &none),
                            ("reviewed_by", &no_review),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `supersede`: accepted → superseded, by the system, when a newer
/// document of the same kind and subject was accepted.
fn supersede() -> Transition<Document> {
    Transition::new(SUPERSEDE)
        .label("Supersede")
        .from([DocumentStatus::Accepted])
        .to(DocumentStatus::Superseded)
        .system()
        .guard("replaced", |document: &Document, ctx| {
            let replaced = input::<SupersedeInput>(ctx)
                .is_some_and(|input| input.replaced_by > 0 && input.replaced_by != document.id);
            Box::pin(async move {
                if replaced {
                    Ok(())
                } else {
                    Err("Only a newer accepted document supersedes this one.".to_owned())
                }
            })
        })
        .effect(|document: &Document, ctx, tx| {
            let id = document.id;
            let by = input::<SupersedeInput>(ctx).unwrap_or_default().replaced_by;
            Box::pin(async move {
                Document::query()
                    .where_(Document::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[("superseded_by", &by), ("superseded_at", &now())],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `archive`: accepted or rejected → archived, by `documents.manage`.
fn archive() -> Transition<Document> {
    archiving(ARCHIVE, permissions::MANAGE).guard("not_sop", |_: &Document, ctx| {
        let sop = input::<KindInput>(ctx).is_some_and(|i| i.sop_kind);
        Box::pin(async move { not_sop(sop) })
    })
}

/// `archive_sop`: as `archive`, by `sops.manage` (the owner role is not
/// needed), for a document of the SOP kind.
fn archive_sop() -> Transition<Document> {
    archiving(ARCHIVE_SOP, permissions::SOPS_MANAGE).guard("sop_kind", |_: &Document, ctx| {
        let sop = input::<SopRunInput>(ctx).is_some_and(|i| i.sop_kind);
        Box::pin(async move { is_sop(sop) })
    })
}

/// accepted or rejected → archived by `permission`.
fn archiving(name: &'static str, permission: &'static str) -> Transition<Document> {
    Transition::new(name)
        .label("Archive")
        .tone("neutral")
        .permission(permission)
        .from([DocumentStatus::Accepted, DocumentStatus::Rejected])
        .to(DocumentStatus::Archived)
        .effect(|document: &Document, _ctx, tx| {
            let id = document.id;
            Box::pin(async move {
                Document::query()
                    .where_(Document::ID.eq(id))
                    .update(&mut *tx, &[("archived_at", &now())])
                    .await?;
                Ok(())
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::TransitionError;

    fn ctx(input: impl Serialize) -> TransitionCtx {
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    fn pending() -> Document {
        Document {
            id: 3,
            current_version: 1,
            ..Document::default()
        }
    }

    #[test]
    fn the_machine_has_every_transition_of_4_1() {
        let machine = Document::machine();
        let names: Vec<_> = machine.transitions().iter().map(|t| t.name()).collect();
        assert_eq!(
            names,
            [
                ACCEPT,
                REJECT,
                RESUBMIT,
                SUPERSEDE,
                ARCHIVE,
                ACCEPT_SOP,
                REJECT_SOP,
                ARCHIVE_SOP
            ]
        );
        assert_eq!(machine.initial_states(), ["pending_review", "accepted"]);
        let accept = machine.find(ACCEPT).unwrap();
        assert_eq!(accept.from_states(), ["pending_review"]);
        assert_eq!(accept.to_state(), Some("accepted"));
        assert!(machine.find(REJECT).unwrap().needs_reason());
        assert!(machine.find(SUPERSEDE).unwrap().is_system());
        assert_eq!(
            machine.find(ARCHIVE).unwrap().from_states(),
            ["accepted", "rejected"]
        );
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(terminal, ["superseded", "archived"]);
    }

    #[renox::test]
    async fn accept_needs_the_fields_the_kind_has() {
        let machine = Document::machine();
        let accept = machine.find(ACCEPT).unwrap();
        let both = AcceptInput {
            needs_number: true,
            needs_expiry: true,
            sop_kind: false,
        };
        let refused = accept.check_guards(&pending(), &ctx(both)).await;
        assert!(matches!(refused, Err(TransitionError::Guard(m)) if m.contains("number")));
        let filled = Document {
            number: Some("784-1990-1234567-1".into()),
            expiry_date: Some("2027-11-30".parse().unwrap()),
            ..pending()
        };
        assert!(accept.check_guards(&filled, &ctx(both)).await.is_ok());
        assert!(
            accept
                .check_guards(&pending(), &ctx(AcceptInput::default()))
                .await
                .is_ok()
        );
        // A bare run, without the kind's rules, is refused.
        assert!(
            accept
                .check_guards(&pending(), &TransitionCtx::new(None))
                .await
                .is_err()
        );
    }

    #[renox::test]
    async fn resubmit_needs_a_new_version_and_supersede_a_newer_document() {
        let machine = Document::machine();
        let resubmit = machine.find(RESUBMIT).unwrap();
        let same = ResubmitInput { version: 1 };
        assert!(resubmit.check_guards(&pending(), &ctx(same)).await.is_err());
        let next = ResubmitInput { version: 2 };
        assert!(resubmit.check_guards(&pending(), &ctx(next)).await.is_ok());
        let supersede = machine.find(SUPERSEDE).unwrap();
        let itself = SupersedeInput { replaced_by: 3 };
        assert!(
            supersede
                .check_guards(&pending(), &ctx(itself))
                .await
                .is_err()
        );
        let newer = SupersedeInput { replaced_by: 8 };
        assert!(
            supersede
                .check_guards(&pending(), &ctx(newer))
                .await
                .is_ok()
        );
    }
}
