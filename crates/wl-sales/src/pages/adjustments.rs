//! The points adjustment pages (wl-sales §4.7, `SAL.ACL.07`; BL-SAL-24).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /loyalty/adjustments` | `wl_sales.loyalty_adjustments` | `loyalty.view` |
//! | `POST /loyalty/accounts/{id}/adjust` | `wl_sales.loyalty_adjust` | `loyalty.adjust` |
//! | `POST /loyalty/accounts/{id}/close` | `wl_sales.loyalty_close` | `loyalty.adjust` |
//! | `POST /loyalty/adjustments/{id}/resubmit` | `wl_sales.loyalty_adjustment_resubmit` | `loyalty.adjust` |
//!
//! Above the limit (and for a closing with points) the adjustment waits for a
//! holder of `loyalty.manage` other than the requester, who decides under
//! "Needs your action" in the kernel's approvals.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use serde::Serialize;

use crate::adjustment::{AdjustmentStatus, PointsAdjustment};
use crate::adjustments;
use crate::loyalty_account::LoyaltyAccount;
use crate::{permissions, texts};

/// The texts a template reads as `tx['adjust.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the pages, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/loyalty/adjustments", index)
                .name("wl_sales.loyalty_adjustments")
                .require_permission(permissions::LOYALTY_VIEW),
        )
        .merge(
            Routes::new()
                .post("/loyalty/accounts/{id}/adjust", adjust)
                .name("wl_sales.loyalty_adjust")
                .post("/loyalty/accounts/{id}/close", close)
                .name("wl_sales.loyalty_close")
                .post("/loyalty/adjustments/{id}/resubmit", resubmit)
                .name("wl_sales.loyalty_adjustment_resubmit")
                .require_permission(permissions::LOYALTY_ADJUST),
        )
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

/// The badge colour of a status.
fn status_kind(status: AdjustmentStatus) -> &'static str {
    match status {
        AdjustmentStatus::Draft => "warning",
        AdjustmentStatus::PendingApproval => "info",
        AdjustmentStatus::Applied => "success",
        AdjustmentStatus::Rejected => "error",
    }
}

/// One adjustment as the list shows it.
#[derive(Serialize, Debug, Clone)]
pub struct AdjustmentRow {
    /// The primary key.
    pub id: i64,
    /// The member's code.
    pub code: String,
    /// The points, signed.
    pub points: i64,
    /// The reason.
    pub reason: String,
    /// Why it was rejected or returned.
    pub outcome: String,
    /// The status in words.
    pub status_label: String,
    /// The badge colour.
    pub status_kind: &'static str,
    /// The requester may change it and submit it again.
    pub can_resubmit: bool,
    /// Closing an account is not changed, only submitted again.
    pub closes_account: bool,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/adjustments.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    adjustments: Vec<AdjustmentRow>,
}

/// `GET /loyalty/adjustments`: the latest adjustments, newest first.
async fn index(State(db): State<Db>, user: AuthUser, lang: Lang) -> Result<View> {
    let mut rows = Vec::new();
    for a in PointsAdjustment::query()
        .order_by_desc(PointsAdjustment::ID)
        .get(&db)
        .await?
        .into_iter()
        .take(100)
    {
        let code = LoyaltyAccount::find(&db, a.account_id)
            .await?
            .map(|account| account.code)
            .unwrap_or_default();
        rows.push(AdjustmentRow {
            id: a.id,
            code,
            points: a.points,
            reason: a.reason.clone(),
            outcome: a.outcome_reason.clone().unwrap_or_default(),
            status_label: text(&lang, &format!("adjust.status.{}", a.status.as_str())),
            status_kind: status_kind(a.status),
            can_resubmit: a.status == AdjustmentStatus::Draft
                && a.requested_by == user.id
                && user.has_permission(permissions::LOYALTY_ADJUST),
            closes_account: a.closes_account,
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        adjustments: rows,
    }))
}

/// The points of the form, signed.
fn points_of(form: &BTreeMap<String, String>, lang: &Lang) -> Result<i64> {
    form.get("points")
        .map_or("", |v| v.trim())
        .trim_start_matches('+')
        .parse::<i64>()
        .map_err(|_| invalid("points", text(lang, "loyalty.errors.number"), form))
}

/// Where the toast and the redirect of a saved adjustment go.
fn saved(lang: &Lang, adjustment: &PointsAdjustment, applied: &str) -> Result<Response> {
    let key = if adjustment.status == AdjustmentStatus::PendingApproval {
        "adjust.waiting"
    } else {
        applied
    };
    Ok((
        Toast::success(text(lang, key)),
        Redirect::route("wl_sales.loyalty_adjustments", &[])?,
    )
        .into_response())
}

/// `POST /loyalty/accounts/{id}/adjust`: adjusts the points by `points`
/// (signed) with a `reason`.
async fn adjust(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(account): Found<LoyaltyAccount>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let points = points_of(&form, &lang)?;
    let reason = form.get("reason").map_or("", |v| v.trim());
    let made = adjustments::request(&state, user.id, account.id, points, reason).await?;
    saved(&lang, &made, "adjust.requested")
}

/// `POST /loyalty/accounts/{id}/close`: staff close the account with a
/// `reason`.
async fn close(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(account): Found<LoyaltyAccount>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let reason = form.get("reason").map_or("", |v| v.trim());
    let made = adjustments::close_account(&state, user.id, account.id, reason).await?;
    saved(&lang, &made, "adjust.closed")
}

/// `POST /loyalty/adjustments/{id}/resubmit`: changes the returned
/// adjustment (`points`, `reason`) and submits it again.
async fn resubmit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(adjustment): Found<PointsAdjustment>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let points = if adjustment.closes_account {
        adjustment.points
    } else {
        points_of(&form, &lang)?
    };
    let reason = form.get("reason").map_or("", |v| v.trim());
    let made = adjustments::resubmit(&state, user.id, adjustment.id, points, reason).await?;
    saved(&lang, &made, "adjust.resubmitted")
}
