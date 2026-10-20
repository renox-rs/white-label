//! The return services: one function per action, each gathering what the
//! machine's guards need and running the transition (04 §7.1 *Return*;
//! BL-ACC-16, BL-ACC-17). What reaches other modules after the commit
//! (notices, the preparer's task, the linked deadline task) is in
//! [`crate::return_effects`].
//!
//! A client approver is a portal user acting for the return's client with
//! the grant role `owner` or `approver` (`ACC.ACL.11`), through the kernel's
//! [`PortalUsers`](wl_kernel::portal::PortalUsers); once the engagement has
//! ended the portal is read-only, so nobody approves, sends back or
//! withdraws any more (`POR.ACL.01`).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::deadlines::{DeadlineTaskId, deadline_tasks};
use wl_kernel::documents::today;
use wl_kernel::portal::portal_users;

use crate::cdd::CddFile;
use crate::db::{refused, set_return};
use crate::deadline_notices::APPROVER_ROLES;
use crate::engagement::{Engagement, EngagementStatus};
use crate::permissions;
use crate::reasons;
use crate::return_machine::{
    APPROVE, CANCEL, CORRECT, FILE, REVIEW, ReturnInput, SEND_BACK, SUBMIT, WITHDRAW_APPROVAL,
};
use crate::tax_return::{CorrectionRoute, Return, ReturnType, document_hash};
use crate::texts::english;

/// What a new return is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NewReturn {
    /// The engagement it is prepared under.
    pub engagement: i64,
    /// VAT or corporate tax.
    pub return_type: ReturnType,
    /// The first day of the period.
    pub period_start: Option<NaiveDate>,
    /// The last day of the period.
    pub period_end: NaiveDate,
    /// The day it is due to the FTA.
    pub due_date: NaiveDate,
    /// The calendar task that tracks it; the open task of the engagement of
    /// the same type and due date when empty.
    pub task: Option<DeadlineTaskId>,
}

impl NewReturn {
    /// A return of `return_type` for the engagement `engagement`, for the
    /// period ending `period_end` and due on `due_date`.
    pub fn new(
        engagement: i64,
        return_type: ReturnType,
        period_end: NaiveDate,
        due_date: NaiveDate,
    ) -> Self {
        NewReturn {
            engagement,
            return_type,
            period_start: None,
            period_end,
            due_date,
            task: None,
        }
    }

    /// Tracked by the calendar task `task`.
    pub fn task(mut self, task: DeadlineTaskId) -> Self {
        self.task = Some(task);
        self
    }

    /// Covering the period from `start`.
    pub fn period_start(mut self, start: NaiveDate) -> Self {
        self.period_start = Some(start);
        self
    }
}

/// The return `id`, or 404.
async fn load(state: &AppState, id: i64) -> Result<Return> {
    Return::find_or_404(&state.db, id).await
}

/// `Forbidden` unless `user` holds `permission`.
fn require(user: &AuthUser, permission: &str) -> Result {
    if user.has_permission(permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// Opens a `preparing` return under engagement `new.engagement`, linked to
/// the calendar task that tracks it (the open task of the engagement with
/// the return's task type and due date, when none is named).
///
/// # Errors
///
/// A 422 unless the engagement is `active` or `suspended`; the database's
/// and the task service's errors.
pub async fn open(state: &AppState, by: Option<i64>, new: NewReturn) -> Result<Return> {
    let engagement = Engagement::find_or_404(&state.db, new.engagement).await?;
    if !engagement.accepts_time_entries() {
        return Err(refused(english(
            "accounting.returns.problem.engagement",
            &[],
        )));
    }
    let task = match new.task {
        Some(task) => Some(task),
        None => deadline_tasks(state)
            .of_subject(&state.db, &engagement.subject())
            .await?
            .into_iter()
            .find(|task| {
                task.task_type == new.return_type.task_type()
                    && task.due_date == new.due_date
                    && !task.status.is_terminal()
            })
            .map(|task| task.id),
    };
    Return::create(
        &state.db,
        Return {
            client_party_id: engagement.client_party_id,
            engagement_id: engagement.id,
            task_id: task.map(|t| t.0),
            return_type: new.return_type,
            period_start: new.period_start,
            period_end: new.period_end,
            due_date: new.due_date,
            version: 1,
            created_by: by,
            ..Return::default()
        },
    )
    .await
}

/// `by` records the figures of return `id` and whether its workpapers are
/// attached; only while it is `preparing`.
///
/// Decision: the workpapers themselves are documents of wl-documents; this
/// records that they are attached.
///
/// # Errors
///
/// `Forbidden` without `returns.prepare`; a 422 unless `preparing`.
pub async fn record_figures(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    figures: &str,
    workpapers_attached: bool,
) -> Result<Return> {
    require(by, permissions::RETURNS_PREPARE)?;
    let ret = load(state, id).await?;
    if ret.status != crate::tax_return::ReturnStatus::Preparing {
        return Err(refused(english(
            "accounting.returns.problem.not_preparing",
            &[],
        )));
    }
    let figures = figures.trim();
    let mut tx = state.db.begin().await?;
    set_return(
        &mut tx,
        id,
        &[
            (
                "figures",
                &(!figures.is_empty()).then(|| figures.to_owned()),
            ),
            ("workpapers_attached", &workpapers_attached),
        ],
    )
    .await?;
    tx.commit().await?;
    load(state, id).await
}

/// Whether `user` is a client approver of `ret`'s client today, with the
/// engagement still running (`ACC.ACL.11`, `POR.ACL.01`).
///
/// # Errors
///
/// The portal's and the database's errors.
pub async fn is_client_approver(state: &AppState, user: &AuthUser, ret: &Return) -> Result<bool> {
    let acting = portal_users(state)
        .acting_for(&state.db, user.id, ret.client(), today())
        .await?;
    let Some(acting) = acting else {
        return Ok(false);
    };
    if !acting.has_any_role(&APPROVER_ROLES) {
        return Ok(false);
    }
    let engagement = Engagement::find_or_404(&state.db, ret.engagement_id).await?;
    Ok(engagement.status != EngagementStatus::Ended)
}

/// What a user may do with `ret`, gathered for the guards.
async fn who(state: &AppState, user: &AuthUser, ret: &Return) -> Result<ReturnInput> {
    Ok(ReturnInput {
        on: Some(today()),
        reviewer: user.has_permission(permissions::RETURNS_REVIEW),
        client_approver: is_client_approver(state, user, ret).await?,
        ..ReturnInput::default()
    })
}

/// Runs transition `name` on return `id` as `by`.
async fn run(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    name: &str,
    reason: Option<&str>,
    input: ReturnInput,
) -> Result<Return> {
    let ret = load(state, id).await?;
    let mut run = ret.transition(name).by(by).input(&input);
    if let Some(reason) = reason {
        run = run.reason(reason.trim());
    }
    let _outcome = run.run(state).await.map_err(TransitionError::into_error)?;
    load(state, id).await
}

/// `by` submits return `id` for review: the figures are entered and the
/// workpapers attached; the reviewer is told after the commit.
///
/// # Errors
///
/// As the transition: `Forbidden` without `returns.prepare`; a 422 listing
/// what is missing.
pub async fn submit(state: &AppState, by: &AuthUser, id: i64) -> Result<Return> {
    let ret = load(state, id).await?;
    let input = ReturnInput {
        figures_entered: ret.figures.as_deref().is_some_and(|f| !f.trim().is_empty()),
        workpapers_attached: ret.workpapers_attached,
        ..ReturnInput::on(today())
    };
    run(state, by, id, SUBMIT, None, input).await
}

/// `by` (the reviewer, or a client approver) sends return `id` back to the
/// preparer with `comment`.
///
/// # Errors
///
/// A 422 without a comment or unless `by` is a reviewer or client approver.
pub async fn send_back(state: &AppState, by: &AuthUser, id: i64, comment: &str) -> Result<Return> {
    let ret = load(state, id).await?;
    let input = who(state, by, &ret).await?;
    run(state, by, id, SEND_BACK, Some(comment), input).await
}

/// `by` (a reviewer other than the preparer) sends return `id` to the
/// client approver.
///
/// # Errors
///
/// `Forbidden` without `returns.review`; a 422 for the preparer
/// (`ACC.ACL.03`).
pub async fn review(state: &AppState, by: &AuthUser, id: i64) -> Result<Return> {
    run(state, by, id, REVIEW, None, ReturnInput::on(today())).await
}

/// The client approver `by` approves return `id` from `ip`; the signature is
/// recorded (`DOC.SIG.01`).
///
/// # Errors
///
/// A 422 unless `by` is a client approver of the return's client.
pub async fn approve(state: &AppState, by: &AuthUser, id: i64, ip: Option<&str>) -> Result<Return> {
    let ret = load(state, id).await?;
    let input = ReturnInput {
        signer_name: Some(by.name.clone()),
        ip_address: ip.map(str::to_owned),
        document_hash: Some(document_hash(&summary(&ret))),
        ..who(state, by, &ret).await?
    };
    run(state, by, id, APPROVE, None, input).await
}

/// The summary of `ret` the client approves, and the signature's hash is of.
///
/// Decision: the summary PDF of `review` comes with wl-documents' templates;
/// until then the summary is this text.
pub fn summary(ret: &Return) -> String {
    format!(
        "{}|{}|{}|{}|v{}|{}",
        ret.number(),
        ret.return_type.as_str(),
        ret.period_end,
        ret.due_date,
        ret.version,
        ret.figures.as_deref().unwrap_or_default()
    )
}

/// `by` withdraws the approval of return `id` with `reason`: the client
/// approver on their own, or a reviewer who attaches the client's written
/// `instruction`. The signature is kept and marked withdrawn; the return is
/// `preparing` again.
///
/// # Errors
///
/// A 422 without a reason, or from a reviewer without the instruction, or
/// from anybody else, or unless `approved`.
pub async fn withdraw_approval(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    instruction: Option<&str>,
) -> Result<Return> {
    let ret = load(state, id).await?;
    let input = ReturnInput {
        instruction: instruction.map(str::to_owned),
        ..who(state, by, &ret).await?
    };
    run(state, by, id, WITHDRAW_APPROVAL, Some(reason), input).await
}

/// `by` marks return `id` filed on EmaraTax with the FTA `acknowledgement`.
/// Basic form: BL-ACC-18 adds the tax agent rule and the block check.
///
/// # Errors
///
/// `Forbidden` without `returns.file`; a 422 without an acknowledgement.
pub async fn file(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    acknowledgement: &str,
    filed_by_client: bool,
    tax_agent_registration: Option<&str>,
) -> Result<Return> {
    let input = ReturnInput {
        acknowledgement: Some(acknowledgement.to_owned()),
        filed_by_client,
        tax_agent_registration: tax_agent_registration.map(str::to_owned),
        ..ReturnInput::on(today())
    };
    run(state, by, id, FILE, None, input).await
}

/// `by` corrects the filed return `id` with `reason`. Basic form:
/// BL-ACC-19 adds the route rules and the new version.
///
/// # Errors
///
/// `Forbidden` without `returns.prepare`; a 422 without a reason.
pub async fn correct(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    route: Option<CorrectionRoute>,
) -> Result<Return> {
    let input = ReturnInput {
        route,
        ..ReturnInput::on(today())
    };
    run(state, by, id, CORRECT, Some(reason), input).await
}

/// `by` (a manager) cancels return `id` with `reason`, one of
/// [`reasons::RETURN_CANCEL_REASONS`]. After the commit the linked task is
/// waived, handed over or listed for the manager, and the client is told who
/// files it ([`crate::return_effects`]); nothing is sent for an engagement
/// whose client was exited through CDD (`ACC.ACL.14`).
///
/// # Errors
///
/// `Forbidden` without `returns.review`; a 422 without a listed reason, or
/// for a return that is no longer open.
pub async fn cancel(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<Return> {
    let ret = load(state, id).await?;
    let restricted = CddFile::current(&state.db, ret.client())
        .await?
        .is_some_and(|file| file.ended_relationship());
    let input = ReturnInput {
        restricted,
        ..ReturnInput::on(today())
    };
    run(state, by, id, CANCEL, Some(reason), input).await
}

/// The reasons a user may give for `cancel` (for the forms).
pub fn cancel_reasons() -> &'static [&'static str] {
    &reasons::RETURN_CANCEL_REASONS
}

/// The returns of `engagement` that are still open (`preparing`,
/// `in_review`, `client_approval`, `approved`).
///
/// # Errors
///
/// The database's errors.
pub async fn open_of(db: &Db, engagement: i64) -> Result<Vec<Return>> {
    Ok(Return::of_engagement(db, engagement)
        .await?
        .into_iter()
        .filter(|ret| ret.status.is_open())
        .collect())
}
