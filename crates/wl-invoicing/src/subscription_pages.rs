//! The subscription pages (wl-invoicing §4 *Subscription*, BL-INV-21):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /subscriptions` | `wl_invoicing.subscriptions` | `subscriptions.view` |
//! | `GET /subscriptions/{id}` | `wl_invoicing.subscription` | `subscriptions.view` |
//! | `POST /subscriptions/{id}/cancel` | `wl_invoicing.subscription_cancel` | `subscriptions.manage` |
//!
//! The list is a `renox::grid` (`?export=` also `subscriptions.manage`) with each subscription's plan, party, agreement, state and next
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
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::TransitionError;
use serde::Serialize;
use wl_kernel::documents::today;
use wl_kernel::subscriptions::{BillingCycle, SubscriptionStatus};

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

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/subscriptions.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: GridPage<Subscription>,
}

/// The source column in words, as SQL: each source type of `types` as its
/// entity and number ("Engagement #9", as [`wl_kernel::Subject::words`]
/// writes it), so the column reads, sorts, searches and exports as words and
/// never as `accounting.engagement:9`. A type missing from `types` (a row
/// saved while the page was built) shows its number alone, `#9`.
fn source_words_sql(types: &[String]) -> String {
    let cases: String = types
        .iter()
        .filter(|t| {
            t.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
        })
        .map(|t| {
            let words = wl_kernel::subject::words_of(t, 0);
            let prefix = words.strip_suffix('0').unwrap_or(&words);
            format!(
                " WHEN '{t}' THEN '{}' || {{T}}.\"source_id\"",
                prefix.replace('\'', "''")
            )
        })
        .collect();
    if cases.is_empty() {
        "'#' || {T}.\"source_id\"".to_owned()
    } else {
        format!("CASE {{T}}.\"source_type\"{cases} ELSE '#' || {{T}}.\"source_id\" END")
    }
}

/// The source types the subscriptions have, once each.
async fn source_types(db: &Db) -> Result<Vec<String>> {
    let mut types: Vec<String> = Subscription::query()
        .pluck::<String, _>(db, "source_type")
        .await?;
    types.sort();
    types.dedup();
    Ok(types)
}

/// The subscriptions grid: search on the plan and the source (in words),
/// filters by state and cycle, the price in each subscription's currency.
/// `source_types` are the types the source column names in words.
pub fn grid(lang: &Lang, can_export: bool, source_types: &[String]) -> Grid {
    let statuses: Vec<(String, String)> = SubscriptionStatus::ALL
        .iter()
        .map(|s| {
            (
                s.as_str().to_owned(),
                text(lang, &format!("subscription.status.{}", s.as_str())),
            )
        })
        .collect();
    let cycles: Vec<(String, String)> = BillingCycle::ALL
        .iter()
        .map(|c| {
            (
                c.as_str().to_owned(),
                text(lang, &format!("subscription.cycle.{}", c.as_str())),
            )
        })
        .collect();
    let grid = Grid::new("wl-invoicing-subscriptions")
        .title(&text(lang, "subscription.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("plan", &text(lang, "subscription.fields.plan"))
                .frozen()
                .mobile()
                .searchable()
                .link("/subscriptions/{id}"),
        )
        // The name comes from the party directory, not from a table.
        .column(Column::custom("party", &text(lang, "subscription.fields.party")).mobile())
        .column(
            Column::text("source", &text(lang, "subscription.fields.source"))
                .computed(&source_words_sql(source_types))
                .searchable(),
        )
        .column(
            Column::select(
                "status",
                &text(lang, "subscription.fields.status"),
                statuses,
            )
            .badges(&[
                ("active", "success"),
                ("past_due", "warning"),
                ("suspended", "danger"),
                ("paused", "info"),
                ("cancelled", "info"),
            ])
            .mobile(),
        )
        .column(
            // A cancelled subscription has no next cycle.
            Column::date(
                "next_cycle_date",
                &text(lang, "subscription.fields.next_cycle"),
            )
            .computed(
                "CASE WHEN {T}.\"status\" = 'cancelled' THEN NULL ELSE {T}.\"next_cycle_date\" END",
            )
            .mobile(),
        )
        .column(
            Column::money("unit_price", &text(lang, "subscription.fields.price"))
                .currency_from("currency")
                .hidden(),
        )
        .column(Column::select("cycle", &text(lang, "subscription.fields.cycle"), cycles).hidden())
        .column(Column::date("start_date", &text(lang, "subscription.fields.start")).hidden())
        .column(Column::date("end_date", &text(lang, "subscription.fields.end")).hidden())
        .sort_by("-id")
        .row_url("/subscriptions/{id}")
        .cards_on_mobile()
        .empty_state(&text(lang, "subscription.empty"), None);
    if can_export { grid.exports() } else { grid }
}

/// `GET /subscriptions`: the grid, or its export (`?export=csv|xlsx|print`,
/// with `subscriptions.manage`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let can_export = crate::may_export(&user, permissions::SUBSCRIPTIONS_MANAGE);
    let types = source_types(&state.db).await?;
    let grid = grid(&lang, can_export, &types);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(Subscription::query(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(Subscription::query(), &request).await?;
    let mut parties = BTreeMap::new();
    for subscription in page.items() {
        parties.insert(
            subscription.id,
            party_name(&state, subscription.party()).await?,
        );
    }
    let rows = page.extend(|subscription| json!({ "party": parties.get(&subscription.id) }));
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
    })
    .into_response())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_column_names_each_type_in_words() {
        let sql = source_words_sql(&["accounting.engagement".to_owned()]);
        assert_eq!(
            sql,
            "CASE {T}.\"source_type\" WHEN 'accounting.engagement' THEN 'Engagement #' || {T}.\"source_id\" \
             ELSE '#' || {T}.\"source_id\" END"
        );
        assert!(!sql.contains("':'"), "never type:id");
        assert_eq!(source_words_sql(&[]), "'#' || {T}.\"source_id\"");
        // A value that is not a subject type is left out of the SQL.
        assert_eq!(
            source_words_sql(&["x'; DROP".to_owned()]),
            "'#' || {T}.\"source_id\""
        );
    }
}
