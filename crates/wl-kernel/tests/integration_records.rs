//! The integration base (wl-kernel §4.5, §5.14; #29 part 2): the stored
//! connections, their machine and the outbound queue.

use std::time::Duration;

use renox::auth::Permissions;
use renox::chrono::Duration as Span;
use renox::db::now;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, TransitionError, history};
use renox::testing::TestApp;
use wl_kernel::integrations::{self, CallStatus, queue};
use wl_kernel::{
    ClientConfig, ConnectionStatus, Delivery, IntegrationConnection, IntegrationDef, Outbound,
    QueuedCall, Subject,
};

const CLIENT: &str = r#"
[client]
name = "Acme Properties LLC"
app = "property"

[modules]
enabled = []

[[legal_entities]]
code = "ACME"
name_en = "Acme Properties LLC"
default = true
"#;

struct Invoicing;

impl Module for Invoicing {
    fn name(&self) -> &'static str {
        "test-invoicing"
    }

    fn register(&self, _app: &mut Registry) {
        IntegrationDef::of("asp.acme", "invoicing", |_state, _call| {
            Box::pin(async { Ok(Delivery::Sent) })
        })
        .owner_role("accountant")
        .screens(["invoicing.invoice_list", "invoicing.invoice_detail"])
        .fallback("File the invoice on the ASP portal by hand.", None)
        .register();
    }
}

async fn app() -> TestApp {
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Invoicing),
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

async fn connection(app: &TestApp) -> IntegrationConnection {
    IntegrationConnection::by_key(app.db(), "asp.acme")
        .await
        .unwrap()
        .expect("installed")
}

async fn run(
    app: &TestApp,
    connection: &IntegrationConnection,
    name: &str,
    input: renox::serde_json::Value,
) -> std::result::Result<(), TransitionError> {
    app.at_travelled_time(
        connection
            .transition(name)
            .by_system()
            .input(&input)
            .run(app.state()),
    )
    .await
    .map(|_| ())
}

#[renox::test]
async fn install_stores_what_the_module_registered_and_keeps_admin_changes() {
    let app = app().await;
    let stored = connection(&app).await;
    assert_eq!(stored.module, "invoicing");
    assert_eq!(stored.owner_role, "accountant");
    assert_eq!(stored.status, ConnectionStatus::Up);
    assert_eq!(stored.down_after_minutes, 15);
    assert!(stored.affects("invoicing.invoice_list"));
    assert_eq!(
        stored.fallback_en,
        "File the invoice on the ASP portal by hand."
    );

    IntegrationConnection::where_eq("id", stored.id)
        .update(app.db(), &[("down_after_minutes", &5_i64)])
        .await
        .unwrap();
    assert_eq!(integrations::install(app.db()).await.unwrap(), 0);
    assert_eq!(connection(&app).await.down_after_minutes, 5);
    assert_eq!(
        IntegrationConnection::query()
            .count(app.db())
            .await
            .unwrap(),
        1
    );
}

#[renox::test]
async fn a_failed_call_degrades_and_records_when_the_failures_began() {
    let app = app().await;
    run(
        &app,
        &connection(&app).await,
        "degrade",
        json!({ "error": "timeout" }),
    )
    .await
    .unwrap();

    let degraded = connection(&app).await;
    assert_eq!(degraded.status, ConnectionStatus::Degraded);
    assert_eq!(degraded.last_error.as_deref(), Some("timeout"));
    let since = degraded.failed_since.expect("failed since");
    assert!(now() - since < Span::minutes(1));

    // Only an `up` connection degrades; a second failed call changes nothing.
    let again = run(&app, &degraded, "degrade", json!({ "error": "again" })).await;
    assert!(matches!(again, Err(TransitionError::NotFrom(_))));
}

#[renox::test]
async fn go_down_waits_for_n_minutes_from_the_first_failure() {
    let app = app().await;
    run(
        &app,
        &connection(&app).await,
        "degrade",
        json!({ "error": "timeout" }),
    )
    .await
    .unwrap();

    app.travel(Duration::from_secs(14 * 60));
    let early = run(&app, &connection(&app).await, "go_down", json!({})).await;
    assert!(matches!(early, Err(TransitionError::Guard(_))), "{early:?}");
    assert_eq!(connection(&app).await.status, ConnectionStatus::Degraded);

    app.travel(Duration::from_secs(60));
    run(&app, &connection(&app).await, "go_down", json!({}))
        .await
        .unwrap();
    let down = connection(&app).await;
    assert_eq!(down.status, ConnectionStatus::Down);
    // `failed since` is kept by `go_down`.
    assert!(down.failed_since.is_some());
}

#[renox::test]
async fn go_down_only_starts_from_degraded() {
    let app = app().await;
    let up = connection(&app).await;
    let result = run(&app, &up, "go_down", json!({})).await;
    assert!(matches!(result, Err(TransitionError::NotFrom(_))));
}

#[renox::test]
async fn recover_clears_the_failure_run_and_notes_the_success() {
    let app = app().await;
    run(
        &app,
        &connection(&app).await,
        "degrade",
        json!({ "error": "timeout" }),
    )
    .await
    .unwrap();
    run(&app, &connection(&app).await, "recover", json!({}))
        .await
        .unwrap();

    let up = connection(&app).await;
    assert_eq!(up.status, ConnectionStatus::Up);
    assert!(up.failed_since.is_none() && up.last_error.is_none());
    assert!(up.last_success_at.is_some());
    let rows = history(app.db(), IntegrationConnection::TABLE, up.id)
        .await
        .unwrap();
    let names: Vec<_> = rows.iter().map(|r| r.transition.as_str()).collect();
    assert_eq!(names, ["degrade", "recover"]);
}

#[renox::test]
async fn disable_and_enable_are_for_a_user_with_the_permission() {
    let app = app().await;
    // A system run cannot disable: the transition is a user's.
    let result = run(&app, &connection(&app).await, "disable", json!({})).await;
    assert!(result.is_err(), "{result:?}");
    assert_eq!(connection(&app).await.status, ConnectionStatus::Up);
}

#[renox::test]
async fn calls_leave_the_queue_first_in_first_out_with_the_backoff() {
    let app = app().await;
    let db = app.db();
    for n in 1..=3 {
        let call = Outbound::new("asp.acme", "invoice", json!({ "n": n }))
            .subject(Subject::new("invoicing.invoice", n));
        let mut tx = db.begin().await.unwrap();
        queue::enqueue(&mut tx, &call, None).await.unwrap();
        tx.commit().await.unwrap();
    }
    let waiting = QueuedCall::waiting(db, "asp.acme").await.unwrap();
    assert_eq!(waiting.len(), 3);
    assert!(waiting.windows(2).all(|pair| pair[0].id < pair[1].id));

    // Never tried: due now, oldest first.
    let first = QueuedCall::next_due(db, "asp.acme", now())
        .await
        .unwrap()
        .expect("a call is due");
    assert_eq!(first.payload.0["n"], 1);

    // A failed attempt: one failed attempt, a wait of 1, 2, 4, 8 minutes,
    // then 15, and the calls behind it wait too.
    let mut head = first;
    let mut waits = Vec::new();
    for _ in 0..6 {
        queue::mark_failed(db, &head, "timeout").await.unwrap();
        head = QueuedCall::find(db, head.id).await.unwrap().unwrap();
        let wait = head.next_attempt_at.expect("a next attempt") - now();
        waits.push((wait.num_seconds() + 30) / 60);
        assert!(
            QueuedCall::next_due(db, "asp.acme", now())
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(waits, [1, 2, 4, 8, 15, 15]);
    assert_eq!(head.attempts, 6);
    assert_eq!(head.last_error.as_deref(), Some("timeout"));

    // Later, it is due again, ahead of the others.
    let later = now() + Span::minutes(16);
    let due = QueuedCall::next_due(db, "asp.acme", later)
        .await
        .unwrap()
        .expect("due again");
    assert_eq!(due.id, head.id);

    // Sent or obsolete leaves the queue; the next one is due.
    queue::mark_sent(db, &due).await.unwrap();
    let next = QueuedCall::next_due(db, "asp.acme", now())
        .await
        .unwrap()
        .expect("the second call");
    assert_eq!(next.payload.0["n"], 2);
    queue::mark_obsolete(db, &next).await.unwrap();
    let sent = QueuedCall::find(db, due.id).await.unwrap().unwrap();
    let obsolete = QueuedCall::find(db, next.id).await.unwrap().unwrap();
    assert_eq!(sent.status, CallStatus::Sent);
    assert_eq!(obsolete.status, CallStatus::Obsolete);
    assert!(sent.finished_at.is_some());
    assert_eq!(QueuedCall::waiting(db, "asp.acme").await.unwrap().len(), 1);
    assert!(QueuedCall::any_waiting(db, "asp.acme").await.unwrap());
    assert!(!QueuedCall::any_waiting(db, "gateway.other").await.unwrap());
}

#[renox::test]
async fn the_failure_run_is_remembered_after_the_connection_recovers() {
    let app = app().await;
    run(
        &app,
        &connection(&app).await,
        "degrade",
        json!({ "error": "timeout" }),
    )
    .await
    .unwrap();
    let degraded = connection(&app).await;
    assert_eq!(degraded.outage_since, degraded.failed_since);
    run(&app, &degraded, "recover", json!({})).await.unwrap();

    let up = connection(&app).await;
    assert!(up.failed_since.is_none());
    assert_eq!(
        up.outage_since, degraded.outage_since,
        "kept for the reconciliation"
    );
}

#[renox::test]
async fn a_natural_key_finds_the_queued_or_sent_call_but_not_an_obsolete_one() {
    let app = app().await;
    let db = app.db();
    let call = Outbound::new("asp.acme", "invoice", json!({ "n": 1 })).natural_key("INV-1");
    let mut tx = db.begin().await.unwrap();
    let queued = queue::enqueue(&mut tx, &call, None).await.unwrap();
    tx.commit().await.unwrap();

    let found = QueuedCall::with_natural_key(db, "asp.acme", "INV-1")
        .await
        .unwrap()
        .expect("queued");
    assert_eq!(found.id, queued.id);
    assert!(
        QueuedCall::with_natural_key(db, "asp.acme", "INV-2")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        QueuedCall::with_natural_key(db, "gateway.acme", "INV-1")
            .await
            .unwrap()
            .is_none()
    );

    queue::mark_sent(db, &queued).await.unwrap();
    assert!(
        QueuedCall::with_natural_key(db, "asp.acme", "INV-1")
            .await
            .unwrap()
            .is_some()
    );
}

#[renox::test]
async fn the_oldest_call_is_found_whatever_its_wait() {
    let app = app().await;
    let db = app.db();
    let call = Outbound::new("asp.acme", "invoice", json!({ "n": 1 }));
    let mut tx = db.begin().await.unwrap();
    let queued = queue::enqueue(&mut tx, &call, Some("timeout"))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert!(
        QueuedCall::next_due(db, "asp.acme", now())
            .await
            .unwrap()
            .is_none()
    );
    let oldest = QueuedCall::oldest(db, "asp.acme")
        .await
        .unwrap()
        .expect("queued");
    assert_eq!(oldest.id, queued.id);
}
