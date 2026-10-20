//! What a client user sees of its engagements in the portal (`ACC.ACL.14`,
//! `PTY.ACL.02`): each engagement with its status only, **On hold** for a
//! suspended one and **Ended** for an ended one, whatever the reason. There
//! is no reason line, and nothing about a block, a count or a badge.
//!
//! The host registers these rows as the wl-portal section `engagements`
//! (`host/src/app/accounting_portal.rs`); the app does not depend on the
//! portal crate.
//!
//! After an engagement ends the client keeps read-only access to its
//! returns for [`CLOSES_AFTER_DAYS`] days, then the portal closes
//! (`POR.ACL.01`): [`is_open`] says which, and the sections show nothing
//! once it is closed. A return the firm cancelled because the client files
//! it itself (or the engagement ended) shows the deadline as the client's:
//! "You file it by 2026-11-28" ([`returns`]).

use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::PartyId;

use crate::engagement::{Engagement, EngagementStatus};
use crate::kpi::{KpiPack, KpiStatus};
use crate::reasons;
use crate::tax_return::{Return, ReturnStatus};
use crate::texts::english;

/// The key of the portal section.
pub const SECTION: &str = "engagements";
/// The key of the portal section of the returns.
pub const RETURNS_SECTION: &str = "returns";
/// The key of the portal section of the KPI packs.
pub const KPI_SECTION: &str = "kpi_packs";
/// Days after an engagement ends the client's read-only access closes
/// (`POR.ACL.01`).
pub const CLOSES_AFTER_DAYS: i64 = 90;

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

/// Whether the portal is open to the client `party` on `on`: it has no
/// engagement yet, or one that has not ended, or one that ended less than
/// [`CLOSES_AFTER_DAYS`] days ago (`POR.ACL.01`). Read-only once an
/// engagement has ended; closed after the days.
///
/// # Errors
///
/// The database's errors.
pub async fn is_open(state: &AppState, party: PartyId, on: NaiveDate) -> Result<bool> {
    let engagements = Engagement::of_client(&state.db, party).await?;
    if engagements.is_empty() {
        return Ok(true);
    }
    Ok(engagements.iter().any(|engagement| {
        engagement.status != EngagementStatus::Ended
            || engagement
                .ended_on
                .is_none_or(|ended| on <= ended + Duration::days(CLOSES_AFTER_DAYS))
    }))
}

/// Whether the portal is read-only for `party` on `on`: all its engagements
/// ended.
///
/// # Errors
///
/// The database's errors.
pub async fn is_read_only(state: &AppState, party: PartyId) -> Result<bool> {
    let engagements = Engagement::of_client(&state.db, party).await?;
    Ok(!engagements.is_empty()
        && engagements
            .iter()
            .all(|engagement| engagement.status == EngagementStatus::Ended))
}

/// The engagements of `party` (the client company), oldest first; none once
/// the portal has closed.
///
/// # Errors
///
/// The database's errors.
pub async fn rows(state: &AppState, party: PartyId) -> Result<Vec<PortalRow>> {
    if !is_open(state, party, wl_kernel::documents::today()).await? {
        return Ok(Vec::new());
    }
    Ok(Engagement::of_client(&state.db, party)
        .await?
        .iter()
        .map(|engagement| PortalRow {
            label: format!("{} · {}", engagement.number(), engagement.title),
            status: status_text(engagement.status),
        })
        .collect())
}

/// The section's title for the returns.
pub fn returns_title() -> String {
    english("accounting.portal.returns_title", &[])
}

/// Whether the deadline of `ret` is the client's: the firm cancelled it
/// because the client files it itself, or the engagement ended.
pub fn client_files(ret: &Return) -> bool {
    ret.status == ReturnStatus::Cancelled
        && matches!(
            ret.cancel_reason.as_deref(),
            Some(reasons::CLIENT_FILES_ITSELF | reasons::ENGAGEMENT_ENDED)
        )
}

/// One return as the portal shows it.
fn return_row(ret: &Return) -> PortalRow {
    let kind = english(
        &format!("accounting.returns.type.{}", ret.return_type.as_str()),
        &[],
    );
    let status = if client_files(ret) {
        english("accounting.portal.you_file", &[("due", &ret.due_date)])
    } else {
        english(
            &format!("accounting.portal.return_status.{}", ret.status.as_str()),
            &[],
        )
    };
    PortalRow {
        label: format!(
            "{} · {} {} · {}",
            ret.number(),
            kind,
            ret.period_end,
            english("accounting.portal.due", &[("due", &ret.due_date)])
        ),
        status,
    }
}

/// The returns of `party` the client may see (`ACC.ACL.01`): those in
/// progress, the filed ones, and those whose deadline is now the client's;
/// not the superseded versions or the ones cancelled as not required. None
/// once the portal has closed.
///
/// # Errors
///
/// The database's errors.
pub async fn returns(state: &AppState, party: PartyId) -> Result<Vec<PortalRow>> {
    if !is_open(state, party, wl_kernel::documents::today()).await? {
        return Ok(Vec::new());
    }
    Ok(Return::of_client(&state.db, party)
        .await?
        .iter()
        .filter(|ret| match ret.status {
            ReturnStatus::Superseded => false,
            ReturnStatus::Cancelled => client_files(ret),
            _ => true,
        })
        .map(return_row)
        .collect())
}

/// The section's title for the KPI packs.
pub fn kpi_title() -> String {
    english("accounting.portal.kpi_title", &[])
}

/// The KPI packs of `party` the client may see (`ACC.ACL.01`): the latest
/// published version of each period, *being revised* while a newer draft
/// exists; never a draft, a superseded or a withdrawn pack. None once the
/// portal has closed.
///
/// # Errors
///
/// The database's errors.
pub async fn kpi_packs(state: &AppState, party: PartyId) -> Result<Vec<PortalRow>> {
    if !is_open(state, party, wl_kernel::documents::today()).await? {
        return Ok(Vec::new());
    }
    let mut rows: Vec<PortalRow> = Vec::new();
    let mut shown: Vec<NaiveDate> = Vec::new();
    // Newest period and version first: the first published pack of a period
    // is the latest one.
    for pack in KpiPack::of_client(&state.db, party).await? {
        if pack.status != KpiStatus::Published || shown.contains(&pack.period_end) {
            continue;
        }
        shown.push(pack.period_end);
        let status = if pack.is_being_revised(&state.db).await? {
            english("accounting.portal.kpi_revised", &[])
        } else {
            english(
                "accounting.portal.kpi_published",
                &[("date", &pack.published_on.unwrap_or(pack.period_end))],
            )
        };
        rows.push(PortalRow {
            label: format!("{} · {}", pack.number(), pack.period_end.format("%B %Y")),
            status,
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_return_the_client_files_shows_its_deadline_as_the_clients() {
        let mut ret = Return {
            id: 2,
            period_end: NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
            due_date: NaiveDate::from_ymd_opt(2026, 11, 28).unwrap(),
            status: ReturnStatus::Cancelled,
            cancel_reason: Some(reasons::CLIENT_FILES_ITSELF.to_owned()),
            ..Return::default()
        };
        assert!(client_files(&ret));
        let row = return_row(&ret);
        assert_eq!(row.status, "You file it by 2026-11-28");
        assert_eq!(
            row.label,
            "RET-0002 · VAT return 2026-10-31 · due 2026-11-28"
        );
        ret.cancel_reason = Some(reasons::NOT_REQUIRED.to_owned());
        assert!(!client_files(&ret));
    }

    #[test]
    fn a_suspension_reads_on_hold_and_an_end_reads_ended() {
        assert_eq!(status_text(EngagementStatus::Suspended), "On hold");
        assert_eq!(status_text(EngagementStatus::Ended), "Ended");
        assert_eq!(status_text(EngagementStatus::Active), "Active");
    }
}
