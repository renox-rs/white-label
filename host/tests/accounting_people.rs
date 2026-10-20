//! BL-ACC-23 (#347), the people side of the 04 Accounting client with the real
//! modules of the host: the lead sources and campaign figures (`SAL.CRM.02`,
//! `SAL.ACL.07`), the onboarding checklists and their reminders
//! (`HRM.ONB.01`, `HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`) and the SOP
//! library (`DOC.RUL.04`).

mod support;

use host::roles::{ADMIN, ENGAGEMENT_MANAGER, MARKETING, MARKETING_MANAGER, TAX_SPECIALIST};
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_documents::acknowledgements::{AckStatus, Acknowledgements, SopAcknowledgement};
use wl_documents::{Document, Sop, SopReader};
use wl_hr::{
    Employee, EmployeeStatus, NewEmployee, Onboarding, OnboardingStep, OnboardingTemplate,
    employees, reminders,
};
use wl_kernel::approvals::{self, ApprovalStatus};
use wl_kernel::documents::today;
use wl_kernel::sales::{Attribute, FirstSale, campaign_attribution};
use wl_kernel::{PartyId, Subject};
use wl_sales::costs::{self, CostInput};
use wl_sales::report::{self, Filter, GroupBy, Viewer};
use wl_sales::{
    Campaign, CostEntry, CostStatus, Lead, LeadSource, MarketingChannel, NewLead, Purpose,
};

const CODE: &str = "SEARCH26";

/// The search-ads campaign, active, AED 5,000 budget.
async fn search_campaign(app: &TestApp, owner: &User) -> Campaign {
    let channel = accounting::seed::CAMPAIGN_CHANNELS
        .iter()
        .find(|(name, _)| *name == "Search ads")
        .expect("the firm offers the search ads channel")
        .1;
    assert_eq!(channel, "google");
    let source = LeadSource::by_code(app.db(), "paid_ads")
        .await
        .unwrap()
        .unwrap();
    let campaign = Campaign {
        name: "Search ads autumn".into(),
        code: CODE.into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::Google),
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

/// The sources of the spec are offered, walk-in and aggregator are off.
#[renox::test]
async fn the_lead_sources_are_the_seven_of_the_spec() {
    let app = accounting_booted().await;
    for code in accounting::seed::LEAD_SOURCES {
        let source = LeadSource::by_code(app.db(), code)
            .await
            .unwrap()
            .expect(code);
        assert!(source.active, "{code}");
    }
    for code in ["walk_in", "aggregator"] {
        let source = LeadSource::by_code(app.db(), code)
            .await
            .unwrap()
            .expect(code);
        assert!(!source.active, "{code} is not used by an accounting firm");
    }
}

/// A search-ads campaign reports leads, conversion, cost per lead, CAC and
/// ROAS; a cost entry over the budget waits for the marketing manager.
#[renox::test]
async fn the_search_ads_campaign_reports_its_figures_and_an_overspend_waits_for_a_manager() {
    let app = accounting_booted().await;
    let manager = person(&app, "Mona", MARKETING_MANAGER).await;
    let staff = person(&app, "Marco", MARKETING).await;
    let campaign = search_campaign(&app, &manager).await;
    let input = |amount| CostInput {
        campaign_id: campaign.id,
        entry_date: day(-5),
        amount,
        description: "Search ads".into(),
        requested_by: staff.id,
        manages_costs: false,
    };
    for amount in [200_000, 100_000] {
        let entry = costs::save(app.state(), input(amount))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.status, CostStatus::Approved);
    }
    for i in 0..20 {
        let new = NewLead::staff(None, &format!("Enquirer {i}"), "paid_ads").campaign_code(CODE);
        let wl_sales::Captured::Created(lead) = wl_sales::lead::capture(app.state(), new, day(-3))
            .await
            .unwrap()
            .unwrap()
        else {
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
    let cells = report.rows[0].cells();
    assert_eq!(
        (
            cells.leads.as_str(),
            cells.won.as_str(),
            cells.conversion.as_str()
        ),
        ("20", "4", "20.0%")
    );
    assert_eq!(cells.cost_per_lead, "150.00");
    assert_eq!(cells.cac, "1,000.00");
    assert_eq!(cells.roas, "10.00");

    // AED 3,000 spent of 5,000: AED 2,500 more is over the budget.
    let over = costs::save(app.state(), input(250_000))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(over.status, CostStatus::PendingApproval);
    let request = over.approval_request_id.expect("a request");
    assert!(
        approvals::approve(app.state(), request, &staff, "")
            .await
            .is_err()
    );
    approvals::approve(app.state(), request, &manager, "ok")
        .await
        .unwrap();
    let approved = CostEntry::find(app.db(), over.id).await.unwrap().unwrap();
    assert_eq!(approved.status, CostStatus::Approved);
    let stored = approvals::ApprovalRequest::find(app.db(), request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ApprovalStatus::Approved);
}

/// A tax specialist hired today, with an account: the employee, their user
/// and the onboarding.
async fn hired_specialist(app: &TestApp, name: &str) -> (Employee, User, Onboarding) {
    let user = person(app, name, TAX_SPECIALIST).await;
    let party = tenant(app, name).await;
    let employee = employees::create(
        app.state(),
        NewEmployee::new(party, "tax_specialist", today())
            .user(user.id)
            .status(EmployeeStatus::Onboarding),
    )
    .await
    .unwrap();
    let onboarding = Onboarding::query()
        .where_(Onboarding::EMPLOYEE_ID.eq(employee.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("hiring starts the onboarding");
    (employee, user, onboarding)
}

fn at(day: NaiveDate, plus: u64) -> NaiveDate {
    day.checked_add_days(Days::new(plus)).unwrap()
}

/// Hiring a tax specialist creates the tax specialist checklist with tasks to
/// the admin, the manager and the employee; the role is active before it is
/// complete; an open step is reminded and escalated (`HRM.RUL.17`).
#[renox::test]
async fn hiring_a_tax_specialist_creates_the_checklist_and_its_reminders_run() {
    let app = accounting_booted().await;
    let admin = person(&app, "Adel", ADMIN).await;
    let manager = person(&app, "Mona", ENGAGEMENT_MANAGER).await;
    let (employee, specialist, onboarding) = hired_specialist(&app, "Tariq").await;

    let template = OnboardingTemplate::query()
        .where_(OnboardingTemplate::NAME_EN.eq("Tax specialist onboarding"))
        .first(app.db())
        .await
        .unwrap()
        .expect("seeded");
    assert!(!template.blocks_activation);
    let templates = onboarding.templates(app.db()).await.unwrap();
    assert!(templates.iter().any(|t| t.template_id == template.id));
    let steps = onboarding.steps(app.db()).await.unwrap();
    for s in &steps {
        let who = s.assignees(app.db()).await.unwrap();
        match s.owner.as_str() {
            "employee" => assert_eq!(who, [specialist.id], "{}", s.title_en),
            "manager" => assert_eq!(who, [manager.id], "{}", s.title_en),
            _ => assert!(who.contains(&admin.id), "{}: {who:?}", s.title_en),
        }
    }
    assert!(
        steps.iter().any(|s| s.owner == "manager") && steps.iter().any(|s| s.owner == "employee")
    );

    // Active before the checklist is complete.
    app.acting_as(&admin);
    app.htmx()
        .post(
            &format!("/hr/employees/{}/transitions/activate", employee.id),
            &[("reason", "")],
        )
        .await
        .assert_status(303);
    assert_eq!(
        Employee::find_or_404(app.db(), employee.id)
            .await
            .unwrap()
            .status,
        EmployeeStatus::Active
    );
    let still = Onboarding::find_or_404(app.db(), onboarding.id)
        .await
        .unwrap();
    assert_ne!(still.status, wl_hr::OnboardingStatus::Completed);

    // HRM.RUL.17: reminded on the first run after the due date, then every
    // second day; after 5 days the admin and the manager are told once.
    let own = steps
        .iter()
        .filter(|s| s.owner == "employee" && s.required)
        .min_by_key(|s| s.due_date)
        .expect("a step of the employee");
    let due = own.due_date;
    let state_of = || async { OnboardingStep::find_or_404(app.db(), own.id).await.unwrap() };
    reminders::run(app.state(), due).await.unwrap();
    assert_eq!(state_of().await.last_reminded_on, None);
    reminders::run(app.state(), at(due, 1)).await.unwrap();
    assert_eq!(state_of().await.last_reminded_on, Some(at(due, 1)));
    reminders::run(app.state(), at(due, 2)).await.unwrap();
    assert_eq!(state_of().await.last_reminded_on, Some(at(due, 1)));
    reminders::run(app.state(), at(due, 3)).await.unwrap();
    assert_eq!(state_of().await.last_reminded_on, Some(at(due, 3)));
    reminders::run(app.state(), at(due, 5)).await.unwrap();
    assert_eq!(state_of().await.escalated_on, Some(at(due, 5)));
    let escalate = "hr.onboarding_step_escalated";
    assert!(!messages_to(&app, escalate, &manager).await.is_empty());
    assert!(!messages_to(&app, escalate, &admin).await.is_empty());
}

/// Uploads a new version of the SOP `sop` as `by` and accepts it.
async fn publish_version(app: &TestApp, by: &User, sop: &Sop) -> Document {
    app.acting_as(by);
    let uploaded = app
        .post(
            &format!("/probe/sops/{}/versions", sop.id),
            &[("file_name", "aml.pdf")],
        )
        .await;
    uploaded.assert_ok();
    let version = Document::find_or_404(app.db(), uploaded.text().parse().unwrap())
        .await
        .unwrap();
    app.htmx()
        .post(
            &format!("/documents/sop-versions/{}/accept", version.id),
            &[],
        )
        .await
        .assert_ok();
    Document::find_or_404(app.db(), version.id).await.unwrap()
}

/// A new version of an SOP owned by the MLRO asks every required reader to
/// acknowledge again within 7 days; no one else acknowledges for them.
#[renox::test]
async fn a_new_mlro_sop_version_asks_the_readers_to_acknowledge_again_within_seven_days() {
    let app = accounting_booted().await;
    let owner = person(&app, "Mariam", host::roles::COMPLIANCE).await;
    // The MLRO also manages SOPs (the company admin right).
    owner.assign_role(app.db(), ADMIN).await.unwrap();
    let (employee, specialist, _) = hired_specialist(&app, "Tariq").await;
    let sop = Sop::query()
        .where_(Sop::CODE.eq("SOP-ACC-AML-CDD"))
        .first(app.db())
        .await
        .unwrap()
        .expect("seeded");
    assert_eq!(sop.owner_role, "compliance");
    assert_eq!(sop.ack_due_days, 7);
    let readers: Vec<String> = SopReader::query()
        .where_(SopReader::SOP_ID.eq(sop.id))
        .get(app.db())
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.role_key)
        .collect();
    assert!(
        readers.contains(&"tax_specialist".to_owned()) && readers.contains(&"partner".to_owned())
    );
    let manager_sop = Sop::query()
        .where_(Sop::CODE.eq("SOP-ACC-RETURNS"))
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(manager_sop.owner_role, "engagement_manager");

    let first = publish_version(&app, &owner, &sop).await;
    let acks = Acknowledgements::of_version(app.db(), sop.id, first.id)
        .await
        .unwrap();
    let mine = acks
        .iter()
        .find(|a| a.employee_id == employee.id)
        .expect("asked");
    assert_eq!(mine.status, AckStatus::Pending);
    assert_eq!(mine.due_date, at(today(), 7));
    app.acting_as(&owner);
    let refused = app
        .htmx()
        .post(
            &format!("/documents/acknowledgements/{}/acknowledge", mine.id),
            &[],
        )
        .await;
    assert_eq!(refused.status.as_u16(), 403);
    Acknowledgements::open(app.state(), mine.id, specialist.id, first.id)
        .await
        .unwrap()
        .unwrap();
    app.acting_as(&specialist);
    app.htmx()
        .post(
            &format!("/documents/acknowledgements/{}/acknowledge", mine.id),
            &[],
        )
        .await
        .assert_ok();
    let done = SopAcknowledgement::find(app.db(), mine.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.acknowledged_by, Some(specialist.id));

    let second = publish_version(&app, &owner, &sop).await;
    let again = Acknowledgements::of_version(app.db(), sop.id, second.id)
        .await
        .unwrap();
    let asked = again
        .iter()
        .find(|a| a.employee_id == employee.id)
        .expect("asked again");
    assert_eq!(asked.status, AckStatus::Pending);
    assert_eq!(asked.due_date, at(today(), 7));
}
