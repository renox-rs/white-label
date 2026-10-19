//! The money side of the supplier bill (wl-invoicing §4 `pay`, #75 part 2):
//! `pay` and `pay_in_full` write the allocation rows and the balance in one
//! transaction, are refused for a wrong amount or state, and are never
//! offered to a user.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::bill_machine::approve;
use wl_invoicing::bill_pay::pay;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::credit_notes::By;
use wl_invoicing::supplier_bill::{self, BillDraft, SupplierBillStatus};
use wl_invoicing::{BillAllocation, BillLineDraft, SupplierBill};
use wl_kernel::invoicing::VatCategory;
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

const SUPPLIER: PartyId = PartyId(2);
/// AED 10,000.00 plus 5% VAT.
const GROSS: i64 = 1_050_000;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

fn payment(id: i64) -> Subject {
    Subject::new("payments.payment", id)
}

async fn approved(t: &Booted) -> SupplierBill {
    let draft = BillDraft::new(SUPPLIER, "INV-1", day(5)).line(BillLineDraft::new(
        "Pump repair",
        1_000_000,
        VatCategory::Standard,
    ));
    let bill = supplier_bill::create(t.db(), &draft, Some(7))
        .await
        .unwrap();
    submit(t.state(), By::System(Some(7)), bill.id)
        .await
        .unwrap();
    approve(t.state(), By::System(Some(8)), bill.id, "")
        .await
        .unwrap()
}

async fn reload(t: &Booted, bill: &SupplierBill) -> SupplierBill {
    SupplierBill::find_or_404(t.db(), bill.id).await.unwrap()
}

#[renox::test]
async fn a_new_bill_has_its_whole_gross_open() {
    let t = boot().await;
    let bill = approved(&t).await;
    assert_eq!(bill.gross_total, GROSS);
    assert_eq!((bill.allocated_total, bill.balance), (0, GROSS));
}

#[renox::test]
async fn a_part_payment_leaves_a_balance_and_the_rest_settles_it() {
    let t = boot().await;
    let bill = approved(&t).await;
    pay(t.state(), &bill, &payment(1), 600_000, Some(9))
        .await
        .unwrap();
    let after = reload(&t, &bill).await;
    assert_eq!(after.status, SupplierBillStatus::PartiallyPaid);
    assert_eq!((after.allocated_total, after.balance), (600_000, 450_000));

    pay(t.state(), &after, &payment(2), 450_000, None)
        .await
        .unwrap();
    let paid = reload(&t, &bill).await;
    assert_eq!(paid.status, SupplierBillStatus::Paid);
    assert_eq!((paid.allocated_total, paid.balance), (GROSS, 0));

    let rows = BillAllocation::of_bill(t.db(), bill.id).await.unwrap();
    let seen: Vec<_> = rows
        .iter()
        .map(|r| (r.source(), r.amount, r.created_by, r.is_active()))
        .collect();
    assert_eq!(
        seen,
        [
            (payment(1), 600_000, Some(9), true),
            (payment(2), 450_000, None, true)
        ]
    );
}

#[renox::test]
async fn paying_in_full_in_one_go_marks_the_bill_paid() {
    let t = boot().await;
    let bill = approved(&t).await;
    pay(t.state(), &bill, &payment(1), GROSS, None)
        .await
        .unwrap();
    let paid = reload(&t, &bill).await;
    assert_eq!(paid.status, SupplierBillStatus::Paid);
    assert_eq!(paid.balance, 0);
}

#[renox::test]
async fn a_wrong_amount_or_state_is_refused_and_nothing_is_written() {
    let t = boot().await;
    let bill = approved(&t).await;
    for amount in [0, -5, GROSS + 1] {
        assert!(
            pay(t.state(), &bill, &payment(1), amount, None)
                .await
                .is_err()
        );
    }
    let same = reload(&t, &bill).await;
    assert_eq!(same.status, SupplierBillStatus::Approved);
    assert_eq!((same.allocated_total, same.balance), (0, GROSS));
    assert!(
        BillAllocation::of_bill(t.db(), bill.id)
            .await
            .unwrap()
            .is_empty()
    );

    // A bill that is not approved yet, and a paid one, take no money.
    let draft = BillDraft::new(SUPPLIER, "INV-2", day(6)).line(BillLineDraft::new(
        "Filter",
        1_000,
        VatCategory::Standard,
    ));
    let other = supplier_bill::create(t.db(), &draft, Some(7))
        .await
        .unwrap();
    assert!(
        pay(t.state(), &other, &payment(1), 100, None)
            .await
            .is_err()
    );
    pay(t.state(), &bill, &payment(1), GROSS, None)
        .await
        .unwrap();
    let paid = reload(&t, &bill).await;
    assert!(pay(t.state(), &paid, &payment(2), 1, None).await.is_err());
}

#[renox::test]
async fn a_stale_bill_cannot_be_paid_twice() {
    let t = boot().await;
    let bill = approved(&t).await;
    pay(t.state(), &bill, &payment(1), 600_000, None)
        .await
        .unwrap();
    // `bill` still shows the old balance: the compare-and-set refuses it.
    assert!(
        pay(t.state(), &bill, &payment(2), 600_000, None)
            .await
            .is_err()
    );
    let after = reload(&t, &bill).await;
    assert_eq!(after.balance, 450_000);
    assert_eq!(
        BillAllocation::of_bill(t.db(), bill.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[renox::test]
async fn pay_is_system_only_so_no_user_is_offered_it() {
    let machine = <SupplierBill as renox::state_machine::StateMachine>::machine();
    for name in ["pay", "pay_in_full"] {
        assert!(machine.find(name).expect(name).is_system());
    }
}
