//! Routes the seed uses where a service needs the signed-in user and no page
//! of the back office calls it yet (holding a deposit: the deposit pages list,
//! refund, forfeit and transfer deposits, but the money is held where the
//! lease is paid). They answer only in `local` and `testing`, and only to a
//! signed-in user who may do what the service asks, so a server in production
//! has them as 404s.

use renox::Environment;
use renox::prelude::*;
use wl_kernel::payments::{PaymentId, PaymentMethod, RecordPayment, payment_recorder};
use wl_kernel::{PartyId, Subject};

/// The module with the seed's routes.
pub struct Hooks;

#[derive(serde::Deserialize)]
struct HoldForm {
    party: i64,
    entity: i64,
    lease: i64,
    amount: i64,
    reference: String,
    date: Option<String>,
}

fn allowed(state: &AppState) -> Result {
    if matches!(state.config.env, Environment::Local | Environment::Testing) {
        Ok(())
    } else {
        Err(Error::NotFound)
    }
}

/// `POST /demo/deposits`: the tenant pays `amount` fils by transfer on `date`
/// and the deposit of the lease holds it. The id of the deposit.
async fn hold_deposit(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<HoldForm>,
) -> Result<String> {
    allowed(&state)?;
    let mut request = RecordPayment::new(
        PartyId(form.party),
        form.entity,
        PaymentMethod::Transfer,
        form.amount,
    );
    request.reference = Some(form.reference);
    request.date = form
        .date
        .as_deref()
        .and_then(|d| d.parse::<renox::chrono::NaiveDate>().ok());
    let payment = payment_recorder(&state)
        .record(&state, request, Some(user.id))
        .await
        .map_err(|e| Error::Status(StatusCode::UNPROCESSABLE_ENTITY, format!("{e:?}")))?;
    let held = wl_payments::deposit::HoldDeposit {
        party: PartyId(form.party),
        legal_entity_id: form.entity,
        source: Subject::new("property.lease", form.lease),
        amount: form.amount,
        currency: "AED".to_owned(),
        received_payment: Some(PaymentId(payment.id.0)),
    };
    let deposit = wl_payments::deposit::hold(&state, &user, &held)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    Ok(deposit.id.to_string())
}

impl Module for Hooks {
    fn name(&self) -> &'static str {
        "demo_hooks"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .post("/demo/deposits", hold_deposit)
            .require_auth()
    }
}
