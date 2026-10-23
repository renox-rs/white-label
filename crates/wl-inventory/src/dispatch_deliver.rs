//! Confirming a delivery per line and taking the refused goods to quarantine
//! (wl-inventory §4 *Dispatch* row `confirm_delivery` and the paragraph
//! *Refused goods coming back*, §5.4 step 4, issue #241).
//!
//! - `confirm_delivery` moves a `dispatched` dispatch to `delivered` with the
//!   quantity delivered and refused per line. The system runs it on
//!   `DeliveryDelivered` of `wl-delivery` (the quantities of the proof); for a
//!   courier or a collection a `dispatch.manage` user runs it with the
//!   quantities from the courier's proof ([`confirm`]). The refused quantity
//!   is recorded *in return* with its refusal reason and posts **no
//!   movement**: the goods are still with the driver. Every refused quantity
//!   needs a reason, delivered plus refused cannot be above what was sent,
//!   and a manual confirmation names every line. `DispatchDelivered` follows
//!   the commit, with the delivered and the refused quantity per line
//!   ([`crate::dispatch_events`]); the source fulfils and bills the delivered
//!   quantity only.
//! - [`receive_refused`] takes the refused goods back: when the run closes,
//!   `DeliveryRefusedReceived` gives the quantity received back and the
//!   quantity missing per line. What came back is posted as a `return_in`
//!   movement into **quarantine** (never free stock) with the refusal reason
//!   and the lot picked for the line, whatever the reason (damaged, wrong
//!   item, not ordered, short-dated, temperature, other). What is missing is a
//!   loss of its value through the cost poster (reason *missing on return*)
//!   and the dispatcher gets a task. The state of the dispatch does not
//!   change. For a courier, `dispatch.manage` records the same by hand.
//!
//! Decisions:
//!
//! - one confirmation per proof of delivery: the proof's id is kept on the
//!   dispatch and a repeated event does nothing; a repeated
//!   `DeliveryRefusedReceived` does nothing either, because the quantity is
//!   no longer *in return*;
//! - a line the event does not name is left with nothing delivered or refused;
//! - after a delivery reversal the refused quantity stays: a new
//!   confirmation keeps it (at least as much as was refused before) and takes
//!   delivered only from the remaining quantity;
//! - the loss of a missing quantity is posted at once. `STK.ACL.02` asks a
//!   second person above the value, but the goods are gone and the event is
//!   the system's, so no approval can wait here: the dispatcher's task says
//!   that the loss is above the limit and needs the second person.

use std::collections::BTreeMap;

use renox::auth::permissions::users_with_permission_in;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::delivery::DeliveryDelivered;
use wl_kernel::inventory::{
    DispatchDelivered, DispatchGoodsQuarantined, ITEM_SUBJECT_TYPE, LOT_SUBJECT_TYPE,
    QuarantinedLine,
};
use wl_kernel::invoicing::{PostCost, post_cost};
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, task_creator};

use crate::config_of;
use crate::dispatch::{Dispatch, DispatchLine, DispatchStatus, Handover, lines_of, refused};
use crate::dispatch_events::{dispatch_of, event_lines, line_id, recipient};
use crate::item::{quantity_text, value_of};
use crate::location;
use crate::movement::{MovementType, StockMovement};
use crate::permissions;
use crate::shortage_alert::first_not_admin;
use crate::texts::said;

/// The user's `confirm_delivery` transition's name (a courier's proof,
/// entered by hand).
pub const CONFIRM_DELIVERY: &str = "confirm_delivery";

/// The system's `confirm_delivery` transition's name: the kernel's
/// `DeliveryDelivered` of `wl-delivery` runs it.
pub const CONFIRM_DELIVERY_BY_DELIVERY: &str = "confirm_delivery_by_delivery";

/// The delivered and refused quantity of one line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineQuantity {
    /// The dispatch line.
    pub line_id: i64,
    /// The quantity accepted, in thousandths.
    pub delivered_milli: i64,
    /// The quantity refused, in thousandths.
    pub refused_milli: i64,
    /// The refusal reason's code.
    pub reason: Option<String>,
}

/// What a `confirm_delivery` run carries to its guards and its effect.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ConfirmInput {
    /// The user who confirms it; `None` for the system.
    pub by: Option<i64>,
    /// The proof of delivery of `wl-delivery` that confirmed it.
    pub proof: Option<i64>,
    /// The quantities per line.
    pub lines: Vec<LineQuantity>,
    /// Whether a user enters them (every line must be named).
    pub manual: bool,
    /// What is wrong with them, `None` when nothing is.
    pub problem: Option<String>,
}

/// What is wrong with `entered` for `dispatch`, `None` when nothing is.
///
/// # Errors
///
/// A database error.
pub async fn check(
    tx: &mut Transaction,
    dispatch: &Dispatch,
    entered: &[LineQuantity],
    manual: bool,
) -> Result<Option<String>> {
    let lines = lines_of(&mut *tx, dispatch.id).await?;
    if manual && dispatch.handover.as_deref() == Some(Handover::OwnDelivery.as_str()) {
        return Ok(Some(said("wl_inventory.dispatch.proof_from_delivery", &[])));
    }
    for line in &lines {
        let position = line.position.to_string();
        let Some(given) = entered.iter().find(|given| given.line_id == line.id) else {
            if manual {
                return Ok(Some(said(
                    "wl_inventory.dispatch.line_missing",
                    &[("position", &position)],
                )));
            }
            continue;
        };
        if given.delivered_milli < 0 || given.refused_milli < 0 {
            return Ok(Some(said(
                "wl_inventory.dispatch.bad_quantity",
                &[("position", &position)],
            )));
        }
        if given.delivered_milli + given.refused_milli > line.quantity_milli {
            return Ok(Some(said(
                "wl_inventory.dispatch.too_much",
                &[
                    ("position", &position),
                    ("quantity", &quantity_text(line.quantity_milli)),
                ],
            )));
        }
        let reason_given = given
            .reason
            .as_deref()
            .is_some_and(|reason| !reason.trim().is_empty());
        if given.refused_milli > 0 && !reason_given && line.refusal_reason.is_none() {
            return Ok(Some(said(
                "wl_inventory.dispatch.reason_needed",
                &[("position", &position)],
            )));
        }
    }
    if manual && !entered.iter().any(|given| given.delivered_milli > 0) && entered.is_empty() {
        return Ok(Some(said("wl_inventory.dispatch.nothing_entered", &[])));
    }
    Ok(None)
}

fn input(ctx: &TransitionCtx) -> Option<ConfirmInput> {
    ctx.input_as::<ConfirmInput>().ok()
}

/// The guards and the effect the two confirm transitions share.
fn rules(transition: Transition<Dispatch>) -> Transition<Dispatch> {
    transition
        .from([DispatchStatus::Dispatched])
        .to(DispatchStatus::Delivered)
        .guard("quantities_add_up", |_dispatch: &Dispatch, ctx| {
            let problem = input(ctx).map_or_else(
                || Some(said("wl_inventory.dispatch.not_dispatched", &[])),
                |input| input.problem,
            );
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|dispatch: &Dispatch, ctx, tx| {
            let dispatch = dispatch.clone();
            let run = input(ctx).unwrap_or_default();
            Box::pin(async move { confirm_in(tx, dispatch, &run).await })
        })
}

/// The effect of the confirm transitions: the delivered and the refused
/// quantity per line. No movement.
async fn confirm_in(tx: &mut Transaction, dispatch: Dispatch, run: &ConfirmInput) -> Result {
    let now = renox::db::now();
    for line in lines_of(&mut *tx, dispatch.id).await? {
        let given = run.lines.iter().find(|given| given.line_id == line.id);
        // The refused quantity of an earlier delivery stays; the new
        // delivery takes only what is left.
        let entered_refused = given.map_or(0, |given| given.refused_milli);
        let refused_milli = line.refused_milli.max(entered_refused);
        let delivered_milli = given.map_or(0, |given| {
            given
                .delivered_milli
                .min((line.quantity_milli - refused_milli).max(0))
        });
        let reason = given
            .and_then(|given| given.reason.as_deref())
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .map(str::to_owned)
            .or(line.refusal_reason.clone())
            .filter(|_| refused_milli > 0);
        DispatchLine::query()
            .where_(DispatchLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[
                    ("delivered_milli", &delivered_milli),
                    ("refused_milli", &refused_milli),
                    ("refusal_reason", &reason),
                    ("updated_at", &now),
                ],
            )
            .await?;
    }
    Dispatch::query()
        .where_(Dispatch::ID.eq(dispatch.id))
        .update(
            &mut *tx,
            &[
                ("delivered_at", &Some(now)),
                ("proof_id", &run.proof),
                ("proof_reversed", &false),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// `confirm_delivery` for a user and for the system.
pub(crate) fn add_to(machine: Machine<Dispatch>) -> Machine<Dispatch> {
    let by_user = rules(
        Transition::new(CONFIRM_DELIVERY)
            .label("Confirm delivery")
            .tone("primary")
            .permission_in(permissions::DISPATCH_MANAGE, |dispatch: &Dispatch| {
                dispatch.scope()
            }),
    );
    let by_system = rules(
        Transition::new(CONFIRM_DELIVERY_BY_DELIVERY)
            .label("Confirm delivery (proof of delivery)")
            .system(),
    );
    machine.transition(by_user).transition(by_system)
}

/// `user` confirms the delivery of dispatch `id` of a courier or a
/// collection, with the quantities `entered` from the proof (every line).
///
/// # Errors
///
/// `Forbidden` without `dispatch.manage` in the location's scope; `NotFrom`
/// unless `dispatched`; `Guard` (422) when a line is missing, delivered plus
/// refused is above what was sent or a refused quantity has no reason.
pub async fn confirm(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    entered: Vec<LineQuantity>,
) -> std::result::Result<Dispatch, TransitionError> {
    let dispatch = Dispatch::find_or_404(&state.db, id).await?;
    let mut tx = state.db.begin().await?;
    let problem = check(&mut tx, &dispatch, &entered, true).await?;
    tx.rollback().await?;
    let run = ConfirmInput {
        by: Some(user.id),
        proof: None,
        lines: entered,
        manual: true,
        problem,
    };
    let _outcome = dispatch
        .transition(CONFIRM_DELIVERY)
        .by(user)
        .input(&run)
        .run(state)
        .await?;
    Ok(Dispatch::find_or_404(&state.db, id).await?)
}

/// Confirms `dispatch` as the system from the quantities of a proof of
/// delivery. Does nothing when the dispatch is not `dispatched` or the proof
/// already confirmed it (one `confirm_delivery` per proof).
///
/// # Errors
///
/// A database error, or a refusal of the transition (quantities above what
/// was sent).
pub async fn confirm_from_delivery(
    state: &AppState,
    dispatch: &Dispatch,
    proof: i64,
    entered: Vec<LineQuantity>,
) -> Result<bool> {
    if dispatch.status != DispatchStatus::Dispatched || dispatch.proof_id == Some(proof) {
        return Ok(false);
    }
    let mut tx = state.db.begin().await?;
    let problem = check(&mut tx, dispatch, &entered, false).await?;
    tx.rollback().await?;
    let run = ConfirmInput {
        by: None,
        proof: Some(proof),
        lines: entered,
        manual: false,
        problem,
    };
    let _outcome = dispatch
        .transition(CONFIRM_DELIVERY_BY_DELIVERY)
        .by_system()
        .input(&run)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(true)
}

/// The refused goods of one line that came back or are missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedEntry {
    /// The dispatch line.
    pub line_id: i64,
    /// The quantity received back, in thousandths.
    pub received_milli: i64,
    /// The quantity reported missing, in thousandths.
    pub missing_milli: i64,
}

/// What [`receive_refused`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReceivedOutcome {
    /// The quantity posted into quarantine, in thousandths.
    pub received_milli: i64,
    /// The quantity written off as missing, in thousandths.
    pub missing_milli: i64,
    /// The value written off, in fils.
    pub loss_fils: i64,
    /// What was posted into quarantine: the line and the quantity.
    pub quarantined: Vec<(i64, i64)>,
}

/// Takes the refused goods of dispatch `id` back: `return_in` into
/// quarantine for what `entries` received (with the refusal reason and the
/// picked lot), a loss and a task for the dispatcher for what is missing. The
/// dispatch keeps its state. `by` is the user, `None` for the system.
///
/// With `strict`, a quantity above what is still *in return* is a 422; the
/// system's call (`strict` off) skips such a line, which makes a repeated
/// event harmless.
///
/// # Errors
///
/// 422 for a dispatch that has not left, a line that is not its own or (when
/// `strict`) a quantity above what is in return.
pub async fn receive_refused(
    state: &AppState,
    by: Option<i64>,
    id: i64,
    entries: &[ReceivedEntry],
    strict: bool,
) -> Result<ReceivedOutcome> {
    let dispatch = Dispatch::find_or_404(&state.db, id).await?;
    if !matches!(
        dispatch.status,
        DispatchStatus::Delivered | DispatchStatus::Dispatched | DispatchStatus::Lost
    ) {
        return Err(refused(said("wl_inventory.dispatch.not_left", &[])));
    }
    let mut outcome = ReceivedOutcome::default();
    let mut missing_lines: Vec<(DispatchLine, i64)> = Vec::new();
    let now = renox::db::now();
    let mut tx = state.db.begin().await?;
    let lines: BTreeMap<i64, DispatchLine> = lines_of(&mut tx, id)
        .await?
        .into_iter()
        .map(|line| (line.id, line))
        .collect();
    for entry in entries {
        let Some(line) = lines.get(&entry.line_id) else {
            if strict {
                tx.rollback().await?;
                return Err(Error::NotFound);
            }
            continue;
        };
        let position = line.position.to_string();
        if entry.received_milli < 0 || entry.missing_milli < 0 {
            tx.rollback().await?;
            return Err(refused(said(
                "wl_inventory.dispatch.bad_quantity",
                &[("position", &position)],
            )));
        }
        let wanted = entry.received_milli + entry.missing_milli;
        if wanted == 0 {
            continue;
        }
        if wanted > line.in_return_milli() {
            if strict {
                tx.rollback().await?;
                return Err(refused(said(
                    "wl_inventory.dispatch.more_than_in_return",
                    &[
                        ("position", &position),
                        ("quantity", &quantity_text(line.in_return_milli())),
                    ],
                )));
            }
            continue;
        }
        if entry.received_milli > 0 {
            let quarantine = location::quarantine_location(&mut tx).await?;
            let mut moved = StockMovement::moving(
                MovementType::ReturnIn,
                line.item_id,
                line.lot_id,
                None,
                Some(quarantine),
                entry.received_milli,
                line.unit_cost,
                value_of(entry.received_milli, line.unit_cost),
                &dispatch.subject(),
                by,
            );
            moved.reason = line.refusal_reason.clone();
            moved.moved_at = now;
            moved.insert(&mut tx).await?;
        }
        let returned = line.returned_milli + entry.received_milli;
        let missing = line.missing_milli + entry.missing_milli;
        DispatchLine::query()
            .where_(DispatchLine::ID.eq(line.id))
            .update(
                &mut tx,
                &[
                    ("returned_milli", &returned),
                    ("missing_milli", &missing),
                    ("updated_at", &now),
                ],
            )
            .await?;
        outcome.received_milli += entry.received_milli;
        if entry.received_milli > 0 {
            outcome.quarantined.push((line.id, entry.received_milli));
        }
        if entry.missing_milli > 0 {
            outcome.missing_milli += entry.missing_milli;
            outcome.loss_fils += value_of(entry.missing_milli, line.unit_cost);
            missing_lines.push((line.clone(), missing));
        }
    }
    tx.commit().await?;
    for (line, missing_total) in &missing_lines {
        let amount = value_of(missing_total - line.missing_milli, line.unit_cost);
        let request = PostCost::new(
            dispatch.subject(),
            format!("inventory.dispatch_missing:{}:{missing_total}", line.id),
            dispatch.subject(),
            amount,
        )
        .cost_code("missing_on_return");
        post_cost(state, request).await?;
    }
    if outcome.missing_milli > 0 {
        open_missing_task(state, &dispatch, &outcome).await?;
    }
    if !outcome.quarantined.is_empty() {
        announce_quarantine(state, &dispatch, &outcome).await?;
    }
    Ok(outcome)
}

/// Says that refused goods were booked into quarantine
/// (`DispatchGoodsQuarantined`), so a vertical can inspect each lot.
async fn announce_quarantine(
    state: &AppState,
    dispatch: &Dispatch,
    outcome: &ReceivedOutcome,
) -> Result<()> {
    let lines = lines_of(&state.db, dispatch.id).await?;
    let booked: Vec<QuarantinedLine> = outcome
        .quarantined
        .iter()
        .filter_map(|(id, quantity)| {
            let line = lines.iter().find(|line| line.id == *id)?;
            Some(QuarantinedLine {
                line: line.subject(),
                item: Subject::new(ITEM_SUBJECT_TYPE, line.item_id),
                lot: line.lot_id.map(|id| Subject::new(LOT_SUBJECT_TYPE, id)),
                source_line: line.source_line(),
                quantity_milli: *quantity,
                reason: line.refusal_reason.clone(),
            })
        })
        .collect();
    state
        .emit(DispatchGoodsQuarantined {
            dispatch: dispatch.subject(),
            source: dispatch.source(),
            recipient: recipient(dispatch),
            returned: false,
            complaint: None,
            lines: booked,
        })
        .await
}

/// The dispatcher's task for goods missing on return: one open task per
/// dispatch.
async fn open_missing_task(
    state: &AppState,
    dispatch: &Dispatch,
    outcome: &ReceivedOutcome,
) -> Result {
    let creator = task_creator(state);
    let key = format!("inventory.dispatch_missing:{}", dispatch.id);
    if creator.find_open(state, &key).await?.is_some() {
        return Ok(());
    }
    let number = dispatch.number.clone().unwrap_or_default();
    let title = said(
        "wl_inventory.dispatch.missing_title",
        &[("number", &number)],
    );
    let above = outcome.loss_fils > config_of(state).wastage_approval_fils;
    let mut description = said(
        "wl_inventory.dispatch.missing_body",
        &[
            ("number", &number),
            ("quantity", &quantity_text(outcome.missing_milli)),
        ],
    );
    if above {
        description.push(' ');
        description.push_str(&said("wl_inventory.dispatch.missing_approval", &[]));
    }
    let holders: Vec<i64> =
        users_with_permission_in(&state.db, permissions::DISPATCH_MANAGE, &dispatch.scope())
            .await?
            .iter()
            .map(|user| user.id)
            .collect();
    let mut ticket = OpenTicket::new(dispatch.subject(), TASK_CATEGORY, &title)
        .description(&description)
        .source_key(&key);
    let first = match first_not_admin(&state.db, &holders).await? {
        Some(id) => Some(id),
        None => holders.first().copied(),
    };
    if let Some(first) = first {
        ticket = ticket.assign_to(Assignee::user(first));
    }
    creator.open(state, ticket).await?;
    Ok(())
}

/// Registers the listeners: `DispatchDelivered` after a confirmation, and
/// the events of `wl-delivery` that confirm a delivery and bring the refused
/// goods back.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<Dispatch>, state: AppState| async move {
            if ![CONFIRM_DELIVERY, CONFIRM_DELIVERY_BY_DELIVERY]
                .contains(&event.transition.as_str())
            {
                return Ok(());
            }
            let dispatch = Dispatch::find_or_404(&state.db, event.key).await?;
            let lines = lines_of(&state.db, dispatch.id).await?;
            state
                .emit(DispatchDelivered {
                    dispatch: dispatch.subject(),
                    source: dispatch.source(),
                    recipient: recipient(&dispatch),
                    lines: event_lines(&lines),
                })
                .await
        },
    );
    app.listen(|event: DeliveryDelivered, state: AppState| async move {
        let Some(dispatch) = dispatch_of(&state, &event.source).await? else {
            return Ok(());
        };
        let entered: Vec<LineQuantity> = event
            .lines
            .iter()
            .filter_map(|line| {
                Some(LineQuantity {
                    line_id: line_id(&line.line)?,
                    delivered_milli: line.delivered_milli,
                    refused_milli: line.refused_milli,
                    reason: line.refusal_reason.clone(),
                })
            })
            .collect();
        confirm_from_delivery(&state, &dispatch, event.proof, entered).await?;
        Ok(())
    });
    app.listen(
        |event: wl_kernel::delivery::DeliveryRefusedReceived, state: AppState| async move {
            let Some(dispatch) = dispatch_of(&state, &event.source).await? else {
                return Ok(());
            };
            let entries: Vec<ReceivedEntry> = event
                .lines
                .iter()
                .filter_map(|line| {
                    Some(ReceivedEntry {
                        line_id: line_id(&line.line)?,
                        received_milli: line.received_milli,
                        missing_milli: line.missing_milli,
                    })
                })
                .collect();
            receive_refused(&state, None, dispatch.id, &entries, false).await?;
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delivery_is_confirmed_by_a_user_or_by_the_system_from_dispatched() {
        let machine = Dispatch::machine();
        let user = machine.find(CONFIRM_DELIVERY).expect("confirm_delivery");
        let system = machine
            .find(CONFIRM_DELIVERY_BY_DELIVERY)
            .expect("the system's");
        assert!(!user.is_system() && system.is_system());
        for transition in [user, system] {
            assert_eq!(transition.from_states(), ["dispatched"]);
            assert_eq!(transition.to_state(), Some("delivered"));
            assert_eq!(transition.guard_names(), ["quantities_add_up"]);
        }
    }

    #[test]
    fn the_confirm_input_defaults_to_nothing_wrong() {
        let input = ConfirmInput::default();
        assert!(input.problem.is_none() && !input.manual && input.lines.is_empty());
    }
}
