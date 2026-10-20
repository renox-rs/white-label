//! BL-PRP-27 (#277): the Property client's values for the shared compliance,
//! document, notice, sales and staff mechanisms are in force: the seed stores
//! them, the guards and providers work with them, and the settings page shows
//! each one with its spec reference.
//!
//! Dates are relative to the business date (`documents::today()`), so the
//! checks hold at any real date.

use host::roles::{ADMIN, MEMBER};
use property::enquiry::{self, NewEnquiry};
use property::ticket_update::{self, TicketUpdate};
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_documents::{Document, DocumentKind, DocumentStatus, Sop, SopReader};
use wl_hr::OnboardingTemplate;
use wl_kernel::compliance::service::recipients;
use wl_kernel::compliance::{AlertKind, Raised, is_critical, raise, schedule_subject};
use wl_kernel::documents::{self, BlockMode, OverrideMode, today};
use wl_kernel::late_entries::Entered;
use wl_kernel::notify::{GiveUpReaction, notification_type};
use wl_kernel::tickets::{Assignee, OpenTicket};
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_parties::{CompanyDetails, NewParty, PersonDetails, create};
use wl_sales::{LeadSource, PartyAcquisition};
use wl_ticketing::{Category, NewContract, ServiceContract, ServiceContracts, Ticket, Tickets};

/// The app, with `wl:install` and the seeds run as the demo does.
async fn booted() -> TestApp {
    let app = TestApp::new(host::app()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// A verified user with `role`.
async fn person(app: &TestApp, email: &str, role: &str) -> User {
    let mut user = User::register(app.db(), "Anna", email, "secret123")
        .await
        .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    user.assign_role(app.db(), role).await.unwrap();
    user
}

/// A vendor company.
async fn vendor(app: &TestApp, name: &str) -> PartyId {
    create(
        app.state(),
        NewParty {
            kind: PartyKind::Company,
            legal_name_en: name.into(),
            company: Some(CompanyDetails::default()),
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id()
}

/// The vendor's Civil Defence approval, accepted, valid through `expires`.
async fn approval(app: &TestApp, vendor: PartyId, expires: NaiveDate) {
    let kind = DocumentKind::by_key(app.db(), "civil_defence_approval")
        .await
        .unwrap()
        .expect("the Civil Defence kind is seeded");
    Document::create(
        app.db(),
        Document {
            subject_type: "parties.party".into(),
            subject_id: vendor.0,
            kind_id: kind.id,
            title: "Civil Defence approval".into(),
            expiry_date: Some(expires),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

fn building() -> Subject {
    Subject::new("property.building", 1)
}

/// A ticket of `category` assigned to `vendor`; its id.
async fn ticket(app: &TestApp, category: &str, vendor: PartyId) -> i64 {
    Tickets::open_at(
        app.state(),
        OpenTicket::new(building(), category, "Check the panel").assign_to(Assignee::party(vendor)),
        renox::db::now(),
    )
    .await
    .unwrap()
    .id
    .0
}

/// The SOP `code`.
async fn sop(app: &TestApp, code: &str) -> Sop {
    Sop::query()
        .where_(Sop::CODE.eq(code))
        .first(app.db())
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("{code}"))
}

/// A draft contract of `vendor` covering `category` from today.
async fn draft_contract(app: &TestApp, vendor: PartyId, category: &str) -> ServiceContract {
    let contract = ServiceContracts::create_draft(
        app.state(),
        NewContract {
            vendor,
            title: "Annual maintenance".into(),
            start_date: today(),
            end_date: today() + Days::new(365),
            renewal_notice_days: 60,
            sla_policy_id: None,
        },
    )
    .await
    .unwrap();
    ServiceContracts::add_coverage(app.db(), &contract, building(), category)
        .await
        .unwrap();
    contract
}

/// The ticket categories are seeded with the compliance flags, once.
#[renox::test]
async fn the_seed_stores_the_ticket_categories_once() {
    let app = booted().await;
    for key in ["fire_life_safety", "lifts"] {
        let category = Category::by_key(app.db(), key).await.unwrap().expect(key);
        assert!(
            category.compliance_critical && category.contract_end_reassign_only,
            "{key}"
        );
        assert!(category.active && category.sla_policy_id.is_some(), "{key}");
    }
    for key in [
        "move_out_inspection",
        "legal_case",
        "owner_statement_dispute",
        "billing_dispute",
    ] {
        let category = Category::by_key(app.db(), key).await.unwrap().expect(key);
        assert!(!category.compliance_critical, "{key}");
    }
    let before = Category::query().count(app.db()).await.unwrap();
    app.kernel()
        .call("property:install", Vec::<String>::new())
        .await
        .unwrap();
    assert_eq!(Category::query().count(app.db()).await.unwrap(), before);
}

/// The Civil Defence approval is a locked legal block on `ticket.resolve` and
/// `service_contract.activate` for fire and life safety.
#[renox::test]
async fn the_civil_defence_approval_is_a_legal_block_that_cannot_be_overridden() {
    let app = booted().await;
    let kind = DocumentKind::by_key(app.db(), "civil_defence_approval")
        .await
        .unwrap()
        .expect("seeded");
    assert!(kind.legal_block && kind.compliance_critical);
    assert_eq!(kind.override_mode, OverrideMode::Never);
    assert_eq!(kind.look_ahead_days, 30);
    assert_eq!(kind.responsible_role.as_deref(), Some("coordinator"));
    assert!(kind.also_notify.0.contains(&"property_manager".to_owned()));

    let mut blocks = kind.blocks(app.db()).await.unwrap();
    blocks.sort_by(|a, b| a.guarded.cmp(&b.guarded));
    let guarded: Vec<&str> = blocks.iter().map(|b| b.guarded.as_str()).collect();
    assert_eq!(guarded, ["service_contract.activate", "ticket.resolve"]);
    for block in &blocks {
        assert_eq!(block.mode, BlockMode::Block);
        assert!(block.locked, "{}", block.guarded);
        let condition = block.condition().expect("a condition");
        assert_eq!(condition.field, "category");
        assert_eq!(condition.values, ["fire_life_safety"]);
    }
}

/// The compliance certificates, the registrations and the responsible roles.
#[renox::test]
async fn the_certificate_and_registration_kinds_and_the_responsible_roles_are_seeded() {
    let app = booted().await;
    for key in ["building_safety_certificate", "lift_certificate"] {
        let kind = DocumentKind::by_key(app.db(), key)
            .await
            .unwrap()
            .expect(key);
        assert!(kind.compliance_critical && !kind.legal_block, "{key}");
        assert_eq!(kind.reminder_offsets.0, [90, 30, 7]);
        assert_eq!(kind.responsible_role.as_deref(), Some("coordinator"));
        let blocks = kind.blocks(app.db()).await.unwrap();
        assert_eq!(blocks.len(), 1, "{key}");
        assert_eq!(blocks[0].mode, BlockMode::Flag);
        assert_eq!(blocks[0].guarded, "building.certify");
        assert!(blocks.iter().all(|b| !b.guarded.starts_with("lease.")));
    }
    for (key, role) in [
        ("emirates_id", "property_manager"),
        ("passport", "property_manager"),
        ("residence_visa", "property_manager"),
        ("trade_licence", "property_manager"),
        ("trn_certificate", "property_manager"),
        ("signed_tenancy_contract", "property_manager"),
        ("ejari_certificate", "property_manager"),
        ("tenant_acceptance", "property_manager"),
        ("repossession_basis", "property_manager"),
        ("judgment", "property_manager"),
        ("execution_record", "property_manager"),
        ("management_agreement", "property_manager"),
        ("service_contract", "coordinator"),
        ("service_report", "coordinator"),
    ] {
        let kind = DocumentKind::by_key(app.db(), key)
            .await
            .unwrap()
            .expect(key);
        assert_eq!(kind.responsible_role.as_deref(), Some(role), "{key}");
    }
    // The tenant's documents guard `submit` by the kind of tenant.
    let emirates_id = DocumentKind::by_key(app.db(), "emirates_id")
        .await
        .unwrap()
        .unwrap();
    let blocks = emirates_id.blocks(app.db()).await.unwrap();
    assert_eq!(blocks[0].guarded, "lease.submit");
    assert_eq!(blocks[0].condition().unwrap().values, ["person"]);

    // The registrations: on for 01, responsible role the company admin.
    let trademark = DocumentKind::by_key(app.db(), "trademark_registration")
        .await
        .unwrap()
        .unwrap();
    assert!(trademark.enabled && trademark.blocks(app.db()).await.unwrap().is_empty());
    assert_eq!(trademark.reminder_offsets.0, [180, 90, 30]);
    assert_eq!(trademark.responsible_role.as_deref(), Some("admin"));
    let other = DocumentKind::by_key(app.db(), "other_certification")
        .await
        .unwrap()
        .unwrap();
    assert!(other.enabled);
    assert_eq!(other.reminder_offsets.0, [90, 30, 7]);
    assert_eq!(other.responsible_role.as_deref(), Some("admin"));
    assert_eq!(other.subject_types.0[0], "kernel.legal_entity");
    let halal = DocumentKind::by_key(app.db(), "halal_certificate")
        .await
        .unwrap()
        .unwrap();
    assert!(!halal.enabled, "a kind 01 does not use stays off");
}

/// With the approval expired, resolving a fire and life safety ticket of the
/// vendor is refused with no override; activating a fire and life safety
/// contract is refused; a general ticket resolves.
#[renox::test]
async fn an_expired_civil_defence_approval_refuses_fire_safety_work_but_not_other_work() {
    let app = booted().await;
    let v = vendor(&app, "SafeFire LLC").await;
    let expired = today() - Days::new(2);
    approval(&app, v, expired).await;
    let state = app.state();

    let fire = ticket(&app, "fire_life_safety", v).await;
    let refused = documents::guard(state)
        .check(state, "ticket.resolve", fire, today())
        .await
        .unwrap()
        .unwrap_err();
    let message = refused.message();
    assert!(
        message.contains(&format!("Civil Defence approval expired on {expired}")),
        "{message}"
    );
    assert!(message.contains("override: not allowed"), "{message}");

    let general = ticket(&app, "general", v).await;
    let allowed = documents::guard(state)
        .check(state, "ticket.resolve", general, today())
        .await
        .unwrap();
    assert!(
        allowed.is_ok(),
        "a cleaning ticket of the same vendor resolves"
    );

    let contract = draft_contract(&app, v, "fire_life_safety").await;
    let refused = documents::guard(state)
        .check(state, "service_contract.activate", contract.id, today())
        .await
        .unwrap()
        .unwrap_err();
    assert!(
        refused.message().contains("Civil Defence approval"),
        "{}",
        refused.message()
    );
    assert!(refused.message().contains("override: not allowed"));

    // A valid approval lets the same work through, and lifts need none.
    let valid = vendor(&app, "Firewatch LLC").await;
    approval(&app, valid, today() + Days::new(200)).await;
    let ok = ticket(&app, "fire_life_safety", valid).await;
    assert!(
        documents::guard(state)
            .check(state, "ticket.resolve", ok, today())
            .await
            .unwrap()
            .is_ok()
    );
    let lifts = ticket(&app, "lifts", v).await;
    assert!(
        documents::guard(state)
            .check(state, "ticket.resolve", lifts, today())
            .await
            .unwrap()
            .is_ok()
    );
}

/// The `FutureWork` provider lists the vendor's visits planned after the
/// expiry date, and no others.
#[renox::test]
async fn the_look_ahead_lists_the_vendors_visits_planned_after_the_expiry() {
    let app = booted().await;
    let v = vendor(&app, "SafeFire LLC").await;
    let expires = today() + Days::new(20);
    approval(&app, v, expires).await;
    let state = app.state();

    let before = ticket(&app, "fire_life_safety", v).await;
    let after = ticket(&app, "fire_life_safety", v).await;
    let cleaning = ticket(&app, "general", v).await;
    for (id, due) in [(before, 10), (after, 25), (cleaning, 25)] {
        Ticket::where_eq("id", id)
            .update(
                app.db(),
                &[("planned_due_date", &Some(today() + Days::new(due)))],
            )
            .await
            .unwrap();
    }

    let entries = wl_documents::blocks_for(app.db(), "ticket.resolve")
        .await
        .unwrap();
    let entry = entries
        .iter()
        .find(|e| e.kind == "civil_defence_approval")
        .expect("the Civil Defence entry");
    let work = documents::future_work("ticket.resolve").expect("the provider is registered");
    let planned = work
        .planned_from(state, entry, &v.subject(), expires + Days::new(1))
        .await
        .unwrap();
    let ids: Vec<i64> = planned.iter().map(|p| p.record.subject_id).collect();
    assert_eq!(ids, [after], "only the fire visit planned after the expiry");
    assert_eq!(planned[0].date, today() + Days::new(25));
    assert!(documents::future_work("service_contract.activate").is_some());
}

/// The compliance-critical rules are marked, and a failed critical run alerts
/// each company admin once.
#[renox::test]
async fn a_failed_critical_rule_alerts_the_company_admins_once() {
    let app = booted().await;
    for rule in [
        "PRP.RUL.09",
        "WRK.RUL.14",
        "WRK.RUL.18",
        "DOC.RUL.01",
        "DOC.RUL.02",
        "DOC.RUL.03",
    ] {
        assert!(is_critical(rule), "{rule}");
    }
    assert!(!is_critical("PRP.RUL.12"), "a reminder is not critical");

    let first = person(&app, "ada@example.com", ADMIN).await;
    let second = person(&app, "omar@example.com", ADMIN).await;
    let told = recipients(app.db(), "admin").await.unwrap();
    let mut unique = told.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        told.len(),
        unique.len(),
        "the owner role and the admin are one role: told once"
    );
    assert!(told.contains(&first.id) && told.contains(&second.id));

    let raised = Raised::new(
        "PRP.RUL.09",
        schedule_subject(),
        "PRP.RUL.09:2027-01-10",
        AlertKind::Failed,
    )
    .error("the renewal run failed");
    let alert = raise(app.state(), raised.clone())
        .await
        .unwrap()
        .expect("an alert");
    assert_eq!(alert.owner_role, "admin");
    assert!(
        raise(app.state(), raised).await.unwrap().is_none(),
        "one alert per occurrence"
    );
}

/// The notification types take their group's give-up reaction.
#[renox::test]
async fn the_notification_types_take_the_reaction_of_their_group() {
    let _app = booted().await;
    let reactions = |key: &str| {
        let def = notification_type(key).unwrap_or_else(|| panic!("{key} is registered"));
        (def.give_up.clone(), def.staff_role.clone(), def.statutory)
    };
    let portal = vec![
        GiveUpReaction::MarkContactInvalid,
        GiveUpReaction::StaffTask,
    ];
    for key in [
        "lease.contract_for_signature",
        "lease.renewal_window",
        "lease.notice_withdrawn",
    ] {
        assert_eq!(
            reactions(key),
            (portal.clone(), Some("property_manager".to_owned()), false),
            "{key}"
        );
    }
    let statutory = vec![
        GiveUpReaction::MarkContactInvalid,
        GiveUpReaction::StatutoryNotice,
    ];
    for key in [
        "lease.notice_of_changed_terms",
        "lease.landlord_notice",
        "lease.payment_demand",
    ] {
        assert_eq!(
            reactions(key),
            (statutory.clone(), Some("property_manager".to_owned()), true),
            "{key}"
        );
    }
    let statements = vec![
        GiveUpReaction::MarkContactInvalid,
        GiveUpReaction::AlternativeRecipient,
        GiveUpReaction::StaffTask,
    ];
    for key in [
        "lease.new_payee",
        "owner_statement.published",
        "owner_statement.revision_follows",
        "owner_statement.dispute_outcome",
    ] {
        assert_eq!(
            reactions(key),
            (statements.clone(), Some("accountant".to_owned()), false),
            "{key}"
        );
    }
    for key in [
        "property.lease_unsigned",
        "property.renewal_due",
        "property.holding_over",
    ] {
        assert_eq!(
            reactions(key).0,
            vec![GiveUpReaction::None],
            "a staff-only type creates no task: {key}"
        );
    }
}

/// A ticket update entered 3 hours late keeps the work time and carries the
/// late-entry flag with its reason; one without a reason is refused; one more
/// than 24 hours late waits for approval.
#[renox::test]
async fn a_late_ticket_update_keeps_the_work_time_and_carries_the_flag() {
    let app = booted().await;
    let technician = person(&app, "tariq@example.com", ADMIN).await;
    let v = vendor(&app, "SafeFire LLC").await;
    let id = ticket(&app, "general", v).await;
    let state = app.state();

    let worked = renox::db::now() - renox::chrono::Duration::hours(3);
    let refused =
        ticket_update::enter(state, technician.id, id, worked, "Replaced the valve", None)
            .await
            .expect_err("a late entry without a reason is refused");
    assert!(
        matches!(refused, Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)),
        "{refused:?}"
    );

    let entered = ticket_update::enter(
        state,
        technician.id,
        id,
        worked,
        "Replaced the valve",
        Some("No signal in the basement"),
    )
    .await
    .unwrap();
    let record = match entered {
        Entered::Recorded { record_id, .. } => record_id,
        _ => panic!("a flagged entry is recorded"),
    };
    let update = TicketUpdate::find(app.db(), record).await.unwrap().unwrap();
    assert_eq!(
        update.worked_at, worked,
        "the work time is the business time"
    );
    assert!(update.late && update.entered_at > update.worked_at);
    assert_eq!(
        update.late_reason.as_deref(),
        Some("No signal in the basement")
    );
    assert_eq!(update.ticket_id, id);

    let on_time = ticket_update::enter(state, technician.id, id, renox::db::now(), "Started", None)
        .await
        .unwrap();
    assert!(matches!(on_time, Entered::OnTime { .. }));

    let days_ago = renox::db::now() - renox::chrono::Duration::hours(30);
    let waiting = ticket_update::enter(
        state,
        technician.id,
        id,
        days_ago,
        "Inspected",
        Some("Phone lost"),
    )
    .await
    .unwrap();
    assert!(matches!(waiting, Entered::Pending { .. }));
    assert_eq!(
        TicketUpdate::of_ticket(app.db(), id).await.unwrap().len(),
        2,
        "the pending one is not created yet"
    );
}

/// The lead sources, the campaign figures' approval, the checklists and the SOP
/// library are seeded.
#[renox::test]
async fn the_lead_sources_checklists_and_sops_are_seeded() {
    let app = booted().await;
    for code in [
        "referral", "walk_in", "website", "social", "paid_ads", "partner", "event", "other",
    ] {
        let source = LeadSource::by_code(app.db(), code)
            .await
            .unwrap()
            .expect(code);
        assert!(source.active, "{code}");
    }
    let aggregator = LeadSource::by_code(app.db(), "aggregator")
        .await
        .unwrap()
        .unwrap();
    assert!(!aggregator.active, "01 does not use the aggregator source");

    for (name, role) in [
        ("Property manager onboarding", "property_manager"),
        ("Leasing agent onboarding", "leasing_agent"),
        ("Accountant onboarding", "accountant"),
        ("Maintenance coordinator onboarding", "coordinator"),
        ("Technician onboarding", "technician"),
    ] {
        let template = OnboardingTemplate::query()
            .where_(OnboardingTemplate::NAME_EN.eq(name))
            .first(app.db())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{name}"));
        assert_eq!(template.job_roles, role);
        assert!(
            !template.blocks_activation,
            "the role is active before the checklist is complete"
        );
        let steps = template.steps(app.db()).await.unwrap();
        let owners: Vec<&str> = steps.iter().map(|s| s.owner.as_str()).collect();
        assert!(
            owners.contains(&"role:admin")
                && owners.contains(&"manager")
                && owners.contains(&"employee"),
            "{name}"
        );
    }

    let leasing = sop(&app, "SOP-PRP-LEASING").await;
    assert_eq!(leasing.owner_role, "property_manager");
    assert_eq!(leasing.ack_due_days, 7);
    let readers: Vec<String> = SopReader::query()
        .where_(SopReader::SOP_ID.eq(leasing.id))
        .get(app.db())
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.role_key)
        .collect();
    assert!(readers.contains(&"leasing_agent".to_owned()));
    assert_eq!(sop(&app, "SOP-PRP-CHEQUES").await.owner_role, "accountant");
    assert_eq!(
        sop(&app, "SOP-PRP-OWNER-STATEMENTS").await.owner_role,
        "accountant"
    );
}

/// The source and the campaign of an enquiry reach the tenant's party when it
/// converts; an unknown source is refused.
#[renox::test]
async fn an_enquiry_hands_its_source_to_the_tenant_party() {
    let app = booted().await;
    let state = app.state();
    let refused = enquiry::record(
        app.db(),
        NewEnquiry {
            prospect_name: "Layla".into(),
            source: "aggregator".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        refused,
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)
    ));

    let recorded = enquiry::record(
        app.db(),
        NewEnquiry {
            prospect_name: "Layla".into(),
            contact: "+971500000400".into(),
            source: "walk_in".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(recorded.source, "walk_in");

    let tenant = create(
        state,
        NewParty {
            kind: PartyKind::Person,
            legal_name_en: "Layla Haddad".into(),
            person: Some(PersonDetails::default()),
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id();
    let converted = enquiry::convert(state, recorded.id, tenant).await.unwrap();
    assert_eq!(converted.converted_party_id, Some(tenant.0));
    let acquisition = PartyAcquisition::of_party(app.db(), tenant)
        .await
        .unwrap()
        .expect("the party has an acquisition");
    let source = LeadSource::by_code(app.db(), "walk_in")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(acquisition.source_id, source.id);
    assert_eq!(acquisition.set_from, "lead");
    assert!(
        enquiry::convert(state, recorded.id, tenant).await.is_err(),
        "it converts once"
    );
}

/// The settings page shows each setting with its value, who set it and the
/// spec reference, and flags the open items.
#[renox::test]
async fn an_admin_sees_the_compliance_settings_of_the_property_client() {
    let app = booted().await;
    let admin = person(&app, "admin2@example.com", ADMIN).await;
    app.acting_as(&admin);
    app.get("/settings/compliance")
        .await
        .assert_ok()
        .assert_see("Compliance, documents and notices")
        .assert_see("Civil Defence approval")
        .assert_see("look-ahead 30 days")
        .assert_see("legal block")
        .assert_see("fire_life_safety")
        .assert_see("Property seed (BL-PRP-27)")
        .assert_see("default of the module")
        .assert_see("L-19")
        .assert_see("AU-08")
        .assert_see("lease.payment_demand")
        .assert_see("owner property_manager");
}

/// A member does not see the settings.
#[renox::test]
async fn a_member_does_not_see_the_compliance_settings() {
    let app = booted().await;
    let member = person(&app, "member2@example.com", MEMBER).await;
    app.acting_as(&member);
    app.get("/settings/compliance").await.assert_forbidden();
}
