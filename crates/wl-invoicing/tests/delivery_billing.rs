//! BL-INV-22 (#85), `X-11`, 07-N4-a, 03-N3-a, wl-invoicing §5.2 step 4: a
//! draft bills the delivered quantity per line, never the refused one; a
//! cash-on-delivery difference stays open when the customer owes it, is the
//! client's remittance line when the client bears it, and is a negative line
//! on a draft or a credit note on an issued invoice when the business or the
//! driver bears it. The events are emitted as `wl-delivery` emits them (the
//! real module is in `delivery_real.rs`).

mod support;

use renox::prelude::*;
use wl_invoicing::delivery_billing::{CodDifference, CodOutcome};
use wl_invoicing::reminders::{self, AFTER_DUE};
use wl_invoicing::{Invoice, InvoiceLine};
use wl_kernel::delivery::{DeliveredLine, DeliveryDelivered, DeliveryId};
use wl_kernel::invoicing::{
    CreditNoteStatus, DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus, VatCategory, invoicer,
};
use wl_kernel::Subject;

use support::{BLUE_DUNES, Booted, boot};

const DELIVERY: DeliveryId = DeliveryId(42);

fn order() -> Subject {
    Subject::new("sales.order", 9)
}

fn order_line(n: i64) -> Subject {
    Subject::new("sales.order_line", n)
}

/// A draft with one line per `(order line, quantity in thousandths, unit price
/// in fils)`, standard rated.
async fn draft_of(t: &Booted, lines: &[(i64, i64, i64)]) -> Invoice {
    let mut draft = InvoiceDraft::new(BLUE_DUNES, order());
    for (n, quantity, price) in lines {
        draft = draft.line(
            DraftLine::new(&format!("Order line {n}"), *price, VatCategory::Standard)
                .quantity_milli(*quantity)
                .origin(order_line(*n)),
        );
    }
    t.draft_of(draft).await
}

/// What the recipient accepted and refused of one order line.
fn delivered_line(n: i64, ordered: i64, delivered: i64) -> DeliveredLine {
    DeliveredLine {
        line: Some(order_line(n)),
        description: format!("Order line {n}"),
        quantity_milli: ordered,
        delivered_milli: delivered,
        refused_milli: ordered - delivered,
        refusal_reason: (ordered != delivered).then(|| "damaged".to_owned()),
    }
}

/// `DeliveryDelivered` with `lines`, a COD difference of `difference` fils
/// and the absorber when there is one.
fn delivered(
    lines: Vec<DeliveredLine>,
    difference: i64,
    absorbed_by: Option<&str>,
) -> DeliveryDelivered {
    DeliveryDelivered {
        delivery: DELIVERY,
        source: order(),
        attempt: 1,
        proof: 1,
        lines,
        cash_expected: 10_500,
        cash_collected: 10_500 - difference,
        cod_difference: difference,
        cod_difference_reason: (difference > 0).then(|| "recipient short of cash".to_owned()),
        absorbed_by: absorbed_by.map(str::to_owned),
        cheque: None,
    }
}

async fn lines_of(t: &Booted, invoice: &Invoice) -> Vec<InvoiceLine> {
    wl_invoicing::invoicer::lines_of(t.db(), invoice.id)
        .await
        .unwrap()
}

/// AC1: an order line of 10 cartons at AED 50.00 and `DeliveryDelivered` with
/// 8 delivered and 2 refused: the draft bills 8 x AED 50.00 = AED 400.00 plus
/// VAT AED 20.00; the refused cartons are not billed. A second event changes
/// nothing.
#[renox::test]
async fn the_draft_bills_the_delivered_quantity_and_never_the_refused_one() {
    let t = boot().await;
    let draft = draft_of(&t, &[(1, 10_000, 5_000)]).await;
    assert_eq!(draft.gross_total, 52_500);

    t.state()
        .emit(delivered(vec![delivered_line(1, 10_000, 8_000)], 0, None))
        .await
        .unwrap();

    let lines = lines_of(&t, &draft).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].quantity_milli, 8_000);
    assert_eq!(lines[0].net_amount, 40_000);
    assert_eq!(lines[0].vat_amount, 2_000);
    let after = t.reload(&draft).await;
    assert_eq!(
        (after.net_total, after.vat_total, after.gross_total, after.balance),
        (40_000, 2_000, 42_000, 42_000)
    );
    assert_eq!(after.status, InvoiceStatus::Draft);

    t.state()
        .emit(delivered(vec![delivered_line(1, 10_000, 8_000)], 0, None))
        .await
        .unwrap();
    assert_eq!(t.reload(&draft).await.gross_total, 42_000);
}

/// AC1 (edge): a line with nothing delivered leaves the draft and the others
/// close up; a draft whose every line has nothing delivered is not kept; an
/// issued invoice is never rewritten; the draft's discount follows the
/// quantity.
#[renox::test]
async fn a_line_with_nothing_delivered_leaves_and_an_empty_draft_is_discarded() {
    let t = boot().await;
    let draft = draft_of(&t, &[(1, 2_000, 10_000), (2, 3_000, 10_000), (3, 1_000, 10_000)]).await;

    t.state()
        .emit(delivered(
            vec![delivered_line(2, 3_000, 0), delivered_line(3, 1_000, 1_000)],
            0,
            None,
        ))
        .await
        .unwrap();

    let lines = lines_of(&t, &draft).await;
    assert_eq!(
        lines.iter().map(|l| (l.position, l.origin_id)).collect::<Vec<_>>(),
        [(1, Some(1)), (2, Some(3))]
    );
    // Lines 1 and 3 are untouched: 2 x 100.00 + 1 x 100.00 plus 5% VAT.
    assert_eq!(t.reload(&draft).await.gross_total, 31_500);

    // Nothing delivered of the rest: the draft is gone.
    t.state()
        .emit(delivered(
            vec![delivered_line(1, 2_000, 0), delivered_line(3, 1_000, 0)],
            0,
            None,
        ))
        .await
        .unwrap();
    assert!(Invoice::find(t.db(), draft.id).await.unwrap().is_none());

    let a = t.user_a().await;
    let issued = {
        let draft = draft_of(&t, &[(5, 10_000, 5_000)]).await;
        t.issue(&draft, &a).await
    };
    t.state()
        .emit(delivered(vec![delivered_line(5, 10_000, 8_000)], 0, None))
        .await
        .unwrap();
    assert_eq!(t.reload(&issued).await.gross_total, 52_500);
}

/// AC2: a COD difference of AED 30.00 owed by the customer: the invoice keeps
/// AED 30.00 open and the reminder goes out for AED 30.00.
#[renox::test]
async fn a_difference_the_customer_owes_stays_open_and_is_chased() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t
        .issued(&[(100_000, VatCategory::Standard)], Some(t.day(-10)), &a)
        .await;
    assert_eq!(invoice.gross_total, 105_000);
    t.pay(&invoice, 1, 102_000).await;

    t.state()
        .emit(delivered(Vec::new(), 3_000, Some("customer")))
        .await
        .unwrap();

    let after = t.reload(&invoice).await;
    assert_eq!(after.balance, 3_000);
    assert_eq!(after.status, InvoiceStatus::PartiallyPaid);
    assert!(
        invoicer(t.state())
            .credit_notes_for(t.db(), InvoiceId(invoice.id))
            .await
            .unwrap()
            .is_empty()
    );
    let row = CodDifference::of_delivery(t.db(), DELIVERY.0).await.unwrap().unwrap();
    assert_eq!(row.outcome, CodOutcome::CustomerOwes);
    assert_eq!(row.absorbed_by, "customer");

    assert_eq!(reminders::run(t.state(), t.day(0)).await.unwrap(), 1);
    let sent = t.notices.of_type(AFTER_DUE);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].data["balance"], "AED 30.00");
}

/// AC3: AED 30.00 absorbed by the client (03): the invoice is not touched and
/// the remittance line is -AED 30.00 with the reason.
#[renox::test]
async fn a_difference_the_client_absorbs_is_its_remittance_line() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t
        .issued(&[(100_000, VatCategory::Standard)], None, &a)
        .await;
    let event = delivered(Vec::new(), 3_000, Some("client"));
    let absorption = event.cod_absorption().expect("a difference with an absorber");
    assert_eq!(absorption.remittance_adjustment(), -3_000);
    assert_eq!(absorption.reason, "recipient short of cash");

    t.state().emit(event).await.unwrap();

    let after = t.reload(&invoice).await;
    assert_eq!((after.balance, after.status), (105_000, InvoiceStatus::Issued));
    assert!(
        invoicer(t.state())
            .credit_notes_for(t.db(), InvoiceId(invoice.id))
            .await
            .unwrap()
            .is_empty()
    );
    let row = CodDifference::of_delivery(t.db(), DELIVERY.0).await.unwrap().unwrap();
    assert_eq!(row.outcome, CodOutcome::ClientRemittance);
    assert_eq!(row.amount, 3_000);
}

/// AC4: AED 30.00 absorbed by the business on a draft: the draft has a
/// -AED 30.00 line "COD difference absorbed (business)" with the reason; an
/// event received twice reduces it once.
#[renox::test]
async fn a_difference_the_business_absorbs_is_a_negative_line_on_a_draft() {
    let t = boot().await;
    let draft = draft_of(&t, &[(1, 1_000, 10_000)]).await;
    assert_eq!(draft.gross_total, 10_500);

    let event = delivered(vec![delivered_line(1, 1_000, 1_000)], 3_000, Some("business"));
    t.state().emit(event.clone()).await.unwrap();
    t.state().emit(event).await.unwrap();

    let lines = lines_of(&t, &draft).await;
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1].position, 2);
    assert!(lines[1].description.starts_with("COD difference absorbed (business)"));
    assert!(lines[1].description.contains("recipient short of cash"));
    assert_eq!(lines[1].gross_amount, -3_000);
    assert_eq!(lines[1].net_amount, -2_857);
    assert_eq!(lines[1].origin(), Some(DELIVERY.subject()));
    let after = t.reload(&draft).await;
    assert_eq!((after.gross_total, after.balance), (7_500, 7_500));
    let row = CodDifference::of_delivery(t.db(), DELIVERY.0).await.unwrap().unwrap();
    assert_eq!(row.outcome, CodOutcome::DraftReduced);
    assert_eq!(row.line_position, Some(2));
}

/// AC4: AED 30.00 absorbed by the business on an issued invoice: a credit note
/// of AED 30.00 with the reason; the driver's share is a credit note too (and
/// is recovered in the cash handover, never from the customer).
#[renox::test]
async fn a_difference_the_business_or_driver_absorbs_is_a_credit_note_on_an_issued_invoice() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t
        .issued(&[(100_000, VatCategory::Standard)], None, &a)
        .await;
    t.pay(&invoice, 1, 102_000).await;

    t.state()
        .emit(delivered(Vec::new(), 3_000, Some("business")))
        .await
        .unwrap();

    let notes = invoicer(t.state())
        .credit_notes_for(t.db(), InvoiceId(invoice.id))
        .await
        .unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].gross_total, 3_000);
    assert_ne!(notes[0].status, CreditNoteStatus::Draft);
    let after = t.reload(&invoice).await;
    assert_eq!((after.balance, after.status), (0, InvoiceStatus::Paid));
    let row = CodDifference::of_delivery(t.db(), DELIVERY.0).await.unwrap().unwrap();
    assert_eq!(row.outcome, CodOutcome::Credited);
    assert!(row.credit_note_id.is_some());

    // The same event again credits nothing more.
    t.state()
        .emit(delivered(Vec::new(), 3_000, Some("business")))
        .await
        .unwrap();
    assert_eq!(
        invoicer(t.state())
            .credit_notes_for(t.db(), InvoiceId(invoice.id))
            .await
            .unwrap()
            .len(),
        1
    );

    // Another delivery of the driver's share credits its own note.
    let mut other = delivered(Vec::new(), 2_000, Some("driver"));
    other.delivery = DeliveryId(43);
    t.state().emit(other).await.unwrap();
    let notes = invoicer(t.state())
        .credit_notes_for(t.db(), InvoiceId(invoice.id))
        .await
        .unwrap();
    assert_eq!(notes.iter().map(|n| n.gross_total).sum::<i64>(), 5_000);
}

/// A difference of a delivery that no invoice bills (yet) is recorded and
/// changes nothing; a difference without an absorber is not handled.
#[renox::test]
async fn a_difference_without_an_invoice_or_an_absorber_changes_nothing() {
    let t = boot().await;
    t.state()
        .emit(delivered(Vec::new(), 3_000, Some("business")))
        .await
        .unwrap();
    let row = CodDifference::of_delivery(t.db(), DELIVERY.0).await.unwrap().unwrap();
    assert_eq!(row.outcome, CodOutcome::NoInvoice);
    assert_eq!(row.invoice_id, None);

    let mut other = delivered(Vec::new(), 3_000, None);
    other.delivery = DeliveryId(44);
    t.state().emit(other).await.unwrap();
    assert!(CodDifference::of_delivery(t.db(), 44).await.unwrap().is_none());
}
