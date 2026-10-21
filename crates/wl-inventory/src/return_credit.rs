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
//! `credit` ends a `dispatched` return as `credited`, in two ways:
//!
//! - the system's `credit_covered`, when the supplier's debit notes
//!   (`SupplierDebitNoteIssued`, which names what its lines debit) cover every
//!   `credit` line and replacement receipts cover every `replace` line.
//!   Coverage is kept per line (`covered_milli`); a note that covers some
//!   lines, or part of one, leaves the return `dispatched`. A debit note is
//!   matched to a line by the purchase order line (a line without one by the
//!   notes' lines without one), in the order of the lines; a note whose event
//!   names no line covers nothing. `SupplierDebitNoteCancelled` takes its
//!   quantities off the lines of a `dispatched` return again and gives
//!   procurement a task; a `credited` return stays so (terminal);
//! - a holder of `supplier_returns.manage` settles it by hand ([`credit`])
//!   with a basis and a reason: *refund received* (the money came in through
//!   wl-payments) or *supplier refused*. A refused credit is a loss: the value
//!   of what no note or replacement covered is posted as wastage through the
//!   `CostPoster` after the commit (the goods rejected at receipt never had
//!   stock, and the stock lines already left with `return_out`, so there is no
//!   movement), and above the value (default AED 1,000, `STK.ACL.02`) the
//!   transition becomes an approval request for a second holder of the
//!   permission, never the requester.
//!
//! `SupplierReturnCredited` follows each of the three ([`crate::events`]).
//!
//! Decisions: the choice for the kept goods is asked only when the return has
//! lines rejected at receipt; the draft receipt of *received again* is opened
//! after the commit, on the order of each such line, with the rejected
//! quantity as received and accepted for the storekeeper to correct.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{
    Machine, Outcome, StateMachine, Transition, TransitionCtx, TransitionError,
};
use serde::{Deserialize, Serialize};

use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::inventory::SUPPLIER_RETURN_SUBJECT_TYPE;
use wl_kernel::invoicing::{SupplierDebitNoteCancelled, SupplierDebitNoteIssued};
use wl_kernel::role_tasks::{self, RoleTask};

use crate::goods_receipt::{DraftLine, DraftReceipt};
use crate::item::value_of;
use crate::permissions;
use crate::purchase_order::PurchaseOrderLine;
use crate::supplier_return::{
    self, ReturnOrigin, ReturnStatus, SupplierReturn, SupplierReturnLine, WantedOutcome,
};
use crate::texts::said;

/// The user's `cancel` transition's name.
pub const CANCEL: &str = "cancel";

/// The system's `cancel_with_receipt` transition's name: a reversed goods
/// receipt withdraws the draft returns made from it.
pub const CANCEL_WITH_RECEIPT: &str = "cancel_with_receipt";

/// The user's `credit` transition's name.
pub const CREDIT: &str = "credit";

/// The system's `credit_approved` transition's name: the kernel runs it when
/// the approval of a refused credit above the value is given.
pub const CREDIT_APPROVED: &str = "credit_approved";

/// The system's `credit_covered` transition's name: the debit notes and the
/// replacement receipts cover every line.
pub const CREDIT_COVERED: &str = "credit_covered";

/// The approval policy's action key of `credit` (`STK.ACL.02`).
pub const CREDIT_ACTION: &str = "inventory.supplier_return_credit";

/// The role that gets the tasks about returns (procurement).
pub const PROCUREMENT_ROLE: &str = "procurement";

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
        .chain(credit_transitions())
        .fold(machine, Machine::transition)
}

/// How a manual credit is settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreditBasis {
    /// The supplier's refund came in (wl-payments).
    RefundReceived,
    /// The supplier refused the credit: the uncovered value is a loss.
    SupplierRefused,
}

impl CreditBasis {
    /// The basis as the return keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            CreditBasis::RefundReceived => "refund_received",
            CreditBasis::SupplierRefused => "supplier_refused",
        }
    }

    /// The basis named `text` (`refund_received` or `supplier_refused`).
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "refund_received" => Some(CreditBasis::RefundReceived),
            "supplier_refused" => Some(CreditBasis::SupplierRefused),
            _ => None,
        }
    }
}

/// What a `credit` run carries to its guard and effect.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CreditInput {
    /// The wastage value in fils: what the approval policy's condition reads
    /// (`STK.ACL.02`); 0 unless the supplier refused.
    pub amount: i64,
    /// The user who credits it (who asked, when it waited for approval).
    pub by: Option<i64>,
    /// How: `debit_note`, `replacement`, `refund_received`, `supplier_refused`.
    pub basis: String,
    /// The reason of a manual credit.
    pub reason: Option<String>,
    /// The debit note that completed the cover.
    pub debit_note_id: Option<i64>,
    /// The first thing wrong, if anything is.
    pub problem: Option<String>,
}

fn credit_input(ctx: &TransitionCtx) -> Option<CreditInput> {
    ctx.input_as::<CreditInput>().ok()
}

/// The shared guard and effect of the three credit transitions.
fn credit_rules(transition: Transition<SupplierReturn>) -> Transition<SupplierReturn> {
    transition
        .from([ReturnStatus::Dispatched])
        .to(ReturnStatus::Credited)
        .guard("settled", |_ret: &SupplierReturn, ctx| {
            let problem = credit_input(ctx).map_or_else(
                || Some(said("wl_inventory.return.credit_dispatched_only", &[])),
                |input| input.problem,
            );
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|ret: &SupplierReturn, ctx, tx| {
            let id = ret.id;
            let input = credit_input(ctx).unwrap_or_default();
            Box::pin(async move { credit_in(tx, id, &input).await })
        })
}

/// `credit`, the system's `credit_approved` and `credit_covered`.
fn credit_transitions() -> Vec<Transition<SupplierReturn>> {
    vec![
        credit_rules(
            Transition::new(CREDIT)
                .label("Credit")
                .tone("primary")
                .permission(permissions::SUPPLIER_RETURNS_MANAGE),
        ),
        credit_rules(
            Transition::new(CREDIT_APPROVED)
                .label("Credit (approved)")
                .system(),
        ),
        credit_rules(Transition::new(CREDIT_COVERED).label("Credited").system()),
    ]
}

/// The effect of the three credit transitions: how, why, by whom, the debit
/// note and the wastage value, on the return.
async fn credit_in(tx: &mut Transaction, id: i64, input: &CreditInput) -> Result {
    let now = renox::db::now();
    SupplierReturn::query()
        .where_(SupplierReturn::ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("credit_basis", &Some(input.basis.clone())),
                ("credit_reason", &input.reason),
                ("credited_by", &input.by),
                ("credited_at", &Some(now)),
                ("debit_note_id", &input.debit_note_id),
                ("wastage_fils", &input.amount),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// What is not covered of `lines` yet, in fils: the value of the quantity no
/// debit note or replacement receipt took.
pub fn uncovered_value(lines: &[SupplierReturnLine]) -> i64 {
    lines
        .iter()
        .map(|line| {
            value_of(
                (line.quantity_milli - line.covered_milli).max(0),
                line.unit_cost,
            )
        })
        .sum()
}

/// Whether every line is covered.
pub fn all_covered(lines: &[SupplierReturnLine]) -> bool {
    lines
        .iter()
        .all(|line| line.covered_milli >= line.quantity_milli)
}

/// Runs the system's `credit_covered` in `tx` when every line of `ret` is
/// covered, `basis` being `debit_note` or `replacement`. The caller emits the
/// outcome after the commit.
///
/// # Errors
///
/// A database error, or a refusal of the transition.
pub(crate) async fn credit_if_covered(
    tx: &mut Transaction,
    ret: &SupplierReturn,
    basis: &str,
    debit_note_id: Option<i64>,
) -> Result<Option<Outcome<SupplierReturn>>> {
    let lines = supplier_return::lines_in(tx, ret.id).await?;
    if lines.is_empty() || !all_covered(&lines) {
        return Ok(None);
    }
    let input = CreditInput {
        basis: basis.to_owned(),
        debit_note_id,
        ..CreditInput::default()
    };
    let outcome = ret
        .transition(CREDIT_COVERED)
        .by_system()
        .input(&input)
        .run_in(&mut *tx)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(Some(outcome))
}

/// What [`credit`] did.
#[derive(Debug, Clone)]
pub enum CreditOutcome {
    /// The return is `credited`.
    Credited(SupplierReturn),
    /// The refused value is above the limit: the return stays `dispatched`
    /// and waits for a second person's approval (`STK.ACL.02`).
    Waiting(ApprovalRequest),
}

/// `user` settles dispatched return `return_id` by hand: `basis` and a
/// `reason`. When the supplier refused, the value no note or replacement
/// covered is the wastage; above the limit it becomes an approval request.
///
/// # Errors
///
/// `Forbidden` without `supplier_returns.manage`; `Guard` (422) when the
/// return is not `dispatched` or the reason is empty.
pub async fn credit(
    state: &AppState,
    user: &AuthUser,
    return_id: i64,
    basis: CreditBasis,
    reason: &str,
) -> std::result::Result<CreditOutcome, TransitionError> {
    let ret = SupplierReturn::find_or_404(&state.db, return_id).await?;
    let lines = supplier_return::lines_of(&state.db, ret.id).await?;
    let problem = if ret.status != ReturnStatus::Dispatched {
        Some(said("wl_inventory.return.credit_dispatched_only", &[]))
    } else if reason.trim().is_empty() {
        Some(said("wl_inventory.return.reason_needed", &[]))
    } else {
        None
    };
    if let Some(problem) = problem {
        return Err(TransitionError::Guard(problem));
    }
    let input = CreditInput {
        amount: match basis {
            CreditBasis::SupplierRefused => uncovered_value(&lines),
            CreditBasis::RefundReceived => 0,
        },
        by: Some(user.id),
        basis: basis.as_str().to_owned(),
        reason: Some(reason.trim().to_owned()),
        ..CreditInput::default()
    };
    let outcome = ret
        .transition(CREDIT)
        .by(user)
        .reason(reason.trim())
        .input(&input)
        .run(state)
        .await?;
    approvals::announce(state, &outcome).await?;
    match outcome {
        Outcome::Done(_) => Ok(CreditOutcome::Credited(
            SupplierReturn::find_or_404(&state.db, ret.id).await?,
        )),
        Outcome::Pending(_) => {
            let request = ApprovalRequest::pending_about(&state.db, &ret.subject())
                .await?
                .pop()
                .ok_or(Error::NotFound)?;
            Ok(CreditOutcome::Waiting(request))
        }
    }
}

/// The return a debit note event is about, read in `tx`.
async fn return_of(
    tx: &mut Transaction,
    subject: Option<&wl_kernel::Subject>,
) -> Result<Option<SupplierReturn>> {
    match subject {
        Some(subject) if subject.subject_type == SUPPLIER_RETURN_SUBJECT_TYPE => {
            SupplierReturn::find(&mut *tx, subject.subject_id).await
        }
        _ => Ok(None),
    }
}

/// Takes `debited` (purchase order line, quantity) off the credit lines in
/// turn: `add` is true to cover, false to uncover. Returns the new covered
/// quantity of each line that changed.
fn spread(
    lines: &[SupplierReturnLine],
    debited: &[(Option<i64>, i64)],
    add: bool,
) -> Vec<(i64, i64)> {
    let mut pool: Vec<(Option<i64>, i64)> = debited.to_vec();
    let mut changed = Vec::new();
    let order: Vec<&SupplierReturnLine> = if add {
        lines.iter().collect()
    } else {
        lines.iter().rev().collect()
    };
    for line in order
        .into_iter()
        .filter(|line| line.wanted_outcome == WantedOutcome::Credit)
    {
        let room = if add {
            line.quantity_milli - line.covered_milli
        } else {
            line.covered_milli
        };
        let mut left = room.max(0);
        let mut moved = 0;
        for entry in pool
            .iter_mut()
            .filter(|(po_line, _)| *po_line == line.po_line_id)
        {
            let take = entry.1.min(left);
            entry.1 -= take;
            left -= take;
            moved += take;
        }
        if moved > 0 {
            let covered = if add {
                line.covered_milli + moved
            } else {
                line.covered_milli - moved
            };
            changed.push((line.id, covered));
        }
    }
    changed
}

/// What the lines of a debit note debit, as (order line, quantity).
fn debited(lines: &[wl_kernel::invoicing::DebitedLine]) -> Vec<(Option<i64>, i64)> {
    lines
        .iter()
        .map(|line| (line.po_line, line.quantity_milli))
        .collect()
}

/// A supplier debit note was issued: its lines cover the credit lines of the
/// dispatched return it came from; when every line is covered the return is
/// `credited`.
///
/// # Errors
///
/// A database error, or a refusal of the transition.
pub(crate) async fn on_debit_note_issued(
    state: &AppState,
    event: &SupplierDebitNoteIssued,
) -> Result {
    let mut tx = state.db.begin().await?;
    let Some(ret) = return_of(&mut tx, event.return_to_supplier.as_ref()).await? else {
        return Ok(());
    };
    if ret.status != ReturnStatus::Dispatched {
        return Ok(());
    }
    let lines = supplier_return::lines_in(&mut tx, ret.id).await?;
    let now = renox::db::now();
    for (line_id, covered) in spread(&lines, &debited(&event.lines), true) {
        SupplierReturnLine::query()
            .where_(SupplierReturnLine::ID.eq(line_id))
            .update(
                &mut tx,
                &[("covered_milli", &covered), ("updated_at", &now)],
            )
            .await?;
    }
    let outcome = credit_if_covered(&mut tx, &ret, "debit_note", Some(event.debit_note.0)).await?;
    tx.commit().await?;
    if let Some(outcome) = outcome {
        outcome.emit(state).await?;
    }
    Ok(())
}

/// A supplier debit note was cancelled: a `dispatched` return loses what the
/// note covered and stays `dispatched`; procurement gets a task (the finance
/// side of a `credited` return is handled on the bill).
///
/// # Errors
///
/// A database error.
pub(crate) async fn on_debit_note_cancelled(
    state: &AppState,
    event: &SupplierDebitNoteCancelled,
) -> Result {
    let mut tx = state.db.begin().await?;
    let Some(ret) = return_of(&mut tx, event.return_to_supplier.as_ref()).await? else {
        return Ok(());
    };
    if ret.status == ReturnStatus::Dispatched {
        let lines = supplier_return::lines_in(&mut tx, ret.id).await?;
        let now = renox::db::now();
        for (line_id, covered) in spread(&lines, &debited(&event.lines), false) {
            SupplierReturnLine::query()
                .where_(SupplierReturnLine::ID.eq(line_id))
                .update(
                    &mut tx,
                    &[("covered_milli", &covered), ("updated_at", &now)],
                )
                .await?;
        }
    }
    tx.commit().await?;
    if !matches!(
        ret.status,
        ReturnStatus::Dispatched | ReturnStatus::Credited
    ) {
        return Ok(());
    }
    let number = ret.number.clone().unwrap_or_default();
    let title = said(
        "wl_inventory.return.debit_note_cancelled_title",
        &[("number", &number)],
    );
    let description = said(
        "wl_inventory.return.debit_note_cancelled",
        &[("number", &number), ("reason", &event.reason)],
    );
    let task = RoleTask::new(
        ret.subject(),
        PROCUREMENT_ROLE,
        &title,
        &format!("inventory.debit_note_cancelled:{}", event.debit_note.0),
    )
    .description(&description);
    role_tasks::open(state, task).await?;
    Ok(())
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
