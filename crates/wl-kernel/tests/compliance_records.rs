//! The compliance base (wl-kernel §3, §5.15; #32 part 2): the stored flag
//! and owner role per rule, and the alert per occurrence.

use renox::auth::Permissions;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::compliance::{
    self, AlertKind, AlertStatus, ComplianceAlert, ComplianceRule, CriticalRule,
};
use wl_kernel::{ClientConfig, Subject};

const CLIENT: &str = r#"
[client]
name = "Acme Manpower LLC"
app = "manpower"

[modules]
enabled = []

[roles]
owner = "md"

[[legal_entities]]
code = "ACME"
name_en = "Acme Manpower LLC"
default = true
"#;

struct Hr;

impl Module for Hr {
    fn name(&self) -> &'static str {
        "test-hr"
    }

    fn register(&self, _app: &mut Registry) {
        CriticalRule::new("HRM.RUL.03")
            .owner_role("hr_head")
            .register();
        CriticalRule::new("DOC.RUL.03").daily_at("01:00").register();
    }
}

async fn app() -> TestApp {
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Hr),
        config,
    )
    .expect("the kernel applies");
    let app = TestApp::new(app).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app
}

#[renox::test]
async fn install_stores_each_registered_rule_once() {
    let app = app().await;
    let db = app.db();
    let wps = ComplianceRule::of(db, "HRM.RUL.03")
        .await
        .unwrap()
        .expect("stored");
    assert!(wps.critical);
    assert_eq!(wps.owner_role.as_deref(), Some("hr_head"));
    assert!(wps.watched_since.is_some());
    let nightly = ComplianceRule::of(db, "DOC.RUL.03")
        .await
        .unwrap()
        .expect("stored");
    assert_eq!(nightly.owner_role, None);

    assert_eq!(compliance::install(db).await.unwrap(), 0);
    assert_eq!(ComplianceRule::query().count(db).await.unwrap(), 2);
}

#[renox::test]
async fn a_client_cannot_turn_the_flag_off_but_can_change_the_owner() {
    let app = app().await;
    let db = app.db();
    ComplianceRule::where_eq("rule", "HRM.RUL.03")
        .update(
            db,
            &[
                ("critical", &false),
                ("owner_role", &Some("partner".to_owned())),
            ],
        )
        .await
        .unwrap();

    compliance::install(db).await.unwrap();
    let row = ComplianceRule::of(db, "HRM.RUL.03").await.unwrap().unwrap();
    assert!(row.critical, "written back to yes");
    assert_eq!(
        row.owner_role.as_deref(),
        Some("partner"),
        "the owner is kept"
    );
}

#[renox::test]
async fn the_owner_role_is_the_stored_one_then_the_modules_then_the_deployments() {
    let app = app().await;
    let state = app.state();
    // The module named hr_head.
    assert_eq!(
        compliance::owner_role(state, "HRM.RUL.03").await.unwrap(),
        "hr_head"
    );
    // The module named none: the deployment's `[roles] owner`.
    assert_eq!(
        compliance::owner_role(state, "DOC.RUL.03").await.unwrap(),
        "md"
    );

    ComplianceRule::where_eq("rule", "HRM.RUL.03")
        .update(app.db(), &[("owner_role", &Some("partner".to_owned()))])
        .await
        .unwrap();
    assert_eq!(
        compliance::owner_role(state, "HRM.RUL.03").await.unwrap(),
        "partner"
    );
}

#[renox::test]
async fn an_occurrence_has_one_alert() {
    let app = app().await;
    let db = app.db();
    let subject = Subject::new("hr.payroll_run", 4);
    let alert = ComplianceAlert {
        rule: "HRM.RUL.03".into(),
        subject_type: subject.subject_type.clone(),
        subject_id: subject.subject_id,
        occurrence_key: "wps:2026-11".into(),
        kind: AlertKind::Failed,
        error: Some("bank file rejected".into()),
        owner_role: "hr_head".into(),
        ..Default::default()
    };
    ComplianceAlert::create(db, alert.clone()).await.unwrap();

    let found = ComplianceAlert::of_occurrence(db, "HRM.RUL.03", &subject, "wps:2026-11")
        .await
        .unwrap()
        .expect("found");
    assert_eq!(found.status, AlertStatus::Open);
    assert_eq!(found.error.as_deref(), Some("bank file rejected"));
    assert_eq!(ComplianceAlert::open(db).await.unwrap().len(), 1);

    let again = ComplianceAlert::create(db, alert).await.unwrap_err();
    assert!(again.is_unique_violation());
}
