//! Auto-close after resolve (`WRK.RUL.02`, wl-ticketing §7): a resolved
//! ticket whose requester did nothing for N days (3 unless
//! [`WlTicketing::auto_close_days`](crate::WlTicketing::auto_close_days) says
//! otherwise) is confirmed by the system and closed. It counts as the
//! requester's `confirm`: `TicketClosed` is emitted and the invoice draft of
//! a category that invoices on close is made then. A ticket without a
//! requester has nobody to confirm; staff close it.
//!
//! The hourly task [`TASK`] runs [`run`].

use std::sync::Arc;

use renox::chrono::TimeDelta;
use renox::db::DateTime;
use renox::prelude::*;

use crate::ticket::{Ticket, TicketStatus};
use crate::work::auto_confirm;

/// The scheduled task and command.
pub const TASK: &str = "ticketing:auto_close";

/// Days after `resolve` before the system confirms: §7 default.
pub const DEFAULT_DAYS: i64 = 3;

/// The configured number of days (provided by the module).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AutoCloseDays(pub i64);

/// The days a resolved ticket waits for its requester.
pub fn days(state: &AppState) -> i64 {
    state
        .provided::<AutoCloseDays>()
        .map_or(DEFAULT_DAYS, |days: Arc<AutoCloseDays>| days.0)
}

/// Whether the resolved `ticket` is due for the auto-close at `at`: it has a
/// requester and was resolved `days` or more days ago.
pub fn is_due(ticket: &Ticket, days: i64, at: DateTime) -> bool {
    ticket.status == TicketStatus::Resolved
        && ticket.requester().is_some()
        && ticket
            .resolved_at
            .is_some_and(|resolved| resolved + TimeDelta::days(days) <= at)
}

/// Confirms, as the system, every resolved ticket that is due at `at`.
/// Returns how many were closed. A ticket another run closed meanwhile is
/// skipped.
///
/// # Errors
///
/// The database's errors; a refusal other than "no longer resolved".
pub async fn run(state: &AppState, at: DateTime) -> Result<usize> {
    let days = days(state);
    let resolved = Ticket::query()
        .where_(Ticket::STATUS.eq(TicketStatus::Resolved))
        .order_by(Ticket::ID)
        .get(&state.db)
        .await?;
    let mut closed = 0;
    for ticket in resolved.iter().filter(|t| is_due(t, days, at)) {
        match auto_confirm(state, ticket, at).await {
            Ok(()) => closed += 1,
            Err(renox::state_machine::TransitionError::NotFrom(_)) => {}
            Err(other) => return Err(other.into_error()),
        }
    }
    Ok(closed)
}

/// Registers the hourly task and its command.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule().hourly(TASK, |state: AppState| async move {
        run(&state, renox::db::now()).await.map(|_| ())
    });
    app.command(
        TASK,
        "Confirm the resolved tickets whose requester did nothing for N days (WRK.RUL.02)",
        |_args, state: AppState| async move {
            let closed = run(&state, renox::db::now()).await?;
            println!("{closed} tickets closed.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::{TimeZone, Utc};

    fn at(day: u32) -> DateTime {
        Utc.with_ymd_and_hms(2026, 11, day, 9, 0, 0).unwrap()
    }

    fn resolved(day: u32, requester: Option<i64>) -> Ticket {
        Ticket {
            status: TicketStatus::Resolved,
            resolved_at: Some(at(day)),
            requester_user_id: requester,
            ..Ticket::default()
        }
    }

    #[test]
    fn a_resolved_ticket_is_due_after_n_days() {
        let ticket = resolved(3, Some(7));
        assert!(!is_due(&ticket, 3, at(5)));
        assert!(is_due(&ticket, 3, at(6)));
        assert!(is_due(&ticket, 3, at(9)));
    }

    #[test]
    fn a_ticket_without_a_requester_or_not_resolved_is_never_due() {
        assert!(!is_due(&resolved(3, None), 3, at(20)));
        let mut open = resolved(3, Some(7));
        open.status = TicketStatus::InProgress;
        assert!(!is_due(&open, 3, at(20)));
    }
}
