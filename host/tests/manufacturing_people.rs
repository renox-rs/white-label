//! BL-MFG-14 (#395), the people and supplier side of the 07 client with the
//! real modules of the host, run under the 07 seed (the modules test the same
//! mechanisms with their own defaults):
//!
//! - *Lead sources and campaigns* (`SAL.CRM.02`, `SAL.ACL.07`): the
//!   trade-exhibition campaign reports leads, conversion, cost per lead, CAC
//!   and ROAS; a cost that takes it over its budget waits for the sales
//!   manager, never the person who entered it.
//! - *Supplier scorecard* (`STK.RUL.04`, `STK.ACL.07`): a blocked preferred
//!   supplier sends the purchase request to the alternative; a change of the
//!   preferred supplier needs a second person.
//! - *Onboarding and SOPs* (`HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`,
//!   `DOC.RUL.04`): hiring an operator starts the checklist, the role is
//!   active before the checklist is complete (07 blocks nothing), overdue
//!   steps are reminded the day after their due date and every 2 days, the
//!   production manager and the line supervisor are told after 5 days; a new
//!   SOP version asks its readers to acknowledge within 7 days, reminds every
//!   3 days and tells the reader's manager and the SOP's owner after 7 days
//!   overdue; only the employee acknowledges, or records their own step.

mod support;

use host::manufacturing_seed::{CAMPAIGN_CHANNELS, LEAD_SOURCES};
use host::roles::{
    ADMIN, LINE_OPERATOR, PROCUREMENT, PROCUREMENT_MANAGER, PRODUCTION_MANAGER, SALES,
    SALES_MANAGER, SUPERVISOR,
};
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_documents::acknowledgements::run as ack_run;
use wl_documents::{AckStatus, Document, Sop, SopAcknowledgement};
use wl_hr::steps::{self, DoneInput};
use wl_hr::{
    Employee, EmployeeStatus, NewEmployee, Onboarding, OnboardingStep, StepStatus, employees,
    reminders,
};
use wl_inventory::item_supplier::RequestStatus;
use wl_inventory::reorder;
use wl_inventory::{
    Item, ItemSupplier, Location, PreferredSupplierRequest, ReorderSuggestion, SuggestionStatus,
    SupplierRole,
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

use support::{company, manufacturing_booted, messages_to, person, tenant};

const CODE: &str = "ARABPLAST26";

fn day(days: i64) -> NaiveDate {
    support::day(days)
}

fn at(from: NaiveDate, plus: u64) -> NaiveDate {
    from.checked_add_days(Days::new(plus)).unwrap()
}

// ---------------------------------------------------- lead sources, campaigns

/// The trade-exhibition campaign of the client, active, AED 5,000 budget.
async fn exhibition(app: &TestApp, owner: &User) -> Campaign {
    let channel = CAMPAIGN_CHANNELS
        .iter()
        .find(|(name, _)| *name == "Trade exhibitions")
        .expect("the client offers the trade exhibitions channel")
        .1;
    assert_eq!(channel, "event");
    assert!(LEAD_SOURCES.contains(&"event"));
    let source = LeadSource::by_code(app.db(), "event")
        .await
        .unwrap()
        .unwrap();
    assert!(source.active, "07 uses the event source");
    let campaign = Campaign {
        name: "Arabplast 2026".into(),
        code: CODE.into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::Event),
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

/// A campaign with the channel *trade exhibitions*, a budget and cost
/// entries: 20 leads from the exhibition, 4 won, AED 3,000 of costs, 3
/// customers whose first sale is AED 10,000: conversion 20.0%, cost per lead
/// 150.00, CAC 1,000.00 and ROAS 10.00. The sales manager opens the report;
/// sales do not see it.
#[renox::test]
async fn the_trade_exhibition_campaign_reports_leads_conversion_cost_per_lead_cac_and_roas() {
    let app = manufacturing_booted().await;
    let manager = person(&app, "Mona", SALES_MANAGER).await;
    let staff = person(&app, "Sami", SALES).await;
    let campaign = exhibition(&app, &manager).await;

    for amount in [200_000, 100_000] {
        let entry = costs::save(
            app.state(),
            CostInput {
                campaign_id: campaign.id,
                entry_date: day(-5),
                amount,
                description: "Stand fee".into(),
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
        let new = NewLead::staff(None, &format!("Visitor {i}"), "event").campaign_code(CODE);
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
            .attribute(app.state(), Attribute::party(party, "event").campaign(CODE))
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
    let by_campaign = report::compute(
        &facts,
        &Filter::default(),
        GroupBy::Campaign,
        &Viewer::all(),
    );
    assert_eq!(by_campaign.rows.len(), 1);
    let cells = by_campaign.rows[0].cells();
    assert_eq!(cells.leads, "20");
    assert_eq!(cells.won, "4");
    assert_eq!(cells.conversion, "20.0%");
    assert_eq!(cells.customers, "3");
    assert_eq!(cells.cost, "3,000.00");
    assert_eq!(cells.cost_per_lead, "150.00");
    assert_eq!(cells.cac, "1,000.00");
    assert_eq!(cells.value, "30,000.00");
    assert_eq!(cells.roas, "10.00");
    let by_source = report::compute(&facts, &Filter::default(), GroupBy::Source, &Viewer::all());
    assert_eq!(by_source.rows.len(), 1, "the event source only");
    assert_eq!(by_source.rows[0].cells().leads, "20");
    let by_channel = report::compute(&facts, &Filter::default(), GroupBy::Channel, &Viewer::all());
    assert_eq!(by_channel.rows[0].cells().leads, "20");

    // The page: the sales manager reads it, sales do not.
    app.acting_as(&manager);
    app.get("/sales/acquisition-report?group=campaign")
        .await
        .assert_ok()
        .assert_see(CODE);
    app.acting_as(&staff);
    app.get("/sales/acquisition-report?group=campaign")
        .await
        .assert_forbidden();
}

/// A cost entry that takes the campaign over its budget waits for the sales
/// manager (`SAL.ACL.07`, on in 07); the person who entered it cannot approve
/// it, the sales manager can; the waiting entry is not counted.
#[renox::test]
async fn a_cost_entry_over_the_budget_waits_for_the_sales_manager() {
    let app = manufacturing_booted().await;
    let manager = person(&app, "Mona", SALES_MANAGER).await;
    let staff = person(&app, "Sami", SALES).await;
    let campaign = exhibition(&app, &manager).await;
    let input = |amount| CostInput {
        campaign_id: campaign.id,
        entry_date: day(-5),
        amount,
        description: "Stand fee".into(),
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
        "the one who entered it cannot approve it"
    );
    approvals::approve(app.state(), request, &manager, "agreed")
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
    assert_eq!(
        costs::totals(app.db(), campaign.id, None)
            .await
            .unwrap()
            .approved,
        550_000,
        "counted once approved"
    );
}

// ------------------------------------------------------------- suppliers

/// Resin, reordered at or below 20 kg up to 100 kg, 10 kg on hand, with Gulf
/// Resin preferred (5 days) and Jebel Polymers the first alternative (10
/// days), listed by a procurement officer through the page.
async fn resin(app: &TestApp, officer: &User) -> (Item, Location, PartyId, PartyId) {
    let preferred = company(app, "Gulf Resin LLC").await;
    let alternative = company(app, "Jebel Polymers LLC").await;
    let mut item = Item::new("RESIN", "Resin", "kg").reordered(20_000, 100_000);
    item.save(app.db()).await.unwrap();
    let mut store = Location::new("RAW", "Raw materials");
    store.save(app.db()).await.unwrap();
    wl_inventory::movement::StockMovement::receive(
        item.id,
        None,
        store.id,
        10_000,
        1_800,
        18_000,
        &Subject::new("inventory.goods_receipt", 1),
        None,
    )
    .save(app.db())
    .await
    .unwrap();
    app.acting_as(officer);
    for (supplier, role, extra) in [
        (preferred, "preferred", ("lead_days", "5")),
        (alternative, "alternative", ("lead_days", "10")),
    ] {
        let id = supplier.0.to_string();
        let mut form = vec![("supplier", id.as_str()), ("role", role), extra];
        if role == "alternative" {
            form.push(("rank", "1"));
        }
        app.htmx()
            .post(&format!("/items/{}/suppliers/add", item.id), &form)
            .await
            .assert_ok();
    }
    (item, store, preferred, alternative)
}

/// `STK.RUL.04` with the 07 values: the preferred supplier is blocked and the
/// resin is below its reorder point, so the purchase request (the reorder
/// suggestion of 07:00) goes to the alternative supplier, marked
/// *alternative* with the reason; the blocked one is shown as on hold.
#[renox::test]
async fn a_blocked_preferred_supplier_sends_the_purchase_request_to_the_alternative() {
    let app = manufacturing_booted().await;
    let officer = person(&app, "Olga", PROCUREMENT).await;
    let (item, store, preferred, alternative) = resin(&app, &officer).await;

    support::block(&app, preferred).await;
    app.run_scheduled(reorder::TASK).await.unwrap();

    let found = ReorderSuggestion::query()
        .where_(ReorderSuggestion::ITEM_ID.eq(item.id))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    let request = &found[0];
    assert_eq!(request.location_id, store.id);
    assert_eq!(request.supplier_party_id, Some(alternative.0));
    assert_eq!(request.supplier_role.as_deref(), Some("alternative"));
    assert_eq!(
        request.reason.as_deref(),
        Some("preferred supplier on hold")
    );
    assert_eq!(request.status, SuggestionStatus::Open);
    assert_eq!(request.suggested_qty_milli, 90_000, "up to the maximum");
    let shown = request.alternatives();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].supplier_party_id, preferred.0);
    assert!(shown[0].on_hold);
}

/// `STK.ACL.07` with the 07 values (on): procurement asks for the alternative
/// to become preferred; nothing changes until a second person, the
/// procurement manager, approves; the officer cannot decide and the manager
/// cannot approve a request of their own.
#[renox::test]
async fn a_second_person_approves_a_new_preferred_supplier() {
    let app = manufacturing_booted().await;
    let officer = person(&app, "Olga", PROCUREMENT).await;
    let boss = person(&app, "Basil", PROCUREMENT_MANAGER).await;
    let other = person(&app, "Bela", PROCUREMENT_MANAGER).await;
    let (item, _store, preferred, alternative) = resin(&app, &officer).await;
    let rows = || async {
        ItemSupplier::query()
            .where_(ItemSupplier::ITEM_ID.eq(item.id))
            .order_by(ItemSupplier::RANK)
            .get(app.db())
            .await
            .unwrap()
            .into_iter()
            .map(|r| (r.supplier_party_id, r.role))
            .collect::<Vec<_>>()
    };

    // The officer asks; the officer cannot decide.
    app.acting_as(&officer);
    let alt = alternative.0.to_string();
    app.htmx()
        .post(
            &format!("/items/{}/preferred-requests", item.id),
            &[("supplier", alt.as_str())],
        )
        .await
        .assert_ok();
    let request = PreferredSupplierRequest::query()
        .where_(PreferredSupplierRequest::ITEM_ID.eq(item.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("a request");
    assert_eq!(request.status, RequestStatus::Pending);
    assert_eq!(request.requested_by, Some(officer.id));
    let path = format!("/preferred-requests/{}/approve", request.id);
    app.acting_as(&officer);
    app.htmx().post(&path, &[]).await.assert_forbidden();
    assert_eq!(
        rows().await[0],
        (preferred.0, SupplierRole::Preferred),
        "nothing changed yet"
    );

    // A manager who asks cannot approve their own request.
    app.acting_as(&boss);
    app.htmx().post(&path, &[]).await.assert_ok();
    let approved = PreferredSupplierRequest::find_or_404(app.db(), request.id)
        .await
        .unwrap();
    assert_eq!(approved.status, RequestStatus::Approved);
    assert_eq!(approved.decided_by, Some(boss.id));
    assert_eq!(
        rows().await,
        [
            (alternative.0, SupplierRole::Preferred),
            (preferred.0, SupplierRole::Alternative),
        ]
    );

    // The new preferred supplier: the manager asks to go back, and cannot
    // decide it; the other manager does.
    let back = preferred.0.to_string();
    app.htmx()
        .post(
            &format!("/items/{}/preferred-requests", item.id),
            &[("supplier", back.as_str())],
        )
        .await
        .assert_ok();
    let second = PreferredSupplierRequest::query()
        .where_(PreferredSupplierRequest::ITEM_ID.eq(item.id))
        .where_(PreferredSupplierRequest::STATUS.eq(RequestStatus::Pending))
        .first(app.db())
        .await
        .unwrap()
        .expect("a second request");
    let path = format!("/preferred-requests/{}/approve", second.id);
    app.htmx().post(&path, &[]).await.assert_status(422);
    app.acting_as(&other);
    app.htmx().post(&path, &[]).await.assert_ok();
    assert_eq!(rows().await[0], (preferred.0, SupplierRole::Preferred));
}

// ----------------------------------------------------- onboarding and SOPs

/// A line operator hired today with an account: the employee and their
/// onboarding.
async fn hired_operator(app: &TestApp, name: &str) -> (Employee, User, Onboarding) {
    let user = person(app, name, LINE_OPERATOR).await;
    let party = tenant(app, name).await;
    let employee = employees::create(
        app.state(),
        NewEmployee::new(party, "line_operator", today())
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
    onboarding
        .steps(app.db())
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.title_en == title)
        .unwrap_or_else(|| panic!("{title}"))
}

/// `HRM.ACL.10` and the activation rule of 07: the operator's checklist
/// gives the documents step to the production manager, the welcome step to
/// the line supervisor and the upload step to the employee; the role is
/// active before the checklist is complete (07 blocks nothing); a step of the
/// employee is recorded by the employee alone, not by the production manager.
#[renox::test]
async fn the_role_is_active_before_the_checklist_is_complete_and_the_employee_records_their_own_step()
 {
    let app = manufacturing_booted().await;
    let pia = person(&app, "Pia", PRODUCTION_MANAGER).await;
    let sam = person(&app, "Sam", SUPERVISOR).await;
    let admin = person(&app, "Ada", ADMIN).await;
    let (employee, olga, onboarding) = hired_operator(&app, "Olga").await;

    let documents = step(&app, &onboarding, "Documents and access").await;
    assert_eq!(documents.assignees(app.db()).await.unwrap(), [pia.id]);
    let welcome = step(&app, &onboarding, "Welcome and introductions").await;
    assert_eq!(welcome.assignees(app.db()).await.unwrap(), [sam.id]);
    let own = step(&app, &onboarding, "Upload your personal documents").await;
    assert_eq!(own.assignees(app.db()).await.unwrap(), [olga.id]);

    // `HRM.ACL.10`: nobody records the employee's own step for them, not the
    // production manager, nor the supervisor; the employee does.
    for by in [&pia, &sam] {
        let refused = steps::mark_done(app.state(), own.id, by.id, DoneInput::default())
            .await
            .unwrap();
        assert!(refused.is_err(), "{} cannot record it", by.name);
    }
    let done = steps::mark_done(app.state(), own.id, olga.id, DoneInput::default())
        .await
        .unwrap()
        .expect("the employee records their own step");
    assert_eq!(done.status, StepStatus::Done);

    // The role is active with the checklist open.
    app.acting_as(&admin);
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

/// `HRM.RUL.17` with the 07 values: the production manager's step "Documents
/// and access" (due 2 days after the start) is reminded on the first run
/// after its due date, then every 2 days; 5 days after it, the production
/// manager and the line supervisor are told once.
#[renox::test]
async fn an_overdue_step_is_reminded_the_day_after_then_every_second_day_and_escalated_after_five()
{
    let app = manufacturing_booted().await;
    let pia = person(&app, "Pia", PRODUCTION_MANAGER).await;
    let sam = person(&app, "Sam", SUPERVISOR).await;
    let (_, olga, onboarding) = hired_operator(&app, "Olga").await;
    let documents = step(&app, &onboarding, "Documents and access").await;
    let due = documents.due_date;
    assert_eq!(due, at(today(), 2));
    let remind = "hr.onboarding_step_reminder";
    let escalate = "hr.onboarding_step_escalated";
    let state_of = || async {
        OnboardingStep::find_or_404(app.db(), documents.id)
            .await
            .unwrap()
    };

    reminders::run(app.state(), due).await.unwrap();
    assert_eq!(state_of().await.last_reminded_on, None, "not yet overdue");
    let reminded = || async {
        messages_to(&app, remind, &pia)
            .await
            .into_iter()
            .filter(|m| m.subject_label.as_deref() == Some("Documents and access"))
            .count()
    };
    assert_eq!(reminded().await, 0);
    reminders::run(app.state(), at(due, 1)).await.unwrap();
    assert_eq!(
        state_of().await.last_reminded_on,
        Some(at(due, 1)),
        "the first run after the due date"
    );
    assert_eq!(reminded().await, 1);
    reminders::run(app.state(), at(due, 2)).await.unwrap();
    assert_eq!(state_of().await.last_reminded_on, Some(at(due, 1)));
    reminders::run(app.state(), at(due, 3)).await.unwrap();
    assert_eq!(
        state_of().await.last_reminded_on,
        Some(at(due, 3)),
        "again 2 days later"
    );
    assert_eq!(reminded().await, 2);
    assert!(
        messages_to(&app, remind, &olga)
            .await
            .iter()
            .all(|m| m.subject_label.as_deref() != Some("Documents and access")),
        "the employee is not reminded of the production manager's step"
    );

    assert_eq!(state_of().await.escalated_on, None);
    assert!(messages_to(&app, escalate, &pia).await.is_empty());
    reminders::run(app.state(), at(due, 5)).await.unwrap();
    assert_eq!(state_of().await.escalated_on, Some(at(due, 5)));
    let told = |user: &User| {
        let user = user.clone();
        let app = &app;
        async move {
            messages_to(app, escalate, &user)
                .await
                .into_iter()
                .filter(|m| m.subject_label.as_deref() == Some("Documents and access"))
                .count()
        }
    };
    assert_eq!(told(&pia).await, 1, "the production manager is told");
    assert_eq!(told(&sam).await, 1, "the line supervisor is told");
    reminders::run(app.state(), at(due, 7)).await.unwrap();
    assert_eq!(state_of().await.escalated_on, Some(at(due, 5)), "once");
    assert_eq!(told(&pia).await, 1);
}

/// Uploads a new version of the SOP `sop` as `by` (a probe route: the upload
/// needs an `AuthUser`) and accepts it through the page route.
async fn publish_version(app: &TestApp, by: &User, sop: &Sop) -> Document {
    app.acting_as(by);
    let uploaded = app
        .post(
            &format!("/probe/sops/{}/versions", sop.id),
            &[("file_name", "work-instructions.pdf")],
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

/// `DOC.RUL.04` with the 07 values: a version of the work instructions, owned
/// by the production manager, asks the operators to acknowledge within 7
/// days; overdue, the reader is reminded the day after and every 3 days, and
/// after 7 days the line supervisor and the production manager (the SOP's
/// owner role) are told once; nobody acknowledges for the operator (`HRM.ACL.10`);
/// the operator does in their own session; a new version asks again within 7
/// days.
#[renox::test]
async fn a_new_sop_version_asks_the_operator_to_acknowledge_within_seven_days_and_again_for_the_next()
 {
    let app = manufacturing_booted().await;
    let pia = person(&app, "Pia", PRODUCTION_MANAGER).await;
    let sam = person(&app, "Sam", SUPERVISOR).await;
    let (employee, olga, _) = hired_operator(&app, "Olga").await;
    let sop = Sop::query()
        .where_(Sop::CODE.eq("SOP-MFG-WORK-INSTRUCTIONS"))
        .first(app.db())
        .await
        .unwrap()
        .expect("seeded");
    assert_eq!(sop.owner_role, "production_manager");
    assert_eq!(sop.ack_due_days, 7);

    let first = publish_version(&app, &pia, &sop).await;
    let asked = SopAcknowledgement::query()
        .where_(SopAcknowledgement::EMPLOYEE_ID.eq(employee.id))
        .where_(SopAcknowledgement::DOCUMENT_ID.eq(first.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("the operator is asked");
    assert_eq!(asked.status, AckStatus::Pending);
    assert_eq!(asked.due_date, at(today(), 7));

    // Overdue: reminded the day after, not again until 3 days later.
    let due = asked.due_date;
    let reload = || async {
        SopAcknowledgement::find(app.db(), asked.id)
            .await
            .unwrap()
            .unwrap()
    };
    let remind = "documents.sop_ack_reminder";
    let escalate = "documents.sop_ack_escalated";
    ack_run::run(app.state(), due).await.unwrap();
    assert_eq!(reload().await.last_reminded_on, None, "due, not overdue");
    ack_run::run(app.state(), at(due, 1)).await.unwrap();
    assert_eq!(reload().await.last_reminded_on, Some(at(due, 1)));
    ack_run::run(app.state(), at(due, 3)).await.unwrap();
    assert_eq!(reload().await.last_reminded_on, Some(at(due, 1)));
    ack_run::run(app.state(), at(due, 4)).await.unwrap();
    assert_eq!(reload().await.last_reminded_on, Some(at(due, 4)));
    assert_eq!(messages_to(&app, remind, &olga).await.len(), 2);
    assert!(messages_to(&app, escalate, &pia).await.is_empty());
    ack_run::run(app.state(), at(due, 7)).await.unwrap();
    assert_eq!(reload().await.escalated_on, Some(at(due, 7)));
    assert_eq!(messages_to(&app, escalate, &pia).await.len(), 1);
    assert_eq!(messages_to(&app, escalate, &sam).await.len(), 1);
    ack_run::run(app.state(), at(due, 9)).await.unwrap();
    assert_eq!(messages_to(&app, escalate, &pia).await.len(), 1, "once");

    // Nobody acknowledges for the operator; the operator does.
    for by in [&pia, &sam] {
        app.acting_as(by);
        let refused = app
            .htmx()
            .post(
                &format!("/documents/acknowledgements/{}/acknowledge", asked.id),
                &[],
            )
            .await;
        assert_eq!(refused.status.as_u16(), 403, "{}", by.name);
    }
    wl_documents::Acknowledgements::open(app.state(), asked.id, olga.id, first.id)
        .await
        .unwrap()
        .unwrap();
    app.acting_as(&olga);
    app.htmx()
        .post(
            &format!("/documents/acknowledgements/{}/acknowledge", asked.id),
            &[],
        )
        .await
        .assert_ok();
    let done = reload().await;
    assert_eq!(done.status, AckStatus::Acknowledged);
    assert_eq!(done.acknowledged_by, Some(olga.id));

    // A new version: asked again, within 7 days of today.
    let second = publish_version(&app, &pia, &sop).await;
    assert_ne!(first.id, second.id);
    let again = wl_documents::Acknowledgements::of_version(app.db(), sop.id, second.id)
        .await
        .unwrap();
    let new = again
        .iter()
        .find(|a| a.employee_id == employee.id)
        .expect("asked again");
    assert_eq!(new.status, AckStatus::Pending);
    assert_eq!(new.due_date, at(today(), 7));
    assert_ne!(new.id, asked.id);
}
