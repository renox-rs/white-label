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

use renox::prelude::*;
use serde::Serialize;

use crate::pages::{Texts, tx};
use crate::permissions;
use crate::rates::aed;
use crate::reversal::{self, Reversal};
use crate::shipment::Shipment;
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

/// One task of the list.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Row {
    id: i64,
    shipment_id: i64,
    number: String,
    reason: String,
    amount: String,
    driver: String,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "logistics/reversals.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: Vec<Row>,
}

/// `GET /reversals`: the tasks waiting for the operations manager.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<Response> {
    if !user.has_permission(permissions::SHIPMENTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let mut rows = Vec::new();
    for reversal in reversal::open_tasks(&state.db).await? {
        let shipment = Shipment::find_or_404(&state.db, reversal.shipment_id).await?;
        rows.push(row_of(&reversal, &shipment));
    }
    Ok(View::typed(IndexPage {
        tx: tx(&lang),
        rows,
    })
    .into_response())
}

fn row_of(reversal: &Reversal, shipment: &Shipment) -> Row {
    Row {
        id: reversal.id,
        shipment_id: shipment.id,
        number: shipment.number.clone(),
        reason: reversal.reason.clone(),
        amount: aed(reversal.cash_amount),
        driver: reversal
            .driver_user_id
            .map(|driver| format!("#{driver}"))
            .unwrap_or_default(),
    }
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
