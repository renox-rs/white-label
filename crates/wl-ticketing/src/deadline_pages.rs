//! The back-office pages of deadline tasks (wl-ticketing §4.4, §6 `Tasks`;
//! BL-TKT-09, BL-TKT-10, 04 BL-ACC-13): the list of an agreement's or an
//! owner's tasks, a task's page and its actions.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /deadline-tasks` | `wl_ticketing.deadline_tasks` | `tasks.view` (`?subject_type=&subject_id=` one agreement's tasks, `?owner=` one user's, `?mine=1` the viewer's; without a filter the open tasks) |
//! | `GET /deadline-tasks/{id}` | `wl_ticketing.deadline_task` | `tasks.view` |
//! | `POST /deadline-tasks/{id}/actions/{name}` | `wl_ticketing.deadline_task_action` | `tasks.view` and the transition's own permission |
//!
//! The actions are `start`, `wait` (with a reason), `unblock`,
//! `submit_review`, `send_back` (with a comment), `complete` (with the
//! evidence; `complete_reviewed` for a task in review), `waive` (with a
//! reason; `tasks.manage`) and `close_missed` (with a reason from the
//! client's list when it has one, [`deadlines::missed_reasons`];
//! `tasks.manage`). Each runs the service of [`deadlines`] on the Asia/Dubai
//! date of the request; a refusal answers 403 or 422.
//!
//! A held task shows the badge "Held" and nothing else: no hold reason is
//! stored, so a restricted reason (a sanctions block) cannot show
//! (BL-TKT-10, 04 `ACC.ACL.14`). A task that went overdue shows "Late"
//! (completed: "Completed late").
//!
//! Decision: the pages check `tasks.view`, not the scope of the agreement
//! (the kernel's `DeadlineTasks` has no scope of its own yet); the verticals
//! link to the list of their agreement.

use std::collections::{BTreeMap, HashMap};

use renox::axum::extract::Query;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::deadlines::DeadlineTaskStatus;

use crate::deadline::DeadlineTask;
use crate::deadlines;
use crate::pages::invalid;
use crate::permissions;
use crate::texts;

/// The texts a template reads as `tx['deadline.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The open states, as the list shows them without a filter.
const OPEN: [DeadlineTaskStatus; 5] = [
    DeadlineTaskStatus::Upcoming,
    DeadlineTaskStatus::InProgress,
    DeadlineTaskStatus::Waiting,
    DeadlineTaskStatus::InReview,
    DeadlineTaskStatus::Overdue,
];

/// The routes, behind `tasks.view`; each action also needs its
/// transition's permission.
pub fn routes() -> Routes {
    Routes::new()
        .get("/deadline-tasks", index)
        .name("wl_ticketing.deadline_tasks")
        .get("/deadline-tasks/{id}", show)
        .name("wl_ticketing.deadline_task")
        .post("/deadline-tasks/{id}/actions/{name}", store_action)
        .name("wl_ticketing.deadline_task_action")
        .require_permission(permissions::TASKS_VIEW)
}

/// The package text `key` (without the `wl_ticketing.` prefix) in `lang`.
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_ticketing.{key}"))
}

/// The badge colour of a status.
fn status_kind(status: DeadlineTaskStatus) -> &'static str {
    match status {
        DeadlineTaskStatus::Upcoming
        | DeadlineTaskStatus::NotRequired
        | DeadlineTaskStatus::InProgress
        | DeadlineTaskStatus::InReview => "info",
        DeadlineTaskStatus::Waiting => "warning",
        DeadlineTaskStatus::Overdue | DeadlineTaskStatus::Missed => "error",
        DeadlineTaskStatus::Completed => "success",
    }
}

/// The filters of the list.
#[derive(Deserialize, Debug, Default)]
pub struct TaskQuery {
    /// Only the tasks of this agreement type…
    #[serde(default)]
    pub subject_type: Option<String>,
    /// …and this id.
    #[serde(default)]
    pub subject_id: Option<i64>,
    /// Only the tasks this user owns.
    #[serde(default)]
    pub owner: Option<i64>,
    /// `1`: only the viewer's tasks.
    #[serde(default)]
    pub mine: Option<String>,
}

/// One task as the list and the page show it.
#[derive(Serialize, Debug, Clone)]
pub struct TaskRow {
    /// The task's id.
    pub id: i64,
    /// "VAT Q3".
    pub title: String,
    /// The task type key (`vat_return`).
    pub task_type: String,
    /// The agreement in words, `Engagement #12`.
    pub source: String,
    /// The due date, `2026-11-28`.
    pub due: String,
    /// The start date, `2026-11-07`.
    pub start: String,
    /// The status label.
    pub status: String,
    /// The badge colour of the status.
    pub status_kind: &'static str,
    /// The held flag (no reason is shown, there is none).
    pub held: bool,
    /// Whether it went overdue: "Late" (or "Completed late").
    pub late: bool,
    /// The late badge's text.
    pub late_label: String,
    /// The owner's name, or the owning role.
    pub owner: String,
}

/// Each owner's name, read once.
async fn owner_name(
    state: &AppState,
    names: &mut HashMap<i64, String>,
    task: &DeadlineTask,
) -> Result<String> {
    let Some(id) = task.owner_user_id else {
        return Ok(task.owner_role.clone().unwrap_or_default());
    };
    if let Some(name) = names.get(&id) {
        return Ok(name.clone());
    }
    let name = User::find(&state.db, id)
        .await?
        .map_or_else(|| format!("#{id}"), |u| u.name);
    names.insert(id, name.clone());
    Ok(name)
}

/// `task` as a row.
async fn row(
    state: &AppState,
    lang: &Lang,
    names: &mut HashMap<i64, String>,
    task: &DeadlineTask,
) -> Result<TaskRow> {
    let late_key = if task.status == DeadlineTaskStatus::Completed {
        "deadline.completed_late"
    } else {
        "deadline.late"
    };
    Ok(TaskRow {
        id: task.id,
        title: task.title.clone(),
        task_type: task.task_type.clone(),
        source: wl_kernel::subject::words_of(&task.subject_type, task.subject_id),
        due: task.due_date.to_string(),
        start: task.start_date.to_string(),
        status: match (&deadlines::settings(state).waiting_label, task.status) {
            (Some(label), DeadlineTaskStatus::Waiting) => label.clone(),
            _ => text(lang, &format!("deadline.status.{}", task.status.as_str())),
        },
        status_kind: status_kind(task.status),
        held: task.held,
        late: task.late,
        late_label: text(lang, late_key),
        owner: owner_name(state, names, task).await?,
    })
}

/// The agreement types the deadline tasks have, once each.
pub async fn source_types(db: &Db) -> Result<Vec<String>> {
    let mut types: Vec<String> = DeadlineTask::query()
        .pluck::<String, _>(db, "subject_type")
        .await?;
    types.sort();
    types.dedup();
    Ok(types)
}

/// The deadline tasks grid: search on the title, agreement (in words) and
/// owner, filters by status, due date and owner, by due date. The waiting
/// label of the client's settings replaces "Waiting". `source_types` are the
/// agreement types the source column names in words ([`source_types`]).
pub fn grid(state: &AppState, lang: &Lang, can_export: bool, source_types: &[String]) -> Grid {
    let statuses: Vec<(String, String)> = DeadlineTaskStatus::ALL
        .iter()
        .map(|status| {
            let label = match (&deadlines::settings(state).waiting_label, status) {
                (Some(label), DeadlineTaskStatus::Waiting) => label.clone(),
                _ => text(lang, &format!("deadline.status.{}", status.as_str())),
            };
            (status.as_str().to_owned(), label)
        })
        .collect();
    let tones: Vec<(&str, &str)> = DeadlineTaskStatus::ALL
        .iter()
        .map(|status| {
            (
                status.as_str(),
                match status_kind(*status) {
                    "error" => "danger",
                    other => other,
                },
            )
        })
        .collect();
    let grid = Grid::new("wl-ticketing-deadline-tasks")
        .title(&text(lang, "deadline.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("title", &text(lang, "deadline.fields.title"))
                .frozen()
                .mobile()
                .searchable()
                .link("/deadline-tasks/{id}"),
        )
        // The type key in words (`vat_return` reads "Vat return"), never the key.
        .column(
            Column::text("task_type", &text(lang, "deadline.fields.type"))
                .computed(
                    "UPPER(SUBSTR({T}.\"task_type\", 1, 1)) || REPLACE(SUBSTR({T}.\"task_type\", 2), '_', ' ')",
                )
                .searchable()
                .hidden(),
        )
        .column(
            Column::text("source", &text(lang, "deadline.fields.source"))
                .computed(&wl_kernel::subject::words_sql("subject_type", "subject_id", source_types))
                .searchable()
                .mobile(),
        )
        .column(Column::date("due_date", &text(lang, "deadline.fields.due")).mobile())
        .column(Column::date("start_date", &text(lang, "deadline.fields.start")).hidden())
        .column(
            Column::select("status", &text(lang, "deadline.fields.status"), statuses)
                .badges(&tones)
                .mobile(),
        )
        .column(Column::bool("held", &text(lang, "task.held")))
        .column(Column::bool("late", &text(lang, "deadline.late")))
        .column(
            Column::text("owner", &text(lang, "deadline.fields.owner"))
                .computed("COALESCE((SELECT \"name\" FROM \"users\" WHERE \"id\" = {T}.\"owner_user_id\"), {T}.\"owner_role\", '')")
                .searchable()
                .mobile(),
        )
        .sort_by("due_date,id")
        .row_url("/deadline-tasks/{id}")
        .cards_on_mobile()
        .empty_state(
            &text(lang, "deadline.empty"),
            Some(&text(lang, "deadline.empty_hint")),
        );
    if can_export { grid.exports() } else { grid }
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-ticketing/deadline_tasks.html")]
pub(crate) struct TasksPage {
    tx: Texts,
    /// The page's subtitle: what the list is filtered on.
    subtitle: String,
    tasks: GridPage<DeadlineTask>,
}

/// `GET /deadline-tasks`: the tasks of an agreement or an owner by due
/// date, or the open ones (or the export, with `tasks.manage`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<TaskQuery>,
    request: GridRequest,
) -> Result<Response> {
    let mut found = DeadlineTask::query();
    let mut filtered = false;
    let mut subtitle = text(&lang, "deadline.subtitle_open");
    if let (Some(kind), Some(id)) = (query.subject_type.as_deref(), query.subject_id)
        && !kind.trim().is_empty()
    {
        found = found
            .where_(DeadlineTask::SUBJECT_TYPE.eq(kind.trim()))
            .where_(DeadlineTask::SUBJECT_ID.eq(id));
        filtered = true;
        subtitle = wl_kernel::subject::words_of(kind.trim(), id);
    }
    let owner = if query.mine.as_deref() == Some("1") {
        Some(user.id)
    } else {
        query.owner
    };
    if let Some(owner) = owner {
        found = found.where_(DeadlineTask::OWNER_USER_ID.eq(owner));
        filtered = true;
        if owner == user.id {
            subtitle = text(&lang, "deadline.subtitle_mine");
        }
    }
    if !filtered {
        found = found.where_(DeadlineTask::STATUS.is_in(OPEN));
    }
    let can_export = user.has_permission(permissions::TASKS_MANAGE);
    let types = source_types(&state.db).await?;
    let grid = grid(&state, &lang, can_export, &types);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(found.clone(), &request).await? {
        return Ok(file);
    }
    Ok(View::typed(TasksPage {
        tx: texts::all(&lang),
        subtitle,
        tasks: grid.page(found, &request).await?,
    })
    .into_response())
}

/// An action without input of its own, as a button.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ActionRow {
    name: &'static str,
    label: String,
}

/// What the actions of `task` offered to `user` are, as the services will
/// accept them: the status, then who may run it. The service still checks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Offered {
    start: bool,
    wait: bool,
    unblock: bool,
    submit_review: bool,
    send_back: bool,
    complete: bool,
    waive: bool,
    close_missed: bool,
}

fn offered(task: &DeadlineTask, user: &AuthUser, waive_started: bool) -> Offered {
    use DeadlineTaskStatus::{InProgress, InReview, Overdue, Upcoming, Waiting};
    let works = user.has_permission(permissions::TASKS_WORK);
    let reviews = user.has_permission(permissions::TASKS_REVIEW);
    let manages = user.has_permission(permissions::TASKS_MANAGE);
    let status = task.status;
    let reviewed = task.reviewer_role.is_some();
    Offered {
        start: works && matches!(status, Upcoming | Overdue),
        wait: works && status == InProgress,
        unblock: works && status == Waiting,
        submit_review: works && reviewed && status == InProgress,
        send_back: reviews && status == InReview,
        complete: (works && !reviewed && status == InProgress) || (reviews && status == InReview),
        waive: manages
            && (status == Upcoming || (waive_started && matches!(status, InProgress | Waiting))),
        close_missed: manages && status == Overdue,
    }
}

/// What a task's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-ticketing/deadline_task.html")]
pub(crate) struct TaskPage {
    tx: Texts,
    task: TaskRow,
    description: String,
    evidence: String,
    waiting_reason: String,
    closed_reason: String,
    overdue_on: String,
    reviewer_role: String,
    /// The buttons: start, unblock, submit for review.
    actions: Vec<ActionRow>,
    can_wait: bool,
    can_send_back: bool,
    can_complete: bool,
    can_waive: bool,
    can_close_missed: bool,
    /// The client's reasons for `close_missed`, `(code, name)`; empty: a
    /// free text.
    missed_reasons: Vec<(String, String)>,
    /// Whether one of the reasons needs a text.
    missed_needs_text: bool,
}

/// `GET /deadline-tasks/{id}`: the task, its flags and its actions.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let task = DeadlineTask::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let mut names = HashMap::new();
    let offered = offered(&task, &user, deadlines::settings(&state).waive_started);
    let actions = [
        ("start", offered.start),
        ("unblock", offered.unblock),
        ("submit_review", offered.submit_review),
    ]
    .into_iter()
    .filter(|(_, shown)| *shown)
    .map(|(name, _)| ActionRow {
        name,
        label: text(&lang, &format!("deadline.action.{name}")),
    })
    .collect();
    let reasons = deadlines::missed_reasons(&state);
    Ok(View::typed(TaskPage {
        tx: texts::all(&lang),
        task: row(&state, &lang, &mut names, &task).await?,
        description: task.description.clone(),
        evidence: task.evidence.clone().unwrap_or_default(),
        waiting_reason: task.waiting_reason.clone().unwrap_or_default(),
        closed_reason: task.closed_reason.clone().unwrap_or_default(),
        overdue_on: task.overdue_on.map(|d| d.to_string()).unwrap_or_default(),
        reviewer_role: task.reviewer_role.clone().unwrap_or_default(),
        actions,
        can_wait: offered.wait,
        can_send_back: offered.send_back,
        can_complete: offered.complete,
        can_waive: offered.waive,
        can_close_missed: offered.close_missed,
        missed_needs_text: reasons.iter().any(|r| r.needs_text),
        missed_reasons: reasons.into_iter().map(|r| (r.code, r.name)).collect(),
    }))
}

/// The form of every action; each reads what it needs.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ActionForm {
    /// The reason (`wait`, `waive`, `close_missed` without a list) or the
    /// comment (`send_back`), or the text of a reason that needs one.
    #[validate(max = 1000)]
    pub reason: Option<String>,
    /// The reason's code from the client's list (`close_missed`).
    #[validate(max = 64)]
    pub reason_code: Option<String>,
    /// The evidence (`complete`): a document reference or a note.
    #[validate(max = 1000)]
    pub evidence: Option<String>,
}

/// `POST /deadline-tasks/{id}/actions/{name}`: runs the action as the
/// viewer on today's date (Asia/Dubai).
async fn store_action(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, name)): Path<(i64, String)>,
    Valid(form): Valid<ActionForm>,
) -> Result<Response> {
    let task = DeadlineTask::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let today = wl_kernel::documents::dubai_date(renox::db::now());
    let reason = form.reason.as_deref();
    let (done, field) = match name.as_str() {
        "start" => (deadlines::start(&state, &user, &task).await, "reason"),
        "wait" => (
            deadlines::wait(&state, &user, &task, reason).await,
            "reason",
        ),
        "unblock" => (deadlines::unblock(&state, &user, &task).await, "reason"),
        "submit_review" => (
            deadlines::submit_review(&state, &user, &task).await,
            "reason",
        ),
        "send_back" => (
            deadlines::send_back(&state, &user, &task, reason).await,
            "reason",
        ),
        "complete" => (
            deadlines::complete(&state, &user, &task, form.evidence.as_deref(), today).await,
            "evidence",
        ),
        "waive" => (
            deadlines::waive(&state, &user, &task, reason, today).await,
            "reason",
        ),
        "close_missed" => {
            let field = if deadlines::missed_reasons(&state).is_empty() {
                "reason"
            } else {
                "reason_code"
            };
            (
                deadlines::close_missed(&state, &user, &task, form.reason_code.as_deref(), reason)
                    .await,
                field,
            )
        }
        _ => return Err(Error::NotFound),
    };
    match done {
        Ok(()) => Ok((
            Toast::success(text(&lang, "deadline.done")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::ReasonRequired) => Err(invalid(
            field,
            text(&lang, "deadline.reason_required"),
            &form,
        )),
        Err(TransitionError::Guard(message)) => Err(invalid(field, message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_has_a_badge_kind_of_the_kit() {
        for status in DeadlineTaskStatus::ALL.iter().copied() {
            assert!(["info", "success", "warning", "error"].contains(&status_kind(status)));
        }
    }
}
