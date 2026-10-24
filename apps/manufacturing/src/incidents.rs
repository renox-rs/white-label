//! The NCRs on incoming lots as the supplier scorecard's quality incidents
//! (07 Manufacturing §7.1 *NCR* cause paragraph, wl-inventory §8
//! `SupplierIncidentSource`, `STK.SUP.01`; BL-MFG-15, #396).
//!
//! [`NcrIncidents`] answers, for a supplier and a range of days, one
//! incident per NCR whose cause is *supplier* and that is not `cancelled`,
//! dated the day it was raised (Asia/Dubai), not the day it was dispositioned.
//! It reads the NCR as it is now, so a later change of cause or a
//! cancellation drops the incident out of the next answer (the live view and
//! the next snapshot); a monthly snapshot already stored is wl-inventory's
//! and is never edited. An NCR on a lot with no supplier (own production) has
//! no supplier and is never reported; a reopened NCR stays one incident (it is
//! one row). Cause *internal process* or *unknown* is not reported.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::inventory::{MatchFuture, SupplierIncident, SupplierIncidentSource};

use crate::ncr::{Cause, Ncr, NcrStatus};

/// The scorecard's source of quality incidents.
pub struct NcrIncidents;

impl SupplierIncidentSource for NcrIncidents {
    fn incidents<'a>(
        &'a self,
        db: &'a Db,
        supplier: PartyId,
        from: NaiveDate,
        to: NaiveDate,
    ) -> MatchFuture<'a, Vec<SupplierIncident>> {
        Box::pin(async move {
            Ok(Ncr::query()
                .where_(Ncr::SUPPLIER_PARTY_ID.eq(Some(supplier.0)))
                .where_(Ncr::CAUSE.eq(Cause::Supplier))
                .where_(Ncr::STATUS.ne(NcrStatus::Cancelled))
                .where_(Ncr::RAISED_ON.gte(from))
                .where_(Ncr::RAISED_ON.lte(to))
                .order_by(Ncr::RAISED_ON)
                .order_by(Ncr::ID)
                .get(db)
                .await?
                .into_iter()
                .map(|ncr| SupplierIncident {
                    reference: ncr.subject(),
                    supplier,
                    on: ncr.raised_on,
                })
                .collect())
        })
    }
}
