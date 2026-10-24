//! `GET /construction/projects`: the projects the person may see, with their
//! status, contract value, IPAs and the links to the reports.

use renox::grid::{Column, Grid, GridRequest, Summary};
use renox::prelude::*;
use serde::Serialize;
use renox::serde_json::json;
use wl_kernel::documents::today;

use super::{Texts, closing_url, ipa_kind, ipa_url, tx, variations_url, word};
use crate::ipa::Ipa;
use crate::permissions;
use crate::project::{Project, ProjectStatus};
use crate::texts::english;
use crate::watch::{self, EXPIRY_DAYS, WAIT_DAYS};

/// The projects grid: search on the code, name and client, filters by
/// status and emirate, sums of the contract value and the budget. The IPAs,
/// the flags and the report links are drawn by the template (`custom`).
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let t = tx(lang);
    let text = |key: &str| t.get(key).cloned().unwrap_or_else(|| key.to_owned());
    let statuses: Vec<(String, String)> = ProjectStatus::ALL
        .iter()
        .map(|status| {
            (
                status.as_str().to_owned(),
                word(lang, "status", status.as_str()),
            )
        })
        .collect();
    let grid = Grid::new("construction-projects")
        .title(&text("projects.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("code", &text("projects.code"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/construction/projects/{id}/closing"),
        )
        .column(
            Column::text("name", &text("projects.name"))
                .mobile()
                .searchable(),
        )
        .column(
            Column::related(
                "client",
                &text("projects.client"),
                "wl_parties_parties",
                "client_party_id",
                "display_name",
            )
            .mobile()
            .searchable(),
        )
        .column(
            Column::select("status", &text("projects.status"), statuses)
                .badges(&[
                    ("in_progress", "success"),
                    ("practical_completion", "success"),
                    ("on_hold", "warning"),
                    ("terminated", "danger"),
                    ("setup", "info"),
                    ("closed", "info"),
                    ("cancelled", "info"),
                ])
                .mobile(),
        )
        .column(
            Column::money("contract_value", &text("projects.contract_value"))
                .mobile()
                .summary(Summary::Sum),
        )
        .column(Column::date("completion_due_on", &text("projects.completion_due")).hidden())
        .column(
            Column::money("budget", &text("projects.budget"))
                .hidden()
                .summary(Summary::Sum),
        )
        .column(
            Column::money("advance_paid", &text("projects.advance_paid"))
                .hidden()
                .summary(Summary::Sum),
        )
        .column(Column::text("emirate", &text("projects.emirate")).hidden())
        .column(Column::custom("ipas", &text("projects.ipas")))
        .column(Column::custom("flags", &text("projects.waiting")).mobile())
        .column(Column::custom("reports", &text("projects.reports")))
        .sort_by("code")
        .row_url("/construction/projects/{id}/closing")
        .cards_on_mobile()
        .empty_state(
            &text("projects.empty"),
            Some(&text("projects.empty_text")),
        );
    if can_export { grid.exports() } else { grid }
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "construction/projects.html")]
pub(crate) struct ProjectsPage {
    tx: Texts,
    any: bool,
    projects: renox::grid::GridPage<Project>,
    in_progress: usize,
    contract_total: i64,
    waiting: usize,
    subtitle: String,
}

/// The page of the projects `user` may see.
pub(crate) async fn page(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    request: GridRequest,
) -> Result<Response> {
    let visible = watch::visible_projects(&state.db, user).await?;
    let visible_ids = watch::ids_of(&visible);
    let day = today();
    let waiting_ipas = watch::ipas_waiting(&state.db, &visible_ids, day).await?;
    let expiring = watch::guarantees_expiring(&state.db, &visible_ids, day).await?;
    let waiting_variations = watch::variations_waiting(&state.db, &visible_ids, day).await?;
    // The grid searches, sorts and pages the projects the person may see.
    let can_export = [permissions::PROJECTS_MANAGE]
        .iter()
        .any(|permission| user.allows(permission) || user.has_permission_anywhere(permission));
    let grid = grid(lang, can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    let query = Project::query().where_(Project::ID.is_in(if visible_ids.is_empty() {
        vec![-1]
    } else {
        visible_ids.clone()
    }));
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    // The IPAs of the projects on this page, and what waits on each.
    let on_page = watch::ids_of(page.items());
    let ipas: Vec<Ipa> = if on_page.is_empty() {
        Vec::new()
    } else {
        Ipa::query()
            .where_(Ipa::PROJECT_ID.is_in(on_page))
            .order_by(Ipa::ID)
            .get(&state.db)
            .await?
    };
    let page = page.extend(|project| {
        let waiting = |found: usize, key: &str, days: i64| {
            (found > 0).then(|| english(key, &[("count", &found), ("days", &days)]))
        };
        let flags: Vec<String> = [
            waiting(
                watch::count_of(&waiting_ipas, |ipa| ipa.project_id == project.id),
                "construction.screen.projects.ipas_waiting",
                WAIT_DAYS,
            ),
            waiting(
                watch::count_of(&expiring, |guarantee| guarantee.project_id == project.id),
                "construction.screen.projects.guarantees_expiring",
                EXPIRY_DAYS,
            ),
            waiting(
                watch::count_of(&waiting_variations, |variation| {
                    variation.project_id == project.id
                }),
                "construction.screen.projects.variations_waiting",
                WAIT_DAYS,
            ),
        ]
        .into_iter()
        .flatten()
        .collect();
        let links: Vec<_> = ipas
            .iter()
            .filter(|ipa| ipa.project_id == project.id)
            .map(|ipa| {
                json!({
                    "label": english(
                        "construction.screen.projects.ipa_label",
                        &[("number", &ipa.number)],
                    ),
                    "url": ipa_url(ipa.id),
                    "status": word(lang, "status", ipa.status.as_str()),
                    "status_kind": ipa_kind(ipa.status),
                })
            })
            .collect();
        json!({
            "ipas": links,
            "flags": flags,
            "closing_url": closing_url(project.id),
            "variations_url": variations_url(project.id),
        })
    });
    let in_progress = watch::in_progress(&visible).len();
    let contract_total = visible.iter().map(|project| project.contract_value).sum();
    let attention = waiting_ipas.len() + expiring.len() + waiting_variations.len();
    Ok(View::typed(ProjectsPage {
        tx: tx(lang),
        any: !visible.is_empty(),
        projects: page,
        in_progress,
        contract_total,
        waiting: attention,
        subtitle: format!(
            "{} {}",
            tx(lang)["projects.subtitle"],
            english(
                "construction.screen.projects.waiting_hint",
                &[("days", &WAIT_DAYS), ("expiry", &EXPIRY_DAYS)],
            ),
        ),
    })
    .into_response())
}
