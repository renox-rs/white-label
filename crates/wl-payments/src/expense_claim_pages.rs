//! The expense claim page (wl-payments §3, §4, issue #90):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /expense-claims` | `wl_payments.expense_claims.index` | `expenses.view` |
//! | `POST /expense-claims` | `wl_payments.expense_claims.store` | `expenses.submit` |
//! | `POST /expense-claims/{id}/submit` | `wl_payments.expense_claims.submit` | `expenses.submit` |
//! | `POST /expense-claims/{id}/approve` | `wl_payments.expense_claims.approve` | `expenses.approve` |
//! | `POST /expense-claims/{id}/reject` | `wl_payments.expense_claims.reject` | `expenses.approve` (field `reason`) |
//! | `POST /expense-claims/{id}/return` | `wl_payments.expense_claims.return` | `expenses.approve` (field `reason`) |
//!
//! A claimant fills a claim with up to three lines (`line_<n>_category`,
//! `_amount`, `_vat`, `_date`, `_merchant`, `_description`) and submits it; an
//! approver approves, rejects or returns it ([`crate::expense_claim`]). The
//! template is compiled in and an app can replace it.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::TransitionError;
use serde::Serialize;
use wl_kernel::LegalEntity;

use crate::expense_category::ExpenseCategory;
use crate::expense_claim::{
    self, ClaimDrawing, ClaimStatus, ExpenseClaim, LineInput, drawing_problem,
};
use crate::pages::{invalid, parse_amount, texts_in};
use crate::permissions;
use crate::texts;

/// How many lines the form has.
const SLOTS: usize = 3;

/// The routes of the expense claim page.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/expense-claims", index)
                .name("wl_payments.expense_claims.index")
                .require_permission(permissions::EXPENSES_VIEW),
        )
        .merge(
            Routes::new()
                .post("/expense-claims", store)
                .name("wl_payments.expense_claims.store")
                .post("/expense-claims/{id}/submit", store_submit)
                .name("wl_payments.expense_claims.submit")
                .require_permission(permissions::EXPENSES_SUBMIT),
        )
        .merge(
            Routes::new()
                .post("/expense-claims/{id}/approve", store_approve)
                .name("wl_payments.expense_claims.approve")
                .post("/expense-claims/{id}/reject", store_reject)
                .name("wl_payments.expense_claims.reject")
                .post("/expense-claims/{id}/return", store_return)
                .name("wl_payments.expense_claims.return")
                .require_permission(permissions::EXPENSES_APPROVE),
        )
}

/// The claims grid: search on the title and the claimant, filters by entity
/// and status, sums of the total and the VAT. The note and the actions
/// (submit, approve, return, reject) are drawn by the template (`custom`).
pub fn grid(lang: &Lang, can_export: bool) -> Grid {
    let t = texts_in(lang);
    let statuses: Vec<(String, String)> = ["draft", "submitted", "approved", "rejected"]
        .iter()
        .map(|code| {
            (
                (*code).to_owned(),
                t(&format!("expense_claims.status.{code}")),
            )
        })
        .collect();
    let grid = Grid::new("wl-payments-expense-claims")
        .title(&t("expense_claims.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("title", &t("expense_claims.claim_title"))
                .frozen()
                .mobile()
                .searchable(),
        )
        .column(
            Column::related(
                "claimant",
                &t("expense_claims.claimant"),
                "users",
                "claimant_id",
                "name",
            )
            .mobile()
            .searchable(),
        )
        .column(Column::related(
            "entity",
            &t("fields.entity"),
            "wl_legal_entities",
            "legal_entity_id",
            "code",
        ))
        .column(
            Column::money("total", &wl_kernel::money_label(&t("expense_claims.total")))
                .mobile()
                .summary(Summary::Sum),
        )
        .column(
            Column::money(
                "vat_total",
                &wl_kernel::money_label(&t("expense_claims.vat")),
            )
            .hidden()
            .summary(Summary::Sum),
        )
        .column(
            Column::money(
                "drawing_amount",
                &wl_kernel::money_label(&t("expense_claims.drawing")),
            )
            .hidden()
            .summary(Summary::Sum),
        )
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("draft", "neutral"),
                    ("submitted", "info"),
                    ("approved", "success"),
                    ("rejected", "neutral"),
                ])
                .mobile(),
        )
        .column(Column::custom("note", &t("expense_claims.note")))
        .column(Column::datetime("created_at", &t("fields.created_at")).hidden())
        .column(Column::custom("actions", &t("expense_claims.actions")).mobile())
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(
            &t("expense_claims.empty"),
            Some(&t("expense_claims.empty_hint")),
        );
    if can_export { grid.exports() } else { grid }
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/expense_claims.html")]
pub(crate) struct IndexPage {
    tx: BTreeMap<&'static str, String>,
    claims: GridPage<ExpenseClaim>,
    /// The ids of the claims on the page that can be decided, for their sheets.
    deciding: Vec<i64>,
    /// The ids of the claims on the page that can be submitted.
    submitting: Vec<i64>,
    entities: Vec<(i64, String)>,
    categories: Vec<(String, String)>,
    slots: Vec<usize>,
    can_submit: bool,
}

/// `GET /expense-claims`: the claims, newest first, and the sheet of a new
/// one; or their export (`?export=`, for approvers).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let can_submit = user.has_permission(permissions::EXPENSES_SUBMIT);
    let can_approve = user.has_permission(permissions::EXPENSES_APPROVE);
    let grid = grid(&lang, can_approve);
    if request.param("export").is_some() && !can_approve {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(ExpenseClaim::query(), &request).await? {
        return Ok(file);
    }
    let entities: Vec<(i64, String)> = LegalEntity::query()
        .where_(LegalEntity::STATUS.eq("active"))
        .order_by(LegalEntity::CODE)
        .get(&state.db)
        .await?
        .into_iter()
        .map(|e| (e.id, format!("{} - {}", e.code, e.name_en)))
        .collect();
    let categories = ExpenseCategory::query()
        .where_(ExpenseCategory::ACTIVE.eq(true))
        .order_by(ExpenseCategory::POSITION)
        .get(&state.db)
        .await?
        .into_iter()
        .map(|c| (c.code, c.name_en))
        .collect();
    let claims = grid.page(ExpenseClaim::query(), &request).await?;
    // What the user may do with a claim of the page.
    let may_submit = |claim: &ExpenseClaim| {
        can_submit && claim.status == ClaimStatus::Draft && claim.claimant_id == user.id
    };
    let may_decide = |claim: &ExpenseClaim| {
        can_approve
            && claim.status == ClaimStatus::Submitted
            && claim.claimant_id != user.id
            && claim.drawing_status != Some(ClaimDrawing::Requested)
    };
    let deciding = claims
        .items()
        .iter()
        .filter(|claim| may_decide(claim))
        .map(|claim| claim.id)
        .collect();
    let submitting = claims
        .items()
        .iter()
        .filter(|claim| may_submit(claim))
        .map(|claim| claim.id)
        .collect();
    let claims = claims.extend(|claim| {
        // What holds a submitted claim back, or why it was refused.
        let note = if claim.status == ClaimStatus::Submitted {
            drawing_problem(claim).unwrap_or_default()
        } else {
            claim.decision_reason.clone().unwrap_or_default()
        };
        json!({
            "note": note,
            "can_submit": may_submit(claim),
            "can_decide": may_decide(claim),
        })
    });
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        claims,
        deciding,
        submitting,
        entities,
        categories,
        slots: (1..=SLOTS).collect(),
        can_submit,
    })
    .into_response())
}

/// The line rows of the form: a row with no category and no amount is
/// skipped. A field that is not valid is returned as its name and the key of
/// its error text.
fn lines_of_form(
    form: &BTreeMap<String, String>,
) -> std::result::Result<Vec<LineInput>, (String, &'static str)> {
    let get = |n: usize, field: &str| {
        form.get(&format!("line_{n}_{field}"))
            .map_or("", |v| v.trim())
    };
    let mut lines = Vec::new();
    for n in 1..=SLOTS {
        let (category, amount) = (get(n, "category"), get(n, "amount"));
        if category.is_empty() && amount.is_empty() {
            continue;
        }
        let name = |field: &str| format!("line_{n}_{field}");
        let expense_date = get(n, "date")
            .parse::<NaiveDate>()
            .map_err(|_| (name("date"), "bad_date"))?;
        let amount = parse_amount(amount).ok_or((name("amount"), "bad_amount"))?;
        let vat_amount = match get(n, "vat") {
            "" => 0,
            text => parse_amount(text).ok_or((name("vat"), "bad_amount"))?,
        };
        lines.push(LineInput {
            expense_date,
            merchant: get(n, "merchant").to_owned(),
            category: category.to_owned(),
            description: get(n, "description").to_owned(),
            amount,
            vat_amount,
        });
    }
    Ok(lines)
}

/// `POST /expense-claims`: records a draft claim.
async fn store(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let entity = form
        .get("legal_entity_id")
        .and_then(|v| v.trim().parse::<i64>().ok())
        .ok_or_else(|| invalid("legal_entity_id", t("unknown_entity"), &form))?;
    let lines = lines_of_form(&form).map_err(|(field, key)| invalid(&field, t(key), &form))?;
    let title = form.get("title").map_or("", |v| v.trim());
    expense_claim::create(&state.db, user.id, entity, title, &lines)
        .await
        .map_err(|e| match e {
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message) => {
                invalid("base", message, &form)
            }
            other => other,
        })?;
    Ok((
        Toast::success(t("expense_claims.recorded")),
        Redirect::route("wl_payments.expense_claims.index", &[])?,
    )
        .into_response())
}

fn done(lang: &Lang, key: &str) -> Result<Response> {
    Ok((
        Toast::success(texts_in(lang)(key)),
        Redirect::route("wl_payments.expense_claims.index", &[])?,
    )
        .into_response())
}

/// `POST /expense-claims/{id}/submit`.
async fn store_submit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    expense_claim::submit(&state, &user, id)
        .await
        .map_err(TransitionError::into_error)?;
    done(&lang, "expense_claims.submit.done")
}

/// `POST /expense-claims/{id}/approve`.
async fn store_approve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    expense_claim::approve(&state, &user, id)
        .await
        .map_err(TransitionError::into_error)?;
    done(&lang, "expense_claims.approve.done")
}

/// The reason sheet's form.
#[derive(serde::Deserialize, Serialize, Validate, Debug)]
pub struct ReasonForm {
    /// Why.
    #[validate(max = 500)]
    pub reason: Option<String>,
}

/// `POST /expense-claims/{id}/reject`.
async fn store_reject(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReasonForm>,
) -> Result<Response> {
    let reason = form.reason.clone().unwrap_or_default();
    match expense_claim::reject(&state, &user, id, &reason).await {
        Ok(_) => done(&lang, "expense_claims.reject.done"),
        Err(TransitionError::Guard(message)) => Err(invalid("reason", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// `POST /expense-claims/{id}/return`.
async fn store_return(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReasonForm>,
) -> Result<Response> {
    let reason = form.reason.clone().unwrap_or_default();
    match expense_claim::return_to_draft(&state, &user, id, &reason).await {
        Ok(_) => done(&lang, "expense_claims.return.done"),
        Err(TransitionError::Guard(message)) => Err(invalid("reason", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn the_rows_of_the_form_are_read_and_empty_rows_skipped() {
        let lines = lines_of_form(&form(&[
            ("line_1_category", "fuel"),
            ("line_1_amount", "105.00"),
            ("line_1_vat", "5"),
            ("line_1_date", "2027-03-10"),
            ("line_3_category", "owner_drawings"),
            ("line_3_amount", "2000"),
            ("line_3_date", "2027-03-11"),
        ]))
        .unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0].amount, lines[0].vat_amount), (10_500, 500));
        assert_eq!(lines[1].category, "owner_drawings");
        assert_eq!(lines[1].vat_amount, 0);
    }

    #[test]
    fn a_bad_field_is_named() {
        assert_eq!(
            lines_of_form(&form(&[
                ("line_1_category", "fuel"),
                ("line_1_amount", "x"),
                ("line_1_date", "2027-03-10"),
            ]))
            .unwrap_err(),
            ("line_1_amount".to_owned(), "bad_amount")
        );
        assert_eq!(
            lines_of_form(&form(&[
                ("line_1_category", "fuel"),
                ("line_1_amount", "5"),
                ("line_1_date", "10/03"),
            ]))
            .unwrap_err(),
            ("line_1_date".to_owned(), "bad_date")
        );
    }
}
