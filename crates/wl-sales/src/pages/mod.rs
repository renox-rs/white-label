//! The back-office pages of the sales module, each behind its permission.

pub mod campaigns;
pub mod lead_sources;

use renox::prelude::*;

/// The module's routes.
pub fn routes() -> Routes {
    Routes::new()
        .merge(campaigns::routes())
        .merge(lead_sources::routes())
}
