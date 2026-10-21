//! Reminding the deciders of an open stock hold and escalating it (wl-inventory
//! §6 `STK.RUL.07`, issue #232): held food and medicines cannot wait forever.
//!
//! The hourly [`TASK`] looks at every `open` hold that still has an
//! undecided line. Counted from the moment the hold was placed:
//!
//! - every H hours (H = 24, [`crate::InventoryConfig::reminder_hours`]) the
//!   deciders, the holders of `lots.release` in the scope of the locations of
//!   the lines still undecided, are reminded: at H, 2H, 3H...;
//! - every 2 x H hours the escalation role (the business owner by default,
//!   06: the kitchen manager) is told: at 2H, 4H, 6H... while lines stay
//!   undecided.
//!
//! H and the escalation role are settings that can differ per reason or per
//! location: the most specific one wins (location over reason over default).
//!
//! Decisions: the number of reminders and escalations sent is kept on the
//! hold, so each period gives one notice however often the job runs, and a
//! job that was down catches up with one notice, not several; a hold whose
//! lines were all decided but that is still `open` (a crash between the
//! decision and the closing) is settled by the job instead of reminded; the
//! escalation role is told through its holders, and the company admins when
//! nobody holds it; a hold that is not `open` is never reminded.

use std::collections::BTreeSet;

use renox::auth::permissions::users_with_role;
use renox::prelude::*;
use wl_kernel::COMPANY_ADMIN_ROLE;
use wl_kernel::compliance::business_owner_role;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::config_of;
use crate::hold_decide::settle;
use crate::hold_service::tell_deciders;
use crate::location::Location;
use crate::stock_hold::{HoldLineStatus, HoldStatus, StockHold, lines_of};
use crate::texts::english;

/// The hourly scheduled task that runs [`run`].
pub const TASK: &str = "inventory:remind-holds";

/// The notification type of the reminder to the deciders.
pub const REMINDER_NOTICE: &str = "inventory.stock_hold_reminder";

/// The notification type of the notice to the escalation role.
pub const ESCALATION_NOTICE: &str = "inventory.stock_hold_escalation";

/// The notification type of the reminder to the deciders.
pub fn reminder_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(REMINDER_NOTICE, "inventory")
        .group("holds")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_inventory.hold.reminder_subject")),
            english("wl_inventory.hold.reminder_body"),
        )
}

/// The notification type of the notice to the escalation role.
pub fn escalation_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(ESCALATION_NOTICE, "inventory")
        .group("holds")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_inventory.hold.escalation_subject")),
            english("wl_inventory.hold.escalation_body"),
        )
}

/// Registers the notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(reminder_notice_type());
    notify::register_type(escalation_notice_type());
}

/// Registers the hourly [`TASK`].
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .hourly(TASK, |state: AppState| async move {
            run(&state, renox::db::now()).await.map(|_| ())
        });
}

/// What one run of [`run`] sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sent {
    /// Holds whose deciders were reminded.
    pub reminders: usize,
    /// Holds whose escalation role was told.
    pub escalations: usize,
}

/// The users to tell about an escalation: the holders of `role`, else the
/// company admins.
async fn escalation_users(db: &Db, role: &str) -> Result<Vec<i64>> {
    for role in [role, COMPANY_ADMIN_ROLE] {
        let users = users_with_role(db, role).await?;
        if !users.is_empty() {
            return Ok(users.iter().map(|user| user.id).collect());
        }
    }
    Ok(Vec::new())
}

/// Reminds and escalates at `at`. Returns what was sent.
///
/// # Errors
///
/// A database error, or the notifier's.
pub async fn run(state: &AppState, at: DateTime) -> Result<Sent> {
    let config = config_of(state);
    let mut sent = Sent::default();
    let holds = StockHold::query()
        .where_(StockHold::STATUS.eq(HoldStatus::Open))
        .order_by(StockHold::ID)
        .get(&state.db)
        .await?;
    for hold in holds {
        let lines = lines_of(&state.db, hold.id).await?;
        let waiting: Vec<_> = lines
            .iter()
            .filter(|line| line.status == HoldLineStatus::Undecided)
            .collect();
        if waiting.is_empty() {
            // Every line is decided: nothing to remind; close it if a crash
            // left it open.
            settle(state, hold.id).await?;
            continue;
        }
        let Some(placed) = hold.placed_at.or(hold.created_at) else {
            continue;
        };
        let location = match hold.location_id {
            Some(id) => Location::find(&state.db, id).await?,
            None => None,
        };
        let code = location.as_ref().map(|found| found.code.as_str());
        let hours = config.reminder_hours(&hold.reason, code).max(1);
        let elapsed = (at - placed).num_seconds().max(0);
        let due_reminders = elapsed / (hours * 3600);
        let due_escalations = elapsed / (2 * hours * 3600);
        let number = hold.number.clone().unwrap_or_default();
        if due_reminders > hold.reminders_sent {
            let places: BTreeSet<i64> = waiting.iter().map(|line| line.location_id).collect();
            tell_deciders(
                state,
                &hold,
                REMINDER_NOTICE,
                &places,
                renox::serde_json::json!({
                    "number": number,
                    "reason": hold.reason,
                    "lines": waiting.len().to_string(),
                    "hours": (due_reminders * hours).to_string(),
                }),
            )
            .await?;
            StockHold::query()
                .where_(StockHold::ID.eq(hold.id))
                .update(
                    &state.db,
                    &[
                        ("reminders_sent", &due_reminders),
                        ("last_reminded_at", &Some(at)),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
            sent.reminders += 1;
        }
        if due_escalations > hold.escalations_sent {
            let role = config
                .escalation_role(&hold.reason, code)
                .map_or_else(|| business_owner_role(state), str::to_owned);
            let people = escalation_users(&state.db, &role).await?;
            if !people.is_empty() {
                let mut request = NotifyRequest::new(ESCALATION_NOTICE, hold.subject())
                    .subject_label(&number)
                    .data(renox::serde_json::json!({
                        "number": number,
                        "reason": hold.reason,
                        "lines": waiting.len().to_string(),
                        "hours": (due_escalations * 2 * hours).to_string(),
                    }));
                for id in people {
                    request = request.to_user(id);
                }
                notify::notifier(state).notify(state, request).await?;
            }
            StockHold::query()
                .where_(StockHold::ID.eq(hold.id))
                .update(
                    &state.db,
                    &[
                        ("escalations_sent", &due_escalations),
                        ("last_escalated_at", &Some(at)),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
            sent.escalations += 1;
        }
    }
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notices_are_staff_transactional() {
        assert_eq!(reminder_notice_type().key, REMINDER_NOTICE);
        assert_eq!(escalation_notice_type().key, ESCALATION_NOTICE);
    }
}
