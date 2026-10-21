//! The Purchase order (wl-inventory §3, §4 *Purchase order*): an order to a
//! supplier, with its lines and the quantity accepted on each. Only what
//! receiving needs is built: the model, its lines, the machine's states and
//! `receive`, the system transition a posted goods receipt runs. `issue`,
//! `revise`, `short_close`, `close` and `cancel` come with the issues that
//! build them; a fixture is inserted in the state it needs. The system's
//! `reopen` is here as well.
//!
//! ```text
//! issued, partially_received --receive--> partially_received
//! issued, partially_received --receive_in_full--> received
//! ```
//!
//! ```text
//! received --reopen--> partially_received
//! ```
//!
//! `reopen` is run by the system when goods of a fully received order go back
//! to the supplier for replacement (a Return to supplier `dispatch`) or a
//! receipt is reversed: the returned quantity is outstanding again, so the
//! replacement is received on the order and its committed cost
//! ([`committed_value`]: outstanding quantity at the order's price) is back.
//! Users are not offered it, and a `closed` order is not reopened.
//!
//! The spec has one `receive` with two targets; as with a payment's
//! `allocate` and an invoice's, it is two transitions here, `receive` when
//! something is still outstanding and `receive_in_full` when every line is
//! fully accepted, and a receipt picks one by the quantities.
//!
//! Only the accepted quantity counts: a rejected quantity stays outstanding
//! (`outstanding = ordered - accepted`, never below 0). The accepted
//! quantity may go above the ordered one by the tolerance (§7, 5%); beyond it
//! the receipt needed an approval first, and `approved` says it had one.

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::PURCHASE_ORDER_SUBJECT_TYPE;

use crate::item::value_of;
use crate::texts::said;

/// The `receive` transition's name: some quantity is still outstanding.
pub const RECEIVE: &str = "receive";

/// The `receive_in_full` transition's name: every line is fully accepted.
pub const RECEIVE_IN_FULL: &str = "receive_in_full";

/// The `reopen` transition's name.
pub const REOPEN: &str = "reopen";

/// Where a purchase order stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PurchaseOrderStatus {
    /// Being prepared.
    #[default]
    Draft,
    /// Approved and sent to the supplier.
    Issued,
    /// Some quantities received.
    PartiallyReceived,
    /// All quantities received (or short-closed).
    Received,
    /// Received and matched to supplier bills (terminal).
    Closed,
    /// Cancelled before any receipt (terminal).
    Cancelled,
}

/// An order to a supplier. Money is fils of [`PurchaseOrder::currency`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_purchase_orders",
    state_machine,
    unique(number),
    index(status),
    index(supplier_party_id)
)]
pub struct PurchaseOrder {
    /// The primary key.
    pub id: i64,
    /// The number from the `PO-` series, set at `issue`.
    pub number: Option<String>,
    /// The supplier.
    pub supplier_party_id: i64,
    /// The delivery location.
    pub location_id: i64,
    /// The ISO currency code.
    pub currency: String,
    /// The revision number.
    pub revision: i64,
    /// The day the goods are expected.
    pub expected_date: Option<NaiveDate>,
    /// Who asked for the expected date of the revision in force when a
    /// revision set a later one: `us` or `supplier`.
    pub date_requested_by: Option<String>,
    /// The PDF document of the revision in force, "Revision N" from the
    /// second revision on.
    pub document: Option<String>,
    /// When the order was issued.
    pub issued_at: Option<DateTime>,
    /// When the revision in force was issued: the issue, or the last revision.
    pub revision_issued_at: Option<DateTime>,
    /// The lifecycle state; changed only by a transition.
    pub status: PurchaseOrderStatus,
    /// The user who created it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PurchaseOrder {
    /// A draft order to `supplier` for delivery at `location_id`, in AED.
    pub fn new(supplier: wl_kernel::PartyId, location_id: i64) -> Self {
        PurchaseOrder {
            supplier_party_id: supplier.0,
            location_id,
            currency: "AED".to_owned(),
            revision: 1,
            ..PurchaseOrder::default()
        }
    }

    /// The order as a [`Subject`]: `inventory.purchase_order:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(PURCHASE_ORDER_SUBJECT_TYPE, self.id)
    }

    /// The supplier.
    pub fn supplier(&self) -> wl_kernel::PartyId {
        wl_kernel::PartyId(self.supplier_party_id)
    }

    /// Whether goods can be received on it now.
    pub fn is_open_for_receiving(&self) -> bool {
        matches!(
            self.status,
            PurchaseOrderStatus::Issued | PurchaseOrderStatus::PartiallyReceived
        )
    }
}

/// One line of a purchase order. Quantities are thousandths of the item's
/// base unit, money is fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_purchase_order_lines",
    unique(purchase_order_id, position)
)]
pub struct PurchaseOrderLine {
    /// The primary key.
    pub id: i64,
    /// The order.
    pub purchase_order_id: i64,
    /// The order of the line, from 1.
    pub position: i64,
    /// The item.
    pub item_id: i64,
    /// What was ordered.
    pub quantity_milli: i64,
    /// The price of one base unit, before VAT.
    pub unit_price: i64,
    /// What was accepted so far; rejected goods are not in it.
    pub accepted_qty_milli: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PurchaseOrderLine {
    /// A line of `quantity_milli` of `item_id` at `unit_price`.
    pub fn new(
        purchase_order_id: i64,
        position: i64,
        item_id: i64,
        quantity_milli: i64,
        unit_price: i64,
    ) -> Self {
        PurchaseOrderLine {
            purchase_order_id,
            position,
            item_id,
            quantity_milli,
            unit_price,
            ..PurchaseOrderLine::default()
        }
    }

    /// What is still to come: ordered less accepted, never below 0.
    pub fn outstanding_milli(&self) -> i64 {
        (self.quantity_milli - self.accepted_qty_milli).max(0)
    }
}

/// The committed cost of an order: what is still outstanding on its lines at
/// the order's prices, in fils (`FIN.CST.01`).
pub fn committed_value(lines: &[PurchaseOrderLine]) -> i64 {
    lines
        .iter()
        .map(|line| value_of(line.outstanding_milli(), line.unit_price))
        .sum()
}

/// Reopens order `order_id` inside `tx` when it is `received`: the system's
/// `reopen`, run once the returned quantity is outstanding again. An order in
/// any other state is left as it is (a `closed` one is never reopened).
/// Whether it was reopened.
///
/// # Errors
///
/// A database error, or a refusal of the transition.
pub async fn reopen_in(tx: &mut Transaction, order_id: i64) -> Result<bool> {
    let order = PurchaseOrder::find_or_404(&mut *tx, order_id).await?;
    if order.status != PurchaseOrderStatus::Received {
        return Ok(false);
    }
    // Nothing listens to the order yet: the outcome is not emitted.
    let _outcome = order
        .transition(REOPEN)
        .by_system()
        .run_in(&mut *tx)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(true)
}

/// The lines of order `purchase_order_id`, in order, read in `tx`.
pub async fn lines_in(
    tx: &mut Transaction,
    purchase_order_id: i64,
) -> Result<Vec<PurchaseOrderLine>> {
    PurchaseOrderLine::query()
        .where_(PurchaseOrderLine::PURCHASE_ORDER_ID.eq(purchase_order_id))
        .order_by(PurchaseOrderLine::POSITION)
        .get(&mut *tx)
        .await
}

/// The total of the lines, excluding VAT, in minor units of the currency:
/// ordered quantity times price per line (not what is outstanding).
pub fn total_of(lines: &[PurchaseOrderLine]) -> i64 {
    lines
        .iter()
        .map(|line| value_of(line.quantity_milli, line.unit_price))
        .sum()
}

/// The lines of order `purchase_order_id`, in order.
pub async fn lines_of(db: &Db, purchase_order_id: i64) -> Result<Vec<PurchaseOrderLine>> {
    PurchaseOrderLine::query()
        .where_(PurchaseOrderLine::PURCHASE_ORDER_ID.eq(purchase_order_id))
        .order_by(PurchaseOrderLine::POSITION)
        .get(db)
        .await
}

/// How far `accepted_total` is above `ordered`, in basis points of what was
/// ordered, rounded up so that a fraction over the tolerance is over it. 0
/// when it is not above. A line ordered at 0 with anything accepted is far
/// above.
pub fn over_receipt_bps(ordered_milli: i64, accepted_total_milli: i64) -> i64 {
    let excess = i128::from(accepted_total_milli) - i128::from(ordered_milli);
    if excess <= 0 {
        return 0;
    }
    if ordered_milli <= 0 {
        return 1_000_000;
    }
    let bps = (excess * 10_000 + i128::from(ordered_milli) - 1) / i128::from(ordered_milli);
    i64::try_from(bps).unwrap_or(1_000_000)
}

/// One line of the order as a `receive` run sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiveLine {
    /// The order line.
    pub po_line_id: i64,
    /// Its position, for the messages.
    pub position: i64,
    /// What was ordered.
    pub ordered_milli: i64,
    /// What was accepted before this receipt.
    pub accepted_before_milli: i64,
    /// What this receipt accepts on it.
    pub add_milli: i64,
}

impl ReceiveLine {
    /// The accepted quantity once this receipt is in.
    pub fn accepted_after_milli(&self) -> i64 {
        self.accepted_before_milli + self.add_milli
    }
}

/// What a `receive` run carries to its guards and effect: every line of the
/// order, the tolerance and whether the receipt had its approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReceiveInput {
    /// Every line of the order, with what this receipt adds.
    pub lines: Vec<ReceiveLine>,
    /// The over-receipt tolerance, in basis points.
    pub tolerance_bps: i64,
    /// Whether the receipt was approved above the tolerance.
    pub approved: bool,
}

impl ReceiveInput {
    /// Whether every line is fully accepted once this receipt is in.
    pub fn is_complete(&self) -> bool {
        self.lines
            .iter()
            .all(|line| line.accepted_after_milli() >= line.ordered_milli)
    }

    /// The transition this receipt runs.
    pub fn transition(&self) -> &'static str {
        if self.is_complete() {
            RECEIVE_IN_FULL
        } else {
            RECEIVE
        }
    }

    /// The first line above the tolerance, if the receipt had no approval.
    pub fn over_tolerance(&self) -> Option<&ReceiveLine> {
        if self.approved {
            return None;
        }
        self.lines.iter().find(|line| {
            line.add_milli > 0
                && over_receipt_bps(line.ordered_milli, line.accepted_after_milli())
                    > self.tolerance_bps
        })
    }
}

fn input(ctx: &TransitionCtx) -> Option<ReceiveInput> {
    ctx.input_as::<ReceiveInput>().ok()
}

/// The guards and the effect `receive` and `receive_in_full` share.
fn rules(transition: Transition<PurchaseOrder>) -> Transition<PurchaseOrder> {
    use PurchaseOrderStatus::{Issued, PartiallyReceived};
    transition
        .system()
        .from([Issued, PartiallyReceived])
        .guard("quantities_given", |_order: &PurchaseOrder, ctx| {
            let given = input(ctx).is_some_and(|input| !input.lines.is_empty());
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(said("wl_inventory.order.bad_input", &[]))
                }
            })
        })
        .guard("within_tolerance", |_order: &PurchaseOrder, ctx| {
            let over = input(ctx)
                .and_then(|input| input.over_tolerance().map(|line| line.position.to_string()));
            Box::pin(async move {
                match over {
                    Some(position) => Err(said(
                        "wl_inventory.order.over_tolerance",
                        &[("position", &position)],
                    )),
                    None => Ok(()),
                }
            })
        })
        .effect(|_order: &PurchaseOrder, ctx, tx| {
            let input = input(ctx).unwrap_or_default();
            Box::pin(async move {
                let now = renox::db::now();
                for line in input.lines.iter().filter(|line| line.add_milli > 0) {
                    let accepted = line.accepted_after_milli();
                    PurchaseOrderLine::query()
                        .where_(PurchaseOrderLine::ID.eq(line.po_line_id))
                        .update(
                            &mut *tx,
                            &[("accepted_qty_milli", &accepted), ("updated_at", &now)],
                        )
                        .await?;
                }
                Ok(())
            })
        })
}

impl StateMachine for PurchaseOrder {
    type State = PurchaseOrderStatus;
    const COLUMN: renox::db::Col<PurchaseOrder, PurchaseOrderStatus> = PurchaseOrder::STATUS;

    fn machine() -> Machine<Self> {
        use PurchaseOrderStatus::{Cancelled, Closed, Draft, Issued, PartiallyReceived, Received};
        let machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Issued)
            .tone("info")
            .state(PartiallyReceived)
            .tone("info")
            .state(Received)
            .tone("success")
            .state(Closed)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(rules(
                Transition::new(RECEIVE)
                    .label("Receive")
                    .to(PartiallyReceived),
            ))
            .transition(rules(
                Transition::new(RECEIVE_IN_FULL)
                    .label("Receive in full")
                    .to(Received),
            ))
            .transition(
                Transition::new(REOPEN)
                    .label("Reopen")
                    .system()
                    .from([Received])
                    .to(PartiallyReceived),
            );
        crate::po_revise::add_to(crate::po_issue::add_to(machine))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(ordered: i64, before: i64, add: i64) -> ReceiveLine {
        ReceiveLine {
            po_line_id: 1,
            position: 1,
            ordered_milli: ordered,
            accepted_before_milli: before,
            add_milli: add,
        }
    }

    #[test]
    fn what_is_outstanding_is_ordered_less_accepted_and_never_below_zero() {
        let mut line = PurchaseOrderLine::new(1, 1, 2, 100_000, 1_200);
        assert_eq!(line.outstanding_milli(), 100_000);
        line.accepted_qty_milli = 80_000;
        assert_eq!(line.outstanding_milli(), 20_000);
        line.accepted_qty_milli = 106_000;
        assert_eq!(line.outstanding_milli(), 0);
    }

    #[test]
    fn the_over_receipt_is_rounded_up_in_basis_points() {
        assert_eq!(over_receipt_bps(100_000, 100_000), 0);
        assert_eq!(over_receipt_bps(100_000, 80_000), 0);
        assert_eq!(over_receipt_bps(100_000, 105_000), 500);
        assert_eq!(over_receipt_bps(100_000, 106_000), 600);
        // A fraction over the tolerance is over it.
        assert_eq!(over_receipt_bps(100_000, 105_001), 501);
        assert!(over_receipt_bps(0, 1) > 10_000);
    }

    #[test]
    fn a_receipt_is_complete_when_every_line_is_fully_accepted() {
        let partial = ReceiveInput {
            lines: vec![line(100_000, 0, 80_000)],
            tolerance_bps: 500,
            approved: false,
        };
        assert!(!partial.is_complete());
        assert_eq!(partial.transition(), RECEIVE);
        let full = ReceiveInput {
            lines: vec![line(100_000, 80_000, 20_000), line(10_000, 10_000, 0)],
            tolerance_bps: 500,
            approved: false,
        };
        assert!(full.is_complete());
        assert_eq!(full.transition(), RECEIVE_IN_FULL);
    }

    #[test]
    fn the_tolerance_is_checked_on_the_accepted_quantity_unless_approved() {
        let mut input = ReceiveInput {
            lines: vec![line(100_000, 0, 105_000)],
            tolerance_bps: 500,
            approved: false,
        };
        assert!(input.over_tolerance().is_none());
        input.lines[0].add_milli = 106_000;
        assert!(input.over_tolerance().is_some());
        input.approved = true;
        assert!(input.over_tolerance().is_none());
        // A line the receipt does not touch is not checked.
        let untouched = ReceiveInput {
            lines: vec![line(100_000, 120_000, 0)],
            tolerance_bps: 500,
            approved: false,
        };
        assert!(untouched.over_tolerance().is_none());
    }

    #[test]
    fn the_committed_value_is_what_is_outstanding_at_the_orders_prices() {
        let mut first = PurchaseOrderLine::new(1, 1, 2, 50_000, 5_000);
        let second = PurchaseOrderLine::new(1, 2, 3, 10_000, 100);
        assert_eq!(
            committed_value(&[first.clone(), second.clone()]),
            250_000 + 1_000
        );
        first.accepted_qty_milli = 50_000;
        assert_eq!(committed_value(&[first.clone(), second]), 1_000);
        // 10 come back for replacement: 10 x AED 50.
        first.accepted_qty_milli = 40_000;
        assert_eq!(committed_value(&[first]), 50_000);
        assert_eq!(committed_value(&[]), 0);
    }

    #[test]
    fn reopen_is_for_the_system_and_only_from_received() {
        let machine = PurchaseOrder::machine();
        let reopen = machine.find(REOPEN).expect("transition");
        assert!(reopen.is_system());
        assert_eq!(reopen.from_states(), ["received"]);
        assert_eq!(reopen.to_state(), Some("partially_received"));
    }

    #[test]
    fn receive_is_a_system_transition_from_the_open_states() {
        let machine = PurchaseOrder::machine();
        for name in [RECEIVE, RECEIVE_IN_FULL] {
            let transition = machine.find(name).expect("transition");
            assert!(transition.is_system(), "{name}");
            assert_eq!(transition.from_states(), ["issued", "partially_received"]);
        }
        assert_eq!(
            machine.find(RECEIVE).unwrap().to_state(),
            Some("partially_received")
        );
        assert_eq!(
            machine.find(RECEIVE_IN_FULL).unwrap().to_state(),
            Some("received")
        );
        assert_eq!(machine.initial_states(), ["draft"]);
    }
}
