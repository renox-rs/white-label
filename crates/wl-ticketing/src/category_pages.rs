//! The ticket category settings pages (wl-ticketing §7):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /ticket-categories` | `wl_ticketing.categories` | `tickets.manage` |
//! | `POST /ticket-categories/{key}` | `wl_ticketing.category_rename` | `tickets.manage` (fields `name_en`, `name_ar`) |
//! | `POST /ticket-categories/{key}/active` | `wl_ticketing.category_active` | `tickets.manage` (field `active`: `1` on, else off; a platform category stays on) |
//! | `DELETE /ticket-categories/{key}` | `wl_ticketing.category_destroy` | `tickets.manage` (a platform category or one with tickets is refused, 422) |
//!
//! The templates are compiled in and can be replaced by the app
//! (`resources/views/wl-ticketing/categories.html`).

use std::collections::BTreeMap;

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::categories;
use crate::config::Category;
use crate::permissions;
use crate::texts;

/// The texts a template reads as `tx['categories.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes, behind `tickets.manage`.
pub fn routes() -> Routes {
    Routes::new()
        .get("/ticket-categories", index)
        .name("wl_ticketing.categories")
        .post("/ticket-categories/{key}", rename)
        .name("wl_ticketing.category_rename")
        .post("/ticket-categories/{key}/active", set_active)
        .name("wl_ticketing.category_active")
        .delete("/ticket-categories/{key}", destroy)
        .name("wl_ticketing.category_destroy")
        .require_permission(permissions::MANAGE)
}

/// One category as the list shows it.
#[derive(Serialize, Debug, Clone)]
pub struct CategoryRow {
    /// The fixed key.
    pub key: String,
    /// The English name.
    pub name_en: String,
    /// The Arabic name.
    pub name_ar: String,
    /// Whether the platform owns it.
    pub platform: bool,
    /// Whether it is confidential.
    pub confidential: bool,
    /// Whether new tickets can use it.
    pub active: bool,
    /// The role that looks after its tickets, if one does.
    pub assignee_role: String,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-ticketing/categories.html")]
pub(crate) struct CategoriesPage {
    tx: Texts,
    categories: Vec<CategoryRow>,
    /// `(value, label)` pairs for the status select: `1` on, `0` off.
    status_options: Vec<(String, String)>,
}

/// `GET /ticket-categories`: every category, with rename, turn off and
/// delete (not for a platform category).
async fn index(State(db): State<Db>, lang: Lang) -> Result<View> {
    let rows = Category::query()
        .order_by(Category::KEY)
        .get(&db)
        .await?
        .into_iter()
        .map(|c| CategoryRow {
            key: c.key,
            name_en: c.name_en,
            name_ar: c.name_ar,
            platform: c.platform,
            confidential: c.confidential,
            active: c.active,
            assignee_role: c.default_assignee_role.unwrap_or_default(),
        })
        .collect();
    Ok(View::typed(CategoriesPage {
        tx: texts::all(&lang),
        categories: rows,
        status_options: vec![
            ("1".to_owned(), texts::t(&lang, "wl_ticketing.categories.on")),
            ("0".to_owned(), texts::t(&lang, "wl_ticketing.categories.off")),
        ],
    }))
}

/// The rename form.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct RenameForm {
    /// The English name.
    #[validate(required, max = 120)]
    pub name_en: String,
    /// The Arabic name.
    #[validate(max = 120)]
    pub name_ar: Option<String>,
}

/// `POST /ticket-categories/{key}`: renames the category and reloads.
async fn rename(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    Path(key): Path<String>,
    Valid(form): Valid<RenameForm>,
) -> Result<Response> {
    categories::rename(
        &db,
        &user,
        &key,
        &form.name_en,
        form.name_ar.as_deref().unwrap_or_default(),
    )
    .await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_ticketing.categories.renamed")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The turn-on/off form.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ActiveForm {
    /// `1` turns the category on; anything else turns it off.
    #[serde(default)]
    pub active: String,
}

/// `POST /ticket-categories/{key}/active`: turns the category on or off.
async fn set_active(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    Path(key): Path<String>,
    Valid(form): Valid<ActiveForm>,
) -> Result<Response> {
    categories::set_active(&db, &user, &key, form.active == "1").await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_ticketing.categories.updated")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `DELETE /ticket-categories/{key}`: deletes the category and goes back.
async fn destroy(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    back: Back,
    Path(key): Path<String>,
) -> Result<Response> {
    categories::delete(&db, &user, &key).await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_ticketing.categories.deleted")),
        back,
    )
        .into_response())
}
