//! A lost shipment and the goods found again (03 Logistics §4.1 row *Shipment
//! cannot be found*, §4.4 steps 3 and 4, §7.1 rows `mark_lost` and `found`,
//! `WRK.RUL.17`, `LOG.ACL.14`; BL-LOG-11, BL-LOG-12).
//!
//! - **`mark_lost`** has three ways in, one set of effects. On a delivery the
//!   system runs `mark_lost_system` when wl-delivery marks the delivery lost
//!   (`DeliveryLost`, `WRK.RUL.17`, liability *none*). Off a delivery a user
//!   with `shipments.manage` asks with a reason and a liability and another
//!   holder of `shipments.approve_loss` approves it: `mark_lost`, deferred by
//!   the kernel into an approval request, and `mark_lost_approved` once
//!   approved ([`crate::trace`], BL-LOG-12).
//! - **The effects** ([`record`], [`settled`]): the shipment is `lost`, expects
//!   no COD (what was expected is kept to expect again when it is found; COD
//!   already remitted is not taken back, customer service decides through the
//!   claim), and leaves billing ([`crate::shipment::Shipment::billable`]; an
//!   invoiced one gets a credit due and the invoice owner a task,
//!   [`crate::credit`]). One claim ticket is linked: the one wl-delivery opened
//!   for the delivery's loss, else this module opens it, whatever the
//!   liability. The client is told once, and `ShipmentLost` is emitted.
//! - **`found`** (`shipments.scan`, or the system on `DeliveryFound`): the
//!   shipment is `at_hub` again and expects its COD again; the client is told
//!   and the claim ticket gets a comment *goods found at <depot>*. After a
//!   settled claim ([`settle_claim`]) the comment says the claim is reopened
//!   and the system holds the shipment (`hold_system`, *client instruction*)
//!   until the client chooses ([`choose`]): *delivery* drafts an invoice for
//!   the claim credit and releases the shipment, *keep the settlement* sends
//!   it back to the client, or, as the client instructs, disposes of it
//!   ([`choose_keep_and_dispose`], BL-LOG-06).

use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::delivery::{DeliveryFound, DeliveryLost};
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory};
use wl_kernel::tickets::{self, TaskComment, TicketId};

use crate::claim;
use crate::credit;
use crate::disposal::{DisposalInput, DisposeRequest};
use crate::hold;
use crate::notices::tell_client;
use crate::permissions;
use crate::rates::aed;
use crate::shipment::{Shipment, ShipmentLost, ShipmentStatus, hold_reason, track};
use crate::shipment_machine::{FOUND, FOUND_SYSTEM, HOLD_SYSTEM, MARK_LOST_SYSTEM, ShipmentInput};
use crate::texts::english;

/// Who can carry a loss.
pub const LIABILITIES: [&str; 4] = ["driver", "business", "client", "none"];

/// What the client chose after a settled claim: the goods are delivered.
pub const CHOICE_DELIVERY: &str = "delivery";
/// What the client chose after a settled claim: keep the settlement.
pub const CHOICE_KEEP: &str = "keep";

/// The states a shipment can be lost from.
pub const LOSABLE: [ShipmentStatus; 7] = [
    ShipmentStatus::OutForDelivery,
    ShipmentStatus::Attempted,
    ShipmentStatus::UnderReview,
    ShipmentStatus::PickedUp,
    ShipmentStatus::AtHub,
    ShipmentStatus::OnHold,
    ShipmentStatus::Returning,
];

/// The states from which a shipment is on no delivery: marked lost by a user
/// with an approval (`LOG.ACL.14`).
pub const OFF_DELIVERY: [ShipmentStatus; 4] = [
    ShipmentStatus::PickedUp,
    ShipmentStatus::AtHub,
    ShipmentStatus::OnHold,
    ShipmentStatus::Returning,
];

/// What a loss carries: the user's request, which is also the approval
/// request's payload, or the delivery module's event.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LossInput {
    /// Why it is lost.
    #[serde(default)]
    pub reason: String,
    /// Who bears the loss: one of [`LIABILITIES`].
    #[serde(default)]
    pub liability: String,
    /// The claim ticket the delivery module opened for the loss, if it did.
    #[serde(default)]
    pub claim_ticket: Option<i64>,
}

/// The guard "reason and liability" of a manual loss.
pub(crate) fn reason_and_liability(input: Option<&LossInput>) -> std::result::Result<(), String> {
    match input {
        None => Err(english("logistics.loss.not_prepared", &[])),
        Some(input) if input.reason.trim().is_empty() => {
            Err(english("logistics.loss.reason_needed", &[]))
        }
        Some(input) if !LIABILITIES.contains(&input.liability.as_str()) => {
            Err(english("logistics.loss.liability_needed", &[]))
        }
        Some(_) => Ok(()),
    }
}

/// The effect of every way to `lost`, inside the transition: the reason, the
/// state it was lost from, the liability and the claim ticket of the delivery
/// are stored, and no COD is expected any more.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn record(
    tx: &mut Transaction,
    shipment: &Shipment,
    input: &LossInput,
    actor: Option<i64>,
) -> Result {
    let at = Some(renox::db::now());
    let from = Some(shipment.status.as_str().to_owned());
    let reason = input.reason.trim().to_owned();
    let reason_value = Some(reason.clone());
    let liability = Some(if input.liability.trim().is_empty() {
        "none".to_owned()
    } else {
        input.liability.trim().to_owned()
    });
    let expected: i64 = 0;
    let ticket = input.claim_ticket;
    let no_number: Option<String> = None;
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(
            &mut *tx,
            &[
                ("lost_reason", &reason_value),
                ("lost_at", &at),
                ("lost_from", &from),
                ("liability", &liability),
                ("lost_cod", &shipment.cod_amount),
                ("cod_amount", &expected),
                ("claim_ticket_id", &ticket),
                ("claim_ticket_number", &no_number),
            ],
        )
        .await?;
    track(&mut *tx, shipment.id, "lost", &reason, actor).await
}

/// The effect of `found`, inside the transition: the depot is stored and the
/// COD the shipment expected before it was lost is expected again.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn record_found(
    tx: &mut Transaction,
    shipment: &Shipment,
    depot: &str,
    actor: Option<i64>,
) -> Result {
    let depot_value = Some(depot.trim().to_owned());
    let expected = if shipment.lost_cod > 0 {
        shipment.lost_cod
    } else {
        shipment.cod_amount
    };
    let none: i64 = 0;
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(
            &mut *tx,
            &[
                ("found_depot", &depot_value),
                ("cod_amount", &expected),
                ("lost_cod", &none),
            ],
        )
        .await?;
    track(&mut *tx, shipment.id, "found", depot.trim(), actor).await
}

/// Runs the system's `mark_lost_system` of `shipment` for `event`
/// (`WRK.RUL.17`): the delivery was untraced and is lost. A shipment that
/// cannot be lost any more is left alone.
///
/// # Errors
///
/// Database errors and the engine's.
pub(crate) async fn follow_lost(
    state: &AppState,
    shipment: Shipment,
    event: &DeliveryLost,
) -> Result {
    if !LOSABLE.contains(&shipment.status) {
        return Ok(());
    }
    let input = LossInput {
        reason: english("logistics.loss.untraced", &[]),
        liability: event.liability.clone(),
        claim_ticket: event.claim_ticket.map(|ticket| ticket.0),
    };
    let _outcome = shipment
        .transition(MARK_LOST_SYSTEM)
        .by_system()
        .input(&ShipmentInput {
            loss: Some(input),
            ..ShipmentInput::default()
        })
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// Runs the system's `found_system` of `shipment` for `event`: the delivery
/// was found at a depot. A shipment that is not lost is left alone.
///
/// # Errors
///
/// Database errors and the engine's.
pub(crate) async fn follow_found(
    state: &AppState,
    shipment: Shipment,
    event: &DeliveryFound,
) -> Result {
    if shipment.status != ShipmentStatus::Lost {
        return Ok(());
    }
    let _outcome = shipment
        .transition(FOUND_SYSTEM)
        .by_system()
        .input(&ShipmentInput {
            depot: Some(event.depot.clone()),
            ..ShipmentInput::default()
        })
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// What follows the commit of a loss of shipment `id`: the credit due of an
/// invoiced one, the claim ticket, the client's notice and `ShipmentLost`.
///
/// # Errors
///
/// Database errors and the ticketing, notifier and event ones.
pub(crate) async fn settled(state: &AppState, id: i64) -> Result {
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    credit::due(state, &shipment, "lost", shipment.id).await?;
    if shipment.claim_ticket_id.is_none() {
        // A lost shipment always gets a claim, whatever the liability.
        let title = english("logistics.loss.claim_title", &[("number", &shipment.number)]);
        let description = english(
            "logistics.loss.claim_description",
            &[
                ("number", &shipment.number),
                ("reason", &shipment.lost_reason.clone().unwrap_or_default()),
                ("liability", &shipment.liability.clone().unwrap_or_default()),
            ],
        );
        claim::open(
            state,
            &shipment,
            &format!("logistics.loss_claim:{}", shipment.id),
            &title,
            &description,
        )
        .await?;
    }
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    let title = english("logistics.loss.title", &[("number", &shipment.number)]);
    let body = match (&shipment.claim_ticket_number, shipment.claim_ticket_id) {
        (Some(number), _) => english(
            "logistics.loss.body_claim",
            &[("number", &shipment.number), ("claim", number)],
        ),
        (None, Some(ticket)) => english(
            "logistics.loss.body_claim",
            &[("number", &shipment.number), ("claim", &ticket)],
        ),
        (None, None) => english("logistics.loss.body", &[("number", &shipment.number)]),
    };
    tell_client(state, &shipment, &title, &body).await?;
    state
        .emit(ShipmentLost {
            shipment: shipment.id,
            number: shipment.number.clone(),
            client: shipment.client(),
            from: shipment.lost_from.clone().unwrap_or_default(),
            reason: shipment.lost_reason.clone().unwrap_or_default(),
            liability: shipment.liability.clone().unwrap_or_else(|| "none".to_owned()),
            claim_ticket: shipment.claim_ticket_id,
        })
        .await
}

/// What follows the commit of `found` of shipment `id`: the client is told,
/// the claim ticket gets a comment (reopened when the claim was settled) and,
/// after a settled claim, the system holds the shipment until the client
/// chooses.
///
/// # Errors
///
/// Database errors and the ticketing and notifier ones.
pub(crate) async fn found_settled(state: &AppState, id: i64) -> Result {
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    let depot = shipment.found_depot.clone().unwrap_or_default();
    let title = english("logistics.loss.found_title", &[("number", &shipment.number)]);
    let body = english(
        "logistics.loss.found_body",
        &[("number", &shipment.number), ("depot", &depot)],
    );
    tell_client(state, &shipment, &title, &body).await?;
    let settled = shipment.claim_settled_amount > 0 && shipment.claim_choice.is_none();
    if let Some(ticket) = shipment.claim_ticket_id {
        let key = if settled {
            "logistics.loss.found_comment_reopened"
        } else {
            "logistics.loss.found_comment"
        };
        let text = english(
            key,
            &[
                ("depot", &depot),
                ("amount", &aed(shipment.claim_settled_amount)),
            ],
        );
        tickets::task_creator(state)
            .comment(state, TicketId(ticket), TaskComment::internal(&text))
            .await?;
    }
    if settled {
        // Not dispatched before the client chooses.
        let _outcome = shipment
            .transition(HOLD_SYSTEM)
            .by_system()
            .input(&ShipmentInput {
                hold_reason: Some(hold_reason::CLIENT_INSTRUCTION.to_owned()),
                ..ShipmentInput::default()
            })
            .run(state)
            .await
            .map_err(TransitionError::into_error)?;
    }
    Ok(())
}

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

async fn load(state: &AppState, id: i64) -> std::result::Result<Shipment, TransitionError> {
    Shipment::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// `user` (`shipments.scan`) scans the lost shipment `id` at `depot`: it is
/// `at_hub` again.
///
/// # Errors
///
/// `Forbidden` without `shipments.scan`; `NotFrom` unless the shipment is
/// `lost`; `Guard` (422) without a depot.
pub async fn found(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    depot: &str,
) -> std::result::Result<Shipment, TransitionError> {
    let shipment = load(state, id).await?;
    let _outcome = shipment
        .transition(FOUND)
        .by(user)
        .input(&ShipmentInput {
            depot: Some(depot.trim().to_owned()),
            ..ShipmentInput::default()
        })
        .run(state)
        .await?;
    load(state, id).await
}

/// `user` (`shipments.manage`) records that the claim of the lost shipment
/// `id` was settled with a credit of `amount` fils for the client (customer
/// service values it by hand against the client's terms).
///
/// # Errors
///
/// `Forbidden` without `shipments.manage`; `Guard` (422) unless the shipment
/// is `lost`, and for an amount that is not above 0.
pub async fn settle_claim(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    amount: i64,
) -> std::result::Result<Shipment, TransitionError> {
    if !user.has_permission(permissions::SHIPMENTS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let shipment = load(state, id).await?;
    if shipment.status != ShipmentStatus::Lost {
        return Err(guard(english("logistics.loss.not_lost", &[])));
    }
    if amount <= 0 {
        return Err(guard(english("logistics.loss.amount_needed", &[])));
    }
    let at: Option<DateTime> = Some(renox::db::now());
    Shipment::query()
        .where_(Shipment::ID.eq(id))
        .update(
            &state.db,
            &[("claim_settled_amount", &amount), ("claim_settled_at", &at)],
        )
        .await?;
    track(
        &state.db,
        id,
        "claim_settled",
        &aed(amount),
        Some(user.id),
    )
    .await?;
    if let Some(ticket) = shipment.claim_ticket_id {
        let text = english("logistics.loss.settled_comment", &[("amount", &aed(amount))]);
        tickets::task_creator(state)
            .comment(state, TicketId(ticket), TaskComment::internal(&text))
            .await?;
    }
    load(state, id).await
}

/// The checks of the client's choice for the held shipment `id`: staff acts,
/// the shipment waits for the choice, and the choice is known.
async fn choice_due(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    choice: &str,
) -> std::result::Result<Shipment, TransitionError> {
    if !user.has_permission(permissions::SHIPMENTS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let shipment = load(state, id).await?;
    let waiting = shipment.status == ShipmentStatus::OnHold
        && shipment.hold_reason.as_deref() == Some(hold_reason::CLIENT_INSTRUCTION)
        && shipment.claim_settled_amount > 0
        && shipment.claim_choice.is_none();
    if !waiting {
        return Err(guard(english("logistics.loss.no_choice_due", &[])));
    }
    if choice != CHOICE_DELIVERY && choice != CHOICE_KEEP {
        return Err(guard(english("logistics.loss.choice_unknown", &[])));
    }
    Ok(shipment)
}

/// Records the client's `choice` on `shipment` as the hold's instruction.
async fn record_choice(
    state: &AppState,
    user: &AuthUser,
    shipment: &Shipment,
    choice: &str,
) -> std::result::Result<(), TransitionError> {
    let id = shipment.id;
    let chosen = Some(choice.to_owned());
    Shipment::query()
        .where_(Shipment::ID.eq(id))
        .update(&state.db, &[("claim_choice", &chosen)])
        .await?;
    track(&state.db, id, "claim_choice", choice, Some(user.id)).await?;
    let note = english(
        &format!("logistics.loss.choice_note.{choice}"),
        &[("amount", &aed(shipment.claim_settled_amount))],
    );
    hold::record_instruction(state, user, id, &note).await?;
    Ok(())
}

/// `user` (`shipments.manage`, for the client) records the client's choice for
/// the shipment `id`, held after the goods were found following a settled
/// claim: `delivery` drafts the invoice that takes the claim credit back and
/// releases the shipment; `keep` sends the goods back to the client (a
/// disposal on the client's instruction is
/// [`choose_keep_and_dispose`]).
///
/// # Errors
///
/// `Forbidden` without `shipments.manage`; `Guard` (422) unless the shipment
/// is held for the client's instruction after a settled claim, or for an
/// unknown choice.
pub async fn choose(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    choice: &str,
) -> std::result::Result<Shipment, TransitionError> {
    let choice = choice.trim();
    let shipment = choice_due(state, user, id, choice).await?;
    record_choice(state, user, &shipment, choice).await?;
    if choice == CHOICE_DELIVERY {
        // The claim credit is reversed by an invoice (§4.4 step 4).
        let draft = InvoiceDraft::new(shipment.client(), shipment.subject()).line(
            DraftLine::new(
                &english("logistics.loss.claim_reversed", &[]),
                shipment.claim_settled_amount,
                VatCategory::OutOfScope,
            )
            .origin(shipment.subject()),
        );
        let invoice = wl_kernel::invoicing::invoicer(state)
            .draft(&state.db, draft)
            .await
            .map_err(TransitionError::Failed)?;
        match invoice {
            Some(invoice) => {
                let invoice = Some(invoice.0);
                Shipment::query()
                    .where_(Shipment::ID.eq(id))
                    .update(&state.db, &[("claim_invoice_id", &invoice)])
                    .await?;
            }
            None => {
                track(&state.db, id, "claim_invoice_skipped", "invoicing is off", None).await?;
            }
        }
        hold::release(state, user, id).await
    } else {
        hold::send_back(state, user, id).await
    }
}

/// `user` (`shipments.manage`) records the client's choice *keep the
/// settlement* for the shipment `id` and, as the client instructed, asks to
/// dispose of the goods instead of returning them (03 §4.4 step 4). The
/// disposal is the same request as [`crate::disposal::request`]: it waits for
/// an operations manager's approval and the shipment stays held until then.
/// The request is checked first, so a refused one records nothing.
///
/// # Errors
///
/// As [`choose`], and `Guard` (422) when the disposal lacks a reason, a photo,
/// the client's instruction (unless dangerous) or a liability.
pub async fn choose_keep_and_dispose(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    disposal: &DisposalInput,
) -> std::result::Result<DisposeRequest, TransitionError> {
    let shipment = choice_due(state, user, id, CHOICE_KEEP).await?;
    crate::disposal::check(Some(disposal)).map_err(guard)?;
    record_choice(state, user, &shipment, CHOICE_KEEP).await?;
    crate::disposal::request(state, user, id, disposal).await
}
