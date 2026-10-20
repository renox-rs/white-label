//! Cancelling a draft Return to supplier, and crediting a dispatched one
//! (wl-inventory §4 *Return to supplier* `cancel` and `credit`, `STK.ACL.02`,
//! issues #224 to #226).
//!
//! `cancel` (a holder of `supplier_returns.manage`, with a reason) moves a
//! `draft` return to `cancelled`: it is not sent and the goods stay. The stock
//! lines it held aside (see [`crate::supplier_return::reserved_milli`]) are
//! free again as the return leaves `draft`. Goods rejected at receipt and kept
//! for collection are, as the user chooses, *received again* (a draft goods
//! receipt on the purchase order is opened) or *discarded* (no movement: their
//! value never entered stock). The system's `cancel_with_receipt` withdraws the
//! draft returns of a receipt that was reversed.
//!
//! Decisions: the choice for the kept goods is asked only when the return has
//! lines rejected at receipt; the draft receipt of *received again* is opened
//! after the commit, on the order of each such line, with the rejected
//! quantity as received and accepted for the storekeeper to correct.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};

use crate::goods_receipt::{DraftLine, DraftReceipt};
use crate::permissions;
use crate::purchase_order::PurchaseOrderLine;
use crate::supplier_return::{self, ReturnOrigin, ReturnStatus, SupplierReturn};
use crate::texts::said;

/// The user's `cancel` transition's name.
pub const CANCEL: &str = "cancel";

/// The system's `cancel_with_receipt` transition's name: a reversed goods
/// receipt withdraws the draft returns made from it.
pub const CANCEL_WITH_RECEIPT: &str = "cancel_with_receipt";

/// What the user does with rejected goods kept for collection when the
/// return that waited for the supplier is cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeptGoods {
    /// Receive them on a new goods receipt on the purchase order.
    ReceiveAgain,
    /// Discard them: no movement, the value never entered stock.
    Discard,
}

impl KeptGoods {
    /// The choice as the return keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            KeptGoods::ReceiveAgain => "received_again",
            KeptGoods::Discard => "discarded",
        }
    }

    /// The choice named `text` (`receive_again` or `discard`).
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "receive_again" => Some(KeptGoods::ReceiveAgain),
            "discard" => Some(KeptGoods::Discard),
            _ => None,
        }
    }
}

/// What a `cancel` run carries to its guard and effect.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CancelInput {
    /// The user who cancels.
    pub by: Option<i64>,
    /// The choice for the kept goods.
    pub kept: Option<KeptGoods>,
    /// The goods rejected at receipt need a choice and none was made.
    pub kept_problem: Option<String>,
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

fn cancel_input(ctx: &TransitionCtx) -> Option<CancelInput> {
    ctx.input_as::<CancelInput>().ok()
}

/// `cancel` and `cancel_with_receipt`.
fn cancel_transitions() -> Vec<Transition<SupplierReturn>> {
    let user = Transition::new(CANCEL)
        .label("Cancel")
        .tone("danger")
        .permission(permissions::SUPPLIER_RETURNS_MANAGE)
        .reason_required()
        .from([ReturnStatus::Draft])
        .to(ReturnStatus::Cancelled)
        .guard("kept_goods_decided", |_ret: &SupplierReturn, ctx| {
            let problem = cancel_input(ctx).map_or_else(
                || Some(said("wl_inventory.return.cancel_draft_only", &[])),
                |input| input.kept_problem,
            );
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|ret: &SupplierReturn, ctx, tx| {
            let id = ret.id;
            let input = cancel_input(ctx).unwrap_or_default();
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move { cancel_in(tx, id, &input, reason).await })
        });
    let system = Transition::new(CANCEL_WITH_RECEIPT)
        .label("Cancel")
        .system()
        .from([ReturnStatus::Draft])
        .to(ReturnStatus::Cancelled);
    vec![user, system]
}

/// The effect of `cancel`: who, when, why, and the choice for the kept goods.
async fn cancel_in(
    tx: &mut Transaction,
    id: i64,
    input: &CancelInput,
    reason: Option<String>,
) -> Result {
    let now = renox::db::now();
    let kept = input.kept.map(|kept| kept.as_str().to_owned());
    SupplierReturn::query()
        .where_(SupplierReturn::ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("cancel_reason", &reason),
                ("cancelled_by", &input.by),
                ("cancelled_at", &Some(now)),
                ("kept_goods", &kept),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// The transitions of the return that this module owns.
pub(crate) fn add_to(machine: Machine<SupplierReturn>) -> Machine<SupplierReturn> {
    cancel_transitions()
        .into_iter()
        .fold(machine, Machine::transition)
}

/// `user` cancels draft return `return_id` for `reason`. When the return has
/// goods rejected at receipt (kept for collection) `kept` says what becomes of
/// them; for *received again* a draft goods receipt is opened on each
/// purchase order concerned.
///
/// # Errors
///
/// `Forbidden` without `supplier_returns.manage`; `Guard` (422) when the
/// return is not a draft, the reason is empty or the choice is missing.
pub async fn cancel(
    state: &AppState,
    user: &AuthUser,
    return_id: i64,
    reason: &str,
    kept: Option<KeptGoods>,
) -> std::result::Result<SupplierReturn, TransitionError> {
    let ret = SupplierReturn::find_or_404(&state.db, return_id).await?;
    if ret.status != ReturnStatus::Draft {
        return Err(TransitionError::Guard(said(
            "wl_inventory.return.cancel_draft_only",
            &[],
        )));
    }
    let lines = supplier_return::lines_of(&state.db, ret.id).await?;
    let kept_lines = lines
        .iter()
        .any(|line| line.origin == ReturnOrigin::RejectedAtReceipt);
    let input = CancelInput {
        by: Some(user.id),
        kept,
        kept_problem: (kept_lines && kept.is_none())
            .then(|| said("wl_inventory.return.kept_goods_needed", &[])),
    };
    if reason.trim().is_empty() {
        return Err(TransitionError::ReasonRequired);
    }
    let outcome = ret
        .transition(CANCEL)
        .by(user)
        .reason(reason.trim())
        .input(&input)
        .run(state)
        .await?;
    outcome.emit(state).await?;
    if kept == Some(KeptGoods::ReceiveAgain) {
        reopen_receipts(state, &ret, user.id).await?;
    }
    Ok(SupplierReturn::find_or_404(&state.db, ret.id).await?)
}

/// Opens a draft goods receipt on each purchase order the kept goods of the
/// cancelled `ret` were rejected on, with their quantity.
async fn reopen_receipts(state: &AppState, ret: &SupplierReturn, by: i64) -> Result {
    let Some(location_id) = ret.location_id else {
        return Ok(());
    };
    let lines = supplier_return::lines_of(&state.db, ret.id).await?;
    let mut orders: Vec<(i64, Vec<DraftLine>)> = Vec::new();
    for line in lines
        .iter()
        .filter(|line| line.origin == ReturnOrigin::RejectedAtReceipt)
    {
        let Some(po_line_id) = line.po_line_id else {
            continue;
        };
        let po_line = PurchaseOrderLine::find_or_404(&state.db, po_line_id).await?;
        let draft = DraftLine::on(po_line.id, line.quantity_milli, line.quantity_milli);
        match orders
            .iter_mut()
            .find(|(order_id, _)| *order_id == po_line.purchase_order_id)
        {
            Some((_, drafts)) => drafts.push(draft),
            None => orders.push((po_line.purchase_order_id, vec![draft])),
        }
    }
    for (purchase_order_id, lines) in orders {
        crate::goods_receipt::create_draft(
            &state.db,
            Some(by),
            DraftReceipt {
                purchase_order_id,
                location_id,
                delivery_note: None,
                lines,
            },
        )
        .await?;
    }
    Ok(())
}
