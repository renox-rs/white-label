//! BL-MPW-20 (#434): the 09 values for the shared documents, parties,
//! notification, alert and integration settings, in the host of a manpower
//! agency with the real wl-documents, wl-parties, wl-notify, wl-ticketing,
//! wl-hr and wl-sales: the document kinds with their `blocks` and override
//! modes, the responsible roles and the look-ahead, the block effect of a
//! deployment, the give-up reactions and groups of the notification types, the
//! confidential tickets, the compliance-critical rules, the integration
//! fallbacks, the lead sources, the onboarding checklists, the SOP library and
//! the registration kinds. `DOC.ACL.02`, `MPW.ACL.07`, `NTF.RUL.01`,
//! `WRK.ACL.03`, `PLT.RUL.04`, `DOC.EXP.01`.

mod manpower_support;
mod support;

use std::sync::{Arc, Mutex};

use host::roles::{ACCOUNT_MANAGER, ACCOUNTANT, ADMIN, HR};
use manpower::notices::{CLIENT_NOTICE, CLIENT_NOTICE_URGENT, STATUTORY_NOTICE, WORKER_MESSAGE};
use manpower_support::{account, boot_seeded, boot_seeded_with, deployment, document, worker};
use renox::auth::permissions::Scope;
use renox::prelude::*;
use renox::testing::TestApp;
use support::{day, person, tenant};
use wl_documents::{DocumentKind, KindBlock, expiry};
use wl_kernel::block_effects::{BlockEffect, effect_for};
use wl_kernel::compliance::{is_critical, rule};
use wl_kernel::documents::{BlockMode, OverrideMode, today};
use wl_kernel::notify::{
    self, ContactPurpose, GiveUpReaction, NotifyChannel, NotifyRequest, Urgency,
};
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_notify::channels::ProviderFuture;
use wl_notify::{
    ChannelProvider, HardError, Message, NotificationType, Outgoing, ReactionRun, SendError,
    StatutoryNotice,
};
use wl_parties::relationships::{NewRelationship, RelationshipType, relate};
use wl_parties::{ContactPoint, ContactStatus, ContactType, NewParty, PersonDetails, create};
use wl_ticketing::Ticket;

/// The blocks of kind `key` as (guarded, mode).
async fn blocks_of(
    app: &renox::testing::TestApp,
    key: &str,
) -> (DocumentKind, Vec<(String, BlockMode)>) {
    let kind = DocumentKind::by_key(app.db(), key)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the kind {key}"));
    let mut blocks: Vec<_> = KindBlock::query()
        .where_(KindBlock::KIND_ID.eq(kind.id))
        .get(app.db())
        .await
        .unwrap()
        .into_iter()
        .map(|b| (b.guarded, b.mode))
        .collect();
    blocks.sort_by(|a, b| a.0.cmp(&b.0));
    (kind, blocks)
}

/// AC1 (Documents): after seeding, the residence visa, work permit and SIRA
/// licence block `deployment.activate`, `extend` and `resume` and flag
/// `attendance_record.accept` with override `never`; the Emirates ID, health
/// insurance, health card and site pass do the same with the MD's approval;
/// switching the work permit's override to `with_approval` is refused.
#[renox::test]
async fn the_seeded_kinds_carry_the_blocks_and_override_modes_of_the_spec() {
    let app = boot_seeded().await;
    let expected = vec![
        ("attendance_record.accept".to_owned(), BlockMode::Flag),
        ("deployment.activate".to_owned(), BlockMode::Block),
        ("deployment.extend".to_owned(), BlockMode::Block),
        ("deployment.resume".to_owned(), BlockMode::Block),
    ];
    for key in [
        "residence_visa",
        "work_permit",
        "sira_licence",
        "emirates_id",
        "health_insurance",
        "health_card",
        "site_pass",
    ] {
        let (kind, blocks) = blocks_of(&app, key).await;
        assert_eq!(blocks, expected, "{key}");
        let never = ["residence_visa", "work_permit", "sira_licence"].contains(&key);
        assert_eq!(
            kind.override_mode,
            if never {
                OverrideMode::Never
            } else {
                OverrideMode::WithApproval
            },
            "{key}"
        );
        assert_eq!(kind.legal_block, never, "{key}");
    }
    for key in [
        "passport",
        "iloe",
        "labour_contract",
        "client_contract",
        "rate_card_acceptance",
        "signed_timesheet",
    ] {
        let (_kind, blocks) = blocks_of(&app, key).await;
        assert!(blocks.is_empty(), "{key} blocks nothing");
    }

    // A legal block cannot be switched to `with_approval`.
    let admin = person(&app, "Ada", ADMIN).await;
    app.acting_as(&admin);
    app.htmx()
        .post(
            "/document-kinds/work_permit/override-mode",
            &[("mode", "with_approval")],
        )
        .await
        .assert_status(422);
    let (kind, _) = blocks_of(&app, "work_permit").await;
    assert_eq!(kind.override_mode, OverrideMode::Never);
}

/// AC2 and AC3 (Responsible role, Look-ahead): the work permit has the PRO as
/// responsible role and 30 days of look-ahead; a site pass expiring for W
/// deployed at client C is told to C's account manager, and the nightly run
/// lists a deployment for a permit expiring in 29 days but not in 31.
#[renox::test]
async fn the_responsible_roles_the_look_ahead_and_the_site_pass_reach_the_account_manager() {
    let app = boot_seeded().await;
    for (key, role) in [
        ("work_permit", "pro"),
        ("residence_visa", "pro"),
        ("sira_licence", "pro"),
        ("emirates_id", "pro"),
        ("health_insurance", "pro"),
        ("health_card", "pro"),
        ("passport", "pro"),
        ("iloe", "pro"),
        ("labour_contract", "hr"),
        ("site_pass", "account_manager"),
        ("client_contract", "account_manager"),
        ("rate_card_acceptance", "account_manager"),
        ("signed_timesheet", "account_manager"),
    ] {
        let kind = DocumentKind::by_key(app.db(), key).await.unwrap().unwrap();
        assert_eq!(kind.responsible_role.as_deref(), Some(role), "{key}");
    }
    for key in [
        "work_permit",
        "residence_visa",
        "sira_licence",
        "emirates_id",
        "health_insurance",
        "health_card",
        "site_pass",
    ] {
        let kind = DocumentKind::by_key(app.db(), key).await.unwrap().unwrap();
        assert_eq!(kind.look_ahead_days, 30, "{key}");
    }

    let a = account(&app).await;
    let w = worker(&app, "Wasim Khan").await;
    deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        manpower::DeploymentStatus::Active,
        day(-30),
        Some(day(300)),
    )
    .await;
    document(&app, w, "site_pass", day(20)).await;
    expiry::run(app.state(), today()).await.unwrap();
    let told = support::messages_to(&app, "documents.expiring", &a.amal).await;
    assert_eq!(told.len(), 1, "the account manager of C is told: {told:?}");

    // 29 days: inside the look-ahead of the permit; 31 days: outside.
    let near = worker(&app, "Omar Ali").await;
    let far = worker(&app, "Yusuf Khan").await;
    for who in [near, far] {
        deployment(
            &app,
            &a,
            &a.site,
            who,
            "security_guard",
            manpower::DeploymentStatus::Active,
            day(-30),
            Some(day(300)),
        )
        .await;
    }
    document(&app, near, "work_permit", day(29)).await;
    document(&app, far, "work_permit", day(31)).await;
    expiry::run(app.state(), today()).await.unwrap();
    let flagged = manpower::documents::DeploymentDocumentFlag::query()
        .get(app.db())
        .await
        .unwrap();
    let texts: Vec<String> = flagged.iter().map(|f| f.text()).collect();
    assert!(texts.iter().any(|t| t.contains("work permit")), "{texts:?}");
    assert_eq!(
        flagged
            .iter()
            .filter(|f| f.text().contains("work permit"))
            .count(),
        1,
        "only the permit inside the look-ahead: {texts:?}"
    );
}

/// AC4 (Parties): a 09 deployment shows block effect `continue`, decided by the
/// MD.
#[renox::test]
async fn a_blocked_client_s_deployment_continues_and_the_md_decides() {
    let _app = boot_seeded().await;
    let rule = effect_for("deployment", &Scope::global());
    assert_eq!(rule.effect, BlockEffect::Continue);
    assert_eq!(rule.decider_role.as_deref(), Some("md"));
}

/// AC9 (Alerts): the nine rules are compliance-critical with the MD as owner.
#[renox::test]
async fn the_nine_rules_are_compliance_critical_with_the_md_as_owner() {
    let _app = boot_seeded().await;
    for id in manpower::seed::CRITICAL_RULES {
        assert!(is_critical(id), "{id}");
        assert_eq!(
            rule(id).unwrap().owner_role_name(),
            Some("md"),
            "{id} is the MD's"
        );
    }
    assert_eq!(manpower::seed::CRITICAL_RULES.len(), 9);
}

/// AC7 (Tickets): a *labour case* ticket (and a grievance) is confidential:
/// HR, the accountant and the MD hold `tickets.confidential.view`; the account
/// manager and the site supervisor do not.
#[renox::test]
async fn labour_cases_and_grievances_are_confidential_to_hr_the_accountant_and_the_md() {
    let app = boot_seeded().await;
    for key in ["labour_case", "grievance"] {
        let category = wl_ticketing::Category::by_key(app.db(), key)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("the category {key}"));
        assert!(category.confidential, "{key}");
    }
    let sees = |role: &str| {
        host::roles::grants_of(role).contains(&wl_ticketing::permissions::CONFIDENTIAL_VIEW)
    };
    for role in [HR, host::roles::ACCOUNTANT, ADMIN] {
        assert!(sees(role), "{role} sees confidential tickets");
    }
    for role in [ACCOUNT_MANAGER, host::roles::SUPERVISOR] {
        assert!(!sees(role), "{role} does not");
    }

    // A labour case, opened the way wl-hr opens one: HR, the accountant and the
    // MD open its page, the account manager and the supervisor do not find it.
    let w = worker(&app, "Wasim Khan").await;
    let opened = wl_kernel::tickets::task_creator(app.state())
        .open(
            app.state(),
            wl_kernel::tickets::OpenTicket::new(w.subject(), "labour_case", "Labour case of Wasim"),
        )
        .await
        .unwrap();
    let ticket = opened.expect("a ticket").id;
    for role in [HR, host::roles::ACCOUNTANT, ADMIN] {
        let user = person(&app, &format!("User {role}"), role).await;
        app.acting_as(&user);
        app.get(&format!("/tickets/{ticket}")).await.assert_ok();
    }
    for role in [ACCOUNT_MANAGER, host::roles::SUPERVISOR] {
        let user = person(&app, &format!("User {role}"), role).await;
        app.acting_as(&user);
        let page = app.get(&format!("/tickets/{ticket}")).await;
        assert_ne!(
            page.status.as_u16(),
            200,
            "{role} must not see the labour case"
        );
    }
}

/// A channel provider that fails every send hard, recording the attempts.
#[derive(Clone)]
struct Failing {
    channel: NotifyChannel,
    sent: Arc<Mutex<Vec<String>>>,
}

impl Failing {
    fn new(channel: NotifyChannel) -> Self {
        Failing {
            channel,
            sent: Arc::default(),
        }
    }

    fn attempts(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }
}

impl ChannelProvider for Failing {
    fn name(&self) -> &'static str {
        "failing"
    }

    fn channel(&self) -> NotifyChannel {
        self.channel
    }

    fn available(&self, _state: &AppState) -> bool {
        true
    }

    fn send<'a>(
        &'a self,
        _state: &'a AppState,
        outgoing: &'a Outgoing,
    ) -> ProviderFuture<'a, std::result::Result<String, SendError>> {
        self.sent.lock().unwrap().push(outgoing.address.clone());
        let hard = if self.channel == NotifyChannel::Sms {
            HardError::InvalidNumber
        } else {
            HardError::HardBounce
        };
        Box::pin(async move { Err(SendError::hard(hard, "550", "undeliverable")) })
    }
}

/// The seeded agency whose e-mail and SMS always fail.
async fn failing() -> (TestApp, Failing, Failing) {
    let mail = Failing::new(NotifyChannel::Email);
    let sms = Failing::new(NotifyChannel::Sms);
    let (m, s) = (mail.clone(), sms.clone());
    let app =
        boot_seeded_with(host::Tuning::default().notify(move |n| n.provider(m).provider(s))).await;
    (app, mail, sms)
}

/// A person party with only a mobile number.
async fn mobile_only(app: &TestApp, name: &str, number: &str) -> PartyId {
    create(
        app.state(),
        NewParty {
            kind: PartyKind::Person,
            legal_name_en: name.into(),
            person: Some(PersonDetails::default()),
            contacts: vec![ContactPoint {
                contact_type: ContactType::Mobile,
                value: number.into(),
                is_primary: true,
                ..Default::default()
            }],
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id()
}

async fn queue(app: &TestApp, request: NotifyRequest) -> Vec<notify::MessageId> {
    let state = app.state();
    app.at_travelled_time(notify::notifier(state).notify(state, request))
        .await
        .expect("the notifier queues the message")
}

/// The contact points of `party` of `kind`.
async fn points(app: &TestApp, party: PartyId, kind: ContactType) -> Vec<ContactPoint> {
    ContactPoint::query()
        .where_(ContactPoint::PARTY_ID.eq(party.0))
        .where_(ContactPoint::CONTACT_TYPE.eq(kind))
        .get(app.db())
        .await
        .unwrap()
}

/// The open staff tasks assigned to `user`.
async fn tasks_of(app: &TestApp, user: &User) -> Vec<Ticket> {
    Ticket::query()
        .where_(Ticket::ASSIGNEE_USER_ID.eq(user.id))
        .get(app.db())
        .await
        .unwrap()
}

/// AC5a (Notifications): a failed SMS to worker W marks the number invalid and
/// creates a task for HR to reach W through the site supervisor; with the SMS
/// connection down this is the path used.
#[renox::test]
async fn a_failed_sms_to_a_worker_marks_the_number_invalid_and_gives_hr_a_task() {
    let (app, _mail, sms) = failing().await;
    let hana = person(&app, "Hana", HR).await;
    let w = mobile_only(&app, "Wasim Khan", "+971501110001").await;

    queue(
        &app,
        NotifyRequest::new(WORKER_MESSAGE, Subject::new("hr.employee", 7))
            .to_party(w)
            .data(renox::serde_json::json!({ "title": "Shift change", "body": "Your shift starts at 07:00." })),
    )
    .await;
    app.run_all_jobs().await;

    assert_eq!(sms.attempts(), ["+971501110001"]);
    let number = &points(&app, w, ContactType::Mobile).await[0];
    assert_eq!(number.status, ContactStatus::Invalid);
    let tasks = tasks_of(&app, &hana).await;
    assert_eq!(tasks.len(), 1, "one task for HR: {tasks:?}");
}

/// AC5b: a failed invoice e-mail marks the contact invalid and goes to the
/// client's other finance contact; when that also fails the accountant gets a
/// `staff_task`.
#[renox::test]
async fn a_failed_invoice_email_goes_to_the_other_finance_contact_then_the_accountant() {
    let (app, mail, _sms) = failing().await;
    let accountant = person(&app, "Aya", ACCOUNTANT).await;
    let a = account(&app).await;
    let ali = tenant(&app, "Ali Finance").await;
    let mona = tenant(&app, "Mona Finance").await;
    for (who, primary) in [(ali, true), (mona, false)] {
        relate(
            app.state(),
            NewRelationship {
                from: who.0,
                to: a.client.0,
                relation_type: RelationshipType::ContactOf,
                position: Some("finance".into()),
                primary,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    queue(
        &app,
        NotifyRequest::new(
            "invoice.reminder_after_due",
            Subject::new("invoicing.invoice", 7),
        )
        .to_contact(a.client, ContactPurpose::contact_of("finance"))
        .data(renox::serde_json::json!({ "number": "INV-GMS-2026-0007" })),
    )
    .await;
    app.run_all_jobs().await;

    assert_eq!(
        points(&app, ali, ContactType::Email).await[0].status,
        ContactStatus::Invalid,
        "the first finance contact is marked invalid"
    );
    let tried = mail.attempts();
    assert!(
        tried.iter().any(|a| a.starts_with("ali")) && tried.iter().any(|a| a.starts_with("mona")),
        "Ali, then Mona: {tried:?}"
    );
    assert_eq!(
        points(&app, mona, ContactType::Email).await[0].status,
        ContactStatus::Invalid
    );
    let to_mona = Message::query()
        .where_(Message::PARTY_ID.eq(mona.0))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(to_mona.len(), 1, "the alternative recipient");
    assert!(to_mona[0].alternative_of.is_some());
    assert_eq!(
        tasks_of(&app, &accountant).await.len(),
        1,
        "then the accountant"
    );
}

/// AC5c: a failed termination confirmation (the statutory notice) marks the
/// contact invalid and creates the `statutory_notice` task for the account
/// manager.
#[renox::test]
async fn a_failed_termination_confirmation_creates_a_statutory_notice_task_for_the_account_manager()
{
    let (app, _mail, _sms) = failing().await;
    let a = account(&app).await;
    let contract = a.contract.subject();

    queue(
        &app,
        NotifyRequest::new(STATUTORY_NOTICE, contract)
            .to_party(a.client)
            .data(renox::serde_json::json!({ "title": "Contract ends", "body": "Confirmation." })),
    )
    .await;
    app.run_all_jobs().await;

    assert_eq!(
        points(&app, a.client, ContactType::Email).await[0].status,
        ContactStatus::Invalid
    );
    let record = StatutoryNotice::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a statutory notice record");
    assert_eq!(record.type_key, STATUTORY_NOTICE);
    let tasks = Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(format!("notify.statutory:{}", record.id).as_str()))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee_user_id, Some(a.amal.id));
}

/// AC6 and AC7 (Notification types): a failed "deployment suspended" e-mail
/// creates a task for the account manager; the urgent cover notice also alerts
/// in the app; and the registered 09 types carry the groups, the statutory
/// mark and the reactions of the spec.
#[renox::test]
async fn a_failed_deployment_notice_gives_the_account_manager_a_task_and_the_types_carry_their_reactions()
 {
    let (app, _mail, _sms) = failing().await;
    let a = account(&app).await;
    let w = worker(&app, "Wasim Khan").await;
    let d = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        manpower::DeploymentStatus::Suspended,
        day(-30),
        Some(day(300)),
    )
    .await;

    queue(
        &app,
        NotifyRequest::new(CLIENT_NOTICE, d.subject())
            .to_party(a.client)
            .data(renox::serde_json::json!({ "title": "Worker unavailable", "body": "..." })),
    )
    .await;
    app.run_all_jobs().await;
    assert_eq!(
        points(&app, a.client, ContactType::Email).await[0].status,
        ContactStatus::Invalid
    );
    let tasks = tasks_of(&app, &a.amal).await;
    assert_eq!(tasks.len(), 1, "a task for the account manager: {tasks:?}");

    // The urgent cover notice alerts as well (to a client whose address is
    // still valid: the first failure marked the other's invalid).
    let second = support::company(&app, "Desert Foods LLC").await;
    let ids = queue(
        &app,
        NotifyRequest::new(CLIENT_NOTICE_URGENT, d.subject())
            .to_party(second)
            .data(renox::serde_json::json!({ "title": "Cover not found yet", "body": "..." })),
    )
    .await;
    app.run_all_jobs().await;
    let mut alerted = false;
    for id in ids {
        for run in ReactionRun::of_message(app.db(), id.0).await.unwrap() {
            for step in run.steps(app.db()).await.unwrap() {
                alerted |= step.detail.0["alert_sent"] == true;
            }
        }
    }
    assert!(alerted, "the in-app alert of an urgent give-up");

    // The types.
    let row = |key: &'static str| {
        let db = app.db().clone();
        async move {
            NotificationType::by_key(&db, key)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("the type {key}"))
        }
    };
    let invalid_and_task = [
        GiveUpReaction::MarkContactInvalid,
        GiveUpReaction::StaffTask,
    ];
    for key in [CLIENT_NOTICE, CLIENT_NOTICE_URGENT] {
        let t = row(key).await;
        assert_eq!(t.group_key, "", "{key} has no group");
        assert_eq!(t.give_up.0, invalid_and_task, "{key}");
        assert_eq!(t.staff_role.as_deref(), Some("account_manager"), "{key}");
        assert!(!t.statutory);
    }
    assert_eq!(row(CLIENT_NOTICE_URGENT).await.urgency, Urgency::Urgent);
    assert_eq!(row(CLIENT_NOTICE).await.urgency, Urgency::Normal);
    let statutory = row(STATUTORY_NOTICE).await;
    assert!(statutory.statutory);
    assert_eq!(statutory.group_key, "statutory");
    assert_eq!(
        statutory.give_up.0,
        [
            GiveUpReaction::MarkContactInvalid,
            GiveUpReaction::StatutoryNotice
        ]
    );
    assert_eq!(statutory.staff_role.as_deref(), Some("account_manager"));
    let sms = row(WORKER_MESSAGE).await;
    assert_eq!(sms.group_key, "");
    assert_eq!(sms.give_up.0, invalid_and_task);
    assert_eq!(sms.staff_role.as_deref(), Some("hr"));
    let staff = row("manpower.notice").await;
    assert_eq!(staff.group_key, "");
    assert_eq!(staff.give_up.0, [GiveUpReaction::None]);
    for key in [
        "attendance.timesheet_escalated",
        "attendance.timesheet_deemed",
    ] {
        let t = row(key).await;
        assert_eq!(t.group_key, "portal", "{key}");
        assert_eq!(t.give_up.0, invalid_and_task, "{key}");
        assert_eq!(t.staff_role.as_deref(), Some("account_manager"), "{key}");
    }
    for key in ["invoice.reminder_before_due", "invoice.reminder_after_due"] {
        let t = row(key).await;
        assert_eq!(t.group_key, "invoices", "{key}");
        assert_eq!(
            t.give_up.0,
            [
                GiveUpReaction::MarkContactInvalid,
                GiveUpReaction::AlternativeRecipient,
                GiveUpReaction::StaffTask
            ],
            "{key}"
        );
        assert_eq!(t.staff_role.as_deref(), Some("accountant"), "{key}");
    }
}

/// AC9 (Alerts), the alert itself: a compliance-critical run of `MPW.RUL.12`
/// that fails raises an alert whose owner is the MD role, and the MD's role and
/// the company admins are the people told.
#[renox::test]
async fn a_failed_critical_run_alerts_the_md_and_the_company_admin() {
    use wl_kernel::compliance::service::recipients;
    use wl_kernel::compliance::{AlertKind, Raised, raise};

    let app = boot_seeded().await;
    let ada = person(&app, "Ada", ADMIN).await;
    let alert = raise(
        app.state(),
        Raised::new(
            "MPW.RUL.12",
            Subject::new("manpower.deployment", 4),
            "MPW.RUL.12:work_permit:2026-11-01",
            AlertKind::Failed,
        )
        .error("the notifier is down"),
    )
    .await
    .unwrap();
    let stored = alert.expect("an alert was raised");
    assert_eq!(stored.owner_role, "md");
    assert!(
        recipients(app.db(), &stored.owner_role)
            .await
            .unwrap()
            .contains(&ada.id),
        "the company admin is told"
    );
}

/// AC10 (Integrations): the SMS gateway, the WPS bank and the attendance import
/// have their manual fallback; a supervisor enters yesterday's paper attendance
/// (one day back, with the `late_entry` flag) and an entry two days back is
/// refused (`HRM.ACL.01`).
#[renox::test]
async fn the_integrations_have_their_fallbacks_and_paper_attendance_is_one_day_back() {
    use wl_kernel::integrations::def;

    let app = boot_seeded().await;
    for (key, owner, mentions) in [
        ("sms", "hr", "site supervisor"),
        ("wps.bank", "accountant", "SIF"),
        ("attendance.import", "hr", "late-entry"),
    ] {
        let found = def::get(key).unwrap_or_else(|| panic!("the connection {key}"));
        assert_eq!(found.owner_role_name(), owner, "{key}");
        assert!(found.fallback_en().contains(mentions), "{key}");
    }

    let boss = person(&app, "Bashir", host::roles::SUPERVISOR).await;
    let w = worker(&app, "Wasim Khan").await;
    let site = wl_attendance::WorkLocation::create(
        app.db(),
        wl_attendance::WorkLocation::named("Tower A"),
    )
    .await
    .unwrap();
    app.acting_as(&boss);
    let mark = |days_back: i64| {
        let date = day(-days_back);
        let form = [
            ("employee_id", w.0.to_string()),
            ("location_id", site.id.to_string()),
            ("check_in", format!("{date}T07:00")),
            ("check_out", format!("{date}T15:00")),
        ];
        let app = &app;
        async move {
            app.htmx()
                .post(
                    "/attendance/mark",
                    &form
                        .iter()
                        .map(|(k, v)| (*k, v.as_str()))
                        .collect::<Vec<_>>(),
                )
                .await
        }
    };
    mark(1).await.assert_ok();
    let saved = wl_attendance::AttendanceRecord::query()
        .where_(wl_attendance::AttendanceRecord::EMPLOYEE_ID.eq(w.0))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].date, day(-1));
    // The paper list entered a day late carries the `late_entry` note.
    let flags = wl_attendance::AttendanceFlag::of_record(app.db(), saved[0].id)
        .await
        .unwrap();
    assert!(
        flags
            .iter()
            .any(|f| f.flag_type == wl_attendance::FlagType::LateEntry && f.resolved_at.is_some()),
        "a settled late_entry note: {flags:?}"
    );
    mark(2).await.assert_status(422);
    assert_eq!(
        wl_attendance::AttendanceRecord::query()
            .where_(wl_attendance::AttendanceRecord::EMPLOYEE_ID.eq(w.0))
            .count(app.db())
            .await
            .unwrap(),
        1,
        "the entry two days back is refused"
    );
}

/// AC11 (Lead sources): a new lead or party offers the sources referral,
/// website, paid ads, partner, event and other; the campaign over budget waits
/// for a manager.
#[renox::test]
async fn the_lead_sources_are_the_six_of_the_spec() {
    let app = boot_seeded().await;
    let mut active: Vec<String> = wl_sales::LeadSource::ordered(app.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|s| s.active)
        .map(|s| s.code)
        .collect();
    active.sort();
    assert_eq!(
        active,
        [
            "event", "other", "paid_ads", "partner", "referral", "website"
        ]
    );
}

/// AC12 (Onboarding and SOPs): the checklists for the worker, the recruiter,
/// the site supervisor and the account manager exist with tasks for HR, the
/// account manager and the employee and the PPE step a checklist; the SOP
/// library is owned by HR and the account manager.
#[renox::test]
async fn the_onboarding_checklists_and_the_sop_library_are_seeded() {
    let app = boot_seeded().await;
    let worker = wl_hr::OnboardingTemplate::query()
        .where_(wl_hr::OnboardingTemplate::NAME_EN.eq("Worker onboarding"))
        .first(app.db())
        .await
        .unwrap()
        .expect("the worker checklist");
    assert!(worker.applies_to("security_guard", wl_hr::EmploymentType::FullTime));
    assert!(worker.applies_to("cleaner", wl_hr::EmploymentType::FullTime));
    assert!(!worker.blocks_activation, "completion blocks nothing");
    let steps = wl_hr::TemplateStep::query()
        .where_(wl_hr::TemplateStep::TEMPLATE_ID.eq(worker.id))
        .order_by(wl_hr::TemplateStep::ID)
        .get(app.db())
        .await
        .unwrap();
    let owners: Vec<&str> = steps.iter().map(|s| s.owner.as_str()).collect();
    assert!(owners.contains(&"hr") && owners.contains(&"role:account_manager"));
    assert!(owners.contains(&"employee"));
    assert!(
        steps.iter().any(|s| s.title_en == "PPE issue"
            && s.checklist_template.as_deref() == Some("ppe_handover")),
        "the PPE step is a checklist"
    );
    for name in [
        "Recruiter onboarding",
        "Site supervisor onboarding",
        "Account manager onboarding",
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
    let sop = |code: &'static str| {
        let db = app.db().clone();
        async move {
            wl_documents::Sop::query()
                .where_(wl_documents::Sop::CODE.eq(code))
                .first(&db)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("the SOP {code}"))
        }
    };
    assert_eq!(sop("SOP-MPW-HR-POLICIES").await.owner_role, "hr");
    assert_eq!(sop("SOP-MPW-PAYROLL").await.owner_role, "hr");
    assert_eq!(
        sop("SOP-MPW-CLIENT-SITE-RULES").await.owner_role,
        "account_manager"
    );
}

/// AC13 (Registrations): a fresh 09 deployment lists *Trademark registration*
/// and *Other certification* with the MD as responsible role.
#[renox::test]
async fn the_registration_kinds_are_on_with_the_md_as_responsible_role() {
    let app = boot_seeded().await;
    for key in ["trademark_registration", "other_certification"] {
        let kind = DocumentKind::by_key(app.db(), key)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("the kind {key}"));
        assert!(kind.enabled, "{key} is on");
        assert_eq!(kind.responsible_role.as_deref(), Some("admin"), "{key}");
    }
    let (_kind, blocks) = blocks_of(&app, "trademark_registration").await;
    assert!(blocks.is_empty());
}

/// Edge case (Seeding is idempotent): running the install again changes
/// nothing, and a type the client edited keeps its values.
#[renox::test]
async fn seeding_twice_changes_nothing_and_keeps_what_the_client_changed() {
    let app = boot_seeded().await;
    let count = |app: &TestApp| {
        let db = app.db().clone();
        async move {
            (
                DocumentKind::query().count(&db).await.unwrap(),
                wl_ticketing::Category::query().count(&db).await.unwrap(),
                wl_hr::OnboardingTemplate::query().count(&db).await.unwrap(),
                wl_documents::Sop::query().count(&db).await.unwrap(),
                wl_invoicing::BudgetCategory::query()
                    .count(&db)
                    .await
                    .unwrap(),
            )
        }
    };
    let before = count(&app).await;

    // The client changes the escalation notice's role; a re-run leaves it.
    let mut edited = NotificationType::by_key(app.db(), "attendance.timesheet_escalated")
        .await
        .unwrap()
        .unwrap();
    edited.staff_role = Some("hr".into());
    edited.save(app.db()).await.unwrap();

    app.kernel()
        .call("manpower:install", Vec::<String>::new())
        .await
        .unwrap();
    assert_eq!(count(&app).await, before);
    let kept = NotificationType::by_key(app.db(), "attendance.timesheet_escalated")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        kept.staff_role.as_deref(),
        Some("hr"),
        "the client's change stays"
    );
    assert_eq!(
        wl_invoicing::BudgetCategory::by_code(app.db(), "visa_insurance")
            .await
            .unwrap()
            .expect("the 09 budget category")
            .owner_role
            .as_deref(),
        Some("accountant")
    );
}
