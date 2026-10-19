//! The reminder before a fiscal year without an approved budget (wl-invoicing
//! §6 `FIN.RUL.26`, `FIN.BUD.01`).
//!
//! [`run`] looks at the fiscal years that are about to start or just started
//! and, for each active legal entity with no `approved` version for such a
//! year, tells the accountant and the finance manager
//! ([`crate::InvoicingConfig::budget_reminder_roles`]): once from N days
//! before the year starts (default 30), and once more from its first day. A
//! reminder already sent is stored ([`BudgetReminder`]) and not sent again, a
//! version approved in between stops the next one, and nothing is stored when
//! nobody holds the roles, so the next run tries again. The daily task
//! [`TASK`] calls [`run`] for today in Asia/Dubai.

use renox::chrono::{Datelike, Days, NaiveDate};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::notify::{self, NotifyRequest};
use wl_kernel::{LegalEntity, Subject};

use crate::budget::{CompanyBudget, year_start};
use crate::budget_notices::{REMINDER, holders_of};
use crate::config_of;

/// The rule, as the specification names it.
pub const RULE: &str = "FIN.RUL.26";

/// The daily scheduled task that runs [`run`].
pub const TASK: &str = "invoicing:budget-reminders";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "06:10";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The reminder sent N days before the year.
pub const ADVANCE: &str = "advance";

/// The reminder sent from the first day of the year.
pub const FIRST_DAY: &str = "first_day";

/// A reminder that went out.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_budget_reminders",
    unique(legal_entity_id, fiscal_year, kind)
)]
pub struct BudgetReminder {
    /// The primary key.
    pub id: i64,
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The fiscal year the budget is missing for.
    pub fiscal_year: i64,
    /// `advance` or `first_day`.
    pub kind: String,
    /// The day it was sent.
    pub sent_on: NaiveDate,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Which reminder `on` calls for a year that starts on `start`, if any:
/// [`ADVANCE`] from `days` before it until it starts, [`FIRST_DAY`] from its
/// first day for `days` days (at least one).
pub fn due_kind(start: NaiveDate, on: NaiveDate, days: u32) -> Option<&'static str> {
    let window = u64::from(days.max(1));
    let before = start.checked_sub_days(Days::new(u64::from(days)))?;
    let after = start.checked_add_days(Days::new(window))?;
    if on >= before && on < start {
        Some(ADVANCE)
    } else if on >= start && on < after {
        Some(FIRST_DAY)
    } else {
        None
    }
}

/// `starts on 2028-01-01`, or `started on 2028-01-01` for the first-day
/// reminder.
fn when_text(kind: &str, start: NaiveDate) -> String {
    let verb = if kind == FIRST_DAY { "started" } else { "starts" };
    format!("{verb} on {}", start.format("%Y-%m-%d"))
}

/// Sends the reminders due on `on`. Returns how many it sent (one per
/// entity, year and kind, whatever the number of people told).
pub async fn run(state: &AppState, on: NaiveDate) -> Result<usize> {
    let config = config_of(state);
    let recipients = holders_of(&state.db, &config.budget_reminder_roles).await?;
    if recipients.is_empty() {
        return Ok(0);
    }
    let entities = LegalEntity::query()
        .order_by(LegalEntity::ID)
        .get(&state.db)
        .await?;
    let mut sent = 0;
    for entity in entities.iter().filter(|e| e.is_active()) {
        for year in [
            i64::from(on.year()) - 1,
            i64::from(on.year()),
            i64::from(on.year()) + 1,
        ] {
            let month =
                CompanyBudget::start_month_for(&state.db, entity.id, year, config.budget_fiscal_start_month)
                    .await?;
            let start = year_start(year, month);
            let Some(kind) = due_kind(start, on, config.budget_reminder_days) else {
                continue;
            };
            if CompanyBudget::approved_for(&state.db, entity.id, year)
                .await?
                .is_some()
            {
                continue;
            }
            let done = BudgetReminder::query()
                .where_(BudgetReminder::LEGAL_ENTITY_ID.eq(entity.id))
                .where_(BudgetReminder::FISCAL_YEAR.eq(year))
                .where_(BudgetReminder::KIND.eq(kind))
                .exists(&state.db)
                .await?;
            if done {
                continue;
            }
            let mut request = NotifyRequest::new(
                REMINDER,
                Subject::new("kernel.legal_entity", entity.id),
            )
            .data(json!({
                "entity": entity.code,
                "year": year,
                "when": when_text(kind, start),
                "link": format!("/budgets?entity={}", entity.id),
            }));
            for id in &recipients {
                request = request.to_user(*id);
            }
            notify::notifier(state).notify(state, request).await?;
            BudgetReminder {
                legal_entity_id: entity.id,
                fiscal_year: year,
                kind: kind.to_owned(),
                sent_on: on,
                ..Default::default()
            }
            .insert(&state.db)
            .await?;
            sent += 1;
        }
    }
    Ok(sent)
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

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_advance_reminder_comes_n_days_before_the_year() {
        let start = day(2028, 1, 1);
        assert_eq!(due_kind(start, day(2027, 12, 1), 30), None);
        assert_eq!(due_kind(start, day(2027, 12, 2), 30), Some(ADVANCE));
        assert_eq!(due_kind(start, day(2027, 12, 31), 30), Some(ADVANCE));
    }

    #[test]
    fn the_notice_says_when_the_year_starts_or_started() {
        let start = day(2028, 1, 1);
        assert_eq!(when_text(ADVANCE, start), "starts on 2028-01-01");
        assert_eq!(when_text(FIRST_DAY, start), "started on 2028-01-01");
    }

    #[test]
    fn the_first_day_reminder_comes_from_the_first_day() {
        let start = day(2028, 1, 1);
        assert_eq!(due_kind(start, day(2028, 1, 1), 30), Some(FIRST_DAY));
        assert_eq!(due_kind(start, day(2028, 1, 30), 30), Some(FIRST_DAY));
        assert_eq!(due_kind(start, day(2028, 1, 31), 30), None);
        // N = 0: only the first day.
        assert_eq!(due_kind(start, day(2027, 12, 31), 0), None);
        assert_eq!(due_kind(start, day(2028, 1, 1), 0), Some(FIRST_DAY));
        assert_eq!(due_kind(start, day(2028, 1, 2), 0), None);
    }
}
