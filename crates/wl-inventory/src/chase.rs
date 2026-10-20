//! Chasing returns to supplier that are not credited (wl-inventory §6
//! `STK.RUL.08`, issue #226).
//!
//! Money waits on a return that stays `dispatched`. The daily task [`TASK`]
//! (07:00 Asia/Dubai) looks at every `dispatched` return: when it has been
//! `dispatched` for more than N days (default 30, calendar days in Asia/Dubai
//! from the `dispatch` date) and no notice was sent yet, or the last one is N
//! days old, procurement and finance are notified and procurement gets one
//! task to chase the supplier. The first notice goes out on day N + 1; the
//! next on the day N after that notice (days N + 1, 2N + 1, 3N + 1...), until
//! the return leaves `dispatched`.
//!
//! Decisions: the task is opened on the days of a notice only, under one
//! source key per return, so a task still open is the one a later notice
//! finds (never one per day); it is resolved as the return is credited. The
//! recipients are the holders of the roles `procurement` and `finance`; a
//! notice nobody holds a role for is not sent, and the day is not recorded,
//! so it is tried again the next run. The notice and the task are done before
//! the day is kept on the return, so a failure is tried again at the next run.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::role_tasks::{self, RoleTask};

use crate::config_of;
use crate::supplier_return::{ReturnStatus, SupplierReturn};
use crate::texts::{english, said};

/// The daily scheduled task that runs [`run`].
pub const TASK: &str = "inventory:chase-returns";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "07:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The notification type of the notice to procurement and finance.
pub const NOTICE: &str = "inventory.supplier_return_chase";

/// The role that is told and gets the task.
pub const PROCUREMENT_ROLE: &str = "procurement";

/// The other role that is told.
pub const FINANCE_ROLE: &str = "finance";

/// The notification type of `STK.RUL.08`: staff, transactional, by e-mail.
pub fn notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NOTICE, "inventory")
        .group("returns")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_inventory.chase.subject")),
            english("wl_inventory.chase.body"),
        )
}

/// Registers the notification type with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(notice_type());
}

/// Registers the daily [`TASK`].
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
}

/// Whether a return dispatched on `dispatched` is to be chased on `today`:
/// it has been `dispatched` for more than `days` days, and no notice went out
/// yet or the last one (`last`) is `days` days old.
pub fn is_due(dispatched: NaiveDate, last: Option<NaiveDate>, today: NaiveDate, days: i64) -> bool {
    if (today - dispatched).num_days() <= days {
        return false;
    }
    match last {
        None => true,
        Some(last) => (today - last).num_days() >= days,
    }
}

/// The key of the task to chase the supplier of return `id`.
pub fn task_key(id: i64) -> String {
    format!("inventory.return_chase:{id}")
}

/// The ids of the users who hold `role` globally.
async fn holders(db: &Db, role: &str) -> Result<Vec<i64>> {
    Ok(renox::auth::permissions::users_with_role_in(
        db,
        role,
        &renox::auth::permissions::Scope::global(),
    )
    .await?
    .iter()
    .map(|user| user.id)
    .collect())
}

/// Chases the returns that are due today. Returns how many were chased.
///
/// # Errors
///
/// A database error, or the notifier's.
pub async fn run(state: &AppState) -> Result<usize> {
    let days = config_of(state).return_chase_days;
    let today = today();
    let dispatched = SupplierReturn::query()
        .where_(SupplierReturn::STATUS.eq(ReturnStatus::Dispatched))
        .order_by(SupplierReturn::ID)
        .get(&state.db)
        .await?;
    let mut chased = 0;
    for ret in dispatched {
        let Some(at) = ret.dispatched_at else {
            continue;
        };
        let sent = dubai_date(at);
        if !is_due(sent, ret.last_chased_on, today, days) {
            continue;
        }
        let mut people = holders(&state.db, PROCUREMENT_ROLE).await?;
        people.extend(holders(&state.db, FINANCE_ROLE).await?);
        people.sort_unstable();
        people.dedup();
        if people.is_empty() {
            continue;
        }
        let number = ret.number.clone().unwrap_or_default();
        let waiting = (today - sent).num_days().to_string();
        let mut request = NotifyRequest::new(NOTICE, ret.subject())
            .subject_label(&number)
            .data(renox::serde_json::json!({
                "number": number,
                "days": waiting,
                "dispatched": sent.format("%Y-%m-%d").to_string(),
                "supplier": ret.supplier_party_id,
            }));
        for id in people {
            request = request.to_user(id);
        }
        notify::notifier(state).notify(state, request).await?;
        let title = said("wl_inventory.chase.task_title", &[("number", &number)]);
        let description = said(
            "wl_inventory.chase.task",
            &[("number", &number), ("days", &waiting)],
        );
        let task = RoleTask::new(ret.subject(), PROCUREMENT_ROLE, &title, &task_key(ret.id))
            .description(&description);
        role_tasks::open(state, task).await?;
        let count = ret.chase_count + 1;
        SupplierReturn::query()
            .where_(SupplierReturn::ID.eq(ret.id))
            .update(
                &state.db,
                &[("last_chased_on", &Some(today)), ("chase_count", &count)],
            )
            .await?;
        chased += 1;
    }
    Ok(chased)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    #[test]
    fn nothing_is_due_until_the_day_after_n_days() {
        // Dispatched 2026-09-01, N = 30: 2026-10-01 is day 30, 10-02 day 31.
        assert!(!is_due(day(9, 1), None, day(10, 1), 30));
        assert!(is_due(day(9, 1), None, day(10, 2), 30));
        assert!(is_due(day(9, 1), None, day(10, 3), 30));
    }

    #[test]
    fn the_notice_repeats_every_n_days_from_the_last_one() {
        let last = Some(day(10, 2));
        assert!(!is_due(day(9, 1), last, day(10, 3), 30));
        assert!(!is_due(day(9, 1), last, day(10, 31), 30));
        assert!(is_due(day(9, 1), last, day(11, 1), 30));
    }

    #[test]
    fn the_task_key_names_the_return() {
        assert_eq!(task_key(7), "inventory.return_chase:7");
    }
}
