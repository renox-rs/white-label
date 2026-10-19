//! Bill links and voids (#138 part 6; AC7, AC8, AC9, AC12). A test cannot
//! build an `AuthUser`, so the void runs through its system twin
//! (`void_approved`) and the audit listener through `RecordVoided`.

mod support;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::StateMachine;
use wl_kernel::approvals;
use wl_kernel::invoicing::{
    SupplierBillApproved, SupplierBillFuture, SupplierBillId, SupplierBillLine, SupplierBillLineId,
    SupplierBillStatus, SupplierBillVoided, SupplierBills, provide_supplier_bills,
};
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRequest, provide_notifier};
use wl_kernel::parties::PartyId;
use wl_kernel::voids::{RecordVoided, VoidRecord};
use wl_sales::bill_links::{self, BillLinkInput};
use wl_sales::costs::{self, CostInput};
use wl_sales::permissions as perm;
use wl_sales::{
    Campaign, CostEntry, CostRefused, CostStatus, MarketingChannel, Purpose, voids as sales_voids,
};

use support::{Booted, boot_with};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

/// A test-only `SupplierBills`.
#[derive(Clone, Default)]
struct Bills {
    lines: Arc<Mutex<HashMap<i64, SupplierBillLine>>>,
}

impl Bills {
    fn put(&self, bill: i64, line: i64, amount: i64, status: SupplierBillStatus) {
        self.lines.lock().unwrap().insert(
            line,
            SupplierBillLine {
                bill: SupplierBillId(bill),
                line: SupplierBillLineId(line),
                bill_number: format!("INV-{bill}"),
                supplier: PartyId(1),
                legal_entity_id: None,
                description: "Meta ads".into(),
                amount_aed: amount,
                bill_status: status,
            },
        );
    }

    fn set_status(&self, bill: i64, status: SupplierBillStatus) {
        for line in self.lines.lock().unwrap().values_mut() {
            if line.bill.0 == bill {
                line.bill_status = status;
            }
        }
    }
}

impl SupplierBills for Bills {
    fn line<'a>(
        &'a self,
        _db: &'a Db,
        id: SupplierBillLineId,
    ) -> SupplierBillFuture<'a, Option<SupplierBillLine>> {
        Box::pin(async move { Ok(self.lines.lock().unwrap().get(&id.0).cloned()) })
    }
}

#[derive(Clone, Default)]
struct Recorder {
    sent: Arc<Mutex<Vec<NotifyRequest>>>,
}

impl Recorder {
    fn sales(&self, key: &str) -> Vec<NotifyRequest> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.type_key == key)
            .cloned()
            .collect()
    }
}

impl Notifier for Recorder {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            self.sent.lock().unwrap().push(request);
            Ok(vec![MessageId(1)])
        })
    }
}

struct Providers(Recorder, Bills);

impl Module for Providers {
    fn name(&self) -> &'static str {
        "providers"
    }

    fn register(&self, app: &mut Registry) {
        provide_notifier(app, self.0.clone());
        provide_supplier_bills(app, self.1.clone());
    }
}

struct World {
    t: Booted,
    recorder: Recorder,
    bills: Bills,
    owner: User,
    a: User,
    b: User,
}

async fn world() -> World {
    let recorder = Recorder::default();
    let bills = Bills::default();
    let t = boot_with(Providers(recorder.clone(), bills.clone())).await;
    approvals::install(t.db()).await.unwrap();
    let owner = t.user("Owner", &[]).await;
    let a = t
        .user("Aisha", &[perm::CAMPAIGNS_VIEW, perm::CAMPAIGNS_MANAGE])
        .await;
    let b = t
        .user(
            "Bilal",
            &[
                perm::CAMPAIGNS_VIEW,
                perm::CAMPAIGNS_MANAGE,
                perm::CAMPAIGNS_COSTS_MANAGE,
            ],
        )
        .await;
    World {
        t,
        recorder,
        bills,
        owner,
        a,
        b,
    }
}

async fn campaign(w: &World, budget: Option<i64>, code: &str) -> Campaign {
    Campaign {
        name: "Spring".into(),
        code: code.into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::Meta),
        owner_id: Some(w.owner.id),
        start_date: Some(day(3, 1)),
        end_date: Some(day(3, 31)),
        budget,
        ..Campaign::default()
    }
    .store(w.t.db())
    .await
    .unwrap()
    .unwrap()
}

fn link(c: &Campaign, line: i64, amount: Option<i64>, by: &User) -> BillLinkInput {
    BillLinkInput {
        campaign_id: c.id,
        entry_date: day(3, 5),
        line: SupplierBillLineId(line),
        amount,
        description: None,
        requested_by: by.id,
    }
}

async fn linked(w: &World, c: &Campaign, line: i64, amount: Option<i64>) -> CostEntry {
    bill_links::save_linked(w.t.state(), link(c, line, amount, &w.a))
        .await
        .unwrap()
        .unwrap()
}

async fn approved_total(w: &World, c: &Campaign) -> i64 {
    costs::totals(w.t.db(), c.id, None).await.unwrap().approved
}

async fn reload(w: &World, id: i64) -> CostEntry {
    CostEntry::find(w.t.db(), id).await.unwrap().unwrap()
}

async fn manual(w: &World, c: &Campaign, amount: i64) -> CostEntry {
    costs::save(
        w.t.state(),
        CostInput {
            campaign_id: c.id,
            entry_date: day(3, 5),
            amount,
            description: "Ads".into(),
            requested_by: w.a.id,
            manages_costs: false,
        },
    )
    .await
    .unwrap()
    .unwrap()
}

/// AC7: one line is split over campaigns, never past its amount.
#[renox::test]
async fn a_line_is_split_over_campaigns_but_never_past_its_amount() {
    let w = world().await;
    w.bills.put(1, 10, 300_000, SupplierBillStatus::Approved);
    let c1 = campaign(&w, None, "SPRING26").await;
    let c2 = campaign(&w, None, "SUMMER26").await;

    let first = linked(&w, &c1, 10, Some(200_000)).await;
    assert_eq!(first.status, CostStatus::Approved);
    assert_eq!(first.supplier_bill_id, Some(1));
    assert_eq!(first.linked_by, Some(w.a.id));
    assert_eq!(first.description, "Meta ads", "the line's description");
    // No amount: what is left of the line.
    let second = linked(&w, &c2, 10, None).await;
    assert_eq!(second.amount, 100_000);

    let third = bill_links::save_linked(w.t.state(), link(&c1, 10, Some(100), &w.a))
        .await
        .unwrap();
    assert_eq!(third.err(), Some(CostRefused::LineExceeded));

    // Leaving an entry out of the sum frees its share.
    let mut tx = w.t.db().begin().await.unwrap();
    assert_eq!(
        bill_links::line_used(&mut tx, 10, Some(second.id))
            .await
            .unwrap(),
        200_000
    );
    tx.rollback().await.unwrap();
}

#[renox::test]
async fn bad_links_are_refused() {
    let w = world().await;
    w.bills.put(1, 10, 300_000, SupplierBillStatus::Approved);
    w.bills.put(2, 20, 50_000, SupplierBillStatus::Void);
    let c = campaign(&w, None, "SPRING26").await;
    let save = |line, amount| {
        let input = link(&c, line, amount, &w.a);
        bill_links::save_linked(w.t.state(), input)
    };
    assert_eq!(
        save(99, None).await.unwrap().err(),
        Some(CostRefused::LineNotFound)
    );
    assert_eq!(
        save(20, None).await.unwrap().err(),
        Some(CostRefused::BillVoided)
    );
    assert_eq!(
        save(10, Some(-5)).await.unwrap().err(),
        Some(CostRefused::NotPositive)
    );
    assert_eq!(
        save(10, Some(0)).await.unwrap().err(),
        Some(CostRefused::NotPositive)
    );
    assert_eq!(
        save(10, Some(300_001)).await.unwrap().err(),
        Some(CostRefused::LineExceeded)
    );
}

/// AC8: a linked entry waits for its bill; a voided bill voids it.
#[renox::test]
async fn an_entry_waits_for_its_bill_and_is_voided_with_it() {
    let w = world().await;
    w.bills.put(1, 10, 100_000, SupplierBillStatus::Exception);
    w.bills.put(2, 20, 70_000, SupplierBillStatus::Matched);
    let c = campaign(&w, None, "SPRING26").await;

    let waiting = linked(&w, &c, 10, None).await;
    assert_eq!(waiting.status, CostStatus::AwaitingBill);
    assert_eq!(approved_total(&w, &c).await, 0);
    let totals = costs::totals(w.t.db(), c.id, None).await.unwrap();
    assert_eq!(totals.awaiting_bill, 100_000);

    let other = linked(&w, &c, 20, None).await;
    assert_eq!(other.status, CostStatus::AwaitingBill);

    w.bills.set_status(1, SupplierBillStatus::Approved);
    w.t.state()
        .emit(SupplierBillApproved {
            bill: SupplierBillId(1),
            supplier: PartyId(1),
            amount: 100_000,
            cost_objects: Vec::new(),
            landed_costs: Vec::new(),
        })
        .await
        .unwrap();
    let counted = reload(&w, waiting.id).await;
    assert_eq!(counted.status, CostStatus::Approved);
    assert_eq!(approved_total(&w, &c).await, 100_000);
    assert_eq!(
        reload(&w, other.id).await.status,
        CostStatus::AwaitingBill,
        "another bill's entry does not move"
    );

    w.t.state()
        .emit(SupplierBillVoided {
            bill: SupplierBillId(2),
            reason: "duplicate".into(),
        })
        .await
        .unwrap();
    let voided = reload(&w, other.id).await;
    assert_eq!(voided.status, CostStatus::Voided);
    assert_eq!(voided.reason.as_deref(), Some("supplier bill voided"));
    assert_eq!(approved_total(&w, &c).await, 100_000);
    let told = w.recorder.sales("sales.campaign_cost_bill_voided");
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].recipients.len(), 1);

    // An approved entry stays when its bill is voided later.
    w.t.state()
        .emit(SupplierBillVoided {
            bill: SupplierBillId(1),
            reason: "late".into(),
        })
        .await
        .unwrap();
    assert_eq!(reload(&w, waiting.id).await.status, CostStatus::Approved);
    // The line of a voided entry is free again.
    let mut tx = w.t.db().begin().await.unwrap();
    assert_eq!(bill_links::line_used(&mut tx, 20, None).await.unwrap(), 0);
    tx.rollback().await.unwrap();
}

/// AC9: after the bill is approved the entry is checked against the budget
/// with the linking user as requester.
#[renox::test]
async fn an_approved_bill_goes_through_the_budget_check() {
    let w = world().await;
    w.bills.put(1, 10, 200_000, SupplierBillStatus::Matched);
    let c = campaign(&w, Some(500_000), "SPRING26").await;
    manual(&w, &c, 400_000).await;
    assert_eq!(w.recorder.sales("sales.campaign_budget_alert").len(), 1);

    let waiting = linked(&w, &c, 10, None).await;
    assert_eq!(waiting.status, CostStatus::AwaitingBill);
    assert_eq!(w.recorder.sales("sales.campaign_budget_alert").len(), 1);

    w.bills.set_status(1, SupplierBillStatus::Approved);
    w.t.state()
        .emit(SupplierBillApproved {
            bill: SupplierBillId(1),
            supplier: PartyId(1),
            amount: 200_000,
            cost_objects: Vec::new(),
            landed_costs: Vec::new(),
        })
        .await
        .unwrap();
    let pending = reload(&w, waiting.id).await;
    assert_eq!(pending.status, CostStatus::PendingApproval);
    assert_eq!(pending.requested_by, w.a.id);
    assert_eq!(approved_total(&w, &c).await, 400_000);

    approvals::approve(w.t.state(), pending.approval_request_id.unwrap(), &w.b, "")
        .await
        .unwrap();
    assert_eq!(reload(&w, waiting.id).await.status, CostStatus::Approved);
    assert_eq!(approved_total(&w, &c).await, 600_000);
    let alerts = w.recorder.sales("sales.campaign_budget_alert");
    assert_eq!(alerts.len(), 2, "80% then 100%");
}

#[renox::test]
async fn editing_a_linked_entry_keeps_it_within_its_line() {
    let w = world().await;
    w.bills.put(1, 10, 300_000, SupplierBillStatus::Approved);
    let c = campaign(&w, None, "SPRING26").await;
    let entry = linked(&w, &c, 10, Some(200_000)).await;
    assert_eq!(entry.status, CostStatus::Approved);
    // An approved entry is never edited.
    let change = CostInput {
        campaign_id: c.id,
        entry_date: day(3, 6),
        amount: 250_000,
        description: "Ads".into(),
        requested_by: w.a.id,
        manages_costs: false,
    };
    assert_eq!(
        costs::edit(w.t.state(), entry.id, change)
            .await
            .unwrap()
            .err(),
        Some(CostRefused::Approved)
    );
}

/// AC12: a void with a reason by the system twin; the Void record, the
/// audit entry and the totals.
#[renox::test]
async fn a_voided_entry_drops_out_of_the_totals_and_is_audited() {
    let w = world().await;
    let c = campaign(&w, Some(500_000), "SPRING26").await;
    let entry = manual(&w, &c, 100_000).await;
    assert_eq!(entry.status, CostStatus::Approved);

    let outcome = reload(&w, entry.id)
        .await
        .transition("void_approved")
        .by_system()
        .reason("Entered twice")
        .meta(json!({ "requester_id": w.b.id }))
        .run(w.t.state())
        .await
        .unwrap();
    drop(outcome);
    let voided = reload(&w, entry.id).await;
    assert_eq!(voided.status, CostStatus::Voided);
    assert_eq!(approved_total(&w, &c).await, 0);

    let record = VoidRecord::of_subject(w.t.db(), &voided.subject())
        .await
        .unwrap()
        .expect("a Void record");
    assert_eq!(record.reason, "Entered twice");
    assert_eq!(record.state_before, "approved");
    assert_eq!(record.voided_by, w.b.id);

    w.t.state().emit(RecordVoided::of(&record)).await.unwrap();
    let log = renox::audit::for_subject(w.t.db(), CostEntry::TABLE, entry.id, 10)
        .await
        .unwrap();
    let entry_log = log
        .iter()
        .find(|l| l.action == sales_voids::AUDIT_ACTION)
        .expect("the void in the activity log");
    assert_eq!(entry_log.user_id, Some(w.b.id));
    assert_eq!(entry_log.data["reason"], "Entered twice");
    assert_eq!(entry_log.data["amount"], 100_000);
    assert_eq!(entry_log.data["campaign_code"], "SPRING26");

    // Terminal: nothing leaves `voided`.
    assert!(
        reload(&w, entry.id)
            .await
            .transition("reject")
            .by_system()
            .run(w.t.state())
            .await
            .is_err()
    );
}

/// A voided pending entry's request ends with it.
#[renox::test]
async fn voiding_a_pending_entry_ends_its_request() {
    let w = world().await;
    let c = campaign(&w, Some(500_000), "SPRING26").await;
    manual(&w, &c, 480_000).await;
    let over = manual(&w, &c, 40_000).await;
    assert_eq!(over.status, CostStatus::PendingApproval);
    let request = over.approval_request_id.unwrap();

    let _done = reload(&w, over.id)
        .await
        .transition("void_approved")
        .by_system()
        .reason("Wrong campaign")
        .meta(json!({ "requester_id": w.b.id }))
        .run(w.t.state())
        .await
        .unwrap();
    assert_eq!(reload(&w, over.id).await.status, CostStatus::Voided);
    let stored = approvals::ApprovalRequest::find(w.t.db(), request)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(stored.status, approvals::ApprovalStatus::Pending);
    assert_eq!(approved_total(&w, &c).await, 480_000);
    // Nobody can approve it any more.
    assert!(
        approvals::approve(w.t.state(), request, &w.b, "")
            .await
            .is_err()
    );
}
