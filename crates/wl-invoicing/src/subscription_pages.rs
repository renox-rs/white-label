//! The subscription pages (wl-invoicing §4 *Subscription*, BL-INV-21):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /subscriptions` | `wl_invoicing.subscriptions` | `subscriptions.view` |
//! | `GET /subscriptions/{id}` | `wl_invoicing.subscription` | `subscriptions.view` |
//! | `POST /subscriptions/{id}/cancel` | `wl_invoicing.subscription_cancel` | `subscriptions.manage` |
//!
//! The list shows each subscription's plan, party, agreement, state and next
//! cycle; a subscription's page shows its terms, its open pause causes (a
//! block shows only `on_hold`), its cycles with their invoices and, for a
//! holder of `subscriptions.manage`, the cancel sheet. A user cancel ends at
//! once (or, with a notice period, when the notice ends) and is prorated
//! only when *pro rata on cancel* is on (§7). Starting, pausing and resuming
//! come from the vertical's agreement (the kernel's `Subscriptions`).
//! The templates are compiled in and can be replaced by the app
//! (`resources/views/wl-invoicing/subscriptions.html`, `subscription.html`).

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::Serialize;
use wl_kernel::documents::today;
use wl_kernel::subscriptions::SubscriptionStatus;

use crate::allocate::amount_text;
use crate::permissions;
use crate::subscription::{Subscription, SubscriptionPause};
use crate::subscriptions::{self, Canceller};
use crate::texts;

/// The texts a template reads as `tx['subscription.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The package text `key` (without the `wl_invoicing.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_invoicing.{key}"))
}

/// The module's subscription routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/subscriptions", index)
                .name("wl_invoicing.subscriptions")
                .get("/subscriptions/{id}", show)
                .name("wl_invoicing.subscription")
                .require_permission(permissions::SUBSCRIPTIONS_VIEW),
        )
        .merge(
            Routes::new()
                .post("/subscriptions/{id}/cancel", cancel)
                .name("wl_invoicing.subscription_cancel")
                .require_permission(permissions::SUBSCRIPTIONS_MANAGE),
        )
}

/// The badge colour of a subscription state (the kit has info, success,
/// warning and error).
fn status_kind(status: SubscriptionStatus) -> &'static str {
    match status {
        SubscriptionStatus::Active => "success",
        SubscriptionStatus::PastDue => "warning",
        SubscriptionStatus::Suspended => "error",
        SubscriptionStatus::Paused | SubscriptionStatus::Cancelled => "info",
    }
}

/// `14 Mar 2027`, or an empty text.
fn day(date: Option<NaiveDate>) -> String {
    date.map(|d| d.format("%d %b %Y").to_string())
        .unwrap_or_default()
}

/// The party's display name, else its id.
async fn party_name(state: &AppState, party: wl_kernel::PartyId) -> Result<String> {
    Ok(wl_kernel::parties::directory(state)
        .find(&state.db, party)
        .await?
        .map(|p| p.display_name)
        .unwrap_or_else(|| format!("#{}", party.0)))
}

/// One row of the list.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Row {
    id: i64,
    plan: String,
    party: String,
    source: String,
    status: String,
    next_cycle: String,
    link: String,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/subscriptions.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: Vec<Row>,
}

/// `GET /subscriptions`: every subscription, newest first.
async fn index(State(state): State<AppState>, lang: Lang) -> Result<View> {
    let all = Subscription::query()
        .order_by_desc(Subscription::ID)
        .limit(200)
        .get(&state.db)
        .await?;
    let mut rows = Vec::with_capacity(all.len());
    for subscription in all {
        rows.push(Row {
            id: subscription.id,
            plan: subscription.plan.clone(),
            party: party_name(&state, subscription.party()).await?,
            source: subscription.source().words(),
            status: text(
                &lang,
                &format!("subscription.status.{}", subscription.status.as_str()),
            ),
            next_cycle: if subscription.status == SubscriptionStatus::Cancelled {
                String::new()
            } else {
                day(Some(subscription.next_cycle_date))
            },
            link: format!("/subscriptions/{}", subscription.id),
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
    }))
}

/// One open pause cause.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct CauseRow {
    id: i64,
    cause: String,
    reason: String,
    since: String,
    until: String,
}

/// One billed cycle.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct CycleRow {
    id: i64,
    period: String,
    invoice: String,
    link: String,
    status: String,
    balance: String,
}

/// What a subscription's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/subscription.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    title: String,
    subscription: Subscription,
    status_label: String,
    status_kind: &'static str,
    party: String,
    source: String,
    price: String,
    cycle: String,
    start: String,
    end: String,
    next_cycle: String,
    cancelled_on: String,
    cancel_on: String,
    causes: Vec<CauseRow>,
    cycles: Vec<CycleRow>,
    can_cancel: bool,
    cancel_description: String,
}

/// `GET /subscriptions/{id}`: the subscription's terms, pause causes and
/// cycles.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(subscription): Found<Subscription>,
) -> Result<View> {
    let causes = SubscriptionPause::open_of(&state.db, subscription.id)
        .await?
        .into_iter()
        .map(|p| CauseRow {
            id: p.id,
            cause: text(&lang, &format!("subscription.cause.{}", p.cause.as_str())),
            reason: p.reason,
            since: day(Some(p.started_on)),
            until: day(p.until),
        })
        .collect();
    let cycles = subscriptions::cycle_invoices(&state.db, subscription.id)
        .await?
        .into_iter()
        .map(|(cycle, invoice)| CycleRow {
            id: cycle.id,
            period: format!(
                "{} - {}",
                day(Some(cycle.period_start)),
                day(Some(cycle.period_end))
            ),
            invoice: invoice
                .number
                .clone()
                .unwrap_or_else(|| text(&lang, "draft_number")),
            link: format!("/invoices/{}", invoice.id),
            status: text(&lang, &format!("status.{}", invoice.status.as_str())),
            balance: amount_text(&invoice.currency, invoice.balance),
        })
        .collect();
    let can_cancel = user.has_permission(permissions::SUBSCRIPTIONS_MANAGE)
        && subscription.status != SubscriptionStatus::Cancelled;
    let cancel_description = if config_on(&state) {
        text(&lang, "subscription.cancel.prorated")
    } else {
        text(&lang, "subscription.cancel.not_prorated")
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: subscription.plan.clone(),
        status_label: text(
            &lang,
            &format!("subscription.status.{}", subscription.status.as_str()),
        ),
        status_kind: status_kind(subscription.status),
        party: party_name(&state, subscription.party()).await?,
        source: subscription.source().words(),
        price: amount_text(&subscription.currency, subscription.unit_price),
        cycle: text(
            &lang,
            &format!("subscription.cycle.{}", subscription.cycle.as_str()),
        ),
        start: day(Some(subscription.start_date)),
        end: day(subscription.end_date),
        next_cycle: day(Some(subscription.next_cycle_date)),
        cancelled_on: day(subscription.cancelled_on),
        cancel_on: day(subscription.cancel_on),
        causes,
        cycles,
        can_cancel,
        cancel_description,
        subscription,
    }))
}

/// Whether *pro rata on cancel* is on.
fn config_on(state: &AppState) -> bool {
    crate::config_of(state).subscription_pro_rata_on_cancel
}

/// `POST /subscriptions/{id}/cancel`: a user cancel on the day `on`
/// (`YYYY-MM-DD`, default today) with an optional reason.
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(subscription): Found<Subscription>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let back = Redirect::route("wl_invoicing.subscription", &[&subscription.id])?;
    let on = match form.get("on").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        None => today(),
        Some(given) => match given.parse::<NaiveDate>() {
            Ok(date) => date,
            Err(_) => {
                let toast = Toast::error(text(&lang, "subscription.cancel.bad_date"));
                return Ok((toast, back).into_response());
            }
        },
    };
    let reason = form.get("reason").map(String::as_str);
    let toast = match subscriptions::cancel(
        &state,
        &subscription,
        Canceller::User(&user),
        on,
        reason,
    )
    .await
    {
        Ok(after) if after.status == SubscriptionStatus::Cancelled => {
            Toast::success(text(&lang, "subscription.cancel.done"))
        }
        Ok(_) => Toast::success(text(&lang, "subscription.cancel.scheduled")),
        Err(TransitionError::Guard(message)) => Toast::error(message),
        Err(other) => return Err(other.into_error()),
    };
    Ok((toast, back).into_response())
}
