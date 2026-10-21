//! The figures of the Construction client: the projects in progress and what
//! waits on them (IPAs the client has not certified, variations it has not
//! answered, guarantees about to end), the open absence cases, the permits
//! about to end, the purchase orders waiting for approval, the cost codes near
//! their budget, and the shared modules' overdue invoices and bounced
//! cheques. Each tile links to its page and is left out when the person lacks
//! the permission of the page or the module is off.

use renox::chrono::Duration;
use renox::prelude::*;
use wl_documents::{Document, DocumentKind, DocumentStatus};
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus};
use wl_kernel::documents::today;
use wl_kernel::inventory::PURCHASE_ORDER_SUBJECT_TYPE;

use super::{Figures, Tile, cheque_tiles, invoice_tile, module_on, number, ticket_tiles};

/// The projects list, where the figures of the projects are broken down.
const PROJECTS_URL: &str = "/construction/projects";

/// The days ahead a permit's end is listed.
const PERMIT_DAYS: i64 = 30;

/// The share of a budget, in basis points, from which a cost code is listed.
const BUDGET_BP: i64 = 9_000;

/// The figures `user` may see in a Construction client.
pub(super) async fn figures(state: &AppState, user: &AuthUser) -> Result<Figures> {
    let mut out = Figures {
        lead: "dashboard.lead_construction",
        ..Figures::default()
    };
    let day = today();

    // The projects the person may see, and what waits on them.
    let projects = ::construction::watch::visible_projects(&state.db, user).await?;
    if !projects.is_empty() {
        let ids = ::construction::watch::ids_of(&projects);
        let running = ::construction::watch::in_progress(&projects).len();
        out.tiles.push(Tile::new(
            "dashboard.tiles.projects_in_progress",
            number(running as u64),
            PROJECTS_URL,
            "hard-hat",
        ));
        let waiting = ::construction::watch::ipas_waiting(&state.db, &ids, day).await?;
        out.tiles.push(
            Tile::new(
                "dashboard.tiles.ipas_waiting",
                number(waiting.len() as u64),
                PROJECTS_URL,
                "clock",
            )
            .toned("sand"),
        );
        let unanswered = ::construction::watch::variations_waiting(&state.db, &ids, day).await?;
        out.tiles.push(
            Tile::new(
                "dashboard.tiles.variations_waiting",
                number(unanswered.len() as u64),
                PROJECTS_URL,
                "file-text",
            )
            .toned("sand"),
        );
        let expiring = ::construction::watch::guarantees_expiring(&state.db, &ids, day).await?;
        out.tiles.push(
            Tile::new(
                "dashboard.tiles.guarantees_expiring",
                number(expiring.len() as u64),
                PROJECTS_URL,
                "shield",
            )
            .toned("clay"),
        );
    }

    // Purchase orders that wait for a second person.
    if module_on(state, "inventory") && may_issue_orders(user) {
        let waiting = ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_TYPE.eq(PURCHASE_ORDER_SUBJECT_TYPE))
            .where_(ApprovalRequest::STATUS.eq(ApprovalStatus::Pending))
            .count(&state.db)
            .await?;
        // There is no list of orders: the requests that wait are in the
        // decisions block of this page.
        out.tiles.push(Tile::new(
            "dashboard.tiles.purchase_orders_waiting",
            number(waiting),
            "/dashboard#awaiting-title",
            "shopping-cart",
        ));
    }

    // The cost codes at or above 90 % of this month's budget.
    if module_on(state, "invoicing")
        && let Some(near) =
            wl_invoicing::budget_report::categories_near_limit(state, user, day, BUDGET_BP).await?
    {
        out.tiles.push(
            Tile::new(
                "dashboard.tiles.budget_near_limit",
                number(near as u64),
                "/budget-report",
                "gauge",
            )
            .toned("clay"),
        );
    }

    // People: the open absence cases and the permits about to end.
    if module_on(state, "hr") && user.has_permission(wl_hr::permissions::EMPLOYEES_VIEW) {
        let open = ::construction::AbsenceCase::query()
            .where_(::construction::AbsenceCase::ENDED_ON.is_null())
            .count(&state.db)
            .await?;
        out.tiles.push(
            Tile::new(
                "dashboard.tiles.absence_open",
                number(open),
                "/hr/employees",
                "user",
            )
            .toned("clay"),
        );
        if module_on(state, "documents") {
            let ending = permits_ending(state, day, day + Duration::days(PERMIT_DAYS)).await?;
            out.tiles.push(
                Tile::new(
                    "dashboard.tiles.permits_expiring",
                    number(ending),
                    "/hr/employees",
                    "id-card",
                )
                .toned("sand"),
            );
        }
    }

    // Money: what the shared modules count.
    invoice_tile(state, user, &mut out.tiles).await?;
    cheque_tiles(state, user, false, &mut out.tiles).await?;
    ticket_tiles(state, user, false, &mut out).await?;
    Ok(out)
}

/// Whether `user` may issue purchase orders somewhere, which is who the
/// approval of a revision waits for.
fn may_issue_orders(user: &AuthUser) -> bool {
    [
        wl_inventory::permissions::PURCHASE_ORDERS_ISSUE,
        wl_inventory::permissions::PURCHASE_ORDERS_VIEW,
    ]
    .iter()
    .any(|permission| user.allows(permission) || user.has_permission_anywhere(permission))
}

/// The accepted work permits and residence visas whose end falls from `from`
/// to `to`.
async fn permits_ending(
    state: &AppState,
    from: renox::chrono::NaiveDate,
    to: renox::chrono::NaiveDate,
) -> Result<u64> {
    let kinds = state
        .provided::<wl_attendance::AttendanceConfig>()
        .map(|config| config.permit_kinds.clone())
        .unwrap_or_default();
    let mut ids = Vec::new();
    for key in &kinds {
        if let Some(kind) = DocumentKind::by_key(&state.db, key).await? {
            ids.push(kind.id);
        }
    }
    if ids.is_empty() {
        return Ok(0);
    }
    let found = Document::query()
        .where_(Document::KIND_ID.is_in(ids))
        .where_(Document::STATUS.eq(DocumentStatus::Accepted))
        .get(&state.db)
        .await?;
    Ok(found
        .iter()
        .filter(|document| {
            document
                .expiry_date
                .is_some_and(|day| day >= from && day <= to)
        })
        .count() as u64)
}
