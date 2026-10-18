//! The host reads the client's `client.toml` (wl-kernel §5.1).

use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::{ClientConfig, LegalEntity};

#[renox::test]
async fn the_demo_client_boots_with_its_legal_entities() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let default = LegalEntity::default_entity(app.db()).await.unwrap();
    assert_eq!(default.code, "PGP");
    assert_eq!(LegalEntity::query().count(app.db()).await.unwrap(), 2);
    // The client's brand colour is the theme every page links.
    app.get("/").await.assert_ok().assert_see("/_renox/theme/");
}

/// #55 AC10: the demo boots with `notify`; wl-parties answers `contacts`
/// for the notifier, and a message to a seeded party goes out by Renox mail.
#[renox::test]
async fn the_demo_client_sends_notifications_to_its_parties() {
    use wl_kernel::notify::{self, MessageId, NotificationTypeDef, NotifyChannel, NotifyRequest};
    use wl_kernel::parties::directory;

    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let al_noor = wl_parties::Party::query()
        .where_(wl_parties::Party::LEGAL_NAME_EN.eq("Al Noor Trading LLC"))
        .first(app.db())
        .await
        .unwrap()
        .expect("the seeded party");
    let contacts = directory(app.state())
        .contacts(app.db(), al_noor.party_id())
        .await
        .unwrap()
        .expect("wl-parties answers");
    let email = contacts.point_for(NotifyChannel::Email).expect("an e-mail");
    assert_eq!(email.address, "accounts@alnoortrading.example");
    assert!(email.primary);

    notify::register_type(
        NotificationTypeDef::new("host_test.notice", "property").template(
            NotifyChannel::Email,
            "en",
            Some("Notice {{ number }}"),
            "Your lease {{ number }} is renewed.",
        ),
    );
    let request = NotifyRequest::new(
        "host_test.notice",
        wl_kernel::Subject::new("property.lease", 1),
    )
    .to_party(al_noor.party_id())
    .data(renox::serde_json::json!({ "number": "L-0001" }));
    let state = app.state();
    let ids = notify::notifier(state)
        .notify(state, request)
        .await
        .unwrap();
    let [MessageId(id)] = ids[..] else {
        panic!("one message: {ids:?}")
    };
    app.run_jobs().await;
    let message = wl_notify::Message::find(app.db(), id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(message.status, wl_notify::MessageStatus::Sent);
    app.assert_mail_sent("accounts@alnoortrading.example", "Notice L-0001");
}

#[test]
fn a_client_of_an_unbuilt_app_or_module_is_refused() {
    let demo = std::fs::read_to_string(host::DEMO_CLIENT).unwrap();
    let manpower =
        ClientConfig::parse(&demo.replace(r#"app     = "property""#, r#"app = "manpower""#))
            .unwrap();
    let err = host::app_for(manpower).err().unwrap();
    assert!(err.0.contains("app `manpower` is not built yet"), "{err}");

    let with_inventory = ClientConfig::parse(&demo.replace(
        r#"enabled = ["parties", "documents", "notify", "invoicing", "ticketing"]"#,
        r#"enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "inventory"]"#,
    ))
    .unwrap();
    let err = host::app_for(with_inventory).err().unwrap();
    assert!(err.0.contains("`inventory` is not built yet"), "{err}");
}

#[test]
fn an_unreadable_brand_colour_stops_the_boot() {
    let demo = std::fs::read_to_string(host::DEMO_CLIENT).unwrap();
    let pale = ClientConfig::parse(&demo.replace("#0F766E", "#FFF59D")).unwrap();
    let err = host::app_for(pale).err().unwrap();
    assert!(err.0.contains("the nearest colour that is"), "{err}");
}

#[renox::test]
async fn admins_find_the_legal_entities_in_the_sidebar() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let mut admin = User::register(app.db(), "Rana", "rana@example.com", "secret123")
        .await
        .unwrap();
    admin.email_verified_at = Some(renox::db::now());
    admin.save(app.db()).await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    admin
        .assign_role(app.db(), host::roles::ADMIN)
        .await
        .unwrap();
    app.acting_as(&admin);
    app.get("/legal-entities")
        .await
        .assert_ok()
        .assert_see("Palm Grove Facilities FZ-LLC")
        .assert_see(r#"class="rx-sidebar__link" href="/legal-entities" aria-current="page""#);
}
