//! BL-AUT-14 against the REAL wl-invoicing, wl-notify and wl-ticketing of the
//! host: a zero-rated invoice of AED 25,000 registered as an export sale; the
//! accountant is told through the real notifier at 60 days, at 90 days the sale
//! is flagged and the accountant gets a real task about the invoice; no credit
//! note or new invoice is issued; evidence stops the rule. The app's own tests
//! (`apps/auto/tests`) show the same, and the failed run, against stand-ins.

mod support;

use auto::ExportSale;
use auto::export_rule::{self, NOTICE_TYPE, task_key};
use auto::export_sale::{AIRWAY_BILL, BILL_OF_LADING};
use auto::export_sales;
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::autofx::{boot, sent_of, shop};
use support::tenant;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus, VatCategory, invoicer,
};
use wl_kernel::{PartyId, Subject};

/// The tasks of the ticketing module whose source key starts with `prefix`.
async fn tasks(w: &support::autofx::World, prefix: &str) -> Vec<wl_ticketing::Ticket> {
    wl_ticketing::Ticket::query()
        .get(w.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|t| {
            t.source_key
                .as_deref()
                .is_some_and(|k| k.starts_with(prefix))
        })
        .collect()
}

/// An issued invoice of AED 25,000, zero-rated, for `customer`.
async fn export_invoice(
    w: &support::autofx::World,
    accountant: &AuthUser,
    customer: PartyId,
    reference: i64,
) -> InvoiceId {
    let books = invoicer(w.state());
    let draft =
        InvoiceDraft::new(customer, Subject::new("auto.export", reference)).line(DraftLine::new(
            "Brake pads, export to Mombasa",
            2_500_000,
            VatCategory::ZeroRated,
        ));
    let id = books
        .draft(w.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    books
        .issue(w.state(), id, Some(accountant))
        .await
        .expect("the accountant issues it");
    id
}

/// INV zero-rated AED 25,000 supplied today with no evidence: told at 60 days
/// (the real message to the accountant), flagged at 90 with a real task that
/// links the invoice, once each; nothing is credited or issued; a sale with a
/// bill of lading is left alone.
#[renox::test]
async fn aut_14_real_the_accountant_is_told_and_gets_the_correction_task() {
    let w = boot().await;
    let s = shop(&w, None).await;
    let customer = tenant(&w.app, "Gulf Movers Trading").await;
    let slow = export_invoice(&w, &s.aisha, customer, 140).await;
    let quick = export_invoice(&w, &s.aisha, customer, 141).await;
    let a = export_sales::record(w.state(), &s.aisha, slow.0, today())
        .await
        .unwrap();
    let b = export_sales::record(w.state(), &s.aisha, quick.0, today())
        .await
        .unwrap();
    assert_eq!(a.gross_total, 2_500_000, "zero-rated: no VAT");
    export_sales::attach_evidence(w.state(), &s.aisha, b.id, BILL_OF_LADING, "BL-7788")
        .await
        .unwrap();

    export_rule::run_daily(w.state(), today() + TimeDelta::days(59))
        .await
        .unwrap();
    assert!(sent_of(&w, NOTICE_TYPE).await.is_empty());
    export_rule::run_daily(w.state(), today() + TimeDelta::days(60))
        .await
        .unwrap();
    let mails = sent_of(&w, NOTICE_TYPE).await;
    assert_eq!(mails.len(), 1, "only the sale without evidence");
    assert_eq!(mails[0].user_id, Some(s.aisha.id));
    assert!(tasks(&w, "auto.export_vat").await.is_empty());

    export_rule::run_daily(w.state(), today() + TimeDelta::days(90))
        .await
        .unwrap();
    let flagged = ExportSale::find_or_404(w.db(), a.id).await.unwrap();
    assert!(flagged.vat_flagged_at.is_some());
    let waiting = tasks(&w, &task_key(a.id)).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].subject_type, "invoicing.invoice");
    assert_eq!(waiting[0].subject_id, slow.0, "it links the invoice");
    assert_eq!(waiting[0].assignee_user_id, Some(s.aisha.id));
    assert!(tasks(&w, &task_key(b.id)).await.is_empty());
    export_rule::run_daily(w.state(), today() + TimeDelta::days(91))
        .await
        .unwrap();
    assert_eq!(tasks(&w, "auto.export_vat").await.len(), 1);

    // No document was issued by the rule.
    assert!(
        invoicer(w.state())
            .credit_notes_for(w.db(), slow)
            .await
            .unwrap()
            .is_empty()
    );
    let invoice = invoicer(w.state())
        .find(w.db(), slow)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(invoice.status, InvoiceStatus::Credited);
    assert_eq!(invoice.gross_total, 2_500_000);

    // Evidence after the flag: the flag stays, the accountant is told.
    export_sales::attach_evidence(w.state(), &s.aisha, a.id, AIRWAY_BILL, "AW-12")
        .await
        .unwrap();
    let kept = ExportSale::find_or_404(w.db(), a.id).await.unwrap();
    assert!(kept.vat_flagged_at.is_some() && kept.late_evidence_notified_at.is_some());
    assert_eq!(sent_of(&w, NOTICE_TYPE).await.len(), 2);
}
