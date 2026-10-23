//! `AUT.RUL.15`: export sales that do not leave in time (08 §2 re-export, §4.1
//! *When it goes wrong*, §7.2, §9; BL-AUT-14). **Compliance-critical**
//! (`PLT.RUL.04`).
//!
//! Every day at 08:00 Asia/Dubai, for each export sale with **no** exit
//! evidence (a customs exit certificate, a bill of lading or an airway bill):
//!
//! - **60 days** after the supply date the accountant is told;
//! - **90 days** after it the sale is flagged *5% VAT due* and the accountant
//!   gets a task linking the invoice, to issue a credit note and a new invoice
//!   at 5% by hand (recharging the VAT to the customer where the sale terms
//!   allow, otherwise absorbing it). **No document is issued automatically.**
//!
//! Days are calendar days (Asia/Dubai) from the supply date; each step fires
//! once; a late run fires every step whose day has passed and that has not
//! fired, in order. Evidence attached before a step stops the rule for the
//! sale; evidence after the 90 days leaves the flag and tells the accountant
//! ([`tell_late_evidence`]); nothing is reversed (open item A-12).
//!
//! The run is a kernel rule run per day (`RuleRun`, key `AUT.RUL.15:<day>`): a
//! run that fails is tried again with a backoff by [`retry`] and, after the
//! fifth failure, the Owner / GM and the company admin are alerted with an
//! owner task; a day with no run 60 minutes after 08:00 is alerted by the
//! kernel's check for missed runs. A successful rerun closes the alert.

use renox::auth::permissions::users_with_role;
use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::COMPANY_ADMIN_ROLE;
use wl_kernel::automation::{RuleRun, RuleRunSpec, RuleRunStatus, run_rule};
use wl_kernel::compliance::{CriticalRule, occurrence_key, schedule_subject};
use wl_kernel::documents::today;
use wl_kernel::notify::{
    self, Category, GROUP_INVOICES, GiveUpReaction, NotificationTypeDef, NotifyChannel,
    NotifyRequest, notifier,
};
use wl_kernel::role_tasks::{self, RoleTask};

use crate::export_sale::ExportSale;
use crate::permissions;
use crate::texts::english;

/// The rule.
pub const RULE: &str = "AUT.RUL.15";
/// N: the days after the supply date the accountant is told.
pub const NOTICE_DAYS: i64 = 60;
/// N: the days after the supply date the sale is flagged and the task opened.
pub const VAT_DAYS: i64 = 90;
/// The scheduled task and command of the daily run.
pub const TASK: &str = "auto:export-evidence";
/// The time of day it runs, Asia/Dubai.
pub const TASK_AT: &str = "08:00";
/// The time zone of [`TASK_AT`].
pub const TASK_ZONE: &str = "Asia/Dubai";
/// The job and command that try the failed runs again.
pub const RETRY_JOB: &str = "auto:export-evidence-retry";
/// How often [`RETRY_JOB`] runs, in minutes.
pub const RETRY_EVERY_MINUTES: u64 = 15;
/// The notification type of the notices to the accountant.
pub const NOTICE_TYPE: &str = "auto.export_notice";

/// The notification type of the staff notices: an email to the accountant; a
/// message that gives up gives the accountant role a task.
pub fn notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NOTICE_TYPE, "auto")
        .group(GROUP_INVOICES)
        .category(Category::Transactional)
        .give_up(&[GiveUpReaction::StaffTask])
        .staff_role(permissions::ACCOUNTANT_ROLE)
        .channels(&[NotifyChannel::Email])
        .template(
            NotifyChannel::Email,
            "en",
            Some("{{ title }}"),
            "{{ body }}",
        )
        .template(
            NotifyChannel::Email,
            "ar",
            Some("{{ title }}"),
            "{{ body }}",
        )
}

/// The users to tell: the accountants, else the company admins.
async fn accountants(state: &AppState) -> Result<Vec<i64>> {
    for role in [permissions::ACCOUNTANT_ROLE, COMPANY_ADMIN_ROLE] {
        let users = users_with_role(&state.db, role).await?;
        if !users.is_empty() {
            return Ok(users.into_iter().map(|user| user.id).collect());
        }
    }
    Ok(Vec::new())
}

/// Tells the accountants `title` and `body` about `sale`.
async fn tell(state: &AppState, sale: &ExportSale, title: &str, body: &str) -> Result {
    let users = accountants(state).await?;
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(NOTICE_TYPE, sale.subject())
        .subject_label(&sale.number())
        .data(json!({ "title": title, "body": body }));
    for user in users {
        request = request.to_user(user);
    }
    notifier(state).notify(state, request).await?;
    Ok(())
}

/// The words of a notice about `sale`: the invoice's number, the customer's
/// id is not shown, the supply date.
fn words(sale: &ExportSale, title_key: &str, body_key: &str) -> (String, String) {
    let number = sale.number();
    let supplied = sale.supply_date;
    let params: [(&str, &dyn std::fmt::Display); 2] = [("invoice", &number), ("date", &supplied)];
    (english(title_key, &params), english(body_key, &params))
}

/// Stamps `column` of sale `id` as done.
async fn mark(state: &AppState, id: i64, column: &str) -> Result {
    ExportSale::query()
        .where_(ExportSale::ID.eq(id))
        .update(&state.db, &[(column, &Some(now())), ("updated_at", &now())])
        .await?;
    Ok(())
}

/// The steps of one sale on `on`.
async fn step(state: &AppState, sale: &ExportSale, on: NaiveDate) -> Result {
    if !sale.evidence(&state.db).await?.is_empty() {
        return Ok(());
    }
    let days = sale.days_since_supply(on);
    if days >= NOTICE_DAYS && sale.notified_at.is_none() {
        let (title, body) = words(sale, "auto.export.notice_title", "auto.export.notice_body");
        tell(state, sale, &title, &body).await?;
        mark(state, sale.id, "notified_at").await?;
    }
    if days >= VAT_DAYS && sale.vat_flagged_at.is_none() {
        let (title, body) = words(sale, "auto.export.task_title", "auto.export.task_body");
        let task = RoleTask::new(
            sale.invoice_subject(),
            permissions::ACCOUNTANT_ROLE,
            &title,
            &task_key(sale.id),
        )
        .description(&body);
        role_tasks::open(state, task).await?;
        mark(state, sale.id, "vat_flagged_at").await?;
    }
    Ok(())
}

/// The key the correction task of sale `id` is found by while it is open.
pub fn task_key(id: i64) -> String {
    format!("auto.export_vat:{id}")
}

/// Every step of the day `on`; the first error is returned after all were
/// tried.
async fn process(state: &AppState, on: NaiveDate) -> Result {
    let mut failure = None;
    let sales = ExportSale::query()
        .order_by(ExportSale::ID)
        .get(&state.db)
        .await?;
    for sale in sales {
        if sale.notified_at.is_some() && sale.vat_flagged_at.is_some() {
            continue;
        }
        if let Err(error) = step(state, &sale, on).await {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// The accountant is told that evidence came for a sale already flagged: the
/// flag stays and nothing is reversed (open item A-12). Once.
///
/// # Errors
///
/// The notifier's and the database's errors.
pub async fn tell_late_evidence(state: &AppState, sale: &ExportSale, kind: &str) -> Result {
    let kind_label = english(&format!("auto.export.kind.{kind}"), &[]);
    let number = sale.number();
    let (title, body) = {
        let params: [(&str, &dyn std::fmt::Display); 2] =
            [("invoice", &number), ("kind", &kind_label)];
        (
            english("auto.export.late_title", &params),
            english("auto.export.late_body", &params),
        )
    };
    tell(state, sale, &title, &body).await?;
    mark(state, sale.id, "late_evidence_notified_at").await
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

/// Marks `AUT.RUL.15` compliance-critical, owned by the Owner / GM and
/// expected every day at 08:00.
pub(crate) fn register_rule() {
    CriticalRule::new(RULE)
        .owner_role(permissions::OWNER_ROLE)
        .daily_at(TASK_AT)
        .register();
}

/// Registers the notification type, the rule, the daily run, the job that tries
/// the failed runs again, and their commands.
pub(crate) fn register(app: &mut Registry) {
    notify::register_type(notice_type());
    register_rule();
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run_daily(&state, today()).await
        })
        .timezone(TASK_ZONE);
    app.schedule().every_minutes(
        RETRY_EVERY_MINUTES,
        RETRY_JOB,
        |state: AppState| async move { retry(&state).await },
    );
    app.command(
        TASK,
        "Tell the accountant about export sales without exit evidence at 60 days and flag them at 90 days",
        |_args, state: AppState| async move {
            run_daily(&state, today()).await?;
            println!("The export evidence is up to date.");
            Ok(())
        },
    );
    app.command(
        RETRY_JOB,
        "Try again the export evidence runs that failed",
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
        assert_eq!((NOTICE_DAYS, VAT_DAYS), (60, 90));
    }

    #[test]
    fn the_notice_is_an_email_to_the_accountant() {
        let def = notice_type();
        assert_eq!(def.channels, [NotifyChannel::Email]);
        assert_eq!(def.staff_role.as_deref(), Some("accountant"));
    }

    #[test]
    fn the_task_is_found_by_the_sale() {
        assert_eq!(task_key(4), "auto.export_vat:4");
    }
}
