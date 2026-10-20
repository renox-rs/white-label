//! The `FutureWork` providers the Property client registers for the Civil
//! Defence approval (01 Property §7.2 *Document guard hooks*, row 4 of the
//! BL-PRP-27 settings; `DOC.RUL.02`).
//!
//! `BlockSubjects` for the same two transitions (a ticket or a service
//! contract maps to its vendor) are wl-ticketing's own (`vendor_guard`); what
//! wl-ticketing cannot know is which planned work a vendor still has when the
//! approval is about to expire. These providers list it, for the two guarded
//! transitions:
//!
//! | Guarded | Planned records after the expiry date |
//! |---|---|
//! | `ticket.resolve` | the vendor's open tickets and planned visits whose planned due date (else visit date) is on or after the date, in a category the entry guards |
//! | `service_contract.activate` | the vendor's `draft` contracts starting on or after the date that cover a category the entry guards |
//!
//! A record that does not match the entry's condition is left out, as the
//! `BlockSubjects` answer is empty for it. The coordinator's list of "the
//! vendor's visits after the expiry" on the day the look-ahead window opens
//! is built by wl-documents from these.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::{
    BlockEntry, DocumentFuture, FutureWork, PlannedRecord, dubai_date, register_future_work,
};
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_ticketing::contract::ContractStatus;
use wl_ticketing::{Category, ServiceContract, Ticket};

/// The vendor's open visits and tickets planned from the date.
struct PlannedVisits;

impl FutureWork for PlannedVisits {
    fn planned_from<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        subject: &'a Subject,
        from: NaiveDate,
    ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
        Box::pin(async move {
            if subject.subject_type != PARTY_SUBJECT_TYPE {
                return Ok(Vec::new());
            }
            let tickets = Ticket::query()
                .where_(Ticket::ASSIGNEE_PARTY_ID.eq(subject.subject_id))
                .order_by(Ticket::ID)
                .get(&state.db)
                .await?;
            let mut found = Vec::new();
            for ticket in tickets {
                if !ticket.is_open() {
                    continue;
                }
                let Some(date) = ticket
                    .planned_due_date
                    .or_else(|| ticket.visit_at.map(dubai_date))
                else {
                    continue;
                };
                if date < from {
                    continue;
                }
                let category = Category::find(&state.db, ticket.category_id).await?;
                if !entry.applies_to(category.as_ref().map(|c| c.key.as_str())) {
                    continue;
                }
                let scope = wl_kernel::scopes::scope_of(&state.db, &ticket.subject()).await?;
                found.push(PlannedRecord {
                    record: Subject::new(wl_kernel::tickets::TICKET_SUBJECT_TYPE, ticket.id),
                    date,
                    label: format!("{} {}", ticket.number, ticket.title),
                    scope: Some(scope),
                });
            }
            Ok(found)
        })
    }
}

/// The vendor's draft service contracts starting from the date.
struct PlannedContracts;

impl FutureWork for PlannedContracts {
    fn planned_from<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        subject: &'a Subject,
        from: NaiveDate,
    ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
        Box::pin(async move {
            if subject.subject_type != PARTY_SUBJECT_TYPE {
                return Ok(Vec::new());
            }
            let contracts = ServiceContract::query()
                .where_(ServiceContract::VENDOR_PARTY_ID.eq(subject.subject_id))
                .order_by(ServiceContract::ID)
                .get(&state.db)
                .await?;
            let mut found = Vec::new();
            for contract in contracts {
                if contract.status != ContractStatus::Draft || contract.start_date < from {
                    continue;
                }
                let mut guarded = entry.condition.is_none();
                for row in contract.coverage(&state.db).await? {
                    if guarded {
                        break;
                    }
                    let category = Category::find(&state.db, row.category_id).await?;
                    guarded = entry.applies_to(category.as_ref().map(|c| c.key.as_str()));
                }
                if !guarded {
                    continue;
                }
                found.push(PlannedRecord {
                    record: contract.subject(),
                    date: contract.start_date,
                    label: format!("{} {}", contract.number, contract.title),
                    scope: None,
                });
            }
            Ok(found)
        })
    }
}

/// Registers the two providers (process-wide, like the module registries).
pub fn register() {
    register_future_work(wl_ticketing::vendor_guard::RESOLVE, PlannedVisits);
    register_future_work(wl_ticketing::vendor_guard::ACTIVATE, PlannedContracts);
}
