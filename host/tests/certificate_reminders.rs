//! BL-PRP-25 (#275) point 1 through the real wl-documents: the nightly
//! `DOC.RUL.01` run warns the maintenance coordinator and the property manager
//! of a building safety certificate expiring on 2027-05-31 on 2027-03-02
//! (90 days), 2027-05-01 (30 days) and 2027-05-24 (7 days), and on no other
//! night.

mod support;

use host::roles::{COORDINATOR, PROPERTY_MANAGER, TECHNICIAN};
use property::seed::BUILDING_SAFETY_CERTIFICATE;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_documents::expiry;
use wl_documents::{Document, DocumentKind, DocumentStatus};
use wl_notify::Message;

fn date(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

async fn person(app: &TestApp, name: &str, role: &str) -> User {
    let mut user = User::register(
        app.db(),
        name,
        &format!("{}@example.com", name.to_lowercase()),
        "secret123",
    )
    .await
    .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    user.assign_role(app.db(), role).await.unwrap();
    user
}

/// The users told by the expiry reminder so far, with the days left each
/// message says, oldest first.
async fn told(app: &TestApp) -> Vec<(i64, i64)> {
    let mut messages = Message::query()
        .where_(Message::TYPE_KEY.eq("documents.expiring"))
        .order_by(Message::ID)
        .get(app.db())
        .await
        .unwrap();
    messages.retain(|m| m.user_id.is_some());
    messages
        .iter()
        .map(|m| {
            (
                m.user_id.unwrap(),
                m.data.0["days_left"].as_i64().expect("days left"),
            )
        })
        .collect()
}

#[renox::test]
async fn bl_prp_25_the_certificate_warns_the_coordinator_and_the_property_manager_three_times() {
    let app = TestApp::new(host::app().provide(support::noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    let coordinator = person(&app, "Carla", COORDINATOR).await;
    let manager = person(&app, "Pam", PROPERTY_MANAGER).await;
    // Someone else holding a role of the staff: not told.
    let technician = person(&app, "Tim", TECHNICIAN).await;

    let kind = DocumentKind::by_key(app.db(), BUILDING_SAFETY_CERTIFICATE)
        .await
        .unwrap()
        .expect("the kind is seeded");
    assert_eq!(kind.reminder_offsets.0, [90, 30, 7]);
    Document::create(
        app.db(),
        Document {
            subject_type: "property.building".into(),
            subject_id: 1,
            kind_id: kind.id,
            title: "Tower A safety certificate".into(),
            expiry_date: Some(date("2027-05-31")),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // 2027-03-01 is 91 days away: no offset is reached, nobody is told.
    let report = expiry::run(app.state(), date("2027-03-01")).await.unwrap();
    assert_eq!(report.messages, 0);
    assert!(told(&app).await.is_empty());

    // 2027-03-02: 90 days; the coordinator and the property manager.
    let report = expiry::run(app.state(), date("2027-03-02")).await.unwrap();
    assert_eq!((report.events, report.messages, report.failed), (1, 2, 0));
    let first = told(&app).await;
    assert!(first.contains(&(coordinator.id, 90)), "{first:?}");
    assert!(first.contains(&(manager.id, 90)), "{first:?}");
    assert!(first.iter().all(|(user, _)| *user != technician.id));
    assert_eq!(first.len(), 2);

    // The same night again: nothing more.
    let again = expiry::run(app.state(), date("2027-03-02")).await.unwrap();
    assert_eq!(again.messages, 0);
    assert_eq!(told(&app).await.len(), 2);

    // 2027-05-01: 30 days.
    expiry::run(app.state(), date("2027-05-01")).await.unwrap();
    let second = told(&app).await;
    assert_eq!(second.len(), 4);
    assert!(second.contains(&(coordinator.id, 30)));
    assert!(second.contains(&(manager.id, 30)));

    // 2027-05-24: 7 days.
    expiry::run(app.state(), date("2027-05-24")).await.unwrap();
    let third = told(&app).await;
    assert_eq!(third.len(), 6);
    assert!(third.contains(&(coordinator.id, 7)));
    assert!(third.contains(&(manager.id, 7)));

    // Between the offsets, and after the last, nothing more.
    for night in ["2027-03-03", "2027-04-15", "2027-05-25"] {
        let report = expiry::run(app.state(), date(night)).await.unwrap();
        assert_eq!(report.messages, 0, "{night}");
    }
    assert_eq!(told(&app).await.len(), 6);
}

/// The "Buildings non-compliant" list is open to the maintenance coordinator,
/// the property manager and the company admin, and not to a technician.
#[renox::test]
async fn bl_prp_25_the_compliance_list_is_open_to_the_coordinator_the_manager_and_the_admin() {
    let app = TestApp::new(host::app().provide(support::noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    for (name, role, allowed) in [
        ("Carla", COORDINATOR, true),
        ("Pam", PROPERTY_MANAGER, true),
        ("Alia", host::roles::ADMIN, true),
        ("Tim", TECHNICIAN, false),
    ] {
        let user = person(&app, name, role).await;
        app.acting_as(&user);
        let page = app.get("/property/compliance").await;
        if allowed {
            page.assert_ok();
        } else {
            page.assert_status(403);
        }
    }
}
