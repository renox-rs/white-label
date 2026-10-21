//! The staff pages and actions on shipments (03 Logistics §7.1 *Shipment*).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /shipments` | `logistics.shipments` | `shipments.view` |
//! | `GET /shipments/{id}` | `logistics.shipment` | `shipments.view` |
//! | `POST /shipments` | `logistics.shipment_book` | `shipments.book` |
//! | `POST /shipments/import` | `logistics.shipment_import` | `shipments.book` |
//! | `POST /shipments/{id}/pickup` | `logistics.shipment_pickup` | `shipments.scan` |
//! | `POST /shipments/{id}/hub` | `logistics.shipment_hub` | `shipments.scan` |
//! | `POST /scans` | `logistics.scan` | `shipments.scan` |
//! | `POST /shipments/{id}/void` | `logistics.shipment_void` | `shipments.void` |
//! | `POST /shipments/{id}/cancel` | `logistics.shipment_cancel` | `shipments.cancel` |
//! | `POST /shipments/{id}/instruction` | `logistics.shipment_instruction` | `shipments.manage` |
//! | `POST /shipments/{id}/cancel-return` | `logistics.shipment_cancel_return` | `shipments.manage` |
//! | `POST /shipments/{id}/returned` | `logistics.shipment_returned` | `shipments.deliver` |
//! | `POST /shipments/{id}/reweigh` | `logistics.shipment_reweigh` | `shipments.scan` or `.manage` |
//! | `POST /shipments/{id}/reweigh-dispute` | `logistics.shipment_reweigh_dispute` | `shipments.book` (own company) or `.manage` |
//! | `POST /shipments/{id}/reweigh-dispute/resolve` | `logistics.shipment_reweigh_resolve` | `shipments.manage` |
//!
//! A successful action answers a redirect (303) with a toast. The import with
//! `dry_run` answers the report as JSON and books nothing. A refused action
//! answers 422 with the reason (403 without the permission).

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::Serialize;
use wl_kernel::voids::{self, VoidOutcome};
use wl_kernel::{PartyId, Subject};

use crate::booking::{self, NewShipment};
use crate::cancel;
use crate::permissions;
use crate::reweigh::{self, NewMeasurement};
use crate::scan;
use crate::screen_texts::SCREEN_TEXTS;
use crate::shipment::{SUBJECT_TYPE, Shipment, ShipmentStatus};
use crate::texts::t;

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "logistics/shipments.html",
        include_str!("../views/logistics/shipments.html"),
    ),
    (
        "logistics/shipment.html",
        include_str!("../views/logistics/shipment.html"),
    ),
];

/// The texts a template reads as `tx['title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

const PREFIX: &str = "logistics.screen.";

/// Every text of the screens, by the name a template reads.
pub(crate) fn tx(lang: &Lang) -> Texts {
    SCREEN_TEXTS
        .iter()
        .filter_map(|(key, _)| Some((key.strip_prefix(PREFIX)?, t(lang, key))))
        .collect()
}

/// The word of a status.
fn status_word(lang: &Lang, status: ShipmentStatus) -> String {
    t(
        lang,
        &format!("logistics.screen.status.{}", status.as_str()),
    )
}

/// The badge colour of a status.
fn status_kind(status: ShipmentStatus) -> &'static str {
    use ShipmentStatus::{
        AtHub, Attempted, Cancelled, Created, Delivered, Disposed, Lost, OnHold, OutForDelivery,
        PickedUp, Returned, Returning, UnderReview, Voided,
    };
    match status {
        Delivered => "success",
        PickedUp | AtHub | OutForDelivery => "info",
        Attempted | OnHold | Returning | UnderReview => "warning",
        Lost => "error",
        Created | Returned | Disposed | Cancelled | Voided => "info",
    }
}

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/shipments", index)
        .name("logistics.shipments")
        .post("/shipments", book)
        .name("logistics.shipment_book")
        .post("/shipments/import", import)
        .name("logistics.shipment_import")
        .get("/shipments/{id}", show)
        .name("logistics.shipment")
        .post("/shipments/{id}/pickup", pickup)
        .name("logistics.shipment_pickup")
        .post("/shipments/{id}/hub", hub)
        .name("logistics.shipment_hub")
        .post("/scans", scan_label)
        .name("logistics.scan")
        .post("/shipments/{id}/void", void)
        .name("logistics.shipment_void")
        .post("/shipments/{id}/cancel", cancel_shipment)
        .name("logistics.shipment_cancel")
        .post("/shipments/{id}/instruction", record_instruction)
        .name("logistics.shipment_instruction")
        .post("/shipments/{id}/cancel-return", cancel_return)
        .name("logistics.shipment_cancel_return")
        .post("/shipments/{id}/returned", returned)
        .name("logistics.shipment_returned")
        .post("/shipments/{id}/reweigh", reweigh)
        .name("logistics.shipment_reweigh")
        .post("/shipments/{id}/reweigh-dispute", reweigh_dispute)
        .name("logistics.shipment_reweigh_dispute")
        .post("/shipments/{id}/reweigh-dispute/resolve", reweigh_resolve)
        .name("logistics.shipment_reweigh_resolve")
        .require_auth()
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

fn answer(error: TransitionError) -> Error {
    error.into_error()
}

type Fields = Form<BTreeMap<String, String>>;

fn field(form: &BTreeMap<String, String>, name: &str) -> String {
    form.get(name).cloned().unwrap_or_default()
}

/// A redirect to the shipment's page with a toast.
fn back_to(lang: &Lang, key: &str, id: i64) -> Result<Response> {
    Ok((
        Toast::success(t(lang, key)),
        Redirect::route("logistics.shipment", &[&id])?,
    )
        .into_response())
}

/// One shipment of the list.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Row {
    id: i64,
    number: String,
    client_reference: String,
    client_party_id: i64,
    recipient: String,
    area: String,
    parcels: i64,
    cod_amount: i64,
    status: String,
    status_kind: &'static str,
    url: String,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "logistics/shipments.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    shipments: Vec<Row>,
}

async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<Response> {
    if !user.has_permission(permissions::SHIPMENTS_VIEW) {
        return Err(Error::Forbidden);
    }
    let shipments = Shipment::query()
        .order_by_desc(Shipment::ID)
        .limit(200)
        .get(&state.db)
        .await?;
    let shipments = shipments
        .iter()
        .map(|s| Row {
            id: s.id,
            number: s.number.clone(),
            client_reference: s.client_reference.clone().unwrap_or_default(),
            client_party_id: s.client_party_id,
            recipient: s.recipient_name.clone(),
            area: s.area.clone(),
            parcels: s.parcel_count,
            cod_amount: s.cod_amount,
            status: status_word(&lang, s.status),
            status_kind: status_kind(s.status),
            url: format!("/shipments/{}", s.id),
        })
        .collect();
    Ok(View::typed(IndexPage {
        tx: tx(&lang),
        shipments,
    })
    .into_response())
}

/// A parcel of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ParcelRow {
    barcode: String,
    weight_grams: i64,
    state: String,
}

/// A tracking event of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct EventRow {
    kind: String,
    note: String,
    at: String,
}

/// A charge of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ChargeRow {
    kind: String,
    amount: i64,
    note: String,
}

/// What the shipment's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "logistics/shipment.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    shipment: Shipment,
    status_label: String,
    status_kind: &'static str,
    parcels: Vec<ParcelRow>,
    events: Vec<EventRow>,
    charges: Vec<ChargeRow>,
    delivery_url: String,
    can_pickup: bool,
    can_hub: bool,
    can_void: bool,
    can_cancel: bool,
    can_cancel_return: bool,
    can_return: bool,
    can_reweigh: bool,
    can_dispute: bool,
    can_resolve_dispute: bool,
    charged_price: i64,
    outcomes: Vec<[String; 2]>,
    instruction: String,
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !user.has_permission(permissions::SHIPMENTS_VIEW) {
        return Err(Error::Forbidden);
    }
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    let offered: Vec<String> = shipment
        .available(&user, &state.db)
        .await?
        .into_iter()
        .map(|action| action.name)
        .collect();
    let has = |name: &str| offered.iter().any(|offered| offered == name);
    let manages = user.has_permission(permissions::SHIPMENTS_MANAGE);
    let scans = user.has_permission(permissions::SHIPMENTS_SCAN);
    use ShipmentStatus::{
        AtHub, Attempted, Cancelled, Created, Disposed, OnHold, PickedUp, Returning, Voided,
    };
    // A client user of the company cancels what a colleague booked.
    let cancel_for_company = shipment.status == Created
        && user.has_permission(permissions::SHIPMENTS_CANCEL)
        && booking::acts_for(&state, &user, shipment.client()).await?;
    let surcharge = reweigh::surcharge_of(&state.db, &shipment)
        .await?
        .filter(|charge| charge.billed_on.is_none());
    let can_dispute = match &surcharge {
        Some(charge) if !charge.held => {
            manages
                || (user.has_permission(permissions::SHIPMENTS_BOOK)
                    && booking::acts_for(&state, &user, shipment.client()).await?)
        }
        _ => false,
    };
    let page = ShowPage {
        tx: tx(&lang),
        status_label: status_word(&lang, shipment.status),
        status_kind: status_kind(shipment.status),
        parcels: shipment
            .parcels(&state.db)
            .await?
            .into_iter()
            .map(|p| ParcelRow {
                barcode: p.barcode,
                weight_grams: p.weight_grams,
                state: p.state,
            })
            .collect(),
        events: shipment
            .events(&state.db)
            .await?
            .into_iter()
            .map(|e| EventRow {
                kind: e.kind,
                note: e.note,
                at: e.occurred_at.to_string(),
            })
            .collect(),
        charges: shipment
            .charges(&state.db)
            .await?
            .into_iter()
            .map(|c| ChargeRow {
                kind: c.kind,
                amount: c.amount,
                note: c.note,
            })
            .collect(),
        delivery_url: shipment
            .delivery_id
            .map(|id| format!("/deliveries/{id}"))
            .unwrap_or_default(),
        can_pickup: scans && shipment.status == Created,
        can_hub: scans
            && matches!(
                shipment.status,
                Created | PickedUp | Attempted | Returning
            ),
        can_void: has("void"),
        can_cancel: has("cancel") || cancel_for_company,
        can_cancel_return: manages && matches!(shipment.status, PickedUp | AtHub | Attempted | OnHold),
        can_return: has("return_delivered")
            || (user.has_permission(permissions::SHIPMENTS_DELIVER)
                && shipment.status == Returning),
        can_reweigh: (scans || manages)
            && !matches!(shipment.status, Created | Cancelled | Voided | Disposed),
        can_dispute,
        can_resolve_dispute: manages && surcharge.as_ref().is_some_and(|charge| charge.held),
        charged_price: shipment.charged_price(),
        outcomes: vec![
            ["confirm".to_owned(), t(&lang, "logistics.screen.outcome_confirm")],
            ["remove".to_owned(), t(&lang, "logistics.screen.outcome_remove")],
        ],
        instruction: shipment.return_instruction.clone().unwrap_or_default(),
        shipment,
    };
    Ok(View::typed(page).into_response())
}

/// `POST /shipments`: `client_party_id`, `client_reference`, `recipient_name`,
/// `recipient_phone`, `address`, `area`, `parcels` (a count, 1), `weight_grams`
/// (per parcel) and `cod` (dirhams).
async fn book(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let bad = || refused(t(&lang, "logistics.bad_input"));
    let client = field(&form, "client_party_id")
        .trim()
        .parse::<i64>()
        .map_err(|_| bad())?;
    let parcels = match field(&form, "parcels").trim() {
        "" => 1,
        text => text.parse::<usize>().map_err(|_| bad())?,
    };
    let weight = match field(&form, "weight_grams").trim() {
        "" => 0,
        text => text.parse::<i64>().map_err(|_| bad())?,
    };
    let cod_amount = booking::parse_dirhams(&field(&form, "cod")).ok_or_else(bad)?;
    let new = NewShipment {
        client: PartyId(client),
        client_reference: Some(field(&form, "client_reference")),
        recipient_name: field(&form, "recipient_name"),
        recipient_phone: field(&form, "recipient_phone"),
        address: field(&form, "address"),
        area: field(&form, "area"),
        parcel_weights_grams: vec![weight; parcels],
        cod_amount,
    };
    let shipment = booking::book(&state, &user, &new).await.map_err(answer)?;
    back_to(&lang, "logistics.booked", shipment.id)
}

/// `POST /shipments/import`: `client_party_id`, `csv` (the file's text) and
/// `dry_run` (`1`: list what would happen and book nothing).
async fn import(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let client = field(&form, "client_party_id")
        .trim()
        .parse::<i64>()
        .map_err(|_| refused(t(&lang, "logistics.bad_input")))?;
    let dry_run = matches!(field(&form, "dry_run").trim(), "1" | "true" | "on");
    let report = booking::import_csv(&state, &user, PartyId(client), &field(&form, "csv"), dry_run)
        .await
        .map_err(answer)?;
    if dry_run {
        return Ok(Json(report).into_response());
    }
    Ok((
        Toast::success(t(&lang, "logistics.imported")),
        Redirect::route("logistics.shipments", &[])?,
    )
        .into_response())
}

/// The labels scanned, those marked not received and the reason of a failed
/// pickup, from a form.
fn handover_of(form: &BTreeMap<String, String>) -> scan::Handover {
    scan::Handover {
        barcodes: scan::barcodes_in(&field(form, "barcodes")),
        not_received: scan::barcodes_in(&field(form, "not_received")),
        failed_reason: field(form, "reason"),
    }
}

/// `POST /shipments/{id}/pickup`: `barcodes` (the labels scanned),
/// `not_received` (the labels of the parcels the client did not hand over)
/// and `reason` (why nothing was handed over).
async fn pickup(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    scan::pick_up_handover(&state, &user, id, &handover_of(&form))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.picked_up", id)
}

/// `POST /shipments/{id}/hub`: `barcodes` (the labels scanned), and from
/// `created` (a drop-off) `not_received` and `reason` as at pickup.
async fn hub(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    scan::receive_handover(&state, &user, id, &handover_of(&form))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.hub_scanned", id)
}

/// `POST /scans`: `barcode` and `kind` (`pickup`, or `hub`).
async fn scan_label(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Fields,
) -> Result<Response> {
    let hub = field(&form, "kind").trim() == "hub";
    let shipment = scan::scan_label(&state, &user, &field(&form, "barcode"), hub)
        .await
        .map_err(answer)?;
    back_to(
        &lang,
        if hub {
            "logistics.hub_scanned"
        } else {
            "logistics.picked_up"
        },
        shipment.id,
    )
}

/// `POST /shipments/{id}/void`: `reason` (duplicate, entered in error, or a
/// note); the kernel's void run.
async fn void(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let reason = field(&form, "reason");
    if reason.trim().is_empty() {
        return Err(refused(t(&lang, "logistics.void.reason_required")));
    }
    let outcome = voids::void(&state, &user, &Subject::new(SUBJECT_TYPE, id), &reason)
        .await
        .map_err(answer)?;
    match outcome {
        VoidOutcome::Voided(_) | VoidOutcome::Pending(_) => back_to(&lang, "logistics.voided", id),
    }
}

/// `POST /shipments/{id}/cancel`.
async fn cancel_shipment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    cancel::cancel(&state, &user, id).await.map_err(answer)?;
    back_to(&lang, "logistics.cancelled", id)
}

/// `POST /shipments/{id}/instruction`: `note`, the client's instruction.
async fn record_instruction(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    cancel::record_instruction(&state, &user, id, &field(&form, "note"))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.instruction_recorded", id)
}

/// `POST /shipments/{id}/cancel-return`: `instruction`, when it is not
/// recorded yet.
async fn cancel_return(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    cancel::cancel_return(&state, &user, id, &field(&form, "instruction"))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.cancel_returned", id)
}

/// `POST /shipments/{id}/returned`: `pod_reference`, the client's e-POD.
async fn returned(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    cancel::return_delivered(&state, &user, id, &field(&form, "pod_reference"))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.returned", id)
}

/// A number from a form, in whole units; blank is 0.
fn whole(form: &BTreeMap<String, String>, name: &str) -> Option<i64> {
    match field(form, name).trim() {
        "" => Some(0),
        text => text.parse::<i64>().ok(),
    }
}

/// `POST /shipments/{id}/reweigh`: `weight_kg` (like 2.4), `length_cm`,
/// `width_cm`, `height_cm` and `photo` (the reference of the photo of the
/// scale).
async fn reweigh(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let bad = || refused(t(&lang, "logistics.bad_input"));
    // Kilograms to 0.01 kg as entered, in grams.
    let weight_grams = booking::parse_dirhams(&field(&form, "weight_kg"))
        .ok_or_else(bad)?
        * 10;
    let new = NewMeasurement {
        weight_grams,
        length_cm: whole(&form, "length_cm").ok_or_else(bad)?,
        width_cm: whole(&form, "width_cm").ok_or_else(bad)?,
        height_cm: whole(&form, "height_cm").ok_or_else(bad)?,
        photo: Some(field(&form, "photo")),
    };
    reweigh::record(&state, &user, id, &new).await.map_err(answer)?;
    back_to(&lang, "logistics.reweighed", id)
}

/// `POST /shipments/{id}/reweigh-dispute`: `note`.
async fn reweigh_dispute(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    reweigh::open_dispute(&state, &user, id, &field(&form, "note"))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.reweigh_disputed", id)
}

/// `POST /shipments/{id}/reweigh-dispute/resolve`: `outcome` (`confirm`, or
/// `remove`).
async fn reweigh_resolve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let confirm = match field(&form, "outcome").trim() {
        "confirm" => true,
        "remove" => false,
        _ => return Err(refused(t(&lang, "logistics.bad_input"))),
    };
    reweigh::resolve_dispute(&state, &user, id, confirm)
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.reweigh_resolved", id)
}
