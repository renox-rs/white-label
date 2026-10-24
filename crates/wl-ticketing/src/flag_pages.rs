//! The compliance report (wl-ticketing §6, issue #148): the open compliance
//! flags by subject, reason and age.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /compliance-flags` | `wl_ticketing.compliance_flags` | `tickets.view` (`?subject_type=&subject_id=` lists one subject's flags) |
//!
//! There is no report library yet; this page is the report and is what the
//! dashboard of a vertical links to. The template is compiled in and can be
//! replaced by the app (`resources/views/wl-ticketing/compliance_flags.html`).

use std::collections::BTreeMap;

use renox::axum::extract::Query;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::config::Category;
use crate::flags::ComplianceFlag;
use crate::permissions;
use crate::texts;
use crate::ticket::Ticket;

/// The texts a template reads as `tx['flags.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes, behind `tickets.view`.
pub fn routes() -> Routes {
    // Anywhere: a manager sees the flags of their buildings.
    wl_kernel::scopes::anywhere(
        Routes::new()
            .get("/compliance-flags", index)
            .name("wl_ticketing.compliance_flags"),
        permissions::VIEW,
    )
}

/// The query of the report.
#[derive(Deserialize, Debug, Default)]
pub struct FlagQuery {
    /// Only the flags on this subject type…
    #[serde(default)]
    pub subject_type: Option<String>,
    /// …and this id.
    #[serde(default)]
    pub subject_id: Option<i64>,
}

/// The compliance flags grid: search on the subject (in words, "Lease #3",
/// never `property.lease #3`) and category, filters by category, reason and
/// raised date, the oldest first. The visit and contract links and the age
/// are drawn by the template (`custom`). `subject_types` are the types the
/// subject column names in words, `categories` the (name, name) choices of
/// the category filter.
pub fn grid(
    lang: &Lang,
    can_export: bool,
    subject_types: &[String],
    categories: Vec<(String, String)>,
) -> Grid {
    let t = |key: &str| texts::t(lang, &format!("wl_ticketing.{key}"));
    let reasons: Vec<(String, String)> = ["no_contract", "visit_missed", "vendor_document_expired"]
        .into_iter()
        .map(|key| (key.to_owned(), t(&format!("flags.reason_{key}"))))
        .collect();
    let category_sql = if lang.locale == "ar" {
        "(SELECT COALESCE(NULLIF(\"name_ar\", ''), \"name_en\") FROM \"wl_ticketing_categories\" WHERE \"id\" = {T}.\"category_id\")"
    } else {
        "(SELECT \"name_en\" FROM \"wl_ticketing_categories\" WHERE \"id\" = {T}.\"category_id\")"
    };
    let grid = Grid::new("wl-ticketing-compliance-flags")
        .title(&t("flags.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("subject", &t("flags.subject"))
                .computed(&wl_kernel::subject::words_sql(
                    "subject_type",
                    "subject_id",
                    subject_types,
                ))
                .frozen()
                .mobile()
                .searchable(),
        )
        .column(
            Column::select("category", &t("flags.category"), categories)
                .computed(category_sql)
                .searchable()
                .mobile(),
        )
        .column(
            Column::select("reason", &t("flags.reason"), reasons)
                .badges(&[
                    ("no_contract", "danger"),
                    ("visit_missed", "danger"),
                    ("vendor_document_expired", "danger"),
                ])
                .mobile(),
        )
        .column(Column::datetime("raised_at", &t("flags.raised")))
        .column(Column::custom("age", &t("flags.age")).mobile())
        .column(Column::custom("visit", &t("flags.visit")))
        .column(Column::custom("contract", &t("flags.contract")))
        .sort_by("raised_at,id")
        .cards_on_mobile()
        .empty_state(&t("flags.empty"), Some(&t("flags.empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// What the report needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-ticketing/compliance_flags.html")]
pub(crate) struct FlagsPage {
    tx: Texts,
    flags: GridPage<ComplianceFlag>,
}

/// `GET /compliance-flags`: the open flags, oldest first (or the export,
/// with `tickets.manage`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<FlagQuery>,
    request: GridRequest,
) -> Result<Response> {
    let reach = wl_kernel::scopes::reach(&state.db, &user, permissions::VIEW).await?;
    let subject = match (query.subject_type, query.subject_id) {
        (Some(kind), Some(id)) if !kind.trim().is_empty() => Some(Subject::new(kind.trim(), id)),
        _ => None,
    };
    let flags = match &subject {
        Some(subject) => ComplianceFlag::open_for(&state.db, subject).await?,
        None => ComplianceFlag::open(&state.db).await?,
    };
    // Only the flags on the subjects of the user's scope.
    let mut ids = Vec::new();
    for flag in flags {
        if reach
            .contains(
                &state.db,
                &Subject::new(&flag.subject_type, flag.subject_id),
            )
            .await?
        {
            ids.push(flag.id);
        }
    }
    let can_export = user.has_permission(permissions::MANAGE);
    let mut subject_types: Vec<String> = ComplianceFlag::query()
        .pluck::<String, _>(&state.db, "subject_type")
        .await?;
    subject_types.sort();
    subject_types.dedup();
    // The category filter offers the names the column shows, in the page's
    // language.
    let mut categories: Vec<(String, String)> = Category::query()
        .get(&state.db)
        .await?
        .into_iter()
        .map(|c| {
            let name = if lang.locale == "ar" && !c.name_ar.trim().is_empty() {
                c.name_ar
            } else {
                c.name_en
            };
            (name.clone(), name)
        })
        .collect();
    categories.sort();
    categories.dedup();
    let grid = grid(&lang, can_export, &subject_types, categories);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    let visible = ComplianceFlag::query().where_(ComplianceFlag::ID.is_in(if ids.is_empty() {
        vec![-1]
    } else {
        ids
    }));
    if let Some(file) = grid.export(visible.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(visible, &request).await?;
    let today = wl_kernel::documents::dubai_date(renox::db::now());
    let mut visits: BTreeMap<i64, String> = BTreeMap::new();
    for flag in page.items() {
        if let Some(id) = flag.ticket_id
            && let Some(ticket) = Ticket::find(&state.db, id).await?
        {
            visits.insert(flag.id, ticket.number);
        }
    }
    let page = page.extend(|flag| {
        let age = (today - wl_kernel::documents::dubai_date(flag.raised_at)).num_days();
        json!({
            "age": texts::t(&lang, "wl_ticketing.flags.days").replace(":count", &age.to_string()),
            "visit": visits.get(&flag.id),
            "visit_id": flag.ticket_id,
            "contract": flag.contract_id.map(|c| format!("#{c}")),
            "contract_id": flag.contract_id,
        })
    });
    Ok(View::typed(FlagsPage {
        tx: texts::all(&lang),
        flags: page,
    })
    .into_response())
}
