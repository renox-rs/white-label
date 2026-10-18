//! Rule runs (wl-kernel §4.2, §5.3; #29 part 3): the occurrence key, retries
//! with backoff, `fail` with `AutomationFailed`, and the chain-depth `stop`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use renox::auth::Permissions;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_kernel::automation::transitions;
use wl_kernel::{
    AutomationFailed, ClientConfig, RuleRun, RuleRunSpec, RuleRunStatus, RunOutcome, Subject,
    run_rule,
};

const CLIENT: &str = r#"
[client]
name = "Acme Properties LLC"
app = "property"

[modules]
enabled = ["parties"]

[[legal_entities]]
code = "ACME"
name_en = "Acme Properties LLC"
default = true
"#;

const RULE: &str = "TST.RUL.01";

async fn app() -> TestApp {
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(App::new().module(Auth::new()).module(Permissions), config)
        .expect("the kernel applies");
    let app = TestApp::new(app).await;
    app.fake_events();
    app
}

fn spec(key: &str) -> RuleRunSpec {
    RuleRunSpec::new(RULE, Subject::new("tests.thing", 7), key)
}

/// Runs `spec` with an action that counts its calls and fails when `fail`.
async fn attempt(
    app: &TestApp,
    spec: RuleRunSpec,
    calls: &Arc<AtomicUsize>,
    fail: bool,
) -> RunOutcome {
    let calls = Arc::clone(calls);
    app.at_travelled_time(run_rule(app.state(), spec, move |_state| async move {
        calls.fetch_add(1, Ordering::SeqCst);
        if fail {
            Err(Error::Status(
                StatusCode::BAD_GATEWAY,
                "provider down".to_owned(),
            ))
        } else {
            Ok(())
        }
    }))
    .await
    .unwrap()
}

async fn the_run(app: &TestApp) -> RuleRun {
    RuleRun::query().first(app.db()).await.unwrap().unwrap()
}

#[renox::test]
async fn an_occurrence_key_runs_once() {
    let app = app().await;
    let calls = Arc::new(AtomicUsize::new(0));
    assert_eq!(
        attempt(&app, spec("a"), &calls, false).await,
        RunOutcome::Succeeded
    );
    assert_eq!(
        attempt(&app, spec("a"), &calls, false).await,
        RunOutcome::Skipped
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(RuleRun::query().count(app.db()).await.unwrap(), 1);
    let run = the_run(&app).await;
    assert_eq!(run.status, RuleRunStatus::Succeeded);
    assert_eq!(run.attempts, 1);
    assert!(run.started_at.is_some() && run.finished_at.is_some());

    // Another key, rule or subject is another occurrence.
    assert_eq!(
        attempt(&app, spec("b"), &calls, false).await,
        RunOutcome::Succeeded
    );
    assert!(app.emitted::<AutomationFailed>().is_empty());
}

#[renox::test]
async fn a_failing_action_is_retried_with_backoff_then_fails_once() {
    let app = app().await;
    let calls = Arc::new(AtomicUsize::new(0));
    // Waits after the 1st to 4th failure: 1, 5, 15 and 60 minutes.
    for (n, wait) in [60_u64, 300, 900, 3600].into_iter().enumerate() {
        assert_eq!(
            attempt(&app, spec("a"), &calls, true).await,
            RunOutcome::Retrying
        );
        assert_eq!(calls.load(Ordering::SeqCst), n + 1);
        let run = the_run(&app).await;
        assert_eq!(run.status, RuleRunStatus::Retrying);
        assert_eq!(run.attempts, i64::try_from(n + 1).unwrap());
        assert_eq!(run.error.as_deref(), Some("provider down"));

        // Not yet due: nothing runs.
        app.travel(Duration::from_secs(wait - 1));
        assert_eq!(
            attempt(&app, spec("a"), &calls, true).await,
            RunOutcome::Retrying
        );
        assert_eq!(calls.load(Ordering::SeqCst), n + 1);
        app.travel(Duration::from_secs(1));
    }
    assert!(app.emitted::<AutomationFailed>().is_empty());

    // The 5th attempt is the last.
    assert_eq!(
        attempt(&app, spec("a"), &calls, true).await,
        RunOutcome::Failed
    );
    assert_eq!(calls.load(Ordering::SeqCst), 5);
    let run = the_run(&app).await;
    assert_eq!(run.status, RuleRunStatus::Failed);
    assert_eq!(run.attempts, 5);
    assert!(run.finished_at.is_some());
    let failed = app.emitted::<AutomationFailed>();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].rule, RULE);
    assert_eq!(failed[0].subject, Subject::new("tests.thing", 7));
    assert_eq!(failed[0].error, "provider down");

    // A failed run is left alone: no attempt, no second event.
    app.travel(Duration::from_secs(24 * 3600));
    assert_eq!(
        attempt(&app, spec("a"), &calls, false).await,
        RunOutcome::Failed
    );
    assert_eq!(calls.load(Ordering::SeqCst), 5);
    assert_eq!(app.emitted::<AutomationFailed>().len(), 1);
}

#[renox::test]
async fn a_retry_that_succeeds_ends_the_run() {
    let app = app().await;
    let calls = Arc::new(AtomicUsize::new(0));
    assert_eq!(
        attempt(&app, spec("a"), &calls, true).await,
        RunOutcome::Retrying
    );
    app.travel(Duration::from_secs(60));
    assert_eq!(
        attempt(&app, spec("a"), &calls, false).await,
        RunOutcome::Succeeded
    );
    let run = the_run(&app).await;
    assert_eq!(run.status, RuleRunStatus::Succeeded);
    assert_eq!(run.attempts, 2);
    assert!(app.emitted::<AutomationFailed>().is_empty());
}

#[renox::test]
async fn a_chain_deeper_than_three_is_stopped() {
    let app = app().await;
    let calls = Arc::new(AtomicUsize::new(0));
    assert_eq!(
        attempt(&app, spec("ok").chain_depth(3), &calls, false).await,
        RunOutcome::Succeeded
    );
    assert_eq!(
        attempt(
            &app,
            spec("deep").chain_depth(4).compliance_critical(),
            &calls,
            false
        )
        .await,
        RunOutcome::Stopped
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let stopped = RuleRun::query()
        .where_(RuleRun::OCCURRENCE_KEY.eq("deep"))
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stopped.status, RuleRunStatus::Stopped);
    assert_eq!(stopped.attempts, 0);
    let failed = app.emitted::<AutomationFailed>();
    assert_eq!(failed.len(), 1);
    assert!(failed[0].compliance_critical);

    // The key is used up: asking again skips it and says nothing more.
    assert_eq!(
        attempt(&app, spec("deep").chain_depth(4), &calls, false).await,
        RunOutcome::Skipped
    );
    assert_eq!(app.emitted::<AutomationFailed>().len(), 1);
}

#[renox::test]
async fn only_the_listed_transitions_are_allowed() {
    let app = app().await;
    let calls = Arc::new(AtomicUsize::new(0));
    attempt(&app, spec("a"), &calls, false).await;
    let run = the_run(&app).await;

    // A succeeded run cannot be retried, failed, stopped or rerun.
    for name in [
        transitions::RUN,
        transitions::SKIP,
        transitions::RETRY,
        transitions::FAIL,
        transitions::STOP,
    ] {
        assert!(
            run.transition(name)
                .by_system()
                .run(app.state())
                .await
                .is_err(),
            "{name}"
        );
    }
    // `rerun` is for a user holding `automations.retry`, never the system.
    assert!(
        run.transition(transitions::RERUN)
            .by_system()
            .run(app.state())
            .await
            .is_err()
    );

    // The engine's guards: a queued run is not past its attempts or too deep.
    let queued = RuleRun::create(
        app.db(),
        RuleRun {
            rule: RULE.into(),
            subject_type: "tests.thing".into(),
            subject_id: 1,
            occurrence_key: "q".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        queued
            .transition(transitions::STOP)
            .by_system()
            .run(app.state())
            .await
            .is_err(),
        "depth 0 is not too deep"
    );
    assert!(
        queued
            .transition(transitions::FAIL)
            .by_system()
            .run(app.state())
            .await
            .is_err(),
        "only a retrying run fails"
    );
    let _ = queued
        .transition(transitions::SKIP)
        .by_system()
        .run(app.state())
        .await
        .unwrap();
    let skipped = RuleRun::find_or_404(app.db(), queued.id).await.unwrap();
    assert_eq!(skipped.status, RuleRunStatus::Skipped);
    assert!(skipped.finished_at.is_some());
}
