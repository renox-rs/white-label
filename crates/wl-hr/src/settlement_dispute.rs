//! Disputing and resolving a final settlement (wl-hr §3 *Final settlement*:
//! dispute, §4, §5.4 step 6, `HRM.ACL.04`, issue #212).
//!
//! - **`dispute`** (approved, paid -> disputed, `settlements.prepare`): the
//!   worker contests the amounts, or a MoHRE complaint or a court case is
//!   open. The source (`worker`, `mohre_complaint`, `court`), the reference
//!   and the undisputed amount are given. After the commit a labour-case
//!   ticket opens with the hearing or reply dates, HR, the accountant and the
//!   MD are told and `SettlementDisputed` follows ([`crate::settlement_events`]).
//!   The undisputed amount is paid as a payment against the settlement
//!   ([`crate::settlement_payment::record`]); the state stays `disputed`.
//!   A second dispute after a resolution is allowed from `approved` or `paid`.
//! - **Revised amounts:** while `disputed`, a `settlements.prepare` user enters
//!   the recalculated figures ([`crate::settlement_amounts::set_amounts`]).
//! - **`resolve`** (disputed -> approved, `settlements.approve`): the outcome
//!   (`upheld`, `revised`) and the document of the decision (the MoHRE
//!   decision, the agreement or the judgment) are given. `revised` must be run
//!   by a user other than the one who entered the revised amounts
//!   (`HRM.ACL.04`); it is the second-user approval, there is no separate
//!   `approve`. The settlement gets a new statement and the difference is paid
//!   with `pay`; a revised net below what was already paid makes the excess a
//!   receivable from the employee ([`crate::settlement_offset`]). `upheld`
//!   goes back to `approved` when the net is not yet paid in full.
//! - **`resolve_closed`** (disputed -> closed): the `resolve` of an `upheld`
//!   outcome when the net is paid in full or nothing is payable. A transition
//!   has one fixed target state, so the one `resolve` of the spec is these two;
//!   [`resolve`] picks the right one.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::permissions;
use crate::settlement::{
    DISPUTE, FinalSettlement, RESOLVE, RESOLVE_CLOSED, SettlementStatus, refuse,
};
use crate::settlement_offset::{recalculate_in, sync_receivable_in};
use crate::settlement_payment::{paid_total, paid_total_in};
use crate::texts::english;

/// The worker contests the amounts.
pub const SOURCE_WORKER: &str = "worker";
/// A MoHRE complaint is open.
pub const SOURCE_MOHRE: &str = "mohre_complaint";
/// A court case is open.
pub const SOURCE_COURT: &str = "court";
/// The sources of a dispute.
pub const SOURCES: [&str; 3] = [SOURCE_WORKER, SOURCE_MOHRE, SOURCE_COURT];

/// The decision stands: the amounts are not changed.
pub const UPHELD: &str = "upheld";
/// The amounts are recalculated.
pub const REVISED: &str = "revised";

/// What `dispute` carries.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct DisputeInput {
    /// [`SOURCE_WORKER`], [`SOURCE_MOHRE`] or [`SOURCE_COURT`].
    pub source: String,
    /// The complaint or case reference.
    pub reference: String,
    /// The undisputed amount, AED fils; `None` when it was not entered.
    pub undisputed_amount: Option<i64>,
    /// The hearing date.
    pub hearing_on: Option<NaiveDate>,
    /// The date a reply is due.
    pub reply_due_on: Option<NaiveDate>,
}

/// What `resolve` carries.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct ResolveInput {
    /// [`UPHELD`] or [`REVISED`].
    pub outcome: String,
    /// The decision, agreement or judgment: a reference to the stored
    /// document.
    pub document: String,
}

/// `dispute`: source, reference and the undisputed amount (0 up to the net).
pub(crate) fn dispute() -> Transition<FinalSettlement> {
    Transition::new(DISPUTE)
        .label("Dispute")
        .tone("danger")
        .permission(permissions::SETTLEMENTS_PREPARE)
        .from([SettlementStatus::Approved, SettlementStatus::Paid])
        .to(SettlementStatus::Disputed)
        .guard("source_and_reference", |_s: &FinalSettlement, ctx| {
            let input = ctx.input_as::<DisputeInput>().unwrap_or_default();
            let given =
                SOURCES.contains(&input.source.trim()) && !input.reference.trim().is_empty();
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("wl_hr.settlement.dispute_source_required", &[]))
                }
            })
        })
        .guard("undisputed_amount", |s: &FinalSettlement, ctx| {
            let input = ctx.input_as::<DisputeInput>().unwrap_or_default();
            let net = s.net;
            let valid = input
                .undisputed_amount
                .is_some_and(|amount| (0..=net).contains(&amount));
            Box::pin(async move {
                if valid {
                    Ok(())
                } else {
                    Err(english("wl_hr.settlement.dispute_undisputed_required", &[]))
                }
            })
        })
        .effect(|s: &FinalSettlement, ctx, tx| {
            let id = s.id;
            let from = s.status.as_str().to_owned();
            let by = ctx.actor().map(|user| user.id);
            let input = ctx.input_as::<DisputeInput>().unwrap_or_default();
            Box::pin(async move {
                FinalSettlement::query()
                    .where_(FinalSettlement::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("dispute_source", &Some(input.source.trim().to_owned())),
                            (
                                "dispute_reference",
                                &Some(input.reference.trim().to_owned()),
                            ),
                            ("undisputed_amount", &input.undisputed_amount.unwrap_or(0)),
                            ("hearing_on", &input.hearing_on),
                            ("reply_due_on", &input.reply_due_on),
                            ("dispute_ticket", &None::<i64>),
                            ("disputed_from", &Some(from)),
                            ("disputed_by", &by),
                            ("revised_by", &None::<i64>),
                            ("dispute_outcome", &None::<String>),
                            ("resolution_document", &None::<String>),
                            ("resolved_by", &None::<i64>),
                            ("closes_on", &None::<NaiveDate>),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `resolve` (back to `approved`) or `resolve_closed` (`closing`).
pub(crate) fn resolve(closing: bool) -> Transition<FinalSettlement> {
    let (name, to) = if closing {
        (RESOLVE_CLOSED, SettlementStatus::Closed)
    } else {
        (RESOLVE, SettlementStatus::Approved)
    };
    Transition::new(name)
        .label("Resolve")
        .tone("primary")
        .permission(permissions::SETTLEMENTS_APPROVE)
        .from([SettlementStatus::Disputed])
        .to(to)
        .guard("outcome_and_document", |_s: &FinalSettlement, ctx| {
            let input = ctx.input_as::<ResolveInput>().unwrap_or_default();
            let key = if ![UPHELD, REVISED].contains(&input.outcome.trim()) {
                Some("wl_hr.settlement.resolve_outcome_required")
            } else if input.document.trim().is_empty() {
                Some("wl_hr.settlement.resolve_document_required")
            } else {
                None
            };
            Box::pin(async move { key.map_or(Ok(()), |key| Err(english(key, &[]))) })
        })
        .guard("not_the_reviser", |s: &FinalSettlement, ctx| {
            let input = ctx.input_as::<ResolveInput>().unwrap_or_default();
            let revised = input.outcome.trim() == REVISED;
            let key = match (revised, s.revised_by, ctx.actor()) {
                (false, _, _) => None,
                (true, None, _) => Some("wl_hr.settlement.revised_amounts_required"),
                (true, Some(reviser), Some(user)) if user.id == reviser => {
                    Some("wl_hr.settlement.same_user")
                }
                _ => None,
            };
            Box::pin(async move { key.map_or(Ok(()), |key| Err(english(key, &[]))) })
        })
        .effect(move |s: &FinalSettlement, ctx, tx| {
            let id = s.id;
            let version = s.statement_version + 1;
            let by = ctx.actor().map(|user| user.id);
            let input = ctx.input_as::<ResolveInput>().unwrap_or_default();
            Box::pin(async move {
                let revised = input.outcome.trim() == REVISED;
                let paid = paid_total_in(tx, id).await?;
                if revised {
                    if closing {
                        return Err(refuse("wl_hr.settlement.resolve_revised_approves"));
                    }
                    let current = recalculate_in(tx, id).await?;
                    FinalSettlement::query()
                        .where_(FinalSettlement::ID.eq(id))
                        .update(
                            &mut *tx,
                            &[
                                ("approved_by", &by),
                                ("statement_version", &version),
                                ("overpaid", &(paid - current.net).max(0)),
                            ],
                        )
                        .await?;
                    sync_receivable_in(tx, id).await?;
                } else {
                    let current = recalculate_in(tx, id).await?;
                    let in_full = paid >= current.net;
                    if closing && !in_full {
                        return Err(refuse("wl_hr.settlement.resolve_unpaid_net"));
                    }
                    if !closing && in_full {
                        return Err(refuse("wl_hr.settlement.resolve_closes"));
                    }
                }
                let closed = closing.then(today);
                FinalSettlement::query()
                    .where_(FinalSettlement::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("dispute_outcome", &Some(input.outcome.trim().to_owned())),
                            (
                                "resolution_document",
                                &Some(input.document.trim().to_owned()),
                            ),
                            ("resolved_by", &by),
                            ("closed_on", &closed),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `by` (a holder of `settlements.prepare`) disputes settlement `id`.
///
/// # Errors
///
/// 403 without the permission; 422 when the settlement is not `approved` or
/// `paid`, or the source, reference or undisputed amount is missing.
pub async fn dispute_settlement(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    input: DisputeInput,
) -> Result<FinalSettlement> {
    let settlement = FinalSettlement::find_or_404(&state.db, id).await?;
    let _outcome = settlement
        .transition(DISPUTE)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    FinalSettlement::find_or_404(&state.db, id).await
}

/// `by` (a holder of `settlements.approve`) resolves the dispute of settlement
/// `id`: `closed` for an `upheld` outcome with the net paid in full (or
/// nothing payable), else `approved`.
///
/// # Errors
///
/// 403 without the permission; 422 when the settlement is not `disputed`, the
/// outcome or document is missing, or `revised` is run by the user who entered
/// the revised amounts or before any were entered.
pub async fn resolve_dispute(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    input: ResolveInput,
) -> Result<FinalSettlement> {
    let settlement = FinalSettlement::find_or_404(&state.db, id).await?;
    if settlement.status != SettlementStatus::Disputed {
        return Err(refuse("wl_hr.settlement.not_disputed"));
    }
    let paid = paid_total(&state.db, id).await?;
    let name = if input.outcome.trim() == UPHELD && paid >= settlement.net {
        RESOLVE_CLOSED
    } else {
        RESOLVE
    };
    let _outcome = settlement
        .transition(name)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    FinalSettlement::find_or_404(&state.db, id).await
}
