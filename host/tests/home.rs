use renox::prelude::*;
use renox::testing::TestApp;
use std::time::Duration;

// The app's own items in a group of their own: rustfmt sorts each group, so
// this order holds whatever the app is called.
use host::roles::{self, ADMIN, MEMBER};

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
async fn the_front_page_sends_guests_to_the_login_and_people_to_the_dashboard() {
    let app = TestApp::new(host::app()).await;
    // No marketing page of the starter kit, no sign-up.
    app.get("/").await.assert_redirect("/login");
    let anna = person(&app, "anna@example.com", MEMBER).await;
    app.acting_as(&anna);
    app.get("/").await.assert_redirect("/dashboard");
}

#[renox::test]
async fn pages_that_are_not_there_are_in_the_layout() {
    let app = TestApp::new(host::app()).await;
    app.get("/nowhere")
        .await
        .assert_not_found()
        .assert_see("Go to the home page");
    let anna = person(&app, "anna@example.com", MEMBER).await;
    app.acting_as(&anna);
    app.get("/nowhere")
        .await
        .assert_not_found()
        // The dashboard standard: the shell fills the screen on a large one.
        .assert_see("rx-shell rx-shell--fill");
}

#[renox::test]
async fn the_login_page_lists_the_seeded_people_in_local_only() {
    // TestApp runs with APP_ENV=testing: no list, even after seeding.
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    app.get("/login")
        .await
        .assert_ok()
        .assert_dont_see("admin@example.com");

    let local = TestApp::with_config(host::app(), |config| {
        config.env = renox::Environment::Local;
    })
    .await;
    // Not before `db:seed` made them.
    local
        .get("/login")
        .await
        .assert_dont_see("admin@example.com");
    local.kernel().seed().await.unwrap();
    local
        .get("/login")
        .await
        .assert_see("Seeded accounts")
        .assert_see("data-demo-email=\"admin@example.com\"")
        .assert_see("member@example.com");
    // Only on the login page.
    local
        .get("/password/forgot")
        .await
        .assert_dont_see("admin@example.com");
}

#[renox::test]
async fn nobody_signs_up_on_their_own() {
    let app = TestApp::new(host::app()).await;
    app.get("/register").await.assert_not_found();
    let answer = app
        .post(
            "/register",
            &[
                ("name", "Anna"),
                ("email", "anna@example.com"),
                ("password", "secret123"),
                ("password_confirmation", "secret123"),
            ],
        )
        .await;
    assert!(answer.status.is_client_error(), "{}", answer.status);
    assert!(
        User::find_by_email(app.db(), "anna@example.com")
            .await
            .unwrap()
            .is_none()
    );
    // And no page offers it.
    app.get("/login")
        .await
        .assert_ok()
        .assert_dont_see("/register");
    app.get("/nowhere").await.assert_dont_see("/register");
}

#[renox::test]
async fn the_dashboard_is_for_verified_users() {
    let app = TestApp::new(host::app()).await;
    app.get("/dashboard").await.assert_redirect("/login");
    let anna = person(&app, "anna@example.com", MEMBER).await;
    app.acting_as(&anna);
    app.get("/dashboard")
        .await
        .assert_ok()
        .assert_see("Welcome back,")
        .assert_see("<em>Anna.</em>")
        .assert_see("Your recent activity")
        // The activity is the pane that takes the height left.
        .assert_see("rx-pane rx-fill");
}

#[renox::test]
async fn only_admins_manage_users_and_read_the_activity_log() {
    let app = TestApp::new(host::app()).await;
    let member = person(&app, "member@example.com", MEMBER).await;
    app.acting_as(&member);
    app.get("/users").await.assert_forbidden();
    app.get("/activity").await.assert_forbidden();
    // The sidebar menu (modules add their links, see `app.nav`) shows only
    // what this person may open.
    app.get("/dashboard")
        .await
        .assert_ok()
        .assert_see(r#"class="rx-sidebar__link" href="/dashboard" aria-current="page""#)
        .assert_see(r#"class="rx-sidebar__link" href="/account""#)
        .assert_dont_see(r#"class="rx-sidebar__link" href="/users""#)
        .assert_dont_see(r#"class="rx-sidebar__link" href="/activity""#);

    let admin = person(&app, "admin@example.com", ADMIN).await;
    app.acting_as(&admin);
    app.get("/users")
        .await
        .assert_ok()
        .assert_see("member@example.com")
        .assert_see(r#"class="rx-sidebar__link" href="/users" aria-current="page""#)
        .assert_see(r#"class="rx-sidebar__link" href="/activity""#);
    app.htmx()
        .put(
            &format!("/users/{}/roles", member.id),
            &[("roles", "admin"), ("roles", "member")],
        )
        .await
        .assert_ok();
    assert_eq!(member.roles(app.db()).await.unwrap(), ["admin", "member"]);
    // Recorded, and shown on the activity page.
    app.get("/activity")
        .await
        .assert_ok()
        .assert_see("user.roles_changed");
}

#[renox::test]
async fn the_first_admin_is_made_with_a_command() {
    let app = TestApp::new(host::app()).await;
    let anna = User::register(app.db(), "Anna", "anna@example.com", "secret123")
        .await
        .unwrap();
    app.kernel()
        .call("users:admin", ["anna@example.com"])
        .await
        .unwrap();
    assert_eq!(anna.roles(app.db()).await.unwrap(), [ADMIN]);
    assert!(
        app.kernel()
            .call("users:admin", ["nobody@example.com"])
            .await
            .is_err()
    );
}

#[renox::test]
async fn admins_keep_their_own_admin_role() {
    let app = TestApp::new(host::app()).await;
    let admin = person(&app, "admin@example.com", ADMIN).await;
    app.acting_as(&admin);
    app.htmx()
        .put(
            &format!("/users/{}/roles", admin.id),
            &[("roles", "member")],
        )
        .await
        .assert_invalid("roles");
}

#[renox::test]
async fn sessions_end_after_their_lifetime() {
    let app = TestApp::new(host::app()).await;
    let anna = person(&app, "anna@example.com", MEMBER).await;
    app.acting_as(&anna);
    app.get("/account").await.assert_ok();
    // Time moves for the app (renox::db::now(), sessions, the queue), not for the test.
    app.travel(Duration::from_secs(3 * 60 * 60)); // past SESSION_LIFETIME's 120 minutes
    app.get("/account").await.assert_redirect("/login");
}

#[renox::test]
async fn the_models_match_the_schema() {
    TestApp::new(host::app())
        .await
        .assert_models_match_schema()
        .await;
}

#[renox::test]
async fn every_view_compiles() {
    let app = TestApp::new(host::app()).await;
    app.assert_views_compile();
}

#[renox::test]
async fn members_can_log_in_with_a_link_sent_by_mail() {
    let app = TestApp::new(host::app()).await;
    let anna = person(&app, "anna@example.com", MEMBER).await;
    app.get("/login")
        .await
        .assert_ok()
        .assert_see("Email me a login link");
    app.post("/login/link", &[("email", "anna@example.com")])
        .await
        .assert_redirect("/login/link");
    let link = app.last_login_link().await.expect("a link was mailed");
    let path = &link[link.find("/login/link/").unwrap()..];
    // Opening the link logs nobody in; its button does.
    app.get(path)
        .await
        .assert_ok()
        .assert_see("anna@example.com");
    app.assert_guest();
    app.post(path, &[]).await.assert_redirect("/dashboard");
    app.assert_authenticated(Some(&anna));
}

/// A request returned to `requester` about an invoice, with its open
/// "fix and resubmit" task.
async fn returned_invoice_request(app: &TestApp, requester: &User) -> i64 {
    use wl_kernel::approvals::{ApprovalPolicy, ApprovalRequest, ApprovalStatus, ResubmitTask};

    let policy = ApprovalPolicy::create(
        app.db(),
        ApprovalPolicy {
            action: "invoicing.void".into(),
            active: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let request = ApprovalRequest::create(
        app.db(),
        ApprovalRequest {
            policy_id: policy.id,
            subject_type: "invoicing.invoice".into(),
            subject_id: 42,
            action: "invoicing.void".into(),
            transition: "void".into(),
            requester_id: requester.id,
            current_step: 1,
            subject_state_before: "issued".into(),
            decided_at: Some(renox::db::now()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // A new request starts `pending`; the return is the state it ended in.
    renox::db::sql("UPDATE wl_approval_requests SET status = ? WHERE id = ?")
        .bind(ApprovalStatus::Returned)
        .bind(request.id)
        .execute(app.db())
        .await
        .unwrap();
    ResubmitTask::create(
        app.db(),
        ResubmitTask {
            request_id: request.id,
            requester_id: requester.id,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    request.id
}

/// A pending request about an invoice that any `admin` may decide.
async fn pending_invoice_request(app: &TestApp, requester: &User) -> i64 {
    use renox::db::Json;
    use wl_kernel::approvals::{ApprovalPolicy, ApprovalRequest, Step};

    let policy = ApprovalPolicy::create(
        app.db(),
        ApprovalPolicy {
            action: "invoicing.void".into(),
            active: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    ApprovalRequest::create(
        app.db(),
        ApprovalRequest {
            policy_id: policy.id,
            subject_type: "invoicing.invoice".into(),
            subject_id: 42,
            action: "invoicing.void".into(),
            transition: "void".into(),
            requester_id: requester.id,
            current_step: 1,
            steps: Json(vec![Step::one_of([ADMIN])]),
            sod: true,
            subject_state_before: "issued".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .id
}

#[renox::test]
async fn the_dashboard_counts_the_approvals_waiting_for_the_person_who_may_decide_them() {
    let app = TestApp::new(host::app()).await;
    let anna = person(&app, "anna@example.com", MEMBER).await;
    let bilal = person(&app, "bilal@example.com", ADMIN).await;
    pending_invoice_request(&app, &anna).await;

    app.acting_as(&bilal);
    app.get("/dashboard")
        .await
        .assert_ok()
        .assert_see("Needs your action")
        .assert_see("1 approval waiting for you")
        .assert_see("Decide: invoicing.void")
        .assert_see(r#"href="/invoices/42""#);

    // The requester decides nothing of their own.
    app.acting_as(&anna);
    app.get("/dashboard")
        .await
        .assert_ok()
        .assert_dont_see("waiting for you");
}

#[renox::test]
async fn the_dashboard_lists_the_returned_requests_of_the_person_who_must_fix_them() {
    let app = TestApp::new(host::app()).await;
    let anna = person(&app, "anna@example.com", MEMBER).await;
    let bilal = person(&app, "bilal@example.com", MEMBER).await;
    app.acting_as(&anna);
    app.get("/dashboard")
        .await
        .assert_ok()
        .assert_dont_see("Fix and resubmit");

    returned_invoice_request(&app, &anna).await;
    app.get("/dashboard")
        .await
        .assert_ok()
        .assert_see("Needs your action")
        .assert_see("Fix and resubmit: invoicing.void")
        .assert_see(r#"href="/invoices/42""#);

    app.acting_as(&bilal);
    app.get("/dashboard")
        .await
        .assert_ok()
        .assert_dont_see("Needs your action")
        .assert_dont_see("Fix and resubmit");
}

/// `deployment.activate` of the override acceptance check (#50 AC1): the
/// deployment's worker.
struct Deployments;

impl wl_kernel::documents::BlockSubjects for Deployments {
    fn subjects<'a>(
        &'a self,
        _state: &'a renox::AppState,
        _entry: &'a wl_kernel::documents::BlockEntry,
        record_id: i64,
    ) -> wl_kernel::documents::DocumentFuture<'a, Vec<wl_kernel::Subject>> {
        Box::pin(async move { Ok(vec![wl_kernel::Subject::new("hr.employee", record_id)]) })
    }
}

#[renox::test]
async fn the_dashboard_lists_an_override_request_for_the_admin_who_may_approve_it() {
    use wl_documents::{Document, DocumentKind, DocumentStatus, Documents, KindSeed};
    use wl_kernel::documents::{BlockMode, register_block_subjects};

    register_block_subjects("deployment.activate", Deployments);
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let anna = person(&app, "anna@example.com", MEMBER).await;
    let bilal = person(&app, "bilal@example.com", ADMIN).await;
    let seed = KindSeed::new("health_card", "Health card")
        .subject_type("hr.employee")
        .expires()
        .blocks("deployment.activate", BlockMode::Block);
    wl_documents::kinds::install_seeds(app.db(), [&seed])
        .await
        .unwrap();
    let kind = DocumentKind::by_key(app.db(), "health_card")
        .await
        .unwrap()
        .unwrap();
    Document::create(
        app.db(),
        Document {
            subject_type: "hr.employee".into(),
            subject_id: 7,
            kind_id: kind.id,
            title: "Health card".into(),
            expiry_date: Some(wl_kernel::documents::today() - renox::chrono::Days::new(3)),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let rows = Documents::request_override(
        app.state(),
        &anna,
        "health_card",
        "deployment.activate",
        7,
        "renewal booked 2026-12-03",
    )
    .await
    .unwrap();

    app.acting_as(&bilal);
    app.get("/dashboard")
        .await
        .assert_ok()
        .assert_see("Needs your action")
        .assert_see("1 approval waiting for you")
        .assert_see(&format!(r#"href="/documents/overrides/{}""#, rows[0].id));

    app.acting_as(&anna);
    app.get("/dashboard")
        .await
        .assert_ok()
        .assert_dont_see("waiting for you");
}
