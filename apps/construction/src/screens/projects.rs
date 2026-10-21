//! `GET /construction/projects`: the projects the person may see, with their
//! status, contract value, IPAs and the links to the reports.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::documents::today;

use super::{
    Texts, closing_url, ipa_kind, ipa_url, party_name, project_kind, tx, variations_url, word,
};
use crate::ipa::Ipa;
use crate::texts::english;
use crate::watch::{self, EXPIRY_DAYS, WAIT_DAYS};

/// An IPA link of a row.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct IpaLink {
    label: String,
    url: String,
    status: String,
    status_kind: &'static str,
}

/// One project of the list.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ProjectRow {
    id: i64,
    code: String,
    name: String,
    client: String,
    status: String,
    status_kind: &'static str,
    contract_value: i64,
    completion_due_on: Option<NaiveDate>,
    closing_url: String,
    variations_url: String,
    ipas: Vec<IpaLink>,
    flags: Vec<String>,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "construction/projects.html")]
pub(crate) struct ProjectsPage {
    tx: Texts,
    projects: Vec<ProjectRow>,
    in_progress: usize,
    contract_total: i64,
    waiting: usize,
}

/// The page of the projects `user` may see.
pub(crate) async fn page(state: &AppState, user: &AuthUser, lang: &Lang) -> Result<Response> {
    let projects = watch::visible_projects(&state.db, user).await?;
    let ids = watch::ids_of(&projects);
    let day = today();
    let waiting_ipas = watch::ipas_waiting(&state.db, &ids, day).await?;
    let expiring = watch::guarantees_expiring(&state.db, &ids, day).await?;
    let waiting_variations = watch::variations_waiting(&state.db, &ids, day).await?;
    let ipas: Vec<Ipa> = if ids.is_empty() {
        Vec::new()
    } else {
        Ipa::query()
            .where_(Ipa::PROJECT_ID.is_in(ids.clone()))
            .order_by(Ipa::ID)
            .get(&state.db)
            .await?
    };
    let mut rows = Vec::with_capacity(projects.len());
    for project in &projects {
        let mut flags = Vec::new();
        let waiting = |found: usize, key: &str, days: i64| {
            (found > 0).then(|| english(key, &[("count", &found), ("days", &days)]))
        };
        flags.extend(waiting(
            watch::count_of(&waiting_ipas, |ipa| ipa.project_id == project.id),
            "construction.screen.projects.ipas_waiting",
            WAIT_DAYS,
        ));
        flags.extend(waiting(
            watch::count_of(&expiring, |guarantee| guarantee.project_id == project.id),
            "construction.screen.projects.guarantees_expiring",
            EXPIRY_DAYS,
        ));
        flags.extend(waiting(
            watch::count_of(&waiting_variations, |variation| {
                variation.project_id == project.id
            }),
            "construction.screen.projects.variations_waiting",
            WAIT_DAYS,
        ));
        rows.push(ProjectRow {
            id: project.id,
            code: project.code.clone(),
            name: project.name.clone(),
            client: party_name(state, lang, project.client_party_id).await,
            status: word(lang, "status", project.status.as_str()),
            status_kind: project_kind(project.status),
            contract_value: project.contract_value,
            completion_due_on: project.completion_due_on,
            closing_url: closing_url(project.id),
            variations_url: variations_url(project.id),
            ipas: ipas
                .iter()
                .filter(|ipa| ipa.project_id == project.id)
                .map(|ipa| IpaLink {
                    label: english(
                        "construction.screen.projects.ipa_label",
                        &[("number", &ipa.number)],
                    ),
                    url: ipa_url(ipa.id),
                    status: word(lang, "status", ipa.status.as_str()),
                    status_kind: ipa_kind(ipa.status),
                })
                .collect(),
            flags,
        });
    }
    let in_progress = watch::in_progress(&projects).len();
    let contract_total = projects.iter().map(|project| project.contract_value).sum();
    let attention = waiting_ipas.len() + expiring.len() + waiting_variations.len();
    Ok(View::typed(ProjectsPage {
        tx: tx(
            lang,
            &[
                "projects.title",
                "projects.subtitle",
                "projects.code",
                "projects.name",
                "projects.client",
                "status",
                "projects.contract_value",
                "projects.ipas",
                "projects.reports",
                "projects.waiting",
                "projects.closing",
                "projects.variations",
                "projects.in_progress",
                "projects.contract_total",
                "projects.attention",
                "projects.no_ipas",
                "projects.empty",
                "projects.empty_text",
            ],
        ),
        projects: rows,
        in_progress,
        contract_total,
        waiting: attention,
    })
    .into_response())
}
