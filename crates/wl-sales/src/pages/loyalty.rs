//! The loyalty pages (wl-sales §5.9), each behind its permission.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /loyalty` | `wl_sales.loyalty` | `loyalty.view` |
//! | `POST /loyalty/programme` | `wl_sales.loyalty_programme` | `loyalty.manage` |
//! | `GET /loyalty/review` | `wl_sales.loyalty_review` | `loyalty.manage` |
//! | `POST /loyalty/review/{id}/resolve` | `wl_sales.loyalty_review_resolve` | `loyalty.manage` |
//! | `POST /loyalty/accounts` | `wl_sales.loyalty_enrol` | `loyalty.enrol` |
//! | `POST /loyalty/accounts/{id}/leave` | `wl_sales.loyalty_leave` | `loyalty.enrol` |
//! | `POST /loyalty/programme/changes` | `wl_sales.loyalty_terms_change` | `loyalty.manage` |
//! | `POST /loyalty/programme/off` | `wl_sales.loyalty_programme_off` | `loyalty.manage` |
//! | `POST /counter-sales/{id}/member` | `wl_sales.counter_sale_member` | `loyalty.enrol` |
//! | `POST /sales-orders/{id}/member` | `wl_sales.sales_order_member` | `loyalty.enrol` |
//!
//! The page shows the programme, the latest accounts with their balance, and
//! the sheets to turn the programme on and to enrol a member.

use std::collections::BTreeMap;

use renox::grid::{Action, Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::PartyId;

use crate::closure;
use crate::counter_sale::CounterSale;
use crate::loyalty_account::{self, Enrolment, LoyaltyAccount};
use crate::loyalty_attach::{self, MemberRef};
use crate::loyalty_points::LoyaltyReview;
use crate::loyalty_programme::{self, EarnKind, LoyaltyProgramme, NewProgramme};
use crate::notices::aed;
use crate::pages::campaigns::parse_fils;
use crate::sales_order::SalesOrder;
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
                .get("/loyalty/review", review)
                .name("wl_sales.loyalty_review")
                .post("/loyalty/review/{id}/resolve", resolve_review)
                .name("wl_sales.loyalty_review_resolve")
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
    /// The last day points can be redeemed once the programme was turned off.
    pub closing: String,
}

/// The members grid: search on the code, name and mobile number, filter by
/// status, the balance in points (the visits of a per-visit programme). The
/// member's name, the reminder note and the row buttons are drawn by the
/// template (`custom`).
pub(crate) fn grid(lang: &Lang, per_visit: bool, can_export: bool) -> Grid {
    let t = |key: &str| text(lang, key);
    let statuses = vec![
        ("active".to_owned(), t("loyalty.status.active")),
        ("closed".to_owned(), t("loyalty.status.closed")),
    ];
    let balance = if per_visit {
        Column::number("visits", &t("loyalty.balance"))
    } else {
        Column::number("points_balance", &t("loyalty.balance")).summary(Summary::Sum)
    };
    let grid = Grid::new("sales-loyalty-members")
        .title(&t("loyalty.accounts"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("code", &t("loyalty.code"))
                .frozen()
                .mobile()
                .searchable()
                .copyable(),
        )
        .column(Column::custom("party", &t("loyalty.member")).mobile())
        .column(
            Column::related(
                "member_name",
                &t("loyalty.member"),
                "wl_parties_parties",
                "party_id",
                "display_name",
            )
            .hidden()
            .searchable(),
        )
        .column(Column::text("mobile", &t("loyalty.mobile")).searchable())
        .column(balance.mobile())
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[("active", "success"), ("closed", "info")])
                .mobile(),
        )
        .column(Column::custom("reminder", &t("loyalty.reminder")))
        .column(Column::custom("actions", ""))
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(&t("loyalty.no_accounts"), Some(&t("loyalty.no_accounts_hint")));
    if can_export { grid.exports() } else { grid }
}

/// What a member's sheets need: the points that leaving would forfeit.
#[derive(Serialize, Debug, Clone)]
pub struct AccountSheet {
    /// The account.
    pub id: i64,
    /// The points leaving would forfeit.
    pub leave_points: i64,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/loyalty.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    programme: Option<ProgrammeView>,
    accounts: GridPage<LoyaltyAccount>,
    sheets: Vec<AccountSheet>,
    subtitle: String,
    can_manage: bool,
    can_enrol: bool,
    can_adjust: bool,
}

/// `GET /loyalty`: the programme and its members (`?export=` with
/// `loyalty.manage`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let db = state.db.clone();
    let programme = LoyaltyProgramme::first_active(&db).await?;
    let per_visit = programme
        .as_ref()
        .is_some_and(|p| p.earn_kind == EarnKind::PerVisit);
    let can_manage = user.has_permission(permissions::LOYALTY_MANAGE);
    let grid = grid(&lang, per_visit, can_manage);
    if request.param("export").is_some() && !can_manage {
        return Err(Error::Forbidden);
    }
    let query = LoyaltyAccount::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra = Vec::new();
    let mut sheets = Vec::new();
    for a in page.items() {
        let leave_points = if a.is_active() {
            closure::forfeit_of(&db, a).await?.points
        } else {
            0
        };
        if a.is_active() {
            sheets.push(AccountSheet {
                id: a.id,
                leave_points,
            });
        }
        extra.push(json!({
            "party": super::party_name(&state, &lang, a.party_id).await,
            "reminder": if a.expiry_reminder_failed_at.is_some() {
                text(&lang, "loyalty.reminder_not_delivered")
            } else {
                String::new()
            },
            "open": a.is_active(),
        }));
    }
    let mut extra = extra.into_iter();
    let accounts = page.extend(|_| extra.next().unwrap_or_default());
    let subtitle = if programme.is_some() {
        text(&lang, "loyalty.subtitle")
    } else {
        format!(
            "{} {}",
            text(&lang, "loyalty.subtitle"),
            text(&lang, "loyalty.no_programme")
        )
    };
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
            closing: p.closing_date.map(|d| d.to_string()).unwrap_or_default(),
            name: p.name.clone(),
        }
    });
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        programme: view,
        accounts,
        sheets,
        subtitle,
        can_manage,
        can_enrol: user.has_permission(permissions::LOYALTY_ENROL),
        can_adjust: user.has_permission(permissions::LOYALTY_ADJUST),
    })
    .into_response())
}

/// `POST /loyalty/programme`: turns the programme on. Fields: `name`,
/// `earn_kind` (`per_aed` or `per_visit`), `earn_rate`, `minimum` and
/// `point_value` (AED), `validity_months`, `terms_version`, `terms_en`,
/// `terms_ar`, `brand`, `excluded_items` (codes separated by commas),
/// `visit_reward`, `min_redeem_points` and `max_share_percent` (the largest
/// part of a sale points may pay).
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
    new.min_redeem_points = number("min_redeem_points", 500)?;
    new.max_share_bps = number("max_share_percent", 100)?.saturating_mul(100);
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

/// The review grid: the accounts that earned on more sales in a day than the
/// limit, with "Mark reviewed" in each row's menu.
pub(crate) fn review_grid(lang: &Lang) -> Grid {
    let t = |key: &str| text(lang, key);
    Grid::new("sales-loyalty-review")
        .title(&t("loyalty.review.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::related(
                "code",
                &t("loyalty.code"),
                "wl_sales_loyalty_accounts",
                "account_id",
                "code",
            )
            .frozen()
            .mobile()
            .searchable(),
        )
        .column(Column::custom("party", &t("loyalty.member")).mobile())
        .column(Column::date("on_date", &t("loyalty.review.day")).mobile())
        .column(Column::number("sales_count", &t("loyalty.review.sales")).mobile())
        .sort_by("-on_date")
        .row_action(
            Action::new(
                &t("loyalty.review.resolve"),
                "/loyalty/review/{id}/resolve",
            )
            .confirm(&t("loyalty.review.resolve")),
        )
        .cards_on_mobile()
        .empty_state(
            &t("loyalty.review.empty"),
            Some(&t("loyalty.review.empty_hint")),
        )
}

/// What the review page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/loyalty_review.html")]
pub(crate) struct ReviewPage {
    tx: Texts,
    reviews: GridPage<LoyaltyReview>,
}

/// `GET /loyalty/review`: the accounts that earned on more sales in a day
/// than the limit and that no manager has reviewed (`SAL.ACL.08`).
async fn review(
    State(state): State<AppState>,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let db = state.db.clone();
    let query = LoyaltyReview::query().where_(LoyaltyReview::RESOLVED_AT.is_null());
    let page = review_grid(&lang).page(query, &request).await?;
    let mut parties = Vec::new();
    for item in page.items() {
        let name = match LoyaltyAccount::find(&db, item.account_id).await? {
            Some(account) => super::party_name(&state, &lang, account.party_id).await,
            None => String::new(),
        };
        parties.push(json!({ "party": name }));
    }
    let mut parties = parties.into_iter();
    let reviews = page.extend(|_| parties.next().unwrap_or_default());
    Ok(View::typed(ReviewPage {
        tx: texts::all(&lang),
        reviews,
    })
    .into_response())
}

/// `POST /loyalty/review/{id}/resolve`: a manager reviewed the account.
async fn resolve_review(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let item = LoyaltyReview::find(&db, id).await?.ok_or(Error::NotFound)?;
    LoyaltyReview::query()
        .where_(LoyaltyReview::ID.eq(item.id))
        .update(
            &db,
            &[
                ("resolved_at", &Some(renox::db::now())),
                ("resolved_by", &Some(user.id)),
            ],
        )
        .await?;
    Ok((
        Toast::success(text(&lang, "loyalty.review.resolved")),
        Redirect::route("wl_sales.loyalty_review", &[])?,
    )
        .into_response())
}
