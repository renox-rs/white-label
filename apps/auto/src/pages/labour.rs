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

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Texts, parse_milli, parse_money, text};
use crate::labour_line::{LabourPricing, hours_text};
use crate::labour_menu::{self, NewOperation};
use crate::money_text;
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

/// One operation of the menu.
#[derive(Serialize, Debug, Clone)]
pub struct OperationRow {
    /// The primary key.
    pub id: i64,
    /// The code.
    pub code: String,
    /// The name.
    pub name: String,
    /// Hourly or flat-rate, in words.
    pub pricing: String,
    /// The standard hours of a flat-rate operation, or a dash.
    pub hours: String,
    /// The hourly rate or the flat price.
    pub rate: String,
    /// Whether it is on the menu, in words.
    pub active: String,
    /// Whether it is on the menu.
    pub is_active: bool,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/labour_menu.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: Vec<OperationRow>,
    pricing_options: Vec<[String; 2]>,
    can_manage: bool,
}

/// `GET /auto/labour-menu`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    if !(user.has_permission(permissions::LABOUR_MENU_MANAGE)
        || user.has_permission(permissions::JOB_CARDS_VIEW))
    {
        return Err(Error::Forbidden);
    }
    let rows = labour_menu::menu(&state.db)
        .await?
        .into_iter()
        .map(|op| OperationRow {
            id: op.id,
            code: op.code.clone(),
            name: op.name_en.clone(),
            pricing: text(&lang, &format!("jobs.pricing.{}", op.pricing.as_str())),
            hours: if op.pricing == LabourPricing::FlatRate {
                format!("{} h", hours_text(op.hours_milli))
            } else {
                text(&lang, "none")
            },
            rate: money_text(op.rate),
            active: if op.active {
                text(&lang, "labour.on")
            } else {
                text(&lang, "labour.off")
            },
            is_active: op.active,
        })
        .collect();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
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
    }))
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
