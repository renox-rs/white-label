//! `ACC.RUL.07`: an invoice overdue by N days suspends the engagement, a
//! disputed one never does (04 §4.5 step 4 and *When it goes wrong*, §7.2;
//! BL-ACC-22, issue #346).
//!
//! The rule is **off** until the firm turns it on ([`switch`], the command
//! `accounting:overdue-suspension on|off [days]`); while it is off the daily
//! run does nothing at all, not even a rule run. When it is on, the daily run
//! ([`run`], 02:00 Asia/Dubai, a kernel rule run once per day) looks at every
//! `active` engagement: when its client has an open invoice with a balance
//! whose due date is N days back or more (N = [`DEFAULT_DAYS`], 60) and that
//! is not `disputed`, the engagement is suspended by the system
//! (`auto_suspend`) with the reason *non-payment*, and every partner is told
//! ([`OVERDUE_SUSPENDED`]). The suspension's own effects (tasks held, retainer
//! paused, `ACC.RUL.09` deadline notice for *non-payment*) follow from the
//! transition (BL-ACC-03, BL-ACC-04).
//!
//! Decisions:
//!
//! - **Days overdue** are counted from the due date (Asia/Dubai): an invoice
//!   due 2026-09-01 is 60 days overdue on 2026-10-31. Days in dispute are not
//!   excluded: once the dispute is resolved *upheld* the invoice counts from
//!   its original due date, so the grace period of an upheld dispute
//!   (`chasing_paused_until`, which pauses reminders) does not pause this
//!   rule. Only the `disputed` flag does.
//! - **Which invoices.** Invoices belong to the client party, not to one
//!   engagement (wl-invoicing keeps no engagement on an invoice), so an
//!   overdue invoice of the client suspends each of its `active` engagements,
//!   as the resume page lists the client's overdue invoices
//!   ([`crate::engagements::overdue_invoices`]).
//! - **Already suspended** (or in any state but `active`): nothing.
//! - **The partner** is every holder of the partner role, as for the MLRO's
//!   notice (an engagement has no partner field of its own).

use std::sync::{Arc, Mutex, PoisonError};

use renox::auth::permissions::users_with_role;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::compliance::{occurrence_key, schedule_subject};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceSummary, invoicer};
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest, notifier};

use crate::engagement::{Engagement, EngagementStatus};
use crate::engagement_machine::{AUTO_SUSPEND, EngagementInput};
use crate::money_text;
use crate::permissions::PARTNER_ROLE;
use crate::reasons::NON_PAYMENT;
use crate::settings;
use crate::texts::english;

/// The rule.
pub const RULE: &str = "ACC.RUL.07";
/// The setting that turns the rule on (off by default).
pub const ENABLED_KEY: &str = "overdue_suspension_enabled";
/// The setting with the days overdue (N).
pub const DAYS_KEY: &str = "overdue_suspension_days";
/// N: the days after the due date an unpaid invoice suspends the engagement.
pub const DEFAULT_DAYS: i64 = 60;
/// When the daily run happens, Asia/Dubai.
pub const RUN_AT: &str = "02:00";
/// The scheduled task of the daily run.
pub const TASK: &str = "accounting:overdue-suspension-run";
/// The command that turns the rule on or off.
pub const SWITCH_COMMAND: &str = "accounting:overdue-suspension";
/// The notice to the partners that an engagement was suspended.
pub const OVERDUE_SUSPENDED: &str = "accounting.overdue_suspended";

/// Whether `ACC.RUL.07` is on, and its N.
///
/// # Errors
///
/// The database's errors.
pub async fn setting(db: &Db) -> Result<Option<i64>> {
    if !settings::flag(db, ENABLED_KEY, false).await? {
        return Ok(None);
    }
    let days = settings::number(db, DAYS_KEY, DEFAULT_DAYS).await?;
    Ok(Some(if days > 0 { days } else { DEFAULT_DAYS }))
}

/// Turns `ACC.RUL.07` on (with N = `days`, [`DEFAULT_DAYS`] when `None`) or
/// off; `by` is the user who did, if any.
///
/// # Errors
///
/// The database's errors.
pub async fn switch(db: &Db, on: bool, days: Option<i64>, by: Option<i64>) -> Result {
    settings::set(db, ENABLED_KEY, if on { "on" } else { "off" }, by).await?;
    if let Some(days) = days.filter(|d| *d > 0) {
        settings::set(db, DAYS_KEY, &days.to_string(), by).await?;
    }
    Ok(())
}

/// The invoice of `invoices` that makes the rule fire on `on` with N =
/// `days`: open with a balance, not disputed, due `days` days back or more.
/// The oldest due date wins.
pub fn overdue_invoice(
    invoices: &[InvoiceSummary],
    on: NaiveDate,
    days: i64,
) -> Option<&InvoiceSummary> {
    invoices
        .iter()
        .filter(|i| i.status.is_open() && i.balance > 0 && !i.disputed)
        .filter(|i| i.due_date.is_some_and(|due| (on - due).num_days() >= days))
        .min_by_key(|i| (i.due_date, i.id))
}

/// `ACC.RUL.07` on `on`: the engagements it suspended. Nothing while the
/// rule is off.
///
/// # Errors
///
/// The database's errors (a failing run is retried by the rule run).
pub async fn run(state: &AppState, on: NaiveDate) -> Result<Vec<i64>> {
    let Some(days) = setting(&state.db).await? else {
        return Ok(Vec::new());
    };
    let spec = RuleRunSpec::new(RULE, schedule_subject(), occurrence_key(RULE, on));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let out = Arc::clone(&seen);
    run_rule(state, spec, move |state: AppState| async move {
        let suspended = run_now(&state, on, days).await?;
        *out.lock().unwrap_or_else(PoisonError::into_inner) = suspended;
        Ok(())
    })
    .await?;
    let suspended = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Ok(suspended)
}

/// The suspensions due on `on` with N = `days`, outside a rule run (the
/// caller checked the rule is on).
///
/// # Errors
///
/// The database's, the invoicing module's and the transition's errors.
pub async fn run_now(state: &AppState, on: NaiveDate, days: i64) -> Result<Vec<i64>> {
    let engagements = Engagement::query()
        .where_(Engagement::STATUS.eq(EngagementStatus::Active))
        .order_by(Engagement::ID)
        .get(&state.db)
        .await?;
    let mut suspended = Vec::new();
    for engagement in engagements {
        let invoices = invoicer(state)
            .open_of(&state.db, engagement.client())
            .await?;
        let Some(invoice) = overdue_invoice(&invoices, on, days) else {
            continue;
        };
        let number = invoice
            .number
            .clone()
            .unwrap_or_else(|| invoice.id.0.to_string());
        let due = invoice.due_date.map(|d| d.to_string()).unwrap_or_default();
        let balance = money_text(invoice.balance);
        // Reloaded: an earlier engagement's effects may have changed it.
        let Some(current) = Engagement::find(&state.db, engagement.id).await? else {
            continue;
        };
        if current.status != EngagementStatus::Active {
            continue;
        }
        let input = EngagementInput::on(on);
        let _outcome = current
            .transition(AUTO_SUSPEND)
            .by_system()
            .reason(NON_PAYMENT)
            .input(&input)
            .run(state)
            .await
            .map_err(TransitionError::into_error)?;
        tell_partners(state, &current, &number, &due, &balance, days).await?;
        suspended.push(current.id);
    }
    Ok(suspended)
}

/// Tells each partner that `engagement` was suspended for invoice `number`.
async fn tell_partners(
    state: &AppState,
    engagement: &Engagement,
    number: &str,
    due: &str,
    balance: &str,
    days: i64,
) -> Result {
    let link = format!("/accounting/engagements/{}", engagement.id);
    for user in users_with_role(&state.db, PARTNER_ROLE).await? {
        let request = NotifyRequest::new(OVERDUE_SUSPENDED, engagement.subject())
            .subject_label(&engagement.number())
            .to_user(user.id)
            .data(json!({
                "number": engagement.number(),
                "invoice": number,
                "due": due,
                "balance": balance,
                "days": days,
                "link": link,
            }));
        notifier(state).notify(state, request).await?;
    }
    Ok(())
}

/// The notification type of the partners' notice.
pub fn notification_types() -> Vec<NotificationTypeDef> {
    let subject = english("accounting.overdue_suspension.notice_subject", &[]);
    let body = english("accounting.overdue_suspension.notice_body", &[]);
    vec![
        NotificationTypeDef::new(OVERDUE_SUSPENDED, "accounting")
            .group("compliance")
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
    ]
}

/// Registers the notification type, the daily run and the switch command.
pub(crate) fn register(app: &mut Registry) {
    for def in notification_types() {
        notify::register_type(def);
    }
    app.schedule()
        .daily_at(RUN_AT, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        SWITCH_COMMAND,
        "Turn ACC.RUL.07 on or off: `on [days]` suspends an engagement for an invoice overdue by N days (default 60), never a disputed one; `off` stops it",
        |args, state: AppState| async move {
            let words: Vec<String> = args.positional().iter().map(|w| (*w).to_owned()).collect();
            let on = match words.first().map(String::as_str) {
                Some("on") => true,
                Some("off") => false,
                _ => {
                    return Err(Error::permanent_message(
                        "usage: accounting:overdue-suspension on [days] | off".to_owned(),
                    ));
                }
            };
            let days = words.get(1).and_then(|d| d.parse::<i64>().ok());
            switch(&state.db, on, days, None).await?;
            match setting(&state.db).await? {
                Some(days) => println!("ACC.RUL.07 is on: {days} days overdue."),
                None => println!("ACC.RUL.07 is off."),
            }
            Ok(())
        },
    );
}
