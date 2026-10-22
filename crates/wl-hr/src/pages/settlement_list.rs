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

use renox::prelude::*;
use serde::Serialize;
use wl_kernel::documents::today;

use crate::config::config_of;
use crate::employee::Employee;
use crate::pages::payroll::{aed_text, date_text};
use crate::pages::settlements::{kind, reason_text};
use crate::settlement::{FinalSettlement, SettlementStatus};
use crate::settlement_payment::paid_total;
use crate::{permissions, texts};

type Texts = BTreeMap<&'static str, String>;

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/hr/settlements", index)
        .name("wl_hr.settlements")
        .require_auth()
}

/// One settlement of the list.
#[derive(Serialize, Debug, Clone)]
pub struct SettlementRow {
    /// The primary key.
    pub id: i64,
    /// Its page.
    pub url: String,
    /// The employee's name.
    pub employee: String,
    /// The reason in words.
    pub reason: String,
    /// The status in words.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// The dues.
    pub dues: String,
    /// The recoveries.
    pub recoveries: String,
    /// The net.
    pub net: String,
    /// What was paid.
    pub paid: String,
    /// The legal deadline, or empty without a last working day.
    pub deadline: String,
    /// Whether the deadline has passed and the settlement is not paid.
    pub overdue: bool,
}

/// The list page.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/settlements.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    settlements: Vec<SettlementRow>,
}

/// `GET /hr/settlements`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
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
    let days = config_of(&state).settlement_deadline_days;
    let mut rows = Vec::new();
    for s in FinalSettlement::query()
        .order_by_desc(FinalSettlement::ID)
        .get(&state.db)
        .await?
    {
        let employee = match Employee::find(&state.db, s.employee_id).await? {
            Some(e) => crate::pages::employees::employee_name(&state, &lang, &e).await,
            None => format!("#{}", s.employee_id),
        };
        let deadline = s.deadline(days);
        let open = matches!(
            s.status,
            SettlementStatus::Draft | SettlementStatus::Approved | SettlementStatus::Disputed
        );
        rows.push(SettlementRow {
            id: s.id,
            url: format!("/hr/settlements/{}", s.id),
            employee,
            reason: reason_text(&s.reason),
            status: texts::t(
                &lang,
                &format!("wl_hr.settlement.status.{}", s.status.as_str()),
            ),
            status_kind: kind(s.status),
            dues: aed_text(s.dues()),
            recoveries: aed_text(s.recoveries_total()),
            net: aed_text(s.net),
            paid: aed_text(paid_total(&state.db, s.id).await?),
            deadline: deadline.map(date_text).unwrap_or_default(),
            overdue: open && deadline.is_some_and(|d| d < today()),
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        settlements: rows,
    }))
}
