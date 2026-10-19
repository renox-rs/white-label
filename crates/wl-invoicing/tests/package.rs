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
