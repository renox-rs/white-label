//! The F&B order: the kitchen fulfilment that follows a sales order
//! (06 F&B §7.1 *Order*, the base for BL-FNB-01 to BL-FNB-03).
//!
//! The order data is the sales order's (`SAL.ORD.01`, wl-sales); this record
//! is the fulfilment overlay of the kitchen: where the order is on its way
//! from `placed` to `completed`, where it goes, its delivery, and what became
//! of it when the delivery failed. Its status moves only through the machine
//! in [`crate::order_machine`].
//!
//! The record is made when the sales order is placed (a direct order waiting
//! for acceptance is `placed`) or confirmed (an aggregator order starts in
//! `accepted`): see [`crate::placing`].

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::sales::SalesOrderId;
use wl_kernel::{PartyId, Subject};

/// The subject type of an F&B order: the source of its delivery, so the
/// delivery events name it, and the key of the perishable policy and the
/// delivery promise of the 06 client (`WRK.RUL.16`).
pub const SUBJECT_TYPE: &str = "fnb.order";

/// Where an F&B order is (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FnbOrderStatus {
    /// Direct order received and paid (or cash on delivery), waiting for the
    /// outlet to accept.
    #[default]
    Placed,
    /// Confirmed; aggregator orders start here.
    Accepted,
    /// At least one kitchen ticket started.
    Preparing,
    /// All tickets bumped; waiting for pickup or rider.
    Ready,
    /// Out for delivery with an own rider.
    Dispatched,
    /// Own-rider delivery failed; the food is coming back or was discarded;
    /// the refund decision is pending.
    DeliveryFailed,
    /// Picked up or delivered (terminal).
    Completed,
    /// Not accepted by the outlet; payment refunded (terminal).
    Rejected,
    /// Cancelled before acceptance, after acceptance, or after a failed
    /// delivery; reason and responsible party recorded (terminal).
    Cancelled,
}

impl FnbOrderStatus {
    /// Whether the order is finished.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            FnbOrderStatus::Completed | FnbOrderStatus::Rejected | FnbOrderStatus::Cancelled
        )
    }
}

/// How the order reaches the customer.
pub mod kind {
    /// An own rider takes it to the customer's address.
    pub const DELIVERY: &str = "delivery";
    /// The customer or the aggregator's courier collects it at the outlet.
    pub const PICKUP: &str = "pickup";
}

/// How the customer pays (the stored names of wl-sales' payment modes).
pub mod payment {
    /// Cash to the rider at the door.
    pub const CASH_ON_DELIVERY: &str = "cash_on_delivery";
    /// Paid online before the order was placed.
    pub const ONLINE: &str = "online";
}

/// Why a delivery failed, as the order says it (§7.1 *Failure reasons from
/// wl-delivery*).
pub mod reason {
    /// Not home, or access denied.
    pub const CUSTOMER_UNREACHABLE: &str = "customer_unreachable";
    /// The address was wrong.
    pub const WRONG_ADDRESS: &str = "wrong_address";
    /// Refused, for a cause other than the two below.
    pub const REFUSED: &str = "refused";
    /// Refused because the order was late (outlet side).
    pub const REFUSED_LATE: &str = "refused_late";
    /// Refused because the order had the wrong item (outlet side).
    pub const REFUSED_WRONG_ITEM: &str = "refused_wrong_item";
    /// The customer refused to pay or had no cash.
    pub const NO_CASH: &str = "no_cash";
    /// The rider could not go on (vehicle, lost, late beyond the promise).
    pub const RIDER_ISSUE: &str = "rider_issue";
    /// The run was aborted and the stop could not be handed over.
    pub const RUN_ABORTED: &str = "run_aborted";
}

/// Whose side a failure is on (`FNB.ACL.17`).
pub mod side {
    /// The customer's: unreachable, wrong address, refused (other), no cash.
    pub const CUSTOMER: &str = "customer";
    /// The outlet's: rider issue, run aborted, refused late or wrong item.
    pub const OUTLET: &str = "outlet";
}

/// Who is responsible for a failed delivery.
pub mod party {
    /// The customer.
    pub const CUSTOMER: &str = "customer";
    /// The rider.
    pub const RIDER: &str = "rider";
    /// The business.
    pub const BUSINESS: &str = "business";
    /// The aggregator (a cancellation only: the aggregator owes
    /// compensation).
    pub const AGGREGATOR: &str = "aggregator";
    /// Every party, as the kitchen manager may choose.
    pub const ALL: [&str; 3] = [CUSTOMER, RIDER, BUSINESS];
    /// The parties of a cancellation (`FNB.ACL`: business, aggregator or
    /// customer; the kitchen is the business).
    pub const CANCEL: [&str; 3] = [BUSINESS, AGGREGATOR, CUSTOMER];
}

/// Why an order is cancelled, as the cashier or the manager picks it
/// (BL-FNB-04).
pub mod cancel_reason {
    /// The order was entered by mistake: the responsible party is the
    /// business and cannot be changed. Orders are never voided.
    pub const ENTERED_IN_ERROR: &str = "entered_in_error";
    /// The customer asked.
    pub const CUSTOMER_REQUEST: &str = "customer_request";
    /// An item ran out.
    pub const ITEM_UNAVAILABLE: &str = "item_unavailable";
    /// The kitchen could not make it.
    pub const KITCHEN_ISSUE: &str = "kitchen_issue";
    /// The aggregator cancelled it, or its rider never came (`FNB.RUL.16`).
    pub const AGGREGATOR_CANCELLED: &str = "aggregator_cancelled";
    /// Any other reason, in the note.
    pub const OTHER: &str = "other";
    /// What a person may pick on the order page.
    pub const CHOICES: [&str; 5] = [
        ENTERED_IN_ERROR,
        CUSTOMER_REQUEST,
        ITEM_UNAVAILABLE,
        KITCHEN_ISSUE,
        OTHER,
    ];
}

/// The refund decision of a closed failed delivery (`FNB.ACL.17`).
pub mod decision {
    /// Everything paid is refunded.
    pub const FULL_REFUND: &str = "full_refund";
    /// Nothing is refunded.
    pub const NO_REFUND: &str = "no_refund";
    /// A manager approved a refund of an amount.
    pub const MANAGER_APPROVED: &str = "manager_approved";
    /// Nothing was collected on cash on delivery, so there is nothing to
    /// refund.
    pub const NOT_APPLICABLE: &str = "not_applicable";
}

/// A failure reason of wl-delivery mapped to the order's (§7.1 table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapped {
    /// The order's failure reason, one of [`reason`].
    pub reason: &'static str,
    /// The side, one of [`side`].
    pub side: &'static str,
    /// The proposed responsible party, one of [`party`].
    pub party: &'static str,
}

/// The order's failure reason for the wl-delivery failure reason `code`
/// (§7.1 *Failure reasons from wl-delivery*). 06 adds `rider_issue`,
/// `refused_late` and `refused_wrong_item` to wl-delivery's list
/// ([`crate::seed::FAILURE_REASONS`]): the rider picks the customer's stated
/// cause when recording *refused*.
///
/// A code the table does not name (`reversed`, `cancelled_by_client`, or one
/// a client added) is the outlet's: the outlet cannot blame the customer for
/// a reason it did not classify, so it maps to *rider issue*.
pub fn map_reason(code: &str) -> Mapped {
    match code.trim() {
        "not_home" | "access_denied" => Mapped {
            reason: reason::CUSTOMER_UNREACHABLE,
            side: side::CUSTOMER,
            party: party::CUSTOMER,
        },
        "wrong_address" => Mapped {
            reason: reason::WRONG_ADDRESS,
            side: side::CUSTOMER,
            party: party::CUSTOMER,
        },
        "refused" => Mapped {
            reason: reason::REFUSED,
            side: side::CUSTOMER,
            party: party::CUSTOMER,
        },
        "refused_late" => Mapped {
            reason: reason::REFUSED_LATE,
            side: side::OUTLET,
            party: party::BUSINESS,
        },
        "refused_wrong_item" => Mapped {
            reason: reason::REFUSED_WRONG_ITEM,
            side: side::OUTLET,
            party: party::BUSINESS,
        },
        "refused_to_pay" | "no_cash" => Mapped {
            reason: reason::NO_CASH,
            side: side::CUSTOMER,
            party: party::CUSTOMER,
        },
        "run_aborted" => Mapped {
            reason: reason::RUN_ABORTED,
            side: side::OUTLET,
            party: party::BUSINESS,
        },
        _ => Mapped {
            reason: reason::RIDER_ISSUE,
            side: side::OUTLET,
            party: party::RIDER,
        },
    }
}

/// An F&B order.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "fnb_orders",
    state_machine,
    unique(sales_order_id),
    index(status),
    index(party_id),
    index(delivery_id)
)]
pub struct FnbOrder {
    /// The primary key.
    pub id: i64,
    /// The number of the sales order, e.g. `SO-2026-0001`.
    pub number: String,
    /// The sales order (`wl-sales`).
    pub sales_order_id: i64,
    /// The customer: the party found by phone number (`FNB.RUL.14`).
    pub party_id: i64,
    /// The brand the order was placed with.
    pub brand_id: Option<i64>,
    /// The outlet that cooks it.
    pub outlet_id: Option<i64>,
    /// The channel it came through.
    pub channel: String,
    /// `delivery` or `pickup`, one of [`kind`].
    pub kind: String,
    /// `cash_on_delivery`, `online`, ...: the sales order's payment mode.
    pub payment_mode: String,
    /// What the customer paid before the order was placed, in fils.
    pub paid_amount: i64,
    /// The total with VAT, in fils.
    pub total: i64,
    /// Who receives it.
    pub recipient_name: String,
    /// Their phone.
    pub recipient_phone: String,
    /// The address as text.
    pub address: String,
    /// The area.
    pub area: String,
    /// The wl-delivery delivery of an order that goes out.
    pub delivery_id: Option<i64>,
    /// The run the delivery is on.
    pub run_id: Option<i64>,
    /// The rider driving that run (a user).
    pub rider_user_id: Option<i64>,
    /// When the order was placed.
    pub placed_at: Option<DateTime>,
    /// When it was accepted.
    pub accepted_at: Option<DateTime>,
    /// When it was completed.
    pub completed_at: Option<DateTime>,
    /// When all its tickets were bumped and it became `ready` (the clock of
    /// `FNB.RUL.16`).
    pub ready_at: Option<DateTime>,
    /// When the kitchen manager was told to chase the aggregator's rider.
    pub pickup_alert_at: Option<DateTime>,
    /// When the task to record what happened was opened.
    pub pickup_task_at: Option<DateTime>,
    /// The task opened for the kitchen manager.
    pub pickup_task_id: Option<i64>,
    /// Whether the kitchen's consumption of the recipe was posted.
    pub consumption_posted: bool,
    /// The order's failure reason, one of [`reason`] (BL-FNB-01).
    pub failure_reason: Option<String>,
    /// The wl-delivery failure reason, kept on the order too.
    pub delivery_reason: Option<String>,
    /// The side of the failure, one of [`side`].
    pub failure_side: Option<String>,
    /// The attempt that failed.
    pub failure_attempt: i64,
    /// When the delivery failed.
    pub failed_at: Option<DateTime>,
    /// The party responsible, one of [`party`]: proposed from the reason, the
    /// kitchen manager may change it with a note.
    pub responsible_party: Option<String>,
    /// The kitchen manager's note for changing it.
    pub responsible_note: Option<String>,
    /// The user who changed it.
    pub responsible_changed_by: Option<i64>,
    /// The refund decision, one of [`decision`] (BL-FNB-02).
    pub refund_decision: Option<String>,
    /// What was refunded at the close, in fils.
    pub refund_amount: i64,
    /// The wl-payments refund asked at the close.
    pub refund_id: Option<i64>,
    /// The reference of the wastage record the delivery's `discard` made.
    pub wastage_ref: Option<String>,
    /// Whether the food was posted as wastage (by the discard, or by the
    /// scrap of the cancellation): never posted twice (`FNB.ACL.16`).
    pub wastage_posted: bool,
    /// When the failed delivery was closed.
    pub closed_at: Option<DateTime>,
    /// Why it was closed, as the cancel reason.
    pub close_reason: Option<String>,
    /// The party responsible for a cancellation, one of [`party::CANCEL`]
    /// (BL-FNB-04). Kept on the order while the cancellation waits for a
    /// manager.
    pub cancel_party: Option<String>,
    /// The reason of a cancellation, one of [`cancel_reason`].
    pub cancel_reason_code: Option<String>,
    /// The user who asked for the cancellation.
    pub cancel_requested_by: Option<i64>,
    /// The approval request of a cancellation that waits for a manager.
    pub cancel_request_id: Option<i64>,
    /// The user whose cancellation was made.
    pub cancelled_by: Option<i64>,
    /// Whether the customer cancelled it from the order page.
    pub cancelled_by_customer: bool,
    /// When it was cancelled.
    pub cancelled_at: Option<DateTime>,
    /// An aggregator order cancelled with the party *aggregator*: the payout
    /// reconciliation expects the aggregator to pay for the food.
    pub compensation_expected: bool,
    /// The lifecycle state; changed only by a transition.
    pub status: FnbOrderStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl FnbOrder {
    /// The order as a [`Subject`]: `fnb.order:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The sales order.
    pub fn sales_order(&self) -> SalesOrderId {
        SalesOrderId(self.sales_order_id)
    }

    /// The customer.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// Whether the cash is collected at the door.
    pub fn is_cash_on_delivery(&self) -> bool {
        self.payment_mode == payment::CASH_ON_DELIVERY
    }

    /// Whether the order goes out with an own rider.
    pub fn is_delivery(&self) -> bool {
        self.kind == kind::DELIVERY
    }

    /// The order made for the sales order `id`, if any.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_sales_order(db: &Db, id: i64) -> Result<Option<FnbOrder>> {
        FnbOrder::query()
            .where_(FnbOrder::SALES_ORDER_ID.eq(id))
            .first(db)
            .await
    }

    /// The order the delivery `delivery` is for, when its source is one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_source(db: &Db, source: &Subject) -> Result<Option<FnbOrder>> {
        if source.subject_type != SUBJECT_TYPE {
            return Ok(None);
        }
        FnbOrder::find(db, source.subject_id).await
    }

    /// The orders, newest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn newest(db: &Db, limit: u64) -> Result<Vec<FnbOrder>> {
        FnbOrder::query()
            .order_by_desc(FnbOrder::ID)
            .limit(limit)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reasons_of_wl_delivery_map_to_the_orders_with_their_side_and_party() {
        let table = [
            ("not_home", reason::CUSTOMER_UNREACHABLE, side::CUSTOMER, party::CUSTOMER),
            ("access_denied", reason::CUSTOMER_UNREACHABLE, side::CUSTOMER, party::CUSTOMER),
            ("wrong_address", reason::WRONG_ADDRESS, side::CUSTOMER, party::CUSTOMER),
            ("refused", reason::REFUSED, side::CUSTOMER, party::CUSTOMER),
            ("refused_to_pay", reason::NO_CASH, side::CUSTOMER, party::CUSTOMER),
            ("no_cash", reason::NO_CASH, side::CUSTOMER, party::CUSTOMER),
            ("refused_late", reason::REFUSED_LATE, side::OUTLET, party::BUSINESS),
            ("refused_wrong_item", reason::REFUSED_WRONG_ITEM, side::OUTLET, party::BUSINESS),
            ("rider_issue", reason::RIDER_ISSUE, side::OUTLET, party::RIDER),
            ("not_attempted", reason::RIDER_ISSUE, side::OUTLET, party::RIDER),
            ("run_aborted", reason::RUN_ABORTED, side::OUTLET, party::BUSINESS),
        ];
        for (code, expected, expected_side, expected_party) in table {
            let mapped = map_reason(code);
            assert_eq!(mapped.reason, expected, "{code}");
            assert_eq!(mapped.side, expected_side, "{code}");
            assert_eq!(mapped.party, expected_party, "{code}");
        }
    }

    #[test]
    fn a_reason_nobody_classified_is_the_outlets() {
        assert_eq!(map_reason("reversed").side, side::OUTLET);
        assert_eq!(map_reason("something_new").reason, reason::RIDER_ISSUE);
    }
}
