//! The production batch screens (06 F&B §7.1 *Production batch*; BL-FNB-11):
//! the list with the plan sheet, and a batch's page with the ingredient lots
//! it used, the lot it made, its wastage and the sheets to start it (pick the
//! ingredient lots), finish it (yield and expiry), discard it (reason) and
//! cancel it while it is planned.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /batches` | `fnb.batches` | `production.view` |
//! | `POST /batches` | `fnb.batch_plan` | `production.manage`, fields `recipe`, `yield`, `outlet` |
//! | `GET /batches/{id}` | `fnb.batch` | `production.view` |
//! | `POST /batches/{id}/start` | `fnb.batch_start` | `production.manage`, fields `lot_<recipe line>` |
//! | `POST /batches/{id}/finish` | `fnb.batch_finish` | `production.manage`, fields `actual_yield`, `expiry` |
//! | `POST /batches/{id}/discard` | `fnb.batch_discard` | `production.manage`, field `reason` |
//! | `POST /batches/{id}/cancel` | `fnb.batch_cancel` | `production.manage` |
//!
//! A refused action answers 422 with the reason (a lot on hold is named).

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::inventory::batch_stock;

use crate::batch::{BatchRecipe, BatchStatus, BatchWastage, ProductionBatch};
use crate::batches::{self, Discarded, FinishAsk, PlanAsk};
use crate::catalogue::Outlet;
use crate::money::{milli_text, parse_milli};
use crate::pages::{Fields, Texts, field, tx};
use crate::permissions;
use crate::placing::refused;
use crate::texts::t;

/// The templates, compiled in.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "fnb/batches.html",
        include_str!("../views/fnb/batches.html"),
    ),
    ("fnb/batch.html", include_str!("../views/fnb/batch.html")),
];

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/batches", index)
        .name("fnb.batches")
        .post("/batches", plan)
        .name("fnb.batch_plan")
        .get("/batches/{id}", show)
        .name("fnb.batch")
        .post("/batches/{id}/start", start)
        .name("fnb.batch_start")
        .post("/batches/{id}/finish", finish)
        .name("fnb.batch_finish")
        .post("/batches/{id}/discard", discard)
        .name("fnb.batch_discard")
        .post("/batches/{id}/cancel", cancel)
        .name("fnb.batch_cancel")
        .require_auth()
}

/// The badge colour of a status.
fn status_kind(status: BatchStatus) -> &'static str {
    match status {
        BatchStatus::Planned => "info",
        BatchStatus::InProduction => "info",
        BatchStatus::Produced | BatchStatus::Consumed => "success",
        BatchStatus::OnHold => "warning",
        BatchStatus::Discarded => "error",
    }
}

/// The word of a status.
fn status_word(lang: &Lang, status: BatchStatus) -> String {
    t(lang, &format!("fnb.screen.batch.status.{}", status.key()))
}

/// One batch of the list.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct BatchRow {
    id: i64,
    number: String,
    recipe: String,
    planned: String,
    actual: String,
    expiry: String,
    status: String,
    status_kind: &'static str,
    url: String,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/batches.html")]
pub(crate) struct BatchesPage {
    tx: Texts,
    batches: Vec<BatchRow>,
    recipes: Vec<[String; 2]>,
    outlets: Vec<[String; 2]>,
    can_manage: bool,
}

async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<Response> {
    if !user.has_permission_anywhere(permissions::PRODUCTION_VIEW) {
        return Err(Error::Forbidden);
    }
    let recipes = BatchRecipe::query()
        .order_by(BatchRecipe::NAME)
        .get(&state.db)
        .await?;
    let mut rows = Vec::new();
    for batch in ProductionBatch::newest(&state.db, 200).await? {
        let recipe = recipes
            .iter()
            .find(|r| r.id == batch.recipe_id)
            .map(|r| r.name.clone())
            .unwrap_or_default();
        rows.push(BatchRow {
            id: batch.id,
            number: batch.number.clone(),
            recipe,
            planned: milli_text(batch.planned_yield_milli),
            actual: batch.actual_yield_milli.map(milli_text).unwrap_or_default(),
            expiry: batch.expiry_date.map(|d| d.to_string()).unwrap_or_default(),
            status: status_word(&lang, batch.status),
            status_kind: status_kind(batch.status),
            url: format!("/batches/{}", batch.id),
        });
    }
    let outlets = Outlet::query()
        .where_(Outlet::ACTIVE.eq(true))
        .order_by(Outlet::NAME)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|o| o.location_id.is_some())
        .map(|o| [o.id.to_string(), o.name])
        .collect();
    Ok(View::typed(BatchesPage {
        tx: tx(&lang),
        batches: rows,
        recipes: recipes
            .into_iter()
            .map(|r| [r.id.to_string(), r.name])
            .collect(),
        outlets,
        can_manage: user.has_permission_anywhere(permissions::PRODUCTION_MANAGE),
    })
    .into_response())
}

/// `POST /batches`: plans a batch.
async fn plan(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Fields,
) -> Result<Response> {
    let planned = field(&form, "yield");
    let ask = PlanAsk {
        recipe_id: field(&form, "recipe").trim().parse().unwrap_or(0),
        planned_yield_milli: if planned.trim().is_empty() {
            0
        } else {
            parse_milli(&planned).ok_or_else(|| refused(t(&lang, "fnb.batch.problem.bad_yield")))?
        },
        outlet_id: field(&form, "outlet").trim().parse().ok(),
    };
    let batch = batches::plan(&state, &user, &ask).await?;
    Ok((
        Toast::success(t(&lang, "fnb.batch.planned")),
        Redirect::route("fnb.batch", &[&batch.id])?,
    )
        .into_response())
}

/// One ingredient lot a batch used.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct InputRow {
    item: String,
    lot: String,
    quantity: String,
    expiry: String,
    status: String,
    status_kind: &'static str,
}

/// One ingredient of the start sheet.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ProposalRow {
    line_id: i64,
    item: String,
    needed: String,
    selected: String,
    options: Vec<[String; 2]>,
    none: bool,
}

/// The wastage entry of a discarded batch.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct WastageRow {
    quantity: String,
    value: i64,
    reason: String,
    posted_by: String,
}

/// What a batch's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/batch.html")]
pub(crate) struct ShowBatchPage {
    tx: Texts,
    batch: ProductionBatch,
    recipe: String,
    status_label: String,
    status_kind: &'static str,
    planned: String,
    actual: String,
    inputs: Vec<InputRow>,
    proposals: Vec<ProposalRow>,
    lot_number: String,
    lot_status: String,
    lot_left: String,
    wastage: Option<WastageRow>,
    notes: Vec<String>,
    can_start: bool,
    can_finish: bool,
    can_discard: bool,
    can_cancel: bool,
    today: String,
}

/// The word of a lot status.
fn lot_word(lang: &Lang, status: &str) -> String {
    t(lang, &format!("fnb.screen.batch.lot_status.{status}"))
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !user.has_permission_anywhere(permissions::PRODUCTION_VIEW) {
        return Err(Error::Forbidden);
    }
    let batch = ProductionBatch::find_or_404(&state.db, id).await?;
    let manager = user.has_permission_anywhere(permissions::PRODUCTION_MANAGE);
    let recipe = BatchRecipe::find(&state.db, batch.recipe_id).await?;
    let mut inputs = Vec::new();
    for input in batch.inputs(&state.db).await? {
        let info = batch_stock(&state).lot_info(&state, &input.lot()).await?;
        let (lot, expiry, status) = match &info {
            Some(info) => (
                info.number.clone(),
                info.expiry_date.map(|d| d.to_string()).unwrap_or_default(),
                info.status.clone(),
            ),
            None => (String::new(), String::new(), String::new()),
        };
        inputs.push(InputRow {
            item: item_code(&state, &batch, input.item_id).await?,
            lot,
            quantity: milli_text(input.quantity_milli),
            expiry,
            status_kind: if status == "on_hold" {
                "warning"
            } else {
                "info"
            },
            status: if status.is_empty() {
                String::new()
            } else {
                lot_word(&lang, &status)
            },
        });
    }
    let mut proposal_rows = Vec::new();
    if batch.status == BatchStatus::Planned {
        for proposal in batches::proposals(&state, &batch).await? {
            let options: Vec<[String; 2]> = proposal
                .choice
                .as_ref()
                .map(|choice| {
                    choice
                        .lots
                        .iter()
                        .map(|lot| {
                            [
                                lot.lot.subject_id.to_string(),
                                format!(
                                    "{} / {} / {}",
                                    lot.number,
                                    lot.expiry_date.map(|d| d.to_string()).unwrap_or_default(),
                                    milli_text(lot.available_milli)
                                ),
                            ]
                        })
                        .collect()
                })
                .unwrap_or_default();
            proposal_rows.push(ProposalRow {
                line_id: proposal.line.id,
                item: proposal.line.item_code.clone(),
                needed: milli_text(proposal.needed_milli),
                selected: options.first().map(|o| o[0].clone()).unwrap_or_default(),
                none: options.is_empty(),
                options,
            });
        }
    }
    let (lot_number, lot_status, lot_left) = match batch.lot() {
        Some(lot) => match batch_stock(&state).lot_info(&state, &lot).await? {
            Some(info) => (
                info.number,
                lot_word(&lang, &info.status),
                milli_text(info.left_milli),
            ),
            None => (String::new(), String::new(), String::new()),
        },
        None => (String::new(), String::new(), String::new()),
    };
    let wastage = BatchWastage::of_batch(&state.db, batch.id)
        .await?
        .map(|w| WastageRow {
            quantity: milli_text(w.quantity_milli),
            value: w.value,
            reason: w.reason,
            posted_by: t(
                &lang,
                &format!("fnb.screen.batch.posted_by.{}", w.posted_by),
            ),
        });
    let mut notes = Vec::new();
    match batch.status {
        BatchStatus::OnHold => notes.push(t(&lang, "fnb.screen.batch.note_on_hold")),
        BatchStatus::Planned if proposal_rows.iter().any(|p| p.none) => {
            notes.push(t(&lang, "fnb.screen.batch.note_no_lot"));
        }
        BatchStatus::Produced => notes.push(t(&lang, "fnb.screen.batch.note_produced")),
        _ => {}
    }
    Ok(View::typed(ShowBatchPage {
        tx: tx(&lang),
        status_label: status_word(&lang, batch.status),
        status_kind: status_kind(batch.status),
        recipe: recipe.map(|r| r.name).unwrap_or_default(),
        planned: milli_text(batch.planned_yield_milli),
        actual: batch.actual_yield_milli.map(milli_text).unwrap_or_default(),
        inputs,
        proposals: proposal_rows,
        lot_number,
        lot_status,
        lot_left,
        wastage,
        notes,
        can_start: manager && batch.status == BatchStatus::Planned,
        can_finish: manager && batch.status == BatchStatus::InProduction,
        can_discard: manager && batch.status == BatchStatus::Produced,
        can_cancel: manager && batch.status == BatchStatus::Planned,
        today: wl_kernel::documents::today().to_string(),
        batch,
    })
    .into_response())
}

/// The code of the stock item `item_id`, from the recipe line that names it.
async fn item_code(state: &AppState, batch: &ProductionBatch, item_id: i64) -> Result<String> {
    let Some(recipe) = BatchRecipe::find(&state.db, batch.recipe_id).await? else {
        return Ok(String::new());
    };
    Ok(recipe
        .lines(&state.db)
        .await?
        .into_iter()
        .find(|line| line.item_id == item_id)
        .map(|line| line.item_code)
        .unwrap_or_default())
}

/// `POST /batches/{id}/start`.
async fn start(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let mut chosen = BTreeMap::new();
    for (name, value) in &form {
        if let Some(line) = name.strip_prefix("lot_")
            && let (Ok(line), Ok(lot)) = (line.parse::<i64>(), value.trim().parse::<i64>())
        {
            chosen.insert(line, lot);
        }
    }
    batches::start(&state, &user, id, &chosen).await?;
    Ok((
        Toast::success(t(&lang, "fnb.batch.started")),
        Redirect::route("fnb.batch", &[&id])?,
    )
        .into_response())
}

/// `POST /batches/{id}/finish`.
async fn finish(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let yielded = parse_milli(&field(&form, "actual_yield"))
        .ok_or_else(|| refused(t(&lang, "fnb.batch.problem.yield")))?;
    let ask = FinishAsk {
        actual_yield_milli: yielded,
        expiry: NaiveDate::parse_from_str(field(&form, "expiry").trim(), "%Y-%m-%d").ok(),
    };
    batches::finish(&state, &user, id, &ask).await?;
    Ok((
        Toast::success(t(&lang, "fnb.batch.finished")),
        Redirect::route("fnb.batch", &[&id])?,
    )
        .into_response())
}

/// `POST /batches/{id}/discard`.
async fn discard(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let key = match batches::discard(&state, &user, id, &field(&form, "reason")).await? {
        Discarded::Done(_) => "fnb.batch.discarded",
        Discarded::Waiting(_) => "fnb.batch.discard_waiting",
    };
    Ok((
        Toast::success(t(&lang, key)),
        Redirect::route("fnb.batch", &[&id])?,
    )
        .into_response())
}

/// `POST /batches/{id}/cancel`.
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    batches::cancel(&state, &user, id).await?;
    Ok((
        Toast::success(t(&lang, "fnb.batch.cancelled")),
        Redirect::route("fnb.batches", &[])?,
    )
        .into_response())
}
