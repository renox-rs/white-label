//! Vendor no-shows (`WRK.RUL.19`, wl-ticketing §4.1, §6, issue #150): a
//! vendor that has not started a ticket M hours after its visit was
//! scheduled.
//!
//! **When.** M is 2 elapsed clock hours (not working hours: a visit at 17:00
//! is a no-show at 19:00) after the ticket's *visit scheduled at*
//! ([`WlTicketing::no_show_hours`](crate::WlTicketing::no_show_hours)); a
//! ticket with no appointment is a no-show at its *response due* time itself,
//! with no extra M. **If** the ticket is still `assigned` (not started, not
//! `on_hold`) to a vendor party; staff assignees never.
//!
//! **Then** a [`NoShow`] is recorded against the vendor and the contract the
//! ticket is handled under (the vendor only, for an ad hoc vendor),
//! `VendorNoShow` is emitted, and the coordinator is told with the actions to
//! reassign (`assign`, reason `vendor_no_show`) or to set a **new time** with
//! the same vendor ([`new_time`], which tells the requester the new time); for
//! a compliance-critical category the manager in scope is told too. Nothing is
//! reassigned automatically. The contract page counts the no-shows per vendor.
//!
//! **Occurrences.** One rule run per ticket and scheduled time
//! (`WRK.RUL.19:<scheduled>`), so a visit time changed before the trigger
//! skips the old occurrence, and a new time with the same vendor can be a
//! no-show again. The rule is checked every ten minutes. A start entered late
//! after a no-show was recorded does not take it back.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use renox::chrono::TimeDelta;
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::calendar::calendar;
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::parties::directory;
use wl_kernel::tickets::{ServiceContractId, VendorNoShow};

use crate::assign::{AssignRequest, assign};
use crate::config::Category;
use crate::texts::english;
use crate::ticket::{Ticket, TicketStatus};
use crate::visit_check::{holders_in, settings};

/// The rule id.
pub const RULE: &str = "WRK.RUL.19";

/// The scheduled task and command.
pub const TASK: &str = "ticketing:no_show";

/// The clock hours after the visit time before a vendor counts as a no-show
/// (§6).
pub const DEFAULT_HOURS: i64 = 2;

/// The notice to the coordinator (and the manager).
pub const NOTIFICATION_TYPE: &str = "ticketing.vendor_no_show";

/// The configured hours (provided by the module).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NoShowHours(pub i64);

/// The hours a vendor may be late.
pub fn hours(state: &AppState) -> i64 {
    state
        .provided::<NoShowHours>()
        .map_or(DEFAULT_HOURS, |hours: Arc<NoShowHours>| hours.0)
}

/// A vendor that did not start a ticket in time.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_ticketing_no_shows",
    unique(ticket_id, scheduled_at),
    index(contract_id),
    index(vendor_party_id)
)]
pub struct NoShow {
    /// The primary key.
    pub id: i64,
    /// The ticket.
    #[model(references = "wl_ticketing_tickets")]
    pub ticket_id: i64,
    /// The vendor party.
    pub vendor_party_id: i64,
    /// The contract the ticket is handled under, if it is.
    pub contract_id: Option<i64>,
    /// The visit time (or response due time) the vendor missed.
    pub scheduled_at: DateTime,
    /// When the rule recorded it.
    pub recorded_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl NoShow {
    /// The vendor.
    pub fn vendor(&self) -> PartyId {
        PartyId(self.vendor_party_id)
    }

    /// The no-shows recorded under `contract`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_contract(db: &Db, contract: i64) -> Result<Vec<NoShow>> {
        NoShow::query()
            .where_(NoShow::CONTRACT_ID.eq(contract))
            .order_by(NoShow::SCHEDULED_AT)
            .order_by(NoShow::ID)
            .get(db)
            .await
    }

    /// The no-show recorded for the visit time `ticket` has now, if any: the
    /// one the coordinator still has to answer.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn current_for(db: &Db, ticket: &Ticket) -> Result<Option<NoShow>> {
        let Some(scheduled) = scheduled_for(ticket) else {
            return Ok(None);
        };
        if ticket.status != TicketStatus::Assigned || ticket.assignee_party_id.is_none() {
            return Ok(None);
        }
        NoShow::query()
            .where_(NoShow::TICKET_ID.eq(ticket.id))
            .where_(NoShow::SCHEDULED_AT.eq(scheduled))
            .first(db)
            .await
    }
}

/// The time the vendor is expected: the visit scheduled at, else the response
/// due time.
pub fn scheduled_for(ticket: &Ticket) -> Option<DateTime> {
    ticket.visit_at.or(ticket.response_due)
}

/// When the vendor of `ticket` becomes a no-show: `hours` clock hours after
/// the visit time, or the response due time itself without an appointment.
pub fn trigger_at(ticket: &Ticket, hours: i64) -> Option<DateTime> {
    match ticket.visit_at {
        Some(visit) => Some(visit + TimeDelta::hours(hours)),
        None => ticket.response_due,
    }
}

/// Whether the vendor of `ticket` is a no-show at `now`: still `assigned` to
/// a vendor party and past its trigger time.
pub fn is_due(ticket: &Ticket, hours: i64, now: DateTime) -> bool {
    ticket.status == TicketStatus::Assigned
        && ticket.assignee_party_id.is_some()
        && trigger_at(ticket, hours).is_some_and(|at| at <= now)
}

/// The occurrence key of the run for `scheduled`.
fn occurrence(scheduled: DateTime) -> String {
    format!("{RULE}:{}", scheduled.timestamp())
}

/// The notification type [`NOTIFICATION_TYPE`].
pub fn notification_type() -> NotificationTypeDef {
    let subject = english("wl_ticketing.notice.vendor_no_show.subject").to_owned();
    let body = english("wl_ticketing.notice.vendor_no_show.body").to_owned();
    NotificationTypeDef::new(NOTIFICATION_TYPE, "ticketing")
        .group("tickets")
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// How a time reads in a notice: Asia/Dubai, e.g. `03 Nov 2026 10:00`.
fn time_text(state: &AppState, at: DateTime) -> String {
    calendar(state)
        .local(at)
        .format("%d %b %Y %H:%M")
        .to_string()
}

/// Records the no-show of `ticket` for `scheduled` at `at`, emits
/// `VendorNoShow` and tells the coordinator (and for a compliance-critical
/// category the manager). `false` when it was recorded already.
async fn record(
    state: &AppState,
    ticket: &Ticket,
    scheduled: DateTime,
    at: DateTime,
) -> Result<bool> {
    let Some(vendor) = ticket.assignee_party_id else {
        return Ok(false);
    };
    let row = NoShow {
        ticket_id: ticket.id,
        vendor_party_id: vendor,
        contract_id: ticket.contract_id,
        scheduled_at: scheduled,
        recorded_at: at,
        ..NoShow::default()
    };
    match NoShow::create(&state.db, row).await {
        Ok(_) => {}
        // Recorded by an earlier attempt of the same occurrence.
        Err(error) if error.is_unique_violation() => return Ok(false),
        Err(error) => return Err(error),
    }
    state
        .emit(VendorNoShow {
            ticket: ticket.ticket_id(),
            vendor: PartyId(vendor),
            contract: ticket.contract_id.map(ServiceContractId),
            scheduled_at: scheduled,
        })
        .await?;

    let settings = settings(state);
    let critical = Category::find(&state.db, ticket.category_id)
        .await?
        .is_some_and(|c| c.compliance_critical);
    let subject = ticket.ticket_id().subject();
    let mut recipients = holders_in(state, &subject, &settings.coordinator_role).await?;
    if critical {
        recipients.extend(holders_in(state, &subject, &settings.manager_role).await?);
        recipients.sort_unstable();
        recipients.dedup();
    }
    let vendor_name = directory(state)
        .display_name(&state.db, PartyId(vendor), "en")
        .await?
        .unwrap_or_else(|| format!("#{vendor}"));
    for user in recipients {
        let request = NotifyRequest::new(NOTIFICATION_TYPE, subject.clone())
            .to_user(user)
            .data(json!({
                "number": ticket.number,
                "title": ticket.title,
                "vendor": vendor_name,
                "scheduled": time_text(state, scheduled),
                "hours": hours(state),
                "link": format!("/tickets/{}", ticket.id),
            }));
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(true)
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// No-shows recorded.
    pub recorded: usize,
}

/// Records the no-show of every vendor that is late at `now`: one rule run
/// per ticket and scheduled time.
///
/// # Errors
///
/// The database's errors.
pub async fn run(state: &AppState, now: DateTime) -> Result<Report> {
    let hours = hours(state);
    let assigned = Ticket::query()
        .where_(Ticket::STATUS.eq(TicketStatus::Assigned))
        .where_(Ticket::ASSIGNEE_PARTY_ID.is_not_null())
        .order_by(Ticket::ID)
        .get(&state.db)
        .await?;
    let recorded = Arc::new(AtomicUsize::new(0));
    for ticket in assigned.into_iter().filter(|t| is_due(t, hours, now)) {
        let Some(scheduled) = scheduled_for(&ticket) else {
            continue;
        };
        let spec = RuleRunSpec::new(RULE, ticket.ticket_id().subject(), occurrence(scheduled));
        let count = Arc::clone(&recorded);
        run_rule(state, spec, move |state: AppState| async move {
            if record(&state, &ticket, scheduled, now).await? {
                count.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        })
        .await?;
    }
    Ok(Report {
        recorded: recorded.load(Ordering::SeqCst),
    })
}

/// `user` sets a new visit time with the same vendor: a visit change, which
/// tells the requester the new time. The vendor may be a no-show again
/// `hours` after the new time.
///
/// # Errors
///
/// `Forbidden` without `tickets.assign`; `Guard` (422) for a ticket that has
/// no assignee, the same time, or an assignee who cannot take work; `NotFrom`
/// unless it is `assigned`.
pub async fn new_time(
    state: &AppState,
    user: &AuthUser,
    ticket: &Ticket,
    visit_at: DateTime,
) -> std::result::Result<(), TransitionError> {
    let Some(assignee) = ticket.assignee() else {
        return Err(TransitionError::Guard(
            "The ticket has nobody to visit.".to_owned(),
        ));
    };
    let request = AssignRequest {
        assignee,
        priority: ticket.priority,
        visit_at: Some(visit_at),
        reason_code: None,
        reason_text: None,
    };
    assign(state, user, ticket, request, renox::db::now()).await?;
    Ok(())
}

/// Registers the notice, the ten-minute task and its command.
pub(crate) fn register(app: &mut Registry) {
    notify::register_type(notification_type());
    app.schedule()
        .every_minutes(10, TASK, |state: AppState| async move {
            run(&state, renox::db::now()).await.map(|_| ())
        });
    app.command(
        TASK,
        "Record the vendors that did not start a ticket in time (WRK.RUL.19)",
        |_args, state: AppState| async move {
            let report = run(&state, renox::db::now()).await?;
            println!("{} vendor no-shows recorded.", report.recorded);
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::BusinessCalendar;

    fn dubai(date: &str, hour: u32, minute: u32) -> DateTime {
        let local = date
            .parse::<renox::chrono::NaiveDate>()
            .expect("a date")
            .and_hms_opt(hour, minute, 0)
            .expect("a time");
        BusinessCalendar::default().from_local(local)
    }

    fn visit(status: TicketStatus) -> Ticket {
        Ticket {
            status,
            assignee_party_id: Some(5),
            visit_at: Some(dubai("2026-11-03", 10, 0)),
            ..Ticket::default()
        }
    }

    #[test]
    fn a_vendor_is_a_no_show_two_clock_hours_after_the_visit_time() {
        let ticket = visit(TicketStatus::Assigned);
        assert!(!is_due(&ticket, 2, dubai("2026-11-03", 11, 59)));
        assert!(is_due(&ticket, 2, dubai("2026-11-03", 12, 0)));
        assert!(is_due(&ticket, 2, dubai("2026-11-04", 9, 0)));
    }

    #[test]
    fn the_clock_does_not_stop_after_working_hours() {
        let mut ticket = visit(TicketStatus::Assigned);
        ticket.visit_at = Some(dubai("2026-11-03", 17, 0));
        assert_eq!(trigger_at(&ticket, 2), Some(dubai("2026-11-03", 19, 0)));
    }

    #[test]
    fn without_an_appointment_the_response_due_time_is_the_trigger() {
        let mut ticket = visit(TicketStatus::Assigned);
        ticket.visit_at = None;
        ticket.response_due = Some(dubai("2026-11-03", 15, 0));
        assert_eq!(trigger_at(&ticket, 2), Some(dubai("2026-11-03", 15, 0)));
        assert!(is_due(&ticket, 2, dubai("2026-11-03", 15, 0)));
        ticket.response_due = None;
        assert_eq!(trigger_at(&ticket, 2), None);
        assert!(!is_due(&ticket, 2, dubai("2027-01-01", 9, 0)));
    }

    #[test]
    fn only_an_assigned_ticket_of_a_vendor_counts() {
        let late = dubai("2026-11-04", 9, 0);
        for status in [
            TicketStatus::InProgress,
            TicketStatus::OnHold,
            TicketStatus::Resolved,
            TicketStatus::Cancelled,
        ] {
            assert!(!is_due(&visit(status), 2, late), "{status}");
        }
        let mut staff = visit(TicketStatus::Assigned);
        staff.assignee_party_id = None;
        staff.assignee_user_id = Some(7);
        assert!(!is_due(&staff, 2, late), "never a staff assignee");
    }

    #[test]
    fn the_occurrence_follows_the_scheduled_time() {
        let a = occurrence(dubai("2026-11-03", 10, 0));
        let b = occurrence(dubai("2026-11-03", 15, 0));
        assert_ne!(a, b);
        assert!(a.starts_with("WRK.RUL.19:"));
    }

    #[test]
    fn the_notice_has_an_english_text() {
        let def = notification_type();
        let template = def
            .template_for(NotifyChannel::Email, "en")
            .expect("a text");
        assert!(!template.body.starts_with("wl_ticketing."));
        assert!(
            template
                .subject
                .as_deref()
                .unwrap_or_default()
                .contains("{{ number }}")
        );
    }
}
