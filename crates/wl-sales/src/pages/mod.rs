//! The back-office pages of the sales module, each behind its permission.

pub mod acquisition;
pub mod campaigns;
pub mod lead_sources;
pub mod leads;
pub mod report;

use renox::prelude::*;

/// The module's routes.
pub fn routes() -> Routes {
    Routes::new()
        .merge(acquisition::routes())
        .merge(campaigns::routes())
        .merge(lead_sources::routes())
        .merge(leads::routes())
        .merge(report::routes())
}
