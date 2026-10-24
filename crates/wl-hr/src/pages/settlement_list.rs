//! The final settlement list (issue #510, `BL-MPW-30`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /hr/settlements` | `wl_hr.settlements` | `settlements.prepare`, `.approve` or `.pay` |
//!
//! Employee, reason, status in words, dues, recoveries, net and paid in AED,
//! and the legal deadline (`settlement_deadline_days` after the last working
//! day, 14 by default). A draft past its deadline carries a warning badge.

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::documents::today;

use crate::config::config_of;
use crate::employee::Employee;
use crate::pages::payroll::date_text;
use crate::settlement::{FinalSettlement, SettlementStatus};
use crate::{permissions, texts};

type Texts = BTreeMap<&'static str, String>;

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/hr/settlements", index)
        .name("wl_hr.settlements")
        .require_auth()
}

/// The list page.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/settlements.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    settlements: GridPage<FinalSettlement>,
}

/// The settlements grid: filters by status, the dues, recoveries, net and
/// paid in AED with their sums. The employee's name and the legal deadline
/// (a badge when overdue) are drawn by the template (`custom`).
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let t = |key: &str| texts::t(lang, &format!("wl_hr.{key}"));
    let statuses: Vec<(String, String)> = [
        SettlementStatus::Draft,
        SettlementStatus::Approved,
        SettlementStatus::Disputed,
        SettlementStatus::Paid,
        SettlementStatus::Closed,
        SettlementStatus::Cancelled,
    ]
    .iter()
    .map(|status| {
        (
            status.as_str().to_owned(),
            t(&format!("settlement.status.{}", status.as_str())),
        )
    })
    .collect();
    let grid = Grid::new("hr-settlements")
        .title(&t("settlement.list.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::related(
                "employee_number",
                &t("employees.number"),
                "wl_hr_employees",
                "employee_id",
                "employee_number",
            )
            .frozen()
            .mobile()
            .searchable()
            .link("/hr/settlements/{id}"),
        )
        .column(Column::custom("employee", &t("settlement.list.employee")).mobile())
        .column(Column::text("reason", &t("settlement.reason")).searchable())
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("draft", "info"),
                    ("approved", "success"),
                    ("disputed", "warning"),
                    ("paid", "success"),
                    ("closed", "success"),
                    ("cancelled", "info"),
                ])
                .mobile(),
        )
        .column(
            Column::money("dues", &wl_kernel::money_label(&t("settlement.list.dues")))
                .computed(
                    "({T}.\"unpaid_salary\" + {T}.\"leave_encashment\" + {T}.\"gratuity\" \
                     + {T}.\"notice_pay\" + {T}.\"other_dues\")",
                )
                .summary(Summary::Sum),
        )
        .column(
            Column::money(
                "recoveries_total",
                &wl_kernel::money_label(&t("settlement.list.recoveries")),
            )
            .computed(
                "({T}.\"recoveries\" + {T}.\"advance_recoveries\" + {T}.\"fine_recoveries\" \
                     + {T}.\"court_debt_recoveries\" + {T}.\"damage_recoveries\")",
            )
            .summary(Summary::Sum),
        )
        .column(
            Column::money("net", &wl_kernel::money_label(&t("settlement.list.net")))
                .mobile()
                .summary(Summary::Sum),
        )
        .column(
            Column::money("paid", &wl_kernel::money_label(&t("settlement.list.paid")))
                .computed(
                    "(SELECT COALESCE(SUM(\"wl_hr_settlement_payments\".\"amount\"), 0) \
                     FROM \"wl_hr_settlement_payments\" \
                     WHERE \"wl_hr_settlement_payments\".\"settlement_id\" = {T}.\"id\")",
                )
                .summary(Summary::Sum),
        )
        .column(Column::date("last_working_day", &t("employees.last_working_day")).hidden())
        .column(Column::custom("deadline", &t("settlement.deadline")).mobile())
        .sort_by("-id")
        .row_url("/hr/settlements/{id}")
        .cards_on_mobile()
        .empty_state(
            &t("settlement.list.empty"),
            Some(&t("settlement.list.empty_hint")),
        );
    if can_export { grid.exports() } else { grid }
}

/// `GET /hr/settlements`: the grid, or its export (`?export=`, with
/// `settlements.prepare` or `settlements.approve`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    if ![
        permissions::SETTLEMENTS_PREPARE,
        permissions::SETTLEMENTS_APPROVE,
        permissions::SETTLEMENTS_PAY,
    ]
    .iter()
    .any(|p| user.has_permission(p))
    {
        return Err(Error::Forbidden);
    }
    let can_export = user.has_permission(permissions::SETTLEMENTS_PREPARE)
        || user.has_permission(permissions::SETTLEMENTS_APPROVE);
    let grid = grid(&lang, can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(FinalSettlement::query(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(FinalSettlement::query(), &request).await?;
    let days = config_of(&state).settlement_deadline_days;
    let mut names: BTreeMap<i64, String> = BTreeMap::new();
    for settlement in page.items() {
        let name = match Employee::find(&state.db, settlement.employee_id).await? {
            Some(e) => crate::pages::employees::employee_name(&state, &lang, &e).await,
            None => format!("#{}", settlement.employee_id),
        };
        names.insert(settlement.id, name);
    }
    let day = today();
    let settlements = page.extend(|s| {
        let deadline = s.deadline(days);
        let open = matches!(
            s.status,
            SettlementStatus::Draft | SettlementStatus::Approved | SettlementStatus::Disputed
        );
        renox::serde_json::json!({
            "employee": names.get(&s.id).cloned().unwrap_or_default(),
            "deadline": deadline.map(date_text).unwrap_or_default(),
            "overdue": open && deadline.is_some_and(|d| d < day),
        })
    });
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        settlements,
    })
    .into_response())
}
