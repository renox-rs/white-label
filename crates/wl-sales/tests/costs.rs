//! Cost entries and the budget check (#138 part 5; AC5, AC6, AC10, AC11,
//! AC13 and the delete rule of AC14). A test cannot build an `AuthUser`, so
//! the service is called with the user's id and the kernel's approval
//! functions with `User` rows.

mod support;

use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::approvals::{self, ApprovalRequest, ApprovalStatus};
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRequest, provide_notifier};
use wl_sales::costs::{self, CostInput};
use wl_sales::permissions as perm;
use wl_sales::{
    BudgetAlert, Campaign, CampaignRefused, CostEntry, CostRefused, CostStatus, MarketingChannel,
    Purpose, campaigns,
};

use support::{Booted, boot_with};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

#[derive(Clone, Default)]
struct Recorder {
    sent: Arc<Mutex<Vec<NotifyRequest>>>,
}

impl Recorder {
    /// The sales notices sent so far, by type key.
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

struct Recording(Recorder);

impl Module for Recording {
    fn name(&self) -> &'static str {
        "recording"
    }

    fn register(&self, app: &mut Registry) {
        provide_notifier(app, self.0.clone());
    }
}

/// The app, the recorder, the owner, user A (`marketing`) and user B
/// (`marketing_manager`).
struct World {
    t: Booted,
    recorder: Recorder,
    owner: User,
    a: User,
    b: User,
}

async fn world() -> World {
    let recorder = Recorder::default();
    let t = boot_with(Recording(recorder.clone())).await;
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
        owner,
        a,
        b,
    }
}

/// C1 of the plan: `SPRING26`, 1-31 Mar 2026, AED 5,000 (500,000 fils).
async fn c1(w: &World, budget: Option<i64>, code: &str) -> Campaign {
    Campaign {
        name: "Spring".into(),
        code: code.into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::Meta.into()),
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

fn input(campaign: &Campaign, amount: i64, by: &User) -> CostInput {
    CostInput {
        campaign_id: campaign.id,
        entry_date: day(3, 5),
        amount,
        description: "Ads".into(),
        requested_by: by.id,
        manages_costs: false,
    }
}

async fn save(w: &World, campaign: &Campaign, amount: i64, by: &User) -> CostEntry {
    costs::save(w.t.state(), input(campaign, amount, by))
        .await
        .unwrap()
        .unwrap()
}

async fn approved_total(w: &World, campaign: &Campaign) -> i64 {
    costs::totals(w.t.db(), campaign.id, None)
        .await
        .unwrap()
        .approved
}

async fn reload(w: &World, id: i64) -> CostEntry {
    CostEntry::find(w.t.db(), id).await.unwrap().unwrap()
}

/// AC5: over the budget the entry waits for another holder, who approves.
#[renox::test]
async fn an_entry_over_the_budget_waits_for_a_second_holder() {
    let w = world().await;
    let c = c1(&w, Some(500_000), "SPRING26").await;
    let first = save(&w, &c, 480_000, &w.a).await;
    assert_eq!(first.status, CostStatus::Approved);
    assert_eq!(first.approved_by, None, "approved on save");

    let over = save(&w, &c, 40_000, &w.a).await;
    assert_eq!(over.status, CostStatus::PendingApproval);
    let request = over.approval_request_id.expect("a kernel request");
    let stored = ApprovalRequest::find(w.t.db(), request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ApprovalStatus::Pending);
    assert_eq!(stored.action, "campaigns.cost_over_budget");
    assert_eq!(stored.requester_id, w.a.id);
    assert_eq!(approved_total(&w, &c).await, 480_000);
    let totals = costs::totals(w.t.db(), c.id, None).await.unwrap();
    assert_eq!(totals.pending, 40_000);

    // The requester may not approve, not even as a holder (SoD).
    let root =
        w.t.user(
            "Root",
            &[
                perm::CAMPAIGNS_VIEW,
                perm::CAMPAIGNS_MANAGE,
                perm::CAMPAIGNS_COSTS_MANAGE,
            ],
        )
        .await;
    let own = save(&w, &c, 30_000, &root).await;
    assert_eq!(own.status, CostStatus::PendingApproval);
    assert!(
        approvals::approve(w.t.state(), own.approval_request_id.unwrap(), &root, "")
            .await
            .is_err()
    );
    assert!(
        approvals::approve(w.t.state(), request, &w.a, "")
            .await
            .is_err(),
        "A does not hold the permission"
    );
    assert_eq!(
        reload(&w, over.id).await.status,
        CostStatus::PendingApproval
    );

    approvals::approve(w.t.state(), request, &w.b, "")
        .await
        .unwrap();
    let approved = reload(&w, over.id).await;
    assert_eq!(approved.status, CostStatus::Approved);
    assert_eq!(approved.approved_by, Some(w.b.id));
    assert_eq!(approved_total(&w, &c).await, 520_000);
}

/// AC6: a rejection ends in `rejected` with the comment, out of every total.
#[renox::test]
async fn a_rejected_entry_keeps_the_comment_and_tells_the_owner() {
    let w = world().await;
    let c = c1(&w, Some(500_000), "SPRING26").await;
    save(&w, &c, 480_000, &w.a).await;
    let over = save(&w, &c, 40_000, &w.a).await;
    let request = over.approval_request_id.unwrap();
    assert!(
        approvals::reject(w.t.state(), request, &w.b, " ")
            .await
            .is_err(),
        "a comment is required"
    );
    approvals::reject(w.t.state(), request, &w.b, "Too much for March")
        .await
        .unwrap();
    let rejected = reload(&w, over.id).await;
    assert_eq!(rejected.status, CostStatus::Rejected);
    assert_eq!(rejected.reason.as_deref(), Some("Too much for March"));
    assert_eq!(approved_total(&w, &c).await, 480_000);
    let totals = costs::totals(w.t.db(), c.id, None).await.unwrap();
    assert_eq!(totals.pending, 0);

    let told = w.recorder.sales("sales.campaign_cost_rejected");
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["reason"], "Too much for March");
    assert_eq!(told[0].recipients.len(), 1);
}

/// The requester's edit withdraws the request and checks the budget again.
#[renox::test]
async fn editing_a_pending_entry_withdraws_it_and_submits_again() {
    let w = world().await;
    let c = c1(&w, Some(500_000), "SPRING26").await;
    save(&w, &c, 480_000, &w.a).await;
    let over = save(&w, &c, 40_000, &w.a).await;
    let request = over.approval_request_id.unwrap();

    let mut change = input(&c, 10_000, &w.a);
    // Someone else cannot edit it.
    change.requested_by = w.b.id;
    assert_eq!(
        costs::edit(w.t.state(), over.id, change)
            .await
            .unwrap()
            .unwrap_err(),
        CostRefused::NotRequester
    );
    let edited = costs::edit(w.t.state(), over.id, input(&c, 10_000, &w.a))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(edited.status, CostStatus::Approved, "within the budget now");
    assert_eq!(edited.amount, 10_000);
    let old = ApprovalRequest::find(w.t.db(), request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old.status, ApprovalStatus::Withdrawn);
    assert_eq!(approved_total(&w, &c).await, 490_000);

    // An approved entry is never edited.
    assert_eq!(
        costs::edit(w.t.state(), over.id, input(&c, 20_000, &w.a))
            .await
            .unwrap()
            .unwrap_err(),
        CostRefused::Approved
    );

    // A still-too-high edit waits again, with a new request.
    let again = save(&w, &c, 5_000, &w.a).await;
    assert_eq!(again.status, CostStatus::Approved);
    let waiting = save(&w, &c, 50_000, &w.a).await;
    assert_eq!(waiting.status, CostStatus::PendingApproval);
    let first_request = waiting.approval_request_id.unwrap();
    let edited = costs::edit(w.t.state(), waiting.id, input(&c, 60_000, &w.a))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(edited.status, CostStatus::PendingApproval);
    assert_ne!(edited.approval_request_id, Some(first_request));
}

/// AC10 and AC11: the alerts, once per threshold; a credit lowers the total
/// without a check, and a later rise is not alerted again.
#[renox::test]
async fn the_owner_is_alerted_once_at_80_and_100_percent() {
    let w = world().await;
    let c = c1(&w, Some(500_000), "SPRING26").await;
    save(&w, &c, 300_000, &w.a).await;
    assert!(w.recorder.sales("sales.campaign_budget_alert").is_empty());
    save(&w, &c, 100_000, &w.a).await;
    let alerts = w.recorder.sales("sales.campaign_budget_alert");
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].data["percent"], 80);
    assert_eq!(alerts[0].data["approved"], "4,000.00");
    assert_eq!(alerts[0].data["budget"], "5,000.00");
    save(&w, &c, 100_000, &w.a).await;
    let alerts = w.recorder.sales("sales.campaign_budget_alert");
    assert_eq!(alerts.len(), 2);
    assert_eq!(alerts[1].data["percent"], 100);
    assert_eq!(BudgetAlert::query().count(w.t.db()).await.unwrap(), 2);

    // A credit needs no budget check and no manager's approval to be saved
    // by one who holds the permission; it lowers the total.
    let mut credit = input(&c, -30_000, &w.b);
    credit.manages_costs = true;
    let credit = costs::save(w.t.state(), credit).await.unwrap().unwrap();
    assert_eq!(credit.status, CostStatus::Approved);
    assert_eq!(approved_total(&w, &c).await, 470_000);
    // Without the permission a credit is refused.
    assert_eq!(
        costs::save(w.t.state(), input(&c, -100, &w.a))
            .await
            .unwrap()
            .unwrap_err(),
        CostRefused::NegativeNeedsManager
    );
    // Rising past the thresholds again alerts nobody.
    save(&w, &c, 30_000, &w.a).await;
    assert_eq!(w.recorder.sales("sales.campaign_budget_alert").len(), 2);
}

/// An approval after the fact also raises the alert, once.
#[renox::test]
async fn an_approved_over_budget_entry_raises_the_100_percent_alert() {
    let w = world().await;
    let c = c1(&w, Some(500_000), "SPRING26").await;
    save(&w, &c, 400_000, &w.a).await;
    assert_eq!(w.recorder.sales("sales.campaign_budget_alert").len(), 1);
    let over = save(&w, &c, 200_000, &w.a).await;
    assert_eq!(over.status, CostStatus::PendingApproval);
    assert_eq!(w.recorder.sales("sales.campaign_budget_alert").len(), 1);
    approvals::approve(w.t.state(), over.approval_request_id.unwrap(), &w.b, "")
        .await
        .unwrap();
    let alerts = w.recorder.sales("sales.campaign_budget_alert");
    assert_eq!(alerts.len(), 2);
    assert_eq!(alerts[1].data["percent"], 100);
}

/// A campaign without a budget never asks for approval and never alerts.
#[renox::test]
async fn a_campaign_without_a_budget_is_never_checked() {
    let w = world().await;
    let c = c1(&w, None, "NOBUDGET").await;
    let entry = save(&w, &c, 99_999_999, &w.a).await;
    assert_eq!(entry.status, CostStatus::Approved);
    assert_eq!(entry.approval_request_id, None);
    assert!(w.recorder.sales("sales.campaign_budget_alert").is_empty());
}

/// AC13: two entries saved at the same time on AED 4,500.
#[renox::test]
async fn two_entries_saved_together_do_not_both_fit() {
    let w = world().await;
    let c = c1(&w, Some(500_000), "SPRING26").await;
    save(&w, &c, 450_000, &w.a).await;
    let (x, y) = renox::tokio::join!(
        costs::save(w.t.state(), input(&c, 30_000, &w.a)),
        costs::save(w.t.state(), input(&c, 30_000, &w.b)),
    );
    let mut states = vec![x.unwrap().unwrap().status, y.unwrap().unwrap().status];
    states.sort_by_key(|s| s.to_string());
    assert_eq!(states, [CostStatus::Approved, CostStatus::PendingApproval]);
    assert_eq!(approved_total(&w, &c).await, 480_000);
}

/// A18: only approved entries count, split into bill-linked and manual.
#[renox::test]
async fn totals_count_only_approved_entries_by_date() {
    let w = world().await;
    let c = c1(&w, Some(500_000), "SPRING26").await;
    let make = |status, amount, date, bill: Option<i64>| CostEntry {
        campaign_id: c.id,
        entry_date: date,
        amount,
        requested_by: w.a.id,
        supplier_bill_id: bill,
        status,
        ..CostEntry::default()
    };
    for entry in [
        make(CostStatus::Approved, 100_000, day(3, 5), None),
        make(CostStatus::Approved, 50_000, day(3, 20), Some(7)),
        make(CostStatus::Approved, 25_000, day(4, 2), None),
        make(CostStatus::PendingApproval, 1_000, day(3, 6), None),
        make(CostStatus::AwaitingBill, 2_000, day(3, 6), Some(8)),
        make(CostStatus::Draft, 4_000, day(3, 6), None),
        make(CostStatus::Rejected, 8_000, day(3, 6), None),
        make(CostStatus::Voided, 16_000, day(3, 6), None),
    ] {
        let mut entry = entry;
        let mut tx = w.t.db().begin().await.unwrap();
        let status = entry.status;
        entry
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    let all = costs::totals(w.t.db(), c.id, None).await.unwrap();
    assert_eq!(
        (all.approved, all.bill_linked, all.manual),
        (175_000, 50_000, 125_000)
    );
    assert_eq!((all.pending, all.awaiting_bill), (1_000, 2_000));
    let march = costs::totals(w.t.db(), c.id, Some((day(3, 1), day(3, 31))))
        .await
        .unwrap();
    assert_eq!((march.approved, march.manual), (150_000, 100_000));
}

/// AC14: a cost entry in any status keeps the campaign.
#[renox::test]
async fn a_campaign_with_a_cost_entry_cannot_be_deleted() {
    let w = world().await;
    let c = c1(&w, Some(500_000), "SPRING26").await;
    let spare = c1(&w, None, "SPARE").await;
    save(&w, &c, 10_000, &w.a).await;
    assert_eq!(
        campaigns::delete(w.t.db(), c.id).await.unwrap(),
        Err(CampaignRefused::InUse(
            "This campaign has cost entries; it cannot be deleted".to_owned()
        ))
    );
    campaigns::delete(w.t.db(), spare.id)
        .await
        .unwrap()
        .unwrap();
}

#[renox::test]
async fn bad_input_is_refused() {
    let w = world().await;
    let c = c1(&w, Some(500_000), "SPRING26").await;
    assert_eq!(
        costs::save(w.t.state(), input(&c, 0, &w.a))
            .await
            .unwrap()
            .unwrap_err(),
        CostRefused::NoAmount
    );
    let mut missing = input(&c, 100, &w.a);
    missing.campaign_id = 999;
    assert_eq!(
        costs::save(w.t.state(), missing)
            .await
            .unwrap()
            .unwrap_err(),
        CostRefused::CampaignNotFound
    );
    assert_eq!(
        costs::edit(w.t.state(), 999, input(&c, 100, &w.a))
            .await
            .unwrap()
            .unwrap_err(),
        CostRefused::NotFound
    );
}
