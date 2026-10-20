//! Supplier bill lines coded to a landed cost (wl-invoicing §3 *Supplier
//! bill* landed cost, §4 `approve`, §5.4 step 4, #78): the line posts no cost
//! to a cost object, its amount goes to the landed cost with
//! `SupplierBillApproved`, and its VAT is recorded as input VAT like any other.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::bill_machine::approve;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::cash_forecast_load::payable;
use wl_invoicing::credit_notes::By;
use wl_invoicing::supplier_bill::{self, BillDraft};
use wl_invoicing::{BillLineDraft, SupplierBill, SupplierBillStatus};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{LandedCostLine, SupplierBillLineId, VatCategory};
use wl_kernel::Subject;

use support::{BLUE_DUNES, boot_with_landed};

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

fn lc_12() -> Subject {
    Subject::new("inventory.landed_cost", 12)
}

fn unit() -> Subject {
    Subject::new("property.unit", 4)
}

/// Goods of AED 5,000.00 (standard, VAT AED 250.00) coded to a cost object
/// and freight of AED 400.00 (standard, VAT AED 20.00) coded to LC-12.
fn bill_lines() -> (BillLineDraft, BillLineDraft) {
    (
        BillLineDraft::new("Goods", 500_000, VatCategory::Standard).cost_object(unit()),
        BillLineDraft::new("Freight", 40_000, VatCategory::Standard).landed_cost(lc_12()),
    )
}

/// On approval cost of AED 5,000.00 is posted and AED 0.00 for the freight;
/// input VAT of AED 270.00 is recorded for the bill's period; and
/// `SupplierBillApproved` carries LC-12 for the freight line.
#[renox::test]
async fn inv15_a_landed_cost_line_posts_no_cost_but_its_vat_counts() {
    let (t, ledger, desk) = boot_with_landed().await;
    let entity = t.entity("PGP").await;
    desk.add(lc_12(), entity.id);
    let (goods, freight) = bill_lines();
    let mut draft = BillDraft::new(BLUE_DUNES, "INV-9", day(0))
        .line(goods)
        .line(freight);
    draft.legal_entity_id = Some(entity.id);
    let bill = supplier_bill::create(t.db(), &draft, Some(A)).await.unwrap();
    let lines = supplier_bill::lines_of(t.db(), bill.id).await.unwrap();
    assert_eq!(lines[1].landed_cost(), Some(lc_12()));
    assert_eq!(lines[1].cost_object(), None);
    submit(t.state(), By::System(Some(A)), bill.id).await.unwrap();
    assert!(ledger.postings().is_empty());

    let approved = approve(t.state(), By::System(Some(B)), bill.id, "").await.unwrap();
    assert_eq!(approved.status, SupplierBillStatus::Approved);

    // Cost of the goods only.
    let posts = ledger.postings();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].amount_aed, 500_000);
    assert_eq!(posts[0].cost_object, unit());
    assert_eq!(posts[0].source, bill.subject());
    assert_eq!(posts[0].key, format!("invoicing.supplier_bill_line:{}", lines[0].id));
    assert!(posts.iter().all(|p| p.amount_aed != 40_000));

    // Input VAT of AED 270.00 (250.00 + 20.00) for the bill's period.
    let stored = SupplierBill::find_or_404(t.db(), bill.id).await.unwrap();
    assert_eq!(stored.vat_total, 27_000);
    let config = t.config();
    assert_eq!(
        payable(t.db(), &config, entity.id, day(-30), day(30)).await.unwrap(),
        -27_000
    );

    // The event names the cost object of the goods and LC-12 for the freight.
    let events = t.bill_approved.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].landed_costs.len(), 1);
    assert_eq!(
        events[0].landed_costs[0],
        LandedCostLine {
            line: SupplierBillLineId(lines[1].id),
            landed_cost: lc_12(),
            amount: 40_000
        }
    );
    assert_eq!(events[0].cost_objects.len(), 1);
    assert_eq!(events[0].cost_objects[0].cost_object, unit());
    assert_eq!(events[0].cost_objects[0].amount, 500_000);
    assert_eq!(events[0].amount, 567_000);
}

/// A line coded to a landed cost and to a cost object would count the cost
/// twice: refused when the bill is saved.
#[renox::test]
async fn inv15_a_line_cannot_have_a_landed_cost_and_a_cost_object() {
    let (t, _, _) = boot_with_landed().await;
    let line = BillLineDraft::new("Freight", 40_000, VatCategory::Standard)
        .landed_cost(lc_12())
        .cost_object(unit());
    let draft = BillDraft::new(BLUE_DUNES, "INV-9", day(0)).line(line);
    let refused = supplier_bill::create(t.db(), &draft, Some(A)).await;
    assert!(matches!(
        refused,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
}

/// A landed cost of another legal entity, or one that does not exist, is
/// refused at `submit`; the bill stays a draft.
#[renox::test]
async fn inv15_the_landed_cost_must_exist_and_belong_to_the_receiving_entity() {
    let (t, ledger, desk) = boot_with_landed().await;
    let own = t.entity("PGP").await;
    let other = t.entity("PGFZ").await;
    desk.add(lc_12(), other.id);
    let mut draft = BillDraft::new(BLUE_DUNES, "INV-9", day(0)).line(
        BillLineDraft::new("Freight", 40_000, VatCategory::Standard).landed_cost(lc_12()),
    );
    draft.legal_entity_id = Some(own.id);
    let bill = supplier_bill::create(t.db(), &draft, Some(A)).await.unwrap();
    let message = submit(t.state(), By::System(Some(A)), bill.id)
        .await
        .map(|_| ())
        .map_err(|e| e.into_error());
    match message {
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, text)) => {
            assert!(text.contains("another legal entity"), "{text}");
            assert!(text.contains("inventory.landed_cost:12"), "{text}");
        }
        other => panic!("not refused: {other:?}"),
    }
    let after = SupplierBill::find_or_404(t.db(), bill.id).await.unwrap();
    assert_eq!(after.status, SupplierBillStatus::Draft);

    // No such landed cost.
    let mut draft = BillDraft::new(BLUE_DUNES, "INV-10", day(0)).line(
        BillLineDraft::new("Duty", 10_000, VatCategory::Standard)
            .landed_cost(Subject::new("inventory.landed_cost", 99)),
    );
    draft.legal_entity_id = Some(own.id);
    let bill = supplier_bill::create(t.db(), &draft, Some(A)).await.unwrap();
    let refused = submit(t.state(), By::System(Some(A)), bill.id).await;
    assert!(refused.is_err());
    assert!(ledger.postings().is_empty());

    // The landed cost of the receiving entity is accepted.
    desk.add(Subject::new("inventory.landed_cost", 13), own.id);
    let mut draft = BillDraft::new(BLUE_DUNES, "INV-11", day(0)).line(
        BillLineDraft::new("Clearing", 10_000, VatCategory::Standard)
            .landed_cost(Subject::new("inventory.landed_cost", 13)),
    );
    draft.legal_entity_id = Some(own.id);
    let bill = supplier_bill::create(t.db(), &draft, Some(A)).await.unwrap();
    let matched = submit(t.state(), By::System(Some(A)), bill.id).await.unwrap();
    assert_eq!(matched.status, SupplierBillStatus::Matched);
}

/// Without a provider of landed costs a line coded to one is refused (none is
/// known).
#[renox::test]
async fn inv15_without_landed_costs_a_coded_line_is_refused() {
    let t = support::boot().await;
    let draft = BillDraft::new(BLUE_DUNES, "INV-9", day(0)).line(
        BillLineDraft::new("Freight", 40_000, VatCategory::Standard).landed_cost(lc_12()),
    );
    let bill = supplier_bill::create(t.db(), &draft, Some(A)).await.unwrap();
    assert!(submit(t.state(), By::System(Some(A)), bill.id).await.is_err());
}
