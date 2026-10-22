//! `approve` and `pay` of a final settlement (wl-hr §4 *Final settlement*,
//! `HRM.ACL.04`, `FIN.PAY.01`, issues #212, #213).
//!
//! - **`approve`** (draft -> approved, `settlements.approve`): the approver is
//!   not the preparer who entered the amounts (`HRM.ACL.04`). The offset
//!   within the caps is worked out again and the settlement statement
//!   (`DOC.GEN.01`) gets its version; the statement document itself is made by
//!   wl-documents from the settlement, there is no contract for it yet.
//! - **`pay`** (approved -> paid, `settlements.pay`): the net still unpaid is
//!   recorded with the transition (through WPS or a bank transfer, optional
//!   when the earlier payments already cover the net); or nothing is payable
//!   (net 0). For the payee *estate* the representative and the document
//!   giving the right must be recorded (`⚠ open: lawyer (L-25)`). Recoveries
//!   above the caps become a receivable from the employee
//!   ([`crate::settlement_offset::sync_receivable_in`]), and an acknowledgement
//!   is requested for signature (not for the estate, `DOC.SIG.01`). The task
//!   for the accountant, the event `SettlementPaid` and the end of the period
//!   to dispute follow after the commit ([`crate::settlement_events`]).

use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionError};
use wl_kernel::documents::today;

use crate::permissions;
use crate::settlement::{APPROVE, FinalSettlement, PAY, PAYEE_ESTATE, SettlementStatus, refuse};
use crate::settlement_offset::{recalculate_in, sync_receivable_in};
use crate::settlement_payment::{PaymentInput, insert_in, paid_total_in};
use crate::texts::english;

/// `approve`: another user than the preparer.
pub(crate) fn approve() -> Transition<FinalSettlement> {
    Transition::new(APPROVE)
        .label("Approve")
        .tone("primary")
        .permission(permissions::SETTLEMENTS_APPROVE)
        .from([SettlementStatus::Draft])
        .to(SettlementStatus::Approved)
        .guard("not_the_preparer", |settlement: &FinalSettlement, ctx| {
            let same = matches!(
                (ctx.actor(), settlement.prepared_by),
                (Some(user), Some(preparer)) if user.id == preparer
            );
            Box::pin(async move {
                if same {
                    Err(english("wl_hr.settlement.same_user", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|settlement: &FinalSettlement, ctx, tx| {
            let id = settlement.id;
            let version = settlement.statement_version + 1;
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                recalculate_in(tx, id).await?;
                FinalSettlement::query()
                    .where_(FinalSettlement::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("approved_by", &by),
                            ("statement_version", &version),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `pay`: the net is covered by the payments, or nothing is payable.
pub(crate) fn pay() -> Transition<FinalSettlement> {
    Transition::new(PAY)
        .label("Pay")
        .tone("primary")
        .permission(permissions::SETTLEMENTS_PAY)
        .from([SettlementStatus::Approved])
        .to(SettlementStatus::Paid)
        .guard("payee_documented", |settlement: &FinalSettlement, _ctx| {
            let missing = settlement.payee == PAYEE_ESTATE
                && [&settlement.representative, &settlement.right_document]
                    .iter()
                    .any(|field| field.as_deref().is_none_or(|text| text.trim().is_empty()));
            Box::pin(async move {
                if missing {
                    Err(english("wl_hr.settlement.estate_payee_required", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|settlement: &FinalSettlement, ctx, tx| {
            let id = settlement.id;
            let estate = settlement.payee == PAYEE_ESTATE;
            let by = ctx.actor().map(|user| user.id);
            let input = ctx.input_as::<PaymentInput>().unwrap_or_default();
            Box::pin(async move {
                // The recoveries may have grown since `approve` (a
                // supplementary run): pay the net as it stands now.
                let current = recalculate_in(tx, id).await?;
                if input.amount > 0 {
                    insert_in(tx, &current, &input, by).await?;
                }
                if paid_total_in(tx, id).await? < current.net {
                    return Err(refuse("wl_hr.settlement.net_unpaid"));
                }
                let day = today();
                let requested = (!estate).then_some(day);
                FinalSettlement::query()
                    .where_(FinalSettlement::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("paid_on", &Some(day)),
                            ("paid_by", &by),
                            ("acknowledgement_requested_on", &requested),
                            (
                                "acknowledgement_signed_on",
                                &None::<renox::chrono::NaiveDate>,
                            ),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                sync_receivable_in(tx, id).await?;
                Ok(())
            })
        })
}

/// `by` (a holder of `settlements.pay`) pays settlement `id`: `input` is the
/// rest of the net, recorded with the transition (an amount of 0 when the
/// earlier payments cover it).
///
/// # Errors
///
/// 403 without the permission; 422 when the settlement is not `approved`, the
/// net is not covered, or the payee *estate* lacks its representative and the
/// document giving the right.
pub async fn pay_settlement(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    input: PaymentInput,
) -> Result<FinalSettlement> {
    let settlement = FinalSettlement::find_or_404(&state.db, id).await?;
    let _outcome = settlement
        .transition(PAY)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    FinalSettlement::find_or_404(&state.db, id).await
}
