//! Vehicles that are not collected (08 §4.4 row *The vehicle is not
//! collected*, §7.2 `AUT.RUL.18`, §9; BL-AUT-05).
//!
//! A vehicle that is `ready`, `declined` or `cancelled` and not released blocks
//! a bay. Every day at 08:00 Asia/Dubai ([`run_daily`]):
//!
//! - **Day 3 and day 7** after the job card entered the state (`ready_at`, or
//!   `closed_at` for a declined or cancelled one) the customer is asked to
//!   collect it by SMS (`auto.collect_vehicle`: the plate, the job card, the
//!   days, the storage fee a day when it applies, the workshop's address and
//!   phone); a customer with no mobile number gives the advisor a task to
//!   call. A job card cancelled for a total loss gets the reminders too.
//! - **Day 30** the branch manager gets a task *legal next steps open (L-36)*:
//!   what a workshop may do with an abandoned vehicle is an open item for the
//!   lawyer and the app offers no sale, disposal or lien action.
//!
//! Each step fires once, in order after a late run, and none fires for a
//! vehicle released meanwhile.
//!
//! **The storage fee** is charged only when the client turned it on
//! ([`crate::settings::STORAGE_FEE_ENABLED`], off by default) and the customer
//! agreed a daily rate at check-in (`storage_fee_daily`), never for a job card
//! cancelled for a total loss (storage is billed through the claim, BL-AUT-09).
//! It is one day's fee for each calendar day from the day the first reminder
//! fell due ([`JobCard::collect_first_on`]) up to the day before the vehicle is
//! released or delivered, invoiced then ([`bill_release`]) on an invoice of its
//! own.
//!
//! Decisions: the first reminder's due day, not the day a late run sent it,
//! starts the fee, so the fee does not depend on when the run happened.
//! Whether a fee needs more than the customer's agreement at check-in is
//! `⚠ open: lawyer (L-36)`.

use renox::chrono::{Duration, NaiveDate};
use renox::db::{Col, DateTime, now};
use renox::prelude::*;
use renox::serde_json::json;
use std::sync::Arc;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceId, VatCategory, invoicer};
use wl_kernel::notify::{NotifyChannel, NotifyRequest, notifier};
use wl_kernel::parties::directory;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::{ClientConfig, PartyId};

use crate::declines::COLLECT_TYPE;
use crate::job_card::{JobCard, JobCardStatus};
use crate::permissions;
use crate::settings;
use crate::stop::TOTAL_LOSS;
use crate::texts::english;
use crate::vehicle::Vehicle;

/// The rule.
pub const RULE: &str = "AUT.RUL.18";
/// N: the days after which the customer is reminded.
pub const REMINDER_DAYS: [i64; 2] = [3, 7];
/// M: the day the branch manager gets a task.
pub const MANAGER_DAYS: i64 = 30;
/// The scheduled task and command of the daily run.
pub const TASK: &str = "auto:uncollected";
/// The time of day it runs, Asia/Dubai.
pub const TASK_AT: &str = "08:00";
/// The time zone of [`TASK_AT`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The day the wait of `card` began: it became `ready`, or was declined or
/// cancelled. `None` for a job card in another state.
pub fn start_of(card: &JobCard) -> Option<NaiveDate> {
    match card.status {
        JobCardStatus::Ready => card.ready_at.map(dubai_date),
        JobCardStatus::Declined | JobCardStatus::Cancelled => card.closed_at.map(dubai_date),
        _ => None,
    }
}

/// Whether the storage fee applies to `card`: the client turned it on, the
/// customer agreed a daily rate, and it was not cancelled for a total loss.
///
/// # Errors
///
/// The database's errors.
pub async fn fee_applies(db: &Db, card: &JobCard) -> Result<bool> {
    Ok(card.storage_fee_daily > 0
        && card.closed_code.as_deref() != Some(TOTAL_LOSS)
        && !card.internal
        && settings::storage_fee_enabled(db).await?)
}

/// The days of storage of `card` when it leaves on `leaves`: from the first
/// reminder's day up to the day before, 0 when no reminder fell due.
pub fn days_charged(card: &JobCard, leaves: NaiveDate) -> i64 {
    card.collect_first_on
        .map_or(0, |first| (leaves - first).num_days().max(0))
}

/// The workshop's name, address and phone for a message.
fn workshop_contact(state: &AppState) -> (String, String, String) {
    let Some(config) = state.provided::<Arc<ClientConfig>>() else {
        return (String::new(), String::new(), String::new());
    };
    let address = config
        .legal_entities
        .iter()
        .find_map(|entity| entity.address_en.clone())
        .unwrap_or_default();
    let phone = config
        .section("auto")
        .and_then(|section| section.get("phone"))
        .and_then(|phone| phone.as_str())
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    (config.client.name.clone(), address, phone)
}

/// The reminder number `step` (1 or 2) for `card`, `days` days after it began.
async fn remind(state: &AppState, card: &JobCard, step: usize, days: i64) -> Result {
    let vehicle = Vehicle::find_or_404(&state.db, card.vehicle_id).await?;
    let plate = vehicle.plate();
    let number = card.number();
    let applies = fee_applies(&state.db, card).await?;
    let party = PartyId(card.customer_party_id);
    let has_mobile = directory(state)
        .contacts(&state.db, party)
        .await?
        .is_some_and(|contacts| contacts.point_for(NotifyChannel::Sms).is_some());
    if has_mobile {
        let (workshop, address, phone) = workshop_contact(state);
        let reason = {
            let key = match card.status {
                JobCardStatus::Ready => "auto.storage.reason_ready",
                JobCardStatus::Declined => "auto.storage.reason_declined",
                _ => "auto.storage.reason_cancelled",
            };
            english(key, &[("days", &days)])
        };
        let fee = if applies {
            let amount = crate::money_text(card.storage_fee_daily);
            english("auto.storage.fee_sms", &[("amount", &amount)])
        } else {
            String::new()
        };
        let request = NotifyRequest::new(COLLECT_TYPE, card.subject())
            .subject_label(&number)
            .to_party(party)
            .data(json!({
                "plate": plate,
                "job": number,
                "reason": reason,
                "fee": fee,
                "workshop": workshop,
                "address": address,
                "phone": phone,
            }));
        notifier(state).notify(state, request).await?;
    } else {
        let customer = directory(state)
            .display_name(&state.db, party, "en")
            .await?
            .unwrap_or_default();
        let title = {
            let params: [(&str, &dyn std::fmt::Display); 3] =
                [("customer", &customer), ("job", &number), ("days", &days)];
            english("auto.storage.task_call", &params)
        };
        let task = RoleTask::new(
            card.subject(),
            permissions::SERVICE_ADVISOR_ROLE,
            &title,
            &format!("auto.collect_call:{}:{step}", card.id),
        );
        role_tasks::open(state, task).await?;
    }
    Ok(())
}

/// The task for the branch manager at day 30.
async fn tell_manager(state: &AppState, card: &JobCard, days: i64) -> Result {
    let number = card.number();
    let (title, body) = {
        let params: [(&str, &dyn std::fmt::Display); 2] = [("job", &number), ("days", &days)];
        (
            english("auto.storage.manager_title", &params),
            english("auto.storage.manager_body", &params),
        )
    };
    let task = RoleTask::new(
        card.subject(),
        permissions::BRANCH_MANAGER_ROLE,
        &title,
        &format!("auto.uncollected:{}", card.id),
    )
    .description(&body);
    role_tasks::open(state, task).await?;
    Ok(())
}

/// Claims a step of job card `id`: stamps `column` with `day` only if it is
/// still empty, and tells whether this call did. The step is claimed *before*
/// its message goes out, so a failure after the message cannot send it twice.
async fn claim(
    state: &AppState,
    id: i64,
    when: Col<JobCard, Option<NaiveDate>>,
    column: &str,
    day: NaiveDate,
) -> Result<bool> {
    let claimed = JobCard::query()
        .where_(JobCard::ID.eq(id))
        .where_(when.is_null())
        .update(&state.db, &[(column, &Some(day)), ("updated_at", &now())])
        .await?;
    Ok(claimed > 0)
}

/// Gives a claimed step back, because its message could not be sent: the next
/// run does it again.
async fn unclaim(state: &AppState, id: i64, column: &str) -> Result {
    let none: Option<NaiveDate> = None;
    JobCard::query()
        .where_(JobCard::ID.eq(id))
        .update(&state.db, &[(column, &none), ("updated_at", &now())])
        .await?;
    Ok(())
}

/// The steps of one job card on `on`. Each is claimed first, then done; a step
/// that fails is given back so the next run tries it again.
async fn step_card(state: &AppState, card: &JobCard, on: NaiveDate) -> Result {
    let Some(start) = start_of(card) else {
        return Ok(());
    };
    if card.released_at.is_some() {
        return Ok(());
    }
    let days = (on - start).num_days();
    if days >= REMINDER_DAYS[0] && card.collect_first_on.is_none() {
        let day = start + Duration::days(REMINDER_DAYS[0]);
        let column = "collect_first_on";
        if claim(state, card.id, JobCard::COLLECT_FIRST_ON, column, day).await? {
            if let Err(error) = remind(state, card, 1, REMINDER_DAYS[0]).await {
                unclaim(state, card.id, column).await?;
                return Err(error);
            }
        }
    }
    if days >= REMINDER_DAYS[1] && card.collect_second_on.is_none() {
        let day = start + Duration::days(REMINDER_DAYS[1]);
        let column = "collect_second_on";
        if claim(state, card.id, JobCard::COLLECT_SECOND_ON, column, day).await? {
            if let Err(error) = remind(state, card, 2, REMINDER_DAYS[1]).await {
                unclaim(state, card.id, column).await?;
                return Err(error);
            }
        }
    }
    if days >= MANAGER_DAYS && card.collect_manager_on.is_none() {
        let day = start + Duration::days(MANAGER_DAYS);
        let column = "collect_manager_on";
        if claim(state, card.id, JobCard::COLLECT_MANAGER_ON, column, day).await? {
            if let Err(error) = tell_manager(state, card, MANAGER_DAYS).await {
                unclaim(state, card.id, column).await?;
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Every step of the day `on`; the first error is returned after all were
/// tried, and the step that failed is tried again by the next run.
///
/// # Errors
///
/// The database's errors, and the first step that failed.
pub async fn process(state: &AppState, on: NaiveDate) -> Result {
    let waiting = JobCard::query()
        .where_(JobCard::STATUS.is_in([
            JobCardStatus::Ready,
            JobCardStatus::Declined,
            JobCardStatus::Cancelled,
        ]))
        .where_(JobCard::RELEASED_AT.is_null())
        .order_by(JobCard::ID)
        .get(&state.db)
        .await?;
    let mut failure = None;
    for card in waiting {
        if let Err(error) = step_card(state, &card, on).await {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// How many days after it left a vehicle's storage invoice is still retried.
pub const BILL_RETRY_DAYS: i64 = 35;

/// Bills again the storage of the vehicles that left (released or delivered)
/// and whose storage invoice did not get made (the invoicer failed at the
/// release): without this the vehicle stays released and unbilled. A job card
/// is billed once ([`bill_release`] claims it), so a card billed or with no fee
/// is left alone.
///
/// # Errors
///
/// The database's errors, and the first invoicer error after all were tried.
pub async fn retry_unbilled(state: &AppState, on: NaiveDate) -> Result<Vec<i64>> {
    let cards = JobCard::query()
        .where_(JobCard::STORAGE_INVOICED_AT.is_null())
        .where_(JobCard::COLLECT_FIRST_ON.is_not_null())
        .order_by(JobCard::ID)
        .get(&state.db)
        .await?;
    let mut billed = Vec::new();
    let mut failure = None;
    for card in cards {
        let Some(left) = card.released_at.or(card.delivered_at) else {
            continue;
        };
        if card.storage_fee_daily <= 0 || (on - dubai_date(left)).num_days() > BILL_RETRY_DAYS {
            continue;
        }
        match bill_release(state, card.id).await {
            Ok(Some(_)) => billed.push(card.id),
            Ok(None) => {}
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(billed), Err)
}

/// The daily run of `AUT.RUL.18` for the day `on`: the steps, then the storage
/// invoices that failed at a release ([`retry_unbilled`]).
///
/// # Errors
///
/// As [`process`], and the first invoicer error of the retry.
pub async fn run_daily(state: &AppState, on: NaiveDate) -> Result {
    let steps = process(state, on).await;
    let billing = retry_unbilled(state, on).await.map(|_| ());
    steps.and(billing)
}

/// Invoices the storage fee of the job card `job_card_id` that left (released,
/// or delivered): one day's fee for each day from the first reminder's day up
/// to the day before. The invoice is issued where invoicing is set to; a job
/// card is billed once. `None` when no fee applies.
///
/// # Errors
///
/// The database's and the invoicer's errors (the claim on the job card is
/// undone, so the billing can run again).
pub async fn bill_release(state: &AppState, job_card_id: i64) -> Result<Option<InvoiceId>> {
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    if card.storage_invoiced_at.is_some() {
        return Ok(card.storage_invoice_id.map(InvoiceId));
    }
    let left: Option<DateTime> = card.released_at.or(card.delivered_at);
    let Some(left) = left else {
        return Ok(None);
    };
    if !fee_applies(&state.db, &card).await? {
        return Ok(None);
    }
    let leaves = dubai_date(left);
    let days = days_charged(&card, leaves);
    if days <= 0 {
        return Ok(None);
    }
    let none: Option<DateTime> = None;
    let claimed = JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .where_(JobCard::STORAGE_INVOICED_AT.is_null())
        .update(
            &state.db,
            &[
                ("storage_invoiced_at", &Some(now())),
                ("storage_days", &days),
            ],
        )
        .await?;
    if claimed == 0 {
        let again = JobCard::find_or_404(&state.db, card.id).await?;
        return Ok(again.storage_invoice_id.map(InvoiceId));
    }
    let origin = card.subject();
    let description = {
        let rate = crate::money_text(card.storage_fee_daily);
        english(
            "auto.storage.invoice_line",
            &[("days", &days), ("rate", &rate)],
        )
    };
    let mut draft = InvoiceDraft::new(PartyId(card.customer_party_id), origin.clone())
        .date_of_supply(leaves)
        .line(
            DraftLine::new(&description, card.storage_fee_daily, VatCategory::Standard)
                .quantity_milli(days * 1000)
                .origin(origin),
        );
    if let Some(workshop) = card.workshop() {
        draft = draft.scope(workshop);
    }
    let books = invoicer(state);
    let drafted = match books.draft(&state.db, draft).await {
        Ok(drafted) => drafted,
        Err(error) => {
            JobCard::query()
                .where_(JobCard::ID.eq(card.id))
                .update(
                    &state.db,
                    &[("storage_invoiced_at", &none), ("storage_days", &0_i64)],
                )
                .await?;
            return Err(error);
        }
    };
    let Some(id) = drafted else {
        // Invoicing is off: nothing was billed, and nothing is claimed.
        JobCard::query()
            .where_(JobCard::ID.eq(card.id))
            .update(
                &state.db,
                &[("storage_invoiced_at", &none), ("storage_days", &0_i64)],
            )
            .await?;
        return Ok(None);
    };
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(
            &state.db,
            &[("storage_invoice_id", &Some(id.0)), ("updated_at", &now())],
        )
        .await?;
    let _issued = books.issue(state, id, None).await;
    Ok(Some(id))
}

/// Registers the daily run and its command.
pub(crate) fn register(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run_daily(&state, today()).await
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Remind the customers to collect their vehicles, and tell the branch manager about vehicles left for 30 days",
        |_args, state: AppState| async move {
            run_daily(&state, today()).await?;
            println!("The uncollected vehicles are up to date.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_day(date: &str) -> DateTime {
        let day: NaiveDate = date.parse().unwrap();
        wl_kernel::calendar::BusinessCalendar::default()
            .from_local(day.and_hms_opt(10, 0, 0).unwrap())
    }

    #[test]
    fn the_wait_starts_when_the_job_card_became_ready_or_ended() {
        let ready = JobCard {
            status: JobCardStatus::Ready,
            ready_at: Some(at_day("2026-11-01")),
            ..JobCard::default()
        };
        assert_eq!(start_of(&ready), "2026-11-01".parse().ok());
        let declined = JobCard {
            status: JobCardStatus::Declined,
            closed_at: Some(at_day("2026-11-02")),
            ..JobCard::default()
        };
        assert_eq!(start_of(&declined), "2026-11-02".parse().ok());
        assert_eq!(start_of(&JobCard::default()), None);
    }

    #[test]
    fn the_days_run_from_the_first_reminder_to_the_day_before_release() {
        let card = JobCard {
            collect_first_on: "2026-11-04".parse().ok(),
            ..JobCard::default()
        };
        // 2026-11-04 to 2026-11-09 is six days when the vehicle leaves on the 10th.
        assert_eq!(days_charged(&card, "2026-11-10".parse().unwrap()), 6);
        assert_eq!(days_charged(&card, "2026-11-04".parse().unwrap()), 0);
        assert_eq!(
            days_charged(&JobCard::default(), "2026-11-10".parse().unwrap()),
            0
        );
    }

    #[test]
    fn the_reminders_are_at_three_and_seven_days_and_the_task_at_thirty() {
        assert_eq!(REMINDER_DAYS, [3, 7]);
        assert_eq!(MANAGER_DAYS, 30);
    }
}
