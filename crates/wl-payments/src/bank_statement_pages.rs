//! The bank statement page (wl-payments §3, §6, issue #118):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /bank-statements` | `wl_payments.bank_statements.index` | `bank.reconcile` |
//! | `POST /bank-statements` | `wl_payments.bank_statements.store` | `bank.reconcile` |
//!
//! The accountant records a statement upload with its period and, when the
//! file carries none, the closing balance. Reading the lines and reconciling
//! them come with the bank reconciliation feature. The template is compiled
//! in and an app can replace it.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;

use crate::bank_statement::{self, BankStatementImport, FORMATS, ImportStatus, Upload};
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
            status: t(&format!("bank_statements.status.{}", status_code(import.status))),
        })
        .collect();
    let formats = FORMATS
        .iter()
        .map(|code| ((*code).to_owned(), t(&format!("bank_statements.format.{code}"))))
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
    };
    if bank_statement::upload_problem(&upload).is_some() {
        return Err(invalid("period_end", t("bank_statements.bad_upload"), &form));
    }
    bank_statement::record(&state.db, Some(user.id), &upload).await?;
    Ok((
        Toast::success(t("bank_statements.recorded")),
        Redirect::route("wl_payments.bank_statements.index", &[])?,
    )
        .into_response())
}
