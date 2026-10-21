//! The back-office chargeback actions on the payment page (wl-payments §4,
//! §5.2a):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /payments/{id}/chargeback` | `wl_payments.chargeback` | `payments.chargeback` (fields `reference`, `deadline`, `fee`): records a chargeback by hand, for N-Genius |
//! | `POST /payments/{id}/evidence` | `wl_payments.chargeback_evidence` | `payments.chargeback`: the evidence was submitted in the gateway |
//! | `POST /payments/{id}/accept-chargeback` | `wl_payments.chargeback_accept` | `payments.chargeback` (field `reason`): asks to accept the chargeback; another holder of the permission approves it from "Needs your action" (`FIN.ACL.18`) |
//! | `POST /payments/{id}/chargeback-lost` | `wl_payments.chargeback_lost` | `payments.chargeback`: records that the dispute was lost (N-Genius) |
//!
//! Each has its own route because its guards read the input the module
//! gathers; the kernel's transition route serves the others. A refused guard
//! answers 422 under the field.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};

use crate::chargeback::{charge_back_by_hand, evidence_submitted};
use crate::chargeback_close::{accept, lose_by_hand};
use crate::pages::{filled, invalid, parse_amount, texts_in};
use crate::payment::Payment;
use crate::permissions;

/// The routes, behind `payments.chargeback`.
pub fn routes() -> Routes {
    Routes::new()
        .post("/payments/{id}/chargeback", store_chargeback)
        .name("wl_payments.chargeback")
        .post("/payments/{id}/evidence", store_evidence)
        .name("wl_payments.chargeback_evidence")
        .post("/payments/{id}/accept-chargeback", store_accept)
        .name("wl_payments.chargeback_accept")
        .post("/payments/{id}/chargeback-lost", store_lost)
        .name("wl_payments.chargeback_lost")
        .require_permission(permissions::CHARGEBACK)
}

/// The chargeback sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct ChargebackForm {
    /// The acquirer's dispute reference.
    #[validate(max = 128)]
    pub reference: Option<String>,
    /// The day the evidence is due.
    pub deadline: Option<NaiveDate>,
    /// The gateway's fee in dirhams, like `50.00`; empty when not known.
    #[validate(max = 32)]
    pub fee: Option<String>,
}

/// `POST /payments/{id}/chargeback`: records the chargeback and reloads the
/// page. A refusal (no reference or deadline, a reference already recorded, a
/// payment that is not a gateway payment) answers 422.
async fn store_chargeback(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(payment): Found<Payment>,
    Valid(form): Valid<ChargebackForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let fee = match filled(&form.fee) {
        None => None,
        Some(text) => match parse_amount(text) {
            Some(fee) => Some(fee),
            None => return Err(invalid("fee", t("bad_amount"), &form)),
        },
    };
    let reference = filled(&form.reference).unwrap_or_default().to_owned();
    match charge_back_by_hand(&state, &user, &payment, &reference, form.deadline, fee).await {
        Ok(_) => Ok((Toast::success(t("chargeback.done")), renox::HxRefresh).into_response()),
        Err(TransitionError::Guard(message)) => Err(invalid("reference", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// `POST /payments/{id}/evidence`: the evidence was submitted.
async fn store_evidence(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(payment): Found<Payment>,
) -> Result<Response> {
    let t = texts_in(&lang);
    evidence_submitted(&state, &user, &payment)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((Toast::success(t("evidence.done")), renox::HxRefresh).into_response())
}

/// The accept sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct AcceptForm {
    /// Why the chargeback is not contested.
    #[validate(max = 2000)]
    pub reason: Option<String>,
}

/// `POST /payments/{id}/accept-chargeback`: asks to accept the chargeback.
/// It waits for another person's approval; without a reason it answers 422
/// under the field.
async fn store_accept(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(payment): Found<Payment>,
    Valid(form): Valid<AcceptForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let reason = filled(&form.reason).unwrap_or_default().to_owned();
    match accept(&state, &user, &payment, &reason).await {
        Ok(accepted) => {
            let key = if accepted.request.is_some() {
                "accept_chargeback.pending"
            } else {
                "accept_chargeback.done"
            };
            Ok((Toast::success(t(key)), renox::HxRefresh).into_response())
        }
        Err(TransitionError::ReasonRequired) => Err(invalid(
            "reason",
            t("accept_chargeback.reason_required"),
            &form,
        )),
        Err(other) => Err(other.into_error()),
    }
}

/// `POST /payments/{id}/chargeback-lost`: the dispute was lost.
async fn store_lost(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(payment): Found<Payment>,
) -> Result<Response> {
    let t = texts_in(&lang);
    lose_by_hand(&state, &user, &payment)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((Toast::success(t("lose_chargeback.done")), renox::HxRefresh).into_response())
}
