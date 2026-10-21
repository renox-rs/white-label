//! The public page of a payment link (wl-payments §5.2 step 1):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /pay/{token}` | `wl_payments.pay` | nobody: public, the sealed token is the key (a token that opens nothing shows a neutral page) |
//! | `POST /pay/{token}/start` | `wl_payments.pay_start` | the same: starts the online payment and sends the party to the gateway's hosted page |
//!
//! | `POST /payments/links/{id}/cancel` | `wl_payments.links.cancel` | `payments.links` |
//!
//! The page shows the amount and one button. After a payment on the link
//! failed it shows the reason and the button reads "Try again": the link stays
//! open until it is paid, expires or is cancelled, and a retry is a new payment
//! on it. A link that was paid, expired or cancelled says so and offers no
//! button. The template is compiled in and can be replaced by the app
//! (`resources/views/wl-payments/pay.html`).

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::Serialize;
use wl_kernel::payments::PaymentStatus;

use crate::followup::money_text;
use crate::online;
use crate::pages::Texts;
use crate::payment::Payment;
use crate::payment_link::{self, LinkStatus, PaymentLink};
use crate::permissions;
use crate::texts;

/// The public routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/pay/{token}", show)
        .name(payment_link::ROUTE)
        .post("/pay/{token}/start", start)
        .name("wl_payments.pay_start")
        .merge(
            Routes::new()
                .post("/payments/links/{id}/cancel", cancel)
                .name("wl_payments.links.cancel")
                .require_permission(permissions::LINKS),
        )
}

/// `POST /payments/links/{id}/cancel`: staff withdraw an open link.
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    let link = PaymentLink::find_or_404(&state.db, id).await?;
    link.transition(payment_link::CANCEL)
        .by(&user)
        .run(&state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(Redirect::route("wl_payments.index", &[])?.into_response())
}

/// What the public page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/pay.html")]
pub(crate) struct PayPage {
    tx: Texts,
    /// `open`, `retry` (a payment failed), `waiting`, `paid`, `expired`,
    /// `cancelled` or `invalid`.
    step: &'static str,
    brand: String,
    amount: String,
    /// Why the last payment on the link failed.
    failure: Option<String>,
    /// Why a start was refused.
    problem: Option<String>,
    start_action: String,
}

/// The step the page shows for `link`, whose latest payment is `latest`, at
/// `now`.
pub(crate) fn step_of(
    link: &PaymentLink,
    latest: Option<&Payment>,
    now: renox::db::DateTime,
) -> &'static str {
    match link.status {
        LinkStatus::Paid => "paid",
        LinkStatus::Cancelled => "cancelled",
        LinkStatus::Expired => "expired",
        LinkStatus::Open if now >= link.expires_at => "expired",
        LinkStatus::Open => match latest.map(|p| p.status) {
            Some(PaymentStatus::Pending) => "waiting",
            Some(PaymentStatus::Failed) => "retry",
            _ => "open",
        },
    }
}

/// The page for `token`, with `problem` when a start was refused.
async fn page(
    state: &AppState,
    lang: &Lang,
    token: String,
    problem: Option<String>,
) -> Result<View> {
    let link = match payment_link::id_of(state, &token) {
        Some(id) => PaymentLink::find(&state.db, id).await?,
        None => None,
    };
    let Some(link) = link else {
        return Ok(View::typed(PayPage {
            tx: texts::all(lang),
            step: "invalid",
            brand: state.config.name.clone(),
            amount: String::new(),
            failure: None,
            problem: None,
            start_action: String::new(),
        }));
    };
    let latest = Payment::query()
        .where_(Payment::PAYMENT_LINK_ID.eq(link.id))
        .order_by_desc(Payment::ID)
        .first(&state.db)
        .await?;
    let step = step_of(&link, latest.as_ref(), renox::db::now());
    let failure = latest
        .as_ref()
        .filter(|p| p.status == PaymentStatus::Failed)
        .and_then(|p| p.failure_reason.clone());
    Ok(View::typed(PayPage {
        tx: texts::all(lang),
        step,
        brand: state.config.name.clone(),
        amount: money_text(&link.currency, link.amount),
        failure,
        problem,
        start_action: format!("/pay/{token}/start"),
    }))
}

/// `GET /pay/{token}`.
async fn show(
    State(state): State<AppState>,
    lang: Lang,
    Path(token): Path<String>,
) -> Result<View> {
    page(&state, &lang, token, None).await
}

/// `POST /pay/{token}/start`: starts an online payment on the link and sends
/// the party to the hosted page; a link that cannot be paid shows the page
/// again with the reason.
async fn start(
    State(state): State<AppState>,
    lang: Lang,
    Path(token): Path<String>,
) -> Result<Response> {
    let Some(id) = payment_link::id_of(&state, &token) else {
        return Ok(page(&state, &lang, token, None).await?.into_response());
    };
    match online::start_online(&state, id).await {
        Ok(started) => Ok(Redirect::to(&started.checkout.url).into_response()),
        Err(TransitionError::Guard(message)) => {
            Ok(page(&state, &lang, token, Some(message)).await?.into_response())
        }
        Err(other) => Err(other.into_error()),
    }
}
