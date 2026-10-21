//! The Freight job state machine (03 Logistics §7.1 *Freight job*).
//!
//! ```text
//! booked --request_documents--> documents_pending
//! booked | documents_pending --depart--> in_transit --arrive--> customs_clearance
//! customs_clearance --clear--> ready_for_delivery --deliver--> delivered
//! in_transit | customs_clearance | ready_for_delivery --hold--> on_hold
//! on_hold --release--> the state before the hold
//! delivered --close--> closed --reopen--> delivered
//! booked | documents_pending --cancel--> cancelled
//! ```
//!
//! `release` is three transitions with the same rule, one for each state a job
//! can be held from ([`RELEASE_IN_TRANSIT`] and the others): a transition has
//! one target, and the target is never the user's choice, so the service
//! ([`crate::freight_hold`]) runs the one that matches the state stored at
//! `hold`, and each refuses a job held from another state.
//!
//! Renox guards get no `AppState`: what needs the database (the cost lines
//! still unallocated, the invoice issued) is worked out by the service and
//! carried in the [`FreightInput`].

use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::freight::{FreightJob, FreightStatus, track};
use crate::freight_costing;
use crate::freight_hold;
use crate::freight_reopen;
use crate::permissions;
use crate::texts::english;

/// `request_documents`: booked → documents_pending.
pub const REQUEST_DOCUMENTS: &str = "request_documents";
/// `depart`: booked, documents_pending → in_transit.
pub const DEPART: &str = "depart";
/// `arrive`: in_transit → customs_clearance.
pub const ARRIVE: &str = "arrive";
/// `clear`: customs_clearance → ready_for_delivery.
pub const CLEAR: &str = "clear";
/// `hold`: in_transit, customs_clearance, ready_for_delivery → on_hold.
pub const HOLD: &str = "hold";
/// `release` back to `in_transit`.
pub const RELEASE_IN_TRANSIT: &str = "release_in_transit";
/// `release` back to `customs_clearance`.
pub const RELEASE_CUSTOMS_CLEARANCE: &str = "release_customs_clearance";
/// `release` back to `ready_for_delivery`.
pub const RELEASE_READY_FOR_DELIVERY: &str = "release_ready_for_delivery";
/// `deliver`: ready_for_delivery → delivered.
pub const DELIVER: &str = "deliver";
/// `close`: delivered → closed.
pub const CLOSE: &str = "close";
/// `reopen`: closed → delivered.
pub const REOPEN: &str = "reopen";
/// `cancel`: booked, documents_pending → cancelled.
pub const CANCEL: &str = "cancel";

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreightInput {
    /// A reason: the cancellation's, the hold's or the reopening's.
    #[serde(default)]
    pub reason: Option<String>,
    /// A note: what documents are missing, the hold's note, how the hold was
    /// resolved, the reopening's note.
    #[serde(default)]
    pub note: Option<String>,
    /// The bill of lading or air waybill (`depart`).
    #[serde(default)]
    pub bl_awb: Option<String>,
    /// The declaration number (`clear`).
    #[serde(default)]
    pub declaration_no: Option<String>,
    /// The duties, in fils (`clear`).
    #[serde(default)]
    pub duties: Option<i64>,
    /// The delivery order attached (`clear`).
    #[serde(default)]
    pub delivery_order: Option<String>,
    /// The e-POD (`deliver`).
    #[serde(default)]
    pub pod_reference: Option<String>,
    /// How many cost lines have no liability yet (`close`).
    #[serde(default)]
    pub costs_open: i64,
    /// How many revenue lines are not issued (`close`).
    #[serde(default)]
    pub revenue_unissued: i64,
    /// Whether the job has a revenue line at all (`close`).
    #[serde(default)]
    pub has_revenue: bool,
}

/// The input of a run; the default without one (`available`).
pub(crate) fn input_of(ctx: &TransitionCtx) -> FreightInput {
    ctx.input_as::<FreightInput>().unwrap_or_default()
}

/// Whether `text` holds something.
pub(crate) fn given(text: &Option<String>) -> bool {
    text.as_deref().is_some_and(|text| !text.trim().is_empty())
}

pub(crate) fn trimmed(text: &Option<String>) -> Option<String> {
    text.as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// A guard that refuses with the text `key` unless `ok(job, input)`.
pub(crate) fn needs(
    transition: Transition<FreightJob>,
    name: &'static str,
    key: &'static str,
    ok: fn(&FreightJob, &FreightInput) -> bool,
) -> Transition<FreightJob> {
    transition.guard(name, move |job: &FreightJob, ctx| {
        let fine = ok(job, &input_of(ctx));
        Box::pin(async move {
            if fine {
                Ok(())
            } else {
                Err(english(key, &[]))
            }
        })
    })
}

/// A transition by the documentation officer (`jobs.manage`).
fn step(
    name: &'static str,
    label: &'static str,
    from: &[FreightStatus],
    to: FreightStatus,
    event: &'static str,
) -> Transition<FreightJob> {
    Transition::new(name)
        .label(label)
        .tone("primary")
        .permission(permissions::JOBS_MANAGE)
        .from(from.iter().copied())
        .to(to)
        .effect(move |job: &FreightJob, ctx, tx| {
            let id = job.id;
            let actor = ctx.actor().map(|user| user.id);
            let note = input_of(ctx).note.unwrap_or_default();
            Box::pin(async move { track(&mut *tx, id, event, note.trim(), actor).await })
        })
}

fn request_documents() -> Transition<FreightJob> {
    step(
        REQUEST_DOCUMENTS,
        "Request documents",
        &[FreightStatus::Booked],
        FreightStatus::DocumentsPending,
        "documents_requested",
    )
    .effect(|job: &FreightJob, ctx, tx| {
        let id = job.id;
        let note = trimmed(&input_of(ctx).note);
        Box::pin(async move {
            FreightJob::query()
                .where_(FreightJob::ID.eq(id))
                .update(&mut *tx, &[("documents_note", &note)])
                .await?;
            Ok(())
        })
    })
}

fn depart() -> Transition<FreightJob> {
    let transition = step(
        DEPART,
        "Depart",
        &[FreightStatus::Booked, FreightStatus::DocumentsPending],
        FreightStatus::InTransit,
        "departed",
    );
    needs(transition, "bl_or_awb_recorded", "logistics.freight.bl_needed", |job, input| {
        given(&input.bl_awb) || given(&job.bl_awb)
    })
    .effect(|job: &FreightJob, ctx, tx| {
        let id = job.id;
        let bl = trimmed(&input_of(ctx).bl_awb).or_else(|| job.bl_awb.clone());
        Box::pin(async move {
            FreightJob::query()
                .where_(FreightJob::ID.eq(id))
                .update(&mut *tx, &[("bl_awb", &bl)])
                .await?;
            Ok(())
        })
    })
}

fn arrive() -> Transition<FreightJob> {
    step(
        ARRIVE,
        "Arrived",
        &[FreightStatus::InTransit],
        FreightStatus::CustomsClearance,
        "arrived",
    )
}

fn clear() -> Transition<FreightJob> {
    let transition = step(
        CLEAR,
        "Cleared",
        &[FreightStatus::CustomsClearance],
        FreightStatus::ReadyForDelivery,
        "cleared",
    );
    needs(
        transition,
        "declaration_duties_and_delivery_order_recorded",
        "logistics.freight.clearance_needed",
        |job, input| {
            (given(&input.declaration_no) || given(&job.declaration_no))
                && (input.duties.is_some() || job.duties.is_some())
                && (given(&input.delivery_order) || given(&job.delivery_order_ref))
        },
    )
    .effect(|job: &FreightJob, ctx, tx| {
        let id = job.id;
        let input = input_of(ctx);
        let declaration = trimmed(&input.declaration_no).or_else(|| job.declaration_no.clone());
        let duties = input.duties.or(job.duties);
        let order = trimmed(&input.delivery_order).or_else(|| job.delivery_order_ref.clone());
        Box::pin(async move {
            FreightJob::query()
                .where_(FreightJob::ID.eq(id))
                .update(
                    &mut *tx,
                    &[
                        ("declaration_no", &declaration),
                        ("duties", &duties),
                        ("delivery_order_ref", &order),
                    ],
                )
                .await?;
            Ok(())
        })
    })
}

fn deliver() -> Transition<FreightJob> {
    let transition = Transition::new(DELIVER)
        .label("Delivered")
        .tone("primary")
        .permission(permissions::JOBS_DELIVER)
        .from([FreightStatus::ReadyForDelivery])
        .to(FreightStatus::Delivered);
    needs(
        transition,
        "e_pod_captured",
        "logistics.freight.pod_needed",
        |job, input| given(&input.pod_reference) || given(&job.pod_ref),
    )
    .effect(|job: &FreightJob, ctx, tx| {
        let id = job.id;
        let actor = ctx.actor().map(|user| user.id);
        let pod = trimmed(&input_of(ctx).pod_reference).or_else(|| job.pod_ref.clone());
        Box::pin(async move {
            FreightJob::query()
                .where_(FreightJob::ID.eq(id))
                .update(&mut *tx, &[("pod_ref", &pod)])
                .await?;
            track(&mut *tx, id, "delivered", pod.as_deref().unwrap_or_default(), actor).await
        })
    })
}

fn close() -> Transition<FreightJob> {
    let transition = Transition::new(CLOSE)
        .label("Close")
        .tone("primary")
        .permission(permissions::JOBS_CLOSE)
        .from([FreightStatus::Delivered])
        .to(FreightStatus::Closed);
    let transition = needs(
        transition,
        "supplier_bills_and_disbursements_allocated",
        "logistics.freight.costs_open",
        |_job, input| input.costs_open == 0,
    );
    needs(
        transition,
        "invoice_issued",
        "logistics.freight.invoice_needed",
        |_job, input| input.has_revenue && input.revenue_unissued == 0,
    )
    .effect(|job: &FreightJob, ctx, tx| {
        let id = job.id;
        let actor = ctx.actor().map(|user| user.id);
        Box::pin(async move {
            let at = Some(renox::db::now());
            FreightJob::query()
                .where_(FreightJob::ID.eq(id))
                .update(&mut *tx, &[("closed_at", &at)])
                .await?;
            // The job margin is final: kept at each close.
            let margin = freight_costing::record_margin(&mut *tx, id, actor).await?;
            let note = format!("margin {}", margin.margin);
            track(&mut *tx, id, "closed", &note, actor).await
        })
    })
}

fn cancel() -> Transition<FreightJob> {
    let transition = step(
        CANCEL,
        "Cancel",
        &[FreightStatus::Booked, FreightStatus::DocumentsPending],
        FreightStatus::Cancelled,
        "cancelled",
    );
    needs(transition, "reason_given", "logistics.freight.reason_needed", |_job, input| {
        given(&input.reason)
    })
    .effect(|job: &FreightJob, ctx, tx| {
        let id = job.id;
        let actor = ctx.actor().map(|user| user.id);
        let reason = trimmed(&input_of(ctx).reason);
        Box::pin(async move {
            FreightJob::query()
                .where_(FreightJob::ID.eq(id))
                .update(&mut *tx, &[("cancel_reason", &reason)])
                .await?;
            track(&mut *tx, id, "cancelled", reason.as_deref().unwrap_or_default(), actor).await
        })
    })
}

/// Every transition of the machine; the freight issues add theirs here.
pub(crate) fn transitions() -> Vec<Transition<FreightJob>> {
    vec![
        request_documents(),
        depart(),
        arrive(),
        clear(),
        deliver(),
        close(),
        cancel(),
        freight_reopen::reopen_transition(),
        freight_hold::hold_transition(),
        freight_hold::release_transition(RELEASE_IN_TRANSIT, FreightStatus::InTransit),
        freight_hold::release_transition(
            RELEASE_CUSTOMS_CLEARANCE,
            FreightStatus::CustomsClearance,
        ),
        freight_hold::release_transition(
            RELEASE_READY_FOR_DELIVERY,
            FreightStatus::ReadyForDelivery,
        ),
    ]
}

impl StateMachine for FreightJob {
    type State = FreightStatus;
    const COLUMN: renox::db::Col<FreightJob, FreightStatus> = FreightJob::STATUS;

    fn machine() -> Machine<Self> {
        use FreightStatus::{
            Booked, Cancelled, Closed, CustomsClearance, Delivered, DocumentsPending, InTransit,
            OnHold, ReadyForDelivery,
        };
        let machine = Machine::new()
            .initial([Booked])
            .state(Booked)
            .tone("neutral")
            .state(DocumentsPending)
            .tone("warning")
            .state(InTransit)
            .tone("info")
            .state(CustomsClearance)
            .tone("info")
            .state(ReadyForDelivery)
            .tone("info")
            .state(OnHold)
            .tone("warning")
            .state(Delivered)
            .tone("success")
            .state(Closed)
            .tone("success")
            .state(Cancelled)
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
        let machine = FreightJob::machine();
        assert_eq!(machine.states().len(), 9);
        for name in [
            HOLD,
            REOPEN,
            RELEASE_IN_TRANSIT,
            RELEASE_CUSTOMS_CLEARANCE,
            RELEASE_READY_FOR_DELIVERY,
            REQUEST_DOCUMENTS,
            DEPART,
            ARRIVE,
            CLEAR,
            DELIVER,
            CLOSE,
            CANCEL,
        ] {
            assert!(machine.find(name).is_ok(), "{name}");
        }
        let depart = machine.find(DEPART).unwrap();
        assert_eq!(depart.from_states(), ["booked", "documents_pending"]);
    }
}
