//! The lead transitions (wl-sales §4.1, issue #139): `contact`, `qualify`,
//! `lose` and `reopen` post to the kernel's machine route, each also needing
//! its own permission (`leads.manage`). The reason field carries the contact
//! note of `contact` and the lost reason of `lose`. The lead pages themselves
//! come with the lead issues of the roadmap.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /leads/{id}/transitions/{name}` | `wl_sales.lead_transitions.*` | `leads.view` and the transition's permission |

use renox::prelude::*;

use crate::lead::Lead;
use crate::permissions;

/// The routes of the lead transitions.
pub fn routes() -> Routes {
    Routes::new()
        .group(
            "/leads",
            "wl_sales.lead_transitions.",
            wl_kernel::machines::routes::<Lead>(),
        )
        .require_permission(permissions::LEADS_VIEW)
}
