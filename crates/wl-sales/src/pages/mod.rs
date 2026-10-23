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
pub mod membership;
pub mod redeem;
pub mod report;
pub mod sales_orders;
pub mod sales_returns;
pub mod segments;
pub mod till_sessions;

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
        .merge(membership::routes())
        .merge(redeem::routes())
        .merge(report::routes())
        .merge(sales_orders::routes())
        .merge(sales_returns::routes())
        .merge(segments::routes())
        .merge(till_sessions::routes())
}

/// The name of the party `id` in the page's language, else `#id`: a member
/// or a customer is shown by name, never by the number of the party.
pub(crate) async fn party_name(state: &AppState, lang: &Lang, id: i64) -> String {
    wl_kernel::parties::directory(state)
        .display_name(&state.db, wl_kernel::PartyId(id), &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{id}"))
}
