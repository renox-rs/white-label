//! The Ticket (wl-ticketing §3, §4.1): a piece of work on any record (its
//! subject), with who asked for it, who works on it, and the SLA clock. Its
//! status moves only through the machine in [`crate::machine`].

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::tickets::{Assignee, Requester, TicketChannel, TicketId, TicketRef};
use wl_kernel::{PartyId, Subject};

use crate::priority::Priority;

/// Where a ticket is in its life (wl-ticketing §4.1). Stored as `open`,
/// `assigned`, `in_progress`, `on_hold`, `resolved`, `closed` or
/// `cancelled`.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TicketStatus {
    /// Raised, not yet triaged.
    #[default]
    Open,
    /// Has an assignee, a priority and an SLA.
    Assigned,
    /// Work started.
    InProgress,
    /// Paused for a configured reason.
    OnHold,
    /// Work done; waiting for the requester's sign-off when the category
    /// requires it.
    Resolved,
    /// Confirmed by the requester, auto-closed, or closed by staff.
    Closed,
    /// Not needed, duplicate or invalid.
    Cancelled,
}

/// A ticket. Dates are instants; show them in Asia/Dubai.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_ticketing_tickets",
    state_machine,
    unique(number),
    index(status),
    index(subject_type, subject_id),
    index(assignee_user_id),
    index(source_key),
    index(contract_id)
)]
pub struct Ticket {
    /// The primary key.
    pub id: i64,
    /// The number from the `ticket` series, e.g. `TKT-2026-0001`.
    pub number: String,
    /// The type of the record it is about, e.g. `property.unit`.
    pub subject_type: String,
    /// The id of the record it is about.
    pub subject_id: i64,
    /// The type of the record that opened it, if not the subject.
    pub source_type: Option<String>,
    /// The id of the record that opened it.
    pub source_id: Option<i64>,
    /// The category.
    #[model(references = "wl_ticketing_categories")]
    pub category_id: i64,
    /// How urgent it is; defaults from the category.
    pub priority: Priority,
    /// The user who asked for it, if a user did.
    pub requester_user_id: Option<i64>,
    /// The party who asked for it, if a party did.
    pub requester_party_id: Option<i64>,
    /// How it came in.
    pub channel: TicketChannel,
    /// The title.
    pub title: String,
    /// The description; empty when none.
    pub description: String,
    /// The staff user who works on it, if one does.
    pub assignee_user_id: Option<i64>,
    /// The vendor party that works on it, if one does.
    pub assignee_party_id: Option<i64>,
    /// When the visit is scheduled, if it is.
    pub visit_at: Option<DateTime>,
    /// The SLA policy, copied from the category when opened.
    #[model(references = "wl_ticketing_sla_policies")]
    pub sla_policy_id: Option<i64>,
    /// When the SLA started: the first assignment. Never restarted.
    pub sla_started_at: Option<DateTime>,
    /// When the first response is due.
    pub response_due: Option<DateTime>,
    /// When the resolution is due; moved later by the time on hold.
    pub resolution_due: Option<DateTime>,
    /// When work started (`start`).
    pub responded_at: Option<DateTime>,
    /// When the SLA clock was paused; `None` while it runs.
    pub sla_paused_at: Option<DateTime>,
    /// The working minutes the resolution clock stood still so far.
    pub sla_paused_minutes: i64,
    /// Why it is on hold, while it is.
    #[model(references = "wl_ticketing_hold_reasons")]
    pub hold_reason_id: Option<i64>,
    /// When it was resolved; cleared by `reopen`.
    pub resolved_at: Option<DateTime>,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// Whether only `tickets.confidential.view` holders see it (copied from
    /// the category).
    pub confidential: bool,
    /// Whether it can carry charges (copied from the category).
    pub chargeable: bool,
    /// The draft invoice made for it, once there is one: one per ticket.
    pub invoice_id: Option<i64>,
    /// The party a chargeable ticket is invoiced to when its requester is a
    /// user; none bills the requester's party.
    pub bill_party_id: Option<i64>,
    /// A planned visit's planned due date: the due date of the maintenance
    /// plan occurrence that opened it. Not the visit scheduled at, and not an
    /// SLA due time: moving the appointment does not move it.
    pub planned_due_date: Option<NaiveDate>,
    /// The reference of the vendor's service report attached to a planned
    /// visit (`PRP.ACL.15`).
    pub service_report: Option<String>,
    /// When `WRK.RUL.14` told the coordinator the visit was missed.
    pub missed_notified_at: Option<DateTime>,
    /// The service contract it is handled under: set for a planned visit of
    /// the contract's plan, and for a ticket opened on a covered subject.
    pub contract_id: Option<i64>,
    /// The vendor document flag of a planned visit: the vendor's document the
    /// category needs is expired or missing; cleared by a renewed document or
    /// by reassigning the visit ([`crate::vendor_flags`]).
    pub vendor_document_flag: bool,
    /// The lifecycle state; changed only by a transition.
    pub status: TicketStatus,
    /// When it is due, if its opener gave it a due time of its own (a staff
    /// task); apart from the SLA due times.
    pub due_at: Option<DateTime>,
    /// The key its opener finds it by while it is open, e.g.
    /// `notify.give_up:…`.
    pub source_key: Option<String>,
    /// The role that owns the ticket until a holder takes it (BL-TKT-12),
    /// e.g. `receivables`; `None` for a ticket of a user, a vendor or nobody.
    pub assignee_role: Option<String>,
    /// The role's name for people, e.g. `Receivables`.
    pub assignee_role_label: Option<String>,
    /// The permission that defines the role's holders, when it is one.
    pub assignee_role_permission: Option<String>,
    /// The role's scope: its kind (empty for global).
    pub assignee_scope_type: Option<String>,
    /// The role's scope: its key.
    pub assignee_scope_id: Option<String>,
    /// The role queue the ticket is in: `role|scope kind|scope id`.
    pub assignee_role_key: Option<String>,
    /// The role and scope for people: `Receivables (ACME Properties LLC)`.
    pub assignee_role_display: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Ticket {
    /// Whether the ticket is still open for work: any state except
    /// `resolved`, `closed` and `cancelled`.
    pub fn is_open(&self) -> bool {
        !matches!(
            self.status,
            TicketStatus::Resolved | TicketStatus::Closed | TicketStatus::Cancelled
        )
    }

    /// The id as the kernel's [`TicketId`].
    pub fn ticket_id(&self) -> TicketId {
        TicketId(self.id)
    }

    /// The id and number, as other modules see it.
    pub fn reference(&self) -> TicketRef {
        TicketRef {
            id: self.ticket_id(),
            number: self.number.clone(),
        }
    }

    /// The record it is about.
    pub fn subject(&self) -> Subject {
        Subject {
            subject_type: self.subject_type.clone(),
            subject_id: self.subject_id,
        }
    }

    /// Who works on it: a staff user or a vendor party, if anyone.
    pub fn assignee(&self) -> Option<Assignee> {
        match (self.assignee_user_id, self.assignee_party_id) {
            (Some(id), _) => Some(Assignee::User { id }),
            (None, Some(id)) => Some(Assignee::Party { id: PartyId(id) }),
            (None, None) => None,
        }
    }

    /// The role that owns it until a holder takes it, if a role does.
    pub fn role_assignee(&self) -> Option<wl_kernel::ticket_routing::RoleAssignee> {
        let role = self.assignee_role.clone()?;
        Some(wl_kernel::ticket_routing::RoleAssignee {
            label: self
                .assignee_role_label
                .clone()
                .unwrap_or_else(|| wl_kernel::ticket_routing::role_label(&role)),
            role,
            permission: self.assignee_role_permission.clone(),
            scope_kind: self.assignee_scope_type.clone().unwrap_or_default(),
            scope_id: self.assignee_scope_id.clone().unwrap_or_default(),
        })
    }

    /// The party an invoice for the ticket is addressed to: the one named
    /// when it was opened, else the requester's party.
    pub fn bill_party(&self) -> Option<PartyId> {
        self.bill_party_id.or(self.requester_party_id).map(PartyId)
    }

    /// Who asked for it, if anyone.
    pub fn requester(&self) -> Option<Requester> {
        match (self.requester_user_id, self.requester_party_id) {
            (Some(id), _) => Some(Requester::User { id }),
            (None, Some(id)) => Some(Requester::Party { id: PartyId(id) }),
            (None, None) => None,
        }
    }
}

/// Registers how to find the scope of a ticket (#56 A2): its subject's
/// scope, or global when the ticket is unknown.
pub(crate) fn register_scope() {
    wl_kernel::scopes::register(wl_kernel::tickets::TICKET_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            match Ticket::find(db, id).await? {
                Some(ticket) => wl_kernel::scopes::scope_of(db, &ticket.subject()).await,
                None => Ok(renox::auth::permissions::Scope::global()),
            }
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let keys: Vec<_> = TicketStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            keys,
            [
                "open",
                "assigned",
                "in_progress",
                "on_hold",
                "resolved",
                "closed",
                "cancelled"
            ]
        );
        assert_eq!(TicketStatus::default(), TicketStatus::Open);
        assert_eq!(TicketStatus::OnHold.to_string(), "on_hold");
    }

    #[test]
    fn the_assignee_and_requester_are_a_user_or_a_party() {
        let mut ticket = Ticket::default();
        assert_eq!(ticket.assignee(), None);
        assert_eq!(ticket.requester(), None);
        ticket.assignee_user_id = Some(7);
        ticket.requester_party_id = Some(3);
        assert_eq!(ticket.assignee(), Some(Assignee::User { id: 7 }));
        assert_eq!(
            ticket.requester(),
            Some(Requester::Party { id: PartyId(3) })
        );
        ticket.assignee_user_id = None;
        ticket.assignee_party_id = Some(9);
        assert_eq!(ticket.assignee(), Some(Assignee::Party { id: PartyId(9) }));
    }

    #[test]
    fn the_bill_party_is_the_named_one_else_the_requesters_party() {
        let mut ticket = Ticket::default();
        assert_eq!(ticket.bill_party(), None);
        ticket.requester_user_id = Some(4);
        assert_eq!(ticket.bill_party(), None);
        ticket.requester_user_id = None;
        ticket.requester_party_id = Some(3);
        assert_eq!(ticket.bill_party(), Some(PartyId(3)));
        ticket.requester_user_id = Some(4);
        ticket.bill_party_id = Some(9);
        assert_eq!(ticket.bill_party(), Some(PartyId(9)));
    }

    #[test]
    fn a_ticket_is_a_subject_and_a_reference() {
        let ticket = Ticket {
            id: 42,
            number: "TKT-2026-0042".into(),
            subject_type: "property.unit".into(),
            subject_id: 12,
            ..Ticket::default()
        };
        assert_eq!(ticket.subject(), Subject::new("property.unit", 12));
        assert_eq!(ticket.reference().id, TicketId(42));
        assert_eq!(ticket.reference().number, "TKT-2026-0042");
    }
}
