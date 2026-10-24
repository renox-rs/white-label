//! The grids of the back-office lists: the client contracts, the manpower
//! requests and the deployments (`GET /manpower/contracts`, `/requests`,
//! `/deployments`). The handlers stay in `pages.rs`: they decide which records
//! the person may see (and answer the JSON) and hand the ids to the functions
//! here, which add search, sort, filters, paging and the export on top.
//!
//! The names of the client and the worker come from the parties and workforce
//! directories, not from a table of this app: the client is a `select` filter
//! on the id with the names as options (drawn by the template), and the
//! search finds a client or a worker by name through a hidden column the
//! handler builds from the directories (`crate::grid_lookup`); neither is
//! sorted. The site is a related column of the app's own table and is
//! searched.

use std::collections::BTreeMap;

use renox::db::Query;
use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::Serialize;

use super::{Texts, date_or, date_text, options, request_page, role_text};
use crate::contract::ClientContract;
use crate::deployment::Deployment;
use crate::documents::DeploymentDocumentFlag;
use crate::grid_lookup;
use crate::hold::{ContractHold, reason_lines};
use crate::pages::{party_name, suspension_note};
use crate::permissions;
use crate::request::ManpowerRequest;
use crate::screens::deployment_page;
use crate::texts::{screen_texts, t, word};
use wl_kernel::documents::today;

/// The text `manpower.screen.<key>`.
fn text(lang: &Lang, key: &str) -> String {
    t(lang, &format!("manpower.screen.{key}"))
}

/// The ids the query is limited to; `-1` (no row) when the person sees none.
fn only(ids: Vec<i64>) -> Vec<i64> {
    if ids.is_empty() { vec![-1] } else { ids }
}

/// The export file when the request asks for one: refused for a person who may
/// not export.
async fn exported<M: Model + Serialize>(
    grid: &Grid,
    query: Query<M>,
    request: &GridRequest,
    can_export: bool,
) -> Result<Option<Response>> {
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    grid.export(query, request).await
}

/// The grid with `.exports()` for those who may export.
fn with_exports(grid: Grid, can_export: bool) -> Grid {
    if can_export { grid.exports() } else { grid }
}

// ---------------------------------------------------------------- contracts

/// The hidden column the search reads for names kept in another module: the
/// searched text for the rows the handler found by it (`sql`).
fn match_column(lang: &Lang, sql: &str) -> Column {
    Column::text("name_match", &text(lang, "client"))
        .computed(sql)
        .hidden()
        .searchable()
        .sortable(false)
        .filterable(false)
}

/// The client column: a filter on the client company (`clients`, the
/// options); the template draws the name.
fn client_column(lang: &Lang, clients: Vec<(String, String)>) -> Column {
    Column::select("client_party_id", &text(lang, "client"), clients).sortable(false)
}

/// The contracts grid: search on the reference and the client's name
/// (`client_match`), filters by client (`clients`), status, billing and
/// dates. The client and the credit hold are drawn by the template.
fn contracts_grid(
    lang: &Lang,
    clients: Vec<(String, String)>,
    client_match: &str,
    can_export: bool,
) -> Grid {
    let statuses = options(
        lang,
        "status",
        &[
            "draft",
            "active",
            "on_hold",
            "terminating",
            "ended",
            "cancelled",
        ],
    );
    let grid = Grid::new("manpower-contracts")
        .title(&text(lang, "contracts.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("reference", &text(lang, "contracts.reference"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/manpower/contracts/{id}"),
        )
        .column(client_column(lang, clients).mobile())
        .column(match_column(lang, client_match))
        .column(
            Column::select("status", &text(lang, "status"), statuses)
                .badges(&[
                    ("active", "success"),
                    ("on_hold", "warning"),
                    ("terminating", "warning"),
                    ("draft", "info"),
                    ("ended", "info"),
                    ("cancelled", "info"),
                ])
                .mobile(),
        )
        .column(Column::date("start_date", &text(lang, "form.start_date")))
        .column(Column::date("end_date", &text(lang, "form.end_date")).mobile())
        .column(Column::select(
            "billing_cycle",
            &text(lang, "contracts.billing"),
            options(lang, "billing", &["monthly", "fortnightly", "weekly"]),
        ))
        // The obligation in words (*Cover required* / *Not required*), not a tick.
        .column(
            Column::select(
                "cover",
                &text(lang, "contracts.cover"),
                [
                    ("1", text(lang, "contracts.cover_required")),
                    ("0", text(lang, "contracts.cover_none")),
                ],
            )
            .computed("CASE WHEN {T}.\"cover_required\" THEN '1' ELSE '0' END"),
        )
        .column(Column::custom("hold", &text(lang, "contracts.hold")))
        .column(Column::date("signed_on", &text(lang, "contracts.signed_on")).hidden())
        .sort_by("reference")
        .row_url("/manpower/contracts/{id}")
        .cards_on_mobile()
        .empty_state(
            &text(lang, "contracts.empty"),
            Some(&text(lang, "contracts.empty_text")),
        );
    with_exports(grid, can_export)
}

/// What the contracts page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manpower/contracts.html")]
pub(crate) struct ContractsPage {
    tx: Texts,
    contracts: GridPage<ClientContract>,
    /// Whether the person may start a contract for some client.
    can_create: bool,
    billing_cycles: Vec<(String, String)>,
    billing_selected: String,
    today: String,
}

/// The page of `visible`, the contracts the person may see.
pub(crate) async fn contracts(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    visible: &[ClientContract],
    session: &Session,
    request: GridRequest,
) -> Result<Response> {
    let can_create = user.has_permission_anywhere(permissions::CONTRACTS_MANAGE);
    let clients = grid_lookup::party_options(
        state,
        &lang.locale,
        visible
            .iter()
            .map(|contract| contract.client_party_id)
            .collect::<Vec<i64>>(),
    )
    .await;
    let term = grid_lookup::searched(&request, session, "manpower-contracts");
    let client_match = grid_lookup::party_match(state, term.as_deref(), &["client_party_id"]).await;
    let grid = contracts_grid(lang, clients, &client_match, can_create);
    let query = ClientContract::query().where_(
        ClientContract::ID.is_in(only(visible.iter().map(|contract| contract.id).collect())),
    );
    if let Some(file) = exported(&grid, query.clone(), &request, can_create).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut clients = BTreeMap::new();
    let mut holds = BTreeMap::new();
    for contract in page.items() {
        clients.insert(
            contract.client_party_id,
            party_name(state, lang, contract.client_party_id).await,
        );
        if contract.status == crate::contract::ContractStatus::OnHold {
            let open = ContractHold::open_of(&state.db, contract.id).await?;
            holds.insert(
                contract.id,
                json!({
                    "since": contract.held_at.map(|at| date_text(wl_kernel::documents::dubai_date(at))),
                    "before": contract
                        .status_before_hold
                        .map(|status| word(lang, "status", status.as_str())),
                    "reasons": reason_lines(&open),
                }),
            );
        }
    }
    let page = page.extend(|contract| {
        json!({
            "client": clients.get(&contract.client_party_id),
            "hold": holds.get(&contract.id).cloned().unwrap_or(Value::Null),
        })
    });
    Ok(View::typed(ContractsPage {
        tx: screen_texts(lang),
        contracts: page,
        can_create,
        billing_cycles: options(lang, "billing", &["monthly", "fortnightly", "weekly"]),
        billing_selected: "monthly".to_owned(),
        today: today().to_string(),
    })
    .into_response())
}

// ----------------------------------------------------------------- requests

/// The roles a request can ask for, with their words.
fn role_options() -> Vec<(String, String)> {
    request_page::ROLES
        .iter()
        .map(|role| ((*role).to_owned(), role_text(role)))
        .collect()
}

/// The requests grid: search on the site and the client's name
/// (`client_match`), filters by client (`clients`), status, role, shift and
/// dates, sums of the positions and the filled ones.
fn requests_grid(
    lang: &Lang,
    clients: Vec<(String, String)>,
    client_match: &str,
    can_export: bool,
) -> Grid {
    let statuses = options(
        lang,
        "request_status",
        &["draft", "open", "partially_filled", "filled", "cancelled"],
    );
    let grid = Grid::new("manpower-requests")
        .title(&text(lang, "requests.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::select("role", &text(lang, "form.role"), role_options())
                .frozen()
                .mobile()
                .link("/manpower/requests/{id}"),
        )
        .column(
            Column::related(
                "site",
                &text(lang, "requests.site"),
                "manpower_sites",
                "site_id",
                "name",
            )
            .mobile()
            .searchable(),
        )
        .column(client_column(lang, clients).mobile())
        .column(match_column(lang, client_match))
        .column(
            Column::select("status", &text(lang, "status"), statuses)
                .badges(&[
                    ("filled", "success"),
                    ("open", "warning"),
                    ("partially_filled", "warning"),
                    ("draft", "info"),
                    ("cancelled", "info"),
                ])
                .mobile(),
        )
        .column(
            Column::select(
                "kind",
                &text(lang, "requests.kind"),
                [
                    ("standard", text(lang, "requests.standard")),
                    ("replacement", text(lang, "requests.cover")),
                ],
            )
            .badges(&[("replacement", "warning")]),
        )
        .column(Column::number("headcount", &text(lang, "form.positions")).summary(Summary::Sum))
        .column(
            Column::number("filled_count", &text(lang, "requests.filled")).summary(Summary::Sum),
        )
        .column(Column::select(
            "shift_type",
            &text(lang, "requests.shift"),
            options(lang, "shift", &request_page::SHIFTS),
        ))
        .column(Column::date("start_date", &text(lang, "form.start_date")))
        // The end date, or *Open-ended* for a request with none (`custom`).
        .column(Column::custom("end_text", &text(lang, "form.end_date")))
        .column(Column::datetime("submitted_at", &text(lang, "requests.submitted")).hidden())
        .sort_by("-start_date")
        .row_url("/manpower/requests/{id}")
        .cards_on_mobile()
        .empty_state(
            &text(lang, "requests.empty"),
            Some(&text(lang, "requests.empty_text")),
        );
    with_exports(grid, can_export)
}

/// What the requests page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manpower/requests.html")]
pub(crate) struct RequestsPage {
    tx: Texts,
    requests: GridPage<ManpowerRequest>,
    /// Whether the person may write a request for some client; then the
    /// sheet of *New request* has its options.
    can_create: bool,
    form: request_page::NewRequestForm,
}

/// The page of `visible`, the ids of the requests the person may see.
pub(crate) async fn requests(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    visible: Vec<i64>,
    session: &Session,
    request: GridRequest,
) -> Result<Response> {
    let can_export = user.has_permission_anywhere(permissions::REQUESTS_MANAGE);
    let query = ManpowerRequest::query().where_(ManpowerRequest::ID.is_in(only(visible)));
    let client_ids = query
        .clone()
        .pluck::<i64, _>(&state.db, "client_party_id")
        .await?;
    let clients = grid_lookup::party_options(state, &lang.locale, client_ids).await;
    let term = grid_lookup::searched(&request, session, "manpower-requests");
    let client_match = grid_lookup::party_match(state, term.as_deref(), &["client_party_id"]).await;
    let grid = requests_grid(lang, clients, &client_match, can_export);
    if let Some(file) = exported(&grid, query.clone(), &request, can_export).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut clients = BTreeMap::new();
    for item in page.items() {
        clients.insert(
            item.client_party_id,
            party_name(state, lang, item.client_party_id).await,
        );
    }
    let open_ended = text(lang, "open_ended");
    let page = page.extend(|item| {
        json!({
            "client": clients.get(&item.client_party_id),
            "end_text": date_or(item.end_date, &open_ended),
        })
    });
    let form = request_page::new_request_form(state, user, lang).await?;
    Ok(View::typed(RequestsPage {
        tx: screen_texts(lang),
        requests: page,
        can_create: !form.contracts.is_empty(),
        form,
    })
    .into_response())
}

// -------------------------------------------------------------- deployments

/// The filters of the deployments grid that staff get: the clients and the
/// requests of their deployments (the options).
#[derive(Default)]
struct DeploymentFilters {
    clients: Vec<(String, String)>,
    requests: Vec<(String, String)>,
}

/// The deployments grid. Staff see the client, the site and the status when
/// they pick them (hidden at first) and the warnings, and filter by client and
/// request (`filters`); a client user's grid has only the worker, role, site,
/// shift, dates and status (`MPW.ACL.02`) and no export. The search finds the
/// site and the worker by name (`worker_match`, built by the handler).
fn deployments_grid(
    lang: &Lang,
    staff: bool,
    filters: DeploymentFilters,
    worker_match: &str,
    can_export: bool,
) -> Grid {
    let statuses = options(
        lang,
        "status",
        &["planned", "active", "suspended", "ended", "cancelled"],
    );
    let worker = Column::custom("worker", &text(lang, "deployments.worker"))
        .frozen()
        .mobile();
    let site = Column::related(
        "site",
        &text(lang, "deployments.site"),
        "manpower_sites",
        "site_id",
        "name",
    )
    .searchable();
    let status = Column::select("status", &text(lang, "status"), statuses).badges(&[
        ("active", "success"),
        ("suspended", "warning"),
        ("planned", "info"),
        ("ended", "info"),
        ("cancelled", "info"),
    ]);
    let mut grid = Grid::new("manpower-deployments")
        .title(&text(lang, "deployments.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(worker)
        .column(
            Column::text("name_match", &text(lang, "deployments.worker"))
                .computed(worker_match)
                .hidden()
                .searchable()
                .sortable(false)
                .filterable(false),
        )
        .column(Column::select("role", &text(lang, "deployments.role"), role_options()).mobile());
    if staff {
        grid = grid
            .column(client_column(lang, filters.clients).hidden())
            .column(
                Column::select(
                    "request_id",
                    &text(lang, "deployments.request"),
                    filters.requests,
                )
                .hidden()
                .sortable(false),
            )
            .column(site.hidden())
            .column(status.hidden().mobile());
    } else {
        grid = grid.column(site.mobile()).column(status.mobile());
    }
    grid = grid
        .column(Column::select(
            "shift_type",
            &text(lang, "deployments.shift"),
            options(lang, "shift", &request_page::SHIFTS),
        ))
        .column(Column::date("start_date", &text(lang, "form.start_date")))
        .column(Column::date("end_date", &text(lang, "form.end_date")).mobile());
    if staff {
        grid = grid
            .column(Column::custom("flags", &text(lang, "deployments.flags")))
            .column(
                Column::select(
                    "kind",
                    &text(lang, "deployments.kind"),
                    [
                        ("standard", text(lang, "deployments.standard")),
                        ("cover", text(lang, "deployments.cover")),
                    ],
                )
                .badges(&[("cover", "warning")])
                .hidden(),
            )
            .row_url("/manpower/deployments/{id}");
    }
    let grid = grid.sort_by("-start_date").cards_on_mobile().empty_state(
        &text(lang, "deployments.empty"),
        Some(&text(lang, "deployments.empty_text")),
    );
    with_exports(grid, staff && can_export)
}

/// The options of the staff's filters and the SQL of the worker search for the
/// deployments of `query`. The worker's name is the workforce's: the search
/// reads the names of the workers of those deployments (only when there is a
/// searched text) and matches them in Rust; a client company is also found
/// by name for staff.
async fn deployment_lookups(
    state: &AppState,
    lang: &Lang,
    query: &Query<Deployment>,
    staff: bool,
    term: Option<&str>,
) -> Result<(DeploymentFilters, String)> {
    let shown = query.clone().get(&state.db).await?;
    let mut filters = DeploymentFilters::default();
    if staff {
        filters.clients = grid_lookup::party_options(
            state,
            &lang.locale,
            shown
                .iter()
                .map(|deployment| deployment.client_party_id)
                .collect::<Vec<i64>>(),
        )
        .await;
        let mut request_ids: Vec<i64> = shown.iter().filter_map(|d| d.request_id).collect();
        request_ids.sort_unstable();
        request_ids.dedup();
        let requests = if request_ids.is_empty() {
            Vec::new()
        } else {
            ManpowerRequest::query()
                .where_(ManpowerRequest::ID.is_in(request_ids))
                .order_by(ManpowerRequest::ID)
                .get(&state.db)
                .await?
        };
        for item in &requests {
            let site = crate::site::Site::find(&state.db, item.site_id)
                .await?
                .map(|site| site.name)
                .unwrap_or_default();
            filters.requests.push((
                item.id.to_string(),
                format!(
                    "{} · {} · {}",
                    role_text(&item.role),
                    site,
                    date_text(item.start_date)
                ),
            ));
        }
    }
    let Some(term) = term else {
        return Ok((filters, "''".to_owned()));
    };
    let wanted = term.to_lowercase();
    let mut names: BTreeMap<i64, String> = BTreeMap::new();
    let mut found = Vec::new();
    for deployment in &shown {
        if !names.contains_key(&deployment.employee_id) {
            let name = crate::deployments::worker_name(state, &lang.locale, deployment.employee())
                .await
                .to_lowercase();
            names.insert(deployment.employee_id, name);
        }
        if names
            .get(&deployment.employee_id)
            .is_some_and(|name| name.contains(&wanted))
        {
            found.push(deployment.id);
        }
    }
    let mut sql = grid_lookup::matched_sql(Some(term), &["id"], &found);
    if staff {
        let clients = grid_lookup::party_ids(state, Some(term)).await;
        if !clients.is_empty() {
            let by_client = grid_lookup::matched_sql(Some(term), &["client_party_id"], &clients);
            sql = format!("CASE WHEN ({sql}) <> '' THEN ({sql}) ELSE ({by_client}) END");
        }
    }
    Ok((filters, sql))
}

/// What the deployments page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manpower/deployments.html")]
pub(crate) struct DeploymentsPage {
    tx: Texts,
    deployments: GridPage<Deployment>,
    /// Whether the person may plan a deployment; the sheet's options follow.
    can_plan: bool,
    plan: deployment_page::PlanForm,
}

/// The page of `visible`, the ids of the deployments the person may see: those
/// of staff (`staff`) or the workers on site of a client user.
pub(crate) async fn deployments(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    visible: Vec<i64>,
    staff: bool,
    session: &Session,
    request: GridRequest,
) -> Result<Response> {
    let can_manage = user.has_permission_anywhere(permissions::DEPLOYMENTS_MANAGE);
    let query = Deployment::query().where_(Deployment::ID.is_in(only(visible)));
    let term = grid_lookup::searched(&request, session, "manpower-deployments");
    let (filters, worker_match) =
        deployment_lookups(state, lang, &query, staff, term.as_deref()).await?;
    let grid = deployments_grid(lang, staff, filters, &worker_match, can_manage);
    if let Some(file) = exported(&grid, query.clone(), &request, staff && can_manage).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut workers = BTreeMap::new();
    let mut clients = BTreeMap::new();
    let mut flags = BTreeMap::new();
    for item in page.items() {
        workers.insert(
            item.id,
            crate::deployments::worker_name(state, &lang.locale, item.employee()).await,
        );
        if staff {
            clients.insert(
                item.client_party_id,
                party_name(state, lang, item.client_party_id).await,
            );
            let mut lines: Vec<String> = DeploymentDocumentFlag::open_for(&state.db, item.id)
                .await?
                .iter()
                .map(DeploymentDocumentFlag::text)
                .collect();
            lines.extend(suspension_note(state, item).await?);
            flags.insert(item.id, lines);
        }
    }
    let page = page.extend(|item| {
        json!({
            "worker": workers.get(&item.id),
            "client": clients.get(&item.client_party_id),
            "flags": flags.get(&item.id),
            "url": if staff { deployment_page::url(item.id) } else { String::new() },
        })
    });
    let can_plan = can_manage;
    let plan = if can_plan {
        deployment_page::plan_form(state, user, lang).await?
    } else {
        deployment_page::PlanForm::default()
    };
    Ok(View::typed(DeploymentsPage {
        tx: screen_texts(lang),
        deployments: page,
        can_plan: can_plan && !plan.requests.is_empty(),
        plan,
    })
    .into_response())
}
