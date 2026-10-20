//! The notices and tasks of cash custody (wl-payments §5.5, `FIN.RUL.22`).
//!
//! - `payments.cash_shortage`: the holder and the holder's manager are told a
//!   handover is short (`CashShortage`), or a shortage was raised against the
//!   holder later (transactional, e-mail, EN).
//! - `payments.cash_task`: the manager is told of a task about a handover
//!   (reach the holder, count what was received, approve a deduction), and the
//!   company admin that a holder did not hand over (`FIN.RUL.22`).
//!
//! The holder's manager is the one recorded on the holder, else the first
//! manager wl-hr names for the holder's employee record, else the first
//! holder of the configured manager role (`cash_manager_role`), else a
//! company admin.

use renox::auth::permissions::users_with_role;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::scopes::COMPANY_ADMIN_ROLE;
use wl_kernel::tickets::{self, Assignee, OpenTicket, TASK_CATEGORY};

use crate::cash_handover::{CashHandover, CashHolder};
use crate::config_of;
use crate::followup::money_text;
use crate::texts::english;

/// The type of the shortage notice to the holder and the manager.
pub const CASH_SHORTAGE: &str = "payments.cash_shortage";

/// The type of the notice of a task about a handover.
pub const CASH_TASK: &str = "payments.cash_task";

const MODULE: &str = "payments";
const GROUP: &str = "payments";

/// The notification type [`CASH_SHORTAGE`].
pub fn shortage_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CASH_SHORTAGE, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.cash_shortage.subject")),
            english("wl_payments.cash_shortage.body"),
        )
}

/// The notification type [`CASH_TASK`].
pub fn task_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CASH_TASK, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.cash_task.subject")),
            english("wl_payments.cash_task.body"),
        )
}

/// Registers the types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(shortage_type());
    notify::register_type(task_type());
}

/// The manager of the holder of `handover`: see the module text.
///
/// # Errors
///
/// Database errors.
pub async fn manager_of(state: &AppState, handover: &CashHandover) -> Result<Option<i64>> {
    if let Some(holder) = CashHolder::find(&state.db, handover.holder_id).await?
        && let Some(manager) = holder.manager_user_id
    {
        return Ok(Some(manager));
    }
    let workforce = wl_kernel::workforce::workforce(state);
    if let Some(employee) = workforce
        .employee_of_user(&state.db, handover.holder_user_id)
        .await?
        && let Some(profile) = workforce.staff(state, employee).await?
        && let Some(manager) = profile.managers.first()
    {
        return Ok(Some(*manager));
    }
    wl_kernel::role_tasks::assignee(&state.db, &config_of(state).cash_manager_role).await
}

/// The company admins.
async fn admins(state: &AppState) -> Result<Vec<i64>> {
    Ok(users_with_role(&state.db, COMPANY_ADMIN_ROLE)
        .await?
        .iter()
        .map(|user| user.id)
        .collect())
}

/// The users told of a shortage on `handover`: the holder, the responsible
/// person and the manager, each once.
pub(crate) fn shortage_recipients(handover: &CashHandover, manager: Option<i64>) -> Vec<i64> {
    let mut ids: Vec<i64> = vec![handover.holder_user_id, handover.responsible_user_id];
    ids.extend(manager);
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// The words of a handover in a notice: its period.
pub(crate) fn period_text(handover: &CashHandover) -> String {
    match handover.period_subject() {
        Some(subject) => format!(
            "{} {} of {}",
            english(&format!("wl_payments.cash.period.{}", handover.period_kind.as_str())),
            subject.subject_id,
            handover.period_date.format("%Y-%m-%d")
        ),
        None => format!(
            "{} {}",
            english(&format!("wl_payments.cash.period.{}", handover.period_kind.as_str())),
            handover.period_date.format("%Y-%m-%d")
        ),
    }
}

/// Tells the holder and the holder's manager that `handover` is short.
///
/// # Errors
///
/// Database errors, or a notice that failed.
pub(crate) async fn shortage(state: &AppState, handover: &CashHandover) -> Result {
    let manager = manager_of(state, handover).await?;
    let ids = shortage_recipients(handover, manager);
    let mut request = NotifyRequest::new(CASH_SHORTAGE, handover.subject()).data(json!({
        "amount": money_text(&handover.currency, handover.difference),
        "period": period_text(handover),
        "link": format!("/cash/{}", handover.id),
    }));
    for id in ids {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells `users` of a task about `handover`.
pub(crate) async fn task(
    state: &AppState,
    handover: &CashHandover,
    users: &[i64],
    text: &str,
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(CASH_TASK, handover.subject()).data(json!({
        "text": text,
        "link": format!("/cash/{}", handover.id),
    }));
    for id in users {
        request = request.to_user(*id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the company admins of a task about `handover`.
pub(crate) async fn tell_admins(
    state: &AppState,
    handover: &CashHandover,
    text: &str,
) -> Result<Vec<i64>> {
    let ids = admins(state).await?;
    task(state, handover, &ids, text).await?;
    Ok(ids)
}

/// The key a handover's reach-the-holder ticket is found by while open.
pub(crate) fn reach_key(handover: &CashHandover) -> String {
    format!("payments.cash_reach:{}", handover.id)
}

/// Opens the ticket that asks `manager` to reach the holder of `handover`
/// (none when ticketing is off, or one is open already).
///
/// # Errors
///
/// Database errors, or a ticket that failed.
pub(crate) async fn open_reach_ticket(
    state: &AppState,
    handover: &CashHandover,
    manager: Option<i64>,
    title: &str,
    description: &str,
) -> Result {
    let creator = tickets::task_creator(state);
    let key = reach_key(handover);
    if creator.find_open(state, &key).await?.is_some() {
        return Ok(());
    }
    let mut ticket = OpenTicket::new(handover.subject(), TASK_CATEGORY, title)
        .description(description)
        .source_key(&key);
    if let Some(manager) = manager {
        ticket = ticket.assign_to(Assignee::user(manager));
    }
    creator.open(state, ticket).await?;
    Ok(())
}

/// Closes the reach-the-holder ticket of `handover` once it is counted.
///
/// # Errors
///
/// Database errors, or a ticket that failed.
pub(crate) async fn close_reach_ticket(state: &AppState, handover: &CashHandover) -> Result {
    let creator = tickets::task_creator(state);
    if let Some(ticket) = creator.find_open(state, &reach_key(handover)).await? {
        creator
            .complete(state, ticket.id, english("wl_payments.cash.reach_done"))
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handover() -> CashHandover {
        CashHandover {
            id: 3,
            holder_user_id: 7,
            responsible_user_id: 7,
            period_date: "2027-04-01".parse().unwrap(),
            ..Default::default()
        }
    }

    #[test]
    fn the_shortage_goes_to_the_holder_and_the_manager_once_each() {
        assert_eq!(shortage_recipients(&handover(), Some(9)), [7, 9]);
        assert_eq!(shortage_recipients(&handover(), Some(7)), [7]);
        assert_eq!(shortage_recipients(&handover(), None), [7]);
        let mut till = handover();
        till.responsible_user_id = 8;
        assert_eq!(shortage_recipients(&till, Some(9)), [7, 8, 9]);
    }

    #[test]
    fn the_period_reads_as_words() {
        assert_eq!(period_text(&handover()), "Day 2027-04-01");
        let mut run = handover();
        run.period_kind = wl_kernel::custody::PeriodKind::Run;
        run.period_type = Some("delivery.run".to_owned());
        run.period_id = Some(12);
        assert_eq!(period_text(&run), "Run 12 of 2027-04-01");
    }

    #[test]
    fn the_types_have_english_templates() {
        for def in [shortage_type(), task_type()] {
            assert!(def.template_for(NotifyChannel::Email, "en").is_some());
        }
    }
}
