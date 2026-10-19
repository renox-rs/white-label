//! The approval of an owner drawing (wl-invoicing §5.16 steps 2 and 3,
//! `FIN.ACL.21`, `FIN.ACL.23`, `PLT.APR.01`): the Drawing approval state
//! machine.
//!
//! ```text
//! [*] --> draft --request--> requested --request_approved--> approved
//!         draft --record_self--> approved          (the sole owner, FIN.ACL.23)
//!         draft --record_below_limit--> approved   (at or below the limit)
//!         requested --back_to_draft--> rejected    (no, withdrawn, expired, failed)
//! ```
//!
//! All five transitions are the system's; [`crate::drawings`] runs them. A
//! supplier bill with a line coded to owner drawings is approved only after
//! one of these is `approved` for the same amount. `requested` is the waiting
//! state: [`REQUEST`] creates the approval request in its effect
//! (`approvals::require_in`, action [`ACTION`]) to the holders of
//! `drawings.approve`, never the requester (SoD); the kernel runs
//! `request_approved` on the last yes and `back_to_draft` when the request
//! ends without one, which here means `rejected` with the reason of the end.
//! The limit is the stored policy's amount condition: AED 0, so every drawing
//! is approved; raised, a drawing at or below it is recorded at once
//! (`record_below_limit`).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use renox::auth::permissions::Scope;
use wl_kernel::{LegalEntity, Subject};
use wl_kernel::approvals::{self, PolicyDefault, Require, Required};
use wl_kernel::machines::SubjectMachine;

use crate::budget_category::refused;
use crate::permissions;
use crate::texts::english;

/// The subject type of a drawing approval in the kernel registry.
pub const SUBJECT_TYPE: &str = "invoicing.drawing_approval";

/// The approval policies' action key of a drawing (`FIN.ACL.21`).
pub const ACTION: &str = "invoicing.owner_drawing";

/// The default limit (`FIN.ACL.21`), in fils: AED 0, so every drawing is
/// approved; the limit is changed on the stored policy.
pub const DRAWINGS_LIMIT: i64 = 0;

/// Hours a drawing request may wait before it can expire.
pub const EXPIRY_HOURS: i64 = 168;

/// `draft → requested`.
pub const REQUEST: &str = "request";
/// `requested → approved`: the kernel runs it on the last yes.
pub const REQUEST_APPROVED: &str = "request_approved";
/// `requested → rejected`: the kernel runs it when the request ends without a
/// yes.
pub const BACK_TO_DRAFT: &str = "back_to_draft";
/// `draft → approved`: the sole owner records their own drawing.
pub const RECORD_SELF: &str = "record_self";
/// `draft → approved`: the drawing is at or below the limit.
pub const RECORD_BELOW_LIMIT: &str = "record_below_limit";

/// Where a drawing approval stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DrawingStatus {
    /// Just made, not yet asked for.
    #[default]
    Draft,
    /// Waiting for the holders of `drawings.approve`.
    Requested,
    /// The drawing may be recorded.
    Approved,
    /// Refused, withdrawn, expired or failed: nothing is approved.
    Rejected,
}

/// The approval of one owner drawing.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_drawing_approvals",
    state_machine,
    index(status),
    index(source_type, source_id)
)]
pub struct DrawingApproval {
    /// The primary key.
    pub id: i64,
    /// The legal entity the drawing is in.
    pub legal_entity_id: i64,
    /// The type of the record the drawing is on, e.g. `invoicing.supplier_bill`.
    pub source_type: String,
    /// The id of that record.
    pub source_id: i64,
    /// The drawing in AED fils, with the VAT that is not recovered.
    pub amount: i64,
    /// The user who asked.
    pub requested_by: Option<i64>,
    /// The user who gave the last yes.
    pub approved_by: Option<i64>,
    /// When it was approved.
    pub approved_at: Option<DateTime>,
    /// The approval request (`PLT.APR.01`), if one was needed.
    pub approval_request_id: Option<i64>,
    /// Whether it was recorded without a second approver (`FIN.ACL.23`).
    pub self_approved: bool,
    /// Why it was not approved: the approver's comment or the kernel's text.
    pub rejected_reason: Option<String>,
    /// The lifecycle state; changed only by a transition.
    pub status: DrawingStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DrawingApproval {
    /// The approval as a [`Subject`]: `invoicing.drawing_approval:4`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The record the drawing is on.
    pub fn source(&self) -> Subject {
        Subject::new(&self.source_type, self.source_id)
    }

    /// The approvals about `source`, newest first.
    pub async fn for_source(
        db: impl renox::db::Executor<'_>,
        source: &Subject,
    ) -> Result<Vec<DrawingApproval>> {
        DrawingApproval::query()
            .where_(DrawingApproval::SOURCE_TYPE.eq(source.subject_type.as_str()))
            .where_(DrawingApproval::SOURCE_ID.eq(source.subject_id))
            .order_by_desc(DrawingApproval::ID)
            .get(db)
            .await
    }

    /// The day it was approved (Asia/Dubai), if it was.
    pub fn approved_on(&self) -> Option<NaiveDate> {
        self.approved_at.map(wl_kernel::documents::dubai_date)
    }
}

/// What a `request` or `record_self` run carries: who asked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RequestInput {
    /// The requester's user id.
    pub requester_id: i64,
}

/// The requester of a run.
fn requester(ctx: &TransitionCtx) -> Option<i64> {
    ctx.input_as::<RequestInput>().ok().map(|i| i.requester_id)
}

/// Writes columns other than the status, in the transition's transaction.
async fn record(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    DrawingApproval::query()
        .where_(DrawingApproval::ID.eq(id))
        .update(&mut *tx, columns)
        .await?;
    Ok(())
}

fn request_transition() -> Transition<DrawingApproval> {
    Transition::new(REQUEST)
        .label("Request approval")
        .from([DrawingStatus::Draft])
        .to(DrawingStatus::Requested)
        .system()
        .effect(|approval: &DrawingApproval, ctx, tx| {
            let id = approval.id;
            let amount = approval.amount;
            let subject = approval.subject();
            let by = requester(ctx);
            Box::pin(async move {
                let by = by.ok_or_else(|| {
                    refused(english("wl_invoicing.drawings.problem.requester_unknown"))
                })?;
                let require =
                    Require::new(ACTION, subject, REQUEST, by, "draft").payload(json!({ "amount": amount }));
                match approvals::require_in(&mut *tx, &require).await? {
                    Required::Pending(request) => {
                        record(tx, id, &[("approval_request_id", &Some(request.id))]).await
                    }
                    // Without the policy a drawing would be recorded
                    // unapproved; the transaction rolls back instead.
                    Required::Allowed => Err(refused(english(
                        "wl_invoicing.drawings.problem.no_approval_policy",
                    ))),
                }
            })
        })
}

fn approved_transition() -> Transition<DrawingApproval> {
    Transition::new(REQUEST_APPROVED)
        .label("Approve")
        .from([DrawingStatus::Requested])
        .to(DrawingStatus::Approved)
        .system()
        .effect(|approval: &DrawingApproval, ctx, tx| {
            let id = approval.id;
            let approver = ctx.meta().get("actor_id").and_then(Value::as_i64);
            Box::pin(async move {
                record(
                    tx,
                    id,
                    &[
                        ("approved_by", &approver),
                        ("approved_at", &Some(renox::db::now())),
                    ],
                )
                .await
            })
        })
}

fn rejected_transition() -> Transition<DrawingApproval> {
    Transition::new(BACK_TO_DRAFT)
        .label("Not approved")
        .from([DrawingStatus::Requested])
        .to(DrawingStatus::Rejected)
        .system()
        .effect(|approval: &DrawingApproval, ctx, tx| {
            let id = approval.id;
            let reason = ctx
                .reason()
                .map(|r| r.trim().to_owned())
                .filter(|r| !r.is_empty());
            Box::pin(async move { record(tx, id, &[("rejected_reason", &reason)]).await })
        })
}

fn self_transition() -> Transition<DrawingApproval> {
    Transition::new(RECORD_SELF)
        .label("Record (sole owner)")
        .from([DrawingStatus::Draft])
        .to(DrawingStatus::Approved)
        .system()
        .effect(|approval: &DrawingApproval, ctx, tx| {
            let id = approval.id;
            let by = requester(ctx);
            Box::pin(async move {
                record(
                    tx,
                    id,
                    &[
                        ("approved_by", &by),
                        ("approved_at", &Some(renox::db::now())),
                        ("self_approved", &true),
                    ],
                )
                .await
            })
        })
}

fn below_limit_transition() -> Transition<DrawingApproval> {
    Transition::new(RECORD_BELOW_LIMIT)
        .label("Record (below the limit)")
        .from([DrawingStatus::Draft])
        .to(DrawingStatus::Approved)
        .system()
        .effect(|approval: &DrawingApproval, _ctx, tx| {
            let id = approval.id;
            Box::pin(async move {
                record(tx, id, &[("approved_at", &Some(renox::db::now()))]).await
            })
        })
}

impl StateMachine for DrawingApproval {
    type State = DrawingStatus;
    const COLUMN: renox::db::Col<DrawingApproval, DrawingStatus> = DrawingApproval::STATUS;

    fn machine() -> Machine<Self> {
        use DrawingStatus::{Approved, Draft, Rejected, Requested};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Requested)
            .tone("warning")
            .state(Approved)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("error")
            .terminal()
            .transition(request_transition())
            .transition(approved_transition())
            .transition(rejected_transition())
            .transition(self_transition())
            .transition(below_limit_transition())
    }
}

/// The default policy: every drawing above the limit (AED 0) needs one
/// approval by a holder of `drawings.approve` other than the requester.
pub fn approval_policy() -> PolicyDefault {
    PolicyDefault::new(ACTION)
        .amount_above(DRAWINGS_LIMIT)
        .step_permission(permissions::DRAWINGS_APPROVE)
        .expiry_hours(Some(EXPIRY_HOURS))
}

/// The page of the record a drawing approval is about, for the approvers'
/// "Needs your action" list: the approval redirects to its bill.
pub(crate) fn page_of(id: i64) -> String {
    format!("/drawing-approvals/{id}")
}

/// Registers the machine with the kernel (waiting state `requested`), the
/// default policy and the page for the approvers.
pub(crate) fn register() {
    SubjectMachine::of::<DrawingApproval>(SUBJECT_TYPE)
        .waiting(DrawingStatus::Requested)
        .scope(|drawing| Scope::of_id::<LegalEntity>(drawing.legal_entity_id))
        .register();
    renox::state_machine::register::<DrawingApproval>();
    approval_policy().register();
    approvals::register_subject_link(SUBJECT_TYPE, page_of);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_the_five_system_transitions() {
        let machine = DrawingApproval::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        let find = |name: &str| machine.find(name).expect("transition");
        assert_eq!(find(REQUEST).from_states(), ["draft"]);
        assert_eq!(find(REQUEST).to_state(), Some("requested"));
        assert_eq!(find(REQUEST_APPROVED).from_states(), ["requested"]);
        assert_eq!(find(REQUEST_APPROVED).to_state(), Some("approved"));
        assert_eq!(find(BACK_TO_DRAFT).to_state(), Some("rejected"));
        assert_eq!(find(RECORD_SELF).to_state(), Some("approved"));
        assert_eq!(find(RECORD_BELOW_LIMIT).to_state(), Some("approved"));
        assert!(machine.transitions().iter().all(|t| t.is_system()));
    }

    #[test]
    fn the_approval_waits_in_requested_and_asks_for_drawings_approve() {
        let machine = SubjectMachine::of::<DrawingApproval>(SUBJECT_TYPE)
            .waiting(DrawingStatus::Requested)
            .build();
        assert_eq!(machine.subject_type(), "invoicing.drawing_approval");
        assert_eq!(machine.waiting_state(), Some("requested"));
        let expected = PolicyDefault::new("invoicing.owner_drawing")
            .amount_above(0)
            .step_permission("drawings.approve")
            .expiry_hours(Some(168));
        assert_eq!(approval_policy(), expected);
    }

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let statuses: Vec<_> = DrawingStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(statuses, ["draft", "requested", "approved", "rejected"]);
    }

    #[test]
    fn the_source_is_the_record_the_drawing_is_on() {
        let approval = DrawingApproval {
            id: 4,
            source_type: "invoicing.supplier_bill".into(),
            source_id: 9,
            ..Default::default()
        };
        assert_eq!(approval.subject(), Subject::new("invoicing.drawing_approval", 4));
        assert_eq!(approval.source(), Subject::new("invoicing.supplier_bill", 9));
        assert_eq!(page_of(4), "/drawing-approvals/4");
    }
}
