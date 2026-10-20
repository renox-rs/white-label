//! What a client user sees of its engagements in the portal (`ACC.ACL.14`,
//! `PTY.ACL.02`): each engagement with its status only, **On hold** for a
//! suspended one and **Ended** for an ended one, whatever the reason. There
//! is no reason line, and nothing about a block, a count or a badge.
//!
//! The host registers these rows as the wl-portal section `engagements`
//! (`host/src/app/accounting_portal.rs`); the app does not depend on the
//! portal crate.

use renox::prelude::*;
use serde::Serialize;
use wl_kernel::PartyId;

use crate::engagement::{Engagement, EngagementStatus};
use crate::texts::english;

/// The key of the portal section.
pub const SECTION: &str = "engagements";

/// One engagement as the portal shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PortalRow {
    /// `ENG-0007 · Bookkeeping and VAT`.
    pub label: String,
    /// The status a client may see: "Onboarding", "Active", "On hold",
    /// "Ended" or "Declined".
    pub status: String,
}

/// The section's title.
pub fn title() -> String {
    english("accounting.portal.title", &[])
}

/// The status of `status` as the portal shows it.
pub fn status_text(status: EngagementStatus) -> String {
    english(
        &format!("accounting.portal.status.{}", status.as_str()),
        &[],
    )
}

/// The engagements of `party` (the client company), oldest first.
///
/// # Errors
///
/// The database's errors.
pub async fn rows(state: &AppState, party: PartyId) -> Result<Vec<PortalRow>> {
    Ok(Engagement::of_client(&state.db, party)
        .await?
        .iter()
        .map(|engagement| PortalRow {
            label: format!("{} · {}", engagement.number(), engagement.title),
            status: status_text(engagement.status),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_suspension_reads_on_hold_and_an_end_reads_ended() {
        assert_eq!(status_text(EngagementStatus::Suspended), "On hold");
        assert_eq!(status_text(EngagementStatus::Ended), "Ended");
        assert_eq!(status_text(EngagementStatus::Active), "Active");
    }
}
