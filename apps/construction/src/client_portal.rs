//! The client portal after the final account (02 §3.3 `POR.ACL.01`, §4.8
//! step 6, §7.1 *Project* `close` and `settle_final_account`; BL-CON-19,
//! issue #298).
//!
//! Once a project is `closed` (by `close` after the defects liability period,
//! or by `settle_final_account` after a termination), the client contacts keep
//! **read-only** access to its documents: they can read them, nothing can be
//! uploaded or answered. After [`CLOSE_AFTER_DAYS`] (90) calendar days,
//! counted from the Asia/Dubai day the project became `closed`, the project is
//! no longer visible in the portal. The window is per project: the client's
//! other projects are not touched ([`access_on`], [`visible_projects`]).
//!
//! Who calls what: after the commit of `close` or `settle_final_account` the
//! module emits [`ProjectClosed`] (the client, the day). The host hands it to
//! wl-portal: when the client has no project left that is not `closed` or
//! `cancelled` ([`has_open_project`]), its portal grants in the
//! [`CLIENT_AUDIENCE`] run `end` on that day, and wl-portal's close-after
//! timer closes them on day 91, so the last project's window and the grant's
//! end together. While another project runs, the grant stays `active` and
//! the closed project is read-only through [`access_on`].
//!
//! The client portal pages themselves (`POR.SLF.01`, Later for 02) are not
//! built yet; they ask [`access_on`] for each project and refuse an upload
//! with [`upload_problem`].
//!
//! Decisions:
//!
//! - **Day 90 is the last day.** A project closed on 2028-01-31 is readable
//!   through 2028-04-30 and hidden from 2028-05-01, as wl-portal closes a
//!   90-day grant on day 91.
//! - **Other states.** Only `closed` changes the access here; a `cancelled`
//!   project is outside this rule.

use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::dubai_date;
use wl_kernel::{PartyId, Subject};

use crate::project::{Project, ProjectStatus};
use crate::texts::english;

/// The portal audience of the 02 client contacts (`POR.SLF.01`).
pub const CLIENT_AUDIENCE: &str = "client";
/// The days a closed project stays readable (`POR.ACL.01`).
pub const CLOSE_AFTER_DAYS: i64 = 90;

/// What a client contact may do with a project in the portal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ClientAccess {
    /// The project runs: its documents can be read and uploaded.
    Full,
    /// The project is `closed`: its documents can be read until `until`
    /// (inclusive), nothing can be uploaded or answered.
    ReadOnly {
        /// The last day it is visible.
        until: NaiveDate,
    },
    /// No longer visible.
    Hidden,
}

impl ClientAccess {
    /// Whether the project is shown.
    pub fn visible(self) -> bool {
        !matches!(self, ClientAccess::Hidden)
    }

    /// Whether something can be uploaded or answered.
    pub fn may_upload(self) -> bool {
        matches!(self, ClientAccess::Full)
    }
}

/// The Asia/Dubai day `project` became `closed`; `None` when it is not.
pub fn closed_on(project: &Project) -> Option<NaiveDate> {
    if project.status != ProjectStatus::Closed {
        return None;
    }
    project.closed_at.map(dubai_date)
}

/// The last day a project closed on `closed` is visible.
pub fn readable_until(closed: NaiveDate) -> NaiveDate {
    closed + Duration::days(CLOSE_AFTER_DAYS)
}

/// What a client contact may do with `project` on `day`.
pub fn access_on(project: &Project, day: NaiveDate) -> ClientAccess {
    if project.status != ProjectStatus::Closed {
        return ClientAccess::Full;
    }
    // A closed project without its stamp has no window to count: hidden.
    let Some(closed) = closed_on(project) else {
        return ClientAccess::Hidden;
    };
    let until = readable_until(closed);
    if day <= until {
        ClientAccess::ReadOnly { until }
    } else {
        ClientAccess::Hidden
    }
}

/// Why nothing can be uploaded to `project` on `day`; `None` when it can.
pub fn upload_problem(project: &Project, day: NaiveDate) -> Option<String> {
    match access_on(project, day) {
        ClientAccess::Full => None,
        ClientAccess::ReadOnly { until } => Some(english(
            "construction.client_portal.read_only",
            &[("until", &until)],
        )),
        ClientAccess::Hidden => Some(english("construction.client_portal.hidden", &[])),
    }
}

/// The projects of `client` its contacts see on `day`, with their access,
/// oldest first.
///
/// # Errors
///
/// The database's errors.
pub async fn visible_projects(
    db: &Db,
    client: PartyId,
    day: NaiveDate,
) -> Result<Vec<(Project, ClientAccess)>> {
    let projects = Project::query()
        .where_(Project::CLIENT_PARTY_ID.eq(client.0))
        .order_by(Project::ID)
        .get(db)
        .await?;
    Ok(projects
        .into_iter()
        .map(|project| {
            let access = access_on(&project, day);
            (project, access)
        })
        .filter(|(_, access)| access.visible())
        .collect())
}

/// Whether `client` still has a project that is neither `closed` nor
/// `cancelled`: its portal grant must then stay `active`.
///
/// # Errors
///
/// The database's errors.
pub async fn has_open_project(db: &Db, client: PartyId) -> Result<bool> {
    Ok(Project::query()
        .where_(Project::CLIENT_PARTY_ID.eq(client.0))
        .where_(Project::STATUS.is_in([
            ProjectStatus::Setup,
            ProjectStatus::InProgress,
            ProjectStatus::OnHold,
            ProjectStatus::PracticalCompletion,
            ProjectStatus::Terminated,
        ]))
        .first(db)
        .await?
        .is_some())
}

/// A project's final account is closed (`close` or `settle_final_account`
/// committed): its client portal turns read-only (`POR.ACL.01`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectClosed {
    /// The project, `construction.project:4`.
    pub project: Subject,
    /// Its client.
    pub client: PartyId,
    /// The Asia/Dubai day it became `closed`.
    pub closed_on: NaiveDate,
    /// The transition: `close` or `settle_final_account`.
    pub transition: String,
}

impl Event for ProjectClosed {}

/// After the commit of `transition` on project `id`: emits [`ProjectClosed`]
/// when the project is `closed`.
///
/// # Errors
///
/// The database's errors and the listeners'.
pub async fn after_close(state: &AppState, id: i64, transition: &str) -> Result {
    let project = Project::find_or_404(&state.db, id).await?;
    let Some(closed) = closed_on(&project) else {
        return Ok(());
    };
    state
        .emit(ProjectClosed {
            project: project.subject(),
            client: project.client(),
            closed_on: closed,
            transition: transition.to_owned(),
        })
        .await
}
