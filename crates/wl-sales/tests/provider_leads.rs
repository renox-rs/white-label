//! BL-SAL-27 (#470): a vertical's own lead records reach the acquisition
//! report through the kernel's lead-capture contract
//! (`wl_kernel::lead_records`). The provider here is a test module standing
//! in for 01's enquiries; the real one is checked in the host tests.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::lead_records::{LeadRecord, LeadRecords, LeadRecordsFuture, register_lead_records};
use wl_sales::costs::{self, CostInput};
use wl_sales::permissions as perm;
use wl_sales::report::{self, Filter, GroupBy, Viewer};
use wl_sales::{Campaign, LeadSource, NewLead, Purpose, SalesConfig};

use support::{Booted, boot, boot_with};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

/// Twelve enquiries on `BAYUTOCT` in October 2026: the last one closed as a
/// duplicate, the first two won with their leases activated in October.
struct Enquiries;

impl LeadRecords for Enquiries {
    fn records<'a>(
        &'a self,
        _db: &'a Db,
        _from: Option<NaiveDate>,
        _to: Option<NaiveDate>,
    ) -> LeadRecordsFuture<'a, Vec<LeadRecord>> {
        Box::pin(async move {
            Ok((0..12)
                .map(|i| LeadRecord {
                    subject: Some(wl_kernel::Subject::new("test.enquiry", i + 1)),
                    source: Some("paid_ads".into()),
                    campaign: Some("bayutoct".into()),
                    created: day(10, 1 + u32::try_from(i).unwrap()),
                    won_on: (i < 2).then(|| day(10, 20)),
                    excluded: i == 11,
                    ..LeadRecord::default()
                })
                .collect())
        })
    }
}

/// The vertical's module: it registers its enquiries as a lead provider.
struct Vertical;

impl Module for Vertical {
    fn name(&self) -> &'static str {
        "test_vertical"
    }

    fn register(&self, app: &mut Registry) {
        register_lead_records(app, "test.enquiries", "Enquiries", Enquiries);
    }
}

/// An active acquisition campaign `code` with an approved cost of `cost` fils
/// entered on 5 October 2026 (none when 0).
async fn campaign(t: &Booted, name: &str, code: &str, cost: i64) -> Campaign {
    let stored = Campaign {
        name: name.into(),
        code: code.into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some("other_online".into()),
        owner_id: Some(1),
        start_date: Some(day(10, 1)),
        end_date: Some(day(10, 31)),
        ..Campaign::default()
    }
    .store(t.db())
    .await
    .unwrap()
    .unwrap();
    Campaign::query()
        .where_(Campaign::ID.eq(stored.id))
        .update(t.db(), &[("status", &"active")])
        .await
        .unwrap();
    if cost > 0 {
        costs::save(
            t.state(),
            CostInput {
                campaign_id: stored.id,
                entry_date: day(10, 5),
                amount: cost,
                description: "Listing fee".into(),
                requested_by: 1,
                manages_costs: true,
            },
        )
        .await
        .unwrap()
        .unwrap();
    }
    Campaign::find(t.db(), stored.id).await.unwrap().unwrap()
}

/// The October 2026 report by campaign.
async fn october(t: &Booted) -> report::Report {
    let facts = report::load_with_providers(t.state(), &SalesConfig::default())
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
async fn the_enquiries_of_a_campaign_count_as_its_leads_without_the_duplicate() {
    let t = boot_with(Vertical).await;
    let bayut = campaign(&t, "Bayut October", "BAYUTOCT", 300_000).await;
    let r = october(&t).await;
    let row = row_of(&r, &bayut);
    assert_eq!(row.leads, 11);
    let cells = row.cells();
    assert_eq!(cells.cost, "3,000.00");
    assert_eq!(cells.cost_per_lead, "272.73");
    // Two converted with their leases activated in October.
    assert_eq!(row.won, 2);
    assert_eq!(cells.conversion, "18.2%");
    // The source comes from the record.
    let paid_ads = LeadSource::by_code(t.db(), "paid_ads")
        .await
        .unwrap()
        .unwrap();
    let by_source = report::compute(
        &report::load_with_providers(t.state(), &SalesConfig::default())
            .await
            .unwrap(),
        &Filter::default().month(2026, 10),
        GroupBy::Source,
        &Viewer::all(),
    );
    let source_row = by_source
        .rows
        .iter()
        .find(|row| row.key.value.as_deref() == Some(paid_ads.id.to_string().as_str()))
        .expect("the paid ads row");
    assert_eq!(source_row.leads, 11);
    // Nothing was copied into wl-sales.
    assert_eq!(wl_sales::Lead::query().count(t.db()).await.unwrap(), 0);
}

#[renox::test]
async fn a_campaign_without_enquiries_shows_zero_and_no_cost_per_lead() {
    let t = boot_with(Vertical).await;
    let quiet = campaign(&t, "Signboards October", "SIGNOCT", 100_000).await;
    let r = october(&t).await;
    let row = row_of(&r, &quiet);
    assert_eq!(row.leads, 0);
    assert_eq!(row.cells().cost_per_lead, "—");
}

#[renox::test]
async fn the_page_names_the_leads_as_the_vertical_does() {
    let t = boot_with(Vertical).await;
    campaign(&t, "Bayut October", "BAYUTOCT", 300_000).await;
    let viewer = t.user("Rana", &[perm::ACQUISITION_REPORTS_VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .get("/sales/acquisition-report?group=campaign&month=2026-10&campaign=BAYUTOCT")
        .await
        .assert_ok()
        .assert_see("Enquiries")
        .assert_see("272.73")
        .assert_see("18.2%");
}

#[renox::test]
async fn both_kinds_are_counted_when_the_vertical_also_has_wl_sales_leads() {
    let t = boot_with(Vertical).await;
    let bayut = campaign(&t, "Bayut October", "BAYUTOCT", 300_000).await;
    wl_sales::lead::capture(
        t.state(),
        NewLead::web_form(None, "Web lead").campaign_code("BAYUTOCT"),
        day(10, 15),
    )
    .await
    .unwrap()
    .unwrap();
    let facts = report::load_with_providers(t.state(), &SalesConfig::default())
        .await
        .unwrap();
    assert_eq!(facts.own_leads, 1);
    assert_eq!(facts.provider_labels, ["Enquiries"]);
    let r = report::compute(
        &facts,
        &Filter::default(),
        GroupBy::Campaign,
        &Viewer::all(),
    );
    assert_eq!(row_of(&r, &bayut).leads, 12);
}

#[renox::test]
async fn without_the_vertical_the_report_shows_the_wl_sales_leads_only() {
    let t = boot().await;
    let bayut = campaign(&t, "Bayut October", "BAYUTOCT", 300_000).await;
    wl_sales::lead::capture(
        t.state(),
        NewLead::web_form(None, "Web lead").campaign_code("BAYUTOCT"),
        day(10, 15),
    )
    .await
    .unwrap()
    .unwrap();
    let facts = report::load_with_providers(t.state(), &SalesConfig::default())
        .await
        .unwrap();
    assert!(facts.provider_labels.is_empty());
    assert_eq!(facts.leads.len(), 1);
    let r = report::compute(
        &facts,
        &Filter::default(),
        GroupBy::Campaign,
        &Viewer::all(),
    );
    assert_eq!(row_of(&r, &bayut).leads, 1);
    let viewer = t.user("Rana", &[perm::ACQUISITION_REPORTS_VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .get("/sales/acquisition-report?group=campaign")
        .await
        .assert_ok()
        .assert_dont_see("Enquiries");
}
