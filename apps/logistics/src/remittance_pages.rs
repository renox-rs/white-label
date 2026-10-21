//! The staff pages and actions on COD remittances (03 Logistics §7.1 *COD
//! remittance*; BL-LOG-14, BL-LOG-15).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /remittances` | `logistics.remittances` | `remittances.view` |
//! | `GET /remittances/{id}` | `logistics.remittance` | `remittances.view` |
//! | `POST /remittances/build` | `logistics.remittance_build` | `remittances.prepare` |
//! | `POST /remittances/{id}/approve` | `logistics.remittance_approve` | `remittances.approve` |
//! | `POST /remittances/{id}/reopen` | `logistics.remittance_reopen` | `remittances.approve` |
//! | `POST /remittances/{id}/pay` | `logistics.remittance_pay` | `remittances.pay` |
//! | `POST /remittances/adjustments` | `logistics.adjustment_add` | `remittances.prepare` |
//! | `POST /remittances/adjustments/{id}/approve` | `logistics.adjustment_approve` | `remittances.approve` |
//! | `POST /remittances/adjustments/{id}/reject` | `logistics.adjustment_reject` | `remittances.approve` |
//! | `POST /remittances/adjustments/{id}/resubmit` | `logistics.adjustment_resubmit` | `remittances.prepare` |
//! | `POST /remittances/adjustments/{id}/delete` | `logistics.adjustment_delete` | `remittances.prepare` |
//!
//! A successful action answers a redirect (303) with a toast. A refused
//! action answers 422 with the reason (403 without the permission).

use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::Serialize;
use wl_kernel::PartyId;

use crate::adjustment::{self, AdjustmentLine, AdjustmentStatus, NewAdjustment};
use crate::pages::{Fields, Texts, field, tx};
use crate::permissions;
use crate::remit;
use crate::remittance::{Remittance, RemittanceStatus};
use crate::shipment::Shipment;
use crate::texts::t;

/// The templates, compiled in.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "logistics/remittances.html",
        include_str!("../views/logistics/remittances.html"),
    ),
    (
        "logistics/remittance.html",
        include_str!("../views/logistics/remittance.html"),
    ),
];

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/remittances", index)
        .name("logistics.remittances")
        .get("/remittances/{id}", show)
        .name("logistics.remittance")
        .post("/remittances/build", build)
        .name("logistics.remittance_build")
        .post("/remittances/{id}/approve", approve)
        .name("logistics.remittance_approve")
        .post("/remittances/{id}/reopen", reopen)
        .name("logistics.remittance_reopen")
        .post("/remittances/{id}/pay", pay)
        .name("logistics.remittance_pay")
        .post("/remittances/adjustments", add_adjustment)
        .name("logistics.adjustment_add")
        .post("/remittances/adjustments/{id}/approve", approve_adjustment)
        .name("logistics.adjustment_approve")
        .post("/remittances/adjustments/{id}/reject", reject_adjustment)
        .name("logistics.adjustment_reject")
        .post("/remittances/adjustments/{id}/resubmit", resubmit_adjustment)
        .name("logistics.adjustment_resubmit")
        .post("/remittances/adjustments/{id}/delete", delete_adjustment)
        .name("logistics.adjustment_delete")
        .require_auth()
}

/// The word of a status.
pub(crate) fn status_word(lang: &Lang, status: RemittanceStatus) -> String {
    t(
        lang,
        &format!("logistics.screen.remit_status.{}", status.as_str()),
    )
}

/// The badge colour of a status.
pub(crate) fn status_kind(status: RemittanceStatus) -> &'static str {
    match status {
        RemittanceStatus::Draft => "info",
        RemittanceStatus::Approved => "warning",
        RemittanceStatus::Paid => "success",
        RemittanceStatus::PaymentReturned => "error",
    }
}

fn answer(error: TransitionError) -> Error {
    error.into_error()
}

/// A redirect to the remittance's page with a toast.
fn back_to(lang: &Lang, key: &str, id: i64) -> Result<Response> {
    Ok((
        Toast::success(t(lang, key)),
        Redirect::route("logistics.remittance", &[&id])?,
    )
        .into_response())
}

/// One remittance of the list.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Row {
    number: String,
    client_party_id: i64,
    cod_total: i64,
    fee_total: i64,
    adjustment_total: i64,
    net: i64,
    status: String,
    status_kind: &'static str,
    url: String,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "logistics/remittances.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    remittances: Vec<Row>,
    can_build: bool,
    can_add_adjustment: bool,
}

async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<Response> {
    if !user.has_permission(permissions::REMITTANCES_VIEW) {
        return Err(Error::Forbidden);
    }
    let found = Remittance::query()
        .order_by_desc(Remittance::ID)
        .limit(200)
        .get(&state.db)
        .await?;
    let remittances = found
        .iter()
        .map(|r| Row {
            number: r.number.clone(),
            client_party_id: r.client_party_id,
            cod_total: r.cod_total,
            fee_total: r.fee_total,
            adjustment_total: r.adjustment_total,
            net: r.net,
            status: status_word(&lang, r.status),
            status_kind: status_kind(r.status),
            url: format!("/remittances/{}", r.id),
        })
        .collect();
    Ok(View::typed(IndexPage {
        tx: tx(&lang),
        remittances,
        can_build: user.has_permission(permissions::REMITTANCES_PREPARE),
        can_add_adjustment: user.has_permission(permissions::REMITTANCES_PREPARE),
    })
    .into_response())
}

/// A shipment line of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct LineRow {
    shipment: String,
    shipment_url: String,
    cod_amount: i64,
    absorbed: i64,
    absorbed_reason: String,
    collected: i64,
    fee: i64,
    amount: i64,
}

/// An adjustment line of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct AdjustmentRow {
    id: i64,
    amount: i64,
    reason: String,
    source: String,
    status: String,
    counts: bool,
    can_decide: bool,
    can_delete: bool,
}

/// What the remittance's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "logistics/remittance.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    remittance: Remittance,
    status_label: String,
    status_kind: &'static str,
    lines: Vec<LineRow>,
    adjustments: Vec<AdjustmentRow>,
    can_approve: bool,
    can_reopen: bool,
    can_pay: bool,
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !user.has_permission(permissions::REMITTANCES_VIEW) {
        return Err(Error::Forbidden);
    }
    let remittance = Remittance::find_or_404(&state.db, id).await?;
    let mut lines = Vec::new();
    for line in remittance.lines(&state.db).await? {
        let number = Shipment::find(&state.db, line.shipment_id)
            .await?
            .map(|s| s.number)
            .unwrap_or_default();
        lines.push(LineRow {
            shipment_url: format!("/shipments/{}", line.shipment_id),
            shipment: number,
            cod_amount: line.cod_amount,
            absorbed: line.absorbed,
            absorbed_reason: line.absorbed_reason.clone(),
            collected: line.collected_for_client(),
            fee: line.fee,
            amount: line.amount,
        });
    }
    let adjustments = remittance
        .adjustments(&state.db)
        .await?
        .iter()
        .map(|line: &AdjustmentLine| AdjustmentRow {
            id: line.id,
            amount: line.amount,
            reason: line.reason.clone(),
            source: line.source.clone(),
            status: t(
                &lang,
                &format!("logistics.screen.adjustment_status.{}", line.status.as_str()),
            ),
            counts: line.counts(),
            can_decide: line.status == AdjustmentStatus::Pending
                && user.has_permission(permissions::REMITTANCES_APPROVE)
                && line.prepared_by != Some(user.id),
            can_delete: line.status != AdjustmentStatus::Approved
                && user.has_permission(permissions::REMITTANCES_PREPARE)
                && line.prepared_by == Some(user.id),
        })
        .collect();
    let draft = remittance.status == RemittanceStatus::Draft;
    let approved = remittance.status == RemittanceStatus::Approved;
    Ok(View::typed(ShowPage {
        tx: tx(&lang),
        status_label: status_word(&lang, remittance.status),
        status_kind: status_kind(remittance.status),
        lines,
        adjustments,
        can_approve: draft
            && user.has_permission(permissions::REMITTANCES_APPROVE)
            && remittance.prepared_by != Some(user.id),
        can_reopen: approved && user.has_permission(permissions::REMITTANCES_APPROVE),
        can_pay: approved && user.has_permission(permissions::REMITTANCES_PAY),
        remittance,
    })
    .into_response())
}

/// `POST /remittances/build`: `client_party_id`.
async fn build(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Fields,
) -> Result<Response> {
    let client = field(&form, "client_party_id")
        .trim()
        .parse::<i64>()
        .map_err(|_| {
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, t(&lang, "logistics.bad_input"))
        })?;
    let remittance = remit::build(&state, &user, PartyId(client))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.remittance_built", remittance.id)
}

/// `POST /remittances/{id}/approve`.
async fn approve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    remit::approve(&state, &user, id).await.map_err(answer)?;
    back_to(&lang, "logistics.remittance_approved", id)
}

/// `POST /remittances/{id}/reopen`.
async fn reopen(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    remit::reopen(&state, &user, id).await.map_err(answer)?;
    back_to(&lang, "logistics.remittance_reopened", id)
}

/// `POST /remittances/{id}/pay`: `payment_reference`, the bank reference.
async fn pay(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    remit::pay(&state, &user, id, &field(&form, "payment_reference"))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.remittance_paid", id)
}

/// A whole number from a form; blank is none, anything else that is not a
/// number is `Err`.
fn id_in(form: &std::collections::BTreeMap<String, String>, name: &str) -> std::result::Result<Option<i64>, ()> {
    match field(form, name).trim() {
        "" => Ok(None),
        text => text.parse::<i64>().map(Some).map_err(|_| ()),
    }
}

fn bad_input(lang: &Lang) -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        t(lang, "logistics.bad_input"),
    )
}

/// A redirect to the remittance list with a toast.
fn back_to_list(lang: &Lang, key: &str) -> Result<Response> {
    Ok((
        Toast::success(t(lang, key)),
        Redirect::route("logistics.remittances", &[])?,
    )
        .into_response())
}

/// `POST /remittances/adjustments`: `client_party_id`, `amount` (dirhams,
/// minus to take back), `reason`, `source` (blank: manual), and optionally
/// `shipment_id`, `corrects_remittance_id` and `remittance_id` (a draft to
/// put it on now).
async fn add_adjustment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Fields,
) -> Result<Response> {
    let client = field(&form, "client_party_id")
        .trim()
        .parse::<i64>()
        .map_err(|_| bad_input(&lang))?;
    let amount = adjustment::parse_signed_dirhams(&field(&form, "amount"))
        .ok_or_else(|| bad_input(&lang))?;
    let new = NewAdjustment {
        client: PartyId(client),
        amount,
        reason: field(&form, "reason"),
        source: field(&form, "source"),
        shipment_id: id_in(&form, "shipment_id").map_err(|()| bad_input(&lang))?,
        corrects_remittance_id: id_in(&form, "corrects_remittance_id")
            .map_err(|()| bad_input(&lang))?,
        remittance_id: id_in(&form, "remittance_id").map_err(|()| bad_input(&lang))?,
    };
    let line = adjustment::add(&state, &user, &new).await.map_err(answer)?;
    match line.remittance_id {
        Some(id) => back_to(&lang, "logistics.adjustment_added", id),
        None => back_to_list(&lang, "logistics.adjustment_added"),
    }
}

/// Where a decision on `line` goes back to.
fn back_after(lang: &Lang, key: &str, line: &AdjustmentLine) -> Result<Response> {
    match line.remittance_id {
        Some(id) => back_to(lang, key, id),
        None => back_to_list(lang, key),
    }
}

/// `POST /remittances/adjustments/{id}/approve`: `note`.
async fn approve_adjustment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let line = adjustment::approve_line(&state, &user, id, &field(&form, "note"))
        .await
        .map_err(answer)?;
    back_after(&lang, "logistics.adjustment_approved", &line)
}

/// `POST /remittances/adjustments/{id}/reject`: `note`.
async fn reject_adjustment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let line = adjustment::reject_line(&state, &user, id, &field(&form, "note"))
        .await
        .map_err(answer)?;
    back_after(&lang, "logistics.adjustment_rejected", &line)
}

/// `POST /remittances/adjustments/{id}/resubmit`: `amount` and `reason`, the
/// preparer's corrected line.
async fn resubmit_adjustment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let amount = adjustment::parse_signed_dirhams(&field(&form, "amount"))
        .ok_or_else(|| bad_input(&lang))?;
    let line = adjustment::resubmit_line(&state, &user, id, amount, &field(&form, "reason"))
        .await
        .map_err(answer)?;
    back_after(&lang, "logistics.adjustment_resubmitted", &line)
}

/// `POST /remittances/adjustments/{id}/delete`.
async fn delete_adjustment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    adjustment::delete_line(&state, &user, id)
        .await
        .map_err(answer)?;
    back_to_list(&lang, "logistics.adjustment_deleted")
}
