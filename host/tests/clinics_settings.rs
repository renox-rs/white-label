//! BL-CLN-15 (#363): the 05 values for the shared booking, document, kernel
//! and notification mechanisms, proved through the real host of the demo
//! clinic (`wl:install` and the seeds), not as constants: the practitioner
//! kinds and the override nobody can loosen, the facility licence, the
//! registration kinds, the scheduling numbers, the caps of the messages, the
//! give-up reactions, the connections with their manual fallbacks, the
//! compliance-critical rules and the roles the earlier issues left open
//! (05 §3.3, §4.5, §7.2, §8).

mod support;

use clinics::seed::{EMIRATES_ID_KIND, FACILITY_LICENCE_KIND, LICENCE_KIND, PRIVILEGES_KIND};
use host::roles::{
    self, ADMIN, BILLING_COORDINATOR, CLINIC_MANAGER, OWNER, RECEPTIONIST, STOREKEEPER,
};
use renox::chrono::Days;
use renox::prelude::*;
use support::{
    BouncingMail, clinics_booted, clinics_booted_with_mail, messages_to, person, tenant,
};
use wl_documents::kinds::{self, RefusalReason};
use wl_documents::registrations::{OTHER, TRADEMARK};
use wl_documents::{Document, DocumentKind, DocumentStatus, expiry};
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::compliance::is_critical;
use wl_kernel::compliance::service::NOTIFICATION_TYPE as OWNER_ALERT;
use wl_kernel::documents::{BlockMode, OverrideMode, guard, today};
use wl_kernel::integrations::{
    self, ConnectionStatus, IntegrationConnection, go_down, report_failure,
};
use wl_kernel::notify::{
    self, GROUP_BOOKINGS, GROUP_DOCUMENTS, GiveUpReaction, NotifyRequest, Urgency,
    notification_type,
};
use wl_kernel::{LegalEntity, Subject};
use wl_notify::{Message, MessageStatus, SendWindow};
use wl_scheduling::ServiceType;
use wl_ticketing::Ticket;

/// The grants of `role` in the host.
fn grants(role: &str) -> &'static [&'static str] {
    roles::ROLES
        .iter()
        .find(|(name, _)| *name == role)
        .map(|(_, grants)| *grants)
        .unwrap_or_else(|| panic!("no role {role}"))
}

// ---- Licences and bookings ------------------------------------------------

/// Done when (licences and bookings), 1: the practitioner licence and the
/// branch privileges block `booking.book`, `booking.check_in` and
/// `booking.start`, with an override mode the client cannot loosen, and the
/// page offers no override action.
#[renox::test]
async fn the_practitioner_kinds_block_three_steps_and_their_override_mode_is_fixed() {
    let app = clinics_booted().await;
    let documents = guard(app.state());
    for guarded in ["booking.book", "booking.check_in", "booking.start"] {
        let entries = documents.blocks_for(app.db(), guarded).await.unwrap();
        let mut kinds: Vec<&str> = entries.iter().map(|e| e.kind.as_str()).collect();
        kinds.sort_unstable();
        assert_eq!(kinds, [PRIVILEGES_KIND, LICENCE_KIND], "{guarded}");
        assert!(entries.iter().all(|e| e.mode == BlockMode::Block));
        assert!(
            entries
                .iter()
                .all(|e| e.override_mode == OverrideMode::Never)
        );
    }

    for key in [LICENCE_KIND, PRIVILEGES_KIND] {
        let kind = DocumentKind::by_key(app.db(), key)
            .await
            .unwrap()
            .expect(key);
        assert_eq!(kind.override_mode, OverrideMode::Never);
        assert!(!kind.legal_block, "{key} is not a legal block (L-19)");
        assert_eq!(kind.responsible_role.as_deref(), Some(CLINIC_MANAGER));

        // A client cannot switch the block to `with_approval`.
        let refused = kinds::set_override_mode(app.db(), key, OverrideMode::WithApproval)
            .await
            .unwrap()
            .expect_err("the override mode is fixed");
        assert_eq!(refused.reason, RefusalReason::OverrideFixed);
        let again = DocumentKind::by_key(app.db(), key).await.unwrap().unwrap();
        assert_eq!(again.override_mode, OverrideMode::Never);
    }

    // The kind's page offers no way to change it, and the route refuses.
    let admin = person(&app, "Ada", ADMIN).await;
    app.acting_as(&admin);
    app.get(&format!("/document-kinds/{LICENCE_KIND}"))
        .await
        .assert_ok()
        .assert_dont_see(&format!("/document-kinds/{LICENCE_KIND}/override-mode"));
    app.htmx()
        .post(
            &format!("/document-kinds/{LICENCE_KIND}/override-mode"),
            &[("mode", "with_approval")],
        )
        .await
        .assert_status(422);
}

/// An administrator who removed the three `blocks` entries of a practitioner
/// kind loses nothing for good: the next install puts them back, with the
/// seeded mode, and leaves the others as they are.
#[renox::test]
async fn install_restores_the_blocks_of_a_fixed_kind() {
    use wl_documents::KindBlock;

    let app = clinics_booted().await;
    let kind = DocumentKind::by_key(app.db(), LICENCE_KIND)
        .await
        .unwrap()
        .expect("seeded");
    let guarded = |db| {
        KindBlock::query()
            .where_(KindBlock::KIND_ID.eq(kind.id))
            .count(db)
    };
    assert_eq!(guarded(app.db()).await.unwrap(), 3);
    KindBlock::query()
        .where_(KindBlock::KIND_ID.eq(kind.id))
        .delete(app.db())
        .await
        .unwrap();
    assert_eq!(guarded(app.db()).await.unwrap(), 0);

    kinds::install(app.db()).await.unwrap();
    assert_eq!(guarded(app.db()).await.unwrap(), 3, "put back");
    let entries = guard(app.state())
        .blocks_for(app.db(), "booking.start")
        .await
        .unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e.kind == LICENCE_KIND && e.mode == BlockMode::Block)
    );
}

/// Done when (licences and bookings), 3: the facility licence is the
/// owner's, compliance-critical and blocks nothing; the Emirates ID and the
/// insurance card are the receptionist's and tell the patient as the party.
#[renox::test]
async fn the_responsible_roles_of_the_document_kinds_are_the_05_ones() {
    let app = clinics_booted().await;
    let kind = |key: &'static str| {
        let db = app.db().clone();
        async move {
            DocumentKind::by_key(&db, key)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("no kind {key}"))
        }
    };

    let facility = kind(FACILITY_LICENCE_KIND).await;
    assert_eq!(facility.responsible_role.as_deref(), Some(OWNER));
    assert!(facility.compliance_critical && facility.has_expiry);
    assert_eq!(facility.reminder_offsets.0, [180, 90, 30, 7]);
    assert!(facility.blocks(app.db()).await.unwrap().is_empty());

    for key in [EMIRATES_ID_KIND, "insurance_card"] {
        let kind = kind(key).await;
        assert_eq!(
            kind.responsible_role.as_deref(),
            Some(RECEPTIONIST),
            "{key}"
        );
        assert!(kind.notify_party, "{key} tells the patient as the party");
    }

    // The two registration kinds, with the owner / medical director and no
    // `blocks`, reminded at 180, 90 and 30 days and at 90, 30 and 7.
    let trademark = kind(TRADEMARK).await;
    let other = kind(OTHER).await;
    for kind in [&trademark, &other] {
        assert_eq!(kind.responsible_role.as_deref(), Some(OWNER));
        assert!(kind.blocks(app.db()).await.unwrap().is_empty());
        assert!(kind.enabled);
    }
    assert_eq!(trademark.reminder_offsets.0, [180, 90, 30]);
    assert_eq!(other.reminder_offsets.0, [90, 30, 7]);
    assert_eq!(trademark.name_en, "Trademark registration");
    assert_eq!(other.name_en, "Other certification");
}

/// Done when (sales, finance, staff and registrations), last: a trademark
/// registration expiring in 90 days notifies the owner / medical director.
#[renox::test]
async fn a_trademark_registration_expiring_in_90_days_tells_the_owner() {
    let app = clinics_booted().await;
    let owner = person(&app, "Olga", OWNER).await;
    let receptionist = person(&app, "Rita", RECEPTIONIST).await;
    let entity = LegalEntity::default_entity(app.db()).await.unwrap();
    let kind = DocumentKind::by_key(app.db(), TRADEMARK)
        .await
        .unwrap()
        .expect("seeded");
    Document::create(
        app.db(),
        Document {
            subject_type: "kernel.legal_entity".into(),
            subject_id: entity.id,
            kind_id: kind.id,
            title: "Palm Grove trademark".into(),
            expiry_date: Some(today().checked_add_days(Days::new(90)).unwrap()),
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
        messages_to(&app, "documents.expiring", &receptionist)
            .await
            .is_empty()
    );
}

/// Done when (licences and bookings), 2: `SCH.RUL.05` looks 30 days ahead and
/// `SCH.RUL.06` escalates 2 hours before the start; the reminders, the
/// no-show grace and the manager are the 05 ones. The service types start
/// without a booking deposit, and a walk-out refunds the remainder.
#[renox::test]
async fn the_scheduling_numbers_are_the_05_ones() {
    let app = clinics_booted().await;
    let config = wl_scheduling::config_of(app.state());
    assert_eq!(config.look_ahead_days, 30);
    assert_eq!(config.escalation_hours, 2);
    assert_eq!(config.reminder_hours, [24, 2]);
    assert_eq!(config.no_show_grace_minutes, 15);
    assert_eq!(config.manager_role.as_deref(), Some(CLINIC_MANAGER));
    assert!(
        config
            .extra_reason_codes
            .iter()
            .any(|(code, by)| code == "coverage_not_confirmed"
                && *by == wl_kernel::scheduling::Initiator::Business)
    );

    let types = ServiceType::query().get(app.db()).await.unwrap();
    assert_eq!(types.len(), 3);
    for service in &types {
        assert_eq!(
            service.deposit_amount, 0,
            "{}: no deposit by default",
            service.code
        );
        assert!(
            !service.walk_out_forfeits,
            "{}: the remainder is refunded",
            service.code
        );
        assert_eq!(service.deposit_forfeit_hours, 24);
    }
}

// ---- Deposits and reason codes -------------------------------------------

/// Done when (deposits), 1: a waiver needs `bookings.waive_deposit`, which the
/// clinic manager holds and the receptionist does not.
#[renox::test]
async fn only_the_clinic_manager_waives_a_deposit() {
    let app = clinics_booted().await;
    let waive = wl_scheduling::permissions::WAIVE_DEPOSIT;
    assert!(grants(CLINIC_MANAGER).contains(&waive));
    assert!(!grants(RECEPTIONIST).contains(&waive));

    let manager = person(&app, "Mona", CLINIC_MANAGER).await;
    let receptionist = person(&app, "Rita", RECEPTIONIST).await;
    app.acting_as(&receptionist);
    app.htmx()
        .post(
            "/scheduling/bookings/1/waive-deposit",
            &[("reason", "goodwill")],
        )
        .await
        .assert_forbidden();
    app.acting_as(&manager);
    let answer = app
        .htmx()
        .post(
            "/scheduling/bookings/1/waive-deposit",
            &[("reason", "goodwill")],
        )
        .await;
    assert_ne!(answer.status.as_u16(), 403, "the manager may ask");
}

// ---- Alerts and void ---------------------------------------------------------

/// Done when (alerts), 1: the four rules of §7.2 are compliance-critical, and
/// a stopped run of `CLN.RUL.03` alerts the owner / medical director and the
/// company admin once each; there is no setting that switches it off.
#[renox::test]
async fn the_compliance_critical_rules_alert_the_owner_and_cannot_be_switched_off() {
    let app = clinics_booted().await;
    for rule in clinics::seed::CRITICAL_RULES {
        assert!(is_critical(rule), "{rule}");
    }
    assert!(
        !is_critical("CLN.RUL.05"),
        "a denied claim task is not critical"
    );

    let owner = person(&app, "Olga", OWNER).await;
    let admin = person(&app, "Ada", ADMIN).await;
    let manager = person(&app, "Mona", CLINIC_MANAGER).await;
    let spec =
        RuleRunSpec::new("CLN.RUL.03", Subject::new("clinics.claim", 99), "stopped").chain_depth(4);
    let outcome = run_rule(app.state(), spec, |_state| async { Ok(()) })
        .await
        .unwrap();
    assert_eq!(outcome, wl_kernel::automation::RunOutcome::Stopped);
    assert_eq!(messages_to(&app, OWNER_ALERT, &owner).await.len(), 1);
    assert_eq!(messages_to(&app, OWNER_ALERT, &admin).await.len(), 1);
    assert!(messages_to(&app, OWNER_ALERT, &manager).await.is_empty());
}

/// Done when (alerts), 2: `void` is offered on a draft or ready claim and on
/// no appointment.
#[renox::test]
async fn void_is_offered_on_a_claim_before_it_is_sent_and_not_on_an_appointment() {
    let _app = clinics_booted().await;
    let claim = wl_kernel::voids::get("clinics.claim").expect("a claim is voidable");
    assert_eq!(claim.subject_type(), "clinics.claim");
    let machine = wl_kernel::machines::get("clinics.claim").expect("the claim's machine");
    assert_eq!(
        machine.transition("void").map(|t| t.from.clone()),
        Some(vec!["draft".to_owned(), "ready".to_owned()])
    );
    assert!(wl_kernel::voids::get("scheduling.booking").is_none());
    assert!(wl_kernel::voids::get("clinics.visit").is_none());
}

// ---- Connections and messages ----------------------------------------------

/// Done when (connections), 1 and 2: the SMS, WhatsApp, payment gateway,
/// claims channel and e-mail connections exist with the manual fallback of
/// §8 and the role responsible; with the claims channel down the claim list
/// shows "submit through the payer portal and record the reference".
#[renox::test]
async fn the_connections_have_their_manual_fallback_and_the_claims_screen_shows_it() {
    let app = clinics_booted().await;
    let expected = [
        (integrations::SMS, CLINIC_MANAGER, "Phone the patients"),
        (integrations::WHATSAPP, CLINIC_MANAGER, "Phone the patients"),
        (
            integrations::PAYMENT_GATEWAY,
            roles::ACCOUNTANT,
            "card terminal or in cash",
        ),
        (
            clinics::seed::CLAIMS_CHANNEL,
            BILLING_COORDINATOR,
            "Submit through the payer portal and record the reference",
        ),
        (integrations::EMAIL, ADMIN, "printed copy"),
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

    // The claims channel goes down: the claims list says what to do.
    let coordinator = person(&app, "Bea", BILLING_COORDINATOR).await;
    app.acting_as(&coordinator);
    app.get("/clinics/claims")
        .await
        .assert_ok()
        .assert_dont_see("Submit through the payer portal");
    report_failure(app.state(), clinics::seed::CLAIMS_CHANNEL, "timeout")
        .await
        .unwrap();
    let failing = IntegrationConnection::by_key(app.db(), clinics::seed::CLAIMS_CHANNEL)
        .await
        .unwrap()
        .unwrap();
    app.travel(std::time::Duration::from_secs(16 * 60));
    app.at_travelled_time(go_down(app.state(), &failing))
        .await
        .unwrap();
    app.get("/clinics/claims")
        .await
        .assert_ok()
        .assert_see("Submit through the payer portal and record the reference");
}

/// Done when (connections), 1: with the SMS connection down the front desk
/// sees the call list; the page says what to do.
#[renox::test]
async fn with_sms_down_the_front_desk_sees_the_call_list_page() {
    let app = clinics_booted().await;
    let receptionist = person(&app, "Rita", RECEPTIONIST).await;
    app.acting_as(&receptionist);
    app.get("/clinics/call-list")
        .await
        .assert_ok()
        .assert_see("Nobody to call");
    report_failure(app.state(), integrations::SMS, "gateway timeout")
        .await
        .unwrap();
    let failing = IntegrationConnection::by_key(app.db(), integrations::SMS)
        .await
        .unwrap()
        .unwrap();
    app.travel(std::time::Duration::from_secs(16 * 60));
    app.at_travelled_time(go_down(app.state(), &failing))
        .await
        .unwrap();
    let down = IntegrationConnection::by_key(app.db(), integrations::SMS)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(down.status, ConnectionStatus::Down);
    app.get("/clinics/call-list")
        .await
        .assert_ok()
        .assert_see("Phone the patients on the call list");
    // Someone who is not at the desk is refused.
    let member = person(&app, "Mia", roles::MEMBER).await;
    app.acting_as(&member);
    app.get("/clinics/call-list").await.assert_forbidden();
}

/// Done when (connections), 5: the four 05 notification types are registered
/// with the groups and urgency of §7.2.
#[renox::test]
async fn the_05_notification_types_have_their_groups_urgency_and_give_up() {
    let _app = clinics_booted().await;
    let card = notification_type("clinics.card_upload_request").expect("registered");
    assert_eq!(card.group, GROUP_DOCUMENTS);
    assert_eq!(card.urgency, Urgency::Normal);
    assert_eq!(card.staff_role.as_deref(), Some(RECEPTIONIST));
    assert!(card.give_up.contains(&GiveUpReaction::StaffTask));

    for key in ["clinics.recall_invite", "clinics.prior_auth_denied"] {
        let def = notification_type(key).unwrap_or_else(|| panic!("{key} is registered"));
        assert_eq!(def.group, GROUP_BOOKINGS, "{key}");
        assert_eq!(def.urgency, Urgency::Normal, "{key}");
        assert!(
            !def.give_up.contains(&GiveUpReaction::StaffTask),
            "{key}: no task"
        );
    }

    let recall = notification_type("clinics.product_recall_notice").expect("registered");
    assert_eq!(recall.urgency, Urgency::Urgent);
    assert!(recall.give_up.contains(&GiveUpReaction::StaffTask));
    assert_eq!(recall.staff_role.as_deref(), Some(RECEPTIONIST));

    // The appointment reminder gives up with no reaction at all.
    let reminder = notification_type(wl_scheduling::notices::REMINDER).expect("registered");
    assert_eq!(reminder.give_up, [GiveUpReaction::None]);
}

/// Done when (connections), 4: the reassignment, change, reschedule-offer,
/// recall and the other patient templates carry no reason, in English or in
/// Arabic.
#[renox::test]
async fn the_patient_change_messages_state_no_reason() {
    use wl_scheduling::notices::{CHANGED, REASSIGNED, RESCHEDULE_OFFER};

    let _app = clinics_booted().await;
    for key in [
        REASSIGNED,
        CHANGED,
        RESCHEDULE_OFFER,
        "clinics.product_recall_notice",
        "clinics.prior_auth_denied",
        "clinics.recall_invite",
    ] {
        let def = notification_type(key).unwrap_or_else(|| panic!("{key} is registered"));
        assert!(!def.templates.is_empty(), "{key} has a template");
        for template in &def.templates {
            let text = format!(
                "{} {}",
                template.subject.clone().unwrap_or_default(),
                template.body
            )
            .to_lowercase();
            for banned in ["reason", "سبب", "{{ reason", "{{ cancel_reason"] {
                assert!(
                    !text.contains(banned),
                    "{key} ({}) states a reason: {text}",
                    template.language
                );
            }
        }
    }
    let recall = notification_type("clinics.product_recall_notice").unwrap();
    let languages: Vec<&str> = recall
        .templates
        .iter()
        .map(|t| t.language.as_str())
        .collect();
    assert_eq!(languages, ["en", "ar"]);
    let invite = notification_type("clinics.recall_invite").unwrap();
    assert_eq!(invite.templates.len(), 2);
}

/// A message of `type_key` to `party` about `subject`, sent and run through
/// the jobs; the message.
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
        .order_by(Message::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("a message")
}

/// The tasks opened for messages about `subject_type` that gave up (the
/// notice of a new task that bounces opens a task of its own, about the
/// ticket).
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

/// Done when (connections), 3 and 5: a reminder that gives up creates no
/// task; a card upload request that gives up creates a task for the
/// receptionist.
#[renox::test]
async fn a_reminder_that_gives_up_opens_no_task_and_a_card_request_opens_one_for_the_receptionist()
{
    let mail = BouncingMail::default();
    let app = clinics_booted_with_mail(mail.clone()).await;
    let receptionist = person(&app, "Rita", RECEPTIONIST).await;
    let patient = tenant(&app, "Pat Lane").await;

    let reminder = bounce(
        &app,
        wl_scheduling::notices::REMINDER,
        Subject::new("scheduling.booking", 1),
        patient,
    )
    .await;
    assert!(!mail.attempts().is_empty(), "the reminder was e-mailed");
    assert_eq!(reminder.status, MessageStatus::Abandoned);
    assert!(
        give_up_tasks(&app, "scheduling.booking").await.is_empty(),
        "a reminder needs no action"
    );

    // The bounced address is on the suppression list now: another patient.
    let other = tenant(&app, "Dan Lane").await;
    let request = bounce(
        &app,
        "clinics.card_upload_request",
        Subject::new("clinics.insurance_policy", 1),
        other,
    )
    .await;
    assert_eq!(request.status, MessageStatus::Abandoned);
    let tasks = give_up_tasks(&app, "clinics.insurance_policy").await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee_user_id, Some(receptionist.id));
}

/// Done when (connections), 3: a recall notice that gives up goes back to
/// *gave up* and opens a call task for the front desk (the receptionist),
/// through the real notifier with an e-mail that bounces.
#[renox::test]
async fn a_recall_notice_that_gives_up_opens_a_call_task_for_the_front_desk() {
    use clinics::recall_notice::{
        NOTICE_TYPE, RECALL_NOTICE_SUBJECT_TYPE, RecallNotice, RecallNoticeStatus,
    };
    use renox::state_machine::StateMachine;

    let mail = BouncingMail::default();
    let app = clinics_booted_with_mail(mail.clone()).await;
    let receptionist = person(&app, "Rita", RECEPTIONIST).await;
    let patient = tenant(&app, "Pat Lane").await;
    let mut notice = RecallNotice {
        recall_id: 1,
        party_id: patient.0,
        visit_dates: "2026-10-01".to_owned(),
        status: RecallNoticeStatus::Sent,
        ..RecallNotice::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    notice
        .insert_in_state(&mut tx, RecallNoticeStatus::Sent, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let message = bounce(
        &app,
        NOTICE_TYPE,
        Subject::new(RECALL_NOTICE_SUBJECT_TYPE, notice.id),
        patient,
    )
    .await;
    assert_eq!(message.status, MessageStatus::Abandoned);
    let now = RecallNotice::find_or_404(app.db(), notice.id).await.unwrap();
    assert_eq!(now.status, RecallNoticeStatus::GaveUp);
    let tasks = give_up_tasks(&app, RECALL_NOTICE_SUBJECT_TYPE).await;
    assert_eq!(tasks.len(), 1, "one call task");
    assert_eq!(tasks[0].assignee_user_id, Some(receptionist.id));
}

// ---- Notifications ---------------------------------------------------------

/// The marketing caps and windows of §7.2 and `NTF.RUL.02`, `NTF.ACL.03`: at
/// most 2 marketing messages per patient in 7 days, a broadcast to more than
/// 200 patients needs a second approver, and marketing goes out 07:00 to
/// 21:00.
#[renox::test]
async fn the_marketing_caps_and_windows_are_the_05_ones() {
    use wl_kernel::notify::NotifyChannel;

    let app = clinics_booted().await;
    let cap = app
        .state()
        .provided::<wl_notify::cap::CapSettings>()
        .expect("wl-notify provides its cap");
    assert_eq!(cap.max, 2);
    let broadcast = app
        .state()
        .provided::<wl_notify::BroadcastSettings>()
        .expect("wl-notify provides its broadcast settings");
    assert_eq!(broadcast.approval_above, 200);

    for channel in [
        NotifyChannel::Sms,
        NotifyChannel::Whatsapp,
        NotifyChannel::Email,
    ] {
        let window = SendWindow::for_channel(app.db(), channel).await.unwrap();
        assert_eq!((window.start, window.end), (7 * 60, 21 * 60), "{channel}");
        let stored = SendWindow::query()
            .where_(SendWindow::CHANNEL.eq(channel))
            .exists(app.db())
            .await
            .unwrap();
        assert!(
            stored,
            "{channel} has its window stored for the client to edit"
        );
    }
}

// ---- Roles left open -------------------------------------------------------

/// The roles the earlier issues left open (DEFERRED-b5): the trace, notify
/// and call permissions of a recall, the permission to open a recall and the
/// ones to place and decide holds.
#[renox::test]
async fn the_recall_permissions_are_granted_to_the_right_05_roles() {
    use clinics::permissions::{RECALLS_CALL, RECALLS_NOTIFY, RECALLS_TRACE};
    use wl_inventory::permissions::{LOTS_RELEASE, RECALLS_MANAGE, STOCK_HOLD};

    // The owner / medical director owns the recall: opens it, traces it and
    // decides the lines of a hold.
    for permission in [RECALLS_MANAGE, RECALLS_TRACE, LOTS_RELEASE, STOCK_HOLD] {
        assert!(grants(OWNER).contains(&permission), "owner: {permission}");
    }
    // The clinic manager sends the notices, traces and calls, and is the
    // second holder of `lots.release` (`STK.ACL.05`).
    for permission in [
        RECALLS_TRACE,
        RECALLS_NOTIFY,
        RECALLS_CALL,
        LOTS_RELEASE,
        STOCK_HOLD,
    ] {
        assert!(
            grants(CLINIC_MANAGER).contains(&permission),
            "manager: {permission}"
        );
    }
    assert!(!grants(CLINIC_MANAGER).contains(&RECALLS_MANAGE));
    // The front desk only records the calls; the storekeeper holds and
    // releases stock.
    assert!(grants(RECEPTIONIST).contains(&RECALLS_CALL));
    assert!(!grants(RECEPTIONIST).contains(&RECALLS_TRACE));
    assert!(!grants(RECEPTIONIST).contains(&RECALLS_NOTIFY));
    assert!(grants(STOREKEEPER).contains(&LOTS_RELEASE));

    // The company admin has them all.
    for permission in [RECALLS_MANAGE, RECALLS_TRACE, RECALLS_NOTIFY, RECALLS_CALL] {
        assert!(grants(ADMIN).contains(&permission), "admin: {permission}");
    }

    // And the owner reaches the recall page where the manager's branch does.
    let app = clinics_booted().await;
    let owner = person(&app, "Olga", OWNER).await;
    app.acting_as(&owner);
    assert_ne!(
        app.get("/clinics/recalls/1").await.status.as_u16(),
        403,
        "the owner may trace a recall"
    );
}
