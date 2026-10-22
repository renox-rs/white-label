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
            .name("wl_attendance.timesheets")
            .get("/timesheets/{id}", show)
            .name("wl_attendance.timesheet"),
        permissions::VIEW,
    )
}

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

impl Filter {
    /// The list's address with this filter.
    fn href(&self) -> String {
        let mut query: Vec<String> = Vec::new();
        for (name, value) in [
            ("client", &self.client),
            ("month", &self.month),
            ("status", &self.status),
        ] {
            if !value.is_empty() {
                query.push(format!("{name}={value}"));
            }
        }
        if query.is_empty() {
            "/timesheets".to_owned()
        } else {
            format!("/timesheets?{}", query.join("&"))
        }
    }

    fn with_status(&self, status: &str) -> Filter {
        Filter {
            status: status.to_owned(),
            ..self.clone()
        }
    }
}

/// The standings, in the order of the tabs.
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

/// A timesheet in the list.
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
    status_kind: &'static str,
    adjustment: bool,
}

/// The list page.
#[derive(Serialize, ViewData)]
#[view(template = "wl-attendance/timesheets.html")]
pub(crate) struct ListPage {
    tx: Texts,
    rows: Vec<Row>,
    status_tabs: Vec<(String, String)>,
    status_current: String,
    status: String,
    clients: Vec<(String, String)>,
    client: String,
    month: String,
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

/// `GET /timesheets`: the timesheets in the person's reach, as a page or JSON.
async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> Result<Response> {
    let reach = wl_kernel::scopes::reach(&state.db, &user, permissions::VIEW).await?;
    let sheets = Timesheet::query()
        .order_by_desc(Timesheet::PERIOD_FROM)
        .order_by_desc(Timesheet::ID)
        .limit(500)
        .get(&state.db)
        .await?;
    let ids: Vec<i64> = sheets.iter().map(|sheet| sheet.id).collect();
    let entries = TimesheetEntry::query()
        .where_in("timesheet_id", ids)
        .get(&state.db)
        .await?;
    let mut rows = Vec::new();
    let mut clients: Vec<(String, String)> = Vec::new();
    let wanted_month = filter.month.trim();
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
        let own: Vec<&TimesheetEntry> = entries
            .iter()
            .filter(|entry| entry.timesheet_id == sheet.id)
            .collect();
        rows.push(Row {
            id: sheet.id,
            url: url(sheet.id),
            worker: name_of(&state, &lang, sheet.owner()).await,
            client: info
                .as_ref()
                .map(|info| info.client.clone())
                .filter(|client| !client.is_empty())
                .unwrap_or_else(|| "-".to_owned()),
            work: info.map_or_else(|| "-".to_owned(), |info| info.label),
            period: format!(
                "{} - {}",
                date_text(sheet.period_from),
                date_text(sheet.period_to)
            ),
            month: sheet.period_to.format("%b %Y").to_string(),
            hours: hours_text(minutes_of(&own)),
            status: text(&lang, &format!("status.{now}")),
            status_key: now,
            status_kind: kind_of(now),
            adjustment: sheet.is_adjustment(),
        });
    }
    if !wants_html(&headers) {
        let mut data = renox::serde_json::to_value(&rows)?;
        if let Some(rows) = data.as_array_mut() {
            for row in rows {
                if let Some(map) = row.as_object_mut() {
                    map.remove("status_kind");
                }
            }
        }
        return Ok(Json(data).into_response());
    }
    let mut status_tabs = vec![(filter.with_status("").href(), text(&lang, "all"))];
    for key in STANDINGS {
        status_tabs.push((
            filter.with_status(key).href(),
            text(&lang, &format!("status.{key}")),
        ));
    }
    let mut options = vec![(String::new(), text(&lang, "all_clients"))];
    options.extend(clients);
    Ok(View::typed(ListPage {
        tx: screen_texts(&lang),
        rows,
        status_current: filter.href(),
        status_tabs,
        status: filter.status.clone(),
        clients: options,
        client: filter.client.clone(),
        month: filter.month.clone(),
    })
    .into_response())
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

/// `Forbidden` unless the timesheet's context is in the person's reach.
async fn ensure_reach(state: &AppState, user: &AuthUser, sheet: &Timesheet) -> Result {
    let reach = wl_kernel::scopes::reach(&state.db, user, permissions::VIEW).await?;
    if reach.is_all() {
        return Ok(());
    }
    match sheet.context() {
        Some(context) if reach.contains(&state.db, &context).await? => Ok(()),
        _ => Err(Error::Forbidden),
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

/// `GET /timesheets/{id}`: one timesheet, as a page or JSON.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response> {
    let sheet = Timesheet::find_or_404(&state.db, id).await?;
    ensure_reach(&state, &user, &sheet).await?;
    let page = build(&state, &user, &lang, &sheet).await?;
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
) -> Result<SheetPage> {
    let db = &state.db;
    let system = text(lang, "history.system");
    let not_yet = text(lang, "fact.not_yet");
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
                    ("who", &user_name(db, sheet.submitted_by, &system).await),
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
                    ("who", &user_name(db, sheet.approved_by, &system).await),
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
            by: user_name(db, row.actor_id, &system).await,
            reason: row.reason.clone().unwrap_or_default(),
        });
    }

    // The signed sheets of an approval on behalf.
    let signed = ApprovalEvidence::of_timesheet(db, sheet.id)
        .await?
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
        invoice_url,
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

    #[test]
    fn the_filter_keeps_what_was_chosen_in_the_address() {
        let filter = Filter {
            client: "7".into(),
            month: "2026-11".into(),
            status: String::new(),
        };
        assert_eq!(
            filter.with_status("billed").href(),
            "/timesheets?client=7&month=2026-11&status=billed"
        );
        assert_eq!(Filter::default().href(), "/timesheets");
    }
}
