//! Warning before a client contract ends (09 Manpower §4.1 row *The contract
//! expires, or ends early*, §7.2 `MPW.RUL.11`; BL-MPW-05).
//!
//! A contract must not run out unnoticed with workers on site. Daily at 02:00
//! Asia/Dubai ([`daily`]) the rule looks at every contract `active` or
//! `on_hold` and tells its account manager, N = 60 and M = 30 days before the
//! end date ([`crate::config`]), which deployments will end with it (the
//! workers `active` or `suspended`, with their end date).
//!
//! - **One notice per threshold per end date.** A row ([`ContractEndNotice`],
//!   unique on contract, end date and threshold) is the claim, so a second run,
//!   or a run at the same moment, tells nobody twice. A renewal moves the end
//!   date, so the notices of the new date are due again and the ones already
//!   sent for the old date are not sent again.
//! - **A missed notice is sent at the next run.** A contract created or renewed
//!   inside the window (40 days left) gets the 60-day notice at its next run
//!   and the 30-day one on time. When both thresholds are behind (25 days left,
//!   nothing sent), one message goes out for the nearer threshold and both are
//!   recorded: the account manager is not told twice about the same state.
//! - **Nobody to tell.** When the contract has no account manager and nobody
//!   holds the role, nothing is recorded, so the notice goes out the day there
//!   is someone.
//! - **The rule never ends a contract.** Ending it on the day after the end
//!   date is the contract's `end` (BL-MPW-06). A contract `terminating` is
//!   not warned: its end date was set by the notice.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::config::{DEFAULT_END_NOTICE_FIRST_DAYS, DEFAULT_END_NOTICE_SECOND_DAYS};
use crate::contract::{ClientContract, ContractStatus};
use crate::deployment::{Deployment, DeploymentStatus};
use crate::deployments::worker_name;
use crate::notices;
use crate::site::Site;
use crate::texts::english;

/// A notice of `MPW.RUL.11` that was claimed: the threshold in days before
/// the end date, for one end date of one contract.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manpower_contract_end_notices",
    unique(client_contract_id, end_date, days_before),
    index(client_contract_id)
)]
pub struct ContractEndNotice {
    /// The primary key.
    pub id: i64,
    /// The contract.
    pub client_contract_id: i64,
    /// The end date the notice was about.
    pub end_date: NaiveDate,
    /// The threshold: 60 or 30 days before the end date.
    pub days_before: i64,
    /// When it was sent.
    pub sent_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The thresholds, the nearer first.
const THRESHOLDS: [i64; 2] = [
    DEFAULT_END_NOTICE_SECOND_DAYS,
    DEFAULT_END_NOTICE_FIRST_DAYS,
];

/// The thresholds of `contract` on `on` that have been reached: the days left
/// are at most the threshold.
fn reached(contract: &ClientContract, on: NaiveDate) -> Vec<i64> {
    let left = (contract.end_date - on).num_days();
    THRESHOLDS
        .into_iter()
        .filter(|threshold| left <= *threshold)
        .collect()
}

/// The deployments that end with `contract`, one line each.
async fn lines(state: &AppState, contract: &ClientContract) -> Result<Vec<String>> {
    let deployments = Deployment::query()
        .where_(Deployment::CLIENT_CONTRACT_ID.eq(contract.id))
        .where_(Deployment::STATUS.is_in([DeploymentStatus::Active, DeploymentStatus::Suspended]))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?;
    let mut lines = Vec::new();
    for deployment in &deployments {
        let worker = worker_name(state, "en", deployment.employee()).await;
        let site = Site::find(&state.db, deployment.site_id)
            .await?
            .map(|site| site.name)
            .unwrap_or_default();
        let until = deployment
            .end_date
            .map_or_else(|| "-".to_owned(), |end| end.to_string());
        lines.push(english(
            "manpower.expiry.line",
            &[
                ("worker", &worker),
                ("role", &deployment.role),
                ("site", &site),
                ("until", &until),
            ],
        ));
    }
    Ok(lines)
}

/// Runs `MPW.RUL.11` for contract `contract` on `on`: tells the account manager
/// once per threshold reached and end date. Returns the threshold told, none
/// when nothing was due.
///
/// # Errors
///
/// The database's and the notifier's errors.
pub async fn run_contract(
    state: &AppState,
    contract: &ClientContract,
    on: NaiveDate,
) -> Result<Option<i64>> {
    if !matches!(
        contract.status,
        ContractStatus::Active | ContractStatus::OnHold
    ) || contract.end_date < on
    {
        return Ok(None);
    }
    let reached = reached(contract, on);
    let sent: Vec<i64> = ContractEndNotice::query()
        .where_(ContractEndNotice::CLIENT_CONTRACT_ID.eq(contract.id))
        .where_(ContractEndNotice::END_DATE.eq(contract.end_date))
        .get(&state.db)
        .await?
        .iter()
        .map(|row| row.days_before)
        .collect();
    let due: Vec<i64> = reached
        .iter()
        .copied()
        .filter(|threshold| !sent.contains(threshold))
        .collect();
    let Some(nearest) = due.first().copied() else {
        return Ok(None);
    };
    // Claim every threshold reached: the unique index makes a second run, or
    // one at the same moment, find the claim.
    let mut claimed = Vec::new();
    for threshold in &due {
        let row = ContractEndNotice::create(
            &state.db,
            ContractEndNotice {
                client_contract_id: contract.id,
                end_date: contract.end_date,
                days_before: *threshold,
                sent_at: Some(now()),
                ..ContractEndNotice::default()
            },
        )
        .await;
        match row {
            Ok(row) => claimed.push(row),
            Err(_) => return Ok(None),
        }
    }
    let managers =
        notices::account_managers(state, contract.client(), contract.account_manager_user_id)
            .await?;
    let lines = lines(state, contract).await?;
    let days = (contract.end_date - on).num_days();
    let count = lines.len();
    let listing = if lines.is_empty() {
        english("manpower.expiry.none", &[])
    } else {
        lines.join("\n")
    };
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 4] = [
        ("contract", &contract.reference),
        ("end", &contract.end_date),
        ("days", &days),
        ("count", &count),
    ];
    let title = english("manpower.expiry.title", &params);
    let body = format!("{}\n{listing}", english("manpower.expiry.body", &params));
    let told = notices::tell(
        state,
        contract.subject(),
        &contract.reference,
        &managers,
        &title,
        &body,
    )
    .await?;
    if !told {
        // Nobody to tell: release the claim so the notice goes out later.
        for row in claimed {
            ContractEndNotice::query()
                .where_(ContractEndNotice::ID.eq(row.id))
                .delete(&state.db)
                .await?;
        }
        return Ok(None);
    }
    Ok(Some(nearest))
}

/// Runs `MPW.RUL.11` for every contract on `on`. Returns the contracts told,
/// with the threshold.
///
/// # Errors
///
/// As [`run_contract`].
pub async fn run(state: &AppState, on: NaiveDate) -> Result<Vec<(i64, i64)>> {
    let contracts = ClientContract::query()
        .where_(ClientContract::STATUS.is_in([ContractStatus::Active, ContractStatus::OnHold]))
        .where_(ClientContract::END_DATE.gte(on))
        .order_by(ClientContract::ID)
        .get(&state.db)
        .await?;
    let mut told = Vec::new();
    for contract in &contracts {
        if let Some(threshold) = run_contract(state, contract, on).await? {
            told.push((contract.id, threshold));
        }
    }
    Ok(told)
}

/// The daily run at 02:00 Asia/Dubai.
///
/// # Errors
///
/// As [`run`].
pub async fn daily(state: &AppState) -> Result {
    run(state, today()).await?;
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

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn the_thresholds_are_reached_by_the_days_left() {
        let end = contract(day(2026, 12, 31));
        assert!(reached(&end, day(2026, 10, 31)).is_empty(), "61 days left");
        assert_eq!(reached(&end, day(2026, 11, 1)), [60], "60 days left");
        assert_eq!(reached(&end, day(2026, 11, 20)), [60], "41 days left");
        assert_eq!(reached(&end, day(2026, 12, 1)), [30, 60], "30 days left");
        assert_eq!(reached(&end, day(2026, 12, 31)), [30, 60], "the end date");
    }
}
