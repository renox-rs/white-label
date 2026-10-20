//! The bank statement page (wl-payments §3, §6, issue #118):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /bank-statements` | `wl_payments.bank_statements.index` | `bank.reconcile` |
//! | `POST /bank-statements` | `wl_payments.bank_statements.store` | `bank.reconcile` |
//! | `GET /bank-statements/{id}` | `wl_payments.bank_statements.show` | `bank.reconcile` |
//! | `POST /bank-statements/{id}/parse` | `wl_payments.bank_statements.parse` | `bank.reconcile` |
//! | `POST /bank-statements/{id}/complete` | `wl_payments.bank_statements.complete` | `bank.reconcile` |
//! | `POST /bank-statements/{id}/revert` | `wl_payments.bank_statements.revert` | `bank.reconcile` |
//! | `POST /bank-statement-lines/{id}/ignore` | `wl_payments.bank_statements.ignore` | `bank.reconcile` (field `reason`) |
//! | `POST /bank-statement-lines/{id}/drawing` | `wl_payments.bank_statements.drawing` | `bank.reconcile` (field `category`) |
//!
//! The accountant records a statement upload with its period, the text of
//! the file (CSV or MT940) and, when the file carries none, the closing
//! balance. The statement page reads the file into lines, shows them, ignores
//! a line with a reason or marks it as an owner drawing, and completes or
//! reverts the import ([`crate::bank_statement_flow`]). The template is
//! compiled in and an app can replace it.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;

use crate::bank_statement::{self, BankStatementImport, FORMATS, ImportStatus, Upload};
use crate::bank_statement_flow;
use crate::bank_statement_line::{BankStatementLine, LineStatus};
use crate::followup::money_text;
use crate::pages::{filled, invalid, texts_in};
use crate::permissions;
use crate::texts;

/// The routes of the bank statement page.
pub fn routes() -> Routes {
    Routes::new()
        .get("/bank-statements", index)
        .name("wl_payments.bank_statements.index")
        .post("/bank-statements", store)
        .name("wl_payments.bank_statements.store")
        .get("/bank-statements/{id}", show)
        .name("wl_payments.bank_statements.show")
        .post("/bank-statements/{id}/parse", store_parse)
        .name("wl_payments.bank_statements.parse")
        .post("/bank-statements/{id}/complete", store_complete)
        .name("wl_payments.bank_statements.complete")
        .post("/bank-statements/{id}/revert", store_revert)
        .name("wl_payments.bank_statements.revert")
        .post("/bank-statement-lines/{id}/ignore", store_ignore)
        .name("wl_payments.bank_statements.ignore")
        .post("/bank-statement-lines/{id}/drawing", store_drawing)
        .name("wl_payments.bank_statements.drawing")
        .require_permission(permissions::BANK_RECONCILE)
}

/// One import of the table.
#[derive(Serialize)]
struct Row {
    id: i64,
    entity: String,
    bank_account: String,
    file_name: String,
    format: String,
    period: String,
    closing_balance: String,
    status: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/bank_statements.html")]
pub(crate) struct IndexPage {
    tx: BTreeMap<&'static str, String>,
    rows: Vec<Row>,
    entities: Vec<(i64, String)>,
    formats: Vec<(String, String)>,
}

/// The active legal entities as select options.
async fn entities(state: &AppState) -> Result<Vec<(i64, String)>> {
    Ok(LegalEntity::query()
        .where_(LegalEntity::STATUS.eq("active"))
        .order_by(LegalEntity::CODE)
        .get(&state.db)
        .await?
        .into_iter()
        .map(|e| (e.id, format!("{} - {}", e.code, e.name_en)))
        .collect())
}

/// `GET /bank-statements`: the imports, newest period first, and the upload
/// form.
async fn index(State(state): State<AppState>, lang: Lang) -> Result<View> {
    let t = texts_in(&lang);
    let entities = entities(&state).await?;
    let imports = BankStatementImport::query()
        .order_by_desc(BankStatementImport::PERIOD_END)
        .order_by_desc(BankStatementImport::ID)
        .get(&state.db)
        .await?;
    let rows = imports
        .iter()
        .map(|import| Row {
            id: import.id,
            entity: entities
                .iter()
                .find(|(id, _)| *id == import.legal_entity_id)
                .map(|(_, label)| label.clone())
                .unwrap_or_default(),
            bank_account: import.bank_account.clone(),
            file_name: import.file_name.clone(),
            format: t(&format!("bank_statements.format.{}", import.format)),
            period: format!(
                "{} - {}",
                import.period_start.format("%Y-%m-%d"),
                import.period_end.format("%Y-%m-%d")
            ),
            closing_balance: import
                .closing_balance
                .map(|fils| money_text(&import.currency, fils))
                .unwrap_or_else(|| t("bank_statements.no_balance")),
            status: t(&format!(
                "bank_statements.status.{}",
                status_code(import.status)
            )),
        })
        .collect();
    let formats = FORMATS
        .iter()
        .map(|code| {
            (
                (*code).to_owned(),
                t(&format!("bank_statements.format.{code}")),
            )
        })
        .collect();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
        entities,
        formats,
    }))
}

/// The code of a status, for its text.
fn status_code(status: ImportStatus) -> &'static str {
    match status {
        ImportStatus::Uploaded => "uploaded",
        ImportStatus::InReview => "in_review",
        ImportStatus::Completed => "completed",
        ImportStatus::Failed => "failed",
        ImportStatus::Reverted => "reverted",
    }
}

/// The upload form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct UploadForm {
    /// The legal entity's id.
    #[validate(required)]
    pub legal_entity_id: Option<i64>,
    /// The bank account label.
    #[validate(required, max = 100)]
    pub bank_account: Option<String>,
    /// The file name.
    #[validate(required, max = 200)]
    pub file_name: Option<String>,
    /// `csv`, `excel` or `mt940`.
    #[validate(required, max = 16)]
    pub format: Option<String>,
    /// The first day of the period.
    #[validate(required)]
    pub period_start: Option<NaiveDate>,
    /// The last day of the period.
    #[validate(required)]
    pub period_end: Option<NaiveDate>,
    /// The closing balance in dirhams, when the file carries none.
    #[validate(max = 32)]
    pub closing_balance: Option<String>,
    /// The text of the file (CSV or MT940).
    #[validate(max = 2000000)]
    pub content: Option<String>,
}

/// `POST /bank-statements`: records the upload.
async fn store(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<UploadForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let closing_balance = match filled(&form.closing_balance) {
        None => None,
        Some(text) => match bank_statement::parse_balance(text) {
            Some(fils) => Some(fils),
            None => return Err(invalid("closing_balance", t("bad_amount"), &form)),
        },
    };
    let (Some(legal_entity_id), Some(period_start), Some(period_end)) =
        (form.legal_entity_id, form.period_start, form.period_end)
    else {
        return Err(invalid("legal_entity_id", t("unknown_entity"), &form));
    };
    let upload = Upload {
        legal_entity_id,
        bank_account: filled(&form.bank_account).unwrap_or_default().to_owned(),
        file_name: filled(&form.file_name).unwrap_or_default().to_owned(),
        format: filled(&form.format).unwrap_or_default().to_owned(),
        period_start,
        period_end,
        closing_balance,
        content: form.content.clone(),
    };
    if bank_statement::upload_problem(&upload).is_some() {
        return Err(invalid(
            "period_end",
            t("bank_statements.bad_upload"),
            &form,
        ));
    }
    bank_statement::record(&state.db, Some(user.id), &upload).await?;
    Ok((
        Toast::success(t("bank_statements.recorded")),
        Redirect::route("wl_payments.bank_statements.index", &[])?,
    )
        .into_response())
}

/// One line of the statement page.
#[derive(Serialize)]
struct LineRow {
    id: i64,
    date: String,
    amount: String,
    description: String,
    reference: String,
    status: String,
    status_kind: &'static str,
    note: String,
    open: bool,
}

/// What the statement page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/bank_statement.html")]
pub(crate) struct ShowPage {
    tx: BTreeMap<&'static str, String>,
    import: Row,
    title: String,
    status_kind: &'static str,
    lines: Vec<LineRow>,
    can_parse: bool,
    can_complete: bool,
    can_revert: bool,
}

/// The badge colour of an import status.
fn status_kind(status: ImportStatus) -> &'static str {
    match status {
        ImportStatus::Uploaded | ImportStatus::Reverted => "neutral",
        ImportStatus::InReview => "info",
        ImportStatus::Completed => "success",
        ImportStatus::Failed => "error",
    }
}

fn line_status_code(status: LineStatus) -> &'static str {
    match status {
        LineStatus::Unmatched => "unmatched",
        LineStatus::Suggested => "suggested",
        LineStatus::Matched => "matched",
        LineStatus::Ignored => "ignored",
    }
}

/// `GET /bank-statements/{id}`: the import, its lines and its actions.
async fn show(
    State(state): State<AppState>,
    lang: Lang,
    Found(import): Found<BankStatementImport>,
) -> Result<View> {
    let t = texts_in(&lang);
    let entities = entities(&state).await?;
    let lines = BankStatementLine::of_import(&state.db, import.id)
        .await?
        .into_iter()
        .map(|line| LineRow {
            id: line.id,
            date: line.line_date.format("%Y-%m-%d").to_string(),
            amount: money_text(&line.currency, line.amount),
            description: line.description.clone(),
            reference: line.reference.clone(),
            status: t(&format!(
                "bank_statements.line.status.{}",
                line_status_code(line.status)
            )),
            status_kind: match line.status {
                LineStatus::Unmatched => "warning",
                LineStatus::Suggested => "info",
                LineStatus::Matched => "success",
                LineStatus::Ignored => "info",
            },
            note: line
                .reason
                .clone()
                .or_else(|| line.ignore_reason.clone())
                .unwrap_or_default(),
            open: import.status == ImportStatus::InReview && line.status == LineStatus::Unmatched,
        })
        .collect();
    let row = Row {
        id: import.id,
        entity: entities
            .iter()
            .find(|(id, _)| *id == import.legal_entity_id)
            .map(|(_, label)| label.clone())
            .unwrap_or_default(),
        bank_account: import.bank_account.clone(),
        file_name: import.file_name.clone(),
        format: t(&format!("bank_statements.format.{}", import.format)),
        period: format!(
            "{} - {}",
            import.period_start.format("%Y-%m-%d"),
            import.period_end.format("%Y-%m-%d")
        ),
        closing_balance: import
            .closing_balance
            .map(|fils| money_text(&import.currency, fils))
            .unwrap_or_else(|| t("bank_statements.no_balance")),
        status: t(&format!(
            "bank_statements.status.{}",
            status_code(import.status)
        )),
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: format!("{} - {}", import.bank_account, import.file_name),
        status_kind: status_kind(import.status),
        can_parse: import.status == ImportStatus::Uploaded,
        can_complete: import.status == ImportStatus::InReview,
        can_revert: import.status == ImportStatus::InReview,
        import: row,
        lines,
    }))
}

/// The statement page, refreshed, with `done` as the toast.
fn refreshed(done: String) -> Result<Response> {
    Ok((Toast::success(done), renox::HxRefresh).into_response())
}

/// `POST /bank-statements/{id}/parse`: reads the file as the system.
async fn store_parse(
    State(state): State<AppState>,
    lang: Lang,
    Found(import): Found<BankStatementImport>,
) -> Result<Response> {
    bank_statement_flow::parse(&state, import.id)
        .await
        .map_err(TransitionError::into_error)?;
    refreshed(texts_in(&lang)("bank_statements.parse.done"))
}

/// `POST /bank-statements/{id}/complete`.
async fn store_complete(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(import): Found<BankStatementImport>,
) -> Result<Response> {
    bank_statement_flow::complete(&state, &user, &import)
        .await
        .map_err(TransitionError::into_error)?;
    refreshed(texts_in(&lang)("bank_statements.complete.done"))
}

/// `POST /bank-statements/{id}/revert`.
async fn store_revert(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(import): Found<BankStatementImport>,
) -> Result<Response> {
    bank_statement_flow::revert(&state, &user, &import)
        .await
        .map_err(TransitionError::into_error)?;
    refreshed(texts_in(&lang)("bank_statements.revert.done"))
}

/// The ignore sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct IgnoreForm {
    /// Why the line is left out.
    #[validate(max = 500)]
    pub reason: Option<String>,
}

/// `POST /bank-statement-lines/{id}/ignore`.
async fn store_ignore(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<IgnoreForm>,
) -> Result<Response> {
    let reason = form.reason.clone().unwrap_or_default();
    match bank_statement_flow::ignore(&state, &user, id, &reason).await {
        Ok(_) => refreshed(texts_in(&lang)("bank_statements.line.ignore.done")),
        Err(TransitionError::Guard(message)) => Err(invalid("reason", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// The owner drawing sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct DrawingForm {
    /// The code of the drawings category.
    #[validate(max = 64)]
    pub category: Option<String>,
}

/// `POST /bank-statement-lines/{id}/drawing`.
async fn store_drawing(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<DrawingForm>,
) -> Result<Response> {
    let category = filled(&form.category).unwrap_or(crate::expense_category::OWNER_DRAWINGS);
    match bank_statement_flow::mark_drawing(&state, &user, id, category).await {
        Ok(_) => refreshed(texts_in(&lang)("bank_statements.line.drawing.done")),
        Err(TransitionError::Guard(message)) => Err(invalid("category", message, &form)),
        Err(TransitionError::Failed(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message))) => {
            Err(invalid("category", message, &form))
        }
        Err(other) => Err(other.into_error()),
    }
}
