//! What the home page of a Manufacturing deployment says (BL-MFG-14).
//!
//! The people of the plant see the counts they can act on, each linking to its
//! list and left out when it is zero, without the permission of that list or
//! when the module is off: the work orders in progress, held and planned, the
//! open NCRs, the returned goods waiting for a decision and the lots waiting
//! for QC, the conformity certificates that end within 30 days, the exports
//! without evidence, the open recalls and the orders on credit hold. Then come
//! the shared modules' money and ticket tiles.
//!
//! - **Scope.** A work order or an NCR counts when the person may open it: the
//!   permission in its plant or line, or globally.
//! - **Plural.** The label is the singular for 1 and the plural otherwise,
//!   next to the number.

use ::manufacturing::exports::ExportWatch;
use ::manufacturing::permissions;
use ::manufacturing::returns::{ReturnDecision, ReturnInspection};
use ::manufacturing::{Ncr, NcrStatus, WorkOrder, WorkOrderStatus};
use renox::chrono::Duration;
use renox::prelude::*;
use wl_documents::{Document, DocumentStatus};
use wl_kernel::documents::today;
use wl_sales::{SalesOrder, SalesOrderStatus};

use super::{Figures, Tile, cheque_tiles, invoice_tile, module_on, number, ticket_tiles};

/// The days ahead a certificate's expiry is listed.
const CERTIFICATE_DAYS: i64 = 30;

/// The document kinds of the conformity certificates.
const CERTIFICATES: [&str; 3] = [
    ::manufacturing::conformity::ECAS,
    ::manufacturing::conformity::EQM,
    ::manufacturing::conformity::FOOD_CONTACT,
];

/// Whether `user` holds any of `needed`, here or anywhere.
fn may(user: &AuthUser, needed: &[&str]) -> bool {
    needed.iter().any(|permission| {
        user.has_permission(permission) || user.has_permission_anywhere(permission)
    })
}

/// The figures `user` may see in a Manufacturing deployment.
pub(super) async fn figures(state: &AppState, user: &AuthUser) -> Result<Figures> {
    let mut out = Figures {
        lead: "dashboard.lead_manufacturing",
        ..Figures::default()
    };
    let mut block: Vec<Tile> = Vec::new();
    let mut add = |count: u64,
                   one: &'static str,
                   many: &'static str,
                   url: &str,
                   icon: &'static str,
                   tone: Option<&'static str>| {
        if count == 0 {
            return;
        }
        let label = if count == 1 { one } else { many };
        let tile = Tile::new(label, number(count), url, icon);
        block.push(match tone {
            Some(tone) => tile.toned(tone),
            None => tile,
        });
    };

    // Work orders: running, held and waiting to be released.
    if may(
        user,
        &[
            permissions::WORK_ORDERS_VIEW,
            permissions::WORK_ORDERS_CREATE,
            permissions::WORK_ORDERS_REPORT,
        ],
    ) {
        let orders: Vec<WorkOrder> = WorkOrder::query()
            .get(&state.db)
            .await?
            .into_iter()
            .filter(|order| {
                [
                    permissions::WORK_ORDERS_VIEW,
                    permissions::WORK_ORDERS_CREATE,
                    permissions::WORK_ORDERS_REPORT,
                ]
                .iter()
                .any(|permission| order.may(user, permission))
            })
            .collect();
        let count = |status: WorkOrderStatus| orders.iter().filter(|o| o.status == status).count();
        add(
            count(WorkOrderStatus::InProgress) as u64,
            "dashboard.tiles.work_orders_running_one",
            "dashboard.tiles.work_orders_running_many",
            "/manufacturing/work-orders?status=in_progress",
            "boxes",
            None,
        );
        add(
            count(WorkOrderStatus::OnHold) as u64,
            "dashboard.tiles.work_orders_held_one",
            "dashboard.tiles.work_orders_held_many",
            "/manufacturing/work-orders?status=on_hold",
            "clock",
            Some("clay"),
        );
        add(
            count(WorkOrderStatus::Planned) as u64,
            "dashboard.tiles.work_orders_planned_one",
            "dashboard.tiles.work_orders_planned_many",
            "/manufacturing/work-orders?status=planned",
            "calendar-days",
            Some("sand"),
        );
    }

    // Open NCRs.
    if may(user, &[permissions::NCRS_VIEW, permissions::NCRS_RAISE]) {
        let open = Ncr::query()
            .where_(Ncr::STATUS.eq(NcrStatus::Open))
            .count(&state.db)
            .await?;
        add(
            open,
            "dashboard.tiles.ncrs_open_one",
            "dashboard.tiles.ncrs_open_many",
            "/manufacturing/ncrs",
            "triangle-alert",
            Some("clay"),
        );
    }

    // Returned goods: sales decides, QC inspects.
    if may(
        user,
        &[
            permissions::RETURNED_GOODS_VIEW,
            permissions::RETURNED_GOODS_DECIDE,
        ],
    ) {
        let waiting = ReturnDecision::query()
            .where_(ReturnDecision::STATUS.eq("open"))
            .count(&state.db)
            .await?;
        add(
            waiting,
            "dashboard.tiles.returns_waiting_one",
            "dashboard.tiles.returns_waiting_many",
            "/manufacturing/returns",
            "refresh-cw",
            Some("clay"),
        );
    }
    if may(
        user,
        &[
            permissions::INSPECTIONS_VIEW,
            permissions::INSPECTIONS_RECORD,
        ],
    ) {
        let waiting = ReturnInspection::query()
            .where_(ReturnInspection::STATUS.eq("open"))
            .count(&state.db)
            .await?;
        add(
            waiting,
            "dashboard.tiles.inspections_waiting_one",
            "dashboard.tiles.inspections_waiting_many",
            "/manufacturing/inspections",
            "clipboard-list",
            Some("sand"),
        );
    }

    // Certificates that end soon.
    if module_on(state, "documents") && may(user, &[permissions::CONFORMITY_MANAGE]) {
        let day = today();
        let until = day + Duration::days(CERTIFICATE_DAYS);
        let kinds: Vec<i64> = {
            let mut ids = Vec::new();
            for key in CERTIFICATES {
                if let Some(kind) = wl_documents::DocumentKind::by_key(&state.db, key).await? {
                    ids.push(kind.id);
                }
            }
            ids
        };
        let expiring = Document::query()
            .where_(Document::STATUS.eq(DocumentStatus::Accepted))
            .where_(Document::KIND_ID.is_in(kinds))
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
            "dashboard.tiles.certificates_expiring_one",
            "dashboard.tiles.certificates_expiring_many",
            "/manufacturing/conformity",
            "shield-check",
            Some("sand"),
        );
    }

    // Exports whose exit evidence is missing after 60 days.
    if may(user, &[permissions::EXPORTS_MANAGE]) {
        let missing = ExportWatch::query()
            .where_(ExportWatch::EVIDENCE_ON.is_null())
            .where_(ExportWatch::WARNED_ON.is_not_null())
            .count(&state.db)
            .await?;
        add(
            missing,
            "dashboard.tiles.exports_missing_one",
            "dashboard.tiles.exports_missing_many",
            "/manufacturing/exports",
            "truck",
            Some("clay"),
        );
    }

    // Open recalls.
    if module_on(state, "inventory")
        && may(
            user,
            &[permissions::RECALLS_VIEW, permissions::RECALLS_MANAGE],
        )
    {
        let open = renox::db::sql(
            "select count(*) from wl_inventory_recalls where status not in ('closed', 'cancelled')"
                .to_owned(),
        )
        .scalar::<i64>(&state.db)
        .await?;
        add(
            u64::try_from(open).unwrap_or(0),
            "dashboard.tiles.recalls_open_one",
            "dashboard.tiles.recalls_open_many",
            "/recalls",
            "triangle-alert",
            Some("clay"),
        );
    }

    // Orders held for credit.
    if module_on(state, "sales") && may(user, &[wl_sales::permissions::SALES_ORDERS_VIEW]) {
        let held = SalesOrder::query()
            .where_(SalesOrder::STATUS.eq(SalesOrderStatus::CreditHold))
            .count(&state.db)
            .await?;
        add(
            held,
            "dashboard.tiles.orders_held_one",
            "dashboard.tiles.orders_held_many",
            "/sales-orders?in.status=credit_hold",
            "lock",
            Some("clay"),
        );
    }

    out.tiles = block;

    // Money: what the shared modules count.
    invoice_tile(state, user, &mut out.tiles).await?;
    cheque_tiles(state, user, false, &mut out.tiles).await?;
    ticket_tiles(state, user, false, &mut out).await?;
    Ok(out)
}
