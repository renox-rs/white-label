//! The kernel contract and notices of the Supplier bill (wl-invoicing §6, §7,
//! #74 part 3): `Payables` through `supplier_bills(state)`, the events
//! after a commit and the accountant notice of a held possible duplicate.

mod support;

use renox::chrono::NaiveDate;
use wl_invoicing::bill_machine::{approve, void};
use wl_invoicing::bill_submit::submit;
use wl_invoicing::credit_notes::By;
use wl_invoicing::supplier_bill::{self, BillDraft};
use wl_invoicing::{BillLineDraft, SupplierBill};
use wl_kernel::PartyId;
use wl_kernel::invoicing::{
    SupplierBillId, SupplierBillLineId, SupplierBillStatus as KernelStatus, VatCategory,
    supplier_bills,
};

use support::{BLUE_DUNES, Booted, boot};

const A: i64 = 7;
const B: i64 = 8;

const NOTICE: &str = "invoicing.supplier_bill_possible_duplicate";

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

async fn draft(t: &Booted, supplier: PartyId, number: &str) -> SupplierBill {
    let draft = BillDraft::new(supplier, number, day(5)).line(BillLineDraft::new(
        "Pump repair",
        100_000,
        VatCategory::Standard,
    ));
    supplier_bill::create(t.db(), &draft, Some(A))
        .await
        .unwrap()
}

#[renox::test]
async fn the_provider_shows_a_line_with_its_amount_and_status() {
    let t = boot().await;
    let bill = draft(&t, BLUE_DUNES, "INV-1").await;
    let line = supplier_bill::lines_of(t.db(), bill.id).await.unwrap()[0].clone();
    let provider = supplier_bills(t.state());
    let shown = provider
        .line(t.db(), SupplierBillLineId(line.id))
        .await
        .unwrap()
        .expect("the line");
    assert_eq!(shown.bill, SupplierBillId(bill.id));
    assert_eq!(shown.bill_number, "INV-1");
    assert_eq!(shown.supplier, BLUE_DUNES);
    assert_eq!(shown.description, "Pump repair");
    assert_eq!(shown.amount_aed, 100_000);
    assert_eq!(shown.bill_status, KernelStatus::Draft);

    submit(t.state(), By::System(Some(A)), bill.id)
        .await
        .unwrap();
    let shown = provider
        .line(t.db(), SupplierBillLineId(line.id))
        .await
        .unwrap()
        .expect("the line");
    assert_eq!(shown.bill_status, KernelStatus::Matched);
    assert!(
        provider
            .line(t.db(), SupplierBillLineId(9_999))
            .await
            .unwrap()
            .is_none()
    );
}

#[renox::test]
async fn approving_and_voiding_emit_once_after_the_commit() {
    let t = boot().await;
    let bill = draft(&t, BLUE_DUNES, "INV-1").await;
    submit(t.state(), By::System(Some(A)), bill.id)
        .await
        .unwrap();
    assert!(t.bill_approved.all().is_empty());
    approve(t.state(), By::System(Some(B)), bill.id, "")
        .await
        .unwrap();
    let approved = t.bill_approved.all();
    assert_eq!(approved.len(), 1);
    assert_eq!(approved[0].bill, SupplierBillId(bill.id));
    assert_eq!(approved[0].supplier, BLUE_DUNES);
    assert_eq!(approved[0].amount, 105_000);
    assert!(t.bill_voided.all().is_empty());

    let other = draft(&t, BLUE_DUNES, "INV-2").await;
    // A refused void emits nothing.
    assert!(
        void(t.state(), By::System(Some(A)), other.id, " ")
            .await
            .is_err()
    );
    assert!(t.bill_voided.all().is_empty());
    void(t.state(), By::System(Some(A)), other.id, "wrong supplier")
        .await
        .unwrap();
    let voided = t.bill_voided.all();
    assert_eq!(voided.len(), 1);
    assert_eq!(voided[0].bill, SupplierBillId(other.id));
    assert_eq!(voided[0].reason, "wrong supplier");
    assert_eq!(t.bill_approved.all().len(), 1);
}

#[renox::test]
async fn a_held_bill_tells_each_accountant_once_and_a_clean_one_nobody() {
    let t = boot().await;
    let amal = t.accountant("Amal").await;
    let bilal = t.accountant("Bilal").await;
    let first = draft(&t, BLUE_DUNES, "INV-77").await;
    submit(t.state(), By::System(Some(A)), first.id)
        .await
        .unwrap();
    assert!(t.notices.of_type(NOTICE).is_empty());

    let second = draft(&t, BLUE_DUNES, "inv-77 ").await;
    submit(t.state(), By::System(Some(A)), second.id)
        .await
        .unwrap();
    let told = t.notices.of_type(NOTICE);
    assert_eq!(told.len(), 1);
    let mut ids: Vec<i64> = told[0]
        .recipients
        .iter()
        .map(|r| match r {
            wl_kernel::notify::NotifyRecipient::User { id } => *id,
            other => panic!("not a user: {other:?}"),
        })
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, [amal.id, bilal.id]);
    assert_eq!(told[0].data["number"], "inv-77");
    assert_eq!(told[0].data["earlier_number"], "INV-77");
    assert_eq!(told[0].data["supplier"], "Blue Dunes LLC");
    assert_eq!(
        told[0].data["link"],
        format!("/supplier-bills/{}", second.id)
    );
}
