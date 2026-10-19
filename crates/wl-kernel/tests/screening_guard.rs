//! #41 (BL-PTY-08), the kernel side of the acceptance checks: with `parties`
//! or screening off nobody provides a screening guard, so `screening_guard(state)`
//! is the no-op and `run_screened` runs the transition directly (AC7). The
//! same engagement with a test double of the guard shows that `run_screened`
//! warns without refusing and that a refusal runs nothing and writes no
//! history row (AC3, AC6 at the kernel level). The real guard and the
//! party page are in `crates/wl-parties/tests/stale_onboarding.rs`.

use renox::auth::Permissions;
use renox::db::{Migration, Transaction};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, history};
use renox::testing::TestApp;
use serde::{Deserialize, Serialize};
use wl_kernel::screening_guard::{
    ScreenedError, ScreeningFuture, ScreeningGuard, ScreeningOutcome, ScreeningRefusal, StaleList,
    provide_screening_guard, run_screened, screening_guard,
};
use wl_kernel::{ClientConfig, PartyId};

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum EngagementStatus {
    #[default]
    Draft,
    Active,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_kernel_engagements", state_machine)]
struct Engagement {
    id: i64,
    status: EngagementStatus,
    party_id: i64,
}

impl StateMachine for Engagement {
    type State = EngagementStatus;
    const COLUMN: renox::db::Col<Engagement, EngagementStatus> = Engagement::STATUS;

    fn machine() -> Machine<Self> {
        use EngagementStatus::{Active, Draft};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .state(Active)
            .terminal()
            .transition(Transition::new("activate").from([Draft]).to(Active))
    }
}

const UP: &str = r#"
CREATE TABLE "test_kernel_engagements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL DEFAULT 'draft',
    party_id INTEGER NOT NULL DEFAULT 0
);
"#;

const DOWN: &str = r#"DROP TABLE "test_kernel_engagements";"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261019130003_create_test_kernel_engagements",
    UP,
    Some(DOWN),
)];

/// Every party is onboarded on the UN list of 2026-11-02, stale; block mode
/// refuses, warn mode lets it pass.
struct StaleUn {
    block: bool,
}

impl StaleUn {
    fn lists() -> Vec<StaleList> {
        vec![StaleList::new(
            "UN Consolidated List",
            renox::chrono::NaiveDate::from_ymd_opt(2026, 11, 2),
        )]
    }
}

impl ScreeningGuard for StaleUn {
    fn check_in<'a>(
        &'a self,
        _tx: &'a mut Transaction,
        party: PartyId,
    ) -> ScreeningFuture<'a, ScreeningOutcome> {
        let block = self.block;
        Box::pin(async move {
            Ok(if block {
                ScreeningOutcome::Blocked(ScreeningRefusal::new(party, Self::lists()))
            } else {
                ScreeningOutcome::Warn(Self::lists())
            })
        })
    }

    fn latest_notice_in<'a>(
        &'a self,
        _tx: &'a mut Transaction,
        _party: PartyId,
    ) -> ScreeningFuture<'a, Vec<StaleList>> {
        Box::pin(async { Ok(Self::lists()) })
    }
}

/// Runs `activate` as the signed-in user (a test cannot build an `AuthUser`).
async fn activate(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<String> {
    let record = Engagement::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let done = run_screened(
        &state,
        &user,
        &record,
        "activate",
        PartyId(record.party_id),
        json!({}),
    )
    .await
    .map_err(ScreenedError::into_error)?;
    Ok(match done.screening {
        ScreeningOutcome::Warn(lists) => format!(
            "warn: {}",
            lists
                .iter()
                .map(StaleList::label_en)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => "active".to_owned(),
    })
}

/// `Some(block)` provides the double; `None` provides nothing, as with
/// `parties` or screening off.
struct Engagements(Option<bool>);

impl Module for Engagements {
    fn name(&self) -> &'static str {
        "test-kernel-engagements"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .post("/engagements/{id}/activate", activate)
            .require_auth()
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Engagement>();
        if let Some(block) = self.0 {
            provide_screening_guard(app, StaleUn { block });
        }
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

async fn world(guard: Option<bool>) -> (TestApp, Engagement) {
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Engagements(guard)),
        config,
    )
    .expect("the kernel applies");
    let app = TestApp::new(app).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    let user = User::register(app.db(), "Aisha", "aisha@example.com", "password123")
        .await
        .unwrap();
    let engagement = Engagement::create(
        app.db(),
        Engagement {
            party_id: 42,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    app.acting_as(&user);
    (app, engagement)
}

async fn rows(app: &TestApp, engagement: &Engagement) -> Vec<String> {
    history(app.db(), Engagement::TABLE, engagement.id)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.transition)
        .collect()
}

async fn status(app: &TestApp, engagement: &Engagement) -> EngagementStatus {
    Engagement::find(app.db(), engagement.id)
        .await
        .unwrap()
        .unwrap()
        .status
}

/// AC7: nobody provides a guard, so any party is allowed with no notice and
/// the transition runs directly, with its history row.
#[renox::test]
async fn ac7_without_a_provided_guard_everything_is_allowed_and_the_transition_runs() {
    let (app, engagement) = world(None).await;
    let state = app.state();
    let guard = screening_guard(state);
    let mut tx = state.db.begin().await.unwrap();
    for party in [1, 42, 999_999] {
        let outcome = guard.check_in(&mut tx, PartyId(party)).await.unwrap();
        assert_eq!(outcome, ScreeningOutcome::Allowed, "party {party}");
        let notice = guard.latest_notice_in(&mut tx, PartyId(party)).await;
        assert!(notice.unwrap().is_empty(), "party {party}");
    }
    drop(tx);

    let res = app
        .post(
            &format!("/engagements/{}/activate", engagement.id),
            &[("x", "y")],
        )
        .await;
    assert_eq!(res.text(), "active");
    assert_eq!(status(&app, &engagement).await, EngagementStatus::Active);
    assert_eq!(rows(&app, &engagement).await, ["activate"]);
}

/// AC6 (kernel side): a warning never refuses; the transition runs and the
/// caller gets the lists for its flash message.
#[renox::test]
async fn a_warning_runs_the_transition_and_returns_the_lists() {
    let (app, engagement) = world(Some(false)).await;
    let res = app
        .post(
            &format!("/engagements/{}/activate", engagement.id),
            &[("x", "y")],
        )
        .await;
    res.assert_ok();
    assert_eq!(res.text(), "warn: UN Consolidated List of 2026-11-02");
    assert_eq!(status(&app, &engagement).await, EngagementStatus::Active);
    assert_eq!(rows(&app, &engagement).await, ["activate"]);
}

/// AC3 (kernel side): a refusal is a 422 `stale_list` naming the list and its
/// date; nothing runs and no history row is written.
#[renox::test]
async fn a_refusal_is_a_422_that_runs_nothing_and_writes_no_history() {
    let (app, engagement) = world(Some(true)).await;
    let res = app
        .post(
            &format!("/engagements/{}/activate", engagement.id),
            &[("x", "y")],
        )
        .await;
    res.assert_status(422);
    assert!(
        res.text()
            .contains("Screening list is out of date: UN Consolidated List of 2026-11-02"),
        "{}",
        res.text()
    );
    assert_eq!(status(&app, &engagement).await, EngagementStatus::Draft);
    assert!(rows(&app, &engagement).await.is_empty());
}
