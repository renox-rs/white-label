//! The redemption pages (wl-sales §5.9 step 3), behind `loyalty.redeem`.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /counter-sales/{id}/redeem` | `wl_sales.counter_sale_redeem` | `loyalty.redeem` |
//! | `POST /counter-sales/{id}/redeem/confirm` | `wl_sales.counter_sale_redeem_confirm` | `loyalty.redeem` |
//! | `POST /sales-orders/{id}/redeem` | `wl_sales.sales_order_redeem` | `loyalty.redeem` |
//! | `POST /sales-orders/{id}/redeem/confirm` | `wl_sales.sales_order_redeem_confirm` | `loyalty.redeem` |
//!
//! The cashier asks for `points`; the member reads out the `code` sent to
//! their mobile. A refusal (below the minimum, above the maximum share, a
//! wrong code, three wrong codes, a code that could not be sent) is a 422 with
//! its reason, and the sale goes on without the redemption line.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::Subject;

use crate::counter_sale::CounterSale;
use crate::notices::aed;
use crate::redeem::{self, RedeemRequest, Requested};
use crate::redemption::{Redemption, RedemptionStatus};
use crate::sales_order::SalesOrder;
use crate::{permissions, texts};

/// The routes, behind `loyalty.redeem`.
pub fn routes() -> Routes {
    Routes::new()
        .post("/counter-sales/{id}/redeem", redeem_counter_sale)
        .name("wl_sales.counter_sale_redeem")
        .post("/counter-sales/{id}/redeem/confirm", confirm_counter_sale)
        .name("wl_sales.counter_sale_redeem_confirm")
        .post("/sales-orders/{id}/redeem", redeem_sales_order)
        .name("wl_sales.sales_order_redeem")
        .post("/sales-orders/{id}/redeem/confirm", confirm_sales_order)
        .name("wl_sales.sales_order_redeem_confirm")
        .require_permission(permissions::LOYALTY_REDEEM)
}

/// The package text `key` (without the `wl_sales.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_sales.{key}"))
}

/// A 422 with `message` under `field`, keeping what was typed.
fn invalid<F: Serialize>(field: &str, message: String, form: &F) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// What a sale page shows of the redemption.
#[derive(Serialize, Debug, Clone, Default)]
pub struct RedeemView {
    /// `800 points, AED 8.00 (held)`, when the sale has a live redemption.
    pub summary: Option<String>,
    /// The cashier may ask for points.
    pub can_redeem: bool,
    /// A redemption waits for the member's code and the cashier may enter it.
    pub can_confirm: bool,
}

/// The redemption block of the page of the open or closed sale `sale`.
pub(crate) async fn view_of(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    sale: &Subject,
    open: bool,
    has_member: bool,
) -> Result<RedeemView> {
    let rows = Redemption::of_sale(&state.db, sale).await?;
    let live = rows.iter().rev().find(|r| {
        matches!(
            r.status,
            RedemptionStatus::Pending | RedemptionStatus::Held | RedemptionStatus::Applied
        )
    });
    let summary = live.map(|r| {
        format!(
            "{} {}, AED {} ({})",
            r.points,
            text(lang, "redeem.points").to_lowercase(),
            aed(r.value),
            text(lang, &format!("redeem.status.{}", r.status.as_str())).to_lowercase()
        )
    });
    let may = user.has_permission(permissions::LOYALTY_REDEEM);
    let settled = live.is_some_and(|r| r.status != RedemptionStatus::Pending);
    Ok(RedeemView {
        summary,
        can_redeem: open && has_member && may && !settled,
        can_confirm: open && may && live.is_some_and(|r| r.status == RedemptionStatus::Pending),
    })
}

/// Asks for the points of the form on `sale`.
async fn ask(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    sale: Subject,
    route: &str,
    id: i64,
    form: BTreeMap<String, String>,
) -> Result<Response> {
    let points: i64 = form
        .get("points")
        .map_or("", |v| v.trim())
        .parse()
        .map_err(|_| invalid("points", text(lang, "loyalty.errors.number"), &form))?;
    let requested = redeem::request(state, RedeemRequest::new(sale, points).by(user.id)).await?;
    let message = match requested {
        Requested::CodeSent { .. } => "redeem.code_sent",
        Requested::Held { .. } => "redeem.held",
    };
    Ok((
        Toast::success(text(lang, message)),
        Redirect::route(route, &[&id])?,
    )
        .into_response())
}

/// Confirms the pending redemption of `sale` with the `code` of the form.
async fn confirm(
    state: &AppState,
    lang: &Lang,
    sale: Subject,
    route: &str,
    id: i64,
    form: BTreeMap<String, String>,
) -> Result<Response> {
    let code = form.get("code").map_or("", |v| v.trim());
    redeem::confirm(state, &sale, code).await?;
    Ok((
        Toast::success(text(lang, "redeem.confirmed")),
        Redirect::route(route, &[&id])?,
    )
        .into_response())
}

/// `POST /counter-sales/{id}/redeem`.
async fn redeem_counter_sale(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(sale): Found<CounterSale>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    ask(&state, &user, &lang, sale.subject(), "wl_sales.counter_sale", sale.id, form).await
}

/// `POST /counter-sales/{id}/redeem/confirm`.
async fn confirm_counter_sale(
    State(state): State<AppState>,
    lang: Lang,
    Found(sale): Found<CounterSale>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    confirm(&state, &lang, sale.subject(), "wl_sales.counter_sale", sale.id, form).await
}

/// `POST /sales-orders/{id}/redeem`.
async fn redeem_sales_order(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(order): Found<SalesOrder>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    ask(&state, &user, &lang, order.subject(), "wl_sales.sales_order", order.id, form).await
}

/// `POST /sales-orders/{id}/redeem/confirm`.
async fn confirm_sales_order(
    State(state): State<AppState>,
    lang: Lang,
    Found(order): Found<SalesOrder>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    confirm(&state, &lang, order.subject(), "wl_sales.sales_order", order.id, form).await
}
