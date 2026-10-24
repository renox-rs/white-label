//! The leave screens: my leave and the queue of leave to decide (wl-hr §4
//! *Leave request*, §5.2).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /hr/leave` | `wl_hr.my_leave` | `leave.request` |
//! | `POST /hr/leave` | `wl_hr.leave_request` | `leave.request`, for the user's own employee record |
//! | `POST /hr/leave/{id}/cancel` | `wl_hr.leave_cancel` | the employee, or `leave.approve` in their scopes |
//! | `GET /hr/leave/queue` | `wl_hr.leave_queue` | `leave.approve` |
//! | `POST /hr/leave/{id}/approve` | `wl_hr.leave_approve` | `leave.approve` in the employee's scopes |
//! | `POST /hr/leave/{id}/reject` | `wl_hr.leave_reject` | `leave.approve` in the employee's scopes |
//!
//! *My leave* is the signed-in user's own requests: ask for leave (a type and
//! the first and last day), cancel one before it starts, and see what was
//! decided and why, with the working days taken this year by type. The queue
//! lists the requests waiting for the user's decision, in the scopes where the
//! user holds `leave.approve`, soonest first, each with *Approve* and *Reject*
//! (a reason is required); a sick leave without its sick note says so and
//! offers *Attach the sick note* (the route of the sick-note flow), since the
//! machine refuses to approve it before. Below the queue, the leave already
//! decided that has not started can still be cancelled by the approver.
//!
//! The services are [`crate::leave_service`] and [`crate::sick_leave`]; the
//! routes carry no permission guard of their own (the guard counts global
//! roles only, and a manager holds `leave.approve` in a branch), and the
//! handlers and services check who acts and answer 403.
//!
//! Decision: there is no leave balance to show, because the base does not
//! keep an entitlement (`leave_service` module docs: annual leave is never
//! short); *My leave* shows the working days taken this year instead.

use std::collections::BTreeMap;

use renox::chrono::{Datelike, NaiveDate};
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::calendar::calendar;
use wl_kernel::documents::today;

use crate::employee::Employee;
use crate::leave::{LeaveRequest, LeaveStatus};
use crate::leave_service::{self, LEAVE_TYPES, working_days};
use crate::pages::employees::employee_name;
use crate::{permissions, sick_leave, texts};

/// The texts a template reads as `tx['my_leave.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the leave screens.
pub fn routes() -> Routes {
    Routes::new()
        .get("/hr/leave", my_leave)
        .name("wl_hr.my_leave")
        .post("/hr/leave", request)
        .name("wl_hr.leave_request")
        .get("/hr/leave/queue", queue)
        .name("wl_hr.leave_queue")
        .post("/hr/leave/{id}/cancel", cancel)
        .name("wl_hr.leave_cancel")
        .post("/hr/leave/{id}/approve", approve)
        .name("wl_hr.leave_approve")
        .post("/hr/leave/{id}/reject", reject)
        .name("wl_hr.leave_reject")
        .require_auth()
}

fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_hr.{key}"))
}

/// The leave types as a select's options.
fn type_options(lang: &Lang) -> Vec<(String, String)> {
    LEAVE_TYPES
        .iter()
        .map(|kind| {
            (
                (*kind).to_owned(),
                text(lang, &format!("leave_type.{kind}")),
            )
        })
        .collect()
}

/// The employee record of `user`.
async fn employee_of(db: &Db, user: i64) -> Result<Option<Employee>> {
    Employee::query()
        .where_(Employee::USER_ID.eq(Some(user)))
        .first(db)
        .await
}

/// `first - last`, or the day when they are one.
fn period(from: NaiveDate, to: NaiveDate) -> String {
    if from == to {
        from.to_string()
    } else {
        format!("{from} - {to}")
    }
}

/// Working days taken this year, by type.
#[derive(Serialize, Debug, Clone)]
pub struct TakenRow {
    kind: String,
    days: i64,
}

/// What *My leave* needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/my_leave.html")]
pub(crate) struct MyLeavePage {
    tx: Texts,
    year: i32,
    has_employee: bool,
    rows: GridPage<LeaveRequest>,
    /// The requests of the page that can still be cancelled: one sheet each.
    cancel_sheets: Vec<LeaveSheet>,
    taken: Vec<TakenRow>,
    types: Vec<(String, String)>,
    today: String,
}

/// The id of a row and the words of its sheets.
#[derive(Serialize, Debug, Clone)]
pub struct LeaveSheet {
    id: i64,
    who: String,
    kind: String,
    period: String,
}

fn sick_note_text(lang: &Lang, request: &LeaveRequest) -> String {
    match (request.sick_note_attached_on, request.sick_note_due_on) {
        (Some(_), _) => text(lang, "sick.note_attached"),
        (None, Some(due)) => texts::t_with(lang, "wl_hr.sick.note_due", &[("date", &due)]),
        (None, None) => String::new(),
    }
}

/// The columns both leave grids share: type, dates, working days, status,
/// the reason given and the sick note. `queue` adds the employee.
fn leave_grid(id: &str, title: &str, lang: &Lang, queue: bool) -> Grid {
    let t = |key: &str| text(lang, key);
    let statuses: Vec<(String, String)> = [
        LeaveStatus::Requested,
        LeaveStatus::Approved,
        LeaveStatus::Completed,
        LeaveStatus::Rejected,
        LeaveStatus::Cancelled,
    ]
    .iter()
    .map(|status| {
        (
            status.as_str().to_owned(),
            t(&format!("leave_status.{}", status.as_str())),
        )
    })
    .collect();
    let mut grid = Grid::new(id)
        .title(title)
        .per_page(25)
        .remember()
        .advanced_filter();
    if queue {
        grid = grid
            .column(
                Column::related(
                    "employee_number",
                    &t("employees.number"),
                    "wl_hr_employees",
                    "employee_id",
                    "employee_number",
                )
                .frozen()
                .searchable(),
            )
            .column(Column::custom("who", &t("queue.who")).mobile());
    }
    let kind = Column::select("leave_type", &t("my_leave.type"), type_options(lang)).mobile();
    grid.column(if queue { kind } else { kind.frozen() })
        .column(Column::custom("period", &t("my_leave.period")).mobile())
        .column(Column::date("from_date", &t("my_leave.from")).hidden())
        .column(Column::date("to_date", &t("my_leave.to")).hidden())
        .column(Column::custom("days", &t("my_leave.days")))
        .column(Column::number("unpaid_days", &t("leave.unpaid_days")).hidden())
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("requested", "warning"),
                    ("approved", "success"),
                    ("completed", "info"),
                    ("rejected", "danger"),
                    ("cancelled", "info"),
                ])
                .mobile(),
        )
        .column(Column::text("decision_reason", &t("my_leave.reason")).searchable())
        .column(Column::custom("note", &t("sick.note")))
        .column(Column::custom("actions", &t("leave.actions")))
        .sort_by(if queue { "from_date" } else { "-from_date" })
        .cards_on_mobile()
}

/// The grid of *My leave*: no export, the person's own requests.
pub(crate) fn my_grid(lang: &Lang) -> Grid {
    let t = |key: &str| text(lang, key);
    leave_grid("hr-my-leave", &t("my_leave.title"), lang, false)
        .empty_state(&t("my_leave.empty"), Some(&t("my_leave.empty_hint")))
}

/// `GET /hr/leave`: the user's own leave.
async fn my_leave(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<View> {
    if !user.has_permission_anywhere(permissions::LEAVE_REQUEST) {
        return Err(Error::Forbidden);
    }
    let day = today();
    let employee = employee_of(&state.db, user.id).await?;
    let cal = calendar(&state);
    let mut days_by_type: BTreeMap<String, i64> = BTreeMap::new();
    if let Some(employee) = &employee {
        for request in LeaveRequest::query()
            .where_(LeaveRequest::EMPLOYEE_ID.eq(employee.id))
            .get(&state.db)
            .await?
        {
            if request.status.is_approved() && request.from_date.year() == day.year() {
                *days_by_type.entry(request.leave_type.clone()).or_default() +=
                    working_days(&cal, request.from_date, request.to_date);
            }
        }
    }
    let query = LeaveRequest::query()
        .where_(LeaveRequest::EMPLOYEE_ID.eq(employee.as_ref().map_or(-1, |e| e.id)));
    let page = my_grid(&lang).page(query, &request).await?;
    let cancellable = |r: &LeaveRequest| {
        matches!(r.status, LeaveStatus::Requested | LeaveStatus::Approved) && r.from_date > day
    };
    let cancel_sheets = page
        .items()
        .iter()
        .filter(|r| cancellable(r))
        .map(|r| sheet_of(&lang, r, String::new()))
        .collect();
    let rows = page.extend(|r| {
        renox::serde_json::json!({
            "period": period(r.from_date, r.to_date),
            "days": working_days(&cal, r.from_date, r.to_date),
            "note": if r.leave_type == sick_leave::SICK { sick_note_text(&lang, r) } else { String::new() },
            "can_cancel": cancellable(r),
        })
    });
    let taken = LEAVE_TYPES
        .iter()
        .filter_map(|kind| {
            days_by_type.get(*kind).map(|days| TakenRow {
                kind: text(&lang, &format!("leave_type.{kind}")),
                days: *days,
            })
        })
        .collect();
    Ok(View::typed(MyLeavePage {
        tx: texts::all(&lang),
        year: day.year(),
        has_employee: employee.is_some(),
        rows,
        cancel_sheets,
        taken,
        types: type_options(&lang),
        today: day.to_string(),
    }))
}

/// The words of the sheets of request `r`.
fn sheet_of(lang: &Lang, r: &LeaveRequest, who: String) -> LeaveSheet {
    LeaveSheet {
        id: r.id,
        who,
        kind: text(lang, &format!("leave_type.{}", r.leave_type)),
        period: period(r.from_date, r.to_date),
    }
}

/// What the employee enters to ask for leave.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct RequestForm {
    /// One of the leave types.
    #[validate(max = 32)]
    pub leave_type: Option<String>,
    /// The first day, like `2026-11-03`.
    #[validate(max = 10)]
    pub from_date: Option<String>,
    /// The last day, like `2026-11-05`.
    #[validate(max = 10)]
    pub to_date: Option<String>,
}

fn invalid(lang: &Lang, key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, text(lang, key))
}

fn date_of(field: &Option<String>) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(field.as_deref().unwrap_or_default().trim(), "%Y-%m-%d").ok()
}

/// `POST /hr/leave`: the user asks for leave for their own employee record.
async fn request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<RequestForm>,
) -> Result<Response> {
    if !user.has_permission_anywhere(permissions::LEAVE_REQUEST) {
        return Err(Error::Forbidden);
    }
    let Some(employee) = employee_of(&state.db, user.id).await? else {
        return Err(invalid(&lang, "my_leave.no_employee"));
    };
    let from = date_of(&form.from_date).ok_or_else(|| invalid(&lang, "my_leave.problem.from"))?;
    let to = date_of(&form.to_date).ok_or_else(|| invalid(&lang, "my_leave.problem.to"))?;
    leave_service::request(
        &state,
        user.id,
        employee.id,
        form.leave_type.as_deref().unwrap_or_default(),
        from,
        to,
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "leave.requested")),
        renox::HxRefresh,
    )
        .into_response())
}

/// What a user enters to cancel or reject.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ReasonForm {
    /// Why.
    #[validate(max = 500)]
    pub reason: Option<String>,
}

/// `POST /hr/leave/{id}/cancel`: before it starts.
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReasonForm>,
) -> Result<Response> {
    leave_service::cancel(
        &state,
        user.id,
        id,
        form.reason.as_deref().unwrap_or_default(),
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "leave.cancelled")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /hr/leave/{id}/approve`.
async fn approve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    leave_service::approve(&state, user.id, id).await?;
    Ok((
        Toast::success(text(&lang, "leave.approved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /hr/leave/{id}/reject`: the reason is required.
async fn reject(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReasonForm>,
) -> Result<Response> {
    let reason = form.reason.as_deref().unwrap_or_default().trim().to_owned();
    if reason.is_empty() {
        return Err(invalid(&lang, "queue.problem.reason"));
    }
    leave_service::reject(&state, user.id, id, &reason).await?;
    Ok((
        Toast::success(text(&lang, "leave.rejected")),
        renox::HxRefresh,
    )
        .into_response())
}

/// What the queue needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/leave_queue.html")]
pub(crate) struct QueuePage {
    tx: Texts,
    requests: GridPage<LeaveRequest>,
    /// The requests of the page that wait for a sick note, for a decision, or
    /// can be cancelled: one sheet each.
    note_sheets: Vec<LeaveSheet>,
    decide_sheets: Vec<LeaveSheet>,
    cancel_sheets: Vec<LeaveSheet>,
    waiting: usize,
}

/// The grid of the queue: the requests to decide and those decided lately,
/// exportable by who may approve.
pub(crate) fn queue_grid(lang: &Lang) -> Grid {
    let t = |key: &str| text(lang, key);
    leave_grid("hr-leave-queue", &t("queue.title"), lang, true)
        .exports()
        .empty_state(&t("queue.empty"), Some(&t("queue.empty_text")))
}

/// Whether `request` still needs its sick note.
fn needs_note(request: &LeaveRequest) -> bool {
    request.leave_type == sick_leave::SICK
        && request.sick_note_document_id.is_none()
        && !matches!(
            request.status,
            LeaveStatus::Rejected | LeaveStatus::Cancelled
        )
}

/// `GET /hr/leave/queue`: the requests to decide in the user's scopes, then
/// the recent decisions; `?export=` too.
async fn queue(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    if !user.has_permission(permissions::LEAVE_APPROVE)
        && !user.has_permission_anywhere(permissions::LEAVE_APPROVE)
    {
        return Err(Error::Forbidden);
    }
    let day = today();
    let cal = calendar(&state);
    // The requests of the people in the user's scopes: waiting, or decided in
    // the last 30 days (or not over yet).
    let mut visible = Vec::new();
    let mut waiting = 0;
    let mut approvers: BTreeMap<i64, bool> = BTreeMap::new();
    for request in LeaveRequest::query().get(&state.db).await? {
        let recent = request.status != LeaveStatus::Requested
            && request.to_date >= day - renox::chrono::Duration::days(30);
        if request.status != LeaveStatus::Requested && !recent {
            continue;
        }
        let allowed = match approvers.get(&request.employee_id) {
            Some(allowed) => *allowed,
            None => {
                let employee = Employee::find_or_404(&state.db, request.employee_id).await?;
                let allowed = leave_service::holds_approve(&state, user.id, &employee).await?;
                approvers.insert(request.employee_id, allowed);
                allowed
            }
        };
        if allowed {
            waiting += usize::from(request.status == LeaveStatus::Requested);
            visible.push(request.id);
        }
    }
    let query = LeaveRequest::query().where_(LeaveRequest::ID.is_in(if visible.is_empty() {
        vec![-1]
    } else {
        visible
    }));
    let grid = queue_grid(&lang);
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut names: BTreeMap<i64, String> = BTreeMap::new();
    for item in page.items() {
        let employee = Employee::find_or_404(&state.db, item.employee_id).await?;
        names.insert(item.id, employee_name(&state, &lang, &employee).await);
    }
    let sheets = |keep: &dyn Fn(&LeaveRequest) -> bool| -> Vec<LeaveSheet> {
        page.items()
            .iter()
            .filter(|r| keep(r))
            .map(|r| sheet_of(&lang, r, names.get(&r.id).cloned().unwrap_or_default()))
            .collect()
    };
    let note_sheets = sheets(&needs_note);
    let decide_sheets = sheets(&|r| r.status == LeaveStatus::Requested);
    let cancel_sheets = sheets(&|r| r.status == LeaveStatus::Approved && r.from_date > day);
    let requests = page.extend(|r| {
        renox::serde_json::json!({
            "who": names.get(&r.id).cloned().unwrap_or_default(),
            "period": period(r.from_date, r.to_date),
            "days": working_days(&cal, r.from_date, r.to_date),
            "note": if r.leave_type == sick_leave::SICK { sick_note_text(&lang, r) } else { String::new() },
            "needs_note": needs_note(r),
            "can_decide": r.status == LeaveStatus::Requested,
            "can_cancel": r.status == LeaveStatus::Approved && r.from_date > day,
        })
    });
    Ok(View::typed(QueuePage {
        tx: texts::all(&lang),
        requests,
        note_sheets,
        decide_sheets,
        cancel_sheets,
        waiting,
    })
    .into_response())
}
