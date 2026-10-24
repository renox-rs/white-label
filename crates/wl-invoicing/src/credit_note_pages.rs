//! The credit note pages and the credit actions (wl-invoicing §6, A13):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /credit-notes` | `wl_invoicing.credit_notes` | `invoices.view` |
//! | `GET /credit-notes/{id}` | `wl_invoicing.credit_note` | `invoices.view` |
//! | `POST /credit-notes/{id}/issue` | `wl_invoicing.credit_note_issue` | `invoices.credit` |
//! | `POST /credit-notes/{id}/use` | `wl_invoicing.credit_note_use` | `invoices.credit` |
//! | `POST /credit-notes/{id}/refund` | `wl_invoicing.credit_note_refund` | `invoices.credit` |
//! | `POST /invoices/{id}/credit` | `wl_invoicing.invoice_credit` | `invoices.credit` |
//! | `POST /invoices/{id}/credit-notes` | `wl_invoicing.invoice_credit_note` | `invoices.credit` |
//!
//! The list is a `renox::grid`. The page of one credit note shows the issuer
//! and party snapshots, the lines with their VAT category, the totals per
//! category, the remaining credit, the movements, the history and the
//! issue, use-on-invoice and refund actions. The invoice page gets the
//! "Credit in full" sheet, the "New credit note" form and the list of the
//! invoice's credit notes ([`invoice_credit`]). The POST routes call the
//! services: 403 without `invoices.credit`, 422 for a refused guard (the
//! message under the field). The credit note's other transitions (`apply`,
//! `reopen`) are the system's, so the page has no transition buttons beyond
//! `issue`. Templates are compiled in and can be replaced by the app.

use std::collections::{BTreeMap, HashMap};

use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{self, TransitionError};
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::IssuerSnapshot;
use wl_kernel::invoicing::{CreditLine, CreditNoteStatus, INVOICE_SUBJECT_TYPE, InvoiceStatus};
use wl_kernel::notify::fill;
use wl_kernel::payments::{PaymentMethod, REFUND_SUBJECT_TYPE};

use crate::allocate::amount_text;
use crate::credit::credit_in_full;
use crate::credit_note::CreditNote;
use crate::credit_note_movement::{CreditNoteMovement, for_credit_note};
use crate::credit_notes::{self, By};
use crate::invoice::{Invoice, PartySnapshot};
use crate::invoicer::lines_of as invoice_lines_of;
use crate::permissions;
use crate::refund::request_refund;
use crate::texts;
use crate::vat::VatTotals;

/// The texts a template reads as `tx['credit_note.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The package text `key` (without the `wl_invoicing.` prefix) in a language.
type Text<'a> = &'a dyn Fn(&str) -> String;

/// The package texts in `lang`.
fn texts_in(lang: &Lang) -> impl Fn(&str) -> String + '_ {
    move |key: &str| texts::t(lang, &format!("wl_invoicing.{key}"))
}

/// The module's credit note routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(wl_kernel::scopes::anywhere(
            Routes::new()
                .get("/credit-notes", index)
                .name("wl_invoicing.credit_notes")
                .get("/credit-notes/{id}", show)
                .name("wl_invoicing.credit_note"),
            permissions::VIEW,
        ))
        .merge(
            Routes::new()
                .post("/credit-notes/{id}/issue", store_issue)
                .name("wl_invoicing.credit_note_issue")
                .post("/credit-notes/{id}/use", store_use)
                .name("wl_invoicing.credit_note_use")
                .post("/credit-notes/{id}/refund", store_refund)
                .name("wl_invoicing.credit_note_refund")
                .post("/invoices/{id}/credit", store_credit)
                .name("wl_invoicing.invoice_credit")
                .post("/invoices/{id}/credit-notes", store_credit_note)
                .name("wl_invoicing.invoice_credit_note")
                .require_permission(permissions::CREDIT),
        )
}

/// The badge colour of a status.
fn status_kind(status: CreditNoteStatus) -> &'static str {
    match status {
        CreditNoteStatus::Draft => "neutral",
        CreditNoteStatus::Issued => "info",
        CreditNoteStatus::Applied => "success",
    }
}

/// The credit notes grid: search on the number and the party, filters by
/// status and issuing entity, money in each credit note's currency.
pub fn grid(lang: &Lang, can_export: bool) -> Grid {
    grid_with(&texts_in(lang), can_export)
}

/// [`grid`] with the texts of `t`.
fn grid_with(t: Text<'_>, can_export: bool) -> Grid {
    let statuses: Vec<(String, String)> = CreditNoteStatus::ALL
        .iter()
        .map(|s| {
            (
                s.as_str().to_owned(),
                t(&format!("credit_note.status.{}", s.as_str())),
            )
        })
        .collect();
    let grid = Grid::new("wl-invoicing-credit-notes")
        .title(&t("credit_note.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &t("fields.number"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/credit-notes/{id}"),
        )
        .column(Column::related(
            "invoice",
            &t("credit_note.fields.invoice"),
            "wl_invoicing_invoices",
            "invoice_id",
            "number",
        ))
        .column(
            Column::text("party_name", &t("fields.party"))
                .computed("json_extract({T}.\"party_snapshot\", '$.legal_name_en')")
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
        .column(Column::date("issue_date", &t("fields.issue_date")))
        .column(
            Column::money("gross_total", &t("fields.gross_total"))
                .currency_from("currency")
                .summary(Summary::Sum),
        )
        .column(
            Column::money("remaining", &t("credit_note.fields.remaining"))
                .currency_from("currency")
                .summary(Summary::Sum)
                .mobile(),
        )
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("draft", "neutral"),
                    ("issued", "info"),
                    ("applied", "success"),
                ])
                .mobile(),
        )
        .column(Column::datetime("created_at", &t("fields.created_at")).hidden())
        .sort_by("-created_at")
        .row_url("/credit-notes/{id}")
        .cards_on_mobile()
        .empty_state(&t("credit_note.empty"), None);
    if can_export { grid.exports() } else { grid }
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/credit_notes.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    credit_notes: GridPage<CreditNote>,
}

/// `GET /credit-notes`: the grid.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    // The credit notes of the invoices in the user's scope.
    let reach = wl_kernel::scopes::reach(&state.db, &user, permissions::VIEW).await?;
    let query = reach
        .apply_ids(
            &state.db,
            CreditNote::query(),
            "invoice_id",
            wl_kernel::invoicing::INVOICE_SUBJECT_TYPE,
        )
        .await?;
    let can_export = crate::may_export(&user, permissions::CREDIT);
    let grid = grid(&lang, can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let credit_notes = grid.page(query, &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        credit_notes,
    })
    .into_response())
}

/// One line of a credit note.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct LineRow {
    id: i64,
    description: String,
    category: String,
    net: String,
    vat: String,
    gross: String,
}

/// One VAT category's totals.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct VatRow {
    category: String,
    net: String,
    vat: String,
}

/// One row of the ledger.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct MovementRow {
    id: i64,
    kind: String,
    amount: String,
    /// The invoice or refund it went to, or came back from.
    target: String,
    target_link: Option<String>,
    attempt: Option<i64>,
    at: String,
}

/// The page of the target of a movement, when it has one.
fn target_link(movement: &CreditNoteMovement) -> Option<String> {
    let id = movement.target_id?;
    match movement.target_type.as_deref()? {
        INVOICE_SUBJECT_TYPE => Some(format!("/invoices/{id}")),
        REFUND_SUBJECT_TYPE => Some(format!("/refunds/{id}")),
        _ => None,
    }
}

/// What the credit note page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/credit_note.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    credit_note: CreditNote,
    /// The number, or "Draft".
    title: String,
    status_label: String,
    status_kind: &'static str,
    kind_label: String,
    /// The credited invoice's number and page.
    invoice_number: String,
    invoice_link: String,
    issuer: Option<IssuerSnapshot>,
    party: Option<PartySnapshot>,
    /// Net, VAT, gross and remaining, formatted.
    amounts: BTreeMap<&'static str, String>,
    lines: Vec<LineRow>,
    vat: Vec<VatRow>,
    movements: Vec<MovementRow>,
    /// The credit note's history.
    timeline: Vec<Value>,
    can_issue: bool,
    can_use: bool,
    can_refund: bool,
    /// The open invoices of the party in the currency, `(id, label)`.
    invoices: Vec<(String, String)>,
    /// The refund methods, `(code, label)`; the first is "the default".
    methods: Vec<(String, String)>,
    use_description: String,
    refund_description: String,
}

/// The totals of `totals` as rows.
fn vat_rows(t: Text<'_>, currency: &str, totals: &VatTotals) -> Vec<VatRow> {
    totals
        .categories
        .iter()
        .map(|c| VatRow {
            category: t(&format!("vat_category.{}", c.category.as_str())),
            net: amount_text(currency, c.net),
            vat: amount_text(currency, c.vat),
        })
        .collect()
}

/// The ledger of `note` as rows.
fn movement_rows(
    t: Text<'_>,
    note: &CreditNote,
    movements: &[CreditNoteMovement],
) -> Vec<MovementRow> {
    movements
        .iter()
        .map(|m| MovementRow {
            id: m.id,
            kind: t(&format!("credit_note.movement.{}", m.kind.as_str())),
            amount: amount_text(&note.currency, m.amount),
            target: m.target().map(|s| s.to_string()).unwrap_or_default(),
            target_link: target_link(m),
            attempt: m.attempt,
            at: m
                .created_at
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default(),
        })
        .collect()
}

/// `GET /credit-notes/{id}`: the snapshots, lines, VAT, remaining, ledger,
/// history and the actions.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(note): Found<CreditNote>,
) -> Result<View> {
    let reach = wl_kernel::scopes::reach(&state.db, &user, permissions::VIEW).await?;
    let invoice =
        wl_kernel::Subject::new(wl_kernel::invoicing::INVOICE_SUBJECT_TYPE, note.invoice_id);
    if !reach.contains(&state.db, &invoice).await? {
        return Err(Error::Forbidden);
    }
    let t = texts_in(&lang);
    let db = state.db.clone();
    let money = |fils: i64| amount_text(&note.currency, fils);
    let lines = credit_notes::lines_of(&db, note.id)
        .await?
        .iter()
        .map(|line| LineRow {
            id: line.id,
            description: line.description.clone(),
            category: t(&format!("vat_category.{}", line.vat_category.as_str())),
            net: money(line.net_amount),
            vat: money(line.vat_amount),
            gross: money(line.gross_amount),
        })
        .collect();
    let totals = credit_notes::totals_of(&db, note.id).await?;
    let vat = vat_rows(&t, &note.currency, &totals);
    let ledger = for_credit_note(&db, note.id).await?;
    let movements = movement_rows(&t, &note, &ledger);
    let amounts = BTreeMap::from([
        ("net_total", money(note.net_total)),
        ("vat_total", money(note.vat_total)),
        ("gross_total", money(note.gross_total)),
        ("remaining", money(note.remaining)),
    ]);
    let history = state_machine::history(&db, CreditNote::TABLE, note.id).await?;
    let timeline = state_machine::timeline(&history, Some(&state_machine::info_of::<CreditNote>()));
    let invoice = Invoice::find(&db, note.invoice_id).await?;
    let may_credit = user.has_permission_anywhere(permissions::CREDIT);
    let usable = may_credit && note.status == CreditNoteStatus::Issued && note.remaining > 0;
    let invoices: Vec<(String, String)> = if usable {
        Invoice::query()
            .where_(Invoice::PARTY_ID.eq(note.party_id))
            .where_(Invoice::CURRENCY.eq(note.currency.clone()))
            .where_(Invoice::STATUS.is_in([
                InvoiceStatus::Issued,
                InvoiceStatus::PartiallyPaid,
                InvoiceStatus::Overdue,
            ]))
            .order_by(Invoice::ID)
            .get(&db)
            .await?
            .iter()
            .filter(|i| i.balance > 0)
            .map(|i| {
                (
                    i.id.to_string(),
                    format!(
                        "{} - {}",
                        i.number.clone().unwrap_or_else(|| t("draft_number")),
                        money(i.balance)
                    ),
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut methods = vec![(String::new(), t("credit_note.method.default"))];
    methods.extend(PaymentMethod::ALL.iter().map(|m| {
        (
            m.as_str().to_owned(),
            t(&format!("credit_note.method.{}", m.as_str())),
        )
    }));
    let describe = |key: &str| {
        fill(
            &t(key),
            &renox::serde_json::json!({ "remaining": money(note.remaining) }),
        )
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: note
            .number
            .clone()
            .unwrap_or_else(|| t("credit_note.draft_number")),
        status_label: t(&format!("credit_note.status.{}", note.status.as_str())),
        status_kind: status_kind(note.status),
        kind_label: t(&format!("credit_note.kind.{}", note.kind.as_str())),
        invoice_number: invoice
            .as_ref()
            .and_then(|i| i.number.clone())
            .unwrap_or_else(|| t("draft_number")),
        invoice_link: format!("/invoices/{}", note.invoice_id),
        issuer: note.issuer_snapshot.as_ref().map(|s| s.0.clone()),
        party: note.party_snapshot.as_ref().map(|s| s.0.clone()),
        amounts,
        lines,
        vat,
        movements,
        timeline,
        can_issue: may_credit && note.status == CreditNoteStatus::Draft,
        can_use: usable && !invoices.is_empty(),
        can_refund: usable,
        invoices,
        methods,
        use_description: describe("credit_note.use.description"),
        refund_description: describe("credit_note.refund.description"),
        credit_note: note,
    }))
}

/// `text` ("150", "1,500.5", "150.25") as fils; `None` for anything else or
/// an amount of 0 or less.
pub(crate) fn parse_amount(text: &str) -> Option<i64> {
    let text = text.trim().replace(',', "");
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (text.as_str(), ""),
    };
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if !digits(whole) || !digits(fraction) || fraction.len() > 2 {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let fraction: i64 = format!("{fraction:0<2}").parse().ok()?;
    let fils = whole.checked_mul(100)?.checked_add(fraction)?;
    (fils > 0).then_some(fils)
}

/// A 422 with `message` under `field`, keeping the input.
fn invalid<F: Serialize>(field: &str, message: String, form: &F) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// A service refusal as the response: a guard's message goes under `field`,
/// anything else answers like [`TransitionError::into_error`].
fn refusal<F: Serialize>(field: &str, form: &F, error: TransitionError) -> Error {
    match error {
        TransitionError::Guard(message) => invalid(field, message, form),
        other => other.into_error(),
    }
}

/// `POST /credit-notes/{id}/issue`: issues the draft and reloads its page.
async fn store_issue(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(note): Found<CreditNote>,
) -> Result<Response> {
    credit_notes::issue(&state, By::User(&user), &note)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_invoicing.credit_note.issue.done")),
        Redirect::route("wl_invoicing.credit_note", &[&note.id])?,
    )
        .into_response())
}

/// The "use on an invoice" sheet's form.
#[derive(Deserialize, Serialize, Debug)]
pub struct UseForm {
    /// The invoice's id.
    pub invoice_id: String,
    /// The amount, like `150.00`.
    pub amount: String,
}

/// `POST /credit-notes/{id}/use`: uses an amount of the remaining credit on
/// an open invoice of the party and reloads the page.
async fn store_use(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(note): Found<CreditNote>,
    Form(form): Form<UseForm>,
) -> Result<Response> {
    let amount = parse_amount(&form.amount).ok_or_else(|| {
        invalid(
            "amount",
            texts::t(&lang, "wl_invoicing.credit_note.errors.amount"),
            &form,
        )
    })?;
    let invoice = match form.invoice_id.trim().parse::<i64>() {
        Ok(id) => Invoice::find(&state.db, id).await?,
        Err(_) => None,
    }
    .ok_or_else(|| {
        invalid(
            "invoice_id",
            texts::t(&lang, "wl_invoicing.credit_note.errors.invoice"),
            &form,
        )
    })?;
    credit_notes::use_on_invoice(&state, By::User(&user), &note, &invoice, amount)
        .await
        .map_err(|e| refusal("amount", &form, e))?;
    Ok((
        Toast::success(texts::t(&lang, "wl_invoicing.credit_note.use.done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The refund sheet's form.
#[derive(Deserialize, Serialize, Debug)]
pub struct RefundForm {
    /// The amount, like `150.00`.
    pub amount: String,
    /// A method code, or empty for payments' default.
    pub method: Option<String>,
    /// Why the money goes back.
    pub reason: String,
}

/// `POST /credit-notes/{id}/refund`: asks payments to refund an amount of
/// the remaining credit to the party and reloads the page.
async fn store_refund(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(note): Found<CreditNote>,
    Form(form): Form<RefundForm>,
) -> Result<Response> {
    let amount = parse_amount(&form.amount).ok_or_else(|| {
        invalid(
            "amount",
            texts::t(&lang, "wl_invoicing.credit_note.errors.amount"),
            &form,
        )
    })?;
    let code = form.method.as_deref().map(str::trim).unwrap_or_default();
    let method = if code.is_empty() {
        None
    } else {
        Some(
            PaymentMethod::ALL
                .iter()
                .copied()
                .find(|m| m.as_str() == code)
                .ok_or_else(|| {
                    invalid(
                        "method",
                        texts::t(&lang, "wl_invoicing.credit_note.errors.method"),
                        &form,
                    )
                })?,
        )
    };
    request_refund(&state, By::User(&user), &note, amount, method, &form.reason)
        .await
        .map_err(|e| refusal("amount", &form, e))?;
    Ok((
        Toast::success(texts::t(&lang, "wl_invoicing.credit_note.refund.done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The "Credit in full" sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct CreditForm {
    /// Why the invoice is credited.
    #[validate(required, max = 2000)]
    pub reason: String,
    /// Set when the lines' origins are billed again.
    #[serde(default)]
    pub release_origins: bool,
}

/// `POST /invoices/{id}/credit`: credits the invoice in full and reloads its
/// page. A refusal (something is allocated, the wrong status) answers 422
/// under the reason.
async fn store_credit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(invoice): Found<Invoice>,
    Valid(form): Valid<CreditForm>,
) -> Result<Response> {
    credit_in_full(
        &state,
        &invoice,
        &form.reason,
        form.release_origins,
        By::User(&user),
    )
    .await
    .map_err(|e| refusal("reason", &form, e))?;
    Ok((
        Toast::success(texts::t(&lang, "wl_invoicing.credit_note.credit.done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The nets typed in a new credit note's form, `net_<position>`, as credit
/// lines. A blank field is skipped; anything else that is not an amount is
/// returned as the field's name.
fn credit_lines(form: &HashMap<String, String>) -> std::result::Result<Vec<CreditLine>, String> {
    let mut lines = Vec::new();
    for (key, value) in form {
        let Some(position) = key.strip_prefix("net_").and_then(|p| p.parse::<i64>().ok()) else {
            continue;
        };
        if value.trim().is_empty() {
            continue;
        }
        let net = parse_amount(value).ok_or_else(|| key.clone())?;
        lines.push(CreditLine::new(position, net));
    }
    lines.sort_by_key(|l| l.invoice_line);
    Ok(lines)
}

/// `POST /invoices/{id}/credit-notes`: drafts and issues a credit note for
/// the nets typed per line and reloads the invoice page.
async fn store_credit_note(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(invoice): Found<Invoice>,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response> {
    let lines = credit_lines(&form).map_err(|field| {
        invalid(
            &field,
            texts::t(&lang, "wl_invoicing.credit_note.errors.amount"),
            &form,
        )
    })?;
    let reason = form.get("reason").map(String::as_str).unwrap_or_default();
    if reason.trim().is_empty() {
        return Err(invalid(
            "reason",
            texts::t(&lang, "wl_invoicing.credit_note.errors.reason"),
            &form,
        ));
    }
    credit_notes::credit_partly(&state, invoice.id, &lines, reason, By::User(&user))
        .await
        .map_err(|e| refusal("reason", &form, e))?;
    Ok((
        Toast::success(texts::t(&lang, "wl_invoicing.credit_note.new.done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// One credit note in the invoice page's list.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct NoteRow {
    id: i64,
    number: String,
    status: String,
    status_kind: &'static str,
    issue_date: Option<NaiveDate>,
    gross: String,
    remaining: String,
}

/// One invoice line in the new credit note form.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct CreditLineRow {
    position: i64,
    description: String,
    /// The most that can still be credited on it, formatted.
    remaining: String,
}

/// The credit parts of the invoice page.
#[derive(Serialize, Debug, Clone, Default)]
pub(crate) struct InvoiceCredit {
    /// Whether to offer "Credit in full".
    pub can_credit: bool,
    /// Whether to offer the new credit note form.
    pub can_new: bool,
    /// The invoice's credit notes, oldest first.
    pub notes: Vec<NoteRow>,
    /// The lines that can still be credited.
    pub lines: Vec<CreditLineRow>,
}

/// What the invoice page shows of the credits on `invoice`, for `user`.
pub(crate) async fn invoice_credit(
    db: &Db,
    lang: &Lang,
    user: &AuthUser,
    invoice: &Invoice,
) -> Result<InvoiceCredit> {
    let t = texts_in(lang);
    let notes = credit_notes::for_invoice(db, invoice.id)
        .await?
        .into_iter()
        .map(|n| NoteRow {
            id: n.id,
            number: n
                .number
                .clone()
                .unwrap_or_else(|| t("credit_note.draft_number")),
            status: t(&format!("credit_note.status.{}", n.status.as_str())),
            status_kind: status_kind(n.status),
            issue_date: n.issue_date,
            gross: amount_text(&n.currency, n.gross_total),
            remaining: amount_text(&n.currency, n.remaining),
        })
        .collect();
    let may_credit = user.has_permission_anywhere(permissions::CREDIT);
    let can_credit = may_credit
        && [InvoiceStatus::Issued, InvoiceStatus::Overdue].contains(&invoice.status)
        && invoice.allocated_total == 0;
    let creditable = [
        InvoiceStatus::Issued,
        InvoiceStatus::PartiallyPaid,
        InvoiceStatus::Overdue,
        InvoiceStatus::Paid,
    ]
    .contains(&invoice.status);
    let mut lines = Vec::new();
    if may_credit && creditable {
        let credited = credit_notes::credited_by_line(db, invoice.id).await?;
        for line in invoice_lines_of(db, invoice.id).await? {
            let left = line.net_amount - credited.get(&line.id).copied().unwrap_or(0);
            if left > 0 {
                lines.push(CreditLineRow {
                    position: line.position,
                    description: line.description.clone(),
                    remaining: amount_text(&invoice.currency, left),
                });
            }
        }
    }
    Ok(InvoiceCredit {
        can_credit,
        can_new: !lines.is_empty(),
        notes,
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credit_note_movement::MovementKind;

    fn english_text(key: &str) -> String {
        texts::english(&format!("wl_invoicing.{key}")).to_owned()
    }

    #[test]
    fn amounts_are_read_as_fils() {
        assert_eq!(parse_amount("150"), Some(15_000));
        assert_eq!(parse_amount("1,500.5"), Some(150_050));
        assert_eq!(parse_amount(" 0.05 "), Some(5));
        assert_eq!(parse_amount("0"), None);
        assert_eq!(parse_amount("-3"), None);
        assert_eq!(parse_amount("1.234"), None);
        assert_eq!(parse_amount("abc"), None);
        assert_eq!(parse_amount(""), None);
    }

    #[test]
    fn the_form_gives_the_typed_lines_in_order() {
        let form: HashMap<String, String> = [
            ("net_2", "10.00"),
            ("net_1", "200"),
            ("net_3", " "),
            ("reason", "Damaged"),
            ("_token", "x"),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
        assert_eq!(
            credit_lines(&form),
            Ok(vec![CreditLine::new(1, 20_000), CreditLine::new(2, 1_000)])
        );
        let bad: HashMap<String, String> =
            [("net_4".to_owned(), "x".to_owned())].into_iter().collect();
        assert_eq!(credit_lines(&bad), Err("net_4".to_owned()));
    }

    #[test]
    fn the_grid_lists_the_credit_note_columns() {
        let grid = grid_with(&english_text, false);
        let keys: Vec<&str> = grid.columns().iter().map(Column::key).collect();
        assert_eq!(
            keys,
            [
                "number",
                "invoice",
                "party_name",
                "entity",
                "issue_date",
                "gross_total",
                "remaining",
                "status",
                "created_at"
            ]
        );
    }

    #[test]
    fn every_movement_kind_status_and_method_has_a_text() {
        let mut keys: Vec<String> = MovementKind::ALL
            .iter()
            .map(|k| format!("credit_note.movement.{}", k.as_str()))
            .collect();
        keys.extend(
            CreditNoteStatus::ALL
                .iter()
                .map(|s| format!("credit_note.status.{}", s.as_str())),
        );
        keys.extend(
            PaymentMethod::ALL
                .iter()
                .map(|m| format!("credit_note.method.{}", m.as_str())),
        );
        for key in keys {
            assert_ne!(english_text(&key), format!("wl_invoicing.{key}"), "{key}");
        }
    }
}
