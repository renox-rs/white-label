//! Suspending a deployment (09 Manpower §4.5 *When it goes wrong* row 1, §7.1
//! *Deployment* `suspend`, §7.2 `MPW.RUL.12`, BL-MPW-11).
//!
//! A worker whose visa, work permit, Emirates ID, health insurance or site
//! licence expires may not work at the site, and the agency stops billing.
//! The deployment is `suspended`: attendance at the site is refused
//! ([`crate::attendance`]), the days are not billed ([`Deployment::days_suspended`]
//! keeps them), the client is told with a neutral reason, the account manager
//! is told, and cover is requested when the contract requires it
//! ([`crate::cover::open_cover`], cause `suspension`, one worker with the role,
//! site and shift, from the suspension date, open-ended).
//!
//! - **By hand** ([`suspend`], `deployments.manage` for the client): an
//!   `active` deployment with a reason (*site access withdrawn*,
//!   *investigation*...; refused without one). The suspension date is the day.
//!   On a deployment already `suspended` the reason is added.
//! - **`MPW.RUL.12`, the expiry part** ([`listen`]): the kernel's
//!   `DocumentExpired` and wl-hr's `QualificationExpired` (the day after the
//!   expiry date, the first nightly run) suspend every `active` deployment of
//!   the worker that needs the kind: a deployment needs it when the kind has a
//!   `blocks` entry in mode `block` on a deployment transition whose condition
//!   on the role matches. The suspension date is **the day after the expiry
//!   date**. A kind that is valid again today (a renewal accepted before the
//!   run) suspends nothing. A deployment already `suspended` gets the kind added
//!   to its reasons: no second transition, no second message. A `planned`
//!   deployment is not changed (its `activate` is refused, `MPW.ACL.07`).
//!   The same event twice suspends once. Nothing here ends a deployment.
//! - **A rule run** (compliance-critical, `PLT.RUL.04`): each deployment is one
//!   `run_rule` with the key `MPW.RUL.12:<kind>:<expiry date>`, retried by the
//!   02:00 run ([`retry`]) up to five attempts, then the MD and the company
//!   admin get the kernel's alert. The follow-up after the commit ([`announce`])
//!   is marked done on the deployment (`suspension_announced_at`), so a failed
//!   follow-up is finished by the retry and a done one never repeats.
//! - **Reasons** are rows ([`DeploymentSuspension`]): a document reason is
//!   cleared by the acceptance of a renewed document (BL-MPW-12), a manual one
//!   only by staff. Decision: the client's message never names the document
//!   (*is temporarily unavailable*); the account manager's does.
//! - **Billing.** The suspended days are kept (`suspended_on`, `resumed_on`,
//!   `suspended_days`) for the usage billing of the deployment; **the billing
//!   itself (`MPW.RUL.05`) is not built in this app**, so nothing here
//!   reduces an invoice.

use std::collections::BTreeSet;

use renox::chrono::{Days, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::automation::{RuleRun, RuleRunSpec, RuleRunStatus, run_rule};
use wl_kernel::documents::{BlockMode, DocumentExpired, guard, today};
use wl_kernel::workforce::{EMPLOYEE_SUBJECT_TYPE, QualificationExpired};

use crate::contract::ClientContract;
use crate::cover::{self, CoverAsk, causes};
use crate::db::{answer, refused};
use crate::deployment::{
    AUTO_SUSPEND, Deployment, DeploymentInput, DeploymentStatus, SUBJECT_TYPE, SUSPEND,
};
use crate::deployments::worker_name;
use crate::events::DeploymentSuspended;
use crate::notices;
use crate::permissions;
use crate::site::Site;
use crate::texts::english;

/// The rule id.
pub const RULE: &str = "MPW.RUL.12";

/// Why a deployment is suspended.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SuspensionKind {
    /// A document or licence the deployment needs expired.
    #[default]
    Document,
    /// Staff suspended it (site access withdrawn, investigation...).
    Manual,
}

/// One reason a deployment is suspended.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manpower_deployment_suspensions", index(deployment_id))]
pub struct DeploymentSuspension {
    /// The primary key.
    pub id: i64,
    /// The deployment.
    pub deployment_id: i64,
    /// A document or a manual reason.
    pub kind: SuspensionKind,
    /// The document kind's key, or what staff wrote.
    pub reason: String,
    /// A document reason: the expiry date; the kind was valid through it.
    pub expired_on: Option<NaiveDate>,
    /// The day the reason was added.
    pub added_on: NaiveDate,
    /// The user who suspended; none when the rule did.
    pub added_by: Option<i64>,
    /// The day the cause was gone; none while it holds.
    pub cleared_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DeploymentSuspension {
    /// The reasons of deployment `deployment_id` that still hold, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn open_for(
        db: impl renox::db::Executor<'_>,
        deployment_id: i64,
    ) -> Result<Vec<DeploymentSuspension>> {
        DeploymentSuspension::query()
            .where_(DeploymentSuspension::DEPLOYMENT_ID.eq(deployment_id))
            .where_(DeploymentSuspension::CLEARED_ON.is_null())
            .order_by(DeploymentSuspension::ID)
            .get(db)
            .await
    }

    /// The reason as people read it: the document kind without underscores, or
    /// the text staff wrote.
    pub fn text(&self) -> String {
        match self.kind {
            SuspensionKind::Document => self.reason.replace('_', " "),
            SuspensionKind::Manual => self.reason.clone(),
        }
    }
}

/// What a suspension is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The document kind `kind` expired; it was valid through `expired_on`.
    Document {
        /// The kind's key.
        kind: String,
        /// The last valid day.
        expired_on: NaiveDate,
    },
    /// Staff gave this reason.
    Manual(String),
}

impl Reason {
    /// The text kept on the transition.
    fn history(&self) -> String {
        match self {
            Reason::Document { kind, .. } => english(
                "manpower.deployment.suspend_document",
                &[("kind", &kind.replace('_', " "))],
            ),
            Reason::Manual(text) => text.clone(),
        }
    }
}

/// What [`apply`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// The deployment became `suspended`.
    Suspended,
    /// It was `suspended` already: the reason was added.
    Added,
    /// Nothing changed: the reason was there, or the deployment is not on site.
    Unchanged,
}

/// Whether `open` already holds `reason`.
fn holds(open: &[DeploymentSuspension], reason: &Reason) -> bool {
    open.iter().any(|row| match reason {
        Reason::Document { kind, .. } => {
            row.kind == SuspensionKind::Document && &row.reason == kind
        }
        Reason::Manual(text) => row.kind == SuspensionKind::Manual && &row.reason == text,
    })
}

/// Suspends deployment `id` for `reason` from `on`, or adds the reason to a
/// deployment that is `suspended` already, in one transaction. `by` is the
/// user (none: the system, which ignores a deployment that is not on site; a
/// user is refused one).
///
/// # Errors
///
/// A 422 for a user when the deployment is not `active` or `suspended`; the
/// database's errors.
pub async fn apply(
    state: &AppState,
    id: i64,
    reason: &Reason,
    on: NaiveDate,
    by: Option<&AuthUser>,
) -> Result<Applied> {
    let mut tx = state.db.begin().await?;
    let deployment = Deployment::query()
        .where_(Deployment::ID.eq(id))
        .lock_for_update()
        .first(&mut tx)
        .await?
        .ok_or(Error::NotFound)?;
    let open = DeploymentSuspension::open_for(&mut tx, id).await?;
    let applied = match deployment.status {
        DeploymentStatus::Active => Applied::Suspended,
        DeploymentStatus::Suspended if holds(&open, reason) => Applied::Unchanged,
        DeploymentStatus::Suspended => Applied::Added,
        _ if by.is_some() => {
            return Err(refused(english("manpower.deployment.suspend_state", &[])));
        }
        _ => return Ok(Applied::Unchanged),
    };
    if applied == Applied::Unchanged {
        return Ok(applied);
    }
    let outcome = if applied == Applied::Suspended {
        let input = DeploymentInput {
            problems: Vec::new(),
            on: Some(on),
            new_end_date: None,
        };
        let transition = match by {
            Some(user) => deployment.transition(SUSPEND).by(user),
            None => deployment.transition(AUTO_SUSPEND).by_system(),
        };
        Some(
            transition
                .reason(reason.history())
                .input(&input)
                .run_in(&mut tx)
                .await
                .map_err(answer)?,
        )
    } else {
        None
    };
    let (kind, text, expired_on) = match reason {
        Reason::Document { kind, expired_on } => {
            (SuspensionKind::Document, kind.clone(), Some(*expired_on))
        }
        Reason::Manual(text) => (SuspensionKind::Manual, text.clone(), None),
    };
    DeploymentSuspension::create(
        &mut tx,
        DeploymentSuspension {
            deployment_id: id,
            kind,
            reason: text,
            expired_on,
            added_on: on,
            added_by: by.map(|user| user.id),
            ..DeploymentSuspension::default()
        },
    )
    .await?;
    tx.commit().await?;
    if let Some(outcome) = outcome {
        outcome.emit(state).await?;
    }
    Ok(applied)
}

/// `by` suspends active deployment `id` with `reason` (*site access
/// withdrawn*, *investigation*...), from today; on a deployment that is
/// `suspended` already the reason is added. The client is told it is
/// *temporarily unavailable*, the account manager is told, and cover is
/// requested when the contract requires it.
///
/// # Errors
///
/// `Forbidden` without `deployments.manage` for the client; a 422 without a
/// reason or unless the deployment is `active` or `suspended`.
pub async fn suspend(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    lang: &str,
) -> Result<Deployment> {
    let deployment = Deployment::find_or_404(&state.db, id).await?;
    if !by.has_permission_in(permissions::DEPLOYMENTS_MANAGE, &deployment.scope()) {
        return Err(Error::Forbidden);
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refused(english("manpower.deployment.suspend_reason", &[])));
    }
    apply(
        state,
        id,
        &Reason::Manual(reason.to_owned()),
        today(),
        Some(by),
    )
    .await?;
    announce(state, id, lang).await?;
    Deployment::find_or_404(&state.db, id).await
}

/// What follows the commit of a suspension, once: [`DeploymentSuspended`], the
/// cover request when the contract requires it, the account manager's notice
/// and the client's neutral one. Marked done on the deployment, so a second
/// call, or a deployment that is not `suspended`, does nothing; a failure
/// gives the mark back so the next call finishes the work.
///
/// # Errors
///
/// The errors of the steps.
pub async fn announce(state: &AppState, id: i64, lang: &str) -> Result {
    let deployment = Deployment::find_or_404(&state.db, id).await?;
    if deployment.status != DeploymentStatus::Suspended
        || deployment.suspension_announced_at.is_some()
    {
        return Ok(());
    }
    let claimed = Deployment::query()
        .where_(Deployment::ID.eq(id))
        .where_(Deployment::SUSPENSION_ANNOUNCED_AT.is_null())
        .update(
            &state.db,
            &[("suspension_announced_at", &Some(renox::db::now()))],
        )
        .await?;
    if claimed == 0 {
        return Ok(());
    }
    match follow_up(state, &deployment, lang).await {
        Ok(()) => Ok(()),
        Err(error) => {
            let none: Option<DateTime> = None;
            Deployment::query()
                .where_(Deployment::ID.eq(id))
                .update(&state.db, &[("suspension_announced_at", &none)])
                .await?;
            Err(error)
        }
    }
}

/// The steps of [`announce`].
async fn follow_up(state: &AppState, deployment: &Deployment, lang: &str) -> Result {
    let id = deployment.id;
    let since = deployment.suspended_on.unwrap_or_else(today);
    let contract = ClientContract::find_or_404(&state.db, deployment.client_contract_id).await?;
    let reasons: Vec<String> = DeploymentSuspension::open_for(&state.db, id)
        .await?
        .iter()
        .map(DeploymentSuspension::text)
        .collect();
    state
        .emit(DeploymentSuspended {
            deployment: deployment.subject(),
            employee: deployment.employee(),
            client: deployment.client(),
            from: since,
            reasons: reasons.clone(),
        })
        .await?;
    cover::open_cover(
        state,
        CoverAsk {
            deployment,
            from: since,
            to: None,
            cause: causes::SUSPENSION,
            cause_ref: Some(format!("{SUBJECT_TYPE}:{id}:suspension:{since}")),
        },
    )
    .await?;
    let worker = worker_name(state, lang, deployment.employee()).await;
    let site = Site::find(&state.db, deployment.site_id)
        .await?
        .map(|site| site.name)
        .unwrap_or_default();
    let why = reasons.join(", ");
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 4] = [
        ("worker", &worker),
        ("site", &site),
        ("from", &since),
        ("reasons", &why),
    ];
    let managers =
        notices::account_managers(state, contract.client(), contract.account_manager_user_id)
            .await?;
    notices::tell(
        state,
        deployment.subject(),
        &contract.reference,
        &managers,
        &english("manpower.deployment.suspended_staff_title", &params),
        &english("manpower.deployment.suspended_staff_body", &params),
    )
    .await?;
    notices::tell_client(
        state,
        deployment.subject(),
        &contract.reference,
        deployment.client(),
        &english("manpower.deployment.suspended_title", &params),
        &english("manpower.deployment.suspended_body", &params),
    )
    .await
}

/// The key of the run for `kind` expired on `expires_on`.
fn key(kind: &str, expires_on: NaiveDate) -> String {
    format!("{RULE}:{kind}:{expires_on}")
}

/// One `MPW.RUL.12` run for deployment `id`: suspends it (or adds the kind to
/// its reasons) for the expired `kind` and finishes the follow-up.
async fn run(state: &AppState, id: i64, kind: &str, expires_on: NaiveDate) -> Result<()> {
    let spec = RuleRunSpec::new(RULE, Subject::new(SUBJECT_TYPE, id), key(kind, expires_on))
        .compliance_critical();
    let kind = kind.to_owned();
    run_rule(state, spec, move |state: AppState| async move {
        react(&state, id, &kind, expires_on).await
    })
    .await?;
    Ok(())
}

/// The action of a run.
async fn react(state: &AppState, id: i64, kind: &str, expires_on: NaiveDate) -> Result {
    let Some(deployment) = Deployment::find(&state.db, id).await? else {
        return Ok(());
    };
    // Only the deployments on site: a planned one is refused at `activate`.
    if !deployment.is_on_site() {
        return Ok(());
    }
    // A renewal accepted before the run: the kind is valid again.
    let subject = Subject::new(EMPLOYEE_SUBJECT_TYPE, deployment.employee_id);
    if guard(state)
        .is_valid(&state.db, &subject, kind, today())
        .await?
        .is_valid()
    {
        return Ok(());
    }
    let reason = Reason::Document {
        kind: kind.to_owned(),
        expired_on: expires_on,
    };
    let suspension_date = expires_on
        .checked_add_days(Days::new(1))
        .unwrap_or(expires_on);
    apply(state, id, &reason, suspension_date, None).await?;
    announce(state, id, "en").await
}

/// Runs `MPW.RUL.12` for the deployments `ids` and the expired `kind`.
async fn react_all(
    state: &AppState,
    ids: BTreeSet<i64>,
    kind: &str,
    expires_on: NaiveDate,
) -> Result {
    let mut failure = None;
    for id in ids {
        if let Err(error) = run(state, id, kind, expires_on).await {
            eprintln!("{RULE}: deployment {id}: {error:?}");
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// `DocumentExpired` for a worker: suspends the deployments on site that need
/// the kind.
async fn on_document_expired(state: &AppState, event: &DocumentExpired) -> Result {
    if event.subject.subject_type != EMPLOYEE_SUBJECT_TYPE {
        return Ok(());
    }
    let entries: Vec<_> = event
        .blocks
        .iter()
        .filter(|entry| entry.entity() == "deployment" && entry.mode == BlockMode::Block)
        .collect();
    if entries.is_empty() {
        return Ok(());
    }
    let ids = Deployment::query()
        .where_(Deployment::EMPLOYEE_ID.eq(event.subject.subject_id))
        .where_(Deployment::STATUS.is_in([DeploymentStatus::Active, DeploymentStatus::Suspended]))
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|deployment| {
            entries
                .iter()
                .any(|entry| entry.applies_to(Some(deployment.role.as_str())))
        })
        .map(|deployment| deployment.id)
        .collect();
    react_all(state, ids, &event.kind, event.expires_on).await
}

/// `QualificationExpired` for a worker: the deployments on site that the
/// expiry blocks (the same runs as the document's: one per deployment, kind
/// and expiry date).
async fn on_qualification_expired(state: &AppState, event: &QualificationExpired) -> Result {
    let ids: BTreeSet<i64> = event
        .affected
        .iter()
        .filter(|work| work.open && work.record.subject_type == SUBJECT_TYPE)
        .map(|work| work.record.subject_id)
        .collect();
    react_all(state, ids, &event.kind, event.expires_on).await
}

/// Tries again the runs of `MPW.RUL.12` that failed and wait (the scheduled
/// 02:00 run): five attempts, then the alert of the kernel.
///
/// # Errors
///
/// The first error of the runs.
pub async fn retry(state: &AppState) -> Result {
    let waiting = RuleRun::query()
        .where_(RuleRun::RULE.eq(RULE))
        .where_(RuleRun::STATUS.eq(RuleRunStatus::Retrying))
        .order_by(RuleRun::ID)
        .get(&state.db)
        .await?;
    let mut failure = None;
    for waiting_run in waiting {
        // `MPW.RUL.12:<kind>:<expiry date>`.
        let mut parts = waiting_run.occurrence_key.splitn(3, ':');
        let (Some(_rule), Some(kind), Some(date)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let Ok(expires_on) = date.parse::<NaiveDate>() else {
            continue;
        };
        if let Err(error) = run(state, waiting_run.subject_id, kind, expires_on).await {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// Marks `MPW.RUL.12` compliance-critical, owned by the MD (`PLT.RUL.04`).
pub(crate) fn register() {
    wl_kernel::compliance::CriticalRule::new(RULE)
        .owner_role(permissions::MD_ROLE)
        .register();
}

/// Listens for the expiry of a document or a qualification.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: DocumentExpired, state: AppState| async move {
        on_document_expired(&state, &event).await
    });
    app.listen(|event: QualificationExpired, state: AppState| async move {
        on_qualification_expired(&state, &event).await
    });
}
