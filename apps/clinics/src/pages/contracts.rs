//! The payer contract pages (05 `CLN.INS.01`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /clinics/contracts` | `clinics.contracts` | `insurance_policies.view` |
//! | `POST /clinics/contracts` | `clinics.contract_create` | `insurance.manage` |
//! | `POST /clinics/contracts/{id}` | `clinics.contract_update` | `insurance.manage` |
//! | `POST /clinics/contract-tariffs` | `clinics.contract_tariff` | `insurance.manage` |
//! | `POST /clinics/contract-missed-window` | `clinics.contract_missed_window` | `contracts.missed_window_billing` (the owner's) |
//!
//! The list shows each contract with the terms the charge split reads and the
//! tariff of its services. A contract is edited in place: the agreement it is
//! billed under (picked by name from the payer agreements), the copay, the
//! deductible, the last day, the windows and the reminder; the payer, the plan
//! and the first day name the contract and stay (a different one is a new
//! contract). Each change is audited with the terms before it.

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::{subject_label, today};
use wl_kernel::{PartyId, Subject};

use super::claims::parse_aed;
use super::{
    Texts, invalid, parse_date, parse_id, parse_party, party_exists, party_name, require, text,
    text_with,
};
use crate::claim_closing;
use crate::contract::{PayerContract, PayerTariff};
use crate::contracts::{self, NewContract};
use crate::money_text;
use crate::permissions;
use crate::service_names;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/clinics/contracts", index)
        .name("clinics.contracts")
        .post("/clinics/contracts", create)
        .name("clinics.contract_create")
        .post("/clinics/contracts/{id}", update)
        .name("clinics.contract_update")
        .post("/clinics/contract-tariffs", tariff)
        .name("clinics.contract_tariff")
        .post("/clinics/contract-missed-window", missed_window)
        .name("clinics.contract_missed_window")
        .require_auth()
}

/// What the edit sheet of one contract starts with.
#[derive(Serialize, Debug, Clone)]
pub struct ContractSheet {
    /// The primary key.
    pub id: i64,
    /// The payer's name.
    pub payer: String,
    /// The plan, or a dash for every plan.
    pub plan: String,
    /// The agreement's id, for the edit form's picker.
    pub agreement_id: String,
    /// The copay as the edit form shows it (`20`, `12.5`).
    pub copay_edit: String,
    /// The deductible as the edit form shows it (`50.00`).
    pub deductible_edit: String,
    /// The last day as the edit form shows it.
    pub valid_to_edit: String,
    /// The first-submission days.
    pub first_days: i64,
    /// The resubmission days.
    pub resubmission_days: i64,
    /// The resubmission limit.
    pub resubmission_limit: i64,
    /// The reminder days.
    pub reminder_days: i64,
    /// `yes` or `no`.
    pub late_edit: &'static str,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/contracts.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    contracts: GridPage<PayerContract>,
    sheets: Vec<ContractSheet>,
    contract_options: Vec<[String; 2]>,
    contract_selected: String,
    covered_options: Vec<[String; 2]>,
    late_options: Vec<[String; 2]>,
    allowed_options: Vec<[String; 2]>,
    today: String,
    can_manage: bool,
    can_set_missed_billing: bool,
}

/// The contracts grid: search on the plan, filters on the dates, the
/// deductible and whether the patient may be billed for a missed window. The
/// payer, agreement, copay, windows and tariffs are drawn by the template
/// (`custom`), and so is the edit button of each row.
pub(crate) fn grid(lang: &Lang, can_export: bool, can_edit: bool) -> Grid {
    let mut grid = Grid::new("clinics-contracts")
        .title(&text(lang, "contract.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::custom("payer", &text(lang, "policies.payer"))
                .frozen()
                .mobile(),
        )
        .column(
            Column::text("plan", &text(lang, "contract.plan"))
                .mobile()
                .searchable(),
        )
        .column(Column::custom("agreement", &text(lang, "contract.agreement")))
        .column(Column::custom("copay", &text(lang, "contract.copay")).mobile())
        .column(Column::money("deductible", &text(lang, "contract.deductible")))
        .column(Column::date("valid_from", &text(lang, "contract.valid_from")).hidden())
        .column(Column::date("valid_to", &text(lang, "contract.valid_to")).mobile())
        .column(Column::custom("windows", &text(lang, "contract.windows")).hidden())
        .column(
            Column::bool(
                "bill_missed_to_patient",
                &text(lang, "contract.missed_billing"),
            )
            .hidden(),
        )
        .column(Column::custom("tariffs", &text(lang, "contract.tariffs")).hidden());
    if can_edit {
        grid = grid.column(Column::custom("edit", &text(lang, "contract.edit")).frozen_right());
    }
    let grid = grid
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(
            &text(lang, "contract.empty"),
            Some(&text(lang, "contract.subtitle")),
        );
    if can_export { grid.exports() } else { grid }
}

/// A copay in basis points as a percentage: `20%`, `12.5%`.
fn percent(bp: i64) -> String {
    if bp % 100 == 0 {
        format!("{}%", bp / 100)
    } else {
        format!("{}.{:02}%", bp / 100, bp % 100)
    }
}

/// A copay typed as a percentage (`20`, `12.5`, `12,5 %`) in basis points;
/// `None` outside 0 to 100%.
fn parse_percent(text: &str) -> Option<i64> {
    let cleaned = text.trim().trim_end_matches('%').trim().replace(',', ".");
    let (whole, fraction) = cleaned.split_once('.').unwrap_or((cleaned.as_str(), ""));
    if fraction.len() > 2 || !fraction.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let fraction: i64 = format!("{fraction:0<2}").parse().ok()?;
    let basis_points = whole.checked_mul(100)?.checked_add(fraction)?;
    (0..=10_000).contains(&basis_points).then_some(basis_points)
}

/// The tariff of a contract as the list reads it:
/// `laser AED 450.00, peel excluded`.
fn tariffs_text(
    lang: &Lang,
    names: &std::collections::BTreeMap<String, String>,
    tariffs: &[PayerTariff],
) -> String {
    let shown = tariffs
        .iter()
        .map(|t| {
            let what = service_names::name_of(names, &t.service_code);
            if !t.covered {
                format!("{what} {}", text(lang, "contract.covered_no"))
            } else {
                match t.price {
                    Some(price) => format!("{what} {}", money_text(price)),
                    None => what,
                }
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    if shown.is_empty() {
        text(lang, "none")
    } else {
        shown
    }
}

/// `GET /clinics/contracts` (`?export=` needs `insurance.manage`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    require(&user, permissions::POLICIES_VIEW)?;
    let can_manage = user.has_permission(permissions::AUTHORISATIONS_MANAGE);
    let grid = grid(&lang, can_manage, can_manage);
    if request.param("export").is_some() && !can_manage {
        return Err(Error::Forbidden);
    }
    let query = PayerContract::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let dash = text(&lang, "none");
    let service_names = service_names::names(&state, &lang.locale).await?;
    // The payer, agreement and tariff of the contracts on this page.
    let mut payers = BTreeMap::new();
    let mut agreements = BTreeMap::new();
    let mut tariffs = BTreeMap::new();
    for c in page.items() {
        payers.insert(c.id, party_name(&state, &lang, c.payer()).await);
        if let Some(id) = c.agreement_id {
            let label = subject_label(
                &state.db,
                &Subject::new(wl_kernel::invoicing::PAYER_AGREEMENT_SUBJECT_TYPE, id),
            )
            .await?
            .unwrap_or_else(|| format!("#{id}"));
            agreements.insert(c.id, label);
        }
        let found = PayerTariff::of_contract(&state.db, c.id).await?;
        tariffs.insert(c.id, tariffs_text(&lang, &service_names, &found));
    }
    let sheets: Vec<ContractSheet> = if can_manage {
        page.items()
            .iter()
            .map(|c| ContractSheet {
                id: c.id,
                payer: payers.get(&c.id).cloned().unwrap_or_default(),
                plan: c.plan.clone().unwrap_or_else(|| dash.clone()),
                agreement_id: c.agreement_id.map(|id| id.to_string()).unwrap_or_default(),
                copay_edit: percent(c.copay_bp).trim_end_matches('%').to_owned(),
                deductible_edit: format!("{}.{:02}", c.deductible / 100, c.deductible % 100),
                valid_to_edit: c.valid_to.map(|d| d.to_string()).unwrap_or_default(),
                first_days: c.first_submission_days,
                resubmission_days: c.resubmission_days,
                resubmission_limit: c.resubmission_limit,
                reminder_days: c.reminder_days,
                late_edit: if c.accepts_late_claims { "yes" } else { "no" },
            })
            .collect()
    } else {
        Vec::new()
    };
    let page = page.extend(|c| {
        json!({
            "payer": payers.get(&c.id),
            "agreement": agreements.get(&c.id).unwrap_or(&dash),
            "copay": percent(c.copay_bp),
            "windows": text_with(
                &lang,
                "contract.windows_text",
                &[
                    ("first", &c.first_submission_days),
                    ("reminder", &c.reminder_days),
                    ("again", &c.resubmission_days),
                    ("limit", &c.resubmission_limit),
                ],
            ),
            "tariffs": tariffs.get(&c.id),
        })
    });
    // The pickers of the tariff and missed-window sheets name every contract.
    let all = PayerContract::query()
        .order_by_desc(PayerContract::ID)
        .get(&state.db)
        .await?;
    let mut contract_options = Vec::with_capacity(all.len());
    for c in &all {
        let payer = party_name(&state, &lang, c.payer()).await;
        contract_options.push([
            c.id.to_string(),
            format!(
                "{} — {}",
                payer,
                c.plan.clone().unwrap_or_else(|| dash.clone())
            ),
        ]);
    }
    let contract_selected = contract_options
        .first()
        .map(|option| option[0].clone())
        .unwrap_or_default();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        contracts: page,
        sheets,
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
        allowed_options: vec![
            ["no".to_owned(), text(&lang, "contract.missed_billing_no")],
            ["yes".to_owned(), text(&lang, "contract.missed_billing_yes")],
        ],
        today: today().to_string(),
        can_manage,
        can_set_missed_billing: user.has_permission(permissions::MISSED_WINDOW_BILLING),
    })
    .into_response())
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
    /// The copay as a percentage of the covered amount (`20`, `12.5`).
    #[serde(default)]
    pub copay: String,
    /// The deductible of a visit in dirhams (`50`, `50.00`).
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
    let payer =
        parse_party(&form.payer).ok_or_else(|| invalid(&text(&lang, "policies.problem.payer")))?;
    if !party_exists(&state, payer).await? {
        return Err(invalid(&text(&lang, "policies.problem.payer_unknown")));
    }
    let from = parse_date(&form.valid_from)?.unwrap_or_else(today);
    let new = NewContract::new(PartyId(payer), from).plan(&form.plan);
    let new = apply_terms(new, &form, &lang)?;
    contracts::create(&state, Some(user.id), new).await?;
    Ok((
        Toast::success(text(&lang, "contract.created")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The terms a contract form sets on `new`: the agreement, the copay, the
/// deductible, the last day, the windows, the late-claims switch and the
/// reminder days (a blank field keeps the default).
fn apply_terms(mut new: NewContract, form: &ContractForm, lang: &Lang) -> Result<NewContract> {
    if let Some(agreement) = parse_id(&form.agreement) {
        new = new.agreement(agreement);
    }
    if !form.copay.trim().is_empty() {
        new = new.copay_bp(
            parse_percent(&form.copay)
                .ok_or_else(|| invalid(&text(lang, "contract.problem.copay")))?,
        );
    }
    if !form.deductible.trim().is_empty() {
        new = new.deductible(parse_aed(&form.deductible)?);
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
        new = new
            .reminder_days(parse_id(&form.reminder_days).ok_or_else(|| invalid("reminder: days"))?);
    }
    Ok(new)
}

/// `POST /clinics/contracts/{id}`: changes the terms of a contract; the
/// payer, the plan and the first day stay as they are.
async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ContractForm>,
) -> Result<Response> {
    require(&user, permissions::AUTHORISATIONS_MANAGE)?;
    let stored = PayerContract::find_or_404(&state.db, id).await?;
    let new = NewContract::new(stored.payer(), stored.valid_from)
        .plan(stored.plan.as_deref().unwrap_or_default());
    let new = apply_terms(new, &form, &lang)?;
    contracts::update(&state, Some(user.id), id, new).await?;
    Ok((
        Toast::success(text(&lang, "contract.updated")),
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
    /// The payer's price of one unit in dirhams; blank uses the price list.
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
    let id = parse_id(&form.contract)
        .ok_or_else(|| invalid(&text(&lang, "contract.problem.contract")))?;
    let price = if form.price.trim().is_empty() {
        None
    } else {
        Some(parse_aed(&form.price)?)
    };
    contracts::set_tariff(&state, id, &form.code, price, form.covered != "no").await?;
    Ok((
        Toast::success(text(&lang, "contract.tariff_set")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that sets the owner's flag.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct MissedWindowForm {
    /// The contract's id.
    #[serde(default)]
    pub contract: String,
    /// `yes` or `no`.
    #[serde(default)]
    pub allowed: String,
}

/// `POST /clinics/contract-missed-window`: whether the patient may be billed
/// for a payer share the clinic did not submit in time (`CLN.ACL.16`); the
/// owner's alone.
async fn missed_window(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<MissedWindowForm>,
) -> Result<Response> {
    require(&user, permissions::MISSED_WINDOW_BILLING)?;
    let id = parse_id(&form.contract)
        .ok_or_else(|| invalid(&text(&lang, "contract.problem.contract")))?;
    claim_closing::set_missed_window_billing(&state, &user, id, form.allowed == "yes").await?;
    Ok((
        Toast::success(text(&lang, "contract.missed_billing_set")),
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

    #[test]
    fn a_percentage_is_read_as_basis_points() {
        assert_eq!(parse_percent("20"), Some(2_000));
        assert_eq!(parse_percent(" 12.5 % "), Some(1_250));
        assert_eq!(parse_percent("12,5"), Some(1_250));
        assert_eq!(parse_percent("100"), Some(10_000));
        assert_eq!(parse_percent("0"), Some(0));
        assert_eq!(parse_percent("100.01"), None);
        assert_eq!(parse_percent("-5"), None);
        assert_eq!(parse_percent("2.555"), None);
        assert_eq!(parse_percent("a lot"), None);
    }
}
