//! The demo commands (`migrate:fresh`, `db:seed`, `property:demo`) are
//! refused in production.

use renox::prelude::*;
use renox::testing::TestApp;

#[test]
fn the_demo_commands_are_refused_in_production() {
    for command in ["migrate:fresh", "db:seed", "property:demo"] {
        for env in ["production", "Production", "prod"] {
            let refusal = host::refuse_in_production(Some(command), Some(env)).unwrap_err();
            assert!(refusal.contains(command) && refusal.contains("production"));
        }
        // Local, testing and unset are fine.
        for env in [Some("local"), Some("testing"), None] {
            assert!(host::refuse_in_production(Some(command), env).is_ok());
        }
    }
    // Other commands run in production.
    for command in [
        "serve",
        "migrate",
        "users:invite",
        "wl:install",
        "queue:work",
    ] {
        assert!(host::refuse_in_production(Some(command), Some("production")).is_ok());
    }
    assert!(host::refuse_in_production(None, Some("production")).is_ok());
}

#[renox::test]
async fn the_seeders_and_the_demo_command_refuse_a_production_database() {
    let app = TestApp::with_config(host::app(), |config| {
        config.env = renox::Environment::Production;
    })
    .await;
    assert!(app.kernel().seed().await.is_err());
    assert!(
        User::find_by_email(app.db(), "admin@example.com")
            .await
            .unwrap()
            .is_none()
    );
    let error = app
        .kernel()
        .call("property:demo", Vec::<String>::new())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("local or testing"), "{error}");
}
