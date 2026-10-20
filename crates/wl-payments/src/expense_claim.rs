//! Expense claims (wl-payments §3 *Expense claim*, *Expense line*, §4, `FIN.ACL.11`,
//! `FIN.ACL.21`; issue #90): the part of the claim lifecycle that routes a line in
//! an *Owner drawings* category through the drawings approval.
//!
//! ```text
//! [*] --> draft --submit--> submitted --approve--> approved (terminal here)
//!         draft <--return-- submitted --reject--> rejected (terminal)
//! ```
//!
//! A claim has lines, each with an expense category ([`crate::expense_category`]),
//! the amount paid with its VAT and the day. A line whose category is an owner
//! drawings category is a **drawing** (VAT blocked, so counted with it). At
//! `submit` the drawings of the claim are asked for as one drawing
//! ([`wl_kernel::drawings::OwnerDrawings::request`], source
//! [`wl_kernel::drawings::EXPENSE_CLAIM_SUBJECT_TYPE`]): a request to a holder of
//! `drawings.approve` other than the claimant; or, for the entity's only
//! holder, an approval at once flagged *self-approved* (`FIN.ACL.23`), the
//! accountant being told by wl-invoicing. `approve` of the claim needs the
//! drawing to be approved ([`ClaimDrawing`]) and an approver other than the
//! claimant (`FIN.ACL.11`). A rejected, withdrawn or expired drawing request
//! leaves the claim `submitted` with the reason shown; the claim is returned to
//! `draft`, the line recoded or removed, and submitted again (a recoded claim
//! that is still a drawing gets a new request).
//!
//! Not built here (the expense feature, `FIN.EXP.01`): receipts and claim
//! windows, approval limits in steps (`FIN.ACL.12`), cost posting and input VAT,
//! reimbursement and the petty cash float, the claim PDF and the mobile form.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::drawings::{
    DrawingRequest, DrawingStart, EXPENSE_CLAIM_SUBJECT_TYPE, OwnerDrawingDecided, owner_drawings,
};
use wl_kernel::{LegalEntity, Subject};

use crate::expense_category::ExpenseCategory;
use crate::followup::money_text;
use crate::permissions;
use crate::texts::english;

/// The `submit` transition's name.
pub const SUBMIT: &str = "submit";
/// The `approve` transition's name.
pub const APPROVE: &str = "approve";
/// The `reject` transition's name.
pub const REJECT: &str = "reject";
/// The `return` transition's name.
pub const RETURN: &str = "return";

/// The lifecycle state of a claim (wl-payments §4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClaimStatus {
    /// Being filled in.
    #[default]
    Draft,
    /// Waiting for approval.
    Submitted,
    /// Approved.
    Approved,
    /// Not approved.
    Rejected,
}

/// Where the drawing of a claim stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClaimDrawing {
    /// Waiting for a holder of `drawings.approve`.
    #[default]
    Requested,
    /// Approved.
    Approved,
    /// Refused, withdrawn or expired.
    Rejected,
}

/// An expense claim. Money is AED fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_payments_expense_claims", index(status))]
pub struct ExpenseClaim {
    /// The primary key.
    pub id: i64,
    /// The legal entity the claim is in.
    pub legal_entity_id: i64,
    /// The user who claims.
    pub claimant_id: i64,
    /// What it is for.
    pub title: String,
    /// The lifecycle state.
    pub status: ClaimStatus,
    /// The sum of the lines, VAT included.
    pub total: i64,
    /// The VAT in the total.
    pub vat_total: i64,
    /// The part of the total in owner drawings categories.
    pub drawing_amount: i64,
    /// Where the drawing stands; `None` without one.
    pub drawing_status: Option<ClaimDrawing>,
    /// Whether the sole owner approved the drawing alone (`FIN.ACL.23`).
    pub drawing_self_approved: bool,
    /// The user who gave the last yes to the drawing.
    pub drawing_approved_by: Option<i64>,
    /// Why the drawing was not approved.
    pub drawing_reason: Option<String>,
    /// The user who approved the claim.
    pub approved_by: Option<i64>,
    /// Why the claim was rejected or returned.
    pub decision_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ExpenseClaim {
    /// The claim as a [`Subject`]: `payments.expense_claim:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(EXPENSE_CLAIM_SUBJECT_TYPE, self.id)
    }

    /// The lines of the claim, in order.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn lines(&self, db: &Db) -> Result<Vec<ExpenseLine>> {
        ExpenseLine::query()
            .where_(ExpenseLine::CLAIM_ID.eq(self.id))
            .order_by(ExpenseLine::POSITION)
            .get(db)
            .await
    }
}

/// One receipt of a claim.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_expense_lines",
    unique(claim_id, position),
    index(claim_id)
)]
pub struct ExpenseLine {
    /// The primary key.
    pub id: i64,
    /// The claim.
    pub claim_id: i64,
    /// The place in the claim, from 1.
    pub position: i64,
    /// The day of the expense.
    pub expense_date: NaiveDate,
    /// Who was paid.
    pub merchant: String,
    /// The code of the expense category.
    pub category: String,
    /// What it was for.
    pub description: String,
    /// What was paid, VAT included.
    pub amount: i64,
    /// The VAT in the amount.
    pub vat_amount: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A line to enter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineInput {
    /// The day of the expense.
    pub expense_date: NaiveDate,
    /// Who was paid.
    pub merchant: String,
    /// The code of the expense category.
    pub category: String,
    /// What it was for.
    pub description: String,
    /// What was paid, VAT included, in fils.
    pub amount: i64,
    /// The VAT in it, in fils.
    pub vat_amount: i64,
}

fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

fn guard(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

/// What `submit` carries: where the drawing stands after it was asked for.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SubmitInput {
    /// The sum of the lines.
    pub total: i64,
    /// The VAT in it.
    pub vat_total: i64,
    /// The part in owner drawings categories.
    pub drawing_amount: i64,
    /// The drawing's start; `None` without one.
    pub drawing: Option<ClaimDrawing>,
    /// Whether it was approved without a second approver.
    pub self_approved: bool,
    /// The user who approved it at once.
    pub approved_by: Option<i64>,
}

fn input<T: serde::de::DeserializeOwned + Default>(ctx: &TransitionCtx) -> T {
    ctx.input_as::<T>().unwrap_or_default()
}

/// Writes columns other than the status, in the transition's transaction.
async fn record(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    ExpenseClaim::query()
        .where_(ExpenseClaim::ID.eq(id))
        .update(&mut *tx, columns)
        .await?;
    Ok(())
}

impl StateMachine for ExpenseClaim {
    type State = ClaimStatus;
    const COLUMN: renox::db::Col<ExpenseClaim, ClaimStatus> = ExpenseClaim::STATUS;

    fn machine() -> Machine<Self> {
        use ClaimStatus::{Approved, Draft, Rejected, Submitted};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Submitted)
            .tone("info")
            .state(Approved)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("neutral")
            .terminal()
            .transition(submit_transition())
            .transition(approve_transition())
            .transition(reject_transition())
            .transition(return_transition())
    }
}

fn submit_transition() -> Transition<ExpenseClaim> {
    Transition::new(SUBMIT)
        .label("Submit")
        .tone("primary")
        .permission(permissions::EXPENSES_SUBMIT)
        .from([ClaimStatus::Draft])
        .to(ClaimStatus::Submitted)
        .guard("has_lines", |_claim: &ExpenseClaim, ctx| {
            let total = input::<SubmitInput>(ctx).total;
            Box::pin(async move {
                if total > 0 {
                    Ok(())
                } else {
                    Err(english("wl_payments.expense_claims.problem.no_lines").to_owned())
                }
            })
        })
        .effect(|claim: &ExpenseClaim, ctx, tx| {
            let id = claim.id;
            let given = input::<SubmitInput>(ctx);
            Box::pin(async move {
                record(
                    tx,
                    id,
                    &[
                        ("total", &given.total),
                        ("vat_total", &given.vat_total),
                        ("drawing_amount", &given.drawing_amount),
                        ("drawing_status", &given.drawing),
                        ("drawing_self_approved", &given.self_approved),
                        ("drawing_approved_by", &given.approved_by),
                        ("drawing_reason", &None::<String>),
                        ("decision_reason", &None::<String>),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await
            })
        })
}

fn approve_transition() -> Transition<ExpenseClaim> {
    Transition::new(APPROVE)
        .label("Approve")
        .tone("primary")
        .permission(permissions::EXPENSES_APPROVE)
        .from([ClaimStatus::Submitted])
        .to(ClaimStatus::Approved)
        // `FIN.ACL.11`: a claimant never approves their own claim.
        .guard("not_the_claimant", |claim: &ExpenseClaim, ctx| {
            let own = ctx.actor().is_some_and(|user| user.id == claim.claimant_id);
            Box::pin(async move {
                if own {
                    Err(english("wl_payments.expense_claims.problem.own_claim").to_owned())
                } else {
                    Ok(())
                }
            })
        })
        // `FIN.ACL.21`: a claim with a drawing is approved only after the
        // drawing is.
        .guard("drawing_approved", |claim: &ExpenseClaim, _ctx| {
            let problem = drawing_problem(claim);
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|claim: &ExpenseClaim, ctx, tx| {
            let id = claim.id;
            let approver = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                record(
                    tx,
                    id,
                    &[
                        ("approved_by", &approver),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await
            })
        })
}

/// Why the drawing of `claim` holds its approval back, if it does.
pub(crate) fn drawing_problem(claim: &ExpenseClaim) -> Option<String> {
    if claim.drawing_amount <= 0 {
        return None;
    }
    match claim.drawing_status {
        Some(ClaimDrawing::Approved) => None,
        Some(ClaimDrawing::Rejected) => Some(
            english("wl_payments.expense_claims.problem.drawing_rejected")
                .replace("{reason}", claim.drawing_reason.as_deref().unwrap_or("")),
        ),
        _ => Some(english("wl_payments.expense_claims.problem.drawing_waits").to_owned()),
    }
}

fn reject_transition() -> Transition<ExpenseClaim> {
    Transition::new(REJECT)
        .label("Reject")
        .tone("danger")
        .permission(permissions::EXPENSES_APPROVE)
        .from([ClaimStatus::Submitted])
        .to(ClaimStatus::Rejected)
        .reason_required()
        .effect(|claim: &ExpenseClaim, ctx, tx| {
            let id = claim.id;
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                record(
                    tx,
                    id,
                    &[
                        ("decision_reason", &reason),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await
            })
        })
}

fn return_transition() -> Transition<ExpenseClaim> {
    Transition::new(RETURN)
        .label("Return")
        .permission(permissions::EXPENSES_APPROVE)
        .from([ClaimStatus::Submitted])
        .to(ClaimStatus::Draft)
        .reason_required()
        .effect(|claim: &ExpenseClaim, ctx, tx| {
            let id = claim.id;
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                record(
                    tx,
                    id,
                    &[
                        ("decision_reason", &reason),
                        ("drawing_status", &None::<ClaimDrawing>),
                        ("drawing_self_approved", &false),
                        ("drawing_approved_by", &None::<i64>),
                        ("drawing_reason", &None::<String>),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await
            })
        })
}

/// The categories of `lines`, each found and active.
async fn categories_of(db: &Db, lines: &[LineInput]) -> Result<Vec<ExpenseCategory>> {
    let mut found = Vec::with_capacity(lines.len());
    for line in lines {
        let category = ExpenseCategory::by_code(db, line.category.trim())
            .await?
            .filter(|c| c.active)
            .ok_or_else(|| refused(english("wl_payments.expense_claims.problem.category")))?;
        found.push(category);
    }
    Ok(found)
}

fn check_lines(lines: &[LineInput]) -> Result {
    if lines.is_empty() {
        return Err(refused(english(
            "wl_payments.expense_claims.problem.no_lines",
        )));
    }
    if lines
        .iter()
        .any(|l| l.amount <= 0 || l.vat_amount < 0 || l.vat_amount > l.amount)
    {
        return Err(refused(english(
            "wl_payments.expense_claims.problem.amount",
        )));
    }
    Ok(())
}

/// Replaces the lines of the `draft` claim `claim_id` with `lines` and
/// refreshes its totals.
async fn store_lines(db: &Db, claim_id: i64, lines: &[LineInput]) -> Result<()> {
    ExpenseLine::query()
        .where_(ExpenseLine::CLAIM_ID.eq(claim_id))
        .delete(db)
        .await?;
    for (i, line) in lines.iter().enumerate() {
        ExpenseLine {
            claim_id,
            position: i64::try_from(i).unwrap_or(0) + 1,
            expense_date: line.expense_date,
            merchant: line.merchant.trim().to_owned(),
            category: line.category.trim().to_owned(),
            description: line.description.trim().to_owned(),
            amount: line.amount,
            vat_amount: line.vat_amount,
            ..Default::default()
        }
        .insert(db)
        .await?;
    }
    Ok(())
}

/// A draft claim of `claimant` in the entity `legal_entity_id` with `lines`.
///
/// # Errors
///
/// 422 for no line, an amount that is not above 0 (or a VAT above it), a
/// category that is unknown or inactive, or an unknown entity.
pub async fn create(
    db: &Db,
    claimant: i64,
    legal_entity_id: i64,
    title: &str,
    lines: &[LineInput],
) -> Result<ExpenseClaim> {
    check_lines(lines)?;
    categories_of(db, lines).await?;
    if LegalEntity::find(db, legal_entity_id).await?.is_none() {
        return Err(refused(english(
            "wl_payments.expense_claims.problem.entity",
        )));
    }
    let mut claim = ExpenseClaim {
        legal_entity_id,
        claimant_id: claimant,
        title: title.trim().to_owned(),
        status: ClaimStatus::Draft,
        total: lines.iter().map(|l| l.amount).sum(),
        vat_total: lines.iter().map(|l| l.vat_amount).sum(),
        ..Default::default()
    };
    claim.insert(db).await?;
    store_lines(db, claim.id, lines).await?;
    Ok(claim)
}

/// Replaces the lines of the `draft` claim `id` (after a return, to recode or
/// remove a drawing).
///
/// # Errors
///
/// 422 as for [`create`], or when the claim is not a draft.
pub async fn replace_lines(db: &Db, id: i64, lines: &[LineInput]) -> Result<ExpenseClaim> {
    let mut claim = ExpenseClaim::find_or_404(db, id).await?;
    if claim.status != ClaimStatus::Draft {
        return Err(refused(english(
            "wl_payments.expense_claims.problem.not_draft",
        )));
    }
    check_lines(lines)?;
    categories_of(db, lines).await?;
    store_lines(db, claim.id, lines).await?;
    claim.total = lines.iter().map(|l| l.amount).sum();
    claim.vat_total = lines.iter().map(|l| l.vat_amount).sum();
    claim.save(db).await?;
    Ok(claim)
}

async fn reload(state: &AppState, id: i64) -> Result<ExpenseClaim> {
    ExpenseClaim::find_or_404(&state.db, id).await
}

/// `user` (the claimant) submits the draft claim `id`. The drawings of its
/// lines are asked for as one drawing first (see the module documentation):
/// the claim then waits for it, or has it already when the claimant is the
/// entity's sole owner.
///
/// # Errors
///
/// `Forbidden` without `expenses.submit` or for another user's claim;
/// `NotFrom` unless the claim is a draft; `Guard` (422) for a claim without
/// lines; the refusals of the drawing request.
pub async fn submit(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<ExpenseClaim, TransitionError> {
    let claim = ExpenseClaim::find_or_404(&state.db, id).await?;
    if !user.has_permission(permissions::EXPENSES_SUBMIT) || claim.claimant_id != user.id {
        return Err(TransitionError::Forbidden);
    }
    if claim.status != ClaimStatus::Draft {
        return Err(TransitionError::NotFrom(claim.status.as_str().to_owned()));
    }
    let lines = claim.lines(&state.db).await?;
    if lines.is_empty() {
        return Err(guard(english(
            "wl_payments.expense_claims.problem.no_lines",
        )));
    }
    let mut given = SubmitInput {
        total: lines.iter().map(|l| l.amount).sum(),
        vat_total: lines.iter().map(|l| l.vat_amount).sum(),
        ..Default::default()
    };
    // The drawings of the claim: lines in an owner drawings category.
    let mut category = None;
    for line in &lines {
        if let Some(found) = ExpenseCategory::by_code(&state.db, &line.category).await?
            && found.is_owner_drawings()
        {
            given.drawing_amount += line.amount;
            category = category.or(found.budget_category.clone());
        }
    }
    if given.drawing_amount > 0 {
        let code = category.unwrap_or_default();
        let started = owner_drawings(state)
            .request(
                state,
                DrawingRequest {
                    legal_entity_id: claim.legal_entity_id,
                    category: code,
                    source: claim.subject(),
                    amount: given.drawing_amount,
                    requested_by: user.id,
                },
            )
            .await?;
        match started {
            DrawingStart::Pending => given.drawing = Some(ClaimDrawing::Requested),
            DrawingStart::Approved { self_approved } => {
                given.drawing = Some(ClaimDrawing::Approved);
                given.self_approved = self_approved;
                given.approved_by = self_approved.then_some(user.id);
            }
        }
    }
    let _outcome = claim
        .transition(SUBMIT)
        .by(user)
        .input(&given)
        .run(state)
        .await?;
    Ok(reload(state, id).await?)
}

/// `user` approves the submitted claim `id`.
///
/// # Errors
///
/// `Forbidden` without `expenses.approve`; `Guard` (422) for the claimant
/// (`FIN.ACL.11`) and for a drawing that is not approved (`FIN.ACL.21`).
pub async fn approve(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<ExpenseClaim, TransitionError> {
    let claim = ExpenseClaim::find_or_404(&state.db, id).await?;
    let _outcome = claim.transition(APPROVE).by(user).run(state).await?;
    Ok(reload(state, id).await?)
}

/// `user` rejects the submitted claim `id` with `reason`.
///
/// # Errors
///
/// `Forbidden` without `expenses.approve`; `Guard` (422) without a reason.
pub async fn reject(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> std::result::Result<ExpenseClaim, TransitionError> {
    let claim = ExpenseClaim::find_or_404(&state.db, id).await?;
    let _outcome = claim
        .transition(REJECT)
        .by(user)
        .reason(reason.to_owned())
        .run(state)
        .await?;
    Ok(reload(state, id).await?)
}

/// `user` returns the submitted claim `id` to `draft` with `reason`.
///
/// # Errors
///
/// `Forbidden` without `expenses.approve`; `Guard` (422) without a reason.
pub async fn return_to_draft(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> std::result::Result<ExpenseClaim, TransitionError> {
    let claim = ExpenseClaim::find_or_404(&state.db, id).await?;
    let _outcome = claim
        .transition(RETURN)
        .by(user)
        .reason(reason.to_owned())
        .run(state)
        .await?;
    Ok(reload(state, id).await?)
}

/// The decision on the drawing of a claim is announced: the claim keeps the
/// approval (and who gave it), or the reason it was refused. A claim whose
/// drawing is no longer `requested` (a repeated delivery) is left as it is.
///
/// # Errors
///
/// A database error, or 422 when the claimant would approve their own drawing.
pub async fn decided(state: &AppState, event: &OwnerDrawingDecided) -> Result<()> {
    if event.source.subject_type != EXPENSE_CLAIM_SUBJECT_TYPE {
        return Ok(());
    }
    let Some(mut claim) = ExpenseClaim::find(&state.db, event.source.subject_id).await? else {
        return Ok(());
    };
    if claim.drawing_status != Some(ClaimDrawing::Requested) {
        return Ok(());
    }
    if event.approved {
        // `FIN.ACL.21`: the claimant never approves their own drawing (the
        // sole owner's own approval is `DrawingStart::Approved`, not this).
        if event.approved_by == Some(claim.claimant_id) {
            return Err(refused(english(
                "wl_payments.expense_claims.problem.own_drawing",
            )));
        }
        claim.drawing_status = Some(ClaimDrawing::Approved);
        claim.drawing_approved_by = event.approved_by;
        claim.drawing_reason = None;
    } else {
        claim.drawing_status = Some(ClaimDrawing::Rejected);
        claim.drawing_reason = event.reason.clone();
    }
    claim.save(&state.db).await?;
    Ok(())
}

/// Registers the listener that follows the decision on a claim's drawing.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: OwnerDrawingDecided, state: AppState| async move {
        decided(&state, &event).await
    });
}

/// The total of `claim` as text.
pub(crate) fn total_text(claim: &ExpenseClaim) -> String {
    money_text("AED", claim.total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(drawing: i64, status: Option<ClaimDrawing>) -> ExpenseClaim {
        ExpenseClaim {
            drawing_amount: drawing,
            drawing_status: status,
            drawing_reason: Some("personal".into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_claim_without_a_drawing_has_nothing_to_wait_for() {
        assert!(drawing_problem(&claim(0, None)).is_none());
    }

    #[test]
    fn a_drawing_holds_the_claim_back_until_it_is_approved() {
        assert!(drawing_problem(&claim(200_000, Some(ClaimDrawing::Approved))).is_none());
        let waits = drawing_problem(&claim(200_000, Some(ClaimDrawing::Requested))).unwrap();
        assert!(waits.contains("waits"), "{waits}");
        let rejected = drawing_problem(&claim(200_000, Some(ClaimDrawing::Rejected))).unwrap();
        assert!(rejected.contains("personal"), "{rejected}");
    }

    #[test]
    fn the_states_have_tones_and_the_machine_declares_four_transitions() {
        let machine = ExpenseClaim::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        for terminal in ["approved", "rejected"] {
            let state = machine.states().iter().find(|s| s.name == terminal);
            assert!(state.is_some_and(|s| s.terminal), "{terminal}");
        }
    }
}
