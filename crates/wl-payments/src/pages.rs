//! The back-office pages (wl-payments §6, A14):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /payments` | `wl_payments.index` | `payments.view` |
//! | `GET /payments/tasks` | `wl_payments.tasks` | `payments.view` |
//! | `GET /payments/{id}` | `wl_payments.show` | `payments.view` |
//! | `GET /payments/drawings/{id}` | `wl_payments.drawing` | `payments.view` |
//! | `GET /payments/new` | `wl_payments.create` | `payments.record` |
//! | `POST /payments` | `wl_payments.store` | `payments.record` |
//! | `POST /payments/{id}/allocate` | `wl_payments.allocate` | `payments.record` |
//! | `POST /payments/tasks/{id}/done` | `wl_payments.tasks.done` | `payments.record` |
//! | `POST /payments/{id}/reverse` | `wl_payments.reverse` | `payments.reverse` |
//! | `POST /payments/{id}/transitions/{name}` | `wl_payments.transitions.*` | `payments.view` and the transition's permission |
//!
//! `reverse` has its own route because its guards read the input the module
//! gathers (the gateway's answer); the kernel's transition route serves
//! `<rx-transition-actions>` for the others. Every template is compiled in
//! and can be replaced by the app (`resources/views/wl-payments/…`).

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{self, Available, StateMachine, TransitionError};
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::PartyId;
use wl_kernel::drawings::PURPOSE;
use wl_kernel::invoicing::{InvoiceSummary, allocation_proposal, invoicer};
use wl_kernel::parties::directory;
use wl_kernel::payments::{
    AllocationTarget, Direction, PaymentMethod, PaymentStatus, RecordPayment, ReversalReason,
};

use crate::allocate::allocate;
use crate::allocation::Allocation;
use crate::config_of;
use crate::credit;
use crate::drawings::{self, DrawingInput, DrawingStatus, PaymentDrawing};
use crate::followup::{ACCOUNTANT_ROLE, FollowupTask, MANAGER_ROLE, mark_done, money_text};
use crate::payment::Payment;
use crate::permissions;
use crate::record::record;
use crate::reverse::reverse;
use crate::texts;

/// The texts a template reads as `tx['fields.number']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

/// The package text `key` (without the `wl_payments.` prefix) in a language.
pub(crate) type Text<'a> = &'a (dyn Fn(&str) -> String + Sync);

/// The package texts in `lang`.
pub(crate) fn texts_in(lang: &Lang) -> impl Fn(&str) -> String + '_ {
    move |key: &str| texts::t(lang, &format!("wl_payments.{key}"))
}

/// The module's routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/payments", index)
                .name("wl_payments.index")
                .get("/payments/tasks", tasks)
                .name("wl_payments.tasks")
                .get("/payments/{id}", show)
                .name("wl_payments.show")
                .get("/payments/drawings/{id}", show_drawing)
                .name("wl_payments.drawing")
                .group(
                    "/payments",
                    "wl_payments.transitions.",
                    wl_kernel::machines::routes::<Payment>(),
                )
                .require_permission(permissions::VIEW),
        )
        .merge(
            Routes::new()
                .get("/payments/new", new)
                .name("wl_payments.create")
                .post("/payments", store)
                .name("wl_payments.store")
                .post("/payments/{id}/allocate", store_allocation)
                .name("wl_payments.allocate")
                .post("/payments/tasks/{id}/done", store_done)
                .name("wl_payments.tasks.done")
                .require_permission(permissions::RECORD),
        )
        .merge(
            Routes::new()
                .post("/payments/{id}/reverse", store_reverse)
                .name("wl_payments.reverse")
                .require_permission(permissions::REVERSE),
        )
}

/// The badge colour of a status.
fn status_kind(status: PaymentStatus) -> &'static str {
    match status {
        PaymentStatus::Received => "success",
        PaymentStatus::Pending => "info",
        PaymentStatus::Failed => "error",
        PaymentStatus::ChargedBack => "warning",
        PaymentStatus::Reversed => "neutral",
    }
}

/// The payments grid: search on the number and the reference, filters by
/// status, method and entity, money in each payment's currency.
pub fn grid(lang: &Lang) -> Grid {
    grid_with(&texts_in(lang))
}

/// The labels of the codes in `codes`, as `(code, label)` under `group`.
pub(crate) fn options(t: Text<'_>, group: &str, codes: &[&str]) -> Vec<(String, String)> {
    codes
        .iter()
        .map(|code| ((*code).to_owned(), t(&format!("{group}.{code}"))))
        .collect()
}

/// The status codes.
fn status_codes() -> Vec<&'static str> {
    PaymentStatus::ALL.iter().map(|s| s.as_str()).collect()
}

/// The method codes.
fn method_codes() -> Vec<&'static str> {
    PaymentMethod::ALL.iter().map(|m| m.as_str()).collect()
}

/// [`grid`] with the texts of `t`.
fn grid_with(t: Text<'_>) -> Grid {
    Grid::new("wl-payments")
        .title(&t("title"))
        .column(
            Column::text("number", &t("fields.number"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/payments/{id}"),
        )
        .column(Column::date("payment_date", &t("fields.date")))
        .column(
            Column::text("party_name", &t("fields.party"))
                .computed(
                    "(SELECT \"wl_parties_parties\".\"display_name\" FROM \"wl_parties_parties\" \
                     WHERE \"wl_parties_parties\".\"id\" = {T}.\"party_id\")",
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
        .column(Column::select(
            "method",
            &t("fields.method"),
            options(&t, "method", &method_codes()),
        ))
        .column(Column::money("amount", &t("fields.amount")).currency_from("currency"))
        .column(
            Column::money("unallocated", &t("fields.unallocated"))
                .computed(
                    "CASE WHEN {T}.\"status\" = 'received' THEN {T}.\"amount\" - COALESCE(\
                     (SELECT SUM(\"a\".\"amount\") FROM \"wl_payments_allocations\" \"a\" \
                     WHERE \"a\".\"payment_id\" = {T}.\"id\" AND \"a\".\"removed_at\" IS NULL), 0) \
                     ELSE 0 END",
                )
                .currency_from("currency")
                .mobile(),
        )
        .column(
            Column::select(
                "status",
                &t("fields.status"),
                options(&t, "status", &status_codes()),
            )
            .badges(&[
                ("pending", "info"),
                ("received", "success"),
                ("failed", "danger"),
                ("charged_back", "warning"),
                ("reversed", "neutral"),
            ])
            .mobile(),
        )
        .column(Column::datetime("created_at", &t("fields.created_at")).hidden())
        .sort_by("-created_at")
        .row_url("/payments/{id}")
        .cards_on_mobile()
        .empty_state(&t("empty"), Some(&t("empty_hint")))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/index.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    payments: GridPage<Payment>,
    can_record: bool,
}

/// `GET /payments`: the grid.
async fn index(lang: Lang, user: AuthUser, request: GridRequest) -> Result<View> {
    let payments = grid(&lang).page(Payment::query(), &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        payments,
        can_record: user.has_permission(permissions::RECORD),
    }))
}

/// What the new-payment page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/new.html")]
pub(crate) struct NewPage {
    tx: Texts,
    /// The enabled methods, `(code, label)`.
    methods: Vec<(String, String)>,
    /// The active legal entities, `(id, code - name)`.
    entities: Vec<(i64, String)>,
    /// The purposes of a payment, `(code, label)`: none or owner drawings.
    purposes: Vec<(String, String)>,
    /// A drawing or a repayment by the owner, `(code, label)`.
    drawing_directions: Vec<(String, String)>,
}

/// The enabled methods and active entities, for the form.
async fn form_options(
    state: &AppState,
    t: Text<'_>,
) -> Result<(Vec<(String, String)>, Vec<(i64, String)>)> {
    let config = config_of(state);
    let methods = method_codes()
        .into_iter()
        .filter(|code| config.method_enabled(code))
        .map(|code| (code.to_owned(), t(&format!("method.{code}"))))
        .collect();
    let entities = LegalEntity::query()
        .where_(LegalEntity::STATUS.eq("active"))
        .order_by(LegalEntity::CODE)
        .get(&state.db)
        .await?
        .into_iter()
        .map(|e| (e.id, format!("{} - {}", e.code, e.name_en)))
        .collect();
    Ok((methods, entities))
}

/// `GET /payments/new`: the record form.
async fn new(State(state): State<AppState>, lang: Lang) -> Result<View> {
    let (methods, entities) = form_options(&state, &texts_in(&lang)).await?;
    let t = texts_in(&lang);
    Ok(View::typed(NewPage {
        tx: texts::all(&lang),
        methods,
        entities,
        purposes: vec![
            (String::new(), t("purpose.none")),
            (PURPOSE.to_owned(), t("purpose.owner_drawings")),
        ],
        drawing_directions: options(&t, "drawing_direction", &["out", "in"]),
    }))
}

/// The record form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct PaymentForm {
    /// The code of the party who paid.
    #[validate(required, max = 32)]
    pub party_code: Option<String>,
    /// The legal entity's id.
    #[validate(required)]
    pub legal_entity_id: Option<i64>,
    /// The method code.
    #[validate(required, max = 32)]
    pub method: Option<String>,
    /// The amount in dirhams, like `1500.00`.
    #[validate(required, max = 32)]
    pub amount: Option<String>,
    /// The day the money moved; today when empty.
    pub payment_date: Option<NaiveDate>,
    /// The payer's or bank's reference.
    #[validate(max = 200)]
    pub reference: Option<String>,
    /// The bank account the money moved through.
    #[validate(max = 100)]
    pub bank_account: Option<String>,
    /// A cheque's number.
    #[validate(max = 64)]
    pub cheque_number: Option<String>,
    /// A cheque's bank.
    #[validate(max = 100)]
    pub cheque_bank: Option<String>,
    /// A cheque's date.
    pub cheque_date: Option<NaiveDate>,
    /// `owner_drawings` for an owner drawing or a repayment by the owner
    /// (`FIN.ACL.21`); empty for an ordinary payment.
    #[validate(max = 32)]
    pub purpose: Option<String>,
    /// The code of the drawings category; the seeded `owner_drawings` when
    /// empty.
    #[validate(max = 64)]
    pub drawing_category: Option<String>,
    /// `out` (a drawing, the default) or `in` (a repayment by the owner).
    #[validate(max = 8)]
    pub drawing_direction: Option<String>,
}

/// A validation error on `field`, with the form's input filled in again.
pub(crate) fn invalid<F: Serialize>(field: &str, message: String, form: &F) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// `text` ("1500", "1,500.5", "1500.25") as fils; `None` for anything else
/// or an amount of 0 or less.
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

/// The method with code `code`.
fn method_of(code: &str) -> Option<PaymentMethod> {
    PaymentMethod::ALL
        .iter()
        .copied()
        .find(|m| m.as_str() == code.trim())
}

/// The text of `value` when it has any.
pub(crate) fn filled(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// `POST /payments`: records the payment, then shows it. The guards'
/// refusals (a transfer without a reference, a disabled method) answer 422.
async fn store(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<PaymentForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let code = filled(&form.party_code).unwrap_or_default();
    let Some(party) = directory(&state).find_by_code(&state.db, code).await? else {
        return Err(invalid("party_code", t("unknown_party"), &form));
    };
    let Some(entity) = form.legal_entity_id else {
        return Err(invalid("legal_entity_id", t("unknown_entity"), &form));
    };
    let Some(amount) = filled(&form.amount).and_then(parse_amount) else {
        return Err(invalid("amount", t("bad_amount"), &form));
    };
    let Some(method) = filled(&form.method).and_then(method_of) else {
        return Err(invalid("method", t("fields.method"), &form));
    };
    let mut request = RecordPayment::new(party.id, entity, method, amount).collected_by(user.id);
    if let Some(date) = form.payment_date {
        request = request.date(date);
    }
    if let Some(reference) = filled(&form.reference) {
        request = request.reference(reference);
    }
    if let Some(account) = filled(&form.bank_account) {
        request = request.bank_account(account);
    }
    let mut details = renox::serde_json::Map::new();
    if let Some(number) = filled(&form.cheque_number) {
        details.insert("number".to_owned(), json!(number));
    }
    if let Some(bank) = filled(&form.cheque_bank) {
        details.insert("bank".to_owned(), json!(bank));
    }
    if let Some(date) = form.cheque_date {
        details.insert(
            "date".to_owned(),
            json!(date.format("%Y-%m-%d").to_string()),
        );
    }
    request = request.method_details(Value::Object(details));
    if filled(&form.purpose) == Some(PURPOSE) {
        return store_drawing(&state, &user, &t, &form, &request).await;
    }
    let payment = record(&state, Some(&user), &request)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t("recorded")),
        Redirect::route("wl_payments.show", &[&payment.id])?,
    )
        .into_response())
}

/// A drawing or a repayment from the record form (`FIN.ACL.21`): a drawing is
/// kept with its details and waits for its approval (nothing is recorded), a
/// repayment by the owner is recorded at once.
async fn store_drawing(
    state: &AppState,
    user: &AuthUser,
    t: Text<'_>,
    form: &PaymentForm,
    request: &RecordPayment,
) -> Result<Response> {
    let input = DrawingInput {
        legal_entity_id: request.legal_entity_id,
        category: filled(&form.drawing_category)
            .unwrap_or(crate::expense_category::OWNER_DRAWINGS)
            .to_owned(),
        party: request.party,
        method: request.method,
        method_details: request.method_details.clone(),
        amount: request.amount,
        currency: request.currency.clone(),
        date: request.date,
        reference: request.reference.clone(),
        bank_account: request.bank_account.clone(),
    };
    let refusal = |error: Error| match error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message) => {
            invalid("purpose", message, form)
        }
        other => other,
    };
    if filled(&form.drawing_direction) == Some("in") {
        let payment = drawings::repay(state, user, &input).await.map_err(refusal)?;
        return Ok((
            Toast::success(t("recorded")),
            Redirect::route("wl_payments.show", &[&payment.id])?,
        )
            .into_response());
    }
    let drawing = drawings::request(state, user, &input).await.map_err(refusal)?;
    let toast = match drawing.status {
        DrawingStatus::Recorded => t("drawings.recorded"),
        _ => t("drawings.requested"),
    };
    Ok((
        Toast::success(toast),
        Redirect::route("wl_payments.drawing", &[&drawing.id])?,
    )
        .into_response())
}

/// What the page of a drawing request needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/drawing.html")]
pub(crate) struct DrawingPage {
    tx: Texts,
    drawing: PaymentDrawing,
    title: String,
    status_label: String,
    status_kind: &'static str,
    party_name: String,
    entity_name: String,
    method_label: String,
    amount: String,
    /// The recorded payment's page, if any.
    payment_link: String,
}

/// The badge colour of a drawing request's status.
fn drawing_kind(status: DrawingStatus) -> &'static str {
    match status {
        DrawingStatus::Requested => "warning",
        DrawingStatus::Approved => "info",
        DrawingStatus::Recorded => "success",
        DrawingStatus::Rejected => "error",
    }
}

/// `GET /payments/drawings/{id}`: an owner drawing request and where it
/// stands. The approvers' "Needs your action" card links here.
async fn show_drawing(
    State(state): State<AppState>,
    lang: Lang,
    Found(drawing): Found<PaymentDrawing>,
) -> Result<View> {
    let t = texts_in(&lang);
    let entity = LegalEntity::find(&state.db, drawing.legal_entity_id).await?;
    Ok(View::typed(DrawingPage {
        tx: texts::all(&lang),
        title: format!("{} #{}", t("drawings.title"), drawing.id),
        status_label: t(&format!("drawings.status.{}", drawing.status.as_str())),
        status_kind: drawing_kind(drawing.status),
        party_name: party_name(&state, drawing.party()).await?,
        entity_name: entity.map(|e| e.code).unwrap_or_default(),
        method_label: t(&format!("method.{}", drawing.method.as_str())),
        amount: money_text(&drawing.currency, drawing.amount),
        payment_link: drawing
            .payment_id
            .map(|id| format!("/payments/{id}"))
            .unwrap_or_default(),
        drawing,
    }))
}

/// One allocation as the payment page shows it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct AllocationRow {
    id: i64,
    target: String,
    amount: String,
    allocated_at: DateTime,
    removed: bool,
    removed_at: Option<DateTime>,
    removed_reason: String,
}

/// One follow-up task.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct TaskRow {
    id: i64,
    text: String,
    party: String,
    amount: String,
    payment_id: Option<i64>,
    /// The failed refund a `change_method` task is about.
    refund_id: Option<i64>,
    /// The cheque a cheque task is about.
    cheque_id: Option<i64>,
    created_at: Option<DateTime>,
    open: bool,
    status: String,
}

/// One written-off invoice of the payment's party that can take money.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct WrittenOffRow {
    id: i64,
    number: String,
    /// The invoice page in the back office.
    link: String,
    /// What is left to recover, formatted.
    left: String,
    /// What the allocate form fills in: the payment's unallocated amount or
    /// what is left, whichever is smaller, as `525.00`.
    proposal: String,
    /// The single choice of the row's sheet, `(id, number)`.
    options: Vec<(String, String)>,
}

/// `fils` as `525.00`, the way the allocate form takes it.
fn plain_amount(fils: i64) -> String {
    format!("{}.{:02}", fils / 100, (fils % 100).abs())
}

/// The rows for the written-off `invoices`, each with the proposal for
/// `unallocated` fils. An invoice with nothing to propose is left out.
fn written_off_rows(
    currency: &str,
    invoices: &[InvoiceSummary],
    unallocated: i64,
) -> Vec<WrittenOffRow> {
    invoices
        .iter()
        .filter_map(|invoice| {
            let proposal = allocation_proposal(invoice, unallocated);
            if proposal <= 0 {
                return None;
            }
            let number = invoice
                .number
                .clone()
                .unwrap_or_else(|| format!("#{}", invoice.id.0));
            Some(WrittenOffRow {
                id: invoice.id.0,
                link: format!("/invoices/{}", invoice.id.0),
                left: money_text(currency, invoice.balance),
                proposal: plain_amount(proposal),
                options: vec![(invoice.id.0.to_string(), number.clone())],
                number,
            })
        })
        .collect()
}

/// What the payment page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/show.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    payment: Payment,
    title: String,
    status_label: String,
    status_kind: &'static str,
    party_name: String,
    entity_name: String,
    method_label: String,
    direction_label: String,
    source_label: String,
    reversal_reason_label: String,
    /// Amount, allocated and unallocated, formatted.
    amounts: BTreeMap<&'static str, String>,
    allocations: Vec<AllocationRow>,
    tasks: Vec<TaskRow>,
    timeline: Vec<Value>,
    actions: Vec<Available>,
    can_allocate: bool,
    can_reverse: bool,
    /// The reasons a user may give, `(code, label)`.
    reasons: Vec<(String, String)>,
    allocate_description: String,
    /// The party's written-off invoices, with the proposed amounts.
    written_off: Vec<WrittenOffRow>,
}

/// The display name of `party`, else `#id`.
pub(crate) async fn party_name(state: &AppState, party: PartyId) -> Result<String> {
    Ok(directory(state)
        .display_name(&state.db, party, "en")
        .await?
        .unwrap_or_else(|| format!("#{}", party.0)))
}

/// The task rows for `tasks`, with their parties' names.
async fn task_rows(state: &AppState, t: Text<'_>, tasks: &[FollowupTask]) -> Result<Vec<TaskRow>> {
    let mut rows = Vec::new();
    for task in tasks {
        rows.push(TaskRow {
            id: task.id,
            text: task.text.clone(),
            party: party_name(state, task.party()).await?,
            amount: money_text(&task.currency, task.amount),
            payment_id: task.payment_id,
            refund_id: task.refund_id,
            cheque_id: task.cheque_id,
            created_at: task.created_at,
            open: task.is_open(),
            status: t(if task.is_open() {
                "tasks.open"
            } else {
                "tasks.finished"
            }),
        });
    }
    Ok(rows)
}

/// `GET /payments/{id}`: the card, the allocations with the removed ones,
/// the allocate and reverse sheets, the follow-up tasks and the history.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(payment): Found<Payment>,
) -> Result<View> {
    let t = texts_in(&lang);
    let db = state.db.clone();
    let money = |fils: i64| money_text(&payment.currency, fils);
    let allocated = credit::allocated(&db, payment.id).await?;
    let unallocated = credit::unallocated(&db, &payment).await?;
    let amounts = BTreeMap::from([
        ("amount", money(payment.amount)),
        ("allocated", money(allocated)),
        ("unallocated", money(unallocated)),
    ]);
    let allocations = Allocation::all_of(&db, payment.id)
        .await?
        .into_iter()
        .map(|a| AllocationRow {
            id: a.id,
            target: a.target().to_string(),
            amount: money(a.amount),
            allocated_at: a.allocated_at,
            removed: !a.is_active(),
            removed_at: a.removed_at,
            removed_reason: a
                .removed_reason
                .as_deref()
                .map(|code| t(&format!("reason.{code}")))
                .unwrap_or_default(),
        })
        .collect();
    let tasks = task_rows(
        &state,
        &t,
        &FollowupTask::of_payment(&db, payment.id).await?,
    )
    .await?;
    let history = state_machine::history(&db, Payment::TABLE, payment.id).await?;
    let mut timeline =
        state_machine::timeline(&history, Some(&state_machine::info_of::<Payment>()));
    timeline.sort_by(|a, b| a["time"].as_str().cmp(&b["time"].as_str()));
    // `reverse` has its own sheet; `record` is not offered.
    let actions = payment
        .available(&user, &db)
        .await?
        .into_iter()
        .filter(|a| a.name != crate::reverse::REVERSE && a.name != crate::record::RECORD)
        .collect();
    let received = payment.status.is_received();
    let can_allocate = received
        && payment.direction == Direction::In
        && unallocated > 0
        && user.has_permission(permissions::RECORD);
    let can_reverse = received && user.has_permission(permissions::REVERSE);
    let reasons = options(&t, "reason", &["entered_in_error", "bank_return"]);
    let entity_name = match LegalEntity::find(&db, payment.legal_entity_id).await? {
        Some(entity) => entity.code,
        None => format!("#{}", payment.legal_entity_id),
    };
    let written_off = if can_allocate {
        let invoices = invoicer(&state)
            .written_off_of(&db, payment.party(), &payment.currency)
            .await?;
        written_off_rows(&payment.currency, &invoices, unallocated)
    } else {
        Vec::new()
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: payment.number.clone().unwrap_or_else(|| t("no_number")),
        status_label: t(&format!("status.{}", payment.status.as_str())),
        status_kind: status_kind(payment.status),
        party_name: party_name(&state, payment.party()).await?,
        entity_name,
        method_label: t(&format!("method.{}", payment.method.as_str())),
        direction_label: t(&format!("direction.{}", payment.direction.as_str())),
        source_label: t(&format!("source.{}", payment.source.as_str())),
        reversal_reason_label: payment
            .reversal_reason
            .map(|r| t(&format!("reason.{}", r.as_str())))
            .unwrap_or_default(),
        written_off,
        allocate_description: format!("{}: {}", t("allocate.description"), money(unallocated)),
        amounts,
        allocations,
        tasks,
        timeline,
        actions,
        can_allocate,
        can_reverse,
        reasons,
        payment,
    }))
}

/// The allocate sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct AllocateForm {
    /// The invoice's id.
    #[validate(required)]
    pub invoice_id: Option<i64>,
    /// The amount in dirhams, like `1500.00`.
    #[validate(required, max = 32)]
    pub amount: Option<String>,
}

/// `POST /payments/{id}/allocate`: allocates to the invoice and reloads the
/// page. The invoice's refusals answer 422.
async fn store_allocation(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(payment): Found<Payment>,
    Valid(form): Valid<AllocateForm>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let Some(amount) = filled(&form.amount).and_then(parse_amount) else {
        return Err(invalid("amount", t("bad_amount"), &form));
    };
    let invoice = form.invoice_id.unwrap_or_default();
    allocate(
        &state,
        payment.payment_id(),
        AllocationTarget::Invoice(invoice.into()),
        amount,
        Some(user.id),
    )
    .await?;
    Ok((Toast::success(t("allocate.done")), renox::HxRefresh).into_response())
}

/// The reverse sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct ReverseForm {
    /// The reason code: `entered_in_error` or `bank_return`.
    #[validate(max = 32)]
    pub reason: Option<String>,
    /// The note.
    #[validate(max = 2000)]
    pub note: Option<String>,
}

/// The reason with code `code`, if a user may give it.
fn reason_of(code: &str) -> Option<ReversalReason> {
    ReversalReason::ALL
        .iter()
        .copied()
        .find(|r| r.as_str() == code.trim() && r.is_manual())
}

/// `POST /payments/{id}/reverse`: reverses the payment and reloads the
/// page. A refusal (no reason, an open dispute) answers 422 under the
/// reason field; a second reverse is refused too.
async fn store_reverse(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(payment): Found<Payment>,
    Valid(form): Valid<ReverseForm>,
) -> Result<Response> {
    let reason = filled(&form.reason).and_then(reason_of);
    match reverse(
        &state,
        Some(&user),
        None,
        &payment,
        reason,
        form.note.as_deref(),
    )
    .await
    {
        Ok(_) => Ok((
            Toast::success(texts::t(&lang, "wl_payments.reverse.done")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(invalid("reason", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// What the tasks page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/tasks.html")]
pub(crate) struct TasksPage {
    tx: Texts,
    tasks: Vec<TaskRow>,
    can_mark: bool,
}

/// `GET /payments/tasks`: the open follow-up tasks of the accountant role and
/// those assigned to the user.
async fn tasks(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    // The accountants' tasks, and those assigned to the user (a case owner).
    let mut open = FollowupTask::open_for_user(&state.db, ACCOUNTANT_ROLE, user.id).await?;
    // A manager also sees the decisions left to the manager (`FIN.RUL.18`).
    if user.has_role(MANAGER_ROLE) {
        open.extend(FollowupTask::open_for(&state.db, MANAGER_ROLE).await?);
        open.sort_by_key(|task| task.id);
        open.dedup_by_key(|task| task.id);
    }
    Ok(View::typed(TasksPage {
        tx: texts::all(&lang),
        tasks: task_rows(&state, &texts_in(&lang), &open).await?,
        can_mark: user.has_permission(permissions::RECORD),
    }))
}

/// `POST /payments/tasks/{id}/done`: marks the task done.
async fn store_done(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    mark_done(&state.db, &user, id).await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_payments.tasks.done")),
        Redirect::route("wl_payments.tasks", &[])?,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::invoicing::InvoiceStatus;
    use wl_kernel::payments::PaymentSource;

    fn english_text(key: &str) -> String {
        texts::english(&format!("wl_payments.{key}")).to_owned()
    }

    fn written_off(id: i64, status: InvoiceStatus, balance: i64) -> InvoiceSummary {
        InvoiceSummary {
            id: wl_kernel::invoicing::InvoiceId(id),
            number: Some(format!("INV-{id}")),
            status,
            party: PartyId(1),
            legal_entity_id: Some(2),
            currency: "AED".into(),
            gross_total: 105_000,
            balance,
            due_date: None,
            disputed: false,
            chasing_paused_until: None,
        }
    }

    #[test]
    fn the_allocate_form_proposes_the_smaller_of_credit_and_what_is_left() {
        let invoices = [
            written_off(1, InvoiceStatus::WrittenOff, 105_000),
            written_off(2, InvoiceStatus::WrittenOff, 52_500),
            written_off(3, InvoiceStatus::Paid, 0),
        ];
        let rows = written_off_rows("AED", &invoices, 200_000);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].proposal, "1050.00");
        assert_eq!(rows[1].proposal, "525.00");
        assert_eq!(rows[1].number, "INV-2");
        assert_eq!(rows[1].options, [("2".to_owned(), "INV-2".to_owned())]);
        let small = written_off_rows("AED", &invoices, 10_005);
        assert_eq!(small[0].proposal, "100.05");
        assert!(written_off_rows("AED", &invoices, 0).is_empty());
    }

    #[test]
    fn every_new_text_is_defined() {
        for key in [
            "sections.written_off",
            "fields.invoice",
            "written_off.left",
            "written_off.proposal",
            "written_off.description",
        ] {
            assert_ne!(english_text(key), format!("wl_payments.{key}"));
        }
    }

    #[test]
    fn the_grid_lists_the_payment_columns_and_statuses() {
        let grid = grid_with(&english_text);
        let keys: Vec<&str> = grid.columns().iter().map(Column::key).collect();
        assert_eq!(
            keys,
            [
                "number",
                "payment_date",
                "party_name",
                "entity",
                "method",
                "amount",
                "unallocated",
                "status",
                "created_at"
            ]
        );
        let status = grid
            .columns()
            .iter()
            .find(|c| c.key() == "status")
            .expect("status column");
        assert_eq!(status.options().len(), PaymentStatus::ALL.len());
    }

    #[test]
    fn amounts_are_read_as_fils() {
        assert_eq!(parse_amount("1500"), Some(150_000));
        assert_eq!(parse_amount("1,500.5"), Some(150_050));
        assert_eq!(parse_amount(" 0.05 "), Some(5));
        assert_eq!(parse_amount(".5"), Some(50));
        assert_eq!(parse_amount("0"), None);
        assert_eq!(parse_amount("-3"), None);
        assert_eq!(parse_amount("1.234"), None);
        assert_eq!(parse_amount("abc"), None);
        assert_eq!(parse_amount(""), None);
        assert_eq!(parse_amount("."), None);
    }

    #[test]
    fn a_user_cannot_give_the_chargeback_reason() {
        assert_eq!(reason_of("bank_return"), Some(ReversalReason::BankReturn));
        assert_eq!(reason_of("chargeback"), None);
        assert_eq!(reason_of(""), None);
    }

    #[test]
    fn every_status_method_direction_and_reason_has_a_label() {
        let mut keys = Vec::new();
        keys.extend(
            PaymentStatus::ALL
                .iter()
                .map(|s| format!("status.{}", s.as_str())),
        );
        keys.extend(
            PaymentMethod::ALL
                .iter()
                .map(|s| format!("method.{}", s.as_str())),
        );
        keys.extend(
            Direction::ALL
                .iter()
                .map(|s| format!("direction.{}", s.as_str())),
        );
        keys.extend(
            PaymentSource::ALL
                .iter()
                .map(|s| format!("source.{}", s.as_str())),
        );
        keys.extend(
            ReversalReason::ALL
                .iter()
                .map(|s| format!("reason.{}", s.as_str())),
        );
        for key in keys {
            assert_ne!(english_text(&key), format!("wl_payments.{key}"), "{key}");
        }
    }
}
