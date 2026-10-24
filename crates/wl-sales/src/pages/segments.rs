//! The customer segment pages (wl-sales §5.10, BL-SAL-25).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /segments` | `wl_sales.segments` | `segments.manage` |
//! | `GET /segments/new` | `wl_sales.segment_new` | `segments.manage` |
//! | `POST /segments` | `wl_sales.segment_store` | `segments.manage` |
//! | `GET /segments/{id}` | `wl_sales.segment` | `segments.manage` |
//! | `POST /segments/{id}` | `wl_sales.segment_update` | `segments.manage` |
//! | `POST /segments/{id}/preview` | `wl_sales.segment_preview` | `segments.manage` |
//! | `GET /segments/{id}/export` | `wl_sales.segment_export` | `segments.export` |
//!
//! The editor asks one question per rule kind and keeps the ones that were
//! answered. It offers the items that were sold, without the clinical
//! services of 05 unless the setting allows them. The preview count is an
//! estimate; exporting the members is a sensitive export and is audited.

use std::collections::{BTreeMap, BTreeSet};

use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
use renox::validation::{Errors, ValidationError};
use serde::Serialize;

use crate::config::config_of;
use crate::notices::aed;
use crate::sale_line::{KIND_ITEM, SaleLine};
use crate::segment::{MatchMode, Rule, Segment};
use crate::segments;
use crate::{permissions, texts};

/// The texts a template reads as `tx['segments.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/segments", index)
                .name("wl_sales.segments")
                .get("/segments/new", new_form)
                .name("wl_sales.segment_new")
                .post("/segments", store)
                .name("wl_sales.segment_store")
                .get("/segments/{id}", edit_form)
                .name("wl_sales.segment")
                .post("/segments/{id}", update)
                .name("wl_sales.segment_update")
                .post("/segments/{id}/preview", preview)
                .name("wl_sales.segment_preview")
                .require_permission(permissions::SEGMENTS_MANAGE),
        )
        .merge(
            Routes::new()
                .get("/segments/{id}/export", export)
                .name("wl_sales.segment_export")
                .require_permission(permissions::SEGMENTS_EXPORT),
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

/// A rule in words.
pub fn describe(lang: &Lang, rule: &Rule) -> String {
    match rule {
        Rule::NoSaleForDays { days } => texts::t_with(
            lang,
            "wl_sales.segments.rule.no_sale_for_days",
            &[("days", days)],
        ),
        Rule::SpendOver { amount, days } => {
            let amount = format!("AED {}", aed(*amount));
            texts::t_with(
                lang,
                "wl_sales.segments.rule.spend_over",
                &[("amount", &amount), ("days", days)],
            )
        }
        Rule::SalesAtLeast { count, days } => texts::t_with(
            lang,
            "wl_sales.segments.rule.sales_at_least",
            &[("count", count), ("days", days)],
        ),
        Rule::BoughtItem { item, days } => match days {
            Some(days) => texts::t_with(
                lang,
                "wl_sales.segments.rule.bought_item_within",
                &[("item", item), ("days", days)],
            ),
            None => texts::t_with(
                lang,
                "wl_sales.segments.rule.bought_item",
                &[("item", item)],
            ),
        },
        Rule::LoyaltyBalanceAtLeast { points } => texts::t_with(
            lang,
            "wl_sales.segments.rule.loyalty_balance",
            &[("points", points)],
        ),
        Rule::PackageCreditsAtLeast { credits } => texts::t_with(
            lang,
            "wl_sales.segments.rule.package_credits",
            &[("credits", credits)],
        ),
        Rule::EnrolledInLoyalty { enrolled } => text(
            lang,
            if *enrolled {
                "segments.rule.enrolled"
            } else {
                "segments.rule.not_enrolled"
            },
        ),
        Rule::Vertical { name, .. } => {
            texts::t_with(lang, "wl_sales.segments.rule.vertical", &[("name", name)])
        }
    }
}

/// The match mode in words.
fn mode_text(lang: &Lang, mode: MatchMode) -> String {
    text(lang, &format!("segments.mode.{}", mode.as_str()))
}

/// The segments grid: search on the name, filter by match mode, the rules in
/// words (`custom`, worked out from the stored JSON) and the last count.
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let t = |key: &str| text(lang, key);
    let modes: Vec<(String, String)> = MatchMode::ALL
        .iter()
        .map(|mode| (mode.as_str().to_owned(), mode_text(lang, *mode)))
        .collect();
    let grid = Grid::new("sales-segments")
        .title(&t("segments.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("name", &t("segments.name"))
                .frozen()
                .mobile()
                .searchable()
                .link("/segments/{id}"),
        )
        .column(Column::select("match_mode", &t("segments.match"), modes))
        .column(Column::custom("rules", &t("segments.rules")).mobile())
        .column(
            Column::number("last_member_count", &t("segments.count"))
                .mobile()
                .summary(Summary::Sum),
        )
        .column(Column::datetime("last_counted_at", &t("segments.counted_at")).hidden())
        .column(Column::bool("archived", &t("segments.archived")).hidden())
        .sort_by("name")
        .row_url("/segments/{id}")
        .cards_on_mobile()
        .empty_state(&t("segments.empty"), Some(&t("segments.empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/segments.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    segments: GridPage<Segment>,
}

/// `GET /segments`: the segments with their rules and last count
/// (`?export=` with `segments.export`).
async fn index(user: AuthUser, lang: Lang, request: GridRequest) -> Result<Response> {
    let can_export = user.has_permission(permissions::SEGMENTS_EXPORT);
    let grid = grid(&lang, can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    let query = Segment::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let segments = grid.page(query, &request).await?.extend(|s| {
        json!({
            "rules": s
                .rules()
                .iter()
                .map(|r| describe(&lang, r))
                .collect::<Vec<_>>()
                .join("; "),
        })
    });
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        segments,
    })
    .into_response())
}

/// What the editor needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/segment.html")]
pub(crate) struct EditPage {
    tx: Texts,
    title: String,
    action: String,
    segment_id: Option<i64>,
    values: BTreeMap<&'static str, String>,
    modes: Vec<(String, String)>,
    items: Vec<(String, String)>,
    enrolled: Vec<(String, String)>,
    count: Option<String>,
    can_export: bool,
}

/// The items the editor offers: the codes that were sold, without the
/// clinical ones unless the setting allows them.
async fn offered_items(db: &Db, state: &AppState) -> Result<Vec<(String, String)>> {
    let config = config_of(state);
    let codes: BTreeSet<String> = SaleLine::query()
        .get(db)
        .await?
        .into_iter()
        .filter(|l| l.kind == KIND_ITEM)
        .map(|l| l.item_code)
        .filter(|code| !config.is_blocked_clinical(code))
        .collect();
    Ok(codes.into_iter().map(|c| (c.clone(), c)).collect())
}

/// The editor's answers for the rules of `segment`.
fn values_of(segment: &Segment) -> BTreeMap<&'static str, String> {
    let mut values = BTreeMap::new();
    values.insert("name", segment.name.clone());
    values.insert("match_mode", segment.match_mode.as_str().to_owned());
    for rule in segment.rules() {
        match rule {
            Rule::NoSaleForDays { days } => {
                values.insert("no_sale_days", days.to_string());
            }
            Rule::SpendOver { amount, days } => {
                values.insert("spend_amount", aed(amount).replace(',', ""));
                values.insert("spend_days", days.to_string());
            }
            Rule::SalesAtLeast { count, days } => {
                values.insert("sales_count", count.to_string());
                values.insert("sales_days", days.to_string());
            }
            Rule::BoughtItem { item, days } => {
                values.insert("item", item);
                values.insert("item_days", days.map(|d| d.to_string()).unwrap_or_default());
            }
            Rule::LoyaltyBalanceAtLeast { points } => {
                values.insert("balance", points.to_string());
            }
            Rule::PackageCreditsAtLeast { credits } => {
                values.insert("credits", credits.to_string());
            }
            Rule::EnrolledInLoyalty { enrolled } => {
                values.insert("enrolled", if enrolled { "yes" } else { "no" }.to_owned());
            }
            Rule::Vertical { .. } => {}
        }
    }
    values
}

/// The editor for a new segment (`segment` is `None`) or to change one.
async fn editor(
    state: &AppState,
    lang: &Lang,
    user: &AuthUser,
    segment: Option<&Segment>,
) -> Result<View> {
    let modes = [MatchMode::All, MatchMode::Any]
        .iter()
        .map(|m| (m.as_str().to_owned(), mode_text(lang, *m)))
        .collect();
    let enrolled = vec![
        ("yes".to_owned(), text(lang, "segments.enrolled.yes")),
        ("no".to_owned(), text(lang, "segments.enrolled.no")),
    ];
    let (title, action) = match segment {
        Some(s) => (s.name.clone(), format!("/segments/{}", s.id)),
        None => (text(lang, "segments.new_title"), "/segments".to_owned()),
    };
    let mut values = segment.map(values_of).unwrap_or_default();
    values
        .entry("match_mode")
        .or_insert_with(|| MatchMode::All.as_str().to_owned());
    Ok(View::typed(EditPage {
        tx: texts::all(lang),
        title,
        action,
        segment_id: segment.map(|s| s.id),
        values,
        modes,
        items: offered_items(&state.db, state).await?,
        enrolled,
        count: segment
            .and_then(|s| s.last_member_count)
            .map(|c| c.to_string()),
        can_export: user.has_permission(permissions::SEGMENTS_EXPORT),
    }))
}

/// `GET /segments/new`: the empty editor.
async fn new_form(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    editor(&state, &lang, &user, None).await
}

/// `GET /segments/{id}`: the editor of a segment.
async fn edit_form(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(segment): Found<Segment>,
) -> Result<View> {
    editor(&state, &lang, &user, Some(&segment)).await
}

/// The rules the form's answers describe.
fn rules_of_form(form: &BTreeMap<String, String>, lang: &Lang) -> Result<Vec<Rule>> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let number = |key: &str| -> Result<Option<i64>> {
        match get(key) {
            "" => Ok(None),
            v => v
                .parse::<i64>()
                .map(Some)
                .map_err(|_| invalid(key, text(lang, "segments.errors.number"), form)),
        }
    };
    let mut rules = Vec::new();
    if let Some(days) = number("no_sale_days")? {
        rules.push(Rule::NoSaleForDays { days });
    }
    if !get("spend_amount").is_empty() {
        let amount = crate::pages::campaigns::parse_fils(get("spend_amount"))
            .ok_or_else(|| invalid("spend_amount", text(lang, "segments.errors.amount"), form))?;
        let days = number("spend_days")?.unwrap_or(365);
        rules.push(Rule::SpendOver { amount, days });
    }
    if let Some(count) = number("sales_count")? {
        let days = number("sales_days")?.unwrap_or(365);
        rules.push(Rule::SalesAtLeast { count, days });
    }
    if !get("item").is_empty() {
        rules.push(Rule::BoughtItem {
            item: get("item").to_owned(),
            days: number("item_days")?,
        });
    }
    if let Some(points) = number("balance")? {
        rules.push(Rule::LoyaltyBalanceAtLeast { points });
    }
    if let Some(credits) = number("credits")? {
        rules.push(Rule::PackageCreditsAtLeast { credits });
    }
    match get("enrolled") {
        "yes" => rules.push(Rule::EnrolledInLoyalty { enrolled: true }),
        "no" => rules.push(Rule::EnrolledInLoyalty { enrolled: false }),
        _ => {}
    }
    Ok(rules)
}

/// The segment the submitted form describes, on top of `base`. The rules of
/// a vertical kind that the editor cannot show are kept.
fn segment_of_form(
    form: &BTreeMap<String, String>,
    lang: &Lang,
    user: &AuthUser,
    base: Segment,
) -> Result<Segment> {
    let mut rules = rules_of_form(form, lang)?;
    rules.extend(
        base.rules()
            .into_iter()
            .filter(|r| matches!(r, Rule::Vertical { .. })),
    );
    let mode = match form.get("match_mode").map_or("", |v| v.trim()) {
        "any" => MatchMode::Any,
        _ => MatchMode::All,
    };
    let mut segment = Segment::of(form.get("name").map_or("", |v| v.as_str()), mode, &rules);
    segment.id = base.id;
    if base.id == 0 {
        segment.created_by = Some(user.id);
    }
    Ok(segment)
}

/// A refusal of the save as the response, under the field it concerns.
fn refused<F: Serialize>(refusal: crate::segment::SegmentRefused, form: &F) -> Error {
    use crate::segment::SegmentRefused as R;
    let field = match refusal {
        R::NameRequired | R::NameTaken => "name",
        R::NoRules | R::BadRule(_) | R::ClinicalItem(_) => "match_mode",
        R::NotFound => return refusal.into_error(),
    };
    invalid(field, refusal.message(), form)
}

/// `POST /segments`: saves a new segment and opens its editor.
async fn store(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let segment = segment_of_form(&form, &lang, &user, Segment::default())?;
    let saved = segment
        .store(&state.db, &config_of(&state))
        .await?
        .map_err(|r| refused(r, &form))?;
    Ok((
        Toast::success(text(&lang, "segments.saved")),
        Redirect::route("wl_sales.segment", &[&saved.id])?,
    )
        .into_response())
}

/// `POST /segments/{id}`: changes a segment.
async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(current): Found<Segment>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let id = current.id;
    let segment = segment_of_form(&form, &lang, &user, current)?;
    segment
        .store(&state.db, &config_of(&state))
        .await?
        .map_err(|r| refused(r, &form))?;
    Ok((
        Toast::success(text(&lang, "segments.saved")),
        Redirect::route("wl_sales.segment", &[&id])?,
    )
        .into_response())
}

/// `POST /segments/{id}/preview`: counts the members now (an estimate) and
/// shows the count in the editor.
async fn preview(
    State(state): State<AppState>,
    lang: Lang,
    Found(segment): Found<Segment>,
) -> Result<Response> {
    let count = segments::preview(&state, &segment).await?;
    let message = texts::t_with(&lang, "wl_sales.segments.previewed", &[("count", &count)]);
    Ok((
        Toast::success(message),
        Redirect::route("wl_sales.segment", &[&segment.id])?,
    )
        .into_response())
}

/// `GET /segments/{id}/export`: the members as a CSV; audited.
async fn export(
    State(state): State<AppState>,
    user: AuthUser,
    Found(segment): Found<Segment>,
) -> Result<renox::Download> {
    let body = segments::export_csv(&state, &segment, user.id).await?;
    Ok(renox::Download::bytes(
        "segment-members.csv",
        "text/csv; charset=utf-8",
        body.into_bytes(),
    ))
}
