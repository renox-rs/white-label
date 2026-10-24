//! Dispatching a picked dispatch (wl-inventory §4 *Dispatch* row `dispatch`,
//! §5.4 step 3, issue #240). The goods leave: `dispatch` (`dispatch.manage`
//! in the location's scope) moves a `picked` dispatch to `dispatched` and
//! posts a `dispatch` movement per line at the item's average cost, writes the
//! delivery note and keeps the hand-over. `DispatchSent` follows the commit
//! ([`crate::dispatch_events`]).
//!
//! The guards, in this order (the first problem is the refusal, and the
//! dispatch stays `picked`, its stock reserved):
//!
//! 1. no picked lot on hold: a lot that is `on_hold` (or no longer
//!    `released`, or past its expiry date) refuses with "re-pick"
//!    ([`crate::dispatch_pick::repick`] picks the line again from another
//!    lot), and so does quantity held at the location;
//! 2. the hand-over is chosen: our own delivery (a delivery is asked of
//!    `wl-delivery` through the kernel `DeliveryRequester`), a courier with
//!    a tracking number, or collection by the customer with the collector's
//!    signature (`DOC.SIG.01`);
//! 3. for a dispatch of a sales order, the credit is checked again with the
//!    kernel `CreditChecker` (`credit_recheck`; `wl-sales` provides it, and
//!    without it the check passes). A failure puts the order on
//!    `credit_hold` (that is `wl-sales`' own effect, kept even though the
//!    dispatch is refused), and the dispatch stays `picked` and reserved:
//!    nothing here releases the reservation. Once the order is released the
//!    dispatch can be tried again and the check runs again.
//!
//! The order of the guards is the spec's: a dispatch that cannot leave anyway
//! (a held lot) puts no order on credit hold.
//!
//! Decisions: the credit check has an effect of its own in `wl-sales`, so
//! it runs in the service before the transition, outside its transaction, and
//! its verdict goes in the transition's input where the `credit_recheck`
//! guard reads it; an own delivery is asked for after the guards pass and
//! before the transition, and asked to cancel again if the transition then
//! fails.

use std::collections::BTreeMap;

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::delivery::{DeliveryItem, DeliveryRequest};
use wl_kernel::documents::dubai_date;
use wl_kernel::sales::{CreditVerdict, SALES_ORDER_SUBJECT_TYPE, SalesOrderId};

use crate::balance;
use crate::dispatch::{Dispatch, DispatchLine, DispatchStatus, Handover, lines_of, refused};
use crate::dispatch_pick::lot_problem;
use crate::item::{Item, quantity_text, value_of};
use crate::lot::Lot;
use crate::movement::{MovementType, StockMovement};
use crate::permissions;
use crate::return_note;
use crate::texts::said;

/// The `dispatch` transition's name.
pub const DISPATCH: &str = "dispatch";

/// How the goods are handed over, as the user enters it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoverChoice {
    /// `own_delivery`, `courier` or `collection`.
    pub kind: String,
    /// The courier's tracking number (`courier`).
    pub tracking_number: Option<String>,
    /// The collector's signature, a document reference (`collection`).
    pub signature: Option<String>,
}

impl HandoverChoice {
    /// Our own delivery run.
    pub fn own_delivery() -> Self {
        HandoverChoice {
            kind: Handover::OwnDelivery.as_str().to_owned(),
            tracking_number: None,
            signature: None,
        }
    }

    /// A courier with `tracking_number`.
    pub fn courier(tracking_number: &str) -> Self {
        HandoverChoice {
            kind: Handover::Courier.as_str().to_owned(),
            tracking_number: Some(tracking_number.to_owned()),
            signature: None,
        }
    }

    /// Collection by the customer, signed for with `signature`.
    pub fn collection(signature: &str) -> Self {
        HandoverChoice {
            kind: Handover::Collection.as_str().to_owned(),
            tracking_number: None,
            signature: Some(signature.to_owned()),
        }
    }
}

/// What a `dispatch` run carries to its guards and its effect.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SendInput {
    /// The user who dispatches it.
    pub by: Option<i64>,
    /// The hand-over chosen.
    pub handover: Option<HandoverChoice>,
    /// The delivery asked of `wl-delivery`, for an own delivery.
    pub delivery: Option<i64>,
    /// The dispatch has lines.
    pub lines_problem: Option<String>,
    /// A picked lot is on hold (or not released): re-pick.
    pub lot_problem: Option<String>,
    /// The stock is not there, or part of it is on hold at the location.
    pub stock_problem: Option<String>,
    /// The hand-over is not chosen or not complete.
    pub handover_problem: Option<String>,
    /// The credit re-check failed.
    pub credit_problem: Option<String>,
}

impl SendInput {
    /// The first problem, in the order the guards run: lot hold first, the
    /// credit last.
    pub fn first_problem(&self) -> Option<&str> {
        [
            &self.lines_problem,
            &self.lot_problem,
            &self.stock_problem,
            &self.handover_problem,
            &self.credit_problem,
        ]
        .into_iter()
        .find_map(|problem| problem.as_deref())
    }

    /// The problem that comes before the credit check: when there is one the
    /// dispatch cannot leave anyway and the credit is not checked.
    pub fn problem_before_credit(&self) -> bool {
        self.lines_problem.is_some()
            || self.lot_problem.is_some()
            || self.stock_problem.is_some()
            || self.handover_problem.is_some()
    }
}

/// What stands in the way of dispatching `dispatch` with `choice`, read in
/// `tx`; the credit is not looked at here.
///
/// # Errors
///
/// A database error.
pub async fn gather(
    tx: &mut Transaction,
    dispatch: &Dispatch,
    choice: Option<&HandoverChoice>,
    by: Option<i64>,
) -> Result<SendInput> {
    let lines = lines_of(&mut *tx, dispatch.id).await?;
    let mut found = SendInput {
        by,
        handover: choice.cloned(),
        ..SendInput::default()
    };
    if lines.is_empty() {
        found.lines_problem = Some(said("wl_inventory.dispatch.no_lines", &[]));
        return Ok(found);
    }
    let mut claimed: BTreeMap<(i64, Option<i64>), i64> = BTreeMap::new();
    for line in &lines {
        if let Some(lot_id) = line.lot_id
            && found.lot_problem.is_none()
        {
            let lot = Lot::find_or_404(&mut *tx, lot_id).await?;
            found.lot_problem = lot_problem(&lot, line.position)
                .map(|problem| format!("{problem} {}", said("wl_inventory.dispatch.repick", &[])));
        }
        let needed = claimed.entry((line.item_id, line.lot_id)).or_insert(0);
        *needed += line.quantity_milli;
        let needed = *needed;
        if found.stock_problem.is_some() {
            continue;
        }
        let item = Item::find_or_404(&mut *tx, line.item_id).await?;
        let at =
            balance::balance(&mut *tx, line.item_id, dispatch.location_id, line.lot_id).await?;
        let position = line.position.to_string();
        if at.on_hand_milli < needed {
            found.stock_problem = Some(said(
                "wl_inventory.dispatch.not_in_stock",
                &[
                    ("position", &position),
                    ("item", &item.code),
                    ("available", &quantity_text(at.on_hand_milli.max(0))),
                    ("quantity", &quantity_text(needed)),
                ],
            ));
        } else if at.on_hand_milli - at.reserved_milli - at.held_milli < 0 {
            // The dispatch's own reservation is part of `reserved`: what is
            // left over must not be negative once the hold is counted.
            found.stock_problem = Some(format!(
                "{} {}",
                said(
                    "wl_inventory.dispatch.held",
                    &[
                        ("position", &position),
                        ("item", &item.code),
                        ("held", &quantity_text(at.held_milli)),
                    ],
                ),
                said("wl_inventory.dispatch.repick", &[])
            ));
        }
    }
    found.handover_problem = handover_problem(dispatch, choice);
    Ok(found)
}

/// What is missing from the hand-over `choice`, `None` when it is complete.
fn handover_problem(dispatch: &Dispatch, choice: Option<&HandoverChoice>) -> Option<String> {
    let Some(choice) = choice else {
        return Some(said("wl_inventory.dispatch.handover_needed", &[]));
    };
    let given = |text: &Option<String>| text.as_deref().is_some_and(|t| !t.trim().is_empty());
    match Handover::parse(&choice.kind) {
        None => Some(said("wl_inventory.dispatch.handover_needed", &[])),
        Some(Handover::Courier) if !given(&choice.tracking_number) => {
            Some(said("wl_inventory.dispatch.tracking_needed", &[]))
        }
        Some(Handover::Collection) if !given(&choice.signature) => {
            Some(said("wl_inventory.dispatch.signature_needed", &[]))
        }
        Some(Handover::OwnDelivery) if dispatch.recipient_name.trim().is_empty() => {
            Some(said("wl_inventory.dispatch.recipient_needed", &[]))
        }
        Some(_) => None,
    }
}

fn input(ctx: &TransitionCtx) -> Option<SendInput> {
    ctx.input_as::<SendInput>().ok()
}

/// A guard that refuses with the problem `pick` finds in the run's input.
fn with_problem(
    transition: Transition<Dispatch>,
    name: &'static str,
    pick: fn(&SendInput) -> Option<String>,
) -> Transition<Dispatch> {
    transition.guard(name, move |_dispatch: &Dispatch, ctx| {
        let problem = input(ctx).map_or_else(
            || Some(said("wl_inventory.dispatch.not_picked", &[])),
            |input| pick(&input),
        );
        Box::pin(async move { problem.map_or(Ok(()), Err) })
    })
}

/// The effect of `dispatch`: the goods leave stock.
async fn send_in(tx: &mut Transaction, dispatch: Dispatch, run: &SendInput) -> Result {
    // Stock may have moved or been held since the guards looked.
    let found = gather(tx, &dispatch, run.handover.as_ref(), run.by).await?;
    if let Some(problem) = found
        .lot_problem
        .as_ref()
        .or(found.stock_problem.as_ref())
        .or(found.handover_problem.as_ref())
    {
        return Err(refused(problem.clone()));
    }
    let now = renox::db::now();
    let source = dispatch.subject();
    let lines = lines_of(&mut *tx, dispatch.id).await?;
    let mut note_lines = Vec::new();
    for line in &lines {
        let item = Item::find_or_404(&mut *tx, line.item_id).await?;
        let lot = match line.lot_id {
            Some(id) => Lot::find(&mut *tx, id).await?,
            None => None,
        };
        let value = value_of(line.quantity_milli, item.average_cost);
        let mut moved = StockMovement::leaving(
            MovementType::Dispatch,
            item.id,
            line.lot_id,
            dispatch.location_id,
            line.quantity_milli,
            item.average_cost,
            value,
            &source,
            None,
            run.by,
        );
        moved.moved_at = now;
        moved.insert(&mut *tx).await?;
        DispatchLine::query()
            .where_(DispatchLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[("unit_cost", &item.average_cost), ("updated_at", &now)],
            )
            .await?;
        let lot_text = lot
            .map(|lot| format!(" lot {}", lot.number))
            .unwrap_or_default();
        note_lines.push((
            11,
            format!(
                "{} x {} {}{}",
                line.position,
                quantity_text(line.quantity_milli),
                item.code,
                lot_text
            ),
        ));
    }
    let choice = run
        .handover
        .clone()
        .unwrap_or(HandoverChoice::own_delivery());
    let mut text = vec![
        (
            16,
            format!(
                "Delivery note {}",
                dispatch.number.clone().unwrap_or_default()
            ),
        ),
        (11, format!("Date: {}", dubai_date(now))),
        (11, format!("To: {}", dispatch.recipient_name)),
        (11, format!("Address: {}", dispatch.address)),
        (11, format!("For: {}", dispatch.source())),
        (11, format!("Hand-over: {}", choice.kind.replace('_', " "))),
        (11, String::new()),
        (12, "Goods".to_owned()),
    ];
    text.extend(note_lines);
    let note = return_note::document(&text);
    Dispatch::query()
        .where_(Dispatch::ID.eq(dispatch.id))
        .update(
            &mut *tx,
            &[
                ("handover", &Some(choice.kind.clone())),
                ("tracking_number", &choice.tracking_number),
                ("collector_signature", &choice.signature),
                ("delivery_id", &run.delivery),
                ("delivery_note", &Some(note)),
                ("dispatched_by", &run.by),
                ("dispatched_at", &Some(now)),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// `dispatch`.
pub(crate) fn add_to(machine: Machine<Dispatch>) -> Machine<Dispatch> {
    use DispatchStatus::{Dispatched, Picked};
    let send = Transition::new(DISPATCH)
        .label("Dispatch")
        .tone("primary")
        .permission_in(permissions::DISPATCH_MANAGE, |dispatch: &Dispatch| {
            dispatch.scope()
        })
        .from([Picked])
        .to(Dispatched);
    let send = with_problem(send, "has_lines", |i| i.lines_problem.clone());
    let send = with_problem(send, "no_picked_lot_on_hold", |i| {
        i.lot_problem.clone().or_else(|| i.stock_problem.clone())
    });
    let send = with_problem(send, "handover_chosen", |i| i.handover_problem.clone());
    let send = with_problem(send, "credit_recheck", |i| i.credit_problem.clone());
    let send = send.effect(|dispatch: &Dispatch, ctx, tx| {
        let dispatch = dispatch.clone();
        let run = input(ctx).unwrap_or_default();
        Box::pin(async move { send_in(tx, dispatch, &run).await })
    });
    machine.transition(crate::search::reindex_dispatch(send))
}

/// The text of a failed credit check.
fn credit_text(failure: wl_kernel::sales::CreditFailure) -> String {
    let key = format!("wl_inventory.dispatch.credit_{}", failure.as_str());
    said(&key, &[])
}

/// Asks `wl-delivery` for the delivery of `dispatch`, an own-delivery
/// hand-over.
async fn request_delivery(state: &AppState, dispatch: &Dispatch) -> Result<i64> {
    let lines = lines_of(&state.db, dispatch.id).await?;
    let mut request = DeliveryRequest::new(dispatch.subject(), &dispatch.recipient_name);
    request.recipient_party = dispatch.recipient_party_id.map(wl_kernel::PartyId);
    request.address = dispatch.address.clone();
    for line in &lines {
        let item = Item::find_or_404(&state.db, line.item_id).await?;
        request = request.item(
            DeliveryItem::new(&item.name_en, line.quantity_milli).source_line(line.subject()),
        );
    }
    let delivery = wl_kernel::delivery::requester(state)
        .request(state, request)
        .await?;
    Ok(delivery.0)
}

/// `user` dispatches picked dispatch `id` with the hand-over `choice`.
/// Refused with the first problem before anything moves: a lot on hold says
/// "re-pick", a failed credit re-check puts the order on `credit_hold`; in both
/// cases the dispatch stays `picked` and its stock reserved.
///
/// # Errors
///
/// `Forbidden` without `dispatch.manage` in the location's scope; `NotFrom`
/// unless `picked`; `Guard` (422) for a problem of the dispatch.
pub async fn dispatch(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    choice: &HandoverChoice,
) -> std::result::Result<Dispatch, TransitionError> {
    let dispatch = Dispatch::find_or_404(&state.db, id).await?;
    if !user.has_permission_in(permissions::DISPATCH_MANAGE, &dispatch.scope()) {
        return Err(TransitionError::Forbidden);
    }
    if dispatch.status != DispatchStatus::Picked {
        return Err(TransitionError::NotFrom(
            dispatch.status.as_str().to_owned(),
        ));
    }
    // Reads in a transaction of its own, rolled back: nothing is held while
    // the credit check and the transition open theirs.
    let mut tx = state.db.begin().await?;
    let mut found = gather(&mut tx, &dispatch, Some(choice), Some(user.id)).await?;
    tx.rollback().await?;
    // The item flags and the documents of the vertical (`MFG.ACL.11`) again on
    // the dispatch date: a certificate that expired since the pick refuses it
    // and the dispatch can be cancelled.
    if !found.problem_before_credit() {
        crate::dispatch_guard::check(
            state,
            crate::dispatch_guard::DISPATCH,
            &dispatch,
            Some(user.id),
            wl_kernel::documents::today(),
        )
        .await?;
    }
    if !found.problem_before_credit() && dispatch.source_type == SALES_ORDER_SUBJECT_TYPE {
        let verdict = wl_kernel::sales::credit_checker(state)
            .credit_recheck(state, SalesOrderId(dispatch.source_id))
            .await?;
        if let CreditVerdict::Failed(failure) = verdict {
            found.credit_problem = Some(credit_text(failure));
        }
    }
    if let Some(problem) = found.first_problem() {
        return Err(TransitionError::Guard(problem.to_owned()));
    }
    if Handover::parse(&choice.kind) == Some(Handover::OwnDelivery) {
        found.delivery = Some(request_delivery(state, &dispatch).await?);
    }
    let outcome = dispatch
        .transition(DISPATCH)
        .by(user)
        .input(&found)
        .run(state)
        .await;
    if let Err(error) = outcome {
        if let Some(delivery) = found.delivery {
            let reason = said("wl_inventory.dispatch.delivery_cancelled", &[]);
            let _ = wl_kernel::delivery::requester(state)
                .cancel(state, wl_kernel::delivery::DeliveryId(delivery), &reason)
                .await;
        }
        return Err(error);
    }
    Ok(Dispatch::find_or_404(&state.db, id).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_problem_puts_the_lot_before_the_credit() {
        let mut input = SendInput {
            credit_problem: Some("credit".into()),
            ..SendInput::default()
        };
        assert_eq!(input.first_problem(), Some("credit"));
        assert!(!input.problem_before_credit());
        input.handover_problem = Some("handover".into());
        assert_eq!(input.first_problem(), Some("handover"));
        input.stock_problem = Some("stock".into());
        input.lot_problem = Some("lot".into());
        assert_eq!(input.first_problem(), Some("lot"));
        assert!(input.problem_before_credit());
    }

    #[test]
    fn dispatch_leaves_a_picked_dispatch_after_four_guards() {
        let machine = Dispatch::machine();
        let send = machine.find(DISPATCH).expect("dispatch");
        assert_eq!(send.from_states(), ["picked"]);
        assert_eq!(send.to_state(), Some("dispatched"));
        assert_eq!(
            send.guard_names(),
            [
                "has_lines",
                "no_picked_lot_on_hold",
                "handover_chosen",
                "credit_recheck"
            ]
        );
        assert!(!send.is_system());
    }

    #[test]
    fn a_handover_needs_what_its_kind_asks() {
        let dispatch = Dispatch {
            recipient_name: "Al Noor Trading".into(),
            ..Dispatch::default()
        };
        assert!(handover_problem(&dispatch, None).is_some());
        assert!(handover_problem(&dispatch, Some(&HandoverChoice::own_delivery())).is_none());
        assert!(handover_problem(&dispatch, Some(&HandoverChoice::courier(" "))).is_some());
        assert!(handover_problem(&dispatch, Some(&HandoverChoice::courier("TRK-1"))).is_none());
        assert!(handover_problem(&dispatch, Some(&HandoverChoice::collection(""))).is_some());
        assert!(handover_problem(&dispatch, Some(&HandoverChoice::collection("SIG-9"))).is_none());
        let nameless = Dispatch::default();
        assert!(handover_problem(&nameless, Some(&HandoverChoice::own_delivery())).is_some());
    }
}
