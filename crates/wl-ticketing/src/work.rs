//! Working a ticket (wl-ticketing §4.1): `start`, `hold`, `resume`,
//! `resolve`, `confirm`, `close`, `reopen` and `cancel`, with the SLA effects
//! of #146 A8 and A9. (`assign` comes with the reassignment.)
//!
//! A transition's effects get no `AppState`, so the services here (each
//! takes the time `at` it happens, which routes pass as
//! `renox::db::now()`) work out what needs the calendar and the database,
//! and pass it in a [`WorkInput`]:
//!
//! - `start` sets `responded_at`.
//! - `hold` needs a known hold reason; one that pauses the SLA sets
//!   `sla_paused_at`.
//! - `resume` moves `resolution_due` later by the working minutes paused and
//!   clears `sla_paused_at`; so does `reopen`, counting from `resolved_at`.
//!   Both add the minutes to `sla_paused_minutes`, which `TicketClosed`
//!   leaves out of the resolution time.
//! - `resolve` sets `resolved_at`; `reopen` clears it.
//!
//! Who may run them: `start`, `hold`, `resume` and `resolve` are for the
//! assignee user (holding `tickets.work`), or a holder of `tickets.assign`
//! acting for a vendor party without a login; `confirm` only for the
//! requester user; `close` and `cancel` need `tickets.assign`; `reopen` is
//! for the requester user or `tickets.assign`, within seven days of
//! `resolved_at`. A bare run without the input (the generic transition
//! route) is refused by the guards. After a `confirm`, `close` or
//! system `auto_confirm` or `complete` commits, `TicketClosed` is emitted.

use renox::chrono::TimeDelta;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{
    StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::tickets::{Assignee, TicketClosed};

use crate::comments::{Comment, CommentVisibility};
use crate::config::{Category, HoldReason};
use crate::permissions;
use crate::sla::{SlaCalendar, sla_calendar};
use crate::ticket::{Ticket, TicketStatus};

/// `start`: the assignee begins the work.
pub const START: &str = "start";
/// `hold`: the work is paused for a hold reason.
pub const HOLD: &str = "hold";
/// `resume`: the work goes on.
pub const RESUME: &str = "resume";
/// `resume`, run by the system.
pub const AUTO_RESUME: &str = "auto_resume";
/// `resolve`: the work is done.
pub const RESOLVE: &str = "resolve";
/// `confirm`: the requester signs the work off.
pub const CONFIRM: &str = "confirm";
/// `close`: staff close a resolved ticket.
pub const CLOSE: &str = "close";
/// `confirm`, run by the system when the requester does nothing
/// (`WRK.RUL.02`).
pub const AUTO_CONFIRM: &str = "auto_confirm";
/// `reopen`: a resolved ticket goes back to work.
pub const REOPEN: &str = "reopen";
/// `cancel`: the ticket is not needed.
pub const CANCEL: &str = "cancel";
/// `resolve`, run by the system for the record that opened the ticket.
pub const RESOLVE_FROM_SOURCE: &str = "resolve_from_source";
/// `close`, run by the system for the record that opened the ticket.
pub const COMPLETE: &str = "complete";
/// `cancel`, run by the system (e.g. the contract ended).
pub const AUTO_CANCEL: &str = "auto_cancel";

/// How many days after `resolved_at` a ticket may be reopened (§7).
pub const REOPEN_DAYS: i64 = 7;

/// What a lifecycle run carries to its guards and effects, worked out by the
/// services in this module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WorkInput {
    /// When the action happens.
    pub at: DateTime,
    /// The hold reason chosen, when it is known (`hold`).
    #[serde(default)]
    pub hold_reason_id: Option<i64>,
    /// Whether that reason pauses the SLA (`hold`).
    #[serde(default)]
    pub pauses_sla: bool,
    /// The resolution due time after the clock shift (`resume`, `reopen`).
    #[serde(default)]
    pub resolution_due: Option<DateTime>,
    /// The working minutes the clock stood still (`resume`, `reopen`).
    #[serde(default)]
    pub paused_minutes: i64,
    /// Whether the category needs the requester's sign-off (`close`).
    #[serde(default)]
    pub requires_signoff: bool,
    /// How many charge lines the ticket has (`resolve`): a chargeable ticket
    /// needs at least one.
    #[serde(default)]
    pub charge_lines: u64,
}

impl WorkInput {
    /// An input for an action at `at`, with nothing else set.
    fn at(at: DateTime) -> Self {
        WorkInput {
            at,
            hold_reason_id: None,
            pauses_sla: false,
            resolution_due: None,
            paused_minutes: 0,
            requires_signoff: false,
            charge_lines: 0,
        }
    }
}

/// The run's input, if it is a [`WorkInput`].
fn input(ctx: &TransitionCtx) -> Option<WorkInput> {
    ctx.input_as::<WorkInput>().ok()
}

/// The user running a transition and what they may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Who {
    id: i64,
    can_work: bool,
    can_assign: bool,
}

/// The actor of `ctx`, if a user runs it.
fn who(ctx: &TransitionCtx) -> Option<Who> {
    ctx.actor().map(|user| Who {
        id: user.id,
        can_work: user.has_permission(permissions::WORK),
        can_assign: user.has_permission(permissions::ASSIGN),
    })
}

/// Whether `who` works `ticket`: its assignee user holding `tickets.work`,
/// or a holder of `tickets.assign` acting for a vendor party (which has no
/// login).
fn works_on(ticket: &Ticket, who: Option<Who>) -> bool {
    let Some(who) = who else { return false };
    match ticket.assignee() {
        Some(Assignee::User { id }) => id == who.id && who.can_work,
        Some(Assignee::Party { .. }) => who.can_assign,
        None => false,
    }
}

/// Whether `who` is the user who asked for `ticket`.
fn is_requester(ticket: &Ticket, who: Option<Who>) -> bool {
    matches!((ticket.requester_user_id, who), (Some(r), Some(w)) if r == w.id)
}

/// Whether `at` is within the reopen window after `resolved_at`.
fn in_reopen_window(resolved_at: DateTime, at: DateTime) -> bool {
    at <= resolved_at + TimeDelta::days(REOPEN_DAYS)
}

/// The resolution due time after the clock stood still from `from` to `to`
/// in working time, and the working minutes that took.
fn shifted_due(
    calendar: &dyn SlaCalendar,
    due: Option<DateTime>,
    from: DateTime,
    to: DateTime,
) -> (Option<DateTime>, i64) {
    let minutes = calendar.working_minutes_between(from, to);
    let due = due.map(|due| {
        if minutes > 0 {
            calendar.add_working(due, minutes)
        } else {
            due
        }
    });
    (due, minutes)
}

/// The [`TicketClosed`] event for a closed `ticket`: the working minutes
/// from the SLA start to the resolution, paused time left out, and whether
/// it was resolved by its due time.
pub(crate) fn closed_event(ticket: &Ticket, calendar: &dyn SlaCalendar) -> TicketClosed {
    let resolution_minutes = match (ticket.sla_started_at, ticket.resolved_at) {
        (Some(started), Some(resolved)) => Some(
            (calendar.working_minutes_between(started, resolved) - ticket.sla_paused_minutes)
                .max(0),
        ),
        _ => None,
    };
    let within_sla = match (ticket.resolved_at, ticket.resolution_due) {
        (Some(resolved), Some(due)) => resolved <= due,
        _ => true,
    };
    TicketClosed {
        ticket: ticket.ticket_id(),
        resolution_minutes,
        within_sla,
    }
}

/// Adds the guard that refuses a run without a [`WorkInput`].
fn with_input(transition: Transition<Ticket>) -> Transition<Ticket> {
    transition.guard("input", |_ticket: &Ticket, ctx| {
        let present = input(ctx).is_some();
        Box::pin(async move {
            if present {
                Ok(())
            } else {
                Err("The action was not prepared by the ticket service.".to_owned())
            }
        })
    })
}

/// Adds the guard that only the assignee (or a coordinator for a vendor)
/// works the ticket.
fn by_assignee(transition: Transition<Ticket>) -> Transition<Ticket> {
    transition.guard("by_assignee", |ticket: &Ticket, ctx| {
        let allowed = works_on(ticket, who(ctx));
        Box::pin(async move {
            if allowed {
                Ok(())
            } else {
                Err("Only the assignee works this ticket.".to_owned())
            }
        })
    })
}

/// Stores `cancel_reason` and clears the pause, for both cancels.
fn cancelling(transition: Transition<Ticket>) -> Transition<Ticket> {
    transition
        .from([
            TicketStatus::Open,
            TicketStatus::Assigned,
            TicketStatus::OnHold,
        ])
        .to(TicketStatus::Cancelled)
        .reason_required()
        .effect(|ticket: &Ticket, ctx, tx| {
            let id = ticket.id;
            let reason = ctx.reason().map(|r| r.trim().to_owned());
            Box::pin(async move {
                let none: Option<DateTime> = None;
                // Not the status column, so outside the machine's guard.
                Ticket::query()
                    .where_(Ticket::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[("cancel_reason", &reason), ("sla_paused_at", &none)],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// Adds the effect that moves the resolution clock, for `resume` and
/// `reopen`: the due time and minutes come from the input.
fn shifting(transition: Transition<Ticket>, clear_resolved: bool) -> Transition<Ticket> {
    transition.effect(move |ticket: &Ticket, ctx, tx| {
        let id = ticket.id;
        let paused_before = ticket.sla_paused_minutes;
        let due = ticket.resolution_due;
        let input = input(ctx);
        Box::pin(async move {
            let input = input
                .ok_or_else(|| Error::from(renox::anyhow::anyhow!("a resume without its input")))?;
            let none: Option<DateTime> = None;
            let none_id: Option<i64> = None;
            let due = input.resolution_due.or(due);
            let minutes = paused_before + input.paused_minutes;
            let mut values: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = vec![
                ("resolution_due", &due),
                ("sla_paused_minutes", &minutes),
                ("sla_paused_at", &none),
                ("hold_reason_id", &none_id),
            ];
            if clear_resolved {
                values.push(("resolved_at", &none));
            }
            Ticket::query()
                .where_(Ticket::ID.eq(id))
                .update(&mut *tx, &values)
                .await?;
            Ok(())
        })
    })
}

/// Every lifecycle transition but `assign`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Ticket>> {
    vec![
        start_transition(),
        hold_transition(),
        resume_transition(),
        auto_resume_transition(),
        resolve_transition(),
        confirm_transition(),
        auto_confirm_transition(),
        close_transition(),
        reopen_transition(),
        cancel_transition(),
        auto_cancel_transition(),
        resolve_from_source_transition(),
        complete_transition(),
    ]
}

/// `start`: assigned → in_progress; the response SLA is met.
fn start_transition() -> Transition<Ticket> {
    by_assignee(with_input(
        Transition::new(START)
            .label("Start")
            .tone("primary")
            .from([TicketStatus::Assigned])
            .to(TicketStatus::InProgress),
    ))
    .effect(|ticket: &Ticket, ctx, tx| {
        let id = ticket.id;
        let at = input(ctx).map(|i| i.at);
        Box::pin(async move {
            Ticket::query()
                .where_(Ticket::ID.eq(id))
                .update(&mut *tx, &[("responded_at", &at)])
                .await?;
            Ok(())
        })
    })
}

/// `hold`: in_progress → on_hold, for a known hold reason; the SLA clock
/// pauses when the reason pauses it.
fn hold_transition() -> Transition<Ticket> {
    by_assignee(with_input(
        Transition::new(HOLD)
            .label("Put on hold")
            .tone("warning")
            .from([TicketStatus::InProgress])
            .to(TicketStatus::OnHold),
    ))
    .guard("hold_reason", |_ticket: &Ticket, ctx| {
        let known = input(ctx).is_some_and(|i| i.hold_reason_id.is_some());
        Box::pin(async move {
            if known {
                Ok(())
            } else {
                Err("Choose a hold reason.".to_owned())
            }
        })
    })
    .effect(|ticket: &Ticket, ctx, tx| {
        let id = ticket.id;
        let input = input(ctx);
        Box::pin(async move {
            let input = input
                .ok_or_else(|| Error::from(renox::anyhow::anyhow!("a hold without its input")))?;
            let paused_at = input.pauses_sla.then_some(input.at);
            Ticket::query()
                .where_(Ticket::ID.eq(id))
                .update(
                    &mut *tx,
                    &[
                        ("hold_reason_id", &input.hold_reason_id),
                        ("sla_paused_at", &paused_at),
                    ],
                )
                .await?;
            Ok(())
        })
    })
}

/// `resume`: on_hold → in_progress; the SLA clock goes on.
fn resume_transition() -> Transition<Ticket> {
    shifting(
        by_assignee(with_input(
            Transition::new(RESUME)
                .label("Resume")
                .tone("primary")
                .from([TicketStatus::OnHold])
                .to(TicketStatus::InProgress),
        )),
        false,
    )
}

/// `resume`, run by the system.
fn auto_resume_transition() -> Transition<Ticket> {
    shifting(
        with_input(
            Transition::new(AUTO_RESUME)
                .label("Resume (system)")
                .from([TicketStatus::OnHold])
                .to(TicketStatus::InProgress)
                .system(),
        ),
        false,
    )
}

/// `resolve`: in_progress → resolved. (The vendor document guard runs in
/// [`resolve`] before the transition, [`crate::vendor_guard`]; the after-photo
/// and custom field guards come with their issues.)
fn resolve_transition() -> Transition<Ticket> {
    by_assignee(with_input(
        Transition::new(RESOLVE)
            .label("Resolve")
            .tone("success")
            .from([TicketStatus::InProgress])
            .to(TicketStatus::Resolved),
    ))
    .guard("charge", |ticket: &Ticket, ctx| {
        // A chargeable ticket has at least one charge line (§4.1).
        let missing = ticket.chargeable && input(ctx).is_none_or(|i| i.charge_lines == 0);
        Box::pin(async move {
            if missing {
                Err(crate::texts::english("wl_ticketing.charge.needed").to_owned())
            } else {
                Ok(())
            }
        })
    })
    .effect(|ticket: &Ticket, ctx, tx| {
        let id = ticket.id;
        let at = input(ctx).map(|i| i.at);
        Box::pin(async move {
            Ticket::query()
                .where_(Ticket::ID.eq(id))
                .update(&mut *tx, &[("resolved_at", &at)])
                .await?;
            Ok(())
        })
    })
}

/// `resolve_from_source`, run by the system: open, assigned, in_progress or
/// on_hold → resolved, with the note as the resolution (#67 A8). The note is
/// kept as a comment the requester sees. Closing follows `close`/`confirm`.
fn resolve_from_source_transition() -> Transition<Ticket> {
    with_input(
        Transition::new(RESOLVE_FROM_SOURCE)
            .label("Resolve (source)")
            .tone("success")
            .from([
                TicketStatus::Open,
                TicketStatus::Assigned,
                TicketStatus::InProgress,
                TicketStatus::OnHold,
            ])
            .to(TicketStatus::Resolved)
            .reason_required()
            .system(),
    )
    .effect(|ticket: &Ticket, ctx, tx| {
        let id = ticket.id;
        let at = input(ctx).map(|i| i.at);
        let note = ctx.reason().map(|r| r.trim().to_owned());
        Box::pin(async move {
            Ticket::query()
                .where_(Ticket::ID.eq(id))
                .update(&mut *tx, &[("resolved_at", &at)])
                .await?;
            let mut comment = Comment {
                ticket_id: id,
                author_user_id: None,
                visibility: CommentVisibility::Requester,
                text: note.unwrap_or_default(),
                ..Comment::default()
            };
            comment.insert(&mut *tx).await?;
            Ok(())
        })
    })
}

/// `complete`, run by the system: open, assigned, in_progress or on_hold →
/// closed, with the note as the resolution. The work is done by the record
/// that opened the ticket, so there is nothing to sign off; `resolved_at` is
/// set so `TicketClosed` can report the resolution time.
fn complete_transition() -> Transition<Ticket> {
    with_input(
        Transition::new(COMPLETE)
            .label("Complete (source)")
            .tone("success")
            .from([
                TicketStatus::Open,
                TicketStatus::Assigned,
                TicketStatus::InProgress,
                TicketStatus::OnHold,
            ])
            .to(TicketStatus::Closed)
            .reason_required()
            .system(),
    )
    .effect(|ticket: &Ticket, ctx, tx| {
        let id = ticket.id;
        let at = input(ctx).map(|i| i.at);
        let note = ctx.reason().map(|r| r.trim().to_owned());
        Box::pin(async move {
            Ticket::query()
                .where_(Ticket::ID.eq(id))
                .update(&mut *tx, &[("resolved_at", &at)])
                .await?;
            let mut comment = Comment {
                ticket_id: id,
                author_user_id: None,
                visibility: CommentVisibility::Requester,
                text: note.unwrap_or_default(),
                ..Comment::default()
            };
            comment.insert(&mut *tx).await?;
            Ok(())
        })
    })
}

/// `confirm`: resolved → closed, by the requester user.
fn confirm_transition() -> Transition<Ticket> {
    Transition::new(CONFIRM)
        .label("Confirm")
        .tone("success")
        .from([TicketStatus::Resolved])
        .to(TicketStatus::Closed)
        .guard("by_requester", |ticket: &Ticket, ctx| {
            let allowed = is_requester(ticket, who(ctx));
            Box::pin(async move {
                if allowed {
                    Ok(())
                } else {
                    Err("Only the requester confirms the work.".to_owned())
                }
            })
        })
}

/// `confirm`, run by the system (`WRK.RUL.02`): resolved → closed when the
/// requester did nothing for N days. It counts as the requester's `confirm`.
fn auto_confirm_transition() -> Transition<Ticket> {
    with_input(
        Transition::new(AUTO_CONFIRM)
            .label("Confirm (system)")
            .tone("success")
            .from([TicketStatus::Resolved])
            .to(TicketStatus::Closed)
            .system(),
    )
}

/// `close`: resolved → closed, by `tickets.assign`, when the category needs
/// no sign-off or the ticket has no requester.
fn close_transition() -> Transition<Ticket> {
    with_input(
        Transition::new(CLOSE)
            .label("Close")
            .tone("success")
            .permission(permissions::ASSIGN)
            .from([TicketStatus::Resolved])
            .to(TicketStatus::Closed),
    )
    .guard("signoff", |ticket: &Ticket, ctx| {
        let allowed =
            ticket.requester().is_none() || input(ctx).is_some_and(|i| !i.requires_signoff);
        Box::pin(async move {
            if allowed {
                Ok(())
            } else {
                Err("The requester confirms this ticket; staff cannot close it.".to_owned())
            }
        })
    })
}

/// `reopen`: resolved → in_progress, by the requester user or
/// `tickets.assign`, within seven days of `resolved_at`; the resolution
/// clock goes on.
fn reopen_transition() -> Transition<Ticket> {
    shifting(
        with_input(
            Transition::new(REOPEN)
                .label("Reopen")
                .tone("warning")
                .from([TicketStatus::Resolved])
                .to(TicketStatus::InProgress),
        )
        .guard("by_requester_or_coordinator", |ticket: &Ticket, ctx| {
            let who = who(ctx);
            let allowed = is_requester(ticket, who) || who.is_some_and(|w| w.can_assign);
            Box::pin(async move {
                if allowed {
                    Ok(())
                } else {
                    Err("Only the requester or a coordinator reopens a ticket.".to_owned())
                }
            })
        })
        .guard("reopen_window", |ticket: &Ticket, ctx| {
            let open = match (ticket.resolved_at, input(ctx)) {
                (Some(resolved), Some(input)) => in_reopen_window(resolved, input.at),
                _ => false,
            };
            Box::pin(async move {
                if open {
                    Ok(())
                } else {
                    Err(format!(
                        "A ticket is reopened within {REOPEN_DAYS} days of being resolved."
                    ))
                }
            })
        }),
        true,
    )
}

/// `cancel`: open, assigned or on_hold → cancelled, by `tickets.assign`,
/// with a reason.
fn cancel_transition() -> Transition<Ticket> {
    cancelling(
        Transition::new(CANCEL)
            .label("Cancel")
            .tone("danger")
            .permission(permissions::ASSIGN),
    )
}

/// `cancel`, run by the system (`WRK.RUL.18`, the contract ended).
fn auto_cancel_transition() -> Transition<Ticket> {
    cancelling(
        Transition::new(AUTO_CANCEL)
            .label("Cancel (system)")
            .system(),
    )
}

/// `user` starts `ticket` at `at`.
///
/// # Errors
///
/// `Guard` (422) unless `user` is its assignee (or a coordinator acting for
/// a vendor); `NotFrom` unless it is `assigned`.
pub async fn start(
    state: &AppState,
    user: &AuthUser,
    ticket: &Ticket,
    at: DateTime,
) -> std::result::Result<(), TransitionError> {
    let _outcome = ticket
        .transition(START)
        .by(user)
        .input(&WorkInput::at(at))
        .run(state)
        .await?;
    Ok(())
}

/// `user` puts `ticket` on hold at `at` for the hold reason `reason_key`.
///
/// # Errors
///
/// `Guard` (422) for an unknown hold reason or a user who is not the
/// assignee; `NotFrom` unless it is `in_progress`.
pub async fn hold(
    state: &AppState,
    user: &AuthUser,
    ticket: &Ticket,
    reason_key: &str,
    at: DateTime,
) -> std::result::Result<(), TransitionError> {
    let reason = HoldReason::by_key(&state.db, reason_key.trim()).await?;
    let input = WorkInput {
        hold_reason_id: reason.as_ref().map(|r| r.id),
        pauses_sla: reason.is_some_and(|r| r.pauses_sla),
        ..WorkInput::at(at)
    };
    let _outcome = ticket
        .transition(HOLD)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Ok(())
}

/// What `resume` carries for `ticket` resumed at `at`: the due time moved
/// later by the working minutes paused.
fn resume_input(state: &AppState, ticket: &Ticket, at: DateTime) -> WorkInput {
    let calendar = sla_calendar(state);
    let (due, minutes) = match ticket.sla_paused_at {
        Some(paused_at) => shifted_due(calendar.as_ref(), ticket.resolution_due, paused_at, at),
        None => (ticket.resolution_due, 0),
    };
    WorkInput {
        resolution_due: due,
        paused_minutes: minutes,
        ..WorkInput::at(at)
    }
}

/// `user` resumes the held `ticket` at `at`; the resolution due time moves
/// later by the working minutes it was paused.
///
/// # Errors
///
/// `Guard` (422) for a user who is not the assignee; `NotFrom` unless it is
/// `on_hold`.
pub async fn resume(
    state: &AppState,
    user: &AuthUser,
    ticket: &Ticket,
    at: DateTime,
) -> std::result::Result<(), TransitionError> {
    let _outcome = ticket
        .transition(RESUME)
        .by(user)
        .input(&resume_input(state, ticket, at))
        .run(state)
        .await?;
    Ok(())
}

/// The system resumes the held `ticket` at `at`, as [`resume`] does.
///
/// # Errors
///
/// `NotFrom` unless it is `on_hold`.
pub async fn auto_resume(
    state: &AppState,
    ticket: &Ticket,
    at: DateTime,
) -> std::result::Result<(), TransitionError> {
    let _outcome = ticket
        .transition(AUTO_RESUME)
        .by_system()
        .input(&resume_input(state, ticket, at))
        .run(state)
        .await?;
    Ok(())
}

/// `user` resolves `ticket` at `at`.
///
/// # Errors
///
/// `Guard` (422) for a user who is not the assignee, or a chargeable ticket
/// without a charge line; `Failed` (422) while the assigned vendor's document
/// is expired or missing for a guarded category ([`crate::vendor_guard`]);
/// `NotFrom` unless it is `in_progress`.
pub async fn resolve(
    state: &AppState,
    user: &AuthUser,
    ticket: &Ticket,
    at: DateTime,
) -> std::result::Result<(), TransitionError> {
    let charge_lines = if ticket.chargeable {
        crate::charge::ChargeLine::query()
            .where_(crate::charge::ChargeLine::TICKET_ID.eq(ticket.id))
            .count(&state.db)
            .await?
    } else {
        0
    };
    let input = WorkInput {
        charge_lines,
        ..WorkInput::at(at)
    };
    // `WRK.ACL.11`: a category guarded by a vendor document kind is not
    // resolved while the assigned vendor's document is expired or missing.
    crate::vendor_guard::run(
        state,
        crate::vendor_guard::RESOLVE,
        ticket.id,
        wl_kernel::documents::dubai_date(at),
        Some(user.id),
        || async {
            // `PRP.ACL.15`: a compliance-critical planned visit needs the
            // vendor's service report, checked after the vendor's document.
            crate::job_cancel::ensure_service_report(state, ticket).await?;
            ticket
                .transition(RESOLVE)
                .by(user)
                .input(&input)
                .run(state)
                .await
                .map(|_| ())
        },
    )
    .await
}

/// The requester `user` confirms the work on `ticket`; it is `closed` and
/// `TicketClosed` follows the commit.
///
/// # Errors
///
/// `Guard` (422) for anyone but the requester user; `NotFrom` unless it is
/// `resolved`.
pub async fn confirm(
    state: &AppState,
    user: &AuthUser,
    ticket: &Ticket,
) -> std::result::Result<(), TransitionError> {
    let _outcome = ticket.transition(CONFIRM).by(user).run(state).await?;
    Ok(())
}

/// The system confirms the resolved `ticket` at `at` for a requester who did
/// nothing (`WRK.RUL.02`); it is `closed` and `TicketClosed` follows the
/// commit.
///
/// # Errors
///
/// `NotFrom` unless it is `resolved`.
pub async fn auto_confirm(
    state: &AppState,
    ticket: &Ticket,
    at: DateTime,
) -> std::result::Result<(), TransitionError> {
    let _outcome = ticket
        .transition(AUTO_CONFIRM)
        .by_system()
        .input(&WorkInput::at(at))
        .run(state)
        .await?;
    Ok(())
}

/// `user` closes the resolved `ticket`; `TicketClosed` follows the commit.
///
/// # Errors
///
/// `Forbidden` without `tickets.assign`; `Guard` (422) when its category
/// needs the requester's sign-off and it has a requester; `NotFrom` unless
/// it is `resolved`.
pub async fn close(
    state: &AppState,
    user: &AuthUser,
    ticket: &Ticket,
) -> std::result::Result<(), TransitionError> {
    // A category that is gone cannot waive the sign-off.
    let category = Category::find(&state.db, ticket.category_id).await?;
    let input = WorkInput {
        requires_signoff: category.is_none_or(|c| c.requires_signoff),
        ..WorkInput::at(renox::db::now())
    };
    let _outcome = ticket
        .transition(CLOSE)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Ok(())
}

/// `user` reopens the resolved `ticket` at `at`; the time since it was
/// resolved is added to the resolution due time in working minutes.
///
/// # Errors
///
/// `Guard` (422) for anyone but the requester user or a holder of
/// `tickets.assign`, or after seven days from `resolved_at`; `NotFrom` unless
/// it is `resolved`.
pub async fn reopen(
    state: &AppState,
    user: &AuthUser,
    ticket: &Ticket,
    at: DateTime,
) -> std::result::Result<(), TransitionError> {
    let calendar = sla_calendar(state);
    let (due, minutes) = match ticket.resolved_at {
        Some(resolved_at) => shifted_due(calendar.as_ref(), ticket.resolution_due, resolved_at, at),
        None => (ticket.resolution_due, 0),
    };
    let input = WorkInput {
        resolution_due: due,
        paused_minutes: minutes,
        ..WorkInput::at(at)
    };
    let _outcome = ticket
        .transition(REOPEN)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Ok(())
}

/// `user` cancels `ticket` for `reason`.
///
/// # Errors
///
/// `Forbidden` without `tickets.assign`; `ReasonRequired` without a reason;
/// `NotFrom` unless it is `open`, `assigned` or `on_hold`.
pub async fn cancel(
    state: &AppState,
    user: &AuthUser,
    ticket: &Ticket,
    reason: &str,
) -> std::result::Result<(), TransitionError> {
    // `PRP.ACL.15`: a compliance-critical planned visit is rescheduled, not
    // skipped; only the system cancels it when the contract ends.
    crate::job_cancel::ensure_not_skipped(state, ticket).await?;
    let _outcome = ticket
        .transition(CANCEL)
        .by(user)
        .reason(reason)
        .run(state)
        .await?;
    Ok(())
}

/// The system cancels `ticket` for `reason`.
///
/// # Errors
///
/// `ReasonRequired` without a reason; `NotFrom` unless it is `open`,
/// `assigned` or `on_hold`.
pub async fn auto_cancel(
    state: &AppState,
    ticket: &Ticket,
    reason: &str,
) -> std::result::Result<(), TransitionError> {
    let _outcome = ticket
        .transition(AUTO_CANCEL)
        .by_system()
        .reason(reason)
        .run(state)
        .await?;
    Ok(())
}

/// The system resolves `ticket` at `at` for the record that opened it, with
/// `note` as the resolution.
///
/// # Errors
///
/// `ReasonRequired` without a note; `NotFrom` unless it is `open`,
/// `assigned`, `in_progress` or `on_hold`.
pub async fn resolve_from_source(
    state: &AppState,
    ticket: &Ticket,
    note: &str,
    at: DateTime,
) -> std::result::Result<(), TransitionError> {
    let _outcome = ticket
        .transition(RESOLVE_FROM_SOURCE)
        .by_system()
        .reason(note)
        .input(&WorkInput::at(at))
        .run(state)
        .await?;
    Ok(())
}

/// The system completes `ticket` at `at` for the record that opened it, with
/// `note` as the resolution; `TicketClosed` follows the commit.
///
/// # Errors
///
/// `ReasonRequired` without a note; `NotFrom` unless it is `open`,
/// `assigned`, `in_progress` or `on_hold`.
pub async fn complete(
    state: &AppState,
    ticket: &Ticket,
    note: &str,
    at: DateTime,
) -> std::result::Result<(), TransitionError> {
    let _outcome = ticket
        .transition(COMPLETE)
        .by_system()
        .reason(note)
        .input(&WorkInput::at(at))
        .run(state)
        .await?;
    Ok(())
}

/// Registers the listener that emits [`TicketClosed`] after a `confirm`,
/// `close` or `complete` commits.
pub(crate) fn register_events(app: &mut Registry) {
    app.listen(|event: Transitioned<Ticket>, state: AppState| async move {
        if ![CONFIRM, AUTO_CONFIRM, CLOSE, COMPLETE].contains(&event.transition.as_str()) {
            return Ok(());
        }
        // The committed row: the times the lifecycle wrote.
        let ticket = Ticket::find_or_404(&state.db, event.key).await?;
        let calendar = sla_calendar(&state);
        state.emit(closed_event(&ticket, calendar.as_ref())).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;
    use wl_kernel::BusinessCalendar;

    /// A Dubai local time, as an instant.
    fn dubai(date: &str, hour: u32) -> DateTime {
        let local = date
            .parse::<NaiveDate>()
            .expect("a date")
            .and_hms_opt(hour, 0, 0)
            .expect("a time");
        BusinessCalendar::default().from_local(local)
    }

    fn run(input: &WorkInput) -> TransitionCtx {
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    fn ticket(status: TicketStatus) -> Ticket {
        Ticket {
            id: 1,
            status,
            assignee_user_id: Some(7),
            requester_user_id: Some(3),
            ..Ticket::default()
        }
    }

    const WHO_ASSIGNEE: Who = Who {
        id: 7,
        can_work: true,
        can_assign: false,
    };

    #[test]
    fn the_assignee_works_the_ticket_and_a_coordinator_works_for_a_vendor() {
        let by_user = ticket(TicketStatus::Assigned);
        assert!(works_on(&by_user, Some(WHO_ASSIGNEE)));
        // Another technician.
        let other = Who {
            id: 8,
            ..WHO_ASSIGNEE
        };
        assert!(!works_on(&by_user, Some(other)));
        // The assignee without `tickets.work`.
        let idle = Who {
            can_work: false,
            ..WHO_ASSIGNEE
        };
        assert!(!works_on(&by_user, Some(idle)));
        // A coordinator is not the assignee user.
        let coordinator = Who {
            id: 9,
            can_work: false,
            can_assign: true,
        };
        assert!(!works_on(&by_user, Some(coordinator)));
        // The system works nothing.
        assert!(!works_on(&by_user, None));

        let vendor = Ticket {
            assignee_user_id: None,
            assignee_party_id: Some(5),
            ..by_user
        };
        assert!(works_on(&vendor, Some(coordinator)));
        assert!(!works_on(&vendor, Some(other)));
        let unassigned = Ticket {
            assignee_party_id: None,
            ..vendor
        };
        assert!(!works_on(&unassigned, Some(coordinator)));
    }

    #[test]
    fn only_the_requester_user_confirms() {
        let ticket = ticket(TicketStatus::Resolved);
        let requester = Who {
            id: 3,
            can_work: false,
            can_assign: false,
        };
        assert!(is_requester(&ticket, Some(requester)));
        assert!(!is_requester(&ticket, Some(WHO_ASSIGNEE)));
        assert!(!is_requester(&ticket, None));
        let internal = Ticket {
            requester_user_id: None,
            ..ticket
        };
        assert!(!is_requester(&internal, Some(requester)));
    }

    #[test]
    fn reopening_works_on_day_seven_and_not_on_day_eight() {
        let resolved = dubai("2026-11-03", 10);
        assert!(in_reopen_window(resolved, dubai("2026-11-03", 11)));
        assert!(in_reopen_window(resolved, dubai("2026-11-10", 10)));
        assert!(!in_reopen_window(resolved, dubai("2026-11-10", 11)));
        assert!(!in_reopen_window(resolved, dubai("2026-11-11", 10)));
    }

    #[test]
    fn a_pause_moves_the_due_time_by_the_working_minutes_paused() {
        let calendar = BusinessCalendar::default();
        // Held Tuesday 10:00, resumed 12:00: two working hours.
        let due = Some(dubai("2026-11-05", 18));
        let (shifted, minutes) = shifted_due(
            &calendar,
            due,
            dubai("2026-11-03", 10),
            dubai("2026-11-03", 12),
        );
        assert_eq!(minutes, 120);
        // Thursday's closing plus two working hours is Friday 11:00.
        assert_eq!(shifted, Some(dubai("2026-11-06", 11)));
        // A pause across the weekend counts only working time.
        let (_, minutes) = shifted_due(
            &calendar,
            due,
            dubai("2026-11-06", 17),
            dubai("2026-11-09", 10),
        );
        assert_eq!(minutes, 120);
        // Nothing paused, or no due time.
        let (same, none) = shifted_due(
            &calendar,
            due,
            dubai("2026-11-07", 10),
            dubai("2026-11-08", 10),
        );
        assert_eq!((same, none), (due, 0));
        assert_eq!(
            shifted_due(
                &calendar,
                None,
                dubai("2026-11-03", 10),
                dubai("2026-11-03", 12)
            ),
            (None, 120)
        );
    }

    #[test]
    fn a_closed_ticket_reports_its_resolution_time_and_whether_it_was_in_sla() {
        let calendar = BusinessCalendar::default();
        let ticket = Ticket {
            id: 42,
            sla_started_at: Some(dubai("2026-11-03", 9)),
            resolution_due: Some(dubai("2026-11-05", 18)),
            resolved_at: Some(dubai("2026-11-04", 9)),
            sla_paused_minutes: 120,
            status: TicketStatus::Closed,
            ..Ticket::default()
        };
        let event = closed_event(&ticket, &calendar);
        assert_eq!(event.ticket, wl_kernel::tickets::TicketId(42));
        // One working day (540 minutes) less the two hours paused.
        assert_eq!(event.resolution_minutes, Some(540 - 120));
        assert!(event.within_sla);

        let late = Ticket {
            resolved_at: Some(dubai("2026-11-09", 9)),
            ..ticket.clone()
        };
        assert!(!closed_event(&late, &calendar).within_sla);

        let no_sla = Ticket {
            sla_started_at: None,
            resolution_due: None,
            ..ticket
        };
        let event = closed_event(&no_sla, &calendar);
        assert_eq!(event.resolution_minutes, None);
        assert!(event.within_sla);
    }

    #[renox::test]
    async fn a_bare_run_without_the_service_input_is_refused() {
        let machine = Ticket::machine();
        let bare = TransitionCtx::new(None);
        for name in [START, HOLD, RESUME, AUTO_RESUME, RESOLVE, CLOSE, REOPEN] {
            let transition = machine.find(name).expect("transition");
            assert!(
                matches!(
                    transition
                        .check_guards(&ticket(TicketStatus::Resolved), &bare)
                        .await,
                    Err(TransitionError::Guard(_))
                ),
                "{name}"
            );
        }
    }

    #[renox::test]
    async fn work_guards_refuse_a_system_actor_and_an_unknown_hold_reason() {
        let machine = Ticket::machine();
        let input = run(&WorkInput::at(dubai("2026-11-03", 10)));
        // `by_assignee` needs a user.
        for name in [START, HOLD, RESUME, RESOLVE] {
            let transition = machine.find(name).expect("transition");
            assert!(
                matches!(
                    transition
                        .check_guards(&ticket(TicketStatus::Assigned), &input)
                        .await,
                    Err(TransitionError::Guard(_))
                ),
                "{name}"
            );
        }
        // The system's resume has nobody to check.
        let auto = machine.find(AUTO_RESUME).expect("transition");
        assert!(
            auto.check_guards(&ticket(TicketStatus::OnHold), &input)
                .await
                .is_ok()
        );
    }

    #[renox::test]
    async fn close_needs_no_signoff_or_no_requester() {
        let machine = Ticket::machine();
        let close = machine.find(CLOSE).expect("transition");
        let resolved = ticket(TicketStatus::Resolved);
        let needs = run(&WorkInput {
            requires_signoff: true,
            ..WorkInput::at(dubai("2026-11-03", 10))
        });
        let not = run(&WorkInput::at(dubai("2026-11-03", 10)));
        assert!(matches!(
            close.check_guards(&resolved, &needs).await,
            Err(TransitionError::Guard(_))
        ));
        assert!(close.check_guards(&resolved, &not).await.is_ok());
        let internal = Ticket {
            requester_user_id: None,
            ..resolved
        };
        assert!(close.check_guards(&internal, &needs).await.is_ok());
    }

    #[test]
    fn reopen_checks_who_and_the_window() {
        let machine = Ticket::machine();
        let reopen = machine.find(REOPEN).expect("transition");
        assert_eq!(
            reopen.guard_names(),
            ["input", "by_requester_or_coordinator", "reopen_window"]
        );
    }
}

#[cfg(test)]
mod system_tests {
    use super::*;
    use renox::chrono::NaiveDate;
    use renox::testing::TestApp;
    use wl_kernel::BusinessCalendar;

    use crate::seed;

    fn dubai(date: &str, hour: u32) -> DateTime {
        let local = date
            .parse::<NaiveDate>()
            .expect("a date")
            .and_hms_opt(hour, 0, 0)
            .expect("a time");
        BusinessCalendar::default().from_local(local)
    }

    async fn app() -> TestApp {
        let app = TestApp::new(App::new().module(crate::WlTicketing::new())).await;
        seed::install(app.db()).await.expect("seeded");
        app
    }

    /// A ticket born in `status`, assigned to a vendor at Tuesday 09:00 with
    /// the resolution due on Thursday's closing.
    async fn born_in(
        app: &TestApp,
        number: &str,
        status: TicketStatus,
        paused_at: Option<DateTime>,
    ) -> Ticket {
        let general = Category::by_key(app.db(), "general")
            .await
            .unwrap()
            .unwrap();
        let mut ticket = Ticket {
            number: number.to_owned(),
            subject_type: "property.unit".into(),
            subject_id: 12,
            category_id: general.id,
            title: "Chiller".into(),
            assignee_party_id: Some(5),
            sla_started_at: Some(dubai("2026-11-03", 9)),
            response_due: Some(dubai("2026-11-03", 17)),
            resolution_due: Some(dubai("2026-11-05", 18)),
            sla_paused_at: paused_at,
            status,
            ..Ticket::default()
        };
        let mut tx = app.db().begin().await.unwrap();
        ticket
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        ticket
    }

    #[renox::test]
    async fn resuming_after_a_pausing_hold_moves_the_resolution_due_later() {
        let app = app().await;
        let held = born_in(
            &app,
            "TKT-2026-9001",
            TicketStatus::OnHold,
            Some(dubai("2026-11-03", 10)),
        )
        .await;
        auto_resume(app.state(), &held, dubai("2026-11-03", 12))
            .await
            .unwrap();
        let ticket = Ticket::find_or_404(app.db(), held.id).await.unwrap();
        assert_eq!(ticket.status, TicketStatus::InProgress);
        // Two working hours later than Thursday's closing: Friday 11:00.
        assert_eq!(ticket.resolution_due, Some(dubai("2026-11-06", 11)));
        assert_eq!(ticket.sla_paused_minutes, 120);
        assert_eq!(ticket.sla_paused_at, None);
        assert_eq!(ticket.hold_reason_id, None);
        // The response due and the SLA start are never touched.
        assert_eq!(ticket.response_due, held.response_due);
        assert_eq!(ticket.sla_started_at, held.sla_started_at);
    }

    #[renox::test]
    async fn resuming_after_a_hold_that_does_not_pause_keeps_the_due_time() {
        let app = app().await;
        let held = born_in(&app, "TKT-2026-9002", TicketStatus::OnHold, None).await;
        auto_resume(app.state(), &held, dubai("2026-11-03", 12))
            .await
            .unwrap();
        let ticket = Ticket::find_or_404(app.db(), held.id).await.unwrap();
        assert_eq!(ticket.status, TicketStatus::InProgress);
        assert_eq!(ticket.resolution_due, held.resolution_due);
        assert_eq!(ticket.sla_paused_minutes, 0);
    }

    #[renox::test]
    async fn a_cancel_needs_a_reason_and_is_not_for_a_started_ticket() {
        let app = app().await;
        let assigned = born_in(&app, "TKT-2026-9003", TicketStatus::Assigned, None).await;
        let blank = auto_cancel(app.state(), &assigned, "  ").await;
        assert!(matches!(blank, Err(TransitionError::ReasonRequired)));
        assert_eq!(
            Ticket::find_or_404(app.db(), assigned.id)
                .await
                .unwrap()
                .status,
            TicketStatus::Assigned
        );

        auto_cancel(app.state(), &assigned, "Contract ended")
            .await
            .unwrap();
        let ticket = Ticket::find_or_404(app.db(), assigned.id).await.unwrap();
        assert_eq!(ticket.status, TicketStatus::Cancelled);
        assert_eq!(ticket.cancel_reason.as_deref(), Some("Contract ended"));

        let started = born_in(&app, "TKT-2026-9004", TicketStatus::InProgress, None).await;
        let refused = auto_cancel(app.state(), &started, "No longer needed").await;
        assert!(matches!(refused, Err(TransitionError::NotFrom(_))));
        let again = auto_cancel(app.state(), &ticket, "Again").await;
        assert!(matches!(again, Err(TransitionError::NotFrom(_))));
    }

    #[renox::test]
    async fn the_system_resolves_from_the_source_with_the_note() {
        let app = app().await;
        let at = dubai("2026-11-04", 10);
        for (number, status) in [
            ("TKT-2026-9010", TicketStatus::Open),
            ("TKT-2026-9011", TicketStatus::Assigned),
            ("TKT-2026-9012", TicketStatus::InProgress),
            ("TKT-2026-9013", TicketStatus::OnHold),
        ] {
            let born = born_in(&app, number, status, None).await;
            let blank = resolve_from_source(app.state(), &born, " ", at).await;
            assert!(matches!(blank, Err(TransitionError::ReasonRequired)));
            resolve_from_source(app.state(), &born, " Credited AED 12 ", at)
                .await
                .unwrap();
            let ticket = Ticket::find_or_404(app.db(), born.id).await.unwrap();
            assert_eq!(ticket.status, TicketStatus::Resolved, "{number}");
            assert_eq!(ticket.resolved_at, Some(at));
            let comments = Comment::of_ticket(app.db(), born.id).await.unwrap();
            assert_eq!(comments.len(), 1);
            assert_eq!(comments[0].text, "Credited AED 12");
            assert_eq!(comments[0].author_user_id, None);
            assert_eq!(comments[0].visibility, CommentVisibility::Requester);
            // Not twice.
            let again = resolve_from_source(app.state(), &ticket, "x", at).await;
            assert!(matches!(again, Err(TransitionError::NotFrom(_))));
        }
    }

    #[renox::test]
    async fn a_user_transition_is_refused_to_the_system() {
        let app = app().await;
        let assigned = born_in(&app, "TKT-2026-9005", TicketStatus::Assigned, None).await;
        let run = assigned
            .transition(START)
            .by_system()
            .input(&WorkInput::at(dubai("2026-11-03", 10)))
            .run(app.state())
            .await;
        assert!(matches!(run, Err(TransitionError::Unauthenticated)));
        let bare = assigned
            .transition(CANCEL)
            .reason("x")
            .run(app.state())
            .await;
        assert!(matches!(bare, Err(TransitionError::Unauthenticated)));
    }
}
