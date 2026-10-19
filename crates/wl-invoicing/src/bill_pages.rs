//! The supplier bill pages (wl-invoicing §6, A9):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /supplier-bills` | `wl_invoicing.supplier_bills` | `supplier_bills.view` |
//! | `GET /supplier-bills/{id}` | `wl_invoicing.supplier_bill` | `supplier_bills.view` |
//! | `POST /supplier-bills/{id}/transitions/{name}` | `wl_invoicing.supplier_bill_transitions.*` | `supplier_bills.view` and the transition's permission |
//! | `GET /supplier-bills/new` | `wl_invoicing.supplier_bill_new` | `supplier_bills.create` |
//! | `POST /supplier-bills` | `wl_invoicing.supplier_bill_store` | `supplier_bills.create` |
//! | `GET /supplier-bills/{id}/edit` | `wl_invoicing.supplier_bill_edit` | `supplier_bills.create` |
//! | `POST /supplier-bills/{id}` | `wl_invoicing.supplier_bill_update` | `supplier_bills.create` |
//! | `POST /supplier-bills/{id}/delete` | `wl_invoicing.supplier_bill_delete` | `supplier_bills.create` |
//! | `POST /supplier-bills/{id}/submit` | `wl_invoicing.supplier_bill_submit` | `supplier_bills.create` |
//!
//! The list is a `renox::grid`. The form holds the header and a fixed set of
//! line rows (empty rows are skipped; the supplier is the party code, like
//! `P-00001`); only a `draft` can be changed or deleted. The page of one
//! bill shows the header, the lines and totals, every exception reason (a
//! possible duplicate links to the earlier bill), the approver and reason or
//! the void reason, the applied payments (removed ones struck through, with
//! the reason), the "Reopened" log, its open tasks and the history. `submit` has its own route (the
//! service picks `submit` or `submit_held`); `approve` and `void` are posted
//! with their reason from a sheet each to the kernel's transition route.
//! `approve` is not offered to the creator of a held duplicate. Templates are
//! compiled in and can be replaced by the app.

use std::collections::BTreeMap;

use renox::auth::User;
use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{self, TransitionError};
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::approvals::{self, ApprovalCard};
use wl_kernel::invoicing::VatCategory;
use wl_kernel::{LegalEntity, PartyId};

use crate::allocate::amount_text;
use crate::bill_allocation::BillAllocation;
use crate::bill_line::{BillLine, BillLineDraft};
use crate::bill_submit::submit;
use crate::bill_unallocate::reason_key;
use crate::bill_unallocation::BillUnallocation;
use crate::budget_actual::Mapped;
use crate::credit_notes::By;
use crate::drawings::{self, amount_of};
use crate::drawings_machine::{DrawingApproval, DrawingStatus};
use crate::duplicates::exceptions_of;
use crate::pages::{AllocationRow, ReopenedRow, TaskRow, reopen_reason_text};
use crate::permissions;
use crate::supplier_bill::{self, BillDraft, SupplierBill, SupplierBillStatus};
use crate::task;
use crate::texts;
use crate::unallocation::UnallocationOutcome;

/// The texts a template reads as `tx['supplier_bill.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The package text `key` (without the `wl_invoicing.` prefix) in a language.
type Text<'a> = &'a dyn Fn(&str) -> String;

/// The line rows the form offers at least.
const MIN_SLOTS: usize = 5;

/// The most line rows the form reads.
const MAX_SLOTS: usize = 60;

/// The package texts in `lang`.
fn texts_in(lang: &Lang) -> impl Fn(&str) -> String + '_ {
    move |key: &str| texts::t(lang, &format!("wl_invoicing.{key}"))
}

/// The module's supplier bill routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/supplier-bills", index)
                .name("wl_invoicing.supplier_bills")
                .get("/supplier-bills/{id}", show)
                .name("wl_invoicing.supplier_bill")
                .group(
                    "/supplier-bills",
                    "wl_invoicing.supplier_bill_transitions.",
                    wl_kernel::machines::routes::<SupplierBill>(),
                )
                .require_permission(permissions::SUPPLIER_BILLS_VIEW),
        )
        .merge(
            Routes::new()
                .get("/supplier-bills/new", new_form)
                .name("wl_invoicing.supplier_bill_new")
                .post("/supplier-bills", store)
                .name("wl_invoicing.supplier_bill_store")
                .get("/supplier-bills/{id}/edit", edit_form)
                .name("wl_invoicing.supplier_bill_edit")
                .post("/supplier-bills/{id}", update)
                .name("wl_invoicing.supplier_bill_update")
                .post("/supplier-bills/{id}/delete", destroy)
                .name("wl_invoicing.supplier_bill_delete")
                .post("/supplier-bills/{id}/submit", store_submit)
                .name("wl_invoicing.supplier_bill_submit")
                .require_permission(permissions::SUPPLIER_BILLS_CREATE),
        )
}

/// The badge colour of a status, as the machine's tones give it.
fn status_kind(status: SupplierBillStatus) -> &'static str {
    match status {
        SupplierBillStatus::Draft | SupplierBillStatus::Void => "neutral",
        SupplierBillStatus::Matched | SupplierBillStatus::PartiallyPaid => "info",
        SupplierBillStatus::Exception => "warning",
        SupplierBillStatus::Approved | SupplierBillStatus::Paid => "success",
    }
}

/// The supplier bills grid: search on the supplier's invoice number, filters
/// by status and supplier, money in each bill's currency.
pub fn grid(lang: &Lang) -> Grid {
    grid_with(&texts_in(lang))
}

/// [`grid`] with the texts of `t`.
fn grid_with(t: Text<'_>) -> Grid {
    let statuses: Vec<(String, String)> = SupplierBillStatus::ALL
        .iter()
        .map(|s| {
            (
                s.as_str().to_owned(),
                t(&format!("supplier_bill.status.{}", s.as_str())),
            )
        })
        .collect();
    Grid::new("wl-invoicing-supplier-bills")
        .title(&t("supplier_bill.title"))
        .column(
            Column::text("supplier_invoice_number", &t("supplier_bill.fields.number"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/supplier-bills/{id}"),
        )
        .column(Column::number("party_id", &t("supplier_bill.fields.supplier")).mobile())
        .column(Column::related(
            "entity",
            &t("supplier_bill.fields.entity"),
            "wl_legal_entities",
            "legal_entity_id",
            "code",
        ))
        .column(Column::date(
            "bill_date",
            &t("supplier_bill.fields.bill_date"),
        ))
        .column(Column::date(
            "due_date",
            &t("supplier_bill.fields.due_date"),
        ))
        .column(
            Column::money("gross_total", &t("fields.gross_total"))
                .currency_from("currency")
                .mobile(),
        )
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("draft", "neutral"),
                    ("matched", "info"),
                    ("exception", "warning"),
                    ("approved", "success"),
                    ("partially_paid", "info"),
                    ("paid", "success"),
                    ("void", "neutral"),
                ])
                .mobile(),
        )
        .column(Column::datetime("created_at", &t("fields.created_at")).hidden())
        .sort_by("-created_at")
        .row_url("/supplier-bills/{id}")
        .cards_on_mobile()
        .empty_state(&t("supplier_bill.empty"), None)
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/bills_index.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    bills: GridPage<SupplierBill>,
    can_create: bool,
}

/// `GET /supplier-bills`: the grid.
async fn index(user: AuthUser, lang: Lang, request: GridRequest) -> Result<View> {
    let bills = grid(&lang).page(SupplierBill::query(), &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        bills,
        can_create: user.has_permission(permissions::SUPPLIER_BILLS_CREATE),
    }))
}

/// One line of a bill.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct LineRow {
    id: i64,
    description: String,
    quantity: String,
    unit_price: String,
    category: String,
    net: String,
    vat: String,
    gross: String,
    service_date: Option<NaiveDate>,
    cost_code: Option<String>,
}

/// One VAT category's totals.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct VatRow {
    category: String,
    net: String,
    vat: String,
}

/// One reason a bill is held.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ExceptionRow {
    id: i64,
    reason: String,
    /// The earlier bill's supplier invoice number and page.
    related: Option<String>,
    related_link: Option<String>,
    /// What the reason is about, e.g. the lines of work after the contract
    /// end.
    detail: Option<String>,
}

/// One reason sheet (`approve` or `void`).
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ReasonSheet {
    id: String,
    action: String,
    label: String,
    title: String,
    description: String,
    submit: String,
    variant: &'static str,
    icon: &'static str,
    reason_required: bool,
}

/// What the bill page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/bill_show.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    bill: SupplierBill,
    title: String,
    supplier: String,
    entity: String,
    status_label: String,
    status_kind: &'static str,
    /// Net, VAT, gross and gross in AED, formatted.
    amounts: BTreeMap<&'static str, String>,
    contract: Option<String>,
    period: Option<String>,
    created_by: Option<String>,
    approved_by: Option<String>,
    lines: Vec<LineRow>,
    vat: Vec<VatRow>,
    exceptions: Vec<ExceptionRow>,
    /// The payments and debit notes applied, removed ones included.
    allocations: Vec<AllocationRow>,
    /// The `unallocate` runs, done or refused.
    reopened: Vec<ReopenedRow>,
    /// The bill's open reversal tasks.
    tasks: Vec<TaskRow>,
    /// Whether the user may open the tasks list.
    can_tasks: bool,
    /// The bill's history.
    timeline: Vec<Value>,
    can_edit: bool,
    can_submit: bool,
    /// The approve and void sheets this user may use.
    sheets: Vec<ReasonSheet>,
    /// The owner drawings of the bill (`FIN.ACL.21`).
    drawings: DrawingsView,
    /// The approval cards of the drawing (`wl-kernel/approval_card.html`).
    approvals: Vec<ApprovalCard>,
}

/// One recoding of a line, as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct RecodingRow {
    line: String,
    from: String,
    to: String,
    by: String,
    at: Option<DateTime>,
}

/// What the page shows of the owner drawings on a bill: the total, never the
/// lines unless the user holds `drawings.approve` (`FIN.ACL.22`).
#[derive(Serialize, Debug, Clone, Default)]
pub(crate) struct DrawingsView {
    present: bool,
    total: String,
    status: String,
    status_kind: &'static str,
    reason: Option<String>,
    can_request: bool,
    can_recode: bool,
    /// The lines of the bill, `(id, label)`, for the recode sheet.
    lines: Vec<(String, String)>,
    recodings: Vec<RecodingRow>,
}

/// The name of user `id`, if there is one.
async fn user_name(db: &Db, id: Option<i64>) -> Option<String> {
    User::find(db, id?).await.ok().flatten().map(|u| u.name)
}

/// The supplier's name, else its id.
async fn supplier_name(state: &AppState, party_id: i64) -> String {
    wl_kernel::parties::directory(state)
        .billing_identity(&state.db, PartyId(party_id))
        .await
        .ok()
        .flatten()
        .map_or_else(|| format!("#{party_id}"), |identity| identity.legal_name_en)
}

/// The `approve` and `void` sheets `user` may use on `bill`. `approve` is
/// not offered to the creator of a held duplicate (the machine refuses it).
fn sheets_for(t: Text<'_>, user: &AuthUser, bill: &SupplierBill) -> Vec<ReasonSheet> {
    if !user.has_permission(permissions::SUPPLIER_BILLS_APPROVE) {
        return Vec::new();
    }
    let held = bill.status == SupplierBillStatus::Exception;
    let own_duplicate = held
        && (bill.duplicate_of.is_some() || bill.after_end_hold)
        && bill.created_by == Some(user.id);
    let mut sheets = Vec::new();
    if matches!(
        bill.status,
        SupplierBillStatus::Matched | SupplierBillStatus::Exception
    ) && !own_duplicate
    {
        sheets.push(ReasonSheet {
            id: format!("approve-supplier-bill-{}", bill.id),
            action: format!("/supplier-bills/{}/transitions/approve", bill.id),
            label: t("supplier_bill.approve.action"),
            title: t("supplier_bill.approve.title"),
            description: t("supplier_bill.approve.description"),
            submit: t("supplier_bill.approve.submit"),
            variant: "primary",
            icon: "check",
            reason_required: held,
        });
    }
    if matches!(
        bill.status,
        SupplierBillStatus::Draft | SupplierBillStatus::Matched | SupplierBillStatus::Exception
    ) {
        sheets.push(ReasonSheet {
            id: format!("void-supplier-bill-{}", bill.id),
            action: format!("/supplier-bills/{}/transitions/void", bill.id),
            label: t("supplier_bill.void.action"),
            title: t("supplier_bill.void.title"),
            description: t("supplier_bill.void.description"),
            submit: t("supplier_bill.void.submit"),
            variant: "danger",
            icon: "ban",
            reason_required: true,
        });
    }
    sheets
}

/// `GET /supplier-bills/{id}`: the header, lines, totals, exception reasons,
/// approval or void reason, history and the actions.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(bill): Found<SupplierBill>,
) -> Result<View> {
    let t = texts_in(&lang);
    let db = state.db.clone();
    let money = |fils: i64| amount_text(&bill.currency, fils);
    let mapped = Mapped::load(&db).await?;
    let can_see_drawings = match bill.legal_entity_id {
        Some(entity) => user
            .scopes_with::<LegalEntity>(permissions::DRAWINGS_APPROVE)
            .contains(&entity),
        None => user.has_permission(permissions::DRAWINGS_APPROVE),
    };
    let bill_lines = supplier_bill::lines_of(&db, bill.id).await?;
    let lines = bill_lines
        .iter()
        .map(|line: &BillLine| {
            // The lines of owner drawings are shown to the holders of
            // `drawings.approve` only (`FIN.ACL.22`); the totals are not.
            if !can_see_drawings && mapped.is_drawings_code(line.cost_code.as_deref()) {
                let dash = || "\u{2014}".to_owned();
                return LineRow {
                    id: line.id,
                    description: t("drawings.hidden"),
                    quantity: dash(),
                    unit_price: dash(),
                    category: dash(),
                    net: dash(),
                    vat: dash(),
                    gross: dash(),
                    service_date: None,
                    cost_code: None,
                };
            }
            LineRow {
                id: line.id,
                description: line.description.clone(),
                quantity: decimal_text(line.quantity_milli, 3, 0),
                unit_price: money(line.unit_price),
                category: t(&format!("vat_category.{}", line.vat_category.as_str())),
                net: money(line.net_amount),
                vat: money(line.vat_amount),
                gross: money(line.gross_amount),
                service_date: line.service_date,
                cost_code: line.cost_code.clone(),
            }
        })
        .collect();
    let vat = supplier_bill::totals_of(&db, bill.id)
        .await?
        .categories
        .iter()
        .map(|c| VatRow {
            category: t(&format!("vat_category.{}", c.category.as_str())),
            net: money(c.net),
            vat: money(c.vat),
        })
        .collect();
    let mut exceptions = Vec::new();
    for row in exceptions_of(&db, bill.id).await? {
        let related = match row.related_bill_id {
            Some(id) => SupplierBill::find(&db, id).await?,
            None => None,
        };
        let key = format!("supplier_bill.exception.{}", row.reason_code);
        let label = t(&key);
        exceptions.push(ExceptionRow {
            id: row.id,
            reason: if label == format!("wl_invoicing.{key}") {
                row.reason_code.clone()
            } else {
                label
            },
            related: related.map(|r| r.supplier_invoice_number),
            related_link: row
                .related_bill_id
                .map(|id| format!("/supplier-bills/{id}")),
            detail: row.detail.clone(),
        });
    }
    let allocations = BillAllocation::of_bill(&db, bill.id)
        .await?
        .into_iter()
        .map(|a| AllocationRow {
            id: a.id,
            source: a.source().to_string(),
            amount: money(a.amount),
            allocated_at: a.allocated_at,
            removed: a.removed_at.is_some(),
            removed_at: a.removed_at,
            removed_reason: a
                .removed_at
                .map(|_| reopen_reason_text(&t, a.removed_reason.as_deref().unwrap_or(""))),
        })
        .collect();
    let reopened = BillUnallocation::query()
        .where_(BillUnallocation::BILL_ID.eq(bill.id))
        .order_by(BillUnallocation::ID)
        .get(&db)
        .await?
        .into_iter()
        .map(|u| {
            let mut reason = reopen_reason_text(&t, &reason_key(u.reason));
            if let Some(why) = u.reversal_reason.as_deref().filter(|r| !r.is_empty()) {
                reason = format!("{reason} ({})", reopen_reason_text(&t, why));
            }
            let done = u.outcome == UnallocationOutcome::Done;
            ReopenedRow {
                id: u.id,
                date: u.created_at,
                amount: money(u.amount),
                reason,
                outcome: t(if done {
                    "reopened.outcome.done"
                } else {
                    "reopened.outcome.failed"
                }),
                outcome_kind: if done { "success" } else { "danger" },
            }
        })
        .collect();
    let tasks = task::open_of_bill(&db, bill.id)
        .await?
        .into_iter()
        .map(|task| TaskRow {
            id: task.id,
            kind: t(&format!("tasks.kind.{}", task.kind.as_str())),
            message: task.message,
        })
        .collect();
    let mut amounts = BTreeMap::from([
        ("net_total", money(bill.net_total)),
        ("vat_total", money(bill.vat_total)),
        ("gross_total", money(bill.gross_total)),
    ]);
    if let (Some(aed), false) = (bill.gross_total_aed, bill.currency == "AED") {
        amounts.insert("gross_total_aed", amount_text("AED", aed));
    }
    let entity = match bill.legal_entity_id {
        Some(id) => LegalEntity::find(&db, id).await?.map(|e| e.name_en),
        None => None,
    }
    .unwrap_or_default();
    let history = state_machine::history(&db, SupplierBill::TABLE, bill.id).await?;
    let timeline =
        state_machine::timeline(&history, Some(&state_machine::info_of::<SupplierBill>()));
    let draft = bill.status == SupplierBillStatus::Draft;
    let may_create = user.has_permission(permissions::SUPPLIER_BILLS_CREATE);
    let (drawings, approval_cards) = drawings_view(
        &state,
        &lang,
        &user,
        &bill,
        &mapped,
        &bill_lines,
        (can_see_drawings, may_create),
    )
    .await?;
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: bill.supplier_invoice_number.clone(),
        supplier: supplier_name(&state, bill.party_id).await,
        entity,
        status_label: t(&format!("supplier_bill.status.{}", bill.status.as_str())),
        status_kind: status_kind(bill.status),
        amounts,
        contract: bill.contract().map(|c| c.to_string()),
        period: bill.period().map(|(start, end)| format!("{start} - {end}")),
        created_by: user_name(&db, bill.created_by).await,
        approved_by: user_name(&db, bill.approved_by).await,
        lines,
        vat,
        exceptions,
        allocations,
        reopened,
        tasks,
        can_tasks: user.has_permission(permissions::RECEIVABLES_VIEW),
        timeline,
        can_edit: draft && may_create,
        can_submit: draft && may_create,
        sheets: sheets_for(&t, &user, &bill),
        drawings,
        approvals: approval_cards,
        bill,
    }))
}

/// The owner drawings of `bill` as the page shows them, with the approval
/// cards of the drawing that waits. `rights` is whether the user may see the
/// lines and whether they may change the bill.
async fn drawings_view(
    state: &AppState,
    lang: &Lang,
    user: &AuthUser,
    bill: &SupplierBill,
    mapped: &Mapped,
    bill_lines: &[BillLine],
    rights: (bool, bool),
) -> Result<(DrawingsView, Vec<ApprovalCard>)> {
    let (can_see, may_create) = rights;
    let t = texts_in(lang);
    let db = &state.db;
    let drawing_lines = drawings::drawing_lines(mapped, bill_lines.to_vec());
    let approvals_of = DrawingApproval::for_source(db, &bill.subject()).await?;
    let recodings = drawings::recodings_of(db, bill.id).await?;
    if drawing_lines.is_empty() && approvals_of.is_empty() && recodings.is_empty() {
        return Ok((DrawingsView::default(), Vec::new()));
    }
    let total = amount_of(&drawing_lines);
    let latest = approvals_of.first();
    let (status, status_kind) = match latest {
        None => (t("drawings.status.none"), "warning"),
        Some(a) => (
            if a.self_approved {
                t("drawings.status.self_approved")
            } else {
                t(&format!("drawings.status.{}", a.status.as_str()))
            },
            match a.status {
                DrawingStatus::Draft => "info",
                DrawingStatus::Requested => "warning",
                DrawingStatus::Approved => "success",
                DrawingStatus::Rejected => "error",
            },
        ),
    };
    let editable = matches!(
        bill.status,
        SupplierBillStatus::Draft | SupplierBillStatus::Matched | SupplierBillStatus::Exception
    );
    let covered = approvals_of
        .iter()
        .any(|a| a.amount == total && matches!(a.status, DrawingStatus::Requested | DrawingStatus::Approved));
    let mut cards = Vec::new();
    for approval in approvals_of
        .iter()
        .filter(|a| a.status == DrawingStatus::Requested)
    {
        cards.extend(approvals::cards_for(db, &approval.subject(), user, lang).await?);
    }
    let positions: BTreeMap<i64, i64> = bill_lines.iter().map(|l| (l.id, l.position)).collect();
    let by = names_of(db, recodings.iter().filter_map(|r| r.recoded_by).collect()).await?;
    let view = DrawingsView {
        present: true,
        total: amount_text("AED", total),
        status,
        status_kind,
        reason: latest
            .filter(|a| a.status == DrawingStatus::Rejected)
            .and_then(|a| a.rejected_reason.clone()),
        can_request: may_create && editable && !drawing_lines.is_empty() && !covered,
        can_recode: may_create && editable,
        lines: bill_lines
            .iter()
            .map(|l| {
                let hidden = !can_see && mapped.is_drawings_code(l.cost_code.as_deref());
                let label = if hidden {
                    format!("{}. {}", l.position, t("drawings.hidden"))
                } else {
                    format!("{}. {}", l.position, l.description)
                };
                (l.id.to_string(), label)
            })
            .collect(),
        recodings: recodings
            .iter()
            .map(|r| RecodingRow {
                line: positions
                    .get(&r.line_id)
                    .map_or_else(|| format!("#{}", r.line_id), ToString::to_string),
                from: r.from_code.clone().unwrap_or_default(),
                to: r.to_code.clone().unwrap_or_default(),
                by: r
                    .recoded_by
                    .and_then(|id| by.get(&id).cloned())
                    .unwrap_or_default(),
                at: r.created_at,
            })
            .collect(),
    };
    Ok((view, cards))
}

/// The names of users `ids`.
async fn names_of(db: &Db, ids: Vec<i64>) -> Result<BTreeMap<i64, String>> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let users = User::query().where_in("id", ids).get(db).await?;
    Ok(users.into_iter().map(|user| (user.id, user.name)).collect())
}

/// One line row of the form.
#[derive(Serialize, Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LineSlot {
    n: usize,
    description: String,
    quantity: String,
    unit_price: String,
    vat: String,
    service_date: String,
    cost_code: String,
}

/// What the form page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/bill_form.html")]
pub(crate) struct FormPage {
    tx: Texts,
    title: String,
    /// Where the form is sent.
    action: String,
    /// Where "Cancel" goes.
    cancel: String,
    /// The header fields' starting values.
    values: BTreeMap<&'static str, String>,
    /// The legal entities, `(id, label)`.
    entities: Vec<(String, String)>,
    /// The VAT categories, `(code, label)`.
    categories: Vec<(String, String)>,
    lines: Vec<LineSlot>,
}

/// `value` with `scale` decimals as text, e.g. 1500 with scale 3 and keep 0
/// is `1.5`; the fraction keeps at least `keep` digits.
fn decimal_text(value: i64, scale: u32, keep: usize) -> String {
    let factor = 10_i64.pow(scale);
    let sign = if value < 0 { "-" } else { "" };
    let value = value.unsigned_abs();
    let whole = value / factor.unsigned_abs();
    let mut fraction = format!(
        "{:0width$}",
        value % factor.unsigned_abs(),
        width = scale as usize
    );
    while fraction.len() > keep && fraction.ends_with('0') {
        fraction.pop();
    }
    if fraction.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction}")
    }
}

/// `text` ("150", "1,500.5", "3.6725") as a whole number of 10^-`scale`
/// units; `None` for anything else, a negative number or too many decimals.
fn parse_decimal(text: &str, scale: u32) -> Option<i64> {
    let text = text.trim().replace(',', "");
    let (whole, fraction) = text.split_once('.').unwrap_or((text.as_str(), ""));
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if !digits(whole) || !digits(fraction) || fraction.len() > scale as usize {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let fraction: i64 = format!("{fraction:0<width$}", width = scale as usize)
        .parse()
        .ok()?;
    whole.checked_mul(10_i64.pow(scale))?.checked_add(fraction)
}

/// `text` as a date (`2026-10-05`).
fn parse_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").ok()
}

/// The VAT category with this code.
fn category_of(code: &str) -> Option<VatCategory> {
    VatCategory::ALL
        .iter()
        .copied()
        .find(|c| c.as_str() == code.trim())
}

/// The line rows of the form `form` (`line_<n>_description`, `_quantity`,
/// `_unit_price`, `_vat`, `_service_date`). A row with no description,
/// quantity and price is skipped. A field that is not valid is returned as
/// its name and the suffix of its error text.
fn lines_of_form(
    form: &BTreeMap<String, String>,
) -> std::result::Result<Vec<BillLineDraft>, (String, &'static str)> {
    let get = |n: usize, field: &str| {
        form.get(&format!("line_{n}_{field}"))
            .map_or("", |v| v.trim())
    };
    let mut lines = Vec::new();
    for n in 1..=MAX_SLOTS {
        let (description, quantity, price) = (
            get(n, "description"),
            get(n, "quantity"),
            get(n, "unit_price"),
        );
        if description.is_empty() && quantity.is_empty() && price.is_empty() {
            continue;
        }
        let name = |field: &str| format!("line_{n}_{field}");
        let quantity_milli = if quantity.is_empty() {
            1_000
        } else {
            parse_decimal(quantity, 3).ok_or((name("quantity"), "quantity"))?
        };
        let unit_price = parse_decimal(price, 2).ok_or((name("unit_price"), "amount"))?;
        let category = match get(n, "vat") {
            "" => VatCategory::Standard,
            code => category_of(code).ok_or((name("vat"), "category"))?,
        };
        let mut line =
            BillLineDraft::new(description, unit_price, category).quantity_milli(quantity_milli);
        match get(n, "service_date") {
            "" => {}
            date => {
                line = line.service_date(parse_date(date).ok_or((name("service_date"), "date"))?);
            }
        }
        line = line.cost_code(get(n, "cost_code"));
        lines.push(line);
    }
    Ok(lines)
}

/// A 422 with `message` under `field`, keeping the input.
fn invalid<F: Serialize>(field: &str, message: String, form: &F) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// A service refusal as the response: a 422 message goes under `base`.
fn refused<F: Serialize>(error: Error, form: &F) -> Error {
    match error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message) => invalid("base", message, form),
        other => other,
    }
}

/// The draft a submitted form describes.
async fn draft_of_form(
    state: &AppState,
    lang: &Lang,
    form: &BTreeMap<String, String>,
) -> Result<BillDraft> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let error = |field: &str, key: &str| {
        invalid(
            field,
            texts::t(lang, &format!("wl_invoicing.supplier_bill.errors.{key}")),
            form,
        )
    };
    let code = get("supplier");
    let supplier = match code {
        "" => None,
        code => {
            wl_kernel::parties::directory(state)
                .find_by_code(&state.db, code)
                .await?
        }
    }
    .ok_or_else(|| error("supplier", "supplier"))?;
    let bill_date = parse_date(get("bill_date")).ok_or_else(|| error("bill_date", "date"))?;
    let mut draft = BillDraft::new(supplier.id, get("supplier_invoice_number"), bill_date);
    draft.legal_entity_id = match get("legal_entity_id") {
        "" => None,
        id => {
            let id: i64 = id.parse().map_err(|_| error("legal_entity_id", "entity"))?;
            LegalEntity::find(&state.db, id)
                .await?
                .ok_or_else(|| error("legal_entity_id", "entity"))?;
            Some(id)
        }
    };
    draft.due_date = match get("due_date") {
        "" => None,
        date => Some(parse_date(date).ok_or_else(|| error("due_date", "date"))?),
    };
    let currency = get("currency");
    if !currency.is_empty() {
        draft.currency = currency.to_owned();
    }
    draft.exchange_rate_micro = match get("exchange_rate") {
        "" => None,
        rate => Some(parse_decimal(rate, 6).ok_or_else(|| error("exchange_rate", "rate"))?),
    };
    draft.contract = match (get("contract_type"), get("contract_id")) {
        ("", "") => None,
        (kind, id) if !kind.is_empty() => Some(wl_kernel::Subject::new(
            kind,
            id.parse().map_err(|_| error("contract_id", "contract"))?,
        )),
        _ => return Err(error("contract_type", "contract")),
    };
    draft.period = match (get("period_start"), get("period_end")) {
        ("", "") => None,
        (start, end) => Some((
            parse_date(start).ok_or_else(|| error("period_start", "date"))?,
            parse_date(end).ok_or_else(|| error("period_end", "date"))?,
        )),
    };
    draft.lines = lines_of_form(form).map_err(|(field, key)| error(&field, key))?;
    Ok(draft)
}

/// The legal entities and VAT categories the form offers.
async fn options(db: &Db, lang: &Lang) -> Result<(Vec<(String, String)>, Vec<(String, String)>)> {
    let t = texts_in(lang);
    let entities = LegalEntity::query()
        .order_by(LegalEntity::ID)
        .get(db)
        .await?
        .into_iter()
        .filter(LegalEntity::is_active)
        .map(|e| (e.id.to_string(), format!("{} - {}", e.code, e.name_en)))
        .collect();
    let categories = VatCategory::ALL
        .iter()
        .map(|c| {
            (
                c.as_str().to_owned(),
                t(&format!("vat_category.{}", c.as_str())),
            )
        })
        .collect();
    Ok((entities, categories))
}

/// The form page for a new bill (`bill` is `None`) or to change draft `bill`.
async fn form_page(state: &AppState, lang: &Lang, bill: Option<&SupplierBill>) -> Result<View> {
    let t = texts_in(lang);
    let (entities, categories) = options(&state.db, lang).await?;
    let mut values: BTreeMap<&'static str, String> = BTreeMap::new();
    let mut lines: Vec<LineSlot> = Vec::new();
    if let Some(bill) = bill {
        let directory = wl_kernel::parties::directory(state);
        let code = directory
            .find(&state.db, bill.supplier())
            .await?
            .map_or_else(|| bill.party_id.to_string(), |p| p.code);
        let date = |d: Option<NaiveDate>| d.map(|d| d.to_string()).unwrap_or_default();
        values.insert("supplier", code);
        values.insert(
            "legal_entity_id",
            bill.legal_entity_id
                .map(|id| id.to_string())
                .unwrap_or_default(),
        );
        values.insert(
            "supplier_invoice_number",
            bill.supplier_invoice_number.clone(),
        );
        values.insert("bill_date", bill.bill_date.to_string());
        values.insert("due_date", date(bill.due_date));
        values.insert("currency", bill.currency.clone());
        values.insert(
            "exchange_rate",
            bill.exchange_rate_micro
                .map(|r| decimal_text(r, 6, 0))
                .unwrap_or_default(),
        );
        values.insert(
            "contract_type",
            bill.contract_type.clone().unwrap_or_default(),
        );
        values.insert(
            "contract_id",
            bill.contract_id
                .map(|id| id.to_string())
                .unwrap_or_default(),
        );
        values.insert("period_start", date(bill.period_start));
        values.insert("period_end", date(bill.period_end));
        for (i, line) in supplier_bill::lines_of(&state.db, bill.id)
            .await?
            .iter()
            .enumerate()
        {
            lines.push(LineSlot {
                n: i + 1,
                description: line.description.clone(),
                quantity: decimal_text(line.quantity_milli, 3, 0),
                unit_price: decimal_text(line.unit_price, 2, 2),
                vat: line.vat_category.as_str().to_owned(),
                service_date: date(line.service_date),
                cost_code: line.cost_code.clone().unwrap_or_default(),
            });
        }
    } else {
        values.insert(
            "currency",
            wl_kernel::invoicing::DEFAULT_CURRENCY.to_owned(),
        );
    }
    let slots = (lines.len() + 2).max(MIN_SLOTS);
    for n in lines.len() + 1..=slots {
        lines.push(LineSlot {
            n,
            vat: VatCategory::Standard.as_str().to_owned(),
            ..LineSlot::default()
        });
    }
    let (title, action, cancel) = match bill {
        Some(bill) => (
            t("supplier_bill.form.edit_title"),
            format!("/supplier-bills/{}", bill.id),
            format!("/supplier-bills/{}", bill.id),
        ),
        None => (
            t("supplier_bill.form.new_title"),
            "/supplier-bills".to_owned(),
            "/supplier-bills".to_owned(),
        ),
    };
    Ok(View::typed(FormPage {
        tx: texts::all(lang),
        title,
        action,
        cancel,
        values,
        entities,
        categories,
        lines,
    }))
}

/// `GET /supplier-bills/new`: the empty form.
async fn new_form(State(state): State<AppState>, lang: Lang) -> Result<View> {
    form_page(&state, &lang, None).await
}

/// `GET /supplier-bills/{id}/edit`: the form of a draft; any other bill is
/// 422.
async fn edit_form(
    State(state): State<AppState>,
    lang: Lang,
    Found(bill): Found<SupplierBill>,
) -> Result<View> {
    if bill.status != SupplierBillStatus::Draft {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            texts::t(&lang, "wl_invoicing.supplier_bill.errors.not_draft"),
        ));
    }
    form_page(&state, &lang, Some(&bill)).await
}

/// `POST /supplier-bills`: creates the draft and opens its page.
async fn store(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let draft = draft_of_form(&state, &lang, &form).await?;
    let bill = supplier_bill::create(&state.db, &draft, Some(user.id))
        .await
        .map_err(|e| refused(e, &form))?;
    Ok((
        Toast::success(texts::t(&lang, "wl_invoicing.supplier_bill.form.saved")),
        Redirect::route("wl_invoicing.supplier_bill", &[&bill.id])?,
    )
        .into_response())
}

/// `POST /supplier-bills/{id}`: replaces the header and lines of a draft.
async fn update(
    State(state): State<AppState>,
    lang: Lang,
    Found(bill): Found<SupplierBill>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let draft = draft_of_form(&state, &lang, &form).await?;
    supplier_bill::change(&state.db, bill.id, &draft)
        .await
        .map_err(|e| refused(e, &form))?;
    Ok((
        Toast::success(texts::t(&lang, "wl_invoicing.supplier_bill.form.saved")),
        Redirect::route("wl_invoicing.supplier_bill", &[&bill.id])?,
    )
        .into_response())
}

/// `POST /supplier-bills/{id}/delete`: deletes a draft (422 for any other
/// bill) and goes back to the list.
async fn destroy(
    State(state): State<AppState>,
    lang: Lang,
    Found(bill): Found<SupplierBill>,
) -> Result<Response> {
    supplier_bill::discard(&state.db, bill.id).await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_invoicing.supplier_bill.discard.done")),
        Redirect::route("wl_invoicing.supplier_bills", &[])?,
    )
        .into_response())
}

/// `POST /supplier-bills/{id}/submit`: submits the draft (`matched`, or
/// `exception` when it may duplicate an earlier bill) and reloads its page.
async fn store_submit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(bill): Found<SupplierBill>,
) -> Result<Response> {
    let done = submit(&state, By::User(&user), bill.id)
        .await
        .map_err(TransitionError::into_error)?;
    let key = if done.status == SupplierBillStatus::Exception {
        "wl_invoicing.supplier_bill.submit.held"
    } else {
        "wl_invoicing.supplier_bill.submit.done"
    };
    let toast = if done.status == SupplierBillStatus::Exception {
        Toast::warning(texts::t(&lang, key))
    } else {
        Toast::success(texts::t(&lang, key))
    };
    Ok((
        toast,
        Redirect::route("wl_invoicing.supplier_bill", &[&bill.id])?,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english_text(key: &str) -> String {
        texts::english(&format!("wl_invoicing.{key}")).to_owned()
    }

    fn form(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn decimals_are_read_and_written_back() {
        assert_eq!(parse_decimal("150", 2), Some(15_000));
        assert_eq!(parse_decimal("1,500.5", 2), Some(150_050));
        assert_eq!(parse_decimal("0", 2), Some(0));
        assert_eq!(parse_decimal("2.5", 3), Some(2_500));
        assert_eq!(parse_decimal("3.6725", 6), Some(3_672_500));
        assert_eq!(parse_decimal("1.234", 2), None);
        assert_eq!(parse_decimal("-3", 2), None);
        assert_eq!(parse_decimal("abc", 2), None);
        assert_eq!(parse_decimal("", 2), None);
        assert_eq!(decimal_text(15_000, 2, 2), "150.00");
        assert_eq!(decimal_text(2_500, 3, 0), "2.5");
        assert_eq!(decimal_text(1_000, 3, 0), "1");
        assert_eq!(decimal_text(3_672_500, 6, 0), "3.6725");
        assert_eq!(decimal_text(5, 2, 2), "0.05");
    }

    #[test]
    fn the_form_gives_its_filled_rows_in_order() {
        let lines = lines_of_form(&form(&[
            ("line_2_description", "Pump"),
            ("line_2_quantity", "2.5"),
            ("line_2_unit_price", "10.00"),
            ("line_2_vat", "exempt"),
            ("line_2_service_date", "2026-10-03"),
            ("line_1_description", "Labour"),
            ("line_1_unit_price", "200"),
            ("line_3_vat", "standard"),
            ("supplier", "P-00001"),
        ]))
        .unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].description, "Labour");
        assert_eq!(lines[0].quantity_milli, 1_000);
        assert_eq!(lines[0].unit_price, 20_000);
        assert_eq!(lines[0].vat_category, VatCategory::Standard);
        assert_eq!(lines[1].quantity_milli, 2_500);
        assert_eq!(lines[1].vat_category, VatCategory::Exempt);
        assert_eq!(lines[1].service_date, NaiveDate::from_ymd_opt(2026, 10, 3));
    }

    #[test]
    fn a_bad_field_is_named() {
        let bad = |pairs: &[(&str, &str)]| lines_of_form(&form(pairs)).unwrap_err();
        assert_eq!(
            bad(&[("line_1_description", "A"), ("line_1_unit_price", "x")]),
            ("line_1_unit_price".to_owned(), "amount")
        );
        assert_eq!(
            bad(&[("line_1_description", "A")]),
            ("line_1_unit_price".to_owned(), "amount")
        );
        assert_eq!(
            bad(&[
                ("line_4_description", "A"),
                ("line_4_unit_price", "1"),
                ("line_4_quantity", "-1")
            ]),
            ("line_4_quantity".to_owned(), "quantity")
        );
        assert_eq!(
            bad(&[
                ("line_1_description", "A"),
                ("line_1_unit_price", "1"),
                ("line_1_vat", "nope")
            ]),
            ("line_1_vat".to_owned(), "category")
        );
        assert_eq!(
            bad(&[
                ("line_1_description", "A"),
                ("line_1_unit_price", "1"),
                ("line_1_service_date", "03/10")
            ]),
            ("line_1_service_date".to_owned(), "date")
        );
    }

    #[test]
    fn the_grid_lists_the_supplier_bill_columns() {
        let grid = grid_with(&english_text);
        let keys: Vec<&str> = grid.columns().iter().map(Column::key).collect();
        assert_eq!(
            keys,
            [
                "supplier_invoice_number",
                "party_id",
                "entity",
                "bill_date",
                "due_date",
                "gross_total",
                "status",
                "created_at"
            ]
        );
    }

    #[test]
    fn every_status_and_error_has_a_text() {
        let mut keys: Vec<String> = SupplierBillStatus::ALL
            .iter()
            .map(|s| format!("supplier_bill.status.{}", s.as_str()))
            .collect();
        keys.extend(
            [
                "supplier",
                "date",
                "entity",
                "rate",
                "contract",
                "quantity",
                "amount",
                "category",
                "not_draft",
            ]
            .iter()
            .map(|k| format!("supplier_bill.errors.{k}")),
        );
        keys.extend(
            VatCategory::ALL
                .iter()
                .map(|c| format!("vat_category.{}", c.as_str())),
        );
        for key in keys {
            assert_ne!(english_text(&key), format!("wl_invoicing.{key}"), "{key}");
        }
    }
}
