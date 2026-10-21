//! Terminating a project mid-way (`terminate`, `CON.ACL.15`, 02 Construction
//! §3.3, §4.8, §7.1 *Project*; BL-CON-01).
//!
//! When the client terminates the subcontract, the main contractor becomes
//! insolvent or both sides agree to stop, the project manager **requests**
//! the termination ([`request`]) with a reason, the termination notice and
//! the effective date. The kernel's interceptor defers the run as an approval
//! request for the MD (a holder of `projects.cancel`, never the requester,
//! [`crate::project::terminate_policy`]); the project keeps its state, and
//! work goes on, while it waits. At the final approval the kernel runs
//! `terminate_approved`, which has the same guard and effect.
//!
//! **In the transition's transaction** ([`terminate_effects`]): the reason,
//! the notice, the effective date and the requester are stored on the
//! project, both retention parts are made due (`make_due`, unless
//! `retention_due_on_termination` is off, `⚠ open: lawyer (L-37)`), and the
//! final account IPA is made: a `draft` IPA of the project becomes it, valued
//! to the effective date; an IPA still `submitted` stays and is certified
//! normally ([`crate::ipa::ensure_final_account_in`]).
//!
//! **After the commit** ([`after_termination`], once per project):
//!
//! - attendance recorded after the effective date (a past effective date is
//!   allowed) is kept and flagged for the reviewer; attendance for a later
//!   day is refused from now on ([`ProjectClosure`] answers the attendance
//!   module), so crews work until the effective date;
//! - the project manager gets a task to reassign the crews, due on the
//!   effective date;
//! - the open purchase requests with no order are cancelled, procurement gets
//!   one task listing the open orders to cancel or short-close (nothing is
//!   cancelled for the supplier), and a stock count of the site store is
//!   opened (the kernel's [`ScopeStock`](wl_kernel::inventory::ScopeStock));
//! - the accountant gets one task per `active` guarantee;
//! - the open snag and defect tickets are cancelled with the reason;
//! - the client contacts are e-mailed with the effective date;
//! - `ProjectTerminated` is emitted.
//!
//! Decisions where the spec is silent:
//!
//! - The request is `projects.terminate` in the project (the project
//!   manager); the approval step is `projects.cancel` or the company admin
//!   (the MD). The spec lists `projects.cancel` for `terminate`, which would
//!   let the MD request and approve: the project manager asks and the MD
//!   decides (`CON.ACL.15`).
//! - Crew assignments do not exist yet (wl-attendance has no crews): the
//!   reassign task stands for ending them, and the attendance gate is what
//!   keeps crews from working after the effective date.
//! - The wind-down runs from the `Transitioned` event, after the commit, so
//!   a failure leaves the project `terminated` and the step undone; the
//!   claim `wound_down_at` is taken first, so a repeat does nothing twice.

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::attendance::{ContextClosure, LabourFuture, attendance_review};
use wl_kernel::inventory::scope_stock;
use wl_kernel::notify::{
    self, Category, ContactPurpose, ContactRelation, NotificationTypeDef, NotifyChannel,
    NotifyRequest,
};
use wl_kernel::tickets::task_creator;

use crate::events::ProjectTerminated;
use crate::guarantee::{self, GuaranteeKind};
use crate::ipa::ensure_final_account_in;
use crate::permissions;
use crate::project::{Project, ProjectStatus, SUBJECT_TYPE, TERMINATE};
use crate::retention::make_all_due_in;
use crate::tasks::{self, Task};
use crate::texts::{english, template};

/// The notification type of the e-mail to the client contacts.
pub const NOTICE: &str = "construction.project_terminated";

/// The ticket categories of the snags and defects of a project.
const SNAG_CATEGORIES: [&str; 2] = ["snag", "defect"];

/// Why the project stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationReason {
    /// The client terminated the subcontract.
    TerminatedByClient,
    /// We are in default.
    OurDefault,
    /// The main contractor became insolvent.
    MainContractorInsolvent,
    /// Both sides agreed to stop.
    Agreed,
}

impl TerminationReason {
    /// The stored text.
    pub const fn as_str(self) -> &'static str {
        match self {
            TerminationReason::TerminatedByClient => "terminated_by_client",
            TerminationReason::OurDefault => "our_default",
            TerminationReason::MainContractorInsolvent => "main_contractor_insolvent",
            TerminationReason::Agreed => "agreed",
        }
    }

    /// The reason stored as `text`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "terminated_by_client" => Some(TerminationReason::TerminatedByClient),
            "our_default" => Some(TerminationReason::OurDefault),
            "main_contractor_insolvent" => Some(TerminationReason::MainContractorInsolvent),
            "agreed" => Some(TerminationReason::Agreed),
            _ => None,
        }
    }
}

/// What a `terminate` run carries to its guard and effect; it is also the
/// approval request's payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminateInput {
    /// One of [`TerminationReason`].
    #[serde(default)]
    pub reason: String,
    /// The termination notice, a document reference.
    #[serde(default)]
    pub notice: String,
    /// The last working day.
    #[serde(default)]
    pub effective_on: Option<NaiveDate>,
    /// The user who asked.
    #[serde(default)]
    pub requested_by: Option<i64>,
}

/// The input of a `terminate` run.
pub(crate) fn terminate_input(ctx: &TransitionCtx) -> TerminateInput {
    ctx.input_as::<TerminateInput>().unwrap_or_default()
}

/// Why a project cannot be terminated with `input`, if it cannot: the reason
/// is one of the four, the notice is recorded and the effective date is given.
pub fn terminate_problem(input: &TerminateInput) -> Option<String> {
    if TerminationReason::parse(&input.reason).is_none() {
        return Some(english("construction.terminate.reason_required", &[]));
    }
    if input.notice.trim().is_empty() {
        return Some(english("construction.terminate.notice_required", &[]));
    }
    if input.effective_on.is_none() {
        return Some(english("construction.terminate.date_required", &[]));
    }
    None
}

/// The effect of `terminate` inside its transaction (see the module).
pub(crate) async fn terminate_effects(
    tx: &mut Transaction,
    project: &Project,
    input: &TerminateInput,
) -> Result {
    let Some(effective_on) = input.effective_on else {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("construction.terminate.date_required", &[]),
        ));
    };
    let reason = Some(input.reason.clone());
    let notice = Some(input.notice.trim().to_owned());
    let on = Some(effective_on);
    let at = Some(renox::db::now());
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(
            &mut *tx,
            &[
                ("termination_reason", &reason),
                ("termination_notice", &notice),
                ("termination_effective_on", &on),
                ("termination_requested_by", &input.requested_by),
                ("terminated_at", &at),
            ],
        )
        .await?;
    if project.retention_due_on_termination {
        make_all_due_in(&mut *tx, project.id).await?;
    }
    ensure_final_account_in(&mut *tx, project.id, effective_on).await?;
    Ok(())
}

/// What a user enters to request the termination.
#[derive(Debug, Clone)]
pub struct TerminateRequest {
    /// Why the project stops.
    pub reason: TerminationReason,
    /// The termination notice, a document reference.
    pub notice: String,
    /// The last working day; a date in the past is allowed.
    pub effective_on: NaiveDate,
}

/// `user` requests the termination of project `id`. The run is deferred as an
/// approval request, announced to the approvers; the outcome is `Pending`
/// while a policy asks for the MD.
///
/// # Errors
///
/// `Forbidden` without `projects.terminate` in the project, `NotFrom` unless
/// the project is `in_progress` or `on_hold`, `Guard` (422) naming the missing
/// input. Nothing is written then.
pub async fn request(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    ask: TerminateRequest,
) -> std::result::Result<Outcome<Project>, TransitionError> {
    let project = Project::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let input = TerminateInput {
        reason: ask.reason.as_str().to_owned(),
        notice: ask.notice.trim().to_owned(),
        effective_on: Some(ask.effective_on),
        requested_by: Some(user.id),
    };
    let history = english(
        &format!("construction.terminate.history_{}", ask.reason.as_str()),
        &[],
    );
    let outcome = project
        .transition(TERMINATE)
        .by(user)
        .reason(history)
        .input(&input)
        .run(state)
        .await?;
    wl_kernel::approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(outcome)
}

/// What [`after_termination`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WoundDown {
    /// Whether this run did the work (`false` for a repeat).
    pub ran: bool,
    /// The attendance records flagged for review.
    pub flagged: usize,
    /// The purchase requests cancelled.
    pub requests_cancelled: usize,
    /// The open purchase orders procurement was told about.
    pub open_orders: usize,
    /// The stock counts opened.
    pub counts_opened: usize,
    /// The follow-up tasks opened for the guarantees.
    pub guarantee_tasks: usize,
    /// The snag and defect tickets cancelled.
    pub tickets_cancelled: usize,
}

/// The notification type of the e-mail to the client contacts.
pub fn notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NOTICE, "construction")
        .group("projects")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("construction.terminate.subject")),
            template("construction.terminate.body"),
        )
}

/// Registers the notification type.
pub(crate) fn register_types() {
    notify::register_type(notice_type());
}

/// The guidance text of the follow-up task of a guarantee of `kind`.
fn guarantee_guidance(kind: GuaranteeKind) -> String {
    english(
        &format!("construction.terminate.guarantee_{}", kind.as_str()),
        &[],
    )
}

/// Runs what follows the commit of the termination of project `id` (see the
/// module). Idempotent: a project whose wind-down ran is left alone.
///
/// # Errors
///
/// The database's errors, and those of the contracts it calls.
pub async fn after_termination(state: &AppState, id: i64) -> Result<WoundDown> {
    let project = Project::find_or_404(&state.db, id).await?;
    let (Some(effective_on), Some(reason)) = (
        project.termination_effective_on,
        project
            .termination_reason
            .as_deref()
            .and_then(TerminationReason::parse),
    ) else {
        return Ok(WoundDown::default());
    };
    if project.status != ProjectStatus::Terminated {
        return Ok(WoundDown::default());
    }
    // Claim the wind-down: a second run finds nothing to do.
    let now = Some(renox::db::now());
    let claimed = Project::query()
        .where_(Project::ID.eq(id))
        .where_(Project::WOUND_DOWN_AT.is_null())
        .update(&state.db, &[("wound_down_at", &now)])
        .await?;
    if claimed == 0 {
        return Ok(WoundDown::default());
    }
    let subject = project.subject();
    let mut done = WoundDown {
        ran: true,
        ..WoundDown::default()
    };

    // Attendance after the last working day is kept and flagged.
    let note = english(
        "construction.terminate.attendance_note",
        &[("date", &effective_on)],
    );
    done.flagged = attendance_review(state)
        .flag_after(state, &subject, effective_on, &note)
        .await?
        .len();

    // The project manager reassigns the crews.
    let name = project.name.clone();
    tasks::open(
        state,
        &project,
        Task {
            subject: subject.clone(),
            role: permissions::PROJECT_MANAGER_ROLE,
            assignee: project.manager_user_id,
            source_key: format!("construction.terminate_reassign:{id}"),
            title: english(
                "construction.terminate.reassign_title",
                &[("project", &name)],
            ),
            description: english(
                "construction.terminate.reassign_body",
                &[("date", &effective_on)],
            ),
            due: Some(effective_on),
        },
    )
    .await?;

    // Purchasing and the site store.
    let stock = scope_stock(state);
    let requests_reason = english("construction.terminate.requests_reason", &[]);
    done.requests_cancelled = stock
        .cancel_open_requests(state, &subject, &requests_reason)
        .await?
        .len();
    let orders = stock.open_orders(state, &subject).await?;
    done.open_orders = orders.len();
    if !orders.is_empty() {
        let listed: Vec<String> = orders
            .iter()
            .map(|order| {
                order
                    .number
                    .clone()
                    .unwrap_or_else(|| order.order.to_string())
            })
            .collect();
        tasks::open(
            state,
            &project,
            Task {
                subject: subject.clone(),
                role: permissions::PROCUREMENT_ROLE,
                assignee: None,
                source_key: format!("construction.terminate_orders:{id}"),
                title: english("construction.terminate.orders_title", &[("project", &name)]),
                description: english(
                    "construction.terminate.orders_body",
                    &[("orders", &listed.join(", "))],
                ),
                due: None,
            },
        )
        .await?;
    }
    let count_reason = english("construction.terminate.count_reason", &[]);
    done.counts_opened = stock
        .open_stock_counts(state, &subject, &count_reason)
        .await?
        .len();

    // The accountant follows each active guarantee up.
    for held in guarantee::active_of(&state.db, id).await? {
        let kind = held.kind.as_str().replace('_', " ");
        let opened = tasks::open(
            state,
            &project,
            Task {
                subject: held.subject(),
                role: permissions::ACCOUNTANT_ROLE,
                assignee: None,
                source_key: format!("construction.terminate_guarantee:{}", held.id),
                title: english(
                    "construction.terminate.guarantee_title",
                    &[("kind", &kind), ("project", &name)],
                ),
                description: guarantee_guidance(held.kind),
                due: None,
            },
        )
        .await?;
        if opened.is_some() {
            done.guarantee_tasks += 1;
        }
    }

    // The parts made due at the termination are announced to the accountant.
    crate::release::announce_due(state, id).await?;

    // The open snags and defects are cancelled with the reason.
    let creator = task_creator(state);
    let tickets_reason = english("construction.terminate.tickets_reason", &[]);
    for category in SNAG_CATEGORIES {
        for ticket in creator.open_of(state, &subject, category).await? {
            if creator
                .cancel_from_source(state, ticket.id, &tickets_reason)
                .await?
            {
                done.tickets_cancelled += 1;
            }
        }
    }

    // The client contacts are told.
    let told = NotifyRequest::new(NOTICE, subject.clone())
        .subject_label(&project.code)
        .data(renox::serde_json::json!({
            "project": project.name,
            "date": effective_on.to_string(),
        }))
        .to_contact(
            project.client(),
            ContactPurpose {
                relation: ContactRelation::ContactOf,
                position: None,
            },
        );
    notify::notifier(state).notify(state, told).await?;

    state
        .emit(ProjectTerminated {
            project: subject,
            reason: reason.as_str().to_owned(),
            effective_on,
            requested_by: project.termination_requested_by,
        })
        .await?;
    Ok(done)
}

/// Tells the attendance module the last working day of a terminated project:
/// attendance is accepted up to and including the effective date and refused
/// from the day after.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProjectClosure;

impl ContextClosure for ProjectClosure {
    fn last_working_day<'a>(
        &'a self,
        state: &'a AppState,
        context: &'a Subject,
    ) -> LabourFuture<'a, Option<NaiveDate>> {
        Box::pin(async move {
            if context.subject_type != SUBJECT_TYPE {
                return Ok(None);
            }
            let project = Project::find(&state.db, context.subject_id).await?;
            Ok(project
                .filter(|project| project.status == ProjectStatus::Terminated)
                .and_then(|project| project.termination_effective_on))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> TerminateInput {
        TerminateInput {
            reason: "main_contractor_insolvent".into(),
            notice: "NOTICE-17.pdf".into(),
            effective_on: NaiveDate::from_ymd_opt(2027, 5, 31),
            requested_by: Some(3),
        }
    }

    #[test]
    fn a_reason_is_one_of_the_four() {
        for reason in [
            TerminationReason::TerminatedByClient,
            TerminationReason::OurDefault,
            TerminationReason::MainContractorInsolvent,
            TerminationReason::Agreed,
        ] {
            assert_eq!(TerminationReason::parse(reason.as_str()), Some(reason));
        }
        assert_eq!(TerminationReason::parse("bored"), None);
    }

    #[test]
    fn the_reason_the_notice_and_the_date_are_needed() {
        assert_eq!(terminate_problem(&input()), None);
        let mut wrong = input();
        wrong.reason = "bored".into();
        assert!(terminate_problem(&wrong).is_some_and(|p| p.contains("Say why")));
        let mut wrong = input();
        wrong.notice = "  ".into();
        assert!(terminate_problem(&wrong).is_some_and(|p| p.contains("notice")));
        let mut wrong = input();
        wrong.effective_on = None;
        assert!(terminate_problem(&wrong).is_some_and(|p| p.contains("effective date")));
    }

    #[test]
    fn the_input_round_trips_as_the_approval_payload() {
        let json = renox::serde_json::to_value(input()).unwrap();
        assert_eq!(json["effective_on"], "2027-05-31");
        assert_eq!(
            renox::serde_json::from_value::<TerminateInput>(json).unwrap(),
            input()
        );
    }
}
