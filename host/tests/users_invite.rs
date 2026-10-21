//! Nobody signs up on their own; an admin adds people, and a failing mail
//! leaves a whole user.

use renox::prelude::*;
use renox::testing::TestApp;

use host::roles::{self, ADMIN, MEMBER};

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
async fn an_admin_adds_a_user_who_chooses_a_password_from_a_mailed_link() {
    let app = TestApp::new(host::app()).await;
    let admin = person(&app, "admin@example.com", ADMIN).await;
    app.acting_as(&admin);
    app.htmx()
        .post(
            "/users",
            &[
                ("name", "Bilal"),
                ("email", "Bilal@Example.com"),
                ("roles", "accountant"),
            ],
        )
        .await
        .assert_ok();
    let bilal = User::find_by_email(app.db(), "bilal@example.com")
        .await
        .unwrap()
        .unwrap();
    assert!(bilal.email_verified_at.is_some());
    assert_eq!(bilal.roles(app.db()).await.unwrap(), ["accountant"]);
    app.assert_mail_sent("bilal@example.com", "You are invited");
    app.get("/activity").await.assert_see("user.invited");

    // The link in the mail opens the page where the password is chosen.
    let mail = app.sent_mail().pop().unwrap();
    let start = mail.text.find("http").unwrap();
    let link = mail.text[start..].lines().next().unwrap().trim().to_owned();
    let path = &link[link.find("/reset-password/").unwrap()..];
    app.post("/logout", &[]).await;
    app.get(path).await.assert_ok();

    // The same address twice, an unknown role and a member (no admin) are refused.
    app.acting_as(&admin);
    app.htmx()
        .post(
            "/users",
            &[("name", "Bilal"), ("email", "bilal@example.com")],
        )
        .await
        .assert_invalid("email");
    let member = person(&app, "member@example.com", MEMBER).await;
    app.acting_as(&member);
    app.htmx()
        .post("/users", &[("name", "Eve"), ("email", "eve@example.com")])
        .await
        .assert_forbidden();
    assert!(
        User::find_by_email(app.db(), "eve@example.com")
            .await
            .unwrap()
            .is_none()
    );
}

#[renox::test]
async fn a_mail_that_fails_leaves_a_whole_user_and_no_error_page() {
    let app = TestApp::with_config(host::app(), |config| {
        config.mail.mailer = renox::mail::MailDriver::Smtp;
        config.mail.host = "mail.invalid".into();
        config.mail.timeout = std::time::Duration::from_secs(2);
    })
    .await;
    let admin = person(&app, "admin@example.com", ADMIN).await;
    app.acting_as(&admin);
    // The request succeeds and the user is there, verified, with the role.
    app.htmx()
        .post(
            "/users",
            &[("name", "Bilal"), ("email", "bilal@example.com")],
        )
        .await
        .assert_ok();
    let bilal = User::find_by_email(app.db(), "bilal@example.com")
        .await
        .unwrap()
        .unwrap();
    assert!(bilal.email_verified_at.is_some());
    assert_eq!(bilal.roles(app.db()).await.unwrap(), ["member"]);
    // The mail was tried and failed; the log says so.
    let logged: String =
        renox::db::sql("select data from audit_logs where action = 'user.invited'")
            .scalar(app.db())
            .await
            .unwrap();
    assert!(logged.contains("\"mailed\":false"), "{logged}");
    // The command prints the link, so the first admin does not need the mail.
    app.kernel()
        .call(
            "users:invite",
            ["carla@example.com", "Carla", "--role=admin"],
        )
        .await
        .unwrap();
    let carla = User::find_by_email(app.db(), "carla@example.com")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(carla.roles(app.db()).await.unwrap(), [ADMIN]);
    assert!(
        app.kernel()
            .call("users:invite", ["dan@example.com", "Dan", "--role=nothing"])
            .await
            .is_err()
    );
}
