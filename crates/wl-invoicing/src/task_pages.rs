//! The reversal task list (wl-invoicing §6, #70 A12):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /invoices/tasks` | `wl_invoicing.tasks` | `receivables.view` |
//! | `GET /invoices/tasks/{id}` | `wl_invoicing.task` | `receivables.view` |
//! | `POST /invoices/tasks/{id}/done` | `wl_invoicing.task_done` | `receivables.view` |
//!
//! The list is a `renox::grid` of the open [`InvoicingTask`]s, oldest first.
//! A row opens its invoice or supplier bill (through `GET /invoices/tasks/{id}`, which only
//! redirects, since a grid link knows nothing but the row's id), and "Mark
//! done" closes the task. No route unallocates anything. The template is
//! compiled in and can be replaced by the app
//! (`resources/views/wl-invoicing/tasks.html`).

use std::collections::BTreeMap;

use renox::grid::{Action, Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use serde::Serialize;

use crate::permissions;
use crate::task::{InvoicingTask, TaskKind, TaskStatus};
use crate::texts;

/// The texts a template reads as `tx['tasks.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The package text `key` (without the `wl_invoicing.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_invoicing.{key}"))
}

/// The module's task routes, each behind `receivables.view`.
pub fn routes() -> Routes {
    Routes::new()
        .get("/invoices/tasks", index)
        .name("wl_invoicing.tasks")
        .get("/invoices/tasks/{id}", open)
        .name("wl_invoicing.task")
        .post("/invoices/tasks/{id}/done", done)
        .name("wl_invoicing.task_done")
        .require_permission(permissions::RECEIVABLES_VIEW)
}

/// The open tasks grid.
pub fn grid(lang: &Lang) -> Grid {
    let kinds: Vec<(String, String)> = [
        TaskKind::ReversalAfterWriteOff,
        TaskKind::ReversalRefused,
        TaskKind::BillReversalRefused,
    ]
    .iter()
    .map(|kind| {
        let key = kind.as_str();
        (key.to_owned(), text(lang, &format!("tasks.kind.{key}")))
    })
    .collect();
    Grid::new("wl-invoicing-tasks")
        .title(&text(lang, "tasks.title"))
        .column(
            Column::related(
                "invoice",
                &text(lang, "tasks.invoice"),
                "wl_invoicing_invoices",
                "invoice_id",
                "number",
            )
            .frozen()
            .mobile(),
        )
        .column(
            Column::related(
                "bill",
                &text(lang, "tasks.bill"),
                "wl_invoicing_supplier_bills",
                "bill_id",
                "supplier_invoice_number",
            )
            .mobile(),
        )
        .column(
            Column::select("kind", &text(lang, "tasks.kind"), kinds)
                .badges(&[
                    ("reversal_after_write_off", "warning"),
                    ("reversal_refused", "danger"),
                    ("bill_reversal_refused", "danger"),
                ])
                .mobile(),
        )
        .column(
            Column::text("message", &text(lang, "tasks.message"))
                .searchable()
                .wrap()
                .mobile(),
        )
        .column(Column::datetime("created_at", &text(lang, "tasks.created")))
        .sort_by("created_at")
        .row_url("/invoices/tasks/{id}")
        .row_action(
            Action::new(&text(lang, "tasks.mark_done"), "/invoices/tasks/{id}/done")
                .confirm(&text(lang, "tasks.confirm")),
        )
        .cards_on_mobile()
        .empty_state(
            &text(lang, "tasks.empty"),
            Some(&text(lang, "tasks.empty_hint")),
        )
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/tasks.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    tasks: GridPage<InvoicingTask>,
}

/// `GET /invoices/tasks`: the open tasks.
async fn index(lang: Lang, request: GridRequest) -> Result<View> {
    let open = InvoicingTask::query().where_(InvoicingTask::STATUS.eq(TaskStatus::Open));
    let tasks = grid(&lang).page(open, &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        tasks,
    }))
}

/// `GET /invoices/tasks/{id}`: to the task's invoice or supplier bill.
async fn open(Found(task): Found<InvoicingTask>) -> Result<Response> {
    if let Some(bill_id) = task.bill_id {
        return Ok(Redirect::route("wl_invoicing.supplier_bill", &[&bill_id])?.into_response());
    }
    let invoice_id = task.invoice_id.ok_or(Error::NotFound)?;
    Ok(Redirect::route("wl_invoicing.show", &[&invoice_id])?.into_response())
}

/// `POST /invoices/tasks/{id}/done`: closes the task for the user and
/// reloads the list. A task that is already done stays as it was.
async fn done(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    Found(mut task): Found<InvoicingTask>,
) -> Result<Response> {
    if task.status == TaskStatus::Done {
        return Ok((
            Toast::success(text(&lang, "tasks.already_done")),
            renox::HxRefresh,
        )
            .into_response());
    }
    task.status = TaskStatus::Done;
    task.done_by = Some(user.id);
    task.done_at = Some(renox::db::now());
    task.save_only(&db, &["status", "done_by", "done_at"])
        .await?;
    Ok((Toast::success(text(&lang, "tasks.done")), renox::HxRefresh).into_response())
}
