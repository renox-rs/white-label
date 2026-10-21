//! Deciding the lines of a stock hold, closing it and cancelling it
//! (wl-inventory §4 *Stock hold*, issue #231); the rows and the machines are
//! in [`crate::stock_hold`] and [`crate::hold_line`].
//!
//! - [`decide`]: a holder of `lots.release` in the location's scope decides
//!   one line (`release`, `discard` or `return_to_supplier`). A decision that
//!   needs a second person (`STK.ACL.05` for a listed reason, `STK.ACL.02`
//!   above the value) becomes an approval request: the line stays
//!   `undecided` and nothing posts until the last yes. A rejected request
//!   leaves the line undecided.
//! - [`settle`]: after each decision (a listener on the line's transitions,
//!   so also after an approval) the lots of the hold are released or
//!   discarded when nothing else holds them, and the hold `close`s when its
//!   last line is decided. It is idempotent: the hourly job of `STK.RUL.07`
//!   runs it again for a hold whose decisions were all made.
//! - [`cancel`]: a hold placed in error is cancelled before any line is
//!   decided; each lot goes back to the status it had (`unhold`) unless
//!   another open hold has it.
//!
//! Decisions: a lot is released when every line of it in the hold is decided
//! `release` or `discard`, something of it is left and no other open hold on
//! whole lots has it; it is discarded when nothing of it is left; a lot with
//! a line returned to the supplier stays `on_hold` until the return is
//! dispatched (the dispatch rejects it). The wastage of a discarded line is
//! posted to the recall as cost object when the hold is a recall's, else to
//! the hold, after the commit, under one key per line.

use std::collections::BTreeSet;

use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, TransitionError, Transitioned};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::inventory::{
    GOODS_RECEIPT_SUBJECT_TYPE, HeldOutcome, ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE,
    LOT_SUBJECT_TYPE, LotDiscarded, LotReleased, STOCK_HOLD_LINE_SUBJECT_TYPE, StockHoldClosed,
};
use wl_kernel::invoicing::{PostCost, post_cost};

use crate::balance;
use crate::config_of;
use crate::goods_receipt::GoodsReceipt;
use crate::hold_line::{
    DISCARD, DISCARD_APPROVED, DecideInput, RELEASE, RELEASE_APPROVED, RETURN_TO_SUPPLIER,
};
use crate::item::{Item, value_of};
use crate::lot::{self, Lot, LotStatus};
use crate::movement::{MovementType, StockMovement};
use crate::return_credit::{CANCEL as RETURN_CANCEL, CANCEL_WITH_RECEIPT};
use crate::stock_hold::{
    CANCEL, CLOSE, HoldLineStatus, HoldScope, HoldStatus, SettleInput, StockHold, StockHoldLine,
    lines_of,
};
use crate::supplier_return::{
    self, ReturnOrigin, ReturnStatus, SupplierReturn, SupplierReturnLine,
};

/// What a holder of `lots.release` decides for a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The quantity is available again.
    Release,
    /// The quantity is wasted.
    Discard,
    /// The quantity goes to quarantine and onto a draft return.
    ReturnToSupplier,
}

impl Decision {
    /// The line's transition that makes it.
    pub fn transition(self) -> &'static str {
        match self {
            Decision::Release => RELEASE,
            Decision::Discard => DISCARD,
            Decision::ReturnToSupplier => RETURN_TO_SUPPLIER,
        }
    }
}

/// What [`decide`] did.
#[derive(Debug, Clone)]
pub enum DecideOutcome {
    /// The line is decided and its effect posted.
    Decided(StockHoldLine),
    /// A second person must approve first; the line stays `undecided`.
    Waiting(ApprovalRequest),
}

/// The supplier of `lot`: the supplier of the goods receipt it came from;
/// `None` for a lot with another source (a produced lot).
async fn supplier_of(db: &Db, lot: &Lot) -> Result<Option<i64>> {
    match lot.source() {
        Some(source) if source.subject_type == GOODS_RECEIPT_SUBJECT_TYPE => {
            Ok(GoodsReceipt::find(db, source.subject_id)
                .await?
                .map(|receipt| receipt.supplier_party_id))
        }
        _ => Ok(None),
    }
}

/// `user` decides line `id`. The decision is a user's run of the line's
/// transition, which the kernel turns into an approval request when a
/// policy asks for one.
///
/// # Errors
///
/// `Forbidden` without `lots.release` in the location's scope; `Guard`
/// (422) when the hold is not open, the lot is expired (`release`) or has no
/// supplier (`return_to_supplier`), or the stock is no longer at the
/// location; `NotFrom` for a line already decided.
pub async fn decide(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    decision: Decision,
) -> std::result::Result<DecideOutcome, TransitionError> {
    let line = StockHoldLine::find_or_404(&state.db, id).await?;
    let hold = StockHold::find_or_404(&state.db, line.stock_hold_id).await?;
    let item = Item::find_or_404(&state.db, line.item_id).await?;
    let lot = match line.lot_id {
        Some(lot_id) => Lot::find(&state.db, lot_id).await?,
        None => None,
    };
    let supplier = match &lot {
        Some(lot) => supplier_of(&state.db, lot).await?,
        None => None,
    };
    let run = DecideInput {
        amount: value_of(line.quantity_milli, item.average_cost),
        second_person: i64::from(config_of(state).release_needs_second_person(&hold.reason)),
        by: Some(user.id),
        hold_open: hold.status == HoldStatus::Open,
        lot_expired: lot
            .as_ref()
            .is_some_and(|lot| lot.status == LotStatus::Expired),
        supplier,
        at: None,
    };
    let outcome = line
        .transition(decision.transition())
        .by(user)
        .input(&run)
        .run(state)
        .await?;
    approvals::announce(state, &outcome).await?;
    match outcome {
        Outcome::Done(_) => Ok(DecideOutcome::Decided(
            StockHoldLine::find_or_404(&state.db, id).await?,
        )),
        Outcome::Pending(_) => {
            let request = ApprovalRequest::pending_about(&state.db, &line.subject())
                .await?
                .pop()
                .ok_or(Error::NotFound)?;
            Ok(DecideOutcome::Waiting(request))
        }
    }
}

/// Whether another open hold on whole lots, besides `except`, still has an
/// undecided line of `lot_id`.
async fn held_elsewhere(tx: &mut renox::db::Transaction, lot_id: i64, except: i64) -> Result<bool> {
    let rows = StockHoldLine::query()
        .where_(StockHoldLine::LOT_ID.eq(Some(lot_id)))
        .where_(StockHoldLine::STATUS.eq(HoldLineStatus::Undecided))
        .get(&mut *tx)
        .await?;
    for row in rows {
        if row.stock_hold_id == except {
            continue;
        }
        let hold = StockHold::find_or_404(&mut *tx, row.stock_hold_id).await?;
        if hold.status == HoldStatus::Open && hold.scope_kind == HoldScope::Lots {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `user` cancels hold `id` placed in error, with `reason`.
///
/// # Errors
///
/// `Forbidden` without `lots.release` in the hold's scope; `ReasonRequired`
/// without a reason; `Guard` (422) for a recall hold or when a line is
/// decided already; `NotFrom` unless the hold is `open`.
pub async fn cancel(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> std::result::Result<StockHold, TransitionError> {
    let hold = StockHold::find_or_404(&state.db, id).await?;
    let lines = lines_of(&state.db, hold.id).await?;
    let decided = lines
        .iter()
        .filter(|line| line.status != HoldLineStatus::Undecided)
        .count() as i64;
    let run = SettleInput {
        undecided: lines.len() as i64 - decided,
        decided,
    };
    let mut tx = state.db.begin().await?;
    let hold_outcome = hold
        .transition(CANCEL)
        .by(user)
        .reason(reason.trim().to_owned())
        .input(&run)
        .run_in(&mut tx)
        .await?;
    let lots: BTreeSet<i64> = lines.iter().filter_map(|line| line.lot_id).collect();
    let mut lot_outcomes = Vec::new();
    for lot_id in lots {
        let lot = Lot::find_or_404(&mut tx, lot_id).await?;
        if lot.status != LotStatus::OnHold || held_elsewhere(&mut tx, lot_id, hold.id).await? {
            continue;
        }
        let name = if lot.status_before_hold.as_deref() == Some("quarantine") {
            lot::UNHOLD_TO_QUARANTINE
        } else {
            lot::UNHOLD
        };
        lot_outcomes.push(lot.transition(name).by_system().run_in(&mut tx).await?);
    }
    tx.commit().await?;
    hold_outcome.emit(state).await?;
    for outcome in lot_outcomes {
        outcome.emit(state).await?;
    }
    Ok(StockHold::find_or_404(&state.db, id).await?)
}

/// After a decision: releases or discards the lots of hold `hold_id` that
/// nothing else holds, and closes the hold when its last line is decided.
/// Does nothing for a hold that is not `open`.
///
/// # Errors
///
/// A database error.
pub async fn settle(state: &AppState, hold_id: i64) -> Result {
    let hold = StockHold::find_or_404(&state.db, hold_id).await?;
    if hold.status != HoldStatus::Open {
        return Ok(());
    }
    let mut tx = state.db.begin().await?;
    let lines = lines_of(&mut tx, hold.id).await?;
    let lots: BTreeSet<i64> = lines.iter().filter_map(|line| line.lot_id).collect();
    let mut lot_outcomes = Vec::new();
    for lot_id in lots {
        let lot = Lot::find_or_404(&mut tx, lot_id).await?;
        if lot.status != LotStatus::OnHold {
            continue;
        }
        let open = lines.iter().any(|line| {
            line.lot_id == Some(lot_id)
                && matches!(
                    line.status,
                    HoldLineStatus::Undecided | HoldLineStatus::ReturnedToSupplier
                )
        });
        if open || held_elsewhere(&mut tx, lot_id, hold.id).await? {
            continue;
        }
        let left = balance::lot_left(&mut tx, lot_id).await?;
        let result = if left <= 0 {
            lot.transition(lot::DISCARD)
                .by_system()
                .reason(hold.reason.clone())
                .run_in(&mut tx)
                .await
        } else {
            lot.transition(lot::RELEASE)
                .by_system()
                .run_in(&mut tx)
                .await
        };
        lot_outcomes.push(result.map_err(TransitionError::into_error)?);
    }
    let undecided = lines
        .iter()
        .filter(|line| line.status == HoldLineStatus::Undecided)
        .count() as i64;
    let closed = if undecided == 0 {
        let run = SettleInput {
            undecided,
            decided: lines.len() as i64,
        };
        Some(
            hold.transition(CLOSE)
                .by_system()
                .input(&run)
                .run_in(&mut tx)
                .await
                .map_err(TransitionError::into_error)?,
        )
    } else {
        None
    };
    tx.commit().await?;
    for outcome in lot_outcomes {
        outcome.emit(state).await?;
    }
    if let Some(outcome) = closed {
        outcome.emit(state).await?;
    }
    Ok(())
}

/// Posts the wastage of discarded line `line` to its cost object: the
/// recall for a recall hold, else the hold. A repeat posts nothing twice.
async fn post_wastage(state: &AppState, line: &StockHoldLine) -> Result {
    let value: i64 = StockMovement::query()
        .where_(StockMovement::SOURCE_TYPE.eq(STOCK_HOLD_LINE_SUBJECT_TYPE))
        .where_(StockMovement::SOURCE_ID.eq(line.id))
        .where_(StockMovement::MOVEMENT_TYPE.eq(MovementType::Wastage))
        .get(&state.db)
        .await?
        .iter()
        .map(|moved| moved.value)
        .sum();
    if value <= 0 {
        return Ok(());
    }
    let hold = StockHold::find_or_404(&state.db, line.stock_hold_id).await?;
    let object = if hold.reason == "recall" {
        hold.reference().unwrap_or_else(|| hold.subject())
    } else {
        hold.subject()
    };
    let request = PostCost::new(
        line.subject(),
        format!("inventory.hold_line_wastage:{}", line.id),
        object,
        value,
    )
    .cost_code("wastage");
    post_cost(state, request).await
}

/// The event of the closed hold `hold` with its `lines`.
pub fn closed_event(hold: &StockHold, lines: &[StockHoldLine]) -> StockHoldClosed {
    StockHoldClosed {
        hold: hold.subject(),
        number: hold.number.clone().unwrap_or_default(),
        reason: hold.reason.clone(),
        reference: hold.reference(),
        outcomes: lines
            .iter()
            .map(|line| HeldOutcome {
                item: Subject::new(ITEM_SUBJECT_TYPE, line.item_id),
                location: Subject::new(LOCATION_SUBJECT_TYPE, line.location_id),
                lot: line.lot_id.map(|id| Subject::new(LOT_SUBJECT_TYPE, id)),
                quantity_milli: line.quantity_milli,
                outcome: line.outcome.clone().unwrap_or_default(),
            })
            .collect(),
    }
}

/// The quantity of lot `lot_id` that went to wastage, in thousandths: what
/// the lines of its hold discarded one by one and what the lot's own
/// `discard` wrote off after them. Decision: `LotDiscarded` reports the whole
/// quantity the lot cost the business, not only the last step. A discard
/// through the lines leaves nothing for the lot's own step, and the event
/// still names what was discarded.
async fn discarded_quantity(db: &Db, lot_id: i64) -> Result<i64> {
    Ok(StockMovement::query()
        .where_(StockMovement::LOT_ID.eq(Some(lot_id)))
        .where_(StockMovement::MOVEMENT_TYPE.eq(MovementType::Wastage))
        .get(db)
        .await?
        .iter()
        .map(|moved| moved.quantity_milli)
        .sum())
}

/// A cancelled Return to supplier gives back the lots it held aside.
///
/// `return_to_supplier` closes the hold but leaves the lot `on_hold` until
/// the return is dispatched. When the draft is cancelled instead, the goods
/// stay in the quarantine location with a lot nobody holds any more.
/// Decision: each such lot goes back to `quarantine` (`unhold_to_quarantine`),
/// not straight to `released`: the goods were held for a quality reason, were
/// moved to quarantine, and nobody has decided to release them; the
/// storekeeper inspects and releases them. A lot another open hold has
/// undecided lines of, or that another draft return still lists, stays held.
async fn restore_after_cancelled_return(state: &AppState, return_id: i64) -> Result {
    let mut tx = state.db.begin().await?;
    let lines = supplier_return::lines_in(&mut tx, return_id).await?;
    let lots: BTreeSet<i64> = lines
        .iter()
        .filter(|line| line.origin == ReturnOrigin::FromStock)
        .filter_map(|line| line.lot_id)
        .collect();
    let mut outcomes = Vec::new();
    for lot_id in lots {
        let lot = Lot::find_or_404(&mut tx, lot_id).await?;
        if lot.status != LotStatus::OnHold || held_elsewhere(&mut tx, lot_id, 0).await? {
            continue;
        }
        let mut listed_elsewhere = false;
        for other in SupplierReturnLine::query()
            .where_(SupplierReturnLine::LOT_ID.eq(Some(lot_id)))
            .where_(SupplierReturnLine::SUPPLIER_RETURN_ID.ne(return_id))
            .get(&mut tx)
            .await?
        {
            let ret = SupplierReturn::find_or_404(&mut tx, other.supplier_return_id).await?;
            listed_elsewhere |= ret.status == ReturnStatus::Draft;
        }
        if listed_elsewhere {
            continue;
        }
        outcomes.push(
            lot.transition(lot::UNHOLD_TO_QUARANTINE)
                .by_system()
                .run_in(&mut tx)
                .await
                .map_err(TransitionError::into_error)?,
        );
    }
    tx.commit().await?;
    for outcome in outcomes {
        outcome.emit(state).await?;
    }
    Ok(())
}

/// Registers the listeners of the decisions: a decided line posts its
/// wastage and settles its hold; a closed hold, a released lot and a
/// discarded lot are told to the other modules.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<StockHoldLine>, state: AppState| async move {
            let name = event.transition.as_str();
            if ![
                RELEASE,
                RELEASE_APPROVED,
                DISCARD,
                DISCARD_APPROVED,
                RETURN_TO_SUPPLIER,
            ]
            .contains(&name)
            {
                return Ok(());
            }
            let line = StockHoldLine::find_or_404(&state.db, event.key).await?;
            if line.status == HoldLineStatus::Undecided {
                return Ok(());
            }
            if line.status == HoldLineStatus::Discarded {
                post_wastage(&state, &line).await?;
            }
            settle(&state, line.stock_hold_id).await
        },
    );
    app.listen(
        |event: Transitioned<StockHold>, state: AppState| async move {
            if event.transition != CLOSE {
                return Ok(());
            }
            let hold = StockHold::find_or_404(&state.db, event.key).await?;
            let lines = lines_of(&state.db, hold.id).await?;
            state.emit(closed_event(&hold, &lines)).await
        },
    );
    app.listen(
        |event: Transitioned<SupplierReturn>, state: AppState| async move {
            if event.transition != RETURN_CANCEL && event.transition != CANCEL_WITH_RECEIPT {
                return Ok(());
            }
            restore_after_cancelled_return(&state, event.key).await
        },
    );
    app.listen(|event: Transitioned<Lot>, state: AppState| async move {
        let lot = Lot::find_or_404(&state.db, event.key).await?;
        let subject = Subject::new(LOT_SUBJECT_TYPE, lot.id);
        let item = Subject::new(ITEM_SUBJECT_TYPE, lot.item_id);
        if event.transition == lot::RELEASE {
            state.emit(LotReleased { lot: subject, item }).await
        } else if event.transition == lot::DISCARD {
            let quantity_milli = discarded_quantity(&state.db, lot.id).await?;
            state
                .emit(LotDiscarded {
                    lot: subject,
                    item,
                    reason: event.reason.clone().unwrap_or_default(),
                    quantity_milli,
                })
                .await
        } else {
            Ok(())
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_decision_is_a_transition_of_the_line() {
        assert_eq!(Decision::Release.transition(), "release");
        assert_eq!(Decision::Discard.transition(), "discard");
        assert_eq!(
            Decision::ReturnToSupplier.transition(),
            "return_to_supplier"
        );
    }

    #[test]
    fn the_closed_event_names_the_outcome_of_each_line() {
        let hold = StockHold {
            id: 3,
            number: Some("HLD-2026-0001".into()),
            reason: "quality".into(),
            reference_type: Some("ticketing.ticket".into()),
            reference_id: Some(9),
            ..StockHold::default()
        };
        let line = StockHoldLine {
            item_id: 4,
            location_id: 2,
            lot_id: Some(7),
            quantity_milli: 5_000,
            outcome: Some("discard".into()),
            ..StockHoldLine::default()
        };
        let event = closed_event(&hold, &[line]);
        assert_eq!(event.hold.to_string(), "inventory.stock_hold:3");
        assert_eq!(event.reference.unwrap().to_string(), "ticketing.ticket:9");
        assert_eq!(event.outcomes[0].outcome, "discard");
        assert_eq!(event.outcomes[0].quantity_milli, 5_000);
        assert_eq!(
            event.outcomes[0].lot.as_ref().unwrap().to_string(),
            "inventory.lot:7"
        );
    }
}
