//! A blocked client company (09 Manpower §4.1 row *The client is blocked as a
//! party*, §7.1 *Client contract*, BL-MPW-15, wl-parties §5.6, `PTY.ACL.03`).
//!
//! The block effect of a deployment is `continue`
//! ([`BlockEffectOverride`](wl_kernel::block_effects::BlockEffectOverride),
//! decided by the MD): the workers already on site stay, the agency's
//! obligations do not stop, and the contract and the deployments change no
//! state on `PartyBlocked`. What stops is anything new:
//!
//! - `activate` of a deployment and `submit` of a request start new work
//!   ([`ActionKind::Start`]), `extend` of a deployment extends it
//!   ([`ActionKind::Extend`]), and the contract's `renew` renews it
//!   ([`ActionKind::Renew`]): the services ask the kernel's block guard and
//!   refuse a blocked client. `renew` is BL-MPW-05's; it calls [`ensure`] with
//!   `ActionKind::Renew`. A suspended deployment may still `resume` (resume
//!   is not an extension) and a `planned` deployment may still be planned or
//!   cancelled.
//! - The refusal is the kernel's [`BlockRefusal`](wl_kernel::block_guard::BlockRefusal)
//!   as the guard words it for the viewer: *This party is on hold* for staff
//!   without `parties.block_reason.view`, the real reasons for holders. The
//!   restricted code never leaves wl-parties.
//! - `PartyBlocked` opens a task for the MD (the role `md`, else the company
//!   admin) and for the account manager of each of the client's contracts,
//!   listing the client's deployments that are planned, active or suspended.
//!   One task per person while it is open (a second block adds none). The
//!   client gets no message. Decision: a client with no such deployment gets
//!   no task, there is nothing to list.
//! - `PartyUnblocked` needs no listener: nothing was paused, the guards stop
//!   refusing.
//!
//! Decision: no "Request exception" offer (`block_guarded` transitions): the
//! spec gives the decider (the MD) `terminate` for ending, not an exception
//! for starting new work with a blocked client.

use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::block_effects::{BlockEffect, BlockEffectOverride};
use wl_kernel::block_guard::{self, ActionKind, GuardOutcome};
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, task_creator};
use wl_kernel::{PartyBlocked, PartyId, role_tasks};

use crate::contract::ClientContract;
use crate::db::refused;
use crate::deployment::{Deployment, DeploymentStatus};
use crate::deployments::worker_name;
use crate::notices::account_managers;
use crate::permissions;
use crate::site::Site;
use crate::texts::english;

/// The agreement kind of wl-parties §5.6 a deployment is.
pub const KIND: &str = "deployment";

/// Refuses `kind` for the blocked `client` in the caller's transaction:
/// *This party is on hold* for a viewer without `parties.block_reason.view`
/// (and for the system, `by` = `None`).
///
/// # Errors
///
/// A 422 when the client is blocked.
pub async fn check(
    tx: &mut Transaction,
    state: &AppState,
    by: Option<&User>,
    client: PartyId,
    kind: ActionKind,
) -> Result {
    match block_guard::guard(state)
        .check_in(tx, by, client, kind)
        .await?
    {
        GuardOutcome::Allowed => Ok(()),
        GuardOutcome::Blocked(refusal) => Err(refused(refusal.message_en())),
    }
}

/// [`check`] in a transaction of its own, nothing written: for the services
/// that run their transition on their own.
///
/// # Errors
///
/// A 422 when the client is blocked.
pub async fn ensure(
    state: &AppState,
    by: Option<&User>,
    client: PartyId,
    kind: ActionKind,
) -> Result {
    let mut tx = state.db.begin().await?;
    let checked = check(&mut tx, state, by, client, kind).await;
    drop(tx);
    checked
}

/// The line of `deployment` in a task.
async fn line(state: &AppState, deployment: &Deployment) -> String {
    let worker = worker_name(state, "en", deployment.employee()).await;
    let site = Site::find(&state.db, deployment.site_id)
        .await
        .ok()
        .flatten()
        .map(|site| site.name)
        .unwrap_or_default();
    let ends = deployment
        .end_date
        .map_or_else(|| "-".to_owned(), |end| end.to_string());
    english(
        "manpower.blocked.line",
        &[
            ("worker", &worker),
            ("role", &deployment.role),
            ("site", &site),
            ("status", &deployment.status.as_str()),
            ("from", &deployment.start_date),
            ("until", &ends),
        ],
    )
}

/// The tasks of a blocked client: the MD and the account managers get the list
/// of the client's deployments. Answers how many tasks were opened now; one
/// that is still open from an earlier block is not opened again.
///
/// # Errors
///
/// The database's and the ticketing contract's errors.
pub async fn open_tasks(state: &AppState, client: PartyId) -> Result<usize> {
    let deployments = Deployment::query()
        .where_(Deployment::CLIENT_PARTY_ID.eq(client.0))
        .where_(Deployment::STATUS.is_in([
            DeploymentStatus::Planned,
            DeploymentStatus::Active,
            DeploymentStatus::Suspended,
        ]))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?;
    if deployments.is_empty() {
        return Ok(0);
    }
    let mut lines = Vec::new();
    for deployment in &deployments {
        lines.push(line(state, deployment).await);
    }
    let title = english("manpower.blocked.task_title", &[]);
    let description = format!(
        "{}\n{}",
        english("manpower.blocked.task_body", &[]),
        lines.join("\n")
    );
    let mut asked = 0;
    // The MD.
    let creator = task_creator(state);
    let key = format!("manpower.client_blocked:{}:md", client.0);
    let open_already = creator.find_open(state, &key).await?.is_some();
    let task = role_tasks::RoleTask::new(client.subject(), permissions::MD_ROLE, &title, &key)
        .description(&description);
    if role_tasks::open(state, task).await?.is_some() && !open_already {
        asked += 1;
    }
    // The account manager of each of the client's contracts.
    let mut managers = Vec::new();
    for contract in ClientContract::query()
        .where_(ClientContract::CLIENT_PARTY_ID.eq(client.0))
        .order_by(ClientContract::ID)
        .get(&state.db)
        .await?
    {
        for user in account_managers(state, client, contract.account_manager_user_id).await? {
            if !managers.contains(&user) {
                managers.push(user);
            }
        }
    }
    for user in managers {
        let key = format!("manpower.client_blocked:{}:user:{user}", client.0);
        if creator.find_open(state, &key).await?.is_some() {
            continue;
        }
        let ticket = OpenTicket::new(client.subject(), TASK_CATEGORY, &title)
            .description(&description)
            .source_key(&key)
            .assign_to(Assignee::user(user));
        if creator.open(state, ticket).await?.is_some() {
            asked += 1;
        }
    }
    Ok(asked)
}

/// `PartyBlocked` changes no contract or deployment: the MD and the account
/// managers get the task.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: PartyBlocked, state: AppState| async move {
        open_tasks(&state, event.party).await?;
        Ok(())
    });
}

/// The block effect of a deployment: `continue`, decided by the MD.
pub(crate) fn register() {
    BlockEffectOverride::kind(KIND)
        .effect(BlockEffect::Continue)
        .decider(permissions::MD_ROLE)
        .register();
}
