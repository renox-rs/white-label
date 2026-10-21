//! The back-office pages of the sales module, each behind its permission.

pub mod acquisition;
pub mod campaigns;
pub mod counter_sales;
pub mod enquiry;
pub mod lead_sources;
pub mod leads;
pub mod loyalty;
pub mod report;
pub mod sales_orders;
pub mod sales_returns;

use renox::prelude::*;

/// The module's routes.
pub fn routes() -> Routes {
    Routes::new()
        .merge(acquisition::routes())
        .merge(campaigns::routes())
        .merge(counter_sales::routes())
        .merge(enquiry::routes())
        .merge(lead_sources::routes())
        .merge(leads::routes())
        .merge(loyalty::routes())
        .merge(report::routes())
        .merge(sales_orders::routes())
        .merge(sales_returns::routes())
}
