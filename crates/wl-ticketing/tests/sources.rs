//! Tickets opened by another record, as wl-invoicing opens a billing
//! dispute (#67 part 7): the `billing_dispute` category, the assignee at
//! opening, the source link, and `resolve_from_source`.

mod support;

use renox::prelude::Model;
use wl_kernel::Subject;
use wl_kernel::tickets::{Assignee, OpenTicket, TicketChannel, task_creator};
use wl_ticketing::{Category, Ticket, TicketStatus, seed};

use support::{COOLAIR, boot};

/// The category exists only once invoicing is on; a ticket opened in it with
/// an assignee, a requester party and a source is `assigned` through the
/// system first assignment, links back to its source, and is resolved by the
/// source once with its note; a second resolve does nothing.
#[renox::test]
async fn a_billing_dispute_ticket_is_opened_assigned_and_resolved_by_its_source() {
    let t = boot().await;
    let creator = task_creator(t.state());
    let invoice = Subject::new("invoicing.invoice", 7);
    let dispute = Subject::new("invoicing.dispute", 3);

    // Without invoicing the category is unknown and nothing is opened.
    assert!(
        Category::by_key(t.db(), seed::BILLING_DISPUTE_CATEGORY)
            .await
            .unwrap()
            .is_none()
    );
    let refused = creator
        .open(
            t.state(),
            OpenTicket::new(invoice.clone(), "billing_dispute", "Dispute on INV-1"),
        )
        .await;
    assert!(refused.is_err());

    seed::install_with(t.db(), true).await.unwrap();
    let owner = t.coordinator().await;
    let opened = creator
        .open(
            t.state(),
            OpenTicket::new(invoice.clone(), "billing_dispute", "Dispute on INV-1")
                .description("Reason: wrong hours")
                .requester_party(COOLAIR)
                .channel(TicketChannel::Staff)
                .source(dispute.clone())
                .assign_to(Assignee::user(owner.id)),
        )
        .await
        .unwrap()
        .expect("ticketing is on");

    let ticket = t.ticket(opened.id.0).await;
    assert_eq!(ticket.status, TicketStatus::Assigned);
    assert_eq!(ticket.assignee(), Some(Assignee::user(owner.id)));
    assert_eq!(ticket.subject_type, "invoicing.invoice");
    assert_eq!(ticket.subject_id, 7);
    assert_eq!(ticket.source_type.as_deref(), Some("invoicing.dispute"));
    assert_eq!(ticket.source_id, Some(3));
    assert_eq!(ticket.channel, TicketChannel::Staff);
    assert_eq!(ticket.requester_party_id, Some(COOLAIR.0));
    assert!(ticket.resolution_due.is_some(), "the SLA started");
    assert_eq!(t.history(&ticket).await.len(), 1, "the first assignment");

    // Without an assignee it stays open.
    let unassigned = creator
        .open(
            t.state(),
            OpenTicket::new(invoice.clone(), "billing_dispute", "Dispute on INV-2")
                .requester_party(COOLAIR),
        )
        .await
        .unwrap()
        .expect("ticketing is on");
    let unassigned = t.ticket(unassigned.id.0).await;
    assert_eq!(unassigned.status, TicketStatus::Open);
    assert_eq!(unassigned.assignee(), None);

    // The source resolves it, once.
    assert!(
        creator
            .resolve_from_source(t.state(), opened.id, "Credited AED 1,200.00 (CN-1)")
            .await
            .unwrap()
    );
    let resolved = Ticket::find_or_404(t.db(), opened.id.0).await.unwrap();
    assert_eq!(resolved.status, TicketStatus::Resolved);
    assert!(resolved.resolved_at.is_some());
    assert!(
        !creator
            .resolve_from_source(t.state(), opened.id, "Again")
            .await
            .unwrap()
    );
}
