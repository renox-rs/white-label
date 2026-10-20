//! Dispatching a Return to supplier (wl-inventory §4 *Return to supplier*
//! `dispatch`, `STK.ACL.06`, issue #222). The goods go back to the supplier
//! for a credit or a replacement; `dispatch` (a holder of
//! `supplier_returns.manage`) moves the return from `draft` to `dispatched`:
//!
//! - the guards: the return has a supplier and lines; a collection proof (the
//!   supplier driver's signature, a courier number, or the goods receipt);
//!   the lines taken from stock are available at the return's location (and
//!   lot); a `replace` line from stock names an order that is not `closed`
//!   (after closing, goods are credited only);
//! - above the value (default AED 5,000: quantity times unit cost per line,
//!   lines rejected at receipt at the order's price, excluding VAT) the
//!   user's `dispatch` becomes an approval request (`PLT.APR.01`) for a
//!   second holder of `supplier_returns.manage`, never the requester; the
//!   return stays `draft` until `dispatch_approved`, and a rejection leaves
//!   it a draft;
//! - the effect, in the transaction: a `return_out` movement per line taken
//!   from stock (none for goods rejected at receipt, they never entered
//!   stock), valued at the source receipt's cost when known, the difference
//!   to the item's average cost kept on the line as the price variance; the
//!   lot `reject`ed when it was on hold and nothing of it is left; a
//!   `replace` line from stock gives its quantity back to the order line
//!   (outstanding goes up, once per line), and lines rejected at receipt give
//!   nothing back, their quantity stayed outstanding at `post`; the return
//!   note; `SupplierReturnDispatched` after the commit ([`crate::events`]).
//!
//! The goods refused at the door never entered stock and already left: the
//! system's `dispatch_at_door` (run by a goods receipt's `post`, with the
//! receipt as the collection proof) has no approval and no movement.
//!
//! Decisions: the guards read what [`gather`] found in a transaction of its
//! own that is rolled back (or in the transaction of the receipt's `post`),
//! and the effect finds it again and refuses with a 422 if stock moved in
//! between. The price variance is kept on the line (and reaches the cost
//! poster with the cost objects); the supplier contact is told through the
//! event.

use std::collections::BTreeMap;

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, Outcome, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::documents::dubai_date;

use crate::item::{Item, quantity_text, value_of};
use crate::lot::{self, Lot, LotStatus};
use crate::movement::{self, MovementType, StockMovement};
use crate::permissions;
use crate::purchase_order::{PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus};
use crate::return_note::{self, Note, NoteLine};
use crate::supplier_return::{
    self, RejectedHandling, ReturnOrigin, ReturnStatus, SupplierReturn, SupplierReturnLine,
    WantedOutcome,
};
use crate::texts::said;

/// The user's `dispatch` transition's name.
pub const DISPATCH: &str = "dispatch";

/// The system's `dispatch_approved` transition's name: the kernel runs it
/// when the approval of a dispatch above the value is given.
pub const DISPATCH_APPROVED: &str = "dispatch_approved";

/// The system's `dispatch_at_door` transition's name: a goods receipt's
/// `post` dispatches the goods refused at the door.
pub const DISPATCH_AT_DOOR: &str = "dispatch_at_door";

/// The approval policy's action key of `dispatch` (`STK.ACL.06`).
pub const ACTION: &str = "inventory.supplier_return_dispatch";

/// How the goods were handed over to the supplier (`DOC.SIG.01`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionProof {
    /// The supplier driver signed for the goods; the reference of the
    /// signature.
    DriverSignature(String),
    /// A courier took them; the courier's number.
    Courier(String),
    /// The goods receipt they were refused on.
    GoodsReceipt(String),
}

impl CollectionProof {
    /// The proof as the return keeps it: `driver_signature:SIG-1`.
    pub fn to_text(&self) -> String {
        let (kind, reference) = match self {
            CollectionProof::DriverSignature(r) => ("driver_signature", r),
            CollectionProof::Courier(r) => ("courier", r),
            CollectionProof::GoodsReceipt(r) => ("goods_receipt", r),
        };
        format!("{kind}:{}", reference.trim())
    }

    /// The proof of `kind` (`driver_signature`, `courier`, `goods_receipt`)
    /// with `reference`; `None` for another kind.
    pub fn parse(kind: &str, reference: &str) -> Option<Self> {
        let reference = reference.trim().to_owned();
        match kind.trim() {
            "driver_signature" => Some(CollectionProof::DriverSignature(reference)),
            "courier" => Some(CollectionProof::Courier(reference)),
            "goods_receipt" => Some(CollectionProof::GoodsReceipt(reference)),
            _ => None,
        }
    }

    /// Whether the proof says which signature, courier or receipt.
    pub fn is_given(&self) -> bool {
        match self {
            CollectionProof::DriverSignature(r)
            | CollectionProof::Courier(r)
            | CollectionProof::GoodsReceipt(r) => !r.trim().is_empty(),
        }
    }
}

/// What a `dispatch` run carries to its guards and effect, gathered by
/// [`gather`]. Each `*_problem` is the text of the first thing wrong of that
/// kind, `None` when nothing is.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DispatchInput {
    /// The value of the return in fils: what the approval policy's condition
    /// reads (`STK.ACL.06`).
    pub amount: i64,
    /// The user who dispatches it (who asked, when it waited for approval).
    pub by: Option<i64>,
    /// The collection proof, as [`CollectionProof::to_text`].
    pub proof: Option<String>,
    /// The return has a supplier and lines.
    pub lines_problem: Option<String>,
    /// There is no collection proof.
    pub proof_problem: Option<String>,
    /// Part of the stock lines is not at the location.
    pub stock_problem: Option<String>,
    /// A replacement is wanted on an order that is closed.
    pub order_problem: Option<String>,
}

impl DispatchInput {
    /// The first problem, in the order the guards run.
    pub fn first_problem(&self) -> Option<&str> {
        [
            &self.lines_problem,
            &self.proof_problem,
            &self.stock_problem,
            &self.order_problem,
        ]
        .into_iter()
        .find_map(|problem| problem.as_deref())
    }
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The value of a return in fils: quantity times unit cost per line,
/// excluding VAT.
pub fn value_of_lines(lines: &[SupplierReturnLine]) -> i64 {
    lines
        .iter()
        .map(|line| value_of(line.quantity_milli, line.unit_cost))
        .sum()
}

/// What stands in the way of dispatching `ret`, and its value, read in `tx`.
/// `proof` is what the user hands over; the return's own proof (the goods
/// receipt of a door-refused return) is used when there is none.
///
/// # Errors
///
/// A database error.
pub async fn gather(
    tx: &mut Transaction,
    ret: &SupplierReturn,
    proof: Option<&CollectionProof>,
    by: Option<i64>,
) -> Result<DispatchInput> {
    let lines = supplier_return::lines_in(tx, ret.id).await?;
    let proof_text = match proof {
        Some(proof) if proof.is_given() => Some(proof.to_text()),
        Some(_) => None,
        None => ret
            .collection_proof
            .clone()
            .filter(|text| !text.trim().is_empty()),
    };
    let mut input = DispatchInput {
        amount: value_of_lines(&lines),
        by,
        proof: proof_text.clone(),
        ..DispatchInput::default()
    };
    if lines.is_empty() || ret.supplier_party_id == 0 {
        input.lines_problem = Some(said("wl_inventory.return.no_lines", &[]));
    }
    if proof_text.is_none() {
        input.proof_problem = Some(said("wl_inventory.return.proof_needed", &[]));
    }
    input.stock_problem = stock_problem(tx, ret, &lines).await?;
    input.order_problem = order_problem(tx, &lines).await?;
    Ok(input)
}

/// The first stock line that is not (all of it) at the return's location.
async fn stock_problem(
    tx: &mut Transaction,
    ret: &SupplierReturn,
    lines: &[SupplierReturnLine],
) -> Result<Option<String>> {
    let mut claimed: BTreeMap<(i64, Option<i64>), i64> = BTreeMap::new();
    for line in lines.iter().filter(|line| line.origin == ReturnOrigin::FromStock) {
        let position = line.position.to_string();
        let Some(location_id) = ret.location_id else {
            return Ok(Some(said(
                "wl_inventory.return.no_location",
                &[("position", &position)],
            )));
        };
        let item = Item::find_or_404(&mut *tx, line.item_id).await?;
        let needed = claimed.entry((line.item_id, line.lot_id)).or_insert(0);
        *needed += line.quantity_milli;
        let needed = *needed;
        let available = match line.lot_id {
            Some(lot_id) => movement::on_hand_lot_in(tx, line.item_id, lot_id, location_id).await?,
            None => movement::on_hand_in(tx, line.item_id, location_id).await?,
        };
        if available < needed {
            return Ok(Some(said(
                "wl_inventory.return.not_in_stock",
                &[
                    ("position", &position),
                    ("item", &item.code),
                    ("available", &quantity_text(available.max(0))),
                    ("quantity", &quantity_text(needed)),
                ],
            )));
        }
    }
    Ok(None)
}

/// The first replacement from stock on an order that is `closed`.
async fn order_problem(
    tx: &mut Transaction,
    lines: &[SupplierReturnLine],
) -> Result<Option<String>> {
    for line in lines.iter().filter(|line| {
        line.origin == ReturnOrigin::FromStock && line.wanted_outcome == WantedOutcome::Replace
    }) {
        let Some(po_line_id) = line.po_line_id else {
            continue;
        };
        let Some(po_line) = PurchaseOrderLine::find(&mut *tx, po_line_id).await? else {
            continue;
        };
        let order = PurchaseOrder::find(&mut *tx, po_line.purchase_order_id).await?;
        if order.is_some_and(|order| order.status == PurchaseOrderStatus::Closed) {
            return Ok(Some(said(
                "wl_inventory.return.order_closed",
                &[("position", &line.position.to_string())],
            )));
        }
    }
    Ok(None)
}

fn input(ctx: &TransitionCtx) -> Option<DispatchInput> {
    ctx.input_as::<DispatchInput>().ok()
}

/// A guard that refuses with the problem `pick` finds in the run's input.
fn with_problem(
    transition: Transition<SupplierReturn>,
    name: &'static str,
    pick: fn(&DispatchInput) -> Option<String>,
) -> Transition<SupplierReturn> {
    transition.guard(name, move |_ret: &SupplierReturn, ctx| {
        let problem = input(ctx).map_or_else(
            || Some(said("wl_inventory.return.not_draft", &[])),
            |input| pick(&input),
        );
        Box::pin(async move { problem.map_or(Ok(()), Err) })
    })
}

/// The guards and the effect the three dispatch transitions share.
fn rules(transition: Transition<SupplierReturn>) -> Transition<SupplierReturn> {
    let transition = transition
        .from([ReturnStatus::Draft])
        .to(ReturnStatus::Dispatched);
    let transition = with_problem(transition, "has_lines", |i| i.lines_problem.clone());
    let transition = with_problem(transition, "collection_proof", |i| i.proof_problem.clone());
    let transition = with_problem(transition, "stock_available", |i| i.stock_problem.clone());
    let transition = with_problem(transition, "order_open_for_replacement", |i| {
        i.order_problem.clone()
    });
    transition.effect(|ret: &SupplierReturn, ctx, tx| {
        let ret = ret.clone();
        let input = input(ctx).unwrap_or_default();
        Box::pin(async move { dispatch_in(tx, ret, &input).await })
    })
}

/// `dispatch`, the system's `dispatch_approved` and `dispatch_at_door`.
pub(crate) fn transitions() -> Vec<Transition<SupplierReturn>> {
    vec![
        rules(
            Transition::new(DISPATCH)
                .label("Dispatch")
                .tone("primary")
                .permission(permissions::SUPPLIER_RETURNS_MANAGE),
        ),
        rules(
            Transition::new(DISPATCH_APPROVED)
                .label("Dispatch (approved)")
                .tone("primary")
                .system(),
        ),
        // The goods never entered stock and already left: no approval (the
        // action is not under a policy), no movement (the lines are all
        // rejected at receipt).
        rules(
            Transition::new(DISPATCH_AT_DOOR)
                .label("Dispatch at the door")
                .system(),
        )
        .guard("refused_at_the_door", |ret: &SupplierReturn, _ctx| {
            let door = ret.handling == RejectedHandling::RefusedAtDoor;
            Box::pin(async move {
                if door {
                    Ok(())
                } else {
                    Err(said("wl_inventory.return.not_at_door", &[]))
                }
            })
        }),
    ]
}

/// The machine of the return: the states and the dispatch transitions.
pub(crate) fn add_to(machine: Machine<SupplierReturn>) -> Machine<SupplierReturn> {
    transitions().into_iter().fold(machine, Machine::transition)
}

/// The effect of the three dispatch transitions, in the run's transaction.
async fn dispatch_in(tx: &mut Transaction, ret: SupplierReturn, run: &DispatchInput) -> Result {
    // Stock may have moved since the guards looked.
    let found = gather(tx, &ret, None, run.by).await?;
    let stock_or_order = found.stock_problem.as_ref().or(found.order_problem.as_ref());
    if let Some(problem) = stock_or_order {
        return Err(refused(problem.clone()));
    }
    let now = renox::db::now();
    let source = ret.subject();
    let lines = supplier_return::lines_in(tx, ret.id).await?;
    let mut note_lines = Vec::new();
    for line in &lines {
        let item = Item::find_or_404(&mut *tx, line.item_id).await?;
        let lot = match line.lot_id {
            Some(id) => Lot::find(&mut *tx, id).await?,
            None => None,
        };
        note_lines.push(NoteLine {
            item: item.code.clone(),
            lot: lot.as_ref().map(|lot| lot.number.clone()),
            quantity_milli: line.quantity_milli,
            unit: item.base_unit.clone(),
            reason: line.reason.clone(),
            outcome: line.wanted_outcome.as_str().to_owned(),
        });
        if line.origin != ReturnOrigin::FromStock {
            continue;
        }
        let location_id = ret
            .location_id
            .ok_or_else(|| refused(said("wl_inventory.return.no_location", &[])))?;
        // Valued at the source receipt's cost; the difference to the average
        // cost is the price variance.
        let value = value_of(line.quantity_milli, line.unit_cost);
        let variance = value - value_of(line.quantity_milli, item.average_cost);
        let mut moved = StockMovement::leaving(
            MovementType::ReturnOut,
            item.id,
            line.lot_id,
            location_id,
            line.quantity_milli,
            line.unit_cost,
            value,
            &source,
            Some(line.reason.clone()),
            run.by,
        );
        moved.insert(&mut *tx).await?;
        SupplierReturnLine::query()
            .where_(SupplierReturnLine::ID.eq(line.id))
            .update(&mut *tx, &[("variance", &variance), ("updated_at", &now)])
            .await?;
        // A held lot with nothing left is rejected.
        if let Some(lot) = lot {
            if lot.status == LotStatus::OnHold && movement::lot_left_in(tx, lot.id).await? <= 0 {
                let _outcome = lot
                    .transition(lot::REJECT)
                    .by_system()
                    .run_in(&mut *tx)
                    .await
                    .map_err(TransitionError::into_error)?;
            }
        }
        // A replacement from stock is outstanding on the order again, once.
        if line.wanted_outcome == WantedOutcome::Replace && line.restored_milli == 0 {
            if let Some(po_line_id) = line.po_line_id {
                let po_line = PurchaseOrderLine::find_or_404(&mut *tx, po_line_id).await?;
                let restored = line.quantity_milli.min(po_line.accepted_qty_milli).max(0);
                let accepted = po_line.accepted_qty_milli - restored;
                PurchaseOrderLine::query()
                    .where_(PurchaseOrderLine::ID.eq(po_line.id))
                    .update(
                        &mut *tx,
                        &[("accepted_qty_milli", &accepted), ("updated_at", &now)],
                    )
                    .await?;
                SupplierReturnLine::query()
                    .where_(SupplierReturnLine::ID.eq(line.id))
                    .update(
                        &mut *tx,
                        &[("restored_milli", &restored), ("updated_at", &now)],
                    )
                    .await?;
            }
        }
    }
    let proof = run
        .proof
        .clone()
        .or_else(|| ret.collection_proof.clone())
        .unwrap_or_default();
    let note = return_note::render(&Note {
        number: ret.number.clone().unwrap_or_default(),
        supplier: format!("Supplier {}", ret.supplier_party_id),
        source: ret.source().to_string(),
        proof: proof.clone(),
        date: dubai_date(now).to_string(),
        lines: note_lines,
    });
    SupplierReturn::query()
        .where_(SupplierReturn::ID.eq(ret.id))
        .update(
            &mut *tx,
            &[
                ("collection_proof", &Some(proof)),
                ("dispatched_by", &run.by),
                ("dispatched_at", &Some(now)),
                ("return_note", &Some(note)),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// Dispatches the return `ret`, refused at the door, as the system, inside
/// the transaction of the receipt's `post`.
///
/// # Errors
///
/// A database error, or a refusal of the transition.
pub(crate) async fn dispatch_at_door(
    tx: &mut Transaction,
    ret: &SupplierReturn,
    by: Option<i64>,
) -> Result {
    let found = gather(tx, ret, None, by).await?;
    let _outcome = ret
        .transition(DISPATCH_AT_DOOR)
        .by_system()
        .input(&found)
        .run_in(&mut *tx)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// What [`dispatch`] did.
#[derive(Debug, Clone)]
pub enum DispatchOutcome {
    /// The return is `dispatched`: the stock, the order and the note are done.
    Dispatched(SupplierReturn),
    /// The return is worth more than the limit: it stays `draft` and waits for
    /// a second person's approval (`STK.ACL.06`).
    Waiting(ApprovalRequest),
}

/// `user` dispatches return `return_id` with `proof`. Refused with the first
/// problem before anything is asked or written; above the value it becomes an
/// approval request instead (see the module docs).
///
/// # Errors
///
/// `Forbidden` without `supplier_returns.manage`; `NotFrom` unless `draft`;
/// `Guard` (422) for a problem of the return.
pub async fn dispatch(
    state: &AppState,
    user: &AuthUser,
    return_id: i64,
    proof: &CollectionProof,
) -> std::result::Result<DispatchOutcome, TransitionError> {
    let ret = SupplierReturn::find_or_404(&state.db, return_id).await?;
    if ret.status != ReturnStatus::Draft {
        return Err(TransitionError::Guard(said("wl_inventory.return.not_draft", &[])));
    }
    // Reads in a transaction of its own, rolled back: nothing is held while
    // the transition opens its own.
    let mut tx = state.db.begin().await?;
    let found = gather(&mut tx, &ret, Some(proof), Some(user.id)).await?;
    tx.rollback().await?;
    if let Some(problem) = found.first_problem() {
        return Err(TransitionError::Guard(problem.to_owned()));
    }
    let outcome = ret.transition(DISPATCH).by(user).input(&found).run(state).await?;
    approvals::announce(state, &outcome).await?;
    match outcome {
        Outcome::Done(_) => {
            let done = SupplierReturn::find_or_404(&state.db, ret.id).await?;
            Ok(DispatchOutcome::Dispatched(done))
        }
        Outcome::Pending(_) => {
            let request = ApprovalRequest::pending_about(&state.db, &ret.subject())
                .await?
                .pop()
                .ok_or(Error::NotFound)?;
            Ok(DispatchOutcome::Waiting(request))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(position: i64, qty: i64, cost: i64) -> SupplierReturnLine {
        SupplierReturnLine {
            position,
            quantity_milli: qty,
            unit_cost: cost,
            ..SupplierReturnLine::default()
        }
    }

    #[test]
    fn a_proof_is_a_kind_and_a_reference() {
        let proof = CollectionProof::parse("driver_signature", " SIG-1 ").unwrap();
        assert_eq!(proof.to_text(), "driver_signature:SIG-1");
        assert!(proof.is_given());
        assert!(!CollectionProof::parse("courier", "  ").unwrap().is_given());
        assert!(CollectionProof::parse("pigeon", "1").is_none());
    }

    #[test]
    fn the_value_is_quantity_times_cost_per_line() {
        // 120 units at AED 50.00 is AED 6,000.00; 2.5 at AED 1.00 is 2.50.
        assert_eq!(value_of_lines(&[line(1, 120_000, 5_000)]), 600_000);
        assert_eq!(value_of_lines(&[line(1, 120_000, 5_000), line(2, 2_500, 100)]), 600_250);
        assert_eq!(value_of_lines(&[]), 0);
    }

    #[test]
    fn the_first_problem_follows_the_order_of_the_guards() {
        let mut input = DispatchInput {
            order_problem: Some("order".into()),
            ..DispatchInput::default()
        };
        assert_eq!(input.first_problem(), Some("order"));
        input.stock_problem = Some("stock".into());
        input.proof_problem = Some("proof".into());
        assert_eq!(input.first_problem(), Some("proof"));
        input.lines_problem = Some("lines".into());
        assert_eq!(input.first_problem(), Some("lines"));
        assert_eq!(DispatchInput::default().first_problem(), None);
    }

    #[test]
    fn dispatch_is_for_users_and_the_other_two_for_the_system() {
        let machine = SupplierReturn::machine();
        let user = machine.find(DISPATCH).expect("dispatch");
        assert!(!user.is_system());
        for name in [DISPATCH_APPROVED, DISPATCH_AT_DOOR] {
            let transition = machine.find(name).expect("transition");
            assert!(transition.is_system(), "{name}");
        }
        for name in [DISPATCH, DISPATCH_APPROVED, DISPATCH_AT_DOOR] {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), ["draft"], "{name}");
            assert_eq!(transition.to_state(), Some("dispatched"), "{name}");
        }
    }
}
