//! Acceptance checks of #30: draining the queue and reconciling manual work
//! when a connection recovers.
//!
//! The connection `gateway.acme` is registered by a test-only payments
//! module. Its send handler fails while `test_gateway.mode` is `down`, or for
//! the call `fail_n`; a call whose number is in `test_cancelled` is obsolete.
//! Inbound callbacks are matched to the payments entered by hand by their
//! external reference (the natural key). Tasks are recorded by a test
//! `TaskCreator`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use renox::auth::{Permissions, permissions};
use renox::chrono::Duration as Span;
use renox::db::{Migration, now, sql};
use renox::prelude::*;
use renox::serde_json::json;
use renox::testing::TestApp;
use wl_kernel::integrations::{Received, Reconciliation, Submitted, call_failed, service};
use wl_kernel::late_entries::{Entered, EntrySpec};
use wl_kernel::machines::BoxFuture;
use wl_kernel::tickets::{
    OpenTicket, TaskComment, TaskCreator, TicketFuture, TicketId, TicketRef, provide_task_creator,
};
use wl_kernel::{
    ClientConfig, ConnectionStatus, Delivery, Inbound, IntegrationConnection, IntegrationDef,
    IntegrationStatusChanged, LateEntryDef, Match, NewRecord, Outbound, QueuedCall, Subject,
};

const KEY: &str = "gateway.acme";
const PAYMENT: &str = "payments.payment";

const UP: &str = r#"
CREATE TABLE "test_gateway" (mode TEXT NOT NULL, fail_n INTEGER NOT NULL);
INSERT INTO "test_gateway" (mode, fail_n) VALUES ('down', 0);
CREATE TABLE "test_sent" (id INTEGER PRIMARY KEY AUTOINCREMENT, n INTEGER NOT NULL);
CREATE TABLE "test_cancelled" (n INTEGER NOT NULL);
CREATE TABLE "test_payments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_reference TEXT NOT NULL,
    amount INTEGER NOT NULL,
    late INTEGER NOT NULL DEFAULT 0
);
"#;

const DOWN: &str = r#"
DROP TABLE "test_payments";
DROP TABLE "test_cancelled";
DROP TABLE "test_sent";
DROP TABLE "test_gateway";
"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261019100299_create_test_gateway",
    UP,
    Some(DOWN),
)];

/// Fails while the gateway is down, or for call `fail_n`; else records the
/// call.
fn send<'a>(state: &'a AppState, call: &'a Outbound) -> BoxFuture<'a, Result<Delivery>> {
    Box::pin(async move {
        let n = call.payload["n"].as_i64().unwrap_or_default();
        let mode = sql("SELECT mode FROM test_gateway LIMIT 1")
            .scalar::<String>(&state.db)
            .await?;
        let fail_n = sql("SELECT fail_n FROM test_gateway LIMIT 1")
            .scalar::<i64>(&state.db)
            .await?;
        if mode == "down" || n == fail_n {
            return Err(call_failed("504 gateway timeout"));
        }
        sql("INSERT INTO test_sent (n) VALUES (?)")
            .bind(n)
            .execute(&state.db)
            .await?;
        Ok(Delivery::Sent)
    })
}

/// A call whose record was cancelled while it was queued is not due.
fn due<'a>(state: &'a AppState, call: &'a Outbound) -> BoxFuture<'a, Result<bool>> {
    Box::pin(async move {
        let n = call.payload["n"].as_i64().unwrap_or_default();
        let cancelled = sql("SELECT COUNT(*) FROM test_cancelled WHERE n = ?")
            .bind(n)
            .scalar::<i64>(&state.db)
            .await?;
        Ok(cancelled == 0)
    })
}

/// The payment entered by hand with the callback's external reference.
fn matcher<'a>(state: &'a AppState, item: &'a Inbound) -> BoxFuture<'a, Result<Match>> {
    Box::pin(async move {
        let found = sql("SELECT id, amount FROM test_payments WHERE external_reference = ?")
            .bind(item.natural_key.as_str())
            .fetch_optional(&state.db)
            .await?;
        let Some(row) = found else {
            return Ok(Match::NoRecord);
        };
        let id: i64 = row.try_get("id")?;
        let manual: i64 = row.try_get("amount")?;
        let gateway = item.payload["amount"].as_i64().unwrap_or_default();
        let subject = Subject::new(PAYMENT, id);
        Ok(if manual == gateway {
            Match::Same(subject)
        } else {
            Match::Differs {
                subject,
                detail: format!(
                    "AED {} manual vs AED {} gateway",
                    manual / 100,
                    gateway / 100
                ),
            }
        })
    })
}

fn create_payment<'a>(
    tx: &'a mut renox::db::Transaction,
    new: &'a NewRecord,
) -> BoxFuture<'a, Result<i64>> {
    Box::pin(async move {
        let key = new.payload["external_reference"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let amount = new.payload["amount"].as_i64().unwrap_or_default();
        sql("INSERT INTO test_payments (external_reference, amount, late) VALUES (?, ?, ?)")
            .bind(key)
            .bind(amount)
            .bind(i64::from(new.late))
            .execute(&mut *tx)
            .await?;
        let id = sql("SELECT MAX(id) FROM test_payments")
            .scalar::<i64>(&mut *tx)
            .await?;
        Ok(id)
    })
}

/// A recording task creator.
#[derive(Clone, Default)]
struct Tasks(Arc<Mutex<Recorded>>);

#[derive(Default)]
struct Recorded {
    opened: Vec<OpenTicket>,
    comments: Vec<(i64, String)>,
}

impl Tasks {
    fn opened(&self) -> Vec<OpenTicket> {
        self.0.lock().unwrap().opened.clone()
    }

    fn comments(&self) -> Vec<(i64, String)> {
        self.0.lock().unwrap().comments.clone()
    }
}

impl TaskCreator for Tasks {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let mut recorded = self.0.lock().unwrap();
            recorded.opened.push(ticket);
            let id = i64::try_from(recorded.opened.len()).unwrap_or(i64::MAX);
            Ok(Some(TicketRef {
                id: TicketId(id),
                number: format!("TKT-2026-{id:04}"),
            }))
        })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let recorded = self.0.lock().unwrap();
            Ok(recorded
                .opened
                .iter()
                .position(|t| t.source_key.as_deref() == Some(source_key))
                .map(|index| {
                    let id = i64::try_from(index + 1).unwrap_or(i64::MAX);
                    TicketRef {
                        id: TicketId(id),
                        number: format!("TKT-2026-{id:04}"),
                    }
                }))
        })
    }

    fn comment<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        comment: TaskComment,
    ) -> TicketFuture<'a, ()> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .comments
                .push((ticket.0, comment.text));
            Ok(())
        })
    }
}

struct Payments {
    tasks: Tasks,
}

impl Module for Payments {
    fn name(&self) -> &'static str {
        "test-payments"
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        provide_task_creator(app, self.tasks.clone());
        IntegrationDef::of(KEY, "payments", send)
            .owner_role("accountant")
            .screens(["payments.payment_list"])
            .fallback("Take card payments on the standalone terminal.", None)
            .due(due)
            .inbound(matcher, |item| {
                EntrySpec::new(PAYMENT, item.user_id, item.occurred_at)
                    .reason("received late from the gateway")
                    .payload(json!({
                        "external_reference": item.natural_key,
                        "amount": item.payload["amount"],
                    }))
            })
            .register();
        LateEntryDef::of(PAYMENT, create_payment)
            .business_time("paid_at")
            .register();
    }
}

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

struct World {
    app: TestApp,
    tasks: Tasks,
    accountant: User,
    manager: User,
}

async fn world() -> World {
    let tasks = Tasks::default();
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Payments {
                tasks: tasks.clone(),
            }),
        config,
    )
    .expect("the kernel applies");
    let app = TestApp::new(app).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app.fake_events();

    let db = app.db();
    permissions::define_role(db, "admin", &[]).await.unwrap();
    permissions::define_role(db, "accountant", &[])
        .await
        .unwrap();
    permissions::define_role(
        db,
        "integrations_manager",
        &[
            wl_kernel::permissions::INTEGRATIONS_VIEW,
            wl_kernel::permissions::INTEGRATIONS_MANAGE,
        ],
    )
    .await
    .unwrap();
    let user = |name: &'static str, role: &'static str| async move {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(db, name, &email, "password123")
            .await
            .unwrap();
        user.assign_role(db, role).await.unwrap();
        user
    };
    World {
        accountant: user("Amir", "accountant").await,
        manager: user("Mona", "integrations_manager").await,
        tasks,
        app,
    }
}

impl World {
    fn refund(&self, n: i64) -> Outbound {
        Outbound::new(KEY, "refund", json!({ "n": n })).subject(Subject::new("payments.refund", n))
    }

    async fn submit(&self, call: Outbound) -> Submitted {
        self.app
            .at_travelled_time(service::submit(self.app.state(), call))
            .await
            .expect("the call is handed over")
    }

    async fn tick(&self, minutes: u64) {
        self.app.travel(Duration::from_secs(minutes * 60));
        self.app
            .run_scheduled(service::TASK)
            .await
            .expect("the task runs");
    }

    async fn set(&self, mode: &str, fail_n: i64) {
        sql("UPDATE test_gateway SET mode = ?, fail_n = ?")
            .bind(mode)
            .bind(fail_n)
            .execute(self.app.db())
            .await
            .unwrap();
    }

    /// Three refund calls queued while the gateway is down, and the
    /// connection `down` (the checks run at "10:15").
    async fn down_with_three_calls(&self) {
        for n in 1..=3 {
            assert!(matches!(
                self.submit(self.refund(n)).await,
                Submitted::Queued { .. }
            ));
        }
        self.tick(15).await;
        assert_eq!(self.connection().await.status, ConnectionStatus::Down);
        assert_eq!(self.waiting().await.len(), 3);
    }

    /// A call succeeded: the gateway is back (the checks run at "10:40").
    async fn succeed(&self, fail_n: i64) -> bool {
        self.set("ok", fail_n).await;
        self.app.travel(Duration::from_secs(25 * 60));
        self.app
            .at_travelled_time(service::report_success(self.app.state(), KEY))
            .await
            .expect("the success is reported")
    }

    async fn connection(&self) -> IntegrationConnection {
        IntegrationConnection::by_key(self.app.db(), KEY)
            .await
            .unwrap()
            .expect("the connection is stored")
    }

    async fn waiting(&self) -> Vec<QueuedCall> {
        QueuedCall::waiting(self.app.db(), KEY).await.unwrap()
    }

    async fn sent(&self) -> Vec<i64> {
        sql("SELECT n FROM test_sent ORDER BY id")
            .scalars::<i64>(self.app.db())
            .await
            .unwrap()
    }

    async fn payments(&self) -> i64 {
        sql("SELECT COUNT(*) FROM test_payments")
            .scalar::<i64>(self.app.db())
            .await
            .unwrap()
    }

    /// A card payment of `amount` fils entered by hand with `reference`.
    async fn manual_payment(&self, reference: &str, amount: i64) {
        sql("INSERT INTO test_payments (external_reference, amount, late) VALUES (?, ?, 1)")
            .bind(reference)
            .bind(amount)
            .execute(self.app.db())
            .await
            .unwrap();
    }

    fn callback(&self, reference: &str, amount: i64, ago: Span) -> Inbound {
        Inbound::new(
            KEY,
            reference,
            now() - ago,
            json!({ "amount": amount }),
            self.manager.id,
        )
    }

    async fn receive(&self, item: Inbound) -> Received {
        self.app
            .at_travelled_time(service::receive(self.app.state(), item))
            .await
            .expect("the callback is processed")
    }

    fn changes(&self) -> Vec<(String, String)> {
        self.app
            .emitted::<IntegrationStatusChanged>()
            .into_iter()
            .map(|e| (e.from, e.to))
            .collect()
    }
}

/// AC1: with the connection down and three queued refund calls, a success
/// brings it up, sends the three calls in order and removes the banner.
#[renox::test]
async fn a_recovery_sends_the_queued_calls_in_order() {
    let w = world().await;
    w.down_with_three_calls().await;

    assert!(w.succeed(0).await, "the connection recovered");
    let up = w.connection().await;
    assert_eq!(up.status, ConnectionStatus::Up);
    assert!(up.failed_since.is_none());
    assert_eq!(w.sent().await, [1, 2, 3]);
    assert!(w.waiting().await.is_empty());
    assert_eq!(
        w.changes().last(),
        Some(&("down".to_owned(), "up".to_owned()))
    );
    // Nothing needed reconciling: no task.
    assert!(w.tasks.opened().is_empty());
    assert!(
        Reconciliation::of_connection(w.app.db(), KEY)
            .await
            .unwrap()
            .is_empty()
    );
}

/// AC2: a card payment of AED 250 entered by hand with external reference
/// TX-889 and the gateway's late callback for TX-889 of AED 250: no new
/// payment and no task.
#[renox::test]
async fn a_callback_for_a_payment_done_by_hand_is_linked_not_created() {
    let w = world().await;
    w.manual_payment("TX-889", 25_000).await;

    let received = w
        .receive(w.callback("TX-889", 25_000, Span::minutes(20)))
        .await;
    assert!(matches!(received, Received::Linked(subject) if subject.subject_id == 1));
    assert_eq!(w.payments().await, 1);
    assert!(w.tasks.opened().is_empty());
}

/// AC3: the callback says AED 205: the owner role gets one reconciliation
/// task listing TX-889 (AED 250 manual vs AED 205 gateway); a second
/// mismatch of the same recovery is added to that task, not to a new one.
#[renox::test]
async fn a_mismatch_opens_one_task_for_the_owner_role() {
    let w = world().await;
    w.down_with_three_calls().await;
    w.manual_payment("TX-889", 25_000).await;
    w.manual_payment("TX-890", 10_000).await;
    w.succeed(0).await;

    let first = w
        .receive(w.callback("TX-889", 20_500, Span::minutes(20)))
        .await;
    assert!(
        matches!(&first, Received::Mismatch { detail, .. } if detail == "AED 250 manual vs AED 205 gateway"),
        "{first:?}"
    );
    assert_eq!(w.payments().await, 2, "no payment was created");

    let opened = w.tasks.opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(
        opened[0].assignee,
        Some(wl_kernel::tickets::Assignee::user(w.accountant.id))
    );
    assert!(
        opened[0]
            .description
            .contains("TX-889: AED 250 manual vs AED 205 gateway"),
        "{}",
        opened[0].description
    );
    assert!(opened[0].title.contains(KEY));

    // Another mismatch of the same failure run: the same task.
    w.receive(w.callback("TX-890", 9_000, Span::minutes(15)))
        .await;
    assert_eq!(w.tasks.opened().len(), 1);
    let comments = w.tasks.comments();
    assert_eq!(comments.len(), 1);
    assert!(
        comments[0]
            .1
            .contains("TX-890: AED 100 manual vs AED 90 gateway")
    );
    let rows = Reconciliation::of_connection(w.app.db(), KEY)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].lines.len(), 2);
    assert_eq!(rows[0].ticket_number.as_deref(), Some("TKT-2026-0001"));
}

/// AC4: the second queued call fails during the drain: the connection went
/// up, is now degraded with `failed since` at that failure, the second and
/// third calls are still queued in that order and the first is not sent
/// again.
#[renox::test]
async fn a_failure_during_the_drain_degrades_again_and_keeps_the_rest_in_order() {
    let w = world().await;
    w.down_with_three_calls().await;
    let first_run = w.connection().await.failed_since.expect("failed since");

    assert!(w.succeed(2).await);
    assert_eq!(w.sent().await, [1]);
    let again = w.connection().await;
    assert_eq!(again.status, ConnectionStatus::Degraded);
    let since = again.failed_since.expect("failed since");
    assert!(
        since > first_run + Span::minutes(20),
        "the failure of the drain"
    );
    let waiting = w.waiting().await;
    let numbers: Vec<i64> = waiting
        .iter()
        .map(|c| c.payload.0["n"].as_i64().unwrap())
        .collect();
    assert_eq!(numbers, [2, 3]);
    assert_eq!(
        w.changes()
            .iter()
            .rev()
            .take(2)
            .cloned()
            .collect::<Vec<_>>(),
        [
            ("up".to_owned(), "degraded".to_owned()),
            ("down".to_owned(), "up".to_owned())
        ]
    );

    // Once the gateway answers, the rest goes out in order and the first is
    // not sent again.
    w.set("ok", 0).await;
    w.tick(16).await;
    assert_eq!(w.sent().await, [1, 2, 3]);
    assert_eq!(w.connection().await.status, ConnectionStatus::Up);
}

/// AC5: `enable` on a `disabled` connection with 2 queued items sends both.
#[renox::test]
async fn enable_sends_the_queued_calls() {
    let w = world().await;
    w.set("ok", 0).await;
    w.app.acting_as(&w.manager);
    let off = w
        .app
        .post(
            "/integrations/gateway.acme/disable",
            &[("reason", "maintenance")],
        )
        .await;
    assert_eq!(off.status.as_u16(), 303, "{}", off.text());
    for n in 1..=2 {
        assert!(matches!(
            w.submit(w.refund(n)).await,
            Submitted::Queued { .. }
        ));
    }
    assert!(w.sent().await.is_empty());

    let on = w
        .app
        .post("/integrations/gateway.acme/enable", &[("x", "y")])
        .await;
    assert_eq!(on.status.as_u16(), 303, "{}", on.text());
    assert_eq!(w.connection().await.status, ConnectionStatus::Up);
    assert_eq!(w.sent().await, [1, 2]);
    assert!(w.waiting().await.is_empty());
}

/// §5.14 step 4: a queued call whose record changed meanwhile is not sent;
/// it is listed on the reconciliation task.
#[renox::test]
async fn an_obsolete_call_is_not_sent_and_is_listed_for_reconciliation() {
    let w = world().await;
    w.down_with_three_calls().await;
    sql("INSERT INTO test_cancelled (n) VALUES (2)")
        .execute(w.app.db())
        .await
        .unwrap();

    w.succeed(0).await;
    assert_eq!(w.sent().await, [1, 3]);
    assert!(w.waiting().await.is_empty());
    let opened = w.tasks.opened();
    assert_eq!(opened.len(), 1);
    assert!(
        opened[0].description.contains("refund payments.refund:2"),
        "{}",
        opened[0].description
    );
}

/// A callback with no payment done by hand is entered as a new record with
/// its own business time: 90 minutes late, it is flagged with the module's
/// reason (the late-entry rules).
#[renox::test]
async fn a_callback_with_no_manual_payment_is_entered_with_its_own_time() {
    let w = world().await;
    let received = w
        .receive(w.callback("TX-901", 12_000, Span::minutes(90)))
        .await;
    let Received::Created(entered) = received else {
        panic!("expected a new record");
    };
    assert!(matches!(*entered, Entered::Recorded { .. }), "{entered:?}");
    assert_eq!(w.payments().await, 1);
    assert!(w.tasks.opened().is_empty());

    // The same callback again finds the payment: no second one.
    let again = w
        .receive(w.callback("TX-901", 12_000, Span::minutes(90)))
        .await;
    assert!(matches!(again, Received::Linked(_)));
    assert_eq!(w.payments().await, 1);
}

/// A natural key prevents a second call.
#[renox::test]
async fn a_call_with_a_known_natural_key_is_not_queued_twice() {
    let w = world().await;
    let call = || w.refund(1).natural_key("R-1");
    let first = w.submit(call()).await;
    let Submitted::Queued { id } = first else {
        panic!("expected a queued call");
    };
    assert_eq!(w.submit(call()).await, Submitted::Duplicate { id });
    assert_eq!(w.waiting().await.len(), 1);
}
