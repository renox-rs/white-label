//! Where a worker is placed, for the employee page (BL-MPW-31, #511).
//!
//! `wl-hr` shows the *Deployments* tab of an employee without knowing the
//! vertical: it asks the kernel's [`Placements`], which this vertical provides.
//! HR (`employees.manage`) and the holders of `deployments.view` everywhere see
//! every deployment of the worker; the account manager, whose role is assigned
//! in the client company, sees the deployments of those clients only, and a
//! user who may see none of them gets no tab (`None`). A worker without a
//! deployment is on the bench (`Some(empty)`).

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::parties::directory;
use wl_kernel::placements::{Placement, Placements, PlacementsFuture};
use wl_kernel::workforce::EmployeeId;

use crate::deployment::{Deployment, DeploymentStatus};
use crate::permissions;
use crate::screens::role_text;
use crate::site::Site;
use crate::texts::word;

/// The kernel contract, provided in `register`.
pub struct WorkerPlacements;

/// HR's permission to manage employees: it sees every placement of a worker.
const EMPLOYEES_MANAGE: &str = "employees.manage";

impl Placements for WorkerPlacements {
    fn of_employee<'a>(
        &'a self,
        state: &'a AppState,
        user: &'a AuthUser,
        employee: EmployeeId,
        locale: &'a str,
    ) -> PlacementsFuture<'a, Option<Vec<Placement>>> {
        Box::pin(async move {
            let sees_all = user.has_permission(permissions::DEPLOYMENTS_VIEW)
                || user.has_permission(permissions::DEPLOYMENTS_MANAGE)
                || user.has_permission(EMPLOYEES_MANAGE);
            let sees_some = sees_all
                || user.has_permission_anywhere(permissions::DEPLOYMENTS_VIEW)
                || user.has_permission_anywhere(permissions::DEPLOYMENTS_MANAGE);
            if !sees_some {
                return Ok(None);
            }
            let lang = state.lang(locale);
            let mut out = Vec::new();
            for deployment in Deployment::query()
                .where_(Deployment::EMPLOYEE_ID.eq(employee.0))
                .order_by_desc(Deployment::START_DATE)
                .order_by_desc(Deployment::ID)
                .get(&state.db)
                .await?
            {
                let may = sees_all
                    || [
                        permissions::DEPLOYMENTS_VIEW,
                        permissions::DEPLOYMENTS_MANAGE,
                    ]
                    .iter()
                    .any(|permission| {
                        user.has_permission_in(permission, &deployment.scope())
                            || user.has_permission_in(permission, &deployment.site_scope())
                    });
                if !may {
                    continue;
                }
                let site = Site::find(&state.db, deployment.site_id)
                    .await?
                    .map(|site| site.name)
                    .unwrap_or_default();
                let client = directory(state)
                    .display_name(&state.db, PartyId(deployment.client_party_id), locale)
                    .await?
                    .unwrap_or_default();
                out.push(Placement {
                    url: format!("/manpower/deployments/{}", deployment.id),
                    client,
                    site,
                    role: role_text(&deployment.role),
                    from: deployment.start_date,
                    to: deployment.end_date,
                    status: word(&lang, "status", deployment.status.as_str()),
                    on_site: matches!(
                        deployment.status,
                        DeploymentStatus::Active | DeploymentStatus::Suspended
                    ),
                });
            }
            Ok(Some(out))
        })
    }
}
