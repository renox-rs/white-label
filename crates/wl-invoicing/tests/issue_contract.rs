//! `Invoicer::issue`: another module issues the draft it made through the
//! invoice's own `issue` transition. A user needs `invoices.issue`; the system
//! issues only a source the module is set to issue automatically.

mod support;

use wl_invoicing::{Invoice, WlInvoicing};
use wl_kernel::Subject;
use wl_kernel::invoicing::{
    DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus, VatCategory, invoicer,
};

use support::{BLUE_DUNES, boot, boot_with};

fn draft_of(source: Subject) -> InvoiceDraft {
    InvoiceDraft::new(BLUE_DUNES, source).line(DraftLine::new(
        "Call-out",
        20_000,
        VatCategory::Standard,
    ))
}

/// A user with `invoices.issue` issues through the contract: the number, the
/// dates and the event are those of the invoice's own `issue`.
#[renox::test]
async fn a_user_with_the_permission_issues_a_draft_through_the_contract() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let draft = t
        .draft_of(draft_of(Subject::new("ticketing.ticket", 5)))
        .await;
    assert_eq!(t.contract_issue(&draft, &clerk).await, "ok");
    let issued = t.reload(&draft).await;
    assert_eq!(issued.status, InvoiceStatus::Issued);
    assert!(issued.number.is_some());
    assert_eq!(issued.owner_id, Some(clerk.id));
    let events = t.issued.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].invoice.0, draft.id);
    assert_eq!(events[0].total, 21_000);
}

/// Without the permission nothing is issued, and an issued invoice is not
/// issued twice.
#[renox::test]
async fn the_permission_and_the_state_are_still_checked() {
    let t = boot().await;
    let viewer = t.viewer().await;
    let clerk = t.user_a().await;
    let draft = t
        .draft_of(draft_of(Subject::new("ticketing.ticket", 5)))
        .await;
    assert_eq!(t.contract_issue(&draft, &viewer).await, "403");
    assert_eq!(t.reload(&draft).await.status, InvoiceStatus::Draft);
    assert_eq!(t.contract_issue(&draft, &clerk).await, "ok");
    assert_eq!(t.contract_issue(&draft, &clerk).await, "422");
    assert_eq!(t.issued.all().len(), 1);
}

/// The system issues nothing by default; a source set to auto-issue is
/// issued at once, and another source still is not.
#[renox::test]
async fn the_system_issues_only_a_source_the_module_issues_automatically() {
    let t = boot().await;
    let draft = t
        .draft_of(draft_of(Subject::new("ticketing.ticket", 5)))
        .await;
    assert!(
        invoicer(t.state())
            .issue(t.state(), InvoiceId(draft.id), None)
            .await
            .is_err()
    );
    assert_eq!(t.reload(&draft).await.status, InvoiceStatus::Draft);

    let t = boot_with(WlInvoicing::new().auto_issue_source("ticketing.ticket")).await;
    let ticket = t
        .draft_of(draft_of(Subject::new("ticketing.ticket", 5)))
        .await;
    let lease = t
        .draft_of(draft_of(Subject::new("property.lease", 9)))
        .await;
    let contract = invoicer(t.state());
    contract
        .issue(t.state(), InvoiceId(ticket.id), None)
        .await
        .unwrap();
    let issued: Invoice = t.reload(&ticket).await;
    assert_eq!(issued.status, InvoiceStatus::Issued);
    assert!(issued.number.is_some());
    assert_eq!(issued.owner_id, None);
    assert_eq!(t.issued.all().len(), 1);
    assert!(
        contract
            .issue(t.state(), InvoiceId(lease.id), None)
            .await
            .is_err()
    );
    assert_eq!(t.reload(&lease).await.status, InvoiceStatus::Draft);
}
