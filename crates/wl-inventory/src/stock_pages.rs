//! The actions on stock (wl-inventory §4 *Stock hold*, *Transfer*):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /stock-holds` | `wl_inventory.hold_place` | `stock.hold` (at the location, for a hold on a location) |
//! | `POST /stock-hold-lines/{id}/release` | `wl_inventory.hold_line_release` | `lots.release` at the location; a second person for a listed reason |
//! | `POST /stock-hold-lines/{id}/discard` | `wl_inventory.hold_line_discard` | `lots.release` at the location; a second person above the value |
//! | `POST /stock-hold-lines/{id}/return-to-supplier` | `wl_inventory.hold_line_return` | `lots.release` at the location |
//! | `POST /stock-holds/{id}/cancel` | `wl_inventory.hold_cancel` | `lots.release` in the hold's scope |
//! | `POST /recalls` | `wl_inventory.recall_open` | `recalls.manage` |
//! | `POST /stock-issues` | `wl_inventory.stock_issue` | `stock.issue` at the location |
//! | `GET /pending-issues` | `wl_inventory.pending_list` | `stock.adjust` (the pending issues at the locations in its scope); a page for a browser ([`crate::stock_screens`]), JSON for the rest |
//! | `POST /pending-issues/{id}/clear` | `wl_inventory.pending_clear` | `stock.adjust` at the location |
//! | `POST /transfers/{id}/send` | `wl_inventory.transfer_send` | `stock.transfer` at the source |
//! | `POST /transfers/{id}/receive` | `wl_inventory.transfer_receive` | `stock.transfer` at the destination |
//!
//! Receiving stock into a location that is on hold warns first ("Fridge 2 is
//! on temperature hold"): the answer is a 422 with the warning until the
//! form says `held_ok=1`; then the transfer is received and the quantity
//! joins the hold ([`crate::arrival`]).
//!
//! The routes need a signed-in user and carry no permission guard of their
//! own: the guard counts global roles only, while a storekeeper may hold the
//! permission at one site store. The handlers and the transitions check it in
//! the scope of the location.

use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::hold_decide::{DecideOutcome, Decision, cancel, decide};
use crate::hold_service::{NewHold, NewScope, place_as, warnings_for};
use crate::permissions;
use crate::recall::{NewRecall, RecallSource, ScopeEntry};
use crate::texts;
use crate::transfer::{Transfer, TransferStatus, lines_of};

/// The routes of this file.
pub fn routes() -> Routes {
    Routes::new()
        .post("/stock-holds", place_hold)
        .name("wl_inventory.hold_place")
        .post("/stock-hold-lines/{id}/release", release_line)
        .name("wl_inventory.hold_line_release")
        .post("/stock-hold-lines/{id}/discard", discard_line)
        .name("wl_inventory.hold_line_discard")
        .post("/stock-hold-lines/{id}/return-to-supplier", return_line)
        .name("wl_inventory.hold_line_return")
        .post("/stock-holds/{id}/cancel", cancel_hold)
        .name("wl_inventory.hold_cancel")
        .post("/recalls", open_recall)
        .name("wl_inventory.recall_open")
        .post("/stock-issues", issue_stock)
        .name("wl_inventory.stock_issue")
        .get("/pending-issues", list_pending_issues)
        .name("wl_inventory.pending_list")
        .post("/pending-issues/{id}/clear", clear_pending_issue)
        .name("wl_inventory.pending_clear")
        .post("/transfers/{id}/send", send_transfer)
        .name("wl_inventory.transfer_send")
        .post("/transfers/{id}/receive", receive_transfer)
        .name("wl_inventory.transfer_receive")
        .require_auth()
}

/// The package text `key`.
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_inventory.{key}"))
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The ids in `text` (`3, 4`); `None` when one is not a number.
fn ids(text: &Option<String>) -> Option<Vec<i64>> {
    let mut found = Vec::new();
    for part in text
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        found.push(part.parse().ok()?);
    }
    Some(found)
}

/// What a user enters to place a hold.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct HoldForm {
    /// `recall`, `quality`, `temperature`, `complaint` or `other`.
    #[validate(max = 32)]
    pub reason: Option<String>,
    /// The record that caused it, like `ticketing.ticket:5`.
    #[validate(max = 100)]
    pub reference: Option<String>,
    /// The ids of the lots to hold, like `4, 5`: a hold on whole lots.
    #[validate(max = 200)]
    pub lots: Option<String>,
    /// The id of the location to hold: a hold on a location.
    #[validate(max = 20)]
    pub location: Option<String>,
    /// The ids of the items held at the location; all of them when empty.
    #[validate(max = 200)]
    pub items: Option<String>,
}

/// `POST /stock-holds`: places a hold on whole lots or on a location.
async fn place_hold(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<HoldForm>,
) -> Result<Response> {
    let bad = || refused(text(&lang, "hold.bad_form"));
    let reference = form
        .reference
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(bad)?
        .parse::<Subject>()
        .map_err(|_| bad())?;
    let lots = ids(&form.lots).ok_or_else(bad)?;
    let items = ids(&form.items).ok_or_else(bad)?;
    let location = form
        .location
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.parse::<i64>().map_err(|_| bad()))
        .transpose()?;
    let scope = match (lots.is_empty(), location) {
        (false, None) => NewScope::Lots(lots),
        (true, Some(location_id)) => NewScope::Location {
            location_id,
            item_ids: items,
        },
        _ => return Err(bad()),
    };
    place_as(
        &state,
        &user,
        NewHold {
            reason: form.reason.clone().unwrap_or_default(),
            reference,
            scope,
            by: None,
        },
    )
    .await?;
    Ok((Toast::success(text(&lang, "hold.placed")), renox::HxRefresh).into_response())
}

/// Decides line `id` and answers with the toast: decided, or waiting for a
/// second person.
async fn decide_line(
    state: &AppState,
    user: &AuthUser,
    texts: (String, String),
    id: i64,
    decision: Decision,
) -> Result<Response> {
    let (decided, waiting) = texts;
    match decide(state, user, id, decision).await {
        Ok(DecideOutcome::Decided(_)) => {
            Ok((Toast::success(decided), renox::HxRefresh).into_response())
        }
        Ok(DecideOutcome::Waiting(_)) => {
            Ok((Toast::success(waiting), renox::HxRefresh).into_response())
        }
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}

/// The toasts of a decision: decided, and waiting for a second person.
fn line_texts(lang: &Lang) -> (String, String) {
    (text(lang, "hold.decided"), text(lang, "hold.waiting"))
}

/// `POST /stock-hold-lines/{id}/release`: the quantity is available again;
/// for a listed reason (`recall`) a second person approves first.
async fn release_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    decide_line(&state, &user, line_texts(&lang), id, Decision::Release).await
}

/// `POST /stock-hold-lines/{id}/discard`: the quantity is wasted; above the
/// wastage value a second person approves first.
async fn discard_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    decide_line(&state, &user, line_texts(&lang), id, Decision::Discard).await
}

/// `POST /stock-hold-lines/{id}/return-to-supplier`: the quantity goes to
/// quarantine and onto a draft return to the lot's supplier.
async fn return_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    decide_line(
        &state,
        &user,
        line_texts(&lang),
        id,
        Decision::ReturnToSupplier,
    )
    .await
}

/// What a user enters to cancel a hold.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct CancelForm {
    /// Why; *placed in error* when empty.
    #[validate(max = 200)]
    pub reason: Option<String>,
}

/// `POST /stock-holds/{id}/cancel`: cancels a hold placed in error, before
/// any line is decided; never a recall's.
async fn cancel_hold(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<CancelForm>,
) -> Result<Response> {
    let reason = form
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .map_or_else(|| text(&lang, "hold.cancel_reason"), str::to_owned);
    match cancel(&state, &user, id, &reason).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "hold.cancelled")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}

/// The non-empty, trimmed parts of the comma-separated `text`.
fn words(text: &Option<String>) -> Vec<String> {
    text.as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// What a user enters to open a recall.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct RecallForm {
    /// `supplier`, `authority`, `internal` or `complaint`.
    #[validate(max = 32)]
    pub source: Option<String>,
    /// Why.
    #[validate(max = 500)]
    pub reason: Option<String>,
    /// The id of the supplier behind a supplier recall, when known.
    #[validate(max = 20)]
    pub supplier: Option<String>,
    /// The ids of the items recalled, like `4, 5`.
    #[validate(max = 200)]
    pub items: Option<String>,
    /// The lot numbers recalled, like `L9, L10`, of each item named.
    #[validate(max = 500)]
    pub lot_numbers: Option<String>,
    /// The supplier lot numbers recalled, of each item named.
    #[validate(max = 500)]
    pub supplier_lots: Option<String>,
}

/// `POST /recalls`: opens a recall and holds every lot in its scope at every
/// location, in transit too.
async fn open_recall(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<RecallForm>,
) -> Result<Response> {
    let bad = || refused(text(&lang, "recall.bad_form"));
    let source = RecallSource::parse(form.source.as_deref().unwrap_or_default())
        .ok_or_else(|| refused(text(&lang, "recall.bad_source")))?;
    let items = ids(&form.items).ok_or_else(bad)?;
    let mut scope = Vec::new();
    for item_id in items {
        for number in words(&form.lot_numbers) {
            scope.push(ScopeEntry::lot(item_id, &number));
        }
        for number in words(&form.supplier_lots) {
            scope.push(ScopeEntry::supplier_lot(item_id, &number));
        }
    }
    let supplier_party_id = id_of(&form.supplier)?;
    crate::recall::open(
        &state,
        &user,
        NewRecall {
            source,
            reason: form.reason.clone().unwrap_or_default(),
            supplier_party_id,
            scope,
        },
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "recall.opened")),
        renox::HxRefresh,
    )
        .into_response())
}

/// What a user enters to issue stock.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct IssueForm {
    /// The id of the item.
    #[validate(max = 20)]
    pub item: Option<String>,
    /// The id of the location.
    #[validate(max = 20)]
    pub location: Option<String>,
    /// The quantity in the item's base unit, like `5` or `0.5`.
    #[validate(max = 32)]
    pub quantity: Option<String>,
    /// The id of the lot, when it matters which one.
    #[validate(max = 20)]
    pub lot: Option<String>,
    /// The cost object, like `construction.cost_code:3`.
    #[validate(max = 100)]
    pub cost_object: Option<String>,
    /// Why.
    #[validate(max = 200)]
    pub reason: Option<String>,
}

/// The id in `field`, `None` when empty.
fn id_of(field: &Option<String>) -> Result<Option<i64>> {
    match field.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(value) => value
            .parse()
            .map(Some)
            .map_err(|_| refused("an id must be a number")),
    }
}

/// `POST /stock-issues`: issues stock to a cost object. Refused when the
/// quantity is short or part of it is on hold.
async fn issue_stock(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<IssueForm>,
) -> Result<Response> {
    let item_id = id_of(&form.item)?.ok_or_else(|| refused("item: required"))?;
    let location_id = id_of(&form.location)?.ok_or_else(|| refused("location: required"))?;
    let quantity_milli = crate::pages::parse_quantity(form.quantity.as_deref().unwrap_or_default())
        .ok_or_else(|| refused("quantity: not a quantity"))?;
    let cost_object = match form
        .cost_object
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        None => None,
        Some(value) => Some(
            value
                .parse::<Subject>()
                .map_err(|_| refused("cost_object: like construction.cost_code:3"))?,
        ),
    };
    crate::stock_service::issue(
        &state,
        &user,
        crate::stock_service::IssueRequest {
            item_id,
            location_id,
            quantity_milli,
            lot_id: id_of(&form.lot)?,
            cost_object,
            reason: form.reason.clone().filter(|r| !r.trim().is_empty()),
        },
    )
    .await?;
    Ok((Toast::success(text(&lang, "issue.done")), renox::HxRefresh).into_response())
}

/// What the storekeeper enters to clear a pending issue.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ClearForm {
    /// Why the stock is entered: it was used but never on the books.
    #[validate(max = 200)]
    pub reason: Option<String>,
    /// The id of the lot to enter the stock in, for a lot-tracked item.
    #[validate(max = 20)]
    pub lot: Option<String>,
}

/// One open pending issue as the storekeeper's page lists it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PendingRow {
    /// The pending issue's id (the clear form posts to it).
    pub id: i64,
    /// The item's code.
    pub item: String,
    /// The location's code.
    pub location: String,
    /// The short quantity, in thousandths.
    pub quantity_milli: i64,
    /// The record that consumed, like `clinics.visit:12`.
    pub source: String,
    /// When it was recorded.
    pub recorded_at: String,
}

/// `GET /pending-issues`: the open pending issues, oldest first, at the
/// locations where the user holds `stock.adjust`, as JSON. Decision: the
/// storekeeper's page lists only what the storekeeper may clear.
async fn list_pending_issues(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: renox::axum::http::HeaderMap,
    request: renox::grid::GridRequest,
) -> Result<Response> {
    if !user.has_permission_anywhere(permissions::STOCK_ADJUST) {
        return Err(Error::Forbidden);
    }
    // A browser gets the page; everything else gets the JSON.
    if wl_kernel::negotiate::wants_html(&headers) {
        return crate::stock_screens::pending_page(&state, &user, &lang, request).await;
    }
    let mut rows = Vec::new();
    for pending in crate::pending_issue::open_of(&state.db).await? {
        if !user.has_permission_in(permissions::STOCK_ADJUST, &pending.scope()) {
            continue;
        }
        let item = crate::item::Item::find_or_404(&state.db, pending.item_id).await?;
        let location =
            crate::location::Location::find_or_404(&state.db, pending.location_id).await?;
        rows.push(PendingRow {
            id: pending.id,
            item: item.code.clone(),
            location: location.code.clone(),
            quantity_milli: pending.quantity_milli,
            source: format!("{}:{}", pending.source_type, pending.source_id),
            recorded_at: pending.recorded_at.to_string(),
        });
    }
    let body = renox::serde_json::to_string(&rows)
        .map_err(|error| Error::Internal(renox::anyhow::Error::new(error)))?;
    Ok(([("content-type", "application/json")], body).into_response())
}

/// `POST /pending-issues/{id}/clear`: clears a pending issue with a reason.
/// Above the `STK.ACL.02` value it asks a second person to approve instead.
async fn clear_pending_issue(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ClearForm>,
) -> Result<Response> {
    let reason = form.reason.clone().unwrap_or_default();
    if reason.trim().is_empty() {
        return Err(refused(text(&lang, "pending.reason_needed")));
    }
    let lot_id = id_of(&form.lot)?;
    match crate::pending_issue::clear(&state, &user, id, &reason, lot_id).await {
        Ok(crate::pending_issue::ClearOutcome::Cleared(_)) => Ok((
            Toast::success(text(&lang, "pending.cleared")),
            renox::HxRefresh,
        )
            .into_response()),
        Ok(crate::pending_issue::ClearOutcome::Waiting(_)) => Ok((
            Toast::success(text(&lang, "pending.waiting")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}

/// `POST /transfers/{id}/send`: sends a draft transfer. Refused with the
/// first problem: stock not available, a lot not released, held quantity.
async fn send_transfer(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    match crate::transfer::send(&state, &user, id).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "transfer.sent")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}

/// What the receiver enters.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ReceiveForm {
    /// `1` to go on after the warning that the destination is on hold.
    #[validate(max = 8)]
    pub held_ok: Option<String>,
}

/// `POST /transfers/{id}/receive`: receives an in-transit transfer. When the
/// destination is on hold the answer is the warning until `held_ok=1`.
async fn receive_transfer(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReceiveForm>,
) -> Result<Response> {
    let transfer = Transfer::find_or_404(&state.db, id).await?;
    if !user.has_permission_in(permissions::STOCK_TRANSFER, &transfer.to_scope()) {
        return Err(Error::Forbidden);
    }
    if transfer.status == TransferStatus::InTransit && form.held_ok.as_deref() != Some("1") {
        let items: Vec<i64> = lines_of(&state.db, id)
            .await?
            .iter()
            .map(|line| line.item_id)
            .collect();
        let warnings = warnings_for(&state.db, transfer.to_location_id, &items).await?;
        if !warnings.is_empty() {
            return Err(refused(format!(
                "{} {}",
                warnings.join(" "),
                text(&lang, "hold.go_on")
            )));
        }
    }
    match crate::transfer::receive(&state, &user, id).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "transfer.received")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}
