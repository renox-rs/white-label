//! BL-PRP-27 (#277), the people side of the Property client with the real
//! modules of the host: the campaign of the listing portals (its source and
//! campaign reach the tenant party, its figures, a cost over the budget waits
//! for the marketing manager) and the staff (hiring a leasing agent starts
//! the checklist for the admin, the line manager and the employee; the
//! reminders of `HRM.RUL.17`; a new version of an SOP asks its readers to
//! acknowledge again within 7 days, `DOC.RUL.04`; only the employee
//! acknowledges for themselves, `HRM.ACL.10`).
//!
//! Decided where the spec is silent: the *enquiries* of the campaign report are
//! the leads of wl-sales for the campaign (a Property enquiry row is not a
//! wl-sales lead, see `DEFERRED-G8b.md`), and *listing portals* is the
//! marketing channel `listing_portals` of the 01 seed (`seed::CAMPAIGN_CHANNELS`,
//! BL-SAL-28).

mod support;

use host::roles::{ADMIN, LEASING_AGENT, MARKETING, MARKETING_MANAGER, PROPERTY_MANAGER};
use property::enquiry::{self, NewEnquiry};
use property::seed::{CAMPAIGN_CHANNELS, LEAD_SOURCES};
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_documents::acknowledgements::{AckStatus, Acknowledgements, SopAcknowledgement};
use wl_documents::{Document, Sop};
use wl_hr::{
    Employee, EmployeeStatus, NewEmployee, Onboarding, OnboardingStep, employees, reminders,
};
use wl_kernel::approvals::{self, ApprovalStatus};
use wl_kernel::documents::today;
use wl_kernel::sales::{Attribute, FirstSale, campaign_attribution};
use wl_kernel::{PartyId, Subject};
use wl_sales::costs::{self, CostInput};
use wl_sales::report::{self, Filter, GroupBy, Viewer};
use wl_sales::{
    Campaign, CostEntry, CostStatus, Lead, LeadSource, NewLead, PartyAcquisition, Purpose,
};

const CODE: &str = "LISTING26";

/// The listing-portals campaign of the client, active, AED 5,000 budget.
async fn portals_campaign(app: &TestApp, owner: &User) -> Campaign {
    let channel = CAMPAIGN_CHANNELS
        .iter()
        .find(|(name, _)| *name == "Listing portals")
        .expect("the client offers the listing portals channel")
        .1;
    assert_eq!(channel, "listing_portals");
    let source = LeadSource::by_code(app.db(), "paid_ads")
        .await
        .unwrap()
        .unwrap();
    let campaign = Campaign {
        name: "Listing portals autumn".into(),
        code: CODE.into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(channel.into()),
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

/// The new enquiry form offers the sources of the spec and no other.
#[renox::test]
async fn a_new_enquiry_offers_the_eight_lead_sources_and_refuses_another() {
    let app = booted().await;
    assert_eq!(
        LEAD_SOURCES,
        [
            "referral", "walk_in", "website", "social", "paid_ads", "partner", "event", "other"
        ]
    );
    // The client's page: the form lists the sources, not the aggregator.
    let agent = person(&app, "Aya", LEASING_AGENT).await;
    app.acting_as(&agent);
    let page = app.get("/property/enquiries").await;
    page.assert_ok();
    let html = page.text();
    for code in LEAD_SOURCES {
        assert!(html.contains(code), "{code} is offered");
    }
    assert!(!html.contains("aggregator"));
    for code in LEAD_SOURCES {
        enquiry::record(
            app.db(),
            NewEnquiry {
                prospect_name: format!("Prospect {code}"),
                source: code.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    assert!(
        enquiry::record(
            app.db(),
            NewEnquiry {
                prospect_name: "Prospect".into(),
                source: "aggregator".into(),
                ..Default::default()
            },
        )
        .await
        .is_err()
    );
}

/// The source and the campaign of an enquiry reach the tenant's party when
/// the enquiry converts.
#[renox::test]
async fn an_enquiry_hands_its_source_and_campaign_to_the_tenant_party() {
    let app = booted().await;
    let owner = person(&app, "Mona", MARKETING_MANAGER).await;
    let campaign = portals_campaign(&app, &owner).await;
    let recorded = enquiry::record(
        app.db(),
        NewEnquiry {
            prospect_name: "Layla".into(),
            contact: "+971500000400".into(),
            source: "paid_ads".into(),
            campaign: Some(CODE.into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let layla = tenant(&app, "Layla Haddad").await;
    enquiry::convert(app.state(), recorded.id, layla)
        .await
        .unwrap();
    let acquisition = PartyAcquisition::of_party(app.db(), layla)
        .await
        .unwrap()
        .expect("the party has an acquisition");
    let source = LeadSource::by_code(app.db(), "paid_ads")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(acquisition.source_id, source.id);
    assert_eq!(acquisition.campaign_id, Some(campaign.id));
    assert_eq!(acquisition.set_from, "lead");
}

/// A campaign with the channel *listing portals*, a budget and cost entries:
/// 20 enquiries, 4 won, AED 3,000 of costs, 3 tenants whose first rent
/// invoice is AED 10,000: conversion 20.0%, cost per lead 150.00, CAC
/// 1,000.00 and ROAS 10.00.
#[renox::test]
async fn the_listing_portals_campaign_reports_enquiries_conversion_cost_per_lead_cac_and_roas() {
    let app = booted().await;
    let manager = person(&app, "Mona", MARKETING_MANAGER).await;
    let staff = person(&app, "Marco", MARKETING).await;
    let campaign = portals_campaign(&app, &manager).await;

    for amount in [200_000, 100_000] {
        let entry = costs::save(
            app.state(),
            CostInput {
                campaign_id: campaign.id,
                entry_date: day(-5),
                amount,
                description: "Portal listing fee".into(),
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

    // By channel the same row is the listing portals.
    let by_channel = report::compute(&facts, &Filter::default(), GroupBy::Channel, &Viewer::all());
    assert_eq!(by_channel.rows.len(), 1);
    assert_eq!(by_channel.rows[0].cells().leads, "20");
}

/// A cost entry that takes the campaign over its budget waits for the
/// marketing manager (`SAL.ACL.07`); the person who entered it cannot approve
/// it, the marketing manager can.
#[renox::test]
async fn a_cost_entry_over_the_budget_waits_for_the_marketing_manager() {
    let app = booted().await;
    let manager = person(&app, "Mona", MARKETING_MANAGER).await;
    let staff = person(&app, "Marco", MARKETING).await;
    let campaign = portals_campaign(&app, &manager).await;
    let input = |amount| CostInput {
        campaign_id: campaign.id,
        entry_date: day(-5),
        amount,
        description: "Portal listing fee".into(),
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
    // The marketing staff cannot approve their own entry; the manager can.
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
    assert_eq!(approved.approved_by, Some(manager.id));
    let stored = approvals::ApprovalRequest::find(app.db(), request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ApprovalStatus::Approved);
}

// ---- Onboarding and SOPs.

/// A leasing agent hired today at the building `subject`, with an account:
/// the employee, their user and the onboarding.
async fn hired_agent(app: &TestApp, subject: &Subject, name: &str) -> (Employee, User, Onboarding) {
    let user = person(app, name, LEASING_AGENT).await;
    let party = tenant(app, name).await;
    let employee = employees::create(
        app.state(),
        NewEmployee::new(party, "leasing_agent", today())
            .scope(subject.clone())
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

async fn step(app: &TestApp, onboarding: &Onboarding, title: &str) -> OnboardingStep {
    let steps = onboarding.steps(app.db()).await.unwrap();
    steps
        .into_iter()
        .find(|s| s.title_en == title)
        .unwrap_or_else(|| panic!("{title}"))
}

/// Hiring a leasing agent creates the leasing agent checklist: tasks to the
/// company admin, the line manager of the building and the employee; the role
/// is active before the checklist is complete.
#[renox::test]
async fn hiring_a_leasing_agent_creates_the_checklist_for_admin_manager_and_employee() {
    let app = booted().await;
    let admin = person(&app, "Adel", ADMIN).await;
    let (_tower, subject) = building(&app, "TWR-A").await;
    let (_other, _other_subject) = building(&app, "TWR-B").await;
    let scope = property::Building::scope_of(_tower.id);
    let manager = scoped_person(&app, "Pam", PROPERTY_MANAGER, &scope).await;
    let other_scope = property::Building::scope_of(_other.id);
    let other_manager = scoped_person(&app, "Omar", PROPERTY_MANAGER, &other_scope).await;
    let (employee, agent, onboarding) = hired_agent(&app, &subject, "Aya").await;

    let templates = onboarding.templates(app.db()).await.unwrap();
    let leasing = wl_hr::OnboardingTemplate::query()
        .where_(wl_hr::OnboardingTemplate::NAME_EN.eq("Leasing agent onboarding"))
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    assert!(templates.iter().any(|t| t.template_id == leasing.id));
    let account = step(&app, &onboarding, "System account and access").await;
    let handover = step(
        &app,
        &onboarding,
        "Availability, pricing and enquiry process",
    )
    .await;
    let mut employee_steps = 0;
    for s in onboarding.steps(app.db()).await.unwrap() {
        let who = s.assignees(app.db()).await.unwrap();
        match s.owner.as_str() {
            "employee" => {
                employee_steps += 1;
                assert_eq!(who, [agent.id], "{}", s.title_en);
            }
            "manager" => {
                assert_eq!(who, [manager.id], "{}", s.title_en);
                assert!(!who.contains(&other_manager.id));
            }
            _ => assert!(who.contains(&admin.id), "{}: {who:?}", s.title_en),
        }
    }
    assert!(employee_steps > 0);
    assert_eq!(handover.owner, "manager");
    assert_ne!(account.owner, "employee");

    // The role is active before the checklist is complete.
    let admin_user = admin;
    app.acting_as(&admin_user);
    app.htmx()
        .post(
            &format!("/hr/employees/{}/transitions/activate", employee.id),
            &[("reason", "")],
        )
        .await
        .assert_status(303);
    let activated = Employee::find_or_404(app.db(), employee.id).await.unwrap();
    assert_eq!(activated.status, EmployeeStatus::Active);
    let still_open = Onboarding::find_or_404(app.db(), onboarding.id)
        .await
        .unwrap();
    assert_ne!(
        still_open.status,
        wl_hr::OnboardingStatus::Completed,
        "the checklist is not complete yet"
    );
}

fn at(day: NaiveDate, plus: u64) -> NaiveDate {
    day.checked_add_days(Days::new(plus)).unwrap()
}

/// A required step still open on the first run after its due date reminds its
/// assignee, then every second day; after 5 days the admin and the line
/// manager are told once (`HRM.RUL.17`).
#[renox::test]
async fn an_open_onboarding_step_is_reminded_every_second_day_and_escalated_after_five() {
    let app = booted().await;
    let admin = person(&app, "Adel", ADMIN).await;
    let (_tower, subject) = building(&app, "TWR-A").await;
    let scope = property::Building::scope_of(_tower.id);
    let manager = scoped_person(&app, "Pam", PROPERTY_MANAGER, &scope).await;
    let (_, agent, onboarding) = hired_agent(&app, &subject, "Aya").await;
    // The employee's own step: due `n` days after the start.
    let own = onboarding
        .steps(app.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|s| s.owner == "employee" && s.required)
        .min_by_key(|s| s.due_date)
        .expect("a required step of the employee");
    let due = own.due_date;
    let remind = "hr.onboarding_step_reminder";
    let escalate = "hr.onboarding_step_escalated";

    let state_of = || async { OnboardingStep::find_or_404(app.db(), own.id).await.unwrap() };

    reminders::run(app.state(), due).await.unwrap();
    assert_eq!(state_of().await.last_reminded_on, None, "not yet overdue");
    reminders::run(app.state(), at(due, 1)).await.unwrap();
    assert_eq!(
        state_of().await.last_reminded_on,
        Some(at(due, 1)),
        "the first run after the due date"
    );
    let first = messages_to(&app, remind, &agent).await.len();
    assert!(first >= 1);
    reminders::run(app.state(), at(due, 2)).await.unwrap();
    assert_eq!(state_of().await.last_reminded_on, Some(at(due, 1)));
    reminders::run(app.state(), at(due, 3)).await.unwrap();
    assert_eq!(
        state_of().await.last_reminded_on,
        Some(at(due, 3)),
        "again 2 days later"
    );
    assert!(messages_to(&app, remind, &agent).await.len() > first);

    // Five days past the due date the admin and the line manager are told,
    // once.
    assert_eq!(state_of().await.escalated_on, None);
    reminders::run(app.state(), at(due, 5)).await.unwrap();
    assert_eq!(state_of().await.escalated_on, Some(at(due, 5)));
    let to_manager = messages_to(&app, escalate, &manager).await.len();
    let to_admin = messages_to(&app, escalate, &admin).await.len();
    assert!(to_manager >= 1 && to_admin >= 1);
    reminders::run(app.state(), at(due, 7)).await.unwrap();
    assert_eq!(state_of().await.escalated_on, Some(at(due, 5)), "once");
}

/// Uploads a new version of the SOP `sop` as `by` (a probe route: the upload
/// needs an `AuthUser`) and accepts it through the page route.
async fn publish_version(app: &TestApp, by: &User, sop: &Sop) -> Document {
    app.acting_as(by);
    let uploaded = app
        .post(
            &format!("/probe/sops/{}/versions", sop.id),
            &[("file_name", "leasing.pdf")],
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

/// A new version of an SOP owned by the property manager asks every required
/// reader to acknowledge again within 7 days (`DOC.RUL.04`); no one else can
/// acknowledge for the employee, and the employee does it in their own session
/// (`HRM.ACL.10`).
#[renox::test]
async fn a_new_sop_version_asks_the_readers_to_acknowledge_again_within_seven_days() {
    let app = booted().await;
    let (_tower, subject) = building(&app, "TWR-A").await;
    let scope = property::Building::scope_of(_tower.id);
    let manager = scoped_person(&app, "Pam", PROPERTY_MANAGER, &scope).await;
    // The owner role and the right to manage SOPs: the property manager who
    // is also a company admin.
    manager.assign_role(app.db(), ADMIN).await.unwrap();
    manager
        .assign_role(app.db(), PROPERTY_MANAGER)
        .await
        .unwrap();
    let (employee, agent, _) = hired_agent(&app, &subject, "Aya").await;
    let sop = Sop::query()
        .where_(Sop::CODE.eq("SOP-PRP-LEASING"))
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sop.owner_role, "property_manager");

    let first = publish_version(&app, &manager, &sop).await;
    let acks = Acknowledgements::of_version(app.db(), sop.id, first.id)
        .await
        .unwrap();
    let mine = acks
        .iter()
        .find(|a| a.employee_id == employee.id)
        .expect("the leasing agent is asked");
    assert_eq!(mine.status, AckStatus::Pending);
    assert_eq!(mine.due_date, at(today(), 7));

    // Nobody else acknowledges for the employee.
    app.acting_as(&manager);
    let refused = app
        .htmx()
        .post(
            &format!("/documents/acknowledgements/{}/acknowledge", mine.id),
            &[],
        )
        .await;
    assert_eq!(refused.status.as_u16(), 403);

    // The employee opens the file and acknowledges in their own session.
    Acknowledgements::open(app.state(), mine.id, agent.id, first.id)
        .await
        .unwrap()
        .unwrap();
    app.acting_as(&agent);
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
    assert_eq!(done.status, AckStatus::Acknowledged);
    assert_eq!(done.acknowledged_by, Some(agent.id));

    // A second version: asked again, within 7 days of today.
    let second = publish_version(&app, &manager, &sop).await;
    assert_ne!(first.id, second.id);
    let again = Acknowledgements::of_version(app.db(), sop.id, second.id)
        .await
        .unwrap();
    let asked = again
        .iter()
        .find(|a| a.employee_id == employee.id)
        .expect("asked again");
    assert_eq!(asked.status, AckStatus::Pending);
    assert_eq!(asked.due_date, at(today(), 7));
    assert_ne!(asked.id, mine.id);
}
