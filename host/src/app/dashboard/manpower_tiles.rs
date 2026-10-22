//! What the home page of a Manpower deployment says to the client's own
//! users (BL-MPW-28): how many timesheets wait for the client approver, with
//! the link to the list in the portal. The tiles of the agency's staff come
//! with BL-MPW-32.

use renox::prelude::*;

use super::app_of;

/// The portal page of the timesheets.
pub(super) const TIMESHEETS_URL: &str = "/portal/manpower/timesheets";

/// How many timesheets wait for `user` as a client approver; 0 for anyone who
/// is not one, and on a deployment of another vertical.
pub(super) async fn timesheets_waiting(state: &AppState, user: &AuthUser) -> Result<usize> {
    if app_of(state) != "manpower" {
        return Ok(0);
    }
    Ok(::manpower::portal::waiting(state, user.id).await?.len())
}
