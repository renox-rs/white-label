//! Tickets owned by a role in a scope (BL-TKT-12, wl-ticketing §5.1 point 2,
//! §8 `TicketRouter`).
//!
//! When a ticket is opened without an assignee, [`route_new`] asks the
//! category's router ([`wl_kernel::ticket_routing::ticket_router`]), else
//! takes the category's default assignee role in the subject's scope:
//!
//! - holders found: `route_to_role` (system) makes the ticket `assigned`,
//!   owned by the role, and starts the SLA as any first assignment does;
//!   every holder is told ([`crate::notices::ASSIGNED`]);
//! - nobody holds the role there: the ticket stays `open` and the company
//!   admins are told ([`UNROUTED`]);
//! - no role: the ticket stays `open` for the coordinator.
//!
//! Any holder may [`take`] the ticket: a reassignment to themselves with the
//! reason `taken_from_queue`, which keeps the SLA. A ticket a user took
//! stays with that user. A role ticket is shown only to the holders of the
//! role in its scope ([`allowed_keys`], [`crate::visibility`]).

use renox::db::{DateTime, ToDbValue, Transaction};
use renox::prelude::*;
use renox::serde_json::{self, Value};
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::notify::{self, Category as NoticeCategory, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::scopes::COMPANY_ADMIN_ROLE;
use wl_kernel::ticket_routing::{RoleAssignee, default_route, ticket_router};
use wl_kernel::tickets::{Assignee, OpenTicket};

use crate::assign::{
    AssignInput, AssignKind, AssignMeta, Assignment, assigning, classify, due_times,
};
use crate::config::Category;
use crate::priority::Priority;
use crate::ticket::{Ticket, TicketStatus};

/// The system transition that gives an `open` ticket to a role.
pub const ROUTE_TO_ROLE: &str = "route_to_role";

/// A holder of the role takes the ticket from the queue.
pub const TAKE: &str = "take";

/// The reason code of a take.
pub const TAKEN_FROM_QUEUE: &str = "taken_from_queue";

/// The notice to the company admins that a ticket found nobody in its role.
pub const UNROUTED: &str = "ticketing.unrouted";

/// What `route_to_role` carries to its effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RouteInput {
    /// When it happens.
    pub at: DateTime,
    /// The role and scope.
    pub role: RoleAssignee,
    /// The role and scope for people.
    pub display: String,
    /// The priority.
    pub priority: Priority,
    /// The response due time.
    #[serde(default)]
    pub response_due: Option<DateTime>,
    /// The resolution due time.
    #[serde(default)]
    pub resolution_due: Option<DateTime>,
}

fn route_input(ctx: &TransitionCtx) -> Option<RouteInput> {
    ctx.input_as::<RouteInput>().ok()
}

/// `route_to_role` for the machine: open → assigned, by the system.
pub(crate) fn route_transition() -> Transition<Ticket> {
    Transition::new(ROUTE_TO_ROLE)
        .label("Assign to role (system)")
        .system()
        .from([TicketStatus::Open])
        .to(TicketStatus::Assigned)
        .guard("input", |_ticket: &Ticket, ctx| {
            let present = route_input(ctx).is_some();
            Box::pin(async move {
                if present {
                    Ok(())
                } else {
                    Err("The action was not prepared by the ticket service.".to_owned())
                }
            })
        })
        .effect(|ticket: &Ticket, ctx, tx| {
            let input = route_input(ctx);
            Box::pin(async move {
                let input = input.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a route without its input"))
                })?;
                apply_route(tx, ticket, &input).await
            })
        })
}

/// `take` for the machine: a holder of the role reassigns the role's ticket
/// to themselves, only through [`take`] (which checks the holder).
pub(crate) fn take_transition() -> Transition<Ticket> {
    assigning(Transition::new(TAKE).label("Take").tone("primary"))
}

/// The effect of `route_to_role`: the role on the ticket, the SLA start and
/// the history row.
async fn apply_route(tx: &mut Transaction, ticket: &Ticket, input: &RouteInput) -> Result<()> {
    let role = &input.role;
    let permission = role.permission.clone();
    let key = role.key();
    let values: Vec<(&str, &(dyn ToDbValue + Sync))> = vec![
        ("assignee_role", &role.role),
        ("assignee_role_label", &role.label),
        ("assignee_role_permission", &permission),
        ("assignee_scope_type", &role.scope_kind),
        ("assignee_scope_id", &role.scope_id),
        ("assignee_role_key", &key),
        ("assignee_role_display", &input.display),
        ("priority", &input.priority),
    ];
    Ticket::query()
        .where_(Ticket::ID.eq(ticket.id))
        .update(&mut *tx, &values)
        .await?;
    let started = Some(input.at);
    Ticket::query()
        .where_(Ticket::ID.eq(ticket.id))
        .where_(Ticket::SLA_STARTED_AT.is_null())
        .update(
            &mut *tx,
            &[
                ("sla_started_at", &started),
                ("response_due", &input.response_due),
                ("resolution_due", &input.resolution_due),
            ],
        )
        .await?;
    let mut row = Assignment {
        ticket_id: ticket.id,
        kind: AssignKind::First,
        reason_code: Some(ROUTE_TO_ROLE.to_owned()),
        reason_text: Some(input.display.clone()),
        at: input.at,
        ..Assignment::default()
    };
    row.insert(&mut *tx).await?;
    Ok(())
}

/// The role a new ticket of `category` goes to: its router's answer, else
/// the category's default role in the subject's scope, else none.
///
/// # Errors
///
/// The database's errors.
pub async fn role_for(
    state: &AppState,
    category: &Category,
    request: &OpenTicket,
) -> Result<Option<RoleAssignee>> {
    if let Some(router) = ticket_router(&category.key) {
        return router.route(state, request).await;
    }
    match category
        .default_assignee_role
        .as_deref()
        .map(str::trim)
        .filter(|role| !role.is_empty())
    {
        Some(role) => Ok(Some(default_route(&state.db, role, request).await?)),
        None => Ok(None),
    }
}

/// Routes the new `open` `ticket` (opened by `request` without an assignee)
/// to its role, at `at`: see the module documentation. Returns the role it
/// went to, or `None` when it stays `open`.
///
/// # Errors
///
/// The database's errors; a refused transition.
pub async fn route_new(
    state: &AppState,
    ticket: &Ticket,
    category: &Category,
    request: &OpenTicket,
    at: DateTime,
) -> Result<Option<RoleAssignee>> {
    let Some(role) = role_for(state, category, request).await? else {
        return Ok(None);
    };
    let holders = role.holders(&state.db).await?;
    if holders.is_empty() {
        tell_admins(state, ticket, &role).await?;
        return Ok(None);
    }
    let display = role.display(&state.db).await?;
    let (response_due, resolution_due) =
        due_times(state, ticket, category.default_priority, at).await?;
    let input = RouteInput {
        at,
        role: role.clone(),
        display: display.clone(),
        priority: category.default_priority,
        response_due,
        resolution_due,
    };
    let _outcome = ticket
        .transition(ROUTE_TO_ROLE)
        .by_system()
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    let ticket = Ticket::find_or_404(&state.db, ticket.id).await?;
    let mut notice = NotifyRequest::new(crate::notices::ASSIGNED, ticket.ticket_id().subject())
        .subject_label(&ticket.number)
        .data(json!({
            "number": ticket.number,
            "title": ticket.title,
            "priority": ticket.priority.name(),
            "visit": "",
            "assignee": display,
            "response_due": due_text(state, ticket.response_due),
            "resolution_due": due_text(state, ticket.resolution_due),
        }));
    for holder in &holders {
        notice = notice.to_user(holder.id);
    }
    notify::notifier(state).notify(state, notice).await?;
    Ok(Some(role))
}

fn due_text(state: &AppState, at: Option<DateTime>) -> String {
    at.map(|at| {
        wl_kernel::calendar::calendar(state)
            .local(at)
            .format("%Y-%m-%d %H:%M")
            .to_string()
    })
    .unwrap_or_default()
}

/// Tells the company admins that `ticket` stays `open`: nobody holds `role`
/// in its scope.
async fn tell_admins(state: &AppState, ticket: &Ticket, role: &RoleAssignee) -> Result<()> {
    let admins = renox::auth::permissions::users_with_role(&state.db, COMPANY_ADMIN_ROLE).await?;
    if admins.is_empty() {
        return Ok(());
    }
    let mut notice = NotifyRequest::new(UNROUTED, ticket.ticket_id().subject())
        .subject_label(&ticket.number)
        .data(json!({
            "number": ticket.number,
            "title": ticket.title,
            "role": role.display(&state.db).await?,
        }));
    for admin in admins {
        notice = notice.to_user(admin.id);
    }
    notify::notifier(state).notify(state, notice).await?;
    Ok(())
}

/// The notification type of [`UNROUTED`].
pub fn unrouted_type() -> NotificationTypeDef {
    NotificationTypeDef::new(UNROUTED, "ticketing")
        .category(NoticeCategory::Transactional)
        .channels(&[NotifyChannel::Email])
        .template(
            NotifyChannel::Email,
            "en",
            Some("Ticket {{ number }} has nobody to work on it"),
            "Ticket {{ number }} ({{ title }}) stays open: nobody holds the role {{ role }}. Give the role to someone or assign the ticket.",
        )
        .template(
            NotifyChannel::Email,
            "ar",
            Some("لا يوجد من يعمل على التذكرة {{ number }}"),
            "تبقى التذكرة {{ number }} ({{ title }}) مفتوحة: لا أحد يحمل الدور {{ role }}. امنح الدور لأحد أو أسند التذكرة.",
        )
}

/// Registers [`UNROUTED`].
pub(crate) fn register() {
    notify::register_type(unrouted_type());
}

/// Why a take was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TakeRefused {
    /// The ticket is not owned by a role (a user took it, or nobody has it).
    NotARoleTicket,
    /// The user does not hold the role in the ticket's scope.
    NotAHolder,
}

/// `user` takes the role ticket `ticket` at `at`: a reassignment to them
/// with the reason `taken_from_queue`; the SLA started at the routing is
/// unchanged. After the commit, the notices and `TicketAssigned` follow.
///
/// # Errors
///
/// `Guard` (422) when the ticket is not a role ticket or the user does not
/// hold the role in its scope; `NotFrom` unless it is `assigned`.
pub async fn take(
    state: &AppState,
    user: &AuthUser,
    ticket: &Ticket,
    at: DateTime,
) -> std::result::Result<(), TransitionError> {
    if ticket.status != TicketStatus::Assigned {
        return Err(TransitionError::NotFrom(ticket.status.to_string()));
    }
    let Some(role) = ticket.role_assignee() else {
        return Err(TransitionError::Guard(
            "This ticket is not waiting in a role queue.".to_owned(),
        ));
    };
    if !role.is_held_by(&state.db, user.id).await? {
        return Err(TransitionError::Guard(
            "Only a holder of the role can take this ticket.".to_owned(),
        ));
    }
    let assignee = Assignee::user(user.id);
    classify(ticket, assignee, ticket.visit_at).map_err(TransitionError::Guard)?;
    let input = AssignInput {
        at,
        assignee,
        priority: ticket.priority,
        visit_at: ticket.visit_at,
        reason_code: Some(TAKEN_FROM_QUEUE.to_owned()),
        reason_text: ticket.assignee_role_display.clone(),
        response_due: None,
        resolution_due: None,
    };
    let meta = AssignMeta {
        kind: AssignKind::Reassign.as_str().to_owned(),
        previous_assignee: None,
        assignee,
        reason_code: Some(TAKEN_FROM_QUEUE.to_owned()),
        reason_text: ticket.assignee_role_display.clone(),
        visit_at: ticket.visit_at,
    };
    let _outcome = ticket
        .transition(TAKE)
        .by(user)
        .input(&input)
        .reason(TAKEN_FROM_QUEUE)
        .meta(serde_json::to_value(&meta).unwrap_or(Value::Null))
        .run(state)
        .await?;
    Ok(())
}

/// The role queues `user` may see: the keys of the role tickets whose role
/// the user holds in its scope.
///
/// # Errors
///
/// The database's errors.
pub async fn allowed_keys(db: &Db, user_id: i64) -> Result<Vec<String>> {
    let tickets = Ticket::query()
        .where_(Ticket::ASSIGNEE_ROLE_KEY.is_not_null())
        .get(db)
        .await?;
    let mut seen: Vec<String> = Vec::new();
    let mut allowed = Vec::new();
    for ticket in tickets {
        let (Some(key), Some(role)) = (ticket.assignee_role_key.clone(), ticket.role_assignee())
        else {
            continue;
        };
        if seen.contains(&key) {
            continue;
        }
        seen.push(key.clone());
        if role.is_held_by(db, user_id).await? {
            allowed.push(key);
        }
    }
    Ok(allowed)
}

/// The queue of user `user_id`: the open tickets owned by a role they hold
/// in its scope, oldest first.
///
/// # Errors
///
/// The database's errors.
pub async fn queue_of(db: &Db, user_id: i64) -> Result<Vec<Ticket>> {
    let keys = allowed_keys(db, user_id).await?;
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let tickets = Ticket::query()
        .where_(Ticket::ASSIGNEE_ROLE_KEY.is_in(keys))
        .order_by(Ticket::ID)
        .get(db)
        .await?;
    Ok(tickets.into_iter().filter(Ticket::is_open).collect())
}
