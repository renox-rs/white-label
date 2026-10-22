//! The visa case actions of the back office (wl-hr §4 *Visa case*, issues #193
//! and #195).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /hr/visa-cases/{id}/transitions/{name}` | `wl_hr.visa_transitions.*` | `visas.manage` (`cancel`) |
//! | `POST /hr/visa-cases/{id}/reject` | `wl_hr.visa_reject` | `visas.manage`; the authority's reference and the reason |
//! | `POST /hr/visa-cases/{id}/cancellation-not-needed` | `wl_hr.visa_cancellation_not_needed` | `visas.manage`; a reason |
//!
//! The actions answer with a redirect back to the page they were sent from.

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::permissions;
use crate::visa::{self, VisaCase};

/// The routes of the visa case actions.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .group(
                    "/hr/visa-cases",
                    "wl_hr.visa_transitions.",
                    wl_kernel::machines::routes::<VisaCase>(),
                )
                .post("/hr/visa-cases/{id}/reject", reject)
                .name("wl_hr.visa_reject")
                .post(
                    "/hr/visa-cases/{id}/cancellation-not-needed",
                    cancellation_not_needed,
                )
                .name("wl_hr.visa_cancellation_not_needed")
                .require_permission(permissions::VISAS_MANAGE),
        )
        .require_auth()
}

/// The form of `reject`.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct RejectForm {
    /// The authority's reference of the refusal.
    #[validate(max = 100)]
    #[serde(default)]
    pub reference: String,
    /// Why the authority refused.
    #[validate(max = 1000)]
    #[serde(default)]
    pub reason: String,
}

/// `POST /hr/visa-cases/{id}/reject`.
async fn reject(
    State(state): State<AppState>,
    user: AuthUser,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<RejectForm>,
) -> Result<Response> {
    visa::reject(&state, &user, id, &form.reference, &form.reason).await?;
    Ok(back.into_response())
}

/// The form of *cancellation not needed*.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct NotNeededForm {
    /// Why no cancellation case is needed.
    #[validate(max = 1000)]
    #[serde(default)]
    pub reason: String,
}

/// `POST /hr/visa-cases/{id}/cancellation-not-needed`.
async fn cancellation_not_needed(
    State(state): State<AppState>,
    user: AuthUser,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<NotNeededForm>,
) -> Result<Response> {
    visa::record_cancellation_not_needed(&state, &user, id, &form.reason).await?;
    Ok(back.into_response())
}
