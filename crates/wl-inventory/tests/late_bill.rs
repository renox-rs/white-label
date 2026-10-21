//! A late supplier bill coded to a landed cost (wl-inventory §4 *Landed cost*,
//! §5.8 step 1, issue #229): the approval of the bill in wl-invoicing drafts
//! an adjustment of an allocated landed cost, one per bill, and tells the
//! purchaser. Nothing is posted by itself.

mod support;

use renox::prelude::*;
use wl_inventory::landed_adjust::{self, AdjustmentStatus, LandedCostAdjustment, shares_of};
use wl_inventory::landed_cost::{
    self, AllocationBasis, CostKind, CostLine, NewCostLine, NewLandedCost,
};
use wl_inventory::{DraftLine, LandedCost, LandedCostStatus};
use wl_kernel::Subject;
use wl_kernel::invoicing::{
    LandedCostLine, SupplierBillApproved, SupplierBillId, SupplierBillLineId,
};

use support::{Booted, SUPPLIER, boot, boot_with_parties};

/// An allocated landed cost (AED 2,000 over a receipt of 100 units) and a
/// procurement user who is told.
async fn allocated(t: &Booted) -> LandedCost {
    let store = t.location("STORE-1", None).await;
    let item = t.item("PIPE").await;
    let (order, line) = t.order(&store, &item, 100_000, 1_000).await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![DraftLine::on(line.id, 100_000, 100_000)],
        )
        .await;
    let clerk = t.storekeeper("Sam").await;
    t.post_as(&clerk, &receipt).await.assert_ok();
    let landed = landed_cost::create_draft(
        t.db(),
        None,
        NewLandedCost {
            basis: AllocationBasis::Value,
            receipts: vec![receipt.id],
            lines: vec![NewCostLine::of(CostKind::Freight, 200_000)],
        },
    )
    .await
    .unwrap();
    let buyer = t.procurement("Pia").await;
    t.allocate_as(&buyer, &landed, &[]).await.assert_ok();
    t.landed_now(&landed).await
}

/// A bill approved with lines `(bill line, amount)` coded to `landed`.
fn approved(bill: i64, landed: &LandedCost, lines: &[(i64, i64)]) -> SupplierBillApproved {
    SupplierBillApproved {
        bill: SupplierBillId(bill),
        supplier: SUPPLIER,
        amount: lines.iter().map(|(_, amount)| amount).sum(),
        cost_objects: Vec::new(),
        landed_costs: lines
            .iter()
            .map(|(line, amount)| LandedCostLine {
                line: SupplierBillLineId(*line),
                landed_cost: landed.subject(),
                amount: *amount,
            })
            .collect(),
    }
}

async fn drafts(t: &Booted, landed: &LandedCost) -> Vec<LandedCostAdjustment> {
    LandedCostAdjustment::query()
        .where_(LandedCostAdjustment::LANDED_COST_ID.eq(landed.id))
        .order_by(LandedCostAdjustment::ID)
        .get(t.db())
        .await
        .unwrap()
}

fn told(t: &Booted) -> Vec<wl_kernel::notify::NotifyRequest> {
    t.notices
        .all()
        .into_iter()
        .filter(|n| n.type_key == "inventory.landed_cost_adjustment_drafted")
        .collect()
}

/// Done when 1: an allocated landed cost, a freight bill of AED 800 coded to
/// it is approved: a draft adjustment with that line and the bill reference
/// exists and the purchaser is notified.
#[renox::test]
async fn a_late_freight_bill_drafts_an_adjustment_and_tells_the_purchaser() {
    let t = boot().await;
    let landed = allocated(&t).await;
    let buyer = t.procurement("Bea").await;
    t.bills
        .add(31, 311, "FWD-880", "Sea freight, Jebel Ali", 80_000);

    t.state()
        .emit(approved(31, &landed, &[(311, 80_000)]))
        .await
        .unwrap();

    let found = drafts(&t, &landed).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].status, AdjustmentStatus::Draft);
    assert_eq!(found[0].bill_id, Some(31));
    assert_eq!(found[0].bill_reference, "FWD-880");
    assert_eq!(found[0].total, 80_000);
    let lines = landed_adjust::lines_of(t.db(), found[0].id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].amount, 80_000);
    assert_eq!(lines[0].kind, CostKind::Freight);
    assert_eq!(lines[0].bill_reference.as_deref(), Some("FWD-880"));
    assert_eq!(lines[0].bill_line_id, Some(311));
    // The purchaser is told.
    let notices = told(&t);
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0].recipients.iter().any(
            |r| matches!(r, wl_kernel::notify::NotifyRecipient::User { id } if *id == buyer.id)
        )
    );
    // Nothing is posted by itself.
    assert!(
        shares_of(t.db(), landed.id, Some(found[0].id))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(t.adjusted.all().is_empty());
}

/// Done when 2: the same event twice creates one draft.
#[renox::test]
async fn the_same_bill_twice_makes_one_draft() {
    let t = boot().await;
    let landed = allocated(&t).await;
    let event = approved(31, &landed, &[(311, 80_000)]);

    t.state().emit(event.clone()).await.unwrap();
    t.state().emit(event).await.unwrap();

    assert_eq!(drafts(&t, &landed).await.len(), 1);
    assert_eq!(told(&t).len(), 1);
}

/// Done when 3: with that draft still unposted, a duty bill of AED 300 coded
/// to the same landed cost makes a second draft with only the AED 300 line;
/// the first is unchanged.
#[renox::test]
async fn another_bill_makes_its_own_draft() {
    let t = boot().await;
    let landed = allocated(&t).await;
    t.bills.add(31, 311, "FWD-880", "Sea freight", 80_000);
    t.bills.add(32, 321, "CUS-300", "Customs duty", 30_000);
    t.state()
        .emit(approved(31, &landed, &[(311, 80_000)]))
        .await
        .unwrap();
    let first = drafts(&t, &landed).await.remove(0);

    t.state()
        .emit(approved(32, &landed, &[(321, 30_000)]))
        .await
        .unwrap();

    let found = drafts(&t, &landed).await;
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].id, first.id);
    assert_eq!(found[0].total, 80_000);
    assert_eq!(found[1].bill_id, Some(32));
    assert_eq!(found[1].total, 30_000);
    let lines = landed_adjust::lines_of(t.db(), found[1].id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].kind, CostKind::Duty);
    assert_eq!(lines[0].amount, 30_000);
    // The first is as it was.
    assert_eq!(
        landed_adjust::lines_of(t.db(), first.id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(told(&t).len(), 2);
}

/// Several lines of one bill make one adjustment with several cost lines; a
/// landed cost still in `draft` takes the bill line as a cost line and makes
/// no adjustment; one it does not know is left alone.
#[renox::test]
async fn several_lines_one_draft_and_a_draft_landed_cost_takes_the_line() {
    let t = boot().await;
    let landed = allocated(&t).await;
    t.bills.add(33, 331, "CLR-1", "Port clearing", 20_000);
    t.bills.add(33, 332, "CLR-1", "Marine insurance", 10_000);

    t.state()
        .emit(approved(33, &landed, &[(331, 20_000), (332, 10_000)]))
        .await
        .unwrap();

    let found = drafts(&t, &landed).await;
    assert_eq!(found.len(), 1);
    let lines = landed_adjust::lines_of(t.db(), found[0].id).await.unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].kind, CostKind::Clearing);
    assert_eq!(lines[1].kind, CostKind::Insurance);
    assert_eq!(found[0].total, 30_000);

    // A landed cost that is still a draft: the line joins its cost lines.
    let receipts = vec![
        wl_inventory::landed_cost::LandedCostReceipt::query()
            .where_(wl_inventory::landed_cost::LandedCostReceipt::LANDED_COST_ID.eq(landed.id))
            .first(t.db())
            .await
            .unwrap()
            .expect("a receipt")
            .goods_receipt_id,
    ];
    let open = landed_cost::create_draft(
        t.db(),
        None,
        NewLandedCost {
            basis: AllocationBasis::Value,
            receipts,
            lines: vec![NewCostLine::of(CostKind::Freight, 5_000)],
        },
    )
    .await
    .unwrap();
    assert_eq!(open.status, LandedCostStatus::Draft);
    let event = approved(34, &open, &[(341, 7_000)]);
    t.state().emit(event.clone()).await.unwrap();
    t.state().emit(event).await.unwrap();
    assert!(drafts(&t, &open).await.is_empty());
    let costs = CostLine::query()
        .where_(CostLine::LANDED_COST_ID.eq(open.id))
        .order_by(CostLine::POSITION)
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(costs.len(), 2);
    assert_eq!(costs[1].amount, 7_000);
    assert_eq!(costs[1].bill_line_id, Some(341));
    assert_eq!(costs[1].position, 2);

    // A landed cost nobody knows is left alone.
    let stranger = LandedCost {
        id: 9_999,
        ..LandedCost::default()
    };
    t.state()
        .emit(approved(35, &stranger, &[(351, 1_000)]))
        .await
        .unwrap();
    assert_eq!(
        stranger.subject(),
        Subject::new("inventory.landed_cost", 9_999)
    );
}

/// The draft is posted by the purchaser like any other adjustment.
#[renox::test]
async fn the_purchaser_posts_the_drafted_adjustment() {
    let t = boot().await;
    let landed = allocated(&t).await;
    let buyer = t.procurement("Bea").await;
    t.state()
        .emit(approved(31, &landed, &[(311, 80_000)]))
        .await
        .unwrap();
    let draft = drafts(&t, &landed).await.remove(0);

    t.post_adjustment_as(&buyer, &landed, &draft)
        .await
        .assert_ok();

    assert_eq!(
        t.adjustment_now(&draft).await.status,
        AdjustmentStatus::Posted
    );
    assert_eq!(t.adjusted.all().len(), 1);
}

/// Done when 1 to 3 with the real wl-invoicing: a supplier bill with a line
/// coded to the allocated landed cost is submitted and approved, and the
/// event its `Payables` and `bill_events` produce drafts the adjustment (one
/// draft, the bill line and the reference on it, the purchaser told). A
/// second approved bill (duty) drafts a second adjustment and leaves the
/// first as it was.
#[renox::test]
async fn real_approved_bills_draft_one_adjustment_each() {
    use renox::chrono::Days;
    use wl_invoicing::bill_machine::approve;
    use wl_invoicing::bill_submit::submit;
    use wl_invoicing::credit_notes::By;
    use wl_invoicing::supplier_bill::{self, BillDraft};
    use wl_invoicing::{BillLineDraft, SupplierBillStatus};
    use wl_kernel::documents::today;
    use wl_kernel::invoicing::VatCategory;

    let t = boot_with_parties().await;
    let landed = allocated(&t).await;
    let buyer = t.procurement("Bea").await;
    let supplier = wl_kernel::PartyId(
        wl_parties::party::Party::query()
            .where_(wl_parties::party::Party::LEGAL_NAME_EN.eq("Gulf Build Materials LLC"))
            .first(t.db())
            .await
            .unwrap()
            .expect("the demo vendor")
            .id,
    );
    let entity = wl_kernel::LegalEntity::find(t.db(), landed.legal_entity_id)
        .await
        .unwrap()
        .expect("the entity of the landed cost");

    // Freight of AED 800.00 coded to the landed cost.
    let mut freight = BillDraft::new(supplier, "FWD-880", today()).line(
        BillLineDraft::new("Sea freight, Jebel Ali", 80_000, VatCategory::Standard)
            .landed_cost(landed.subject()),
    );
    freight.legal_entity_id = Some(entity.id);
    freight.due_date = Some(today() + Days::new(30));
    let bill = supplier_bill::create(t.db(), &freight, None).await.unwrap();
    submit(t.state(), By::System(None), bill.id).await.unwrap();
    let approved = approve(t.state(), By::System(None), bill.id, "")
        .await
        .unwrap();
    assert_eq!(approved.status, SupplierBillStatus::Approved);

    let found = drafts(&t, &landed).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].status, AdjustmentStatus::Draft);
    assert_eq!(found[0].bill_id, Some(bill.id));
    assert_eq!(found[0].bill_reference, "FWD-880");
    assert_eq!(found[0].total, 80_000);
    let bill_lines = supplier_bill::lines_of(t.db(), bill.id).await.unwrap();
    let lines = landed_adjust::lines_of(t.db(), found[0].id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].amount, 80_000);
    assert_eq!(lines[0].bill_line_id, Some(bill_lines[0].id));
    let notices = told(&t);
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0].recipients.iter().any(
            |r| matches!(r, wl_kernel::notify::NotifyRecipient::User { id } if *id == buyer.id)
        )
    );
    assert!(t.adjusted.all().is_empty());

    // A second bill, customs duty of AED 300.00, with the first still unposted.
    let mut duty = BillDraft::new(supplier, "CUS-300", today()).line(
        BillLineDraft::new("Customs duty", 30_000, VatCategory::Standard)
            .landed_cost(landed.subject()),
    );
    duty.legal_entity_id = Some(entity.id);
    duty.due_date = Some(today() + Days::new(30));
    let second = supplier_bill::create(t.db(), &duty, None).await.unwrap();
    submit(t.state(), By::System(None), second.id)
        .await
        .unwrap();
    approve(t.state(), By::System(None), second.id, "")
        .await
        .unwrap();

    let found = drafts(&t, &landed).await;
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].bill_id, Some(bill.id));
    assert_eq!(found[0].total, 80_000);
    assert_eq!(found[1].bill_id, Some(second.id));
    assert_eq!(found[1].total, 30_000);
    let lines = landed_adjust::lines_of(t.db(), found[1].id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].kind, CostKind::Duty);
    assert_eq!(told(&t).len(), 2);
}
