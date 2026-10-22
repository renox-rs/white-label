//! What the home page of a Manpower deployment says (BL-MPW-28, BL-MPW-32).
//!
//! The agency's staff see the counts they can act on, each linking to its list
//! and left out without the permission of that list or when the module is off:
//! the contracts on hold, the contracts ending within 60 days, the requests
//! still to fill, the worker documents expiring within 30 days, the suspended
//! deployments, the open absence cases, the timesheets waiting for the client
//! and the payroll runs waiting for approval. Then come the shared modules'
//! money and ticket tiles.
//!
//! - **Scope.** A record counts only when the person may open it: the
//!   contract, request and deployment by the permission in its client (or
//!   site) scope, a timesheet by the reach of `attendance.view`. An account
//!   manager counts his clients only.
//! - **Zero.** A count of zero is left out. When the person holds the
//!   permission of at least one of these tiles and every count is zero, the
//!   page says *Nothing needs your action* instead (`Figures::quiet`).
//! - **Plural.** The label is the singular for 1 (*Contract on hold*) and the
//!   plural otherwise, next to the number.
//! - **Documents.** There is no list of documents: the tile counts the
//!   accepted documents of workers (`hr.employee`) and opens the workers' list
//!   (so it needs `employees.view` as well as `documents.view`, global).
//! - **Timesheets.** The count is the list's own standing *submitted*
//!   (`/timesheets?status=submitted`): the escalated ones are counted there
//!   under their own standing.
//!
//! The client approver's own line (timesheets waiting for them, BL-MPW-28) is
//! [`timesheets_waiting`].

use renox::prelude::*;

use ::manpower::absence::AbsenceCase;
use ::manpower::permissions;
use ::manpower::{ClientContract, ContractStatus, Deployment, DeploymentStatus, ManpowerRequest};
use renox::chrono::Duration;
use wl_attendance::{Timesheet, TimesheetStatus};
use wl_documents::{Document, DocumentStatus};
use wl_hr::{PayrollRun, PayrollRunStatus};
use wl_kernel::documents::today;

use super::{Figures, Tile, app_of, cheque_tiles, invoice_tile, module_on, number, ticket_tiles};

/// The days ahead a contract's end is listed.
const ENDING_DAYS: i64 = 60;

/// The days ahead a worker document's expiry is listed.
const DOCUMENT_DAYS: i64 = 30;

/// The subject type of the documents of workers.
const WORKER_DOCUMENTS: &str = "hr.employee";

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

/// The figures `user` may see in a Manpower deployment.
pub(super) async fn figures(state: &AppState, user: &AuthUser) -> Result<Figures> {
    let mut out = Figures {
        lead: "dashboard.lead_manpower",
        ..Figures::default()
    };
    let day = today();
    // The tiles the person may open, and how many of them have something to say.
    let mut may = 0;
    let mut block: Vec<Tile> = Vec::new();
    let mut add = |count: u64,
                   one: &'static str,
                   many: &'static str,
                   url: &str,
                   icon: &'static str,
                   tone: Option<&'static str>| {
        let label = if count == 1 { one } else { many };
        if count > 0 {
            let tile = Tile::new(label, number(count), url, icon);
            block.push(match tone {
                Some(tone) => tile.toned(tone),
                None => tile,
            });
        }
    };

    // Contracts: on hold, and ending soon.
    if may_see_contracts(user) {
        may += 2;
        let contracts: Vec<ClientContract> = ClientContract::query()
            .get(&state.db)
            .await?
            .into_iter()
            .filter(|contract| {
                [permissions::CONTRACTS_VIEW, permissions::CONTRACTS_MANAGE]
                    .iter()
                    .any(|permission| user.has_permission_in(permission, &contract.scope()))
            })
            .collect();
        let held = contracts
            .iter()
            .filter(|contract| contract.status == ContractStatus::OnHold)
            .count();
        add(
            held as u64,
            "dashboard.tiles.contracts_on_hold_one",
            "dashboard.tiles.contracts_on_hold_many",
            "/manpower/contracts?status=on_hold",
            "clock",
            Some("clay"),
        );
        let ending = contracts
            .iter()
            .filter(|contract| ends_within(contract, day))
            .count();
        add(
            ending as u64,
            "dashboard.tiles.contracts_ending_one",
            "dashboard.tiles.contracts_ending_many",
            "/manpower/contracts?ending=60",
            "calendar-days",
            Some("sand"),
        );
    }

    // Requests still to fill.
    if may_see_requests(user) {
        may += 1;
        let unfilled = ManpowerRequest::query()
            .get(&state.db)
            .await?
            .into_iter()
            .filter(|request| request.status.takes_workers())
            .filter(|request| {
                [
                    permissions::REQUESTS_VIEW,
                    permissions::REQUESTS_CREATE,
                    permissions::REQUESTS_MANAGE,
                ]
                .iter()
                .any(|permission| user.has_permission_in(permission, &request.scope()))
            })
            .count();
        add(
            unfilled as u64,
            "dashboard.tiles.requests_unfilled_one",
            "dashboard.tiles.requests_unfilled_many",
            "/manpower/requests?status=missing",
            "users",
            Some("sand"),
        );
    }

    // The documents of workers that end soon.
    if module_on(state, "documents")
        && module_on(state, "hr")
        && user.has_permission(wl_documents::permissions::VIEW)
        && user.has_permission(wl_hr::permissions::EMPLOYEES_VIEW)
    {
        may += 1;
        let until = day + Duration::days(DOCUMENT_DAYS);
        let expiring = Document::query()
            .where_(Document::SUBJECT_TYPE.eq(WORKER_DOCUMENTS))
            .where_(Document::STATUS.eq(DocumentStatus::Accepted))
            .get(&state.db)
            .await?
            .into_iter()
            .filter(|document| {
                document
                    .expiry_date
                    .is_some_and(|date| date >= day && date <= until)
            })
            .count();
        add(
            expiring as u64,
            "dashboard.tiles.documents_expiring_one",
            "dashboard.tiles.documents_expiring_many",
            "/hr/employees?documents=expiring",
            "id-card",
            Some("sand"),
        );
    }

    // Deployments on hold at the client's request.
    if may_see_deployments(user) {
        may += 1;
        let suspended = Deployment::query()
            .where_(Deployment::STATUS.eq(DeploymentStatus::Suspended))
            .get(&state.db)
            .await?
            .into_iter()
            .filter(|deployment| {
                [
                    permissions::DEPLOYMENTS_VIEW,
                    permissions::DEPLOYMENTS_MANAGE,
                ]
                .iter()
                .any(|permission| {
                    user.has_permission_in(permission, &deployment.scope())
                        || user.has_permission_in(permission, &deployment.site_scope())
                })
            })
            .count();
        add(
            suspended as u64,
            "dashboard.tiles.deployments_suspended_one",
            "dashboard.tiles.deployments_suspended_many",
            "/manpower/deployments?status=suspended",
            "user",
            Some("clay"),
        );
    }

    // Absence cases HR has not closed.
    if user.has_permission(permissions::ABSENCE_MANAGE) {
        may += 1;
        let open = AbsenceCase::query()
            .where_(AbsenceCase::CLOSED_ON.is_null())
            .count(&state.db)
            .await?;
        add(
            open,
            "dashboard.tiles.absence_cases_one",
            "dashboard.tiles.absence_cases_many",
            "/manpower/absence",
            "user",
            Some("clay"),
        );
    }

    // Timesheets the client has not answered.
    if module_on(state, "attendance") && may_see_timesheets(user) {
        may += 1;
        let waiting = timesheets_for_client(state, user).await?;
        add(
            waiting as u64,
            "dashboard.tiles.timesheets_waiting_one",
            "dashboard.tiles.timesheets_waiting_many",
            "/timesheets?status=submitted",
            "clock",
            Some("sand"),
        );
    }

    // Payroll runs a second person has to approve.
    if module_on(state, "hr") && user.has_permission(wl_hr::permissions::PAYROLL_VIEW) {
        may += 1;
        let waiting = PayrollRun::query()
            .where_(PayrollRun::STATUS.eq(PayrollRunStatus::Calculated))
            .count(&state.db)
            .await?;
        add(
            waiting,
            "dashboard.tiles.payroll_waiting_one",
            "dashboard.tiles.payroll_waiting_many",
            "/hr/payroll",
            "banknote",
            Some("sand"),
        );
    }

    out.quiet = may > 0 && block.is_empty();
    out.tiles = block;

    // Money: what the shared modules count.
    invoice_tile(state, user, &mut out.tiles).await?;
    cheque_tiles(state, user, false, &mut out.tiles).await?;
    ticket_tiles(state, user, false, &mut out).await?;
    Ok(out)
}

/// Whether the contract is running and ends from `day` to 60 days on.
fn ends_within(contract: &ClientContract, day: renox::chrono::NaiveDate) -> bool {
    matches!(
        contract.status,
        ContractStatus::Active | ContractStatus::OnHold
    ) && contract.end_date >= day
        && contract.end_date <= day + Duration::days(ENDING_DAYS)
}

/// Whether `user` may open the contracts list at all.
fn may_see_contracts(user: &AuthUser) -> bool {
    [permissions::CONTRACTS_VIEW, permissions::CONTRACTS_MANAGE]
        .iter()
        .any(|permission| user.allows(permission) || user.has_permission_anywhere(permission))
}

/// Whether `user` may open the requests list at staff level.
fn may_see_requests(user: &AuthUser) -> bool {
    [
        permissions::REQUESTS_VIEW,
        permissions::REQUESTS_CREATE,
        permissions::REQUESTS_MANAGE,
    ]
    .iter()
    .any(|permission| user.allows(permission) || user.has_permission_anywhere(permission))
}

/// Whether `user` may open the deployments list at staff level.
fn may_see_deployments(user: &AuthUser) -> bool {
    [
        permissions::DEPLOYMENTS_VIEW,
        permissions::DEPLOYMENTS_MANAGE,
    ]
    .iter()
    .any(|permission| user.allows(permission) || user.has_permission_anywhere(permission))
}

/// Whether `user` may open the timesheets list.
fn may_see_timesheets(user: &AuthUser) -> bool {
    let permission = wl_attendance::permissions::VIEW;
    user.allows(permission) || user.has_permission_anywhere(permission)
}

/// The timesheets in the person's reach that were sent to the client and not
/// yet answered nor escalated.
async fn timesheets_for_client(state: &AppState, user: &AuthUser) -> Result<usize> {
    let reach = wl_kernel::scopes::reach(&state.db, user, wl_attendance::permissions::VIEW).await?;
    let sheets = Timesheet::query()
        .where_(Timesheet::STATUS.eq(TimesheetStatus::Submitted))
        .get(&state.db)
        .await?;
    let mut count = 0;
    for sheet in sheets.iter().filter(|sheet| sheet.escalated_at.is_none()) {
        let in_reach = match sheet.context() {
            _ if reach.is_all() => true,
            Some(context) => reach.contains(&state.db, &context).await?,
            None => false,
        };
        if in_reach {
            count += 1;
        }
    }
    Ok(count)
}
