//! Payment reminders before and after the due date (wl-invoicing §5.3,
//! `FIN.RUL.01`).
//!
//! The reminder steps are days relative to the due date
//! ([`InvoicingConfig::reminder_steps`](crate::InvoicingConfig), default
//! 3 days before and 1, 7 and 14 days after). [`run`] looks at every
//! `issued`, `partially_paid` or `overdue` invoice with a balance that is
//! not `disputed` (nor inside the grace period after an upheld dispute,
//! [`Invoice::chasing_paused`]) and whose source is not held by its vertical
//! (a legal case on a lease, [`wl_kernel::chase_holds`]; after the hold the
//! steps reached meanwhile are not sent), and sends it the latest step whose day has come, unless
//! that step (or a later one) is already in its [`ReminderLog`]: at most one
//! reminder per invoice per run (§5.3 step 2). `draft`, `paid`,
//! `written_off` and `credited` invoices are never picked.
//!
//! The reminder goes to the invoice's party through the kernel
//! [`Notifier`](wl_kernel::notify::Notifier), as [`BEFORE_DUE`] (steps on
//! or before the due date) or [`AFTER_DUE`] (steps after it); both types are
//! registered in `register` ([`register_types`]). The daily task [`TASK`]
//! calls [`run`] for today in Asia/Dubai.

use renox::chrono::{Days, Duration, NaiveDate};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::chase_holds::{self, ChaseHold};
use wl_kernel::documents::today;
use wl_kernel::invoicing::InvoiceStatus;
use wl_kernel::notify::{
    self, Category, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest,
};

use crate::allocate::amount_text;
use crate::config_of;
use crate::invoice::Invoice;
use crate::texts::english;

/// The rule stored on each log row.
pub const RULE: &str = "FIN.RUL.01";

/// The notification type of a reminder on or before the due date.
pub const BEFORE_DUE: &str = "invoice.reminder_before_due";

/// The notification type of a reminder after the due date.
pub const AFTER_DUE: &str = "invoice.reminder_after_due";

/// The daily scheduled task that runs [`run`].
pub const TASK: &str = "invoicing:reminders";

/// When [`TASK`] runs, in Asia/Dubai: after `invoicing:mark-overdue`
/// (00:30) and in the morning, so the party reads it during the day.
pub const TASK_AT: &str = "08:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The module that sends the reminders, on their notification types.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the types belong to.
const GROUP: &str = "invoices";

/// The staff role told when a reminder gives up (wl-notify §5.4: the
/// collector role; the host's receivables role).
const STAFF_ROLE: &str = "receivables";

/// The states an invoice is reminded in.
const OPEN: [InvoiceStatus; 3] = [
    InvoiceStatus::Issued,
    InvoiceStatus::PartiallyPaid,
    InvoiceStatus::Overdue,
];

/// A reminder sent for an invoice (wl-invoicing §3 *Reminder log*). One row
/// per invoice and step.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_reminder_logs",
    unique(invoice_id, step),
    index(invoice_id)
)]
pub struct ReminderLog {
    /// The primary key.
    pub id: i64,
    /// The invoice.
    pub invoice_id: i64,
    /// The rule that sent it: [`RULE`].
    pub rule: String,
    /// The step, in days relative to the due date (negative before it).
    pub step: i64,
    /// The notification type asked for: [`BEFORE_DUE`] or [`AFTER_DUE`].
    pub notification_type: String,
    /// The type's first channel.
    pub channel: NotifyChannel,
    /// The first message id the notifier returned; `None` without wl-notify.
    pub message_id: Option<i64>,
    /// When it was sent.
    pub sent_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The step to send an invoice due on `due` on `on`: the latest of `steps`
/// whose day (`due` + step days) is `on` or earlier, unless it or a later
/// step is in `sent`. `None` when no step has come yet or it was sent.
pub fn due_step(steps: &[i32], due: NaiveDate, on: NaiveDate, sent: &[i32]) -> Option<i32> {
    let days = (on - due).num_days();
    let latest = steps
        .iter()
        .copied()
        .filter(|&step| i64::from(step) <= days)
        .max()?;
    if sent.iter().any(|&done| done >= latest) {
        return None;
    }
    Some(latest)
}

/// The notification type of `step`.
pub fn type_for(step: i32) -> &'static str {
    if step <= 0 { BEFORE_DUE } else { AFTER_DUE }
}

/// `date` as `18 Oct 2026`.
fn date_text(date: Option<NaiveDate>) -> String {
    date.map(|d| d.format("%-d %b %Y").to_string())
        .unwrap_or_default()
}

/// The template variables of a reminder for `invoice` at `step`.
fn data(invoice: &Invoice, step: i32) -> renox::serde_json::Value {
    let party_name = invoice
        .party_snapshot
        .as_ref()
        .map(|snapshot| snapshot.0.legal_name_en.clone())
        .unwrap_or_default();
    json!({
        "number": invoice.number.clone().unwrap_or_default(),
        "party_name": party_name,
        "issue_date": date_text(invoice.issue_date),
        "due_date": date_text(invoice.due_date),
        "gross_total": amount_text(&invoice.currency, invoice.gross_total),
        "balance": amount_text(&invoice.currency, invoice.balance),
        "step": step,
        "days": step.unsigned_abs(),
    })
}

/// The reminder type `key` with its English e-mail text under `texts`
/// (`wl_invoicing.reminder.{texts}.subject` and `.body`).
fn reminder_type(key: &str, texts: &str) -> NotificationTypeDef {
    let subject = english(&format!("wl_invoicing.reminder.{texts}.subject")).to_owned();
    let body = english(&format!("wl_invoicing.reminder.{texts}.body")).to_owned();
    NotificationTypeDef::new(key, MODULE)
        .group(GROUP)
        .category(Category::Reminder)
        .give_up(&[
            GiveUpReaction::MarkContactInvalid,
            GiveUpReaction::AlternativeRecipient,
            GiveUpReaction::StaffTask,
        ])
        .staff_role(STAFF_ROLE)
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// The two reminder types: [`BEFORE_DUE`] and [`AFTER_DUE`].
pub fn notification_types() -> [NotificationTypeDef; 2] {
    [
        reminder_type(BEFORE_DUE, "before_due"),
        reminder_type(AFTER_DUE, "after_due"),
    ]
}

/// Registers the reminder types with the kernel registry.
pub(crate) fn register_types() {
    for def in notification_types() {
        notify::register_type(def);
    }
}

/// Sends the reminders due on `on` (`FIN.RUL.01`), one step at most per
/// invoice, and returns how many were sent.
///
/// A step is logged before it is sent, so two runs never send it twice
/// (the log is unique per invoice and step); when the notifier fails, the
/// log row is removed again and the run stops with the error.
pub async fn run(state: &AppState, on: NaiveDate) -> Result<usize> {
    let steps = config_of(state).reminder_steps.clone();
    let Some(&first) = steps.first() else {
        return Ok(0);
    };
    // No step can have come for an invoice due after `on - first`.
    let latest_due = if first >= 0 {
        on.checked_sub_days(Days::new(u64::from(first.unsigned_abs())))
    } else {
        on.checked_add_days(Days::new(u64::from(first.unsigned_abs())))
    };
    let Some(latest_due) = latest_due else {
        return Ok(0);
    };
    let candidates = Invoice::query()
        .where_(Invoice::STATUS.is_in(OPEN))
        .where_(Invoice::BALANCE.gt(0))
        .where_(Invoice::DISPUTED.eq(false))
        .where_(Invoice::DUE_DATE.lte(latest_due))
        .order_by(Invoice::ID)
        .get(&state.db)
        .await?;
    let mut sent = 0;
    for invoice in candidates {
        let Some(due) = invoice.due_date else {
            continue;
        };
        // After an upheld dispute, chasing waits out the grace period; the
        // latest step due then goes out once (`due_step`).
        if invoice.chasing_paused(on) {
            continue;
        }
        // A vertical holds chasing for its own records (a legal case on a
        // lease): nothing is sent while it lasts, and the steps whose day was
        // reached meanwhile are not sent after it.
        let hold = chase_holds::hold(&state.db, &invoice.source()).await?;
        if hold.is_held() {
            continue;
        }
        let mut logged: Vec<i32> = ReminderLog::query()
            .where_(ReminderLog::INVOICE_ID.eq(invoice.id))
            .get(&state.db)
            .await?
            .iter()
            .filter_map(|log| i32::try_from(log.step).ok())
            .collect();
        if let ChaseHold::Resumed { on: ended } = hold
            && let Some(missed) = steps
                .iter()
                .copied()
                .filter(|&step| due + Duration::days(i64::from(step)) <= ended)
                .max()
        {
            logged.push(missed);
        }
        let Some(step) = due_step(&steps, due, on, &logged) else {
            continue;
        };
        if send(state, &invoice, step).await? {
            sent += 1;
        }
    }
    Ok(sent)
}

/// Logs and sends `step` for `invoice`; `false` when another run logged it
/// first.
async fn send(state: &AppState, invoice: &Invoice, step: i32) -> Result<bool> {
    let type_key = type_for(step);
    let channel = notify::notification_type(type_key)
        .and_then(|def| def.channels.first().copied())
        .unwrap_or_default();
    let mut log = ReminderLog {
        invoice_id: invoice.id,
        rule: RULE.to_owned(),
        step: i64::from(step),
        notification_type: type_key.to_owned(),
        channel,
        sent_at: renox::db::now(),
        ..Default::default()
    };
    match log.insert(&state.db).await {
        Ok(()) => {}
        Err(error) if error.is_unique_violation() => return Ok(false),
        Err(error) => return Err(error),
    }
    let request = NotifyRequest::new(type_key, invoice.subject())
        .to_party(invoice.party())
        .data(data(invoice, step));
    match notify::notifier(state).notify(state, request).await {
        Ok(ids) => {
            if let Some(id) = ids.first() {
                log.message_id = Some(id.0);
                log.save(&state.db).await?;
            }
            Ok(true)
        }
        Err(error) => {
            log.delete(&state.db).await?;
            Err(error)
        }
    }
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

    const STEPS: [i32; 4] = [-3, 1, 7, 14];

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).expect("date")
    }

    #[test]
    fn nothing_is_due_before_the_first_step() {
        assert_eq!(due_step(&STEPS, day(20), day(16), &[]), None);
    }

    #[test]
    fn the_before_step_is_sent_once() {
        assert_eq!(due_step(&STEPS, day(20), day(17), &[]), Some(-3));
        assert_eq!(due_step(&STEPS, day(20), day(18), &[-3]), None);
        assert_eq!(due_step(&STEPS, day(20), day(20), &[-3]), None);
    }

    #[test]
    fn the_after_step_comes_on_its_day() {
        assert_eq!(due_step(&STEPS, day(20), day(21), &[-3]), Some(1));
        assert_eq!(due_step(&STEPS, day(20), day(22), &[-3, 1]), None);
        assert_eq!(due_step(&STEPS, day(20), day(27), &[-3, 1]), Some(7));
    }

    #[test]
    fn only_the_latest_passed_step_is_sent() {
        // Nothing sent yet, 10 days late: the 7-day step, not 1 and 7.
        assert_eq!(due_step(&STEPS, day(1), day(11), &[]), Some(7));
        // A later step already sent: nothing older is sent again.
        assert_eq!(due_step(&STEPS, day(1), day(11), &[14]), None);
    }

    #[test]
    fn steps_on_or_before_the_due_date_use_the_before_type() {
        assert_eq!(type_for(-3), BEFORE_DUE);
        assert_eq!(type_for(0), BEFORE_DUE);
        assert_eq!(type_for(1), AFTER_DUE);
    }

    #[test]
    fn the_types_are_reminders_with_an_english_text() {
        for def in notification_types() {
            assert_eq!(def.module, "invoicing");
            assert_eq!(def.group, "invoices");
            assert_eq!(def.category, Category::Reminder);
            assert_eq!(def.staff_role.as_deref(), Some("receivables"));
            let template = def.template_for(NotifyChannel::Email, "en").expect("text");
            assert!(
                template
                    .subject
                    .as_deref()
                    .unwrap()
                    .contains("{{ number }}")
            );
            assert!(template.body.contains("{{ balance }}"));
        }
    }

    #[test]
    fn the_data_fills_the_texts() {
        let invoice = Invoice {
            number: Some("INV-PGP-2026-0001".into()),
            currency: "AED".into(),
            issue_date: Some(day(1)),
            due_date: Some(day(20)),
            gross_total: 105_000,
            balance: 63_000,
            ..Default::default()
        };
        let [_, after] = notification_types();
        let template = after.template_for(NotifyChannel::Email, "en").unwrap();
        let body = notify::fill(&template.body, &data(&invoice, 7));
        assert!(body.contains("INV-PGP-2026-0001 of 1 Oct 2026"));
        assert!(body.contains("due on 20 Oct 2026, 7 day(s) ago"));
        assert!(body.contains("AED 630.00"));
        assert_eq!(
            notify::fill(template.subject.as_deref().unwrap(), &data(&invoice, 7)),
            "Invoice INV-PGP-2026-0001 is overdue"
        );
    }
}
