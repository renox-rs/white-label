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
//! | `POST /shipments/{id}/hold` | `logistics.shipment_hold` | `shipments.manage` or `.scan` |
//! | `POST /shipments/{id}/release` | `logistics.shipment_release` | `shipments.manage` |
//! | `POST /shipments/{id}/hold-instruction` | `logistics.shipment_hold_instruction` | `shipments.manage`, or the client's user |
//! | `POST /shipments/{id}/address` | `logistics.shipment_address` | `shipments.manage`, or the client's user |
//! | `POST /shipments/{id}/address/confirm` | `logistics.shipment_address_confirm` | `shipments.manage`, or the client's user |
//! | `POST /shipments/{id}/liable` | `logistics.shipment_liable` | `shipments.manage` |
//! | `POST /shipments/{id}/return` | `logistics.shipment_return` | `shipments.manage` |
//! | `POST /shipments/{id}/reweigh` | `logistics.shipment_reweigh` | `shipments.scan` or `.manage` |
//! | `POST /shipments/{id}/reweigh-dispute` | `logistics.shipment_reweigh_dispute` | `shipments.book` (own company) or `.manage` |
//! | `POST /shipments/{id}/reweigh-dispute/resolve` | `logistics.shipment_reweigh_resolve` | `shipments.manage` |
//! | `POST /shipments/{id}/cod-instruction` | `logistics.shipment_cod_instruction` | `shipments.manage` (field `note`; in [`crate::cod`]) |
//! | `POST /shipments/{id}/dispute` | `logistics.shipment_dispute` | `shipments.manage` (field `reason`) |
//!
//! A successful action answers a redirect (303) with a toast. The import with
//! `dry_run` answers the report as JSON and books nothing. A refused action
//! answers 422 with the reason (403 without the permission).

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, TransitionError};
use serde::Serialize;
use wl_kernel::voids::{self, VoidOutcome};
use wl_kernel::{PartyId, Subject};

use crate::booking::{self, NewShipment};
use crate::cancel;
use crate::hold::{self, NewAddress, NewHold};
use crate::permissions;
use crate::reweigh::{self, NewMeasurement};
use crate::scan;
use crate::screen_texts::SCREEN_TEXTS;
use crate::shipment::{SUBJECT_TYPE, Shipment, ShipmentStatus, hold_reason};
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
        .post("/shipments/{id}/hold", hold)
        .name("logistics.shipment_hold")
        .post("/shipments/{id}/release", release)
        .name("logistics.shipment_release")
        .post("/shipments/{id}/hold-instruction", hold_instruction)
        .name("logistics.shipment_hold_instruction")
        .post("/shipments/{id}/address", correct_address)
        .name("logistics.shipment_address")
        .post("/shipments/{id}/address/confirm", confirm_address)
        .name("logistics.shipment_address_confirm")
        .post("/shipments/{id}/liable", liable)
        .name("logistics.shipment_liable")
        .post("/shipments/{id}/return", send_back)
        .name("logistics.shipment_return")
        .post("/shipments/{id}/reweigh", reweigh)
        .name("logistics.shipment_reweigh")
        .post("/shipments/{id}/reweigh-dispute", reweigh_dispute)
        .name("logistics.shipment_reweigh_dispute")
        .post("/shipments/{id}/reweigh-dispute/resolve", reweigh_resolve)
        .name("logistics.shipment_reweigh_resolve")
        .post("/shipments/{id}/dispute", dispute_delivery)
        .name("logistics.shipment_dispute")
        .require_auth()
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

fn answer(error: TransitionError) -> Error {
    error.into_error()
}

pub(crate) type Fields = Form<BTreeMap<String, String>>;

pub(crate) fn field(form: &BTreeMap<String, String>, name: &str) -> String {
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

/// The text `logistics.screen.{key}`.
pub(crate) fn text(lang: &Lang, key: &str) -> String {
    t(lang, &format!("logistics.screen.{key}"))
}

/// The shipments grid: search on the number, reference, recipient, phone,
/// area and client, filters by status and by the day booked or delivered (a
/// range), the sum of the COD. The *client on hold* badge
/// is drawn by the template (`custom`).
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    use ShipmentStatus::{
        AtHub, Attempted, Cancelled, Created, Delivered, Disposed, Lost, OnHold, OutForDelivery,
        PickedUp, Returned, Returning, UnderReview, Voided,
    };
    let statuses: Vec<(String, String)> = [
        Created,
        PickedUp,
        AtHub,
        OutForDelivery,
        Attempted,
        OnHold,
        Returning,
        Delivered,
        UnderReview,
        Lost,
        Returned,
        Disposed,
        Cancelled,
        Voided,
    ]
    .into_iter()
    .map(|status| (status.as_str().to_owned(), status_word(lang, status)))
    .collect();
    let grid = Grid::new("logistics-shipments")
        .title(&text(lang, "title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &text(lang, "number"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/shipments/{id}"),
        )
        .column(Column::text("client_reference", &text(lang, "reference")).searchable())
        .column(
            Column::text("recipient_name", &text(lang, "recipient"))
                .mobile()
                .searchable(),
        )
        .column(
            Column::text("recipient_phone", &text(lang, "phone"))
                .hidden()
                .searchable(),
        )
        .column(Column::text("area", &text(lang, "area")).searchable())
        .column(
            Column::related(
                "client",
                &text(lang, "client"),
                "wl_parties_parties",
                "client_party_id",
                "display_name",
            )
            .searchable()
            .hidden(),
        )
        .column(Column::number("parcel_count", &text(lang, "parcels")).hidden())
        .column(
            Column::money("cod_amount", &wl_kernel::money_label(&text(lang, "cod")))
                .mobile()
                .summary(Summary::Sum),
        )
        .column(
            Column::select("status", &text(lang, "state"), statuses)
                .badges(&[
                    ("delivered", "success"),
                    ("picked_up", "info"),
                    ("at_hub", "info"),
                    ("out_for_delivery", "info"),
                    ("attempted", "warning"),
                    ("on_hold", "warning"),
                    ("returning", "warning"),
                    ("under_review", "warning"),
                    ("lost", "danger"),
                    ("created", "info"),
                    ("returned", "info"),
                    ("disposed", "info"),
                    ("cancelled", "info"),
                    ("voided", "info"),
                ])
                .mobile(),
        )
        .column(Column::datetime("created_at", &text(lang, "booked_at")))
        .column(Column::datetime("delivered_at", &text(lang, "delivered_at")).hidden())
        .column(Column::custom("hold", &text(lang, "client_on_hold")))
        .sort_by("-number")
        .row_url("/shipments/{id}")
        .cards_on_mobile()
        .empty_state(&text(lang, "empty"), Some(&text(lang, "empty_text")));
    if can_export { grid.exports() } else { grid }
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "logistics/shipments.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    shipments: GridPage<Shipment>,
}

async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    if !user.has_permission(permissions::SHIPMENTS_VIEW) {
        return Err(Error::Forbidden);
    }
    let can_export = user.has_permission(permissions::SHIPMENTS_MANAGE);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    let grid = grid(&lang, can_export);
    if let Some(file) = grid.export(Shipment::query(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(Shipment::query(), &request).await?;
    // Whether each client that has a shipment waiting for pickup on this page
    // is blocked.
    let mut on_hold: BTreeMap<i64, bool> = BTreeMap::new();
    for s in page
        .items()
        .iter()
        .filter(|s| s.status == ShipmentStatus::Created)
    {
        if let std::collections::btree_map::Entry::Vacant(slot) = on_hold.entry(s.client_party_id) {
            slot.insert(crate::block::on_hold(&state, s.client()).await?);
        }
    }
    let page = page.extend(|s| {
        json!({
            "client_on_hold": s.status == ShipmentStatus::Created
                && on_hold.get(&s.client_party_id).copied().unwrap_or(false),
        })
    });
    Ok(View::typed(IndexPage {
        tx: tx(&lang),
        shipments: page,
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
    can_hold: bool,
    can_release: bool,
    can_hold_instruction: bool,
    can_address: bool,
    can_confirm_address: bool,
    can_liable: bool,
    can_send_back: bool,
    hold_reasons: Vec<[String; 2]>,
    hold_reason: String,
    hold_instruction: String,
    claim_ticket: String,
    can_reweigh: bool,
    can_dispute: bool,
    can_resolve_dispute: bool,
    can_review: bool,
    can_cod_instruction: bool,
    can_found: bool,
    can_mark_lost: bool,
    can_dispose: bool,
    disposal_liabilities: Vec<[String; 2]>,
    can_depot_scan: bool,
    can_note: bool,
    liabilities: Vec<[String; 2]>,
    can_claim_settled: bool,
    can_claim_choice: bool,
    claim_choices: Vec<[String; 2]>,
    cod_instruction: String,
    review_ticket: String,
    linked: String,
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
        AtHub, Attempted, Cancelled, Created, Disposed, OnHold, OutForDelivery, PickedUp,
        Returning, Voided,
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
    // A client user of the company, or staff, may correct the address and
    // record the client's answers on a hold.
    let acts = manages
        || (user.has_permission(permissions::SHIPMENTS_BOOK)
            && booking::acts_for(&state, &user, shipment.client()).await?);
    let held = shipment.status == OnHold;
    let address_hold = held && shipment.hold_reason.as_deref() == Some(hold_reason::ADDRESS_ISSUE);
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
        can_hub: scans && matches!(shipment.status, Created | PickedUp | Attempted | Returning),
        can_void: has("void"),
        can_cancel: has("cancel") || cancel_for_company,
        can_cancel_return: manages
            && matches!(shipment.status, PickedUp | AtHub | Attempted | OnHold),
        can_return: has("return_delivered")
            || (user.has_permission(permissions::SHIPMENTS_DELIVER)
                && shipment.status == Returning),
        can_hold: (scans || manages) && matches!(shipment.status, Created | AtHub | Attempted),
        can_release: manages && held,
        can_hold_instruction: acts && held,
        can_address: acts
            && matches!(
                shipment.status,
                Created | PickedUp | AtHub | Attempted | OnHold
            ),
        can_confirm_address: acts && address_hold,
        can_liable: manages
            && held
            && shipment.hold_reason.as_deref() == Some(hold_reason::DAMAGED)
            && !shipment.company_liable,
        can_send_back: manages && matches!(shipment.status, Attempted | OnHold),
        hold_reasons: hold_reason::ALL
            .iter()
            .map(|reason| {
                [
                    (*reason).to_owned(),
                    t(&lang, &format!("logistics.screen.hold_reason.{reason}")),
                ]
            })
            .collect(),
        hold_reason: shipment
            .hold_reason
            .as_deref()
            .filter(|_| held)
            .map(|reason| t(&lang, &format!("logistics.screen.hold_reason.{reason}")))
            .unwrap_or_default(),
        hold_instruction: shipment.hold_instruction.clone().unwrap_or_default(),
        claim_ticket: shipment.claim_ticket_number.clone().unwrap_or_default(),
        can_reweigh: (scans || manages)
            && !matches!(shipment.status, Created | Cancelled | Voided | Disposed),
        can_dispute,
        can_resolve_dispute: manages && surcharge.as_ref().is_some_and(|charge| charge.held),
        can_review: manages
            && shipment.status == ShipmentStatus::Delivered
            && shipment.delivery_id.is_some(),
        can_cod_instruction: manages && shipment.status == OutForDelivery,
        can_found: scans && shipment.status == ShipmentStatus::Lost,
        can_mark_lost: manages && crate::trace::requestable(shipment.status),
        can_dispose: manages && crate::disposal::requestable(shipment.status),
        disposal_liabilities: crate::disposal::LIABILITIES
            .iter()
            .map(|liability| {
                [
                    (*liability).to_owned(),
                    t(
                        &lang,
                        &format!("logistics.screen.disposal_liability_{liability}"),
                    ),
                ]
            })
            .collect(),
        can_depot_scan: scans && matches!(shipment.status, PickedUp | AtHub | OnHold | Returning),
        can_note: manages && !matches!(shipment.status, Cancelled | Voided | Disposed),
        liabilities: crate::loss::LIABILITIES
            .iter()
            .map(|liability| {
                [
                    (*liability).to_owned(),
                    t(&lang, &format!("logistics.screen.liability_{liability}")),
                ]
            })
            .collect(),
        can_claim_settled: manages && shipment.status == ShipmentStatus::Lost,
        can_claim_choice: manages
            && held
            && shipment.hold_reason.as_deref() == Some(hold_reason::CLIENT_INSTRUCTION)
            && shipment.claim_settled_amount > 0
            && shipment.claim_choice.is_none(),
        claim_choices: ["delivery", "keep"]
            .iter()
            .map(|choice| {
                [
                    (*choice).to_owned(),
                    t(&lang, &format!("logistics.screen.claim_choice_{choice}")),
                ]
            })
            .collect(),
        cod_instruction: shipment.cod_instruction.clone().unwrap_or_default(),
        linked: linked_numbers(&state, &shipment).await?,
        review_ticket: shipment
            .review_ticket_id
            .map(|ticket| ticket.to_string())
            .unwrap_or_default(),
        charged_price: shipment.charged_price(),
        outcomes: vec![
            [
                "confirm".to_owned(),
                t(&lang, "logistics.screen.outcome_confirm"),
            ],
            [
                "remove".to_owned(),
                t(&lang, "logistics.screen.outcome_remove"),
            ],
        ],
        instruction: shipment.return_instruction.clone().unwrap_or_default(),
        shipment,
    };
    Ok(View::typed(page).into_response())
}

/// The numbers of the shipments linked to `shipment`: the one a return
/// shipment was made from, or the return shipments of one (BL-LOG-07).
async fn linked_numbers(state: &AppState, shipment: &Shipment) -> Result<String> {
    if let Some(original) = shipment.return_of_id {
        let original = Shipment::find_or_404(&state.db, original).await?;
        return Ok(original.number);
    }
    Ok(shipment
        .returns(&state.db)
        .await?
        .into_iter()
        .map(|ret| ret.number)
        .collect::<Vec<_>>()
        .join(", "))
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
    let report = booking::import_csv(
        &state,
        &user,
        PartyId(client),
        &field(&form, "csv"),
        dry_run,
    )
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
    let weight_grams = booking::parse_dirhams(&field(&form, "weight_kg")).ok_or_else(bad)? * 10;
    let new = NewMeasurement {
        weight_grams,
        length_cm: whole(&form, "length_cm").ok_or_else(bad)?,
        width_cm: whole(&form, "width_cm").ok_or_else(bad)?,
        height_cm: whole(&form, "height_cm").ok_or_else(bad)?,
        photo: Some(field(&form, "photo")),
    };
    reweigh::record(&state, &user, id, &new)
        .await
        .map_err(answer)?;
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
    if checked(&form, "stop_delivery") {
        // The client asked to stop the delivery: held, reason re-weigh dispute.
        hold::hold_for_reweigh_dispute(&state, id).await?;
    }
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

/// `POST /shipments/{id}/dispute`: `reason`. The delivery goes under review
/// in wl-delivery, which opens the dispute ticket; the shipment follows on the
/// event.
async fn dispute_delivery(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    crate::review::raise(&state, &user, id, &field(&form, "reason"))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.disputed", id)
}

/// Whether a checkbox of the form is ticked.
pub(crate) fn checked(form: &BTreeMap<String, String>, name: &str) -> bool {
    matches!(field(form, name).trim(), "1" | "true" | "on")
}

/// `POST /shipments/{id}/hold`: `reason` (`damaged`, `address_issue`,
/// `client_instruction` or `reweigh_dispute`), `photos` (references, one per
/// line), `company_liable` (ticked) and `barcode` (the label scanned).
async fn hold(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let new = NewHold {
        reason: field(&form, "reason"),
        photos: NewHold::photos_in(&field(&form, "photos")),
        company_liable: checked(&form, "company_liable"),
        barcode: Some(field(&form, "barcode")),
    };
    hold::hold(&state, &user, id, &new).await.map_err(answer)?;
    back_to(&lang, "logistics.held", id)
}

/// `POST /shipments/{id}/release`.
async fn release(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    hold::release(&state, &user, id).await.map_err(answer)?;
    back_to(&lang, "logistics.released", id)
}

/// `POST /shipments/{id}/hold-instruction`: `note`, the client's instruction.
async fn hold_instruction(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    hold::record_instruction(&state, &user, id, &field(&form, "note"))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.hold_instruction", id)
}

/// `POST /shipments/{id}/address`: `address` and `area`.
async fn correct_address(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let new = NewAddress {
        address: field(&form, "address"),
        area: field(&form, "area"),
    };
    hold::correct_address(&state, &user, id, &new)
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.address_corrected", id)
}

/// `POST /shipments/{id}/address/confirm`: `address` and `area` of the
/// corrected address; blank for "address unchanged, confirmed by the client".
async fn confirm_address(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let new = NewAddress {
        address: field(&form, "address"),
        area: field(&form, "area"),
    };
    let corrected = (!new.address.trim().is_empty()).then_some(&new);
    hold::confirm_address(&state, &user, id, corrected)
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.address_confirmed", id)
}

/// `POST /shipments/{id}/liable`: the company is liable for the damage.
async fn liable(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    hold::set_company_liable(&state, &user, id)
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.liable", id)
}

/// `POST /shipments/{id}/return`: send an attempted or held shipment back.
async fn send_back(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    hold::send_back(&state, &user, id).await.map_err(answer)?;
    back_to(&lang, "logistics.sent_back", id)
}
