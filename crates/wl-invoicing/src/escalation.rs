//! Escalating an open dispute (wl-invoicing §5.10, `FIN.RUL.15`).
//!
//! An open dispute is told again every N whole days since it was raised
//! (Asia/Dubai, N = `dispute_escalation_days`, default 14). [`step_on`]
//! counts the steps; [`notification_type`] is the notice
//! ([`ESCALATED`]) and [`data`] its variables.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::automation::{self, RuleRunSpec};
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::allocate::amount_text;
use crate::config_of;
use crate::dispute::Dispute;
use crate::escalation_log::EscalationLog;
use crate::invoice::Invoice;
use crate::owner::{self, OwnerRole};
use crate::texts::english;

/// The rule, as the specification names it.
pub const RULE: &str = "FIN.RUL.15";

/// The notification type of an escalation.
pub const ESCALATED: &str = "invoicing.dispute_escalation";

/// The module that sends the notice.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the type belongs to.
const GROUP: &str = "invoices";

/// The daily scheduled task that runs [`run`].
pub const TASK: &str = "invoicing:dispute-escalation";

/// When [`TASK`] runs, in Asia/Dubai (like `invoicing:reminders`).
pub const TASK_AT: &str = "08:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The role a finance manager is told as, in the escalation log.
const FINANCE_MANAGER: &str = "finance_manager";

/// The step that is current on `today` for a dispute raised on `raised_on`
/// (its Asia/Dubai date): `(today - raised_on) / n` whole days, when that
/// is at least 1, else `None`. The step stays the same until the next
/// whole `n` days have passed, so a day the task skipped is caught up by the
/// next run, which sends the current step only. `n` is never below 1 (0 is
/// read as 1).
pub fn step_on(raised_on: NaiveDate, today: NaiveDate, n: u32) -> Option<u32> {
    let days = (today - raised_on).num_days();
    if days < 0 {
        return None;
    }
    let step = u32::try_from(days / i64::from(n.max(1))).ok()?;
    (step >= 1).then_some(step)
}

/// `date` as `18 Oct 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// Who a copy of the notice is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience<'a> {
    /// The owner side: the owner, or the receivables holders.
    Owner,
    /// The finance manager, with the owner's name (or "receivables").
    FinanceManager {
        /// The owner's name, or `receivables` when the owner is gone.
        owner: &'a str,
    },
}

/// The template variables of the notice for `step` (the days open are
/// `step * days`). `ticket_number` is the dispute's ticket, when it has one.
pub fn data(
    invoice: &Invoice,
    dispute: &Dispute,
    raised_on: NaiveDate,
    step: u32,
    days: u32,
    ticket_number: Option<&str>,
    audience: Audience<'_>,
) -> Value {
    let party_name = invoice
        .party_snapshot
        .as_ref()
        .map(|snapshot| snapshot.0.legal_name_en.clone())
        .unwrap_or_default();
    let owner = match audience {
        Audience::Owner => "",
        Audience::FinanceManager { owner } => owner,
    };
    json!({
        "number": invoice.number.clone().unwrap_or_default(),
        "party_name": party_name,
        "amount": amount_text("AED", dispute.amount),
        "reason": dispute.reason,
        "raised_on": date_text(raised_on),
        "days": step.saturating_mul(days),
        "step": step,
        "ticket": ticket_number.unwrap_or("-"),
        "owner": owner,
        "link": format!("/invoices/{}", invoice.id),
    })
}

/// The notification type [`ESCALATED`]: staff, transactional, by e-mail.
pub fn notification_type() -> NotificationTypeDef {
    NotificationTypeDef::new(ESCALATED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.escalation.subject")),
            english("wl_invoicing.escalation.body"),
        )
}

/// Registers [`ESCALATED`] with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(notification_type());
}

/// The occurrence key of one step of one dispute.
pub fn occurrence_key(dispute_id: i64, step: u32) -> String {
    format!("dispute:{dispute_id}:step:{step}")
}

/// Sends the escalation notices due on `on` (`FIN.RUL.15`) and returns how
/// many notices were sent.
///
/// Picks the open disputes (`outcome IS NULL`) whose invoice is still
/// `disputed` with that dispute, and for each one the step [`step_on`] gives
/// for `on`. Each (dispute, step) is one rule run with the key
/// [`occurrence_key`], so a step is never run twice. One failing dispute
/// does not stop the others; the first error is returned at the end.
pub async fn run(state: &AppState, on: NaiveDate) -> Result<usize> {
    let days = config_of(state).dispute_escalation_days.max(1);
    let open = Dispute::query()
        .where_(Dispute::OUTCOME.is_null())
        .order_by(Dispute::ID)
        .get(&state.db)
        .await?;
    let sent = Arc::new(AtomicUsize::new(0));
    let mut first_error = None;
    for dispute in open {
        let Some(step) = step_on(dubai_date(dispute.raised_at), on, days) else {
            continue;
        };
        if let Err(error) = run_step(state, &dispute, step, days, &sent).await {
            first_error.get_or_insert(error);
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(sent.load(Ordering::SeqCst)),
    }
}

/// One rule run: `step` of `dispute`.
async fn run_step(
    state: &AppState,
    dispute: &Dispute,
    step: u32,
    days: u32,
    sent: &Arc<AtomicUsize>,
) -> Result<()> {
    let Some(invoice) = Invoice::find(&state.db, dispute.invoice_id).await? else {
        return Ok(());
    };
    if !invoice.disputed || invoice.dispute_id != Some(dispute.id) {
        return Ok(());
    }
    let dispute_id = dispute.id;
    let spec = RuleRunSpec::new(RULE, invoice.subject(), occurrence_key(dispute_id, step));
    let sent = Arc::clone(sent);
    automation::run_rule(state, spec, move |state: AppState| async move {
        notify_step(&state, dispute_id, step, days, &sent).await
    })
    .await?;
    Ok(())
}

/// The action of the rule run: reads the dispute again (a dispute resolved
/// meanwhile sends nothing) and tells each recipient once, logging the user
/// before sending and removing the row again when sending fails.
async fn notify_step(
    state: &AppState,
    dispute_id: i64,
    step: u32,
    days: u32,
    sent: &AtomicUsize,
) -> Result<()> {
    let db = &state.db;
    let Some(dispute) = Dispute::find(db, dispute_id).await? else {
        return Ok(());
    };
    if !dispute.is_open() {
        return Ok(());
    }
    let Some(invoice) = Invoice::find(db, dispute.invoice_id).await? else {
        return Ok(());
    };
    if !invoice.disputed || invoice.dispute_id != Some(dispute.id) {
        return Ok(());
    }
    let raised_on = dubai_date(dispute.raised_at);
    let ticket = dispute.ticket_id.map(|id| format!("#{id}"));
    let (owner_role, owners) = owner::recipients(db, &invoice).await?;
    let owner_name = match (owner_role, owners.first()) {
        (OwnerRole::Owner, Some(user)) => user.name.clone(),
        _ => "receivables".to_owned(),
    };
    let mut recipients: Vec<(User, &'static str, bool)> = owners
        .into_iter()
        .map(|user| (user, owner_role.as_str(), false))
        .collect();
    if step >= 2 {
        let role = &config_of(state).dispute_escalation_role;
        for user in owner::finance_managers(db, role, &invoice).await? {
            if !recipients.iter().any(|(told, _, _)| told.id == user.id) {
                recipients.push((user, FINANCE_MANAGER, true));
            }
        }
    }
    let notifier = notify::notifier(state);
    for (user, role, manager) in recipients {
        let Some(mut log) =
            EscalationLog::claim(db, dispute.id, invoice.id, i64::from(step), user.id, role)
                .await?
        else {
            continue;
        };
        let audience = if manager {
            Audience::FinanceManager { owner: &owner_name }
        } else {
            Audience::Owner
        };
        let request = NotifyRequest::new(ESCALATED, invoice.subject())
            .to_user(user.id)
            .data(data(
                &invoice,
                &dispute,
                raised_on,
                step,
                days,
                ticket.as_deref(),
                audience,
            ));
        match notifier.notify(state, request).await {
            Ok(ids) => {
                if let Some(id) = ids.first() {
                    log.message_id = Some(id.0);
                    log.save(db).await?;
                }
                sent.fetch_add(1, Ordering::SeqCst);
            }
            Err(error) => {
                log.delete(db).await?;
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Registers the daily [`TASK`].
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    #[test]
    fn nothing_before_the_first_whole_period() {
        assert_eq!(step_on(day(11, 1), day(11, 1), 14), None);
        assert_eq!(step_on(day(11, 1), day(11, 14), 14), None);
        assert_eq!(step_on(day(11, 1), day(10, 20), 14), None);
    }

    #[test]
    fn steps_of_fourteen_days() {
        assert_eq!(step_on(day(11, 1), day(11, 15), 14), Some(1));
        assert_eq!(step_on(day(11, 1), day(11, 28), 14), Some(1));
        assert_eq!(step_on(day(11, 1), day(11, 29), 14), Some(2));
        assert_eq!(step_on(day(11, 1), day(12, 12), 14), Some(2));
        assert_eq!(step_on(day(11, 1), day(12, 13), 14), Some(3));
    }

    #[test]
    fn steps_of_seven_days() {
        assert_eq!(step_on(day(11, 1), day(11, 7), 7), None);
        assert_eq!(step_on(day(11, 1), day(11, 8), 7), Some(1));
        assert_eq!(step_on(day(11, 1), day(11, 15), 7), Some(2));
    }

    #[test]
    fn a_skipped_day_is_caught_up_with_the_current_step() {
        // No run on 15 Nov: the run of 16 Nov still sees step 1, and the run
        // of 30 Nov sees step 2 (step 1 was missed and is not sent).
        assert_eq!(step_on(day(11, 1), day(11, 16), 14), Some(1));
        assert_eq!(step_on(day(11, 1), day(11, 30), 14), Some(2));
    }

    #[test]
    fn a_zero_period_counts_as_one_day() {
        assert_eq!(step_on(day(11, 1), day(11, 3), 0), Some(2));
    }

    #[test]
    fn a_late_evening_utc_is_the_next_dubai_day() {
        let raised_at = NaiveDate::from_ymd_opt(2026, 10, 31)
            .unwrap()
            .and_hms_opt(21, 30, 0)
            .unwrap()
            .and_utc();
        let raised_on = dubai_date(raised_at);
        assert_eq!(raised_on, day(11, 1));
        assert_eq!(step_on(raised_on, day(11, 14), 14), None);
        assert_eq!(step_on(raised_on, day(11, 15), 14), Some(1));
    }

    #[test]
    fn the_occurrence_key_names_the_dispute_and_the_step() {
        assert_eq!(occurrence_key(7, 2), "dispute:7:step:2");
    }

    #[test]
    fn the_task_runs_at_eight_in_dubai() {
        assert_eq!(TASK, "invoicing:dispute-escalation");
        assert_eq!((TASK_AT, TASK_ZONE), ("08:00", "Asia/Dubai"));
    }

    #[test]
    fn the_notice_has_its_texts() {
        assert!(english("wl_invoicing.escalation.body").contains("{{ days }}"));
        assert_eq!(notification_type().key, ESCALATED);
    }
}
