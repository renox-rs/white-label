//! The conformity flags of the items (`GET /manufacturing/conformity`,
//! BL-MFG-11, C-02): which items are regulated (they need a conformity
//! certificate to leave the plant), which are export only, which certificate
//! kinds count for each, and the certificates on file with their expiry. The
//! sheet *Flag an item* names the item by its code. The page is a
//! `renox::grid` filtered on the three flags (yes or no) and searched by the
//! item's number; the item's name and the certificates come from the stock
//! and document modules, so they are drawn per row. JSON keeps answering the
//! rows in words.

use std::collections::HashMap;

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::Serialize;
use wl_kernel::Subject;
use wl_kernel::documents::{guard, subject_text, today};
use wl_kernel::inventory::{ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE, stock_service};

use super::{Texts, date_text, done, grids};
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
    flags: GridPage<ItemFlag>,
    can_flag: bool,
    kinds: Vec<(String, String)>,
}

/// The items with a flag: regulated, export only or made for one customer.
fn flagged() -> renox::db::Query<ItemFlag> {
    ItemFlag::query().where_any(|q| {
        q.where_eq("regulated", true)
            .where_eq("export_only", true)
            .where_eq("customer_specific", true)
    })
}

/// The conformity grid: the item first and frozen, its number (hidden,
/// searched), the three flags as yes or no, the certificate kinds that count
/// and the certificates on file.
pub(crate) fn grid(lang: &Lang) -> Grid {
    let text = |key: &str| t(lang, &format!("manufacturing.screen.{key}"));
    grids::baseline(Grid::new("manufacturing-conformity"))
        .title(&text("conformity.title"))
        .column(
            Column::custom("item", &text("conformity.item"))
                .frozen()
                .mobile(),
        )
        .column(
            Column::text("item_number", &text("grid.item_number"))
                .computed("CAST({T}.\"item_id\" AS TEXT)")
                .searchable()
                .hidden(),
        )
        .column(Column::bool("regulated", &text("conformity.regulated")).mobile())
        .column(Column::bool("export_only", &text("conformity.export_only")).mobile())
        .column(Column::bool(
            "customer_specific",
            &text("conformity.customer_specific"),
        ))
        .column(Column::custom("accepted", &text("grid.accepted")))
        .column(Column::custom(
            "certificates",
            &text("conformity.certificates"),
        ))
        .sort_by("item_id")
        .empty_state(
            &text("conformity.empty"),
            Some(&text("conformity.empty_text")),
        )
}

fn validity_kind(validity: &str) -> &'static str {
    match validity {
        "valid" => "success",
        "expiring" | "pending" => "warning",
        _ => "error",
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
    let kinds = KINDS
        .iter()
        .map(|kind| ((*kind).to_owned(), word(&lang, "conformity_kind", kind)))
        .collect();
    if !wl_kernel::negotiate::wants_html(&headers) {
        let mut rows = Vec::new();
        for flag in flagged().order_by(ItemFlag::ITEM_ID).get(&state.db).await? {
            rows.push(row(&state, &lang, &flag).await?);
        }
        return Ok(Json(&rows).into_response());
    }
    // Who may see the flags may export them; the item's name and the
    // certificates are custom columns and stay on screen.
    let grid = grids::exporting(grid(&lang), true);
    if let Some(file) = grids::export_file(&grid, true, flagged(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(flagged(), &request).await?;
    let mut extra: HashMap<i64, Value> = HashMap::new();
    for flag in page.items() {
        let found = row(&state, &lang, flag).await?;
        extra.insert(
            flag.id,
            json!({
                "item": found.item,
                "accepted": found.accepted,
                "certificates": found.certificates,
            }),
        );
    }
    let flags = page.extend(|flag| extra.get(&flag.id).cloned().unwrap_or(Value::Null));
    Ok(View::typed(ConformityPage {
        tx: screen_texts(&lang),
        flags,
        can_flag,
        kinds,
    })
    .into_response())
}

/// An item's flags in words, for the JSON answer and the grid's custom cells.
async fn row(state: &AppState, lang: &Lang, flag: &ItemFlag) -> Result<Row> {
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
    let yes_no = |on: bool| {
        t(
            lang,
            if on {
                "manufacturing.screen.conformity.yes"
            } else {
                "manufacturing.screen.conformity.no"
            },
        )
    };
    Ok(Row {
        item: subject_text(&state.db, &subject).await?,
        regulated: yes_no(flag.regulated),
        export_only: yes_no(flag.export_only),
        customer_specific: yes_no(flag.customer_specific),
        accepted: if flag.regulated {
            flag.accepted()
                .iter()
                .map(|kind| word(lang, "conformity_kind", kind))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            String::new()
        },
        certificates,
    })
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
