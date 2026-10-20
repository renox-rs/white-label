//! The Return machine (04 §7.1 *Return*).
//!
//! | Transition | From | To | By |
//! |---|---|---|---|
//! | [`SUBMIT`] | preparing | in_review | `returns.prepare` |
//! | [`SEND_BACK`] | in_review, client_approval | preparing | `returns.review` or a client approver |
//! | [`REVIEW`] | in_review | client_approval | `returns.review`, not the preparer (`ACC.ACL.03`) |
//! | [`APPROVE`] | client_approval | approved | a client approver (`ACC.ACL.11`) |
//! | [`WITHDRAW_APPROVAL`] | approved | preparing | a client approver, or `returns.review` on the client's written instruction |
//! | [`FILE`] | approved | filed | `returns.file` |
//! | [`CORRECT`] | filed | superseded | `returns.prepare` |
//! | [`CANCEL`] | preparing, in_review, client_approval, approved | cancelled | `returns.review` |
//!
//! A client approver has no staff permission: those transitions are open to
//! any signed-in user and refuse unless the service found the user to be a
//! client approver of the return's client ([`ReturnInput::client_approver`],
//! from the portal grants), so a transition run directly with the flag unset
//! is refused too. The services in [`crate::returns`] gather what the guards
//! need; the effects write only the return's own columns and its
//! signatures. What reaches other modules (notices, the linked task) runs
//! after the commit ([`crate::events`]).
//!
//! Built here in full: `submit`, `send_back`, `review`, `approve`,
//! `withdraw_approval` and `cancel`. `file` and `correct` are entries with
//! their basic guards (an acknowledgement; a reason) and effects; the tax
//! agent rule, the MLRO's approval for a blocked client, the linked task's
//! completion and the new version come with BL-ACC-18 and BL-ACC-19.

use renox::chrono::NaiveDate;
use renox::db::ToDbValue;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals;
use wl_kernel::documents::today;
use wl_kernel::machines::SubjectMachine;

use crate::db::set_return;
use crate::permissions;
use crate::reasons;
use crate::tax_return::{
    CorrectionRoute, RETURN_SUBJECT_TYPE, Return, ReturnSignature, ReturnStatus,
};
use crate::texts::english;

/// The `submit` transition's name.
pub const SUBMIT: &str = "submit";
/// The `send_back` transition's name.
pub const SEND_BACK: &str = "send_back";
/// The `review` transition's name.
pub const REVIEW: &str = "review";
/// The `approve` transition's name.
pub const APPROVE: &str = "approve";
/// The `withdraw_approval` transition's name.
pub const WITHDRAW_APPROVAL: &str = "withdraw_approval";
/// The `file` transition's name.
pub const FILE: &str = "file";
/// The `correct` transition's name.
pub const CORRECT: &str = "correct";
/// The `cancel` transition's name.
pub const CANCEL: &str = "cancel";

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnInput {
    /// The business day of the run (Asia/Dubai); today when empty.
    #[serde(default)]
    pub on: Option<NaiveDate>,
    /// `submit`: the figures are entered.
    #[serde(default)]
    pub figures_entered: bool,
    /// `submit`: the workpapers are attached.
    #[serde(default)]
    pub workpapers_attached: bool,
    /// The user holds `returns.review` (`send_back`, `withdraw_approval`).
    #[serde(default)]
    pub reviewer: bool,
    /// The user is a client approver of the return's client with the
    /// engagement still running (`approve`, `send_back`,
    /// `withdraw_approval`; `ACC.ACL.11`).
    #[serde(default)]
    pub client_approver: bool,
    /// `withdraw_approval` by a reviewer: the client's written instruction,
    /// attached.
    #[serde(default)]
    pub instruction: Option<String>,
    /// `approve`: the signer's name.
    #[serde(default)]
    pub signer_name: Option<String>,
    /// `approve`: the signer's IP address.
    #[serde(default)]
    pub ip_address: Option<String>,
    /// `approve`: the hash of the summary that was approved.
    #[serde(default)]
    pub document_hash: Option<String>,
    /// `file`: the FTA acknowledgement attached.
    #[serde(default)]
    pub acknowledgement: Option<String>,
    /// `file`: the client filed it through its own EmaraTax user.
    #[serde(default)]
    pub filed_by_client: bool,
    /// `file`: the filer's tax agent registration number.
    #[serde(default)]
    pub tax_agent_registration: Option<String>,
    /// `cancel`: the engagement ended after a CDD exit or rejection, so the
    /// client is told nothing automatically (`ACC.ACL.14`).
    #[serde(default)]
    pub restricted: bool,
    /// `correct`: how the error is corrected.
    #[serde(default)]
    pub route: Option<CorrectionRoute>,
}

impl ReturnInput {
    /// An input for the day `on`.
    pub fn on(on: NaiveDate) -> Self {
        ReturnInput {
            on: Some(on),
            ..ReturnInput::default()
        }
    }
}

/// The input of a run; the default without one (`available`).
pub(crate) fn input_of(ctx: &TransitionCtx) -> ReturnInput {
    ctx.input_as::<ReturnInput>().unwrap_or_default()
}

fn given(text: &Option<String>) -> bool {
    text.as_deref().is_some_and(|t| !t.trim().is_empty())
}

impl StateMachine for Return {
    type State = ReturnStatus;
    const COLUMN: renox::db::Col<Return, ReturnStatus> = Return::STATUS;

    fn machine() -> Machine<Self> {
        use ReturnStatus::{
            Approved, Cancelled, ClientApproval, Filed, InReview, Preparing, Superseded,
        };
        Machine::new()
            .initial([Preparing])
            .state(Preparing)
            .tone("neutral")
            .state(InReview)
            .tone("info")
            .state(ClientApproval)
            .tone("info")
            .state(Approved)
            .tone("success")
            .state(Filed)
            .tone("success")
            .state(Superseded)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(submit())
            .transition(send_back())
            .transition(review())
            .transition(approve())
            .transition(withdraw_approval())
            .transition(file())
            .transition(correct())
            .transition(cancel())
    }
}

/// `submit`: preparing → in_review once the figures are entered and the
/// workpapers attached; the submitter is the preparer.
fn submit() -> Transition<Return> {
    Transition::new(SUBMIT)
        .label("Submit for review")
        .tone("primary")
        .permission(permissions::RETURNS_PREPARE)
        .from([ReturnStatus::Preparing])
        .to(ReturnStatus::InReview)
        .guard("figures_and_workpapers", |_r: &Return, ctx| {
            let input = input_of(ctx);
            Box::pin(async move {
                if !input.figures_entered {
                    Err(english("accounting.returns.problem.no_figures", &[]))
                } else if !input.workpapers_attached {
                    Err(english("accounting.returns.problem.no_workpapers", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|r: &Return, ctx, tx| {
            let id = r.id;
            let by = ctx.actor().map(|u| u.id);
            Box::pin(async move {
                set_return(
                    &mut *tx,
                    id,
                    &[
                        ("preparer_user_id", &by),
                        ("send_back_comment", &None::<String>),
                    ],
                )
                .await
            })
        })
}

/// `send_back`: in_review or client_approval → preparing, by the reviewer or
/// a client approver, with a comment.
fn send_back() -> Transition<Return> {
    Transition::new(SEND_BACK)
        .label("Send back")
        .tone("warning")
        .reason_required()
        .from([ReturnStatus::InReview, ReturnStatus::ClientApproval])
        .to(ReturnStatus::Preparing)
        .guard("reviewer_or_client_approver", |_r: &Return, ctx| {
            let input = input_of(ctx);
            Box::pin(async move {
                if input.reviewer || input.client_approver {
                    Ok(())
                } else {
                    Err(english("accounting.returns.problem.not_allowed", &[]))
                }
            })
        })
        .effect(|r: &Return, ctx, tx| {
            let id = r.id;
            let comment = ctx.reason().map(|c| c.trim().to_owned());
            Box::pin(
                async move { set_return(&mut *tx, id, &[("send_back_comment", &comment)]).await },
            )
        })
}

/// `review`: in_review → client_approval, by a reviewer who is not the
/// preparer (`ACC.ACL.03`).
fn review() -> Transition<Return> {
    Transition::new(REVIEW)
        .label("Send to the client")
        .tone("primary")
        .permission(permissions::RETURNS_REVIEW)
        .from([ReturnStatus::InReview])
        .to(ReturnStatus::ClientApproval)
        .guard("reviewer_not_preparer", |r: &Return, ctx| {
            let same = match (ctx.actor().map(|u| u.id), r.preparer_user_id) {
                (Some(reviewer), Some(preparer)) => reviewer == preparer,
                _ => false,
            };
            Box::pin(async move {
                if same {
                    Err(english("accounting.returns.problem.same_person", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|r: &Return, ctx, tx| {
            let id = r.id;
            let by = ctx.actor().map(|u| u.id);
            Box::pin(async move { set_return(&mut *tx, id, &[("reviewer_user_id", &by)]).await })
        })
}

/// `approve`: client_approval → approved, by a client approver; records the
/// signature (`DOC.SIG.01`).
fn approve() -> Transition<Return> {
    Transition::new(APPROVE)
        .label("Approve")
        .tone("success")
        .from([ReturnStatus::ClientApproval])
        .to(ReturnStatus::Approved)
        .guard("client_approver", |_r: &Return, ctx| {
            let approver = input_of(ctx).client_approver;
            Box::pin(async move {
                if approver {
                    Ok(())
                } else {
                    Err(english("accounting.returns.problem.not_approver", &[]))
                }
            })
        })
        .effect(|r: &Return, ctx, tx| {
            let input = input_of(ctx);
            let mut signature = ReturnSignature {
                return_id: r.id,
                signer_user_id: ctx.actor().map_or(0, |u| u.id),
                signer_name: input.signer_name.unwrap_or_default(),
                signed_at: renox::db::now(),
                ip_address: input.ip_address,
                document_hash: input.document_hash.unwrap_or_default(),
                ..ReturnSignature::default()
            };
            Box::pin(async move {
                signature.insert(&mut *tx).await?;
                Ok(())
            })
        })
}

/// `withdraw_approval`: approved → preparing, by a client approver with a
/// reason, or by a reviewer who attaches the client's written instruction.
/// The signature is kept and marked withdrawn.
fn withdraw_approval() -> Transition<Return> {
    Transition::new(WITHDRAW_APPROVAL)
        .label("Withdraw approval")
        .tone("warning")
        .reason_required()
        .from([ReturnStatus::Approved])
        .to(ReturnStatus::Preparing)
        .guard("client_approver_or_reviewer", |_r: &Return, ctx| {
            let input = input_of(ctx);
            Box::pin(async move {
                if input.client_approver || input.reviewer {
                    Ok(())
                } else {
                    Err(english("accounting.returns.problem.not_allowed", &[]))
                }
            })
        })
        .guard("instruction_attached", |_r: &Return, ctx| {
            let input = input_of(ctx);
            let needed = !input.client_approver;
            let attached = given(&input.instruction);
            Box::pin(async move {
                if needed && !attached {
                    Err(english("accounting.returns.problem.no_instruction", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|r: &Return, ctx, tx| {
            let id = r.id;
            let by = ctx.actor().map(|u| u.id);
            let reason = ctx.reason().map(|c| c.trim().to_owned());
            let instruction = if input_of(ctx).client_approver {
                None
            } else {
                input_of(ctx).instruction.map(|i| i.trim().to_owned())
            };
            Box::pin(async move {
                let at = Some(renox::db::now());
                let values: [(&str, &(dyn ToDbValue + Sync)); 4] = [
                    ("withdrawn_at", &at),
                    ("withdrawn_by", &by),
                    ("withdrawn_reason", &reason),
                    ("withdrawn_instruction", &instruction),
                ];
                ReturnSignature::query()
                    .where_(ReturnSignature::RETURN_ID.eq(id))
                    .where_(ReturnSignature::WITHDRAWN_AT.is_null())
                    .update(&mut *tx, &values)
                    .await?;
                Ok(())
            })
        })
}

/// `file`: approved → filed with the FTA acknowledgement attached. Basic
/// guard only; BL-ACC-18 adds the tax agent rule and the block check.
fn file() -> Transition<Return> {
    Transition::new(FILE)
        .label("Mark as filed")
        .tone("success")
        .permission(permissions::RETURNS_FILE)
        .from([ReturnStatus::Approved])
        .to(ReturnStatus::Filed)
        .guard("acknowledgement", |_r: &Return, ctx| {
            let attached = given(&input_of(ctx).acknowledgement);
            Box::pin(async move {
                if attached {
                    Ok(())
                } else {
                    Err(english(
                        "accounting.returns.problem.no_acknowledgement",
                        &[],
                    ))
                }
            })
        })
        .effect(|r: &Return, ctx, tx| {
            let id = r.id;
            let input = input_of(ctx);
            let on = input.on.unwrap_or_else(today);
            let by = ctx.actor().map(|u| u.id);
            Box::pin(async move {
                set_return(
                    &mut *tx,
                    id,
                    &[
                        ("filed_on", &Some(on)),
                        ("filed_by_user_id", &by),
                        ("filed_by_client", &input.filed_by_client),
                        ("tax_agent_registration", &input.tax_agent_registration),
                        ("acknowledgement", &input.acknowledgement),
                    ],
                )
                .await
            })
        })
}

/// `correct`: filed → superseded with a reason. Basic entry only;
/// BL-ACC-19 adds the route rules and creates the new version.
fn correct() -> Transition<Return> {
    Transition::new(CORRECT)
        .label("Correct")
        .tone("warning")
        .permission(permissions::RETURNS_PREPARE)
        .reason_required()
        .from([ReturnStatus::Filed])
        .to(ReturnStatus::Superseded)
        .effect(|r: &Return, ctx, tx| {
            let id = r.id;
            let reason = ctx.reason().map(|c| c.trim().to_owned());
            let route = input_of(ctx).route;
            Box::pin(async move {
                set_return(
                    &mut *tx,
                    id,
                    &[("correction_reason", &reason), ("correction_route", &route)],
                )
                .await
            })
        })
}

/// `cancel`: a return in progress the firm will not file, with a reason of
/// [`reasons::RETURN_CANCEL_REASONS`].
fn cancel() -> Transition<Return> {
    Transition::new(CANCEL)
        .label("Cancel")
        .tone("danger")
        .permission(permissions::RETURNS_REVIEW)
        .reason_required()
        .from([
            ReturnStatus::Preparing,
            ReturnStatus::InReview,
            ReturnStatus::ClientApproval,
            ReturnStatus::Approved,
        ])
        .to(ReturnStatus::Cancelled)
        .guard("reason_listed", |_r: &Return, ctx| {
            let ok = reasons::is_one_of(ctx.reason(), &reasons::RETURN_CANCEL_REASONS);
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(english("accounting.problems.reason_unknown", &[]))
                }
            })
        })
        .effect(|r: &Return, ctx, tx| {
            let id = r.id;
            let code = ctx.reason().map(|c| c.trim().to_owned());
            let restricted = input_of(ctx).restricted;
            Box::pin(async move {
                set_return(
                    &mut *tx,
                    id,
                    &[
                        ("cancel_reason", &code),
                        ("client_messages_suppressed", &restricted),
                    ],
                )
                .await
            })
        })
}

/// Registers the machine for `rnx state:diagram` and the admin, and with
/// the kernel (the return's page for "Needs your action").
pub(crate) fn register() {
    SubjectMachine::of::<Return>(RETURN_SUBJECT_TYPE).register();
    renox::state_machine::register::<Return>();
    approvals::register_subject_link(RETURN_SUBJECT_TYPE, |id| {
        format!("/accounting/returns/{id}")
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition() {
        let machine = Return::machine();
        let states: Vec<(String, Option<String>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.clone(), s.tone.clone(), s.terminal))
            .collect();
        assert_eq!(
            states,
            vec![
                ("preparing".into(), Some("neutral".into()), false),
                ("in_review".into(), Some("info".into()), false),
                ("client_approval".into(), Some("info".into()), false),
                ("approved".into(), Some("success".into()), false),
                ("filed".into(), Some("success".into()), false),
                ("superseded".into(), Some("neutral".into()), true),
                ("cancelled".into(), Some("neutral".into()), true),
            ]
        );
        let transitions: Vec<(&str, Vec<String>, Option<&str>, bool)> = machine
            .transitions()
            .iter()
            .map(|t| {
                (
                    t.name(),
                    t.from_states().to_vec(),
                    t.to_state(),
                    t.is_system(),
                )
            })
            .collect();
        let open = || {
            vec![
                "preparing".to_owned(),
                "in_review".to_owned(),
                "client_approval".to_owned(),
                "approved".to_owned(),
            ]
        };
        assert_eq!(
            transitions,
            vec![
                (SUBMIT, vec!["preparing".into()], Some("in_review"), false),
                (
                    SEND_BACK,
                    vec!["in_review".into(), "client_approval".into()],
                    Some("preparing"),
                    false
                ),
                (
                    REVIEW,
                    vec!["in_review".into()],
                    Some("client_approval"),
                    false
                ),
                (
                    APPROVE,
                    vec!["client_approval".into()],
                    Some("approved"),
                    false
                ),
                (
                    WITHDRAW_APPROVAL,
                    vec!["approved".into()],
                    Some("preparing"),
                    false
                ),
                (FILE, vec!["approved".into()], Some("filed"), false),
                (CORRECT, vec!["filed".into()], Some("superseded"), false),
                (CANCEL, open(), Some("cancelled"), false),
            ]
        );
        assert_eq!(
            machine.find(SUBMIT).unwrap().guard_names(),
            ["figures_and_workpapers"]
        );
        assert_eq!(
            machine.find(REVIEW).unwrap().guard_names(),
            ["reviewer_not_preparer"]
        );
        assert_eq!(
            machine.find(APPROVE).unwrap().guard_names(),
            ["client_approver"]
        );
        assert_eq!(
            machine.find(WITHDRAW_APPROVAL).unwrap().guard_names(),
            ["client_approver_or_reviewer", "instruction_attached"]
        );
        assert_eq!(
            machine.find(FILE).unwrap().guard_names(),
            ["acknowledgement"]
        );
        assert_eq!(
            machine.find(CANCEL).unwrap().guard_names(),
            ["reason_listed"]
        );
        for name in [SEND_BACK, WITHDRAW_APPROVAL, CORRECT, CANCEL] {
            assert!(machine.find(name).unwrap().needs_reason(), "{name}");
        }
        for name in [SUBMIT, REVIEW, APPROVE, FILE] {
            assert!(!machine.find(name).unwrap().needs_reason(), "{name}");
        }
    }
}
