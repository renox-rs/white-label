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
//! ```
//!
//! Every state of §7.1 is declared with its tone, so the diagram is whole;
//! the transitions that come with later issues (`create_return`, `open_review`, `close_review`, `reverse_delivery`,
//! `mark_lost`, `found`, `dispose`) are added to [`transitions`] by them.
//! `dispatch`, `deliver` and `fail_attempt` are the system's: they run when
//! the shipment's delivery in wl-delivery does ([`crate::events`]).
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

use crate::permissions;
use crate::rates;
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
            .update(
                &mut *tx,
                &[("state", &scanned_state), ("scanned_at", &now)],
            )
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
    Transition::new(name)
        .label(match name {
            DISPATCH => "Out for delivery",
            DELIVER => "Delivered",
            _ => "Delivery attempt failed",
        })
        .system()
        .from([from])
        .to(to)
        .effect(move |shipment: &Shipment, ctx, tx| {
            let id = shipment.id;
            let note = input_of(ctx).reason.unwrap_or_default();
            let kind = match name {
                DISPATCH => "dispatched",
                DELIVER => "delivered",
                _ => "attempt_failed",
            };
            Box::pin(async move { track(&mut *tx, id, kind, &note, None).await })
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
        .guard("confirmation_or_instruction_recorded", |s: &Shipment, _ctx| {
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
        })
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
pub async fn start_return(
    tx: &mut Transaction,
    shipment: &Shipment,
    actor: Option<i64>,
) -> Result {
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
            let given = text_given(&input_of(ctx).pod_reference);
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
                Shipment::query()
                    .where_(Shipment::ID.eq(id))
                    .update(&mut *tx, &[("return_pod_ref", &reference)])
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
            voids::VOID,
            voids::VOID_APPROVED,
        ] {
            assert!(machine.find(name).is_some(), "{name}");
        }
        let cancel_return = machine.find(CANCEL_RETURN).unwrap();
        assert_eq!(
            cancel_return.from_states(),
            ["picked_up", "at_hub", "attempted", "on_hold"]
        );
        assert!(machine.find(DISPATCH).unwrap().is_system());
    }
}
