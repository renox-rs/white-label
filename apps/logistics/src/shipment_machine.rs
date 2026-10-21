//! The Shipment state machine (03 Logistics §7.1 *Shipment*).
//!
//! ```text
//! created --pick_up--> picked_up
//! created | picked_up | attempted | returning --receive_at_hub--> at_hub
//! at_hub --dispatch (system, the delivery left)--> out_for_delivery
//! out_for_delivery --deliver (system, DeliveryDelivered)--> delivered
//! out_for_delivery --fail_attempt (system, DeliveryAttemptFailed)--> attempted
//! created --cancel--> cancelled          created --void--> voided
//! picked_up | at_hub | attempted | on_hold --cancel_return--> returning
//! returning --return_delivered--> returned
//! created | at_hub | attempted --hold--> on_hold --release--> at_hub
//! attempted | on_hold --return--> returning
//! delivered --open_review (system, DeliveryDisputed)--> under_review
//! under_review --close_review (system, DeliveryDisputeClosed)--> delivered
//! delivered | under_review --reverse_delivery (system, DeliveryReversed)--> attempted
//! delivered | under_review --reverse_delivery_on_road (system)--> out_for_delivery
//! picked_up | at_hub | on_hold | returning --mark_lost (approved)--> lost
//! out_for_delivery | attempted | under_review | (the above) --mark_lost_system--> lost
//! lost --found (scan, or system on DeliveryFound)--> at_hub
//! on_hold --dispose (approved)--> disposed
//! ```
//!
//! Every state of §7.1 is declared with its tone, so the diagram is whole;
//! the transitions that come with later issues (`create_return`, `open_review`, `close_review`, `reverse_delivery`,
//! `mark_lost`, `found`, `dispose`) are added to [`transitions`] by them.
//! `dispatch`, `deliver` and `fail_attempt` are the system's: they run when
//! the shipment's delivery in wl-delivery does ([`crate::events`]); so do
//! `open_review` and `close_review` (no user runs them, `WRK.ACL.06`).
//!
//! Renox guards get no `AppState`, so the checks that need the database run
//! in the services ([`crate::scan`], [`crate::cancel`]) and the guards read
//! what they worked out in a [`ShipmentInput`], or what the row itself says
//! (the parcels scanned, the instruction recorded, who booked it).

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::voids;

use crate::disposal::{self, DisposalInput};
use crate::loss::{self, LossInput};
use crate::permissions;
use crate::rates;
use crate::reversal::ReversalInput;
use crate::shipment::{
    Parcel, SUBJECT_TYPE, Shipment, ShipmentStatus, charge_kind, hold_reason, parcel_state, track,
};
use crate::texts::english;

/// `pick_up`: created → picked_up.
pub const PICK_UP: &str = "pick_up";
/// `receive_at_hub`: created, picked_up, attempted, returning → at_hub.
pub const RECEIVE_AT_HUB: &str = "receive_at_hub";
/// `dispatch`: at_hub → out_for_delivery, by the system with the delivery.
pub const DISPATCH: &str = "dispatch";
/// `deliver`: out_for_delivery → delivered, by the system with the delivery.
pub const DELIVER: &str = "deliver";
/// `fail_attempt`: out_for_delivery → attempted, by the system with the
/// delivery's failed attempt.
pub const FAIL_ATTEMPT: &str = "fail_attempt";
/// `cancel`: created → cancelled.
pub const CANCEL: &str = "cancel";
/// `cancel_return`: picked_up, at_hub, attempted, on_hold → returning.
pub const CANCEL_RETURN: &str = "cancel_return";
/// `return_delivered`: returning → returned.
pub const RETURN_DELIVERED: &str = "return_delivered";
/// `hold`: created, at_hub, attempted → on_hold, for an exception.
pub const HOLD: &str = "hold";
/// `hold_system`: `hold` run by the system, after a failed attempt with the
/// reason *wrong address* (a transition is either a user's or the system's).
pub const HOLD_SYSTEM: &str = "hold_system";
/// `release`: on_hold → at_hub, once the confirmation or the instruction is
/// recorded.
pub const RELEASE: &str = "release";
/// `return`: attempted, on_hold → returning.
pub const RETURN: &str = "return";
/// `open_review`: delivered → under_review, by the system with the
/// delivery's dispute (`DeliveryDisputed`).
pub const OPEN_REVIEW: &str = "open_review";
/// `close_review`: under_review → delivered, by the system when the review
/// is upheld (`DeliveryDisputeClosed`).
pub const CLOSE_REVIEW: &str = "close_review";
/// `reverse_delivery`: delivered, under_review → attempted, by the system
/// when wl-delivery reverses the delivery and leaves it `attempted`
/// (BL-LOG-10).
pub const REVERSE_DELIVERY: &str = "reverse_delivery";
/// `reverse_delivery`, run by the system when wl-delivery put the delivery
/// back on the road: delivered, under_review → out_for_delivery.
pub const REVERSE_DELIVERY_ON_ROAD: &str = "reverse_delivery_on_road";
/// `mark_lost`: picked_up, at_hub, on_hold, returning → lost, a user's
/// request that the kernel defers into an approval (`LOG.ACL.14`).
pub const MARK_LOST: &str = "mark_lost";
/// `mark_lost`, run by the kernel once the request is approved.
pub const MARK_LOST_APPROVED: &str = "mark_lost_approved";
/// `mark_lost`, run by the system when the delivery is lost (`DeliveryLost`,
/// `WRK.RUL.17`).
pub const MARK_LOST_SYSTEM: &str = "mark_lost_system";
/// `found`: lost → at_hub, scanned at a depot.
pub const FOUND: &str = "found";
/// `found`, run by the system on `DeliveryFound`.
pub const FOUND_SYSTEM: &str = "found_system";
/// `dispose`: on_hold → disposed, a user's request that the kernel defers
/// into an approval (`LOG.ACL.14`, BL-LOG-06).
pub const DISPOSE: &str = "dispose";
/// `dispose`, run by the kernel once the request is approved.
pub const DISPOSE_APPROVED: &str = "dispose_approved";
/// `void`: created → voided (the kernel's void pattern).
pub const VOID: &str = voids::VOID;

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipmentInput {
    /// The barcodes scanned (`pick_up`, `receive_at_hub`).
    #[serde(default)]
    pub barcodes: Vec<String>,
    /// The barcodes the driver marked *not received* (`pick_up`, and
    /// `receive_at_hub` from `created`).
    #[serde(default)]
    pub not_received: Vec<String>,
    /// Why it is held, one of [`crate::shipment::hold_reason`] (`hold`).
    #[serde(default)]
    pub hold_reason: Option<String>,
    /// The references of the photos of a damaged parcel (`hold`).
    #[serde(default)]
    pub photos: Vec<String>,
    /// Whether the company is liable for the damage (`hold`).
    #[serde(default)]
    pub company_liable: bool,
    /// The failure reason's code (`fail_attempt`).
    #[serde(default)]
    pub reason: Option<String>,
    /// The client's instruction to send it back: who asked, when and how
    /// (`cancel_return`).
    #[serde(default)]
    pub instruction: Option<String>,
    /// The reference of the client's e-POD (`return_delivered`).
    #[serde(default)]
    pub pod_reference: Option<String>,
    /// Whether the user acts for the shipment's client company, as the
    /// portal says (`cancel` by a client user).
    #[serde(default)]
    pub client_acts: bool,
    /// The COD difference recorded at the stop, in fils (`deliver`).
    #[serde(default)]
    pub cod_difference: i64,
    /// The reason of that difference (`deliver`).
    #[serde(default)]
    pub cod_difference_reason: Option<String>,
    /// Who absorbs the difference (`deliver`).
    #[serde(default)]
    pub absorbed_by: Option<String>,
    /// The cash collected at the stop, in fils (`deliver`).
    #[serde(default)]
    pub cod_collected: i64,
    /// The parcels the recipient refused (`deliver`, BL-LOG-07).
    #[serde(default)]
    pub refused: Vec<RefusedParcel>,
    /// The return shipment to create for them (`deliver`, BL-LOG-07).
    #[serde(default)]
    pub return_plan: Option<ReturnPlan>,
    /// The dispute ticket linked to the review (`open_review`).
    #[serde(default)]
    pub review_ticket: Option<i64>,
    /// Whether ticketing is off, so the review has no ticket to link
    /// (`open_review`).
    #[serde(default)]
    pub ticketing_off: bool,
    /// What the delivery's reversal says (`reverse_delivery`).
    #[serde(default)]
    pub reversal: Option<ReversalInput>,
    /// The reason, liability and claim ticket of a loss (`mark_lost`).
    #[serde(default)]
    pub loss: Option<LossInput>,
    /// The depot where the goods were scanned (`found`).
    #[serde(default)]
    pub depot: Option<String>,
    /// The reason, photos, instruction and liability of a disposal
    /// (`dispose`).
    #[serde(default)]
    pub disposal: Option<DisposalInput>,
}

/// A parcel the recipient refused, with the refusal reason's code.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusedParcel {
    /// The parcel.
    pub parcel_id: i64,
    /// The code of the reason; a refused parcel has one.
    pub reason: String,
}

/// What the return shipment of the refused parcels is made of, worked out
/// before the transition (its number is taken from the series).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnPlan {
    /// The number of the return shipment.
    pub number: String,
    /// Who it goes to: the client.
    pub recipient_name: String,
    /// The client's pickup address.
    pub address: String,
    /// Its area.
    pub area: String,
}

/// The input of a run; the default without one (`available`).
pub(crate) fn input_of(ctx: &TransitionCtx) -> ShipmentInput {
    ctx.input_as::<ShipmentInput>().unwrap_or_default()
}

fn text_given(text: &Option<String>) -> bool {
    text.as_deref().is_some_and(|text| !text.trim().is_empty())
}

/// Marks the barcodes scanned, counts the parcels scanned and writes the
/// tracking event of `kind`.
async fn scan_effects(
    tx: &mut Transaction,
    shipment: &Shipment,
    input: &ShipmentInput,
    kind: &str,
    actor: Option<i64>,
) -> Result {
    let now = Some(renox::db::now());
    let scanned_state = parcel_state::SCANNED.to_owned();
    for code in &input.barcodes {
        Parcel::query()
            .where_(Parcel::SHIPMENT_ID.eq(shipment.id))
            .where_(Parcel::BARCODE.eq(code.as_str()))
            .update(&mut *tx, &[("state", &scanned_state), ("scanned_at", &now)])
            .await?;
    }
    let scanned = Parcel::query()
        .where_(Parcel::SHIPMENT_ID.eq(shipment.id))
        .where_(Parcel::STATE.eq(parcel_state::SCANNED))
        .count(&mut *tx)
        .await?;
    let scanned = i64::try_from(scanned).unwrap_or(i64::MAX);
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(&mut *tx, &[("parcels_scanned", &scanned)])
        .await?;
    let mut note = input.barcodes.join(", ");
    if shipment.status == ShipmentStatus::Created {
        // Pickup or drop-off: what was not handed over is settled now.
        crate::pickup::settle(&mut *tx, shipment, input).await?;
        if !input.not_received.is_empty() {
            note = format!("{note}; not received: {}", input.not_received.join(", "));
        }
    }
    track(&mut *tx, shipment.id, kind, &note, actor).await
}

/// `pick_up`: at least one parcel scanned; the parcels not handed over are
/// marked *not received* and the weight and price are recalculated from the
/// parcels received (BL-LOG-03). It is never refused for missing parcels.
fn pick_up() -> Transition<Shipment> {
    Transition::new(PICK_UP)
        .label("Pick up")
        .tone("primary")
        .permission(permissions::SHIPMENTS_SCAN)
        .from([ShipmentStatus::Created])
        .to(ShipmentStatus::PickedUp)
        .guard("a_parcel_scanned", |_s: &Shipment, ctx| {
            let scanned = !input_of(ctx).barcodes.is_empty();
            Box::pin(async move {
                if scanned {
                    Ok(())
                } else {
                    Err(english("logistics.scan.parcel_needed", &[]))
                }
            })
        })
        .effect(|shipment: &Shipment, ctx, tx| {
            let shipment = shipment.clone();
            let input = input_of(ctx);
            let actor = ctx.actor().map(|user| user.id);
            Box::pin(
                async move { scan_effects(&mut *tx, &shipment, &input, "picked_up", actor).await },
            )
        })
}

/// `receive_at_hub` (`LOG.HUB.01`): from `created` it is a drop-off and needs
/// a parcel scanned, as `pick_up` does, and marks the parcels not handed over
/// *not received* the same way.
fn receive_at_hub() -> Transition<Shipment> {
    Transition::new(RECEIVE_AT_HUB)
        .label("Receive at the hub")
        .tone("primary")
        .permission(permissions::SHIPMENTS_SCAN)
        .from([
            ShipmentStatus::Created,
            ShipmentStatus::PickedUp,
            ShipmentStatus::Attempted,
            ShipmentStatus::Returning,
        ])
        .to(ShipmentStatus::AtHub)
        .guard("a_parcel_scanned_on_drop_off", |s: &Shipment, ctx| {
            let missing = s.status == ShipmentStatus::Created && input_of(ctx).barcodes.is_empty();
            Box::pin(async move {
                if missing {
                    Err(english("logistics.scan.parcel_needed", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|shipment: &Shipment, ctx, tx| {
            let shipment = shipment.clone();
            let input = input_of(ctx);
            let actor = ctx.actor().map(|user| user.id);
            Box::pin(
                async move { scan_effects(&mut *tx, &shipment, &input, "hub_scan", actor).await },
            )
        })
}

/// A system transition that only writes the tracking event `kind`.
fn followed(name: &'static str, from: ShipmentStatus, to: ShipmentStatus) -> Transition<Shipment> {
    let transition = Transition::new(name)
        .label(match name {
            DISPATCH => "Out for delivery",
            DELIVER => "Delivered",
            _ => "Delivery attempt failed",
        })
        .system()
        .from([from])
        .to(to);
    let transition = if name == DELIVER {
        // §7.1 `deliver`: only a shipment of several parcels may have refused
        // parcels (`WRK.POD.01`), and each has a reason.
        transition.guard(
            "refused_parcels_allowed_and_explained",
            |s: &Shipment, ctx| {
                let input = input_of(ctx);
                let single = s.parcel_count <= 1 && !input.refused.is_empty();
                let unexplained = input
                    .refused
                    .iter()
                    .any(|refused| refused.reason.trim().is_empty());
                Box::pin(async move {
                    if single {
                        Err(english("logistics.partial.single_parcel", &[]))
                    } else if unexplained {
                        Err(english("logistics.partial.reason_needed", &[]))
                    } else {
                        Ok(())
                    }
                })
            },
        )
    } else {
        transition
    };
    transition.effect(move |shipment: &Shipment, ctx, tx| {
        let original = shipment.clone();
        let id = shipment.id;
        let input = input_of(ctx);
        let note = input.reason.clone().unwrap_or_default();
        let kind = match name {
            DISPATCH => "dispatched",
            DELIVER => "delivered",
            _ => "attempt_failed",
        };
        Box::pin(async move {
            if name == DELIVER {
                // What the COD remittance reads (BL-LOG-14): when it was
                // delivered and the difference with who absorbs it.
                let at = Some(renox::db::now());
                Shipment::query()
                    .where_(Shipment::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("delivered_at", &at),
                            ("cod_collected", &input.cod_collected),
                            ("cod_difference", &input.cod_difference),
                            ("cod_difference_reason", &input.cod_difference_reason),
                            ("cod_absorbed_by", &input.absorbed_by),
                            ("reversed", &false),
                        ],
                    )
                    .await?;
                // A difference the business absorbs is a COD loss
                // (BL-LOG-08).
                crate::cod::record_loss(&mut *tx, &original, &input).await?;
                if !input.refused.is_empty() {
                    // `create_return` (BL-LOG-07): the refused parcels go back
                    // on a linked return shipment.
                    crate::partial::create_return(&mut *tx, &original, &input, None).await?;
                }
            }
            track(&mut *tx, id, kind, &note, None).await
        })
    })
}

/// `cancel` (`LOG.ACL.09`): a client user of the company or staff with
/// `shipments.manage`, while the shipment is `created`; a cancellation fee
/// line only when the rate card has one.
fn cancel() -> Transition<Shipment> {
    Transition::new(CANCEL)
        .label("Cancel")
        .tone("danger")
        .permission(permissions::SHIPMENTS_CANCEL)
        .from([ShipmentStatus::Created])
        .to(ShipmentStatus::Cancelled)
        .guard("not_picked_up", |s: &Shipment, _ctx| {
            let scanned = s.parcels_scanned > 0;
            Box::pin(async move {
                if scanned {
                    Err(english("logistics.cancel.picked_up", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .guard("client_or_manager", |s: &Shipment, ctx| {
            let allowed = match ctx.actor() {
                None => true,
                Some(user) => {
                    user.has_permission(permissions::SHIPMENTS_MANAGE)
                        || input_of(ctx).client_acts
                        || s.booked_by_user_id == Some(user.id)
                }
            };
            Box::pin(async move {
                if allowed {
                    Ok(())
                } else {
                    Err(english("logistics.cancel.not_your_shipment", &[]))
                }
            })
        })
        .effect(|shipment: &Shipment, ctx, tx| {
            let shipment = shipment.clone();
            let actor = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                if let Some(fee) =
                    rates::price(&mut *tx, shipment.client(), charge_kind::CANCELLATION).await?
                {
                    rates::record_charge(&mut *tx, &shipment, charge_kind::CANCELLATION, fee, "")
                        .await?;
                }
                track(&mut *tx, shipment.id, "cancelled", "", actor).await
            })
        })
}

/// `cancel_return` (`LOG.ACL.09`): after pickup only `shipments.manage` sends
/// the shipment back, on the client's recorded instruction; one return charge
/// per rate card, at AED 0 with a note for the accountant when the card has
/// no return line. It is not offered from `out_for_delivery` (the driver
/// first fails the stop with *cancelled by client*) or from `returning`.
fn cancel_return() -> Transition<Shipment> {
    Transition::new(CANCEL_RETURN)
        .label("Cancel and return")
        .tone("danger")
        .permission(permissions::SHIPMENTS_MANAGE)
        .from([
            ShipmentStatus::PickedUp,
            ShipmentStatus::AtHub,
            ShipmentStatus::Attempted,
            ShipmentStatus::OnHold,
        ])
        .to(ShipmentStatus::Returning)
        .guard("instruction_recorded", |s: &Shipment, ctx| {
            let recorded = s.has_return_instruction() || text_given(&input_of(ctx).instruction);
            Box::pin(async move {
                if recorded {
                    Ok(())
                } else {
                    Err(english("logistics.cancel.instruction_needed", &[]))
                }
            })
        })
        .effect(|shipment: &Shipment, ctx, tx| {
            let shipment = shipment.clone();
            let instruction = input_of(ctx).instruction;
            let actor = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                if let Some(text) = instruction.filter(|text| !text.trim().is_empty()) {
                    crate::cancel::store_instruction(&mut *tx, shipment.id, &text, actor).await?;
                }
                start_return(&mut *tx, &shipment, actor).await
            })
        })
}

/// `hold` (§7.1; `hold_system` is the same run by the system): hub staff from a scan, or staff with `shipments.manage`, or
/// the system after a failed attempt with *wrong address*. It needs one of
/// the exception reasons, and photos when the parcel is damaged. The state it
/// leaves is kept ([`Shipment::held_from`]); the instruction and the address
/// confirmation of an earlier hold are forgotten.
fn hold(name: &'static str, by_system: bool) -> Transition<Shipment> {
    let transition = Transition::new(name)
        .label("Hold")
        .tone("warning")
        .from([
            ShipmentStatus::Created,
            ShipmentStatus::AtHub,
            ShipmentStatus::Attempted,
        ])
        .to(ShipmentStatus::OnHold)
        .guard("manager_scanner_or_system", |_s: &Shipment, ctx| {
            let allowed = ctx.actor().is_none_or(|user| {
                user.has_permission(permissions::SHIPMENTS_MANAGE)
                    || user.has_permission(permissions::SHIPMENTS_SCAN)
            });
            Box::pin(async move {
                if allowed {
                    Ok(())
                } else {
                    Err(english("logistics.forbidden", &[]))
                }
            })
        })
        .guard("exception_reason", |_s: &Shipment, ctx| {
            let known = input_of(ctx)
                .hold_reason
                .is_some_and(|reason| hold_reason::ALL.contains(&reason.as_str()));
            Box::pin(async move {
                if known {
                    Ok(())
                } else {
                    Err(english("logistics.hold.reason_needed", &[]))
                }
            })
        })
        .guard("photos_when_damaged", |_s: &Shipment, ctx| {
            let input = input_of(ctx);
            let missing = input.hold_reason.as_deref() == Some(hold_reason::DAMAGED)
                && input.photos.iter().all(|photo| photo.trim().is_empty());
            Box::pin(async move {
                if missing {
                    Err(english("logistics.hold.photos_needed", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|shipment: &Shipment, ctx, tx| {
            let shipment = shipment.clone();
            let input = input_of(ctx);
            let actor = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let reason = input.hold_reason.clone().unwrap_or_default();
                let reason_value = Some(reason.clone());
                let from = Some(shipment.status.as_str().to_owned());
                let photos = Some(
                    input
                        .photos
                        .iter()
                        .map(|photo| photo.as_str().trim())
                        .filter(|photo| !photo.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
                let no_text: Option<String> = None;
                let no_user: Option<i64> = None;
                let no_time: Option<renox::db::DateTime> = None;
                Shipment::query()
                    .where_(Shipment::ID.eq(shipment.id))
                    .update(
                        &mut *tx,
                        &[
                            ("hold_reason", &reason_value),
                            ("held_from", &from),
                            ("hold_photos", &photos),
                            ("company_liable", &input.company_liable),
                            ("hold_instruction", &no_text),
                            ("hold_instruction_by", &no_user),
                            ("hold_instruction_at", &no_time),
                            ("address_confirmed_at", &no_time),
                            ("address_confirmed_by", &no_user),
                        ],
                    )
                    .await?;
                track(&mut *tx, shipment.id, "held", &reason, actor).await
            })
        });
    if by_system {
        transition.system()
    } else {
        transition
    }
}

/// `release` (§7.1): `shipments.manage`, back to `at_hub`, once the address
/// of an *address issue* hold is confirmed, or the client's instruction is
/// recorded for any other reason. (A shipment held from `created` or
/// `attempted` also returns to `at_hub`, as the spec says; the state it was
/// held from stays in [`Shipment::held_from`].)
fn release() -> Transition<Shipment> {
    Transition::new(RELEASE)
        .label("Release")
        .tone("primary")
        .permission(permissions::SHIPMENTS_MANAGE)
        .from([ShipmentStatus::OnHold])
        .to(ShipmentStatus::AtHub)
        .guard(
            "confirmation_or_instruction_recorded",
            |s: &Shipment, _ctx| {
                let recorded = if s.hold_reason.as_deref() == Some(hold_reason::ADDRESS_ISSUE) {
                    s.address_confirmed_at.is_some()
                } else {
                    s.has_hold_instruction()
                };
                let address_issue = s.hold_reason.as_deref() == Some(hold_reason::ADDRESS_ISSUE);
                Box::pin(async move {
                    if recorded {
                        Ok(())
                    } else if address_issue {
                        Err(english("logistics.hold.address_needed", &[]))
                    } else {
                        Err(english("logistics.hold.instruction_needed", &[]))
                    }
                })
            },
        )
        .effect(|shipment: &Shipment, ctx, tx| {
            let id = shipment.id;
            let actor = ctx.actor().map(|user| user.id);
            let note = shipment.hold_reason.clone().unwrap_or_default();
            Box::pin(async move { track(&mut *tx, id, "released", &note, actor).await })
        })
}

/// `return` (§7.1): `shipments.manage` sends an attempted or held shipment
/// back; one return charge per rate card, whichever path sent it back. The
/// client is told and the open delivery is cancelled after the commit.
fn send_back() -> Transition<Shipment> {
    Transition::new(RETURN)
        .label("Return")
        .tone("danger")
        .permission(permissions::SHIPMENTS_MANAGE)
        .from([ShipmentStatus::Attempted, ShipmentStatus::OnHold])
        .to(ShipmentStatus::Returning)
        .effect(|shipment: &Shipment, ctx, tx| {
            let shipment = shipment.clone();
            let actor = ctx.actor().map(|user| user.id);
            Box::pin(async move { start_return(&mut *tx, &shipment, actor).await })
        })
}

/// What starts a return, whichever path sent the shipment back: the return
/// charge, once, and the tracking event.
///
/// # Errors
///
/// Database errors.
pub async fn start_return(tx: &mut Transaction, shipment: &Shipment, actor: Option<i64>) -> Result {
    match rates::price(&mut *tx, shipment.client(), charge_kind::RETURN).await? {
        Some(amount) => {
            rates::record_charge(&mut *tx, shipment, charge_kind::RETURN, amount, "").await?;
        }
        None => {
            let note = english(
                "logistics.cancel.no_return_price",
                &[("client", &shipment.client_party_id)],
            );
            rates::record_charge(&mut *tx, shipment, charge_kind::RETURN, 0, &note).await?;
        }
    }
    track(&mut *tx, shipment.id, "return_started", "", actor).await
}

/// `return_delivered`: the client's e-POD for the returned goods.
fn return_delivered() -> Transition<Shipment> {
    Transition::new(RETURN_DELIVERED)
        .label("Returned to the client")
        .tone("primary")
        .permission(permissions::SHIPMENTS_DELIVER)
        .from([ShipmentStatus::Returning])
        .to(ShipmentStatus::Returned)
        .guard("e_pod_from_the_client", |_s: &Shipment, ctx| {
            // `available` asks without any input: the driver is offered the
            // action; the e-POD is then checked again by the effect, so a
            // run without one never gets through.
            let given = ctx.input().is_null() || text_given(&input_of(ctx).pod_reference);
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("logistics.return.pod_needed", &[]))
                }
            })
        })
        .effect(|shipment: &Shipment, ctx, tx| {
            let id = shipment.id;
            let reference = input_of(ctx).pod_reference.map(|r| r.trim().to_owned());
            let actor = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                if reference.as_deref().is_none_or(str::is_empty) {
                    return Err(Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        english("logistics.return.pod_needed", &[]),
                    ));
                }
                // The month of this e-POD is the month its return charge is
                // billed in (`LOG.RUL.07`, BL-LOG-20).
                let at = Some(renox::db::now());
                Shipment::query()
                    .where_(Shipment::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[("return_pod_ref", &reference), ("returned_at", &at)],
                    )
                    .await?;
                track(
                    &mut *tx,
                    id,
                    "returned",
                    reference.as_deref().unwrap_or_default(),
                    actor,
                )
                .await
            })
        })
}

/// `open_review` (`WRK.ACL.06`): the system, when wl-delivery puts the
/// shipment's delivery under review. The dispute ticket is linked (or
/// ticketing is off, so there is none). The ticket is kept on the shipment;
/// the COD is left out of the remittances and the client is told after the
/// commit ([`crate::review`]).
fn open_review() -> Transition<Shipment> {
    Transition::new(OPEN_REVIEW)
        .label("Under review")
        .system()
        .from([ShipmentStatus::Delivered])
        .to(ShipmentStatus::UnderReview)
        .guard("dispute_ticket_linked", |_s: &Shipment, ctx| {
            let input = input_of(ctx);
            let linked = input.review_ticket.is_some() || input.ticketing_off;
            Box::pin(async move {
                if linked {
                    Ok(())
                } else {
                    Err(english("logistics.review.ticket_needed", &[]))
                }
            })
        })
        .effect(|shipment: &Shipment, ctx, tx| {
            let id = shipment.id;
            let ticket = input_of(ctx).review_ticket;
            Box::pin(async move {
                Shipment::query()
                    .where_(Shipment::ID.eq(id))
                    .update(&mut *tx, &[("review_ticket_id", &ticket)])
                    .await?;
                let note = ticket.map(|t| format!("ticket {t}")).unwrap_or_default();
                track(&mut *tx, id, "review_opened", &note, None).await
            })
        })
}

/// `close_review`: the system, when the review is upheld; the COD and the
/// billing are released (the shipment is `delivered` again, so the next
/// draft remittance and the next invoice take it).
fn close_review() -> Transition<Shipment> {
    Transition::new(CLOSE_REVIEW)
        .label("Review upheld")
        .system()
        .from([ShipmentStatus::UnderReview])
        .to(ShipmentStatus::Delivered)
        .effect(|shipment: &Shipment, _ctx, tx| {
            let id = shipment.id;
            Box::pin(async move { track(&mut *tx, id, "review_closed", "upheld", None).await })
        })
}

/// `reverse_delivery` (`WRK.ACL.10`): the system, when wl-delivery reverses
/// the delivery with the operations manager's approval; `name` says where the
/// delivery went (`attempted`, or back out on the road). The original e-POD is
/// wl-delivery's and stays; the COD, the billing, the liability and the
/// client's notice follow after the commit ([`crate::reversal`]). No user runs
/// it.
fn reverse_delivery(name: &'static str, to: ShipmentStatus) -> Transition<Shipment> {
    Transition::new(name)
        .label("Delivery reversed")
        .system()
        .from([ShipmentStatus::Delivered, ShipmentStatus::UnderReview])
        .to(to)
        .guard("reversal_given", |_s: &Shipment, ctx| {
            let given = input_of(ctx).reversal.is_some();
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("logistics.reversal.event_needed", &[]))
                }
            })
        })
        .effect(|shipment: &Shipment, ctx, tx| {
            let shipment = shipment.clone();
            let reversal = input_of(ctx).reversal;
            Box::pin(async move {
                match reversal {
                    Some(reversal) => crate::reversal::record(&mut *tx, &shipment, &reversal).await,
                    None => Ok(()),
                }
            })
        })
}

/// `mark_lost` (§7.1, `LOG.ACL.14`): `name` is the user's request, the run the
/// kernel makes once it is approved, or the system's on `DeliveryLost`. The
/// two first need a reason and a liability and run off a delivery; the
/// system's also runs from the states on one. The effects are shared
/// ([`crate::loss`]).
fn mark_lost(name: &'static str) -> Transition<Shipment> {
    let system = name == MARK_LOST_SYSTEM;
    let transition = Transition::new(name)
        .label(if system {
            "Lost (untraced)"
        } else {
            "Mark lost"
        })
        .tone("danger")
        .from(if system {
            loss::LOSABLE.to_vec()
        } else {
            loss::OFF_DELIVERY.to_vec()
        })
        .to(ShipmentStatus::Lost);
    let transition = match name {
        MARK_LOST => transition.permission(permissions::SHIPMENTS_MANAGE),
        _ => transition.system(),
    };
    let transition = if system {
        transition
    } else {
        transition.guard("reason_and_liability", |_s: &Shipment, ctx| {
            let checked = loss::reason_and_liability(input_of(ctx).loss.as_ref());
            Box::pin(async move { checked })
        })
    };
    transition.effect(|shipment: &Shipment, ctx, tx| {
        let shipment = shipment.clone();
        let input = input_of(ctx).loss.unwrap_or_default();
        let actor = ctx.actor().map(|user| user.id);
        Box::pin(async move { loss::record(&mut *tx, &shipment, &input, actor).await })
    })
}

/// `found` (§7.1): scanned at a depot by `shipments.scan`, or by the system on
/// `DeliveryFound`. The shipment is `at_hub` again and expects its COD.
fn found(name: &'static str) -> Transition<Shipment> {
    let transition = Transition::new(name)
        .label("Found")
        .tone("success")
        .from([ShipmentStatus::Lost])
        .to(ShipmentStatus::AtHub)
        .guard("scanned_at_a_depot", |_s: &Shipment, ctx| {
            let given = text_given(&input_of(ctx).depot);
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("logistics.loss.depot_needed", &[]))
                }
            })
        });
    let transition = if name == FOUND {
        transition.permission(permissions::SHIPMENTS_SCAN)
    } else {
        transition.system()
    };
    transition.effect(|shipment: &Shipment, ctx, tx| {
        let shipment = shipment.clone();
        let depot = input_of(ctx).depot.unwrap_or_default();
        let actor = ctx.actor().map(|user| user.id);
        Box::pin(async move { loss::record_found(&mut *tx, &shipment, &depot, actor).await })
    })
}

/// `dispose` (§7.1, `LOG.ACL.14`): `name` is the user's request, or the run
/// the kernel makes once it is approved. Both need a reason, a photo, the
/// client's written instruction (unless the goods are dangerous) and a
/// liability, and run only from `on_hold`. The effects are shared
/// ([`crate::disposal`]).
fn dispose(name: &'static str) -> Transition<Shipment> {
    let transition = Transition::new(name)
        .label("Dispose")
        .tone("danger")
        .from([ShipmentStatus::OnHold])
        .to(ShipmentStatus::Disposed)
        .guard(
            "reason_photo_instruction_and_liability",
            |_s: &Shipment, ctx| {
                let checked = disposal::check(input_of(ctx).disposal.as_ref());
                Box::pin(async move { checked })
            },
        );
    let transition = if name == DISPOSE {
        transition.permission(permissions::SHIPMENTS_MANAGE)
    } else {
        transition.system()
    };
    transition.effect(|shipment: &Shipment, ctx, tx| {
        let shipment = shipment.clone();
        let input = input_of(ctx).disposal.unwrap_or_default();
        let actor = ctx.actor().map(|user| user.id);
        Box::pin(async move { disposal::record(&mut *tx, &shipment, &input, actor).await })
    })
}

/// What `void` adds to the kernel's guards (BL-LOG-01): no parcel scanned
/// (`PLT.ACL.09`: a safety check for a scan recorded without a state
/// change), and a client user voids only the shipment they booked
/// themselves; staff with `shipments.manage` void any `created` one.
fn void_rules(transition: Transition<Shipment>) -> Transition<Shipment> {
    transition
        .guard("no_parcel_scanned", |s: &Shipment, _ctx| {
            let scanned = s.parcels_scanned > 0;
            Box::pin(async move {
                if scanned {
                    Err(english("logistics.void.scanned", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .guard("booked_by_the_user_or_manager", |s: &Shipment, ctx| {
            let allowed = ctx.actor().is_none_or(|user| {
                user.has_permission(permissions::SHIPMENTS_MANAGE)
                    || s.booked_by_user_id == Some(user.id)
            });
            Box::pin(async move {
                if allowed {
                    Ok(())
                } else {
                    Err(english("logistics.void.not_your_shipment", &[]))
                }
            })
        })
}

/// Every transition of the machine.
pub fn transitions() -> Vec<Transition<Shipment>> {
    use ShipmentStatus::{AtHub, Attempted, Created, Delivered, OutForDelivery};
    let mut all = vec![
        pick_up(),
        receive_at_hub(),
        followed(DISPATCH, AtHub, OutForDelivery),
        followed(DELIVER, OutForDelivery, Delivered),
        followed(FAIL_ATTEMPT, OutForDelivery, Attempted),
        cancel(),
        cancel_return(),
        return_delivered(),
        hold(HOLD, false),
        hold(HOLD_SYSTEM, true),
        release(),
        send_back(),
        open_review(),
        close_review(),
        reverse_delivery(REVERSE_DELIVERY, Attempted),
        reverse_delivery(REVERSE_DELIVERY_ON_ROAD, OutForDelivery),
        mark_lost(MARK_LOST),
        mark_lost(MARK_LOST_APPROVED),
        mark_lost(MARK_LOST_SYSTEM),
        found(FOUND),
        found(FOUND_SYSTEM),
        dispose(DISPOSE),
        dispose(DISPOSE_APPROVED),
    ];
    all.extend(
        voids::transitions::<Shipment, _>(SUBJECT_TYPE, [Created])
            .into_iter()
            .map(void_rules),
    );
    all
}

impl StateMachine for Shipment {
    type State = ShipmentStatus;
    const COLUMN: renox::db::Col<Shipment, ShipmentStatus> = Shipment::STATUS;

    fn machine() -> Machine<Self> {
        use ShipmentStatus::{
            AtHub, Attempted, Cancelled, Created, Delivered, Disposed, Lost, OnHold,
            OutForDelivery, PickedUp, Returned, Returning, UnderReview, Voided,
        };
        let machine = Machine::new()
            .initial([Created])
            .state(Created)
            .tone("neutral")
            .state(PickedUp)
            .tone("info")
            .state(AtHub)
            .tone("info")
            .state(OutForDelivery)
            .tone("info")
            .state(Attempted)
            .tone("warning")
            .state(OnHold)
            .tone("warning")
            .state(Returning)
            .tone("warning")
            .state(Delivered)
            .tone("success")
            .state(UnderReview)
            .tone("warning")
            .state(Lost)
            .tone("error")
            .state(Returned)
            .tone("neutral")
            .terminal()
            .state(Disposed)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .state(Voided)
            .tone("neutral")
            .terminal();
        transitions().into_iter().fold(machine, Machine::transition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_declares_every_state_and_the_transitions_of_the_base() {
        let machine = Shipment::machine();
        assert_eq!(machine.states().len(), 14);
        for name in [
            PICK_UP,
            RECEIVE_AT_HUB,
            DISPATCH,
            DELIVER,
            FAIL_ATTEMPT,
            CANCEL,
            CANCEL_RETURN,
            RETURN_DELIVERED,
            HOLD,
            HOLD_SYSTEM,
            RELEASE,
            RETURN,
            OPEN_REVIEW,
            CLOSE_REVIEW,
            REVERSE_DELIVERY,
            REVERSE_DELIVERY_ON_ROAD,
            MARK_LOST,
            MARK_LOST_APPROVED,
            MARK_LOST_SYSTEM,
            FOUND,
            FOUND_SYSTEM,
            DISPOSE,
            DISPOSE_APPROVED,
            voids::VOID,
            voids::VOID_APPROVED,
        ] {
            assert!(machine.find(name).is_ok(), "{name}");
        }
        let cancel_return = machine.find(CANCEL_RETURN).unwrap();
        assert_eq!(
            cancel_return.from_states(),
            ["picked_up", "at_hub", "attempted", "on_hold"]
        );
        assert!(machine.find(DISPATCH).unwrap().is_system());
        assert!(machine.find(OPEN_REVIEW).unwrap().is_system());
        assert!(machine.find(CLOSE_REVIEW).unwrap().is_system());
        assert!(machine.find(MARK_LOST_APPROVED).unwrap().is_system());
        assert!(machine.find(MARK_LOST_SYSTEM).unwrap().is_system());
        assert!(!machine.find(MARK_LOST).unwrap().is_system());
        assert!(machine.find(FOUND_SYSTEM).unwrap().is_system());
        assert!(machine.find(DISPOSE_APPROVED).unwrap().is_system());
        assert!(!machine.find(DISPOSE).unwrap().is_system());
        assert!(machine.find(REVERSE_DELIVERY).unwrap().is_system());
        assert!(machine.find(REVERSE_DELIVERY_ON_ROAD).unwrap().is_system());
    }
}
