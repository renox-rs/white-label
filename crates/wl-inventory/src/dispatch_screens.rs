//! The dispatch screen (wl-inventory §4 *Dispatch*; base for BL-STK-26): the
//! dispatches with their lines, source and status, and the buttons that pick
//! a draft and cancel a draft or picked one, each with the reason it is
//! disabled in text when the user cannot press it.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /dispatches` | `wl_inventory.dispatches` | `dispatch.manage` or `stock.view` somewhere |
//! | `POST /dispatches/{id}/pick` | `wl_inventory.dispatch_pick` | `dispatch.manage` at the location |
//! | `POST /dispatches/{id}/cancel` | `wl_inventory.dispatch_cancel` | `dispatch.manage` at the location; a reason |
//!
//! The dispatch itself is left (`dispatch`, delivery) to BL-STK-21 and Epic
//! 07; a dispatch that was cancelled by a sales cancellation shows the reason.

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};

use crate::dispatch::{DISPATCH_SUBJECT_TYPE, Dispatch, DispatchStatus, cancel, lines_of, pick};
use crate::item::Item;
use crate::permissions;
use crate::stock_screens::{amount, exported, item_name, label};
use crate::texts::t;

/// The templates, compiled in.
pub(crate) const VIEWS: &[(&str, &str)] = &[(
    "wl-inventory/dispatches.html",
    include_str!("../views/wl-inventory/dispatches.html"),
)];

/// Every text of the dispatch screens and refusals, in English.
pub(crate) const DISPATCH_TEXTS: &[(&str, &str)] = &[
    ("wl_inventory.dispatch.title", "Dispatches"),
    (
        "wl_inventory.dispatch.subtitle",
        "Goods picked and shipped for a sales order. A cancelled order line takes its goods off the dispatch.",
    ),
    ("wl_inventory.dispatch.number", "Dispatch"),
    ("wl_inventory.dispatch.source", "For"),
    ("wl_inventory.dispatch.location", "From"),
    ("wl_inventory.dispatch.lines", "Lines"),
    ("wl_inventory.dispatch.status", "Status"),
    ("wl_inventory.dispatch.note", "Note"),
    ("wl_inventory.dispatch.empty", "No dispatch yet"),
    (
        "wl_inventory.dispatch.empty_hint",
        "A dispatch appears when goods are picked for a sales order.",
    ),
    ("wl_inventory.dispatch.source_type", "Source type"),
    ("wl_inventory.dispatch.when", "Created"),
    ("wl_inventory.dispatch.actions", "Actions"),
    ("wl_inventory.dispatch.pick", "Pick"),
    (
        "wl_inventory.dispatch.pick_description",
        "Pick and pack the lines. The stock is reserved until the dispatch leaves or is cancelled.",
    ),
    ("wl_inventory.dispatch.cancel", "Cancel"),
    (
        "wl_inventory.dispatch.cancel_description",
        "Cancel the dispatch and release the reservation. The goods stay in stock.",
    ),
    ("wl_inventory.dispatch.reason", "Reason"),
    ("wl_inventory.dispatch.picked", "Dispatch picked."),
    ("wl_inventory.dispatch.cancelled", "Dispatch cancelled."),
    (
        "wl_inventory.dispatch.reason_needed",
        "Give the reason for cancelling the dispatch.",
    ),
    (
        "wl_inventory.dispatch.cannot_pick",
        "Only a draft dispatch can be picked.",
    ),
    (
        "wl_inventory.dispatch.cannot_cancel",
        "Only a draft or picked dispatch can be cancelled: after it left, the goods follow the delivery.",
    ),
    (
        "wl_inventory.dispatch.no_permission",
        "You need the dispatch permission at this location.",
    ),
    (
        "wl_inventory.dispatch.no_lines",
        "A dispatch needs at least one line.",
    ),
    (
        "wl_inventory.dispatch.bad_line",
        "A dispatch line needs a known item and a quantity above zero.",
    ),
    (
        "wl_inventory.dispatch.not_draft",
        "Only a draft dispatch can be picked.",
    ),
    ("wl_inventory.dispatch.status_draft", "Draft"),
    ("wl_inventory.dispatch.status_picked", "Picked"),
    ("wl_inventory.dispatch.status_dispatched", "Dispatched"),
    ("wl_inventory.dispatch.status_delivered", "Delivered"),
    ("wl_inventory.dispatch.status_returned", "Returned"),
    ("wl_inventory.dispatch.status_discarded", "Discarded"),
    ("wl_inventory.dispatch.status_lost", "Lost"),
    ("wl_inventory.dispatch.status_cancelled", "Cancelled"),
    (
        "wl_inventory.dispatch.cancel_task_title",
        "{{ dispatch }}: a cancelled order line, goods already dispatched",
    ),
    (
        "wl_inventory.dispatch.cancel_task_body",
        "Sale line {{ line }} of the order was cancelled after {{ dispatch }} left (now {{ status }}). The dispatch is not changed: the goods follow the delivery, returned or refused. Follow it up with the delivery.",
    ),
];

type Texts = BTreeMap<&'static str, String>;

const PREFIX: &str = "wl_inventory.dispatch.";

fn tx(lang: &Lang) -> Texts {
    DISPATCH_TEXTS
        .iter()
        .filter_map(|(key, _)| Some((key.strip_prefix(PREFIX)?, t(lang, key))))
        .collect()
}

fn tr(lang: &Lang, key: &str) -> String {
    t(lang, &format!("{PREFIX}{key}"))
}

/// The routes of the dispatch screen.
pub fn routes() -> Routes {
    Routes::new()
        .get("/dispatches", dispatches)
        .name("wl_inventory.dispatches")
        .post("/dispatches/{id}/pick", pick_dispatch)
        .name("wl_inventory.dispatch_pick")
        .post("/dispatches/{id}/cancel", cancel_dispatch)
        .name("wl_inventory.dispatch_cancel")
        .require_auth()
}

/// What the dispatches page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/dispatches.html")]
pub(crate) struct DispatchesPage {
    tx: Texts,
    rows: renox::grid::GridPage<Dispatch>,
    /// The dispatches the person may pick or cancel (a sheet each).
    sheets: Vec<DispatchSheet>,
}

/// The sheets of the buttons of one dispatch.
#[derive(Serialize, Debug, Clone)]
pub struct DispatchSheet {
    id: i64,
    can_pick: bool,
    can_cancel: bool,
}

/// The dispatches grid: search on the store, the source and the note, filters
/// by status. The lines, the number and the buttons are drawn by the template
/// (`custom`).
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let statuses: Vec<(String, String)> = [
        "draft",
        "picked",
        "dispatched",
        "delivered",
        "returned",
        "discarded",
        "lost",
        "cancelled",
    ]
    .iter()
    .map(|status| ((*status).to_owned(), tr(lang, &format!("status_{status}"))))
    .collect();
    let grid = Grid::new("inventory-dispatches")
        .title(&tr(lang, "title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(Column::custom("number", &tr(lang, "number")).frozen().mobile())
        .column(Column::custom("source", &tr(lang, "source")).mobile())
        .column(
            Column::text("source_type", &tr(lang, "source_type"))
                .hidden()
                .searchable(),
        )
        .column(
            Column::related(
                "location",
                &tr(lang, "location"),
                "wl_inventory_locations",
                "location_id",
                "name_en",
            )
            .searchable(),
        )
        .column(Column::custom("lines", &tr(lang, "lines")))
        .column(
            Column::select("status", &tr(lang, "status"), statuses)
                .badges(&[
                    ("draft", "info"),
                    ("picked", "warning"),
                    ("dispatched", "warning"),
                    ("delivered", "success"),
                    ("returned", "error"),
                    ("discarded", "error"),
                    ("lost", "error"),
                    ("cancelled", "error"),
                ])
                .mobile(),
        )
        .column(Column::text("cancel_reason", &tr(lang, "note")).searchable())
        .column(Column::datetime("created_at", &tr(lang, "when")).hidden())
        .column(Column::custom("actions", &tr(lang, "actions")))
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(&tr(lang, "empty"), Some(&tr(lang, "empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// `GET /dispatches`.
async fn dispatches(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    if !user.has_permission_anywhere(permissions::DISPATCH_MANAGE)
        && !user.has_permission_anywhere(permissions::STOCK_VIEW)
    {
        return Err(Error::Forbidden);
    }
    let can_export = user.has_permission_anywhere(permissions::DISPATCH_MANAGE);
    let grid = grid(&lang, can_export);
    let query = Dispatch::query();
    if let Some(file) = exported(&grid, &query, &request, can_export).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra = std::collections::HashMap::new();
    let mut sheets = Vec::new();
    for dispatch in page.items() {
        let mut lines = Vec::new();
        for line in lines_of(&state.db, dispatch.id).await? {
            let item = Item::find_or_404(&state.db, line.item_id).await?;
            lines.push(format!(
                "{} · {}",
                item_name(&item, &lang),
                amount(line.quantity_milli, &item.base_unit)
            ));
        }
        let allowed = user.has_permission_in(permissions::DISPATCH_MANAGE, &dispatch.scope());
        let can_pick = allowed && dispatch.status == DispatchStatus::Draft;
        let can_cancel = allowed && dispatch.status.in_store();
        // Why the buttons are off, as text.
        let disabled_reason = if !allowed {
            tr(&lang, "no_permission")
        } else if !dispatch.status.in_store() {
            tr(&lang, "cannot_cancel")
        } else {
            String::new()
        };
        let source = label(&state, &dispatch.source()).await;
        if can_pick || can_cancel {
            sheets.push(DispatchSheet {
                id: dispatch.id,
                can_pick,
                can_cancel,
            });
        }
        extra.insert(
            dispatch.id,
            json!({
                "number": dispatch.number(),
                "source": if source.is_empty() {
                    format!("{}:{}", dispatch.source_type, dispatch.source_id)
                } else {
                    source
                },
                "lines": if lines.is_empty() { "-".to_owned() } else { lines.join(", ") },
                "cancel_reason": dispatch
                    .cancel_reason
                    .clone()
                    .filter(|reason| !reason.is_empty())
                    .unwrap_or(disabled_reason),
                "can_pick": can_pick,
                "can_cancel": can_cancel,
            }),
        );
    }
    let page = page.extend(|dispatch| extra.get(&dispatch.id).cloned().unwrap_or_else(|| json!({})));
    Ok(View::typed(DispatchesPage {
        tx: tx(&lang),
        rows: page,
        sheets,
    })
    .into_response())
}

fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// `POST /dispatches/{id}/pick`.
async fn pick_dispatch(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    match pick(&state, &user, id).await {
        Ok(_) => Ok((Toast::success(tr(&lang, "picked")), renox::HxRefresh).into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(TransitionError::NotFrom(_)) => Err(refused(tr(&lang, "cannot_pick"))),
        Err(other) => Err(other.into_error()),
    }
}

/// What a user enters to cancel a dispatch.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct CancelForm {
    /// Why.
    #[validate(max = 500)]
    pub reason: Option<String>,
}

/// `POST /dispatches/{id}/cancel`.
async fn cancel_dispatch(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<CancelForm>,
) -> Result<Response> {
    let reason = form.reason.unwrap_or_default();
    if reason.trim().is_empty() {
        return Err(refused(tr(&lang, "reason_needed")));
    }
    match cancel(&state, &user, id, reason.trim()).await {
        Ok(_) => Ok((Toast::success(tr(&lang, "cancelled")), renox::HxRefresh).into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(TransitionError::NotFrom(_)) => Err(refused(tr(&lang, "cannot_cancel"))),
        Err(other) => Err(other.into_error()),
    }
}

/// The number as the label of a dispatch (`DSP-0007`).
pub(crate) fn register_labels() {
    wl_kernel::documents::register_subject_label(DISPATCH_SUBJECT_TYPE, |db, id| {
        Box::pin(async move { Ok(Dispatch::find(db, id).await?.map(|row| row.number())) })
    });
}
