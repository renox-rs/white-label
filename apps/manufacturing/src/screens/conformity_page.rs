//! The conformity flags of the items (`GET /manufacturing/conformity`,
//! BL-MFG-11, C-02): which items are regulated (they need a conformity
//! certificate to leave the plant), which are export only, which certificate
//! kinds count for each, and the certificates on file with their expiry. The
//! sheet *Flag an item* names the item by its code.
//!
//! The list is the standard grid over the flagged items (search on the item's
//! code and name, filters on the three flags); the item and its certificates
//! are custom cells.

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;
use wl_kernel::Subject;
use wl_kernel::documents::{guard, subject_text, today};
use wl_kernel::inventory::{ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE, stock_service};

use super::{Texts, date_text, done, exported, with_item_search};
use crate::conformity::{self, FlagAsk, ItemFlag, KINDS};
use crate::db::refused;
use crate::permissions;
use crate::texts::{english, screen_texts, t, word};

/// The address of the page.
pub(crate) const URL: &str = "/manufacturing/conformity";

/// A certificate on file.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Certificate {
    kind: String,
    number: String,
    valid_to: String,
    validity: String,
    validity_kind: &'static str,
}

/// An item with its flags.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Row {
    item: String,
    regulated: String,
    export_only: String,
    customer_specific: String,
    accepted: String,
    certificates: Vec<Certificate>,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/conformity.html")]
pub(crate) struct ConformityPage {
    tx: Texts,
    rows: GridPage<ItemFlag>,
    can_flag: bool,
    kinds: Vec<(String, String)>,
}

/// The flagged items grid. The item and its certificates are custom cells.
pub(crate) fn grid(lang: &Lang) -> Grid {
    let tx = |key: &str| t(lang, &format!("manufacturing.screen.conformity.{key}"));
    let grid = Grid::new("manufacturing-conformity")
        .title(&tx("title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(Column::custom("item", &tx("item")).frozen().mobile())
        .column(Column::bool("regulated", &tx("regulated")).mobile())
        .column(Column::bool("export_only", &tx("export_only")).mobile())
        .column(Column::bool("customer_specific", &tx("customer_specific")))
        .column(Column::custom("accepted", &tx("accepted")).hidden())
        .column(Column::custom("certificates", &tx("certificates")))
        .sort_by("item_id")
        .cards_on_mobile()
        .exports()
        .empty_state(&tx("empty"), Some(&tx("empty_text")));
    with_item_search(grid, lang, "item_id", false)
}

fn validity_kind(validity: &str) -> &'static str {
    match validity {
        "valid" => "success",
        "expiring" | "pending" => "warning",
        _ => "error",
    }
}

/// The flags that make an item listed: any of the three.
fn flagged() -> renox::db::Query<ItemFlag> {
    ItemFlag::query().where_any(|q| {
        q.where_(ItemFlag::REGULATED.eq(true))
            .where_(ItemFlag::EXPORT_ONLY.eq(true))
            .where_(ItemFlag::CUSTOMER_SPECIFIC.eq(true))
    })
}

/// The certificates on file of a flagged item.
async fn certificates_of(
    state: &AppState,
    lang: &Lang,
    flag: &ItemFlag,
) -> Result<Vec<Certificate>> {
    let subject = flag.subject();
    let mut certificates = Vec::new();
    for line in guard(state)
        .documents_of(&state.db, &subject, today(), &lang.locale)
        .await?
    {
        certificates.push(Certificate {
            kind: line.kind,
            number: line.number.unwrap_or_default(),
            valid_to: line
                .valid_to
                .map_or_else(|| t(lang, "manufacturing.screen.not_set"), date_text),
            validity: word(lang, "doc_validity", &line.validity),
            validity_kind: validity_kind(&line.validity),
        });
    }
    Ok(certificates)
}

/// The kinds a regulated item accepts, in words.
fn accepted_text(lang: &Lang, flag: &ItemFlag) -> String {
    if flag.regulated {
        flag.accepted()
            .iter()
            .map(|kind| word(lang, "conformity_kind", kind))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        String::new()
    }
}

/// `GET /manufacturing/conformity`.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    request: GridRequest,
) -> Result<Response> {
    let can_flag = conformity::may_flag(&user);
    if !can_flag
        && !user.has_permission_anywhere(permissions::WORK_ORDERS_VIEW)
        && !user.has_permission_anywhere(permissions::NCRS_VIEW)
    {
        return Err(Error::Forbidden);
    }
    let yes = t(&lang, "manufacturing.screen.conformity.yes");
    let no = t(&lang, "manufacturing.screen.conformity.no");
    let word_of = |on: bool| if on { yes.clone() } else { no.clone() };
    if !wl_kernel::negotiate::wants_html(&headers) {
        let mut rows = Vec::new();
        for flag in flagged().order_by(ItemFlag::ITEM_ID).get(&state.db).await? {
            rows.push(Row {
                item: subject_text(&state.db, &flag.subject()).await?,
                regulated: word_of(flag.regulated),
                export_only: word_of(flag.export_only),
                customer_specific: word_of(flag.customer_specific),
                accepted: accepted_text(&lang, &flag),
                certificates: certificates_of(&state, &lang, &flag).await?,
            });
        }
        return Ok(Json(&rows).into_response());
    }
    let grid = grid(&lang);
    let query = flagged();
    if let Some(file) = exported(&grid, &query, &request, true).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra = std::collections::HashMap::new();
    for flag in page.items() {
        extra.insert(
            flag.id,
            json!({
                "item": subject_text(&state.db, &flag.subject()).await?,
                "accepted": accepted_text(&lang, flag),
                "certificates": certificates_of(&state, &lang, flag).await?,
            }),
        );
    }
    let page = page.extend(|flag| extra.get(&flag.id).cloned().unwrap_or_else(|| json!({})));
    Ok(View::typed(ConformityPage {
        tx: screen_texts(&lang),
        rows: page,
        can_flag,
        kinds: KINDS
            .iter()
            .map(|kind| ((*kind).to_owned(), word(&lang, "conformity_kind", kind)))
            .collect(),
    })
    .into_response())
}

/// `POST /manufacturing/conformity`: flags the item named by its code. The
/// kinds are the form's `kind` fields (none: all three).
pub(crate) async fn flag(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<Vec<(String, String)>>,
) -> Result<Response> {
    let field = |name: &str| {
        form.iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.trim().to_owned())
            .unwrap_or_default()
    };
    let checked = |name: &str| matches!(field(name).as_str(), "1" | "on" | "true");
    let code = field("code");
    if code.is_empty() {
        return Err(refused(english("manufacturing.conformity.code", &[])));
    }
    let item = stock_service(&state)
        .choices(&state, &code, &Subject::new(LOCATION_SUBJECT_TYPE, 0))
        .await?
        .ok_or_else(|| {
            refused(english(
                "manufacturing.conformity.unknown_item",
                &[("code", &code)],
            ))
        })?;
    let kinds: Vec<String> = form
        .iter()
        .filter(|(key, _)| key == "kind")
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect();
    debug_assert_eq!(item.item.subject_type, ITEM_SUBJECT_TYPE);
    conformity::set_flags(
        &state,
        &user,
        item.item.subject_id,
        FlagAsk {
            regulated: checked("regulated"),
            export_only: checked("export_only"),
            customer_specific: checked("customer_specific"),
            kinds,
        },
    )
    .await?;
    Ok(done(&lang, URL, "manufacturing.screen.conformity.done"))
}
