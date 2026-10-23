//! The conformity flags of the items (`GET /manufacturing/conformity`,
//! BL-MFG-11, C-02): which items are regulated (they need a conformity
//! certificate to leave the plant), which are export only, which certificate
//! kinds count for each, and the certificates on file with their expiry. The
//! sheet *Flag an item* names the item by its code.

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::Subject;
use wl_kernel::documents::{guard, subject_text, today};
use wl_kernel::inventory::{ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE, stock_service};

use super::{Texts, date_text, done};
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
    accepted: String,
    certificates: Vec<Certificate>,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/conformity.html")]
pub(crate) struct ConformityPage {
    tx: Texts,
    rows: Vec<Row>,
    can_flag: bool,
    kinds: Vec<(String, String)>,
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
) -> Result<Response> {
    let can_flag = conformity::may_flag(&user);
    if !can_flag
        && !user.has_permission_anywhere(permissions::WORK_ORDERS_VIEW)
        && !user.has_permission_anywhere(permissions::NCRS_VIEW)
    {
        return Err(Error::Forbidden);
    }
    let mut rows = Vec::new();
    for flag in ItemFlag::query()
        .order_by(ItemFlag::ITEM_ID)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|flag| flag.regulated || flag.export_only)
    {
        let subject = flag.subject();
        let mut certificates = Vec::new();
        for line in guard(&state)
            .documents_of(&state.db, &subject, today(), &lang.locale)
            .await?
        {
            certificates.push(Certificate {
                kind: line.kind,
                number: line.number.unwrap_or_default(),
                valid_to: line
                    .valid_to
                    .map_or_else(|| t(&lang, "manufacturing.screen.not_set"), date_text),
                validity: word(&lang, "doc_validity", &line.validity),
                validity_kind: validity_kind(&line.validity),
            });
        }
        let yes = t(&lang, "manufacturing.screen.conformity.yes");
        let no = t(&lang, "manufacturing.screen.conformity.no");
        rows.push(Row {
            item: subject_text(&state.db, &subject).await?,
            regulated: if flag.regulated {
                yes.clone()
            } else {
                no.clone()
            },
            export_only: if flag.export_only { yes } else { no },
            accepted: if flag.regulated {
                flag.accepted()
                    .iter()
                    .map(|kind| word(&lang, "conformity_kind", kind))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                String::new()
            },
            certificates,
        });
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        return Ok(Json(&rows).into_response());
    }
    Ok(View::typed(ConformityPage {
        tx: screen_texts(&lang),
        rows,
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
            kinds,
        },
    )
    .await?;
    Ok(done(&lang, URL, "manufacturing.screen.conformity.done"))
}
