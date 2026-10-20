//! Booting wl-invoicing (wl-invoicing §6): the acceptance checks AC1 and AC4
//! of #64. The host side of AC1 is `host/tests/client.rs`; the module-off
//! side is `crates/wl-kernel/tests/package.rs`.

mod support;

use renox::prelude::*;
use wl_invoicing::{Invoice, InvoiceKind};
use wl_kernel::invoicing::{InvoiceStatus, VatCategory};

use support::boot;

/// AC1: `/invoices` answers 200 with `invoices.view` and 403 without it.
#[renox::test]
async fn the_invoice_list_needs_the_view_permission() {
    let t = boot().await;
    let a = t.user_a().await;
    let viewer = t.viewer().await;
    let nobody = t.user("Nora", &[]).await;
    let invoice = t
        .issued(&[(100_000, VatCategory::Standard)], None, &a)
        .await;
    let number = invoice.number.clone().unwrap();

    for user in [&viewer, &a] {
        t.app.acting_as(user);
        t.app.get("/invoices").await.assert_ok().assert_see(&number);
    }
    t.app.acting_as(&nobody);
    t.app.get("/invoices").await.assert_forbidden();
    t.app
        .get(&format!("/invoices/{}", invoice.id))
        .await
        .assert_forbidden();
    // Viewing is not enough to issue or to write off.
    let draft = t.draft(&[(100_000, VatCategory::Standard)], None).await;
    t.post_issue(&draft, &viewer).await.assert_forbidden();
    t.post_write_off(&invoice, &viewer, "insolvent", "")
        .await
        .assert_forbidden();
}

/// AC4: a draft invoice cannot be inserted in another state, and saving a
/// changed status outside a transition is refused.
#[renox::test]
async fn the_status_changes_only_through_a_transition() {
    let t = boot().await;
    for status in [
        InvoiceStatus::Issued,
        InvoiceStatus::Paid,
        InvoiceStatus::WrittenOff,
    ] {
        let mut invoice = Invoice {
            party_id: 1,
            kind: InvoiceKind::TaxInvoice,
            source_type: "property.lease".into(),
            source_id: 1,
            currency: "AED".into(),
            status,
            ..Default::default()
        };
        assert!(invoice.insert(t.db()).await.is_err(), "{status}");
    }
    assert_eq!(Invoice::query().count(t.db()).await.unwrap(), 0);

    let mut draft = t.draft(&[(100_000, VatCategory::Standard)], None).await;
    assert_eq!(draft.status, InvoiceStatus::Draft);
    draft.status = InvoiceStatus::Paid;
    assert!(draft.save(t.db()).await.is_err());
    assert_eq!(t.reload(&draft).await.status, InvoiceStatus::Draft);
}

#[renox::test]
async fn the_models_match_the_schema() {
    boot().await.app.assert_models_match_schema().await;
}

#[renox::test]
async fn every_view_compiles() {
    boot().await.app.assert_views_compile();
}

/// #147: a module that drafts one invoice per record replaces the lines of
/// its draft through the kernel contract; the totals follow, the party and
/// source stay, and an issued invoice is never rewritten.
#[renox::test]
async fn update_draft_replaces_the_lines_of_a_draft_only() {
    use wl_kernel::invoicing::{DraftLine, InvoiceDraft, invoicer};

    let t = boot().await;
    let a = t.user_a().await;
    let draft = t.draft(&[(33_333, VatCategory::Standard)], None).await;
    assert_eq!(draft.gross_total, 35_000);

    let replacement = InvoiceDraft::new(
        wl_kernel::PartyId(999),
        wl_kernel::Subject::new("property.lease", 10),
    )
    .line(DraftLine::new(
        "Replace AC capacitor",
        20_000,
        VatCategory::Standard,
    ))
    .line(DraftLine::new("Call-out", 5_000, VatCategory::Exempt));
    let changed = invoicer(t.state())
        .update_draft(t.db(), draft.invoice_id(), replacement.clone())
        .await
        .unwrap();
    assert!(changed);
    let after = t.reload(&draft).await;
    assert_eq!(
        (after.net_total, after.vat_total, after.gross_total),
        (25_000, 1_000, 26_000)
    );
    assert_eq!(after.balance, 26_000);
    assert_eq!(after.party_id, draft.party_id, "the party stays");
    assert_eq!(after.source_id, draft.source_id);
    let lines = wl_invoicing::invoicer::lines_of(t.db(), draft.id)
        .await
        .unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].description, "Replace AC capacitor");

    let issued = t
        .issued(&[(100_000, VatCategory::Standard)], None, &a)
        .await;
    let untouched = invoicer(t.state())
        .update_draft(t.db(), issued.invoice_id(), replacement)
        .await
        .unwrap();
    assert!(!untouched);
    assert_eq!(t.reload(&issued).await.gross_total, issued.gross_total);
}
