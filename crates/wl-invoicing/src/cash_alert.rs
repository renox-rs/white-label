//! The daily cash check (wl-invoicing §6 `FIN.RUL.23` and `FIN.RUL.25`, issue
//! #93, `FIN.CFL.01`).
//!
//! [`run`] projects the cash of every active legal entity the way the forecast
//! page does and then does two things:
//!
//! - **`FIN.RUL.23`:** when a week of the horizon closes below the entity's
//!   threshold ([`crate::InvoicingConfig::cash_threshold_of`], default 0), it
//!   tells the finance manager and the company admin
//!   ([`crate::InvoicingConfig::cash_alert_roles`]; a role nobody holds falls
//!   back to the admin) the first week below, its balance, the five largest
//!   payments of that week and the warnings of the forecast, and emits
//!   `CashForecastBelowThreshold`. The balance it reads leaves out the income
//!   budget fill unless the setting counts it
//!   ([`crate::cash_forecast::Forecast::balance_for_alert`]); scheduled
//!   charges and subscription cycles count. It warns once: only the last
//!   alert of the entity is stored ([`CashAlert`]), and it warns again when
//!   the first week below moves to an earlier calendar date, or after a run
//!   with no week below. A missing or stale bank balance is a warning in the
//!   notice, never a reason for one. Nothing is stored when nobody can be
//!   told, so the next run tries again.
//! - **`FIN.RUL.25`:** for each bank account of the entity with no balance,
//!   or whose latest one is older than the stale limit, one open task for the
//!   accountant ([`TaskKind::StatementBalanceStale`]), keyed on the account
//!   and the date of the balance it saw: no second task while the same balance
//!   is the latest, and the task closes when a newer statement balance or
//!   opening cash balance exists (a voided balance does not count: the task it
//!   had closed opens again, while one a person closed stays closed).
//!
//! The daily task [`TASK`] calls [`run`] for today in Asia/Dubai; a missed run
//! is not made up. Neither rule changes a document.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::cash_forecast::CashForecastBelowThreshold;
use wl_kernel::documents::today;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest, fill};
use wl_kernel::payments::Direction;
use wl_kernel::{LegalEntity, Subject};

use crate::allocate::amount_text;
use crate::budget_notices::holders_of;
use crate::cash_forecast::{self, Forecast, Line, LineKind, Warning};
use crate::cash_forecast_load;
use crate::cash_forecast_pages::warning_text;
use crate::config::InvoicingConfig;
use crate::config_of;
use crate::task::{ACCOUNTANT, InvoicingTask, TaskKind, TaskStatus};
use crate::texts::english;

/// The rule that warns of a low projected balance.
pub const RULE: &str = "FIN.RUL.23";

/// The rule that asks for a bank balance.
pub const STALE_RULE: &str = "FIN.RUL.25";

/// The daily scheduled task that runs [`run`].
pub const TASK: &str = "invoicing:cash-alerts";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The notification type of `FIN.RUL.23`.
pub const BELOW_THRESHOLD: &str = "invoicing.cash_below_threshold";

/// The company admin role, the fallback of a role nobody holds.
pub const ADMIN: &str = "admin";

/// How many payments the notice names.
const LARGEST: usize = 5;

/// The module that sends the notice.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the type belongs to.
const GROUP: &str = "cash";

/// The last alert of a legal entity.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_cash_alerts", unique(legal_entity_id))]
pub struct CashAlert {
    /// The primary key.
    pub id: i64,
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The first day of the first week below the threshold when the alert
    /// went out; `None` after a run with no week below.
    pub first_week_start: Option<NaiveDate>,
    /// The projected balance of that week, in AED fils.
    pub projected_balance: i64,
    /// The threshold then, in AED fils.
    pub threshold: i64,
    /// The day it went out.
    pub sent_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CashAlert {
    /// The last alert of `entity`, if any.
    pub async fn of(db: &Db, entity: i64) -> Result<Option<CashAlert>> {
        CashAlert::query()
            .where_(CashAlert::LEGAL_ENTITY_ID.eq(entity))
            .first(db)
            .await
    }
}

/// The first week (from 0) whose projected balance, as `FIN.RUL.23` reads it,
/// is below `threshold`.
pub fn first_week_below(forecast: &Forecast, threshold: i64, income_fill: bool) -> Option<usize> {
    (0..forecast.weeks.len())
        .find(|week| forecast.balance_for_alert(*week, income_fill) < threshold)
}

/// Whether a first week below that starts on `start` calls for a notice, given
/// the first week of the last alert: the first alert, one after a run with no
/// week below (`last` is `None`), and one for a week that starts earlier.
pub fn notice_due(last: Option<NaiveDate>, start: NaiveDate) -> bool {
    last.is_none_or(|previous| start < previous)
}

/// The largest payments of week `week` (from 0), largest first.
pub fn largest_payments(forecast: &Forecast, week: usize, count: usize) -> Vec<&Line> {
    let mut lines: Vec<&Line> = forecast
        .lines
        .iter()
        .filter(|l| l.week == week && l.direction == Direction::Out)
        .collect();
    lines.sort_by(|a, b| b.amount.cmp(&a.amount).then_with(|| a.label.cmp(&b.label)));
    lines.truncate(count);
    lines
}

/// The ids of the users who hold `roles`; a role nobody holds counts as the
/// company admin. Once each, in id order.
pub async fn recipients(db: &Db, roles: &[String]) -> Result<Vec<i64>> {
    let mut ids: Vec<i64> = Vec::new();
    for role in roles {
        let mut held = holders_of(db, std::slice::from_ref(role)).await?;
        if held.is_empty() && role != ADMIN {
            held = holders_of(db, &[ADMIN.to_owned()]).await?;
        }
        ids.extend(held);
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// The notification type of `FIN.RUL.23`: staff, transactional, by e-mail.
pub fn below_threshold_type() -> NotificationTypeDef {
    NotificationTypeDef::new(BELOW_THRESHOLD, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.cash_alert.subject")),
            english("wl_invoicing.cash_alert.body"),
        )
}

/// Registers the notification type with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(below_threshold_type());
}

/// The text of the warnings of `forecast` for the notice: those of the forecast
/// and, for an entity with no bank account known at all, the missing balance.
fn warnings_text(entity: &LegalEntity, forecast: &Forecast) -> String {
    let t = |key: &str| english(&format!("wl_invoicing.{key}")).to_owned();
    let codes = std::collections::BTreeMap::from([(entity.id, entity.code.clone())]);
    let mut lines: Vec<String> = forecast
        .warnings
        .iter()
        // The budget warning is about the fill, not about the cash.
        .filter(|w| !matches!(w, Warning::NoBudget { .. }))
        .map(|w| warning_text(&t, &codes, w))
        .collect();
    if forecast.openings.is_empty() {
        lines.push(format!("{} - {}", entity.code, t("cash.no_accounts")));
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("Warnings: {}.", lines.join("; "))
    }
}

/// The label of a payment line in the notice: a drawings total keeps its
/// details back.
fn payment_text(line: &Line) -> String {
    let label = if line.total_only && matches!(line.kind, LineKind::Bill | LineKind::BudgetFill) {
        english("wl_invoicing.cash.drawings").to_owned()
    } else {
        line.label.clone()
    };
    format!("{label} {}", amount_text("AED", line.amount))
}

/// `FIN.RUL.23` for one entity. Returns whether a notice went out.
async fn check_threshold(
    state: &AppState,
    config: &InvoicingConfig,
    entity: &LegalEntity,
    forecast: &Forecast,
    on: NaiveDate,
) -> Result<bool> {
    let threshold = config.cash_threshold_of(&entity.code);
    let stored = CashAlert::of(&state.db, entity.id).await?;
    let Some(week) = first_week_below(forecast, threshold, config.cash_income_fill_counted) else {
        // A run with no week below: a later breach notifies again.
        if let Some(mut row) = stored.filter(|r| r.first_week_start.is_some()) {
            row.first_week_start = None;
            row.save(&state.db).await?;
        }
        return Ok(false);
    };
    let first = &forecast.weeks[week];
    let last = stored.as_ref().and_then(|r| r.first_week_start);
    if !notice_due(last, first.start) {
        return Ok(false);
    }
    let people = recipients(&state.db, &config.cash_alert_roles).await?;
    if people.is_empty() {
        return Ok(false);
    }
    let balance = forecast.balance_for_alert(week, config.cash_income_fill_counted);
    let payments = largest_payments(forecast, week, LARGEST)
        .into_iter()
        .map(payment_text)
        .collect::<Vec<_>>()
        .join("; ");
    let mut request = NotifyRequest::new(
        BELOW_THRESHOLD,
        Subject::new("kernel.legal_entity", entity.id),
    )
    .data(json!({
        "entity": entity.code,
        "week": first.number,
        "from": first.start.format("%Y-%m-%d").to_string(),
        "to": first.end.format("%Y-%m-%d").to_string(),
        "balance": amount_text("AED", balance),
        "threshold": amount_text("AED", threshold),
        "payments": if payments.is_empty() { "none".to_owned() } else { payments },
        "warnings": warnings_text(entity, forecast),
        "link": format!("/cash-forecast?entity={}", entity.id),
    }));
    for id in people {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    let mut row = stored.unwrap_or(CashAlert {
        legal_entity_id: entity.id,
        ..Default::default()
    });
    row.first_week_start = Some(first.start);
    row.projected_balance = balance;
    row.threshold = threshold;
    row.sent_on = Some(on);
    if row.id == 0 {
        row.insert(&state.db).await?;
    } else {
        row.save(&state.db).await?;
    }
    state
        .emit(CashForecastBelowThreshold {
            legal_entity_id: entity.id,
            week: u32::try_from(first.number).unwrap_or(0),
            week_start: first.start,
            projected_balance: balance,
            threshold,
        })
        .await?;
    Ok(true)
}

/// The key of the task for `account` whose latest balance is dated `as_of`.
fn task_key(entity: i64, account: &str, as_of: Option<NaiveDate>) -> String {
    format!(
        "{entity}|{account}|{}",
        as_of.map_or_else(|| "none".to_owned(), |d| d.format("%Y-%m-%d").to_string())
    )
}

/// The account and the balance date a task key names.
fn parse_key(key: &str) -> Option<(String, Option<NaiveDate>)> {
    let (_, rest) = key.split_once('|')?;
    let (account, date) = rest.rsplit_once('|')?;
    Some((account.to_owned(), date.parse::<NaiveDate>().ok()))
}

/// Whether a balance dated `now` is newer than the one a task saw (`seen`).
fn is_newer(now: Option<NaiveDate>, seen: Option<NaiveDate>) -> bool {
    match (now, seen) {
        (Some(_), None) => true,
        (Some(now), Some(seen)) => now > seen,
        (None, _) => false,
    }
}

/// Closes the open stale balance tasks of `entity` for `account` whose seen
/// balance is older than `as_of`. Returns how many closed.
///
/// # Errors
///
/// A database error.
pub async fn close_resolved(
    db: &Db,
    entity: i64,
    account: &str,
    as_of: NaiveDate,
) -> Result<usize> {
    let open = InvoicingTask::query()
        .where_(InvoicingTask::KIND.eq(TaskKind::StatementBalanceStale))
        .where_(InvoicingTask::SOURCE_ID.eq(entity))
        .where_(InvoicingTask::STATUS.eq(TaskStatus::Open))
        .get(db)
        .await?;
    let mut closed = 0;
    for mut task in open {
        let Some((name, seen)) = parse_key(&task.key) else {
            continue;
        };
        if name != account || !is_newer(Some(as_of), seen) {
            continue;
        }
        task.status = TaskStatus::Done;
        task.done_at = Some(renox::db::now());
        task.save_only(db, &["status", "done_at"]).await?;
        closed += 1;
    }
    Ok(closed)
}

/// `FIN.RUL.25` for one entity. Returns how many tasks it opened.
async fn check_balances(
    state: &AppState,
    config: &InvoicingConfig,
    entity: &LegalEntity,
    forecast: &Forecast,
    on: NaiveDate,
) -> Result<usize> {
    let db = &state.db;
    let mine: Vec<_> = forecast
        .openings
        .iter()
        .filter(|o| o.legal_entity_id == entity.id)
        .collect();
    for opening in &mine {
        if let Some(balance) = opening.balance {
            close_resolved(db, entity.id, &opening.account, balance.as_of).await?;
        }
    }
    let mut opened = 0;
    for opening in mine {
        let as_of = opening.balance.map(|b| b.as_of);
        let stale = as_of.is_none_or(|d| (on - d).num_days() > config.cash_stale_days);
        if !stale {
            continue;
        }
        let message = fill(
            english("wl_invoicing.task.statement_balance_stale"),
            &json!({
                "entity": entity.code,
                "account": opening.account,
                "as_of": as_of.map_or_else(
                    || "never".to_owned(),
                    |d| d.format("%Y-%m-%d").to_string(),
                ),
                "days": config.cash_stale_days,
            }),
        );
        let mut task = InvoicingTask {
            kind: TaskKind::StatementBalanceStale,
            source_type: "kernel.legal_entity".to_owned(),
            source_id: entity.id,
            key: task_key(entity.id, &opening.account, as_of),
            message,
            role: ACCOUNTANT.to_owned(),
            status: TaskStatus::Open,
            ..Default::default()
        };
        match task.insert(db).await {
            Ok(()) => opened += 1,
            Err(error) if error.is_unique_violation() => {
                // A task a newer balance closed comes back when that balance
                // is gone (voided); one a person closed stays closed.
                let existing = InvoicingTask::query()
                    .where_(InvoicingTask::KIND.eq(TaskKind::StatementBalanceStale))
                    .where_(InvoicingTask::KEY.eq(task.key.as_str()))
                    .first(db)
                    .await?;
                if let Some(mut existing) = existing.filter(|t| {
                    t.status == TaskStatus::Done && t.done_by.is_none()
                }) {
                    existing.status = TaskStatus::Open;
                    existing.done_at = None;
                    existing.save_only(db, &["status", "done_at"]).await?;
                    opened += 1;
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(opened)
}

/// Runs both rules for every active legal entity as of `on`. Returns how many
/// alerts went out.
///
/// # Errors
///
/// A database or notifier error.
pub async fn run(state: &AppState, on: NaiveDate) -> Result<usize> {
    let config = config_of(state);
    let entities = LegalEntity::query()
        .order_by(LegalEntity::ID)
        .get(&state.db)
        .await?;
    let mut sent = 0;
    for entity in entities.iter().filter(|e| e.is_active()) {
        let inputs =
            cash_forecast_load::load(state, &config, std::slice::from_ref(entity), on).await?;
        let forecast = cash_forecast::assemble(&inputs);
        check_balances(state, &config, entity, &forecast, on).await?;
        if check_threshold(state, &config, entity, &forecast, on).await? {
            sent += 1;
        }
    }
    Ok(sent)
}

/// Registers the daily [`TASK`] at the configured time.
pub(crate) fn schedule(app: &mut Registry, config: &InvoicingConfig) {
    app.schedule()
        .daily_at(&config.cash_alert_time, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cash_forecast::{EntityInput, Inputs, InvoiceInput};
    use renox::chrono::Weekday;
    use wl_kernel::PartyId;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn forecast_of(entity: EntityInput) -> Forecast {
        cash_forecast::assemble(&Inputs {
            today: day(2027, 1, 13),
            weeks: 13,
            week_start: Weekday::Mon,
            stale_days: 7,
            disputed_counted: false,
            vat_counted: true,
            unapproved_counted: true,
            days_late: false,
            entities: vec![entity],
        })
    }

    #[test]
    fn a_notice_is_due_for_the_first_week_below_and_for_an_earlier_one() {
        let week6 = day(2027, 2, 15);
        let week4 = day(2027, 2, 1);
        assert!(notice_due(None, week6));
        assert!(!notice_due(Some(week6), week6));
        assert!(!notice_due(Some(week4), week6));
        assert!(notice_due(Some(week6), week4));
    }

    #[test]
    fn the_first_week_below_is_the_earliest_and_the_income_fill_can_be_left_out() {
        let mut entity = EntityInput {
            legal_entity_id: 1,
            ..Default::default()
        };
        entity.invoices.push(InvoiceInput {
            subject: Subject::new("invoicing.invoice", 1),
            label: "INV-1".into(),
            party: PartyId(1),
            due: day(2027, 1, 20),
            balance: Some(100),
            disputed: 0,
            blocked: false,
        });
        let forecast = forecast_of(entity);
        // No payments: the balance never falls below 0.
        assert_eq!(first_week_below(&forecast, 0, false), None);
        // Below a threshold of AED 1.00 from week 1 until the receipt, after
        // which the balance is exactly 100 fils.
        assert_eq!(first_week_below(&forecast, 101, false), Some(0));
    }

    #[test]
    fn a_task_key_names_the_account_and_the_balance_it_saw() {
        let key = task_key(3, "ENBD-1", Some(day(2027, 1, 5)));
        assert_eq!(key, "3|ENBD-1|2027-01-05");
        assert_eq!(parse_key(&key), Some(("ENBD-1".to_owned(), Some(day(2027, 1, 5)))));
        let none = task_key(3, "A|B", None);
        assert_eq!(parse_key(&none), Some(("A|B".to_owned(), None)));
    }

    #[test]
    fn only_a_later_balance_resolves_a_task() {
        assert!(is_newer(Some(day(2027, 1, 12)), Some(day(2027, 1, 5))));
        assert!(!is_newer(Some(day(2027, 1, 5)), Some(day(2027, 1, 5))));
        assert!(is_newer(Some(day(2027, 1, 5)), None));
        assert!(!is_newer(None, None));
    }

    #[test]
    fn the_notice_names_the_five_largest_payments_largest_first() {
        use wl_kernel::payments::UnpaidRefund;
        let mut entity = EntityInput {
            legal_entity_id: 1,
            ..Default::default()
        };
        for (i, amount) in [100, 700, 300, 500, 200, 600, 400].iter().enumerate() {
            entity.refunds.push(UnpaidRefund {
                id: wl_kernel::payments::RefundId(i as i64 + 1),
                party: PartyId(1),
                legal_entity_id: 1,
                amount: *amount,
                currency: "AED".into(),
                failed: false,
                approved_on: None,
                source: Subject::new("invoicing.credit_note", 2),
            });
        }
        let forecast = forecast_of(entity);
        let amounts: Vec<i64> = largest_payments(&forecast, 0, LARGEST)
            .iter()
            .map(|l| l.amount)
            .collect();
        assert_eq!(amounts, [700, 600, 500, 400, 300]);
    }

    #[test]
    fn the_notification_type_is_transactional_with_an_english_email() {
        let def = below_threshold_type();
        assert_eq!(def.key, BELOW_THRESHOLD);
        assert_eq!(def.category, Category::Transactional);
        assert_eq!(def.channels, [NotifyChannel::Email]);
    }
}
