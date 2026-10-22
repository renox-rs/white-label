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

fn status_kind(status: LeaveStatus) -> &'static str {
    match status {
        LeaveStatus::Requested => "warning",
        LeaveStatus::Approved => "success",
        LeaveStatus::Completed => "info",
        LeaveStatus::Rejected => "error",
        LeaveStatus::Cancelled => "info",
    }
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

/// One of the user's own requests.
#[derive(Serialize, Debug, Clone)]
pub struct MyRow {
    id: i64,
    kind: String,
    period: String,
    days: i64,
    status: String,
    status_kind: &'static str,
    reason: String,
    note: String,
    can_cancel: bool,
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
    rows: Vec<MyRow>,
    taken: Vec<TakenRow>,
    types: Vec<(String, String)>,
    today: String,
}

fn sick_note_text(lang: &Lang, request: &LeaveRequest) -> String {
    match (request.sick_note_attached_on, request.sick_note_due_on) {
        (Some(_), _) => text(lang, "sick.note_attached"),
        (None, Some(due)) => texts::t_with(lang, "wl_hr.sick.note_due", &[("date", &due)]),
        (None, None) => String::new(),
    }
}

/// `GET /hr/leave`: the user's own leave.
async fn my_leave(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    if !user.has_permission(permissions::LEAVE_REQUEST) {
        return Err(Error::Forbidden);
    }
    let day = today();
    let employee = employee_of(&state.db, user.id).await?;
    let mut rows = Vec::new();
    let mut days_by_type: BTreeMap<String, i64> = BTreeMap::new();
    if let Some(employee) = &employee {
        let cal = calendar(&state);
        for request in LeaveRequest::query()
            .where_(LeaveRequest::EMPLOYEE_ID.eq(employee.id))
            .order_by_desc(LeaveRequest::FROM_DATE)
            .limit(100)
            .get(&state.db)
            .await?
        {
            if request.status.is_approved() && request.from_date.year() == day.year() {
                *days_by_type.entry(request.leave_type.clone()).or_default() +=
                    working_days(&cal, request.from_date, request.to_date);
            }
            rows.push(MyRow {
                id: request.id,
                kind: text(&lang, &format!("leave_type.{}", request.leave_type)),
                period: period(request.from_date, request.to_date),
                days: working_days(&cal, request.from_date, request.to_date),
                status: text(&lang, &format!("leave_status.{}", request.status.as_str())),
                status_kind: status_kind(request.status),
                reason: request.decision_reason.clone().unwrap_or_default(),
                note: if request.leave_type == sick_leave::SICK {
                    sick_note_text(&lang, &request)
                } else {
                    String::new()
                },
                can_cancel: matches!(
                    request.status,
                    LeaveStatus::Requested | LeaveStatus::Approved
                ) && request.from_date > day,
            });
        }
    }
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
        taken,
        types: type_options(&lang),
        today: day.to_string(),
    }))
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
    if !user.has_permission(permissions::LEAVE_REQUEST) {
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

/// One request in the queue.
#[derive(Serialize, Debug, Clone)]
pub struct QueueRow {
    id: i64,
    who: String,
    kind: String,
    period: String,
    days: i64,
    unpaid: i64,
    status: String,
    status_kind: &'static str,
    reason: String,
    note: String,
    needs_note: bool,
    can_decide: bool,
    can_cancel: bool,
}

/// What the queue needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/leave_queue.html")]
pub(crate) struct QueuePage {
    tx: Texts,
    waiting: Vec<QueueRow>,
    decided: Vec<QueueRow>,
}

/// `GET /hr/leave/queue`: the requests to decide in the user's scopes, then
/// the recent decisions.
async fn queue(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    if !user.has_permission(permissions::LEAVE_APPROVE)
        && !user.has_permission_anywhere(permissions::LEAVE_APPROVE)
    {
        return Err(Error::Forbidden);
    }
    let day = today();
    let cal = calendar(&state);
    let mut waiting = Vec::new();
    let mut decided = Vec::new();
    for request in LeaveRequest::query()
        .order_by(LeaveRequest::FROM_DATE)
        .limit(300)
        .get(&state.db)
        .await?
    {
        let recent = request.status != LeaveStatus::Requested
            && request.to_date >= day - renox::chrono::Duration::days(30);
        if request.status != LeaveStatus::Requested && !recent {
            continue;
        }
        let employee = Employee::find_or_404(&state.db, request.employee_id).await?;
        if !leave_service::holds_approve(&state, user.id, &employee).await? {
            continue;
        }
        let needs_note = request.leave_type == sick_leave::SICK
            && request.sick_note_document_id.is_none()
            && !matches!(
                request.status,
                LeaveStatus::Rejected | LeaveStatus::Cancelled
            );
        let row = QueueRow {
            id: request.id,
            who: employee_name(&state, &lang, &employee).await,
            kind: text(&lang, &format!("leave_type.{}", request.leave_type)),
            period: period(request.from_date, request.to_date),
            days: working_days(&cal, request.from_date, request.to_date),
            unpaid: request.unpaid_days,
            status: text(&lang, &format!("leave_status.{}", request.status.as_str())),
            status_kind: status_kind(request.status),
            reason: request.decision_reason.clone().unwrap_or_default(),
            note: if request.leave_type == sick_leave::SICK {
                sick_note_text(&lang, &request)
            } else {
                String::new()
            },
            needs_note,
            can_decide: request.status == LeaveStatus::Requested,
            can_cancel: request.status == LeaveStatus::Approved && request.from_date > day,
        };
        if request.status == LeaveStatus::Requested {
            waiting.push(row);
        } else {
            decided.push(row);
        }
    }
    decided.reverse();
    Ok(View::typed(QueuePage {
        tx: texts::all(&lang),
        waiting,
        decided,
    }))
}
