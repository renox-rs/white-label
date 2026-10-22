//! BL-EDU-17 (#452): the 10 values for the shared document, scheduling, party,
//! notification, alert, integration, sales, loyalty, staff and registration
//! mechanisms, proved through the real host of the demo training centre
//! (`wl:install` and the seeds) and not as constants: the document kinds with
//! their `blocks` and override modes, the look-ahead windows and responsible
//! roles, the scheduling numbers and the one-to-one service type, the block
//! effects, the groups and give-up reactions of the notification types, the
//! compliance-critical rules, the connections with their manual fallbacks, the
//! lead sources, the loyalty programme, the onboarding checklists, the SOP
//! library, the registration kinds and the settings page (10 §4.1, §7.1
//! *Booking*, §7.2, §8, §9).
//!
//! Proved elsewhere: the booking refusal of a tutor without a clearance and
//! the freelance permit (`apps/edtech/tests/edu_17_settings.rs`, the real
//! wl-scheduling), the correction of paper attendance after the tutor's 24
//! hours with a reason (`edu_14_correct_attendance.rs`), the e-invoice still
//! issued and sent as PDF while the ASP is down (`money_einvoice.rs`).

mod support;

use edtech::notices::{FAMILY_NOTICE, NOTICE, TERMS_REQUEST};
use edtech::seed::{
    CENTRE_PERMIT_KIND, COURSE_APPROVAL_KIND, ONE_TO_ONE_SERVICE, POLICE_CLEARANCE_KIND,
    TUTOR_PERMIT_KIND,
};
use host::roles::{
    self, ACADEMIC_COORDINATOR, ACCOUNTANT, ADMIN, CENTRE_MANAGER, COUNSELLOR, OWNER,
};
use renox::auth::permissions::Scope;
use renox::prelude::*;
use support::{
    BouncingMail, day, edtech_booted, edtech_booted_with_mail, messages_to, person, tenant,
};
use wl_documents::kinds::{self, RefusalReason};
use wl_documents::registrations::{OTHER, TRADEMARK};
use wl_documents::{Document, DocumentKind, DocumentStatus, expiry};
use wl_kernel::approvals::{ApprovalPolicy, Step};
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::block_effects::{BlockEffect, effect_for};
use wl_kernel::compliance::is_critical;
use wl_kernel::compliance::service::NOTIFICATION_TYPE as OWNER_ALERT;
use wl_kernel::documents::{BlockMode, OverrideMode, today};
use wl_kernel::integrations::{
    self, ConnectionStatus, IntegrationConnection, go_down, report_failure,
};
use wl_kernel::notify::{
    self, GROUP_GUARDIAN, GROUP_INVOICES, GROUP_SIGNATURES, GiveUpReaction, NotifyRequest,
    notification_type,
};
use wl_kernel::{LegalEntity, Subject};
use wl_notify::{Message, MessageStatus, NotificationType};
use wl_scheduling::ServiceType;
use wl_ticketing::Ticket;

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

/// `(guarded, mode)` pairs, sorted by the guarded transition.
fn entries(list: &[(&str, BlockMode)]) -> Vec<(String, BlockMode)> {
    let mut entries: Vec<(String, BlockMode)> = list
        .iter()
        .map(|(guarded, mode)| ((*guarded).to_owned(), *mode))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

// ---- Documents ---------------------------------------------------------------

/// Documents: the four kinds carry the entries of the spec; switching the
/// centre permit's override (or the course approval's or the tutor permit's)
/// is refused; switching the police clearance to `with_approval` works, and
/// the owner decides the override, never the requester.
#[renox::test]
async fn edu_17_the_four_kinds_carry_their_entries_and_only_the_clearance_may_be_switched() {
    let app = edtech_booted().await;

    let (permit, blocks) = blocks_of(&app, CENTRE_PERMIT_KIND).await;
    assert_eq!(
        blocks,
        entries(&[
            ("batch.open", BlockMode::Block),
            ("certificate.issue", BlockMode::Block)
        ])
    );
    assert_eq!(permit.override_mode, OverrideMode::Never);
    let (approval, blocks) = blocks_of(&app, COURSE_APPROVAL_KIND).await;
    assert_eq!(
        blocks,
        entries(&[
            ("batch.open", BlockMode::Block),
            ("certificate.issue", BlockMode::Block)
        ])
    );
    assert_eq!(approval.override_mode, OverrideMode::Never);
    let tutor_blocks = entries(&[
        ("batch.open", BlockMode::Block),
        ("batch.change_tutor", BlockMode::Block),
        ("session.change_tutor", BlockMode::Block),
        ("session.reschedule", BlockMode::Block),
        ("booking.book", BlockMode::Block),
        ("booking.reassign", BlockMode::Block),
        ("session.submit_attendance", BlockMode::Flag),
    ]);
    let (clearance, blocks) = blocks_of(&app, POLICE_CLEARANCE_KIND).await;
    assert_eq!(blocks, tutor_blocks);
    assert_eq!(clearance.override_mode, OverrideMode::Never);
    let (visa, blocks) = blocks_of(&app, TUTOR_PERMIT_KIND).await;
    assert_eq!(blocks, tutor_blocks);
    assert_eq!(visa.override_mode, OverrideMode::Never);

    // The centre permit, the course approval and the freelance permit or visa
    // cannot be switched.
    for key in [CENTRE_PERMIT_KIND, COURSE_APPROVAL_KIND, TUTOR_PERMIT_KIND] {
        let refused = kinds::set_override_mode(app.db(), key, OverrideMode::WithApproval)
            .await
            .unwrap()
            .expect_err("the override mode is fixed");
        assert_eq!(refused.reason, RefusalReason::OverrideFixed, "{key}");
        let again = DocumentKind::by_key(app.db(), key).await.unwrap().unwrap();
        assert_eq!(again.override_mode, OverrideMode::Never, "{key}");
    }
    let admin = person(&app, "Ada", ADMIN).await;
    app.acting_as(&admin);
    app.htmx()
        .post(
            &format!("/document-kinds/{CENTRE_PERMIT_KIND}/override-mode"),
            &[("mode", "with_approval")],
        )
        .await
        .assert_status(422);

    // The police clearance may be switched, and back.
    let switched =
        kinds::set_override_mode(app.db(), POLICE_CLEARANCE_KIND, OverrideMode::WithApproval)
            .await
            .unwrap()
            .expect("the client may switch the police clearance");
    assert_eq!(switched.override_mode, OverrideMode::WithApproval);
    kinds::set_override_mode(app.db(), POLICE_CLEARANCE_KIND, OverrideMode::Never)
        .await
        .unwrap()
        .expect("and back");

    // The owner decides an override, never the one who asked for it.
    let policies = ApprovalPolicy::query()
        .where_(ApprovalPolicy::ACTION.eq("documents.block_override"))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(policies.len(), 1);
    assert_eq!(policies[0].steps.0, [Step::one_of(["owner", "admin"])]);
    assert!(policies[0].sod, "the requester never approves");
    assert!(roles::grants_of(OWNER).contains(&wl_kernel::permissions::APPROVALS_VIEW));
}

/// Look-ahead: a course approval expiring in 60 days produces
/// `DocumentExpiring` (the reminder reaches the academic coordinator, its
/// responsible role); a police clearance at 30 days does too; the windows are
/// 60 and 30 days.
#[renox::test]
async fn edu_17_a_course_approval_at_60_days_and_a_clearance_at_30_days_are_told() {
    let app = edtech_booted().await;
    let coordinator = person(&app, "Amal", ACADEMIC_COORDINATOR).await;
    let manager = person(&app, "Mona", CENTRE_MANAGER).await;
    for (key, window) in [
        (CENTRE_PERMIT_KIND, 60),
        (COURSE_APPROVAL_KIND, 60),
        (POLICE_CLEARANCE_KIND, 30),
        (TUTOR_PERMIT_KIND, 30),
    ] {
        let kind = DocumentKind::by_key(app.db(), key).await.unwrap().unwrap();
        assert_eq!(kind.look_ahead_days, window, "{key}");
    }

    let mut course = edtech::Course {
        code: "ENG-1".to_owned(),
        name_en: "English".to_owned(),
        approval_required: true,
        ..edtech::Course::default()
    };
    course.save(app.db()).await.unwrap();
    let approval = DocumentKind::by_key(app.db(), COURSE_APPROVAL_KIND)
        .await
        .unwrap()
        .unwrap();
    let clearance = DocumentKind::by_key(app.db(), POLICE_CLEARANCE_KIND)
        .await
        .unwrap()
        .unwrap();
    let layla = tenant(&app, "Layla Hassan").await;
    let tutor = wl_hr::employee::create(
        app.state(),
        wl_hr::employee::NewEmployee::new(layla, "tutor", day(-400))
            .status(wl_hr::employee::EmployeeStatus::Active),
    )
    .await
    .unwrap();
    for (subject_type, subject_id, kind_id, title, days) in [
        ("edtech.course", course.id, approval.id, "KHDA approval", 60),
        (
            "hr.employee",
            tutor.id,
            clearance.id,
            "Police clearance",
            30,
        ),
    ] {
        Document::create(
            app.db(),
            Document {
                subject_type: subject_type.into(),
                subject_id,
                kind_id,
                title: title.into(),
                expiry_date: Some(day(days)),
                current_version: 1,
                status: DocumentStatus::Accepted,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    expiry::run(app.state(), today()).await.unwrap();

    let told: Vec<i64> = messages_to(&app, "documents.expiring", &coordinator)
        .await
        .iter()
        .map(|m| m.data.0["days_left"].as_i64().expect("days left"))
        .collect();
    assert_eq!(told, [60, 30]);
    assert!(
        messages_to(&app, "documents.expiring", &manager)
            .await
            .is_empty(),
        "the centre manager renews the permit, not these"
    );
}

/// Responsible role: a centre permit expiring notifies the centre manager (not
/// the coordinator); the roles of the four kinds are the centre manager's for
/// the permit and the academic coordinator's for the others.
#[renox::test]
async fn edu_17_a_centre_permit_expiring_notifies_the_centre_manager() {
    let app = edtech_booted().await;
    let manager = person(&app, "Mona", CENTRE_MANAGER).await;
    let coordinator = person(&app, "Amal", ACADEMIC_COORDINATOR).await;
    for (key, role) in [
        (CENTRE_PERMIT_KIND, CENTRE_MANAGER),
        (COURSE_APPROVAL_KIND, ACADEMIC_COORDINATOR),
        (POLICE_CLEARANCE_KIND, ACADEMIC_COORDINATOR),
        (TUTOR_PERMIT_KIND, ACADEMIC_COORDINATOR),
    ] {
        let kind = DocumentKind::by_key(app.db(), key).await.unwrap().unwrap();
        assert_eq!(kind.responsible_role.as_deref(), Some(role), "{key}");
    }

    let entity = LegalEntity::default_entity(app.db()).await.unwrap();
    let permit = DocumentKind::by_key(app.db(), CENTRE_PERMIT_KIND)
        .await
        .unwrap()
        .unwrap();
    Document::create(
        app.db(),
        Document {
            subject_type: "kernel.legal_entity".into(),
            subject_id: entity.id,
            kind_id: permit.id,
            title: "KHDA centre permit".into(),
            expiry_date: Some(day(30)),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    expiry::run(app.state(), today()).await.unwrap();

    assert_eq!(
        messages_to(&app, "documents.expiring", &manager)
            .await
            .len(),
        1
    );
    assert!(
        messages_to(&app, "documents.expiring", &coordinator)
            .await
            .is_empty()
    );
}

// ---- Scheduling and parties ----------------------------------------------------

/// Scheduling: eligibility looks 30 days ahead (`SCH.RUL.05`), an open conflict
/// goes to the centre manager 2 hours before the start (`SCH.RUL.06`), the
/// reminders are 24 and 2 hours before; the one-to-one service type asks the
/// police clearance, takes no deposit and no check-in, and the other service
/// types ask nothing.
#[renox::test]
async fn edu_17_the_scheduling_numbers_and_the_one_to_one_service_type() {
    let app = edtech_booted().await;
    let config = wl_scheduling::config_of(app.state());
    assert_eq!(config.look_ahead_days, 30);
    assert_eq!(config.escalation_hours, 2);
    assert_eq!(config.reminder_hours, [24, 2]);
    assert_eq!(config.manager_role.as_deref(), Some(CENTRE_MANAGER));

    let types = ServiceType::query().get(app.db()).await.unwrap();
    assert_eq!(types.len(), 3);
    for service in &types {
        assert_eq!(service.deposit_amount, 0, "{}: no deposit", service.code);
        assert!(!service.check_in_required, "{}: no check-in", service.code);
        if service.code == ONE_TO_ONE_SERVICE {
            assert_eq!(service.required_qualifications, POLICE_CLEARANCE_KIND);
        } else {
            assert_eq!(service.required_qualifications, "", "{}", service.code);
        }
    }
}

/// Parties: a blocked payer's enrolment shows the effect *pause*, decided by
/// the centre manager; the membership pauses too.
#[renox::test]
async fn edu_17_a_blocked_payers_enrolment_pauses_and_the_centre_manager_decides() {
    let _app = edtech_booted().await;
    let enrolment = effect_for("enrolment", &Scope::global());
    assert_eq!(enrolment.effect, BlockEffect::Pause);
    assert_eq!(enrolment.decider_role.as_deref(), Some(CENTRE_MANAGER));
    let membership = effect_for("subscription", &Scope::global());
    assert_eq!(membership.effect, BlockEffect::Pause);
}

// ---- Notifications ----------------------------------------------------------------

/// The stored row of the type `key`.
async fn row(app: &renox::testing::TestApp, key: &str) -> NotificationType {
    NotificationType::by_key(app.db(), key)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the type {key}"))
}

/// Notification types: the messages to students and guardians are *Guardian
/// messages*; the invoice, membership charge and reminder types are *Invoices,
/// statements, payment reminders* with a task to the accountant; terms, refund
/// policy and consent requests are *Portal invitations, document requests,
/// signature requests* with a task to the counsellor; the booking messages
/// keep the wl-scheduling group; staff-only types name no group and take
/// `none`; none is statutory.
#[renox::test]
async fn edu_17_the_registered_types_carry_their_groups_and_reactions() {
    let app = edtech_booted().await;

    let family = row(&app, FAMILY_NOTICE).await;
    assert_eq!(family.group_key, GROUP_GUARDIAN);
    assert_eq!(
        family.give_up.0,
        [
            GiveUpReaction::AlternativeRecipient,
            GiveUpReaction::StaffTask
        ]
    );
    assert_eq!(family.staff_role.as_deref(), Some(COUNSELLOR));

    let request = row(&app, TERMS_REQUEST).await;
    assert_eq!(request.group_key, GROUP_SIGNATURES);
    assert_eq!(
        request.give_up.0,
        [
            GiveUpReaction::MarkContactInvalid,
            GiveUpReaction::StaffTask
        ]
    );
    assert_eq!(request.staff_role.as_deref(), Some(COUNSELLOR));

    for key in [
        "invoice.reminder_before_due",
        "invoice.reminder_after_due",
        "invoicing.subscription_past_due",
        "invoicing.subscription_retry",
    ] {
        let t = row(&app, key).await;
        assert_eq!(t.group_key, GROUP_INVOICES, "{key}");
        assert_eq!(
            t.give_up.0,
            [
                GiveUpReaction::MarkContactInvalid,
                GiveUpReaction::AlternativeRecipient,
                GiveUpReaction::StaffTask
            ],
            "{key}"
        );
        assert_eq!(t.staff_role.as_deref(), Some(ACCOUNTANT), "{key}");
    }

    let staff = row(&app, NOTICE).await;
    assert_eq!(staff.group_key, "");
    assert_eq!(staff.give_up.0, [GiveUpReaction::None]);

    // The booking messages keep the group wl-scheduling registered them under
    // (10 §7.2: "keep the wl-scheduling group"); the vertical does not move them.
    let booking = row(&app, wl_scheduling::notices::REMINDER).await;
    let registered = notification_type(wl_scheduling::notices::REMINDER).expect("registered");
    assert_eq!(booking.group_key, registered.group);
    assert_eq!(booking.group_key, "scheduling");

    for def in notify::notification_types() {
        if def.key.starts_with("edtech.") || def.key.starts_with("invoice.") {
            assert!(!def.statutory, "{}: none is statutory", def.key);
        }
    }
    assert_eq!(
        notification_type(FAMILY_NOTICE)
            .expect("registered")
            .channels,
        [
            notify::NotifyChannel::Whatsapp,
            notify::NotifyChannel::Email
        ]
    );
}

/// A message of `type_key` to `party` about `subject`, sent and run through the
/// jobs; the newest message of the type.
async fn bounce(
    app: &renox::testing::TestApp,
    type_key: &str,
    subject: Subject,
    party: wl_kernel::PartyId,
) -> Message {
    let state = app.state();
    let request = NotifyRequest::new(type_key, subject).to_party(party);
    notify::notifier(state)
        .notify(state, request)
        .await
        .expect("the message is queued");
    app.run_all_jobs().await;
    Message::query()
        .where_(Message::TYPE_KEY.eq(type_key))
        .order_by_desc(Message::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("a message")
}

/// The tasks opened for messages about `subject_type` that gave up.
async fn give_up_tasks(app: &renox::testing::TestApp, subject_type: &str) -> Vec<Ticket> {
    let prefix = format!("notify.give_up:{subject_type}:");
    Ticket::query()
        .get(app.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|t| {
            t.source_key
                .as_deref()
                .is_some_and(|key| key.starts_with(&prefix))
        })
        .collect()
}

/// Notifications: a message to an adult student without a guardian that cannot
/// be delivered has no other contact, so it goes straight to a task for the
/// counsellor; a failed invoice e-mail ends in a task for the accountant.
#[renox::test]
async fn edu_17_a_message_that_gives_up_ends_in_a_task_for_the_counsellor_or_the_accountant() {
    let mail = BouncingMail::default();
    let app = edtech_booted_with_mail(mail.clone()).await;
    let counsellor = person(&app, "Cora", COUNSELLOR).await;
    let accountant = person(&app, "Anwar", ACCOUNTANT).await;
    let student = tenant(&app, "Sam Student").await;
    let payer = tenant(&app, "Pat Payer").await;

    let message = bounce(
        &app,
        FAMILY_NOTICE,
        Subject::new("edtech.enrolment", 1),
        student,
    )
    .await;
    assert!(!mail.attempts().is_empty(), "the notice was e-mailed");
    assert_eq!(message.status, MessageStatus::Abandoned);
    let tasks = give_up_tasks(&app, "edtech.enrolment").await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee_user_id, Some(counsellor.id));

    let invoice = bounce(
        &app,
        wl_invoicing::reminders::AFTER_DUE,
        Subject::new("invoicing.invoice", 7),
        payer,
    )
    .await;
    assert_eq!(invoice.status, MessageStatus::Abandoned);
    let tasks = give_up_tasks(&app, "invoicing.invoice").await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee_user_id, Some(accountant.id));
}

// ---- Alerts and integrations ---------------------------------------------------------

/// Alerts: the three rules are compliance-critical; a failed `EDU.RUL.12` run
/// alerts the company owner and the company admin once each, and nobody else.
#[renox::test]
async fn edu_17_a_failed_tutor_alert_run_alerts_the_owner_and_the_admin() {
    let app = edtech_booted().await;
    for rule in edtech::defaults::CRITICAL_RULES {
        assert!(is_critical(rule), "{rule}");
    }
    let owner = person(&app, "Olga", OWNER).await;
    let admin = person(&app, "Ada", ADMIN).await;
    let manager = person(&app, "Mona", CENTRE_MANAGER).await;

    let spec =
        RuleRunSpec::new("EDU.RUL.12", Subject::new("hr.employee", 99), "stopped").chain_depth(4);
    let outcome = run_rule(app.state(), spec, |_state| async { Ok(()) })
        .await
        .unwrap();
    assert_eq!(outcome, wl_kernel::automation::RunOutcome::Stopped);
    assert_eq!(messages_to(&app, OWNER_ALERT, &owner).await.len(), 1);
    assert_eq!(messages_to(&app, OWNER_ALERT, &admin).await.len(), 1);
    assert!(messages_to(&app, OWNER_ALERT, &manager).await.is_empty());
}

/// Integrations: the gateway, WhatsApp, e-mail and attendance connections exist
/// with the manual fallback of §8 and the role responsible; with the gateway
/// `down` the desk shows the card-terminal fallback.
#[renox::test]
async fn edu_17_the_connections_have_their_fallbacks_and_the_desk_shows_the_terminal() {
    let app = edtech_booted().await;
    let expected = [
        (integrations::PAYMENT_GATEWAY, ACCOUNTANT, "card terminal"),
        (
            integrations::WHATSAPP,
            CENTRE_MANAGER,
            "e-mail instead of WhatsApp",
        ),
        (integrations::EMAIL, ADMIN, "printed copy"),
        (
            edtech::defaults::ATTENDANCE_CONNECTION,
            ACADEMIC_COORDINATOR,
            "on paper",
        ),
    ];
    for (key, role, words) in expected {
        let connection = IntegrationConnection::by_key(app.db(), key)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("no connection {key}"));
        assert_eq!(connection.owner_role, role, "{key}");
        assert!(connection.fallback_en.contains(words), "{key}");
        assert!(
            connection
                .fallback_ar
                .as_deref()
                .is_some_and(|ar| !ar.trim().is_empty()),
            "{key} has an Arabic fallback"
        );
        assert_eq!(connection.status, ConnectionStatus::Up, "{key}");
    }
    let attendance =
        IntegrationConnection::by_key(app.db(), edtech::defaults::ATTENDANCE_CONNECTION)
            .await
            .unwrap()
            .unwrap();
    assert!(attendance.affects(edtech::defaults::SESSION_SCREEN));

    // Up: the desk shows no banner. Down: it shows the terminal and the links.
    let counsellor = person(&app, "Cora", COUNSELLOR).await;
    app.acting_as(&counsellor);
    app.get("/edtech/enrolments")
        .await
        .assert_ok()
        .assert_dont_see("card terminal");
    report_failure(app.state(), integrations::PAYMENT_GATEWAY, "timeout")
        .await
        .unwrap();
    let failing = IntegrationConnection::by_key(app.db(), integrations::PAYMENT_GATEWAY)
        .await
        .unwrap()
        .unwrap();
    app.travel(std::time::Duration::from_secs(16 * 60));
    app.at_travelled_time(go_down(app.state(), &failing))
        .await
        .unwrap();
    app.get("/edtech/enrolments")
        .await
        .assert_ok()
        .assert_see("card terminal or in cash with a receipt")
        .assert_see("payment links are sent again");
}

// ---- Sales and loyalty --------------------------------------------------------------------

/// Lead sources: referral, walk-in, website, social, paid ads, event, partner
/// and other are offered (not the aggregator); none requires a campaign; a
/// cost entry over a campaign's budget waits for a manager.
#[renox::test]
async fn edu_17_the_lead_sources_and_the_campaign_settings() {
    let app = edtech_booted().await;
    let mut active: Vec<String> = wl_sales::LeadSource::ordered(app.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|s| s.active)
        .map(|s| s.code)
        .collect();
    active.sort();
    let mut wanted: Vec<String> = edtech::defaults::LEAD_SOURCES
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    wanted.sort();
    assert_eq!(active, wanted);

    let config = wl_sales::config::config_of(app.state());
    assert!(config.campaign_required_sources.is_empty());
    assert!(config.over_budget_approval);
}

/// Loyalty: fees of AED 1,000 earn 100 points; 100 points are an AED 10
/// discount, at most 10% of the fee; points are valid 12 months, reminded 30
/// days before; an adjustment above 500 points and a redemption without a code
/// are not allowed; the programme is the one the seed made.
#[renox::test]
async fn edu_17_the_loyalty_programme_earns_and_redeems_as_the_spec_says() {
    let app = edtech_booted().await;
    let programme = wl_sales::LoyaltyProgramme::first_active(app.db())
        .await
        .unwrap()
        .expect("the seeded programme");
    assert_eq!(
        programme.points_for(100_000),
        100,
        "AED 1,000 earns 100 points"
    );
    assert_eq!(programme.points_for(99_999), 99);
    assert_eq!(
        programme.min_redeem_points * programme.point_value,
        1_000,
        "AED 10"
    );
    assert_eq!(programme.max_share_bps, 1_000, "at most 10% of the fee");
    assert_eq!(programme.validity_months, 12);

    let config = wl_sales::config::config_of(app.state());
    assert_eq!(config.expiry_reminder_days, 30);
    assert_eq!(config.adjustment_limit, 500);
    assert!(config.confirm_redemption_by_code);
}

// ---- Staff and registrations ----------------------------------------------------------------

/// Onboarding and SOPs: the tutor / trainer, counsellor and coordinator
/// checklists exist with tasks for the centre manager, the academic
/// coordinator and the employee, and completing them blocks nothing; the
/// coordinator owns teaching, assessment and safeguarding, the centre manager
/// the front desk and refunds.
#[renox::test]
async fn edu_17_the_checklists_and_the_sop_library_are_seeded() {
    let app = edtech_booted().await;
    let tutor = wl_hr::OnboardingTemplate::query()
        .where_(wl_hr::OnboardingTemplate::NAME_EN.eq("Tutor and trainer onboarding"))
        .first(app.db())
        .await
        .unwrap()
        .expect("the tutor checklist");
    assert!(!tutor.blocks_activation, "completion blocks nothing");
    let steps = wl_hr::TemplateStep::query()
        .where_(wl_hr::TemplateStep::TEMPLATE_ID.eq(tutor.id))
        .order_by(wl_hr::TemplateStep::ID)
        .get(app.db())
        .await
        .unwrap();
    let owners: Vec<&str> = steps.iter().map(|s| s.owner.as_str()).collect();
    for owner in [
        "role:centre_manager",
        "role:academic_coordinator",
        "employee",
    ] {
        assert!(owners.contains(&owner), "{owner}");
    }
    for name in [
        "Admissions counsellor onboarding",
        "Academic coordinator onboarding",
    ] {
        assert_eq!(
            wl_hr::OnboardingTemplate::query()
                .where_(wl_hr::OnboardingTemplate::NAME_EN.eq(name))
                .count(app.db())
                .await
                .unwrap(),
            1,
            "{name}"
        );
    }
    for (code, owner) in [
        ("SOP-EDU-TEACHING", "academic_coordinator"),
        ("SOP-EDU-ASSESSMENT", "academic_coordinator"),
        ("SOP-EDU-SAFEGUARDING", "academic_coordinator"),
        ("SOP-EDU-FRONT-DESK", "centre_manager"),
        ("SOP-EDU-REFUNDS", "centre_manager"),
    ] {
        let sop = wl_documents::Sop::query()
            .where_(wl_documents::Sop::CODE.eq(code))
            .first(app.db())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("the SOP {code}"));
        assert_eq!(sop.owner_role, owner, "{code}");
        assert_eq!(sop.ack_due_days, 7, "{code}: acknowledged within 7 days");
    }
}

/// Registrations: a fresh deployment lists *Trademark registration* and *Other
/// certification* with the company owner as responsible role and no blocks; a
/// trademark registration expiring in 90 days notifies the owner.
#[renox::test]
async fn edu_17_the_registration_kinds_are_on_and_a_trademark_at_90_days_tells_the_owner() {
    let app = edtech_booted().await;
    let owner = person(&app, "Olga", OWNER).await;
    let manager = person(&app, "Mona", CENTRE_MANAGER).await;
    for key in [TRADEMARK, OTHER] {
        let kind = DocumentKind::by_key(app.db(), key).await.unwrap().unwrap();
        assert!(kind.enabled, "{key} is on");
        assert_eq!(kind.responsible_role.as_deref(), Some(OWNER), "{key}");
        assert!(kind.blocks(app.db()).await.unwrap().is_empty(), "{key}");
    }
    let trademark = DocumentKind::by_key(app.db(), TRADEMARK)
        .await
        .unwrap()
        .unwrap();
    let other = DocumentKind::by_key(app.db(), OTHER)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(trademark.reminder_offsets.0, [180, 90, 30]);
    assert_eq!(other.reminder_offsets.0, [90, 30, 7]);

    let entity = LegalEntity::default_entity(app.db()).await.unwrap();
    Document::create(
        app.db(),
        Document {
            subject_type: "kernel.legal_entity".into(),
            subject_id: entity.id,
            kind_id: trademark.id,
            title: "Gulf Skills trademark".into(),
            expiry_date: Some(day(90)),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    expiry::run(app.state(), today()).await.unwrap();

    assert_eq!(
        messages_to(&app, "documents.expiring", &owner).await.len(),
        1
    );
    assert!(
        messages_to(&app, "documents.expiring", &manager)
            .await
            .is_empty()
    );
}

// ---- The page and the install ----------------------------------------------------------------

/// The settings page shows the 10 values to whoever may see the settings, one
/// row per line of the issue's table, and refuses everyone else.
#[renox::test]
async fn edu_17_the_settings_page_shows_the_10_values() {
    let app = edtech_booted().await;
    let admin = person(&app, "Ada", ADMIN).await;
    app.acting_as(&admin);
    app.get("/settings/compliance")
        .await
        .assert_ok()
        .assert_see("Blocks and overrides")
        .assert_see("Look-ahead windows")
        .assert_see("Compliance-critical rules and owner alerts")
        .assert_see("DOC.RUL.03 (marked)")
        .assert_see("EDU.RUL.12 (marked)")
        .assert_see("Onboarding checklists")
        .assert_see("SOP library");
    let tutor = person(&app, "Tariq", roles::TUTOR).await;
    app.acting_as(&tutor);
    app.get("/settings/compliance").await.assert_forbidden();
}

/// Seeding is idempotent: running the install again changes nothing, and a
/// type the client edited keeps its values.
#[renox::test]
async fn edu_17_seeding_twice_changes_nothing_and_keeps_what_the_client_changed() {
    let app = edtech_booted().await;
    let count = |app: &renox::testing::TestApp| {
        let db = app.db().clone();
        async move {
            (
                wl_hr::OnboardingTemplate::query().count(&db).await.unwrap(),
                wl_documents::Sop::query().count(&db).await.unwrap(),
                wl_sales::LoyaltyProgramme::query()
                    .count(&db)
                    .await
                    .unwrap(),
                count_rows(&db).await,
            )
        }
    };
    let before = count(&app).await;

    let mut edited = row(&app, "invoicing.subscription_retry").await;
    edited.staff_role = Some("counsellor".into());
    edited.save(app.db()).await.unwrap();

    let again = host::edtech_seed::install(app.state()).await.unwrap();
    assert_eq!(again.total(), 0);
    assert_eq!(count(&app).await, before);
    assert_eq!(
        row(&app, "invoicing.subscription_retry")
            .await
            .staff_role
            .as_deref(),
        Some("counsellor"),
        "the client's edit is kept"
    );
}

/// The send windows stored.
async fn count_rows(db: &Db) -> usize {
    wl_notify::SendWindow::query().count(db).await.unwrap() as usize
}
