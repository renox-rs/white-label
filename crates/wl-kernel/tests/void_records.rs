//! The void base (wl-kernel §5.12; #27 part 1): the Void record's table.

use renox::auth::Permissions;
use renox::db::{Json, now};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::{ClientConfig, Effect, EffectKind, Subject, VoidRecord};

const CLIENT: &str = r#"
[client]
name = "Acme Properties LLC"
app = "property"

[modules]
enabled = ["parties"]

[[legal_entities]]
code = "ACME"
name_en = "Acme Properties LLC"
default = true
"#;

async fn app() -> TestApp {
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(App::new().module(Auth::new()).module(Permissions), config)
        .expect("the kernel applies");
    TestApp::new(app).await
}

fn void(subject_id: i64) -> VoidRecord {
    VoidRecord {
        subject_type: "payments.receipt".into(),
        subject_id,
        number: Some("RCP-ACME-2026-0042".into()),
        series_key: Some("receipt".into()),
        state_before: "issued".into(),
        reason: "duplicate".into(),
        voided_by: 7,
        effects: Json(vec![
            Effect::new(EffectKind::Allocation, "invoice INV-ACME-2026-0101").amount(300_000),
        ]),
        at: now(),
        ..Default::default()
    }
}

#[renox::test]
async fn a_void_record_keeps_its_number_and_what_was_reversed() {
    let app = app().await;
    VoidRecord::create(app.db(), void(3)).await.unwrap();

    let found = VoidRecord::of_subject(app.db(), &Subject::new("payments.receipt", 3))
        .await
        .unwrap()
        .expect("the record is there");
    assert_eq!(found.number.as_deref(), Some("RCP-ACME-2026-0042"));
    assert_eq!(found.series_key.as_deref(), Some("receipt"));
    assert_eq!(found.reason, "duplicate");
    assert_eq!(found.voided_by, 7);
    assert_eq!(found.approval_request_id, None);
    assert_eq!(found.effects.0.len(), 1);
    assert_eq!(found.effects.0[0].amount, 300_000);
    assert_eq!(found.subject(), Subject::new("payments.receipt", 3));
    assert!(
        VoidRecord::of_subject(app.db(), &Subject::new("payments.receipt", 4))
            .await
            .unwrap()
            .is_none()
    );
}

#[renox::test]
async fn a_record_is_voided_once() {
    let app = app().await;
    VoidRecord::create(app.db(), void(3)).await.unwrap();
    let error = VoidRecord::create(app.db(), void(3)).await.unwrap_err();
    assert!(error.is_unique_violation());
    // Another record of the same type, or the same id of another type, is fine.
    VoidRecord::create(app.db(), void(4)).await.unwrap();
    let mut other = void(3);
    other.subject_type = "bookings.booking".into();
    other.number = None;
    other.series_key = None;
    VoidRecord::create(app.db(), other).await.unwrap();
}
