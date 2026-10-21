//! The back-office chargeback actions on the payment page (wl-payments §4,
//! §5.2a):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /payments/{id}/chargeback` | `wl_payments.chargeback` | `payments.chargeback` (fields `reference`, `deadline`, `fee`): records a chargeback by hand, for N-Genius |
//! | `POST /payments/{id}/evidence` | `wl_payments.chargeback_evidence` | `payments.chargeback`: the evidence was submitted in the gateway |
//!
//! Each has its own route because its guards read the input the module
//! gathers; the kernel's transition route serves the others. A refused guard
//! answers 422 under the field.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};

use crate::chargeback::{charge_back_by_hand, evidence_submitted};
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
