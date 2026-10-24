//! The 02 Construction client portal after the final account (02 §3.3
//! `POR.ACL.01`; BL-CON-19, issue #298): the host hands construction's
//! [`ProjectClosed`] to wl-portal.
//!
//! When a project's final account is closed (`close` or
//! `settle_final_account`), the client's portal grants in the
//! [`CLIENT_AUDIENCE`] run `end` on the day it closed, but only when the
//! client has no other project that is neither `closed` nor `cancelled`;
//! wl-portal's daily timer closes them 90 days later. While another project
//! runs the grant stays `active`, and the closed project is read-only and then
//! hidden through [`construction::client_portal::access_on`].
//!
//! The 02 audience and its roles (`POR.SLF.01`) are Later; without an audience
//! row wl-portal counts its default 90 days.

use construction::ProjectClosed;
use construction::client_portal::{CLIENT_AUDIENCE, has_open_project};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_portal::PortalAccess;

/// A project of `client` closed on `on`: `end` on the client's grants unless
/// another project of theirs still runs. Returns how many grants turned
/// read-only.
///
/// # Errors
///
/// The database's errors.
pub async fn project_closed(
    state: &AppState,
    client: PartyId,
    on: NaiveDate,
    reason: &str,
) -> Result<usize> {
    if has_open_project(&state.db, client).await? {
        return Ok(0);
    }
    let ended = PortalAccess::contract_ended(state, client, CLIENT_AUDIENCE, on, reason).await?;
    Ok(ended.len())
}

/// The module: the contract-end listener. The host mounts it for a
/// Construction client with the portal on.
pub struct ConstructionPortal;

impl Module for ConstructionPortal {
    fn name(&self) -> &'static str {
        "construction_portal"
    }

    fn register(&self, app: &mut Registry) {
        app.listen(|event: ProjectClosed, state: AppState| async move {
            project_closed(
                &state,
                event.client,
                event.closed_on,
                "final account closed",
            )
            .await
            .map(|_| ())
        });
    }
}
