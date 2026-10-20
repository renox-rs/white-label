//! Drafting from a return and issuing a supplier debit note (wl-invoicing
//! §4, §5.13 steps 1 to 3, `FIN.RUL.17`, `FIN.ACL.17`, #76): the draft at the
//! bill's prices or at the PO price, the number, the split between the bill
//! and the supplier's debit balance, the cost reversal, the input VAT and the
//! event.

mod support;

use renox::chrono::{Datelike, NaiveDate};
use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_invoicing::bill_machine::approve;
use wl_invoicing::bill_pay::pay;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::cash_forecast_load::payable;
use wl_invoicing::credit_notes::By;
use wl_invoicing::debit_note_vat;
use wl_invoicing::debit_notes::{self, DebitLine};
use wl_invoicing::supplier_bill::{self, BillDraft};
use wl_invoicing::vat_summary::{VatSummary, ensure_open};
use wl_invoicing::{
    BillAllocation, BillLine, BillLineDraft, DebitNoteOrigin, PricedAt, SupplierBill,
    SupplierBillStatus, SupplierDebitNote,
};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    SupplierDebitNoteStatus, SupplierReturnDispatched, SupplierReturnLine, VatCategory,
    supplier_debit_notes,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::{LegalEntity, Subject};

use support::{BLUE_DUNES, Booted, CostLedger, boot, boot_with_costs};

const A: i64 = 7;
const B: i64 = 8;
const DRAFTED: &str = "invoicing.supplier_debit_note_drafted";
const ISSUED: &str = "invoicing.supplier_debit_note_issued";

/// `n` days from today (negative: before).
fn day(n: i64) -> NaiveDate {
    let days = renox::chrono::Days::new(n.unsigned_abs());
    if n >= 0 {
        today() + days
    } else {
        today() - days
    }
}

fn po_line() -> Subject {
    Subject::new("procurement.po_line", 31)
}

fn unit() -> Subject {
    Subject::new("property.unit", 4)
}

/// An approved bill of 10 units at `price` fils + 5% VAT (one line, on the
/// PO line, coded to the cost object).
async fn bill_at(t: &Booted, number: &str, price: i64) -> (SupplierBill, BillLine) {
    let line = BillLineDraft::new("Pump", price, VatCategory::Standard)
        .quantity_milli(10_000)
        .po_line(po_line())
        .cost_object(unit());
    let draft = BillDraft::new(BLUE_DUNES, number, day(0)).line(line);
    let bill = supplier_bill::create(t.db(), &draft, Some(A))
        .await
        .unwrap();
    submit(t.state(), By::System(Some(A)), bill.id)
        .await
        .unwrap();
    let bill = approve(t.state(), By::System(Some(B)), bill.id, "")
        .await
        .unwrap();
    let line = supplier_bill::lines_of(t.db(), bill.id)
        .await
        .unwrap()
        .remove(0);
    (bill, line)
}

/// The bill of the examples: 10 units at AED 100.00 + 5% VAT (AED 1,050.00).
async fn approved_bill(t: &Booted) -> (SupplierBill, BillLine) {
    bill_at(t, "INV-9", 10_000).await
}

/// 3 units back to the supplier, bought at `po_price` fils.
fn dispatched(return_id: i64, qty_milli: i64, po_price: i64) -> SupplierReturnDispatched {
    SupplierReturnDispatched {
        return_id,
        supplier: BLUE_DUNES,
        source: Subject::new("inventory.supplier_return", return_id),
        lines: vec![SupplierReturnLine {
            item: Subject::new("inventory.item", 5),
            lot: None,
            qty_milli,
            po_line: po_line(),
            po_price,
            reason: "damaged".to_owned(),
        }],
    }
}

fn refused<T>(result: std::result::Result<T, TransitionError>) -> Option<String> {
    match result {
        Err(error) => match error.into_error() {
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message) => Some(message),
            other => panic!("not a 422: {other:?}"),
        },
        Ok(_) => None,
    }
}

async fn reload(t: &Booted, note: &SupplierDebitNote) -> SupplierDebitNote {
    SupplierDebitNote::find_or_404(t.db(), note.id).await.unwrap()
}

async fn bill_now(t: &Booted, bill: &SupplierBill) -> SupplierBill {
    SupplierBill::find_or_404(t.db(), bill.id).await.unwrap()
}

/// A return of 3 units is dispatched for the `approved` bill: a draft debit
/// note for AED 300.00 + AED 15.00 VAT is created, at the bill's prices, and
/// the accountant is told. The same return again changes nothing.
#[renox::test]
async fn inv13_a_dispatched_return_is_drafted_at_the_bills_prices_once() {
    let t = boot().await;
    let accountant = t.accountant("Zaid").await;
    let (bill, bill_line) = approved_bill(&t).await;

    // The event is the way wl-inventory tells.
    t.state()
        .emit(dispatched(12, 3_000, 9_500))
        .await
        .unwrap();
    let notes = SupplierDebitNote::query().get(t.db()).await.unwrap();
    assert_eq!(notes.len(), 1);
    let note = &notes[0];
    assert_eq!(note.status, SupplierDebitNoteStatus::Draft);
    assert_eq!(note.origin, DebitNoteOrigin::Return);
    assert_eq!(note.bill_id, Some(bill.id));
    assert_eq!(note.party_id, BLUE_DUNES.0);
    assert_eq!(note.legal_entity_id, bill.legal_entity_id.unwrap());
    assert_eq!(
        note.source(),
        Some(Subject::new("inventory.supplier_return", 12))
    );
    assert_eq!(note.number, None);
    assert_eq!(
        (note.net_total, note.vat_total, note.gross_total),
        (30_000, 1_500, 31_500)
    );
    let lines = debit_notes::lines_of(t.db(), note.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].priced_at, PricedAt::Bill);
    assert_eq!(lines[0].bill_line_id, Some(bill_line.id));
    assert_eq!(lines[0].unit_price, 10_000);
    assert_eq!(lines[0].quantity_milli, 3_000);
    assert_eq!(lines[0].cost_object(), Some(unit()));

    let told = t.notices.of_type(DRAFTED);
    assert_eq!(told.len(), 1);
    assert!(told[0]
        .recipients
        .iter()
        .any(|r| matches!(r, NotifyRecipient::User { id } if *id == accountant.id)));

    // One draft per return, also through the contract.
    t.state()
        .emit(dispatched(12, 3_000, 9_500))
        .await
        .unwrap();
    let again = supplier_debit_notes(t.state())
        .draft_for_return(t.state(), dispatched(12, 3_000, 9_500))
        .await
        .unwrap();
    assert_eq!(again.0, note.id);
    assert_eq!(SupplierDebitNote::query().get(t.db()).await.unwrap().len(), 1);
    assert_eq!(t.notices.of_type(DRAFTED).len(), 1);
    // The bill is not touched by a draft.
    assert_eq!(bill_now(&t, &bill).await.balance, 105_000);
}

/// Issuing: the number, the bill balance falls by AED 315.00, AED 300.00 of
/// cost is reversed on the cost object, input VAT falls by AED 15.00, the
/// supplier is told and `SupplierDebitNoteIssued` is emitted.
#[renox::test]
async fn inv13_issue_reduces_the_bill_reverses_cost_and_input_vat() {
    let (t, ledger): (Booted, CostLedger) = boot_with_costs().await;
    let (bill, _) = approved_bill(&t).await;
    t.state()
        .emit(dispatched(12, 3_000, 9_500))
        .await
        .unwrap();
    let draft = SupplierDebitNote::query().first(t.db()).await.unwrap().unwrap();
    let entity = bill.legal_entity_id.unwrap();
    let config = t.config();
    let (from, to) = (day(-1), day(1));
    assert_eq!(debit_note_vat::reduction(t.db(), &config, entity, from, to).await.unwrap(), 0);
    // Before: the bill's input VAT of AED 50.00 alone.
    assert_eq!(payable(t.db(), &config, entity, day(-30), day(30)).await.unwrap(), -5_000);

    let issued = debit_notes::issue(t.state(), By::System(Some(B)), draft.id)
        .await
        .unwrap();
    assert_eq!(issued.status, SupplierDebitNoteStatus::Issued);
    let number = issued.number.clone().unwrap();
    assert!(number.starts_with("DN-"), "{number}");
    assert!(number.ends_with("-0001"), "{number}");
    assert!(number.contains(&today().year().to_string()), "{number}");
    assert_eq!(issued.issue_date, Some(today()));
    assert_eq!(issued.issued_by, Some(B));
    assert_eq!((issued.applied_to_bill, issued.debit_balance), (31_500, 0));

    // The bill balance falls by AED 315.00 through `pay`, from the note.
    let after = bill_now(&t, &bill).await;
    assert_eq!(after.status, SupplierBillStatus::PartiallyPaid);
    assert_eq!((after.allocated_total, after.balance), (31_500, 73_500));
    assert_eq!(after.gross_total, 105_000);
    let allocations = BillAllocation::of_bill(t.db(), bill.id).await.unwrap();
    assert_eq!(allocations.len(), 1);
    assert_eq!(allocations[0].source(), issued.subject());
    assert_eq!(allocations[0].amount, 31_500);

    // Cost of AED 300.00 reversed on the cost object, once.
    let reversals = ledger.reversals();
    assert_eq!(reversals.len(), 1);
    assert_eq!(reversals[0].source, issued.subject());
    assert_eq!(reversals[0].key, "invoicing.supplier_debit_note:1");
    assert_eq!(reversals[0].lines.len(), 1);
    assert_eq!(reversals[0].lines[0].cost_object, unit());
    assert_eq!(reversals[0].lines[0].amount_aed, 30_000);

    // Input VAT falls by AED 15.00.
    assert_eq!(debit_note_vat::reduction(t.db(), &config, entity, from, to).await.unwrap(), 1_500);
    assert_eq!(payable(t.db(), &config, entity, day(-30), day(30)).await.unwrap(), -3_500);

    // The supplier is told, and the event is emitted.
    let told = t.notices.of_type(ISSUED);
    assert_eq!(told.len(), 1);
    assert!(told[0]
        .recipients
        .iter()
        .any(|r| matches!(r, NotifyRecipient::Party { id } if *id == BLUE_DUNES)));
    assert_eq!(told[0].data["number"], number.as_str());
    let events = t.debit_issued.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].debit_note.0, issued.id);
    assert_eq!(events[0].supplier, BLUE_DUNES);
    assert_eq!(events[0].bill.0, bill.id);
    assert_eq!((events[0].amount, events[0].vat), (31_500, 1_500));
    assert_eq!(
        events[0].return_to_supplier,
        Some(Subject::new("inventory.supplier_return", 12))
    );

    // Not issued twice.
    assert!(refused(debit_notes::issue(t.state(), By::System(Some(B)), draft.id).await).is_some());
    assert_eq!(t.debit_issued.all().len(), 1);
}

/// On a `paid` bill, issuing opens the supplier's debit balance of AED
/// 315.00 and the bill stays `paid`; on a partly paid bill the part above the
/// open balance becomes the debit balance.
#[renox::test]
async fn inv13_a_paid_bill_gets_a_debit_balance_and_stays_paid() {
    let t = boot().await;
    let (bill, _) = approved_bill(&t).await;
    pay(t.state(), &bill, &Subject::new("payments.payment", 9), 105_000, None)
        .await
        .unwrap();
    assert_eq!(bill_now(&t, &bill).await.status, SupplierBillStatus::Paid);
    t.state()
        .emit(dispatched(12, 3_000, 9_500))
        .await
        .unwrap();
    let draft = SupplierDebitNote::query().first(t.db()).await.unwrap().unwrap();
    let issued = debit_notes::issue(t.state(), By::System(Some(B)), draft.id)
        .await
        .unwrap();
    assert_eq!((issued.applied_to_bill, issued.debit_balance), (0, 31_500));
    let after = bill_now(&t, &bill).await;
    assert_eq!(after.status, SupplierBillStatus::Paid);
    assert_eq!((after.allocated_total, after.balance), (105_000, 0));
    assert!(BillAllocation::of_bill(t.db(), bill.id)
        .await
        .unwrap()
        .iter()
        .all(|a| a.source() != issued.subject()));
    // The kernel contract reads the balance the supplier owes.
    let owed = supplier_debit_notes(t.state())
        .debit_balance(t.db(), BLUE_DUNES, bill.legal_entity_id.unwrap(), "AED")
        .await
        .unwrap();
    assert_eq!(owed, 31_500);
}

#[renox::test]
async fn inv13_the_part_above_the_open_balance_of_a_partly_paid_bill_is_the_debit_balance() {
    let t = boot().await;
    let (bill, _) = approved_bill(&t).await;
    // AED 1,050.00 less AED 100.00 paid: AED 10.00 is open.
    pay(t.state(), &bill, &Subject::new("payments.payment", 9), 104_000, None)
        .await
        .unwrap();
    t.state()
        .emit(dispatched(12, 3_000, 9_500))
        .await
        .unwrap();
    let draft = SupplierDebitNote::query().first(t.db()).await.unwrap().unwrap();
    let issued = debit_notes::issue(t.state(), By::System(Some(B)), draft.id)
        .await
        .unwrap();
    assert_eq!((issued.applied_to_bill, issued.debit_balance), (1_000, 30_500));
    let after = bill_now(&t, &bill).await;
    assert_eq!(after.status, SupplierBillStatus::Paid);
    assert_eq!(after.balance, 0);
}

/// A manual debit note by user A is issued only by user B (`FIN.ACL.17`); a
/// refusal uses no number.
#[renox::test]
async fn inv13_a_manual_note_is_issued_by_another_person_than_its_creator() {
    let t = boot().await;
    let (bill, line) = approved_bill(&t).await;
    let note = debit_notes::draft_manual(
        t.db(),
        &bill,
        "price_difference",
        "",
        &[DebitLine::new(line.id, 3_000, 10_000)],
        Some(A),
    )
    .await
    .unwrap();
    let message = refused(debit_notes::issue(t.state(), By::System(Some(A)), note.id).await);
    assert!(message.unwrap().contains("FIN.ACL.17"));
    assert_eq!(reload(&t, &note).await.status, SupplierDebitNoteStatus::Draft);
    assert_eq!(reload(&t, &note).await.number, None);
    assert_eq!(bill_now(&t, &bill).await.balance, 105_000);

    let issued = debit_notes::issue(t.state(), By::System(Some(B)), note.id)
        .await
        .unwrap();
    assert_eq!(issued.status, SupplierDebitNoteStatus::Issued);
    // The refusal took no number: the first issue is the first number.
    assert!(issued.number.unwrap().ends_with("-0001"));
}

/// A debit note for 8 units after an earlier one for 3 (bill of 10) is
/// refused, at the draft and, for two drafts made together, at the issue.
#[renox::test]
async fn inv13_quantities_are_limited_by_the_bill_less_earlier_notes() {
    let t = boot().await;
    let (bill, line) = approved_bill(&t).await;
    let line_of = |quantity_milli| [DebitLine::new(line.id, quantity_milli, 10_000)];
    let first = debit_notes::draft_manual(t.db(), &bill, "short_delivery", "", &line_of(3_000), Some(A))
        .await
        .unwrap();
    // Two drafts, each within the bill on its own.
    let second = debit_notes::draft_manual(t.db(), &bill, "short_delivery", "", &line_of(8_000), Some(A))
        .await
        .unwrap();
    debit_notes::issue(t.state(), By::System(Some(B)), first.id)
        .await
        .unwrap();
    // 3 issued + 8 is above 10: refused at the issue, and at a new draft.
    let message = refused(debit_notes::issue(t.state(), By::System(Some(B)), second.id).await);
    assert!(message.unwrap().contains("bill line"));
    assert_eq!(reload(&t, &second).await.status, SupplierDebitNoteStatus::Draft);
    let late = debit_notes::draft_manual(t.db(), &bill, "short_delivery", "", &line_of(8_000), Some(A)).await;
    assert!(matches!(late, Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))));
    // The bill only fell by the first note.
    assert_eq!(bill_now(&t, &bill).await.balance, 105_000 - 31_500);
}

/// A return dispatched before the bill waits as a draft at the PO price
/// (AED 95.00 a unit); when the bill at AED 100.00 a unit is approved, the
/// draft links to it and is re-priced to AED 100.00. It cannot be issued
/// while it waits.
#[renox::test]
async fn inv13_a_return_before_the_bill_waits_and_is_repriced_when_the_bill_is_approved() {
    let t = boot().await;
    t.state()
        .emit(dispatched(12, 3_000, 9_500))
        .await
        .unwrap();
    let waiting = SupplierDebitNote::query().first(t.db()).await.unwrap().unwrap();
    assert_eq!(waiting.bill_id, None);
    assert_eq!(waiting.origin, DebitNoteOrigin::Return);
    assert_eq!(waiting.net_total, 28_500);
    let lines = debit_notes::lines_of(t.db(), waiting.id).await.unwrap();
    assert_eq!(lines[0].priced_at, PricedAt::Po);
    assert_eq!(lines[0].unit_price, 9_500);
    assert_eq!(lines[0].bill_line_id, None);
    let message = refused(debit_notes::issue(t.state(), By::System(Some(B)), waiting.id).await);
    assert!(message.unwrap().contains("waiting"));

    let (bill, bill_line) = approved_bill(&t).await;
    let linked = reload(&t, &waiting).await;
    assert_eq!(linked.bill_id, Some(bill.id));
    assert_eq!(linked.status, SupplierDebitNoteStatus::Draft);
    assert_eq!(
        (linked.net_total, linked.vat_total, linked.gross_total),
        (30_000, 1_500, 31_500)
    );
    let lines = debit_notes::lines_of(t.db(), linked.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].priced_at, PricedAt::Bill);
    assert_eq!(lines[0].unit_price, 10_000);
    assert_eq!(lines[0].bill_line_id, Some(bill_line.id));
    assert_eq!(lines[0].cost_object(), Some(unit()));
    // Now it can be issued.
    let issued = debit_notes::issue(t.state(), By::System(Some(B)), linked.id)
        .await
        .unwrap();
    assert_eq!(issued.status, SupplierDebitNoteStatus::Issued);
}

/// A bill of another supplier does not take a waiting draft.
#[renox::test]
async fn inv13_a_bill_of_another_supplier_does_not_link_a_waiting_draft() {
    let t = boot().await;
    t.state()
        .emit(dispatched(12, 3_000, 9_500))
        .await
        .unwrap();
    let line = BillLineDraft::new("Pump", 10_000, VatCategory::Standard)
        .quantity_milli(10_000)
        .po_line(po_line());
    let draft = BillDraft::new(support::QUIET_SANDS, "INV-1", day(0)).line(line);
    let bill = supplier_bill::create(t.db(), &draft, Some(A)).await.unwrap();
    submit(t.state(), By::System(Some(A)), bill.id).await.unwrap();
    approve(t.state(), By::System(Some(B)), bill.id, "").await.unwrap();
    let waiting = SupplierDebitNote::query().first(t.db()).await.unwrap().unwrap();
    assert_eq!(waiting.bill_id, None);
}

/// A debit note is not issued in a locked VAT period of its entity
/// (`FIN.ACL.09`).
#[renox::test]
async fn inv13_issue_is_refused_in_a_locked_vat_period() {
    let t = boot().await;
    let (bill, line) = approved_bill(&t).await;
    let note = debit_notes::draft_manual(
        t.db(),
        &bill,
        "price_difference",
        "",
        &[DebitLine::new(line.id, 3_000, 10_000)],
        Some(A),
    )
    .await
    .unwrap();
    let entity = LegalEntity::find_or_404(t.db(), bill.legal_entity_id.unwrap())
        .await
        .unwrap();
    ensure_open(t.db(), &t.config(), &entity, today()).await.unwrap();
    let summary = VatSummary::containing(t.db(), &t.config(), &entity, today())
        .await
        .unwrap()
        .unwrap();
    renox::db::sql(format!("UPDATE {} SET status = 'locked' WHERE id = ?", VatSummary::TABLE))
        .bind(summary.id)
        .execute(t.db())
        .await
        .unwrap();
    let message = refused(debit_notes::issue(t.state(), By::System(Some(B)), note.id).await);
    assert!(message.unwrap().contains("VAT period"));
    assert_eq!(reload(&t, &note).await.status, SupplierDebitNoteStatus::Draft);
    assert_eq!(bill_now(&t, &bill).await.balance, 105_000);
}
