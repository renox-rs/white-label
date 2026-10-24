//! The global search page (BL-AGT-02, wl-kernel §5.8, `PLT.SRC.01`):
//!
//! - `GET /search?q=`: the records the user may open, grouped by type, the
//!   group of the best match first, up to [`PER_TYPE`] per type with *Show
//!   all N*. When exactly one record the user may see has an identifier
//!   equal to the query (`INV-PGP-2026-0004`, also typed `inv pgp 2026
//!   0004`), the page redirects to it.
//! - `GET /search?q=&type=invoicing.invoice&page=2`: one type, paged by
//!   [`PER_PAGE`].
//!
//! The box in the shell's top bar is the app layout's (the host's
//! `layouts/app.html`): it reads the menu [`MENU`], which holds one link
//! shown only to a user who may search something ([`can_search`], in
//! memory), and is drawn turned off with the reason for anyone else.
//! `mod+k` focuses it. A user whose role can search nothing sees the box of
//! this page turned off too, with the reason.

use std::collections::BTreeMap;

use renox::nav::NavLink;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::access::{Access, can_search};
use super::query::{
    PER_PAGE, PER_TYPE, SearchGroup, SearchHit, search, search_type, searchable_query,
};
use crate::documents::dubai_text;
use crate::texts;

/// The name of the search route.
pub const ROUTE: &str = "wl_kernel.search";

/// The page's template.
pub const TEMPLATE: &str = "wl-kernel/search.html";

/// The menu the top bar reads to know whether the user may search: one link
/// to the page, visible only to users who hold the permission of some
/// searchable type (`nav('search')` in the layout).
pub const MENU: &str = "search";

/// The texts a template reads as `tx['key']`.
type Texts = BTreeMap<&'static str, String>;

/// The keys of `wl_kernel.search.*` the page reads.
const PAGE_TEXTS: &[&str] = &[
    "title",
    "label",
    "placeholder",
    "submit",
    "all_results",
    "previous",
    "next",
    "void",
];

/// What the search page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-kernel/search.html")]
pub struct SearchPage {
    tx: Texts,
    found: Found,
}

/// The page's state, under a name no kit tag has.
#[derive(Debug, Serialize, Default)]
pub struct Found {
    /// The query as typed.
    q: String,
    /// The type of a *Show all* page.
    only: String,
    /// Whether the user may search at all.
    allowed: bool,
    /// Why the box is off, or a hint for a short query.
    hint: Option<String>,
    /// The heading under the title.
    subtitle: String,
    /// The empty state's title and message, when nothing was found.
    empty: Option<String>,
    empty_hint: Option<String>,
    groups: Vec<GroupView>,
    /// The paging links of a *Show all* page.
    back_url: Option<String>,
    prev_url: Option<String>,
    next_url: Option<String>,
    page_label: Option<String>,
}

#[derive(Debug, Serialize)]
struct GroupView {
    subject_type: String,
    heading: String,
    total: u64,
    count: String,
    rows: Vec<HitView>,
    more_url: Option<String>,
    more_label: Option<String>,
}

#[derive(Debug, Serialize)]
struct HitView {
    title: String,
    secondary: String,
    url: String,
    status: Option<String>,
    voided: bool,
    matched: Option<String>,
    updated: Option<String>,
}

/// The query string.
#[derive(Debug, Default, Deserialize)]
struct Params {
    #[serde(default)]
    q: String,
    #[serde(default, rename = "type")]
    only: Option<String>,
    #[serde(default)]
    page: Option<u64>,
}

/// The routes, for logged-in users; what each user sees is filtered per
/// request.
pub fn routes() -> Routes {
    Routes::new()
        .get("/search", page)
        .name(ROUTE)
        .require_auth()
}

/// The top bar's link: in [`MENU`], shown only to a user who may search.
pub(crate) fn nav_link() -> NavLink {
    NavLink::route(ROUTE, &texts::en("wl_kernel.search.placeholder"))
        .key("wl_kernel.search")
        .icon("search")
        .when(can_search)
}

/// `text` encoded for a query string.
pub(crate) fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(*byte));
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn hit_view(lang: &Lang, hit: SearchHit) -> HitView {
    HitView {
        updated: hit.updated_at.map(|at| {
            let at = dubai_text(at);
            texts::t_with(lang, "wl_kernel.search.updated", &[("at", &at)])
        }),
        matched: hit
            .matched
            .map(|value| texts::t_with(lang, "wl_kernel.search.matched", &[("value", &value)])),
        title: hit.title,
        secondary: hit.secondary,
        url: hit.url,
        status: hit.status,
        voided: hit.voided,
    }
}

fn group_view(lang: &Lang, query: &str, group: SearchGroup, with_more: bool) -> GroupView {
    let shown = u64::try_from(group.hits.len()).unwrap_or(0);
    let more = with_more && group.total > shown;
    GroupView {
        heading: group.label.clone(),
        total: group.total,
        count: texts::t_with(lang, "wl_kernel.search.count", &[("count", &group.total)]),
        more_url: more.then(|| {
            format!(
                "/search?q={}&type={}",
                encode(query),
                encode(&group.subject_type)
            )
        }),
        more_label: more.then(|| {
            texts::t_with(
                lang,
                "wl_kernel.search.show_all",
                &[("count", &group.total)],
            )
        }),
        subject_type: group.subject_type,
        rows: group
            .hits
            .into_iter()
            .map(|hit| hit_view(lang, hit))
            .collect(),
    }
}

fn page_texts(lang: &Lang) -> Texts {
    PAGE_TEXTS
        .iter()
        .map(|key| (*key, texts::t(lang, &format!("wl_kernel.search.{key}"))))
        .collect()
}

async fn page(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    Query(params): Query<Params>,
) -> Result<Response> {
    let access = Access::of(&db, &user).await?;
    let found = found(&db, &access, &lang, &params).await?;
    Ok(match found {
        Answer::Open(url) => Redirect::to(&url).into_response(),
        Answer::Page(found) => View::typed(SearchPage {
            tx: page_texts(&lang),
            found: *found,
        })
        .into_response(),
    })
}

/// What a search answers: a page, or the one record to open.
enum Answer {
    Page(Box<Found>),
    Open(String),
}

async fn found(db: &Db, access: &Access, lang: &Lang, params: &Params) -> Result<Answer> {
    let q = params.q.trim().to_owned();
    let mut found = Found {
        q: q.clone(),
        only: params.only.clone().unwrap_or_default(),
        allowed: !access.searchable_types().is_empty(),
        subtitle: texts::t(lang, "wl_kernel.search.label"),
        ..Default::default()
    };
    if !found.allowed {
        found.hint = Some(texts::t(lang, "wl_kernel.search.no_permission"));
        return Ok(Answer::Page(Box::new(found)));
    }
    if !searchable_query(&q) {
        if !q.is_empty() {
            found.hint = Some(texts::t(lang, "wl_kernel.search.too_short"));
        }
        return Ok(Answer::Page(Box::new(found)));
    }
    found.subtitle = texts::t_with(lang, "wl_kernel.search.results_for", &[("query", &q)]);
    let nothing = |found: &mut Found| {
        found.empty = Some(texts::t_with(
            lang,
            "wl_kernel.search.nothing_for",
            &[("query", &q)],
        ));
        found.empty_hint = Some(texts::t(lang, "wl_kernel.search.nothing_hint"));
    };

    if let Some(only) = params.only.as_deref().filter(|s| !s.is_empty()) {
        let page = params.page.unwrap_or(1).max(1);
        found.back_url = Some(format!("/search?q={}", encode(&q)));
        // A type the user may not see reads as one without results.
        match search_type(db, access, &q, only, page, PER_PAGE).await? {
            Some(group) if group.total > 0 => {
                let pages = group.total.div_ceil(PER_PAGE).max(1);
                let link =
                    |n: u64| format!("/search?q={}&type={}&page={n}", encode(&q), encode(only));
                found.prev_url = (page > 1).then(|| link(page - 1));
                found.next_url = (page < pages).then(|| link(page + 1));
                found.page_label = Some(texts::t_with(
                    lang,
                    "wl_kernel.search.page_of",
                    &[("page", &page), ("pages", &pages)],
                ));
                found.groups.push(group_view(lang, &q, group, false));
            }
            _ => nothing(&mut found),
        }
        return Ok(Answer::Page(Box::new(found)));
    }

    let results = search(db, access, &q, PER_TYPE).await?;
    if let Some(url) = results.open {
        return Ok(Answer::Open(url));
    }
    if results.groups.is_empty() {
        nothing(&mut found);
    }
    found.groups = results
        .groups
        .into_iter()
        .map(|group| group_view(lang, &q, group, true))
        .collect();
    Ok(Answer::Page(Box::new(found)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_is_encoded_for_a_link() {
        assert_eq!(encode("Al Ali & co"), "Al+Ali+%26+co");
        assert_eq!(encode("invoicing.invoice"), "invoicing.invoice");
    }
}
