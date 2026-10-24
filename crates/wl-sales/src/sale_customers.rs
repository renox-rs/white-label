//! The kernel's `SaleCustomers` (wl-sales §6): who bought, behind a counter
//! sale or a sales order known only as a subject. A recall trace of a
//! vertical (08 `BL-AUT-16`) asks it for the customer of a sale; a counter
//! sale that names no customer is a walk-in sale.

use renox::prelude::*;
use wl_kernel::sales::{
    COUNTER_SALE_SUBJECT_TYPE, SALES_ORDER_SUBJECT_TYPE, SaleCustomer, SaleCustomers, SalesFuture,
};
use wl_kernel::{PartyId, Subject};

use crate::counter_sale::CounterSale;
use crate::sales_order::SalesOrder;

/// `wl-sales`' [`SaleCustomers`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Customers;

impl SaleCustomers for Customers {
    fn customer<'a>(
        &'a self,
        state: &'a AppState,
        sale: &'a Subject,
    ) -> SalesFuture<'a, SaleCustomer> {
        Box::pin(async move {
            Ok(match sale.subject_type.as_str() {
                COUNTER_SALE_SUBJECT_TYPE => {
                    match CounterSale::find(&state.db, sale.subject_id).await? {
                        Some(found) => found
                            .party_id
                            .map_or(SaleCustomer::WalkIn, |id| SaleCustomer::Party(PartyId(id))),
                        None => SaleCustomer::Unknown,
                    }
                }
                SALES_ORDER_SUBJECT_TYPE => {
                    match SalesOrder::find(&state.db, sale.subject_id).await? {
                        Some(found) => SaleCustomer::Party(PartyId(found.party_id)),
                        None => SaleCustomer::Unknown,
                    }
                }
                _ => SaleCustomer::Unknown,
            })
        })
    }
}

/// Registers the kernel's `SaleCustomers`.
pub(crate) fn register(app: &mut Registry) {
    wl_kernel::sales::provide_sale_customers(app, Customers);
}
