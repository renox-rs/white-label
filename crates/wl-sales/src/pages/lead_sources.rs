//! The lead sources page (wl-sales §3, #138 A5): list, add, rename,
//! activate and deactivate.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /sales/lead-sources` | `wl_sales.lead_sources` | `campaigns.manage` |
//! | `POST /sales/lead-sources` | `wl_sales.lead_sources.store` | `campaigns.manage` |
//! | `POST /sales/lead-sources/{id}` | `wl_sales.lead_sources.update` | `campaigns.manage` |
//! | `POST /sales/lead-sources/{id}/activate` | `wl_sales.lead_sources.activate` | `campaigns.manage` |
//! | `POST /sales/lead-sources/{id}/deactivate` | `wl_sales.lead_sources.deactivate` | `campaigns.manage` |
//!
//! There is no delete route: a source is switched off, and
//! [`LeadSource::delete_unused`] is the service for the rest.

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};

use crate::lead_source::{LeadSource, LeadSourceRefused};
use crate::permissions;
use crate::texts;

/// The texts a template reads as `tx['fields.code']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the page, behind `campaigns.manage`.
pub fn routes() -> Routes {
    Routes::new()
        .get("/sales/lead-sources", index)
        .name("wl_sales.lead_sources")
        .post("/sales/lead-sources", store)
        .name("wl_sales.lead_sources.store")
        .post("/sales/lead-sources/{id}", update)
        .name("wl_sales.lead_sources.update")
        .post("/sales/lead-sources/{id}/activate", activate)
        .name("wl_sales.lead_sources.activate")
        .post("/sales/lead-sources/{id}/deactivate", deactivate)
        .name("wl_sales.lead_sources.deactivate")
        .require_permission(permissions::CAMPAIGNS_MANAGE)
}

/// The package text `key` (without the `wl_sales.lead_sources.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_sales.lead_sources.{key}"))
}

/// A 422 with `message` under `field`, keeping what was typed.
fn invalid(field: &str, message: String, input: &impl Serialize) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    ValidationError::new(errors).with_input(input).into()
}

/// One source as the list shows it.
#[derive(Serialize, Debug, Clone)]
pub struct SourceRow {
    /// The primary key.
    pub id: i64,
    /// The code.
    pub code: String,
    /// The name in the page's language.
    pub name: String,
    /// The English name, for the rename form.
    pub name_en: String,
    /// The Arabic name, for the rename form.
    pub name_ar: String,
    /// Whether it can be chosen.
    pub active: bool,
    /// Whether something names it.
    pub in_use: bool,
}

/// The lead sources grid: search on the names and the code, filter by
/// status. The name's "in use" mark and the row buttons are drawn by the
/// template (`custom`).
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let f = |key: &str| texts::t(lang, &format!("wl_sales.fields.{key}"));
    let grid = Grid::new("sales-lead-sources")
        .title(&text(lang, "title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("name_en", &f("name_en"))
                .frozen()
                .mobile()
                .searchable(),
        )
        .column(Column::text("name_ar", &f("name_ar")).hidden().searchable())
        .column(Column::text("code", &f("code")).mobile().searchable())
        .column(Column::custom("in_use", &text(lang, "in_use")))
        .column(Column::bool("active", &f("status")).icons().mobile())
        .column(Column::datetime("created_at", &text(lang, "created_at")).hidden())
        .column(Column::custom("actions", ""))
        .sort_by("code")
        .cards_on_mobile()
        .empty_state(&text(lang, "empty"), Some(&text(lang, "empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/lead_sources.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    sources: GridPage<LeadSource>,
    /// The sources of the page, for their rename and switch sheets.
    sheets: Vec<SourceRow>,
}

/// `GET /sales/lead-sources`: every source with its actions (`?export=`).
async fn index(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let can_export = user.has_permission(permissions::CAMPAIGNS_MANAGE);
    let grid = grid(&lang, can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    let query = LeadSource::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut sheets = Vec::new();
    for source in page.items() {
        sheets.push(SourceRow {
            id: source.id,
            code: source.code.clone(),
            name: source.name(&lang.locale).to_owned(),
            name_en: source.name_en.clone(),
            name_ar: source.name_ar.clone(),
            active: source.active,
            in_use: source.in_use(&db).await?,
        });
    }
    let mut in_use = sheets.iter().map(|row| row.in_use);
    let sources = page.extend(|_| json!({ "in_use": in_use.next().unwrap_or(false) }));
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        sources,
        sheets,
    })
    .into_response())
}

/// The add form.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct AddForm {
    /// The code.
    #[validate(required, max = 64)]
    pub code: String,
    /// The English name.
    #[validate(required, max = 120)]
    pub name_en: String,
    /// The Arabic name.
    #[validate(max = 120)]
    #[serde(default)]
    pub name_ar: String,
}

/// `POST /sales/lead-sources`: adds a source and reloads the page.
async fn store(State(db): State<Db>, lang: Lang, Valid(form): Valid<AddForm>) -> Result<Response> {
    match LeadSource::add(&db, &form.code, &form.name_en, &form.name_ar).await? {
        Ok(_) => Ok((Toast::success(text(&lang, "added")), renox::HxRefresh).into_response()),
        Err(refusal) => {
            let (field, key) = match refusal {
                LeadSourceRefused::BadCode => ("code", "bad_code"),
                LeadSourceRefused::CodeTaken => ("code", "code_taken"),
                LeadSourceRefused::NameRequired => ("name_en", "name_required"),
                LeadSourceRefused::InUse => ("code", "in_use_refused"),
            };
            Err(invalid(field, text(&lang, key), &form))
        }
    }
}

/// The rename form.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct RenameForm {
    /// The English name.
    #[validate(required, max = 120)]
    pub name_en: String,
    /// The Arabic name.
    #[validate(max = 120)]
    #[serde(default)]
    pub name_ar: String,
}

/// The source `id`, or a 404.
async fn find(db: &Db, id: i64) -> Result<LeadSource> {
    LeadSource::find(db, id).await?.ok_or(Error::NotFound)
}

/// `POST /sales/lead-sources/{id}`: renames a source.
async fn update(
    State(db): State<Db>,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<RenameForm>,
) -> Result<Response> {
    let mut source = find(&db, id).await?;
    if form.name_en.trim().is_empty() {
        return Err(invalid("name_en", text(&lang, "name_required"), &form));
    }
    source.name_en = form.name_en.trim().to_owned();
    source.name_ar = form.name_ar.trim().to_owned();
    source.save(&db).await?;
    Ok((Toast::success(text(&lang, "renamed")), renox::HxRefresh).into_response())
}

/// Sets `active` on source `id` and reloads the page.
async fn set_active(db: &Db, lang: &Lang, id: i64, active: bool) -> Result<Response> {
    let mut source = find(db, id).await?;
    source.active = active;
    source.save(db).await?;
    let key = if active { "activated" } else { "deactivated" };
    Ok((Toast::success(text(lang, key)), renox::HxRefresh).into_response())
}

/// `POST /sales/lead-sources/{id}/activate`.
async fn activate(State(db): State<Db>, lang: Lang, Path(id): Path<i64>) -> Result<Response> {
    set_active(&db, &lang, id, true).await
}

/// `POST /sales/lead-sources/{id}/deactivate`.
async fn deactivate(State(db): State<Db>, lang: Lang, Path(id): Path<i64>) -> Result<Response> {
    set_active(&db, &lang, id, false).await
}
