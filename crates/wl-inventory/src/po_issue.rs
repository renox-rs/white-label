//! Issuing a purchase order (wl-inventory §4 *Purchase order* `issue`, issue
//! #246). Only what refusing a blocked supplier needs is built here: the
//! `issue` transition itself (`draft` to `issued`, a holder of
//! `purchase_orders.issue`), its block guard and the exception.
//!
//! - the guards: the order has a supplier and a delivery location, and at
//!   least one line with a quantity and a price. They are read in the
//!   transaction of the run, so a refusal leaves the order a `draft`;
//! - `BlockGuard::check(supplier, start)` (`PTY.ACL.03`) runs first, in the
//!   same transaction ([`wl_kernel::block_guard::run_guarded`]), before any
//!   other check, so no approval request for the order itself is made for a
//!   blocked supplier. The refusal shows the neutral *on hold* text, and the
//!   real reasons to a holder of `parties.block_reason.view`;
//! - an exception ([`issue`] with `exception`) is an approval request of the
//!   action `parties.block_exception` for the role the guard names. When it
//!   is approved the kernel runs the system's `issue_approved` once, as the
//!   requester asked it; a revision of that order needs an exception of its
//!   own. The final approval checks the guard again for any other request;
//! - the effect: the number from the `PO-` series (taken in the same
//!   transaction, so a refusal leaves no gap), and the PDF document of
//!   revision 1.
//!
//! - a vertical's rule (`PurchaseOrderRules`, 02 `CON.ACL.13`) may ask for an
//!   approval of its own before the order is issued: after the block guard
//!   passed, an approval request for the steps the rule names is made, the
//!   order stays a `draft` ([`IssueOutcome::ApprovalRequested`]), and the
//!   kernel runs `issue_approved` at the last approval. The requester never
//!   approves.
//!
//! Decisions: the segregation of duties and the value limits of `issue`
//! (`STK.ACL.01`, `STK.ACL.03`) are left to the verticals that set limits
//! (none is registered here), and `PurchaseOrderIssued` comes with the issue
//! that builds the committed cost. A goods receipt is never refused by the
//! guard, nor is a return to the supplier.

use renox::db::Transaction;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{Machine, Transition, TransitionError};
use wl_kernel::approvals::{self, ApprovalRequest, Require, Required};
use wl_kernel::block_guard::{self, ActionKind, BlockRefusal, GuardOutcome, GuardedError};
use wl_kernel::inventory::{OrderFacts, order_rules};
use wl_kernel::numbering::{Reset, Series};

use crate::item::Item;
use crate::permissions;
use crate::po_revise::{Sheet, SheetLine, render};
use crate::purchase_order::{PurchaseOrder, PurchaseOrderStatus, lines_in};
use crate::texts::said;

/// The user's `issue` transition's name.
pub const ISSUE: &str = "issue";

/// The system's `issue_approved` transition's name: the kernel runs it when
/// the exception for a blocked supplier is approved.
pub const ISSUE_APPROVED: &str = "issue_approved";

/// The numbers of purchase orders: `PO-2026-0001`.
pub const PURCHASE_ORDER_SERIES: Series = Series {
    key: "purchase_order",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "PO",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: false,
};

/// What [`issue`] did.
#[derive(Debug)]
pub enum IssueOutcome {
    /// The order is `issued`.
    Issued(PurchaseOrder),
    /// The supplier is blocked: nothing was written.
    Refused(BlockRefusal),
    /// The exception was asked for the approver the guard names; the order
    /// stays a `draft`.
    ExceptionRequested(ApprovalRequest),
    /// A vertical's rule needs an approval before the order is issued
    /// (`CON.ACL.13`); the order stays a `draft` until the last step approves.
    ApprovalRequested(ApprovalRequest),
}

/// The first thing that keeps `order` from being issued, read in `tx`.
async fn problem_in(tx: &mut Transaction, order: &PurchaseOrder) -> Result<Option<String>> {
    if order.supplier_party_id == 0 {
        return Ok(Some(said("wl_inventory.issue.no_supplier", &[])));
    }
    if order.location_id == 0 {
        return Ok(Some(said("wl_inventory.issue.no_location", &[])));
    }
    let lines = lines_in(tx, order.id).await?;
    if lines.is_empty() {
        return Ok(Some(said("wl_inventory.issue.no_lines", &[])));
    }
    if let Some(line) = lines
        .iter()
        .find(|line| line.quantity_milli <= 0 || line.unit_price <= 0)
    {
        return Ok(Some(said(
            "wl_inventory.issue.no_price",
            &[("position", &line.position.to_string())],
        )));
    }
    Ok(None)
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The effect of `issue` and `issue_approved`: check again, number the order
/// and render its document.
async fn issue_in(tx: &mut Transaction, order: PurchaseOrder) -> Result {
    if let Some(problem) = problem_in(tx, &order).await? {
        return Err(refused(problem));
    }
    let number = PURCHASE_ORDER_SERIES
        .next(
            tx,
            None,
            None,
            wl_kernel::documents::dubai_date(renox::db::now()),
        )
        .await?;
    let lines = lines_in(tx, order.id).await?;
    let mut sheet_lines = Vec::new();
    for line in &lines {
        let item = Item::find_or_404(&mut *tx, line.item_id).await?;
        sheet_lines.push(SheetLine {
            item: item.code.clone(),
            quantity_milli: line.quantity_milli,
            unit: item.base_unit.clone(),
            unit_price: line.unit_price,
        });
    }
    let document = render(&Sheet {
        number: number.clone(),
        revision: 1,
        supplier: format!("Supplier {}", order.supplier_party_id),
        currency: order.currency.clone(),
        expected: order.expected_date.map(|date| date.to_string()),
        reason: None,
        lines: sheet_lines,
    });
    let now = renox::db::now();
    PurchaseOrder::query()
        .where_(PurchaseOrder::ID.eq(order.id))
        .update(
            &mut *tx,
            &[
                ("number", &Some(number)),
                ("document", &Some(document)),
                ("issued_at", &Some(now)),
                ("revision_issued_at", &Some(now)),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

fn rules(transition: Transition<PurchaseOrder>) -> Transition<PurchaseOrder> {
    transition
        .from([PurchaseOrderStatus::Draft])
        .to(PurchaseOrderStatus::Issued)
        .effect(|order: &PurchaseOrder, _ctx, tx| {
            let order = order.clone();
            Box::pin(async move { issue_in(tx, order).await })
        })
}

/// The two transitions.
pub(crate) fn transitions() -> Vec<Transition<PurchaseOrder>> {
    vec![
        rules(
            Transition::new(ISSUE)
                .label("Issue")
                .tone("primary")
                .permission(permissions::PURCHASE_ORDERS_ISSUE),
        ),
        rules(
            Transition::new(ISSUE_APPROVED)
                .label("Issue (exception approved)")
                .system(),
        ),
    ]
}

/// The machine of the order with the issue transitions.
pub(crate) fn add_to(machine: Machine<PurchaseOrder>) -> Machine<PurchaseOrder> {
    transitions().into_iter().fold(machine, Machine::transition)
}

/// `user` issues order `order_id`, or asks for the exception when
/// `exception` is set. A blocked supplier is [`IssueOutcome::Refused`] and
/// nothing is written.
///
/// # Errors
///
/// `Forbidden` without `purchase_orders.issue`; `Guard` (422) for a problem
/// of the order, or an exception asked for a supplier that is not blocked.
pub async fn issue(
    state: &AppState,
    user: &AuthUser,
    order_id: i64,
    exception: bool,
) -> std::result::Result<IssueOutcome, TransitionError> {
    let holders = renox::auth::permissions::users_with_permission_in(
        &state.db,
        permissions::PURCHASE_ORDERS_ISSUE,
        &renox::auth::permissions::Scope::global(),
    )
    .await?;
    if !holders.iter().any(|holder| holder.id == user.id) {
        return Err(TransitionError::Failed(Error::Forbidden));
    }
    let order = PurchaseOrder::find_or_404(&state.db, order_id).await?;
    if order.status != PurchaseOrderStatus::Draft {
        return Err(TransitionError::Guard(said(
            "wl_inventory.issue.not_draft",
            &[("status", &format!("{:?}", order.status))],
        )));
    }
    if exception {
        return ask_exception(state, user, &order).await;
    }
    if let Some(request) = ask_rules(state, user, &order).await? {
        return Ok(request);
    }
    match block_guard::run_guarded(state, user, &order, ISSUE, json!({})).await {
        Ok(_outcome) => {
            let done = PurchaseOrder::find_or_404(&state.db, order.id).await?;
            Ok(IssueOutcome::Issued(done))
        }
        Err(GuardedError::Blocked(refusal)) => Ok(IssueOutcome::Refused(refusal)),
        Err(GuardedError::Transition(error)) => Err(error),
    }
}

/// The approval a vertical's rule asks for before `order` is issued, if any.
/// The order is checked first, and so is the block guard: a blocked supplier
/// is refused before any request of the rule is made.
async fn ask_rules(
    state: &AppState,
    user: &AuthUser,
    order: &PurchaseOrder,
) -> std::result::Result<Option<IssueOutcome>, TransitionError> {
    let scope = crate::location::Location::find(&state.db, order.location_id)
        .await?
        .and_then(|location| location.scope_subject());
    let facts = OrderFacts {
        order: order.subject(),
        supplier: order.supplier(),
        scope,
        on_date: wl_kernel::documents::today(),
        transition: ISSUE.to_owned(),
    };
    let Some(extra) = order_rules(state).approval_for(state, &facts).await? else {
        return Ok(None);
    };
    let mut tx = state.db.begin().await?;
    if let Some(problem) = problem_in(&mut tx, order).await? {
        return Err(TransitionError::Guard(problem));
    }
    let blocked = block_guard::guard(state)
        .check_in(&mut tx, Some(user), order.supplier(), ActionKind::Start)
        .await?;
    if let GuardOutcome::Blocked(refusal) = blocked {
        return Ok(Some(IssueOutcome::Refused(refusal)));
    }
    let require =
        Require::new(extra.action, order.subject(), ISSUE, user.id, "draft").steps(extra.steps);
    match approvals::require_in(&mut tx, &require).await? {
        Required::Allowed => Ok(None),
        Required::Pending(request) => {
            tx.commit().await?;
            approvals::announce_request(state, &request).await?;
            Ok(Some(IssueOutcome::ApprovalRequested(*request)))
        }
    }
}

/// The exception request for a blocked supplier. The order is checked first,
/// so an order that could not be issued anyway is not asked for.
async fn ask_exception(
    state: &AppState,
    user: &AuthUser,
    order: &PurchaseOrder,
) -> std::result::Result<IssueOutcome, TransitionError> {
    let mut tx = state.db.begin().await?;
    if let Some(problem) = problem_in(&mut tx, order).await? {
        return Err(TransitionError::Guard(problem));
    }
    let request =
        block_guard::request_exception_in(state, &mut tx, user, &order.subject(), ISSUE, json!({}))
            .await?;
    tx.commit().await?;
    approvals::announce_request(state, &request).await?;
    Ok(IssueOutcome::ExceptionRequested(request))
}

#[cfg(test)]
mod tests {
    use renox::state_machine::StateMachine;

    use super::*;

    #[test]
    fn issue_is_a_user_transition_and_its_twin_is_the_systems() {
        let machine = PurchaseOrder::machine();
        let issue = machine.find(ISSUE).expect("transition");
        assert!(!issue.is_system());
        assert_eq!(issue.from_states(), ["draft"]);
        assert_eq!(issue.to_state(), Some("issued"));
        let approved = machine.find(ISSUE_APPROVED).expect("transition");
        assert!(approved.is_system());
        assert_eq!(approved.from_states(), ["draft"]);
    }
}
