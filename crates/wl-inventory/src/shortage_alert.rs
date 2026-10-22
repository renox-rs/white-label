//! Alerting on pending issues and negative balances (wl-inventory §6
//! `STK.RUL.06`, issue #239): the storekeeper must know at once when stock
//! runs short, and the manager when it stays short.
//!
//! - When a pending issue is recorded (the kernel event [`StockNegative`]),
//!   the storekeeper of the location is told: the holders of `stock.adjust`
//!   in the location's scope, at most once an hour per item and location
//!   (a shortage that began less than an hour ago is told once, not once per
//!   consumption).
//! - The hourly [`TASK`] looks at every item and location that is still
//!   short, that is, with a pending issue still `open`. After H
//!   hours (default 24, [`crate::InventoryConfig::pending_alert_hours`]) it
//!   opens one task for the storekeeper to count or clear (one open task per
//!   item and location) and tells the manager, the holders of `stock.approve`
//!   in the scope, once.
//! - What is not short any more (the pending issue posted or was cleared, the
//!   balance is back) is forgotten, and its task is resolved: nothing is
//!   created for a shortage that ended before H hours.
//!
//! Decisions: only a pending issue starts and keeps an alert. A balance below
//! zero cannot arise yet: a location blocks negative stock, and the policy
//! that lets automatic consumption go below zero (`allow_consumption`) is
//! BL-STK-19, which will emit [`StockNegative`] without a pending issue
//! ([`on_negative`] already starts the alert for such an event) and extend
//! [`still_short`] with the balance check at the same time. Until then the
//! balance branch would be dead code, so it is not here.
//!
//! The storekeeper is whoever holds `stock.adjust` in the location's scope
//! and the manager whoever holds `stock.approve`; the task goes to the first
//! storekeeper; the hours count from when the oldest open pending issue was
//! recorded.

use renox::auth::permissions::users_with_permission_in;
use renox::chrono::Duration;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::inventory::{ITEM_SUBJECT_TYPE, StockNegative};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::tickets::{self, Assignee, OpenTicket, TASK_CATEGORY};

use crate::config_of;
use crate::item::{Item, quantity_text};
use crate::location::Location;
use crate::pending_issue::{PendingIssue, PendingStatus};
use crate::permissions;
use crate::texts::{english, said};

/// The hourly scheduled task that runs [`run`].
pub const TASK: &str = "inventory:alert-shortages";

/// The notification type of the notice to the storekeeper.
pub const SHORT_NOTICE: &str = "inventory.stock_short";

/// The notification type of the notice to the manager.
pub const OVERDUE_NOTICE: &str = "inventory.stock_short_overdue";

/// The hour in which the storekeeper is told once per item and location.
const BATCH_HOURS: i64 = 1;

/// Where stock stays short: an item at a location.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_shortage_alerts", unique(item_id, location_id))]
pub struct ShortageAlert {
    /// The primary key.
    pub id: i64,
    /// The item.
    pub item_id: i64,
    /// The location.
    pub location_id: i64,
    /// When it first became short.
    pub first_short_at: DateTime,
    /// When the storekeeper was last told.
    pub last_notified_at: Option<DateTime>,
    /// When the task was opened and the manager told.
    pub escalated_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The notification type of `STK.RUL.06` to the storekeeper.
pub fn short_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(SHORT_NOTICE, "inventory")
        .group("stock")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_inventory.short.subject")),
            english("wl_inventory.short.body"),
        )
}

/// The notification type of `STK.RUL.06` to the manager.
pub fn overdue_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(OVERDUE_NOTICE, "inventory")
        .group("stock")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_inventory.short.overdue_subject")),
            english("wl_inventory.short.overdue_body"),
        )
}

/// Registers the notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(short_notice_type());
    notify::register_type(overdue_notice_type());
}

/// Registers the hourly [`TASK`].
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule().hourly(TASK, |state: AppState| async move {
        run(&state, renox::db::now()).await.map(|_| ())
    });
}

/// The key of the task to count or clear `item_id` at `location_id`: one
/// open task per item and location.
pub fn task_key(location_id: i64, item_id: i64) -> String {
    format!("inventory.short_stock:{location_id}:{item_id}")
}

/// The ids of the users who hold `permission` in the scope of `location`.
async fn holders(db: &Db, permission: &str, location: &Location) -> Result<Vec<i64>> {
    Ok(users_with_permission_in(db, permission, &location.scope())
        .await?
        .iter()
        .map(|user| user.id)
        .collect())
}

/// The first of `ids` who does not hold the company administrator role.
async fn first_not_admin(db: &Db, ids: &[i64]) -> Result<Option<i64>> {
    for id in ids {
        let user = renox::auth::User::find_or_404(db, *id).await?;
        if !user
            .roles(db)
            .await?
            .iter()
            .any(|role| role == wl_kernel::COMPANY_ADMIN_ROLE)
        {
            return Ok(Some(*id));
        }
    }
    Ok(None)
}

/// Whether `item_id` is still short at `location_id`: a pending issue is
/// `open` (see the module docs for the balance below zero).
async fn still_short(db: &Db, item_id: i64, location_id: i64) -> Result<bool> {
    PendingIssue::query()
        .where_(PendingIssue::ITEM_ID.eq(item_id))
        .where_(PendingIssue::LOCATION_ID.eq(location_id))
        .where_(PendingIssue::STATUS.eq(PendingStatus::Open))
        .exists(db)
        .await
}

/// `STK.RUL.06` when a pending issue is recorded: tells the storekeeper of
/// the location, at most once an hour per item and location.
///
/// # Errors
///
/// A database error, or the notifier's.
pub async fn on_negative(state: &AppState, event: &StockNegative) -> Result {
    let now = renox::db::now();
    let item_id = event.item.subject_id;
    let location_id = event.location.subject_id;
    let began = match &event.pending_issue {
        Some(subject) => PendingIssue::find(&state.db, subject.subject_id)
            .await?
            .map_or(now, |pending| pending.recorded_at),
        None => now,
    };
    let mut alert = match ShortageAlert::query()
        .where_(ShortageAlert::ITEM_ID.eq(item_id))
        .where_(ShortageAlert::LOCATION_ID.eq(location_id))
        .first(&state.db)
        .await?
    {
        Some(alert) => alert,
        None => {
            let mut alert = ShortageAlert {
                item_id,
                location_id,
                first_short_at: began,
                ..ShortageAlert::default()
            };
            alert.save(&state.db).await?;
            alert
        }
    };
    if alert
        .last_notified_at
        .is_some_and(|last| now - last < Duration::hours(BATCH_HOURS))
    {
        return Ok(());
    }
    let location = Location::find_or_404(&state.db, location_id).await?;
    let item = Item::find_or_404(&state.db, item_id).await?;
    let people = holders(&state.db, permissions::STOCK_ADJUST, &location).await?;
    if people.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(SHORT_NOTICE, location.subject())
        .subject_label(&location.name_en)
        .data(renox::serde_json::json!({
            "item": item.code,
            "location": location.name_en,
            "quantity": quantity_text(event.quantity_milli),
            "source": event.source.as_ref().map(ToString::to_string).unwrap_or_default(),
        }));
    for id in people {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    alert.last_notified_at = Some(now);
    alert.save(&state.db).await?;
    Ok(())
}

/// Looks at every shortage at `at`: forgets (and resolves the task of) what
/// is not short any more, and after H hours opens the task and tells the
/// manager, once. Returns how many shortages were escalated.
///
/// # Errors
///
/// A database error, or the notifier's or the task creator's.
pub async fn run(state: &AppState, at: DateTime) -> Result<usize> {
    let hours = config_of(state).pending_alert_hours;
    let creator = tickets::task_creator(state);
    let alerts = ShortageAlert::query()
        .order_by(ShortageAlert::ID)
        .get(&state.db)
        .await?;
    let mut escalated = 0;
    for alert in alerts {
        let key = task_key(alert.location_id, alert.item_id);
        if !still_short(&state.db, alert.item_id, alert.location_id).await? {
            if let Some(task) = creator.find_open(state, &key).await? {
                let note = said("wl_inventory.short.task_done", &[]);
                creator.resolve_from_source(state, task.id, &note).await?;
            }
            ShortageAlert::query()
                .where_(ShortageAlert::ID.eq(alert.id))
                .delete(&state.db)
                .await?;
            continue;
        }
        if alert.escalated_at.is_some() || at - alert.first_short_at < Duration::hours(hours) {
            continue;
        }
        let location = Location::find_or_404(&state.db, alert.location_id).await?;
        let item = Item::find_or_404(&state.db, alert.item_id).await?;
        let storekeepers = holders(&state.db, permissions::STOCK_ADJUST, &location).await?;
        let managers = holders(&state.db, permissions::STOCK_APPROVE, &location).await?;
        let waiting = hours.to_string();
        let title = said(
            "wl_inventory.short.task_title",
            &[("item", &item.code), ("location", &location.name_en)],
        );
        let description = said(
            "wl_inventory.short.task_body",
            &[
                ("item", &item.code),
                ("location", &location.name_en),
                ("hours", &waiting),
            ],
        );
        if creator.find_open(state, &key).await?.is_none() {
            let mut ticket = OpenTicket::new(location.subject(), TASK_CATEGORY, &title)
                .description(&description)
                .source_key(&key);
            // A storekeeper who is not the company administrator (who holds
            // every permission) has the task before anyone else.
            let first = match first_not_admin(&state.db, &storekeepers).await? {
                Some(id) => Some(id),
                None => storekeepers.first().or(managers.first()).copied(),
            };
            if let Some(first) = first {
                ticket = ticket.assign_to(Assignee::user(first));
            }
            creator.open(state, ticket).await?;
        }
        if !managers.is_empty() {
            let mut request = NotifyRequest::new(OVERDUE_NOTICE, location.subject())
                .subject_label(&location.name_en)
                .data(renox::serde_json::json!({
                    "item": item.code,
                    "location": location.name_en,
                    "hours": waiting,
                    "item_ref": format!("{ITEM_SUBJECT_TYPE}:{}", item.id),
                }));
            for id in managers {
                request = request.to_user(id);
            }
            notify::notifier(state).notify(state, request).await?;
        }
        ShortageAlert::query()
            .where_(ShortageAlert::ID.eq(alert.id))
            .update(
                &state.db,
                &[
                    ("escalated_at", &Some(at)),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
        escalated += 1;
    }
    Ok(escalated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_task_is_kept_per_item_and_location() {
        assert_eq!(task_key(3, 9), "inventory.short_stock:3:9");
    }

    #[test]
    fn the_notices_are_staff_transactional() {
        assert_eq!(short_notice_type().key, SHORT_NOTICE);
        assert_eq!(overdue_notice_type().key, OVERDUE_NOTICE);
    }
}
