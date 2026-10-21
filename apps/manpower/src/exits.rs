//! The deployments follow the non-standard exits of the worker (09 Manpower
//! §4.2 row *The new visa case is rejected*, §4.7 rows *The notice is
//! withdrawn*, *The worker absconds*, *The worker dies in service*, §7.1
//! *Deployment* `end` and `cancel`, §7.2 `MPW.RUL.12`; BL-MPW-13).
//!
//! wl-hr changes the employee and announces it through the kernel
//! (`wl_kernel::workforce`); this module is the vertical's side of
//! `MPW.RUL.12`:
//!
//! | Event | Reaction |
//! |---|---|
//! | `EmployeeAbsconded` | `active` and `suspended` deployments `end` (`auto_end`, reason *absconded*, last day the report date); `planned` ones are cancelled (*worker absconded*) |
//! | `EmployeeDeceased` | the same with the reason *deceased* and the date of death |
//! | `EmployeeExited` | the same with the reason *exit* and the exit date (a worker who left through notice; whatever is still on site goes) |
//! | `EmployeeOnboardingCancelled` | `planned` deployments are cancelled; their requests stay unfilled and the account manager is told |
//! | `EmployeeNoticeGiven` | the end date of the deployments that run past the last working day is set to it; the end date it replaced is kept (`notice_last_day`, `end_date_before_notice`) |
//! | `EmployeeNoticeWithdrawn` | the end date is restored to the one `EmployeeNoticeGiven` replaced (empty if there was none); the client and the account manager are told |
//! | `VisaCaseRejected` (*renewal*) | the account manager of each client and the MD get one message listing the deployments that run past the current visa's expiry date; nothing changes on a deployment, the client is told nothing. A *new* or *cancellation* case is ignored here |
//! | `EmployeeReinstated` | **no reaction**: an ended or cancelled deployment stays so and the account manager plans a new one |
//!
//! Decisions taken where the spec is open:
//!
//! - **The client is told no reason.** After an exit the client gets *<worker>
//!   is no longer available* (never *absconded* or *died*); the account manager
//!   gets the real reason, one message per contract with the deployments
//!   ended and cancelled and the requests that stay unfilled. A request that
//!   the ended deployment filled does not reopen. A cancelled `planned`
//!   deployment is announced to the client only when its start was
//!   communicated, which is when it filled a request of the client
//!   ([`Deployment::start_communicated`]).
//! - **The last day** of an ended deployment is the date of the event (report
//!   date, date of death, exit date), or the end date if that is earlier; the
//!   days of absence before it have no attendance and are not billed.
//! - **`give_notice`** is not built in wl-hr, so the kernel got the event
//!   `EmployeeNoticeGiven` that it will emit; this module caps the end date on
//!   it. A deployment extended after the notice keeps its new end date when
//!   the notice is withdrawn (the restore only touches an end date that is
//!   still the one the notice set).
//! - **Idempotency.** Each reaction is one compliance-critical `run_rule`
//!   (`MPW.RUL.12:exit:<what>:<employee>:<date>`), retried by the 02:00 run
//!   ([`crate::suspension::retry`]) and alerting the MD and the admin after five
//!   attempts; a repeated event finds nothing to change. The follow-up after
//!   the commit (client, account manager, events, last timesheet) is claimed per
//!   deployment (`exit_announced_at`), so a failed follow-up is finished by the
//!   retry and a done one never repeats.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::Subject;
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::documents::today;
use wl_kernel::workforce::{
    EMPLOYEE_SUBJECT_TYPE, EmployeeAbsconded, EmployeeDeceased, EmployeeExited, EmployeeId,
    EmployeeNoticeGiven, EmployeeNoticeWithdrawn, EmployeeOnboardingCancelled, VisaCaseRejected,
};

use crate::contract::ClientContract;
use crate::db::{answer, set_deployment};
use crate::deployment::{
    AUTO_CANCEL, AUTO_END, Deployment, DeploymentInput, DeploymentStatus, reasons,
};
use crate::deployments::{announce_cancelled, finish_end_as, tell_client, worker_name};
use crate::notices;
use crate::request::ManpowerRequest;
use crate::site::Site;
use crate::suspension::RULE;
use crate::texts::english;

/// What `MPW.RUL.12` is asked to do, as the retry key names it.
pub mod what {
    /// `EmployeeAbsconded`.
    pub const ABSCONDED: &str = "absconded";
    /// `EmployeeDeceased`.
    pub const DECEASED: &str = "deceased";
    /// `EmployeeExited`.
    pub const EXITED: &str = "exited";
    /// `EmployeeOnboardingCancelled`.
    pub const ONBOARDING_CANCELLED: &str = "onboarding_cancelled";
    /// `EmployeeNoticeGiven`.
    pub const NOTICE_GIVEN: &str = "notice_given";
    /// `EmployeeNoticeWithdrawn`.
    pub const NOTICE_WITHDRAWN: &str = "notice_withdrawn";
    /// `VisaCaseRejected` for a renewal.
    pub const VISA_REJECTED: &str = "visa_rejected";
}

impl Deployment {
    /// Whether the client knows the start of this deployment: it fills a
    /// request of the client, which names the position and its start.
    pub fn start_communicated(&self) -> bool {
        self.request_id.is_some()
    }
}

/// The end reason of the exit `what`, and the word for the account manager.
fn end_reason(what: &str) -> Option<&'static str> {
    match what {
        what::ABSCONDED => Some(reasons::ABSCONDED),
        what::DECEASED => Some(reasons::DECEASED),
        what::EXITED => Some(reasons::EXIT),
        _ => None,
    }
}

/// The reason a planned deployment is cancelled with for the exit `what`.
fn cancel_reason(what: &str) -> String {
    english(&format!("manpower.exit.reason.{what}"), &[])
}

/// Ends the `active` and `suspended` deployments of `employee` with the
/// reason of the exit `what` on `on` and cancels the `planned` ones (only the
/// cancelling for an onboarding that was cancelled), in one transaction, and
/// then takes care of the follow-up. Returns the deployments ended and
/// cancelled now; nothing is left to change the second time.
///
/// # Errors
///
/// The database's, the kernel's and the notifier's errors.
pub async fn leave(
    state: &AppState,
    employee: EmployeeId,
    what: &str,
    on: NaiveDate,
) -> Result<(usize, usize)> {
    let reason = end_reason(what);
    let cancelled_for = cancel_reason(what);
    let mut statuses = vec![DeploymentStatus::Planned];
    if reason.is_some() {
        statuses.extend([DeploymentStatus::Active, DeploymentStatus::Suspended]);
    }
    let mut tx = state.db.begin().await?;
    let found = Deployment::query()
        .where_(Deployment::EMPLOYEE_ID.eq(employee.0))
        .where_(Deployment::STATUS.is_in(statuses))
        .order_by(Deployment::ID)
        .lock_for_update()
        .get(&mut tx)
        .await?;
    let input = DeploymentInput {
        problems: Vec::new(),
        on: Some(on),
        new_end_date: None,
        ..DeploymentInput::default()
    };
    let mut outcomes = Vec::new();
    let (mut ended, mut cancelled) = (0, 0);
    for deployment in &found {
        if deployment.status == DeploymentStatus::Planned {
            outcomes.push(
                deployment
                    .transition(AUTO_CANCEL)
                    .by_system()
                    .reason(&cancelled_for)
                    .run_in(&mut tx)
                    .await
                    .map_err(answer)?,
            );
            cancelled += 1;
        } else if let Some(reason) = reason {
            outcomes.push(
                deployment
                    .transition(AUTO_END)
                    .by_system()
                    .reason(reason)
                    .input(&input)
                    .run_in(&mut tx)
                    .await
                    .map_err(answer)?,
            );
            ended += 1;
        }
    }
    tx.commit().await?;
    for outcome in outcomes {
        outcome.emit(state).await?;
    }
    follow_up(state, employee, what, on).await?;
    Ok((ended, cancelled))
}

/// The deployments of `employee` that this exit ended or cancelled and whose
/// follow-up is still to do.
async fn unannounced(
    state: &AppState,
    employee: EmployeeId,
    what: &str,
    on: NaiveDate,
) -> Result<Vec<Deployment>> {
    let mut rows = Vec::new();
    if let Some(reason) = end_reason(what) {
        rows.extend(
            Deployment::query()
                .where_(Deployment::EMPLOYEE_ID.eq(employee.0))
                .where_(Deployment::STATUS.eq(DeploymentStatus::Ended))
                .where_(Deployment::END_REASON.eq(reason))
                .where_(Deployment::ENDED_ON.eq(on))
                .where_(Deployment::EXIT_ANNOUNCED_AT.is_null())
                .order_by(Deployment::ID)
                .get(&state.db)
                .await?,
        );
    }
    rows.extend(
        Deployment::query()
            .where_(Deployment::EMPLOYEE_ID.eq(employee.0))
            .where_(Deployment::STATUS.eq(DeploymentStatus::Cancelled))
            .where_(Deployment::CANCEL_REASON.eq(cancel_reason(what)))
            .where_(Deployment::EXIT_ANNOUNCED_AT.is_null())
            .order_by(Deployment::ID)
            .get(&state.db)
            .await?,
    );
    Ok(rows)
}

/// Claims the follow-up of deployment `id`: true for the one caller that
/// marks it.
async fn claim(state: &AppState, id: i64) -> Result<bool> {
    Ok(Deployment::query()
        .where_(Deployment::ID.eq(id))
        .where_(Deployment::EXIT_ANNOUNCED_AT.is_null())
        .update(&state.db, &[("exit_announced_at", &Some(now()))])
        .await?
        > 0)
}

/// Gives the claim of deployment `id` back, so the retry does the work.
async fn release(state: &AppState, id: i64) -> Result {
    let none: Option<DateTime> = None;
    Deployment::query()
        .where_(Deployment::ID.eq(id))
        .update(&state.db, &[("exit_announced_at", &none)])
        .await?;
    Ok(())
}

/// What follows the commit: for each deployment ended the last timesheet is
/// closed, `DeploymentEnded` is emitted and the client told it is no longer
/// available; for each cancelled one `DeploymentCancelled` is emitted and the
/// client told when the start was communicated; the account manager gets one
/// message per contract.
async fn follow_up(state: &AppState, employee: EmployeeId, what: &str, on: NaiveDate) -> Result {
    let mut by_contract: BTreeMap<i64, Vec<Deployment>> = BTreeMap::new();
    for deployment in unannounced(state, employee, what, on).await? {
        if !claim(state, deployment.id).await? {
            continue;
        }
        let done = announce_one(state, &deployment, what).await;
        if let Err(error) = done {
            release(state, deployment.id).await?;
            return Err(error);
        }
        by_contract
            .entry(deployment.client_contract_id)
            .or_default()
            .push(deployment);
    }
    let worker = worker_name(state, "en", employee).await;
    for (contract_id, deployments) in by_contract {
        tell_account_manager(state, &worker, what, contract_id, &deployments).await?;
    }
    Ok(())
}

/// The follow-up of one deployment.
async fn announce_one(state: &AppState, deployment: &Deployment, what: &str) -> Result {
    if deployment.status == DeploymentStatus::Ended {
        finish_end_as(
            state,
            deployment.id,
            "en",
            "manpower.deployment.unavailable",
        )
        .await?;
        return Ok(());
    }
    announce_cancelled(state, deployment, &cancel_reason(what)).await?;
    if deployment.start_communicated() {
        let worker = worker_name(state, "en", deployment.employee()).await;
        tell_client(
            state,
            deployment,
            "manpower.deployment.unavailable",
            &worker,
        )
        .await?;
    }
    Ok(())
}

/// The name of site `id`, empty when unknown.
async fn site_name(state: &AppState, id: i64) -> Result<String> {
    Ok(Site::find(&state.db, id)
        .await?
        .map(|site| site.name)
        .unwrap_or_default())
}

/// One message to the account manager of `contract_id` with what happened to
/// `deployments` and which requests stay unfilled.
async fn tell_account_manager(
    state: &AppState,
    worker: &str,
    what: &str,
    contract_id: i64,
    deployments: &[Deployment],
) -> Result {
    let contract = ClientContract::find_or_404(&state.db, contract_id).await?;
    let mut lines = Vec::new();
    for deployment in deployments {
        // Re-read: the end date is the last day.
        let deployment = Deployment::find_or_404(&state.db, deployment.id).await?;
        let site = site_name(state, deployment.site_id).await?;
        let until = deployment
            .end_date
            .map_or_else(|| "-".to_owned(), |end| end.to_string());
        let params: [(&str, &(dyn std::fmt::Display + Sync)); 4] = [
            ("site", &site),
            ("role", &deployment.role),
            ("from", &deployment.start_date),
            ("until", &until),
        ];
        lines.push(if deployment.status == DeploymentStatus::Ended {
            english("manpower.exit.line_ended", &params)
        } else {
            english("manpower.exit.line_cancelled", &params)
        });
        if let Some(request_id) = deployment.request_id
            && let Some(request) = ManpowerRequest::find(&state.db, request_id).await?
        {
            lines.push(english(
                "manpower.exit.line_request",
                &[
                    ("request", &request.id),
                    ("filled", &request.filled_count),
                    ("headcount", &request.headcount),
                ],
            ));
        }
    }
    let why = cancel_reason(what);
    let text = lines.join("\n");
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 4] = [
        ("worker", &worker),
        ("what", &why),
        ("contract", &contract.reference),
        ("lines", &text),
    ];
    let managers =
        notices::account_managers(state, contract.client(), contract.account_manager_user_id)
            .await?;
    notices::tell(
        state,
        contract.subject(),
        &contract.reference,
        &managers,
        &english("manpower.exit.staff_title", &params),
        &english("manpower.exit.staff_body", &params),
    )
    .await?;
    Ok(())
}

/// The deployments of `employee` that notice can change: not yet over.
async fn open_deployments(tx: &mut Transaction, employee: EmployeeId) -> Result<Vec<Deployment>> {
    Deployment::query()
        .where_(Deployment::EMPLOYEE_ID.eq(employee.0))
        .where_(Deployment::STATUS.is_in([
            DeploymentStatus::Planned,
            DeploymentStatus::Active,
            DeploymentStatus::Suspended,
        ]))
        .order_by(Deployment::ID)
        .lock_for_update()
        .get(&mut *tx)
        .await
}

/// `EmployeeNoticeGiven`: the end date of every deployment of `employee`
/// that runs past `last_day` (or has no end date) becomes `last_day`, and
/// the end date it replaced is kept. Returns the deployments changed.
///
/// # Errors
///
/// The database's errors.
pub async fn cap_at_notice(
    state: &AppState,
    employee: EmployeeId,
    last_day: NaiveDate,
) -> Result<usize> {
    let mut tx = state.db.begin().await?;
    let mut changed = 0;
    for deployment in open_deployments(&mut tx, employee).await? {
        if deployment.end_date.is_some_and(|end| end <= last_day) {
            continue;
        }
        // A notice already caps it: keep the end date from before the first.
        let before = if deployment.notice_last_day.is_some() {
            deployment.end_date_before_notice
        } else {
            deployment.end_date
        };
        set_deployment(
            &mut tx,
            deployment.id,
            &[
                ("end_date", &Some(last_day)),
                ("notice_last_day", &Some(last_day)),
                ("end_date_before_notice", &before),
            ],
        )
        .await?;
        changed += 1;
    }
    tx.commit().await?;
    Ok(changed)
}

/// `EmployeeNoticeWithdrawn`: the end date that the notice set goes back to
/// the one it replaced (empty if there was none) for every deployment still
/// running; the client and the account manager are told. A deployment whose
/// end date is no longer the one the notice set (it was extended) keeps it.
/// Returns the deployments restored.
///
/// # Errors
///
/// The database's and the notifier's errors.
pub async fn restore_after_notice(state: &AppState, employee: EmployeeId) -> Result<usize> {
    let mut tx = state.db.begin().await?;
    let mut restored = Vec::new();
    for deployment in open_deployments(&mut tx, employee).await? {
        let Some(capped) = deployment.notice_last_day else {
            continue;
        };
        let none: Option<NaiveDate> = None;
        let back = deployment.end_date == Some(capped);
        let end = if back {
            deployment.end_date_before_notice
        } else {
            deployment.end_date
        };
        set_deployment(
            &mut tx,
            deployment.id,
            &[
                ("end_date", &end),
                ("notice_last_day", &none),
                ("end_date_before_notice", &none),
            ],
        )
        .await?;
        if back {
            restored.push((deployment, capped));
        }
    }
    tx.commit().await?;
    let worker = worker_name(state, "en", employee).await;
    let mut by_contract: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for (before, capped) in &restored {
        let deployment = Deployment::find_or_404(&state.db, before.id).await?;
        tell_client(
            state,
            &deployment,
            "manpower.deployment.notice_withdrawn",
            &worker,
        )
        .await?;
        let site = site_name(state, deployment.site_id).await?;
        let until = deployment
            .end_date
            .map_or_else(|| "-".to_owned(), |end| end.to_string());
        by_contract
            .entry(deployment.client_contract_id)
            .or_default()
            .push(english(
                "manpower.exit.line_restored",
                &[
                    ("site", &site),
                    ("role", &deployment.role),
                    ("until", &until),
                    ("was", &capped),
                ],
            ));
    }
    for (contract_id, lines) in by_contract {
        let contract = ClientContract::find_or_404(&state.db, contract_id).await?;
        let text = lines.join("\n");
        let params: [(&str, &(dyn std::fmt::Display + Sync)); 3] = [
            ("worker", &worker),
            ("contract", &contract.reference),
            ("lines", &text),
        ];
        let managers =
            notices::account_managers(state, contract.client(), contract.account_manager_user_id)
                .await?;
        notices::tell(
            state,
            contract.subject(),
            &contract.reference,
            &managers,
            &english("manpower.exit.notice_restored_title", &params),
            &english("manpower.exit.notice_restored_body", &params),
        )
        .await?;
    }
    Ok(restored.len())
}

/// `VisaCaseRejected` for a renewal: the account manager of each contract and
/// the MD get one message listing the deployments of `employee` (`planned`,
/// `active`, `suspended`) that end after `expiry` or have no end date. No
/// deployment changes and the client is told nothing. Returns how many
/// deployments were listed.
///
/// # Errors
///
/// The database's and the notifier's errors.
pub async fn visa_rejected(
    state: &AppState,
    employee: EmployeeId,
    expiry: NaiveDate,
) -> Result<usize> {
    let affected: Vec<Deployment> = Deployment::query()
        .where_(Deployment::EMPLOYEE_ID.eq(employee.0))
        .where_(Deployment::STATUS.is_in([
            DeploymentStatus::Planned,
            DeploymentStatus::Active,
            DeploymentStatus::Suspended,
        ]))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|deployment| deployment.end_date.is_none_or(|end| end > expiry))
        .collect();
    if affected.is_empty() {
        return Ok(0);
    }
    let worker = worker_name(state, "en", employee).await;
    let mut for_user: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    let mut all = Vec::new();
    let mut first_client = None;
    for deployment in &affected {
        let contract =
            ClientContract::find_or_404(&state.db, deployment.client_contract_id).await?;
        first_client.get_or_insert(contract.client());
        let site = site_name(state, deployment.site_id).await?;
        let until = deployment
            .end_date
            .map_or_else(|| "-".to_owned(), |end| end.to_string());
        let status = format!("{:?}", deployment.status).to_lowercase();
        let line = english(
            "manpower.exit.line_visa",
            &[
                ("contract", &contract.reference),
                ("site", &site),
                ("role", &deployment.role),
                ("status", &status),
                ("until", &until),
            ],
        );
        let managers =
            notices::account_managers(state, contract.client(), contract.account_manager_user_id)
                .await?;
        for user in managers {
            for_user.entry(user).or_default().push(line.clone());
        }
        all.push(line);
    }
    if let Some(client) = first_client {
        for user in notices::md_users(state, client).await? {
            for_user.insert(user, all.clone());
        }
    }
    let subject = Subject::new(EMPLOYEE_SUBJECT_TYPE, employee.0);
    for (user, lines) in for_user {
        let text = lines.join("\n");
        let params: [(&str, &(dyn std::fmt::Display + Sync)); 3] =
            [("worker", &worker), ("expiry", &expiry), ("lines", &text)];
        notices::tell(
            state,
            subject.clone(),
            &worker,
            &[user],
            &english("manpower.exit.visa_title", &params),
            &english("manpower.exit.visa_body", &params),
        )
        .await?;
    }
    Ok(affected.len())
}

/// The key of the run: `MPW.RUL.12:exit:<what>:<employee>:<date>`.
fn key(what: &str, employee: EmployeeId, date: NaiveDate) -> String {
    format!("{RULE}:exit:{what}:{}:{date}", employee.0)
}

/// The action of one run.
async fn react(state: &AppState, what: &str, employee: EmployeeId, date: NaiveDate) -> Result {
    match what {
        what::ABSCONDED | what::DECEASED | what::EXITED | what::ONBOARDING_CANCELLED => {
            leave(state, employee, what, date).await?;
        }
        what::NOTICE_GIVEN => {
            cap_at_notice(state, employee, date).await?;
        }
        what::NOTICE_WITHDRAWN => {
            restore_after_notice(state, employee).await?;
        }
        what::VISA_REJECTED => {
            visa_rejected(state, employee, date).await?;
        }
        _ => {}
    }
    Ok(())
}

/// One compliance-critical run of `MPW.RUL.12` for the exit `what` of
/// `employee` on `date`: a second event with the same key changes nothing, a
/// failed run is retried by the 02:00 run.
///
/// # Errors
///
/// The kernel's errors; the action's own are kept in the run.
pub async fn run(state: &AppState, what: &str, employee: EmployeeId, date: NaiveDate) -> Result {
    let spec = RuleRunSpec::new(
        RULE,
        Subject::new(EMPLOYEE_SUBJECT_TYPE, employee.0),
        key(what, employee, date),
    )
    .compliance_critical();
    let what = what.to_owned();
    run_rule(state, spec, move |state: AppState| async move {
        react(&state, &what, employee, date).await
    })
    .await?;
    Ok(())
}

/// Tries again the run `what` of `employee` for `date` that failed and
/// waits (the 02:00 run).
pub(crate) async fn retry(state: &AppState, what: &str, employee: i64, date: &str) -> Result {
    let Ok(date) = date.parse::<NaiveDate>() else {
        return Ok(());
    };
    run(state, what, EmployeeId(employee), date).await
}

/// Listens for the exits.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: EmployeeAbsconded, state: AppState| async move {
        run(&state, what::ABSCONDED, event.employee, event.report_date).await
    });
    app.listen(|event: EmployeeDeceased, state: AppState| async move {
        run(&state, what::DECEASED, event.employee, event.date).await
    });
    app.listen(|event: EmployeeExited, state: AppState| async move {
        run(&state, what::EXITED, event.employee, event.exit_date).await
    });
    app.listen(
        |event: EmployeeOnboardingCancelled, state: AppState| async move {
            run(&state, what::ONBOARDING_CANCELLED, event.employee, today()).await
        },
    );
    app.listen(|event: EmployeeNoticeGiven, state: AppState| async move {
        run(
            &state,
            what::NOTICE_GIVEN,
            event.employee,
            event.last_working_day,
        )
        .await
    });
    app.listen(
        |event: EmployeeNoticeWithdrawn, state: AppState| async move {
            run(&state, what::NOTICE_WITHDRAWN, event.employee, today()).await
        },
    );
    app.listen(|event: VisaCaseRejected, state: AppState| async move {
        match (event.case_type.as_str(), event.visa_expiry) {
            ("renewal", Some(expiry)) => {
                run(&state, what::VISA_REJECTED, event.employee, expiry).await
            }
            _ => Ok(()),
        }
    });
}
