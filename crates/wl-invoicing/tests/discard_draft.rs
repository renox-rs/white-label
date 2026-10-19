//! `Invoicer::discard_draft` (#251): a draft is deleted with its lines; an
//! issued invoice never is.

mod support;

use renox::prelude::*;
use wl_invoicing::{Invoice, InvoiceLine};
use wl_kernel::invoicing::{InvoiceId, VatCategory, invoicer};

use support::boot;

/// A draft goes, with its lines, and a second discard finds nothing.
#[renox::test]
async fn a_draft_is_deleted_with_its_lines() {
    let t = boot().await;
    let draft = t
        .draft(
            &[
                (100_000, VatCategory::Standard),
                (50_000, VatCategory::ZeroRated),
            ],
            None,
        )
        .await;
    let contract = invoicer(t.state());
    assert!(
        contract
            .discard_draft(t.db(), InvoiceId(draft.id))
            .await
            .unwrap()
    );
    assert!(Invoice::find(t.db(), draft.id).await.unwrap().is_none());
    let lines = InvoiceLine::query()
        .where_(InvoiceLine::INVOICE_ID.eq(draft.id))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(lines, 0);
    assert!(
        !contract
            .discard_draft(t.db(), InvoiceId(draft.id))
            .await
            .unwrap()
    );
}

/// An issued invoice stays: it is credited, never deleted.
#[renox::test]
async fn an_issued_invoice_is_not_discarded() {
    let t = boot().await;
    let accountant = t.accountant("Aisha").await;
    let draft = t.draft(&[(100_000, VatCategory::Standard)], None).await;
    let issued = t.issue(&draft, &accountant).await;
    assert!(
        !invoicer(t.state())
            .discard_draft(t.db(), InvoiceId(issued.id))
            .await
            .unwrap()
    );
    assert!(Invoice::find(t.db(), issued.id).await.unwrap().is_some());
}
