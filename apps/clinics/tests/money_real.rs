//! The deposit of a visit settled against the **real** wl-invoicing and
//! wl-payments (BL-CLN-05, #353 and BL-CLN-06, #354): the patient's invoice is
//! the module's own, the deposit is a received payment held for the booking
//! and allocated to that invoice, and what the bill cannot use is a refund
//! row of the payments module, `requested`, with the booking as its source.
//! The booking, the visit, the routes and the bill are the real ones.
//!
//! The approval and the execution of that refund (`FIN.ACL.04`) are the
//! payments module's own and are not repeated here.

mod support;

use clinics::VisitStatus;
use renox::prelude::*;
use support::day;
use support::desk::desk_with_money;
use wl_kernel::invoicing::{InvoiceStatus, invoicer};
use wl_kernel::payments::payment_recorder;
use wl_kernel::scheduling::BookingId;
use wl_payments::Refund;
use wl_payments::refund::RefundStatus;
use wl_scheduling::BookingStatus;

/// #353 points 1 and 2: a filler syringe (AED 150.00) issued, the patient
/// walks out with a deposit of AED 200.00: the visit's invoice is AED 150.00
/// and paid by the deposit, and AED 50.00 is asked back from the real
/// payments module.
#[renox::test]
async fn a_walk_out_settles_the_deposit_against_the_real_invoice_and_refunds_the_rest() {
    let d = desk_with_money().await;
    d.deposit(20_000).await;
    d.t.prices.set("filler_syringe", day(-400), 15_000);
    let booking = d.booking(&d.consultation, 9).await;
    let visit = d.check_in(&booking).await;
    d.issue(&visit, "filler_syringe", 1).await.assert_ok();

    d.cancel(&booking, "walked_out").await.assert_status(303);

    let done = d.t.booking(booking.id).await;
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert_eq!(done.deposit_applied, 15_000);
    assert_eq!(done.deposit_refunded, 5_000);
    let visit = d.visit(visit.id).await;
    assert_eq!(visit.status, VisitStatus::Cancelled);
    assert_eq!((visit.patient_total, visit.payer_total), (15_000, 0));

    // The real invoice: issued, AED 150.00, nothing left to pay.
    let invoice = invoicer(d.t.state())
        .find(d.t.db(), wl_kernel::invoicing::InvoiceId(visit.invoice_id.unwrap()))
        .await
        .unwrap()
        .expect("the invoice exists");
    assert_eq!(invoice.gross_total, 15_000);
    assert_ne!(invoice.status, InvoiceStatus::Draft, "issued at once");
    assert_eq!(invoice.balance, 0, "the deposit settled it");

    // The real deposit: AED 150.00 allocated, the rest asked back.
    let held = payment_recorder(d.t.state())
        .payments_for(d.t.db(), &BookingId(booking.id).subject())
        .await
        .unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!((held[0].allocated, held[0].amount), (15_000, 20_000));
    let refunds = Refund::all(d.t.db()).await.unwrap();
    assert_eq!(refunds.len(), 1, "one refund, for what the bill did not use");
    assert_eq!(refunds[0].status, RefundStatus::Requested);
    assert_eq!(refunds[0].source_kind, "booking");
    assert_eq!(refunds[0].source_id, booking.id);
    assert_eq!(refunds[0].amount, 5_000);
}

/// #354 point 1: the planned laser (deposit AED 300.00) is not suitable, the
/// consultation (AED 250.00) is billed: the real invoice is paid by the
/// deposit and AED 50.00 is asked back from the real payments module.
#[renox::test]
async fn a_procedure_that_is_not_suitable_bills_the_consultation_against_the_real_deposit() {
    let d = desk_with_money().await;
    let laser = d.service("laser", 30_000).await;
    d.t.prices.set("laser", day(-400), 120_000);
    d.t.prices.set("consultation", day(-300), 25_000);
    let booking = d.booking(&laser, 9).await;
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    d.deliver(&visit, "laser", 1).await.assert_ok();
    d.post(
        &d.dana,
        &format!("/clinics/visits/{}/not-suitable", visit.id),
        &[("consultation", "consultation")],
    )
    .await
    .assert_ok();

    d.complete(&booking).await;

    let visit = d.visit(visit.id).await;
    assert_eq!(visit.status, VisitStatus::Completed);
    assert_eq!(visit.patient_total, 25_000);
    let invoice = invoicer(d.t.state())
        .find(d.t.db(), wl_kernel::invoicing::InvoiceId(visit.invoice_id.unwrap()))
        .await
        .unwrap()
        .expect("the invoice exists");
    assert_eq!(invoice.gross_total, 25_000);
    assert_eq!(invoice.balance, 0, "the deposit covers it");
    let refunds = Refund::all(d.t.db()).await.unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, RefundStatus::Requested);
    assert_eq!(refunds[0].source_kind, "booking");
    assert_eq!(refunds[0].amount, 5_000);
    assert_eq!(d.t.booking(booking.id).await.deposit_forfeited, 0);
}
