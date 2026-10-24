//! Terminating a client contract early (09 Manpower §3.3 `MPW.ACL.17`, §4.1 row
//! *The contract expires, or ends early*, §7.1 *Client contract* `terminate`;
//! BL-MPW-04).
//!
//! Either side may end a client contract before its end date. The account
//! manager (`contracts.manage`) requests it with who started it (the client or
//! the agency), the reason, the notice date and the new end date; the kernel
//! defers the run as an approval request for the MD (`MPW.ACL.17`, one step,
//! never the requester, [`terminate_policy`]) and, at the final approval, runs
//! `terminate_approved`, which has the same guard and effect (the pattern of
//! 02 Construction's `terminate`). A contract `active` or `on_hold` can be
//! terminated; the contract stays as it is, and nothing is cut, until the MD
//! approves.
//!
//! - **The guard** ([`terminate_problem`]) lists every failing check: who
//!   started it is `client` or `agency`, a reason, a notice date and an end
//!   date are given; the end date is not later than the current one (a later
//!   one is `renew`'s) and not before the notice date; and it is at least the
//!   notice date plus the contract's notice period in calendar days, unless a
//!   waiver is attached. It runs again at the approval, against the contract
//!   as it is then.
//! - **The effect**, in the transaction of the approval ([`terminate_effects`]):
//!   the contract is `terminating` with the new end date (the old one kept);
//!   a hold is cleared (the state before is dropped, the reasons closed as
//!   *terminated*, and `MPW.RUL.10` may hold the contract again); `active` and
//!   `suspended` deployments whose end date is later or empty end on the new
//!   end date (an earlier one is kept); `planned` deployments that start after
//!   it are cancelled with the reason *contract terminated* (`auto_cancel`),
//!   the others are capped at it; the requests still `draft`, `open` or
//!   `partially_filled` that start after it are cancelled (`auto_cancel`, with
//!   their planned deployments), the others stay for the contract's `end`
//!   (BL-MPW-06).
//! - **After the commit** ([`after_termination`], once per contract): the
//!   deployments cancelled are announced (`DeploymentCancelled`), a pending
//!   request for a release of the hold is withdrawn, the account manager is
//!   told and the client is sent the confirmation. **No deployment ends now**:
//!   they end on the end date, through the daily run and `end` (BL-MPW-06).
//!
//! Decisions where the spec is silent:
//!
//! - The confirmation to the client (`DOC.GEN.01`, the letter) goes out as a
//!   message to the client company with the terms, as the other letters of the
//!   verticals do until the document generator exists.
//! - An `end` or a `renew` of the contract while a request waits withdraws it
//!   with [`withdraw_pending`], which `renew` (BL-MPW-05) and `end` (BL-MPW-06) call.
//! - The new end date is the contract's end date from the approval; the date
//!   it had is kept in `original_end_date`.

use renox::chrono::{Duration, NaiveDate};
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionCtx, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{ApprovalRequest, PolicyDefault, announce, withdraw_because};

use crate::contract::{ClientContract, ContractStatus, TERMINATE};
use crate::db::{answer, refused, set_contract, set_deployment};
use crate::deployment::{AUTO_CANCEL, Deployment, DeploymentStatus};
use crate::deployments::announce_all_cancelled;
use crate::notices;
use crate::permissions;
use crate::release::close_open;
use crate::request::{self, ManpowerRequest, RequestStatus};
use crate::texts::{english, refusal};

/// The approval action of `terminate` (`MPW.ACL.17`).
pub const ACTION: &str = "manpower.terminate";

/// Who started the termination.
pub mod initiators {
    /// The client ended the contract.
    pub const CLIENT: &str = "client";
    /// The agency ended the contract.
    pub const AGENCY: &str = "agency";
    /// Every initiator.
    pub const ALL: &[&str] = &[CLIENT, AGENCY];
}

/// The default approval policy of `terminate` (`MPW.ACL.17`): every request
/// needs one step by the MD (the company admin when nobody holds the role),
/// never the requester.
pub fn terminate_policy() -> PolicyDefault {
    PolicyDefault::new(ACTION).step([permissions::MD_ROLE, wl_kernel::COMPANY_ADMIN_ROLE])
}

/// What a `terminate` run carries to its guard and effect; it is also the
/// approval request's payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminateInput {
    /// Who started it: one of [`initiators`].
    #[serde(default)]
    pub initiated_by: String,
    /// Why.
    #[serde(default)]
    pub reason: String,
    /// The day the notice was given.
    #[serde(default)]
    pub notice_on: Option<NaiveDate>,
    /// The new end date of the contract.
    #[serde(default)]
    pub end_date: Option<NaiveDate>,
    /// The waiver of a short notice, a document reference.
    #[serde(default)]
    pub waiver: Option<String>,
    /// The user who asked.
    #[serde(default)]
    pub requested_by: Option<i64>,
}

/// The input of a `terminate` run.
pub(crate) fn terminate_input(ctx: &TransitionCtx) -> TerminateInput {
    ctx.input_as::<TerminateInput>().unwrap_or_default()
}

/// Every reason `contract` cannot be terminated with `input`, one English line
/// each; `None` when it can.
pub fn terminate_problem(contract: &ClientContract, input: &TerminateInput) -> Option<String> {
    let mut problems = Vec::new();
    if !initiators::ALL.contains(&input.initiated_by.trim()) {
        problems.push(refusal("manpower.terminate.initiated_by", &[]));
    }
    if input.reason.trim().is_empty() {
        problems.push(refusal("manpower.terminate.reason", &[]));
    }
    match (input.notice_on, input.end_date) {
        (Some(notice), Some(end)) => {
            if end > contract.end_date {
                problems.push(refusal("manpower.terminate.later", &[]));
            }
            if end < notice {
                problems.push(refusal("manpower.terminate.before_notice", &[]));
            }
            let waived = input
                .waiver
                .as_deref()
                .is_some_and(|waiver| !waiver.trim().is_empty());
            let earliest = notice + Duration::days(contract.notice_period_days);
            if end < earliest && !waived {
                problems.push(refusal(
                    "manpower.terminate.short_notice",
                    &[
                        ("days", &contract.notice_period_days),
                        ("earliest", &earliest),
                    ],
                ));
            }
        }
        _ => problems.push(refusal("manpower.terminate.dates", &[])),
    }
    (!problems.is_empty()).then(|| problems.join("\n"))
}

/// The effect of `terminate` and `terminate_approved` inside their transaction
/// (see the module).
pub(crate) async fn terminate_effects(
    tx: &mut Transaction,
    contract: &ClientContract,
    input: &TerminateInput,
) -> Result {
    let (Some(end), Some(notice)) = (input.end_date, input.notice_on) else {
        return Err(refused(refusal("manpower.terminate.dates", &[])));
    };
    let waiver = input
        .waiver
        .as_deref()
        .map(str::trim)
        .filter(|waiver| !waiver.is_empty())
        .map(str::to_owned);
    set_contract(
        &mut *tx,
        contract.id,
        &[
            ("end_date", &end),
            ("original_end_date", &Some(contract.end_date)),
            (
                "termination_initiated_by",
                &Some(input.initiated_by.trim().to_owned()),
            ),
            ("termination_reason", &Some(input.reason.trim().to_owned())),
            ("termination_notice_on", &Some(notice)),
            ("termination_waiver", &waiver),
            ("termination_requested_by", &input.requested_by),
            ("terminated_at", &Some(now())),
        ],
    )
    .await?;
    if contract.status == ContractStatus::OnHold {
        let none_text: Option<String> = None;
        let none_at: Option<DateTime> = None;
        let none_user: Option<i64> = None;
        set_contract(
            &mut *tx,
            contract.id,
            &[
                ("status_before_hold", &none_text),
                ("held_at", &none_at),
                ("held_by", &none_user),
            ],
        )
        .await?;
        close_open(&mut *tx, contract.id, "terminated", false).await?;
    }
    let reason = english("manpower.terminate.cancel_reason", &[]);
    cut_deployments(&mut *tx, contract.id, end, &reason).await?;
    cancel_requests(&mut *tx, contract.id, end, &reason).await
}

/// Caps the deployments of contract `contract_id` at `end` and cancels the
/// planned ones that start after it.
async fn cut_deployments(
    tx: &mut Transaction,
    contract_id: i64,
    end: NaiveDate,
    reason: &str,
) -> Result {
    let deployments = Deployment::query()
        .where_(Deployment::CLIENT_CONTRACT_ID.eq(contract_id))
        .where_(Deployment::STATUS.is_in([
            DeploymentStatus::Planned,
            DeploymentStatus::Active,
            DeploymentStatus::Suspended,
        ]))
        .order_by(Deployment::ID)
        .get(&mut *tx)
        .await?;
    for deployment in deployments {
        if deployment.status == DeploymentStatus::Planned && deployment.start_date > end {
            let _outcome = deployment
                .transition(AUTO_CANCEL)
                .by_system()
                .reason(reason)
                .run_in(&mut *tx)
                .await
                .map_err(answer)?;
        } else if deployment.end_date.is_none_or(|current| current > end) {
            set_deployment(&mut *tx, deployment.id, &[("end_date", &Some(end))]).await?;
        }
    }
    Ok(())
}

/// Cancels the requests of contract `contract_id` that start after `end`, with
/// their planned deployments.
async fn cancel_requests(
    tx: &mut Transaction,
    contract_id: i64,
    end: NaiveDate,
    reason: &str,
) -> Result {
    let requests = ManpowerRequest::query()
        .where_(ManpowerRequest::CLIENT_CONTRACT_ID.eq(contract_id))
        .where_(ManpowerRequest::STATUS.is_in([
            RequestStatus::Draft,
            RequestStatus::Open,
            RequestStatus::PartiallyFilled,
        ]))
        .where_(ManpowerRequest::START_DATE.gt(end))
        .order_by(ManpowerRequest::ID)
        .get(&mut *tx)
        .await?;
    for request in requests {
        let _outcome = request
            .transition(request::AUTO_CANCEL)
            .by_system()
            .reason(reason)
            .run_in(&mut *tx)
            .await
            .map_err(answer)?;
        let _cancelled =
            crate::deployments::cancel_planned_of_request(&mut *tx, request.id, reason).await?;
    }
    Ok(())
}

/// What the account manager enters to request the termination.
#[derive(Debug, Clone)]
pub struct TerminateAsk {
    /// Who started it: one of [`initiators`].
    pub initiated_by: String,
    /// Why.
    pub reason: String,
    /// The day the notice was given.
    pub notice_on: NaiveDate,
    /// The new end date.
    pub end_date: NaiveDate,
    /// The waiver of a short notice, a document reference.
    pub waiver: Option<String>,
}

/// What a request for the termination did.
#[derive(Debug, Clone)]
pub struct Requested {
    /// The contract: unchanged while the MD decides.
    pub contract: ClientContract,
    /// The approval request, when the MD must decide.
    pub approval_request: Option<i64>,
}

/// `by` requests the termination of contract `id`. The run is deferred as an
/// approval request, announced to the MD; nothing changes until it is
/// approved.
///
/// # Errors
///
/// `Forbidden` without `contracts.manage` for the client; a 422 listing every
/// failing check, or unless the contract is `active` or `on_hold`.
pub async fn request(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    ask: TerminateAsk,
) -> Result<Requested> {
    let contract = ClientContract::find_or_404(&state.db, id).await?;
    let input = TerminateInput {
        initiated_by: ask.initiated_by.trim().to_owned(),
        reason: ask.reason.trim().to_owned(),
        notice_on: Some(ask.notice_on),
        end_date: Some(ask.end_date),
        waiver: ask.waiver.filter(|waiver| !waiver.trim().is_empty()),
        requested_by: Some(by.id),
    };
    let history = english(
        "manpower.terminate.history",
        &[("by", &input.initiated_by), ("reason", &input.reason)],
    );
    let outcome = contract
        .transition(TERMINATE)
        .by(by)
        .reason(history)
        .input(&input)
        .run(state)
        .await
        .map_err(answer)?;
    announce(state, &outcome).await?;
    let approval_request = match &outcome {
        renox::state_machine::Outcome::Pending(pending) => pending
            .data
            .get(wl_kernel::approvals::submit::REQUEST_KEY)
            .and_then(renox::serde_json::Value::as_i64),
        renox::state_machine::Outcome::Done(_) => None,
    };
    outcome.emit(state).await?;
    Ok(Requested {
        contract: ClientContract::find_or_404(&state.db, id).await?,
        approval_request,
    })
}

/// The pending request for the termination of `contract`, if any.
async fn pending_termination(state: &AppState, contract: &ClientContract) -> Result<Vec<i64>> {
    Ok(
        ApprovalRequest::pending_about(&state.db, &contract.subject())
            .await?
            .iter()
            .filter(|request| request.action == ACTION)
            .map(|request| request.id)
            .collect(),
    )
}

/// The contract `end`s or is `renew`ed while a request for its termination
/// waits: the request is withdrawn with `reason` and the account manager is
/// told. Returns how many were withdrawn. BL-MPW-05 (`renew`) and BL-MPW-06
/// (`end`) call it.
///
/// # Errors
///
/// The kernel's and the notifier's errors.
pub async fn withdraw_pending(
    state: &AppState,
    contract: &ClientContract,
    reason: &str,
) -> Result<usize> {
    let pending = pending_termination(state, contract).await?;
    for id in &pending {
        withdraw_because(state, *id, reason).await?;
    }
    if !pending.is_empty() {
        let managers =
            notices::account_managers(state, contract.client(), contract.account_manager_user_id)
                .await?;
        notices::tell(
            state,
            contract.subject(),
            &contract.reference,
            &managers,
            &english(
                "manpower.terminate.withdrawn_title",
                &[("contract", &contract.reference)],
            ),
            &english(
                "manpower.terminate.withdrawn_body",
                &[("contract", &contract.reference), ("reason", &reason)],
            ),
        )
        .await?;
    }
    Ok(pending.len())
}

/// What follows the commit of the termination of contract `id`, once: the
/// deployments cancelled are announced, a request to release the hold is
/// withdrawn, the account manager is told and the client is sent the
/// confirmation. Answers whether this run did it.
///
/// # Errors
///
/// The database's, the kernel's and the notifier's errors.
pub async fn after_termination(state: &AppState, id: i64) -> Result<bool> {
    let contract = ClientContract::find_or_404(&state.db, id).await?;
    if contract.status != ContractStatus::Terminating {
        return Ok(false);
    }
    // Claim the announcement: a second run finds nothing to do.
    let claimed = ClientContract::query()
        .where_(ClientContract::ID.eq(id))
        .where_(ClientContract::TERMINATION_ANNOUNCED_AT.is_null())
        .update(&state.db, &[("termination_announced_at", &Some(now()))])
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    let reason = english("manpower.terminate.cancel_reason", &[]);
    let cancelled = Deployment::query()
        .where_(Deployment::CLIENT_CONTRACT_ID.eq(id))
        .where_(Deployment::STATUS.eq(DeploymentStatus::Cancelled))
        .where_(Deployment::CANCEL_REASON.eq(reason.clone()))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?;
    announce_all_cancelled(state, &cancelled, &reason).await?;
    // The hold is gone: a request to release it has nothing left to ask.
    let stale = crate::release::pending_release_ids(state, &contract).await?;
    for request in stale {
        withdraw_because(state, request, &reason).await?;
    }
    let end = contract.end_date.to_string();
    let initiator = contract
        .termination_initiated_by
        .clone()
        .unwrap_or_default();
    let notice = contract
        .termination_notice_on
        .map_or_else(|| "-".to_owned(), |day| day.to_string());
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 5] = [
        ("contract", &contract.reference),
        ("date", &end),
        ("by", &initiator),
        ("notice", &notice),
        (
            "reason",
            &contract.termination_reason.clone().unwrap_or_default(),
        ),
    ];
    let managers =
        notices::account_managers(state, contract.client(), contract.account_manager_user_id)
            .await?;
    notices::tell(
        state,
        contract.subject(),
        &contract.reference,
        &managers,
        &english("manpower.terminate.approved_title", &params),
        &english("manpower.terminate.approved_body", &params),
    )
    .await?;
    notices::tell_client_as(
        state,
        notices::STATUTORY_NOTICE,
        contract.subject(),
        &contract.reference,
        contract.client(),
        &english("manpower.terminate.letter_title", &params),
        &english("manpower.terminate.letter_body", &params),
    )
    .await?;
    Ok(true)
}

/// A termination run, direct or approved, tells the people once it committed.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(
        |event: Transitioned<ClientContract>, state: AppState| async move {
            let terminated = [TERMINATE, wl_kernel::machines::approved(TERMINATE).as_str()]
                .contains(&event.transition.as_str());
            if terminated && event.to == ContractStatus::Terminating.as_str() {
                after_termination(&state, event.key).await?;
            }
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    fn contract() -> ClientContract {
        ClientContract {
            end_date: NaiveDate::from_ymd_opt(2027, 6, 30).unwrap(),
            notice_period_days: 30,
            ..ClientContract::default()
        }
    }

    fn input(end: NaiveDate) -> TerminateInput {
        TerminateInput {
            initiated_by: "client".into(),
            reason: "budget cut".into(),
            notice_on: Some(day(11, 1)),
            end_date: Some(end),
            waiver: None,
            requested_by: Some(3),
        }
    }

    #[test]
    fn the_notice_period_counts_in_calendar_days_from_the_notice() {
        assert_eq!(terminate_problem(&contract(), &input(day(12, 1))), None);
        assert!(terminate_problem(&contract(), &input(day(11, 30))).is_some());
        assert!(
            terminate_problem(&contract(), &input(day(11, 15)))
                .is_some_and(|p| p.contains("30 days"))
        );
    }

    #[test]
    fn a_waiver_allows_a_short_notice_but_not_a_later_end() {
        let mut short = input(day(11, 15));
        short.waiver = Some("WAIVER.pdf".into());
        assert_eq!(terminate_problem(&contract(), &short), None);
        let mut blank = input(day(11, 15));
        blank.waiver = Some("  ".into());
        assert!(terminate_problem(&contract(), &blank).is_some());
        let mut later = input(NaiveDate::from_ymd_opt(2027, 7, 1).unwrap());
        later.waiver = Some("WAIVER.pdf".into());
        assert!(terminate_problem(&contract(), &later).is_some_and(|p| p.contains("renew")));
    }

    #[test]
    fn who_why_and_the_dates_are_needed() {
        let mut wrong = input(day(12, 1));
        wrong.initiated_by = "bored".into();
        wrong.reason = " ".into();
        let problem = terminate_problem(&contract(), &wrong).unwrap();
        assert_eq!(problem.lines().count(), 2);
        let mut undated = input(day(12, 1));
        undated.end_date = None;
        assert!(terminate_problem(&contract(), &undated).is_some());
    }

    #[test]
    fn the_input_round_trips_as_the_approval_payload() {
        let json = renox::serde_json::to_value(input(day(12, 1))).unwrap();
        assert_eq!(json["end_date"], "2026-12-01");
        assert_eq!(
            renox::serde_json::from_value::<TerminateInput>(json).unwrap(),
            input(day(12, 1))
        );
    }
}
