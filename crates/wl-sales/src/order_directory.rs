//! The kernel contracts of the 07 vertical that the sales order answers
//! (wl-sales §6): [`OrderLines`] for a work order serving a sale line, and
//! [`CustomerPoCheck`] for the PO a buyer uploads on the portal.
//!
//! Decisions:
//! - A line is *cancelled* when the order ended (`cancelled`, `rejected`) or
//!   all its quantity was cancelled; its open quantity is what the sale line
//!   still has open ([`SaleLine::open_milli`]). The owner of the order is the
//!   user who entered it.
//! - The PO check is [`crate::po_number::buyer_refusal`]: nothing when the
//!   setting *Unique customer PO number per customer* is off, and only the
//!   number and status of the existing order otherwise.
//! - Not provided here (no data to answer from yet, the kernel's default
//!   stays): `PromisedLines` (a sale line has no promised day),
//!   `DeliveryDestination` (an order has no delivery address) and, with it,
//!   the vertical's `OrderLineRules` at `submit` and `amend` (they need to know
//!   whether the delivery is domestic).

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::production::{OrderLineInfo, OrderLines, ProductionFuture};
use wl_kernel::sales::{CustomerPoCheck, PoHolder, SalesFuture, SalesOrderId};

use crate::po_number::buyer_refusal;
use crate::sale_line::SaleLine;
use crate::sales_order::{SalesOrder, SalesOrderStatus};

/// The kernel's [`OrderLines`].
pub(crate) struct Lines;

impl OrderLines for Lines {
    fn line<'a>(
        &'a self,
        state: &'a AppState,
        order: SalesOrderId,
        line: i64,
    ) -> ProductionFuture<'a, Option<OrderLineInfo>> {
        Box::pin(async move {
            let Some(found) = SalesOrder::find(&state.db, order.0).await? else {
                return Ok(None);
            };
            let lines = SaleLine::of(&state.db, &found.subject()).await?;
            let Some(sale) = lines.iter().find(|candidate| candidate.id == line) else {
                return Ok(None);
            };
            let ended = matches!(
                found.status,
                SalesOrderStatus::Cancelled | SalesOrderStatus::Rejected
            );
            Ok(Some(OrderLineInfo {
                order,
                line,
                number: Some(found.number.clone()),
                owner: found.created_by,
                open_milli: sale.open_milli(),
                cancelled: ended || sale.cancelled_milli >= sale.quantity_milli,
                customer: Some(found.party()),
            }))
        })
    }
}

/// The kernel's [`CustomerPoCheck`].
pub(crate) struct PoCheck;

impl CustomerPoCheck for PoCheck {
    fn holder<'a>(
        &'a self,
        state: &'a AppState,
        customer: PartyId,
        po: &'a str,
        call_off: &'a str,
    ) -> SalesFuture<'a, Option<PoHolder>> {
        Box::pin(async move {
            Ok(buyer_refusal(state, customer.0, po, call_off)
                .await?
                .map(|refusal| PoHolder {
                    number: refusal.number,
                    status: refusal.status,
                }))
        })
    }
}
