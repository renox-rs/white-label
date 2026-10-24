//! The labour menu page (08 `AUT.JOB.03`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /auto/labour-menu` | `auto.labour_menu` | `labour_menu.manage` or `job_cards.view` |
//! | `POST /auto/labour-menu` | `auto.labour_menu_create` | `labour_menu.manage` |
//! | `POST /auto/labour-menu/{id}/active` | `auto.labour_menu_active` | `labour_menu.manage` |
//!
//! The menu lists the flat-rate and hourly operations an estimate is drafted
//! from, with the standard hours and the price.

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::Value;
use serde::{Deserialize, Serialize};

use super::lists;
use super::{Texts, parse_milli, parse_money, text};
use crate::labour_line::LabourPricing;
use crate::labour_menu::{self, NewOperation};
use crate::labour_operation::LabourOperation;
use crate::permissions;
use crate::texts;

/// The routes of the page.
pub fn routes() -> Routes {
    Routes::new()
        .get("/auto/labour-menu", index)
        .name("auto.labour_menu")
        .post("/auto/labour-menu", create)
        .name("auto.labour_menu_create")
        .post("/auto/labour-menu/{id}/active", set_active)
        .name("auto.labour_menu_active")
        .require_auth()
}

/// The labour menu grid: the code first and frozen, the name, hourly or
/// flat-rate, the standard hours, the rate or flat price in dirhams and
/// whether it is on the menu; searched by code and name.
pub(crate) fn grid(t: lists::Text<'_>) -> Grid {
    let pricing = lists::options(
        t,
        "jobs.pricing",
        [LabourPricing::FlatRate, LabourPricing::Hourly]
            .iter()
            .map(|p| p.as_str()),
    );
    let standing = vec![
        ("on".to_owned(), t("labour.on")),
        ("off".to_owned(), t("labour.off")),
    ];
    lists::baseline(Grid::new("auto-labour-menu"))
        .title(&t("labour.title"))
        .column(
            Column::text("code", &t("labour.code"))
                .frozen()
                .mobile()
                .searchable(),
        )
        .column(
            Column::text("name_en", &t("labour.name"))
                .searchable()
                .mobile(),
        )
        .column(Column::select("pricing", &t("labour.pricing"), pricing))
        .column(
            Column::number("hours", &t("labour.hours"))
                .computed("{T}.\"hours_milli\" / 1000.0")
                .decimals(1),
        )
        .column(lists::money("rate", &t("labour.rate")).mobile())
        .column(
            Column::select("standing", &t("fields.status"), standing)
                .computed("CASE WHEN {T}.\"active\" THEN 'on' ELSE 'off' END")
                .badges(&[("on", "success"), ("off", "neutral")])
                .mobile(),
        )
        .column(Column::custom("actions", "").frozen_right())
        .sort_by("code")
        .cards_on_mobile()
        .empty_state(&t("labour.empty"), Some(&t("labour.empty_hint")))
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/labour_menu.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    operations: GridPage<LabourOperation>,
    /// The operations of the page, `[id, name, on the menu]`, for the sheets
    /// that take one off the menu or put it back.
    sheets: Vec<(i64, String, bool)>,
    pricing_options: Vec<[String; 2]>,
    can_manage: bool,
}

/// `GET /auto/labour-menu`.
async fn index(user: AuthUser, lang: Lang, request: GridRequest) -> Result<Response> {
    if !(user.has_permission(permissions::LABOUR_MENU_MANAGE)
        || user.has_permission(permissions::JOB_CARDS_VIEW))
    {
        return Err(Error::Forbidden);
    }
    let can_export = user.has_permission(permissions::LABOUR_MENU_MANAGE);
    let grid = lists::exporting(grid(&lists::texts_in(&lang)), can_export);
    if let Some(file) = lists::export_file(&grid, can_export, LabourOperation::query(), &request).await? {
        return Ok(file);
    }
    let operations = grid.page(LabourOperation::query(), &request).await?;
    let sheets = operations
        .items()
        .iter()
        .map(|op| (op.id, op.name_en.clone(), op.active))
        .collect();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        operations: lists::with_currency(operations, |_| Value::Null),
        sheets,
        pricing_options: [LabourPricing::FlatRate, LabourPricing::Hourly]
            .into_iter()
            .map(|p| {
                [
                    p.as_str().to_owned(),
                    text(&lang, &format!("jobs.pricing.{}", p.as_str())),
                ]
            })
            .collect(),
        can_manage: user.has_permission(permissions::LABOUR_MENU_MANAGE),
    })
    .into_response())
}

/// The form of an operation.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct OperationForm {
    /// The code.
    #[serde(default)]
    pub code: String,
    /// The name in English.
    #[serde(default)]
    pub name_en: String,
    /// The name in Arabic.
    #[serde(default)]
    pub name_ar: String,
    /// `flat_rate` or `hourly`.
    #[serde(default)]
    pub pricing: String,
    /// The standard hours of a flat-rate operation.
    #[serde(default)]
    pub hours: String,
    /// The hourly rate or the flat price in dirhams.
    #[serde(default)]
    pub rate: String,
}

/// `POST /auto/labour-menu`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<OperationForm>,
) -> Result<Response> {
    let rate = parse_money(&form.rate)?;
    let mut new = if form.pricing.trim() == "hourly" {
        NewOperation::hourly(&form.code, &form.name_en, rate)
    } else {
        NewOperation::flat(&form.code, &form.name_en, parse_milli(&form.hours)?, rate)
    };
    if !form.name_ar.trim().is_empty() {
        new.name_ar = Some(form.name_ar.clone());
    }
    labour_menu::create(&state, &user, new).await?;
    Ok((
        Toast::success(text(&lang, "labour.added")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that switches an operation.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ActiveForm {
    /// `1` to put it on the menu, anything else to take it off.
    #[serde(default)]
    pub on: String,
}

/// `POST /auto/labour-menu/{id}/active`.
async fn set_active(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ActiveForm>,
) -> Result<Response> {
    labour_menu::set_active(&state, &user, id, form.on.trim() == "1").await?;
    Ok((
        Toast::success(text(&lang, "labour.saved")),
        renox::HxRefresh,
    )
        .into_response())
}
