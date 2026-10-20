//! Settling and cancelling a supplier debit note (wl-invoicing §4 `settle`
//! and `cancel`, §5.13 steps 4 and 5, #77): the supplier's tax credit note,
//! the refund, the set off on a later bill, the accepted difference and the
//! cancel that gives every effect back.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_invoicing::bill_machine::approve;
use wl_invoicing::bill_pay::pay;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::credit_notes::By;
use wl_invoicing::debit_note_settle::{
    cancel, open_tasks, record_tax_credit_note, refund, set_off, settle,
};
use wl_invoicing::debit_note_vat;
use wl_invoicing::debit_notes::{self, DebitLine};
use wl_invoicing::supplier_bill::{self, BillDraft};
use wl_invoicing::{
    BillLine, BillLineDraft, DebitMovementKind, DebitNoteMovement, InputVatTiming, SupplierBill,
    SupplierBillStatus, SupplierDebitNote, WlInvoicing,
};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    SupplierBillStatus as KernelBillStatus, SupplierDebitNoteStatus, VatCategory,
    supplier_debit_notes,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::{PartyId, Subject};

use support::{BLUE_DUNES, Booted, QUIET_SANDS, boot, boot_with, boot_with_costs};

const A: i64 = 7;
const B: i64 = 8;

fn day(n: i64) -> NaiveDate {
    let days = renox::chrono::Days::new(n.unsigned_abs());
    if n >= 0 {
        today() + days
    } else {
        today() - days
    }
}

fn unit() -> Subject {
    Subject::new("property.unit", 4)
}

fn payment(id: i64) -> Subject {
    Subject::new("payments.payment", id)
}

/// An approved bill of `units` units at AED 100.00 + 5% VAT from `supplier`
/// (entity `entity`, the default one when `None`), coded to the cost object.
async fn bill_of(
    t: &Booted,
    supplier: PartyId,
    number: &str,
    units: i64,
    entity: Option<i64>,
) -> (SupplierBill, BillLine) {
    let line = BillLineDraft::new("Pump", 10_000, VatCategory::Standard)
        .quantity_milli(units * 1_000)
        .cost_object(unit());
    let mut draft = BillDraft::new(supplier, number, day(0)).line(line);
    draft.legal_entity_id = entity;
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

/// A manual debit note for 3 units (AED 300.00 + AED 15.00 VAT) of the bill
/// line, drafted by A and issued by B.
async fn issued_note(t: &Booted, bill: &SupplierBill, line: &BillLine) -> SupplierDebitNote {
    let draft = debit_notes::draft_manual(
        t.db(),
        bill,
        "price_difference",
        "",
        &[DebitLine::new(line.id, 3_000, 10_000)],
        Some(A),
    )
    .await
    .unwrap();
    debit_notes::issue(t.state(), By::System(Some(B)), draft.id)
        .await
        .unwrap()
}

/// A debit note of AED 315.00 issued on a bill that was paid in full: the
/// supplier owes AED 315.00.
async fn on_paid_bill(t: &Booted, number: &str) -> (SupplierBill, SupplierDebitNote) {
    let (bill, line) = bill_of(t, BLUE_DUNES, number, 10, None).await;
    pay(t.state(), &bill, &payment(9), 105_000, None)
        .await
        .unwrap();
    let note = issued_note(t, &bill, &line).await;
    assert_eq!((note.applied_to_bill, note.debit_balance), (0, 31_500));
    (bill, note)
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

/// An `issued` debit note of AED 315.00 on a `paid` bill, an incoming payment
/// of AED 315.00 from the supplier allocated to it and the tax credit note
/// recorded: it is `settled`. Until the tax credit note is recorded it stays
/// `issued` and is not settled by hand either.
#[renox::test]
async fn inv14_the_supplier_payment_and_the_tax_credit_note_settle_the_note() {
    let t = boot().await;
    let (_, note) = on_paid_bill(&t, "INV-9").await;
    assert_eq!(open_tasks(t.db(), &note).await.unwrap().len(), 1);

    // The payment back is a refund through the kernel contract.
    supplier_debit_notes(t.state())
        .settle_from_payment(
            t.state(),
            wl_kernel::invoicing::SupplierDebitNoteId(note.id),
            payment(21),
            31_500,
            "payments.allocation:5",
        )
        .await
        .unwrap();
    let after = reload(&t, &note).await;
    assert_eq!(after.status, SupplierDebitNoteStatus::Issued);
    assert_eq!(after.debit_balance, 0);
    assert!(t.debit_settled.all().is_empty());
    // The same delivery again changes nothing.
    refund(t.state(), note.id, &payment(21), 31_500, "payments.allocation:5")
        .await
        .unwrap();
    let moves = DebitNoteMovement::of_note(t.db(), note.id).await.unwrap();
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].kind, DebitMovementKind::Refund);
    assert_eq!(moves[0].target(), Some(payment(21)));

    // `settle` without the tax credit note (setting required) is refused.
    let message = refused(settle(t.state(), By::System(Some(B)), note.id, None).await);
    assert!(message.unwrap().contains("tax credit note"));
    assert_eq!(reload(&t, &note).await.status, SupplierDebitNoteStatus::Issued);

    // The last thing to come settles it, by the user who recorded it.
    let settled = record_tax_credit_note(
        t.state(),
        By::System(Some(B)),
        note.id,
        "TCN-77",
        day(0),
        Some(31),
    )
    .await
    .unwrap();
    assert_eq!(settled.status, SupplierDebitNoteStatus::Settled);
    assert_eq!(settled.tax_credit_note_number.as_deref(), Some("TCN-77"));
    assert_eq!(settled.tax_credit_note_document, Some(31));
    assert_eq!(settled.settled_by, Some(B));
    assert_eq!(settled.settle_difference, 0);
    assert!(settled.settled_at.is_some());
    assert!(open_tasks(t.db(), &note).await.unwrap().is_empty());
    let events = t.debit_settled.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].debit_note.0, note.id);
    assert_eq!((events[0].amount, events[0].difference), (31_500, 0));
}

/// With a payment of AED 300.00 it settles with a difference of AED 15.00,
/// and the reason is stored; a difference needs a reason and must be what is
/// left.
#[renox::test]
async fn inv14_a_short_payment_settles_with_the_difference_and_its_reason() {
    let t = boot().await;
    let (_, note) = on_paid_bill(&t, "INV-9").await;
    refund(t.state(), note.id, &payment(21), 30_000, "payments.allocation:5")
        .await
        .unwrap();
    record_tax_credit_note(t.state(), By::System(Some(B)), note.id, "TCN-77", day(0), None)
        .await
        .unwrap();
    // AED 15.00 is still open, so the note is not settled by itself.
    let open = reload(&t, &note).await;
    assert_eq!(open.status, SupplierDebitNoteStatus::Issued);
    assert_eq!(open.debit_balance, 1_500);

    let no_reason = refused(settle(t.state(), By::System(Some(B)), note.id, Some((1_500, " "))).await);
    assert!(no_reason.unwrap().contains("reason"));
    let wrong = refused(settle(t.state(), By::System(Some(B)), note.id, Some((1_000, "short"))).await);
    assert!(wrong.unwrap().contains("not fully applied"));
    let none = refused(settle(t.state(), By::System(Some(B)), note.id, None).await);
    assert!(none.is_some());
    assert_eq!(reload(&t, &note).await.status, SupplierDebitNoteStatus::Issued);

    let settled = settle(
        t.state(),
        By::System(Some(B)),
        note.id,
        Some((1_500, "Supplier deducted the freight")),
    )
    .await
    .unwrap();
    assert_eq!(settled.status, SupplierDebitNoteStatus::Settled);
    assert_eq!(settled.settle_difference, 1_500);
    assert_eq!(
        settled.settle_difference_reason.as_deref(),
        Some("Supplier deducted the freight")
    );
    assert_eq!(settled.debit_balance, 0);
    let moves = DebitNoteMovement::of_note(t.db(), note.id).await.unwrap();
    assert_eq!(moves.len(), 2);
    assert_eq!(moves[1].kind, DebitMovementKind::Difference);
    assert_eq!(moves[1].amount, 1_500);
    let events = t.debit_settled.all();
    assert_eq!(events.len(), 1);
    assert_eq!((events[0].amount, events[0].difference), (31_500, 1_500));
}

/// The tax credit note may come first: the refund then settles the note.
#[renox::test]
async fn inv14_the_refund_settles_when_the_tax_credit_note_is_already_there() {
    let t = boot().await;
    let (_, note) = on_paid_bill(&t, "INV-9").await;
    let recorded =
        record_tax_credit_note(t.state(), By::System(Some(B)), note.id, "TCN-1", day(0), None)
            .await
            .unwrap();
    assert_eq!(recorded.status, SupplierDebitNoteStatus::Issued);
    // Not twice.
    let twice =
        record_tax_credit_note(t.state(), By::System(Some(B)), note.id, "TCN-2", day(0), None).await;
    assert!(refused(twice).is_some());
    refund(t.state(), note.id, &payment(21), 31_500, "k1").await.unwrap();
    assert_eq!(reload(&t, &note).await.status, SupplierDebitNoteStatus::Settled);
    // Too much, or a note that is settled: refused.
    assert!(refund(t.state(), note.id, &payment(22), 100, "k2").await.is_err());
}

/// A debit balance of AED 315.00 and an `approved` bill #2 of AED 2,000.00
/// from the same supplier and entity, no tax credit note yet: setting off
/// gives bill #2 `partially_paid`, balance AED 1,685.00; the note stays
/// `issued` and the accountant has the task. Recording the tax credit note
/// settles it.
#[renox::test]
async fn inv14_a_set_off_pays_the_next_bill_and_the_tax_credit_note_settles() {
    let t = boot().await;
    let (_, note) = on_paid_bill(&t, "INV-9").await;
    let line = BillLineDraft::new("Freight", 200_000, VatCategory::ZeroRated);
    let draft = BillDraft::new(BLUE_DUNES, "INV-10", day(0)).line(line);
    let second = supplier_bill::create(t.db(), &draft, Some(A)).await.unwrap();
    // A draft bill takes no set off.
    let early = refused(set_off(t.state(), By::System(Some(B)), note.id, second.id, 31_500).await);
    assert!(early.unwrap().contains("approved"));
    submit(t.state(), By::System(Some(A)), second.id).await.unwrap();
    let second = approve(t.state(), By::System(Some(B)), second.id, "").await.unwrap();
    assert_eq!(second.gross_total, 200_000);

    let after = set_off(t.state(), By::System(Some(B)), note.id, second.id, 31_500)
        .await
        .unwrap();
    assert_eq!(after.debit_balance, 0);
    assert_eq!(after.status, SupplierDebitNoteStatus::Issued);
    let bill2 = bill_now(&t, &second).await;
    assert_eq!(bill2.status, SupplierBillStatus::PartiallyPaid);
    assert_eq!(bill2.balance, 168_500);
    let moves = DebitNoteMovement::of_note(t.db(), note.id).await.unwrap();
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].kind, DebitMovementKind::SetOff);
    assert_eq!(moves[0].target(), Some(second.subject()));
    assert_eq!(open_tasks(t.db(), &note).await.unwrap().len(), 1);
    assert!(t.debit_settled.all().is_empty());

    let settled = record_tax_credit_note(t.state(), By::System(Some(B)), note.id, "TCN-3", day(0), None)
        .await
        .unwrap();
    assert_eq!(settled.status, SupplierDebitNoteStatus::Settled);
    assert!(open_tasks(t.db(), &note).await.unwrap().is_empty());
}

/// Setting off on a bill of another supplier or another receiving entity is
/// refused, as is more than the debit balance or the bill's balance.
#[renox::test]
async fn inv14_a_set_off_is_for_the_same_supplier_and_entity_within_both_balances() {
    let t = boot().await;
    let (_, note) = on_paid_bill(&t, "INV-9").await;
    let other_supplier = bill_of(&t, QUIET_SANDS, "Q-1", 10, None).await.0;
    let facilities = t.entity("PGFZ").await;
    let other_entity = bill_of(&t, BLUE_DUNES, "INV-11", 10, Some(facilities.id)).await.0;
    let small = bill_of(&t, BLUE_DUNES, "INV-12", 1, None).await.0;
    let big = bill_of(&t, BLUE_DUNES, "INV-13", 10, None).await.0;
    for bill in [&other_supplier, &other_entity] {
        let message = refused(set_off(t.state(), By::System(Some(B)), note.id, bill.id, 1_000).await);
        assert!(message.unwrap().contains("same supplier"), "{}", bill.id);
        assert_eq!(bill_now(&t, bill).await.balance, 105_000);
    }
    // Above the bill's balance (AED 105.00), the debit balance, or nothing.
    for (bill, amount) in [(&small, 10_501), (&big, 31_501), (&big, 0), (&big, -5)] {
        let message = refused(set_off(t.state(), By::System(Some(B)), note.id, bill.id, amount).await);
        assert!(message.unwrap().contains("more than 0"), "{amount}");
    }
    assert_eq!(reload(&t, &note).await.debit_balance, 31_500);
    // Part of it, twice.
    set_off(t.state(), By::System(Some(B)), note.id, big.id, 10_000).await.unwrap();
    let rest = set_off(t.state(), By::System(Some(B)), note.id, big.id, 21_500).await.unwrap();
    assert_eq!(rest.debit_balance, 0);
    assert_eq!(bill_now(&t, &big).await.balance, 105_000 - 31_500);
}

/// An `issued` debit note on an `approved` bill (the balance already reduced
/// at issue): recording the tax credit note gives `settled`.
#[renox::test]
async fn inv14_a_note_on_an_unpaid_bill_settles_when_the_tax_credit_note_is_recorded() {
    let t = boot().await;
    let (bill, line) = bill_of(&t, BLUE_DUNES, "INV-9", 10, None).await;
    let note = issued_note(&t, &bill, &line).await;
    assert_eq!((note.applied_to_bill, note.debit_balance), (31_500, 0));
    assert_eq!(reload(&t, &note).await.status, SupplierDebitNoteStatus::Issued);
    let settled = record_tax_credit_note(t.state(), By::System(Some(B)), note.id, "TCN-9", day(0), None)
        .await
        .unwrap();
    assert_eq!(settled.status, SupplierDebitNoteStatus::Settled);
    assert_eq!(t.debit_settled.all().len(), 1);
    // A blank number records nothing.
    let (bill, line) = bill_of(&t, BLUE_DUNES, "INV-10", 10, None).await;
    let other = issued_note(&t, &bill, &line).await;
    let blank = record_tax_credit_note(t.state(), By::System(Some(B)), other.id, "  ", day(0), None).await;
    assert!(refused(blank).is_some());
}

/// With the setting that accepts the debit note alone, a note applied in
/// full settles by itself, at issue or at the refund, and no task is opened.
#[renox::test]
async fn inv14_the_setting_can_accept_the_debit_note_without_a_tax_credit_note() {
    let t = boot_with(WlInvoicing::new().tax_credit_note_required(false)).await;
    let (bill, line) = bill_of(&t, BLUE_DUNES, "INV-9", 10, None).await;
    let note = issued_note(&t, &bill, &line).await;
    assert_eq!(reload(&t, &note).await.status, SupplierDebitNoteStatus::Settled);
    assert!(open_tasks(t.db(), &note).await.unwrap().is_empty());
    assert_eq!(t.debit_settled.all().len(), 1);

    let (paid, line) = bill_of(&t, BLUE_DUNES, "INV-10", 10, None).await;
    pay(t.state(), &paid, &payment(9), 105_000, None).await.unwrap();
    let owed = issued_note(&t, &paid, &line).await;
    assert_eq!(reload(&t, &owed).await.status, SupplierDebitNoteStatus::Issued);
    refund(t.state(), owed.id, &payment(30), 31_500, "k").await.unwrap();
    assert_eq!(reload(&t, &owed).await.status, SupplierDebitNoteStatus::Settled);
}

/// The input VAT a note takes back counts on the tax credit note's date when
/// the setting says so.
#[renox::test]
async fn inv14_input_vat_can_wait_for_the_tax_credit_note() {
    let t = boot_with(WlInvoicing::new().debit_note_vat_timing(InputVatTiming::OnTaxCreditNote)).await;
    let (bill, line) = bill_of(&t, BLUE_DUNES, "INV-9", 10, None).await;
    let entity = bill.legal_entity_id.unwrap();
    let note = issued_note(&t, &bill, &line).await;
    let config = t.config();
    let reduction = || async {
        debit_note_vat::reduction(t.db(), &config, entity, day(-30), day(30))
            .await
            .unwrap()
    };
    assert_eq!(reduction().await, 0);
    record_tax_credit_note(t.state(), By::System(Some(B)), note.id, "TCN-9", day(0), None)
        .await
        .unwrap();
    assert_eq!(reduction().await, 1_500);
}

/// An `issued` debit note and the return cancelled: `cancel` restores the
/// bill balance, puts AED 300.00 of cost back, takes the AED 15.00 of input
/// VAT back off the reduction, tells the supplier and emits the event. The
/// number stays.
#[renox::test]
async fn inv14_cancel_restores_the_bill_the_cost_and_the_input_vat() {
    let (t, ledger) = boot_with_costs().await;
    let (bill, line) = bill_of(&t, BLUE_DUNES, "INV-9", 10, None).await;
    let entity = bill.legal_entity_id.unwrap();
    let note = issued_note(&t, &bill, &line).await;
    assert_eq!(bill_now(&t, &bill).await.balance, 73_500);
    let config = t.config();
    let reduction = || async {
        debit_note_vat::reduction(t.db(), &config, entity, day(-30), day(30))
            .await
            .unwrap()
    };
    assert_eq!(reduction().await, 1_500);

    // A reason is needed.
    assert!(refused(cancel(t.state(), By::System(Some(B)), note.id, "  ").await).is_some());
    assert_eq!(reload(&t, &note).await.status, SupplierDebitNoteStatus::Issued);
    assert_eq!(bill_now(&t, &bill).await.balance, 73_500);

    let cancelled = cancel(t.state(), By::System(Some(B)), note.id, "The return was cancelled")
        .await
        .unwrap();
    assert_eq!(cancelled.status, SupplierDebitNoteStatus::Cancelled);
    assert_eq!(cancelled.number, note.number);
    assert_eq!(cancelled.cancel_reason.as_deref(), Some("The return was cancelled"));
    assert_eq!(cancelled.cancelled_by, Some(B));
    assert_eq!(cancelled.debit_balance, 0);
    assert!(open_tasks(t.db(), &note).await.unwrap().is_empty());

    let after = bill_now(&t, &bill).await;
    assert_eq!(after.status, SupplierBillStatus::Approved);
    assert_eq!((after.allocated_total, after.balance), (0, 105_000));
    let reopened = t.bill_reopened.all();
    assert_eq!(reopened.len(), 1);
    assert_eq!(reopened[0].new_status, KernelBillStatus::Approved);
    assert_eq!(reopened[0].amount, 31_500);
    assert_eq!(reopened[0].source, note.subject());

    let restored = ledger.restorations();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].lines.len(), 1);
    assert_eq!(restored[0].lines[0].cost_object, unit());
    assert_eq!(restored[0].lines[0].amount_aed, 30_000);
    assert_eq!(reduction().await, 0);

    let told = t.notices.of_type("invoicing.supplier_debit_note_cancelled");
    assert_eq!(told.len(), 1);
    assert!(told[0]
        .recipients
        .iter()
        .any(|r| matches!(r, NotifyRecipient::Party { id } if *id == BLUE_DUNES)));
    let events = t.debit_cancelled.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].debit_note.0, note.id);
    assert_eq!(events[0].bill.map(|b| b.0), Some(bill.id));
    assert_eq!(events[0].amount, 31_500);
    assert_eq!(events[0].reason, "The return was cancelled");

    // A cancelled note is final.
    assert!(refused(cancel(t.state(), By::System(Some(B)), note.id, "again").await).is_some());
    assert!(refused(settle(t.state(), By::System(Some(B)), note.id, None).await).is_some());
}

/// A bill of AED 10,500.00 paid AED 10,185.00 and set `paid` by a debit note
/// of AED 315.00: `cancel` runs the bill's `unallocate`, the bill is
/// `partially_paid` with a balance of AED 315.00 and `SupplierBillReopened`
/// is emitted.
#[renox::test]
async fn inv14_cancel_reopens_a_bill_the_note_had_set_to_paid() {
    let t = boot().await;
    let (bill, line) = bill_of(&t, BLUE_DUNES, "INV-9", 100, None).await;
    assert_eq!(bill.gross_total, 1_050_000);
    pay(t.state(), &bill, &payment(9), 1_018_500, None).await.unwrap();
    let note = issued_note(&t, &bill, &line).await;
    assert_eq!(bill_now(&t, &bill).await.status, SupplierBillStatus::Paid);

    cancel(t.state(), By::System(Some(B)), note.id, "Supplier rejected the claim")
        .await
        .unwrap();
    let after = bill_now(&t, &bill).await;
    assert_eq!(after.status, SupplierBillStatus::PartiallyPaid);
    assert_eq!((after.allocated_total, after.balance), (1_018_500, 31_500));
    let reopened = t.bill_reopened.all();
    assert_eq!(reopened.len(), 1);
    assert_eq!(reopened[0].new_status, KernelBillStatus::PartiallyPaid);
    assert_eq!(reopened[0].balance, 31_500);
}

/// Cancelling a note that opened a debit balance closes it; a set off on a
/// later bill is given back too; a draft is cancelled without effects; and a
/// note the supplier paid back on is not cancelled.
#[renox::test]
async fn inv14_cancel_closes_the_debit_balance_and_gives_back_a_set_off() {
    let (t, ledger) = boot_with_costs().await;
    let (_, note) = on_paid_bill(&t, "INV-9").await;
    let big = bill_of(&t, BLUE_DUNES, "INV-13", 10, None).await.0;
    set_off(t.state(), By::System(Some(B)), note.id, big.id, 10_000).await.unwrap();
    assert_eq!(bill_now(&t, &big).await.balance, 95_000);
    assert_eq!(
        supplier_debit_notes(t.state())
            .debit_balance(t.db(), BLUE_DUNES, note.legal_entity_id, "AED")
            .await
            .unwrap(),
        21_500
    );

    // Through the kernel contract, as wl-inventory's return cancel would.
    supplier_debit_notes(t.state())
        .cancel(
            t.state(),
            wl_kernel::invoicing::SupplierDebitNoteId(note.id),
            "Return cancelled",
            Some(B),
        )
        .await
        .unwrap();
    let after = reload(&t, &note).await;
    assert_eq!(after.status, SupplierDebitNoteStatus::Cancelled);
    assert_eq!(after.debit_balance, 0);
    assert_eq!(
        supplier_debit_notes(t.state())
            .debit_balance(t.db(), BLUE_DUNES, note.legal_entity_id, "AED")
            .await
            .unwrap(),
        0
    );
    // The bill the note was set off on is open again.
    let reopened = bill_now(&t, &big).await;
    assert_eq!(reopened.status, SupplierBillStatus::Approved);
    assert_eq!(reopened.balance, 105_000);
    assert_eq!(ledger.restorations().len(), 1);
    assert_eq!(t.debit_cancelled.all().len(), 1);
}

#[renox::test]
async fn inv14_a_draft_is_cancelled_without_effects_and_a_refunded_note_is_not() {
    let (t, ledger) = boot_with_costs().await;
    let (bill, line) = bill_of(&t, BLUE_DUNES, "INV-9", 10, None).await;
    let draft = debit_notes::draft_manual(
        t.db(),
        &bill,
        "quality",
        "",
        &[DebitLine::new(line.id, 3_000, 10_000)],
        Some(A),
    )
    .await
    .unwrap();
    let cancelled = cancel(t.state(), By::System(Some(B)), draft.id, "Not needed")
        .await
        .unwrap();
    assert_eq!(cancelled.status, SupplierDebitNoteStatus::Cancelled);
    assert_eq!(cancelled.number, None);
    assert_eq!(bill_now(&t, &bill).await.balance, 105_000);
    assert!(ledger.restorations().is_empty());
    assert!(t.notices.of_type("invoicing.supplier_debit_note_cancelled").is_empty());
    assert_eq!(SupplierDebitNote::find(t.db(), draft.id).await.unwrap().map(|n| n.id), Some(draft.id));

    let (_, owed) = on_paid_bill(&t, "INV-20").await;
    refund(t.state(), owed.id, &payment(40), 1_000, "k").await.unwrap();
    let message = refused(cancel(t.state(), By::System(Some(B)), owed.id, "Too late").await);
    assert!(message.unwrap().contains("paid part"));
    assert_eq!(reload(&t, &owed).await.status, SupplierDebitNoteStatus::Issued);
}
