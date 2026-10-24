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

use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::approvals::{self, ApprovalCard};

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

/// The adjustments grid: search on the member's code and the reason, filter
/// by status, sum of the points. "Submit again" is drawn by the template
/// (`custom`), for the requester of a returned adjustment.
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let t = |key: &str| text(lang, key);
    let statuses: Vec<(String, String)> = AdjustmentStatus::ALL
        .iter()
        .map(|status| {
            (
                status.as_str().to_owned(),
                text(lang, &format!("adjust.status.{}", status.as_str())),
            )
        })
        .collect();
    let grid = Grid::new("sales-points-adjustments")
        .title(&t("adjust.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::related(
                "code",
                &t("adjust.account"),
                "wl_sales_loyalty_accounts",
                "account_id",
                "code",
            )
            .frozen()
            .mobile()
            .searchable(),
        )
        .column(
            Column::number("points", &t("adjust.points"))
                .mobile()
                .summary(Summary::Sum),
        )
        .column(Column::text("reason", &t("adjust.reason")).searchable())
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("draft", "warning"),
                    ("pending_approval", "info"),
                    ("applied", "success"),
                    ("rejected", "danger"),
                ])
                .mobile(),
        )
        .column(Column::text("outcome_reason", &t("adjust.outcome")))
        .column(Column::datetime("created_at", &t("adjust.created_at")).hidden())
        .column(Column::custom("actions", ""))
        .sort_by("-created_at")
        .cards_on_mobile()
        .empty_state(&t("adjust.empty"), Some(&t("adjust.empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/adjustments.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    adjustments: GridPage<PointsAdjustment>,
    /// The adjustments of the page the viewer may change and submit again.
    sheets: Vec<i64>,
    /// The cards of the adjustments that wait for a manager: where the manager
    /// who is not the requester approves, rejects or returns them.
    approvals: Vec<ApprovalCard>,
}

/// `GET /loyalty/adjustments`: the adjustments, newest first (`?export=` with
/// `loyalty.adjust`).
async fn index(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let can_adjust = user.has_permission(permissions::LOYALTY_ADJUST);
    let grid = grid(&lang, can_adjust);
    if request.param("export").is_some() && !can_adjust {
        return Err(Error::Forbidden);
    }
    let query = PointsAdjustment::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut cards = Vec::new();
    let mut sheets = Vec::new();
    let mut extra = Vec::new();
    for a in page.items() {
        if a.status == AdjustmentStatus::PendingApproval {
            cards.extend(approvals::cards_for(&db, &a.subject(), &user, &lang).await?);
        }
        // Closing an account is not changed, only submitted again.
        let can_resubmit = a.status == AdjustmentStatus::Draft
            && a.requested_by == user.id
            && can_adjust;
        if can_resubmit {
            sheets.push(a.id);
        }
        extra.push(json!({
            "can_resubmit": can_resubmit,
            "closes_account": a.closes_account,
        }));
    }
    let mut extra = extra.into_iter();
    let adjustments = page.extend(|_| extra.next().unwrap_or_default());
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        adjustments,
        sheets,
        approvals: cards,
    })
    .into_response())
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
