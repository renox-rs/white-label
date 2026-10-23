//! The aggregator's rider who does not arrive (06 F&B §4.1 row *The
//! aggregator's rider does not arrive*, §7.2 `FNB.RUL.16`; BL-FNB-06).
//!
//! An aggregator order is `ready` and waits at the pass for the aggregator's
//! rider. A scheduled check ([`run`], every minute) looks at the orders that
//! are still `ready`:
//!
//! - **N = 15 minutes** after `ready` (setting
//!   [`pickup_chase_minutes`](crate::settings::FnbSettings::pickup_chase_minutes)):
//!   the kitchen managers are told to chase the aggregator on its tablet;
//! - **M = 45 minutes** after `ready` (setting `pickup_task_minutes`): a task
//!   for the kitchen manager asks to record what happened.
//!
//! The manager records it on the order page: [`picked_up_late`] completes the
//! order, [`crate::cancel::by_aggregator`] cancels it with the party
//! *aggregator*, the food scrapped as wastage and *compensation expected* set.
//! The manager's choice is the approval of `SAL.ACL.05`: no second one is
//! asked. Nothing is cancelled automatically and the customer is never
//! refunded (the aggregator holds the customer's money).
//!
//! An order picked up before 45 minutes gets no task, and a task that is open
//! closes when the order leaves `ready` ([`close_task`]). The timers run from
//! the `ready` time, in or out of opening hours. An order that became ready
//! before the clock was recorded (`ready_at` empty) is not watched.

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::sales::order_desk;
use wl_kernel::tickets::task_creator;

use crate::notices::{about, tell_managers};
use crate::order::{FnbOrder, FnbOrderStatus};
use crate::order_machine::COMPLETE;
use crate::permissions;
use crate::placing::refused;
use crate::settings::settings;
use crate::texts::english;

/// The scheduled task, and the command, of the rule.
pub const TASK: &str = "fnb:aggregator-pickup";

/// How often the rule looks, in minutes.
pub const EVERY_MINUTES: u64 = 1;

/// What one run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Done {
    /// Kitchen managers told to chase an aggregator, per order.
    pub alerts: usize,
    /// Tasks opened to record the outcome.
    pub tasks: usize,
}

/// The key a task about `order` is found by while it is open.
pub fn task_key(order: &FnbOrder) -> String {
    format!("fnb.aggregator_pickup:{}", order.id)
}

/// Looks at the aggregator orders that are `ready` at `now`: the alert at N
/// minutes, the task at M. Each is done once per order.
///
/// # Errors
///
/// Database errors and the notifier's.
pub async fn run(state: &AppState, now: DateTime) -> Result<Done> {
    let mut done = Done::default();
    let rules = settings(state);
    if !rules.pickup_rule {
        return Ok(done);
    }
    let waiting = FnbOrder::query()
        .where_(FnbOrder::STATUS.eq(FnbOrderStatus::Ready))
        .get(&state.db)
        .await?;
    for order in waiting {
        let Some(ready) = order.ready_at else {
            continue;
        };
        let minutes = (now - ready).num_minutes();
        if minutes < rules.pickup_chase_minutes {
            continue;
        }
        let aggregator = order_desk(state)
            .find(state, order.sales_order())
            .await?
            .is_some_and(|view| view.aggregator);
        if !aggregator {
            continue;
        }
        if order.pickup_alert_at.is_none() {
            chase(state, &order, minutes).await?;
            done.alerts += 1;
        }
        if minutes >= rules.pickup_task_minutes && order.pickup_task_at.is_none() {
            open_task(state, &order, minutes, now).await?;
            done.tasks += 1;
        }
    }
    Ok(done)
}

/// Tells the kitchen managers to chase the aggregator, once.
async fn chase(state: &AppState, order: &FnbOrder, minutes: i64) -> Result {
    let at = Some(renox::db::now());
    FnbOrder::query()
        .where_(FnbOrder::ID.eq(order.id))
        .update(&state.db, &[("pickup_alert_at", &at)])
        .await?;
    let waited = minutes.to_string();
    tell_managers(
        state,
        order,
        permissions::ORDERS_MANAGE,
        &about("fnb.pickup.alert_title", order),
        &english(
            "fnb.pickup.alert_body",
            &[("number", &order.number), ("minutes", &waited)],
        ),
    )
    .await?;
    Ok(())
}

/// Opens the task for the kitchen manager to record what happened, once.
async fn open_task(state: &AppState, order: &FnbOrder, minutes: i64, now: DateTime) -> Result {
    let waited = minutes.to_string();
    let task = RoleTask::new(
        order.subject(),
        permissions::KITCHEN_MANAGER_ROLE,
        &about("fnb.pickup.task_title", order),
        &task_key(order),
    )
    .description(&english(
        "fnb.pickup.task_body",
        &[
            ("number", &order.number),
            ("minutes", &waited),
            ("link", &format!("/orders/{}", order.id)),
        ],
    ))
    .due_at(now);
    let ticket = role_tasks::open(state, task)
        .await?
        .map(|opened| opened.id.0);
    let at = Some(renox::db::now());
    FnbOrder::query()
        .where_(FnbOrder::ID.eq(order.id))
        .update(
            &state.db,
            &[("pickup_task_at", &at), ("pickup_task_id", &ticket)],
        )
        .await?;
    Ok(())
}

/// The order left `ready` (picked up, cancelled): its open task closes.
///
/// # Errors
///
/// Database errors and the ticket module's.
pub(crate) async fn close_task(state: &AppState, id: i64) -> Result {
    let Some(order) = FnbOrder::find(&state.db, id).await? else {
        return Ok(());
    };
    if order.pickup_task_id.is_none() {
        return Ok(());
    }
    let tickets = task_creator(state);
    if let Some(open) = tickets.find_open(state, &task_key(&order)).await? {
        let note = english("fnb.pickup.task_closed", &[("number", &order.number)]);
        tickets.complete(state, open.id, &note).await?;
    }
    Ok(())
}

/// The kitchen manager records that the aggregator's rider came late: the
/// order is completed.
///
/// # Errors
///
/// `Forbidden` without `orders.manage`; 422 unless the order is an
/// aggregator's and `ready`.
pub async fn picked_up_late(state: &AppState, user: &AuthUser, id: i64) -> Result {
    if !user.has_permission(permissions::ORDERS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    ensure_waiting_aggregator(state, &order).await?;
    let history = english("fnb.pickup.late_history", &[("number", &order.number)]);
    let _outcome = order
        .transition(COMPLETE)
        .by(user)
        .reason(history)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Ok(())
}

/// 422 unless `order` is an aggregator's and still `ready`.
pub(crate) async fn ensure_waiting_aggregator(state: &AppState, order: &FnbOrder) -> Result {
    if order.status != FnbOrderStatus::Ready {
        return Err(refused(english("fnb.pickup.not_ready", &[])));
    }
    let aggregator = order_desk(state)
        .find(state, order.sales_order())
        .await?
        .is_some_and(|view| view.aggregator);
    if !aggregator {
        return Err(refused(english("fnb.pickup.not_aggregator", &[])));
    }
    Ok(())
}

/// Registers the rule on the scheduler and as a command.
pub(crate) fn register(app: &mut Registry) {
    app.schedule()
        .every_minutes(EVERY_MINUTES, TASK, |state: AppState| async move {
            run(&state, renox::db::now()).await.map(|_| ())
        });
}
