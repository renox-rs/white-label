//! *Pay supplier* (BL-PAY-28, [`crate::supplier_pay`]):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /payments/suppliers/{party}/pay` | `wl_payments.supplier_pay` | `payments.record` (`?amount=` proposes the amounts, oldest bill first) |
//! | `POST /payments/suppliers/{party}/pay` | `wl_payments.supplier_pay.store` | `payments.record` (`method`, `legal_entity_id`, `bank_account`, `date`, `reference`, `cheque_bank`, `bill_<id>` per bill; `exception=1` asks for an exception) |
//!
//! The page lists the supplier's approved bills with their open balance and an
//! amount per bill. A blocked supplier's payment is refused with the on-hold
//! text (422 under `bills`); the same form then offers *Request exception*.

use std::collections::{BTreeMap, HashMap};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::invoicing::SupplierBillId;
use wl_kernel::payments::PaymentMethod;

use crate::followup::money_text;
use crate::pages::{invalid, parse_amount, texts_in};
use crate::permissions;
use crate::supplier_pay::{
    self, ExceptionOutcome, SupplierPayOutcome, SupplierPayment, payable_bills, propose,
};
use crate::texts;

/// The routes, behind `payments.record`.
pub fn routes() -> Routes {
    Routes::new()
        .get("/payments/suppliers/{party}/pay", show)
        .name("wl_payments.supplier_pay")
        .post("/payments/suppliers/{party}/pay", store)
        .name("wl_payments.supplier_pay.store")
        .require_permission(permissions::RECORD)
}

/// One bill of the page.
#[derive(Serialize, Debug, Clone)]
struct BillRow {
    id: i64,
    number: String,
    balance: String,
    field: String,
    proposed: String,
    entity: i64,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-payments/supplier_pay.html")]
pub(crate) struct PayPage {
    tx: BTreeMap<&'static str, String>,
    supplier_name: String,
    bill_rows: Vec<BillRow>,
    action: String,
    amount: String,
    entity: String,
    methods: Vec<(String, String)>,
}

/// The query of the page.
#[derive(Deserialize, Debug, Default)]
pub struct PayQuery {
    /// The amount to spread over the bills, in dirhams.
    #[serde(default)]
    pub amount: Option<String>,
}

/// `GET /payments/suppliers/{party}/pay`.
async fn show(
    State(state): State<AppState>,
    lang: Lang,
    Path(party): Path<i64>,
    Query(query): Query<PayQuery>,
) -> Result<View> {
    let t = texts_in(&lang);
    let bills = payable_bills(&state, PartyId(party)).await?;
    let amount = query.amount.as_deref().and_then(parse_amount);
    let proposed = amount.map(|a| propose(&bills, a)).unwrap_or_default();
    let bill_rows = bills
        .iter()
        .map(|bill| BillRow {
            id: bill.id.0,
            number: bill.number.clone(),
            balance: money_text(&bill.currency, bill.balance),
            field: format!("bill_{}", bill.id.0),
            proposed: proposed
                .iter()
                .find(|(id, _)| *id == bill.id)
                .map(|(_, fils)| format!("{}.{:02}", fils / 100, fils % 100))
                .unwrap_or_default(),
            entity: bill.legal_entity_id.unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let supplier_name = wl_kernel::parties::directory(&state)
        .display_name(&state.db, PartyId(party), &lang.locale)
        .await?
        .unwrap_or_else(|| format!("#{party}"));
    let entity = bill_rows
        .first()
        .map(|b| b.entity.to_string())
        .unwrap_or_default();
    Ok(View::typed(PayPage {
        tx: texts::all(&lang),
        supplier_name,
        bill_rows,
        action: format!("/payments/suppliers/{party}/pay"),
        amount: query.amount.unwrap_or_default(),
        entity,
        methods: vec![
            ("transfer".to_owned(), t("supplier_pay.method.transfer")),
            ("cheque".to_owned(), t("supplier_pay.method.cheque")),
        ],
    }))
}

/// The request a submitted form describes.
fn request_of(
    party: i64,
    form: &HashMap<String, String>,
) -> std::result::Result<SupplierPayment, String> {
    let get = |key: &str| form.get(key).map(|v| v.trim()).filter(|v| !v.is_empty());
    let method = match get("method").unwrap_or("transfer") {
        "cheque" => PaymentMethod::Cheque,
        _ => PaymentMethod::Transfer,
    };
    let legal_entity_id = get("legal_entity_id")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| "Choose the entity.".to_owned())?;
    let date = match get("date") {
        Some(text) => Some(
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .map_err(|_| "Enter the date as YYYY-MM-DD.".to_owned())?,
        ),
        None => None,
    };
    let mut bills = Vec::new();
    let mut keys: Vec<&String> = form.keys().collect();
    keys.sort();
    for key in keys {
        let Some(id) = key
            .strip_prefix("bill_")
            .and_then(|id| id.parse::<i64>().ok())
        else {
            continue;
        };
        let Some(text) = get(key) else {
            continue;
        };
        let amount = parse_amount(text)
            .ok_or_else(|| format!("The amount for bill {id} is not a number above 0."))?;
        bills.push((SupplierBillId(id), amount));
    }
    Ok(SupplierPayment {
        supplier: PartyId(party),
        legal_entity_id,
        method,
        bank_account: get("bank_account").map(str::to_owned),
        date,
        reference: get("reference").map(str::to_owned),
        cheque_bank: get("cheque_bank").map(str::to_owned),
        bills,
    })
}

/// `POST /payments/suppliers/{party}/pay`: pays, or asks for an exception.
async fn store(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(party): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response> {
    let t = texts_in(&lang);
    let request = request_of(party, &form).map_err(|message| invalid("bills", message, &form))?;
    let refused = |error: TransitionError| match error {
        TransitionError::Guard(message) => invalid("bills", message, &form),
        other => other.into_error(),
    };
    if form.get("exception").is_some_and(|v| v == "1") {
        let outcome = supplier_pay::request_exception(&state, &user, &request)
            .await
            .map_err(refused)?;
        let done = match outcome {
            ExceptionOutcome::Requested(_) => t("supplier_pay.exception_requested"),
            ExceptionOutcome::Paid(_) => t("supplier_pay.paid"),
        };
        return Ok((Toast::success(done), renox::HxRefresh).into_response());
    }
    match supplier_pay::pay(&state, &user, &request)
        .await
        .map_err(refused)?
    {
        SupplierPayOutcome::Paid(_) => {
            Ok((Toast::success(t("supplier_pay.paid")), renox::HxRefresh).into_response())
        }
        SupplierPayOutcome::ChequeIssued(_) => Ok((
            Toast::success(t("supplier_pay.cheque_issued")),
            renox::HxRefresh,
        )
            .into_response()),
        SupplierPayOutcome::OnHold(text) => Err(invalid(
            "bills",
            format!("{text}. {}", t("supplier_pay.exception_hint")),
            &form,
        )),
    }
}
