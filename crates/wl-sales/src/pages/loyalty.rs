//! The loyalty pages (wl-sales §5.9), each behind its permission.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /loyalty` | `wl_sales.loyalty` | `loyalty.view` |
//! | `POST /loyalty/programme` | `wl_sales.loyalty_programme` | `loyalty.manage` |
//! | `POST /loyalty/accounts` | `wl_sales.loyalty_enrol` | `loyalty.enrol` |
//! | `POST /counter-sales/{id}/member` | `wl_sales.counter_sale_member` | `loyalty.enrol` |
//! | `POST /sales-orders/{id}/member` | `wl_sales.sales_order_member` | `loyalty.enrol` |
//!
//! The page shows the programme, the latest accounts with their balance, and
//! the sheets to turn the programme on and to enrol a member.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::PartyId;

use crate::counter_sale::CounterSale;
use crate::loyalty_account::{self, Enrolment, LoyaltyAccount};
use crate::loyalty_attach::{self, MemberRef};
use crate::loyalty_programme::{self, EarnKind, LoyaltyProgramme, NewProgramme};
use crate::notices::aed;
use crate::sales_order::SalesOrder;
use crate::pages::campaigns::parse_fils;
use crate::{permissions, texts};

/// The texts a template reads as `tx['loyalty.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the pages, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/loyalty", index)
                .name("wl_sales.loyalty")
                .require_permission(permissions::LOYALTY_VIEW),
        )
        .merge(
            Routes::new()
                .post("/loyalty/programme", create_programme)
                .name("wl_sales.loyalty_programme")
                .require_permission(permissions::LOYALTY_MANAGE),
        )
        .merge(
            Routes::new()
                .post("/loyalty/accounts", enrol)
                .name("wl_sales.loyalty_enrol")
                .post("/counter-sales/{id}/member", attach_counter_sale)
                .name("wl_sales.counter_sale_member")
                .post("/sales-orders/{id}/member", attach_sales_order)
                .name("wl_sales.sales_order_member")
                .require_permission(permissions::LOYALTY_ENROL),
        )
}

/// The package text `key` (without the `wl_sales.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_sales.{key}"))
}

/// A 422 with `message` under `field`, keeping what was typed.
fn invalid<F: Serialize>(field: &str, message: String, form: &F) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// The programme as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct ProgrammeView {
    /// The name.
    pub name: String,
    /// What it counts, in words.
    pub earn: String,
    /// The least eligible amount, `AED 0.00`.
    pub minimum: String,
    /// What a point is worth, `AED 0.01`.
    pub point_value: String,
    /// The months points are valid.
    pub validity_months: i64,
    /// The version of the terms.
    pub terms_version: String,
}

/// One account as the list shows it.
#[derive(Serialize, Debug, Clone)]
pub struct AccountRow {
    /// The primary key.
    pub id: i64,
    /// The member's code.
    pub code: String,
    /// The member's party id.
    pub party: i64,
    /// The mobile number.
    pub mobile: String,
    /// The balance in points, or the visits since the last reward.
    pub balance: i64,
    /// The status in words.
    pub status_label: String,
    /// The badge colour.
    pub status_kind: &'static str,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/loyalty.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    programme: Option<ProgrammeView>,
    accounts: Vec<AccountRow>,
    can_manage: bool,
    can_enrol: bool,
}

/// `GET /loyalty`: the programme and the latest accounts.
async fn index(State(db): State<Db>, user: AuthUser, lang: Lang) -> Result<View> {
    let programme = LoyaltyProgramme::first_active(&db).await?;
    let per_visit = programme
        .as_ref()
        .is_some_and(|p| p.earn_kind == EarnKind::PerVisit);
    let accounts = LoyaltyAccount::query()
        .order_by_desc(LoyaltyAccount::ID)
        .get(&db)
        .await?
        .into_iter()
        .take(50)
        .map(|a| AccountRow {
            id: a.id,
            code: a.code.clone(),
            party: a.party_id,
            mobile: a.mobile.clone(),
            balance: if per_visit { a.visits } else { a.points_balance },
            status_label: text(&lang, &format!("loyalty.status.{}", a.status.as_str())),
            status_kind: if a.is_active() { "success" } else { "info" },
        })
        .collect();
    let view = programme.map(|p| {
        let rate: [(&str, &dyn std::fmt::Display); 1] = [("rate", &p.earn_rate)];
        let key = match p.earn_kind {
            EarnKind::PerAed => "wl_sales.loyalty.earn.per_aed",
            EarnKind::PerVisit => "wl_sales.loyalty.earn.per_visit",
        };
        ProgrammeView {
            earn: texts::t_with(&lang, key, &rate),
            minimum: format!("AED {}", aed(p.minimum_amount)),
            point_value: format!("AED {}", aed(p.point_value)),
            validity_months: p.validity_months,
            terms_version: p.terms_version.clone(),
            name: p.name.clone(),
        }
    });
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        programme: view,
        accounts,
        can_manage: user.has_permission(permissions::LOYALTY_MANAGE),
        can_enrol: user.has_permission(permissions::LOYALTY_ENROL),
    }))
}

/// `POST /loyalty/programme`: turns the programme on. Fields: `name`,
/// `earn_kind` (`per_aed` or `per_visit`), `earn_rate`, `minimum` and
/// `point_value` (AED), `validity_months`, `terms_version`, `terms_en`,
/// `terms_ar`, `brand`, `excluded_items` (codes separated by commas) and
/// `visit_reward`.
async fn create_programme(
    State(state): State<AppState>,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let number = |key: &str, default: i64| -> Result<i64> {
        match get(key) {
            "" => Ok(default),
            v => v
                .parse::<i64>()
                .map_err(|_| invalid(key, text(&lang, "loyalty.errors.number"), &form)),
        }
    };
    let fils = |key: &str, default: i64| -> Result<i64> {
        match get(key) {
            "" => Ok(default),
            v => parse_fils(v)
                .filter(|f| *f >= 0)
                .ok_or_else(|| invalid(key, text(&lang, "loyalty.errors.amount"), &form)),
        }
    };
    let mut new = NewProgramme::new(get("name"));
    match get("earn_kind") {
        "" | "per_aed" => new.earn_kind = EarnKind::PerAed,
        "per_visit" => {
            new.earn_kind = EarnKind::PerVisit;
            new.visit_reward = get("visit_reward").to_owned();
        }
        _ => {
            return Err(invalid(
                "earn_kind",
                text(&lang, "loyalty.errors.choice"),
                &form,
            ));
        }
    }
    new.earn_rate = number("earn_rate", 1)?;
    new.minimum_amount = fils("minimum", 0)?;
    new.point_value = fils("point_value", 1)?;
    new.validity_months = number("validity_months", 12)?;
    if !get("terms_version").is_empty() {
        new.terms_version = get("terms_version").to_owned();
    }
    new.terms_text_en = get("terms_en").to_owned();
    new.terms_text_ar = get("terms_ar").to_owned();
    new = new.brand(get("brand"));
    new.excluded_items = get("excluded_items")
        .split(',')
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty())
        .collect();
    loyalty_programme::create(&state.db, new).await?;
    Ok((
        Toast::success(text(&lang, "loyalty.programme_created")),
        Redirect::route("wl_sales.loyalty", &[])?,
    )
        .into_response())
}

/// `POST /loyalty/accounts`: enrols a member at the counter. Fields:
/// `party_id`, `mobile`, `guardian_party_id` (a minor's guardian, who
/// accepts the terms) and `terms` (the checkbox that records the member
/// accepted the programme's current terms).
async fn enrol(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let party: i64 = get("party_id")
        .parse()
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| invalid("party_id", text(&lang, "loyalty.errors.number"), &form))?;
    let mut enrolment = Enrolment::new(PartyId(party), get("mobile")).by_user(user.id);
    if !get("guardian_party_id").is_empty() {
        let guardian: i64 = get("guardian_party_id")
            .parse()
            .ok()
            .filter(|p| *p > 0)
            .ok_or_else(|| {
                invalid(
                    "guardian_party_id",
                    text(&lang, "loyalty.errors.number"),
                    &form,
                )
            })?;
        enrolment = enrolment.guardian(PartyId(guardian));
    }
    if matches!(get("terms"), "1" | "on") {
        enrolment = enrolment.accepted_terms();
    }
    loyalty_account::enrol(&state, enrolment).await?;
    Ok((
        Toast::success(text(&lang, "loyalty.enrolled")),
        Redirect::route("wl_sales.loyalty", &[])?,
    )
        .into_response())
}

/// The member the form names: a `code`, else a `mobile` number, else the
/// member's `party_id` (a portal login).
fn member_of(form: &BTreeMap<String, String>, lang: &Lang) -> Result<MemberRef> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    if !get("code").is_empty() {
        return Ok(MemberRef::Code(get("code").to_owned()));
    }
    if !get("mobile").is_empty() {
        return Ok(MemberRef::Mobile(get("mobile").to_owned()));
    }
    match get("party_id").parse::<i64>() {
        Ok(party) if party > 0 => Ok(MemberRef::Party(PartyId(party))),
        _ => Err(invalid("code", text(lang, "loyalty.errors.member"), form)),
    }
}

/// `POST /counter-sales/{id}/member`: attaches the member (`code`, `mobile`
/// or `party_id`) to the open sale, `SAL.ACL.08`.
async fn attach_counter_sale(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(sale): Found<CounterSale>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let member = member_of(&form, &lang)?;
    loyalty_attach::attach_to_counter_sale(&state, &user, sale.id, &member).await?;
    Ok((
        Toast::success(text(&lang, "loyalty.attached")),
        Redirect::route("wl_sales.counter_sale", &[&sale.id])?,
    )
        .into_response())
}

/// `POST /sales-orders/{id}/member`: attaches the member to the order until
/// it is fulfilled.
async fn attach_sales_order(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(order): Found<SalesOrder>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let member = member_of(&form, &lang)?;
    loyalty_attach::attach_to_sales_order(&state, &user, order.id, &member).await?;
    Ok((
        Toast::success(text(&lang, "loyalty.attached")),
        Redirect::route("wl_sales.sales_order", &[&order.id])?,
    )
        .into_response())
}
