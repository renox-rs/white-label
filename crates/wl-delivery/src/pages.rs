//! The pages (wl-delivery §6, DESIGN-SYSTEM): the dispatcher's board and the
//! driver's phone.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /deliveries` | `wl_delivery.index` | `deliveries.view` |
//! | `GET /deliveries/{id}` | `wl_delivery.show` | `deliveries.view` |
//! | `POST /deliveries/{id}/assign` | `wl_delivery.assign` | `deliveries.dispatch` (field `run_id`) |
//! | `POST /deliveries/{id}/unassign` | `wl_delivery.unassign` | `deliveries.dispatch` |
//! | `POST /deliveries/{id}/open-review` | `wl_delivery.open_review` | `deliveries.manage` (field `reason`) |
//! | `POST /deliveries/{id}/close-review` | `wl_delivery.close_review` | `deliveries.manage` (fields `outcome`, `note`) |
//! | `POST /deliveries/{id}/found` | `wl_delivery.found` | `deliveries.manage` (field `depot`) |
//! | `POST /deliveries/{id}/trace` | `wl_delivery.trace` | `deliveries.manage` (fields `kind`: `scan`, `note` or `depot_receipt`; `note`, `depot`) |
//! | `POST /deliveries/{id}/mark-lost` | `wl_delivery.mark_lost` | `deliveries.reverse` (fields `reason`, `liability`) |
//! | `GET /runs` | `wl_delivery.runs` | `runs.view` |
//! | `GET /runs/{id}` | `wl_delivery.run` | `runs.view` |
//! | `POST /runs` | `wl_delivery.runs_store` | `runs.dispatch` (fields `run_date`, `driver_user_id`, `vehicle_type`, `vehicle_asset_id`, `depot`) |
//! | `POST /runs/{id}/dispatch` | `wl_delivery.run_dispatch` | `runs.dispatch` |
//! | `POST /runs/{id}/receipts` | `wl_delivery.run_receipts` | `runs.dispatch` (fields `item_{delivery}_0` = `received` or `missing`; `received_{delivery}_{line}`, `missing_{delivery}_{line}`) |
//! | `POST /runs/{id}/cash` | `wl_delivery.run_cash` | `runs.dispatch` (field `amount`; not the run's driver) |
//! | `POST /runs/{id}/close` | `wl_delivery.run_close` | `runs.dispatch` |
//! | `POST /driver/runs/{id}/return` | `wl_delivery.run_return` | `runs.drive` (the run's driver) |
//! | `GET /driver` | `wl_delivery.driver` | `runs.drive` (today's stops of the user) |
//! | `GET /driver/stops/{id}` | `wl_delivery.stop` | `runs.drive` (404 for anyone else's stop) |
//! | `POST /driver/stops/{id}/deliver` | `wl_delivery.stop_deliver` | `deliveries.complete` and the run's driver |
//! | `POST /driver/stops/{id}/fail` | `wl_delivery.stop_fail` | `deliveries.complete` and the run's driver |
//!
//! Every POST calls the services of [`crate::plan`] and [`crate::proof`]
//! with the time of the request (`renox::db::now()`), never the generic
//! transition route, because the checks and the effects need what the
//! services work out. A refusal answers 422 with its message. Every template
//! is compiled in and can be replaced by the app
//! (`resources/views/wl-delivery/…`).

use std::collections::{BTreeMap, HashMap};

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{self, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::delivery::ReviewOutcome;
use wl_kernel::documents::{dubai_text, today};

use crate::close;
use crate::day_list::{StopView, stop_of, stops_of};
use crate::delivery::{Attempt, Delivery, DeliveryStatus};
use crate::lost;
use crate::permissions;
use crate::plan::{self, NewRun};
use crate::proof::{self, DeliverForm, LineEntry, aed, milli_text};
use crate::reasons::{FailureReason, RefusalReason};
use crate::review;
use crate::run::{Run, RunStatus, VehicleType};
use crate::texts;

/// The texts a template reads as `tx['fields.number']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

type Text<'a> = &'a dyn Fn(&str) -> String;

/// The package texts in `lang`.
pub(crate) fn texts_in(lang: &Lang) -> impl Fn(&str) -> String + '_ {
    move |key: &str| texts::t(lang, &format!("wl_delivery.{key}"))
}

/// The module's routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/deliveries", index)
                .name("wl_delivery.index")
                .get("/deliveries/{id}", show)
                .name("wl_delivery.show")
                .require_permission(permissions::VIEW),
        )
        .merge(
            Routes::new()
                .post("/deliveries/{id}/assign", store_assign)
                .name("wl_delivery.assign")
                .post("/deliveries/{id}/unassign", store_unassign)
                .name("wl_delivery.unassign")
                .require_permission(permissions::DISPATCH),
        )
        .merge(
            Routes::new()
                .post("/deliveries/{id}/open-review", store_open_review)
                .name("wl_delivery.open_review")
                .post("/deliveries/{id}/close-review", store_close_review)
                .name("wl_delivery.close_review")
                .post("/deliveries/{id}/found", store_found)
                .name("wl_delivery.found")
                .post("/deliveries/{id}/trace", store_trace)
                .name("wl_delivery.trace")
                .require_permission(permissions::MANAGE),
        )
        .merge(
            Routes::new()
                .post("/deliveries/{id}/mark-lost", store_mark_lost)
                .name("wl_delivery.mark_lost")
                .require_permission(permissions::REVERSE),
        )
        .merge(
            Routes::new()
                .get("/runs", runs)
                .name("wl_delivery.runs")
                .get("/runs/{id}", run_show)
                .name("wl_delivery.run")
                .require_permission(permissions::RUNS_VIEW),
        )
        .merge(
            Routes::new()
                .post("/runs", store_run)
                .name("wl_delivery.runs_store")
                .post("/runs/{id}/dispatch", store_dispatch)
                .name("wl_delivery.run_dispatch")
                .post("/runs/{id}/receipts", store_receipts)
                .name("wl_delivery.run_receipts")
                .post("/runs/{id}/cash", store_cash)
                .name("wl_delivery.run_cash")
                .post("/runs/{id}/close", store_close)
                .name("wl_delivery.run_close")
                .require_permission(permissions::RUNS_DISPATCH),
        )
        .merge(
            Routes::new()
                .get("/driver", driver)
                .name("wl_delivery.driver")
                .get("/driver/stops/{id}", stop)
                .name("wl_delivery.stop")
                .post("/driver/runs/{id}/return", store_return)
                .name("wl_delivery.run_return")
                .require_permission(permissions::RUNS_DRIVE),
        )
        .merge(
            Routes::new()
                .post("/driver/stops/{id}/deliver", store_deliver)
                .name("wl_delivery.stop_deliver")
                .post("/driver/stops/{id}/fail", store_fail)
                .name("wl_delivery.stop_fail")
                .require_permission(permissions::COMPLETE),
        )
}

/// The badge colour of a delivery status.
fn status_kind(status: DeliveryStatus) -> &'static str {
    match status {
        DeliveryStatus::Delivered => "success",
        DeliveryStatus::Attempted | DeliveryStatus::UnderReview => "warning",
        DeliveryStatus::Lost => "error",
        _ => "info",
    }
}

/// The badge colour of a run status.
fn run_status_kind(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Closed => "success",
        RunStatus::ReturnedToDepot => "warning",
        _ => "info",
    }
}

/// `(value, label)` pairs for a select, labelled with `prefix.value`.
fn options<'a>(
    t: Text<'_>,
    prefix: &str,
    values: impl IntoIterator<Item = &'a str>,
) -> Vec<(String, String)> {
    values
        .into_iter()
        .map(|value| (value.to_owned(), t(&format!("{prefix}.{value}"))))
        .collect()
}

/// `text` as a number scaled by 10^`decimals` (`3.5` at 3 is 3500): digits
/// with at most `decimals` after the point; `None` for anything else, and
/// for blank text.
pub(crate) fn parse_scaled(text: &str, decimals: usize) -> Option<i64> {
    let text = text.trim();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if (whole.is_empty() && fraction.is_empty())
        || !digits(whole)
        || !digits(fraction)
        || fraction.len() > decimals
    {
        return None;
    }
    let mut padded = fraction.to_owned();
    while padded.len() < decimals {
        padded.push('0');
    }
    let scale = 10_i64.checked_pow(u32::try_from(decimals).ok()?)?;
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let fraction: i64 = if padded.is_empty() {
        0
    } else {
        padded.parse().ok()?
    };
    whole.checked_mul(scale)?.checked_add(fraction)
}

/// A coordinate typed in degrees (`25.204849`, possibly negative) as
/// millionths of a degree.
fn parse_degrees(text: &str) -> Option<i64> {
    let text = text.trim();
    match text.strip_prefix('-') {
        Some(rest) => parse_scaled(rest, 6).map(|micro| -micro),
        None => parse_scaled(text, 6),
    }
}

// ---------------------------------------------------------------------------
// Deliveries
// ---------------------------------------------------------------------------

/// The deliveries grid: search on the number, recipient and area; filter by
/// status.
pub fn deliveries_grid(lang: &Lang) -> Grid {
    let t = texts_in(lang);
    let statuses = options(&t, "status", DeliveryStatus::ALL.iter().map(|s| s.as_str()));
    Grid::new("wl-delivery")
        .title(&t("title"))
        .column(
            Column::text("number", &t("fields.number"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/deliveries/{id}"),
        )
        .column(
            Column::text("recipient_name", &t("fields.recipient"))
                .mobile()
                .searchable(),
        )
        .column(Column::text("area", &t("fields.area")).searchable())
        .column(Column::datetime("window_from", &t("fields.window")))
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("pending", "neutral"),
                    ("assigned", "info"),
                    ("out_for_delivery", "info"),
                    ("attempted", "warning"),
                    ("delivered", "success"),
                    ("under_review", "warning"),
                    ("lost", "danger"),
                    ("returned", "neutral"),
                    ("discarded", "neutral"),
                    ("cancelled", "neutral"),
                ])
                .mobile(),
        )
        .column(Column::datetime("created_at", &t("fields.date")).hidden())
        .sort_by("-created_at")
        .row_url("/deliveries/{id}")
        .cards_on_mobile()
        .empty_state(&t("empty"), Some(&t("empty_hint")))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-delivery/index.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    deliveries: GridPage<Delivery>,
}

/// `GET /deliveries`: the grid.
async fn index(lang: Lang, request: GridRequest) -> Result<View> {
    let deliveries = deliveries_grid(&lang)
        .page(Delivery::query(), &request)
        .await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        deliveries,
    }))
}

/// One line of the delivery as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct LineRow {
    id: i64,
    description: String,
    quantity: String,
    delivered: String,
    refused: String,
    reason: Option<String>,
}

/// One visit.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct AttemptRow {
    id: i64,
    number: i64,
    outcome: String,
    reason: Option<String>,
    at: String,
}

/// The saved proof.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ProofRow {
    recipient_name: String,
    methods: String,
    at: String,
    cash_expected: String,
    cash_collected: String,
    cheque: Option<String>,
}

/// The delivery as the card shows it.
#[derive(Serialize, Debug, Clone, Default)]
pub(crate) struct Card {
    window: Option<String>,
    run: Option<String>,
    amount_to_collect: String,
    pod_required: String,
    partial_acceptance: String,
    attempts: String,
}

/// The review as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ReviewRow {
    reason: String,
    ticket: Option<String>,
    frozen: bool,
    note: Option<String>,
}

/// The loss as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct LostRow {
    reason: String,
    liability: String,
    claim: Option<String>,
}

/// What the delivery page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-delivery/show.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    delivery: Delivery,
    status_label: String,
    status_kind: &'static str,
    card: Card,
    lines: Vec<LineRow>,
    proof: Option<ProofRow>,
    attempts: Vec<AttemptRow>,
    timeline: Vec<Value>,
    can_assign: bool,
    can_unassign: bool,
    can_open_review: bool,
    can_close_review: bool,
    can_mark_lost: bool,
    can_find: bool,
    /// The loss of a delivery that is or was lost.
    lost: Option<LostRow>,
    /// `(value, label)` of who can bear a loss.
    liabilities: Vec<(String, String)>,
    /// The review of a delivery that is or was under review.
    review: Option<ReviewRow>,
    /// `(value, label)` of the outcomes a review closes with.
    review_outcomes: Vec<(String, String)>,
    /// The planned runs the assign sheet offers, `(id, label)`.
    planned_runs: Vec<(String, String)>,
}

/// The window as text: `from - to`.
fn window_text(delivery: &Delivery) -> Option<String> {
    match (delivery.window_from, delivery.window_to) {
        (Some(from), Some(to)) => Some(format!("{} - {}", dubai_text(from), dubai_text(to))),
        (Some(from), None) => Some(dubai_text(from)),
        _ => None,
    }
}

/// The proof methods as words.
fn methods_text(t: Text<'_>, methods: &str) -> String {
    methods
        .split(',')
        .filter(|m| !m.is_empty())
        .map(|m| t(&format!("method.{m}")))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `GET /deliveries/{id}`: the delivery.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(delivery): Found<Delivery>,
) -> Result<View> {
    let t = texts_in(&lang);
    let run = match delivery.run_id {
        Some(id) => Run::find(&state.db, id).await?,
        None => None,
    };
    let lines = delivery
        .lines(&state.db)
        .await?
        .into_iter()
        .map(|line| LineRow {
            id: line.id,
            description: line.description.clone(),
            quantity: milli_text(line.quantity_milli),
            delivered: milli_text(line.delivered_milli),
            refused: milli_text(line.refused_milli),
            reason: line.refusal_reason.clone(),
        })
        .collect();
    let attempts = delivery
        .attempts_made(&state.db)
        .await?
        .into_iter()
        .map(|attempt: Attempt| AttemptRow {
            id: attempt.id,
            number: attempt.number,
            outcome: t(&format!("outcome.{}", attempt.outcome.as_str())),
            reason: attempt.failure_reason.clone(),
            at: dubai_text(attempt.occurred_at),
        })
        .collect();
    let proof = match delivery.latest_proof(&state.db).await? {
        Some(proof) => {
            let at = Attempt::find(&state.db, proof.attempt_id)
                .await?
                .map(|a| dubai_text(a.occurred_at))
                .unwrap_or_default();
            Some(ProofRow {
                recipient_name: proof.recipient_name.clone(),
                methods: methods_text(&t, &proof.methods),
                at,
                cash_expected: aed(proof.cash_expected),
                cash_collected: aed(proof.cash_collected),
                cheque: proof.cheque_reference.clone(),
            })
        }
        None => None,
    };
    let history = state_machine::history(&state.db, Delivery::TABLE, delivery.id).await?;
    let timeline = state_machine::timeline(&history, Some(&state_machine::info_of::<Delivery>()));
    let dispatcher = user.has_permission(permissions::DISPATCH);
    let can_assign = dispatcher && delivery.status == DeliveryStatus::Pending;
    let can_unassign = dispatcher
        && delivery.status == DeliveryStatus::Assigned
        && run.as_ref().is_some_and(|r| r.status == RunStatus::Planned);
    let manager = user.has_permission(permissions::MANAGE);
    let can_open_review = manager && delivery.status == DeliveryStatus::Delivered;
    let can_close_review = manager && delivery.status == DeliveryStatus::UnderReview;
    let review = delivery.review_reason.as_ref().map(|reason| ReviewRow {
        reason: reason.clone(),
        ticket: delivery.review_ticket_id.map(|id| id.to_string()),
        frozen: delivery.frozen,
        note: delivery.review_note.clone(),
    });
    let review_outcomes = vec![("upheld".to_owned(), t("outcome.upheld"))];
    let driver_of_run = run.as_ref().is_some_and(|r| r.driver_user_id == user.id);
    let can_mark_lost = user.has_permission(permissions::REVERSE)
        && !driver_of_run
        && matches!(
            delivery.status,
            DeliveryStatus::OutForDelivery
                | DeliveryStatus::Attempted
                | DeliveryStatus::UnderReview
        );
    let can_find = manager && delivery.status == DeliveryStatus::Lost;
    let lost = delivery.lost_reason.as_ref().map(|reason| LostRow {
        reason: reason.clone(),
        liability: t(&format!(
            "liability.{}",
            delivery.lost_liability.as_deref().unwrap_or("none")
        )),
        claim: delivery.claim_ticket_id.map(|id| id.to_string()),
    });
    let liabilities = crate::lost::LIABILITIES
        .iter()
        .map(|key| ((*key).to_owned(), t(&format!("liability.{key}"))))
        .collect();
    let planned_runs = if can_assign {
        Run::query()
            .where_(Run::STATUS.eq(RunStatus::Planned))
            .order_by(Run::RUN_DATE)
            .get(&state.db)
            .await?
            .into_iter()
            .map(|run| {
                (
                    run.id.to_string(),
                    format!("{} ({})", run.number, run.run_date.format("%Y-%m-%d")),
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    let card = Card {
        window: window_text(&delivery),
        run: run.as_ref().map(|r| r.number.clone()),
        amount_to_collect: aed(delivery.amount_to_collect),
        pod_required: methods_text(&t, &delivery.pod_methods),
        partial_acceptance: t(if delivery.partial_acceptance {
            "partial.on"
        } else {
            "partial.off"
        }),
        attempts: format!("{} / {}", delivery.attempts, delivery.max_attempts),
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        status_label: t(&format!("status.{}", delivery.status.as_str())),
        status_kind: status_kind(delivery.status),
        delivery,
        card,
        lines,
        proof,
        attempts,
        timeline,
        can_assign,
        can_unassign,
        can_open_review,
        can_close_review,
        can_mark_lost,
        can_find,
        lost,
        liabilities,
        review,
        review_outcomes,
        planned_runs,
    }))
}

/// The form that puts a delivery on a run.
#[derive(Deserialize, Serialize, Debug)]
pub struct AssignForm {
    /// The run's id.
    pub run_id: i64,
}

/// `POST /deliveries/{id}/assign`: puts the delivery on the run. A refusal
/// (the run is not planned, the driver is on leave, a driver or vehicle
/// document is not valid on the run's date) answers 422 with the message.
async fn store_assign(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(delivery): Found<Delivery>,
    Form(form): Form<AssignForm>,
) -> Result<Response> {
    let run = Run::find_or_404(&state.db, form.run_id).await?;
    plan::assign(&state, &user, &delivery, &run)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_delivery.actions.assigned")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /deliveries/{id}/unassign`.
async fn store_unassign(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(delivery): Found<Delivery>,
) -> Result<Response> {
    plan::unassign(&state, &user, &delivery)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_delivery.actions.unassigned")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that puts a delivery under review.
#[derive(Deserialize, Serialize, Debug)]
pub struct OpenReviewForm {
    /// Why the delivery is disputed.
    pub reason: String,
}

/// `POST /deliveries/{id}/open-review`: puts the delivered delivery under
/// review; a ticket is opened. A blank reason or another state answers 422.
async fn store_open_review(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(delivery): Found<Delivery>,
    Form(form): Form<OpenReviewForm>,
) -> Result<Response> {
    review::open_review(&state, &user, &delivery, &form.reason)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_delivery.actions.review_opened")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that closes a review.
#[derive(Deserialize, Serialize, Debug)]
pub struct CloseReviewForm {
    /// How the review ends; only `upheld` is offered.
    #[serde(default)]
    pub outcome: String,
    /// The note the review is closed with (required).
    #[serde(default)]
    pub note: String,
}

/// `POST /deliveries/{id}/close-review`: closes the review as upheld. A
/// missing note, another outcome or another state answers 422.
async fn store_close_review(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(delivery): Found<Delivery>,
    Form(form): Form<CloseReviewForm>,
) -> Result<Response> {
    let outcome = match form.outcome.trim() {
        "" | "upheld" => ReviewOutcome::Upheld,
        _ => {
            return Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                texts::t(&lang, "wl_delivery.actions.close_review_description"),
            ));
        }
    };
    review::close_review(&state, &user, &delivery, outcome, &form.note)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_delivery.actions.review_closed")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that asks to mark a delivery lost.
#[derive(Deserialize, Serialize, Debug)]
pub struct MarkLostForm {
    /// Why the delivery is lost.
    #[serde(default)]
    pub reason: String,
    /// Who bears the loss: `driver`, `business`, `client` or `none`.
    #[serde(default)]
    pub liability: String,
}

/// `POST /deliveries/{id}/mark-lost`: asks to mark the delivery lost. With
/// the policy `delivery.mark_lost` the request waits for a second holder of
/// `deliveries.reverse`; the answer says so. A refusal answers 422.
async fn store_mark_lost(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(delivery): Found<Delivery>,
    Form(form): Form<MarkLostForm>,
) -> Result<Response> {
    let asked = lost::mark_lost(&state, &user, &delivery, &form.reason, &form.liability)
        .await
        .map_err(TransitionError::into_error)?;
    let key = match asked {
        lost::LostRequest::Lost => "wl_delivery.actions.mark_lost_done",
        lost::LostRequest::Requested(_) => "wl_delivery.actions.mark_lost_requested",
    };
    Ok((Toast::success(texts::t(&lang, key)), renox::HxRefresh).into_response())
}

/// The form that records that a lost delivery was found.
#[derive(Deserialize, Serialize, Debug)]
pub struct FoundForm {
    /// The depot where the goods were scanned or confirmed.
    #[serde(default)]
    pub depot: String,
}

/// `POST /deliveries/{id}/found`: the goods of a lost delivery were found
/// at a depot; it goes back to `attempted`.
async fn store_found(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(delivery): Found<Delivery>,
    Form(form): Form<FoundForm>,
) -> Result<Response> {
    lost::found(&state, &user, &delivery, &form.depot)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_delivery.actions.found_done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /driver/runs/{id}/return`: the driver brings the run back to the
/// depot; every stop needs an outcome first.
async fn store_return(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(run): Found<Run>,
) -> Result<Response> {
    close::return_to_depot(&state, &user, &run)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_delivery.close.returned")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The answer to a receipt or a count: the run closed, or what it waits for.
fn closing_toast(lang: &Lang, closed: &close::CloseState, saved: &str) -> Response {
    let key = match closed {
        close::CloseState::Closed => "wl_delivery.close.closed",
        close::CloseState::Waiting(_) => saved,
    };
    (Toast::success(texts::t(lang, key)), renox::HxRefresh).into_response()
}

/// `POST /runs/{id}/receipts`: the depot ticks the items of a run that is
/// back. For an undelivered delivery the field `item_{delivery}_0` is
/// `received` or `missing`; for a refused line the quantities
/// `received_{delivery}_{line}` and `missing_{delivery}_{line}` (one of them
/// is enough: the other is what is left). Items without a field are left as
/// they are. The run closes when this completes it.
async fn store_receipts(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(fields): Form<HashMap<String, String>>,
) -> Result<Response> {
    let run = Run::find_or_404(&state.db, id).await?;
    let mut entries = Vec::new();
    for item in close::items_of(&state.db, &run).await? {
        if item.line_id == 0 {
            let name = format!("item_{}_0", item.delivery_id);
            let (received, missing) = match field(&fields, &name) {
                Some("received") => (close::WHOLE, 0),
                Some("missing") => (0, close::WHOLE),
                Some(_) => {
                    return Err(unprocessable(format!(
                        "{}: choose received or missing.",
                        item.number
                    )));
                }
                None => continue,
            };
            entries.push(close::Entry {
                delivery_id: item.delivery_id,
                line_id: 0,
                received_milli: received,
                missing_milli: missing,
            });
            continue;
        }
        let quantity = |name: String| -> Result<Option<i64>> {
            match field(&fields, &name) {
                Some(text) => parse_scaled(text, 3).map(Some).ok_or_else(|| {
                    unprocessable(format!("{}: {name} is not a number.", item.description))
                }),
                None => Ok(None),
            }
        };
        let received = quantity(format!("received_{}_{}", item.delivery_id, item.line_id))?;
        let missing = quantity(format!("missing_{}_{}", item.delivery_id, item.line_id))?;
        let (received, missing) = match (received, missing) {
            (None, None) => continue,
            (Some(r), Some(m)) => (r, m),
            (Some(r), None) => (r, item.expected_milli - r),
            (None, Some(m)) => (item.expected_milli - m, m),
        };
        entries.push(close::Entry {
            delivery_id: item.delivery_id,
            line_id: item.line_id,
            received_milli: received,
            missing_milli: missing,
        });
    }
    let closed = close::receive(&state, &user, &run, &entries)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(closing_toast(
        &lang,
        &closed,
        "wl_delivery.close.receipts_saved",
    ))
}

/// `POST /runs/{id}/cash`: the depot records the cash it counted (field
/// `amount`, AED); never the run's driver. The run closes when this
/// completes it.
async fn store_cash(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(run): Found<Run>,
    Form(fields): Form<HashMap<String, String>>,
) -> Result<Response> {
    let amount = field(&fields, "amount")
        .and_then(|text| parse_scaled(text, 2))
        .ok_or_else(|| unprocessable("The cash counted is not an amount.".to_owned()))?;
    let closed = close::count_cash(&state, &user, &run, amount)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(closing_toast(
        &lang,
        &closed,
        "wl_delivery.close.cash_saved",
    ))
}

/// `POST /runs/{id}/close`: closes the run when everything is accounted for;
/// otherwise answers 422 with what it waits for.
async fn store_close(
    State(state): State<AppState>,
    lang: Lang,
    Found(run): Found<Run>,
) -> Result<Response> {
    match close::try_close(&state, run.id)
        .await
        .map_err(TransitionError::into_error)?
    {
        close::CloseState::Closed => Ok((
            Toast::success(texts::t(&lang, "wl_delivery.close.closed")),
            renox::HxRefresh,
        )
            .into_response()),
        close::CloseState::Waiting(reasons) => Err(unprocessable(reasons.join(" "))),
    }
}

/// The form that records a scan, a note or a depot receipt.
#[derive(Deserialize, Serialize, Debug)]
pub struct TraceForm {
    /// `scan`, `note` or `depot_receipt`.
    #[serde(default)]
    pub kind: String,
    /// The note.
    #[serde(default)]
    pub note: String,
    /// The depot of a scan or a receipt.
    #[serde(default)]
    pub depot: String,
}

/// `POST /deliveries/{id}/trace`: records that the goods were traced (a scan
/// at a depot, a note, a depot receipt); the untraced clock of the lost rule
/// restarts, and a manual loss waiting for approval fails.
async fn store_trace(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(delivery): Found<Delivery>,
    Form(form): Form<TraceForm>,
) -> Result<Response> {
    crate::trace::record(
        &state,
        &user,
        &delivery,
        form.kind.trim(),
        &form.note,
        &form.depot,
    )
    .await
    .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_delivery.actions.traced")),
        renox::HxRefresh,
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// Runs
// ---------------------------------------------------------------------------

/// The runs grid.
pub fn runs_grid(lang: &Lang) -> Grid {
    let t = texts_in(lang);
    let statuses = options(&t, "run_status", RunStatus::ALL.iter().map(|s| s.as_str()));
    let vehicles = options(&t, "vehicle", VehicleType::ALL.iter().map(|s| s.as_str()));
    Grid::new("wl-delivery-runs")
        .title(&t("runs.title"))
        .column(
            Column::text("number", &t("fields.number"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/runs/{id}"),
        )
        .column(Column::date("run_date", &t("fields.date")).mobile())
        .column(
            Column::text("driver", &t("fields.driver"))
                .computed(
                    "(SELECT \"users\".\"name\" FROM \"users\" WHERE \"users\".\"id\" = {T}.\"driver_user_id\")",
                )
                .searchable(),
        )
        .column(Column::select(
            "vehicle_type",
            &t("fields.vehicle_type"),
            vehicles,
        ))
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("planned", "neutral"),
                    ("dispatched", "info"),
                    ("returned_to_depot", "warning"),
                    ("closed", "success"),
                    ("cancelled", "neutral"),
                ])
                .mobile(),
        )
        .sort_by("-run_date")
        .row_url("/runs/{id}")
        .cards_on_mobile()
        .empty_state(&t("runs.empty"), None)
}

/// What the runs page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-delivery/runs.html")]
pub(crate) struct RunsPage {
    tx: Texts,
    can_dispatch: bool,
    vehicle_types: Vec<(String, String)>,
    runs: GridPage<Run>,
}

/// `GET /runs`.
async fn runs(lang: Lang, user: AuthUser, request: GridRequest) -> Result<View> {
    let t = texts_in(&lang);
    let runs = runs_grid(&lang).page(Run::query(), &request).await?;
    Ok(View::typed(RunsPage {
        tx: texts::all(&lang),
        can_dispatch: user.has_permission(permissions::RUNS_DISPATCH),
        vehicle_types: options(&t, "vehicle", VehicleType::ALL.iter().map(|v| v.as_str())),
        runs,
    }))
}

/// The form that plans a run.
#[derive(Deserialize, Serialize, Debug)]
pub struct RunForm {
    /// The day, `2026-11-12`.
    pub run_date: NaiveDate,
    /// The driver's user id.
    pub driver_user_id: i64,
    /// The vehicle type's key.
    pub vehicle_type: String,
    /// The vehicle's asset id; blank for an on-foot rider.
    #[serde(default)]
    pub vehicle_asset_id: Option<String>,
    /// The depot.
    #[serde(default)]
    pub depot: Option<String>,
}

/// `POST /runs`: plans a run.
async fn store_run(
    State(state): State<AppState>,
    lang: Lang,
    Form(form): Form<RunForm>,
) -> Result<Response> {
    let vehicle_type = VehicleType::parse(&form.vehicle_type).ok_or_else(|| {
        Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Unknown vehicle type.".to_owned(),
        )
    })?;
    let vehicle_asset_id = match form.vehicle_asset_id.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(text) => Some(text.parse::<i64>().map_err(|_| {
            Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                "The vehicle is an asset id.".to_owned(),
            )
        })?),
    };
    let mut new =
        NewRun::new(form.run_date, form.driver_user_id).vehicle(vehicle_type, vehicle_asset_id);
    new.depot = form.depot.clone().unwrap_or_default();
    let run = plan::create_run(&state, new).await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_delivery.runs.created")),
        Redirect::route("wl_delivery.run", &[&run.id])?,
    )
        .into_response())
}

/// The run as the card shows it.
#[derive(Serialize, Debug, Clone, Default)]
pub(crate) struct RunCard {
    date: String,
    driver: String,
    vehicle_type: String,
    vehicle: Option<String>,
}

/// What the run page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-delivery/run.html")]
pub(crate) struct RunPage {
    tx: Texts,
    run: Run,
    status_label: String,
    status_kind: &'static str,
    card: RunCard,
    stops: Vec<StopRow>,
    can_dispatch: bool,
    /// The depot's list of a run that is back; none before.
    depot: Option<DepotView>,
}

/// One item the depot accounts for.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ItemRow {
    delivery_id: i64,
    line_id: i64,
    number: String,
    description: String,
    whole: bool,
    expected: String,
    received: String,
    missing: String,
    accounted: bool,
}

/// The depot section of a run that is back or closed.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct DepotView {
    items: Vec<ItemRow>,
    cash_expected: String,
    cash_counted: Option<String>,
    /// Whether the viewer may tick items and count cash.
    can_receive: bool,
    /// Whether the cash is still to be counted.
    cash_open: bool,
    open: bool,
}

/// One stop of a run.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct StopRow {
    id: i64,
    sequence: i64,
    number: String,
    recipient: String,
    area: String,
    status: String,
}

/// `GET /runs/{id}`.
async fn run_show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(run): Found<Run>,
) -> Result<View> {
    let t = texts_in(&lang);
    let driver = User::find(&state.db, run.driver_user_id)
        .await?
        .map_or_else(|| format!("#{}", run.driver_user_id), |u| u.name);
    let stops = Delivery::query()
        .where_(Delivery::RUN_ID.eq(run.id))
        .order_by(Delivery::RUN_SEQUENCE)
        .get(&state.db)
        .await?
        .into_iter()
        .map(|d| StopRow {
            id: d.id,
            sequence: d.run_sequence.unwrap_or_default(),
            number: d.number.clone(),
            recipient: d.recipient_name.clone(),
            area: d.area.clone(),
            status: t(&format!("status.{}", d.status.as_str())),
        })
        .collect();
    let card = RunCard {
        date: run.run_date.format("%Y-%m-%d").to_string(),
        driver,
        vehicle_type: t(&format!("vehicle.{}", run.vehicle_type.as_str())),
        vehicle: run.vehicle_asset_id.map(|id| format!("#{id}")),
    };
    let depot = if matches!(run.status, RunStatus::ReturnedToDepot | RunStatus::Closed) {
        let items = close::items_of(&state.db, &run)
            .await?
            .into_iter()
            .map(|item| ItemRow {
                delivery_id: item.delivery_id,
                line_id: item.line_id,
                number: item.number.clone(),
                description: item.description.clone(),
                whole: item.line_id == 0,
                expected: milli_text(item.expected_milli),
                received: milli_text(item.received_milli),
                missing: milli_text(item.missing_milli),
                accounted: item.accounted,
            })
            .collect();
        let expected = close::cash_expected(&state.db, &run).await?;
        Some(DepotView {
            items,
            cash_expected: aed(expected),
            cash_counted: run.cash_counted_amount.map(aed),
            can_receive: user.has_permission(permissions::RUNS_DISPATCH)
                && run.status == RunStatus::ReturnedToDepot,
            cash_open: expected > 0 && run.cash_counted_at.is_none(),
            open: run.status == RunStatus::ReturnedToDepot,
        })
    } else {
        None
    };
    Ok(View::typed(RunPage {
        tx: texts::all(&lang),
        status_label: t(&format!("run_status.{}", run.status.as_str())),
        status_kind: run_status_kind(run.status),
        can_dispatch: user.has_permission(permissions::RUNS_DISPATCH)
            && run.status == RunStatus::Planned,
        depot,
        run,
        card,
        stops,
    }))
}

/// `POST /runs/{id}/dispatch`: dispatches the run. A refusal (a driver or
/// vehicle document is not valid today) answers 422 with the message and the
/// run stays planned.
async fn store_dispatch(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(run): Found<Run>,
) -> Result<Response> {
    plan::dispatch_run(&state, &user, &run)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_delivery.actions.dispatched")),
        renox::HxRefresh,
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// The driver's phone
// ---------------------------------------------------------------------------

/// What the day list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-delivery/driver.html")]
pub(crate) struct DriverPage {
    tx: Texts,
    today: String,
    stops: Vec<StopView>,
    /// `(id, number)` of the driver's runs of today that are out, which the
    /// driver brings back to the depot.
    returnable: Vec<(i64, String)>,
}

/// `GET /driver`: today's stops of the signed-in driver.
async fn driver(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    let day = today();
    let stops = stops_of(&state.db, user.id, day)
        .await?
        .iter()
        .map(|stop| stop.view())
        .collect();
    let returnable = Run::query()
        .where_(Run::DRIVER_USER_ID.eq(user.id))
        .where_(Run::RUN_DATE.eq(day))
        .where_(Run::STATUS.eq(RunStatus::Dispatched))
        .order_by(Run::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .map(|run| (run.id, run.number))
        .collect();
    Ok(View::typed(DriverPage {
        tx: texts::all(&lang),
        today: day.format("%Y-%m-%d").to_string(),
        stops,
        returnable,
    }))
}

/// What the stop page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-delivery/stop.html")]
pub(crate) struct StopPage {
    tx: Texts,
    delivery: Delivery,
    stop: StopView,
    status_label: String,
    status_kind: &'static str,
    amount_to_collect: String,
    pod_required: String,
    partial_acceptance: bool,
    lines: Vec<LineRow>,
    refusal_reasons: Vec<(String, String)>,
    failure_reasons: Vec<(String, String)>,
    otp_options: Vec<(String, String)>,
}

/// `GET /driver/stops/{id}`: the stop; 404 unless it is on a run of the
/// signed-in driver today (`WRK.ACL.05`).
async fn stop(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let t = texts_in(&lang);
    let found = stop_of(&state.db, user.id, today(), id)
        .await?
        .ok_or(Error::NotFound)?;
    let delivery = found.delivery.clone();
    let lines = delivery
        .lines(&state.db)
        .await?
        .into_iter()
        .map(|line| LineRow {
            id: line.id,
            description: line.description.clone(),
            quantity: milli_text(line.quantity_milli),
            delivered: milli_text(line.delivered_milli),
            refused: milli_text(line.refused_milli),
            reason: line.refusal_reason.clone(),
        })
        .collect();
    let refusal_reasons = RefusalReason::active_ones(&state.db)
        .await?
        .into_iter()
        .map(|r| (r.code, r.name_en))
        .collect();
    let failure_reasons = FailureReason::active_ones(&state.db)
        .await?
        .into_iter()
        .map(|r| (r.code, r.name_en))
        .collect();
    Ok(View::typed(StopPage {
        tx: texts::all(&lang),
        stop: found.view(),
        status_label: t(&format!("status.{}", delivery.status.as_str())),
        status_kind: status_kind(delivery.status),
        amount_to_collect: aed(delivery.amount_to_collect),
        pod_required: methods_text(&t, &delivery.pod_methods),
        partial_acceptance: delivery.partial_acceptance,
        lines,
        refusal_reasons,
        failure_reasons,
        otp_options: vec![("0".to_owned(), t("no")), ("1".to_owned(), t("yes"))],
        delivery,
    }))
}

fn unprocessable(message: String) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// The text field `name` of the posted form, blank as none.
fn field<'a>(fields: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    fields.get(name).map(|v| v.trim()).filter(|v| !v.is_empty())
}

/// `POST /driver/stops/{id}/deliver`: records the outcome *delivered*. The
/// fields: `recipient_name`, `photo`, `signature`, `otp_verified` (`1`),
/// `latitude`, `longitude` (degrees), `cash_collected` (AED), `cheque`,
/// `notes`, and per line `delivered_{id}`, `refused_{id}`, `reason_{id}`,
/// `photo_{id}` (quantities as decimals; the refused fields are absent when
/// partial acceptance is off).
async fn store_deliver(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(fields): Form<HashMap<String, String>>,
) -> Result<Response> {
    let delivery = Delivery::find_or_404(&state.db, id).await?;
    let mut entries = Vec::new();
    for line in delivery.lines(&state.db).await? {
        let delivered = match field(&fields, &format!("delivered_{}", line.id)) {
            Some(text) => parse_scaled(text, 3).ok_or_else(|| {
                unprocessable(format!(
                    "{}: the delivered quantity is not a number.",
                    line.description
                ))
            })?,
            None => line.quantity_milli,
        };
        let refused = match field(&fields, &format!("refused_{}", line.id)) {
            Some(text) => parse_scaled(text, 3).ok_or_else(|| {
                unprocessable(format!(
                    "{}: the refused quantity is not a number.",
                    line.description
                ))
            })?,
            None => 0,
        };
        entries.push(LineEntry {
            line_id: line.id,
            delivered_milli: delivered,
            refused_milli: refused,
            reason: field(&fields, &format!("reason_{}", line.id)).map(str::to_owned),
            photo: field(&fields, &format!("photo_{}", line.id)).map(str::to_owned),
        });
    }
    let cash_collected = match field(&fields, "cash_collected") {
        Some(text) => parse_scaled(text, 2)
            .ok_or_else(|| unprocessable("The cash collected is not an amount.".to_owned()))?,
        None => 0,
    };
    let form = DeliverForm {
        recipient_name: field(&fields, "recipient_name")
            .unwrap_or_default()
            .to_owned(),
        photo: field(&fields, "photo").map(str::to_owned),
        signature: field(&fields, "signature").map(str::to_owned),
        otp_verified: field(&fields, "otp_verified") == Some("1"),
        latitude_micro: field(&fields, "latitude").and_then(parse_degrees),
        longitude_micro: field(&fields, "longitude").and_then(parse_degrees),
        lines: entries,
        cash_collected,
        cheque: field(&fields, "cheque").map(str::to_owned),
        notes: field(&fields, "notes").unwrap_or_default().to_owned(),
    };
    proof::deliver(&state, &user, &delivery, form, renox::db::now())
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_delivery.driver.delivered")),
        Redirect::route("wl_delivery.driver", &[])?,
    )
        .into_response())
}

/// `POST /driver/stops/{id}/fail`: records a failed visit (`reason`,
/// `latitude`, `longitude`).
async fn store_fail(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(fields): Form<HashMap<String, String>>,
) -> Result<Response> {
    let delivery = Delivery::find_or_404(&state.db, id).await?;
    let location = (
        field(&fields, "latitude").and_then(parse_degrees),
        field(&fields, "longitude").and_then(parse_degrees),
    );
    let at: DateTime = renox::db::now();
    proof::fail(
        &state,
        &user,
        &delivery,
        field(&fields, "reason").unwrap_or_default(),
        location,
        at,
    )
    .await
    .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_delivery.driver.failed")),
        Redirect::route("wl_delivery.driver", &[])?,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantities_and_amounts_are_parsed_as_scaled_decimals() {
        assert_eq!(parse_scaled("3", 3), Some(3_000));
        assert_eq!(parse_scaled("2.5", 3), Some(2_500));
        assert_eq!(parse_scaled("130.00", 2), Some(13_000));
        assert_eq!(parse_scaled("0.1234", 3), None);
        assert_eq!(parse_scaled("abc", 3), None);
        assert_eq!(parse_scaled("", 3), None);
    }

    #[test]
    fn coordinates_are_parsed_in_millionths_of_a_degree() {
        assert_eq!(parse_degrees("25.204849"), Some(25_204_849));
        assert_eq!(parse_degrees("-0.5"), Some(-500_000));
        assert_eq!(parse_degrees("north"), None);
    }
}
