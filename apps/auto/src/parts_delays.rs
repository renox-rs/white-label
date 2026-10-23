//! `AUT.RUL.16`: the legal period to supply spare parts and the substitute
//! vehicle of a warranty repair (08 §2 consumer protection, §4.4 row *Parts do
//! not arrive*, §7.2, §9; BL-AUT-04). **Compliance-critical** (`PLT.RUL.04`).
//!
//! Every day at 08:00 Asia/Dubai, for the open job cards:
//!
//! - **Waiting for parts.** N = 2 days before the legal supply date of the wait
//!   ([`crate::supply`]), or on it, the service advisor is told, and so is the
//!   customer, with the new date: the expected arrival date entered on the part
//!   (the latest of the parts requested), or *we will confirm the date* and a
//!   task for the advisor when none was entered. A customer with no mobile
//!   number is not texted: the advisor gets a task to call. On the legal date
//!   the branch manager is told.
//! - **Warranty repair.** A job card linked to a ticket of category *warranty*
//!   that is not `ready` 7 days after check-in, in any state before `ready`, gets
//!   a task for the advisor to provide a substitute vehicle ([`crate::substitute`]),
//!   closed only by the substitute record or the customer's refusal with a
//!   reason.
//!
//! Each step fires once, per wait for parts and per job card for the
//! substitute. A late run fires every step whose day has passed and that has not
//! fired, in order (the date notices, then the manager's); a step whose wait
//! ended does not fire.
//!
//! The run is a kernel rule run per day (`RuleRun`, key `AUT.RUL.16:<day>`): a
//! run that fails is tried again with a backoff by [`retry`] and, after the
//! fifth failure, the Owner / GM and the company admin are alerted with an
//! owner task; a day with no run 60 minutes after 08:00 is alerted by the
//! kernel's check for missed runs.

use renox::auth::permissions::users_with_role;
use renox::chrono::{Duration, NaiveDate};
use renox::db::now;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::automation::{RuleRun, RuleRunSpec, RuleRunStatus, run_rule};
use wl_kernel::compliance::{CriticalRule, occurrence_key, schedule_subject};
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::notify::{
    self, Category, GROUP_BOOKINGS, GiveUpReaction, NotificationTypeDef, NotifyChannel,
    NotifyRequest, Urgency, notifier,
};
use wl_kernel::parties::directory;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::task_creator;
use wl_kernel::{ClientConfig, PartyId};

use crate::estimate::Estimate;
use crate::job_card::{JobCard, JobCardStatus};
use crate::labour_line::LineStatus;
use crate::parts_wait::PartsWait;
use crate::permissions;
use crate::substitute::{self, WARRANTY_CATEGORY};
use crate::texts::english;
use crate::vehicle::Vehicle;

/// The rule.
pub const RULE: &str = "AUT.RUL.16";
/// N: the days before the legal supply date the date notices go out.
pub const DAYS_BEFORE: i64 = 2;
/// The days after check-in a warranty repair gets a substitute vehicle.
pub const WARRANTY_DAYS: i64 = 7;
/// The scheduled task and command of the daily run.
pub const TASK: &str = "auto:parts-delays";
/// The time of day it runs, Asia/Dubai.
pub const TASK_AT: &str = "08:00";
/// The time zone of [`TASK_AT`].
pub const TASK_ZONE: &str = "Asia/Dubai";
/// The job and command that try the failed runs again.
pub const RETRY_JOB: &str = "auto:parts-delays-retry";
/// How often [`RETRY_JOB`] runs, in minutes.
pub const RETRY_EVERY_MINUTES: u64 = 15;
/// The notification type of the message to the customer.
pub const DELAY_TYPE: &str = "auto.parts_delay";
/// The notification type of the notices to staff.
pub const NOTICE_TYPE: &str = "auto.parts_notice";

/// The notification types: the customer's SMS (urgent; on give-up the contact
/// is marked invalid and the advisor gets a task) and the staff notices.
pub fn notification_types() -> Vec<NotificationTypeDef> {
    vec![
        NotificationTypeDef::new(DELAY_TYPE, "auto")
            .group(GROUP_BOOKINGS)
            .category(Category::Transactional)
            .urgency(Urgency::Urgent)
            .channels(&[NotifyChannel::Sms])
            .give_up(&[GiveUpReaction::MarkContactInvalid, GiveUpReaction::StaffTask])
            .staff_role(permissions::SERVICE_ADVISOR_ROLE)
            .template(
                NotifyChannel::Sms,
                "en",
                None,
                &english("auto.parts.sms_en", &[]),
            )
            .template(
                NotifyChannel::Sms,
                "ar",
                None,
                &english("auto.parts.sms_ar", &[]),
            ),
        NotificationTypeDef::new(NOTICE_TYPE, "auto")
            .group(GROUP_BOOKINGS)
            .category(Category::Transactional)
            .template(NotifyChannel::Email, "en", Some("{{ title }}"), "{{ body }}")
            .template(NotifyChannel::Email, "ar", Some("{{ title }}"), "{{ body }}"),
    ]
}

/// The workshop's name for a message.
fn workshop_name(state: &AppState) -> String {
    state
        .provided::<std::sync::Arc<ClientConfig>>()
        .map(|config| config.client.name.clone())
        .unwrap_or_default()
}

/// The users to tell about `card`: its advisor, else every service advisor.
async fn advisors(state: &AppState, card: &JobCard) -> Result<Vec<i64>> {
    if let Some(advisor) = card.advisor_user_id {
        return Ok(vec![advisor]);
    }
    Ok(users_with_role(&state.db, permissions::SERVICE_ADVISOR_ROLE)
        .await?
        .into_iter()
        .map(|user| user.id)
        .collect())
}

/// The date the parts are expected, as the purchaser entered it: the latest of
/// the parts still to come.
async fn expected_date(state: &AppState, card: &JobCard) -> Result<Option<NaiveDate>> {
    let estimate = Estimate::load(&state.db, card.id).await?;
    Ok(estimate
        .parts
        .iter()
        .filter(|line| line.approval == LineStatus::Approved && line.outstanding_milli() > 0)
        .filter_map(|line| line.expected_date)
        .max())
}

/// Tells `users` (a staff notice) `title` and `body` about `card`.
async fn tell_staff(
    state: &AppState,
    card: &JobCard,
    users: &[i64],
    title: &str,
    body: &str,
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(NOTICE_TYPE, card.subject())
        .subject_label(&card.number())
        .data(json!({ "title": title, "body": body }));
    for user in users {
        request = request.to_user(*user);
    }
    notifier(state).notify(state, request).await?;
    Ok(())
}

/// The steps of one wait for parts on `on`.
async fn step_wait(state: &AppState, wait: &PartsWait, on: NaiveDate) -> Result {
    let card = JobCard::find_or_404(&state.db, wait.job_card_id).await?;
    if card.status != JobCardStatus::AwaitingParts {
        return Ok(());
    }
    let vehicle = Vehicle::find_or_404(&state.db, card.vehicle_id).await?;
    let plate = vehicle.plate();
    let number = card.number();
    let date = wait.legal_date;
    if on >= date - Duration::days(DAYS_BEFORE) {
        if wait.advisor_notified_at.is_none() {
            let users = advisors(state, &card).await?;
            let (title, body) = {
                let params: [(&str, &dyn std::fmt::Display); 3] =
                    [("job", &number), ("plate", &plate), ("date", &date)];
                (
                    english("auto.parts.advisor_title", &params),
                    english("auto.parts.advisor_body", &params),
                )
            };
            tell_staff(state, &card, &users, &title, &body).await?;
            mark(state, wait.id, "advisor_notified_at").await?;
        }
        if wait.customer_notified_at.is_none() {
            tell_customer(state, &card, wait, &plate).await?;
            mark(state, wait.id, "customer_notified_at").await?;
        }
    }
    if on >= date && wait.legal_notified_at.is_none() {
        let managers: Vec<i64> = users_with_role(&state.db, permissions::BRANCH_MANAGER_ROLE)
            .await?
            .into_iter()
            .map(|user| user.id)
            .collect();
        let since = dubai_date(wait.started_at);
        let (title, body) = {
            let params: [(&str, &dyn std::fmt::Display); 4] = [
                ("job", &number),
                ("plate", &plate),
                ("date", &date),
                ("since", &since),
            ];
            (
                english("auto.parts.manager_title", &params),
                english("auto.parts.manager_body", &params),
            )
        };
        tell_staff(state, &card, &managers, &title, &body).await?;
        mark(state, wait.id, "legal_notified_at").await?;
    }
    Ok(())
}

/// Stamps the step `column` of wait `id` as done.
async fn mark(state: &AppState, id: i64, column: &str) -> Result {
    PartsWait::query()
        .where_(PartsWait::ID.eq(id))
        .update(&state.db, &[(column, &Some(now())), ("updated_at", &now())])
        .await?;
    Ok(())
}

/// Tells the customer the new date by SMS, or gives the advisor a task to call
/// when there is no mobile number; with no date entered the message says we
/// will confirm it and the advisor gets a task to find out.
async fn tell_customer(state: &AppState, card: &JobCard, wait: &PartsWait, plate: &str) -> Result {
    let expected = expected_date(state, card).await?;
    let number = card.number();
    let date_text = match expected {
        Some(date) => date.to_string(),
        None => english("auto.parts.date_unknown", &[]),
    };
    if expected.is_none() {
        let title = english("auto.parts.task_date", &[("job", &number)]);
        let task = RoleTask::new(
            card.subject(),
            permissions::SERVICE_ADVISOR_ROLE,
            &title,
            &format!("auto.parts_date:{}", wait.id),
        );
        role_tasks::open(state, task).await?;
    }
    let party = PartyId(card.customer_party_id);
    let has_mobile = directory(state)
        .contacts(&state.db, party)
        .await?
        .is_some_and(|contacts| contacts.point_for(NotifyChannel::Sms).is_some());
    if has_mobile {
        let request = NotifyRequest::new(DELAY_TYPE, card.subject())
            .subject_label(&number)
            .to_party(party)
            .data(json!({
                "workshop": workshop_name(state),
                "plate": plate,
                "job": number,
                "date": date_text,
            }));
        notifier(state).notify(state, request).await?;
    } else {
        let customer = directory(state)
            .display_name(&state.db, party, "en")
            .await?
            .unwrap_or_default();
        let title = {
            let params: [(&str, &dyn std::fmt::Display); 3] =
                [("customer", &customer), ("job", &number), ("date", &date_text)];
            english("auto.parts.task_call", &params)
        };
        let task = RoleTask::new(
            card.subject(),
            permissions::SERVICE_ADVISOR_ROLE,
            &title,
            &format!("auto.call_customer:{}", wait.id),
        );
        role_tasks::open(state, task).await?;
    }
    Ok(())
}

/// The warranty repair `card`, 7 days after check-in and not `ready`: the
/// advisor's task to provide a substitute vehicle, once.
async fn step_warranty(state: &AppState, card: &JobCard, on: NaiveDate) -> Result {
    let Some(checked_in) = card.created_at.map(dubai_date) else {
        return Ok(());
    };
    if on < checked_in + Duration::days(WARRANTY_DAYS) {
        return Ok(());
    }
    if substitute::Substitute::of_job_card(&state.db, card.id)
        .await?
        .is_some()
    {
        return Ok(());
    }
    let tickets = task_creator(state)
        .tickets_of(state, &card.subject(), WARRANTY_CATEGORY)
        .await?;
    if tickets.is_empty() {
        return Ok(());
    }
    let number = card.number();
    let (title, description) = {
        let params: [(&str, &dyn std::fmt::Display); 1] = [("job", &number)];
        (
            english("auto.substitute.task_title", &params),
            english("auto.substitute.task_body", &params),
        )
    };
    let task = RoleTask::new(
        card.subject(),
        permissions::SERVICE_ADVISOR_ROLE,
        &title,
        &substitute::task_key(card.id),
    )
    .description(&description);
    role_tasks::open(state, task).await?;
    substitute::request(state, card).await?;
    Ok(())
}

/// Every step of the day `on`; the first error is returned after all were
/// tried.
async fn process(state: &AppState, on: NaiveDate) -> Result {
    let mut failure = None;
    let waits = PartsWait::query()
        .where_(PartsWait::ENDED_AT.is_null())
        .order_by(PartsWait::ID)
        .get(&state.db)
        .await?;
    for wait in waits {
        if let Err(error) = step_wait(state, &wait, on).await {
            failure.get_or_insert(error);
        }
    }
    let open = JobCard::query()
        .where_(JobCard::STATUS.is_in([
            JobCardStatus::CheckedIn,
            JobCardStatus::Inspecting,
            JobCardStatus::AwaitingApproval,
            JobCardStatus::InProgress,
            JobCardStatus::AwaitingParts,
            JobCardStatus::QualityCheck,
        ]))
        .order_by(JobCard::ID)
        .get(&state.db)
        .await?;
    for card in open {
        if let Err(error) = step_warranty(state, &card, on).await {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// The run of the day `on`: one kernel rule run, tried again by [`retry`] when
/// it fails and, after the fifth failure, alerting the owner and the company
/// admin (`PLT.RUL.04`).
///
/// # Errors
///
/// The database's errors.
pub async fn run_daily(state: &AppState, on: NaiveDate) -> Result {
    let spec =
        RuleRunSpec::new(RULE, schedule_subject(), occurrence_key(RULE, on)).compliance_critical();
    run_rule(state, spec, move |state: AppState| async move {
        process(&state, on).await
    })
    .await?;
    Ok(())
}

/// Tries again the runs of the rule that failed and wait (the backoff of the
/// kernel decides when).
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
    for run in waiting {
        let Some(on) = run
            .occurrence_key
            .rsplit(':')
            .next()
            .and_then(|text| text.parse::<NaiveDate>().ok())
        else {
            continue;
        };
        if let Err(error) = run_daily(state, on).await {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// Marks `AUT.RUL.16` compliance-critical, owned by the Owner / GM and
/// expected every day at 08:00.
pub(crate) fn register_rule() {
    CriticalRule::new(RULE)
        .owner_role(permissions::OWNER_ROLE)
        .daily_at(TASK_AT)
        .register();
}

/// Registers the notification types, the rule, the daily run, the job that
/// tries the failed runs again, and their commands.
pub(crate) fn register(app: &mut Registry) {
    for def in notification_types() {
        notify::register_type(def);
    }
    register_rule();
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run_daily(&state, today()).await
        })
        .timezone(TASK_ZONE);
    app.schedule()
        .every_minutes(RETRY_EVERY_MINUTES, RETRY_JOB, |state: AppState| async move {
            retry(&state).await
        });
    app.command(
        TASK,
        "Tell the advisor, the customer and the branch manager about parts that are late, and ask for a substitute vehicle on warranty repairs",
        |_args, state: AppState| async move {
            run_daily(&state, today()).await?;
            println!("The parts delays are up to date.");
            Ok(())
        },
    );
    app.command(
        RETRY_JOB,
        "Try again the parts delay runs that failed",
        |_args, state: AppState| async move {
            retry(&state).await?;
            println!("The failed runs were tried again.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_is_compliance_critical_and_runs_at_eight() {
        register_rule();
        assert!(wl_kernel::compliance::is_critical(RULE));
        assert_eq!((TASK_AT, TASK_ZONE), ("08:00", "Asia/Dubai"));
        assert_eq!((DAYS_BEFORE, WARRANTY_DAYS), (2, 7));
    }

    #[test]
    fn the_customer_type_is_urgent_and_the_advisor_hears_of_a_give_up() {
        let types = notification_types();
        assert_eq!(types[0].urgency, Urgency::Urgent);
        assert_eq!(
            types[0].staff_role.as_deref(),
            Some(permissions::SERVICE_ADVISOR_ROLE)
        );
    }
}
