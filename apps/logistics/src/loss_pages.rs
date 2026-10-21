//! The staff actions on a lost shipment (03 Logistics §4.4 steps 3 and 4,
//! §7.1 rows `mark_lost` and `found`; BL-LOG-11).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /shipments/{id}/found` | `logistics.shipment_found` | `shipments.scan` (field `depot`) |
//! | `POST /shipments/{id}/claim-settled` | `logistics.shipment_claim_settled` | `shipments.manage` (field `amount`, dirhams) |
//! | `POST /shipments/{id}/claim-choice` | `logistics.shipment_claim_choice` | `shipments.manage` (field `choice`: `delivery` or `keep`) |
//! | `POST /shipments/{id}/mark-lost` | `logistics.shipment_mark_lost` | `shipments.manage` (fields `reason`, `liability`) |
//! | `POST /shipments/{id}/depot-scan` | `logistics.shipment_depot_scan` | `shipments.scan` (field `depot`) |
//! | `POST /shipments/{id}/note` | `logistics.shipment_note` | `shipments.manage` (field `note`) |
//!
//! A successful action answers a redirect (303) with a toast, a refusal 422
//! with the reason (403 without the permission).

use renox::prelude::*;
use renox::state_machine::TransitionError;

use crate::booking::parse_dirhams;
use crate::loss;
use crate::pages::{Fields, field};
use crate::trace::{self, LostRequest};
use crate::texts::{english, t};

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .post("/shipments/{id}/found", found)
        .name("logistics.shipment_found")
        .post("/shipments/{id}/claim-settled", claim_settled)
        .name("logistics.shipment_claim_settled")
        .post("/shipments/{id}/claim-choice", claim_choice)
        .name("logistics.shipment_claim_choice")
        .post("/shipments/{id}/mark-lost", mark_lost)
        .name("logistics.shipment_mark_lost")
        .post("/shipments/{id}/depot-scan", depot_scan)
        .name("logistics.shipment_depot_scan")
        .post("/shipments/{id}/note", note)
        .name("logistics.shipment_note")
        .require_auth()
}

fn answer(error: TransitionError) -> Error {
    error.into_error()
}

/// A redirect to the shipment's page with a toast.
fn back_to(lang: &Lang, key: &str, id: i64) -> Result<Response> {
    Ok((
        Toast::success(t(lang, key)),
        Redirect::route("logistics.shipment", &[&id])?,
    )
        .into_response())
}

/// `POST /shipments/{id}/found`: `depot`. The goods were scanned at a depot.
async fn found(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    loss::found(&state, &user, id, &field(&form, "depot"))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.loss_found", id)
}

/// `POST /shipments/{id}/claim-settled`: `amount` (dirhams). The claim was
/// settled with a credit for the client.
async fn claim_settled(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let Some(amount) = parse_dirhams(&field(&form, "amount")) else {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("logistics.loss.amount_needed", &[]),
        ));
    };
    loss::settle_claim(&state, &user, id, amount)
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.loss_claim_settled", id)
}

/// `POST /shipments/{id}/claim-choice`: `choice` (`delivery` or `keep`).
async fn claim_choice(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    loss::choose(&state, &user, id, &field(&form, "choice"))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.loss_claim_chosen", id)
}

/// `POST /shipments/{id}/mark-lost`: `reason` and `liability`. The request
/// waits for an operations manager or the company admin other than the user.
async fn mark_lost(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let asked = trace::request(
        &state,
        &user,
        id,
        &field(&form, "reason"),
        &field(&form, "liability"),
    )
    .await
    .map_err(answer)?;
    let key = match asked {
        LostRequest::Lost => "logistics.loss_marked",
        LostRequest::Requested(_) => "logistics.loss_requested",
    };
    back_to(&lang, key, id)
}

/// `POST /shipments/{id}/depot-scan`: `depot`. The shipment was scanned at a
/// depot; a lost one is found.
async fn depot_scan(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    trace::scan_at_depot(&state, &user, id, &field(&form, "depot"))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.depot_scanned", id)
}

/// `POST /shipments/{id}/note`: `note`.
async fn note(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    trace::note(&state, &user, id, &field(&form, "note"))
        .await
        .map_err(answer)?;
    back_to(&lang, "logistics.noted", id)
}
