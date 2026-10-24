//! The timesheet screens (wl-attendance §5.4 to §5.8; 09 Manpower
//! `MPW.ACL.09`, BL-MPW-27):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /timesheets` | `wl_attendance.timesheets` | `attendance.view`, in reach |
//! | `GET /timesheets/{id}` | `wl_attendance.timesheet` | `attendance.view` in the scope of the context |
//!
//! A `GET` answers the page when the request asks for HTML
//! ([`wl_kernel::negotiate::wants_html`]) and the same data as JSON otherwise.
//! The buttons post to the existing routes of [`crate::pages`] (`submit`,
//! `approve`, `reject`, `approve-on-behalf`, `reopen`, `adjustments`), which
//! call the services and answer with a toast and a refresh of the page; a
//! refusal is their 422, shown in the sheet that sent the form.
//!
//! Decisions:
//!
//! - **Status in words.** The machine has four states; the page tells the
//!   rest from the record: *Escalated* is `submitted` with `escalated_at`,
//!   *Deemed approved* is `approved` with the basis `deemed`, *Billed* is
//!   `locked` with `billed_by`, *Rejected* is `open` after a `reject`.
//! - **Scope.** The list shows the timesheets whose context is in the reach of
//!   the person ([`wl_kernel::scopes::reach`]), so an account manager sees the
//!   clients they look after and a client approver, in the portal of BL-MPW-28
//!   that reuses this page, only their own client.
//! - **The context** (the client, the work in words) is asked of the app's
//!   [`ApproverResolver::describe_context`](wl_kernel::timesheets::ApproverResolver::describe_context).
//! - **The signed sheet** of *Approve on behalf* is a document of
//!   wl-documents. Staff upload it under the documents of the timesheet
//!   (`/documents/of/attendance.timesheet/{id}`) and enter its number; this
//!   crate does not depend on wl-documents and cannot list its documents.
//! - **Reopen** is offered to those who hold `timesheets.reopen` globally: the
//!   transition checks it that way, so a scoped holder would only meet a 403.
//! - **Hours** are `8.00 h`, days `Mon 02 Nov`.

use std::collections::BTreeMap;

use renox::axum::http::HeaderMap;
use renox::chrono::{Datelike, NaiveDate};
use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::dubai_date;
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;
use wl_kernel::negotiate::wants_html;
use wl_kernel::timesheets::{ContextInfo, approver_resolver};
use wl_kernel::workforce::EmployeeId;

use crate::access;
use crate::approval::{self, ApprovalEvidence};
use crate::config::config_of;
use crate::crew::name_of;
use crate::entry::{Category, TimesheetEntry};
use crate::permissions;
use crate::texts;
use crate::timesheet::{ApprovalBasis, ApproverKind, Timesheet, TimesheetKind, TimesheetStatus};
use crate::timesheet_texts::PREFIX;

/// The texts a template reads as `tx['status.draft']`.
type Texts = BTreeMap<String, String>;

/// The module's timesheet pages.
pub fn routes() -> Routes {
    wl_kernel::scopes::anywhere(
        Routes::new()
            .get("/timesheets", list)
            .name("wl_attendance.timesheets"),
        permissions::VIEW,
    )
    // The page of one timesheet is also the client approver's (and viewer's)
    // in the portal (BL-MPW-28): `ensure_reach` lets in who acts for the
    // client of the timesheet's context.
    .merge(
        Routes::new()
            .get("/timesheets/{id}", show)
            .name("wl_attendance.timesheet")
            .require_auth(),
    )
}

/// The client's own list of timesheets in the portal (BL-MPW-28).
pub const PORTAL_LIST: &str = "/portal/manpower/timesheets";

/// The page of a timesheet.
pub fn url(id: i64) -> String {
    format!("/timesheets/{id}")
}

/// The text `key` (below `wl_attendance.sheet.`).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("{PREFIX}{key}"))
}

/// The text `key` with `{{ name }}` filled.
fn filled(lang: &Lang, key: &str, values: &[(&str, &str)]) -> String {
    let mut out = text(lang, key);
    for (name, value) in values {
        out = out.replace(&format!("{{{{ {name} }}}}"), value);
    }
    out
}

/// Every text of the pages, keyed without the prefix.
fn screen_texts(lang: &Lang) -> Texts {
    crate::timesheet_texts::TEXTS
        .iter()
        .filter_map(|(key, _)| {
            key.strip_prefix(PREFIX)
                .map(|rest| (rest.to_owned(), texts::t(lang, key)))
        })
        .collect()
}

/// A date as `01 Nov 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%d %b %Y").to_string()
}

/// Minutes as `8.00 h`; a sign is kept.
fn hours_text(minutes: i64) -> String {
    let sign = if minutes < 0 { "-" } else { "" };
    let abs = minutes.abs();
    format!("{sign}{}.{:02} h", abs / 60, (abs % 60) * 100 / 60)
}

/// The words of a standing and the tone of its badge.
fn kind_of(standing: &str) -> &'static str {
    match standing {
        "submitted" => "info",
        "escalated" | "rejected" => "warning",
        "approved" | "deemed_approved" | "billed" | "locked" => "success",
        _ => "info",
    }
}

/// Where a timesheet stands, as the key of its word. `sent_back` is whether
/// its latest change was a `reject`.
fn standing(sheet: &Timesheet, sent_back: bool) -> &'static str {
    match sheet.status {
        TimesheetStatus::Open if sent_back => "rejected",
        TimesheetStatus::Open => "draft",
        TimesheetStatus::Submitted if sheet.escalated_at.is_some() => "escalated",
        TimesheetStatus::Submitted => "submitted",
        TimesheetStatus::Approved if sheet.approval_basis == Some(ApprovalBasis::Deemed) => {
            "deemed_approved"
        }
        TimesheetStatus::Approved => "approved",
        TimesheetStatus::Locked if sheet.billed_by.is_some() => "billed",
        TimesheetStatus::Locked => "locked",
    }
}

/// Whether the latest recorded change of the timesheet is a `reject`.
async fn sent_back(db: &Db, sheet: &Timesheet) -> Result<bool> {
    if sheet.status != TimesheetStatus::Open {
        return Ok(false);
    }
    let history = renox::state_machine::history(db, Timesheet::TABLE, sheet.id).await?;
    Ok(history
        .last()
        .is_some_and(|row| row.transition == approval::REJECT))
}

/// The sum of the minutes of `entries`.
fn minutes_of(entries: &[&TimesheetEntry]) -> i64 {
    entries
        .iter()
        .map(|entry| {
            entry.normal_minutes
                + entry.overtime_minutes
                + entry.night_minutes
                + entry.rest_day_minutes
                + entry.public_holiday_minutes
        })
        .sum()
}

/// What the list can be narrowed by.
#[derive(Deserialize, Default, Debug, Clone)]
pub struct Filter {
    /// A client, as a party id.
    #[serde(default)]
    client: String,
    /// A month, `2026-11`.
    #[serde(default)]
    month: String,
    /// A standing: `draft`, `submitted`, `escalated`, `approved`,
    /// `deemed_approved`, `locked`, `billed` or `rejected`.
    #[serde(default)]
    status: String,
}

/// The standings, in the order of the filter.
const STANDINGS: [&str; 8] = [
    "draft",
    "submitted",
    "escalated",
    "approved",
    "deemed_approved",
    "locked",
    "billed",
    "rejected",
];

/// A timesheet in the list (the JSON of the route).
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Row {
    id: i64,
    url: String,
    worker: String,
    client: String,
    work: String,
    period: String,
    month: String,
    hours: String,
    status: String,
    status_key: &'static str,
    adjustment: bool,
}

/// The list page.
#[derive(Serialize, ViewData)]
#[view(template = "wl-attendance/timesheets.html")]
pub(crate) struct ListPage {
    tx: Texts,
    rows: GridPage<Timesheet>,
    clients: Vec<(String, String)>,
    client: String,
    month: String,
    status: String,
}

/// The context of `sheet` in words, from the app.
async fn describe(state: &AppState, sheet: &Timesheet) -> Option<ContextInfo> {
    let context = sheet.context()?;
    approver_resolver(state)
        .describe_context(state, &context)
        .await
        .ok()
        .flatten()
}

/// The standing of a timesheet as SQL, the same words as [`standing`]: the
/// state, `escalated_at`, the basis, `billed_by`, and whether the latest
/// recorded change was a `reject`.
fn standing_sql() -> String {
    format!(
        "(CASE {{T}}.\"status\" \
         WHEN 'open' THEN CASE WHEN (SELECT \"transition\" FROM \"state_transitions\" \
           WHERE \"model\" = '{table}' AND \"record_key\" = CAST({{T}}.\"id\" AS TEXT) \
           ORDER BY \"id\" DESC LIMIT 1) = '{reject}' THEN 'rejected' ELSE 'draft' END \
         WHEN 'submitted' THEN CASE WHEN {{T}}.\"escalated_at\" IS NOT NULL THEN 'escalated' ELSE 'submitted' END \
         WHEN 'approved' THEN CASE WHEN {{T}}.\"approval_basis\" = 'deemed' THEN 'deemed_approved' ELSE 'approved' END \
         ELSE CASE WHEN {{T}}.\"billed_by\" IS NOT NULL THEN 'billed' ELSE 'locked' END END)",
        table = Timesheet::TABLE,
        reject = approval::REJECT,
    )
}

/// `text` as a quoted SQL literal: quotes doubled; no `?` (a placeholder) and
/// no backslash, at most 200 characters.
fn literal(text: &str) -> String {
    let clean: String = text
        .chars()
        .filter(|c| !matches!(c, '?' | '\\'))
        .take(200)
        .collect();
    format!("'{}'", clean.replace('\'', "''"))
}

/// The SQL of a text column whose words are worked out in Rust for each
/// timesheet (`words`: the id to the words), empty for the others: what lets
/// the grid search, sort, filter and export the names that live in another
/// module (the worker in the directory, the client and work in the app).
fn worded_sql(words: &BTreeMap<i64, String>) -> String {
    if words.is_empty() {
        return "''".to_owned();
    }
    let arms = words
        .iter()
        .map(|(id, text)| format!("WHEN {id} THEN {}", literal(text)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("(CASE {{T}}.\"id\" {arms} ELSE '' END)")
}

/// The words of the worker, client and work of the listed timesheets, as the
/// SQL of their columns (see [`worded_sql`]).
pub(crate) struct SheetNames {
    worker: String,
    client: String,
    work: String,
}

/// The timesheets grid: the worker (a link to the sheet), client and work in
/// words (`names`, from the directory and the app) searched, sorted and
/// exported like any text; filters by standing, kind and the dates of the
/// period; the hours drawn by the template. The client and the month of the
/// header are page filters that feed the query (they stay in the address,
/// BL-MPW-27). Exported by who submits or approves timesheets.
pub(crate) fn sheets_grid(lang: &Lang, names: &SheetNames, can_export: bool) -> Grid {
    let t = |key: &str| text(lang, key);
    let standings: Vec<(String, String)> = STANDINGS
        .iter()
        .map(|key| ((*key).to_owned(), t(&format!("status.{key}"))))
        .collect();
    let kinds = vec![
        ("regular".to_owned(), t("fact.kind_regular")),
        ("adjustment".to_owned(), t("adjustment_tag")),
    ];
    let grid = Grid::new("attendance-timesheets")
        .title(&t("title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("worker", &t("worker"))
                .computed(&names.worker)
                .frozen()
                .mobile()
                .searchable()
                .link("/timesheets/{id}"),
        )
        .column(
            Column::text("client", &t("client"))
                .computed(&names.client)
                .mobile()
                .searchable(),
        )
        .column(
            Column::text("work", &t("work"))
                .computed(&names.work)
                .searchable(),
        )
        .column(Column::date("period_from", &t("period_from")).hidden())
        .column(Column::date("period_to", &t("period_to")).hidden())
        .column(Column::custom("period", &t("period")).mobile())
        .column(Column::custom("hours", &t("hours")).mobile())
        .column(
            Column::number("minutes", &t("minutes"))
                .computed(&format!(
                    "(SELECT COALESCE(SUM(\"normal_minutes\" + \"overtime_minutes\" + \"night_minutes\" \
                     + \"rest_day_minutes\" + \"public_holiday_minutes\"), 0) \
                     FROM \"{}\" WHERE \"timesheet_id\" = {{T}}.\"id\")",
                    TimesheetEntry::TABLE
                ))
                .hidden()
                .summary(Summary::Sum),
        )
        .column(
            Column::select("standing", &t("status"), standings)
                .computed(&standing_sql())
                .badges(&[
                    ("draft", "info"),
                    ("submitted", "info"),
                    ("escalated", "warning"),
                    ("rejected", "warning"),
                    ("approved", "success"),
                    ("deemed_approved", "success"),
                    ("locked", "success"),
                    ("billed", "success"),
                ])
                .mobile(),
        )
        .column(Column::select("kind", &t("fact.kind"), kinds).hidden())
        .column(Column::text("reason", &t("reason")).hidden().searchable())
        .sort_by("-period_from")
        .row_url("/timesheets/{id}")
        .cards_on_mobile()
        .empty_state(&t("empty"), Some(&t("empty_text")));
    if can_export { grid.exports() } else { grid }
}

/// `GET /timesheets`: the timesheets in the person's reach, as a page or JSON.
/// The client, the month and (as before) the standing in the address narrow
/// the query; the grid searches, sorts and pages what is left.
async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
    request: GridRequest,
) -> Result<Response> {
    let reach = wl_kernel::scopes::reach(&state.db, &user, permissions::VIEW).await?;
    let sheets = Timesheet::query()
        .order_by_desc(Timesheet::PERIOD_FROM)
        .order_by_desc(Timesheet::ID)
        .get(&state.db)
        .await?;
    let wanted_month = filter.month.trim();
    let mut kept: Vec<(&Timesheet, Option<ContextInfo>, &'static str)> = Vec::new();
    let mut clients: Vec<(String, String)> = Vec::new();
    for sheet in &sheets {
        if !reach.is_all() {
            let Some(context) = sheet.context() else {
                continue;
            };
            if !reach.contains(&state.db, &context).await? {
                continue;
            }
        }
        let info = describe(&state, sheet).await;
        if let Some(info) = &info
            && let Some(party) = info.client_party
        {
            let id = party.to_string();
            if !clients.iter().any(|(value, _)| *value == id) {
                clients.push((id, info.client.clone()));
            }
        }
        if !filter.client.is_empty()
            && info
                .as_ref()
                .and_then(|info| info.client_party)
                .map(|party| party.to_string())
                .as_deref()
                != Some(filter.client.as_str())
        {
            continue;
        }
        if !wanted_month.is_empty() {
            let month_of = |day: NaiveDate| format!("{}-{:02}", day.year(), day.month());
            if month_of(sheet.period_from) != wanted_month
                && month_of(sheet.period_to) != wanted_month
            {
                continue;
            }
        }
        let now = standing(sheet, sent_back(&state.db, sheet).await?);
        if !filter.status.is_empty() && filter.status != now {
            continue;
        }
        kept.push((sheet, info, now));
    }
    // A grid refresh (htmx) and an export want the page too.
    let wants_page = wants_html(&headers)
        || headers.contains_key("hx-request")
        || request.param("export").is_some();
    if !wants_page {
        let ids: Vec<i64> = kept.iter().map(|(sheet, _, _)| sheet.id).collect();
        let entries = TimesheetEntry::query()
            .where_in("timesheet_id", ids)
            .get(&state.db)
            .await?;
        let mut rows = Vec::new();
        for (sheet, info, now) in kept {
            let own: Vec<&TimesheetEntry> = entries
                .iter()
                .filter(|entry| entry.timesheet_id == sheet.id)
                .collect();
            rows.push(Row {
                id: sheet.id,
                url: url(sheet.id),
                worker: name_of(&state, &lang, sheet.owner()).await,
                client: client_text(info.as_ref()),
                work: info.map_or_else(|| "-".to_owned(), |info| info.label),
                period: period_text(sheet),
                month: sheet.period_to.format("%b %Y").to_string(),
                hours: hours_text(minutes_of(&own)),
                status: text(&lang, &format!("status.{now}")),
                status_key: now,
                adjustment: sheet.is_adjustment(),
            });
        }
        return Ok(Json(renox::serde_json::to_value(&rows)?).into_response());
    }
    let ids: Vec<i64> = kept.iter().map(|(sheet, _, _)| sheet.id).collect();
    // The names of every listed sheet, so that the grid searches and sorts
    // them all, not only the rows of the page.
    let mut workers: BTreeMap<i64, String> = BTreeMap::new();
    let mut clients_of: BTreeMap<i64, String> = BTreeMap::new();
    let mut works: BTreeMap<i64, String> = BTreeMap::new();
    let mut owners: BTreeMap<i64, String> = BTreeMap::new();
    for (sheet, info, _) in &kept {
        let worker = match owners.get(&sheet.owner_employee_id) {
            Some(name) => name.clone(),
            None => {
                let name = name_of(&state, &lang, sheet.owner()).await;
                owners.insert(sheet.owner_employee_id, name.clone());
                name
            }
        };
        workers.insert(sheet.id, worker);
        clients_of.insert(sheet.id, client_text(info.as_ref()));
        works.insert(
            sheet.id,
            info.as_ref()
                .map_or_else(|| "-".to_owned(), |info| info.label.clone()),
        );
    }
    let names = SheetNames {
        worker: worded_sql(&workers),
        client: worded_sql(&clients_of),
        work: worded_sql(&works),
    };
    let can_export = user.has_permission_anywhere(permissions::TIMESHEETS_SUBMIT)
        || user.has_permission_anywhere(permissions::TIMESHEETS_APPROVE);
    let grid = sheets_grid(&lang, &names, can_export);
    let query =
        Timesheet::query().where_(Timesheet::ID.is_in(if ids.is_empty() { vec![-1] } else { ids }));
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    // The period and hours of the page's rows.
    let on_page: Vec<i64> = page.items().iter().map(|sheet| sheet.id).collect();
    let entries = TimesheetEntry::query()
        .where_in("timesheet_id", on_page)
        .get(&state.db)
        .await?;
    let mut extra: BTreeMap<i64, renox::serde_json::Value> = BTreeMap::new();
    for sheet in page.items() {
        let own: Vec<&TimesheetEntry> = entries
            .iter()
            .filter(|entry| entry.timesheet_id == sheet.id)
            .collect();
        extra.insert(
            sheet.id,
            renox::serde_json::json!({
                "period": period_text(sheet),
                "hours": hours_text(minutes_of(&own)),
                "url": url(sheet.id),
            }),
        );
    }
    let rows = page.extend(|sheet| extra.get(&sheet.id).cloned().unwrap_or_default());
    let mut options = vec![(String::new(), text(&lang, "all_clients"))];
    options.extend(clients);
    Ok(View::typed(ListPage {
        tx: screen_texts(&lang),
        rows,
        clients: options,
        client: filter.client.clone(),
        month: filter.month.clone(),
        status: filter.status.clone(),
    })
    .into_response())
}

/// The client of a context in words, `-` without one.
fn client_text(info: Option<&ContextInfo>) -> String {
    info.map(|info| info.client.clone())
        .filter(|client| !client.is_empty())
        .unwrap_or_else(|| "-".to_owned())
}

/// The period of a timesheet as `01 Nov 2026 - 30 Nov 2026`.
fn period_text(sheet: &Timesheet) -> String {
    format!(
        "{} - {}",
        date_text(sheet.period_from),
        date_text(sheet.period_to)
    )
}

/// One fact of the overview.
#[derive(Serialize, Debug, Clone)]
struct Fact {
    label: String,
    value: String,
}

/// A day of the grid.
#[derive(Serialize, Debug, Clone)]
struct DayRow {
    day: String,
    normal: String,
    overtime: String,
    night: String,
    rest_day: String,
    public_holiday: String,
    total: String,
}

/// A line of the history.
#[derive(Serialize, Debug, Clone)]
struct HistoryRow {
    at: String,
    what: String,
    by: String,
    reason: String,
}

/// A signed sheet attached to an approval on behalf.
#[derive(Serialize, Debug, Clone)]
struct Signed {
    label: String,
    url: String,
}

/// An adjustment of the timesheet.
#[derive(Serialize, Debug, Clone)]
struct AdjustmentRow {
    url: String,
    period: String,
    reason: String,
    status: String,
    status_kind: &'static str,
}

/// What a person can do with the timesheet now: shown when they may, with the
/// reason when a rule blocks it.
#[derive(Serialize, Debug, Clone, Default)]
struct Actions {
    submit: bool,
    approve: bool,
    approve_blocked: String,
    reject: bool,
    on_behalf: bool,
    on_behalf_blocked: String,
    reopen: bool,
    adjust: bool,
}

/// The page of a timesheet.
#[derive(Serialize, ViewData)]
#[view(template = "wl-attendance/timesheet.html")]
pub(crate) struct SheetPage {
    tx: Texts,
    id: i64,
    title: String,
    status: String,
    status_key: &'static str,
    status_kind: &'static str,
    facts: Vec<Fact>,
    /// The sentences that say how it came to stand where it does.
    notes: Vec<String>,
    days: Vec<DayRow>,
    totals: DayRow,
    history: Vec<HistoryRow>,
    signed: Vec<Signed>,
    adjustments: Vec<AdjustmentRow>,
    original_url: String,
    invoice_url: String,
    documents_url: String,
    actions: Actions,
    origins: Vec<(String, String)>,
    affects: Vec<(String, String)>,
    categories: Vec<(String, String)>,
    period_from: String,
    period_to: String,
    /// Where *back* goes: the list for staff, the portal's list for the
    /// client's own user.
    back_url: String,
}

/// The sheet's entries as the days of the grid, with the totals row.
fn grid(entries: &[TimesheetEntry]) -> (Vec<DayRow>, DayRow) {
    let mut by_day: BTreeMap<NaiveDate, [i64; 5]> = BTreeMap::new();
    for entry in entries {
        let slot = by_day.entry(entry.date).or_default();
        slot[0] += entry.normal_minutes;
        slot[1] += entry.overtime_minutes;
        slot[2] += entry.night_minutes;
        slot[3] += entry.rest_day_minutes;
        slot[4] += entry.public_holiday_minutes;
    }
    let row = |day: String, minutes: [i64; 5]| DayRow {
        day,
        normal: hours_text(minutes[0]),
        overtime: hours_text(minutes[1]),
        night: hours_text(minutes[2]),
        rest_day: hours_text(minutes[3]),
        public_holiday: hours_text(minutes[4]),
        total: hours_text(minutes.iter().sum()),
    };
    let mut totals = [0_i64; 5];
    let mut days = Vec::new();
    for (day, minutes) in &by_day {
        for (sum, value) in totals.iter_mut().zip(minutes) {
            *sum += value;
        }
        days.push(row(day.format("%a %d %b").to_string(), *minutes));
    }
    (days, row(String::new(), totals))
}

/// How a person reaches a timesheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    /// As staff: the context is in their reach.
    Staff,
    /// As a user of the client the timesheet is for, in the portal.
    Portal,
}

/// How `user` reaches `sheet`. Staff (`attendance.view` somewhere) need the
/// context in their reach, else `Forbidden`. Anyone else must act in the portal
/// for the client of the context (`MPW.ACL.01`); a portal user of *another*
/// client gets `NotFound`, as if the sheet did not exist, and a person with no
/// portal access `Forbidden`.
async fn ensure_reach(state: &AppState, user: &AuthUser, sheet: &Timesheet) -> Result<Seen> {
    if has_view(user) {
        let reach = wl_kernel::scopes::reach(&state.db, user, permissions::VIEW).await?;
        if reach.is_all() {
            return Ok(Seen::Staff);
        }
        return match sheet.context() {
            Some(context) if reach.contains(&state.db, &context).await? => Ok(Seen::Staff),
            _ => Err(Error::Forbidden),
        };
    }
    match standing_in_portal(state, user, sheet).await? {
        Some(true) => Ok(Seen::Portal),
        Some(false) => Err(Error::NotFound),
        None => Err(Error::Forbidden),
    }
}

/// Whether the person holds `attendance.view` globally or in some scope.
fn has_view(user: &AuthUser) -> bool {
    user.allows(permissions::VIEW) || user.has_permission_anywhere(permissions::VIEW)
}

/// `Some(true)` when `user` acts in the portal for the client of `sheet`,
/// `Some(false)` when they act for other clients only, `None` when for none.
async fn standing_in_portal(
    state: &AppState,
    user: &AuthUser,
    sheet: &Timesheet,
) -> Result<Option<bool>> {
    let users = wl_kernel::portal::portal_users(state);
    let day = wl_kernel::documents::today();
    if let Some(party) = describe(state, sheet)
        .await
        .and_then(|info| info.client_party)
        && users
            .acting_for(&state.db, user.id, wl_kernel::PartyId(party), day)
            .await?
            .is_some()
    {
        return Ok(Some(true));
    }
    Ok(
        if users.parties_of(&state.db, user.id, day).await?.is_empty() {
            None
        } else {
            Some(false)
        },
    )
}

/// For the actions on a timesheet: a person who acts in the portal only for
/// *other* clients meets `NotFound`, as if the sheet did not exist
/// (`MPW.ACL.01`). Everyone else passes; the service decides.
///
/// # Errors
///
/// `NotFound` as above.
pub(crate) async fn not_of_another_client(
    state: &AppState,
    user: &AuthUser,
    sheet: &Timesheet,
) -> Result {
    if has_view(user) {
        return Ok(());
    }
    match standing_in_portal(state, user, sheet).await? {
        Some(false) => Err(Error::NotFound),
        _ => Ok(()),
    }
}

/// What `user` may do with `sheet` now.
async fn actions(
    state: &AppState,
    user: &AuthUser,
    sheet: &Timesheet,
    lang: &Lang,
) -> Result<Actions> {
    let context = sheet.context();
    let holds = |permission: &'static str| {
        let context = context.clone();
        async move { access::holds_in_context(state, user.id, permission, context.as_ref()).await }
    };
    let mut out = Actions::default();
    match sheet.status {
        TimesheetStatus::Open => {
            out.submit = holds(permissions::TIMESHEETS_SUBMIT).await?;
        }
        TimesheetStatus::Submitted => {
            let kind = approval::approver_kind_for(state, user.id, sheet).await?;
            let on_behalf = holds(permissions::TIMESHEETS_APPROVE_ON_BEHALF).await?;
            let own_adjustment = sheet.is_adjustment() && sheet.created_by == Some(user.id);
            out.approve = kind == Some(sheet.approver_kind);
            if out.approve && own_adjustment {
                out.approve_blocked = text(lang, "action.creator_blocked");
            }
            out.reject = kind.is_some() || on_behalf;
            out.on_behalf = on_behalf && sheet.approver_kind == ApproverKind::External;
            if out.on_behalf && own_adjustment {
                out.on_behalf_blocked = text(lang, "action.creator_blocked");
            }
        }
        TimesheetStatus::Approved => {
            out.reopen = !sheet.consumed() && user.has_permission(permissions::TIMESHEETS_REOPEN);
        }
        TimesheetStatus::Locked => {
            out.adjust = !sheet.is_adjustment() && holds(permissions::TIMESHEETS_ADJUST).await?;
        }
    }
    Ok(out)
}

/// The word of a transition of the history.
fn transition_text(lang: &Lang, name: &str) -> String {
    let key = format!("{PREFIX}tr.{name}");
    let found = texts::t(lang, &key);
    if found != key {
        return found;
    }
    let spaced = name.replace('_', " ");
    let mut chars = spaced.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// The name of user `id`, or `fallback`.
async fn user_name(db: &Db, id: Option<i64>, fallback: &str) -> String {
    let Some(id) = id else {
        return fallback.to_owned();
    };
    User::find(db, id)
        .await
        .ok()
        .flatten()
        .map_or_else(|| fallback.to_owned(), |user| user.name)
}

/// Who did what, as the viewer may read it. The client sees the names of its
/// own people and *The agency* for the staff (their names are not the
/// client's to know); staff see every name.
struct Names<'a> {
    state: &'a AppState,
    seen: Seen,
    viewer: i64,
    client_party: Option<i64>,
    system: String,
    agency: String,
}

impl Names<'_> {
    async fn of(&self, actor: Option<i64>) -> String {
        let Some(id) = actor else {
            return self.system.clone();
        };
        if self.seen == Seen::Portal && id != self.viewer && !self.is_of_the_client(id).await {
            return self.agency.clone();
        }
        user_name(&self.state.db, actor, &self.system).await
    }

    async fn is_of_the_client(&self, id: i64) -> bool {
        let Some(party) = self.client_party else {
            return false;
        };
        wl_kernel::portal::portal_users(self.state)
            .acting_for(
                &self.state.db,
                id,
                wl_kernel::PartyId(party),
                wl_kernel::documents::today(),
            )
            .await
            .ok()
            .flatten()
            .is_some()
    }
}

/// `GET /timesheets/{id}`: one timesheet, as a page or JSON.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response> {
    let sheet = Timesheet::find_or_404(&state.db, id).await?;
    let seen = ensure_reach(&state, &user, &sheet).await?;
    let page = build(&state, &user, &lang, &sheet, seen).await?;
    if wants_html(&headers) {
        return Ok(View::typed(page).into_response());
    }
    let mut data = renox::serde_json::to_value(&page)?;
    if let Some(map) = data.as_object_mut() {
        map.remove("tx");
    }
    Ok(Json(data).into_response())
}

async fn build(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    sheet: &Timesheet,
    seen: Seen,
) -> Result<SheetPage> {
    let db = &state.db;
    let system = text(lang, "history.system");
    let not_yet = text(lang, "fact.not_yet");
    let names = Names {
        state,
        seen,
        viewer: user.id,
        client_party: if seen == Seen::Portal {
            describe(state, sheet)
                .await
                .and_then(|info| info.client_party)
        } else {
            None
        },
        system: system.clone(),
        agency: text(lang, "history.agency"),
    };
    let history = renox::state_machine::history(db, Timesheet::TABLE, sheet.id).await?;
    let was_sent_back = history
        .last()
        .is_some_and(|row| row.transition == approval::REJECT)
        && sheet.status == TimesheetStatus::Open;
    let now = standing(sheet, was_sent_back);
    let info = describe(state, sheet).await;
    let worker = name_of(state, lang, EmployeeId(sheet.owner_employee_id)).await;
    let entries = TimesheetEntry::query()
        .where_(TimesheetEntry::TIMESHEET_ID.eq(sheet.id))
        .order_by(TimesheetEntry::DATE)
        .order_by(TimesheetEntry::ID)
        .get(db)
        .await?;
    let (days, totals) = grid(&entries);

    // The facts.
    let mut facts = vec![
        Fact {
            label: text(lang, "worker"),
            value: worker.clone(),
        },
        Fact {
            label: text(lang, "period"),
            value: format!(
                "{} - {}",
                date_text(sheet.period_from),
                date_text(sheet.period_to)
            ),
        },
        Fact {
            label: text(lang, "hours"),
            value: totals.total.clone(),
        },
    ];
    if let Some(info) = &info {
        if !info.client.is_empty() {
            facts.insert(
                1,
                Fact {
                    label: text(lang, "client"),
                    value: info.client.clone(),
                },
            );
        }
        facts.insert(
            2,
            Fact {
                label: text(lang, "work"),
                value: info.label.clone(),
            },
        );
    }
    facts.push(Fact {
        label: text(lang, "fact.approver"),
        value: text(
            lang,
            match sheet.approver_kind {
                ApproverKind::Internal => "fact.approver_internal",
                ApproverKind::External => "fact.approver_external",
            },
        ),
    });
    facts.push(Fact {
        label: text(lang, "fact.kind"),
        value: text(
            lang,
            match sheet.kind {
                TimesheetKind::Regular => "fact.kind_regular",
                TimesheetKind::Adjustment => "fact.kind_adjustment",
            },
        ),
    });
    facts.push(Fact {
        label: text(lang, "fact.submitted"),
        value: match sheet.submitted_at {
            Some(at) => filled(
                lang,
                "fact.on",
                &[
                    ("date", &date_text(dubai_date(at))),
                    ("who", &names.of(sheet.submitted_by).await),
                ],
            ),
            None => not_yet.clone(),
        },
    });
    facts.push(Fact {
        label: text(lang, "fact.approved"),
        value: match (sheet.approved_at, sheet.approval_basis) {
            (Some(at), Some(ApprovalBasis::Deemed)) => filled(
                lang,
                "fact.deemed_by",
                &[("date", &date_text(dubai_date(at)))],
            ),
            (Some(at), _) => filled(
                lang,
                "fact.on",
                &[
                    ("date", &date_text(dubai_date(at))),
                    ("who", &names.of(sheet.approved_by).await),
                ],
            ),
            _ => not_yet.clone(),
        },
    });
    if sheet.is_adjustment() {
        if let Some(origin) = sheet.origin {
            facts.push(Fact {
                label: text(lang, "fact.origin"),
                value: text(lang, &format!("origin.{}", origin.as_str())),
            });
        }
        if let Some(affects) = sheet.affects {
            facts.push(Fact {
                label: text(lang, "fact.affects"),
                value: text(lang, &format!("affects.{}", affects.as_str())),
            });
        }
    }

    // How it came to stand where it does.
    let mut notes = Vec::new();
    match now {
        "escalated" => {
            if let Some(at) = sheet.escalated_at {
                notes.push(filled(
                    lang,
                    "note.escalated",
                    &[
                        ("date", &dubai_date(at).format("%d %b").to_string()),
                        ("days", &config_of(state).escalation_days.to_string()),
                    ],
                ));
            }
        }
        "deemed_approved" => {
            let date = sheet
                .approved_at
                .map(|at| date_text(dubai_date(at)))
                .unwrap_or_default();
            let days = match (sheet.submitted_at, sheet.context()) {
                (Some(at), Some(context)) => approver_resolver(state)
                    .deemed_approval_days_at(state, Some(&context), at)
                    .await
                    .ok()
                    .flatten(),
                _ => None,
            };
            notes.push(match days {
                Some(days) => filled(
                    lang,
                    "note.deemed",
                    &[("date", &date), ("days", &days.to_string())],
                ),
                None => filled(lang, "note.deemed_plain", &[("date", &date)]),
            });
        }
        "rejected" => {
            if let Some(row) = history.last() {
                notes.push(filled(
                    lang,
                    "note.rejected",
                    &[
                        ("date", &date_text(dubai_date(row.at))),
                        ("reason", row.reason.as_deref().unwrap_or_default()),
                    ],
                ));
            }
        }
        "locked" => notes.push(text(lang, "note.locked")),
        _ => {}
    }
    if sheet.approval_basis == Some(ApprovalBasis::OnBehalf) {
        notes.push(text(lang, "note.on_behalf"));
    }
    let mut invoice_url = String::new();
    if now == "billed"
        && let Some(reference) = sheet.billed_by.as_deref()
    {
        let mut label = reference.to_owned();
        if let Ok(subject) = reference.parse::<Subject>() {
            if let Ok(Some(found)) = wl_kernel::documents::subject_label(db, &subject).await {
                label = found;
            }
            if subject.subject_type == INVOICE_SUBJECT_TYPE {
                invoice_url = format!("/invoices/{}", subject.subject_id);
            }
        }
        notes.push(filled(lang, "note.billed", &[("invoice", &label)]));
    }
    if sheet.is_adjustment()
        && let Some(reason) = sheet.reason.as_deref()
    {
        notes.push(filled(lang, "note.adjustment", &[("reason", reason)]));
    }

    // The history.
    let mut rows = Vec::new();
    for row in &history {
        rows.push(HistoryRow {
            at: row
                .at
                .with_timezone(&wl_kernel::BusinessCalendar::offset())
                .format("%d %b %Y %H:%M")
                .to_string(),
            what: transition_text(lang, &row.transition),
            by: names.of(row.actor_id).await,
            reason: row.reason.clone().unwrap_or_default(),
        });
    }

    // The signed sheets of an approval on behalf.
    // The file is the agency's evidence (`documents.review`): the client's own
    // approver is not offered a link he cannot open.
    let evidence = if seen == Seen::Portal {
        Vec::new()
    } else {
        ApprovalEvidence::of_timesheet(db, sheet.id).await?
    };
    let signed = evidence
        .into_iter()
        .map(|evidence| Signed {
            label: filled(
                lang,
                "history.signed",
                &[
                    ("name", &evidence.approver_name),
                    ("date", &date_text(evidence.signed_on)),
                ],
            ),
            url: format!("/documents/{}/file", evidence.document_id),
        })
        .collect();

    // The adjustments, and the timesheet an adjustment corrects.
    let mut adjustments = Vec::new();
    for adjustment in sheet.adjusted_by(db).await? {
        let adjusted = standing(&adjustment, sent_back(db, &adjustment).await?);
        adjustments.push(AdjustmentRow {
            url: url(adjustment.id),
            period: format!(
                "{} - {}",
                date_text(adjustment.period_from),
                date_text(adjustment.period_to)
            ),
            reason: adjustment.reason.clone().unwrap_or_default(),
            status: text(lang, &format!("status.{adjusted}")),
            status_kind: kind_of(adjusted),
        });
    }

    let category = |c: Category| {
        (
            c.as_str().to_owned(),
            text(lang, &format!("grid.{}", c.as_str())),
        )
    };
    Ok(SheetPage {
        tx: screen_texts(lang),
        id: sheet.id,
        title: filled(lang, "detail_title", &[("worker", &worker)]),
        status: text(lang, &format!("status.{now}")),
        status_key: now,
        status_kind: kind_of(now),
        facts,
        notes,
        days,
        totals,
        history: rows,
        signed,
        adjustments,
        original_url: sheet.adjusts_id.map(url).unwrap_or_default(),
        invoice_url: if seen == Seen::Portal {
            String::new()
        } else {
            invoice_url
        },
        back_url: if seen == Seen::Portal {
            PORTAL_LIST.to_owned()
        } else {
            "/timesheets".to_owned()
        },
        documents_url: format!(
            "/documents/of/{}/{}",
            sheet.subject().subject_type,
            sheet.id
        ),
        actions: actions(state, user, sheet, lang).await?,
        origins: ["client_dispute", "attendance_error", "payroll_error"]
            .iter()
            .map(|key| ((*key).to_owned(), text(lang, &format!("origin.{key}"))))
            .collect(),
        affects: ["billing", "payroll", "both"]
            .iter()
            .map(|key| ((*key).to_owned(), text(lang, &format!("affects.{key}"))))
            .collect(),
        categories: Category::ALL.iter().map(|c| category(*c)).collect(),
        period_from: sheet.period_from.to_string(),
        period_to: sheet.period_to.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `tx['name']` a template reads is a text of the package: a missing
    /// one would show as nothing on the page.
    #[test]
    fn every_text_a_timesheet_template_reads_exists() {
        let keys: Vec<&str> = crate::timesheet_texts::TEXTS
            .iter()
            .filter_map(|(key, _)| key.strip_prefix(PREFIX))
            .collect();
        for (name, source) in [
            (
                "timesheets.html",
                include_str!("../views/wl-attendance/timesheets.html"),
            ),
            (
                "timesheet.html",
                include_str!("../views/wl-attendance/timesheet.html"),
            ),
        ] {
            let mut rest = source;
            while let Some(at) = rest.find("tx['") {
                rest = &rest[at + 4..];
                let end = rest.find('\'').expect("a closed key");
                assert!(
                    keys.contains(&&rest[..end]),
                    "{name} reads `{}`, which has no text",
                    &rest[..end]
                );
            }
        }
    }

    #[test]
    fn hours_are_written_with_two_decimals() {
        assert_eq!(hours_text(480), "8.00 h");
        assert_eq!(hours_text(90), "1.50 h");
        assert_eq!(hours_text(-60), "-1.00 h");
        assert_eq!(hours_text(0), "0.00 h");
    }

    #[test]
    fn a_timesheet_stands_where_its_record_says() {
        let mut sheet = Timesheet::default();
        assert_eq!(standing(&sheet, false), "draft");
        assert_eq!(standing(&sheet, true), "rejected");
        sheet.status = TimesheetStatus::Submitted;
        assert_eq!(standing(&sheet, false), "submitted");
        sheet.escalated_at = Some(renox::db::now());
        assert_eq!(standing(&sheet, false), "escalated");
        sheet.status = TimesheetStatus::Approved;
        sheet.approval_basis = Some(ApprovalBasis::Deemed);
        assert_eq!(standing(&sheet, false), "deemed_approved");
        sheet.status = TimesheetStatus::Locked;
        assert_eq!(standing(&sheet, false), "locked");
        sheet.billed_by = Some("invoicing.invoice:1".into());
        assert_eq!(standing(&sheet, false), "billed");
    }
}
