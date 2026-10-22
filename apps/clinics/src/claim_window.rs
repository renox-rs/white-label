//! `CLN.RUL.03`: the first-submission window is about to pass (05 §4.3 "When
//! it goes wrong", §7.2; BL-CLN-11).
//!
//! A claim left in `draft` or `ready` is chased twice, by one daily run
//! ([`run`], 02:00 Asia/Dubai, also the command [`TASK`]):
//!
//! - **N days after the visit** the billing coordinator is told (a message
//!   per coordinator, type [`NOTICE`]). N is the payer contract's
//!   `reminder_days` (7), always below its first-submission window.
//! - **D days before the window ends** (D = 2, [`crate::contracts::TASK_DAYS_BEFORE`])
//!   the clinic manager gets a task (a staff task for the holder of the
//!   clinic manager role, key `clinics.claim_window:<claim>`, due at the end
//!   of the last day).
//!
//! The rule is **compliance-critical**: the run is one kernel rule run per
//! date, so a run that fails five times alerts the owner / medical director
//! (`PLT.RUL.04`), and so does a date with no run at all (the kernel's
//! missed-run check, 60 minutes after 02:00). It is registered here, not by
//! the client, so the client cannot switch it off.
//!
//! Decisions:
//!
//! - A step fires on the **first run on or after its day**, once per claim:
//!   the claim keeps the day each step fired (`window_notified_on`,
//!   `window_task_on`), so a rerun skips it and a run that failed half way
//!   does not tell anybody twice. Both steps stop after the last day of the
//!   window: a claim past it is closed by `write_off` (`CLN.ACL.16`).
//! - A window shorter than D (the 24-hour rule): the task is due on the visit
//!   date, so it fires on the first run after the claim exists. With N = 0
//!   the coordinator is told at the same run.
//! - A claim that is back in `draft` after a technical rejection of a
//!   resubmission is counted: the task counts D days before its
//!   *resubmission* window ends; the coordinator's message (N) is for the
//!   first submission only.
//! - A claim without a payer contract has no window and is not chased.
//! - The coordinators are the users holding the billing coordinator role;
//!   when nobody holds it, the company administrators.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::chrono::{Days, NaiveDate};
use renox::db::now;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::COMPANY_ADMIN_ROLE;
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::compliance::{CriticalRule, occurrence_key, schedule_subject};
use wl_kernel::documents::today;
use wl_kernel::notify::{
    self, GROUP_DOCUMENTS, NotificationTypeDef, NotifyChannel, NotifyRequest, notifier,
};
use wl_kernel::role_tasks::{self, RoleTask};

use crate::claim::Claim;
use crate::claim_rules::end_of;
use crate::claims::attempt_deadline;
use crate::contract::PayerContract;
use crate::contracts::TASK_DAYS_BEFORE;
use crate::permissions::{BILLING_COORDINATOR_ROLE, CLINIC_MANAGER_ROLE, OWNER_ROLE};
use crate::texts::english;

/// The rule.
pub const RULE: &str = "CLN.RUL.03";
/// The scheduled task, and the command, of the daily run.
pub const TASK: &str = "clinics:claim_windows";
/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "02:00";
/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";
/// The notification type of the message to the billing coordinator.
pub const NOTICE: &str = "clinics.claim_window";

/// What a run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    /// Claims whose coordinator was told (N).
    pub notified: usize,
    /// Claims the clinic manager got a task for (D).
    pub tasked: usize,
}

/// The source key of the clinic manager's task for claim `claim`.
pub fn task_key(claim: i64) -> String {
    format!("clinics.claim_window:{claim}")
}

/// The first day the clinic manager gets the task for `claim` under
/// `contract`: D days before the window of the attempt being prepared ends,
/// but not before the visit.
pub fn task_date(claim: &Claim, contract: &PayerContract) -> Option<NaiveDate> {
    if claim.resubmission_count == 0 {
        contract.task_date(claim.visit_date, TASK_DAYS_BEFORE)
    } else {
        attempt_deadline(claim, Some(contract))?
            .checked_sub_days(Days::new(u64::try_from(TASK_DAYS_BEFORE).ok()?))
    }
}

/// The users to tell: the billing coordinators, else the company
/// administrators, by id.
async fn coordinators(state: &AppState) -> Result<Vec<i64>> {
    let mut users = BTreeSet::new();
    for user in users_with_role_in(&state.db, BILLING_COORDINATOR_ROLE, &Scope::global()).await? {
        users.insert(user.id);
    }
    if users.is_empty() {
        for user in users_with_role_in(&state.db, COMPANY_ADMIN_ROLE, &Scope::global()).await? {
            users.insert(user.id);
        }
    }
    Ok(users.into_iter().collect())
}

/// Takes the step of claim `id` for `day`: sets the day it fired when no run
/// did before. Returns whether this run took it.
async fn take_notice(state: &AppState, id: i64, day: NaiveDate) -> Result<bool> {
    let taken = Claim::query()
        .where_(Claim::ID.eq(id))
        .where_(Claim::WINDOW_NOTIFIED_ON.is_null())
        .update(
            &state.db,
            &[("window_notified_on", &Some(day)), ("updated_at", &now())],
        )
        .await?;
    Ok(taken > 0)
}

/// Like [`take_notice`] for the clinic manager's task.
async fn take_task(state: &AppState, id: i64, day: NaiveDate) -> Result<bool> {
    let taken = Claim::query()
        .where_(Claim::ID.eq(id))
        .where_(Claim::WINDOW_TASK_ON.is_null())
        .update(
            &state.db,
            &[("window_task_on", &Some(day)), ("updated_at", &now())],
        )
        .await?;
    Ok(taken > 0)
}

/// Gives the step back: nobody was told, or no task could be opened, so the
/// next run does it.
async fn give_back(state: &AppState, id: i64, column: &str) -> Result {
    Claim::query()
        .where_(Claim::ID.eq(id))
        .update(
            &state.db,
            &[(column, &None::<NaiveDate>), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}

/// Tells the coordinators that `claim` is still not sent (N).
async fn tell(state: &AppState, claim: &Claim, deadline: NaiveDate, day: NaiveDate) -> Result<()> {
    let status = english(
        &format!("clinics.claim.status.{}", claim.status.as_str()),
        &[],
    );
    let days_left = (deadline - day).num_days();
    let users = coordinators(state).await?;
    if users.is_empty() {
        return Ok(());
    }
    // One request for all of them, so that a failure never leaves some told
    // and the step open for the next run to tell them again.
    let mut request = NotifyRequest::new(NOTICE, claim.subject())
        .subject_label(&claim.number())
        .data(json!({
            "number": claim.number(),
            "visit_date": claim.visit_date.to_string(),
            "status": status,
            "deadline": deadline.to_string(),
            "days_left": days_left,
        }));
    for user in users {
        request = request.to_user(user);
    }
    notifier(state).notify(state, request).await?;
    Ok(())
}

/// Opens the clinic manager's task for `claim` (D). Returns whether a task
/// was opened (not when ticketing is off, or it exists).
async fn open_task(state: &AppState, claim: &Claim, deadline: NaiveDate) -> Result<bool> {
    let number = claim.number();
    let status = english(
        &format!("clinics.claim.status.{}", claim.status.as_str()),
        &[],
    );
    let deadline_text = deadline.to_string();
    let visit_date = claim.visit_date.to_string();
    let title = english(
        "clinics.window.task_title",
        &[("number", &number), ("deadline", &deadline_text)],
    );
    let description = english(
        "clinics.window.task_description",
        &[
            ("number", &number),
            ("visit_date", &visit_date),
            ("status", &status),
            ("deadline", &deadline_text),
        ],
    );
    let mut task = RoleTask::new(
        claim.subject(),
        CLINIC_MANAGER_ROLE,
        &title,
        &task_key(claim.id),
    )
    .description(&description);
    if let Some(due) = end_of(deadline) {
        task = task.due_at(due);
    }
    Ok(role_tasks::open(state, task).await?.is_some())
}

/// The steps due for `claim` on `day`; counts them in `report`.
async fn chase(state: &AppState, claim: &Claim, day: NaiveDate, report: &mut Report) -> Result<()> {
    let Some(contract_id) = claim.contract_id else {
        return Ok(());
    };
    let Some(contract) = PayerContract::find(&state.db, contract_id).await? else {
        return Ok(());
    };
    let Some(deadline) = attempt_deadline(claim, Some(&contract)) else {
        return Ok(());
    };
    if day > deadline {
        return Ok(());
    }
    // Decision: the step is taken before anybody is told and given back
    // when telling fails, so that two runs never tell twice and a failure
    // is retried; the same for the task, which is also given back when
    // ticketing is off (no task could be opened), so that it opens once
    // ticketing is on.
    if claim.resubmission_count == 0
        && claim.window_notified_on.is_none()
        && contract
            .reminder_date(claim.visit_date)
            .is_some_and(|due| day >= due)
        && take_notice(state, claim.id, day).await?
    {
        if let Err(error) = tell(state, claim, deadline, day).await {
            give_back(state, claim.id, "window_notified_on").await?;
            return Err(error);
        }
        report.notified += 1;
    }
    if claim.window_task_on.is_none()
        && task_date(claim, &contract).is_some_and(|due| day >= due)
        && take_task(state, claim.id, day).await?
    {
        match open_task(state, claim, deadline).await {
            Ok(true) => report.tasked += 1,
            Ok(false) => give_back(state, claim.id, "window_task_on").await?,
            Err(error) => {
                give_back(state, claim.id, "window_task_on").await?;
                return Err(error);
            }
        }
    }
    Ok(())
}

/// The steps of every claim still a `draft` or `ready` on `day`. A claim that
/// fails does not stop the others; the first failure is returned at the end.
async fn steps(state: &AppState, day: NaiveDate, report: &mut Report) -> Result<()> {
    let all = Claim::query().order_by(Claim::ID).get(&state.db).await?;
    let mut failure = None;
    for claim in all.iter().filter(|c| c.status.is_editable()) {
        if let Err(error) = chase(state, claim, day, report).await {
            eprintln!("{TASK}: claim {}: {error:?}", claim.id);
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// Runs `CLN.RUL.03` for `day` (Asia/Dubai) as one compliance-critical rule
/// run: a second run for the same date does nothing, a failed one is retried,
/// and the fifth failure alerts the owner role (`PLT.RUL.04`).
///
/// # Errors
///
/// The database's errors.
pub async fn run(state: &AppState, day: NaiveDate) -> Result<Report> {
    let spec =
        RuleRunSpec::new(RULE, schedule_subject(), occurrence_key(RULE, day)).compliance_critical();
    let seen = Arc::new(Mutex::new(Report::default()));
    let out = Arc::clone(&seen);
    run_rule(state, spec, move |state: AppState| async move {
        let mut report = Report::default();
        let result = steps(&state, day, &mut report).await;
        *out.lock().unwrap_or_else(PoisonError::into_inner) = report;
        result
    })
    .await?;
    let report = *seen.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(report)
}

/// The notification type of the message.
pub fn notification_types() -> Vec<NotificationTypeDef> {
    let subject = english("clinics.window.subject", &[]);
    let body = english("clinics.window.body", &[]);
    vec![
        NotificationTypeDef::new(NOTICE, "clinics")
            .group(GROUP_DOCUMENTS)
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
    ]
}

/// Registers the notification type, the compliance-critical rule owned by the
/// owner / medical director, the daily task and its command.
pub(crate) fn register(app: &mut Registry) {
    for def in notification_types() {
        notify::register_type(def);
    }
    CriticalRule::new(RULE)
        .owner_role(OWNER_ROLE)
        .daily_at(TASK_AT)
        .register();
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Chase the claims still draft or ready before the payer's first-submission window ends (--date YYYY-MM-DD checks another day)",
        |args, state: AppState| async move {
            let on = match args.value("--date") {
                None => today(),
                Some(text) => text.parse().map_err(|_| {
                    Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("--date must be YYYY-MM-DD, got `{text}`"),
                    )
                })?,
            };
            let report = run(&state, on).await?;
            println!(
                "{} coordinators told, {} tasks opened.",
                report.notified, report.tasked
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    fn contract(window: i64, reminder: i64) -> PayerContract {
        PayerContract {
            first_submission_days: window,
            resubmission_days: 30,
            reminder_days: reminder,
            ..PayerContract::default()
        }
    }

    #[test]
    fn the_task_is_due_two_days_before_the_window_ends() {
        let claim = Claim {
            visit_date: day("2026-11-01"),
            ..Claim::default()
        };
        assert_eq!(task_date(&claim, &contract(30, 7)), Some(day("2026-11-29")));
        // A window shorter than D: due on the visit date.
        assert_eq!(task_date(&claim, &contract(1, 0)), Some(day("2026-11-01")));
    }

    #[test]
    fn a_resubmitted_claim_counts_from_its_resubmission_window() {
        let claim = Claim {
            visit_date: day("2026-11-01"),
            resubmission_count: 1,
            last_remittance_on: Some(day("2026-12-10")),
            ..Claim::default()
        };
        assert_eq!(task_date(&claim, &contract(30, 7)), Some(day("2027-01-07")));
    }

    #[test]
    fn the_task_key_names_the_claim() {
        assert_eq!(task_key(101), "clinics.claim_window:101");
    }
}
