//! The campaigns back office (#138 part 7): the list, the form, the
//! campaign page with its transitions, reopen, delete, the "Add cost" sheet
//! and Void. The acceptance checks of the whole issue come in part 8.

mod support;

use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use wl_kernel::approvals;
use wl_kernel::documents::today;
use wl_sales::costs::{self, CostInput};
use wl_sales::permissions as perm;
use wl_sales::{Campaign, CampaignStatus, CostEntry, CostStatus, MarketingChannel, Purpose};

use support::{Booted, boot};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

/// A draft campaign with everything `activate` needs.
async fn spring(t: &Booted, owner: &User, budget: Option<i64>) -> Campaign {
    Campaign {
        name: "Spring".into(),
        code: "SPRING26".into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::Meta),
        owner_id: Some(owner.id),
        start_date: Some(day(3, 1)),
        end_date: Some(day(3, 31)),
        budget,
        promo_code: Some("SPRING26".into()),
        ..Campaign::default()
    }
    .store(t.db())
    .await
    .unwrap()
    .unwrap()
}

async fn viewer(t: &Booted, name: &str) -> User {
    t.user(name, &[perm::CAMPAIGNS_VIEW]).await
}

async fn manager(t: &Booted, name: &str) -> User {
    t.user(name, &[perm::CAMPAIGNS_VIEW, perm::CAMPAIGNS_MANAGE])
        .await
}

async fn approver(t: &Booted, name: &str) -> User {
    t.user(
        name,
        &[
            perm::CAMPAIGNS_VIEW,
            perm::CAMPAIGNS_MANAGE,
            perm::CAMPAIGNS_COSTS_MANAGE,
        ],
    )
    .await
}

async fn reload(t: &Booted, id: i64) -> Campaign {
    Campaign::find(t.db(), id).await.unwrap().unwrap()
}

#[renox::test]
async fn the_list_needs_campaigns_view_and_shows_the_campaigns() {
    let t = boot().await;
    let owner = t.user("Owner", &[]).await;
    spring(&t, &owner, Some(500_000)).await;

    let nobody = t.user("Nobody", &[]).await;
    t.app.acting_as(&nobody);
    t.app.get("/campaigns").await.assert_forbidden();

    let reader = viewer(&t, "Vera").await;
    t.app.acting_as(&reader);
    t.app
        .get("/campaigns")
        .await
        .assert_ok()
        .assert_see("SPRING26")
        .assert_see("Spring")
        .assert_dont_see("New campaign");

    let boss = manager(&t, "Mona").await;
    t.app.acting_as(&boss);
    t.app
        .get("/campaigns")
        .await
        .assert_ok()
        .assert_see("New campaign");
}

#[renox::test]
async fn the_form_creates_and_changes_a_campaign_and_refuses_bad_input() {
    let t = boot().await;
    let boss = manager(&t, "Mona").await;
    t.app.acting_as(&boss);
    t.app
        .get("/campaigns/new")
        .await
        .assert_ok()
        .assert_see("Marketing channel");

    let fields = |code: &'static str| {
        vec![
            ("name", "Spring offer"),
            ("code", code),
            ("purpose", "acquisition"),
            ("marketing_channel", "meta"),
            ("start_date", "2026-03-01"),
            ("end_date", "2026-03-31"),
            ("budget", "5,000.00"),
            ("promo_code", "spring26"),
        ]
    };
    t.app
        .post("/campaigns", &fields(" spring26 "))
        .await
        .assert_status(303);
    let c = Campaign::by_code(t.db(), "SPRING26")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.status, CampaignStatus::Draft);
    assert_eq!(c.budget, Some(500_000));
    assert_eq!(c.promo_code.as_deref(), Some("SPRING26"));

    // The same code in any case, a bad code and an end before the start.
    t.app
        .htmx()
        .post("/campaigns", &fields("Spring26"))
        .await
        .assert_invalid("code");
    t.app
        .htmx()
        .post("/campaigns", &fields("x"))
        .await
        .assert_invalid("code");
    let mut backwards = fields("OTHER1");
    backwards.push(("end_date", "2026-02-01"));
    t.app
        .htmx()
        .post("/campaigns", &backwards)
        .await
        .assert_invalid("end_date");
    let mut bad_budget = fields("OTHER2");
    bad_budget.push(("budget", "lots"));
    t.app
        .htmx()
        .post("/campaigns", &bad_budget)
        .await
        .assert_invalid("budget");

    // Changing keeps the status.
    t.app
        .get(&format!("/campaigns/{}/edit", c.id))
        .await
        .assert_ok()
        .assert_see("SPRING26");
    let mut changed = fields("SPRING26");
    changed.push(("name", "Spring offer 2"));
    t.app
        .post(&format!("/campaigns/{}", c.id), &changed)
        .await
        .assert_status(303);
    let c = reload(&t, c.id).await;
    assert_eq!(c.name, "Spring offer 2");
    assert_eq!(c.status, CampaignStatus::Draft);

    // A viewer cannot reach the form.
    let reader = viewer(&t, "Vera").await;
    t.app.acting_as(&reader);
    t.app.get("/campaigns/new").await.assert_forbidden();
}

#[renox::test]
async fn the_page_runs_the_transitions_and_reopens_with_a_future_end_date() {
    let t = boot().await;
    let owner = t.user("Owner", &[]).await;
    let boss = manager(&t, "Mona").await;
    let c = spring(&t, &owner, Some(500_000)).await;
    t.app.acting_as(&boss);

    t.app
        .get(&format!("/campaigns/{}", c.id))
        .await
        .assert_ok()
        .assert_see("Spring")
        .assert_see("Activate")
        .assert_see("Add cost")
        .assert_see("Delete")
        .assert_dont_see("Supplier bill line");

    let base = format!("/campaigns/{}/transitions", c.id);
    t.app
        .post(&format!("{base}/activate"), &[("x", "")])
        .await
        .assert_status(303);
    assert_eq!(reload(&t, c.id).await.status, CampaignStatus::Active);
    t.app
        .get(&format!("/campaigns/{}", c.id))
        .await
        .assert_see("End")
        .assert_dont_see("Reopen");
    t.app
        .post(&format!("{base}/end"), &[("x", "")])
        .await
        .assert_status(303);
    assert_eq!(reload(&t, c.id).await.status, CampaignStatus::Ended);

    t.app
        .get(&format!("/campaigns/{}", c.id))
        .await
        .assert_ok()
        .assert_see("Reopen");
    let reopen = format!("/campaigns/{}/reopen", c.id);
    t.app
        .htmx()
        .post(&reopen, &[("end_date", "2026-01-01")])
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(&reopen, &[("end_date", "soon")])
        .await
        .assert_invalid("end_date");
    let later = (today() + Duration::days(30)).to_string();
    t.app
        .post(&reopen, &[("end_date", later.as_str())])
        .await
        .assert_status(303);
    let c = reload(&t, c.id).await;
    assert_eq!(c.status, CampaignStatus::Active);
    assert_eq!(c.end_date.map(|d| d.to_string()), Some(later));
}

#[renox::test]
async fn only_a_draft_without_costs_is_deleted() {
    let t = boot().await;
    let owner = t.user("Owner", &[]).await;
    let boss = approver(&t, "Bilal").await;
    t.app.acting_as(&boss);

    let clean = spring(&t, &owner, None).await;
    t.app
        .post(&format!("/campaigns/{}/delete", clean.id), &[("x", "")])
        .await
        .assert_status(303);
    assert!(Campaign::find(t.db(), clean.id).await.unwrap().is_none());

    // The deleted code stays taken.
    let again = Campaign {
        name: "Again".into(),
        code: "SPRING26".into(),
        ..Campaign::default()
    }
    .store(t.db())
    .await
    .unwrap();
    assert!(again.is_err());

    let c = Campaign {
        name: "Costly".into(),
        code: "COSTLY".into(),
        budget: Some(500_000),
        ..Campaign::default()
    }
    .store(t.db())
    .await
    .unwrap()
    .unwrap();
    costs::save(
        t.state(),
        CostInput {
            campaign_id: c.id,
            entry_date: day(3, 5),
            amount: 10_000,
            description: "Ads".into(),
            requested_by: boss.id,
            manages_costs: true,
        },
    )
    .await
    .unwrap()
    .unwrap();
    t.app
        .htmx()
        .post(&format!("/campaigns/{}/delete", c.id), &[("x", "")])
        .await
        .assert_status(422);
    assert!(Campaign::find(t.db(), c.id).await.unwrap().is_some());
}

#[renox::test]
async fn adding_a_cost_over_the_budget_waits_for_the_approval_card() {
    let t = boot().await;
    approvals::install(t.db()).await.unwrap();
    let owner = t.user("Owner", &[]).await;
    let a = manager(&t, "Aisha").await;
    let b = approver(&t, "Bilal").await;
    let c = spring(&t, &owner, Some(500_000)).await;

    t.app.acting_as(&a);
    t.app
        .post(
            &format!("/campaigns/{}/costs", c.id),
            &[
                ("entry_date", "2026-03-05"),
                ("amount", "4,800.00"),
                ("description", "Ads"),
            ],
        )
        .await
        .assert_ok();
    t.app
        .post(
            &format!("/campaigns/{}/costs", c.id),
            &[
                ("entry_date", "2026-03-06"),
                ("amount", "400"),
                ("description", "More ads"),
            ],
        )
        .await
        .assert_ok();
    let entries = CostEntry::query()
        .order_by(CostEntry::ID)
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(entries[0].status, CostStatus::Approved);
    assert_eq!(entries[1].status, CostStatus::PendingApproval);
    let totals = costs::totals(t.db(), c.id, None).await.unwrap();
    assert_eq!((totals.approved, totals.pending), (480_000, 40_000));

    // No amount, a bad amount and a credit without the permission.
    let url = format!("/campaigns/{}/costs", c.id);
    t.app
        .htmx()
        .post(&url, &[("amount", "")])
        .await
        .assert_invalid("amount");
    t.app
        .htmx()
        .post(&url, &[("amount", "12.345")])
        .await
        .assert_invalid("amount");
    t.app
        .htmx()
        .post(&url, &[("amount", "-5")])
        .await
        .assert_invalid("amount");

    // The requester sees the entry as pending, with its request, but cannot decide.
    let page = format!("/campaigns/{}", c.id);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("Pending approval")
        .assert_see("AED 400.00")
        .assert_dont_see("Reject");

    // The holder sees the card with its buttons.
    t.app.acting_as(&b);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("Pending approval")
        .assert_see("Reject");
    approvals::approve(t.state(), entries[1].approval_request_id.unwrap(), &b, "")
        .await
        .unwrap();
    let entry = CostEntry::find(t.db(), entries[1].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entry.status, CostStatus::Approved);
    t.app.get(&page).await.assert_ok().assert_see("Approved");
}

#[renox::test]
async fn void_is_offered_to_a_holder_and_refused_to_a_plain_manager() {
    let t = boot().await;
    let owner = t.user("Owner", &[]).await;
    let a = manager(&t, "Aisha").await;
    let b = approver(&t, "Bilal").await;
    let c = spring(&t, &owner, Some(500_000)).await;
    let entry = costs::save(
        t.state(),
        CostInput {
            campaign_id: c.id,
            entry_date: day(3, 5),
            amount: 100_000,
            description: "Ads".into(),
            requested_by: a.id,
            manages_costs: false,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(entry.status, CostStatus::Approved);
    let page = format!("/campaigns/{}", c.id);
    let void = format!("/voids/sales.campaign_cost_entry/{}", entry.id);

    t.app.acting_as(&a);
    t.app.get(&page).await.assert_ok().assert_dont_see("Void");
    t.app
        .htmx()
        .post(&void, &[("reason", "Entered twice")])
        .await
        .assert_status(422);
    assert_eq!(
        CostEntry::find(t.db(), entry.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        CostStatus::Approved
    );

    t.app.acting_as(&b);
    t.app.get(&page).await.assert_ok().assert_see("Void");
    t.app
        .htmx()
        .post(&void, &[("reason", "")])
        .await
        .assert_invalid("reason");
    t.app
        .post(&void, &[("reason", "Entered twice")])
        .await
        .assert_status(303);
    let entry = CostEntry::find(t.db(), entry.id).await.unwrap().unwrap();
    assert_eq!(entry.status, CostStatus::Voided);
    let totals = costs::totals(t.db(), c.id, None).await.unwrap();
    assert_eq!(totals.approved, 0);
    t.app.get(&page).await.assert_ok().assert_see("Voided");
}
