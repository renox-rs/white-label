//! What happens after a return transition commits (04 §7.1 *Return*;
//! BL-ACC-16, BL-ACC-17): the notices, the preparer's task and the linked
//! deadline task. They run in the listener of [`crate::events`], which
//! re-reads the row, never inside an effect.
//!
//! | Transition | What happens |
//! |---|---|
//! | `submit` | the reviewer is told; the preparer's "client withdrew approval" task closes |
//! | `send_back` | the preparer is told, with the comment |
//! | `review` | the client approvers get the summary and the link to approve it |
//! | `withdraw_approval` | the preparer and the reviewer are told; the preparer gets a task (due 1 working day) to rework the return and bring the linked task back to work |
//! | `correct` | the voluntary disclosure task, or the adjustment attached to the named return ([`crate::corrections`]) |
//! | `file`, `file_approved` | the linked task is completed with the acknowledgement (*late* when it was overdue); the client approvers get the payment reminder when tax is payable; an expired client KYC document flags the return for the MLRO; the corrections made through this return are cancelled as *not required* |
//! | `cancel` | the linked task is waived (*not required*, `upcoming`), listed for the manager (*not required*, otherwise) or handed over to the client (*client files itself*, *engagement ended*); the client is told who files it, unless the reason is restricted |
//!
//! The linked task is never moved back to work by the system after a
//! withdrawal (wl-ticketing has no system transition into `in_progress`);
//! when `ACC.RUL.04` makes it `overdue` meanwhile, the preparer's task gets
//! a comment saying to `start` it ([`on_task_overdue`]).

use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::Transitioned;
use wl_kernel::calendar::calendar;
use wl_kernel::deadlines::{
    DeadlineTaskStatus, DeadlineTaskSummary, HandOverTask, TaskOverdue, WaiveTask, deadline_tasks,
};
use wl_kernel::documents::today;
use wl_kernel::notify::{
    self, GROUP_PORTAL, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest, notifier,
};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, TaskComment, task_creator};

use crate::corrections;
use crate::deadline_notices::client_recipients;
use crate::documents;
use crate::engagement::Engagement;
use crate::permissions::{ACCOUNTANT_ROLE, MANAGER_ROLE};
use crate::reasons;
use crate::return_machine::{
    CANCEL, CORRECT, FILE, FILE_APPROVED, REVIEW, SEND_BACK, SUBMIT, WITHDRAW_APPROVAL,
};
use crate::tax_return::{Return, ReturnStatus};
use crate::texts::english;

/// The module that sends the notices.
const MODULE: &str = "accounting";

/// The reviewer is told a return was submitted.
pub const SUBMITTED_NOTICE: &str = "accounting.return_submitted";
/// The preparer is told a return was sent back.
pub const SENT_BACK_NOTICE: &str = "accounting.return_sent_back";
/// The client approvers get the return to approve.
pub const FOR_APPROVAL_NOTICE: &str = "accounting.return_for_approval";
/// The preparer and the reviewer are told the client withdrew its approval.
pub const WITHDRAWN_NOTICE: &str = "accounting.return_approval_withdrawn";
/// The client is told it files the return itself.
pub const HANDED_OVER_NOTICE: &str = "accounting.return_handed_over";
/// The client approvers are reminded to pay the tax of a filed return.
pub const PAYMENT_REMINDER_NOTICE: &str = "accounting.return_payment_reminder";

/// The key the preparer's task for a withdrawn approval is found by.
pub fn withdrawn_key(return_id: i64) -> String {
    format!("accounting.return_withdrawn:{return_id}")
}

/// The key of the manager's task listing the started linked task of a
/// cancelled return.
pub fn left_key(return_id: i64) -> String {
    format!("accounting.return_task_left:{return_id}")
}

/// The notification types of the return.
pub fn notification_types() -> Vec<NotificationTypeDef> {
    let staff = |key: &str, texts: &str| {
        NotificationTypeDef::new(key, MODULE)
            .group(GROUP_PORTAL)
            .template(
                NotifyChannel::Email,
                "en",
                Some(&english(
                    &format!("accounting.returns.{texts}_subject"),
                    &[],
                )),
                &english(&format!("accounting.returns.{texts}_body"), &[]),
            )
    };
    vec![
        staff(SUBMITTED_NOTICE, "submitted"),
        staff(SENT_BACK_NOTICE, "sent_back"),
        // 04 §9 *Notices*: a return approval request that gives up goes to
        // another client approver, then to a staff task for the manager.
        staff(FOR_APPROVAL_NOTICE, "for_approval")
            .give_up(&[
                GiveUpReaction::MarkContactInvalid,
                GiveUpReaction::AlternativeRecipient,
                GiveUpReaction::StaffTask,
            ])
            .staff_role(MANAGER_ROLE),
        staff(WITHDRAWN_NOTICE, "withdrawn"),
        staff(HANDED_OVER_NOTICE, "handed_over"),
        staff(PAYMENT_REMINDER_NOTICE, "payment_reminder"),
    ]
}

/// Registers the notification types.
pub(crate) fn register_types() {
    for def in notification_types() {
        notify::register_type(def);
    }
}

/// The data every notice of `ret` fills its template with.
fn notice_data(ret: &Return) -> Value {
    json!({
        "number": ret.number(),
        "return": english(
            &format!("accounting.returns.type.{}", ret.return_type.as_str()),
            &[],
        ),
        "period": ret.period_end.to_string(),
        "due": ret.due_date.to_string(),
        "comment": ret.send_back_comment.clone().unwrap_or_default(),
        "link": format!("/accounting/returns/{}", ret.id),
        "portal_link": format!("/accounting/portal/returns/{}", ret.id),
    })
}

/// The staff to tell about `ret`: its reviewer, else the manager of the
/// engagement, else the holder of the manager role.
async fn reviewers(state: &AppState, ret: &Return) -> Result<Vec<i64>> {
    if let Some(reviewer) = ret.reviewer_user_id {
        return Ok(vec![reviewer]);
    }
    let engagement = Engagement::find_or_404(&state.db, ret.engagement_id).await?;
    if let Some(manager) = engagement.manager_user_id {
        return Ok(vec![manager]);
    }
    Ok(role_tasks::assignee(&state.db, MANAGER_ROLE)
        .await?
        .into_iter()
        .collect())
}

/// Tells `users` with the notice `key`.
async fn tell_users(state: &AppState, key: &str, ret: &Return, users: &[i64]) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(key, ret.subject())
        .subject_label(&ret.number())
        .data(notice_data(ret));
    for user in users {
        request = request.to_user(*user);
    }
    notifier(state).notify(state, request).await?;
    Ok(())
}

/// The linked deadline task of `ret`, if there is one and ticketing is on.
async fn linked_task(state: &AppState, ret: &Return) -> Result<Option<DeadlineTaskSummary>> {
    match ret.task() {
        Some(id) => deadline_tasks(state).find(&state.db, id).await,
        None => Ok(None),
    }
}

/// What the preparer does with the linked task `task`, by its state.
fn hint(task: &DeadlineTaskSummary) -> String {
    english(
        &format!("accounting.returns.withdrawn.hint.{}", task.status.as_str()),
        &[],
    )
}

/// The description of the preparer's task for a withdrawn approval: the
/// reason, and the linked task with its state and what to do with it.
fn withdrawn_description(ret: &Return, reason: &str, task: Option<&DeadlineTaskSummary>) -> String {
    let mut text = english(
        "accounting.returns.withdrawn.description",
        &[
            ("number", &ret.number()),
            ("reason", &reason),
            ("link", &format!("/accounting/returns/{}", ret.id)),
        ],
    );
    if let Some(task) = task {
        text.push('\n');
        text.push_str(&english(
            "accounting.returns.withdrawn.linked",
            &[
                ("title", &task.title),
                ("status", &task.status.as_str()),
                ("hint", &hint(task)),
            ],
        ));
    }
    text
}

/// The last reason a signature of `ret` was withdrawn with.
async fn withdrawn_reason(state: &AppState, ret: &Return) -> Result<String> {
    let signatures = crate::tax_return::ReturnSignature::of_return(&state.db, ret.id).await?;
    Ok(signatures
        .iter()
        .rev()
        .find_map(|s| s.withdrawn_reason.clone())
        .unwrap_or_default())
}

/// Opens the preparer's task: due in one working day, naming the linked
/// task and its state.
async fn open_withdrawn_task(state: &AppState, ret: &Return) -> Result {
    let reason = withdrawn_reason(state, ret).await?;
    let task = linked_task(state, ret).await?;
    let title = english(
        "accounting.returns.withdrawn.title",
        &[("number", &ret.number())],
    );
    let description = withdrawn_description(ret, &reason, task.as_ref());
    let due = calendar(state).add_working_days(renox::db::now(), 1);
    let key = withdrawn_key(ret.id);
    match ret.preparer_user_id {
        Some(preparer) => {
            let creator = task_creator(state);
            if creator.find_open(state, &key).await?.is_none() {
                let ticket = OpenTicket::new(ret.subject(), TASK_CATEGORY, &title)
                    .description(&description)
                    .assign_to(Assignee::user(preparer))
                    .due_at(due)
                    .source_key(&key);
                creator.open(state, ticket).await?;
            }
        }
        None => {
            let task = RoleTask::new(ret.subject(), ACCOUNTANT_ROLE, &title, &key)
                .description(&description)
                .due_at(due);
            role_tasks::open(state, task).await?;
        }
    }
    Ok(())
}

/// Closes the preparer's task for a withdrawn approval, if it is open.
async fn close_withdrawn_task(state: &AppState, ret: &Return, note: &str) -> Result {
    let creator = task_creator(state);
    if let Some(open) = creator.find_open(state, &withdrawn_key(ret.id)).await? {
        creator.complete(state, open.id, note).await?;
    }
    Ok(())
}

/// After the commit of `event` on a return.
///
/// # Errors
///
/// The notifier's, the task services' and the database's errors.
pub async fn after(state: &AppState, event: &Transitioned<Return>) -> Result {
    let ret = Return::find_or_404(&state.db, event.key).await?;
    match event.transition.as_str() {
        SUBMIT => {
            let note = english("accounting.returns.withdrawn.closed", &[]);
            close_withdrawn_task(state, &ret, &note).await?;
            let users = reviewers(state, &ret).await?;
            tell_users(state, SUBMITTED_NOTICE, &ret, &users).await?;
        }
        SEND_BACK => {
            let users: Vec<i64> = ret.preparer_user_id.into_iter().collect();
            tell_users(state, SENT_BACK_NOTICE, &ret, &users).await?;
        }
        REVIEW => {
            let recipients = client_recipients(state, ret.client(), today()).await?;
            let mut request = NotifyRequest::new(FOR_APPROVAL_NOTICE, ret.subject())
                .subject_label(&ret.number())
                .data(notice_data(&ret));
            for recipient in recipients {
                request = request.to(recipient);
            }
            notifier(state).notify(state, request).await?;
        }
        WITHDRAW_APPROVAL => {
            let mut users: Vec<i64> = ret.preparer_user_id.into_iter().collect();
            users.extend(ret.reviewer_user_id);
            users.sort_unstable();
            users.dedup();
            let reason = withdrawn_reason(state, &ret).await?;
            if !users.is_empty() {
                let mut data = notice_data(&ret);
                data["reason"] = Value::String(reason);
                let mut request = NotifyRequest::new(WITHDRAWN_NOTICE, ret.subject())
                    .subject_label(&ret.number())
                    .data(data);
                for user in users {
                    request = request.to_user(user);
                }
                notifier(state).notify(state, request).await?;
            }
            open_withdrawn_task(state, &ret).await?;
        }
        CANCEL => cancelled(state, &ret).await?,
        CORRECT => corrections::after_correct(state, ret.id).await?,
        FILE | FILE_APPROVED => filed(state, &ret).await?,
        _ => {}
    }
    Ok(())
}

/// A return was filed (by a tax agent, by the client, or on the MLRO's
/// approval): the linked task is completed with the acknowledgement, the
/// client is reminded to pay, an expired client KYC document flags the
/// return, and the corrections made through it are cancelled.
async fn filed(state: &AppState, ret: &Return) -> Result {
    corrections::on_filed(state, ret).await?;
    if let Some(task) = ret.task() {
        let evidence = ret.acknowledgement.clone().unwrap_or_default();
        deadline_tasks(state)
            .complete(state, task, &evidence)
            .await?;
    }
    if ret.tax_payable.unwrap_or(0) > 0 {
        let recipients = client_recipients(state, ret.client(), today()).await?;
        let mut data = notice_data(ret);
        data["amount"] = Value::String(crate::money_text(ret.tax_payable.unwrap_or(0)));
        let mut request = NotifyRequest::new(PAYMENT_REMINDER_NOTICE, ret.subject())
            .subject_label(&ret.number())
            .data(data);
        for recipient in recipients {
            request = request.to(recipient);
        }
        notifier(state).notify(state, request).await?;
    }
    documents::flag_expired(state, ret).await?;
    Ok(())
}

/// A return was cancelled: its linked task and the client.
async fn cancelled(state: &AppState, ret: &Return) -> Result {
    let note = english("accounting.returns.cancelled.closed", &[]);
    close_withdrawn_task(state, ret, &note).await?;
    let reason = ret.cancel_reason.as_deref().unwrap_or_default();
    let tasks = deadline_tasks(state);
    let linked = linked_task(state, ret).await?;
    match reason {
        reasons::NOT_REQUIRED => {
            let Some(task) = linked else {
                return Ok(());
            };
            if task.status == DeadlineTaskStatus::Upcoming {
                let waive =
                    WaiveTask::new(&english("accounting.returns.cancelled.waive_reason", &[]));
                tasks.waive(state, task.id, waive).await?;
            } else if !task.status.is_terminal() {
                list_for_manager(state, ret, &task).await?;
            }
        }
        reasons::CLIENT_FILES_ITSELF | reasons::ENGAGEMENT_ENDED => {
            if let Some(task) = linked {
                // The engagement ending generates no next occurrence.
                let hand_over = if reason == reasons::ENGAGEMENT_ENDED {
                    HandOverTask::new().without_next()
                } else {
                    HandOverTask::new()
                };
                tasks.hand_over(state, task.id, hand_over).await?;
            }
            if !ret.client_messages_suppressed {
                tell_client(state, ret).await?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Lists the started linked `task` of the cancelled `ret` for the manager,
/// who completes it or closes it (04 keeps *waive a started task* off).
async fn list_for_manager(state: &AppState, ret: &Return, task: &DeadlineTaskSummary) -> Result {
    let title = english(
        "accounting.returns.cancelled.left_title",
        &[("number", &ret.number())],
    );
    let description = english(
        "accounting.returns.cancelled.left_description",
        &[
            ("number", &ret.number()),
            ("title", &task.title),
            ("status", &task.status.as_str()),
        ],
    );
    let key = left_key(ret.id);
    let list = RoleTask::new(ret.subject(), MANAGER_ROLE, &title, &key).description(&description);
    role_tasks::open(state, list).await?;
    Ok(())
}

/// Tells the client approvers who files the return now and by when.
async fn tell_client(state: &AppState, ret: &Return) -> Result {
    let recipients = client_recipients(state, ret.client(), today()).await?;
    let mut request = NotifyRequest::new(HANDED_OVER_NOTICE, ret.subject())
        .subject_label(&ret.number())
        .data(notice_data(ret));
    for recipient in recipients {
        request = request.to(recipient);
    }
    notifier(state).notify(state, request).await?;
    Ok(())
}

/// `ACC.RUL.04` made the deadline task `event.task` overdue: for a return it
/// tracks whose approval was withdrawn, the preparer's task now says to
/// `start` the linked task.
///
/// # Errors
///
/// The task service's and the database's errors.
pub async fn on_task_overdue(state: &AppState, event: &TaskOverdue) -> Result {
    let returns = Return::query()
        .where_(Return::TASK_ID.eq(event.task.0))
        .where_(Return::STATUS.eq(ReturnStatus::Preparing))
        .get(&state.db)
        .await?;
    for ret in returns {
        let creator = task_creator(state);
        let Some(open) = creator.find_open(state, &withdrawn_key(ret.id)).await? else {
            continue;
        };
        let text = english(
            "accounting.returns.withdrawn.overdue",
            &[
                ("number", &ret.number()),
                ("on", &event.due_date.succ_opt().unwrap_or(event.due_date)),
            ],
        );
        creator
            .comment(state, open.id, TaskComment::internal(&text))
            .await?;
    }
    Ok(())
}
