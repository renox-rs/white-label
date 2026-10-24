//! The credit pages of the sales module (wl-sales §3 *Customer account*,
//! §4.2, `SAL.ACL.03`; BL-SAL-10, BL-SAL-12): the customer accounts, what
//! each customer owes against its limit, and the orders on credit hold.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /credit/accounts` | `wl_sales.credit_accounts` | `credit.view` |
//! | `GET /credit/accounts/{party}` | `wl_sales.credit_account` | `credit.view` |
//! | `POST /credit/accounts` | `wl_sales.credit_account_save` | `credit.limits.approve` |
//! | `POST /credit/accounts/{party}/clear-hold` | `wl_sales.credit_account_clear` | `credit.release` |
//! | `GET /credit/clearings/{id}` | `wl_sales.credit_clearing` | `credit.view` |
//!
//! The orders on credit hold are the sales order list with `?view=credit_hold`
//! (`sales_orders.view`); the reason of a hold is shown only to a holder of
//! `credit.view`.

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::PartyId;

use crate::account_hold;
use crate::account_hold_clearing::{AccountHoldClearing, ClearingStatus, KINDS};
use crate::config::config_of;
use crate::credit;
use crate::customer_account::{self, AccountHoldSource, CustomerAccount, Terms};
use crate::notices::aed;
use crate::pages::campaigns::parse_fils;
use crate::pages::party_name;
use crate::sale_line::SaleLine;
use crate::sales_order::{SalesOrder, SalesOrderStatus};
use crate::{permissions, texts};

/// The texts a template reads as `tx['credit.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the pages, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/credit/accounts", index)
                .name("wl_sales.credit_accounts")
                .get("/credit/accounts/{party}", show)
                .name("wl_sales.credit_account")
                .get("/credit/clearings/{id}", clearing)
                .name("wl_sales.credit_clearing")
                .require_permission(permissions::CREDIT_VIEW),
        )
        .merge(
            Routes::new()
                .post("/credit/accounts/{party}/clear-hold", clear_hold)
                .name("wl_sales.credit_account_clear")
                .require_permission(permissions::CREDIT_RELEASE),
        )
        .merge(
            Routes::new()
                .post("/credit/accounts", save)
                .name("wl_sales.credit_account_save")
                .require_permission(permissions::CREDIT_LIMITS_APPROVE),
        )
}

/// The package text `key` (without the `wl_sales.` prefix).
pub(crate) fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_sales.{key}"))
}

/// `fils` as `AED 4,000.00`.
pub(crate) fn money(fils: i64) -> String {
    format!("AED {}", aed(fils))
}

/// A 422 with `message` under `field`, keeping what was typed.
fn invalid<F: Serialize>(field: &str, message: String, form: &F) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/credit_accounts.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    accounts: GridPage<CustomerAccount>,
    can_edit: bool,
}

/// Whether an open source holds the account, worked out in the query so the
/// standing can be filtered and sorted.
const STANDING_SQL: &str = "CASE WHEN EXISTS (SELECT 1 FROM \"wl_sales_account_hold_sources\" \
    WHERE \"wl_sales_account_hold_sources\".\"account_id\" = {T}.\"id\" \
    AND \"wl_sales_account_hold_sources\".\"cleared_at\" IS NULL) THEN 'on_hold' ELSE 'good' END";

/// The accounts grid: search on the customer's name, filters by standing,
/// limit and terms. What the customer owes now is worked out from the open
/// invoices, cheques and orders, so it is drawn by the template (`custom`) and
/// is not sorted or filtered.
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let t = |key: &str| text(lang, key);
    let grid = Grid::new("sales-credit-accounts")
        .title(&t("credit.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::custom("customer", &t("orders.customer_name"))
                .frozen()
                .mobile(),
        )
        .column(
            Column::related(
                "customer_name",
                &t("orders.customer_name"),
                "wl_parties_parties",
                "party_id",
                "display_name",
            )
            .hidden()
            .searchable(),
        )
        .column(
            Column::money("credit_limit", &t("credit.limit"))
                .mobile()
                .summary(renox::grid::Summary::Sum),
        )
        .column(Column::custom("owes", &wl_kernel::money_label(&t("credit.owes"))).mobile())
        .column(Column::number("payment_terms_days", &t("credit.terms")))
        .column(Column::number("grace_days", &t("credit.grace")).hidden())
        .column(
            Column::related(
                "sales_rep",
                &t("credit.sales_rep_name"),
                "users",
                "sales_rep_id",
                "name",
            )
            .hidden()
            .searchable(),
        )
        .column(
            Column::select(
                "standing",
                &t("fields.status"),
                [
                    ("on_hold", t("credit.on_hold")),
                    ("good", t("credit.not_on_hold")),
                ],
            )
            .computed(STANDING_SQL)
            .badges(&[("on_hold", "error"), ("good", "success")])
            .mobile(),
        )
        .sort_by("id")
        .cards_on_mobile()
        .empty_state(&t("credit.empty"), Some(&t("credit.empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// `GET /credit/accounts`: the customer accounts.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let can_export = user.has_permission(permissions::CREDIT_LIMITS_APPROVE)
        || user.has_permission(permissions::CREDIT_RELEASE);
    let grid = grid(&lang, can_export);
    let query = CustomerAccount::query();
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut owes = std::collections::HashMap::new();
    let mut names = std::collections::HashMap::new();
    for account in page.items() {
        names.insert(
            account.id,
            party_name(&state, &lang, account.party_id).await,
        );
        let total = credit::exposure_of(&state, account.party(), None)
            .await?
            .total();
        owes.insert(account.id, money(total));
    }
    let page = page.extend(|account| {
        json!({
            "owes": owes.get(&account.id),
            "name": names.get(&account.id),
            "url": format!("/credit/accounts/{}", account.party_id),
        })
    });
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        accounts: page,
        can_edit: user.has_permission(permissions::CREDIT_LIMITS_APPROVE),
    })
    .into_response())
}

/// `AED 50,000.00`, or *No limit*.
fn limit_text(lang: &Lang, limit: Option<i64>) -> String {
    limit.map_or_else(|| text(lang, "credit.no_limit"), money)
}

/// *30 days*.
fn terms_text(lang: &Lang, days: i64) -> String {
    texts::t_with(lang, "wl_sales.credit.days", &[("days", &days)])
}

/// One order on hold as the account page lists it.
#[derive(Serialize, Debug, Clone)]
pub struct HeldRow {
    /// The order's id.
    pub id: i64,
    /// The order number.
    pub number: String,
    /// The value still to deliver.
    pub value: String,
    /// Why, in words.
    pub reason: String,
    /// Where it returns to, in words.
    pub returns_to: String,
}

/// One open source of the account's hold as the account page lists it.
#[derive(Serialize, Debug, Clone)]
pub struct SourceRow {
    /// The source's id.
    pub id: i64,
    /// What caused it, in words: *Cheque no. 12*.
    pub what: String,
    /// Why, in words.
    pub reason: String,
    /// Since when, `18 Oct 2026`.
    pub since: String,
    /// Why the clear button is disabled (a clearing waits); empty if not.
    pub blocked: String,
}

/// What the account page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/credit_account.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    sources: Vec<SourceRow>,
    approvals: Vec<wl_kernel::approvals::ApprovalCard>,
    can_clear: bool,
    kinds: Vec<(String, String)>,
    title: String,
    party: i64,
    account: CustomerAccount,
    limit: String,
    terms: String,
    grace: String,
    group: String,
    invoices: String,
    cheques: String,
    orders: String,
    owes: String,
    overdue: String,
    available: String,
    flag: String,
    held: Vec<HeldRow>,
    can_edit: bool,
    limit_input: String,
    terms_input: String,
    grace_input: String,
    rep_input: String,
}

/// `GET /credit/accounts/{party}`: one customer's terms, what it owes and
/// its orders on hold.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(party): Path<i64>,
) -> Result<View> {
    let account = CustomerAccount::of_party(&state.db, party)
        .await?
        .ok_or(Error::NotFound)?;
    let config = config_of(&state);
    let exposure = credit::exposure_of(&state, account.party(), None).await?;
    let overdue = credit::overdue_of(
        &state,
        account.party(),
        account.grace(config.credit_grace_days),
    )
    .await?;
    let mut held = Vec::new();
    for order in SalesOrder::query()
        .where_(SalesOrder::PARTY_ID.eq(party))
        .where_(SalesOrder::STATUS.eq(SalesOrderStatus::CreditHold))
        .order_by(SalesOrder::ID)
        .get(&state.db)
        .await?
    {
        let lines = SaleLine::of(&state.db, &order.subject()).await?;
        held.push(HeldRow {
            id: order.id,
            number: order.number.clone(),
            value: money(credit::undelivered_value(&lines)),
            reason: hold_reason_text(&lang, order.hold_reason.as_deref()),
            returns_to: text(
                &lang,
                &format!(
                    "orders.status.{}",
                    order.held_from.as_deref().unwrap_or("confirmed")
                ),
            ),
        });
    }
    let open = account.open_sources(&state.db).await?;
    let mut sources = Vec::new();
    let mut cards = Vec::new();
    for source in &open {
        let waiting = AccountHoldClearing::query()
            .where_(AccountHoldClearing::HOLD_SOURCE_ID.eq(source.id))
            .where_(AccountHoldClearing::STATUS.eq(ClearingStatus::PendingApproval))
            .get(&state.db)
            .await?;
        for clearing in &waiting {
            cards.extend(
                wl_kernel::approvals::cards_for(&state.db, &clearing.subject(), &user, &lang)
                    .await?,
            );
        }
        sources.push(source_row(&lang, source, !waiting.is_empty()));
    }
    let flag = if account.is_on_hold(&state.db).await? {
        text(&lang, "credit.on_hold")
    } else {
        text(&lang, "credit.not_on_hold")
    };
    let available = account.credit_limit.map_or_else(
        || text(&lang, "credit.no_limit"),
        |limit| money(limit - exposure.total()),
    );
    let kinds = KINDS
        .iter()
        .map(|kind| {
            (
                (*kind).to_owned(),
                text(&lang, &format!("credit.kind.{kind}")),
            )
        })
        .collect();
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        sources,
        approvals: cards,
        can_clear: user.has_permission(permissions::CREDIT_RELEASE),
        kinds,
        title: party_name(&state, &lang, party).await,
        party,
        limit: limit_text(&lang, account.credit_limit),
        terms: terms_text(&lang, account.payment_terms_days),
        grace: terms_text(&lang, account.grace(config.credit_grace_days)),
        group: match account.group(&state.db).await? {
            Some(group) => group.name,
            None => text(&lang, "credit.no_group"),
        },
        invoices: money(exposure.invoices),
        cheques: money(exposure.cheques),
        orders: money(exposure.orders),
        owes: money(exposure.total()),
        overdue: money(overdue),
        available,
        flag,
        held,
        can_edit: user.has_permission(permissions::CREDIT_LIMITS_APPROVE),
        limit_input: account
            .credit_limit
            .map(|fils| format!("{}.{:02}", fils / 100, fils % 100))
            .unwrap_or_default(),
        terms_input: account.payment_terms_days.to_string(),
        grace_input: account
            .grace_days
            .map(|d| d.to_string())
            .unwrap_or_default(),
        rep_input: account
            .sales_rep_id
            .map(|d| d.to_string())
            .unwrap_or_default(),
        account,
    }))
}

/// One open source of a hold as a row.
fn source_row(lang: &Lang, source: &AccountHoldSource, waiting: bool) -> SourceRow {
    let what = match source.source_type.as_str() {
        wl_kernel::cheques::CHEQUE_SUBJECT_TYPE => texts::t_with(
            lang,
            "wl_sales.credit.source_cheque",
            &[("id", &source.source_id)],
        ),
        _ => source.source().to_string(),
    };
    SourceRow {
        id: source.id,
        what,
        reason: text(lang, &format!("credit.source.{}", source.reason)),
        since: source
            .created_at
            .map(|at| at.format("%-d %b %Y").to_string())
            .unwrap_or_default(),
        blocked: if waiting {
            text(lang, "credit.clear_waiting")
        } else {
            String::new()
        },
    }
}

/// `GET /credit/clearings/{id}`: the page of the account a clearing is about
/// (where the approval card of a request in "Needs your action" leads).
async fn clearing(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Response> {
    let found = AccountHoldClearing::find_or_404(&state.db, id).await?;
    let account = CustomerAccount::find_or_404(&state.db, found.account_id).await?;
    Ok(Redirect::route("wl_sales.credit_account", &[&account.party_id])?.into_response())
}

/// `POST /credit/accounts/{party}/clear-hold`: a credit releaser asks to
/// clear one open source (`source`) of the hold, with `kind` (cheque
/// replaced, balance paid, other) and a `note`; another releaser approves.
async fn clear_hold(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(party): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let account = CustomerAccount::of_party(&state.db, party)
        .await?
        .ok_or(Error::NotFound)?;
    let source: i64 = get("source")
        .parse()
        .map_err(|_| invalid("source", text(&lang, "orders.errors.number"), &form))?;
    account_hold::request_clearing(
        &state,
        user.id,
        account.id,
        source,
        get("kind"),
        get("note"),
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "credit.clear_requested")),
        Redirect::route("wl_sales.credit_account", &[&party])?,
    )
        .into_response())
}

/// The reason of a hold in words.
pub(crate) fn hold_reason_text(lang: &Lang, reason: Option<&str>) -> String {
    text(
        lang,
        &format!("credit.reason.{}", reason.unwrap_or("limit")),
    )
}

/// `POST /credit/accounts`: makes or changes the account of the customer
/// `party_id`. Fields: `party_id`, `credit_limit` (AED, blank: none),
/// `payment_terms_days`, `grace_days` (blank: the setting), `sales_rep`
/// (user id, blank: none), `group_owner` (party id, blank: no group),
/// `group_name` and `group_limit` (AED, blank: none).
async fn save(
    State(state): State<AppState>,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let number = |field: &str, key: &str| -> Result<Option<i64>> {
        match get(field) {
            "" => Ok(None),
            v => v
                .parse::<i64>()
                .map(Some)
                .map_err(|_| invalid(field, text(&lang, key), &form)),
        }
    };
    let amount = |field: &str| -> Result<Option<i64>> {
        match get(field) {
            "" => Ok(None),
            v => parse_fils(v)
                .filter(|fils| *fils >= 0)
                .map(Some)
                .ok_or_else(|| invalid(field, text(&lang, "orders.errors.amount"), &form)),
        }
    };
    let party = number("party_id", "orders.errors.number")?
        .filter(|p| *p > 0)
        .ok_or_else(|| invalid("party_id", text(&lang, "orders.errors.number"), &form))?;
    let group = match number("group_owner", "orders.errors.number")? {
        Some(owner) => {
            let name = if get("group_name").is_empty() {
                format!("#{owner}")
            } else {
                get("group_name").to_owned()
            };
            Some(
                customer_account::save_group(&state, PartyId(owner), &name, amount("group_limit")?)
                    .await?
                    .id,
            )
        }
        None => None,
    };
    let terms = Terms {
        credit_limit: amount("credit_limit")?,
        payment_terms_days: number("payment_terms_days", "orders.errors.number")?.unwrap_or(30),
        grace_days: number("grace_days", "orders.errors.number")?,
        credit_group: group,
        sales_rep: number("sales_rep", "orders.errors.number")?,
    };
    customer_account::save(&state, PartyId(party), terms).await?;
    Ok((
        Toast::success(text(&lang, "credit.saved")),
        Redirect::route("wl_sales.credit_account", &[&party])?,
    )
        .into_response())
}
