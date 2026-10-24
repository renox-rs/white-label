//! The absence case screens (`GET /manpower/absence`, `GET
//! /manpower/absence/{id}`, BL-MPW-25, `docs/UX.md` §2.2, 09 Manpower §4.7):
//! the list of cases, open and closed, and the case page where HR sees the
//! worker, the days absent, the failed contact days, the MoHRE report and the
//! tickets of the case, records the report and closes the case.
//!
//! Decisions where the spec is silent:
//!
//! - **Days absent** are the calendar days from day 0 (the last day present,
//!   `MPW.RUL.09`) to today, or to the day the case closed.
//! - **Report as absconded** is a link to the employee's page, where the exit
//!   sheet and its guards (BL-HRM-07) live; the page never runs
//!   `report_absconding`. The link is drawn disabled with *Two failed contact
//!   days and the MoHRE number are needed* until both are recorded.
//! - **The tickets** are those of the category *absence* about the worker.
//! - **The held pay** of a closed case is released by wl-hr; the page links
//!   the payroll run that pays it when there is one.
//! - The JSON of the list stays the open cases, as before; the page lists
//!   all of them.

use std::collections::BTreeMap;

use renox::axum::http::HeaderMap;
use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::tickets::{task_creator, ticket_path};
use wl_kernel::workforce::{EMPLOYEE_SUBJECT_TYPE, workforce};

use super::{Act, Texts, date_or, date_text, party_text, user_text};
use crate::absence::{self, AbsenceCase, FAILED_ATTEMPTS};
use crate::deployment::Deployment;
use crate::deployments::worker_name;
use crate::grid_lookup;
use crate::seed::ABSENCE_CATEGORY;
use crate::site::Site;
use crate::texts::{screen_texts, t, tf, word};

/// The address of a case's page.
pub(crate) fn url(id: i64) -> String {
    format!("/manpower/absence/{id}")
}

/// The days from day 0 to `until`.
fn days_absent(case: &AbsenceCase, until: NaiveDate) -> i64 {
    (until - case.last_present_on).num_days().max(0)
}

/// The absence cases grid: search on the MoHRE report number and the worker's
/// name (`worker_match`, built by the handler from the workforce), filters by
/// status and dates. The worker, the client, the contact attempts and the
/// report line are drawn by the template. The days absent run to today for an
/// open case, so they are worked out in Rust and not sorted in SQL; the last
/// day present sorts the open cases in the same order.
fn cases_grid(lang: &Lang, worker_match: &str) -> Grid {
    let text = |key: &str| t(lang, &format!("manpower.screen.{key}"));
    Grid::new("manpower-absence")
        .title(&text("absence.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::custom("worker", &text("absence.worker"))
                .frozen()
                .mobile(),
        )
        .column(Column::custom("client", &text("absence.client")).mobile())
        .column(Column::date(
            "last_present_on",
            &text("absence.last_present"),
        ))
        .column(Column::custom("days", &text("absence.days_column")).mobile())
        .column(Column::custom("contacts", &text("absence.contacts")))
        .column(Column::custom("report", &text("absence.report")))
        .column(
            Column::select(
                "status",
                &text("absence.status"),
                [
                    ("open", word(lang, "absence_status", "open")),
                    ("closed", word(lang, "absence_status", "closed")),
                ],
            )
            .computed("CASE WHEN {T}.\"closed_on\" IS NULL THEN 'open' ELSE 'closed' END")
            .badges(&[("open", "warning"), ("closed", "info")])
            .mobile(),
        )
        .column(Column::date("opened_on", &text("absence.opened")).hidden())
        .column(Column::date("closed_on", &text("absence.closed_title")).hidden())
        .column(
            Column::text("report_number", &text("absence.report_number"))
                .searchable()
                .hidden(),
        )
        .column(Column::date("report_date", &text("absence.report_date")).hidden())
        .column(
            Column::text("worker_match", &text("absence.worker"))
                .computed(worker_match)
                .hidden()
                .searchable()
                .sortable(false)
                .filterable(false),
        )
        .sort_by("-opened_on")
        .row_url("/manpower/absence/{id}")
        .cards_on_mobile()
        .exports()
        .empty_state(&text("absence.empty"), Some(&text("absence.empty_text")))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manpower/absence_cases.html")]
pub(crate) struct CasesPage {
    tx: Texts,
    cases: GridPage<AbsenceCase>,
}

/// A fact of the overview.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Fact {
    label: String,
    value: String,
}

/// A failed contact day.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ContactLine {
    day: String,
    outcome: String,
}

/// A ticket of the case.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TicketLine {
    url: String,
    number: String,
    title: String,
    status: String,
}

/// How a closed case ended.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ClosedView {
    text: String,
    pay: String,
    runs: Vec<String>,
}

/// What the person may do with the case now.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CaseActions {
    report: Act,
    close: Act,
    abscond: Act,
}

/// What the case page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manpower/absence_case.html")]
pub(crate) struct CasePage {
    tx: Texts,
    id: i64,
    title: String,
    subtitle: String,
    status: String,
    status_key: String,
    status_kind: &'static str,
    facts: Vec<Fact>,
    contacts_text: String,
    contacts: Vec<ContactLine>,
    report: String,
    closed: Option<ClosedView>,
    tickets: Vec<TicketLine>,
    actions: CaseActions,
    employee_url: String,
    deployment_url: String,
    today: String,
    report_number: String,
}

fn status_of(lang: &Lang, case: &AbsenceCase) -> (String, &'static str) {
    if case.is_open() {
        (word(lang, "absence_status", "open"), "warning")
    } else {
        (word(lang, "absence_status", "closed"), "info")
    }
}

/// *2 of 2 failed contact days*.
fn contacts_text(lang: &Lang, days: usize) -> String {
    tf(
        lang,
        "manpower.screen.absence.contacts_of",
        &[
            ("done", &days.min(FAILED_ATTEMPTS)),
            ("need", &FAILED_ATTEMPTS),
        ],
    )
}

/// The MoHRE report as the list and the page say it.
fn report_text(lang: &Lang, case: &AbsenceCase) -> String {
    match &case.report_number {
        Some(number) if !number.trim().is_empty() => tf(
            lang,
            "manpower.screen.absence.report_line",
            &[
                ("number", number),
                (
                    "date",
                    &date_or(case.report_date, &t(lang, "manpower.screen.not_set")),
                ),
            ],
        ),
        _ => t(lang, "manpower.screen.absence.report_none"),
    }
}

/// `GET /manpower/absence`: the page of all cases for a browser, the open
/// cases as JSON otherwise.
pub(crate) async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    session: Session,
    request: GridRequest,
) -> Result<Response> {
    if !wl_kernel::negotiate::wants_html(&headers) {
        return crate::pages::absence_json(&state, &user).await;
    }
    // Only HR (`absence.manage`) sees the cases, and all of them.
    absence::require_manage(&user)?;
    // The worker's name is the workforce's: with a searched text, the names
    // of the workers of the cases are matched here.
    let term = grid_lookup::searched(&request, &session, "manpower-absence");
    let mut found = Vec::new();
    if let Some(term) = term.as_deref() {
        let wanted = term.to_lowercase();
        let mut names: BTreeMap<i64, bool> = BTreeMap::new();
        for case in AbsenceCase::query().get(&state.db).await? {
            let hit = match names.get(&case.employee_id) {
                Some(hit) => *hit,
                None => {
                    let hit = worker_name(&state, &lang.locale, case.employee())
                        .await
                        .to_lowercase()
                        .contains(&wanted);
                    names.insert(case.employee_id, hit);
                    hit
                }
            };
            if hit {
                found.push(case.id);
            }
        }
    }
    let worker_match = grid_lookup::matched_sql(term.as_deref(), &["id"], &found);
    let grid = cases_grid(&lang, &worker_match);
    let query = AbsenceCase::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut lines = BTreeMap::new();
    for case in page.items() {
        let deployment = Deployment::find(&state.db, case.deployment_id).await?;
        let until = case.closed_on.unwrap_or_else(today);
        let failed = workforce(&state)
            .failed_contact_days(&state.db, case.employee(), case.last_present_on)
            .await?;
        lines.insert(
            case.id,
            json!({
                "worker": worker_name(&state, &lang.locale, case.employee()).await,
                "client": party_text(
                    &state,
                    &lang,
                    deployment.as_ref().map(|d| d.client_party_id),
                    &t(&lang, "manpower.screen.not_set"),
                )
                .await,
                "days": days_absent(case, until),
                "contacts": contacts_text(&lang, failed.len()),
                "report": report_text(&lang, case),
                "url": url(case.id),
            }),
        );
    }
    let page = page.extend(|case| lines.get(&case.id).cloned().unwrap_or_default());
    Ok(View::typed(CasesPage {
        tx: screen_texts(&lang),
        cases: page,
    })
    .into_response())
}

/// `GET /manpower/absence/{id}`: the page for a browser, the same data as
/// JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response> {
    absence::require_manage(&user)?;
    let case = AbsenceCase::find_or_404(&state.db, id).await?;
    let page = build(&state, &lang, &case).await?;
    if wl_kernel::negotiate::wants_html(&headers) {
        return Ok(View::typed(page).into_response());
    }
    let mut data = renox::serde_json::to_value(&page)?;
    if let Some(map) = data.as_object_mut() {
        map.remove("tx");
    }
    Ok(Json(data).into_response())
}

async fn build(state: &AppState, lang: &Lang, case: &AbsenceCase) -> Result<CasePage> {
    let not_set = t(lang, "manpower.screen.not_set");
    let deployment = Deployment::find(&state.db, case.deployment_id).await?;
    let worker = worker_name(state, &lang.locale, case.employee()).await;
    let client = party_text(
        state,
        lang,
        deployment.as_ref().map(|d| d.client_party_id),
        &not_set,
    )
    .await;
    let site = match &deployment {
        Some(d) => Site::find(&state.db, d.site_id)
            .await?
            .map(|site| site.name)
            .unwrap_or_else(|| not_set.clone()),
        None => not_set.clone(),
    };
    let until = case.closed_on.unwrap_or_else(today);
    let days = days_absent(case, until);
    let failed = workforce(state)
        .failed_contact_days(&state.db, case.employee(), case.last_present_on)
        .await?;
    let (status, status_kind) = status_of(lang, case);
    let fact = |label: &str, value: String| Fact {
        label: t(lang, &format!("manpower.screen.absence.{label}")),
        value,
    };
    let facts = vec![
        fact("worker", worker.clone()),
        fact("client", client.clone()),
        fact("site", site.clone()),
        fact("last_present", date_text(case.last_present_on)),
        fact("opened", date_text(case.opened_on)),
        fact(
            "days",
            tf(
                lang,
                "manpower.screen.absence.days_absent",
                &[("days", &days)],
            ),
        ),
    ];
    let contacts: Vec<ContactLine> = failed
        .iter()
        .map(|day| ContactLine {
            day: date_text(*day),
            outcome: t(lang, "manpower.screen.absence.failed"),
        })
        .collect();

    let mut tickets = Vec::new();
    let employee = Subject::new(EMPLOYEE_SUBJECT_TYPE, case.employee_id);
    for ticket in task_creator(state)
        .tickets_of(state, &employee, ABSENCE_CATEGORY)
        .await?
    {
        tickets.push(TicketLine {
            url: ticket_path(ticket.id),
            number: ticket.number,
            title: ticket.title,
            status: word(lang, "ticket_status", &ticket.status),
        });
    }

    let closed = match (case.closed_on, case.close_reason.as_deref()) {
        (Some(on), reason) => {
            let by = match case.closed_by {
                Some(id) => user_text(state, Some(id), &not_set).await,
                None => t(lang, "manpower.screen.activity.system"),
            };
            let runs = workforce(state)
                .released_pay_paths(&state.db, case.employee())
                .await?;
            Some(ClosedView {
                text: tf(
                    lang,
                    "manpower.screen.absence.closed_line",
                    &[
                        ("date", &date_text(on)),
                        ("by", &by),
                        ("reason", &reason.unwrap_or_default()),
                    ],
                ),
                pay: if runs.is_empty() {
                    t(lang, "manpower.screen.absence.pay_released")
                } else {
                    t(lang, "manpower.screen.absence.pay_proposed")
                },
                runs,
            })
        }
        _ => None,
    };

    let reportable = failed.len() >= FAILED_ATTEMPTS
        && case
            .report_number
            .as_deref()
            .is_some_and(|number| !number.trim().is_empty());
    let open = case.is_open();
    let actions = CaseActions {
        report: Act::when(open),
        close: Act::when(open),
        abscond: if open {
            Act::unless((!reportable).then(|| t(lang, "manpower.screen.absence.abscond_blocked")))
        } else {
            Act::hidden()
        },
    };

    Ok(CasePage {
        tx: screen_texts(lang),
        id: case.id,
        title: worker,
        subtitle: format!("{client}, {site}"),
        status,
        status_key: if open { "open" } else { "closed" }.to_owned(),
        status_kind,
        facts,
        contacts_text: contacts_text(lang, failed.len()),
        contacts,
        report: report_text(lang, case),
        closed,
        tickets,
        actions,
        employee_url: format!("/hr/employees/{}", case.employee_id),
        deployment_url: deployment
            .map(|d| super::deployment_page::url(d.id))
            .unwrap_or_default(),
        today: today().to_string(),
        report_number: case.report_number.clone().unwrap_or_default(),
    })
}
