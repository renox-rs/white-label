//! What follows a final settlement's transitions, after the commit (wl-hr §4
//! *Final settlement*, §5, §9 events, issues #212 to #214). Listeners, not
//! effects: a notice or a ticket that cannot be made does not undo the change.
//!
//! - **`pay`:** the period to dispute starts (`closes_on`, N days after the
//!   payment day, [`crate::HrConfig::settlement_close_days`]); a receivable
//!   the payment made gets a recovery task for the accountant, once
//!   ([`receivable_key`]); `SettlementPaid` follows.
//! - **`dispute`:** a labour-case ticket opens on the employee (category
//!   `labour_case` of wl-ticketing) with the source, the reference and the
//!   hearing or reply dates; HR, the accountant and the MD are told;
//!   `SettlementDisputed` follows. Without ticketing no ticket is made.
//! - **`resolve`, `resolve_closed`:** the labour-case ticket is closed.

use renox::auth::permissions::users_with_role;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{self, OpenTicket, TicketId};
use wl_kernel::{EmployeeId, SettlementDisputed, SettlementId, SettlementPaid};

use crate::config::config_of;
use crate::employee::Employee;
use crate::notices;
use crate::receivable::Receivable;
use crate::settlement::{DISPUTE, FinalSettlement, PAY, RESOLVE, RESOLVE_CLOSED};
use crate::settlement_close::closes_on;
use crate::settlement_payment::paid_total;
use crate::texts::english;

/// The category of the ticket of a dispute (seeded by wl-ticketing when `hr`
/// is on).
pub const LABOUR_CASE_CATEGORY: &str = "labour_case";

/// The source key of the recovery task for receivable `id`.
pub fn receivable_key(id: i64) -> String {
    format!("hr.settlement_receivable:{id}")
}

/// The source key of the labour-case ticket of settlement `id`.
pub fn dispute_key(id: i64) -> String {
    format!("hr.settlement_dispute:{id}")
}

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<FinalSettlement>, state: AppState| async move {
            on_transitioned(&state, &event).await
        },
    );
}

async fn on_transitioned(state: &AppState, event: &Transitioned<FinalSettlement>) -> Result {
    match event.transition.as_str() {
        PAY => paid(state, event.key).await,
        DISPUTE => disputed(state, event.key).await,
        RESOLVE | RESOLVE_CLOSED => resolved(state, event.key).await,
        _ => Ok(()),
    }
}

/// The users holding any of `roles`, once each.
pub(crate) async fn users_of(state: &AppState, roles: &[&str]) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    for role in roles {
        ids.extend(users_with_role(&state.db, role).await?.iter().map(|u| u.id));
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// Opens the recovery task for the accountant for each receivable the
/// settlement caused, once each.
async fn receivable_tasks(state: &AppState, settlement: &FinalSettlement) -> Result {
    let config = config_of(state);
    let id = settlement.id;
    let receivables = Receivable::query()
        .where_(Receivable::SETTLEMENT_ID.eq(id))
        .order_by(Receivable::ID)
        .get(&state.db)
        .await?;
    for receivable in receivables {
        let title = english(
            "wl_hr.settlement.recovery_task_title",
            &[
                ("amount", &crate::pages::payroll::aed(receivable.amount)),
                ("id", &id),
            ],
        );
        let description = english("wl_hr.settlement.recovery_task_description", &[]);
        let task = RoleTask::new(
            settlement.subject(),
            &config.accountant_role,
            &title,
            &receivable_key(receivable.id),
        )
        .description(&description);
        role_tasks::open(state, task).await?;
    }
    Ok(())
}

async fn paid(state: &AppState, id: i64) -> Result {
    let Some(settlement) = FinalSettlement::find(&state.db, id).await? else {
        return Ok(());
    };
    let config = config_of(state);
    if let Some(paid_on) = settlement.paid_on {
        FinalSettlement::query()
            .where_(FinalSettlement::ID.eq(id))
            .update(
                &state.db,
                &[(
                    "closes_on",
                    &Some(closes_on(paid_on, config.settlement_close_days)),
                )],
            )
            .await?;
    }
    receivable_tasks(state, &settlement).await?;
    let amount = paid_total(&state.db, id).await?;
    state
        .emit(SettlementPaid {
            settlement: SettlementId(id),
            employee: EmployeeId(settlement.employee_id),
            amount,
        })
        .await?;
    Ok(())
}

async fn disputed(state: &AppState, id: i64) -> Result {
    let Some(settlement) = FinalSettlement::find(&state.db, id).await? else {
        return Ok(());
    };
    let Some(employee) = Employee::find(&state.db, settlement.employee_id).await? else {
        return Ok(());
    };
    let name = notices::name_of(state, &employee).await;
    let source = settlement.dispute_source.clone().unwrap_or_default();
    let reference = settlement.dispute_reference.clone().unwrap_or_default();
    let title = english(
        "wl_hr.settlement.dispute_ticket_title",
        &[("employee", &name), ("reference", &reference)],
    );
    let dates = [
        settlement
            .hearing_on
            .map(|d| english("wl_hr.settlement.dispute_hearing", &[("date", &d)])),
        settlement
            .reply_due_on
            .map(|d| english("wl_hr.settlement.dispute_reply", &[("date", &d)])),
    ];
    let description = english(
        "wl_hr.settlement.dispute_ticket_description",
        &[
            ("source", &source),
            ("reference", &reference),
            (
                "dates",
                &dates.into_iter().flatten().collect::<Vec<_>>().join(" "),
            ),
        ],
    );
    let ticket = OpenTicket::new(employee.subject(), LABOUR_CASE_CATEGORY, &title)
        .description(&description)
        .source(settlement.subject())
        .source_key(&dispute_key(id));
    if let Some(opened) = tickets::task_creator(state).open(state, ticket).await? {
        FinalSettlement::query()
            .where_(FinalSettlement::ID.eq(id))
            .update(
                &state.db,
                &[
                    ("dispute_ticket", &Some(opened.id.0)),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    let config = config_of(state);
    let users = users_of(
        state,
        &[&config.hr_role, &config.accountant_role, &config.md_role],
    )
    .await?;
    notices::settlement_notice(
        state,
        notices::SETTLEMENT_DISPUTED,
        &settlement,
        &name,
        settlement.deadline(config.settlement_deadline_days),
        &users,
    )
    .await?;
    state
        .emit(SettlementDisputed {
            settlement: SettlementId(id),
            employee: EmployeeId(settlement.employee_id),
            source,
            reference,
        })
        .await?;
    Ok(())
}

async fn resolved(state: &AppState, id: i64) -> Result {
    let Some(settlement) = FinalSettlement::find(&state.db, id).await? else {
        return Ok(());
    };
    // A revised net below what was paid made a receivable.
    receivable_tasks(state, &settlement).await?;
    let Some(ticket) = settlement.dispute_ticket else {
        return Ok(());
    };
    let note = english(
        "wl_hr.settlement.dispute_ticket_resolved",
        &[(
            "outcome",
            &settlement.dispute_outcome.clone().unwrap_or_default(),
        )],
    );
    tickets::task_creator(state)
        .complete(state, TicketId(ticket), &note)
        .await?;
    Ok(())
}
