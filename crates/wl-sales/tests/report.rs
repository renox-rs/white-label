//! BL-SAL-21 (#140): the acquisition report: leads, conversion, cost per
//! lead, CAC and ROAS per source, campaign, channel, brand, branch and month,
//! within the viewer's scope, and its CSV. The scope rules are checked on the
//! computation with a branch-scoped viewer, since a test cannot build the
//! roles of every branch; the pages are checked as a global user.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::sales::{Attribute, FirstSale, campaign_attribution};
use wl_kernel::{PartyId, Subject};
use wl_sales::costs::{self, CostInput};
use wl_sales::permissions as perm;
use wl_sales::report::{self, Filter, GroupBy, Viewer};
use wl_sales::{
    Campaign, CostEntry, Lead, LeadSource, MarketingChannel, NewLead, Purpose, WlSales,
};

use support::{Booted, boot_events, boot_events_with};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

async fn source_id(t: &Booted, code: &str) -> i64 {
    LeadSource::by_code(t.db(), code).await.unwrap().unwrap().id
}

/// Active campaign `SPRING26` for *paid ads*, in March 2026, no budget.
async fn c1(t: &Booted) -> Campaign {
    let campaign = Campaign {
        name: "Spring".into(),
        code: "SPRING26".into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::Meta),
        lead_source_id: Some(source_id(t, "paid_ads").await),
        brand: Some("Palm".into()),
        owner_id: Some(1),
        start_date: Some(day(3, 1)),
        end_date: Some(day(3, 31)),
        ..Campaign::default()
    }
    .store(t.db())
    .await
    .unwrap()
    .unwrap();
    Campaign::query()
        .where_(Campaign::ID.eq(campaign.id))
        .update(t.db(), &[("status", &"active")])
        .await
        .unwrap();
    Campaign::find(t.db(), campaign.id).await.unwrap().unwrap()
}

/// An approved cost entry of `amount` fils, linked to a bill line when
/// `linked`.
async fn cost(t: &Booted, campaign: &Campaign, amount: i64, linked: bool) {
    let entry = costs::save(
        t.state(),
        CostInput {
            campaign_id: campaign.id,
            entry_date: day(3, 5),
            amount,
            description: "Ads".into(),
            requested_by: 1,
            manages_costs: true,
        },
    )
    .await
    .unwrap()
    .unwrap();
    if linked {
        CostEntry::query()
            .where_(CostEntry::ID.eq(entry.id))
            .update(
                t.db(),
                &[
                    ("supplier_bill_id", &Some(900_i64)),
                    ("supplier_bill_line_id", &Some(901_i64)),
                ],
            )
            .await
            .unwrap();
    }
}

/// `count` leads from the web form on `SPRING26` (or from `source` when the
/// code is `None`), of which `won` are won and `spam` closed as spam.
async fn leads(t: &Booted, count: usize, won: usize, spam: usize, source: Option<&str>) {
    for i in 0..count {
        let new = match source {
            Some(code) => NewLead::staff(None, &format!("Lead {i}"), code),
            None => NewLead::web_form(None, &format!("Lead {i}")).campaign_code("SPRING26"),
        };
        let lead = match wl_sales::lead::capture(t.state(), new, day(3, 10))
            .await
            .unwrap()
            .unwrap()
        {
            wl_sales::Captured::Created(lead) => lead,
            wl_sales::Captured::Noted(_) => panic!("no note expected"),
        };
        if i < won {
            Lead::query()
                .where_(Lead::ID.eq(lead.id))
                .update(
                    t.db(),
                    &[("status", &"won"), ("won_at", &Some(renox::db::now()))],
                )
                .await
                .unwrap();
        } else if i >= count - spam {
            Lead::query()
                .where_(Lead::ID.eq(lead.id))
                .update(
                    t.db(),
                    &[
                        ("status", &"lost"),
                        ("lost_reason", &Some("spam_or_invalid")),
                    ],
                )
                .await
                .unwrap();
        }
    }
}

/// `customers` parties acquired on C1, each with a first invoice of
/// `first` fils and, when `second` is set, a later one.
async fn customers(t: &Booted, customers: i64, first: i64, second: Option<i64>) {
    let attribution = campaign_attribution(t.state());
    for n in 0..customers {
        let party = PartyId(100 + n);
        attribution
            .attribute(
                t.state(),
                Attribute::party(party, "paid_ads").campaign("SPRING26"),
            )
            .await
            .unwrap();
        let sale = FirstSale::new(
            party,
            Subject::new("invoicing.invoice", 1000 + n),
            first,
            day(3, 25),
        );
        attribution
            .record_first_sale(t.state(), sale)
            .await
            .unwrap();
        if let Some(second) = second {
            let sale = FirstSale::new(
                party,
                Subject::new("invoicing.invoice", 2000 + n),
                second,
                day(6, 2),
            );
            attribution
                .record_first_sale(t.state(), sale)
                .await
                .unwrap();
        }
    }
}

async fn report_of(t: &Booted, by: GroupBy, filter: Filter) -> report::Report {
    let facts = report::load(t.db(), &wl_sales::SalesConfig::default())
        .await
        .unwrap();
    report::compute(&facts, &filter, by, &Viewer::all())
}

/// AC1: C1 with costs AED 5,000, 50 leads, 10 won, 8 new customers with first
/// sales of AED 20,000 excluding VAT: conversion 20.0%, cost per lead AED
/// 100.00, CAC AED 625.00, ROAS 4.00; AC7: AED 3,000 linked to bills and AED
/// 2,000 manual.
#[renox::test]
async fn sal21_the_campaign_row_shows_conversion_cost_per_lead_cac_and_roas() {
    let (t, _) = boot_events().await;
    let campaign = c1(&t).await;
    cost(&t, &campaign, 300_000, true).await;
    cost(&t, &campaign, 200_000, false).await;
    leads(&t, 50, 10, 0, None).await;
    customers(&t, 8, 250_000, None).await;

    let report = report_of(&t, GroupBy::Campaign, Filter::default()).await;
    assert_eq!(report.rows.len(), 1);
    let cells = report.rows[0].cells();
    assert_eq!(cells.leads, "50");
    assert_eq!(cells.won, "10");
    assert_eq!(cells.conversion, "20.0%");
    assert_eq!(cells.customers, "8");
    assert_eq!(cells.value, "20,000.00");
    assert_eq!(cells.cost, "5,000.00");
    assert_eq!(cells.cost_linked, "3,000.00");
    assert_eq!(cells.cost_manual, "2,000.00");
    assert_eq!(cells.cost_per_lead, "100.00");
    assert_eq!(cells.cac, "625.00");
    assert_eq!(cells.roas, "4.00");
}

/// AC2: the source *referral* with 12 leads and no campaign shows leads and
/// conversion and "—" for cost per lead, CAC and ROAS.
#[renox::test]
async fn sal21_a_source_without_a_campaign_has_no_cost_ratios() {
    let (t, _) = boot_events().await;
    let campaign = c1(&t).await;
    cost(&t, &campaign, 500_000, false).await;
    leads(&t, 12, 3, 0, Some("referral")).await;
    let referral = source_id(&t, "referral").await;
    let report = report_of(
        &t,
        GroupBy::Source,
        Filter {
            source: Some(referral),
            ..Filter::default()
        },
    )
    .await;
    assert_eq!(report.rows.len(), 1);
    let cells = report.rows[0].cells();
    assert_eq!(cells.leads, "12");
    assert_eq!(cells.conversion, "25.0%");
    assert_eq!(
        (
            cells.cost.as_str(),
            cells.cost_per_lead.as_str(),
            cells.cac.as_str(),
            cells.roas.as_str()
        ),
        ("—", "—", "—", "—")
    );
}

/// AC3: with the attribution window of 12 months and those customers' sales
/// of the first 12 months totalling AED 45,000, ROAS is 9.00.
#[renox::test]
async fn sal21_the_attribution_window_changes_the_converted_value() {
    let (t, _) = boot_events_with(WlSales::new().attribution_window_months(Some(12))).await;
    let campaign = c1(&t).await;
    cost(&t, &campaign, 500_000, false).await;
    leads(&t, 50, 10, 0, None).await;
    customers(&t, 8, 250_000, Some(312_500)).await;
    let facts = report::load(t.db(), &wl_sales::config::config_of(t.state()))
        .await
        .unwrap();
    let report = report::compute(
        &facts,
        &Filter::default(),
        GroupBy::Campaign,
        &Viewer::all(),
    );
    let cells = report.rows[0].cells();
    assert_eq!(cells.value, "45,000.00");
    assert_eq!(cells.roas, "9.00");
    // Without the window only the first sale counts.
    let plain = report::load(t.db(), &wl_sales::SalesConfig::default())
        .await
        .unwrap();
    let report = report::compute(
        &plain,
        &Filter::default(),
        GroupBy::Campaign,
        &Viewer::all(),
    );
    assert_eq!(report.rows[0].cells().roas, "4.00");
}

/// AC6: of C1's 50 leads 5 are lost as *spam or invalid*: 45 leads and cost
/// per lead AED 111.11.
#[renox::test]
async fn sal21_spam_leads_are_left_out_of_the_counts() {
    let (t, _) = boot_events().await;
    let campaign = c1(&t).await;
    cost(&t, &campaign, 500_000, false).await;
    leads(&t, 50, 10, 5, None).await;
    let report = report_of(&t, GroupBy::Campaign, Filter::default()).await;
    let cells = report.rows[0].cells();
    assert_eq!(cells.leads, "45");
    assert_eq!(cells.cost_per_lead, "111.11");
}

/// AC8: a first sale of AED 2,000 credited by AED 500 adds AED 1,500 to the
/// converted value.
#[renox::test]
async fn sal21_a_credit_note_lowers_the_converted_value() {
    let (t, _) = boot_events().await;
    c1(&t).await;
    customers(&t, 1, 200_000, None).await;
    wl_sales::acquisition::credit_sale(t.state(), &Subject::new("invoicing.invoice", 1000), 50_000)
        .await
        .unwrap();
    let report = report_of(&t, GroupBy::Campaign, Filter::default()).await;
    assert_eq!(report.rows[0].cells().value, "1,500.00");
}

/// Periods: the cost of March and a customer of April fall in different
/// months, and the month filter keeps the facts of its month.
#[renox::test]
async fn sal21_costs_and_customers_are_counted_in_their_own_months() {
    let (t, _) = boot_events().await;
    let campaign = c1(&t).await;
    cost(&t, &campaign, 100_000, false).await;
    let attribution = campaign_attribution(t.state());
    attribution
        .attribute(
            t.state(),
            Attribute::party(PartyId(100), "paid_ads").campaign("SPRING26"),
        )
        .await
        .unwrap();
    let sale = FirstSale::new(
        PartyId(100),
        Subject::new("invoicing.invoice", 1),
        80_000,
        day(4, 2),
    );
    attribution
        .record_first_sale(t.state(), sale)
        .await
        .unwrap();
    let by_month = report_of(&t, GroupBy::Month, Filter::default()).await;
    assert_eq!(by_month.rows.len(), 2);
    let march = report_of(&t, GroupBy::Campaign, Filter::default().month(2026, 3)).await;
    assert_eq!(march.rows[0].customers, 0);
    assert_eq!(march.rows[0].cells().cost, "1,000.00");
    assert_eq!(march.rows[0].cells().cac, "—");
    let april = report_of(&t, GroupBy::Campaign, Filter::default().month(2026, 4)).await;
    assert_eq!(april.rows[0].customers, 1);
    assert_eq!(april.rows[0].cells().cost, "—");
}

/// AC4: a user scoped to branch B1 does not see B2's leads; for the
/// entity-wide campaign C9 (AED 1,000) the B1 user sees C9's B1 leads with
/// cost "—", and a user who sees every branch sees AED 1,000.
#[renox::test]
async fn sal21_a_branch_scoped_user_sees_their_leads_and_no_entity_wide_cost() {
    let (t, _) = boot_events().await;
    let campaign = c1(&t).await;
    cost(&t, &campaign, 100_000, false).await;
    let b1 = Subject::new("test.branch", 1);
    let b2 = Subject::new("test.branch", 2);
    for branch in [&b1, &b2] {
        let new = NewLead::web_form(None, "Omar")
            .campaign_code("SPRING26")
            .branch(branch.clone());
        wl_sales::lead::capture(t.state(), new, day(3, 10))
            .await
            .unwrap()
            .unwrap();
    }
    let facts = report::load(t.db(), &wl_sales::SalesConfig::default())
        .await
        .unwrap();
    let scoped = report::compute(
        &facts,
        &Filter::default(),
        GroupBy::Campaign,
        &Viewer::scoped(vec![b1.clone()]),
    );
    assert_eq!(scoped.rows[0].leads, 1);
    let cells = scoped.rows[0].cells();
    assert_eq!(
        (
            cells.cost.as_str(),
            cells.cost_per_lead.as_str(),
            cells.cac.as_str(),
            cells.roas.as_str()
        ),
        ("—", "—", "—", "—")
    );
    let all = report::compute(
        &facts,
        &Filter::default(),
        GroupBy::Campaign,
        &Viewer::all(),
    );
    assert_eq!(all.rows[0].leads, 2);
    assert_eq!(all.rows[0].cells().cost, "1,000.00");
    assert_eq!(all.rows[0].cells().cost_per_lead, "500.00");
    assert_eq!(facts.branches().len(), 2);
}

/// AC5: the report exports to CSV with the numbers of the page; a user with
/// no role for it gets a 403.
#[renox::test]
async fn sal21_the_page_and_the_csv_show_the_same_numbers() {
    let (t, _) = boot_events().await;
    let campaign = c1(&t).await;
    cost(&t, &campaign, 300_000, true).await;
    cost(&t, &campaign, 200_000, false).await;
    leads(&t, 50, 10, 0, None).await;
    customers(&t, 8, 250_000, None).await;

    let nobody = t.user("Nobody", &[]).await;
    t.app.acting_as(&nobody);
    t.app
        .get("/sales/acquisition-report")
        .await
        .assert_forbidden();
    t.app
        .get("/sales/acquisition-report.csv")
        .await
        .assert_forbidden();

    let owner = t.user("Owner", &[perm::ACQUISITION_REPORTS_VIEW]).await;
    t.app.acting_as(&owner);
    t.app
        .get("/sales/acquisition-report?group=campaign")
        .await
        .assert_ok()
        .assert_see("SPRING26")
        .assert_see("20.0%")
        .assert_see("625.00")
        .assert_see("4.00");
    t.app
        .get("/sales/acquisition-report.csv?group=campaign")
        .await
        .assert_ok()
        .assert_see("SPRING26,50,10,20.0%,8,20000.00,5000.00,3000.00,2000.00,100.00,625.00,4.00");
    // The filters narrow it: another source has nothing.
    t.app
        .get("/sales/acquisition-report?group=campaign&source=referral")
        .await
        .assert_ok()
        .assert_dont_see("SPRING26");
}
