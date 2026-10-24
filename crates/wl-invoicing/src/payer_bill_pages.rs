//! The payer bill pages (wl-invoicing §6):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /payer-bills` | `wl_invoicing.payer_bills` | `payer_bills.view` |
//! | `GET /payer-bills/{id}` | `wl_invoicing.payer_bill` | `payer_bills.view` |
//! | `POST /payer-bills/{id}/issue` | `wl_invoicing.payer_bill_issue` | `payer_bills.issue` |
//! | `POST /payer-bills/{id}/resubmit` | `wl_invoicing.payer_bill_resubmit` | `payer_bills.issue` |
//! | `POST /payer-bills/{id}/submission-rejected` | `wl_invoicing.payer_bill_rejected` | `payer_bills.issue` |
//! | `POST /payer-bills/{id}/remittance` | `wl_invoicing.payer_bill_remittance` | `payer_bills.reconcile` |
//! | `POST /payer-bills/{id}/recoupment` | `wl_invoicing.payer_bill_recoupment` | `payer_bills.reconcile` |
//! | `POST /payer-bills/{id}/write-off` | `wl_invoicing.payer_bill_write_off` | `payer_bills.write_off` |
//! | `POST /payer-bills/{id}/transfer` | `wl_invoicing.payer_bill_transfer` | `payer_bills.write_off` |
//! | `POST /payer-bills/{id}/transitions/{name}` | `wl_invoicing.payer_bill_transitions.*` | `payer_bills.view` and the transition's permission |
//!
//! The list is a `renox::grid` (`?export=` also `payer_bills.issue`) with each
//! payer bill's number, payer, customer, source, state and open amount, newest first. A bill's page shows its terms, its lines
//! with what the payer paid and denied, the errors the submission channel
//! gave per attempt and the actions the user may run. `issue` and `resubmit`
//! have their own routes because their guards read the input the module
//! gathers; the kernel's transition route serves the others that need only a
//! reason (`void`) and announces an approval request a run created. The
//! templates are compiled in and can be replaced by the app
//! (`resources/views/wl-invoicing/payer_bills.html`, `payer_bill.html`).

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::TransitionError;
use serde::Serialize;
use wl_kernel::approvals::{self, ApprovalCard};
use wl_kernel::payer_bills::{
    PayerAnswer, PayerBillStatus, Recoupment, Remittance, RemittanceLine,
};

use crate::allocate::amount_text;
use crate::bill_pages::{parse_date, parse_decimal};
use crate::credit_notes::By;
use crate::payer_agreement::PayerAgreement;
use crate::payer_bill::{PayerBill, lines_of};
use crate::payer_closing;
use crate::payer_credit;
use crate::payer_issue;
use crate::payer_recoupment::{self, PayerRecoupment};
use crate::payer_rejection::{self, PayerSubmissionError};
use crate::payer_remittance::{self, PayerRemittance};
use crate::permissions;
use crate::texts;

/// The texts a template reads as `tx['payer_bill.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The package text `key` (without the `wl_invoicing.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_invoicing.{key}"))
}

/// The module's payer bill routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/payer-bills", index)
                .name("wl_invoicing.payer_bills")
                .get("/payer-bills/{id}", show)
                .name("wl_invoicing.payer_bill")
                .get("/payer-agreements/options", agreement_options)
                .name("wl_invoicing.payer_agreement_options")
                .group(
                    "/payer-bills",
                    "wl_invoicing.payer_bill_transitions.",
                    wl_kernel::machines::routes::<PayerBill>(),
                )
                .require_permission(permissions::PAYER_BILLS_VIEW),
        )
        .merge(
            Routes::new()
                .post("/payer-bills/{id}/issue", issue)
                .name("wl_invoicing.payer_bill_issue")
                .post("/payer-bills/{id}/resubmit", resubmit)
                .name("wl_invoicing.payer_bill_resubmit")
                .post("/payer-bills/{id}/submission-rejected", rejected)
                .name("wl_invoicing.payer_bill_rejected")
                .require_permission(permissions::PAYER_BILLS_ISSUE),
        )
        .merge(
            Routes::new()
                .post("/payer-bills/{id}/write-off", write_off)
                .name("wl_invoicing.payer_bill_write_off")
                .post("/payer-bills/{id}/transfer", transfer)
                .name("wl_invoicing.payer_bill_transfer")
                .require_permission(permissions::PAYER_BILLS_WRITE_OFF),
        )
        .merge(
            Routes::new()
                .post("/payer-bills/{id}/remittance", remittance)
                .name("wl_invoicing.payer_bill_remittance")
                .post("/payer-bills/{id}/recoupment", recoupment)
                .name("wl_invoicing.payer_bill_recoupment")
                .require_permission(permissions::PAYER_BILLS_RECONCILE),
        )
}

/// `GET /payer-agreements/options`: the payer agreements a picker offers as
/// the person types (`?q=`, matched against the agreement's name and the
/// payer's), or the labels of the values it already holds (`?values=`), as
/// `name · payer`. Only active agreements are offered for a search; a held
/// one keeps its label whatever its state. For those who see payer bills.
async fn agreement_options(
    State(state): State<AppState>,
    query: renox::select::OptionQuery,
) -> Result<Json<Vec<renox::select::SelectOption>>> {
    let directory = wl_kernel::parties::directory(&state);
    let mut agreements = Vec::new();
    if query.is_lookup() {
        for id in query.values_as::<i64>() {
            agreements.extend(PayerAgreement::find(&state.db, id).await?);
        }
    } else {
        let needle = query.q.trim().to_lowercase();
        for agreement in PayerAgreement::query()
            .where_(PayerAgreement::ACTIVE.eq(true))
            .order_by(PayerAgreement::NAME)
            .get(&state.db)
            .await?
        {
            let payer = directory
                .display_name(&state.db, agreement.payer(), "en")
                .await?
                .unwrap_or_default();
            if needle.is_empty()
                || agreement.name.to_lowercase().contains(&needle)
                || payer.to_lowercase().contains(&needle)
            {
                agreements.push(agreement);
            }
            if agreements.len() >= 20 {
                break;
            }
        }
    }
    let mut options = Vec::new();
    for agreement in agreements {
        let payer = directory
            .display_name(&state.db, agreement.payer(), "en")
            .await?
            .unwrap_or_default();
        options.push(renox::select::SelectOption::new(
            agreement.id,
            format!("{} · {payer}", agreement.name),
        ));
    }
    Ok(Json(options))
}

/// The badge colour of a state (the kit has info, success, warning and
/// error).
fn status_kind(status: PayerBillStatus) -> &'static str {
    match status {
        PayerBillStatus::Draft
        | PayerBillStatus::Issued
        | PayerBillStatus::Closed
        | PayerBillStatus::Voided => "info",
        PayerBillStatus::PartiallyPaid => "warning",
        PayerBillStatus::Denied => "error",
        PayerBillStatus::Paid => "success",
    }
}

/// `14 Mar 2027`, or an empty text.
fn day(date: Option<NaiveDate>) -> String {
    date.map(|d| d.format("%d %b %Y").to_string())
        .unwrap_or_default()
}

/// The party's display name, else its id.
async fn party_name(state: &AppState, party: wl_kernel::PartyId) -> Result<String> {
    Ok(wl_kernel::parties::directory(state)
        .find(&state.db, party)
        .await?
        .map(|p| p.display_name)
        .unwrap_or_else(|| format!("#{}", party.0)))
}

/// The number of a bill, or the draft text while it has none.
fn number_text(lang: &Lang, bill: &PayerBill) -> String {
    bill.number
        .clone()
        .unwrap_or_else(|| text(lang, "payer_bill.draft_number"))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/payer_bills.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: GridPage<PayerBill>,
}

/// The payer bills grid: search on the number, filters by state, sums of the
/// claimed and open amounts, in each bill's currency. The payer, customer
/// and source record are drawn by the template (`custom`).
pub fn grid(lang: &Lang, can_export: bool) -> Grid {
    let statuses: Vec<(String, String)> = PayerBillStatus::ALL
        .iter()
        .map(|s| {
            (
                s.as_str().to_owned(),
                text(lang, &format!("payer_bill.status.{}", s.as_str())),
            )
        })
        .collect();
    // A draft has no number yet: it shows the draft text.
    let draft = text(lang, "payer_bill.draft_number").replace('\'', "''");
    let grid = Grid::new("wl-invoicing-payer-bills")
        .title(&text(lang, "payer_bill.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &text(lang, "payer_bill.fields.number"))
                .computed(&format!("COALESCE({{T}}.\"number\", '{draft}')"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/payer-bills/{id}"),
        )
        // The names come from the party directory, which this crate reads
        // through the kernel and not from a table: the template draws them.
        .column(Column::custom("payer", &text(lang, "payer_bill.fields.payer")).mobile())
        .column(Column::custom(
            "customer",
            &text(lang, "payer_bill.fields.customer"),
        ))
        .column(Column::custom(
            "source",
            &text(lang, "payer_bill.fields.source"),
        ))
        .column(
            Column::select("status", &text(lang, "payer_bill.fields.status"), statuses)
                .badges(&[
                    ("draft", "info"),
                    ("issued", "info"),
                    ("partially_paid", "warning"),
                    ("denied", "danger"),
                    ("paid", "success"),
                    ("closed", "info"),
                    ("voided", "info"),
                ])
                .mobile(),
        )
        .column(
            Column::money("claimed_total", &text(lang, "payer_bill.fields.claimed"))
                .currency_from("currency")
                .summary(Summary::Sum),
        )
        .column(
            Column::money("balance", &text(lang, "payer_bill.fields.open"))
                .currency_from("currency")
                .summary(Summary::Sum)
                .mobile(),
        )
        .column(
            Column::money("paid_total", &text(lang, "payer_bill.fields.paid"))
                .currency_from("currency")
                .hidden()
                .summary(Summary::Sum),
        )
        .column(
            Column::money("denied_total", &text(lang, "payer_bill.fields.denied"))
                .currency_from("currency")
                .hidden()
                .summary(Summary::Sum),
        )
        .column(
            Column::date(
                "service_date",
                &text(lang, "payer_bill.fields.service_date"),
            )
            .hidden(),
        )
        .column(Column::datetime("created_at", &text(lang, "fields.created_at")).hidden())
        .sort_by("-id")
        .row_url("/payer-bills/{id}")
        .cards_on_mobile()
        .empty_state(&text(lang, "payer_bill.empty"), None);
    if can_export { grid.exports() } else { grid }
}

/// The record a bill comes from, named for people (`VIS-0009`) when its module
/// registered a label for the type, else `type:id`.
async fn source_label(state: &AppState, bill: &PayerBill) -> Result<String> {
    let source = bill.source();
    Ok(wl_kernel::documents::subject_label(&state.db, &source)
        .await?
        .unwrap_or_else(|| source.to_string()))
}

/// `GET /payer-bills`: the grid, or its export (`?export=csv|xlsx|print`,
/// with `payer_bills.issue`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let can_export = crate::may_export(&user, permissions::PAYER_BILLS_ISSUE);
    let grid = grid(&lang, can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(PayerBill::query(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(PayerBill::query(), &request).await?;
    // The source record of each bill on the page, named for people.
    let mut names = BTreeMap::new();
    for bill in page.items() {
        names.insert(
            bill.id,
            (
                party_name(&state, bill.payer()).await?,
                party_name(&state, bill.customer()).await?,
                source_label(&state, bill).await?,
            ),
        );
    }
    let rows = page.extend(|bill| {
        let (payer, customer, source) = names.get(&bill.id).cloned().unwrap_or_default();
        json!({ "payer": payer, "customer": customer, "source": source })
    });
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
    })
    .into_response())
}

/// One line of a bill.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct LineRow {
    id: i64,
    position: i64,
    description: String,
    claimed: String,
    paid: String,
    denied: String,
    open: String,
    code: String,
}

/// One error the channel gave.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ErrorRow {
    id: i64,
    attempt: i64,
    code: String,
    message: String,
    date: String,
}

/// One remittance recorded on the bill.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct RemittanceRow {
    id: i64,
    reference: String,
    date: String,
    paid: String,
    denied: String,
    overpayment: String,
}

/// One recoupment recorded on the bill.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct RecoupmentRow {
    id: i64,
    code: String,
    amount: String,
    from_credit: String,
    deducted: String,
    carrier: String,
    reversal: bool,
}

/// One movement of the payer credit balance, as the page lists it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct CreditRow {
    id: i64,
    kind: String,
    amount: String,
    /// The wl-payments refund of a refund movement.
    refund: String,
    /// Whose approval of the refund is the movement's.
    approved_by: String,
}

/// What a payer bill's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/payer_bill.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    title: String,
    bill: PayerBill,
    status_label: String,
    status_kind: &'static str,
    payer: String,
    customer: String,
    agreement: String,
    recipient: String,
    source: String,
    reference: String,
    service_date: String,
    claimed: String,
    paid: String,
    denied: String,
    open: String,
    lines: Vec<LineRow>,
    errors: Vec<ErrorRow>,
    remittances: Vec<RemittanceRow>,
    recoupments: Vec<RecoupmentRow>,
    credit: String,
    credit_movements: Vec<CreditRow>,
    /// The approval cards of the requests about the bill (a remittance above
    /// what is open, a write-off, a void): where the second person decides.
    approvals: Vec<ApprovalCard>,
    can_issue: bool,
    can_resubmit: bool,
    can_reject: bool,
    can_remit: bool,
    can_recoup: bool,
    can_close: bool,
    can_void: bool,
    void_action: String,
}

/// `GET /payer-bills/{id}`: the bill's terms, lines and actions.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(bill): Found<PayerBill>,
) -> Result<View> {
    let agreement = PayerAgreement::find(&state.db, bill.agreement_id).await?;
    let lines = lines_of(&state.db, bill.id)
        .await?
        .into_iter()
        .map(|line| LineRow {
            id: line.id,
            position: line.position,
            description: line.description.clone(),
            claimed: amount_text(&bill.currency, line.claimed),
            paid: amount_text(&bill.currency, line.paid),
            denied: amount_text(&bill.currency, line.denied),
            open: amount_text(&bill.currency, line.open()),
            code: line
                .recoupment_code
                .clone()
                .or(line.denial_code.clone())
                .unwrap_or_default(),
        })
        .collect();
    let errors = PayerSubmissionError::of_bill(&state.db, bill.id)
        .await?
        .into_iter()
        .map(|error| ErrorRow {
            id: error.id,
            attempt: error.attempt,
            code: error.code,
            message: error.message,
            date: day(Some(error.recorded_on)),
        })
        .collect();
    let remittances = PayerRemittance::query()
        .where_(PayerRemittance::PAYER_BILL_ID.eq(bill.id))
        .order_by(PayerRemittance::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .map(|row| RemittanceRow {
            id: row.id,
            reference: row.reference,
            date: day(Some(row.advice_date)),
            paid: amount_text(&bill.currency, row.paid_total),
            denied: amount_text(&bill.currency, row.denied_total),
            overpayment: amount_text(&bill.currency, row.overpayment),
        })
        .collect();
    let recoupments = PayerRecoupment::query()
        .where_(PayerRecoupment::PAYER_BILL_ID.eq(bill.id))
        .order_by(PayerRecoupment::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .map(|row| RecoupmentRow {
            id: row.id,
            code: row.code,
            amount: amount_text(&bill.currency, row.amount),
            from_credit: amount_text(&bill.currency, row.from_credit),
            deducted: amount_text(&bill.currency, row.deducted),
            carrier: row.carrier_reference.unwrap_or_default(),
            reversal: row.reversal,
        })
        .collect();
    let credit = payer_credit::balance(
        &state.db,
        bill.payer(),
        bill.legal_entity_id,
        &bill.currency,
    )
    .await?;
    let mut credit_movements = Vec::new();
    for movement in payer_credit::PayerCreditMovement::query()
        .where_(payer_credit::PayerCreditMovement::PAYER_PARTY_ID.eq(bill.payer_party_id))
        .where_(payer_credit::PayerCreditMovement::LEGAL_ENTITY_ID.eq(bill.legal_entity_id))
        .where_(payer_credit::PayerCreditMovement::CURRENCY.eq(bill.currency.as_str()))
        .order_by_desc(payer_credit::PayerCreditMovement::ID)
        .limit(20)
        .get(&state.db)
        .await?
    {
        let approved_by = match movement.approved_by {
            Some(id) => User::find(&state.db, id)
                .await?
                .map(|u| u.name)
                .unwrap_or_default(),
            None => String::new(),
        };
        credit_movements.push(CreditRow {
            id: movement.id,
            kind: text(&lang, &format!("payer_bill.credit.kind.{}", movement.kind)),
            amount: amount_text(&bill.currency, movement.amount),
            refund: movement
                .refund_id
                .map(|id| format!("/refunds/{id}"))
                .unwrap_or_default(),
            approved_by,
        });
    }
    let can_reconcile = user.has_permission(permissions::PAYER_BILLS_RECONCILE);
    let can_issue = user.has_permission(permissions::PAYER_BILLS_ISSUE);
    let approval_cards = approvals::cards_for(&state.db, &bill.subject(), &user, &lang).await?;
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: number_text(&lang, &bill),
        status_label: text(
            &lang,
            &format!("payer_bill.status.{}", bill.status.as_str()),
        ),
        status_kind: status_kind(bill.status),
        payer: party_name(&state, bill.payer()).await?,
        customer: party_name(&state, bill.customer()).await?,
        agreement: agreement.map(|a| a.name).unwrap_or_default(),
        recipient: text(
            &lang,
            &format!("payer_bill.recipient.{}", bill.vat_recipient.as_str()),
        ),
        source: source_label(&state, &bill).await?,
        reference: bill.payer_reference.clone().unwrap_or_default(),
        service_date: day(Some(bill.service_date)),
        claimed: amount_text(&bill.currency, bill.claimed_total),
        paid: amount_text(&bill.currency, bill.paid_total),
        denied: amount_text(&bill.currency, bill.denied_total),
        open: amount_text(&bill.currency, bill.balance),
        lines,
        errors,
        remittances,
        recoupments,
        credit: amount_text(&bill.currency, credit),
        credit_movements,
        approvals: approval_cards,
        can_close: user.has_permission(permissions::PAYER_BILLS_WRITE_OFF)
            && matches!(
                bill.status,
                PayerBillStatus::Draft | PayerBillStatus::PartiallyPaid | PayerBillStatus::Denied
            )
            && bill.balance > 0,
        can_void: can_issue && bill.status == PayerBillStatus::Draft && !bill.has_remittance,
        void_action: format!("/payer-bills/{}/transitions/void", bill.id),
        can_remit: can_reconcile && bill.status == PayerBillStatus::Issued,
        can_recoup: can_reconcile
            && matches!(
                bill.status,
                PayerBillStatus::Paid | PayerBillStatus::PartiallyPaid | PayerBillStatus::Closed
            ),
        can_reject: can_issue && bill.status == PayerBillStatus::Issued,
        can_issue: can_issue && bill.status == PayerBillStatus::Draft,
        can_resubmit: can_issue
            && matches!(
                bill.status,
                PayerBillStatus::PartiallyPaid | PayerBillStatus::Denied
            ),
        bill,
    }))
}

/// Where an action goes back to: the bill's page.
fn back(bill: &PayerBill) -> Result<Redirect> {
    Redirect::route("wl_invoicing.payer_bill", &[&bill.id])
}

/// The answer of an action: the success toast and the bill's page, or the
/// refusal as its status (a guard's message is a 422 the page shows).
fn answered(
    lang: &Lang,
    bill: &PayerBill,
    done: std::result::Result<PayerBill, TransitionError>,
    key: &str,
) -> Result<Response> {
    done.map_err(TransitionError::into_error)?;
    Ok((Toast::success(text(lang, key)), back(bill)?).into_response())
}

/// `POST /payer-bills/{id}/issue`: issues the draft.
async fn issue(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(bill): Found<PayerBill>,
) -> Result<Response> {
    let done = payer_issue::issue(&state, By::User(&user), bill.id).await;
    answered(&lang, &bill, done, "payer_bill.issue.done")
}

/// `POST /payer-bills/{id}/resubmit`: sends the bill again with the written
/// `corrections`.
async fn resubmit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(bill): Found<PayerBill>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let corrections = form.get("corrections").map_or("", |c| c.trim());
    let done = payer_issue::resubmit(&state, By::User(&user), bill.id, corrections).await;
    answered(&lang, &bill, done, "payer_bill.resubmit.done")
}

/// `POST /payer-bills/{id}/submission-rejected`: records the channel's
/// `errors` (one `CODE: message` per line) and sends the bill back to draft.
async fn rejected(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(bill): Found<PayerBill>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let errors = payer_rejection::parse_errors(form.get("errors").map_or("", String::as_str));
    let done = payer_rejection::submission_rejected(&state, By::User(&user), bill.id, errors)
        .await
        .map(|(rejected, _)| rejected);
    answered(&lang, &bill, done, "payer_bill.reject.done")
}

/// The answer of an action that may wait for a second person: the toast of
/// what happened and the bill's page, or the refusal as its status.
fn answered_money(
    lang: &Lang,
    bill: &PayerBill,
    done: std::result::Result<(PayerBill, PayerAnswer), TransitionError>,
    key: &str,
) -> Result<Response> {
    let (_, answer) = done.map_err(TransitionError::into_error)?;
    let toast = match answer {
        PayerAnswer::Done => Toast::success(text(lang, key)),
        PayerAnswer::Pending { .. } => Toast::info(text(lang, "payer_bill.waits_for_approval")),
    };
    Ok((toast, back(bill)?).into_response())
}

/// A 422 with the text of `key`.
fn bad(lang: &Lang, key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, text(lang, key))
}

/// An amount typed as `400.00`; blank is 0.
fn fils_of(lang: &Lang, typed: &str) -> Result<i64> {
    if typed.trim().is_empty() {
        return Ok(0);
    }
    parse_decimal(typed, 2).ok_or_else(|| bad(lang, "payer_bill.errors.amount_format"))
}

/// The day typed as `2026-12-10`; blank is today.
fn date_of(lang: &Lang, typed: &str) -> Result<NaiveDate> {
    if typed.trim().is_empty() {
        return Ok(wl_kernel::documents::today());
    }
    parse_date(typed).ok_or_else(|| bad(lang, "payer_bill.errors.date_format"))
}

/// `POST /payer-bills/{id}/remittance`: records the payer's advice. The form
/// has the `reference`, the `date`, the wl-payments `payment` if any and, per
/// line, `line_<id>_paid`, `line_<id>_denied`, `line_<id>_code` and
/// `line_<id>_reason`; a line with nothing paid or denied is left out.
async fn remittance(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(bill): Found<PayerBill>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let mut advice = Remittance::new(get("reference"), date_of(&lang, get("date"))?);
    if !get("payment").is_empty() {
        let payment = get("payment")
            .parse::<i64>()
            .map_err(|_| bad(&lang, "payer_bill.errors.payment_format"))?;
        advice = advice.payment(payment);
    }
    for line in lines_of(&state.db, bill.id).await? {
        let paid = fils_of(&lang, get(&format!("line_{}_paid", line.id)))?;
        let denied = fils_of(&lang, get(&format!("line_{}_denied", line.id)))?;
        if paid == 0 && denied == 0 {
            continue;
        }
        advice = advice.line(RemittanceLine::paid(line.id, paid).denied(
            denied,
            get(&format!("line_{}_code", line.id)),
            get(&format!("line_{}_reason", line.id)),
        ));
    }
    let done = payer_remittance::record(&state, &user, bill.id, advice).await;
    answered_money(&lang, &bill, done, "payer_bill.remit.done")
}

/// `POST /payer-bills/{id}/recoupment`: records what the payer took back.
/// The form has the `code`, the `reason`, the `carrier_reference` and
/// `carrier_date` of the later remittance that carried it and, per line,
/// `line_<id>_recouped`.
async fn recoupment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(bill): Found<PayerBill>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let mut taken = Recoupment::new(get("code")).reason(get("reason"));
    if !get("carrier_reference").is_empty() {
        taken = taken.carried_by(
            get("carrier_reference"),
            date_of(&lang, get("carrier_date"))?,
        );
    }
    for line in lines_of(&state.db, bill.id).await? {
        let amount = fils_of(&lang, get(&format!("line_{}_recouped", line.id)))?;
        if amount != 0 {
            taken = taken.line(line.id, amount);
        }
    }
    let done = payer_recoupment::record(&state, &user, bill.id, taken).await;
    answered_money(&lang, &bill, done, "payer_bill.recoup.done")
}

/// `POST /payer-bills/{id}/write-off`: writes the open amount off with the
/// `reason`; above the agreement's limit it waits for a second person.
async fn write_off(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(bill): Found<PayerBill>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let reason = form.get("reason").map_or("", |r| r.trim());
    let done = payer_closing::write_off(&state, &user, bill.id, reason).await;
    answered_money(&lang, &bill, done, "payer_bill.write_off.done")
}

/// `POST /payer-bills/{id}/transfer`: moves the open amount to the customer
/// with the `reason`, where the payer agreement allows it.
async fn transfer(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(bill): Found<PayerBill>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let reason = form.get("reason").map_or("", |r| r.trim());
    let done = payer_closing::transfer_to_customer(&state, &user, bill.id, reason).await;
    answered(&lang, &bill, done, "payer_bill.transfer.done")
}
