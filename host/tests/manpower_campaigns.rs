//! BL-MPW-20 (#434), `SAL.CRM.02`, `SAL.ACL.07`: a LinkedIn campaign of the
//! seeded manpower agency (the 09 channel *LinkedIn* is recorded as
//! `other_online`, `seed::CAMPAIGN_CHANNELS`) with a budget and cost entries
//! reports its leads, conversion, cost per lead, CAC and ROAS; a cost entry
//! over the budget waits for a manager, as the seeded sales module is set.

mod manpower_support;
mod support;

use host::roles::{MARKETING, MARKETING_MANAGER};
use manpower::seed::CAMPAIGN_CHANNELS;
use manpower_support::boot_seeded;
use renox::prelude::*;
use renox::testing::TestApp;
use support::{day, person};
use wl_kernel::approvals::{self, ApprovalStatus};
use wl_kernel::sales::{Attribute, FirstSale, campaign_attribution};
use wl_kernel::{PartyId, Subject};
use wl_sales::costs::{self, CostInput};
use wl_sales::report::{self, Filter, GroupBy, Viewer};
use wl_sales::{
    Campaign, CostEntry, CostStatus, Lead, LeadSource, MarketingChannel, NewLead, Purpose,
};

const CODE: &str = "LINKEDIN26";

/// The LinkedIn campaign of the agency, active, AED 5,000 budget.
async fn linkedin_campaign(app: &TestApp, owner: &User) -> Campaign {
    let channel = CAMPAIGN_CHANNELS
        .iter()
        .find(|(name, _)| *name == "LinkedIn")
        .expect("the agency names the LinkedIn channel")
        .1;
    assert_eq!(channel, "other_online");
    let source = LeadSource::by_code(app.db(), "paid_ads")
        .await
        .unwrap()
        .unwrap();
    let campaign = Campaign {
        name: "LinkedIn recruitment drive".into(),
        code: CODE.into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::OtherOnline.into()),
        lead_source_id: Some(source.id),
        owner_id: Some(owner.id),
        start_date: Some(day(-30)),
        end_date: Some(day(60)),
        budget: Some(500_000),
        ..Campaign::default()
    }
    .store(app.db())
    .await
    .unwrap()
    .unwrap();
    Campaign::query()
        .where_(Campaign::ID.eq(campaign.id))
        .update(app.db(), &[("status", &"active")])
        .await
        .unwrap();
    Campaign::find(app.db(), campaign.id)
        .await
        .unwrap()
        .unwrap()
}

/// AC10: a LinkedIn campaign with a budget and cost entries: 20 leads, 4 won,
/// AED 3,000 of costs, 3 clients whose first invoice is AED 10,000: conversion
/// 20.0%, cost per lead 150.00, CAC 1,000.00 and ROAS 10.00.
#[renox::test]
async fn the_linkedin_campaign_reports_leads_conversion_cost_per_lead_cac_and_roas() {
    let app = boot_seeded().await;
    let manager = person(&app, "Mona", MARKETING_MANAGER).await;
    let staff = person(&app, "Marco", MARKETING).await;
    let campaign = linkedin_campaign(&app, &manager).await;

    for amount in [200_000, 100_000] {
        let entry = costs::save(
            app.state(),
            CostInput {
                campaign_id: campaign.id,
                entry_date: day(-5),
                amount,
                description: "LinkedIn sponsored post".into(),
                requested_by: staff.id,
                manages_costs: false,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(entry.status, CostStatus::Approved, "within the budget");
    }

    for i in 0..20 {
        let new = NewLead::staff(None, &format!("Enquirer {i}"), "paid_ads").campaign_code(CODE);
        let captured = wl_sales::lead::capture(app.state(), new, day(-3))
            .await
            .unwrap()
            .unwrap();
        let wl_sales::Captured::Created(lead) = captured else {
            panic!("a lead is created");
        };
        if i < 4 {
            Lead::query()
                .where_(Lead::ID.eq(lead.id))
                .update(
                    app.db(),
                    &[("status", &"won"), ("won_at", &Some(renox::db::now()))],
                )
                .await
                .unwrap();
        }
    }
    let attribution = campaign_attribution(app.state());
    for n in 0..3_i64 {
        let party = PartyId(500 + n);
        attribution
            .attribute(
                app.state(),
                Attribute::party(party, "paid_ads").campaign(CODE),
            )
            .await
            .unwrap();
        attribution
            .record_first_sale(
                app.state(),
                FirstSale::new(
                    party,
                    Subject::new("invoicing.invoice", 700 + n),
                    1_000_000,
                    day(-1),
                ),
            )
            .await
            .unwrap();
    }

    let facts = report::load(app.db(), &wl_sales::config::config_of(app.state()))
        .await
        .unwrap();
    let report = report::compute(
        &facts,
        &Filter::default(),
        GroupBy::Campaign,
        &Viewer::all(),
    );
    assert_eq!(report.rows.len(), 1);
    let cells = report.rows[0].cells();
    assert_eq!(cells.leads, "20");
    assert_eq!(cells.won, "4");
    assert_eq!(cells.conversion, "20.0%");
    assert_eq!(cells.customers, "3");
    assert_eq!(cells.cost, "3,000.00");
    assert_eq!(cells.cost_per_lead, "150.00");
    assert_eq!(cells.cac, "1,000.00");
    assert_eq!(cells.value, "30,000.00");
    assert_eq!(cells.roas, "10.00");

    // By channel the same row is LinkedIn's `other_online`.
    let by_channel = report::compute(&facts, &Filter::default(), GroupBy::Channel, &Viewer::all());
    assert_eq!(by_channel.rows.len(), 1);
    assert_eq!(by_channel.rows[0].cells().leads, "20");
}

/// AC10, `SAL.ACL.07`: a cost entry that takes the campaign over its budget
/// waits for a manager; the person who entered it cannot approve it.
#[renox::test]
async fn a_cost_entry_over_the_budget_waits_for_a_manager() {
    let app = boot_seeded().await;
    let manager = person(&app, "Mona", MARKETING_MANAGER).await;
    let staff = person(&app, "Marco", MARKETING).await;
    let campaign = linkedin_campaign(&app, &manager).await;
    let input = |amount| CostInput {
        campaign_id: campaign.id,
        entry_date: day(-5),
        amount,
        description: "LinkedIn sponsored post".into(),
        requested_by: staff.id,
        manages_costs: false,
    };
    let within = costs::save(app.state(), input(450_000))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(within.status, CostStatus::Approved);

    let over = costs::save(app.state(), input(100_000))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(over.status, CostStatus::PendingApproval);
    let request = over.approval_request_id.expect("a request");
    assert_eq!(
        costs::totals(app.db(), campaign.id, None)
            .await
            .unwrap()
            .approved,
        450_000,
        "the waiting entry is not counted"
    );
    assert!(
        approvals::approve(app.state(), request, &staff, "")
            .await
            .is_err(),
        "marketing staff cannot approve their own entry"
    );
    approvals::approve(app.state(), request, &manager, "ok")
        .await
        .unwrap();
    let approved = CostEntry::find(app.db(), over.id).await.unwrap().unwrap();
    assert_eq!(approved.status, CostStatus::Approved);
    assert_eq!(approved.approved_by, Some(manager.id));
    let stored = approvals::ApprovalRequest::find(app.db(), request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ApprovalStatus::Approved);
}
