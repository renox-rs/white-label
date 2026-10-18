//! AC8 of #37 (BL-PTY-04): with the `parties` module off, nobody provides a
//! block guard, so `guard(state)` is the no-op: every kind is allowed and
//! `run_guarded` runs the transition directly. The rest of the acceptance
//! checks are in `crates/wl-parties/tests/block_guard.rs`; the guard with a
//! test double is in `guarded_transitions.rs`.

use renox::auth::Permissions;
use renox::db::{Migration, sql};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, history};
use renox::testing::TestApp;
use serde::{Deserialize, Serialize};
use wl_kernel::block_guard::{self, ActionKind, GuardOutcome, guard};
use wl_kernel::{ClientConfig, PartyId, SubjectMachine};

const PAYMENT: &str = "payments.payment";

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum PaymentStatus {
    #[default]
    Requested,
    Paid,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_unguarded_payments", state_machine)]
struct Payment {
    id: i64,
    status: PaymentStatus,
    party_id: i64,
    amount: i64,
}

impl StateMachine for Payment {
    type State = PaymentStatus;
    const COLUMN: renox::db::Col<Payment, PaymentStatus> = Payment::STATUS;

    fn machine() -> Machine<Self> {
        use PaymentStatus::{Paid, Requested};
        Machine::new()
            .initial([Requested])
            .state(Requested)
            .state(Paid)
            .terminal()
            .transition(Transition::new("pay").from([Requested]).to(Paid))
    }
}

const UP: &str = r#"
CREATE TABLE "test_unguarded_payments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL DEFAULT 'requested',
    party_id INTEGER NOT NULL DEFAULT 0,
    amount INTEGER NOT NULL DEFAULT 0
);
"#;

const DOWN: &str = r#"DROP TABLE "test_unguarded_payments";"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261019130002_create_test_unguarded_payments",
    UP,
    Some(DOWN),
)];

/// Runs `pay` guarded as the signed-in user (a test cannot build an
/// `AuthUser`).
async fn pay(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<&'static str> {
    let payment = Payment::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    let outcome = block_guard::run_guarded(&state, &user, &payment, "pay", json!({}))
        .await
        .map_err(|error| error.into_error())?;
    Ok(if outcome.is_pending() {
        "pending"
    } else {
        "paid"
    })
}

/// A module that declares `pay` guarded but runs in an app with no
/// `parties` module, so no guard is ever provided.
struct Payments;

impl Module for Payments {
    fn name(&self) -> &'static str {
        "test-unguarded-payments"
    }

    fn routes(&self) -> Routes {
        Routes::new().post("/payments/{id}/pay", pay).require_auth()
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Payment>();
        SubjectMachine::of::<Payment>(PAYMENT)
            .block_guarded("pay", ActionKind::MoneyOut, |p: &Payment| {
                Some(PartyId(p.party_id))
            })
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

/// AC8: every kind is allowed for any party, and a guarded transition runs
/// directly: it is done, with its history row, and nothing is refused.
#[renox::test]
async fn ac8_without_parties_nothing_is_refused_and_the_transition_runs_directly() {
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Payments),
        config,
    )
    .expect("the kernel applies");
    let app = TestApp::new(app).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    let state = app.state();

    let guard = guard(state);
    let mut tx = state.db.begin().await.unwrap();
    for kind in ActionKind::ALL {
        for party in [1, 42, 999_999] {
            let outcome = guard
                .check_in(&mut tx, None, PartyId(party), kind)
                .await
                .unwrap();
            assert_eq!(outcome, GuardOutcome::Allowed, "{kind} for {party}");
        }
    }
    drop(tx);
    assert_eq!(guard.exception_step().role, "finance_manager");

    let db = app.db();
    let user = User::register(db, "Aisha", "aisha@example.com", "password123")
        .await
        .unwrap();
    let payment = Payment::create(
        db,
        Payment {
            party_id: 42,
            amount: 840_000,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    app.acting_as(&user);
    let res = app
        .post(&format!("/payments/{}/pay", payment.id), &[("x", "y")])
        .await;
    assert_eq!(res.text(), "paid");
    let paid = Payment::find(db, payment.id).await.unwrap().unwrap();
    assert_eq!(paid.status, PaymentStatus::Paid);
    let rows = history(db, Payment::TABLE, payment.id).await.unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r.transition.as_str())
            .collect::<Vec<_>>(),
        ["pay"]
    );
    let requests = sql("SELECT COUNT(*) FROM wl_approval_requests")
        .scalar::<i64>(db)
        .await
        .unwrap();
    assert_eq!(requests, 0, "no approval request was created");
}
