//! BL-LOG-25 (#323), 03 §3.3, §4.1, §7.2, §8, §9, `X-13`, `X-19`, `03-N8-a`,
//! `L-19`: the 03 deployment's values for the shared document, rule,
//! integration, notice, late-entry, e-invoicing, sales and staff mechanisms are
//! in force in a fresh 03 deployment (`wl:install` and the seeds), run by the
//! real modules of the host, not as constants. A test names the Done-when
//! point it covers.
//!
//! Dates are relative to the business date (`documents::today()`), so the
//! checks hold at any real date.

mod support;

use host::roles::{ADMIN, DISPATCHER, FLEET_MANAGER, MEMBER, OPERATIONS_MANAGER, SHIPPING_CLIENT};
use logistics::defaults;
use logistics::{VehicleGrounding, grounding};
use renox::auth::permissions::Scope;
use renox::chrono::{Days, Months, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_delivery::{Run, VehicleType};
use wl_documents::expiry::{self, RenewalTask};
use wl_documents::{Document, DocumentKind, DocumentStatus, Sop, SopReader, kinds};
use wl_hr::OnboardingTemplate;
use wl_kernel::compliance::service::recipients;
use wl_kernel::compliance::{AlertKind, Raised, is_critical, raise, schedule_subject};
use wl_kernel::documents::{self, BlockMode, OverrideMode, today};
use wl_kernel::integrations;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceId, VatCategory, invoicer};
use wl_kernel::late_entries::{Assessment, assess, limits_for};
use wl_kernel::notify::{GiveUpReaction, notification_type};
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_parties::{CompanyDetails, NewParty, create};
use wl_sales::LeadSource;

use support::{
    messages_to,
    company, entity, logistics_booted, logistics_booted_with, person, scoped_person, tenant,
};

/// The vehicle (an asset) of the runs.
const VAN: i64 = 7;

fn day(days: i64) -> NaiveDate {
    if days >= 0 {
        today() + Days::new(days.unsigned_abs())
    } else {
        today() - Days::new(days.unsigned_abs())
    }
}

/// The accepted document of `kind` of `subject`, valid through `expires`.
async fn document(app: &TestApp, subject: &Subject, kind: &str, expires: NaiveDate) -> Document {
    let kind = DocumentKind::by_key(app.db(), kind)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the kind {kind} is seeded"));
    Document::create(
        app.db(),
        Document {
            subject_type: subject.subject_type.clone(),
            subject_id: subject.subject_id,
            kind_id: kind.id,
            title: kind.name_en.clone(),
            expiry_date: Some(expires),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

/// The vehicle's subject.
fn van() -> Subject {
    Subject::new(defaults::VEHICLE_SUBJECT, VAN)
}

/// A planned run of `driver` on `date` from `depot` in `vehicle_type`.
async fn run(
    app: &TestApp,
    number: &str,
    driver: &User,
    date: NaiveDate,
    depot: &str,
    vehicle_type: VehicleType,
) -> Run {
    let mut run = Run {
        number: number.to_owned(),
        run_date: date,
        depot: depot.to_owned(),
        driver_user_id: driver.id,
        vehicle_type,
        vehicle_asset_id: (vehicle_type != VehicleType::OnFoot).then_some(VAN),
        ..Run::default()
    };
    run.insert(app.db()).await.unwrap();
    run
}

/// The driver's subject when the user has no employee record.
fn driver_subject(driver: &User) -> Subject {
    Subject::new(defaults::DRIVER_USER_SUBJECT, driver.id)
}

/// AC1: the three kind rows (and the others) are in a fresh 03 deployment
/// with the values of §9: legal blocks, override never, look-ahead 30 days,
/// the responsible roles, `run.dispatch` and `delivery.assign`.
#[renox::test]
async fn a_03_deployment_lists_the_kind_rows_with_their_blocks_and_responsible_roles() {
    let app = logistics_booted().await;
    let expected: [(&str, &[&str], &str); 5] = [
        (
            "driving_licence",
            &["delivery.assign", "run.dispatch"],
            "operations_manager",
        ),
        (
            "rider_permit",
            &["delivery.assign", "run.dispatch"],
            "operations_manager",
        ),
        ("mulkiya", &["run.dispatch"], "fleet_manager"),
        ("vehicle_insurance", &["run.dispatch"], "fleet_manager"),
        ("vehicle_permit", &["run.dispatch"], "fleet_manager"),
    ];
    for (key, guarded, role) in expected {
        let kind = DocumentKind::by_key(app.db(), key)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{key} is seeded"));
        assert!(kind.legal_block && kind.compliance_critical, "{key}");
        assert_eq!(kind.override_mode, OverrideMode::Never, "{key}");
        assert_eq!(kind.look_ahead_days, 30, "{key}");
        assert_eq!(kind.responsible_role.as_deref(), Some(role), "{key}");
        let mut blocks = kind.blocks(app.db()).await.unwrap();
        blocks.sort_by(|a, b| a.guarded.cmp(&b.guarded));
        let names: Vec<&str> = blocks.iter().map(|b| b.guarded.as_str()).collect();
        assert_eq!(names, guarded, "{key}");
        assert!(
            blocks
                .iter()
                .all(|b| b.mode == BlockMode::Block && b.locked)
        );
    }
    // The rider permit is for a motorbike rider only.
    let rider = DocumentKind::by_key(app.db(), "rider_permit")
        .await
        .unwrap()
        .unwrap();
    for block in rider.blocks(app.db()).await.unwrap() {
        let condition = block.condition().expect("a condition");
        assert_eq!(condition.field, "vehicle_type");
        assert_eq!(condition.values, ["motorbike"]);
    }
    // The other kinds only remind their responsible role.
    for (key, role) in [
        ("freight_documents", "documentation_officer"),
        ("client_contract", "accountant"),
        ("rate_card_agreement", "accountant"),
    ] {
        let kind = DocumentKind::by_key(app.db(), key)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{key} is seeded"));
        assert!(!kind.legal_block, "{key}");
        assert_eq!(kind.responsible_role.as_deref(), Some(role), "{key}");
        assert!(kind.blocks(app.db()).await.unwrap().is_empty(), "{key}");
    }
}

/// AC1: refusing `run.dispatch` for an expired Mulkiya shows no override; a
/// motorbike run needs the rider permit and a van run does not.
#[renox::test]
async fn an_expired_mulkiya_refuses_dispatch_and_offers_no_override() {
    let app = logistics_booted().await;
    let driver = person(&app, "Dev", host::roles::DRIVER).await;
    let subject = driver_subject(&driver);
    document(&app, &subject, "driving_licence", day(200)).await;
    document(&app, &van(), "vehicle_insurance", day(200)).await;
    document(&app, &van(), "vehicle_permit", day(200)).await;
    document(&app, &van(), "mulkiya", day(-1)).await;
    let planned = run(
        &app,
        "RUN-T-1",
        &driver,
        today(),
        "Depot A",
        VehicleType::Van,
    )
    .await;

    let guard = documents::guard(app.state());
    let refused = guard
        .check(app.state(), "run.dispatch", planned.id, today())
        .await
        .expect("the check runs")
        .expect_err("an expired Mulkiya refuses the dispatch");
    assert!(
        refused.message().contains("Mulkiya"),
        "{}",
        refused.message()
    );
    assert!(
        refused.override_links().is_empty(),
        "a legal block offers no override"
    );

    // Renewed, the same run may leave; a van needs no rider permit.
    document(&app, &van(), "mulkiya", day(300)).await;
    assert!(
        guard
            .check(app.state(), "run.dispatch", planned.id, today())
            .await
            .unwrap()
            .is_ok()
    );

    // A motorbike run is refused without the rider permit.
    let bike = run(
        &app,
        "RUN-T-2",
        &driver,
        today(),
        "Depot A",
        VehicleType::Motorbike,
    )
    .await;
    let refused = guard
        .check(app.state(), "run.dispatch", bike.id, today())
        .await
        .unwrap()
        .expect_err("a motorbike rider needs the rider permit");
    assert!(refused.message().contains("RTA rider permit"));
    document(&app, &subject, "rider_permit", day(200)).await;
    assert!(
        guard
            .check(app.state(), "run.dispatch", bike.id, today())
            .await
            .unwrap()
            .is_ok()
    );
}

/// AC1: a driver licence expiring in 25 days lists the driver's planned runs
/// after the expiry (and not the run before it, nor another driver's).
#[renox::test]
async fn a_licence_expiring_in_25_days_lists_the_drivers_planned_runs() {
    let app = logistics_booted().await;
    let driver = person(&app, "Dev", host::roles::DRIVER).await;
    let other = person(&app, "Dora", host::roles::DRIVER).await;
    let expires = day(25);
    document(&app, &driver_subject(&driver), "driving_licence", expires).await;
    let after = run(
        &app,
        "RUN-T-1",
        &driver,
        day(30),
        "Depot A",
        VehicleType::Van,
    )
    .await;
    run(
        &app,
        "RUN-T-2",
        &driver,
        day(10),
        "Depot A",
        VehicleType::Van,
    )
    .await;
    run(
        &app,
        "RUN-T-3",
        &other,
        day(30),
        "Depot A",
        VehicleType::Truck,
    )
    .await;

    let entries = wl_documents::blocks_for(app.db(), "run.dispatch")
        .await
        .unwrap();
    let entry = entries
        .iter()
        .find(|e| e.kind == "driving_licence")
        .expect("the driving licence entry");
    let work = documents::future_work("run.dispatch").expect("the provider is registered");
    let planned = work
        .planned_from(
            app.state(),
            entry,
            &driver_subject(&driver),
            expires + Days::new(1),
        )
        .await
        .unwrap();
    let ids: Vec<i64> = planned.iter().map(|p| p.record.subject_id).collect();
    assert_eq!(ids, [after.id]);
    assert_eq!(planned[0].date, day(30));
    // The same provider answers for the assignment.
    assert!(documents::future_work("delivery.assign").is_some());
    assert!(documents::block_subjects("run.dispatch").is_some());
    assert!(documents::block_subjects("delivery.assign").is_some());
}

/// AC1: the company admin cannot change the override mode of the blocking
/// kinds, nor loosen their blocks (`L-19`).
#[renox::test]
async fn the_company_admin_cannot_change_the_override_mode_of_the_three_kinds() {
    let app = logistics_booted().await;
    for key in [
        "driving_licence",
        "mulkiya",
        "vehicle_insurance",
        "vehicle_permit",
    ] {
        let refused = kinds::set_override_mode(app.db(), key, OverrideMode::WithApproval)
            .await
            .unwrap()
            .expect_err("the override mode is locked to never");
        assert_eq!(
            refused.reason,
            wl_documents::RefusalReason::LegalBlock,
            "{key}"
        );
        let kind = DocumentKind::by_key(app.db(), key).await.unwrap().unwrap();
        assert_eq!(kind.override_mode, OverrideMode::Never, "{key}");
    }
    let refused = wl_documents::remove_block(app.db(), "mulkiya", "run.dispatch")
        .await
        .unwrap()
        .expect_err("a locked block cannot be removed");
    assert_eq!(refused.reason, wl_documents::RefusalReason::Locked);
    let refused =
        wl_documents::set_block_mode(app.db(), "mulkiya", "run.dispatch", BlockMode::Flag)
            .await
            .unwrap()
            .expect_err("a locked block cannot be flagged only");
    assert_eq!(refused.reason, wl_documents::RefusalReason::Locked);
}

/// AC2: the expiry reminder of a Depot A driver's licence goes to Depot A's
/// operations managers, not Depot B's; the renewal task for an expired Mulkiya
/// goes to the fleet manager.
#[renox::test]
async fn the_reminder_goes_to_the_drivers_depot_and_the_mulkiya_task_to_the_fleet_manager() {
    let app = logistics_booted().await;
    let depot_a = Scope::new("logistics.depot", 1);
    let depot_b = Scope::new("logistics.depot", 2);
    let olivia = scoped_person(&app, "Olivia", OPERATIONS_MANAGER, &depot_a).await;
    let omar = scoped_person(&app, "Omar", OPERATIONS_MANAGER, &depot_b).await;
    let fadi = person(&app, "Fadi", FLEET_MANAGER).await;

    // Hiring the driver into Depot A.
    let user = person(&app, "Dev", host::roles::DRIVER).await;
    let party = tenant(&app, "Dev Driver").await;
    let employee = wl_hr::employees::create(
        app.state(),
        wl_hr::NewEmployee::new(party, "driver", day(-30))
            .user(user.id)
            .scope(Subject::new("logistics.depot", 1)),
    )
    .await
    .unwrap();
    document(&app, &employee.subject(), "driving_licence", day(30)).await;
    document(&app, &van(), "mulkiya", day(-1)).await;

    let kind = DocumentKind::by_key(app.db(), "driving_licence")
        .await
        .unwrap()
        .unwrap();
    let told = expiry::resolve(
        app.state(),
        "operations_manager",
        &kind.key,
        &employee.subject(),
        &[],
    )
    .await
    .unwrap();
    let told: Vec<i64> = told.iter().map(|r| r.user_id).collect();
    assert!(told.contains(&olivia.id), "Depot A's operations manager");
    assert!(!told.contains(&omar.id), "not Depot B's");

    let report = expiry::run(app.state(), today()).await.unwrap();
    assert_eq!(report.failed, 0, "{report:?}");
    // The message the run sent about the licence (30 days left) went to
    // Depot A's operations manager and to nobody at Depot B.
    assert_eq!(
        messages_to(&app, expiry::run::NOTIFICATION_TYPE, &olivia)
            .await
            .len(),
        1,
        "Depot A's operations manager is told once"
    );
    assert!(
        messages_to(&app, expiry::run::NOTIFICATION_TYPE, &omar)
            .await
            .is_empty(),
        "Depot B's operations manager is not"
    );
    let mulkiya = DocumentKind::by_key(app.db(), "mulkiya")
        .await
        .unwrap()
        .unwrap();
    let task = RenewalTask::find_open(app.db(), defaults::VEHICLE_SUBJECT, VAN, mulkiya.id)
        .await
        .unwrap()
        .expect("a renewal task for the expired Mulkiya");
    let ticket = wl_ticketing::Ticket::find_or_404(app.db(), task.ticket_id.expect("a ticket"))
        .await
        .unwrap();
    assert_eq!(ticket.assignee_user_id, Some(fadi.id));
}

/// AC3: the notification types name the groups of §9, which decide what a
/// message that gives up does.
#[renox::test]
async fn the_notification_types_take_the_reaction_of_their_group() {
    let _app = logistics_booted().await;
    let reactions = |key: &str| {
        let def = notification_type(key).unwrap_or_else(|| panic!("{key} is registered"));
        (
            def.give_up.clone(),
            def.staff_role.clone(),
            def.statutory,
            def.group.clone(),
        )
    };
    // A COD statement e-mail that gives up goes to the client's other finance
    // contact, then creates a task for the accountant.
    let (statement, role, statutory, group) = reactions("logistics.statement_notice");
    assert_eq!(
        statement,
        [
            GiveUpReaction::MarkContactInvalid,
            GiveUpReaction::AlternativeRecipient,
            GiveUpReaction::StaffTask
        ]
    );
    assert_eq!(
        (role.as_deref(), statutory, group.as_str()),
        (Some("accountant"), false, "invoices")
    );
    // A failed-attempt notice (a client shipment notice) that gives up creates
    // no task.
    let (tracking, _, _, group) = reactions("logistics.client_notice");
    assert_eq!(tracking, [GiveUpReaction::None]);
    assert_eq!(group, "tracking");
    // An address confirmation request that gives up marks the contact invalid
    // and creates a task for the dispatcher; the freight requests, holds and
    // free-time alerts do for the documentation officer; the bank account
    // confirmation for the accountant.
    let portal = vec![
        GiveUpReaction::MarkContactInvalid,
        GiveUpReaction::StaffTask,
    ];
    for (key, owner) in [
        ("logistics.action_request_dispatcher", "dispatcher"),
        (
            "logistics.action_request_documentation",
            "documentation_officer",
        ),
        ("logistics.account_confirm", "accountant"),
    ] {
        let (reaction, role, statutory, group) = reactions(key);
        assert_eq!(reaction, portal, "{key}");
        assert_eq!(role.as_deref(), Some(owner), "{key}");
        assert!(!statutory, "no 03 type is statutory: {key}");
        assert_eq!(group, "portal", "{key}");
    }
}

/// AC4: `LOG.RUL.10`, `DOC.RUL.03` and `WRK.RUL.15` are compliance-critical;
/// when `LOG.RUL.10` fails to run the company admin gets the owner alert, once.
#[renox::test]
async fn a_failed_log_rul_10_alerts_the_company_admin() {
    let app = logistics_booted().await;
    for rule in defaults::CRITICAL_RULES {
        assert!(is_critical(rule), "{rule}");
    }
    let rule = wl_kernel::compliance::rule("LOG.RUL.10").expect("registered");
    assert!(
        matches!(rule.schedule(), Some(wl_kernel::compliance::Schedule::DailyAt(at)) if at.as_str() == "00:30"),
        "the nightly run is watched for a missed run"
    );
    let admin = person(&app, "Ada", ADMIN).await;
    let manager = person(&app, "Olivia", OPERATIONS_MANAGER).await;
    let raised = Raised::new(
        "LOG.RUL.10",
        schedule_subject(),
        "LOG.RUL.10:2026-11-10",
        AlertKind::Failed,
    )
    .error("the document guard failed");
    let alert = raise(app.state(), raised.clone())
        .await
        .unwrap()
        .expect("an alert");
    assert_eq!(alert.owner_role, "operations_manager", "03's owner role");
    let told = recipients(app.db(), &alert.owner_role).await.unwrap();
    assert!(told.contains(&admin.id), "the company admin is alerted too");
    assert!(told.contains(&manager.id));
    assert!(
        raise(app.state(), raised).await.unwrap().is_none(),
        "one alert per occurrence"
    );
}

/// AC4 (behaviour): the nightly `LOG.RUL.10` itself fails (the grounding
/// cannot be stored) on five attempts with the kernel's backoff; the run is
/// `failed` and the owner alert goes to the company admin and the operations
/// manager, once.
#[renox::test]
async fn the_rule_that_fails_for_good_alerts_the_admin_and_the_owner_role() {
    use host::logistics_rules::run_ground_rule;
    use wl_kernel::automation::RunOutcome;
    use wl_kernel::compliance::ComplianceAlert;
    let app = logistics_booted().await;
    let admin = person(&app, "Ada", ADMIN).await;
    let manager = person(&app, "Olivia", OPERATIONS_MANAGER).await;
    let driver = person(&app, "Dev", host::roles::DRIVER).await;
    document(&app, &van(), "mulkiya", day(-3)).await;
    run(&app, "RUN-T-1", &driver, day(1), "Depot A", VehicleType::Van).await;
    // The grounding cannot be stored: the rule fails.
    renox::db::sql("DROP TABLE logistics_vehicle_groundings")
        .execute(app.db())
        .await
        .unwrap();

    let night = today();
    for wait in [60_u64, 300, 900, 3600] {
        let outcome = app
            .at_travelled_time(run_ground_rule(app.state(), night))
            .await
            .unwrap();
        assert_eq!(outcome, RunOutcome::Retrying);
        app.travel(std::time::Duration::from_secs(wait));
    }
    let outcome = app
        .at_travelled_time(run_ground_rule(app.state(), night))
        .await
        .unwrap();
    assert_eq!(outcome, RunOutcome::Failed, "the fifth attempt fails for good");

    let alerts = ComplianceAlert::query().get(app.db()).await.unwrap();
    assert_eq!(alerts.len(), 1, "one alert");
    assert_eq!(alerts[0].rule, "LOG.RUL.10");
    assert_eq!(alerts[0].owner_role, "operations_manager");
    let told = recipients(app.db(), &alerts[0].owner_role).await.unwrap();
    assert!(told.contains(&admin.id), "the company admin is alerted");
    assert!(told.contains(&manager.id), "and the owner role");
    // A sixth call does not alert again.
    app.at_travelled_time(run_ground_rule(app.state(), night))
        .await
        .unwrap();
    assert_eq!(ComplianceAlert::query().count(app.db()).await.unwrap(), 1);
}

/// `LOG.RUL.10` grounds a vehicle whose Mulkiya expired, tells the fleet
/// manager and the dispatcher, and runs once for the day.
#[renox::test]
async fn the_rule_grounds_a_vehicle_with_an_expired_document_once() {
    let app = logistics_booted().await;
    let fadi = person(&app, "Fadi", FLEET_MANAGER).await;
    let dina = person(&app, "Dina", DISPATCHER).await;
    let driver = person(&app, "Dev", host::roles::DRIVER).await;
    document(&app, &van(), "mulkiya", day(-3)).await;
    document(&app, &van(), "vehicle_insurance", day(100)).await;
    run(
        &app,
        "RUN-T-1",
        &driver,
        day(1),
        "Depot A",
        VehicleType::Van,
    )
    .await;

    let outcome = host::logistics_rules::run_ground_rule(app.state(), today())
        .await
        .unwrap();
    assert_eq!(outcome, wl_kernel::automation::RunOutcome::Succeeded);
    let grounded = grounding::groundings_of(app.db(), VAN).await.unwrap();
    assert_eq!(grounded.len(), 1);
    assert_eq!(grounded[0].document_kind, "mulkiya");
    assert_eq!(grounded[0].expired_on, day(-3));
    let messages = wl_notify::Message::query().get(app.db()).await.unwrap();
    for user in [fadi.id, dina.id] {
        assert!(
            messages.iter().any(|m| m.user_id == Some(user)),
            "user {user} is told"
        );
    }
    // The same day again is a skipped run: nothing is grounded twice.
    let again = host::logistics_rules::run_ground_rule(app.state(), today())
        .await
        .unwrap();
    assert_eq!(again, wl_kernel::automation::RunOutcome::Skipped);
    assert_eq!(VehicleGrounding::query().count(app.db()).await.unwrap(), 1);
}

/// AC5: the SMS gateway is a connection with a manual fallback; tracking SMS
/// are queued while it is not up.
#[renox::test]
async fn the_sms_gateway_is_registered_with_its_fallback_and_queues_while_down() {
    let app = logistics_booted().await;
    let def = integrations::get(defaults::SMS_GATEWAY).expect("the SMS gateway is registered");
    assert_eq!(def.owner_role_name(), "dispatcher");
    assert!(def.fallback_en().contains("queued"));
    assert!(
        def.fallback_en()
            .contains("dispatcher calls the recipients")
    );
    let row = integrations::ensure(app.db(), defaults::SMS_GATEWAY)
        .await
        .unwrap();
    assert_eq!(row.owner_role, "dispatcher");
    assert_eq!(row.status, integrations::ConnectionStatus::Up);

    integrations::report_failure(app.state(), defaults::SMS_GATEWAY, "gateway timeout")
        .await
        .unwrap();
    let submitted = integrations::submit(
        app.state(),
        integrations::Outbound::new(
            defaults::SMS_GATEWAY,
            "tracking_sms",
            renox::serde_json::json!({ "phone": "+971500000001", "text": "Out for delivery" }),
        ),
    )
    .await
    .unwrap();
    assert!(
        matches!(submitted, integrations::Submitted::Queued { .. }),
        "{submitted:?}"
    );
}

/// AC5: the e-invoicing ASP is down for 24 hours: the task is raised after
/// `FIN.RUL.16`'s 24 hours, a value of the deployment, not of code.
#[renox::test]
async fn the_asp_task_is_raised_after_24_hours() {
    let _app = logistics_booted().await;
    let invoicing = host::invoicing_for("logistics", &["NEC".to_owned()]);
    assert_eq!(invoicing.config().einvoice_task_hours, 24);
}

/// AC6: an e-POD entered 3 hours after the stop is flagged with a reason; one
/// entered 30 hours after needs an operations manager's approval.
#[renox::test]
async fn a_late_epod_is_flagged_after_30_minutes_and_needs_approval_after_24_hours() {
    let app = logistics_booted().await;
    let limits = limits_for(app.db(), defaults::EPOD_LATE_ENTRY)
        .await
        .unwrap();
    assert_eq!(limits.flag_after_minutes, 30);
    assert_eq!(limits.approval_after_hours, 24);
    let period = wl_kernel::late_entries::Period::Open;
    let stop = renox::db::now() - renox::chrono::Duration::hours(3);
    let found = assess(limits, stop, renox::db::now(), &period);
    assert!(matches!(found, Assessment::Flagged { .. }), "{found:?}");
    let stop = renox::db::now() - renox::chrono::Duration::hours(30);
    let found = assess(limits, stop, renox::db::now(), &period);
    assert!(
        matches!(found, Assessment::NeedsApproval { .. }),
        "{found:?}"
    );
    let stop = renox::db::now() - renox::chrono::Duration::minutes(10);
    let found = assess(limits, stop, renox::db::now(), &period);
    assert!(!found.is_late());
    // The operations manager approves; the digest goes to the same managers.
    let manager = person(&app, "Olivia", OPERATIONS_MANAGER).await;
    let approvers = renox::auth::permissions::users_with_permission(
        app.db(),
        wl_kernel::permissions::LATE_ENTRIES_APPROVE,
    )
    .await
    .unwrap();
    assert!(approvers.iter().any(|u| u.id == manager.id));
}

/// An invoice for the freight job `job`, of a client, issued on `issued`.
async fn job_invoice(
    app: &TestApp,
    client: PartyId,
    job: &logistics::FreightJob,
    issued: NaiveDate,
) -> i64 {
    let draft = InvoiceDraft::new(client, job.subject()).line(DraftLine::new(
        "Freight",
        100_000,
        VatCategory::Standard,
    ));
    let id: InvoiceId = invoicer(app.state())
        .draft(app.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    wl_invoicing::Invoice::query()
        .where_(wl_invoicing::Invoice::ID.eq(id.0))
        .update(app.db(), &[("issue_date", &Some(issued))])
        .await
        .unwrap();
    id.0
}

/// A freight job of `mode` for `client`, marked international when asked.
async fn freight_job(
    app: &TestApp,
    client: PartyId,
    mode: &str,
    international: bool,
) -> logistics::FreightJob {
    logistics::freight::create(
        app.state(),
        None,
        client,
        "Cargo",
        &logistics::freight::NewJob {
            mode: mode.to_owned(),
            international,
            ..logistics::freight::NewJob::default()
        },
    )
    .await
    .unwrap()
}

/// AC7: an invoice for an international air job (an air waybill) issued within
/// 24 months of go-live is not transmitted; a sea freight invoice is; so is the
/// air one after the 24 months.
#[renox::test]
async fn an_air_waybill_invoice_is_left_out_of_the_mandate_for_24_months() {
    let go_live = day(-100);
    let app = logistics_booted_with(
        host::Tuning::default().invoicing(move |m| m.einvoicing("NEC", go_live, "fake")),
    )
    .await;
    let client = company(&app, "Souq Express Trading LLC").await;
    let nec = entity(&app, "NEC").await;
    let air = freight_job(&app, client, "air", true).await;
    let sea = freight_job(&app, client, "sea", true).await;
    let domestic_air = freight_job(&app, client, "air", false).await;
    let scope = host::logistics_seed::AirFreightExclusion;
    use wl_invoicing::einvoice_adapter::EInvoiceScope;
    let in_scope = |invoice: i64| {
        let subject = InvoiceId(invoice).subject();
        let nec = nec.clone();
        let state = app.state().clone();
        async move {
            scope
                .in_scope(&state, &nec, &subject, Some(PartyKind::Company))
                .await
                .unwrap()
        }
    };
    let within = job_invoice(&app, client, &air, go_live + Days::new(30)).await;
    assert!(!in_scope(within).await, "air waybill, within 24 months");
    let sea_invoice = job_invoice(&app, client, &sea, go_live + Days::new(30)).await;
    assert!(in_scope(sea_invoice).await, "sea freight");
    let domestic = job_invoice(&app, client, &domestic_air, go_live + Days::new(30)).await;
    assert!(
        in_scope(domestic).await,
        "an air job that is not international"
    );
    let after = job_invoice(&app, client, &air, go_live + Months::new(24)).await;
    assert!(in_scope(after).await, "after the 24 months");
    let last_day = job_invoice(&app, client, &air, go_live + Months::new(24) - Days::new(1)).await;
    assert!(!in_scope(last_day).await, "the last day of the window");
}

/// AC8: the lead sources 03 uses are offered, walk-in and aggregator are off,
/// no source requires a campaign and a cost over a campaign's budget needs a
/// manager (`SAL.ACL.07`).
#[renox::test]
async fn the_lead_sources_and_the_campaign_approval_are_seeded() {
    let app = logistics_booted().await;
    for code in defaults::LEAD_SOURCES {
        let source = LeadSource::by_code(app.db(), code)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{code}"));
        assert!(source.active, "{code}");
    }
    for code in ["walk_in", "aggregator"] {
        let source = LeadSource::by_code(app.db(), code).await.unwrap().unwrap();
        assert!(!source.active, "03 does not use {code}");
    }
    let sales = host::logistics_seed::sales(wl_sales::WlSales::new());
    assert!(sales.config().over_budget_approval);
    assert!(sales.config().campaign_required_sources.is_empty());
}

/// AC9: hiring a driver starts the driver checklist with steps for the
/// operations manager, the line manager and the employee; the role is active
/// before it is complete; the SOP library has the owners and readers of §5, and
/// a second run of the seed changes nothing.
#[renox::test]
async fn hiring_a_driver_starts_the_checklist_and_the_sops_are_seeded() {
    let app = logistics_booted().await;
    for (name, role) in [
        ("Driver onboarding", "driver"),
        ("Hub staff onboarding", "hub_staff"),
        ("Dispatcher onboarding", "dispatcher"),
        ("Customer service onboarding", "customer_service"),
    ] {
        let template = OnboardingTemplate::query()
            .where_(OnboardingTemplate::NAME_EN.eq(name))
            .first(app.db())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{name}"));
        assert_eq!(template.job_roles, role);
        assert!(!template.blocks_activation, "{name}");
    }

    let user = person(&app, "Dev", host::roles::DRIVER).await;
    let party = tenant(&app, "Dev Driver").await;
    let employee = wl_hr::employees::create(
        app.state(),
        wl_hr::NewEmployee::new(party, "driver", today()).user(user.id),
    )
    .await
    .unwrap();
    assert_eq!(employee.status, wl_hr::EmployeeStatus::Onboarding);
    let onboarding = wl_hr::Onboarding::query()
        .where_(wl_hr::Onboarding::EMPLOYEE_ID.eq(employee.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("the onboarding started");
    let steps = wl_hr::OnboardingStep::query()
        .where_(wl_hr::OnboardingStep::ONBOARDING_ID.eq(onboarding.id))
        .get(app.db())
        .await
        .unwrap();
    let owners: Vec<&str> = steps.iter().map(|s| s.owner.as_str()).collect();
    for owner in ["role:operations_manager", "manager", "employee"] {
        assert!(owners.contains(&owner), "{owner}");
    }
    assert_eq!(
        host::logistics_seed::hr(wl_hr::WlHr::new())
            .config()
            .manager_role,
        "dispatcher"
    );

    let dispatch = Sop::query()
        .where_(Sop::CODE.eq("SOP-LOG-DISPATCH"))
        .first(app.db())
        .await
        .unwrap()
        .expect("seeded");
    assert_eq!(dispatch.owner_role, "operations_manager");
    assert_eq!(dispatch.ack_due_days, 7);
    let readers: Vec<String> = SopReader::query()
        .where_(SopReader::SOP_ID.eq(dispatch.id))
        .get(app.db())
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.role_key)
        .collect();
    assert!(readers.contains(&"driver".to_owned()));
    let vehicle = Sop::query()
        .where_(Sop::CODE.eq("SOP-LOG-VEHICLE-CHECK"))
        .first(app.db())
        .await
        .unwrap()
        .expect("seeded");
    assert_eq!(vehicle.owner_role, "fleet_manager");

    let sops = Sop::query().count(app.db()).await.unwrap();
    let installed = host::logistics_seed::install(app.state()).await.unwrap();
    assert_eq!(installed.total(), 0, "a second run changes nothing");
    assert_eq!(Sop::query().count(app.db()).await.unwrap(), sops);
}

/// AC10: a fresh 03 deployment lists *Trademark registration* and *Other
/// certification* with the company admin as responsible role; a trademark
/// expiring in 90 days notifies the company admin.
#[renox::test]
async fn the_registration_kinds_have_the_company_admin_and_a_trademark_notifies_it() {
    let app = logistics_booted().await;
    let admin = person(&app, "Ada", ADMIN).await;
    for key in ["trademark_registration", "other_certification"] {
        let kind = DocumentKind::by_key(app.db(), key)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{key}"));
        assert_eq!(kind.responsible_role.as_deref(), Some("admin"), "{key}");
        assert!(kind.enabled, "{key}");
        assert!(kind.blocks(app.db()).await.unwrap().is_empty(), "{key}");
    }
    // The reminder days are per kind: 180/90/30 for a trademark, 90/30/7 for
    // another certification.
    for (key, days) in [
        ("trademark_registration", defaults::TRADEMARK_REMINDERS),
        ("other_certification", defaults::OTHER_CERTIFICATION_REMINDERS),
    ] {
        let kind = DocumentKind::by_key(app.db(), key).await.unwrap().unwrap();
        assert_eq!(kind.reminder_offsets.0, days, "{key}");
    }
    let nec = entity(&app, "NEC").await;
    let subject = Subject::new("kernel.legal_entity", nec.id);
    document(&app, &subject, "trademark_registration", day(90)).await;
    expiry::run(app.state(), today()).await.unwrap();
    let messages = wl_notify::Message::query().get(app.db()).await.unwrap();
    assert!(
        messages.iter().any(|m| m.user_id == Some(admin.id)),
        "the company admin is told"
    );
}

/// The compliance page shows the 03 rows, each with its value, who set it and
/// the spec; a member does not see it.
#[renox::test]
async fn an_admin_sees_the_03_compliance_settings_and_a_member_does_not() {
    let app = logistics_booted().await;
    let admin = person(&app, "Ada", ADMIN).await;
    app.acting_as(&admin);
    app.get("/settings/compliance")
        .await
        .assert_ok()
        .assert_see("Driving licence")
        .assert_see("never")
        .assert_see("LOG.RUL.10")
        .assert_see("sms.gateway")
        .assert_see("L-19");
    let member = person(&app, "Mia", MEMBER).await;
    app.acting_as(&member);
    app.get("/settings/compliance").await.assert_forbidden();
    let client = person(&app, "Noor", SHIPPING_CLIENT).await;
    app.acting_as(&client);
    app.get("/settings/compliance").await.assert_forbidden();
}

/// BL-LOG-11 (#309), BL-LOG-06 (#304), BL-LOG-09: the claim ticket of a loss,
/// disposal or hold and the re-weigh dispute open in a fresh 03 deployment.
/// The app opens them in `hold::CLAIM_CATEGORY` and
/// `reweigh::DISPUTE_CATEGORY`; the real wl-ticketing refuses a category it
/// does not have, so a missing seed would end every claim as a failed ticket.
#[renox::test]
async fn the_claim_and_the_reweigh_dispute_open_in_the_real_ticket_desk() {
    use wl_kernel::tickets::OpenTicket;
    let app = logistics_booted().await;
    let subject = Subject::new("logistics.shipment", 1);
    for category in [
        logistics::hold::CLAIM_CATEGORY,
        logistics::reweigh::DISPUTE_CATEGORY,
    ] {
        let ticket = wl_ticketing::Tickets::open_at(
            app.state(),
            OpenTicket::new(subject.clone(), category, "A claim"),
            renox::db::now(),
        )
        .await
        .unwrap_or_else(|e| panic!("{category} is a ticket category of 03: {e:?}"));
        assert!(ticket.id.0 > 0);
    }
    let refused = wl_ticketing::Tickets::open_at(
        app.state(),
        OpenTicket::new(subject, "no_such_category", "A claim"),
        renox::db::now(),
    )
    .await;
    assert!(refused.is_err(), "the real desk refuses an unknown category");
}

/// AC5 (ASP half): with the e-invoicing ASP out of reach (no adapter answers)
/// the invoice is issued all the same, its transmission stays queued, and
/// 24 hours after the issue `FIN.RUL.16` raises one task: not before, not
/// twice.
#[renox::test]
async fn an_asp_that_is_down_for_24_hours_leaves_the_invoice_issued_and_raises_the_task() {
    use wl_invoicing::EInvoiceTransmission;
    use wl_invoicing::task::{InvoicingTask, TaskKind};
    let app = logistics_booted_with(
        host::Tuning::default().invoicing(|m| m.einvoicing("NEC", day(-10), "asp")),
    )
    .await;
    let clerk = person(&app, "Ibrahim", ADMIN).await;
    let client = create(
        app.state(),
        NewParty {
            kind: PartyKind::Company,
            legal_name_en: "Souq Express Trading LLC".into(),
            company: Some(CompanyDetails {
                einvoice_recipient_id: Some("0235:100000000000011".into()),
                ..CompanyDetails::default()
            }),
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id();
    let invoice = support::issued_invoice(
        &app,
        &clerk,
        client,
        100_000,
        wl_kernel::invoicing::VatCategory::Standard,
        day(30),
    )
    .await;
    assert_eq!(invoice.status, wl_kernel::invoicing::InvoiceStatus::Issued);
    let queued = EInvoiceTransmission::query().get(app.db()).await.unwrap();
    assert_eq!(queued.len(), 1, "the transmission waits for the ASP");

    let stuck = |app: &TestApp| {
        let db = app.db().clone();
        async move {
            InvoicingTask::query()
                .where_(InvoicingTask::KIND.eq(TaskKind::EinvoiceStuck))
                .get(&db)
                .await
                .unwrap()
        }
    };
    app.travel(std::time::Duration::from_secs(23 * 3600));
    app.at_travelled_time(wl_invoicing::einvoice_send::run_due(app.state()))
        .await
        .unwrap();
    assert!(stuck(&app).await.is_empty(), "not before 24 hours");

    app.travel(std::time::Duration::from_secs(2 * 3600));
    for _ in 0..2 {
        app.at_travelled_time(wl_invoicing::einvoice_send::run_due(app.state()))
            .await
            .unwrap();
    }
    let tasks = stuck(&app).await;
    assert_eq!(tasks.len(), 1, "one task, once");
    assert!(tasks[0].message.contains("24 hours"), "{}", tasks[0].message);
}

/// Sends the notice `type_key` to `party` with a bouncing mailbox, runs the
/// queued jobs, and answers the staff tasks of the give-up (tickets whose
/// source key is that of a give-up about the shipment).
async fn gives_up(
    app: &TestApp,
    type_key: &str,
    party: PartyId,
) -> Vec<wl_ticketing::Ticket> {
    use wl_kernel::notify::{self, NotifyRequest};
    let request = NotifyRequest::new(type_key, Subject::new("logistics.shipment", 1))
        .subject_label("S-1")
        .to_party(party)
        .data(renox::serde_json::json!({ "title": "Notice", "body": "Please note" }));
    notify::notifier(app.state())
        .notify(app.state(), request)
        .await
        .unwrap();
    app.run_all_jobs().await;
    wl_ticketing::Ticket::query()
        .order_by(wl_ticketing::Ticket::ID)
        .get(app.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|t| {
            t.source_key
                .as_deref()
                .is_some_and(|k| k.starts_with("notify.give_up:logistics.shipment:"))
        })
        .collect()
}

/// The status of the e-mail contact point of `party`.
async fn email_status(app: &TestApp, party: PartyId) -> wl_parties::ContactStatus {
    wl_parties::ContactPoint::query()
        .where_(wl_parties::ContactPoint::PARTY_ID.eq(party.0))
        .where_(wl_parties::ContactPoint::CONTACT_TYPE.eq(wl_parties::ContactType::Email))
        .first(app.db())
        .await
        .unwrap()
        .expect("an e-mail contact")
        .status
}

/// AC3 (behaviour): a COD statement e-mail that gives up marks the contact
/// invalid and opens a task for the accountant; a failed-attempt notice that
/// gives up does nothing more; an address-confirmation request that gives up
/// marks the contact invalid and opens a task for the dispatcher.
#[renox::test]
async fn a_notice_that_gives_up_is_followed_by_the_reaction_of_its_group() {
    use host::roles::ACCOUNTANT;
    let mail = support::BouncingMail::default();
    let app = logistics_booted_with(host::Tuning::default().notify({
        let mail = mail.clone();
        move |n| n.provider(mail)
    }))
    .await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let dispatcher = person(&app, "Dina", host::roles::DISPATCHER).await;

    // A failed-attempt notice (tracking group): nothing follows.
    let tracked = company(&app, "Tracked Trading LLC").await;
    let tasks = gives_up(&app, logistics::notices::CLIENT_NOTICE, tracked).await;
    assert!(!mail.attempts().is_empty(), "the notice was e-mailed");
    assert!(tasks.is_empty(), "a tracking notice that gives up opens no task");

    // The COD statement (invoices group): the contact is invalid and the
    // accountant has the task.
    let billed = company(&app, "Billed Trading LLC").await;
    let tasks = gives_up(&app, logistics::notices::STATEMENT_NOTICE, billed).await;
    assert_eq!(email_status(&app, billed).await, wl_parties::ContactStatus::Invalid);
    assert_eq!(tasks.len(), 1, "one task for the accountant");
    assert_eq!(tasks[0].assignee_user_id, Some(accountant.id));

    // The address confirmation (portal group): the dispatcher has the task.
    let asked = company(&app, "Asked Trading LLC").await;
    let tasks = gives_up(&app, logistics::notices::ACTION_REQUEST_DISPATCHER, asked).await;
    assert_eq!(email_status(&app, asked).await, wl_parties::ContactStatus::Invalid);
    let for_dispatcher: Vec<_> = tasks
        .iter()
        .filter(|t| t.assignee_user_id == Some(dispatcher.id))
        .collect();
    assert_eq!(for_dispatcher.len(), 1, "one task for the dispatcher");
}

/// AC9: `HRM.RUL.17` reminds the assignees of an overdue onboarding step on
/// the first run after the due date and again every 2 days, and tells HR (the
/// operations manager) after 5 days, once.
#[renox::test]
async fn an_overdue_onboarding_step_is_reminded_every_2_days_and_escalated_after_5() {
    let app = logistics_booted().await;
    let boss = person(&app, "Olivia", OPERATIONS_MANAGER).await;
    let user = person(&app, "Dev", host::roles::DRIVER).await;
    let party = tenant(&app, "Dev Driver").await;
    let employee = wl_hr::employees::create(
        app.state(),
        wl_hr::NewEmployee::new(party, "driver", today()).user(user.id),
    )
    .await
    .unwrap();
    let onboarding = wl_hr::Onboarding::query()
        .where_(wl_hr::Onboarding::EMPLOYEE_ID.eq(employee.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("the onboarding started");
    let dues: Vec<NaiveDate> = wl_hr::OnboardingStep::query()
        .where_(wl_hr::OnboardingStep::ONBOARDING_ID.eq(onboarding.id))
        .get(app.db())
        .await
        .unwrap()
        .into_iter()
        .map(|step| step.due_date)
        .collect();
    let first_due = *dues.iter().min().expect("steps");
    let last_due = *dues.iter().max().expect("steps");

    // Once every step is overdue the cadence is the same for all of them.
    let at = |days: u64| first_due + Days::new(days);
    let earliest = || async {
        wl_hr::OnboardingStep::query()
            .where_(wl_hr::OnboardingStep::ONBOARDING_ID.eq(onboarding.id))
            .where_(wl_hr::OnboardingStep::DUE_DATE.eq(first_due))
            .first(app.db())
            .await
            .unwrap()
            .expect("the earliest step")
    };
    let before = wl_hr::reminders::run(app.state(), first_due).await.unwrap();
    assert_eq!(before.reminded, 0, "nothing on the due date itself");
    let first = wl_hr::reminders::run(app.state(), at(1)).await.unwrap();
    assert!(first.reminded > 0, "reminded on the first run after the due date");
    assert_eq!(first.escalated, 0, "not yet");
    assert_eq!(earliest().await.last_reminded_on, Some(at(1)));
    wl_hr::reminders::run(app.state(), at(2)).await.unwrap();
    assert_eq!(
        earliest().await.last_reminded_on,
        Some(at(1)),
        "not the next day"
    );
    wl_hr::reminders::run(app.state(), at(3)).await.unwrap();
    assert_eq!(
        earliest().await.last_reminded_on,
        Some(at(3)),
        "again after 2 days"
    );
    let four = wl_hr::reminders::run(app.state(), at(4)).await.unwrap();
    assert_eq!(four.escalated, 0, "4 days overdue");

    let escalated = wl_hr::reminders::run(app.state(), at(5)).await.unwrap();
    assert!(escalated.escalated > 0, "told after 5 days");
    assert!(
        !messages_to(&app, "hr.onboarding_step_escalated", &boss)
            .await
            .is_empty(),
        "the operations manager is told"
    );
    let told_on = |step: &wl_hr::OnboardingStep| step.escalated_on;
    let earliest = earliest().await;
    assert_eq!(told_on(&earliest), Some(at(5)));
    wl_hr::reminders::run(app.state(), at(9)).await.unwrap();
    let still = wl_hr::OnboardingStep::find_or_404(app.db(), earliest.id)
        .await
        .unwrap();
    assert_eq!(told_on(&still), Some(at(5)), "told once");
    let _ = last_due;
}
