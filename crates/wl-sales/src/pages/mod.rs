//! The back-office pages of the sales module, each behind its permission.

pub mod acquisition;
pub mod adjustments;
pub mod campaigns;
pub mod counter_sales;
pub mod enquiry;
pub mod lead_sources;
pub mod leads;
pub mod liability;
pub mod loyalty;
pub mod redeem;
pub mod report;
pub mod sales_orders;
pub mod sales_returns;
pub mod segments;

use renox::prelude::*;

/// The module's routes.
pub fn routes() -> Routes {
    Routes::new()
        .merge(acquisition::routes())
        .merge(adjustments::routes())
        .merge(campaigns::routes())
        .merge(counter_sales::routes())
        .merge(enquiry::routes())
        .merge(lead_sources::routes())
        .merge(leads::routes())
        .merge(liability::routes())
        .merge(loyalty::routes())
        .merge(redeem::routes())
        .merge(report::routes())
        .merge(sales_orders::routes())
        .merge(sales_returns::routes())
        .merge(segments::routes())
}
