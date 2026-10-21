//! The payer contract pages (05 `CLN.INS.01`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /clinics/contracts` | `clinics.contracts` | `insurance_policies.view` |
//! | `POST /clinics/contracts` | `clinics.contract_create` | `insurance.manage` |
//! | `POST /clinics/contract-tariffs` | `clinics.contract_tariff` | `insurance.manage` |
//!
//! The list shows each contract with the terms the charge split reads and the
//! tariff of its services.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::documents::today;

use super::{Texts, invalid, parse_date, parse_id, party_name, require, text, text_with};
use crate::contract::{PayerContract, PayerTariff};
use crate::contracts::{self, NewContract};
use crate::money_text;
use crate::permissions;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/clinics/contracts", index)
        .name("clinics.contracts")
        .post("/clinics/contracts", create)
        .name("clinics.contract_create")
        .post("/clinics/contract-tariffs", tariff)
        .name("clinics.contract_tariff")
        .require_auth()
}

/// One contract of the list.
#[derive(Serialize, Debug, Clone)]
pub struct ContractRow {
    /// The primary key.
    pub id: i64,
    /// The payer's name.
    pub payer: String,
    /// The plan, or a dash for every plan.
    pub plan: String,
    /// The payer agreement, or a dash.
    pub agreement: String,
    /// The copay as a percentage.
    pub copay: String,
    /// The deductible of a visit.
    pub deductible: String,
    /// The first day.
    pub valid_from: String,
    /// The last day, or a dash.
    pub valid_to: String,
    /// The tariff: `laser AED 450.00, peel excluded`.
    pub tariffs: String,
    /// The windows: `30 days to send, 30 to resubmit (2 times)`.
    pub windows: String,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/contracts.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    contracts: Vec<ContractRow>,
    contract_options: Vec<[String; 2]>,
    contract_selected: String,
    covered_options: Vec<[String; 2]>,
    late_options: Vec<[String; 2]>,
    today: String,
    can_manage: bool,
}

/// A copay in basis points as a percentage: `20%`, `12.5%`.
fn percent(bp: i64) -> String {
    if bp % 100 == 0 {
        format!("{}%", bp / 100)
    } else {
        format!("{}.{:02}%", bp / 100, bp % 100)
    }
}

/// `GET /clinics/contracts`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    require(&user, permissions::POLICIES_VIEW)?;
    let all = PayerContract::query()
        .order_by_desc(PayerContract::ID)
        .get(&state.db)
        .await?;
    let dash = text(&lang, "none");
    let mut rows = Vec::with_capacity(all.len());
    let mut contract_options = Vec::with_capacity(all.len());
    for c in &all {
        let payer = party_name(&state, &lang, c.payer()).await;
        let tariffs = PayerTariff::of_contract(&state.db, c.id)
            .await?
            .iter()
            .map(|t| {
                if !t.covered {
                    format!("{} {}", t.service_code, text(&lang, "contract.covered_no"))
                } else {
                    match t.price {
                        Some(price) => format!("{} {}", t.service_code, money_text(price)),
                        None => t.service_code.clone(),
                    }
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        contract_options.push([
            c.id.to_string(),
            format!(
                "{} — {}",
                payer,
                c.plan.clone().unwrap_or_else(|| dash.clone())
            ),
        ]);
        rows.push(ContractRow {
            id: c.id,
            payer,
            plan: c.plan.clone().unwrap_or_else(|| dash.clone()),
            agreement: c
                .agreement_id
                .map_or_else(|| dash.clone(), |id| format!("#{id}")),
            copay: percent(c.copay_bp),
            deductible: money_text(c.deductible),
            valid_from: c.valid_from.to_string(),
            valid_to: c.valid_to.map_or_else(|| dash.clone(), |d| d.to_string()),
            windows: text_with(
                &lang,
                "contract.windows_text",
                &[
                    ("first", &c.first_submission_days),
                    ("reminder", &c.reminder_days),
                    ("again", &c.resubmission_days),
                    ("limit", &c.resubmission_limit),
                ],
            ),
            tariffs: if tariffs.is_empty() {
                dash.clone()
            } else {
                tariffs
            },
        });
    }
    let contract_selected = contract_options
        .first()
        .map(|option| option[0].clone())
        .unwrap_or_default();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        contracts: rows,
        contract_options,
        contract_selected,
        covered_options: vec![
            ["yes".to_owned(), text(&lang, "contract.covered_yes")],
            ["no".to_owned(), text(&lang, "contract.covered_no")],
        ],
        late_options: vec![
            ["no".to_owned(), text(&lang, "contract.late_no")],
            ["yes".to_owned(), text(&lang, "contract.late_yes")],
        ],
        today: today().to_string(),
        can_manage: user.has_permission(permissions::AUTHORISATIONS_MANAGE),
    }))
}

/// The form that records a contract.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ContractForm {
    /// The payer, a party's id.
    #[serde(default)]
    pub payer: String,
    /// The plan; blank for every plan.
    #[serde(default)]
    pub plan: String,
    /// The payer agreement in wl-invoicing.
    #[serde(default)]
    pub agreement: String,
    /// The copay in basis points.
    #[serde(default)]
    pub copay_bp: String,
    /// The deductible of a visit in fils.
    #[serde(default)]
    pub deductible: String,
    /// The first day.
    #[serde(default)]
    pub valid_from: String,
    /// The last day.
    #[serde(default)]
    pub valid_to: String,
    /// Days after the visit a first claim is accepted; blank = 30.
    #[serde(default)]
    pub first_submission_days: String,
    /// Days after a remittance a resubmission is accepted; blank = 30.
    #[serde(default)]
    pub resubmission_days: String,
    /// How many resubmissions of a claim are accepted; blank = 2.
    #[serde(default)]
    pub resubmission_limit: String,
    /// `yes` when a late first claim is accepted with a late reason.
    #[serde(default)]
    pub accepts_late_claims: String,
    /// Days after the visit to remind the billing coordinator of a claim
    /// not sent (`CLN.RUL.03`, N); blank = 7, or below a short window.
    #[serde(default)]
    pub reminder_days: String,
}

/// `POST /clinics/contracts`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<ContractForm>,
) -> Result<Response> {
    require(&user, permissions::AUTHORISATIONS_MANAGE)?;
    let payer = parse_id(&form.payer).ok_or_else(|| invalid("payer: 1, 2, 3 ..."))?;
    let from = parse_date(&form.valid_from)?.unwrap_or_else(today);
    let mut new = NewContract::new(PartyId(payer), from).plan(&form.plan);
    if let Some(agreement) = parse_id(&form.agreement) {
        new = new.agreement(agreement);
    }
    if !form.copay_bp.trim().is_empty() {
        new = new.copay_bp(parse_id(&form.copay_bp).ok_or_else(|| invalid("copay: 0 to 10000"))?);
    }
    if !form.deductible.trim().is_empty() {
        new =
            new.deductible(parse_id(&form.deductible).ok_or_else(|| invalid("deductible: fils"))?);
    }
    if let Some(to) = parse_date(&form.valid_to)? {
        new = new.valid_to(to);
    }
    let days = |text: &str, default: i64| -> std::result::Result<i64, Error> {
        if text.trim().is_empty() {
            Ok(default)
        } else {
            parse_id(text).ok_or_else(|| invalid("windows: days"))
        }
    };
    let first = days(&form.first_submission_days, new.first_submission_days)?;
    let resubmission = days(&form.resubmission_days, new.resubmission_days)?;
    let limit = days(&form.resubmission_limit, new.resubmission_limit)?;
    new = new.windows(first, resubmission, limit);
    if form.accepts_late_claims == "yes" {
        new = new.accepts_late_claims();
    }
    if !form.reminder_days.trim().is_empty() {
        new = new.reminder_days(
            parse_id(&form.reminder_days).ok_or_else(|| invalid("reminder: days"))?,
        );
    }
    contracts::create(&state, Some(user.id), new).await?;
    Ok((
        Toast::success(text(&lang, "contract.created")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that sets a tariff.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct TariffForm {
    /// The contract's id.
    #[serde(default)]
    pub contract: String,
    /// The service code.
    #[serde(default)]
    pub code: String,
    /// The payer's price of one unit in fils; blank uses the price list.
    #[serde(default)]
    pub price: String,
    /// `yes` (the default) or `no` for an exclusion.
    #[serde(default)]
    pub covered: String,
}

/// `POST /clinics/contract-tariffs`.
async fn tariff(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<TariffForm>,
) -> Result<Response> {
    require(&user, permissions::AUTHORISATIONS_MANAGE)?;
    let id = parse_id(&form.contract).ok_or_else(|| invalid("contract: 1, 2, 3 ..."))?;
    let price = if form.price.trim().is_empty() {
        None
    } else {
        Some(parse_id(&form.price).ok_or_else(|| invalid("price: fils"))?)
    };
    contracts::set_tariff(&state, id, &form.code, price, form.covered != "no").await?;
    Ok((
        Toast::success(text(&lang, "contract.tariff_set")),
        renox::HxRefresh,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copay_reads_as_a_percentage() {
        assert_eq!(percent(2_000), "20%");
        assert_eq!(percent(1_250), "12.50%");
        assert_eq!(percent(0), "0%");
    }
}
