//! BL-CLN-15 (#363), the sales, finance, staff and registration side of the
//! 05 Clinics client with the real modules of the host: the lead sources and
//! the campaign figures (`SAL.CRM.02`, `SAL.ACL.07`), the advertising permit
//! (`CLN.ACL.13`), the loyalty programme and the segments (`SAL.LOY.01`,
//! `SAL.RUL.07`, `SAL.RUL.08`, `SAL.ACL.07`), the onboarding checklists
//! (`HRM.ONB.01`, `HRM.RUL.17`) and the SOP library (`DOC.RUL.04`,
//! `HRM.ACL.10`).

mod support;

use host::roles::{ADMIN, CLINIC_MANAGER, MARKETING, MARKETING_MANAGER, OWNER, RECEPTIONIST};
use renox::chrono::{Days, Months, NaiveDate};
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
    Campaign, CampaignStatus, CostEntry, CostStatus, Lead, LeadSource, LoyaltyProgramme,
    MarketingChannel, MatchMode, NewLead, PointsAdjustment, Purpose, Rule, Segment,
};

const CODE: &str = "INSTA26";

/// The Instagram campaign, active, AED 5,000 budget.
async fn instagram_campaign(app: &TestApp, owner: &User) -> Campaign {
    let source = LeadSource::by_code(app.db(), "social")
        .await
        .unwrap()
        .unwrap();
    let campaign = Campaign {
        name: "Instagram autumn".into(),
        code: CODE.into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::Meta),
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

/// The nine sources of the spec are offered, a lead from *paid ads* or
/// *social* names a campaign, and a lead from any other source does not.
#[renox::test]
async fn the_lead_sources_are_the_nine_of_the_spec_and_two_need_a_campaign() {
    let app = clinics_booted().await;
    for code in clinics::seed::LEAD_SOURCES {
        let source = LeadSource::by_code(app.db(), code)
            .await
            .unwrap()
            .expect(code);
        assert!(source.active, "{code}");
    }
    let capture = |source: &'static str| {
        let state = app.state().clone();
        async move {
            wl_sales::lead::capture(&state, NewLead::staff(None, "Pat Lane", source), day(0))
                .await
                .unwrap()
        }
    };
    assert!(
        capture("paid_ads").await.is_err(),
        "paid ads needs a campaign"
    );
    assert!(capture("social").await.is_err(), "social needs a campaign");
    for source in [
        "referral",
        "walk_in",
        "website",
        "aggregator",
        "partner",
        "event",
        "other",
    ] {
        assert!(capture(source).await.is_ok(), "{source}");
    }
}

/// An Instagram campaign with a budget and cost entries reports leads,
/// conversion, cost per lead, CAC and ROAS; a cost entry over the budget
/// waits for a manager (`SAL.ACL.07`).
#[renox::test]
async fn the_instagram_campaign_reports_its_figures_and_an_overspend_waits_for_a_manager() {
    let app = clinics_booted().await;
    let manager = person(&app, "Mona", MARKETING_MANAGER).await;
    let staff = person(&app, "Marco", MARKETING).await;
    let campaign = instagram_campaign(&app, &manager).await;
    let input = |amount| CostInput {
        campaign_id: campaign.id,
        entry_date: day(-5),
        amount,
        description: "Instagram ads".into(),
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
        let new = NewLead::staff(None, &format!("Enquirer {i}"), "social").campaign_code(CODE);
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
                Attribute::party(party, "social").campaign(CODE),
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

/// `CLN.ACL.13`: a campaign that promotes services without a permit number
/// cannot be activated; with the medical director's permit it can; the day
/// after the permit's last valid day the daily run ends it.
#[renox::test]
async fn a_campaign_needs_its_advertising_permit_to_be_activated_and_ends_when_it_lapses() {
    let app = clinics_booted().await;
    let manager = person(&app, "Mona", MARKETING_MANAGER).await;
    let owner = person(&app, "Olga", OWNER).await;
    let campaign = Campaign {
        name: "Laser promotion".into(),
        code: "LASER26".into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::Meta),
        owner_id: Some(manager.id),
        start_date: Some(day(0)),
        end_date: Some(day(60)),
        ..Campaign::default()
    }
    .store(app.db())
    .await
    .unwrap()
    .unwrap();
    let activate = format!("/campaigns/{}/transitions/activate", campaign.id);
    let status = || async {
        Campaign::find(app.db(), campaign.id)
            .await
            .unwrap()
            .unwrap()
            .status
    };

    // No permit recorded: refused, with the words, and the marketing
    // coordinator cannot record one.
    app.acting_as(&manager);
    let refused = app.htmx().post(&activate, &[("x", "")]).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("advertising permit"),
        "{}",
        refused.text()
    );
    assert_eq!(status().await, CampaignStatus::Draft);
    app.htmx()
        .post(
            "/clinics/campaign-permits",
            &[
                ("campaign", campaign.id.to_string().as_str()),
                ("promotes", "no"),
            ],
        )
        .await
        .assert_forbidden();

    // The medical director records the permit, valid through today.
    app.acting_as(&owner);
    app.htmx()
        .post(
            "/clinics/campaign-permits",
            &[
                ("campaign", campaign.id.to_string().as_str()),
                ("promotes", "yes"),
                ("permit_number", "AD-2026-17"),
                ("valid_to", today().to_string().as_str()),
            ],
        )
        .await
        .assert_ok();
    app.get("/clinics/campaign-permits")
        .await
        .assert_ok()
        .assert_see("AD-2026-17");

    app.acting_as(&manager);
    app.htmx()
        .post(&activate, &[("x", "")])
        .await
        .assert_status(303);
    assert_eq!(status().await, CampaignStatus::Active);

    // On the last valid day it still runs; the day after, it ends.
    let on_last = wl_sales::campaigns::run_daily(app.state(), today())
        .await
        .unwrap();
    assert_eq!(on_last.ended, 0);
    assert_eq!(status().await, CampaignStatus::Active);
    let after = wl_sales::campaigns::run_daily(app.state(), day(1))
        .await
        .unwrap();
    assert_eq!((after.ended, after.failed), (1, 0));
    assert_eq!(status().await, CampaignStatus::Ended);
}

/// `SAL.LOY.01`, `SAL.RUL.07`, `SAL.RUL.08`, `SAL.ACL.07`: the seeded
/// programme earns 20 points on AED 200, 100 points are an AED 10 discount,
/// points earned on 2026-11-01 expire on 2027-11-01 with a reminder on
/// 2027-10-02, and an adjustment of 600 points needs a manager.
#[renox::test]
async fn the_loyalty_programme_earns_one_point_per_ten_dirhams_and_expires_after_a_year() {
    let app = clinics_booted().await;
    let programme = LoyaltyProgramme::first_active(app.db())
        .await
        .unwrap()
        .expect("the clinic starts with its programme");
    assert_eq!(programme.earn_rate, 1);
    assert_eq!(programme.earn_unit_aed, 10);
    assert_eq!(programme.validity_months, 12);
    // A self-pay invoice of AED 200 earns 20 points; none on the part an
    // insurer pays (the module earns on the patient's share only).
    assert_eq!(programme.points_for(20_000), 20);
    assert_eq!(programme.points_for(0), 0);
    // 100 points are an AED 10 discount.
    assert_eq!(programme.min_redeem_points, 100);
    assert_eq!(programme.min_redeem_points * programme.point_value, 1_000);

    let config = wl_sales::config::config_of(app.state());
    assert!(config.confirm_redemption_by_code);
    assert_eq!(config.expiry_reminder_days, 30);
    let earned: NaiveDate = "2026-11-01".parse().unwrap();
    let expires = earned
        .checked_add_months(Months::new(
            u32::try_from(programme.validity_months).unwrap(),
        ))
        .unwrap();
    assert_eq!(expires.to_string(), "2027-11-01");
    let reminder = expires
        .checked_sub_days(Days::new(
            u64::try_from(config.expiry_reminder_days).unwrap(),
        ))
        .unwrap();
    assert_eq!(reminder.to_string(), "2027-10-02");

    let adjustment = |points| PointsAdjustment {
        points,
        ..PointsAdjustment::default()
    };
    assert!(adjustment(600).needs_manager(config.adjustment_limit));
    assert!(adjustment(-600).needs_manager(config.adjustment_limit));
    assert!(!adjustment(500).needs_manager(config.adjustment_limit));
}

/// The segments: no visit in 6 months is there; a segment rule on a clinical
/// service is refused, one on a non-clinical item is not (`CLN.ACL.13`,
/// wl-sales §7).
#[renox::test]
async fn a_segment_cannot_use_a_clinical_service_as_a_rule() {
    let app = clinics_booted().await;
    let lapsed = Segment::query()
        .where_(Segment::NAME.eq(clinics::seed::LAPSED_SEGMENT_NAME))
        .first(app.db())
        .await
        .unwrap()
        .expect("seeded");
    assert_eq!(
        lapsed.rules(),
        [Rule::NoSaleForDays {
            days: clinics::seed::LAPSED_SEGMENT_DAYS
        }]
    );

    let config = wl_sales::config::config_of(app.state());
    let by = |item: &str, name: &str| {
        Segment::of(
            name,
            MatchMode::All,
            &[Rule::BoughtItem {
                item: item.to_owned(),
                days: None,
            }],
        )
    };
    for code in ["consultation", "follow_up", "procedure"] {
        let stored = by(code, &format!("Bought {code}"))
            .store(app.db(), &config)
            .await
            .unwrap();
        assert!(stored.is_err(), "{code} is a clinical service");
    }
    let stored = by("toothbrush", "Bought a toothbrush")
        .store(app.db(), &config)
        .await
        .unwrap();
    assert!(stored.is_ok());
}

/// A receptionist hired today, with an account: the employee, their user and
/// the onboarding.
async fn hired_receptionist(app: &TestApp, name: &str) -> (Employee, User, Onboarding) {
    let user = person(app, name, RECEPTIONIST).await;
    let party = tenant(app, name).await;
    let employee = employees::create(
        app.state(),
        NewEmployee::new(party, "receptionist", today())
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

fn after(day: NaiveDate, plus: u64) -> NaiveDate {
    day.checked_add_days(Days::new(plus)).unwrap()
}

/// Hiring a receptionist creates the receptionist checklist with tasks to the
/// clinic manager, the line manager and the employee; the role is active
/// before it is complete; an open step is reminded and escalated
/// (`HRM.RUL.17`).
#[renox::test]
async fn hiring_a_receptionist_creates_the_checklist_and_its_reminders_run() {
    let app = clinics_booted().await;
    let admin = person(&app, "Adel", ADMIN).await;
    let manager = person(&app, "Mona", CLINIC_MANAGER).await;
    let (employee, receptionist, onboarding) = hired_receptionist(&app, "Rita").await;

    for name in [
        "Receptionist onboarding",
        "Call-centre agent onboarding",
        "Nurse and assistant onboarding",
        "Practitioner onboarding",
        "Billing coordinator onboarding",
        "Storekeeper onboarding",
    ] {
        let template = OnboardingTemplate::query()
            .where_(OnboardingTemplate::NAME_EN.eq(name))
            .first(app.db())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{name} is seeded"));
        assert!(
            !template.blocks_activation,
            "{name}: completion blocks nothing"
        );
    }
    let template = OnboardingTemplate::query()
        .where_(OnboardingTemplate::NAME_EN.eq("Receptionist onboarding"))
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    let templates = onboarding.templates(app.db()).await.unwrap();
    assert!(templates.iter().any(|t| t.template_id == template.id));
    let steps = onboarding.steps(app.db()).await.unwrap();
    for s in &steps {
        let who = s.assignees(app.db()).await.unwrap();
        match s.owner.as_str() {
            "employee" => assert_eq!(who, [receptionist.id], "{}", s.title_en),
            "manager" => assert_eq!(who, [manager.id], "{}", s.title_en),
            "role:clinic_manager" => assert!(who.contains(&manager.id), "{}", s.title_en),
            // The module's own steps (accounts and access) belong to HR: the
            // holders of the onboarding permission.
            "hr" => assert!(who.contains(&admin.id), "{}", s.title_en),
            other => panic!("unexpected owner {other}"),
        }
    }
    assert!(steps.iter().any(|s| s.owner == "role:clinic_manager"));
    assert!(steps.iter().any(|s| s.owner == "manager"));
    assert!(steps.iter().any(|s| s.owner == "employee"));

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
    // second day; after 5 days the clinic manager is told.
    let own = steps
        .iter()
        .filter(|s| s.owner == "employee" && s.required)
        .min_by_key(|s| s.due_date)
        .expect("a step of the employee");
    let due = own.due_date;
    let state_of = || async { OnboardingStep::find_or_404(app.db(), own.id).await.unwrap() };
    reminders::run(app.state(), due).await.unwrap();
    assert_eq!(state_of().await.last_reminded_on, None);
    reminders::run(app.state(), after(due, 1)).await.unwrap();
    assert_eq!(state_of().await.last_reminded_on, Some(after(due, 1)));
    reminders::run(app.state(), after(due, 2)).await.unwrap();
    assert_eq!(state_of().await.last_reminded_on, Some(after(due, 1)));
    reminders::run(app.state(), after(due, 3)).await.unwrap();
    assert_eq!(state_of().await.last_reminded_on, Some(after(due, 3)));
    reminders::run(app.state(), after(due, 5)).await.unwrap();
    assert_eq!(state_of().await.escalated_on, Some(after(due, 5)));
    assert!(
        !messages_to(&app, "hr.onboarding_step_escalated", &manager)
            .await
            .is_empty()
    );
}

/// Uploads a new version of the SOP `sop` as `by` and accepts it.
async fn publish_version(app: &TestApp, by: &User, sop: &Sop) -> Document {
    app.acting_as(by);
    let uploaded = app
        .post(
            &format!("/probe/sops/{}/versions", sop.id),
            &[("file_name", "front-desk.pdf")],
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

/// A new version of an SOP owned by the clinic manager asks every required
/// reader to acknowledge again within 7 days; no one else acknowledges for
/// them (`DOC.RUL.04`, `HRM.ACL.10`).
#[renox::test]
async fn a_new_front_desk_sop_version_asks_the_readers_to_acknowledge_again_within_seven_days() {
    let app = clinics_booted().await;
    let owner = person(&app, "Mona", CLINIC_MANAGER).await;
    let (employee, receptionist, _) = hired_receptionist(&app, "Rita").await;

    // The library: the clinic manager owns the front desk, billing and
    // complaints SOPs; the owner / medical director the clinical ones.
    for spec in &clinics::seed::SOPS {
        let sop = Sop::query()
            .where_(Sop::CODE.eq(spec.code))
            .first(app.db())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{} is seeded", spec.code));
        assert_eq!(sop.owner_role, spec.owner_role);
        assert_eq!(sop.ack_due_days, 7);
        let readers: Vec<String> = SopReader::query()
            .where_(SopReader::SOP_ID.eq(sop.id))
            .get(app.db())
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.role_key)
            .collect();
        for role in spec.readers {
            assert!(
                readers.contains(&(*role).to_owned()),
                "{}: {role}",
                spec.code
            );
        }
    }
    let sop = Sop::query()
        .where_(Sop::CODE.eq("SOP-CLN-FRONT-DESK"))
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sop.owner_role, CLINIC_MANAGER);
    let infection = Sop::query()
        .where_(Sop::CODE.eq("SOP-CLN-INFECTION-CONTROL"))
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(infection.owner_role, OWNER);

    let first = publish_version(&app, &owner, &sop).await;
    let acks = Acknowledgements::of_version(app.db(), sop.id, first.id)
        .await
        .unwrap();
    let mine = acks
        .iter()
        .find(|a| a.employee_id == employee.id)
        .expect("asked");
    assert_eq!(mine.status, AckStatus::Pending);
    assert_eq!(mine.due_date, after(today(), 7));
    app.acting_as(&owner);
    let refused = app
        .htmx()
        .post(
            &format!("/documents/acknowledgements/{}/acknowledge", mine.id),
            &[],
        )
        .await;
    assert_eq!(refused.status.as_u16(), 403);
    Acknowledgements::open(app.state(), mine.id, receptionist.id, first.id)
        .await
        .unwrap()
        .unwrap();
    app.acting_as(&receptionist);
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
    assert_eq!(done.acknowledged_by, Some(receptionist.id));

    let second = publish_version(&app, &owner, &sop).await;
    let again = Acknowledgements::of_version(app.db(), sop.id, second.id)
        .await
        .unwrap();
    let asked = again
        .iter()
        .find(|a| a.employee_id == employee.id)
        .expect("asked again");
    assert_eq!(asked.status, AckStatus::Pending);
    assert_eq!(asked.due_date, after(today(), 7));
}

/// `DOC.RUL.04` for the 05 SOPs: a pending acknowledgement is reminded every
/// 3 days after its due date, and 7 days after it the reader's manager and
/// the SOP's owner role are told, once.
#[renox::test]
async fn an_overdue_sop_acknowledgement_is_reminded_every_three_days_and_escalated_once() {
    use wl_documents::acknowledgements::run::run;

    const REMINDER: &str = "documents.sop_ack_reminder";
    const ESCALATED: &str = "documents.sop_ack_escalated";

    let app = clinics_booted().await;
    let owner = person(&app, "Mona", CLINIC_MANAGER).await;
    let (employee, receptionist, _) = hired_receptionist(&app, "Rita").await;
    let sop = Sop::query()
        .where_(Sop::CODE.eq("SOP-CLN-FRONT-DESK"))
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    let version = publish_version(&app, &owner, &sop).await;
    let ack = Acknowledgements::of_version(app.db(), sop.id, version.id)
        .await
        .unwrap()
        .into_iter()
        .find(|a| a.employee_id == employee.id)
        .expect("asked");
    let due = ack.due_date;
    assert_eq!(due, after(today(), 7));

    run(app.state(), due).await.unwrap();
    assert!(messages_to(&app, REMINDER, &receptionist).await.is_empty());
    run(app.state(), after(due, 1)).await.unwrap();
    assert_eq!(messages_to(&app, REMINDER, &receptionist).await.len(), 1);
    run(app.state(), after(due, 2)).await.unwrap();
    run(app.state(), after(due, 3)).await.unwrap();
    assert_eq!(messages_to(&app, REMINDER, &receptionist).await.len(), 1);
    run(app.state(), after(due, 4)).await.unwrap();
    assert_eq!(messages_to(&app, REMINDER, &receptionist).await.len(), 2);
    assert!(messages_to(&app, ESCALATED, &owner).await.is_empty());

    run(app.state(), after(due, 7)).await.unwrap();
    let told = messages_to(&app, ESCALATED, &owner).await.len();
    assert!(told >= 1, "the clinic manager is told");
    run(app.state(), after(due, 8)).await.unwrap();
    run(app.state(), after(due, 12)).await.unwrap();
    assert_eq!(
        messages_to(&app, ESCALATED, &owner).await.len(),
        told,
        "once"
    );
    let stored = SopAcknowledgement::find(app.db(), ack.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.escalated_on, Some(after(due, 7)));
}
