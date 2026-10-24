//! The agent inbox (BL-AGT-09, #549): the `inbox` tool lists the same
//! approvals as the dashboard's "awaiting you" for the same user, never
//! another user's, and `mark_read` clears the bell's count.

mod support;

use renox::db::Json as DbJson;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::testing::{TestApp, TestResponse};
use wl_kernel::agent::AgentsOn;
use wl_kernel::approvals::{ApprovalPolicy, ApprovalRequest, Step};

use support::{noon_clock, person};

async fn booted_with_agents() -> TestApp {
    let policy = wl_kernel::agent::access::policy(AgentsOn::all());
    let app = TestApp::new(host::app().provide(noon_clock()).provide(policy)).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

async fn tool(app: &TestApp, name: &str, args: Value) -> TestResponse {
    app.request()
        .header("X-Agent-Client", "Claude")
        .json()
        .post_json(&format!("/agent/tools/{name}"), &args)
        .await
}

/// A pending request about invoice `subject_id` that any holder of `role`
/// may decide.
async fn pending_request(app: &TestApp, requester: &User, role: &str, subject_id: i64) -> i64 {
    let policy = ApprovalPolicy::create(
        app.db(),
        ApprovalPolicy {
            action: format!("invoicing.void_{role}"),
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
            subject_id,
            action: "invoicing.void".into(),
            transition: "void".into(),
            requester_id: requester.id,
            current_step: 1,
            steps: DbJson(vec![Step::one_of([role])]),
            sod: true,
            subject_state_before: "issued".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .id
}

/// A bell notification for `user`.
async fn notification(app: &TestApp, user: &User, kind: &str, title: &str) {
    renox::db::sql(
        "INSERT INTO notifications (user_id, kind, data, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(user.id)
    .bind(kind)
    .bind(json!({ "status": "info", "title": title, "url": "/dashboard" }).to_string())
    .bind(renox::db::now())
    .execute(app.db())
    .await
    .unwrap();
}

fn group<'a>(inbox: &'a Value, key: &str) -> &'a Vec<Value> {
    inbox["groups"]
        .as_array()
        .expect("groups")
        .iter()
        .find(|g| g["key"] == key)
        .unwrap_or_else(|| panic!("no group {key}"))["items"]
        .as_array()
        .expect("items")
}

#[renox::test]
async fn the_inbox_equals_the_dashboards_awaiting_you_for_the_same_user() {
    let app = booted_with_agents().await;
    let ana = person(&app, "Ana", host::roles::ADMIN).await;
    let clerk = person(&app, "Carl", host::roles::RECEIVABLES).await;
    let mine = pending_request(&app, &clerk, host::roles::ADMIN, 42).await;
    // A request only an owner decides: neither on Ana's dashboard nor in her
    // inbox.
    let other = pending_request(&app, &clerk, host::roles::OWNER, 43).await;

    let lang = app.state().lang("en");
    let expected: Vec<(String, Option<String>)> =
        wl_kernel::approvals::awaiting_items(app.db(), &ana, &lang)
            .await
            .unwrap()
            .into_iter()
            .map(|item| (item.title, item.url))
            .collect();
    assert_eq!(expected.len(), 1);

    app.acting_as(&ana);
    let inbox = tool(&app, "inbox", json!({})).await;
    inbox.assert_ok();
    let inbox: Value = inbox.json();
    let approvals = group(&inbox, "approvals");
    let listed: Vec<(String, Option<String>)> = approvals
        .iter()
        .map(|item| {
            (
                item["title"].as_str().unwrap().to_owned(),
                item["url"].as_str().map(str::to_owned),
            )
        })
        .collect();
    assert_eq!(listed, expected);
    assert_eq!(approvals[0]["id"], format!("approval:{mine}"));
    assert!(
        !approvals
            .iter()
            .any(|item| item["id"] == format!("approval:{other}"))
    );
    // The five groups, always.
    assert_eq!(inbox["groups"].as_array().unwrap().len(), 5);
    // The dashboard lists the same item for Ana.
    let dashboard = app.get("/dashboard").await;
    dashboard.assert_ok();
    for (title, _) in &listed {
        dashboard.assert_see(title);
    }

    // The requester sees none of it: the inbox lists only the user's own.
    app.acting_as(&clerk);
    let theirs: Value = tool(&app, "inbox", json!({})).await.json();
    assert!(group(&theirs, "approvals").is_empty());
}

#[renox::test]
async fn mark_read_clears_the_bell_count() {
    let app = booted_with_agents().await;
    let ana = person(&app, "Ana", host::roles::ADMIN).await;
    let carl = person(&app, "Carl", host::roles::RECEIVABLES).await;
    notification(&app, &ana, "wl-notification", "Invoice INV-0004 is overdue").await;
    notification(&app, &ana, "ticket.mention", "Carl mentioned you on ticket T-12").await;
    notification(&app, &carl, "wl-notification", "Not Ana's").await;
    assert_eq!(ana.unread_notification_count(app.db()).await.unwrap(), 2);

    app.acting_as(&ana);
    let inbox: Value = tool(&app, "inbox", json!({})).await.json();
    let notifications = group(&inbox, "notifications");
    let mentions = group(&inbox, "mentions");
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0]["title"], "Invoice INV-0004 is overdue");
    assert_eq!(mentions.len(), 1);
    assert!(
        !notifications
            .iter()
            .chain(mentions)
            .any(|n| n["title"] == "Not Ana's")
    );
    let ids: Vec<Value> = notifications
        .iter()
        .chain(mentions)
        .map(|n| n["id"].clone())
        .collect();

    let marked = tool(&app, "mark_read", json!({ "ids": ids })).await;
    marked.assert_ok();
    let marked: Value = marked.json();
    assert_eq!(marked["marked"], 2);
    assert_eq!(marked["unread"], 0);
    assert_eq!(ana.unread_notification_count(app.db()).await.unwrap(), 0);
    // Carl's notification is untouched.
    assert_eq!(carl.unread_notification_count(app.db()).await.unwrap(), 1);
    let after: Value = tool(&app, "inbox", json!({})).await.json();
    assert!(group(&after, "notifications").is_empty());
    assert!(group(&after, "mentions").is_empty());
}
