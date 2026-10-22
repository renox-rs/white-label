//! What the client's own users see of the manpower business in the portal
//! (09 Manpower §3.3 `MPW.ACL.01`, `MPW.ACL.02`, §4.4; BL-MPW-28): their
//! company's requests, the workers on site and the timesheets, and the
//! timesheets waiting for the client approver.
//!
//! The rows here are the section rows of wl-portal (`GET /portal/{party}/{section}`;
//! the host registers them, `host/src/app/manpower_portal.rs`, because the app
//! does not depend on the portal crate) and the data of the full pages
//! ([`crate::screens::portal_page`]). Nothing here knows a rate, a salary, a
//! document of a worker, a block or a hold reason (`PTY.ACL.02`, `MPW.ACL.02`):
//! a contract on credit hold reads *On hold* and nothing more.
//!
//! Who is a client approver is the portal grant of the user for the company:
//! the portal role `owner` or `approver`
//! ([`CLIENT_APPROVER_PORTAL_ROLES`]); a `viewer` only sees. Which companies a
//! user acts for is asked of the kernel's
//! [`PortalUsers::parties_of`](wl_kernel::portal::PortalUsers::parties_of).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::portal::portal_users;
use wl_kernel::timesheets::{TimesheetSummary, timesheets};

use crate::contract::ClientContract;
use crate::deployment::Deployment;
use crate::deployments::worker_name;
use crate::permissions::CLIENT_APPROVER_PORTAL_ROLES;
use crate::request::ManpowerRequest;
use crate::screens::role_text;
use crate::site::Site;
use crate::texts::english;

/// The key of the portal section of the requests.
pub const REQUESTS_SECTION: &str = "manpower_requests";
/// The key of the portal section of the workers on site.
pub const DEPLOYMENTS_SECTION: &str = "manpower_deployments";
/// The key of the portal section of the timesheets.
pub const TIMESHEETS_SECTION: &str = "manpower_timesheets";

/// A client company the user acts for, and how.
#[derive(Debug, Clone, Serialize)]
pub struct Acting {
    /// The company.
    pub party: PartyId,
    /// Whether the user may act (approve, order) and not only see.
    pub approver: bool,
}

/// One line of a section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PortalRow {
    /// What it is.
    pub label: String,
    /// Where it stands, in words.
    pub status: String,
}

/// The client companies with a manpower contract that `user` acts for today.
///
/// # Errors
///
/// The database's errors.
pub async fn acting(state: &AppState, user: i64) -> Result<Vec<Acting>> {
    let mut out = Vec::new();
    for grant in portal_users(state)
        .parties_of(&state.db, user, today())
        .await?
    {
        if out.iter().any(|a: &Acting| a.party == grant.acting_for) {
            continue;
        }
        let has_contract = ClientContract::query()
            .where_(ClientContract::CLIENT_PARTY_ID.eq(grant.acting_for.0))
            .count(&state.db)
            .await?
            > 0;
        if has_contract {
            out.push(Acting {
                party: grant.acting_for,
                approver: grant.has_any_role(CLIENT_APPROVER_PORTAL_ROLES),
            });
        }
    }
    Ok(out)
}

/// A timesheet of a client's worker, as the portal lists it.
#[derive(Debug, Clone, Serialize)]
pub struct SheetItem {
    /// The timesheet.
    pub id: i64,
    /// The client company.
    pub client: PartyId,
    /// The worker's name.
    pub worker: String,
    /// The role and the site, `Security guard, Marina Tower`.
    pub work: String,
    /// The first day.
    pub from: NaiveDate,
    /// The last day.
    pub to: NaiveDate,
    /// The hours of all categories, in minutes.
    pub minutes: i64,
    /// Where it stands: `submitted`, `escalated`, `approved` or `deemed`.
    pub standing: &'static str,
    /// It is an adjustment of an approved sheet.
    pub adjustment: bool,
    /// It waits for this user's approval.
    pub waiting: bool,
}

/// Where a timesheet stands for the client; none for one not yet submitted
/// (the client sees only what was sent to him).
pub fn standing(sheet: &TimesheetSummary) -> Option<&'static str> {
    match sheet.status.as_str() {
        "submitted" if sheet.escalated => Some("escalated"),
        "submitted" => Some("submitted"),
        "approved" if sheet.deemed => Some("deemed"),
        "approved" | "locked" => Some("approved"),
        _ => None,
    }
}

/// The workers of `client` the client sees: on site now or before.
async fn on_site(state: &AppState, client: PartyId) -> Result<Vec<Deployment>> {
    Ok(Deployment::query()
        .where_(Deployment::CLIENT_PARTY_ID.eq(client.0))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(Deployment::shown_to_client)
        .collect())
}

/// The timesheets of the workers of `acting.party` that were sent to the
/// client, newest period first, the ones waiting for an approver first of all.
///
/// # Errors
///
/// The database's errors.
pub async fn sheets(state: &AppState, acting: &Acting, locale: &str) -> Result<Vec<SheetItem>> {
    let mut out = Vec::new();
    for deployment in on_site(state, acting.party).await? {
        let site = Site::find(&state.db, deployment.site_id)
            .await?
            .map(|site| site.name)
            .unwrap_or_default();
        let worker = worker_name(state, locale, deployment.employee()).await;
        let work = format!("{}, {site}", role_text(&deployment.role));
        for sheet in timesheets(state)
            .of_context(state, &deployment.subject())
            .await?
        {
            let Some(standing) = standing(&sheet) else {
                continue;
            };
            out.push(SheetItem {
                id: sheet.subject.subject_id,
                client: acting.party,
                worker: worker.clone(),
                work: work.clone(),
                from: sheet.from,
                to: sheet.to,
                minutes: sheet.minutes,
                standing,
                adjustment: sheet.kind == "adjustment",
                waiting: acting.approver
                    && sheet.status == "submitted"
                    && sheet.approver_kind == "external",
            });
        }
    }
    out.sort_by(|a, b| {
        b.waiting
            .cmp(&a.waiting)
            .then(b.to.cmp(&a.to))
            .then(b.id.cmp(&a.id))
    });
    Ok(out)
}

/// The timesheets waiting for `user`'s approval in all the companies he acts
/// for.
///
/// # Errors
///
/// The database's errors.
pub async fn waiting(state: &AppState, user: i64) -> Result<Vec<SheetItem>> {
    let mut out = Vec::new();
    for acting in acting(state, user).await? {
        if acting.approver {
            out.extend(
                sheets(state, &acting, "en")
                    .await?
                    .into_iter()
                    .filter(|sheet| sheet.waiting),
            );
        }
    }
    Ok(out)
}

/// `8.00 h` for minutes.
pub fn hours_text(minutes: i64) -> String {
    format!("{}.{:02} h", minutes / 60, (minutes.abs() % 60) * 100 / 60)
}

/// The section's title.
pub fn requests_title() -> String {
    english("manpower.portal.requests_title", &[])
}

/// The section's title.
pub fn deployments_title() -> String {
    english("manpower.portal.deployments_title", &[])
}

/// The section's title.
pub fn timesheets_title() -> String {
    english("manpower.portal.timesheets_title", &[])
}

/// The requests of `party` for the section: what was asked and where it stands.
///
/// # Errors
///
/// The database's errors.
pub async fn requests(state: &AppState, party: PartyId) -> Result<Vec<PortalRow>> {
    let mut rows = Vec::new();
    for request in ManpowerRequest::query()
        .where_(ManpowerRequest::CLIENT_PARTY_ID.eq(party.0))
        .order_by(ManpowerRequest::ID)
        .get(&state.db)
        .await?
    {
        let site = Site::find(&state.db, request.site_id)
            .await?
            .map(|site| site.name)
            .unwrap_or_default();
        rows.push(PortalRow {
            label: format!(
                "{} x {}, {site}",
                request.headcount,
                role_text(&request.role)
            ),
            status: english(
                &format!("manpower.screen.request_status.{}", request.status.as_str()),
                &[],
            ),
        });
    }
    Ok(rows)
}

/// The workers on site of `party` for the section.
///
/// # Errors
///
/// The database's errors.
pub async fn deployments(state: &AppState, party: PartyId) -> Result<Vec<PortalRow>> {
    let mut rows = Vec::new();
    for deployment in on_site(state, party).await? {
        let site = Site::find(&state.db, deployment.site_id)
            .await?
            .map(|site| site.name)
            .unwrap_or_default();
        rows.push(PortalRow {
            label: format!(
                "{}, {}, {site}",
                worker_name(state, "en", deployment.employee()).await,
                role_text(&deployment.role)
            ),
            status: english(
                &format!("manpower.screen.status.{}", deployment.status.as_str()),
                &[],
            ),
        });
    }
    Ok(rows)
}

/// The timesheets of `party` for the section, as the viewer of the company
/// sees them (no action).
///
/// # Errors
///
/// The database's errors.
pub async fn timesheet_rows(state: &AppState, party: PartyId) -> Result<Vec<PortalRow>> {
    let acting = Acting {
        party,
        approver: false,
    };
    Ok(sheets(state, &acting, "en")
        .await?
        .into_iter()
        .map(|sheet| PortalRow {
            label: format!(
                "{}, {}, {}",
                sheet.worker,
                sheet.to.format("%b %Y"),
                hours_text(sheet.minutes)
            ),
            status: english(
                &format!("manpower.screen.portal.sheet.{}", sheet.standing),
                &[],
            ),
        })
        .collect())
}
