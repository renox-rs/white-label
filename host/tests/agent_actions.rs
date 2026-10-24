//! Agent actions (BL-AGT-08, #548): an assistant previews issuing a draft
//! invoice (nothing written; the number to be assigned, the total and the
//! VAT), confirms it once (a second confirm returns the same invoice), an
//! issue over the user's limit becomes an approval request, and the history
//! shows "via Claude". Driven through `POST /agent/tools/{tool}`, as the MCP
//! connector calls the same tools.

mod support;

use renox::db::Json as DbJson;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::testing::{TestApp, TestResponse};
use wl_invoicing::Invoice;
use wl_kernel::agent::{AgentRun, AgentsOn};
use wl_kernel::approvals::{ApprovalPolicy, ApprovalStatus, ConditionKind, Step, pending_for};
use wl_kernel::invoicing::{InvoiceStatus, VatCategory};
use wl_kernel::Subject;

use support::{day, draft_invoice, noon_clock, person, reload, tenant};

/// The Property host with AI assistants turned on for every type (the
/// BL-AGT-06 settings page is not built: the policy is provided).
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

/// Calls agent tool `name` as Claude for the signed-in user.
async fn tool(app: &TestApp, name: &str, args: Value) -> TestResponse {
    app.request()
        .header("X-Agent-Client", "Claude")
        .json()
        .post_json(&format!("/agent/tools/{name}"), &args)
        .await
}

/// A draft invoice of AED 1,000.00 plus 5% VAT to a tenant.
async fn draft(app: &TestApp) -> Invoice {
    let party = tenant(app, "Al Ali").await;
    draft_invoice(
        app,
        party,
        Subject::new("property.lease", 9),
        100_000,
        VatCategory::Standard,
        day(30),
    )
    .await
}

async fn count(app: &TestApp, statement: &str) -> i64 {
    renox::db::sql(statement)
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

/// What preview could have written: history rows, approval requests and
/// agent runs.
async fn written(app: &TestApp) -> (i64, i64, i64) {
    (
        count(app, "SELECT COUNT(*) FROM state_transitions").await,
        count(app, "SELECT COUNT(*) FROM wl_approval_requests").await,
        count(app, "SELECT COUNT(*) FROM wl_agent_runs").await,
    )
}

fn amount(preview: &Value, label: &str) -> i64 {
    preview["change"]["amounts"]
        .as_array()
        .expect("amounts")
        .iter()
        .find(|a| a["label"] == label)
        .unwrap_or_else(|| panic!("no amount {label}"))["minor"]
        .as_i64()
        .expect("minor units")
}

fn line<'a>(preview: &'a Value, label: &str) -> &'a str {
    preview["change"]["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .find(|l| l["label"] == label)
        .unwrap_or_else(|| panic!("no line {label}"))["value"]
        .as_str()
        .expect("text")
}

async fn preview_issue(app: &TestApp, invoice: &Invoice) -> Value {
    let response = tool(
        app,
        "preview_action",
        json!({ "action": "invoicing.invoice.issue", "record": format!("invoicing.invoice:{}", invoice.id) }),
    )
    .await;
    response.assert_ok();
    response.json()
}

#[renox::test]
async fn preview_of_issue_writes_nothing_and_states_the_number_total_and_vat() {
    let app = booted_with_agents().await;
    let budi = person(&app, "Budi", host::roles::RECEIVABLES).await;
    let invoice = draft(&app).await;
    let before = written(&app).await;
    app.acting_as(&budi);

    let preview = preview_issue(&app, &invoice).await;

    assert!(line(&preview, "Number").starts_with("Assigned when issued"));
    assert_eq!(amount(&preview, "Total"), invoice.gross_total);
    assert_eq!(amount(&preview, "VAT"), invoice.vat_total);
    assert_eq!(amount(&preview, "VAT"), 5_000);
    assert_eq!(preview["change"]["amounts"][0]["currency"], "AED");
    assert_eq!(preview["requires_approval"], false);
    assert!(preview["token"].as_str().is_some_and(|t| !t.is_empty()));
    // Nothing was written.
    assert_eq!(written(&app).await, before);
    let stored = reload(&app, &invoice).await;
    assert_eq!(stored.status, InvoiceStatus::Draft);
    assert_eq!(stored.number, None);
}

#[renox::test]
async fn confirm_issues_once_and_a_second_confirm_returns_the_same_invoice() {
    let app = booted_with_agents().await;
    let budi = person(&app, "Budi", host::roles::RECEIVABLES).await;
    let invoice = draft(&app).await;
    app.acting_as(&budi);
    let preview = preview_issue(&app, &invoice).await;
    let token = preview["token"].as_str().unwrap().to_owned();

    let first = tool(&app, "confirm_action", json!({ "token": token })).await;
    first.assert_ok();
    let first: Value = first.json();
    assert_eq!(first["outcome"], "done");
    assert_eq!(first["record"], format!("invoicing.invoice:{}", invoice.id));
    let issued = reload(&app, &invoice).await;
    assert_eq!(issued.status, InvoiceStatus::Issued);
    let number = issued.number.clone().expect("a number");
    assert_eq!(first["data"]["number"], number.as_str());

    let second = tool(&app, "confirm_action", json!({ "token": token })).await;
    second.assert_ok();
    let second: Value = second.json();
    assert_eq!(second, first);
    assert_eq!(reload(&app, &invoice).await.number, Some(number));
    let issues = renox::db::sql(
        "SELECT COUNT(*) FROM state_transitions WHERE model = ? AND record_key = ? AND transition = 'issue'",
    )
    .bind(Invoice::TABLE)
    .bind(invoice.id.to_string())
    .scalar::<i64>(app.db())
    .await
    .unwrap();
    assert_eq!(issues, 1);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM wl_agent_runs").await, 1);
    let runs = AgentRun::about(app.db(), &Subject::new("invoicing.invoice", invoice.id))
        .await
        .unwrap();
    assert_eq!(runs[0].client, "Claude");
    assert_eq!(runs[0].user_id, budi.id);
}

#[renox::test]
async fn an_issue_over_the_users_limit_creates_an_approval_request() {
    let app = booted_with_agents().await;
    let budi = person(&app, "Budi", host::roles::RECEIVABLES).await;
    // The company's limit: an issue above AED 500.00 is approved by an admin.
    ApprovalPolicy::create(
        app.db(),
        ApprovalPolicy {
            action: "invoicing.issue".into(),
            condition: ConditionKind::AmountAbove,
            threshold: 50_000,
            steps: DbJson(vec![Step::one_of([host::roles::ADMIN])]),
            sod: true,
            active: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let invoice = draft(&app).await;
    app.acting_as(&budi);
    let preview = preview_issue(&app, &invoice).await;
    assert_eq!(preview["requires_approval"], true);

    let confirmed = tool(
        &app,
        "confirm_action",
        json!({ "token": preview["token"].as_str().unwrap() }),
    )
    .await;
    confirmed.assert_ok();
    let confirmed: Value = confirmed.json();
    assert_eq!(confirmed["outcome"], "approval_requested");
    let stored = reload(&app, &invoice).await;
    assert_eq!(stored.status, InvoiceStatus::Draft);
    assert_eq!(stored.number, None);
    let pending = pending_for(app.db(), &Subject::new("invoicing.invoice", invoice.id))
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].status, ApprovalStatus::Pending);
    assert_eq!(pending[0].requester_id, budi.id);
}

#[renox::test]
async fn the_history_shows_the_issue_was_made_via_claude() {
    let app = booted_with_agents().await;
    let budi = person(&app, "Budi", host::roles::RECEIVABLES).await;
    let invoice = draft(&app).await;
    app.acting_as(&budi);
    let preview = preview_issue(&app, &invoice).await;
    tool(
        &app,
        "confirm_action",
        json!({ "token": preview["token"].as_str().unwrap() }),
    )
    .await
    .assert_ok();

    let history = renox::state_machine::history(app.db(), Invoice::TABLE, invoice.id)
        .await
        .unwrap();
    let issue = history
        .iter()
        .find(|row| row.transition == "issue")
        .expect("the issue row");
    assert_eq!(issue.actor_id, Some(budi.id));
    assert_eq!(wl_kernel::agent::history::via(issue), Some("Claude"));
    let items = wl_kernel::agent::history::timeline(app.db(), &history, None)
        .await
        .unwrap();
    assert!(
        items
            .iter()
            .any(|item| item["body"].as_str() == Some("by Budi via Claude"))
    );
    app.get(&format!("/invoices/{}", invoice.id))
        .await
        .assert_ok()
        .assert_see("by Budi via Claude");
}

#[renox::test]
async fn the_record_moved_since_the_preview_is_refused_with_preview_again() {
    let app = booted_with_agents().await;
    let budi = person(&app, "Budi", host::roles::RECEIVABLES).await;
    let invoice = draft(&app).await;
    app.acting_as(&budi);
    let preview = preview_issue(&app, &invoice).await;
    // Someone changes the draft's due date on the screen meanwhile.
    Invoice::query()
        .where_(Invoice::ID.eq(invoice.id))
        .update(app.db(), &[("due_date", &day(45))])
        .await
        .unwrap();

    let refused = tool(
        &app,
        "confirm_action",
        json!({ "token": preview["token"].as_str().unwrap() }),
    )
    .await;
    refused.assert_status(409);
    assert!(refused.text().contains("preview the action again"));
    assert_eq!(reload(&app, &invoice).await.status, InvoiceStatus::Draft);
}

#[renox::test]
async fn approvals_and_voids_are_not_offered_and_a_user_without_the_right_is_refused() {
    let app = booted_with_agents().await;
    let viewer = person(&app, "Vera", host::roles::MEMBER).await;
    let invoice = draft(&app).await;
    app.acting_as(&viewer);
    let listed = tool(&app, "list_actions", json!({})).await;
    listed.assert_ok();
    let listed: Value = listed.json();
    let keys: Vec<&str> = listed["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["key"].as_str())
        .collect();
    assert!(keys.contains(&"invoicing.invoice.issue"));
    for never in ["approve", "approve_step", "reject", "void"] {
        assert!(!keys.iter().any(|k| k.ends_with(&format!(".{never}"))), "{never}");
    }
    tool(
        &app,
        "preview_action",
        json!({ "action": "invoicing.invoice.issue", "record": invoice.id }),
    )
    .await
    .assert_status(403);
}

#[renox::test]
async fn with_no_policy_every_agent_call_is_refused() {
    let app = support::booted().await;
    let budi = person(&app, "Budi", host::roles::RECEIVABLES).await;
    app.acting_as(&budi);
    let refused = tool(&app, "inbox", json!({})).await;
    refused.assert_status(403);
    assert!(
        refused
            .text()
            .contains(wl_kernel::agent::access::TURNED_OFF)
    );
}
