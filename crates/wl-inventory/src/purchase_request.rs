//! The Purchase request (wl-inventory §3, §4 *Purchase request*): an
//! internal need at a location, with the cost object it is for. Only what
//! stopping a scope needs is built: the model, the machine's states and
//! `cancel`, which a holder of `purchase_requests.approve` runs and the
//! system runs (`auto_cancel`) when a project is terminated. `submit`,
//! `approve`, `reject` and `order` come with the issues that build them; a
//! fixture is inserted in the state it needs. The lines of a request come
//! with them as well.
//!
//! ```text
//! draft, submitted, approved --cancel--> cancelled
//! draft, submitted, approved --auto_cancel--> cancelled
//! ```
//!
//! A request in `ordered` is covered by purchase orders and is never
//! cancelled: its orders are cancelled or short-closed instead.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{Machine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::PURCHASE_REQUEST_SUBJECT_TYPE;

use crate::permissions;

/// The `cancel` transition's name.
pub const CANCEL: &str = "cancel";

/// The system's `auto_cancel` transition's name.
pub const AUTO_CANCEL: &str = "auto_cancel";

/// Where a purchase request stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PurchaseRequestStatus {
    /// Being written.
    #[default]
    Draft,
    /// Waiting for approval.
    Submitted,
    /// Can be ordered.
    Approved,
    /// Covered by one or more purchase orders (terminal).
    Ordered,
    /// Not approved (terminal).
    Rejected,
    /// Withdrawn (terminal).
    Cancelled,
}

impl PurchaseRequestStatus {
    /// The states a request can be cancelled from: no order is issued yet.
    pub const OPEN: [PurchaseRequestStatus; 3] = [
        PurchaseRequestStatus::Draft,
        PurchaseRequestStatus::Submitted,
        PurchaseRequestStatus::Approved,
    ];
}

/// An internal need for goods at a location.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_purchase_requests",
    state_machine,
    index(location_id)
)]
pub struct PurchaseRequest {
    /// The primary key.
    pub id: i64,
    /// The location the goods are needed at.
    pub location_id: i64,
    /// The cost object, as a subject text (`construction.cost_code:3`).
    pub cost_object: Option<String>,
    /// The day the goods are needed by.
    pub needed_by: Option<NaiveDate>,
    /// The user who wrote it.
    pub requester_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: PurchaseRequestStatus,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PurchaseRequest {
    /// A draft request for goods at `location_id`.
    pub fn new(location_id: i64) -> Self {
        PurchaseRequest {
            location_id,
            ..PurchaseRequest::default()
        }
    }

    /// The request as a [`Subject`]: `inventory.purchase_request:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(PURCHASE_REQUEST_SUBJECT_TYPE, self.id)
    }
}

/// Stores the reason of a cancellation; the status is the machine's.
fn cancelling(transition: Transition<PurchaseRequest>) -> Transition<PurchaseRequest> {
    transition
        .from(PurchaseRequestStatus::OPEN)
        .to(PurchaseRequestStatus::Cancelled)
        .reason_required()
        .effect(|request: &PurchaseRequest, ctx, tx| {
            let id = request.id;
            let reason = ctx.reason().map(|reason| reason.trim().to_owned());
            Box::pin(async move {
                PurchaseRequest::query()
                    .where_(PurchaseRequest::ID.eq(id))
                    .update(&mut *tx, &[("cancel_reason", &reason)])
                    .await?;
                Ok(())
            })
        })
}

impl renox::state_machine::StateMachine for PurchaseRequest {
    type State = PurchaseRequestStatus;
    const COLUMN: renox::db::Col<PurchaseRequest, PurchaseRequestStatus> = PurchaseRequest::STATUS;

    fn machine() -> Machine<Self> {
        use PurchaseRequestStatus::{Approved, Cancelled, Draft, Ordered, Rejected, Submitted};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Submitted)
            .tone("info")
            .state(Approved)
            .tone("success")
            .state(Ordered)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(cancelling(
                Transition::new(CANCEL)
                    .label("Cancel")
                    .tone("danger")
                    .permission(permissions::PURCHASE_REQUESTS_APPROVE),
            ))
            .transition(cancelling(
                Transition::new(AUTO_CANCEL)
                    .label("Cancel (system)")
                    .system(),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::StateMachine;

    #[test]
    fn only_a_request_without_an_order_is_cancelled() {
        let machine = PurchaseRequest::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        for name in [CANCEL, AUTO_CANCEL] {
            let transition = machine.find(name).expect("transition");
            assert_eq!(transition.from_states(), ["draft", "submitted", "approved"]);
            assert_eq!(transition.to_state(), Some("cancelled"));
            assert!(transition.needs_reason());
        }
        assert!(!machine.find(CANCEL).expect("transition").is_system());
        assert!(machine.find(AUTO_CANCEL).expect("transition").is_system());
    }
}
