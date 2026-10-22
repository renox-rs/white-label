//! Ending a client contract with its deployments, requests and last
//! timesheets (09 Manpower §4.1 row *The contract expires, or ends early*,
//! §7.1 *Client contract* `end`, *Manpower request* `cancel`, *Deployment*
//! `end` and `cancel`; BL-MPW-06).
//!
//! On the end date, whether it came by expiry or after an early termination,
//! the contract `end`s and everything under it closes cleanly. The system does
//! it at the first daily run after the end date (00:05 Asia/Dubai,
//! [`daily`]): a contract `active`, `on_hold` or `terminating` whose end date
//! is before today. The end date is the last working day.
//!
//! - **The guard** is *not renewed*: the end date is before the day of the run.
//!   A contract renewed before the run has a later end date, so nothing ends;
//!   a second run finds the contract `ended` and does nothing.
//! - **The effect**, in the transaction of the `end` ([`end_effects`]): the
//!   `active` and `suspended` deployments `end` with the reason *contract end*
//!   and the contract's end date as their last day (`auto_end`; suspended
//!   days stay unbilled); the `planned` ones are cancelled (`auto_cancel`);
//!   the requests still `draft`, `open` or `partially_filled` are cancelled
//!   (`auto_cancel`) with their planned deployments; a credit hold is cleared
//!   and its reasons closed as *contract end* (overdue invoices stay in
//!   receivables: nothing here touches an invoice).
//! - **After the commit** ([`after_end`], once per contract, claimed by
//!   `end_announced_at` so a run that was cut short is finished by the next
//!   one): for each deployment ended the open timesheet of the worker is
//!   closed at the end date and submitted at once, through the kernel's
//!   `TimesheetClosing` (wl-attendance), `DeploymentEnded` is emitted and the
//!   client is told per deployment; the deployments cancelled are announced;
//!   [`ContractEnded`] is emitted; the account manager gets one message with
//!   the workers back on the bench ([`crate::bench`]: `active` in HR and not
//!   on another site) and the timesheets closed.
//! - **Billing.** Nothing here stops the billing of the hours: the closed
//!   timesheet is submitted to the client like any other and is billed after
//!   its approval though the contract is `ended`: the approval records the
//!   hours as usage and the next `MPW.RUL.05` run drafts the invoice
//!   ([`crate::billing`]; an `ended` contract is billed on every run).
//! - A request to terminate the contract that still waits for the MD is
//!   withdrawn ([`crate::termination::withdraw_pending`]).

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::documents::today;
use wl_kernel::workforce::EmployeeId;

use crate::bench;
use crate::contract::{ClientContract, ContractInput, ContractStatus, END};
use crate::db::{answer, set_contract};
use crate::deployment::{AUTO_CANCEL, AUTO_END, Deployment, DeploymentInput, DeploymentStatus};
use crate::deployments::{announce_all_cancelled, finish_end, worker_name};
use crate::events::ContractEnded;
use crate::notices;
use crate::release::close_open;
use crate::request::{self, ManpowerRequest, RequestStatus};
use crate::termination::withdraw_pending;
use crate::texts::english;

/// Whether `contract` is past its end date on `on`: the guard *not renewed*.
pub fn end_problem(contract: &ClientContract, on: NaiveDate) -> Option<String> {
    (contract.end_date >= on).then(|| {
        english(
            "manpower.end.not_yet",
            &[("end", &contract.end_date), ("day", &on)],
        )
    })
}

/// The effect of `end` inside its transaction (see the module).
pub(crate) async fn end_effects(tx: &mut Transaction, contract: &ClientContract) -> Result {
    let none_text: Option<String> = None;
    let none_at: Option<DateTime> = None;
    let none_user: Option<i64> = None;
    set_contract(
        &mut *tx,
        contract.id,
        &[
            ("ended_at", &Some(now())),
            ("status_before_hold", &none_text),
            ("held_at", &none_at),
            ("held_by", &none_user),
        ],
    )
    .await?;
    close_open(&mut *tx, contract.id, "contract_end", false).await?;
    let cancel_reason = english("manpower.end.cancel_reason", &[]);
    let deployments = Deployment::query()
        .where_(Deployment::CLIENT_CONTRACT_ID.eq(contract.id))
        .where_(Deployment::STATUS.is_in([
            DeploymentStatus::Planned,
            DeploymentStatus::Active,
            DeploymentStatus::Suspended,
        ]))
        .order_by(Deployment::ID)
        .get(&mut *tx)
        .await?;
    let input = DeploymentInput {
        problems: Vec::new(),
        on: Some(contract.end_date),
        new_end_date: None,
        ..DeploymentInput::default()
    };
    for deployment in deployments {
        if deployment.status == DeploymentStatus::Planned {
            let _outcome = deployment
                .transition(AUTO_CANCEL)
                .by_system()
                .reason(&cancel_reason)
                .run_in(&mut *tx)
                .await
                .map_err(answer)?;
        } else {
            let _outcome = deployment
                .transition(AUTO_END)
                .by_system()
                .reason(crate::deployment::reasons::CONTRACT_END)
                .input(&input)
                .run_in(&mut *tx)
                .await
                .map_err(answer)?;
        }
    }
    let requests = ManpowerRequest::query()
        .where_(ManpowerRequest::CLIENT_CONTRACT_ID.eq(contract.id))
        .where_(ManpowerRequest::STATUS.is_in([
            RequestStatus::Draft,
            RequestStatus::Open,
            RequestStatus::PartiallyFilled,
        ]))
        .order_by(ManpowerRequest::ID)
        .get(&mut *tx)
        .await?;
    for request in requests {
        let _outcome = request
            .transition(request::AUTO_CANCEL)
            .by_system()
            .reason(&cancel_reason)
            .run_in(&mut *tx)
            .await
            .map_err(answer)?;
    }
    Ok(())
}

/// What finishing the end of a contract did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Finished {
    /// The workers back on the bench.
    pub bench: Vec<EmployeeId>,
    /// The deployments ended.
    pub ended: usize,
    /// The deployments cancelled.
    pub cancelled: usize,
    /// The timesheets closed and submitted.
    pub submitted: usize,
    /// The timesheets a check refused: they stay `open` with a task.
    pub refused: usize,
}

/// What follows the commit of the `end` of contract `id`, once: see the
/// module. Answers what it did, or `None` when another run did it.
///
/// # Errors
///
/// The database's, the kernel's and the notifier's errors.
pub async fn after_end(state: &AppState, id: i64) -> Result<Option<Finished>> {
    let contract = ClientContract::find_or_404(&state.db, id).await?;
    if contract.status != ContractStatus::Ended {
        return Ok(None);
    }
    // Claim the work: a second run finds nothing to do.
    let claimed = ClientContract::query()
        .where_(ClientContract::ID.eq(id))
        .where_(ClientContract::END_ANNOUNCED_AT.is_null())
        .update(&state.db, &[("end_announced_at", &Some(now()))])
        .await?;
    if claimed == 0 {
        return Ok(None);
    }
    let reason = crate::deployment::reasons::CONTRACT_END;
    let ended = Deployment::query()
        .where_(Deployment::CLIENT_CONTRACT_ID.eq(id))
        .where_(Deployment::STATUS.eq(DeploymentStatus::Ended))
        .where_(Deployment::END_REASON.eq(reason))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?;
    let mut done = Finished {
        ended: ended.len(),
        ..Finished::default()
    };
    let mut workers = Vec::new();
    for deployment in &ended {
        // The open timesheet of the period is closed at the end date and goes
        // to the client at once, as at any deployment `end`.
        let (_ended, closed) = finish_end(state, deployment.id, "en").await?;
        done.submitted += closed.iter().filter(|sheet| sheet.submitted).count();
        done.refused += closed.iter().filter(|sheet| !sheet.submitted).count();
        workers.push(deployment.employee());
    }
    let cancel_reason = english("manpower.end.cancel_reason", &[]);
    let cancelled = Deployment::query()
        .where_(Deployment::CLIENT_CONTRACT_ID.eq(id))
        .where_(Deployment::STATUS.eq(DeploymentStatus::Cancelled))
        .where_(Deployment::CANCEL_REASON.eq(cancel_reason.clone()))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?;
    announce_all_cancelled(state, &cancelled, &cancel_reason).await?;
    done.cancelled = cancelled.len();
    done.bench = bench::of(state, &workers).await?;
    state
        .emit(ContractEnded {
            contract: contract.subject(),
            client: contract.client(),
            end_date: contract.end_date,
            bench: done.bench.clone(),
        })
        .await?;
    let mut names = Vec::new();
    for employee in &done.bench {
        names.push(worker_name(state, "en", *employee).await);
    }
    let on_bench = if names.is_empty() {
        english("manpower.end.bench_none", &[])
    } else {
        names.join(", ")
    };
    let (count, off, sheets, held) = (done.ended, done.cancelled, done.submitted, done.refused);
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 7] = [
        ("contract", &contract.reference),
        ("end", &contract.end_date),
        ("ended", &count),
        ("cancelled", &off),
        ("timesheets", &sheets),
        ("open", &held),
        ("bench", &on_bench),
    ];
    let managers =
        notices::account_managers(state, contract.client(), contract.account_manager_user_id)
            .await?;
    notices::tell(
        state,
        contract.subject(),
        &contract.reference,
        &managers,
        &english("manpower.end.title", &params),
        &english("manpower.end.body", &params),
    )
    .await?;
    Ok(Some(done))
}

/// The contracts that are past their end date on `on` and still not `ended`.
async fn due(state: &AppState, on: NaiveDate) -> Result<Vec<ClientContract>> {
    ClientContract::query()
        .where_(ClientContract::STATUS.is_in([
            ContractStatus::Active,
            ContractStatus::OnHold,
            ContractStatus::Terminating,
        ]))
        .where_(ClientContract::END_DATE.lt(on))
        .order_by(ClientContract::ID)
        .get(&state.db)
        .await
}

/// Ends `contract` on `on` as the system; `false` when it cannot (renewed, or
/// ended by another run).
async fn end_one(state: &AppState, contract: &ClientContract, on: NaiveDate) -> Result<bool> {
    let input = ContractInput {
        problems: Vec::new(),
        on: Some(on),
    };
    let done = contract
        .transition(END)
        .by_system()
        .reason(english(
            "manpower.end.history",
            &[("end", &contract.end_date)],
        ))
        .input(&input)
        .run(state)
        .await;
    match done {
        Ok(_outcome) => {}
        // Renewed meanwhile, or another run ended it first.
        Err(
            TransitionError::Guard(_) | TransitionError::NotFrom(_) | TransitionError::Conflict,
        ) => {
            return Ok(false);
        }
        Err(other) => return Err(answer(other)),
    }
    let ended = ClientContract::find_or_404(&state.db, contract.id).await?;
    withdraw_pending(state, &ended, &english("manpower.end.withdraw_reason", &[])).await?;
    Ok(true)
}

/// Runs the end of every contract past its end date on `on`, then finishes
/// the ones whose follow-up was cut short. Returns the contracts ended now.
///
/// # Errors
///
/// The database's, the kernel's and the notifier's errors.
pub async fn end_due(state: &AppState, on: NaiveDate) -> Result<Vec<i64>> {
    let mut ended = Vec::new();
    for contract in due(state, on).await? {
        if end_one(state, &contract, on).await? {
            ended.push(contract.id);
            after_end(state, contract.id).await?;
        }
    }
    // A run that ended a contract and stopped before the follow-up.
    let unfinished = ClientContract::query()
        .where_(ClientContract::STATUS.eq(ContractStatus::Ended))
        .where_(ClientContract::END_ANNOUNCED_AT.is_null())
        .order_by(ClientContract::ID)
        .get(&state.db)
        .await?;
    for contract in unfinished {
        after_end(state, contract.id).await?;
    }
    Ok(ended)
}

/// The daily run at 00:05 Asia/Dubai.
///
/// # Errors
///
/// As [`end_due`].
pub async fn daily(state: &AppState) -> Result {
    end_due(state, today()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract(end: NaiveDate) -> ClientContract {
        ClientContract {
            end_date: end,
            ..ClientContract::default()
        }
    }

    #[test]
    fn a_contract_ends_only_after_its_end_date() {
        let end = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
        let day = |d| NaiveDate::from_ymd_opt(2026, 12, d).unwrap();
        assert!(
            end_problem(&contract(end), day(1)).is_some(),
            "the end date itself"
        );
        assert!(end_problem(&contract(end), day(2)).is_none());
        assert!(end_problem(&contract(day(31)), day(2)).is_some(), "renewed");
    }
}
