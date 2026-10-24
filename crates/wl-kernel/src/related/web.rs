//! The *Related* panel of a record (BL-AGT-03, `PLT.REL.01`):
//!
//! - `GET /related/{subject}/panel` (`invoicing.invoice:42`): the panel
//!   alone, for a record page to load in place:
//!   `<div hx-get="/related/invoicing.invoice:42/panel" hx-trigger="load"></div>`.
//! - `GET /related/{subject}`: the panel on a page of its own, where a
//!   search result or a related record of a type without a page of its own
//!   opens.
//!
//! Both show [`super::related`] at [`PANEL_DEPTH`] for the signed-in user,
//! one section per relation with its count and up to ten rows. A record the
//! user may not open ([`can_open`]) answers 404, as if it did not exist.

use std::collections::BTreeMap;

use renox::prelude::*;
use serde::Serialize;

use super::{PANEL_DEPTH, RelatedGroup, ROWS_PER_GROUP, can_open, related};
use crate::search::Access;
use crate::subject::Subject;
use crate::texts;

/// The name of the related records page's route.
pub const PAGE_ROUTE: &str = "wl_kernel.related";

/// The name of the panel's route.
pub const PANEL_ROUTE: &str = "wl_kernel.related.panel";

/// The page's template.
pub const PAGE_TEMPLATE: &str = "wl-kernel/related.html";

/// The panel's template; a record page may include it with the data of
/// [`panel`] under `related`.
pub const PANEL_TEMPLATE: &str = "wl-kernel/related_panel.html";

/// The texts a template reads as `tx['key']`.
type Texts = BTreeMap<&'static str, String>;

/// The page of a record's related records.
#[derive(Serialize, ViewData)]
#[view(template = "wl-kernel/related.html")]
pub struct RelatedPage {
    tx: Texts,
    related: Panel,
}

/// The panel alone.
#[derive(Serialize, ViewData)]
#[view(template = "wl-kernel/related_panel.html")]
pub struct RelatedPanel {
    tx: Texts,
    related: Panel,
}

/// What the panel draws.
#[derive(Debug, Clone, Serialize)]
pub struct Panel {
    /// The record, e.g. `invoicing.invoice:42`.
    pub subject: String,
    /// What the record is called.
    pub record: String,
    /// The line under the panel's title.
    pub subtitle: String,
    /// The groups, each with its count and up to ten rows.
    pub groups: Vec<GroupView>,
}

/// One group as the panel draws it.
#[derive(Debug, Clone, Serialize)]
pub struct GroupView {
    /// The relation's name.
    pub name: String,
    /// The heading.
    pub label: String,
    /// How many records the user may see.
    pub count: u64,
    /// The rows listed.
    pub records: Vec<super::RelatedRecord>,
    /// "and 3 more" when the group has more than ten.
    pub more: Option<String>,
}

/// The address of `subject`'s related records page.
pub fn url_of(subject: &Subject) -> String {
    format!("/related/{subject}")
}

/// The address of `subject`'s panel, for `hx-get`.
pub fn panel_url_of(subject: &Subject) -> String {
    format!("/related/{subject}/panel")
}

/// The routes, for logged-in users; who may see what is checked per
/// request.
pub fn routes() -> Routes {
    Routes::new()
        .get("/related/{subject}", page)
        .name(PAGE_ROUTE)
        .get("/related/{subject}/panel", panel_route)
        .name(PANEL_ROUTE)
        .require_auth()
}

fn page_texts(lang: &Lang) -> Texts {
    ["title", "empty", "void"]
        .into_iter()
        .map(|key| (key, texts::t(lang, &format!("wl_kernel.related.{key}"))))
        .collect()
}

/// The panel of `subject` for `access`, or `None` when the user may not
/// open the record. For a record page that draws the panel itself
/// (`<rx-include template="wl-kernel/related_panel.html"/>` with this under
/// `related` and the texts of the panel under `tx`).
pub async fn panel(
    db: &Db,
    access: &Access,
    lang: &Lang,
    subject: &Subject,
) -> Result<Option<Panel>> {
    let Some(record) = can_open(db, access, subject).await? else {
        return Ok(None);
    };
    let groups = related(db, access, subject, PANEL_DEPTH).await?;
    Ok(Some(Panel {
        subject: subject.to_string(),
        subtitle: texts::t_with(lang, "wl_kernel.related.subtitle", &[("record", &record)]),
        record,
        groups: groups
            .into_iter()
            .map(|group| group_view(lang, group))
            .collect(),
    }))
}

fn group_view(lang: &Lang, group: RelatedGroup) -> GroupView {
    let listed = u64::try_from(group.records.len().min(ROWS_PER_GROUP)).unwrap_or(0);
    let rest = group.count.saturating_sub(listed);
    GroupView {
        more: (rest > 0).then(|| texts::t_with(lang, "wl_kernel.related.more", &[("count", &rest)])),
        name: group.name,
        label: group.label,
        count: group.count,
        records: group.records,
    }
}

/// The subject of the path, or 404.
fn subject_of(text: &str) -> Result<Subject> {
    text.parse::<Subject>().map_err(|_| Error::NotFound)
}

async fn page(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    Path(subject): Path<String>,
) -> Result<View> {
    let subject = subject_of(&subject)?;
    let access = Access::of(&db, &user).await?;
    let related = panel(&db, &access, &lang, &subject)
        .await?
        .ok_or(Error::NotFound)?;
    Ok(View::typed(RelatedPage {
        tx: page_texts(&lang),
        related,
    }))
}

async fn panel_route(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    Path(subject): Path<String>,
) -> Result<View> {
    let subject = subject_of(&subject)?;
    let access = Access::of(&db, &user).await?;
    let related = panel(&db, &access, &lang, &subject)
        .await?
        .ok_or(Error::NotFound)?;
    Ok(View::typed(RelatedPanel {
        tx: page_texts(&lang),
        related,
    }))
}
