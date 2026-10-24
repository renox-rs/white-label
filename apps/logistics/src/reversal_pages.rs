//! The operations manager's tasks for a reversed delivery whose driver is
//! liable (03 Logistics §4.1; BL-LOG-10).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /reversals` | `logistics.reversals` | `shipments.manage` |
//! | `POST /reversals/{id}/raise-shortage` | `logistics.reversal_shortage` | `shipments.manage` (and `cash.count` in payments) |
//! | `POST /reversals/{id}/hr-case` | `logistics.reversal_hr_case` | `shipments.manage` |
//!
//! A successful action answers a redirect (303) with a toast.

use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use serde::Serialize;

use crate::pages::{Texts, tx};
use crate::permissions;
use crate::reversal::{self, Reversal};
use crate::texts::t;

/// The templates, compiled in.
pub(crate) const VIEWS: &[(&str, &str)] = &[(
    "logistics/reversals.html",
    include_str!("../views/logistics/reversals.html"),
)];

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/reversals", index)
        .name("logistics.reversals")
        .post("/reversals/{id}/raise-shortage", raise_shortage)
        .name("logistics.reversal_shortage")
        .post("/reversals/{id}/hr-case", hr_case)
        .name("logistics.reversal_hr_case")
        .require_auth()
}

/// The tasks grid: the open reversals with the shipment, cash and driver. The
/// two actions are drawn by the template (`custom`).
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let text = |key: &str| crate::pages::text(lang, key);
    let grid = Grid::new("logistics-reversals")
        .title(&text("rev_title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::related(
                "number",
                &text("number"),
                "logistics_shipments",
                "shipment_id",
                "number",
            )
            .frozen()
            .mobile()
            .searchable(),
        )
        .column(
            Column::money("cash_amount", &wl_kernel::money_label(&text("rev_amount")))
                .mobile()
                .summary(Summary::Sum),
        )
        .column(Column::number("driver_user_id", &text("rev_driver")).mobile())
        .column(Column::text("reason", &text("reason")).searchable())
        .column(Column::custom("actions", &text("actions")).mobile())
        .sort_by("id")
        .row_url("/shipments/{shipment_id}")
        .cards_on_mobile()
        .empty_state(&text("rev_empty"), Some(&text("rev_empty_text")));
    if can_export { grid.exports() } else { grid }
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "logistics/reversals.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: GridPage<Reversal>,
}

/// `GET /reversals`: the tasks waiting for the operations manager.
async fn index(user: AuthUser, lang: Lang, request: GridRequest) -> Result<Response> {
    if !user.has_permission(permissions::SHIPMENTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let grid = grid(&lang, true);
    let query = Reversal::query().where_(Reversal::TASK_STATUS.eq(reversal::TASK_OPEN));
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let rows = grid.page(query, &request).await?;
    Ok(View::typed(IndexPage {
        tx: tx(&lang),
        rows,
    })
    .into_response())
}

fn back(lang: &Lang, key: &str) -> Result<Response> {
    Ok((
        Toast::success(t(lang, key)),
        Redirect::route("logistics.reversals", &[])?,
    )
        .into_response())
}

/// `POST /reversals/{id}/raise-shortage`: a later shortage against the driver.
async fn raise_shortage(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    reversal::raise_shortage(&state, &user, id).await?;
    back(&lang, "logistics.reversal_shortage_raised")
}

/// `POST /reversals/{id}/hr-case`: an HR case for the driver.
async fn hr_case(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    reversal::open_hr_case(&state, &user, id).await?;
    back(&lang, "logistics.reversal_hr_opened")
}
