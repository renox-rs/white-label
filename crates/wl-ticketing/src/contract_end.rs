//! When a service contract ends (`WRK.RUL.18`, wl-ticketing §4.6, §5.6 step
//! 6, issue #149).
//!
//! A contract ends without renewal when it `expire`s, or on the termination
//! date of a `terminate` (at once when that is today; a contract terminated
//! with a future date keeps covering its subjects until then). [`handle`]
//! then, once per contract end:
//!
//! - **cancels** the open planned visits due *after* the end date (`open`,
//!   `assigned` or `on_hold`; the system `cancel` with the reason
//!   `contract_ended`, and the vendor is told); a visit due *on* the end date
//!   is not cancelled;
//! - leaves the visits already `in_progress` to finish, and reactive tickets
//!   alone;
//! - opens **one task** for the coordinator listing the other open visits
//!   (due on or before the end date), to reassign or cancel on the contract
//!   page ([`crate::contract_pages`]); due one working day later; no task
//!   when there are none. A category set to *reassign only*
//!   ([`Category::contract_end_reassign_only`](crate::Category)) offers no
//!   cancel there;
//! - **ends the contract's maintenance plans**;
//! - for each covered subject and compliance-critical category, clears a
//!   `vendor_document_expired` flag and, unless another `active` contract
//!   covers the same subject and category, raises `no_contract` and tells the
//!   manager in scope;
//! - marks the end as handled (`ended_at`) and emits `ServiceContractEnded`.
//!
//! The steps are repeatable: a run that failed half way is finished by the
//! next one (the nightly run, [`run`]), and only the run that marks the end
//! emits the event. The nightly run is the compliance-critical rule
//! `WRK.RUL.18` (`PLT.RUL.04`): one rule run per date, so a failed or missed
//! one alerts the business owner. It also moves a contract into its renewal
//! window (`WRK.RUL.12`) and expires one whose end date has passed
//! (`WRK.RUL.13`).

use std::sync::{Arc, Mutex, PoisonError};

use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::TransitionError;
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::calendar::calendar;
use wl_kernel::compliance::{CriticalRule, occurrence_key, schedule_subject};
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{ComplianceFlagReason, ServiceContractEnded, TicketId};

use crate::config::Category;
use crate::contract::{ContractStatus, ServiceContract, ServiceContracts, is_open_visit};
use crate::flags::{self, CONTRACT_ENDED, Raise};
use crate::plans;
use crate::texts::english;
use crate::ticket::{Ticket, TicketStatus};
use crate::visit_check::{holders_in, settings};
use crate::work;

/// The rule id.
pub const RULE: &str = "WRK.RUL.18";

/// The scheduled task and command.
pub const TASK: &str = "ticketing:contract_end";

/// When the nightly run happens, Asia/Dubai.
pub const TASK_AT: &str = "01:45";

/// The reassignment and cancel reason of a visit of an ended contract.
pub const CONTRACT_ENDED_REASON: &str = "contract_ended";

/// The notice to the vendor of a cancelled visit.
pub const VISIT_CANCELLED_NOTICE: &str = "ticketing.visit_cancelled";

/// The notice to the manager in scope of a subject left without a contract.
pub const NO_CONTRACT_NOTICE: &str = "ticketing.no_contract";

/// The key the coordinator's task of contract `id` is found by while open.
pub fn task_key(id: i64) -> String {
    format!("ticketing.contract_end:{id}")
}

/// What [`handle`] did for one contract end.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EndReport {
    /// The contract.
    pub contract: i64,
    /// The planned visits cancelled because they were due after the end date.
    pub cancelled: Vec<i64>,
    /// The open visits left to the coordinator.
    pub left: Vec<i64>,
    /// The `no_contract` flags raised.
    pub flags: usize,
    /// The maintenance plans ended.
    pub plans_ended: usize,
    /// The coordinator's task, when one was opened.
    pub task: Option<i64>,
}

/// The notification types of this rule.
pub fn notification_types() -> [NotificationTypeDef; 2] {
    let of = |key: &str, texts: &str| {
        let subject = english(&format!("wl_ticketing.notice.{texts}.subject")).to_owned();
        let body = english(&format!("wl_ticketing.notice.{texts}.body")).to_owned();
        NotificationTypeDef::new(key, "ticketing")
            .group("tickets")
            .template(NotifyChannel::Email, "en", Some(&subject), &body)
    };
    [
        of(VISIT_CANCELLED_NOTICE, "visit_cancelled"),
        of(NO_CONTRACT_NOTICE, "no_contract"),
    ]
}

/// Cancels the open planned visits of `contract` due after `end`; returns
/// nothing, the committed rows tell which were cancelled.
async fn cancel_late_visits(
    state: &AppState,
    contract: &ServiceContract,
    end: NaiveDate,
) -> Result<()> {
    for visit in contract.visits(&state.db).await? {
        let late = visit.planned_due_date.is_some_and(|due| due > end);
        if !late || !is_open_visit(&visit) {
            continue;
        }
        match work::auto_cancel(state, &visit, CONTRACT_ENDED_REASON).await {
            Ok(()) => {
                if let Some(assignee) = visit.assignee() {
                    let request =
                        NotifyRequest::new(VISIT_CANCELLED_NOTICE, visit.ticket_id().subject())
                            .to(assignee.recipient())
                            .data(json!({
                                "number": visit.number,
                                "title": visit.title,
                                "contract": contract.number,
                            }));
                    notify::notifier(state).notify(state, request).await?;
                }
            }
            // Started or cancelled meanwhile.
            Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => {}
            Err(other) => return Err(other.into_error()),
        }
    }
    Ok(())
}

/// One line of the coordinator's task for `visit`.
fn task_line(visit: &Ticket) -> String {
    let due = visit
        .planned_due_date
        .map(|d| d.to_string())
        .unwrap_or_default();
    format!("{} - {} (due {due})", visit.number, visit.title)
}

/// Opens the coordinator's task for the `left` visits of `contract`, unless
/// one is open already. Returns its ticket id.
async fn open_task(
    state: &AppState,
    contract: &ServiceContract,
    left: &[Ticket],
) -> Result<Option<i64>> {
    if left.is_empty() {
        return Ok(None);
    }
    let settings = settings(state);
    let title =
        english("wl_ticketing.contract_end.task_title").replace("{number}", &contract.number);
    let lines: Vec<String> = left.iter().map(task_line).collect();
    let description = format!(
        "{}\n{}\n/service-contracts/{}",
        english("wl_ticketing.contract_end.task_description"),
        lines.join("\n"),
        contract.id
    );
    let due = calendar(state).add_working_days(renox::db::now(), 1);
    let task = RoleTask::new(
        contract.subject(),
        &settings.coordinator_role,
        &title,
        &task_key(contract.id),
    )
    .description(&description)
    .due_at(due);
    Ok(role_tasks::open(state, task)
        .await?
        .map(|ticket| ticket.id.0))
}

/// Clears and raises the compliance flags of the ended `contract`; returns
/// how many `no_contract` flags were raised.
async fn settle_flags(
    state: &AppState,
    contract: &ServiceContract,
    at: renox::db::DateTime,
) -> Result<usize> {
    let manager_role = settings(state).manager_role;
    let mut raised = 0;
    for row in contract.coverage(&state.db).await? {
        let Some(category) = Category::find(&state.db, row.category_id).await? else {
            continue;
        };
        if !category.compliance_critical {
            continue;
        }
        let subject = row.subject();
        // The vendor's document no longer matters: the contract is over.
        flags::clear_matching(
            state,
            &subject,
            category.id,
            &[ComplianceFlagReason::VendorDocumentExpired],
            CONTRACT_ENDED,
            at,
        )
        .await?;
        if ServiceContracts::has_replacement(&state.db, &subject, category.id, contract.id).await? {
            continue;
        }
        let flag = flags::raise(
            state,
            Raise {
                subject: subject.clone(),
                category: category.key.clone(),
                reason: ComplianceFlagReason::NoContract,
                contract: Some(contract.id),
                ticket: None,
            },
            at,
        )
        .await?;
        if flag.is_none() {
            continue;
        }
        raised += 1;
        for user in holders_in(state, &subject, &manager_role).await? {
            let request = NotifyRequest::new(NO_CONTRACT_NOTICE, contract.subject())
                .to_user(user)
                .data(json!({
                    "contract": contract.number,
                    "subject": format!("{} #{}", subject.subject_type, subject.subject_id),
                    "category": category.name_en,
                }));
            notify::notifier(state).notify(state, request).await?;
        }
    }
    Ok(raised)
}

/// Handles the end of contract `contract_id` on the Asia/Dubai date `today`
/// (`WRK.RUL.18`): see the module. `None` when the contract is unknown, has
/// not ended by `today` or was handled already; the report of the run
/// otherwise.
///
/// # Errors
///
/// The database's errors, or a notice or a transition that failed; the next
/// run finishes what is left.
pub async fn handle(
    state: &AppState,
    contract_id: i64,
    today: NaiveDate,
) -> Result<Option<EndReport>> {
    let Some(contract) = ServiceContract::find(&state.db, contract_id).await? else {
        return Ok(None);
    };
    if !contract.end_is_due(today) {
        return Ok(None);
    }
    let at = renox::db::now();
    let end = contract.ending_on();

    cancel_late_visits(state, &contract, end).await?;
    let visits = contract.visits(&state.db).await?;
    let cancelled: Vec<Ticket> = visits
        .iter()
        .filter(|v| {
            v.status == TicketStatus::Cancelled
                && v.cancel_reason.as_deref() == Some(CONTRACT_ENDED_REASON)
                && v.planned_due_date.is_some_and(|due| due > end)
        })
        .cloned()
        .collect();
    let left: Vec<Ticket> = visits
        .iter()
        .filter(|v| is_open_visit(v) && v.planned_due_date.is_some_and(|due| due <= end))
        .cloned()
        .collect();

    let task = open_task(state, &contract, &left).await?;
    let plans_ended = plans::end_of_contract(state, contract.id).await?;
    let flags = settle_flags(state, &contract, at).await?;

    // Only the run that marks the end reports it.
    let marked = ServiceContract::query()
        .where_(ServiceContract::ID.eq(contract.id))
        .where_(ServiceContract::ENDED_AT.is_null())
        .update(&state.db, &[("ended_at", &Some(at))])
        .await?;
    let report = EndReport {
        contract: contract.id,
        cancelled: cancelled.iter().map(|v| v.id).collect(),
        left: left.iter().map(|v| v.id).collect(),
        flags,
        plans_ended,
        task,
    };
    if marked == 0 {
        return Ok(None);
    }
    let outcome = match contract.status {
        ContractStatus::Terminated => "terminated",
        _ => "expired",
    };
    state
        .emit(ServiceContractEnded {
            contract: contract.contract_id(),
            outcome: outcome.to_owned(),
            new_version: None,
            visits_cancelled: cancelled
                .iter()
                .map(Ticket::ticket_id)
                .collect::<Vec<TicketId>>(),
            visits_left: left.iter().map(Ticket::ticket_id).collect(),
        })
        .await?;
    Ok(Some(report))
}

/// What a nightly run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NightlyReport {
    /// Contracts moved into their renewal window.
    pub flagged_expiring: usize,
    /// Contracts expired.
    pub expired: usize,
    /// Contract ends handled.
    pub ended: Vec<EndReport>,
}

/// The action of the rule run: the renewal windows reached, the end dates
/// passed, and every end not handled yet.
async fn nightly(state: &AppState, today: NaiveDate, report: &mut NightlyReport) -> Result<()> {
    let active = ServiceContract::query()
        .where_(ServiceContract::STATUS.eq(ContractStatus::Active))
        .order_by(ServiceContract::ID)
        .get(&state.db)
        .await?;
    for contract in active
        .iter()
        .filter(|c| today >= c.end_date - Duration::days(c.renewal_notice_days))
    {
        match ServiceContracts::flag_expiring(state, contract, today).await {
            Ok(()) => report.flagged_expiring += 1,
            Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => {}
            Err(other) => return Err(other.into_error()),
        }
    }
    let expiring = ServiceContract::query()
        .where_(ServiceContract::STATUS.eq(ContractStatus::Expiring))
        .order_by(ServiceContract::ID)
        .get(&state.db)
        .await?;
    for contract in expiring.iter().filter(|c| today > c.end_date) {
        match ServiceContracts::expire(state, contract, today).await {
            Ok(done) => {
                report.expired += 1;
                report.ended.extend(done);
            }
            Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => {}
            Err(other) => return Err(other.into_error()),
        }
    }
    let ended = ServiceContract::query()
        .where_(ServiceContract::ENDED_AT.is_null())
        .where_(
            ServiceContract::STATUS.is_in([ContractStatus::Expired, ContractStatus::Terminated]),
        )
        .order_by(ServiceContract::ID)
        .get(&state.db)
        .await?;
    for contract in ended.iter().filter(|c| c.end_is_due(today)) {
        if let Some(done) = handle(state, contract.id, today).await? {
            report.ended.push(done);
        }
    }
    Ok(())
}

/// Runs `WRK.RUL.18` for `today` (Asia/Dubai): one compliance-critical rule
/// run per date, so a second run for the same date does nothing and a failed
/// one is retried and alerts the business owner.
///
/// # Errors
///
/// The database's errors.
pub async fn run(state: &AppState, today: NaiveDate) -> Result<NightlyReport> {
    let spec = RuleRunSpec::new(RULE, schedule_subject(), occurrence_key(RULE, today))
        .compliance_critical();
    let seen = Arc::new(Mutex::new(NightlyReport::default()));
    let out = Arc::clone(&seen);
    run_rule(state, spec, move |state: AppState| async move {
        let mut report = NightlyReport::default();
        let result = nightly(&state, today, &mut report).await;
        *out.lock().unwrap_or_else(PoisonError::into_inner) = report;
        result
    })
    .await?;
    let report = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Ok(report)
}

/// Marks the rule compliance-critical, registers the notices, the nightly
/// task and its command.
pub(crate) fn register(app: &mut Registry) {
    CriticalRule::new(RULE).daily_at(TASK_AT).register();
    for def in notification_types() {
        notify::register_type(def);
    }
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, wl_kernel::documents::today()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        TASK,
        "Handle the service contracts that ended: open visits, plans and compliance flags (WRK.RUL.18)",
        |_args, state: AppState| async move {
            let report = run(&state, wl_kernel::documents::today()).await?;
            println!(
                "{} contracts flagged as expiring, {} expired, {} ends handled.",
                report.flagged_expiring,
                report.expired,
                report.ended.len()
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_is_found_by_the_contract() {
        assert_eq!(task_key(7), "ticketing.contract_end:7");
        assert_eq!(RULE, "WRK.RUL.18");
        assert_eq!(CONTRACT_ENDED_REASON, "contract_ended");
    }

    #[test]
    fn both_notices_have_an_english_text() {
        for def in notification_types() {
            let template = def
                .template_for(NotifyChannel::Email, "en")
                .expect("a text");
            let subject = template.subject.as_deref().unwrap_or_default();
            assert!(!subject.starts_with("wl_ticketing."), "{}", def.key);
            assert!(!template.body.starts_with("wl_ticketing."), "{}", def.key);
        }
    }

    #[test]
    fn a_task_line_names_the_visit_and_its_planned_date() {
        let visit = Ticket {
            number: "TKT-2026-0042".into(),
            title: "Quarterly lift visit".into(),
            planned_due_date: NaiveDate::from_ymd_opt(2026, 12, 20),
            ..Ticket::default()
        };
        assert_eq!(
            task_line(&visit),
            "TKT-2026-0042 - Quarterly lift visit (due 2026-12-20)"
        );
    }
}
