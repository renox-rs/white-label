//! The budget category pages (wl-invoicing §3 *Budget category*, §6
//! `budget_categories.manage`):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /budget-categories` | `wl_invoicing.budget_categories` | `budgets.view` |
//! | `GET /budget-categories/new` | `wl_invoicing.budget_category_new` | `budget_categories.manage` |
//! | `POST /budget-categories` | `wl_invoicing.budget_category_create` | `budget_categories.manage` |
//! | `GET /budget-categories/{id}/edit` | `wl_invoicing.budget_category_edit` | `budget_categories.manage` |
//! | `POST /budget-categories/{id}` | `wl_invoicing.budget_category_update` | `budget_categories.manage` |
//!
//! The list shows each category with the keys it collects. A save that
//! repeats a mapping key of another category, gives an owner drawings
//! category no owner party, or changes the kind of a category that has a
//! budget line answers 422 with the reason. The templates are compiled in
//! and can be replaced by the app (`resources/views/wl-invoicing/…`).

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use serde::Serialize;

use crate::budget_category::{
    BudgetCategory, BudgetKind, CategoryInput, mappings_of, parse_mappings, save,
};
use crate::permissions;
use crate::texts;

/// The texts a template reads as `tx['budget.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The module's category routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/budget-categories", index)
                .name("wl_invoicing.budget_categories")
                .require_permission(permissions::BUDGETS_VIEW),
        )
        .merge(
            Routes::new()
                .get("/budget-categories/new", new_form)
                .name("wl_invoicing.budget_category_new")
                .post("/budget-categories", create)
                .name("wl_invoicing.budget_category_create")
                .get("/budget-categories/{id}/edit", edit_form)
                .name("wl_invoicing.budget_category_edit")
                .post("/budget-categories/{id}", update)
                .name("wl_invoicing.budget_category_update")
                .require_permission(permissions::BUDGET_CATEGORIES_MANAGE),
        )
}

/// One category as the list shows it.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct CategoryRow {
    id: i64,
    code: String,
    name: String,
    kind: String,
    parent: String,
    owner_role: String,
    keys: String,
    active: bool,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/budget_categories.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: Vec<CategoryRow>,
    can_manage: bool,
}

/// `GET /budget-categories`: the categories and what they collect.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    let t = |key: &str| texts::t(&lang, &format!("wl_invoicing.{key}"));
    let categories = BudgetCategory::listed(&state.db, true).await?;
    let mut rows = Vec::with_capacity(categories.len());
    for category in &categories {
        let keys = mappings_of(&state.db, category.id)
            .await?
            .iter()
            .map(|m| format!("{}: {}", m.key_kind, m.key_value))
            .collect::<Vec<_>>()
            .join(", ");
        let parent = categories
            .iter()
            .find(|c| Some(c.id) == category.parent_id)
            .map(|c| c.name_en.clone())
            .unwrap_or_default();
        rows.push(CategoryRow {
            id: category.id,
            code: category.code.clone(),
            name: category.name_en.clone(),
            kind: t(&format!("budget.kind.{}", category.kind.as_str())),
            parent,
            owner_role: category.owner_role.clone().unwrap_or_default(),
            keys,
            active: category.active,
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
        can_manage: user.has_permission(permissions::BUDGET_CATEGORIES_MANAGE),
    }))
}

/// What the form needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/budget_category_form.html")]
pub(crate) struct FormPage {
    tx: Texts,
    title: String,
    action: String,
    cancel: String,
    kinds: Vec<(String, String)>,
    values: BTreeMap<&'static str, String>,
    editing: bool,
}

/// The kinds a category can have, with their labels.
fn kinds(lang: &Lang) -> Vec<(String, String)> {
    BudgetKind::ALL
        .iter()
        .map(|k| {
            (
                k.as_str().to_owned(),
                texts::t(lang, &format!("wl_invoicing.budget.kind.{}", k.as_str())),
            )
        })
        .collect()
}

/// The form page for a new category (`category` is `None`) or to change one.
async fn form_page(
    state: &AppState,
    lang: &Lang,
    category: Option<&BudgetCategory>,
) -> Result<View> {
    let t = |key: &str| texts::t(lang, &format!("wl_invoicing.{key}"));
    let mut values: BTreeMap<&'static str, String> = BTreeMap::new();
    values.insert("kind", BudgetKind::Expense.as_str().to_owned());
    values.insert("active", "1".to_owned());
    if let Some(category) = category {
        values.insert("code", category.code.clone());
        values.insert("name_en", category.name_en.clone());
        values.insert("name_ar", category.name_ar.clone().unwrap_or_default());
        values.insert("kind", category.kind.as_str().to_owned());
        let parent = match category.parent_id {
            Some(id) => BudgetCategory::find(&state.db, id).await?.map(|c| c.code),
            None => None,
        };
        values.insert("parent_code", parent.unwrap_or_default());
        values.insert(
            "owner_role",
            category.owner_role.clone().unwrap_or_default(),
        );
        values.insert(
            "owner_party_id",
            category
                .owner_party_id
                .map(|id| id.to_string())
                .unwrap_or_default(),
        );
        values.insert(
            "mappings",
            mappings_of(&state.db, category.id)
                .await?
                .iter()
                .map(|m| format!("{}: {}", m.key_kind, m.key_value))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        values.insert("active", if category.active { "1" } else { "" }.to_owned());
    }
    let (title, action) = match category {
        Some(c) => (
            t("budget.category.edit_title"),
            format!("/budget-categories/{}", c.id),
        ),
        None => (
            t("budget.category.new_title"),
            "/budget-categories".to_owned(),
        ),
    };
    Ok(View::typed(FormPage {
        tx: texts::all(lang),
        title,
        action,
        cancel: "/budget-categories".to_owned(),
        kinds: kinds(lang),
        values,
        editing: category.is_some(),
    }))
}

/// `GET /budget-categories/new`.
async fn new_form(State(state): State<AppState>, lang: Lang) -> Result<View> {
    form_page(&state, &lang, None).await
}

/// `GET /budget-categories/{id}/edit`.
async fn edit_form(
    State(state): State<AppState>,
    lang: Lang,
    Found(category): Found<BudgetCategory>,
) -> Result<View> {
    form_page(&state, &lang, Some(&category)).await
}

/// A 422 with `message` under `field`, keeping the input.
fn invalid<F: Serialize>(field: &str, message: String, form: &F) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// A service refusal as the response: a 422 message goes under `base`.
fn on_base<F: Serialize>(error: Error, form: &F) -> Error {
    match error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message) => invalid("base", message, form),
        other => other,
    }
}

/// The category a submitted form describes.
fn input_of(form: &BTreeMap<String, String>, code: Option<&str>) -> Result<CategoryInput> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let kind = get("kind")
        .parse::<BudgetKind>()
        .map_err(|_| {
            invalid(
                "kind",
                texts::english("wl_invoicing.budget.errors.kind").to_owned(),
                form,
            )
        })?;
    let owner_party_id = match get("owner_party_id") {
        "" => None,
        id => Some(
            id.parse::<i64>().map_err(|_| {
                invalid(
                    "owner_party_id",
                    texts::english("wl_invoicing.budget.errors.party").to_owned(),
                    form,
                )
            })?,
        ),
    };
    let mappings = parse_mappings(get("mappings")).map_err(|e| on_base(e, form))?;
    let not_blank = |text: &str| (!text.is_empty()).then(|| text.to_owned());
    Ok(CategoryInput {
        code: code.unwrap_or_else(|| get("code")).to_owned(),
        name_en: get("name_en").to_owned(),
        name_ar: not_blank(get("name_ar")),
        kind,
        parent_code: not_blank(get("parent_code")),
        owner_role: not_blank(get("owner_role")),
        owner_party_id,
        mappings,
        active: form.contains_key("active"),
    })
}

/// `POST /budget-categories`: saves a new category and goes to the list.
async fn create(
    State(state): State<AppState>,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let input = input_of(&form, None)?;
    save(&state.db, None, input)
        .await
        .map_err(|e| on_base(e, &form))?;
    Ok((
        Toast::success(texts::t(&lang, "wl_invoicing.budget.category.saved")),
        Redirect::route("wl_invoicing.budget_categories", &[])?,
    )
        .into_response())
}

/// `POST /budget-categories/{id}`: saves the change and goes to the list.
async fn update(
    State(state): State<AppState>,
    lang: Lang,
    Found(category): Found<BudgetCategory>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let input = input_of(&form, Some(&category.code))?;
    save(&state.db, Some(category.id), input)
        .await
        .map_err(|e| on_base(e, &form))?;
    Ok((
        Toast::success(texts::t(&lang, "wl_invoicing.budget.category.saved")),
        Redirect::route("wl_invoicing.budget_categories", &[])?,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn the_form_becomes_a_category_input() {
        let input = input_of(
            &form(&[
                ("code", "Fuel"),
                ("name_en", " Fuel "),
                ("kind", "expense"),
                ("mappings", "expense_category: fuel\ncost_code: fuel"),
                ("active", "on"),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(input.code, "Fuel");
        assert_eq!(input.name_en, "Fuel");
        assert_eq!(input.kind, BudgetKind::Expense);
        assert_eq!(input.mappings.len(), 2);
        assert!(input.active);
        assert_eq!(input.owner_party_id, None);
        let unticked = input_of(
            &form(&[("name_en", "x"), ("kind", "income"), ("owner_party_id", "7")]),
            Some("kept"),
        )
        .unwrap();
        assert_eq!(unticked.code, "kept");
        assert!(!unticked.active);
        assert_eq!(unticked.owner_party_id, Some(7));
    }

    #[test]
    fn a_bad_kind_party_or_mapping_is_refused() {
        assert!(input_of(&form(&[("kind", "other")]), None).is_err());
        assert!(
            input_of(
                &form(&[("kind", "income"), ("owner_party_id", "x")]),
                None
            )
            .is_err()
        );
        assert!(
            input_of(
                &form(&[("kind", "income"), ("mappings", "no colon")]),
                None
            )
            .is_err()
        );
    }
}
