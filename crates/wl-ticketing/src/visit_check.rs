//! Planned visits that were missed (`WRK.RUL.14`, wl-ticketing §6, issue
//! #148): N days after a planned visit's planned due date, a visit that is
//! not resolved is reported.
//!
//! The coordinator (the holders of the coordinator role in the ticket's
//! scope) is told once. For a **compliance-critical** category (01: fire and
//! life safety, lifts) the manager in scope is told too, and the compliance
//! flag `visit_missed` is raised on the visit's subject
//! ([`crate::flags::raise`]); it clears when the visit is resolved. A visit
//! cancelled by the coordinator is not looked at: a flag raised before stays
//! until the visit is resolved.
//!
//! **Days.** N is 3 calendar days (Asia/Dubai) unless
//! [`WlTicketing::visit_missed_days`](crate::WlTicketing::visit_missed_days)
//! says otherwise; the check runs every night. The planned due date is the
//! ticket's own field, so moving the appointment (*visit scheduled at*) does
//! not postpone it.
//!
//! The rule is **compliance-critical** (`PLT.RUL.04`): the nightly run is one
//! kernel rule run per date, so a failed or missed run alerts the business
//! owner.

use std::sync::{Arc, Mutex, PoisonError};

use renox::auth::permissions::users_with_role_in;
use renox::chrono::{Duration, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::compliance::{CriticalRule, occurrence_key, schedule_subject};
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::tickets::ComplianceFlagReason;
use wl_kernel::{COMPANY_ADMIN_ROLE, Subject, scopes};

use crate::config::Category;
use crate::flags::{self, Raise};
use crate::texts::english;
use crate::ticket::{Ticket, TicketStatus};

/// The rule id.
pub const RULE: &str = "WRK.RUL.14";

/// The scheduled task and command.
pub const TASK: &str = "ticketing:visit_check";

/// When the nightly check runs, Asia/Dubai.
pub const TASK_AT: &str = "01:30";

/// The days after the planned due date before a visit counts as missed (§6).
pub const DEFAULT_DAYS: i64 = 3;

/// The notice to the coordinator and the manager.
pub const NOTIFICATION_TYPE: &str = "ticketing.visit_missed";

/// The settings of the check, provided by the module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settings {
    pub days: i64,
    pub coordinator_role: String,
    pub manager_role: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            days: DEFAULT_DAYS,
            coordinator_role: "coordinator".to_owned(),
            manager_role: COMPANY_ADMIN_ROLE.to_owned(),
        }
    }
}

/// The settings of this app.
pub(crate) fn settings(state: &AppState) -> Settings {
    state
        .provided::<Settings>()
        .map_or_else(Settings::default, |s| (*s).clone())
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Visits found missed and reported.
    pub missed: usize,
    /// Compliance flags raised.
    pub flags: usize,
    /// Notices sent.
    pub notices: usize,
}

/// The notification type [`NOTIFICATION_TYPE`].
pub fn notification_type() -> NotificationTypeDef {
    let subject = english("wl_ticketing.notice.visit_missed.subject").to_owned();
    let body = english("wl_ticketing.notice.visit_missed.body").to_owned();
    NotificationTypeDef::new(NOTIFICATION_TYPE, "ticketing")
        .group("tickets")
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// Whether the visit `ticket` is missed on `today`: planned, not resolved,
/// closed or cancelled, not reported yet, and `days` or more days past its
/// planned due date.
pub fn is_missed(ticket: &Ticket, days: i64, today: NaiveDate) -> bool {
    ticket.missed_notified_at.is_none()
        && !matches!(
            ticket.status,
            TicketStatus::Resolved | TicketStatus::Closed | TicketStatus::Cancelled
        )
        && ticket
            .planned_due_date
            .is_some_and(|due| due + Duration::days(days) <= today)
}

/// The users holding `role` in the scope of `ticket`, once each.
async fn holders(state: &AppState, ticket: &Ticket, role: &str) -> Result<Vec<i64>> {
    holders_in(state, &ticket.ticket_id().subject(), role).await
}

/// The users holding `role` in the scope of `subject`, once each.
pub(crate) async fn holders_in(state: &AppState, subject: &Subject, role: &str) -> Result<Vec<i64>> {
    let scope = scopes::scope_of(&state.db, subject).await?;
    let mut ids: Vec<i64> = users_with_role_in(&state.db, role, &scope)
        .await?
        .into_iter()
        .map(|user| user.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// Reports the missed visit `ticket`: tells the coordinator, and for a
/// compliance-critical category the manager too and raises the flag.
async fn report_one(
    state: &AppState,
    ticket: &Ticket,
    category: &Category,
    settings: &Settings,
    at: DateTime,
    report: &mut Report,
) -> Result<()> {
    let mut recipients = holders(state, ticket, &settings.coordinator_role).await?;
    if category.compliance_critical {
        recipients.extend(holders(state, ticket, &settings.manager_role).await?);
        recipients.sort_unstable();
        recipients.dedup();
    }
    let planned = ticket
        .planned_due_date
        .map(|d| d.format("%-d %b %Y").to_string())
        .unwrap_or_default();
    for user in recipients {
        let request = NotifyRequest::new(NOTIFICATION_TYPE, ticket.ticket_id().subject())
            .to_user(user)
            .data(json!({
                "number": ticket.number,
                "title": ticket.title,
                "category": category.name_en,
                "planned_due": planned,
                "days": settings.days,
                "compliance_critical": category.compliance_critical,
            }));
        notify::notifier(state).notify(state, request).await?;
        report.notices += 1;
    }
    if category.compliance_critical {
        let raised = flags::raise(
            state,
            Raise {
                subject: ticket.subject(),
                category: category.key.clone(),
                reason: ComplianceFlagReason::VisitMissed,
                contract: None,
                ticket: Some(ticket.id),
            },
            at,
        )
        .await?;
        report.flags += usize::from(raised.is_some());
    }
    Ticket::query()
        .where_(Ticket::ID.eq(ticket.id))
        .update(&state.db, &[("missed_notified_at", &Some(at))])
        .await?;
    report.missed += 1;
    Ok(())
}

/// The action of the rule run: every missed visit on `today`.
async fn check(state: &AppState, today: NaiveDate, report: &mut Report) -> Result<()> {
    let settings = settings(state);
    let at = renox::db::now();
    let planned = Ticket::query()
        .where_(Ticket::PLANNED_DUE_DATE.is_not_null())
        .where_(Ticket::MISSED_NOTIFIED_AT.is_null())
        .order_by(Ticket::ID)
        .get(&state.db)
        .await?;
    for ticket in planned
        .iter()
        .filter(|t| is_missed(t, settings.days, today))
    {
        let Some(category) = Category::find(&state.db, ticket.category_id).await? else {
            continue;
        };
        report_one(state, ticket, &category, &settings, at, report).await?;
    }
    Ok(())
}

/// Runs `WRK.RUL.14` for `today` (Asia/Dubai): one compliance-critical rule
/// run per date, so a second run for the same date does nothing and a failed
/// one is retried and alerts the business owner.
///
/// # Errors
///
/// The database's errors.
pub async fn run(state: &AppState, today: NaiveDate) -> Result<Report> {
    let spec = RuleRunSpec::new(RULE, schedule_subject(), occurrence_key(RULE, today))
        .compliance_critical();
    let seen = Arc::new(Mutex::new(Report::default()));
    let out = Arc::clone(&seen);
    run_rule(state, spec, move |state: AppState| async move {
        let mut report = Report::default();
        let result = check(&state, today, &mut report).await;
        *out.lock().unwrap_or_else(PoisonError::into_inner) = report;
        result
    })
    .await?;
    let report = *seen.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(report)
}

/// Marks the rule compliance-critical, registers the notice, the nightly
/// task and its command.
pub(crate) fn register(app: &mut Registry) {
    CriticalRule::new(RULE).daily_at(TASK_AT).register();
    notify::register_type(notification_type());
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, wl_kernel::documents::today()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        TASK,
        "Report the planned visits not resolved N days after their planned due date (WRK.RUL.14)",
        |_args, state: AppState| async move {
            let report = run(&state, wl_kernel::documents::today()).await?;
            println!(
                "{} visits missed, {} flags raised, {} notices sent.",
                report.missed, report.flags, report.notices
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    fn visit(due: u32, status: TicketStatus) -> Ticket {
        Ticket {
            planned_due_date: Some(day(due)),
            status,
            ..Ticket::default()
        }
    }

    #[test]
    fn a_visit_is_missed_three_days_after_its_planned_due_date() {
        let ticket = visit(1, TicketStatus::Assigned);
        assert!(!is_missed(&ticket, 3, day(3)));
        assert!(is_missed(&ticket, 3, day(4)));
        assert!(is_missed(&ticket, 3, day(10)));
        assert!(!is_missed(&ticket, 5, day(4)));
    }

    #[test]
    fn a_resolved_cancelled_or_reported_visit_is_not_missed() {
        for status in [
            TicketStatus::Resolved,
            TicketStatus::Closed,
            TicketStatus::Cancelled,
        ] {
            assert!(!is_missed(&visit(1, status), 3, day(10)));
        }
        let mut reported = visit(1, TicketStatus::Open);
        reported.missed_notified_at = Some(renox::db::now());
        assert!(!is_missed(&reported, 3, day(10)));
        assert!(!is_missed(&Ticket::default(), 3, day(10)), "not planned");
    }
}
