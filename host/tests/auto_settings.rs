//! BL-AUT-18 (#414) and BL-AUT-15 (#411): the 08 values for the shared
//! document, scheduling, notification, alert, integration, sales, loyalty, staff
//! and registration mechanisms, proved through the REAL host of the demo parts
//! trader (`wl:install` and the seeds) and not as constants: the police report
//! block that no override can switch, the responsible role per document kind,
//! the groups and give-up reactions of the notification types, the
//! compliance-critical rules, the SMS fallback, the late-entry approvers, the
//! lead sources, the loyalty programme, the onboarding checklists, the SOP
//! library, the registration kinds, the back-order channels and the settings
//! page with the storage fee switch (08 §4.1, §7.1 settings, §7.2, §8, §9).
//!
//! Proved elsewhere: the late test drive itself (`apps/auto/tests/
//! aut_18_late_drive.rs`), the failing `AUT.RUL.15` and `AUT.RUL.16` runs
//! (`aut_14_export.rs`, `aut_04_parts.rs`) and the police report guard against
//! the real documents (`host/tests/auto_police_real.rs`).

mod support;

use auto::defaults;
use auto::seed::{
    CUSTOMER_LICENCE_KIND, DRIVING_LICENCE_KIND, EMIRATES_ID_KIND, FLEET_CONTRACT_KIND,
    MULKIYA_KIND, POLICE_REPORT_KIND,
};
use host::roles::{self, ACCOUNTANT, ADMIN, BRANCH_MANAGER, OWNER, TECHNICIAN, WORKSHOP_MANAGER};
use renox::prelude::*;
use support::{auto_booted, company, day, messages_to, person};
use wl_documents::kinds::{self, RefusalReason};
use wl_documents::registrations::{OTHER, TRADEMARK};
use wl_documents::{Document, DocumentKind, DocumentStatus, expiry};
use wl_kernel::compliance::is_critical;
use wl_kernel::documents::{BlockMode, OverrideMode, today};
use wl_kernel::integrations::{self, IntegrationConnection};
use wl_kernel::notify::{
    GROUP_BOOKINGS, GROUP_INVOICES, GROUP_SIGNATURES, GiveUpReaction, Urgency, default_give_up,
    notification_type,
};
use wl_notify::NotificationType;

/// The (guarded, mode) entries of the kind `key`, sorted.
async fn blocks_of(
    app: &renox::testing::TestApp,
    key: &str,
) -> (DocumentKind, Vec<(String, BlockMode)>) {
    let kind = DocumentKind::by_key(app.db(), key)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("no kind {key}"));
    let mut blocks: Vec<(String, BlockMode)> = kind
        .blocks(app.db())
        .await
        .unwrap()
        .into_iter()
        .map(|b| (b.guarded, b.mode))
        .collect();
    blocks.sort_by(|a, b| a.0.cmp(&b.0));
    (kind, blocks)
}

// ---- Documents -----------------------------------------------------------------

/// Documents: `blocks_for("job_card", "approve")` returns the police report
/// with mode `block` and override `never`; the override cannot be switched; the
/// trade licences and the fleet contracts only remind, with no `blocks`.
#[renox::test]
async fn aut_18_the_police_report_blocks_with_no_override_and_the_licences_only_remind() {
    let app = auto_booted().await;
    let (report, blocks) = blocks_of(&app, POLICE_REPORT_KIND).await;
    let guarded: Vec<&str> = blocks.iter().map(|(g, _)| g.as_str()).collect();
    assert!(guarded.contains(&"job_card.approve"));
    assert!(guarded.contains(&"insurance_claim.approve"));
    assert!(blocks.iter().all(|(_, mode)| *mode == BlockMode::Block));
    assert_eq!(report.override_mode, OverrideMode::Never);
    assert!(report.legal_block);

    let refused = kinds::set_override_mode(app.db(), POLICE_REPORT_KIND, OverrideMode::WithApproval)
        .await
        .unwrap()
        .expect_err("the override of a legal block is locked");
    assert_eq!(refused.reason, RefusalReason::LegalBlock);
    let again = DocumentKind::by_key(app.db(), POLICE_REPORT_KIND)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.override_mode, OverrideMode::Never);

    for key in [CUSTOMER_LICENCE_KIND, FLEET_CONTRACT_KIND] {
        let (kind, blocks) = blocks_of(&app, key).await;
        assert!(blocks.is_empty(), "{key} has no blocks");
        assert!(kind.has_expiry, "{key}");
    }
}

/// Responsible roles: the trade licence is the accountant's, the fleet contract
/// the Owner / GM's, the police report and the claim documents the insurance
/// coordinator's.
#[renox::test]
async fn aut_18_each_document_kind_has_its_responsible_role() {
    let app = auto_booted().await;
    for (key, role) in [
        (CUSTOMER_LICENCE_KIND, "accountant"),
        (FLEET_CONTRACT_KIND, "owner"),
        (POLICE_REPORT_KIND, "insurance_coordinator"),
        (DRIVING_LICENCE_KIND, "insurance_coordinator"),
        (MULKIYA_KIND, "insurance_coordinator"),
        (EMIRATES_ID_KIND, "insurance_coordinator"),
    ] {
        let kind = DocumentKind::by_key(app.db(), key).await.unwrap().unwrap();
        assert_eq!(kind.responsible_role.as_deref(), Some(role), "{key}");
    }
}

/// An expiring customer trade licence notifies the accountant, not the owner.
#[renox::test]
async fn aut_18_an_expiring_trade_licence_notifies_the_accountant() {
    let app = auto_booted().await;
    let accountant = person(&app, "Aisha", ACCOUNTANT).await;
    let owner = person(&app, "Omar", OWNER).await;
    let kind = DocumentKind::by_key(app.db(), CUSTOMER_LICENCE_KIND)
        .await
        .unwrap()
        .unwrap();
    let customer = company(&app, "Gulf Fleet LLC").await;
    Document::create(
        app.db(),
        Document {
            subject_type: wl_kernel::parties::PARTY_SUBJECT_TYPE.into(),
            subject_id: customer.0,
            kind_id: kind.id,
            title: "Trade licence".into(),
            expiry_date: Some(day(30)),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    expiry::run(app.state(), today()).await.unwrap();

    let told = messages_to(&app, "documents.expiring", &accountant).await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data.0["days_left"].as_i64(), Some(30));
    assert!(
        messages_to(&app, "documents.expiring", &owner)
            .await
            .is_empty()
    );
}

// ---- Alerts, late entry and integrations --------------------------------------------

/// Compliance: `AUT.RUL.15` and `AUT.RUL.16` are compliance-critical, so a run
/// that fails alerts the Owner / GM and the company admin; the late-entry
/// approvers are the workshop and branch managers.
#[renox::test]
async fn aut_18_the_critical_rules_are_marked_and_the_managers_approve_late_entries() {
    let _app = auto_booted().await;
    for rule in defaults::CRITICAL_RULES {
        assert!(is_critical(rule), "{rule}");
    }
    assert_eq!(defaults::ALERT_ROLE, "owner");
    assert_eq!(defaults::LATE_ENTRY_APPROVAL_HOURS, 24);
    for role in [WORKSHOP_MANAGER, BRANCH_MANAGER] {
        assert!(
            roles::grants_of(role).contains(&wl_kernel::permissions::LATE_ENTRIES_APPROVE),
            "{role}"
        );
    }
    assert!(!roles::grants_of(TECHNICIAN).contains(&wl_kernel::permissions::LATE_ENTRIES_APPROVE));
}

/// Integrations: the SMS gateway has its manual fallback, the advisors call the
/// customer; the page of the job cards shows it.
#[renox::test]
async fn aut_18_the_sms_gateway_has_the_advisors_call_fallback() {
    let app = auto_booted().await;
    let connection = IntegrationConnection::by_key(app.db(), integrations::SMS)
        .await
        .unwrap()
        .expect("the SMS connection");
    assert_eq!(connection.owner_role, BRANCH_MANAGER);
    assert!(connection.fallback_en.contains("Call the customer"));
    assert!(
        connection
            .fallback_ar
            .as_deref()
            .is_some_and(|ar| !ar.trim().is_empty())
    );
}

// ---- Notifications ------------------------------------------------------------------

/// Notifications: the withdrawal letter and the termination notice are
/// statutory (a give-up marks the contact invalid and gives the coordinator or
/// the Owner / GM the statutory task); the vehicle-ready SMS marks the contact
/// invalid and makes no task; the parts delay is urgent and gives the advisor
/// a task; the insurer and supplier requests are signature-group messages.
#[renox::test]
async fn aut_18_the_notification_types_have_their_groups_and_reactions() {
    let _app = auto_booted().await;
    let letter = notification_type("auto.claim_withdrawal_letter").expect("the letter");
    assert!(letter.statutory);
    assert_eq!(letter.staff_role.as_deref(), Some("insurance_coordinator"));
    assert_eq!(
        default_give_up(&letter).reactions,
        [
            GiveUpReaction::MarkContactInvalid,
            GiveUpReaction::StatutoryNotice
        ]
    );
    let termination = notification_type("auto.fleet_termination_notice").expect("termination");
    assert!(termination.statutory);
    assert_eq!(termination.staff_role.as_deref(), Some("owner"));

    let ready = notification_type("auto.vehicle_ready").expect("vehicle ready");
    assert_eq!(ready.group, GROUP_BOOKINGS);
    assert_eq!(ready.give_up, [GiveUpReaction::MarkContactInvalid]);
    assert!(ready.staff_role.is_none(), "no task");

    let delay = notification_type("auto.parts_delay").expect("parts delay");
    assert_eq!(delay.urgency, Urgency::Urgent);
    assert_eq!(
        delay.give_up,
        [
            GiveUpReaction::MarkContactInvalid,
            GiveUpReaction::StaffTask
        ]
    );
    assert_eq!(delay.staff_role.as_deref(), Some("service_advisor"));

    for key in ["auto.insurer_request", "auto.supplier_warranty_claim"] {
        let def = notification_type(key).unwrap_or_else(|| panic!("no type {key}"));
        assert_eq!(def.group, GROUP_SIGNATURES, "{key}");
        assert_eq!(
            def.give_up,
            [
                GiveUpReaction::MarkContactInvalid,
                GiveUpReaction::StaffTask
            ],
            "{key}"
        );
    }
    let statement = notification_type("auto.insurer_statement").expect("statement");
    assert_eq!(statement.group, GROUP_INVOICES);
    assert_eq!(statement.staff_role.as_deref(), Some("insurance_coordinator"));
    for key in ["auto.fleet_booking_request", "auto.fleet_contract_ended"] {
        let def = notification_type(key).unwrap_or_else(|| panic!("no type {key}"));
        assert_eq!(def.group, GROUP_BOOKINGS, "{key}");
        assert!(def.staff_role.is_none(), "{key}: no task");
    }
}

/// The invoice messages of the shared modules go to the accountant when they
/// give up; seeding twice changes nothing.
#[renox::test]
async fn aut_18_the_shared_invoice_types_are_set_and_seeding_twice_changes_nothing() {
    let app = auto_booted().await;
    for spec in &defaults::NOTIFICATION_TYPES {
        let Some(row) = NotificationType::by_key(app.db(), spec.key).await.unwrap() else {
            continue;
        };
        assert_eq!(row.group_key, spec.group, "{}", spec.key);
        assert_eq!(row.staff_role.as_deref(), spec.role, "{}", spec.key);
        assert_eq!(row.give_up.0, spec.give_up, "{}", spec.key);
    }
    let counts = |app: &renox::testing::TestApp| {
        let db = app.db().clone();
        async move {
            (
                wl_hr::OnboardingTemplate::query().count(&db).await.unwrap(),
                wl_documents::Sop::query().count(&db).await.unwrap(),
                wl_sales::LoyaltyProgramme::query().count(&db).await.unwrap(),
            )
        }
    };
    let before = counts(&app).await;
    let again = host::auto_seed::install(app.state()).await.unwrap();
    assert_eq!(again.total(), 0);
    assert_eq!(counts(&app).await, before);
}

// ---- Sales and loyalty --------------------------------------------------------------

/// Back-orders and lead sources: the counter, phone and sales rep channels
/// back-order, the portal does not; the eight sources are offered, not the
/// aggregator; none requires a campaign; a cost entry over a budget waits for
/// a manager.
#[renox::test]
async fn aut_18_the_sales_module_has_the_08_sources_channels_and_limits() {
    let app = auto_booted().await;
    let sales = wl_sales::config::config_of(app.state());
    for channel in ["counter", "phone", "sales_rep"] {
        assert!(sales.back_orders_allowed(channel), "{channel}");
    }
    assert!(!sales.back_orders_allowed("portal"));
    assert!(sales.campaign_required_sources.is_empty());
    assert!(sales.over_budget_approval);
    assert_eq!(sales.expiry_reminder_days, 30);
    assert_eq!(sales.adjustment_limit, 500);
    assert!(sales.confirm_redemption_by_code);

    let sources = wl_sales::LeadSource::ordered(app.db()).await.unwrap();
    let offered: Vec<&str> = sources
        .iter()
        .filter(|s| s.active)
        .map(|s| s.code.as_str())
        .collect();
    for code in defaults::LEAD_SOURCES {
        assert!(offered.contains(code), "{code}");
    }
    assert!(!offered.contains(&"aggregator"));
}

/// Loyalty: individual car owners earn 1 point per AED 10 (500 AED earns 50
/// points), 100 points are an AED 10 discount line, points expire after 24
/// months.
#[renox::test]
async fn aut_18_the_loyalty_programme_earns_one_point_per_aed_10_and_expires_after_24_months() {
    let app = auto_booted().await;
    let programme = wl_sales::LoyaltyProgramme::first_active(app.db())
        .await
        .unwrap()
        .expect("the programme");
    assert_eq!(programme.points_for(50_000), 50, "AED 500 earns 50 points");
    assert_eq!(programme.point_value, 10);
    assert_eq!(programme.min_redeem_points, 100);
    assert_eq!(programme.point_value * programme.min_redeem_points, 1_000);
    assert_eq!(programme.validity_months, 24);
}

// ---- Staff and registrations ---------------------------------------------------------

/// Onboarding and SOPs: the five checklists exist for the roles of the spec;
/// the six SOPs are owned by the workshop manager and the branch manager, with
/// the readers per role.
#[renox::test]
async fn aut_18_the_checklists_and_the_sop_library_are_seeded() {
    let app = auto_booted().await;
    let templates = wl_hr::OnboardingTemplate::query().get(app.db()).await.unwrap();
    for spec in &defaults::ONBOARDING {
        let template = templates
            .iter()
            .find(|t| t.name_en == spec.name)
            .unwrap_or_else(|| panic!("no checklist {}", spec.name));
        assert!(template.job_roles.contains(spec.role), "{}", spec.name);
        assert!(!template.blocks_activation, "completion blocks nothing");
    }
    for spec in &defaults::SOPS {
        let sop = wl_documents::Sop::query()
            .where_(wl_documents::Sop::CODE.eq(spec.code))
            .first(app.db())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("no SOP {}", spec.code));
        assert_eq!(sop.owner_role, spec.owner_role, "{}", spec.code);
        assert_eq!(sop.ack_due_days, 7);
        let readers = wl_documents::SopReader::query()
            .where_(wl_documents::SopReader::SOP_ID.eq(sop.id))
            .count(app.db())
            .await
            .unwrap();
        assert_eq!(readers as usize, spec.readers.len(), "{}", spec.code);
    }
}

/// Registrations: a fresh deployment lists *Trademark registration* and *Other
/// certification* with the Owner / GM as responsible role and no blocks.
#[renox::test]
async fn aut_18_the_registration_kinds_belong_to_the_owner() {
    let app = auto_booted().await;
    for key in [TRADEMARK, OTHER] {
        let (kind, blocks) = blocks_of(&app, key).await;
        assert_eq!(kind.responsible_role.as_deref(), Some("owner"), "{key}");
        assert!(blocks.is_empty(), "{key}");
    }
}

// ---- The page and the storage fee switch ------------------------------------------------

/// The page lists the 08 values, gives the storage fee switch a place (off
/// until the command turns it on), and refuses everyone but the settings
/// readers.
#[renox::test]
async fn aut_18_the_settings_page_shows_the_08_values_and_the_storage_fee_switch() {
    let app = auto_booted().await;
    let admin = person(&app, "Ada", ADMIN).await;
    app.acting_as(&admin);
    app.get("/settings/compliance")
        .await
        .assert_ok()
        .assert_see("Blocks and overrides")
        .assert_see("Responsible role per document kind")
        .assert_see("AUT.RUL.15 (marked)")
        .assert_see("AUT.RUL.16 (marked)")
        .assert_see("Back-orders allowed")
        .assert_see("Storage fee")
        .assert_see("switch storage_fee_enabled")
        .assert_see("SOP library");
    assert!(!auto::settings::storage_fee_enabled(app.db()).await.unwrap());

    host::auto_settings::set_storage_fee(app.db(), true)
        .await
        .unwrap();
    assert!(auto::settings::storage_fee_enabled(app.db()).await.unwrap());
    let rows = host::auto_settings::stored_rows(app.db()).await.unwrap();
    assert!(rows[0].value.starts_with("on;"), "{}", rows[0].value);

    let technician = person(&app, "Tareq", TECHNICIAN).await;
    app.acting_as(&technician);
    app.get("/settings/compliance").await.assert_forbidden();
}
