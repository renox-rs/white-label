//! The payer bill pages (wl-invoicing §6):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /payer-bills` | `wl_invoicing.payer_bills` | `payer_bills.view` |
//! | `GET /payer-bills/{id}` | `wl_invoicing.payer_bill` | `payer_bills.view` |
//! | `POST /payer-bills/{id}/issue` | `wl_invoicing.payer_bill_issue` | `payer_bills.issue` |
//! | `POST /payer-bills/{id}/resubmit` | `wl_invoicing.payer_bill_resubmit` | `payer_bills.issue` |
//! | `POST /payer-bills/{id}/transitions/{name}` | `wl_invoicing.payer_bill_transitions.*` | `payer_bills.view` and the transition's permission |
//!
//! The list shows each payer bill's number, payer, customer, source, state
//! and open amount, newest first. A bill's page shows its terms, its lines
//! with what the payer paid and denied, the errors the submission channel
//! gave per attempt and the actions the user may run. `issue` and `resubmit`
//! have their own routes because their guards read the input the module
//! gathers; the kernel's transition route serves the others that need only a
//! reason (`void`) and announces an approval request a run created. The
//! templates are compiled in and can be replaced by the app
//! (`resources/views/wl-invoicing/payer_bills.html`, `payer_bill.html`).

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::Serialize;
use wl_kernel::payer_bills::PayerBillStatus;

use crate::allocate::amount_text;
use crate::credit_notes::By;
use crate::payer_agreement::PayerAgreement;
use crate::payer_bill::{PayerBill, lines_of};
use crate::payer_issue;
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
                .require_permission(permissions::PAYER_BILLS_ISSUE),
        )
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

/// One row of the list.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Row {
    id: i64,
    number: String,
    payer: String,
    customer: String,
    source: String,
    status: String,
    claimed: String,
    open: String,
    link: String,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/payer_bills.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: Vec<Row>,
}

/// `GET /payer-bills`: every payer bill, newest first.
async fn index(State(state): State<AppState>, lang: Lang) -> Result<View> {
    let all = PayerBill::query()
        .order_by_desc(PayerBill::ID)
        .limit(200)
        .get(&state.db)
        .await?;
    let mut rows = Vec::with_capacity(all.len());
    for bill in all {
        rows.push(Row {
            id: bill.id,
            number: number_text(&lang, &bill),
            payer: party_name(&state, bill.payer()).await?,
            customer: party_name(&state, bill.customer()).await?,
            source: bill.source().to_string(),
            status: text(
                &lang,
                &format!("payer_bill.status.{}", bill.status.as_str()),
            ),
            claimed: amount_text(&bill.currency, bill.claimed_total),
            open: amount_text(&bill.currency, bill.balance),
            link: format!("/payer-bills/{}", bill.id),
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
    }))
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
    can_issue: bool,
    can_resubmit: bool,
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
    let can_issue = user.has_permission(permissions::PAYER_BILLS_ISSUE);
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
        source: bill.source().to_string(),
        reference: bill.payer_reference.clone().unwrap_or_default(),
        service_date: day(Some(bill.service_date)),
        claimed: amount_text(&bill.currency, bill.claimed_total),
        paid: amount_text(&bill.currency, bill.paid_total),
        denied: amount_text(&bill.currency, bill.denied_total),
        open: amount_text(&bill.currency, bill.balance),
        lines,
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

/// The answer of an action: the success toast, or the guard's message as an
/// error toast; any other refusal goes back as its status.
fn answered(
    lang: &Lang,
    bill: &PayerBill,
    done: std::result::Result<PayerBill, TransitionError>,
    key: &str,
) -> Result<Response> {
    let toast = match done {
        Ok(_) => Toast::success(text(lang, key)),
        Err(TransitionError::Guard(message)) => Toast::error(message),
        Err(other) => return Err(other.into_error()),
    };
    Ok((toast, back(bill)?).into_response())
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
