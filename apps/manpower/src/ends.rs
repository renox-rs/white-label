//! Ending a deployment for a reason that needs more than a date (09 Manpower
//! §4.1 rows *The client stops paying* and *The client rejects a deployed
//! worker*, §7.1 *Deployment* `end` and the paragraph after the diagram;
//! BL-MPW-14).
//!
//! The user's reasons are *end date*, *replaced* ([`crate::deployments::end`]
//! passes no details) and three that ask for more ([`EndDetails`]):
//!
//! - **Client rejected.** The client's note is required. The deployment ends
//!   today (the last day), the worker goes back to the bench, and when the
//!   contract requires cover a replacement request opens at once: one worker
//!   with the role, site and shift, from tomorrow to the end date of the
//!   deployment ([`crate::cover::open_cover`], cause `rejection`, one request
//!   per deployment). With *conduct involved* ticked a complaint ticket of the
//!   category `conduct` is opened through the kernel's `TaskCreator`
//!   (wl-ticketing), about the worker and requested by the client, assigned to
//!   the account manager. With the cover obligation off no request is made.
//!   The portal has no rejection action: staff record it.
//! - **Client non-payment.** Only the MD (the role `md`, else the company
//!   admin as everywhere the MD decides) holding `deployments.manage` may
//!   choose it; anybody else is refused with 403. The end date is the last day on site and must be at
//!   least the contract's notice period in calendar days after today unless
//!   the MD attaches a waiver. **Decision:** the worker stays until that day,
//!   so a future end date does not end the deployment now: it sets the end
//!   date and the reason it will end with (`planned_end_reason`), tells the
//!   client the date, and the daily run ends it on the day after with the
//!   reason *client non-payment* ([`crate::deployments::end_due`]). With the
//!   end date today (a waiver) it ends at once.
//! - **Entered in error.** The deployment ends and the attendance of the
//!   worker at the site and under the deployment, from its first day, is
//!   rejected through the kernel's `AttendanceReview` (wl-attendance), so the
//!   hours are not billed or paid; a record already accepted or in a locked
//!   timesheet cannot be rejected and the account manager is told how many
//!   need an adjustment timesheet (`HRM.ACL.08`).
//!
//! After every end the open timesheet is closed and the client told
//! ([`crate::deployments::finish_end`]).

use renox::chrono::{Days, NaiveDate};
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::Subject;
use wl_kernel::attendance::attendance_review;
use wl_kernel::tickets::{Assignee, OpenTicket, task_creator};
use wl_kernel::workforce::EMPLOYEE_SUBJECT_TYPE;

use crate::contract::ClientContract;
use crate::cover::{self, CoverAsk, causes};
use crate::db::{answer, set_deployment};
use crate::deployment::{Deployment, DeploymentInput, END, reasons};
use crate::deployments::{after_end, load, require_manage, tell_client_as, worker_name};
use crate::notices;
use crate::permissions;
use crate::site::{self, Site};
use crate::texts::english;
use wl_kernel::documents::today;

/// What a user gives besides the reason.
#[derive(Debug, Clone, Default)]
pub struct EndDetails {
    /// The client's note (required for *client rejected*).
    pub note: Option<String>,
    /// *Client rejected*: conduct is involved, so a complaint ticket opens.
    pub conduct: bool,
    /// *Client non-payment*: the last day on site; today when none.
    pub end_date: Option<NaiveDate>,
    /// *Client non-payment*: the waiver of a short notice, a document
    /// reference.
    pub waiver: Option<String>,
}

/// The text of `value` when it is not blank.
fn clean(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Whether `by` is the MD: the role, else the company admin.
fn is_md(by: &AuthUser) -> bool {
    by.has_role(permissions::MD_ROLE) || by.has_role(wl_kernel::COMPANY_ADMIN_ROLE)
}

/// Ends deployment `id` for `by` with `reason` and `details`: see the module.
///
/// # Errors
///
/// `Forbidden` without `deployments.manage` for the client, or for *client
/// non-payment* when `by` is not the MD; a 422 for a reason the user may not
/// give, a missing note, an end date that is past or inside the notice period
/// without a waiver, or unless the deployment is `active` or `suspended`.
pub async fn end(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    details: EndDetails,
    lang: &str,
) -> Result<Deployment> {
    let deployment = load(state, id).await?;
    let reason = reason.trim();
    require_manage(by, deployment.client())?;
    // *Client non-payment* is the MD's decision, on top of the permission.
    if reason == reasons::CLIENT_NON_PAYMENT && !is_md(by) {
        return Err(Error::Forbidden);
    }
    let on = today();
    let note = clean(&details.note);
    let waiver = clean(&details.waiver);
    let mut problems = Vec::new();
    match reason {
        reasons::END_DATE => {
            if !deployment.end_date.is_some_and(|end| end <= on) {
                problems.push(english("manpower.deployment.end_date_not_come", &[]));
            }
        }
        reasons::CLIENT_REJECTED => {
            if note.is_none() {
                problems.push(english("manpower.end.note_required", &[]));
            }
        }
        reasons::CLIENT_NON_PAYMENT => {
            let contract =
                ClientContract::find_or_404(&state.db, deployment.client_contract_id).await?;
            let last = details.end_date.unwrap_or(on);
            let earliest = on + Days::new(u64::try_from(contract.notice_period_days).unwrap_or(0));
            if last < on {
                problems.push(english("manpower.end.before_today", &[]));
            } else if last < earliest && waiver.is_none() {
                problems.push(english(
                    "manpower.end.short_notice",
                    &[
                        ("days", &contract.notice_period_days),
                        ("earliest", &earliest),
                    ],
                ));
            }
            if problems.is_empty() && last > on && deployment.is_on_site() {
                return withdraw_later(state, &deployment, last, note, waiver, lang).await;
            }
        }
        _ => {}
    }
    let input = DeploymentInput {
        problems,
        on: Some(on),
        note: note.clone(),
        waiver,
        ..DeploymentInput::default()
    };
    let _outcome = deployment
        .transition(END)
        .by(by)
        .reason(reason)
        .input(&input)
        .run(state)
        .await
        .map_err(answer)?;
    let ended = after_end(state, id, lang).await?;
    match reason {
        reasons::CLIENT_REJECTED => {
            rejected(state, &deployment, on, note.as_deref(), details.conduct).await?;
        }
        reasons::ENTERED_IN_ERROR => entered_in_error(state, &deployment).await?,
        _ => {}
    }
    Ok(ended)
}

/// *Client non-payment* with an end date to come: the worker stays until
/// `last`; the deployment ends then, with this reason.
async fn withdraw_later(
    state: &AppState,
    deployment: &Deployment,
    last: NaiveDate,
    note: Option<String>,
    waiver: Option<String>,
    lang: &str,
) -> Result<Deployment> {
    let when = deployment.end_date.map_or(last, |end| end.min(last));
    let mut tx: Transaction = state.db.begin().await?;
    set_deployment(
        &mut tx,
        deployment.id,
        &[
            ("end_date", &Some(when)),
            (
                "planned_end_reason",
                &Some(reasons::CLIENT_NON_PAYMENT.to_owned()),
            ),
            ("end_waiver", &waiver),
            ("end_note", &note),
        ],
    )
    .await?;
    tx.commit().await?;
    let scheduled = load(state, deployment.id).await?;
    let worker = worker_name(state, lang, scheduled.employee()).await;
    // The notice that workers are withdrawn for non-payment is statutory.
    tell_client_as(
        state,
        crate::notices::STATUTORY_NOTICE,
        &scheduled,
        "manpower.deployment.withdrawn",
        &worker,
    )
    .await?;
    Ok(scheduled)
}

/// What follows a rejection: the cover request when the contract requires
/// it, and the conduct ticket when ticked.
async fn rejected(
    state: &AppState,
    deployment: &Deployment,
    on: NaiveDate,
    note: Option<&str>,
    conduct: bool,
) -> Result {
    let tomorrow = on + Days::new(1);
    if deployment.end_date.is_none_or(|end| end >= tomorrow) {
        cover::open_cover(
            state,
            CoverAsk {
                deployment,
                from: tomorrow,
                to: deployment.end_date,
                cause: causes::REJECTION,
                cause_ref: Some(format!("{}:rejection", deployment.subject())),
            },
        )
        .await?;
    }
    if conduct {
        open_conduct_ticket(state, deployment, note.unwrap_or_default()).await?;
    }
    Ok(())
}

/// The complaint ticket of the category `conduct`, about the worker and
/// requested by the client, assigned to the account manager. One per
/// deployment; none when ticketing is off.
async fn open_conduct_ticket(state: &AppState, deployment: &Deployment, note: &str) -> Result {
    let key = format!("{}:conduct", deployment.subject());
    let creator = task_creator(state);
    if creator.find_open(state, &key).await?.is_some() {
        return Ok(());
    }
    let worker = worker_name(state, "en", deployment.employee()).await;
    let site = Site::find(&state.db, deployment.site_id)
        .await?
        .map(|site| site.name)
        .unwrap_or_default();
    let contract = ClientContract::find_or_404(&state.db, deployment.client_contract_id).await?;
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 3] =
        [("worker", &worker), ("site", &site), ("note", &note)];
    let mut ticket = OpenTicket::new(
        Subject::new(EMPLOYEE_SUBJECT_TYPE, deployment.employee_id),
        crate::seed::CONDUCT_CATEGORY,
        &english("manpower.end.conduct_title", &params),
    )
    .description(&english("manpower.end.conduct_body", &params))
    .requester_party(deployment.client())
    .source(deployment.subject())
    .source_key(&key);
    if let Some(manager) =
        notices::account_managers(state, contract.client(), contract.account_manager_user_id)
            .await?
            .first()
    {
        ticket = ticket.assign_to(Assignee::user(*manager));
    }
    creator.open(state, ticket).await?;
    Ok(())
}

/// *Entered in error*: the attendance of the deployment is rejected; the
/// account manager is told how many records need an adjustment.
async fn entered_in_error(state: &AppState, deployment: &Deployment) -> Result {
    let contexts = [
        Subject::new(site::SUBJECT_TYPE, deployment.site_id),
        deployment.subject(),
    ];
    let done = attendance_review(state)
        .reject_work(
            state,
            &contexts,
            deployment.employee(),
            deployment.start_date,
            &english("manpower.end.error_reason", &[]),
        )
        .await?;
    if done.kept.is_empty() {
        return Ok(());
    }
    let contract = ClientContract::find_or_404(&state.db, deployment.client_contract_id).await?;
    let worker = worker_name(state, "en", deployment.employee()).await;
    let count = done.kept.len();
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 2] =
        [("worker", &worker), ("count", &count)];
    let managers =
        notices::account_managers(state, contract.client(), contract.account_manager_user_id)
            .await?;
    notices::tell(
        state,
        deployment.subject(),
        &contract.reference,
        &managers,
        &english("manpower.end.adjust_title", &params),
        &english("manpower.end.adjust_body", &params),
    )
    .await?;
    Ok(())
}
