//! The statutory deadline notice of a suspension or an end (04 §4.3, §6
//! *Engagement*, §7.2 `ACC.RUL.09`; BL-ACC-04).
//!
//! When the firm stops acting for a client, the client must know which tax
//! deadlines are coming and that the firm will not act on them. After an
//! engagement `suspend` or `end` commits, [`run_for`] runs `ACC.RUL.09` once
//! per transition (a rule run, so a failure is retried and, the rule being
//! compliance-critical, alerts the partner role and the company admin):
//!
//! | Transition | Reason | What happens |
//! |---|---|---|
//! | `suspend` (a user) | not restricted | the notice is sent to the client approvers |
//! | `auto_suspend` | *non-payment* (`ACC.RUL.07`) | the notice is sent |
//! | `auto_suspend` | *on hold* (a block), not restricted | nothing is sent; the partner gets a task with the deadline list and the neutral template |
//! | `auto_suspend` | *on hold*, restricted | nothing is sent, no task |
//! | `end` | not restricted | the notice is sent |
//! | `end` | restricted (after a CDD exit or rejection) | nothing is sent; the MLRO gets a task with the deadline list and the neutral template |
//!
//! The notice lists the client's compliance calendar tasks of a statutory
//! type ([`STATUTORY_TYPES`]) due from the transition day to 90 days later
//! inclusive ([`WINDOW_DAYS`]), says the firm will not act on them (or no
//! longer acts on them), and asks the client approver to acknowledge it in
//! the portal; with no deadline in the window it says so and still asks.
//! It is a [`DeadlineNotice`] record, whose subject
//! (`accounting.deadline_notice:N`) the notice is about, so its delivery is
//! known on its own.
//!
//! The notice is a statutory notification type: when it cannot be
//! delivered, wl-notify marks the contact point invalid and gives staff a
//! task to deliver it another way and record the proof (BL-NTF-05). The
//! 3-day reminder and the 7-day partner task count from the delivery day
//! (the provider's receipt) or the day the proof states, never before
//! ([`follow_up`], daily). One acknowledgement by any client approver is
//! enough ([`acknowledge`]). When the engagement resumes before it,
//! the reminder and the partner task are cancelled ([`cancel_pending`]).
//!
//! The client approvers are the portal users acting for the client company
//! with an `owner` or `approver` grant role ([`APPROVER_ROLES`]), through the
//! kernel's [`PortalUsers`](wl_kernel::portal::PortalUsers). Decision: the
//! notice goes to each approver as a party (the person on the grant), so a
//! bounce marks that person's contact point invalid; with no approver it
//! goes to the client company itself.

use std::sync::{Arc, Mutex, PoisonError};

use renox::chrono::{Duration, NaiveDate};
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::history;
use serde::{Deserialize, Serialize};
use wl_kernel::automation::{RuleRunSpec, RunOutcome, run_rule};
use wl_kernel::compliance::{CriticalRule, occurrence_key, schedule_subject};
use wl_kernel::deadlines::deadline_tasks;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::notify::{
    self, GROUP_REMINDERS, NoticeStatus, NotificationTypeDef, NotifyChannel, NotifyRecipient,
    NotifyRequest, notifier,
};
use wl_kernel::portal::{PortalUser, portal_users};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{TicketId, task_creator};
use wl_kernel::{PartyId, Subject};

use crate::client_notices::NeutralTemplate;
use crate::db::refused;
use crate::engagement::Engagement;
use crate::engagement_machine::{AUTO_SUSPEND, END, SUSPEND};
use crate::permissions::{self, MLRO_ROLE, PARTNER_ROLE};
use crate::reasons;
use crate::texts::english;

/// The rule.
pub const RULE: &str = "ACC.RUL.09";
/// The days after the transition day the window ends (N = 90, inclusive).
pub const WINDOW_DAYS: i64 = 90;
/// Days after delivery the client approvers are reminded.
pub const REMIND_AFTER_DAYS: i64 = 3;
/// Days after delivery without an acknowledgement the partner gets a task.
pub const ESCALATE_AFTER_DAYS: i64 = 7;
/// When the daily follow-up runs, Asia/Dubai.
pub const FOLLOW_UP_AT: &str = "08:00";
/// The scheduled task and command of the daily follow-up.
pub const FOLLOW_UP_TASK: &str = "accounting:deadline_notices";

/// The statutory notice sent to the client approvers.
pub const DEADLINE_NOTICE: &str = "accounting.deadline_notice";
/// The reminder after 3 days without an acknowledgement.
pub const DEADLINE_REMINDER: &str = "accounting.deadline_notice_reminder";
/// The subject type of a deadline notice.
pub const DEADLINE_NOTICE_SUBJECT_TYPE: &str = "accounting.deadline_notice";

/// The grant roles that make a portal user a client approver.
pub const APPROVER_ROLES: [&str; 2] = ["owner", "approver"];

/// The task types of the compliance calendar that are statutory deadlines:
/// VAT return and payment, corporate tax registration and return, UBO
/// updates, trade licence renewal.
pub const STATUTORY_TYPES: [&str; 6] = [
    crate::calendar::VAT_RETURN,
    "vat_payment",
    "ct_registration",
    crate::calendar::CT_RETURN,
    "ubo_update",
    "trade_licence_renewal",
];

/// The module that sends the notices.
const MODULE: &str = "accounting";

/// What the notice follows.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NoticeCause {
    /// The engagement was suspended.
    #[default]
    Suspend,
    /// The engagement ended.
    End,
}

/// A statutory deadline notice of an engagement.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "accounting_deadline_notices", index(engagement_id))]
pub struct DeadlineNotice {
    /// The primary key.
    pub id: i64,
    /// The engagement.
    #[model(references = "accounting_engagements")]
    pub engagement_id: i64,
    /// The client company.
    pub client_party_id: i64,
    /// The suspension or the end.
    pub cause: NoticeCause,
    /// The role that decides when it is sent (the partner after a block, the
    /// MLRO after a restricted end); `None` when it was sent automatically.
    pub prepared_for: Option<String>,
    /// The transition day: the window's first day (Asia/Dubai).
    pub on_date: NaiveDate,
    /// The window's last day.
    pub window_end: NaiveDate,
    /// The deadlines listed, one per line ("2026-11-28 — VAT return …").
    pub lines: String,
    /// How many deadlines are listed.
    pub deadline_count: i64,
    /// The task of the role that decides, when prepared.
    pub task_id: Option<i64>,
    /// When it was sent to the client.
    pub sent_at: Option<DateTime>,
    /// Who sent it, when a user did (a prepared notice).
    pub sent_by: Option<i64>,
    /// The day it was delivered (the provider's receipt) or the day a proof
    /// of delivery states: the 3-day and 7-day clocks start here.
    pub delivered_on: Option<NaiveDate>,
    /// The day the reminder was sent.
    pub reminded_on: Option<NaiveDate>,
    /// The day the partner got the task for want of an acknowledgement.
    pub escalated_on: Option<NaiveDate>,
    /// That task.
    pub escalation_task_id: Option<i64>,
    /// When a client approver acknowledged it.
    pub acknowledged_at: Option<DateTime>,
    /// The user who acknowledged it.
    pub acknowledged_by: Option<i64>,
    /// The day the follow-up was cancelled (the engagement resumed).
    pub cancelled_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DeadlineNotice {
    /// The notice as a kernel subject: `accounting.deadline_notice:4`.
    pub fn subject(&self) -> Subject {
        Subject::new(DEADLINE_NOTICE_SUBJECT_TYPE, self.id)
    }

    /// The client company.
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// Whether it was sent and waits for an acknowledgement: not yet
    /// acknowledged and not cancelled.
    pub fn is_open(&self) -> bool {
        self.sent_at.is_some() && self.acknowledged_at.is_none() && self.cancelled_on.is_none()
    }

    /// The deadlines listed, one text each; empty when there are none.
    pub fn line_list(&self) -> Vec<String> {
        if self.deadline_count == 0 {
            return Vec::new();
        }
        self.lines
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// The portal page where the client approver reads and acknowledges it.
    pub fn portal_path(&self) -> String {
        format!("/accounting/portal/deadline-notices/{}", self.id)
    }

    /// The notices of engagement `id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_engagement(db: &Db, id: i64) -> Result<Vec<DeadlineNotice>> {
        DeadlineNotice::query()
            .where_(DeadlineNotice::ENGAGEMENT_ID.eq(id))
            .order_by(DeadlineNotice::ID)
            .get(db)
            .await
    }
}

/// One statutory deadline in the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoticeLine {
    /// The due date.
    pub due_date: NaiveDate,
    /// The task's title ("VAT return, period ending 2026-10-31").
    pub title: String,
}

/// The statutory deadlines of `engagement` due from `on` to `on` + 90 days
/// inclusive, by due date: its open compliance calendar tasks of a
/// statutory type.
///
/// # Errors
///
/// The task service's errors.
pub async fn deadlines(
    state: &AppState,
    engagement: &Engagement,
    on: NaiveDate,
) -> Result<Vec<NoticeLine>> {
    let until = on + Duration::days(WINDOW_DAYS);
    let mut lines: Vec<NoticeLine> = deadline_tasks(state)
        .of_subject(&state.db, &engagement.subject())
        .await?
        .into_iter()
        .filter(|task| {
            STATUTORY_TYPES.contains(&task.task_type.as_str())
                && !task.status.is_terminal()
                && task.due_date >= on
                && task.due_date <= until
        })
        .map(|task| NoticeLine {
            due_date: task.due_date,
            title: task.title,
        })
        .collect();
    lines.sort_by(|a, b| {
        a.due_date
            .cmp(&b.due_date)
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(lines)
}

/// The deadlines as the notice lists them, one per line; the sentence that
/// there are none when `lines` is empty.
pub fn lines_text(lines: &[NoticeLine]) -> String {
    if lines.is_empty() {
        return english(
            "accounting.deadline_notices.none",
            &[("days", &WINDOW_DAYS)],
        );
    }
    lines
        .iter()
        .map(|line| {
            english(
                "accounting.deadline_notices.line",
                &[("date", &line.due_date), ("title", &line.title)],
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the notice says the firm will do: "will not act on these while the
/// engagement is suspended" or "no longer acts on these".
pub fn wording(cause: NoticeCause, number: &str) -> String {
    let key = match cause {
        NoticeCause::Suspend => "accounting.deadline_notices.wording_suspend",
        NoticeCause::End => "accounting.deadline_notices.wording_end",
    };
    english(key, &[("number", &number), ("days", &WINDOW_DAYS)])
}

/// What `ACC.RUL.09` does after a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticePlan {
    /// Send the notice to the client approvers now.
    Send,
    /// Send nothing; give the holder of this role a task with the deadline
    /// list and the neutral template.
    Prepare(&'static str),
    /// Send nothing and open no task.
    Nothing,
}

/// The plan for `transition` on `engagement` as it is after the commit.
///
/// Decision: a suspension of a blocked client is told apart by its reason
/// *on hold*; whether the block's own reason is restricted is what the
/// engagement keeps as "client messages suppressed" (`ACC.ACL.14`), never
/// the reason itself.
pub fn plan_for(transition: &str, engagement: &Engagement) -> NoticePlan {
    let restricted = engagement.client_messages_suppressed;
    let from_block = engagement.reason_code.as_deref() == Some(reasons::ON_HOLD);
    match transition {
        SUSPEND if !restricted => NoticePlan::Send,
        AUTO_SUSPEND if from_block && !restricted => NoticePlan::Prepare(PARTNER_ROLE),
        AUTO_SUSPEND if !restricted => NoticePlan::Send,
        END if restricted => NoticePlan::Prepare(MLRO_ROLE),
        END => NoticePlan::Send,
        _ => NoticePlan::Nothing,
    }
}

/// The client approvers of `client` on `on`: the portal users acting for it
/// with an `owner` or `approver` grant role.
///
/// # Errors
///
/// The portal's errors.
pub async fn approvers(
    state: &AppState,
    client: PartyId,
    on: NaiveDate,
) -> Result<Vec<PortalUser>> {
    Ok(portal_users(state)
        .users_for(&state.db, client, on)
        .await?
        .into_iter()
        .filter(|user| user.has_any_role(&APPROVER_ROLES))
        .collect())
}

/// Who a message to the client goes to: each client approver (the person
/// on the grant, once), else the client company.
///
/// # Errors
///
/// The portal's errors.
pub async fn client_recipients(
    state: &AppState,
    client: PartyId,
    on: NaiveDate,
) -> Result<Vec<NotifyRecipient>> {
    let mut recipients = Vec::new();
    for user in approvers(state, client, on).await? {
        let recipient = NotifyRecipient::Party { id: user.person };
        if !recipients.contains(&recipient) {
            recipients.push(recipient);
        }
    }
    if recipients.is_empty() {
        recipients.push(NotifyRecipient::Party { id: client });
    }
    Ok(recipients)
}

/// The id and day of the latest history row of `transition` on engagement
/// `id`: the occurrence a run is for.
async fn occurrence(state: &AppState, id: i64, transition: &str) -> Result<(i64, NaiveDate)> {
    let rows = history(&state.db, Engagement::TABLE, id).await?;
    Ok(rows
        .iter()
        .rev()
        .find(|row| row.transition == transition)
        .map_or((0, today()), |row| (row.id, dubai_date(row.at))))
}

/// Runs `ACC.RUL.09` for the `transition` engagement `id` just went
/// through (`suspend`, `auto_suspend` or `end`), once per transition: a
/// second call for the same history row does nothing.
///
/// # Errors
///
/// The database's errors (a failing action is retried by the rule run).
pub async fn run_for(state: &AppState, id: i64, transition: &str) -> Result<RunOutcome> {
    let (row, on) = occurrence(state, id, transition).await?;
    let subject = Subject::new(crate::engagement::ENGAGEMENT_SUBJECT_TYPE, id);
    let key = format!("{RULE}:{subject}:{transition}:{row}");
    let spec = RuleRunSpec::new(RULE, subject, key).compliance_critical();
    let transition = transition.to_owned();
    run_rule(state, spec, move |state: AppState| async move {
        apply(&state, id, &transition, on).await.map(|_| ())
    })
    .await
}

/// `ACC.RUL.09` for engagement `id` after `transition` on `on`: sends the
/// notice, prepares it for the partner or the MLRO, or does nothing
/// ([`plan_for`]). Answers the notice recorded, if any.
///
/// # Errors
///
/// The database's, the task service's and the notifier's errors.
pub async fn apply(
    state: &AppState,
    id: i64,
    transition: &str,
    on: NaiveDate,
) -> Result<Option<DeadlineNotice>> {
    let engagement = Engagement::find_or_404(&state.db, id).await?;
    let plan = plan_for(transition, &engagement);
    if plan == NoticePlan::Nothing {
        return Ok(None);
    }
    let cause = if transition == END {
        NoticeCause::End
    } else {
        NoticeCause::Suspend
    };
    let lines = deadlines(state, &engagement, on).await?;
    let notice = DeadlineNotice {
        engagement_id: engagement.id,
        client_party_id: engagement.client_party_id,
        cause,
        prepared_for: match plan {
            NoticePlan::Prepare(role) => Some(role.to_owned()),
            _ => None,
        },
        on_date: on,
        window_end: on + Duration::days(WINDOW_DAYS),
        lines: lines_text(&lines),
        deadline_count: i64::try_from(lines.len()).unwrap_or(i64::MAX),
        ..DeadlineNotice::default()
    };
    let mut notice = DeadlineNotice::create(&state.db, notice).await?;
    match plan {
        NoticePlan::Send => send(state, &engagement, &mut notice, None).await?,
        NoticePlan::Prepare(role) => prepare(state, &engagement, &mut notice, role).await?,
        NoticePlan::Nothing => {}
    }
    Ok(Some(notice))
}

/// The template data of the notice and its reminder.
fn notice_data(engagement: &Engagement, notice: &DeadlineNotice) -> Value {
    json!({
        "number": engagement.number(),
        "wording": wording(notice.cause, &engagement.number()),
        "lines": notice.lines,
        "until": notice.window_end.to_string(),
        "link": notice.portal_path(),
    })
}

/// Sends `notice` of `engagement` to the client approvers now (`by` when a
/// user decided it) and records when.
async fn send(
    state: &AppState,
    engagement: &Engagement,
    notice: &mut DeadlineNotice,
    by: Option<i64>,
) -> Result {
    let recipients = client_recipients(state, engagement.client(), today()).await?;
    notice.sent_at = Some(now());
    notice.sent_by = by;
    notice.save(&state.db).await?;
    let mut request = NotifyRequest::new(DEADLINE_NOTICE, notice.subject())
        .subject_label(&engagement.number())
        .data(notice_data(engagement, notice));
    for recipient in recipients {
        request = request.to(recipient);
    }
    notifier(state).notify(state, request).await?;
    Ok(())
}

/// The key the task of the role that decides is found by.
pub fn prepared_key(notice: i64) -> String {
    format!("accounting.deadline_notice:{notice}")
}

/// The key the partner's task for want of an acknowledgement is found by.
pub fn escalation_key(notice: i64) -> String {
    format!("accounting.deadline_notice_ack:{notice}")
}

/// The neutral template that fits `cause`, as the client reads it.
fn neutral_text(cause: NoticeCause) -> String {
    let template = match cause {
        NoticeCause::Suspend => NeutralTemplate::OnHold,
        NoticeCause::End => NeutralTemplate::Ended,
    };
    english(
        &format!("accounting.client_notices.{}_body", template.as_str()),
        &[],
    )
}

/// Gives `role` the task with the pre-filled deadline list and the neutral
/// template of `notice`; nothing goes to the client.
async fn prepare(
    state: &AppState,
    engagement: &Engagement,
    notice: &mut DeadlineNotice,
    role: &str,
) -> Result {
    let number = engagement.number();
    let title = english(
        "accounting.deadline_notices.prepared_title",
        &[("number", &number)],
    );
    let description = english(
        "accounting.deadline_notices.prepared_description",
        &[
            ("number", &number),
            ("until", &notice.window_end),
            ("lines", &notice.lines),
            ("template", &neutral_text(notice.cause)),
            (
                "link",
                &format!("/accounting/engagements/{}", engagement.id),
            ),
        ],
    );
    let task = RoleTask::new(engagement.subject(), role, &title, &prepared_key(notice.id))
        .description(&description);
    let opened = role_tasks::open(state, task).await?;
    notice.task_id = opened.map(|t| t.id.0);
    notice.save(&state.db).await?;
    Ok(())
}

/// The permission that sends a notice prepared for `role`: the partner's
/// `engagements.end`, the MLRO's `parties.block_reason.view`.
fn sender_permission(role: &str) -> &'static str {
    if role == MLRO_ROLE {
        permissions::BLOCK_REASON_VIEW
    } else {
        permissions::ENGAGEMENTS_END
    }
}

/// Whether `user` may send the prepared notice `notice` now.
pub fn may_send(user: &AuthUser, notice: &DeadlineNotice) -> bool {
    notice.sent_at.is_none()
        && notice.cancelled_on.is_none()
        && notice
            .prepared_for
            .as_deref()
            .is_some_and(|role| user.has_permission(sender_permission(role)))
}

/// `by` (the partner, or the MLRO for a restricted end) decides to send the
/// prepared notice `id`: it goes to the client approvers, the clocks start
/// from its delivery, and the task is completed.
///
/// # Errors
///
/// `Forbidden` without the permission of the role it was prepared for; a
/// 422 when it was not prepared, or was already sent or cancelled; the
/// notifier's errors.
pub async fn send_prepared(state: &AppState, by: &AuthUser, id: i64) -> Result<DeadlineNotice> {
    let mut notice = DeadlineNotice::find_or_404(&state.db, id).await?;
    let Some(role) = notice.prepared_for.clone() else {
        return Err(refused(english(
            "accounting.deadline_notices.not_prepared",
            &[],
        )));
    };
    if !by.has_permission(sender_permission(&role)) {
        return Err(Error::Forbidden);
    }
    if !may_send(by, &notice) {
        return Err(refused(english(
            "accounting.deadline_notices.not_prepared",
            &[],
        )));
    }
    let engagement = Engagement::find_or_404(&state.db, notice.engagement_id).await?;
    send(state, &engagement, &mut notice, Some(by.id)).await?;
    if let Some(task) = notice.task_id {
        let note = english("accounting.deadline_notices.sent", &[]);
        let _closed = task_creator(state)
            .complete(state, TicketId(task), &note)
            .await?;
    }
    Ok(notice)
}

/// The portal user `user` acting for the client of `notice` today, if any.
///
/// # Errors
///
/// The portal's errors.
pub async fn acting_for(
    state: &AppState,
    user: i64,
    notice: &DeadlineNotice,
) -> Result<Option<PortalUser>> {
    portal_users(state)
        .acting_for(&state.db, user, notice.client(), today())
        .await
}

/// A client approver (`user`, acting for the client in the portal)
/// acknowledges notice `id`; one acknowledgement is enough, a second one
/// changes nothing. The partner's task for want of an acknowledgement, if
/// already open, is completed.
///
/// # Errors
///
/// 404 when the notice was not sent or `user` is not a client approver of
/// its client.
pub async fn acknowledge(state: &AppState, user: &AuthUser, id: i64) -> Result<DeadlineNotice> {
    let mut notice = DeadlineNotice::find_or_404(&state.db, id).await?;
    if notice.sent_at.is_none() {
        return Err(Error::NotFound);
    }
    let approver = acting_for(state, user.id, &notice)
        .await?
        .is_some_and(|acting| acting.has_any_role(&APPROVER_ROLES));
    if !approver {
        return Err(Error::NotFound);
    }
    if notice.acknowledged_at.is_some() {
        return Ok(notice);
    }
    notice.acknowledged_at = Some(now());
    notice.acknowledged_by = Some(user.id);
    notice.save(&state.db).await?;
    if let Some(task) = notice.escalation_task_id {
        let note = english("accounting.deadline_notices.acknowledged_note", &[]);
        let _closed = task_creator(state)
            .complete(state, TicketId(task), &note)
            .await?;
    }
    Ok(notice)
}

/// What a follow-up run did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FollowUp {
    /// The notices whose client approvers were reminded.
    pub reminded: Vec<i64>,
    /// The notices for which the partner got a task.
    pub escalated: Vec<i64>,
    /// The notices not delivered yet (no receipt, no proof): no clock runs.
    pub undelivered: Vec<i64>,
}

/// The day the clocks of `notice` start: its delivery day, asked of the
/// notifier the first time it is known; `None` while not delivered.
async fn delivered_on(state: &AppState, notice: &mut DeadlineNotice) -> Result<Option<NaiveDate>> {
    if notice.delivered_on.is_some() {
        return Ok(notice.delivered_on);
    }
    let day = match notifier(state)
        .notice_status(state, &notice.subject(), DEADLINE_NOTICE)
        .await?
    {
        NoticeStatus::Delivered { at } => dubai_date(at),
        NoticeStatus::ProofRecorded { delivered_on } => delivered_on,
        NoticeStatus::NotDelivered => return Ok(None),
    };
    notice.delivered_on = Some(day);
    notice.save(&state.db).await?;
    Ok(Some(day))
}

/// The follow-up of every open notice on `on`, without the rule run.
///
/// # Errors
///
/// The database's, the notifier's and the task service's errors.
pub async fn follow_up_now(state: &AppState, on: NaiveDate) -> Result<FollowUp> {
    let mut report = FollowUp::default();
    let all = DeadlineNotice::query()
        .where_(DeadlineNotice::SENT_AT.is_not_null())
        .order_by(DeadlineNotice::ID)
        .get(&state.db)
        .await?;
    for mut notice in all.into_iter().filter(DeadlineNotice::is_open) {
        let Some(engagement) = Engagement::find(&state.db, notice.engagement_id).await? else {
            continue;
        };
        let Some(delivered) = delivered_on(state, &mut notice).await? else {
            report.undelivered.push(notice.id);
            continue;
        };
        if notice.reminded_on.is_none()
            && on >= delivered + Duration::days(REMIND_AFTER_DAYS)
            && engagement.may_message_client()
        {
            remind(state, &engagement, &notice).await?;
            notice.reminded_on = Some(on);
            notice.save(&state.db).await?;
            report.reminded.push(notice.id);
        }
        if notice.escalated_on.is_none() && on >= delivered + Duration::days(ESCALATE_AFTER_DAYS) {
            let number = engagement.number();
            let title = english(
                "accounting.deadline_notices.escalation_title",
                &[("number", &number)],
            );
            let description = english(
                "accounting.deadline_notices.escalation_description",
                &[
                    ("number", &number),
                    ("delivered", &delivered),
                    ("days", &ESCALATE_AFTER_DAYS),
                    ("lines", &notice.lines),
                ],
            );
            let task = RoleTask::new(
                engagement.subject(),
                PARTNER_ROLE,
                &title,
                &escalation_key(notice.id),
            )
            .description(&description);
            let opened = role_tasks::open(state, task).await?;
            notice.escalated_on = Some(on);
            notice.escalation_task_id = opened.map(|t| t.id.0);
            notice.save(&state.db).await?;
            report.escalated.push(notice.id);
        }
    }
    Ok(report)
}

/// Reminds the client approvers of `notice`.
async fn remind(state: &AppState, engagement: &Engagement, notice: &DeadlineNotice) -> Result {
    let mut request = NotifyRequest::new(DEADLINE_REMINDER, notice.subject())
        .subject_label(&engagement.number())
        .data(notice_data(engagement, notice));
    for recipient in client_recipients(state, engagement.client(), today()).await? {
        request = request.to(recipient);
    }
    notifier(state).notify(state, request).await?;
    Ok(())
}

/// Runs the follow-up of `on` as `ACC.RUL.09`'s daily run: one rule run per
/// day, so a second call for the same day does nothing and a failure is
/// retried and alerts the owners.
///
/// # Errors
///
/// The database's errors.
pub async fn follow_up(state: &AppState, on: NaiveDate) -> Result<FollowUp> {
    let spec =
        RuleRunSpec::new(RULE, schedule_subject(), occurrence_key(RULE, on)).compliance_critical();
    let seen = Arc::new(Mutex::new(FollowUp::default()));
    let out = Arc::clone(&seen);
    run_rule(state, spec, move |state: AppState| async move {
        let report = follow_up_now(&state, on).await?;
        *out.lock().unwrap_or_else(PoisonError::into_inner) = report;
        Ok(())
    })
    .await?;
    let report = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Ok(report)
}

/// The engagement `id` resumed on `on`: its notices still waiting for an
/// acknowledgement send no more reminder and open no partner task; a
/// prepared notice not sent is not sent any more; their open tasks are
/// completed. Answers the notices cancelled.
///
/// # Errors
///
/// The database's and the task service's errors.
pub async fn cancel_pending(state: &AppState, id: i64, on: NaiveDate) -> Result<Vec<i64>> {
    let mut cancelled = Vec::new();
    let note = english("accounting.deadline_notices.cancelled", &[]);
    for mut notice in DeadlineNotice::of_engagement(&state.db, id).await? {
        if notice.acknowledged_at.is_some() || notice.cancelled_on.is_some() {
            continue;
        }
        notice.cancelled_on = Some(on);
        notice.save(&state.db).await?;
        let open_task = if notice.sent_at.is_none() {
            notice.task_id
        } else {
            notice.escalation_task_id
        };
        if let Some(task) = open_task {
            let _closed = task_creator(state)
                .complete(state, TicketId(task), &note)
                .await?;
        }
        cancelled.push(notice.id);
    }
    Ok(cancelled)
}

/// The notification types: the statutory notice (its give-up is wl-notify's
/// `statutory_notice`: contact point invalid and a delivery task for the
/// partner) and the reminder.
pub fn notification_types() -> Vec<NotificationTypeDef> {
    let subject = english("accounting.deadline_notices.notice_subject", &[]);
    let body = english("accounting.deadline_notices.notice_body", &[]);
    let reminder_subject = english("accounting.deadline_notices.reminder_subject", &[]);
    let reminder_body = english("accounting.deadline_notices.reminder_body", &[]);
    vec![
        NotificationTypeDef::new(DEADLINE_NOTICE, MODULE)
            .group("statutory")
            .statutory(true)
            .staff_role(PARTNER_ROLE)
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
        NotificationTypeDef::new(DEADLINE_REMINDER, MODULE)
            .group(GROUP_REMINDERS)
            .template(
                NotifyChannel::Email,
                "en",
                Some(&reminder_subject),
                &reminder_body,
            ),
    ]
}

/// Registers the notification types, the compliance-critical rule (owner:
/// the partner role, with the company admin) and the daily follow-up.
pub(crate) fn register(app: &mut Registry) {
    for def in notification_types() {
        notify::register_type(def);
    }
    CriticalRule::new(RULE)
        .owner_role(PARTNER_ROLE)
        .daily_at(FOLLOW_UP_AT)
        .register();
    app.schedule()
        .daily_at(FOLLOW_UP_AT, FOLLOW_UP_TASK, |state: AppState| async move {
            follow_up(&state, today()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        FOLLOW_UP_TASK,
        "Remind the client approvers and tell the partner of the deadline notices not acknowledged (ACC.RUL.09)",
        |_args, state: AppState| async move {
            let report = follow_up(&state, today()).await?;
            println!(
                "{} reminders sent, {} partner tasks opened, {} notices not delivered yet.",
                report.reminded.len(),
                report.escalated.len(),
                report.undelivered.len()
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn the_plan_follows_the_transition_and_the_reason() {
        let mut engagement = Engagement {
            reason_code: Some(reasons::CLIENT_REQUEST.into()),
            ..Engagement::default()
        };
        assert_eq!(plan_for(SUSPEND, &engagement), NoticePlan::Send);
        assert_eq!(plan_for(END, &engagement), NoticePlan::Send);
        engagement.reason_code = Some(reasons::ON_HOLD.into());
        assert_eq!(
            plan_for(AUTO_SUSPEND, &engagement),
            NoticePlan::Prepare(PARTNER_ROLE)
        );
        engagement.client_messages_suppressed = true;
        assert_eq!(plan_for(AUTO_SUSPEND, &engagement), NoticePlan::Nothing);
        assert_eq!(plan_for(END, &engagement), NoticePlan::Prepare(MLRO_ROLE));
        engagement.reason_code = Some(reasons::NON_PAYMENT.into());
        engagement.client_messages_suppressed = false;
        assert_eq!(plan_for(AUTO_SUSPEND, &engagement), NoticePlan::Send);
        assert_eq!(plan_for("resume", &engagement), NoticePlan::Nothing);
    }

    #[test]
    fn the_lines_name_each_deadline_or_say_there_is_none() {
        assert_eq!(
            lines_text(&[]),
            "There are no statutory deadlines in the next 90 days."
        );
        let text = lines_text(&[NoticeLine {
            due_date: day("2026-11-28"),
            title: "VAT return".into(),
        }]);
        assert_eq!(text, "2026-11-28 — VAT return");
        assert!(wording(NoticeCause::Suspend, "ENG-0001").contains("will not act on these"));
        assert!(wording(NoticeCause::End, "ENG-0001").contains("no longer acts on these"));
    }

    #[test]
    fn the_notice_is_statutory_and_its_give_up_asks_for_a_proof() {
        let types = notification_types();
        let notice = &types[0];
        assert!(notice.statutory);
        assert_eq!(
            notify::default_give_up(notice).reactions,
            [
                notify::GiveUpReaction::MarkContactInvalid,
                notify::GiveUpReaction::StatutoryNotice
            ]
        );
        assert!(!types[1].statutory);
    }
}
