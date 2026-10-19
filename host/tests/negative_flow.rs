//! BL-PRP-26 (#276): the shared money settings of the Property client are in
//! force and the settings page shows them to an admin.

use host::roles::{self, ADMIN, MEMBER};
use renox::prelude::*;
use renox::testing::TestApp;

/// A verified user with `role`.
async fn person(app: &TestApp, email: &str, role: &str) -> User {
    let mut user = User::register(app.db(), "Anna", email, "secret123")
        .await
        .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    roles::define(app.db()).await.unwrap();
    user.assign_role(app.db(), role).await.unwrap();
    user
}

#[renox::test]
async fn an_admin_sees_the_money_settings_of_the_property_client() {
    let app = TestApp::new(host::app()).await;
    let admin = person(&app, "admin@example.com", ADMIN).await;
    app.acting_as(&admin);
    app.get("/settings/negative-flow")
        .await
        .assert_ok()
        .assert_see("Money settings")
        .assert_see("Maximum presentations of one cheque")
        .assert_see("none until the client sets one")
        .assert_see("Duplicate check keys")
        .assert_see("PGP AED 50,000.00");
}

#[renox::test]
async fn a_member_does_not_see_the_money_settings() {
    let app = TestApp::new(host::app()).await;
    let member = person(&app, "member@example.com", MEMBER).await;
    app.acting_as(&member);
    app.get("/settings/negative-flow").await.assert_forbidden();
}
