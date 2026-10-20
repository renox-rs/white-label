//! BL-PRP-27 (#277), the compliance settings of the Property client run by
//! the real modules of the host: the owner alert of a compliance-critical rule
//! (`PLT.RUL.04`), the Civil Defence approval through the real `resolve` and
//! `activate` transitions (`WRK.ACL.11`), its look-ahead list for the
//! coordinators 30 days before the expiry and their renewal task when it
//! expires (`DOC.RUL.01` to `DOC.RUL.03`), the tenant's Emirates ID and the
//! trademark registration reminders.

mod support;

use host::roles::{ADMIN, COORDINATOR, PROPERTY_MANAGER};
use property::renewal;
use renox::auth::permissions::{Scope, define_role};
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_kernel::automation::{MAX_ATTEMPTS, RuleRun, RuleRunSpec, run_rule};
use wl_kernel::compliance::service::NOTIFICATION_TYPE as OWNER_ALERT;
use wl_kernel::compliance::{is_critical, occurrence_key, schedule_subject};
use wl_kernel::documents::today;
use wl_kernel::ClientConfig;
use wl_ticketing::{NewContract, ServiceContract, ServiceContracts, Ticket, TicketStatus};

/// Renames the lease table so that every rule reading it fails, and back.
async fn break_leases(app: &TestApp) {
    renox::db::sql("ALTER TABLE property_leases RENAME TO property_leases_off")
        .execute(app.db())
        .await
        .unwrap();
}

async fn mend_leases(app: &TestApp) {
    renox::db::sql("ALTER TABLE property_leases_off RENAME TO property_leases")
        .execute(app.db())
        .await
        .unwrap();
}

/// Runs `PRP.RUL.09` for today until it has failed for good: five attempts,
/// each when the one before is due again.
async fn fail_the_deadline_run(app: &TestApp) {
    break_leases(app).await;
    for _ in 0..MAX_ATTEMPTS {
        renewal::run_deadlines(app.state(), today()).await.unwrap();
        RuleRun::query()
            .update(
                app.db(),
                &[("next_attempt_at", &Some(renox::db::now() - renox::chrono::Duration::hours(1)))],
            )
            .await
            .unwrap();
    }
    mend_leases(app).await;
}

/// When the `PRP.RUL.09` run fails, each company admin gets the owner alert
/// once (owner role and company admin are the same role by default).
#[renox::test]
async fn a_failed_prp_rul_09_run_alerts_each_company_admin_once() {
    let app = booted().await;
    let ada = person(&app, "Ada", ADMIN).await;
    let omar = person(&app, "Omar", ADMIN).await;
    let pam = person(&app, "Pam", PROPERTY_MANAGER).await;

    fail_the_deadline_run(&app).await;
    assert_eq!(messages_to(&app, OWNER_ALERT, &ada).await.len(), 1);
    assert_eq!(messages_to(&app, OWNER_ALERT, &omar).await.len(), 1);
    assert!(messages_to(&app, OWNER_ALERT, &pam).await.is_empty());
    let alert = &messages_to(&app, OWNER_ALERT, &ada).await[0];
    assert_eq!(alert.data.0["rule"], "PRP.RUL.09");

    // The failed run is over: another run for the same day alerts no one.
    renewal::run_deadlines(app.state(), today()).await.unwrap();
    assert_eq!(messages_to(&app, OWNER_ALERT, &ada).await.len(), 1);
}

/// With the managing director named as the owner role, the managing director
/// and the company admin are both alerted.
#[renox::test]
async fn with_the_managing_director_as_owner_role_both_are_alerted() {
    let config = ClientConfig::load(host::DEMO_CLIENT).expect("the demo client");
    let mut config = config;
    config.roles.owner = Some("managing_director".to_owned());
    let app = TestApp::new(host::app_for(config).expect("the app").provide(noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    define_role(app.db(), "managing_director", &[]).await.unwrap();
    let md = person(&app, "Mia", "managing_director").await;
    let ada = person(&app, "Ada", ADMIN).await;
    let pam = person(&app, "Pam", PROPERTY_MANAGER).await;

    fail_the_deadline_run(&app).await;
    assert_eq!(messages_to(&app, OWNER_ALERT, &md).await.len(), 1);
    assert_eq!(messages_to(&app, OWNER_ALERT, &ada).await.len(), 1);
    assert!(messages_to(&app, OWNER_ALERT, &pam).await.is_empty());
}

/// A non-critical rule (`PRP.RUL.12`, the lease that waits for its signature)
/// that fails does not alert the owner, run by the property or by the kernel.
#[renox::test]
async fn a_failed_non_critical_rule_does_not_alert_the_owner() {
    let app = booted().await;
    let ada = person(&app, "Ada", ADMIN).await;
    assert!(!is_critical("PRP.RUL.12"));
    assert!(is_critical("PRP.RUL.09"));

    break_leases(&app).await;
    assert!(property::chase::run(app.state(), today()).await.is_err());
    // The same rule id through the kernel's engine, failing every attempt.
    let spec = RuleRunSpec::new(
        "PRP.RUL.12",
        schedule_subject(),
        occurrence_key("PRP.RUL.12", today()),
    );
    for _ in 0..MAX_ATTEMPTS {
        run_rule(app.state(), spec.clone(), |_state| async {
            Err(Error::Status(
                StatusCode::INTERNAL_SERVER_ERROR,
                "the chase failed".to_owned(),
            ))
        })
        .await
        .unwrap();
        RuleRun::query()
            .update(
                app.db(),
                &[("next_attempt_at", &Some(renox::db::now() - renox::chrono::Duration::hours(1)))],
            )
            .await
            .unwrap();
    }
    mend_leases(&app).await;
    assert!(messages_to(&app, OWNER_ALERT, &ada).await.is_empty());
    assert_eq!(count_messages(&app, OWNER_ALERT).await, 0);
}

/// A fire ticket and a draft fire contract of `vendor`, in a building.
struct Fire {
    ticket: Ticket,
    contract: ServiceContract,
}

async fn fire_work(app: &TestApp, vendor: wl_kernel::PartyId, code: &str) -> Fire {
    let (_building, subject) = building(app, code).await;
    let ticket = vendor_ticket(app, &subject, "fire_life_safety", vendor, None).await;
    let contract = ServiceContracts::create_draft(
        app.state(),
        NewContract {
            vendor,
            title: "Fire alarm maintenance".into(),
            start_date: day(0),
            end_date: day(365),
            renewal_notice_days: 60,
            sla_policy_id: None,
        },
    )
    .await
    .unwrap();
    ServiceContracts::add_coverage(app.db(), &contract, subject, "fire_life_safety")
        .await
        .unwrap();
    Fire { ticket, contract }
}

async fn status(app: &TestApp, ticket: &Ticket) -> TicketStatus {
    Ticket::find_or_404(app.db(), ticket.id).await.unwrap().status
}

/// With vendor V's Civil Defence approval expired, resolving V's fire and life
/// safety ticket is refused with no override (an admin cannot either),
/// activating a new fire contract for V is refused, a cleaning ticket of V
/// resolves, and a vendor with a valid approval resolves its fire ticket.
#[renox::test]
async fn an_expired_civil_defence_approval_refuses_the_real_resolve_and_activate() {
    let app = booted().await;
    let coordinator = person(&app, "Carla", COORDINATOR).await;
    let admin = person(&app, "Alia", ADMIN).await;
    let v = company(&app, "SafeFire LLC").await;
    let w = company(&app, "Firewatch LLC").await;
    let expired = day(-2);
    civil_defence_approval(&app, v, expired).await;
    civil_defence_approval(&app, w, day(200)).await;

    let fire = fire_work(&app, v, "TWR-A").await;
    app.acting_as(&coordinator);
    app.htmx()
        .post(&format!("/tickets/{}/actions/start", fire.ticket.id), &[])
        .await
        .assert_status(303);
    for user in [&coordinator, &admin] {
        app.acting_as(user);
        let refused = app
            .htmx()
            .post(&format!("/tickets/{}/actions/resolve", fire.ticket.id), &[])
            .await;
        refused.assert_status(422);
        let body = refused.text();
        assert!(
            body.contains(&format!("Civil Defence approval expired on {expired}")),
            "{body}"
        );
        assert!(body.contains("override: not allowed"), "{body}");
    }
    assert_eq!(status(&app, &fire.ticket).await, TicketStatus::InProgress);

    // A new fire and life safety contract for V cannot be activated.
    app.acting_as(&admin);
    let activation = app
        .htmx()
        .post(
            &format!("/service-contracts/{}/activate", fire.contract.id),
            &[("x", "y")],
        )
        .await;
    activation.assert_status(422);
    assert!(
        activation.text().contains("Civil Defence approval"),
        "{}",
        activation.text()
    );
    assert_eq!(
        ServiceContract::find_or_404(app.db(), fire.contract.id)
            .await
            .unwrap()
            .status,
        wl_ticketing::ContractStatus::Draft
    );

    // A cleaning ticket of the same vendor resolves.
    let (_b, subject) = building(&app, "TWR-B").await;
    let cleaning = vendor_ticket(&app, &subject, "general", v, None).await;
    app.acting_as(&coordinator);
    app.htmx()
        .post(&format!("/tickets/{}/actions/start", cleaning.id), &[])
        .await
        .assert_status(303);
    app.htmx()
        .post(&format!("/tickets/{}/actions/resolve", cleaning.id), &[])
        .await
        .assert_status(303);
    assert_eq!(status(&app, &cleaning).await, TicketStatus::Resolved);

    // A vendor whose approval is valid resolves its fire ticket.
    let valid = fire_work(&app, w, "TWR-C").await;
    app.htmx()
        .post(&format!("/tickets/{}/actions/start", valid.ticket.id), &[])
        .await
        .assert_status(303);
    app.htmx()
        .post(&format!("/tickets/{}/actions/resolve", valid.ticket.id), &[])
        .await
        .assert_status(303);
    assert_eq!(status(&app, &valid.ticket).await, TicketStatus::Resolved);
}

/// The look-ahead of the Civil Defence approval, 30 days before it expires:
/// each coordinator of a building where the vendor has fire and life safety
/// visits planned after the expiry date gets the list of those visits (and
/// not the ones before it, nor a cleaning visit, nor another building's
/// coordinator); the property manager is told too. On the day after the
/// expiry the coordinators of those buildings get the expiry notice and their
/// renewal task is opened, once.
#[renox::test]
async fn the_coordinators_of_the_vendors_buildings_get_the_look_ahead_and_the_renewal_task() {
    use wl_documents::expiry;
    use wl_documents::{RenewalTask, DocumentKind};

    let app = booted().await;
    let pam = person(&app, "Pam", PROPERTY_MANAGER).await;
    let ada = person(&app, "Ada", ADMIN).await;
    let carla = person(&app, "Carla", host::roles::MEMBER).await;
    let cora = person(&app, "Cora", host::roles::MEMBER).await;
    let (tower_a, a) = building(&app, "TWR-A").await;
    let (tower_b, b) = building(&app, "TWR-B").await;
    let (_tower_c, c) = building(&app, "TWR-C").await;
    // Carla coordinates tower A, Cora tower B; nobody coordinates tower C.
    carla
        .assign_role_in(app.db(), COORDINATOR, &Scope::of_id::<property::Building>(tower_a.id))
        .await
        .unwrap();
    cora
        .assign_role_in(app.db(), COORDINATOR, &Scope::of_id::<property::Building>(tower_b.id))
        .await
        .unwrap();
    let v = company(&app, "SafeFire LLC").await;
    let expires = day(30);
    civil_defence_approval(&app, v, expires).await;

    // Visits: one in A after the expiry, one in A before it (left out), a
    // cleaning visit in A after it (left out), one in B after it.
    let after_a = vendor_ticket(&app, &a, "fire_life_safety", v, Some(day(45))).await;
    let before_a = vendor_ticket(&app, &a, "fire_life_safety", v, Some(day(10))).await;
    let cleaning_a = vendor_ticket(&app, &a, "general", v, Some(day(45))).await;
    let after_b = vendor_ticket(&app, &b, "fire_life_safety", v, Some(day(50))).await;
    let _ = c;

    // 30 days before: the first night inside the look-ahead.
    expiry::run(app.state(), today()).await.unwrap();
    let to_carla = messages_to(&app, "documents.expiring", &carla).await;
    let to_cora = messages_to(&app, "documents.expiring", &cora).await;
    assert_eq!(to_carla.len(), 1);
    assert_eq!(to_cora.len(), 1);
    let body = |m: &wl_notify::Message| m.rendered_body.clone();
    assert!(body(&to_carla[0]).contains(&after_a.number), "{}", body(&to_carla[0]));
    assert!(!body(&to_carla[0]).contains(&before_a.number));
    assert!(!body(&to_carla[0]).contains(&cleaning_a.number));
    assert!(!body(&to_carla[0]).contains(&after_b.number), "another building");
    assert!(body(&to_cora[0]).contains(&after_b.number));
    assert!(!body(&to_cora[0]).contains(&after_a.number));
    // The property manager of the kind's also-notify roles is told as well.
    assert!(!messages_to(&app, "documents.expiring", &pam).await.is_empty());

    // The day after the expiry: the expiry notice and the renewal task.
    let report = expiry::run(app.state(), expires + renox::chrono::Days::new(1))
        .await
        .unwrap();
    assert_eq!((report.expired, report.expired_events, report.tasks), (1, 1, 1));
    assert!(!messages_to(&app, "documents.expired", &carla).await.is_empty());
    assert!(!messages_to(&app, "documents.expired", &cora).await.is_empty());
    assert!(!messages_to(&app, "documents.expired", &pam).await.is_empty());
    assert!(
        !messages_to(&app, "documents.expired", &ada).await.is_empty(),
        "the owner role is told of a compliance-critical expiry"
    );
    let tasks = RenewalTask::query().get(app.db()).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].closed_at.is_none());
    assert!(tasks[0].ticket_id.is_some(), "the task is a ticket");
    // Once: another night opens no second task.
    let again = expiry::run(app.state(), expires + renox::chrono::Days::new(2))
        .await
        .unwrap();
    assert_eq!(again.tasks, 0);
    let kind = DocumentKind::by_key(app.db(), "civil_defence_approval")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tasks[0].kind_id, kind.id);
}

/// A tenant's Emirates ID expiring in 90 days notifies the property manager
/// and the tenant (`DOC.RUL.01`, a KYC document).
#[renox::test]
async fn an_expiring_emirates_id_notifies_the_property_manager_and_the_tenant() {
    use wl_documents::{Document, DocumentKind, DocumentStatus, expiry};

    let app = booted().await;
    let pam = person(&app, "Pam", PROPERTY_MANAGER).await;
    let tara = tenant(&app, "Tara Nasser").await;
    let kind = DocumentKind::by_key(app.db(), "emirates_id")
        .await
        .unwrap()
        .unwrap();
    Document::create(
        app.db(),
        Document {
            subject_type: "parties.party".into(),
            subject_id: tara.0,
            kind_id: kind.id,
            title: "Emirates ID".into(),
            expiry_date: Some(day(90)),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    expiry::run(app.state(), today()).await.unwrap();
    let to_manager = messages_to(&app, "documents.expiring", &pam).await;
    assert_eq!(to_manager.len(), 1);
    assert_eq!(to_manager[0].data.0["days_left"], 90);
    let to_tenant = messages_to_party(&app, "documents.expiring", tara).await;
    assert_eq!(to_tenant.len(), 1, "the tenant is told too");
}

/// A trademark registration (a kind 01 turns on, responsible role company
/// admin) expiring in 90 days notifies the company admin.
#[renox::test]
async fn a_trademark_registration_expiring_in_ninety_days_notifies_the_company_admin() {
    use wl_documents::{Document, DocumentKind, DocumentStatus, expiry};

    let app = booted().await;
    let ada = person(&app, "Ada", ADMIN).await;
    let pam = person(&app, "Pam", PROPERTY_MANAGER).await;
    let pgp = entity(&app, "PGP").await;
    let kind = DocumentKind::by_key(app.db(), "trademark_registration")
        .await
        .unwrap()
        .unwrap();
    assert!(kind.enabled);
    assert_eq!(kind.responsible_role.as_deref(), Some(ADMIN));
    Document::create(
        app.db(),
        Document {
            subject_type: "kernel.legal_entity".into(),
            subject_id: pgp.id,
            kind_id: kind.id,
            title: "Palm Grove trademark".into(),
            expiry_date: Some(day(90)),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    expiry::run(app.state(), today()).await.unwrap();
    let told = messages_to(&app, "documents.expiring", &ada).await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data.0["days_left"], 90);
    assert!(messages_to(&app, "documents.expiring", &pam).await.is_empty());
}
