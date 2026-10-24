//! BL-SAL-27 (#470) on the Property client with the real modules: 01's
//! enquiries are the campaign's leads in the acquisition report, through the
//! kernel's lead-capture contract. apps/property registers them; wl-sales
//! reads every registered provider.
//!
//! Fixtures: an enquiry is recorded today by `enquiry::record`; its day of
//! receipt is then set in October 2026 so the month is fixed. The won day is
//! the activation day of the tenant's first lease, set on a draft lease
//! (only the day is read; the lease lifecycle is covered elsewhere).

mod support;

use host::roles::MARKETING_MANAGER;
use property::enquiry::{self, Enquiry, NewEnquiry};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_sales::costs::{self, CostInput};
use wl_sales::report::{self, Filter, GroupBy, Viewer};
use wl_sales::{Campaign, Purpose};

fn october(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

/// An active campaign `code` with an approved cost of `cost` fils.
async fn campaign(app: &TestApp, owner: &User, name: &str, code: &str, cost: i64) -> Campaign {
    let stored = Campaign {
        name: name.into(),
        code: code.into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some("listing_portals".into()),
        owner_id: Some(owner.id),
        start_date: Some(october(1)),
        end_date: Some(october(31)),
        ..Campaign::default()
    }
    .store(app.db())
    .await
    .unwrap()
    .unwrap();
    Campaign::query()
        .where_(Campaign::ID.eq(stored.id))
        .update(app.db(), &[("status", &"active")])
        .await
        .unwrap();
    costs::save(
        app.state(),
        CostInput {
            campaign_id: stored.id,
            entry_date: october(5),
            amount: cost,
            description: "Portal listings".into(),
            requested_by: owner.id,
            manages_costs: true,
        },
    )
    .await
    .unwrap()
    .unwrap();
    Campaign::find(app.db(), stored.id).await.unwrap().unwrap()
}

/// An enquiry on `code` received on 1 + `i` October 2026.
async fn enquiry_on(app: &TestApp, code: &str, i: u32) -> Enquiry {
    let recorded = enquiry::record(
        app.db(),
        NewEnquiry {
            prospect_name: format!("Prospect {i}"),
            contact: format!("+97150000{i:04}"),
            source: "paid_ads".into(),
            campaign: Some(code.into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    Enquiry::query()
        .where_(Enquiry::ID.eq(recorded.id))
        .update(app.db(), &[("received_on", &october(1 + i))])
        .await
        .unwrap();
    Enquiry::find_or_404(app.db(), recorded.id).await.unwrap()
}

/// Converts `enquiry` to a new tenant whose first lease was activated on
/// 20 October 2026.
async fn won(app: &TestApp, enquiry: &Enquiry, n: u32) {
    let party = tenant(app, &format!("Tenant {n}")).await;
    enquiry::convert(app.state(), enquiry.id, party).await.unwrap();
    let unit = unit_of(app, &format!("BY{n}"), "101", property::UnitUse::Residential).await;
    let lease = draft_lease(app, &unit, party, 0).await;
    property::Lease::query()
        .where_(property::Lease::ID.eq(lease.id))
        .update(app.db(), &[("activated_on", &Some(october(20)))])
        .await
        .unwrap();
}

async fn october_report(app: &TestApp) -> report::Report {
    let config = wl_sales::config::config_of(app.state());
    let facts = report::load_with_providers(app.state(), &config)
        .await
        .unwrap();
    report::compute(
        &facts,
        &Filter::default().month(2026, 10),
        GroupBy::Campaign,
        &Viewer::all(),
    )
}

fn row_of(r: &report::Report, campaign: &Campaign) -> report::Row {
    r.rows
        .iter()
        .find(|row| row.key.value.as_deref() == Some(campaign.id.to_string().as_str()))
        .cloned()
        .unwrap_or_else(|| panic!("a row for {}", campaign.code))
}

#[renox::test]
async fn the_enquiries_of_bayut_october_are_its_leads_won_and_cost_per_lead() {
    let app = booted().await;
    let owner = person(&app, "Mona", MARKETING_MANAGER).await;
    let bayut = campaign(&app, &owner, "Bayut October", "BAYUTOCT", 300_000).await;
    let quiet = campaign(&app, &owner, "Signboards October", "SIGNOCT", 100_000).await;
    let mut enquiries = Vec::new();
    for i in 0..12 {
        enquiries.push(enquiry_on(&app, "BAYUTOCT", i).await);
    }
    // One is a duplicate.
    let duplicate = enquiry::close(app.db(), enquiries[11].id, "duplicate")
        .await
        .unwrap();
    assert_eq!(duplicate.closed_reason.as_deref(), Some("duplicate"));
    // Two converted, their leases activated in October.
    won(&app, &enquiries[0], 1).await;
    won(&app, &enquiries[1], 2).await;

    let r = october_report(&app).await;
    let row = row_of(&r, &bayut);
    assert_eq!(row.leads, 11, "the duplicate is left out");
    assert_eq!(row.won, 2, "a converted enquiry is counted once");
    let cells = row.cells();
    assert_eq!(cells.cost_per_lead, "272.73");
    assert_eq!(cells.conversion, "18.2%");
    // The campaign without enquiries.
    let none = row_of(&r, &quiet);
    assert_eq!(none.leads, 0);
    assert_eq!(none.cells().cost_per_lead, "—");
    // The enquiries were not copied into wl-sales.
    assert_eq!(wl_sales::Lead::query().count(app.db()).await.unwrap(), 0);

    // The page calls them Enquiries.
    app.acting_as(&owner);
    app.get("/sales/acquisition-report?group=campaign&month=2026-10&campaign=BAYUTOCT")
        .await
        .assert_ok()
        .assert_see("Enquiries")
        .assert_see("272.73")
        .assert_see("18.2%");
}
