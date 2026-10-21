//! BL-MPW-09 (#423), `DOC.RUL.02`, `MPW.ACL.07` (the providers), through the
//! real host of a manpower agency: the deployment registers `BlockSubjects` and
//! `FutureWork`, the nightly look-ahead of wl-documents lists the deployments
//! an expiring document stops, they are flagged until the renewal is accepted
//! and the account manager gets one message per document per night.

mod support;

use host::roles::ACCOUNT_MANAGER;
use manpower::documents::DeploymentDocumentFlag;
use manpower::permissions::client_scope;
use manpower::{ClientContract, ContractStatus, Deployment, DeploymentStatus, Site};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{noon_clock, person, scoped_person, without_document_kinds};
use wl_documents::expiry;
use wl_documents::kinds::{KindSeed, blocks_for, install_seeds};
use wl_documents::{Document, DocumentKind, DocumentStatus, Documents, NewDocument};
use wl_kernel::documents::{
    self, BlockCondition, BlockMode, DocumentAccepted, DocumentExpiring, block_subjects,
    future_work,
};
use wl_kernel::{ClientConfig, PartyId, Subject};
use wl_notify::Message;

/// A manpower agency: the vertical on, with the shared modules it reaches.
const CLIENT: &str = r#"
[client]
name = "Gulf Manpower Services LLC"
app = "manpower"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "hr", "attendance", "portal"]

[[legal_entities]]
code = "GMS"
name_en = "Gulf Manpower Services LLC"
default = true
"#;

/// The client company.
const TOWER: PartyId = PartyId(31);

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

fn employee(id: i64) -> Subject {
    Subject::new("hr.employee", id)
}

/// The agency: the document kinds of a security agency are installed.
async fn boot() -> TestApp {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap().provide(noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    // The fixtures below replace the kinds the host seeded.
    without_document_kinds(&app).await;
    let visa = KindSeed::new("residence_visa", "Residence visa")
        .subject_type("hr.employee")
        .expires()
        .reminders(&[60, 30, 7])
        .look_ahead(30)
        .responsible_role("hr")
        .also_notify(&[ACCOUNT_MANAGER])
        .blocks("deployment.activate", BlockMode::Block);
    let sira = KindSeed::new("sira_licence", "SIRA licence")
        .subject_type("hr.employee")
        .expires()
        .reminders(&[30])
        .look_ahead(30)
        .responsible_role("hr")
        .also_notify(&[ACCOUNT_MANAGER])
        .blocks_when(
            "deployment.activate",
            BlockMode::Block,
            BlockCondition::new("role", ["security_guard"]),
        );
    install_seeds(app.db(), [&visa, &sira]).await.unwrap();
    app
}

/// A document of `kind` for the employee `who`, accepted, valid through `until`.
async fn document(app: &TestApp, who: i64, kind: &str, until: NaiveDate) -> Document {
    let kind = DocumentKind::by_key(app.db(), kind).await.unwrap().unwrap();
    Document::create(
        app.db(),
        Document {
            subject_type: "hr.employee".into(),
            subject_id: who,
            kind_id: kind.id,
            title: kind.key.clone(),
            expiry_date: Some(until),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

/// A site, and an active contract for the client.
async fn contract_and_site(app: &TestApp) -> (ClientContract, Site) {
    let mut contract = ClientContract {
        reference: "CC-0001".into(),
        client_party_id: TOWER.0,
        start_date: date(2026, 1, 1),
        end_date: date(2027, 12, 31),
        payment_terms_days: 30,
        notice_period_days: 30,
        status: ContractStatus::Active,
        ..ClientContract::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    contract
        .insert_in_state(&mut tx, ContractStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let site = Site::create(
        app.db(),
        Site {
            client_party_id: TOWER.0,
            name: "Tower A".into(),
            active: true,
            ..Site::default()
        },
    )
    .await
    .unwrap();
    (contract, site)
}

/// A deployment of the employee `who` as `role` at `site`, in `status`.
async fn deployment(
    app: &TestApp,
    place: &(ClientContract, Site),
    who: i64,
    role: &str,
    status: DeploymentStatus,
    start: NaiveDate,
    end: Option<NaiveDate>,
) -> Deployment {
    let (contract, site) = place;
    let mut deployment = Deployment {
        employee_id: who,
        client_contract_id: contract.id,
        client_party_id: contract.client_party_id,
        site_id: site.id,
        role: role.into(),
        shift_type: "day".into(),
        start_date: start,
        end_date: end,
        status,
        ..Deployment::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    deployment
        .insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    deployment
}

/// The messages of `documents.expiring` to `user`.
async fn told(app: &TestApp, user: &User) -> Vec<Message> {
    Message::query()
        .where_(Message::TYPE_KEY.eq("documents.expiring"))
        .where_(Message::USER_ID.eq(user.id))
        .order_by(Message::ID)
        .get(app.db())
        .await
        .unwrap()
}

/// AC1: with W's visa expiring 2026-12-10 and W's active deployment D ending
/// 2027-03-31, the nightly run on 2026-11-10 lists D, flags it, and the account
/// manager gets one message, however many deployments are listed; a second run
/// the same night changes nothing.
#[renox::test]
async fn the_nightly_run_flags_the_deployment_and_tells_the_account_manager_once() {
    let app = boot().await;
    let amal = scoped_person(&app, "Amal", ACCOUNT_MANAGER, &client_scope(TOWER.0)).await;
    let place = contract_and_site(&app).await;
    document(&app, 41, "residence_visa", date(2026, 12, 10)).await;
    let d = deployment(
        &app,
        &place,
        41,
        "security_guard",
        DeploymentStatus::Active,
        date(2026, 6, 1),
        Some(date(2027, 3, 31)),
    )
    .await;
    // A second deployment of W, planned after the expiry: the same message.
    let planned = deployment(
        &app,
        &place,
        41,
        "security_guard",
        DeploymentStatus::Planned,
        date(2027, 4, 1),
        Some(date(2027, 9, 30)),
    )
    .await;

    expiry::run(app.state(), date(2026, 11, 10)).await.unwrap();

    for id in [d.id, planned.id] {
        let flags = DeploymentDocumentFlag::open_for(app.db(), id)
            .await
            .unwrap();
        assert_eq!(flags.len(), 1, "deployment {id}");
        assert_eq!(
            flags[0].text(),
            "document expiring residence visa 2026-12-10"
        );
    }
    let messages = told(&app, &amal).await;
    assert_eq!(
        messages.len(),
        1,
        "one message for the document: {messages:?}"
    );
    let items = messages[0].data.0["items"].as_array().unwrap().len();
    assert_eq!(items, 2, "listing both deployments");

    expiry::run(app.state(), date(2026, 11, 10)).await.unwrap();
    assert_eq!(
        told(&app, &amal).await.len(),
        1,
        "nothing more the same night"
    );
    assert_eq!(
        DeploymentDocumentFlag::open_for(app.db(), d.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// AC2: a deployment ending 2026-12-05, before the visa expires on 2026-12-10,
/// is not listed or flagged, and the account manager's reminder for it lists
/// nothing.
#[renox::test]
async fn a_deployment_that_ends_before_the_expiry_is_not_listed() {
    let app = boot().await;
    let amal = scoped_person(&app, "Amal", ACCOUNT_MANAGER, &client_scope(TOWER.0)).await;
    let place = contract_and_site(&app).await;
    document(&app, 42, "residence_visa", date(2026, 12, 10)).await;
    let ends_early = deployment(
        &app,
        &place,
        42,
        "security_guard",
        DeploymentStatus::Active,
        date(2026, 6, 1),
        Some(date(2026, 12, 5)),
    )
    .await;
    // Ending on the expiry date itself is still covered: the visa is valid
    // through it.
    let ends_on_expiry = deployment(
        &app,
        &place,
        42,
        "security_guard",
        DeploymentStatus::Suspended,
        date(2026, 6, 1),
        Some(date(2026, 12, 10)),
    )
    .await;

    expiry::run(app.state(), date(2026, 11, 10)).await.unwrap();

    for id in [ends_early.id, ends_on_expiry.id] {
        assert!(
            DeploymentDocumentFlag::open_for(app.db(), id)
                .await
                .unwrap()
                .is_empty(),
            "deployment {id}"
        );
    }
    for message in told(&app, &amal).await {
        assert!(message.data.0["items"].as_array().unwrap().is_empty());
    }
}

/// AC3: after a renewed visa is accepted, the flag disappears; a renewal that
/// is rejected leaves it.
#[renox::test]
async fn the_flag_goes_when_the_renewed_document_is_accepted() {
    let app = boot().await;
    let rana = person(&app, "Rana", host::roles::ADMIN).await;
    let place = contract_and_site(&app).await;
    document(&app, 43, "residence_visa", date(2026, 12, 10)).await;
    let d = deployment(
        &app,
        &place,
        43,
        "cleaner",
        DeploymentStatus::Active,
        date(2026, 6, 1),
        None,
    )
    .await;
    expiry::run(app.state(), date(2026, 11, 10)).await.unwrap();
    assert_eq!(
        DeploymentDocumentFlag::open_for(app.db(), d.id)
            .await
            .unwrap()
            .len(),
        1,
        "an open-ended deployment runs past any expiry"
    );

    let upload = |name: &str| Upload::new(name, "application/pdf", b"%PDF-1.7 renewal".to_vec());
    let rejected = Documents::attach(
        app.state(),
        None,
        NewDocument::new(employee(43), "residence_visa", upload("v1.pdf"))
            .expires_on(date(2028, 12, 10)),
    )
    .await
    .unwrap()
    .expect("attached");
    app.acting_as(&rana);
    app.htmx()
        .post(
            &format!("/documents/{}/reject", rejected.id),
            &[("reason", "Unreadable scan")],
        )
        .await
        .assert_ok();
    assert_eq!(
        DeploymentDocumentFlag::open_for(app.db(), d.id)
            .await
            .unwrap()
            .len(),
        1,
        "a rejected renewal does not clear the flag"
    );

    let renewal = Documents::attach(
        app.state(),
        None,
        NewDocument::new(employee(43), "residence_visa", upload("v2.pdf"))
            .expires_on(date(2028, 12, 10)),
    )
    .await
    .unwrap()
    .expect("attached");
    app.acting_as(&rana);
    app.htmx()
        .post(&format!("/documents/{}/accept", renewal.id), &[])
        .await
        .assert_ok();
    assert!(
        DeploymentDocumentFlag::open_for(app.db(), d.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// AC4: `BlockSubjects` for a security guard's deployment at site S returns W
/// for the SIRA licence, scoped to S; for a cleaner it returns none, so the
/// licence does not apply; the visa applies to both. The guard works through
/// the provider: an expired SIRA licence refuses the guard, a valid one lets
/// the cleaner through.
#[renox::test]
async fn the_provider_maps_a_deployment_to_the_employee_scoped_to_the_site() {
    let app = boot().await;
    let place = contract_and_site(&app).await;
    let guard = deployment(
        &app,
        &place,
        44,
        "security_guard",
        DeploymentStatus::Planned,
        date(2027, 1, 1),
        None,
    )
    .await;
    let cleaner = deployment(
        &app,
        &place,
        45,
        "cleaner",
        DeploymentStatus::Planned,
        date(2027, 1, 1),
        None,
    )
    .await;
    let entries = blocks_for(app.db(), "deployment.activate").await.unwrap();
    let entry = |kind: &str| entries.iter().find(|e| e.kind == kind).unwrap().clone();
    let provider = block_subjects("deployment.activate").expect("registered");
    let state = app.state();

    assert_eq!(
        provider
            .subjects(state, &entry("sira_licence"), guard.id)
            .await
            .unwrap(),
        [employee(44)]
    );
    assert!(
        provider
            .subjects(state, &entry("sira_licence"), cleaner.id)
            .await
            .unwrap()
            .is_empty()
    );
    for (deployment, who) in [(&guard, 44), (&cleaner, 45)] {
        assert_eq!(
            provider
                .subjects(state, &entry("residence_visa"), deployment.id)
                .await
                .unwrap(),
            [employee(who)]
        );
    }
    assert_eq!(
        provider
            .scope(state, &entry("sira_licence"), guard.id)
            .await
            .unwrap(),
        Some(guard.site_scope())
    );
    for guarded in ["deployment.resume", "deployment.extend"] {
        assert!(block_subjects(guarded).is_some(), "{guarded}");
    }
    assert!(future_work("deployment.activate").is_some());

    // The guard uses it.
    document(&app, 44, "residence_visa", date(2027, 6, 1)).await;
    document(&app, 44, "sira_licence", date(2026, 12, 31)).await;
    document(&app, 45, "residence_visa", date(2027, 6, 1)).await;
    let docs = documents::guard(state);
    let refused = docs
        .check(state, "deployment.activate", guard.id, date(2027, 1, 1))
        .await
        .unwrap()
        .unwrap_err();
    assert!(
        refused.message().contains("SIRA licence"),
        "{}",
        refused.message()
    );
    assert!(
        docs.check(state, "deployment.activate", cleaner.id, date(2027, 1, 1))
            .await
            .unwrap()
            .is_ok()
    );
}

/// The events the listeners hear are the kernel's: a `DocumentExpiring` that
/// lists a deployment flags it, and one for another record changes nothing.
#[renox::test]
async fn only_deployments_of_the_event_are_flagged() {
    let app = boot().await;
    let place = contract_and_site(&app).await;
    let d = deployment(
        &app,
        &place,
        46,
        "cleaner",
        DeploymentStatus::Active,
        date(2026, 6, 1),
        None,
    )
    .await;
    let accepted = DocumentAccepted {
        document: documents::DocumentId(1),
        subject: Subject::new("property.lease", 3),
        kind: "residence_visa".into(),
        expires_on: None,
    };
    app.state().emit(accepted).await.unwrap();
    let event = DocumentExpiring {
        document_id: documents::DocumentId(9),
        subject: employee(46),
        kind: "residence_visa".into(),
        expires_on: date(2026, 12, 10),
        days_left: 30,
        offset: None,
        affected: vec![documents::AffectedGroup {
            guarded: "deployment.activate".into(),
            records: vec![documents::PlannedRecord {
                record: Subject::new("property.lease", 3),
                date: date(2026, 12, 11),
                label: "A lease".into(),
                scope: None,
            }],
        }],
    };
    app.state().emit(event).await.unwrap();
    assert!(
        DeploymentDocumentFlag::open_for(app.db(), d.id)
            .await
            .unwrap()
            .is_empty()
    );
}
