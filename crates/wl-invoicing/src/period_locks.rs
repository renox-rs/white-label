//! The kernel's [`PeriodLocks`] for wl-invoicing: a date is in a locked
//! period when the VAT summary of the legal entity's tax period that holds
//! it is `locked` (filed). Other modules, such as the landed cost
//! adjustment of wl-inventory, refuse a posting date in such a period.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::LegalEntity;
use wl_kernel::invoicing::{PeriodLocks, SupplierBillFuture};

use crate::config::InvoicingConfig;
use crate::vat_summary::{VatSummary, VatSummaryStatus};

/// The module's [`PeriodLocks`]; it reads the tax periods of the module's
/// settings.
#[derive(Debug, Clone)]
pub struct VatPeriodLocks {
    config: InvoicingConfig,
}

impl VatPeriodLocks {
    /// The locks under the tax periods of `config`.
    pub fn new(config: InvoicingConfig) -> Self {
        VatPeriodLocks { config }
    }
}

impl PeriodLocks for VatPeriodLocks {
    fn is_locked<'a>(
        &'a self,
        db: &'a Db,
        entity: i64,
        date: NaiveDate,
    ) -> SupplierBillFuture<'a, bool> {
        Box::pin(async move {
            let Some(entity) = LegalEntity::find(db, entity).await? else {
                return Ok(false);
            };
            Ok(VatSummary::containing(db, &self.config, &entity, date)
                .await?
                .is_some_and(|summary| summary.status == VatSummaryStatus::Locked))
        })
    }
}
