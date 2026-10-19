//! The company budget pages (wl-invoicing §6, DESIGN-SYSTEM):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /budgets` | `wl_invoicing.budgets` | `budgets.view` |
//! | `GET /budgets/{id}` | `wl_invoicing.budget` | `budgets.view` |
//! | `GET /budgets/new` | `wl_invoicing.budget_new` | `budgets.manage` |
//! | `POST /budgets` | `wl_invoicing.budget_create` | `budgets.manage` |
//! | `POST /budgets/{id}/lines` | `wl_invoicing.budget_set_line` | `budgets.manage` |
//! | `POST /budgets/{id}/lines/clear` | `wl_invoicing.budget_clear_line` | `budgets.manage` |
//! | `POST /budgets/{id}/import` | `wl_invoicing.budget_import` | `budgets.manage` |
//! | `POST /budgets/{id}/revision-reason` | `wl_invoicing.budget_revision_reason` | `budgets.manage` |
//! | `POST /budgets/{id}/submit` | `wl_invoicing.budget_submit` | `budgets.manage` |
//! | `POST /budgets/{id}/discard` | `wl_invoicing.budget_discard` | `budgets.manage` |
//!
//! The list is a `renox::grid` limited to the entities of the user's scope.
//! A version's page shows its figures (an amount per category and month),
//! the approval card (`approve`, `reject` and `return` are the kernel's
//! buttons; the version's approvers are the holders of `budgets.approve`),
//! and, for a `draft`, the sheets that set an amount for a category over a
//! range of months, import pasted CSV (a dry run first) and discard the
//! draft. The templates are compiled in and can be replaced by the app
//! (`resources/views/wl-invoicing/…`).

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{self, TransitionError};
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::LegalEntity;
use wl_kernel::approvals::{self, ApprovalCard};
use wl_kernel::documents::today;

use crate::allocate::amount_text;
use crate::budget::{
    BudgetStatus, CompanyBudget, CompanyBudgetLine, StartChoice, parse_budget_amount,
};
use crate::budget_category::BudgetCategory;
use crate::budget_lines::{self, LineInput, NewBudget, parse_month};
use crate::budget_machine::{self, start_key};
use crate::config_of;
use crate::credit_notes::By;
use crate::permissions;
use crate::texts;

/// The texts a template reads as `tx['budget.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The package text `key` (without the `wl_invoicing.` prefix) in a language.
type Text<'a> = &'a dyn Fn(&str) -> String;

/// The package texts in `lang`.
fn texts_in(lang: &Lang) -> impl Fn(&str) -> String + '_ {
    move |key: &str| texts::t(lang, &format!("wl_invoicing.{key}"))
}

/// The module's budget routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/budgets", index)
                .name("wl_invoicing.budgets")
                .get("/budgets/{id}", show)
                .name("wl_invoicing.budget")
                .require_permission(permissions::BUDGETS_VIEW),
        )
        .merge(
            Routes::new()
                .get("/budgets/new", new_form)
                .name("wl_invoicing.budget_new")
                .post("/budgets", store)
                .name("wl_invoicing.budget_create")
                .post("/budgets/{id}/lines", set_line)
                .name("wl_invoicing.budget_set_line")
                .post("/budgets/{id}/lines/clear", clear_line)
                .name("wl_invoicing.budget_clear_line")
                .post("/budgets/{id}/import", import)
                .name("wl_invoicing.budget_import")
                .post("/budgets/{id}/revision-reason", revision_reason)
                .name("wl_invoicing.budget_revision_reason")
                .post("/budgets/{id}/submit", submit)
                .name("wl_invoicing.budget_submit")
                .post("/budgets/{id}/discard", discard)
                .name("wl_invoicing.budget_discard")
                .require_permission(permissions::BUDGETS_MANAGE),
        )
}

/// The badge colour of a budget status (the kit has info, success, warning
/// and error).
pub(crate) fn status_kind(status: BudgetStatus) -> &'static str {
    match status {
        BudgetStatus::Draft | BudgetStatus::Superseded | BudgetStatus::Discarded => "info",
        BudgetStatus::Submitted => "warning",
        BudgetStatus::Approved => "success",
    }
}

/// An amount as the budget table writes it: `20,000.00`.
fn plain(fils: i64) -> String {
    amount_text("AED", fils)
        .trim_start_matches("AED ")
        .to_owned()
}

/// `Jan 2027`.
pub(crate) fn month_label(month: NaiveDate) -> String {
    month.format("%b %Y").to_string()
}

/// The budgets grid: filters by entity and status.
pub fn grid(lang: &Lang) -> Grid {
    grid_with(&texts_in(lang))
}

/// [`grid`] with the texts of `t`.
fn grid_with(t: Text<'_>) -> Grid {
    let statuses: Vec<(String, String)> = BudgetStatus::ALL
        .iter()
        .map(|s| {
            (
                s.as_str().to_owned(),
                t(&format!("budget.status.{}", s.as_str())),
            )
        })
        .collect();
    Grid::new("wl-invoicing-budgets")
        .title(&t("budget.title"))
        .column(
            Column::related(
                "entity",
                &t("budget.fields.entity"),
                "wl_legal_entities",
                "legal_entity_id",
                "code",
            )
            .frozen()
            .mobile(),
        )
        .column(
            Column::number("fiscal_year", &t("budget.fields.year"))
                .mobile()
                .link("/budgets/{id}"),
        )
        .column(Column::number("version", &t("budget.fields.version")).mobile())
        .column(Column::text("name", &t("budget.fields.name")))
        .column(
            Column::select("status", &t("budget.fields.status"), statuses)
                .badges(&[
                    ("draft", "info"),
                    ("submitted", "warning"),
                    ("approved", "success"),
                    ("superseded", "info"),
                    ("discarded", "info"),
                ])
                .mobile(),
        )
        .column(Column::datetime("approved_at", &t("budget.fields.approved_at")).hidden())
        .sort_by("-fiscal_year")
        .row_url("/budgets/{id}")
        .cards_on_mobile()
        .empty_state(&t("budget.empty"), Some(&t("budget.empty_hint")))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/budgets.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    budgets: GridPage<CompanyBudget>,
    can_manage: bool,
}

/// `GET /budgets`: the grid.
async fn index(lang: Lang, user: AuthUser, request: GridRequest) -> Result<View> {
    let scope = user.scopes_with::<LegalEntity>(permissions::BUDGETS_VIEW);
    let query = scope.apply(CompanyBudget::query(), &["legal_entity_id"]);
    let budgets = grid(&lang).page(query, &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        budgets,
        can_manage: user.has_permission(permissions::BUDGETS_MANAGE),
    }))
}

/// One row of the figures: a category and its amount in each month.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct FigureRow {
    category: String,
    cells: Vec<String>,
    total: String,
}

/// The figures of `lines`: one row per category that has a line, in the
/// order of `categories`, one cell per month of `months`.
pub(crate) fn figures(
    categories: &[BudgetCategory],
    lines: &[CompanyBudgetLine],
    months: &[NaiveDate],
) -> Vec<FigureRow> {
    categories
        .iter()
        .filter(|c| lines.iter().any(|l| l.category_id == c.id))
        .map(|category| {
            let mut total = 0;
            let cells = months
                .iter()
                .map(|month| {
                    match lines
                        .iter()
                        .find(|l| l.category_id == category.id && l.month == *month)
                    {
                        Some(line) => {
                            total += line.amount;
                            plain(line.amount)
                        }
                        None => "-".to_owned(),
                    }
                })
                .collect();
            FigureRow {
                category: category.name_en.clone(),
                cells,
                total: plain(total),
            }
        })
        .collect()
}

/// What the page of one version needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/budget.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    budget: CompanyBudget,
    title: String,
    entity: String,
    status_label: String,
    status_kind: &'static str,
    start_label: String,
    prepared_by: Option<String>,
    submitted_by: Option<String>,
    approved_by: Option<String>,
    head: Vec<String>,
    rows: Vec<FigureRow>,
    can_edit: bool,
    is_revision: bool,
    pending: bool,
    /// The active categories, `(id, name)`.
    categories: Vec<(String, String)>,
    /// The months of the fiscal year, `(first day, label)`.
    months: Vec<(String, String)>,
    /// The approval cards (`wl-kernel/approval_card.html`).
    approvals: Vec<ApprovalCard>,
    timeline: Vec<Value>,
}

/// The names of users `ids`.
async fn names(db: &Db, ids: Vec<i64>) -> Result<BTreeMap<i64, String>> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let users = User::query().where_in("id", ids).get(db).await?;
    Ok(users.into_iter().map(|user| (user.id, user.name)).collect())
}

/// `GET /budgets/{id}`: the figures, who did what, the approval card, the
/// sheets and the history.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(budget): Found<CompanyBudget>,
) -> Result<View> {
    let t = texts_in(&lang);
    let db = state.db.clone();
    if !user
        .scopes_with::<LegalEntity>(permissions::BUDGETS_VIEW)
        .contains(&budget.legal_entity_id)
    {
        return Err(Error::Forbidden);
    }
    let entity = LegalEntity::find(&db, budget.legal_entity_id)
        .await?
        .map(|e| e.code)
        .unwrap_or_default();
    let who = names(
        &db,
        [budget.prepared_by, budget.submitted_by, budget.approved_by]
            .into_iter()
            .flatten()
            .collect(),
    )
    .await?;
    let name_of = |id: Option<i64>| id.map(|id| who.get(&id).cloned().unwrap_or(format!("#{id}")));
    let categories = BudgetCategory::listed(&db, true).await?;
    let lines = CompanyBudgetLine::of(&db, budget.id).await?;
    let months = budget.months();
    let locked_months: Vec<NaiveDate> = lines.iter().filter(|l| l.locked).map(|l| l.month).collect();
    let head = std::iter::once(t("budget.lines.category"))
        .chain(months.iter().map(|m| {
            if locked_months.contains(m) {
                format!("{} ({})", month_label(*m), t("budget.lines.locked"))
            } else {
                month_label(*m)
            }
        }))
        .chain(std::iter::once(t("budget.lines.total")))
        .collect();
    let rows = figures(&categories, &lines, &months);
    let subject = budget.subject();
    let pending = !approvals::pending_for(&db, &subject).await?.is_empty();
    let approval_cards = approvals::cards_for(&db, &subject, &user, &lang).await?;
    let history = state_machine::history(&db, CompanyBudget::TABLE, budget.id).await?;
    let mut timeline =
        state_machine::timeline(&history, Some(&state_machine::info_of::<CompanyBudget>()));
    timeline.extend(approvals::timeline_for(&db, &subject, &lang).await?);
    timeline.sort_by(|a, b| a["time"].as_str().cmp(&b["time"].as_str()));
    let can_edit = user.has_permission(permissions::BUDGETS_MANAGE) && budget.is_editable();
    let year = budget.fiscal_year;
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: format!("{entity} {year} v{}", budget.version),
        entity,
        status_label: t(&format!("budget.status.{}", budget.status.as_str())),
        status_kind: status_kind(budget.status),
        start_label: t(start_key(budget.start_choice)),
        prepared_by: name_of(budget.prepared_by),
        submitted_by: name_of(budget.submitted_by),
        approved_by: name_of(budget.approved_by),
        head,
        rows,
        can_edit,
        is_revision: budget.is_revision(),
        pending,
        categories: categories
            .iter()
            .filter(|c| c.active)
            .map(|c| (c.id.to_string(), c.name_en.clone()))
            .collect(),
        months: months
            .iter()
            .map(|m| (m.to_string(), month_label(*m)))
            .collect(),
        approvals: approval_cards,
        timeline,
        budget,
    }))
}

/// What the new budget form needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/budget_form.html")]
pub(crate) struct FormPage {
    tx: Texts,
    title: String,
    action: String,
    cancel: String,
    entities: Vec<(String, String)>,
    starts: Vec<(String, String)>,
    values: BTreeMap<&'static str, String>,
}

/// `GET /budgets/new`: the form that makes a draft version.
async fn new_form(State(state): State<AppState>, lang: Lang) -> Result<View> {
    let t = texts_in(&lang);
    let entities = LegalEntity::query()
        .order_by(LegalEntity::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(LegalEntity::is_active)
        .map(|e| (e.id.to_string(), format!("{} - {}", e.code, e.name_en)))
        .collect();
    let year = crate::budget::fiscal_year_of(
        config_of(&state).budget_fiscal_start_month,
        today(),
    );
    let starts = [StartChoice::Empty, StartChoice::LastBudget, StartChoice::LastActual]
        .iter()
        .map(|c| {
            (c.as_str().to_owned(), t(start_key(*c)))
        })
        .collect();
    let mut values = BTreeMap::new();
    values.insert("fiscal_year", (year + 1).to_string());
    values.insert("start", StartChoice::Empty.as_str().to_owned());
    Ok(View::typed(FormPage {
        tx: texts::all(&lang),
        title: t("budget.new.title"),
        action: "/budgets".to_owned(),
        cancel: "/budgets".to_owned(),
        entities,
        starts,
        values,
    }))
}

/// A 422 with `message` under `field`, keeping the input.
fn invalid<F: Serialize>(field: &str, message: String, form: &F) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// A service refusal as the response: a 422 message goes under `field`.
fn on_field<F: Serialize>(error: Error, field: &str, form: &F) -> Error {
    match error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message) => invalid(field, message, form),
        other => other,
    }
}

/// The value of `key` in a form, trimmed.
fn value<'a>(form: &'a BTreeMap<String, String>, key: &str) -> &'a str {
    form.get(key).map_or("", |v| v.trim())
}

/// `POST /budgets`: makes the draft and opens its page.
async fn store(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let legal_entity_id = value(&form, "legal_entity_id")
        .parse()
        .map_err(|_| invalid("legal_entity_id", t("budget.errors.entity"), &form))?;
    let fiscal_year = value(&form, "fiscal_year")
        .parse()
        .map_err(|_| invalid("fiscal_year", t("budget.errors.year"), &form))?;
    let start = value(&form, "start")
        .parse::<StartChoice>()
        .ok()
        .filter(|c| *c != StartChoice::Revision)
        .unwrap_or(StartChoice::Empty);
    let new = NewBudget {
        legal_entity_id,
        fiscal_year,
        name: value(&form, "name").to_owned(),
        start,
        revision_reason: Some(value(&form, "revision_reason").to_owned()),
    };
    let budget = budget_lines::create(&state, new, Some(user.id))
        .await
        .map_err(|e| on_field(e, "base", &form))?;
    Ok((
        Toast::success(t("budget.new.done")),
        Redirect::route("wl_invoicing.budget", &[&budget.id])?,
    )
        .into_response())
}

/// `POST /budgets/{id}/lines`: sets an amount for a category over a range
/// of months and reloads the page.
async fn set_line(
    State(state): State<AppState>,
    lang: Lang,
    Found(budget): Found<CompanyBudget>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let category_id: i64 = value(&form, "category_id")
        .parse()
        .map_err(|_| invalid("category_id", t("budget.errors.category"), &form))?;
    let from = parse_month(value(&form, "from"))
        .ok_or_else(|| invalid("from", t("budget.errors.month"), &form))?;
    let to = match value(&form, "to") {
        "" => from,
        text => parse_month(text).ok_or_else(|| invalid("to", t("budget.errors.month"), &form))?,
    };
    if to < from {
        return Err(invalid("to", t("budget.errors.range"), &form));
    }
    let amount = parse_budget_amount(value(&form, "amount"))
        .ok_or_else(|| invalid("amount", t("budget.errors.amount"), &form))?;
    let note = Some(value(&form, "note").to_owned());
    let inputs: Vec<LineInput> = budget
        .months()
        .into_iter()
        .filter(|m| *m >= from && *m <= to)
        .map(|month| LineInput {
            category_id,
            month,
            amount,
            note: note.clone(),
        })
        .collect();
    if inputs.is_empty() {
        return Err(invalid("from", t("budget.errors.month"), &form));
    }
    budget_lines::set_lines(&state.db, budget.id, &inputs)
        .await
        .map_err(|e| on_field(e, "amount", &form))?;
    Ok((Toast::success(t("budget.set.done")), renox::HxRefresh).into_response())
}

/// `POST /budgets/{id}/lines/clear`: removes the line of a category in a
/// month and reloads the page.
async fn clear_line(
    State(state): State<AppState>,
    lang: Lang,
    Found(budget): Found<CompanyBudget>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let category_id: i64 = value(&form, "category_id")
        .parse()
        .map_err(|_| invalid("category_id", t("budget.errors.category"), &form))?;
    let month = parse_month(value(&form, "month"))
        .ok_or_else(|| invalid("month", t("budget.errors.month"), &form))?;
    budget_lines::delete_line(&state.db, budget.id, category_id, month)
        .await
        .map_err(|e| on_field(e, "month", &form))?;
    Ok((Toast::success(t("budget.clear.done")), renox::HxRefresh).into_response())
}

/// `POST /budgets/{id}/import`: checks the pasted CSV and, unless it is a
/// dry run, stores its lines. Rejected rows answer 422 under the text with
/// their row numbers.
async fn import(
    State(state): State<AppState>,
    lang: Lang,
    Found(budget): Found<CompanyBudget>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let dry_run = form.contains_key("dry_run");
    let text = form.get("csv").map_or("", String::as_str);
    if text.trim().is_empty() {
        return Err(invalid("csv", t("budget.import.empty"), &form));
    }
    let report = budget_lines::import_csv(&state.db, budget.id, text, dry_run)
        .await
        .map_err(|e| on_field(e, "csv", &form))?;
    if !report.errors.is_empty() {
        let message = report
            .errors
            .iter()
            .map(|e| format!("{} {}: {}", t("budget.import.row"), e.row, e.message))
            .collect::<Vec<_>>()
            .join(" ");
        return Err(invalid("csv", message, &form));
    }
    if dry_run {
        let message = wl_kernel::notify::fill(
            &t("budget.import.dry_run_ok"),
            &renox::serde_json::json!({ "rows": report.valid }),
        );
        return Ok((Toast::info(message), renox::HxRefresh).into_response());
    }
    let message = wl_kernel::notify::fill(
        &t("budget.import.done"),
        &renox::serde_json::json!({ "rows": report.valid }),
    );
    Ok((Toast::success(message), renox::HxRefresh).into_response())
}

/// `POST /budgets/{id}/revision-reason`: changes the reason of a revision.
async fn revision_reason(
    State(state): State<AppState>,
    lang: Lang,
    Found(budget): Found<CompanyBudget>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let t = texts_in(&lang);
    budget_lines::set_revision_reason(&state.db, budget.id, value(&form, "reason"))
        .await
        .map_err(|e| on_field(e, "reason", &form))?;
    Ok((Toast::success(t("budget.reason.done")), renox::HxRefresh).into_response())
}

/// `POST /budgets/{id}/submit`: submits the draft for approval and reloads
/// its page; a refusal comes back as a message.
async fn submit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(budget): Found<CompanyBudget>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let toast = match budget_machine::submit(&state, &user, budget.id).await {
        Ok(_) => Toast::success(t("budget.submit.done")),
        Err(TransitionError::Guard(message)) => Toast::error(message),
        Err(other) => return Err(other.into_error()),
    };
    Ok((toast, Redirect::route("wl_invoicing.budget", &[&budget.id])?).into_response())
}

/// `POST /budgets/{id}/discard`: gives the draft up with a reason.
async fn discard(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(budget): Found<CompanyBudget>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let t = texts_in(&lang);
    match budget_machine::discard(&state, By::User(&user), budget.id, value(&form, "reason")).await
    {
        Ok(_) => {}
        Err(TransitionError::ReasonRequired) => {
            return Err(invalid("reason", t("budget.discard.reason_required"), &form));
        }
        Err(TransitionError::Guard(message)) => return Err(invalid("reason", message, &form)),
        Err(other) => return Err(other.into_error()),
    }
    Ok((
        Toast::success(t("budget.discard.done")),
        Redirect::route("wl_invoicing.budgets", &[])?,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english_text(key: &str) -> String {
        texts::english(&format!("wl_invoicing.{key}")).to_owned()
    }

    fn category(id: i64, name: &str) -> BudgetCategory {
        BudgetCategory {
            id,
            name_en: name.to_owned(),
            ..Default::default()
        }
    }

    fn day(m: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, 1).unwrap()
    }

    #[test]
    fn the_grid_lists_the_budget_columns_and_statuses() {
        let grid = grid_with(&english_text);
        let keys: Vec<&str> = grid.columns().iter().map(Column::key).collect();
        assert_eq!(
            keys,
            ["entity", "fiscal_year", "version", "name", "status", "approved_at"]
        );
        let status = grid
            .columns()
            .iter()
            .find(|c| c.key() == "status")
            .expect("status column");
        assert_eq!(status.options().len(), BudgetStatus::ALL.len());
    }

    #[test]
    fn every_status_and_start_has_a_label() {
        for status in BudgetStatus::ALL {
            let key = format!("budget.status.{}", status.as_str());
            assert_ne!(english_text(&key), format!("wl_invoicing.{key}"));
        }
        for choice in StartChoice::ALL {
            let key = start_key(*choice);
            assert_ne!(english_text(key), format!("wl_invoicing.{key}"));
        }
    }

    #[test]
    fn the_figures_have_a_row_per_category_with_lines() {
        let categories = [category(1, "Rent"), category(2, "Marketing"), category(3, "Fuel")];
        let lines = [
            CompanyBudgetLine {
                category_id: 1,
                month: day(1),
                amount: 2_000_000,
                ..Default::default()
            },
            CompanyBudgetLine {
                category_id: 1,
                month: day(2),
                amount: 2_000_000,
                ..Default::default()
            },
            CompanyBudgetLine {
                category_id: 3,
                month: day(2),
                amount: 5_050,
                ..Default::default()
            },
        ];
        let months = [day(1), day(2), day(3)];
        let rows = figures(&categories, &lines, &months);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].category, "Rent");
        assert_eq!(rows[0].cells, ["20,000.00", "20,000.00", "-"]);
        assert_eq!(rows[0].total, "40,000.00");
        assert_eq!(rows[1].category, "Fuel");
        assert_eq!(rows[1].cells, ["-", "50.50", "-"]);
    }

    #[test]
    fn a_service_refusal_goes_under_its_field() {
        let form = BTreeMap::from([("amount".to_owned(), "x".to_owned())]);
        let error = on_field(
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, "nope".into()),
            "amount",
            &form,
        );
        assert!(matches!(error, Error::Validation(_)));
        let other = on_field(Error::NotFound, "amount", &form);
        assert!(matches!(other, Error::NotFound));
    }

    #[test]
    fn badges_use_only_the_kits_kinds() {
        for status in BudgetStatus::ALL {
            assert!(["info", "success", "warning", "error"].contains(&status_kind(*status)));
        }
    }
}
